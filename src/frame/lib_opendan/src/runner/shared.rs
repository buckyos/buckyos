//! State shared by the drive loop, the checkpoint hook and the tools
//! ([`Shared`]), the live `LLMContext` of a drive ([`LiveCtx`]) and the
//! registry report every commit goes through.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use llm_context::deps::{LLMContextDeps, LlmClient};
use llm_context::state::LLMContextSnapshot;
use llm_context::{LLMContext, LLMContextInterruptHandle, RunningTaskResolver};

use crate::channel::InputSource;
use crate::error::Result;
use crate::lock::Lease;
use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::session::{Session, SessionDir};
use crate::state::AgentStateClient;

use super::rounds::{CountingLlm, RoundCounter, UsageLog};
use super::RunnerDeps;

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
    pub workspace_lease: Mutex<Option<Arc<Lease>>>,
    pub workspace_failure: Mutex<Option<String>>,
    /// Task resolver of the run opened last in this drive: answers for the
    /// background tasks the session still follows after that run ended.
    pub tasks: Mutex<Option<Arc<dyn RunningTaskResolver>>>,
    /// The Turn this drive closed last (`StopWhen::TurnClosed`).
    pub turn_closed: Mutex<Option<ClosedTurn>>,
    /// Tool called last in this drive (the activity shown on the Turn's task).
    pub current_tool: Arc<Mutex<Option<String>>>,
    /// What the Turn's task was told last by this drive.
    pub task_status: Mutex<Option<(String, super::turn_task::TurnTaskStatus)>>,
    /// Serializes hand-overs of the outbox.
    pub flush: tokio::sync::Mutex<()>,
}

/// A Turn closed by a commit of this drive.
#[derive(Debug, Clone)]
pub struct ClosedTurn {
    pub turn: u64,
    pub status: TurnStatus,
    pub answer: Option<String>,
}

impl Shared {
    pub fn agent(&self) -> &dyn AgentStateClient {
        self.deps.agent.as_ref()
    }
}

/// An active llm_context of this drive.
pub(super) struct LiveCtx {
    pub(super) ctx: LLMContext,
    pub(super) run: RunHandle,
    pub(super) behavior: bool,
    /// Resumed mid-run: may continue without new input.
    pub(super) ready: bool,
    /// Suspended calls were just answered: the tool batch / step they
    /// belong to continues first, no input batch can be placed before it.
    pub(super) filled: bool,
    /// Deps of the context (rebuilding it for a normal behavior switch).
    pub(super) deps: LLMContextDeps,
    /// Rounds made through `deps.llm` not yet recorded.
    pub(super) rounds: Arc<RoundCounter>,
    /// The run's client without Round counting (history summarization).
    pub(super) summary_llm: Arc<dyn LlmClient>,
    /// The run's view on running tasks (in-process + host task managers).
    pub(super) resolver: Arc<dyn RunningTaskResolver>,
}

/// A run suspended on tool calls whose tasks are still running
/// (`PendingTool`): it has no live `LLMContext`; the driver waits for the
/// tasks outside the context and resumes it with their results.
pub(super) struct WaitingRun {
    pub(super) run: RunHandle,
    pub(super) snapshot: LLMContextSnapshot,
    pub(super) behavior: bool,
    pub(super) deps: LLMContextDeps,
    pub(super) rounds: Arc<RoundCounter>,
    pub(super) summary_llm: Arc<dyn LlmClient>,
    pub(super) resolver: Arc<dyn RunningTaskResolver>,
}

impl WaitingRun {
    pub(super) fn of(lc: LiveCtx) -> Self {
        Self {
            snapshot: lc.ctx.snapshot(),
            run: lc.run,
            behavior: lc.behavior,
            deps: lc.deps,
            rounds: lc.rounds,
            summary_llm: lc.summary_llm,
            resolver: lc.resolver,
        }
    }

    /// Task ids the suspended calls wait for.
    pub(super) fn task_ids(&self) -> Vec<String> {
        self.snapshot
            .state
            .pending_calls()
            .iter()
            .map(|p| p.task_id.clone())
            .collect()
    }

    /// Earliest `until_ms` of the suspended calls.
    pub(super) fn deadline_ms(&self) -> Option<u64> {
        self.snapshot
            .state
            .pending_calls()
            .iter()
            .filter_map(|p| p.until_ms)
            .min()
    }
}

/// What opening a run gave: a context to run, or a run still waiting for
/// its tasks.
pub(super) enum Opened {
    Ctx(LiveCtx),
    Waiting(WaitingRun),
}

/// The run's client for the context: every inference is a counted Round
/// whose usage goes to the session's `usage.jsonl`.
pub(super) fn counted(
    sh: &Shared,
    run_id: &str,
    llm: Arc<dyn LlmClient>,
) -> (Arc<dyn LlmClient>, Arc<RoundCounter>) {
    let counter = Arc::new(RoundCounter::default());
    let usage = UsageLog {
        path: sh.dir.file(crate::protocol::USAGE_FILE),
        run_id: run_id.to_string(),
    };
    (
        Arc::new(CountingLlm::new(llm, counter.clone()).with_usage(usage)),
        counter,
    )
}

/// Registry report (best effort; `reported_rev` drives the catch-up).
pub async fn report(sh: &Shared, status: SessionStatus) {
    super::turn_task::sync_turn_task(sh).await;
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

pub(super) async fn commit_and_report(sh: &Shared, s: &mut Session) -> Result<()> {
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
