use super::dir::load_dir;
use super::dir::load_dir_with_root;
use json_value_merge::Merge;
use serde_json::Value;
use std::path::Path;

pub struct ConfigMerger {}

impl ConfigMerger {
    pub async fn load_dir(dir: &Path) -> Result<Value, Box<dyn std::error::Error>> {
        info!("Loading config files from directory: {:?}", dir);

        // First load all config files in the directory
        let configs = load_dir(dir).await?;

        info!("Loaded {} config files: {:?}", configs.len(), configs);

        // Merge all config files to first one
        let mut merged = configs[0].clone();
        for config in configs.iter().skip(1) {
            info!("Will merge config: {:?} -> {:?}", config.path, merged.path);
            merged.value.merge(&config.value);
        }

        Ok(merged.value)
    }

    pub async fn load_dir_with_root(dir: &Path, root_file: &Path) -> Result<Value, Box<dyn std::error::Error>> {
        info!("Loading config files from directory: {:?} with root file: {:?}", dir, root_file);

        let configs = load_dir_with_root(dir, root_file).await?;

        info!("Loaded {} config files: {:?}", configs.len(), configs);

        if configs.len() == 1 {
            return Ok(configs[0].value.clone());
        }

        let mut merged = configs[0].clone();
        for config in configs.iter().skip(1) {
            info!("Will merge config: {:?} -> {:?}", config.path, merged.path);
            merged.value.merge(&config.value);
        }

        //merged.value.merge(&root.value);

        Ok(merged.value)
    }

    pub async fn load_config<T>(dir: &Path) -> Result<T, Box<dyn std::error::Error>>
    where
        T: serde::de::DeserializeOwned,
    {
        let value = Self::load_dir(dir).await?;
        let config: T = serde_json::from_value(value).map_err(|e| {
            let msg = format!("Failed to parse config: {:?}", e);
            error!("{}", msg);
            msg
        })?;
        Ok(config)
    }
}
