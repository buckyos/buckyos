use serde::{Deserialize, Serialize};
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
    async fn query_logs(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String>;
}

pub type LogStorageRef = Arc<Box<dyn LogStorage>>;

pub struct LogQueryRequest {
    pub node: Option<String>,
    pub service: Option<String>,
    pub level: Option<slog::LogLevel>,
    pub start_time: Option<u64>,
    pub end_time: Option<u64>,
    pub limit: Option<usize>,
}
