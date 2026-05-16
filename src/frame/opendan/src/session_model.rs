use serde::{Deserialize, Serialize};

/// Internal wake-up signal for the session worker. The worker consumes the
/// actual payload from `SessionMeta::pending_inputs` (which is persisted) —
/// this channel only nudges the worker to check.
#[derive(Debug, Clone)]
pub enum SessionInput {
    /// New item enqueued to `meta.pending_inputs` — worker should re-check.
    Wakeup,
    /// Cooperative cancel (used by `stop()`).
    Cancel,
}

/// How an interrupt should wind down outstanding tool calls. Chosen
/// per-call by the caller of `AgentSession::interrupt` — different
/// upper-layer control flows targeting the same session may legitimately
/// want different strategies, so this is not a per-behavior or per-agent
/// default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterruptMode {
    /// Inject `Observation::Cancelled` for every pending tool call and
    /// drive the existing LLMContext to a terminal outcome.
    Graceful,
    /// Discard the trailing assistant turn that owns the unresolved
    /// `tool_use` blocks and continue from the truncated history.
    Discard,
}

/// One inbound item parked on the session until the worker is ready to
/// consume it. Persisted as part of [`SessionMeta`] so that a crash between
/// enqueue and LLM processing never loses a message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingInput {
    Msg {
        record_id: String,
        from: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_did: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tunnel_did: Option<String>,
        text: String,
    },
    Event {
        event_id: String,
        data: serde_json::Value,
    },
    Interrupt {
        mode: InterruptMode,
        id: String,
    },
}

impl PendingInput {
    /// Stable dedup key. Two `PendingInput`s with the same key are treated
    /// as the same logical item.
    pub fn dedup_key(&self) -> String {
        match self {
            PendingInput::Msg { record_id, .. } => format!("msg:{record_id}"),
            PendingInput::Event { event_id, .. } => format!("event:{event_id}"),
            PendingInput::Interrupt { id, .. } => format!("interrupt:{id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Ui,
    Work,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Running,
    WaitingInput,
    WaitingTool,
    Ended,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub kind: SessionKind,
    pub current_behavior: String,
    pub status: SessionStatus,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub one_line_status: String,
    #[serde(default)]
    pub pending_inputs: Vec<PendingInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_tunnel_did: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_subscriptions: Vec<EventSubscription>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_task_calls: Vec<PendingTaskCall>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub objective: String,
    #[serde(default)]
    pub bootstrap_done: bool,
    #[serde(default)]
    pub process_entry: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_stack: Vec<ProcessFrame>,
}

impl SessionMeta {
    pub fn new(
        session_id: String,
        kind: SessionKind,
        current_behavior: String,
        owner: String,
    ) -> Self {
        Self {
            session_id,
            kind,
            current_behavior: current_behavior.clone(),
            status: SessionStatus::Idle,
            owner,
            one_line_status: String::new(),
            pending_inputs: Vec::new(),
            peer_did: None,
            peer_tunnel_did: None,
            event_subscriptions: Vec::new(),
            workspace_id: None,
            pending_task_calls: Vec::new(),
            title: String::new(),
            objective: String::new(),
            bootstrap_done: false,
            process_entry: current_behavior,
            process_stack: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingTaskCall {
    pub call_id: String,
    pub tool_name: String,
    pub task_id: i64,
    pub event_pattern: String,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    pub kind: SessionKind,
    pub title: String,
    pub objective: String,
    pub status: SessionStatus,
    pub one_line_status: String,
    pub workspace_id: Option<String>,
    pub current_behavior: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessFrame {
    pub entry: String,
    pub current: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventSubscription {
    pub pattern: String,
    #[serde(default)]
    pub subscribed_at_ms: u64,
}
