
use std::sync::Arc;
use std::result::Result;
use std::net::IpAddr;
use serde::{Serialize, Deserialize};
use async_trait::async_trait;
use uuid::Uuid;
use serde_json::*;

use ::kRPC::*;
use buckyos_api::*;

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
        let queue_id = req.get_str_param("queue_id")?;
        if queue_id.contains(":") {
            return Err(RPCErrors::ParseRequestError("queue_id is invalid".to_string()));
        }
        let config = req.params.get("config");
        if config.is_none() {
            return Err(RPCErrors::ParseRequestError("config is required".to_string()));
        }
        let config = config.unwrap();
        let config = serde_json::from_value(config.clone())
            .map_err(|_| RPCErrors::ParseRequestError("invalid config".to_string()))?;
        let runtime = get_buckyos_api_runtime()?;
        //TODO:msg_queue的权限应该是和workspace绑定比较好
        let (user_id,app_id) = runtime.enforce(&req, "write", "dfs://system/msg_queue").await?;
        let real_queue_id = format!("{}:{}",  app_id, queue_id);
        //TODO：判断应用是否有足够的 资源配额 ，来创建queue
        self.create_queue(&real_queue_id, config).await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code": "0",
            "message": format!("queue {} created", queue_id)
        })),
            req.id,
        ))
    }
    
    pub async fn delete_queue(&self, queue_id: &str) -> Result<(), MsgQueueError> {
        self.backend.delete_queue(queue_id).await
    }

    pub async fn handle_delete_queue(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let queue_id = req.get_str_param("queue_id")?;
        if queue_id.contains(":") {
            return Err(RPCErrors::ParseRequestError("queue_id is invalid".to_string()));
        }
        let runtime = get_buckyos_api_runtime()?;
        let (user_id,app_id) = runtime.enforce(&req, "write", "dfs://system/msg_queue").await?;
        let real_queue_id = format!("{}:{}",  app_id, queue_id);
        self.delete_queue(&real_queue_id).await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code": "0",
            "message": format!("queue {} deleted", queue_id)
        })),
            req.id,
        ))
    }
    
    pub async fn post_message(&self, queue_id: &str, message: Message) -> Result<(), MsgQueueError> {
        self.backend.post_message(queue_id, message).await
    }

    pub async fn handle_post_message(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let msg = req.params.get("message");
        if msg.is_none() {
            return Err(RPCErrors::ParseRequestError("message is required".to_string()));
        }
        let msg = msg.unwrap();
        let msg:Message = serde_json::from_value(msg.clone())
            .map_err(|_| RPCErrors::ParseRequestError("invalid message".to_string()))?;

        let queue_id = msg.queue_id.clone();
        if queue_id.contains(":") {
            return Err(RPCErrors::ParseRequestError("queue_id is invalid".to_string()));
        }
        let runtime = get_buckyos_api_runtime()?;
        let (user_id,app_id) = runtime.enforce(&req, "write", "dfs://system/msg_queue").await?;
        let real_queue_id = format!("{}:{}",  app_id, queue_id);
        let msg_id = msg.id.clone();
        self.post_message(&real_queue_id, msg).await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code": "0",
            "message": format!("message {} posted", msg_id)
        })),
            req.id,
        ))
  
    }
    
    pub async fn pop_message(&self, queue_id: &str) -> Result<Option<Message>, MsgQueueError> {
        self.backend.pop_message(queue_id).await
    }

    pub async fn handle_pop_message(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let queue_id = req.get_str_param("queue_id")?;
        if queue_id.contains(":") {
            return Err(RPCErrors::ParseRequestError("queue_id is invalid".to_string()));
        }
        let runtime = get_buckyos_api_runtime()?;
        let (user_id,app_id) = runtime.enforce(&req, "read", "dfs://system/msg_queue").await?;
        let real_queue_id = format!("{}:{}",  app_id, queue_id);

        let msg = self.pop_message(&real_queue_id).await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        
        let msg_value = serde_json::to_value(msg).unwrap();

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code": "0",
            "message": msg_value
        })),
            req.id,
        ))
    }
    
    pub async fn get_message_reply(&self, message_id: &str) -> Result<Option<MessageReply>, MsgQueueError> {
        self.backend.get_message_reply(message_id).await
    }

    pub async fn handle_get_message_reply(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let message_id = req.get_str_param("message_id")?;
        let reply = self.get_message_reply(&message_id).await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        if reply.is_none() {
            return Err(RPCErrors::ParseRequestError("reply is not found".to_string()));
        }

        let reply_value = serde_json::to_value(reply).unwrap();
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code": "0",
            "reply": reply_value
        })),
            req.id,
        ))

    }
    
    pub async fn reply_message(&self, reply: MessageReply) -> Result<(), MsgQueueError> {
        self.backend.reply_message(reply).await
    }

    pub async fn handle_reply_message(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        //TODO:如何处理权限问题?
        let reply = req.params.get("reply");
        if reply.is_none() {
            return Err(RPCErrors::ParseRequestError("reply is required".to_string()));
        }
        let reply = reply.unwrap();
        let reply:MessageReply = serde_json::from_value(reply.clone())
            .map_err(|_| RPCErrors::ParseRequestError("invalid reply".to_string()))?;

        let msg_id = reply.message_id.clone();
        self.reply_message(reply).await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code": "0",
            "message": format!("reply {} posted", msg_id)
        })),
            req.id,
        ))
    }
    
    pub async fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats, MsgQueueError> {
        self.backend.get_queue_stats(queue_id).await
    }

    pub async fn handle_get_queue_stats(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let queue_id = req.get_str_param("queue_id")?;
        if queue_id.contains(":") {
            return Err(RPCErrors::ParseRequestError("queue_id is invalid".to_string()));
        }
        let runtime = get_buckyos_api_runtime()?;
        let (user_id,app_id) = runtime.enforce(&req, "read", "dfs://system/msg_queue").await?;
        let real_queue_id = format!("{}:{}",  app_id, queue_id);
        let stats = self.get_queue_stats(&real_queue_id).await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let stats_value = serde_json::to_value(stats).unwrap();

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "code": "0",
            "stats": stats_value
        })),
            req.id,
        ))
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