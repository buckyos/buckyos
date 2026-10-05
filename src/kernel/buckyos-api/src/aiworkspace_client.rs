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
}

impl Default for AiWorkspaceSettings {
    fn default() -> Self {
        AiWorkspaceSettings {
            max_commit_bytes: 8 * 1024 * 1024,
            max_ops: 10_000,
            max_wait_ms: 30_000,
            max_asset_bytes: 32 * 1024 * 1024,
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
