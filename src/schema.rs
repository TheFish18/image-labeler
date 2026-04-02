use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub const APP_KEYBINDS_FILE: &str = "app-keybinds.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelType {
    Rectangle,
    Polygon,
    Global,
}

impl LabelType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rectangle => "rectangle",
            Self::Polygon => "polygon",
            Self::Global => "global",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KeybindConfig {
    #[serde(default)]
    pub chord: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchemaLabel {
    pub name: String,
    #[serde(rename = "type")]
    pub label_type: LabelType,
    pub color_rgb: [u8; 3],
    #[serde(default)]
    pub keybind: KeybindConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchemaDefinition {
    #[serde(default, alias = "annotation_labels", alias = "labels")]
    pub labels: Vec<SchemaLabel>,
}

impl SchemaDefinition {
    pub fn default_schema() -> Self {
        Self {
            labels: vec![
                SchemaLabel {
                    name: "object".to_string(),
                    label_type: LabelType::Rectangle,
                    color_rgb: [255, 99, 71],
                    keybind: KeybindConfig {
                        chord: "o".to_string(),
                    },
                },
                SchemaLabel {
                    name: "region".to_string(),
                    label_type: LabelType::Polygon,
                    color_rgb: [64, 159, 255],
                    keybind: KeybindConfig {
                        chord: "r".to_string(),
                    },
                },
                SchemaLabel {
                    name: "contains-object".to_string(),
                    label_type: LabelType::Global,
                    color_rgb: [255, 206, 84],
                    keybind: KeybindConfig {
                        chord: "shift+o".to_string(),
                    },
                },
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppKeybinds {
    #[serde(default = "default_prev_image_keybind")]
    pub previous_image: KeybindConfig,
    #[serde(default = "default_next_image_keybind")]
    pub next_image: KeybindConfig,
    #[serde(default = "default_rotate_left_keybind")]
    pub rotate_left: KeybindConfig,
    #[serde(default = "default_rotate_right_keybind")]
    pub rotate_right: KeybindConfig,
    #[serde(default = "default_mirror_horizontal_keybind")]
    pub mirror_horizontal: KeybindConfig,
    #[serde(default = "default_mirror_vertical_keybind")]
    pub mirror_vertical: KeybindConfig,
    #[serde(default = "default_save_transformed_keybind")]
    pub save_transformed_image: KeybindConfig,
}

impl Default for AppKeybinds {
    fn default() -> Self {
        Self {
            previous_image: default_prev_image_keybind(),
            next_image: default_next_image_keybind(),
            rotate_left: default_rotate_left_keybind(),
            rotate_right: default_rotate_right_keybind(),
            mirror_horizontal: default_mirror_horizontal_keybind(),
            mirror_vertical: default_mirror_vertical_keybind(),
            save_transformed_image: default_save_transformed_keybind(),
        }
    }
}

fn default_prev_image_keybind() -> KeybindConfig {
    KeybindConfig {
        chord: "shift+h".to_string(),
    }
}

fn default_next_image_keybind() -> KeybindConfig {
    KeybindConfig {
        chord: "shift+l".to_string(),
    }
}

fn default_rotate_left_keybind() -> KeybindConfig {
    KeybindConfig {
        chord: "shift+j".to_string(),
    }
}

fn default_rotate_right_keybind() -> KeybindConfig {
    KeybindConfig {
        chord: "shift+k".to_string(),
    }
}

fn default_mirror_horizontal_keybind() -> KeybindConfig {
    KeybindConfig {
        chord: "h".to_string(),
    }
}

fn default_mirror_vertical_keybind() -> KeybindConfig {
    KeybindConfig {
        chord: "v".to_string(),
    }
}

fn default_save_transformed_keybind() -> KeybindConfig {
    KeybindConfig {
        chord: "shift+w".to_string(),
    }
}

pub fn ensure_default_files() -> Result<()> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

    let schema_path = dir.join("default.toml");
    if !schema_path.exists() {
        save_schema("default", &SchemaDefinition::default_schema())?;
    }

    let keybind_path = app_keybinds_path()?;
    if !keybind_path.exists() {
        save_app_keybinds(&AppKeybinds::default())?;
    }

    Ok(())
}

pub fn config_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(value) = env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(value).join("image-labeler"));
        }
        if let Some(value) = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(value).join("image-labeler"));
        }
        bail!("APPDATA and LOCALAPPDATA are unavailable");
    }

    #[cfg(not(target_os = "windows"))]
    {
        let base = match env::var_os("XDG_CONFIG_HOME") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => {
                let home = env::var_os("HOME")
                    .context("HOME is not set and XDG_CONFIG_HOME is unavailable")?;
                PathBuf::from(home).join(".config")
            }
        };
        Ok(base.join("image-labeler"))
    }
}

pub fn list_schema_names() -> Result<Vec<String>> {
    ensure_default_files()?;
    let dir = config_dir()?;
    let mut names = Vec::new();

    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }
        if path.file_name().and_then(|v| v.to_str()) == Some(APP_KEYBINDS_FILE) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
            names.push(stem.to_string());
        }
    }

    names.sort();
    if names.is_empty() {
        bail!("no schema files were found in {}", dir.display());
    }
    Ok(names)
}

pub fn load_schema(name: &str) -> Result<SchemaDefinition> {
    ensure_default_files()?;
    let path = schema_path(name)?;
    load_schema_from_path(&path)
}

pub fn load_schema_from_path(path: &Path) -> Result<SchemaDefinition> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn save_schema(name: &str, schema: &SchemaDefinition) -> Result<PathBuf> {
    if schema.labels.is_empty() {
        bail!("a schema must contain at least one label");
    }

    let normalized_name = normalize_schema_name(name);
    if normalized_name.is_empty() {
        bail!("schema name cannot be empty");
    }

    let dir = config_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(format!("{normalized_name}.toml"));
    let content = toml::to_string_pretty(schema).context("failed to serialize schema")?;
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

pub fn import_schema_from_file(path: &Path) -> Result<(String, PathBuf, SchemaDefinition)> {
    let schema = load_schema_from_path(path)?;
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(normalize_schema_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "imported-schema".to_string());
    let saved_path = save_schema(&name, &schema)?;
    Ok((name, saved_path, schema))
}

pub fn app_keybinds_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(APP_KEYBINDS_FILE))
}

pub fn load_app_keybinds() -> Result<AppKeybinds> {
    ensure_default_files()?;
    let path = app_keybinds_path()?;
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn save_app_keybinds(keybinds: &AppKeybinds) -> Result<PathBuf> {
    let path = app_keybinds_path()?;
    let content = toml::to_string_pretty(keybinds).context("failed to serialize app keybinds")?;
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn schema_path(name: &str) -> Result<PathBuf> {
    let normalized_name = normalize_schema_name(name);
    if normalized_name.is_empty() {
        bail!("schema name cannot be empty");
    }
    Ok(config_dir()?.join(format!("{normalized_name}.toml")))
}

fn normalize_schema_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
