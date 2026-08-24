use ::kRPC::{RPCContext, RPCErrors, Result};
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use ndn_lib::NamedObject;
use package_lib::PackageId;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::scheduler_server::SchedulerServer;
use crate::system_config_agent::{refresh_rbac, schedule_loop};
use crate::system_config_builder::BootstrapAgentProvision;

const EXECUTION_ROOT: &str = "system/scheduler/install_plan_executions";
const GATEWAY_SETTINGS_KEY: &str = "services/gateway/settings";
const BOOTSTRAP_AGENT_ROOT: &str = "system/scheduler/bootstrap_agents";
const MAX_CAS_RETRIES: usize = 8;

fn install_error(
    code: InstallErrorCode,
    retryable: bool,
    message: impl Into<String>,
) -> InstallError {
    InstallError::new(InstallStage::Deploy, code, retryable, message)
}

fn rpc_error(error: impl ToString) -> RPCErrors {
    RPCErrors::ReasonError(error.to_string())
}

fn serialize<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(rpc_error)
}

fn validate_dns_label(value: &str) -> std::result::Result<(), InstallError> {
    if value.is_empty()
        || value.len() > 63
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(install_error(
            InstallErrorCode::InvalidRequest,
            false,
            format!("invalid shortcut hostname `{value}`"),
        ));
    }
    Ok(())
}

fn validate_plan(plan: &InstallPlan) -> std::result::Result<Vec<DeploymentPackage>, InstallError> {
    if plan.schema_version != APP_INSTALL_SCHEMA_VERSION {
        return Err(install_error(
            InstallErrorCode::UnsupportedSchemaVersion,
            false,
            format!(
                "unsupported InstallPlan schema_version {}",
                plan.schema_version
            ),
        ));
    }
    if !plan.fingerprint_is_valid() {
        return Err(install_error(
            InstallErrorCode::PlanStale,
            false,
            "InstallPlan fingerprint does not match its immutable material",
        ));
    }
    if plan.owner_user_id != plan.app_instance_id.owner_user_id()
        || plan.app.did != *plan.app_doc.app_did()
        || plan.resolution.app_did != plan.app.did
        || plan.resolution.app_doc_object_id.as_ref() != Some(&plan.app.object_id)
        || AppId::from_app_did(&plan.app.did).as_ref() != Ok(plan.app_instance_id.app_id())
    {
        return Err(install_error(
            InstallErrorCode::InvalidRequest,
            false,
            "InstallPlan AppDID/AppInstance/owner identity is inconsistent",
        ));
    }
    plan.app_doc.validate().map_err(|error| {
        install_error(
            InstallErrorCode::VerificationFailed,
            false,
            error.to_string(),
        )
    })?;
    let (app_doc_object_id, _) = plan.app_doc.gen_obj_id();
    if app_doc_object_id != plan.app.object_id {
        return Err(install_error(
            InstallErrorCode::VerificationFailed,
            false,
            "InstallPlan AppDoc snapshot ObjectId does not match AppDocumentRef",
        ));
    }

    plan.selected_packages
        .iter()
        .map(|package| {
            let meta_id = package.package_meta_id.clone().ok_or_else(|| {
                install_error(
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!(
                        "deployment package `{}` does not pin a Package Meta ObjectId",
                        package.sub_pkg_name
                    ),
                )
            })?;
            let parsed = PackageId::parse(&package.pkg_id).map_err(|error| {
                install_error(
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!("invalid exact PackageId `{}`: {error}", package.pkg_id),
                )
            })?;
            if parsed.objid.as_deref() != Some(meta_id.to_string().as_str()) {
                return Err(install_error(
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!(
                        "PackageId `{}` does not pin Package Meta ObjectId `{meta_id}`",
                        package.pkg_id
                    ),
                ));
            }
            Ok(DeploymentPackage {
                sub_pkg_name: package.sub_pkg_name.clone(),
                pkg_id: package.pkg_id.clone(),
                package_meta_object_id: meta_id,
                docker_image_name: package.docker_image_name.clone(),
                docker_image_digest: package.docker_image_digest.clone(),
            })
        })
        .collect()
}

fn default_reserved_hostnames(settings: &ZoneGatewaySettings) -> BTreeSet<String> {
    let mut reserved = BTreeSet::from(["_".to_string(), "www".to_string(), "sys".to_string()]);
    reserved.extend(settings.shortcuts.keys().cloned());
    reserved
}

fn apply_default_hostname(config: &mut ServiceSpecConfig, hostname: &str) {
    for expose in config.expose_config.values_mut() {
        if matches!(
            &expose.route,
            ServiceExposeRouteConfig::Web { sub_hostname, .. } if sub_hostname.is_empty()
        ) {
            expose.set_sub_hostname(vec![hostname.to_string()]);
        }
    }
}

fn initial_guest_availability_policy(
    app_instance_id: &AppInstanceId,
    config: &ServiceSpecConfig,
    updated_by: &str,
    updated_at: u64,
) -> Option<AppAvailabilityPolicy> {
    config
        .expose_config
        .values()
        .any(|expose| expose.allow_guest)
        .then(|| AppAvailabilityPolicy {
            schema_version: APP_AVAILABILITY_SCHEMA_VERSION,
            app_instance_id: app_instance_id.clone(),
            default_effect: AvailabilityEffect::Deny,
            group_rules: vec![AppAvailabilityGroupRule {
                group_id: "guest".to_string(),
                effect: AvailabilityEffect::Allow,
            }],
            user_rules: Vec::new(),
            revision: 1,
            updated_by: updated_by.to_string(),
            updated_at,
        })
}

impl SchedulerServer {
    async fn load_gateway_settings(&self) -> Result<(ZoneGatewaySettings, Option<u64>)> {
        match self.system_config_client.get(GATEWAY_SETTINGS_KEY).await {
            Ok(value) => Ok((
                serde_json::from_str(&value.value).map_err(rpc_error)?,
                Some(value.version),
            )),
            Err(SystemConfigError::KeyNotFound(_)) => Ok((ZoneGatewaySettings::default(), None)),
            Err(error) => Err(rpc_error(error)),
        }
    }

    async fn load_execution(
        &self,
        key: &InstallPlanExecutionKey,
    ) -> Result<(InstallPlanExecutionRecord, u64)> {
        let path = key.storage_key();
        let value = self
            .system_config_client
            .get(&path)
            .await
            .map_err(rpc_error)?;
        let record =
            serde_json::from_str::<InstallPlanExecutionRecord>(&value.value).map_err(rpc_error)?;
        if record.schema_version != INSTALL_PLAN_EXECUTION_SCHEMA_VERSION || record.key != *key {
            return Err(rpc_error("corrupt scheduler InstallPlan execution record"));
        }
        Ok((record, value.version))
    }

    async fn update_execution(
        &self,
        path: &str,
        record: &InstallPlanExecutionRecord,
        expected_revision: u64,
    ) -> Result<()> {
        let mut actions = HashMap::new();
        actions.insert(path.to_string(), KVAction::Update(serialize(record)?));
        self.system_config_client
            .exec_tx(actions, Some((path.to_string(), expected_revision)))
            .await
            .map_err(rpc_error)?;
        Ok(())
    }

    async fn fail_execution(
        &self,
        path: &str,
        mut record: InstallPlanExecutionRecord,
        revision: u64,
        error: InstallError,
    ) -> Result<InstallPlanExecutionRecord> {
        record.state = InstallPlanExecutionState::Failed;
        record.error = Some(error);
        record.updated_at = buckyos_get_unix_timestamp();
        self.update_execution(path, &record, revision).await?;
        Ok(record)
    }

    async fn publish_committed_install(
        &self,
        key: &InstallPlanExecutionKey,
    ) -> Result<InstallPlanExecutionRecord> {
        let path = key.storage_key();
        match schedule_loop(false, true).await {
            Ok(_) => {
                let (mut record, record_revision) = self.load_execution(key).await?;
                let install_record_path = install_record_key(
                    &record.plan.owner_user_id,
                    record.plan.app_instance_id.app_id(),
                );
                let install_value = self
                    .system_config_client
                    .get(&install_record_path)
                    .await
                    .map_err(rpc_error)?;
                let mut install_record: InstallRecord =
                    serde_json::from_str(&install_value.value).map_err(rpc_error)?;
                install_record.state = InstallRecordState::Installed;
                install_record.updated_at = buckyos_get_unix_timestamp();
                install_record.last_error = None;
                record.state = InstallPlanExecutionState::Completed;
                record.commit_point = InstallPlanCommitPoint::NodeConfigPublished;
                record.updated_at = install_record.updated_at;
                let mut actions = HashMap::new();
                actions.insert(path.clone(), KVAction::Update(serialize(&record)?));
                actions.insert(
                    install_record_path,
                    KVAction::Update(serialize(&install_record)?),
                );
                self.system_config_client
                    .exec_tx(actions, Some((path, record_revision)))
                    .await
                    .map_err(rpc_error)?;
                Ok(record)
            }
            Err(error) => {
                let (record, revision) = self.load_execution(key).await?;
                self.fail_execution(
                    &path,
                    record,
                    revision,
                    install_error(InstallErrorCode::ActivationFailed, true, error.to_string()),
                )
                .await
            }
        }
    }

    async fn execute_install_plan(
        &self,
        key: &InstallPlanExecutionKey,
        resume_claimed: bool,
    ) -> Result<InstallPlanExecutionRecord> {
        let path = key.storage_key();
        let (mut record, mut record_revision) = self.load_execution(key).await?;
        if record.state == InstallPlanExecutionState::Pending {
            record.state = InstallPlanExecutionState::Claimed;
            record.commit_point = InstallPlanCommitPoint::Claimed;
            record.claimed_at = buckyos_get_unix_timestamp();
            record.updated_at = record.claimed_at;
            self.update_execution(&path, &record, record_revision)
                .await?;
            let loaded = self.load_execution(key).await?;
            record = loaded.0;
            record_revision = loaded.1;
        } else if record.state == InstallPlanExecutionState::Claimed && resume_claimed {
            record.claimed_at = buckyos_get_unix_timestamp();
            record.updated_at = record.claimed_at;
            self.update_execution(&path, &record, record_revision)
                .await?;
            let loaded = self.load_execution(key).await?;
            record = loaded.0;
            record_revision = loaded.1;
        } else {
            return Ok(record);
        }

        let packages = match validate_plan(&record.plan) {
            Ok(packages) => packages,
            Err(error) => {
                return self
                    .fail_execution(&path, record, record_revision, error)
                    .await
            }
        };

        for _ in 0..MAX_CAS_RETRIES {
            let loaded = self.load_execution(key).await?;
            record = loaded.0;
            record_revision = loaded.1;
            if record.state != InstallPlanExecutionState::Claimed
                || record.commit_point != InstallPlanCommitPoint::Claimed
            {
                return Ok(record);
            }
            let registry_value = self
                .system_config_client
                .get(APP_REGISTRY_KEY)
                .await
                .map_err(rpc_error)?;
            let mut registry: AppRegistry =
                serde_json::from_str(&registry_value.value).map_err(rpc_error)?;
            let (gateway_settings, _) = self.load_gateway_settings().await?;
            let reserved = default_reserved_hostnames(&gateway_settings);
            let zone_owner = self
                .system_config_client
                .get_zone_owner_user_id()
                .await
                .map_err(rpc_error)?;
            let (_, allocation) = match registry.allocate(
                &record.plan.app.did,
                &record.plan.owner_user_id,
                &zone_owner,
                &reserved,
                buckyos_get_unix_timestamp(),
            ) {
                Ok(value) => value,
                Err(error) => {
                    return self
                        .fail_execution(
                            &path,
                            record,
                            record_revision,
                            install_error(InstallErrorCode::ConfigBlocked, false, error),
                        )
                        .await
                }
            };
            let app_allocation = registry.apps[record.plan.app_instance_id.app_id()].clone();
            let spec_key = user_app_spec_key(
                &record.plan.owner_user_id,
                record.plan.app_instance_id.app_id(),
            );
            let current_spec = match self.system_config_client.get(&spec_key).await {
                Ok(value) => Some((
                    serde_json::from_str::<AppServiceSpec>(&value.value).map_err(rpc_error)?,
                    value.version,
                )),
                Err(SystemConfigError::KeyNotFound(_)) => None,
                Err(error) => return Err(rpc_error(error)),
            };
            if matches!(record.plan.plan_use, InstallPlanUse::FreshInstall)
                && current_spec.is_some()
            {
                return self
                    .fail_execution(
                        &path,
                        record,
                        record_revision,
                        install_error(
                            InstallErrorCode::PlanNotApplicable,
                            false,
                            "FreshInstall cannot replace an existing AppSpec",
                        ),
                    )
                    .await;
            }
            if matches!(record.plan.plan_use, InstallPlanUse::Upgrade) && current_spec.is_none() {
                return self
                    .fail_execution(
                        &path,
                        record,
                        record_revision,
                        install_error(
                            InstallErrorCode::PlanNotApplicable,
                            false,
                            "Upgrade requires an existing AppSpec",
                        ),
                    )
                    .await;
            }
            if matches!(record.plan.plan_use, InstallPlanUse::Satisfied) {
                record.state = InstallPlanExecutionState::Completed;
                record.commit_point = InstallPlanCommitPoint::DesiredStateCommitted;
                record.updated_at = buckyos_get_unix_timestamp();
                self.update_execution(&path, &record, record_revision)
                    .await?;
                return Ok(record);
            }

            let generation = current_spec
                .as_ref()
                .map(|(spec, _)| spec.deployment.spec_generation + 1)
                .unwrap_or(1);
            let pikg_digest = match &record.plan.source_identity {
                InstallSourceIdentity::Pikg { pikg_digest, .. } => Some(pikg_digest.clone()),
                InstallSourceIdentity::Catalog { .. } => None,
            };
            let deployment = DeploymentIdentity {
                app_instance_id: record.plan.app_instance_id.clone(),
                task_id: record.plan.task_id.clone(),
                app_doc_object_id: record.plan.app.object_id.clone(),
                spec_generation: generation,
                pikg_digest: pikg_digest.clone(),
            };
            let mut spec_config = record.plan.service_spec_config.clone();
            apply_default_hostname(&mut spec_config, &allocation.app_host_name);
            let spec = AppServiceSpec {
                app_instance_id: record.plan.app_instance_id.clone(),
                app_did: record.plan.app.did.clone(),
                deployment: deployment.clone(),
                app_doc: record.plan.app_doc.clone(),
                app_name: app_allocation.app_name,
                app_host_name: allocation.app_host_name,
                app_index: allocation.app_index,
                owner_user_id: record.plan.owner_user_id.clone(),
                permission: record.plan.install_params.permissions.clone(),
                selected_components: record.plan.install_params.selected_components.clone(),
                packages: packages.clone(),
                enable: record.plan.install_params.auto_start,
                expected_instance_count: record.plan.install_params.expected_instance_count,
                state: if record.plan.install_params.auto_start {
                    ServiceState::New
                } else {
                    ServiceState::Stopped
                },
                spec_config: spec_config.clone(),
            };
            let install_record_key = install_record_key(
                &record.plan.owner_user_id,
                record.plan.app_instance_id.app_id(),
            );
            let current_install_record = self
                .system_config_client
                .get(&install_record_key)
                .await
                .ok();
            let now = buckyos_get_unix_timestamp();
            let initial_guest_policy = initial_guest_availability_policy(
                &record.plan.app_instance_id,
                &spec_config,
                &record.plan.owner_user_id,
                now,
            );
            let policy_key = app_availability_policy_key(&record.plan.app_instance_id);
            let create_initial_guest_policy = if initial_guest_policy.is_some() {
                match self.system_config_client.get(&policy_key).await {
                    Ok(_) => false,
                    Err(SystemConfigError::KeyNotFound(_)) => true,
                    Err(error) => return Err(rpc_error(error)),
                }
            } else {
                false
            };
            let install_record = InstallRecord {
                schema_version: APP_INSTALL_SCHEMA_VERSION,
                app: record.plan.app.clone(),
                owner_user_id: record.plan.owner_user_id.clone(),
                app_instance_id: record.plan.app_instance_id.clone(),
                resolution: record.plan.resolution.clone(),
                package_meta_ids: packages
                    .iter()
                    .map(|package| package.package_meta_object_id.clone())
                    .collect(),
                pikg_digest,
                target: record.plan.target.clone(),
                install_params: record.plan.install_params.clone(),
                service_spec_config: spec_config,
                target_deployment: Some(deployment.clone()),
                previous_deployment: current_spec
                    .as_ref()
                    .map(|(spec, _)| spec.deployment.clone()),
                state: InstallRecordState::Deploying,
                task_id: record.plan.task_id.clone(),
                proof_id: None,
                plan_fingerprint: record.plan.plan_fingerprint.clone(),
                created_at: current_install_record
                    .as_ref()
                    .and_then(|value| serde_json::from_str::<InstallRecord>(&value.value).ok())
                    .map(|value| value.created_at)
                    .unwrap_or(now),
                updated_at: now,
                last_error: None,
            };

            record.state = InstallPlanExecutionState::Committed;
            record.commit_point = InstallPlanCommitPoint::DesiredStateCommitted;
            record.registry_revision = Some(registry_value.version + 1);
            record.app_spec_revision = Some(
                current_spec
                    .as_ref()
                    .map(|(_, revision)| revision + 1)
                    .unwrap_or(1),
            );
            record.registry = Some(registry.clone());
            record.updated_at = now;

            let mut actions = HashMap::new();
            actions.insert(
                APP_REGISTRY_KEY.to_string(),
                KVAction::Update(serialize(&registry)?),
            );
            actions.insert(
                spec_key,
                if current_spec.is_some() {
                    KVAction::Update(serialize(&spec)?)
                } else {
                    KVAction::Create(serialize(&spec)?)
                },
            );
            actions.insert(
                install_record_key,
                if current_install_record.is_some() {
                    KVAction::Update(serialize(&install_record)?)
                } else {
                    KVAction::Create(serialize(&install_record)?)
                },
            );
            if create_initial_guest_policy {
                let policy = initial_guest_policy
                    .as_ref()
                    .expect("guest policy creation requires a policy");
                actions.insert(policy_key, KVAction::Create(serialize(policy)?));
                actions.insert(
                    app_availability_audit_key(&record.plan.app_instance_id, policy.revision),
                    KVAction::Create(
                        serde_json::json!({
                            "schema_version": APP_AVAILABILITY_SCHEMA_VERSION,
                            "app_instance_id": record.plan.app_instance_id,
                            "updated_by": policy.updated_by,
                            "updated_at": policy.updated_at,
                            "old_revision": 0,
                            "new_revision": policy.revision,
                            "change": "install_default",
                            "group_rule_count": policy.group_rules.len(),
                            "user_rule_count": policy.user_rules.len(),
                            "guest_allowed": true,
                        })
                        .to_string(),
                    ),
                );
            }
            actions.insert(path.clone(), KVAction::Update(serialize(&record)?));
            match self
                .system_config_client
                .exec_tx(
                    actions,
                    Some((APP_REGISTRY_KEY.to_string(), registry_value.version)),
                )
                .await
            {
                Ok(_) => {
                    let loaded = self.load_execution(key).await?;
                    record = loaded.0;
                    record_revision = loaded.1;
                    break;
                }
                Err(_) => continue,
            }
        }

        if record.commit_point != InstallPlanCommitPoint::DesiredStateCommitted {
            return self
                .fail_execution(
                    &path,
                    record,
                    record_revision,
                    install_error(
                        InstallErrorCode::AppMutationInProgress,
                        true,
                        "AppRegistry CAS conflict retry budget exhausted",
                    ),
                )
                .await;
        }

        self.publish_committed_install(key).await
    }

    pub async fn recover_install_plan_executions(&self) -> Result<()> {
        let entries = match self.system_config_client.list(EXECUTION_ROOT).await {
            Ok(entries) => entries,
            Err(SystemConfigError::KeyNotFound(_)) => return Ok(()),
            Err(error) => return Err(rpc_error(error)),
        };
        for entry in entries {
            let path = format!("{EXECUTION_ROOT}/{entry}");
            let Ok(value) = self.system_config_client.get(&path).await else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<InstallPlanExecutionRecord>(&value.value)
            else {
                continue;
            };
            if matches!(
                record.state,
                InstallPlanExecutionState::Pending | InstallPlanExecutionState::Claimed
            ) {
                let _ = self.execute_install_plan(&record.key, true).await;
            } else if record.commit_point == InstallPlanCommitPoint::DesiredStateCommitted
                && matches!(
                    record.state,
                    InstallPlanExecutionState::Committed | InstallPlanExecutionState::Failed
                )
            {
                let _ = self.publish_committed_install(&record.key).await;
            }
        }
        self.recover_bootstrap_agent_provisions().await?;
        Ok(())
    }

    async fn recover_bootstrap_agent_provisions(&self) -> Result<()> {
        let entries = match self.system_config_client.list(BOOTSTRAP_AGENT_ROOT).await {
            Ok(entries) => entries,
            Err(SystemConfigError::KeyNotFound(_)) => return Ok(()),
            Err(error) => return Err(rpc_error(error)),
        };
        for entry in entries {
            let staging_path = format!("{BOOTSTRAP_AGENT_ROOT}/{entry}");
            let staging_value = self
                .system_config_client
                .get(&staging_path)
                .await
                .map_err(rpc_error)?;
            let provision: BootstrapAgentProvision =
                serde_json::from_str(&staging_value.value).map_err(rpc_error)?;
            if provision.schema_version != AGENT_SPEC_SCHEMA_VERSION {
                return Err(rpc_error("unsupported bootstrap Agent provision schema"));
            }
            provision.agent_spec.validate().map_err(rpc_error)?;
            let target = &provision.agent_spec.binding.target_app_instance_id;
            let target_spec_path = user_app_spec_key(target.owner_user_id(), target.app_id());
            let target_value = self
                .system_config_client
                .get(&target_spec_path)
                .await
                .map_err(rpc_error)?;
            let target_spec: AppServiceSpec =
                serde_json::from_str(&target_value.value).map_err(rpc_error)?;
            if target_spec.app_instance_id != *target
                || !target_spec
                    .spec_config
                    .service_config
                    .contains_key(&provision.agent_spec.binding.service_name)
            {
                return Err(rpc_error(
                    "bootstrap Agent binding target AppInstance/service is not installed",
                ));
            }
            let agent_id = &provision.agent_spec.agent_id;
            let spec_path = agent_spec_key(&provision.owner_user_id, agent_id);
            let key_path = format!("users/{}/agents/{}/key", provision.owner_user_id, agent_id);
            let settings_path = format!(
                "users/{}/agents/{}/settings",
                provision.owner_user_id, agent_id
            );
            let record_path = agent_install_record_key(&provision.owner_user_id, agent_id);
            let install_record = AgentInstallRecord {
                schema_version: AGENT_SPEC_SCHEMA_VERSION,
                agent_id: agent_id.clone(),
                agent_doc_object_id: provision.agent_spec.agent_doc_object_id.clone(),
                target_app_instance_id: target.clone(),
                service_name: provision.agent_spec.binding.service_name.clone(),
                generation: provision.agent_spec.generation,
                state: AgentInstallState::Bound,
                updated_at: buckyos_get_unix_timestamp(),
            };
            let mut actions = HashMap::new();
            actions.insert(
                spec_path,
                KVAction::Create(serialize(&provision.agent_spec)?),
            );
            actions.insert(key_path, KVAction::Create(provision.private_key_pem));
            actions.insert(
                settings_path,
                KVAction::Create(serialize(&provision.settings)?),
            );
            actions.insert(record_path, KVAction::Create(serialize(&install_record)?));
            actions.insert(staging_path.clone(), KVAction::Remove);
            self.system_config_client
                .exec_tx(actions, Some((staging_path, staging_value.version)))
                .await
                .map_err(rpc_error)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_instance_id() -> AppInstanceId {
        AppInstanceId::new(AppId::parse("demo.buckyos.bns.did").unwrap(), "alice").unwrap()
    }

    #[test]
    fn public_expose_seeds_guest_policy() {
        let app_instance_id = app_instance_id();
        let mut config = ServiceSpecConfig::default();
        config.expose_config.insert(
            "www".to_string(),
            ServiceExposeConfig::web(Vec::new(), String::new(), true),
        );

        let policy =
            initial_guest_availability_policy(&app_instance_id, &config, "alice", 42).unwrap();
        assert_eq!(policy.app_instance_id, app_instance_id);
        assert_eq!(policy.default_effect, AvailabilityEffect::Deny);
        assert_eq!(policy.revision, 1);
        assert_eq!(policy.updated_by, "alice");
        assert_eq!(policy.updated_at, 42);
        assert_eq!(policy.group_rules.len(), 1);
        assert_eq!(policy.group_rules[0].group_id, "guest");
        assert_eq!(policy.group_rules[0].effect, AvailabilityEffect::Allow);
    }

    #[test]
    fn private_expose_does_not_seed_guest_policy() {
        let app_instance_id = app_instance_id();
        let mut config = ServiceSpecConfig::default();
        config.expose_config.insert(
            "www".to_string(),
            ServiceExposeConfig::web(Vec::new(), String::new(), false),
        );

        assert!(
            initial_guest_availability_policy(&app_instance_id, &config, "alice", 42).is_none()
        );
        assert!(initial_guest_availability_policy(
            &app_instance_id,
            &ServiceSpecConfig::default(),
            "alice",
            42,
        )
        .is_none());
    }
}

#[async_trait]
impl SchedulerHandler for SchedulerServer {
    async fn handle_run_thunk(
        &self,
        request: SchedulerRunThunkRequest,
        _ctx: RPCContext,
    ) -> Result<SchedulerRunThunkResponse> {
        self.thunk_runner
            .run_thunk(request.task_id, request.thunk, request.function_object)
            .await
            .map_err(rpc_error)
    }

    async fn handle_refresh_rbac(&self, _ctx: RPCContext) -> Result<SchedulerRefreshRbacResponse> {
        refresh_rbac().await.map_err(rpc_error)
    }

    async fn handle_submit_install_plan(
        &self,
        plan: InstallPlan,
        _ctx: RPCContext,
    ) -> Result<InstallPlanExecutionRecord> {
        let key = InstallPlanExecutionKey::from_plan(&plan);
        let path = key.storage_key();
        let now = buckyos_get_unix_timestamp();
        let record = InstallPlanExecutionRecord {
            schema_version: INSTALL_PLAN_EXECUTION_SCHEMA_VERSION,
            key: key.clone(),
            plan,
            state: InstallPlanExecutionState::Pending,
            commit_point: InstallPlanCommitPoint::BeforeClaim,
            registry_revision: None,
            app_spec_revision: None,
            registry: None,
            error: None,
            claimed_at: 0,
            updated_at: now,
        };
        match self
            .system_config_client
            .create(&path, &serialize(&record)?)
            .await
        {
            Ok(_) => self.execute_install_plan(&key, false).await,
            Err(_) => {
                let (existing, _) = self.load_execution(&key).await?;
                if existing.plan != record.plan {
                    return Err(rpc_error("InstallPlan idempotency key collision"));
                }
                Ok(existing)
            }
        }
    }

    async fn handle_get_install_plan_status(
        &self,
        key: InstallPlanExecutionKey,
        _ctx: RPCContext,
    ) -> Result<InstallPlanExecutionRecord> {
        self.load_execution(&key).await.map(|value| value.0)
    }

    async fn handle_cancel_install_plan(
        &self,
        key: InstallPlanExecutionKey,
        _ctx: RPCContext,
    ) -> Result<InstallPlanExecutionRecord> {
        let path = key.storage_key();
        for _ in 0..MAX_CAS_RETRIES {
            let (mut record, revision) = self.load_execution(&key).await?;
            if matches!(
                record.commit_point,
                InstallPlanCommitPoint::DesiredStateCommitted
                    | InstallPlanCommitPoint::NodeConfigPublished
            ) {
                return Err(rpc_error(
                    "InstallPlan cannot be canceled after desired-state commit",
                ));
            }
            if matches!(
                record.state,
                InstallPlanExecutionState::Completed
                    | InstallPlanExecutionState::Failed
                    | InstallPlanExecutionState::Canceled
            ) {
                return Ok(record);
            }
            record.state = InstallPlanExecutionState::Canceled;
            record.updated_at = buckyos_get_unix_timestamp();
            if record.commit_point == InstallPlanCommitPoint::Claimed {
                let registry_value = self
                    .system_config_client
                    .get(APP_REGISTRY_KEY)
                    .await
                    .map_err(rpc_error)?;
                let mut actions = HashMap::new();
                actions.insert(
                    APP_REGISTRY_KEY.to_string(),
                    KVAction::Update(registry_value.value),
                );
                actions.insert(path.clone(), KVAction::Update(serialize(&record)?));
                if self
                    .system_config_client
                    .exec_tx(
                        actions,
                        Some((APP_REGISTRY_KEY.to_string(), registry_value.version)),
                    )
                    .await
                    .is_ok()
                {
                    return Ok(record);
                }
            } else if self
                .update_execution(&path, &record, revision)
                .await
                .is_ok()
            {
                return Ok(record);
            }
        }
        Err(rpc_error(
            "InstallPlan cancellation CAS retry budget exhausted",
        ))
    }

    async fn handle_retry_install_plan(
        &self,
        key: InstallPlanExecutionKey,
        _ctx: RPCContext,
    ) -> Result<InstallPlanExecutionRecord> {
        let path = key.storage_key();
        let (mut record, revision) = self.load_execution(&key).await?;
        if record.state != InstallPlanExecutionState::Failed {
            return Err(rpc_error("only failed InstallPlans can be retried"));
        }
        record.error = None;
        record.updated_at = buckyos_get_unix_timestamp();
        if record.commit_point == InstallPlanCommitPoint::Claimed {
            record.state = InstallPlanExecutionState::Pending;
            record.commit_point = InstallPlanCommitPoint::BeforeClaim;
            self.update_execution(&path, &record, revision).await?;
            self.execute_install_plan(&key, false).await
        } else if record.commit_point == InstallPlanCommitPoint::DesiredStateCommitted {
            record.state = InstallPlanExecutionState::Committed;
            self.update_execution(&path, &record, revision).await?;
            self.publish_committed_install(&key).await
        } else {
            Err(rpc_error("InstallPlan commit point is not retryable"))
        }
    }

    async fn handle_mutate_shortcut(
        &self,
        plan: SchedulerShortcutMutationPlan,
        _ctx: RPCContext,
    ) -> Result<SchedulerShortcutMutationRecord> {
        validate_dns_label(&plan.shortcut_hostname).map_err(rpc_error)?;
        if plan.schema_version != INSTALL_PLAN_EXECUTION_SCHEMA_VERSION
            || !plan.fingerprint_is_valid()
        {
            return Err(rpc_error("invalid shortcut mutation schema or fingerprint"));
        }
        let path = plan.storage_key();
        let now = buckyos_get_unix_timestamp();
        let mut record = SchedulerShortcutMutationRecord {
            schema_version: INSTALL_PLAN_EXECUTION_SCHEMA_VERSION,
            plan: plan.clone(),
            state: SchedulerShortcutMutationState::Claimed,
            settings_revision: None,
            error: None,
            claimed_at: now,
            updated_at: now,
        };
        let created = self
            .system_config_client
            .create(&path, &serialize(&record)?)
            .await
            .is_ok();
        if !created {
            let value = self
                .system_config_client
                .get(&path)
                .await
                .map_err(rpc_error)?;
            let existing = serde_json::from_str::<SchedulerShortcutMutationRecord>(&value.value)
                .map_err(rpc_error)?;
            if existing.plan != plan {
                return Err(rpc_error("shortcut mutation idempotency key collision"));
            }
            if existing.state != SchedulerShortcutMutationState::Claimed {
                return Ok(existing);
            }
            record = existing;
        }

        for _ in 0..MAX_CAS_RETRIES {
            let record_value = self
                .system_config_client
                .get(&path)
                .await
                .map_err(rpc_error)?;
            record = serde_json::from_str(&record_value.value).map_err(rpc_error)?;
            if record.state != SchedulerShortcutMutationState::Claimed {
                return Ok(record);
            }
            let (mut settings, settings_revision) = self.load_gateway_settings().await?;
            let registry_value = self
                .system_config_client
                .get(APP_REGISTRY_KEY)
                .await
                .map_err(rpc_error)?;
            let registry: AppRegistry =
                serde_json::from_str(&registry_value.value).map_err(rpc_error)?;
            let reserved = BTreeSet::from(["_".to_string(), "www".to_string(), "sys".to_string()]);
            let collides = plan.target_app_instance_id.is_some()
                && (reserved.contains(&plan.shortcut_hostname)
                    || registry
                        .apps
                        .values()
                        .any(|allocation| allocation.app_name == plan.shortcut_hostname)
                    || registry
                        .instances
                        .values()
                        .any(|allocation| allocation.app_host_name == plan.shortcut_hostname));
            if collides {
                record.state = SchedulerShortcutMutationState::Failed;
                record.error = Some(install_error(
                    InstallErrorCode::ConfigBlocked,
                    false,
                    "shortcut hostname collides with the scheduler-owned default namespace",
                ));
                record.updated_at = buckyos_get_unix_timestamp();
                let mut actions = HashMap::new();
                actions.insert(path.clone(), KVAction::Update(serialize(&record)?));
                self.system_config_client
                    .exec_tx(actions, Some((path, record_value.version)))
                    .await
                    .map_err(rpc_error)?;
                return Ok(record);
            }
            if let Some(target) = &plan.target_app_instance_id {
                let spec_key = user_app_spec_key(target.owner_user_id(), target.app_id());
                let spec_value = self
                    .system_config_client
                    .get(&spec_key)
                    .await
                    .map_err(rpc_error)?;
                let spec: AppServiceSpec =
                    serde_json::from_str(&spec_value.value).map_err(rpc_error)?;
                if spec.app_instance_id != *target {
                    return Err(rpc_error("shortcut target AppSpec identity mismatch"));
                }
                settings.shortcuts.insert(
                    plan.shortcut_hostname.clone(),
                    ShortcutTarget::App {
                        app_instance_id: target.clone(),
                    },
                );
            } else {
                settings.shortcuts.remove(&plan.shortcut_hostname);
            }
            record.state = SchedulerShortcutMutationState::Committed;
            record.settings_revision = Some(settings_revision.unwrap_or(0) + 1);
            record.updated_at = buckyos_get_unix_timestamp();
            let mut actions = HashMap::new();
            actions.insert(
                APP_REGISTRY_KEY.to_string(),
                KVAction::Update(serialize(&registry)?),
            );
            actions.insert(
                GATEWAY_SETTINGS_KEY.to_string(),
                if settings_revision.is_some() {
                    KVAction::Update(serialize(&settings)?)
                } else {
                    KVAction::Create(serialize(&settings)?)
                },
            );
            actions.insert(path.clone(), KVAction::Update(serialize(&record)?));
            if self
                .system_config_client
                .exec_tx(
                    actions,
                    Some((APP_REGISTRY_KEY.to_string(), registry_value.version)),
                )
                .await
                .is_ok()
            {
                return Ok(record);
            }
        }

        let value = self
            .system_config_client
            .get(&path)
            .await
            .map_err(rpc_error)?;
        record = serde_json::from_str(&value.value).map_err(rpc_error)?;
        record.state = SchedulerShortcutMutationState::Failed;
        record.error = Some(install_error(
            InstallErrorCode::AppMutationInProgress,
            true,
            "shortcut mutation CAS retry budget exhausted",
        ));
        record.updated_at = buckyos_get_unix_timestamp();
        let mut actions = HashMap::new();
        actions.insert(path.clone(), KVAction::Update(serialize(&record)?));
        self.system_config_client
            .exec_tx(actions, Some((path, value.version)))
            .await
            .map_err(rpc_error)?;
        Ok(record)
    }
}
