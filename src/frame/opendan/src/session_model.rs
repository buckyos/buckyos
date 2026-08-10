use std::collections::BTreeMap;

use buckyos_api::{match_event_patterns, AiMessage};
use chrono::{DateTime, FixedOffset};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

pub const UI_CLOCK_TIMER_EVENT_ID: &str = "timer";
pub const UI_CLOCK_TIMER_INTERVAL_MS: u64 = 2 * 60 * 1000;

// AgentSession lifecycle invariants.
//
// END is terminal for the current worker run: `on_wakeup` is not triggered after
// END, and implicit routing must not reopen the same session. Follow-up user
// input that belongs to the same topic creates a new Work-family session and
// may copy/read old context explicitly at a higher layer. Routing metadata is
// kept in `.meta/session.json` for audit/debug after END, but it is not an
// active dispatch target.

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
        ai_message: AiMessage,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Ui,
    Work,
    SelfCheck,
    SelfImprove,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Ui => "ui",
            SessionKind::Work => "work",
            SessionKind::SelfCheck => "self_check",
            SessionKind::SelfImprove => "self_improve",
        }
    }

    pub fn is_work_family(self) -> bool {
        matches!(
            self,
            SessionKind::Work | SessionKind::SelfCheck | SessionKind::SelfImprove
        )
    }
}

impl Serialize for SessionKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SessionKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.trim() {
            "ui" => SessionKind::Ui,
            "work" => SessionKind::Work,
            "self_check" => SessionKind::SelfCheck,
            "self_improve" => SessionKind::SelfImprove,
            // Migration guard: old or experimental data must not fail a
            // restore just because the kind string drifted. Unknown
            // non-UI sessions get the Work body semantics.
            _ => SessionKind::Work,
        })
    }
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
    pub status_changed_at_ms: u64,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_events: Vec<BgEventSnapshot>,
    #[serde(default, skip_serializing_if = "BackgroundHintState::is_empty")]
    pub background_hint_state: BackgroundHintState,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub last_schedule_task_list_access_at: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub last_workspace_list_access_at: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub last_notebook_last_items_access_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_task_calls: Vec<PendingTaskCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub improvement_budget: Option<ImprovementBudget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_improvement_tasks: Vec<ImprovementTask>,
    #[serde(default, skip_serializing_if = "AlreadyImprovedState::is_empty")]
    pub already_improved: AlreadyImprovedState,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report_delivery: Option<ReportDeliveryState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_continuation: Option<InternalContinuation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_binding: Option<AgentTaskBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_profile: Option<SessionLogicalProfile>,
    /// Self-Check wakeup-gating state (see doc/opendan/Self-Check Behavior.md
    /// §8.3). `seen_item_update_secs` is the max notebook-item `updated_at`
    /// (unix secs) we have already acted on — a newer value means the Notebook
    /// changed and the next timer tick should run a real round.
    /// `last_round_at_ms` / `idle_heartbeats` drive the heartbeat that forces a
    /// round every 15min, backing off to 30min while the Notebook stays quiet.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub self_check_seen_item_update_secs: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub self_check_last_round_at_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub self_check_idle_heartbeats: u32,
}

impl SessionMeta {
    pub fn new(
        session_id: String,
        kind: SessionKind,
        current_behavior: String,
        owner: String,
    ) -> Self {
        let mut meta = Self {
            session_id,
            kind,
            current_behavior: current_behavior.clone(),
            status: SessionStatus::Idle,
            status_changed_at_ms: 0,
            owner,
            one_line_status: String::new(),
            pending_inputs: Vec::new(),
            peer_did: None,
            peer_tunnel_did: None,
            event_subscriptions: Vec::new(),
            background_events: Vec::new(),
            background_hint_state: BackgroundHintState::default(),
            last_schedule_task_list_access_at: 0,
            last_workspace_list_access_at: 0,
            last_notebook_last_items_access_at: 0,
            workspace_id: None,
            pending_task_calls: Vec::new(),
            improvement_budget: None,
            pending_improvement_tasks: Vec::new(),
            already_improved: AlreadyImprovedState::default(),
            title: String::new(),
            objective: String::new(),
            bootstrap_done: false,
            process_entry: current_behavior,
            process_stack: Vec::new(),
            last_report_delivery: None,
            internal_continuation: None,
            task_binding: None,
            session_profile: None,
            self_check_seen_item_update_secs: 0,
            self_check_last_round_at_ms: 0,
            self_check_idle_heartbeats: 0,
        };
        meta.ensure_default_event_subscriptions(0);
        meta
    }

    pub fn ensure_default_event_subscriptions(&mut self, subscribed_at_ms: u64) -> bool {
        if !matches!(self.kind, SessionKind::Ui) {
            return false;
        }
        if self
            .event_subscriptions
            .iter()
            .any(|sub| sub.pattern == UI_CLOCK_TIMER_EVENT_ID)
        {
            return false;
        }
        self.event_subscriptions.push(EventSubscription {
            pattern: UI_CLOCK_TIMER_EVENT_ID.to_string(),
            subscribed_at_ms,
            mode: EventSubscriptionMode::BackgroundOnly,
            message_template: None,
        });
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTaskBinding {
    pub task_id: String,
    pub root_id: String,
    pub task_type: String,
    pub runner: String,
    pub task_name: String,
    pub user_id: String,
    pub app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportDeliveryState {
    pub report_hash: String,
    pub phase: String,
    pub report_id: String,
    pub delivered_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingTaskCall {
    pub call_id: String,
    pub tool_name: String,
    pub task_id: String,
    pub event_pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImprovementBudget {
    pub unit: ImprovementBudgetUnit,
    pub remaining: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementBudgetUnit {
    Token,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImprovementTask {
    pub task_id: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_report: String,
    pub created_at_ms: u64,
    pub status: ImprovementTaskStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementTaskStatus {
    Pending,
    Dispatched,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AlreadyImprovedState {
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub committed_round_index: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub committed_at_ms: u64,
}

impl AlreadyImprovedState {
    pub fn is_empty(&self) -> bool {
        self.committed_round_index == 0 && self.committed_at_ms == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionLogicalProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<LogicalTreeOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogicalTreeOverlay {
    pub path: String,
    #[serde(default)]
    pub merge_mode: OverlayMergeMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub items: BTreeMap<String, SessionModelItem>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub item_overrides: BTreeMap<String, SessionModelItemPatch>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exact_model_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_line: Option<SessionModelDisable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMergeMode {
    Inherit,
    Replace,
}

impl Default for OverlayMergeMode {
    fn default() -> Self {
        OverlayMergeMode::Inherit
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionModelItem {
    pub target: String,
    #[serde(default = "default_model_item_weight")]
    pub weight: f64,
}

impl SessionModelItem {
    pub fn new(target: impl Into<String>, weight: f64) -> Self {
        Self {
            target: target.into(),
            weight,
        }
    }
}

fn default_model_item_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionModelItemPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionModelDisable {
    #[serde(default, skip_serializing_if = "is_false")]
    pub streaming: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_call: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub json_schema: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub web_search: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub vision: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_context_tokens: Option<u64>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRef {
    pub event_id: String,
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BgEventSnapshot {
    pub event_id: String,
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BackgroundHintState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hint_fingerprints: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub last_non_empty_background_hints_at_ms: u64,
}

impl BackgroundHintState {
    pub fn is_empty(&self) -> bool {
        self.hint_fingerprints.is_empty() && self.last_non_empty_background_hints_at_ms == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundHint {
    pub path: String,
    pub kind: String,
    pub text: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "serde_json_value_is_null")]
    pub data: serde_json::Value,
}

fn serde_json_value_is_null(value: &serde_json::Value) -> bool {
    value.is_null()
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessFrame {
    pub entry: String,
    pub current: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub fork: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InternalContinuation {
    pub reason: String,
    #[serde(default)]
    pub user_messages: Vec<AiMessage>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventSubscription {
    pub pattern: String,
    #[serde(default)]
    pub subscribed_at_ms: u64,
    #[serde(default)]
    pub mode: EventSubscriptionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_template: Option<String>,
}

impl EventSubscription {
    pub fn matches(&self, event_id: &str) -> bool {
        match_event_patterns(std::slice::from_ref(&self.pattern), event_id)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventSubscriptionMode {
    Full,
    BackgroundOnly,
}

impl Default for EventSubscriptionMode {
    fn default() -> Self {
        EventSubscriptionMode::Full
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimerReason {
    pub trigger_type: TimerTriggerType,
    pub target_type: TimerTargetType,
    pub target_id: String,
    pub expected_trigger_time: DateTime<FixedOffset>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimerTriggerType {
    HardBarrier,
    PreciseTrigger,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimerEventKind {
    ReminderCheck,
    HardBarrier,
    ScheduledTaskCheck,
}

impl TimerEventKind {
    pub fn event_id(self) -> &'static str {
        match self {
            TimerEventKind::ReminderCheck => "timer.reminder_check",
            TimerEventKind::HardBarrier => "timer.hard_barrier",
            TimerEventKind::ScheduledTaskCheck => "timer.scheduled_task_check",
        }
    }

    pub fn parse_event_id(value: &str) -> Option<Self> {
        match value.trim() {
            "timer.reminder_check" => Some(TimerEventKind::ReminderCheck),
            "timer.hard_barrier" => Some(TimerEventKind::HardBarrier),
            "timer.scheduled_task_check" => Some(TimerEventKind::ScheduledTaskCheck),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerTargetType {
    Reminder,
    ScheduledTask,
    Other,
    Named(String),
}

impl Serialize for TimerTargetType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value = match self {
            TimerTargetType::Reminder => "reminder",
            TimerTargetType::ScheduledTask => "scheduled_task",
            TimerTargetType::Other => "other",
            TimerTargetType::Named(value) => value.as_str(),
        };
        serializer.serialize_str(value)
    }
}

impl<'de> Deserialize<'de> for TimerTargetType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(de::Error::custom("timer target_type must not be empty"));
        }
        Ok(match trimmed {
            "reminder" => TimerTargetType::Reminder,
            "scheduled_task" => TimerTargetType::ScheduledTask,
            "other" => TimerTargetType::Other,
            value => TimerTargetType::Named(value.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_kind_unknown_deserializes_to_work() {
        let kind: SessionKind = serde_json::from_str("\"old_experiment\"").unwrap();
        assert_eq!(kind, SessionKind::Work);
        assert_eq!(
            serde_json::to_string(&SessionKind::SelfCheck).unwrap(),
            "\"self_check\""
        );
    }

    #[test]
    fn session_profile_round_trips_aicc_overlay_shape() {
        let profile = SessionLogicalProfile {
            name: Some("only-local".to_string()),
            overlays: vec![LogicalTreeOverlay {
                path: "llm".to_string(),
                merge_mode: OverlayMergeMode::Replace,
                items: [(
                    "local".to_string(),
                    SessionModelItem::new("qwen3@local", 1.0),
                )]
                .into_iter()
                .collect(),
                item_overrides: BTreeMap::new(),
                exact_model_weights: BTreeMap::new(),
                disable_line: Some(SessionModelDisable {
                    web_search: true,
                    ..Default::default()
                }),
                fallback: None,
                route_policy_override: Some(json!({
                    "allow_fallback": {
                        "value": false,
                        "locked": false
                    }
                })),
                source: None,
            }],
            route_policy_override: None,
        };
        let value = serde_json::to_value(&profile).unwrap();

        assert_eq!(value["overlays"][0]["merge_mode"], "replace");
        assert_eq!(
            value["overlays"][0]["items"]["local"]["target"],
            "qwen3@local"
        );
        assert_eq!(value["overlays"][0]["disable_line"]["web_search"], true);

        let restored: SessionLogicalProfile = serde_json::from_value(value).unwrap();
        assert_eq!(restored, profile);
    }

    #[test]
    fn timer_reason_round_trips_fixed_schema() {
        let value = json!({
            "trigger_type": "precise_trigger",
            "target_type": "reminder",
            "target_id": "reminder_123",
            "expected_trigger_time": "2026-05-24T15:00:00-07:00",
            "reason": "check whether reminder_123 should be delivered"
        });
        let reason: TimerReason = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(reason.trigger_type, TimerTriggerType::PreciseTrigger);
        assert_eq!(reason.target_type, TimerTargetType::Reminder);
        assert_eq!(reason.target_id, "reminder_123");
        assert_eq!(serde_json::to_value(reason).unwrap(), value);
    }

    #[test]
    fn timer_event_kind_is_closed_namespace() {
        assert_eq!(
            TimerEventKind::parse_event_id("timer.reminder_check"),
            Some(TimerEventKind::ReminderCheck)
        );
        assert!(TimerEventKind::parse_event_id("timer.anything_else").is_none());
    }
}
