//! App 代登录 (self-host-group-extension TODO EXT-09, D-7): an App signs its own
//! external customers in. Its backend, with its App service identity, asks
//! `user.app_login.issue` for a one-time code; Control Panel maps the external
//! account to a Limited account (created on first sight, never a system group),
//! writes an exclusive-invite record with `issuer.kind = "app"`, and the App's page
//! exchanges the code through `auth.app_login.exchange` for a restricted session
//! targeting that App instance. The owner switches it per App instance with
//! `apps.app_login.set`; records live under `services/control_panel/app_login/`
//! and `services/control_panel/app_identities/`.

use crate::agent_mgr::{AgentKv, AgentNameIndex, ZoneKv};
use crate::exclusive_invite::{
    invite_view, is_invite_id, issue_credential_with_ttl, load_account, load_invite,
    new_invite_id, revoke_families, validate_return_path,
};
use crate::user_mgr::{after_account_created, new_account_actions, validate_username};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use buckyos_api::{
    app_identity_digest, app_identity_key, app_login_code, app_login_settings_key,
    exclusive_invite_key, get_buckyos_api_runtime, split_app_login_code, AccessDenial,
    AppAccessTokenLoginRequest, AppIdentityBinding, AppInstanceId, AppLoginSettings, AuthTarget,
    DenyReason, ExclusiveInvite, ExclusiveInviteLoginResult, InviteChannel, InviteCredentialState,
    InviteIssuer, InviteLoginRefusal, InviteResource, InviteResourceState, InviteState,
    PasswordState, RevokeFamiliesRequest, TokenPrincipalKind, UserSettings, UserState, UserType,
    APP_IDENTITIES_PREFIX, APP_LOGIN_CHANNEL_KIND, APP_LOGIN_CODE_TTL_SECS,
    APP_LOGIN_ISSUE_PER_MINUTE, CONTROL_PANEL_SERVICE_UNIQUE_ID,
};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use log::{info, warn};
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

const ISSUE_METHOD: &str = "user.app_login.issue";
const EXCHANGE_METHOD: &str = "auth.app_login.exchange";
const MAX_EXTERNAL_ACCOUNT_ID_LEN: usize = 256;
const LIMIT_WINDOW_SECS: u64 = 60;
const EXCHANGES_PER_CLIENT_PER_MINUTE: usize = 30;

/// Issuances per App instance and exchanges per client address, each within a minute.
#[derive(Default)]
pub(crate) struct AppLoginLimiter {
    issues: HashMap<String, VecDeque<u64>>,
    exchanges: HashMap<IpAddr, VecDeque<u64>>,
}

fn admit(times: &mut VecDeque<u64>, now: u64, max: usize) -> bool {
    let since = now.saturating_sub(LIMIT_WINDOW_SECS);
    while times.front().is_some_and(|at| *at <= since) {
        times.pop_front();
    }
    if times.len() >= max {
        return false;
    }
    times.push_back(now);
    true
}

impl AppLoginLimiter {
    pub(crate) fn admit_issue(&mut self, app_instance_id: &str, now: u64) -> bool {
        self.issues.retain(|_, times| times.back().is_some_and(|at| at + LIMIT_WINDOW_SECS > now));
        admit(
            self.issues.entry(app_instance_id.to_string()).or_default(),
            now,
            APP_LOGIN_ISSUE_PER_MINUTE,
        )
    }

    /// A loopback address stands for "client unknown" and is not limited.
    pub(crate) fn admit_exchange(&mut self, client: IpAddr, now: u64) -> bool {
        if client.is_loopback() {
            return true;
        }
        self.exchanges
            .retain(|_, times| times.back().is_some_and(|at| at + LIMIT_WINDOW_SECS > now));
        admit(
            self.exchanges.entry(client).or_default(),
            now,
            EXCHANGES_PER_CLIENT_PER_MINUTE,
        )
    }
}

fn rate_limited(method: &str) -> RPCErrors {
    AccessDenial::new(DenyReason::AuthorizationUnavailable)
        .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, method)
        .with_detail("rate_limited")
        .into_rpc_error()
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct IssueParams {
    external_account_id: String,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    show_name: Option<String>,
    #[serde(default)]
    resource: Option<InviteResource>,
    #[serde(default)]
    return_path: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppInstanceParams {
    app_instance_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsSetParams {
    app_instance_id: String,
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExchangeParams {
    code: String,
    nonce: String,
    #[serde(default)]
    return_path: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ExchangeResult {
    Ready { callback_url: String, user_id: String },
    Denied { denial: AccessDenial },
}

#[derive(Debug)]
pub(crate) struct IssuedAppLogin {
    pub(crate) invite: ExclusiveInvite,
    pub(crate) secret: String,
    pub(crate) account_did: DID,
    pub(crate) new_account: bool,
}

impl IssuedAppLogin {
    pub(crate) fn code(&self) -> String {
        app_login_code(&self.invite.invite_id, &self.secret)
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(key: &str, raw: &str) -> Result<T, RPCErrors> {
    serde_json::from_str(raw)
        .map_err(|error| RPCErrors::ReasonError(format!("invalid `{key}`: {error}")))
}

fn to_json<T: Serialize>(value: &T) -> Result<String, RPCErrors> {
    serde_json::to_string(value)
        .map_err(|error| RPCErrors::ReasonError(format!("serialize failed: {error}")))
}

pub(crate) async fn load_settings(
    kv: &dyn AgentKv,
    app_instance_id: &AppInstanceId,
) -> Result<(AppLoginSettings, Option<u64>), RPCErrors> {
    let key = app_login_settings_key(&app_instance_id.to_string());
    match kv.get(&key).await? {
        Some((raw, version)) => Ok((parse_json(&key, &raw)?, Some(version))),
        None => Ok((AppLoginSettings::default(), None)),
    }
}

async fn load_binding(
    kv: &dyn AgentKv,
    key: &str,
) -> Result<Option<(AppIdentityBinding, u64)>, RPCErrors> {
    match kv.get(key).await? {
        Some((raw, version)) => Ok(Some((parse_json(key, &raw)?, version))),
        None => Ok(None),
    }
}

/// `<app label>-<16 hex of sha256(digest)>`: stable for one external account of one
/// App instance, meaningless to anyone else.
pub(crate) fn app_login_user_id(app_instance_id: &AppInstanceId, digest: &str) -> String {
    use sha2::Digest;
    let label: String = app_instance_id
        .app_id()
        .as_str()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(20)
        .collect();
    let label = if label.is_empty() { "app".to_string() } else { label };
    let suffix = hex::encode(sha2::Sha256::digest(digest.as_bytes()));
    format!("{label}-{}", &suffix[..16])
}

fn disabled(method: &str) -> RPCErrors {
    AccessDenial::new(DenyReason::ZoneMethodDisabled)
        .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, method)
        .with_detail("app_login_disabled")
        .into_rpc_error()
}

/// Maps the external account to its Limited account (creating it on first sight),
/// and writes the one-time code record; one transaction. An explicit `user_id` must
/// already be bound to the same external account (SSO-10: no naming other accounts).
pub(crate) async fn issue_app_login(
    kv: &dyn AgentKv,
    zone_did: &DID,
    app_instance_id: &AppInstanceId,
    params: IssueParams,
    now: u64,
) -> Result<IssuedAppLogin, RPCErrors> {
    let (settings, _) = load_settings(kv, app_instance_id).await?;
    if !settings.enabled {
        return Err(disabled(ISSUE_METHOD));
    }
    let external = params.external_account_id.trim();
    if external.is_empty() || external.len() > MAX_EXTERNAL_ACCOUNT_ID_LEN {
        return Err(RPCErrors::ParseRequestError(format!(
            "external_account_id must be 1-{MAX_EXTERNAL_ACCOUNT_ID_LEN} characters"
        )));
    }
    let return_path = match params.return_path {
        Some(path) => {
            validate_return_path(&path)?;
            path
        }
        None => "/".to_string(),
    };
    let app = app_instance_id.to_string();
    let digest = app_identity_digest(&app, external);
    let binding_key = app_identity_key(&app, &digest);
    let requested = params
        .user_id
        .as_deref()
        .map(|id| id.trim().to_lowercase())
        .filter(|id| !id.is_empty());
    let mut actions = HashMap::new();
    let mut main_key = None;
    let (user_id, account_did, new_account) = match load_binding(kv, &binding_key).await? {
        Some((mut binding, version)) => {
            if requested.as_deref().is_some_and(|id| id != binding.user_id) {
                return Err(RPCErrors::NoPermission(
                    "user_id is not the account bound to this external account".to_string(),
                ));
            }
            let (settings, _) = load_account(kv, &binding.user_id).await?.ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "bound account `{}` no longer exists",
                    binding.user_id
                ))
            })?;
            if settings.user_type != UserType::Limited
                || matches!(settings.state, UserState::Deleted)
            {
                return Err(AccessDenial::new(DenyReason::AccountState)
                    .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, ISSUE_METHOD)
                    .with_detail("bound_account_unusable")
                    .into_rpc_error());
            }
            let doc_key = format!("users/{}/doc", binding.user_id);
            let (raw_doc, _) = kv.get(&doc_key).await?.ok_or_else(|| {
                RPCErrors::ReasonError(format!("account `{}` has no DID document", binding.user_id))
            })?;
            let did = buckyos_api::user_did_from_doc_value(&doc_key, &raw_doc)?;
            binding.last_issued_at = now;
            binding.issue_count += 1;
            actions.insert(binding_key.clone(), KVAction::Update(to_json(&binding)?));
            main_key = Some((binding_key.clone(), version));
            (binding.user_id, did, false)
        }
        None => {
            if requested.is_some() {
                return Err(RPCErrors::NoPermission(
                    "user_id is not bound to this external account".to_string(),
                ));
            }
            let user_id = app_login_user_id(app_instance_id, &digest);
            validate_username(&user_id)?;
            AgentNameIndex::load(kv, zone_did)
                .await?
                .check(&user_id)
                .map_err(|conflict| {
                    RPCErrors::ReasonError(format!(
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
            let show_name = params
                .show_name
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| user_id.clone());
            let (account_actions, did) =
                new_account_actions(&settings, &show_name, zone_did, false)?;
            actions.extend(account_actions);
            let binding = AppIdentityBinding {
                user_id: user_id.clone(),
                created_at: now,
                last_issued_at: now,
                issue_count: 1,
            };
            actions.insert(binding_key.clone(), KVAction::Create(to_json(&binding)?));
            (user_id, did, true)
        }
    };
    let invite_id = new_invite_id();
    let (credential, secret) = issue_credential_with_ttl(1, now, APP_LOGIN_CODE_TTL_SECS);
    let resource_state = match params.resource {
        Some(InviteResource::GroupSession { .. }) => InviteResourceState::Pending,
        _ => InviteResourceState::None,
    };
    let invite = ExclusiveInvite {
        invite_id: invite_id.clone(),
        user_id,
        issuer: InviteIssuer::App {
            app_instance_id: app,
        },
        channel: InviteChannel {
            kind: APP_LOGIN_CHANNEL_KIND.to_string(),
            target_hint: String::new(),
        },
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
    kv.exec_tx(actions, main_key).await?;
    Ok(IssuedAppLogin {
        invite,
        secret,
        account_did,
        new_account,
    })
}

/// The one-time code has been exchanged: its record says so, whatever happens to
/// the in-memory consumption mark of verify-hub.
async fn mark_code_consumed(kv: &dyn AgentKv, invite_id: &str, now: u64) -> Result<(), RPCErrors> {
    let Some((invite, _)) = load_invite(kv, invite_id).await? else {
        return Ok(());
    };
    if invite.credential.state != InviteCredentialState::Active {
        return Ok(());
    }
    let consumed = serde_json::to_value(InviteCredentialState::Consumed)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let actions = HashMap::from([(
        exclusive_invite_key(invite_id),
        KVAction::SetByJsonPath(HashMap::from([
            ("credential/state".to_string(), Some(consumed)),
            ("updated_at".to_string(), Some(Value::from(now))),
        ])),
    )]);
    kv.exec_tx(actions, None).await
}

/// Every Limited account this App instance signed in.
async fn bound_user_ids(kv: &dyn AgentKv, app_instance_id: &str) -> Result<Vec<String>, RPCErrors> {
    let prefix = format!("{APP_IDENTITIES_PREFIX}/{app_instance_id}");
    let mut users = Vec::new();
    for digest in kv.list(&prefix).await.unwrap_or_default() {
        if let Some((binding, _)) = load_binding(kv, &format!("{prefix}/{digest}")).await? {
            users.push(binding.user_id);
        }
    }
    users.sort();
    users.dedup();
    Ok(users)
}

/// The App service identity behind an RPC: `principal_kind = app`, targeting its own
/// instance, whose owner is the token subject.
fn require_app_principal(principal: &RpcAuthPrincipal) -> Result<AppInstanceId, RPCErrors> {
    let denied = || {
        RPCErrors::NoPermission(format!(
            "{ISSUE_METHOD} is called by an App with its own service identity"
        ))
    };
    if principal.principal_kind != TokenPrincipalKind::App || principal.session.is_sudo {
        return Err(denied());
    }
    match AuthTarget::from_canonical_key(&principal.authenticated_app_id) {
        Ok(AuthTarget::App { app_instance_id })
            if app_instance_id.owner_user_id() == principal.username =>
        {
            Ok(app_instance_id)
        }
        _ => Err(denied()),
    }
}

/// The gateway host label of an App instance (`<label>.<zone>`): the `app_info` key of
/// `node_gateway_info.json` that names this instance. Apps choose their own sub-hostname,
/// so it is not derivable from the AppId.
fn app_host_label(app_instance_id: &AppInstanceId) -> Result<String, RPCErrors> {
    let path = crate::gateway_etc_dir().join("node_gateway_info.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|error| RPCErrors::ReasonError(format!("read node_gateway_info.json: {error}")))?;
    let info: Value = serde_json::from_str(&content)
        .map_err(|error| RPCErrors::ReasonError(format!("parse node_gateway_info.json: {error}")))?;
    let wanted = app_instance_id.to_string();
    info.get("app_info")
        .and_then(Value::as_object)
        .and_then(|apps| {
            apps.iter().find_map(|(key, app)| {
                (app.get("service_id").is_none()
                    && app.get("app_instance_id").and_then(Value::as_str) == Some(wanted.as_str()))
                .then(|| key.clone())
            })
        })
        .ok_or_else(|| {
            AccessDenial::new(DenyReason::EntryUnavailable)
                .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, EXCHANGE_METHOD)
                .with_detail("app_route_missing")
                .into_rpc_error()
        })
}

fn success(value: impl Serialize, seq: u64) -> Result<RPCResponse, RPCErrors> {
    Ok(RPCResponse::new(RPCResult::Success(json!(value)), seq))
}

impl ControlPanelServer {
    /// `apps.app_login.get {app_instance_id}` → `{settings, bound_accounts}`; the App
    /// owner or an admin.
    pub(crate) async fn handle_app_login_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let params: AppInstanceParams = Self::typed_params(&req)?;
        let app_instance_id: AppInstanceId = params
            .app_instance_id
            .parse()
            .map_err(RPCErrors::ParseRequestError)?;
        if !principal.user_type.is_privileged()
            && app_instance_id.owner_user_id() != principal.username
        {
            return Err(RPCErrors::NoPermission(
                "only the app owner or an admin can inspect App login".to_string(),
            ));
        }
        let kv = ZoneKv::connect().await?;
        let (settings, _) = load_settings(&kv, &app_instance_id).await?;
        let bound = bound_user_ids(&kv, &app_instance_id.to_string()).await?;
        success(
            json!({"settings": settings, "bound_accounts": bound.len()}),
            req.seq,
        )
    }

    /// `apps.app_login.set {app_instance_id, enabled}` → `{settings, families_revoked}`;
    /// the App owner from a Control Panel session. Switching off ends every session
    /// the App signed in.
    pub(crate) async fn handle_app_login_set(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        if !principal.is_user_session || !principal.is_control_panel_session {
            return Err(RPCErrors::NoPermission(
                "App login changes require a user-authenticated Control Panel session"
                    .to_string(),
            ));
        }
        let params: SettingsSetParams = Self::typed_params(&req)?;
        let app_instance_id: AppInstanceId = params
            .app_instance_id
            .parse()
            .map_err(RPCErrors::ParseRequestError)?;
        let resolver = Self::app_availability_resolver().await?;
        let installation = resolver.resolve_installation(&app_instance_id).await?;
        if installation.spec.owner_user_id != principal.username {
            return Err(RPCErrors::NoPermission(
                "only the app owner can switch App login".to_string(),
            ));
        }
        let kv = ZoneKv::connect().await?;
        let (_, version) = load_settings(&kv, &app_instance_id).await?;
        let now = buckyos_get_unix_timestamp();
        let settings = AppLoginSettings {
            enabled: params.enabled,
            updated_by: principal.username.clone(),
            updated_at: now,
        };
        let key = app_login_settings_key(&app_instance_id.to_string());
        let action = match version {
            Some(_) => KVAction::Update(to_json(&settings)?),
            None => KVAction::Create(to_json(&settings)?),
        };
        kv.exec_tx(
            HashMap::from([(key.clone(), action)]),
            version.map(|version| (key, version)),
        )
        .await?;
        let mut families_revoked = 0;
        if !params.enabled {
            for user_id in bound_user_ids(&kv, &app_instance_id.to_string()).await? {
                families_revoked += revoke_families(RevokeFamiliesRequest {
                    subject: user_id,
                    source_app_instance_id: Some(app_instance_id.to_string()),
                    ..Default::default()
                })
                .await
                .unwrap_or(0);
            }
        }
        info!(
            "App login for {} switched {} by '{}'",
            app_instance_id,
            if params.enabled { "on" } else { "off" },
            principal.username
        );
        success(
            json!({"settings": settings, "families_revoked": families_revoked}),
            req.seq,
        )
    }

    /// `user.app_login.issue {external_account_id, user_id?, show_name?, resource?,
    /// return_path?}` → `{user_id, code, expires_at, invite, new_account}`; the App's
    /// own service session. The code appears only in this response.
    pub(crate) async fn handle_app_login_issue(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_instance_id = require_app_principal(principal)?;
        let params: IssueParams = Self::typed_params(&req)?;
        let caller_token = Self::extract_rpc_session_token(&req)
            .ok_or_else(|| RPCErrors::InvalidToken("missing session token".to_string()))?;
        let now = buckyos_get_unix_timestamp();
        if !self
            .app_login_limiter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .admit_issue(&app_instance_id.to_string(), now)
        {
            return Err(rate_limited(ISSUE_METHOD));
        }
        let runtime = get_buckyos_api_runtime()?;
        let kv = ZoneKv::connect().await?;
        let issued = issue_app_login(&kv, &runtime.zone_id, &app_instance_id, params, now).await?;
        if issued.new_account {
            if let Err(error) = after_account_created(&issued.invite.user_id, ISSUE_METHOD).await {
                warn!("RBAC refresh after App login account failed: {error}");
            }
        }
        info!(
            "App {} issued a login code {} for account '{}' (new: {})",
            app_instance_id, issued.invite.invite_id, issued.invite.user_id, issued.new_account
        );
        let code = issued.code();
        let invite = self
            .invite_resource_step(&kv, &caller_token, issued.invite, &issued.account_did)
            .await?;
        success(
            json!({
                "user_id": invite.user_id,
                "code": code,
                "expires_at": invite.credential.expires_at,
                "invite": invite_view(&invite),
                "new_account": issued.new_account,
            }),
            req.seq,
        )
    }

    /// `auth.app_login.exchange {code, nonce, return_path?}` (public) →
    /// `{status:"ready", callback_url, user_id}` | `{status:"denied", denial}`. The
    /// callback sets the host-only cookies of the App's own origin and returns to
    /// `return_path` there.
    pub(crate) async fn handle_auth_app_login_exchange(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        let params: ExchangeParams = Self::typed_params(&req)?;
        let now = buckyos_get_unix_timestamp();
        let client = Self::forwarded_client_ip(&http::HeaderMap::new(), ip_from);
        if !self
            .app_login_limiter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .admit_exchange(client, now)
        {
            return Err(rate_limited(EXCHANGE_METHOD));
        }
        let denied = |denial: AccessDenial, seq| success(ExchangeResult::Denied { denial }, seq);
        let Some((invite_id, _)) = split_app_login_code(&params.code).filter(|(id, _)| is_invite_id(id))
        else {
            return denied(InviteLoginRefusal::Revoked.denial(), req.seq);
        };
        if let Some(path) = &params.return_path {
            validate_return_path(path)?;
        }
        let kv = ZoneKv::connect().await?;
        let Some((invite, _)) = load_invite(&kv, invite_id).await? else {
            return denied(InviteLoginRefusal::Revoked.denial(), req.seq);
        };
        let InviteIssuer::App { app_instance_id } = &invite.issuer else {
            return denied(
                AccessDenial::new(DenyReason::EntryUnavailable).with_detail("target_mismatch"),
                req.seq,
            );
        };
        let app_instance_id: AppInstanceId = app_instance_id.parse().map_err(|error| {
            RPCErrors::ReasonError(format!("invite {invite_id} app instance: {error}"))
        })?;
        let target = AuthTarget::app(app_instance_id.clone());
        let runtime = get_buckyos_api_runtime()?;
        let verify_hub = runtime.get_verify_hub_client().await?;
        let token_pair = match verify_hub
            .login_by_app_access_token(AppAccessTokenLoginRequest {
                code: params.code.trim().to_string(),
                nonce: params.nonce.clone(),
                target: target.clone(),
            })
            .await?
        {
            ExclusiveInviteLoginResult::Denied { denial } => return denied(denial, req.seq),
            ExclusiveInviteLoginResult::Issued {
                user_id,
                token_pair,
            } => {
                let issued_to = RPCSessionToken::from_string(&token_pair.session_token)?.sub;
                if issued_to.as_deref() != Some(user_id.as_str()) {
                    let _ = verify_hub.logout(&token_pair.refresh_token).await;
                    return Err(RPCErrors::InvalidToken(
                        "App login session does not belong to its account".to_string(),
                    ));
                }
                token_pair
            }
        };
        if let Err(error) = mark_code_consumed(&kv, invite_id, now).await {
            warn!("App login code {invite_id} could not be marked consumed: {error}");
        }
        let prepared = async {
            let zone_host = Self::sso_zone_host(runtime)?;
            let mut origin = url::Url::parse(&Self::zone_root_origin(&req, &zone_host))
                .map_err(|error| RPCErrors::ReasonError(format!("zone origin: {error}")))?;
            origin
                .set_host(Some(&format!("{}.{zone_host}", app_host_label(&app_instance_id)?)))
                .map_err(|error| RPCErrors::ReasonError(format!("app origin: {error}")))?;
            let return_path = params.return_path.as_deref().unwrap_or(&invite.return_path);
            let redirect_url = format!("{}{return_path}", origin.as_str().trim_end_matches('/'));
            let resolved = Self::resolve_sso_auth_target(
                &redirect_url,
                &zone_host,
                !runtime.force_https,
                runtime.node_gateway_port,
            )?;
            if resolved.auth_target != target {
                return Err(RPCErrors::ParseRequestError(
                    "return_path does not lead to the App that issued the code".to_string(),
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
        let user_id = invite.user_id.clone();
        let nonce = self
            .store_new_pending_sso_login(
                token_pair.session_token,
                token_pair.refresh_token,
                &resolved,
            )
            .await;
        info!(
            "App login code {} exchanged for '{}' into {}",
            invite_id, user_id, app_instance_id
        );
        success(
            ExchangeResult::Ready {
                callback_url: Self::sso_callback_url(&resolved, nonce)?,
                user_id,
            },
            req.seq,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclusive_invite::tests::{zone, MemKv};

    fn crm() -> AppInstanceId {
        "crm@bob".parse().unwrap()
    }

    fn enable(kv: &MemKv, app: &AppInstanceId) {
        kv.put(
            &app_login_settings_key(&app.to_string()),
            &AppLoginSettings {
                enabled: true,
                updated_by: "bob".into(),
                updated_at: 1,
            },
        );
    }

    fn params(value: Value) -> IssueParams {
        serde_json::from_value(value).unwrap()
    }

    async fn issue(kv: &MemKv, app: &AppInstanceId, value: Value, now: u64) -> Result<IssuedAppLogin, RPCErrors> {
        issue_app_login(kv, &zone(), app, params(value), now).await
    }

    #[tokio::test]
    async fn issue_binds_one_limited_account_per_external_identity() {
        let kv = MemKv::default();
        let crm = crm();
        let denied = issue(&kv, &crm, json!({"external_account_id": "crm:42"}), 10)
            .await
            .unwrap_err();
        assert_eq!(
            AccessDenial::from_rpc_error(&denied).unwrap().detail.as_deref(),
            Some("app_login_disabled")
        );
        enable(&kv, &crm);

        let first = issue(
            &kv,
            &crm,
            json!({"external_account_id": "crm:42", "show_name": "Alice", "resource": {"kind": "group_session", "group_did": "did:bns:support.test", "session_id": "t-42", "title": "Ticket 42"}}),
            10,
        )
        .await
        .unwrap();
        assert!(first.new_account);
        let user_id = first.invite.user_id.clone();
        assert!(user_id.starts_with("crm-") && user_id.len() == 4 + 16, "{user_id}");
        let settings: UserSettings = kv.read(&format!("users/{user_id}/settings"));
        assert_eq!(settings.user_type, UserType::Limited);
        assert_eq!(settings.password_state, PasswordState::Unset);
        assert!(settings.groups.is_empty());
        assert!(kv.has(&format!("users/{user_id}/doc")));
        assert_eq!(
            first.invite.issuer,
            InviteIssuer::App {
                app_instance_id: "crm@bob".into()
            }
        );
        assert_eq!(first.invite.channel.kind, APP_LOGIN_CHANNEL_KIND);
        assert_eq!(first.invite.resource_state, InviteResourceState::Pending);
        assert_eq!(first.invite.credential.expires_at, 10 + APP_LOGIN_CODE_TTL_SECS);
        assert_eq!(first.invite.return_path, "/");
        let code = first.code();
        let (invite_id, secret) = split_app_login_code(&code).unwrap();
        assert_eq!(invite_id, first.invite.invite_id);
        assert_eq!(secret, first.secret);
        let stored: ExclusiveInvite = kv.read(&exclusive_invite_key(invite_id));
        assert_eq!(stored.credential.digest, buckyos_api::invite_secret_digest(secret));
        let digest = app_identity_digest("crm@bob", "crm:42");
        let binding: AppIdentityBinding = kv.read(&app_identity_key("crm@bob", &digest));
        assert_eq!(binding.user_id, user_id);
        assert_eq!(binding.issue_count, 1);

        // The same external account again: same account, a new code.
        let again = issue(&kv, &crm, json!({"external_account_id": "crm:42", "return_path": "/tickets/42"}), 20)
            .await
            .unwrap();
        assert!(!again.new_account);
        assert_eq!(again.invite.user_id, user_id);
        assert_ne!(again.invite.invite_id, first.invite.invite_id);
        assert_eq!(again.invite.return_path, "/tickets/42");
        let binding: AppIdentityBinding = kv.read(&app_identity_key("crm@bob", &digest));
        assert_eq!((binding.issue_count, binding.last_issued_at), (2, 20));
        // Naming the bound account is fine; naming any other account is not.
        issue(&kv, &crm, json!({"external_account_id": "crm:42", "user_id": user_id}), 21)
            .await
            .unwrap();
        assert!(matches!(
            issue(&kv, &crm, json!({"external_account_id": "crm:42", "user_id": "bob"}), 22).await,
            Err(RPCErrors::NoPermission(_))
        ));
        assert!(matches!(
            issue(&kv, &crm, json!({"external_account_id": "crm:7", "user_id": user_id}), 23).await,
            Err(RPCErrors::NoPermission(_))
        ));
        // Another external account, and the same external id at another App instance,
        // are other accounts.
        let other = issue(&kv, &crm, json!({"external_account_id": "crm:7"}), 30).await.unwrap();
        assert_ne!(other.invite.user_id, user_id);
        let erp: AppInstanceId = "erp@bob".parse().unwrap();
        enable(&kv, &erp);
        let elsewhere = issue(&kv, &erp, json!({"external_account_id": "crm:42"}), 31).await.unwrap();
        assert!(elsewhere.new_account);
        assert_ne!(elsewhere.invite.user_id, user_id);
        assert!(elsewhere.invite.user_id.starts_with("erp-"));
        assert_eq!(bound_user_ids(&kv, "crm@bob").await.unwrap().len(), 2);
        assert!(matches!(
            issue(&kv, &crm, json!({"external_account_id": ""}), 40).await,
            Err(RPCErrors::ParseRequestError(_))
        ));
        assert!(matches!(
            issue(&kv, &crm, json!({"external_account_id": "x", "return_path": "https://evil/"}), 41).await,
            Err(RPCErrors::ParseRequestError(_))
        ));
    }

    #[tokio::test]
    async fn a_consumed_code_is_marked_once_and_only_while_active() {
        let kv = MemKv::default();
        let crm = crm();
        enable(&kv, &crm);
        let issued = issue(&kv, &crm, json!({"external_account_id": "crm:1"}), 10).await.unwrap();
        mark_code_consumed(&kv, &issued.invite.invite_id, 11).await.unwrap();
        let stored: ExclusiveInvite = kv.read(&exclusive_invite_key(&issued.invite.invite_id));
        assert_eq!(stored.credential.state, InviteCredentialState::Consumed);
        assert_eq!(stored.updated_at, 11);
        assert_eq!(
            stored.check_credential(&buckyos_api::invite_secret_digest(&issued.secret), 12),
            Err(InviteLoginRefusal::Consumed)
        );
        mark_code_consumed(&kv, &issued.invite.invite_id, 13).await.unwrap();
        let stored: ExclusiveInvite = kv.read(&exclusive_invite_key(&issued.invite.invite_id));
        assert_eq!(stored.updated_at, 11);
        mark_code_consumed(&kv, "ei-missing", 14).await.unwrap();
    }

    #[test]
    fn limiter_bounds_issues_per_app_and_exchanges_per_client() {
        let mut limiter = AppLoginLimiter::default();
        for _ in 0..APP_LOGIN_ISSUE_PER_MINUTE {
            assert!(limiter.admit_issue("crm@bob", 100));
        }
        assert!(!limiter.admit_issue("crm@bob", 100));
        assert!(limiter.admit_issue("erp@bob", 100));
        assert!(limiter.admit_issue("crm@bob", 100 + LIMIT_WINDOW_SECS + 1));
        let client: IpAddr = "203.0.113.9".parse().unwrap();
        for _ in 0..EXCHANGES_PER_CLIENT_PER_MINUTE {
            assert!(limiter.admit_exchange(client, 100));
        }
        assert!(!limiter.admit_exchange(client, 100));
        assert!(limiter.admit_exchange("127.0.0.1".parse().unwrap(), 100));
        assert!(limiter.admit_exchange(client, 100 + LIMIT_WINDOW_SECS + 1));
    }

    #[test]
    fn app_principals_are_the_only_issuers() {
        let base = RpcAuthPrincipal {
            username: "bob".into(),
            owner_user_id: "bob".into(),
            authenticated_app_id: "app:crm@bob".into(),
            user_type: UserType::User,
            owner_did: String::new(),
            is_user_session: false,
            is_control_panel_session: false,
            principal_kind: TokenPrincipalKind::App,
            agent_permission_mode: None,
            session: crate::PrincipalSession::default(),
        };
        assert_eq!(require_app_principal(&base).unwrap(), crm());
        let mut user = base.clone();
        user.principal_kind = TokenPrincipalKind::User;
        user.is_user_session = true;
        assert!(require_app_principal(&user).is_err());
        let mut foreign = base.clone();
        foreign.username = "alice".into();
        assert!(require_app_principal(&foreign).is_err());
        let mut system = base.clone();
        system.authenticated_app_id = "system:control-panel".into();
        assert!(require_app_principal(&system).is_err());
        let mut sudo = base;
        sudo.session.is_sudo = true;
        assert!(require_app_principal(&sudo).is_err());
    }
}
