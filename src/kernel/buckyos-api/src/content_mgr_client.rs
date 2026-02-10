use crate::{AppDoc, AppType, SelectorType};
use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use ndn_lib::ObjId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;

pub const CONTENT_MGR_SERVICE_UNIQUE_ID: &str = "publish-content-mgr";
pub const CONTENT_MGR_SERVICE_SERVICE_NAME: &str = "publish-content-mgr";

pub const SHARE_POLICY_PUBLIC: &str = "public";
pub const SHARE_POLICY_TOKEN_REQUIRED: &str = "token_required";
pub const SHARE_POLICY_ENCRYPTED: &str = "encrypted";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PublishRequest {
    pub name: String,
    pub obj_id: ObjId,
    pub share_policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_policy_config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_device_id: Option<String>,
}

impl PublishRequest {
    pub fn new(
        name: String,
        obj_id: ObjId,
        share_policy: String,
        share_policy_config: Option<Value>,
        expected_sequence: Option<u64>,
        op_device_id: Option<String>,
    ) -> Self {
        Self {
            name,
            obj_id,
            share_policy,
            share_policy_config,
            expected_sequence,
            op_device_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SharedItemInfo {
    pub name: String,
    pub current_obj_id: String,
    pub share_policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_policy_config: Option<Value>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<u64>,
    pub sequence: u64,
    pub history_count: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RevisionMetadata {
    pub name: String,
    pub sequence: u64,
    pub obj_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_policy_config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub committed_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessLogEntry {
    pub name: String,
    pub req_ts: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_device_id: Option<String>,
    pub bytes_sent: u64,
    pub status_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeBucketStat {
    pub name: String,
    pub time_bucket: u64,
    pub request_count: u64,
    pub bytes_sent: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_access_ts: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrPublishReq {
    pub name: String,
    pub obj_id: ObjId,
    pub share_policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_policy_config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_device_id: Option<String>,
}

impl ContentMgrPublishReq {
    pub fn new(
        name: String,
        obj_id: ObjId,
        share_policy: String,
        share_policy_config: Option<Value>,
        expected_sequence: Option<u64>,
        op_device_id: Option<String>,
    ) -> Self {
        Self {
            name,
            obj_id,
            share_policy,
            share_policy_config,
            expected_sequence,
            op_device_id,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse ContentMgrPublishReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrResolveReq {
    pub name: String,
}

impl ContentMgrResolveReq {
    pub fn new(name: String) -> Self {
        Self { name }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse ContentMgrResolveReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrResolveVersionReq {
    pub name: String,
    pub sequence: u64,
}

impl ContentMgrResolveVersionReq {
    pub fn new(name: String, sequence: u64) -> Self {
        Self { name, sequence }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse ContentMgrResolveVersionReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrListItemsReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

impl ContentMgrListItemsReq {
    pub fn new(prefix: Option<String>, limit: Option<usize>, offset: Option<u64>) -> Self {
        Self {
            prefix,
            limit,
            offset,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse ContentMgrListItemsReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrListHistoryReq {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

impl ContentMgrListHistoryReq {
    pub fn new(name: String, limit: Option<usize>, offset: Option<u64>) -> Self {
        Self {
            name,
            limit,
            offset,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse ContentMgrListHistoryReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrRecordBatchReq {
    pub logs: Vec<AccessLogEntry>,
}

impl ContentMgrRecordBatchReq {
    pub fn new(logs: Vec<AccessLogEntry>) -> Self {
        Self { logs }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse ContentMgrRecordBatchReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrGetStatsReq {
    pub name: String,
    pub start_ts: u64,
    pub end_ts: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket_size: Option<u64>,
}

impl ContentMgrGetStatsReq {
    pub fn new(name: String, start_ts: u64, end_ts: u64, bucket_size: Option<u64>) -> Self {
        Self {
            name,
            start_ts,
            end_ts,
            bucket_size,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse ContentMgrGetStatsReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrQueryLogsReq {
    pub filter: LogFilter,
}

impl ContentMgrQueryLogsReq {
    pub fn new(filter: LogFilter) -> Self {
        Self { filter }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse ContentMgrQueryLogsReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrGetItemReq {
    pub name: String,
}

impl ContentMgrGetItemReq {
    pub fn new(name: String) -> Self {
        Self { name }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse ContentMgrGetItemReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMgrSetItemEnabledReq {
    pub name: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ContentMgrSetItemEnabledReq {
    pub fn new(name: String, enabled: bool, reason: Option<String>) -> Self {
        Self {
            name,
            enabled,
            reason,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse ContentMgrSetItemEnabledReq: {}",
                error
            ))
        })
    }
}

pub enum ContentMgrClient {
    InProcess(Box<dyn ContentMgrHandler>),
    KRPC(Box<kRPC>),
}

impl ContentMgrClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_in_process(handler: Box<dyn ContentMgrHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self::KRPC(krpc_client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await;
            }
        }
    }

    pub async fn publish(&self, request: PublishRequest) -> std::result::Result<u64, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_publish(request, ctx).await
            }
            Self::KRPC(client) => {
                let req = ContentMgrPublishReq::new(
                    request.name,
                    request.obj_id,
                    request.share_policy,
                    request.share_policy_config,
                    request.expected_sequence,
                    request.op_device_id,
                );
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrPublishReq: {}",
                        error
                    ))
                })?;
                let result = client.call("publish", req_json).await?;
                result.as_u64().ok_or_else(|| {
                    RPCErrors::ParserResponseError(
                        "Expected publish result as u64 sequence".to_string(),
                    )
                })
            }
        }
    }

    pub async fn resolve(&self, name: &str) -> std::result::Result<Option<String>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_resolve(name, ctx).await
            }
            Self::KRPC(client) => {
                let req = ContentMgrResolveReq::new(name.to_string());
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrResolveReq: {}",
                        error
                    ))
                })?;
                let result = client.call("resolve", req_json).await?;
                if result.is_null() {
                    return Ok(None);
                }
                let obj_id = result.as_str().ok_or_else(|| {
                    RPCErrors::ParserResponseError(
                        "Expected resolve result as string or null".to_string(),
                    )
                })?;
                Ok(Some(obj_id.to_string()))
            }
        }
    }

    pub async fn resolve_version(
        &self,
        name: &str,
        sequence: u64,
    ) -> std::result::Result<Option<String>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_resolve_version(name, sequence, ctx).await
            }
            Self::KRPC(client) => {
                let req = ContentMgrResolveVersionReq::new(name.to_string(), sequence);
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrResolveVersionReq: {}",
                        error
                    ))
                })?;
                let result = client.call("resolve_version", req_json).await?;
                if result.is_null() {
                    return Ok(None);
                }
                let obj_id = result.as_str().ok_or_else(|| {
                    RPCErrors::ParserResponseError(
                        "Expected resolve_version result as string or null".to_string(),
                    )
                })?;
                Ok(Some(obj_id.to_string()))
            }
        }
    }

    pub async fn get_item(
        &self,
        name: &str,
    ) -> std::result::Result<Option<SharedItemInfo>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_get_item(name, ctx).await
            }
            Self::KRPC(client) => {
                let req = ContentMgrGetItemReq::new(name.to_string());
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrGetItemReq: {}",
                        error
                    ))
                })?;
                let result = client.call("get_item", req_json).await?;
                if result.is_null() {
                    return Ok(None);
                }
                let item = serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse get_item response: {}",
                        error
                    ))
                })?;
                Ok(Some(item))
            }
        }
    }

    pub async fn set_item_enabled(
        &self,
        name: &str,
        enabled: bool,
        reason: Option<&str>,
    ) -> std::result::Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_set_item_enabled(name, enabled, reason, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = ContentMgrSetItemEnabledReq::new(
                    name.to_string(),
                    enabled,
                    reason.map(|value| value.to_string()),
                );
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrSetItemEnabledReq: {}",
                        error
                    ))
                })?;
                client.call("set_item_enabled", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn is_item_enabled(&self, name: &str) -> std::result::Result<bool, RPCErrors> {
        let item = self.get_item(name).await?;
        match item {
            Some(item) => Ok(item.enabled),
            None => Err(RPCErrors::ReasonError(format!(
                "Item not found for name: {}",
                name
            ))),
        }
    }

    pub async fn list_items(
        &self,
        prefix: Option<&str>,
        limit: Option<usize>,
        offset: Option<u64>,
    ) -> std::result::Result<Vec<SharedItemInfo>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_list_items(prefix, limit, offset, ctx).await
            }
            Self::KRPC(client) => {
                let req = ContentMgrListItemsReq::new(
                    prefix.map(|value| value.to_string()),
                    limit,
                    offset,
                );
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrListItemsReq: {}",
                        error
                    ))
                })?;
                let result = client.call("list_items", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse list_items response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn list_history(
        &self,
        name: &str,
        limit: Option<usize>,
        offset: Option<u64>,
    ) -> std::result::Result<Vec<RevisionMetadata>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_list_history(name, limit, offset, ctx).await
            }
            Self::KRPC(client) => {
                let req = ContentMgrListHistoryReq::new(name.to_string(), limit, offset);
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrListHistoryReq: {}",
                        error
                    ))
                })?;
                let result = client.call("list_history", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse list_history response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn record_batch(
        &self,
        logs: Vec<AccessLogEntry>,
    ) -> std::result::Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_record_batch(logs, ctx).await
            }
            Self::KRPC(client) => {
                let req = ContentMgrRecordBatchReq::new(logs);
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrRecordBatchReq: {}",
                        error
                    ))
                })?;
                client.call("record_batch", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn get_stats(
        &self,
        name: &str,
        start_ts: u64,
        end_ts: u64,
        bucket_size: Option<u64>,
    ) -> std::result::Result<Vec<TimeBucketStat>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_get_stats(name, start_ts, end_ts, bucket_size, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req =
                    ContentMgrGetStatsReq::new(name.to_string(), start_ts, end_ts, bucket_size);
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrGetStatsReq: {}",
                        error
                    ))
                })?;
                let result = client.call("get_stats", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse get_stats response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn query_logs(
        &self,
        filter: LogFilter,
    ) -> std::result::Result<Vec<AccessLogEntry>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_query_logs(filter, ctx).await
            }
            Self::KRPC(client) => {
                let req = ContentMgrQueryLogsReq::new(filter);
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize ContentMgrQueryLogsReq: {}",
                        error
                    ))
                })?;
                let result = client.call("query_logs", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse query_logs response: {}",
                        error
                    ))
                })
            }
        }
    }
}

#[async_trait]
pub trait ContentMgrHandler: Send + Sync {
    async fn handle_publish(
        &self,
        request: PublishRequest,
        ctx: RPCContext,
    ) -> std::result::Result<u64, RPCErrors>;

    async fn handle_resolve(
        &self,
        name: &str,
        ctx: RPCContext,
    ) -> std::result::Result<Option<String>, RPCErrors>;

    async fn handle_resolve_version(
        &self,
        name: &str,
        sequence: u64,
        ctx: RPCContext,
    ) -> std::result::Result<Option<String>, RPCErrors>;

    async fn handle_get_item(
        &self,
        name: &str,
        ctx: RPCContext,
    ) -> std::result::Result<Option<SharedItemInfo>, RPCErrors>;

    async fn handle_set_item_enabled(
        &self,
        name: &str,
        enabled: bool,
        reason: Option<&str>,
        ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors>;

    async fn handle_list_items(
        &self,
        prefix: Option<&str>,
        limit: Option<usize>,
        offset: Option<u64>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<SharedItemInfo>, RPCErrors>;

    async fn handle_list_history(
        &self,
        name: &str,
        limit: Option<usize>,
        offset: Option<u64>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<RevisionMetadata>, RPCErrors>;

    async fn handle_record_batch(
        &self,
        logs: Vec<AccessLogEntry>,
        ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors>;

    async fn handle_get_stats(
        &self,
        name: &str,
        start_ts: u64,
        end_ts: u64,
        bucket_size: Option<u64>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<TimeBucketStat>, RPCErrors>;

    async fn handle_query_logs(
        &self,
        filter: LogFilter,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<AccessLogEntry>, RPCErrors>;
}

pub struct ContentMgrServerHandler<T: ContentMgrHandler>(pub T);

impl<T: ContentMgrHandler> ContentMgrServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: ContentMgrHandler> RPCHandler for ContentMgrServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            "publish" => {
                let publish_req = ContentMgrPublishReq::from_json(req.params)?;
                let request = PublishRequest::new(
                    publish_req.name,
                    publish_req.obj_id,
                    publish_req.share_policy,
                    publish_req.share_policy_config,
                    publish_req.expected_sequence,
                    publish_req.op_device_id,
                );
                let result = self.0.handle_publish(request, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "resolve" => {
                let resolve_req = ContentMgrResolveReq::from_json(req.params)?;
                let result = self.0.handle_resolve(&resolve_req.name, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "resolve_version" => {
                let resolve_req = ContentMgrResolveVersionReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_resolve_version(&resolve_req.name, resolve_req.sequence, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "get_item" => {
                let item_req = ContentMgrGetItemReq::from_json(req.params)?;
                let result = self.0.handle_get_item(&item_req.name, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "set_item_enabled" => {
                let set_req = ContentMgrSetItemEnabledReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_set_item_enabled(
                        &set_req.name,
                        set_req.enabled,
                        set_req.reason.as_deref(),
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "list_items" => {
                let list_req = ContentMgrListItemsReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_list_items(
                        list_req.prefix.as_deref(),
                        list_req.limit,
                        list_req.offset,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "list_history" => {
                let list_req = ContentMgrListHistoryReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_list_history(&list_req.name, list_req.limit, list_req.offset, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "record_batch" => {
                let record_req = ContentMgrRecordBatchReq::from_json(req.params)?;
                let result = self.0.handle_record_batch(record_req.logs, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "get_stats" => {
                let stats_req = ContentMgrGetStatsReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_stats(
                        &stats_req.name,
                        stats_req.start_ts,
                        stats_req.end_ts,
                        stats_req.bucket_size,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "query_logs" => {
                let query_req = ContentMgrQueryLogsReq::from_json(req.params)?;
                let result = self.0.handle_query_logs(query_req.filter, ctx).await?;
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

pub fn generate_content_mgr_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        CONTENT_MGR_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Share Content Manager")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{Arc, Mutex};

    #[derive(Default, Debug)]
    struct MockCalls {
        publish: Option<PublishRequest>,
        resolve: Option<String>,
        resolve_version: Option<(String, u64)>,
        get_item: Option<String>,
        set_item_enabled: Option<(String, bool, Option<String>)>,
        list_items: Option<(Option<String>, Option<usize>, Option<u64>)>,
        list_history: Option<(String, Option<usize>, Option<u64>)>,
        record_batch_len: Option<usize>,
        get_stats: Option<(String, u64, u64, Option<u64>)>,
        query_logs: Option<LogFilter>,
    }

    #[derive(Clone)]
    struct MockContentMgr {
        calls: Arc<Mutex<MockCalls>>,
    }

    impl MockContentMgr {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(MockCalls::default())),
            }
        }
    }

    #[async_trait]
    impl ContentMgrHandler for MockContentMgr {
        async fn handle_publish(
            &self,
            request: PublishRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<u64, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.publish = Some(request);
            Ok(3)
        }

        async fn handle_resolve(
            &self,
            name: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<Option<String>, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.resolve = Some(name.to_string());
            Ok(Some("obj-3".to_string()))
        }

        async fn handle_resolve_version(
            &self,
            name: &str,
            sequence: u64,
            _ctx: RPCContext,
        ) -> std::result::Result<Option<String>, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.resolve_version = Some((name.to_string(), sequence));
            Ok(Some(format!("obj-{}", sequence)))
        }

        async fn handle_get_item(
            &self,
            name: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<Option<SharedItemInfo>, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.get_item = Some(name.to_string());
            Ok(Some(SharedItemInfo {
                name: name.to_string(),
                current_obj_id: "obj-3".to_string(),
                share_policy: SHARE_POLICY_PUBLIC.to_string(),
                share_policy_config: Some(json!({"ttl_seconds": 3600})),
                enabled: true,
                disabled_reason: None,
                disabled_at: None,
                sequence: 3,
                history_count: 3,
                created_at: 1,
                updated_at: 3,
            }))
        }

        async fn handle_set_item_enabled(
            &self,
            name: &str,
            enabled: bool,
            reason: Option<&str>,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.set_item_enabled = Some((
                name.to_string(),
                enabled,
                reason.map(|value| value.to_string()),
            ));
            Ok(())
        }

        async fn handle_list_items(
            &self,
            prefix: Option<&str>,
            limit: Option<usize>,
            offset: Option<u64>,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<SharedItemInfo>, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.list_items = Some((prefix.map(|value| value.to_string()), limit, offset));
            Ok(vec![SharedItemInfo {
                name: "home/docs/readme.md".to_string(),
                current_obj_id: "obj-3".to_string(),
                share_policy: SHARE_POLICY_PUBLIC.to_string(),
                share_policy_config: Some(json!({
                    "token_issuer": "verify-hub",
                    "token_expire_seconds": 600
                })),
                enabled: true,
                disabled_reason: None,
                disabled_at: None,
                sequence: 3,
                history_count: 3,
                created_at: 1,
                updated_at: 3,
            }])
        }

        async fn handle_list_history(
            &self,
            name: &str,
            limit: Option<usize>,
            offset: Option<u64>,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<RevisionMetadata>, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.list_history = Some((name.to_string(), limit, offset));
            Ok(vec![RevisionMetadata {
                name: name.to_string(),
                sequence: 1,
                obj_id: "obj-1".to_string(),
                share_policy: Some(SHARE_POLICY_PUBLIC.to_string()),
                share_policy_config: Some(json!({"token_issuer": "verify-hub"})),
                enabled: Some(true),
                committed_at: 1,
                op_device_id: Some("dev-1".to_string()),
            }])
        }

        async fn handle_record_batch(
            &self,
            logs: Vec<AccessLogEntry>,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.record_batch_len = Some(logs.len());
            Ok(())
        }

        async fn handle_get_stats(
            &self,
            name: &str,
            start_ts: u64,
            end_ts: u64,
            bucket_size: Option<u64>,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<TimeBucketStat>, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.get_stats = Some((name.to_string(), start_ts, end_ts, bucket_size));
            Ok(vec![TimeBucketStat {
                name: name.to_string(),
                time_bucket: start_ts,
                request_count: 10,
                bytes_sent: 2048,
                last_access_ts: Some(end_ts),
            }])
        }

        async fn handle_query_logs(
            &self,
            filter: LogFilter,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<AccessLogEntry>, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.query_logs = Some(filter);
            Ok(vec![AccessLogEntry {
                name: "home/docs/readme.md".to_string(),
                req_ts: 100,
                source_device_id: Some("dev-1".to_string()),
                bytes_sent: 128,
                status_code: 200,
                user_agent: Some("mock-agent".to_string()),
            }])
        }
    }

    #[test]
    fn test_generate_content_mgr_service_doc() {
        let doc = generate_content_mgr_service_doc();
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }

    #[tokio::test]
    async fn test_in_process_client() {
        let handler = MockContentMgr::new();
        let calls = handler.calls.clone();
        let client = ContentMgrClient::new_in_process(Box::new(handler));

        let publish_seq = client
            .publish(PublishRequest::new(
                "home/docs/readme.md".to_string(),
                ObjId::new("chunk:123456").unwrap(),
                SHARE_POLICY_PUBLIC.to_string(),
                Some(json!({"ttl_seconds": 3600})),
                Some(2),
                Some("dev-1".to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(publish_seq, 3);

        let resolved = client.resolve("home/docs/readme.md").await.unwrap();
        assert_eq!(resolved, Some("obj-3".to_string()));

        client
            .set_item_enabled("home/docs/readme.md", false, Some("manual block"))
            .await
            .unwrap();
        let item = client.get_item("home/docs/readme.md").await.unwrap();
        assert_eq!(item.unwrap().enabled, true);
        let enabled = client.is_item_enabled("home/docs/readme.md").await.unwrap();
        assert_eq!(enabled, true);

        let stats = client
            .get_stats("home/docs/readme.md", 10, 20, Some(10))
            .await
            .unwrap();
        assert_eq!(stats.len(), 1);

        let calls = calls.lock().unwrap();
        assert!(calls.publish.is_some());
        assert_eq!(calls.resolve, Some("home/docs/readme.md".to_string()));
        assert_eq!(
            calls.set_item_enabled,
            Some((
                "home/docs/readme.md".to_string(),
                false,
                Some("manual block".to_string())
            ))
        );
        assert_eq!(calls.get_item, Some("home/docs/readme.md".to_string()));
        assert_eq!(
            calls.get_stats,
            Some(("home/docs/readme.md".to_string(), 10, 20, Some(10)))
        );
    }

    #[tokio::test]
    async fn test_rpc_handler_dispatch() {
        let handler = MockContentMgr::new();
        let calls = handler.calls.clone();
        let rpc_handler = ContentMgrServerHandler::new(handler);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        let query_req = RPCRequest {
            method: "query_logs".to_string(),
            params: json!({
                "filter": {
                    "name": "home/docs/readme.md",
                    "status_code": 200,
                    "limit": 20
                }
            }),
            seq: 7,
            token: None,
            trace_id: Some("trace-1".to_string()),
        };

        let resp = rpc_handler.handle_rpc_call(query_req, ip).await.unwrap();
        match resp.result {
            RPCResult::Success(value) => {
                let logs: Vec<AccessLogEntry> = serde_json::from_value(value).unwrap();
                assert_eq!(logs.len(), 1);
                assert_eq!(logs[0].status_code, 200);
            }
            _ => panic!("Expected success response"),
        }

        let set_req = RPCRequest {
            method: "set_item_enabled".to_string(),
            params: json!({
                "name": "home/docs/readme.md",
                "enabled": false,
                "reason": "risk-control"
            }),
            seq: 8,
            token: None,
            trace_id: Some("trace-2".to_string()),
        };
        let set_resp = rpc_handler.handle_rpc_call(set_req, ip).await.unwrap();
        match set_resp.result {
            RPCResult::Success(_) => {}
            _ => panic!("Expected success response"),
        }

        let calls = calls.lock().unwrap();
        assert!(calls.query_logs.is_some());
        assert_eq!(
            calls.query_logs.as_ref().unwrap().name.as_deref(),
            Some("home/docs/readme.md")
        );
        assert_eq!(
            calls.set_item_enabled,
            Some((
                "home/docs/readme.md".to_string(),
                false,
                Some("risk-control".to_string())
            ))
        );
    }
}
