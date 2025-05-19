use async_trait::async_trait;
use crate::error::MsgQueueError;
use crate::types::{Message, MessageReply, QueueConfig, QueueStats};

#[async_trait]
pub trait MsgQueueBackend: Send + Sync {
    async fn create_queue(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError>;
    
    async fn delete_queue(&self, queue_id: &str) -> Result<(), MsgQueueError>;
    
    async fn post_message(&self, queue_id: &str, message: Message) -> Result<(), MsgQueueError>;
    
    async fn pop_message(&self, queue_id: &str) -> Result<Option<Message>, MsgQueueError>;
    
    async fn get_message_reply(&self, message_id: &str) -> Result<Option<MessageReply>, MsgQueueError>;
    
    async fn reply_to_message(&self, reply: MessageReply) -> Result<(), MsgQueueError>;
    
    async fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats, MsgQueueError>;
    
    async fn update_queue_config(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError>;
} 