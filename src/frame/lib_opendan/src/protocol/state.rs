//! `.opendan_agent_session/state.json` — the session commit point (§4.3).
//!
//! Small file, replaced atomically as a whole. Everything written in the same
//! commit (worklog appends, report.md, run checkpoints) lands before it; a
//! reader that sees a new `rev` can rely on the referenced content existing.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::ContextMode;
use super::input::{AgentEvent, EventReceipt, EventSource, ReplyRoute};

/// 5: session input protocol 3 — `pending_events`, `reply`,
/// `watched_tasks`, accepted Input events per source; `pending_task_calls`
/// removed. Earlier versions are read-only until migrated.
pub const SESSION_STATE_SCHEMA: &str = "opendan.session_state/6";

/// Upper bound on `inputs.recent_keys` (bounded dedup cache).
pub const RECENT_KEYS_LIMIT: usize = 256;
/// Upper bound on `consumed_above` per source before we warn.
pub const CONSUMED_ABOVE_WARN: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Created,
    Ready,
    Running,
    Waiting,
    Finished,
}

impl RunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunState::Created => "created",
            RunState::Ready => "ready",
            RunState::Running => "running",
            RunState::Waiting => "waiting",
            RunState::Finished => "finished",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, RunState::Running | RunState::Waiting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Acceptance {
    #[default]
    #[serde(rename = "n/a")]
    NotApplicable,
    Pending,
    Accepted,
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WriterInfo {
    pub runner_id: String,
    pub principal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub pid: u32,
    pub lock_epoch: u64,
}

/// What a waiting session waits for (`waiting_for.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaitingKind {
    Input,
    Tool,
    Event,
    Children,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaitingFor {
    pub kind: WaitingKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
}

/// Inputs of one logical Turn that entered a run. The input batch that
/// opens the Turn (or the first batch of an already open Turn in this run)
/// starts an entry; later batches of the same Turn (hand-over, observation,
/// supplementary input) extend it, so the list grows with Turns, not with
/// batches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TurnInputs {
    pub turn: u64,
    /// Stable input ids, e.g. `q#121`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Keys of the semi-subscription state versions injected with them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// Controlled input of the first batch (`on_init`, `on_input`,
    /// `on_context_switch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    /// `input_seq` of the first batch of this Turn in the run.
    #[serde(default)]
    pub input_seq: u64,
    #[serde(default)]
    pub at_ms: u64,
}

/// A run that is not finished yet (xllm run directory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LiveRun {
    pub run_id: String,
    #[serde(default)]
    pub turns: Vec<TurnInputs>,
    /// Receipt batches of this run already applied to state (§8.3).
    #[serde(default)]
    pub applied_input_seq: u64,
    /// Function call runs: messages after the history prefix (within
    /// `flushed_epoch`) already in the worklog.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flushed_message_count: u64,
    /// Behavior runs: steps with `step_index <` this are in the worklog
    /// (identity high-water mark, not a count: inherited steps of a fork
    /// child and steps rewritten away are never written).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flushed_step_index: u64,
    /// Behavior runs: receipts (injected messages) with `input_seq ≤` this
    /// are in the worklog.
    #[serde(default)]
    pub flushed_input_seq: u64,
    /// History epoch (`HostMeta.history_epoch`) `flushed_message_count`
    /// counts in. A snapshot of a newer epoch starts at zero: its whole
    /// history before the rewrite was flushed before the rewrite was
    /// published.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flushed_epoch: u64,
    /// Process entry the run belongs to (behavior process ↔ run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_entry: Option<String>,
    /// `at_ms` of the run's hand-over record (run.json `handover`) this
    /// state already committed: a record with this stamp is not a transfer
    /// left to do.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub handover_at_ms: u64,
}

/// Why a run sits in `process_stack`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FrameRole {
    /// A context left through SWITCH_CONTEXT: re-entered when a hand-over
    /// names its entry again.
    Parked,
    /// The caller of a sub context (create-sub-context / fork): live again
    /// when the child returns.
    Caller,
}

/// How a sub context was called, i.e. where its result returns to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallTrigger {
    /// `next_behavior` of a complete Step: the result returns as
    /// `process_result` in the caller's hand-over batch.
    Behavior,
    /// A tool call / behavior action: the caller is suspended on
    /// `PendingTool`, the result is filled as the tool result of `call_id`.
    Tool { call_id: String, task_id: String },
}

/// The sub context a `Caller` frame waits for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChildCall {
    /// `create_sub_context | fork` (how the child was constructed).
    pub mode: ContextMode,
    pub behavior: String,
    pub trigger: CallTrigger,
    /// Task given to the child (tool trigger: the call's `task` argument).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// Suspended behavior process (its run is kept, §4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessFrame {
    pub entry: String,
    pub role: FrameRole,
    /// `Caller` frames: the child being waited for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<ChildCall>,
    pub run_id: String,
    #[serde(default)]
    pub turns: Vec<TurnInputs>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flushed_message_count: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flushed_step_index: u64,
    #[serde(default)]
    pub flushed_input_seq: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flushed_epoch: u64,
    #[serde(default)]
    pub applied_input_seq: u64,
    /// `at_ms` of the hand-over record this suspension committed (see
    /// [`LiveRun::handover_at_ms`]).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub handover_at_ms: u64,
}

/// How a logical Turn ended (decided by the session from the outcome and
/// the hand-over state, never by the waist alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    /// A result was delivered (final answer, a reply before waiting for
    /// input, or the session finished).
    Completed,
    /// A non-recoverable error ended the run.
    Failed,
    /// The run's budget was exhausted.
    BudgetExhausted,
    /// `control(stop)`.
    Stopped,
}

/// The logical Turn in progress (one Input → result of the AgentSession).
///
/// Opened by the first input batch committed while no Turn is open
/// (bootstrap, msg / event). Context switches, sub context calls and
/// returns, observation injections, resumable
/// suspensions (interrupt, retryable error, context limit, pending tool),
/// history epoch rewrites and restarts keep it open; inputs consumed while
/// it is open join it. Only the session closes it, when it interprets an
/// outcome as the Turn's result or failure ([`TurnStatus`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenTurn {
    /// Session-wide Turn number, starting at 1.
    pub index: u64,
    /// Input batch that opened it: `(run_id, input_seq)`.
    pub run_id: String,
    pub input_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    /// msg / event inputs that make up the Turn's logical input: those of
    /// the opening batch, then those consumed while it was open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// A message (not only events) is among them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_msg: bool,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    /// Not handed to the outbound sink yet, or the hand-over failed in a
    /// way worth retrying.
    Pending,
    Sent,
    /// Refused (route mismatch, rejected by the message service); never
    /// retried.
    Failed,
}

/// Settled outbox entries kept for display.
pub const OUTBOX_KEEP_SETTLED: usize = 16;

/// What an outbox entry is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutboxPurpose {
    /// The reply of a Turn as a message of its own.
    #[default]
    Reply,
    /// "Accepted, working on it": sent while the Turn is open, replaced by
    /// the Turn's final edit.
    Placeholder,
    /// The reply of a Turn as an edit of its placeholder.
    FinalEdit,
}

impl OutboxPurpose {
    fn is_reply(&self) -> bool {
        *self == OutboxPurpose::Reply
    }
}

/// One outbound message of a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OutboxEntry {
    /// Idempotency key: `(sid, turn, run_id, n)`.
    pub key: String,
    /// The complete message, `created_at_ms` included: re-sent unchanged.
    #[schemars(with = "Value")]
    pub msg: ndn_lib::MsgObject,
    pub turn: u64,
    #[serde(default, skip_serializing_if = "OutboxPurpose::is_reply")]
    pub purpose: OutboxPurpose,
    pub status: OutboxStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub updated_at_ms: u64,
}

/// Settled Turn task bindings kept (independent of the outbox trimming).
pub const TURN_TASKS_KEEP_SETTLED: usize = 64;

/// A Turn and the task the host's task service keeps for it. Written when
/// the task was created (the Turn is open), closed in the commit that closes
/// the Turn, `reported` once the task service took the terminal state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TurnTask {
    pub turn: u64,
    pub task_id: String,
    /// The batch that opened the Turn had a message: a placeholder may be
    /// sent for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_msg: bool,
    pub opened_at_ms: u64,
    /// ObjId of the placeholder message of this Turn (the anchor its final
    /// edit targets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<TurnStatus>,
    /// Result summary handed to the task service with the terminal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reported: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorklogBoundary {
    pub committed_seq: u64,
    pub committed_bytes: u64,
}

/// Consumption progress of one input source (§4.5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceProgress {
    /// Highest index acknowledged-able: every index ≤ this was consumed.
    #[serde(default)]
    pub acked_index: u64,
    /// Consumed indices above `acked_index` (selective consumption).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub consumed_above: BTreeSet<u64>,
    /// Input events accepted by an active subscription that are still
    /// pending: their routing was decided when they were first seen, so a
    /// later `unsubscribe` does not turn them into dropped events.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub accepted: BTreeSet<u64>,
}

impl SourceProgress {
    pub fn is_consumed(&self, index: u64) -> bool {
        index <= self.acked_index || self.consumed_above.contains(&index)
    }

    /// Mark `index` consumed and fold the contiguous prefix into
    /// `acked_index`. Never moves backwards.
    pub fn mark(&mut self, index: u64) {
        self.accepted.remove(&index);
        if index <= self.acked_index {
            return;
        }
        self.consumed_above.insert(index);
        while self.consumed_above.remove(&(self.acked_index + 1)) {
            self.acked_index += 1;
        }
    }

    /// Mark every index `< first_available` consumed: the queue no longer
    /// holds anything below it (deleted / retention), so the contiguous
    /// prefix may advance past the hole.
    pub fn skip_below(&mut self, first_available: u64) {
        if first_available == 0 || first_available <= self.acked_index + 1 {
            return;
        }
        self.acked_index = first_available - 1;
        self.consumed_above.retain(|i| *i > first_available - 1);
        self.accepted.retain(|i| *i > first_available - 1);
        while self.consumed_above.remove(&(self.acked_index + 1)) {
            self.acked_index += 1;
        }
    }
}

/// Bounded dedup cache of producer `key`s (§4.5). It only filters duplicate
/// deliveries; it never replaces the receipts persisted in snapshots or the
/// per-source consumption positions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecentKeys(pub Vec<String>);

impl RecentKeys {
    pub fn contains(&self, key: &str) -> bool {
        self.0.iter().any(|k| k == key)
    }

    pub fn push(&mut self, key: &str) {
        if key.is_empty() || self.contains(key) {
            return;
        }
        self.0.push(key.to_string());
        if self.0.len() > RECENT_KEYS_LIMIT {
            let drop = self.0.len() - RECENT_KEYS_LIMIT;
            self.0.drain(0..drop);
        }
    }
}

/// Delivery position of a bus record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InputPos {
    pub src: String,
    pub index: u64,
}

/// One version of an observed source's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingEventVersion {
    #[serde(default)]
    pub seq: Option<u64>,
    pub key: String,
    pub event: String,
    pub summary: String,
    #[serde(default)]
    pub data_ref: Option<String>,
    pub received_at_ms: u64,
    /// Where it was delivered (`None`: synthesized by the runner, e.g. a
    /// pull-mode session subscription).
    #[serde(default)]
    pub input: Option<InputPos>,
}

/// Semi-subscription state waiting to be injected: the newest version of
/// one `(subscription_id, source)`, its terminal event kept separately.
/// Saved when received (the delivery is then consumed); cleared only by the
/// receipt of the batch that injected exactly these versions, by
/// `unsubscribe`, or replaced by a newer version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingEvent {
    #[serde(default)]
    pub subscription_id: Option<String>,
    pub source: EventSource,
    #[serde(default)]
    pub latest: Option<PendingEventVersion>,
    /// Never replaced by later non-terminal versions.
    #[serde(default)]
    pub terminal: Option<PendingEventVersion>,
    /// Versions replaced before they were injected.
    #[serde(default)]
    pub superseded: u64,
}

/// What merging an Observe event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeResult {
    /// Saved as the newest (or terminal) version.
    Saved,
    /// Older than the version kept (`seq`), or the same key again: ignored.
    Stale,
}

/// Merge an Observe event into `pending` (same protocol version only).
/// With `seq` on both sides an older version never replaces a newer one;
/// without it the consumption order decides and `key` tells versions apart.
pub fn merge_pending_event(
    pending: &mut Vec<PendingEvent>,
    subscription_id: Option<&str>,
    ev: &AgentEvent,
    key: &str,
    input: Option<InputPos>,
    now_ms: u64,
) -> MergeResult {
    let version = PendingEventVersion {
        seq: ev.seq,
        key: key.to_string(),
        event: ev.event.clone(),
        summary: ev.summary.clone(),
        data_ref: ev.data_ref.clone(),
        received_at_ms: now_ms,
        input,
    };
    let pos = pending
        .iter()
        .position(|p| p.subscription_id.as_deref() == subscription_id && p.source == ev.source);
    let entry = match pos {
        Some(i) => &mut pending[i],
        None => {
            pending.push(PendingEvent {
                subscription_id: subscription_id.map(str::to_string),
                source: ev.source.clone(),
                latest: None,
                terminal: None,
                superseded: 0,
            });
            pending.last_mut().expect("just pushed")
        }
    };
    let slot = if ev.terminal {
        &mut entry.terminal
    } else {
        &mut entry.latest
    };
    if let Some(old) = slot.as_ref() {
        let older = matches!((version.seq, old.seq), (Some(new), Some(kept)) if new < kept);
        if older || old.key == version.key {
            return MergeResult::Stale;
        }
        entry.superseded += 1;
    }
    *slot = Some(version);
    MergeResult::Saved
}

/// Remove exactly the versions a batch injected: matched by
/// `(subscription_id, source, key)` and by `seq` when the receipt has one.
/// A newer version that arrived meanwhile stays.
pub fn clear_pending_events(pending: &mut Vec<PendingEvent>, injected: &[EventReceipt]) {
    for r in injected {
        for p in pending
            .iter_mut()
            .filter(|p| p.subscription_id == r.subscription_id && p.source == r.source)
        {
            let hit = |v: &Option<PendingEventVersion>| {
                v.as_ref()
                    .is_some_and(|v| v.key == r.key && (r.seq.is_none() || v.seq == r.seq))
            };
            if hit(&p.latest) {
                p.latest = None;
            }
            if hit(&p.terminal) {
                p.terminal = None;
            }
        }
    }
    pending.retain(|p| p.latest.is_some() || p.terminal.is_some());
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Touching {
    /// `path | object | artifact`.
    pub kind: String,
    #[serde(rename = "ref")]
    pub target: String,
    /// `read | write`.
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub since_ms: u64,
}

fn default_mode() -> String {
    "write".to_string()
}

/// Upper bound on `activity.touching`.
pub const TOUCHING_LIMIT: usize = 16;

/// What this session is doing and roughly what it modifies (§6.7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Activity {
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touching: Vec<Touching>,
    #[serde(default)]
    pub heartbeat_ms: u64,
}

impl Activity {
    pub fn is_empty(&self) -> bool {
        self.summary.is_empty() && self.touching.is_empty()
    }

    /// Merge a touching item; the most recent declaration wins, the list is
    /// bounded by [`TOUCHING_LIMIT`] (oldest dropped).
    pub fn touch(&mut self, item: Touching) {
        if let Some(existing) = self
            .touching
            .iter_mut()
            .find(|t| t.kind == item.kind && t.target == item.target)
        {
            if item.mode == "write" {
                existing.mode = item.mode;
            }
            return;
        }
        self.touching.push(item);
        if self.touching.len() > TOUCHING_LIMIT {
            let drop = self.touching.len() - TOUCHING_LIMIT;
            self.touching.drain(0..drop);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Topic {
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_report: Option<super::ReportSubmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_report: Option<super::ReportSubmission>,
    pub schema: String,
    pub rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writer: Option<WriterInfo>,
    pub run_state: RunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<WaitingFor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default)]
    pub acceptance: Acceptance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Number of the latest Turn opened (0 before the first input batch).
    #[serde(default)]
    pub turn_seq: u64,
    /// The Turn in progress, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_turn: Option<OpenTurn>,
    /// Turns closed with [`TurnStatus::Completed`].
    #[serde(default)]
    pub turns_completed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_entry: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_stack: Vec<ProcessFrame>,
    #[serde(default)]
    pub bootstrap_done: bool,
    #[serde(default)]
    pub topic: Topic,
    #[serde(default)]
    pub live_run: Option<LiveRun>,
    #[serde(default)]
    pub last_run: Option<String>,
    #[serde(default)]
    pub worklog: WorklogBoundary,
    /// Per input source consumption progress, keyed by source id.
    #[serde(default)]
    pub inputs: BTreeMap<String, SourceProgress>,
    #[serde(default)]
    pub recent_keys: RecentKeys,
    /// Pull-mode cursors only (registry rev of session subscriptions).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub subscription_cursors: BTreeMap<String, Value>,
    /// Semi-subscription state received and not injected yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_events: Vec<PendingEvent>,
    /// Default reply path: the way the last consumed input message came,
    /// else the parent session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<ReplyRoute>,
    /// Replies produced by committed Turns and their delivery: written in
    /// the commit that closes the Turn, sent afterwards, re-sent as they are
    /// after a restart (the idempotency key and the message never change).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outbox: Vec<OutboxEntry>,
    /// Turns bound to a task of the host's task service: the open Turn's,
    /// closed ones whose terminal state is not reported yet, and the most
    /// recent settled ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turn_tasks: Vec<TurnTask>,
    /// Background tasks of ended runs the runner follows for this session
    /// (implicit active subscriptions on `task:<task_id>`): when one ends,
    /// its completion is handled like a subscribed event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watched_tasks: Vec<String>,
    #[serde(default)]
    pub perception_seq: u64,
    #[serde(default)]
    pub reported_rev: u64,
    #[serde(default)]
    pub activity: Activity,
    #[serde(default)]
    pub one_line_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<Value>,
    /// Decisions seen in the queue but not applied yet (UI display).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_decision: Option<Value>,
    /// Stop requested through `control(stop)`; applied at the next safe point.
    #[serde(default)]
    pub stop_requested: bool,
    /// An input batch to commit without new input (context switch, sub
    /// context call / return through `next_behavior`); it joins the open Turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_continuation: Option<String>,
    /// Result of a sub context, handed to its caller: `{behavior, result,
    /// status: ok | failed | needs_user_input, next_action_id,
    /// next_step_index}`. Behavior trigger: rendered into the caller's
    /// hand-over batch. Tool trigger: carries `call_id` and is filled as that
    /// call's tool result when the caller's run is opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_result: Option<Value>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

impl SessionState {
    pub fn initial(worklog: WorklogBoundary, now_ms: u64) -> Self {
        Self {
            schema: SESSION_STATE_SCHEMA.to_string(),
            latest_report: None,
            final_report: None,
            rev: 1,
            writer: None,
            run_state: RunState::Created,
            waiting_for: None,
            outcome: None,
            acceptance: Acceptance::NotApplicable,
            result: None,
            turn_seq: 0,
            open_turn: None,
            turns_completed: 0,
            current_behavior: None,
            process_entry: None,
            process_stack: Vec::new(),
            bootstrap_done: false,
            topic: Topic::default(),
            live_run: None,
            last_run: None,
            worklog,
            inputs: BTreeMap::new(),
            recent_keys: RecentKeys::default(),
            subscription_cursors: BTreeMap::new(),
            pending_events: Vec::new(),
            reply: None,
            outbox: Vec::new(),
            turn_tasks: Vec::new(),
            watched_tasks: Vec::new(),
            perception_seq: 0,
            reported_rev: 0,
            activity: Activity::default(),
            one_line_status: String::new(),
            last_error: None,
            pending_decision: None,
            stop_requested: false,
            internal_continuation: None,
            process_result: None,
            updated_at_ms: now_ms,
        }
    }

    /// Initial state of a session created from `config`: the parent
    /// session as the default reply path (until a message arrives), and the
    /// user's time zone as the first state of its implicit semi
    /// subscription (shown with the first controlled input).
    pub fn initial_for(
        config: &super::config::SessionConfig,
        worklog: WorklogBoundary,
        now_ms: u64,
    ) -> Self {
        let mut state = Self::initial(worklog, now_ms);
        if let Some(parent) = config
            .session
            .origin
            .as_ref()
            .and_then(|o| o.parent_session.clone())
        {
            state.reply = Some(ReplyRoute::ParentSession { session_id: parent });
        }
        if let Some(tz) = config.session.timezone.as_ref().filter(|t| !t.is_empty()) {
            let ev = AgentEvent {
                subscription_id: Some(super::config::USER_TIMEZONE_SUBSCRIPTION.to_string()),
                source: EventSource::new("system", super::config::USER_TIMEZONE_SOURCE_ID),
                event: "timezone".to_string(),
                seq: None,
                summary: format!("The user's time zone is {tz}. Times in inputs are UTC."),
                data_ref: None,
                terminal: false,
            };
            merge_pending_event(
                &mut state.pending_events,
                Some(super::config::USER_TIMEZONE_SUBSCRIPTION),
                &ev,
                &format!("timezone:{tz}"),
                None,
                now_ms,
            );
        }
        state
    }

    /// Records of the bus not consumed yet, given the source's newest index.
    pub fn pending_inputs(&self, src: &str, last_index: u64) -> usize {
        let p = self.source(src);
        let above = p.consumed_above.iter().filter(|i| **i <= last_index).count() as u64;
        last_index.saturating_sub(p.acked_index).saturating_sub(above) as usize
    }

    pub fn source_mut(&mut self, id: &str) -> &mut SourceProgress {
        self.inputs.entry(id.to_string()).or_default()
    }

    pub fn source(&self, id: &str) -> SourceProgress {
        self.inputs.get(id).cloned().unwrap_or_default()
    }

    /// Run ids this state references (kept by `reconcile_runs`).
    pub fn referenced_runs(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        if let Some(l) = &self.live_run {
            out.insert(l.run_id.clone());
        }
        if let Some(l) = &self.last_run {
            out.insert(l.clone());
        }
        for p in &self.process_stack {
            out.insert(p.run_id.clone());
        }
        out
    }

    pub fn references(&self, run_id: &str) -> bool {
        self.referenced_runs().contains(run_id)
    }

    /// The call the live (or about to be created) run answers, if it is a
    /// sub context.
    pub fn child_call(&self) -> Option<&ChildCall> {
        self.process_stack
            .last()
            .filter(|f| f.role == FrameRole::Caller)
            .and_then(|f| f.call.as_ref())
    }

    /// The live run is a caller whose tool-triggered sub context returned:
    /// its result waits to be filled as the call's tool result.
    pub fn tool_return_pending(&self) -> bool {
        self.live_run.is_some()
            && self
                .process_result
                .as_ref()
                .is_some_and(|r| r.get("call_id").is_some())
    }

    /// Sub contexts in progress (nesting depth of the live run).
    pub fn call_depth(&self) -> usize {
        self.process_stack
            .iter()
            .filter(|f| f.role == FrameRole::Caller)
            .count()
    }

    pub fn is_finished(&self) -> bool {
        self.run_state == RunState::Finished
    }

    /// Whether the session can still consume input: not finished, or
    /// finished with a decision it can take (acceptance pending, accepted —
    /// it can still be discarded — or a `decide` queued). A session that
    /// does not gives its input queue back.
    pub fn takes_input(&self) -> bool {
        !self.is_finished()
            || matches!(self.acceptance, Acceptance::Pending | Acceptance::Accepted)
            || self.pending_decision.is_some()
    }

    /// The task of the open Turn, if it has one.
    pub fn open_turn_task(&self) -> Option<&TurnTask> {
        let turn = self.open_turn.as_ref()?.index;
        self.turn_tasks.iter().find(|t| t.turn == turn)
    }

    /// The Turn entries are attributed to: the open one, else the last one.
    pub fn current_turn(&self) -> u64 {
        self.open_turn
            .as_ref()
            .map(|t| t.index)
            .unwrap_or(self.turn_seq)
    }
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: Option<u64>, summary: &str, terminal: bool) -> AgentEvent {
        AgentEvent {
            subscription_id: Some("s".into()),
            source: EventSource::new("object", "o"),
            event: "changed".into(),
            seq,
            summary: summary.into(),
            data_ref: None,
            terminal,
        }
    }

    fn receipt(key: &str, seq: Option<u64>) -> EventReceipt {
        EventReceipt {
            subscription_id: Some("s".into()),
            source: EventSource::new("object", "o"),
            seq,
            key: key.into(),
        }
    }

    #[test]
    fn an_older_version_never_replaces_a_newer_pending_one() {
        let mut p = Vec::new();
        assert_eq!(
            merge_pending_event(&mut p, Some("s"), &ev(Some(8), "v8", false), "k8", None, 1),
            MergeResult::Saved
        );
        assert_eq!(
            merge_pending_event(&mut p, Some("s"), &ev(Some(7), "v7", false), "k7", None, 2),
            MergeResult::Stale
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].latest.as_ref().unwrap().key, "k8");
        assert_eq!(p[0].superseded, 0);
        // The same delivery again changes nothing.
        assert_eq!(
            merge_pending_event(&mut p, Some("s"), &ev(Some(8), "v8", false), "k8", None, 3),
            MergeResult::Stale
        );
    }

    #[test]
    fn committing_v7_keeps_v8_that_arrived_meanwhile() {
        let mut p = Vec::new();
        merge_pending_event(&mut p, Some("s"), &ev(Some(7), "v7", false), "k7", None, 1);
        // v7 is being injected (its receipt is written); v8 arrives.
        let injected = vec![receipt("k7", Some(7))];
        merge_pending_event(&mut p, Some("s"), &ev(Some(8), "v8", false), "k8", None, 2);
        clear_pending_events(&mut p, &injected);
        assert_eq!(p.len(), 1, "v8 is still to be injected");
        assert_eq!(p[0].latest.as_ref().unwrap().key, "k8");
        clear_pending_events(&mut p, &[receipt("k8", Some(8))]);
        assert!(p.is_empty());
    }

    #[test]
    fn without_seq_the_key_tells_versions_apart() {
        let mut p = Vec::new();
        merge_pending_event(&mut p, Some("s"), &ev(None, "first", false), "a", None, 1);
        let injected = vec![receipt("a", None)];
        merge_pending_event(&mut p, Some("s"), &ev(None, "second", false), "b", None, 2);
        assert_eq!(p[0].superseded, 1);
        clear_pending_events(&mut p, &injected);
        assert_eq!(p[0].latest.as_ref().unwrap().summary, "second");
    }

    #[test]
    fn a_terminal_version_is_kept_next_to_the_latest() {
        let mut p = Vec::new();
        merge_pending_event(&mut p, Some("s"), &ev(Some(3), "done", true), "t", None, 1);
        merge_pending_event(&mut p, Some("s"), &ev(Some(4), "late", false), "l", None, 2);
        assert!(p[0].terminal.is_some() && p[0].latest.is_some());
        clear_pending_events(&mut p, &[receipt("l", Some(4))]);
        assert_eq!(p.len(), 1, "only what was injected is cleared");
        assert!(p[0].terminal.is_some() && p[0].latest.is_none());
    }

    #[test]
    fn pending_inputs_do_not_count_consumed_positions() {
        let mut st = SessionState::initial(WorklogBoundary::default(), 0);
        assert_eq!(st.pending_inputs("q", 10), 10);
        st.source_mut("q").mark(1);
        st.source_mut("q").mark(2);
        st.source_mut("q").mark(5);
        assert_eq!(st.pending_inputs("q", 10), 7);
    }
}
