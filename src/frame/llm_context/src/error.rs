//! Error type produced by the LLMContext loop.
//!
//! The variants here describe *what kind* of failure occurred. Classification
//! into `Recoverable` vs `Fatal` (see `request::ErrorClass`) is a separate
//! concern handled by the loop based on `ErrorPolicy`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Error, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LLMComputeError {
    #[error("llm timeout")]
    Timeout,

    #[error("llm cancelled")]
    Cancelled,

    /// Provider-side failure surfaced after the adapter's own retry/fallback
    /// chain has given up.
    #[error("llm provider failed: {0}")]
    Provider(String),

    /// LLM response did not satisfy the declared `OutputSpec`
    /// (e.g. JSON parse failure / schema mismatch / empty payload).
    #[error("llm output parse failed: {0}")]
    OutputParse(String),

    /// PolicyEngine rejected a tool call.
    #[error("policy rejected: {0}")]
    PolicyRejected(String),

    /// A specific tool call failed during execution.
    #[error("tool `{tool}` failed: {message}")]
    ToolFailed {
        tool: String,
        call_id: String,
        message: String,
    },

    /// Snapshot deserialization / state corruption.
    #[error("snapshot corrupted: {0}")]
    SnapshotCorrupted(String),

    /// Internal / programming error.
    #[error("internal: {0}")]
    Internal(String),
}
