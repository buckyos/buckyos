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
    pub request_id: Option<String>,
    pub retry_after: Option<Duration>,
}

impl ProtocolError {
    pub(crate) fn new(kind: ProtocolErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            request_id: None,
            retry_after: None,
        }
    }

    pub(crate) fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
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
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProtocolError {}

impl From<ProtocolError> for AiccError {
    fn from(error: ProtocolError) -> Self {
        let code = match error.kind {
            ProtocolErrorKind::InvalidConfiguration
            | ProtocolErrorKind::DuplicateAdapter
            | ProtocolErrorKind::UnknownAdapter => AiccErrorCode::InternalError,
            ProtocolErrorKind::InvalidRequest | ProtocolErrorKind::UnsupportedOperation => {
                AiccErrorCode::InvalidRequest
            }
            ProtocolErrorKind::Authentication => AiccErrorCode::ProviderError,
            ProtocolErrorKind::WebhookRejected => AiccErrorCode::PolicyDenied,
            ProtocolErrorKind::Timeout | ProtocolErrorKind::DeadlineExceeded => {
                AiccErrorCode::Timeout
            }
            ProtocolErrorKind::Cancelled => AiccErrorCode::Cancelled,
            ProtocolErrorKind::Transport
            | ProtocolErrorKind::ResponseTooLarge
            | ProtocolErrorKind::InvalidResponse => AiccErrorCode::ProviderError,
        };
        AiccError::new(code, error.message)
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
}
