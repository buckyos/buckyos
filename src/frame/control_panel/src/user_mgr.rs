use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    get_buckyos_api_runtime, SystemConfigClient, UserSettings, UserState, UserTunnelBinding,
    UserType,
};
use buckyos_kit::KVAction;
use log::*;
use name_lib::DID;
use serde_json::{json, Value};
use std::collections::HashMap;

// ─── helpers ────────────────────────────────────────────────────────────────

fn is_admin_or_root(user_type: &UserType) -> bool {
    matches!(user_type, UserType::Admin | UserType::Root)
}

/// Resolve the target user_id from the request; defaults to the caller.
fn resolve_target_user_id(req: &RPCRequest, principal: &RpcAuthPrincipal) -> String {
    ControlPanelServer::param_str(req, "user_id")
        .unwrap_or_else(|| principal.username.clone())
}

/// Build a fresh `SystemConfigClient` authenticated with the *caller's* RPC
/// session token (instead of the control_panel service's own token).
///
/// This is required for any read/write under `users/...` and `agents/...`:
/// per `rootfs/etc/scheduler/boot.template.toml`, the `ood` device only has
/// `read|write` for `users/*/apps/*` and `users/*/agents/*` — it cannot
/// touch `users/{uid}/doc`, `users/{uid}/settings`, or `agents/{id}/doc`.
/// Those keys are gated by `p, admin,/config/users/*,read|write,allow` and
/// `p, admin,/config/agents/*/...,read|write,allow`, so the request must be
/// signed by the admin caller, not by the service.
async fn system_config_client_for_caller(
    req: &RPCRequest,
) -> Result<SystemConfigClient, RPCErrors> {
    let runtime = get_buckyos_api_runtime()?;
    let url = runtime.get_system_config_url();
    let token = req.token.as_deref().ok_or_else(|| {
        RPCErrors::InvalidToken("missing caller session token".to_string())
    })?;
    Ok(SystemConfigClient::new(Some(url.as_str()), Some(token)))
}

/// Ensure the caller is admin/root **or** is operating on their own account.
fn require_self_or_admin(
    principal: &RpcAuthPrincipal,
    target_user_id: &str,
) -> Result<(), RPCErrors> {
    if is_admin_or_root(&principal.user_type) || principal.username == target_user_id {
        Ok(())
    } else {
        Err(RPCErrors::ReasonError(
            "Only admin or the user themselves can perform this operation".to_string(),
        ))
    }
}

fn require_admin(principal: &RpcAuthPrincipal) -> Result<(), RPCErrors> {
    if is_admin_or_root(&principal.user_type) {
        Ok(())
    } else {
        Err(RPCErrors::ReasonError(
            "Admin privileges required".to_string(),
        ))
    }
}

fn validate_username(name: &str) -> Result<(), RPCErrors> {
    if name.is_empty() || name.len() > 64 {
        return Err(RPCErrors::ParseRequestError(
            "user_id must be 1-64 characters".to_string(),
        ));
    }
    // only allow alphanumeric, underscore, hyphen, dot
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(RPCErrors::ParseRequestError(
            "user_id contains invalid characters (allowed: a-z, 0-9, _, -, .)".to_string(),
        ));
    }
    // reserved names
    if matches!(name, "root" | "system" | "admin" | "guest") {
        return Err(RPCErrors::ParseRequestError(format!(
            "'{}' is a reserved username",
            name
        )));
    }
    Ok(())
}

fn parse_user_type(s: &str) -> Result<UserType, RPCErrors> {
    match s.to_lowercase().as_str() {
        "admin" => Ok(UserType::Admin),
        "user" => Ok(UserType::User),
        "limited" => Ok(UserType::Limited),
        "guest" => Ok(UserType::Guest),
        _ => Err(RPCErrors::ParseRequestError(format!(
            "Invalid user_type: {}",
            s
        ))),
    }
}

fn parse_user_state(s: &str) -> Result<UserState, RPCErrors> {
    UserState::try_from(s.to_string()).map_err(|_| {
        RPCErrors::ParseRequestError(format!("Invalid user state: {}", s))
    })
}

// ─── User management handlers ──────────────────────────────────────────────

impl ControlPanelServer {
    // ── user.list ───────────────────────────────────────────────────────

    pub(crate) async fn handle_user_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let _principal = Self::require_rpc_principal(principal)?;
        // Directory enumeration (`list("users")`) checks the bare path
        // `/config/users`, which the admin rule `/config/users/*` does not
        // match. Use the service token here (control-panel is in the `kernel`
        // group and has full read access); individual per-user reads below
        // are unaffected.
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;

        let user_ids = client.list("users").await.map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to list users: {}", e))
        })?;

        let mut users: Vec<Value> = Vec::new();
        for uid in &user_ids {
            let settings_path = format!("users/{}/settings", uid);
            match client.get(&settings_path).await {
                Ok(val) => {
                    if let Ok(settings) = serde_json::from_str::<UserSettings>(&val.value) {
                        let info = settings.to_user_info();
                        if let Ok(v) = serde_json::to_value(&info) {
                            users.push(v);
                        }
                    }
                }
                Err(_) => {
                    // user entry without settings – skip
                    continue;
                }
            }
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "total": users.len(),
                "users": users,
            })),
            req.seq,
        ))
    }

    // ── user.get ────────────────────────────────────────────────────────

    pub(crate) async fn handle_user_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);

        let client = system_config_client_for_caller(&req).await?;

        let settings_path = format!("users/{}/settings", target);
        let settings_val = client.get(&settings_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e))
        })?;
        let settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        // Build response – hide password, include contact only for self or admin
        let include_contact =
            is_admin_or_root(&principal.user_type) || principal.username == target;
        let mut result = json!({
            "user_id": settings.user_id,
            "show_name": settings.show_name,
            "user_type": settings.user_type,
            "state": settings.state,
            "res_pool_id": settings.res_pool_id,
        });
        if include_contact {
            if let Some(contact) = &settings.contact {
                result["contact"] = serde_json::to_value(contact).unwrap_or(json!(null));
            }
        }

        // Try to load the DID document (best-effort)
        let doc_path = format!("users/{}/doc", target);
        if let Ok(doc_val) = client.get(&doc_path).await {
            if let Ok(doc) = serde_json::from_str::<Value>(&doc_val.value) {
                result["did_document"] = doc;
            }
        }

        Ok(RPCResponse::new(RPCResult::Success(result), req.seq))
    }

    // ── user.create ─────────────────────────────────────────────────────

    pub(crate) async fn handle_user_create(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;

        let user_id = Self::require_param_str(&req, "user_id")?;
        let user_id = user_id.trim().to_lowercase();
        validate_username(&user_id)?;

        let password_hash = Self::require_param_str(&req, "password_hash")?;
        if password_hash.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "password_hash cannot be empty".to_string(),
            ));
        }

        let show_name = Self::param_str(&req, "show_name").unwrap_or_else(|| user_id.clone());
        let user_type = Self::param_str(&req, "user_type")
            .map(|s| parse_user_type(&s))
            .transpose()?
            .unwrap_or(UserType::User);

        // Don't allow creating Root users
        if matches!(user_type, UserType::Root) {
            return Err(RPCErrors::ReasonError(
                "Cannot create root users".to_string(),
            ));
        }

        let client = system_config_client_for_caller(&req).await?;

        // Check if user already exists
        let settings_path = format!("users/{}/settings", user_id);
        if client.get(&settings_path).await.is_ok() {
            return Err(RPCErrors::ReasonError(format!(
                "User '{}' already exists",
                user_id
            )));
        }

        // Build UserSettings
        let new_settings = UserSettings {
            user_id: user_id.clone(),
            user_type: user_type.clone(),
            show_name: show_name.clone(),
            password: password_hash,
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            contact: None,
        };
        let settings_json = serde_json::to_string(&new_settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;

        // Build a minimal OwnerConfig-style doc as JSON
        // (We store the DID document so other components can resolve the user.)
        let user_did = DID::new("bns", &user_id);
        let user_doc = json!({
            "id": user_did.to_string(),
            "name": user_id,
            "full_name": show_name,
        });
        let doc_json = serde_json::to_string(&user_doc)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;

        // Execute as transaction
        let doc_path = format!("users/{}/doc", user_id);
        let mut tx = HashMap::new();
        tx.insert(settings_path, KVAction::Create(settings_json));
        tx.insert(doc_path, KVAction::Create(doc_json));

        client.exec_tx(tx, None).await.map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to create user: {}", e))
        })?;

        // Add to RBAC group if admin.
        // NOTE: `system/rbac/policy` is writable only by `ood` (per boot.template.toml);
        // admin has read-only access. So we must use the service's own session token
        // (not the caller's) for this specific append.
        if matches!(user_type, UserType::Admin) {
            let runtime = get_buckyos_api_runtime()?;
            let service_client = runtime.get_system_config_client().await?;
            let policy_line = format!("g, {}, admin", user_id);
            if let Err(e) = service_client
                .append("system/rbac/policy", &policy_line)
                .await
            {
                warn!("Failed to add user {} to admin RBAC group: {}", user_id, e);
            }
        }

        info!("User '{}' created by '{}'", user_id, principal.username);

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": user_id,
                "user_type": new_settings.user_type,
                "state": "active",
            })),
            req.seq,
        ))
    }

    // ── user.update ─────────────────────────────────────────────────────

    pub(crate) async fn handle_user_update(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);
        require_self_or_admin(principal, &target)?;

        let client = system_config_client_for_caller(&req).await?;

        let settings_path = format!("users/{}/settings", target);
        let settings_val = client.get(&settings_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e))
        })?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        // Apply updates
        if let Some(show_name) = Self::param_str(&req, "show_name") {
            settings.show_name = show_name;
        }

        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to update user: {}", e)))?;

        info!("User '{}' updated by '{}'", target, principal.username);

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
            })),
            req.seq,
        ))
    }

    // ── user.update_contact ─────────────────────────────────────────────
    // Updates the user's contact/binding settings (DID, note, groups, tags, bindings).
    // NOTE: Full contact/friend management lives in MessageCenter.
    //       This endpoint manages the *system-level* contact settings stored
    //       alongside the user account (UserSettings.contact).

    pub(crate) async fn handle_user_update_contact(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);
        require_self_or_admin(principal, &target)?;

        let client = system_config_client_for_caller(&req).await?;

        let settings_path = format!("users/{}/settings", target);
        let settings_val = client.get(&settings_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e))
        })?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        let mut contact = settings.contact.clone().unwrap_or_default();

        // Apply partial updates
        if let Some(did) = Self::param_str(&req, "did") {
            contact.did = Some(did);
        }
        if let Some(note) = Self::param_str(&req, "note") {
            contact.note = Some(note);
        }
        if let Some(groups) = req.params.get("groups") {
            if let Ok(g) = serde_json::from_value::<Vec<String>>(groups.clone()) {
                contact.groups = g;
            }
        }
        if let Some(tags) = req.params.get("tags") {
            if let Ok(t) = serde_json::from_value::<Vec<String>>(tags.clone()) {
                contact.tags = t;
            }
        }
        if let Some(bindings) = req.params.get("bindings") {
            if let Ok(b) = serde_json::from_value::<Vec<UserTunnelBinding>>(bindings.clone()) {
                contact.bindings = b;
            }
        }

        settings.contact = Some(contact.clone());

        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| {
                RPCErrors::ReasonError(format!("Failed to update contact settings: {}", e))
            })?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "contact": serde_json::to_value(&contact).unwrap_or(json!(null)),
            })),
            req.seq,
        ))
    }

    // ── user.delete ─────────────────────────────────────────────────────

    pub(crate) async fn handle_user_delete(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;

        let target = Self::require_param_str(&req, "user_id")?;
        let target = target.trim().to_lowercase();

        if target == "root" {
            return Err(RPCErrors::ReasonError(
                "Cannot delete root user".to_string(),
            ));
        }
        if target == principal.username {
            return Err(RPCErrors::ReasonError(
                "Cannot delete yourself".to_string(),
            ));
        }

        let client = system_config_client_for_caller(&req).await?;

        // Mark user as deleted rather than physically removing
        let settings_path = format!("users/{}/settings", target);
        let settings_val = client.get(&settings_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e))
        })?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        settings.state = UserState::Deleted;
        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to delete user: {}", e)))?;

        info!(
            "User '{}' marked as deleted by '{}'",
            target, principal.username
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
            })),
            req.seq,
        ))
    }

    // ── user.change_password ────────────────────────────────────────────

    pub(crate) async fn handle_user_change_password(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);
        require_self_or_admin(principal, &target)?;

        let new_password_hash = Self::require_param_str(&req, "new_password_hash")?;
        if new_password_hash.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "new_password_hash cannot be empty".to_string(),
            ));
        }

        let client = system_config_client_for_caller(&req).await?;

        let settings_path = format!("users/{}/settings", target);
        let settings_val = client.get(&settings_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e))
        })?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        settings.password = new_password_hash;
        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| {
                RPCErrors::ReasonError(format!("Failed to change password: {}", e))
            })?;

        info!(
            "Password changed for user '{}' by '{}'",
            target, principal.username
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "user_id": target })),
            req.seq,
        ))
    }

    // ── user.change_state ───────────────────────────────────────────────

    pub(crate) async fn handle_user_change_state(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;

        let target = Self::require_param_str(&req, "user_id")?;
        let state_str = Self::require_param_str(&req, "state")?;
        let new_state = parse_user_state(&state_str)?;

        if target == "root" && !matches!(new_state, UserState::Active) {
            return Err(RPCErrors::ReasonError(
                "Cannot change root user state to non-active".to_string(),
            ));
        }

        let client = system_config_client_for_caller(&req).await?;

        let settings_path = format!("users/{}/settings", target);
        let settings_val = client.get(&settings_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e))
        })?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        settings.state = new_state;
        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to change state: {}", e)))?;

        info!(
            "User '{}' state changed to '{}' by '{}'",
            target, state_str, principal.username
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "state": state_str,
            })),
            req.seq,
        ))
    }

    // ── user.change_type ────────────────────────────────────────────────

    pub(crate) async fn handle_user_change_type(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;

        let target = Self::require_param_str(&req, "user_id")?;
        let type_str = Self::require_param_str(&req, "user_type")?;
        let new_type = parse_user_type(&type_str)?;

        if matches!(new_type, UserType::Root) {
            return Err(RPCErrors::ReasonError(
                "Cannot promote to root".to_string(),
            ));
        }

        let client = system_config_client_for_caller(&req).await?;

        let settings_path = format!("users/{}/settings", target);
        let settings_val = client.get(&settings_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e))
        })?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        if matches!(settings.user_type, UserType::Root) {
            return Err(RPCErrors::ReasonError(
                "Cannot change root user type".to_string(),
            ));
        }

        let old_is_admin = matches!(settings.user_type, UserType::Admin);
        let new_is_admin = matches!(new_type, UserType::Admin);

        settings.user_type = new_type;
        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to change type: {}", e)))?;

        // Update RBAC policy if admin status changed.
        // NOTE: `system/rbac/policy` is writable only by `ood` (per boot.template.toml);
        // admin has read-only access. Use the service's own session token for the append.
        if !old_is_admin && new_is_admin {
            let runtime = get_buckyos_api_runtime()?;
            let service_client = runtime.get_system_config_client().await?;
            let policy_line = format!("g, {}, admin", target);
            if let Err(e) = service_client
                .append("system/rbac/policy", &policy_line)
                .await
            {
                warn!("Failed to add {} to admin RBAC group: {}", target, e);
            }
        }
        // Note: revoking admin from RBAC requires policy rewrite which is
        // handled by the scheduler on next reconciliation.

        info!(
            "User '{}' type changed to '{}' by '{}'",
            target, type_str, principal.username
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "user_type": type_str,
            })),
            req.seq,
        ))
    }

    // ─── Agent management handlers ──────────────────────────────────────

    // ── agent.list ──────────────────────────────────────────────────────

    pub(crate) async fn handle_agent_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let _principal = Self::require_rpc_principal(principal)?;
        // See handle_user_list for why we use the service token for the
        // directory enumeration here; individual `get` calls below can
        // run with the caller's token but we already have a broad-read
        // client, so we keep using it for the whole handler.
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;

        let agent_ids = client.list("agents").await.map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to list agents: {}", e))
        })?;

        let mut agents: Vec<Value> = Vec::new();
        for agent_id in &agent_ids {
            let doc_path = format!("agents/{}/doc", agent_id);
            match client.get(&doc_path).await {
                Ok(val) => {
                    let mut agent_info = if let Ok(doc) =
                        serde_json::from_str::<Value>(&val.value)
                    {
                        doc
                    } else {
                        json!({ "agent_id": agent_id })
                    };
                    // Ensure agent_id is always present
                    if agent_info.get("agent_id").is_none() {
                        agent_info["agent_id"] = json!(agent_id);
                    }
                    agents.push(agent_info);
                }
                Err(_) => {
                    agents.push(json!({ "agent_id": agent_id }));
                }
            }
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "total": agents.len(),
                "agents": agents,
            })),
            req.seq,
        ))
    }

    // ── agent.get ───────────────────────────────────────────────────────

    pub(crate) async fn handle_agent_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let _principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::require_param_str(&req, "agent_id")?;

        let client = system_config_client_for_caller(&req).await?;

        let doc_path = format!("agents/{}/doc", agent_id);
        let doc_val = client.get(&doc_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("Agent '{}' not found: {}", agent_id, e))
        })?;
        let mut agent_doc: Value = serde_json::from_str(&doc_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted agent doc: {}", e)))?;

        agent_doc["agent_id"] = json!(agent_id);

        // Load agent settings if available (best-effort)
        let settings_path = format!("agents/{}/settings", agent_id);
        if let Ok(settings_val) = client.get(&settings_path).await {
            if let Ok(settings) = serde_json::from_str::<Value>(&settings_val.value) {
                agent_doc["settings"] = settings;
            }
        }

        Ok(RPCResponse::new(
            RPCResult::Success(agent_doc),
            req.seq,
        ))
    }

    // ── agent.set_msg_tunnel ────────────────────────────────────────────
    // Adds or updates a message tunnel binding for an agent.
    // This delegates to the system config store (not MessageCenter),
    // because agent tunnel bindings are part of the agent's system-level config.

    pub(crate) async fn handle_agent_set_msg_tunnel(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;

        let agent_id = Self::require_param_str(&req, "agent_id")?;
        let platform = Self::require_param_str(&req, "platform")?;
        let account_id = Self::require_param_str(&req, "account_id")?;

        let display_id = Self::param_str(&req, "display_id");
        let tunnel_id = Self::param_str(&req, "tunnel_id");
        let meta: HashMap<String, String> = req
            .params
            .get("meta")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let binding = UserTunnelBinding {
            platform: platform.clone(),
            account_id,
            display_id,
            tunnel_id,
            meta,
        };

        let client = system_config_client_for_caller(&req).await?;

        // Store bindings inside agents/{agent_id}/settings under the "bindings"
        // field. RBAC in boot.template.toml grants admin read|write on
        // `agents/*/settings` but NOT on a separate `bindings` key, so we
        // colocate the data here.
        let settings_path = format!("agents/{}/settings", agent_id);
        let mut settings_obj: Value = match client.get(&settings_path).await {
            Ok(val) => serde_json::from_str(&val.value).unwrap_or_else(|_| json!({})),
            Err(_) => json!({}),
        };
        let mut bindings: Vec<UserTunnelBinding> = settings_obj
            .get("bindings")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Replace existing binding for the same platform or add new
        if let Some(pos) = bindings.iter().position(|b| b.platform == platform) {
            bindings[pos] = binding;
        } else {
            bindings.push(binding);
        }

        settings_obj["bindings"] = serde_json::to_value(&bindings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        let settings_json = serde_json::to_string(&settings_obj)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;

        // Try set, fall back to create if the settings key doesn't exist yet
        if client.set(&settings_path, &settings_json).await.is_err() {
            client
                .create(&settings_path, &settings_json)
                .await
                .map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to save agent bindings: {}", e))
                })?;
        }

        info!(
            "Agent '{}' tunnel binding for '{}' set by '{}'",
            agent_id, platform, principal.username
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "agent_id": agent_id,
                "platform": platform,
                "total_bindings": bindings.len(),
            })),
            req.seq,
        ))
    }

    // ── agent.remove_msg_tunnel ─────────────────────────────────────────

    pub(crate) async fn handle_agent_remove_msg_tunnel(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;

        let agent_id = Self::require_param_str(&req, "agent_id")?;
        let platform = Self::require_param_str(&req, "platform")?;

        let client = system_config_client_for_caller(&req).await?;

        // Bindings live inside agents/{id}/settings under the "bindings" key
        // (see handle_agent_set_msg_tunnel for RBAC rationale).
        let settings_path = format!("agents/{}/settings", agent_id);
        let mut settings_obj: Value = match client.get(&settings_path).await {
            Ok(val) => serde_json::from_str(&val.value).unwrap_or_else(|_| json!({})),
            Err(_) => {
                return Err(RPCErrors::ReasonError(format!(
                    "No bindings found for agent '{}'",
                    agent_id
                )));
            }
        };
        let mut bindings: Vec<UserTunnelBinding> = settings_obj
            .get("bindings")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let original_len = bindings.len();
        bindings.retain(|b| b.platform != platform);

        if bindings.len() == original_len {
            return Err(RPCErrors::ReasonError(format!(
                "No binding for platform '{}' found on agent '{}'",
                platform, agent_id
            )));
        }

        settings_obj["bindings"] = serde_json::to_value(&bindings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        let settings_json = serde_json::to_string(&settings_obj)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &settings_json)
            .await
            .map_err(|e| {
                RPCErrors::ReasonError(format!("Failed to update agent bindings: {}", e))
            })?;

        info!(
            "Agent '{}' tunnel binding for '{}' removed by '{}'",
            agent_id, platform, principal.username
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "agent_id": agent_id,
                "platform": platform,
                "remaining_bindings": bindings.len(),
            })),
            req.seq,
        ))
    }
}
