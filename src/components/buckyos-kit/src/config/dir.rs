use serde::Deserialize;
use serde_json::value::Value as JsonValue;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ConfigItem {
    pub path: PathBuf,
    pub value: JsonValue,
}

#[derive(Debug, Deserialize)]
struct Include {
    /// Path to the configuration file
    path: String,
}

#[derive(Debug, Deserialize)]
struct RootConfig {
    /// List of `includes` (parsed from both TOML and JSON files)
    includes: Vec<Include>,
}

fn get_root_file(dir: &Path) -> Option<PathBuf> {
    let root_file = dir.join("root");
    if root_file.exists() {
        return Some(root_file);
    }

    let root_file = dir.join("root.json");
    if root_file.exists() {
        return Some(root_file);
    }

    let root_file = dir.join("root.toml");
    if root_file.exists() {
        return Some(root_file);
    }
    None
}

// Convert TOML to JSON
fn toml_to_json(toml_value: toml::Value) -> Result<JsonValue, Box<dyn std::error::Error>> {
    let json_string = serde_json::to_string(&toml_value).map_err(|e| {
        let msg = format!("Failed to convert TOML to JSON: {:?}", e);
        error!("{}", msg);
        msg
    })?;

    let json_value: JsonValue = serde_json::from_str(&json_string).map_err(|e| {
        let msg = format!("Failed to parse JSON: {:?}", e);
        error!("{}", msg);
        msg
    })?;

    Ok(json_value)
}

// Load a config file, support JSON and TOML
async fn load_file(file: &Path) -> Result<JsonValue, Box<dyn std::error::Error>> {
    debug!("Loading config file: {:?}", file);
    assert!(file.exists());

    let content = tokio::fs::read_to_string(file).await.map_err(|e| {
        let msg = format!("Failed to read file: {:?}, error: {:?}", file, e);
        error!("{}", msg);
        msg
    })?;

    // First check file extension
    if let Some(ext) = file.extension().and_then(|s| s.to_str()) {
        match ext {
            "json" => {
                // Parse JSON file directly
                return serde_json::from_str(&content).map_err(|e| {
                    let msg = format!("Failed to parse JSON: {:?}", e);
                    error!("{}", msg);
                    msg.into()
                });
            }
            "toml" => {
                // Parse TOML file directly
                let toml_value: toml::Value = toml::from_str(&content).map_err(|e| {
                    let msg = format!("Failed to parse TOML: {:?}", e);
                    error!("{}", msg);
                    msg
                })?;

                return toml_to_json(toml_value);
            }
            _ => {
                // Unknown extension, use content to guess
            }
        }
    }

    // Guess by content if no extension
    if content.trim_start().starts_with('{') {
        // If content looks like JSON, parse it directly
        serde_json::from_str(&content).map_err(|e| {
            let msg = format!("Failed to parse JSON: {:?}", e);
            error!("{}", msg);
            msg.into()
        })
    } else {
        // Otherwise, parse it as TOML
        let toml_value: toml::Value = toml::from_str(&content).map_err(|e| {
            let msg = format!("Failed to parse TOML: {:?}", e);
            error!("{}", msg);
            msg
        })?;
        toml_to_json(toml_value)
    }
}

/// Root file format as below:
/// [[includes]]
/// path = "config/default.toml"
///
/// [[includes]]
/// path = "config/override.json"
///
/// or in json format:
/// {
///   "includes": [
///     {
///       "path": "config/default.toml"
///     },
///     {
///       "path": "config/override.json"
///     }
///   ]
/// }
///

pub async fn load_dir_with_root(
    dir: &Path,
    root_file: &Path,
) -> Result<Vec<ConfigItem>, Box<dyn std::error::Error>> {
    assert!(root_file.exists());

    let root_value = load_file(root_file).await?;

    let root_config: RootConfig = serde_json::from_value(root_value).map_err(|e| {
        let msg = format!("Failed to parse root config: {:?}", e);
        error!("{}", msg);
        msg
    })?;

    // Load sub files or dirs
    let mut config = Vec::new();
    for include in root_config.includes {
        let include_path = dir.join(include.path);

        // FIXME: what should we do if include_path not exists? return error or just ignore?
        if !include_path.exists() {
            let msg = format!("Include path not exists: {:?}", include_path);
            error!("{}", msg);
            return Err(msg.into());
        }

        if include_path.is_dir() {
            let items = load_dir(&include_path).await?;
            config.extend(items);
        } else {
            let value = load_file(&include_path).await?;
            config.push(ConfigItem {
                path: include_path,
                value,
            });
        }
    }

    Ok(config)
}

#[derive(Debug)]
struct IndexedFile {
    index: u32,
    path: PathBuf,
}

async fn scan_files(dir: &Path) -> Result<Vec<IndexedFile>, Box<dyn std::error::Error>> {
    let mut indexed_files = Vec::new();

    let mut dir_entries = tokio::fs::read_dir(dir).await.map_err(|e| {
        let msg = format!("Failed to read dir: {:?}, error: {:?}", dir, e);
        error!("{}", msg);
        e
    })?;

    while let Some(entry) = dir_entries.next_entry().await? {
        let path = entry.path();

        let index = extract_index_from_filename(&path);
        if let Some(index) = index {
            indexed_files.push(IndexedFile { index, path });
        } else {
            // As 0 if no index found
            indexed_files.push(IndexedFile { index: 0, path });
        }
    }

    // Check if there is any duplicated index
    let mut index_set = std::collections::HashSet::new();
    for file in &indexed_files {
        if !index_set.insert(file.index) {
            let msg = format!("Duplicated index found: {} {:?}", file.index, file.path);
            error!("{}", msg);
            return Err(msg.into());
        }
    }

    // Sort by index
    indexed_files.sort_by_key(|file| file.index);

    Ok(indexed_files)
}

// File name format: for file like name.1.json, name.2.toml, for dir like dir.3, etc.
fn extract_index_from_filename(path: &Path) -> Option<u32> {
    let file_stem = if path.is_file() {
        path.file_stem()?.to_str()?
    } else {
        path.file_name()?.to_str()?
    };

    let index_part = file_stem.rsplit('.').next()?; 
    
    index_part.parse::<u32>().ok()
}

async fn load_dir_without_root(dir: &Path) -> Result<Vec<ConfigItem>, Box<dyn std::error::Error>> {
    let indexed_files = scan_files(dir).await?;

    debug!("Indexed files: {:?} in {:?}", indexed_files, dir);

    let mut config = Vec::new();
    for file in indexed_files {
        if file.path.is_file() {
            let value = load_file(&file.path).await?;
            config.push(ConfigItem {
                path: file.path,
                value,
            });
        } else {
            // Recursively load sub dir
            let items = load_dir(&file.path).await?;
            config.extend(items);
        }
    }

    Ok(config)
}

#[async_recursion::async_recursion]
pub async fn load_dir(dir: &Path) -> Result<Vec<ConfigItem>, Box<dyn std::error::Error>> {
    // First try load root file in dir, maybe end with .json or .toml
    let root_file = get_root_file(dir);

    match root_file {
        Some(root_file) => load_dir_with_root(dir, &root_file).await,
        None => load_dir_without_root(dir).await,
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn test_file_index() {
        // Test extract_index_from_filename
        let path = std::path::Path::new("name.1.json");
        let index = super::extract_index_from_filename(path);
        assert_eq!(index, Some(1));

        let path = std::path::Path::new("name.2.toml");
        let index = super::extract_index_from_filename(path);
        assert_eq!(index, Some(2));

        let path = std::path::Path::new("name.json");
        let index = super::extract_index_from_filename(path);
        assert_eq!(index, None);

        let path = std::path::Path::new("dir.1");
        let index = super::extract_index_from_filename(path);
        assert_eq!(index, Some(1));
    }
}