use serde::{Deserialize, Serialize};
use serde_json::Value;

mod client;
mod server;

pub use client::*;
pub use server::*;

pub const KLOG_JSON_RPC_VERSION: &str = "2.0";
pub const KLOG_JSON_RPC_SERVICE_PATH: &str = "/kapi/klog-service";
pub const KLOG_JSON_RPC_PATH: &str = "/klog/rpc";
pub const KLOG_RPC_METHOD_LOG_APPEND: &str = "klog.log.append";
pub const KLOG_RPC_METHOD_LOG_QUERY: &str = "klog.log.query";
pub const KLOG_RPC_METHOD_LOG_APPEND_LEGACY: &str = "klog.append";
pub const KLOG_RPC_METHOD_LOG_QUERY_LEGACY: &str = "klog.query";
pub const KLOG_RPC_METHOD_APPEND: &str = KLOG_RPC_METHOD_LOG_APPEND;
pub const KLOG_RPC_METHOD_QUERY: &str = KLOG_RPC_METHOD_LOG_QUERY;
pub const KLOG_RPC_METHOD_META_PUT: &str = "klog.meta.put";
pub const KLOG_RPC_METHOD_META_DELETE: &str = "klog.meta.delete";
pub const KLOG_RPC_METHOD_META_QUERY: &str = "klog.meta.query";

pub const KLOG_RPC_ERR_INVALID_REQUEST: i64 = -32600;
pub const KLOG_RPC_ERR_METHOD_NOT_FOUND: i64 = -32601;
pub const KLOG_RPC_ERR_INVALID_PARAMS: i64 = -32602;
pub const KLOG_RPC_ERR_INTERNAL: i64 = -32000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KLogJsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KLogJsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KLogJsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<KLogJsonRpcError>,
    pub id: u64,
}

impl KLogJsonRpcResponse {
    pub fn success<T: Serialize>(id: u64, result: T) -> Self {
        let result = serde_json::to_value(result).unwrap_or(Value::Null);
        Self {
            jsonrpc: KLOG_JSON_RPC_VERSION.to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn error(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: KLOG_JSON_RPC_VERSION.to_string(),
            result: None,
            error: Some(KLogJsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }

    pub fn error_with_data(
        id: u64,
        code: i64,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Self {
        Self {
            jsonrpc: KLOG_JSON_RPC_VERSION.to_string(),
            result: None,
            error: Some(KLogJsonRpcError {
                code,
                message: message.into(),
                data,
            }),
            id,
        }
    }
}
