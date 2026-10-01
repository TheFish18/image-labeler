mod app;
mod db;
mod geometry;
mod image_data;
mod schema;

use app::LabelerApp;
use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
};

const DEFAULT_DB_PATH: &str = "labels.sqlite3";
const USAGE: &str = "Usage: image-labeler [--db <path>] [--schema <name>|<file.toml>] [--input <dir>]

Options:
  --db <path>           SQLite database to store labels in (default: labels.sqlite3)
  --schema <name>       Start with an existing schema from the config directory
  --schema <file.toml>  Add the schema in this file to the config directory and start with it.
                        If a different schema with the same name exists, asks before overwriting
  --input <dir>         Directory the file browser starts in (default: current directory)
  -h, --help            Print this help";

struct CliArgs {
    db_path: PathBuf,
    schema_path: Option<PathBuf>,
    input_dir: Option<PathBuf>,
}

/// Returns the value of `--name <value>` or `--name=<value>`, if `arg` is that flag.
fn flag_value(
    arg: &str,
    name: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<Option<PathBuf>, String> {
    let value = if arg == name {
        args.next().ok_or(format!("{name} requires a path"))?
    } else if let Some(value) = arg.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')) {
        value.to_string()
    } else {
        return Ok(None);
    };
    if value.is_empty() {
        return Err(format!("{name} path cannot be empty"));
    }
    Ok(Some(PathBuf::from(value)))
}

fn parse_args() -> Result<CliArgs, String> {
    let mut db_path = PathBuf::from(DEFAULT_DB_PATH);
    let mut schema_path = None;
    let mut input_dir = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        if let Some(value) = flag_value(&arg, "--db", &mut args)? {
            db_path = value;
        } else if let Some(value) = flag_value(&arg, "--schema", &mut args)? {
            schema_path = Some(value);
        } else if let Some(value) = flag_value(&arg, "--input", &mut args)? {
            input_dir = Some(value);
        } else {
            return Err(format!("unrecognized argument `{arg}`"));
        }
    }

    Ok(CliArgs {
        db_path,
        schema_path,
        input_dir,
    })
}

/// Resolves `--schema` to the name of a schema in the config directory.
/// A value ending in `.toml` is a schema file to add; anything else is a schema name.
fn resolve_schema(value: &Path) -> Result<String, String> {
    let is_file = value
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"));
    if is_file {
        add_schema_file(value).map_err(|error| format!("--schema: {error:#}"))
    } else {
        select_schema_by_name(&value.to_string_lossy())
    }
}

fn select_schema_by_name(name: &str) -> Result<String, String> {
    let to_error = |error: anyhow::Error| format!("--schema: {error:#}");
    if let Some(found) = schema::find_schema(name).map_err(to_error)? {
        return Ok(found);
    }
    let available = schema::list_schema_names().map_err(to_error)?.join(", ");
    Err(format!(
        "--schema: no schema named `{name}` in {}\n  available: {available}\n  \
         to add a new schema, pass a .toml file: --schema path/to/{name}.toml",
        schema::config_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default()
    ))
}

fn add_schema_file(source: &Path) -> anyhow::Result<String> {
    let file = schema::read_schema_file(source)?;
    let name = file.name.clone();
    let existing_path = schema::schema_file_path(&name)?;

    if schema::find_schema(&name)?.is_none() {
        let path = schema::write_schema_file(&file)?;
        println!("Added schema `{name}` to {}", path.display());
        return Ok(name);
    }
    let unchanged = std::fs::read_to_string(&existing_path)
        .is_ok_and(|existing| existing == file.content);
    if unchanged {
        return Ok(name);
    }

    println!(
        "A different schema named `{name}` already exists at {}",
        existing_path.display()
    );
    println!("(To load the existing schema, run with: --schema {name})");
    if confirm(&format!(
        "Overwrite it with {}? [y/N] ",
        source.display()
    ))? {
        schema::write_schema_file(&file)?;
        println!("Overwrote schema `{name}`");
    } else {
        println!("Keeping the existing schema `{name}`");
    }
    Ok(name)
}

/// Asks a yes/no question on the terminal. Without an interactive terminal
/// (e.g. launched from a desktop shortcut) the answer is no.
fn confirm(question: &str) -> io::Result<bool> {
    if !io::stdin().is_terminal() {
        println!("{question}no terminal to answer in; not overwriting");
        return Ok(false);
    }
    print!("{question}");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn resolve_input_dir(path: &PathBuf) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err(format!("--input: {} is not a directory", path.display()));
    }
    Ok(std::path::absolute(path).unwrap_or_else(|_| path.clone()))
}

fn main() -> eframe::Result<()> {
    let fail = |error: String| -> ! {
        eprintln!("error: {error}\n\n{USAGE}");
        std::process::exit(2);
    };
    let args = parse_args().unwrap_or_else(|error| fail(error));
    let initial_schema = args
        .schema_path
        .as_ref()
        .map(|path| resolve_schema(path).unwrap_or_else(|error| fail(error)));
    let input_dir = args
        .input_dir
        .as_ref()
        .map(|path| resolve_input_dir(path).unwrap_or_else(|error| fail(error)));

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Image Labeler",
        options,
        Box::new(move |cc| {
            Box::new(
                LabelerApp::new(cc, args.db_path, initial_schema, input_dir)
                    .expect("failed to start app"),
            )
        }),
    )
}
