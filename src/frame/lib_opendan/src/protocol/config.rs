//! `.opendan_agent_session/session_config.json` — launch configuration (§4.2).
//!
//! The file is replaced atomically as a whole; only a handful of fields
//! (dynamic subscriptions) may change while the session runs, each change
//! bumps `config_rev`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 5: `session.policy` (session template), `origin.report /
/// created_by_call`, `prompt.frozen` (behaviors frozen from the agent's
/// catalog) and `prompt.initial_inputs` (bootstrap material of a session
/// without an input queue). 4: input templates, `input.mode / media`,
/// `session.timezone`, event sources of subscriptions. Earlier versions are
/// read-only until migrated.
pub const SESSION_CONFIG_SCHEMA: &str = "opendan.session_config/5";

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
    /// How the parent session hears about this session (chosen by the
    /// parent when it created it). `None`: no implicit reporting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ReportMode>,
    /// `<run_id>/<call_id>` of the tool call that created the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_call: Option<String>,
}

/// What a parent receives from a sub session without subscribing: the
/// registry state stays the truth, these events only accelerate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportMode {
    /// Needs attention (waiting for input / a decision) and the end: Input.
    #[default]
    Final,
    /// `final` plus progress, kept as semi-subscription state.
    Progress,
    /// Nothing is pushed; the parent reads the registry itself.
    None,
}

/// Meaning of `WAIT_USER_MSG` in a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaitPolicy {
    /// The Turn stays open waiting for input.
    #[default]
    Allowed,
    /// Nobody answers: the Turn fails with `needs_user_input` and the
    /// session finishes as failed; the question is in the report.
    FinishFailed,
    /// Treated as the delivered result.
    FinishCompleted,
}

/// Material of the semi-subscription snapshot and the fresh view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObserveScope {
    Off,
    Events,
    #[default]
    EventsAndActive,
}

fn default_true() -> bool {
    true
}
fn default_process_depth() -> u8 {
    MAX_CALL_DEPTH as u8
}
fn default_sub_sessions() -> u8 {
    4
}
fn default_session_depth() -> u8 {
    2
}

/// `session.policy`: the session template resolved at creation (xAgent
/// §4.7) and frozen with the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionPolicy {
    #[serde(default)]
    pub wait_user_msg: WaitPolicy,
    #[serde(default)]
    pub observe: ObserveScope,
    #[serde(default = "default_true")]
    pub load_hints: bool,
    /// Callers on `process_stack` at most (sub contexts in progress).
    #[serde(default = "default_process_depth")]
    pub max_process_depth: u8,
    /// Sub sessions not finished at the same time.
    #[serde(default = "default_sub_sessions")]
    pub max_sub_sessions: u8,
    /// Nesting depth of sub sessions below this session's root.
    #[serde(default = "default_session_depth")]
    pub max_session_depth: u8,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            wait_user_msg: WaitPolicy::default(),
            observe: ObserveScope::default(),
            load_hints: true,
            max_process_depth: default_process_depth(),
            max_sub_sessions: default_sub_sessions(),
            max_session_depth: default_session_depth(),
        }
    }
}

impl SessionPolicy {
    pub fn is_default(&self) -> bool {
        self == &SessionPolicy::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndConditionType {
    LlmDeclaresDone,
    OutputSchema,
    /// Finish after `detail.n` (default 1) completed logical Turns; between
    /// them the session waits for input. Internal hand-overs (context
    /// switch, sub context call / return) are not Turns and cost nothing.
    MaxTurns,
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
    /// User time zone bound to the session (IANA name). Never the runner
    /// machine's zone: protocol times are UTC, and this value is shown to the
    /// agent through the default semi subscription
    /// ([`USER_TIMEZONE_SUBSCRIPTION`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "SessionPolicy::is_default")]
    pub policy: SessionPolicy,
}

fn default_class() -> String {
    "work".to_string()
}

/// Mechanical compression parameters for rendering worklog history (§4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MechanicalCompress {
    /// Model responses rendered in full, counted from the newest: the unit
    /// is one recorded response, i.e. a behavior `step` or a function call
    /// `assistant_message` (with the entries after it). Older entries are
    /// cut to `summary_chars`.
    #[serde(default = "default_recent_full_responses")]
    pub recent_full_responses: u32,
    #[serde(default = "default_summary_chars")]
    pub summary_chars: u32,
    #[serde(default = "default_max_result_chars")]
    pub max_result_chars: u32,
    #[serde(default = "default_drop_kinds")]
    pub drop_kinds: Vec<String>,
}

fn default_recent_full_responses() -> u32 {
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
        "event_dropped".to_string(),
        "turn_ended".to_string(),
    ]
}

impl Default for MechanicalCompress {
    fn default() -> Self {
        Self {
            recent_full_responses: default_recent_full_responses(),
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
    /// Entry behavior of the session (`behaviors/<name>` of the agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    /// Behaviors and identity frozen from the agent's catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<super::behavior::FrozenPrompt>,
    /// Bootstrap inputs of a session created without an input queue: `msg`
    /// records only, at most [`super::input::MAX_PENDING_INPUTS`], never
    /// changed after creation. The runner reads them as the read-only source
    /// [`BOOTSTRAP_SRC`] (index from 1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_inputs: Vec<super::input::PostedInput>,
    /// Application system prompt (S-05); composed after the agent identity
    /// and the non-overridable constraints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Input templates of the session's base context (a behavior entry's
    /// `prompt` replaces them per field). Absent: built-in templates.
    #[serde(flatten)]
    pub templates: InputTemplates,
    /// Input consumption of the session's base context.
    #[serde(default, skip_serializing_if = "InputSection::is_default")]
    pub input: InputSection,
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

/// How a behavior is entered when a hand-over (`next_behavior`) or a
/// `call_behavior` tool call names it. Decided by the target behavior, never
/// by the session as a whole; there is no default mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    /// The target has its own context (system, tools, history, run):
    /// created on first entry, resumed from its own snapshot afterwards.
    SwitchContext,
    /// A new sub context per call with the target's system and task input,
    /// plus an explicit selection of the caller's history; its result
    /// returns to the caller.
    CreateSubContext,
    /// A new branch per call that keeps the caller's system and its
    /// complete effective history at the fork point; its result returns to
    /// the caller.
    Fork,
}

impl ContextMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextMode::SwitchContext => "switch_context",
            ContextMode::CreateSubContext => "create_sub_context",
            ContextMode::Fork => "fork",
        }
    }

    /// The result returns to the caller.
    pub fn is_sub_context(&self) -> bool {
        !matches!(self, ContextMode::SwitchContext)
    }
}

/// History a new context starts with (besides its system and task input).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InheritMode {
    #[default]
    None,
    /// The session history rendered by the host (summary + recent worklog
    /// records): a filtered view, labelled `<session_history>`.
    RecentDialogue,
    /// The caller's completed steps as structured records
    /// (`create_sub_context`, both contexts in the behavior loop).
    Steps,
}

/// Templates of the three controlled inputs and of the semi-subscription
/// snapshot. A template's output is the whole user message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InputTemplates {
    /// Session bootstrap message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_init: Option<String>,
    /// Selected external msg / Input events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_input: Option<String>,
    /// Hand-over into the target context (switch, sub context, its return).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_context_switch: Option<String>,
    /// The snapshot message placed before a controlled input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semi_subscription_snapshot: Option<String>,
}

impl InputTemplates {
    /// `self` over `base`, field by field.
    pub fn over(&self, base: &InputTemplates) -> InputTemplates {
        InputTemplates {
            on_init: self.on_init.clone().or_else(|| base.on_init.clone()),
            on_input: self.on_input.clone().or_else(|| base.on_input.clone()),
            on_context_switch: self
                .on_context_switch
                .clone()
                .or_else(|| base.on_context_switch.clone()),
            semi_subscription_snapshot: self
                .semi_subscription_snapshot
                .clone()
                .or_else(|| base.semi_subscription_snapshot.clone()),
        }
    }
}

/// `prompt` of a behavior entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorPrompt {
    /// Application system prompt of the context (replaces `prompt.system`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_init: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_context_switch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semi_subscription_snapshot: Option<String>,
}

impl BehaviorPrompt {
    pub fn is_empty(&self) -> bool {
        self == &BehaviorPrompt::default()
    }

    pub fn templates(&self) -> InputTemplates {
        InputTemplates {
            on_init: self.on_init.clone(),
            on_input: self.on_input.clone(),
            on_context_switch: self.on_context_switch.clone(),
            semi_subscription_snapshot: self.semi_subscription_snapshot.clone(),
        }
    }
}

/// How many selected inputs one `on_input` batch takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// One msg / Input event per batch; the rest stays queued.
    Single,
    /// As many as the batch budget allows.
    #[default]
    Batch,
}

/// Whether attachments also enter the context as image / document blocks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputMedia {
    /// Text references only (`<attachment .../>` lines); the agent reads
    /// attachments with tools.
    #[default]
    Reference,
    /// Image / document attachments additionally as content blocks (at most
    /// [`MAX_INLINE_MEDIA`] per batch).
    Inline,
}

/// Image / document blocks one batch may carry (`input.media = inline`).
pub const MAX_INLINE_MEDIA: usize = 8;

/// Input consumption policy of a context (separate from its templates).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputSection {
    #[serde(default)]
    pub mode: InputMode,
    #[serde(default)]
    pub media: InputMedia,
}

impl InputSection {
    pub fn is_default(&self) -> bool {
        self == &InputSection::default()
    }
}

/// `extensions.opendan.behaviors.<name>`: entry configuration of a behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorEntry {
    pub mode: ContextMode,
    /// `prompt.system` and the input templates of the context. A system
    /// prompt is not allowed for `fork`.
    #[serde(default, skip_serializing_if = "BehaviorPrompt::is_empty")]
    pub prompt: BehaviorPrompt,
    /// Input consumption of the context (`None`: the session's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InputSection>,
    /// Top-level keys replacing those of `prompt.llm_context` (model, tools,
    /// limits …). Not allowed for `fork`.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub llm_context: Value,
    #[serde(default)]
    pub inherit: InheritMode,
}

impl BehaviorEntry {
    pub fn validate(&self, name: &str) -> std::result::Result<(), String> {
        if !self.llm_context.is_null() && !self.llm_context.is_object() {
            return Err(format!("behavior `{name}`: llm_context must be an object"));
        }
        match self.mode {
            ContextMode::Fork => {
                if self.prompt.system.is_some() || !self.llm_context.is_null() {
                    return Err(format!(
                        "behavior `{name}`: fork keeps the caller's system and configuration; use create_sub_context to change them"
                    ));
                }
                if self.inherit != InheritMode::None {
                    return Err(format!(
                        "behavior `{name}`: fork always inherits the complete history; `inherit` only applies to the other modes"
                    ));
                }
            }
            ContextMode::SwitchContext => {
                if self.inherit == InheritMode::Steps {
                    return Err(format!(
                        "behavior `{name}`: a switch_context target has its own history; `inherit: steps` only applies to create_sub_context"
                    ));
                }
            }
            ContextMode::CreateSubContext => {}
        }
        Ok(())
    }
}

/// Nested sub contexts (callers on `process_stack`) allowed in one session
/// by default (`session.policy.max_process_depth`).
pub const MAX_CALL_DEPTH: usize = 4;

/// Id of the read-only input source over `prompt.initial_inputs`.
pub const BOOTSTRAP_SRC: &str = "_bootstrap";

/// Tool a context calls to run a sub context (`create_sub_context` / `fork`
/// targets): `{behavior, task}`.
pub const TOOL_CALL_BEHAVIOR: &str = "call_behavior";

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
    /// A matching event is an Input: it wakes the session and enters a
    /// controlled input batch.
    Active,
    /// A matching event is an Observe: merged into `pending_events` and
    /// shown as the semi-subscription snapshot before the next controlled
    /// input; never triggers inference by itself.
    Semi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubscriptionSource {
    /// Another session of the same agent (pull: registry rev compare; always
    /// observed). Also matches bus events with `source = session:<ref>`.
    Session {
        #[serde(rename = "ref")]
        session_ref: String,
    },
    /// Object events bridged into the bus (`source = object:<object>`).
    /// `event` empty or `*` matches every event name.
    ObjectEvent {
        object: String,
        #[serde(default)]
        event: String,
    },
    /// A task (`source = task:<task_id>`).
    Task { task_id: String },
    /// A timer (`source = timer:<name>`).
    Timer { name: String },
    /// A system source (`source = system:<id>`).
    System {
        #[serde(default)]
        id: String,
    },
}

impl SubscriptionSource {
    /// Whether an event of `source_kind:source_id` named `event` belongs to
    /// this subscription (mechanical, structured fields only).
    pub fn matches(&self, source_kind: &str, source_id: &str, event: &str) -> bool {
        match self {
            SubscriptionSource::Session { session_ref } => {
                source_kind == "session" && source_id == session_ref
            }
            SubscriptionSource::ObjectEvent { object, event: e } => {
                source_kind == "object"
                    && source_id == object
                    && (e.is_empty() || e == "*" || e == event)
            }
            SubscriptionSource::Task { task_id } => source_kind == "task" && source_id == task_id,
            SubscriptionSource::Timer { name } => source_kind == "timer" && source_id == name,
            SubscriptionSource::System { id } => source_kind == "system" && source_id == id,
        }
    }
}

/// Id of the implicit semi subscription every session has on its user's
/// time zone (`source = system:user_timezone`).
pub const USER_TIMEZONE_SUBSCRIPTION: &str = "_user_timezone";
pub const USER_TIMEZONE_SOURCE_ID: &str = "user_timezone";

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
}

impl InputSourceConfig {
    pub fn id(&self) -> &str {
        match self {
            InputSourceConfig::Kmsg { id, .. } => id,
        }
    }
}

/// Reply coordinates of a session bound to one conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OutboundBinding {
    /// The peer (one-to-one) or the group.
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_session: Option<String>,
    /// MsgObject kind of the replies.
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Channels {
    #[serde(default)]
    pub inputs: Vec<InputSourceConfig>,
    /// Where the session's replies go, fixed when it is created: a reply
    /// whose route differs is not sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound: Option<OutboundBinding>,
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

    /// Subscriptions every session has without declaring them.
    pub fn implicit_subscriptions(&self) -> Vec<Subscription> {
        vec![Subscription {
            id: USER_TIMEZONE_SUBSCRIPTION.to_string(),
            mode: SubscriptionMode::Semi,
            source: SubscriptionSource::System {
                id: USER_TIMEZONE_SOURCE_ID.to_string(),
            },
            watch: Vec::new(),
        }]
    }

    /// The valid subscription an event belongs to: the one named by
    /// `subscription_id` (its source must match), else the first explicit or
    /// implicit subscription whose source matches.
    pub fn subscription_for(
        &self,
        subscription_id: Option<&str>,
        source_kind: &str,
        source_id: &str,
        event: &str,
    ) -> Option<Subscription> {
        let implicit = self.implicit_subscriptions();
        let mut all = self.subscriptions.iter().chain(implicit.iter());
        match subscription_id {
            Some(id) => all
                .find(|s| s.id == id)
                .filter(|s| s.source.matches(source_kind, source_id, event))
                .cloned(),
            None => all
                .find(|s| s.source.matches(source_kind, source_id, event))
                .cloned(),
        }
    }

    /// Input templates and consumption policy in effect for `behavior`
    /// (`None`: the session's base context).
    pub fn input_config(&self, entry: Option<&BehaviorEntry>) -> (InputTemplates, InputSection) {
        match entry {
            Some(e) => (
                e.prompt.templates().over(&self.prompt.templates),
                e.input.unwrap_or(self.prompt.input),
            ),
            None => (self.prompt.templates.clone(), self.prompt.input),
        }
    }

    /// Entry configuration of every behavior
    /// (`extensions.opendan.behaviors`), validated. `process_modes` (the
    /// session level table with a normal-switch fallback) is refused.
    pub fn behaviors(&self) -> std::result::Result<BTreeMap<String, BehaviorEntry>, String> {
        let Some(o) = self.extensions.get("opendan") else {
            return Ok(BTreeMap::new());
        };
        if o.get("process_modes").is_some() {
            return Err(
                "extensions.opendan.process_modes is no longer supported; declare extensions.opendan.behaviors.<name>.mode"
                    .to_string(),
            );
        }
        let Some(b) = o.get("behaviors") else {
            return Ok(BTreeMap::new());
        };
        let map: BTreeMap<String, BehaviorEntry> = serde_json::from_value(b.clone())
            .map_err(|e| format!("extensions.opendan.behaviors: {e}"))?;
        for (name, entry) in &map {
            entry.validate(name)?;
        }
        Ok(map)
    }

    /// Entry configuration of `behavior`. The session's initial behavior
    /// (`prompt.behavior`) without an entry of its own is a `switch_context`
    /// target running the session's base configuration. Any other behavior
    /// without an entry is an error: there is no fallback mode.
    pub fn behavior_entry(&self, behavior: &str) -> std::result::Result<BehaviorEntry, String> {
        if let Some(e) = self.behaviors()?.remove(behavior) {
            return Ok(e);
        }
        if self.prompt.behavior.as_deref() == Some(behavior) {
            return Ok(BehaviorEntry {
                mode: ContextMode::SwitchContext,
                prompt: BehaviorPrompt::default(),
                input: None,
                llm_context: Value::Null,
                inherit: InheritMode::RecentDialogue,
            });
        }
        Err(format!(
            "behavior `{behavior}` has no entry mode (extensions.opendan.behaviors.{behavior}.mode)"
        ))
    }
}
