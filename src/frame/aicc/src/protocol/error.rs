use buckyos_api::{AiccError, AiccErrorCode};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtocolErrorKind {
    InvalidConfiguration,
    InvalidRequest,
    Authentication,
    Transport,
    Timeout,
    ResponseTooLarge,
    InvalidResponse,
    ProviderRejected,
    DuplicateAdapter,
    UnknownAdapter,
    UnsupportedOperation,
    DeadlineExceeded,
    Cancelled,
    WebhookRejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProtocolError {
    pub kind: ProtocolErrorKind,
    pub message: String,
    pub provider_code: Option<String>,
    pub request_id: Option<String>,
    pub retry_after: Option<Duration>,
}

impl ProtocolError {
    pub(crate) fn new(kind: ProtocolErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            provider_code: None,
            request_id: None,
            retry_after: None,
        }
    }

    pub(crate) fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    pub(crate) fn with_provider_code(mut self, provider_code: Option<String>) -> Self {
        self.provider_code = provider_code;
        self
    }

    pub(crate) fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }

    pub(crate) fn invalid_configuration(message: impl Into<String>) -> Self {
        Self::new(ProtocolErrorKind::InvalidConfiguration, message)
    }

    pub(crate) fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ProtocolErrorKind::InvalidRequest, message)
    }

    pub(crate) fn invalid_response(message: impl Into<String>) -> Self {
        Self::new(ProtocolErrorKind::InvalidResponse, message)
    }

    pub(crate) fn is_model_unavailable(&self) -> bool {
        self.provider_code.as_deref() == Some("1211")
            || contains_any(
                &self.message,
                &["model does not exist", "model not found", "unknown model"],
            )
    }

    pub(crate) fn retry_same_model(&self) -> bool {
        !self.requires_candidate_change()
            && (matches!(
                self.kind,
                ProtocolErrorKind::Transport
                    | ProtocolErrorKind::Timeout
                    | ProtocolErrorKind::DeadlineExceeded
                    | ProtocolErrorKind::InvalidResponse
            ) || self.retry_after.is_some())
    }

    pub(crate) fn allows_model_failover(&self) -> bool {
        self.requires_candidate_change() || self.retry_same_model()
    }

    fn requires_candidate_change(&self) -> bool {
        self.is_model_unavailable()
            || contains_any(
                &self.message,
                &[
                    "quota exhausted",
                    "quota exceeded",
                    "insufficient quota",
                    "insufficient balance",
                    "model temporarily unavailable",
                ],
            )
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    let value = value.to_ascii_lowercase();
    needles.iter().any(|needle| value.contains(needle))
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProtocolError {}

impl From<ProtocolError> for AiccError {
    fn from(error: ProtocolError) -> Self {
        let retriable = matches!(
            error.kind,
            ProtocolErrorKind::Transport
                | ProtocolErrorKind::Timeout
                | ProtocolErrorKind::DeadlineExceeded
        );
        let code = match error.kind {
            ProtocolErrorKind::InvalidConfiguration
            | ProtocolErrorKind::DuplicateAdapter
            | ProtocolErrorKind::UnknownAdapter => AiccErrorCode::InternalError,
            ProtocolErrorKind::InvalidRequest | ProtocolErrorKind::UnsupportedOperation => {
                AiccErrorCode::InvalidRequest
            }
            ProtocolErrorKind::Authentication | ProtocolErrorKind::ProviderRejected => {
                AiccErrorCode::ProviderError
            }
            ProtocolErrorKind::WebhookRejected => AiccErrorCode::PolicyDenied,
            ProtocolErrorKind::Timeout | ProtocolErrorKind::DeadlineExceeded => {
                AiccErrorCode::Timeout
            }
            ProtocolErrorKind::Cancelled => AiccErrorCode::Cancelled,
            ProtocolErrorKind::Transport
            | ProtocolErrorKind::ResponseTooLarge
            | ProtocolErrorKind::InvalidResponse => AiccErrorCode::ProviderError,
        };
        let mut mapped = AiccError::new(code, error.message);
        mapped.provider_code = error.provider_code;
        mapped.retriable = retriable;
        mapped.details = (error.request_id.is_some() || error.retry_after.is_some()).then(|| {
            serde_json::json!({
                "request_id": error.request_id,
                "retry_after_ms": error.retry_after.map(|duration| duration.as_millis() as u64),
            })
        });
        mapped
    }
}

pub(crate) type ProtocolResultValue<T> = Result<T, ProtocolError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_protocol_failures_to_stable_public_errors() {
        let timeout: AiccError = ProtocolError::new(ProtocolErrorKind::Timeout, "timed out").into();
        assert_eq!(timeout.code, AiccErrorCode::Timeout);

        let malformed: AiccError = ProtocolError::invalid_response("bad response").into();
        assert_eq!(malformed.code, AiccErrorCode::ProviderError);

        let configuration: AiccError = ProtocolError::invalid_configuration("bad adapter").into();
        assert_eq!(configuration.code, AiccErrorCode::InternalError);
    }

    #[test]
    fn retry_classification_separates_jitter_from_candidate_change() {
        let timeout = ProtocolError::new(ProtocolErrorKind::Timeout, "timed out");
        assert!(timeout.retry_same_model());
        assert!(timeout.allows_model_failover());

        for error in [
            ProtocolError::new(ProtocolErrorKind::InvalidRequest, "model does not exist")
                .with_provider_code(Some("1211".to_owned())),
            ProtocolError::new(ProtocolErrorKind::ProviderRejected, "quota exhausted"),
        ] {
            assert!(!error.retry_same_model());
            assert!(error.allows_model_failover());
        }

        let authentication =
            ProtocolError::new(ProtocolErrorKind::Authentication, "invalid API key");
        assert!(!authentication.retry_same_model());
        assert!(!authentication.allows_model_failover());
    }
}
