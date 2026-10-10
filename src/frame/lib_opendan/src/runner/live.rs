//! The live run of a drive: creating a fresh `LLMContext` for a run (§4.4),
//! resuming the live run (§8.6), the mid-run context-limit rewrite (X7),
//! suspending a process into `process_stack`, committing an input batch
//! (§8.3) and removing unreferenced runs.

use std::path::PathBuf;
use std::sync::Arc;

use agent_tool::exec_tracking::{materialize_unresolved, HostRunInfo};
use agent_tool::llm_bash::RunBinding;
use agent_tool::xllm::{
    create_run_llm, hosted_waist_deps, rebuild_toolset, EffectiveConfig, LoopModel, RunRecord,
    RunStatus, XllmTask,
};
use buckyos_api::{AiContent, AiMessage, AiRole};
use llm_context::deps::{Injection, LLMContextDeps, LlmClient};
use llm_context::error::{LLMComputeError, ProviderFailure};
use llm_context::observation::Observation;
use llm_context::outcome::{LLMContextOutcome, ResumeFill};
use llm_context::request::ContextOwnerRef;
use llm_context::state::{LLMContextSnapshot, Suspension};
use llm_context::tasks::{task_state_observation, RunningTaskResolver, TaskState};
use llm_context::LLMContext;
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::runtime::SessionEnv;
use crate::session::runs::RunHandle;

use super::flush::{run_history_entries, FlushMarks};
use super::history::{build_history, compact_for_limit, LlmSummarizer, Summarizer};
use super::hook::{scope_touching, SessionCheckpointHook};
use super::inputs::confirm_inputs;
use super::receipts::{
    apply_receipt, host_meta_of, positions_of, snapshot_host_meta, with_host_meta,
};
use super::shared::{commit_and_report, counted, LiveCtx, Opened, Shared, WaitingRun};
use super::tools::{CallBehaviorTool, SessionToolManager, SUB_CONTEXT_TASK_PREFIX};

/// Mid-run compactions in a row before a context-limit run is paused.
const MAX_LIMIT_COMPACTIONS: u32 = 3;

/// Remove an unreferenced run: lock, then delete. Processes a run's
/// commands left behind are not the run's to stop (long-tool TODO §3.2).
pub(super) async fn remove_if_safe(sh: &Shared, run_id: &str) -> Result<bool> {
    let runs = sh.dir.runs();
    let Some(lock) = runs.try_lock(run_id)? else {
        return Ok(false); // someone executes it (xllm?)
    };
    if runs.record(run_id).is_err() && runs.dir().join(run_id).join("run.json").exists() {
        log::warn!("keeping unreferenced run {run_id}: its record is unreadable");
        return Ok(false);
    }
    runs.remove_locked(run_id, &lock)?;
    Ok(true)
}

/// Opening of the runtime protocol of a session's runs.
pub const SESSION_PROTOCOL_INTRO: &str = "You are running inside an Agent Session. Work on the session's objective with the inputs you are given; the session delivers your result and decides what follows it. Use only the material provided and the results you obtain in this session; distinguish verified facts from assumptions.";

fn default_llm_context() -> Value {
    json!({ "tools": { "enabled": true } })
}

/// xllm deps of a run at call depth `depth` (sub contexts in progress above
/// it): the session's runtime and, when the session declares sub context
/// behaviors, the `call_behavior` tool.
async fn xllm_deps_for(
    sh: &Arc<Shared>,
    env: &SessionEnv,
    depth: usize,
) -> Result<agent_tool::xllm::XllmDeps> {
    let mut x = sh.deps.xllm.clone();
    x.runtime = Some(sh.deps.runtime.clone());
    x.runtime_env = env.env.iter().cloned().collect();
    x.runtime_path_prefix = env.path_layers.clone();
    x.skip_workdir_lock = true;
    if x.host_protocol == agent_tool::xllm::HostProtocolFlavor::OneShot {
        // A session is not a one-shot task: whether the agent may wait for
        // the user is the session's rule (`session.policy.wait_user_msg`).
        x.host_protocol = agent_tool::xllm::HostProtocolFlavor::Session {
            intro: SESSION_PROTOCOL_INTRO.to_string(),
        };
    }
    let (behaviors, max_depth) = {
        let s = sh.session.lock().await;
        (
            s.config
                .behaviors()
                .map_err(OpenDanError::InvalidArgument)?,
            s.config.session.policy.max_process_depth as usize,
        )
    };
    // Suspended calls waiting for a sub session (`session:<sid>`) are
    // answered from the registry; other task ids go to the host's resolver.
    x.buckyos_tasks = Some(Arc::new(super::children::SessionTaskResolver::new(
        sh.deps.agent.clone(),
        sh.deps.xllm.buckyos_tasks.clone(),
    )));
    if let Some(tool) = CallBehaviorTool::new(&behaviors, depth, max_depth) {
        x.host_tools
            .insert(TOOL_CALL_BEHAVIOR.to_string(), Arc::new(tool));
    }
    x.host_tools
        .insert(TOOL_REPORT.into(), Arc::new(super::reports::ReportTool));
    if sh.session.lock().await.config.session.policy.completion == CompletionPolicy::ExplicitReport
    {
        if let agent_tool::xllm::HostProtocolFlavor::Session { intro } = &mut x.host_protocol {
            intro.push_str(" This session requires an explicit final report: call report with is_end=true (function call loop), or use <report end=\"true\"> (behavior loop). Plain assistant completion does not successfully finish this session.");
        }
    }
    Ok(x)
}

/// The configuration a behavior's own context runs with: the session's,
/// with the entry's application system prompt and `llm_context` keys.
fn context_config(cfg: &SessionConfig, entry: &BehaviorEntry) -> SessionConfig {
    let mut c = cfg.clone();
    if let Some(p) = &entry.prompt.system {
        c.prompt.system = Some(p.clone());
    }
    if let Some(over) = entry.llm_context.as_object() {
        if !c.prompt.llm_context.is_object() {
            c.prompt.llm_context = default_llm_context();
        }
        for (k, v) in over {
            c.prompt.llm_context[k.as_str()] = v.clone();
        }
    }
    c
}

fn checkpoint_deps(
    sh: &Arc<Shared>,
    run: &RunHandle,
    cfg: &EffectiveConfig,
    llm: Arc<dyn LlmClient>,
    tools: SessionToolManager,
    rounds: Arc<super::rounds::RoundCounter>,
    resolver: &Arc<dyn RunningTaskResolver>,
) -> llm_context::deps::LLMContextDeps {
    let hook = Arc::new(SessionCheckpointHook::new(sh.clone(), run.clone(), rounds));
    *sh.tasks.lock().expect("tasks") = Some(resolver.clone());
    hosted_waist_deps(cfg, llm, Arc::new(tools))
        .with_checkpoint_hook(hook)
        .with_tasks(resolver.clone())
}

/// What a new run is created for.
struct NewRun {
    /// Behavior the run executes (`request.behavior_name` of a non-fork run).
    behavior: String,
    /// Entry configuration, `None` for a session without behaviors.
    entry: Option<BehaviorEntry>,
    /// Sub context: the call it answers and the caller's run.
    child: Option<(ChildCall, String)>,
    depth: usize,
}

/// A fresh llm_context for the behavior the session is entering (§4.4). How
/// it is built is decided by the target's entry mode:
///
/// - the session's first run / a `switch_context` target entered for the
///   first time: its own system, tools and model; history as configured
///   (`inherit`), never another context's snapshot;
/// - `create_sub_context` child: its own system and configuration plus the
///   selected history of the caller (`derive_child`);
/// - `fork` child: the caller's system, configuration and complete effective
///   history at the fork point (`fork_snapshot`).
///
/// A child continues the caller's step / action numbering; what it inherited
/// is never written to the worklog by it (`HostMeta.inherited_below`,
/// `base_input_len`).
pub(super) async fn new_run_context(
    sh: &Arc<Shared>,
    binding: &Binding,
    env: &SessionEnv,
) -> Result<LiveCtx> {
    let (cfg, new) = {
        let s = sh.session.lock().await;
        let cfg = s.config.clone();
        let behavior = s
            .state
            .current_behavior
            .clone()
            .or_else(|| cfg.prompt.behavior.clone())
            .unwrap_or_default();
        let child = s
            .state
            .process_stack
            .last()
            .filter(|f| f.role == FrameRole::Caller && s.state.live_run.is_none())
            .and_then(|f| f.call.clone().map(|c| (c, f.run_id.clone())));
        let depth = s.state.call_depth();
        (
            cfg,
            NewRun {
                behavior,
                entry: None,
                child,
                depth,
            },
        )
    };
    let mut new = new;
    if !new.behavior.is_empty() {
        new.entry = Some(sh.deps.assembler.behavior_entry(&cfg, &new.behavior)?);
    }
    match &new.child {
        Some((call, parent)) if call.mode == ContextMode::Fork => {
            fork_run_context(sh, env, parent, new.depth).await
        }
        _ => own_run_context(sh, binding, env, &cfg, &new).await,
    }
}

fn parent_snapshot(sh: &Shared, parent: &str) -> Result<(RunRecord, LLMContextSnapshot)> {
    let (record, snap) = sh.dir.runs().load_checked(parent)?;
    let snap = snap
        .ok_or_else(|| OpenDanError::blocked("the caller's run has no snapshot", Some(parent)))?;
    Ok((record, snap))
}

/// fork child: a new run with the caller's configuration, system and
/// complete effective history up to the fork point.
async fn fork_run_context(
    sh: &Arc<Shared>,
    env: &SessionEnv,
    parent: &str,
    depth: usize,
) -> Result<LiveCtx> {
    let (parent_record, parent_snap) = parent_snapshot(sh, parent)?;
    let sid = sh.dir.sid().to_string();
    let runs = sh.dir.runs();
    let (run_id, lock) = runs.create_locked()?;
    let derived = llm_context::fork_snapshot(
        &parent_snap,
        llm_context::ForkOptions {
            trace: Some(run_id.clone()),
            ..Default::default()
        },
    )
    .map_err(|e| OpenDanError::InvalidArgument(e.to_string()))?;
    let now = crate::now_ms();
    let mut record = RunRecord {
        run_id: run_id.clone(),
        status: RunStatus::Running,
        created_at_ms: now,
        updated_at_ms: now,
        pending_input: None,
        latest_snapshot_idx: None,
        last_error: None,
        result: None,
        artifacts: Vec::new(),
        usage: Default::default(),
        limit_reason: None,
        interrupt_reason: None,
        compactions: 0,
        pid: std::process::id(),
        host_commit_pending: None,
        inflight: Vec::new(),
        handover: None,
        ..parent_record
    };
    if let Some(host) = record.host.as_mut() {
        if let Some(extra) = host.extra.as_object_mut() {
            extra.remove("reports");
            extra.remove("finish");
            extra.remove("tasks");
        }
    }
    let xdeps = xllm_deps_for(sh, env, depth).await?;
    let manager = rebuild_toolset(&record, &xdeps).await?;
    let resolver = manager.resolver();
    let llm = create_run_llm(&record, &xdeps).await?;
    let config = record.config.clone();
    let workdir = PathBuf::from(&record.workdir);
    let run = RunHandle::new(runs.store().clone(), record, lock);
    run.write()?;
    let tools = SessionToolManager::new(
        sh.clone(),
        Arc::new(manager),
        run.clone(),
        sh.lease.clone(),
        workdir,
        sh.touched.clone(),
        sh.current_tool.clone(),
    );
    let (ctx_llm, rounds) = counted(sh, run.run_id(), llm.clone());
    let deps = checkpoint_deps(sh, &run, &config, ctx_llm, tools, rounds.clone(), &resolver);
    let mut snap = derived.snapshot;
    let meta = HostMeta {
        session_id: sid,
        base_input_len: derived.boundary.messages as u64,
        process_entry: {
            let s = sh.session.lock().await;
            s.state.process_entry.clone()
        },
        inherited_below: derived.boundary.steps_below,
        input_receipts: Vec::new(),
        ..Default::default()
    };
    snap.state.host = Some(with_host_meta(None, &meta));
    let ctx = LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, deps.clone())
        .map_err(|e| OpenDanError::Llm(format!("fork child: {e}")))?;
    *sh.interrupt.lock().expect("interrupt") = Some(ctx.interrupt_handle());
    Ok(LiveCtx {
        behavior: config.loop_model == LoopModel::Behavior,
        ctx,
        run,
        ready: false,
        filled: false,
        deps,
        rounds,
        summary_llm: llm,
        resolver,
    })
}

fn session_env_check(sh: &Shared, env: &SessionEnv) -> Result<Value> {
    let mut values = std::collections::BTreeMap::new();
    let mut refs = Vec::new();
    for (key, value) in &env.env {
        let upper = key.to_ascii_uppercase();
        if ["TOKEN", "PASSWORD", "SECRET", "PRIVATE_KEY"]
            .iter()
            .any(|word| upper.contains(word))
        {
            refs.push(key.clone());
        } else {
            values.insert(key.clone(), value.clone());
        }
    }
    let manifest_path = sh
        .dir
        .runtime_bin_dir()
        .join(crate::runtime::bin_overlay::MANIFEST);
    let manifest: Value = crate::fsutil::read_json(&manifest_path)?;
    Ok(
        json!({"runtime": sh.deps.runtime.descriptor(), "env": values, "env_refs": refs,
        "path_prefix": env.path_layers, "bin_dir": sh.dir.runtime_bin_dir(), "bin_manifest": manifest}),
    )
}

/// A run with its own system and configuration: the session's first run, a
/// `switch_context` target, or a `create_sub_context` child.
async fn own_run_context(
    sh: &Arc<Shared>,
    binding: &Binding,
    env: &SessionEnv,
    session_cfg: &SessionConfig,
    new: &NewRun,
) -> Result<LiveCtx> {
    let sid = sh.dir.sid().to_string();
    let mut cfg = match &new.entry {
        Some(e) => context_config(session_cfg, e),
        None => session_cfg.clone(),
    };
    if !new.behavior.is_empty() {
        // The run's own view: the assembler renders this behavior's system.
        cfg.prompt.behavior = Some(new.behavior.clone());
    }
    let frozen_budget = session_cfg
        .prompt
        .frozen
        .as_ref()
        .and_then(|f| f.behaviors.get(&new.behavior))
        .map(|b| (b.budget.clone(), b.model.clone()));
    // A session without behaviors keeps reading the session history.
    let inherit = new
        .entry
        .as_ref()
        .map(|e| e.inherit)
        .unwrap_or(InheritMode::RecentDialogue);
    let system = sh
        .deps
        .assembler
        .system_text(&cfg, sh.agent_root.as_deref())
        .await?;
    let xdeps = xllm_deps_for(sh, env, new.depth).await?;
    let mut llm_ctx = if cfg.prompt.llm_context.is_null() {
        default_llm_context()
    } else {
        cfg.prompt.llm_context.clone()
    };
    let function_call = llm_ctx
        .get("loop_model")
        .and_then(Value::as_str)
        .unwrap_or("function_call")
        == "function_call";
    let explicit_report = cfg.session.policy.completion == CompletionPolicy::ExplicitReport;
    let enabled = llm_ctx.pointer("/tools/enabled").and_then(Value::as_bool);
    if function_call && explicit_report && enabled == Some(false) {
        return Err(OpenDanError::InvalidArgument(
            "explicit_report function_call sessions require tools.enabled=true".into(),
        ));
    }
    if function_call
        && (explicit_report || (enabled == Some(true) && llm_ctx.pointer("/tools/tools").is_none()))
    {
        if !llm_ctx.get("tools").is_some_and(Value::is_object) {
            llm_ctx["tools"] = json!({});
        }
        llm_ctx["tools"]["enabled"] = json!(true);
        if llm_ctx.pointer("/tools/tools").is_none() {
            llm_ctx["tools"]["tools"] = if enabled == Some(true) {
                json!([{"groupname":"bash"}])
            } else {
                json!([])
            };
        }
        let sources = llm_ctx["tools"]["tools"]
            .as_array_mut()
            .ok_or_else(|| OpenDanError::InvalidArgument("tools.tools must be an array".into()))?;
        if !sources
            .iter()
            .any(|t| t.get("name").and_then(Value::as_str) == Some(TOOL_REPORT))
        {
            sources.push(json!({"name":TOOL_REPORT}));
        }
    }
    llm_ctx["runtime"] = serde_json::to_value(sh.deps.runtime.config()).unwrap();
    let hosted = XllmTask::prepare_hosted(
        sh.dir.path(),
        &llm_ctx,
        &sh.dir
            .path()
            .join("session_config.json")
            .display()
            .to_string(),
        &system,
        &xdeps,
    )
    .await?;
    let llm = hosted.create_llm(&xdeps).await?;
    // History (may compact first).
    let budget = cfg
        .prompt
        .history_budget_tokens
        .unwrap_or(sh.deps.options.history_budget_tokens);
    let summarizer: Arc<dyn Summarizer> = match &sh.deps.summarizer {
        Some(s) => s.clone(),
        None => Arc::new(LlmSummarizer {
            llm: llm.clone(),
            model: hosted.config.model.clone(),
        }),
    };
    // The session history is a view selected by the target's entry
    // (`inherit: recent_dialogue`), never attached implicitly.
    let history = if inherit == InheritMode::RecentDialogue {
        let mut s = sh.session.lock().await;
        build_history(&mut s, &sh.lease, Some(summarizer.as_ref()), budget)
            .await?
            .0
    } else {
        None
    };
    let runs = sh.dir.runs();
    let (run_id, lock) = runs.create_locked()?;
    let record = hosted.new_record(
        &run_id,
        Some(runs.dir()),
        &cfg.session.objective,
        HostRunInfo {
            assembled_by: "libopendan".into(),
            session_id: Some(sid.clone()),
            runtime_kind: Some(binding.kind.clone()),
            runtime_id: Some(binding.runtime_id.clone()),
            env_check: session_env_check(sh, env)?,
            extra: cfg.workspace_binding.as_ref().map_or(
                Value::Null,
                |workspace| json!({"workspace_binding": workspace}),
            ),
        },
    );
    let system_prompt = hosted.prompt.system_prompt.clone();
    let config = hosted.config.clone();
    let mut manager = hosted.manager;
    manager.bind_run_dir(&run_id, Some(runs.dir().join(&run_id)));
    let resolver = manager.resolver();
    let run = RunHandle::new(runs.store().clone(), record, lock);
    run.write()?;
    let tools = SessionToolManager::new(
        sh.clone(),
        Arc::new(manager),
        run.clone(),
        sh.lease.clone(),
        PathBuf::from(&binding.workdir),
        sh.touched.clone(),
        sh.current_tool.clone(),
    );
    let mut input = vec![AiMessage::text(AiRole::System, system_prompt)];
    if let Some(h) = history {
        input.push(h);
    }
    let behavior_name = new.behavior.clone();
    let mut request = agent_tool::xllm::hosted_request(
        &config,
        ContextOwnerRef::Agent {
            session_id: sid.clone(),
        },
        &run_id,
        &cfg.session.objective,
        &behavior_name,
        input.clone(),
    );
    // A call may suspend the run (`PendingTool`): on a sub context, or on
    // a task this session waits for outside the context and fills in on
    // resume (串行等待).
    request.tool_policy.allow_deferred = true;
    // The frozen behavior's budget for the whole run (the limits xllm reads
    // from `.llm_context` were laid over the config already).
    if let Some((budget, model)) = frozen_budget {
        if let Some(n) = budget.max_total_tokens {
            request.budget.max_total_tokens = Some(n);
        }
        if let Some(n) = budget.max_wallclock_ms {
            request.budget.max_wallclock_ms = Some(n);
        }
        if let Some(n) = budget.max_consecutive_errors {
            request.error_policy.max_consecutive_errors = n;
        }
        if !model.fallbacks.is_empty() {
            request.model_policy.fallbacks = model.fallbacks;
        }
        if model.temperature.is_some() {
            request.model_policy.temperature = model.temperature;
        }
    }
    let (ctx_llm, rounds) = counted(sh, run.run_id(), llm.clone());
    let deps = checkpoint_deps(sh, &run, &config, ctx_llm, tools, rounds.clone(), &resolver);
    let mut inherited_below = 0;
    let mut ctx = match &new.child {
        // create-sub-context: the caller's selected history, its numbering.
        Some((_, parent)) => {
            let (_, parent_snap) = parent_snapshot(sh, parent)?;
            let derived = llm_context::derive_child(
                &parent_snap,
                request,
                if inherit == InheritMode::Steps {
                    llm_context::InheritHistory::Steps
                } else {
                    llm_context::InheritHistory::None
                },
            )
            .map_err(|e| OpenDanError::InvalidArgument(e.to_string()))?;
            inherited_below = derived.boundary.steps_below;
            LLMContext::resume(derived.snapshot, ResumeFill::ResumeFromMidRun, deps.clone())
                .map_err(|e| OpenDanError::Llm(format!("sub context: {e}")))?
        }
        None => LLMContext::new(request, deps.clone()),
    };
    let meta = HostMeta {
        session_id: sid,
        base_input_len: input.len() as u64,
        process_entry: {
            let s = sh.session.lock().await;
            s.state.process_entry.clone()
        },
        inherited_below,
        input_receipts: Vec::new(),
        ..Default::default()
    };
    ctx.set_host_meta(Some(with_host_meta(None, &meta)));
    *sh.interrupt.lock().expect("interrupt") = Some(ctx.interrupt_handle());
    Ok(LiveCtx {
        behavior: config.loop_model == LoopModel::Behavior,
        ctx,
        run,
        ready: false,
        filled: false,
        deps,
        rounds,
        summary_llm: llm,
        resolver,
    })
}

/// Whether a suspended call's wait is over: the task ended (or its state is
/// unknown), or `until_ms` passed — the call is then answered with the
/// task's state at this moment.
fn wait_over(state: &TaskState, until_ms: Option<u64>) -> bool {
    !matches!(state, TaskState::Running { .. }) || until_ms.is_some_and(|t| crate::now_ms() >= t)
}

/// Results for every suspended call of `snapshot`, or `None` while a task
/// is still being waited for. One query path for notifications, the
/// fallback poll, the drive entry and a take-over. `force`: answer with the
/// current state whatever it is (stop).
async fn pending_results(
    snapshot: &LLMContextSnapshot,
    resolver: &dyn RunningTaskResolver,
    returned: Option<&Value>,
    force: bool,
) -> Option<Vec<(String, Observation)>> {
    let returned_call = returned
        .and_then(|r| r.get("call_id"))
        .and_then(Value::as_str);
    let mut results = Vec::new();
    for p in snapshot.state.pending_calls() {
        let call_id = p.call.call_id.as_str();
        if returned_call == Some(call_id) {
            results.push((
                call_id.to_string(),
                sub_result_observation(call_id, returned.unwrap_or(&Value::Null)),
            ));
            continue;
        }
        let state = if p.task_id.starts_with(SUB_CONTEXT_TASK_PREFIX) {
            // A sub context call that never ran (it was not the only
            // suspended call of its batch).
            TaskState::Unknown {
                reason: "the sub context was not started; call it again by itself".into(),
            }
        } else {
            resolver.state(&p.task_id).await
        };
        if !force && !wait_over(&state, p.until_ms) {
            return None;
        }
        results.push((
            call_id.to_string(),
            task_state_observation(call_id, &p.task_id, &state),
        ));
    }
    Some(results)
}

/// Fill the suspended calls of a waiting run when their wait is over and
/// resume the same run (the open Turn continues); polling a task that still
/// runs needs no inference. `force` (stop): cancellable tasks are cancelled
/// and every call is answered now.
pub(super) async fn try_fill(
    sh: &Arc<Shared>,
    w: WaitingRun,
    force: bool,
) -> Result<std::result::Result<LiveCtx, WaitingRun>> {
    if force {
        for p in w.snapshot.state.pending_calls() {
            if !p.task_id.starts_with(SUB_CONTEXT_TASK_PREFIX) {
                let _ = w.resolver.cancel(&p.task_id).await;
            }
        }
    }
    let returned = sh.session.lock().await.state.process_result.clone();
    let Some(results) =
        pending_results(&w.snapshot, w.resolver.as_ref(), returned.as_ref(), force).await
    else {
        return Ok(Err(w));
    };
    let run_id = w.run.run_id().to_string();
    let waited = w.task_ids();
    let ctx = LLMContext::resume(
        w.snapshot,
        ResumeFill::ToolResults { results },
        w.deps.clone(),
    )
    .map_err(|e| {
        OpenDanError::blocked(format!("snapshot cannot be resumed: {e}"), Some(&run_id))
    })?;
    // The results are in the run before anything runs on.
    w.run
        .checkpoint_with_results(&ctx.snapshot(), Some(RunStatus::Running))?;
    crate::fault::point("pending_tool:after_fill");
    {
        let marked = super::children::mark_waited(sh, &waited).await?;
        let mut s = sh.session.lock().await;
        if s.state.run_state == RunState::Waiting || marked {
            if s.state.run_state == RunState::Waiting {
                s.state.run_state = RunState::Running;
                s.state.waiting_for = None;
            }
            s.commit_state(&sh.lease)?;
        }
    }
    *sh.interrupt.lock().expect("interrupt") = Some(ctx.interrupt_handle());
    Ok(Ok(LiveCtx {
        behavior: w.behavior,
        ctx,
        run: w.run,
        ready: true,
        filled: true,
        deps: w.deps,
        rounds: w.rounds,
        summary_llm: w.summary_llm,
        resolver: w.resolver,
    }))
}

/// Resume the live run (§8.6): receipts reconciled by `reconcile_runs`.
pub(super) async fn resume_live_run(
    sh: &Arc<Shared>,
    run: RunHandle,
    snapshot: LLMContextSnapshot,
    env: &SessionEnv,
) -> Result<Opened> {
    let record = run.record();
    let run_id = record.run_id.clone();
    let depth = sh.session.lock().await.state.call_depth();
    let xdeps = xllm_deps_for(sh, env, depth).await?;
    let blocked = |e: String| OpenDanError::blocked(e, Some(&run_id));
    let manager = rebuild_toolset(&record, &xdeps)
        .await
        .map_err(|e| blocked(format!("cannot rebuild the run's tools: {e}")))?;
    let resolver = manager.resolver();
    let llm = create_run_llm(&record, &xdeps)
        .await
        .map_err(|e| blocked(format!("cannot create the run's provider: {e}")))?;
    let behavior = record.config.loop_model == LoopModel::Behavior;
    let mut snapshot = snapshot;
    // In-flight actions without a persisted result → explicit "interrupted,
    // result unknown", worded by the runtime from what it can read (long-tool
    // TODO §3.2); persisted before any further inference. No process is
    // verified or stopped.
    // A call the snapshot is suspended on is not interrupted: it waits for
    // its task and is answered by the fill below.
    let suspended_calls: Vec<String> = snapshot
        .state
        .pending_calls()
        .iter()
        .map(|p| p.call.call_id.clone())
        .collect();
    let interrupted: Vec<_> = record
        .inflight
        .iter()
        .filter(|a| !suspended_calls.contains(&a.call_id))
        .cloned()
        .collect();
    if !interrupted.is_empty() {
        let binding = RunBinding {
            run_id: run_id.clone(),
            run_dir: Some(sh.dir.runs().dir().join(&run_id)),
        };
        let mut reasons = std::collections::HashMap::new();
        for action in &interrupted {
            reasons.insert(
                action.call_id.clone(),
                sh.deps.runtime.describe_interrupted(&binding, action).await,
            );
        }
        materialize_unresolved(&mut snapshot, &interrupted, behavior, &reasons);
        run.checkpoint_with_results(&snapshot, None)?;
    }
    // Suspended on tool calls (`PendingTool`). A sub context call gets the
    // child's hand-back; any other call waits for a task this runner must
    // be able to ask about — a task of a task manager it cannot reach is
    // not taken over (the run is kept as it is). A task it can ask about
    // but that is gone (an in-process task of a previous process) is
    // answered `Unknown` by the resolver and filled as such: never
    // `RecoveryBlocked`, never silently re-created.
    let returned = sh.session.lock().await.state.process_result.clone();
    let returned_call = returned
        .as_ref()
        .and_then(|r| r.get("call_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let suspended_on_tools = matches!(
        snapshot.state.suspended,
        Some(Suspension::PendingTool { .. })
    );
    if suspended_on_tools {
        for p in snapshot.state.pending_calls() {
            if returned_call.as_deref() == Some(p.call.call_id.as_str())
                || p.task_id.starts_with(SUB_CONTEXT_TASK_PREFIX)
            {
                continue;
            }
            if !resolver.can_resolve(&p.task_id) {
                return Err(blocked(format!(
                    "the run waits for task {} of call {}, which this runner cannot resolve",
                    p.task_id, p.call.call_id
                )));
            }
        }
    }
    if behavior {
        // Back from a sub context: continue its action / step numbering.
        if let Some(r) = &returned {
            if let Some(n) = r.get("next_action_id").and_then(Value::as_u64) {
                snapshot.state.next_action_id = snapshot.state.next_action_id.max(n as u32);
            }
            if let Some(n) = r.get("next_step_index").and_then(Value::as_u64) {
                snapshot.state.next_step_index = snapshot.state.next_step_index.max(n as u32);
            }
        }
    }
    let workdir = PathBuf::from(&record.workdir);
    let tools = SessionToolManager::new(
        sh.clone(),
        Arc::new(manager),
        run.clone(),
        sh.lease.clone(),
        workdir,
        sh.touched.clone(),
        sh.current_tool.clone(),
    );
    let (ctx_llm, rounds) = counted(sh, run.run_id(), llm.clone());
    let deps = checkpoint_deps(
        sh,
        &run,
        &record.config,
        ctx_llm,
        tools,
        rounds.clone(),
        &resolver,
    );
    if suspended_on_tools {
        let w = WaitingRun {
            run,
            snapshot,
            behavior,
            deps,
            rounds,
            summary_llm: llm,
            resolver,
        };
        let opened = match try_fill(sh, w, false).await? {
            Ok(lc) => {
                crate::fault::point("sub_return:after_fill");
                Opened::Ctx(lc)
            }
            Err(w) => Opened::Waiting(w),
        };
        if returned_call.is_some() && matches!(opened, Opened::Ctx(_)) {
            // Delivered (now, or before a crash): the hand-back is consumed.
            let mut s = sh.session.lock().await;
            s.state.process_result = None;
            s.commit_state(&sh.lease)?;
        }
        if let Opened::Ctx(lc) = &opened {
            lc.run.set_status(RunStatus::Running, None)?;
        }
        return Ok(opened);
    }
    let ctx = if matches!(
        snapshot.state.suspended,
        Some(Suspension::ContextLimit { .. })
    ) {
        // Paused at the context limit (compactions exhausted, or the
        // rewrite did not complete): compact again before running on.
        rewrite_for_limit(sh, &run, snapshot, behavior, &deps, &llm, 1).await?
    } else {
        LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps.clone())
            .map_err(|e| blocked(format!("snapshot cannot be resumed: {e}")))?
    };
    if returned_call.is_some() {
        // Delivered before a crash: the hand-back is consumed.
        let mut s = sh.session.lock().await;
        s.state.process_result = None;
        s.commit_state(&sh.lease)?;
    }
    *sh.interrupt.lock().expect("interrupt") = Some(ctx.interrupt_handle());
    if let Some(report) = super::reports::final_report(&run)? {
        if let ReportSource::Tool { call_id } = &report.source {
            if agent_tool::exec_tracking::persisted_outcome_ids(&ctx.snapshot()).contains(call_id) {
                ctx.interrupt_handle()
                    .finish("resume accepted final report");
            }
        }
    }
    run.set_status(RunStatus::Running, None)?;
    Ok(Opened::Ctx(LiveCtx {
        behavior,
        ctx,
        run,
        ready: true,
        filled: false,
        deps,
        rounds,
        summary_llm: llm,
        resolver,
    }))
}

/// Run the live context: one run segment of the drive loop, ending in one
/// outcome. A context-limit suspension is compacted and the context runs on
/// (another `run()` call in the same segment), at most
/// [`MAX_LIMIT_COMPACTIONS`] times in a row; the last one goes to
/// `handle_context_outcome`, which pauses the run.
pub(super) async fn run_compacting(
    sh: &Arc<Shared>,
    lc: &mut LiveCtx,
) -> Result<LLMContextOutcome> {
    let mut attempt = 0;
    loop {
        let outcome = lc.ctx.run().await;
        // `input.media = inline` and the provider refused the request (the
        // model takes no images / documents, or an object is unreadable):
        // degrade mechanically, once — drop the media blocks, say so in the
        // text (which still locates every attachment) and run again.
        if let LLMContextOutcome::Error {
            error:
                LLMComputeError::Provider {
                    failure: ProviderFailure::Permanent | ProviderFailure::Unknown,
                    ..
                },
            ..
        } = &outcome
        {
            if let Some(ctx) = degrade_inline_media(sh, lc)? {
                lc.ctx = ctx;
                *sh.interrupt.lock().expect("interrupt") = Some(lc.ctx.interrupt_handle());
                continue;
            }
        }
        let LLMContextOutcome::ContextLimitReached { snapshot, .. } = &outcome else {
            return Ok(outcome);
        };
        if attempt >= MAX_LIMIT_COMPACTIONS {
            return Ok(outcome);
        }
        attempt += 1;
        lc.ctx = rewrite_for_limit(
            sh,
            &lc.run,
            snapshot.clone(),
            lc.behavior,
            &lc.deps,
            &lc.summary_llm,
            attempt,
        )
        .await?;
        *sh.interrupt.lock().expect("interrupt") = Some(lc.ctx.interrupt_handle());
    }
}

/// Appended to a user message whose media blocks were removed.
pub const MEDIA_DEGRADED_NOTE: &str = "[The image / document blocks of this message were removed because the model request was refused with them. Read the attachments listed above with tools if you need their content.]";

fn strip_media(m: &mut AiMessage) -> bool {
    if m.role != AiRole::User {
        return false;
    }
    let before = m.content.len();
    m.content
        .retain(|c| !matches!(c, AiContent::Image { .. } | AiContent::Document { .. }));
    if m.content.len() == before {
        return false;
    }
    match m.content.iter_mut().find_map(|c| match c {
        AiContent::Text { text } => Some(text),
        _ => None,
    }) {
        Some(text) => {
            text.push('\n');
            text.push_str(MEDIA_DEGRADED_NOTE);
        }
        None => m.content.push(AiContent::Text {
            text: MEDIA_DEGRADED_NOTE.to_string(),
        }),
    }
    true
}

/// Remove the inline media blocks of the run's user messages and resume it.
/// `None`: nothing to remove, or it was already done once for this run.
fn degrade_inline_media(sh: &Arc<Shared>, lc: &LiveCtx) -> Result<Option<LLMContext>> {
    let mut snapshot = lc.ctx.snapshot();
    let mut meta = snapshot_host_meta(&snapshot);
    if meta.media_degraded || snapshot.state.suspended.is_some() {
        return Ok(None);
    }
    let mut stripped = false;
    for m in snapshot
        .request
        .input
        .iter_mut()
        .chain(snapshot.state.accumulated.iter_mut())
    {
        stripped |= strip_media(m);
    }
    for step in snapshot
        .state
        .steps
        .iter_mut()
        .chain(snapshot.state.last_step.iter_mut())
    {
        if let Some(m) = step.next_user_message.as_mut() {
            stripped |= strip_media(m);
        }
    }
    if !stripped {
        return Ok(None);
    }
    meta.media_degraded = true;
    snapshot.state.host = Some(with_host_meta(snapshot.state.host.as_ref(), &meta));
    let run_id = lc.run.run_id().to_string();
    let ctx = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, lc.deps.clone())
        .map_err(|e| OpenDanError::blocked(format!("media degrade: {e}"), Some(&run_id)))?;
    lc.run
        .checkpoint_with_results(&ctx.snapshot(), Some(RunStatus::Running))?;
    log::warn!(
        "session {}: provider refused run {run_id} with inline media; retrying once with references only",
        sh.dir.sid()
    );
    Ok(Some(ctx))
}

/// Mid-run rewrite of a run suspended at the context limit (§4.4, X7):
///
/// 1. the run's history so far is flushed to the worklog, closed by a
///    `context_rewritten` outcome entry, and state commits the flush marks
///    (nothing of the run lives only in the snapshot);
/// 2. the session history is compacted (summary.json) so that at most
///    `budget >> attempt` tokens of raw records remain, and the history
///    message is rebuilt;
/// 3. the context resumes with system + history as its new input
///    (`RewrittenHistory` / `RewrittenSteps`) in a new history epoch, and the
///    rewritten snapshot is published before anything runs on.
///
/// A crash before 3 leaves the pre-inference snapshot of the old epoch
/// (already flushed, so nothing is written twice); after 3 the new epoch
/// counts its messages from zero. Receipts keep their `input_seq`; only
/// their positions are left behind in the old epoch. The open Turn is not
/// affected: the rewrite only changes how its history is carried.
async fn rewrite_for_limit(
    sh: &Arc<Shared>,
    run: &RunHandle,
    snapshot: LLMContextSnapshot,
    behavior: bool,
    deps: &LLMContextDeps,
    summary_llm: &Arc<dyn LlmClient>,
    attempt: u32,
) -> Result<LLMContext> {
    let run_id = run.run_id().to_string();
    let summarizer: Arc<dyn Summarizer> = match &sh.deps.summarizer {
        Some(s) => s.clone(),
        None => Arc::new(LlmSummarizer {
            llm: summary_llm.clone(),
            model: run.record().config.model.clone(),
        }),
    };
    let (history, turn) = {
        let mut s = sh.session.lock().await;
        let live = s
            .state
            .live_run
            .clone()
            .filter(|l| l.run_id == run_id)
            .ok_or_else(|| OpenDanError::Other(format!("run {run_id} is not the live run")))?;
        let turn = s.state.current_turn();
        let (mut bodies, marks) =
            run_history_entries(&run_id, &snapshot, behavior, FlushMarks::of(&live), turn);
        // The boundary is written with the history it closes: a redo after a
        // crash, or another rewrite before anything new ran, adds nothing.
        if !bodies.is_empty() {
            bodies.push(WorklogBody::Outcome {
                run_id: run_id.clone(),
                turn,
                kind: "context_rewritten".into(),
                next_behavior: None,
                report: None,
            });
        }
        s.append_worklog(&sh.lease, bodies)?;
        if let Some(l) = s.state.live_run.as_mut() {
            marks.apply(l);
        }
        s.commit_state(&sh.lease)?;
        crate::fault::point("context_limit:after_flush");
        let budget = s
            .config
            .prompt
            .history_budget_tokens
            .unwrap_or(sh.deps.options.history_budget_tokens);
        let keep = budget >> attempt.min(8);
        let history =
            compact_for_limit(&mut s, &sh.lease, summarizer.as_ref(), budget, keep).await?;
        crate::fault::point("context_limit:after_compact");
        (history, turn)
    };
    let mut input: Vec<AiMessage> = snapshot
        .request
        .input
        .iter()
        .take_while(|m| m.role == AiRole::System)
        .cloned()
        .collect();
    input.extend(history);
    let mut meta = snapshot_host_meta(&snapshot);
    meta.history_epoch += 1;
    meta.epoch_turn = turn;
    meta.epoch_input_seq = meta
        .input_receipts
        .iter()
        .map(|r| r.input_seq)
        .max()
        .unwrap_or(0);
    meta.base_input_len = input.len() as u64;
    let fill = if behavior {
        ResumeFill::RewrittenSteps {
            input,
            history_summaries: Vec::new(),
            steps: Vec::new(),
            last_step: None,
        }
    } else {
        ResumeFill::RewrittenHistory { history: input }
    };
    let mut ctx = LLMContext::resume(snapshot, fill, deps.clone())
        .map_err(|e| OpenDanError::blocked(format!("context limit rewrite: {e}"), Some(&run_id)))?;
    let host = with_host_meta(ctx.host_meta(), &meta);
    ctx.set_host_meta(Some(host));
    run.checkpoint_with_results(&ctx.snapshot(), Some(RunStatus::Running))?;
    crate::fault::point("context_limit:after_publish");
    Ok(ctx)
}

/// Open the run `state.live_run` points to (a caller resumed after its sub
/// context returned, or a parked context re-entered).
pub(super) async fn open_state_live_run(sh: &Arc<Shared>, env: &SessionEnv) -> Result<Opened> {
    let (run_id, tool_return) = {
        let s = sh.session.lock().await;
        let run_id = s
            .state
            .live_run
            .as_ref()
            .map(|l| l.run_id.clone())
            .ok_or_else(|| OpenDanError::Other("no live run to open".into()))?;
        (run_id, s.state.tool_return_pending())
    };
    let runs = sh.dir.runs();
    let lock = runs
        .try_lock(&run_id)?
        .ok_or_else(|| OpenDanError::RunBusy {
            run_id: run_id.clone(),
        })?;
    let (record, snapshot) = runs.load_checked(&run_id)?;
    let snapshot = snapshot
        .ok_or_else(|| OpenDanError::blocked("suspended run has no snapshot", Some(&run_id)))?;
    let run = RunHandle::new(runs.store().clone(), record, lock);
    let mut opened = resume_live_run(sh, run, snapshot, env).await?;
    // Resumed on purpose: the hand-over batch brings the input. A caller
    // back from a tool-triggered sub context got its tool result instead
    // and runs on by itself.
    if let Opened::Ctx(lc) = &mut opened {
        lc.ready = tool_return;
    }
    Ok(opened)
}

/// Suspend the live run into `process_stack` (§4.4) and hand over to
/// `next_behavior`: flush its history so far (worklog keeps time order),
/// keep the run directory. `call` = the sub context it calls (the run is
/// the caller and gets the result back); `None` = SWITCH_CONTEXT (the run
/// is parked, and the target's own parked run, if any, becomes live again).
/// One state commit: redone after a crash, it is driven by the run's
/// hand-over record / pending call and lands here once.
pub(super) async fn suspend_run(
    sh: &Arc<Shared>,
    run: &RunHandle,
    behavior: bool,
    snapshot: &LLMContextSnapshot,
    next_behavior: &str,
    call: Option<ChildCall>,
) -> Result<()> {
    let run_id = run.run_id().to_string();
    let mut s = sh.session.lock().await;
    let live = s
        .state
        .live_run
        .clone()
        .filter(|l| l.run_id == run_id)
        .ok_or_else(|| OpenDanError::Other(format!("run {run_id} is not the live run")))?;
    let turn = s.state.current_turn();
    let (mut bodies, marks) =
        run_history_entries(&run_id, snapshot, behavior, FlushMarks::of(&live), turn);
    bodies.push(WorklogBody::Outcome {
        run_id: run_id.clone(),
        turn,
        kind: "suspended".into(),
        next_behavior: Some(next_behavior.to_string()),
        report: None,
    });
    s.append_worklog(&sh.lease, bodies)?;
    let entry = s
        .state
        .process_entry
        .clone()
        .or_else(|| s.state.current_behavior.clone())
        .or_else(|| s.config.prompt.behavior.clone())
        // A session without behaviors (function call loop) has no entry name.
        .unwrap_or_default();
    let switch = call.is_none();
    s.state.process_stack.push(ProcessFrame {
        entry,
        role: if switch {
            FrameRole::Parked
        } else {
            FrameRole::Caller
        },
        call,
        run_id: run_id.clone(),
        turns: live.turns,
        flushed_message_count: marks.messages,
        flushed_step_index: marks.step_index,
        flushed_input_seq: marks.input_seq,
        flushed_epoch: marks.epoch,
        applied_input_seq: live.applied_input_seq,
        handover_at_ms: run.record().handover.map(|h| h.at_ms).unwrap_or(0),
    });
    s.state.live_run = None;
    s.state.process_entry = Some(next_behavior.to_string());
    s.state.current_behavior = Some(next_behavior.to_string());
    s.state.internal_continuation = Some(next_behavior.to_string());
    s.state.run_state = RunState::Ready;
    s.state.waiting_for = None;
    s.state.last_error = None;
    // SWITCH_CONTEXT: re-enter the target's own parked run when it exists.
    if switch {
        if let Some(pos) = s
            .state
            .process_stack
            .iter()
            .position(|f| f.entry == next_behavior && f.role == FrameRole::Parked)
        {
            let f = s.state.process_stack.remove(pos);
            s.state.live_run = Some(live_from_frame(f));
        }
    }
    commit_and_report(sh, &mut s).await
}

/// Tool result a caller gets for its `call_behavior` call from the sub
/// context's hand-back (`state.process_result`).
fn sub_result_observation(call_id: &str, r: &Value) -> Observation {
    let text = |k: &str| {
        r.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match r.get("status").and_then(Value::as_str).unwrap_or("ok") {
        "failed" => Observation::Error {
            call_id: call_id.to_string(),
            message: format!(
                "sub context `{}` failed: {}",
                text("behavior"),
                text("result")
            ),
            tool_result: None,
        },
        status => {
            let content = if status == "needs_user_input" {
                json!({
                    "status": "needs_user_input",
                    "behavior": text("behavior"),
                    "question": text("result"),
                    "note": "the sub context cannot ask the user; ask the user yourself and call it again with the answer",
                })
                .to_string()
            } else if let Some(submission) = r.get("submission") {
                json!({"status": status, "behavior": text("behavior"), "submission": submission})
                    .to_string()
            } else {
                text("result")
            };
            Observation::Success {
                call_id: call_id.to_string(),
                bytes: content.len(),
                content: Value::String(content),
                truncated: false,
                tool_result: None,
            }
        }
    }
}

/// A suspended process becomes the live run again.
pub(super) fn live_from_frame(f: ProcessFrame) -> LiveRun {
    LiveRun {
        run_id: f.run_id,
        turns: f.turns,
        applied_input_seq: f.applied_input_seq,
        flushed_message_count: f.flushed_message_count,
        flushed_step_index: f.flushed_step_index,
        flushed_input_seq: f.flushed_input_seq,
        flushed_epoch: f.flushed_epoch,
        process_entry: Some(f.entry).filter(|e| !e.is_empty()),
        handover_at_ms: f.handover_at_ms,
    }
}

/// One controlled input ready to be committed.
pub(super) struct InputBatch<'a> {
    /// `on_init | on_input | on_context_switch`.
    pub hook: &'a str,
    /// msg / Input events this batch consumes, in consumption order.
    pub picked: &'a [FetchedInput],
    /// The semi-subscription snapshot message and the state versions it
    /// shows (injected before the controlled input).
    pub snapshot: Option<(String, Vec<EventReceipt>)>,
    /// The controlled input message: template output and media blocks.
    pub text: String,
    pub media: Vec<AiContent>,
}

/// Commit one input batch (§8.3): its 1–2 messages + receipt in one
/// snapshot → run.json gate → state.json → clear gate → confirm inputs. No
/// inference before the gate is clear, nothing dequeued before the commit.
/// The batch opens a new logical Turn when none is open (D1); otherwise it
/// joins the open one (hand-over, supplementary input); the snapshot
/// message never counts as a Turn by itself. Committing a batch never
/// completes a Turn.
pub(super) async fn commit_input_batch(
    sh: &Arc<Shared>,
    lc: &mut LiveCtx,
    batch: InputBatch<'_>,
) -> Result<()> {
    let mut s = sh.session.lock().await;
    let run_id = lc.run.run_id().to_string();
    let applied = s
        .state
        .live_run
        .as_ref()
        .filter(|l| l.run_id == run_id)
        .map(|l| l.applied_input_seq)
        .unwrap_or(0);
    if let Some(l) = &s.state.live_run {
        if l.run_id != run_id {
            return Err(OpenDanError::Other(format!(
                "state references live run {} while starting {run_id}",
                l.run_id
            )));
        }
    }
    let opens_turn = s.state.open_turn.is_none();
    // What the Turn is about, for its task: the first message of the batch.
    let title = batch
        .picked
        .iter()
        .find_map(|m| m.msg().map(|msg| msg.msg.content.content.clone()));
    let turn = if opens_turn {
        s.state.turn_seq + 1
    } else {
        s.state.current_turn()
    };
    let inputs: Vec<InputRef> = batch.picked.iter().map(|m| m.input_ref()).collect();
    let mut extra = std::collections::BTreeMap::new();
    if s.state.internal_continuation.is_some() {
        extra.insert("continuation".to_string(), Value::Bool(true));
    }
    // Default reply path after this batch: the way its last message came
    // (by consumption order); events, controls, the snapshot and a
    // hand-over by themselves leave it as it is.
    let reply = batch
        .picked
        .iter()
        .rev()
        .find_map(|m| m.msg().map(|msg| ReplyRoute::of_msg(&m.key, msg)))
        .or_else(|| s.state.reply.clone());
    let after_step = lc.ctx.snapshot().state.next_step_index;
    let mut messages = Vec::new();
    let mut part_texts: Vec<(&str, String)> = Vec::new();
    let mut events = Vec::new();
    if let Some((text, shown)) = batch.snapshot {
        messages.push(AiMessage::text(AiRole::User, text.clone()));
        part_texts.push((PART_SNAPSHOT, text));
        events = shown;
    }
    let mut input_msg = AiMessage::text(AiRole::User, batch.text.clone());
    input_msg.content.extend(batch.media);
    messages.push(input_msg);
    part_texts.push((PART_INPUT, batch.text));
    let count = messages.len();
    let pos = lc.ctx.inject(Injection {
        messages,
        host: None,
    });
    let positions = positions_of(pos, count);
    if positions.len() != count {
        return Err(OpenDanError::Other(
            "the input batch could not be placed into the context".into(),
        ));
    }
    let receipt = InputReceipt {
        run_id: run_id.clone(),
        input_seq: applied + 1,
        turn,
        opens_turn,
        hook: batch.hook.to_string(),
        inputs,
        events,
        reply,
        parts: part_texts
            .into_iter()
            .zip(positions)
            .map(|((part, text), pos)| ReceiptPart {
                part: part.to_string(),
                pos,
                text,
            })
            .collect(),
        bootstrap: !s.state.bootstrap_done,
        after_step,
        extra,
        at_ms: crate::now_ms(),
    };
    let mut meta = host_meta_of(lc.ctx.host_meta());
    meta.input_receipts.push(receipt.clone());
    let host = with_host_meta(lc.ctx.host_meta(), &meta);
    lc.ctx.set_host_meta(Some(host));
    let snap = lc.ctx.snapshot();
    lc.run.publish_input_checkpoint(&snap, receipt.input_seq)?; // ① ②
    crate::fault::point("input_batch:after_input_checkpoint");
    apply_receipt(&mut s.state, &receipt)?;
    if receipt.extra.contains_key("continuation") {
        s.state.internal_continuation = None;
        s.state.process_result = None;
    }
    s.state.run_state = RunState::Running;
    s.state.waiting_for = None;
    s.state.last_error = None;
    if s.state.current_behavior.is_none() {
        s.state.current_behavior = s.config.prompt.behavior.clone();
    }
    if s.state.activity.summary.is_empty() {
        s.state.activity.summary = s.config.session.objective.chars().take(160).collect();
    }
    if receipt.bootstrap {
        let me = sh.agent().sessions().lookup(s.sid()).await.ok().flatten();
        for t in scope_touching(&s.config, me.as_ref()) {
            s.state.activity.touch(t);
        }
    }
    s.state.activity.heartbeat_ms = crate::now_ms();
    commit_and_report(sh, &mut s).await?; // ③
    crate::fault::point("input_batch:after_state_commit");
    lc.run.complete_host_commit()?; // ④
    crate::fault::point("input_batch:after_gate_clear");
    confirm_inputs(&sh.sources, &s.state).await; // ⑤
    drop(s);
    *sh.current_tool.lock().expect("current tool") = None;
    super::turn_task::ensure_turn_task(sh, title).await;
    Ok(())
}
