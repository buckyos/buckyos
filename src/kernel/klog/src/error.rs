use crate::KNode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KLogErrorCode {
    InvalidArgument,
    NotLeader,
    LeaderUnavailable,
    ConfigChangeInProgress,
    PayloadTooLarge,
    Timeout,
    Unavailable,
    AuthRequired,
    Forbidden,
    Internal,
}

impl KLogErrorCode {
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::NotLeader
                | Self::LeaderUnavailable
                | Self::ConfigChangeInProgress
                | Self::Timeout
                | Self::Unavailable
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogErrorEnvelope {
    pub error_code: KLogErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader_hint: Option<KNode>,
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogServiceError {
    pub http_status: u16,
    #[serde(flatten)]
    pub error: KLogErrorEnvelope,
}

impl KLogErrorEnvelope {
    pub fn new(
        error_code: KLogErrorCode,
        message: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            error_code,
            message: message.into(),
            retryable: error_code.is_retryable(),
            leader_hint: None,
            trace_id: trace_id.into(),
        }
    }

    pub fn from_http_status(
        http_status: u16,
        message: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Self {
        Self::new(
            map_http_status_to_error_code(http_status),
            message,
            trace_id,
        )
    }

    pub fn with_leader_hint(mut self, leader_hint: Option<KNode>) -> Self {
        self.leader_hint = leader_hint;
        self
    }
}

impl KLogServiceError {
    pub fn new(
        http_status: u16,
        error_code: KLogErrorCode,
        message: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            http_status,
            error: KLogErrorEnvelope::new(error_code, message, trace_id),
        }
    }

    pub fn from_http_status(
        http_status: u16,
        message: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            http_status,
            error: KLogErrorEnvelope::from_http_status(http_status, message, trace_id),
        }
    }

    pub fn with_leader_hint(mut self, leader_hint: Option<KNode>) -> Self {
        self.error = self.error.with_leader_hint(leader_hint);
        self
    }
}

impl std::fmt::Display for KLogServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "klog service error: status={}, code={:?}, retryable={}, leader_hint={:?}, trace_id={}, message={}",
            self.http_status,
            self.error.error_code,
            self.error.retryable,
            self.error.leader_hint,
            self.error.trace_id,
            self.error.message
        )
    }
}

impl std::error::Error for KLogServiceError {}

pub fn generate_trace_id() -> String {
    Uuid::now_v7().to_string()
}

pub fn normalize_trace_id(raw: Option<&str>) -> String {
    let normalized = raw.map(str::trim).filter(|v| !v.is_empty());
    normalized
        .map(|v| v.to_string())
        .unwrap_or_else(generate_trace_id)
}

pub fn map_http_status_to_error_code(http_status: u16) -> KLogErrorCode {
    match http_status {
        400 => KLogErrorCode::InvalidArgument,
        401 => KLogErrorCode::AuthRequired,
        403 => KLogErrorCode::Forbidden,
        408 | 504 => KLogErrorCode::Timeout,
        409 => KLogErrorCode::NotLeader,
        413 => KLogErrorCode::PayloadTooLarge,
        502 => KLogErrorCode::LeaderUnavailable,
        503 => KLogErrorCode::Unavailable,
        _ => KLogErrorCode::Internal,
    }
}

pub fn map_json_rpc_error_code_to_klog_error_code(code: i64) -> KLogErrorCode {
    match code {
        -32602 => KLogErrorCode::InvalidArgument,
        -32601 => KLogErrorCode::InvalidArgument,
        -32600 => KLogErrorCode::InvalidArgument,
        _ => KLogErrorCode::Internal,
    }
}

pub fn parse_error_envelope_json(raw: &str) -> Option<KLogErrorEnvelope> {
    serde_json::from_str::<KLogErrorEnvelope>(raw).ok()
}
