//! TaskMgr 2.0 shared protocol layer.
//!
//! Design: `doc/task_mgr/task-mgr 2.0.md`. This module carries the shared
//! types, the durable schema DDL, the kRPC request/response payloads, the
//! `TaskManagerClient` and the server-side handler trait for the Task Core
//! half of the TaskMgr 2.0 subsystem. The Dispatch Center protocol lives in
//! `task_dispatcher.rs`.
//!
//! beta2.2 breaking change: there is no compatibility layer for the 1.x
//! `TaskStatus` / mutable `Task.data` / read-write scope protocol.

use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use crate::rdb_mgr::{RdbBackend, RdbInstanceConfig};
use crate::{AppDoc, AppType, SelectorType};

pub const TASK_MANAGER_SERVICE_UNIQUE_ID: &str = "task-manager";
pub const TASK_MANAGER_SERVICE_NAME: &str = "task-manager";
pub const TASK_MANAGER_SERVICE_PORT: u16 = 3380;

/// Logical name of the task-manager rdb instance. Used by both the scheduler
/// (when it writes the service's `spec_config`) and the task-manager itself
/// (when it calls `get_rdb_instance`).
pub const TASK_MANAGER_RDB_INSTANCE_ID: &str = "task-mgr-main";

/// Version of the Task Core durable schema. v7 is the TaskMgr 2.0 model:
/// opaque string task ids, immutable input, one-shot result, composite
/// phase, executor binding with runner epoch, ACL grants and durable events.
pub const TASK_MANAGER_RDB_SCHEMA_VERSION: u64 = 7;

/// Sqlite DDL for the Task Core database. `CREATE TABLE IF NOT EXISTS` so the
/// bootstrap is safe to re-run on every process start. Boolean-like columns
/// are INTEGER 0/1 so both sqlx-any backends decode them identically.
pub const TASK_MANAGER_RDB_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS task (
    task_id                   TEXT PRIMARY KEY,
    schema_id                 TEXT NOT NULL,
    schema_version            INTEGER NOT NULL,
    name                      TEXT NOT NULL,
    input_json                TEXT NOT NULL,
    input_digest              TEXT NOT NULL,
    result_json               TEXT,
    error_json                TEXT,
    creator_user_id           TEXT NOT NULL,
    creator_app_id            TEXT NOT NULL,
    creator_instance_id       TEXT,
    idempotency_key           TEXT NOT NULL,
    origin_kind               TEXT,
    origin_id                 TEXT,
    parent_id                 TEXT REFERENCES task(task_id),
    root_id                   TEXT NOT NULL,
    child_control_policy_json TEXT NOT NULL,
    retry_of                  TEXT,
    supersedes                TEXT,
    executor_kind             TEXT NOT NULL DEFAULT 'Unbound',
    runner_target_id          TEXT,
    runner_instance_id        TEXT,
    runner_app_id             TEXT,
    runner_epoch              INTEGER NOT NULL DEFAULT 0,
    phase                     TEXT NOT NULL,
    wait_reason_json          TEXT,
    control_request_json      TEXT,
    control_profile_json      TEXT NOT NULL,
    progress_json             TEXT,
    message                   TEXT,
    outcome                   TEXT,
    completed_by_user_id      TEXT,
    completed_by_app_id       TEXT,
    policy_preset             TEXT NOT NULL DEFAULT 'collaborative-tree/v1',
    permission_boundary       INTEGER NOT NULL DEFAULT 0,
    revision                  INTEGER NOT NULL DEFAULT 1,
    created_at                INTEGER NOT NULL,
    updated_at                INTEGER NOT NULL,
    completed_at              INTEGER,
    archived_at               INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_task_creator_idempotency ON task(creator_user_id, creator_app_id, idempotency_key);
CREATE UNIQUE INDEX IF NOT EXISTS uq_task_origin ON task(origin_kind, origin_id) WHERE origin_kind IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_task_root_created ON task(root_id, created_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_parent_created ON task(parent_id, created_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_phase_updated ON task(phase, updated_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_creator_created ON task(creator_user_id, creator_app_id, created_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_schema_created ON task(schema_id, schema_version, created_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_runner_phase ON task(runner_target_id, runner_instance_id, phase);

CREATE TABLE IF NOT EXISTS task_assignee (
    task_id            TEXT NOT NULL REFERENCES task(task_id),
    user_id            TEXT NOT NULL,
    granted_by_user_id TEXT NOT NULL,
    granted_by_app_id  TEXT NOT NULL,
    created_at         INTEGER NOT NULL,
    revoked_at         INTEGER,
    PRIMARY KEY (task_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_task_assignee_user_active ON task_assignee(user_id, revoked_at, task_id);

CREATE TABLE IF NOT EXISTS task_acl_grant (
    grant_id           TEXT PRIMARY KEY,
    task_id            TEXT NOT NULL REFERENCES task(task_id),
    subject_kind       TEXT NOT NULL,
    subject_relation   TEXT,
    subject_user_id    TEXT,
    subject_app_id     TEXT,
    subject_system_role TEXT,
    actions_json       TEXT NOT NULL,
    scope              TEXT NOT NULL,
    data_scope         TEXT NOT NULL,
    created_by_user_id TEXT NOT NULL,
    created_by_app_id  TEXT NOT NULL,
    created_at         INTEGER NOT NULL,
    revoked_at         INTEGER
);
CREATE INDEX IF NOT EXISTS idx_task_acl_task_active ON task_acl_grant(task_id, revoked_at);
CREATE INDEX IF NOT EXISTS idx_task_acl_user_active ON task_acl_grant(subject_user_id, revoked_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_acl_app_active ON task_acl_grant(subject_app_id, revoked_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_acl_role_active ON task_acl_grant(subject_system_role, revoked_at, task_id);

CREATE TABLE IF NOT EXISTS task_event (
    event_id          TEXT PRIMARY KEY,
    task_id           TEXT NOT NULL REFERENCES task(task_id),
    root_id           TEXT NOT NULL,
    task_revision     INTEGER NOT NULL,
    event_type        TEXT NOT NULL,
    actor_user_id     TEXT,
    actor_app_id      TEXT,
    actor_instance_id TEXT,
    payload_json      TEXT NOT NULL,
    created_at        INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_task_event_revision ON task_event(task_id, task_revision);
CREATE INDEX IF NOT EXISTS idx_task_event_task ON task_event(task_id, event_id);
CREATE INDEX IF NOT EXISTS idx_task_event_root ON task_event(root_id, event_id);

CREATE TABLE IF NOT EXISTS task_schema (
    schema_id                TEXT NOT NULL,
    schema_version           INTEGER NOT NULL,
    input_schema_json        TEXT NOT NULL,
    output_schema_json       TEXT NOT NULL,
    presentation_schema_json TEXT,
    executor_kinds_json      TEXT NOT NULL,
    user_creatable           INTEGER NOT NULL DEFAULT 0,
    publisher_app_id         TEXT NOT NULL,
    enabled                  INTEGER NOT NULL DEFAULT 1,
    created_at               INTEGER NOT NULL,
    PRIMARY KEY (schema_id, schema_version)
);

CREATE TABLE IF NOT EXISTS task_note (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id         TEXT NOT NULL,
    note_type       TEXT NOT NULL DEFAULT '',
    content         TEXT NOT NULL DEFAULT '',
    data            TEXT,
    author_user_id  TEXT NOT NULL DEFAULT '',
    author_app_id   TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_note_task_created ON task_note(task_id, created_at ASC, id ASC);
"#;

/// Postgres DDL: same logical schema as the sqlite variant.
pub const TASK_MANAGER_RDB_SCHEMA_POSTGRES: &str = r#"
CREATE TABLE IF NOT EXISTS task (
    task_id                   TEXT PRIMARY KEY,
    schema_id                 TEXT NOT NULL,
    schema_version            BIGINT NOT NULL,
    name                      TEXT NOT NULL,
    input_json                TEXT NOT NULL,
    input_digest              TEXT NOT NULL,
    result_json               TEXT,
    error_json                TEXT,
    creator_user_id           TEXT NOT NULL,
    creator_app_id            TEXT NOT NULL,
    creator_instance_id       TEXT,
    idempotency_key           TEXT NOT NULL,
    origin_kind               TEXT,
    origin_id                 TEXT,
    parent_id                 TEXT REFERENCES task(task_id),
    root_id                   TEXT NOT NULL,
    child_control_policy_json TEXT NOT NULL,
    retry_of                  TEXT,
    supersedes                TEXT,
    executor_kind             TEXT NOT NULL DEFAULT 'Unbound',
    runner_target_id          TEXT,
    runner_instance_id        TEXT,
    runner_app_id             TEXT,
    runner_epoch              BIGINT NOT NULL DEFAULT 0,
    phase                     TEXT NOT NULL,
    wait_reason_json          TEXT,
    control_request_json      TEXT,
    control_profile_json      TEXT NOT NULL,
    progress_json             TEXT,
    message                   TEXT,
    outcome                   TEXT,
    completed_by_user_id      TEXT,
    completed_by_app_id       TEXT,
    policy_preset             TEXT NOT NULL DEFAULT 'collaborative-tree/v1',
    permission_boundary       BIGINT NOT NULL DEFAULT 0,
    revision                  BIGINT NOT NULL DEFAULT 1,
    created_at                BIGINT NOT NULL,
    updated_at                BIGINT NOT NULL,
    completed_at              BIGINT,
    archived_at               BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_task_creator_idempotency ON task(creator_user_id, creator_app_id, idempotency_key);
CREATE UNIQUE INDEX IF NOT EXISTS uq_task_origin ON task(origin_kind, origin_id) WHERE origin_kind IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_task_root_created ON task(root_id, created_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_parent_created ON task(parent_id, created_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_phase_updated ON task(phase, updated_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_creator_created ON task(creator_user_id, creator_app_id, created_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_schema_created ON task(schema_id, schema_version, created_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_runner_phase ON task(runner_target_id, runner_instance_id, phase);

CREATE TABLE IF NOT EXISTS task_assignee (
    task_id            TEXT NOT NULL REFERENCES task(task_id),
    user_id            TEXT NOT NULL,
    granted_by_user_id TEXT NOT NULL,
    granted_by_app_id  TEXT NOT NULL,
    created_at         BIGINT NOT NULL,
    revoked_at         BIGINT,
    PRIMARY KEY (task_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_task_assignee_user_active ON task_assignee(user_id, revoked_at, task_id);

CREATE TABLE IF NOT EXISTS task_acl_grant (
    grant_id           TEXT PRIMARY KEY,
    task_id            TEXT NOT NULL REFERENCES task(task_id),
    subject_kind       TEXT NOT NULL,
    subject_relation   TEXT,
    subject_user_id    TEXT,
    subject_app_id     TEXT,
    subject_system_role TEXT,
    actions_json       TEXT NOT NULL,
    scope              TEXT NOT NULL,
    data_scope         TEXT NOT NULL,
    created_by_user_id TEXT NOT NULL,
    created_by_app_id  TEXT NOT NULL,
    created_at         BIGINT NOT NULL,
    revoked_at         BIGINT
);
CREATE INDEX IF NOT EXISTS idx_task_acl_task_active ON task_acl_grant(task_id, revoked_at);
CREATE INDEX IF NOT EXISTS idx_task_acl_user_active ON task_acl_grant(subject_user_id, revoked_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_acl_app_active ON task_acl_grant(subject_app_id, revoked_at, task_id);
CREATE INDEX IF NOT EXISTS idx_task_acl_role_active ON task_acl_grant(subject_system_role, revoked_at, task_id);

CREATE TABLE IF NOT EXISTS task_event (
    event_id          TEXT PRIMARY KEY,
    task_id           TEXT NOT NULL REFERENCES task(task_id),
    root_id           TEXT NOT NULL,
    task_revision     BIGINT NOT NULL,
    event_type        TEXT NOT NULL,
    actor_user_id     TEXT,
    actor_app_id      TEXT,
    actor_instance_id TEXT,
    payload_json      TEXT NOT NULL,
    created_at        BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_task_event_revision ON task_event(task_id, task_revision);
CREATE INDEX IF NOT EXISTS idx_task_event_task ON task_event(task_id, event_id);
CREATE INDEX IF NOT EXISTS idx_task_event_root ON task_event(root_id, event_id);

CREATE TABLE IF NOT EXISTS task_schema (
    schema_id                TEXT NOT NULL,
    schema_version           BIGINT NOT NULL,
    input_schema_json        TEXT NOT NULL,
    output_schema_json       TEXT NOT NULL,
    presentation_schema_json TEXT,
    executor_kinds_json      TEXT NOT NULL,
    user_creatable           BIGINT NOT NULL DEFAULT 0,
    publisher_app_id         TEXT NOT NULL,
    enabled                  BIGINT NOT NULL DEFAULT 1,
    created_at               BIGINT NOT NULL,
    PRIMARY KEY (schema_id, schema_version)
);

CREATE TABLE IF NOT EXISTS task_note (
    id              BIGSERIAL PRIMARY KEY,
    task_id         TEXT NOT NULL,
    note_type       TEXT NOT NULL DEFAULT '',
    content         TEXT NOT NULL DEFAULT '',
    data            TEXT,
    author_user_id  TEXT NOT NULL DEFAULT '',
    author_app_id   TEXT NOT NULL DEFAULT '',
    created_at      BIGINT NOT NULL,
    updated_at      BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_note_task_created ON task_note(task_id, created_at ASC, id ASC);
"#;

/// Default rdb-instance config for the task-manager service.
pub fn task_manager_default_rdb_instance_config() -> RdbInstanceConfig {
    let mut schema = HashMap::new();
    schema.insert(
        RdbBackend::Sqlite,
        TASK_MANAGER_RDB_SCHEMA_SQLITE.to_string(),
    );
    schema.insert(
        RdbBackend::Postgres,
        TASK_MANAGER_RDB_SCHEMA_POSTGRES.to_string(),
    );
    RdbInstanceConfig {
        backend: RdbBackend::Sqlite,
        version: TASK_MANAGER_RDB_SCHEMA_VERSION,
        schema,
        // Empty -> rdb_mgr generates `sqlite://$appdata/main.db` at resolve time.
        connection: String::new(),
    }
}

pub fn generate_task_manager_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        TASK_MANAGER_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Task Manager")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// URL-safe opaque Task ID generated by TaskMgr. Callers must not parse the
/// format, infer creation time or tree relationships, or mint ids themselves.
pub type TaskId = String;
pub type TaskNoteId = i64;

/// Default policy preset applied when a create request does not name one.
pub const TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1: &str = "collaborative-tree/v1";

/// KEvent path for a single task's change notifications.
pub fn task_mgr_task_event_path(task_id: &str) -> String {
    format!("/task_mgr/{}", task_id)
}

/// KEvent path fanning out every event under a task tree.
pub fn task_mgr_tree_event_path(root_id: &str) -> String {
    format!("/task_mgr/tree/{}", root_id)
}

// ---------------------------------------------------------------------------
// Stable error codes (doc §13.6)
// ---------------------------------------------------------------------------

pub const TASK_ERR_NOT_FOUND: &str = "task_not_found";
pub const TASK_ERR_PERMISSION_DENIED: &str = "permission_denied";
pub const TASK_ERR_REVISION_CONFLICT: &str = "revision_conflict";
pub const TASK_ERR_STALE_RUNNER_EPOCH: &str = "stale_runner_epoch";
pub const TASK_ERR_INVALID_PHASE: &str = "invalid_task_phase";
pub const TASK_ERR_CONTROL_NOT_AVAILABLE: &str = "control_not_available";
pub const TASK_ERR_CONTROL_ALREADY_PENDING: &str = "control_already_pending";
pub const TASK_ERR_ALREADY_COMPLETED: &str = "task_already_completed";
pub const TASK_ERR_INPUT_SCHEMA_MISMATCH: &str = "input_schema_mismatch";
pub const TASK_ERR_RESULT_SCHEMA_MISMATCH: &str = "result_schema_mismatch";
pub const TASK_ERR_IDEMPOTENCY_CONFLICT: &str = "idempotency_conflict";
pub const TASK_ERR_SCHEMA_NOT_FOUND: &str = "task_schema_not_found";

const TASK_ERR_CODES: &[&str] = &[
    TASK_ERR_NOT_FOUND,
    TASK_ERR_PERMISSION_DENIED,
    TASK_ERR_REVISION_CONFLICT,
    TASK_ERR_STALE_RUNNER_EPOCH,
    TASK_ERR_INVALID_PHASE,
    TASK_ERR_CONTROL_NOT_AVAILABLE,
    TASK_ERR_CONTROL_ALREADY_PENDING,
    TASK_ERR_ALREADY_COMPLETED,
    TASK_ERR_INPUT_SCHEMA_MISMATCH,
    TASK_ERR_RESULT_SCHEMA_MISMATCH,
    TASK_ERR_IDEMPOTENCY_CONFLICT,
    TASK_ERR_SCHEMA_NOT_FOUND,
];

/// Build an RPC error carrying a stable TaskMgr error code. The code travels
/// as a `code: detail` prefix so it survives the kRPC wire format unchanged.
pub fn task_mgr_error(code: &str, detail: impl fmt::Display) -> RPCErrors {
    match code {
        TASK_ERR_PERMISSION_DENIED => RPCErrors::NoPermission(format!("{}: {}", code, detail)),
        _ => RPCErrors::ReasonError(format!("{}: {}", code, detail)),
    }
}

/// Extract the stable TaskMgr error code from an RPC error, if present.
pub fn task_mgr_error_code(err: &RPCErrors) -> Option<&'static str> {
    let text = match err {
        RPCErrors::ReasonError(msg) | RPCErrors::NoPermission(msg) => msg.as_str(),
        _ => return None,
    };
    TASK_ERR_CODES
        .iter()
        .find(|code| {
            text.strip_prefix(**code)
                .map(|rest| rest.is_empty() || rest.starts_with(':'))
                .unwrap_or(false)
        })
        .copied()
}

// ---------------------------------------------------------------------------
// Actors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorRef {
    pub user_id: String,
    pub app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_instance_id: Option<String>,
}

impl ActorRef {
    pub fn new(user_id: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            app_id: app_id.into(),
            app_instance_id: None,
        }
    }
}

/// Opaque, immutable creation-origin link (e.g. the dispatcher's record id).
/// TaskMgr only enforces uniqueness and immutability; it never interprets or
/// calls back into the origin system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskOriginRef {
    pub kind: String,
    pub id: String,
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskExecutorKind {
    Unbound,
    App,
    HumanSet,
}

impl fmt::Display for TaskExecutorKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for TaskExecutorKind {
    type Err = RPCErrors;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "Unbound" => Ok(Self::Unbound),
            "App" => Ok(Self::App),
            "HumanSet" => Ok(Self::HumanSet),
            _ => Err(RPCErrors::ParseRequestError(format!(
                "invalid executor kind: {}",
                s
            ))),
        }
    }
}

/// Current execution binding of a Task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TaskExecutor {
    /// Legal promised state: no executor found/bound yet.
    Unbound,
    /// Executed by one App. `target_id` is the logical backend frozen by the
    /// dispatcher (absent for direct service-internal tasks);
    /// `app_instance_id` is the currently bound instance.
    App {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_id: Option<String>,
        app_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_instance_id: Option<String>,
    },
    /// Result can be committed by any current assignee; first CAS wins.
    HumanSet,
}

impl TaskExecutor {
    pub fn kind(&self) -> TaskExecutorKind {
        match self {
            TaskExecutor::Unbound => TaskExecutorKind::Unbound,
            TaskExecutor::App { .. } => TaskExecutorKind::App,
            TaskExecutor::HumanSet => TaskExecutorKind::HumanSet,
        }
    }
}

// ---------------------------------------------------------------------------
// Composite state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskPhase {
    Promised,
    Accepted,
    Running,
    Waiting,
    Paused,
    Terminal,
}

impl TaskPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskPhase::Terminal)
    }
}

impl fmt::Display for TaskPhase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for TaskPhase {
    type Err = RPCErrors;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "Promised" => Ok(Self::Promised),
            "Accepted" => Ok(Self::Accepted),
            "Running" => Ok(Self::Running),
            "Waiting" => Ok(Self::Waiting),
            "Paused" => Ok(Self::Paused),
            "Terminal" => Ok(Self::Terminal),
            _ => Err(RPCErrors::ParseRequestError(format!(
                "invalid task phase: {}",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskWaitReasonKind {
    Dispatch,
    Capacity,
    Authorization,
    HumanInput,
    ChildTask,
    Dependency,
    External,
    Other,
}

impl fmt::Display for TaskWaitReasonKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskWaitReason {
    pub kind: TaskWaitReasonKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl TaskWaitReason {
    pub fn new(kind: TaskWaitReasonKind) -> Self {
        Self {
            kind,
            code: None,
            related_task_id: None,
            message: None,
        }
    }

    pub fn with_code(kind: TaskWaitReasonKind, code: impl Into<String>) -> Self {
        Self {
            kind,
            code: Some(code.into()),
            related_task_id: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskOutcome {
    Succeeded,
    Failed,
    Canceled,
}

impl fmt::Display for TaskOutcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for TaskOutcome {
    type Err = RPCErrors;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "Succeeded" => Ok(Self::Succeeded),
            "Failed" => Ok(Self::Failed),
            "Canceled" => Ok(Self::Canceled),
            _ => Err(RPCErrors::ParseRequestError(format!(
                "invalid task outcome: {}",
                s
            ))),
        }
    }
}

/// Stable failure information written once at terminal time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl TaskError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Control protocol
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskControlAction {
    Pause,
    Resume,
    Cancel,
}

impl fmt::Display for TaskControlAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskControlRequest {
    pub request_id: String,
    pub action: TaskControlAction,
    pub requested_by: ActorRef,
    pub requested_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ControlAvailability {
    Available,
    Unavailable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CancelCapability {
    Unavailable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Stop execution; no promise to roll back side effects.
    Interrupt,
    /// Runner promises agreed cleanup before confirming Canceled.
    Safe,
}

/// The current runner's self-declared control capability. Not a TaskMgr
/// promise: the runner may still reject a recorded request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskControlProfile {
    pub pause: ControlAvailability,
    pub resume: ControlAvailability,
    pub cancel: CancelCapability,
    pub updated_at: u64,
}

impl TaskControlProfile {
    /// Baseline profile for unbound / human tasks: no pause/resume; cancel is
    /// interrupt-grade (TaskMgr/Dispatcher-side CAS close, no side-effect
    /// rollback promise).
    pub fn baseline(now: u64) -> Self {
        Self {
            pause: ControlAvailability::Unavailable { reason: None },
            resume: ControlAvailability::Unavailable { reason: None },
            cancel: CancelCapability::Interrupt,
            updated_at: now,
        }
    }
}

/// Whether a parent-level control request propagates to this child task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildControlPolicy {
    pub follow_pause: bool,
    pub follow_resume: bool,
    pub follow_cancel: bool,
}

impl Default for ChildControlPolicy {
    fn default() -> Self {
        Self {
            follow_pause: true,
            follow_resume: true,
            follow_cancel: true,
        }
    }
}

impl ChildControlPolicy {
    pub fn follows(&self, action: TaskControlAction) -> bool {
        match action {
            TaskControlAction::Pause => self.follow_pause,
            TaskControlAction::Resume => self.follow_resume,
            TaskControlAction::Cancel => self.follow_cancel,
        }
    }
}

/// Result of a recursive `request_control`: per-task disposition, no batch
/// final-state write.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchControlResult {
    pub requested: Vec<TaskId>,
    pub skipped_by_policy: Vec<TaskId>,
    pub denied: Vec<TaskId>,
    pub already_terminal: Vec<TaskId>,
}

// ---------------------------------------------------------------------------
// ACL model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskAction {
    ReadMeta,
    ReadInput,
    ReadResult,
    ReportProgress,
    Control,
    Commit,
    CreateChild,
    Reassign,
    Grant,
    Archive,
}

impl fmt::Display for TaskAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TaskGrantSubject {
    RootCreator,
    Creator,
    Runner,
    Assignees,
    User { user_id: String },
    App { app_id: String },
    Principal { user_id: String, app_id: String },
    SystemRole { role: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskGrantScope {
    SelfOnly,
    Subtree,
    WholeTree,
}

impl fmt::Display for TaskGrantScope {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for TaskGrantScope {
    type Err = RPCErrors;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "SelfOnly" => Ok(Self::SelfOnly),
            "Subtree" => Ok(Self::Subtree),
            "WholeTree" => Ok(Self::WholeTree),
            _ => Err(RPCErrors::ParseRequestError(format!(
                "invalid grant scope: {}",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TaskDataScope {
    MetaOnly,
    Payload,
    Full,
}

impl fmt::Display for TaskDataScope {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for TaskDataScope {
    type Err = RPCErrors;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "MetaOnly" => Ok(Self::MetaOnly),
            "Payload" => Ok(Self::Payload),
            "Full" => Ok(Self::Full),
            _ => Err(RPCErrors::ParseRequestError(format!(
                "invalid data scope: {}",
                s
            ))),
        }
    }
}

/// One additive allow grant anchored at a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskAclGrant {
    pub grant_id: String,
    pub task_id: TaskId,
    pub subject: TaskGrantSubject,
    pub actions: Vec<TaskAction>,
    pub scope: TaskGrantScope,
    pub data_scope: TaskDataScope,
    pub created_by: ActorRef,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<u64>,
}

/// Grant payload as supplied by a caller (server assigns id/creator/time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskAclGrantSpec {
    pub subject: TaskGrantSubject,
    pub actions: Vec<TaskAction>,
    pub scope: TaskGrantScope,
    pub data_scope: TaskDataScope,
}

// ---------------------------------------------------------------------------
// Task schema registry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSchemaDefinition {
    /// Stable id including the major version, e.g. `agent.command/v1`.
    pub schema_id: String,
    /// Immutable revision number within the major version.
    pub schema_version: u32,
    pub input_schema: Value,
    pub output_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_schema: Option<Value>,
    pub allowed_executor_kinds: Vec<TaskExecutorKind>,
    pub user_creatable: bool,
    pub publisher_app_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub created_at: u64,
}

// ---------------------------------------------------------------------------
// Task model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    // Stable identity and immutable relationship
    pub task_id: TaskId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TaskId>,
    pub root_id: TaskId,
    pub child_control_policy: ChildControlPolicy,

    // Immutable contract and payload
    pub schema_id: String,
    pub schema_version: u32,
    pub input: Value,
    pub input_digest: String,

    // Immutable creation facts
    pub creator: ActorRef,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_ref: Option<TaskOriginRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<TaskId>,

    // Current execution binding
    pub executor: TaskExecutor,
    pub runner_epoch: u64,
    /// Current active assignees; populated for HumanSet tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,

    // Composite task state
    pub phase: TaskPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_reason: Option<TaskWaitReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_control: Option<TaskControlRequest>,
    pub control_profile: TaskControlProfile,

    // Presentation snapshot
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    // Immutable completion once terminal
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TaskOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_by: Option<ActorRef>,

    // Access and concurrency
    pub policy_preset: String,
    pub permission_boundary: bool,
    pub revision: u64,
    /// Data scope this snapshot was trimmed to for the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_scope: Option<TaskDataScope>,

    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<u64>,
}

/// Lightweight metadata projection for lists and tree views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: TaskId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TaskId>,
    pub root_id: TaskId,
    pub schema_id: String,
    pub schema_version: u32,
    pub creator: ActorRef,
    pub executor_kind: TaskExecutorKind,
    pub phase: TaskPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_reason: Option<TaskWaitReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_control_action: Option<TaskControlAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TaskOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<u64>,
}

// ---------------------------------------------------------------------------
// Durable events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskEventType {
    TaskCreated,
    RunnerBound,
    RunnerReleased,
    PhaseChanged,
    WaitReasonChanged,
    ProgressReported,
    ControlProfileChanged,
    ControlRequested,
    ControlSuperseded,
    ControlApplied,
    ControlRejected,
    AssigneesChanged,
    AccessGranted,
    AccessRevoked,
    ResultCommitted,
    TaskFailed,
    TaskCanceled,
    TaskArchived,
    PayloadRedacted,
}

impl fmt::Display for TaskEventType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub event_id: String,
    pub task_id: TaskId,
    pub root_id: TaskId,
    pub task_revision: u64,
    pub event_type: TaskEventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
    pub payload: Value,
    pub created_at: u64,
}

// ---------------------------------------------------------------------------
// Task notes (unchanged side-channel semantics; task_id is now a string)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNote {
    pub id: TaskNoteId,
    pub task_id: TaskId,
    pub note_type: String,
    pub content: String,
    pub data: Value,
    pub author_user_id: String,
    pub author_app_id: String,
    pub created_at: u64,
    pub updated_at: u64,
}

// ---------------------------------------------------------------------------
// Digest helper
// ---------------------------------------------------------------------------

/// Sha256 over a canonical (recursively key-sorted) JSON encoding. Used for
/// `input_digest` and for the immutable part of idempotent create requests.
pub fn compute_task_input_digest(input: &Value) -> String {
    let mut hasher = Sha256::new();
    hash_canonical_json_value(input, &mut hasher);
    format!("{:x}", hasher.finalize())
}

fn hash_canonical_json_value(value: &Value, hasher: &mut Sha256) {
    match value {
        Value::Object(map) => {
            hasher.update(b"{");
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                hasher.update(key.as_bytes());
                hasher.update(b":");
                hash_canonical_json_value(&map[key], hasher);
                hasher.update(b",");
            }
            hasher.update(b"}");
        }
        Value::Array(items) => {
            hasher.update(b"[");
            for item in items {
                hash_canonical_json_value(item, hasher);
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
// Request / result payloads
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

/// Executor requested at create time. A plain `create_task` can only bind the
/// caller's own authenticated app instance, or a human set — binding another
/// AppInstance requires the Dispatcher or an explicit system-level Reassign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CreateTaskExecutor {
    /// Bind the authenticated caller instance itself: phase starts Accepted.
    SelfApp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_instance_id: Option<String>,
    },
    /// Human task: phase starts Waiting(HumanInput).
    HumanSet { assignees: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskReq {
    pub name: String,
    pub schema_id: String,
    /// None -> latest enabled revision of the schema.
    #[serde(default)]
    pub schema_version: Option<u32>,
    pub input: Value,
    pub executor: CreateTaskExecutor,
    #[serde(default)]
    pub parent_id: Option<TaskId>,
    #[serde(default)]
    pub child_control_policy: Option<ChildControlPolicy>,
    #[serde(default)]
    pub policy_preset: Option<String>,
    #[serde(default)]
    pub permission_boundary: bool,
    pub idempotency_key: String,
    #[serde(default)]
    pub retry_of: Option<TaskId>,
    #[serde(default)]
    pub supersedes: Option<TaskId>,
    #[serde(default)]
    pub message: Option<String>,
}
impl_from_json!(CreateTaskReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskReq {
    pub task_id: TaskId,
}
impl_from_json!(GetTaskReq);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListTasksReq {
    #[serde(default)]
    pub creator_user_id: Option<String>,
    #[serde(default)]
    pub creator_app_id: Option<String>,
    #[serde(default)]
    pub schema_id: Option<String>,
    #[serde(default)]
    pub phase: Option<TaskPhase>,
    #[serde(default)]
    pub root_id: Option<TaskId>,
    #[serde(default)]
    pub executor_kind: Option<TaskExecutorKind>,
    #[serde(default)]
    pub created_after: Option<u64>,
    #[serde(default)]
    pub created_before: Option<u64>,
    /// Include archived tasks (hidden by default).
    #[serde(default)]
    pub include_archived: bool,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}
impl_from_json!(ListTasksReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummaryPage {
    pub tasks: Vec<TaskSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskTreeReq {
    pub root_id: TaskId,
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}
impl_from_json!(GetTaskTreeReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubtasksReq {
    pub task_id: TaskId,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}
impl_from_json!(GetSubtasksReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveTaskReq {
    pub task_id: TaskId,
    pub expected_revision: u64,
}
impl_from_json!(ArchiveTaskReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestControlReq {
    pub task_id: TaskId,
    pub action: TaskControlAction,
    /// Client-supplied idempotency id for the control request.
    pub request_id: String,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub expected_revision: Option<u64>,
}
impl_from_json!(RequestControlReq);

/// Either a single updated task or the batch disposition for recursive calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RequestControlResult {
    Task { task: Task },
    Batch { result: BatchControlResult },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAssigneesReq {
    pub task_id: TaskId,
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
    pub expected_revision: u64,
}
impl_from_json!(UpdateAssigneesReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantTaskAccessReq {
    pub task_id: TaskId,
    pub grant: TaskAclGrantSpec,
    pub expected_revision: u64,
}
impl_from_json!(GrantTaskAccessReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokeTaskAccessReq {
    pub task_id: TaskId,
    pub grant_id: String,
    pub expected_revision: u64,
}
impl_from_json!(RevokeTaskAccessReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskAccessReq {
    pub task_id: TaskId,
}
impl_from_json!(ListTaskAccessReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskAccessResult {
    pub grants: Vec<TaskAclGrant>,
}

/// Common fencing envelope for App-runner state writes (doc §10.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerWriteEnvelope {
    pub task_id: TaskId,
    #[serde(default)]
    pub app_instance_id: Option<String>,
    pub runner_epoch: u64,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportStartedReq {
    #[serde(flatten)]
    pub envelope: RunnerWriteEnvelope,
}
impl_from_json!(ReportStartedReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportProgressReq {
    #[serde(flatten)]
    pub envelope: RunnerWriteEnvelope,
    #[serde(default)]
    pub progress: Option<Value>,
    #[serde(default)]
    pub message: Option<String>,
}
impl_from_json!(ReportProgressReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportWaitingReq {
    #[serde(flatten)]
    pub envelope: RunnerWriteEnvelope,
    pub reason: TaskWaitReason,
}
impl_from_json!(ReportWaitingReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRunningReq {
    #[serde(flatten)]
    pub envelope: RunnerWriteEnvelope,
}
impl_from_json!(ReportRunningReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateControlProfileReq {
    #[serde(flatten)]
    pub envelope: RunnerWriteEnvelope,
    pub profile: TaskControlProfile,
}
impl_from_json!(UpdateControlProfileReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckControlReq {
    #[serde(flatten)]
    pub envelope: RunnerWriteEnvelope,
    pub request_id: String,
    pub applied: bool,
    #[serde(default)]
    pub reject_reason: Option<String>,
}
impl_from_json!(AckControlReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitResultReq {
    pub task_id: TaskId,
    pub result: Value,
    /// Required for App runners; absent for human commits (identity comes
    /// from the verified token and the assignee set).
    #[serde(default)]
    pub app_instance_id: Option<String>,
    #[serde(default)]
    pub runner_epoch: Option<u64>,
    pub expected_revision: u64,
}
impl_from_json!(CommitResultReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailTaskReq {
    #[serde(flatten)]
    pub envelope: RunnerWriteEnvelope,
    pub error: TaskError,
}
impl_from_json!(FailTaskReq);

// --- Trusted promise/executor control-plane commands (doc §13.4) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePromisedTaskReq {
    pub name: String,
    pub schema_id: String,
    #[serde(default)]
    pub schema_version: Option<u32>,
    pub input: Value,
    /// Delegated creator envelope: the original directly-authenticated
    /// caller, verified by the trusted control plane (e.g. the dispatcher).
    pub creator: ActorRef,
    /// Digest check against the control plane's frozen copy.
    #[serde(default)]
    pub expected_input_digest: Option<String>,
    #[serde(default)]
    pub origin_ref: Option<TaskOriginRef>,
    #[serde(default)]
    pub parent_id: Option<TaskId>,
    #[serde(default)]
    pub child_control_policy: Option<ChildControlPolicy>,
    #[serde(default)]
    pub policy_preset: Option<String>,
    #[serde(default)]
    pub permission_boundary: bool,
    pub idempotency_key: String,
    #[serde(default)]
    pub wait_reason: Option<TaskWaitReason>,
    #[serde(default)]
    pub message: Option<String>,
}
impl_from_json!(CreatePromisedTaskReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetPromiseWaitReq {
    pub task_id: TaskId,
    pub wait_reason: TaskWaitReason,
    pub expected_revision: u64,
}
impl_from_json!(SetPromiseWaitReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindAppExecutorReq {
    pub task_id: TaskId,
    #[serde(default)]
    pub target_id: Option<String>,
    pub app_id: String,
    pub app_instance_id: String,
    /// Dispatcher delivery id recorded in the bind event for audit.
    #[serde(default)]
    pub delivery_id: Option<String>,
    pub expected_revision: u64,
}
impl_from_json!(BindAppExecutorReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseAppExecutorReq {
    pub task_id: TaskId,
    pub expected_instance_id: String,
    pub expected_runner_epoch: u64,
    pub reason: TaskWaitReason,
    pub expected_revision: u64,
}
impl_from_json!(ReleaseAppExecutorReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinishPromiseFailureReq {
    pub task_id: TaskId,
    pub error: TaskError,
    pub expected_revision: u64,
}
impl_from_json!(FinishPromiseFailureReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelPromisedTaskReq {
    pub task_id: TaskId,
    pub expected_revision: u64,
}
impl_from_json!(CancelPromisedTaskReq);

// --- Schema registry ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterTaskSchemaReq {
    pub definition: TaskSchemaDefinition,
}
impl_from_json!(RegisterTaskSchemaReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskSchemaReq {
    pub schema_id: String,
    /// None -> latest enabled revision.
    #[serde(default)]
    pub schema_version: Option<u32>,
}
impl_from_json!(GetTaskSchemaReq);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListTaskSchemasReq {
    #[serde(default)]
    pub user_creatable_only: bool,
    #[serde(default)]
    pub include_disabled: bool,
}
impl_from_json!(ListTaskSchemasReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskSchemasResult {
    pub schemas: Vec<TaskSchemaDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetTaskSchemaEnabledReq {
    pub schema_id: String,
    pub schema_version: u32,
    pub enabled: bool,
}
impl_from_json!(SetTaskSchemaEnabledReq);

// --- Events ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskEventsReq {
    /// Exactly one of task_id / root_id must be set.
    #[serde(default)]
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub root_id: Option<TaskId>,
    /// Return events with event_id strictly greater than this cursor.
    #[serde(default)]
    pub after_event_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}
impl_from_json!(ListTaskEventsReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskEventsResult {
    pub events: Vec<TaskEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// --- Notes ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTaskNoteReq {
    pub task_id: TaskId,
    #[serde(default)]
    pub note_type: Option<String>,
    pub content: String,
    #[serde(default)]
    pub data: Option<Value>,
}
impl_from_json!(AddTaskNoteReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskNotesReq {
    pub task_id: TaskId,
}
impl_from_json!(ListTaskNotesReq);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task: Task,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTaskNoteResult {
    pub note_id: TaskNoteId,
    pub note: TaskNote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTaskNotesResult {
    pub notes: Vec<TaskNote>,
}

// ---------------------------------------------------------------------------
// Handler trait (server side)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait TaskManagerHandler: Send + Sync {
    async fn handle_create_task(&self, req: CreateTaskReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("create_task not implemented".to_string()))
    }

    async fn handle_get_task(&self, req: GetTaskReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("get_task not implemented".to_string()))
    }

    async fn handle_list_tasks(&self, req: ListTasksReq, ctx: RPCContext) -> Result<TaskSummaryPage> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("list_tasks not implemented".to_string()))
    }

    async fn handle_get_task_tree(
        &self,
        req: GetTaskTreeReq,
        ctx: RPCContext,
    ) -> Result<TaskSummaryPage> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("get_task_tree not implemented".to_string()))
    }

    async fn handle_get_subtasks(
        &self,
        req: GetSubtasksReq,
        ctx: RPCContext,
    ) -> Result<TaskSummaryPage> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("get_subtasks not implemented".to_string()))
    }

    async fn handle_archive_task(&self, req: ArchiveTaskReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("archive_task not implemented".to_string()))
    }

    async fn handle_request_control(
        &self,
        req: RequestControlReq,
        ctx: RPCContext,
    ) -> Result<RequestControlResult> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("request_control not implemented".to_string()))
    }

    async fn handle_update_assignees(
        &self,
        req: UpdateAssigneesReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("update_assignees not implemented".to_string()))
    }

    async fn handle_grant_task_access(
        &self,
        req: GrantTaskAccessReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("grant_task_access not implemented".to_string()))
    }

    async fn handle_revoke_task_access(
        &self,
        req: RevokeTaskAccessReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("revoke_task_access not implemented".to_string()))
    }

    async fn handle_list_task_access(
        &self,
        req: ListTaskAccessReq,
        ctx: RPCContext,
    ) -> Result<ListTaskAccessResult> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("list_task_access not implemented".to_string()))
    }

    async fn handle_report_started(&self, req: ReportStartedReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("report_started not implemented".to_string()))
    }

    async fn handle_report_progress(&self, req: ReportProgressReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("report_progress not implemented".to_string()))
    }

    async fn handle_report_waiting(&self, req: ReportWaitingReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("report_waiting not implemented".to_string()))
    }

    async fn handle_report_running(&self, req: ReportRunningReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("report_running not implemented".to_string()))
    }

    async fn handle_update_control_profile(
        &self,
        req: UpdateControlProfileReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("update_control_profile not implemented".to_string()))
    }

    async fn handle_ack_control(&self, req: AckControlReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("ack_control not implemented".to_string()))
    }

    async fn handle_commit_result(&self, req: CommitResultReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("commit_result not implemented".to_string()))
    }

    async fn handle_fail_task(&self, req: FailTaskReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("fail_task not implemented".to_string()))
    }

    async fn handle_create_promised_task(
        &self,
        req: CreatePromisedTaskReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("create_promised_task not implemented".to_string()))
    }

    async fn handle_set_promise_wait(&self, req: SetPromiseWaitReq, ctx: RPCContext) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("set_promise_wait not implemented".to_string()))
    }

    async fn handle_bind_app_executor(
        &self,
        req: BindAppExecutorReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("bind_app_executor not implemented".to_string()))
    }

    async fn handle_release_app_executor(
        &self,
        req: ReleaseAppExecutorReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("release_app_executor not implemented".to_string()))
    }

    async fn handle_finish_promise_failure(
        &self,
        req: FinishPromiseFailureReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("finish_promise_failure not implemented".to_string()))
    }

    async fn handle_cancel_promised_task(
        &self,
        req: CancelPromisedTaskReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("cancel_promised_task not implemented".to_string()))
    }

    async fn handle_register_task_schema(
        &self,
        req: RegisterTaskSchemaReq,
        ctx: RPCContext,
    ) -> Result<TaskSchemaDefinition> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("register_task_schema not implemented".to_string()))
    }

    async fn handle_get_task_schema(
        &self,
        req: GetTaskSchemaReq,
        ctx: RPCContext,
    ) -> Result<TaskSchemaDefinition> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("get_task_schema not implemented".to_string()))
    }

    async fn handle_list_task_schemas(
        &self,
        req: ListTaskSchemasReq,
        ctx: RPCContext,
    ) -> Result<ListTaskSchemasResult> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("list_task_schemas not implemented".to_string()))
    }

    async fn handle_set_task_schema_enabled(
        &self,
        req: SetTaskSchemaEnabledReq,
        ctx: RPCContext,
    ) -> Result<TaskSchemaDefinition> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("set_task_schema_enabled not implemented".to_string()))
    }

    async fn handle_list_task_events(
        &self,
        req: ListTaskEventsReq,
        ctx: RPCContext,
    ) -> Result<ListTaskEventsResult> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("list_task_events not implemented".to_string()))
    }

    async fn handle_add_task_note(&self, req: AddTaskNoteReq, ctx: RPCContext) -> Result<TaskNote> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("add_task_note not implemented".to_string()))
    }

    async fn handle_list_task_notes(
        &self,
        req: ListTaskNotesReq,
        ctx: RPCContext,
    ) -> Result<Vec<TaskNote>> {
        let _ = (req, ctx);
        Err(RPCErrors::ReasonError("list_task_notes not implemented".to_string()))
    }
}


// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Client for the task-manager service.
///
/// `InProcess` exists for tests that inject a fake `TaskManagerHandler`;
/// production code always goes through `KRPC` (and therefore through the
/// server-side token authentication).
pub enum TaskManagerClient {
    InProcess(Box<dyn TaskManagerHandler>),
    KRPC(Box<kRPC>),
}

macro_rules! client_task_method {
    ($fn_name:ident, $handler:ident, $method:literal, $req:ty) => {
        pub async fn $fn_name(&self, req: $req) -> Result<Task> {
            match self {
                Self::InProcess(handler) => handler.$handler(req, RPCContext::default()).await,
                Self::KRPC(client) => {
                    let params = serde_json::to_value(&req).map_err(|e| {
                        RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                    })?;
                    let result = client.call($method, params).await?;
                    parse_task_result(result)
                }
            }
        }
    };
}

fn parse_task_result(result: Value) -> Result<Task> {
    serde_json::from_value::<TaskResult>(result)
        .map(|r| r.task)
        .map_err(|e| RPCErrors::ParserResponseError(format!("Expected task in response: {}", e)))
}

impl TaskManagerClient {
    pub fn new(client: kRPC) -> Self {
        Self::KRPC(Box::new(client))
    }

    pub fn new_in_process(handler: Box<dyn TaskManagerHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(client: Box<kRPC>) -> Self {
        Self::KRPC(client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => client.set_context(context).await,
        }
    }

    client_task_method!(create_task, handle_create_task, "create_task", CreateTaskReq);
    client_task_method!(archive_task, handle_archive_task, "archive_task", ArchiveTaskReq);
    client_task_method!(
        update_assignees,
        handle_update_assignees,
        "update_assignees",
        UpdateAssigneesReq
    );
    client_task_method!(
        grant_task_access,
        handle_grant_task_access,
        "grant_task_access",
        GrantTaskAccessReq
    );
    client_task_method!(
        revoke_task_access,
        handle_revoke_task_access,
        "revoke_task_access",
        RevokeTaskAccessReq
    );
    client_task_method!(
        report_started,
        handle_report_started,
        "report_started",
        ReportStartedReq
    );
    client_task_method!(
        report_progress,
        handle_report_progress,
        "report_progress",
        ReportProgressReq
    );
    client_task_method!(
        report_waiting,
        handle_report_waiting,
        "report_waiting",
        ReportWaitingReq
    );
    client_task_method!(
        report_running,
        handle_report_running,
        "report_running",
        ReportRunningReq
    );
    client_task_method!(
        update_control_profile,
        handle_update_control_profile,
        "update_control_profile",
        UpdateControlProfileReq
    );
    client_task_method!(ack_control, handle_ack_control, "ack_control", AckControlReq);
    client_task_method!(
        commit_result,
        handle_commit_result,
        "commit_result",
        CommitResultReq
    );
    client_task_method!(fail_task, handle_fail_task, "fail_task", FailTaskReq);
    client_task_method!(
        create_promised_task,
        handle_create_promised_task,
        "create_promised_task",
        CreatePromisedTaskReq
    );
    client_task_method!(
        set_promise_wait,
        handle_set_promise_wait,
        "set_promise_wait",
        SetPromiseWaitReq
    );
    client_task_method!(
        bind_app_executor,
        handle_bind_app_executor,
        "bind_app_executor",
        BindAppExecutorReq
    );
    client_task_method!(
        release_app_executor,
        handle_release_app_executor,
        "release_app_executor",
        ReleaseAppExecutorReq
    );
    client_task_method!(
        finish_promise_failure,
        handle_finish_promise_failure,
        "finish_promise_failure",
        FinishPromiseFailureReq
    );
    client_task_method!(
        cancel_promised_task,
        handle_cancel_promised_task,
        "cancel_promised_task",
        CancelPromisedTaskReq
    );

    pub async fn get_task(&self, task_id: &str) -> Result<Task> {
        let req = GetTaskReq {
            task_id: task_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => handler.handle_get_task(req, RPCContext::default()).await,
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("get_task", params).await?;
                parse_task_result(result)
            }
        }
    }

    pub async fn list_tasks(&self, req: ListTasksReq) -> Result<TaskSummaryPage> {
        match self {
            Self::InProcess(handler) => handler.handle_list_tasks(req, RPCContext::default()).await,
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("list_tasks", params).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected TaskSummaryPage: {}", e))
                })
            }
        }
    }

    pub async fn get_task_tree(&self, req: GetTaskTreeReq) -> Result<TaskSummaryPage> {
        match self {
            Self::InProcess(handler) => {
                handler.handle_get_task_tree(req, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("get_task_tree", params).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected TaskSummaryPage: {}", e))
                })
            }
        }
    }

    pub async fn get_subtasks(&self, req: GetSubtasksReq) -> Result<TaskSummaryPage> {
        match self {
            Self::InProcess(handler) => {
                handler.handle_get_subtasks(req, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("get_subtasks", params).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected TaskSummaryPage: {}", e))
                })
            }
        }
    }

    pub async fn request_control(&self, req: RequestControlReq) -> Result<RequestControlResult> {
        match self {
            Self::InProcess(handler) => {
                handler.handle_request_control(req, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("request_control", params).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected RequestControlResult: {}", e))
                })
            }
        }
    }

    pub async fn list_task_access(&self, task_id: &str) -> Result<Vec<TaskAclGrant>> {
        let req = ListTaskAccessReq {
            task_id: task_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => Ok(handler
                .handle_list_task_access(req, RPCContext::default())
                .await?
                .grants),
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("list_task_access", params).await?;
                let parsed: ListTaskAccessResult = serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected ListTaskAccessResult: {}", e))
                })?;
                Ok(parsed.grants)
            }
        }
    }

    pub async fn register_task_schema(
        &self,
        definition: TaskSchemaDefinition,
    ) -> Result<TaskSchemaDefinition> {
        let req = RegisterTaskSchemaReq { definition };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_register_task_schema(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("register_task_schema", params).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected TaskSchemaDefinition: {}", e))
                })
            }
        }
    }

    pub async fn get_task_schema(
        &self,
        schema_id: &str,
        schema_version: Option<u32>,
    ) -> Result<TaskSchemaDefinition> {
        let req = GetTaskSchemaReq {
            schema_id: schema_id.to_string(),
            schema_version,
        };
        match self {
            Self::InProcess(handler) => {
                handler.handle_get_task_schema(req, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("get_task_schema", params).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected TaskSchemaDefinition: {}", e))
                })
            }
        }
    }

    pub async fn list_task_schemas(&self, req: ListTaskSchemasReq) -> Result<Vec<TaskSchemaDefinition>> {
        match self {
            Self::InProcess(handler) => Ok(handler
                .handle_list_task_schemas(req, RPCContext::default())
                .await?
                .schemas),
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("list_task_schemas", params).await?;
                let parsed: ListTaskSchemasResult = serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected ListTaskSchemasResult: {}", e))
                })?;
                Ok(parsed.schemas)
            }
        }
    }

    pub async fn set_task_schema_enabled(
        &self,
        schema_id: &str,
        schema_version: u32,
        enabled: bool,
    ) -> Result<TaskSchemaDefinition> {
        let req = SetTaskSchemaEnabledReq {
            schema_id: schema_id.to_string(),
            schema_version,
            enabled,
        };
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_set_task_schema_enabled(req, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("set_task_schema_enabled", params).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected TaskSchemaDefinition: {}", e))
                })
            }
        }
    }

    pub async fn list_task_events(&self, req: ListTaskEventsReq) -> Result<ListTaskEventsResult> {
        match self {
            Self::InProcess(handler) => {
                handler.handle_list_task_events(req, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("list_task_events", params).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected ListTaskEventsResult: {}", e))
                })
            }
        }
    }

    pub async fn add_task_note(
        &self,
        task_id: &str,
        note_type: Option<&str>,
        content: &str,
        data: Option<Value>,
    ) -> Result<TaskNote> {
        let req = AddTaskNoteReq {
            task_id: task_id.to_string(),
            note_type: note_type.map(|s| s.to_string()),
            content: content.to_string(),
            data,
        };
        match self {
            Self::InProcess(handler) => {
                handler.handle_add_task_note(req, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("add_task_note", params).await?;
                let parsed: AddTaskNoteResult = serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected AddTaskNoteResult: {}", e))
                })?;
                Ok(parsed.note)
            }
        }
    }

    pub async fn list_task_notes(&self, task_id: &str) -> Result<Vec<TaskNote>> {
        let req = ListTaskNotesReq {
            task_id: task_id.to_string(),
        };
        match self {
            Self::InProcess(handler) => {
                handler.handle_list_task_notes(req, RPCContext::default()).await
            }
            Self::KRPC(client) => {
                let params = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("list_task_notes", params).await?;
                let parsed: ListTaskNotesResult = serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!("Expected ListTaskNotesResult: {}", e))
                })?;
                Ok(parsed.notes)
            }
        }
    }

    // --- Convenience wrappers ---

    /// Request a cancel on a task with a fresh request id.
    pub async fn cancel_task(&self, task_id: &str, recursive: bool) -> Result<RequestControlResult> {
        self.request_control(RequestControlReq {
            task_id: task_id.to_string(),
            action: TaskControlAction::Cancel,
            request_id: format!("ctl-{}", uuid::Uuid::new_v4().simple()),
            recursive,
            expected_revision: None,
        })
        .await
    }

    // --- Self-driving runner shortcuts ---
    //
    // For services that run their own (SelfApp / bound) tasks: each call
    // fetches the fresh snapshot for the fencing envelope and retries once
    // on a revision race. They are conveniences over the report_*/commit
    // commands, not a different protocol.

    fn snapshot_envelope(task: &Task) -> RunnerWriteEnvelope {
        let app_instance_id = match &task.executor {
            TaskExecutor::App {
                app_instance_id, ..
            } => app_instance_id.clone(),
            _ => None,
        };
        RunnerWriteEnvelope {
            task_id: task.task_id.clone(),
            app_instance_id,
            runner_epoch: task.runner_epoch,
            expected_revision: task.revision,
        }
    }

    /// Drive the task into Running from Accepted (report_started) or
    /// Waiting (report_running). Running/Paused/Terminal pass through.
    pub async fn runner_start(&self, task_id: &str) -> Result<Task> {
        let task = self.get_task(task_id).await?;
        let attempt = match task.phase {
            TaskPhase::Accepted => {
                self.report_started(ReportStartedReq {
                    envelope: Self::snapshot_envelope(&task),
                })
                .await
            }
            TaskPhase::Waiting => {
                self.report_running(ReportRunningReq {
                    envelope: Self::snapshot_envelope(&task),
                })
                .await
            }
            _ => return Ok(task),
        };
        match attempt {
            Ok(task) => Ok(task),
            Err(err)
                if matches!(
                    task_mgr_error_code(&err),
                    Some(TASK_ERR_REVISION_CONFLICT) | Some(TASK_ERR_INVALID_PHASE)
                ) =>
            {
                self.get_task(task_id).await
            }
            Err(err) => Err(err),
        }
    }

    /// Write the mutable progress snapshot / display message.
    pub async fn runner_progress(
        &self,
        task_id: &str,
        progress: Option<Value>,
        message: Option<String>,
    ) -> Result<Task> {
        let task = self.get_task(task_id).await?;
        if task.phase.is_terminal() {
            return Ok(task);
        }
        let attempt = self
            .report_progress(ReportProgressReq {
                envelope: Self::snapshot_envelope(&task),
                progress: progress.clone(),
                message: message.clone(),
            })
            .await;
        match attempt {
            Ok(task) => Ok(task),
            Err(err) if task_mgr_error_code(&err) == Some(TASK_ERR_REVISION_CONFLICT) => {
                let fresh = self.get_task(task_id).await?;
                self.report_progress(ReportProgressReq {
                    envelope: Self::snapshot_envelope(&fresh),
                    progress,
                    message,
                })
                .await
            }
            Err(err) => Err(err),
        }
    }

    /// Park the task in Waiting with the given reason (starting it first
    /// when it has not run yet).
    pub async fn runner_wait(&self, task_id: &str, reason: TaskWaitReason) -> Result<Task> {
        let task = self.runner_start(task_id).await?;
        if task.phase != TaskPhase::Running {
            return Ok(task);
        }
        self.report_waiting(ReportWaitingReq {
            envelope: Self::snapshot_envelope(&task),
            reason,
        })
        .await
    }

    /// Commit the one-shot Result. `task_already_completed` races resolve to
    /// the final task snapshot.
    pub async fn runner_complete(&self, task_id: &str, result: Value) -> Result<Task> {
        let task = self.get_task(task_id).await?;
        if task.phase.is_terminal() {
            return Ok(task);
        }
        let attempt = self
            .commit_result(CommitResultReq {
                task_id: task.task_id.clone(),
                result: result.clone(),
                app_instance_id: Self::snapshot_envelope(&task).app_instance_id,
                runner_epoch: Some(task.runner_epoch),
                expected_revision: task.revision,
            })
            .await;
        match attempt {
            Ok(task) => Ok(task),
            Err(err) if task_mgr_error_code(&err) == Some(TASK_ERR_REVISION_CONFLICT) => {
                let fresh = self.get_task(task_id).await?;
                if fresh.phase.is_terminal() {
                    return Ok(fresh);
                }
                self.commit_result(CommitResultReq {
                    task_id: fresh.task_id.clone(),
                    result,
                    app_instance_id: Self::snapshot_envelope(&fresh).app_instance_id,
                    runner_epoch: Some(fresh.runner_epoch),
                    expected_revision: fresh.revision,
                })
                .await
            }
            Err(err) => Err(err),
        }
    }

    /// Close the task as Failed with a stable error code.
    pub async fn runner_fail(
        &self,
        task_id: &str,
        code: &str,
        message: impl Into<String>,
        detail: Option<Value>,
    ) -> Result<Task> {
        let task = self.get_task(task_id).await?;
        if task.phase.is_terminal() {
            return Ok(task);
        }
        self.fail_task(FailTaskReq {
            envelope: Self::snapshot_envelope(&task),
            error: TaskError {
                code: code.to_string(),
                message: message.into(),
                detail,
            },
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Server-side RPC dispatch
// ---------------------------------------------------------------------------

pub struct TaskManagerServerHandler<T: TaskManagerHandler>(pub T);

impl<T: TaskManagerHandler> TaskManagerServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

macro_rules! dispatch_task_method {
    ($self:ident, $ctx:ident, $params:expr, $req:ty, $handler:ident) => {{
        let req = <$req>::from_json($params)?;
        let task = $self.0.$handler(req, $ctx).await?;
        RPCResult::Success(json!(TaskResult { task }))
    }};
}

#[async_trait]
impl<T: TaskManagerHandler> RPCHandler for TaskManagerServerHandler<T> {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            "create_task" => {
                dispatch_task_method!(self, ctx, req.params, CreateTaskReq, handle_create_task)
            }
            "get_task" => dispatch_task_method!(self, ctx, req.params, GetTaskReq, handle_get_task),
            "list_tasks" => {
                let list_req = ListTasksReq::from_json(req.params)?;
                let page = self.0.handle_list_tasks(list_req, ctx).await?;
                RPCResult::Success(json!(page))
            }
            "get_task_tree" => {
                let tree_req = GetTaskTreeReq::from_json(req.params)?;
                let page = self.0.handle_get_task_tree(tree_req, ctx).await?;
                RPCResult::Success(json!(page))
            }
            "get_subtasks" => {
                let sub_req = GetSubtasksReq::from_json(req.params)?;
                let page = self.0.handle_get_subtasks(sub_req, ctx).await?;
                RPCResult::Success(json!(page))
            }
            "archive_task" => {
                dispatch_task_method!(self, ctx, req.params, ArchiveTaskReq, handle_archive_task)
            }
            "request_control" => {
                let control_req = RequestControlReq::from_json(req.params)?;
                let result = self.0.handle_request_control(control_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "update_assignees" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                UpdateAssigneesReq,
                handle_update_assignees
            ),
            "grant_task_access" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                GrantTaskAccessReq,
                handle_grant_task_access
            ),
            "revoke_task_access" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                RevokeTaskAccessReq,
                handle_revoke_task_access
            ),
            "list_task_access" => {
                let access_req = ListTaskAccessReq::from_json(req.params)?;
                let result = self.0.handle_list_task_access(access_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "report_started" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                ReportStartedReq,
                handle_report_started
            ),
            "report_progress" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                ReportProgressReq,
                handle_report_progress
            ),
            "report_waiting" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                ReportWaitingReq,
                handle_report_waiting
            ),
            "report_running" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                ReportRunningReq,
                handle_report_running
            ),
            "update_control_profile" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                UpdateControlProfileReq,
                handle_update_control_profile
            ),
            "ack_control" => {
                dispatch_task_method!(self, ctx, req.params, AckControlReq, handle_ack_control)
            }
            "commit_result" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                CommitResultReq,
                handle_commit_result
            ),
            "fail_task" => {
                dispatch_task_method!(self, ctx, req.params, FailTaskReq, handle_fail_task)
            }
            "create_promised_task" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                CreatePromisedTaskReq,
                handle_create_promised_task
            ),
            "set_promise_wait" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                SetPromiseWaitReq,
                handle_set_promise_wait
            ),
            "bind_app_executor" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                BindAppExecutorReq,
                handle_bind_app_executor
            ),
            "release_app_executor" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                ReleaseAppExecutorReq,
                handle_release_app_executor
            ),
            "finish_promise_failure" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                FinishPromiseFailureReq,
                handle_finish_promise_failure
            ),
            "cancel_promised_task" => dispatch_task_method!(
                self,
                ctx,
                req.params,
                CancelPromisedTaskReq,
                handle_cancel_promised_task
            ),
            "register_task_schema" => {
                let schema_req = RegisterTaskSchemaReq::from_json(req.params)?;
                let definition = self.0.handle_register_task_schema(schema_req, ctx).await?;
                RPCResult::Success(json!(definition))
            }
            "get_task_schema" => {
                let schema_req = GetTaskSchemaReq::from_json(req.params)?;
                let definition = self.0.handle_get_task_schema(schema_req, ctx).await?;
                RPCResult::Success(json!(definition))
            }
            "list_task_schemas" => {
                let schema_req = ListTaskSchemasReq::from_json(req.params)?;
                let result = self.0.handle_list_task_schemas(schema_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "set_task_schema_enabled" => {
                let schema_req = SetTaskSchemaEnabledReq::from_json(req.params)?;
                let definition = self
                    .0
                    .handle_set_task_schema_enabled(schema_req, ctx)
                    .await?;
                RPCResult::Success(json!(definition))
            }
            "list_task_events" => {
                let events_req = ListTaskEventsReq::from_json(req.params)?;
                let result = self.0.handle_list_task_events(events_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "add_task_note" => {
                let note_req = AddTaskNoteReq::from_json(req.params)?;
                let note = self.0.handle_add_task_note(note_req, ctx).await?;
                RPCResult::Success(json!(AddTaskNoteResult {
                    note_id: note.id,
                    note
                }))
            }
            "list_task_notes" => {
                let note_req = ListTaskNotesReq::from_json(req.params)?;
                let notes = self.0.handle_list_task_notes(note_req, ctx).await?;
                RPCResult::Success(json!(ListTaskNotesResult { notes }))
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
    fn executor_serde_roundtrip() {
        let app = TaskExecutor::App {
            target_id: Some("t1".into()),
            app_id: "app".into(),
            app_instance_id: Some("i1".into()),
        };
        let json = serde_json::to_value(&app).unwrap();
        assert_eq!(json["kind"], "App");
        let back: TaskExecutor = serde_json::from_value(json).unwrap();
        assert_eq!(back, app);

        let unbound_json = serde_json::to_value(TaskExecutor::Unbound).unwrap();
        assert_eq!(unbound_json["kind"], "Unbound");
    }

    #[test]
    fn error_code_roundtrip() {
        let err = task_mgr_error(TASK_ERR_REVISION_CONFLICT, "expected 3, found 5");
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_REVISION_CONFLICT));

        let denied = task_mgr_error(TASK_ERR_PERMISSION_DENIED, "no Control action");
        assert!(matches!(denied, RPCErrors::NoPermission(_)));
        assert_eq!(task_mgr_error_code(&denied), Some(TASK_ERR_PERMISSION_DENIED));

        let unrelated = RPCErrors::ReasonError("task_not_found_elsewhere: x".into());
        assert_eq!(task_mgr_error_code(&unrelated), None);
    }

    #[test]
    fn digest_is_key_order_insensitive() {
        let a = json!({"b": 1, "a": {"y": 2, "x": [1, 2]}});
        let b = json!({"a": {"x": [1, 2], "y": 2}, "b": 1});
        assert_eq!(compute_task_input_digest(&a), compute_task_input_digest(&b));
        assert_ne!(
            compute_task_input_digest(&a),
            compute_task_input_digest(&json!({"a": {"x": [2, 1], "y": 2}, "b": 1}))
        );
    }

    #[test]
    fn control_profile_serde() {
        let profile = TaskControlProfile {
            pause: ControlAvailability::Available,
            resume: ControlAvailability::Unavailable {
                reason: Some("not yet started".into()),
            },
            cancel: CancelCapability::Safe,
            updated_at: 42,
        };
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["pause"]["kind"], "Available");
        assert_eq!(json["cancel"]["kind"], "Safe");
        let back: TaskControlProfile = serde_json::from_value(json).unwrap();
        assert_eq!(back, profile);
    }
}
