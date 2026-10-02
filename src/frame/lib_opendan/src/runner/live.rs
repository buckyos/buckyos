//! The live run of a drive: creating a fresh `LLMContext` for a run (§4.4),
//! resuming the live run (§8.6), the mid-run context-limit rewrite (X7),
//! suspending a process into `process_stack`, committing an input batch
//! (§8.3) and stopping / removing a run's executions.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_tool::exec_tracking::{
    materialize_unresolved, ExecutionRecord, ExecutionRegistrar, HostRunInfo,
};
use agent_tool::xllm::{
    create_run_llm, hosted_waist_deps, rebuild_toolset, EffectiveConfig, LoopModel, RunStatus,
    XllmTask,
};
use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole};
use llm_context::deps::{Injection, LLMContextDeps, LlmClient};
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
use super::tools::{RunRegistrar, SessionToolManager};

/// Mid-run compactions in a row before a context-limit run is paused.
const MAX_LIMIT_COMPACTIONS: u32 = 3;

/// Late-bound registrar (the bash runner exists before the run handle).
struct LateRegistrar {
    run: Mutex<Option<Arc<RunRegistrar>>>,
}

impl LateRegistrar {
    fn get(&self) -> Option<Arc<RunRegistrar>> {
        self.run.lock().expect("registrar").clone()
    }
}

#[async_trait]
impl ExecutionRegistrar for LateRegistrar {
    async fn register(&self, rec: &ExecutionRecord) -> std::result::Result<(), String> {
        match self.get() {
            Some(r) => r.register(rec).await,
            None => Err("run not ready".into()),
        }
    }

    async fn completed(&self, execution_id: &str) {
        if let Some(r) = self.get() {
            r.completed(execution_id).await;
        }
    }
}

pub(super) async fn stop_executions(sh: &Shared, run: &RunHandle) -> Result<()> {
    for exec in run.executions() {
        sh.deps
            .runtime
            .reconcile_execution(&exec)
            .await
            .map_err(|e| match e {
                agent_tool::xllm::XllmError::RecoveryBlocked(reason) => {
                    OpenDanError::blocked(reason, Some(run.run_id()))
                }
                other => OpenDanError::Llm(other.to_string()),
            })?;
        run.complete_execution(&exec.execution_id)?;
    }
    Ok(())
}

/// Remove an unreferenced run: lock, verify no unconfirmed execution, delete.
pub(super) async fn remove_if_safe(sh: &Shared, run_id: &str) -> Result<bool> {
    let runs = sh.dir.runs();
    let Some(lock) = runs.try_lock(run_id)? else {
        return Ok(false); // someone executes it (xllm?)
    };
    let record = match runs.record(run_id) {
        Ok(r) => r,
        Err(e) => {
            if runs.dir().join(run_id).join("run.json").exists() {
                // Unreadable: its executions cannot be checked — keep it.
                log::warn!("keeping unreferenced run {run_id}: {e}");
                return Ok(false);
            }
            // No record: never got past creation.
            runs.remove_locked(run_id, &lock)?;
            return Ok(true);
        }
    };
    for exec in &record.executions {
        if let Err(e) = sh.deps.runtime.reconcile_execution(exec).await {
            log::warn!("keeping run {run_id}: {e}");
            return Ok(false);
        }
    }
    runs.remove_locked(run_id, &lock)?;
    Ok(true)
}

fn default_llm_context() -> Value {
    json!({ "tools": { "enabled": true } })
}

async fn xllm_deps_for(
    sh: &Arc<Shared>,
    env: &SessionEnv,
    registrar: Arc<dyn ExecutionRegistrar>,
) -> agent_tool::xllm::XllmDeps {
    let mut x = sh.deps.xllm.clone();
    x.runtime = Some(sh.deps.runtime.clone());
    x.execution_registrar = Some(registrar);
    x.runtime_env = env.env.iter().cloned().collect();
    x.runtime_path_prefix = env.path_layers.clone();
    x.skip_workdir_lock = true;
    x
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

/// A fresh llm_context: system + summary + reverse-read worklog (§4.4).
pub(super) async fn new_run_context(
    sh: &Arc<Shared>,
    binding: &Binding,
    env: &SessionEnv,
) -> Result<LiveCtx> {
    let fork_parent = {
        let s = sh.session.lock().await;
        s.state
            .process_stack
            .last()
            .filter(|f| f.mode == ProcessMode::Fork && s.state.live_run.is_none())
            .map(|f| f.run_id.clone())
    };
    let mut lc = new_run_context_plain(sh, binding, env).await?;
    if let (Some(parent), true) = (fork_parent, lc.behavior) {
        // Fork child: a new run inheriting the parent process's steps; its
        // control flow must end into the caller.
        let (_, parent_snap) = sh.dir.runs().load_checked(&parent)?;
        let parent_snap = parent_snap.ok_or_else(|| {
            OpenDanError::blocked("fork parent run has no snapshot", Some(&parent))
        })?;
        let mut snap = lc.ctx.snapshot();
        let mut steps = parent_snap.state.steps.clone();
        steps.extend(parent_snap.state.last_step.clone());
        snap.state.steps = steps;
        snap.state.last_step = None;
        snap.state.history_summaries = parent_snap.state.history_summaries.clone();
        snap.state.next_step_index = parent_snap.state.next_step_index;
        snap.state.next_action_id = parent_snap.state.next_action_id;
        // A fork child returns to its caller: jump targets it emits are
        // ignored by `classify_done` (the waist's `forbid_next_behavior`
        // would also scrub xllm's terminal `done` marker).
        let mut meta = snapshot_host_meta(&snap);
        meta.inherited_below = parent_snap.state.next_step_index;
        snap.state.host = Some(with_host_meta(snap.state.host.as_ref(), &meta));
        lc.ctx = LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, lc.deps.clone())
            .map_err(|e| OpenDanError::Llm(format!("fork child: {e}")))?;
        *sh.interrupt.lock().expect("interrupt") = Some(lc.ctx.interrupt_handle());
    }
    Ok(lc)
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

async fn new_run_context_plain(
    sh: &Arc<Shared>,
    binding: &Binding,
    env: &SessionEnv,
) -> Result<LiveCtx> {
    let (cfg, sid) = {
        let s = sh.session.lock().await;
        (s.config.clone(), s.sid().to_string())
    };
    let system = sh
        .deps
        .assembler
        .system_text(&cfg, sh.agent_root.as_deref())
        .await?;
    let registrar = Arc::new(LateRegistrar {
        run: Mutex::new(None),
    });
    let xdeps = xllm_deps_for(sh, env, registrar.clone()).await;
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
    let history = {
        let mut s = sh.session.lock().await;
        build_history(&mut s, &sh.lease, Some(summarizer.as_ref()), budget)
            .await?
            .0
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
    manager.set_run_id(&run_id);
    let run = RunHandle::new(runs.store().clone(), record, lock);
    run.write()?;
    *registrar.run.lock().expect("registrar") = Some(Arc::new(RunRegistrar::new(run.clone())));
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
    let behavior_name = {
        let s = sh.session.lock().await;
        s.state
            .current_behavior
            .clone()
            .or_else(|| cfg.prompt.behavior.clone())
            .unwrap_or_default()
    };
    let request = agent_tool::xllm::hosted_request(
        &config,
        ContextOwnerRef::Agent {
            session_id: sid.clone(),
        },
        &run_id,
        &cfg.session.objective,
        &behavior_name,
        input.clone(),
    );
    let (ctx_llm, rounds) = counted(llm.clone());
    let deps = checkpoint_deps(sh, &run, &config, ctx_llm, tools);
    let mut ctx = LLMContext::new(request, deps.clone());
    let meta = HostMeta {
        session_id: sid,
        base_input_len: input.len() as u64,
        process_entry: {
            let s = sh.session.lock().await;
            s.state.process_entry.clone()
        },
        inherited_below: 0,
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

/// Resume the live run (§8.6): executions already confirmed stopped and
/// receipts reconciled by `reconcile_runs`.
pub(super) async fn resume_live_run(
    sh: &Arc<Shared>,
    run: RunHandle,
    snapshot: LLMContextSnapshot,
    env: &SessionEnv,
) -> Result<LiveCtx> {
    let record = run.record();
    let run_id = record.run_id.clone();
    let registrar: Arc<dyn ExecutionRegistrar> = Arc::new(RunRegistrar::new(run.clone()));
    let xdeps = xllm_deps_for(sh, env, registrar).await;
    let blocked = |e: String| OpenDanError::blocked(e, Some(&run_id));
    let manager = rebuild_toolset(&record, &xdeps)
        .await
        .map_err(|e| blocked(format!("cannot rebuild the run's tools: {e}")))?;
    let llm = create_run_llm(&record, &xdeps)
        .await
        .map_err(|e| blocked(format!("cannot create the run's provider: {e}")))?;
    let behavior = record.config.loop_model == LoopModel::Behavior;
    let mut snapshot = snapshot;
    // In-flight actions without a persisted result → explicit "unknown";
    // persisted before any further inference.
    if !record.inflight.is_empty() {
        materialize_unresolved(&mut snapshot, &record.inflight, behavior);
        run.checkpoint_with_results(&snapshot, None)?;
    }
    if matches!(
        snapshot.state.suspended,
        Some(Suspension::PendingTool { .. })
    ) {
        return Err(blocked(
            "the run waits for deferred tool results, which this runner cannot supply".into(),
        ));
    }
    if behavior {
        let (current, returned) = {
            let s = sh.session.lock().await;
            (
                s.state.current_behavior.clone(),
                s.state.process_result.clone(),
            )
        };
        // A normal switch changes the behavior of the same run; the last
        // published snapshot may predate it.
        if let Some(b) = current {
            snapshot.request.behavior_name = b;
        }
        // Back from a fork child: continue its action / step numbering.
        if let Some(r) = returned {
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
        LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps.clone())
            .map_err(|e| blocked(format!("snapshot cannot be resumed: {e}")))?
    };
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

/// Open the run `state.live_run` points to (a process resumed after a fork
/// child returned, or an independent process re-entered).
pub(super) async fn open_state_live_run(sh: &Arc<Shared>, env: &SessionEnv) -> Result<LiveCtx> {
    let (run_id, behavior_name) = {
        let s = sh.session.lock().await;
        let run_id = s
            .state
            .live_run
            .as_ref()
            .map(|l| l.run_id.clone())
            .ok_or_else(|| OpenDanError::Other("no live run to open".into()))?;
        (run_id, s.state.current_behavior.clone())
    };
    let runs = sh.dir.runs();
    let lock = runs
        .try_lock(&run_id)?
        .ok_or_else(|| OpenDanError::RunBusy {
            run_id: run_id.clone(),
        })?;
    let (record, snapshot) = runs.load_checked(&run_id)?;
    let mut snapshot = snapshot
        .ok_or_else(|| OpenDanError::blocked("suspended run has no snapshot", Some(&run_id)))?;
    let run = RunHandle::new(runs.store().clone(), record, lock);
    stop_executions(sh, &run).await?;
    if let Some(b) = behavior_name {
        snapshot.request.behavior_name = b;
    }
    let mut lc = resume_live_run(sh, run, snapshot, env).await?;
    lc.ready = false; // resumed on purpose: the hand-over batch brings the input
    Ok(lc)
}

/// Suspend the current process run into `process_stack` (§4.4): flush its
/// history so far (worklog keeps time order), keep the run directory.
pub(super) async fn suspend_run(
    sh: &Arc<Shared>,
    lc: &LiveCtx,
    snapshot: &LLMContextSnapshot,
    mode: ProcessMode,
    next_behavior: &str,
) -> Result<()> {
    stop_executions(sh, &lc.run).await?;
    let run_id = lc.run.run_id().to_string();
    let mut s = sh.session.lock().await;
    let live = s
        .state
        .live_run
        .clone()
        .filter(|l| l.run_id == run_id)
        .ok_or_else(|| OpenDanError::Other(format!("run {run_id} is not the live run")))?;
    let turn = s.state.current_turn();
    let (mut bodies, marks) =
        run_history_entries(&run_id, snapshot, lc.behavior, FlushMarks::of(&live), turn);
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
        .unwrap_or_else(|| "main".to_string());
    s.state.process_stack.push(ProcessFrame {
        entry,
        mode,
        run_id: run_id.clone(),
        turns: live.turns,
        flushed_message_count: marks.messages,
        flushed_step_index: marks.step_index,
        flushed_input_seq: marks.input_seq,
        flushed_epoch: marks.epoch,
        applied_input_seq: live.applied_input_seq,
    });
    s.state.live_run = None;
    s.state.process_entry = Some(next_behavior.to_string());
    s.state.current_behavior = Some(next_behavior.to_string());
    s.state.internal_continuation = Some(next_behavior.to_string());
    s.state.run_state = RunState::Ready;
    // Independent: re-enter the target's own suspended run when it exists.
    if mode == ProcessMode::Independent {
        if let Some(pos) = s
            .state
            .process_stack
            .iter()
            .position(|f| f.entry == next_behavior && f.mode == ProcessMode::Independent)
        {
            let f = s.state.process_stack.remove(pos);
            s.state.live_run = Some(live_from_frame(f));
        }
    }
    lc.run.set_status(RunStatus::Paused, None)?;
    commit_and_report(sh, &mut s).await
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
        process_entry: Some(f.entry),
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
