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
use buckyos_api::{AiMessage, AiRole};
use llm_context::deps::{Injection, LLMContextDeps, LlmClient};
use llm_context::observation::Observation;
use llm_context::outcome::{LLMContextOutcome, ResumeFill};
use llm_context::request::ContextOwnerRef;
use llm_context::state::{LLMContextSnapshot, Suspension};
use llm_context::LLMContext;
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::runtime::SessionEnv;
use crate::session::runs::RunHandle;

use super::flush::{run_history_entries, FlushMarks};
use super::history::{build_history, compact_for_limit, LlmSummarizer, Summarizer};
use super::hook::{scope_touching, Changes, SessionCheckpointHook};
use super::inputs::confirm_inputs;
use super::receipts::{
    apply_receipt, host_meta_of, position_of, snapshot_host_meta, with_host_meta,
};
use super::shared::{commit_and_report, counted, LiveCtx, Shared};
use super::tools::{CallBehaviorTool, SessionToolManager};

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
    let behaviors = {
        let s = sh.session.lock().await;
        s.config.behaviors().map_err(OpenDanError::InvalidArgument)?
    };
    if let Some(tool) = CallBehaviorTool::new(&behaviors, depth) {
        x.host_tools
            .insert(TOOL_CALL_BEHAVIOR.to_string(), Arc::new(tool));
    }
    Ok(x)
}

/// Whether the run's tool set has `call_behavior` (the run may then be
/// suspended on a sub context).
fn calls_sub_contexts(cfg: &EffectiveConfig) -> bool {
    cfg.tools.all_names().iter().any(|n| n == TOOL_CALL_BEHAVIOR)
}

/// The configuration a behavior's own context runs with: the session's,
/// with the entry's application system prompt and `llm_context` keys.
fn context_config(cfg: &SessionConfig, entry: &BehaviorEntry) -> SessionConfig {
    let mut c = cfg.clone();
    if let Some(p) = &entry.system_prompt {
        c.prompt.system_prompt = Some(p.clone());
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
) -> llm_context::deps::LLMContextDeps {
    let behavior = cfg.loop_model == LoopModel::Behavior;
    let hook = Arc::new(SessionCheckpointHook::new(
        sh.clone(),
        run.clone(),
        behavior,
    ));
    hosted_waist_deps(cfg, llm, Arc::new(tools)).with_checkpoint_hook(hook)
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
    let record = RunRecord {
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
    let xdeps = xllm_deps_for(sh, env, depth).await?;
    let manager = rebuild_toolset(&record, &xdeps).await?;
    let llm = create_run_llm(&record, &xdeps).await?;
    let config = record.config.clone();
    let workdir = PathBuf::from(&record.workdir);
    let run = RunHandle::new(runs.store().clone(), record, lock);
    run.write()?;
    let tools = SessionToolManager::new(
        Arc::new(manager),
        run.clone(),
        sh.lease.clone(),
        workdir,
        sh.touched.clone(),
    );
    let (ctx_llm, rounds) = counted(llm.clone());
    let deps = checkpoint_deps(sh, &run, &config, ctx_llm, tools);
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
        deps,
        rounds,
        summary_llm: llm,
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
    let cfg = match &new.entry {
        Some(e) => context_config(session_cfg, e),
        None => session_cfg.clone(),
    };
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
            extra: Value::Null,
        },
    );
    let system_prompt = hosted.prompt.system_prompt.clone();
    let config = hosted.config.clone();
    let mut manager = hosted.manager;
    manager.bind_run_dir(&run_id, Some(runs.dir().join(&run_id)));
    let run = RunHandle::new(runs.store().clone(), record, lock);
    run.write()?;
    let tools = SessionToolManager::new(
        Arc::new(manager),
        run.clone(),
        sh.lease.clone(),
        PathBuf::from(&binding.workdir),
        sh.touched.clone(),
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
    request.tool_policy.allow_deferred = calls_sub_contexts(&config);
    let (ctx_llm, rounds) = counted(llm.clone());
    let deps = checkpoint_deps(sh, &run, &config, ctx_llm, tools);
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
        deps,
        rounds,
        summary_llm: llm,
    })
}

/// Resume the live run (§8.6): receipts reconciled by `reconcile_runs`.
pub(super) async fn resume_live_run(
    sh: &Arc<Shared>,
    run: RunHandle,
    snapshot: LLMContextSnapshot,
    env: &SessionEnv,
) -> Result<LiveCtx> {
    let record = run.record();
    let run_id = record.run_id.clone();
    let depth = sh.session.lock().await.state.call_depth();
    let xdeps = xllm_deps_for(sh, env, depth).await?;
    let blocked = |e: String| OpenDanError::blocked(e, Some(&run_id));
    let manager = rebuild_toolset(&record, &xdeps)
        .await
        .map_err(|e| blocked(format!("cannot rebuild the run's tools: {e}")))?;
    let llm = create_run_llm(&record, &xdeps)
        .await
        .map_err(|e| blocked(format!("cannot create the run's provider: {e}")))?;
    let behavior = record.config.loop_model == LoopModel::Behavior;
    let mut snapshot = snapshot;
    // In-flight actions without a persisted result → explicit "interrupted,
    // result unknown", worded by the runtime from what it can read (long-tool
    // TODO §3.2); persisted before any further inference. No process is
    // verified or stopped.
    if !record.inflight.is_empty() {
        let binding = RunBinding {
            run_id: run_id.clone(),
            run_dir: Some(sh.dir.runs().dir().join(&run_id)),
        };
        let mut reasons = std::collections::HashMap::new();
        for action in &record.inflight {
            reasons.insert(
                action.call_id.clone(),
                sh.deps.runtime.describe_interrupted(&binding, action).await,
            );
        }
        materialize_unresolved(&mut snapshot, &record.inflight, behavior, &reasons);
        run.checkpoint_with_results(&snapshot, None)?;
    }
    // Suspended on a sub context call: its result is the tool result of
    // that call (`ResumeFill::ToolResults`); the rest of the batch / step
    // continues afterwards. Any other deferred task has no resolver here.
    let returned = sh.session.lock().await.state.process_result.clone();
    let returned_call = returned
        .as_ref()
        .and_then(|r| r.get("call_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut fill = ResumeFill::ResumeFromMidRun;
    if let Some(Suspension::PendingTool { pending, .. }) = &snapshot.state.suspended {
        let mut results = Vec::new();
        for p in pending {
            if returned_call.as_deref() != Some(p.call.call_id.as_str()) {
                return Err(blocked(format!(
                    "the run waits for task {} of call {}, which this runner cannot supply",
                    p.task_id, p.call.call_id
                )));
            }
            results.push((
                p.call.call_id.clone(),
                sub_result_observation(&p.call.call_id, returned.as_ref().unwrap_or(&Value::Null)),
            ));
        }
        fill = ResumeFill::ToolResults { results };
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
        Arc::new(manager),
        run.clone(),
        sh.lease.clone(),
        workdir,
        sh.touched.clone(),
    );
    let (ctx_llm, rounds) = counted(llm.clone());
    let deps = checkpoint_deps(sh, &run, &record.config, ctx_llm, tools);
    let ctx = if matches!(
        snapshot.state.suspended,
        Some(Suspension::ContextLimit { .. })
    ) {
        // Paused at the context limit (compactions exhausted, or the
        // rewrite did not complete): compact again before running on.
        rewrite_for_limit(sh, &run, snapshot, behavior, &deps, &llm, 1).await?
    } else {
        let filled = matches!(fill, ResumeFill::ToolResults { .. });
        let ctx = LLMContext::resume(snapshot, fill, deps.clone())
            .map_err(|e| blocked(format!("snapshot cannot be resumed: {e}")))?;
        if filled {
            // The result is in the run before anything runs on.
            run.checkpoint_with_results(&ctx.snapshot(), Some(RunStatus::Running))?;
            crate::fault::point("sub_return:after_fill");
        }
        ctx
    };
    if returned_call.is_some() {
        // Delivered (now, or before a crash): the hand-back is consumed.
        let mut s = sh.session.lock().await;
        s.state.process_result = None;
        s.commit_state(&sh.lease)?;
    }
    *sh.interrupt.lock().expect("interrupt") = Some(ctx.interrupt_handle());
    run.set_status(RunStatus::Running, None)?;
    Ok(LiveCtx {
        behavior,
        ctx,
        run,
        ready: true,
        deps,
        rounds,
        summary_llm: llm,
    })
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
pub(super) async fn open_state_live_run(sh: &Arc<Shared>, env: &SessionEnv) -> Result<LiveCtx> {
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
    let mut lc = resume_live_run(sh, run, snapshot, env).await?;
    // Resumed on purpose: the hand-over batch brings the input. A caller
    // back from a tool-triggered sub context got its tool result instead
    // and runs on by itself.
    lc.ready = tool_return;
    Ok(lc)
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
    let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match r.get("status").and_then(Value::as_str).unwrap_or("ok") {
        "failed" => Observation::Error {
            call_id: call_id.to_string(),
            message: format!("sub context `{}` failed: {}", text("behavior"), text("result")),
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

/// Commit one input batch (§8.3): message + receipt in one snapshot →
/// run.json gate → state.json → clear gate → confirm inputs. No inference
/// before the gate is clear. The batch opens a new logical Turn when none
/// is open (D1); otherwise it joins the open one (hand-over, resume,
/// supplementary input). Committing a batch never completes a Turn.
pub(super) async fn commit_input_batch(
    sh: &Arc<Shared>,
    lc: &mut LiveCtx,
    picked: &[InputMessage],
    changes: &Changes,
    text: String,
    hook: &str,
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
    let turn = if opens_turn {
        s.state.turn_seq + 1
    } else {
        s.state.current_turn()
    };
    let mut inputs: Vec<InputRef> = picked.iter().map(|m| m.input_ref()).collect();
    inputs.extend(changes.injected_inputs.iter().cloned());
    let mut extra = std::collections::BTreeMap::new();
    if s.state.internal_continuation.is_some() {
        extra.insert("continuation".to_string(), Value::Bool(true));
    }
    let after_step = lc.ctx.snapshot().state.next_step_index;
    let mut receipt = InputReceipt {
        run_id: run_id.clone(),
        input_seq: applied + 1,
        turn,
        opens_turn,
        hook: Some(hook.to_string()),
        inputs,
        changes: changes.receipts.clone(),
        consumed_only: changes.consumed_only.clone(),
        message_pos: MessagePos::None,
        content: text.clone(),
        bootstrap: !s.state.bootstrap_done,
        after_step,
        extra,
        at_ms: crate::now_ms(),
    };
    let pos = lc.ctx.inject(Injection {
        messages: vec![AiMessage::text(AiRole::User, text)],
        host: None,
    });
    receipt.message_pos = position_of(pos);
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
    let dropped: Vec<WorklogBody> = changes
        .dropped
        .iter()
        .map(|(c, r)| WorklogBody::ChangeDropped {
            change: c.clone(),
            reason: r.clone(),
        })
        .collect();
    s.append_worklog(&sh.lease, dropped)?;
    commit_and_report(sh, &mut s).await?; // ③
    crate::fault::point("input_batch:after_state_commit");
    lc.run.complete_host_commit()?; // ④
    crate::fault::point("input_batch:after_gate_clear");
    confirm_inputs(&sh.sources, &s.state).await; // ⑤
    Ok(())
}
