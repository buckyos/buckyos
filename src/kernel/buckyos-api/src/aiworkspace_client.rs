// aiworkspace (AI Workspace artifact management service) constants, settings
// schema and a thin kRPC client.
//
// Business results always travel in `result` (`{ ok, ... }` or the three-state
// commit result); the kRPC error channel only carries protocol-level failures.
// See doc/workspace and src/frame/aiworkspace/README.md.

use ::kRPC::kRPC;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const AIWORKSPACE_UNIQUE_ID: &str = "aiworkspace";
pub const AIWORKSPACE_SERVICE_NAME: &str = "aiworkspace";
pub const AIWORKSPACE_SERVICE_PORT: u16 = 4120;
pub const AIWORKSPACE_HTTP_PATH: &str = "/kapi/aiworkspace";

/// `services/aiworkspace/settings`. Seeded by the scheduler boot builder with
/// defaults (insert_json_if_absent, so user edits survive re-boot).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiWorkspaceSettings {
    /// Upper bound of one Commit request body, in bytes.
    pub max_commit_bytes: u64,
    /// Upper bound of operations in one Commit.
    pub max_ops: u64,
    /// Upper bound of `doc.wait_changes` long polling, in milliseconds.
    pub max_wait_ms: u64,
    /// Upper bound of one uploaded asset, in bytes (single chunk in this phase).
    pub max_asset_bytes: u64,
    /// Wish runs (xllm executor, program runner).
    pub wish: AiWorkspaceWishSettings,
}

/// Wish execution settings (doc/workspace 许愿格详细设计 §13.3). Models are AICC selectors (logical
/// names such as `llm.code`, or exact `model@provider`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiWorkspaceWishSettings {
    pub analyze_model: String,
    pub execute_model: String,
    /// Model of `aiws.llm.map` (per-item judgements inside programs).
    pub map_model: String,
    /// Deno used to run wish programs; empty = `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno`, then `PATH`.
    pub deno: String,
    pub analyze_tool_iterations: u32,
    pub execute_tool_iterations: u32,
    /// Output limit of one model turn (thinking included). Some providers (Claude) require one; AICC
    /// routes only to models whose output limit reaches it.
    pub max_output_tokens: u32,
    /// Wall clock of one stage, seconds.
    pub stage_timeout_secs: u64,
    pub program_timeout_secs: u64,
    pub program_memory_mb: u32,
    pub llm_map_max_items: u32,
}

impl Default for AiWorkspaceWishSettings {
    fn default() -> Self {
        AiWorkspaceWishSettings {
            analyze_model: "llm.plan".into(),
            execute_model: "llm.code".into(),
            map_model: "llm.chat".into(),
            deno: String::new(),
            analyze_tool_iterations: 30,
            execute_tool_iterations: 60,
            max_output_tokens: 32_000,
            stage_timeout_secs: 1800,
            program_timeout_secs: 120,
            program_memory_mb: 1024,
            llm_map_max_items: 2000,
        }
    }
}

impl Default for AiWorkspaceSettings {
    fn default() -> Self {
        AiWorkspaceSettings {
            max_commit_bytes: 8 * 1024 * 1024,
            max_ops: 10_000,
            max_wait_ms: 30_000,
            max_asset_bytes: 32 * 1024 * 1024,
            wish: AiWorkspaceWishSettings::default(),
        }
    }
}

/// kRPC client for callers without a UI (agents, other services, tests).
pub struct AiWorkspaceClient {
    client: kRPC,
}

impl AiWorkspaceClient {
    pub fn new(client: kRPC) -> Self {
        Self { client }
    }

    /// Call `method` and return the `result` object as is. Callers branch on
    /// `ok` / `status` / `error.code`, never on error text.
    pub async fn call(&self, method: &str, params: Value) -> std::result::Result<Value, ::kRPC::RPCErrors> {
        self.client.call(method, params).await
    }
}
