use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AppInstanceId, ExecutorConfig, ExecutorLimits, ExecutorProfile, TokenPrincipalKind, TokenUse,
};

pub const AGENT_DAEMON_PROTOCOL_VERSION: u32 = 1;
pub const AGENT_DAEMON_MANAGE_PATH: &str = "/kapi/control-panel/executor/manage";
pub const AGENT_DAEMON_DATA_PATH: &str = "/kapi/control-panel/executor/data";
pub const AGENT_DAEMON_ROUTE_PATH: &str = "/kapi/control-panel/executor/route";
pub const AGENT_DAEMON_HEARTBEAT_MS: u64 = 5_000;
pub const AGENT_DAEMON_LEASE_MS: u64 = 15_000;
pub const AGENT_DAEMON_STREAM_TTL_MS: u64 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDaemonDevice {
    pub executor_id: String,
    pub name: String,
    pub owner_user_id: String,
    pub enabled: bool,
    pub expires_at_ms: Option<u64>,
    pub generation: u64,
    pub profiles: BTreeMap<String, ExecutorProfile>,
    pub limits: ExecutorLimits,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDaemonSession {
    pub subject: String,
    pub principal_kind: TokenPrincipalKind,
    pub token_use: TokenUse,
    pub app_instance_id: Option<AppInstanceId>,
    pub authorization_key: String,
    pub expires_at_ms: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentDaemonClientMessage {
    Hello {
        protocol_version: u32,
        executor_id: String,
        instance_id: String,
        resume_token: Option<String>,
    },
    Ready {
        generation: u64,
        store_id: String,
        capabilities: Value,
    },
    Heartbeat {
        generation: u64,
    },
    Goodbye {
        generation: u64,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentDaemonServerMessage {
    Welcome {
        protocol_version: u32,
        executor_id: String,
        generation: u64,
        resume_token: String,
        server_time_ms: u64,
        expires_at_ms: Option<u64>,
        heartbeat_interval_ms: u64,
        lease_timeout_ms: u64,
        config: ExecutorConfig,
    },
    Snapshot {
        generation: u64,
        server_time_ms: u64,
        expires_at_ms: Option<u64>,
        config: ExecutorConfig,
    },
    OpenStream {
        generation: u64,
        stream_id: String,
        ticket: String,
        expires_at_ms: u64,
        profile_id: String,
        session: AgentDaemonSession,
        session_token_sha256: String,
    },
    Closing {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDaemonOnlineTarget {
    pub executor_id: String,
    pub name: String,
    pub generation: u64,
    pub store_id: String,
    pub profiles: Vec<String>,
    pub capabilities: Value,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDaemonStreamRoute {
    pub executor_id: String,
    pub generation: u64,
    pub profile_id: String,
    pub stream_id: String,
    pub ws_url: String,
    pub ticket: String,
    pub expires_at_ms: u64,
}

impl std::fmt::Debug for AgentDaemonClientMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Hello { .. } => "Hello { credentials: [redacted] }",
            Self::Ready { .. } => "Ready",
            Self::Heartbeat { .. } => "Heartbeat",
            Self::Goodbye { .. } => "Goodbye",
        })
    }
}

impl std::fmt::Debug for AgentDaemonServerMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Welcome { .. } => "Welcome { credentials: [redacted] }",
            Self::Snapshot { .. } => "Snapshot",
            Self::OpenStream { .. } => "OpenStream { credentials: [redacted] }",
            Self::Closing { .. } => "Closing",
        })
    }
}
