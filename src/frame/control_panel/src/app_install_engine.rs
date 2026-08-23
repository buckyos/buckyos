//! 安装事务引擎：可恢复的 Stage 状态机（doc/App 安装协议.md §3、§5.5）。
//!
//! 职责边界：
//! - 引擎只负责状态机推进、持久化纪律与恢复语义；每个 Stage 的实际工作
//!   由 `InstallStageDriver` 完成（生产实现分布在 resolver/planner/pikg 与
//!   Acquire/Deploy 适配层，单元测试用 fake 跑全 Stage）；
//! - 每个 Stage 成功后先把完整事务状态写回 Task.progress，再进入下一 Stage；
//! - 重启恢复只相信 TaskManager 持久快照；Stage handler 可重复调用；
//! - TaskManager 是任务状态唯一真相源；业务 RPC 直接启动执行体，启动扫描
//!   和低频 sweep 只负责恢复遗漏。

use async_trait::async_trait;
use buckyos_api::{
    AppInstallDisplayProgress, AppInstallProgressEnvelope, AppInstallStatusSnapshot,
    AppInstallTaskData, AppInstallTerminalOutput, InstallApproval, InstallError, InstallErrorCode,
    InstallInspection, InstallPlanStatus, InstallReadiness, InstallStage, InstallTaskResult,
    InstallTransactionState, InstallUserAction, PreparedDeployment, TaskDataProgress, TaskExecutor,
    TaskOutcome, TaskPhase, TaskWaitReason, TaskWaitReasonKind, APP_INSTALL_SCHEMA_VERSION,
    TASK_DATA_TYPE_APP_INSTALL, TASK_DATA_TYPE_APP_UPDATE,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::{info, warn};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// store 抽象（生产 = TaskManagerClient；测试 = 内存实现）
// ---------------------------------------------------------------------------

/// Engine-local task lifecycle vocabulary (the 1.x TaskStatus shape). The
/// production store maps it onto the 2.0 phase/outcome/wait-reason model;
/// the engine state machine keeps a single-value view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallTaskStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Canceled,
    WaitingForApproval,
}

impl InstallTaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            InstallTaskStatus::Completed | InstallTaskStatus::Failed | InstallTaskStatus::Canceled
        )
    }

    /// Project the 2.0 composite state into the engine vocabulary.
    pub fn from_task(
        phase: TaskPhase,
        outcome: Option<TaskOutcome>,
        wait: Option<&TaskWaitReason>,
    ) -> Self {
        match (phase, outcome) {
            (TaskPhase::Terminal, Some(TaskOutcome::Succeeded)) => InstallTaskStatus::Completed,
            (TaskPhase::Terminal, Some(TaskOutcome::Canceled)) => InstallTaskStatus::Canceled,
            (TaskPhase::Terminal, _) => InstallTaskStatus::Failed,
            (TaskPhase::Paused, _) => InstallTaskStatus::Paused,
            (TaskPhase::Waiting, _) => {
                if wait
                    .map(|reason| reason.kind == TaskWaitReasonKind::Authorization)
                    .unwrap_or(false)
                {
                    InstallTaskStatus::WaitingForApproval
                } else {
                    InstallTaskStatus::Paused
                }
            }
            (TaskPhase::Running, _) => InstallTaskStatus::Running,
            (TaskPhase::Promised, _) | (TaskPhase::Accepted, _) => InstallTaskStatus::Pending,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallTaskView {
    pub id: String,
    pub planning_task_id: String,
    pub parent_id: Option<String>,
    pub root_id: String,
    pub task_type: String,
    pub status: InstallTaskStatus,
    pub user_id: String,
    pub app_id: String,
    pub data: Value,
    pub phase: TaskPhase,
    pub outcome: Option<TaskOutcome>,
    pub wait_reason: Option<TaskWaitReason>,
    pub revision: u64,
    pub runner_epoch: u64,
    pub display: Option<AppInstallDisplayProgress>,
    pub updated_at: u64,
}

#[async_trait]
pub trait InstallTaskStore: Send + Sync {
    async fn create_install_task(
        &self,
        task_id: Option<&str>,
        name: &str,
        task_type: &str,
        data: Value,
        user_id: &str,
        app_id: &str,
        idempotency_key: &str,
        parent_id: Option<&str>,
        retry_of: Option<&str>,
    ) -> Result<String, InstallError>;
    async fn load(&self, task_id: &str) -> Result<InstallTaskView, InstallError>;
    /// 写入完整事务快照，使 Task.progress 与事务结构严格一致。
    async fn write_data(&self, task_id: &str, full_patch: Value) -> Result<(), InstallError>;
    async fn set_status(
        &self,
        task_id: &str,
        status: InstallTaskStatus,
        progress: Option<f32>,
        message: Option<String>,
    ) -> Result<(), InstallError>;
    async fn request_cancel(
        &self,
        _task_id: &str,
        _controller_user_id: &str,
        _controller_app_id: &str,
    ) -> Result<(), InstallError> {
        Ok(())
    }
    /// 本 runner 名下全部非终态 install/update 任务。
    async fn list_active(&self) -> Result<Vec<InstallTaskView>, InstallError>;
}

// ---------------------------------------------------------------------------
// stage driver 抽象
// ---------------------------------------------------------------------------

/// Resolve Stage 输出。
pub struct ResolveOutcome {
    pub candidate: Option<buckyos_api::CandidateHandle>,
    pub resolution: buckyos_api::DidResolutionSnapshot,
    /// 可用于 Inspect 的 App Document body（canonical JSON）。
    pub resolved_app_doc: Option<Value>,
}

/// Activate Stage 输出（instance 证据 + proof id 等）。
pub type ActivateOutcome = InstallTaskResult;

/// 每个 Stage 的实际执行者。实现必须幂等：同一持久状态下重复调用得到
/// 等价结果（引擎在恢复/重试时会重复调用）。
#[async_trait]
pub trait InstallStageDriver: Send + Sync {
    async fn resolve(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<ResolveOutcome, InstallError>;

    async fn inspect(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<Option<InstallInspection>, InstallError>;

    /// 补齐动态 status 中的 missing 内容；不可修改已批准的 immutable plan。
    async fn acquire(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<InstallPlanStatus, InstallError>;

    async fn verify(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<buckyos_api::VerificationReport, InstallError>;

    async fn prepare(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<PreparedDeployment, InstallError>;

    /// 写 spec（Deploy 的开始点）。
    async fn deploy(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError>;

    async fn activate(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<ActivateOutcome, InstallError>;

    /// spec 已写后的取消/失败回滚（恢复旧 spec 或清理新 spec）。
    async fn rollback_deploy(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError>;

    async fn release_staging(
        &self,
        _view: &InstallTaskView,
        _data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 引擎
// ---------------------------------------------------------------------------

fn stage_progress(stage: InstallStage) -> f32 {
    match stage {
        InstallStage::Resolve => 10.0,
        InstallStage::Inspect => 25.0,
        InstallStage::Acquire => 50.0,
        InstallStage::Verify => 65.0,
        InstallStage::Prepare => 75.0,
        InstallStage::Deploy => 85.0,
        InstallStage::Activate => 95.0,
    }
}

fn internal_error(stage: InstallStage, message: impl Into<String>) -> InstallError {
    InstallError::new(stage, InstallErrorCode::Internal, false, message)
}

/// run_task 的收敛结果（非错误的中途停靠点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    WaitingForApproval,
    /// 等待外部条件（信任解析等），任务已置 Paused。
    Waiting,
    /// 任务已是终态或被其它执行体持有。
    NotRun,
}

pub struct InstallEngine {
    store: Arc<dyn InstallTaskStore>,
    driver: Arc<dyn InstallStageDriver>,
    /// 进程内单任务执行守卫（防同进程重复推进；跨进程由 TaskManager 状态收敛）。
    running: Mutex<HashSet<String>>,
}

struct RunGuard<'a> {
    engine: &'a InstallEngine,
    task_id: String,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.engine.running.lock().unwrap().remove(&self.task_id);
    }
}

impl InstallEngine {
    pub fn new(store: Arc<dyn InstallTaskStore>, driver: Arc<dyn InstallStageDriver>) -> Self {
        Self {
            store,
            driver,
            running: Mutex::new(HashSet::new()),
        }
    }

    pub fn store(&self) -> &Arc<dyn InstallTaskStore> {
        &self.store
    }

    /// Side-effect-free Resolve + Inspect path used by Tool fetch/dry-run.
    /// It does not create a Task and never reaches Acquire/Prepare/Deploy.
    pub async fn inspect_request(
        &self,
        request: buckyos_api::AppInstallTaskRequest,
        task_type: &str,
        planning_task_id: Option<String>,
    ) -> Result<InstallInspection, InstallError> {
        let mut state = Self::initial_state_from_options(request.options.as_ref())?;
        if let Some(plan) = request.submitted_plan.as_ref() {
            state.requested_target = Some(plan.target.clone());
            state.requested_params = Some(plan.install_params.clone());
        }
        let mut data = AppInstallTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request,
            state,
        };
        let planning_task_id =
            planning_task_id.unwrap_or_else(|| format!("t-{}", uuid::Uuid::new_v4().simple()));
        let view = InstallTaskView {
            id: "inspect-only".to_string(),
            planning_task_id,
            parent_id: None,
            root_id: "inspect-only".to_string(),
            task_type: task_type.to_string(),
            status: InstallTaskStatus::Pending,
            user_id: data.request.creator_user_id.clone(),
            app_id: data.request.creator_app_id.clone(),
            data: serde_json::to_value(&data)
                .map_err(|err| internal_error(InstallStage::Resolve, err.to_string()))?,
            phase: TaskPhase::Accepted,
            outcome: None,
            wait_reason: None,
            revision: 0,
            runner_epoch: 0,
            display: None,
            updated_at: buckyos_get_unix_timestamp(),
        };
        let resolved = self.driver.resolve(&view, &data).await?;
        data.state.candidate = resolved.candidate;
        data.state.resolution = Some(resolved.resolution);
        data.state.resolved_app_doc = resolved.resolved_app_doc;
        self.driver.inspect(&view, &data).await?.ok_or_else(|| {
            InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::TrustResolutionRequired,
                true,
                "DID resolution did not return an inspectable App Document",
            )
        })
    }

    pub async fn status(
        &self,
        task_id: &str,
        requested_by: &str,
        requester_is_admin: bool,
    ) -> Result<AppInstallStatusSnapshot, InstallError> {
        let view = self.store.load(task_id).await?;
        self.ensure_task_owner(&view, requested_by, requester_is_admin)?;
        let data = self.parse_data(&view)?;
        let display = view.display.as_ref();
        let progress = display.map(|display| TaskDataProgress {
            message: display.message.clone(),
            updated_at: Some(display.updated_at as i64),
            ..Default::default()
        });
        Ok(buckyos_api::build_install_status_snapshot(
            view.id,
            view.phase,
            view.outcome,
            view.wait_reason.as_ref(),
            data.request.owner_user_id.as_str(),
            &data.request.source,
            data.request.policy,
            &data.state,
            progress,
            view.updated_at,
        ))
    }

    fn initial_state_from_options(
        options: Option<&Value>,
    ) -> Result<InstallTransactionState, InstallError> {
        let mut state = InstallTransactionState::default();
        let Some(options) = options else {
            return Ok(state);
        };
        let Some(options) = options.as_object() else {
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::InvalidRequest,
                false,
                "install options must be an object",
            ));
        };
        if let Some(value) = options.get("target").filter(|value| !value.is_null()) {
            state.requested_target =
                Some(serde_json::from_value(value.clone()).map_err(|err| {
                    InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::InvalidRequest,
                        false,
                        format!("invalid options.target: {err}"),
                    )
                })?);
        }
        if let Some(value) = options
            .get("install_params")
            .filter(|value| !value.is_null())
        {
            state.requested_params =
                Some(serde_json::from_value(value.clone()).map_err(|err| {
                    InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::InvalidRequest,
                        false,
                        format!("invalid options.install_params: {err}"),
                    )
                })?);
        }
        Ok(state)
    }

    // -------------------- 任务创建 --------------------

    /// 创建安装事务任务并返回 task_id。不在这里直接推进，由 runner 调度。
    pub async fn create_install_task(
        &self,
        request: buckyos_api::AppInstallTaskRequest,
        app_hint: &str,
    ) -> Result<String, InstallError> {
        let user_id = request.creator_user_id.clone();
        let creator_app_id = request.creator_app_id.clone();
        let idempotency_key = request.idempotency_key.clone();
        let mut state = Self::initial_state_from_options(request.options.as_ref())?;
        if let Some(plan) = request.submitted_plan.as_ref() {
            if plan.schema_version != APP_INSTALL_SCHEMA_VERSION || !plan.fingerprint_is_valid() {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::PlanStale,
                    false,
                    "submitted plan schema or fingerprint is invalid",
                ));
            }
            if plan.plan_use != buckyos_api::InstallPlanUse::FreshInstall {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::PlanNotApplicable,
                    false,
                    "only a FreshInstall plan can be submitted for first installation",
                ));
            }
            state.requested_target = Some(plan.target.clone());
            state.requested_params = Some(plan.install_params.clone());
        }
        let data = AppInstallTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request,
            state,
        };
        let task_name = format!("Install app ({app_hint})");
        let requested_task_id = data
            .request
            .submitted_plan
            .as_ref()
            .map(|plan| plan.task_id.as_str());
        let task_id = self
            .store
            .create_install_task(
                requested_task_id,
                task_name.as_str(),
                TASK_DATA_TYPE_APP_INSTALL,
                serde_json::to_value(&data)
                    .map_err(|err| internal_error(InstallStage::Resolve, err.to_string()))?,
                user_id.as_str(),
                creator_app_id.as_str(),
                idempotency_key.as_str(),
                None,
                None,
            )
            .await?;
        Ok(task_id)
    }

    /// 创建升级事务任务（同一 Stage 流水线，task_type = app.update）。
    pub async fn create_update_task(
        &self,
        request: buckyos_api::AppUpdateTaskRequest,
        app_hint: &str,
    ) -> Result<String, InstallError> {
        self.create_update_task_with_parent(request, app_hint, None)
            .await
    }

    pub async fn create_update_task_with_parent(
        &self,
        request: buckyos_api::AppUpdateTaskRequest,
        app_hint: &str,
        parent_id: Option<&str>,
    ) -> Result<String, InstallError> {
        let user_id = request.creator_user_id.clone();
        let creator_app_id = request.creator_app_id.clone();
        let idempotency_key = request.idempotency_key.clone();
        let mut state = Self::initial_state_from_options(request.options.as_ref())?;
        if let Some(plan) = request.submitted_plan.as_ref() {
            if plan.schema_version != APP_INSTALL_SCHEMA_VERSION || !plan.fingerprint_is_valid() {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::PlanStale,
                    false,
                    "submitted plan schema or fingerprint is invalid",
                ));
            }
            if plan.plan_use != buckyos_api::InstallPlanUse::Upgrade {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::PlanNotApplicable,
                    false,
                    "only an Upgrade plan can be submitted for an installed target",
                ));
            }
            state.requested_target = Some(plan.target.clone());
            state.requested_params = Some(plan.install_params.clone());
        }
        let data = buckyos_api::AppUpdateTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request,
            state,
        };
        let task_name = format!("Update app ({app_hint})");
        let requested_task_id = data
            .request
            .submitted_plan
            .as_ref()
            .map(|plan| plan.task_id.as_str());
        let task_id = self
            .store
            .create_install_task(
                requested_task_id,
                task_name.as_str(),
                TASK_DATA_TYPE_APP_UPDATE,
                serde_json::to_value(&data)
                    .map_err(|err| internal_error(InstallStage::Resolve, err.to_string()))?,
                user_id.as_str(),
                creator_app_id.as_str(),
                idempotency_key.as_str(),
                parent_id,
                None,
            )
            .await?;
        Ok(task_id)
    }

    // -------------------- 状态机推进 --------------------

    pub async fn run_task(&self, task_id: &str) -> Result<RunOutcome, InstallError> {
        // 进程内执行守卫。
        {
            let mut running = self.running.lock().unwrap();
            if !running.insert(task_id.to_string()) {
                return Ok(RunOutcome::NotRun);
            }
        }
        let _guard = RunGuard {
            engine: self,
            task_id: task_id.to_string(),
        };

        let view = self.store.load(task_id).await?;
        if view.status.is_terminal() {
            return Ok(RunOutcome::NotRun);
        }
        if matches!(
            view.status,
            InstallTaskStatus::WaitingForApproval | InstallTaskStatus::Paused
        ) {
            // 等确认/等外部条件的任务只由 confirm/retry 唤醒。
            return Ok(RunOutcome::NotRun);
        }

        match self.drive(view).await {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.record_failure(task_id, &error).await;
                Err(error)
            }
        }
    }

    async fn drive(&self, mut view: InstallTaskView) -> Result<RunOutcome, InstallError> {
        let task_id = view.id.clone();
        let mut data = self.parse_data(&view)?;

        if data.state.stage.is_none() {
            data.state.stage = Some(InstallStage::Resolve);
        }
        let _ = self
            .store
            .set_status(&task_id, InstallTaskStatus::Running, None, None)
            .await;

        loop {
            let stage = data
                .state
                .stage
                .ok_or_else(|| internal_error(InstallStage::Resolve, "stage pointer missing"))?;

            // 幂等跳过：已完成输出存在则直接指向下一 Stage。
            if data.state.is_stage_completed(stage) {
                data.state.stage = stage.next();
                if data.state.stage.is_none() {
                    return Ok(RunOutcome::Completed);
                }
                continue;
            }

            let _ = self
                .store
                .set_status(
                    &task_id,
                    InstallTaskStatus::Running,
                    Some(stage_progress(stage)),
                    Some(format!("stage: {stage}")),
                )
                .await;

            match stage {
                InstallStage::Resolve => {
                    let outcome = self.driver.resolve(&view, &data).await?;
                    data.state.candidate = outcome.candidate;
                    data.state.resolution = Some(outcome.resolution);
                    data.state.resolved_app_doc = outcome.resolved_app_doc;
                    data.state.last_error = None;
                    data.state.mark_stage_completed(InstallStage::Resolve);
                    self.persist(&task_id, &mut data).await?;

                    // app_did 已知：同一 user + App DID 只允许一个事务。
                    self.ensure_single_transaction(&view, &data).await?;

                    // 终止状态立即失败（不可重试）。
                    let snapshot = data.state.resolution.as_ref().unwrap();
                    if snapshot.document_status.is_terminal() {
                        return Err(InstallError::from_document_status(
                            InstallStage::Resolve,
                            snapshot.document_status,
                            &snapshot.app_did,
                        ));
                    }
                }
                InstallStage::Inspect => {
                    let Some(inspection) = self.driver.inspect(&view, &data).await? else {
                        let error = InstallError::new(
                            InstallStage::Resolve,
                            InstallErrorCode::TrustResolutionRequired,
                            true,
                            "DID resolution did not return an installable App Document",
                        )
                        .with_action(InstallUserAction::ConnectNetwork);
                        data.state.last_error = Some(error);
                        self.persist(&task_id, &mut data).await?;
                        let _ = self
                            .store
                            .set_status(
                                &task_id,
                                InstallTaskStatus::Paused,
                                None,
                                Some("waiting for trust resolution".to_string()),
                            )
                            .await;
                        return Ok(RunOutcome::Waiting);
                    };
                    if let Some(submitted) = data.request.submitted_plan.as_ref() {
                        if submitted.plan_fingerprint != inspection.plan.plan_fingerprint {
                            return Err(InstallError::new(
                                InstallStage::Inspect,
                                InstallErrorCode::PlanStale,
                                false,
                                "submitted plan no longer matches authoritative inspection",
                            ));
                        }
                    }
                    if let Some(approved_fingerprint) =
                        data.request.approved_plan_fingerprint.as_deref()
                    {
                        if approved_fingerprint != inspection.plan.plan_fingerprint {
                            return Err(InstallError::new(
                                InstallStage::Inspect,
                                InstallErrorCode::PlanStale,
                                false,
                                "approved fingerprint does not match authoritative inspection",
                            ));
                        }
                        data.state.approval = Some(InstallApproval {
                            plan_fingerprint: inspection.plan.plan_fingerprint.clone(),
                            target: inspection.plan.target.clone(),
                            install_params: inspection.plan.install_params.clone(),
                            approved_by: data.request.owner_user_id.clone(),
                            approved_at: buckyos_get_unix_timestamp(),
                            auto_confirmed: false,
                        });
                    }
                    data.state.plan = Some(inspection.plan);
                    data.state.plan_status = Some(inspection.status);
                    data.state.mark_stage_completed(InstallStage::Inspect);
                    self.persist(&task_id, &mut data).await?;

                    let plan = data.state.plan.as_ref().unwrap();
                    let plan_status = data.state.plan_status.as_ref().ok_or_else(|| {
                        internal_error(InstallStage::Inspect, "inspection status missing")
                    })?;
                    match plan_status.readiness.install {
                        InstallReadiness::IdentityRevoked => {
                            return Err(InstallError::from_document_status(
                                InstallStage::Inspect,
                                plan.resolution.document_status,
                                &plan.app.did,
                            ));
                        }
                        InstallReadiness::UnsupportedTarget => {
                            return Err(InstallError::new(
                                InstallStage::Inspect,
                                InstallErrorCode::UnsupportedTarget,
                                false,
                                format!(
                                    "no package matches target {}/{}",
                                    plan.target.os, plan.target.arch
                                ),
                            )
                            .with_action(InstallUserAction::ChangeTarget));
                        }
                        InstallReadiness::InvalidPackage => {
                            return Err(InstallError::new(
                                InstallStage::Inspect,
                                InstallErrorCode::InvalidPackage,
                                false,
                                "package failed structural validation",
                            ));
                        }
                        InstallReadiness::TrustResolutionRequired => {
                            // 进入 WAITING_FOR_TRUST_RESOLUTION：Paused + 可重试错误。
                            let error = InstallError::new(
                                InstallStage::Resolve,
                                InstallErrorCode::TrustResolutionRequired,
                                true,
                                "content may be ready but DID trust evidence is not acceptable yet",
                            )
                            .with_action(InstallUserAction::ConnectNetwork);
                            data.state.last_error = Some(error);
                            self.persist(&task_id, &mut data).await?;
                            let _ = self
                                .store
                                .set_status(
                                    &task_id,
                                    InstallTaskStatus::Paused,
                                    None,
                                    Some("waiting for trust resolution".to_string()),
                                )
                                .await;
                            return Ok(RunOutcome::Waiting);
                        }
                        InstallReadiness::OfflineReady
                        | InstallReadiness::ContentDownloadRequired
                        | InstallReadiness::ConfigBlocked => {}
                    }
                }
                InstallStage::Acquire => {
                    // 确认闸门：进入 Acquire 前必须有绑定当前 plan 的 approval。
                    match self.approval_gate(&task_id, &mut data).await? {
                        ApprovalGate::Approved => {}
                        ApprovalGate::Waiting => return Ok(RunOutcome::WaitingForApproval),
                    }

                    let updated_status = self.driver.acquire(&view, &data).await?;
                    if updated_status.plan_fingerprint
                        != data.state.plan.as_ref().unwrap().plan_fingerprint
                    {
                        return Err(internal_error(
                            InstallStage::Acquire,
                            "acquire changed immutable plan fingerprint",
                        ));
                    }
                    data.state.plan_status = Some(updated_status);
                    data.state.mark_stage_completed(InstallStage::Acquire);
                    self.persist(&task_id, &mut data).await?;
                }
                InstallStage::Verify => {
                    let report = self.driver.verify(&view, &data).await?;
                    let passed = report.passed;
                    data.state.verification = Some(report);
                    if !passed {
                        self.persist(&task_id, &mut data).await?;
                        return Err(InstallError::new(
                            InstallStage::Verify,
                            InstallErrorCode::VerificationFailed,
                            true,
                            "verification report has failed checks",
                        ));
                    }
                    // Trust + Content + Config 全 ready 才进入 Prepare。
                    let status = data.state.plan_status.as_ref().ok_or_else(|| {
                        internal_error(InstallStage::Verify, "plan status missing")
                    })?;
                    let install_readiness = status.readiness.install;
                    if !install_readiness.is_ready() {
                        self.persist(&task_id, &mut data).await?;
                        return Err(InstallError::new(
                            InstallStage::Verify,
                            InstallErrorCode::VerificationFailed,
                            true,
                            format!(
                                "plan readiness is {:?} after acquisition; refusing to prepare",
                                install_readiness
                            ),
                        ));
                    }
                    data.state.mark_stage_completed(InstallStage::Verify);
                    self.persist(&task_id, &mut data).await?;
                }
                InstallStage::Prepare => {
                    let prepared = self.driver.prepare(&view, &data).await?;
                    data.state.prepared = Some(prepared);
                    data.state.mark_stage_completed(InstallStage::Prepare);
                    self.persist(&task_id, &mut data).await?;
                }
                InstallStage::Deploy => {
                    self.driver.deploy(&view, &data).await?;
                    data.state.mark_stage_completed(InstallStage::Deploy);
                    self.persist(&task_id, &mut data).await?;
                }
                InstallStage::Activate => {
                    let activate_result = self.driver.activate(&view, &data).await;
                    let result = match activate_result {
                        Ok(result) => result,
                        Err(error) => {
                            // 升级：新版本 Activate 失败恢复旧 spec/运行状态
                            //（本机部署回滚，不改变 resolver 状态，P4.4）。
                            if view.task_type == TASK_DATA_TYPE_APP_UPDATE {
                                data.state.last_error = Some(error.clone());
                                self.persist(&task_id, &mut data).await?;
                                if let Err(rollback_err) =
                                    self.driver.rollback_deploy(&view, &data).await
                                {
                                    warn!(
                                        "rollback after failed activation also failed: {rollback_err}"
                                    );
                                } else {
                                    // 旧 spec 已恢复：Deploy 输出失效，retry 时
                                    // 必须重写新 spec 再 Activate。
                                    data.state.invalidate_from(InstallStage::Deploy);
                                    self.persist(&task_id, &mut data).await?;
                                }
                            }
                            return Err(error);
                        }
                    };
                    data.state.result = Some(result);
                    data.state.mark_stage_completed(InstallStage::Activate);
                    self.persist(&task_id, &mut data).await?;
                    self.driver.release_staging(&view, &data).await?;
                    let _ = self
                        .store
                        .set_status(
                            &task_id,
                            InstallTaskStatus::Completed,
                            Some(100.0),
                            Some("App installed".to_string()),
                        )
                        .await;
                    info!("install task {task_id} completed");
                    return Ok(RunOutcome::Completed);
                }
            }

            // 重新加载视图（供 driver 观察最新 message/status；数据以本地为准）。
            view = self.store.load(&task_id).await?;
        }
    }

    // -------------------- 确认 / 重试 / 取消 --------------------

    /// 用户确认只绑定已展示的 plan fingerprint。参数变化必须先走独立
    /// recompute 接口，confirm 不得一边修改计划一边批准。
    pub async fn confirm(
        &self,
        task_id: &str,
        approved_by: &str,
        approver_is_admin: bool,
        plan_fingerprint: &str,
    ) -> Result<(), InstallError> {
        let view = self.store.load(task_id).await?;
        self.ensure_task_owner(&view, approved_by, approver_is_admin)?;
        if view.status != InstallTaskStatus::WaitingForApproval {
            return Err(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::Conflict,
                false,
                format!(
                    "task {task_id} is not waiting for approval (status {:?})",
                    view.status
                ),
            ));
        }
        let mut data = self.parse_data(&view)?;
        let plan = data.state.plan.clone().ok_or_else(|| {
            internal_error(InstallStage::Inspect, "waiting task has no persisted plan")
        })?;
        if plan.plan_fingerprint != plan_fingerprint {
            return Err(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::PlanStale,
                false,
                "approved fingerprint does not match the persisted plan",
            ));
        }
        let status = data.state.plan_status.clone().ok_or_else(|| {
            internal_error(
                InstallStage::Inspect,
                "waiting task has no persisted inspection",
            )
        })?;
        if !status.readiness.target.is_ready() {
            data.state.approval = None;
            self.persist(&task_id, &mut data).await?;
            return Err(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::UnsupportedTarget,
                false,
                status.target_issues.join("; "),
            )
            .with_action(InstallUserAction::ChangeTarget));
        }
        if !status.readiness.config.is_ready() {
            data.state.approval = None;
            self.persist(&task_id, &mut data).await?;
            return Err(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::ConfigBlocked,
                false,
                status.config_issues.join("; "),
            )
            .with_action(InstallUserAction::Confirm));
        }

        data.state.approval = Some(InstallApproval {
            plan_fingerprint: plan.plan_fingerprint.clone(),
            target: plan.target.clone(),
            install_params: plan.install_params.clone(),
            approved_by: approved_by.to_string(),
            approved_at: buckyos_get_unix_timestamp(),
            auto_confirmed: false,
        });
        data.state.last_error = None;
        self.persist(&task_id, &mut data).await?;
        self.store
            .set_status(
                &task_id,
                InstallTaskStatus::Running,
                None,
                Some("approved, continuing".to_string()),
            )
            .await?;
        Ok(())
    }

    /// 重试：从失败 Stage 失效重算。只清除失效输出，不重做已完成 Stage。
    pub async fn retry(
        &self,
        task_id: &str,
        requested_by: &str,
        requester_is_admin: bool,
        requester_app_id: &str,
        idempotency_key: &str,
    ) -> Result<String, InstallError> {
        let view = self.store.load(task_id).await?;
        self.ensure_task_owner(&view, requested_by, requester_is_admin)?;
        if !matches!(
            view.status,
            InstallTaskStatus::Failed | InstallTaskStatus::Paused
        ) {
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::Conflict,
                false,
                format!(
                    "task {task_id} is not retryable in status {:?}",
                    view.status
                ),
            ));
        }
        let mut data = self.parse_data(&view)?;
        if let Some(error) = data.state.last_error.as_ref() {
            if !error.retryable {
                return Err(InstallError::new(
                    error.stage,
                    error.code,
                    false,
                    format!(
                        "task {task_id} failed with non-retryable error: {}",
                        error.message
                    ),
                ));
            }
        }
        // 失败 Stage 与当前指针取更早者：失败后的自动回滚可能已把指针
        // 回退（如升级回滚后指回 Deploy），retry 不得跳过被作废的 Stage。
        let error_stage = data.state.last_error.as_ref().map(|error| error.stage);
        let pointer_stage = data.state.stage;
        let retry_stage = match (error_stage, pointer_stage) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => InstallStage::Resolve,
        };
        data.state.invalidate_from(retry_stage);
        data.state.last_error = None;
        if view.status == InstallTaskStatus::Paused {
            self.persist(&task_id, &mut data).await?;
            self.store
                .set_status(
                    &task_id,
                    InstallTaskStatus::Running,
                    None,
                    Some(format!("resuming from stage {retry_stage}")),
                )
                .await?;
            return Ok(task_id.to_string());
        }

        data.request.creator_user_id = requested_by.to_string();
        data.request.creator_app_id = requester_app_id.to_string();
        data.request.idempotency_key = idempotency_key.to_string();
        let new_task_id = self
            .store
            .create_install_task(
                None,
                format!("Retry {}", view.task_type).as_str(),
                view.task_type.as_str(),
                serde_json::to_value(&data)
                    .map_err(|err| internal_error(InstallStage::Resolve, err.to_string()))?,
                requested_by,
                requester_app_id,
                idempotency_key,
                view.parent_id.as_deref(),
                Some(task_id),
            )
            .await?;
        Ok(new_task_id)
    }

    /// 取消：Prepare 前直接取消；spec 已写（Deploy 开始）后先回滚再取消。
    pub async fn cancel(
        &self,
        task_id: &str,
        requested_by: &str,
        requester_app_id: &str,
        requester_is_admin: bool,
    ) -> Result<(), InstallError> {
        let view = self.store.load(task_id).await?;
        self.ensure_task_owner(&view, requested_by, requester_is_admin)?;
        if view.status.is_terminal() {
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::Conflict,
                false,
                format!("task {task_id} already terminal"),
            ));
        }
        self.store
            .request_cancel(task_id, requested_by, requester_app_id)
            .await?;
        let mut data = self.parse_data(&view)?;
        let spec_written = data.state.is_stage_completed(InstallStage::Deploy)
            || matches!(data.state.stage, Some(InstallStage::Deploy))
            || matches!(data.state.stage, Some(InstallStage::Activate));

        if spec_written {
            // 只在安全边界生效：先回滚/停止收敛。
            self.driver.rollback_deploy(&view, &data).await?;
        }
        let error = InstallError::new(
            data.state.stage.unwrap_or(InstallStage::Resolve),
            InstallErrorCode::Canceled,
            false,
            format!("canceled by {requested_by}"),
        );
        data.state.last_error = Some(error);
        self.persist(&task_id, &mut data).await?;
        self.driver.release_staging(&view, &data).await?;
        self.store
            .set_status(
                &task_id,
                InstallTaskStatus::Canceled,
                None,
                Some(format!("canceled by {requested_by}")),
            )
            .await?;
        Ok(())
    }

    // -------------------- 内部 --------------------

    fn parse_data(&self, view: &InstallTaskView) -> Result<AppInstallTaskData, InstallError> {
        if view.task_type != TASK_DATA_TYPE_APP_INSTALL
            && view.task_type != TASK_DATA_TYPE_APP_UPDATE
        {
            return Err(internal_error(
                InstallStage::Resolve,
                format!(
                    "task {} is not an install task: {}",
                    view.id, view.task_type
                ),
            ));
        }
        let data: AppInstallTaskData =
            serde_json::from_value(view.data.clone()).map_err(|err| {
                internal_error(
                    InstallStage::Resolve,
                    format!(
                        "task {} data is not a valid install transaction: {err}",
                        view.id
                    ),
                )
            })?;
        if data.schema_version != APP_INSTALL_SCHEMA_VERSION {
            return Err(internal_error(
                InstallStage::Resolve,
                format!(
                    "task {} has unsupported app install schema_version {}, expected {}",
                    view.id, data.schema_version, APP_INSTALL_SCHEMA_VERSION
                ),
            ));
        }
        if let Some(plan) = data.state.plan.as_ref() {
            if plan.schema_version != APP_INSTALL_SCHEMA_VERSION {
                return Err(internal_error(
                    InstallStage::Inspect,
                    format!(
                        "task {} plan has unsupported schema_version {}, expected {}",
                        view.id, plan.schema_version, APP_INSTALL_SCHEMA_VERSION
                    ),
                ));
            }
        }
        Ok(data)
    }

    async fn persist(
        &self,
        task_id: &str,
        data: &mut AppInstallTaskData,
    ) -> Result<(), InstallError> {
        data.state.stage_revision = data.state.stage_revision.saturating_add(1);
        self.store.write_data(task_id, data.to_full_patch()).await
    }

    async fn record_failure(&self, task_id: &str, error: &InstallError) {
        // 尽力持久化结构化错误；失败也要把任务状态标出去。
        if let Ok(view) = self.store.load(task_id).await {
            if let Ok(mut data) = self.parse_data(&view) {
                data.state.last_error = Some(error.clone());
                let _ = self.persist(&task_id, &mut data).await;
                let _ = self.driver.release_staging(&view, &data).await;
            }
        }
        let target_status = if error.code == InstallErrorCode::Canceled {
            InstallTaskStatus::Canceled
        } else {
            InstallTaskStatus::Failed
        };
        let _ = self
            .store
            .set_status(&task_id, target_status, None, Some(error.to_string()))
            .await;
        warn!("install task {task_id} failed: {error}");
    }

    fn ensure_task_owner(
        &self,
        view: &InstallTaskView,
        requested_by: &str,
        requester_is_admin: bool,
    ) -> Result<(), InstallError> {
        if requested_by != view.user_id && !requester_is_admin {
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::InvalidRequest,
                false,
                format!(
                    "task {} belongs to `{}`, not `{}`",
                    view.id, view.user_id, requested_by
                ),
            ));
        }
        Ok(())
    }

    /// 同一 user + App DID 只允许一个进行中的 Deploy/Upgrade 事务。
    /// 冲突时较早创建（task_id 较小）者胜出。
    async fn ensure_single_transaction(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let Some(app_did) = data
            .state
            .resolution
            .as_ref()
            .map(|snapshot| snapshot.app_did.to_string())
        else {
            return Ok(());
        };

        let active = self.store.list_active().await?;
        for other in active {
            if other.id == view.id || other.user_id != view.user_id {
                continue;
            }
            if other.status.is_terminal() {
                continue;
            }
            let other_did = other
                .data
                .get("resolution")
                .and_then(|resolution| resolution.get("app_did"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            if other_did.as_deref() == Some(app_did.as_str()) && other.id < view.id {
                return Err(InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::Conflict,
                    false,
                    format!(
                        "another install/update transaction (task {}) is already running for `{app_did}`",
                        other.id
                    ),
                ));
            }
        }
        Ok(())
    }

    async fn approval_gate(
        &self,
        task_id: &str,
        data: &mut AppInstallTaskData,
    ) -> Result<ApprovalGate, InstallError> {
        let plan = data
            .state
            .plan
            .as_ref()
            .ok_or_else(|| internal_error(InstallStage::Acquire, "plan missing at approval gate"))?
            .clone();

        if let Some(approval) = data.state.approval.as_ref() {
            if approval.plan_fingerprint == plan.plan_fingerprint {
                return Ok(ApprovalGate::Approved);
            }
            // plan 已变化：旧确认失效。
            warn!("task {task_id} approval fingerprint mismatch; requiring re-confirmation");
            data.state.approval = None;
            self.persist(&task_id, data).await?;
        }

        // SYSTEM_INTERNAL 可显式 auto-confirm；其余策略不得默认跳过确认。
        let auto_requested = data
            .request
            .options
            .as_ref()
            .and_then(|options| options.get("auto_confirm"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if auto_requested && data.request.policy.allow_auto_confirm() {
            let status = data.state.plan_status.as_ref().ok_or_else(|| {
                internal_error(
                    InstallStage::Inspect,
                    "plan status missing at approval gate",
                )
            })?;
            if !status.readiness.config.is_ready() {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::ConfigBlocked,
                    false,
                    status.config_issues.join("; "),
                ));
            }
            data.state.approval = Some(InstallApproval {
                plan_fingerprint: plan.plan_fingerprint.clone(),
                target: plan.target.clone(),
                install_params: plan.install_params.clone(),
                approved_by: "system".to_string(),
                approved_at: buckyos_get_unix_timestamp(),
                auto_confirmed: true,
            });
            self.persist(&task_id, data).await?;
            return Ok(ApprovalGate::Approved);
        }

        self.store
            .set_status(
                &task_id,
                InstallTaskStatus::WaitingForApproval,
                Some(stage_progress(InstallStage::Inspect)),
                Some("waiting for user approval".to_string()),
            )
            .await?;
        Ok(ApprovalGate::Waiting)
    }
}

enum ApprovalGate {
    Approved,
    Waiting,
}

// ---------------------------------------------------------------------------
// 生产 store：TaskManagerClient 适配
// ---------------------------------------------------------------------------

pub struct TaskMgrInstallStore;

impl TaskMgrInstallStore {
    fn map_err(context: &str, err: impl std::fmt::Display) -> InstallError {
        internal_error(InstallStage::Resolve, format!("{context}: {err}"))
    }

    async fn client() -> Result<buckyos_api::TaskManagerClient, InstallError> {
        let runtime = buckyos_api::get_buckyos_api_runtime()
            .map_err(|err| Self::map_err("get runtime", err))?;
        runtime
            .get_task_mgr_client()
            .await
            .map_err(|err| Self::map_err("get task mgr client", err))
    }

    fn transaction_from_task(task: &buckyos_api::Task) -> Result<AppInstallTaskData, InstallError> {
        if let Some(progress) = task.progress.as_ref() {
            let envelope: AppInstallProgressEnvelope = serde_json::from_value(progress.clone())
                .map_err(|err| Self::map_err("decode install progress envelope", err))?;
            if envelope.schema_version != APP_INSTALL_SCHEMA_VERSION
                || envelope.transaction_revision != envelope.transaction.state.stage_revision
            {
                return Err(internal_error(
                    InstallStage::Resolve,
                    format!(
                        "task {} has an invalid install progress envelope",
                        task.task_id
                    ),
                ));
            }
            return Ok(envelope.transaction);
        }
        serde_json::from_value(task.input.clone())
            .map_err(|err| Self::map_err("decode immutable install task input", err))
    }

    fn progress_envelope(
        task: &buckyos_api::Task,
    ) -> Result<Option<AppInstallProgressEnvelope>, InstallError> {
        task.progress
            .as_ref()
            .map(|value| {
                serde_json::from_value(value.clone())
                    .map_err(|err| Self::map_err("decode install progress envelope", err))
            })
            .transpose()
    }

    fn runner_envelope(task: &buckyos_api::Task) -> buckyos_api::RunnerWriteEnvelope {
        let app_instance_id = match &task.executor {
            TaskExecutor::App {
                app_instance_id, ..
            } => app_instance_id.clone(),
            _ => None,
        };
        buckyos_api::RunnerWriteEnvelope {
            task_id: task.task_id.clone(),
            app_instance_id,
            runner_epoch: task.runner_epoch,
            expected_revision: task.revision,
        }
    }

    fn view_from_task(task: buckyos_api::Task) -> Result<InstallTaskView, InstallError> {
        let status =
            InstallTaskStatus::from_task(task.phase, task.outcome, task.wait_reason.as_ref());
        let task_type = install_task_type(&task.schema_id).to_string();
        let progress_envelope = Self::progress_envelope(&task)?;
        let display = progress_envelope
            .as_ref()
            .map(|envelope| envelope.display.clone());
        let transaction = Self::transaction_from_task(&task)?;
        if task.phase == TaskPhase::Terminal && task.outcome == Some(TaskOutcome::Succeeded) {
            let result = task.result.as_ref().ok_or_else(|| {
                internal_error(
                    InstallStage::Activate,
                    format!("succeeded task {} has no terminal result", task.task_id),
                )
            })?;
            serde_json::from_value::<AppInstallTerminalOutput>(result.clone())
                .map_err(|err| Self::map_err("decode install terminal output", err))?;
        }
        let data = serde_json::to_value(transaction)
            .map_err(|err| Self::map_err("encode install transaction view", err))?;
        Ok(InstallTaskView {
            id: task.task_id.clone(),
            planning_task_id: task.task_id,
            parent_id: task.parent_id,
            root_id: task.root_id,
            task_type,
            status,
            user_id: task.creator.user_id,
            app_id: task.creator.app_id,
            data,
            phase: task.phase,
            outcome: task.outcome,
            wait_reason: task.wait_reason,
            revision: task.revision,
            runner_epoch: task.runner_epoch,
            display,
            updated_at: task.updated_at,
        })
    }
}

#[async_trait]
impl InstallTaskStore for TaskMgrInstallStore {
    async fn create_install_task(
        &self,
        task_id: Option<&str>,
        name: &str,
        task_type: &str,
        data: Value,
        user_id: &str,
        app_id: &str,
        idempotency_key: &str,
        parent_id: Option<&str>,
        retry_of: Option<&str>,
    ) -> Result<String, InstallError> {
        let client = Self::client().await?;
        let task = client
            .create_delegated_task(buckyos_api::CreateDelegatedTaskReq {
                task_id: task_id.map(ToOwned::to_owned),
                name: name.to_string(),
                schema_id: install_schema_id(task_type),
                schema_version: None,
                input: data,
                creator: buckyos_api::ActorRef::new(user_id, app_id),
                runner_app_instance_id: None,
                parent_id: parent_id.map(ToOwned::to_owned),
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                idempotency_key: idempotency_key.to_string(),
                retry_of: retry_of.map(ToOwned::to_owned),
                supersedes: None,
                message: None,
            })
            .await
            .map_err(|err| Self::map_err("create task", err))?;
        Ok(task.task_id)
    }

    async fn load(&self, task_id: &str) -> Result<InstallTaskView, InstallError> {
        let client = Self::client().await?;
        let task = client
            .get_task(task_id)
            .await
            .map_err(|err| Self::map_err("get task", err))?;
        Self::view_from_task(task)
    }

    async fn write_data(&self, task_id: &str, full_patch: Value) -> Result<(), InstallError> {
        let client = Self::client().await?;
        let transaction: AppInstallTaskData = serde_json::from_value(full_patch)
            .map_err(|err| Self::map_err("encode transaction progress", err))?;
        let task = client
            .get_task(task_id)
            .await
            .map_err(|err| Self::map_err("get task for transaction write", err))?;
        let current = Self::transaction_from_task(&task)?;
        if transaction.state.stage_revision < current.state.stage_revision {
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::Conflict,
                true,
                format!(
                    "stale transaction revision {} (current {})",
                    transaction.state.stage_revision, current.state.stage_revision
                ),
            ));
        }
        if transaction.state.stage_revision == current.state.stage_revision {
            if transaction == current {
                return Ok(());
            }
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::Conflict,
                true,
                "conflicting install transaction write at the same revision",
            ));
        }
        let display = Self::progress_envelope(&task)?
            .map(|envelope| envelope.display)
            .unwrap_or_default();
        let progress = serde_json::to_value(AppInstallProgressEnvelope {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            transaction_revision: transaction.state.stage_revision,
            transaction,
            display,
        })
        .map_err(|err| Self::map_err("serialize install progress envelope", err))?;
        client
            .report_progress(buckyos_api::ReportProgressReq {
                envelope: Self::runner_envelope(&task),
                progress: Some(progress),
                message: task.message.clone(),
            })
            .await
            .map_err(|err| Self::map_err("update task data", err))?;
        Ok(())
    }

    async fn set_status(
        &self,
        task_id: &str,
        status: InstallTaskStatus,
        progress: Option<f32>,
        message: Option<String>,
    ) -> Result<(), InstallError> {
        let client = Self::client().await?;
        let task = client
            .get_task(task_id)
            .await
            .map_err(|err| Self::map_err("get task for status update", err))?;
        let transaction = Self::transaction_from_task(&task)?;
        let mut display = Self::progress_envelope(&task)?
            .map(|envelope| envelope.display)
            .unwrap_or_default();
        if progress.is_some() {
            display.percent = progress;
        }
        if message.is_some() {
            display.message = message.clone();
        }
        display.stage = transaction.state.stage;
        display.updated_at = buckyos_get_unix_timestamp();
        let progress_value = serde_json::to_value(AppInstallProgressEnvelope {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            transaction_revision: transaction.state.stage_revision,
            transaction: transaction.clone(),
            display: display.clone(),
        })
        .map_err(|err| Self::map_err("serialize install display progress", err))?;
        if !task.phase.is_terminal() {
            client
                .report_progress(buckyos_api::ReportProgressReq {
                    envelope: Self::runner_envelope(&task),
                    progress: Some(progress_value),
                    message: message.clone(),
                })
                .await
                .map_err(|err| Self::map_err("update task display", err))?;
        }
        let result = match status {
            InstallTaskStatus::Running | InstallTaskStatus::Pending => {
                client.runner_start(task_id).await.map(|_| ())
            }
            InstallTaskStatus::WaitingForApproval => client
                .runner_wait(
                    task_id,
                    TaskWaitReason {
                        kind: TaskWaitReasonKind::Authorization,
                        code: Some("install_approval".to_string()),
                        related_task_id: None,
                        message: None,
                    },
                )
                .await
                .map(|_| ()),
            InstallTaskStatus::Paused => client
                .runner_wait(
                    task_id,
                    TaskWaitReason::with_code(TaskWaitReasonKind::Other, "install_paused"),
                )
                .await
                .map(|_| ()),
            InstallTaskStatus::Completed => {
                let install_result = transaction.state.result.clone().ok_or_else(|| {
                    internal_error(
                        InstallStage::Activate,
                        "completed transaction has no result",
                    )
                })?;
                let progress = TaskDataProgress {
                    message: display.message.clone(),
                    updated_at: Some(display.updated_at as i64),
                    ..Default::default()
                };
                let snapshot = buckyos_api::build_install_status_snapshot(
                    task_id.to_string(),
                    TaskPhase::Terminal,
                    Some(TaskOutcome::Succeeded),
                    None,
                    transaction.request.owner_user_id.as_str(),
                    &transaction.request.source,
                    transaction.request.policy,
                    &transaction.state,
                    Some(progress),
                    display.updated_at,
                );
                let terminal = serde_json::to_value(AppInstallTerminalOutput {
                    schema_version: APP_INSTALL_SCHEMA_VERSION,
                    transaction_revision: transaction.state.stage_revision,
                    result: install_result,
                    status: snapshot,
                })
                .map_err(|err| Self::map_err("serialize install terminal output", err))?;
                client.runner_complete(task_id, terminal).await.map(|_| ())
            }
            InstallTaskStatus::Failed => client
                .runner_fail(
                    task_id,
                    "install_failed",
                    message.unwrap_or_else(|| "install failed".to_string()),
                    None,
                )
                .await
                .map(|_| ()),
            InstallTaskStatus::Canceled => match client.get_task(task_id).await {
                Ok(task) if task.phase.is_terminal() => Ok(()),
                Ok(task) if task.pending_control.is_some() => {
                    let request_id = task
                        .pending_control
                        .as_ref()
                        .map(|pending| pending.request_id.clone())
                        .unwrap();
                    client
                        .ack_control(buckyos_api::AckControlReq {
                            envelope: buckyos_api::RunnerWriteEnvelope {
                                task_id: task.task_id.clone(),
                                app_instance_id: None,
                                runner_epoch: task.runner_epoch,
                                expected_revision: task.revision,
                            },
                            request_id,
                            applied: true,
                            reject_reason: None,
                        })
                        .await
                        .map(|_| ())
                }
                Ok(_) => Err(kRPC::RPCErrors::ReasonError(
                    "cancel intent is missing before terminal acknowledgement".to_string(),
                )),
                Err(error) => Err(error),
            },
        };
        result.map_err(|err| Self::map_err("update task", err))?;
        Ok(())
    }

    async fn request_cancel(
        &self,
        task_id: &str,
        controller_user_id: &str,
        controller_app_id: &str,
    ) -> Result<(), InstallError> {
        let client = Self::client().await?;
        let task = client
            .get_task(task_id)
            .await
            .map_err(|err| Self::map_err("get task for cancel request", err))?;
        if task.phase.is_terminal()
            || task
                .pending_control
                .as_ref()
                .is_some_and(|pending| pending.action == buckyos_api::TaskControlAction::Cancel)
        {
            return Ok(());
        }
        client
            .request_delegated_control(buckyos_api::RequestDelegatedControlReq {
                controller: buckyos_api::ActorRef::new(controller_user_id, controller_app_id),
                task_id: task_id.to_string(),
                action: buckyos_api::TaskControlAction::Cancel,
                request_id: format!("app-install-cancel:{task_id}:{}", task.revision),
                expected_revision: Some(task.revision),
            })
            .await
            .map_err(|err| Self::map_err("persist cancel request", err))?;
        Ok(())
    }

    async fn list_active(&self) -> Result<Vec<InstallTaskView>, InstallError> {
        let client = Self::client().await?;
        let mut views = Vec::new();
        // 只扫本 Service 职责域内的 schema（app.install / app.update）。
        for task_type in [TASK_DATA_TYPE_APP_INSTALL, TASK_DATA_TYPE_APP_UPDATE] {
            let mut cursor = None;
            loop {
                let page = client
                    .list_tasks(buckyos_api::ListTasksReq {
                        schema_id: Some(install_schema_id(task_type)),
                        runner_app_id: Some(buckyos_api::CONTROL_PANEL_SERVICE_NAME.to_string()),
                        cursor: cursor.clone(),
                        limit: Some(100),
                        ..Default::default()
                    })
                    .await
                    .map_err(|err| Self::map_err("list tasks", err))?;
                for summary in page.tasks {
                    if summary.phase.is_terminal() {
                        continue;
                    }
                    let Ok(task) = client.get_task(&summary.task_id).await else {
                        continue;
                    };
                    views.push(Self::view_from_task(task)?);
                }
                let Some(next) = page.next_cursor else {
                    break;
                };
                cursor = Some(next);
            }
        }
        Ok(views)
    }
}

/// Versioned schema id for an install-family legacy task type.
fn install_schema_id(task_type: &str) -> String {
    format!("{}/v1", task_type.replace('/', "."))
}

fn install_task_type(schema_id: &str) -> &str {
    if schema_id == install_schema_id(TASK_DATA_TYPE_APP_INSTALL) {
        TASK_DATA_TYPE_APP_INSTALL
    } else if schema_id == install_schema_id(TASK_DATA_TYPE_APP_UPDATE) {
        TASK_DATA_TYPE_APP_UPDATE
    } else {
        schema_id
    }
}

// ---------------------------------------------------------------------------
// 单元测试：fake store + fake driver 跑全 Stage
// ---------------------------------------------------------------------------
