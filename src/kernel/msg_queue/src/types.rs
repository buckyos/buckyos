use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    pub max_size: usize,
    pub persistence: bool,
    pub retention_period: Option<u64>, // in seconds
    pub max_message_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub queue_id: String,
    pub content: Vec<u8>,
    pub timestamp: u64,
    pub reply_to: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageReply {
    pub message_id: Uuid,
    pub result: Vec<u8>,
    pub timestamp: u64,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStats {
    pub queue_id: String,
    pub message_count: usize,
    pub max_size: usize,
    pub used_storage: usize,
    pub created_at: u64,
    pub last_accessed: u64,
} 