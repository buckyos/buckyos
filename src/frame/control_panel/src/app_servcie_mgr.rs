use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    app_availability_audit_key, app_availability_policy_key, get_buckyos_api_runtime,
    validate_availability_rules, AppAvailabilityGroupRule, AppAvailabilityPolicy,
    AppAvailabilityResolver, AppAvailabilityUserRule, AppClass, AvailabilityEffect,
    AvailabilityMatch, ResolvedAppInstallation, SystemConfigClient, UserType,
    APP_AVAILABILITY_SCHEMA_VERSION,
};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const SYSTEM_APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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
            "app_id": spec.app_doc.name,
            "app_instance_id": spec.app_instance_id(),
            "app_class": spec.app_class,
            "runtime_type": spec.app_doc.get_app_type().to_string(),
            "owner_user_id": spec.user_id,
            "availability_match": availability_match,
            "show_name": spec.app_doc.show_name,
            "version": spec.app_doc.version,
            "app_icon_url": spec.app_doc.app_icon_url(),
            "icon_res_url": format!("res/{}/appicon.png", spec.app_doc.name),
            "author": spec.app_doc.author,
            "tags": spec.app_doc.tags,
            "categories": spec.app_doc.categories,
            "app_index": spec.app_index,
            "enable": spec.enable,
            "state": state,
            "expected_instance_count": spec.expected_instance_count,
            "spec_path": installation.spec_path,
            "web_hosts": app_web_hosts(installation),
        })
    }

    async fn app_service_system_config_client() -> Result<Arc<SystemConfigClient>, RPCErrors> {
        get_buckyos_api_runtime()?.get_system_config_client().await
    }

    async fn app_availability_resolver() -> Result<AppAvailabilityResolver, RPCErrors> {
        Ok(AppAvailabilityResolver::new(
            Self::app_service_system_config_client().await?,
            SYSTEM_APP_VERSION,
        ))
    }

    pub(crate) async fn handle_apps_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let user_id =
            Self::param_str(&req, "user_id").unwrap_or_else(|| principal.username.clone());
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
        let app_instance_id = Self::require_param_str(&req, "app_instance_id")?;
        let resolver = Self::app_availability_resolver().await?;
        let installation = resolver.resolve_installation(&app_instance_id).await?;
        let can_manage = principal_is_admin(principal)
            || (installation.spec.app_class == AppClass::UserInstalled
                && installation.spec.user_id == principal.username);
        let availability_match = if can_manage {
            resolver
                .check_user(&principal.username, &app_instance_id)
                .await
                .ok()
                .and_then(|decision| decision.availability_match)
        } else {
            let decision = resolver
                .check_user(&principal.username, &app_instance_id)
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
                "app_class": installation.spec.app_class,
                "owner_user_id": installation.spec.user_id,
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
        let app_instance_id = Self::require_param_str(&req, "app_instance_id")?;
        let resolver = Self::app_availability_resolver().await?;
        let installation = resolver.resolve_installation(&app_instance_id).await?;
        if !principal_is_admin(principal) && installation.spec.user_id != principal.username {
            return Err(RPCErrors::NoPermission(
                "only the app owner or an admin can inspect the policy".to_string(),
            ));
        }
        if installation.spec.app_class != AppClass::UserInstalled {
            return Err(RPCErrors::NoPermission(
                "system and zone app availability is implicit".to_string(),
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
        let app_instance_id = Self::require_param_str(&req, "app_instance_id")?;
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
        let resolver = AppAvailabilityResolver::new(client.clone(), SYSTEM_APP_VERSION);
        let installation = resolver.resolve_installation(&app_instance_id).await?;
        if installation.spec.app_class != AppClass::UserInstalled {
            return Err(RPCErrors::NoPermission(
                "system and zone app availability is implicit".to_string(),
            ));
        }
        if installation.spec.user_id != principal.username {
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
        let app_instance_id = Self::require_param_str(&req, "app_instance_id")?;
        let user_id =
            Self::param_str(&req, "user_id").unwrap_or_else(|| principal.username.clone());
        let resolver = Self::app_availability_resolver().await?;
        let installation = resolver.resolve_installation(&app_instance_id).await?;
        let can_diagnose = principal_is_admin(principal)
            || principal.username == user_id
            || (installation.spec.app_class == AppClass::UserInstalled
                && installation.spec.user_id == principal.username);
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

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AppDoc, AppServiceSpec, AppType, ServiceExposeConfig, ServiceSpecConfig, ServiceState,
    };
    use name_lib::DID;
    use std::collections::HashMap;

    #[test]
    fn app_web_hosts_prefers_www_and_deduplicates_routes() {
        let owner = DID::new("bns", "alice");
        let app_doc = AppDoc::builder(AppType::Service, "notes", "1.0.0", "alice", &owner)
            .build()
            .unwrap();
        let mut expose_config = HashMap::new();
        expose_config.insert(
            "api".to_string(),
            ServiceExposeConfig::web(
                vec!["notes-api".to_string(), "notes".to_string()],
                String::new(),
                false,
            ),
        );
        expose_config.insert(
            "www".to_string(),
            ServiceExposeConfig::web(
                vec!["notes".to_string(), "notes-web".to_string()],
                String::new(),
                false,
            ),
        );
        let installation = ResolvedAppInstallation {
            spec: AppServiceSpec {
                app_doc,
                app_index: 1,
                user_id: "alice".to_string(),
                app_class: AppClass::UserInstalled,
                permission: Vec::new(),
                enable: true,
                expected_instance_count: 1,
                state: ServiceState::Running,
                spec_config: ServiceSpecConfig {
                    expose_config,
                    ..ServiceSpecConfig::default()
                },
            },
            spec_path: "users/alice/apps/notes/spec".to_string(),
        };

        assert_eq!(
            app_web_hosts(&installation),
            vec!["notes", "notes-web", "notes-api"]
        );
    }
}
