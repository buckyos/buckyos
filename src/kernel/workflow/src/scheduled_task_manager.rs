use buckyos_api::{
    get_buckyos_api_runtime, parse_typed_task_data, CreateTaskExecutor, CreateTaskReq,
    GetSubtasksReq, ListTasksReq, ReportProgressReq, ReportRunningReq, ReportStartedReq,
    ReportWaitingReq, RunnerWriteEnvelope, Task, TaskManagerClient, TaskPhase, TaskWaitReason,
    TaskWaitReasonKind, TypedTaskData,
    WorkflowScheduleOwner, WorkflowSchedulePolicy, WorkflowScheduleTaskData,
    WorkflowScheduleTaskRequest, WorkflowScheduleTaskResult,
};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::state::Owner;

// 2.0：schedule 的业务生命周期是自己的枚举（wire 字符串与 1.x TaskStatus 保持
// 一致，desktop UI 无需再变）。Task DB 里 schedule root task 永远保持非终态，
// 生命周期状态的权威载体是 TaskData.request.status；task phase 只是投影。

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScheduleStatus {
    Running,
    Paused,
    Failed,
    Canceled,
}

impl ScheduleStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, ScheduleStatus::Canceled)
    }

    pub fn from_str_loose(value: &str) -> Option<Self> {
        match value {
            "Running" => Some(Self::Running),
            "Paused" => Some(Self::Paused),
            "Failed" => Some(Self::Failed),
            "Canceled" => Some(Self::Canceled),
            _ => None,
        }
    }
}

impl std::fmt::Display for ScheduleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// 映射约定：Running=启用 / Paused=暂停 / Canceled=归档 / Failed=错误。

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ScheduleSpec {
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

/// beta2.2: the `runner` dispatch field is gone. A schedule target names a
/// `task_type` in this service's own execution domain; there is no generic
/// "deliver to arbitrary runner" parameter anymore.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleSubtaskTemplate {
    pub task_type: String,
    pub name_template: String,
    #[serde(default)]
    pub data_template: Value,
}

pub type ScheduleTarget = ScheduleSubtaskTemplate;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    Skip,
    RunOnce,
    CatchUp,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulePolicy {
    pub misfire: MisfirePolicy,
    pub max_parallel_runs: u32,
    pub catch_up_limit: u32,
    pub jitter_sec: u32,
}

impl Default for SchedulePolicy {
    fn default() -> Self {
        Self {
            misfire: MisfirePolicy::RunOnce,
            max_parallel_runs: 1,
            catch_up_limit: 1,
            jitter_sec: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleTaskMirror {
    #[serde(default)]
    pub root_task_id: Option<String>,
    #[serde(default)]
    pub root_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleState {
    #[serde(default)]
    pub next_fire_at: Option<i64>,
    #[serde(default)]
    pub last_fire_at: Option<i64>,
    #[serde(default)]
    pub last_task_id: Option<String>,
    #[serde(default)]
    pub last_run_id: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowSchedule {
    pub schedule_id: String,
    pub owner: Owner,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub status: ScheduleStatus,
    pub schedule: ScheduleSpec,
    pub target: ScheduleTarget,
    pub state: ScheduleState,
    pub policy: SchedulePolicy,
    #[serde(default)]
    pub task_mirror: ScheduleTaskMirror,
    pub created_at: i64,
    pub updated_at: i64,
}

impl WorkflowSchedule {
    pub fn to_value(&self) -> Value {
        json!(self)
    }

    pub fn to_summary_value(&self) -> Value {
        json!({
            "schedule_id": self.schedule_id,
            "owner": self.owner,
            "name": self.name,
            "description": self.description,
            "status": self.status,
            "schedule": self.schedule,
            "target": self.target,
            "state": self.state,
            "policy": self.policy,
            "task_mirror": self.task_mirror,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FireStatus {
    Created,
    #[serde(alias = "run_created")]
    TaskCreated,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleFireRecord {
    pub fire_id: String,
    pub schedule_id: String,
    pub fire_key: String,
    pub fire_time: i64,
    pub manual: bool,
    pub status: FireStatus,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ScheduleFireRecord {
    pub fn to_value(&self) -> Value {
        json!(self)
    }
}

#[derive(Default)]
struct ScheduleInner {
    schedules: HashMap<String, WorkflowSchedule>,
    fires_by_id: HashMap<String, ScheduleFireRecord>,
    fire_key_index: HashMap<String, String>,
}

/// 进程内的 schedule 运行投影。**不是真相源** —— schedule 的唯一真相落在
/// Task Manager 的 Task DB（task_type=`workflow/schedule` 的 root task，其 TaskData
/// 承载完整定义；触发记录是其子 task）。这里只持有「热扫描路径」需要的内存视图：
/// schedule 定义缓存 + 派生的 `next_fire_at` + 进程内 fire 去重索引。重启时由
/// `hydrate` 从 Task DB 灌入，自身不落盘（空闲 tick 因此零 I/O，写只发生在
/// 真实状态跃迁，经 mirror 落到 Task DB）。
pub struct ScheduleStore {
    inner: RwLock<ScheduleInner>,
}

impl ScheduleStore {
    pub fn new_memory() -> Self {
        Self {
            inner: RwLock::new(ScheduleInner::default()),
        }
    }

    /// 启动时把从 Task DB 读回的 schedule 灌进内存投影。reboot 类 schedule 在
    /// 重启后立即重新 arm（对齐旧 snapshot 恢复语义）。不落盘。
    ///
    /// **insert-if-absent**：已在内存中的 schedule_id 不覆盖 —— 进程内的运行态
    /// （可能含未回写 Task DB 的临时 next_fire_at）比 Task DB 快照更新鲜。配合
    /// scan 循环的「失败重试」兜底，避免迟到的 hydrate clobber 掉新建/在跑的 schedule。
    pub async fn hydrate(&self, schedules: Vec<WorkflowSchedule>) {
        let now = Utc::now().timestamp();
        let mut guard = self.inner.write().await;
        for mut schedule in schedules {
            if guard.schedules.contains_key(&schedule.schedule_id) {
                continue;
            }
            if schedule.status == ScheduleStatus::Running && is_reboot_schedule(&schedule.schedule) {
                schedule.state.next_fire_at = Some(now);
            }
            guard
                .schedules
                .insert(schedule.schedule_id.clone(), schedule);
        }
    }

    pub async fn insert(&self, schedule: WorkflowSchedule) -> WorkflowSchedule {
        let mut guard = self.inner.write().await;
        guard
            .schedules
            .insert(schedule.schedule_id.clone(), schedule.clone());
        schedule
    }

    pub async fn get(&self, schedule_id: &str) -> Option<WorkflowSchedule> {
        self.inner.read().await.schedules.get(schedule_id).cloned()
    }

    pub async fn list(
        &self,
        owner: Option<&Owner>,
        status: Option<ScheduleStatus>,
        workflow_id: Option<&str>,
        name: Option<&str>,
    ) -> Vec<WorkflowSchedule> {
        let mut out: Vec<_> = self
            .inner
            .read()
            .await
            .schedules
            .values()
            .filter(|schedule| owner.map(|o| schedule.owner == *o).unwrap_or(true))
            .filter(|schedule| status.map(|s| schedule.status == s).unwrap_or(true))
            .filter(|schedule| name.map(|n| schedule.name.contains(n)).unwrap_or(true))
            .filter(|schedule| {
                workflow_id
                    .map(|want| schedule_workflow_id(&schedule.target) == Some(want))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    pub async fn update<F>(&self, schedule_id: &str, f: F) -> Option<WorkflowSchedule>
    where
        F: FnOnce(&mut WorkflowSchedule),
    {
        let mut guard = self.inner.write().await;
        let updated = {
            let schedule = guard.schedules.get_mut(schedule_id)?;
            f(schedule);
            schedule.updated_at = Utc::now().timestamp();
            schedule.clone()
        };
        Some(updated)
    }

    pub async fn due(&self, now: i64) -> Vec<WorkflowSchedule> {
        self.inner
            .read()
            .await
            .schedules
            .values()
            .filter(|schedule| schedule.status == ScheduleStatus::Running)
            .filter(|schedule| {
                schedule
                    .state
                    .next_fire_at
                    .map(|ts| ts <= now)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub async fn begin_fire(
        &self,
        schedule_id: &str,
        fire_time: i64,
        manual: bool,
    ) -> (ScheduleFireRecord, bool) {
        let fire_key = fire_key(schedule_id, fire_time);
        let now = Utc::now().timestamp();
        let mut guard = self.inner.write().await;
        if let Some(existing_id) = guard.fire_key_index.get(&fire_key).cloned() {
            if let Some(existing) = guard.fires_by_id.get(&existing_id).cloned() {
                return (existing, false);
            }
        }

        let fire = ScheduleFireRecord {
            fire_id: format!("fire-{}", Uuid::new_v4()),
            schedule_id: schedule_id.to_string(),
            fire_key: fire_key.clone(),
            fire_time,
            manual,
            status: FireStatus::Created,
            task_id: None,
            run_id: None,
            error: None,
            created_at: now,
            updated_at: now,
        };
        guard.fire_key_index.insert(fire_key, fire.fire_id.clone());
        guard.fires_by_id.insert(fire.fire_id.clone(), fire.clone());
        (fire, true)
    }

    pub async fn complete_fire(
        &self,
        fire_id: &str,
        status: FireStatus,
        task_id: Option<String>,
        run_id: Option<String>,
        error: Option<String>,
    ) -> Option<ScheduleFireRecord> {
        let mut guard = self.inner.write().await;
        let updated = {
            let fire = guard.fires_by_id.get_mut(fire_id)?;
            fire.status = status;
            fire.task_id = task_id;
            fire.run_id = run_id;
            fire.error = error;
            fire.updated_at = Utc::now().timestamp();
            fire.clone()
        };
        Some(updated)
    }

    pub async fn history(&self, schedule_id: &str, limit: usize) -> Vec<ScheduleFireRecord> {
        let mut out: Vec<_> = self
            .inner
            .read()
            .await
            .fires_by_id
            .values()
            .filter(|fire| fire.schedule_id == schedule_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| b.fire_time.cmp(&a.fire_time));
        out.truncate(limit);
        out
    }
}

pub struct ScheduleTaskMirrorClient {
    client: Option<Arc<TaskManagerClient>>,
    user_id: String,
    app_id: String,
}

impl ScheduleTaskMirrorClient {
    pub fn new(
        client: Arc<TaskManagerClient>,
        user_id: impl Into<String>,
        app_id: impl Into<String>,
    ) -> Self {
        Self {
            client: Some(client),
            user_id: user_id.into(),
            app_id: app_id.into(),
        }
    }

    pub fn from_runtime(user_id: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            client: None,
            user_id: user_id.into(),
            app_id: app_id.into(),
        }
    }

    async fn client(&self) -> Result<Arc<TaskManagerClient>, String> {
        if let Some(client) = self.client.as_ref() {
            return Ok(client.clone());
        }
        let runtime = get_buckyos_api_runtime().map_err(|err| err.to_string())?;
        Box::pin(runtime.get_task_mgr_client())
            .await
            .map(Arc::new)
            .map_err(|err| err.to_string())
    }

    pub async fn ensure_root_task(
        &self,
        schedule: &WorkflowSchedule,
    ) -> Result<ScheduleTaskMirror, String> {
        if schedule.task_mirror.root_task_id.is_some() {
            self.update_root_task(schedule).await?;
            return Ok(schedule.task_mirror.clone());
        }

        // The schedule id doubles as the idempotency key, so create is also
        // the find path after a restart (no separate scan needed).
        let task_name = schedule_root_task_name(schedule);
        let client = self.client().await?;
        let task = client
            .create_task(CreateTaskReq {
                name: task_name,
                schema_id: WORKFLOW_SCHEDULE_SCHEMA_ID.to_string(),
                schema_version: None,
                input: schedule_task_data(schedule),
                executor: CreateTaskExecutor::SelfApp {
                    app_instance_id: None,
                },
                parent_id: None,
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                idempotency_key: format!("wf-schedule-{}", schedule.schedule_id),
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await
            .map_err(|err| err.to_string())?;
        let task_id = task.task_id.clone();
        self.update_root_task_by_id(task_id.clone(), schedule).await?;
        Ok(ScheduleTaskMirror {
            root_task_id: Some(task_id.clone()),
            root_id: Some(task_id),
        })
    }

    pub async fn update_root_task(&self, schedule: &WorkflowSchedule) -> Result<(), String> {
        let Some(task_id) = schedule.task_mirror.root_task_id.clone() else {
            return Ok(());
        };
        self.update_root_task_by_id(task_id, schedule).await
    }

    /// Mirror the schedule lifecycle onto its root task: the task stays
    /// non-terminal (Running / Waiting projections); the authoritative
    /// status rides in the progress payload.
    async fn update_root_task_by_id(
        &self,
        task_id: String,
        schedule: &WorkflowSchedule,
    ) -> Result<(), String> {
        let client = self.client().await?;
        let task = client.get_task(&task_id).await.map_err(|err| err.to_string())?;
        if task.phase.is_terminal() {
            return Ok(());
        }
        let envelope = |task: &Task| RunnerWriteEnvelope {
            task_id: task.task_id.clone(),
            app_instance_id: None,
            runner_epoch: task.runner_epoch,
            expected_revision: task.revision,
        };
        let task = client
            .report_progress(ReportProgressReq {
                envelope: envelope(&task),
                progress: Some(schedule_task_data(schedule)),
                message: Some(schedule_message(schedule)),
            })
            .await
            .map_err(|err| err.to_string())?;
        match schedule.status {
            ScheduleStatus::Running => {
                let result = match task.phase {
                    TaskPhase::Accepted => client
                        .report_started(ReportStartedReq {
                            envelope: envelope(&task),
                        })
                        .await
                        .map(|_| ()),
                    TaskPhase::Waiting => client
                        .report_running(ReportRunningReq {
                            envelope: envelope(&task),
                        })
                        .await
                        .map(|_| ()),
                    _ => Ok(()),
                };
                result.map_err(|err| err.to_string())
            }
            ScheduleStatus::Paused | ScheduleStatus::Failed | ScheduleStatus::Canceled => {
                if task.phase == TaskPhase::Waiting {
                    return Ok(());
                }
                let task = match task.phase {
                    TaskPhase::Accepted => client
                        .report_started(ReportStartedReq {
                            envelope: envelope(&task),
                        })
                        .await
                        .map_err(|err| err.to_string())?,
                    _ => task,
                };
                let (kind, code) = match schedule.status {
                    ScheduleStatus::Paused => (TaskWaitReasonKind::Other, "schedule_paused"),
                    ScheduleStatus::Failed => (TaskWaitReasonKind::Other, "schedule_error"),
                    _ => (TaskWaitReasonKind::Other, "schedule_archived"),
                };
                client
                    .report_waiting(ReportWaitingReq {
                        envelope: envelope(&task),
                        reason: TaskWaitReason::with_code(kind, code),
                    })
                    .await
                    .map(|_| ())
                    .map_err(|err| err.to_string())
            }
        }
    }

    pub async fn create_fire_subtask(
        &self,
        schedule: &WorkflowSchedule,
        rendered: &RenderedScheduleSubtask,
    ) -> Result<String, String> {
        let Some(parent_id) = schedule.task_mirror.root_task_id.clone() else {
            return Err("schedule root task is missing".to_string());
        };
        // 2.0: the fire subtask's creator is the workflow service identity
        // from the session token; the business owner stays recorded in the
        // schedule payload and gains visibility through the task tree.
        let client = self.client().await?;
        let task = client
            .create_task(CreateTaskReq {
                name: rendered.name.clone(),
                schema_id: fire_subtask_schema_id(rendered.task_type.as_str()),
                schema_version: None,
                input: rendered.data.clone(),
                executor: CreateTaskExecutor::SelfApp {
                    app_instance_id: None,
                },
                parent_id: Some(parent_id),
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                idempotency_key: format!("wf-fire-{}", uuid::Uuid::new_v4().simple()),
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await
            .map_err(|err| err.to_string())?;
        Ok(task.task_id)
    }

    pub async fn active_fire_subtasks(&self, schedule: &WorkflowSchedule) -> Result<u32, String> {
        let Some(parent_id) = schedule.task_mirror.root_task_id.clone() else {
            return Ok(0);
        };
        let client = self.client().await?;
        let page = client
            .get_subtasks(GetSubtasksReq {
                task_id: parent_id,
                cursor: None,
                limit: None,
            })
            .await
            .map_err(|err| err.to_string())?;
        Ok(page
            .tasks
            .iter()
            .filter(|task| !task.phase.is_terminal())
            .count() as u32)
    }

    pub async fn find_fire_subtask_by_run_id(
        &self,
        schedule: &WorkflowSchedule,
        run_id: &str,
    ) -> Result<Option<String>, String> {
        let Some(parent_id) = schedule.task_mirror.root_task_id.clone() else {
            return Ok(None);
        };
        let client = self.client().await?;
        let page = client
            .get_subtasks(GetSubtasksReq {
                task_id: parent_id,
                cursor: None,
                limit: None,
            })
            .await
            .map_err(|err| err.to_string())?;
        for summary in page.tasks {
            let Ok(task) = client.get_task(&summary.task_id).await else {
                continue;
            };
            let payload = schedule_task_payload(&task);
            if matches!(
                parse_typed_task_data("workflow/run", payload),
                Ok(TypedTaskData::WorkflowRun(data)) if data.request.run_id == run_id
            ) {
                return Ok(Some(task.task_id));
            }
        }
        Ok(None)
    }

    /// 启动时从 Task DB 把全部 `workflow/schedule` root task 读回并解析成
    /// WorkflowSchedule。这是「Task DB 是唯一真相源、内存只是投影」的入口：
    /// 解析失败的脏行被跳过（不让一条坏数据挡住整批 hydrate）。
    pub async fn load_schedules(&self) -> Result<Vec<WorkflowSchedule>, String> {
        let client = self.client().await?;
        let page = client
            .list_tasks(ListTasksReq {
                schema_id: Some(WORKFLOW_SCHEDULE_SCHEMA_ID.to_string()),
                include_archived: false,
                ..Default::default()
            })
            .await
            .map_err(|err| err.to_string())?;
        let mut schedules = Vec::new();
        for summary in page.tasks {
            let Ok(task) = client.get_task(&summary.task_id).await else {
                continue;
            };
            if let Some(schedule) = schedule_from_task(&task) {
                schedules.push(schedule);
            }
        }
        Ok(schedules)
    }
}

/// Versioned schema id of the schedule root task.
pub const WORKFLOW_SCHEDULE_SCHEMA_ID: &str = "workflow.schedule/v1";

/// Fire subtasks reuse the versioned schema derived from the rendered legacy
/// task type (`workflow/run` -> `workflow.run_tree/v1`, others mechanical).
fn fire_subtask_schema_id(task_type: &str) -> String {
    match task_type {
        "workflow/run" => crate::task_tracker::WORKFLOW_RUN_SCHEMA_ID.to_string(),
        other => format!("{}/v1", other.replace('/', ".")),
    }
}

/// Payload view of a schedule-tree task: result wins, then progress, then
/// the immutable input.
fn schedule_task_payload(task: &Task) -> Value {
    task.result
        .clone()
        .or_else(|| task.progress.clone())
        .unwrap_or_else(|| task.input.clone())
}

/// Task（workflow/schedule root task）→ WorkflowSchedule 的无损反序列化。
/// schedule_id / created_at / task_mirror 直接取自 Task 列；owner/policy/
/// description 取自 TaskData（Task 列里没有）；schedule/target/state 取自 TaskData。
fn schedule_from_task(task: &Task) -> Option<WorkflowSchedule> {
    let data = match parse_typed_task_data("workflow/schedule", schedule_task_payload(task)) {
        Ok(TypedTaskData::WorkflowSchedule(data)) => data,
        _ => return None,
    };
    let req = data.request;
    let schedule: ScheduleSpec = req
        .schedule
        .and_then(|value| serde_json::from_value(value).ok())?;
    let target: ScheduleTarget = req
        .target
        .and_then(|value| serde_json::from_value(value).ok())?;
    // 2.0：status 的权威来源是 TaskData.request.status；task phase 只是投影。
    let status = req
        .status
        .as_deref()
        .and_then(ScheduleStatus::from_str_loose)
        .unwrap_or(ScheduleStatus::Running);
    let owner = req
        .owner
        .map(|o| Owner {
            user_id: o.user_id,
            app_id: o.app_id,
        })
        .unwrap_or_else(|| Owner {
            user_id: task.creator.user_id.clone(),
            app_id: task.creator.app_id.clone(),
        });
    let policy = req.policy.map(policy_from_typed).unwrap_or_default();
    let result = data.result.unwrap_or_default();
    let state = ScheduleState {
        next_fire_at: result.next_fire_at,
        last_fire_at: result.last_fire_at,
        last_task_id: result.last_task_id,
        last_run_id: result.last_run_id,
        consecutive_failures: result.consecutive_failures as u32,
        last_error: result.last_error.map(|value| match value {
            Value::String(text) => text,
            other => other.to_string(),
        }),
    };
    let schedule_id = if req.schedule_id.trim().is_empty() {
        task.task_id.clone()
    } else {
        req.schedule_id
    };
    Some(WorkflowSchedule {
        schedule_id,
        owner,
        name: req.name.unwrap_or_else(|| task.name.clone()),
        description: req.description,
        status,
        schedule,
        target,
        state,
        policy,
        task_mirror: ScheduleTaskMirror {
            root_task_id: Some(task.task_id.clone()),
            root_id: Some(task.root_id.clone()),
        },
        created_at: task.created_at as i64,
        updated_at: task.updated_at as i64,
    })
}

fn policy_from_typed(value: WorkflowSchedulePolicy) -> SchedulePolicy {
    let mut policy = SchedulePolicy::default();
    if let Some(misfire) = value.misfire.as_deref() {
        policy.misfire = match misfire {
            "skip" => MisfirePolicy::Skip,
            "run_once" => MisfirePolicy::RunOnce,
            "catch_up" => MisfirePolicy::CatchUp,
            "manual" => MisfirePolicy::Manual,
            _ => policy.misfire,
        };
    }
    if let Some(value) = value.max_parallel_runs {
        policy.max_parallel_runs = value;
    }
    if let Some(value) = value.catch_up_limit {
        policy.catch_up_limit = value.max(1);
    }
    if let Some(value) = value.jitter_sec {
        policy.jitter_sec = value;
    }
    policy
}

fn policy_to_typed(policy: &SchedulePolicy) -> WorkflowSchedulePolicy {
    WorkflowSchedulePolicy {
        misfire: Some(misfire_str(policy.misfire).to_string()),
        max_parallel_runs: Some(policy.max_parallel_runs),
        catch_up_limit: Some(policy.catch_up_limit),
        jitter_sec: Some(policy.jitter_sec),
    }
}

fn misfire_str(misfire: MisfirePolicy) -> &'static str {
    match misfire {
        MisfirePolicy::Skip => "skip",
        MisfirePolicy::RunOnce => "run_once",
        MisfirePolicy::CatchUp => "catch_up",
        MisfirePolicy::Manual => "manual",
    }
}

fn schedule_root_task_name(schedule: &WorkflowSchedule) -> String {
    format!(
        "workflow/schedule/{} [{}]",
        schedule.name, schedule.schedule_id
    )
}

fn schedule_task_data(schedule: &WorkflowSchedule) -> Value {
    serde_json::to_value(WorkflowScheduleTaskData {
        request: WorkflowScheduleTaskRequest {
            schedule_id: schedule.schedule_id.clone(),
            name: Some(schedule.name.clone()),
            status: Some(schedule.status.to_string()),
            schedule: serde_json::to_value(&schedule.schedule).ok(),
            target: serde_json::to_value(&schedule.target).ok(),
            description: schedule.description.clone(),
            owner: Some(WorkflowScheduleOwner {
                user_id: schedule.owner.user_id.clone(),
                app_id: schedule.owner.app_id.clone(),
            }),
            policy: Some(policy_to_typed(&schedule.policy)),
        },
        progress: None,
        result: Some(WorkflowScheduleTaskResult {
            next_fire_at: schedule.state.next_fire_at,
            last_fire_at: schedule.state.last_fire_at,
            last_task_id: schedule.state.last_task_id.clone(),
            last_run_id: schedule.state.last_run_id.clone(),
            consecutive_failures: schedule.state.consecutive_failures as u64,
            last_error: schedule.state.last_error.clone().map(Value::String),
        }),
    })
    .unwrap_or_else(|_| Value::Object(Default::default()))
}

fn schedule_message(schedule: &WorkflowSchedule) -> String {
    match schedule.status {
        ScheduleStatus::Running => schedule
            .state
            .next_fire_at
            .map(|ts| format!("next fire at {}", rfc3339(ts)))
            .unwrap_or_else(|| "enabled".to_string()),
        ScheduleStatus::Paused => "paused".to_string(),
        ScheduleStatus::Canceled => "archived".to_string(),
        ScheduleStatus::Failed => schedule
            .state
            .last_error
            .clone()
            .unwrap_or_else(|| "schedule error".to_string()),
    }
}

pub fn fire_key(schedule_id: &str, fire_time: i64) -> String {
    format!("{}:{}", schedule_id, fire_time)
}

pub fn is_reboot_schedule(spec: &ScheduleSpec) -> bool {
    matches!(spec, ScheduleSpec::Cron { expr, .. } if expr == "@reboot")
}

pub fn rfc3339(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}

pub fn schedule_spec_from_value(value: &Value) -> Result<ScheduleSpec, String> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing schedule.kind".to_string())?;
    match kind {
        "cron" => {
            let expr = value
                .get("expr")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing schedule.expr".to_string())?;
            let timezone = value
                .get("timezone")
                .and_then(Value::as_str)
                .unwrap_or("UTC")
                .to_string();
            let expr = normalize_cron_expr(expr)?;
            validate_timezone(&timezone)?;
            parse_cron(&expr)?;
            Ok(ScheduleSpec::Cron {
                expr,
                timezone,
                calendar: value
                    .get("calendar")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                start_at: value.get("start_at").and_then(Value::as_i64),
                end_at: value.get("end_at").and_then(Value::as_i64),
            })
        }
        "once" => {
            let run_at = value
                .get("run_at")
                .and_then(Value::as_i64)
                .ok_or_else(|| "missing schedule.run_at".to_string())?;
            Ok(ScheduleSpec::Once {
                run_at,
                timezone: value
                    .get("timezone")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        }
        "run_every" => {
            let every_sec = value
                .get("every_sec")
                .and_then(Value::as_u64)
                .ok_or_else(|| "missing schedule.every_sec".to_string())?;
            if every_sec == 0 {
                return Err("schedule.every_sec must be greater than zero".to_string());
            }
            if let Some(timezone) = value.get("timezone").and_then(Value::as_str) {
                validate_timezone(timezone)?;
            }
            Ok(ScheduleSpec::RunEvery {
                every_sec,
                start_at: value.get("start_at").and_then(Value::as_i64),
                end_at: value.get("end_at").and_then(Value::as_i64),
                timezone: value
                    .get("timezone")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        }
        other => Err(format!("unsupported schedule.kind `{}`", other)),
    }
}

pub fn schedule_target_from_value(value: &Value) -> Result<ScheduleTarget, String> {
    if value.get("task_type").is_some() {
        return schedule_subtask_template_from_value(value);
    }
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing target.kind".to_string())?;
    match kind {
        "remind" => {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target.text".to_string())?
                .to_string();
            let to = value
                .get("to")
                .and_then(Value::as_str)
                .unwrap_or("self")
                .to_string();
            Ok(ScheduleSubtaskTemplate {
                task_type: "workflow.send_message".to_string(),
                name_template: "remind: ${schedule.name} [${fire.fire_id}]".to_string(),
                data_template: json!({
                    "send_message": {
                        "to": to,
                        "text": text,
                        "trigger": trigger_template()
                    }
                }),
            })
        }
        "agent_task" | "task" => {
            let title = value
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target.title".to_string())?
                .to_string();
            let objective = value
                .get("objective")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target.objective".to_string())?
                .to_string();
            let workspace_id = value
                .get("workspace_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target.workspace_id".to_string())?
                .to_string();
            // The executing agent lives in the task payload
            // (`execution.runner`, OpenDAN's business schema) — it is not a
            // TaskMgr dispatch parameter anymore.
            let agent = value
                .get("agent")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| "${schedule.owner.app_id}".to_string());
            Ok(ScheduleSubtaskTemplate {
                task_type: "agent.delegate".to_string(),
                name_template: title.clone(),
                data_template: json!({
                    "agent_delegate": {
                        "version": 1,
                        "purpose": objective,
                        "title": title,
                        "requester_agent_id": "${schedule.owner.app_id}",
                        "owner_session_id": "schedule-${schedule.schedule_id}",
                        "input": {
                            "text": objective
                        },
                        "workspace_hints": [{
                            "workspace_id": workspace_id
                        }],
                        "trigger": trigger_template(),
                        "execution": {
                            "workspace_id": workspace_id,
                            "behavior": value.get("behavior").cloned().unwrap_or(Value::Null),
                            "runner": agent,
                            "status": "pending"
                        }
                    }
                }),
            })
        }
        "workflow.run" => {
            let workflow_id = value
                .get("workflow_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target.workflow_id".to_string())?
                .to_string();
            Ok(ScheduleSubtaskTemplate {
                task_type: "workflow.run".to_string(),
                name_template: "workflow/run: ${schedule.name} [${fire.fire_id}]".to_string(),
                data_template: json!({
                    "workflow_run": {
                        "workflow_id": workflow_id,
                        "input": value.get("input").cloned().unwrap_or(Value::Null),
                        "trigger": trigger_template()
                    }
                }),
            })
        }
        "opendan.command" => {
            let command = value
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target.command".to_string())?
                .to_string();
            Ok(ScheduleSubtaskTemplate {
                task_type: "opendan.command".to_string(),
                name_template: "opendan.command: ${schedule.name} [${fire.fire_id}]".to_string(),
                data_template: json!({
                    "opendan_command": {
                        "command": command,
                        "args": value.get("args").cloned().unwrap_or(Value::Null),
                        "trigger": trigger_template()
                    }
                }),
            })
        }
        "service.rpc" => {
            let service = value
                .get("service")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target.service".to_string())?
                .to_string();
            let method = value
                .get("method")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target.method".to_string())?
                .to_string();
            Ok(ScheduleSubtaskTemplate {
                task_type: "service.rpc".to_string(),
                name_template: "service.rpc: ${schedule.name} [${fire.fire_id}]".to_string(),
                data_template: json!({
                    "service_rpc": {
                        "service": service,
                        "method": method,
                        "params": value.get("params").cloned().unwrap_or(Value::Null),
                        "trigger": trigger_template()
                    }
                }),
            })
        }
        other => Err(format!("unsupported target.kind `{}`", other)),
    }
}

pub fn schedule_subtask_template_from_value(
    value: &Value,
) -> Result<ScheduleSubtaskTemplate, String> {
    let task_type = value
        .get("task_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing target.task_type".to_string())?
        .to_string();
    let name_template = value
        .get("name_template")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing target.name_template".to_string())?
        .to_string();
    Ok(ScheduleSubtaskTemplate {
        task_type,
        name_template,
        data_template: value.get("data_template").cloned().unwrap_or(Value::Null),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderedScheduleSubtask {
    pub task_type: String,
    pub name: String,
    pub data: Value,
}

pub fn render_subtask_template(
    schedule: &WorkflowSchedule,
    fire: &ScheduleFireRecord,
) -> RenderedScheduleSubtask {
    let context = render_context(schedule, fire);
    RenderedScheduleSubtask {
        task_type: render_string(&schedule.target.task_type, &context),
        name: render_string(&schedule.target.name_template, &context),
        data: render_value(&schedule.target.data_template, &context),
    }
}

pub fn schedule_workflow_id(target: &ScheduleSubtaskTemplate) -> Option<&str> {
    if target.task_type != "workflow.run" {
        return None;
    }
    target
        .data_template
        .pointer("/workflow_run/workflow_id")
        .and_then(Value::as_str)
}

pub fn validate_subtask_template(target: &ScheduleSubtaskTemplate) -> Result<(), String> {
    if target.task_type.trim().is_empty() {
        return Err("target.task_type must not be empty".to_string());
    }
    if target.name_template.trim().is_empty() {
        return Err("target.name_template must not be empty".to_string());
    }
    match target.task_type.as_str() {
        "agent.delegate" => validate_agent_delegate_template(target),
        "workflow.send_message" => validate_send_message_template(target),
        "workflow.run" => schedule_workflow_id(target)
            .filter(|value| !value.trim().is_empty())
            .map(|_| ())
            .ok_or_else(|| {
                "workflow.run target requires data_template.workflow_run.workflow_id".to_string()
            }),
        _ => Ok(()),
    }
}

fn validate_agent_delegate_template(target: &ScheduleSubtaskTemplate) -> Result<(), String> {
    let delegate = target
        .data_template
        .get("agent_delegate")
        .ok_or_else(|| "agent.delegate target requires data_template.agent_delegate".to_string())?;
    for field in ["title", "purpose"] {
        if delegate
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return Err(format!("agent.delegate target requires `{}`", field));
        }
    }
    let workspace_count = delegate
        .get("workspace_hints")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("workspace_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_some()
                })
                .count()
        })
        .unwrap_or(0);
    if workspace_count != 1 {
        return Err("agent.delegate target requires exactly one workspace_id".to_string());
    }
    Ok(())
}

fn validate_send_message_template(target: &ScheduleSubtaskTemplate) -> Result<(), String> {
    let send_message = target.data_template.get("send_message").ok_or_else(|| {
        "workflow.send_message target requires data_template.send_message".to_string()
    })?;
    if send_message
        .get("to")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err("workflow.send_message target requires recipient `to`".to_string());
    }
    if send_message
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err("workflow.send_message target requires non-empty text".to_string());
    }
    Ok(())
}

fn trigger_template() -> Value {
    json!({
        "schedule_id": "${schedule.schedule_id}",
        "fire_id": "${fire.fire_id}",
        "fire_time": "${fire.fire_time}",
        "manual": "${fire.manual}"
    })
}

fn render_context(schedule: &WorkflowSchedule, fire: &ScheduleFireRecord) -> Value {
    json!({
        "schedule": {
            "schedule_id": schedule.schedule_id,
            "name": schedule.name,
            "owner": {
                "user_id": schedule.owner.user_id,
                "app_id": schedule.owner.app_id
            }
        },
        "fire": {
            "fire_id": fire.fire_id,
            "fire_key": fire.fire_key,
            "fire_time": fire.fire_time,
            "manual": fire.manual
        }
    })
}

fn render_value(value: &Value, context: &Value) -> Value {
    match value {
        Value::String(raw) => placeholder_value(raw, context)
            .unwrap_or_else(|| Value::String(render_string(raw, context))),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_value(item, context))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), render_value(value, context)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn render_string(raw: &str, context: &Value) -> String {
    let mut out = raw.to_string();
    for key in [
        "schedule.schedule_id",
        "schedule.name",
        "schedule.owner.user_id",
        "schedule.owner.app_id",
        "fire.fire_id",
        "fire.fire_key",
        "fire.fire_time",
        "fire.manual",
    ] {
        let needle = format!("${{{}}}", key);
        if out.contains(&needle) {
            if let Some(value) = value_at_path(context, key) {
                out = out.replace(&needle, value_to_template_string(value).as_str());
            }
        }
    }
    out
}

fn placeholder_value(raw: &str, context: &Value) -> Option<Value> {
    let trimmed = raw.trim();
    let key = trimmed.strip_prefix("${")?.strip_suffix('}')?;
    value_at_path(context, key).cloned()
}

fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn value_to_template_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

pub fn schedule_policy_from_value(value: Option<&Value>) -> Result<SchedulePolicy, String> {
    let Some(value) = value else {
        return Ok(SchedulePolicy::default());
    };
    let mut policy = SchedulePolicy::default();
    if let Some(raw) = value.get("misfire").and_then(Value::as_str) {
        policy.misfire = match raw {
            "skip" => MisfirePolicy::Skip,
            "run_once" => MisfirePolicy::RunOnce,
            "catch_up" => MisfirePolicy::CatchUp,
            "manual" => MisfirePolicy::Manual,
            other => return Err(format!("unsupported policy.misfire `{}`", other)),
        };
    }
    policy.max_parallel_runs = 1;
    if let Some(value) = value.get("catch_up_limit").and_then(Value::as_u64) {
        policy.catch_up_limit = value.max(1) as u32;
    }
    if let Some(value) = value.get("jitter_sec").and_then(Value::as_u64) {
        policy.jitter_sec = value as u32;
    }
    Ok(policy)
}

pub fn next_fire_after(spec: &ScheduleSpec, after_ts: i64) -> Option<i64> {
    match spec {
        ScheduleSpec::Once { run_at, .. } => {
            if *run_at > after_ts {
                Some(*run_at)
            } else {
                None
            }
        }
        ScheduleSpec::RunEvery {
            every_sec,
            start_at,
            end_at,
            ..
        } => {
            let every_sec = *every_sec as i64;
            if every_sec <= 0 {
                return None;
            }
            let start_at = start_at.unwrap_or(after_ts.saturating_add(every_sec));
            let next = if start_at > after_ts {
                start_at
            } else {
                let elapsed = after_ts.saturating_sub(start_at);
                let steps = elapsed / every_sec + 1;
                start_at.saturating_add(steps.saturating_mul(every_sec))
            };
            if end_at.map(|end| next > end).unwrap_or(false) {
                None
            } else {
                Some(next)
            }
        }
        ScheduleSpec::Cron {
            expr,
            timezone,
            start_at,
            end_at,
            ..
        } => {
            if expr == "@reboot" {
                return Some(Utc::now().timestamp());
            }
            let cron = parse_cron(expr).ok()?;
            let start = start_at.unwrap_or(i64::MIN);
            let end = end_at.unwrap_or(i64::MAX);
            let mut ts = round_to_next_minute(after_ts).max(start);
            let max = ts.saturating_add(366 * 24 * 60 * 60);
            while ts <= max && ts <= end {
                let offset = timezone_offset_seconds(timezone, ts).ok()?;
                let local_ts = ts + offset as i64;
                if let Some(dt) = DateTime::<Utc>::from_timestamp(local_ts, 0) {
                    if cron.matches(dt) {
                        return Some(ts);
                    }
                }
                ts += 60;
            }
            None
        }
    }
}

pub fn due_fire_times(
    schedule: &WorkflowSchedule,
    now_ts: i64,
) -> (Vec<i64>, Option<i64>, Option<String>) {
    let Some(next_fire_at) = schedule.state.next_fire_at else {
        return (Vec::new(), None, None);
    };
    if next_fire_at > now_ts {
        return (Vec::new(), Some(next_fire_at), None);
    }
    match schedule.policy.misfire {
        MisfirePolicy::Skip => {
            let next = next_fire_after(&schedule.schedule, now_ts);
            (Vec::new(), next, None)
        }
        MisfirePolicy::Manual => {
            let next = next_fire_after(&schedule.schedule, now_ts);
            (Vec::new(), next, Some("schedule_missed_manual".to_string()))
        }
        MisfirePolicy::RunOnce => {
            let next = next_fire_after(&schedule.schedule, now_ts);
            (vec![next_fire_at], next, None)
        }
        MisfirePolicy::CatchUp => {
            let mut out = Vec::new();
            let mut cursor = next_fire_at;
            let limit = schedule.policy.catch_up_limit.max(1);
            while cursor <= now_ts && out.len() < limit as usize {
                out.push(cursor);
                let Some(next) = next_fire_after(&schedule.schedule, cursor) else {
                    break;
                };
                cursor = next;
            }
            let next = next_fire_after(&schedule.schedule, now_ts);
            (out, next, None)
        }
    }
}

pub fn next_fire_times(spec: &ScheduleSpec, after_ts: i64, count: usize) -> Vec<i64> {
    let mut out = Vec::new();
    let mut cursor = after_ts;
    for _ in 0..count {
        let Some(next) = next_fire_after(spec, cursor) else {
            break;
        };
        out.push(next);
        cursor = next;
    }
    out
}

fn round_to_next_minute(ts: i64) -> i64 {
    ts - ts.rem_euclid(60) + 60
}

fn normalize_cron_expr(expr: &str) -> Result<String, String> {
    let trimmed = expr.trim();
    let normalized = match trimmed {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        "@reboot" => "@reboot",
        other => other,
    };
    if normalized.contains('%') {
        return Err("cron % stdin syntax is not supported".to_string());
    }
    Ok(normalized.to_string())
}

#[derive(Debug, Clone)]
struct CronExpr {
    minute: BTreeSet<u32>,
    hour: BTreeSet<u32>,
    dom: BTreeSet<u32>,
    month: BTreeSet<u32>,
    dow: BTreeSet<u32>,
    dom_star: bool,
    dow_star: bool,
}

impl CronExpr {
    fn matches(&self, dt: DateTime<Utc>) -> bool {
        let minute = dt.minute();
        let hour = dt.hour();
        let dom = dt.day();
        let month = dt.month();
        let dow = dt.weekday().num_days_from_sunday();
        let day_match = match (self.dom_star, self.dow_star) {
            (true, true) => true,
            (true, false) => self.dow.contains(&dow),
            (false, true) => self.dom.contains(&dom),
            (false, false) => self.dom.contains(&dom) || self.dow.contains(&dow),
        };
        self.minute.contains(&minute)
            && self.hour.contains(&hour)
            && self.month.contains(&month)
            && day_match
    }
}

fn parse_cron(expr: &str) -> Result<CronExpr, String> {
    if expr == "@reboot" {
        return Ok(CronExpr {
            minute: BTreeSet::new(),
            hour: BTreeSet::new(),
            dom: BTreeSet::new(),
            month: BTreeSet::new(),
            dow: BTreeSet::new(),
            dom_star: true,
            dow_star: true,
        });
    }
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err("cron expression must have exactly 5 fields".to_string());
    }
    let (minute, _) = parse_field(parts[0], 0, 59, false)?;
    let (hour, _) = parse_field(parts[1], 0, 23, false)?;
    let (dom, dom_star) = parse_field(parts[2], 1, 31, false)?;
    let (month, _) = parse_field(parts[3], 1, 12, false)?;
    let (dow, dow_star) = parse_field(parts[4], 0, 7, true)?;
    Ok(CronExpr {
        minute,
        hour,
        dom,
        month,
        dow,
        dom_star,
        dow_star,
    })
}

fn parse_field(
    raw: &str,
    min: u32,
    max: u32,
    seven_is_zero: bool,
) -> Result<(BTreeSet<u32>, bool), String> {
    let star = raw == "*";
    let mut values = BTreeSet::new();
    for part in raw.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step = step
                    .parse::<u32>()
                    .map_err(|_| format!("invalid cron step `{}`", part))?;
                if step == 0 {
                    return Err("cron step must be greater than zero".to_string());
                }
                (range, step)
            }
            None => (part, 1),
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((start, end)) = range.split_once('-') {
            (
                parse_field_num(start, min, max, seven_is_zero)?,
                parse_field_num(end, min, max, seven_is_zero)?,
            )
        } else {
            let value = parse_field_num(range, min, max, seven_is_zero)?;
            (value, value)
        };
        if start > end {
            return Err(format!("invalid cron range `{}`", part));
        }
        let mut current = start;
        while current <= end {
            values.insert(if seven_is_zero && current == 7 {
                0
            } else {
                current
            });
            current = current.saturating_add(step);
            if current == 0 {
                break;
            }
        }
    }
    Ok((values, star))
}

fn parse_field_num(raw: &str, min: u32, max: u32, seven_is_zero: bool) -> Result<u32, String> {
    let value = raw
        .parse::<u32>()
        .map_err(|_| format!("invalid cron value `{}`", raw))?;
    let effective_max = if seven_is_zero { max } else { max };
    if value < min || value > effective_max {
        return Err(format!(
            "cron value `{}` out of range {}-{}",
            value, min, effective_max
        ));
    }
    Ok(value)
}

fn validate_timezone(timezone: &str) -> Result<(), String> {
    timezone_offset_seconds(timezone, Utc::now().timestamp()).map(|_| ())
}

fn timezone_offset_seconds(timezone: &str, utc_ts: i64) -> Result<i32, String> {
    match timezone {
        "UTC" | "Etc/UTC" | "Z" => Ok(0),
        "Asia/Shanghai" | "Asia/Chongqing" | "Asia/Hong_Kong" => Ok(8 * 3600),
        "America/Los_Angeles" | "US/Pacific" => Ok(if is_us_dst(utc_ts, -8) {
            -7 * 3600
        } else {
            -8 * 3600
        }),
        "America/New_York" | "US/Eastern" => Ok(if is_us_dst(utc_ts, -5) {
            -4 * 3600
        } else {
            -5 * 3600
        }),
        other => parse_fixed_offset(other)
            .ok_or_else(|| format!("unsupported timezone `{}` without chrono-tz", other)),
    }
}

fn parse_fixed_offset(raw: &str) -> Option<i32> {
    if raw.len() != 6 {
        return None;
    }
    let sign = match &raw[0..1] {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let hour = raw[1..3].parse::<i32>().ok()?;
    let minute = raw[4..6].parse::<i32>().ok()?;
    if &raw[3..4] != ":" || hour > 23 || minute > 59 {
        return None;
    }
    Some(sign * (hour * 3600 + minute * 60))
}

fn is_us_dst(utc_ts: i64, standard_offset_hours: i32) -> bool {
    let Some(utc) = DateTime::<Utc>::from_timestamp(utc_ts, 0) else {
        return false;
    };
    let year = utc.year();
    let start_local = nth_weekday_of_month(year, 3, chrono::Weekday::Sun, 2, 2);
    let end_local = nth_weekday_of_month(year, 11, chrono::Weekday::Sun, 1, 2);
    let start_utc = start_local - (standard_offset_hours as i64 * 3600);
    let end_utc = end_local - ((standard_offset_hours + 1) as i64 * 3600);
    utc_ts >= start_utc && utc_ts < end_utc
}

fn nth_weekday_of_month(
    year: i32,
    month: u32,
    weekday: chrono::Weekday,
    nth: u32,
    hour: u32,
) -> i64 {
    let mut count = 0;
    for day in 1..=31 {
        if let Some(dt) = Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).single() {
            if dt.weekday() == weekday {
                count += 1;
                if count == nth {
                    return dt.timestamp();
                }
            }
        }
    }
    0
}
