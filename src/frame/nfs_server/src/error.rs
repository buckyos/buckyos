//! NFSP structured error model (protocol §8).
//!
//! HTTP status codes are only a coarse hint; the structured `code` is authoritative.
//! Codes not present in NFSP §8 but required by this implementation are documented
//! in README.md (`INVALID_ARGUMENT`, `NOT_EMPTY`, `UNSUPPORTED`, `INTERNAL`).

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NotFound,
    NeedPull,
    RevMismatch,
    TargetMismatch,
    LeaseConflict,
    SeqOutOfWindow,
    Stale,
    NamespaceConflict,
    AmbiguousEntry,
    NotAContainer,
    Referral,
    PermissionDenied,
    PolicyDenied,
    UnsupportedExt,
    QuotaExceeded,
    // Implementation extensions (documented in README):
    InvalidArgument,
    NotEmpty,
    Unsupported,
    Internal,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::NeedPull => "NEED_PULL",
            ErrorCode::RevMismatch => "REV_MISMATCH",
            ErrorCode::TargetMismatch => "TARGET_MISMATCH",
            ErrorCode::LeaseConflict => "LEASE_CONFLICT",
            ErrorCode::SeqOutOfWindow => "SEQ_OUT_OF_WINDOW",
            ErrorCode::Stale => "STALE",
            ErrorCode::NamespaceConflict => "NAMESPACE_CONFLICT",
            ErrorCode::AmbiguousEntry => "AMBIGUOUS_ENTRY",
            ErrorCode::NotAContainer => "NOT_A_CONTAINER",
            ErrorCode::Referral => "REFERRAL",
            ErrorCode::PermissionDenied => "PERMISSION_DENIED",
            ErrorCode::PolicyDenied => "POLICY_DENIED",
            ErrorCode::UnsupportedExt => "UNSUPPORTED_EXT",
            ErrorCode::QuotaExceeded => "QUOTA_EXCEEDED",
            ErrorCode::InvalidArgument => "INVALID_ARGUMENT",
            ErrorCode::NotEmpty => "NOT_EMPTY",
            ErrorCode::Unsupported => "UNSUPPORTED",
            ErrorCode::Internal => "INTERNAL",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            ErrorCode::NotFound => 404,
            ErrorCode::NeedPull => 409,
            ErrorCode::RevMismatch => 409,
            ErrorCode::TargetMismatch => 409,
            ErrorCode::LeaseConflict => 423,
            ErrorCode::SeqOutOfWindow => 409,
            ErrorCode::Stale => 410,
            ErrorCode::NamespaceConflict => 409,
            ErrorCode::AmbiguousEntry => 409,
            ErrorCode::NotAContainer => 400,
            ErrorCode::Referral => 307,
            ErrorCode::PermissionDenied => 403,
            ErrorCode::PolicyDenied => 403,
            ErrorCode::UnsupportedExt => 400,
            ErrorCode::QuotaExceeded => 507,
            ErrorCode::InvalidArgument => 400,
            ErrorCode::NotEmpty => 409,
            ErrorCode::Unsupported => 400,
            ErrorCode::Internal => 500,
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{}: {message}", code.as_str())]
pub struct NfsError {
    pub code: ErrorCode,
    pub message: String,
    /// Extra structured fields merged into the error object
    /// (e.g. `required_op`, `holder`, `obj_id`, `reason`).
    pub details: Value,
}

impl NfsError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        NfsError { code, message: message.into(), details: Value::Null }
    }

    pub fn with(mut self, key: &str, value: Value) -> Self {
        if !self.details.is_object() {
            self.details = json!({});
        }
        self.details.as_object_mut().unwrap().insert(key.to_string(), value);
        self
    }

    pub fn to_json(&self) -> Value {
        let mut obj = json!({
            "code": self.code.as_str(),
            "message": self.message,
        });
        if let Some(extra) = self.details.as_object() {
            for (k, v) in extra {
                obj.as_object_mut().unwrap().insert(k.clone(), v.clone());
            }
        }
        obj
    }
}

pub type NfsResult<T> = Result<T, NfsError>;

pub fn not_found(msg: impl Into<String>) -> NfsError {
    NfsError::new(ErrorCode::NotFound, msg)
}
pub fn stale(msg: impl Into<String>) -> NfsError {
    NfsError::new(ErrorCode::Stale, msg)
}
pub fn invalid(msg: impl Into<String>) -> NfsError {
    NfsError::new(ErrorCode::InvalidArgument, msg)
}
pub fn internal(msg: impl Into<String>) -> NfsError {
    NfsError::new(ErrorCode::Internal, msg)
}
pub fn rev_mismatch(expected: &str, actual: &str) -> NfsError {
    NfsError::new(ErrorCode::RevMismatch, "container revision mismatch")
        .with("expected", json!(expected))
        .with("actual", json!(actual))
}

impl From<std::io::Error> for NfsError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => not_found(format!("io: {}", e)),
            std::io::ErrorKind::PermissionDenied => {
                NfsError::new(ErrorCode::PermissionDenied, format!("io: {}", e))
            }
            _ => internal(format!("io: {}", e)),
        }
    }
}

impl From<rusqlite::Error> for NfsError {
    fn from(e: rusqlite::Error) -> Self {
        internal(format!("filedb: {}", e))
    }
}

impl From<serde_json::Error> for NfsError {
    fn from(e: serde_json::Error) -> Self {
        invalid(format!("json: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_json_shape() {
        let e = NfsError::new(ErrorCode::PermissionDenied, "no")
            .with("required_op", json!("write"));
        let v = e.to_json();
        assert_eq!(v["code"], "PERMISSION_DENIED");
        assert_eq!(v["required_op"], "write");
        assert_eq!(e.code.http_status(), 403);
    }

    #[test]
    fn stale_maps_410() {
        assert_eq!(ErrorCode::Stale.http_status(), 410);
        assert_eq!(ErrorCode::LeaseConflict.http_status(), 423);
        assert_eq!(ErrorCode::QuotaExceeded.http_status(), 507);
    }
}
