//! Exclusive invites (SSO-06, contract §5): an administrator pre-creates a
//! Limited account and a password-free link to one resource. The visitor signs
//! in with the link until it expires, asks the issuer for a new one, or sets a
//! password. Records live at `services/control_panel/exclusive_invites/<id>`
//! and are written with the Control Panel service identity.

use crate::agent_mgr::{AgentKv, AgentNameIndex, ZoneKv};
use crate::user_mgr::{
    after_account_created, new_account_actions, require_control_panel_admin,
    validate_password_hash, validate_username,
};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{kRPC, RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buckyos_api::{
    constant_time_eq, exclusive_invite_key, get_buckyos_api_runtime, invite_secret_digest,
    user_did_from_doc_value, AccessDenial, AuthMethod, AuthTarget, DenyReason, ExclusiveInvite,
    ExclusiveInviteLoginRequest, ExclusiveInviteLoginResult, InviteChannel, InviteCredential,
    InviteCredentialState, InviteIssuer, InviteLoginRefusal, InviteResend, InviteResource,
    InviteResourceState, InviteState, NotifySendReq, PasswordState, RevokeFamiliesRequest,
    SystemServiceId, UserPrivateProfile, UserSettings, UserState, UserType,
    CONTROL_PANEL_SERVICE_UNIQUE_ID, EXCLUSIVE_INVITE_PREFIX,
    EXCLUSIVE_INVITE_RESEND_INTERVAL_SECS, EXCLUSIVE_INVITE_TTL_SECS, MSG_CENTER_SERVICE_NAME,
};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use log::{info, warn};
use name_lib::DID;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

const INVITE_ID_PREFIX: &str = "ei-";
const MAX_RETURN_PATH_LEN: usize = 2048;

pub(crate) fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

fn new_invite_id() -> String {
    let mut buffer = [0u8; 8];
    OsRng.fill_bytes(&mut buffer);
    format!("{INVITE_ID_PREFIX}{}", hex::encode(buffer))
}

fn is_invite_id(invite_id: &str) -> bool {
    invite_id
        .strip_prefix(INVITE_ID_PREFIX)
        .is_some_and(|rest| {
            (1..=48).contains(&rest.len())
                && rest
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        })
}

fn require_invite_id(invite_id: &str) -> Result<(), RPCErrors> {
    if is_invite_id(invite_id) {
        Ok(())
    } else {
        Err(RPCErrors::ParseRequestError(format!(
            "invalid invite_id `{invite_id}`"
        )))
    }
}

/// A Zone-relative page path: one leading `/`, no scheme or authority.
fn validate_return_path(path: &str) -> Result<(), RPCErrors> {
    let valid = path.starts_with('/')
        && !path.starts_with("//")
        && path.len() <= MAX_RETURN_PATH_LEN
        && !path.contains('\\')
        && !path.chars().any(char::is_control);
    if valid {
        Ok(())
    } else {
        Err(RPCErrors::ParseRequestError(
            "return_path must be a path on this Zone".to_string(),
        ))
    }
}

fn default_return_path(resource: Option<&InviteResource>) -> String {
    match resource {
        Some(InviteResource::GroupSession {
            group_did,
            session_id,
            ..
        }) => {
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("group", group_did)
                .append_pair("session", session_id)
                .finish();
            format!("/messagehub/guest?{query}")
        }
        _ => "/".to_string(),
    }
}

fn resource_title(resource: Option<&InviteResource>) -> Option<String> {
    let title = match resource? {
        InviteResource::GroupSession { title, .. } | InviteResource::Workspace { title, .. } => {
            title.trim()
        }
    };
    (!title.is_empty()).then(|| title.to_string())
}

/// A fresh password-free credential and the secret only the link carries.
fn issue_credential(version: u32, now: u64) -> (InviteCredential, String) {
    let secret = random_token(32);
    let credential = InviteCredential {
        version,
        digest: invite_secret_digest(&secret),
        issued_at: now,
        expires_at: now + EXCLUSIVE_INVITE_TTL_SECS,
        state: InviteCredentialState::Active,
    };
    (credential, secret)
}

fn invite_link(origin: &str, invite_id: &str, secret: &str) -> String {
    format!("{origin}/sso/invite/{invite_id}#s={secret}")
}

/// The record as administrators see it: never the secret, nor its digest.
fn invite_view(invite: &ExclusiveInvite) -> Value {
    let mut value = json!(invite);
    if let Some(credential) = value.get_mut("credential").and_then(Value::as_object_mut) {
        credential.remove("digest");
    }
    value
}

fn to_json<T: Serialize>(value: &T) -> Result<String, RPCErrors> {
    serde_json::to_string(value)
        .map_err(|error| RPCErrors::ReasonError(format!("serialize failed: {error}")))
}

async fn get_json<T: serde::de::DeserializeOwned>(
    kv: &dyn AgentKv,
    key: &str,
) -> Result<Option<(T, u64)>, RPCErrors> {
    match kv.get(key).await? {
        Some((raw, version)) => serde_json::from_str(&raw)
            .map(|value| Some((value, version)))
            .map_err(|error| RPCErrors::ReasonError(format!("invalid `{key}`: {error}"))),
        None => Ok(None),
    }
}

pub(crate) async fn load_invite(
    kv: &dyn AgentKv,
    invite_id: &str,
) -> Result<Option<(ExclusiveInvite, u64)>, RPCErrors> {
    if !is_invite_id(invite_id) {
        return Ok(None);
    }
    get_json(kv, &exclusive_invite_key(invite_id)).await
}

pub(crate) async fn load_account(
    kv: &dyn AgentKv,
    user_id: &str,
) -> Result<Option<(UserSettings, u64)>, RPCErrors> {
    get_json(kv, &format!("users/{user_id}/settings")).await
}

async fn list_invites(
    kv: &dyn AgentKv,
    user_id: Option<&str>,
) -> Result<Vec<ExclusiveInvite>, RPCErrors> {
    let mut invites = Vec::new();
    for invite_id in kv.list(EXCLUSIVE_INVITE_PREFIX).await? {
        match load_invite(kv, &invite_id).await {
            Ok(Some((invite, _))) => {
                if user_id.map_or(true, |user_id| invite.user_id == user_id) {
                    invites.push(invite);
                }
            }
            Ok(None) => {}
            Err(error) => warn!("skip unreadable exclusive invite {invite_id}: {error}"),
        }
    }
    invites.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(invites)
}

/// Writes `invite` if its record is still at `version`.
async fn save_invite(
    kv: &dyn AgentKv,
    invite: &ExclusiveInvite,
    version: u64,
) -> Result<(), RPCErrors> {
    let key = exclusive_invite_key(&invite.invite_id);
    kv.exec_tx(
        HashMap::from([(key.clone(), KVAction::Update(to_json(invite)?))]),
        Some((key, version)),
    )
    .await
}

// ─── decisions ─────────────────────────────────────────────────────────────

/// Contract §5.3 in order, with the account read next to the invite: the same
/// answer verify-hub gives, for the page to show before signing in.
pub(crate) fn invite_login_check(
    invite: Option<&ExclusiveInvite>,
    account: Option<&UserSettings>,
    digest: &str,
    now: u64,
) -> Result<(), InviteLoginRefusal> {
    let invite = invite.ok_or(InviteLoginRefusal::Revoked)?;
    if invite.state != InviteState::Active {
        return Err(InviteLoginRefusal::Revoked);
    }
    let account = account
        .filter(|account| account.is_active())
        .ok_or(InviteLoginRefusal::AccountInactive)?;
    if account.password_state.is_set() {
        return Err(InviteLoginRefusal::PasswordRequired);
    }
    invite.check_credential(digest, now)
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ResendDecision {
    Submitted,
    RateLimited,
    NotApplicable,
}

/// D-9: a visitor whose link expired may ask the issuer for a new one, at most
/// once per interval. Revoked invites, accounts with a password and links that
/// were already replaced get nothing.
pub(crate) fn resend_decision(
    invite: Option<&ExclusiveInvite>,
    account: Option<&UserSettings>,
    now: u64,
) -> ResendDecision {
    let Some(invite) = invite else {
        return ResendDecision::NotApplicable;
    };
    let account_waits = account
        .map(|account| account.is_active() && !account.password_state.is_set())
        .unwrap_or(false);
    if invite.state != InviteState::Active
        || !matches!(invite.issuer, InviteIssuer::User { .. })
        || !account_waits
        || invite.credential.state != InviteCredentialState::Active
        || now <= invite.credential.expires_at
    {
        return ResendDecision::NotApplicable;
    }
    match &invite.resend {
        Some(resend) if now < resend.requested_at + EXCLUSIVE_INVITE_RESEND_INTERVAL_SECS => {
            ResendDecision::RateLimited
        }
        _ => ResendDecision::Submitted,
    }
}

/// Rotation hands out a new link: not for a revoked invite or an account that
/// already has a password (the link then only leads to the password sign-in).
fn rotate_check(
    invite: &ExclusiveInvite,
    account: Option<&UserSettings>,
) -> Result<(), AccessDenial> {
    if invite.state != InviteState::Active {
        return Err(AccessDenial::new(DenyReason::SourceRevoked).with_detail("revoked"));
    }
    match account {
        None => Err(AccessDenial::new(DenyReason::AccountState)),
        Some(account) if account.password_state.is_set() => {
            Err(AccessDenial::new(DenyReason::CredentialState).with_detail("password_set"))
        }
        Some(_) => Ok(()),
    }
}

/// The next credential version; the previous secret stops matching at once.
fn rotated(invite: &ExclusiveInvite, now: u64) -> (ExclusiveInvite, String) {
    let (credential, secret) = issue_credential(invite.credential.version + 1, now);
    let mut next = invite.clone();
    next.credential = credential;
    next.resend = None;
    next.updated_at = now;
    (next, secret)
}

/// P-1: the first password is the account's own act, only while it has none.
fn first_password_check(
    principal: &RpcAuthPrincipal,
    account: &UserSettings,
) -> Result<(), AccessDenial> {
    if principal.session.is_external || principal.username != account.user_id {
        return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed));
    }
    if account.password_state.is_set() {
        return Err(AccessDenial::new(DenyReason::CredentialState).with_detail("password_set"));
    }
    Ok(())
}

/// Every still-usable password-free credential of the account becomes
/// `consumed_by_password`; committed with the password itself. Only those two
/// fields are written, so a revoke that lands meanwhile is never undone.
pub(crate) async fn consume_invite_actions(
    kv: &dyn AgentKv,
    user_id: &str,
    now: u64,
) -> Result<HashMap<String, KVAction>, RPCErrors> {
    let consumed = serde_json::to_value(InviteCredentialState::ConsumedByPassword)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let mut actions = HashMap::new();
    for invite in list_invites(kv, Some(user_id)).await? {
        if invite.credential.state != InviteCredentialState::Active {
            continue;
        }
        actions.insert(
            exclusive_invite_key(&invite.invite_id),
            KVAction::SetByJsonPath(HashMap::from([
                ("credential/state".to_string(), Some(consumed.clone())),
                ("updated_at".to_string(), Some(Value::from(now))),
            ])),
        );
    }
    Ok(actions)
}

/// Ends the account's families that a password-free link established, except
/// the caller's own; a failure leaves them to expire (logged, not raised).
pub(crate) async fn revoke_invite_families(user_id: &str, except_session_token: Option<String>) {
    let request = RevokeFamiliesRequest {
        subject: user_id.to_string(),
        auth_method: Some(AuthMethod::ExclusiveInvite),
        source_invite_id: None,
        except_session_token,
    };
    revoke_families(request).await;
}

/// Revocation is what ends the other password-free sessions, so a failed call
/// is retried a few times before it is left logged.
async fn revoke_families(request: RevokeFamiliesRequest) -> Option<u32> {
    const ATTEMPTS: u32 = 3;
    let subject = request.subject.clone();
    let mut attempt = 0;
    let result = loop {
        attempt += 1;
        let result = async {
            get_buckyos_api_runtime()?
                .get_verify_hub_client()
                .await?
                .revoke_families(request.clone())
                .await
        }
        .await;
        if result.is_ok() || attempt >= ATTEMPTS {
            break result;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500 * u64::from(attempt))).await;
    };
    match result {
        Ok(result) => {
            info!("revoked {} session families of {subject}", result.revoked);
            Some(result.revoked)
        }
        Err(error) => {
            warn!("revoke session families of {subject} failed: {error}");
            None
        }
    }
}

// ─── creation ──────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewInviteUser {
    user_id: String,
    #[serde(default)]
    show_name: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateInviteParams {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    new_user: Option<NewInviteUser>,
    channel: InviteChannel,
    #[serde(default)]
    resource: Option<InviteResource>,
    #[serde(default)]
    return_path: Option<String>,
}

pub(crate) struct CreatedInvite {
    invite: ExclusiveInvite,
    secret: String,
    account_did: DID,
    new_account: bool,
}

/// Commits the account (when new) and the invite record in one transaction.
/// Reuse names an existing Limited account explicitly (Q-10); a new account is
/// Limited, has no password, no group and no canonical zone here.
pub(crate) async fn create_invite(
    kv: &dyn AgentKv,
    zone_did: &DID,
    issuer: &str,
    params: CreateInviteParams,
    now: u64,
) -> Result<CreatedInvite, RPCErrors> {
    if params.channel.kind.trim().is_empty() {
        return Err(RPCErrors::ParseRequestError(
            "channel.kind is required".to_string(),
        ));
    }
    let return_path = match params.return_path {
        Some(path) => {
            validate_return_path(&path)?;
            path
        }
        None => default_return_path(params.resource.as_ref()),
    };
    let mut actions = HashMap::new();
    let (user_id, account_did, new_account) = match (params.user_id, params.new_user) {
        (Some(user_id), None) => {
            let user_id = user_id.trim().to_lowercase();
            let (settings, _) = load_account(kv, &user_id).await?.ok_or_else(|| {
                RPCErrors::ParseRequestError(format!("account `{user_id}` does not exist"))
            })?;
            if settings.user_type != UserType::Limited
                || matches!(settings.state, UserState::Deleted)
            {
                return Err(RPCErrors::ParseRequestError(format!(
                    "exclusive invites reuse Limited accounts only; `{user_id}` is not one"
                )));
            }
            let doc_key = format!("users/{user_id}/doc");
            let (raw_doc, _) = kv.get(&doc_key).await?.ok_or_else(|| {
                RPCErrors::ReasonError(format!("account `{user_id}` has no DID document"))
            })?;
            (user_id, user_did_from_doc_value(&doc_key, &raw_doc)?, false)
        }
        (None, Some(new_user)) => {
            let user_id = new_user.user_id.trim().to_lowercase();
            validate_username(&user_id)?;
            AgentNameIndex::load(kv, zone_did)
                .await?
                .check(&user_id)
                .map_err(|conflict| {
                    RPCErrors::ParseRequestError(format!(
                        "`{user_id}` is not available: {}",
                        conflict.message
                    ))
                })?;
            let settings = UserSettings {
                user_id: user_id.clone(),
                user_type: UserType::Limited,
                password: String::new(),
                state: UserState::Active,
                res_pool_id: "default".to_string(),
                is_local: true,
                allow_password_change: None,
                allow_contact_edit: None,
                groups: Vec::new(),
                password_state: PasswordState::Unset,
            };
            let show_name = new_user
                .show_name
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| user_id.clone());
            let (account_actions, did) =
                new_account_actions(&settings, &show_name, zone_did, false)?;
            actions.extend(account_actions);
            (user_id, did, true)
        }
        _ => {
            return Err(RPCErrors::ParseRequestError(
                "name exactly one of user_id and new_user".to_string(),
            ))
        }
    };
    let invite_id = new_invite_id();
    let (credential, secret) = issue_credential(1, now);
    let resource_state = match params.resource {
        Some(InviteResource::GroupSession { .. }) => InviteResourceState::Pending,
        _ => InviteResourceState::None,
    };
    let invite = ExclusiveInvite {
        invite_id: invite_id.clone(),
        user_id,
        issuer: InviteIssuer::User {
            user_id: issuer.to_string(),
        },
        channel: params.channel,
        resource: params.resource,
        resource_state,
        return_path,
        state: InviteState::Active,
        credential,
        resend: None,
        created_at: now,
        updated_at: now,
    };
    actions.insert(
        exclusive_invite_key(&invite_id),
        KVAction::Create(to_json(&invite)?),
    );
    kv.exec_tx(actions, None).await?;
    Ok(CreatedInvite {
        invite,
        secret,
        account_did,
        new_account,
    })
}

/// Invites the account's DID into the Session as a guest, as the administrator
/// (msg-center checks the administrator's right to do so); `invite_id` keeps a
/// retry from inviting twice.
async fn invite_to_resource(
    caller_token: &str,
    invite: &ExclusiveInvite,
    member_did: &DID,
) -> Result<(), RPCErrors> {
    let Some(InviteResource::GroupSession {
        group_did,
        session_id,
        ..
    }) = &invite.resource
    else {
        return Ok(());
    };
    let url = get_buckyos_api_runtime()?
        .get_zone_service_url(MSG_CENTER_SERVICE_NAME, false)
        .await?;
    kRPC::new(&url, Some(caller_token.to_string()))
        .call(
            "group.invite_session_guest",
            json!({
                "group_did": group_did,
                "session_id": session_id,
                "member_did": member_did.to_string(),
                "idempotency_key": invite.invite_id,
            }),
        )
        .await
        .map(|_| ())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteIdParams {
    invite_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteListParams {
    #[serde(default)]
    user_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteSecretParams {
    invite_id: String,
    secret: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPasswordParams {
    new_password_hash: String,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum InviteDescribeResult {
    Active {
        user_id: String,
        show_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        resource_title: Option<String>,
        return_path: String,
    },
    /// `user_id` / `return_path` only for the holder of the current secret, so the
    /// page can offer the password sign-in back to the resource.
    Denied {
        denial: AccessDenial,
        #[serde(skip_serializing_if = "Option::is_none")]
        user_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        return_path: Option<String>,
    },
}

/// What `auth.invite.describe` tells the holder of `digest`.
fn describe_outcome(
    invite: Option<&ExclusiveInvite>,
    account: Option<&UserSettings>,
    show_name: Option<String>,
    digest: &str,
    now: u64,
) -> InviteDescribeResult {
    match (invite_login_check(invite, account, digest, now), invite) {
        (Ok(()), Some(invite)) => InviteDescribeResult::Active {
            user_id: invite.user_id.clone(),
            show_name: show_name.unwrap_or_else(|| invite.user_id.clone()),
            resource_title: resource_title(invite.resource.as_ref()),
            return_path: invite.return_path.clone(),
        },
        (result, invite) => {
            let holder = invite.filter(|invite| {
                constant_time_eq(invite.credential.digest.as_bytes(), digest.as_bytes())
            });
            InviteDescribeResult::Denied {
                denial: result.err().unwrap_or(InviteLoginRefusal::Revoked).denial(),
                user_id: holder.map(|invite| invite.user_id.clone()),
                return_path: holder.map(|invite| invite.return_path.clone()),
            }
        }
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum InviteLoginResult {
    Ready { callback_url: String },
    Denied { denial: AccessDenial },
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SetPasswordResult {
    Ok,
    Denied { denial: AccessDenial },
}

fn success(value: impl Serialize, seq: u64) -> Result<RPCResponse, RPCErrors> {
    Ok(RPCResponse::new(RPCResult::Success(json!(value)), seq))
}

fn caller_token(req: &RPCRequest) -> Result<String, RPCErrors> {
    ControlPanelServer::extract_rpc_session_token(req)
        .ok_or_else(|| RPCErrors::InvalidToken("missing session token".to_string()))
}

impl ControlPanelServer {
    async fn invite_resource_step(
        &self,
        kv: &dyn AgentKv,
        caller_token: &str,
        invite: ExclusiveInvite,
        member_did: &DID,
    ) -> Result<ExclusiveInvite, RPCErrors> {
        if invite.resource_state != InviteResourceState::Pending {
            return Ok(invite);
        }
        if let Err(error) = invite_to_resource(caller_token, &invite, member_did).await {
            warn!(
                "exclusive invite {} resource invitation pending: {error}",
                invite.invite_id
            );
            return Ok(invite);
        }
        let Some((mut current, version)) = load_invite(kv, &invite.invite_id).await? else {
            return Ok(invite);
        };
        current.resource_state = InviteResourceState::Done;
        current.updated_at = buckyos_get_unix_timestamp();
        save_invite(kv, &current, version).await?;
        Ok(current)
    }

    async fn admin_invite(
        kv: &dyn AgentKv,
        req: &RPCRequest,
    ) -> Result<(ExclusiveInvite, u64), RPCErrors> {
        let params: InviteIdParams = Self::typed_params(req)?;
        require_invite_id(&params.invite_id)?;
        load_invite(kv, &params.invite_id).await?.ok_or_else(|| {
            RPCErrors::ParseRequestError(format!(
                "exclusive invite `{}` not found",
                params.invite_id
            ))
        })
    }

    async fn account_did(kv: &dyn AgentKv, user_id: &str) -> Result<DID, RPCErrors> {
        let key = format!("users/{user_id}/doc");
        let (raw, _) = kv
            .get(&key)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("`{key}` is missing")))?;
        user_did_from_doc_value(&key, &raw)
    }

    /// `user.exclusive_invite.create {user_id? | new_user: {user_id, show_name?},
    /// channel: {kind, target_hint?}, resource?, return_path?}` → `{invite, link}`.
    pub(crate) async fn handle_exclusive_invite_create(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_control_panel_admin(principal)?;
        let params: CreateInviteParams = Self::typed_params(&req)?;
        let caller_token = caller_token(&req)?;
        let runtime = get_buckyos_api_runtime()?;
        let zone_host = Self::sso_zone_host(runtime)?;
        let kv = ZoneKv::connect().await?;
        let created = create_invite(
            &kv,
            &runtime.zone_id,
            &principal.username,
            params,
            buckyos_get_unix_timestamp(),
        )
        .await?;
        if created.new_account {
            if let Err(error) =
                after_account_created(&created.invite.user_id, "user.exclusive_invite.create").await
            {
                warn!("RBAC refresh after exclusive invite account failed: {error}");
            }
        }
        info!(
            "exclusive invite {} for '{}' created by '{}'",
            created.invite.invite_id, created.invite.user_id, principal.username
        );
        let link = invite_link(
            &Self::zone_root_origin(&req, &zone_host),
            &created.invite.invite_id,
            &created.secret,
        );
        let invite = self
            .invite_resource_step(&kv, &caller_token, created.invite, &created.account_did)
            .await?;
        success(
            json!({"invite": invite_view(&invite), "link": link}),
            req.seq,
        )
    }

    /// `user.exclusive_invite.list {user_id?}` → `{invites: [invite]}`, newest first.
    pub(crate) async fn handle_exclusive_invite_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        require_control_panel_admin(Self::require_rpc_principal(principal)?)?;
        let params: InviteListParams = Self::typed_params(&req)?;
        let kv = ZoneKv::connect().await?;
        let items = list_invites(&kv, params.user_id.as_deref())
            .await?
            .iter()
            .map(invite_view)
            .collect::<Vec<_>>();
        success(json!({ "invites": items }), req.seq)
    }

    /// `user.exclusive_invite.get {invite_id}` → `{invite}`.
    pub(crate) async fn handle_exclusive_invite_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        require_control_panel_admin(Self::require_rpc_principal(principal)?)?;
        let kv = ZoneKv::connect().await?;
        let (invite, _) = Self::admin_invite(&kv, &req).await?;
        success(json!({ "invite": invite_view(&invite) }), req.seq)
    }

    /// `user.exclusive_invite.revoke {invite_id}` → `{invite, families_revoked}`;
    /// sessions established from this invite stop refreshing.
    pub(crate) async fn handle_exclusive_invite_revoke(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_control_panel_admin(principal)?;
        let kv = ZoneKv::connect().await?;
        let (mut invite, version) = Self::admin_invite(&kv, &req).await?;
        if invite.state != InviteState::Revoked {
            invite.state = InviteState::Revoked;
            invite.updated_at = buckyos_get_unix_timestamp();
            save_invite(&kv, &invite, version).await?;
        }
        let families_revoked = revoke_families(RevokeFamiliesRequest {
            subject: invite.user_id.clone(),
            auth_method: None,
            source_invite_id: Some(invite.invite_id.clone()),
            except_session_token: None,
        })
        .await;
        info!(
            "exclusive invite {} revoked by '{}'",
            invite.invite_id, principal.username
        );
        success(
            json!({"invite": invite_view(&invite), "families_revoked": families_revoked}),
            req.seq,
        )
    }

    /// `user.exclusive_invite.rotate {invite_id}` → `{invite, link}`: a new credential
    /// version; the link appears only in this response.
    pub(crate) async fn handle_exclusive_invite_rotate(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_control_panel_admin(principal)?;
        let runtime = get_buckyos_api_runtime()?;
        let zone_host = Self::sso_zone_host(runtime)?;
        let kv = ZoneKv::connect().await?;
        let (invite, version) = Self::admin_invite(&kv, &req).await?;
        let account = load_account(&kv, &invite.user_id).await?;
        rotate_check(&invite, account.as_ref().map(|(settings, _)| settings)).map_err(
            |denial| {
                denial
                    .at(
                        CONTROL_PANEL_SERVICE_UNIQUE_ID,
                        "user.exclusive_invite.rotate",
                    )
                    .into_rpc_error()
            },
        )?;
        let (next, secret) = rotated(&invite, buckyos_get_unix_timestamp());
        save_invite(&kv, &next, version).await?;
        info!(
            "exclusive invite {} rotated to version {} by '{}'",
            next.invite_id, next.credential.version, principal.username
        );
        let link = invite_link(
            &Self::zone_root_origin(&req, &zone_host),
            &next.invite_id,
            &secret,
        );
        success(json!({"invite": invite_view(&next), "link": link}), req.seq)
    }

    /// `user.exclusive_invite.retry_resource {invite_id}` → `{invite}`.
    pub(crate) async fn handle_exclusive_invite_retry_resource(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        require_control_panel_admin(Self::require_rpc_principal(principal)?)?;
        let caller_token = caller_token(&req)?;
        let kv = ZoneKv::connect().await?;
        let (invite, _) = Self::admin_invite(&kv, &req).await?;
        if invite.state != InviteState::Active {
            return Err(AccessDenial::new(DenyReason::SourceRevoked)
                .at(
                    CONTROL_PANEL_SERVICE_UNIQUE_ID,
                    "user.exclusive_invite.retry_resource",
                )
                .into_rpc_error());
        }
        let did = Self::account_did(&kv, &invite.user_id).await?;
        let invite = self
            .invite_resource_step(&kv, &caller_token, invite, &did)
            .await?;
        success(json!({ "invite": invite_view(&invite) }), req.seq)
    }

    /// `auth.invite.describe {invite_id, secret}` (public) →
    /// `{status:"active", user_id, show_name, resource_title?, return_path}` |
    /// `{status:"denied", denial, user_id?, return_path?}`.
    pub(crate) async fn handle_auth_invite_describe(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let params: InviteSecretParams = Self::typed_params(&req)?;
        let kv = ZoneKv::connect().await?;
        let invite = load_invite(&kv, &params.invite_id)
            .await?
            .map(|(invite, _)| invite);
        let account = match invite.as_ref() {
            Some(invite) => load_account(&kv, &invite.user_id).await?.map(|(a, _)| a),
            None => None,
        };
        let show_name = match invite.as_ref() {
            Some(invite) => {
                get_json::<UserPrivateProfile>(&kv, &format!("users/{}/profile", invite.user_id))
                    .await?
                    .and_then(|(profile, _)| profile.display_name.or(profile.name))
                    .filter(|name| !name.trim().is_empty())
            }
            None => None,
        };
        let result = describe_outcome(
            invite.as_ref(),
            account.as_ref(),
            show_name,
            &invite_secret_digest(&params.secret),
            buckyos_get_unix_timestamp(),
        );
        success(result, req.seq)
    }

    /// `auth.invite.login {invite_id, secret}` (public) →
    /// `{status:"ready", callback_url}` | `{status:"denied", denial}`. The callback
    /// sets the Zone root cookies and returns to the invite's `return_path`.
    pub(crate) async fn handle_auth_invite_login(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let params: InviteSecretParams = Self::typed_params(&req)?;
        if !is_invite_id(&params.invite_id) {
            return success(
                InviteLoginResult::Denied {
                    denial: InviteLoginRefusal::Revoked.denial(),
                },
                req.seq,
            );
        }
        let runtime = get_buckyos_api_runtime()?;
        let target = AuthTarget::system(
            SystemServiceId::parse(CONTROL_PANEL_SERVICE_UNIQUE_ID)
                .map_err(RPCErrors::ReasonError)?,
        );
        let verify_hub = runtime.get_verify_hub_client().await?;
        let token_pair = match verify_hub
            .login_by_exclusive_invite(ExclusiveInviteLoginRequest {
                invite_id: params.invite_id.clone(),
                secret: params.secret,
                target: target.clone(),
            })
            .await?
        {
            ExclusiveInviteLoginResult::Denied { denial } => {
                return success(InviteLoginResult::Denied { denial }, req.seq);
            }
            ExclusiveInviteLoginResult::Issued {
                user_id,
                token_pair,
            } => {
                let issued_to = RPCSessionToken::from_string(&token_pair.session_token)?.sub;
                if issued_to.as_deref() != Some(user_id.as_str()) {
                    let _ = verify_hub.logout(&token_pair.refresh_token).await;
                    return Err(RPCErrors::InvalidToken(
                        "exclusive invite session does not belong to its account".to_string(),
                    ));
                }
                token_pair
            }
        };
        let prepared = async {
            let kv = ZoneKv::connect().await?;
            let (invite, _) = load_invite(&kv, &params.invite_id)
                .await?
                .ok_or_else(|| RPCErrors::ReasonError("invite record vanished".to_string()))?;
            let zone_host = Self::sso_zone_host(runtime)?;
            let redirect_url = format!(
                "{}{}",
                Self::zone_root_origin(&req, &zone_host),
                invite.return_path
            );
            let resolved = Self::resolve_sso_auth_target(
                &redirect_url,
                &zone_host,
                !runtime.force_https,
                runtime.node_gateway_port,
            )?;
            if resolved.auth_target != target {
                return Err(RPCErrors::ParseRequestError(
                    "return_path does not lead to the Zone root".to_string(),
                ));
            }
            Self::validate_sso_token_pair(
                &token_pair.session_token,
                &token_pair.refresh_token,
                &target,
            )?;
            Ok(resolved)
        }
        .await;
        let resolved = match prepared {
            Ok(resolved) => resolved,
            Err(error) => {
                let _ = verify_hub.logout(&token_pair.refresh_token).await;
                return Err(error);
            }
        };
        let nonce = self
            .store_new_pending_sso_login(
                token_pair.session_token,
                token_pair.refresh_token,
                &resolved,
            )
            .await;
        success(
            InviteLoginResult::Ready {
                callback_url: Self::sso_callback_url(&resolved, nonce)?,
            },
            req.seq,
        )
    }

    /// `auth.invite.request_resend {invite_id}` (public) →
    /// `{status:"submitted"|"rate_limited"|"not_applicable"}`; the issuer gets a
    /// System Notify (no secret) and rotates the invite in Control Panel.
    pub(crate) async fn handle_auth_invite_request_resend(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let params: InviteIdParams = Self::typed_params(&req)?;
        let kv = ZoneKv::connect().await?;
        let Some((invite, version)) = load_invite(&kv, &params.invite_id).await? else {
            return success(ResendDecision::NotApplicable, req.seq);
        };
        let account = load_account(&kv, &invite.user_id).await?.map(|(a, _)| a);
        let now = buckyos_get_unix_timestamp();
        let decision = resend_decision(Some(&invite), account.as_ref(), now);
        if decision != ResendDecision::Submitted {
            return success(decision, req.seq);
        }
        let InviteIssuer::User { user_id: issuer } = &invite.issuer else {
            return success(ResendDecision::NotApplicable, req.seq);
        };
        let count = invite.resend.as_ref().map_or(0, |resend| resend.count) + 1;
        let title = "An exclusive invite needs a new link";
        let mut body = format!(
            "Account {} asked for a new sign-in link; its previous link has expired.",
            invite.user_id
        );
        if let Some(title) = resource_title(invite.resource.as_ref()) {
            body.push_str(&format!(" Resource: {title}."));
        }
        body.push_str(" Rotate the invite under Users → Exclusive invites and send the new link through the original channel.");
        let notice = NotifySendReq::notice(
            issuer.clone(),
            title,
            body,
            format!("exclusive-invite-resend:{}:{count}", invite.invite_id),
        );
        get_buckyos_api_runtime()?
            .get_msg_center_client()
            .await?
            .notify_send(notice)
            .await?;
        let mut next = invite.clone();
        next.resend = Some(InviteResend {
            requested_at: now,
            count,
        });
        next.updated_at = now;
        if let Err(error) = save_invite(&kv, &next, version).await {
            warn!(
                "record resend of exclusive invite {} failed: {error}",
                invite.invite_id
            );
            return success(ResendDecision::RateLimited, req.seq);
        }
        info!("exclusive invite {} resend requested", invite.invite_id);
        success(ResendDecision::Submitted, req.seq)
    }

    /// `auth.invite.set_password {new_password_hash}` (the account's own session,
    /// restricted allowed) → `{status:"ok"}` | `{status:"denied", denial}`.
    pub(crate) async fn handle_auth_invite_set_password(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let params: SetPasswordParams = Self::typed_params(&req)?;
        validate_password_hash(&params.new_password_hash)?;
        let session_token = caller_token(&req)?;
        let kv = ZoneKv::connect().await?;
        let settings_key = format!("users/{}/settings", principal.username);
        let Some((mut settings, version)) = (if principal.session.is_external {
            None
        } else {
            load_account(&kv, &principal.username).await?
        }) else {
            return success(
                SetPasswordResult::Denied {
                    denial: AccessDenial::new(DenyReason::SessionKindNotAllowed),
                },
                req.seq,
            );
        };
        if let Err(denial) = first_password_check(principal, &settings) {
            return success(SetPasswordResult::Denied { denial }, req.seq);
        }
        let now = buckyos_get_unix_timestamp();
        settings.password = params.new_password_hash;
        settings.password_state = PasswordState::Set;
        let mut actions = consume_invite_actions(&kv, &principal.username, now).await?;
        actions.insert(settings_key.clone(), KVAction::Update(to_json(&settings)?));
        kv.exec_tx(actions, Some((settings_key, version))).await?;
        revoke_invite_families(&principal.username, Some(session_token)).await;
        info!("account '{}' set its first password", principal.username);
        success(SetPasswordResult::Ok, req.seq)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::PrincipalSession;
    use async_trait::async_trait;
    use buckyos_api::SessionCeiling;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    /// An in-memory system-config with versions and `Create` / CAS semantics.
    #[derive(Default)]
    pub(crate) struct MemKv {
        data: Mutex<HashMap<String, (String, u64)>>,
        revision: AtomicU64,
    }

    impl MemKv {
        pub(crate) fn put_raw(&self, key: &str, value: &str) {
            let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
            self.data
                .lock()
                .unwrap()
                .insert(key.to_string(), (value.to_string(), revision));
        }

        pub(crate) fn put<T: Serialize>(&self, key: &str, value: &T) {
            self.put_raw(key, &serde_json::to_string(value).unwrap());
        }

        pub(crate) fn read<T: serde::de::DeserializeOwned>(&self, key: &str) -> T {
            serde_json::from_str(&self.data.lock().unwrap()[key].0).unwrap()
        }

        pub(crate) fn has(&self, key: &str) -> bool {
            self.data.lock().unwrap().contains_key(key)
        }
    }

    #[async_trait]
    impl AgentKv for MemKv {
        async fn get(&self, key: &str) -> Result<Option<(String, u64)>, RPCErrors> {
            Ok(self.data.lock().unwrap().get(key).cloned())
        }

        async fn list(&self, key: &str) -> Result<Vec<String>, RPCErrors> {
            let prefix = format!("{key}/");
            let mut children = self
                .data
                .lock()
                .unwrap()
                .keys()
                .filter_map(|item| item.strip_prefix(&prefix))
                .map(|rest| rest.split('/').next().unwrap().to_string())
                .collect::<Vec<_>>();
            children.sort();
            children.dedup();
            Ok(children)
        }

        async fn exec_tx(
            &self,
            actions: HashMap<String, KVAction>,
            main_key: Option<(String, u64)>,
        ) -> Result<(), RPCErrors> {
            let mut data = self.data.lock().unwrap();
            if let Some((key, expected)) = main_key {
                if data.get(&key).map(|(_, revision)| *revision) != Some(expected) {
                    return Err(RPCErrors::ReasonError("revision mismatch".to_string()));
                }
            }
            for (key, action) in &actions {
                if matches!(action, KVAction::Create(_)) && data.contains_key(key) {
                    return Err(RPCErrors::ReasonError(format!("{key} exists")));
                }
            }
            for (key, action) in actions {
                let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
                match action {
                    KVAction::Create(value) | KVAction::Update(value) => {
                        data.insert(key, (value, revision));
                    }
                    KVAction::SetByJsonPath(paths) => {
                        let (raw, _) = data
                            .get(&key)
                            .cloned()
                            .expect("set_by_path on a missing key");
                        let mut value: Value = serde_json::from_str(&raw).unwrap();
                        for (path, item) in paths {
                            buckyos_kit::set_json_by_path(&mut value, &path, item.as_ref());
                        }
                        data.insert(key, (value.to_string(), revision));
                    }
                    KVAction::Remove => {
                        data.remove(&key);
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        }
    }

    pub(crate) fn zone() -> DID {
        DID::new("bns", "test")
    }

    fn channel() -> InviteChannel {
        InviteChannel {
            kind: "email".to_string(),
            target_hint: "a***@acme.com".to_string(),
        }
    }

    fn params(value: Value) -> CreateInviteParams {
        serde_json::from_value(value).unwrap()
    }

    fn principal(user: &str, user_type: UserType, ceiling: SessionCeiling) -> RpcAuthPrincipal {
        RpcAuthPrincipal {
            username: user.to_string(),
            owner_user_id: user.to_string(),
            authenticated_app_id: "control-panel".to_string(),
            user_type,
            owner_did: String::new(),
            is_user_session: true,
            is_control_panel_session: true,
            agent_permission_mode: None,
            session: PrincipalSession {
                ceiling,
                ..Default::default()
            },
        }
    }

    async fn new_invite(kv: &MemKv, now: u64) -> CreatedInvite {
        create_invite(
            kv,
            &zone(),
            "devtest",
            params(json!({
                "new_user": {"user_id": "acme-c1", "show_name": "Alice"},
                "channel": channel(),
                "resource": {"kind": "group_session", "group_did": "did:bns:support.test", "session_id": "t-123", "title": "Ticket #123"},
            })),
            now,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn create_makes_a_passwordless_limited_account_without_canonical_zone() {
        let kv = MemKv::default();
        let now = 1_000;
        let created = new_invite(&kv, now).await;
        let invite = &created.invite;
        assert!(created.new_account && is_invite_id(&invite.invite_id));
        assert_eq!(invite.user_id, "acme-c1");
        assert_eq!(
            invite.issuer,
            InviteIssuer::User {
                user_id: "devtest".to_string()
            }
        );
        assert_eq!(invite.resource_state, InviteResourceState::Pending);
        assert_eq!(
            invite.return_path,
            "/messagehub/guest?group=did%3Abns%3Asupport.test&session=t-123"
        );
        assert_eq!(invite.credential.version, 1);
        assert_eq!(
            invite.credential.expires_at,
            now + EXCLUSIVE_INVITE_TTL_SECS
        );
        assert_eq!(
            invite.credential.digest,
            invite_secret_digest(&created.secret)
        );
        assert_eq!(URL_SAFE_NO_PAD.decode(&created.secret).unwrap().len(), 32);
        let stored: ExclusiveInvite = kv.read(&exclusive_invite_key(&invite.invite_id));
        assert_eq!(&stored, invite);
        assert!(!invite_view(&stored)
            .to_string()
            .contains(&stored.credential.digest));

        let settings: UserSettings = kv.read("users/acme-c1/settings");
        assert_eq!(settings.user_type, UserType::Limited);
        assert_eq!(settings.password_state, PasswordState::Unset);
        assert!(settings.password.is_empty() && settings.groups.is_empty());
        let doc: name_lib::OwnerDocument = kv.read("users/acme-c1/doc");
        assert_eq!(doc.get_default_zone_did(), None);
        assert_eq!(created.account_did, doc.id);
        assert!(kv.has("security/acme-c1/key"));

        // The name is taken now; a second invite reuses the account explicitly.
        assert!(create_invite(
            &kv,
            &zone(),
            "devtest",
            params(json!({"new_user": {"user_id": "acme-c1"}, "channel": channel()})),
            now,
        )
        .await
        .is_err());
        let reused = create_invite(
            &kv,
            &zone(),
            "devtest",
            params(
                json!({"user_id": "acme-c1", "channel": channel(), "return_path": "/workspace/w1"}),
            ),
            now,
        )
        .await
        .unwrap();
        assert!(!reused.new_account);
        assert_eq!(reused.account_did, created.account_did);
        assert_eq!(reused.invite.resource_state, InviteResourceState::None);
        assert_ne!(reused.invite.invite_id, invite.invite_id);
    }

    #[tokio::test]
    async fn create_refuses_ambiguous_accounts_and_foreign_paths() {
        let kv = MemKv::default();
        let mut user = UserSettings {
            user_id: "bob".to_string(),
            user_type: UserType::User,
            password: "x".to_string(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: None,
            allow_contact_edit: None,
            groups: Vec::new(),
            password_state: PasswordState::Set,
        };
        kv.put("users/bob/settings", &user);
        for value in [
            json!({"channel": channel()}),
            json!({"user_id": "bob", "new_user": {"user_id": "carl"}, "channel": channel()}),
            json!({"user_id": "bob", "channel": channel()}),
            json!({"user_id": "nobody", "channel": channel()}),
            json!({"new_user": {"user_id": "admin"}, "channel": channel()}),
            json!({"new_user": {"user_id": "carl"}, "channel": channel(), "return_path": "https://evil.example/"}),
            json!({"new_user": {"user_id": "carl"}, "channel": channel(), "return_path": "//evil.example/"}),
        ] {
            assert!(
                matches!(
                    create_invite(&kv, &zone(), "devtest", params(value.clone()), 1).await,
                    Err(RPCErrors::ParseRequestError(_))
                ),
                "{value}"
            );
        }
        assert!(serde_json::from_value::<CreateInviteParams>(
            json!({"new_user": {"user_id": "carl", "user_type": "admin"}, "channel": channel()})
        )
        .is_err());
        user.user_type = UserType::Limited;
        user.state = UserState::Deleted;
        kv.put("users/bob/settings", &user);
        assert!(create_invite(
            &kv,
            &zone(),
            "devtest",
            params(json!({"user_id": "bob", "channel": channel()})),
            1
        )
        .await
        .is_err());
    }

    fn account(password_state: PasswordState) -> UserSettings {
        UserSettings {
            user_id: "acme-c1".to_string(),
            user_type: UserType::Limited,
            password: String::new(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: None,
            allow_contact_edit: None,
            groups: Vec::new(),
            password_state,
        }
    }

    #[tokio::test]
    async fn login_check_follows_the_contract_order() {
        let kv = MemKv::default();
        let now = 1_000;
        let created = new_invite(&kv, now).await;
        let digest = invite_secret_digest(&created.secret);
        let invite = created.invite;
        let unset = account(PasswordState::Unset);
        assert!(invite_login_check(Some(&invite), Some(&unset), &digest, now + 60).is_ok());
        assert_eq!(
            invite_login_check(None, Some(&unset), &digest, now),
            Err(InviteLoginRefusal::Revoked)
        );
        let mut suspended = unset.clone();
        suspended.state = UserState::Suspended("x".to_string());
        assert_eq!(
            invite_login_check(Some(&invite), Some(&suspended), &digest, now),
            Err(InviteLoginRefusal::AccountInactive)
        );
        assert_eq!(
            invite_login_check(
                Some(&invite),
                Some(&account(PasswordState::Set)),
                &digest,
                now
            ),
            Err(InviteLoginRefusal::PasswordRequired)
        );
        assert_eq!(
            invite_login_check(Some(&invite), Some(&unset), "other", now),
            Err(InviteLoginRefusal::LinkReplaced)
        );
        assert_eq!(
            invite_login_check(
                Some(&invite),
                Some(&unset),
                &digest,
                now + EXCLUSIVE_INVITE_TTL_SECS + 1
            ),
            Err(InviteLoginRefusal::Expired)
        );
        let mut revoked = invite.clone();
        revoked.state = InviteState::Revoked;
        assert_eq!(
            invite_login_check(Some(&revoked), Some(&suspended), &digest, now),
            Err(InviteLoginRefusal::Revoked)
        );
        // The holder of the current secret learns where to sign in with a password.
        let expired = serde_json::to_value(describe_outcome(
            Some(&invite),
            Some(&unset),
            None,
            &digest,
            now + EXCLUSIVE_INVITE_TTL_SECS + 1,
        ))
        .unwrap();
        assert_eq!(expired["status"], "denied");
        assert_eq!(expired["denial"]["reason"], "credential_state");
        assert_eq!(expired["denial"]["detail"], "expired");
        assert_eq!(expired["user_id"], "acme-c1");
        assert_eq!(expired["return_path"], invite.return_path);
        let password = serde_json::to_value(describe_outcome(
            Some(&invite),
            Some(&account(PasswordState::Set)),
            None,
            &digest,
            now,
        ))
        .unwrap();
        assert_eq!(password["denial"]["detail"], "password_required");
        assert_eq!(password["user_id"], "acme-c1");
        // Anyone else learns nothing about the account.
        let guessed = serde_json::to_value(describe_outcome(
            Some(&invite),
            Some(&unset),
            None,
            "other",
            now,
        ))
        .unwrap();
        assert_eq!(guessed["denial"]["detail"], "link_replaced");
        assert!(guessed.get("user_id").is_none() && guessed.get("return_path").is_none());
        let missing =
            serde_json::to_value(describe_outcome(None, None, None, &digest, now)).unwrap();
        assert_eq!(missing["denial"]["reason"], "source_revoked");
        assert!(missing.get("user_id").is_none());
        let active = serde_json::to_value(describe_outcome(
            Some(&invite),
            Some(&unset),
            Some("Alice".to_string()),
            &digest,
            now,
        ))
        .unwrap();
        assert_eq!(
            active,
            json!({"status": "active", "user_id": "acme-c1", "show_name": "Alice",
                   "resource_title": "Ticket #123", "return_path": invite.return_path})
        );
    }

    #[tokio::test]
    async fn rotate_replaces_the_link_and_clears_the_resend_request() {
        let kv = MemKv::default();
        let now = 1_000;
        let created = new_invite(&kv, now).await;
        let mut invite = created.invite;
        invite.resend = Some(InviteResend {
            requested_at: now,
            count: 1,
        });
        let unset = account(PasswordState::Unset);
        assert!(rotate_check(&invite, Some(&unset)).is_ok());
        let later = now + EXCLUSIVE_INVITE_TTL_SECS + 10;
        let (next, secret) = rotated(&invite, later);
        assert_eq!(next.credential.version, 2);
        assert_eq!(
            next.credential.expires_at,
            later + EXCLUSIVE_INVITE_TTL_SECS
        );
        assert!(next.resend.is_none());
        let old_digest = invite_secret_digest(&created.secret);
        assert_eq!(
            invite_login_check(Some(&next), Some(&unset), &old_digest, later),
            Err(InviteLoginRefusal::LinkReplaced)
        );
        assert!(invite_login_check(
            Some(&next),
            Some(&unset),
            &invite_secret_digest(&secret),
            later
        )
        .is_ok());

        assert_eq!(
            rotate_check(&invite, Some(&account(PasswordState::Set)))
                .unwrap_err()
                .reason,
            DenyReason::CredentialState
        );
        let mut revoked = invite.clone();
        revoked.state = InviteState::Revoked;
        assert_eq!(
            rotate_check(&revoked, Some(&unset)).unwrap_err().reason,
            DenyReason::SourceRevoked
        );
    }

    #[tokio::test]
    async fn resend_only_for_expired_links_once_per_interval() {
        let kv = MemKv::default();
        let now = 1_000;
        let mut invite = new_invite(&kv, now).await.invite;
        let unset = account(PasswordState::Unset);
        let expired = now + EXCLUSIVE_INVITE_TTL_SECS + 1;
        assert_eq!(
            resend_decision(Some(&invite), Some(&unset), now + 10),
            ResendDecision::NotApplicable
        );
        assert_eq!(
            resend_decision(Some(&invite), Some(&unset), expired),
            ResendDecision::Submitted
        );
        invite.resend = Some(InviteResend {
            requested_at: expired,
            count: 1,
        });
        assert_eq!(
            resend_decision(Some(&invite), Some(&unset), expired + 60),
            ResendDecision::RateLimited
        );
        assert_eq!(
            resend_decision(
                Some(&invite),
                Some(&unset),
                expired + EXCLUSIVE_INVITE_RESEND_INTERVAL_SECS
            ),
            ResendDecision::Submitted
        );
        assert_eq!(
            resend_decision(Some(&invite), Some(&account(PasswordState::Set)), expired),
            ResendDecision::NotApplicable
        );
        let mut consumed = invite.clone();
        consumed.credential.state = InviteCredentialState::ConsumedByPassword;
        assert_eq!(
            resend_decision(Some(&consumed), Some(&unset), expired + 3600),
            ResendDecision::NotApplicable
        );
        let mut revoked = invite.clone();
        revoked.state = InviteState::Revoked;
        assert_eq!(
            resend_decision(Some(&revoked), Some(&unset), expired + 3600),
            ResendDecision::NotApplicable
        );
        assert_eq!(
            resend_decision(None, None, expired),
            ResendDecision::NotApplicable
        );
        assert_eq!(
            serde_json::to_value(ResendDecision::RateLimited).unwrap(),
            json!({"status": "rate_limited"})
        );
    }

    #[tokio::test]
    async fn first_password_consumes_every_active_link_of_the_account() {
        let kv = MemKv::default();
        let now = 1_000;
        let first = new_invite(&kv, now).await.invite;
        let second = create_invite(
            &kv,
            &zone(),
            "devtest",
            params(json!({"user_id": "acme-c1", "channel": channel()})),
            now,
        )
        .await
        .unwrap()
        .invite;
        let actions = consume_invite_actions(&kv, "acme-c1", now + 5)
            .await
            .unwrap();
        assert_eq!(actions.len(), 2);
        kv.exec_tx(actions, None).await.unwrap();
        for invite_id in [&first.invite_id, &second.invite_id] {
            let stored: ExclusiveInvite = kv.read(&exclusive_invite_key(invite_id));
            assert_eq!(
                stored.credential.state,
                InviteCredentialState::ConsumedByPassword
            );
        }
        assert!(consume_invite_actions(&kv, "acme-c1", now)
            .await
            .unwrap()
            .is_empty());

        let limited = SessionCeiling::for_user_type(UserType::Limited);
        let own = principal("acme-c1", UserType::Limited, limited.clone());
        assert!(first_password_check(&own, &account(PasswordState::Unset)).is_ok());
        assert_eq!(
            first_password_check(&own, &account(PasswordState::Set))
                .unwrap_err()
                .reason,
            DenyReason::CredentialState
        );
        let other = principal("devtest", UserType::Admin, SessionCeiling::unrestricted());
        assert_eq!(
            first_password_check(&other, &account(PasswordState::Unset))
                .unwrap_err()
                .reason,
            DenyReason::SessionKindNotAllowed
        );
        let mut external = principal("acme-c1", UserType::Limited, SessionCeiling::external());
        external.session.is_external = true;
        assert!(first_password_check(&external, &account(PasswordState::Unset)).is_err());
    }

    #[test]
    fn invite_ids_and_return_paths_stay_inside_their_namespaces() {
        assert!(is_invite_id(&new_invite_id()));
        for bad in ["", "ei-", "ei-../x", "EI-abc", "x-abc", "ei-ABC"] {
            assert!(!is_invite_id(bad), "{bad}");
        }
        assert!(validate_return_path("/messagehub?sessionId=t-1#x").is_ok());
        for bad in ["", "messagehub", "//evil", "/a\\b", "/a\nb", "https://evil"] {
            assert!(validate_return_path(bad).is_err(), "{bad}");
        }
        assert_eq!(default_return_path(None), "/");
        assert_eq!(
            invite_link("https://test.buckyos.io", "ei-1", "s3cr3t"),
            "https://test.buckyos.io/sso/invite/ei-1#s=s3cr3t"
        );
    }
}
