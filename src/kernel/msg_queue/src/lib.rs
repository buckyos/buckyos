mod error;
mod types;
mod backend;
mod nats_backend;

pub use error::MsgQueueError;
pub use types::{Message, MessageReply, QueueConfig, QueueStats};
pub use backend::MsgQueueBackend;
pub use nats_backend::NatsBackend;

use std::sync::Arc;
pub struct MsgQueueService {
    backend: Arc<dyn MsgQueueBackend>,
}

impl MsgQueueService {
    pub fn new(backend: Arc<dyn MsgQueueBackend>) -> Self {
        Self { backend }
    }
    
    pub async fn create_queue(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError> {
        self.backend.create_queue(queue_id, config).await
    }
    
    pub async fn delete_queue(&self, queue_id: &str) -> Result<(), MsgQueueError> {
        self.backend.delete_queue(queue_id).await
    }
    
    pub async fn post_message(&self, queue_id: &str, message: Message) -> Result<(), MsgQueueError> {
        self.backend.post_message(queue_id, message).await
    }
    
    pub async fn pop_message(&self, queue_id: &str) -> Result<Option<Message>, MsgQueueError> {
        self.backend.pop_message(queue_id).await
    }
    
    pub async fn get_message_reply(&self, message_id: &str) -> Result<Option<MessageReply>, MsgQueueError> {
        self.backend.get_message_reply(message_id).await
    }
    
    pub async fn reply_to_message(&self, reply: MessageReply) -> Result<(), MsgQueueError> {
        self.backend.reply_to_message(reply).await
    }
    
    pub async fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats, MsgQueueError> {
        self.backend.get_queue_stats(queue_id).await
    }
    
    pub async fn update_queue_config(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError> {
        self.backend.update_queue_config(queue_id, config).await
    }
} 