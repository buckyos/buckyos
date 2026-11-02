//! Tool call adaptation — the effect layer (§8.5).
//!
//! Every call: lease admission → host input gate clear → tool runs. What was
//! dispatched is known from the snapshot the waist checkpoints before every
//! call; a call a crashed executor may have started is answered by the
//! runtime's tool manager by its call id (freeze TODO F12). A `shell`
//! command's lifecycle follows standard process semantics (long-tool TODO
//! §3.2).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_tool::runtime::{Sandbox, CURRENT_TOOL_CALL};
use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, AgentToolStatus, CallingConventions,
    SessionRuntimeContext, ToolResultView, ToolSpec, TOOL_SHELL,
};
use async_trait::async_trait;
use buckyos_api::AiToolCall;
use llm_context::deps::{ToolCallCtx, ToolDispatchError, ToolManager, ToolSpecLite};
use llm_context::observation::Observation;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::lock::Lease;
use crate::protocol::{
    BehaviorEntry, BehaviorToolConfig, CallTrigger, ChildCall, Touching, BEHAVIOR_TOOL_PREFIX,
    TOOL_CALL_BEHAVIOR, TOOL_REPORT,
};
use crate::session::runs::RunHandle;


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

/// Whether the run's record carries the host flag `key`.
pub(super) fn run_flag(run: &RunHandle, key: &str) -> Option<Value> {
    run.record()
        .host
        .as_ref()
        .and_then(|h| h.extra.get(key))
        .filter(|v| !v.is_null())
        .cloned()
}

/// What a call that handed its work over as a task was: kept with the run
/// for the input that brings the task's end back.
fn call_brief(call: &AiToolCall) -> Value {
    let what = ["command", "path", "uri", "task"]
        .iter()
        .find_map(|k| call.args.get(*k).and_then(Value::as_str))
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|s| s.chars().take(200).collect::<String>());
    json!({
        "tool": call.name,
        "what": what,
        "intent": agent_tool::intent::intent_of(call),
    })
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

    /// Record the sub context `call` asks for on the run: the session hands
    /// over to it when the run's segment ends. `Some`: the call's result
    /// instead (a second call in one run, a call that cannot be recorded).
    async fn register_sub_call(
        &self,
        call: &AiToolCall,
        _host: &Option<Value>,
    ) -> Option<Observation> {
        let refuse = |message: String| {
            Some(Observation::Error {
                call_id: call.call_id.clone(),
                message,
                host: None,
            })
        };
        if let Some(earlier) = self.run.sub_call() {
            return refuse(format!(
                "sub context `{}` is already called in this run: stop acting (reply without actions or tool calls) to let it run; its result comes back to you as input",
                earlier.behavior
            ));
        }
        let cfg = self.shared.session.lock().await.config.clone();
        let child = match cfg
            .behaviors()
            .map_err(crate::error::OpenDanError::InvalidArgument)
            .and_then(|behaviors| {
                sub_call_of(call, &behaviors, |b| {
                    self.shared.deps.assembler.behavior_entry(&cfg, b)
                })
            }) {
            Ok(c) => c,
            Err(e) => return refuse(format!("the sub context cannot be called: {e}")),
        };
        if let Err(e) = self.run.set_sub_call(Some(&child)) {
            return refuse(format!("the sub context call cannot be recorded: {e}"));
        }
        None
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
        // `call` stays as the model made it (the snapshot keeps it); `exec`
        // is what the tool runs. A context whose every call states its
        // intent checks it here; otherwise the runtime's tools take an
        // optional one themselves.
        let exec = if self.call_intent {
            match agent_tool::take_intent(&call, agent_tool::IntentPolicy::Required) {
                Ok((exec, _)) => exec,
                Err(message) => {
                    return Ok(Observation::Error {
                        call_id: call.call_id.clone(),
                        message,
                        host: None,
                    })
                }
            }
        } else {
            call.clone()
        };
        *self.current_tool.lock().expect("current tool") = Some(call.name.clone());
        // Shown as the progress of the Turn.
        *self.shared.current_intent.lock().expect("current intent") =
            agent_tool::intent::intent_of(&call);
        super::turn_task::live_line_changed(&self.shared).await;
        super::turn_task::sync_turn_task(&self.shared).await;
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
        let sub_context = self
            .shared
            .behavior_functions
            .lock()
            .expect("behavior functions")
            .contains(&call.name);
        if !sub_context && classify_effect(&call.name) == "side_effect" {
            self.infer_touching(&exec);
        }
        let result = self.inner.call_tool(exec, ctx).await;
        // A sub context call: the session hands over to it when this run's
        // segment ends; its result comes back as input (freeze TODO §3.2.1).
        if sub_context {
            if let Ok(Observation::Success { host, .. }) = &result {
                if let Some(refused) = self.register_sub_call(&call, host).await {
                    return Ok(refused);
                }
            }
            return result;
        }
        if let Ok(Observation::Success { host, .. } | Observation::Error { host, .. }) = &result {
            if let Some(t) = ToolResultView::from_host(host) {
                let Some(task_id) = t.task_id.as_deref() else {
                    return result;
                };
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
                if let Err(e) =
                    self.run
                        .note_task(task_id, running, &call.call_id, Some(call_brief(&call)))
                {
                    log::warn!("cannot note task {task_id} of run {}: {e}", self.run.run_id());
                }
                // A sub session whose end was this call's result is not
                // reported again as an event.
                if !t.summary.contains("still running") && t.pending_reason.is_none() {
                    if let Err(e) = super::children::mark_heard(&self.shared, task_id).await {
                        log::warn!("cannot mark sub session {task_id} as heard: {e}");
                    }
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
        specs
            .into_iter()
            .map(|s| agent_tool::with_intent(s, agent_tool::IntentPolicy::Required))
            .collect()
    }

    fn has_tool(&self, name: &str) -> bool {
        self.inner.has_tool(name)
    }
}

/// Task id of a sub context call (`CallTrigger::Tool.task_id`): names the
/// call in the caller's process frame. Opaque to the waist.
pub const SUB_CONTEXT_TASK_PREFIX: &str = "subctx:";

/// A tool that runs a sub context (`create_sub_context` / `fork` target):
/// the generic `call_behavior({behavior, task})`, or the named function a
/// behavior declares with `[tool]`. The tool only validates and returns at
/// once; the session records the call on the run, hands over to it when the
/// run's segment ends, and the sub context's result comes back to the
/// caller as input — the loop never waits for it (§4.4, freeze TODO
/// §3.2.1).
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
                        "Run a sub task in a sub context. The call registers it; the sub context starts when you stop acting (reply without tool calls) and its result comes back to you as input (`<process_result call_id=…>`). State the goal and the expected output in `task`. Available behaviors: {list}."
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
        let call_id = CURRENT_TOOL_CALL.try_with(|c| c.clone()).unwrap_or_default();
        if call_id.is_empty() {
            return Err(AgentToolError::ExecFailed(
                "a sub context is run by the session's own runner; this executor cannot call it".into(),
            ));
        }
        Ok(
            AgentToolResult::from_details(json!({ "behavior": behavior, "task": task }))
                .with_tool(&tool_name)
                .with_status(AgentToolStatus::Success)
                .with_result(format!("sub context `{behavior}` registered"))
                .with_output(format!(
                    "Sub context `{behavior}` is registered. It starts when this context stops acting (reply without actions or tool calls); its result comes back to you as input (`<process_result>`)."
                )),
        )
    }
}

/// The sub context `call` (made through `call_behavior` or a behavior's
/// named function) asks for.
pub(super) fn sub_call_of(
    call: &AiToolCall,
    behaviors: &BTreeMap<String, BehaviorEntry>,
    mode_of: impl Fn(&str) -> crate::error::Result<BehaviorEntry>,
) -> crate::error::Result<ChildCall> {
    let arg = |k: &str| {
        call.args
            .get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
    };
    let (behavior, task) = match behavior_of_function(behaviors, &call.name) {
        Some((behavior, tool)) => (
            behavior,
            Some(tool.render_task(&Value::Object(call.args.clone().into_iter().collect()))),
        ),
        None => (
            arg("behavior").unwrap_or_default(),
            arg("task").or_else(|| arg("body")),
        ),
    };
    let entry = mode_of(&behavior)?;
    Ok(ChildCall {
        mode: entry.mode,
        behavior,
        trigger: CallTrigger::Tool {
            call_id: call.call_id.clone(),
            task_id: format!("{SUB_CONTEXT_TASK_PREFIX}{}", call.call_id),
        },
        task,
    })
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
        use agent_tool::{take_intent, with_intent, IntentPolicy, INTENT_ARG};
        let required = IntentPolicy::Required;
        let spec = with_intent(
            ToolSpecLite {
                name: "report".into(),
                description: String::new(),
                args_schema: json!({"type":"object","additionalProperties":false,"properties":{"report":{"type":"string"}},"required":["report"]}),
            },
            required,
        );
        assert_eq!(spec.args_schema["required"], json!(["intent", "report"]));
        assert_eq!(spec.args_schema["properties"]["intent"]["minLength"], 1);
        assert_eq!(spec.args_schema["additionalProperties"], false);
        let bare = with_intent(
            ToolSpecLite { name: "x".into(), description: String::new(), args_schema: Value::Null },
            required,
        );
        assert_eq!(bare.args_schema["required"], json!(["intent"]));

        let original = call(json!({"intent": "list the files", "command": "ls"}));
        let (exec, intent) = take_intent(&original, required).unwrap();
        assert_eq!(intent.as_deref(), Some("list the files"));
        assert!(!exec.args.contains_key(INTENT_ARG));
        assert_eq!(exec.args.get("command"), Some(&json!("ls")));
        assert_eq!(original.args.get(INTENT_ARG), Some(&json!("list the files")), "the original keeps it");
        for bad in [json!({"command": "ls"}), json!({"intent": " ", "command": "ls"}), json!({"intent": 3, "command": "ls"})] {
            assert!(take_intent(&call(bad), required).unwrap_err().contains("not executed"));
        }
    }

    #[test]
    fn a_task_call_is_briefed_with_what_it_ran_and_why() {
        let b = call_brief(&call(json!({"command": "cargo   build -p x", "intent": "build x"})));
        assert_eq!(b, json!({"tool": "shell", "what": "cargo build -p x", "intent": "build x"}));
        let b = call_brief(&call(json!({})));
        assert_eq!(b, json!({"tool": "shell", "what": null, "intent": null}));
    }
}
