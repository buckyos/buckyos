//! Task Dispatch Center 2.0 shared protocol layer.
//!
//! Design: `doc/task_mgr/task-mgr 2.0.md` §11. The Dispatch Center is the
//! internal durable-queue module of the TaskMgr 2.0 subsystem: it freezes a
//! DispatchPlan, creates the single public Task up front (via the Task Core
//! trusted control plane), then mechanically delivers work to registered
//! runner functions with `offer_task` / `activate_task` push RPCs.
//!
//! beta2.2 breaking change: the 1.x `dispatch -> target_task_id` two-object
//! handoff, `claim_next` polling and `Uncertain` resolution protocol are
//! removed without a compatibility layer.

use ::kRPC::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use crate::rdb_mgr::{RdbBackend, RdbInstanceConfig};
use crate::task_mgr::{Task, TaskId};

pub const TASK_DISPATCHER_SERVICE_NAME: &str = "task-dispatcher";
/// The dispatcher shares the task-manager process and port; it is mounted at
/// its own kapi path with an independent store and authorization policy.
pub const TASK_DISPATCHER_SERVICE_PORT: u16 = 3380;

pub const TASK_DISPATCHER_RDB_INSTANCE_ID: &str = "task-dispatcher-main";

/// Dispatcher durable schema version. v3 is the TaskMgr 2.0 delivery model:
/// records reference a pre-created public task, queue state is explicit and
/// stably ordered, and every RPC is journaled as a DeliveryAttempt.
pub const TASK_DISPATCHER_RDB_SCHEMA_VERSION: u64 = 3;

pub const TASK_DISPATCHER_RDB_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS dispatch_record (
    dispatch_id          TEXT PRIMARY KEY,
    idempotency_key      TEXT NOT NULL,
    requested_by_user    TEXT NOT NULL,
    requested_by_app     TEXT NOT NULL,
    schema_id            TEXT NOT NULL,
    schema_version       INTEGER NOT NULL,
    requested_target_id  TEXT,
    target_id            TEXT NOT NULL,
    target_selection_json TEXT NOT NULL,
    registration_revision INTEGER NOT NULL,
    delivery_policy_json TEXT NOT NULL,
    status               TEXT NOT NULL,
    task_id              TEXT,
    input_json           TEXT NOT NULL,
    input_digest         TEXT NOT NULL,
    auth_json            TEXT NOT NULL,
    priority             INTEGER NOT NULL DEFAULT 0,
    ready_at             INTEGER NOT NULL DEFAULT 0,
    attempt_count        INTEGER NOT NULL DEFAULT 0,
    reject_reason        TEXT,
    approval_json        TEXT,
    message              TEXT,
    expires_at           INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_dispatch_idempotency ON dispatch_record(requested_by_user, requested_by_app, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_dispatch_queue ON dispatch_record(status, ready_at, priority, created_at, dispatch_id);
CREATE INDEX IF NOT EXISTS idx_dispatch_task ON dispatch_record(task_id);
CREATE INDEX IF NOT EXISTS idx_dispatch_target_status ON dispatch_record(target_id, status);

CREATE TABLE IF NOT EXISTS delivery_attempt (
    dispatch_id       TEXT NOT NULL,
    attempt_no        INTEGER NOT NULL,
    delivery_id       TEXT NOT NULL,
    lease_epoch       INTEGER NOT NULL,
    target_id         TEXT NOT NULL,
    instance_id       TEXT NOT NULL,
    endpoint          TEXT NOT NULL,
    stage             TEXT NOT NULL,
    outcome           TEXT,
    outcome_detail    TEXT,
    reservation_token TEXT,
    runner_epoch      INTEGER,
    deadline_at       INTEGER NOT NULL,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    PRIMARY KEY (dispatch_id, attempt_no)
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_delivery_id ON delivery_attempt(delivery_id);

CREATE TABLE IF NOT EXISTS runner_registration (
    target_id             TEXT PRIMARY KEY,
    owner_user_id         TEXT NOT NULL,
    owner_app_id          TEXT NOT NULL,
    registration_revision INTEGER NOT NULL,
    registration_json     TEXT NOT NULL,
    enabled               INTEGER NOT NULL DEFAULT 1,
    last_lease_epoch      INTEGER NOT NULL DEFAULT 0,
    updated_at            INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS operation_route (
    schema_id         TEXT PRIMARY KEY,
    default_target_id TEXT NOT NULL,
    revision          INTEGER NOT NULL,
    enabled           INTEGER NOT NULL DEFAULT 1,
    updated_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS target_instance (
    target_id          TEXT NOT NULL,
    instance_id        TEXT NOT NULL,
    endpoint           TEXT NOT NULL,
    lease_epoch        INTEGER NOT NULL,
    lease_expires_at   INTEGER NOT NULL,
    capacity           INTEGER NOT NULL,
    available_capacity INTEGER NOT NULL,
    attached_at        INTEGER NOT NULL,
    PRIMARY KEY (target_id, instance_id)
);

CREATE TABLE IF NOT EXISTS dispatch_cursor (
    target_id          TEXT PRIMARY KEY,
    cursor_instance_id TEXT,
    updated_at         INTEGER NOT NULL
);
"#;

pub const TASK_DISPATCHER_RDB_SCHEMA_POSTGRES: &str = r#"
CREATE TABLE IF NOT EXISTS dispatch_record (
    dispatch_id          TEXT PRIMARY KEY,
    idempotency_key      TEXT NOT NULL,
    requested_by_user    TEXT NOT NULL,
    requested_by_app     TEXT NOT NULL,
    schema_id            TEXT NOT NULL,
    schema_version       BIGINT NOT NULL,
    requested_target_id  TEXT,
    target_id            TEXT NOT NULL,
    target_selection_json TEXT NOT NULL,
    registration_revision BIGINT NOT NULL,
    delivery_policy_json TEXT NOT NULL,
    status               TEXT NOT NULL,
    task_id              TEXT,
    input_json           TEXT NOT NULL,
    input_digest         TEXT NOT NULL,
    auth_json            TEXT NOT NULL,
    priority             BIGINT NOT NULL DEFAULT 0,
    ready_at             BIGINT NOT NULL DEFAULT 0,
    attempt_count        BIGINT NOT NULL DEFAULT 0,
    reject_reason        TEXT,
    approval_json        TEXT,
    message              TEXT,
    expires_at           BIGINT,
    created_at           BIGINT NOT NULL,
    updated_at           BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_dispatch_idempotency ON dispatch_record(requested_by_user, requested_by_app, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_dispatch_queue ON dispatch_record(status, ready_at, priority, created_at, dispatch_id);
CREATE INDEX IF NOT EXISTS idx_dispatch_task ON dispatch_record(task_id);
CREATE INDEX IF NOT EXISTS idx_dispatch_target_status ON dispatch_record(target_id, status);

CREATE TABLE IF NOT EXISTS delivery_attempt (
    dispatch_id       TEXT NOT NULL,
    attempt_no        BIGINT NOT NULL,
    delivery_id       TEXT NOT NULL,
    lease_epoch       BIGINT NOT NULL,
    target_id         TEXT NOT NULL,
    instance_id       TEXT NOT NULL,
    endpoint          TEXT NOT NULL,
    stage             TEXT NOT NULL,
    outcome           TEXT,
    outcome_detail    TEXT,
    reservation_token TEXT,
    runner_epoch      BIGINT,
    deadline_at       BIGINT NOT NULL,
    created_at        BIGINT NOT NULL,
    updated_at        BIGINT NOT NULL,
    PRIMARY KEY (dispatch_id, attempt_no)
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_delivery_id ON delivery_attempt(delivery_id);

CREATE TABLE IF NOT EXISTS runner_registration (
    target_id             TEXT PRIMARY KEY,
    owner_user_id         TEXT NOT NULL,
    owner_app_id          TEXT NOT NULL,
    registration_revision BIGINT NOT NULL,
    registration_json     TEXT NOT NULL,
    enabled               BIGINT NOT NULL DEFAULT 1,
    last_lease_epoch      BIGINT NOT NULL DEFAULT 0,
    updated_at            BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS operation_route (
    schema_id         TEXT PRIMARY KEY,
    default_target_id TEXT NOT NULL,
    revision          BIGINT NOT NULL,
    enabled           BIGINT NOT NULL DEFAULT 1,
    updated_at        BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS target_instance (
    target_id          TEXT NOT NULL,
    instance_id        TEXT NOT NULL,
    endpoint           TEXT NOT NULL,
    lease_epoch        BIGINT NOT NULL,
    lease_expires_at   BIGINT NOT NULL,
    capacity           BIGINT NOT NULL,
    available_capacity BIGINT NOT NULL,
    attached_at        BIGINT NOT NULL,
    PRIMARY KEY (target_id, instance_id)
);

CREATE TABLE IF NOT EXISTS dispatch_cursor (
    target_id          TEXT PRIMARY KEY,
    cursor_instance_id TEXT,
    updated_at         BIGINT NOT NULL
);
"#;

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
        connection: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Stable dispatch error codes
// ---------------------------------------------------------------------------

pub const DISPATCH_ERR_DEFAULT_TARGET_NOT_CONFIGURED: &str = "default_target_not_configured";
pub const DISPATCH_ERR_IDEMPOTENCY_CONFLICT: &str = "idempotency_conflict";
pub const DISPATCH_ERR_TARGET_NOT_REGISTERED: &str = "target_not_registered";
pub const DISPATCH_ERR_TARGET_DISABLED: &str = "target_disabled";
pub const DISPATCH_ERR_UNSUPPORTED_SCHEMA: &str = "unsupported_schema";
pub const DISPATCH_ERR_STALE_INSTANCE: &str = "stale_instance";
pub const DISPATCH_ERR_ALREADY_TERMINAL: &str = "dispatch_already_terminal";
pub const DISPATCH_ERR_PENDING_APPROVAL: &str = "dispatch_pending_approval";
pub const DISPATCH_ERR_NOT_PENDING_APPROVAL: &str = "dispatch_not_pending_approval";
pub const DISPATCH_ERR_APPROVAL_DENIED: &str = "dispatch_approval_denied";
pub const DISPATCH_ERR_DELIVERY_EXHAUSTED: &str = "delivery_exhausted";
pub const DISPATCH_ERR_RUNNER_REJECTED: &str = "runner_rejected";

pub fn is_stale_instance_err(err: &RPCErrors) -> bool {
    match err {
        RPCErrors::ReasonError(msg) | RPCErrors::NoPermission(msg) => {
            msg.contains(DISPATCH_ERR_STALE_INSTANCE)
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// kevent channels
// ---------------------------------------------------------------------------

/// Per-target notification channel: an "instances or capacity may be due"
/// wake-up hint for the dispatcher evaluation loop and runner instances.
pub fn task_dispatcher_target_event_id(target_key: &str) -> String {
    format!("/task_dispatcher/target/{}", target_key)
}

/// Per-record status-change channel for callers.
pub fn task_dispatcher_record_event_id(dispatch_id: &str) -> String {
    format!("/task_dispatcher/{}", dispatch_id)
}

/// Admin-facing "new record awaits approval" hint channel. Acceleration
/// only: the authoritative queue is always
/// `list_dispatches(status=PendingApproval)`.
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
// Core model
// ---------------------------------------------------------------------------

/// Mechanical delivery protocol states (doc §11.3). This is a transport state
/// machine, never a business one: after `Accepted` the public Task's
/// Running/Waiting/Terminal is not mirrored back into the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchStatus {
    /// Record persisted; the idempotent Task Core `create_promised_task`
    /// call has not been confirmed yet (crash-recovery resumes here).
    CreatingTask,
    /// Manual-release gate before queueing: never evaluated, never offered.
    PendingApproval,
    Queued,
    /// Durable queue with no online instance / capacity; reason kept in the
    /// record. Not a second queue.
    WaitingForTarget,
    /// A DeliveryAttempt is persisted and the offer RPC is in flight or its
    /// outcome is still uncertain.
    Offering,
    /// Runner reserved capacity; dispatcher is idempotently binding the Task
    /// executor.
    Binding,
    /// Task bound; dispatcher is making sure the activate call lands.
    Activating,
    /// Mechanical-delivery absorbing state: handoff complete.
    Accepted,
    Rejected,
    Failed,
    Canceled,
    Expired,
}

impl DispatchStatus {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "CreatingTask" => Ok(Self::CreatingTask),
            "PendingApproval" => Ok(Self::PendingApproval),
            "Queued" => Ok(Self::Queued),
            "WaitingForTarget" => Ok(Self::WaitingForTarget),
            "Offering" => Ok(Self::Offering),
            "Binding" => Ok(Self::Binding),
            "Activating" => Ok(Self::Activating),
            "Accepted" => Ok(Self::Accepted),
            "Rejected" => Ok(Self::Rejected),
            "Failed" => Ok(Self::Failed),
            "Canceled" => Ok(Self::Canceled),
            "Expired" => Ok(Self::Expired),
            _ => Err(RPCErrors::ReasonError(format!(
                "Invalid dispatch status: {}",
                s
            ))),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Accepted | Self::Rejected | Self::Failed | Self::Canceled | Self::Expired
        )
    }

    /// States from which the evaluation loop may start a new offer.
    pub fn is_assignable(&self) -> bool {
        matches!(self, Self::Queued | Self::WaitingForTarget)
    }

    /// States where the record still owns the promised task and may cancel
    /// it atomically with its own queue state.
    pub fn owns_promised_task(&self) -> bool {
        matches!(
            self,
            Self::CreatingTask
                | Self::PendingApproval
                | Self::Queued
                | Self::WaitingForTarget
                | Self::Offering
        )
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
    /// Admin refused to release a `PendingApproval` record.
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

fn default_rpc_timeout_ms() -> u64 {
    15_000
}

fn default_backoff_base_ms() -> u64 {
    2_000
}

fn default_backoff_max_ms() -> u64 {
    300_000
}

fn default_max_attempts() -> u32 {
    10
}

/// Frozen per-record delivery policy (doc §11.2). Copied into the record at
/// creation time; later registration edits never change existing records.
/// Jitter, when needed, must be derived deterministically from
/// `hash(dispatch_id, attempt_no)` — never from process randomness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeliveryPolicy {
    /// Per-RPC (offer/activate) deadline.
    pub rpc_timeout_ms: u64,
    /// Exponential backoff base for requeue after Busy/transport failures.
    pub backoff_base_ms: u64,
    pub backoff_max_ms: u64,
    /// Attempt budget; exhausted -> record `Expired`, task `Terminal/Failed`.
    pub max_attempts: u32,
    pub instance_selection: InstanceSelection,
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            rpc_timeout_ms: default_rpc_timeout_ms(),
            backoff_base_ms: default_backoff_base_ms(),
            backoff_max_ms: default_backoff_max_ms(),
            max_attempts: default_max_attempts(),
            instance_selection: InstanceSelection::default(),
        }
    }
}

impl DeliveryPolicy {
    /// Deterministic requeue delay for `attempt_no` (1-based), including the
    /// hash-derived jitter required by the determinism contract.
    pub fn backoff_ms(&self, dispatch_id: &str, attempt_no: u32) -> u64 {
        let exp = attempt_no.saturating_sub(1).min(16);
        let base = self.backoff_base_ms.saturating_mul(1u64 << exp);
        let capped = base.min(self.backoff_max_ms);
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in dispatch_id.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= attempt_no as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        // Up to +25% deterministic jitter.
        let jitter = (capped / 4).saturating_mul(hash % 1000) / 1000;
        capped.saturating_add(jitter)
    }
}

/// Per-target dispatch ACL. Never a replacement for the Runner's own
/// business authorization at offer time.
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

/// Whose submissions require a manual release before queueing (doc §11.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchApprovalPolicy {
    Never,
    /// Interactive sessions (verify-hub issued, non-sudo) are held;
    /// zone-trusted callers and sudo sessions pass through.
    InteractiveCallers,
    /// Every submission is held.
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

/// Audit record of a manual release decision. Approval never rewrites the
/// auth envelope: release != privilege escalation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchApproval {
    pub decision: ApprovalDecision,
    pub decided_by_user: String,
    pub decided_by_app: String,
    pub decided_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn default_offer_method() -> String {
    "offer_task".to_string()
}

fn default_activate_method() -> String {
    "activate_task".to_string()
}

/// One delivery function a Target exposes for a task schema (doc §9.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerFunctionDescriptor {
    /// Includes the major version, e.g. `agent.command/v1`. This is also the
    /// operation-route key.
    pub schema_id: String,
    /// Inclusive supported registry-revision range within the major version.
    #[serde(default)]
    pub min_schema_version: u32,
    #[serde(default = "u32_max")]
    pub max_schema_version: u32,
    #[serde(default = "default_offer_method")]
    pub offer_method: String,
    #[serde(default = "default_activate_method")]
    pub activate_method: String,
}

fn u32_max() -> u32 {
    u32::MAX
}

impl RunnerFunctionDescriptor {
    pub fn new(schema_id: impl Into<String>) -> Self {
        Self {
            schema_id: schema_id.into(),
            min_schema_version: 0,
            max_schema_version: u32::MAX,
            offer_method: default_offer_method(),
            activate_method: default_activate_method(),
        }
    }

    pub fn supports_version(&self, version: u32) -> bool {
        version >= self.min_schema_version && version <= self.max_schema_version
    }
}

fn default_max_concurrency() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

/// Versioned registration of a logical execution backend and its delivery
/// functions. Owner identity is written by the server from the verified
/// caller token — payload values are ignored on register. Every accepted
/// update increments `registration_revision`; dispatch records freeze the
/// revision they were routed with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetRegistration {
    pub target_id: String,
    #[serde(default)]
    pub owner_user_id: String,
    #[serde(default)]
    pub owner_app_id: String,
    /// The runner functions this target serves, keyed by schema id.
    pub functions: Vec<RunnerFunctionDescriptor>,
    #[serde(default)]
    pub auth_policy: DispatchAuthPolicy,
    #[serde(default)]
    pub approval_policy: DispatchApprovalPolicy,
    #[serde(default)]
    pub delivery_policy: DeliveryPolicy,
    /// Global in-flight offer cap for this target.
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Server-assigned; incremented on every accepted registration update.
    #[serde(default)]
    pub registration_revision: u64,
}

impl TargetRegistration {
    pub fn function_for(&self, schema_id: &str, version: u32) -> Option<&RunnerFunctionDescriptor> {
        self.functions
            .iter()
            .find(|f| f.schema_id == schema_id && f.supports_version(version))
    }

    pub fn supports_schema(&self, schema_id: &str) -> bool {
        self.functions.iter().any(|f| f.schema_id == schema_id)
    }
}

/// Admin-maintained default backend for a schema id. Only affects *new*
/// dispatches: records freeze their resolved `target_id` at creation
/// (target stickiness).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRoute {
    pub schema_id: String,
    pub default_target_id: String,
    pub revision: u64,
    pub enabled: bool,
}

/// A live runtime replica of a target. `lease_epoch` is dispatcher-assigned
/// and strictly increasing per target: stale instances can never act on new
/// delivery attempts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetInstance {
    pub target_id: String,
    pub instance_id: String,
    /// kRPC endpoint URL the dispatcher uses for offer/activate push calls.
    pub endpoint: String,
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

/// The frozen mechanical delivery plan (doc §11.1). Either supplied by the
/// caller/Scheduler or resolved from versioned route config at accept time,
/// then immutable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchPlan {
    pub schema_id: String,
    pub schema_version: u32,
    pub target_id: String,
    pub target_config_revision: u64,
    pub delivery_policy: DeliveryPolicy,
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
    pub schema_id: String,
    pub schema_version: u32,
    /// Registration revision frozen when the record was routed.
    pub registration_revision: u64,
    /// Frozen per-record policy snapshot.
    pub delivery_policy: DeliveryPolicy,
    pub status: DispatchStatus,
    /// The single public Task; backfilled right after the idempotent
    /// `create_promised_task` confirms. `None` only in `CreatingTask`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    pub input: Value,
    pub auth: DispatchAuthEnvelope,
    #[serde(default)]
    pub priority: i64,
    /// Queue readiness time (unix ms); backoff pushes it into the future.
    #[serde(default)]
    pub ready_at: u64,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<DispatchRejectReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<DispatchApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Stages of a persisted DeliveryAttempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStage {
    Offer,
    Activate,
}

impl fmt::Display for DeliveryStage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for DeliveryStage {
    type Err = RPCErrors;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "Offer" => Ok(Self::Offer),
            "Activate" => Ok(Self::Activate),
            _ => Err(RPCErrors::ReasonError(format!(
                "Invalid delivery stage: {}",
                s
            ))),
        }
    }
}

/// Journaled outcome of one delivery RPC. `None` while in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryOutcome {
    OfferAccepted,
    Busy,
    Rejected,
    TransportError,
    Timeout,
    Activated,
    /// Fenced by a newer lease epoch before completing.
    Superseded,
}

impl fmt::Display for DeliveryOutcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for DeliveryOutcome {
    type Err = RPCErrors;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "OfferAccepted" => Ok(Self::OfferAccepted),
            "Busy" => Ok(Self::Busy),
            "Rejected" => Ok(Self::Rejected),
            "TransportError" => Ok(Self::TransportError),
            "Timeout" => Ok(Self::Timeout),
            "Activated" => Ok(Self::Activated),
            "Superseded" => Ok(Self::Superseded),
            _ => Err(RPCErrors::ReasonError(format!(
                "Invalid delivery outcome: {}",
                s
            ))),
        }
    }
}

/// One persisted delivery RPC: written *before* the call, updated with the
/// ordered outcome afterwards (doc §11.2 write-ahead protocol).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliveryAttempt {
    pub dispatch_id: String,
    pub attempt_no: u32,
    /// Stable idempotency key for the runner: `{dispatch_id}#{attempt_no}`.
    pub delivery_id: String,
    pub lease_epoch: u64,
    pub target_id: String,
    pub instance_id: String,
    pub endpoint: String,
    pub stage: DeliveryStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<DeliveryOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_token: Option<String>,
    /// Runner epoch captured at bind time; used to replay activate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_epoch: Option<u64>,
    pub deadline_at: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

pub fn delivery_id_for(dispatch_id: &str, attempt_no: u32) -> String {
    format!("{}#{}", dispatch_id, attempt_no)
}

// ---------------------------------------------------------------------------
// Digest helper (canonical JSON, shared with Task Core input digests)
// ---------------------------------------------------------------------------

/// Sha256 over a canonical (recursively key-sorted) JSON encoding.
pub fn compute_input_digest(input: &Value) -> String {
    crate::task_mgr::compute_task_input_digest(input)
}

// ---------------------------------------------------------------------------
// Dispatcher <-> Runner push protocol (doc §13.5)
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

/// Dispatcher -> Runner: validate, authorize and reserve capacity for a task.
/// Must be idempotent per `delivery_id`; must NOT start business side
/// effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferTaskReq {
    pub delivery_id: String,
    pub task_id: TaskId,
    pub schema_id: String,
    pub schema_version: u32,
    pub target_id: String,
    pub input: Value,
    pub input_digest: String,
    pub auth: DispatchAuthEnvelope,
    pub lease_epoch: u64,
    /// Unix ms deadline for the reservation decision.
    pub deadline_at: u64,
}
impl_from_json!(OfferTaskReq);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum OfferTaskResp {
    /// Capacity reserved (not started). The reservation token round-trips
    /// through bind + activate.
    OfferAccepted {
        app_instance_id: String,
        reservation_token: String,
    },
    Busy {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_ms: Option<u64>,
    },
    Rejected {
        stable_reason: DispatchRejectReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}
impl_from_json!(OfferTaskResp);

/// Dispatcher -> Runner: the task executor is bound; start execution. Must be
/// idempotent per `(task_id, runner_epoch, delivery_id)`. Business side
/// effects may only start after this call confirms the runner epoch is
/// current.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateTaskReq {
    pub delivery_id: String,
    pub task_id: TaskId,
    pub runner_epoch: u64,
    pub reservation_token: String,
}
impl_from_json!(ActivateTaskReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateTaskResp {
    pub activated: bool,
}
impl_from_json!(ActivateTaskResp);

/// Runner-side handler for the dispatcher's push protocol. Services embed
/// this next to their normal RPC surface (same process, own kapi path or an
/// existing one).
#[async_trait]
pub trait TaskRunnerHandler: Send + Sync {
    async fn handle_offer_task(&self, req: OfferTaskReq, ctx: RPCContext) -> Result<OfferTaskResp>;
    async fn handle_activate_task(
        &self,
        req: ActivateTaskReq,
        ctx: RPCContext,
    ) -> Result<ActivateTaskResp>;
}

/// RPC adapter for a `TaskRunnerHandler`; dispatches the two push methods.
pub struct TaskRunnerServerHandler<T: TaskRunnerHandler>(pub T);

impl<T: TaskRunnerHandler> TaskRunnerServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: TaskRunnerHandler> RPCHandler for TaskRunnerServerHandler<T> {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);
        let result = match req.method.as_str() {
            "offer_task" => {
                let offer_req = OfferTaskReq::from_json(req.params)?;
                let resp = self.0.handle_offer_task(offer_req, ctx).await?;
                RPCResult::Success(json!(resp))
            }
            "activate_task" => {
                let activate_req = ActivateTaskReq::from_json(req.params)?;
                let resp = self.0.handle_activate_task(activate_req, ctx).await?;
                RPCResult::Success(json!(resp))
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
// Caller-facing RPC payloads (doc §13.5)
// ---------------------------------------------------------------------------

/// Caller-side submit params. Identity fields (`on_behalf_of`) are validated
/// against the verified token: normal callers may not impersonate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchTaskReq {
    /// `None` = resolve via the schema's admin-configured default route.
    #[serde(default)]
    pub target_id: Option<String>,
    /// Task schema id including the major version (also the route key).
    pub schema_id: String,
    /// `None` -> latest enabled registry revision at accept time (frozen).
    #[serde(default)]
    pub schema_version: Option<u32>,
    /// Display name for the created task; defaults to the schema id.
    #[serde(default)]
    pub name: Option<String>,
    pub input: Value,
    /// Caller-side idempotency key; `(requested_by_user, requested_by_app,
    /// idempotency_key)` is unique. Replays return the existing record.
    pub idempotency_key: String,
    #[serde(default)]
    pub priority: Option<i64>,
    /// Handoff deadline (unix ms). Not `Accepted` by then -> `Expired`.
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub on_behalf_of: Option<String>,
    #[serde(default)]
    pub workflow_ref: Option<WorkflowStepRef>,
    /// Optional parent task for tree-structured work.
    #[serde(default)]
    pub parent_task_id: Option<TaskId>,
}
impl_from_json!(DispatchTaskReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchTaskResult {
    pub task_id: TaskId,
    pub dispatch_id: String,
    pub status: DispatchStatus,
    pub task: Task,
}
impl_from_json!(DispatchTaskResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDispatchReq {
    #[serde(default)]
    pub dispatch_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<TaskId>,
}
impl_from_json!(GetDispatchReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDispatchResult {
    pub record: DispatchRecord,
}
impl_from_json!(GetDispatchResult);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListDispatchesReq {
    #[serde(default)]
    pub status: Option<DispatchStatus>,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub schema_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}
impl_from_json!(ListDispatchesReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDispatchesResult {
    pub records: Vec<DispatchRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
impl_from_json!(ListDispatchesResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelDispatchReq {
    pub dispatch_id: String,
}
impl_from_json!(CancelDispatchReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveDispatchReq {
    pub dispatch_id: String,
    pub decision: ApprovalDecision,
    #[serde(default)]
    pub note: Option<String>,
}
impl_from_json!(ApproveDispatchReq);

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
    pub instance_id: String,
    /// kRPC URL the dispatcher calls offer/activate on.
    pub endpoint: String,
    pub capacity: u32,
    #[serde(default)]
    pub available_capacity: Option<u32>,
    /// Lease duration request in ms; server may clamp.
    #[serde(default)]
    pub lease_ms: Option<u64>,
}
impl_from_json!(AttachInstanceReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachInstanceResult {
    pub lease_epoch: u64,
    pub lease_expires_at: u64,
    /// Kevent channel slug for wake-up hints.
    pub target_key: String,
}
impl_from_json!(AttachInstanceResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewInstanceReq {
    pub target_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
    #[serde(default)]
    pub available_capacity: Option<u32>,
    #[serde(default)]
    pub lease_ms: Option<u64>,
}
impl_from_json!(RenewInstanceReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewInstanceResult {
    pub lease_epoch: u64,
    pub lease_expires_at: u64,
}
impl_from_json!(RenewInstanceResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetachInstanceReq {
    pub target_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
}
impl_from_json!(DetachInstanceReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTargetReq {
    pub target_id: String,
}
impl_from_json!(GetTargetReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTargetResult {
    pub registration: TargetRegistration,
    pub instances: Vec<TargetInstance>,
}
impl_from_json!(GetTargetResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTargetsResult {
    pub targets: Vec<TargetRegistration>,
}
impl_from_json!(ListTargetsResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetOperationRouteReq {
    pub schema_id: String,
    pub default_target_id: String,
}
impl_from_json!(SetOperationRouteReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisableOperationRouteReq {
    pub schema_id: String,
}
impl_from_json!(DisableOperationRouteReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOperationRouteReq {
    pub schema_id: String,
}
impl_from_json!(GetOperationRouteReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOperationRouteResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<OperationRoute>,
}
impl_from_json!(GetOperationRouteResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListOperationRoutesResult {
    pub routes: Vec<OperationRoute>,
}
impl_from_json!(ListOperationRoutesResult);

// ---------------------------------------------------------------------------
// Handler trait (server side)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait TaskDispatcherHandler: Send + Sync {
    async fn handle_dispatch_task(
        &self,
        req: DispatchTaskReq,
        ctx: RPCContext,
    ) -> Result<DispatchTaskResult>;
    async fn handle_get_dispatch(
        &self,
        req: GetDispatchReq,
        ctx: RPCContext,
    ) -> Result<GetDispatchResult>;
    async fn handle_list_dispatches(
        &self,
        req: ListDispatchesReq,
        ctx: RPCContext,
    ) -> Result<ListDispatchesResult>;
    async fn handle_cancel_dispatch(
        &self,
        req: CancelDispatchReq,
        ctx: RPCContext,
    ) -> Result<GetDispatchResult>;
    async fn handle_approve_dispatch(
        &self,
        req: ApproveDispatchReq,
        ctx: RPCContext,
    ) -> Result<GetDispatchResult>;

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

    async fn handle_get_target(&self, req: GetTargetReq, ctx: RPCContext) -> Result<GetTargetResult>;
    async fn handle_list_targets(&self, ctx: RPCContext) -> Result<ListTargetsResult>;
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
    ) -> Result<GetOperationRouteResult>;
    async fn handle_list_operation_routes(
        &self,
        ctx: RPCContext,
    ) -> Result<ListOperationRoutesResult>;
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub enum TaskDispatcherClient {
    InProcess(Box<dyn TaskDispatcherHandler>),
    KRPC(Box<kRPC>),
}

macro_rules! client_call {
    ($self:ident, $method:literal, $req:expr, $resp:ty, $handler:ident) => {
        match $self {
            Self::InProcess(handler) => handler.$handler($req, RPCContext::default()).await,
            Self::KRPC(client) => {
                let params = serde_json::to_value(&$req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call($method, params).await?;
                serde_json::from_value::<$resp>(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!(
                        concat!("Expected ", stringify!($resp), ": {}"),
                        e
                    ))
                })
            }
        }
    };
}

impl TaskDispatcherClient {
    pub fn new(client: kRPC) -> Self {
        Self::KRPC(Box::new(client))
    }

    pub fn new_in_process(handler: Box<dyn TaskDispatcherHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => client.set_context(context).await,
        }
    }

    pub async fn dispatch_task(&self, req: DispatchTaskReq) -> Result<DispatchTaskResult> {
        client_call!(self, "dispatch_task", req, DispatchTaskResult, handle_dispatch_task)
    }

    pub async fn get_dispatch(&self, req: GetDispatchReq) -> Result<DispatchRecord> {
        client_call!(self, "get_dispatch", req, GetDispatchResult, handle_get_dispatch)
            .map(|r| r.record)
    }

    pub async fn list_dispatches(&self, req: ListDispatchesReq) -> Result<ListDispatchesResult> {
        client_call!(
            self,
            "list_dispatches",
            req,
            ListDispatchesResult,
            handle_list_dispatches
        )
    }

    pub async fn cancel_dispatch(&self, dispatch_id: &str) -> Result<DispatchRecord> {
        let req = CancelDispatchReq {
            dispatch_id: dispatch_id.to_string(),
        };
        client_call!(
            self,
            "cancel_dispatch",
            req,
            GetDispatchResult,
            handle_cancel_dispatch
        )
        .map(|r| r.record)
    }

    pub async fn approve_dispatch(&self, req: ApproveDispatchReq) -> Result<DispatchRecord> {
        client_call!(
            self,
            "approve_dispatch",
            req,
            GetDispatchResult,
            handle_approve_dispatch
        )
        .map(|r| r.record)
    }

    pub async fn register_target(&self, registration: TargetRegistration) -> Result<TargetRegistration> {
        let req = RegisterTargetReq { registration };
        client_call!(
            self,
            "register_target",
            req,
            TargetRegistration,
            handle_register_target
        )
    }

    pub async fn disable_target(&self, target_id: &str) -> Result<()> {
        let req = DisableTargetReq {
            target_id: target_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => handler.handle_disable_target(req, RPCContext::default()).await,
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("disable_target", params).await?;
                Ok(())
            }
        }
    }

    pub async fn attach_instance(&self, req: AttachInstanceReq) -> Result<AttachInstanceResult> {
        client_call!(
            self,
            "attach_instance",
            req,
            AttachInstanceResult,
            handle_attach_instance
        )
    }

    pub async fn renew_instance(&self, req: RenewInstanceReq) -> Result<RenewInstanceResult> {
        client_call!(
            self,
            "renew_instance",
            req,
            RenewInstanceResult,
            handle_renew_instance
        )
    }

    pub async fn detach_instance(&self, req: DetachInstanceReq) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                handler.handle_detach_instance(req, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("detach_instance", params).await?;
                Ok(())
            }
        }
    }

    pub async fn get_target(&self, target_id: &str) -> Result<GetTargetResult> {
        let req = GetTargetReq {
            target_id: target_id.to_string(),
        };
        client_call!(self, "get_target", req, GetTargetResult, handle_get_target)
    }

    pub async fn list_targets(&self) -> Result<Vec<TargetRegistration>> {
        match self {
            Self::InProcess(handler) => Ok(handler
                .handle_list_targets(RPCContext::default())
                .await?
                .targets),
            Self::KRPC(client) => {
                let result = client.call("list_targets", json!({})).await?;
                let parsed: ListTargetsResult = serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected ListTargetsResult: {}", e))
                })?;
                Ok(parsed.targets)
            }
        }
    }

    pub async fn set_operation_route(
        &self,
        schema_id: &str,
        default_target_id: &str,
    ) -> Result<OperationRoute> {
        let req = SetOperationRouteReq {
            schema_id: schema_id.to_string(),
            default_target_id: default_target_id.to_string(),
        };
        client_call!(
            self,
            "set_operation_route",
            req,
            OperationRoute,
            handle_set_operation_route
        )
    }

    pub async fn disable_operation_route(&self, schema_id: &str) -> Result<()> {
        let req = DisableOperationRouteReq {
            schema_id: schema_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_disable_operation_route(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("disable_operation_route", params).await?;
                Ok(())
            }
        }
    }

    pub async fn get_operation_route(&self, schema_id: &str) -> Result<Option<OperationRoute>> {
        let req = GetOperationRouteReq {
            schema_id: schema_id.to_string(),
        };
        client_call!(
            self,
            "get_operation_route",
            req,
            GetOperationRouteResult,
            handle_get_operation_route
        )
        .map(|r| r.route)
    }

    pub async fn list_operation_routes(&self) -> Result<Vec<OperationRoute>> {
        match self {
            Self::InProcess(handler) => Ok(handler
                .handle_list_operation_routes(RPCContext::default())
                .await?
                .routes),
            Self::KRPC(client) => {
                let result = client.call("list_operation_routes", json!({})).await?;
                let parsed: ListOperationRoutesResult =
                    serde_json::from_value(result).map_err(|e| {
                        RPCErrors::ParserResponseError(format!(
                            "Expected ListOperationRoutesResult: {}",
                            e
                        ))
                    })?;
                Ok(parsed.routes)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Server-side RPC dispatch
// ---------------------------------------------------------------------------

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
            "dispatch_task" => {
                let dispatch_req = DispatchTaskReq::from_json(req.params)?;
                let result = self.0.handle_dispatch_task(dispatch_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "get_dispatch" => {
                let get_req = GetDispatchReq::from_json(req.params)?;
                let result = self.0.handle_get_dispatch(get_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "list_dispatches" => {
                let list_req = ListDispatchesReq::from_json(req.params)?;
                let result = self.0.handle_list_dispatches(list_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "cancel_dispatch" => {
                let cancel_req = CancelDispatchReq::from_json(req.params)?;
                let result = self.0.handle_cancel_dispatch(cancel_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "approve_dispatch" => {
                let approve_req = ApproveDispatchReq::from_json(req.params)?;
                let result = self.0.handle_approve_dispatch(approve_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "register_target" => {
                let register_req = RegisterTargetReq::from_json(req.params)?;
                let registration = self.0.handle_register_target(register_req, ctx).await?;
                RPCResult::Success(json!(registration))
            }
            "disable_target" => {
                let disable_req = DisableTargetReq::from_json(req.params)?;
                self.0.handle_disable_target(disable_req, ctx).await?;
                RPCResult::Success(json!({}))
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
                RPCResult::Success(json!({}))
            }
            "get_target" => {
                let get_req = GetTargetReq::from_json(req.params)?;
                let result = self.0.handle_get_target(get_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "list_targets" => {
                let result = self.0.handle_list_targets(ctx).await?;
                RPCResult::Success(json!(result))
            }
            "set_operation_route" => {
                let route_req = SetOperationRouteReq::from_json(req.params)?;
                let route = self.0.handle_set_operation_route(route_req, ctx).await?;
                RPCResult::Success(json!(route))
            }
            "disable_operation_route" => {
                let route_req = DisableOperationRouteReq::from_json(req.params)?;
                self.0.handle_disable_operation_route(route_req, ctx).await?;
                RPCResult::Success(json!({}))
            }
            "get_operation_route" => {
                let route_req = GetOperationRouteReq::from_json(req.params)?;
                let result = self.0.handle_get_operation_route(route_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "list_operation_routes" => {
                let result = self.0.handle_list_operation_routes(ctx).await?;
                RPCResult::Success(json!(result))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_key_is_stable_and_kevent_safe() {
        let key = task_dispatcher_target_key("did:bns:agent#worker");
        assert!(key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'));
        assert_eq!(key, task_dispatcher_target_key("did:bns:agent#worker"));
        assert_ne!(key, task_dispatcher_target_key("did:bns:agent-worker"));
    }

    #[test]
    fn backoff_is_deterministic_and_bounded() {
        let policy = DeliveryPolicy::default();
        let a1 = policy.backoff_ms("dsp-1", 1);
        let a2 = policy.backoff_ms("dsp-1", 1);
        assert_eq!(a1, a2);
        let later = policy.backoff_ms("dsp-1", 8);
        assert!(later >= a1);
        assert!(later <= policy.backoff_max_ms + policy.backoff_max_ms / 4);
    }

    #[test]
    fn offer_resp_serde() {
        let accepted = OfferTaskResp::OfferAccepted {
            app_instance_id: "inst-1".into(),
            reservation_token: "res-1".into(),
        };
        let json = serde_json::to_value(&accepted).unwrap();
        assert_eq!(json["kind"], "OfferAccepted");
        let back: OfferTaskResp = serde_json::from_value(json).unwrap();
        assert_eq!(back, accepted);

        let busy: OfferTaskResp = serde_json::from_value(json!({"kind": "Busy"})).unwrap();
        assert_eq!(busy, OfferTaskResp::Busy { retry_after_ms: None });
    }

    #[test]
    fn dispatch_status_transitions() {
        assert!(DispatchStatus::Queued.is_assignable());
        assert!(DispatchStatus::WaitingForTarget.is_assignable());
        assert!(!DispatchStatus::PendingApproval.is_assignable());
        assert!(!DispatchStatus::CreatingTask.is_assignable());
        assert!(DispatchStatus::Accepted.is_terminal());
        assert!(DispatchStatus::Offering.owns_promised_task());
        assert!(!DispatchStatus::Binding.owns_promised_task());
    }
}
