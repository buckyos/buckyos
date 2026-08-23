use crate::app_install_driver::ProductionInstallDriver;
use crate::app_install_engine::InstallTaskView;
use buckyos_api::{
    get_buckyos_api_runtime, install_record_key, AppDoc, AppInstallTaskData, AppType, InstallError,
    InstallErrorCode, InstallParams, InstallPlanCommitPoint, InstallPlanExecutionKey,
    InstallPlanExecutionState, InstallStage, InstallTaskResult, MountPointConfig,
    PreparedDeployment, ServiceEndpointConfig, ServiceExposeConfig, ServiceExposeRouteTips,
    ServiceSpecConfig,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::time::{sleep, Duration, Instant};

const SCHEDULER_WAIT_TIMEOUT_SECS: u64 = 180;

fn stage_error(
    stage: InstallStage,
    code: InstallErrorCode,
    retryable: bool,
    message: impl Into<String>,
) -> InstallError {
    InstallError::new(stage, code, retryable, message)
}

fn execution_key(data: &AppInstallTaskData) -> Result<InstallPlanExecutionKey, InstallError> {
    let plan = data.state.plan.as_ref().ok_or_else(|| {
        stage_error(
            InstallStage::Prepare,
            InstallErrorCode::Internal,
            false,
            "install plan is missing",
        )
    })?;
    Ok(InstallPlanExecutionKey::from_plan(plan))
}

impl ProductionInstallDriver {
    pub(crate) async fn prepare_impl(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<PreparedDeployment, InstallError> {
        let plan = data.state.plan.clone().ok_or_else(|| {
            stage_error(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                false,
                "prepare requires an immutable install plan",
            )
        })?;
        let runtime = get_buckyos_api_runtime().map_err(|error| {
            stage_error(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                true,
                format!("runtime unavailable: {error}"),
            )
        })?;
        let scheduler = runtime.get_scheduler_client().await.map_err(|error| {
            stage_error(
                InstallStage::Prepare,
                InstallErrorCode::DeployFailed,
                true,
                format!("scheduler unavailable: {error}"),
            )
        })?;
        let record = scheduler
            .submit_install_plan(plan.clone())
            .await
            .map_err(|error| {
                stage_error(
                    InstallStage::Prepare,
                    InstallErrorCode::DeployFailed,
                    true,
                    format!("scheduler rejected install plan: {error}"),
                )
            })?;
        if record.key != InstallPlanExecutionKey::from_plan(&plan)
            || matches!(
                record.state,
                InstallPlanExecutionState::Failed | InstallPlanExecutionState::Canceled
            )
        {
            return Err(record.error.unwrap_or_else(|| {
                stage_error(
                    InstallStage::Prepare,
                    InstallErrorCode::Conflict,
                    false,
                    "scheduler returned a mismatched or terminal execution record",
                )
            }));
        }
        Ok(PreparedDeployment {
            app_instance_id: plan.app_instance_id,
            task_id: plan.task_id,
            plan_fingerprint: plan.plan_fingerprint,
            submitted_at: buckyos_get_unix_timestamp(),
        })
    }

    pub(crate) async fn deploy_impl(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let key = execution_key(data)?;
        let runtime = get_buckyos_api_runtime().map_err(|error| {
            stage_error(
                InstallStage::Deploy,
                InstallErrorCode::Internal,
                true,
                error.to_string(),
            )
        })?;
        let scheduler = runtime.get_scheduler_client().await.map_err(|error| {
            stage_error(
                InstallStage::Deploy,
                InstallErrorCode::DeployFailed,
                true,
                error.to_string(),
            )
        })?;
        let record = scheduler
            .get_install_plan_status(key)
            .await
            .map_err(|error| {
                stage_error(
                    InstallStage::Deploy,
                    InstallErrorCode::DeployFailed,
                    true,
                    error.to_string(),
                )
            })?;
        if record.commit_point == InstallPlanCommitPoint::BeforeClaim {
            return Err(stage_error(
                InstallStage::Deploy,
                InstallErrorCode::DeployFailed,
                true,
                "scheduler has not claimed the plan",
            ));
        }
        Ok(())
    }

    pub(crate) async fn activate_impl(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<InstallTaskResult, InstallError> {
        let plan = data.state.plan.as_ref().ok_or_else(|| {
            stage_error(
                InstallStage::Activate,
                InstallErrorCode::Internal,
                false,
                "plan missing",
            )
        })?;
        let key = InstallPlanExecutionKey::from_plan(plan);
        let runtime = get_buckyos_api_runtime().map_err(|error| {
            stage_error(
                InstallStage::Activate,
                InstallErrorCode::Internal,
                true,
                error.to_string(),
            )
        })?;
        let scheduler = runtime.get_scheduler_client().await.map_err(|error| {
            stage_error(
                InstallStage::Activate,
                InstallErrorCode::DeployFailed,
                true,
                error.to_string(),
            )
        })?;
        let deadline = Instant::now() + Duration::from_secs(SCHEDULER_WAIT_TIMEOUT_SECS);
        loop {
            let record = scheduler
                .get_install_plan_status(key.clone())
                .await
                .map_err(|error| {
                    stage_error(
                        InstallStage::Activate,
                        InstallErrorCode::DeployFailed,
                        true,
                        error.to_string(),
                    )
                })?;
            match record.state {
                InstallPlanExecutionState::Completed => {
                    return Ok(InstallTaskResult {
                        install_record_key: Some(install_record_key(
                            &plan.owner_user_id,
                            plan.app_instance_id.app_id(),
                        )),
                        proof_id: None,
                        instance_node_id: None,
                        completed_at: Some(buckyos_get_unix_timestamp()),
                    });
                }
                InstallPlanExecutionState::Failed | InstallPlanExecutionState::Canceled => {
                    return Err(record.error.unwrap_or_else(|| {
                        stage_error(
                            InstallStage::Activate,
                            InstallErrorCode::DeployFailed,
                            false,
                            "scheduler execution ended without a structured error",
                        )
                    }));
                }
                _ if Instant::now() >= deadline => {
                    return Err(stage_error(
                        InstallStage::Activate,
                        InstallErrorCode::DeployFailed,
                        true,
                        "scheduler execution did not complete before timeout",
                    ));
                }
                _ => sleep(Duration::from_secs(2)).await,
            }
        }
    }

    pub(crate) async fn rollback_impl(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let key = execution_key(data)?;
        let runtime = get_buckyos_api_runtime().map_err(|error| {
            stage_error(
                InstallStage::Deploy,
                InstallErrorCode::Internal,
                true,
                error.to_string(),
            )
        })?;
        let scheduler = runtime.get_scheduler_client().await.map_err(|error| {
            stage_error(
                InstallStage::Deploy,
                InstallErrorCode::DeployFailed,
                true,
                error.to_string(),
            )
        })?;
        scheduler.cancel_install_plan(key).await.map_err(|error| {
            stage_error(
                InstallStage::Deploy,
                InstallErrorCode::DeployFailed,
                true,
                error.to_string(),
            )
        })?;
        Ok(())
    }
}

pub(crate) fn build_install_config(
    app_doc: &AppDoc,
    install_params: &InstallParams,
) -> (ServiceSpecConfig, Vec<String>) {
    let tips = &app_doc.service_config_tips;
    let mut issues = Vec::new();
    let mut config = ServiceSpecConfig {
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
        rdb_instances: tips.rdb_instances.clone(),
        instance_volume: tips.instance_volume.clone(),
        runtime_caps: tips.runtime_caps.clone(),
        container_param: tips.container_param.clone(),
        start_param: tips.start_param.clone(),
        ..Default::default()
    };
    for (name, endpoint) in &tips.service_endpoints {
        let setting = install_params.service_settings.services.get(name);
        if matches!(setting, Some(setting) if !setting.enabled) {
            if endpoint.required {
                issues.push(format!("required service endpoint `{name}` is disabled"));
            }
            continue;
        }
        config.service_config.insert(
            name.clone(),
            ServiceEndpointConfig {
                protocol: endpoint.protocol,
                inner_port: endpoint.inner_port,
            },
        );
        if let Some(expose) = setting.and_then(|setting| setting.expose.as_ref()) {
            let mut route = expose.route.clone();
            if let buckyos_api::ServiceExposeRouteConfig::Web { sub_hostname, .. } = &mut route {
                sub_hostname.clear();
            }
            config.expose_config.insert(
                name.clone(),
                ServiceExposeConfig {
                    route,
                    scope: expose.scope.clone(),
                    allow_guest: expose.allow_guest,
                    bind_address: None,
                },
            );
        } else if setting.is_none() {
            if let Some(expose) = &endpoint.expose {
                let expose = match &expose.route {
                    ServiceExposeRouteTips::Web => Some(ServiceExposeConfig::web(
                        Vec::new(),
                        expose.scope.clone(),
                        expose.allow_guest,
                    )),
                    ServiceExposeRouteTips::Port {
                        preferred_port: Some(port),
                    } => Some(ServiceExposeConfig::port(
                        *port,
                        expose.scope.clone(),
                        expose.allow_guest,
                    )),
                    ServiceExposeRouteTips::Port {
                        preferred_port: None,
                    } => None,
                };
                if let Some(expose) = expose {
                    config.expose_config.insert(name.clone(), expose);
                }
            }
        }
    }
    if app_doc.get_app_type() == AppType::Web && !config.expose_config.contains_key("www") {
        config.expose_config.insert(
            "www".to_string(),
            ServiceExposeConfig::web(Vec::new(), String::new(), true),
        );
    }
    apply_selected_mounts(
        &tips.data_mount_points,
        &install_params.data_mount_points,
        &mut config.data_mount_point,
        &mut issues,
    );
    apply_selected_mounts(
        &tips.local_cache_mount_points,
        &install_params.local_cache_mount_points,
        &mut config.local_cache_mount_point,
        &mut issues,
    );
    apply_selected_mounts(
        &tips.external_mount_points,
        &install_params.external_mount_points,
        &mut config.external_mount_point,
        &mut issues,
    );
    config.bash_envs = install_params.bash_envs.clone();
    if let Some(res_pool_id) = &install_params.res_pool_id {
        config.res_pool_id = res_pool_id.clone();
    }
    (config, issues)
}

fn apply_selected_mounts(
    declared: &HashMap<PathBuf, Option<buckyos_api::MountPointInfo>>,
    selected: &HashMap<PathBuf, MountPointConfig>,
    output: &mut HashMap<PathBuf, MountPointConfig>,
    issues: &mut Vec<String>,
) {
    for (path, selected) in selected {
        if declared.contains_key(path) {
            output.insert(path.clone(), selected.clone());
        } else {
            issues.push(format!("unknown mount point `{}`", path.display()));
        }
    }
}

fn mount_config_from_tip(
    path: &std::path::Path,
    info: Option<&buckyos_api::MountPointInfo>,
    default_access: &str,
) -> MountPointConfig {
    MountPointConfig {
        target_path: info
            .map(|info| info.mount_point_name.as_str())
            .filter(|name| !name.is_empty())
            .map(Into::into)
            .unwrap_or_else(|| path.to_path_buf()),
        access: info
            .map(|info| info.access.as_str())
            .filter(|access| !access.is_empty())
            .unwrap_or(default_access)
            .to_string(),
    }
}
