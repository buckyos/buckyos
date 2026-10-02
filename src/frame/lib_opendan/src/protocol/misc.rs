//! Small per-session files: lock holder info, binding, statistics (§4.1, §5).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Holder information written into a lock file (`lease.json`, run `.lock`,
/// `.locks/*.lease`). Rewritten in place by the holder; never replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LockInfo {
    /// `session:<sid> | run:<run_id> | self_improve | artifact:<aid>`.
    pub resource: String,
    /// +1 on every successful acquisition.
    pub epoch: u64,
    pub holder: HolderInfo,
    pub acquired_at_ms: u64,
    #[serde(default)]
    pub released_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HolderInfo {
    pub runner_id: String,
    pub principal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
}

/// `binding.json` — written once on the first drive (§7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Binding {
    pub schema: String,
    pub target: serde_json::Value,
    pub runtime_id: String,
    /// `native | tmux`.
    pub kind: String,
    /// Absolute workdir for exec (the only place absolute paths are allowed
    /// besides `runs/`).
    pub workdir: String,
    pub bound_at_ms: u64,
    pub bound_by: String,
}

impl Binding {
    /// Identity comparison: `bound_at_ms` / `bound_by` do not matter.
    pub fn same_binding(&self, other: &Binding) -> bool {
        self.schema == other.schema
            && self.runtime_id == other.runtime_id
            && self.kind == other.kind
            && self.target == other.target
            && self.workdir == other.workdir
    }
}

/// `static.json` — statistics (tokens, Rounds, Turns, runs, time, cost).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionStatic {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    /// Rounds: inference attempts the runner made through the run's
    /// `LlmClient::infer` (successful, failed and interrupted), added after
    /// every outcome. History summarization is not a Round. Inferences of an
    /// executor that took the run over (xllm) are only in that run's
    /// `run.json` `usage.llm_requests`.
    #[serde(default)]
    pub rounds: u64,
    /// Rounds that returned an error.
    #[serde(default)]
    pub rounds_failed: u64,
    /// Rounds abandoned in flight (interrupt / provider cancellation).
    #[serde(default)]
    pub rounds_interrupted: u64,
    /// Logical Turns closed as completed (`SessionState.turns_completed`).
    #[serde(default)]
    pub turns: u64,
    /// Runs (`LLMContext` executions with their own run directory) ended.
    #[serde(default)]
    pub runs: u64,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default)]
    pub busy_ms: u64,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub updated_at_ms: u64,
}
