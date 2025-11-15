use serde::{Serialize, Deserialize};
use slog::SystemLogRecord;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogRecords {
    pub node: String,
    pub service: String,
    pub logs: Vec<SystemLogRecord>,
}


#[async_trait::async_trait]
pub trait LogStorage: Sync + Send {
    async fn append_logs(&self, records: LogRecords) -> Result<(), String>;
}

pub type LogStorageRef = Arc<Box<dyn LogStorage>>;