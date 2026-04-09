use crate::analysis::AnalysisReport;
use thiserror::Error;

pub type WorkflowResult<T> = Result<T, WorkflowError>;

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("workflow analysis failed")]
    Analysis(AnalysisReport),
    #[error("invalid reference `{0}`")]
    InvalidReference(String),
    #[error("node `{0}` not found")]
    NodeNotFound(String),
    #[error("node `{0}` is not supported by the current orchestrator")]
    UnsupportedNode(String),
    #[error("node `{0}` is not waiting for human input")]
    NodeNotWaitingHuman(String),
    #[error("node `{0}` is not skippable")]
    NodeNotSkippable(String),
    #[error("missing pending thunk for node `{0}`")]
    MissingPendingThunk(String),
    #[error("thunk `{0}` does not belong to this run")]
    ThunkRunMismatch(String),
    #[error("failed to resolve reference `{0}`")]
    ReferenceResolution(String),
    #[error("human action `{action}` is invalid for node `{node_id}`")]
    InvalidHumanAction { node_id: String, action: String },
    #[error("rollback blocked by completed non-idempotent node `{0}`")]
    RollbackBlocked(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("object store error: {0}")]
    ObjectStore(String),
    #[error("dispatcher error: {0}")]
    Dispatcher(String),
    #[error("task tracker error: {0}")]
    TaskTracker(String),
}
