//! Prepare / Deploy / Activate / rollback 的生产实现（协议 §3.2 Stage 5-7，
//! D3 记录纪律，P4.1-P4.4）。
//!
//! 顺序纪律：
//! - Prepare：构造 spec、materialize 内容、检查冲突、CAS 分配 app_index，
//!   然后先写 `install_record(state=prepared)` —— 此时还没有任何系统副作用
//!   会触发 scheduler；
//! - Deploy：写 spec 是部署事务的开始点（不得提前）；旧 spec 已在 Prepare
//!   存进回滚材料；
//! - Activate：等待明确的运行证据 + 健康检查；成功后才依次
//!   `install_record(state=installed)` -> installed proof -> 任务完成；
//! - rollback：恢复旧 spec（升级）或删除新 spec（全新安装），更新同一记录。

use crate::app_install_driver::ProductionInstallDriver;
use crate::app_install_engine::InstallTaskView;
use buckyos_api::{
    app_instance_id, get_buckyos_api_runtime, install_record_key, zone_app_spec_key, AppClass,
    AppDoc, AppInstallTaskData, AppServiceSpec, AppType, ContentLocation, InstallError,
    InstallErrorCode, InstallParams, InstallRecord, InstallRecordState, InstallStage,
    InstallTaskResult, MountPointConfig, PreparedDeployment, ServiceEndpointConfig,
    ServiceExposeConfig, ServiceExposeRouteTips, ServiceInstanceReportInfo, ServiceInstanceState,
    ServiceSpecConfig, ServiceState, SystemConfigClient, SystemConfigError,
    APP_INSTALL_SCHEMA_VERSION, SYSTEM_APP_OWNER_ID,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::{info, warn};
use ndn_lib::{build_obj_id, ActionObject, NamedObject, ObjId, ACTION_TYPE_INSTALLED};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const APP_INDEX_SEQ_KEY: &str = "system/app_installer/app_index_seq";
const ACTIVATE_TIMEOUT_SECS: u64 = 120;
const ACTIVATE_POLL_INTERVAL_SECS: u64 = 2;
const PROOF_EXPIRE_SECS: u64 = 365 * 24 * 60 * 60;

fn stage_err(
    stage: InstallStage,
    code: InstallErrorCode,
    retryable: bool,
    message: impl Into<String>,
) -> InstallError {
    InstallError::new(stage, code, retryable, message)
}

async fn system_config_client() -> Result<Arc<SystemConfigClient>, InstallError> {
    let runtime = get_buckyos_api_runtime().map_err(|err| {
        stage_err(
            InstallStage::Prepare,
            InstallErrorCode::Internal,
            true,
            format!("get runtime failed: {err}"),
        )
    })?;
    runtime.get_system_config_client().await.map_err(|err| {
        stage_err(
            InstallStage::Prepare,
            InstallErrorCode::Internal,
            true,
            format!("get system config client failed: {err}"),
        )
    })
}

impl ProductionInstallDriver {
    // ------------------------------------------------------------------
    // Prepare
    // ------------------------------------------------------------------

    pub(crate) async fn prepare_impl(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<PreparedDeployment, InstallError> {
        let plan = data.state.plan.clone().ok_or_else(|| {
            stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                false,
                "prepare without plan",
            )
        })?;
        let approval = data.state.approval.clone().ok_or_else(|| {
            stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                false,
                "prepare without approval",
            )
        })?;
        if approval.plan_fingerprint != plan.plan_fingerprint {
            return Err(stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Conflict,
                false,
                "approval no longer matches plan fingerprint",
            ));
        }
        let doc_value = data.state.resolved_app_doc.clone().ok_or_else(|| {
            stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                false,
                "prepare without resolved app document",
            )
        })?;
        let app_doc: AppDoc = serde_json::from_value(doc_value).map_err(|err| {
            stage_err(
                InstallStage::Prepare,
                InstallErrorCode::VerificationFailed,
                false,
                format!("persisted app document invalid: {err}"),
            )
        })?;

        let user_id = match data.request.app_class {
            AppClass::UserInstalled => data.request.user_id.clone(),
            AppClass::ZoneInstalled => SYSTEM_APP_OWNER_ID.to_string(),
            AppClass::SystemBuiltin => {
                return Err(stage_err(
                    InstallStage::Prepare,
                    InstallErrorCode::ConfigBlocked,
                    false,
                    "system built-in apps are not created by the installer",
                ))
            }
        };
        let app_id = app_doc.name.clone();
        let is_agent = app_doc.get_app_type() == AppType::Agent;
        if is_agent && data.request.app_class != AppClass::UserInstalled {
            return Err(stage_err(
                InstallStage::Prepare,
                InstallErrorCode::ConfigBlocked,
                false,
                "agents can only be user_installed",
            ));
        }
        let is_update = view.task_type == buckyos_api::TASK_DATA_TYPE_APP_UPDATE;
        let spec_path = if is_agent {
            format!("users/{user_id}/agents/{app_id}/spec")
        } else if data.request.app_class == AppClass::ZoneInstalled {
            zone_app_spec_key(&app_id)
        } else {
            format!("users/{user_id}/apps/{app_id}/spec")
        };

        let client = system_config_client().await?;

        // 已安装冲突：全新安装要求不存在旧 spec；升级要求存在。
        let previous_spec: Option<AppServiceSpec> = match client.get(&spec_path).await {
            Ok(value) => Some(serde_json::from_str(&value.value).map_err(|err| {
                stage_err(
                    InstallStage::Prepare,
                    InstallErrorCode::Internal,
                    false,
                    format!("existing spec at `{spec_path}` unreadable: {err}"),
                )
            })?),
            Err(SystemConfigError::KeyNotFound(_)) => None,
            Err(err) => {
                return Err(stage_err(
                    InstallStage::Prepare,
                    InstallErrorCode::Internal,
                    true,
                    format!("read `{spec_path}` failed: {err}"),
                ))
            }
        };
        if is_update && previous_spec.is_none() {
            return Err(stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Conflict,
                false,
                format!("app `{app_id}` is not installed; can not update"),
            ));
        }
        if !is_update && previous_spec.is_some() {
            return Err(stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Conflict,
                false,
                format!("app `{app_id}` is already installed for `{user_id}`"),
            ));
        }

        // 目标 Node 可用性。
        if let Some(node_id) = plan.target.node_id.as_deref() {
            let node_key = format!("devices/{node_id}/info");
            if client.get(&node_key).await.is_err() {
                return Err(stage_err(
                    InstallStage::Prepare,
                    InstallErrorCode::ConfigBlocked,
                    true,
                    format!("target node `{node_id}` has no device info"),
                ));
            }
        }

        // 为部署 materialize 已验证内容（显式动作，不改变 Resolve/Verify 结论）。
        let materialized = self.materialize_for_deploy(data).await?;

        // 端口冲突检查（expose_port 与现有 spec 冲突即 CONFIG_BLOCKED）。
        let mut install_config = plan.service_spec_config.clone();
        let guest_allowed = data.request.app_class == AppClass::UserInstalled
            && previous_spec
                .as_ref()
                .map(|spec| {
                    spec.spec_config
                        .expose_config
                        .values()
                        .any(|config| config.allow_guest)
                })
                .unwrap_or(false);
        for expose in install_config.expose_config.values_mut() {
            expose.allow_guest = guest_allowed;
        }
        check_expose_conflicts(&client, &install_config, &spec_path).await?;

        // app_index：升级沿用旧值；新装用 CAS 序列 key 分配（修扫描竞态）。
        let app_index = match previous_spec.as_ref() {
            Some(previous) => previous.app_index,
            None => allocate_app_index(&client).await?,
        };
        let new_spec = AppServiceSpec {
            app_doc,
            app_index,
            user_id: user_id.clone(),
            app_class: data.request.app_class,
            permission: plan.install_params.permissions.clone(),
            enable: plan.install_params.auto_start,
            expected_instance_count: 1,
            state: ServiceState::New,
            spec_config: install_config,
        };

        let prepared = PreparedDeployment {
            spec_path: spec_path.clone(),
            service_spec_id: format!("{app_id}@{user_id}"),
            app_index,
            previous_spec,
            new_spec,
            materialized_objects: materialized,
            prepared_at: buckyos_get_unix_timestamp(),
        };

        // 先写 install_record(state=prepared)，成功后（Deploy）才写 spec。
        let record = build_install_record(view, data, InstallRecordState::Prepared, None)?;
        write_install_record(&client, &record, is_agent).await?;

        Ok(prepared)
    }

    // ------------------------------------------------------------------
    // Deploy
    // ------------------------------------------------------------------

    pub(crate) async fn deploy_impl(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let prepared = data.state.prepared.clone().ok_or_else(|| {
            stage_err(
                InstallStage::Deploy,
                InstallErrorCode::Internal,
                false,
                "deploy without prepared deployment",
            )
        })?;
        let client = system_config_client().await?;

        let raw = serde_json::to_string(&prepared.new_spec).map_err(|err| {
            stage_err(
                InstallStage::Deploy,
                InstallErrorCode::Internal,
                false,
                format!("serialize spec failed: {err}"),
            )
        })?;
        client
            .set(prepared.spec_path.as_str(), raw.as_str())
            .await
            .map_err(|err| {
                stage_err(
                    InstallStage::Deploy,
                    InstallErrorCode::DeployFailed,
                    true,
                    format!("write spec `{}` failed: {err}", prepared.spec_path),
                )
            })?;
        info!(
            "install task {}: spec written at `{}` (deploy transaction started)",
            view.id, prepared.spec_path
        );

        let is_agent = prepared.spec_path.contains("/agents/");
        let record = build_install_record(view, data, InstallRecordState::Deploying, None)?;
        write_install_record(&client, &record, is_agent).await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Activate + 健康检查 + record/proof（成功后才写，且只写一次）
    // ------------------------------------------------------------------

    pub(crate) async fn activate_impl(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<InstallTaskResult, InstallError> {
        let prepared = data.state.prepared.clone().ok_or_else(|| {
            stage_err(
                InstallStage::Activate,
                InstallErrorCode::Internal,
                false,
                "activate without prepared deployment",
            )
        })?;
        let client = system_config_client().await?;
        let app_type = prepared.new_spec.app_doc.get_app_type();
        let is_agent = app_type == AppType::Agent;

        let instance_node = match app_type {
            AppType::Web => {
                wait_for_static_web_evidence(&client, &prepared).await?;
                None
            }
            _ => Some(wait_for_instance_started(&client, &prepared).await?),
        };

        // 健康检查通过：更新 install_record=installed -> 写 installed proof。
        let record = build_install_record(view, data, InstallRecordState::Installed, None)?;
        write_install_record(&client, &record, is_agent).await?;

        let proof_id = write_installed_proof(view, data, &prepared).await?;
        if proof_id.is_some() {
            let mut record_with_proof = record.clone();
            record_with_proof.proof_id = proof_id.clone();
            record_with_proof.updated_at = buckyos_get_unix_timestamp();
            write_install_record(&client, &record_with_proof, is_agent).await?;
        }

        Ok(InstallTaskResult {
            install_record_key: Some(install_record_key(
                prepared.new_spec.app_class,
                prepared.new_spec.user_id.as_str(),
                prepared.new_spec.app_doc.name.as_str(),
                is_agent,
            )),
            proof_id,
            instance_node_id: instance_node,
            completed_at: Some(buckyos_get_unix_timestamp()),
        })
    }

    // ------------------------------------------------------------------
    // rollback（本机部署回滚；绝不改变 resolver 状态）
    // ------------------------------------------------------------------

    pub(crate) async fn rollback_impl(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let Some(prepared) = data.state.prepared.clone() else {
            return Ok(()); // spec 从未写过，无需回滚。
        };
        let client = system_config_client().await?;
        match prepared.previous_spec.as_ref() {
            Some(previous) => {
                let raw = serde_json::to_string(previous).map_err(|err| {
                    stage_err(
                        InstallStage::Deploy,
                        InstallErrorCode::Internal,
                        false,
                        format!("serialize previous spec failed: {err}"),
                    )
                })?;
                client
                    .set(prepared.spec_path.as_str(), raw.as_str())
                    .await
                    .map_err(|err| {
                        stage_err(
                            InstallStage::Deploy,
                            InstallErrorCode::DeployFailed,
                            true,
                            format!("restore previous spec failed: {err}"),
                        )
                    })?;
                info!(
                    "install task {}: previous spec restored at `{}`",
                    view.id, prepared.spec_path
                );
            }
            None => {
                if let Err(err) = client.delete(prepared.spec_path.as_str()).await {
                    match err {
                        SystemConfigError::KeyNotFound(_) => {}
                        other => {
                            return Err(stage_err(
                                InstallStage::Deploy,
                                InstallErrorCode::DeployFailed,
                                true,
                                format!("remove deployed spec failed: {other}"),
                            ))
                        }
                    }
                }
                info!(
                    "install task {}: deployed spec removed at `{}`",
                    view.id, prepared.spec_path
                );
            }
        }

        let is_agent = prepared.spec_path.contains("/agents/");
        let record = build_install_record(
            view,
            data,
            InstallRecordState::RolledBack,
            data.state.last_error.clone(),
        )?;
        write_install_record(&client, &record, is_agent).await?;
        Ok(())
    }

    /// 把 plan 中位于 pikg 的已验证内容显式写入 NamedStore，供现有
    /// node-daemon/PackageEnv 加载。这是部署适配动作（P1.3 materialize），
    /// 不是安全前提，也不改变任何 Resolve/Verify 结论。
    async fn materialize_for_deploy(
        &self,
        data: &AppInstallTaskData,
    ) -> Result<Vec<String>, InstallError> {
        let plan = data.state.plan.as_ref().ok_or_else(|| {
            stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                false,
                "materialize without plan",
            )
        })?;
        let pikg_contents: Vec<_> = plan
            .required_contents
            .iter()
            .filter(|content| matches!(content.location, ContentLocation::Pikg))
            .cloned()
            .collect();
        if pikg_contents.is_empty() {
            return Ok(vec![]);
        }
        let Some(reader) = self.open_candidate_pikg(data).await? else {
            return Err(stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                false,
                "plan references pikg contents but pikg is unavailable",
            ));
        };
        let runtime = get_buckyos_api_runtime().map_err(|err| {
            stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                true,
                format!("get runtime failed: {err}"),
            )
        })?;
        let named_store = runtime.get_named_store().await.map_err(|err| {
            stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                true,
                format!("open named store failed: {err}"),
            )
        })?;

        let mut materialized = Vec::new();
        for content in pikg_contents {
            if content.format.as_deref() == Some("named_object") {
                // Package Meta 结构化对象：读出（含 ObjId 复核）后写 NamedStore。
                let Ok(obj_id) = ObjId::new(&content.content_id) else {
                    continue;
                };
                if let Ok(Some(body)) = reader.read_object(&obj_id).await {
                    named_store
                        .put_object(&obj_id, body.as_str())
                        .await
                        .map_err(|err| {
                            stage_err(
                                InstallStage::Prepare,
                                InstallErrorCode::PrepareFailed,
                                true,
                                format!("materialize object `{obj_id}` failed: {err}"),
                            )
                        })?;
                    materialized.push(obj_id.to_string());
                }
                continue;
            }

            // payload：从 pikg 验证后落临时文件，再按 chunk id 写 NamedStore。
            let entry = reader
                .inspection()
                .content_entry(&content.content_id)
                .cloned();
            let Some(entry) = entry else { continue };
            let chunk_id_str = plan
                .selected_packages
                .iter()
                .find(|package| package.sub_pkg_name == entry.sub_pkg_name)
                .and_then(|package| package.package_meta_id.as_ref())
                .map(|meta_id| meta_id.to_string())
                .and_then(|meta_key| {
                    // 从 pikg 的 package_objects 读 meta.content（chunk id）。
                    reader
                        .inspection()
                        .package_meta
                        .package_objects
                        .get(&meta_key)
                        .and_then(|meta| meta.get("content"))
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string())
                });
            let Some(chunk_id_str) = chunk_id_str else {
                continue;
            };
            let Ok(chunk_id) = ndn_lib::ChunkId::new(chunk_id_str.as_str()) else {
                continue;
            };
            if named_store.have_chunk(&chunk_id).await {
                continue;
            }

            let tmp = std::env::temp_dir().join(format!(
                "pikg-materialize-{}-{}",
                std::process::id(),
                entry.path.replace('/', "_")
            ));
            reader
                .copy_content_to_file(&content.content_id, &tmp)
                .await
                .map_err(|err| {
                    stage_err(
                        InstallStage::Prepare,
                        InstallErrorCode::PrepareFailed,
                        true,
                        format!("materialize payload `{}` failed: {err}", content.content_id),
                    )
                })?;
            let file = tokio::fs::File::open(&tmp).await.map_err(|err| {
                stage_err(
                    InstallStage::Prepare,
                    InstallErrorCode::Internal,
                    true,
                    format!("open materialized payload failed: {err}"),
                )
            })?;
            let result = named_store
                .put_chunk_by_reader(&chunk_id, entry.size, Box::pin(file))
                .await;
            let _ = tokio::fs::remove_file(&tmp).await;
            result.map_err(|err| {
                stage_err(
                    InstallStage::Prepare,
                    InstallErrorCode::PrepareFailed,
                    true,
                    format!("write chunk `{chunk_id_str}` into named store failed: {err}"),
                )
            })?;
            materialized.push(chunk_id_str);
        }
        Ok(materialized)
    }
}

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

pub(crate) fn build_install_config(
    app_id: &str,
    app_doc: &AppDoc,
    install_params: &InstallParams,
) -> (ServiceSpecConfig, Vec<String>) {
    let tips = &app_doc.service_config_tips;
    let mut issues = Vec::new();
    let mut install_config = ServiceSpecConfig {
        data_mount_point: HashMap::new(),
        local_cache_mount_point: tips
            .local_cache_mount_points
            .iter()
            .map(|(path, info)| {
                (
                    path.clone(),
                    mount_config_from_tip(path, info.as_ref(), "read_write"),
                )
            })
            .collect(),
        external_mount_point: HashMap::new(),
        rdb_instances: tips.rdb_instances.clone(),
        instance_volume: tips.instance_volume.clone(),
        runtime_caps: tips.runtime_caps.clone(),
        container_param: tips.container_param.clone(),
        start_param: tips.start_param.clone(),
        ..Default::default()
    };

    for (service_name, endpoint) in &tips.service_endpoints {
        let setting = install_params.service_settings.services.get(service_name);
        if matches!(setting, Some(setting) if !setting.enabled) {
            if endpoint.required {
                issues.push(format!(
                    "required service endpoint `{service_name}` is disabled"
                ));
            }
            continue;
        }
        install_config.service_config.insert(
            service_name.clone(),
            ServiceEndpointConfig {
                protocol: endpoint.protocol,
                inner_port: endpoint.inner_port,
            },
        );
        if let Some(expose) = setting.and_then(|setting| setting.expose.as_ref()) {
            install_config.expose_config.insert(
                service_name.clone(),
                ServiceExposeConfig {
                    route: expose.route.clone(),
                    scope: expose.scope.clone(),
                    allow_guest: expose.allow_guest,
                    bind_address: None,
                },
            );
        } else if setting.is_none() {
            let Some(expose) = &endpoint.expose else {
                continue;
            };
            let expose_config = match &expose.route {
                ServiceExposeRouteTips::Web => ServiceExposeConfig::web(
                    vec![app_id.to_string()],
                    expose.scope.clone(),
                    expose.allow_guest,
                ),
                ServiceExposeRouteTips::Port {
                    preferred_port: Some(port),
                } => ServiceExposeConfig::port(*port, expose.scope.clone(), expose.allow_guest),
                ServiceExposeRouteTips::Port {
                    preferred_port: None,
                } => continue,
            };
            install_config
                .expose_config
                .insert(service_name.clone(), expose_config);
        }
    }

    for service_name in install_params.service_settings.services.keys() {
        if !tips.service_endpoints.contains_key(service_name) {
            issues.push(format!("unknown service endpoint `{service_name}`"));
        }
    }

    if app_doc.get_app_type() == AppType::Web
        && !install_config.expose_config.contains_key("www")
        && !install_params.service_settings.services.contains_key("www")
    {
        install_config.expose_config.insert(
            "www".to_string(),
            ServiceExposeConfig::web(vec![app_id.to_string()], String::new(), true),
        );
    }

    apply_selected_mounts(
        "data",
        &tips.data_mount_points,
        &install_params.data_mount_points,
        &mut install_config.data_mount_point,
        &mut issues,
    );
    apply_selected_mounts(
        "local_cache",
        &tips.local_cache_mount_points,
        &install_params.local_cache_mount_points,
        &mut install_config.local_cache_mount_point,
        &mut issues,
    );
    apply_selected_mounts(
        "external",
        &tips.external_mount_points,
        &install_params.external_mount_points,
        &mut install_config.external_mount_point,
        &mut issues,
    );
    install_config.bash_envs = install_params.bash_envs.clone();
    if let Some(res_pool_id) = install_params.res_pool_id.as_ref() {
        install_config.res_pool_id = res_pool_id.clone();
    }

    for (name, info) in &tips.bash_envs {
        if info.required && !install_config.bash_envs.contains_key(name) {
            issues.push(format!("required bash environment `{name}` is missing"));
        }
    }
    for (kind, mounts) in [
        ("data", &install_config.data_mount_point),
        ("local_cache", &install_config.local_cache_mount_point),
        ("external", &install_config.external_mount_point),
    ] {
        for (container_path, mount) in mounts {
            if !is_safe_absolute_path(container_path) {
                issues.push(format!(
                    "{kind} mount container path `{}` is unsafe",
                    container_path.display()
                ));
            }
            if !is_safe_mount_target_path(&mount.target_path) {
                issues.push(format!(
                    "{kind} mount target path `{}` is unsafe",
                    mount.target_path.display()
                ));
            }
            if !matches!(
                mount.access.as_str(),
                "" | "read_only" | "ro" | "read_write" | "read_write_append" | "rw"
            ) {
                issues.push(format!(
                    "{kind} mount `{}` has invalid access `{}`",
                    container_path.display(),
                    mount.access
                ));
            }
        }
    }

    (install_config, issues)
}

fn apply_selected_mounts(
    kind: &str,
    declared: &HashMap<PathBuf, Option<buckyos_api::MountPointInfo>>,
    selected: &HashMap<PathBuf, MountPointConfig>,
    output: &mut HashMap<PathBuf, MountPointConfig>,
    issues: &mut Vec<String>,
) {
    for (container_path, config) in selected {
        let Some(tip) = declared.get(container_path) else {
            issues.push(format!(
                "unknown {kind} mount point `{}`",
                container_path.display()
            ));
            continue;
        };
        if let Some(tip) = tip {
            if matches!(tip.access.as_str(), "read_only" | "ro")
                && matches!(
                    config.access.as_str(),
                    "read_write" | "read_write_append" | "rw"
                )
            {
                issues.push(format!(
                    "{kind} mount `{}` can not exceed the App Document read-only access",
                    container_path.display()
                ));
                continue;
            }
        }
        output.insert(container_path.clone(), config.clone());
    }
}

fn is_safe_absolute_path(path: &std::path::Path) -> bool {
    path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}

fn is_safe_mount_target_path(path: &std::path::Path) -> bool {
    !path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::CurDir
                | std::path::Component::Prefix(_)
        )
    })
}

fn mount_config_from_tip(
    path: &std::path::Path,
    info: Option<&buckyos_api::MountPointInfo>,
    default_access: &str,
) -> MountPointConfig {
    let target_path = info
        .map(|info| info.mount_point_name.as_str())
        .filter(|name| !name.is_empty())
        .map(Into::into)
        .unwrap_or_else(|| path.to_path_buf());
    let access = info
        .map(|info| info.access.as_str())
        .filter(|access| !access.is_empty())
        .unwrap_or(default_access)
        .to_string();
    MountPointConfig {
        target_path,
        access,
    }
}

/// expose_port 冲突检查：与其它已安装 spec 的显式端口重复即 CONFIG_BLOCKED。
async fn check_expose_conflicts(
    client: &SystemConfigClient,
    install_config: &ServiceSpecConfig,
    own_spec_path: &str,
) -> Result<(), InstallError> {
    let requested: Vec<u16> = install_config
        .expose_config
        .values()
        .filter_map(ServiceExposeConfig::expose_port)
        .collect();
    if requested.is_empty() {
        return Ok(());
    }

    for key in installed_spec_paths(client).await {
        if key == own_spec_path {
            continue;
        }
        let Ok(value) = client.get(&key).await else {
            continue;
        };
        let Ok(spec) = serde_json::from_str::<AppServiceSpec>(&value.value) else {
            continue;
        };
        // 已删除的 spec 在 GC 前仍会保留，不再占用端口。
        if spec.state == ServiceState::Deleted {
            continue;
        }
        for expose in spec.spec_config.expose_config.values() {
            if let Some(port) = expose.expose_port() {
                if requested.contains(&port) {
                    return Err(stage_err(
                        InstallStage::Prepare,
                        InstallErrorCode::ConfigBlocked,
                        false,
                        format!("expose port {port} conflicts with installed app at `{key}`"),
                    ));
                }
            }
        }
    }
    Ok(())
}

async fn installed_spec_paths(client: &SystemConfigClient) -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(users) = client.list("users").await {
        for user in users {
            for base in ["apps", "agents"] {
                if let Ok(apps) = client.list(format!("users/{user}/{base}").as_str()).await {
                    for app in apps {
                        paths.push(format!("users/{user}/{base}/{app}/spec"));
                    }
                }
            }
        }
    }
    if let Ok(apps) = client.list("zone/apps").await {
        paths.extend(apps.into_iter().map(|app| format!("zone/apps/{app}/spec")));
    }
    paths
}

/// app_index 分配：专用序列 key + exec_tx CAS，修"扫描 max+1"并发竞态。
pub(crate) async fn allocate_app_index(client: &SystemConfigClient) -> Result<u16, InstallError> {
    for _ in 0..8 {
        match client.get(APP_INDEX_SEQ_KEY).await {
            Ok(current) => {
                let value: u64 = current.value.trim().parse().unwrap_or(0);
                let next = value + 1;
                if next > u16::MAX as u64 {
                    return Err(stage_err(
                        InstallStage::Prepare,
                        InstallErrorCode::Internal,
                        false,
                        "app_index sequence overflow",
                    ));
                }
                let mut actions = std::collections::HashMap::new();
                actions.insert(
                    APP_INDEX_SEQ_KEY.to_string(),
                    buckyos_kit::KVAction::Update(next.to_string()),
                );
                match client
                    .exec_tx(
                        actions,
                        Some((APP_INDEX_SEQ_KEY.to_string(), current.version)),
                    )
                    .await
                {
                    Ok(_) => return Ok(next as u16),
                    Err(err) => {
                        warn!("app_index CAS conflict, retrying: {err}");
                        continue;
                    }
                }
            }
            Err(SystemConfigError::KeyNotFound(_)) => {
                // 初始化：从现有 spec 扫描出基线，避免与历史分配撞号。
                let baseline = scan_max_app_index(client).await.unwrap_or(0);
                let mut actions = std::collections::HashMap::new();
                actions.insert(
                    APP_INDEX_SEQ_KEY.to_string(),
                    buckyos_kit::KVAction::Create(baseline.to_string()),
                );
                if let Err(err) = client.exec_tx(actions, None).await {
                    warn!("init app_index seq failed, retrying: {err}");
                }
                continue;
            }
            Err(err) => {
                return Err(stage_err(
                    InstallStage::Prepare,
                    InstallErrorCode::Internal,
                    true,
                    format!("read app_index sequence failed: {err}"),
                ))
            }
        }
    }
    Err(stage_err(
        InstallStage::Prepare,
        InstallErrorCode::Internal,
        true,
        "app_index allocation contention did not converge",
    ))
}

async fn scan_max_app_index(client: &SystemConfigClient) -> Result<u16, InstallError> {
    let mut max_index = 0u16;
    for key in installed_spec_paths(client).await {
        if let Ok(value) = client.get(&key).await {
            if let Ok(spec) = serde_json::from_str::<AppServiceSpec>(&value.value) {
                max_index = max_index.max(spec.app_index);
            }
        }
    }
    Ok(max_index)
}

fn build_install_record(
    view: &InstallTaskView,
    data: &AppInstallTaskData,
    state: InstallRecordState,
    last_error: Option<InstallError>,
) -> Result<InstallRecord, InstallError> {
    let plan = data.state.plan.as_ref().ok_or_else(|| {
        stage_err(
            InstallStage::Prepare,
            InstallErrorCode::Internal,
            false,
            "install record without plan",
        )
    })?;
    let now = buckyos_get_unix_timestamp();
    Ok(InstallRecord {
        schema_version: APP_INSTALL_SCHEMA_VERSION,
        app: plan.app.clone(),
        user_id: match data.request.app_class {
            AppClass::ZoneInstalled => SYSTEM_APP_OWNER_ID.to_string(),
            _ => data.request.user_id.clone(),
        },
        app_instance_id: app_instance_id(
            plan.app.name.as_str(),
            match data.request.app_class {
                AppClass::ZoneInstalled => SYSTEM_APP_OWNER_ID,
                _ => data.request.user_id.as_str(),
            },
        ),
        app_class: data.request.app_class,
        resolution: plan.resolution.clone(),
        package_meta_ids: plan
            .selected_packages
            .iter()
            .filter_map(|package| package.package_meta_id.clone())
            .collect(),
        pikg_digest: data
            .state
            .candidate
            .as_ref()
            .and_then(|candidate| candidate.pikg_digest.clone()),
        target: plan.target.clone(),
        state,
        task_id: view.id.clone(),
        proof_id: None,
        plan_fingerprint: plan.plan_fingerprint.clone(),
        created_at: now,
        updated_at: now,
        last_error,
    })
}

async fn write_install_record(
    client: &SystemConfigClient,
    record: &InstallRecord,
    is_agent: bool,
) -> Result<(), InstallError> {
    let key = install_record_key(
        record.app_class,
        record.user_id.as_str(),
        record.app.name.as_str(),
        is_agent,
    );
    let raw = serde_json::to_string(record).map_err(|err| {
        stage_err(
            InstallStage::Prepare,
            InstallErrorCode::Internal,
            false,
            format!("serialize install record failed: {err}"),
        )
    })?;
    client
        .set(key.as_str(), raw.as_str())
        .await
        .map_err(|err| {
            stage_err(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                true,
                format!("write install record `{key}` failed: {err}"),
            )
        })?;
    Ok(())
}

/// AppService/Agent：等待明确 Started 实例证据。
async fn wait_for_instance_started(
    client: &SystemConfigClient,
    prepared: &PreparedDeployment,
) -> Result<String, InstallError> {
    let instances_key = format!("services/{}/instances", prepared.service_spec_id);
    let mut waited = 0u64;
    loop {
        if let Ok(node_ids) = client.list(&instances_key).await {
            for node_id in node_ids {
                let key = format!("{instances_key}/{node_id}");
                let Ok(value) = client.get(&key).await else {
                    continue;
                };
                let Ok(report) = serde_json::from_str::<ServiceInstanceReportInfo>(&value.value)
                else {
                    continue;
                };
                if matches!(report.state, ServiceInstanceState::Started) {
                    return Ok(node_id);
                }
            }
        }
        if waited >= ACTIVATE_TIMEOUT_SECS {
            return Err(stage_err(
                InstallStage::Activate,
                InstallErrorCode::ActivationFailed,
                true,
                format!(
                    "no Started instance evidence under `{instances_key}` within {ACTIVATE_TIMEOUT_SECS}s"
                ),
            ));
        }
        tokio::time::sleep(Duration::from_secs(ACTIVATE_POLL_INTERVAL_SECS)).await;
        waited += ACTIVATE_POLL_INTERVAL_SECS;
    }
}

/// Static Web：部署完成/可读取证据。当前 scheduler/node-daemon 缺统一
/// instance 语义，这里接受两类可观察证据之一：
/// - `services/{spec_id}/info`（zone gateway 路由面）；
/// - 本机 web 内容目录（单 OOD zone：node-daemon 物化 `{root}/bin/{app}-web`）。
/// 两者都不可得时报 ACTIVATION_FAILED，绝不静默跳过。
async fn wait_for_static_web_evidence(
    client: &SystemConfigClient,
    prepared: &PreparedDeployment,
) -> Result<(), InstallError> {
    let info_key = format!("services/{}/info", prepared.service_spec_id);
    let web_dir = buckyos_kit::get_buckyos_root_dir()
        .join("bin")
        .join(format!("{}-web", prepared.new_spec.app_doc.name));
    let mut waited = 0u64;
    loop {
        if client.get(&info_key).await.is_ok() {
            return Ok(());
        }
        if web_dir.exists() {
            return Ok(());
        }
        if waited >= ACTIVATE_TIMEOUT_SECS {
            return Err(stage_err(
                InstallStage::Activate,
                InstallErrorCode::ActivationFailed,
                true,
                format!(
                    "no static web evidence (`{info_key}` or `{}`) within {ACTIVATE_TIMEOUT_SECS}s",
                    web_dir.display()
                ),
            ));
        }
        tokio::time::sleep(Duration::from_secs(ACTIVATE_POLL_INTERVAL_SECS)).await;
        waited += ACTIVATE_POLL_INTERVAL_SECS;
    }
}

/// installed proof：Activate + 健康检查成功后才生成（实现规则 10）。
/// details 固化 DID 解析证据；本地 override/observed 结果按快照原样呈现，
/// 不得伪装 Anchored。
async fn write_installed_proof(
    view: &InstallTaskView,
    data: &AppInstallTaskData,
    prepared: &PreparedDeployment,
) -> Result<Option<String>, InstallError> {
    let plan = data.state.plan.as_ref().ok_or_else(|| {
        stage_err(
            InstallStage::Activate,
            InstallErrorCode::Internal,
            false,
            "proof without plan",
        )
    })?;
    let runtime = get_buckyos_api_runtime().map_err(|err| {
        stage_err(
            InstallStage::Activate,
            InstallErrorCode::Internal,
            true,
            format!("get runtime failed: {err}"),
        )
    })?;
    let repo = match runtime.get_repo_client().await {
        Ok(repo) => repo,
        Err(err) => {
            // Repo 是传播记录，不是安装成功的前提：本地 pikg 安装在无
            // RepoService 时仍然成功，只是留不下 proof。
            warn!("repo client unavailable, installed proof skipped: {err}");
            return Ok(None);
        }
    };

    let user_id = data.request.user_id.as_str();
    let subject_did = if user_id.starts_with("did:") {
        user_id.to_string()
    } else {
        format!("did:bns:{user_id}")
    };
    let now = buckyos_get_unix_timestamp();
    let proof = ActionObject {
        subject: build_obj_id("actor", subject_did.as_str()),
        action: ACTION_TYPE_INSTALLED.to_string(),
        target: plan.app.object_id.clone(),
        base_on: None,
        details: Some(json!({
            "subject_did": subject_did,
            "app_did": plan.app.did.to_string(),
            "doc_type": plan.resolution.doc_type,
            "app_doc_id": plan.app.object_id.to_string(),
            "app_version": prepared.new_spec.app_doc.version,
            "did_resolution": plan.resolution,
            "package_meta_ids": plan
                .selected_packages
                .iter()
                .filter_map(|package| package.package_meta_id.as_ref().map(|id| id.to_string()))
                .collect::<Vec<_>>(),
            "pikg_digest": data
                .state
                .candidate
                .as_ref()
                .and_then(|candidate| candidate.pikg_digest.clone()),
            "target": plan.target,
            "task_id": view.id,
            "source": "control_panel.app_installer",
        })),
        iat: now,
        exp: now + PROOF_EXPIRE_SECS,
    };
    let proof_obj_id = proof.gen_obj_id().0;
    match repo.add_proof(buckyos_api::RepoProof::action(proof)).await {
        Ok(_) => Ok(Some(proof_obj_id.to_string())),
        Err(err) => {
            // proof 失败不能推翻已成功的安装，但必须留痕。
            warn!("write installed proof failed (install stays successful): {err}");
            Ok(None)
        }
    }
}
