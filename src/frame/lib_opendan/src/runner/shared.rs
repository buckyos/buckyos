//! State shared by the drive loop, the checkpoint hook and the tools
//! ([`Shared`]), the live `LLMContext` of a drive ([`LiveCtx`]) and the
//! registry report every commit goes through.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use llm_context::deps::{LLMContextDeps, LlmClient};
use agent_tool::task_model::RunningTaskResolver;
use llm_context::{LLMContext, LLMContextInterruptHandle};

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
    /// The Agent-level consolidation lease of a Goal Session whose Goal has
    /// the `memory_consolidation` capability (held for the whole drive).
    pub kind_lease: Mutex<Option<Arc<Lease>>>,
    /// What the session's Goal allows it (read once per drive).
    pub goal_caps: Mutex<Option<super::consolidation::GoalCaps>>,
    /// The agent's Memory component (opened on first use).
    pub memory: Mutex<Option<Arc<crate::memory::Memory>>>,
    pub workspace_lease: Mutex<Option<Arc<Lease>>>,
    pub workspace_failure: Mutex<Option<String>>,
    /// Task resolver of the run opened last in this drive: answers for the
    /// background tasks the session still follows after that run ended.
    pub tasks: Mutex<Option<Arc<dyn RunningTaskResolver>>>,
    /// The Turn this drive closed last (`StopWhen::TurnClosed`).
    pub turn_closed: Mutex<Option<ClosedTurn>>,
    /// Tool called last in this drive (the activity shown on the Turn's task).
    pub current_tool: Arc<Mutex<Option<String>>>,
    /// The intent that call stated, if any (shown with it).
    pub current_intent: Mutex<Option<String>>,
    /// `one_line_status` changed after the last commit (the next boundary
    /// commits it).
    pub line_pending: std::sync::atomic::AtomicBool,
    /// Result hints shown once per session and dedup keys of this drive
    /// (the tool sets' `HintLedger`).
    pub hints: Arc<super::hints::SessionHintLedger>,
    /// What the Turn's task was told last by this drive.
    pub task_status: Mutex<Option<(String, super::turn_task::TurnTaskStatus)>>,
    /// Serializes hand-overs of the outbox.
    pub flush: tokio::sync::Mutex<()>,
    /// The driving process's stop was applied to a Goal Session's
    /// Activation (once per drive).
    pub external_stop_applied: std::sync::atomic::AtomicBool,
    /// Functions that run a sub context (`call_behavior` and the behaviors'
    /// `[tool]` names) offered to the runs of this drive.
    pub behavior_functions: Mutex<std::collections::BTreeSet<String>>,
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
    /// Resumed with a tool batch / behavior step in progress: it continues
    /// first, no input batch can be placed before it.
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
    match sh.agent().sessions().report_state(&sh.lease, &sid, status.clone()).await {
        Ok(_) => {
            let mut s = sh.session.lock().await;
            if s.state.reported_rev < rev {
                // Recorded with the next commit; not worth a commit by itself.
                s.state.reported_rev = rev;
            }
        }
        Err(e) => log::warn!("report state of {sid}: {e}"),
    }
    sh.deps.notifier.session_changed(&sid, &status).await;
}

pub(super) async fn commit_and_report(sh: &Shared, s: &mut Session) -> Result<()> {
    sh.hints.store(&mut s.state);
    s.commit_state(&sh.lease)?;
    let status = s.status(sh.lease.epoch());
    let rev = status.rev;
    let sid = sh.dir.sid().to_string();
    match sh.agent().sessions().report_state(&sh.lease, &sid, status.clone()).await {
        Ok(_) => s.state.reported_rev = s.state.reported_rev.max(rev),
        Err(e) => log::warn!("report state of {sid}: {e}"),
    }
    sh.deps.notifier.session_changed(&sid, &status).await;
    Ok(())
}
