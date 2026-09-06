use crate::app_install_driver::ProductionInstallDriver;
use crate::app_install_engine::InstallTaskView;
use buckyos_api::{
    get_buckyos_api_runtime, install_record_key, AppDoc, AppInstallTaskData, AppType, InstallError,
    InstallErrorCode, InstallParams, InstallPlan, InstallPlanExecutionKey,
    InstallPlanExecutionRecord, InstallPlanExecutionState, InstallStage, InstallTaskResult,
    MountPointConfig, PreparedDeployment, SchedulerClient, ServiceEndpointConfig,
    ServiceExposeConfig, ServiceExposeRouteTips, ServiceSpecConfig,
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

fn install_plan(data: &AppInstallTaskData) -> Result<&InstallPlan, InstallError> {
    data.state.plan.as_ref().ok_or_else(|| {
        stage_error(
            InstallStage::Prepare,
            InstallErrorCode::Internal,
            false,
            "install plan is missing",
        )
    })
}

async fn scheduler_client(stage: InstallStage) -> Result<SchedulerClient, InstallError> {
    get_buckyos_api_runtime()
        .map_err(|error| stage_error(stage, InstallErrorCode::Internal, true, error.to_string()))?
        .get_scheduler_client()
        .await
        .map_err(|error| {
            stage_error(
                stage,
                InstallErrorCode::DeployFailed,
                true,
                error.to_string(),
            )
        })
}

async fn reconcile_execution(
    scheduler: &SchedulerClient,
    plan: &InstallPlan,
    stage: InstallStage,
    submit: bool,
) -> Result<InstallPlanExecutionRecord, InstallError> {
    let key = InstallPlanExecutionKey::from_plan(plan);
    let result = if submit {
        match scheduler.submit_install_plan(plan.clone()).await {
            Ok(record) => Ok(record),
            Err(_) => scheduler.get_install_plan_status(key.clone()).await,
        }
    } else {
        scheduler.get_install_plan_status(key.clone()).await
    };
    let rpc_error = |error: kRPC::RPCErrors| {
        stage_error(
            stage,
            InstallErrorCode::DeployFailed,
            true,
            format!("waiting to reconcile scheduler execution: {error}"),
        )
    };
    let mut record = result.map_err(rpc_error)?;
    for attempt in 0..2 {
        if record.key != key || record.plan != *plan {
            return Err(stage_error(
                stage,
                InstallErrorCode::Conflict,
                false,
                "scheduler execution does not match the approved plan",
            ));
        }
        match record.state {
            InstallPlanExecutionState::Completed => return Ok(record),
            InstallPlanExecutionState::Canceled => {
                return Err(stage_error(
                    stage,
                    InstallErrorCode::Canceled,
                    false,
                    "scheduler execution was canceled",
                ));
            }
            InstallPlanExecutionState::Failed
                if attempt == 1 || record.error.as_ref().is_some_and(|error| !error.retryable) =>
            {
                let mut error = record.error.unwrap_or_else(|| {
                    stage_error(
                        stage,
                        InstallErrorCode::DeployFailed,
                        true,
                        "scheduler execution failed without a structured error",
                    )
                });
                error.stage = stage;
                return Err(error);
            }
            _ if attempt == 0 => {
                record = scheduler
                    .retry_install_plan(key.clone())
                    .await
                    .map_err(rpc_error)?;
            }
            _ => break,
        }
    }
    if !record.commit_point.is_committed() {
        return Err(stage_error(
            stage,
            InstallErrorCode::DeployFailed,
            true,
            "waiting for scheduler desired-state commit",
        ));
    }
    Ok(record)
}

impl ProductionInstallDriver {
    pub(crate) async fn prepare_impl(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<PreparedDeployment, InstallError> {
        let plan = install_plan(data)?;
        let scheduler = scheduler_client(InstallStage::Prepare).await?;
        let exists = scheduler
            .get_install_plan_status(InstallPlanExecutionKey::from_plan(plan))
            .await
            .is_ok();
        if !exists {
            self.materialize_candidate_pikg(data).await?;
        }
        reconcile_execution(&scheduler, plan, InstallStage::Prepare, !exists).await?;
        Ok(PreparedDeployment {
            app_instance_id: plan.app_instance_id.clone(),
            task_id: plan.task_id.clone(),
            plan_fingerprint: plan.plan_fingerprint.clone(),
            submitted_at: buckyos_get_unix_timestamp(),
        })
    }

    pub(crate) async fn deploy_impl(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let scheduler = scheduler_client(InstallStage::Deploy).await?;
        reconcile_execution(&scheduler, install_plan(data)?, InstallStage::Deploy, false).await?;
        Ok(())
    }

    pub(crate) async fn activate_impl(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<InstallTaskResult, InstallError> {
        let plan = install_plan(data)?;
        let scheduler = scheduler_client(InstallStage::Activate).await?;
        let deadline = Instant::now() + Duration::from_secs(SCHEDULER_WAIT_TIMEOUT_SECS);
        loop {
            let record =
                reconcile_execution(&scheduler, plan, InstallStage::Activate, false).await?;
            if record.state == InstallPlanExecutionState::Completed {
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
            if Instant::now() >= deadline {
                return Err(stage_error(
                    InstallStage::Activate,
                    InstallErrorCode::DeployFailed,
                    true,
                    "waiting for scheduler execution to complete",
                ));
            }
            sleep(Duration::from_secs(2)).await;
        }
    }

    pub(crate) async fn rollback_impl(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let scheduler = scheduler_client(InstallStage::Prepare).await?;
        let plan = install_plan(data)?;
        let record = scheduler
            .cancel_install_plan(plan.clone())
            .await
            .map_err(|error| {
                stage_error(
                    InstallStage::Prepare,
                    InstallErrorCode::Conflict,
                    true,
                    error.to_string(),
                )
            })?;
        if record.key != InstallPlanExecutionKey::from_plan(plan)
            || record.plan != *plan
            || record.state != InstallPlanExecutionState::Canceled
            || record.commit_point.is_committed()
        {
            return Err(stage_error(
                InstallStage::Prepare,
                InstallErrorCode::Conflict,
                false,
                "scheduler did not confirm cancellation before desired-state commit",
            ));
        }
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
    let appid = buckyos_api::AppId::from_app_did(app_doc.app_did())
        .map(|value| value.to_string())
        .unwrap_or_else(|_| app_doc.app_did().to_string());
    validate_app_rdb_instances(&appid, &config.rdb_instances, &mut issues);
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

fn validate_app_rdb_instances(
    appid: &str,
    instances: &HashMap<String, buckyos_api::RdbInstanceConfig>,
    issues: &mut Vec<String>,
) {
    for (instance_id, instance) in instances {
        if let Err(error) =
            buckyos_api::validate_rdb_instance_config(instance, appid, instance_id, true)
        {
            issues.push(error.to_string());
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{RdbBackend, RdbInstanceConfig, RdbPartition};

    #[test]
    fn app_rdb_config_rejects_host_partitions() {
        let instances = HashMap::from([(
            "main".to_string(),
            RdbInstanceConfig {
                backend: RdbBackend::Sqlite,
                version: 1,
                schema: HashMap::new(),
                connection: String::new(),
                partitions: vec![RdbPartition::Local],
            },
        )]);
        let mut issues = Vec::new();
        validate_app_rdb_instances("demo.buckyos.did", &instances, &mut issues);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("partition_not_allowed_for_app"));
        assert!(issues[0].contains("appid=demo.buckyos.did"));
        assert!(issues[0].contains("instance_id=main"));
    }
}

#[cfg(test)]
#[path = "app_install_recovery_tests.rs"]
mod recovery_tests;
