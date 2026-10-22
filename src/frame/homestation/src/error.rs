use buckyos_api::{AccessDenial, DenyReason, ACCESS_DENIED_ERROR_CODE};
use http::StatusCode;

#[derive(Debug, Clone, PartialEq)]
pub enum HsError {
    BadRequest(String),
    NotFound(String),
    Forbidden(String),
    Conflict(String),
    Unavailable(String),
    Internal(String),
    /// No valid session: missing, expired or not verifiable.
    Unauthorized(String),
    /// A D-4 denial: the page branches on its reason.
    Denied(AccessDenial),
}

pub type HsResult<T> = Result<T, HsError>;

impl HsError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::NotFound(_) => "not_found",
            Self::Forbidden(_) => "forbidden",
            Self::Conflict(_) => "conflict",
            Self::Unavailable(_) => "unavailable",
            Self::Internal(_) => "internal",
            Self::Unauthorized(_) => "unauthorized",
            Self::Denied(_) => ACCESS_DENIED_ERROR_CODE,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::BadRequest(m)
            | Self::NotFound(m)
            | Self::Forbidden(m)
            | Self::Conflict(m)
            | Self::Unavailable(m)
            | Self::Internal(m)
            | Self::Unauthorized(m) => m,
            Self::Denied(d) => d.reason.as_str(),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Denied(d) if d.reason == DenyReason::AuthorizationUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Denied(_) => StatusCode::FORBIDDEN,
        }
    }

    pub fn retryable(&self) -> bool {
        match self {
            Self::Unavailable(_) | Self::Internal(_) => true,
            Self::Denied(d) => d.reason == DenyReason::AuthorizationUnavailable,
            _ => false,
        }
    }

    /// The denial carried on the wire beside the message (`data` of an `access_denied` error).
    pub fn denial(&self) -> Option<&AccessDenial> {
        match self {
            Self::Denied(d) => Some(d),
            _ => None,
        }
    }
}

impl std::fmt::Display for HsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl From<rusqlite::Error> for HsError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Internal(format!("database: {e}"))
    }
}

impl From<serde_json::Error> for HsError {
    fn from(e: serde_json::Error) -> Self {
        Self::Internal(format!("json: {e}"))
    }
}

pub fn bad(msg: impl Into<String>) -> HsError {
    HsError::BadRequest(msg.into())
}

pub fn denied(reason: DenyReason, method: &str) -> HsError {
    HsError::Denied(AccessDenial::new(reason).at("homestation", method))
}
