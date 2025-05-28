
use std::sync::Arc;
use std::result::Result;
use std::net::IpAddr;
use serde::{Serialize, Deserialize};
use async_trait::async_trait;
use uuid::Uuid;

use ::kRPC::*;
use crate::error::MsgQueueError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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


#[async_trait]
pub trait MsgQueueBackend: Send + Sync {
    async fn create_queue(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError>;
    async fn delete_queue(&self, queue_id: &str) -> Result<(), MsgQueueError>;
    async fn post_message(&self, queue_id: &str, message: Message) -> Result<(), MsgQueueError>;
    async fn pop_message(&self, queue_id: &str) -> Result<Option<Message>, MsgQueueError>;
    async fn get_message_reply(&self, message_id: &str) -> Result<Option<MessageReply>, MsgQueueError>;
    async fn reply_message(&self, reply: MessageReply) -> Result<(), MsgQueueError>;
    async fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats, MsgQueueError>;
    async fn update_queue_config(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError>;
} 

#[derive(Clone)]
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

    pub async fn handle_create_queue(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
    }
    
    pub async fn delete_queue(&self, queue_id: &str) -> Result<(), MsgQueueError> {
        self.backend.delete_queue(queue_id).await
    }

    pub async fn handle_delete_queue(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
    }
    
    pub async fn post_message(&self, queue_id: &str, message: Message) -> Result<(), MsgQueueError> {
        self.backend.post_message(queue_id, message).await
    }

    pub async fn handle_post_message(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
    }
    
    pub async fn pop_message(&self, queue_id: &str) -> Result<Option<Message>, MsgQueueError> {
        self.backend.pop_message(queue_id).await
    }

    pub async fn handle_pop_message(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
    }
    
    
    pub async fn get_message_reply(&self, message_id: &str) -> Result<Option<MessageReply>, MsgQueueError> {
        self.backend.get_message_reply(message_id).await
    }

    pub async fn handle_get_message_reply(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
    }
    
    pub async fn reply_message(&self, reply: MessageReply) -> Result<(), MsgQueueError> {
        self.backend.reply_message(reply).await
    }
    pub async fn handle_reply_message(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
    }
    
    pub async fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats, MsgQueueError> {
        self.backend.get_queue_stats(queue_id).await
    }

    pub async fn handle_get_queue_stats(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
    }
    
    pub async fn update_queue_config(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError> {
        self.backend.update_queue_config(queue_id, config).await
    }
}
#[async_trait]
impl InnerServiceHandler for MsgQueueService {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            "create_queue" => self.handle_create_queue(req).await,
            "delete_queue" => self.handle_delete_queue(req).await,
            "post_message" => self.handle_post_message(req).await,
            "pop_message" => self.handle_pop_message(req).await,
            "reply_message" => self.handle_reply_message(req).await,
            "get_message_reply" => self.handle_get_message_reply(req).await,
            "get_queue_stats" => self.handle_get_queue_stats(req).await,
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }

    async fn handle_http_get(&self, req_path: &str, _ip_from: IpAddr) -> Result<String, RPCErrors> {
        Err(RPCErrors::UnknownMethod(req_path.to_string()))
    }
}