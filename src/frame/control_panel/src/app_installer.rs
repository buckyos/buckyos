use buckyos_api::{
    get_buckyos_api_runtime, AppDoc, AppServiceSpec, AppStartTaskData, AppStartTaskRequest,
    AppType, AppUninstallTaskData, AppUninstallTaskRequest, InstallPolicy, RepoClient,
    ServiceInstanceReportInfo, ServiceInstanceState, ServiceState, SubPkgDesc, SystemConfigClient,
    SystemConfigError, TaskManagerClient, TaskStatus,
};
use buckyos_kit::buckyos_get_unix_timestamp;
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
const INSTALL_TASK_TYPE: &str = "app.install";
const UNINSTALL_TASK_TYPE: &str = "app.uninstall";
const START_TASK_TYPE: &str = "app.start";
const UPDATE_TASK_TYPE: &str = "app.update";
const WAIT_INTERVAL_MS: u64 = 1_000;
const WAIT_TIMEOUT_SECS: u64 = 45;
const PROOF_EXPIRE_SECS: u64 = 365 * 24 * 60 * 60;

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
}

impl AppInstaller {
    pub fn new() -> Self {
        Self {
            wait_interval: Duration::from_millis(WAIT_INTERVAL_MS),
            wait_timeout: Duration::from_secs(WAIT_TIMEOUT_SECS),
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
        if spec.app_doc.get_app_type() == AppType::Agent {
            return format!("users/{}/agents/{}/spec", spec.user_id, spec.app_id());
        }

        format!("users/{}/apps/{}/spec", spec.user_id, spec.app_id())
    }

    fn service_spec_id(spec: &AppServiceSpec) -> String {
        format!("{}@{}", spec.app_id(), spec.user_id)
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
            spec.user_id,
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

    async fn find_matching_specs(
        &self,
        app_id: &str,
        user_id: Option<&str>,
    ) -> Result<Vec<(String, AppServiceSpec)>, RPCErrors> {
        let client = self.system_config_client().await?;
        let users = Self::list_children(&client, "users").await?;
        let mut matches = Vec::new();

        for current_user in users {
            if let Some(expected_user) = user_id {
                if current_user != expected_user {
                    continue;
                }
            }

            for key in [
                format!("users/{}/apps/{}/spec", current_user, app_id),
                format!("users/{}/agents/{}/spec", current_user, app_id),
            ] {
                if let Some(spec) = Self::get_optional_json::<AppServiceSpec>(&client, &key).await?
                {
                    matches.push((key, spec));
                }
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

    async fn wait_for_instance_ready(
        &self,
        spec: &AppServiceSpec,
    ) -> Result<ServiceInstanceReportInfo, RPCErrors> {
        let mut waited = Duration::ZERO;
        while waited <= self.wait_timeout {
            if let Ok(instance) = self
                .get_app_service_instance_config(spec.app_id(), Some(spec.user_id.as_str()))
                .await
            {
                if matches!(instance.state, ServiceInstanceState::Started) {
                    info!(
                        "app `{}` for user `{}` instance ready on node `{}`",
                        spec.app_id(),
                        spec.user_id,
                        instance.node_id
                    );
                    return Ok(instance);
                }
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
            spec.user_id,
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
            let node_ids = Self::list_children(&client, &instances_key).await?;
            if node_ids.is_empty() {
                info!(
                    "all instances removed for app `{}` user `{}`",
                    spec.app_id(),
                    spec.user_id
                );
                return Ok(());
            }

            let mut has_active = false;
            for node_id in node_ids {
                let key = format!("{instances_key}/{node_id}");
                if let Some(instance) =
                    Self::get_optional_json::<ServiceInstanceReportInfo>(&client, &key).await?
                {
                    if matches!(
                        instance.state,
                        ServiceInstanceState::Started | ServiceInstanceState::Deploying
                    ) {
                        has_active = true;
                        break;
                    }
                }
            }

            if !has_active {
                info!(
                    "all active instances stopped for app `{}` user `{}`",
                    spec.app_id(),
                    spec.user_id
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
            spec.user_id,
            error
        );
        Err(error)
    }

    async fn create_task(
        &self,
        name: String,
        task_type: &str,
        data: Value,
        user_id: &str,
        app_id: &str,
    ) -> Result<(TaskManagerClient, i64, String), RPCErrors> {
        let task_mgr = self.task_mgr_client().await?;
        let task = task_mgr
            .create_task(name.as_str(), task_type, Some(data), user_id, app_id, None)
            .await?;
        Ok((task_mgr, task.id, task.root_id))
    }

    async fn run_start_task(
        &self,
        app_id: String,
        user_id: Option<String>,
        task_id: i64,
    ) -> Result<(), RPCErrors> {
        let task_mgr = self.task_mgr_client().await?;
        let client = self.system_config_client().await?;
        let (spec_key, mut spec) = self
            .get_single_matching_spec(&app_id, user_id.as_deref())
            .await?;

        if spec.state == ServiceState::Deleted {
            let error = RPCErrors::ReasonError(format!(
                "App `{app_id}` has been deleted and can not be started"
            ));
            warn!("start app `{app_id}` rejected: {}", error);
            return Err(error);
        }

        let _ = task_mgr
            .update_task(
                task_id,
                Some(TaskStatus::Running),
                Some(15.0),
                Some("Updating app state to running".to_string()),
                None,
            )
            .await;

        spec.state = ServiceState::Running;
        Self::set_spec_at(&client, &spec_key, &spec).await?;
        Self::log_spec_state_change(
            &spec,
            ServiceState::Running,
            format!("start task {} updated spec `{}`", task_id, spec_key).as_str(),
        );

        let mut data = json!({
            "spec_path": spec_key,
            "spec_id": Self::service_spec_id(&spec),
        });

        if Self::should_wait_for_instance(&spec) {
            let instance = self.wait_for_instance_ready(&spec).await?;
            data["instance"] = serde_json::to_value(instance).map_err(|error| {
                RPCErrors::ReasonError(format!("Serialize instance report failed: {error}"))
            })?;
        }

        task_mgr
            .update_task(
                task_id,
                Some(TaskStatus::Completed),
                Some(100.0),
                Some("App started".to_string()),
                Some(data),
            )
            .await?;
        info!(
            "start task {} completed for app `{}` user `{}`",
            task_id,
            spec.app_id(),
            spec.user_id
        );
        Ok(())
    }

    async fn run_uninstall_task(
        &self,
        app_id: String,
        user_id: Option<String>,
        is_remove_data: bool,
        task_id: i64,
    ) -> Result<(), RPCErrors> {
        let task_mgr = self.task_mgr_client().await?;
        let client = self.system_config_client().await?;
        let (spec_key, mut spec) = self
            .get_single_matching_spec(&app_id, user_id.as_deref())
            .await?;

        let _ = task_mgr
            .update_task(
                task_id,
                Some(TaskStatus::Running),
                Some(15.0),
                Some("Stopping app".to_string()),
                None,
            )
            .await;

        self.stop_app(&app_id, user_id.as_deref()).await?;

        let _ = task_mgr
            .update_task(
                task_id,
                None,
                Some(55.0),
                Some("Marking app as deleted".to_string()),
                None,
            )
            .await;

        spec.state = ServiceState::Deleted;
        Self::set_spec_at(&client, &spec_key, &spec).await?;
        Self::log_spec_state_change(
            &spec,
            ServiceState::Deleted,
            format!("uninstall task {} wrote spec `{}`", task_id, spec_key).as_str(),
        );

        self.wait_for_instances_removed(&spec).await?;

        if is_remove_data {
            let _ = task_mgr
                .update_task(
                    task_id,
                    None,
                    Some(80.0),
                    Some("Removing app data".to_string()),
                    None,
                )
                .await;
            self.remove_app_data(&spec).await?;
        }

        let data = json!({
            "spec_path": spec_key,
            "spec_id": Self::service_spec_id(&spec),
            "remove_data": is_remove_data,
        });

        task_mgr
            .update_task(
                task_id,
                Some(TaskStatus::Completed),
                Some(100.0),
                Some("App uninstalled".to_string()),
                Some(data),
            )
            .await?;
        info!(
            "uninstall task {} completed for app `{}` user `{}`",
            task_id,
            spec.app_id(),
            spec.user_id
        );
        Ok(())
    }

    fn removable_data_path(path: &Path, app_id: &str) -> bool {
        let raw = path.to_string_lossy();
        raw.starts_with("/opt/buckyos/data/")
            && (raw.contains(&format!("/{app_id}/")) || raw.ends_with(&format!("/{app_id}")))
    }

    async fn remove_app_data(&self, spec: &AppServiceSpec) -> Result<(), RPCErrors> {
        for mount_config in spec.install_config.data_mount_point.values() {
            let path = mount_config.target_path.clone();
            if !Self::removable_data_path(&path, spec.app_id()) {
                warn!(
                    "Skip unsafe app data cleanup for `{}`: {}",
                    spec.app_id(),
                    path.display()
                );
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
                    info!(
                        "removed app data directory for app `{}`: {}",
                        spec.app_id(),
                        path.display()
                    );
                }
                Ok(_) => {
                    fs::remove_file(&path).await.map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "Remove app data file `{}` failed: {error}",
                            path.display()
                        ))
                    })?;
                    info!(
                        "removed app data file for app `{}`: {}",
                        spec.app_id(),
                        path.display()
                    );
                }
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// 卸载应用。必须先 stop_app 才能返回成功。
    ///
    /// 流程：
    /// 1. get_app_service_spec(app_id)，校验 spec 存在
    /// 2. stop_app(app_id) — 改 spec.state=Stopped，触发调度器把实例 item 收敛到 `target_state=Stopped`
    /// 3. spec.state → Deleted，写回 system_config
    /// 4. 等待实例停止；node config 中的实例 item 仍保留，后续 GC 再决定是否清理
    /// 5. 若 is_remove_data：清理应用数据目录
    pub async fn uninstall_app(
        &self,
        app_id: &str,
        user_id: Option<&str>,
        is_remove_data: bool,
    ) -> Result<u64, RPCErrors> {
        let spec = self.get_app_service_spec(app_id, user_id).await?;
        let (task_mgr, task_id, _root_id) = self
            .create_task(
                format!("Uninstall app {app_id}"),
                UNINSTALL_TASK_TYPE,
                task_data_value(AppUninstallTaskData {
                    request: AppUninstallTaskRequest {
                        app_id: app_id.to_string(),
                        user_id: spec.user_id.clone(),
                        remove_data: is_remove_data,
                    },
                    result: None,
                })?,
                spec.user_id.as_str(),
                app_id,
            )
            .await?;

        info!(
            "queued uninstall task {} for app `{}` user `{}` remove_data={}",
            task_id, app_id, spec.user_id, is_remove_data
        );

        let installer = self.clone();
        let app_id = app_id.to_string();
        let user_id = Some(spec.user_id.clone());
        tokio::spawn(async move {
            if let Err(error) = installer
                .run_uninstall_task(app_id, user_id, is_remove_data, task_id)
                .await
            {
                warn!("uninstall app task {} failed: {}", task_id, error);
                let _ = task_mgr
                    .mark_task_as_failed(task_id, &error.to_string())
                    .await;
            }
        });

        Ok(task_id as u64)
    }

    /// 停止应用。
    ///
    /// 流程：
    /// 1. get_app_service_spec(app_id)
    /// 2. spec.state → Stopped，写回 system_config
    /// 3. 调度器 schedule_loop 检测到 state 变化，把 nodes/{node}/config.apps 中该应用的 target_state 收敛到 `Stopped`
    /// 4. node-daemon 读 config 收敛，停止容器
    pub async fn stop_app(&self, app_id: &str, user_id: Option<&str>) -> Result<(), RPCErrors> {
        let client = self.system_config_client().await?;
        let (spec_key, mut spec) = self.get_single_matching_spec(app_id, user_id).await?;

        if spec.state == ServiceState::Stopped || spec.state == ServiceState::Deleted {
            info!(
                "skip stop for app `{}` user `{}` because current state is {}",
                spec.app_id(),
                spec.user_id,
                Self::service_state_label(&spec.state)
            );
            return Ok(());
        }

        spec.state = ServiceState::Stopped;
        Self::set_spec_at(&client, &spec_key, &spec).await?;
        Self::log_spec_state_change(
            &spec,
            ServiceState::Stopped,
            format!("stop wrote spec `{}`", spec_key).as_str(),
        );

        if Self::should_wait_for_instance(&spec) {
            self.wait_for_instances_removed(&spec).await?;
        }

        info!(
            "stop completed for app `{}` user `{}`",
            spec.app_id(),
            spec.user_id
        );
        Ok(())
    }

    /// 启动应用。成功返回 task_id。
    ///
    /// 流程：
    /// 1. get_app_service_spec(app_id)
    /// 2. spec.state → Running，写回 system_config
    /// 3. 调度器 schedule_loop 检测到 state 变化，选点并写入 nodes/{node}/config.apps（app_service_instance_config）
    /// 4. node-daemon 读 config 收敛，启动容器
    /// 5. 创建 task，返回 task_id
    pub async fn start_app(&self, app_id: &str, user_id: Option<&str>) -> Result<u64, RPCErrors> {
        let spec = self.get_app_service_spec(app_id, user_id).await?;
        let (task_mgr, task_id, _root_id) = self
            .create_task(
                format!("Start app {app_id}"),
                START_TASK_TYPE,
                task_data_value(AppStartTaskData {
                    request: AppStartTaskRequest {
                        app_id: app_id.to_string(),
                        user_id: spec.user_id.clone(),
                    },
                    result: None,
                })?,
                spec.user_id.as_str(),
                app_id,
            )
            .await?;
        info!(
            "queued start task {} for app `{}` user `{}`",
            task_id, app_id, spec.user_id
        );

        let installer = self.clone();
        let app_id = app_id.to_string();
        let user_id = Some(spec.user_id.clone());
        tokio::spawn(async move {
            if let Err(error) = installer.run_start_task(app_id, user_id, task_id).await {
                warn!("start app task {} failed: {}", task_id, error);
                let _ = task_mgr
                    .mark_task_as_failed(task_id, &error.to_string())
                    .await;
            }
        });

        Ok(task_id as u64)
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
                let has_unsupported_subpkg = app_doc_template
                    .pkg_list
                    .iter()
                    .into_iter()
                    .any(|(key, _)| key != "amd64_docker_image" && key != "aarch64_docker_image");
                if has_unsupported_subpkg {
                    return Err(RPCErrors::ReasonError(
                        "AppService publish currently only supports `amd64_docker_image` and `aarch64_docker_image`"
                            .to_string(),
                    ));
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
                                    app_doc_template.name.as_str(),
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
                    format!("{}-app", app_doc_template.name).as_str(),
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
                    format!("{}-{}", app_doc_template.name, scanned.key).as_str(),
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
            app_doc_template.name, app_doc_template.version
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
            app_doc_template.name,
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
            app_doc_template.name, app_doc_template.version, final_obj_id, pikg_digest
        );
        let app_doc_value = serde_json::to_value(&final_doc).map_err(|error| {
            RPCErrors::ReasonError(format!("Serialize final AppDoc failed: {error}"))
        })?;
        Ok(PublishOutput {
            app_did: final_doc.id.clone(),
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
        let tmp_pikg = prepared.temp_root.join(format!("{}.pikg", final_doc.name));
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
            app_doc_template.author.as_str(),
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

        final_doc._base.content.clear();
        final_doc._base.size = 0;
        final_doc._base.last_update_time = buckyos_get_unix_timestamp();

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
use buckyos_api::RepoRecord;

struct RepoAppReleaseCandidate {
    record: RepoRecord,
    app_doc: AppDoc,
    parsed_version: Option<semver::Version>,
}

impl ControlPanelServer {
    pub(crate) fn resolve_target_user_id(req: &RPCRequest, principal: &RpcAuthPrincipal) -> String {
        Self::param_str(req, "user_id").unwrap_or_else(|| principal.username.clone())
    }

    fn repo_record_version(record: &RepoRecord) -> Option<String> {
        record
            .meta
            .get("version")
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn repo_status_rank(status: &str) -> u8 {
        match status {
            "pinned" => 2,
            "collected" => 1,
            _ => 0,
        }
    }

    fn compare_repo_app_release(
        lhs: &RepoAppReleaseCandidate,
        rhs: &RepoAppReleaseCandidate,
    ) -> std::cmp::Ordering {
        match (&lhs.parsed_version, &rhs.parsed_version) {
            (Some(left), Some(right)) if left != right => return left.cmp(right),
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            _ => {}
        }

        let lhs_status = Self::repo_status_rank(lhs.record.status.as_str());
        let rhs_status = Self::repo_status_rank(rhs.record.status.as_str());
        if lhs_status != rhs_status {
            return lhs_status.cmp(&rhs_status);
        }

        let lhs_updated = lhs
            .record
            .updated_at
            .or(lhs.record.pinned_at)
            .or(lhs.record.collected_at)
            .unwrap_or(0);
        let rhs_updated = rhs
            .record
            .updated_at
            .or(rhs.record.pinned_at)
            .or(rhs.record.collected_at)
            .unwrap_or(0);
        if lhs_updated != rhs_updated {
            return lhs_updated.cmp(&rhs_updated);
        }

        lhs.app_doc.version.cmp(&rhs.app_doc.version)
    }

    async fn resolve_repo_app_release(
        &self,
        app_id: &str,
        version: Option<&str>,
    ) -> Result<RepoAppReleaseCandidate, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        info!(
            "resolve repo app release app_id=`{}` version={:?}",
            app_id, version
        );
        let repo = runtime.get_repo_client().await.map_err(|error| {
            warn!(
                "init repo client failed while resolving app `{}`: {}",
                app_id, error
            );
            RPCErrors::ReasonError(format!("Init repo client failed: {}", error))
        })?;
        let requested_version = version
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let records = repo
            .list(Some(buckyos_api::RepoListFilter::new(
                None,
                None,
                Some(app_id.to_string()),
                None,
            )))
            .await
            .map_err(|error| {
                warn!(
                    "repo.list failed while resolving app `{}` version {:?}: {}",
                    app_id, requested_version, error
                );
                error
            })?;

        let mut candidates = Vec::new();
        for record in records {
            if record.content_name.as_deref() != Some(app_id) {
                continue;
            }

            let Some(record_version) = Self::repo_record_version(&record) else {
                warn!(
                    "skip repo record without version for app `{}` content `{}`",
                    app_id, record.content_id
                );
                continue;
            };
            if requested_version
                .as_deref()
                .map(|expected| expected != record_version)
                .unwrap_or(false)
            {
                continue;
            }

            let app_doc: AppDoc = match serde_json::from_value(record.meta.clone()) {
                Ok(value) => value,
                Err(error) => {
                    warn!(
                        "skip repo record `{}` for app `{}` because meta is not AppDoc: {}",
                        record.content_id, app_id, error
                    );
                    continue;
                }
            };
            if app_doc.name != app_id {
                continue;
            }

            candidates.push(RepoAppReleaseCandidate {
                parsed_version: semver::Version::parse(app_doc.version.as_str()).ok(),
                record,
                app_doc,
            });
        }

        let release = candidates
            .into_iter()
            .max_by(Self::compare_repo_app_release)
            .ok_or_else(|| {
                let detail = requested_version
                    .map(|value| format!(" version `{value}`"))
                    .unwrap_or_default();
                RPCErrors::ReasonError(format!("No repo app release found for `{app_id}`{detail}"))
            })?;
        info!(
            "resolved repo app release app_id=`{}` version=`{}` content=`{}` status=`{}`",
            release.app_doc.name,
            release.app_doc.version,
            release.record.content_id,
            release.record.status
        );
        Ok(release)
    }

    fn build_default_install_config(
        app_id: &str,
        app_doc: &AppDoc,
    ) -> buckyos_api::ServiceSpecConfig {
        crate::app_install_deployer::build_install_config(app_id, app_doc, &json!({}))
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
            app_doc.name, app_doc.version, app_type, local_dir
        );
        let output = self
            .app_installer
            .publish_app_to_repo(app_type, std::path::Path::new(local_dir.as_str()), &app_doc)
            .await
            .map_err(|error| {
                warn!(
                    "rpc app.publish failed for app `{}` version `{}` local_dir `{}`: {}",
                    app_doc.name, app_doc.version, local_dir, error
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

    fn install_error_to_rpc(error: buckyos_api::InstallError) -> RPCErrors {
        RPCErrors::ReasonError(serde_json::to_string(&error).unwrap_or_else(|_| error.to_string()))
    }

    async fn start_install_transaction(
        &self,
        source: buckyos_api::InstallSource,
        app_hint: &str,
        user_id: String,
        policy: InstallPolicy,
        options: Option<Value>,
        seq: u64,
    ) -> Result<RPCResponse, RPCErrors> {
        let request = buckyos_api::AppInstallTaskRequest {
            source,
            user_id,
            policy,
            options,
        };
        let task_id = self
            .install_engine
            .create_install_task(request, app_hint)
            .await
            .map_err(Self::install_error_to_rpc)?;
        self.install_runner.dispatch(task_id).await;
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({
                "task_id": task_id.to_string(),
            })),
            seq,
        ))
    }

    /// apps.install { identifier, referrer?, options? } -> { task_id }
    /// v0.5 协议：identifier 是 DID/名称/ObjectID/URL；
    /// 旧 app_id/version 语义已删除，不保留分支兼容。
    pub(crate) async fn handle_apps_install(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let identifier = Self::require_param_str(&req, "identifier")?;
        let referrer = Self::param_str(&req, "referrer");
        let options = req.params.get("options").cloned();
        let user_id = Self::resolve_target_user_id(&req, principal);
        Self::require_install_scope(principal, user_id.as_str())?;
        let policy = Self::parse_install_policy(principal, options.as_ref())?;

        info!(
            "rpc apps.install identifier=`{}` user=`{}` policy={:?}",
            identifier, user_id, policy
        );
        self.start_install_transaction(
            buckyos_api::InstallSource::identifier(identifier.clone(), referrer),
            identifier.as_str(),
            user_id,
            policy,
            options,
            req.seq,
        )
        .await
    }

    /// apps.install_package { staging_handle, options? } -> { task_id }
    /// 只接受 staging handle（D5），不接受服务端路径。
    pub(crate) async fn handle_apps_install_package(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let staging_handle = Self::require_param_str(&req, "staging_handle")?;
        let options = req.params.get("options").cloned();
        let user_id = Self::resolve_target_user_id(&req, principal);
        Self::require_install_scope(principal, user_id.as_str())?;
        let policy = Self::parse_install_policy(principal, options.as_ref())?;

        info!(
            "rpc apps.install_package handle=`{}` user=`{}` policy={:?}",
            staging_handle, user_id, policy
        );
        self.start_install_transaction(
            buckyos_api::InstallSource::local_pikg(staging_handle.clone()),
            staging_handle.as_str(),
            user_id,
            policy,
            options,
            req.seq,
        )
        .await
    }

    fn parse_task_id(req: &RPCRequest) -> Result<i64, RPCErrors> {
        let raw = Self::require_param_str(req, "task_id")?;
        raw.trim()
            .parse::<i64>()
            .map_err(|_| RPCErrors::ParseRequestError(format!("invalid task_id `{raw}`")))
    }

    /// apps.install.confirm { task_id, target?, install_params?, accepted_permissions? }
    pub(crate) async fn handle_apps_install_confirm(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let task_id = Self::parse_task_id(&req)?;
        let target = match req.params.get("target") {
            Some(value) if !value.is_null() => Some(
                serde_json::from_value::<buckyos_api::InstallTarget>(value.clone()).map_err(
                    |err| RPCErrors::ParseRequestError(format!("invalid target: {err}")),
                )?,
            ),
            _ => None,
        };
        let install_params = req
            .params
            .get("install_params")
            .cloned()
            .filter(|value| !value.is_null());
        let accepted_permissions: Vec<String> = match req.params.get("accepted_permissions") {
            Some(value) if !value.is_null() => {
                serde_json::from_value(value.clone()).map_err(|err| {
                    RPCErrors::ParseRequestError(format!("invalid accepted_permissions: {err}"))
                })?
            }
            _ => Vec::new(),
        };

        self.install_engine
            .confirm(
                task_id,
                principal.username.as_str(),
                Self::principal_is_admin(principal),
                target,
                install_params,
                accepted_permissions,
            )
            .await
            .map_err(Self::install_error_to_rpc)?;
        self.install_runner.spawn_run(task_id);
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
        self.install_engine
            .retry(
                task_id,
                principal.username.as_str(),
                Self::principal_is_admin(principal),
            )
            .await
            .map_err(Self::install_error_to_rpc)?;
        self.install_runner.spawn_run(task_id);
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({ "task_id": task_id.to_string() })),
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
                task_id,
                principal.username.as_str(),
                Self::principal_is_admin(principal),
            )
            .await
            .map_err(Self::install_error_to_rpc)?;
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({ "task_id": task_id.to_string() })),
            req.seq,
        ))
    }

    /// apps.update { app_id, options? } -> { task_id }
    /// v0.5：升级走与安装同一条 Stage 流水线（Resolve -> ... -> Prepare 完成
    /// 后才切换旧版本；Activate 失败自动恢复旧 spec）。升级判定以权威
    /// Resolve 的 App Document 为准，不再从 Repo 选 semver 最新。
    pub(crate) async fn handle_apps_update(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let options = req.params.get("options").cloned();
        let user_id = Self::resolve_target_user_id(&req, principal);
        Self::require_install_scope(principal, user_id.as_str())?;
        let policy = Self::parse_install_policy(principal, options.as_ref())?;

        // 已安装 spec 是升级入口的真相源：App DID 从中取得。
        let current_spec = self
            .app_installer
            .get_app_service_spec(app_id.as_str(), Some(user_id.as_str()))
            .await?;
        let app_did = current_spec.app_doc.id.clone();

        let request = buckyos_api::AppUpdateTaskRequest {
            source: buckyos_api::InstallSource::identifier(app_did.to_string(), None),
            user_id,
            app_id: app_id.clone(),
            policy,
            options,
        };
        let task_id = self
            .install_engine
            .create_update_task(request, app_id.as_str())
            .await
            .map_err(Self::install_error_to_rpc)?;
        self.install_runner.dispatch(task_id).await;

        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({
                "task_id": task_id.to_string(),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_uninstall(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        let remove_data = Self::param_bool(&req, "remove_data")
            .or_else(|| Self::param_bool(&req, "is_remove_data"))
            .unwrap_or(false);
        let task_id = self
            .app_installer
            .uninstall_app(app_id.as_str(), Some(user_id.as_str()), remove_data)
            .await?;

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
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        let task_id = self
            .app_installer
            .start_app(app_id.as_str(), Some(user_id.as_str()))
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({
                "ok": true,
                "task_id": task_id.to_string(),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_stop(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        self.app_installer
            .stop_app(app_id.as_str(), Some(user_id.as_str()))
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::json!({ "ok": true })),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use name_lib::DID;

    fn test_installer() -> AppInstaller {
        AppInstaller::new()
    }

    fn test_owner() -> DID {
        DID::from_str("did:bns:tester").expect("parse test did")
    }

    fn build_web_template() -> AppDoc {
        AppDoc::builder(AppType::Web, "demo_web", "0.1.0", "tester", &test_owner())
            .web_pkg(SubPkgDesc::new("demo_web-web#0.1.0"))
            .build()
            .expect("build web template")
    }

    fn build_agent_template() -> AppDoc {
        AppDoc::builder(
            AppType::Agent,
            "demo_agent",
            "0.1.0",
            "tester",
            &test_owner(),
        )
        .agent_pkg(SubPkgDesc::new("demo_agent-agent#0.1.0"))
        .build()
        .expect("build agent template")
    }

    fn build_appservice_template() -> AppDoc {
        AppDoc::builder(
            AppType::AppService,
            "demo_service",
            "0.1.0",
            "tester",
            &test_owner(),
        )
        .amd64_docker_image(
            SubPkgDesc::new("demo_service-img-amd64#0.1.0")
                .docker_image_name("buckyos/demo_service:0.1.0-amd64"),
        )
        .aarch64_docker_image(
            SubPkgDesc::new("demo_service-img-aarch64#0.1.0")
                .docker_image_name("buckyos/demo_service:0.1.0-aarch64"),
        )
        .build()
        .expect("build appservice template")
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
            "demo_web-web.tar.gz".to_string(),
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

        assert_eq!(sub_pkg_meta.name, "demo_web-web");
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
                assert_eq!(packaged_name.as_deref(), Some("demo_service.tar"));
            }
            other => panic!("unexpected source: {:?}", std::mem::discriminant(other)),
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn publish_example_agent_skills_is_rejected() {
        let installer = test_installer();
        let template = AppDoc::builder(
            AppType::Agent,
            "demo_agent_skills",
            "0.1.0",
            "tester",
            &test_owner(),
        )
        .agent_pkg(SubPkgDesc::new("demo_agent_skills-agent#0.1.0"))
        .agent_skills_pkg(SubPkgDesc::new("demo_agent_skills-skills#0.1.0"))
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
            "demo_agent-agent.tar.gz".to_string(),
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

        assert_eq!(sub_pkg_meta.name, "demo_agent-agent");
        assert!(final_doc._base.content.is_empty());
        assert_eq!(final_doc._base.size, 0);
    }
}
