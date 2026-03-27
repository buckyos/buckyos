use crate::error::WorkflowResult;
use async_trait::async_trait;
use buckyos_api::ThunkObject;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct ScheduledThunk {
    pub thunk_obj_id: String,
    pub thunk: ThunkObject,
}

#[async_trait]
pub trait ThunkDispatcher: Send + Sync {
    async fn schedule_thunk(&self, thunk_obj_id: &str, thunk: &ThunkObject) -> WorkflowResult<()>;
    async fn cancel_thunk(&self, _thunk_obj_id: &str) -> WorkflowResult<()> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryThunkDispatcher {
    scheduled: Arc<Mutex<Vec<ScheduledThunk>>>,
}

impl InMemoryThunkDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn scheduled(&self) -> Vec<ScheduledThunk> {
        self.scheduled.lock().await.clone()
    }
}

#[async_trait]
impl ThunkDispatcher for InMemoryThunkDispatcher {
    async fn schedule_thunk(&self, thunk_obj_id: &str, thunk: &ThunkObject) -> WorkflowResult<()> {
        self.scheduled.lock().await.push(ScheduledThunk {
            thunk_obj_id: thunk_obj_id.to_string(),
            thunk: thunk.clone(),
        });
        Ok(())
    }
}
