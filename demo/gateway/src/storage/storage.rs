use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use std::sync::Arc;

use gateway_lib::*;



#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    async fn load(&self) -> GatewayResult<Option<serde_json::Value>>;
    async fn save(&self, config: &serde_json::Value) -> GatewayResult<()>;
}

pub type StorageRef = Arc<Box<dyn Storage>>;

pub struct FileStorage {
    pub local_file: PathBuf,
}

impl FileStorage {
    pub fn new(local_file: PathBuf) -> Self {
        Self { local_file }
    }
}

#[async_trait::async_trait]
impl Storage for FileStorage {
    async fn load(&self) -> GatewayResult<Option<serde_json::Value>> {
        if !self.local_file.exists() {
            info!("Config file not found: {}", self.local_file.display());
            return Ok(None);
        }

        let file = File::open(&self.local_file).await.map_err(|e| {
            let msg = format!("Error opening file {}: {}", self.local_file.display(), e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        let mut reader = BufReader::new(file);
        let mut content = String::new();
        reader.read_to_string(&mut content).await.map_err(|e| {
            let msg = format!("Error reading file {}: {}", self.local_file.display(), e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        let content = content.trim();
        if content.is_empty() {
            warn!("Empty config file: {}", self.local_file.display());
            return Ok(None);
        }

        let config = serde_json::from_str(&content).map_err(|e| {
            let msg = format!(
                "Error parsing config file {}: {}",
                self.local_file.display(),
                e
            );
            error!("{}", msg);
            GatewayError::InvalidConfig(msg)
        })?;


        Ok(config)
    }

    async fn save(&self, config: &serde_json::Value) -> GatewayResult<()> {
        let file = File::create(&self.local_file).await.map_err(|e| {
            let msg = format!("Error creating file {}: {}", self.local_file.display(), e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        let content = serde_json::to_string_pretty(&config).map_err(|e| {
            let msg = format!("Error serializing config: {}", e);
            error!("{}", msg);
            GatewayError::InvalidConfig(msg)
        })?;

        let mut writer = BufWriter::new(file);
        writer.write_all(content.as_bytes()).await.map_err(|e| {
            let msg = format!("Error writing file {}: {}", self.local_file.display(), e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;
    
        writer.flush().await.map_err(|e| {
            let msg = format!("Error flushing file {}: {}", self.local_file.display(), e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        Ok(())
    }
}


pub fn get_data_dir() -> PathBuf {
    let mut data_dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    data_dir.push("cyfs");
    data_dir.push("gateway");

    if !data_dir.exists() {
        std::fs::create_dir_all(&data_dir).unwrap();
    }

    data_dir
}

pub fn default_file_storage() -> StorageRef {
    let data_dir = get_data_dir();
    let local_file = data_dir.join("config.json");

    Arc::new(Box::new(FileStorage::new(local_file)))
}