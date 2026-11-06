//! Workflow Service 注册元数据。
//!
//! 与 task-manager / msg-center / aicc 等其他 kernel service 保持一致：
//! unique id / 端口 / [`AppDoc`] 生成集中放在 buckyos-api，
//! workflow service 自身和 scheduler 都从这里取，避免常量分裂。

use crate::workflow_dsl::WorkflowDefinition;
use crate::workflow_runtime::{EventEnvelope, NodeRunState, RunStatus};
use crate::{AppDoc, AppType, FunctionObject, FunctionRegistration, SelectorType, ThunkExecutionError};
use ::kRPC::*;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const WORKFLOW_SERVICE_UNIQUE_ID: &str = "workflow";
pub const WORKFLOW_SERVICE_NAME: &str = "workflow";
pub const WORKFLOW_SERVICE_PORT: u16 = 4070;
pub const WORKFLOW_SERVICE_HTTP_PATH: &str = "/kapi/workflow";

/// `zone/workflow/zone/artifacts`: files produced by workflow functions
/// (page images, extracted text, frames, audio tracks). Temp data, 30 days.
pub const WORKFLOW_BUCKET_ARTIFACTS: &str = "artifacts";
pub const WORKFLOW_ARTIFACTS_TTL_DAYS: u32 = 30;
/// `zone/workflow/zone/scratch`: inputs frozen for a run. Temp data, 7 days.
pub const WORKFLOW_BUCKET_SCRATCH: &str = "scratch";
pub const WORKFLOW_SCRATCH_TTL_DAYS: u32 = 7;
pub const WORKFLOW_STORAGE_QUOTA_MIB: u64 = 16384;

pub fn workflow_bucket_decls() -> std::collections::HashMap<String, crate::bucket_client::BucketDecl> {
    use crate::bucket_client::{BucketDecl, BucketKind, BucketScope, DataPolicy};
    let temp = |days| BucketDecl {
        kind: BucketKind::Data,
        data_policy: DataPolicy::Temp,
        scope: BucketScope::Zone,
        versioning: None,
        quota_mib: None,
        max_objects: None,
        default_ttl_days: Some(days),
        lifecycle: Vec::new(),
    };
    std::collections::HashMap::from([
        (WORKFLOW_BUCKET_ARTIFACTS.to_string(), temp(WORKFLOW_ARTIFACTS_TTL_DAYS)),
        (WORKFLOW_BUCKET_SCRATCH.to_string(), temp(WORKFLOW_SCRATCH_TTL_DAYS)),
    ])
}

pub fn workflow_artifacts_bucket() -> crate::bucket_client::BucketId {
    crate::bucket_client::BucketId::zone(WORKFLOW_SERVICE_UNIQUE_ID, WORKFLOW_BUCKET_ARTIFACTS)
        .expect("valid workflow bucket id")
}

pub fn workflow_scratch_bucket() -> crate::bucket_client::BucketId {
    crate::bucket_client::BucketId::zone(WORKFLOW_SERVICE_UNIQUE_ID, WORKFLOW_BUCKET_SCRATCH)
        .expect("valid workflow bucket id")
}

pub fn generate_workflow_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        WORKFLOW_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Workflow Service")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WorkflowOwner {
    pub user_id: String,
    pub app_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScheduledTaskStatus {
    Enabled,
    Paused,
    Archived,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WorkflowScheduledTaskSchedule {
    Cron {
        expr: String,
        timezone: String,
        #[serde(default)]
        calendar: Option<String>,
        #[serde(default)]
        start_at: Option<i64>,
        #[serde(default)]
        end_at: Option<i64>,
    },
    Once {
        run_at: i64,
        #[serde(default)]
        timezone: Option<String>,
    },
    RunEvery {
        every_sec: u64,
        #[serde(default)]
        start_at: Option<i64>,
        #[serde(default)]
        end_at: Option<i64>,
        #[serde(default)]
        timezone: Option<String>,
    },
}

/// beta2.2: the `runner` dispatch field was removed — a schedule target only
/// names a task_type in the workflow service's own execution domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowScheduledTaskTarget {
    pub task_type: String,
    pub name_template: String,
    #[serde(default)]
    pub data_template: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScheduledTaskMisfirePolicy {
    Skip,
    RunOnce,
    CatchUp,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowScheduledTaskPolicy {
    pub misfire: WorkflowScheduledTaskMisfirePolicy,
    pub max_parallel_runs: u32,
    pub catch_up_limit: u32,
    pub jitter_sec: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowScheduledTaskMirror {
    #[serde(default)]
    pub root_task_id: Option<i64>,
    #[serde(default)]
    pub root_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowScheduledTaskState {
    #[serde(default)]
    pub next_fire_at: Option<i64>,
    #[serde(default)]
    pub last_fire_at: Option<i64>,
    #[serde(default)]
    pub last_task_id: Option<i64>,
    #[serde(default)]
    pub last_run_id: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowScheduledTask {
    pub schedule_id: String,
    pub owner: WorkflowOwner,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub status: WorkflowScheduledTaskStatus,
    pub schedule: WorkflowScheduledTaskSchedule,
    pub target: WorkflowScheduledTaskTarget,
    pub state: WorkflowScheduledTaskState,
    pub policy: WorkflowScheduledTaskPolicy,
    #[serde(default)]
    pub task_mirror: WorkflowScheduledTaskMirror,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScheduledTaskFireStatus {
    Created,
    TaskCreated,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowScheduledTaskFireRecord {
    pub fire_id: String,
    pub schedule_id: String,
    pub fire_key: String,
    pub fire_time: i64,
    pub manual: bool,
    pub status: WorkflowScheduledTaskFireStatus,
    #[serde(default)]
    pub task_id: Option<i64>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCreateScheduledTaskReq {
    pub owner: WorkflowOwner,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub schedule: WorkflowScheduledTaskSchedule,
    pub target: WorkflowScheduledTaskTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<WorkflowScheduledTaskPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<WorkflowScheduledTaskStatus>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowUpdateScheduledTaskReq {
    pub schedule_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<WorkflowScheduledTaskSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<WorkflowScheduledTaskTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<WorkflowScheduledTaskPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowScheduledTaskIdReq {
    pub schedule_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowListScheduledTasksReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<WorkflowOwner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<WorkflowScheduledTaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunScheduledTaskNowReq {
    pub schedule_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_time: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGetScheduledTaskHistoryReq {
    pub schedule_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowValidateScheduledTaskReq {
    pub schedule: WorkflowScheduledTaskSchedule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<WorkflowScheduledTaskTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowValidateScheduledTaskResult {
    pub valid: bool,
    #[serde(default)]
    pub normalized_expr: Option<String>,
    pub timezone: String,
    #[serde(default)]
    pub next_fire_times: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Structured execution error of a step / run (`{code, message, details, retryable}`).
pub type WorkflowExecutionError = ThunkExecutionError;

/// `{ok:false, error, message, detail}` as the service returns it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowRpcError {
    #[serde(rename = "error")]
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

impl WorkflowRpcError {
    pub fn from_response(value: &Value) -> Option<Self> {
        if value.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return None;
        }
        let code = value.get("error").and_then(Value::as_str)?.to_string();
        Some(Self {
            code,
            message: value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            detail: value.get("detail").cloned().unwrap_or(Value::Null),
        })
    }

    /// Decodes the error a `WorkflowServiceClient` call surfaced.
    pub fn from_rpc_error(err: &RPCErrors) -> Option<Self> {
        let detail = err.detail()?;
        Some(Self {
            code: detail.code.clone(),
            message: detail.message.clone(),
            detail: detail.data.clone().unwrap_or(Value::Null),
        })
    }

    pub fn is_not_found(&self) -> bool {
        self.code.ends_with("_not_found")
    }
}

impl WorkflowRpcError {
    pub fn into_rpc_error(self) -> RPCErrors {
        let data = (!self.detail.is_null()).then(|| self.detail.clone());
        RPCErrors::Detailed(RPCErrorDetail::new(self.code, self.message, data))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowDefinitionSource {
    System,
    UserImported,
    AppRegistered,
    AgentGenerated,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowDefinitionStatus {
    Draft,
    Active,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSubmitDefinitionReq {
    pub definition: WorkflowDefinition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<WorkflowDefinitionSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<WorkflowOwner>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSubmitDefinitionResult {
    pub workflow_id: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub revision: String,
    pub source: WorkflowDefinitionSource,
    #[serde(default)]
    pub analysis: Value,
    #[serde(default)]
    pub definition: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinitionSummary {
    pub id: String,
    pub name: String,
    pub owner: WorkflowOwner,
    pub source: WorkflowDefinitionSource,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub revision: String,
    pub status: WorkflowDefinitionStatus,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub warnings: u64,
    #[serde(default)]
    pub errors: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinitionRecord {
    pub id: String,
    pub name: String,
    pub owner: WorkflowOwner,
    pub source: WorkflowDefinitionSource,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub resolved_functions: BTreeMap<String, String>,
    pub definition: WorkflowDefinition,
    #[serde(default)]
    pub compiled: Value,
    #[serde(default)]
    pub analysis: Value,
    pub status: WorkflowDefinitionStatus,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowListDefinitionsReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<WorkflowOwner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<WorkflowDefinitionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<WorkflowDefinitionSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDryRunResult {
    #[serde(default)]
    pub analysis: Value,
    #[serde(default)]
    pub graph: Value,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub resolved_functions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCreateRunReq {
    /// Record id, or the logical id of a system definition (`media.pdf`).
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub input: Value,
    #[serde(default)]
    pub auto_start: bool,
    /// Join the caller's active run with the same definition revision,
    /// normalized input and environment revision instead of creating one.
    #[serde(default)]
    pub ensure: bool,
    /// How long the service may wait for a terminal state before answering
    /// (capped at 5000 ms by the service).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<WorkflowOwner>,
}

impl WorkflowCreateRunReq {
    pub fn new(workflow_id: impl Into<String>, input: Value) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            input,
            auto_start: false,
            ensure: false,
            wait_ms: None,
            callback_url: None,
            owner: None,
        }
    }

    pub fn ensure(mut self, wait_ms: u64) -> Self {
        self.auto_start = true;
        self.ensure = true;
        self.wait_ms = Some(wait_ms);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCreateRunResult {
    pub run_id: String,
    #[serde(default)]
    pub workflow_id: String,
    pub status: RunStatus,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub reused: bool,
    #[serde(default)]
    pub output: BTreeMap<String, Value>,
    #[serde(default)]
    pub events: Vec<EventEnvelope>,
    #[serde(default)]
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStartRunReq {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCancelRunReq {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunTransition {
    pub run_id: String,
    pub status: RunStatus,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub events: Vec<EventEnvelope>,
    #[serde(default)]
    pub from_seq: u64,
    #[serde(default)]
    pub to_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunGraph {
    pub run_id: String,
    pub workflow_id: String,
    #[serde(default)]
    pub workflow_name: String,
    pub status: RunStatus,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub requested_by: Option<String>,
    #[serde(default)]
    pub executor: Option<String>,
    #[serde(default)]
    pub owner: Option<WorkflowOwner>,
    #[serde(default)]
    pub graph: Value,
    #[serde(default)]
    pub nodes: Value,
    #[serde(default)]
    pub node_states: BTreeMap<String, NodeRunState>,
    #[serde(default)]
    pub node_outputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub node_errors: BTreeMap<String, WorkflowExecutionError>,
    #[serde(default)]
    pub node_sources: BTreeMap<String, String>,
    #[serde(default)]
    pub node_attempts: BTreeMap<String, u32>,
    #[serde(default)]
    pub human_waiting_nodes: Vec<String>,
    #[serde(default)]
    pub pending_thunks: Value,
    #[serde(default)]
    pub metrics: BTreeMap<String, Value>,
    #[serde(default)]
    pub output: BTreeMap<String, Value>,
    #[serde(default)]
    pub deadline_at: Option<i64>,
    #[serde(default)]
    pub execution_epoch: u64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub seq: u64,
}

impl WorkflowRunGraph {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            RunStatus::Completed | RunStatus::Failed | RunStatus::Aborted | RunStatus::BudgetExhausted
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowRunDelivery {
    #[serde(default)]
    pub artifacts: Vec<String>,
    /// How many objects the requester may now read through NDM.
    #[serde(default)]
    pub held: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunOutput {
    pub run_id: String,
    pub status: RunStatus,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub output: BTreeMap<String, Value>,
    #[serde(default)]
    pub node_errors: BTreeMap<String, WorkflowExecutionError>,
    #[serde(default)]
    pub run_error: Option<WorkflowExecutionError>,
    #[serde(default)]
    pub delivery: WorkflowRunDelivery,
}

impl WorkflowRunOutput {
    /// The first structured error of a failed run: the run-level one, else
    /// the first failed node's.
    pub fn first_error(&self) -> Option<&WorkflowExecutionError> {
        self.run_error
            .as_ref()
            .or_else(|| self.node_errors.values().next())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowListRunsReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<WorkflowOwner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunSummary {
    pub run_id: String,
    pub workflow_id: String,
    #[serde(default)]
    pub workflow_name: String,
    pub status: RunStatus,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub owner: Option<WorkflowOwner>,
    #[serde(default)]
    pub requested_by: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub progress: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowHistoryReq {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowHistoryResult {
    pub run_id: String,
    #[serde(default)]
    pub status: Option<RunStatus>,
    #[serde(default)]
    pub events: Vec<EventEnvelope>,
    #[serde(default)]
    pub next_seq: u64,
    #[serde(default)]
    pub current_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowFunctionInfo {
    pub fun_id: String,
    pub registration: FunctionRegistration,
    pub object: FunctionObject,
    #[serde(default)]
    pub published: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCapabilities {
    #[serde(default)]
    pub execution_epoch: u64,
    #[serde(default)]
    pub environment: Value,
    #[serde(default)]
    pub functions: u64,
}

pub struct WorkflowServiceClient {
    krpc_client: Box<kRPC>,
}

impl WorkflowServiceClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self { krpc_client }
    }

    pub async fn set_context(&self, context: RPCContext) {
        self.krpc_client.set_context(context).await;
    }

    pub async fn create_scheduled_task(
        &self,
        request: WorkflowCreateScheduledTaskReq,
    ) -> Result<WorkflowScheduledTask> {
        let value = self.call_ok("create_scheduled_task", request).await?;
        value
            .get("schedule")
            .cloned()
            .ok_or_else(|| RPCErrors::ParserResponseError("missing `schedule`".to_string()))
            .and_then(parse_response)
    }

    pub async fn update_scheduled_task(
        &self,
        request: WorkflowUpdateScheduledTaskReq,
    ) -> Result<WorkflowScheduledTask> {
        let value = self.call_ok("update_scheduled_task", request).await?;
        value
            .get("schedule")
            .cloned()
            .ok_or_else(|| RPCErrors::ParserResponseError("missing `schedule`".to_string()))
            .and_then(parse_response)
    }

    pub async fn get_scheduled_task(&self, schedule_id: &str) -> Result<WorkflowScheduledTask> {
        let value = self
            .call_ok(
                "get_scheduled_task",
                WorkflowScheduledTaskIdReq {
                    schedule_id: schedule_id.to_string(),
                },
            )
            .await?;
        value
            .get("schedule")
            .cloned()
            .ok_or_else(|| RPCErrors::ParserResponseError("missing `schedule`".to_string()))
            .and_then(parse_response)
    }

    pub async fn list_scheduled_tasks(
        &self,
        request: WorkflowListScheduledTasksReq,
    ) -> Result<Vec<WorkflowScheduledTask>> {
        let value = self.call_ok("list_scheduled_tasks", request).await?;
        value
            .get("schedules")
            .cloned()
            .ok_or_else(|| RPCErrors::ParserResponseError("missing `schedules`".to_string()))
            .and_then(parse_response)
    }

    pub async fn pause_scheduled_task(&self, schedule_id: &str) -> Result<WorkflowScheduledTask> {
        self.set_scheduled_task_state("pause_scheduled_task", schedule_id)
            .await
    }

    pub async fn resume_scheduled_task(&self, schedule_id: &str) -> Result<WorkflowScheduledTask> {
        self.set_scheduled_task_state("resume_scheduled_task", schedule_id)
            .await
    }

    pub async fn archive_scheduled_task(&self, schedule_id: &str) -> Result<WorkflowScheduledTask> {
        self.set_scheduled_task_state("archive_scheduled_task", schedule_id)
            .await
    }

    pub async fn run_scheduled_task_now(
        &self,
        request: WorkflowRunScheduledTaskNowReq,
    ) -> Result<WorkflowScheduledTaskFireRecord> {
        let value = self.call_ok("run_scheduled_task_now", request).await?;
        value
            .get("fire")
            .cloned()
            .ok_or_else(|| RPCErrors::ParserResponseError("missing `fire`".to_string()))
            .and_then(parse_response)
    }

    pub async fn get_scheduled_task_history(
        &self,
        request: WorkflowGetScheduledTaskHistoryReq,
    ) -> Result<Vec<WorkflowScheduledTaskFireRecord>> {
        let value = self.call_ok("get_scheduled_task_history", request).await?;
        value
            .get("fires")
            .cloned()
            .ok_or_else(|| RPCErrors::ParserResponseError("missing `fires`".to_string()))
            .and_then(parse_response)
    }

    pub async fn validate_scheduled_task(
        &self,
        request: WorkflowValidateScheduledTaskReq,
    ) -> Result<WorkflowValidateScheduledTaskResult> {
        let value = self.call_ok("validate_scheduled_task", request).await?;
        parse_response(value)
    }

    // ----- Definition / Function -----------------------------------------

    pub async fn submit_definition(
        &self,
        request: WorkflowSubmitDefinitionReq,
    ) -> Result<WorkflowSubmitDefinitionResult> {
        let value = self.call_ok("submit_definition", request).await?;
        parse_response(value)
    }

    pub async fn get_definition(&self, workflow_id: &str) -> Result<WorkflowDefinitionRecord> {
        let value = self
            .call_ok("get_definition", json!({ "workflow_id": workflow_id }))
            .await?;
        field(value, "definition")
    }

    pub async fn list_definitions(
        &self,
        request: WorkflowListDefinitionsReq,
    ) -> Result<Vec<WorkflowDefinitionSummary>> {
        let value = self.call_ok("list_definitions", request).await?;
        field(value, "definitions")
    }

    pub async fn dry_run(&self, definition: WorkflowDefinition) -> Result<WorkflowDryRunResult> {
        let value = self
            .call_ok("dry_run", json!({ "definition": definition }))
            .await?;
        parse_response(value)
    }

    pub async fn list_functions(&self, prefix: Option<&str>) -> Result<Vec<WorkflowFunctionInfo>> {
        let value = self
            .call_ok("list_functions", json!({ "prefix": prefix }))
            .await?;
        field(value, "functions")
    }

    pub async fn get_function(&self, fun_id: &str) -> Result<WorkflowFunctionInfo> {
        let value = self
            .call_ok("get_function", json!({ "fun_id": fun_id }))
            .await?;
        field(value, "function")
    }

    pub async fn get_function_by_name(
        &self,
        name: &str,
        version: &str,
    ) -> Result<WorkflowFunctionInfo> {
        let value = self
            .call_ok("get_function", json!({ "name": name, "version": version }))
            .await?;
        field(value, "function")
    }

    pub async fn get_capabilities(&self) -> Result<WorkflowCapabilities> {
        let value = self.call_ok("get_capabilities", json!({})).await?;
        parse_response(value)
    }

    // ----- Run -------------------------------------------------------------

    pub async fn create_run(&self, request: WorkflowCreateRunReq) -> Result<WorkflowCreateRunResult> {
        let value = self.call_ok("create_run", request).await?;
        parse_response(value)
    }

    pub async fn start_run(&self, request: WorkflowStartRunReq) -> Result<WorkflowRunTransition> {
        let value = self.call_ok("start_run", request).await?;
        parse_response(value)
    }

    pub async fn cancel_run(&self, request: WorkflowCancelRunReq) -> Result<WorkflowRunTransition> {
        let value = self.call_ok("cancel_run", request).await?;
        parse_response(value)
    }

    pub async fn get_run_graph(&self, run_id: &str) -> Result<WorkflowRunGraph> {
        let value = self
            .call_ok("get_run_graph", json!({ "run_id": run_id }))
            .await?;
        parse_response(value)
    }

    pub async fn get_run_output(&self, run_id: &str) -> Result<WorkflowRunOutput> {
        let value = self
            .call_ok("get_run_output", json!({ "run_id": run_id }))
            .await?;
        parse_response(value)
    }

    pub async fn list_runs(&self, request: WorkflowListRunsReq) -> Result<Vec<WorkflowRunSummary>> {
        let value = self.call_ok("list_runs", request).await?;
        field(value, "runs")
    }

    pub async fn get_history(&self, request: WorkflowHistoryReq) -> Result<WorkflowHistoryResult> {
        let value = self.call_ok("get_history", request).await?;
        parse_response(value)
    }

    async fn set_scheduled_task_state(
        &self,
        method: &str,
        schedule_id: &str,
    ) -> Result<WorkflowScheduledTask> {
        let value = self
            .call_ok(
                method,
                WorkflowScheduledTaskIdReq {
                    schedule_id: schedule_id.to_string(),
                },
            )
            .await?;
        value
            .get("schedule")
            .cloned()
            .ok_or_else(|| RPCErrors::ParserResponseError("missing `schedule`".to_string()))
            .and_then(parse_response)
    }

    async fn call_ok<T: Serialize>(&self, method: &str, request: T) -> Result<Value> {
        let request = serde_json::to_value(request).map_err(|error| {
            RPCErrors::ReasonError(format!("serialize workflow request failed: {}", error))
        })?;
        let value = self.krpc_client.call(method, request).await?;
        if value.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(value);
        }
        match WorkflowRpcError::from_response(&value) {
            Some(err) => Err(err.into_rpc_error()),
            None => Err(RPCErrors::ReasonError("workflow request failed".to_string())),
        }
    }
}

fn parse_response<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParserResponseError(format!("parse response failed: {}", error))
    })
}

fn field<T: for<'de> Deserialize<'de>>(value: Value, name: &str) -> Result<T> {
    value
        .get(name)
        .cloned()
        .ok_or_else(|| RPCErrors::ParserResponseError(format!("missing `{name}`")))
        .and_then(parse_response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn create_run_response_round_trips() {
        let value = json!({
            "ok": true,
            "run_id": "run-1",
            "workflow_id": "media.pdf",
            "status": "running",
            "revision": "ab12",
            "reused": true,
            "output": {},
            "events": [],
            "seq": 3
        });
        let result: WorkflowCreateRunResult = parse_response(value).unwrap();
        assert_eq!(result.status, RunStatus::Running);
        assert!(result.reused);
        assert_eq!(result.workflow_id, "media.pdf");
        let req = WorkflowCreateRunReq::new("media.pdf", json!({"obj_id": "cyfile:ab"})).ensure(2000);
        let wire = serde_json::to_value(&req).unwrap();
        assert_eq!(wire, json!({"workflow_id": "media.pdf", "input": {"obj_id": "cyfile:ab"}, "auto_start": true, "ensure": true, "wait_ms": 2000}));
    }

    #[test]
    fn run_output_and_graph_carry_structured_errors() {
        let value = json!({
            "ok": true,
            "run_id": "run-2",
            "status": "failed",
            "terminal": true,
            "revision": "r",
            "output": {},
            "node_errors": {"render_pdf_pages": {"code": "tools_missing", "message": "pdftoppm missing", "details": {"tools_missing": ["pdftoppm"]}, "retryable": false}},
            "run_error": null,
            "delivery": {"artifacts": []}
        });
        let output: WorkflowRunOutput = parse_response(value).unwrap();
        assert_eq!(output.status, RunStatus::Failed);
        assert_eq!(output.first_error().unwrap().code, "tools_missing");
        assert_eq!(output.first_error().unwrap().details["tools_missing"][0], "pdftoppm");

        let graph = json!({
            "ok": true,
            "run_id": "run-2",
            "workflow_id": "media.pdf",
            "workflow_name": "media.pdf",
            "status": "completed",
            "revision": "r",
            "requested_by": "u/app",
            "executor": "workflow",
            "owner": {"user_id": "u", "app_id": "app"},
            "graph": {},
            "nodes": {},
            "node_states": {"probe": "completed"},
            "node_outputs": {"probe": {"kind": "pdf", "size": 1}},
            "node_errors": {},
            "node_sources": {"probe": "cache"},
            "node_attempts": {"probe": 1},
            "human_waiting_nodes": [],
            "pending_thunks": {},
            "metrics": {},
            "output": {"build_catalog": {"manifest_obj_id": "mmanifest:aa"}},
            "deadline_at": null,
            "execution_epoch": 2,
            "created_at": 1,
            "updated_at": 2,
            "seq": 9
        });
        let graph: WorkflowRunGraph = parse_response(graph).unwrap();
        assert!(graph.is_terminal());
        assert_eq!(graph.node_states["probe"], NodeRunState::Completed);
        assert_eq!(graph.node_sources["probe"], "cache");
        assert_eq!(graph.output["build_catalog"]["manifest_obj_id"], "mmanifest:aa");
    }

    #[test]
    fn error_payloads_keep_their_code() {
        let err = WorkflowRpcError::from_response(&json!({
            "ok": false,
            "error": "run_not_found",
            "id": "run-9"
        }))
        .unwrap();
        assert!(err.is_not_found());
        let rpc = err.clone().into_rpc_error();
        let back = WorkflowRpcError::from_rpc_error(&rpc).unwrap();
        assert_eq!(back.code, "run_not_found");
        assert!(WorkflowRpcError::from_response(&json!({"ok": true})).is_none());
    }

    #[test]
    fn definition_summaries_and_functions_parse() {
        let summary: WorkflowDefinitionSummary = parse_response(json!({
            "id": "media.pdf", "name": "media.pdf", "owner": {"user_id": "system", "app_id": "workflow"},
            "source": "system", "read_only": true, "revision": "r", "status": "active", "version": 1,
            "tags": ["system"], "created_at": 1, "updated_at": 1, "warnings": 0, "errors": 0
        }))
        .unwrap();
        assert_eq!(summary.source, WorkflowDefinitionSource::System);
        assert!(summary.read_only);
        let list: Vec<WorkflowFunctionInfo> = field(
            json!({"ok": true, "functions": [{
                "fun_id": "func:00",
                "registration": {"name": "media.probe", "version": "1", "fun_id": "func:00", "timeout": 60},
                "object": {"func_type": "native", "content": "media.probe/impl-1", "is_pure": true,
                           "params_schema": {}, "result_type": {"Object": "media.probe"}, "result_schema": {}}
            }]}),
            "functions",
        )
        .unwrap();
        assert_eq!(list[0].registration.name, "media.probe");
        assert_eq!(list[0].object.func_type, crate::FunctionType::Native);
    }
}
