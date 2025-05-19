use async_nats::jetstream::Context;
use async_nats::Client;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::backend::MsgQueueBackend;
use crate::error::MsgQueueError;
use crate::types::{Message, MessageReply, QueueConfig, QueueStats};

pub struct NatsBackend {
    client: Arc<Client>,
    js: Arc<Context>,
    queue_configs: Arc<RwLock<std::collections::HashMap<String, QueueConfig>>>,
}

impl NatsBackend {
    pub async fn new(nats_url: &str) -> Result<Self, MsgQueueError> {
        let client = async_nats::connect(nats_url)
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
        
        let js = async_nats::jetstream::new(client.clone());
        
        Ok(Self {
            client: Arc::new(client),
            js: Arc::new(js),
            queue_configs: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }
}

#[async_trait]
impl MsgQueueBackend for NatsBackend {
    async fn create_queue(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError> {
        let mut configs = self.queue_configs.write().await;
        if configs.contains_key(queue_id) {
            return Err(MsgQueueError::QueueExists(queue_id.to_string()));
        }
        
        // Create JetStream stream for the queue
        let stream = self.js
            .create_stream(async_nats::jetstream::stream::Config {
                name: queue_id.to_string(),
                subjects: vec![format!("queue.{}.*", queue_id)],
                storage: if config.persistence {
                    async_nats::jetstream::stream::StorageType::File
                } else {
                    async_nats::jetstream::stream::StorageType::Memory
                },
                max_age: config.retention_period.map(|s| std::time::Duration::from_secs(s)).unwrap_or(std::time::Duration::from_secs(0)),
                max_bytes: config.max_size as i64,
                ..Default::default()
            })
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        configs.insert(queue_id.to_string(), config);
        Ok(())
    }
    
    async fn delete_queue(&self, queue_id: &str) -> Result<(), MsgQueueError> {
        self.js
            .delete_stream(queue_id)
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        let mut configs = self.queue_configs.write().await;
        configs.remove(queue_id);
        Ok(())
    }
    
    async fn post_message(&self, queue_id: &str, message: Message) -> Result<(), MsgQueueError> {
        let configs = self.queue_configs.read().await;
        let config = configs.get(queue_id)
            .ok_or_else(|| MsgQueueError::QueueNotFound(queue_id.to_string()))?;
            
        // Check queue size
        let stats = self.get_queue_stats(queue_id).await?;
        if stats.message_count >= config.max_size {
            return Err(MsgQueueError::QueueFull(queue_id.to_string()));
        }
        
        // Publish message
        let subject = format!("queue.{}.msg", queue_id);
        let payload = serde_json::to_vec(&message)
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        self.js
            .publish(subject, payload.into())
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        Ok(())
    }
    
    async fn pop_message(&self, queue_id: &str) -> Result<Option<Message>, MsgQueueError> {
        let stream = self.js
            .get_stream(queue_id)
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        let consumer = stream
            .create_consumer(async_nats::jetstream::consumer::pull::Config {
                durable_name: Some(format!("consumer-{}", Uuid::new_v4())),
                ..Default::default()
            })
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;

        unimplemented!()
            
        // let messages = consumer
        //     .fetch()
        //     .await
        //     .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        // if let Some(msg) = messages.first() {
        //     let message: Message = serde_json::from_slice(&msg.payload)
        //         .map_err(|e| MsgQueueError::BackendError(e.into()))?;
        //     msg.ack()
        //         .await
        //         .map_err(|e| MsgQueueError::BackendError(e.into()))?;
        //     Ok(Some(message))
        // } else {
        //     Ok(None)
        // }
    }
    
    async fn get_message_reply(&self, message_id: &str) -> Result<Option<MessageReply>, MsgQueueError> {
        let subject = format!("reply.{}", message_id);
        let subscription = self.client
            .subscribe(subject)
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        // if let Some(msg) = subscription.next().await {
        //     let reply: MessageReply = serde_json::from_slice(&msg.payload)
        //         .map_err(|e| MsgQueueError::BackendError(e.into()))?;
        //     Ok(Some(reply))
        // } else {
        //     Ok(None)
        // }

        unimplemented!()
    }
    
    async fn reply_to_message(&self, reply: MessageReply) -> Result<(), MsgQueueError> {
        let subject = format!("reply.{}", reply.message_id);
        let payload = serde_json::to_vec(&reply)
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        self.client
            .publish(subject, payload.into())
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        Ok(())
    }
    
    async fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats, MsgQueueError> {
        let mut stream = self.js
            .get_stream(queue_id)
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        let info = stream
            .info()
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        let configs = self.queue_configs.read().await;
        let config = configs.get(queue_id)
            .ok_or_else(|| MsgQueueError::QueueNotFound(queue_id.to_string()))?;
            
        Ok(QueueStats {
            queue_id: queue_id.to_string(),
            message_count: info.state.messages as usize,
            max_size: config.max_size,
            used_storage: info.state.bytes as usize,
            created_at: info.created.unix_timestamp() as u64,
            last_accessed: info.state.last_sequence as u64,
        })
    }
    
    async fn update_queue_config(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError> {
        let mut configs = self.queue_configs.write().await;
        if !configs.contains_key(queue_id) {
            return Err(MsgQueueError::QueueNotFound(queue_id.to_string()));
        }
        
        // Update stream configuration
        self.js
            .update_stream(async_nats::jetstream::stream::Config {
                name: queue_id.to_string(),
                storage: if config.persistence {
                    async_nats::jetstream::stream::StorageType::File
                } else {
                    async_nats::jetstream::stream::StorageType::Memory
                },
                max_age: config.retention_period.map(|s| std::time::Duration::from_secs(s)).unwrap_or(std::time::Duration::from_secs(0)),
                max_bytes: config.max_size as i64,
                ..Default::default()
            })
            .await
            .map_err(|e| MsgQueueError::BackendError(e.into()))?;
            
        configs.insert(queue_id.to_string(), config);
        Ok(())
    }
} 