//! `drive`: advance one session (§8.2) — lease, recovery, inputs, runtime
//! binding, input batches, outcomes, logical Turns, commit order.
//!
//! Terms: an *input batch* is one receipt `(run_id, input_seq)` committed
//! into the context; a *Turn* is the session's logical Input → result,
//! opened by the first batch committed while none is open and closed only
//! here, when an outcome is interpreted as its result or failure
//! (`Next.turn_end`); an *outcome* ends one run segment of the drive loop.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_tool::exec_tracking::{materialize_unresolved, ExecutionRecord, ExecutionRegistrar, HostRunInfo};
use agent_tool::local_llm_context::{
    create_run_llm, hosted_waist_deps, rebuild_toolset, EffectiveConfig, LoopModel,
    RunRecord, RunStatus, XllmTask,
};
use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole, AiUsage};
use llm_context::deps::{Injection, LLMContextDeps, LlmClient};
use llm_context::error::{ErrorSource, LLMComputeError, ProviderFailure};
use llm_context::outcome::{ContextOutput, LLMContextOutcome, ResumeFill};
use llm_context::request::ContextOwnerRef;
use llm_context::state::{LLMContextSnapshot, Suspension};
use llm_context::{LLMContext, LLMContextInterruptHandle, NEXT_BEHAVIOR_END};
use serde_json::{json, Value};

use crate::channel::{InputSource, Inputs};
use crate::error::{OpenDanError, Result};
use crate::lock::{Acquire, Lease};
use crate::protocol::*;
use crate::runtime::{bin_plan_for, bind_or_verify, SessionEnv, SessionEnvCtx};
use crate::session::runs::RunHandle;
use crate::session::{Session, SessionDir};
use crate::state::{run_digest, AgentStateClient};

use super::assembler::InputMaterial;
use super::flush::{run_history_entries, FlushMarks};
use super::history::{build_history, compact_for_limit, maybe_compact, LlmSummarizer, Summarizer};
use super::hook::{check_changes, scope_touching, SessionCheckpointHook};
use super::receipts::{
    apply_receipt, host_meta_of, position_of, receipts_after, snapshot_host_meta,
    validate_receipts, with_host_meta,
};
use super::rounds::{CountingLlm, RoundCounter};
use super::tools::{RunRegistrar, SessionToolManager};
use super::{DriveResult, RunnerDeps, StopWhen};

const WAIT_USER_MSG: &str = "WAIT_USER_MSG";
const FETCH_MAX: usize = 256;
/// Mid-run compactions in a row before a context-limit run is paused.
const MAX_LIMIT_COMPACTIONS: u32 = 3;

/// State shared by the drive loop, the checkpoint hook and the tools.
pub struct Shared {
    pub deps: RunnerDeps,
    pub lease: Arc<Lease>,
    pub session: tokio::sync::Mutex<Session>,
    pub sources: Vec<Arc<dyn InputSource>>,
    pub touched: Arc<Mutex<Vec<Touching>>>,
    pub interrupt: Mutex<Option<LLMContextInterruptHandle>>,
    pub agent_root: Option<PathBuf>,
    pub dir: SessionDir,
    /// `self_improve` lease of a self-improve session.
    pub kind_lease: Mutex<Option<Arc<Lease>>>,
}

impl Shared {
    pub fn agent(&self) -> &dyn AgentStateClient {
        self.deps.agent.as_ref()
    }
}

/// An active llm_context of this drive.
struct LiveCtx {
    ctx: LLMContext,
    run: RunHandle,
    behavior: bool,
    /// Resumed mid-run: may continue without new input.
    ready: bool,
    /// Deps of the context (rebuilding it for a normal behavior switch).
    deps: llm_context::deps::LLMContextDeps,
    /// Rounds made through `deps.llm` not yet recorded.
    rounds: Arc<RoundCounter>,
    /// The run's client without Round counting (history summarization).
    summary_llm: Arc<dyn LlmClient>,
}

/// The run's client for the context: every inference is a counted Round.
fn counted(llm: Arc<dyn LlmClient>) -> (Arc<dyn LlmClient>, Arc<RoundCounter>) {
    let counter = Arc::new(RoundCounter::default());
    (Arc::new(CountingLlm::new(llm, counter.clone())), counter)
}

/// What `handle_context_outcome` decided. Persisted in run.json
/// (`host.extra.finish`) together with a terminal run status, so a finish
/// redone after a crash reaches the same result.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Next {
    run_ended: bool,
    finished: bool,
    outcome: Option<Outcome>,
    waiting: bool,
    /// Stop the drive with this error (state carries it).
    error: Option<Value>,
    kind: String,
    next_behavior: Option<String>,
    answer: Option<String>,
    usage: Option<AiUsage>,
    /// The run was suspended into `process_stack` (not ended, not kept open).
    suspended: bool,
    /// The open Turn ends with this run end (`None`: it continues — a
    /// hand-over, a fork child returning, a resumable suspension).
    turn_end: Option<TurnStatus>,
}

// ---------------------------------------------------------------------------
// inputs
// ---------------------------------------------------------------------------

/// Fetch pending inputs of every source, skipping consumed deliveries and
/// duplicate producer keys (`recent_keys`, except coalescing change keys).
pub async fn fetch_inputs(sh: &Shared) -> Result<Inputs> {
    let state = sh.session.lock().await.state.clone();
    let mut out = Inputs::default();
    for src in &sh.sources {
        let progress = state.source(src.id());
        for m in src.fetch(&progress, FETCH_MAX).await? {
            if m.kind != InputKind::Change && !m.key.is_empty() && state.recent_keys.contains(&m.key)
            {
                // A re-delivery of something already consumed: consume the
                // duplicate silently (it never enters the context twice).
                out.items.push(InputMessage {
                    malformed: Some("duplicate key".into()),
                    ..m
                });
                continue;
            }
            out.items.push(m);
        }
    }
    Ok(out)
}

/// Confirm the committed consumption positions (cumulative ack). Failures are
/// logged; the next drive retries.
pub async fn confirm_inputs(sources: &[Arc<dyn InputSource>], state: &SessionState) {
    for src in sources {
        let p = state.source(src.id());
        if let Err(e) = src.confirm(&p).await {
            log::warn!("confirm input source {}: {e}", src.id());
        }
    }
}

/// Registry report (best effort; `reported_rev` drives the catch-up).
pub async fn report(sh: &Shared, status: SessionStatus) {
    let sid = sh.dir.sid().to_string();
    let rev = status.rev;
    match sh.agent().sessions().report_state(&sh.lease, &sid, status).await {
        Ok(_) => {
            let mut s = sh.session.lock().await;
            if s.state.reported_rev < rev {
                // Recorded with the next commit; not worth a commit by itself.
                s.state.reported_rev = rev;
            }
        }
        Err(e) => log::warn!("report state of {sid}: {e}"),
    }
    sh.deps.notifier.session_changed(&sid, rev).await;
}

async fn commit_and_report(sh: &Shared, s: &mut Session) -> Result<()> {
    s.commit_state(&sh.lease)?;
    let status = s.status(sh.lease.epoch());
    let rev = status.rev;
    let sid = sh.dir.sid().to_string();
    match sh.agent().sessions().report_state(&sh.lease, &sid, status).await {
        Ok(_) => s.state.reported_rev = s.state.reported_rev.max(rev),
        Err(e) => log::warn!("report state of {sid}: {e}"),
    }
    sh.deps.notifier.session_changed(&sid, rev).await;
    Ok(())
}

fn reject(s: &mut Session, m: &InputMessage, reason: &str) -> WorklogBody {
    s.state.source_mut(&m.src).mark(m.index);
    WorklogBody::InputRejected {
        input: m.input_ref(),
        reason: reason.to_string(),
    }
}

/// Apply control / perception / malformed inputs (drive start and every
/// observation boundary). `in_run`: a run is executing.
pub async fn apply_controls(sh: &Shared, inputs: &mut Inputs, in_run: bool) -> Result<()> {
    let malformed = inputs.take_malformed();
    let controls = inputs.take(InputKind::Control);
    let perceptions = inputs.take(InputKind::Perception);
    if malformed.is_empty() && controls.is_empty() && perceptions.is_empty() {
        return Ok(());
    }
    let mut s = sh.session.lock().await;
    let mut bodies = Vec::new();
    for m in &malformed {
        let reason = m.malformed.clone().unwrap_or_default();
        if reason == "duplicate key" {
            s.state.source_mut(&m.src).mark(m.index);
        } else {
            bodies.push(reject(&mut s, m, &reason));
        }
    }
    // Perception inputs: append (idempotent by seq) before consuming.
    if !perceptions.is_empty() {
        let sid = sh.dir.sid().to_string();
        let mut recs = Vec::new();
        let mut seq = s.state.perception_seq;
        for p in &perceptions {
            seq += 1;
            recs.push(PerceptionRecord {
                seq,
                at_ms: p.at_ms.max(1),
                session_id: sid.clone(),
                kind: p
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("observation")
                    .to_string(),
                source: "session".into(),
                tags: p
                    .payload
                    .get("tags")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                objects: p
                    .payload
                    .get("objects")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                summary: p
                    .payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| p.text()),
                payload: p.payload.clone(),
                refs: json!({ "input": p.input_ref().id() }),
            });
        }
        sh.agent().perception().append(&sh.lease, &sid, recs).await?;
        s.state.perception_seq = seq;
        for p in &perceptions {
            s.state.source_mut(&p.src).mark(p.index);
            s.state.recent_keys.push(&p.key);
        }
    }
    let finished = s.state.is_finished();
    for m in &controls {
        let Some(cmd) = m.control() else {
            bodies.push(reject(&mut s, m, "unknown control command"));
            continue;
        };
        match &cmd {
            ControlCommand::Stop { reason } => {
                if finished {
                    bodies.push(reject(&mut s, m, "session already finished"));
                    continue;
                }
                s.state.stop_requested = true;
                bodies.push(WorklogBody::ControlApplied {
                    input: m.input_ref(),
                    command: "stop".into(),
                    detail: json!({ "reason": reason, "from": m.from }),
                });
            }
            ControlCommand::Activity {
                summary,
                touch,
                clear,
            } => {
                if *clear {
                    s.state.activity.touching.clear();
                }
                if let Some(t) = summary {
                    s.state.activity.summary = t.clone();
                }
                for t in touch {
                    s.state.activity.touch(t.clone());
                }
                s.state.activity.heartbeat_ms = crate::now_ms();
                bodies.push(WorklogBody::ControlApplied {
                    input: m.input_ref(),
                    command: "activity".into(),
                    detail: serde_json::to_value(&cmd).unwrap_or(Value::Null),
                });
            }
            ControlCommand::Subscribe { subscription } => {
                s.config.subscriptions.retain(|x| x.id != subscription.id);
                s.config.subscriptions.push(subscription.clone());
                s.write_config(&sh.lease)?;
                bodies.push(WorklogBody::ControlApplied {
                    input: m.input_ref(),
                    command: "subscribe".into(),
                    detail: json!({ "id": subscription.id }),
                });
            }
            ControlCommand::Unsubscribe { id } => {
                s.config.subscriptions.retain(|x| &x.id != id);
                s.state.subscription_cursors.remove(id);
                s.write_config(&sh.lease)?;
                bodies.push(WorklogBody::ControlApplied {
                    input: m.input_ref(),
                    command: "unsubscribe".into(),
                    detail: json!({ "id": id }),
                });
            }
            ControlCommand::Decide { decision, by, note } => {
                if !finished || in_run {
                    // Keep it in the queue; show "waiting for the driver".
                    s.state.pending_decision =
                        Some(json!({ "decision": decision, "by": by, "input": m.input_ref().id() }));
                    continue;
                }
                match apply_decide(sh, &mut s, m, decision, by, note.as_deref()).await {
                    Ok(Some(body)) => bodies.push(body),
                    Ok(None) => continue, // artifact lock busy: retry later
                    Err(OpenDanError::InvalidArgument(reason)) => {
                        bodies.push(reject(&mut s, m, &reason))
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        s.state.source_mut(&m.src).mark(m.index);
        s.state.recent_keys.push(&m.key);
    }
    s.append_worklog(&sh.lease, bodies)?;
    commit_and_report(sh, &mut s).await?;
    confirm_inputs(&sh.sources, &s.state).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// decide (§6.5)
// ---------------------------------------------------------------------------

async fn apply_decide(
    sh: &Shared,
    s: &mut Session,
    m: &InputMessage,
    decision: &str,
    by: &str,
    note: Option<&str>,
) -> Result<Option<WorklogBody>> {
    if s.config.session.kind != SessionKind::Work {
        return Err(OpenDanError::InvalidArgument(
            "decide only applies to work sessions".into(),
        ));
    }
    let valid = match decision {
        "accept" => s.state.acceptance == Acceptance::Pending,
        "discard" => matches!(
            s.state.acceptance,
            Acceptance::Pending | Acceptance::Accepted
        ),
        _ => false,
    };
    if !valid {
        return Err(OpenDanError::InvalidArgument(format!(
            "decide `{decision}` is not valid while acceptance is {:?}",
            s.state.acceptance
        )));
    }
    let sid = s.sid().to_string();
    let mut version: Option<ArtifactVersion> = None;
    let mut head_after: Option<Option<String>> = None;
    if let Some(aid) = s.config.artifact_id.clone() {
        let resource = format!("artifact:{aid}");
        let lease = match sh.agent().locks().acquire(&resource, sh.deps.holder())? {
            Acquire::Acquired(l) => l,
            Acquire::Busy(_) => return Ok(None),
        };
        let ver = format!("v-{sid}");
        match sh.agent().artifacts().version(&aid, &ver).await? {
            Some(_) => {
                let r = sh
                    .agent()
                    .artifacts()
                    .decide(&lease, &aid, &ver, decision)
                    .await?;
                head_after = Some(r.head_after.clone());
                version = Some(r.version);
            }
            None => log::warn!("session {sid} has no version of artifact {aid}"),
        }
        lease.release();
    }
    let mut report = Value::Null;
    if decision == "discard" {
        let unsupported = match &version {
            Some(v) => v.side_effects.clone(),
            None => side_effects_from_worklog(s)?,
        };
        let workspace = match &s.config.workspace {
            None => "none",
            // Workspace rollback is designed separately (Q13): nothing is
            // reverted here; every change is reported as not undone.
            Some(_) => "unsupported",
        };
        let r = DiscardReport {
            reverted: Vec::new(),
            workspace: workspace.into(),
            unsupported,
            head_moved_to: head_after.clone(),
            note: note.map(str::to_string),
        };
        report = serde_json::to_value(&r).unwrap_or(Value::Null);
        s.state.acceptance = Acceptance::Discarded;
        let mut result = s.state.result.clone().unwrap_or_else(|| json!({}));
        result["discard_report"] = report.clone();
        s.state.result = Some(result);
    } else {
        s.state.acceptance = Acceptance::Accepted;
    }
    s.state.pending_decision = None;
    let body = WorklogBody::Decide {
        decision: decision.to_string(),
        by: if by.is_empty() { m.from.clone() } else { by.to_string() },
        report,
    };
    if decision == "discard" {
        // Appended after the commit by the caller's commit; perception after.
        let seq = s.state.perception_seq + 1;
        s.state.perception_seq = seq;
        let rec = PerceptionRecord {
            seq,
            at_ms: crate::now_ms(),
            session_id: sid.clone(),
            kind: "task_discarded".into(),
            source: "session".into(),
            tags: s.state.topic.tags.clone(),
            objects: s
                .config
                .artifact_id
                .iter()
                .map(|a| format!("artifact:{a}"))
                .collect(),
            summary: format!("work session {sid} was discarded"),
            payload: json!({ "session": sid, "artifact_version": version.as_ref().map(|v| v.ver.clone()) }),
            refs: Value::Null,
        };
        if let Err(e) = sh.agent().perception().append(&sh.lease, &sid, vec![rec]).await {
            log::warn!("task_discarded perception of {sid}: {e}");
        }
    }
    Ok(Some(body))
}

/// Side-effect actions of this session, read backwards from the worklog.
fn side_effects_from_worklog(s: &Session) -> Result<Vec<SideEffectRef>> {
    let mut out = Vec::new();
    let mut r = s
        .dir
        .worklog()
        .reverse(s.state.worklog.committed_bytes, 0)?;
    while let Some((_, e)) = r.next_json::<WorklogEntry>()? {
        let calls = match e.body {
            WorklogBody::Step { actions, .. } => actions,
            WorklogBody::AssistantMessage { tool_calls, .. } => tool_calls,
            _ => continue,
        };
        for a in calls {
            if a.effect != "read_only" {
                out.push(SideEffectRef {
                    call_id: a.call_id,
                    tool: a.tool.clone(),
                    note: format!("{} cannot be undone by the session", a.tool),
                });
            }
        }
    }
    out.reverse();
    Ok(out)
}

// ---------------------------------------------------------------------------
// runs: reconcile / create / resume / finish
// ---------------------------------------------------------------------------

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

async fn stop_executions(sh: &Shared, run: &RunHandle) -> Result<()> {
    for exec in run.executions() {
        sh.deps.runtime.reconcile_execution(&exec).await.map_err(|e| match e {
            OpenDanError::RecoveryBlocked(mut b) => {
                b.run_id = Some(run.run_id().to_string());
                OpenDanError::RecoveryBlocked(b)
            }
            other => other,
        })?;
        run.complete_execution(&exec.execution_id)?;
    }
    Ok(())
}

/// Remove an unreferenced run: lock, verify no unconfirmed execution, delete.
async fn remove_if_safe(sh: &Shared, run_id: &str) -> Result<bool> {
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

enum Reconciled {
    None,
    Resume(RunHandle, LLMContextSnapshot),
}

async fn reconcile_runs(sh: &Arc<Shared>) -> Result<Reconciled> {
    let (keep, live_id) = {
        let mut s = sh.session.lock().await;
        s.truncate_uncommitted(&sh.lease)?;
        (
            s.state.referenced_runs(),
            s.state.live_run.as_ref().map(|l| l.run_id.clone()),
        )
    };
    let runs = sh.dir.runs();
    for run_id in runs.list()? {
        if !keep.contains(&run_id) {
            if let Err(e) = remove_if_safe(sh, &run_id).await {
                log::warn!("cleanup of run {run_id}: {e}");
            }
        }
    }
    let Some(run_id) = live_id else {
        return Ok(Reconciled::None);
    };
    let lock = runs
        .try_lock(&run_id)?
        .ok_or_else(|| OpenDanError::RunBusy {
            run_id: run_id.clone(),
        })?;
    let (record, snapshot) = runs.load_checked(&run_id)?;
    let Some(snapshot) = snapshot else {
        return Err(OpenDanError::blocked(
            "the live run has no published snapshot",
            Some(&run_id),
        ));
    };
    let run = RunHandle::new(runs.store().clone(), record.clone(), lock);
    // kill -9 of a runner does not stop its tools.
    stop_executions(sh, &run).await?;
    // Receipts: fill in state from the snapshot, never re-append messages.
    {
        let mut s = sh.session.lock().await;
        let applied = s
            .state
            .live_run
            .as_ref()
            .map(|l| l.applied_input_seq)
            .unwrap_or(0);
        let meta = snapshot_host_meta(&snapshot);
        validate_receipts(&meta, &run_id, applied)?;
        let delta = receipts_after(&meta, applied);
        if !delta.is_empty() {
            for r in &delta {
                apply_receipt(&mut s.state, r)?;
            }
            s.commit_state(&sh.lease)?;
        }
        if let Some(seq) = run.host_commit_pending() {
            let applied = s
                .state
                .live_run
                .as_ref()
                .map(|l| l.applied_input_seq)
                .unwrap_or(0);
            if applied < seq {
                return Err(OpenDanError::blocked(
                    format!("host commit {seq} is pending but state only applied {applied}"),
                    Some(&run_id),
                ));
            }
            run.complete_host_commit()?;
        }
    }
    if record.status.is_terminal() {
        // Finished by xllm, or our finish crashed half-way: redo the finish.
        finish_terminal_record(sh, &run, &record, &snapshot).await?;
        return Ok(Reconciled::None);
    }
    Ok(Reconciled::Resume(run, snapshot))
}

/// Redo the end of a run from its record (xllm finished it, or finish_run
/// crashed before the commit): the decision recorded with the terminal
/// status, else rebuilt from the published snapshot.
async fn finish_terminal_record(
    sh: &Arc<Shared>,
    run: &RunHandle,
    record: &RunRecord,
    snapshot: &LLMContextSnapshot,
) -> Result<()> {
    let behavior = record.config.loop_model == LoopModel::Behavior;
    let recorded = crate::session::runs::finish_info(record)
        .and_then(|v| serde_json::from_value::<Next>(v).ok());
    let next = match recorded {
        Some(n) => n,
        None => derive_next(sh, record, snapshot, behavior).await,
    };
    finish_run(sh, run, snapshot, behavior, next).await
}

/// Decision of a run finished by another executor (xllm), from its record
/// and final snapshot.
async fn derive_next(
    sh: &Arc<Shared>,
    record: &RunRecord,
    snapshot: &LLMContextSnapshot,
    behavior: bool,
) -> Next {
    let (cfg, completed) = {
        let s = sh.session.lock().await;
        (s.config.clone(), s.state.turns_completed)
    };
    match record.status {
        RunStatus::Completed => {
            let st = &snapshot.state;
            let last_step = st.last_step.as_ref().or(st.steps.last());
            let replied =
                has_report(snapshot) || last_step.is_some_and(|s| !s.messages_sent.is_empty());
            let (nb, answer) = if behavior {
                (
                    last_step.and_then(|s| s.next_behavior.clone()),
                    st.last_report
                        .clone()
                        .or_else(|| last_step.and_then(|s| s.self_report.clone())),
                )
            } else {
                (
                    None,
                    st.accumulated
                        .iter()
                        .rev()
                        .find(|m| m.role == AiRole::Assistant)
                        .map(|m| m.text_content()),
                )
            };
            let answer = answer.or_else(|| record.result.as_ref().map(|r| r.raw.clone()));
            let fork_child = is_fork_child(sh, &record.run_id).await;
            let mut next =
                classify_done(&cfg, behavior, nb, answer, replied, fork_child, completed);
            if next.kind == "switch" {
                // The executor ended the run; nothing continues it.
                next.kind = "done".into();
                next.run_ended = true;
                next.next_behavior = None;
                decide_end(&cfg, &mut next, completed + 1);
            }
            next.usage = record.usage.main.clone();
            next
        }
        RunStatus::Failed => Next {
            run_ended: true,
            kind: "error".into(),
            turn_end: Some(TurnStatus::Failed),
            usage: record.usage.main.clone(),
            error: Some(json!({
                "kind": "run_failed",
                "message": record.last_error.as_ref().map(|e| e.message.clone()).unwrap_or_default(),
            })),
            ..Default::default()
        },
        _ => Next {
            run_ended: true,
            kind: "budget".into(),
            turn_end: Some(TurnStatus::BudgetExhausted),
            usage: record.usage.main.clone(),
            error: Some(json!({
                "kind": "limit_reached",
                "message": record.limit_reason.clone().unwrap_or_default(),
            })),
            ..Default::default()
        },
    }
}

fn default_llm_context() -> Value {
    json!({ "tools": { "enabled": true } })
}

async fn xllm_deps_for(sh: &Arc<Shared>, env: &SessionEnv, registrar: Arc<dyn ExecutionRegistrar>) -> agent_tool::local_llm_context::XllmDeps {
    let mut x = sh.deps.xllm.clone();
    x.bash_runner = Some(sh.deps.runtime.bash_runner(env, registrar));
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
    let hook = Arc::new(SessionCheckpointHook::new(sh.clone(), run.clone(), behavior));
    hosted_waist_deps(cfg, llm, Arc::new(tools)).with_checkpoint_hook(hook)
}

/// A fresh llm_context: system + summary + reverse-read worklog (§4.4).
async fn new_run_context(sh: &Arc<Shared>, binding: &Binding, env: &SessionEnv) -> Result<LiveCtx> {
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
    let llm_ctx = if cfg.prompt.llm_context.is_null() {
        default_llm_context()
    } else {
        cfg.prompt.llm_context.clone()
    };
    let hosted = XllmTask::prepare_hosted(
        std::path::Path::new(&binding.workdir),
        &llm_ctx,
        "session_config.json#prompt.llm_context",
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
            env_check: json!({ "workdir": binding.workdir }),
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
    let request = agent_tool::local_llm_context::hosted_request(
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
async fn resume_live_run(
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
            (s.state.current_behavior.clone(), s.state.process_result.clone())
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
async fn run_compacting(sh: &Arc<Shared>, lc: &mut LiveCtx) -> Result<LLMContextOutcome> {
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
async fn open_state_live_run(sh: &Arc<Shared>, env: &SessionEnv) -> Result<LiveCtx> {
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
async fn suspend_run(
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
fn live_from_frame(f: ProcessFrame) -> LiveRun {
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
async fn commit_input_batch(
    sh: &Arc<Shared>,
    lc: &mut LiveCtx,
    picked: &[InputMessage],
    changes: &super::hook::Changes,
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

fn is_retryable_error(e: &LLMComputeError) -> bool {
    match e {
        LLMComputeError::Timeout | LLMComputeError::Cancelled => true,
        LLMComputeError::Provider { failure, .. } => *failure == ProviderFailure::Transient,
        other => other.source() == ErrorSource::Runtime,
    }
}

/// Session end condition after a `Done` that delivers the Turn's result;
/// `completed` counts the completed Turns including this one.
fn decide_end(cfg: &SessionConfig, next: &mut Next, completed: u64) {
    next.turn_end = Some(TurnStatus::Completed);
    match cfg.session.end_condition.kind {
        EndConditionType::LlmDeclaresDone | EndConditionType::OutputSchema => {
            next.finished = true;
            next.outcome = Some(Outcome::Succeeded);
        }
        EndConditionType::MaxTurns => {
            let n = cfg
                .session
                .end_condition
                .detail
                .get("n")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            if completed >= n {
                next.finished = true;
                next.outcome = Some(Outcome::Succeeded);
            } else {
                next.waiting = true;
            }
        }
    }
}

/// The run produced a Self Report (`<report>`).
fn has_report(snapshot: &LLMContextSnapshot) -> bool {
    snapshot
        .state
        .last_report
        .as_deref()
        .is_some_and(|r| !r.trim().is_empty())
}

/// Session-level meaning of a `Done` outcome (also used to rebuild the
/// decision of a run another executor finished). `completed` = Turns
/// completed before this outcome. Hand-overs (switch, fork child return)
/// keep the Turn open; waiting for input completes it only when a reply
/// was delivered (`replied`: a report or a sent message, D2), otherwise the
/// next input joins the same Turn.
fn classify_done(
    cfg: &SessionConfig,
    behavior: bool,
    next_behavior: Option<String>,
    answer: Option<String>,
    replied: bool,
    fork_child: bool,
    completed: u64,
) -> Next {
    let mut next = Next {
        run_ended: true,
        kind: "done".into(),
        answer,
        ..Default::default()
    };
    match next_behavior.as_deref() {
        // A fork child returns to its caller whatever it declares.
        _ if fork_child && next_behavior.as_deref() != Some(WAIT_USER_MSG) => {
            next.kind = "process_done".into();
        }
        Some(WAIT_USER_MSG) => {
            next.kind = "wait".into();
            next.waiting = true;
            if replied {
                next.turn_end = Some(TurnStatus::Completed);
            }
        }
        // `END` (waist) and `done` (xllm: report without actions) are
        // terminal; anything else hands over to that behavior.
        Some(b)
            if behavior
                && !b.eq_ignore_ascii_case(NEXT_BEHAVIOR_END)
                && !b.eq_ignore_ascii_case("done") =>
        {
            next.kind = "switch".into();
            next.next_behavior = Some(b.to_string());
            next.run_ended = false;
        }
        _ => decide_end(cfg, &mut next, completed + 1),
    }
    next
}

async fn is_fork_child(sh: &Shared, run_id: &str) -> bool {
    let s = sh.session.lock().await;
    s.state
        .process_stack
        .last()
        .map(|f| f.mode == ProcessMode::Fork && f.run_id != run_id)
        .unwrap_or(false)
}

/// Interpret one `LLMContext` outcome for the session (run status, Turn
/// end, behavior switch, run end) and commit it. Returning an outcome does
/// not by itself complete the logical Turn: `Next.turn_end` says whether it
/// does.
async fn handle_context_outcome(
    sh: &Arc<Shared>,
    lc: &mut LiveCtx,
    outcome: LLMContextOutcome,
) -> Result<Next> {
    let (cfg, completed, stop) = {
        let s = sh.session.lock().await;
        (
            s.config.clone(),
            s.state.turns_completed,
            s.state.stop_requested,
        )
    };
    let mut next = Next::default();
    let mut snapshot = lc.ctx.snapshot();
    let status;
    match outcome {
        LLMContextOutcome::Done {
            output,
            usage,
            behavior_result,
            response,
            ..
        } => {
            let text = match output {
                ContextOutput::Text { content } => content,
                ContextOutput::Json { content } => content.to_string(),
            };
            let nb = behavior_result.as_ref().and_then(|b| b.next_behavior.clone());
            let report = behavior_result.as_ref().and_then(|b| b.self_report.clone());
            let answer = Some(report.unwrap_or_else(|| {
                if text.is_empty() {
                    response.message.text_content()
                } else {
                    text
                }
            }));
            let replied = has_report(&snapshot)
                || behavior_result
                    .as_ref()
                    .is_some_and(|b| !b.messages_to_send.is_empty());
            let fork_child = is_fork_child(sh, lc.run.run_id()).await;
            next = classify_done(
                &cfg,
                lc.behavior,
                nb,
                answer,
                replied,
                fork_child,
                completed,
            );
            next.usage = Some(usage);
            status = if next.kind == "switch" {
                // The run continues (normal switch) or is suspended into
                // process_stack (fork / independent): never terminal.
                match next
                    .next_behavior
                    .as_deref()
                    .and_then(|b| sh.deps.assembler.process_mode(&cfg, b))
                {
                    None => RunStatus::Running,
                    Some(_) => RunStatus::Paused,
                }
            } else {
                RunStatus::Completed
            };
        }
        LLMContextOutcome::BudgetExhausted { which, usage, .. } => {
            next.usage = Some(usage);
            next.run_ended = true;
            next.kind = "budget".into();
            next.turn_end = Some(TurnStatus::BudgetExhausted);
            status = RunStatus::LimitReached;
            next.error = Some(json!({ "kind": "budget_exhausted", "message": format!("{which:?}") }));
        }
        LLMContextOutcome::Error { error, usage, .. } => {
            next.usage = Some(usage);
            next.kind = "error".into();
            let retry = is_retryable_error(&error);
            next.error = Some(json!({
                "kind": format!("{:?}", error.source()).to_lowercase(),
                "message": error.to_string(),
                "recoverable": retry,
            }));
            if retry {
                // Retryable: the run is kept and the Turn stays open.
                status = RunStatus::Paused;
            } else {
                status = RunStatus::Failed;
                next.run_ended = true;
                next.turn_end = Some(TurnStatus::Failed);
            }
        }
        LLMContextOutcome::Interrupted {
            reason,
            usage,
            snapshot: s,
            ..
        } => {
            next.usage = Some(usage);
            snapshot = s;
            if stop {
                next.kind = "stopped".into();
                next.run_ended = true;
                next.finished = true;
                next.outcome = Some(Outcome::Stopped);
                next.turn_end = Some(TurnStatus::Stopped);
                status = RunStatus::Interrupted;
            } else {
                next.kind = "interrupted".into();
                next.error = Some(json!({ "kind": "interrupted", "message": reason }));
                status = RunStatus::Interrupted;
            }
        }
        LLMContextOutcome::PendingTool { snapshot: s, .. } => {
            snapshot = s;
            next.kind = "pending_tool".into();
            next.waiting = true;
            status = RunStatus::Paused;
        }
        LLMContextOutcome::ContextLimitReached {
            usage, snapshot: s, ..
        } => {
            next.usage = Some(usage);
            snapshot = s;
            next.kind = "context_limit".into();
            next.error = Some(json!({ "kind": "context_limit", "message": "context limit reached" }));
            status = RunStatus::Paused;
        }
    }
    // 1. results and snapshot first; only covered in-flight markers clear. A
    //    run that ends records the decision in the same run.json write.
    if next.run_ended {
        let finish = serde_json::to_value(&next).unwrap_or(Value::Null);
        lc.run.checkpoint_finish(&snapshot, status, finish)?;
    } else {
        lc.run.checkpoint_with_results(&snapshot, Some(status))?;
    }
    // Rounds of this segment: added to run.json (all executors) and to the
    // session statistics (this runner's attempts).
    let rounds = lc.rounds.take();
    if next.usage.is_some() || rounds.attempts > 0 {
        let _ = lc.run.record_usage(next.usage.as_ref(), rounds.attempts);
    }
    if rounds.attempts > 0 {
        let s = sh.session.lock().await;
        let _ = s.update_static(&sh.lease, |st| {
            st.rounds += rounds.attempts;
            st.rounds_failed += rounds.failed;
            st.rounds_interrupted += rounds.interrupted;
        });
    }
    if next.kind == "switch" {
        let b = next.next_behavior.clone().unwrap_or_default();
        match sh.deps.assembler.process_mode(&cfg, &b) {
            None => {
                // Normal switch: same context, same run, new behavior.
                let mut snap = snapshot.clone();
                snap.request.behavior_name = b.clone();
                lc.ctx = LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, lc.deps.clone())
                    .map_err(|e| OpenDanError::Llm(format!("behavior switch: {e}")))?;
                *sh.interrupt.lock().expect("interrupt") = Some(lc.ctx.interrupt_handle());
                let mut s = sh.session.lock().await;
                s.state.current_behavior = Some(b.clone());
                s.state.internal_continuation = Some(b);
                s.state.run_state = RunState::Ready;
                commit_and_report(sh, &mut s).await?;
            }
            Some(mode) => {
                suspend_run(sh, lc, &snapshot, mode, &b).await?;
                next.suspended = true;
            }
        }
        return Ok(next);
    }
    if next.run_ended {
        finish_run(sh, &lc.run, &snapshot, lc.behavior, next.clone()).await?;
        return Ok(next);
    }
    // Run kept (paused / waiting on a tool): state only.
    let mut s = sh.session.lock().await;
    s.state.run_state = if next.waiting {
        RunState::Waiting
    } else {
        RunState::Ready
    };
    if next.waiting {
        s.state.waiting_for = Some(WaitingFor {
            kind: "tool".into(),
            refs: Vec::new(),
            deadline_ms: None,
        });
    }
    s.state.last_error = next.error.clone();
    commit_and_report(sh, &mut s).await?;
    Ok(next)
}

/// End a run (§8.3 `finish_run`; idempotent when redone by reconcile). When
/// `next.turn_end` is set the open Turn is closed in the same commit.
async fn finish_run(
    sh: &Arc<Shared>,
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
    behavior: bool,
    next: Next,
) -> Result<()> {
    // Background processes of the run must be confirmed stopped first.
    stop_executions(sh, run).await?;
    let run_id = run.run_id().to_string();
    let mut s = sh.session.lock().await;
    let sid = s.sid().to_string();
    // The run's unwritten history (after what a suspension already flushed).
    let marks = s
        .state
        .live_run
        .as_ref()
        .filter(|l| l.run_id == run_id)
        .map(FlushMarks::of)
        .unwrap_or_default();
    let turn = s.state.current_turn();
    let (mut bodies, _) = run_history_entries(&run_id, snapshot, behavior, marks, turn);
    // Close the Turn before the report renders the counters.
    let turn_closed = match (next.turn_end, s.state.open_turn.is_some()) {
        (Some(status), true) => {
            s.state.open_turn = None;
            if status == TurnStatus::Completed {
                s.state.turns_completed += 1;
            }
            Some(status)
        }
        _ => None,
    };
    let mut artifact_ref = None;
    if next.finished {
        if let Some(aid) = s.config.artifact_id.clone() {
            // New work continues from the version the user accepted (S-12).
            let base = sh
                .agent()
                .artifacts()
                .head(&aid)
                .await?
                .and_then(|h| h.head)
                .filter(|h| h != &format!("v-{}", s.sid()));
            let version = register_outputs(&s, run, base, &bodies)?;
            sh.agent()
                .artifacts()
                .register_version(&sh.lease, &aid, s.config.workspace.clone(), version.clone())
                .await?;
            artifact_ref = Some(json!({ "aid": aid, "ver": version.ver }));
        }
        let answer = next.answer.clone().unwrap_or_default();
        s.write_report(&sh.lease, &render_report(&s, &answer, &next))?;
    }
    // Flush the run's unwritten history into the worklog.
    bodies.push(WorklogBody::Outcome {
        run_id: run_id.clone(),
        turn,
        kind: next.kind.clone(),
        next_behavior: next.next_behavior.clone(),
        report: next.answer.clone().map(|a| a.chars().take(2000).collect()),
    });
    if let Some(status) = turn_closed {
        bodies.push(WorklogBody::TurnEnded {
            run_id: run_id.clone(),
            turn,
            status,
            at_ms: crate::now_ms(),
        });
    }
    s.append_worklog(&sh.lease, bodies)?;
    crate::fault::point("finish_run:after_flush");
    let prev = s.state.last_run.clone();
    let child_behavior = s.state.current_behavior.clone();
    s.state.live_run = None;
    s.state.last_run = Some(run_id.clone());
    s.state.one_line_status = one_line(&next);
    s.state.last_error = next.error.clone();
    s.state.waiting_for = None;
    s.state.activity.touching.clear();
    if next.finished {
        s.state.run_state = RunState::Finished;
        s.state.process_stack.clear();
        s.state.outcome = next.outcome;
        s.state.stop_requested = false;
        s.state.activity = Activity::default();
        if s.config.session.kind == SessionKind::Work {
            s.state.acceptance = Acceptance::Pending;
        }
        let mut result = json!({
            "answer": next.answer.clone().map(|a| a.chars().take(2000).collect::<String>()),
            "answer_ref": REPORT_FILE,
        });
        if let Some(a) = artifact_ref {
            result["artifact_ref"] = a;
        }
        s.state.result = Some(result);
    } else if next.waiting {
        s.state.run_state = RunState::Waiting;
        s.state.waiting_for = Some(WaitingFor {
            kind: "input".into(),
            refs: Vec::new(),
            deadline_ms: None,
        });
    } else {
        s.state.run_state = RunState::Ready;
    }
    if next.kind == "process_done" {
        // Fork child ended: its caller becomes live again in this same
        // commit, with the child's result for its hand-over batch (same
        // Turn).
        if let Some(f) = s.state.process_stack.pop() {
            let entry = f.entry.clone();
            s.state.live_run = Some(live_from_frame(f));
            s.state.process_entry = Some(entry.clone());
            s.state.current_behavior = Some(entry.clone());
            s.state.internal_continuation = Some(entry);
            s.state.process_result = Some(json!({
                "behavior": child_behavior.unwrap_or_default(),
                "result": next.answer.clone().unwrap_or_default(),
                // Keep action / step ids unique after the return.
                "next_action_id": snapshot.state.next_action_id,
                "next_step_index": snapshot.state.next_step_index,
            }));
        }
    }
    let digest_seq = s.state.perception_seq + 1;
    s.state.perception_seq = digest_seq + u64::from(next.finished);
    s.commit_state(&sh.lease)?; // commit point
    crate::fault::point("finish_run:after_commit");
    // Cleanup of the replaced run.
    if let Some(p) = prev.filter(|p| p != &run_id && !s.state.references(p)) {
        if let Err(e) = remove_if_safe(sh, &p).await {
            log::warn!("cleanup of previous run {p}: {e}");
        }
    }
    let _ = run.prune(sh.deps.options.keep_snapshots);
    // Compaction by ratio.
    let cfg = s.config.clone();
    let budget = cfg
        .prompt
        .history_budget_tokens
        .unwrap_or(sh.deps.options.history_budget_tokens);
    let ratio = cfg.prompt.compact_ratio.unwrap_or(sh.deps.options.compact_ratio);
    let summarizer = sh.deps.summarizer.clone();
    if let Some(sm) = summarizer {
        if let Err(e) = maybe_compact(&mut s, &sh.lease, Some(sm.as_ref()), budget, ratio).await {
            log::warn!("compaction of {sid}: {e}");
        }
    }
    let usage = next.usage.clone();
    let turns = s.state.turns_completed;
    let _ = s.update_static(&sh.lease, |st| {
        st.runs += 1;
        st.turns = turns;
        if let Some(u) = &usage {
            st.input_tokens += u.input_tokens.unwrap_or(0);
            st.output_tokens += u.output_tokens.unwrap_or(0);
            st.total_tokens += u.total_tokens.unwrap_or(0);
            st.cost += u.reported_cost.unwrap_or(0.0);
        }
    });
    // After the commit: registry, perception, notification (catch-up later).
    let mut recs = vec![run_digest(
        &sid,
        digest_seq,
        &run_id,
        turn,
        turn_closed,
        &s.state.topic,
        s.config.artifact_id.iter().map(|a| format!("artifact:{a}")).collect(),
        &s.state.one_line_status,
        s.state.worklog.committed_seq,
    )];
    if next.finished {
        recs.push(PerceptionRecord {
            seq: digest_seq + 1,
            at_ms: crate::now_ms(),
            session_id: sid.clone(),
            kind: "task_outcome".into(),
            source: "session".into(),
            tags: s.state.topic.tags.clone(),
            objects: Vec::new(),
            summary: s.state.one_line_status.clone(),
            payload: json!({ "outcome": next.outcome, "acceptance": s.state.acceptance }),
            refs: Value::Null,
        });
    }
    if let Err(e) = sh.agent().perception().append(&sh.lease, &sid, recs).await {
        log::warn!("perception of {sid}: {e}");
    }
    let consolidate = next.finished
        && next.outcome == Some(Outcome::Succeeded)
        && s.config.session.kind == SessionKind::SelfImprove;
    let window = perception_window(&s.config);
    let status = s.status(sh.lease.epoch());
    drop(s);
    report(sh, status).await;
    if consolidate {
        let kind_lease = sh.kind_lease.lock().expect("kind lease").clone();
        if let (Some(w), Some(l)) = (window, kind_lease) {
            let base = sh.agent().perception().cursor().await?;
            let upto = w.advanced(&base);
            let batch = crate::state::ConsolidationBatch {
                session_id: sid.clone(),
                summary: next.answer.clone().unwrap_or_default().chars().take(500).collect(),
                memory_ops: 0,
                notebook_ops: 0,
            };
            sh.agent()
                .cognition()
                .commit_consolidation(&l, &batch, &upto)
                .await?;
        }
    }
    Ok(())
}

fn one_line(next: &Next) -> String {
    let base = match (next.finished, next.kind.as_str()) {
        (true, "stopped") => "stopped".to_string(),
        (true, _) => "finished".to_string(),
        (false, "wait") => "waiting for input".to_string(),
        (false, "switch") => format!(
            "switching to {}",
            next.next_behavior.clone().unwrap_or_default()
        ),
        (false, k) => k.to_string(),
    };
    match &next.answer {
        Some(a) if !a.trim().is_empty() => {
            let first: String = a.trim().lines().next().unwrap_or("").chars().take(160).collect();
            format!("{base}: {first}")
        }
        _ => base,
    }
}

fn render_report(s: &Session, answer: &str, next: &Next) -> String {
    format!(
        "# Report — {}\n\n- session: `{}`\n- outcome: {}\n- turns: {}\n\n{}\n",
        s.config
            .session
            .objective
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect::<String>(),
        s.sid(),
        next.outcome
            .map(|o| format!("{o:?}").to_lowercase())
            .unwrap_or_else(|| next.kind.clone()),
        s.state.turns_completed,
        answer.trim()
    )
}

/// The session's contribution to its artifact (§6.5).
fn register_outputs(
    s: &Session,
    _run: &RunHandle,
    base: Option<String>,
    unflushed: &[WorklogBody],
) -> Result<ArtifactVersion> {
    let mut outputs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(s.dir.path()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with('.') || n == README_FILE {
                continue;
            }
            outputs.push(n);
        }
    }
    outputs.sort();
    let mut side_effects = side_effects_from_worklog(s)?;
    // The run being finished is not in the worklog yet.
    for b in unflushed {
        let calls = match b {
            WorklogBody::Step { actions, .. } => actions,
            WorklogBody::AssistantMessage { tool_calls, .. } => tool_calls,
            _ => continue,
        };
        for a in calls {
            if a.effect != "read_only" {
                side_effects.push(SideEffectRef {
                    call_id: a.call_id.clone(),
                    tool: a.tool.clone(),
                    note: format!("{} cannot be undone by the session", a.tool),
                });
            }
        }
    }
    Ok(ArtifactVersion {
        ver: format!("v-{}", s.sid()),
        session: s.sid().to_string(),
        base,
        state: VersionState::Produced,
        outputs,
        workspace_ref: Value::Null,
        side_effects,
        updated_at_ms: crate::now_ms(),
    })
}

// ---------------------------------------------------------------------------
// drive
// ---------------------------------------------------------------------------

/// Advance a session until `until` (§8.2).
pub async fn drive(sd: &SessionDir, deps: &RunnerDeps, until: StopWhen) -> DriveResult {
    let lease = match sd.acquire(deps.holder()) {
        Ok(Acquire::Acquired(l)) => Arc::new(l),
        Ok(Acquire::Busy(info)) => {
            return DriveResult::Busy {
                holder: info.and_then(|i| serde_json::to_value(i).ok()),
            }
        }
        Err(OpenDanError::NotDriver { driver, .. }) => return DriveResult::NotDriver { driver },
        Err(e) => {
            return DriveResult::Error {
                rev: 0,
                error: e.to_json(),
            }
        }
    };
    let result = drive_locked(sd, deps, until, lease.clone()).await;
    match Arc::try_unwrap(lease) {
        Ok(l) => l.release(),
        Err(arc) => drop(arc),
    }
    result
}

async fn drive_locked(
    sd: &SessionDir,
    deps: &RunnerDeps,
    until: StopWhen,
    lease: Arc<Lease>,
) -> DriveResult {
    let mut session = match sd.load(&lease) {
        Ok(s) => s,
        Err(OpenDanError::RecoveryBlocked(b)) => return DriveResult::RecoveryBlocked(b),
        Err(e) => {
            return DriveResult::Error {
                rev: 0,
                error: e.to_json(),
            }
        }
    };
    session.set_writer(WriterInfo {
        runner_id: deps.runner_id.clone(),
        principal: deps.who.clone(),
        host: Some(deps.runtime.host_id().to_string()),
        pid: std::process::id(),
        lock_epoch: lease.epoch(),
    });
    // Only registered sessions are advanced.
    match deps.agent.sessions().lookup(sd.sid()).await {
        Ok(Some(e)) => {
            let a = std::path::Path::new(&e.location)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&e.location));
            let b = sd.path().canonicalize().unwrap_or_else(|_| sd.path().to_path_buf());
            if a != b {
                return DriveResult::Unregistered;
            }
        }
        _ => return DriveResult::Unregistered,
    }
    let sources = match deps.inputs.open(&session.config).await {
        Ok(s) => s,
        Err(e) => {
            return DriveResult::Error {
                rev: session.state.rev,
                error: e.to_json(),
            }
        }
    };
    let sh = Arc::new(Shared {
        deps: deps.clone(),
        lease: lease.clone(),
        session: tokio::sync::Mutex::new(session),
        sources,
        touched: Arc::new(Mutex::new(Vec::new())),
        interrupt: Mutex::new(None),
        agent_root: deps.agent.agent_root().map(|p| p.to_path_buf()),
        dir: sd.clone(),
        kind_lease: Mutex::new(None),
    });
    let r = drive_inner(&sh, until).await;
    let rev = sh.session.lock().await.state.rev;
    match r {
        Ok(res) => res,
        Err(OpenDanError::RecoveryBlocked(b)) => {
            record_error(&sh, &OpenDanError::RecoveryBlocked(b.clone())).await;
            DriveResult::RecoveryBlocked(b)
        }
        Err(OpenDanError::RunBusy { run_id }) => DriveResult::RunBusy { run_id },
        Err(OpenDanError::LeaseLost(_)) => DriveResult::LeaseLost,
        Err(e) => {
            record_error(&sh, &e).await;
            DriveResult::Error {
                rev,
                error: e.to_json(),
            }
        }
    }
}

/// Record `last_error` (keeps live_run, runs and consumption untouched).
async fn record_error(sh: &Arc<Shared>, e: &OpenDanError) {
    if !sh.lease.held() {
        return;
    }
    let mut s = sh.session.lock().await;
    // Only the error is recorded: drop half-applied in-memory changes.
    if let Err(err) = s.reset_to_committed(&sh.lease) {
        log::warn!("cannot reload {} to record an error: {err}", sh.dir.sid());
        return;
    }
    s.state.last_error = Some(e.to_json());
    if let Err(err) = commit_and_report(sh, &mut s).await {
        log::warn!("cannot record error of {}: {err}", sh.dir.sid());
    }
}

async fn catch_up(sh: &Arc<Shared>) -> Result<()> {
    let sid = sh.dir.sid().to_string();
    let (status, perception_seq, last_run, turn, topic, summary, wseq, reported) = {
        let s = sh.session.lock().await;
        (
            s.status(sh.lease.epoch()),
            s.state.perception_seq,
            s.state.last_run.clone().unwrap_or_default(),
            s.state.current_turn(),
            s.state.topic.clone(),
            s.state.one_line_status.clone(),
            s.state.worklog.committed_seq,
            s.state.reported_rev,
        )
    };
    if reported < status.rev {
        report(sh, status).await;
    }
    let last = sh.agent().perception().last_seq(&sid).await?;
    if last < perception_seq {
        let mut recs = Vec::new();
        for seq in last + 1..=perception_seq {
            recs.push(run_digest(
                &sid,
                seq,
                &last_run,
                turn,
                None,
                &topic,
                Vec::new(),
                &summary,
                wseq,
            ));
        }
        sh.agent().perception().append(&sh.lease, &sid, recs).await?;
    }
    Ok(())
}

async fn reject_leftovers(sh: &Arc<Shared>, inputs: &mut Inputs) -> Result<()> {
    let left: Vec<InputMessage> = std::mem::take(&mut inputs.items);
    if left.is_empty() {
        return Ok(());
    }
    let mut s = sh.session.lock().await;
    let mut bodies = Vec::new();
    for m in &left {
        if m.kind == InputKind::Control {
            continue; // decide inputs wait for a valid state
        }
        bodies.push(reject(&mut s, m, "session is finished"));
    }
    if bodies.is_empty() {
        return Ok(());
    }
    s.append_worklog(&sh.lease, bodies)?;
    commit_and_report(sh, &mut s).await?;
    confirm_inputs(&sh.sources, &s.state).await;
    Ok(())
}

fn finished_result(s: &Session) -> DriveResult {
    DriveResult::Finished {
        rev: s.state.rev,
        outcome: s.state.outcome,
        acceptance: s.state.acceptance,
    }
}

async fn stop_session(sh: &Arc<Shared>, live: Option<LiveCtx>, env: &SessionEnv) -> Result<()> {
    let next = Next {
        run_ended: true,
        finished: true,
        outcome: Some(Outcome::Stopped),
        kind: "stopped".into(),
        turn_end: Some(TurnStatus::Stopped),
        ..Default::default()
    };
    // A run state still references (e.g. a parent just resumed after its fork
    // child) ends through the normal finish path.
    let live = match live {
        Some(l) => Some(l),
        None if sh.session.lock().await.state.live_run.is_some() => {
            Some(open_state_live_run(sh, env).await?)
        }
        None => None,
    };
    match live {
        Some(lc) => {
            let snap = lc.ctx.snapshot();
            lc.run.checkpoint_with_results(&snap, Some(RunStatus::Interrupted))?;
            finish_run(sh, &lc.run, &snap, lc.behavior, next).await
        }
        None => {
            let mut s = sh.session.lock().await;
            s.state.run_state = RunState::Finished;
            s.state.outcome = Some(Outcome::Stopped);
            s.state.stop_requested = false;
            s.state.process_stack.clear();
            s.state.internal_continuation = None;
            s.state.activity = Activity::default();
            if s.config.session.kind == SessionKind::Work {
                s.state.acceptance = Acceptance::Pending;
            }
            s.state.one_line_status = "stopped".into();
            let turn = s.state.current_turn();
            let mut bodies = vec![WorklogBody::Outcome {
                run_id: String::new(),
                turn,
                kind: "stopped".into(),
                next_behavior: None,
                report: None,
            }];
            if s.state.open_turn.take().is_some() {
                bodies.push(WorklogBody::TurnEnded {
                    run_id: String::new(),
                    turn,
                    status: TurnStatus::Stopped,
                    at_ms: crate::now_ms(),
                });
            }
            s.append_worklog(&sh.lease, bodies)?;
            commit_and_report(sh, &mut s).await?;
            Ok(())
        }
    }
}

/// `extensions.opendan.perception_window` of a self-improve session.
pub fn perception_window(cfg: &SessionConfig) -> Option<crate::state::Backlog> {
    cfg.extensions
        .get("opendan")
        .and_then(|o| o.get("perception_window"))
        .and_then(|w| serde_json::from_value(w.clone()).ok())
}

async fn perception_window_records(sh: &Shared, cfg: &SessionConfig) -> Result<Vec<PerceptionRecord>> {
    let mut out = Vec::new();
    if let Some(w) = perception_window(cfg) {
        for item in &w.items {
            out.extend(sh.agent().perception().read(item).await?);
        }
    }
    Ok(out)
}

async fn drive_inner(sh: &Arc<Shared>, until: StopWhen) -> Result<DriveResult> {
    // Kind leases: one consolidation at a time for the whole agent.
    let kind = sh.session.lock().await.config.session.kind;
    let _kind_lease = if kind == SessionKind::SelfImprove {
        match sh.agent().locks().acquire("self_improve", sh.deps.holder())? {
            Acquire::Acquired(l) => Some(Arc::new(l)),
            Acquire::Busy(info) => {
                return Ok(DriveResult::Busy {
                    holder: info.and_then(|i| serde_json::to_value(i).ok()),
                })
            }
        }
    } else {
        None
    };
    *sh.kind_lease.lock().expect("kind lease") = _kind_lease.clone();
    // Recovery: runs, receipts, executions — before reading any new input.
    let mut live: Option<LiveCtx> = None;
    let reconciled = reconcile_runs(sh).await?;
    {
        let s = sh.session.lock().await;
        confirm_inputs(&sh.sources, &s.state).await;
    }
    if let Err(e) = catch_up(sh).await {
        log::warn!("catch-up of {}: {e}", sh.dir.sid());
    }
    let mut inputs = fetch_inputs(sh).await?;
    apply_controls(sh, &mut inputs, false).await?;
    {
        let s = sh.session.lock().await;
        if s.state.is_finished() {
            drop(s);
            reject_leftovers(sh, &mut inputs).await?;
            return Ok(finished_result(&*sh.session.lock().await));
        }
    }
    // Runtime binding happens before any inference (Q3).
    let (binding, env) = {
        let cfg = sh.session.lock().await.config.clone();
        let layers: Vec<PathBuf> = sh
            .deps
            .runtime
            .descriptor()
            .path_layers
            .iter()
            .map(PathBuf::from)
            .collect();
        let plan = bin_plan_for(&cfg, sh.agent_root.as_deref(), sh.deps.session_cli.clone(), &layers)?;
        let bound = bind_or_verify(
            &sh.dir,
            &sh.lease,
            sh.deps.runtime.as_ref(),
            &cfg,
            sh.agent_root.as_deref(),
            &sh.deps.app_tools,
            &plan,
            &sh.deps.runner_id,
        )
        .await;
        let binding = match bound {
            Ok(b) => b,
            Err(e @ (OpenDanError::Bind(_) | OpenDanError::RuntimeMismatch { .. })) => {
                let mut s = sh.session.lock().await;
                s.state.last_error = Some(e.to_json());
                commit_and_report(sh, &mut s).await?;
                return Ok(DriveResult::BindFailed { error: e.to_json() });
            }
            Err(e) => return Err(e),
        };
        let ctx = {
            let s = sh.session.lock().await;
            SessionEnvCtx {
                session_id: s.sid().to_string(),
                session_dir: sh.dir.path().to_path_buf(),
                agent_did: s.config.session.agent_did.clone(),
                agent_root: sh.agent_root.clone(),
                input_queue: s.config.channels.kmsg().map(|(_, q, _)| q.to_string()),
                trace_id: s.sid().to_string(),
                extra_env: s
                    .config
                    .runtime
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            }
        };
        let env = match sh.deps.runtime.open_session_env(&binding, &ctx).await {
            Ok(e) => e,
            Err(e) => {
                let mut s = sh.session.lock().await;
                s.state.last_error = Some(e.to_json());
                commit_and_report(sh, &mut s).await?;
                return Ok(DriveResult::BindFailed { error: e.to_json() });
            }
        };
        (binding, env)
    };
    if let Reconciled::Resume(run, snapshot) = reconciled {
        live = Some(resume_live_run(sh, run, snapshot, &env).await?);
    }
    let started = Instant::now();
    let mut outcomes_handled = 0u64;
    loop {
        sh.lease.check()?;
        if sh.session.lock().await.state.stop_requested {
            stop_session(sh, live.take(), &env).await?;
            return Ok(finished_result(&*sh.session.lock().await));
        }
        if let StopWhen::MaxOutcomes { n } = until {
            if outcomes_handled >= n {
                let s = sh.session.lock().await;
                return Ok(DriveResult::OutcomesHandled {
                    rev: s.state.rev,
                    run_state: s.state.run_state,
                });
            }
        }
        let (cfg, state) = {
            let s = sh.session.lock().await;
            (s.config.clone(), s.state.clone())
        };
        // Pull policy: msg / event make an input batch (opening a Turn, or
        // joining the open one); changes ride along.
        let mut picked = inputs.take(InputKind::Msg);
        picked.extend(inputs.take(InputKind::Event));
        picked.sort_by_key(|m| (m.src.clone(), m.index));
        if cfg.session.input_policy == InputPolicy::None && !picked.is_empty() {
            let mut s = sh.session.lock().await;
            let bodies: Vec<WorklogBody> = picked
                .iter()
                .map(|m| reject(&mut s, m, "session input_policy is none"))
                .collect();
            s.append_worklog(&sh.lease, bodies)?;
            commit_and_report(sh, &mut s).await?;
            confirm_inputs(&sh.sources, &s.state).await;
            picked.clear();
        }
        let change_inputs = inputs.take(InputKind::Change);
        let me = sh.agent().sessions().lookup(sh.dir.sid()).await?;
        let changes = check_changes(
            sh.agent(),
            &cfg,
            &state,
            me.as_ref(),
            &change_inputs,
            sh.deps.options.change_budget,
            sh.deps.options.active_sessions_limit,
            false,
        )
        .await?;
        let mut changes = changes;
        let hook = if !state.bootstrap_done {
            "on_init"
        } else if state.internal_continuation.is_some() {
            "on_behavior_switch"
        } else {
            "on_wakeup"
        };
        let hints = if sh.deps.options.load_hints && (!state.bootstrap_done || !picked.is_empty()) {
            sh.agent()
                .cognition()
                .recall_hints(&crate::state::RecallQuery {
                    tags: state.topic.tags.clone(),
                    max_hints: 5,
                })
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let active = sh
            .agent()
            .activity()
            .active(me.as_ref(), sh.deps.options.active_sessions_limit)
            .await
            .unwrap_or_default();
        // The batch message renders the active list: the agent has seen
        // this view.
        changes.receipts.push(ChangeReceipt {
            id: format!("{}@input", super::hook::ACTIVE_CURSOR),
            subscription: super::hook::ACTIVE_CURSOR.to_string(),
            cursor: super::hook::active_view(&active),
        });
        let material = InputMaterial {
            hook: hook.to_string(),
            inputs: picked.clone(),
            changes: changes.items.clone(),
            hints,
            active,
            runtime_status: sh.deps.runtime.status().await,
            now_ms: crate::now_ms(),
            perceptions: if !state.bootstrap_done && cfg.session.kind == SessionKind::SelfImprove {
                perception_window_records(sh, &cfg).await?
            } else {
                Vec::new()
            },
        };
        // Only msg / event inputs (active), bootstrap and a behavior
        // hand-over make an input batch; semi changes ride along (S-15,
        // A-07).
        let triggered = !picked.is_empty()
            || !state.bootstrap_done
            || state.internal_continuation.is_some();
        let mut msg = if triggered {
            sh.deps.assembler.render_input(&cfg, &state, &material).await?
        } else {
            None
        };
        if msg.is_none() && state.internal_continuation.is_some() {
            msg = Some(format!(
                "<session_input hook=\"on_behavior_switch\">Continue with behavior `{}`.</session_input>",
                state.internal_continuation.clone().unwrap_or_default()
            ));
        }
        let resumable = live.as_ref().map(|l| l.ready).unwrap_or(false);
        if msg.is_none() && !resumable {
            // Nothing to infer on. Coalesced-only changes still get consumed.
            if !changes.consumed_only.is_empty() {
                let mut s = sh.session.lock().await;
                for i in &changes.consumed_only {
                    s.state.source_mut(&i.src).mark(i.index);
                }
                let dropped: Vec<WorklogBody> = changes
                    .dropped
                    .iter()
                    .filter(|(c, _)| changes.consumed_only.iter().any(|i| &i.id() == c))
                    .map(|(c, r)| WorklogBody::ChangeDropped {
                        change: c.clone(),
                        reason: r.clone(),
                    })
                    .collect();
                s.append_worklog(&sh.lease, dropped)?;
                commit_and_report(sh, &mut s).await?;
                confirm_inputs(&sh.sources, &s.state).await;
            }
            {
                let mut s = sh.session.lock().await;
                if s.state.bootstrap_done
                    && !s.state.is_finished()
                    && s.state.run_state != RunState::Waiting
                    && s.state.last_error.is_none()
                {
                    s.state.run_state = RunState::Waiting;
                    s.state.waiting_for = Some(WaitingFor {
                        kind: "input".into(),
                        refs: Vec::new(),
                        deadline_ms: None,
                    });
                    commit_and_report(sh, &mut s).await?;
                }
            }
            let (rev, rs) = {
                let s = sh.session.lock().await;
                (s.state.rev, s.state.run_state)
            };
            match until {
                StopWhen::Idle => return Ok(DriveResult::Idle { rev, run_state: rs }),
                StopWhen::MaxOutcomes { .. } => {
                    return Ok(DriveResult::OutcomesHandled { rev, run_state: rs })
                }
                StopWhen::Finished => {
                    if sh.session.lock().await.state.last_error.is_some() {
                        let err = sh.session.lock().await.state.last_error.clone();
                        return Ok(DriveResult::Error {
                            rev,
                            error: err.unwrap_or(Value::Null),
                        });
                    }
                    if started.elapsed() >= sh.deps.options.max_wait {
                        return Ok(DriveResult::Idle { rev, run_state: rs });
                    }
                    let wake = cfg.channels.wake_event.clone();
                    sh.deps
                        .waker
                        .wait(wake.as_deref(), sh.deps.options.poll_interval)
                        .await;
                    inputs = fetch_inputs(sh).await?;
                    apply_controls(sh, &mut inputs, false).await?;
                    if sh.session.lock().await.state.is_finished() {
                        reject_leftovers(sh, &mut inputs).await?;
                        return Ok(finished_result(&*sh.session.lock().await));
                    }
                    continue;
                }
            }
        }
        let mut lc = match live.take() {
            Some(l) => l,
            None if state.live_run.is_some() => open_state_live_run(sh, &env).await?,
            None => new_run_context(sh, &binding, &env).await?,
        };
        if let Some(text) = msg {
            commit_input_batch(sh, &mut lc, &picked, &changes, text, hook).await?;
        }
        lc.ready = false;
        let outcome = run_compacting(sh, &mut lc).await?;
        let next = handle_context_outcome(sh, &mut lc, outcome).await?;
        outcomes_handled += 1;
        if !next.run_ended && !next.suspended {
            live = Some(lc);
        }
        let (rev, rs) = {
            let s = sh.session.lock().await;
            (s.state.rev, s.state.run_state)
        };
        if next.finished {
            return Ok(finished_result(&*sh.session.lock().await));
        }
        if let Some(err) = next.error.clone() {
            return Ok(DriveResult::Error { rev, error: err });
        }
        if next.waiting && until == StopWhen::Idle {
            return Ok(DriveResult::Idle { rev, run_state: rs });
        }
        inputs = fetch_inputs(sh).await?;
        apply_controls(sh, &mut inputs, false).await?;
        let _ = Duration::from_secs(0);
    }
}
