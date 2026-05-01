use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Running,
    WaitingHuman,
    Completed,
    Failed,
    Paused,
    Aborted,
    BudgetExhausted,
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NodeRunState {
    Pending,
    Ready,
    Running,
    Completed,
    Failed,
    Retrying,
    WaitingHuman,
    Skipped,
    Aborted,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingThunk {
    pub node_id: String,
    pub thunk_obj_id: String,
    pub attempt: u32,
    #[serde(default)]
    pub shard_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapState {
    pub for_each_id: String,
    pub body_step_id: String,
    pub items: Vec<Value>,
    pub shard_states: Vec<NodeRunState>,
    pub shard_outputs: Vec<Value>,
    pub shard_attempts: Vec<u32>,
    pub max_concurrency: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParState {
    pub node_id: String,
    pub branches: Vec<String>,
    pub join: ParJoin,
    pub completed_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParJoin {
    All,
    Any,
    NOfM(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub plan_version: u32,
    pub status: RunStatus,
    pub node_states: BTreeMap<String, NodeRunState>,
    #[serde(default)]
    pub node_outputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub node_attempts: BTreeMap<String, u32>,
    #[serde(default)]
    pub branch_iterations: BTreeMap<String, u32>,
    #[serde(default)]
    pub pending_thunks: BTreeMap<String, PendingThunk>,
    #[serde(default)]
    pub activated_nodes: BTreeSet<String>,
    #[serde(default)]
    pub metrics: BTreeMap<String, Value>,
    #[serde(default)]
    pub human_waiting_nodes: BTreeSet<String>,
    #[serde(default)]
    pub map_states: BTreeMap<String, MapState>,
    #[serde(default)]
    pub par_states: BTreeMap<String, ParState>,
    #[serde(default)]
    pub seq: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl WorkflowRun {
    pub fn progress_percent(&self) -> f32 {
        if self.node_states.is_empty() {
            return 0.0;
        }
        let completed = self
            .node_states
            .values()
            .filter(|state| matches!(state, NodeRunState::Completed | NodeRunState::Skipped))
            .count() as f32;
        let total = self.node_states.len() as f32;
        (completed / total) * 100.0
    }

    pub fn node_state_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for state in self.node_states.values() {
            *counts.entry(format!("{:?}", state)).or_insert(0) += 1;
        }
        counts
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub ts: String,
    pub run_id: String,
    pub plan_version: u32,
    pub seq: u64,
    pub actor: String,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub attempt: Option<u32>,
    #[serde(default)]
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanAction {
    pub node_id: String,
    pub action: HumanActionKind,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default = "default_human_actor")]
    pub actor: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanActionKind {
    Approve,
    Modify,
    Reject,
    Retry,
    Skip,
    Abort,
    Rollback,
}

impl HumanActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Modify => "modify",
            Self::Reject => "reject",
            Self::Retry => "retry",
            Self::Skip => "skip",
            Self::Abort => "abort",
            Self::Rollback => "rollback",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanWait {
    pub node_id: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub subject: Option<Value>,
}

fn default_human_actor() -> String {
    "human".to_string()
}
