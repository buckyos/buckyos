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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::Uuid;

    async fn setup_test_service() -> MsgQueueService {
        let backend = NatsBackend::new("nats://localhost:4222")
            .await
            .expect("Failed to create NATS backend");
        MsgQueueService::new(Arc::new(backend))
    }

    fn create_test_message(queue_id: &str) -> Message {
        Message {
            id: Uuid::new_v4(),
            queue_id: queue_id.to_string(),
            content: b"test message".to_vec(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            reply_to: None,
            metadata: None,
        }
    }

    fn create_test_config() -> QueueConfig {
        QueueConfig {
            max_size: 1000,
            persistence: true,
            retention_period: Some(3600),
            max_message_size: Some(1024),
        }
    }

    #[tokio::test]
    async fn test_queue_lifecycle() {
        let service = setup_test_service().await;
        let queue_id = "test-queue";
        let config = create_test_config();

        // Test queue creation
        service.create_queue(queue_id, config.clone())
            .await
            .expect("Failed to create queue");

        // Test queue stats
        let stats = service.get_queue_stats(queue_id)
            .await
            .expect("Failed to get queue stats");
        assert_eq!(stats.queue_id, queue_id);
        assert_eq!(stats.max_size, config.max_size);

        // Test message posting
        let message = create_test_message(queue_id);
        service.post_message(queue_id, message.clone())
            .await
            .expect("Failed to post message");

        // Test message popping
        let popped = service.pop_message(queue_id)
            .await
            .expect("Failed to pop message")
            .expect("No message found");
        assert_eq!(popped.id, message.id);

        // Test queue deletion
        service.delete_queue(queue_id)
            .await
            .expect("Failed to delete queue");
    }

    #[tokio::test]
    async fn test_message_reply() {
        let service = setup_test_service().await;
        let queue_id = "reply-test-queue";
        let config = create_test_config();

        // Setup
        service.create_queue(queue_id, config)
            .await
            .expect("Failed to create queue");

        // Create and post message
        let message = create_test_message(queue_id);
        service.post_message(queue_id, message.clone())
            .await
            .expect("Failed to post message");

        // Create and send reply
        let reply = MessageReply {
            message_id: message.id,
            result: b"reply content".to_vec(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            metadata: None,
        };

        service.reply_to_message(reply.clone())
            .await
            .expect("Failed to send reply");

        // Get reply
        let received_reply = service.get_message_reply(&message.id.to_string())
            .await
            .expect("Failed to get reply")
            .expect("No reply found");

        assert_eq!(received_reply.message_id, reply.message_id);
        assert_eq!(received_reply.result, reply.result);

        // Cleanup
        service.delete_queue(queue_id)
            .await
            .expect("Failed to delete queue");
    }

    // #[tokio::test]
    // async fn test_queue_config_update() {
    //     let service = setup_test_service().await;
    //     let queue_id = "config-test-queue";
    //     let initial_config = create_test_config();

    //     // Create queue with initial config
    //     service.create_queue(queue_id, initial_config.clone())
    //         .await
    //         .expect("Failed to create queue");

    //     // Update config
    //     let updated_config = QueueConfig {
    //         max_size: 2000,
    //         persistence: true,
    //         retention_period: Some(7200),
    //         max_message_size: Some(2048),
    //     };

    //     service.update_queue_config(queue_id, updated_config.clone())
    //         .await
    //         .expect("Failed to update queue config");

    //     // Verify updated config
    //     let stats = service.get_queue_stats(queue_id)
    //         .await
    //         .expect("Failed to get queue stats");
    //     assert_eq!(stats.max_size, updated_config.max_size);

    //     // Cleanup
    //     service.delete_queue(queue_id)
    //         .await
    //         .expect("Failed to delete queue");
    // }
}
