//! State shared by the drive loop, the checkpoint hook and the tools
//! ([`Shared`]), the live `LLMContext` of a drive ([`LiveCtx`]) and the
//! registry report every commit goes through.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use llm_context::deps::{LLMContextDeps, LlmClient};
use llm_context::{LLMContext, LLMContextInterruptHandle};

use crate::channel::InputSource;
use crate::error::Result;
use crate::lock::Lease;
use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::session::{Session, SessionDir};
use crate::state::AgentStateClient;

use super::rounds::{CountingLlm, RoundCounter};
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
    /// Deps of the context (rebuilding it for a normal behavior switch).
    pub(super) deps: LLMContextDeps,
    /// Rounds made through `deps.llm` not yet recorded.
    pub(super) rounds: Arc<RoundCounter>,
    /// The run's client without Round counting (history summarization).
    pub(super) summary_llm: Arc<dyn LlmClient>,
}

/// The run's client for the context: every inference is a counted Round.
pub(super) fn counted(llm: Arc<dyn LlmClient>) -> (Arc<dyn LlmClient>, Arc<RoundCounter>) {
    let counter = Arc::new(RoundCounter::default());
    (Arc::new(CountingLlm::new(llm, counter.clone())), counter)
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
