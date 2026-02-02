实现基础组件 msg_接口如下。


```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// 全局唯一的队列标识符 (Uniform Resource Name)
/// 格式通常为: "appid::owner::name"
pub type QueueUrn = String;

/// 消息索引，单调递增
pub type MsgIndex = u64;

/// 订阅者 ID
pub type SubscriptionId = String;

#[derive(Debug, Error)]
pub enum MsgQueueError {
    #[error("Queue not found: {0}")]
    NotFound(String),
    #[error("Permission denied")]
    PermissionDenied,
    #[error("IO error: {0}")]
    Io(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Invalid argument: {0}")]
    InvalidArg(String),
    #[error("Already exists: {0}")]
    AlreadyExists(String),
    #[error("Storage backend error: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, MsgQueueError>;

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
            index: 0, // 占位，写入时由服务端填充
            created_at: 0, // 建议写入时填充
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
    /// 权限控制，对非创建者的权限控制，一般是同owner_id,不同appid的情况下，允许读（订阅）
    /// TODO:需要细化设计
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_messages: None,
            retention_seconds: None,
            sync_write: false, // 默认追求高性能
        }
    }
}

/// 队列状态统计
#[derive(Debug, Default)]
pub struct QueueStats {
    pub message_count: u64,
    pub first_index: u64,
    pub last_index: u64,
    pub size_bytes: u64,
}

/// 订阅起始位置
#[derive(Debug, Clone, Copy)]
pub enum SubPosition {
    /// 从最早可用的消息开始
    Earliest,
    /// 从最新的消息之后开始（只读新消息）
    Latest,
    /// 从指定索引开始
    At(MsgIndex),
}

/// BuckyOS 核心消息队列接口
/// 采用 Log-Structured 设计，支持多生产者、多消费者、持久化和回溯。
#[async_trait]
pub trait MsgQueue: Send + Sync {
    // -------------------------------------------------------------------------
    // Control Plane (管理面)
    // -------------------------------------------------------------------------

    /// 创建队列
    /// * `name`: 局部名称。如果为 None，自动生成 UUID。
    /// * `appid` & `app_owner`: 用于命名空间隔离和鉴权。
    /// * Returns: 全局唯一的 QueueUrn。
    async fn create_queue(
        &self,
        name: Option<&str>,
        appid: &str,
        app_owner: &str,
        config: QueueConfig,
    ) -> Result<QueueUrn>;

    /// 删除队列及其所有数据
    async fn delete_queue(&self, queue_urn: &str) -> Result<()>;

    /// 获取队列统计信息
    async fn get_queue_stats(&self, queue_urn: &str) -> Result<QueueStats>;

    /// 更新队列配置 (如保留策略)
    async fn update_queue_config(&self, queue_urn: &str, config: QueueConfig) -> Result<()>;

    // -------------------------------------------------------------------------
    // Data Plane: Production (生产面)
    // -------------------------------------------------------------------------

    /// 发送单条消息
    /// 返回该消息分配的 Index。
    async fn post_message(&self, queue_urn: &str, message: Message) -> Result<MsgIndex>;

    // -------------------------------------------------------------------------
    // Data Plane: Consumption (消费面)
    // -------------------------------------------------------------------------

    /// 创建订阅者 (Stateful Subscriber)
    /// 服务端会维护该 Subscriber 的 cursor 状态。
    async fn subscribe(
        &self,
        queue_urn: &str,
        sub_id: Option<String>, // 如果为 None，自动生成一个临时 ID
        position: SubPosition,
    ) -> Result<SubscriptionId>;

    /// 取消订阅 (清理服务端 Cursor 状态)
    async fn unsubscribe(&self, sub_id: &str) -> Result<()>;

    /// 拉取消息
    /// * `length`: 最大拉取条数。
    /// * `auto_commit`: 
    ///     * `true`: 拉取后自动更新服务端游标（At-Most-Once / Best Effort）。
    ///     * `false`: 不更新游标，需手动调用 `commit_ack`（At-Least-Once）。
    async fn fetch_messages(
        &self,
        sub_id: &str,
        length: usize,
        auto_commit: bool
    ) -> Result<Vec<Message>>;

    /// 显式提交游标 (配合 fetch_messages auto_commit=false 使用)
    /// 确认 `index` 及其之前的消息已被处理。
    async fn commit_ack(&self, sub_id: &str, index: MsgIndex) -> Result<()>;

    /// 重置订阅者游标到指定位置
    /// `index` 为 None 表示 Seek 到 Latest。
    async fn seek(&self, sub_id: &str, index: SubPosition) -> Result<()>;

    // -------------------------------------------------------------------------
    // Data Plane: Retention (维护面)
    // -------------------------------------------------------------------------

    /// 删除指定 Index 之前的所有消息 (Log Truncation)
    /// 通常用于 Raft Log Compact 或 磁盘空间回收。
    async fn delete_message_before(&self, queue_urn: &str, index: MsgIndex) -> Result<u64>;
}

// -------------------------------------------------------------------------
// Helper Functions (工具函数)
// -------------------------------------------------------------------------

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


```

