use crate::user_mgr::{refresh_rbac_by_scheduler, require_admin};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    default_contact_projection_rules, get_buckyos_api_runtime, user_group_config_key,
    validate_contact_projection_rules, validate_user_group_id, ContactProjectionConfig,
    ContactProjectionRule, ControlPanelClient, SystemConfigClient, UserGroup, UserGroupDirectory,
    UserSettings, UserType, CONTACT_PROJECTIONS_CONFIG_KEY, USER_GROUP_NAME_MAX_CHARS,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) async fn load_user_group_directory() -> Result<UserGroupDirectory, RPCErrors> {
    let runtime = get_buckyos_api_runtime()?;
    ControlPanelClient::from_shared(runtime.get_system_config_client().await?)
        .load_user_groups()
        .await
}

async fn service_client() -> Result<Arc<SystemConfigClient>, RPCErrors> {
    Ok(get_buckyos_api_runtime()?
        .get_system_config_client()
        .await?)
}

async fn load_account_settings(
    client: &SystemConfigClient,
) -> Result<BTreeMap<String, UserSettings>, RPCErrors> {
    let user_ids = client
        .list("users")
        .await
        .map_err(|e| RPCErrors::ReasonError(format!("Failed to list users: {}", e)))?;
    let mut accounts = BTreeMap::new();
    for user_id in user_ids {
        let Ok(value) = client.get(&format!("users/{}/settings", user_id)).await else {
            continue;
        };
        match serde_json::from_str::<UserSettings>(&value.value) {
            Ok(settings) => {
                accounts.insert(user_id, settings);
            }
            Err(error) => warn!("skip unreadable account record of {user_id}: {error}"),
        }
    }
    Ok(accounts)
}

async fn save_account_settings(
    client: &SystemConfigClient,
    user_id: &str,
    settings: &UserSettings,
) -> Result<(), RPCErrors> {
    let value = serde_json::to_string(settings)
        .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
    client
        .set(&format!("users/{}/settings", user_id), &value)
        .await
        .map_err(|e| RPCErrors::ReasonError(format!("Failed to save user {}: {}", user_id, e)))?;
    Ok(())
}

fn members_of(accounts: &BTreeMap<String, UserSettings>, group_id: &str) -> Vec<String> {
    accounts
        .iter()
        .filter(|(_, settings)| settings.groups.iter().any(|group| group == group_id))
        .map(|(user_id, _)| user_id.clone())
        .collect()
}

fn projections_value(directory: &UserGroupDirectory) -> Value {
    let config = directory.projections.clone();
    json!({
        "rules": directory.projection_rules(),
        "is_default": config.is_none(),
        "updated_by": config.as_ref().map(|c| c.updated_by.clone()).unwrap_or_default(),
        "updated_at": config.as_ref().map(|c| c.updated_at).unwrap_or_default(),
    })
}

fn group_name_param(req: &RPCRequest, required: bool) -> Result<Option<String>, RPCErrors> {
    let name = ControlPanelServer::param_str(req, "name").map(|name| name.trim().to_string());
    match name {
        Some(name) if name.is_empty() || name.chars().count() > USER_GROUP_NAME_MAX_CHARS => {
            Err(RPCErrors::ParseRequestError(format!(
                "name must be 1-{USER_GROUP_NAME_MAX_CHARS} characters"
            )))
        }
        None if required => Err(RPCErrors::ParseRequestError("Missing name".to_string())),
        name => Ok(name),
    }
}

/// Group changes alter RBAC (`g, <user>, group:<id>`) and the contact
/// projection msg-center computes; both follow at once.
async fn propagate(reason: &str) -> Result<(), RPCErrors> {
    refresh_rbac_by_scheduler(reason).await?;
    if let Err(error) = crate::agent_mgr::reload_msg_center().await {
        warn!("msg-center reload after {reason} failed: {error}");
    }
    Ok(())
}

async fn save_projection_rules(
    client: &SystemConfigClient,
    rules: Vec<ContactProjectionRule>,
    updated_by: &str,
) -> Result<ContactProjectionConfig, RPCErrors> {
    let config = ContactProjectionConfig {
        rules,
        updated_by: updated_by.to_string(),
        updated_at: buckyos_get_unix_timestamp(),
    };
    let value = serde_json::to_string(&config)
        .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
    client
        .set(CONTACT_PROJECTIONS_CONFIG_KEY, &value)
        .await
        .map_err(|e| RPCErrors::ReasonError(format!("Failed to save projection rules: {}", e)))?;
    Ok(config)
}

impl ControlPanelServer {
    pub(crate) async fn handle_user_group_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        require_admin(Self::require_rpc_principal(principal)?)?;
        let client = service_client().await?;
        let directory = load_user_group_directory().await?;
        let accounts = load_account_settings(&client).await?;
        let groups: Vec<Value> = directory
            .groups
            .iter()
            .map(|group| {
                let mut value = serde_json::to_value(group).unwrap_or_else(|_| json!({}));
                value["members"] = json!(members_of(&accounts, &group.group_id));
                value
            })
            .collect();
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "groups": groups,
                "projections": projections_value(&directory),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_group_create(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;
        let group_id = Self::require_param_str(&req, "group_id")?.trim().to_string();
        validate_user_group_id(&group_id)?;
        let name = group_name_param(&req, true)?.unwrap_or_default();
        let description = Self::param_str(&req, "description")
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        let now = buckyos_get_unix_timestamp();
        let group = UserGroup {
            group_id: group_id.clone(),
            name,
            description,
            updated_by: principal.username.clone(),
            created_at: now,
            updated_at: now,
        };
        let value = serde_json::to_string(&group)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        let client = service_client().await?;
        let key = user_group_config_key(&group_id);
        if client.get(&key).await.is_ok() {
            return Err(RPCErrors::ReasonError(format!(
                "User group '{}' already exists",
                group_id
            )));
        }
        client
            .create(&key, &value)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to create group: {}", e)))?;
        info!("User group '{}' created by '{}'", group_id, principal.username);
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "group": group })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_group_update(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;
        let group_id = Self::require_param_str(&req, "group_id")?;
        let directory = load_user_group_directory().await?;
        let mut group = directory
            .groups
            .into_iter()
            .find(|group| group.group_id == group_id)
            .ok_or_else(|| RPCErrors::KeyNotExist(format!("user group '{}'", group_id)))?;
        if let Some(name) = group_name_param(&req, false)? {
            group.name = name;
        }
        if let Some(description) = Self::param_text_patch(&req, "description")? {
            group.description = description;
        }
        group.updated_by = principal.username.clone();
        group.updated_at = buckyos_get_unix_timestamp();
        let value = serde_json::to_string(&group)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        service_client()
            .await?
            .set(&user_group_config_key(&group_id), &value)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to update group: {}", e)))?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "group": group })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_group_delete(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;
        let group_id = Self::require_param_str(&req, "group_id")?;
        let directory = load_user_group_directory().await?;
        if !directory.groups.iter().any(|group| group.group_id == group_id) {
            return Err(RPCErrors::KeyNotExist(format!("user group '{}'", group_id)));
        }
        let client = service_client().await?;

        let mut removed_members = 0;
        for (user_id, mut settings) in load_account_settings(&client).await? {
            let before = settings.groups.len();
            settings.groups.retain(|group| group != &group_id);
            if settings.groups.len() != before {
                save_account_settings(&client, &user_id, &settings).await?;
                removed_members += 1;
            }
        }
        let mut removed_rules = 0;
        if let Some(config) = directory.projections.as_ref() {
            let rules: Vec<ContactProjectionRule> = config
                .rules
                .iter()
                .filter(|rule| {
                    rule.source.group_id() != Some(group_id.as_str())
                        && rule.target.group_id() != Some(group_id.as_str())
                })
                .cloned()
                .collect();
            removed_rules = config.rules.len() - rules.len();
            if removed_rules > 0 {
                save_projection_rules(&client, rules, &principal.username).await?;
            }
        }
        client
            .delete(&user_group_config_key(&group_id))
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to delete group: {}", e)))?;
        propagate("user_group.delete").await?;
        info!("User group '{}' deleted by '{}'", group_id, principal.username);
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "group_id": group_id,
                "removed_members": removed_members,
                "removed_rules": removed_rules,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_group_set_members(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;
        let group_id = Self::require_param_str(&req, "group_id")?;
        let user_list = |key: &str| -> Result<Vec<String>, RPCErrors> {
            req.params
                .get(key)
                .map(|value| {
                    serde_json::from_value::<Vec<String>>(value.clone()).map_err(|e| {
                        RPCErrors::ParseRequestError(format!("{key} must be a list of user ids: {e}"))
                    })
                })
                .transpose()
                .map(Option::unwrap_or_default)
        };
        let add = user_list("add")?;
        let remove = user_list("remove")?;
        let directory = load_user_group_directory().await?;
        if !directory.groups.iter().any(|group| group.group_id == group_id) {
            return Err(RPCErrors::KeyNotExist(format!("user group '{}'", group_id)));
        }
        let client = service_client().await?;
        let mut accounts = load_account_settings(&client).await?;
        for user_id in add.iter().chain(remove.iter()) {
            let settings = accounts
                .get(user_id)
                .ok_or_else(|| RPCErrors::KeyNotExist(format!("user '{}'", user_id)))?;
            if matches!(settings.user_type, UserType::Root) {
                return Err(RPCErrors::ParseRequestError(
                    "the root account cannot join user groups".to_string(),
                ));
            }
        }
        let mut changed = false;
        for (user_id, settings) in accounts.iter_mut() {
            let before = settings.groups.clone();
            if remove.contains(user_id) {
                settings.groups.retain(|group| group != &group_id);
            }
            if add.contains(user_id) && !settings.groups.contains(&group_id) {
                settings.groups.push(group_id.clone());
                settings.groups.sort();
            }
            if settings.groups != before {
                save_account_settings(&client, user_id, settings).await?;
                changed = true;
            }
        }
        if changed {
            propagate("user_group.set_members").await?;
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "group_id": group_id,
                "members": members_of(&accounts, &group_id),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_group_set_projections(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;
        let rules: Vec<ContactProjectionRule> = serde_json::from_value(
            req.params
                .get("rules")
                .cloned()
                .ok_or_else(|| RPCErrors::ParseRequestError("Missing rules".to_string()))?,
        )
        .map_err(|e| RPCErrors::ParseRequestError(format!("Invalid rules: {}", e)))?;
        let mut directory = load_user_group_directory().await?;
        validate_contact_projection_rules(&rules, &directory.defined_group_ids())?;
        let client = service_client().await?;
        directory.projections =
            Some(save_projection_rules(&client, rules, &principal.username).await?);
        if let Err(error) = crate::agent_mgr::reload_msg_center().await {
            warn!("msg-center reload after projection change failed: {error}");
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "projections": projections_value(&directory),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_group_reset_projections(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        require_admin(Self::require_rpc_principal(principal)?)?;
        let client = service_client().await?;
        if client.get(CONTACT_PROJECTIONS_CONFIG_KEY).await.is_ok() {
            client
                .delete(CONTACT_PROJECTIONS_CONFIG_KEY)
                .await
                .map_err(|e| RPCErrors::ReasonError(format!("Failed to reset rules: {}", e)))?;
        }
        if let Err(error) = crate::agent_mgr::reload_msg_center().await {
            warn!("msg-center reload after projection reset failed: {error}");
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "projections": {
                    "rules": default_contact_projection_rules(),
                    "is_default": true,
                    "updated_by": "",
                    "updated_at": 0,
                },
            })),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn group_management_is_admin_only() {
        let server = ControlPanelServer::new();
        let principal = RpcAuthPrincipal {
            username: "bob".to_string(),
            owner_user_id: "bob".to_string(),
            authenticated_app_id: "control-panel".to_string(),
            user_type: UserType::User,
            owner_did: "did:bns:bob".to_string(),
            is_user_session: true,
            is_control_panel_session: true,
            agent_permission_mode: None,
            session: crate::PrincipalSession::default(),
        };
        let params = json!({ "group_id": "customer", "name": "Customer", "rules": [] });
        let req = |method: &str| RPCRequest::new(method, params.clone());
        let denied = |result: Result<RPCResponse, RPCErrors>| {
            matches!(result, Err(RPCErrors::ReasonError(reason)) if reason == "Admin privileges required")
        };
        let p = Some(&principal);
        assert!(denied(server.handle_user_group_list(req("user_group.list"), p).await));
        assert!(denied(server.handle_user_group_create(req("user_group.create"), p).await));
        assert!(denied(server.handle_user_group_update(req("user_group.update"), p).await));
        assert!(denied(server.handle_user_group_delete(req("user_group.delete"), p).await));
        assert!(denied(server.handle_user_group_set_members(req("user_group.set_members"), p).await));
        assert!(denied(
            server
                .handle_user_group_set_projections(req("user_group.set_projections"), p)
                .await
        ));
        assert!(denied(
            server
                .handle_user_group_reset_projections(req("user_group.reset_projections"), p)
                .await
        ));
        assert!(denied(
            server
                .handle_user_set_account_options(req("user.set_account_options"), p)
                .await
        ));
    }
}
