//! Tool call adaptation — the effect layer (§8.5).
//!
//! Every call: lease admission → host input gate clear → in-flight record
//! (fsync, non read-only calls) → tool runs. The in-flight marker is **not**
//! cleared here, not even on error or cancellation: only a checkpoint whose
//! snapshot contains the result (or an explicit `Unresolved`) clears it. A
//! `shell` command's lifecycle follows standard process semantics (long-tool
//! TODO §3.2): nothing is tracked beyond the in-flight record.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_tool::exec_tracking::InflightAction;
use agent_tool::runtime::Sandbox;
use agent_tool::TOOL_SHELL;
use async_trait::async_trait;
use buckyos_api::AiToolCall;
use llm_context::deps::{ToolCallCtx, ToolDispatchError, ToolManager, ToolSpecLite};
use llm_context::observation::Observation;

use crate::lock::Lease;
use crate::protocol::Touching;
use crate::session::runs::RunHandle;

use super::flush::canonical_args;


/// `read_only | idempotent | side_effect | unknown` of a tool name.
pub fn classify_effect(tool: &str) -> &'static str {
    match tool {
        "read_file" | "glob" | "grep" | "list_dir" | "read" => "read_only",
        "write_file" | "edit_file" => "side_effect",
        t if t == TOOL_SHELL => "unknown",
        _ => "unknown",
    }
}

/// Session-aware tool manager wrapping xllm's tool set.
pub struct SessionToolManager {
    inner: Arc<dyn Sandbox>,
    run: RunHandle,
    lease: Arc<Lease>,
    workdir: PathBuf,
    /// Write targets inferred from tool calls (merged into activity at the
    /// next observation boundary).
    touched: Arc<Mutex<Vec<Touching>>>,
}

impl SessionToolManager {
    pub fn new(
        inner: Arc<dyn Sandbox>,
        run: RunHandle,
        lease: Arc<Lease>,
        workdir: PathBuf,
        touched: Arc<Mutex<Vec<Touching>>>,
    ) -> Self {
        Self {
            inner,
            run,
            lease,
            workdir,
            touched,
        }
    }

    fn infer_touching(&self, call: &AiToolCall) {
        if classify_effect(&call.name) != "side_effect" {
            return;
        }
        let Some(p) = call.args.get("path").and_then(|v| v.as_str()) else {
            return;
        };
        let path = Path::new(p);
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workdir.join(path)
        };
        let target = match abs.strip_prefix(&self.workdir) {
            Ok(rel) => format!("ws:{}", rel.display()),
            Err(_) => abs.display().to_string(),
        };
        let mut t = self.touched.lock().expect("touched lock");
        if !t.iter().any(|x| x.target == target) {
            t.push(Touching {
                kind: "path".to_string(),
                target,
                mode: "write".to_string(),
                since_ms: crate::now_ms(),
            });
        }
    }
}

#[async_trait]
impl ToolManager for SessionToolManager {
    async fn call_tool(
        &self,
        call: AiToolCall,
        ctx: ToolCallCtx,
    ) -> Result<Observation, ToolDispatchError> {
        if let Err(e) = self.lease.check() {
            return Err(ToolDispatchError::not_started(e.to_string()));
        }
        if let Err(e) = self.run.require_execution_admitted() {
            return Err(ToolDispatchError::not_started(e.to_string()));
        }
        let effect = classify_effect(&call.name);
        if effect != "read_only" {
            let action = InflightAction {
                call_id: call.call_id.clone(),
                tool: call.name.clone(),
                args: canonical_args(&call.args),
                effect: effect.to_string(),
                idempotency_key: None,
                step_index: None,
                started_at_ms: crate::now_ms(),
            };
            if let Err(e) = self.run.register_inflight(action) {
                // Nothing started yet: safe to report as not started.
                return Err(ToolDispatchError::not_started(format!(
                    "cannot persist the in-flight record: {e}"
                )));
            }
            self.infer_touching(&call);
        }
        self.inner.call_tool(call, ctx).await
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        self.inner.list_tool_specs()
    }

    fn has_tool(&self, name: &str) -> bool {
        self.inner.has_tool(name)
    }
}
