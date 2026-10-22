use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::image_data_url::{validate_image_data_url, MAX_AVATAR_PX};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{kRPC, RPCErrors, RPCRequest, RPCResponse, RPCResult};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    get_buckyos_api_runtime, AccessDenial, DenyReason, PasswordState, ProfileLink, SchedulerClient,
    SystemConfigClient, SystemConfigError, UserContactSettings, UserPrivateProfile, UserProfile,
    UserSettings, UserState, UserTunnelBinding, UserType, CONTROL_PANEL_SERVICE_UNIQUE_ID,
    SCHEDULER_SERVICE_SERVICE_PORT,
};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use jsonwebtoken::jwk::Jwk;
use log::*;
use name_lib::{generate_ed25519_key_pair, OwnerDocument, DID};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

const USER_INVITE_PREFIX: &str = "services/control_panel/user_invites";
const PROFILE_SYSTEM_CONTACT_KEY: &str = "system_contact";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UserInviteRecord {
    invite_id: String,
    created_by: String,
    created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_name: Option<String>,
    default_user_type: UserType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted_user_id: Option<String>,
}

// ─── helpers ────────────────────────────────────────────────────────────────

/// Resolve the target user_id from the request; defaults to the caller.
fn resolve_target_user_id(req: &RPCRequest, principal: &RpcAuthPrincipal) -> String {
    ControlPanelServer::param_str(req, "user_id").unwrap_or_else(|| principal.username.clone())
}

/// Build a fresh `SystemConfigClient` authenticated with the *caller's* RPC
/// session token, so user records are read and written under the caller's
/// own RBAC permissions instead of the control_panel service identity.
async fn system_config_client_for_caller(
    req: &RPCRequest,
) -> Result<SystemConfigClient, RPCErrors> {
    let runtime = get_buckyos_api_runtime()?;
    let url = runtime.get_system_config_url();
    let token = req
        .token
        .as_deref()
        .ok_or_else(|| RPCErrors::InvalidToken("missing caller session token".to_string()))?;
    Ok(SystemConfigClient::new(Some(url.as_str()), Some(token)))
}

/// Ensure the caller is admin/root **or** is operating on their own account.
fn require_self_or_admin(
    principal: &RpcAuthPrincipal,
    target_user_id: &str,
) -> Result<(), RPCErrors> {
    if principal.user_type.is_privileged() || principal.username == target_user_id {
        Ok(())
    } else {
        Err(RPCErrors::ReasonError(
            "Only admin or the user themselves can perform this operation".to_string(),
        ))
    }
}

pub(crate) fn require_admin(principal: &RpcAuthPrincipal) -> Result<(), RPCErrors> {
    if principal.user_type.is_privileged() {
        Ok(())
    } else {
        Err(RPCErrors::ReasonError(
            "Admin privileges required".to_string(),
        ))
    }
}

/// Admin methods that write with the Control Panel service identity: only from
/// the administrator's Control Panel session, never from an App holding it.
pub(crate) fn require_control_panel_admin(principal: &RpcAuthPrincipal) -> Result<(), RPCErrors> {
    require_admin(principal)?;
    if !principal.is_control_panel_session {
        return Err(RPCErrors::NoPermission(
            "this admin method needs a Control Panel session".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_username(name: &str) -> Result<(), RPCErrors> {
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

pub(crate) fn validate_password_hash(password_hash: &str) -> Result<(), RPCErrors> {
    let decoded = STANDARD.decode(password_hash).map_err(|_| {
        RPCErrors::ParseRequestError(
            "password_hash must be a base64-encoded SHA-256 digest".to_string(),
        )
    })?;
    if decoded.len() != 32 {
        return Err(RPCErrors::ParseRequestError(
            "password_hash must be a base64-encoded SHA-256 digest".to_string(),
        ));
    }
    Ok(())
}

/// Only the assignable account types (admin, user, limited) are accepted.
fn parse_user_type(s: &str) -> Result<UserType, RPCErrors> {
    UserType::parse(s.trim().to_lowercase().as_str())
        .filter(|user_type| user_type.is_assignable())
        .ok_or_else(|| {
            RPCErrors::ParseRequestError(format!(
                "Invalid user_type: {} (expected admin, user or limited)",
                s
            ))
        })
}

/// `groups` names system user groups; each must be defined.
pub(crate) async fn parse_system_groups(req: &RPCRequest) -> Result<Vec<String>, RPCErrors> {
    let Some(value) = req.params.get("groups") else {
        return Ok(Vec::new());
    };
    let mut groups: Vec<String> = serde_json::from_value(value.clone()).map_err(|e| {
        RPCErrors::ParseRequestError(format!("groups must be a list of group ids: {}", e))
    })?;
    groups.sort();
    groups.dedup();
    if groups.is_empty() {
        return Ok(groups);
    }
    let defined = crate::user_group_mgr::load_user_group_directory()
        .await?
        .defined_group_ids();
    for group in &groups {
        if !defined.contains(group) {
            return Err(RPCErrors::ParseRequestError(format!(
                "user group '{}' does not exist",
                group
            )));
        }
    }
    Ok(groups)
}

/// P-2: an invitation brings in a User or Limited account; Admin comes from
/// `user.change_type`.
fn invite_user_type(raw: Option<&str>) -> Result<UserType, RPCErrors> {
    let user_type = raw
        .map(parse_user_type)
        .transpose()?
        .unwrap_or(UserType::User);
    if !matches!(user_type, UserType::User | UserType::Limited) {
        return Err(RPCErrors::ParseRequestError(
            "user.invite.create invites User or Limited accounts; promote to Admin with user.change_type".to_string(),
        ));
    }
    Ok(user_type)
}

/// PRE-04: who may replace an account's password with `user.change_password`,
/// always from a Control Panel session (an App-target session never changes a
/// password). The account itself when its override (or type default) allows it
/// and it already has a password (the first one goes through
/// `auth.invite.set_password`): a User or Admin re-proves the password with a
/// sudo session, as the system-config rule it replaces required; a Limited
/// account cannot sudo and relies on its override. An administrator changes
/// another account's password with a sudo session.
fn password_change_allowed(
    principal: &RpcAuthPrincipal,
    target: &str,
    settings: &UserSettings,
) -> Result<(), RPCErrors> {
    if principal.session.is_external || !principal.is_control_panel_session {
        return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)
            .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "user.change_password")
            .into_rpc_error());
    }
    if principal.username == target {
        if !settings.password_state.is_set() {
            return Err(AccessDenial::new(DenyReason::CredentialState)
                .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "user.change_password")
                .with_detail("password_unset")
                .into_rpc_error());
        }
        if !settings.can_change_password() {
            return Err(RPCErrors::NoPermission(
                "This account is not allowed to change its password".to_string(),
            ));
        }
        if !principal.is_restricted() && !principal.session.is_sudo {
            return Err(RPCErrors::NoPermission(
                "changing your password needs a sudo session".to_string(),
            ));
        }
        return Ok(());
    }
    if !principal.user_type.is_privileged() || principal.is_restricted() {
        return Err(RPCErrors::NoPermission(
            "Only admin or the user themselves can perform this operation".to_string(),
        ));
    }
    if !principal.session.is_sudo {
        return Err(RPCErrors::NoPermission(
            "changing another account's password needs a sudo session".to_string(),
        ));
    }
    Ok(())
}

fn settings_options_value(settings: &UserSettings) -> Value {
    json!({
        "groups": settings.groups,
        "allow_password_change": settings.allow_password_change,
        "allow_contact_edit": settings.allow_contact_edit,
        "can_change_password": settings.can_change_password(),
        "can_edit_contacts": settings.can_edit_contacts(),
    })
}

fn parse_user_state(s: &str) -> Result<UserState, RPCErrors> {
    UserState::try_from(s.to_string())
        .map_err(|_| RPCErrors::ParseRequestError(format!("Invalid user state: {}", s)))
}

fn default_contact_settings(did: Option<String>) -> UserContactSettings {
    UserContactSettings {
        did,
        ..Default::default()
    }
}

fn public_profile_from_parts(
    did: DID,
    name: Option<String>,
    display_name: Option<String>,
    avatar: Option<String>,
    meta: Option<Value>,
    extra: HashMap<String, Value>,
) -> UserProfile {
    UserProfile {
        did,
        name,
        display_name,
        avatar,
        meta,
        headline: None,
        bio: None,
        location: None,
        organization: None,
        title: None,
        birthday: None,
        tags: Vec::new(),
        bkg_image: None,
        links: HashMap::new(),
        public_contacts: HashMap::new(),
        extra,
    }
}

fn profile_from_owner_config(owner_config: &OwnerDocument) -> UserPrivateProfile {
    UserPrivateProfile::from(public_profile_from_parts(
        owner_config.id.clone(),
        Some(owner_config.name.clone()),
        Some(owner_config.display_name.clone()),
        owner_config.avatar.clone(),
        owner_config.meta.clone(),
        owner_config.extra_info.clone(),
    ))
}

fn profile_from_user_id(user_id: &str) -> UserPrivateProfile {
    UserPrivateProfile::from(public_profile_from_parts(
        DID::new("bns", user_id),
        Some(user_id.to_string()),
        Some(user_id.to_string()),
        None,
        None,
        HashMap::new(),
    ))
}

pub(crate) fn profile_system_contact(profile: &UserPrivateProfile) -> Option<UserContactSettings> {
    profile
        .private_extra
        .get(PROFILE_SYSTEM_CONTACT_KEY)
        .and_then(|value| serde_json::from_value::<UserContactSettings>(value.clone()).ok())
}

fn set_profile_system_contact(
    profile: &mut UserPrivateProfile,
    contact: &UserContactSettings,
) -> Result<(), RPCErrors> {
    profile.private_extra.insert(
        PROFILE_SYSTEM_CONTACT_KEY.to_string(),
        serde_json::to_value(contact)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize contact error: {}", e)))?,
    );
    Ok(())
}

/// The locally saved profile is the explicit setting; the DID document only fills fields it leaves empty.
fn merge_profile_values(local_profile: Option<Value>, did_profile: Option<Value>) -> Value {
    let mut merged = match local_profile {
        Some(Value::Object(profile)) => profile,
        _ => serde_json::Map::new(),
    };
    if let Some(Value::Object(document)) = did_profile {
        for (key, value) in document {
            let unset = merged.get(&key).map_or(true, |current| {
                current.is_null() || current.as_str().is_some_and(|text| text.trim().is_empty())
            });
            if unset {
                merged.insert(key, value);
            }
        }
    }
    Value::Object(merged)
}

fn apply_profile_text_patch(
    profile: &mut UserPrivateProfile,
    req: &RPCRequest,
) -> Result<(), RPCErrors> {
    let avatar_key = if req.params.get("avatar").is_some() {
        "avatar"
    } else {
        "avatar_url"
    };
    if let Some(avatar) = ControlPanelServer::param_text_patch(req, avatar_key)? {
        if let Some(value) = avatar.as_deref() {
            validate_image_data_url("avatar", value, MAX_AVATAR_PX)?;
        }
        profile.avatar = avatar;
    }
    for (key, slot) in [
        ("display_name", &mut profile.display_name),
        ("name", &mut profile.name),
        ("headline", &mut profile.headline),
        ("title", &mut profile.title),
        ("bio", &mut profile.bio),
        ("location", &mut profile.location),
        ("organization", &mut profile.organization),
        ("birthday", &mut profile.birthday),
        ("bkg_image", &mut profile.bkg_image),
    ] {
        if let Some(value) = ControlPanelServer::param_text_patch(req, key)? {
            *slot = value;
        }
    }
    if let Some(website) = ControlPanelServer::param_text_patch(req, "website")? {
        match website {
            Some(url) => {
                profile.links.insert(
                    "website".to_string(),
                    ProfileLink {
                        label: "Website".to_string(),
                        url,
                    },
                );
            }
            None => {
                profile.links.remove("website");
            }
        }
    }
    for key in ["email", "phone"] {
        if let Some(value) = ControlPanelServer::param_text_patch(req, key)? {
            match value {
                Some(text) => {
                    profile.extra.insert(key.to_string(), Value::String(text));
                }
                None => {
                    profile.extra.remove(key);
                }
            }
        }
    }
    Ok(())
}

fn public_profile_value(profile: &UserPrivateProfile) -> Value {
    serde_json::to_value(profile.to_public_profile()).unwrap_or_else(|_| json!({}))
}

fn profile_value_from_doc(doc: &Value) -> Option<Value> {
    doc.get("profile")
        .cloned()
        .or_else(|| doc.get("meta").cloned())
        .filter(|value| value.is_object())
}

fn parse_owner_config_value(value: &Value) -> Result<OwnerDocument, RPCErrors> {
    match value {
        Value::String(raw) => serde_json::from_str(raw)
            .map_err(|e| RPCErrors::ParseRequestError(format!("Invalid owner_config: {}", e))),
        Value::Object(_) => serde_json::from_value(value.clone())
            .map_err(|e| RPCErrors::ParseRequestError(format!("Invalid owner_config: {}", e))),
        _ => Err(RPCErrors::ParseRequestError(
            "owner_config must be a JSON object or string".to_string(),
        )),
    }
}

fn require_invite_owner_session(
    principal: Option<&RpcAuthPrincipal>,
    owner_did: &DID,
) -> Result<(), RPCErrors> {
    let principal = ControlPanelServer::require_rpc_principal(principal)?;
    if !principal.is_user_session
        || !principal.is_control_panel_session
        || principal.session.auth_method != Some(buckyos_api::AuthMethod::DidFederation)
        || principal.owner_did != owner_did.to_string()
    {
        return Err(RPCErrors::NoPermission(
            "accepting an invitation requires a DID federation session for its owner_config DID"
                .to_string(),
        ));
    }
    Ok(())
}

async fn accept_user_invitation(
    kv: &dyn AgentKv,
    invite_id: &str,
    owner_config: &OwnerDocument,
    password_hash: &str,
    now: u64,
) -> Result<(String, UserInviteRecord), RPCErrors> {
    let invite_path = format!("{USER_INVITE_PREFIX}/{invite_id}");
    let (raw, version) = kv
        .get(&invite_path)
        .await?
        .ok_or_else(|| RPCErrors::ReasonError(format!("Invite '{invite_id}' not found")))?;
    let mut invite: UserInviteRecord = serde_json::from_str(&raw)
        .map_err(|error| RPCErrors::ReasonError(format!("Corrupted invite: {error}")))?;
    if invite.state != "pending" {
        return Err(RPCErrors::ReasonError(format!(
            "Invite '{invite_id}' is not pending"
        )));
    }
    if invite
        .expires_at
        .map(|expires| expires < now)
        .unwrap_or(false)
    {
        return Err(RPCErrors::ReasonError(format!(
            "Invite '{invite_id}' has expired"
        )));
    }
    if let Some(target_did) = invite.target_did.as_ref() {
        if target_did != &owner_config.id.to_string() {
            return Err(RPCErrors::ReasonError(format!(
                "Invite target '{target_did}' does not match owner_config '{}'",
                owner_config.id.to_string()
            )));
        }
    }

    let user_id = invite
        .target_user_id
        .clone()
        .unwrap_or_else(|| owner_config.name.trim().to_lowercase());
    validate_username(&user_id)?;
    if !password_hash.is_empty() {
        validate_password_hash(password_hash)?;
    }
    let settings_path = format!("users/{user_id}/settings");
    let existing = kv.get(&settings_path).await?;
    let new_account = existing.is_none();
    let mut settings = match existing {
        Some((raw, _)) => {
            let mut settings: UserSettings = serde_json::from_str(&raw).map_err(|error| {
                RPCErrors::ReasonError(format!("Corrupted user settings: {error}"))
            })?;
            if invite.target_user_id.as_deref() != Some(user_id.as_str())
                || !matches!(settings.state, UserState::Pending)
            {
                return Err(RPCErrors::ReasonError(format!(
                    "User '{user_id}' already exists or is reserved by another invitation"
                )));
            }
            settings.user_type = invite.default_user_type;
            settings.groups = invite.groups.clone();
            if !password_hash.is_empty() {
                settings.password = password_hash.to_string();
            }
            settings.state = UserState::Active;
            settings.is_local = false;
            settings
        }
        None => UserSettings {
            user_id: user_id.clone(),
            user_type: invite.default_user_type,
            password: password_hash.to_string(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: false,
            allow_password_change: None,
            allow_contact_edit: None,
            groups: invite.groups.clone(),
            password_state: PasswordState::Unset,
        },
    };
    settings.password_state = if settings.password.is_empty() {
        PasswordState::Unset
    } else {
        PasswordState::Set
    };
    let mut profile = profile_from_owner_config(owner_config);
    profile.display_name = Some(
        invite
            .show_name
            .clone()
            .unwrap_or_else(|| owner_config.display_name.clone()),
    );
    set_profile_system_contact(
        &mut profile,
        &default_contact_settings(Some(owner_config.id.to_string())),
    )?;
    invite.state = "accepted".to_string();
    invite.accepted_at = Some(now);
    invite.accepted_user_id = Some(user_id.clone());

    let settings_json = to_json(&settings)?;
    let profile_json = to_json(&profile)?;
    let tx = HashMap::from([
        (
            settings_path,
            if new_account {
                KVAction::Create(settings_json)
            } else {
                KVAction::Update(settings_json)
            },
        ),
        (
            format!("users/{user_id}/doc"),
            KVAction::Create(to_json(owner_config)?),
        ),
        (
            format!("users/{user_id}/profile"),
            if new_account {
                KVAction::Create(profile_json)
            } else {
                KVAction::Update(profile_json)
            },
        ),
        (invite_path.clone(), KVAction::Update(to_json(&invite)?)),
    ]);
    kv.exec_tx(tx, Some((invite_path, version)))
        .await
        .map_err(|error| RPCErrors::ReasonError(format!("Failed to accept invite: {error}")))?;
    Ok((user_id, invite))
}

fn parse_user_profile_payload(
    value: &Value,
    fallback_did: &DID,
) -> Result<UserPrivateProfile, RPCErrors> {
    let mut profile_value = value.clone();
    if let Value::Object(map) = &mut profile_value {
        if !map.contains_key("did") && !map.contains_key("id") {
            map.insert("did".to_string(), Value::String(fallback_did.to_string()));
        }
    }
    let profile = serde_json::from_value::<UserPrivateProfile>(profile_value)
        .map_err(|e| RPCErrors::ParseRequestError(format!("Invalid profile payload: {}", e)))?;
    if profile.did != *fallback_did {
        return Err(RPCErrors::ParseRequestError(
            "profile.did does not match target user".to_string(),
        ));
    }
    Ok(profile)
}

fn value_contains_zone(value: &Value, zone_did: &str) -> bool {
    match value {
        Value::String(text) => text == zone_did,
        Value::Array(items) => items.iter().any(|item| value_contains_zone(item, zone_did)),
        Value::Object(map) => map.values().any(|item| value_contains_zone(item, zone_did)),
        _ => false,
    }
}

fn owner_is_bound_to_zone(owner_config: &OwnerDocument, zone_did: &DID) -> bool {
    if owner_config.id == *zone_did {
        return true;
    }
    if owner_config.is_bound_to_zone(zone_did) {
        return true;
    }
    let zone = zone_did.to_string();
    owner_config
        .extra_info
        .get("binded_zone_list")
        .or_else(|| owner_config.extra_info.get("bound_zone_list"))
        .map(|value| value_contains_zone(value, zone.as_str()))
        .unwrap_or(false)
}

/// A Zone-hosted OwnerDocument (`<user>.<zone>`) and its private key. Only
/// accounts this Zone is the canonical zone of name it as the default zone;
/// exclusive-invite and self-registered accounts never do (D-2).
fn generated_owner_config(
    user_id: &str,
    show_name: &str,
    zone_did: &DID,
    canonical_here: bool,
) -> Result<(OwnerDocument, String), RPCErrors> {
    let (private_key, public_key) = generate_ed25519_key_pair();
    let public_key: Jwk = serde_json::from_value(public_key)
        .map_err(|e| RPCErrors::ReasonError(format!("Invalid generated public key: {}", e)))?;
    let user_did = DID::new(
        zone_did.method.as_str(),
        format!("{}.{}", user_id, zone_did.id).as_str(),
    );
    let mut owner_config = OwnerDocument::new(
        user_did,
        user_id.to_string(),
        show_name.to_string(),
        public_key,
    );
    if canonical_here {
        owner_config.set_default_zone_did(zone_did.clone());
    }
    Ok((owner_config, private_key))
}

fn to_json<T: Serialize>(value: &T) -> Result<String, RPCErrors> {
    serde_json::to_string(value)
        .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))
}

/// The records of a new local account (settings, OwnerDocument, key, profile),
/// all `Create` so a concurrent account of the same name fails the transaction.
/// Returns them with the account DID.
pub(crate) fn new_account_actions(
    settings: &UserSettings,
    show_name: &str,
    zone_did: &DID,
    canonical_here: bool,
) -> Result<(HashMap<String, KVAction>, DID), RPCErrors> {
    let user_id = settings.user_id.as_str();
    let (owner_config, private_key) =
        generated_owner_config(user_id, show_name, zone_did, canonical_here)?;
    let user_did = owner_config.id.clone();
    let settings_json = to_json(settings)?;
    let doc_json = to_json(&owner_config)?;
    let mut profile = profile_from_owner_config(&owner_config);
    set_profile_system_contact(
        &mut profile,
        &default_contact_settings(Some(user_did.to_string())),
    )?;
    let profile_json = to_json(&profile)?;
    let actions = HashMap::from([
        (
            format!("users/{user_id}/settings"),
            KVAction::Create(settings_json),
        ),
        (format!("users/{user_id}/doc"), KVAction::Create(doc_json)),
        (
            format!("security/{user_id}/key"),
            KVAction::Create(private_key),
        ),
        (
            format!("users/{user_id}/profile"),
            KVAction::Create(profile_json),
        ),
    ]);
    Ok((actions, user_did))
}

/// After a committed account: RBAC places it in its type group and msg-center
/// learns it. The RBAC result is returned; a failed msg-center reload only warns.
pub(crate) async fn after_account_created(user_id: &str, reason: &str) -> Result<(), RPCErrors> {
    let refresh_result = refresh_rbac_by_scheduler(reason).await;
    if let Err(error) = &refresh_result {
        warn!("User '{user_id}' was committed but scheduler RBAC refresh failed: {error}");
    }
    if let Err(error) = crate::agent_mgr::reload_msg_center().await {
        warn!("msg-center reload after creating {user_id} failed: {error}");
    }
    refresh_result
}

async fn load_user_settings(
    client: &SystemConfigClient,
    user_id: &str,
) -> Result<UserSettings, RPCErrors> {
    let settings_path = format!("users/{}/settings", user_id);
    let settings_val = client
        .get(&settings_path)
        .await
        .map_err(|e| RPCErrors::ReasonError(format!("User '{}' not found: {}", user_id, e)))?;
    serde_json::from_str(&settings_val.value)
        .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))
}

pub(crate) async fn load_user_profile(
    client: &SystemConfigClient,
    user_id: &str,
) -> Result<Option<UserPrivateProfile>, RPCErrors> {
    let profile_path = format!("users/{}/profile", user_id);
    match client.get(&profile_path).await {
        Ok(profile_val) => serde_json::from_str::<UserPrivateProfile>(&profile_val.value)
            .map(Some)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user profile: {}", e))),
        Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
        Err(e) => Err(RPCErrors::ReasonError(format!(
            "Failed to load user profile: {}",
            e
        ))),
    }
}

async fn save_user_profile(
    client: &SystemConfigClient,
    user_id: &str,
    profile: &UserPrivateProfile,
) -> Result<(), RPCErrors> {
    let profile_path = format!("users/{}/profile", user_id);
    let profile_json = serde_json::to_string(profile)
        .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
    if client.set(&profile_path, &profile_json).await.is_err() {
        client
            .create(&profile_path, &profile_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to save user profile: {}", e)))?;
    }
    Ok(())
}

pub(crate) async fn refresh_rbac_by_scheduler(reason: &str) -> Result<(), RPCErrors> {
    let runtime = get_buckyos_api_runtime()?;
    let scheduler_url = format!(
        "http://127.0.0.1:{}/kapi/scheduler",
        SCHEDULER_SERVICE_SERVICE_PORT
    );
    let session_token = runtime.get_session_token().await;
    let scheduler_client =
        SchedulerClient::new(kRPC::new(scheduler_url.as_str(), Some(session_token)));
    let response = scheduler_client.refresh_rbac().await?;
    info!(
        "Scheduler RBAC refresh after {}: updated={} tx_action_count={}",
        reason, response.updated, response.tx_action_count
    );
    Ok(())
}

fn user_create_result(
    user_id: &str,
    user_type: &UserType,
    refresh_result: Result<(), RPCErrors>,
) -> Value {
    let (rbac_refreshed, warning) = match refresh_result {
        Ok(()) => (true, None),
        Err(error) => (
            false,
            Some(format!(
                "User was created, but RBAC refresh is pending: {}",
                error
            )),
        ),
    };
    let mut result = json!({
        "ok": true,
        "created": true,
        "rbac_refreshed": rbac_refreshed,
        "user_id": user_id,
        "user_type": user_type,
        "state": "active",
    });
    if let Some(warning) = warning {
        result["warning"] = Value::String(warning);
    }
    result
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
        let include_deleted = Self::param_bool(&req, "include_deleted").unwrap_or(false);
        // Directory enumeration (`list("users")`) checks the bare path
        // `/config/users`, which the admin rule `/config/users/*` does not
        // match. Use the service token here (control-panel is in the `kernel`
        // group and has full read access); individual per-user reads below
        // are unaffected.
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;

        let user_ids = client
            .list("users")
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to list users: {}", e)))?;

        let mut users: Vec<Value> = Vec::new();
        for uid in &user_ids {
            let settings_path = format!("users/{}/settings", uid);
            match client.get(&settings_path).await {
                Ok(val) => {
                    if let Ok(settings) = serde_json::from_str::<UserSettings>(&val.value) {
                        if !include_deleted && matches!(settings.state, UserState::Deleted) {
                            continue;
                        }
                        let mut info = settings.to_user_info();
                        let mut public_profile = None;
                        if let Ok(Some(profile)) = load_user_profile(&client, uid).await {
                            public_profile = Some(profile.to_public_profile());
                            if let Some(show_name) = profile.display_name.or(profile.name) {
                                info.show_name = show_name;
                            }
                        }
                        if let Ok(mut v) = serde_json::to_value(&info) {
                            if let Some(public_profile) = public_profile {
                                v["did"] = Value::String(public_profile.did.to_string());
                                if let Some(avatar) = public_profile.avatar {
                                    v["avatar"] = Value::String(avatar);
                                }
                            }
                            v["is_local"] = Value::Bool(settings.is_local);
                            if let (Value::Object(target), Value::Object(options)) =
                                (&mut v, settings_options_value(&settings))
                            {
                                target.extend(options);
                            }
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
        let settings_val = client
            .get(&settings_path)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e)))?;
        let settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;
        require_self_or_admin(principal, &target)?;

        // Build response – hide password; profile fields live in users/{user}/profile.
        let mut result = json!({
            "user_id": settings.user_id,
            "user_type": settings.user_type,
            "state": settings.state.clone(),
            "res_pool_id": settings.res_pool_id,
            "is_local": settings.is_local,
            "groups": settings.groups,
            "allow_password_change": settings.can_change_password(),
            "allow_contact_edit": settings.can_edit_contacts(),
        });

        let local_profile = load_user_profile(&client, &target).await?;
        if let Some(profile) = local_profile.as_ref() {
            result["local_profile"] = serde_json::to_value(profile).unwrap_or(json!(null));
            result["profile"] =
                serde_json::to_value(profile.to_public_profile()).unwrap_or(json!(null));
        } else {
            result["profile"] = json!({});
        }

        // Try to load the DID document (best-effort)
        let doc_path = format!("users/{}/doc", target);
        if let Ok(doc_val) = client.get(&doc_path).await {
            if let Ok(doc) = serde_json::from_str::<Value>(&doc_val.value) {
                result["profile"] = merge_profile_values(
                    local_profile.as_ref().map(public_profile_value),
                    profile_value_from_doc(&doc),
                );
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
        validate_password_hash(&password_hash)?;

        let show_name = Self::param_str(&req, "show_name").unwrap_or_else(|| user_id.clone());
        let user_type = Self::param_str(&req, "user_type")
            .map(|s| parse_user_type(&s))
            .transpose()?
            .unwrap_or(UserType::User);
        let allow_password_change = Self::param_bool(&req, "allow_password_change");
        let allow_contact_edit = Self::param_bool(&req, "allow_contact_edit");

        if !matches!(user_type, UserType::User | UserType::Limited) {
            return Err(RPCErrors::ParseRequestError(
                "user.create supports the User and Limited types; promote to Admin with user.change_type".to_string(),
            ));
        }
        let groups = parse_system_groups(&req).await?;

        let client = system_config_client_for_caller(&req).await?;
        let runtime = get_buckyos_api_runtime()?;
        crate::agent_mgr::ensure_name_available(&runtime.zone_id, &user_id).await?;

        // Build UserSettings
        let new_settings = UserSettings {
            user_id: user_id.clone(),
            user_type,
            password: password_hash,
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change,
            allow_contact_edit,
            groups,
            password_state: buckyos_api::PasswordState::Set,
        };
        let (tx, _) = new_account_actions(&new_settings, &show_name, &runtime.zone_id, true)?;
        client
            .exec_tx(tx, None)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to create user: {}", e)))?;

        let refresh_result = after_account_created(&user_id, "user.create").await;

        info!("User '{}' created by '{}'", user_id, principal.username);

        Ok(RPCResponse::new(
            RPCResult::Success(user_create_result(
                &user_id,
                &new_settings.user_type,
                refresh_result,
            )),
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
        let _settings = load_user_settings(&client, &target).await?;
        let mut profile = load_user_profile(&client, &target)
            .await?
            .unwrap_or_else(|| profile_from_user_id(&target));

        // Apply updates
        if let Some(show_name) = Self::param_str(&req, "show_name") {
            profile.display_name = Some(show_name);
        }
        save_user_profile(&client, &target, &profile).await?;
        if let Err(error) = crate::agent_mgr::reload_msg_center().await {
            warn!("msg-center reload after updating {target} failed: {error}");
        }

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
    // Updates the user's system-level contact/binding profile data (DID, note, groups, tags, bindings).
    // NOTE: Full contact/friend management lives in MessageCenter.

    pub(crate) async fn handle_user_update_contact(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);
        require_self_or_admin(principal, &target)?;

        let client = system_config_client_for_caller(&req).await?;
        let _settings = load_user_settings(&client, &target).await?;
        let mut profile = load_user_profile(&client, &target)
            .await?
            .unwrap_or_else(|| profile_from_user_id(&target));
        let mut contact = profile_system_contact(&profile).unwrap_or_default();

        // Apply partial updates
        if let Some(did) = Self::param_str(&req, "did") {
            contact.did = Some(did);
        }
        if let Some(note) = Self::param_str(&req, "note") {
            contact.note = Some(note);
        }
        if req.params.get("groups").is_some() {
            return Err(RPCErrors::ParseRequestError(
                "groups was removed from contact settings; system user groups are managed by user_group.set_members".to_string(),
            ));
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
        set_profile_system_contact(&mut profile, &contact)?;
        save_user_profile(&client, &target, &profile).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "contact": serde_json::to_value(&contact).unwrap_or(json!(null)),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_profile_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);
        require_self_or_admin(principal, &target)?;

        let client = system_config_client_for_caller(&req).await?;
        let settings = load_user_settings(&client, &target).await?;
        let local_profile = load_user_profile(&client, &target).await?;
        let mut did_profile = None;
        let doc_path = format!("users/{}/doc", target);
        if let Ok(doc_val) = client.get(&doc_path).await {
            if let Ok(doc) = serde_json::from_str::<Value>(&doc_val.value) {
                did_profile = profile_value_from_doc(&doc);
            }
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "user_id": target,
                "profile": merge_profile_values(
                    local_profile.as_ref().map(public_profile_value),
                    did_profile.clone()
                ),
                "local_profile": local_profile,
                "did_profile": did_profile,
                "is_local": settings.is_local,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_profile_set(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);
        require_self_or_admin(principal, &target)?;

        let scope = Self::param_str(&req, "scope").unwrap_or_else(|| "local".to_string());
        if scope != "local" {
            return Err(RPCErrors::ReasonError(
                "Only local profile updates are supported by control_panel".to_string(),
            ));
        }

        let client = system_config_client_for_caller(&req).await?;
        let _settings = load_user_settings(&client, &target).await?;
        let fallback_profile = match load_user_profile(&client, &target).await? {
            Some(profile) => profile,
            None => profile_from_user_id(&target),
        };
        let fallback_did = fallback_profile.did.clone();
        let previous_avatar = fallback_profile.avatar.clone();
        let mut profile = if let Some(value) = req.params.get("profile") {
            parse_user_profile_payload(value, &fallback_did)?
        } else {
            fallback_profile
        };
        if profile.avatar != previous_avatar {
            if let Some(avatar) = profile.avatar.as_deref() {
                validate_image_data_url("avatar", avatar, MAX_AVATAR_PX)?;
            }
        }

        apply_profile_text_patch(&mut profile, &req)?;
        if let Some(tags) = req.params.get("tags") {
            profile.tags = serde_json::from_value(tags.clone()).map_err(|e| {
                RPCErrors::ParseRequestError(format!("Invalid profile tags payload: {}", e))
            })?;
        }
        if let Some(extra) = req.params.get("extra") {
            profile.extra = serde_json::from_value(extra.clone()).map_err(|e| {
                RPCErrors::ParseRequestError(format!("Invalid profile extra payload: {}", e))
            })?;
        }
        if let Some(private_extra) = req.params.get("private_extra") {
            profile.private_extra = serde_json::from_value(private_extra.clone()).map_err(|e| {
                RPCErrors::ParseRequestError(format!("Invalid private_extra payload: {}", e))
            })?;
        }

        save_user_profile(&client, &target, &profile).await?;
        if let Err(error) = crate::agent_mgr::reload_msg_center().await {
            warn!("msg-center reload after profile update of {target} failed: {error}");
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "profile": profile,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_set_msg_tunnel(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);
        require_self_or_admin(principal, &target)?;

        let platform = Self::require_param_str(&req, "platform")?;
        let mut account_id = Self::require_param_str(&req, "account_id")?;
        if platform == "telegram" {
            account_id = crate::agent_mgr::bare_telegram_account(&account_id).to_string();
        }
        if account_id.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "account_id cannot be empty".to_string(),
            ));
        }
        let display_id = Self::param_str(&req, "display_id");
        let tunnel_instance_id = Self::param_str(&req, "tunnel_instance_id");
        let status = Self::param_str(&req, "status");
        let last_sync_at = Self::param_u64(&req, "last_sync_at");
        let meta: HashMap<String, String> = req
            .params
            .get("meta")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let client = system_config_client_for_caller(&req).await?;
        let _settings = load_user_settings(&client, &target).await?;
        let mut profile = load_user_profile(&client, &target)
            .await?
            .unwrap_or_else(|| profile_from_user_id(&target));
        let mut contact =
            profile_system_contact(&profile).unwrap_or_else(|| default_contact_settings(None));

        let binding = UserTunnelBinding {
            platform: platform.clone(),
            account_id,
            display_id,
            tunnel_instance_id,
            status,
            last_sync_at,
            meta,
        };
        if let Some(pos) = contact
            .bindings
            .iter()
            .position(|binding| binding.platform == platform)
        {
            contact.bindings[pos] = binding;
        } else {
            contact.bindings.push(binding);
        }
        set_profile_system_contact(&mut profile, &contact)?;
        save_user_profile(&client, &target, &profile).await?;
        if let Err(error) = crate::agent_mgr::reload_msg_center().await {
            warn!("msg-center reload after binding {platform} for {target} failed: {error}");
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "platform": platform,
                "total_bindings": contact.bindings.len(),
                "contact": contact,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_remove_msg_tunnel(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let target = resolve_target_user_id(&req, principal);
        require_self_or_admin(principal, &target)?;
        let platform = Self::require_param_str(&req, "platform")?;

        let client = system_config_client_for_caller(&req).await?;
        let _settings = load_user_settings(&client, &target).await?;
        let mut profile = load_user_profile(&client, &target)
            .await?
            .unwrap_or_else(|| profile_from_user_id(&target));
        let mut contact = profile_system_contact(&profile)
            .ok_or_else(|| RPCErrors::ReasonError("No user contact settings found".to_string()))?;
        let before = contact.bindings.len();
        contact
            .bindings
            .retain(|binding| binding.platform != platform);
        if before == contact.bindings.len() {
            return Err(RPCErrors::ReasonError(format!(
                "No binding for platform '{}' found on user '{}'",
                platform, target
            )));
        }
        set_profile_system_contact(&mut profile, &contact)?;
        save_user_profile(&client, &target, &profile).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "platform": platform,
                "remaining_bindings": contact.bindings.len(),
                "contact": contact,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_invite_create(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;

        let invite_id = Self::param_str(&req, "invite_id")
            .unwrap_or_else(|| Uuid::new_v4().to_string())
            .trim()
            .to_string();
        if invite_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "invite_id cannot be empty".to_string(),
            ));
        }
        let target_user_id = Self::param_str(&req, "user_id")
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        if let Some(user_id) = target_user_id.as_ref() {
            validate_username(user_id)?;
        }
        let target_did = Self::param_str(&req, "target_did");
        let show_name = Self::param_str(&req, "show_name");
        let default_user_type = invite_user_type(Self::param_str(&req, "user_type").as_deref())?;
        let now = buckyos_get_unix_timestamp();
        let expires_at = Self::param_u64(&req, "expires_at")
            .or_else(|| Self::param_u64(&req, "ttl_secs").map(|ttl| now.saturating_add(ttl)));
        let groups = parse_system_groups(&req).await?;
        if req.params.get("app_ids").is_some() {
            return Err(RPCErrors::ParseRequestError(
                "app_ids was removed; App access is managed by apps.availability.set".to_string(),
            ));
        }

        let invite = UserInviteRecord {
            invite_id: invite_id.clone(),
            created_by: principal.username.clone(),
            created_at: now,
            expires_at,
            state: "pending".to_string(),
            target_user_id: target_user_id.clone(),
            target_did: target_did.clone(),
            show_name: show_name.clone(),
            default_user_type,
            groups: groups.clone(),
            accepted_at: None,
            accepted_user_id: None,
        };

        let client = system_config_client_for_caller(&req).await?;
        let invite_path = format!("{}/{}", USER_INVITE_PREFIX, invite_id);
        if client.get(&invite_path).await.is_ok() {
            return Err(RPCErrors::ReasonError(format!(
                "Invite '{}' already exists",
                invite.invite_id
            )));
        }
        let invite_json = serde_json::to_string(&invite)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;

        let mut tx = HashMap::new();
        tx.insert(invite_path.clone(), KVAction::Create(invite_json));
        if let Some(user_id) = target_user_id.as_ref() {
            let settings_path = format!("users/{}/settings", user_id);
            if client.get(&settings_path).await.is_ok() {
                return Err(RPCErrors::ReasonError(format!(
                    "User '{}' already exists",
                    user_id
                )));
            }
            let pending_settings = UserSettings {
                user_id: user_id.clone(),
                user_type: default_user_type,
                password: String::new(),
                state: UserState::Pending,
                res_pool_id: "default".to_string(),
                is_local: false,
                allow_password_change: None,
                allow_contact_edit: None,
                groups: groups.clone(),
                password_state: buckyos_api::PasswordState::Unset,
            };
            let settings_json = serde_json::to_string(&pending_settings)
                .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
            tx.insert(settings_path, KVAction::Create(settings_json));

            let profile_did = target_did
                .as_deref()
                .map(DID::from_str)
                .transpose()
                .map_err(|e| RPCErrors::ParseRequestError(format!("Invalid target_did: {}", e)))?
                .unwrap_or_else(|| DID::new("bns", user_id));
            let mut profile = UserPrivateProfile::from(public_profile_from_parts(
                profile_did,
                Some(user_id.clone()),
                Some(show_name.clone().unwrap_or_else(|| user_id.clone())),
                None,
                None,
                HashMap::new(),
            ));
            let contact = default_contact_settings(target_did.clone());
            set_profile_system_contact(&mut profile, &contact)?;
            let profile_json = serde_json::to_string(&profile)
                .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
            tx.insert(
                format!("users/{}/profile", user_id),
                KVAction::Create(profile_json),
            );
        }

        client
            .exec_tx(tx, None)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to create invite: {}", e)))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "invite": invite,
                "invite_url": format!("/users/invite/{}", invite_id),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_invite_get(
        &self,
        req: RPCRequest,
        _principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let invite_id = Self::require_param_str(&req, "invite_id")?;
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let invite_path = format!("{}/{}", USER_INVITE_PREFIX, invite_id);
        let invite_val = client.get(&invite_path).await.map_err(|e| {
            RPCErrors::ReasonError(format!("Invite '{}' not found: {}", invite_id, e))
        })?;
        let invite: UserInviteRecord = serde_json::from_str(&invite_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted invite: {}", e)))?;
        let now = buckyos_get_unix_timestamp();
        let expired = invite.expires_at.map(|exp| exp < now).unwrap_or(false);

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "invite": invite,
                "expired": expired,
                "zone_did": runtime.zone_id.to_string(),
                "zone_host": runtime.zone_id.to_host_name(),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_user_invite_accept(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let invite_id = Self::require_param_str(&req, "invite_id")?;
        let owner_config_value = req
            .params
            .get("owner_config")
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing owner_config".to_string()))?;
        let owner_config = parse_owner_config_value(owner_config_value)?;
        require_invite_owner_session(principal, &owner_config.id)?;
        let runtime = get_buckyos_api_runtime()?;
        if !owner_is_bound_to_zone(&owner_config, &runtime.zone_id) {
            return Err(RPCErrors::ReasonError(format!(
                "OwnerDocument '{}' is not bound to zone '{}'",
                owner_config.id.to_string(),
                runtime.zone_id.to_string()
            )));
        }

        let kv = ZoneKv::connect().await?;
        let password_hash = Self::param_str(&req, "password_hash").unwrap_or_default();
        let (user_id, invite) = accept_user_invitation(
            &kv,
            &invite_id,
            &owner_config,
            &password_hash,
            buckyos_get_unix_timestamp(),
        )
        .await?;

        refresh_rbac_by_scheduler("user.invite_accept").await?;
        if let Err(error) = crate::agent_mgr::reload_msg_center().await {
            warn!("msg-center reload after invite accept of {user_id} failed: {error}");
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": user_id,
                "state": "active",
                "invite": invite,
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
            return Err(RPCErrors::ReasonError("Cannot delete yourself".to_string()));
        }

        let client = system_config_client_for_caller(&req).await?;

        // Mark user as deleted rather than physically removing
        let settings_path = format!("users/{}/settings", target);
        let settings_val = client
            .get(&settings_path)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e)))?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        settings.state = UserState::Deleted;
        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to delete user: {}", e)))?;

        refresh_rbac_by_scheduler("user.delete").await?;

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
        let new_password_hash = Self::require_param_str(&req, "new_password_hash")?;
        validate_password_hash(&new_password_hash)?;

        // Checked here, written with the service identity: accounts have no
        // system-config write right on their own settings.
        let kv = ZoneKv::connect().await?;
        let settings_path = format!("users/{}/settings", target);
        let (raw, version) = kv
            .get(&settings_path)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("User '{}' not found", target)))?;
        let mut settings: UserSettings = serde_json::from_str(&raw)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;
        password_change_allowed(principal, &target, &settings)?;

        let first_password = !settings.password_state.is_set();
        settings.password = new_password_hash;
        settings.password_state = PasswordState::Set;
        let mut tx =
            HashMap::from([(settings_path.clone(), KVAction::Update(to_json(&settings)?))]);
        if first_password {
            tx.extend(
                crate::exclusive_invite::consume_invite_actions(
                    &kv,
                    &target,
                    buckyos_get_unix_timestamp(),
                )
                .await?,
            );
        }
        kv.exec_tx(tx, Some((settings_path, version)))
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to change password: {}", e)))?;
        if first_password {
            crate::exclusive_invite::revoke_invite_families(&target, None).await;
        }

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
        let settings_val = client
            .get(&settings_path)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e)))?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        settings.state = new_state;
        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to change state: {}", e)))?;

        refresh_rbac_by_scheduler("user.change_state").await?;

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

        let client = system_config_client_for_caller(&req).await?;

        let settings_path = format!("users/{}/settings", target);
        let settings_val = client
            .get(&settings_path)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("User '{}' not found: {}", target, e)))?;
        let mut settings: UserSettings = serde_json::from_str(&settings_val.value)
            .map_err(|e| RPCErrors::ReasonError(format!("Corrupted user settings: {}", e)))?;

        if matches!(settings.user_type, UserType::Root) {
            return Err(RPCErrors::ReasonError(
                "Cannot change root user type".to_string(),
            ));
        }

        settings.user_type = new_type;
        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to change type: {}", e)))?;

        refresh_rbac_by_scheduler("user.change_type").await?;
        if let Err(error) = crate::agent_mgr::reload_msg_center().await {
            warn!("msg-center reload after changing the type of {target} failed: {error}");
        }

        info!(
            "User '{}' type changed to '{}' by '{}'",
            target, type_str, principal.username
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "user_type": new_type,
            })),
            req.seq,
        ))
    }

    // ── user.set_account_options ───────────────────────────────────────
    // Per-user overrides of the UserType defaults; `null` returns to the default.

    pub(crate) async fn handle_user_set_account_options(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_admin(principal)?;
        let target = Self::require_param_str(&req, "user_id")?;
        let option = |key: &str| -> Result<Option<Option<bool>>, RPCErrors> {
            match req.params.get(key) {
                None => Ok(None),
                Some(Value::Null) => Ok(Some(None)),
                Some(Value::Bool(value)) => Ok(Some(Some(*value))),
                Some(_) => Err(RPCErrors::ParseRequestError(format!(
                    "{key} must be a boolean or null"
                ))),
            }
        };
        let allow_password_change = option("allow_password_change")?;
        let allow_contact_edit = option("allow_contact_edit")?;

        let client = system_config_client_for_caller(&req).await?;
        let settings_path = format!("users/{}/settings", target);
        let mut settings = load_user_settings(&client, &target).await?;
        if matches!(settings.user_type, UserType::Root) {
            return Err(RPCErrors::ReasonError(
                "Cannot change root account options".to_string(),
            ));
        }
        if let Some(value) = allow_password_change {
            settings.allow_password_change = value;
        }
        if let Some(value) = allow_contact_edit {
            settings.allow_contact_edit = value;
        }
        let updated_json = serde_json::to_string(&settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Serialize error: {}", e)))?;
        client
            .set(&settings_path, &updated_json)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to save options: {}", e)))?;
        if allow_contact_edit.is_some() {
            if let Err(error) = crate::agent_mgr::reload_msg_center().await {
                warn!("msg-center reload after options of {target} failed: {error}");
            }
        }
        info!(
            "User '{}' account options changed by '{}'",
            target, principal.username
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "user_id": target,
                "allow_password_change": settings.can_change_password(),
                "allow_contact_edit": settings.can_edit_contacts(),
            })),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_data_url::png_data_url;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct InviteKv {
        data: Mutex<HashMap<String, (String, u64)>>,
        failed_read: Option<String>,
        before_commit: Mutex<Option<(String, String)>>,
        guard: Mutex<Option<(String, u64)>>,
        tx_calls: AtomicUsize,
    }

    impl InviteKv {
        fn put<T: Serialize>(&self, key: &str, value: &T) {
            self.data
                .lock()
                .unwrap()
                .insert(key.to_string(), (to_json(value).unwrap(), 1));
        }

        fn snapshot(&self) -> HashMap<String, (String, u64)> {
            self.data.lock().unwrap().clone()
        }

        fn read<T: serde::de::DeserializeOwned>(&self, key: &str) -> T {
            serde_json::from_str(&self.data.lock().unwrap()[key].0).unwrap()
        }
    }

    #[async_trait::async_trait]
    impl AgentKv for InviteKv {
        async fn get(&self, key: &str) -> Result<Option<(String, u64)>, RPCErrors> {
            if self.failed_read.as_deref() == Some(key) {
                return Err(RPCErrors::ReasonError("read unavailable".to_string()));
            }
            Ok(self.data.lock().unwrap().get(key).cloned())
        }

        async fn list(&self, _key: &str) -> Result<Vec<String>, RPCErrors> {
            unreachable!()
        }

        async fn exec_tx(
            &self,
            actions: HashMap<String, KVAction>,
            main_key: Option<(String, u64)>,
        ) -> Result<(), RPCErrors> {
            self.tx_calls.fetch_add(1, Ordering::SeqCst);
            *self.guard.lock().unwrap() = main_key.clone();
            let mut data = self.data.lock().unwrap();
            if let Some((key, value)) = self.before_commit.lock().unwrap().take() {
                let version = data.get(&key).map_or(0, |(_, version)| *version) + 1;
                data.insert(key, (value, version));
            }
            if let Some((key, expected)) = main_key {
                if data.get(&key).map_or(0, |(_, version)| *version) != expected {
                    return Err(RPCErrors::ReasonError("revision mismatch".to_string()));
                }
            }
            for (key, action) in &actions {
                if matches!(action, KVAction::Create(_)) && data.contains_key(key) {
                    return Err(RPCErrors::ReasonError(format!("{key} exists")));
                }
            }
            for (key, action) in actions {
                let version = data.get(&key).map_or(0, |(_, version)| *version) + 1;
                match action {
                    KVAction::Create(value) | KVAction::Update(value) => {
                        data.insert(key, (value, version));
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        }
    }

    fn invitation_fixture() -> (UserInviteRecord, OwnerDocument) {
        let owner = generated_owner_config("alice", "Alice", &DID::new("bns", "home"), false)
            .unwrap()
            .0;
        let invite = UserInviteRecord {
            invite_id: "invite-alice".to_string(),
            created_by: "admin".to_string(),
            created_at: 1,
            expires_at: Some(100),
            state: "pending".to_string(),
            target_user_id: None,
            target_did: Some(owner.id.to_string()),
            show_name: Some("Invited Alice".to_string()),
            default_user_type: UserType::User,
            groups: vec!["family".to_string()],
            accepted_at: None,
            accepted_user_id: None,
        };
        (invite, owner)
    }

    #[tokio::test]
    async fn invite_accept_creates_all_records_and_consumes_invite_in_one_cas() {
        let (invite, owner) = invitation_fixture();
        let kv = InviteKv::default();
        let path = format!("{USER_INVITE_PREFIX}/{}", invite.invite_id);
        kv.put(&path, &invite);
        let password = STANDARD.encode([7u8; 32]);
        let (user_id, accepted) =
            accept_user_invitation(&kv, &invite.invite_id, &owner, &password, 50)
                .await
                .unwrap();

        assert_eq!(user_id, "alice");
        assert_eq!(kv.tx_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*kv.guard.lock().unwrap(), Some((path.clone(), 1)));
        assert_eq!(kv.snapshot().len(), 4);
        let settings: UserSettings = kv.read("users/alice/settings");
        assert!(matches!(settings.state, UserState::Active));
        assert_eq!(settings.password, password);
        assert!(settings.password_state.is_set());
        assert!(!settings.is_local);
        assert_eq!(settings.groups, invite.groups);
        let document: OwnerDocument = kv.read("users/alice/doc");
        assert_eq!(document.id, owner.id);
        let profile: UserPrivateProfile = kv.read("users/alice/profile");
        assert_eq!(profile.did, owner.id);
        assert_eq!(profile.display_name.as_deref(), Some("Invited Alice"));
        assert_eq!(accepted.state, "accepted");
        assert_eq!(accepted.accepted_at, Some(50));
        let saved: UserInviteRecord = kv.read(&path);
        assert_eq!(saved.accepted_user_id.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn invite_accept_activates_only_its_own_reserved_pending_account() {
        let (mut invite, owner) = invitation_fixture();
        invite.target_user_id = Some("reserved".to_string());
        let kv = InviteKv::default();
        kv.put(
            &format!("{USER_INVITE_PREFIX}/{}", invite.invite_id),
            &invite,
        );
        let mut pending = account(UserType::Limited, None, PasswordState::Unset);
        pending.user_id = "reserved".to_string();
        pending.state = UserState::Pending;
        pending.password.clear();
        kv.put("users/reserved/settings", &pending);
        kv.put("users/reserved/profile", &profile_from_user_id("reserved"));

        let (user_id, _) = accept_user_invitation(&kv, &invite.invite_id, &owner, "", 50)
            .await
            .unwrap();
        assert_eq!(user_id, "reserved");
        assert_eq!(kv.snapshot().len(), 4);
        let settings: UserSettings = kv.read("users/reserved/settings");
        assert!(matches!(settings.state, UserState::Active));
        assert_eq!(settings.user_type, UserType::User);
        assert!(!settings.password_state.is_set());
        assert_eq!(kv.read::<OwnerDocument>("users/reserved/doc").id, owner.id);
    }

    #[tokio::test]
    async fn invite_accept_cannot_claim_another_reservation_or_existing_account() {
        for reserved_by_this_invite in [false, true] {
            let (mut invite, owner) = invitation_fixture();
            if reserved_by_this_invite {
                invite.target_user_id = Some("alice".to_string());
            }
            let kv = InviteKv::default();
            kv.put(
                &format!("{USER_INVITE_PREFIX}/{}", invite.invite_id),
                &invite,
            );
            let mut settings = account(UserType::User, None, PasswordState::Unset);
            settings.user_id = "alice".to_string();
            settings.state = if reserved_by_this_invite {
                UserState::Active
            } else {
                UserState::Pending
            };
            kv.put("users/alice/settings", &settings);
            let before = kv.snapshot();

            assert!(
                accept_user_invitation(&kv, &invite.invite_id, &owner, "", 50)
                    .await
                    .is_err()
            );
            assert_eq!(kv.snapshot(), before);
            assert_eq!(kv.tx_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn invite_accept_preserves_all_records_when_a_read_fails() {
        let (invite, owner) = invitation_fixture();
        let path = format!("{USER_INVITE_PREFIX}/{}", invite.invite_id);
        for key in [&path, "users/alice/settings"] {
            let kv = InviteKv {
                failed_read: Some(key.to_string()),
                ..Default::default()
            };
            kv.put(&path, &invite);
            kv.put(
                "users/alice/settings",
                &account(UserType::Admin, None, PasswordState::Set),
            );
            let before = kv.snapshot();

            let error = accept_user_invitation(&kv, &invite.invite_id, &owner, "", 50)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("read unavailable"));
            assert_eq!(kv.snapshot(), before);
            assert_eq!(kv.tx_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn invite_accept_rejects_wrong_did_expiry_replay_and_bad_password_before_writing() {
        for condition in ["wrong_did", "expired", "accepted", "bad_password"] {
            let (mut invite, owner) = invitation_fixture();
            let password = match condition {
                "wrong_did" => {
                    invite.target_did = Some("did:bns:bob".to_string());
                    ""
                }
                "expired" => {
                    invite.expires_at = Some(49);
                    ""
                }
                "accepted" => {
                    invite.state = "accepted".to_string();
                    ""
                }
                _ => "not-a-password-hash",
            };
            let kv = InviteKv::default();
            kv.put(
                &format!("{USER_INVITE_PREFIX}/{}", invite.invite_id),
                &invite,
            );
            let before = kv.snapshot();

            assert!(
                accept_user_invitation(&kv, &invite.invite_id, &owner, password, 50)
                    .await
                    .is_err(),
                "{condition}"
            );
            assert_eq!(kv.snapshot(), before, "{condition}");
            assert_eq!(kv.tx_calls.load(Ordering::SeqCst), 0, "{condition}");
        }
    }

    #[tokio::test]
    async fn invite_accept_record_collision_rolls_back_every_invitation_write() {
        for suffix in ["settings", "doc", "profile"] {
            let (invite, owner) = invitation_fixture();
            let path = format!("{USER_INVITE_PREFIX}/{}", invite.invite_id);
            let collision_key = format!("users/alice/{suffix}");
            let collision_value = "concurrently created record".to_string();
            let kv = InviteKv {
                before_commit: Mutex::new(Some((collision_key.clone(), collision_value.clone()))),
                ..Default::default()
            };
            kv.put(&path, &invite);
            let mut expected = kv.snapshot();
            expected.insert(collision_key, (collision_value, 1));

            assert!(
                accept_user_invitation(&kv, &invite.invite_id, &owner, "", 50)
                    .await
                    .is_err(),
                "{suffix}"
            );
            assert_eq!(kv.snapshot(), expected, "{suffix}");
            assert_eq!(kv.read::<UserInviteRecord>(&path).state, "pending");
        }
    }

    #[tokio::test]
    async fn invite_accept_cas_rejects_an_invitation_consumed_after_reading() {
        let (invite, mut owner) = invitation_fixture();
        owner.name = "second-account".to_string();
        let path = format!("{USER_INVITE_PREFIX}/{}", invite.invite_id);
        let mut consumed = invite.clone();
        consumed.state = "accepted".to_string();
        consumed.accepted_user_id = Some("first-account".to_string());
        let kv = InviteKv {
            before_commit: Mutex::new(Some((path.clone(), to_json(&consumed).unwrap()))),
            ..Default::default()
        };
        kv.put(&path, &invite);

        let error = accept_user_invitation(&kv, &invite.invite_id, &owner, "", 50)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("revision mismatch"));
        assert_eq!(kv.snapshot().len(), 1);
        assert_eq!(
            kv.read::<UserInviteRecord>(&path)
                .accepted_user_id
                .as_deref(),
            Some("first-account")
        );
        assert_eq!(*kv.guard.lock().unwrap(), Some((path, 1)));
    }

    #[test]
    fn local_profile_fields_win_over_did_document_defaults() {
        let merged = merge_profile_values(
            Some(
                json!({ "did": "did:bns:alice", "avatar": "data:image/png;base64,LOCAL", "bio": " " }),
            ),
            Some(
                json!({ "avatar": "https://doc/a.png", "bio": "From doc", "display_name": "Alice" }),
            ),
        );
        assert_eq!(merged["avatar"], "data:image/png;base64,LOCAL");
        assert_eq!(merged["bio"], "From doc");
        assert_eq!(merged["display_name"], "Alice");
        assert_eq!(merged["did"], "did:bns:alice");

        let cleared = merge_profile_values(
            Some(json!({ "did": "did:bns:alice" })),
            Some(json!({ "avatar": "https://doc/a.png" })),
        );
        assert_eq!(cleared["avatar"], "https://doc/a.png");
    }

    #[test]
    fn profile_patch_distinguishes_keep_set_and_clear() {
        let mut profile = profile_from_user_id("alice");
        profile.bio = Some("Old bio".to_string());
        profile.avatar = Some("https://old/avatar.png".to_string());
        let avatar = png_data_url(96, 96);
        let req = RPCRequest::new(
            "user.profile.set",
            json!({ "avatar": avatar, "display_name": " Alice ", "website": "https://a.example" }),
        );
        apply_profile_text_patch(&mut profile, &req).unwrap();
        assert_eq!(profile.avatar.as_deref(), Some(avatar.as_str()));
        assert_eq!(profile.display_name.as_deref(), Some("Alice"));
        assert_eq!(profile.bio.as_deref(), Some("Old bio"));
        assert_eq!(profile.links["website"].url, "https://a.example");

        let req = RPCRequest::new(
            "user.profile.set",
            json!({ "avatar": null, "bio": "", "website": null }),
        );
        apply_profile_text_patch(&mut profile, &req).unwrap();
        assert_eq!(profile.avatar, None);
        assert_eq!(profile.bio, None);
        assert!(!profile.links.contains_key("website"));
        assert_eq!(profile.display_name.as_deref(), Some("Alice"));

        let req = RPCRequest::new(
            "user.profile.set",
            json!({ "avatar": "https://example.com/a.png" }),
        );
        assert!(apply_profile_text_patch(&mut profile, &req).is_err());
        let req = RPCRequest::new("user.profile.set", json!({ "bio": 1 }));
        assert!(apply_profile_text_patch(&mut profile, &req).is_err());
    }

    #[test]
    fn only_assignable_user_types_are_accepted() {
        assert_eq!(parse_user_type("Admin").unwrap(), UserType::Admin);
        assert_eq!(parse_user_type("user").unwrap(), UserType::User);
        assert_eq!(parse_user_type(" limited ").unwrap(), UserType::Limited);
        for raw in ["root", "guest", "external", ""] {
            assert!(parse_user_type(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn account_options_report_overrides_and_effective_values() {
        let mut settings = UserSettings {
            user_id: "carol".to_string(),
            user_type: UserType::Limited,
            password: String::new(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: None,
            allow_contact_edit: None,
            groups: vec!["customer".to_string()],
            password_state: buckyos_api::PasswordState::Set,
        };
        let value = settings_options_value(&settings);
        assert_eq!(value["groups"], json!(["customer"]));
        assert_eq!(value["allow_contact_edit"], Value::Null);
        assert_eq!(value["can_edit_contacts"], false);
        assert_eq!(value["can_change_password"], false);
        settings.allow_contact_edit = Some(true);
        assert_eq!(settings_options_value(&settings)["can_edit_contacts"], true);
        assert!(default_contact_settings(None).tags.is_empty());
    }

    #[test]
    fn invitations_bring_in_user_or_limited_accounts_only() {
        assert_eq!(invite_user_type(None).unwrap(), UserType::User);
        assert_eq!(
            invite_user_type(Some("limited")).unwrap(),
            UserType::Limited
        );
        for raw in ["admin", "Admin", "root"] {
            assert!(
                matches!(
                    invite_user_type(Some(raw)),
                    Err(RPCErrors::ParseRequestError(_))
                ),
                "{raw}"
            );
        }
    }

    fn caller(user: &str, user_type: UserType, sudo: bool) -> RpcAuthPrincipal {
        RpcAuthPrincipal {
            username: user.to_string(),
            owner_user_id: user.to_string(),
            authenticated_app_id: "control-panel".to_string(),
            user_type,
            owner_did: String::new(),
            is_user_session: true,
            is_control_panel_session: true,
            agent_permission_mode: None,
            session: crate::PrincipalSession {
                ceiling: buckyos_api::SessionCeiling::for_user_type(user_type),
                is_sudo: sudo,
                ..Default::default()
            },
        }
    }

    #[test]
    fn invite_accept_requires_the_document_owners_federated_user_session() {
        let did = DID::new("bns", "alice");
        assert!(require_invite_owner_session(None, &did).is_err());
        let mut principal = caller("alice", UserType::User, false);
        principal.owner_did = did.to_string();
        for method in [
            None,
            Some(buckyos_api::AuthMethod::Password),
            Some(buckyos_api::AuthMethod::ExclusiveInvite),
            Some(buckyos_api::AuthMethod::Assertion),
        ] {
            principal.session.auth_method = method;
            assert!(require_invite_owner_session(Some(&principal), &did).is_err());
        }
        principal.session.auth_method = Some(buckyos_api::AuthMethod::DidFederation);
        assert!(require_invite_owner_session(Some(&principal), &did).is_ok());
        assert!(require_invite_owner_session(Some(&principal), &DID::new("bns", "bob")).is_err());
        principal.session.is_external = true;
        principal.session.ceiling = buckyos_api::SessionCeiling::external();
        assert!(require_invite_owner_session(Some(&principal), &did).is_ok());
        principal.is_control_panel_session = false;
        assert!(require_invite_owner_session(Some(&principal), &did).is_err());
        principal.is_control_panel_session = true;
        principal.is_user_session = false;
        assert!(require_invite_owner_session(Some(&principal), &did).is_err());
    }

    fn account(user_type: UserType, allow: Option<bool>, state: PasswordState) -> UserSettings {
        UserSettings {
            user_id: "carol".to_string(),
            user_type,
            password: "x".to_string(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: allow,
            allow_contact_edit: None,
            groups: Vec::new(),
            password_state: state,
        }
    }

    #[test]
    fn password_change_needs_a_control_panel_session_the_override_and_sudo() {
        let set = PasswordState::Set;
        let carol = |user_type| caller("carol", user_type, false);
        // Limited: refused by default, allowed by the override, without sudo.
        assert!(matches!(
            password_change_allowed(
                &carol(UserType::Limited),
                "carol",
                &account(UserType::Limited, None, set)
            ),
            Err(RPCErrors::NoPermission(_))
        ));
        assert!(password_change_allowed(
            &carol(UserType::Limited),
            "carol",
            &account(UserType::Limited, Some(true), set)
        )
        .is_ok());
        // A User re-proves the password with sudo.
        assert!(matches!(
            password_change_allowed(
                &carol(UserType::User),
                "carol",
                &account(UserType::User, None, set)
            ),
            Err(RPCErrors::NoPermission(_))
        ));
        assert!(password_change_allowed(
            &caller("carol", UserType::User, true),
            "carol",
            &account(UserType::User, None, set)
        )
        .is_ok());
        // Never from a session of another target (an App holding the user's session).
        let mut app_session = caller("carol", UserType::User, true);
        app_session.is_control_panel_session = false;
        let error =
            password_change_allowed(&app_session, "carol", &account(UserType::User, None, set))
                .unwrap_err();
        assert_eq!(
            AccessDenial::from_rpc_error(&error).map(|denial| denial.reason),
            Some(DenyReason::SessionKindNotAllowed)
        );
        assert!(matches!(
            password_change_allowed(
                &carol(UserType::User),
                "carol",
                &account(UserType::User, Some(false), set)
            ),
            Err(RPCErrors::NoPermission(_))
        ));
        // The first password is not a change.
        let error = password_change_allowed(
            &carol(UserType::Limited),
            "carol",
            &account(UserType::Limited, Some(true), PasswordState::Unset),
        )
        .unwrap_err();
        assert_eq!(
            AccessDenial::from_rpc_error(&error).map(|denial| denial.reason),
            Some(DenyReason::CredentialState)
        );
        // Another account: an admin with a sudo session only.
        let target = account(UserType::Limited, None, set);
        assert!(password_change_allowed(
            &caller("devtest", UserType::Admin, true),
            "carol",
            &target
        )
        .is_ok());
        assert!(matches!(
            password_change_allowed(&caller("devtest", UserType::Admin, false), "carol", &target),
            Err(RPCErrors::NoPermission(_))
        ));
        assert!(matches!(
            password_change_allowed(&caller("bob", UserType::User, true), "carol", &target),
            Err(RPCErrors::NoPermission(_))
        ));
    }

    #[test]
    fn password_hash_must_be_a_sha256_digest() {
        assert!(validate_password_hash("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").is_ok());
        assert!(validate_password_hash("").is_err());
        assert!(validate_password_hash("not-base64").is_err());
        assert!(validate_password_hash("YQ==").is_err());
    }

    #[test]
    fn committed_create_reports_pending_rbac_refresh_as_success() {
        let result = user_create_result(
            "alice",
            &UserType::User,
            Err(RPCErrors::ReasonError("scheduler unavailable".to_string())),
        );
        assert_eq!(result["ok"], true);
        assert_eq!(result["created"], true);
        assert_eq!(result["rbac_refreshed"], false);
        assert!(result["warning"]
            .as_str()
            .unwrap()
            .contains("RBAC refresh is pending"));
    }
}
