mod app;
mod db;
mod geometry;
mod image_data;
mod schema;

use app::LabelerApp;
use std::path::PathBuf;

const DEFAULT_DB_PATH: &str = "labels.sqlite3";
const USAGE: &str = "Usage: image-labeler [--db <path>]

Options:
  --db <path>  SQLite database to store labels in (default: labels.sqlite3)
  -h, --help   Print this help";

struct CliArgs {
    db_path: PathBuf,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut db_path = PathBuf::from(DEFAULT_DB_PATH);
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--db" => {
                let value = args.next().ok_or("--db requires a path")?;
                db_path = PathBuf::from(value);
            }
            _ => match arg.strip_prefix("--db=") {
                Some(value) => db_path = PathBuf::from(value),
                None => return Err(format!("unrecognized argument `{arg}`")),
            },
        }
    }

    if db_path.as_os_str().is_empty() {
        return Err("--db path cannot be empty".to_string());
    }
    Ok(CliArgs { db_path })
}

fn main() -> eframe::Result<()> {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Image Labeler",
        options,
        Box::new(move |cc| {
            Box::new(LabelerApp::new(cc, args.db_path).expect("failed to start app"))
        }),
    )
}
