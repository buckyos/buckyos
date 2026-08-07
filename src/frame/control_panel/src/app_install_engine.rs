//! 安装事务引擎：可恢复的 Stage 状态机（doc/App 安装协议.md §3、§5.5）。
//!
//! 职责边界：
//! - 引擎只负责状态机推进、持久化纪律与恢复语义；每个 Stage 的实际工作
//!   由 `InstallStageDriver` 完成（生产实现分布在 resolver/planner/pikg 与
//!   Acquire/Deploy 适配层，单元测试用 fake 跑全 Stage）；
//! - 每个 Stage 成功后先把完整事务状态写回 Task.data（全量镜像 patch，
//!   缺席字段写 null 以配合 deep-merge 删除语义），再进入下一 Stage；
//! - 重启恢复只相信 Task.data 持久状态；Stage handler 可重复调用；
//! - TaskManager 是任务状态唯一真相源；MsgQueue/KEvent/启动扫描只是
//!   dispatch 通道（kevent 只是加速，见 runner 模块）。

use async_trait::async_trait;
use buckyos_api::{
    AppInstallTaskData, InstallApproval, InstallError, InstallErrorCode, InstallParams,
    InstallPlan, InstallReadiness, InstallStage, InstallTarget, InstallTaskResult,
    InstallTransactionState, InstallUserAction, PreparedDeployment, TaskStatus,
    APP_INSTALL_SCHEMA_VERSION, TASK_DATA_TYPE_APP_INSTALL, TASK_DATA_TYPE_APP_UPDATE,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::{info, warn};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// store 抽象（生产 = TaskManagerClient；测试 = 内存实现）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct InstallTaskView {
    pub id: i64,
    pub root_id: String,
    pub task_type: String,
    pub status: TaskStatus,
    pub user_id: String,
    pub app_id: String,
    pub data: Value,
}

#[async_trait]
pub trait InstallTaskStore: Send + Sync {
    async fn create_install_task(
        &self,
        name: &str,
        task_type: &str,
        data: Value,
        user_id: &str,
        app_id: &str,
    ) -> Result<i64, InstallError>;
    async fn load(&self, task_id: i64) -> Result<InstallTaskView, InstallError>;
    /// 写入全量镜像 patch（deep merge 后 Task.data 与事务结构严格一致）。
    async fn write_data(&self, task_id: i64, full_patch: Value) -> Result<(), InstallError>;
    async fn set_status(
        &self,
        task_id: i64,
        status: TaskStatus,
        progress: Option<f32>,
        message: Option<String>,
    ) -> Result<(), InstallError>;
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
    ) -> Result<Option<InstallPlan>, InstallError>;

    /// 补齐 plan.missing；返回内容位置已刷新、readiness 已重组的 plan。
    async fn acquire(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<InstallPlan, InstallError>;

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
    running: Mutex<HashSet<i64>>,
}

struct RunGuard<'a> {
    engine: &'a InstallEngine,
    task_id: i64,
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
    ) -> Result<i64, InstallError> {
        let user_id = request.user_id.clone();
        let state = Self::initial_state_from_options(request.options.as_ref())?;
        let data = AppInstallTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request,
            state,
        };
        let task_name = format!("Install app ({app_hint})");
        let task_id = self
            .store
            .create_install_task(
                task_name.as_str(),
                TASK_DATA_TYPE_APP_INSTALL,
                serde_json::to_value(&data)
                    .map_err(|err| internal_error(InstallStage::Resolve, err.to_string()))?,
                user_id.as_str(),
                app_hint,
            )
            .await?;
        Ok(task_id)
    }

    /// 创建升级事务任务（同一 Stage 流水线，task_type = app.update）。
    pub async fn create_update_task(
        &self,
        request: buckyos_api::AppUpdateTaskRequest,
        app_hint: &str,
    ) -> Result<i64, InstallError> {
        let user_id = request.user_id.clone();
        let state = Self::initial_state_from_options(request.options.as_ref())?;
        let data = buckyos_api::AppUpdateTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request,
            state,
        };
        let task_name = format!("Update app ({app_hint})");
        let task_id = self
            .store
            .create_install_task(
                task_name.as_str(),
                TASK_DATA_TYPE_APP_UPDATE,
                serde_json::to_value(&data)
                    .map_err(|err| internal_error(InstallStage::Resolve, err.to_string()))?,
                user_id.as_str(),
                app_hint,
            )
            .await?;
        Ok(task_id)
    }

    // -------------------- 状态机推进 --------------------

    pub async fn run_task(&self, task_id: i64) -> Result<RunOutcome, InstallError> {
        // 进程内执行守卫。
        {
            let mut running = self.running.lock().unwrap();
            if !running.insert(task_id) {
                return Ok(RunOutcome::NotRun);
            }
        }
        let _guard = RunGuard {
            engine: self,
            task_id,
        };

        let view = self.store.load(task_id).await?;
        if view.status.is_terminal() {
            return Ok(RunOutcome::NotRun);
        }
        if matches!(
            view.status,
            TaskStatus::WaitingForApproval | TaskStatus::Paused
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
        let task_id = view.id;
        let mut data = self.parse_data(&view)?;

        if data.state.stage.is_none() {
            data.state.stage = Some(InstallStage::Resolve);
        }
        let _ = self
            .store
            .set_status(task_id, TaskStatus::Running, None, None)
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
                    task_id,
                    TaskStatus::Running,
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
                    self.persist(task_id, &data).await?;

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
                    let Some(plan) = self.driver.inspect(&view, &data).await? else {
                        let error = InstallError::new(
                            InstallStage::Resolve,
                            InstallErrorCode::TrustResolutionRequired,
                            true,
                            "DID resolution did not return an installable App Document",
                        )
                        .with_action(InstallUserAction::ConnectNetwork);
                        data.state.last_error = Some(error);
                        self.persist(task_id, &data).await?;
                        let _ = self
                            .store
                            .set_status(
                                task_id,
                                TaskStatus::Paused,
                                None,
                                Some("waiting for trust resolution".to_string()),
                            )
                            .await;
                        return Ok(RunOutcome::Waiting);
                    };
                    data.state.plan = Some(plan);
                    data.state.mark_stage_completed(InstallStage::Inspect);
                    self.persist(task_id, &data).await?;

                    let plan = data.state.plan.as_ref().unwrap();
                    match plan.readiness.install {
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
                            self.persist(task_id, &data).await?;
                            let _ = self
                                .store
                                .set_status(
                                    task_id,
                                    TaskStatus::Paused,
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
                    match self.approval_gate(task_id, &mut data).await? {
                        ApprovalGate::Approved => {}
                        ApprovalGate::Waiting => return Ok(RunOutcome::WaitingForApproval),
                    }

                    let updated_plan = self.driver.acquire(&view, &data).await?;
                    data.state.plan = Some(updated_plan);
                    data.state.mark_stage_completed(InstallStage::Acquire);
                    self.persist(task_id, &data).await?;
                }
                InstallStage::Verify => {
                    let report = self.driver.verify(&view, &data).await?;
                    let passed = report.passed;
                    data.state.verification = Some(report);
                    if !passed {
                        self.persist(task_id, &data).await?;
                        return Err(InstallError::new(
                            InstallStage::Verify,
                            InstallErrorCode::VerificationFailed,
                            true,
                            "verification report has failed checks",
                        ));
                    }
                    // Trust + Content + Config 全 ready 才进入 Prepare。
                    let plan = data
                        .state
                        .plan
                        .as_ref()
                        .ok_or_else(|| internal_error(InstallStage::Verify, "plan missing"))?;
                    if !plan.readiness.install.is_ready() {
                        self.persist(task_id, &data).await?;
                        return Err(InstallError::new(
                            InstallStage::Verify,
                            InstallErrorCode::VerificationFailed,
                            true,
                            format!(
                                "plan readiness is {:?} after acquisition; refusing to prepare",
                                plan.readiness.install
                            ),
                        ));
                    }
                    data.state.mark_stage_completed(InstallStage::Verify);
                    self.persist(task_id, &data).await?;
                }
                InstallStage::Prepare => {
                    let prepared = self.driver.prepare(&view, &data).await?;
                    data.state.prepared = Some(prepared);
                    data.state.mark_stage_completed(InstallStage::Prepare);
                    self.persist(task_id, &data).await?;
                }
                InstallStage::Deploy => {
                    self.driver.deploy(&view, &data).await?;
                    data.state.mark_stage_completed(InstallStage::Deploy);
                    self.persist(task_id, &data).await?;
                }
                InstallStage::Activate => {
                    let activate_result = self.driver.activate(&view, &data).await;
                    let result = match activate_result {
                        Ok(result) => result,
                        Err(error) => {
                            // 升级：新版本 Activate 失败恢复旧 spec/运行状态
                            //（本机部署回滚，不改变 resolver 状态，P4.4）。
                            let has_previous = data
                                .state
                                .prepared
                                .as_ref()
                                .map(|prepared| prepared.previous_spec.is_some())
                                .unwrap_or(false);
                            if view.task_type == TASK_DATA_TYPE_APP_UPDATE && has_previous {
                                data.state.last_error = Some(error.clone());
                                self.persist(task_id, &data).await?;
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
                                    self.persist(task_id, &data).await?;
                                }
                            }
                            return Err(error);
                        }
                    };
                    data.state.result = Some(result);
                    data.state.mark_stage_completed(InstallStage::Activate);
                    self.persist(task_id, &data).await?;
                    let _ = self
                        .store
                        .set_status(
                            task_id,
                            TaskStatus::Completed,
                            Some(100.0),
                            Some("App installed".to_string()),
                        )
                        .await;
                    info!("install task {task_id} completed");
                    return Ok(RunOutcome::Completed);
                }
            }

            // 重新加载视图（供 driver 观察最新 message/status；数据以本地为准）。
            view = self.store.load(task_id).await?;
        }
    }

    // -------------------- 确认 / 重试 / 取消 --------------------

    /// 用户确认：绑定当前 plan fingerprint；target/params 变化时重算 plan
    /// 后绑定新 fingerprint（禁止在旧 plan 上 patch）。
    pub async fn confirm(
        &self,
        task_id: i64,
        approved_by: &str,
        approver_is_admin: bool,
        target: Option<InstallTarget>,
        install_params: Option<InstallParams>,
    ) -> Result<(), InstallError> {
        let view = self.store.load(task_id).await?;
        self.ensure_task_owner(&view, approved_by, approver_is_admin)?;
        if view.status != TaskStatus::WaitingForApproval {
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
        let current_plan = data.state.plan.clone().ok_or_else(|| {
            internal_error(InstallStage::Inspect, "waiting task has no persisted plan")
        })?;

        let target_changed = target
            .as_ref()
            .map(|value| *value != current_plan.target)
            .unwrap_or(false);
        let params_changed = install_params
            .as_ref()
            .map(|value| *value != current_plan.install_params)
            .unwrap_or(false);

        let plan = if target_changed || params_changed {
            // 参数变化：从 Inspect 失效并用新参数重算 plan。
            if let Some(target) = target.clone() {
                data.state.requested_target = Some(target);
            }
            if let Some(params) = install_params.clone() {
                data.state.requested_params = Some(params);
            }
            data.state.invalidate_from(InstallStage::Inspect);
            let plan = self.driver.inspect(&view, &data).await?.ok_or_else(|| {
                InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::TrustResolutionRequired,
                    true,
                    "DID resolution did not return an installable App Document",
                )
            })?;
            data.state.plan = Some(plan.clone());
            data.state.mark_stage_completed(InstallStage::Inspect);
            plan
        } else {
            current_plan
        };

        if !plan.readiness.target.is_ready() {
            data.state.plan = Some(plan.clone());
            data.state.approval = None;
            self.persist(task_id, &data).await?;
            return Err(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::UnsupportedTarget,
                false,
                plan.target_issues.join("; "),
            )
            .with_action(InstallUserAction::ChangeTarget));
        }
        if !plan.readiness.config.is_ready() {
            data.state.plan = Some(plan.clone());
            data.state.approval = None;
            self.persist(task_id, &data).await?;
            return Err(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::ConfigBlocked,
                false,
                plan.config_issues.join("; "),
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
        self.persist(task_id, &data).await?;
        self.store
            .set_status(
                task_id,
                TaskStatus::Running,
                None,
                Some("approved, continuing".to_string()),
            )
            .await?;
        Ok(())
    }

    /// 重试：从失败 Stage 失效重算。只清除失效输出，不重做已完成 Stage。
    pub async fn retry(
        &self,
        task_id: i64,
        requested_by: &str,
        requester_is_admin: bool,
    ) -> Result<(), InstallError> {
        let view = self.store.load(task_id).await?;
        self.ensure_task_owner(&view, requested_by, requester_is_admin)?;
        if !matches!(view.status, TaskStatus::Failed | TaskStatus::Paused) {
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
        self.persist(task_id, &data).await?;
        self.store
            .set_status(
                task_id,
                TaskStatus::Running,
                None,
                Some(format!("retrying from stage {retry_stage}")),
            )
            .await?;
        Ok(())
    }

    /// 取消：Prepare 前直接取消；spec 已写（Deploy 开始）后先回滚再取消。
    pub async fn cancel(
        &self,
        task_id: i64,
        requested_by: &str,
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
        self.persist(task_id, &data).await?;
        self.store
            .set_status(
                task_id,
                TaskStatus::Canceled,
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

    async fn persist(&self, task_id: i64, data: &AppInstallTaskData) -> Result<(), InstallError> {
        self.store.write_data(task_id, data.to_full_patch()).await
    }

    async fn record_failure(&self, task_id: i64, error: &InstallError) {
        // 尽力持久化结构化错误；失败也要把任务状态标出去。
        if let Ok(view) = self.store.load(task_id).await {
            if let Ok(mut data) = self.parse_data(&view) {
                data.state.last_error = Some(error.clone());
                let _ = self.persist(task_id, &data).await;
            }
        }
        let target_status = if error.code == InstallErrorCode::Canceled {
            TaskStatus::Canceled
        } else {
            TaskStatus::Failed
        };
        let _ = self
            .store
            .set_status(task_id, target_status, None, Some(error.to_string()))
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
        task_id: i64,
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
            self.persist(task_id, data).await?;
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
            if !plan.readiness.config.is_ready() {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::ConfigBlocked,
                    false,
                    plan.config_issues.join("; "),
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
            self.persist(task_id, data).await?;
            return Ok(ApprovalGate::Approved);
        }

        self.store
            .set_status(
                task_id,
                TaskStatus::WaitingForApproval,
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

    fn view_from_task(task: buckyos_api::Task) -> InstallTaskView {
        InstallTaskView {
            id: task.id,
            root_id: task.root_id,
            task_type: task.task_type,
            status: task.status,
            user_id: task.user_id,
            app_id: task.app_id,
            data: task.data,
        }
    }
}

#[async_trait]
impl InstallTaskStore for TaskMgrInstallStore {
    async fn create_install_task(
        &self,
        name: &str,
        task_type: &str,
        data: Value,
        user_id: &str,
        app_id: &str,
    ) -> Result<i64, InstallError> {
        let client = Self::client().await?;
        // control_panel is a zone-trusted service: the task is recorded under
        // the business user it already authenticated at the RPC entry.
        let task = client
            .create_task(name, task_type, Some(data), user_id, app_id, None)
            .await
            .map_err(|err| Self::map_err("create task", err))?;
        Ok(task.id)
    }

    async fn load(&self, task_id: i64) -> Result<InstallTaskView, InstallError> {
        let client = Self::client().await?;
        let task = client
            .get_task(task_id)
            .await
            .map_err(|err| Self::map_err("get task", err))?;
        Ok(Self::view_from_task(task))
    }

    async fn write_data(&self, task_id: i64, full_patch: Value) -> Result<(), InstallError> {
        let client = Self::client().await?;
        client
            .update_task_data(task_id, full_patch)
            .await
            .map_err(|err| Self::map_err("update task data", err))?;
        Ok(())
    }

    async fn set_status(
        &self,
        task_id: i64,
        status: TaskStatus,
        progress: Option<f32>,
        message: Option<String>,
    ) -> Result<(), InstallError> {
        let client = Self::client().await?;
        client
            .update_task(task_id, Some(status), progress, message, None)
            .await
            .map_err(|err| Self::map_err("update task", err))?;
        Ok(())
    }

    async fn list_active(&self) -> Result<Vec<InstallTaskView>, InstallError> {
        let client = Self::client().await?;
        let mut views = Vec::new();
        // 只扫本 Service 职责域内的 task_type（app.install / app.update）。
        // 这些 Task 都由 control_panel 自己的业务接口创建；TaskFilter 一次
        // 只能过滤一个 task_type，本地合并非终态。
        for task_type in [TASK_DATA_TYPE_APP_INSTALL, TASK_DATA_TYPE_APP_UPDATE] {
            let tasks = client
                .list_tasks(Some(buckyos_api::TaskFilter {
                    task_type: Some(task_type.to_string()),
                    ..Default::default()
                }))
                .await
                .map_err(|err| Self::map_err("list tasks", err))?;
            for task in tasks {
                if task.status.is_terminal() {
                    continue;
                }
                views.push(Self::view_from_task(task));
            }
        }
        Ok(views)
    }
}

// ---------------------------------------------------------------------------
// 单元测试：fake store + fake driver 跑全 Stage
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_install_resolver::fake as resolver_fake;
    use crate::app_install_resolver::AppDidResolver;
    use buckyos_api::{
        AppDoc, AppDocumentRef, AppInstallTaskRequest, AppServiceSpec, AppType, DocumentStatus,
        InstallPolicy, InstallSource, PlanReadiness, ReadinessState, ServiceSpecConfig,
        ServiceState, SubPkgDesc, VerificationCheck, VerificationReport,
        APP_INSTALL_SCHEMA_VERSION,
    };
    use name_lib::DID;
    use serde_json::json;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Mutex as StdMutex;

    // ---------------- fake store ----------------

    #[derive(Default)]
    struct MemStoreInner {
        next_id: i64,
        tasks: HashMap<i64, InstallTaskView>,
    }

    struct MemStore {
        inner: StdMutex<MemStoreInner>,
    }

    impl MemStore {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                inner: StdMutex::new(MemStoreInner {
                    next_id: 1,
                    tasks: HashMap::new(),
                }),
            })
        }

        fn status_of(&self, task_id: i64) -> TaskStatus {
            self.inner.lock().unwrap().tasks[&task_id].status
        }

        fn data_of(&self, task_id: i64) -> Value {
            self.inner.lock().unwrap().tasks[&task_id].data.clone()
        }
    }

    /// 复刻 task_db 的 deep-merge 语义（对象递归、null 删键、数组整替）。
    fn merge_json(target: &mut Value, patch: &Value) {
        match (target, patch) {
            (Value::Object(target_map), Value::Object(patch_map)) => {
                for (key, value) in patch_map {
                    if value.is_null() {
                        target_map.remove(key);
                    } else if let Some(existing) = target_map.get_mut(key) {
                        merge_json(existing, value);
                    } else {
                        target_map.insert(key.clone(), value.clone());
                    }
                }
            }
            (target_value, patch_value) => {
                *target_value = patch_value.clone();
            }
        }
    }

    #[async_trait]
    impl InstallTaskStore for MemStore {
        async fn create_install_task(
            &self,
            _name: &str,
            task_type: &str,
            data: Value,
            user_id: &str,
            app_id: &str,
        ) -> Result<i64, InstallError> {
            let mut inner = self.inner.lock().unwrap();
            let id = inner.next_id;
            inner.next_id += 1;
            inner.tasks.insert(
                id,
                InstallTaskView {
                    id,
                    root_id: id.to_string(),
                    task_type: task_type.to_string(),
                    status: TaskStatus::Pending,
                    user_id: user_id.to_string(),
                    app_id: app_id.to_string(),
                    data,
                },
            );
            Ok(id)
        }

        async fn load(&self, task_id: i64) -> Result<InstallTaskView, InstallError> {
            self.inner
                .lock()
                .unwrap()
                .tasks
                .get(&task_id)
                .cloned()
                .ok_or_else(|| internal_error(InstallStage::Resolve, "task not found"))
        }

        async fn write_data(&self, task_id: i64, full_patch: Value) -> Result<(), InstallError> {
            let mut inner = self.inner.lock().unwrap();
            let task = inner
                .tasks
                .get_mut(&task_id)
                .ok_or_else(|| internal_error(InstallStage::Resolve, "task not found"))?;
            merge_json(&mut task.data, &full_patch);
            Ok(())
        }

        async fn set_status(
            &self,
            task_id: i64,
            status: TaskStatus,
            _progress: Option<f32>,
            _message: Option<String>,
        ) -> Result<(), InstallError> {
            let mut inner = self.inner.lock().unwrap();
            let task = inner
                .tasks
                .get_mut(&task_id)
                .ok_or_else(|| internal_error(InstallStage::Resolve, "task not found"))?;
            task.status = status;
            Ok(())
        }

        async fn list_active(&self) -> Result<Vec<InstallTaskView>, InstallError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .tasks
                .values()
                .filter(|task| !task.status.is_terminal())
                .cloned()
                .collect())
        }
    }

    // ---------------- fake driver ----------------

    struct FakeDriver {
        resolver: resolver_fake::FakeAppResolver,
        counts: StdMutex<HashMap<&'static str, u32>>,
        fail_acquire_once: AtomicBool,
        fail_activate_once: AtomicBool,
        rollback_called: AtomicU32,
    }

    impl FakeDriver {
        fn new() -> Self {
            Self {
                resolver: resolver_fake::FakeAppResolver::new(),
                counts: StdMutex::new(HashMap::new()),
                fail_acquire_once: AtomicBool::new(false),
                fail_activate_once: AtomicBool::new(false),
                rollback_called: AtomicU32::new(0),
            }
        }

        fn bump(&self, key: &'static str) {
            *self.counts.lock().unwrap().entry(key).or_insert(0) += 1;
        }

        fn count(&self, key: &'static str) -> u32 {
            self.counts.lock().unwrap().get(key).copied().unwrap_or(0)
        }

        fn app_did_of(data: &AppInstallTaskData) -> DID {
            match &data.request.source {
                InstallSource::Identifier { identifier, .. } => DID::from_str(identifier).unwrap(),
                InstallSource::LocalPikg { .. } => panic!("test uses identifier source"),
            }
        }
    }

    fn ready_plan_from(data: &AppInstallTaskData, content_ready: bool) -> InstallPlan {
        let snapshot = data.state.resolution.clone().unwrap();
        let doc_value = data.state.resolved_app_doc.clone().unwrap();
        let app_doc: AppDoc = serde_json::from_value(doc_value).unwrap();
        let target = data
            .state
            .requested_target
            .clone()
            .unwrap_or_else(|| InstallTarget {
                node_did: None,
                node_id: Some("ood1".to_string()),
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
                kernel_version: None,
                runtime_version: None,
                capabilities: BTreeMap::new(),
            });
        let params = data.state.requested_params.clone().unwrap_or_default();
        let readiness = PlanReadiness::compose(
            ReadinessState::Ready,
            ReadinessState::from_bool(snapshot.is_trust_ready(data.request.policy)),
            ReadinessState::Ready,
            ReadinessState::from_bool(content_ready),
            ReadinessState::Ready,
            ReadinessState::Ready,
            snapshot.document_status,
        );
        let app = AppDocumentRef {
            did: snapshot.app_did.clone(),
            object_id: snapshot.app_doc_object_id.clone().unwrap(),
            name: app_doc.name.clone(),
            version: app_doc.version.clone(),
        };
        let service_spec_config = ServiceSpecConfig::default();
        let fingerprint = InstallPlan::compute_fingerprint(
            &app,
            &snapshot,
            &target,
            &params,
            &service_spec_config,
            &[],
        );
        InstallPlan {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            app,
            resolution: snapshot,
            target,
            selected_packages: vec![],
            required_contents: vec![],
            readiness,
            target_issues: vec![],
            config_issues: vec![],
            permission_options: app_doc.permissions.clone(),
            install_params: params,
            service_spec_config,
            estimated_download_bytes: 0,
            plan_fingerprint: fingerprint,
            created_at: 0,
        }
    }

    #[async_trait]
    impl InstallStageDriver for FakeDriver {
        async fn resolve(
            &self,
            _view: &InstallTaskView,
            data: &AppInstallTaskData,
        ) -> Result<ResolveOutcome, InstallError> {
            self.bump("resolve");
            let app_did = Self::app_did_of(data);
            let resolved = self
                .resolver
                .resolve_app(&app_did, data.request.policy)
                .await?;
            Ok(ResolveOutcome {
                candidate: None,
                resolution: resolved.snapshot,
                resolved_app_doc: resolved.document_value,
            })
        }

        async fn inspect(
            &self,
            _view: &InstallTaskView,
            data: &AppInstallTaskData,
        ) -> Result<Option<InstallPlan>, InstallError> {
            self.bump("inspect");
            if data.state.resolved_app_doc.is_none() {
                return Ok(None);
            }
            Ok(Some(ready_plan_from(data, false)))
        }

        async fn acquire(
            &self,
            _view: &InstallTaskView,
            data: &AppInstallTaskData,
        ) -> Result<InstallPlan, InstallError> {
            self.bump("acquire");
            if self.fail_acquire_once.swap(false, Ordering::SeqCst) {
                return Err(InstallError::new(
                    InstallStage::Acquire,
                    InstallErrorCode::AcquisitionFailed,
                    true,
                    "simulated source failure",
                ));
            }
            Ok(ready_plan_from(data, true))
        }

        async fn verify(
            &self,
            _view: &InstallTaskView,
            _data: &AppInstallTaskData,
        ) -> Result<VerificationReport, InstallError> {
            self.bump("verify");
            Ok(VerificationReport::from_checks(
                vec![VerificationCheck::pass("appdoc", "doc_identity")],
                1,
            ))
        }

        async fn prepare(
            &self,
            _view: &InstallTaskView,
            data: &AppInstallTaskData,
        ) -> Result<PreparedDeployment, InstallError> {
            self.bump("prepare");
            let doc_value = data.state.resolved_app_doc.clone().unwrap();
            let app_doc: AppDoc = serde_json::from_value(doc_value).unwrap();
            let spec = AppServiceSpec {
                permission: app_doc.permissions.clone(),
                app_doc,
                app_index: 7,
                user_id: data.request.user_id.clone(),
                enable: true,
                expected_instance_count: 1,
                state: ServiceState::New,
                spec_config: Default::default(),
            };
            let previous_spec = if _view.task_type == TASK_DATA_TYPE_APP_UPDATE {
                let mut old = spec.clone();
                old.app_index = 6;
                Some(old)
            } else {
                None
            };
            Ok(PreparedDeployment {
                spec_path: format!(
                    "users/{}/apps/{}/spec",
                    data.request.user_id, spec.app_doc.name
                ),
                service_spec_id: format!("{}@{}", spec.app_doc.name, data.request.user_id),
                app_index: 7,
                previous_spec,
                new_spec: spec,
                materialized_objects: vec![],
                prepared_at: 1,
            })
        }

        async fn deploy(
            &self,
            _view: &InstallTaskView,
            _data: &AppInstallTaskData,
        ) -> Result<(), InstallError> {
            self.bump("deploy");
            Ok(())
        }

        async fn activate(
            &self,
            _view: &InstallTaskView,
            _data: &AppInstallTaskData,
        ) -> Result<ActivateOutcome, InstallError> {
            self.bump("activate");
            if self.fail_activate_once.swap(false, Ordering::SeqCst) {
                return Err(InstallError::new(
                    InstallStage::Activate,
                    InstallErrorCode::ActivationFailed,
                    true,
                    "simulated activation failure",
                ));
            }
            Ok(InstallTaskResult {
                install_record_key: Some("users/alice/apps/demo/install_record".to_string()),
                proof_id: Some("proof-1".to_string()),
                instance_node_id: Some("ood1".to_string()),
                completed_at: Some(2),
            })
        }

        async fn rollback_deploy(
            &self,
            _view: &InstallTaskView,
            _data: &AppInstallTaskData,
        ) -> Result<(), InstallError> {
            self.rollback_called.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // ---------------- helpers ----------------

    fn demo_doc_value(app_did: &DID) -> Value {
        let owner = DID::from_str("did:bns:tester").unwrap();
        let doc = AppDoc::builder(AppType::Web, "demo_web", "0.1.0", "tester", &owner)
            .app_did(app_did.clone())
            .web_pkg(SubPkgDesc::new("demo_web-web#0.1.0"))
            .build()
            .unwrap();
        serde_json::to_value(&doc).unwrap()
    }

    fn install_request(app_did: &DID, policy: InstallPolicy, auto: bool) -> AppInstallTaskRequest {
        AppInstallTaskRequest {
            source: InstallSource::identifier(app_did.to_string(), None),
            user_id: "alice".to_string(),
            policy,
            options: if auto {
                Some(json!({"auto_confirm": true}))
            } else {
                None
            },
        }
    }

    fn setup(
        policy: InstallPolicy,
        auto: bool,
    ) -> (Arc<MemStore>, Arc<FakeDriver>, InstallEngine, DID) {
        let store = MemStore::new();
        let driver = Arc::new(FakeDriver::new());
        let app_did = DID::from_str("did:bns:demo_web.tester").unwrap();
        driver.resolver.set_answer(
            &app_did,
            resolver_fake::active_answer(&app_did, demo_doc_value(&app_did), 1),
        );
        let engine = InstallEngine::new(store.clone(), driver.clone());
        let _ = policy;
        let _ = auto;
        (store, driver, engine, app_did)
    }

    async fn create_task(
        engine: &InstallEngine,
        app_did: &DID,
        policy: InstallPolicy,
        auto: bool,
    ) -> i64 {
        engine
            .create_install_task(install_request(app_did, policy, auto), "demo_web")
            .await
            .unwrap()
    }

    #[test]
    fn initial_options_load_typed_install_params() {
        let options = json!({
            "install_params": {
                "permissions": [{
                    "scope_path": "user/home",
                    "required": true,
                    "actions": ["read"],
                    "exp": null
                }],
                "auto_start": false
            }
        });
        let state = InstallEngine::initial_state_from_options(Some(&options)).unwrap();
        let params = state.requested_params.unwrap();
        assert_eq!(params.permissions[0].scope_path, "user/home");
        assert!(!params.auto_start);

        let invalid = json!({"install_params": {"accepted_permissions": ["user/home"]}});
        let error = InstallEngine::initial_state_from_options(Some(&invalid)).unwrap_err();
        assert_eq!(error.code, InstallErrorCode::InvalidRequest);
    }

    // ---------------- tests ----------------

    #[tokio::test]
    async fn full_pipeline_with_approval_gate() {
        let (store, driver, engine, app_did) = setup(InstallPolicy::Normal, false);
        let task_id = create_task(&engine, &app_did, InstallPolicy::Normal, false).await;

        // 第一轮：跑到 Inspect 后等待确认。
        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::WaitingForApproval);
        assert_eq!(store.status_of(task_id), TaskStatus::WaitingForApproval);
        assert_eq!(driver.count("resolve"), 1);
        assert_eq!(driver.count("inspect"), 1);
        assert_eq!(driver.count("acquire"), 0);
        // 尚未部署，绝无 result/proof。
        let data: AppInstallTaskData = serde_json::from_value(store.data_of(task_id)).unwrap();
        assert!(data.state.result.is_none());

        // 确认后继续到完成。
        engine
            .confirm(task_id, "alice", false, None, None)
            .await
            .unwrap();
        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::Completed);
        assert_eq!(store.status_of(task_id), TaskStatus::Completed);

        // 每个 Stage 恰好执行一次。
        for key in [
            "resolve", "inspect", "acquire", "verify", "prepare", "deploy", "activate",
        ] {
            assert_eq!(driver.count(key), 1, "stage {key} should run exactly once");
        }
        let data: AppInstallTaskData = serde_json::from_value(store.data_of(task_id)).unwrap();
        assert_eq!(
            data.state.result.as_ref().unwrap().proof_id.as_deref(),
            Some("proof-1")
        );
        assert!(data.state.is_stage_completed(InstallStage::Activate));
    }

    #[tokio::test]
    async fn restart_recovery_resumes_from_persisted_state() {
        let (store, driver, engine, app_did) = setup(InstallPolicy::Normal, false);
        let task_id = create_task(&engine, &app_did, InstallPolicy::Normal, false).await;
        let _ = engine.run_task(task_id).await.unwrap();
        assert_eq!(store.status_of(task_id), TaskStatus::WaitingForApproval);
        drop(engine);

        // "重启"：同一持久 store，新引擎实例。
        let engine2 = InstallEngine::new(store.clone(), driver.clone());
        engine2
            .confirm(task_id, "alice", false, None, None)
            .await
            .unwrap();
        let outcome = engine2.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::Completed);
        // Resolve/Inspect 没有重跑。
        assert_eq!(driver.count("resolve"), 1);
        assert_eq!(driver.count("inspect"), 1);
    }

    #[tokio::test]
    async fn retry_reruns_only_from_failed_stage() {
        let (store, driver, engine, app_did) = setup(InstallPolicy::SystemInternal, true);
        let task_id = create_task(&engine, &app_did, InstallPolicy::SystemInternal, true).await;
        driver.fail_acquire_once.store(true, Ordering::SeqCst);

        let err = engine.run_task(task_id).await.unwrap_err();
        assert_eq!(err.code, InstallErrorCode::AcquisitionFailed);
        assert_eq!(store.status_of(task_id), TaskStatus::Failed);
        let data: AppInstallTaskData = serde_json::from_value(store.data_of(task_id)).unwrap();
        assert_eq!(
            data.state.last_error.as_ref().unwrap().code,
            InstallErrorCode::AcquisitionFailed
        );

        engine.retry(task_id, "alice", false).await.unwrap();
        // retry 清空 last_error（deep merge null 删除）。
        let data: AppInstallTaskData = serde_json::from_value(store.data_of(task_id)).unwrap();
        assert!(data.state.last_error.is_none());

        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::Completed);
        // Resolve/Inspect 各一次；Acquire 失败一次 + 成功一次。
        assert_eq!(driver.count("resolve"), 1);
        assert_eq!(driver.count("inspect"), 1);
        assert_eq!(driver.count("acquire"), 2);
        assert_eq!(driver.count("deploy"), 1);
    }

    #[tokio::test]
    async fn auto_confirm_only_for_system_internal() {
        // SYSTEM_INTERNAL + auto_confirm：不停在确认。
        let (store, _driver, engine, app_did) = setup(InstallPolicy::SystemInternal, true);
        let task_id = create_task(&engine, &app_did, InstallPolicy::SystemInternal, true).await;
        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::Completed);
        assert_eq!(store.status_of(task_id), TaskStatus::Completed);

        // NORMAL + auto_confirm 请求：仍必须等待确认。
        let (store, _driver, engine, app_did) = setup(InstallPolicy::Normal, true);
        let task_id = create_task(&engine, &app_did, InstallPolicy::Normal, true).await;
        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::WaitingForApproval);
        assert_eq!(store.status_of(task_id), TaskStatus::WaitingForApproval);
    }

    #[tokio::test]
    async fn revoked_identity_fails_non_retryable() {
        let (store, driver, engine, _) = setup(InstallPolicy::Normal, false);
        let revoked_did = DID::from_str("did:bns:revoked.tester").unwrap();
        driver.resolver.set_answer(
            &revoked_did,
            resolver_fake::status_answer(&revoked_did, DocumentStatus::Revoked),
        );
        let task_id = create_task(&engine, &revoked_did, InstallPolicy::Normal, false).await;

        let err = engine.run_task(task_id).await.unwrap_err();
        assert_eq!(err.code, InstallErrorCode::IdentityRevoked);
        assert!(!err.retryable);
        assert_eq!(store.status_of(task_id), TaskStatus::Failed);

        // 不可重试错误拒绝 retry。
        let err = engine.retry(task_id, "alice", false).await.unwrap_err();
        assert_eq!(err.code, InstallErrorCode::IdentityRevoked);
    }

    #[tokio::test]
    async fn unknown_trust_pauses_then_retry_after_resolution() {
        let (store, driver, engine, _) = setup(InstallPolicy::Normal, false);
        let unknown_did = DID::from_str("did:bns:unknown_app.tester").unwrap();
        // 不设置回答 → fake 返回 Unknown。
        let task_id = create_task(&engine, &unknown_did, InstallPolicy::Normal, false).await;

        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::Waiting);
        assert_eq!(store.status_of(task_id), TaskStatus::Paused);
        let data: AppInstallTaskData = serde_json::from_value(store.data_of(task_id)).unwrap();
        assert_eq!(
            data.state.last_error.as_ref().unwrap().code,
            InstallErrorCode::TrustResolutionRequired
        );

        // 权威恢复后 retry → 从 Resolve 重算并等待确认。
        driver.resolver.set_answer(
            &unknown_did,
            resolver_fake::active_answer(&unknown_did, demo_doc_value(&unknown_did), 3),
        );
        engine.retry(task_id, "alice", false).await.unwrap();
        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::WaitingForApproval);
    }

    #[tokio::test]
    async fn cancel_boundaries() {
        // Prepare 前取消：直接 Canceled，无回滚。
        let (store, driver, engine, app_did) = setup(InstallPolicy::Normal, false);
        let task_id = create_task(&engine, &app_did, InstallPolicy::Normal, false).await;
        let _ = engine.run_task(task_id).await.unwrap(); // waiting approval
        engine.cancel(task_id, "alice", false).await.unwrap();
        assert_eq!(store.status_of(task_id), TaskStatus::Canceled);
        assert_eq!(driver.rollback_called.load(Ordering::SeqCst), 0);

        // Deploy 后取消：必须回滚。
        let (store, driver, engine, app_did) = setup(InstallPolicy::SystemInternal, true);
        let task_id = create_task(&engine, &app_did, InstallPolicy::SystemInternal, true).await;
        let _ = engine.run_task(task_id).await.unwrap(); // completed
                                                         // 完成后取消被拒绝（终态）。
        assert!(engine.cancel(task_id, "alice", false).await.is_err());

        // 构造 Deploy 完成但 Activate 未跑的中间态：手工推系统状态。
        let task_id2 = create_task(&engine, &app_did, InstallPolicy::SystemInternal, true).await;
        // 先跑一轮到完成前……直接注入持久状态模拟"Deploy 完成后进程死亡"。
        {
            let view = store.load(task_id2).await.unwrap();
            let mut data: AppInstallTaskData = serde_json::from_value(view.data).unwrap();
            data.state.stage = Some(InstallStage::Activate);
            data.state.completed_stages = vec![
                InstallStage::Resolve,
                InstallStage::Inspect,
                InstallStage::Acquire,
                InstallStage::Verify,
                InstallStage::Prepare,
                InstallStage::Deploy,
            ];
            store
                .write_data(task_id2, data.to_full_patch())
                .await
                .unwrap();
        }
        engine.cancel(task_id2, "alice", false).await.unwrap();
        assert_eq!(store.status_of(task_id2), TaskStatus::Canceled);
        assert_eq!(driver.rollback_called.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_same_app_conflicts() {
        let (store, _driver, engine, app_did) = setup(InstallPolicy::Normal, false);
        let first = create_task(&engine, &app_did, InstallPolicy::Normal, false).await;
        let second = create_task(&engine, &app_did, InstallPolicy::Normal, false).await;

        let _ = engine.run_task(first).await.unwrap(); // waiting approval（非终态）
        let err = engine.run_task(second).await.unwrap_err();
        assert_eq!(err.code, InstallErrorCode::Conflict);
        assert_eq!(store.status_of(second), TaskStatus::Failed);
        // 第一个事务不受影响。
        assert_eq!(store.status_of(first), TaskStatus::WaitingForApproval);
    }

    #[tokio::test]
    async fn confirm_scope_and_double_confirm() {
        let (_store, _driver, engine, app_did) = setup(InstallPolicy::Normal, false);
        let task_id = create_task(&engine, &app_did, InstallPolicy::Normal, false).await;
        let _ = engine.run_task(task_id).await.unwrap();

        // 他人不能确认。
        let err = engine
            .confirm(task_id, "mallory", false, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, InstallErrorCode::InvalidRequest);

        engine
            .confirm(task_id, "alice", false, None, None)
            .await
            .unwrap();
        // 二次确认：任务已不在 WaitingForApproval。
        let err = engine
            .confirm(task_id, "alice", false, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, InstallErrorCode::Conflict);
    }

    #[tokio::test]
    async fn update_activation_failure_rolls_back_and_retry_redeploys() {
        let (store, driver, engine, app_did) = setup(InstallPolicy::SystemInternal, true);
        let request = buckyos_api::AppUpdateTaskRequest {
            source: InstallSource::identifier(app_did.to_string(), None),
            user_id: "alice".to_string(),
            app_id: "demo_web".to_string(),
            policy: InstallPolicy::SystemInternal,
            options: Some(json!({"auto_confirm": true})),
        };
        let task_id = engine
            .create_update_task(request, "demo_web")
            .await
            .unwrap();
        driver.fail_activate_once.store(true, Ordering::SeqCst);

        let err = engine.run_task(task_id).await.unwrap_err();
        assert_eq!(err.code, InstallErrorCode::ActivationFailed);
        // 升级失败自动回滚（旧 spec 恢复）。
        assert_eq!(driver.rollback_called.load(Ordering::SeqCst), 1);
        assert_eq!(store.status_of(task_id), TaskStatus::Failed);
        // 回滚后 Deploy 输出失效。
        let data: AppInstallTaskData = serde_json::from_value(store.data_of(task_id)).unwrap();
        assert!(!data.state.is_stage_completed(InstallStage::Deploy));
        // 失败前没有 result/proof。
        assert!(data.state.result.is_none());

        // retry：重写 spec（deploy 第二次）后 Activate 成功。
        engine.retry(task_id, "alice", false).await.unwrap();
        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::Completed);
        assert_eq!(driver.count("deploy"), 2);
        assert_eq!(driver.count("activate"), 2);
        // Prepare 未重跑（回滚材料仍有效）。
        assert_eq!(driver.count("prepare"), 1);
    }

    #[tokio::test]
    async fn confirm_with_new_target_recomputes_plan() {
        let (store, driver, engine, app_did) = setup(InstallPolicy::Normal, false);
        let task_id = create_task(&engine, &app_did, InstallPolicy::Normal, false).await;
        let _ = engine.run_task(task_id).await.unwrap();

        let old_data: AppInstallTaskData = serde_json::from_value(store.data_of(task_id)).unwrap();
        let old_fp = old_data
            .state
            .plan
            .as_ref()
            .unwrap()
            .plan_fingerprint
            .clone();

        let new_target = InstallTarget {
            node_did: None,
            node_id: Some("ood2".to_string()),
            os: "linux".to_string(),
            arch: "aarch64".to_string(),
            kernel_version: None,
            runtime_version: None,
            capabilities: BTreeMap::new(),
        };
        engine
            .confirm(task_id, "alice", false, Some(new_target.clone()), None)
            .await
            .unwrap();

        let data: AppInstallTaskData = serde_json::from_value(store.data_of(task_id)).unwrap();
        let plan = data.state.plan.as_ref().unwrap();
        assert_eq!(plan.target, new_target);
        assert_ne!(plan.plan_fingerprint, old_fp);
        // approval 绑定新 fingerprint。
        assert_eq!(
            data.state.approval.as_ref().unwrap().plan_fingerprint,
            plan.plan_fingerprint
        );
        // Inspect 重算了一次（初始 1 + confirm 1）。
        assert_eq!(driver.count("inspect"), 2);

        let outcome = engine.run_task(task_id).await.unwrap();
        assert_eq!(outcome, RunOutcome::Completed);
    }
}
