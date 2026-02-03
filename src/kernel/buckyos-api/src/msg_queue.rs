use ::kRPC::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;
use thiserror::Error;

const KMSG_SERVICE_NAME: &str = "kmsg";
pub const KMSG_SERVICE_MAIN_PORT: u16 = 4030;

/// 全局唯一的队列标识符 (Uniform Resource Name)
/// 格式通常为: "appid::owner::name"
pub type QueueUrn = String;

/// 消息索引，单调递增
pub type MsgIndex = u64;

/// 订阅者 ID
pub type SubscriptionId = String;


/// 消息体定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// 全局唯一索引 (Log Index)
    pub index: MsgIndex,
    /// 消息产生时间 (Unix Timestamp)
    pub created_at: u64,
    /// 业务负载
    pub payload: Vec<u8>,
    /// 可选的元数据 (Headers/Tags)
    pub headers: HashMap<String, String>,
}

impl Message {
    pub fn new(payload: Vec<u8>) -> Self {
        Self {
            index: 0,
            created_at: 0,
            payload,
            headers: HashMap::new(),
        }
    }
}

/// 队列配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    /// 消息最大保留条数 (0 表示不限制)
    pub max_messages: Option<u64>,
    /// 消息过期时间 (秒)
    pub retention_seconds: Option<u64>,
    /// 是否需要同步落盘 (Write-Ahead-Log 语义)
    pub sync_write: bool,

    pub other_app_can_read: bool,  
    pub other_app_can_write: bool,
    pub other_user_can_read: bool,
    pub other_user_can_write: bool,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_messages: None,
            retention_seconds: None,
            sync_write: false,
            other_app_can_read: true,
            other_app_can_write: false,
            other_user_can_read: false,
            other_user_can_write: false,
        }
    }
}

/// 队列状态统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueStats {
    pub message_count: u64,
    pub first_index: u64,
    pub last_index: u64,
    pub size_bytes: u64,
}

/// 订阅起始位置
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SubPosition {
    /// 从最早可用的消息开始
    Earliest,
    /// 从最新的消息之后开始（只读新消息）
    Latest,
    /// 从指定索引开始
    At(MsgIndex),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueCreateQueueReq {
    pub name: Option<String>,
    pub appid: String,
    pub app_owner: String,
    pub config: QueueConfig,
}

impl MsgQueueCreateQueueReq {
    pub fn new(
        name: Option<String>,
        appid: String,
        app_owner: String,
        config: QueueConfig,
    ) -> Self {
        Self {
            name,
            appid,
            app_owner,
            config,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueCreateQueueReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueDeleteQueueReq {
    pub queue_urn: QueueUrn,
}

impl MsgQueueDeleteQueueReq {
    pub fn new(queue_urn: QueueUrn) -> Self {
        Self { queue_urn }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueDeleteQueueReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueGetQueueStatsReq {
    pub queue_urn: QueueUrn,
}

impl MsgQueueGetQueueStatsReq {
    pub fn new(queue_urn: QueueUrn) -> Self {
        Self { queue_urn }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueGetQueueStatsReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueUpdateQueueConfigReq {
    pub queue_urn: QueueUrn,
    pub config: QueueConfig,
}

impl MsgQueueUpdateQueueConfigReq {
    pub fn new(queue_urn: QueueUrn, config: QueueConfig) -> Self {
        Self { queue_urn, config }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueUpdateQueueConfigReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueuePostMessageReq {
    pub queue_urn: QueueUrn,
    pub message: Message,
}

impl MsgQueuePostMessageReq {
    pub fn new(queue_urn: QueueUrn, message: Message) -> Self {
        Self { queue_urn, message }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueuePostMessageReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueSubscribeReq {
    pub queue_urn: QueueUrn,
    #[serde(rename = "userid", alias = "user_id")]
    pub user_id: String,
    #[serde(rename = "appid", alias = "app_id")]
    pub app_id: String,
    pub sub_id: Option<String>,
    pub position: SubPosition,
}

impl MsgQueueSubscribeReq {
    pub fn new(
        queue_urn: QueueUrn,
        user_id: String,
        app_id: String,
        sub_id: Option<String>,
        position: SubPosition,
    ) -> Self {
        Self {
            queue_urn,
            user_id,
            app_id,
            sub_id,
            position,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueSubscribeReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueUnsubscribeReq {
    pub sub_id: SubscriptionId,
}

impl MsgQueueUnsubscribeReq {
    pub fn new(sub_id: SubscriptionId) -> Self {
        Self { sub_id }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueUnsubscribeReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueFetchMessagesReq {
    pub sub_id: SubscriptionId,
    pub length: usize,
    pub auto_commit: bool,
}

impl MsgQueueFetchMessagesReq {
    pub fn new(sub_id: SubscriptionId, length: usize, auto_commit: bool) -> Self {
        Self {
            sub_id,
            length,
            auto_commit,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueFetchMessagesReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueCommitAckReq {
    pub sub_id: SubscriptionId,
    pub index: MsgIndex,
}

impl MsgQueueCommitAckReq {
    pub fn new(sub_id: SubscriptionId, index: MsgIndex) -> Self {
        Self { sub_id, index }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueCommitAckReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueSeekReq {
    pub sub_id: SubscriptionId,
    pub index: SubPosition,
}

impl MsgQueueSeekReq {
    pub fn new(sub_id: SubscriptionId, index: SubPosition) -> Self {
        Self { sub_id, index }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueSeekReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgQueueDeleteMessageBeforeReq {
    pub queue_urn: QueueUrn,
    pub index: MsgIndex,
}

impl MsgQueueDeleteMessageBeforeReq {
    pub fn new(queue_urn: QueueUrn, index: MsgIndex) -> Self {
        Self { queue_urn, index }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse MsgQueueDeleteMessageBeforeReq: {}",
                e
            ))
        })
    }
}

pub enum MsgQueueClient {
    InProcess(Box<dyn MsgQueueHandler>),
    KRPC(Box<kRPC>),
}

impl MsgQueueClient {
    pub fn new_in_process(handler: Box<dyn MsgQueueHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(client: Box<kRPC>) -> Self {
        Self::KRPC(client)
    }

    pub async fn set_context(&self, context: RPCContext)  {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await
            }
        }
    }

    pub async fn create_queue(
        &self,
        name: Option<&str>,
        appid: &str,
        app_owner: &str,
        config: QueueConfig,
    ) -> std::result::Result<QueueUrn, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_create_queue(name, appid, app_owner, config, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgQueueCreateQueueReq::new(
                    name.map(|value| value.to_string()),
                    appid.to_string(),
                    app_owner.to_string(),
                    config,
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueCreateQueueReq: {}",
                        e
                    ))
                })?;
                let result = client.call("create_queue", req_json).await?;
                result
                    .as_str()
                    .map(|value| value.to_string())
                    .ok_or_else(|| {
                        RPCErrors::ParserResponseError("Expected QueueUrn string".to_string())
                    })
            }
        }
    }

    pub async fn delete_queue(
        &self,
        queue_urn: &str,
    ) -> std::result::Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_delete_queue(queue_urn, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgQueueDeleteQueueReq::new(queue_urn.to_string());
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueDeleteQueueReq: {}",
                        e
                    ))
                })?;
                client.call("delete_queue", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn get_queue_stats(
        &self,
        queue_urn: &str,
    ) -> std::result::Result<QueueStats, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_get_queue_stats(queue_urn, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgQueueGetQueueStatsReq::new(queue_urn.to_string());
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueGetQueueStatsReq: {}",
                        e
                    ))
                })?;
                let result = client.call("get_queue_stats", req_json).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to deserialize QueueStats response: {}",
                        e
                    ))
                })
            }
        }
    }

    pub async fn update_queue_config(
        &self,
        queue_urn: &str,
        config: QueueConfig,
    ) -> std::result::Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_update_queue_config(queue_urn, config, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgQueueUpdateQueueConfigReq::new(queue_urn.to_string(), config);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueUpdateQueueConfigReq: {}",
                        e
                    ))
                })?;
                client.call("update_queue_config", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn post_message(
        &self,
        queue_urn: &str,
        message: Message,
    ) -> std::result::Result<MsgIndex, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_post_message(queue_urn, message, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgQueuePostMessageReq::new(queue_urn.to_string(), message);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueuePostMessageReq: {}",
                        e
                    ))
                })?;
                let result = client.call("post_message", req_json).await?;
                result.as_u64().ok_or_else(|| {
                    RPCErrors::ParserResponseError("Expected MsgIndex u64".to_string())
                })
            }
        }
    }

    pub async fn subscribe(
        &self,
        queue_urn: &str,
        user_id: &str,
        app_id: &str,
        sub_id: Option<String>,
        position: SubPosition,
    ) -> std::result::Result<SubscriptionId, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_subscribe(queue_urn, user_id, app_id, sub_id, position, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = MsgQueueSubscribeReq::new(
                    queue_urn.to_string(),
                    user_id.to_string(),
                    app_id.to_string(),
                    sub_id,
                    position,
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueSubscribeReq: {}",
                        e
                    ))
                })?;
                let result = client.call("subscribe", req_json).await?;
                result
                    .as_str()
                    .map(|value| value.to_string())
                    .ok_or_else(|| {
                        RPCErrors::ParserResponseError(
                            "Expected SubscriptionId string".to_string(),
                        )
                    })
            }
        }
    }

    pub async fn unsubscribe(
        &self,
        sub_id: &str,
    ) -> std::result::Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_unsubscribe(sub_id, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgQueueUnsubscribeReq::new(sub_id.to_string());
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueUnsubscribeReq: {}",
                        e
                    ))
                })?;
                client.call("unsubscribe", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn fetch_messages(
        &self,
        sub_id: &str,
        length: usize,
        auto_commit: bool,
    ) -> std::result::Result<Vec<Message>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_fetch_messages(sub_id, length, auto_commit, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req =
                    MsgQueueFetchMessagesReq::new(sub_id.to_string(), length, auto_commit);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueFetchMessagesReq: {}",
                        e
                    ))
                })?;
                let result = client.call("fetch_messages", req_json).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to deserialize Vec<Message> response: {}",
                        e
                    ))
                })
            }
        }
    }

    pub async fn commit_ack(
        &self,
        sub_id: &str,
        index: MsgIndex,
    ) -> std::result::Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_commit_ack(sub_id, index, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgQueueCommitAckReq::new(sub_id.to_string(), index);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueCommitAckReq: {}",
                        e
                    ))
                })?;
                client.call("commit_ack", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn seek(
        &self,
        sub_id: &str,
        index: SubPosition,
    ) -> std::result::Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_seek(sub_id, index, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgQueueSeekReq::new(sub_id.to_string(), index);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueSeekReq: {}",
                        e
                    ))
                })?;
                client.call("seek", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn delete_message_before(
        &self,
        queue_urn: &str,
        index: MsgIndex,
    ) -> std::result::Result<u64, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_delete_message_before(queue_urn, index, ctx).await
            }
            Self::KRPC(client) => {
                let req = MsgQueueDeleteMessageBeforeReq::new(queue_urn.to_string(), index);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize MsgQueueDeleteMessageBeforeReq: {}",
                        e
                    ))
                })?;
                let result = client.call("delete_message_before", req_json).await?;
                result.as_u64().ok_or_else(|| {
                    RPCErrors::ParserResponseError("Expected deleted count u64".to_string())
                })
            }
        }
    }
}

#[async_trait]
pub trait MsgQueueHandler: Send + Sync {
    async fn handle_create_queue(
        &self,
        name: Option<&str>,
        appid: &str,
        app_owner: &str,
        config: QueueConfig,
        ctx: RPCContext,
    ) -> std::result::Result<QueueUrn, RPCErrors>;

    async fn handle_delete_queue(
        &self,
        queue_urn: &str,
        ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors>;

    async fn handle_get_queue_stats(
        &self,
        queue_urn: &str,
        ctx: RPCContext,
    ) -> std::result::Result<QueueStats, RPCErrors>;

    async fn handle_update_queue_config(
        &self,
        queue_urn: &str,
        config: QueueConfig,
        ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors>;

    async fn handle_post_message(
        &self,
        queue_urn: &str,
        message: Message,
        ctx: RPCContext,
    ) -> std::result::Result<MsgIndex, RPCErrors>;

    async fn handle_subscribe(
        &self,
        queue_urn: &str,
        user_id: &str,
        app_id: &str,
        sub_id: Option<String>,
        position: SubPosition,
        ctx: RPCContext,
    ) -> std::result::Result<SubscriptionId, RPCErrors>;

    async fn handle_unsubscribe(
        &self,
        sub_id: &str,
        ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors>;

    async fn handle_fetch_messages(
        &self,
        sub_id: &str,
        length: usize,
        auto_commit: bool,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<Message>, RPCErrors>;

    async fn handle_commit_ack(
        &self,
        sub_id: &str,
        index: MsgIndex,
        ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors>;

    async fn handle_seek(
        &self,
        sub_id: &str,
        index: SubPosition,
        ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors>;

    async fn handle_delete_message_before(
        &self,
        queue_urn: &str,
        index: MsgIndex,
        ctx: RPCContext,
    ) -> std::result::Result<u64, RPCErrors>;
}

pub struct MsgQueueServerHandler<T: MsgQueueHandler>(pub T);

impl<T: MsgQueueHandler> MsgQueueServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: MsgQueueHandler> RPCHandler for MsgQueueServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            "create_queue" => {
                let create_req = MsgQueueCreateQueueReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_create_queue(
                        create_req.name.as_deref(),
                        &create_req.appid,
                        &create_req.app_owner,
                        create_req.config,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "delete_queue" => {
                let delete_req = MsgQueueDeleteQueueReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_delete_queue(&delete_req.queue_urn, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "get_queue_stats" => {
                let stats_req = MsgQueueGetQueueStatsReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_queue_stats(&stats_req.queue_urn, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "update_queue_config" => {
                let update_req = MsgQueueUpdateQueueConfigReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_queue_config(&update_req.queue_urn, update_req.config, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "post_message" => {
                let post_req = MsgQueuePostMessageReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_post_message(&post_req.queue_urn, post_req.message, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "subscribe" => {
                let subscribe_req = MsgQueueSubscribeReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_subscribe(
                        &subscribe_req.queue_urn,
                        &subscribe_req.user_id,
                        &subscribe_req.app_id,
                        subscribe_req.sub_id,
                        subscribe_req.position,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "unsubscribe" => {
                let unsubscribe_req = MsgQueueUnsubscribeReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_unsubscribe(&unsubscribe_req.sub_id, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "fetch_messages" => {
                let fetch_req = MsgQueueFetchMessagesReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_fetch_messages(
                        &fetch_req.sub_id,
                        fetch_req.length,
                        fetch_req.auto_commit,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "commit_ack" => {
                let commit_req = MsgQueueCommitAckReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_commit_ack(&commit_req.sub_id, commit_req.index, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "seek" => {
                let seek_req = MsgQueueSeekReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_seek(&seek_req.sub_id, seek_req.index, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "delete_message_before" => {
                let delete_req = MsgQueueDeleteMessageBeforeReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_delete_message_before(&delete_req.queue_urn, delete_req.index, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

/// 计算确定性的 Queue URN (Deterministic Naming)
/// 这是一个纯函数，不涉及 IO。
/// 格式: `appid::owner::name`
pub fn calc_queue_urn(appid: &str, app_owner: &str, name: &str) -> String {
    format!("{}::{}::{}", appid, app_owner, name)
}

/// 解析 URN 获取各个部分
pub fn parse_queue_urn(urn: &str) -> Option<(&str, &str, &str)> {
    let parts: Vec<&str> = urn.split("::").collect();
    if parts.len() == 3 {
        Some((parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::sync::Mutex;

    #[derive(Debug, Clone)]
    struct QueueState {
        config: QueueConfig,
        messages: Vec<Message>,
        next_index: MsgIndex,
    }

    #[derive(Debug, Clone)]
    struct SubscriptionState {
        queue_urn: QueueUrn,
        cursor: MsgIndex,
    }

    #[derive(Debug)]
    struct MockMsgQueue {
        queues: Mutex<HashMap<QueueUrn, QueueState>>,
        subscriptions: Mutex<HashMap<SubscriptionId, SubscriptionState>>,
        next_sub_id: AtomicU64,
        next_queue_id: AtomicU64,
    }

    impl MockMsgQueue {
        fn new() -> Self {
            Self {
                queues: Mutex::new(HashMap::new()),
                subscriptions: Mutex::new(HashMap::new()),
                next_sub_id: AtomicU64::new(1),
                next_queue_id: AtomicU64::new(1),
            }
        }

        fn next_subscription_id(&self) -> SubscriptionId {
            let id = self.next_sub_id.fetch_add(1, Ordering::SeqCst);
            format!("sub-{}", id)
        }

        fn next_queue_name(&self) -> String {
            let id = self.next_queue_id.fetch_add(1, Ordering::SeqCst);
            format!("queue-{}", id)
        }
    }

    #[async_trait]
    impl MsgQueueHandler for MockMsgQueue {
        async fn handle_create_queue(
            &self,
            name: Option<&str>,
            appid: &str,
            app_owner: &str,
            config: QueueConfig,
            _ctx: RPCContext,
        ) -> std::result::Result<QueueUrn, RPCErrors> {
            let name = name.map(|value| value.to_string()).unwrap_or_else(|| {
                self.next_queue_name()
            });
            let urn = calc_queue_urn(appid, app_owner, &name);
            let mut queues = self.queues.lock().await;
            if queues.contains_key(&urn) {
                return Err(RPCErrors::ReasonError(format!(
                    "Queue already exists: {}",
                    urn
                )));
            }
            queues.insert(
                urn.clone(),
                QueueState {
                    config,
                    messages: Vec::new(),
                    next_index: 1,
                },
            );
            Ok(urn)
        }

        async fn handle_delete_queue(
            &self,
            queue_urn: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut queues = self.queues.lock().await;
            if queues.remove(queue_urn).is_none() {
                return Err(RPCErrors::ReasonError(format!(
                    "Queue not found: {}",
                    queue_urn
                )));
            }
            drop(queues);

            let mut subs = self.subscriptions.lock().await;
            subs.retain(|_, sub| sub.queue_urn != queue_urn);
            Ok(())
        }

        async fn handle_get_queue_stats(
            &self,
            queue_urn: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<QueueStats, RPCErrors> {
            let queues = self.queues.lock().await;
            let queue = queues.get(queue_urn).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn))
            })?;
            let message_count = queue.messages.len() as u64;
            let first_index = queue.messages.first().map(|msg| msg.index).unwrap_or(0);
            let last_index = queue.messages.last().map(|msg| msg.index).unwrap_or(0);
            let size_bytes = queue
                .messages
                .iter()
                .map(|msg| msg.payload.len() as u64)
                .sum();

            Ok(QueueStats {
                message_count,
                first_index,
                last_index,
                size_bytes,
            })
        }

        async fn handle_update_queue_config(
            &self,
            queue_urn: &str,
            config: QueueConfig,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut queues = self.queues.lock().await;
            let queue = queues.get_mut(queue_urn).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn))
            })?;
            queue.config = config;
            Ok(())
        }

        async fn handle_post_message(
            &self,
            queue_urn: &str,
            mut message: Message,
            _ctx: RPCContext,
        ) -> std::result::Result<MsgIndex, RPCErrors> {
            let mut queues = self.queues.lock().await;
            let queue = queues.get_mut(queue_urn).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn))
            })?;
            let index = queue.next_index;
            queue.next_index += 1;
            message.index = index;
            queue.messages.push(message);
            Ok(index)
        }

        async fn handle_subscribe(
            &self,
            queue_urn: &str,
            _user_id: &str,
            _app_id: &str,
            sub_id: Option<String>,
            position: SubPosition,
            _ctx: RPCContext,
        ) -> std::result::Result<SubscriptionId, RPCErrors> {
            let queues = self.queues.lock().await;
            let queue = queues.get(queue_urn).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn))
            })?;
            let last_index = queue.messages.last().map(|msg| msg.index).unwrap_or(0);
            let first_index = queue
                .messages
                .first()
                .map(|msg| msg.index)
                .unwrap_or(1);
            let cursor = match position {
                SubPosition::Earliest => first_index,
                SubPosition::Latest => last_index + 1,
                SubPosition::At(index) => index,
            };
            drop(queues);

            let mut subs = self.subscriptions.lock().await;
            let sub_id = sub_id.unwrap_or_else(|| self.next_subscription_id());
            if subs.contains_key(&sub_id) {
                return Err(RPCErrors::ReasonError(format!(
                    "Subscription already exists: {}",
                    sub_id
                )));
            }
            subs.insert(
                sub_id.clone(),
                SubscriptionState {
                    queue_urn: queue_urn.to_string(),
                    cursor,
                },
            );
            Ok(sub_id)
        }

        async fn handle_unsubscribe(
            &self,
            sub_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut subs = self.subscriptions.lock().await;
            if subs.remove(sub_id).is_none() {
                return Err(RPCErrors::ReasonError(format!(
                    "Subscription not found: {}",
                    sub_id
                )));
            }
            Ok(())
        }

        async fn handle_fetch_messages(
            &self,
            sub_id: &str,
            length: usize,
            auto_commit: bool,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<Message>, RPCErrors> {
            let mut subs = self.subscriptions.lock().await;
            let sub = subs.get_mut(sub_id).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Subscription not found: {}", sub_id))
            })?;
            let queues = self.queues.lock().await;
            let queue = queues.get(&sub.queue_urn).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Queue not found: {}", sub.queue_urn))
            })?;

            let mut messages: Vec<Message> = queue
                .messages
                .iter()
                .filter(|msg| msg.index >= sub.cursor)
                .take(length)
                .cloned()
                .collect();

            if auto_commit {
                if let Some(last) = messages.last() {
                    sub.cursor = last.index + 1;
                }
            }

            Ok(messages)
        }

        async fn handle_commit_ack(
            &self,
            sub_id: &str,
            index: MsgIndex,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut subs = self.subscriptions.lock().await;
            let sub = subs.get_mut(sub_id).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Subscription not found: {}", sub_id))
            })?;
            sub.cursor = index + 1;
            Ok(())
        }

        async fn handle_seek(
            &self,
            sub_id: &str,
            index: SubPosition,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut subs = self.subscriptions.lock().await;
            let sub = subs.get_mut(sub_id).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Subscription not found: {}", sub_id))
            })?;
            let queues = self.queues.lock().await;
            let queue = queues.get(&sub.queue_urn).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Queue not found: {}", sub.queue_urn))
            })?;
            let last_index = queue.messages.last().map(|msg| msg.index).unwrap_or(0);
            let first_index = queue
                .messages
                .first()
                .map(|msg| msg.index)
                .unwrap_or(1);
            sub.cursor = match index {
                SubPosition::Earliest => first_index,
                SubPosition::Latest => last_index + 1,
                SubPosition::At(value) => value,
            };
            Ok(())
        }

        async fn handle_delete_message_before(
            &self,
            queue_urn: &str,
            index: MsgIndex,
            _ctx: RPCContext,
        ) -> std::result::Result<u64, RPCErrors> {
            let mut queues = self.queues.lock().await;
            let queue = queues.get_mut(queue_urn).ok_or_else(|| {
                RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn))
            })?;
            let before_len = queue.messages.len();
            queue.messages.retain(|msg| msg.index >= index);
            let removed = before_len - queue.messages.len();
            Ok(removed as u64)
        }
    }

    fn build_client() -> MsgQueueClient {
        MsgQueueClient::new_in_process(Box::new(MockMsgQueue::new()))
    }

    #[tokio::test]
    async fn test_post_and_fetch_with_auto_commit() {
        let client = build_client();
        let queue_urn = client
            .create_queue(Some("alpha"), "app", "owner", QueueConfig::default())
            .await
            .unwrap();

        client
            .post_message(&queue_urn, Message::new(b"first".to_vec()))
            .await
            .unwrap();
        client
            .post_message(&queue_urn, Message::new(b"second".to_vec()))
            .await
            .unwrap();

        let sub_id = client
            .subscribe(&queue_urn, "user", "app", None, SubPosition::Earliest)
            .await
            .unwrap();

        let messages = client.fetch_messages(&sub_id, 1, true).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload, b"first".to_vec());

        let messages = client.fetch_messages(&sub_id, 2, true).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload, b"second".to_vec());
    }

    #[tokio::test]
    async fn test_commit_ack_and_seek() {
        let client = build_client();
        let queue_urn = client
            .create_queue(Some("beta"), "app", "owner", QueueConfig::default())
            .await
            .unwrap();

        let first_index = client
            .post_message(&queue_urn, Message::new(b"one".to_vec()))
            .await
            .unwrap();
        let second_index = client
            .post_message(&queue_urn, Message::new(b"two".to_vec()))
            .await
            .unwrap();

        let sub_id = client
            .subscribe(&queue_urn, "user", "app", None, SubPosition::Earliest)
            .await
            .unwrap();

        let messages = client.fetch_messages(&sub_id, 2, false).await.unwrap();
        assert_eq!(messages.len(), 2);

        client.commit_ack(&sub_id, second_index).await.unwrap();
        let messages = client.fetch_messages(&sub_id, 1, false).await.unwrap();
        assert!(messages.is_empty());

        client.seek(&sub_id, SubPosition::At(first_index)).await.unwrap();
        let messages = client.fetch_messages(&sub_id, 1, false).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload, b"one".to_vec());
    }

    #[tokio::test]
    async fn test_subscribe_latest_and_delete_before() {
        let client = build_client();
        let queue_urn = client
            .create_queue(Some("gamma"), "app", "owner", QueueConfig::default())
            .await
            .unwrap();

        client
            .post_message(&queue_urn, Message::new(b"old".to_vec()))
            .await
            .unwrap();

        let sub_id = client
            .subscribe(&queue_urn, "user", "app", None, SubPosition::Latest)
            .await
            .unwrap();

        let messages = client.fetch_messages(&sub_id, 1, true).await.unwrap();
        assert!(messages.is_empty());

        let new_index = client
            .post_message(&queue_urn, Message::new(b"new".to_vec()))
            .await
            .unwrap();
        let messages = client.fetch_messages(&sub_id, 1, true).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].index, new_index);

        let removed = client
            .delete_message_before(&queue_urn, new_index)
            .await
            .unwrap();
        assert_eq!(removed, 1);
    }
}
