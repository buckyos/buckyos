//! Error type produced by the LLMContext loop.
//!
//! The variants here describe *what kind* of failure occurred. Classification
//! into `Recoverable` vs `Fatal` (see `request::ErrorClass`) is a separate
//! concern handled by the loop based on `ErrorPolicy`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
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

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LLMComputeErrorRepr {
    Timeout,
    Cancelled,
    Provider {
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
            LLMComputeError::Provider(message) => Self::Provider {
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
            LLMComputeErrorRepr::Provider { message } => Self::Provider(message),
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
    use super::LLMComputeError;

    #[test]
    fn provider_error_serializes_as_flat_message() {
        let err = LLMComputeError::Provider("quota exceeded".to_string());

        let value = serde_json::to_value(&err).expect("serialize provider error");

        assert_eq!(
            value,
            serde_json::json!({
                "kind": "provider",
                "message": "quota exceeded",
            })
        );
        assert_eq!(
            serde_json::from_value::<LLMComputeError>(value).expect("deserialize provider error"),
            err
        );
    }

    #[test]
    fn tool_failed_error_keeps_existing_flat_shape() {
        let err = LLMComputeError::ToolFailed {
            tool: "read".to_string(),
            call_id: "call-1".to_string(),
            message: "missing path".to_string(),
        };

        let value = serde_json::to_value(&err).expect("serialize tool error");

        assert_eq!(
            value,
            serde_json::json!({
                "kind": "tool_failed",
                "tool": "read",
                "call_id": "call-1",
                "message": "missing path",
            })
        );
        assert_eq!(
            serde_json::from_value::<LLMComputeError>(value).expect("deserialize tool error"),
            err
        );
    }
}
