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
use agent_tool::runtime::{Sandbox, CURRENT_TOOL_CALL, CURRENT_TOOL_CTX};
use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, AgentToolStatus, CallingConventions,
    SessionRuntimeContext, ToolSpec, TOOL_SHELL,
};
use async_trait::async_trait;
use buckyos_api::AiToolCall;
use llm_context::deps::{ToolCallCtx, ToolDispatchError, ToolManager, ToolSpecLite};
use llm_context::observation::Observation;
use llm_context::state::LLMContextSnapshot;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::lock::Lease;
use crate::protocol::{
    BehaviorEntry, CallTrigger, ChildCall, Touching, TOOL_CALL_BEHAVIOR, TOOL_REPORT,
};
use crate::session::runs::RunHandle;

use super::flush::canonical_args;

/// `read_only | idempotent | side_effect | unknown` of a tool name.
pub fn classify_effect(tool: &str) -> &'static str {
    match tool {
        "read_file" | "glob" | "grep" | "list_dir" | "read" => "read_only",
        // The call itself changes nothing: what the sub context does is
        // tracked by its own run.
        TOOL_CALL_BEHAVIOR => "read_only",
        TOOL_REPORT => "idempotent",
        "write_file" | "edit_file" => "side_effect",
        t if t == TOOL_SHELL => "unknown",
        _ => "unknown",
    }
}

/// Session-aware tool manager wrapping xllm's tool set.
pub struct SessionToolManager {
    shared: Arc<super::shared::Shared>,
    inner: Arc<dyn Sandbox>,
    run: RunHandle,
    lease: Arc<Lease>,
    workdir: PathBuf,
    /// Write targets inferred from tool calls (merged into activity at the
    /// next checkpoint).
    touched: Arc<Mutex<Vec<Touching>>>,
    /// Tool called last (shown as the activity of the Turn's task).
    current_tool: Arc<Mutex<Option<String>>>,
    /// Task ids the results of this run referred to so far.
    seen_tasks: Mutex<std::collections::BTreeSet<String>>,
}

impl SessionToolManager {
    pub fn new(
        shared: Arc<super::shared::Shared>,
        inner: Arc<dyn Sandbox>,
        run: RunHandle,
        lease: Arc<Lease>,
        workdir: PathBuf,
        touched: Arc<Mutex<Vec<Touching>>>,
        current_tool: Arc<Mutex<Option<String>>>,
    ) -> Self {
        let seen_tasks = Mutex::new(run.noted_tasks().into_iter().collect());
        Self {
            shared,
            inner,
            run,
            lease,
            workdir,
            touched,
            current_tool,
            seen_tasks,
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
        *self.current_tool.lock().expect("current tool") = Some(call.name.clone());
        if call.name == TOOL_REPORT && self.inner.has_tool(TOOL_REPORT) {
            return super::reports::call(&self.shared, &self.run, &call)
                .await
                .map_err(|e| ToolDispatchError::not_started(e.to_string()));
        }
        let effect = classify_effect(&call.name);
        if effect != "read_only" {
            let action = InflightAction {
                call_id: call.call_id.clone(),
                tool: call.name.clone(),
                args: canonical_args(&call.args),
                effect: effect.to_string(),
                // Stable identity of the dispatch (session / run / call): a
                // tool that creates a task in an external service uses it as
                // the idempotency key, so a crash between creating and
                // recording the task finds the same task again.
                idempotency_key: Some(crate::ids::h(&[self.run.run_id(), call.call_id.as_str()])),
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
        // A tool that answers `Pending { task_id }` suspends the run: the
        // session waits for the task outside the context (the run's
        // resolver answers for it) and fills the result on resume.
        let result = self.inner.call_tool(call, ctx).await;
        if let Ok(
            Observation::Success { tool_result, .. } | Observation::Error { tool_result, .. },
        ) = &result
        {
            if let Some((task_id, t)) = tool_result
                .as_ref()
                .and_then(|t| t.task_id.as_deref().map(|id| (id, t)))
            {
                // A result that introduces a task (and is not the result of
                // a finished command) or says it is "still running" means
                // the task continues after the call; any other result about
                // a known task is its end as the LLM saw it.
                let first = self
                    .seen_tasks
                    .lock()
                    .expect("seen tasks")
                    .insert(task_id.to_string());
                let ok = matches!(&result, Ok(Observation::Success { .. }));
                let running = t.summary.contains("still running")
                    || (first && ok && t.return_code.is_none());
                if let Err(e) = self.run.note_task(task_id, running) {
                    log::warn!("cannot note task {task_id} of run {}: {e}", self.run.run_id());
                }
            }
        }
        result
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        self.inner.list_tool_specs()
    }

    fn has_tool(&self, name: &str) -> bool {
        self.inner.has_tool(name)
    }
}

/// Task id of a sub context call: the caller's run is suspended on it
/// (`PendingTool`) until the sub context returns. Opaque to the waist; only
/// this runner resolves it.
pub const SUB_CONTEXT_TASK_PREFIX: &str = "subctx:";

/// `call_behavior({behavior, task})`: run a sub context (`create_sub_context`
/// / `fork` target) and return its result as this call's tool result. The
/// tool only validates and suspends; the session runs the child (§4.4).
pub struct CallBehaviorTool {
    /// Behaviors that can be called (sub context modes only).
    targets: BTreeMap<String, BehaviorEntry>,
    /// Sub contexts already in progress above the run this tool belongs to.
    depth: usize,
    /// `session.policy.max_process_depth`.
    max_depth: usize,
}

impl CallBehaviorTool {
    pub fn new(
        behaviors: &BTreeMap<String, BehaviorEntry>,
        depth: usize,
        max_depth: usize,
    ) -> Option<Self> {
        let targets: BTreeMap<String, BehaviorEntry> = behaviors
            .iter()
            .filter(|(_, e)| e.mode.is_sub_context())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        (!targets.is_empty()).then_some(Self {
            targets,
            depth,
            max_depth,
        })
    }
}

#[async_trait]
impl AgentTool for CallBehaviorTool {
    fn spec(&self) -> ToolSpec {
        let list = self
            .targets
            .iter()
            .map(|(k, e)| format!("{k} ({})", e.mode.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        ToolSpec {
            name: TOOL_CALL_BEHAVIOR.into(),
            description: format!(
                "Run a sub task in a sub context and get its result as this call's result. The sub context cannot ask the user; state the goal and the expected output in `task`. Available behaviors: {list}."
            ),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "behavior": { "type": "string", "description": "behavior to run" },
                    "task": { "type": "string", "description": "what the sub context must do and return" }
                },
                "required": ["behavior", "task"]
            }),
            output_schema: json!({ "type": "object" }),
            usage: Some(format!("{TOOL_CALL_BEHAVIOR} behavior=<name> task=<text>")),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Value,
    ) -> std::result::Result<AgentToolResult, AgentToolError> {
        let behavior = args
            .get("behavior")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .ok_or_else(|| AgentToolError::InvalidArgs("`behavior` is required".into()))?;
        let task = args
            .get("task")
            .or_else(|| args.get("body"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| AgentToolError::InvalidArgs("`task` is required".into()))?;
        if !self.targets.contains_key(behavior) {
            return Err(AgentToolError::InvalidArgs(format!(
                "behavior `{behavior}` cannot be called as a sub context (available: {})",
                self.targets.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        if self.depth >= self.max_depth {
            return Err(AgentToolError::ExecFailed(format!(
                "sub contexts are nested {} deep already; do the work in this context",
                self.max_depth
            )));
        }
        let deferred = CURRENT_TOOL_CTX
            .try_with(|c| c.allow_deferred)
            .unwrap_or(false);
        let call_id = CURRENT_TOOL_CALL.try_with(|c| c.clone()).unwrap_or_default();
        if !deferred || call_id.is_empty() {
            return Err(AgentToolError::ExecFailed(
                "this executor cannot suspend the run for a sub context; the session's own runner must drive it".into(),
            ));
        }
        let mut r = AgentToolResult::from_details(json!({ "behavior": behavior, "task": task }))
            .with_tool(TOOL_CALL_BEHAVIOR)
            .with_status(AgentToolStatus::Pending);
        r.task_id = Some(format!("{SUB_CONTEXT_TASK_PREFIX}{call_id}"));
        r.summary = format!("sub context `{behavior}` started");
        Ok(r)
    }
}

/// The sub context call a `PendingTool` snapshot waits for, if that is what
/// it is suspended on (exactly one pending call, made through
/// `call_behavior`).
pub(super) fn pending_sub_call(
    snapshot: &LLMContextSnapshot,
    mode_of: impl Fn(&str) -> crate::error::Result<BehaviorEntry>,
) -> crate::error::Result<Option<ChildCall>> {
    let pending = snapshot.state.pending_calls();
    let [p] = pending else {
        return Ok(None);
    };
    if !p.task_id.starts_with(SUB_CONTEXT_TASK_PREFIX) {
        return Ok(None);
    }
    let arg = |k: &str| {
        p.call
            .args
            .get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
    };
    let behavior = arg("behavior").unwrap_or_default();
    let entry = mode_of(&behavior)?;
    Ok(Some(ChildCall {
        mode: entry.mode,
        behavior,
        trigger: CallTrigger::Tool {
            call_id: p.call.call_id.clone(),
            task_id: p.task_id.clone(),
        },
        task: arg("task").or_else(|| arg("body")),
    }))
}
