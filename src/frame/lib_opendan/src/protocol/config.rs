//! `.opendan_agent_session/session_config.json` — launch configuration (§4.2).
//!
//! The file is replaced atomically as a whole; only a handful of fields
//! (dynamic subscriptions) may change while the session runs, each change
//! bumps `config_rev`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SESSION_CONFIG_SCHEMA: &str = "opendan.session_config/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Ui,
    Work,
    SelfImprove,
    SelfCheck,
}

impl SessionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionKind::Ui => "ui",
            SessionKind::Work => "work",
            SessionKind::SelfImprove => "self_improve",
            SessionKind::SelfCheck => "self_check",
        }
    }

    /// Prefix of the global session id (§4.2).
    pub fn id_prefix(&self) -> &'static str {
        match self {
            SessionKind::Ui => "ui",
            SessionKind::Work => "work",
            SessionKind::SelfImprove => "si",
            SessionKind::SelfCheck => "sc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreatedBy {
    /// `app:<appid>@<owner>` (A9).
    pub principal: String,
    /// `app | ui_session:<sid> | opendan | task_mgr:<task_id>`.
    #[serde(default = "default_via")]
    pub via: String,
}

fn default_via() -> String {
    "app".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DriverRef {
    /// Driving identity = the appid of the driving process (Q9 / Q16).
    pub principal: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Origin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reason_messages: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndConditionType {
    LlmDeclaresDone,
    OutputSchema,
    MaxRounds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EndCondition {
    #[serde(rename = "type")]
    pub kind: EndConditionType,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

impl Default for EndCondition {
    fn default() -> Self {
        Self {
            kind: EndConditionType::LlmDeclaresDone,
            detail: Value::Null,
        }
    }
}

/// Initial activity declaration: what this session plans to modify (§6.7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Scope {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputPolicy {
    #[default]
    Any,
    SupplementOnly,
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentAccess {
    #[default]
    Full,
    StatusOnly,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Acl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readers: Vec<String>,
    #[serde(default)]
    pub agent_access: AgentAccess,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionSection {
    pub session_id: String,
    pub agent_did: String,
    pub kind: SessionKind,
    /// `agent.toml [session.<class>]`; decides loop / driver defaults.
    #[serde(default = "default_class")]
    pub class: String,
    pub created_at_ms: u64,
    pub created_by: CreatedBy,
    pub driver: DriverRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// UI sessions only (deferred, V1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub end_condition: EndCondition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub input_policy: InputPolicy,
    #[serde(default)]
    pub acl: Acl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_binding: Option<Value>,
}

fn default_class() -> String {
    "work".to_string()
}

/// Mechanical compression parameters for rendering worklog history (§4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MechanicalCompress {
    #[serde(default = "default_recent_full_steps")]
    pub recent_full_steps: u32,
    #[serde(default = "default_summary_chars")]
    pub summary_chars: u32,
    #[serde(default = "default_max_result_chars")]
    pub max_result_chars: u32,
    #[serde(default = "default_drop_kinds")]
    pub drop_kinds: Vec<String>,
}

fn default_recent_full_steps() -> u32 {
    2
}
fn default_summary_chars() -> u32 {
    280
}
fn default_max_result_chars() -> u32 {
    4096
}
pub fn default_drop_kinds() -> Vec<String> {
    vec![
        "created".to_string(),
        "decide".to_string(),
        "compaction".to_string(),
        "input_rejected".to_string(),
        "change_dropped".to_string(),
    ]
}

impl Default for MechanicalCompress {
    fn default() -> Self {
        Self {
            recent_full_steps: default_recent_full_steps(),
            summary_chars: default_summary_chars(),
            max_result_chars: default_max_result_chars(),
            drop_kinds: default_drop_kinds(),
        }
    }
}

/// Prompt configuration: a superset of xllm's `.llm_context` (§4.2).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PromptSection {
    /// JSON form of an xllm `.llm_context` file (same schema, strict keys).
    /// xllm can resume a run of this session without understanding the
    /// OpenDAN extensions below.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub llm_context: Value,
    /// `behaviors/<name>` reference (BehaviorAssembler).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    /// Application system prompt (S-05); composed after the agent identity
    /// and the non-overridable constraints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Initial context material supplied by the application.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
    #[serde(default)]
    pub mechanical_compress: MechanicalCompress,
    /// Context budget (tokens) available to the history section when the
    /// next llm_context is assembled. `None` → runner default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_budget_tokens: Option<u32>,
    /// Compact after a run once the history uses more than this ratio of the
    /// budget (`maybe_compact`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_ratio: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeRequirement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    /// Executables that must be reachable on PATH.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// Tools that only exist inside the runner process; verified on every
    /// drive (§7.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeSection {
    #[serde(default)]
    pub requirement: RuntimeRequirement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_plan: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceRef {
    /// Agent internal workspace `<agent_root>/workspace/<id>`.
    Agent { id: String },
    /// Directory owned by a user / application.
    External { path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionMode {
    /// Delivered as `event`; wakes the session and triggers inference.
    Active,
    /// Delivered as `change`; only injected at observation boundaries.
    Semi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubscriptionSource {
    /// Another session of the same agent (pull: registry rev compare).
    Session {
        #[serde(rename = "ref")]
        session_ref: String,
    },
    /// External object events bridged into the kmsg queue.
    ObjectEvent { object: String, event: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Subscription {
    pub id: String,
    pub mode: SubscriptionMode,
    pub source: SubscriptionSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watch: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputSourceConfig {
    /// kmsg queue input (all sessions).
    Kmsg {
        id: String,
        queue: String,
        subscriber: String,
    },
    /// msg-center inbox (UI sessions; deferred, V1).
    MsgCenter {
        id: String,
        did: String,
        session_id: String,
    },
}

impl InputSourceConfig {
    pub fn id(&self) -> &str {
        match self {
            InputSourceConfig::Kmsg { id, .. } | InputSourceConfig::MsgCenter { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Channels {
    #[serde(default)]
    pub inputs: Vec<InputSourceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound: Option<Value>,
    /// kevent id published after posting; every segment only uses
    /// letters, digits and `_ - .`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_event: Option<String>,
}

impl Channels {
    pub fn kmsg(&self) -> Option<(&str, &str, &str)> {
        self.inputs.iter().find_map(|s| match s {
            InputSourceConfig::Kmsg {
                id,
                queue,
                subscriber,
            } => Some((id.as_str(), queue.as_str(), subscriber.as_str())),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionConfig {
    pub schema: String,
    #[serde(default = "one")]
    pub config_rev: u64,
    pub session: SessionSection,
    #[serde(default)]
    pub prompt: PromptSection,
    #[serde(default)]
    pub runtime: RuntimeSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<Subscription>,
    #[serde(default)]
    pub channels: Channels,
    /// Application extension items, preserved verbatim.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

fn one() -> u64 {
    1
}

impl SessionConfig {
    pub fn session_id(&self) -> &str {
        &self.session.session_id
    }
}
