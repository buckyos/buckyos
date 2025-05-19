use thiserror::Error;

#[derive(Error, Debug)]
pub enum MsgQueueError {
    #[error("Queue not found: {0}")]
    QueueNotFound(String),
    
    #[error("Queue already exists: {0}")]
    QueueExists(String),
    
    #[error("Queue is full: {0}")]
    QueueFull(String),
    
    #[error("Message not found: {0}")]
    MessageNotFound(String),
    
    #[error("Backend error: {0}")]
    BackendError(#[from] anyhow::Error),
    
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
} 