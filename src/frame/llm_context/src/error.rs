//! Error type produced by the LLMContext loop.
//!
//! The variants here describe *what kind* of failure occurred and *who* can
//! recover from it. Classification into `Recoverable` (LLM self-correction)
//! vs `Fatal` (run ends) is exhaustive — see [`LLMComputeError::llm_correctable`]
//! — and never falls back to a default branch.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// How a provider failure should be treated by the layer above the waist.
/// The waist never retries; the adapter decides the class from provider /
/// transport error codes and the scheduler decides whether to run again.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFailure {
    /// Network / rate-limit / temporary unavailability: the adapter's own
    /// bounded tolerance is exhausted, a later attempt may succeed.
    Transient,
    /// Auth / model-not-found / invalid configuration: re-running the same
    /// request cannot succeed until configuration changes.
    Permanent,
    /// The adapter could not tell. Must not be treated as safe-to-retry.
    Unknown,
}

/// Which persistence checkpoint failed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointStage {
    /// `TurnHook::before_inference` refused to commit the pre-inference
    /// snapshot; no inference was started.
    BeforeInference,
    /// A runtime above the waist failed to commit the outcome boundary.
    OutcomeBoundary,
}

/// Origin of an error, used by runtimes to dispatch handling without
/// inspecting message text.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSource {
    /// Provider request / transport / cancellation.
    Provider,
    /// The LLM produced output that violates the declared contract.
    LlmOutput,
    /// A tool reported a business failure or a policy gate rejected a call.
    Tool,
    /// Execution / persistence infrastructure failed (dispatcher, checkpoint).
    Runtime,
    /// Snapshot / resume input inconsistency.
    Snapshot,
    /// Programming error or broken invariant.
    Internal,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum LLMComputeError {
    #[error("llm timeout")]
    Timeout,

    #[error("llm cancelled")]
    Cancelled,

    /// Provider-side failure surfaced after the adapter's own retry/fallback
    /// chain has given up.
    #[error("llm provider failed ({failure:?}): {message}")]
    Provider {
        failure: ProviderFailure,
        message: String,
    },

    /// LLM response did not satisfy the declared output protocol (strict
    /// JSON parse failure, behavior parser failure, empty payload).
    #[error("llm output parse failed: {0}")]
    OutputParse(String),

    /// PolicyEngine rejected a tool call.
    #[error("policy rejected: {0}")]
    PolicyRejected(String),

    /// A specific tool call ran and reported a business failure.
    #[error("tool `{tool}` failed: {message}")]
    ToolFailed {
        tool: String,
        call_id: String,
        message: String,
    },

    /// The tool dispatch infrastructure failed. `effect_unknown` is true
    /// when the call may have started and its side effects cannot be
    /// confirmed; the waist never replays such a call.
    #[error("tool `{tool}` dispatch failed (effect_unknown={effect_unknown}): {message}")]
    ToolRuntime {
        tool: String,
        call_id: String,
        message: String,
        effect_unknown: bool,
    },

    /// A critical persistence checkpoint could not be committed. The run
    /// stopped before producing further side effects; the in-memory
    /// snapshot is still valid and resumable.
    #[error("checkpoint {stage:?} failed: {message}")]
    Checkpoint {
        stage: CheckpointStage,
        message: String,
    },

    /// Snapshot deserialization / state corruption / resume fill mismatch.
    #[error("snapshot corrupted: {0}")]
    SnapshotCorrupted(String),

    /// Internal / programming error.
    #[error("internal: {0}")]
    Internal(String),
}

impl LLMComputeError {
    pub fn provider(failure: ProviderFailure, message: impl Into<String>) -> Self {
        Self::Provider {
            failure,
            message: message.into(),
        }
    }

    pub fn source(&self) -> ErrorSource {
        match self {
            Self::Timeout | Self::Cancelled | Self::Provider { .. } => ErrorSource::Provider,
            Self::OutputParse(_) => ErrorSource::LlmOutput,
            Self::PolicyRejected(_) | Self::ToolFailed { .. } => ErrorSource::Tool,
            Self::ToolRuntime { .. } | Self::Checkpoint { .. } => ErrorSource::Runtime,
            Self::SnapshotCorrupted(_) => ErrorSource::Snapshot,
            Self::Internal(_) => ErrorSource::Internal,
        }
    }

    /// True when feeding the error back to the LLM as an observation is a
    /// meaningful recovery strategy. Everything else ends the run.
    pub fn llm_correctable(&self) -> bool {
        match self {
            Self::OutputParse(_) | Self::PolicyRejected(_) | Self::ToolFailed { .. } => true,
            Self::Timeout
            | Self::Cancelled
            | Self::Provider { .. }
            | Self::ToolRuntime { .. }
            | Self::Checkpoint { .. }
            | Self::SnapshotCorrupted(_)
            | Self::Internal(_) => false,
        }
    }

    /// True when the layer above may re-run the failed step without risking
    /// duplicated side effects: nothing of this run is in an unknown state
    /// and the failure kind is known to be temporary.
    pub fn infra_retry_safe(&self) -> bool {
        match self {
            Self::Timeout | Self::Checkpoint { .. } => true,
            Self::Provider { failure, .. } => *failure == ProviderFailure::Transient,
            Self::ToolRuntime { effect_unknown, .. } => !*effect_unknown,
            Self::Cancelled
            | Self::OutputParse(_)
            | Self::PolicyRejected(_)
            | Self::ToolFailed { .. }
            | Self::SnapshotCorrupted(_)
            | Self::Internal(_) => false,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LLMComputeErrorRepr {
    Timeout,
    Cancelled,
    Provider {
        failure: ProviderFailure,
        message: String,
    },
    OutputParse {
        message: String,
    },
    PolicyRejected {
        message: String,
    },
    ToolFailed {
        tool: String,
        call_id: String,
        message: String,
    },
    ToolRuntime {
        tool: String,
        call_id: String,
        message: String,
        effect_unknown: bool,
    },
    Checkpoint {
        stage: CheckpointStage,
        message: String,
    },
    SnapshotCorrupted {
        message: String,
    },
    Internal {
        message: String,
    },
}

impl From<&LLMComputeError> for LLMComputeErrorRepr {
    fn from(value: &LLMComputeError) -> Self {
        match value {
            LLMComputeError::Timeout => Self::Timeout,
            LLMComputeError::Cancelled => Self::Cancelled,
            LLMComputeError::Provider { failure, message } => Self::Provider {
                failure: *failure,
                message: message.clone(),
            },
            LLMComputeError::OutputParse(message) => Self::OutputParse {
                message: message.clone(),
            },
            LLMComputeError::PolicyRejected(message) => Self::PolicyRejected {
                message: message.clone(),
            },
            LLMComputeError::ToolFailed {
                tool,
                call_id,
                message,
            } => Self::ToolFailed {
                tool: tool.clone(),
                call_id: call_id.clone(),
                message: message.clone(),
            },
            LLMComputeError::ToolRuntime {
                tool,
                call_id,
                message,
                effect_unknown,
            } => Self::ToolRuntime {
                tool: tool.clone(),
                call_id: call_id.clone(),
                message: message.clone(),
                effect_unknown: *effect_unknown,
            },
            LLMComputeError::Checkpoint { stage, message } => Self::Checkpoint {
                stage: *stage,
                message: message.clone(),
            },
            LLMComputeError::SnapshotCorrupted(message) => Self::SnapshotCorrupted {
                message: message.clone(),
            },
            LLMComputeError::Internal(message) => Self::Internal {
                message: message.clone(),
            },
        }
    }
}

impl From<LLMComputeErrorRepr> for LLMComputeError {
    fn from(value: LLMComputeErrorRepr) -> Self {
        match value {
            LLMComputeErrorRepr::Timeout => Self::Timeout,
            LLMComputeErrorRepr::Cancelled => Self::Cancelled,
            LLMComputeErrorRepr::Provider { failure, message } => {
                Self::Provider { failure, message }
            }
            LLMComputeErrorRepr::OutputParse { message } => Self::OutputParse(message),
            LLMComputeErrorRepr::PolicyRejected { message } => Self::PolicyRejected(message),
            LLMComputeErrorRepr::ToolFailed {
                tool,
                call_id,
                message,
            } => Self::ToolFailed {
                tool,
                call_id,
                message,
            },
            LLMComputeErrorRepr::ToolRuntime {
                tool,
                call_id,
                message,
                effect_unknown,
            } => Self::ToolRuntime {
                tool,
                call_id,
                message,
                effect_unknown,
            },
            LLMComputeErrorRepr::Checkpoint { stage, message } => {
                Self::Checkpoint { stage, message }
            }
            LLMComputeErrorRepr::SnapshotCorrupted { message } => Self::SnapshotCorrupted(message),
            LLMComputeErrorRepr::Internal { message } => Self::Internal(message),
        }
    }
}

impl Serialize for LLMComputeError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        LLMComputeErrorRepr::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LLMComputeError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        LLMComputeErrorRepr::deserialize(deserializer).map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckpointStage, ErrorSource, LLMComputeError, ProviderFailure};

    fn round_trip(err: &LLMComputeError) -> serde_json::Value {
        let value = serde_json::to_value(err).expect("serialize");
        assert_eq!(
            &serde_json::from_value::<LLMComputeError>(value.clone()).expect("deserialize"),
            err
        );
        value
    }

    #[test]
    fn provider_error_carries_failure_class() {
        let err = LLMComputeError::provider(ProviderFailure::Transient, "quota exceeded");
        assert_eq!(
            round_trip(&err),
            serde_json::json!({
                "kind": "provider",
                "failure": "transient",
                "message": "quota exceeded",
            })
        );
        assert_eq!(err.source(), ErrorSource::Provider);
        assert!(!err.llm_correctable());
        assert!(err.infra_retry_safe());
        let permanent = LLMComputeError::provider(ProviderFailure::Permanent, "bad key");
        assert!(!permanent.infra_retry_safe());
        let unknown = LLMComputeError::provider(ProviderFailure::Unknown, "?");
        assert!(!unknown.infra_retry_safe());
    }

    #[test]
    fn tool_failed_error_keeps_existing_flat_shape() {
        let err = LLMComputeError::ToolFailed {
            tool: "read".to_string(),
            call_id: "call-1".to_string(),
            message: "missing path".to_string(),
        };
        assert_eq!(
            round_trip(&err),
            serde_json::json!({
                "kind": "tool_failed",
                "tool": "read",
                "call_id": "call-1",
                "message": "missing path",
            })
        );
        assert_eq!(err.source(), ErrorSource::Tool);
        assert!(err.llm_correctable());
    }

    #[test]
    fn runtime_errors_are_never_llm_correctable() {
        let dispatch = LLMComputeError::ToolRuntime {
            tool: "bash".to_string(),
            call_id: "c".to_string(),
            message: "sandbox gone".to_string(),
            effect_unknown: true,
        };
        assert_eq!(
            round_trip(&dispatch),
            serde_json::json!({
                "kind": "tool_runtime",
                "tool": "bash",
                "call_id": "c",
                "message": "sandbox gone",
                "effect_unknown": true,
            })
        );
        assert_eq!(dispatch.source(), ErrorSource::Runtime);
        assert!(!dispatch.llm_correctable());
        assert!(!dispatch.infra_retry_safe());

        let checkpoint = LLMComputeError::Checkpoint {
            stage: CheckpointStage::BeforeInference,
            message: "disk full".to_string(),
        };
        assert_eq!(
            round_trip(&checkpoint),
            serde_json::json!({
                "kind": "checkpoint",
                "stage": "before_inference",
                "message": "disk full",
            })
        );
        assert_eq!(checkpoint.source(), ErrorSource::Runtime);
        assert!(!checkpoint.llm_correctable());
        assert!(checkpoint.infra_retry_safe());
        assert!(!LLMComputeError::Internal("bug".into()).llm_correctable());
    }
}
