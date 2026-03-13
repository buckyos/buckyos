use buckyos_api::{
    get_buckyos_api_runtime, AppServiceSpec, AppType, CreateTaskOptions, RepoClient,
    RepoProof, RepoProofFilter, ServiceInstanceReportInfo, ServiceInstanceState, ServiceState,
    SystemConfigClient, SystemConfigError, TaskManagerClient, TaskStatus, REPO_PROOF_TYPE_DOWNLOAD,
    REPO_PROOF_TYPE_REFERRAL, REPO_STATUS_COLLECTED, REPO_STATUS_PINNED,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use kRPC::RPCErrors;
use log::warn;
use ndn_lib::{
    build_obj_id, ActionObject, NamedObject, ObjId, ACTION_TYPE_DOWNLOAD, ACTION_TYPE_INSTALLED,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tokio::time::sleep;

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
const INSTALL_TASK_TYPE: &str = "app_install";
const START_TASK_TYPE: &str = "app_start";
const WAIT_INTERVAL_MS: u64 = 1_000;
const WAIT_TIMEOUT_SECS: u64 = 45;
const PROOF_EXPIRE_SECS: u64 = 365 * 24 * 60 * 60;

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

    async fn system_config_client(&self) -> Result<SystemConfigClient, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_system_config_client().await
    }

    async fn task_mgr_client(&self) -> Result<TaskManagerClient, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_task_mgr_client().await
    }

    async fn repo_client(&self) -> Result<RepoClient, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_repo_client().await
    }

    fn parse_obj_id(raw: &str) -> Result<ObjId, RPCErrors> {
        ObjId::new(raw)
            .map_err(|err| RPCErrors::ReasonError(format!("Invalid obj id `{raw}`: {err}")))
    }

    fn actor_identity(user_id: &str) -> String {
        if user_id.starts_with("did:") {
            user_id.to_string()
        } else {
            format!("did:bns:{user_id}")
        }
    }

    fn build_actor_obj_id(user_id: &str) -> ObjId {
        build_obj_id("actor", &Self::actor_identity(user_id))
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

    fn resolve_content_id(spec: &AppServiceSpec) -> Result<String, RPCErrors> {
        if !spec.app_doc.content.trim().is_empty() {
            return Ok(spec.app_doc.content.trim().to_string());
        }

        for (_, sub_pkg) in spec.app_doc.pkg_list.iter() {
            if let Some(pkg_objid) = sub_pkg.pkg_objid.as_ref() {
                return Ok(pkg_objid.to_string());
            }
        }

        Err(RPCErrors::ReasonError(format!(
            "App `{}` does not include a resolvable content id",
            spec.app_id()
        )))
    }

    fn resolve_download_url(spec: &AppServiceSpec, content_id: &str) -> String {
        for (_, sub_pkg) in spec.app_doc.pkg_list.iter() {
            if let Some(source_url) = sub_pkg.source_url.as_ref() {
                if !source_url.trim().is_empty() {
                    return source_url.trim().to_string();
                }
            }
        }

        format!("cyfs://{content_id}")
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

    async fn load_repo_record_status(&self, content_id: &str) -> Result<Option<String>, RPCErrors> {
        let repo = self.repo_client().await?;
        let records = repo.list(None).await?;
        Ok(records
            .into_iter()
            .find(|record| record.content_id == content_id)
            .map(|record| record.status))
    }

    async fn latest_action_proof_id(
        &self,
        content_id: &str,
        proof_type: &str,
    ) -> Result<Option<ObjId>, RPCErrors> {
        let repo = self.repo_client().await?;
        let proofs = repo
            .get_proofs(
                content_id,
                Some(RepoProofFilter::new(
                    Some(proof_type.to_string()),
                    None,
                    None,
                    None,
                    None,
                )),
            )
            .await?;

        let proof = proofs.into_iter().rev().find_map(|proof| match proof {
            RepoProof::Action(action) => Some(action.gen_obj_id().0),
            RepoProof::Collection(_) => None,
        });
        Ok(proof)
    }

    fn build_download_proof(
        &self,
        user_id: &str,
        content_id: &str,
        base_on: Option<ObjId>,
        task_id: i64,
    ) -> Result<ActionObject, RPCErrors> {
        let target = Self::parse_obj_id(content_id)?;
        let now = buckyos_get_unix_timestamp();
        Ok(ActionObject {
            subject: Self::build_actor_obj_id(user_id),
            action: ACTION_TYPE_DOWNLOAD.to_string(),
            target,
            base_on,
            details: Some(json!({
                "subject_did": Self::actor_identity(user_id),
                "task_id": task_id,
                "source": "control_panel.app_installer",
            })),
            iat: now,
            exp: now + PROOF_EXPIRE_SECS,
        })
    }

    fn build_install_proof(
        &self,
        spec: &AppServiceSpec,
        content_id: &str,
        base_on: Option<ObjId>,
    ) -> Result<ActionObject, RPCErrors> {
        let target = Self::parse_obj_id(content_id)?;
        let now = buckyos_get_unix_timestamp();
        Ok(ActionObject {
            subject: Self::build_actor_obj_id(spec.user_id.as_str()),
            action: ACTION_TYPE_INSTALLED.to_string(),
            target,
            base_on,
            details: Some(json!({
                "subject_did": Self::actor_identity(spec.user_id.as_str()),
                "app_id": spec.app_id(),
                "user_id": spec.user_id,
                "version": spec.app_doc.version,
                "spec_id": Self::service_spec_id(spec),
            })),
            iat: now,
            exp: now + PROOF_EXPIRE_SECS,
        })
    }

    async fn wait_for_download_task(
        &self,
        task_mgr: &TaskManagerClient,
        task_id: i64,
    ) -> Result<(), RPCErrors> {
        let status = task_mgr.wait_for_task_end(task_id).await?;
        if status == TaskStatus::Completed {
            return Ok(());
        }

        let task = task_mgr.get_task(task_id).await?;
        let message = task
            .message
            .unwrap_or_else(|| format!("Download task {task_id} ended with status {status}"));
        Err(RPCErrors::ReasonError(message))
    }

    async fn verify_named_store_ready(&self, content_id: &str) -> Result<(), RPCErrors> {
        let obj_id = Self::parse_obj_id(content_id)?;
        let runtime = get_buckyos_api_runtime()?;
        let named_store = runtime.get_named_store().await.map_err(|error| {
            RPCErrors::ReasonError(format!("Open named store for download verification failed: {error}"))
        })?;
        let _ = named_store.open_reader(&obj_id, None).await.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Downloaded object `{content_id}` is not ready in named-store: {error}"
            ))
        })?;
        Ok(())
    }

    async fn ensure_content_pinned(
        &self,
        spec: &AppServiceSpec,
        parent_task_id: i64,
        root_id: &str,
    ) -> Result<ActionObject, RPCErrors> {
        let content_id = Self::resolve_content_id(spec)?;
        let repo_status = self.load_repo_record_status(&content_id).await?;
        let status = repo_status.ok_or_else(|| {
            RPCErrors::ReasonError(format!(
                "Content `{content_id}` is not collected in RepoService"
            ))
        })?;

        if status == REPO_STATUS_PINNED {
            let base_on = self
                .latest_action_proof_id(&content_id, REPO_PROOF_TYPE_DOWNLOAD)
                .await?;
            return self.build_download_proof(spec.user_id.as_str(), &content_id, base_on, parent_task_id);
        }

        if status != REPO_STATUS_COLLECTED {
            return Err(RPCErrors::ReasonError(format!(
                "Unsupported repo status for `{content_id}`: {status}"
            )));
        }

        let task_mgr = self.task_mgr_client().await?;
        let download_task_id = task_mgr
            .create_download_task(
                Self::resolve_download_url(spec, &content_id).as_str(),
                None,
                None,
                spec.user_id.as_str(),
                spec.app_id(),
                Some(CreateTaskOptions {
                    parent_id: if parent_task_id > 0 {
                        Some(parent_task_id)
                    } else {
                        None
                    },
                    root_id: if root_id.is_empty() {
                        None
                    } else {
                        Some(root_id.to_string())
                    },
                    ..Default::default()
                }),
            )
            .await?;

        if parent_task_id > 0 {
            let _ = task_mgr
                .update_task(
                    parent_task_id,
                    None,
                    Some(20.0),
                    Some("Downloading app package".to_string()),
                    Some(json!({
                        "download_task_id": download_task_id,
                        "content_id": content_id,
                    })),
                )
                .await;
        }

        self.wait_for_download_task(&task_mgr, download_task_id).await?;
        self.verify_named_store_ready(&content_id).await?;

        let referral_base = self
            .latest_action_proof_id(&content_id, REPO_PROOF_TYPE_REFERRAL)
            .await?;
        let download_proof = self.build_download_proof(
            spec.user_id.as_str(),
            &content_id,
            referral_base,
            download_task_id,
        )?;
        let repo = self.repo_client().await?;
        if let Err(error) = repo.pin(&content_id, download_proof.clone()).await {
            let error_text = error.to_string();
            if error_text.contains("is not available locally") {
                warn!(
                    "repo.pin skipped for `{}` because repo-service does not yet read named-store directly: {}",
                    content_id, error_text
                );
            } else {
                return Err(error);
            }
        }
        Ok(download_proof)
    }

    async fn wait_for_instance_ready(
        &self,
        spec: &AppServiceSpec,
    ) -> Result<ServiceInstanceReportInfo, RPCErrors> {
        let mut waited = Duration::ZERO;
        while waited <= self.wait_timeout {
            if let Ok(instance) = self.get_app_service_instance_config(spec.app_id()).await {
                if matches!(instance.state, ServiceInstanceState::Started) {
                    return Ok(instance);
                }
            }
            sleep(self.wait_interval).await;
            waited += self.wait_interval;
        }

        Err(RPCErrors::ReasonError(format!(
            "Timed out waiting for app `{}` instance to become ready",
            spec.app_id()
        )))
    }

    async fn wait_for_instances_removed(&self, spec: &AppServiceSpec) -> Result<(), RPCErrors> {
        let service_spec_id = Self::service_spec_id(spec);
        let client = self.system_config_client().await?;
        let instances_key = format!("services/{service_spec_id}/instances");
        let mut waited = Duration::ZERO;

        while waited <= self.wait_timeout {
            let node_ids = Self::list_children(&client, &instances_key).await?;
            if node_ids.is_empty() {
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
                return Ok(());
            }

            sleep(self.wait_interval).await;
            waited += self.wait_interval;
        }

        Err(RPCErrors::ReasonError(format!(
            "Timed out waiting for app `{}` instances to stop",
            spec.app_id()
        )))
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
            .create_task(
                name.as_str(),
                task_type,
                Some(data),
                user_id,
                app_id,
                None,
            )
            .await?;
        Ok((task_mgr, task.id, task.root_id))
    }

    async fn run_install_task(
        &self,
        spec: AppServiceSpec,
        task_id: i64,
        root_id: String,
    ) -> Result<(), RPCErrors> {
        let task_mgr = self.task_mgr_client().await?;
        let client = self.system_config_client().await?;

        let _ = task_mgr
            .update_task(
                task_id,
                Some(TaskStatus::Running),
                Some(5.0),
                Some("Validating repo content".to_string()),
                None,
            )
            .await;

        if !self
            .find_matching_specs(spec.app_id(), Some(spec.user_id.as_str()))
            .await?
            .is_empty()
        {
            return Err(RPCErrors::ReasonError(format!(
                "App `{}` is already installed for user `{}`",
                spec.app_id(),
                spec.user_id
            )));
        }

        let download_proof = self
            .ensure_content_pinned(&spec, task_id, root_id.as_str())
            .await?;
        let content_id = Self::resolve_content_id(&spec)?;

        let _ = task_mgr
            .update_task(
                task_id,
                None,
                Some(60.0),
                Some("Writing app spec".to_string()),
                Some(json!({
                    "content_id": content_id,
                })),
            )
            .await;

        let mut next_spec = spec.clone();
        next_spec.app_index = self.get_next_app_index().await?;
        next_spec.state = ServiceState::New;
        let spec_path = Self::spec_storage_path(&next_spec);
        Self::set_spec_at(&client, &spec_path, &next_spec).await?;

        let repo = self.repo_client().await?;
        let install_proof = self.build_install_proof(
            &next_spec,
            &content_id,
            Some(download_proof.gen_obj_id().0),
        )?;
        repo.add_proof(RepoProof::action(install_proof)).await?;

        let mut data = json!({
            "spec_path": spec_path,
            "app_index": next_spec.app_index,
            "spec_id": Self::service_spec_id(&next_spec),
        });

        if Self::should_wait_for_instance(&next_spec) {
            let instance = self.wait_for_instance_ready(&next_spec).await?;
            data["instance"] = serde_json::to_value(instance).map_err(|error| {
                RPCErrors::ReasonError(format!("Serialize instance report failed: {error}"))
            })?;
        }

        task_mgr
            .update_task(
                task_id,
                Some(TaskStatus::Completed),
                Some(100.0),
                Some("App installed".to_string()),
                Some(data),
            )
            .await?;
        Ok(())
    }

    async fn run_start_task(
        &self,
        app_id: String,
        task_id: i64,
    ) -> Result<(), RPCErrors> {
        let task_mgr = self.task_mgr_client().await?;
        let client = self.system_config_client().await?;
        let (spec_key, mut spec) = self.get_single_matching_spec(&app_id, None).await?;

        if spec.state == ServiceState::Deleted {
            return Err(RPCErrors::ReasonError(format!(
                "App `{app_id}` has been deleted and can not be started"
            )));
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
        Ok(())
    }

    fn removable_data_path(path: &Path, app_id: &str) -> bool {
        let raw = path.to_string_lossy();
        raw.starts_with("/opt/buckyos/data/")
            && (raw.contains(&format!("/{app_id}/")) || raw.ends_with(&format!("/{app_id}")))
    }

    async fn remove_app_data(&self, spec: &AppServiceSpec) -> Result<(), RPCErrors> {
        for host_path in spec.install_config.data_mount_point.values() {
            let path = PathBuf::from(host_path);
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
                }
                Ok(_) => {
                    fs::remove_file(&path).await.map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "Remove app data file `{}` failed: {error}",
                            path.display()
                        ))
                    })?;
                }
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// 获取下一个可用的 app_index（用于新安装应用的自动分配）。
    /// 流程：遍历 users/{uid}/apps|agents/*/spec，取最大 app_index + 1。
    async fn get_next_app_index(&self) -> Result<u16, RPCErrors> {
        let client = self.system_config_client().await?;
        let users = Self::list_children(&client, "users").await?;
        let mut max_app_index = 0u16;

        for user_id in users {
            for base in ["apps", "agents"] {
                let base_key = format!("users/{user_id}/{base}");
                let app_ids = Self::list_children(&client, &base_key).await?;
                for app_id in app_ids {
                    let spec_key = format!("{base_key}/{app_id}/spec");
                    if let Some(spec) =
                        Self::get_optional_json::<AppServiceSpec>(&client, &spec_key).await?
                    {
                        max_app_index = max_app_index.max(spec.app_index);
                    }
                }
            }
        }

        max_app_index
            .checked_add(1)
            .ok_or_else(|| RPCErrors::ReasonError("app_index overflow".to_string()))
    }

    /// 安装应用。成功返回 task_id 供进度追踪；失败返回错误。
    /// spec 中的 app_index 将被忽略，由 get_next_app_index 自动分配。
    ///
    /// 流程：
    /// 1. 校验 content_id 已在 RepoService 中 collected
    /// 2. 通过task_mgr创建一个可以跟踪的App Install Task
    /// 3. 返回install task_id
    /// 下面流程在工作线程中执行，并会根据task_id更新状态
    /// 3. 通过task_mgr::create_download_task下载app_pkg_meta(会自动下载所有的sub pkg)
    /// 4. 等待下载完成，再次检查 app_pkg_meta 已经在named_store中ready
    /// 5. 调用repo::pin(app_pkg_meta, download_action)将app_pkg_meta pin到RepoService中
    /// 6. get_next_app_index() 分配 app_index，写入 spec
    /// 7. 确定存储路径：users/{uid}/apps/{app}/spec 或 users/{uid}/agents/{app}/spec
    /// 8. 写 spec（state=New）到 system_config
    /// 9. repo.add_proof(install_action)

    pub async fn install_app(&self, spec: &AppServiceSpec) -> Result<u64, RPCErrors> {
        let content_id = Self::resolve_content_id(spec)?;
        let (task_mgr, task_id, root_id) = self
            .create_task(
                format!("Install app {}", spec.app_id()),
                INSTALL_TASK_TYPE,
                json!({
                    "app_id": spec.app_id(),
                    "user_id": spec.user_id,
                    "version": spec.app_doc.version,
                    "content_id": content_id,
                }),
                spec.user_id.as_str(),
                spec.app_id(),
            )
            .await?;

        let installer = self.clone();
        let spec = spec.clone();
        tokio::spawn(async move {
            if let Err(error) = installer.run_install_task(spec, task_id, root_id).await {
                warn!("install app task {} failed: {}", task_id, error);
                let _ = task_mgr.mark_task_as_failed(task_id, &error.to_string()).await;
            }
        });

        Ok(task_id as u64)
    }

    /// 卸载应用。必须先 stop_app 才能返回成功。
    ///
    /// 流程：
    /// 1. get_app_service_spec(app_id)，校验 spec 存在
    /// 2. stop_app(app_id) — 改 spec.state=Stopped，触发调度器删除 instance config
    /// 3. spec.state → Deleted，写回 system_config
    /// 4. 等待调度器 RemoveInstance（删除 nodes/{node}/config 中的实例配置）
    /// 5. 若 is_remove_data：清理应用数据目录
    pub async fn uninstall_app(
        &self,
        app_id: &str,
        is_remove_data: bool,
    ) -> Result<(), RPCErrors> {
        let client = self.system_config_client().await?;
        let (spec_key, mut spec) = self.get_single_matching_spec(app_id, None).await?;

        self.stop_app(app_id).await?;
        spec.state = ServiceState::Deleted;
        Self::set_spec_at(&client, &spec_key, &spec).await?;
        self.wait_for_instances_removed(&spec).await?;

        if is_remove_data {
            self.remove_app_data(&spec).await?;
        }

        Ok(())
    }

    /// 停止应用。
    ///
    /// 流程：
    /// 1. get_app_service_spec(app_id)
    /// 2. spec.state → Stopped，写回 system_config
    /// 3. 调度器 schedule_loop 检测到 state 变化，删除 nodes/{node}/config.apps 中该应用的 app_service_instance_config
    /// 4. node-daemon 读 config 收敛，停止容器
    pub async fn stop_app(&self, app_id: &str) -> Result<(), RPCErrors> {
        let client = self.system_config_client().await?;
        let (spec_key, mut spec) = self.get_single_matching_spec(app_id, None).await?;

        if spec.state == ServiceState::Stopped || spec.state == ServiceState::Deleted {
            return Ok(());
        }

        spec.state = ServiceState::Stopped;
        Self::set_spec_at(&client, &spec_key, &spec).await?;

        if Self::should_wait_for_instance(&spec) {
            self.wait_for_instances_removed(&spec).await?;
        }

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
    pub async fn start_app(&self, app_id: &str) -> Result<u64, RPCErrors> {
        let spec = self.get_app_service_spec(app_id).await?;
        let (task_mgr, task_id, _root_id) = self
            .create_task(
                format!("Start app {app_id}"),
                START_TASK_TYPE,
                json!({
                    "app_id": app_id,
                    "user_id": spec.user_id,
                }),
                spec.user_id.as_str(),
                app_id,
            )
            .await?;

        let installer = self.clone();
        let app_id = app_id.to_string();
        tokio::spawn(async move {
            if let Err(error) = installer.run_start_task(app_id, task_id).await {
                warn!("start app task {} failed: {}", task_id, error);
                let _ = task_mgr.mark_task_as_failed(task_id, &error.to_string()).await;
            }
        });

        Ok(task_id as u64)
    }

    /// 升级应用。调用前 app 需已 stop（无可用 instance）。
    ///
    /// 流程： 基本和install_app类似，也需要创建一个task来跟踪升级流程。注意要pkg下载完成后才开始停止旧版本实例并启动新版本
    /// 1. stop_app(app_id) — 确保无运行实例
    /// 2. 覆盖 users/{uid}/apps|agents/{app}/spec 为新的 AppServiceSpec
    /// 3. spec.state=New，触发调度器重新选点并分配新版本 app_service_instance_config
    /// 4. repo.add_proof(install_action) 新版本安装证明
    pub async fn upgrade_app(&self, spec: &AppServiceSpec) -> Result<(), RPCErrors> {
        let client = self.system_config_client().await?;
        let (spec_key, current_spec) = self
            .get_single_matching_spec(spec.app_id(), Some(spec.user_id.as_str()))
            .await?;

        self.stop_app(spec.app_id()).await?;

        let mut next_spec = spec.clone();
        next_spec.app_index = current_spec.app_index;
        next_spec.state = ServiceState::New;

        let content_id = Self::resolve_content_id(&next_spec)?;
        let repo_status = self.load_repo_record_status(&content_id).await?;
        let status = repo_status.ok_or_else(|| {
            RPCErrors::ReasonError(format!(
                "Content `{content_id}` is not collected in RepoService"
            ))
        })?;
        let download_base = if status == REPO_STATUS_PINNED {
            self.latest_action_proof_id(&content_id, REPO_PROOF_TYPE_DOWNLOAD)
                .await?
        } else {
            let proof = self
                .ensure_content_pinned(&next_spec, 0, "upgrade")
                .await?;
            Some(proof.gen_obj_id().0)
        };

        Self::set_spec_at(&client, &spec_key, &next_spec).await?;

        let repo = self.repo_client().await?;
        let install_proof = self.build_install_proof(&next_spec, &content_id, download_base)?;
        repo.add_proof(RepoProof::action(install_proof)).await?;

        if Self::should_wait_for_instance(&next_spec) {
            let _ = self.wait_for_instance_ready(&next_spec).await?;
        }

        Ok(())
    }

    /// 查询应用 spec。
    /// 流程：从 system_config 读取 users/{uid}/apps/{app}/spec 或 users/{uid}/agents/{app}/spec。
    pub async fn get_app_service_spec(&self, app_id: &str) -> Result<AppServiceSpec, RPCErrors> {
        let (_, spec) = self.get_single_matching_spec(app_id, None).await?;
        Ok(spec)
    }

    /// 查询应用实例状态（ServiceInstanceReportInfo）。
    /// 流程：从 services/{spec}/instances/{node} 或 nodes/{node}/config 聚合实例上报信息。
    pub async fn get_app_service_instance_config(
        &self,
        app_id: &str,
    ) -> Result<ServiceInstanceReportInfo, RPCErrors> {
        let client = self.system_config_client().await?;
        let spec = self.get_app_service_spec(app_id).await?;
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

}
