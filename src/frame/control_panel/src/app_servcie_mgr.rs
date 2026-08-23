use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    app_availability_audit_key, app_availability_policy_key, get_buckyos_api_runtime,
    validate_availability_rules, AppAvailabilityGroupRule, AppAvailabilityPolicy,
    AppAvailabilityResolver, AppAvailabilityUserRule, AppInstallationStatusSnapshot, AppInstanceId,
    AppManagementOrigin, AppScheduledInstanceStatus, AvailabilityEffect, AvailabilityMatch,
    DeploymentHealth, InstallRecord, InstallRecordState, ReadinessState, ResolvedAppInstallation,
    ServiceInstanceReportInfo, ServiceInstanceState, StaticWebDeploymentEvidence,
    SystemConfigClient, SystemConfigError, UserType, APP_AVAILABILITY_SCHEMA_VERSION,
    APP_INSTALL_SCHEMA_VERSION, APP_INSTALL_TASK_SCHEMA_ID, APP_UPDATE_TASK_SCHEMA_ID,
};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

fn principal_is_admin(principal: &RpcAuthPrincipal) -> bool {
    matches!(principal.user_type, UserType::Admin | UserType::Root)
}

fn require_self_or_admin(
    principal: &RpcAuthPrincipal,
    target_user_id: &str,
) -> Result<(), RPCErrors> {
    if principal.username == target_user_id || principal_is_admin(principal) {
        Ok(())
    } else {
        Err(RPCErrors::NoPermission(
            "ordinary users cannot list another user's apps".to_string(),
        ))
    }
}

fn policy_guest_allowed(policy: &AppAvailabilityPolicy) -> bool {
    policy
        .group_rules
        .iter()
        .any(|rule| rule.group_id == "guest" && rule.effect == AvailabilityEffect::Allow)
}

fn app_web_hosts(installation: &ResolvedAppInstallation) -> Vec<String> {
    let mut service_names = installation
        .spec
        .spec_config
        .expose_config
        .keys()
        .collect::<Vec<_>>();
    service_names.sort_by_key(|name| (name.as_str() != "www", name.as_str()));

    let mut seen = HashSet::new();
    let mut hosts = Vec::new();
    for service_name in service_names {
        let Some(expose) = installation
            .spec
            .spec_config
            .expose_config
            .get(service_name)
        else {
            continue;
        };
        for host in expose.sub_hostname() {
            let host = host.trim();
            if !host.is_empty() && seen.insert(host.to_string()) {
                hosts.push(host.to_string());
            }
        }
    }
    hosts
}

impl ControlPanelServer {
    fn build_app_summary(
        installation: &ResolvedAppInstallation,
        availability_match: Option<AvailabilityMatch>,
    ) -> Value {
        let spec = &installation.spec;
        let state = serde_json::to_value(&spec.state)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        json!({
            "app_id": spec.app_id(),
            "app_instance_id": spec.app_instance_id,
            "app_did": spec.app_did,
            "runtime_type": spec.app_doc.get_app_type().to_string(),
            "owner_user_id": spec.owner_user_id,
            "availability_match": availability_match,
            "show_name": spec.app_doc.show_name,
            "version": spec.app_doc.version,
            "app_icon_url": spec.app_doc.app_icon_url(),
            "icon_res_url": format!("res/{}/appicon.png", spec.app_id()),
            "author": spec.app_doc.author,
            "app_index": spec.app_index,
            "enable": spec.enable,
            "state": state,
            "expected_instance_count": spec.expected_instance_count,
            "spec_path": installation.spec_path,
            "web_hosts": app_web_hosts(installation),
        })
    }

    pub(crate) async fn app_service_system_config_client(
    ) -> Result<Arc<SystemConfigClient>, RPCErrors> {
        get_buckyos_api_runtime()?.get_system_config_client().await
    }

    pub(crate) async fn app_availability_resolver() -> Result<AppAvailabilityResolver, RPCErrors> {
        Ok(AppAvailabilityResolver::new(
            Self::app_service_system_config_client().await?,
        ))
    }

    pub(crate) async fn resolve_app_selector(
        &self,
        req: &RPCRequest,
        principal: &RpcAuthPrincipal,
    ) -> Result<ResolvedAppInstallation, RPCErrors> {
        let selector = Self::app_selector_from_req(req)?;
        let owner_user_id = Self::param_str(req, "owner_user_id")
            .unwrap_or_else(|| principal.owner_user_id.clone());
        require_self_or_admin(principal, owner_user_id.as_str())?;

        let resolver = Self::app_availability_resolver().await?;
        let mut candidates = resolver
            .list_user_installations(owner_user_id.as_str())
            .await?
            .into_iter()
            .map(|(installation, _)| installation)
            .collect::<Vec<_>>();
        let selector = selector.trim();
        let did_selector = if selector.starts_with("did:") || selector.contains('.') {
            match crate::app_install_resolver::normalize_identifier(selector) {
                Ok(crate::app_install_resolver::NormalizedIdentifier::AppDid(did)) => Some(did),
                Ok(crate::app_install_resolver::NormalizedIdentifier::DomainAlias(alias)) => Some(
                    crate::app_install_resolver::resolve_domain_alias(alias.as_str())
                        .await
                        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
                ),
                _ => None,
            }
        } else {
            None
        };
        candidates.retain(|candidate| {
            let spec = &candidate.spec;
            selector == spec.app_instance_id.to_string()
                || selector == spec.app_id().as_str()
                || did_selector.as_ref() == Some(&spec.app_did)
                || (did_selector.is_none() && selector == spec.app_doc.show_name)
        });
        candidates
            .sort_by(|left, right| left.spec.app_instance_id.cmp(&right.spec.app_instance_id));
        candidates.dedup_by(|left, right| left.spec.app_instance_id == right.spec.app_instance_id);
        match candidates.len() {
            0 => Err(RPCErrors::ReasonError(format!(
                "APP_NOT_INSTALLED: no visible installation matches `{selector}`"
            ))),
            1 => Ok(candidates.remove(0)),
            _ => {
                let choices = candidates
                    .iter()
                    .map(|candidate| {
                        json!({
                            "app_instance_id": candidate.spec.app_instance_id,
                            "owner_user_id": candidate.spec.owner_user_id,
                        })
                    })
                    .collect::<Vec<_>>();
                Err(RPCErrors::ReasonError(
                    json!({
                        "code": "AMBIGUOUS_APP_TARGET",
                        "selector": selector,
                        "candidates": choices,
                    })
                    .to_string(),
                ))
            }
        }
    }

    pub(crate) fn app_selector_from_req(req: &RPCRequest) -> Result<String, RPCErrors> {
        Self::param_str(req, "selector")
            .or_else(|| Self::param_str(req, "app_instance_id"))
            .or_else(|| Self::param_str(req, "app_did"))
            .or_else(|| Self::param_str(req, "identifier"))
            .ok_or_else(|| RPCErrors::ParseRequestError("selector is required".to_string()))
    }

    pub(crate) async fn handle_apps_status(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let installation = self.resolve_app_selector(&req, principal).await?;
        let spec = &installation.spec;
        let can_manage = principal_is_admin(principal) || spec.owner_user_id == principal.username;
        if !can_manage {
            let decision = Self::app_availability_resolver()
                .await?
                .check_user(&principal.username, spec.app_instance_id())
                .await?;
            if !decision.allowed {
                return Err(RPCErrors::NoPermission("AppAccessDenied".to_string()));
            }
        }

        let client = Self::app_service_system_config_client().await?;
        let record_key =
            buckyos_api::install_record_key(spec.owner_user_id.as_str(), spec.app_id());
        let install_record = match client.get(&record_key).await {
            Ok(value) => Some(serde_json::from_str::<InstallRecord>(&value.value).map_err(
                |error| RPCErrors::ReasonError(format!("invalid install record: {error}")),
            )?),
            Err(SystemConfigError::KeyNotFound(_)) => None,
            Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
        };
        let management_origin = if install_record.is_some() {
            AppManagementOrigin::InstallerManaged
        } else {
            AppManagementOrigin::BootstrapManaged
        };

        let now = buckyos_get_unix_timestamp();
        let instances_root = format!("services/{}/instances", spec.app_instance_id());
        let mut runtime_instances = Vec::new();
        for node_id in match client.list(&instances_root).await {
            Ok(values) => values,
            Err(SystemConfigError::KeyNotFound(_)) => Vec::new(),
            Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
        } {
            if let Ok(value) = client.get(&format!("{instances_root}/{node_id}")).await {
                if let Ok(report) = serde_json::from_str::<ServiceInstanceReportInfo>(&value.value)
                {
                    runtime_instances.push(report);
                }
            }
        }
        let mut static_web_evidence = Vec::new();
        let static_root = format!("services/{}/static_evidence", spec.app_instance_id());
        for node_id in match client.list(&static_root).await {
            Ok(values) => values,
            Err(SystemConfigError::KeyNotFound(_)) => Vec::new(),
            Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
        } {
            if let Ok(value) = client.get(&format!("{static_root}/{node_id}")).await {
                if let Ok(evidence) =
                    serde_json::from_str::<StaticWebDeploymentEvidence>(&value.value)
                {
                    static_web_evidence.push(evidence);
                }
            }
        }
        let runtime_ready_count = runtime_instances
            .iter()
            .filter(|report| {
                report.deployment.as_ref() == Some(&spec.deployment)
                    && report.state == ServiceInstanceState::Started
                    && report.health == DeploymentHealth::Healthy
                    && report.observed_at <= now
                    && report.expires_at > now
            })
            .count() as u32;
        let static_ready_count = static_web_evidence
            .iter()
            .filter(|evidence| {
                evidence.deployment == spec.deployment
                    && evidence.deployment_error.is_none()
                    && evidence.materialized_at > 0
                    && !evidence.gateway_config_generation.is_empty()
                    && evidence.gateway_ready_at >= evidence.materialized_at
                    && evidence.observed_at <= now
                    && evidence.expires_at > now
            })
            .count() as u32;
        let ready_instance_count = if spec.app_doc.get_app_type() == buckyos_api::AppType::Web {
            static_ready_count
        } else {
            runtime_ready_count
        };

        let mut scheduled_instances = Vec::new();
        for node_id in match client.list("nodes").await {
            Ok(values) => values,
            Err(SystemConfigError::KeyNotFound(_)) => Vec::new(),
            Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
        } {
            if let Ok(value) = client.get(&format!("nodes/{node_id}/config")).await {
                if let Ok(config) = serde_json::from_str::<buckyos_api::NodeConfig>(&value.value) {
                    for instance in config.apps.values() {
                        if instance.node_execution_spec.app_instance_id == spec.app_instance_id {
                            scheduled_instances.push(AppScheduledInstanceStatus {
                                node_id: node_id.clone(),
                                target_state: instance.target_state.clone(),
                                deployment: instance.deployment.clone(),
                            });
                        }
                    }
                }
            }
        }

        let mut active_tasks = Vec::new();
        let task_client = get_buckyos_api_runtime()?.get_task_mgr_client().await?;
        for schema_id in [APP_INSTALL_TASK_SCHEMA_ID, APP_UPDATE_TASK_SCHEMA_ID] {
            let page = task_client
                .list_tasks(buckyos_api::ListTasksReq {
                    schema_id: Some(schema_id.to_string()),
                    runner_app_id: Some(buckyos_api::CONTROL_PANEL_SERVICE_NAME.to_string()),
                    limit: Some(100),
                    ..Default::default()
                })
                .await?;
            for task in page
                .tasks
                .into_iter()
                .filter(|task| !task.phase.is_terminal())
            {
                if let Ok(status) = self
                    .install_engine
                    .status(
                        task.task_id.as_str(),
                        principal.username.as_str(),
                        principal_is_admin(principal),
                    )
                    .await
                {
                    if status.app_instance_id.as_ref() == Some(&spec.app_instance_id) {
                        active_tasks.push(status);
                    }
                }
            }
        }

        let stopped = matches!(spec.state, buckyos_api::ServiceState::Stopped);
        let readiness = if stopped {
            ReadinessState::from_bool(ready_instance_count == 0)
        } else {
            ReadinessState::from_bool(ready_instance_count >= spec.expected_instance_count.max(1))
        };
        let available_actions = match management_origin {
            AppManagementOrigin::SystemBuiltin => Vec::new(),
            _ if can_manage => {
                let mut actions = vec!["status".to_string(), "uninstall".to_string()];
                if stopped {
                    actions.push("start".to_string());
                } else {
                    actions.push("stop".to_string());
                    actions.push("restart".to_string());
                }
                actions
            }
            _ => vec!["status".to_string()],
        };
        let last_successful_deployment = match install_record.as_ref() {
            Some(record) if record.state == InstallRecordState::Installed => {
                record.target_deployment.clone()
            }
            Some(record) => record.previous_deployment.clone(),
            None => Some(spec.deployment.clone()),
        };
        let rollback_from_deployment = install_record
            .as_ref()
            .filter(|record| record.state == InstallRecordState::RolledBack)
            .and_then(|record| record.target_deployment.clone());
        let snapshot = AppInstallationStatusSnapshot {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            app_instance_id: spec.app_instance_id.clone(),
            app_did: spec.app_did.clone(),
            app_name: spec.app_name.clone(),
            app_version: spec.app_doc.version.clone(),
            management_origin,
            desired_spec: spec.clone(),
            desired_deployment: spec.deployment.clone(),
            last_successful_deployment,
            rollback_from_deployment,
            install_record,
            active_tasks,
            scheduled_instance_count: scheduled_instances.len() as u32,
            scheduled_instances,
            runtime_instances,
            static_web_evidence,
            desired_instance_count: spec.expected_instance_count,
            ready_instance_count,
            readiness,
            available_actions,
            observed_at: now,
        };
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(snapshot).map_err(|error| {
                RPCErrors::ReasonError(format!("serialize app status failed: {error}"))
            })?),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let user_id =
            Self::param_str(&req, "user_id").unwrap_or_else(|| principal.owner_user_id.clone());
        require_self_or_admin(principal, &user_id)?;

        let resolver = Self::app_availability_resolver().await?;
        let mut apps = resolver
            .list_user_installations(&user_id)
            .await?
            .into_iter()
            .filter_map(|(installation, decision)| {
                decision
                    .availability_match
                    .map(|matched| Self::build_app_summary(&installation, Some(matched)))
            })
            .collect::<Vec<_>>();
        apps.sort_by(|left, right| {
            left.get("app_index")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                .cmp(
                    &right
                        .get("app_index")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                )
                .then_with(|| {
                    left.get("app_instance_id")
                        .and_then(Value::as_str)
                        .cmp(&right.get("app_instance_id").and_then(Value::as_str))
                })
        });

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "user_id": user_id,
                "total": apps.len(),
                "apps": apps,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_app_detials(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let installation = self.resolve_app_selector(&req, principal).await?;
        let app_instance_id = installation.spec.app_instance_id();
        let resolver = Self::app_availability_resolver().await?;
        let can_manage =
            principal_is_admin(principal) || installation.spec.owner_user_id == principal.username;
        let availability_match = if can_manage {
            resolver
                .check_user(&principal.username, app_instance_id)
                .await
                .ok()
                .and_then(|decision| decision.availability_match)
        } else {
            let decision = resolver
                .check_user(&principal.username, app_instance_id)
                .await?;
            if !decision.allowed {
                return Err(RPCErrors::NoPermission("AppAccessDenied".to_string()));
            }
            decision.availability_match
        };
        let summary = Self::build_app_summary(&installation, availability_match);
        let spec = serde_json::to_value(&installation.spec).map_err(|error| {
            RPCErrors::ReasonError(format!("failed to serialize app spec: {error}"))
        })?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "app_id": installation.spec.app_id(),
                "app_instance_id": installation.spec.app_instance_id(),
                "owner_user_id": installation.spec.owner_user_id,
                "spec_path": installation.spec_path,
                "summary": summary,
                "spec": spec,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_app_availability_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_instance_id = Self::require_param_str(&req, "app_instance_id")?
            .parse::<AppInstanceId>()
            .map_err(RPCErrors::ParseRequestError)?;
        let resolver = Self::app_availability_resolver().await?;
        let installation = resolver.resolve_installation(&app_instance_id).await?;
        if !principal_is_admin(principal) && installation.spec.owner_user_id != principal.username {
            return Err(RPCErrors::NoPermission(
                "only the app owner or an admin can inspect the policy".to_string(),
            ));
        }
        let policy = resolver
            .load_policy(&app_instance_id)
            .await?
            .map(|(policy, _)| policy)
            .unwrap_or_else(|| AppAvailabilityPolicy::owner_default(app_instance_id));
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(policy).map_err(|error| {
                RPCErrors::ReasonError(format!("failed to serialize policy: {error}"))
            })?),
            req.seq,
        ))
    }

    pub(crate) async fn handle_app_availability_set(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        if !principal.is_user_session || !principal.is_control_panel_session {
            return Err(RPCErrors::NoPermission(
                "availability changes require a user-authenticated Control Panel session"
                    .to_string(),
            ));
        }
        let app_instance_id = Self::require_param_str(&req, "app_instance_id")?
            .parse::<AppInstanceId>()
            .map_err(RPCErrors::ParseRequestError)?;
        let expected_revision = Self::param_u64(&req, "expected_revision")
            .ok_or_else(|| RPCErrors::ParseRequestError("missing expected_revision".to_string()))?;
        let group_rules: Vec<AppAvailabilityGroupRule> = serde_json::from_value(
            req.params
                .get("group_rules")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )
        .map_err(|error| RPCErrors::ParseRequestError(format!("invalid group_rules: {error}")))?;
        let user_rules: Vec<AppAvailabilityUserRule> = serde_json::from_value(
            req.params
                .get("user_rules")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )
        .map_err(|error| RPCErrors::ParseRequestError(format!("invalid user_rules: {error}")))?;
        validate_availability_rules(&group_rules, &user_rules)?;

        let client = Self::app_service_system_config_client().await?;
        let resolver = AppAvailabilityResolver::new(client.clone());
        let installation = resolver.resolve_installation(&app_instance_id).await?;
        if installation.spec.owner_user_id != principal.username {
            return Err(RPCErrors::NoPermission(
                "only the app owner can modify availability".to_string(),
            ));
        }
        for rule in &user_rules {
            resolver.get_user_settings(&rule.user_id).await?;
        }

        let current = resolver.load_policy(&app_instance_id).await?;
        let (current_revision, system_revision) = match current {
            Some((policy, revision)) => (policy.revision, Some(revision)),
            None => (0, None),
        };
        if current_revision != expected_revision {
            return Err(RPCErrors::ReasonError(format!(
                "availability revision conflict: expected {expected_revision}, current {current_revision}"
            )));
        }

        let next_revision = current_revision + 1;
        let policy = AppAvailabilityPolicy {
            schema_version: APP_AVAILABILITY_SCHEMA_VERSION,
            app_instance_id: app_instance_id.clone(),
            default_effect: AvailabilityEffect::Deny,
            group_rules,
            user_rules,
            revision: next_revision,
            updated_by: principal.username.clone(),
            updated_at: buckyos_get_unix_timestamp(),
        };
        let policy_key = app_availability_policy_key(&app_instance_id);
        let policy_value = serde_json::to_string(&policy).map_err(|error| {
            RPCErrors::ReasonError(format!("failed to serialize availability policy: {error}"))
        })?;
        let audit_key = app_availability_audit_key(&app_instance_id, next_revision);
        let audit_value = json!({
            "schema_version": APP_AVAILABILITY_SCHEMA_VERSION,
            "app_instance_id": app_instance_id,
            "updated_by": principal.username,
            "updated_at": policy.updated_at,
            "old_revision": current_revision,
            "new_revision": next_revision,
            "group_rule_count": policy.group_rules.len(),
            "user_rule_count": policy.user_rules.len(),
            "guest_allowed": policy_guest_allowed(&policy),
        })
        .to_string();

        let mut actions = HashMap::new();
        actions.insert(
            policy_key.clone(),
            if system_revision.is_some() {
                KVAction::Update(policy_value)
            } else {
                KVAction::Create(policy_value)
            },
        );
        actions.insert(audit_key, KVAction::Create(audit_value));
        if !installation.spec.spec_config.expose_config.is_empty() {
            let guest_allowed = policy_guest_allowed(&policy);
            let mut paths = HashMap::new();
            for service_name in installation.spec.spec_config.expose_config.keys() {
                paths.insert(
                    format!("/spec_config/expose_config/{service_name}/allow_guest"),
                    Some(Value::Bool(guest_allowed)),
                );
            }
            actions.insert(
                installation.spec_path.clone(),
                KVAction::SetByJsonPath(paths),
            );
        }

        client
            .exec_tx(
                actions,
                system_revision.map(|revision| (policy_key, revision)),
            )
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("availability CAS update failed: {error}"))
            })?;

        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(policy).map_err(|error| {
                RPCErrors::ReasonError(format!("failed to serialize policy: {error}"))
            })?),
            req.seq,
        ))
    }

    pub(crate) async fn handle_app_availability_check(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_instance_id = Self::require_param_str(&req, "app_instance_id")?
            .parse::<AppInstanceId>()
            .map_err(RPCErrors::ParseRequestError)?;
        let user_id =
            Self::param_str(&req, "user_id").unwrap_or_else(|| principal.owner_user_id.clone());
        let resolver = Self::app_availability_resolver().await?;
        let installation = resolver.resolve_installation(&app_instance_id).await?;
        let can_diagnose = principal_is_admin(principal)
            || principal.username == user_id
            || installation.spec.owner_user_id == principal.username;
        if !can_diagnose {
            return Err(RPCErrors::NoPermission(
                "not allowed to diagnose this app availability relation".to_string(),
            ));
        }
        let decision = if user_id == "guest" {
            resolver.check_guest(&app_instance_id).await?
        } else {
            resolver.check_user(&user_id, &app_instance_id).await?
        };
        Ok(RPCResponse::new(
            RPCResult::Success(serde_json::to_value(decision).map_err(|error| {
                RPCErrors::ReasonError(format!("failed to serialize decision: {error}"))
            })?),
            req.seq,
        ))
    }
}
