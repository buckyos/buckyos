//! Stable error codes (design doc §5.1). Business failures never travel as free text.

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    InvalidSchema,
    InvalidOperation,
    LimitExceeded,
    NotFound,
    TargetDeleted,
    ReferenceBroken,
    PermissionDenied,
    RevisionConflict,
    SchemaConflict,
    IdempotencyMismatch,
    UnsupportedVersion,
    MissingExtension,
    DependencyUnavailable,
    StorageFull,
    StorageIoError,
    WriterBusy,
    BaseUnknown,
    BaseTooOld,
    EpochMismatch,
    SnapshotExpired,
    NotUndoable,
    RunCancelled,
    LockRequired,
    LockHeld,
    LockLost,
}

impl Code {
    pub const ALL: &'static [Code] = &[
        Code::InvalidSchema,
        Code::InvalidOperation,
        Code::LimitExceeded,
        Code::NotFound,
        Code::TargetDeleted,
        Code::ReferenceBroken,
        Code::PermissionDenied,
        Code::RevisionConflict,
        Code::SchemaConflict,
        Code::IdempotencyMismatch,
        Code::UnsupportedVersion,
        Code::MissingExtension,
        Code::DependencyUnavailable,
        Code::StorageFull,
        Code::StorageIoError,
        Code::WriterBusy,
        Code::BaseUnknown,
        Code::BaseTooOld,
        Code::EpochMismatch,
        Code::SnapshotExpired,
        Code::NotUndoable,
        Code::RunCancelled,
        Code::LockRequired,
        Code::LockHeld,
        Code::LockLost,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Code::InvalidSchema => "INVALID_SCHEMA",
            Code::InvalidOperation => "INVALID_OPERATION",
            Code::LimitExceeded => "LIMIT_EXCEEDED",
            Code::NotFound => "NOT_FOUND",
            Code::TargetDeleted => "TARGET_DELETED",
            Code::ReferenceBroken => "REFERENCE_BROKEN",
            Code::PermissionDenied => "PERMISSION_DENIED",
            Code::RevisionConflict => "REVISION_CONFLICT",
            Code::SchemaConflict => "SCHEMA_CONFLICT",
            Code::IdempotencyMismatch => "IDEMPOTENCY_MISMATCH",
            Code::UnsupportedVersion => "UNSUPPORTED_VERSION",
            Code::MissingExtension => "MISSING_EXTENSION",
            Code::DependencyUnavailable => "DEPENDENCY_UNAVAILABLE",
            Code::StorageFull => "STORAGE_FULL",
            Code::StorageIoError => "STORAGE_IO_ERROR",
            Code::WriterBusy => "WRITER_BUSY",
            Code::BaseUnknown => "BASE_UNKNOWN",
            Code::BaseTooOld => "BASE_TOO_OLD",
            Code::EpochMismatch => "EPOCH_MISMATCH",
            Code::SnapshotExpired => "SNAPSHOT_EXPIRED",
            Code::NotUndoable => "NOT_UNDOABLE",
            Code::RunCancelled => "RUN_CANCELLED",
            Code::LockRequired => "LOCK_REQUIRED",
            Code::LockHeld => "LOCK_HELD",
            Code::LockLost => "LOCK_LOST",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Code::StorageFull
                | Code::StorageIoError
                | Code::WriterBusy
                | Code::BaseUnknown
                | Code::SnapshotExpired
        )
    }

    /// Codes reported with `status: "conflict"` (a competing change the caller must
    /// resolve while keeping its candidate), as opposed to `rejected`.
    pub fn is_conflict(&self) -> bool {
        matches!(self, Code::RevisionConflict | Code::SchemaConflict | Code::TargetDeleted)
    }
}

#[derive(Debug, Clone)]
pub struct WsError {
    pub code: Code,
    /// Sub-code, e.g. `ID_CONFLICT` under `INVALID_OPERATION`.
    pub sub: Option<&'static str>,
    pub detail: String,
    /// JSON pointer into the failing operation, when known.
    pub path: Option<String>,
    /// Structured, authorization-safe extras (referrer lists, counts, ...).
    pub data: Option<Value>,
}

pub type WsResult<T> = Result<T, WsError>;

impl WsError {
    pub fn new(code: Code, detail: impl Into<String>) -> Self {
        WsError { code, sub: None, detail: detail.into(), path: None, data: None }
    }
    pub fn sub(code: Code, sub: &'static str, detail: impl Into<String>) -> Self {
        WsError { code, sub: Some(sub), detail: detail.into(), path: None, data: None }
    }
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
    pub fn invalid_op(detail: impl Into<String>) -> Self {
        Self::new(Code::InvalidOperation, detail)
    }
    pub fn invalid_schema(detail: impl Into<String>) -> Self {
        Self::new(Code::InvalidSchema, detail)
    }
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(Code::NotFound, detail)
    }
    pub fn deleted(detail: impl Into<String>) -> Self {
        Self::new(Code::TargetDeleted, detail)
    }
    pub fn denied(detail: impl Into<String>) -> Self {
        Self::new(Code::PermissionDenied, detail)
    }
    pub fn limit(detail: impl Into<String>) -> Self {
        Self::new(Code::LimitExceeded, detail)
    }
    pub fn io(detail: impl Into<String>) -> Self {
        Self::new(Code::StorageIoError, detail)
    }

    /// `{ code, retryable, detail, sub_code?, path?, ... }`
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "code": self.code.as_str(),
            "retryable": self.code.retryable(),
            "detail": self.detail,
        });
        let m = v.as_object_mut().unwrap();
        if let Some(s) = self.sub {
            m.insert("sub_code".into(), json!(s));
        }
        if let Some(p) = &self.path {
            m.insert("path".into(), json!(p));
        }
        if let Some(d) = &self.data {
            m.insert("data".into(), d.clone());
        }
        v
    }
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.sub {
            Some(s) => write!(f, "{}/{}: {}", self.code.as_str(), s, self.detail),
            None => write!(f, "{}: {}", self.code.as_str(), self.detail),
        }
    }
}
impl std::error::Error for WsError {}
