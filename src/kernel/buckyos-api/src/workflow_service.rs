//! Workflow Service 注册元数据。
//!
//! 与 task-manager / msg-center / aicc 等其他 kernel service 保持一致：
//! unique id / 端口 / [`AppDoc`] 生成集中放在 buckyos-api，
//! workflow service 自身和 scheduler 都从这里取，避免常量分裂。

use crate::{AppDoc, AppType, SelectorType};
use ::kRPC::*;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const WORKFLOW_SERVICE_UNIQUE_ID: &str = "workflow";
pub const WORKFLOW_SERVICE_NAME: &str = "workflow";
pub const WORKFLOW_SERVICE_PORT: u16 = 4070;
pub const WORKFLOW_SERVICE_HTTP_PATH: &str = "/kapi/workflow";

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
    #[serde(alias = "run_created")]
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
        let message = value
            .get("message")
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("workflow request failed");
        Err(RPCErrors::ReasonError(message.to_string()))
    }
}

fn parse_response<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParserResponseError(format!("parse response failed: {}", error))
    })
}
