//! `.opendan_agent_session/state.json` — the session commit point (§4.3).
//!
//! Small file, replaced atomically as a whole. Everything written in the same
//! commit (worklog appends, report.md, run checkpoints) lands before it; a
//! reader that sees a new `rev` can rely on the referenced content existing.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SESSION_STATE_SCHEMA: &str = "opendan.session_state/1";

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaitingFor {
    /// `input | tool | event | children`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
}

/// Inputs consumed by one round of a run (written into `round_started`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoundInputs {
    pub round: u64,
    /// Stable input ids, e.g. `q#121`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Subscription changes, e.g. `s1@16`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
    /// Hook point that started the round (`on_init`, `on_wakeup`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    /// `input_seq` of the receipt that opened the round.
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
    pub rounds: Vec<RoundInputs>,
    /// Receipt batches of this run already applied to state (§8.3).
    #[serde(default)]
    pub applied_input_seq: u64,
    /// What of this run is already in the worklog. Function call runs: the
    /// number of messages after the prefix; behavior runs: steps with
    /// `step_index < flushed_step`.
    #[serde(default)]
    pub flushed_step: u64,
    /// Behavior runs: receipts (injected messages) with `input_seq ≤` this
    /// are in the worklog.
    #[serde(default)]
    pub flushed_input_seq: u64,
    /// Process entry the run belongs to (behavior process ↔ run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_entry: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProcessMode {
    Fork,
    Independent,
}

/// Suspended behavior process (its run is kept, §4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessFrame {
    pub entry: String,
    pub mode: ProcessMode,
    pub run_id: String,
    #[serde(default)]
    pub rounds: Vec<RoundInputs>,
    #[serde(default)]
    pub flushed_step: u64,
    #[serde(default)]
    pub flushed_input_seq: u64,
    #[serde(default)]
    pub applied_input_seq: u64,
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
    /// msg-center records consumed but not yet marked `Read` (deferred).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reading: Vec<String>,
}

impl SourceProgress {
    pub fn is_consumed(&self, index: u64) -> bool {
        index <= self.acked_index || self.consumed_above.contains(&index)
    }

    /// Mark `index` consumed and fold the contiguous prefix into
    /// `acked_index`. Never moves backwards.
    pub fn mark(&mut self, index: u64) {
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
    #[serde(default)]
    pub round: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_entry: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_stack: Vec<ProcessFrame>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_task_calls: Vec<Value>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub subscription_cursors: BTreeMap<String, Value>,
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
    /// A round to start without new input (behavior switch hand-over).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_continuation: Option<String>,
    /// Result of a fork child process, handed to the resumed parent in the
    /// next round (`{behavior, result}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_result: Option<Value>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

impl SessionState {
    pub fn initial(worklog: WorklogBoundary, now_ms: u64) -> Self {
        Self {
            schema: SESSION_STATE_SCHEMA.to_string(),
            rev: 1,
            writer: None,
            run_state: RunState::Created,
            waiting_for: None,
            outcome: None,
            acceptance: Acceptance::NotApplicable,
            result: None,
            round: 0,
            current_behavior: None,
            process_entry: None,
            process_stack: Vec::new(),
            pending_task_calls: Vec::new(),
            bootstrap_done: false,
            topic: Topic::default(),
            live_run: None,
            last_run: None,
            worklog,
            inputs: BTreeMap::new(),
            recent_keys: RecentKeys::default(),
            subscription_cursors: BTreeMap::new(),
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

    pub fn is_finished(&self) -> bool {
        self.run_state == RunState::Finished
    }
}
