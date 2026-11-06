use crate::error::WorkflowResult;
use async_trait::async_trait;
use buckyos_api::{ThunkExecution, ThunkObject};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct ScheduledThunk {
    pub execution: ThunkExecution,
    pub thunk: ThunkObject,
}

/// Delivery seam between the orchestrator and whatever executes thunks
/// (`LocalThunkRunner` in P0, Scheduler / Node Runner later). Everything is
/// correlated by `execution.execution_id`; a thunk id may be executing in
/// several runs at once.
#[async_trait]
pub trait ThunkDispatcher: Send + Sync {
    /// Admits the execution; must return without waiting for a slot or for
    /// the computation.
    async fn schedule_thunk(
        &self,
        execution: &ThunkExecution,
        thunk: &ThunkObject,
    ) -> WorkflowResult<()>;

    async fn cancel_thunk(&self, _execution_id: &str) -> WorkflowResult<()> {
        Ok(())
    }
}

/// Bookkeeping dispatcher for tests: records what was scheduled and cancelled.
#[derive(Debug, Default, Clone)]
pub struct InMemoryThunkDispatcher {
    scheduled: Arc<Mutex<Vec<ScheduledThunk>>>,
    cancelled: Arc<Mutex<Vec<String>>>,
}

impl InMemoryThunkDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn scheduled(&self) -> Vec<ScheduledThunk> {
        self.scheduled.lock().await.clone()
    }

    pub async fn cancelled(&self) -> Vec<String> {
        self.cancelled.lock().await.clone()
    }
}

#[async_trait]
impl ThunkDispatcher for InMemoryThunkDispatcher {
    async fn schedule_thunk(
        &self,
        execution: &ThunkExecution,
        thunk: &ThunkObject,
    ) -> WorkflowResult<()> {
        self.scheduled.lock().await.push(ScheduledThunk {
            execution: execution.clone(),
            thunk: thunk.clone(),
        });
        Ok(())
    }

    async fn cancel_thunk(&self, execution_id: &str) -> WorkflowResult<()> {
        self.cancelled.lock().await.push(execution_id.to_string());
        Ok(())
    }
}
