//! Background run driver (TODO T1c): RPC handlers only enqueue work; this
//! component ticks runs off the request path, consumes execution receipts
//! from the runner and finalizes terminal runs (ensure slot, subscriptions).

use crate::state::{DefinitionStore, RunHandle, RunStore};
use crate::subscriptions::RunSubscriptionManager;
use crate::{EventEnvelope, RunStatus, ServiceOrchestrator, WorkflowError, WorkflowResult};
use buckyos_api::ThunkExecutionResult;
use log::{debug, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};

pub struct RunDriver {
    definitions: Arc<DefinitionStore>,
    runs: Arc<RunStore>,
    orchestrator: Arc<ServiceOrchestrator>,
    subscriptions: Option<Arc<RunSubscriptionManager>>,
    changed: Notify,
}

impl RunDriver {
    pub fn new(
        definitions: Arc<DefinitionStore>,
        runs: Arc<RunStore>,
        orchestrator: Arc<ServiceOrchestrator>,
        subscriptions: Option<Arc<RunSubscriptionManager>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            definitions,
            runs,
            orchestrator,
            subscriptions,
            changed: Notify::new(),
        })
    }

    pub fn orchestrator(&self) -> &Arc<ServiceOrchestrator> {
        &self.orchestrator
    }

    /// Ticks the run in the background; the caller returns to its client
    /// immediately.
    pub fn spawn_tick(self: &Arc<Self>, run_id: String) {
        let me = self.clone();
        tokio::spawn(async move {
            if let Err(err) = me.tick_now(&run_id).await {
                warn!("workflow driver: tick run {} failed: {}", run_id, err);
            }
        });
    }

    /// Ticks the run under its lock and finalizes it when terminal.
    pub async fn tick_now(&self, run_id: &str) -> WorkflowResult<Vec<EventEnvelope>> {
        let (handle, definition) = self.lookup(run_id).await?;
        let events = {
            let mut record = handle.state.lock().await;
            if record.run.is_terminal() {
                return Ok(Vec::new());
            }
            let events = self
                .orchestrator
                .tick(&definition.compiled, &mut record.run)
                .await?;
            record.append_events(&events);
            events
        };
        self.finalize_if_terminal(&handle).await;
        Ok(events)
    }

    /// Applies one receipt: stale / duplicate / late receipts are logged and
    /// dropped; accepted ones are followed by a tick so successors start.
    pub async fn apply_result(&self, result: ThunkExecutionResult) -> WorkflowResult<()> {
        let run_id = result
            .execution_id
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string();
        let (handle, definition) = match self.lookup(&run_id).await {
            Ok(pair) => pair,
            Err(err) => {
                debug!(
                    "workflow driver: receipt for unknown run execution_id={} err={}",
                    result.execution_id, err
                );
                return Ok(());
            }
        };
        {
            let mut record = handle.state.lock().await;
            let execution_id = result.execution_id.clone();
            match self
                .orchestrator
                .handle_thunk_result(&definition.compiled, &mut record.run, result)
                .await
            {
                Ok(events) => record.append_events(&events),
                Err(WorkflowError::StaleThunkResult(id)) => {
                    debug!("workflow driver: stale receipt ignored execution_id={}", id);
                    return Ok(());
                }
                Err(err) => {
                    warn!(
                        "workflow driver: receipt failed execution_id={} err={}",
                        execution_id, err
                    );
                    return Err(err);
                }
            }
            if !record.run.is_terminal() {
                match self
                    .orchestrator
                    .tick(&definition.compiled, &mut record.run)
                    .await
                {
                    Ok(events) => record.append_events(&events),
                    Err(err) => warn!("workflow driver: tick after receipt failed: {}", err),
                }
            }
        }
        self.finalize_if_terminal(&handle).await;
        Ok(())
    }

    /// Consumes runner receipts until the channel closes.
    pub fn start_result_loop(self: &Arc<Self>, mut receiver: mpsc::Receiver<ThunkExecutionResult>) {
        let me = self.clone();
        tokio::spawn(async move {
            while let Some(result) = receiver.recv().await {
                if let Err(err) = me.apply_result(result).await {
                    warn!("workflow driver: apply receipt failed: {}", err);
                }
            }
        });
    }

    async fn finalize_if_terminal(&self, handle: &Arc<RunHandle>) {
        let (terminal, run_id, ensure_key) = {
            let mut record = handle.state.lock().await;
            let terminal = record.run.is_terminal();
            let key = if terminal { record.ensure_key.take() } else { None };
            (terminal, record.run.run_id.clone(), key)
        };
        if !terminal {
            return;
        }
        if let Some(key) = ensure_key {
            self.runs.release_ensure(&run_id, &key).await;
        }
        if let Some(subs) = self.subscriptions.as_ref() {
            subs.unwatch_run(&run_id).await;
        }
        self.changed.notify_waiters();
    }

    /// Waits until the run is terminal or `timeout` elapses; returns the last
    /// observed status.
    pub async fn wait_terminal(&self, run_id: &str, timeout: Duration) -> Option<RunStatus> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let handle = self.runs.get(run_id).await?;
            let status = handle.state.lock().await.run.status;
            if matches!(
                status,
                RunStatus::Completed
                    | RunStatus::Failed
                    | RunStatus::Aborted
                    | RunStatus::BudgetExhausted
                    | RunStatus::WaitingHuman
            ) {
                return Some(status);
            }
            if tokio::time::Instant::now() >= deadline {
                return Some(status);
            }
            let _ = tokio::time::timeout(Duration::from_millis(20), self.changed.notified()).await;
        }
    }

    async fn lookup(
        &self,
        run_id: &str,
    ) -> WorkflowResult<(Arc<RunHandle>, Arc<crate::state::DefinitionRecord>)> {
        let handle = self
            .runs
            .get(run_id)
            .await
            .ok_or_else(|| WorkflowError::NodeNotFound(format!("run {run_id}")))?;
        let definition = self
            .definitions
            .get_by_id(&handle.workflow_id)
            .await
            .ok_or_else(|| WorkflowError::NodeNotFound(format!("workflow {}", handle.workflow_id)))?;
        Ok((handle, definition))
    }
}
