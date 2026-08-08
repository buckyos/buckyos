//! Task Dispatch Center protocol types, client and target-side SDK.
//!
//! Design: `doc/task_mgr/task_dispatch_center.md`.
//!
//! The Dispatcher manages the durable-handoff lifecycle *before* a Target
//! accepts a piece of work. After `Accepted(target_task_id)` the business
//! task is a plain TaskMgr task owned by the Target; the Dispatcher keeps
//! only the immutable `dispatch_id <-> target_task_id` link.
//!
//! It shares the task-manager process and port (3380) but is an independent
//! service surface: own RPC path (`/kapi/task-dispatcher`), own rdb instance
//! (`task-dispatcher-main`), own authorization rules. TaskMgr never depends
//! on it.
//!
//! All `*_at` / `*_ms` timestamps in this protocol are unix milliseconds.

use ::kRPC::*;
use async_trait::async_trait;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::kevent_client::KEventClient;
use crate::rdb_mgr::{RdbBackend, RdbInstanceConfig};

pub const TASK_DISPATCHER_SERVICE_NAME: &str = "task-dispatcher";
/// Same process / port as task-manager; only the kapi path differs.
pub const TASK_DISPATCHER_SERVICE_PORT: u16 = 3380;

/// Logical name of the dispatcher rdb instance. Registered by the scheduler
/// inside the *task-manager* service spec (same deployment unit), resolved by
/// the dispatcher module with `get_rdb_instance(TASK_MANAGER_SERVICE_NAME,
/// None, TASK_DISPATCHER_RDB_INSTANCE_ID)`. No table/schema/join is shared
/// with `task-mgr-main`.
pub const TASK_DISPATCHER_RDB_INSTANCE_ID: &str = "task-dispatcher-main";
/// v2 (M4 approval gate): `dispatch_record.approval` column + the
/// `PendingApproval` status. v1 stores are migrated in place on open.
pub const TASK_DISPATCHER_RDB_SCHEMA_VERSION: u64 = 2;

pub const TASK_DISPATCHER_RDB_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS dispatch_target (
    target_id       TEXT PRIMARY KEY,
    owner_user_id   TEXT NOT NULL,
    owner_app_id    TEXT NOT NULL,
    registration    TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    lease_epoch_seq INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS dispatch_operation_route (
    operation         TEXT PRIMARY KEY,
    default_target_id TEXT NOT NULL,
    revision          INTEGER NOT NULL DEFAULT 1,
    enabled           INTEGER NOT NULL DEFAULT 1,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_dor_target ON dispatch_operation_route(default_target_id);

CREATE TABLE IF NOT EXISTS dispatch_instance (
    target_id          TEXT NOT NULL,
    instance_id        TEXT NOT NULL,
    lease_epoch        INTEGER NOT NULL,
    lease_expires_at   INTEGER NOT NULL,
    capacity           INTEGER NOT NULL DEFAULT 1,
    available_capacity INTEGER NOT NULL DEFAULT 1,
    attached_at        INTEGER NOT NULL,
    renewed_at         INTEGER NOT NULL,
    PRIMARY KEY (target_id, instance_id)
);
CREATE INDEX IF NOT EXISTS idx_di_lease ON dispatch_instance(lease_expires_at);

CREATE TABLE IF NOT EXISTS dispatch_record (
    dispatch_id            TEXT PRIMARY KEY,
    requested_target_id    TEXT,
    target_id              TEXT NOT NULL,
    target_selection       TEXT NOT NULL,
    operation              TEXT NOT NULL,
    status                 TEXT NOT NULL,
    input                  TEXT NOT NULL,
    auth                   TEXT NOT NULL,
    idempotency_key        TEXT NOT NULL,
    requested_by_user      TEXT NOT NULL,
    requested_by_app       TEXT NOT NULL,
    on_behalf_of           TEXT NOT NULL,
    offer_instance_id      TEXT,
    offer_lease_expires_at INTEGER,
    offer_delivery_count   INTEGER NOT NULL DEFAULT 0,
    target_task_id         INTEGER,
    reject_reason          TEXT,
    approval               TEXT,
    message                TEXT,
    expires_at             INTEGER,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_dr_idem ON dispatch_record(requested_by_user, requested_by_app, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_dr_target_status ON dispatch_record(target_id, status, created_at);
CREATE INDEX IF NOT EXISTS idx_dr_due ON dispatch_record(status, expires_at);
CREATE INDEX IF NOT EXISTS idx_dr_requester ON dispatch_record(requested_by_user, requested_by_app, created_at DESC);

CREATE TABLE IF NOT EXISTS dispatch_event (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    dispatch_id TEXT NOT NULL,
    ts          INTEGER NOT NULL,
    from_status TEXT NOT NULL,
    to_status   TEXT NOT NULL,
    instance_id TEXT,
    detail      TEXT
);
CREATE INDEX IF NOT EXISTS idx_de_dispatch ON dispatch_event(dispatch_id, ts);
"#;

pub const TASK_DISPATCHER_RDB_SCHEMA_POSTGRES: &str = r#"
CREATE TABLE IF NOT EXISTS dispatch_target (
    target_id       TEXT PRIMARY KEY,
    owner_user_id   TEXT NOT NULL,
    owner_app_id    TEXT NOT NULL,
    registration    TEXT NOT NULL,
    enabled         BIGINT NOT NULL DEFAULT 1,
    lease_epoch_seq BIGINT NOT NULL DEFAULT 0,
    created_at      BIGINT NOT NULL,
    updated_at      BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS dispatch_operation_route (
    operation         TEXT PRIMARY KEY,
    default_target_id TEXT NOT NULL,
    revision          BIGINT NOT NULL DEFAULT 1,
    enabled           BIGINT NOT NULL DEFAULT 1,
    created_at        BIGINT NOT NULL,
    updated_at        BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_dor_target ON dispatch_operation_route(default_target_id);

CREATE TABLE IF NOT EXISTS dispatch_instance (
    target_id          TEXT NOT NULL,
    instance_id        TEXT NOT NULL,
    lease_epoch        BIGINT NOT NULL,
    lease_expires_at   BIGINT NOT NULL,
    capacity           BIGINT NOT NULL DEFAULT 1,
    available_capacity BIGINT NOT NULL DEFAULT 1,
    attached_at        BIGINT NOT NULL,
    renewed_at         BIGINT NOT NULL,
    PRIMARY KEY (target_id, instance_id)
);
CREATE INDEX IF NOT EXISTS idx_di_lease ON dispatch_instance(lease_expires_at);

CREATE TABLE IF NOT EXISTS dispatch_record (
    dispatch_id            TEXT PRIMARY KEY,
    requested_target_id    TEXT,
    target_id              TEXT NOT NULL,
    target_selection       TEXT NOT NULL,
    operation              TEXT NOT NULL,
    status                 TEXT NOT NULL,
    input                  TEXT NOT NULL,
    auth                   TEXT NOT NULL,
    idempotency_key        TEXT NOT NULL,
    requested_by_user      TEXT NOT NULL,
    requested_by_app       TEXT NOT NULL,
    on_behalf_of           TEXT NOT NULL,
    offer_instance_id      TEXT,
    offer_lease_expires_at BIGINT,
    offer_delivery_count   BIGINT NOT NULL DEFAULT 0,
    target_task_id         BIGINT,
    reject_reason          TEXT,
    approval               TEXT,
    message                TEXT,
    expires_at             BIGINT,
    created_at             BIGINT NOT NULL,
    updated_at             BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_dr_idem ON dispatch_record(requested_by_user, requested_by_app, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_dr_target_status ON dispatch_record(target_id, status, created_at);
CREATE INDEX IF NOT EXISTS idx_dr_due ON dispatch_record(status, expires_at);
CREATE INDEX IF NOT EXISTS idx_dr_requester ON dispatch_record(requested_by_user, requested_by_app, created_at DESC);

CREATE TABLE IF NOT EXISTS dispatch_event (
    id          BIGSERIAL PRIMARY KEY,
    dispatch_id TEXT NOT NULL,
    ts          BIGINT NOT NULL,
    from_status TEXT NOT NULL,
    to_status   TEXT NOT NULL,
    instance_id TEXT,
    detail      TEXT
);
CREATE INDEX IF NOT EXISTS idx_de_dispatch ON dispatch_event(dispatch_id, ts);
"#;

/// Default rdb-instance config for the dispatcher store. Dropped by the
/// scheduler into the task-manager service spec under
/// `TASK_DISPATCHER_RDB_INSTANCE_ID` — a second, independent instance next to
/// `task-mgr-main`, not a second deployment unit.
pub fn task_dispatcher_default_rdb_instance_config() -> RdbInstanceConfig {
    let mut schema = HashMap::new();
    schema.insert(
        RdbBackend::Sqlite,
        TASK_DISPATCHER_RDB_SCHEMA_SQLITE.to_string(),
    );
    schema.insert(
        RdbBackend::Postgres,
        TASK_DISPATCHER_RDB_SCHEMA_POSTGRES.to_string(),
    );
    RdbInstanceConfig {
        backend: RdbBackend::Sqlite,
        version: TASK_DISPATCHER_RDB_SCHEMA_VERSION,
        schema,
        connection: "sqlite://$appdata/dispatch.db?mode=rwc".to_string(),
    }
}

// ---------------------------------------------------------------------------
// stable error codes (embedded in RPC error messages)
// ---------------------------------------------------------------------------

pub const DISPATCH_ERR_DEFAULT_TARGET_NOT_CONFIGURED: &str = "default_target_not_configured";
pub const DISPATCH_ERR_IDEMPOTENCY_CONFLICT: &str = "idempotency_conflict";
pub const DISPATCH_ERR_TARGET_NOT_REGISTERED: &str = "target_not_registered";
pub const DISPATCH_ERR_TARGET_DISABLED: &str = "target_disabled";
pub const DISPATCH_ERR_UNSUPPORTED_OPERATION: &str = "unsupported_operation";
pub const DISPATCH_ERR_STALE_INSTANCE: &str = "stale_instance";
pub const DISPATCH_ERR_ALREADY_ACCEPTED: &str = "dispatch_already_accepted";
pub const DISPATCH_ERR_UNCERTAIN_REQUIRES_RESOLVE: &str = "uncertain_requires_resolve";
/// The record sits behind the approval gate: targets cannot claim, accept or
/// reject it, and approve/deny is the only way forward.
pub const DISPATCH_ERR_PENDING_APPROVAL: &str = "dispatch_pending_approval";
pub const DISPATCH_ERR_NOT_PENDING_APPROVAL: &str = "dispatch_not_pending_approval";

pub fn is_stale_instance_err(err: &RPCErrors) -> bool {
    err.to_string().contains(DISPATCH_ERR_STALE_INSTANCE)
}

// ---------------------------------------------------------------------------
// kevent channels
// ---------------------------------------------------------------------------

/// Per-target notification channel (an "offers may be due" hint for target
/// instances). `target_key` is the slug returned by `attach_instance` /
/// [`task_dispatcher_target_key`] — raw target ids (DIDs) may contain chars
/// that are invalid in kevent id segments.
pub fn task_dispatcher_target_event_id(target_key: &str) -> String {
    format!("/task_dispatcher/target/{}", target_key)
}

/// Per-record status-change channel for callers.
pub fn task_dispatcher_record_event_id(dispatch_id: &str) -> String {
    format!("/task_dispatcher/{}", dispatch_id)
}

/// Admin-facing "new record awaits approval" hint channel. Acceleration
/// only: the authoritative queue is always
/// `list_dispatches(status=PendingApproval)`, and a lost hint degrades to
/// admin-side polling without losing backlog. Payload carries ids only.
pub fn task_dispatcher_approvals_event_id() -> String {
    "/task_dispatcher/approvals".to_string()
}

/// Stable slug for a target id, usable as a kevent id segment
/// (`[A-Za-z0-9_.-]` only). A short fnv1a hash of the raw id is appended so
/// that ids differing only in stripped characters cannot collide.
pub fn task_dispatcher_target_key(target_id: &str) -> String {
    let mut slug: String = target_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    slug.truncate(64);
    let slug = slug.trim_matches('-').to_string();
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in target_id.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    if slug.is_empty() {
        format!("t-{:016x}", hash)
    } else {
        format!("{}-{:08x}", slug, (hash as u32) ^ ((hash >> 32) as u32))
    }
}

// ---------------------------------------------------------------------------
// core model
// ---------------------------------------------------------------------------

/// Handoff lifecycle states. Independent from `TaskStatus` — never reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchStatus {
    /// Manual-release gate *before* assignment (doc §7.1): the record is
    /// persisted but never evaluated, never offered, never claimable and
    /// does not consume `max_concurrency` until an admin approves it.
    /// Targets cannot act on it through any path.
    PendingApproval,
    Queued,
    WaitingForTarget,
    Offered,
    /// Normal terminal state: the Target accepted and returned its own
    /// `target_task_id`. The Dispatcher never mirrors that task's execution
    /// state afterwards.
    Accepted,
    Rejected,
    Expired,
    Canceled,
    /// The Target may have created a task but the confirmation was lost and
    /// its registration has no idempotency contract. No automatic
    /// redelivery; resolved via `resolve_uncertain` or a late target accept.
    Uncertain,
}

impl DispatchStatus {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "PendingApproval" => Ok(Self::PendingApproval),
            "Queued" => Ok(Self::Queued),
            "WaitingForTarget" => Ok(Self::WaitingForTarget),
            "Offered" => Ok(Self::Offered),
            "Accepted" => Ok(Self::Accepted),
            "Rejected" => Ok(Self::Rejected),
            "Expired" => Ok(Self::Expired),
            "Canceled" => Ok(Self::Canceled),
            "Uncertain" => Ok(Self::Uncertain),
            _ => Err(RPCErrors::ReasonError(format!(
                "Invalid dispatch status: {}",
                s
            ))),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Accepted | Self::Rejected | Self::Expired | Self::Canceled
        )
    }

    /// States a record may leave toward `Offered` during target evaluation.
    /// `PendingApproval` is deliberately NOT assignable — the approval gate
    /// sits before assignment, so unreleased records never produce offers.
    pub fn is_assignable(&self) -> bool {
        matches!(self, Self::Queued | Self::WaitingForTarget)
    }
}

impl fmt::Display for DispatchStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for DispatchStatus {
    type Err = RPCErrors;
    fn from_str(s: &str) -> Result<Self> {
        DispatchStatus::from_str(s)
    }
}

/// Stable rejection reasons. Capacity shortage / offline instances are *not*
/// rejections — those keep the record in `WaitingForTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchRejectReason {
    SchemaMismatch,
    AuthDenied,
    PolicyDenied,
    PreconditionFailed,
    UnsupportedOperation,
    TargetDisabled,
    InvalidInput,
    /// Admin refused to release a `PendingApproval` record (deny_dispatch).
    ApprovalDenied,
}

impl DispatchRejectReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SchemaMismatch => "schema_mismatch",
            Self::AuthDenied => "auth_denied",
            Self::PolicyDenied => "policy_denied",
            Self::PreconditionFailed => "precondition_failed",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::TargetDisabled => "target_disabled",
            Self::InvalidInput => "invalid_input",
            Self::ApprovalDenied => "approval_denied",
        }
    }

    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "schema_mismatch" => Ok(Self::SchemaMismatch),
            "auth_denied" => Ok(Self::AuthDenied),
            "policy_denied" => Ok(Self::PolicyDenied),
            "precondition_failed" => Ok(Self::PreconditionFailed),
            "unsupported_operation" => Ok(Self::UnsupportedOperation),
            "target_disabled" => Ok(Self::TargetDisabled),
            "invalid_input" => Ok(Self::InvalidInput),
            "approval_denied" => Ok(Self::ApprovalDenied),
            _ => Err(RPCErrors::ReasonError(format!(
                "Invalid dispatch reject reason: {}",
                s
            ))),
        }
    }
}

impl fmt::Display for DispatchRejectReason {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether the Target promises idempotent acceptance for a replayed
/// `dispatch_id`. `IdempotentAccept` is the precondition for automatic offer
/// redelivery; `None`-contract targets fall to `Uncertain` when an offer
/// lease expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdempotencyContract {
    IdempotentAccept,
    None,
}

impl Default for IdempotencyContract {
    fn default() -> Self {
        Self::IdempotentAccept
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstanceSelection {
    RoundRobin,
    LeastLoaded,
}

impl Default for InstanceSelection {
    fn default() -> Self {
        Self::RoundRobin
    }
}

fn default_offer_lease_ms() -> u64 {
    30_000
}

fn default_max_offer_deliveries() -> u32 {
    10
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeliveryPolicy {
    pub offer_lease_ms: u64,
    /// Exhausted -> `Expired` with detail `delivery_exhausted`.
    pub max_offer_deliveries: u32,
    pub instance_selection: InstanceSelection,
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            offer_lease_ms: default_offer_lease_ms(),
            max_offer_deliveries: default_max_offer_deliveries(),
            instance_selection: InstanceSelection::default(),
        }
    }
}

/// Per-target dispatch ACL. Never a replacement for the Target's own
/// business authorization at accept time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchAuthPolicy {
    /// Any authenticated zone user (incl. interactive sessions) may dispatch.
    ZoneUsers,
    /// Only zone-trusted services may dispatch.
    ZoneTrustedOnly,
}

impl Default for DispatchAuthPolicy {
    fn default() -> Self {
        Self::ZoneUsers
    }
}

/// Whose submissions require a manual release before assignment (approval
/// gate, doc §7.1). Judged strictly on the *direct* caller's token grade —
/// the same principle as `on_behalf_of` delegation. `auth_policy` decides
/// first whether a caller may submit at all; this decides whether the
/// accepted submission is held in `PendingApproval`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchApprovalPolicy {
    /// Default: no manual gate, submissions go straight to assignment.
    Never,
    /// Interactive sessions (verify-hub issued, non-sudo) are held;
    /// zone-trusted callers and sudo sessions pass through.
    InteractiveCallers,
    /// Every submission is held — including zone-trusted services and
    /// agent-initiated work ("even the agent's dangerous operations need a
    /// human"). `InteractiveCallers` cannot express that.
    AllCallers,
}

impl Default for DispatchApprovalPolicy {
    fn default() -> Self {
        Self::Never
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    Approved,
    Denied,
}

/// Audit record of a manual release decision. The decider identity comes
/// from the verified token — payload claims are never trusted — and the same
/// decision is also written to `dispatch_event`. Approval never rewrites the
/// auth envelope: release != privilege escalation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchApproval {
    pub decision: ApprovalDecision,
    pub decided_by_user: String,
    pub decided_by_app: String,
    pub decided_at: u64,
    /// Audit/UI note, never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationDescriptor {
    /// Includes the major version, e.g. `"agent.delegate/v1"`.
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema_ref: Option<String>,
}

fn default_max_concurrency() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

/// Persistent registration of a logical execution backend. Owner identity is
/// written by the server from the verified caller token — payload values are
/// ignored on register.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetRegistration {
    pub target_id: String,
    #[serde(default)]
    pub owner_user_id: String,
    #[serde(default)]
    pub owner_app_id: String,
    pub operations: Vec<OperationDescriptor>,
    #[serde(default)]
    pub auth_policy: DispatchAuthPolicy,
    /// Manual-release gate (doc §7.1). `auth_policy` is judged first ("may
    /// this caller submit"), then this ("is the submission held for a
    /// human"). `ZoneTrustedOnly + InteractiveCallers` never triggers —
    /// untrusted callers cannot get through the door at all.
    #[serde(default)]
    pub approval_policy: DispatchApprovalPolicy,
    #[serde(default)]
    pub idempotency_contract: IdempotencyContract,
    #[serde(default)]
    pub delivery_policy: DeliveryPolicy,
    /// Global in-flight offer cap for this target.
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl TargetRegistration {
    pub fn supports_operation(&self, operation: &str) -> bool {
        self.operations
            .iter()
            .any(|descriptor| descriptor.operation == operation)
    }
}

/// Admin-maintained default backend for an operation. Only affects *new*
/// dispatches: records freeze their resolved `target_id` at creation
/// (target stickiness).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRoute {
    pub operation: String,
    pub default_target_id: String,
    pub revision: u64,
    pub enabled: bool,
}

/// A live runtime replica of a target. `lease_epoch` is dispatcher-assigned
/// and strictly increasing per target: stale instances (paused-and-resumed,
/// replayed connections) can never act on new records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetInstance {
    pub target_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
    pub lease_expires_at: u64,
    pub capacity: u32,
    pub available_capacity: u32,
}

/// Audit-only workflow step reference carried in the auth envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStepRef {
    pub workflow_id: String,
    pub run_id: String,
    pub step_id: String,
}

/// How the record's `target_id` was chosen at creation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetSelection {
    Explicit,
    DefaultRoute { route_revision: u64 },
}

/// Immutable request envelope persisted with the record. Every field comes
/// from the *verified* request context — payload identity claims are never
/// trusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchAuthEnvelope {
    pub requested_by_user: String,
    pub requested_by_app: String,
    pub on_behalf_of: String,
    pub zone_trusted_caller: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_ref: Option<WorkflowStepRef>,
    pub input_digest: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchRecord {
    pub dispatch_id: String,
    /// The caller's original choice; `None` means the default route was used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_target_id: Option<String>,
    /// Resolved and frozen at record creation. Immutable afterwards — offer
    /// redelivery, restarts and route updates never rewrite it.
    pub target_id: String,
    pub target_selection: TargetSelection,
    pub operation: String,
    pub status: DispatchStatus,
    pub input: Value,
    pub auth: DispatchAuthEnvelope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer_lease_expires_at: Option<u64>,
    #[serde(default)]
    pub offer_delivery_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_task_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<DispatchRejectReason>,
    /// Manual-release decision — populated only on records that went through
    /// the approval gate. Exempt submissions (zone-trusted / sudo under
    /// `InteractiveCallers`) keep it empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<DispatchApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Caller-side submit params. Identity fields (`on_behalf_of`) are validated
/// against the verified token: normal callers may not impersonate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchRequestParams {
    /// `None` = resolve via the operation's admin-configured default route.
    #[serde(default)]
    pub target_id: Option<String>,
    pub operation: String,
    pub input: Value,
    /// Caller-side idempotency key; `(requested_by_user, requested_by_app,
    /// idempotency_key)` is unique. Replays return the existing record.
    pub idempotency_key: String,
    /// Handoff deadline (unix ms). Not `Accepted` by then -> `Expired`.
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub on_behalf_of: Option<String>,
    #[serde(default)]
    pub workflow_ref: Option<WorkflowStepRef>,
}

/// Sha256 over a canonical (recursively key-sorted) JSON encoding.
pub fn compute_input_digest(input: &Value) -> String {
    let mut hasher = Sha256::new();
    hash_canonical_json(input, &mut hasher);
    format!("{:x}", hasher.finalize())
}

fn hash_canonical_json(value: &Value, hasher: &mut Sha256) {
    match value {
        Value::Object(map) => {
            hasher.update(b"{");
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                hasher.update(key.as_bytes());
                hasher.update(b":");
                hash_canonical_json(&map[key], hasher);
                hasher.update(b",");
            }
            hasher.update(b"}");
        }
        Value::Array(items) => {
            hasher.update(b"[");
            for item in items {
                hash_canonical_json(item, hasher);
                hasher.update(b",");
            }
            hasher.update(b"]");
        }
        other => {
            hasher.update(other.to_string().as_bytes());
        }
    }
}

// ---------------------------------------------------------------------------
// RPC request / result payloads
// ---------------------------------------------------------------------------

macro_rules! impl_from_json {
    ($ty:ty) => {
        impl $ty {
            pub fn from_json(value: Value) -> Result<Self> {
                serde_json::from_value(value).map_err(|e| {
                    RPCErrors::ParseRequestError(format!(
                        concat!("Failed to parse ", stringify!($ty), ": {}"),
                        e
                    ))
                })
            }
        }
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchSubmitResult {
    pub dispatch_id: String,
    pub target_id: String,
    pub target_selection: TargetSelection,
    pub status: DispatchStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDispatchReq {
    pub dispatch_id: String,
}
impl_from_json!(GetDispatchReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDispatchResult {
    pub record: DispatchRecord,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListDispatchesReq {
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub status: Option<DispatchStatus>,
    #[serde(default)]
    pub requested_by_user: Option<String>,
    #[serde(default)]
    pub requested_by_app: Option<String>,
    #[serde(default)]
    pub on_behalf_of: Option<String>,
    /// created_at >= since_ms
    #[serde(default)]
    pub since_ms: Option<u64>,
    /// created_at < until_ms
    #[serde(default)]
    pub until_ms: Option<u64>,
    #[serde(default)]
    pub limit: Option<u32>,
}
impl_from_json!(ListDispatchesReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDispatchesResult {
    pub records: Vec<DispatchRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelDispatchReq {
    pub dispatch_id: String,
}
impl_from_json!(CancelDispatchReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelDispatchResult {
    pub status: DispatchStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterTargetReq {
    pub registration: TargetRegistration,
}
impl_from_json!(RegisterTargetReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisableTargetReq {
    pub target_id: String,
}
impl_from_json!(DisableTargetReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachInstanceReq {
    pub target_id: String,
    #[serde(default = "default_max_concurrency")]
    pub capacity: u32,
}
impl_from_json!(AttachInstanceReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachInstanceResult {
    pub instance_id: String,
    pub lease_epoch: u64,
    pub lease_ttl_ms: u64,
    /// Kevent-safe slug: subscribe `/task_dispatcher/target/{target_key}`.
    pub target_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewInstanceReq {
    pub target_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
    pub available_capacity: u32,
}
impl_from_json!(RenewInstanceReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewInstanceResult {
    pub lease_ttl_ms: u64,
    /// Hint that due records exist for this target/instance — a poll-path
    /// backstop for lost kevents.
    pub has_due: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetachInstanceReq {
    pub target_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
}
impl_from_json!(DetachInstanceReq);

fn default_claim_max() -> u32 {
    16
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimNextReq {
    pub target_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
    #[serde(default = "default_claim_max")]
    pub max: u32,
}
impl_from_json!(ClaimNextReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimNextResult {
    pub records: Vec<DispatchRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptDispatchReq {
    pub dispatch_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
    pub target_task_id: i64,
}
impl_from_json!(AcceptDispatchReq);

/// `accepted == false` means the record had already reached a terminal state
/// (canceled/expired): the target MUST cancel the local task it created for
/// this dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptDispatchResult {
    pub accepted: bool,
    pub status: DispatchStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectDispatchReq {
    pub dispatch_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
    pub reason: DispatchRejectReason,
    #[serde(default)]
    pub detail: Option<String>,
}
impl_from_json!(RejectDispatchReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UncertainResolution {
    Accepted { target_task_id: i64 },
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveUncertainReq {
    pub dispatch_id: String,
    pub resolution: UncertainResolution,
}
impl_from_json!(ResolveUncertainReq);

/// Release a `PendingApproval` record into normal assignment. Idempotent on
/// already-approved records. Never a manual-assignment hook: instance
/// selection stays with `evaluate_target`, and neither the target nor the
/// envelope can be changed here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveDispatchReq {
    pub dispatch_id: String,
    #[serde(default)]
    pub note: Option<String>,
}
impl_from_json!(ApproveDispatchReq);

/// Refuse a `PendingApproval` record: terminal
/// `Rejected(approval_denied)`. Re-submission needs a new idempotency key —
/// a new dispatch, never a revival.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenyDispatchReq {
    pub dispatch_id: String,
    #[serde(default)]
    pub note: Option<String>,
}
impl_from_json!(DenyDispatchReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTargetReq {
    pub target_id: String,
}
impl_from_json!(GetTargetReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTargetResult {
    pub target: TargetRegistration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTargetsResult {
    pub targets: Vec<TargetRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetOperationRouteReq {
    pub operation: String,
    pub default_target_id: String,
}
impl_from_json!(SetOperationRouteReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisableOperationRouteReq {
    pub operation: String,
}
impl_from_json!(DisableOperationRouteReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOperationRouteReq {
    pub operation: String,
}
impl_from_json!(GetOperationRouteReq);

/// `target_valid` = the routed target is currently registered, enabled and
/// still declares the operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRouteView {
    pub route: OperationRoute,
    pub target_valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOperationRouteResult {
    pub route: OperationRouteView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListOperationRoutesResult {
    pub routes: Vec<OperationRouteView>,
}

// ---------------------------------------------------------------------------
// handler trait + server-side rpc dispatch
// ---------------------------------------------------------------------------

#[async_trait]
pub trait TaskDispatcherHandler: Send + Sync {
    // caller side
    async fn handle_dispatch(
        &self,
        params: DispatchRequestParams,
        ctx: RPCContext,
    ) -> Result<DispatchSubmitResult>;
    async fn handle_get_dispatch(
        &self,
        req: GetDispatchReq,
        ctx: RPCContext,
    ) -> Result<DispatchRecord>;
    async fn handle_list_dispatches(
        &self,
        req: ListDispatchesReq,
        ctx: RPCContext,
    ) -> Result<Vec<DispatchRecord>>;
    async fn handle_cancel_dispatch(
        &self,
        req: CancelDispatchReq,
        ctx: RPCContext,
    ) -> Result<CancelDispatchResult>;

    // target side
    async fn handle_register_target(
        &self,
        req: RegisterTargetReq,
        ctx: RPCContext,
    ) -> Result<TargetRegistration>;
    async fn handle_disable_target(&self, req: DisableTargetReq, ctx: RPCContext) -> Result<()>;
    async fn handle_attach_instance(
        &self,
        req: AttachInstanceReq,
        ctx: RPCContext,
    ) -> Result<AttachInstanceResult>;
    async fn handle_renew_instance(
        &self,
        req: RenewInstanceReq,
        ctx: RPCContext,
    ) -> Result<RenewInstanceResult>;
    async fn handle_detach_instance(&self, req: DetachInstanceReq, ctx: RPCContext) -> Result<()>;
    async fn handle_claim_next(
        &self,
        req: ClaimNextReq,
        ctx: RPCContext,
    ) -> Result<Vec<DispatchRecord>>;
    async fn handle_accept_dispatch(
        &self,
        req: AcceptDispatchReq,
        ctx: RPCContext,
    ) -> Result<AcceptDispatchResult>;
    async fn handle_reject_dispatch(&self, req: RejectDispatchReq, ctx: RPCContext) -> Result<()>;

    // admin side
    async fn handle_approve_dispatch(
        &self,
        req: ApproveDispatchReq,
        ctx: RPCContext,
    ) -> Result<DispatchRecord>;
    async fn handle_deny_dispatch(
        &self,
        req: DenyDispatchReq,
        ctx: RPCContext,
    ) -> Result<DispatchRecord>;
    async fn handle_resolve_uncertain(
        &self,
        req: ResolveUncertainReq,
        ctx: RPCContext,
    ) -> Result<DispatchRecord>;
    async fn handle_list_targets(&self, ctx: RPCContext) -> Result<Vec<TargetRegistration>>;
    async fn handle_get_target(
        &self,
        req: GetTargetReq,
        ctx: RPCContext,
    ) -> Result<TargetRegistration>;
    async fn handle_set_operation_route(
        &self,
        req: SetOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<OperationRoute>;
    async fn handle_disable_operation_route(
        &self,
        req: DisableOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<()>;
    async fn handle_get_operation_route(
        &self,
        req: GetOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<OperationRouteView>;
    async fn handle_list_operation_routes(
        &self,
        ctx: RPCContext,
    ) -> Result<Vec<OperationRouteView>>;
}

pub struct TaskDispatcherServerHandler<T: TaskDispatcherHandler>(pub T);

impl<T: TaskDispatcherHandler> TaskDispatcherServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: TaskDispatcherHandler> RPCHandler for TaskDispatcherServerHandler<T> {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            "dispatch" => {
                let params: DispatchRequestParams =
                    serde_json::from_value(req.params).map_err(|e| {
                        RPCErrors::ParseRequestError(format!(
                            "Failed to parse DispatchRequestParams: {}",
                            e
                        ))
                    })?;
                let result = self.0.handle_dispatch(params, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "get_dispatch" => {
                let get_req = GetDispatchReq::from_json(req.params)?;
                let record = self.0.handle_get_dispatch(get_req, ctx).await?;
                RPCResult::Success(json!(GetDispatchResult { record }))
            }
            "list_dispatches" => {
                let list_req = ListDispatchesReq::from_json(req.params)?;
                let records = self.0.handle_list_dispatches(list_req, ctx).await?;
                RPCResult::Success(json!(ListDispatchesResult { records }))
            }
            "cancel_dispatch" => {
                let cancel_req = CancelDispatchReq::from_json(req.params)?;
                let result = self.0.handle_cancel_dispatch(cancel_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "register_target" => {
                let register_req = RegisterTargetReq::from_json(req.params)?;
                let target = self.0.handle_register_target(register_req, ctx).await?;
                RPCResult::Success(json!(GetTargetResult { target }))
            }
            "disable_target" => {
                let disable_req = DisableTargetReq::from_json(req.params)?;
                self.0.handle_disable_target(disable_req, ctx).await?;
                RPCResult::Success(json!(()))
            }
            "attach_instance" => {
                let attach_req = AttachInstanceReq::from_json(req.params)?;
                let result = self.0.handle_attach_instance(attach_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "renew_instance" => {
                let renew_req = RenewInstanceReq::from_json(req.params)?;
                let result = self.0.handle_renew_instance(renew_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "detach_instance" => {
                let detach_req = DetachInstanceReq::from_json(req.params)?;
                self.0.handle_detach_instance(detach_req, ctx).await?;
                RPCResult::Success(json!(()))
            }
            "claim_next" => {
                let claim_req = ClaimNextReq::from_json(req.params)?;
                let records = self.0.handle_claim_next(claim_req, ctx).await?;
                RPCResult::Success(json!(ClaimNextResult { records }))
            }
            "accept_dispatch" => {
                let accept_req = AcceptDispatchReq::from_json(req.params)?;
                let result = self.0.handle_accept_dispatch(accept_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "reject_dispatch" => {
                let reject_req = RejectDispatchReq::from_json(req.params)?;
                self.0.handle_reject_dispatch(reject_req, ctx).await?;
                RPCResult::Success(json!(()))
            }
            "approve_dispatch" => {
                let approve_req = ApproveDispatchReq::from_json(req.params)?;
                let record = self.0.handle_approve_dispatch(approve_req, ctx).await?;
                RPCResult::Success(json!(GetDispatchResult { record }))
            }
            "deny_dispatch" => {
                let deny_req = DenyDispatchReq::from_json(req.params)?;
                let record = self.0.handle_deny_dispatch(deny_req, ctx).await?;
                RPCResult::Success(json!(GetDispatchResult { record }))
            }
            "resolve_uncertain" => {
                let resolve_req = ResolveUncertainReq::from_json(req.params)?;
                let record = self.0.handle_resolve_uncertain(resolve_req, ctx).await?;
                RPCResult::Success(json!(GetDispatchResult { record }))
            }
            "list_targets" => {
                let targets = self.0.handle_list_targets(ctx).await?;
                RPCResult::Success(json!(ListTargetsResult { targets }))
            }
            "get_target" => {
                let get_req = GetTargetReq::from_json(req.params)?;
                let target = self.0.handle_get_target(get_req, ctx).await?;
                RPCResult::Success(json!(GetTargetResult { target }))
            }
            "set_operation_route" => {
                let route_req = SetOperationRouteReq::from_json(req.params)?;
                let route = self.0.handle_set_operation_route(route_req, ctx).await?;
                RPCResult::Success(json!(route))
            }
            "disable_operation_route" => {
                let route_req = DisableOperationRouteReq::from_json(req.params)?;
                self.0
                    .handle_disable_operation_route(route_req, ctx)
                    .await?;
                RPCResult::Success(json!(()))
            }
            "get_operation_route" => {
                let route_req = GetOperationRouteReq::from_json(req.params)?;
                let route = self.0.handle_get_operation_route(route_req, ctx).await?;
                RPCResult::Success(json!(GetOperationRouteResult { route }))
            }
            "list_operation_routes" => {
                let routes = self.0.handle_list_operation_routes(ctx).await?;
                RPCResult::Success(json!(ListOperationRoutesResult { routes }))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

// ---------------------------------------------------------------------------
// client
// ---------------------------------------------------------------------------

/// Client for the Task Dispatch Center.
///
/// `InProcess` exists for tests that inject a fake handler; production code
/// always goes through `KRPC` (and thus server-side token verification).
pub enum TaskDispatcherClient {
    InProcess(Box<dyn TaskDispatcherHandler>),
    KRPC(Box<kRPC>),
}

macro_rules! krpc_call {
    ($client:expr, $method:expr, $req:expr) => {{
        let req_json = serde_json::to_value(&$req)
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to serialize request: {}", e)))?;
        $client.call($method, req_json).await
    }};
}

fn parse_result<T: serde::de::DeserializeOwned>(value: Value, what: &str) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|e| RPCErrors::ParserResponseError(format!("Expected {}: {}", what, e)))
}

impl TaskDispatcherClient {
    pub fn new(client: kRPC) -> Self {
        Self::KRPC(Box::new(client))
    }

    pub fn new_in_process(handler: Box<dyn TaskDispatcherHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub async fn dispatch(&self, params: DispatchRequestParams) -> Result<DispatchSubmitResult> {
        match self {
            Self::InProcess(handler) => {
                handler.handle_dispatch(params, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "dispatch", params)?;
                parse_result(result, "DispatchSubmitResult")
            }
        }
    }

    pub async fn get_dispatch(&self, dispatch_id: &str) -> Result<DispatchRecord> {
        let req = GetDispatchReq {
            dispatch_id: dispatch_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_get_dispatch(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "get_dispatch", req)?;
                let parsed: GetDispatchResult = parse_result(result, "GetDispatchResult")?;
                Ok(parsed.record)
            }
        }
    }

    pub async fn list_dispatches(&self, req: ListDispatchesReq) -> Result<Vec<DispatchRecord>> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_list_dispatches(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "list_dispatches", req)?;
                let parsed: ListDispatchesResult = parse_result(result, "ListDispatchesResult")?;
                Ok(parsed.records)
            }
        }
    }

    pub async fn cancel_dispatch(&self, dispatch_id: &str) -> Result<CancelDispatchResult> {
        let req = CancelDispatchReq {
            dispatch_id: dispatch_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_cancel_dispatch(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "cancel_dispatch", req)?;
                parse_result(result, "CancelDispatchResult")
            }
        }
    }

    pub async fn register_target(
        &self,
        registration: TargetRegistration,
    ) -> Result<TargetRegistration> {
        let req = RegisterTargetReq { registration };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_register_target(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "register_target", req)?;
                let parsed: GetTargetResult = parse_result(result, "GetTargetResult")?;
                Ok(parsed.target)
            }
        }
    }

    pub async fn disable_target(&self, target_id: &str) -> Result<()> {
        let req = DisableTargetReq {
            target_id: target_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_disable_target(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                krpc_call!(client, "disable_target", req)?;
                Ok(())
            }
        }
    }

    pub async fn attach_instance(
        &self,
        target_id: &str,
        capacity: u32,
    ) -> Result<AttachInstanceResult> {
        let req = AttachInstanceReq {
            target_id: target_id.to_string(),
            capacity,
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_attach_instance(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "attach_instance", req)?;
                parse_result(result, "AttachInstanceResult")
            }
        }
    }

    pub async fn renew_instance(&self, req: RenewInstanceReq) -> Result<RenewInstanceResult> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_renew_instance(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "renew_instance", req)?;
                parse_result(result, "RenewInstanceResult")
            }
        }
    }

    pub async fn detach_instance(&self, req: DetachInstanceReq) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_detach_instance(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                krpc_call!(client, "detach_instance", req)?;
                Ok(())
            }
        }
    }

    pub async fn claim_next(&self, req: ClaimNextReq) -> Result<Vec<DispatchRecord>> {
        match self {
            Self::InProcess(handler) => handler.handle_claim_next(req, RPCContext::default()).await,
            Self::KRPC(client) => {
                let result = krpc_call!(client, "claim_next", req)?;
                let parsed: ClaimNextResult = parse_result(result, "ClaimNextResult")?;
                Ok(parsed.records)
            }
        }
    }

    pub async fn accept_dispatch(&self, req: AcceptDispatchReq) -> Result<AcceptDispatchResult> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_accept_dispatch(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "accept_dispatch", req)?;
                parse_result(result, "AcceptDispatchResult")
            }
        }
    }

    pub async fn reject_dispatch(&self, req: RejectDispatchReq) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_reject_dispatch(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                krpc_call!(client, "reject_dispatch", req)?;
                Ok(())
            }
        }
    }

    pub async fn approve_dispatch(
        &self,
        dispatch_id: &str,
        note: Option<String>,
    ) -> Result<DispatchRecord> {
        let req = ApproveDispatchReq {
            dispatch_id: dispatch_id.to_string(),
            note,
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_approve_dispatch(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "approve_dispatch", req)?;
                let parsed: GetDispatchResult = parse_result(result, "GetDispatchResult")?;
                Ok(parsed.record)
            }
        }
    }

    pub async fn deny_dispatch(
        &self,
        dispatch_id: &str,
        note: Option<String>,
    ) -> Result<DispatchRecord> {
        let req = DenyDispatchReq {
            dispatch_id: dispatch_id.to_string(),
            note,
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_deny_dispatch(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "deny_dispatch", req)?;
                let parsed: GetDispatchResult = parse_result(result, "GetDispatchResult")?;
                Ok(parsed.record)
            }
        }
    }

    pub async fn resolve_uncertain(&self, req: ResolveUncertainReq) -> Result<DispatchRecord> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_resolve_uncertain(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "resolve_uncertain", req)?;
                let parsed: GetDispatchResult = parse_result(result, "GetDispatchResult")?;
                Ok(parsed.record)
            }
        }
    }

    pub async fn list_targets(&self) -> Result<Vec<TargetRegistration>> {
        match self {
            Self::InProcess(handler) => handler.handle_list_targets(RPCContext::default()).await,
            Self::KRPC(client) => {
                let result = client.call("list_targets", json!({})).await?;
                let parsed: ListTargetsResult = parse_result(result, "ListTargetsResult")?;
                Ok(parsed.targets)
            }
        }
    }

    pub async fn get_target(&self, target_id: &str) -> Result<TargetRegistration> {
        let req = GetTargetReq {
            target_id: target_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => handler.handle_get_target(req, RPCContext::default()).await,
            Self::KRPC(client) => {
                let result = krpc_call!(client, "get_target", req)?;
                let parsed: GetTargetResult = parse_result(result, "GetTargetResult")?;
                Ok(parsed.target)
            }
        }
    }

    pub async fn set_operation_route(
        &self,
        operation: &str,
        default_target_id: &str,
    ) -> Result<OperationRoute> {
        let req = SetOperationRouteReq {
            operation: operation.to_string(),
            default_target_id: default_target_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_set_operation_route(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "set_operation_route", req)?;
                parse_result(result, "OperationRoute")
            }
        }
    }

    pub async fn disable_operation_route(&self, operation: &str) -> Result<()> {
        let req = DisableOperationRouteReq {
            operation: operation.to_string(),
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_disable_operation_route(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                krpc_call!(client, "disable_operation_route", req)?;
                Ok(())
            }
        }
    }

    pub async fn get_operation_route(&self, operation: &str) -> Result<OperationRouteView> {
        let req = GetOperationRouteReq {
            operation: operation.to_string(),
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_get_operation_route(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = krpc_call!(client, "get_operation_route", req)?;
                let parsed: GetOperationRouteResult =
                    parse_result(result, "GetOperationRouteResult")?;
                Ok(parsed.route)
            }
        }
    }

    pub async fn list_operation_routes(&self) -> Result<Vec<OperationRouteView>> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_list_operation_routes(RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let result = client.call("list_operation_routes", json!({})).await?;
                let parsed: ListOperationRoutesResult =
                    parse_result(result, "ListOperationRoutesResult")?;
                Ok(parsed.routes)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// target-side SDK: attach -> kevent -> claim/accept loop -> renew -> sweep
// ---------------------------------------------------------------------------

/// Business decision for one claimed offer.
pub enum DispatchOfferDecision {
    /// The target re-authorized the envelope and idempotently created (or
    /// looked up) its own business task.
    Accept { target_task_id: i64 },
    /// Stable refusal — terminal for the record.
    Reject {
        reason: DispatchRejectReason,
        detail: Option<String>,
    },
    /// Transient local failure: leave the offer alone; lease expiry drives
    /// redelivery (IdempotentAccept contract) or Uncertain (None contract).
    Defer,
}

/// Implemented by a Dispatch Target. `handle_offer` must follow the receive
/// contract (`task_dispatch_center.md` §6.3): re-authorize first, then
/// idempotently create the owned task, keyed by `record.dispatch_id` in the
/// target's own persistent storage.
#[async_trait]
pub trait DispatchOfferHandler: Send + Sync {
    async fn handle_offer(&self, record: &DispatchRecord) -> DispatchOfferDecision;

    /// Called after a successful accept. Typical use: kick the executor.
    async fn on_accepted(&self, _record: &DispatchRecord, _target_task_id: i64) {}

    /// Called when accept was refused because the record had already reached
    /// `Canceled` / `Expired`: the target must cancel the local task it
    /// created for this dispatch.
    async fn on_dispatch_gone(&self, record: &DispatchRecord, target_task_id: i64) {
        warn!(
            "dispatch {} is gone (target task {} should be canceled by the target)",
            record.dispatch_id, target_task_id
        );
    }
}

#[derive(Debug, Clone)]
pub struct TargetInstanceConfig {
    pub target_id: String,
    /// In-flight offer capacity reported to the dispatcher.
    pub capacity: u32,
    pub claim_batch: u32,
    /// Poll fallback when kevent is silent. Notifications only accelerate;
    /// this sweep and the notification path share one claim entrypoint.
    pub fallback_poll_ms: u64,
}

impl TargetInstanceConfig {
    pub fn new(target_id: impl Into<String>, capacity: u32) -> Self {
        Self {
            target_id: target_id.into(),
            capacity,
            claim_batch: 8,
            fallback_poll_ms: 30_000,
        }
    }
}

async fn acquire_runtime_task_dispatcher_client() -> Result<TaskDispatcherClient> {
    crate::get_buckyos_api_runtime()?
        .get_task_dispatcher_client()
        .await
}

/// Run one target instance until `shutdown` fires: attach, subscribe the
/// target kevent channel, claim/accept in a loop, renew the lease, detach on
/// the way out. Re-attaches (new epoch) if the dispatcher declares this
/// instance stale. Every dispatcher RPC acquires a short-session client from
/// the registered runtime. The business side only implements
/// [`DispatchOfferHandler`].
pub async fn run_target_instance(
    config: TargetInstanceConfig,
    kevent: Option<KEventClient>,
    shutdown: Arc<tokio::sync::Notify>,
    handler: Arc<dyn DispatchOfferHandler>,
) -> Result<()> {
    loop {
        let attach_result = match acquire_runtime_task_dispatcher_client().await {
            Ok(client) => {
                client
                    .attach_instance(config.target_id.as_str(), config.capacity)
                    .await
            }
            Err(err) => Err(err),
        };
        let attach = match attach_result {
            Ok(attach) => attach,
            Err(err) => {
                warn!(
                    "task_dispatcher.run_target_instance[{}]: attach failed: {}; retrying",
                    config.target_id, err
                );
                tokio::select! {
                    _ = shutdown.notified() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                }
            }
        };
        info!(
            "task_dispatcher.run_target_instance[{}]: attached instance={} epoch={}",
            config.target_id, attach.instance_id, attach.lease_epoch
        );

        // The reader holds a weak ref to the kevent client — keep the client
        // alive for the lifetime of the loop iteration.
        let reader = match kevent.as_ref() {
            Some(kevent_client) => {
                let channel = task_dispatcher_target_event_id(attach.target_key.as_str());
                match kevent_client
                    .create_event_reader(vec![channel.clone()])
                    .await
                {
                    Ok(reader) => Some(reader),
                    Err(err) => {
                        warn!(
                            "task_dispatcher.run_target_instance[{}]: subscribe {} failed: {}; polling only",
                            config.target_id, channel, err
                        );
                        None
                    }
                }
            }
            None => None,
        };

        match run_attached_instance(&config, &attach, reader.as_ref(), &shutdown, &handler).await {
            InstanceLoopExit::Shutdown => {
                if let Ok(client) = acquire_runtime_task_dispatcher_client().await {
                    let _ = client
                        .detach_instance(DetachInstanceReq {
                            target_id: config.target_id.clone(),
                            instance_id: attach.instance_id.clone(),
                            lease_epoch: attach.lease_epoch,
                        })
                        .await;
                }
                return Ok(());
            }
            InstanceLoopExit::Stale => {
                info!(
                    "task_dispatcher.run_target_instance[{}]: instance {} became stale; re-attaching",
                    config.target_id, attach.instance_id
                );
                continue;
            }
        }
    }
}

enum InstanceLoopExit {
    Shutdown,
    Stale,
}

async fn run_attached_instance(
    config: &TargetInstanceConfig,
    attach: &AttachInstanceResult,
    reader: Option<&crate::kevent_client::EventReader>,
    shutdown: &tokio::sync::Notify,
    handler: &Arc<dyn DispatchOfferHandler>,
) -> InstanceLoopExit {
    let renew_every = Duration::from_millis((attach.lease_ttl_ms / 3).max(1_000));
    let mut renew_interval = tokio::time::interval(renew_every);
    renew_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // First tick fires immediately; consume it so renew starts one period in.
    renew_interval.tick().await;

    // Initial drain covers records queued while we were offline.
    if let DrainOutcome::Stale = drain_offers(config, attach, handler).await {
        return InstanceLoopExit::Stale;
    }

    loop {
        let wake = async {
            match reader {
                // Notification and poll-timeout deliberately converge on the
                // same claim path below (kevent is acceleration, not truth).
                Some(reader) => {
                    let _ = reader.pull_event(Some(config.fallback_poll_ms)).await;
                }
                None => tokio::time::sleep(Duration::from_millis(config.fallback_poll_ms)).await,
            }
        };
        tokio::select! {
            _ = shutdown.notified() => return InstanceLoopExit::Shutdown,
            _ = renew_interval.tick() => {
                let renew = match acquire_runtime_task_dispatcher_client().await {
                    Ok(client) => client
                        .renew_instance(RenewInstanceReq {
                            target_id: config.target_id.clone(),
                            instance_id: attach.instance_id.clone(),
                            lease_epoch: attach.lease_epoch,
                            available_capacity: config.capacity,
                        })
                        .await,
                    Err(err) => Err(err),
                };
                match renew {
                    Ok(result) => {
                        if result.has_due {
                            if let DrainOutcome::Stale =
                                drain_offers(config, attach, handler).await
                            {
                                return InstanceLoopExit::Stale;
                            }
                        }
                    }
                    Err(err) if is_stale_instance_err(&err) => return InstanceLoopExit::Stale,
                    Err(err) => {
                        warn!(
                            "task_dispatcher.run_target_instance[{}]: renew failed: {}",
                            config.target_id, err
                        );
                    }
                }
            }
            _ = wake => {
                if let DrainOutcome::Stale = drain_offers(config, attach, handler).await {
                    return InstanceLoopExit::Stale;
                }
            }
        }
    }
}

enum DrainOutcome {
    Ok,
    Stale,
}

async fn drain_offers(
    config: &TargetInstanceConfig,
    attach: &AttachInstanceResult,
    handler: &Arc<dyn DispatchOfferHandler>,
) -> DrainOutcome {
    loop {
        let records_result = match acquire_runtime_task_dispatcher_client().await {
            Ok(client) => {
                client
                    .claim_next(ClaimNextReq {
                        target_id: config.target_id.clone(),
                        instance_id: attach.instance_id.clone(),
                        lease_epoch: attach.lease_epoch,
                        max: config.claim_batch,
                    })
                    .await
            }
            Err(err) => Err(err),
        };
        let records = match records_result {
            Ok(records) => records,
            Err(err) if is_stale_instance_err(&err) => return DrainOutcome::Stale,
            Err(err) => {
                warn!(
                    "task_dispatcher.drain_offers[{}]: claim_next failed: {}",
                    config.target_id, err
                );
                return DrainOutcome::Ok;
            }
        };
        if records.is_empty() {
            return DrainOutcome::Ok;
        }

        let mut progressed = false;
        for record in &records {
            match handler.handle_offer(record).await {
                DispatchOfferDecision::Accept { target_task_id } => {
                    progressed = true;
                    let accept = match acquire_runtime_task_dispatcher_client().await {
                        Ok(client) => {
                            client
                                .accept_dispatch(AcceptDispatchReq {
                                    dispatch_id: record.dispatch_id.clone(),
                                    instance_id: attach.instance_id.clone(),
                                    lease_epoch: attach.lease_epoch,
                                    target_task_id,
                                })
                                .await
                        }
                        Err(err) => Err(err),
                    };
                    match accept {
                        Ok(result) if result.accepted => {
                            handler.on_accepted(record, target_task_id).await;
                        }
                        Ok(result) => {
                            warn!(
                                "task_dispatcher.drain_offers[{}]: dispatch {} not accepted (status={})",
                                config.target_id, record.dispatch_id, result.status
                            );
                            handler.on_dispatch_gone(record, target_task_id).await;
                        }
                        Err(err) if is_stale_instance_err(&err) => return DrainOutcome::Stale,
                        Err(err) => {
                            warn!(
                                "task_dispatcher.drain_offers[{}]: accept {} failed: {}",
                                config.target_id, record.dispatch_id, err
                            );
                        }
                    }
                }
                DispatchOfferDecision::Reject { reason, detail } => {
                    progressed = true;
                    let reject = match acquire_runtime_task_dispatcher_client().await {
                        Ok(client) => {
                            client
                                .reject_dispatch(RejectDispatchReq {
                                    dispatch_id: record.dispatch_id.clone(),
                                    instance_id: attach.instance_id.clone(),
                                    lease_epoch: attach.lease_epoch,
                                    reason,
                                    detail,
                                })
                                .await
                        }
                        Err(err) => Err(err),
                    };
                    match reject {
                        Err(err) if is_stale_instance_err(&err) => return DrainOutcome::Stale,
                        Err(err) => {
                            warn!(
                                "task_dispatcher.drain_offers[{}]: reject {} failed: {}",
                                config.target_id, record.dispatch_id, err
                            );
                        }
                        Ok(()) => {}
                    }
                }
                DispatchOfferDecision::Defer => {}
            }
        }

        // Deferred offers come back on every claim within the same lease —
        // without progress this would spin, so wait for the next wake-up.
        if !progressed {
            return DrainOutcome::Ok;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_key_is_kevent_safe_and_stable() {
        let key = task_dispatcher_target_key("did:agent:jarvis");
        assert!(key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.'));
        assert_eq!(key, task_dispatcher_target_key("did:agent:jarvis"));
        // Ids differing only in stripped chars must not collide.
        assert_ne!(
            task_dispatcher_target_key("did:agent:jarvis"),
            task_dispatcher_target_key("did.agent.jarvis")
        );
        crate::kevent_client::validate_eventid(&task_dispatcher_target_event_id(&key)).unwrap();
        crate::kevent_client::validate_eventid(&task_dispatcher_approvals_event_id()).unwrap();
    }

    #[test]
    fn input_digest_is_key_order_independent() {
        let a = json!({"b": 1, "a": {"y": [1, 2], "x": "s"}});
        let b = json!({"a": {"x": "s", "y": [1, 2]}, "b": 1});
        assert_eq!(compute_input_digest(&a), compute_input_digest(&b));
        assert_ne!(
            compute_input_digest(&a),
            compute_input_digest(&json!({"b": 2, "a": {"y": [1, 2], "x": "s"}}))
        );
    }

    #[test]
    fn dispatch_status_round_trip() {
        for status in [
            DispatchStatus::PendingApproval,
            DispatchStatus::Queued,
            DispatchStatus::WaitingForTarget,
            DispatchStatus::Offered,
            DispatchStatus::Accepted,
            DispatchStatus::Rejected,
            DispatchStatus::Expired,
            DispatchStatus::Canceled,
            DispatchStatus::Uncertain,
        ] {
            assert_eq!(
                DispatchStatus::from_str(status.to_string().as_str()).unwrap(),
                status
            );
        }
        assert!(DispatchStatus::Accepted.is_terminal());
        assert!(!DispatchStatus::Uncertain.is_terminal());
        // The approval gate sits before assignment: not terminal, and never
        // eligible for evaluate_target.
        assert!(!DispatchStatus::PendingApproval.is_terminal());
        assert!(!DispatchStatus::PendingApproval.is_assignable());
        assert_eq!(
            DispatchRejectReason::from_str("approval_denied").unwrap(),
            DispatchRejectReason::ApprovalDenied
        );
    }

    #[test]
    fn target_selection_serde_shape() {
        assert_eq!(
            serde_json::to_value(TargetSelection::Explicit).unwrap(),
            json!("Explicit")
        );
        assert_eq!(
            serde_json::to_value(TargetSelection::DefaultRoute { route_revision: 12 }).unwrap(),
            json!({"DefaultRoute": {"route_revision": 12}})
        );
    }

    #[test]
    fn registration_defaults() {
        let registration: TargetRegistration = serde_json::from_value(json!({
            "target_id": "did:agent:jarvis",
            "operations": [{"operation": "agent.delegate/v1"}]
        }))
        .unwrap();
        assert_eq!(registration.auth_policy, DispatchAuthPolicy::ZoneUsers);
        // No manual gate unless the target opts in.
        assert_eq!(registration.approval_policy, DispatchApprovalPolicy::Never);
        assert_eq!(
            registration.idempotency_contract,
            IdempotencyContract::IdempotentAccept
        );
        assert_eq!(registration.delivery_policy.offer_lease_ms, 30_000);
        assert_eq!(registration.delivery_policy.max_offer_deliveries, 10);
        assert_eq!(registration.max_concurrency, 1);
        assert!(registration.enabled);
        assert!(registration.supports_operation("agent.delegate/v1"));
        assert!(!registration.supports_operation("agent.delegate/v2"));
    }
}
