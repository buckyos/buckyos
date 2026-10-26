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
    BehaviorEntry, BehaviorToolConfig, CallTrigger, ChildCall, Touching, BEHAVIOR_TOOL_PREFIX,
    TOOL_CALL_BEHAVIOR, TOOL_REPORT,
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
        super::consolidation::TOOL_BEGIN => "read_only",
        super::consolidation::TOOL_COMMIT => "idempotent",
        "write_file" | "edit_file" => "side_effect",
        t if t == TOOL_SHELL => "unknown",
        _ => "unknown",
    }
}

/// run.json `host.extra` key: every native call of the run states its
/// intent (`[capabilities] call_intent` of the run's behavior, frozen with
/// the run so a resumed or rebuilt run keeps the same contract).
pub(super) const EXTRA_CALL_INTENT: &str = "call_intent";

/// The argument a call states its intent in (host metadata: kept with the
/// call's record, never passed to the tool).
pub const INTENT_ARG: &str = "intent";

const INTENT_DESCRIPTION: &str = "Why you make this call now and what you expect it to give you, in one short sentence; name the Todo and the accepted evidence it rests on when there are. The purpose of the action, not a transcript of your reasoning.";

/// Whether the run's record carries the host flag `key`.
pub(super) fn run_flag(run: &RunHandle, key: &str) -> Option<Value> {
    run.record()
        .host
        .as_ref()
        .and_then(|h| h.extra.get(key))
        .filter(|v| !v.is_null())
        .cloned()
}

/// `spec` with the required `intent` argument.
pub fn with_intent(mut spec: ToolSpecLite) -> ToolSpecLite {
    let schema = &mut spec.args_schema;
    if !schema.is_object() {
        *schema = json!({ "type": "object" });
    }
    if !schema.get("properties").is_some_and(Value::is_object) {
        schema["properties"] = json!({});
    }
    schema["properties"][INTENT_ARG] =
        json!({ "type": "string", "minLength": 1, "description": INTENT_DESCRIPTION });
    match schema.get_mut("required").and_then(Value::as_array_mut) {
        Some(r) if r.iter().any(|v| v == INTENT_ARG) => {}
        Some(r) => r.insert(0, json!(INTENT_ARG)),
        None => schema["required"] = json!([INTENT_ARG]),
    }
    spec
}

/// The call as its tool runs it: `intent` checked and taken out (the
/// original call, with its intent, stays in the run's record). `Err`: the
/// correction the call gets instead of running.
pub fn without_intent(call: &AiToolCall) -> Result<AiToolCall, String> {
    let missing = |what: &str| {
        format!(
            "{what}: this call was not executed. Every function call of this context carries `intent`, one short non-empty sentence saying why you make the call now and what you expect it to give you. Call it again with `intent`."
        )
    };
    match call.args.get(INTENT_ARG) {
        Some(Value::String(s)) if !s.trim().is_empty() => {
            let mut exec = call.clone();
            exec.args.remove(INTENT_ARG);
            Ok(exec)
        }
        Some(Value::String(_)) => Err(missing("`intent` is blank")),
        Some(_) => Err(missing("`intent` must be a string")),
        None => Err(missing("`intent` is missing")),
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
    /// Every call states its intent ([`EXTRA_CALL_INTENT`]).
    call_intent: bool,
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
        let call_intent = run_flag(&run, EXTRA_CALL_INTENT) == Some(Value::Bool(true));
        Self {
            shared,
            inner,
            run,
            lease,
            workdir,
            touched,
            current_tool,
            seen_tasks,
            call_intent,
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
        if let Err(e) = super::drive::verify_workspace_admission(&self.shared).await {
            return Err(ToolDispatchError::not_started(e.to_string()));
        }
        if let Err(e) = self.run.require_execution_admitted() {
            return Err(ToolDispatchError::not_started(e.to_string()));
        }
        // `call` stays as the model made it (the in-flight record compares
        // with the snapshot's call); `exec` is what the tool runs.
        let exec = if self.call_intent {
            match without_intent(&call) {
                Ok(exec) => exec,
                Err(message) => {
                    return Ok(Observation::Error {
                        call_id: call.call_id.clone(),
                        message,
                        tool_result: None,
                    })
                }
            }
        } else {
            call.clone()
        };
        *self.current_tool.lock().expect("current tool") = Some(call.name.clone());
        if call.name == TOOL_REPORT && self.inner.has_tool(TOOL_REPORT) {
            return super::reports::call(&self.shared, &self.run, &exec)
                .await
                .map_err(|e| ToolDispatchError::not_started(e.to_string()));
        }
        // The consolidation tools exist only for the Goal Session whose Goal
        // has the capability; anywhere else they are unknown tools.
        if super::consolidation::is_tool(&call.name)
            && super::consolidation::caps(&self.shared).consolidation
            && self.inner.has_tool(&call.name)
        {
            return super::consolidation::call(&self.shared, &self.run, &exec)
                .await
                .map_err(|e| ToolDispatchError::not_started(e.to_string()));
        }
        // Running a sub context changes nothing by itself: what it does is
        // tracked by its own run.
        let effect = if self
            .shared
            .behavior_functions
            .lock()
            .expect("behavior functions")
            .contains(&call.name)
        {
            "read_only"
        } else {
            classify_effect(&call.name)
        };
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
            self.infer_touching(&exec);
        }
        // A tool that answers `Pending { task_id }` suspends the run: the
        // session waits for the task outside the context (the run's
        // resolver answers for it) and fills the result on resume.
        let call_id = call.call_id.clone();
        let result = self.inner.call_tool(exec, ctx).await;
        // A question to the user is the session's own task.
        if let Ok(Observation::Pending {
            task_id,
            tool_result,
            ..
        }) = &result
        {
            if super::ask::is_user_input(task_id) {
                let detail = tool_result.as_ref().and_then(|t| t.detail.as_ref());
                if let Some(refused) =
                    super::ask::on_pending(&self.shared, &self.run, &call_id, task_id, detail).await
                {
                    return Ok(refused);
                }
                return result;
            }
        }
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
        let specs = self.inner.list_tool_specs();
        if !self.call_intent {
            return specs;
        }
        specs.into_iter().map(with_intent).collect()
    }

    fn has_tool(&self, name: &str) -> bool {
        self.inner.has_tool(name)
    }
}

/// Task id of a sub context call: the caller's run is suspended on it
/// (`PendingTool`) until the sub context returns. Opaque to the waist; only
/// this runner resolves it.
pub const SUB_CONTEXT_TASK_PREFIX: &str = "subctx:";

/// A tool that runs a sub context (`create_sub_context` / `fork` target)
/// and returns its result as this call's tool result: the generic
/// `call_behavior({behavior, task})`, or the named function a behavior
/// declares with `[tool]`. The tool only validates and suspends; the
/// session runs the child (§4.4).
pub struct BehaviorTool {
    kind: BehaviorToolKind,
    /// Sub contexts already in progress above the run this tool belongs to.
    depth: usize,
    /// `session.policy.max_process_depth`.
    max_depth: usize,
}

enum BehaviorToolKind {
    /// `call_behavior`: any sub context behavior, named in the arguments.
    Any(BTreeMap<String, BehaviorEntry>),
    /// One behavior's `[tool]`.
    Named {
        behavior: String,
        tool: BehaviorToolConfig,
    },
}

impl BehaviorTool {
    /// `call_behavior` over the session's sub context behaviors.
    pub fn any(
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
            kind: BehaviorToolKind::Any(targets),
            depth,
            max_depth,
        })
    }

    /// The named functions the behaviors declare, keyed by the tool list
    /// entry that offers them (`behavior:<name>`).
    pub fn named(
        behaviors: &BTreeMap<String, BehaviorEntry>,
        depth: usize,
        max_depth: usize,
    ) -> Vec<(String, Self)> {
        behaviors
            .iter()
            .filter(|(_, e)| e.mode.is_sub_context())
            .filter_map(|(name, e)| {
                let tool = e.tool.clone()?;
                Some((
                    format!("{BEHAVIOR_TOOL_PREFIX}{name}"),
                    Self {
                        kind: BehaviorToolKind::Named {
                            behavior: name.clone(),
                            tool,
                        },
                        depth,
                        max_depth,
                    },
                ))
            })
            .collect()
    }
}

/// The behavior a named function runs (`[tool]` of a frozen behavior).
pub(super) fn behavior_of_function(
    behaviors: &BTreeMap<String, BehaviorEntry>,
    function: &str,
) -> Option<(String, BehaviorToolConfig)> {
    behaviors.iter().find_map(|(name, e)| {
        e.tool
            .as_ref()
            .filter(|t| e.mode.is_sub_context() && t.function_name(name) == function)
            .map(|t| (name.clone(), t.clone()))
    })
}

#[async_trait]
impl AgentTool for BehaviorTool {
    fn spec(&self) -> ToolSpec {
        match &self.kind {
            BehaviorToolKind::Any(targets) => {
                let list = targets
                    .iter()
                    .map(|(k, e)| format!("{k} ({})", e.mode.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ");
                ToolSpec {
                    name: TOOL_CALL_BEHAVIOR.into(),
                    description: format!(
                        "Run a sub task in a sub context and get its result as this call's result. State the goal and the expected output in `task`. Available behaviors: {list}."
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
            BehaviorToolKind::Named { behavior, tool } => ToolSpec {
                name: tool.function_name(behavior),
                description: tool.description.trim().to_string(),
                args_schema: tool.args_schema(),
                output_schema: json!({ "type": "object" }),
                usage: None,
            },
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
        let (behavior, task, tool_name) = match &self.kind {
            BehaviorToolKind::Any(targets) => {
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
                if !targets.contains_key(behavior) {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "behavior `{behavior}` cannot be called as a sub context (available: {})",
                        targets.keys().cloned().collect::<Vec<_>>().join(", ")
                    )));
                }
                (behavior.to_string(), task.to_string(), TOOL_CALL_BEHAVIOR.to_string())
            }
            BehaviorToolKind::Named { behavior, tool } => {
                let task = tool.render_task(&args);
                if task.is_empty() {
                    return Err(AgentToolError::InvalidArgs(
                        "the arguments give the sub context no task".into(),
                    ));
                }
                (behavior.clone(), task, tool.function_name(behavior))
            }
        };
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
            .with_tool(&tool_name)
            .with_status(AgentToolStatus::Pending);
        r.task_id = Some(format!("{SUB_CONTEXT_TASK_PREFIX}{call_id}"));
        r.summary = format!("sub context `{behavior}` started");
        Ok(r)
    }
}

/// The sub context call a `PendingTool` snapshot waits for, if that is what
/// it is suspended on (exactly one pending call, made through
/// `call_behavior` or a behavior's named function).
pub(super) fn pending_sub_call(
    snapshot: &LLMContextSnapshot,
    behaviors: &BTreeMap<String, BehaviorEntry>,
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
    let (behavior, task) = match behavior_of_function(behaviors, &p.call.name) {
        Some((behavior, tool)) => (behavior, Some(tool.render_task(&Value::Object(p.call.args.clone().into_iter().collect())))),
        None => (
            arg("behavior").unwrap_or_default(),
            arg("task").or_else(|| arg("body")),
        ),
    };
    let entry = mode_of(&behavior)?;
    Ok(Some(ChildCall {
        mode: entry.mode,
        behavior,
        trigger: CallTrigger::Tool {
            call_id: p.call.call_id.clone(),
            task_id: p.task_id.clone(),
        },
        task,
        todo: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(args: Value) -> AiToolCall {
        AiToolCall {
            name: "shell".into(),
            args: args.as_object().unwrap().clone().into_iter().collect(),
            call_id: "c1".into(),
        }
    }

    #[test]
    fn intent_is_required_in_the_schema_and_taken_out_of_the_call() {
        let spec = with_intent(ToolSpecLite {
            name: "report".into(),
            description: String::new(),
            args_schema: json!({"type":"object","additionalProperties":false,"properties":{"report":{"type":"string"}},"required":["report"]}),
        });
        assert_eq!(spec.args_schema["required"], json!(["intent", "report"]));
        assert_eq!(spec.args_schema["properties"]["intent"]["minLength"], 1);
        assert_eq!(spec.args_schema["additionalProperties"], false);
        let bare = with_intent(ToolSpecLite { name: "x".into(), description: String::new(), args_schema: Value::Null });
        assert_eq!(bare.args_schema["required"], json!(["intent"]));

        let original = call(json!({"intent": "list the files", "command": "ls"}));
        let exec = without_intent(&original).unwrap();
        assert!(!exec.args.contains_key(INTENT_ARG));
        assert_eq!(exec.args.get("command"), Some(&json!("ls")));
        assert_eq!(original.args.get(INTENT_ARG), Some(&json!("list the files")), "the original keeps it");
        for bad in [json!({"command": "ls"}), json!({"intent": " ", "command": "ls"}), json!({"intent": 3, "command": "ls"})] {
            assert!(without_intent(&call(bad)).unwrap_err().contains("not executed"));
        }
    }
}
