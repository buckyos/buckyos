use buckyos_api::{
    app_availability_audit_key, app_availability_policy_key, get_buckyos_api_runtime,
    parse_app_instance_id, user_app_spec_key, AgentSpec, AppAvailabilityPolicy, AppDataDisposition,
    AppDeletionManifest, AppDoc, AppId, AppInstanceId, AppLifecycleAction, AppLifecycleTaskResult,
    AppServiceSpec, AppStartTaskData, AppStartTaskRequest, AppType, AppUninstallTaskData,
    AppUninstallTaskRequest, AppUninstallTaskResult, AppUpdateAvailability,
    AppUpdateBatchItemOutcome, AppUpdateBatchItemResult, AppUpdateBatchProgress,
    AppUpdateBatchRequestItem, AppUpdateBatchTaskData, AppUpdateBatchTaskRequest,
    AppUpdateBatchTaskResult, AppUpdateState, AvailabilityEffect, InstallPolicy, RepoClient,
    RestartStrategy, ServiceInstanceReportInfo, ServiceInstanceState, ServiceState, SubPkgDesc,
    SystemConfigClient, SystemConfigError, TaskManagerClient, TaskOutcome, TaskPhase,
    APP_AVAILABILITY_SCHEMA_VERSION, APP_INSTALL_SCHEMA_VERSION, APP_INSTALL_TASK_SCHEMA_ID,
    APP_START_TASK_SCHEMA_ID, APP_UNINSTALL_TASK_SCHEMA_ID, APP_UPDATE_BATCH_TASK_SCHEMA_ID,
    APP_UPDATE_TASK_SCHEMA_ID,
};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use flate2::write::GzEncoder;
use flate2::Compression;
use kRPC::RPCErrors;
use log::{info, warn};
use named_store::NamedDataMgr;
use ndn_lib::{build_named_object_by_json, FileObject, NamedObject, ObjId, StoreMode};
use ndn_toolkit::{cacl_file_object, CheckMode};
use package_lib::{PackageId, PackageMeta};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::File as StdFile;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tar::Builder;
use tokio::fs;
use tokio::time::sleep;
use uuid::Uuid;

// 主要流程（AppInstaller 视角）：
// - install_app: 写 users/{uid}/apps|agents/{app}/spec (state=New) -> 等待 scheduler 调度
// - uninstall_app: spec -> Deleted -> stop_app -> 等待 scheduler RemoveInstance
// - start_app / stop_app: 改 spec.state -> 触发 scheduler 调度
// - upgrade_app: stop 后覆盖 spec -> 触发 scheduler 重新分配 instance
//
// 调度器视角核心流程（doc/arch/scheduler.md, system_config_agent.rs）：
// 1. schedule_loop 每 5s: dump_configs_for_scheduler -> create_scheduler_by_system_config -> schedule(last_snapshot) -> exec_tx -> 写 system/scheduler/snapshot
// 2. 输入: devices/*/info(节点), users/*/apps|agents/*/spec(非 Static app), nodes/*/config(已有实例), services/*/instances/*(实例上报)
// 3. schedule() 四阶段: Step1 resort_nodes -> Step2 schedule_spec_change(New->选点+InstanceReplica, Deleted->RemoveInstance) -> Step4 calc_service_infos
// 4. 输出: InstanceReplica -> nodes/{node}/config.apps, RemoveInstance -> 删 node config, UpdateServiceInfo -> services/{spec}/info
// 5. node-daemon 读 nodes/{node}/config 收敛实例; 实例上报 services/{spec}/instances/{node}; gateway 读 service_info 做路由
const UNINSTALL_TASK_TYPE: &str = "app.uninstall";
const START_TASK_TYPE: &str = "app.start";
const WAIT_INTERVAL_MS: u64 = 1_000;
const WAIT_TIMEOUT_SECS: u64 = 45;

#[derive(Clone)]
enum PackageSource {
    Directory(PathBuf),
    File {
        path: PathBuf,
        packaged_name: Option<String>,
    },
}

struct ScannedSubPkg {
    key: String,
    desc: SubPkgDesc,
    source: PackageSource,
}

struct PublishScanPlan {
    app_bundle: Option<PackageSource>,
    sub_pkgs: Vec<ScannedSubPkg>,
}

struct PreparedPayload {
    file_object: Option<FileObject>,
    tarball_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppSubmitAction {
    FreshInstall,
    Upgrade,
    Satisfied,
}

fn decide_app_submit_action(
    submitted_plan_use: Option<buckyos_api::InstallPlanUse>,
    installed_object_id: Option<&ObjId>,
    target_object_id: &ObjId,
    installed_document_version: Option<u64>,
    target_document_version: Option<u64>,
) -> Result<AppSubmitAction, buckyos_api::InstallError> {
    match submitted_plan_use {
        Some(buckyos_api::InstallPlanUse::FreshInstall) => {
            return if installed_object_id.is_some() {
                Err(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::PlanNotApplicable,
                    false,
                    "FreshInstall plan cannot be consumed by an installed target",
                ))
            } else {
                Ok(AppSubmitAction::FreshInstall)
            };
        }
        Some(buckyos_api::InstallPlanUse::Upgrade) if installed_object_id.is_none() => {
            return Err(buckyos_api::InstallError::new(
                buckyos_api::InstallStage::Inspect,
                buckyos_api::InstallErrorCode::PlanNotApplicable,
                false,
                "Upgrade plan requires an installed target",
            ));
        }
        Some(buckyos_api::InstallPlanUse::Satisfied) => {
            return Err(buckyos_api::InstallError::new(
                buckyos_api::InstallStage::Inspect,
                buckyos_api::InstallErrorCode::PlanNotApplicable,
                false,
                "Satisfied inspection result cannot be submitted as a mutation plan",
            ));
        }
        _ => {}
    }
    let Some(installed_object_id) = installed_object_id else {
        return Err(buckyos_api::InstallError::new(
            buckyos_api::InstallStage::Inspect,
            buckyos_api::InstallErrorCode::PlanRequired,
            false,
            "first installation requires an approved FreshInstall plan",
        ));
    };
    if installed_object_id == target_object_id {
        return Ok(AppSubmitAction::Satisfied);
    }
    if matches!(
        (installed_document_version, target_document_version),
        (Some(installed), Some(target)) if target < installed
    ) {
        return Err(buckyos_api::InstallError::new(
            buckyos_api::InstallStage::Inspect,
            buckyos_api::InstallErrorCode::DowngradeNotAllowed,
            false,
            "resolved App Document is older than the installed authority version",
        ));
    }
    Ok(AppSubmitAction::Upgrade)
}

struct PreparedSubPkg {
    key: String,
    desc: SubPkgDesc,
    meta: PackageMeta,
    tarball_path: PathBuf,
}

struct PreparedPublishPlan {
    app_bundle: Option<PreparedPayload>,
    sub_pkgs: Vec<PreparedSubPkg>,
    /// 发布临时目录：payload tar.gz 在 pikg 打包完成前保留。
    temp_root: PathBuf,
}

/// app.publish 的完整产物（§11）。
pub struct PublishOutput {
    pub app_did: name_lib::DID,
    pub app_doc_object_id: ObjId,
    /// 最终 App Document body（canonical 前的 JSON 值，供工具链/测试消费）。
    pub app_doc_value: Value,
    pub pikg_digest: String,
    pub pikg_handle: String,
    pub staged_pikg_path: PathBuf,
    /// 发布状态：发布到 Repo/NamedStore 与发布 App DID 是两个动作；
    /// 这里只完成前者，App Document 仍是未经权威发布的 candidate。
    pub publish_status: &'static str,
}

fn task_data_value<T: Serialize>(data: T) -> Result<Value, RPCErrors> {
    serde_json::to_value(data)
        .map_err(|error| RPCErrors::ReasonError(format!("Serialize task data failed: {error}")))
}

#[derive(Clone)]
pub struct AppInstaller {
    wait_interval: Duration,
    wait_timeout: Duration,
    running_lifecycle_tasks: Arc<tokio::sync::Mutex<HashSet<String>>>,
}

impl AppInstaller {
    pub fn new() -> Self {
        Self {
            wait_interval: Duration::from_millis(WAIT_INTERVAL_MS),
            wait_timeout: Duration::from_secs(WAIT_TIMEOUT_SECS),
            running_lifecycle_tasks: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        }
    }

    async fn system_config_client(&self) -> Result<Arc<SystemConfigClient>, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_system_config_client().await
    }

    async fn task_mgr_client(&self) -> Result<TaskManagerClient, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_task_mgr_client().await
    }

    async fn ensure_runtime_has_no_agent_bindings(
        &self,
        target: &AppInstanceId,
    ) -> Result<(), RPCErrors> {
        let client = self.system_config_client().await?;
        let users = match client.list("users").await {
            Ok(users) => users,
            Err(SystemConfigError::KeyNotFound(_)) => Vec::new(),
            Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
        };
        for owner in users {
            let root = format!("users/{owner}/agents");
            let agent_ids = match client.list(&root).await {
                Ok(agent_ids) => agent_ids,
                Err(SystemConfigError::KeyNotFound(_)) => continue,
                Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
            };
            for agent_id in agent_ids {
                let path = format!("{root}/{agent_id}/spec");
                let Ok(value) = client.get(&path).await else {
                    continue;
                };
                let spec: AgentSpec = serde_json::from_str(&value.value).map_err(|error| {
                    RPCErrors::ReasonError(format!("invalid AgentSpec at {path}: {error}"))
                })?;
                spec.validate().map_err(RPCErrors::ReasonError)?;
                if spec.binding.references_runtime(target) {
                    return Err(RPCErrors::ReasonError(format!(
                        "cannot uninstall runtime {target}: Agent {} still references it",
                        spec.agent_id
                    )));
                }
            }
        }
        Ok(())
    }

    async fn repo_client(&self) -> Result<RepoClient, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_repo_client().await.map_err(|error| {
            warn!("init repo client failed: {}", error);
            RPCErrors::ReasonError(format!("Init repo client failed: {}", error))
        })
    }

    fn should_wait_for_instance(spec: &AppServiceSpec) -> bool {
        spec.app_doc.get_app_type() != AppType::Web
            && spec.enable
            && spec.expected_instance_count > 0
            && spec.state != ServiceState::Stopped
            && spec.state != ServiceState::Deleted
    }

    fn spec_storage_path(spec: &AppServiceSpec) -> String {
        user_app_spec_key(&spec.owner_user_id, spec.app_id())
    }

    fn service_spec_id(spec: &AppServiceSpec) -> String {
        spec.app_instance_id().to_string()
    }

    fn service_state_label(state: &ServiceState) -> &'static str {
        match state {
            ServiceState::New => "new",
            ServiceState::Running => "running",
            ServiceState::Stopped => "stopped",
            ServiceState::Stopping => "stopping",
            ServiceState::Restarting => "restarting",
            ServiceState::Updating => "updating",
            ServiceState::Deleted => "deleted",
        }
    }

    fn log_spec_state_change(spec: &AppServiceSpec, state: ServiceState, detail: &str) {
        info!(
            "app `{}` for user `{}` state -> {}: {}",
            spec.app_id(),
            spec.owner_user_id,
            Self::service_state_label(&state),
            detail
        );
    }

    fn to_rpc_error(error: SystemConfigError) -> RPCErrors {
        RPCErrors::ReasonError(error.to_string())
    }

    async fn list_children(
        client: &SystemConfigClient,
        key: &str,
    ) -> Result<Vec<String>, RPCErrors> {
        match client.list(key).await {
            Ok(items) => Ok(items),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(Vec::new()),
            Err(error) => Err(Self::to_rpc_error(error)),
        }
    }

    async fn get_optional_json<T: DeserializeOwned>(
        client: &SystemConfigClient,
        key: &str,
    ) -> Result<Option<T>, RPCErrors> {
        match client.get(key).await {
            Ok(value) => serde_json::from_str::<T>(&value.value)
                .map(Some)
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("Failed to parse `{key}` as JSON: {error}"))
                }),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
            Err(error) => Err(Self::to_rpc_error(error)),
        }
    }

    async fn set_spec_at(
        client: &SystemConfigClient,
        key: &str,
        spec: &AppServiceSpec,
    ) -> Result<(), RPCErrors> {
        let raw = serde_json::to_string(spec).map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to serialize app spec `{key}`: {error}"))
        })?;
        client.set(key, &raw).await.map_err(Self::to_rpc_error)?;
        Ok(())
    }

    async fn mark_spec_deleted(
        client: &SystemConfigClient,
        spec_key: &str,
        spec: &mut AppServiceSpec,
        updated_by: &str,
    ) -> Result<(), RPCErrors> {
        spec.state = ServiceState::Deleted;
        for expose in spec.spec_config.expose_config.values_mut() {
            expose.allow_guest = false;
        }

        let app_instance_id = spec.app_instance_id();
        let policy_key = app_availability_policy_key(app_instance_id);
        let stored_policy = match client.get(&policy_key).await {
            Ok(value) => value,
            Err(SystemConfigError::KeyNotFound(_)) => {
                return Self::set_spec_at(client, spec_key, spec).await;
            }
            Err(error) => return Err(Self::to_rpc_error(error)),
        };
        let mut policy: AppAvailabilityPolicy = serde_json::from_str(&stored_policy.value)
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "Failed to parse availability policy `{policy_key}`: {error}"
                ))
            })?;
        if policy.schema_version != APP_AVAILABILITY_SCHEMA_VERSION
            || policy.app_instance_id != *app_instance_id
        {
            return Err(RPCErrors::ReasonError(format!(
                "Availability policy `{policy_key}` does not match the app instance"
            )));
        }

        let old_revision = policy.revision;
        policy.revision += 1;
        policy.default_effect = AvailabilityEffect::Deny;
        policy.group_rules.clear();
        policy.user_rules.clear();
        policy.updated_by = updated_by.to_string();
        policy.updated_at = buckyos_get_unix_timestamp();

        let spec_raw = serde_json::to_string(spec).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to serialize app spec `{spec_key}`: {error}"
            ))
        })?;
        let policy_raw = serde_json::to_string(&policy).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to serialize availability policy `{policy_key}`: {error}"
            ))
        })?;
        let audit_key = app_availability_audit_key(&app_instance_id, policy.revision);
        let audit_raw = json!({
            "schema_version": APP_AVAILABILITY_SCHEMA_VERSION,
            "app_instance_id": app_instance_id,
            "updated_by": updated_by,
            "updated_at": policy.updated_at,
            "old_revision": old_revision,
            "new_revision": policy.revision,
            "change": "uninstall_reset",
            "group_rule_count": 0,
            "user_rule_count": 0,
            "guest_allowed": false,
        })
        .to_string();
        let mut actions = HashMap::new();
        actions.insert(spec_key.to_string(), KVAction::Update(spec_raw));
        actions.insert(policy_key.clone(), KVAction::Update(policy_raw));
        actions.insert(audit_key, KVAction::Create(audit_raw));
        client
            .exec_tx(actions, Some((policy_key, stored_policy.version)))
            .await
            .map_err(Self::to_rpc_error)?;
        Ok(())
    }

    async fn find_matching_specs(
        &self,
        app_id: &str,
        user_id: Option<&str>,
    ) -> Result<Vec<(String, AppServiceSpec)>, RPCErrors> {
        let client = self.system_config_client().await?;
        let mut matches = Vec::new();

        let users = Self::list_children(&client, "users").await?;

        for current_user in users {
            if let Some(expected_user) = user_id {
                if current_user != expected_user {
                    continue;
                }
            }

            let key = format!("users/{}/apps/{}/spec", current_user, app_id);
            if let Some(spec) = Self::get_optional_json::<AppServiceSpec>(&client, &key).await? {
                matches.push((key, spec));
            }
        }

        Ok(matches)
    }

    async fn get_single_matching_spec(
        &self,
        app_id: &str,
        user_id: Option<&str>,
    ) -> Result<(String, AppServiceSpec), RPCErrors> {
        let matches = self.find_matching_specs(app_id, user_id).await?;
        if matches.is_empty() {
            return Err(RPCErrors::ReasonError(format!(
                "App `{app_id}` is not installed"
            )));
        }
        if matches.len() > 1 {
            return Err(RPCErrors::ReasonError(format!(
                "App `{app_id}` is installed for multiple users; specify user_id explicitly"
            )));
        }
        Ok(matches.into_iter().next().unwrap())
    }

    async fn wait_for_instance_ready(&self, spec: &AppServiceSpec) -> Result<u32, RPCErrors> {
        let client = self.system_config_client().await?;
        let instances_key = format!("services/{}/instances", Self::service_spec_id(spec));
        let mut waited = Duration::ZERO;
        while waited <= self.wait_timeout {
            let now = buckyos_get_unix_timestamp();
            let mut ready_nodes = HashSet::new();
            for node_id in Self::list_children(&client, &instances_key).await? {
                let key = format!("{instances_key}/{node_id}");
                if let Some(instance) =
                    Self::get_optional_json::<ServiceInstanceReportInfo>(&client, &key).await?
                {
                    if instance.deployment.as_ref() == Some(&spec.deployment)
                        && instance.state == ServiceInstanceState::Started
                        && instance.health == buckyos_api::DeploymentHealth::Healthy
                        && instance.observed_at <= now
                        && instance.expires_at > now
                        && !instance.instance_epoch.is_empty()
                        && !instance.node_session_id.is_empty()
                    {
                        ready_nodes.insert(node_id);
                    }
                }
            }
            if ready_nodes.len() >= spec.expected_instance_count.max(1) as usize {
                return Ok(ready_nodes.len() as u32);
            }
            sleep(self.wait_interval).await;
            waited += self.wait_interval;
        }

        let error = RPCErrors::ReasonError(format!(
            "Timed out waiting for app `{}` instance to become ready",
            spec.app_id()
        ));
        warn!(
            "wait for app `{}` user `{}` instance ready failed: {}",
            spec.app_id(),
            spec.owner_user_id,
            error
        );
        Err(error)
    }

    async fn wait_for_instances_removed(&self, spec: &AppServiceSpec) -> Result<(), RPCErrors> {
        let service_spec_id = Self::service_spec_id(spec);
        let client = self.system_config_client().await?;
        let instances_key = format!("services/{service_spec_id}/instances");
        let mut waited = Duration::ZERO;

        while waited <= self.wait_timeout {
            let now = buckyos_get_unix_timestamp();
            let node_ids = Self::list_children(&client, &instances_key).await?;
            if node_ids.is_empty() {
                info!(
                    "all instances removed for app `{}` user `{}`",
                    spec.app_id(),
                    spec.owner_user_id
                );
                return Ok(());
            }

            let mut has_active = false;
            for node_id in node_ids {
                let key = format!("{instances_key}/{node_id}");
                if let Some(instance) =
                    Self::get_optional_json::<ServiceInstanceReportInfo>(&client, &key).await?
                {
                    if instance.deployment.as_ref() == Some(&spec.deployment)
                        && matches!(
                            instance.state,
                            ServiceInstanceState::Started | ServiceInstanceState::Deploying
                        )
                        && instance.observed_at <= now
                        && instance.expires_at > now
                    {
                        has_active = true;
                        break;
                    }
                }
            }

            if !has_active {
                info!(
                    "all active instances stopped for app `{}` user `{}`",
                    spec.app_id(),
                    spec.owner_user_id
                );
                return Ok(());
            }

            sleep(self.wait_interval).await;
            waited += self.wait_interval;
        }

        let error = RPCErrors::ReasonError(format!(
            "Timed out waiting for app `{}` instances to stop",
            spec.app_id()
        ));
        warn!(
            "wait for app `{}` user `{}` instances removed failed: {}",
            spec.app_id(),
            spec.owner_user_id,
            error
        );
        Err(error)
    }

    async fn create_task(
        &self,
        name: String,
        task_type: &str,
        data: Value,
        creator_user_id: &str,
        creator_app_id: &str,
        idempotency_key: &str,
    ) -> Result<(String, String), RPCErrors> {
        let task_mgr = self.task_mgr_client().await?;
        let task = task_mgr
            .create_delegated_task(buckyos_api::CreateDelegatedTaskReq {
                task_id: None,
                name,
                schema_id: format!("{}/v1", task_type.replace('/', ".")),
                schema_version: None,
                input: data,
                creator: buckyos_api::ActorRef::new(creator_user_id, creator_app_id),
                runner_app_instance_id: None,
                parent_id: None,
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                idempotency_key: idempotency_key.to_string(),
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await?;
        Ok((task.task_id, task.root_id))
    }

    fn task_envelope(task: &buckyos_api::Task) -> buckyos_api::RunnerWriteEnvelope {
        let app_instance_id = match &task.executor {
            buckyos_api::TaskExecutor::App {
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

    async fn enable_safe_cancel(&self, task_id: &str) -> Result<(), RPCErrors> {
        let client = self.task_mgr_client().await?;
        let task = client.get_task(task_id).await?;
        client
            .update_control_profile(buckyos_api::UpdateControlProfileReq {
                envelope: Self::task_envelope(&task),
                profile: buckyos_api::TaskControlProfile {
                    pause: buckyos_api::ControlAvailability::Unavailable { reason: None },
                    resume: buckyos_api::ControlAvailability::Unavailable { reason: None },
                    cancel: buckyos_api::CancelCapability::Safe,
                    updated_at: buckyos_get_unix_timestamp() * 1000,
                },
            })
            .await?;
        Ok(())
    }

    async fn acknowledge_pending_cancel(&self, task_id: &str) -> Result<bool, RPCErrors> {
        let client = self.task_mgr_client().await?;
        let task = client.get_task(task_id).await?;
        let Some(pending) = task.pending_control.as_ref() else {
            return Ok(false);
        };
        if pending.action != buckyos_api::TaskControlAction::Cancel {
            return Ok(false);
        }
        client
            .ack_control(buckyos_api::AckControlReq {
                envelope: Self::task_envelope(&task),
                request_id: pending.request_id.clone(),
                applied: true,
                reject_reason: None,
            })
            .await?;
        Ok(true)
    }

    async fn disable_cancel(&self, task_id: &str, reason: &str) -> Result<(), RPCErrors> {
        let client = self.task_mgr_client().await?;
        let task = client.get_task(task_id).await?;
        client
            .update_control_profile(buckyos_api::UpdateControlProfileReq {
                envelope: Self::task_envelope(&task),
                profile: buckyos_api::TaskControlProfile {
                    pause: buckyos_api::ControlAvailability::Unavailable { reason: None },
                    resume: buckyos_api::ControlAvailability::Unavailable { reason: None },
                    cancel: buckyos_api::CancelCapability::Unavailable {
                        reason: Some(reason.to_string()),
                    },
                    updated_at: buckyos_get_unix_timestamp() * 1000,
                },
            })
            .await?;
        Ok(())
    }

    async fn run_start_task(
        &self,
        data: AppStartTaskData,
        task_id: String,
    ) -> Result<(), RPCErrors> {
        let client = self.system_config_client().await?;
        let request = data.request;
        let (spec_key, mut spec) = self
            .get_single_matching_spec(
                request.app_instance_id.app_id().as_str(),
                Some(request.app_instance_id.owner_user_id()),
            )
            .await?;

        if spec.state == ServiceState::Deleted {
            let error = RPCErrors::ReasonError(format!(
                "App `{}` has been deleted and can not be managed",
                request.app_instance_id
            ));
            return Err(error);
        }
        if spec.app_instance_id != request.app_instance_id {
            return Err(RPCErrors::ReasonError(
                "lifecycle task installation identity changed".to_string(),
            ));
        }
        self.ensure_runtime_has_no_agent_bindings(&spec.app_instance_id)
            .await?;

        let task_mgr = self.task_mgr_client().await?;
        task_mgr.runner_start(&task_id).await?;
        self.enable_safe_cancel(&task_id).await?;
        if self.acknowledge_pending_cancel(&task_id).await? {
            return Ok(());
        }

        let ready_instance_count = match request.action {
            AppLifecycleAction::Start => {
                task_mgr
                    .runner_progress(
                        &task_id,
                        Some(json!({"percent": 20.0, "action": "start"})),
                        Some("Setting desired state to running".to_string()),
                    )
                    .await?;
                spec.state = ServiceState::Running;
                Self::set_spec_at(&client, &spec_key, &spec).await?;
                let ready = if Self::should_wait_for_instance(&spec) {
                    self.wait_for_instance_ready(&spec).await?
                } else {
                    0
                };
                if self.acknowledge_pending_cancel(&task_id).await? {
                    return Ok(());
                }
                ready
            }
            AppLifecycleAction::Stop => {
                task_mgr
                    .runner_progress(
                        &task_id,
                        Some(json!({"percent": 20.0, "action": "stop"})),
                        Some("Setting desired state to stopped".to_string()),
                    )
                    .await?;
                spec.state = ServiceState::Stopped;
                Self::set_spec_at(&client, &spec_key, &spec).await?;
                self.wait_for_instances_removed(&spec).await?;
                if self.acknowledge_pending_cancel(&task_id).await? {
                    return Ok(());
                }
                0
            }
            AppLifecycleAction::Restart => {
                if request.restart_strategy == RestartStrategy::Rolling {
                    return Err(RPCErrors::ReasonError(
                        "rolling restart is not supported by the current deployment strategy"
                            .to_string(),
                    ));
                }
                task_mgr
                    .runner_progress(
                        &task_id,
                        Some(json!({"percent": 15.0, "action": "restart"})),
                        Some("Stopping current instances".to_string()),
                    )
                    .await?;
                spec.state = ServiceState::Stopped;
                Self::set_spec_at(&client, &spec_key, &spec).await?;
                self.wait_for_instances_removed(&spec).await?;
                if self.acknowledge_pending_cancel(&task_id).await? {
                    return Ok(());
                }
                task_mgr
                    .runner_progress(
                        &task_id,
                        Some(json!({"percent": 60.0, "action": "restart"})),
                        Some("Starting replacement instances".to_string()),
                    )
                    .await?;
                spec.state = ServiceState::Running;
                Self::set_spec_at(&client, &spec_key, &spec).await?;
                let ready = if Self::should_wait_for_instance(&spec) {
                    self.wait_for_instance_ready(&spec).await?
                } else {
                    0
                };
                if self.acknowledge_pending_cancel(&task_id).await? {
                    return Ok(());
                }
                ready
            }
        };

        task_mgr
            .runner_complete(
                &task_id,
                serde_json::to_value(AppLifecycleTaskResult {
                    app_instance_id: spec.app_instance_id.clone(),
                    action: request.action,
                    desired_state: spec.state.clone(),
                    ready_instance_count,
                    completed_at: buckyos_get_unix_timestamp(),
                })
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
            )
            .await?;
        info!(
            "start task {} completed for app `{}` user `{}`",
            task_id,
            spec.app_id(),
            spec.owner_user_id
        );
        Ok(())
    }

    async fn run_uninstall_task(
        &self,
        data: AppUninstallTaskData,
        task_id: String,
    ) -> Result<(), RPCErrors> {
        let client = self.system_config_client().await?;
        let request = data.request;
        let (spec_key, mut spec) = self
            .get_single_matching_spec(
                request.app_instance_id.app_id().as_str(),
                Some(request.app_instance_id.owner_user_id()),
            )
            .await?;
        if spec.app_instance_id != request.app_instance_id {
            return Err(RPCErrors::ReasonError(
                "uninstall task installation identity changed".to_string(),
            ));
        }

        let task_mgr = self.task_mgr_client().await?;
        task_mgr.runner_start(&task_id).await?;
        self.enable_safe_cancel(&task_id).await?;
        if self.acknowledge_pending_cancel(&task_id).await? {
            return Ok(());
        }
        task_mgr
            .runner_progress(
                &task_id,
                Some(json!({"percent": 15.0})),
                Some("Stopping app".to_string()),
            )
            .await?;

        spec.state = ServiceState::Stopped;
        Self::set_spec_at(&client, &spec_key, &spec).await?;
        self.wait_for_instances_removed(&spec).await?;
        if self.acknowledge_pending_cancel(&task_id).await? {
            return Ok(());
        }
        self.disable_cancel(
            &task_id,
            "uninstall has crossed the metadata deletion boundary",
        )
        .await?;

        task_mgr
            .runner_progress(
                &task_id,
                Some(json!({"percent": 55.0})),
                Some("Marking app as deleted".to_string()),
            )
            .await?;

        Self::mark_spec_deleted(
            &client,
            &spec_key,
            &mut spec,
            request.creator_user_id.as_str(),
        )
        .await?;
        Self::log_spec_state_change(
            &spec,
            ServiceState::Deleted,
            format!("uninstall task {} wrote spec `{}`", task_id, spec_key).as_str(),
        );

        self.wait_for_instances_removed(&spec).await?;

        let deleted_paths = if request.data_disposition == AppDataDisposition::Delete {
            task_mgr
                .runner_progress(
                    &task_id,
                    Some(json!({"percent": 80.0})),
                    Some("Removing owned app data".to_string()),
                )
                .await?;
            self.remove_app_data(&request.deletion_manifest).await?
        } else {
            Vec::new()
        };

        task_mgr
            .runner_complete(
                &task_id,
                serde_json::to_value(AppUninstallTaskResult {
                    app_instance_id: spec.app_instance_id.clone(),
                    data_disposition: request.data_disposition,
                    deleted_paths,
                    completed_at: buckyos_get_unix_timestamp(),
                })
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
            )
            .await?;
        info!(
            "uninstall task {} completed for app `{}` user `{}`",
            task_id,
            spec.app_id(),
            spec.owner_user_id
        );
        Ok(())
    }

    fn removable_data_path(path: &Path) -> bool {
        let raw = path.to_string_lossy();
        path.is_absolute()
            && path.components().all(|component| {
                !matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
            && (raw.starts_with("/opt/buckyos/data/") || raw.starts_with("/opt/buckyos/cache/"))
    }

    async fn remove_app_data(
        &self,
        manifest: &AppDeletionManifest,
    ) -> Result<Vec<String>, RPCErrors> {
        let mut deleted = Vec::new();
        for raw_path in manifest
            .data_paths
            .iter()
            .chain(manifest.cache_paths.iter())
        {
            let path = PathBuf::from(raw_path);
            if !Self::removable_data_path(&path) {
                warn!("Skip unsafe app data cleanup: {}", path.display());
                continue;
            }

            match fs::metadata(&path).await {
                Ok(metadata) if metadata.is_dir() => {
                    fs::remove_dir_all(&path).await.map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "Remove app data dir `{}` failed: {error}",
                            path.display()
                        ))
                    })?;
                    deleted.push(raw_path.clone());
                }
                Ok(_) => {
                    fs::remove_file(&path).await.map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "Remove app data file `{}` failed: {error}",
                            path.display()
                        ))
                    })?;
                    deleted.push(raw_path.clone());
                }
                Err(_) => {}
            }
        }
        Ok(deleted)
    }

    fn build_deletion_manifest(spec: &AppServiceSpec) -> AppDeletionManifest {
        AppDeletionManifest {
            data_paths: spec
                .spec_config
                .data_mount_point
                .values()
                .map(|mount| mount.target_path.to_string_lossy().to_string())
                .filter(|path| Self::removable_data_path(Path::new(path)))
                .collect(),
            cache_paths: spec
                .spec_config
                .local_cache_mount_point
                .values()
                .map(|mount| mount.target_path.to_string_lossy().to_string())
                .filter(|path| Self::removable_data_path(Path::new(path)))
                .collect(),
        }
    }

    pub async fn uninstall_app(
        &self,
        spec: &AppServiceSpec,
        selector: &str,
        data_disposition: AppDataDisposition,
        creator_user_id: &str,
        creator_app_id: &str,
        idempotency_key: &str,
    ) -> Result<String, RPCErrors> {
        self.ensure_runtime_has_no_agent_bindings(&spec.app_instance_id)
            .await?;
        let data = AppUninstallTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request: AppUninstallTaskRequest {
                selector: selector.to_string(),
                app_instance_id: spec.app_instance_id.clone(),
                creator_user_id: creator_user_id.to_string(),
                creator_app_id: creator_app_id.to_string(),
                idempotency_key: idempotency_key.to_string(),
                data_disposition,
                deletion_manifest: Self::build_deletion_manifest(spec),
            },
            progress: None,
            result: None,
            error: None,
        };
        let (task_id, _root_id) = self
            .create_task(
                format!("Uninstall app {}", spec.app_doc.show_name),
                UNINSTALL_TASK_TYPE,
                task_data_value(data)?,
                creator_user_id,
                creator_app_id,
                idempotency_key,
            )
            .await?;
        Ok(task_id)
    }

    pub async fn lifecycle_app(
        &self,
        spec: &AppServiceSpec,
        selector: &str,
        action: AppLifecycleAction,
        restart_strategy: RestartStrategy,
        creator_user_id: &str,
        creator_app_id: &str,
        idempotency_key: &str,
    ) -> Result<String, RPCErrors> {
        let data = AppStartTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request: AppStartTaskRequest {
                selector: selector.to_string(),
                app_instance_id: spec.app_instance_id.clone(),
                creator_user_id: creator_user_id.to_string(),
                creator_app_id: creator_app_id.to_string(),
                idempotency_key: idempotency_key.to_string(),
                action,
                restart_strategy,
            },
            progress: None,
            result: None,
            error: None,
        };
        let (task_id, _root_id) = self
            .create_task(
                format!("{:?} app {}", action, spec.app_doc.show_name),
                START_TASK_TYPE,
                task_data_value(data)?,
                creator_user_id,
                creator_app_id,
                idempotency_key,
            )
            .await?;
        Ok(task_id)
    }

    async fn release_mutation(&self, app_instance_id: &buckyos_api::AppInstanceId, task_id: &str) {
        let Ok(client) = self.system_config_client().await else {
            return;
        };
        let key = format!("services/control_panel/app_mutations/{app_instance_id}");
        let Ok(current) = client.get(&key).await else {
            return;
        };
        let owned = serde_json::from_str::<Value>(&current.value)
            .ok()
            .and_then(|value| {
                value
                    .get("task_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .as_deref()
            == Some(task_id);
        if owned {
            let mut actions = HashMap::new();
            actions.insert(key.clone(), KVAction::Remove);
            let _ = client.exec_tx(actions, Some((key, current.version))).await;
        }
    }

    pub fn spawn_lifecycle_task(&self, task_id: String) {
        let installer = self.clone();
        tokio::spawn(async move {
            {
                let mut running = installer.running_lifecycle_tasks.lock().await;
                if !running.insert(task_id.clone()) {
                    return;
                }
            }
            let result = installer.run_persisted_lifecycle_task(&task_id).await;
            if let Err(error) = result {
                warn!("lifecycle task {} failed: {}", task_id, error);
                if let Ok(task_mgr) = installer.task_mgr_client().await {
                    let _ = task_mgr
                        .runner_fail(&task_id, "app_lifecycle_failed", error.to_string(), None)
                        .await;
                }
            }
            if let Ok(task_mgr) = installer.task_mgr_client().await {
                if let Ok(task) = task_mgr.get_task(&task_id).await {
                    let app_instance_id = if task.schema_id == APP_START_TASK_SCHEMA_ID {
                        serde_json::from_value::<AppStartTaskData>(task.input)
                            .ok()
                            .map(|data| data.request.app_instance_id)
                    } else {
                        serde_json::from_value::<AppUninstallTaskData>(task.input)
                            .ok()
                            .map(|data| data.request.app_instance_id)
                    };
                    if let Some(app_instance_id) = app_instance_id {
                        installer.release_mutation(&app_instance_id, &task_id).await;
                    }
                }
            }
            installer
                .running_lifecycle_tasks
                .lock()
                .await
                .remove(&task_id);
        });
    }

    async fn run_persisted_lifecycle_task(&self, task_id: &str) -> Result<(), RPCErrors> {
        let task = self.task_mgr_client().await?.get_task(task_id).await?;
        if task.phase.is_terminal() {
            return Ok(());
        }
        match task.schema_id.as_str() {
            APP_START_TASK_SCHEMA_ID => {
                let data: AppStartTaskData =
                    serde_json::from_value(task.input).map_err(|error| {
                        RPCErrors::ReasonError(format!("invalid lifecycle task input: {error}"))
                    })?;
                self.run_start_task(data, task_id.to_string()).await
            }
            APP_UNINSTALL_TASK_SCHEMA_ID => {
                let data: AppUninstallTaskData =
                    serde_json::from_value(task.input).map_err(|error| {
                        RPCErrors::ReasonError(format!("invalid uninstall task input: {error}"))
                    })?;
                self.run_uninstall_task(data, task_id.to_string()).await
            }
            other => Err(RPCErrors::ReasonError(format!(
                "unsupported lifecycle schema `{other}`"
            ))),
        }
    }

    pub fn start_lifecycle_runner(&self) {
        let installer = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let Ok(client) = installer.task_mgr_client().await else {
                    continue;
                };
                for schema_id in [APP_START_TASK_SCHEMA_ID, APP_UNINSTALL_TASK_SCHEMA_ID] {
                    let mut cursor = None;
                    loop {
                        let Ok(page) = client
                            .list_tasks(buckyos_api::ListTasksReq {
                                schema_id: Some(schema_id.to_string()),
                                runner_app_id: Some(
                                    buckyos_api::CONTROL_PANEL_SERVICE_NAME.to_string(),
                                ),
                                cursor: cursor.clone(),
                                limit: Some(100),
                                ..Default::default()
                            })
                            .await
                        else {
                            break;
                        };
                        for task in page.tasks {
                            if !task.phase.is_terminal()
                                && matches!(task.phase, TaskPhase::Accepted | TaskPhase::Running)
                            {
                                installer.spawn_lifecycle_task(task.task_id);
                            }
                        }
                        cursor = page.next_cursor;
                        if cursor.is_none() {
                            break;
                        }
                    }
                }
            }
        });
    }

    /// 查询应用 spec。
    /// 流程：从 system_config 读取 users/{uid}/apps/{app}/spec 或 users/{uid}/agents/{app}/spec。
    pub async fn get_app_service_spec(
        &self,
        app_id: &str,
        user_id: Option<&str>,
    ) -> Result<AppServiceSpec, RPCErrors> {
        let (_, spec) = self.get_single_matching_spec(app_id, user_id).await?;
        Ok(spec)
    }

    pub async fn get_app_service_spec_by_instance(
        &self,
        app_instance_id: &str,
    ) -> Result<AppServiceSpec, RPCErrors> {
        let (app_id, owner_user_id) = parse_app_instance_id(app_instance_id)?;
        let spec = self
            .get_app_service_spec(app_id.as_str(), Some(&owner_user_id))
            .await?;
        if spec.app_instance_id().to_string() != app_instance_id {
            return Err(RPCErrors::ReasonError(format!(
                "installed app spec does not match `{app_instance_id}`"
            )));
        }
        Ok(spec)
    }

    /// 查询应用实例状态（ServiceInstanceReportInfo）。
    /// 流程：从 services/{spec}/instances/{node} 或 nodes/{node}/config 聚合实例上报信息。
    pub async fn get_app_service_instance_config(
        &self,
        app_id: &str,
        user_id: Option<&str>,
    ) -> Result<ServiceInstanceReportInfo, RPCErrors> {
        let client = self.system_config_client().await?;
        let spec = self.get_app_service_spec(app_id, user_id).await?;
        let service_spec_id = Self::service_spec_id(&spec);
        let instances_key = format!("services/{service_spec_id}/instances");
        let node_ids = Self::list_children(&client, &instances_key).await?;

        let mut latest: Option<ServiceInstanceReportInfo> = None;
        for node_id in node_ids {
            let key = format!("{instances_key}/{node_id}");
            if let Some(instance) =
                Self::get_optional_json::<ServiceInstanceReportInfo>(&client, &key).await?
            {
                let replace = latest
                    .as_ref()
                    .map(|current| instance.last_update_time >= current.last_update_time)
                    .unwrap_or(true);
                if replace {
                    latest = Some(instance);
                }
            }
        }

        latest.ok_or_else(|| {
            RPCErrors::ReasonError(format!(
                "No instance report found for app `{}` (spec `{}`)",
                app_id, service_spec_id
            ))
        })
    }

    fn scan_publish_sources(
        &self,
        app_type: AppType,
        local_dir: &Path,
        app_doc_template: &AppDoc,
    ) -> Result<PublishScanPlan, RPCErrors> {
        if !local_dir.exists() || !local_dir.is_dir() {
            return Err(RPCErrors::ReasonError(format!(
                "Publish source directory not found: {}",
                local_dir.display()
            )));
        }

        if app_type == AppType::Service {
            return Err(RPCErrors::ReasonError(
                "publish_app_to_repo does not support Service app".to_string(),
            ));
        }

        let template_type = app_doc_template.get_app_type();
        if template_type != app_type {
            return Err(RPCErrors::ReasonError(format!(
                "App type mismatch: template is `{}`, request is `{}`",
                template_type, app_type
            )));
        }

        match app_type {
            AppType::Web => {
                let web_desc = app_doc_template.pkg_list.web.clone().ok_or_else(|| {
                    RPCErrors::ReasonError("Web app template missing `pkg_list.web`".to_string())
                })?;
                Ok(PublishScanPlan {
                    app_bundle: None,
                    sub_pkgs: vec![ScannedSubPkg {
                        key: "web".to_string(),
                        desc: web_desc,
                        source: PackageSource::Directory(local_dir.to_path_buf()),
                    }],
                })
            }
            AppType::Agent => {
                let agent_desc = app_doc_template.pkg_list.agent.clone().ok_or_else(|| {
                    RPCErrors::ReasonError(
                        "Agent app template missing `pkg_list.agent`".to_string(),
                    )
                })?;
                if app_doc_template.pkg_list.agent_skills.is_some() {
                    return Err(RPCErrors::ReasonError(
                        "Agent publish does not support `pkg_list.agent_skills` yet".to_string(),
                    ));
                }
                if app_doc_template.pkg_list.agent_tools.is_some() {
                    return Err(RPCErrors::ReasonError(
                        "Agent publish does not support `pkg_list.agent_tools` yet".to_string(),
                    ));
                }
                let sub_pkgs = vec![ScannedSubPkg {
                    key: "agent".to_string(),
                    desc: agent_desc,
                    source: PackageSource::Directory(local_dir.to_path_buf()),
                }];

                Ok(PublishScanPlan {
                    app_bundle: None,
                    sub_pkgs,
                })
            }
            AppType::AppService => {
                let has_unsupported_subpkg =
                    app_doc_template
                        .pkg_list
                        .iter()
                        .into_iter()
                        .any(|(key, _)| {
                            key != "amd64_docker_image"
                                && key != "aarch64_docker_image"
                                && key != "script"
                        });
                if has_unsupported_subpkg {
                    return Err(RPCErrors::ReasonError(
                        "AppService publish currently only supports `script`, `amd64_docker_image`, and `aarch64_docker_image`"
                            .to_string(),
                    ));
                }

                if let Some(script_desc) = app_doc_template.pkg_list.script.clone() {
                    if app_doc_template.pkg_list.amd64_docker_image.is_some()
                        || app_doc_template.pkg_list.aarch64_docker_image.is_some()
                    {
                        return Err(RPCErrors::ReasonError(
                            "AppService publish does not support mixing `script` and docker image packages yet"
                                .to_string(),
                        ));
                    }
                    return Ok(PublishScanPlan {
                        app_bundle: None,
                        sub_pkgs: vec![ScannedSubPkg {
                            key: "script".to_string(),
                            desc: script_desc,
                            source: PackageSource::Directory(local_dir.to_path_buf()),
                        }],
                    });
                }

                let mut sub_pkgs = Vec::new();
                for key in ["amd64_docker_image", "aarch64_docker_image"] {
                    let Some(desc) = app_doc_template.pkg_list.get(key).cloned() else {
                        continue;
                    };
                    let tar_path = local_dir.join(format!("{key}.tar"));
                    if tar_path.exists() {
                        sub_pkgs.push(ScannedSubPkg {
                            key: key.to_string(),
                            desc: desc.clone(),
                            source: PackageSource::File {
                                path: tar_path,
                                packaged_name: Self::canonical_packaged_name(
                                    key,
                                    &desc,
                                    app_doc_template.show_name.as_str(),
                                ),
                            },
                        });
                    }
                }

                if sub_pkgs.is_empty()
                    && app_doc_template.pkg_list.amd64_docker_image.is_none()
                    && app_doc_template.pkg_list.aarch64_docker_image.is_none()
                {
                    return Err(RPCErrors::ReasonError(
                        "AppService template must define at least one docker image entry"
                            .to_string(),
                    ));
                }

                Ok(PublishScanPlan {
                    app_bundle: None,
                    sub_pkgs,
                })
            }
            AppType::Service => unreachable!(),
        }
    }

    async fn package_publish_sources(
        &self,
        app_doc_template: &AppDoc,
        plan: PublishScanPlan,
    ) -> Result<PreparedPublishPlan, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let named_store = runtime.get_named_store().await.map_err(|error| {
            RPCErrors::ReasonError(format!("Open named store for publish failed: {error}"))
        })?;
        let temp_root = std::env::temp_dir().join(format!("buckyos-publish-{}", Uuid::new_v4()));
        fs::create_dir_all(&temp_root).await.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Create publish temp directory `{}` failed: {error}",
                temp_root.display()
            ))
        })?;

        let app_bundle = match plan.app_bundle.as_ref() {
            Some(app_bundle_source) => Some(
                self.package_source_to_payload(
                    &named_store,
                    &temp_root,
                    app_bundle_source,
                    format!(
                        "{}-app",
                        AppId::from_app_did(app_doc_template.app_did())
                            .map_err(RPCErrors::ReasonError)?
                    )
                    .as_str(),
                )
                .await?,
            ),
            None => None,
        };

        let mut prepared_sub_pkgs = Vec::new();
        for scanned in plan.sub_pkgs {
            let payload = self
                .package_source_to_payload(
                    &named_store,
                    &temp_root,
                    &scanned.source,
                    format!(
                        "{}-{}",
                        AppId::from_app_did(app_doc_template.app_did())
                            .map_err(RPCErrors::ReasonError)?,
                        scanned.key
                    )
                    .as_str(),
                )
                .await?;
            let file_object = payload.file_object.ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "Packaged sub package `{}` unexpectedly has no file object",
                    scanned.key
                ))
            })?;
            let tarball_path = payload.tarball_path.clone().ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "Packaged sub package `{}` unexpectedly has no tarball",
                    scanned.key
                ))
            })?;
            let meta = self.build_sub_pkg_meta(app_doc_template, &scanned.desc, file_object)?;
            prepared_sub_pkgs.push(PreparedSubPkg {
                key: scanned.key,
                desc: scanned.desc,
                meta,
                tarball_path,
            });
        }

        Ok(PreparedPublishPlan {
            app_bundle,
            sub_pkgs: prepared_sub_pkgs,
            temp_root,
        })
    }

    async fn store_publish_pkg_metas(
        &self,
        app_doc_template: &AppDoc,
        prepared: &PreparedPublishPlan,
    ) -> Result<(ObjId, AppDoc, Vec<Value>), RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let named_store = runtime.get_named_store().await.map_err(|error| {
            RPCErrors::ReasonError(format!("Open named store for publish failed: {error}"))
        })?;
        info!(
            "opening repo client for publish app `{}` version `{}`",
            app_doc_template.show_name, app_doc_template.version
        );
        let repo = self.repo_client().await?;
        let mut resolved_sub_pkgs = Vec::new();
        let mut meta_values = Vec::new();

        for prepared_sub_pkg in prepared.sub_pkgs.iter() {
            let (meta_obj_id, meta_obj_str) = prepared_sub_pkg.meta.gen_obj_id();
            named_store
                .put_object(&meta_obj_id, meta_obj_str.as_str())
                .await
                .map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Write sub package `{}` metadata object into named store failed: {error}",
                        prepared_sub_pkg.key
                    ))
                })?;
            self.store_meta_object_via_repo(
                &repo,
                &meta_obj_id,
                format!("subpkg-{}", prepared_sub_pkg.key).as_str(),
            )
            .await?;

            meta_values.push(
                serde_json::to_value(&prepared_sub_pkg.meta).map_err(|error| {
                    RPCErrors::ReasonError(format!("Serialize sub package meta failed: {error}"))
                })?,
            );
            resolved_sub_pkgs.push((
                prepared_sub_pkg.key.clone(),
                prepared_sub_pkg.desc.clone(),
                meta_obj_id,
            ));
        }

        let final_doc = Self::build_final_app_doc_for_publish(
            app_doc_template,
            prepared
                .app_bundle
                .as_ref()
                .and_then(|payload| payload.file_object.as_ref()),
            resolved_sub_pkgs.as_slice(),
        )?;

        let final_value = serde_json::to_value(&final_doc).map_err(|error| {
            RPCErrors::ReasonError(format!("Serialize final AppDoc failed: {error}"))
        })?;
        // v0.5 D2：App Document 的 obj type 固定为 `appdoc`（不再复用 pkg）。
        let (final_obj_id, final_obj_str) =
            build_named_object_by_json(buckyos_api::OBJ_TYPE_APP_DOC, &final_value);
        named_store
            .put_object(&final_obj_id, final_obj_str.as_str())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "Write final AppDoc object into named store failed: {error}"
                ))
            })?;
        self.store_meta_object_via_repo(&repo, &final_obj_id, "appdoc")
            .await?;

        Ok((final_obj_id, final_doc, meta_values))
    }

    pub async fn publish_app_to_repo(
        &self,
        app_type: AppType,
        local_dir: &Path,
        app_doc_template: &AppDoc,
    ) -> Result<PublishOutput, RPCErrors> {
        info!(
            "begin publish app `{}` type `{}` from `{}`",
            app_doc_template.show_name,
            app_type,
            local_dir.display()
        );
        let plan = self.scan_publish_sources(app_type, local_dir, app_doc_template)?;
        let prepared = self.package_publish_sources(app_doc_template, plan).await?;
        let store_result = self
            .store_publish_pkg_metas(app_doc_template, &prepared)
            .await;
        let (final_obj_id, final_doc, meta_values) = match store_result {
            Ok(result) => result,
            Err(error) => {
                let _ = fs::remove_dir_all(&prepared.temp_root).await;
                return Err(error);
            }
        };

        // 生成本地 .pikg 并用同一个 PikgReader 自校验（packer/verifier 单实现）。
        let pikg_result = self
            .build_publish_pikg(&prepared, &final_doc, &meta_values)
            .await;
        let _ = fs::remove_dir_all(&prepared.temp_root).await;
        let (pikg_digest, staged_pikg_path) = pikg_result?;

        info!(
            "publish completed for app `{}` version `{}` obj `{}` pikg sha256:{}",
            app_doc_template.show_name, app_doc_template.version, final_obj_id, pikg_digest
        );
        let app_doc_value = serde_json::to_value(&final_doc).map_err(|error| {
            RPCErrors::ReasonError(format!("Serialize final AppDoc failed: {error}"))
        })?;
        Ok(PublishOutput {
            app_did: final_doc.app_did().clone(),
            app_doc_object_id: final_obj_id,
            app_doc_value,
            pikg_handle: format!("{}{}", buckyos_api::PIKG_STAGING_HANDLE_PREFIX, pikg_digest),
            pikg_digest,
            staged_pikg_path,
            publish_status: "repo_stored_candidate",
        })
    }

    /// 打包 pikg -> staging root -> 自校验（结构 + 全部内容 hash）。
    async fn build_publish_pikg(
        &self,
        prepared: &PreparedPublishPlan,
        final_doc: &AppDoc,
        meta_values: &[Value],
    ) -> Result<(String, PathBuf), RPCErrors> {
        let mut builder = crate::pikg::PikgBuilder::new()
            .app_doc(final_doc)
            .map_err(|error| RPCErrors::ReasonError(format!("pikg builder: {error}")))?;
        for meta_value in meta_values {
            let (next, _meta_id) = builder
                .add_package_meta_value(meta_value.clone())
                .map_err(|error| RPCErrors::ReasonError(format!("pikg builder: {error}")))?;
            builder = next;
        }
        for sub_pkg in prepared.sub_pkgs.iter() {
            builder = builder
                .add_payload_file(sub_pkg.key.as_str(), sub_pkg.tarball_path.clone())
                .map_err(|error| RPCErrors::ReasonError(format!("pikg builder: {error}")))?;
        }
        let tmp_pikg = prepared.temp_root.join(format!(
            "{}.pikg",
            AppId::from_app_did(final_doc.app_did()).map_err(RPCErrors::ReasonError)?
        ));
        builder
            .write_to(&tmp_pikg)
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("write pikg failed: {error}")))?;

        let staging_root = crate::app_install_driver::pikg_staging_root();
        let (digest, staged_path) =
            crate::pikg::PikgReader::stage_pikg_file(&tmp_pikg, &staging_root)
                .await
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("stage published pikg failed: {error}"))
                })?;

        // 自校验：packer 与 verifier 不允许两套规则漂移。
        let reader = crate::pikg::PikgReader::open(&staged_path, Some(&digest))
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("published pikg failed self-check: {error}"))
            })?;
        reader.verify_all_contents().await.map_err(|error| {
            RPCErrors::ReasonError(format!("published pikg failed content self-check: {error}"))
        })?;
        Ok((digest, staged_path))
    }

    fn canonical_packaged_name(key: &str, desc: &SubPkgDesc, app_id: &str) -> Option<String> {
        if key.ends_with("docker_image") || desc.docker_image_name.is_some() {
            return Some(format!("{app_id}.tar"));
        }
        None
    }

    fn sanitize_publish_name(name: &str) -> String {
        let mut result = String::with_capacity(name.len());
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                result.push(ch);
            } else {
                result.push('_');
            }
        }
        if result.is_empty() {
            "pkg".to_string()
        } else {
            result
        }
    }

    async fn create_tar_gz(&self, src_dir: &Path, tarball_path: &Path) -> Result<(), RPCErrors> {
        let src_dir = src_dir.to_path_buf();
        let tarball_path = tarball_path.to_path_buf();

        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let tar_gz = StdFile::create(&tarball_path).map_err(|error| {
                format!(
                    "Create archive `{}` failed: {error}",
                    tarball_path.display()
                )
            })?;
            let encoder = GzEncoder::new(tar_gz, Compression::default());
            let mut tar = Builder::new(encoder);

            fn append_dir_all(
                tar: &mut Builder<GzEncoder<StdFile>>,
                path: &Path,
                base: &Path,
            ) -> io::Result<()> {
                for entry in std::fs::read_dir(path)? {
                    let entry = entry?;
                    let path = entry.path();
                    let skip = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.starts_with('.'))
                        .unwrap_or(false);
                    if skip {
                        continue;
                    }

                    let relative = path.strip_prefix(base).unwrap();
                    if path.is_dir() {
                        tar.append_dir(relative, &path)?;
                        append_dir_all(tar, &path, base)?;
                    } else {
                        tar.append_file(relative, &mut StdFile::open(&path)?)?;
                    }
                }
                Ok(())
            }

            append_dir_all(&mut tar, &src_dir, &src_dir).map_err(|error| {
                format!(
                    "Append files from `{}` into archive failed: {error}",
                    src_dir.display()
                )
            })?;
            tar.finish().map_err(|error| {
                format!(
                    "Finalize archive `{}` failed: {error}",
                    tarball_path.display()
                )
            })?;
            Ok(())
        })
        .await
        .map_err(|error| RPCErrors::ReasonError(format!("Create tar.gz join failed: {error}")))?
        .map_err(RPCErrors::ReasonError)
    }

    async fn stage_package_source(
        &self,
        temp_root: &Path,
        source: &PackageSource,
    ) -> Result<PathBuf, RPCErrors> {
        match source {
            PackageSource::Directory(path) => Ok(path.clone()),
            PackageSource::File {
                path,
                packaged_name,
            } => {
                let staging_dir = temp_root.join(format!("stage-{}", Uuid::new_v4()));
                fs::create_dir_all(&staging_dir).await.map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Create staging directory `{}` failed: {error}",
                        staging_dir.display()
                    ))
                })?;
                let file_name = packaged_name.clone().unwrap_or_else(|| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("payload.bin")
                        .to_string()
                });
                fs::copy(path, staging_dir.join(file_name))
                    .await
                    .map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "Copy `{}` into staging directory failed: {error}",
                            path.display()
                        ))
                    })?;
                Ok(staging_dir)
            }
        }
    }

    async fn package_source_to_payload(
        &self,
        named_store: &NamedDataMgr,
        temp_root: &Path,
        source: &PackageSource,
        archive_base_name: &str,
    ) -> Result<PreparedPayload, RPCErrors> {
        let staged_dir = self.stage_package_source(temp_root, source).await?;
        let tarball_path = temp_root.join(format!(
            "{}.tar.gz",
            Self::sanitize_publish_name(archive_base_name)
        ));
        self.create_tar_gz(&staged_dir, &tarball_path).await?;

        let file_template = FileObject::default();
        let (file_object, _file_obj_id, _file_obj_str) = cacl_file_object(
            Some(named_store),
            &tarball_path,
            &file_template,
            true,
            &CheckMode::ByFullHash,
            StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Write packaged archive `{}` into named store failed: {error}",
                tarball_path.display()
            ))
        })?;

        match source {
            PackageSource::Directory(_) => {}
            PackageSource::File { .. } => {
                let _ = fs::remove_dir_all(&staged_dir).await;
            }
        }

        Ok(PreparedPayload {
            file_object: Some(file_object),
            tarball_path: Some(tarball_path),
        })
    }

    fn build_sub_pkg_meta(
        &self,
        app_doc_template: &AppDoc,
        desc: &SubPkgDesc,
        file_object: FileObject,
    ) -> Result<PackageMeta, RPCErrors> {
        let package_id = PackageId::parse(desc.pkg_id.as_str()).map_err(|error| {
            RPCErrors::ReasonError(format!("Invalid sub package id `{}`: {error}", desc.pkg_id))
        })?;

        let version = match package_id.version_exp.as_ref() {
            Some(version_exp) if version_exp.is_version() => version_exp.version_exp.to_string(),
            Some(_) => {
                return Err(RPCErrors::ReasonError(format!(
                    "Sub package `{}` must use an exact version for publish",
                    desc.pkg_id
                )))
            }
            None => app_doc_template.version.clone(),
        };

        let mut meta = PackageMeta::new(
            package_id.name.as_str(),
            version.as_str(),
            app_doc_template.author.to_string().as_str(),
            &app_doc_template.owner,
            None,
        );
        meta.size = file_object.size;
        meta.content = file_object.content.clone();
        meta.exp = app_doc_template.exp;
        meta.last_update_time = buckyos_get_unix_timestamp();
        if let Some(tag) = package_id
            .version_exp
            .as_ref()
            .and_then(|version_exp| version_exp.tag.clone())
        {
            meta.version_tag = Some(tag);
        }

        Ok(meta)
    }

    fn build_final_app_doc_for_publish(
        app_doc_template: &AppDoc,
        _app_bundle: Option<&FileObject>,
        resolved_sub_pkgs: &[(String, SubPkgDesc, ObjId)],
    ) -> Result<AppDoc, RPCErrors> {
        let mut final_doc = app_doc_template.clone();

        for (key, desc, meta_obj_id) in resolved_sub_pkgs {
            let mut updated_desc = desc.clone();
            updated_desc.pkg_objid = Some(meta_obj_id.clone());
            updated_desc.source_url = None;
            Self::set_sub_pkg_desc(&mut final_doc, key.as_str(), updated_desc)?;
        }

        final_doc.last_update_time = buckyos_get_unix_timestamp();
        final_doc.validate()?;
        Ok(final_doc)
    }

    async fn store_meta_object_via_repo(
        &self,
        repo: &RepoClient,
        meta_obj_id: &ObjId,
        label: &str,
    ) -> Result<(), RPCErrors> {
        info!(
            "calling repo.store for publish object `{}` obj `{}`",
            label, meta_obj_id
        );
        let stored_id = repo
            .store(&meta_obj_id.to_string())
            .await
            .map_err(|error| {
                warn!(
                    "repo.store failed for publish object `{}` obj `{}`: {}",
                    label, meta_obj_id, error
                );
                error
            })?;
        if stored_id != *meta_obj_id {
            warn!(
                "repo.store returned unexpected obj id for `{}`: expected {}, got {}",
                label, meta_obj_id, stored_id
            );
            return Err(RPCErrors::ReasonError(format!(
                "repo.store returned unexpected obj id for `{}`: expected {}, got {}",
                label, meta_obj_id, stored_id
            )));
        }
        info!(
            "repo.store succeeded for publish object `{}` obj `{}`",
            label, meta_obj_id
        );
        Ok(())
    }

    fn set_sub_pkg_desc(
        app_doc: &mut AppDoc,
        key: &str,
        desc: SubPkgDesc,
    ) -> Result<(), RPCErrors> {
        match key {
            "amd64_docker_image" => app_doc.pkg_list.amd64_docker_image = Some(desc),
            "aarch64_docker_image" => app_doc.pkg_list.aarch64_docker_image = Some(desc),
            "amd64_win_app" => app_doc.pkg_list.amd64_win_app = Some(desc),
            "aarch64_win_app" => app_doc.pkg_list.aarch64_win_app = Some(desc),
            "aarch64_apple_app" => app_doc.pkg_list.aarch64_apple_app = Some(desc),
            "amd64_apple_app" => app_doc.pkg_list.amd64_apple_app = Some(desc),
            "script" => app_doc.pkg_list.script = Some(desc),
            "web" => app_doc.pkg_list.web = Some(desc),
            "agent" => app_doc.pkg_list.agent = Some(desc),
            "agent_skills" => app_doc.pkg_list.agent_skills = Some(desc),
            "agent_tools" => app_doc.pkg_list.agent_tools = Some(desc),
            other => {
                app_doc.pkg_list.others.insert(other.to_string(), desc);
            }
        }
        Ok(())
    }
}

use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCRequest, RPCResponse, RPCResult};

#[derive(Debug, Clone)]
pub(crate) struct PreInstallSubmitOutcome {
    pub action: String,
    pub task_id: Option<String>,
    pub app_instance_id: buckyos_api::AppInstanceId,
    pub plan_fingerprint: String,
}

impl ControlPanelServer {
    pub(crate) fn resolve_target_user_id(req: &RPCRequest, principal: &RpcAuthPrincipal) -> String {
        Self::param_str(req, "owner_user_id").unwrap_or_else(|| principal.owner_user_id.clone())
    }

    fn requested_app_owner(
        req: &RPCRequest,
        principal: &RpcAuthPrincipal,
        selector: &str,
    ) -> String {
        Self::param_str(req, "owner_user_id")
            .or_else(|| {
                selector
                    .parse::<AppInstanceId>()
                    .ok()
                    .map(|value| value.owner_user_id().to_string())
            })
            .unwrap_or_else(|| principal.owner_user_id.clone())
    }

    fn parse_app_type(raw: &str) -> Result<AppType, RPCErrors> {
        AppType::try_from(raw.trim()).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid app_type `{}`: {}", raw, error))
        })
    }

    pub(crate) async fn handle_app_publish(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let _principal = Self::require_rpc_principal(principal)?;
        let local_dir = Self::require_param_str(&req, "local_dir")
            .or_else(|_| Self::require_param_str(&req, "path"))?;
        let app_doc_value = req
            .params
            .get("app_doc")
            .cloned()
            .or_else(|| req.params.get("app_doc_template").cloned())
            .ok_or_else(|| RPCErrors::ReasonError("missing app_doc payload".to_string()))?;
        let app_doc: AppDoc = serde_json::from_value(app_doc_value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid app_doc payload: {}", error))
        })?;
        let app_type = Self::param_str(&req, "app_type")
            .map(|raw| Self::parse_app_type(raw.as_str()))
            .transpose()?
            .unwrap_or_else(|| app_doc.get_app_type());

        info!(
            "rpc app.publish app=`{}` version=`{}` type=`{}` local_dir=`{}`",
            app_doc.show_name, app_doc.version, app_type, local_dir
        );
        let output = self
            .app_installer
            .publish_app_to_repo(app_type, std::path::Path::new(local_dir.as_str()), &app_doc)
            .await
            .map_err(|error| {
                warn!(
                    "rpc app.publish failed for app `{}` version `{}` local_dir `{}`: {}",
                    app_doc.show_name, app_doc.version, local_dir, error
                );
                error
            })?;

        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({
                "ok": true,
                // 兼容字段：即 app_doc_id。
                "obj_id": output.app_doc_object_id.to_string(),
                "app_did": output.app_did.to_string(),
                "app_doc_id": output.app_doc_object_id.to_string(),
                "pikg_handle": output.pikg_handle,
                "pikg_digest": output.pikg_digest,
                "pikg_path": output.staged_pikg_path.to_string_lossy(),
                "app_doc": output.app_doc_value,
                // 发布到 Repo/NamedStore ≠ 发布 App DID：仍是未权威发布的 candidate。
                "publish_status": output.publish_status,
            })),
            req.seq,
        ))
    }

    fn parse_install_policy(
        principal: &RpcAuthPrincipal,
        options: Option<&Value>,
    ) -> Result<InstallPolicy, RPCErrors> {
        let raw = options
            .and_then(|value| value.get("policy"))
            .and_then(|value| value.as_str())
            .unwrap_or("NORMAL");
        let policy = match raw {
            "STRICT_PUBLIC" => InstallPolicy::StrictPublic,
            "NORMAL" => InstallPolicy::Normal,
            "TRUSTED_SHARE" => InstallPolicy::TrustedShare,
            "LOCAL_DEVELOPER" => InstallPolicy::LocalDeveloper,
            "SYSTEM_INTERNAL" => InstallPolicy::SystemInternal,
            other => {
                return Err(RPCErrors::ParseRequestError(format!(
                    "unknown install policy `{other}`"
                )))
            }
        };
        // SYSTEM_INTERNAL（可 auto-confirm）只允许 admin/root 使用。
        if matches!(policy, InstallPolicy::SystemInternal) && !Self::principal_is_admin(principal) {
            return Err(RPCErrors::ReasonError(
                "SYSTEM_INTERNAL install policy requires admin privileges".to_string(),
            ));
        }
        Ok(policy)
    }

    fn principal_is_admin(principal: &RpcAuthPrincipal) -> bool {
        matches!(
            principal.user_type,
            crate::UserType::Admin | crate::UserType::Root
        )
    }

    fn require_install_scope(
        principal: &RpcAuthPrincipal,
        target_user: &str,
    ) -> Result<(), RPCErrors> {
        if principal.username == target_user || Self::principal_is_admin(principal) {
            Ok(())
        } else {
            Err(RPCErrors::ReasonError(
                "only admin can install for another user".to_string(),
            ))
        }
    }

    fn require_app_lifecycle_scope(
        principal: &RpcAuthPrincipal,
        spec: &AppServiceSpec,
    ) -> Result<(), RPCErrors> {
        if spec.owner_user_id == principal.username || Self::principal_is_admin(principal) {
            Ok(())
        } else {
            Err(RPCErrors::NoPermission(
                "only the app owner or an admin can manage this app".to_string(),
            ))
        }
    }

    fn install_error_to_rpc(error: buckyos_api::InstallError) -> RPCErrors {
        RPCErrors::ReasonError(serde_json::to_string(&error).unwrap_or_else(|_| error.to_string()))
    }

    fn parse_install_source(req: &RPCRequest) -> Result<buckyos_api::InstallSource, RPCErrors> {
        if let Some(value) = req.params.get("source") {
            return serde_json::from_value(value.clone())
                .map_err(|error| RPCErrors::ParseRequestError(format!("invalid source: {error}")));
        }
        if let Some(staging_handle) = Self::param_str(req, "staging_handle") {
            return Ok(buckyos_api::InstallSource::local_pikg(staging_handle));
        }
        let identifier = Self::require_param_str(req, "identifier")?;
        Ok(buckyos_api::InstallSource::identifier(
            identifier,
            Self::param_str(req, "referrer"),
        ))
    }

    fn merged_install_options(req: &RPCRequest) -> Result<Value, RPCErrors> {
        let mut options = req
            .params
            .get("options")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let options_map = options
            .as_object_mut()
            .ok_or_else(|| RPCErrors::ParseRequestError("options must be an object".to_string()))?;
        if let Some(target) = req.params.get("target") {
            options_map.insert("target".to_string(), target.clone());
        }
        if let Some(install_params) = req.params.get("install_params") {
            options_map.insert("install_params".to_string(), install_params.clone());
        }
        Ok(options)
    }

    pub(crate) async fn handle_apps_staging_finalize(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let source_raw = Self::require_param_str(&req, "source_obj_id")?;
        let source = ObjId::new(&source_raw).map_err(|error| {
            RPCErrors::ParseRequestError(format!("invalid source_obj_id: {error}"))
        })?;
        let purpose = match Self::param_str(&req, "purpose").as_deref() {
            None | Some("inspect") => buckyos_api::PikgStagingPurpose::Inspect,
            Some("install") => buckyos_api::PikgStagingPurpose::Install,
            Some(other) => {
                return Err(RPCErrors::ParseRequestError(format!(
                    "invalid staging purpose `{other}`"
                )))
            }
        };
        let runtime = get_buckyos_api_runtime()?;
        let metadata = self
            .staging_store
            .finalize_named_object(
                &source,
                principal.username.as_str(),
                principal.authenticated_app_id.as_str(),
                &runtime.zone_id,
                purpose,
            )
            .await?;
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(metadata).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize staging metadata failed: {error}"))
            })?),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_staging_status(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let handle = Self::require_param_str(&req, "staging_handle")?;
        let runtime = get_buckyos_api_runtime()?;
        let metadata = self
            .staging_store
            .status(
                handle.as_str(),
                principal.username.as_str(),
                principal.authenticated_app_id.as_str(),
                &runtime.zone_id,
            )
            .await?;
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(metadata).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize staging metadata failed: {error}"))
            })?),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_staging_release(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let handle = Self::require_param_str(&req, "staging_handle")?;
        let runtime = get_buckyos_api_runtime()?;
        let metadata = self
            .staging_store
            .release(
                handle.as_str(),
                principal.username.as_str(),
                principal.authenticated_app_id.as_str(),
                &runtime.zone_id,
                None,
            )
            .await?;
        self.staging_store.gc().await?;
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(metadata).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize staging metadata failed: {error}"))
            })?),
            req.seq,
        ))
    }

    async fn inspect_from_rpc(
        &self,
        req: &RPCRequest,
        principal: &RpcAuthPrincipal,
        task_type: &str,
        planning_task_id: Option<String>,
    ) -> Result<buckyos_api::InstallInspection, RPCErrors> {
        let source = Self::parse_install_source(req)?;
        let owner_user_id = Self::resolve_target_user_id(req, principal);
        Self::require_install_scope(principal, owner_user_id.as_str())?;
        let options = Self::merged_install_options(req)?;
        let policy = Self::parse_install_policy(principal, Some(&options))?;
        self.install_engine
            .inspect_request(
                buckyos_api::AppInstallTaskRequest {
                    source,
                    creator_user_id: principal.username.clone(),
                    creator_app_id: principal.authenticated_app_id.clone(),
                    owner_user_id,
                    idempotency_key: "inspect-only".to_string(),
                    submitted_plan: None,
                    approved_plan_fingerprint: None,
                    policy,
                    options: Some(options),
                },
                task_type,
                planning_task_id,
            )
            .await
            .map_err(Self::install_error_to_rpc)
    }

    /// apps.inspect: side-effect-free default plan + dynamic inspection.
    pub(crate) async fn handle_apps_inspect(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let task_type = if Self::param_str(&req, "action").as_deref() == Some("upgrade") {
            buckyos_api::TASK_DATA_TYPE_APP_UPDATE
        } else {
            buckyos_api::TASK_DATA_TYPE_APP_INSTALL
        };
        let inspection = self
            .inspect_from_rpc(&req, principal, task_type, None)
            .await?;
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(inspection).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize inspection failed: {error}"))
            })?),
            req.seq,
        ))
    }

    /// apps.plan.recompute: recompute target/params on the authoritative source.
    pub(crate) async fn handle_apps_plan_recompute(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let previous: buckyos_api::InstallPlan = serde_json::from_value(
            req.params
                .get("plan")
                .cloned()
                .ok_or_else(|| RPCErrors::ParseRequestError("plan is required".to_string()))?,
        )
        .map_err(|error| RPCErrors::ParseRequestError(format!("invalid plan: {error}")))?;
        if previous.schema_version != buckyos_api::APP_INSTALL_SCHEMA_VERSION
            || !previous.fingerprint_is_valid()
        {
            return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                buckyos_api::InstallStage::Inspect,
                buckyos_api::InstallErrorCode::PlanStale,
                false,
                "plan schema or fingerprint is invalid",
            )));
        }
        let task_type = if previous.plan_use == buckyos_api::InstallPlanUse::Upgrade {
            buckyos_api::TASK_DATA_TYPE_APP_UPDATE
        } else {
            buckyos_api::TASK_DATA_TYPE_APP_INSTALL
        };
        let inspection = self
            .inspect_from_rpc(&req, principal, task_type, Some(previous.task_id.clone()))
            .await?;
        if inspection.plan.app.did != previous.app.did
            || inspection.plan.app_instance_id != previous.app_instance_id
            || inspection.plan.source_identity != previous.source_identity
        {
            return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                buckyos_api::InstallStage::Inspect,
                buckyos_api::InstallErrorCode::PlanStale,
                false,
                "source identity or installation scope changed while recomputing plan",
            )));
        }
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(inspection).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize inspection failed: {error}"))
            })?),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_install_status(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let task_id = Self::parse_task_id(&req)?;
        let status = self
            .install_engine
            .status(
                task_id.as_str(),
                principal.username.as_str(),
                Self::principal_is_admin(principal),
            )
            .await
            .map_err(Self::install_error_to_rpc)?;
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(status).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize install status failed: {error}"))
            })?),
            req.seq,
        ))
    }

    async fn update_availability_and_inspection_for_spec(
        &self,
        spec: &AppServiceSpec,
        principal: &RpcAuthPrincipal,
    ) -> Result<(AppUpdateAvailability, buckyos_api::InstallInspection), RPCErrors> {
        Self::require_app_lifecycle_scope(principal, spec)?;
        let inspection = self
            .install_engine
            .inspect_request(
                buckyos_api::AppInstallTaskRequest {
                    source: buckyos_api::InstallSource::identifier(spec.app_did.to_string(), None),
                    creator_user_id: principal.username.clone(),
                    creator_app_id: principal.authenticated_app_id.clone(),
                    owner_user_id: spec.owner_user_id.clone(),
                    idempotency_key: format!("availability:{}", spec.app_instance_id),
                    submitted_plan: None,
                    approved_plan_fingerprint: None,
                    policy: InstallPolicy::Normal,
                    options: None,
                },
                buckyos_api::TASK_DATA_TYPE_APP_UPDATE,
                None,
            )
            .await
            .map_err(Self::install_error_to_rpc)?;
        let plan = &inspection.plan;
        let permissions_added = plan
            .install_params
            .permissions
            .iter()
            .filter(|permission| {
                !spec.permission.iter().any(|installed| {
                    installed.scope_path == permission.scope_path
                        && installed.actions == permission.actions
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let target_compatible =
            inspection.status.readiness.target == buckyos_api::ReadinessState::Ready;
        let state = if inspection.resolution_status.document_status.is_terminal() {
            AppUpdateState::IdentityRevoked
        } else if !inspection
            .resolution_status
            .is_trust_ready(InstallPolicy::Normal)
        {
            AppUpdateState::TrustResolutionRequired
        } else if !target_compatible {
            AppUpdateState::IncompatibleTarget
        } else if !permissions_added.is_empty() {
            AppUpdateState::PermissionReconfirmRequired
        } else if spec.deployment.app_doc_object_id == plan.app.object_id {
            AppUpdateState::UpToDate
        } else {
            AppUpdateState::UpdateAvailable
        };
        let record_key =
            buckyos_api::install_record_key(spec.owner_user_id.as_str(), spec.app_id());
        let installed_document_version = match self
            .app_installer
            .system_config_client()
            .await?
            .get(&record_key)
            .await
        {
            Ok(value) => serde_json::from_str::<buckyos_api::InstallRecord>(&value.value)
                .ok()
                .and_then(|record| record.resolution.document_version),
            Err(_) => None,
        };
        let availability = AppUpdateAvailability {
            app_did: spec.app_did.clone(),
            app_instance_id: spec.app_instance_id.clone(),
            state,
            installed_app_doc_id: Some(spec.deployment.app_doc_object_id.clone()),
            resolved_app_doc_id: Some(plan.app.object_id.clone()),
            installed_document_version,
            resolved_document_version: plan.resolution.document_version,
            installed_version: Some(spec.app_doc.version.clone()),
            resolved_version: Some(plan.app.version.clone()),
            did_resolution: Some(inspection.resolution_status.clone()),
            permissions_added,
            target_compatible: Some(target_compatible),
            checked_at: buckyos_get_unix_timestamp(),
        };
        Ok((availability, inspection))
    }

    async fn update_availability_for_spec(
        &self,
        spec: &AppServiceSpec,
        principal: &RpcAuthPrincipal,
    ) -> Result<AppUpdateAvailability, RPCErrors> {
        self.update_availability_and_inspection_for_spec(spec, principal)
            .await
            .map(|(availability, _)| availability)
    }

    pub(crate) async fn handle_apps_update_availability(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let is_batch = req.params.get("selector").is_none()
            && req.params.get("app_instance_id").is_none()
            && req.params.get("app_did").is_none()
            && req.params.get("identifier").is_none();
        let mut results = Vec::new();
        if is_batch {
            let user_id = Self::param_str(&req, "owner_user_id")
                .unwrap_or_else(|| principal.owner_user_id.clone());
            Self::require_install_scope(principal, user_id.as_str())?;
            for (installation, _) in Self::app_availability_resolver()
                .await?
                .list_user_installations(user_id.as_str())
                .await?
            {
                if Self::require_app_lifecycle_scope(principal, &installation.spec).is_ok() {
                    match self
                        .update_availability_for_spec(&installation.spec, principal)
                        .await
                    {
                        Ok(result) => {
                            results.push(serde_json::to_value(result).unwrap_or(Value::Null))
                        }
                        Err(error) => results.push(json!({
                            "app_instance_id": installation.spec.app_instance_id,
                            "state": "UNKNOWN",
                            "error": error.to_string(),
                        })),
                    }
                }
            }
        } else {
            let installation = self.resolve_app_selector(&req, principal).await?;
            results.push(
                serde_json::to_value(
                    self.update_availability_for_spec(&installation.spec, principal)
                        .await?,
                )
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
            );
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "batch": is_batch,
                "total": results.len(),
                "items": results,
            })),
            req.seq,
        ))
    }

    async fn find_update_batch_replay(
        &self,
        principal: &RpcAuthPrincipal,
        idempotency_key: &str,
    ) -> Result<Option<buckyos_api::Task>, RPCErrors> {
        let client = self.app_installer.task_mgr_client().await?;
        let page = client
            .list_tasks(buckyos_api::ListTasksReq {
                creator_user_id: Some(principal.username.clone()),
                creator_app_id: Some(principal.authenticated_app_id.clone()),
                idempotency_key: Some(idempotency_key.to_string()),
                schema_id: Some(APP_UPDATE_BATCH_TASK_SCHEMA_ID.to_string()),
                include_archived: true,
                limit: Some(1),
                ..Default::default()
            })
            .await?;
        match page.tasks.first() {
            Some(summary) => Ok(Some(client.get_task(&summary.task_id).await?)),
            None => Ok(None),
        }
    }

    /// apps.upgrade with no selector creates a recoverable Catalog batch root.
    pub(crate) async fn handle_apps_upgrade_batch(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let idempotency_key = Self::require_param_str(&req, "idempotency_key")?;
        let owner_user_id = Self::param_str(&req, "owner_user_id")
            .unwrap_or_else(|| principal.owner_user_id.clone());
        Self::require_install_scope(principal, owner_user_id.as_str())?;

        if let Some(task) = self
            .find_update_batch_replay(principal, idempotency_key.as_str())
            .await?
        {
            let previous: AppUpdateBatchTaskData = serde_json::from_value(task.input.clone())
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("invalid update batch task input: {error}"))
                })?;
            if previous.request.owner_user_id != owner_user_id {
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::IdempotencyConflict,
                    false,
                    "idempotency key was already used for another batch scope",
                )));
            }
            self.spawn_update_batch_task(task.task_id.clone());
            return Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "action": "replay",
                    "task_id": task.task_id,
                    "root_id": task.root_id,
                    "total": previous.request.items.len(),
                })),
                req.seq,
            ));
        }

        let mut items = Vec::new();
        for (installation, _) in Self::app_availability_resolver()
            .await?
            .list_user_installations(owner_user_id.as_str())
            .await?
        {
            let spec = installation.spec;
            if Self::require_app_lifecycle_scope(principal, &spec).is_err() {
                continue;
            }
            let source = buckyos_api::InstallSource::identifier(spec.app_did.to_string(), None);
            let (availability, submitted_plan, approved_plan_fingerprint) = match self
                .update_availability_and_inspection_for_spec(&spec, principal)
                .await
            {
                Ok((availability, inspection)) => {
                    let submitted_plan = (availability.state == AppUpdateState::UpdateAvailable)
                        .then_some(inspection.plan);
                    let fingerprint = submitted_plan
                        .as_ref()
                        .map(|plan| plan.plan_fingerprint.clone());
                    (availability, submitted_plan, fingerprint)
                }
                Err(_error) => (
                    AppUpdateAvailability {
                        app_did: spec.app_did.clone(),
                        app_instance_id: spec.app_instance_id.clone(),
                        state: AppUpdateState::Unknown,
                        installed_app_doc_id: Some(spec.deployment.app_doc_object_id.clone()),
                        resolved_app_doc_id: None,
                        installed_document_version: None,
                        resolved_document_version: None,
                        installed_version: Some(spec.app_doc.version.clone()),
                        resolved_version: None,
                        did_resolution: None,
                        permissions_added: Vec::new(),
                        target_compatible: None,
                        checked_at: buckyos_get_unix_timestamp(),
                    },
                    None,
                    None,
                ),
            };
            items.push(AppUpdateBatchRequestItem {
                app_id: spec.app_id().clone(),
                app_instance_id: spec.app_instance_id,
                source,
                availability,
                submitted_plan,
                approved_plan_fingerprint,
            });
        }
        items.sort_by(|left, right| left.app_instance_id.cmp(&right.app_instance_id));

        let data = AppUpdateBatchTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request: AppUpdateBatchTaskRequest {
                creator_user_id: principal.username.clone(),
                creator_app_id: principal.authenticated_app_id.clone(),
                owner_user_id,
                idempotency_key: idempotency_key.clone(),
                items,
            },
            progress: None,
            result: None,
            error: None,
        };
        let total = data.request.items.len();
        let update_count = data
            .request
            .items
            .iter()
            .filter(|item| item.availability.state == AppUpdateState::UpdateAvailable)
            .count();
        let task = self
            .app_installer
            .task_mgr_client()
            .await?
            .create_delegated_task(buckyos_api::CreateDelegatedTaskReq {
                task_id: None,
                name: format!("Upgrade {} Catalog apps", update_count),
                schema_id: APP_UPDATE_BATCH_TASK_SCHEMA_ID.to_string(),
                schema_version: None,
                input: task_data_value(data)?,
                creator: buckyos_api::ActorRef::new(
                    principal.username.clone(),
                    principal.authenticated_app_id.clone(),
                ),
                runner_app_instance_id: None,
                parent_id: None,
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                idempotency_key,
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await?;
        self.spawn_update_batch_task(task.task_id.clone());
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "action": "batch_upgrade",
                "task_id": task.task_id,
                "root_id": task.root_id,
                "total": total,
                "update_count": update_count,
            })),
            req.seq,
        ))
    }

    fn batch_item_result(
        item: &AppUpdateBatchRequestItem,
        outcome: AppUpdateBatchItemOutcome,
        child_task_id: Option<String>,
        error: Option<String>,
    ) -> AppUpdateBatchItemResult {
        AppUpdateBatchItemResult {
            app_instance_id: item.app_instance_id.clone(),
            outcome,
            child_task_id,
            error,
        }
    }

    async fn report_update_batch_progress(
        &self,
        task_id: &str,
        results: &[AppUpdateBatchItemResult],
    ) -> Result<(), RPCErrors> {
        let completed_items = results
            .iter()
            .filter(|item| item.outcome != AppUpdateBatchItemOutcome::Pending)
            .count() as u32;
        self.app_installer
            .task_mgr_client()
            .await?
            .runner_progress(
                task_id,
                Some(
                    serde_json::to_value(AppUpdateBatchProgress {
                        completed_items,
                        total_items: results.len() as u32,
                        items: results.to_vec(),
                        updated_at: buckyos_get_unix_timestamp(),
                    })
                    .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
                ),
                Some(format!(
                    "Catalog batch: {completed_items}/{} items complete",
                    results.len()
                )),
            )
            .await?;
        Ok(())
    }

    async fn run_update_batch_task(&self, task_id: &str) -> Result<(), RPCErrors> {
        let task_mgr = self.app_installer.task_mgr_client().await?;
        let task = task_mgr.get_task(task_id).await?;
        if task.phase.is_terminal() {
            return Ok(());
        }
        let data: AppUpdateBatchTaskData = serde_json::from_value(task.input).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid update batch task input: {error}"))
        })?;
        if data.schema_version != APP_INSTALL_SCHEMA_VERSION {
            return Err(RPCErrors::ReasonError(
                "unsupported update batch schema version".to_string(),
            ));
        }
        task_mgr.runner_start(task_id).await?;

        let mut results = Vec::with_capacity(data.request.items.len());
        for item in &data.request.items {
            match item.availability.state {
                AppUpdateState::UpToDate => results.push(Self::batch_item_result(
                    item,
                    AppUpdateBatchItemOutcome::Satisfied,
                    None,
                    None,
                )),
                AppUpdateState::UpdateAvailable => {
                    let Some(fingerprint) = item.approved_plan_fingerprint.clone() else {
                        results.push(Self::batch_item_result(
                            item,
                            AppUpdateBatchItemOutcome::Failed,
                            None,
                            Some("update item has no frozen plan fingerprint".to_string()),
                        ));
                        continue;
                    };
                    let Some(submitted_plan) = item.submitted_plan.clone() else {
                        results.push(Self::batch_item_result(
                            item,
                            AppUpdateBatchItemOutcome::Failed,
                            None,
                            Some("update item has no frozen Upgrade plan".to_string()),
                        ));
                        continue;
                    };
                    let child_idempotency_key =
                        format!("{}:{}", data.request.idempotency_key, item.app_instance_id);
                    let mutation_key = match Self::acquire_app_mutation(
                        &item.app_instance_id,
                        data.request.creator_user_id.as_str(),
                        data.request.creator_app_id.as_str(),
                        child_idempotency_key.as_str(),
                    )
                    .await
                    {
                        Ok(key) => key,
                        Err(error) => {
                            results.push(Self::batch_item_result(
                                item,
                                AppUpdateBatchItemOutcome::Failed,
                                None,
                                Some(error.to_string()),
                            ));
                            continue;
                        }
                    };
                    let child = self
                        .install_engine
                        .create_update_task_with_parent(
                            buckyos_api::AppUpdateTaskRequest {
                                source: item.source.clone(),
                                creator_user_id: data.request.creator_user_id.clone(),
                                creator_app_id: data.request.creator_app_id.clone(),
                                owner_user_id: item.app_instance_id.owner_user_id().to_string(),
                                idempotency_key: child_idempotency_key,
                                submitted_plan: Some(submitted_plan),
                                approved_plan_fingerprint: Some(fingerprint),
                                policy: InstallPolicy::Normal,
                                options: None,
                            },
                            item.app_id.as_str(),
                            Some(task_id),
                        )
                        .await;
                    let child_task_id = match child {
                        Ok(child_task_id) => child_task_id,
                        Err(error) => {
                            Self::release_app_mutation_key(&mutation_key).await;
                            results.push(Self::batch_item_result(
                                item,
                                AppUpdateBatchItemOutcome::Failed,
                                None,
                                Some(error.to_string()),
                            ));
                            continue;
                        }
                    };
                    Self::bind_app_mutation_task(&mutation_key, child_task_id.as_str()).await?;
                    self.install_runner.spawn_run(child_task_id.clone());
                    results.push(Self::batch_item_result(
                        item,
                        AppUpdateBatchItemOutcome::Pending,
                        Some(child_task_id),
                        None,
                    ));
                }
                state => results.push(Self::batch_item_result(
                    item,
                    AppUpdateBatchItemOutcome::Blocked,
                    None,
                    Some(format!("Catalog update is blocked by {state:?}")),
                )),
            }
        }
        self.report_update_batch_progress(task_id, &results).await?;

        loop {
            let root = task_mgr.get_task(task_id).await?;
            if root.phase.is_terminal() {
                return Ok(());
            }
            let mut pending = false;
            let mut changed = false;
            for result in &mut results {
                if result.outcome != AppUpdateBatchItemOutcome::Pending {
                    continue;
                }
                let Some(child_task_id) = result.child_task_id.as_deref() else {
                    result.outcome = AppUpdateBatchItemOutcome::Failed;
                    result.error = Some("batch child id was not persisted".to_string());
                    changed = true;
                    continue;
                };
                let child = task_mgr.get_task(child_task_id).await?;
                if !child.phase.is_terminal() {
                    pending = true;
                    continue;
                }
                result.outcome = match child.outcome {
                    Some(buckyos_api::TaskOutcome::Succeeded) => {
                        AppUpdateBatchItemOutcome::Succeeded
                    }
                    Some(buckyos_api::TaskOutcome::Canceled) => AppUpdateBatchItemOutcome::Canceled,
                    _ => AppUpdateBatchItemOutcome::Failed,
                };
                result.error = child.error.map(|error| error.message);
                changed = true;
            }
            if changed {
                self.report_update_batch_progress(task_id, &results).await?;
            }
            if !pending
                && results
                    .iter()
                    .all(|item| item.outcome != AppUpdateBatchItemOutcome::Pending)
            {
                break;
            }
            sleep(Duration::from_secs(1)).await;
        }

        let count = |outcome| {
            results
                .iter()
                .filter(|item| item.outcome == outcome)
                .count() as u32
        };
        task_mgr
            .runner_complete(
                task_id,
                serde_json::to_value(AppUpdateBatchTaskResult {
                    succeeded: count(AppUpdateBatchItemOutcome::Succeeded),
                    failed: count(AppUpdateBatchItemOutcome::Failed)
                        + count(AppUpdateBatchItemOutcome::Canceled),
                    satisfied: count(AppUpdateBatchItemOutcome::Satisfied),
                    blocked: count(AppUpdateBatchItemOutcome::Blocked),
                    items: results,
                    completed_at: buckyos_get_unix_timestamp(),
                })
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
            )
            .await?;
        Ok(())
    }

    fn spawn_update_batch_task(&self, task_id: String) {
        let server = self.clone();
        tokio::spawn(async move {
            {
                let mut running = server.running_update_batch_tasks.lock().await;
                if !running.insert(task_id.clone()) {
                    return;
                }
            }
            if let Err(error) = server.run_update_batch_task(&task_id).await {
                warn!(
                    "update batch task {} paused for recovery after error: {}",
                    task_id, error
                );
            }
            server
                .running_update_batch_tasks
                .lock()
                .await
                .remove(&task_id);
        });
    }

    pub(crate) fn start_update_batch_runner(&self) {
        let server = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let Ok(client) = server.app_installer.task_mgr_client().await else {
                    continue;
                };
                let mut cursor = None;
                loop {
                    let Ok(page) = client
                        .list_tasks(buckyos_api::ListTasksReq {
                            schema_id: Some(APP_UPDATE_BATCH_TASK_SCHEMA_ID.to_string()),
                            runner_app_id: Some(
                                buckyos_api::CONTROL_PANEL_SERVICE_NAME.to_string(),
                            ),
                            cursor: cursor.clone(),
                            limit: Some(100),
                            ..Default::default()
                        })
                        .await
                    else {
                        break;
                    };
                    for task in page.tasks {
                        if matches!(task.phase, TaskPhase::Accepted | TaskPhase::Running) {
                            server.spawn_update_batch_task(task.task_id);
                        }
                    }
                    cursor = page.next_cursor;
                    if cursor.is_none() {
                        break;
                    }
                }
            }
        });
    }

    async fn find_spec_for_inspection(
        inspection: &buckyos_api::InstallInspection,
    ) -> Result<Option<AppServiceSpec>, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let plan = &inspection.plan;
        let key = buckyos_api::user_app_spec_key(
            plan.app_instance_id.owner_user_id(),
            plan.app_instance_id.app_id(),
        );
        for key in [key] {
            match client.get(&key).await {
                Ok(value) => {
                    let spec: AppServiceSpec =
                        serde_json::from_str(&value.value).map_err(|error| {
                            RPCErrors::ReasonError(format!(
                                "invalid installed spec `{key}`: {error}"
                            ))
                        })?;
                    if spec.app_instance_id != plan.app_instance_id
                        || spec.app_did != plan.app.did
                        || spec.owner_user_id != plan.owner_user_id
                    {
                        return Err(RPCErrors::ReasonError(format!(
                            "installed spec `{key}` does not match installation identity"
                        )));
                    }
                    return Ok(Some(spec));
                }
                Err(SystemConfigError::KeyNotFound(_)) => {}
                Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
            }
        }
        Ok(None)
    }

    async fn acquire_app_mutation(
        app_instance_id: &buckyos_api::AppInstanceId,
        creator_user_id: &str,
        creator_app_id: &str,
        idempotency_key: &str,
    ) -> Result<String, RPCErrors> {
        let key = format!("services/control_panel/app_mutations/{app_instance_id}");
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let now = buckyos_get_unix_timestamp();
        let value = serde_json::json!({
            "schema_version": buckyos_api::APP_INSTALL_SCHEMA_VERSION,
            "app_instance_id": app_instance_id,
            "creator_user_id": creator_user_id,
            "creator_app_id": creator_app_id,
            "idempotency_key": idempotency_key,
            "task_id": null,
            "created_at": now,
            "expires_at": now + 24 * 60 * 60,
        })
        .to_string();
        let mut actions = HashMap::new();
        actions.insert(key.clone(), KVAction::Create(value.clone()));
        if client.exec_tx(actions, None).await.is_ok() {
            return Ok(key);
        }
        let current = client.get(&key).await.map_err(|error| {
            RPCErrors::ReasonError(format!("read app mutation owner failed: {error}"))
        })?;
        let current_value: Value = serde_json::from_str(&current.value).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid app mutation owner: {error}"))
        })?;
        let same_request = current_value.get("creator_user_id").and_then(Value::as_str)
            == Some(creator_user_id)
            && current_value.get("creator_app_id").and_then(Value::as_str) == Some(creator_app_id)
            && current_value.get("idempotency_key").and_then(Value::as_str)
                == Some(idempotency_key);
        if same_request {
            return Ok(key);
        }
        let expired = current_value
            .get("expires_at")
            .and_then(Value::as_u64)
            .map(|expires_at| expires_at <= now)
            .unwrap_or(false);
        if expired {
            let mut actions = HashMap::new();
            actions.insert(key.clone(), KVAction::Update(value));
            if client
                .exec_tx(actions, Some((key.clone(), current.version)))
                .await
                .is_ok()
            {
                return Ok(key);
            }
        }
        Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
            buckyos_api::InstallStage::Inspect,
            buckyos_api::InstallErrorCode::AppMutationInProgress,
            true,
            "another mutation owns this App installation",
        )))
    }

    async fn bind_app_mutation_task(mutation_key: &str, task_id: &str) -> Result<(), RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let current = client.get(mutation_key).await.map_err(|error| {
            RPCErrors::ReasonError(format!("read app mutation owner failed: {error}"))
        })?;
        let mut value: Value = serde_json::from_str(&current.value).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid app mutation owner: {error}"))
        })?;
        value["task_id"] = Value::String(task_id.to_string());
        let mut actions = HashMap::new();
        actions.insert(
            mutation_key.to_string(),
            KVAction::Update(value.to_string()),
        );
        client
            .exec_tx(actions, Some((mutation_key.to_string(), current.version)))
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("bind app mutation task failed: {error}"))
            })?;
        Ok(())
    }

    async fn release_app_mutation_key(mutation_key: &str) {
        let Ok(runtime) = get_buckyos_api_runtime() else {
            return;
        };
        let Ok(client) = runtime.get_system_config_client().await else {
            return;
        };
        let Ok(current) = client.get(mutation_key).await else {
            return;
        };
        let mut actions = HashMap::new();
        actions.insert(mutation_key.to_string(), KVAction::Remove);
        let _ = client
            .exec_tx(actions, Some((mutation_key.to_string(), current.version)))
            .await;
    }

    async fn find_app_submit_replay(
        &self,
        principal: &RpcAuthPrincipal,
        idempotency_key: &str,
    ) -> Result<Option<buckyos_api::Task>, RPCErrors> {
        let client = self.app_installer.task_mgr_client().await?;
        let page = client
            .list_tasks(buckyos_api::ListTasksReq {
                creator_user_id: Some(principal.username.clone()),
                creator_app_id: Some(principal.authenticated_app_id.clone()),
                idempotency_key: Some(idempotency_key.to_string()),
                include_archived: true,
                limit: Some(1),
                ..Default::default()
            })
            .await?;
        match page.tasks.first() {
            Some(summary) => Ok(Some(client.get_task(&summary.task_id).await?)),
            None => Ok(None),
        }
    }

    fn submitted_plan_replay_matches(
        persisted: Option<&buckyos_api::InstallPlan>,
        submitted: Option<&buckyos_api::InstallPlan>,
    ) -> bool {
        match (persisted, submitted) {
            (None, None) => true,
            (Some(persisted), Some(submitted)) => {
                persisted.schema_version == APP_INSTALL_SCHEMA_VERSION
                    && submitted.schema_version == APP_INSTALL_SCHEMA_VERSION
                    && persisted.fingerprint_is_valid()
                    && submitted.fingerprint_is_valid()
                    && persisted.plan_fingerprint == submitted.plan_fingerprint
            }
            _ => false,
        }
    }

    fn app_submit_replay_matches(
        task: &buckyos_api::Task,
        req: &RPCRequest,
        principal: &RpcAuthPrincipal,
        idempotency_key: &str,
        submitted_plan: &Option<buckyos_api::InstallPlan>,
    ) -> Result<bool, RPCErrors> {
        let source = Self::parse_install_source(req)?;
        let owner_user_id = Self::resolve_target_user_id(req, principal);
        Self::require_install_scope(principal, owner_user_id.as_str())?;
        let options = Some(Self::merged_install_options(req)?);
        let policy = Self::parse_install_policy(principal, req.params.get("options"))?;
        let approved_plan_fingerprint = Self::param_str(req, "approved_plan_fingerprint");

        match task.schema_id.as_str() {
            APP_INSTALL_TASK_SCHEMA_ID => {
                let data: buckyos_api::AppInstallTaskData =
                    serde_json::from_value(task.input.clone()).map_err(|error| {
                        RPCErrors::ReasonError(format!("invalid install task input: {error}"))
                    })?;
                Ok(data.request.source == source
                    && data.request.creator_user_id == principal.username
                    && data.request.creator_app_id == principal.authenticated_app_id
                    && data.request.owner_user_id == owner_user_id
                    && data.request.idempotency_key == idempotency_key
                    && Self::submitted_plan_replay_matches(
                        data.request.submitted_plan.as_ref(),
                        submitted_plan.as_ref(),
                    )
                    && data.request.approved_plan_fingerprint == approved_plan_fingerprint
                    && data.request.policy == policy
                    && data.request.options == options)
            }
            APP_UPDATE_TASK_SCHEMA_ID => {
                let data: buckyos_api::AppUpdateTaskData =
                    serde_json::from_value(task.input.clone()).map_err(|error| {
                        RPCErrors::ReasonError(format!("invalid update task input: {error}"))
                    })?;
                Ok(data.request.source == source
                    && data.request.creator_user_id == principal.username
                    && data.request.creator_app_id == principal.authenticated_app_id
                    && data.request.owner_user_id == owner_user_id
                    && data.request.idempotency_key == idempotency_key
                    && Self::submitted_plan_replay_matches(
                        data.request.submitted_plan.as_ref(),
                        submitted_plan.as_ref(),
                    )
                    && data.request.approved_plan_fingerprint == approved_plan_fingerprint
                    && data.request.policy == policy
                    && data.request.options == options)
            }
            _ => Ok(false),
        }
    }

    fn preinstall_intent_id(
        owner_user_id: &str,
        app_id: &buckyos_api::AppId,
        pikg_digest: &str,
        seed: &buckyos_api::PreInstallPlanSeed,
    ) -> Result<String, RPCErrors> {
        let material = serde_json::json!({
            "kind": "preinstall",
            "owner_user_id": owner_user_id,
            "app_id": app_id,
            "pikg_digest": pikg_digest,
            "install_plan": seed,
        });
        let (object_id, _) = build_named_object_by_json("preseed", &material);
        let object_id = object_id.to_string();
        let digest = object_id
            .split_once(':')
            .map(|(_, digest)| digest)
            .ok_or_else(|| {
                RPCErrors::ReasonError("pre-install intent ObjectId is malformed".to_string())
            })?;
        Ok(format!("t-{}", &digest[..32]))
    }

    fn preinstall_idempotency_key(
        owner_user_id: &str,
        app_id: &buckyos_api::AppId,
        pikg_digest: &str,
        plan_fingerprint: &str,
    ) -> String {
        let material = serde_json::json!({
            "kind": "preinstall",
            "owner_user_id": owner_user_id,
            "app_id": app_id,
            "pikg_digest": pikg_digest,
            "plan_fingerprint": plan_fingerprint,
        });
        let (object_id, _) = build_named_object_by_json("preidem", &material);
        format!("preinstall:{}", object_id.to_string().replace(':', "-"))
    }

    fn preinstall_retry_idempotency_key(retry_of_task_id: &str, plan_fingerprint: &str) -> String {
        let material = serde_json::json!({
            "kind": "preinstall_retry",
            "retry_of_task_id": retry_of_task_id,
            "plan_fingerprint": plan_fingerprint,
        });
        let (object_id, _) = build_named_object_by_json("preretry", &material);
        format!(
            "preinstall-retry:{}",
            object_id.to_string().replace(':', "-")
        )
    }

    fn preinstall_replay_matches(
        task: &buckyos_api::Task,
        owner_user_id: &str,
        plan_fingerprint: &str,
        idempotency_key: &str,
    ) -> Result<bool, RPCErrors> {
        let request = match task.schema_id.as_str() {
            APP_INSTALL_TASK_SCHEMA_ID => {
                serde_json::from_value::<buckyos_api::AppInstallTaskData>(task.input.clone())
                    .map_err(|error| {
                        RPCErrors::ReasonError(format!("invalid install task input: {error}"))
                    })?
                    .request
            }
            APP_UPDATE_TASK_SCHEMA_ID => {
                let data =
                    serde_json::from_value::<buckyos_api::AppUpdateTaskData>(task.input.clone())
                        .map_err(|error| {
                            RPCErrors::ReasonError(format!("invalid update task input: {error}"))
                        })?;
                buckyos_api::AppInstallTaskRequest {
                    source: data.request.source,
                    creator_user_id: data.request.creator_user_id,
                    creator_app_id: data.request.creator_app_id,
                    owner_user_id: data.request.owner_user_id,
                    idempotency_key: data.request.idempotency_key,
                    submitted_plan: data.request.submitted_plan,
                    approved_plan_fingerprint: data.request.approved_plan_fingerprint,
                    policy: data.request.policy,
                    options: data.request.options,
                }
            }
            _ => return Ok(false),
        };
        Ok(request.creator_user_id == owner_user_id
            && request.creator_app_id == "system:control-panel"
            && request.owner_user_id == owner_user_id
            && request.idempotency_key == idempotency_key
            && request.policy == InstallPolicy::SystemInternal
            && request.approved_plan_fingerprint.as_deref() == Some(plan_fingerprint)
            && request.submitted_plan.as_ref().is_some_and(|plan| {
                plan.fingerprint_is_valid() && plan.plan_fingerprint == plan_fingerprint
            }))
    }

    async fn resume_preinstall_replay(
        &self,
        principal: &RpcAuthPrincipal,
        owner_user_id: &str,
        app_instance_id: &buckyos_api::AppInstanceId,
        plan_fingerprint: &str,
        mut task: buckyos_api::Task,
        mut idempotency_key: String,
    ) -> Result<PreInstallSubmitOutcome, RPCErrors> {
        const MAX_RETRY_CHAIN_DEPTH: usize = 32;

        for _ in 0..MAX_RETRY_CHAIN_DEPTH {
            if !Self::preinstall_replay_matches(
                &task,
                owner_user_id,
                plan_fingerprint,
                idempotency_key.as_str(),
            )? {
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::IdempotencyConflict,
                    false,
                    "pre-install idempotency key belongs to different immutable input",
                )));
            }

            let should_retry = task.phase == TaskPhase::Paused
                || (task.phase == TaskPhase::Terminal && task.outcome == Some(TaskOutcome::Failed));
            if should_retry {
                let retry_key =
                    Self::preinstall_retry_idempotency_key(&task.task_id, plan_fingerprint);
                if let Some(retry_task) = self
                    .find_app_submit_replay(principal, retry_key.as_str())
                    .await?
                {
                    if retry_task.retry_of.as_deref() != Some(task.task_id.as_str()) {
                        return Err(Self::install_error_to_rpc(
                            buckyos_api::InstallError::new(
                                buckyos_api::InstallStage::Inspect,
                                buckyos_api::InstallErrorCode::IdempotencyConflict,
                                false,
                                "pre-install retry idempotency key belongs to a different predecessor task",
                            ),
                        ));
                    }
                    task = retry_task;
                    idempotency_key = retry_key;
                    continue;
                }

                let retry_task_id = self
                    .install_engine
                    .retry(
                        task.task_id.as_str(),
                        owner_user_id,
                        true,
                        buckyos_api::CONTROL_PANEL_SERVICE_NAME,
                        retry_key.as_str(),
                    )
                    .await
                    .map_err(Self::install_error_to_rpc)?;
                self.install_runner.spawn_run(retry_task_id.clone());
                return Ok(PreInstallSubmitOutcome {
                    action: "retry".to_string(),
                    task_id: Some(retry_task_id),
                    app_instance_id: app_instance_id.clone(),
                    plan_fingerprint: plan_fingerprint.to_string(),
                });
            }

            if task.phase == TaskPhase::Terminal {
                let message = match task.outcome {
                    Some(TaskOutcome::Succeeded) => format!(
                        "pre-install task {} succeeded but the AppSpec is missing",
                        task.task_id
                    ),
                    Some(TaskOutcome::Canceled) => {
                        format!("pre-install task {} was canceled", task.task_id)
                    }
                    Some(TaskOutcome::Failed) => unreachable!("failed task handled above"),
                    None => format!(
                        "pre-install task {} is terminal without an outcome",
                        task.task_id
                    ),
                };
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Deploy,
                    buckyos_api::InstallErrorCode::Conflict,
                    false,
                    message,
                )));
            }

            self.install_runner.spawn_run(task.task_id.clone());
            return Ok(PreInstallSubmitOutcome {
                action: "replay".to_string(),
                task_id: Some(task.task_id),
                app_instance_id: app_instance_id.clone(),
                plan_fingerprint: plan_fingerprint.to_string(),
            });
        }

        Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
            buckyos_api::InstallStage::Inspect,
            buckyos_api::InstallErrorCode::Conflict,
            false,
            "pre-install retry chain exceeds the supported depth",
        )))
    }

    /// Internal pre-install entry. It uses the same inspect, action matrix,
    /// mutation ownership, TaskManager persistence and runner path as apps.submit.
    pub(crate) async fn submit_preinstall(
        &self,
        owner_user_id: &str,
        app_id: &buckyos_api::AppId,
        pikg_digest: &str,
        pikg_app_doc_object_id: &ObjId,
        pikg_app_doc: &buckyos_api::AppDoc,
        staging_handle: &str,
        seed: &buckyos_api::PreInstallPlanSeed,
    ) -> Result<PreInstallSubmitOutcome, RPCErrors> {
        let canonical_app_id = buckyos_api::AppId::from_app_did(pikg_app_doc.app_did())
            .map_err(RPCErrors::ReasonError)?;
        let expected_owner = pikg_app_doc.app_did().upper_did().ok_or_else(|| {
            RPCErrors::ReasonError("pre-install AppDID has no structural owner".to_string())
        })?;
        let app_doc_value = serde_json::to_value(pikg_app_doc).map_err(|error| {
            RPCErrors::ReasonError(format!("serialize pre-install AppDoc failed: {error}"))
        })?;
        let (canonical_app_doc_object_id, _) =
            build_named_object_by_json(buckyos_api::OBJ_TYPE_APP_DOC, &app_doc_value);
        if &canonical_app_id != app_id
            || &canonical_app_doc_object_id != pikg_app_doc_object_id
            || pikg_app_doc.owner != expected_owner
        {
            return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                buckyos_api::InstallStage::Resolve,
                buckyos_api::InstallErrorCode::VerificationFailed,
                false,
                "pre-install AppDID, structural owner or canonical AppDoc ObjectId is inconsistent",
            )));
        }
        let name_client = name_client::get_name_client().ok_or_else(|| {
            RPCErrors::ReasonError("name client unavailable for pre-install authority".to_string())
        })?;
        name_client.set_local_authority_override(
            pikg_app_doc.app_did().clone(),
            name_client::DidDocType::Custom(
                crate::app_install_resolver::APP_DID_DOC_TYPE.to_string(),
            ),
            name_lib::EncodedDocument::JsonLd(app_doc_value),
            "rootfs-preinstall",
            None,
        );
        let principal = RpcAuthPrincipal {
            username: owner_user_id.to_string(),
            owner_user_id: owner_user_id.to_string(),
            authenticated_app_id: "system:control-panel".to_string(),
            user_type: crate::UserType::Admin,
            owner_did: String::new(),
            is_user_session: false,
            is_control_panel_session: true,
        };
        let mut options = serde_json::json!({
            "policy": "SYSTEM_INTERNAL",
            "auto_confirm": true,
        });
        if let Some(target) = seed.target.as_ref() {
            options["target"] = serde_json::to_value(target).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize pre-install target failed: {error}"))
            })?;
        }
        if let Some(install_params) = seed.install_params.as_ref() {
            options["install_params"] = serde_json::to_value(install_params).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize pre-install params failed: {error}"))
            })?;
        }
        let source = buckyos_api::InstallSource::local_pikg(staging_handle.to_string());
        let mut request = RPCRequest::new(
            "apps.submit",
            serde_json::json!({
                "source": source,
                "owner_user_id": owner_user_id,
                "options": options,
            }),
        );
        let planning_task_id =
            Self::preinstall_intent_id(owner_user_id, app_id, pikg_digest, seed)?;
        let mut inspection = self
            .inspect_from_rpc(
                &request,
                &principal,
                buckyos_api::TASK_DATA_TYPE_APP_INSTALL,
                Some(planning_task_id.clone()),
            )
            .await?;
        let installed = Self::find_spec_for_inspection(&inspection).await?;
        if installed.is_some() {
            inspection = self
                .inspect_from_rpc(
                    &request,
                    &principal,
                    buckyos_api::TASK_DATA_TYPE_APP_UPDATE,
                    Some(planning_task_id),
                )
                .await?;
        }
        if inspection.plan.app_instance_id.app_id() != app_id
            || inspection.plan.owner_user_id != owner_user_id
        {
            return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                buckyos_api::InstallStage::Inspect,
                buckyos_api::InstallErrorCode::VerificationFailed,
                false,
                "pre-install map key, AppDID-derived AppId or owner scope does not match",
            )));
        }
        match &inspection.plan.source_identity {
            buckyos_api::InstallSourceIdentity::Pikg {
                app_doc_object_id,
                pikg_digest: inspected_digest,
            } if app_doc_object_id == pikg_app_doc_object_id
                && inspection.plan.app.object_id == *pikg_app_doc_object_id
                && inspected_digest == pikg_digest => {}
            _ => {
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::VerificationFailed,
                    false,
                    "pre-install PIKG digest or AppDoc identity does not bind the final plan",
                )))
            }
        }
        if inspection.plan.selected_packages.is_empty()
            || inspection.plan.required_contents.is_empty()
        {
            return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                buckyos_api::InstallStage::Inspect,
                buckyos_api::InstallErrorCode::InvalidPackage,
                false,
                "pre-install plan has no selected packages or required contents",
            )));
        }

        if installed
            .as_ref()
            .is_some_and(|spec| spec.deployment.app_doc_object_id == inspection.plan.app.object_id)
        {
            return Ok(PreInstallSubmitOutcome {
                action: "satisfied".to_string(),
                task_id: None,
                app_instance_id: inspection.plan.app_instance_id,
                plan_fingerprint: inspection.plan.plan_fingerprint,
            });
        }

        let idempotency_key = Self::preinstall_idempotency_key(
            owner_user_id,
            app_id,
            pikg_digest,
            inspection.plan.plan_fingerprint.as_str(),
        );
        if let Some(task) = self
            .find_app_submit_replay(&principal, idempotency_key.as_str())
            .await?
        {
            return self
                .resume_preinstall_replay(
                    &principal,
                    owner_user_id,
                    &inspection.plan.app_instance_id,
                    inspection.plan.plan_fingerprint.as_str(),
                    task,
                    idempotency_key,
                )
                .await;
        }

        request.params["idempotency_key"] = Value::String(idempotency_key.clone());
        request.params["plan"] = serde_json::to_value(&inspection.plan).map_err(|error| {
            RPCErrors::ReasonError(format!("serialize pre-install plan failed: {error}"))
        })?;
        request.params["approved_plan_fingerprint"] =
            Value::String(inspection.plan.plan_fingerprint.clone());
        self.handle_apps_submit(request, Some(&principal)).await?;
        let task = self
            .find_app_submit_replay(&principal, idempotency_key.as_str())
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(
                    "apps.submit succeeded without a persisted install task".to_string(),
                )
            })?;
        Ok(PreInstallSubmitOutcome {
            action: if installed.is_some() {
                "upgrade".to_string()
            } else {
                "fresh_install".to_string()
            },
            task_id: Some(task.task_id),
            app_instance_id: inspection.plan.app_instance_id,
            plan_fingerprint: inspection.plan.plan_fingerprint,
        })
    }

    /// Authoritative six-cell action matrix for first install/upgrade/satisfied.
    pub(crate) async fn handle_apps_submit(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let idempotency_key = Self::require_param_str(&req, "idempotency_key")?;
        let submitted_plan: Option<buckyos_api::InstallPlan> = req
            .params
            .get("plan")
            .filter(|value| !value.is_null())
            .map(|value| {
                serde_json::from_value(value.clone())
                    .map_err(|error| RPCErrors::ParseRequestError(format!("invalid plan: {error}")))
            })
            .transpose()?;
        if let Some(task) = self
            .find_app_submit_replay(principal, idempotency_key.as_str())
            .await?
        {
            if !Self::app_submit_replay_matches(
                &task,
                &req,
                principal,
                idempotency_key.as_str(),
                &submitted_plan,
            )? {
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::IdempotencyConflict,
                    false,
                    "idempotency key was already used for another immutable request",
                )));
            }
            self.install_runner.spawn_run(task.task_id.clone());
            return Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "action": "replay",
                    "task_id": task.task_id,
                    "phase": task.phase,
                    "outcome": task.outcome,
                })),
                req.seq,
            ));
        }
        if let Some(plan) = submitted_plan.as_ref() {
            if plan.schema_version != buckyos_api::APP_INSTALL_SCHEMA_VERSION
                || !plan.fingerprint_is_valid()
            {
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::PlanStale,
                    false,
                    "submitted plan schema or fingerprint is invalid",
                )));
            }
        }

        let submitted_task_type = match submitted_plan.as_ref().map(|plan| plan.plan_use) {
            Some(buckyos_api::InstallPlanUse::Upgrade) => buckyos_api::TASK_DATA_TYPE_APP_UPDATE,
            _ => buckyos_api::TASK_DATA_TYPE_APP_INSTALL,
        };
        let mut inspection = self
            .inspect_from_rpc(
                &req,
                principal,
                submitted_task_type,
                submitted_plan.as_ref().map(|plan| plan.task_id.clone()),
            )
            .await?;
        if let Some(plan) = submitted_plan.as_ref() {
            if plan.plan_fingerprint != inspection.plan.plan_fingerprint {
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::PlanStale,
                    false,
                    "submitted plan no longer matches authoritative source inspection",
                )));
            }
        }
        let installed = Self::find_spec_for_inspection(&inspection).await?;
        if submitted_plan.is_none() && installed.is_some() {
            inspection = self
                .inspect_from_rpc(
                    &req,
                    principal,
                    buckyos_api::TASK_DATA_TYPE_APP_UPDATE,
                    None,
                )
                .await?;
        }
        let mut installed_document_version = None;
        if let Some(spec) = installed.as_ref() {
            let record_key =
                buckyos_api::install_record_key(spec.owner_user_id.as_str(), spec.app_id());
            let client = get_buckyos_api_runtime()?
                .get_system_config_client()
                .await?;
            if let Ok(value) = client.get(&record_key).await {
                let record: buckyos_api::InstallRecord = serde_json::from_str(&value.value)
                    .map_err(|error| {
                        RPCErrors::ReasonError(format!("invalid install record: {error}"))
                    })?;
                installed_document_version = record.resolution.document_version;
            }
        }
        let action = decide_app_submit_action(
            submitted_plan.as_ref().map(|plan| plan.plan_use),
            installed
                .as_ref()
                .map(|spec| &spec.deployment.app_doc_object_id),
            &inspection.plan.app.object_id,
            installed_document_version,
            inspection.plan.resolution.document_version,
        )
        .map_err(Self::install_error_to_rpc)?;
        if action == AppSubmitAction::Satisfied {
            return Ok(RPCResponse::new(
                RPCResult::Success(serde_json::json!({
                    "action": "satisfied",
                    "task_id": null,
                    "app_instance_id": inspection.plan.app_instance_id,
                    "app_doc_object_id": inspection.plan.app.object_id,
                })),
                req.seq,
            ));
        }

        let approved_plan_fingerprint = Self::param_str(&req, "approved_plan_fingerprint");
        if approved_plan_fingerprint.as_deref() != Some(inspection.plan.plan_fingerprint.as_str()) {
            return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                buckyos_api::InstallStage::Inspect,
                buckyos_api::InstallErrorCode::PlanStale,
                false,
                "submit must bind the authoritative displayed plan fingerprint",
            )));
        }
        let submit_policy = Self::parse_install_policy(principal, req.params.get("options"))?;
        let mutation_key = Self::acquire_app_mutation(
            &inspection.plan.app_instance_id,
            principal.username.as_str(),
            principal.authenticated_app_id.as_str(),
            idempotency_key.as_str(),
        )
        .await?;
        let task_options = Self::merged_install_options(&req)?;
        let task_result = if action == AppSubmitAction::FreshInstall {
            let request = buckyos_api::AppInstallTaskRequest {
                source: Self::parse_install_source(&req)?,
                creator_user_id: principal.username.clone(),
                creator_app_id: principal.authenticated_app_id.clone(),
                owner_user_id: inspection.plan.owner_user_id.clone(),
                idempotency_key: idempotency_key.clone(),
                submitted_plan: Some(inspection.plan.clone()),
                approved_plan_fingerprint,
                policy: submit_policy,
                options: Some(task_options.clone()),
            };
            self.install_engine
                .create_install_task(request, inspection.plan.app.show_name.as_str())
                .await
        } else {
            let request = buckyos_api::AppUpdateTaskRequest {
                source: Self::parse_install_source(&req)?,
                creator_user_id: principal.username.clone(),
                creator_app_id: principal.authenticated_app_id.clone(),
                owner_user_id: inspection.plan.owner_user_id.clone(),
                idempotency_key: idempotency_key.clone(),
                submitted_plan: Some(inspection.plan.clone()),
                approved_plan_fingerprint,
                policy: submit_policy,
                options: Some(task_options),
            };
            self.install_engine
                .create_update_task(request, inspection.plan.app.show_name.as_str())
                .await
        };
        let task_id = match task_result {
            Ok(task_id) => task_id,
            Err(error) => {
                Self::release_app_mutation_key(&mutation_key).await;
                return Err(Self::install_error_to_rpc(error));
            }
        };
        Self::bind_app_mutation_task(&mutation_key, task_id.as_str()).await?;
        self.install_runner.spawn_run(task_id.clone());
        let action = match action {
            AppSubmitAction::FreshInstall => "fresh_install",
            AppSubmitAction::Upgrade => "upgrade",
            AppSubmitAction::Satisfied => unreachable!(),
        };
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({
                "action": action,
                "task_id": task_id,
                "app_instance_id": inspection.plan.app_instance_id,
                "plan_fingerprint": inspection.plan.plan_fingerprint,
            })),
            req.seq,
        ))
    }

    fn parse_task_id(req: &RPCRequest) -> Result<String, RPCErrors> {
        let raw = Self::require_param_str(req, "task_id")?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(RPCErrors::ParseRequestError("invalid empty task_id".into()));
        }
        Ok(trimmed.to_string())
    }

    /// apps.install.confirm { task_id, plan_fingerprint }
    pub(crate) async fn handle_apps_install_confirm(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let task_id = Self::parse_task_id(&req)?;
        let plan_fingerprint = Self::require_param_str(&req, "plan_fingerprint")?;

        self.install_engine
            .confirm(
                &task_id,
                principal.username.as_str(),
                Self::principal_is_admin(principal),
                plan_fingerprint.as_str(),
            )
            .await
            .map_err(Self::install_error_to_rpc)?;
        self.install_runner.spawn_run(task_id.clone());
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({ "task_id": task_id.to_string() })),
            req.seq,
        ))
    }

    /// apps.install.retry { task_id }
    pub(crate) async fn handle_apps_install_retry(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let task_id = Self::parse_task_id(&req)?;
        let idempotency_key = Self::require_param_str(&req, "idempotency_key")?;
        let retry_task_id = self
            .install_engine
            .retry(
                &task_id,
                principal.username.as_str(),
                Self::principal_is_admin(principal),
                principal.authenticated_app_id.as_str(),
                idempotency_key.as_str(),
            )
            .await
            .map_err(Self::install_error_to_rpc)?;
        self.install_runner.spawn_run(retry_task_id.clone());
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({
                "task_id": retry_task_id,
                "retry_of": task_id,
            })),
            req.seq,
        ))
    }

    /// apps.install.cancel { task_id }
    pub(crate) async fn handle_apps_install_cancel(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let task_id = Self::parse_task_id(&req)?;
        self.install_engine
            .cancel(
                &task_id,
                principal.username.as_str(),
                principal.authenticated_app_id.as_str(),
                Self::principal_is_admin(principal),
            )
            .await
            .map_err(Self::install_error_to_rpc)?;
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({ "task_id": task_id.to_string() })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_uninstall(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let selector = Self::app_selector_from_req(&req)?;
        let data_disposition = match Self::param_str(&req, "data_disposition").as_deref() {
            Some("retain") => AppDataDisposition::Retain,
            Some("delete") => AppDataDisposition::Delete,
            _ => {
                return Err(RPCErrors::ParseRequestError(
                    "data_disposition must be `retain` or `delete`".to_string(),
                ))
            }
        };
        let idempotency_key = Self::require_param_str(&req, "idempotency_key")?;
        let requested_owner = Self::requested_app_owner(&req, principal, selector.as_str());
        if let Some(task) = self
            .find_app_submit_replay(principal, idempotency_key.as_str())
            .await?
        {
            let matches = if task.schema_id == APP_UNINSTALL_TASK_SCHEMA_ID {
                let data: AppUninstallTaskData = serde_json::from_value(task.input.clone())
                    .map_err(|error| {
                        RPCErrors::ReasonError(format!("invalid uninstall task input: {error}"))
                    })?;
                data.request.selector == selector
                    && data.request.app_instance_id.owner_user_id() == requested_owner
                    && data.request.creator_user_id == principal.username
                    && data.request.creator_app_id == principal.authenticated_app_id
                    && data.request.idempotency_key == idempotency_key
                    && data.request.data_disposition == data_disposition
            } else {
                false
            };
            if !matches {
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::IdempotencyConflict,
                    false,
                    "idempotency key was already used for another immutable request",
                )));
            }
            self.app_installer
                .spawn_lifecycle_task(task.task_id.clone());
            return Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "action": "replay",
                    "task_id": task.task_id,
                    "phase": task.phase,
                    "outcome": task.outcome,
                })),
                req.seq,
            ));
        }
        let installation = self.resolve_app_selector(&req, principal).await?;
        let spec = installation.spec;
        Self::require_app_lifecycle_scope(principal, &spec)?;
        let mutation_key = Self::acquire_app_mutation(
            &spec.app_instance_id,
            principal.username.as_str(),
            principal.authenticated_app_id.as_str(),
            idempotency_key.as_str(),
        )
        .await?;
        let task_id = match self
            .app_installer
            .uninstall_app(
                &spec,
                selector.as_str(),
                data_disposition,
                principal.username.as_str(),
                principal.authenticated_app_id.as_str(),
                idempotency_key.as_str(),
            )
            .await
        {
            Ok(task_id) => task_id,
            Err(error) => {
                Self::release_app_mutation_key(&mutation_key).await;
                return Err(error);
            }
        };
        Self::bind_app_mutation_task(&mutation_key, task_id.as_str()).await?;
        self.app_installer.spawn_lifecycle_task(task_id.clone());

        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({
                "task_id": task_id.to_string(),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_start(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        self.handle_apps_lifecycle(req, principal, AppLifecycleAction::Start)
            .await
    }

    pub(crate) async fn handle_apps_stop(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        self.handle_apps_lifecycle(req, principal, AppLifecycleAction::Stop)
            .await
    }

    pub(crate) async fn handle_apps_restart(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        self.handle_apps_lifecycle(req, principal, AppLifecycleAction::Restart)
            .await
    }

    async fn handle_apps_lifecycle(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
        action: AppLifecycleAction,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let selector = Self::app_selector_from_req(&req)?;
        let restart_strategy = match Self::param_str(&req, "restart_strategy").as_deref() {
            None | Some("recreate") => RestartStrategy::Recreate,
            Some("rolling") => {
                return Err(RPCErrors::ReasonError(
                    "rolling restart is not supported by the current deployment strategy"
                        .to_string(),
                ))
            }
            Some(other) => {
                return Err(RPCErrors::ParseRequestError(format!(
                    "invalid restart_strategy `{other}`"
                )))
            }
        };
        let idempotency_key = Self::require_param_str(&req, "idempotency_key")?;
        let requested_owner = Self::requested_app_owner(&req, principal, selector.as_str());
        if let Some(task) = self
            .find_app_submit_replay(principal, idempotency_key.as_str())
            .await?
        {
            let matches = if task.schema_id == APP_START_TASK_SCHEMA_ID {
                let data: AppStartTaskData =
                    serde_json::from_value(task.input.clone()).map_err(|error| {
                        RPCErrors::ReasonError(format!("invalid lifecycle task input: {error}"))
                    })?;
                data.request.selector == selector
                    && data.request.app_instance_id.owner_user_id() == requested_owner
                    && data.request.creator_user_id == principal.username
                    && data.request.creator_app_id == principal.authenticated_app_id
                    && data.request.idempotency_key == idempotency_key
                    && data.request.action == action
                    && data.request.restart_strategy == restart_strategy
            } else {
                false
            };
            if !matches {
                return Err(Self::install_error_to_rpc(buckyos_api::InstallError::new(
                    buckyos_api::InstallStage::Inspect,
                    buckyos_api::InstallErrorCode::IdempotencyConflict,
                    false,
                    "idempotency key was already used for another immutable request",
                )));
            }
            self.app_installer
                .spawn_lifecycle_task(task.task_id.clone());
            return Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "action": "replay",
                    "task_id": task.task_id,
                    "phase": task.phase,
                    "outcome": task.outcome,
                })),
                req.seq,
            ));
        }
        let installation = self.resolve_app_selector(&req, principal).await?;
        let spec = installation.spec;
        Self::require_app_lifecycle_scope(principal, &spec)?;
        let mutation_key = Self::acquire_app_mutation(
            &spec.app_instance_id,
            principal.username.as_str(),
            principal.authenticated_app_id.as_str(),
            idempotency_key.as_str(),
        )
        .await?;
        let task_id = match self
            .app_installer
            .lifecycle_app(
                &spec,
                selector.as_str(),
                action,
                restart_strategy,
                principal.username.as_str(),
                principal.authenticated_app_id.as_str(),
                idempotency_key.as_str(),
            )
            .await
        {
            Ok(task_id) => task_id,
            Err(error) => {
                Self::release_app_mutation_key(&mutation_key).await;
                return Err(error);
            }
        };
        Self::bind_app_mutation_task(&mutation_key, task_id.as_str()).await?;
        self.app_installer.spawn_lifecycle_task(task_id.clone());

        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({
                "task_id": task_id.to_string(),
                "action": action,
                "app_instance_id": spec.app_instance_id,
            })),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod submit_action_tests {
    use super::*;

    fn device_principal() -> RpcAuthPrincipal {
        RpcAuthPrincipal {
            username: "ood1".to_string(),
            owner_user_id: "devtest".to_string(),
            authenticated_app_id: "control-panel".to_string(),
            user_type: crate::UserType::Root,
            owner_did: "did:bns:devtest".to_string(),
            is_user_session: false,
            is_control_panel_session: false,
        }
    }

    #[test]
    fn device_install_defaults_to_device_owner() {
        let principal = device_principal();
        let request = RPCRequest::new("apps.submit", json!({}));
        assert_eq!(
            ControlPanelServer::resolve_target_user_id(&request, &principal),
            "devtest"
        );

        let explicit = RPCRequest::new("apps.submit", json!({ "owner_user_id": "alice" }));
        assert_eq!(
            ControlPanelServer::resolve_target_user_id(&explicit, &principal),
            "alice"
        );

        assert_eq!(
            ControlPanelServer::requested_app_owner(
                &request,
                &principal,
                "filebrowser.buckyos.bns.did@ood1"
            ),
            "ood1"
        );
    }

    #[test]
    fn submitted_plan_use_selects_fresh_install_or_upgrade() {
        let installed = ObjId::new_by_raw("appdoc".to_string(), vec![1; 32]);
        let target = ObjId::new_by_raw("appdoc".to_string(), vec![2; 32]);

        assert_eq!(
            decide_app_submit_action(
                Some(buckyos_api::InstallPlanUse::FreshInstall),
                None,
                &target,
                None,
                Some(2),
            )
            .unwrap(),
            AppSubmitAction::FreshInstall
        );
        assert_eq!(
            decide_app_submit_action(
                Some(buckyos_api::InstallPlanUse::Upgrade),
                Some(&installed),
                &target,
                Some(1),
                Some(2),
            )
            .unwrap(),
            AppSubmitAction::Upgrade
        );
        assert_eq!(
            decide_app_submit_action(
                Some(buckyos_api::InstallPlanUse::FreshInstall),
                Some(&installed),
                &target,
                Some(1),
                Some(2),
            )
            .unwrap_err()
            .code,
            buckyos_api::InstallErrorCode::PlanNotApplicable
        );
        assert_eq!(
            decide_app_submit_action(
                Some(buckyos_api::InstallPlanUse::Upgrade),
                None,
                &target,
                None,
                Some(2),
            )
            .unwrap_err()
            .code,
            buckyos_api::InstallErrorCode::PlanNotApplicable
        );
    }

    #[test]
    fn implicit_submit_preserves_satisfied_and_downgrade_guards() {
        let installed = ObjId::new_by_raw("appdoc".to_string(), vec![1; 32]);
        let target = ObjId::new_by_raw("appdoc".to_string(), vec![2; 32]);

        assert_eq!(
            decide_app_submit_action(None, None, &target, None, Some(2))
                .unwrap_err()
                .code,
            buckyos_api::InstallErrorCode::PlanRequired
        );
        assert_eq!(
            decide_app_submit_action(None, Some(&installed), &installed, Some(1), Some(1),)
                .unwrap(),
            AppSubmitAction::Satisfied
        );
        assert_eq!(
            decide_app_submit_action(None, Some(&installed), &target, Some(2), Some(1))
                .unwrap_err()
                .code,
            buckyos_api::InstallErrorCode::DowngradeNotAllowed
        );
    }

    #[test]
    fn preinstall_intent_and_idempotency_bind_immutable_inputs() {
        let app_id = buckyos_api::AppId::parse("demo.buckyos.bns.did").unwrap();
        let seed = buckyos_api::PreInstallPlanSeed::default();
        let first =
            ControlPanelServer::preinstall_intent_id("alice", &app_id, "digest-a", &seed).unwrap();
        let replay =
            ControlPanelServer::preinstall_intent_id("alice", &app_id, "digest-a", &seed).unwrap();
        let changed =
            ControlPanelServer::preinstall_intent_id("alice", &app_id, "digest-b", &seed).unwrap();
        assert_eq!(first, replay);
        assert_ne!(first, changed);
        assert_eq!(first.len(), 34);
        assert!(first.starts_with("t-"));
        assert!(first[2..].bytes().all(|byte| byte.is_ascii_hexdigit()
            && (!byte.is_ascii_alphabetic() || byte.is_ascii_lowercase())));

        let first_key =
            ControlPanelServer::preinstall_idempotency_key("alice", &app_id, "digest-a", "plan-a");
        let changed_key =
            ControlPanelServer::preinstall_idempotency_key("alice", &app_id, "digest-a", "plan-b");
        assert_ne!(first_key, changed_key);

        let retry = ControlPanelServer::preinstall_retry_idempotency_key("task-a", "plan-a");
        let retry_replay = ControlPanelServer::preinstall_retry_idempotency_key("task-a", "plan-a");
        let next_retry = ControlPanelServer::preinstall_retry_idempotency_key("task-b", "plan-a");
        assert_eq!(retry, retry_replay);
        assert_ne!(retry, next_retry);
    }
}

#[cfg(all(test, any()))]
mod tests {
    use super::*;
    use name_lib::DID;

    #[test]
    fn submit_action_matrix_covers_install_upgrade_satisfied_and_downgrade() {
        let installed = ObjId::new_by_raw("appdoc".to_string(), vec![1; 32]);
        let target = ObjId::new_by_raw("appdoc".to_string(), vec![2; 32]);
        assert_eq!(
            decide_app_submit_action(true, None, &target, None, Some(2)).unwrap(),
            AppSubmitAction::FreshInstall
        );
        assert_eq!(
            decide_app_submit_action(true, Some(&installed), &target, Some(1), Some(2))
                .unwrap_err()
                .code,
            buckyos_api::InstallErrorCode::PlanNotApplicable
        );
        assert_eq!(
            decide_app_submit_action(false, None, &target, None, Some(2))
                .unwrap_err()
                .code,
            buckyos_api::InstallErrorCode::PlanRequired
        );
        assert_eq!(
            decide_app_submit_action(false, Some(&installed), &installed, Some(1), Some(1))
                .unwrap(),
            AppSubmitAction::Satisfied
        );
        assert_eq!(
            decide_app_submit_action(false, Some(&installed), &target, Some(1), Some(2)).unwrap(),
            AppSubmitAction::Upgrade
        );
        assert_eq!(
            decide_app_submit_action(false, Some(&installed), &target, Some(2), Some(1))
                .unwrap_err()
                .code,
            buckyos_api::InstallErrorCode::DowngradeNotAllowed
        );
    }

    fn test_installer() -> AppInstaller {
        AppInstaller::new()
    }

    fn test_owner() -> DID {
        DID::from_str("did:bns:tester").expect("parse test did")
    }

    fn build_web_template() -> AppDoc {
        AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &test_owner())
            .web_pkg(SubPkgDesc::new("tester_demo-web-web#0.1.0"))
            .build()
            .expect("build web template")
    }

    fn build_agent_template() -> AppDoc {
        AppDoc::builder(
            AppType::Agent,
            "demo-agent",
            "0.1.0",
            "tester",
            &test_owner(),
        )
        .agent_pkg(SubPkgDesc::new("tester_demo-agent-agent#0.1.0"))
        .build()
        .expect("build agent template")
    }

    fn build_appservice_template() -> AppDoc {
        AppDoc::builder(
            AppType::AppService,
            "demo-service",
            "0.1.0",
            "tester",
            &test_owner(),
        )
        .amd64_docker_image(
            SubPkgDesc::new("tester_demo-service-img-amd64#0.1.0")
                .docker_image_name("buckyos/demo_service:0.1.0-amd64"),
        )
        .aarch64_docker_image(
            SubPkgDesc::new("tester_demo-service-img-aarch64#0.1.0")
                .docker_image_name("buckyos/demo_service:0.1.0-aarch64"),
        )
        .build()
        .expect("build appservice template")
    }

    fn build_script_appservice_template() -> AppDoc {
        AppDoc::builder(
            AppType::AppService,
            "demo-script-service",
            "0.1.0",
            "tester",
            &test_owner(),
        )
        .script_pkg(SubPkgDesc::new("tester_demo-script-service-script#0.1.0"))
        .build()
        .expect("build script appservice template")
    }

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{}-{}", prefix, Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn publish_example_builds_web_sub_pkg_meta_and_final_app_doc() {
        let installer = test_installer();
        let template = build_web_template();
        let web_desc = template.pkg_list.web.clone().expect("web pkg");
        let web_file = FileObject::new(
            "tester_demo-web-web.tar.gz".to_string(),
            321,
            "chunk:demo-web-content".to_string(),
        );

        let sub_pkg_meta = installer
            .build_sub_pkg_meta(&template, &web_desc, web_file.clone())
            .expect("build sub pkg meta");
        let sub_pkg_objid = sub_pkg_meta.gen_obj_id().0;
        let final_doc = AppInstaller::build_final_app_doc_for_publish(
            &template,
            None,
            &[("web".to_string(), web_desc.clone(), sub_pkg_objid.clone())],
        )
        .expect("build final app doc");

        println!(
            "web_sub_pkg_meta = {}",
            serde_json::to_string_pretty(&sub_pkg_meta).expect("serialize sub pkg meta")
        );
        println!(
            "web_final_app_doc = {}",
            serde_json::to_string_pretty(&final_doc).expect("serialize final app doc")
        );

        assert_eq!(sub_pkg_meta.name, "tester_demo-web-web");
        assert_eq!(sub_pkg_meta.version, "0.1.0");
        assert_eq!(sub_pkg_meta.size, 321);
        assert_eq!(sub_pkg_meta.content, "chunk:demo-web-content");

        assert_eq!(final_doc._base.size, 0);
        assert!(final_doc._base.content.is_empty());
        assert_eq!(
            final_doc
                .pkg_list
                .web
                .as_ref()
                .and_then(|desc| desc.pkg_objid.clone()),
            Some(sub_pkg_objid)
        );
        assert_eq!(
            final_doc
                .pkg_list
                .web
                .as_ref()
                .and_then(|desc| desc.source_url.clone()),
            None
        );
    }

    #[test]
    fn publish_example_appservice_can_degenerate_to_pure_meta() {
        let installer = test_installer();
        let template = build_appservice_template();
        let empty_dir = temp_test_dir("appservice-pure-meta");

        let scan_plan = installer
            .scan_publish_sources(AppType::AppService, &empty_dir, &template)
            .expect("scan pure meta appservice");
        assert!(scan_plan.app_bundle.is_none());
        assert!(scan_plan.sub_pkgs.is_empty());

        let final_doc =
            AppInstaller::build_final_app_doc_for_publish(&template, None, &[]).expect("final doc");

        println!(
            "appservice_pure_meta_app_doc = {}",
            serde_json::to_string_pretty(&final_doc).expect("serialize pure meta appdoc")
        );

        assert!(final_doc._base.content.is_empty());
        assert_eq!(final_doc._base.size, 0);
        assert_eq!(
            final_doc
                .pkg_list
                .amd64_docker_image
                .as_ref()
                .and_then(|desc| desc.docker_image_name.clone()),
            Some("buckyos/demo_service:0.1.0-amd64".to_string())
        );
        assert_eq!(
            final_doc
                .pkg_list
                .amd64_docker_image
                .as_ref()
                .and_then(|desc| desc.pkg_objid.clone()),
            None
        );

        let _ = std::fs::remove_dir_all(empty_dir);
    }

    #[test]
    fn publish_example_appservice_fixed_tar_layout_is_detected() {
        let installer = test_installer();
        let template = build_appservice_template();
        let dir = temp_test_dir("appservice-fixed-layout");
        std::fs::write(dir.join("amd64_docker_image.tar"), b"fake docker tar")
            .expect("write amd64 tar");

        let scan_plan = installer
            .scan_publish_sources(AppType::AppService, &dir, &template)
            .expect("scan appservice");

        println!(
            "appservice_scanned_sub_pkg_count = {}",
            scan_plan.sub_pkgs.len()
        );

        assert!(scan_plan.app_bundle.is_none());
        assert_eq!(scan_plan.sub_pkgs.len(), 1);
        assert_eq!(scan_plan.sub_pkgs[0].key, "amd64_docker_image");
        match &scan_plan.sub_pkgs[0].source {
            PackageSource::File {
                path,
                packaged_name,
            } => {
                assert_eq!(path, &dir.join("amd64_docker_image.tar"));
                assert_eq!(packaged_name.as_deref(), Some("demo-service.tar"));
            }
            other => panic!("unexpected source: {:?}", std::mem::discriminant(other)),
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn publish_example_script_appservice_uses_local_dir() {
        let installer = test_installer();
        let template = build_script_appservice_template();
        let dir = temp_test_dir("appservice-script-layout");
        std::fs::write(dir.join("main.py"), "print('hello')").expect("write script entry");

        let scan_plan = installer
            .scan_publish_sources(AppType::AppService, &dir, &template)
            .expect("scan script appservice");

        assert!(scan_plan.app_bundle.is_none());
        assert_eq!(scan_plan.sub_pkgs.len(), 1);
        assert_eq!(scan_plan.sub_pkgs[0].key, "script");
        match &scan_plan.sub_pkgs[0].source {
            PackageSource::Directory(path) => assert_eq!(path, &dir),
            other => panic!("unexpected source: {:?}", std::mem::discriminant(other)),
        }

        let desc = template.pkg_list.script.clone().expect("script pkg");
        let meta_id = ObjId::new_by_raw("pkg".to_string(), vec![7; 32]);
        let final_doc = AppInstaller::build_final_app_doc_for_publish(
            &template,
            None,
            &[("script".to_string(), desc, meta_id.clone())],
        )
        .expect("build script final doc");
        assert_eq!(
            final_doc
                .pkg_list
                .script
                .as_ref()
                .and_then(|value| value.pkg_objid.clone()),
            Some(meta_id)
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn publish_example_script_and_docker_mix_is_rejected() {
        let installer = test_installer();
        let mut template = build_script_appservice_template();
        template.pkg_list.amd64_docker_image = Some(
            SubPkgDesc::new("tester_demo-script-service-img#0.1.0")
                .docker_image_name("buckyos/demo_script_service:0.1.0-amd64"),
        );
        let dir = temp_test_dir("appservice-script-docker-rejected");

        let error = match installer.scan_publish_sources(AppType::AppService, &dir, &template) {
            Ok(_) => panic!("mixed script and docker packages should be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("mixing `script` and docker"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn publish_example_agent_skills_is_rejected() {
        let installer = test_installer();
        let template = AppDoc::builder(
            AppType::Agent,
            "demo-agent-skills",
            "0.1.0",
            "tester",
            &test_owner(),
        )
        .agent_pkg(SubPkgDesc::new("tester_demo-agent-skills-agent#0.1.0"))
        .agent_skills_pkg(SubPkgDesc::new("tester_demo-agent-skills-skills#0.1.0"))
        .build()
        .expect("build agent template with skills");
        let dir = temp_test_dir("agent-skills-rejected");
        std::fs::write(dir.join("prompt.md"), "hello").expect("write prompt");

        let error = match installer.scan_publish_sources(AppType::Agent, &dir, &template) {
            Ok(_) => panic!("agent skills should be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("agent_skills"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn publish_example_agent_sub_pkg_meta_can_be_viewed_locally() {
        let installer = test_installer();
        let template = build_agent_template();
        let agent_desc = template.pkg_list.agent.clone().expect("agent pkg");
        let agent_file = FileObject::new(
            "tester_demo-agent-agent.tar.gz".to_string(),
            512,
            "chunk:demo-agent-content".to_string(),
        );

        let sub_pkg_meta = installer
            .build_sub_pkg_meta(&template, &agent_desc, agent_file.clone())
            .expect("build agent sub pkg meta");
        let final_doc = AppInstaller::build_final_app_doc_for_publish(
            &template,
            None,
            &[("agent".to_string(), agent_desc, sub_pkg_meta.gen_obj_id().0)],
        )
        .expect("build agent final doc");

        println!(
            "agent_sub_pkg_meta = {}",
            serde_json::to_string_pretty(&sub_pkg_meta).expect("serialize agent sub meta")
        );
        println!(
            "agent_final_app_doc = {}",
            serde_json::to_string_pretty(&final_doc).expect("serialize agent final doc")
        );

        assert_eq!(sub_pkg_meta.name, "tester_demo-agent-agent");
        assert!(final_doc._base.content.is_empty());
        assert_eq!(final_doc._base.size, 0);
    }
}
