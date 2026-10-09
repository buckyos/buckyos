use http::StatusCode;

#[derive(Debug, Clone, PartialEq)]
pub enum HsError {
    BadRequest(String),
    NotFound(String),
    Forbidden(String),
    Conflict(String),
    Unavailable(String),
    Internal(String),
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
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::BadRequest(m)
            | Self::NotFound(m)
            | Self::Forbidden(m)
            | Self::Conflict(m)
            | Self::Unavailable(m)
            | Self::Internal(m) => m,
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
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_) | Self::Internal(_))
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
