//! Owner-scoped session semantics of the MessageCenter:
//!
//! * caller identity from the verified session token and the viewer → owner
//!   authorization rules (`Message Center.md` §5.6);
//! * owner-local session lifecycle: registration of empty sessions, archive /
//!   restore, delete watermark (`Message Center.md` §5.5 / §5.8);
//! * the session list / timeline projection that applies those rules on top
//!   of the mailbox index (`UI_DATAMODEL.md` §1.4a activity ordering).

use crate::msg_center::MessageCenter;
use crate::owner_session_db::{DeleteWatermark, VisibleSessionIndexEntry};
use crate::zone_agent::{
    account_owns_agent, find_zone_agent_spec, load_agent_settings, AgentLocationHints,
};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, token_auth_method, token_session_ceiling,
    validate_verify_hub_token_claims, AccessDenial, AgentSpec, AuthMethod, DenyReason,
    MailboxAddress, MailboxKind, MailboxRecord, MsgCenterCreateSessionReq, OwnerSessionState,
    SessionCeiling, SessionLifecycle, SessionListLifecycleFilter, SessionListOrder,
    SessionMessagePage, SessionScope, SessionSummary, SessionSummaryPage, SsoSettings,
    SystemConfigError, TokenPrincipalKind, TokenUse, UiSessionStateEntry, UserGroupDirectory,
    UserSettings, MSG_CENTER_SERVICE_NAME, SSO_SETTINGS_KEY,
};
use kRPC::{RPCContext, RPCErrors, RPCSessionToken};
use log::warn;
use name_lib::DID;
use ndn_lib::MsgObjKind;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

const DEFAULT_SESSION_LIST_LIMIT: usize = 50;
const MAX_SESSION_LIST_LIMIT: usize = 200;
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 500;
const MAX_SESSION_TITLE_CHARS: usize = 64;
const MAX_SESSION_ID_CHARS: usize = 200;

/// Verifies a raw session token. Production uses the runtime trust keys;
/// tests inject a static key.
#[async_trait]
pub trait SessionTokenVerifier: Send + Sync {
    async fn authorize(
        &self,
        token: &str,
        resource: &str,
        action: &str,
    ) -> std::result::Result<(), RPCErrors> {
        let mut req = kRPC::RPCRequest::new("msg.authorize", Value::Null);
        req.token = Some(token.to_string());
        get_buckyos_api_runtime()?
            .enforce(&req, action, resource)
            .await?;
        Ok(())
    }

    /// RBAC of a method opened to restricted sessions of `scope`.
    async fn authorize_opened(
        &self,
        token: &str,
        resource: &str,
        action: &str,
        _scope: SessionScope,
    ) -> std::result::Result<(), RPCErrors> {
        self.authorize(token, resource, action).await
    }

    /// Verify an ordinary session token: restricted sessions are refused.
    async fn verify(&self, token: &str) -> std::result::Result<RPCSessionToken, RPCErrors>;

    /// Verify a session token for a method opened to restricted sessions of
    /// `scope`.
    async fn verify_opened(
        &self,
        token: &str,
        _scope: SessionScope,
    ) -> std::result::Result<RPCSessionToken, RPCErrors> {
        self.verify(token).await
    }

    /// The account's DID: the id of its `users/<id>/doc`, never the profile.
    async fn resolve_user_did(&self, user_id: &str) -> std::result::Result<DID, RPCErrors>;
    /// `None` when `did` is not an Agent of this zone with an installed spec.
    async fn zone_agent(&self, did: &DID) -> std::result::Result<Option<ZoneAgentInfo>, RPCErrors>;

    async fn is_zone_agent(&self, did: &DID) -> std::result::Result<bool, RPCErrors> {
        Ok(self.zone_agent(did).await?.is_some())
    }

    /// Owner of a zone-hosted agent (`AgentDocument.owner`).
    async fn agent_owner(&self, did: &DID) -> std::result::Result<Option<DID>, RPCErrors> {
        Ok(self.zone_agent(did).await?.map(|agent| agent.owner))
    }

    /// The Zone SSO switches as they are now (`system/sso/settings`).
    async fn sso_settings(&self) -> std::result::Result<SsoSettings, RPCErrors> {
        let client = get_buckyos_api_runtime()?
            .get_system_config_client()
            .await?;
        match client.get(SSO_SETTINGS_KEY).await {
            Ok(value) => serde_json::from_str(&value.value).map_err(|error| {
                RPCErrors::ReasonError(format!("invalid `{SSO_SETTINGS_KEY}`: {error}"))
            }),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(SsoSettings::default()),
            Err(error) => Err(RPCErrors::ReasonError(error.to_string())),
        }
    }

    /// An account's settings (type, groups, state) as they are now; `None`
    /// when there is no such account.
    async fn account_settings(
        &self,
        user_id: &str,
    ) -> std::result::Result<Option<UserSettings>, RPCErrors> {
        let client = get_buckyos_api_runtime()?
            .get_system_config_client()
            .await?;
        let key = format!("users/{user_id}/settings");
        match client.get(&key).await {
            Ok(value) => serde_json::from_str(&value.value)
                .map(Some)
                .map_err(|error| RPCErrors::ReasonError(format!("invalid `{key}`: {error}"))),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
            Err(error) => Err(RPCErrors::ReasonError(error.to_string())),
        }
    }

    /// The user groups and contact projection rules as they are now.
    async fn user_groups(&self) -> std::result::Result<UserGroupDirectory, RPCErrors> {
        get_buckyos_api_runtime()?
            .get_control_panel_client()
            .await?
            .load_user_groups()
            .await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneAgentInfo {
    pub owner: DID,
    pub allow_group: bool,
}

#[derive(Default)]
pub struct RuntimeSessionTokenVerifier {
    agent_locations: AgentLocationHints,
}

impl RuntimeSessionTokenVerifier {
    /// The Agent spec of `did` and its owner account. An Agent counts only
    /// when the spec's owner is the DID of the account it is stored under.
    async fn zone_agent_spec(
        &self,
        did: &DID,
    ) -> std::result::Result<Option<(String, AgentSpec)>, RPCErrors> {
        let client = get_buckyos_api_runtime()?
            .get_system_config_client()
            .await?;
        let Some((owner_user_id, spec)) =
            find_zone_agent_spec(client.as_ref(), did, Some(&self.agent_locations))
                .await
                .map_err(|error| permission_denied(error.to_string()))?
        else {
            return Ok(None);
        };
        if !account_owns_agent(client.as_ref(), &owner_user_id, &spec.agent_doc.owner)
            .await
            .map_err(|error| permission_denied(error.to_string()))?
        {
            warn!(
                "agent {} names an owner other than account {}; ignored",
                did.to_string(),
                owner_user_id
            );
            return Ok(None);
        }
        Ok(Some((owner_user_id, spec)))
    }
}

#[async_trait]
impl SessionTokenVerifier for RuntimeSessionTokenVerifier {
    async fn authorize_opened(
        &self,
        token: &str,
        resource: &str,
        action: &str,
        scope: SessionScope,
    ) -> std::result::Result<(), RPCErrors> {
        let mut req = kRPC::RPCRequest::new("msg.authorize", Value::Null);
        req.token = Some(token.to_string());
        get_buckyos_api_runtime()?
            .enforce_opened(&req, action, resource, scope)
            .await?;
        Ok(())
    }

    async fn verify(&self, token: &str) -> std::result::Result<RPCSessionToken, RPCErrors> {
        get_buckyos_api_runtime()?
            .verify_trusted_session_token(token)
            .await
    }

    async fn verify_opened(
        &self,
        token: &str,
        scope: SessionScope,
    ) -> std::result::Result<RPCSessionToken, RPCErrors> {
        get_buckyos_api_runtime()?
            .verify_session_token_opened(token, scope)
            .await
    }

    async fn resolve_user_did(&self, user_id: &str) -> std::result::Result<DID, RPCErrors> {
        get_buckyos_api_runtime()?
            .get_control_panel_client()
            .await?
            .get_user_did(user_id)
            .await
            .map_err(|error| permission_denied(error.to_string()))
    }

    async fn zone_agent(&self, did: &DID) -> std::result::Result<Option<ZoneAgentInfo>, RPCErrors> {
        let Some((owner_user_id, spec)) = self.zone_agent_spec(did).await? else {
            return Ok(None);
        };
        let client = get_buckyos_api_runtime()?
            .get_system_config_client()
            .await?;
        let settings = load_agent_settings(client.as_ref(), &owner_user_id, &spec.agent_id)
            .await
            .map_err(|error| permission_denied(error.to_string()))?;
        Ok(Some(ZoneAgentInfo {
            owner: spec.agent_doc.owner,
            allow_group: settings.allow_group,
        }))
    }

    async fn is_zone_agent(&self, did: &DID) -> std::result::Result<bool, RPCErrors> {
        Ok(self.zone_agent_spec(did).await?.is_some())
    }

    async fn agent_owner(&self, did: &DID) -> std::result::Result<Option<DID>, RPCErrors> {
        Ok(self
            .zone_agent_spec(did)
            .await?
            .map(|(_, spec)| spec.agent_doc.owner))
    }
}

tokio::task_local! {
    static REQUEST_IDENTITY: RequestIdentityMemo;
}

#[derive(Default)]
struct RequestIdentityMemo(std::sync::Mutex<RequestIdentityFacts>);

#[derive(Default)]
struct RequestIdentityFacts {
    verified_tokens: HashMap<String, RPCSessionToken>,
    user_dids: HashMap<String, DID>,
    zone_agents: HashMap<DID, Option<ZoneAgentInfo>>,
    agent_owners: HashMap<DID, Option<DID>>,
}

/// Only successful answers are kept, and only for this request.
pub(crate) async fn serve_with_request_identity<F: std::future::Future>(request: F) -> F::Output {
    REQUEST_IDENTITY
        .scope(RequestIdentityMemo::default(), request)
        .await
}

tokio::task_local! {
    static REQUEST_GATE: RequestGate;
}

/// The method a request serves and the scope it opens to restricted sessions
/// (D-4). Without an opened scope a restricted session is refused.
#[derive(Clone, Debug)]
struct RequestGate {
    method: String,
    scope: Option<SessionScope>,
}

fn request_gate() -> RequestGate {
    REQUEST_GATE.try_with(Clone::clone).unwrap_or(RequestGate {
        method: "*".to_string(),
        scope: None,
    })
}

/// Methods that act on the caller's own address book (`account_self`).
const OWN_CONTACT_METHODS: &[&str] = &[
    "contact.get_contact",
    "contact.list_contacts",
    "contact.update_contact",
    "contact.block_contact",
    "contact.import_contacts",
    "contact.merge_contacts",
    buckyos_api::METHOD_CONTACT_LOOKUP,
    buckyos_api::METHOD_CONTACT_UPDATE_TAGS,
    buckyos_api::METHOD_CONTACT_REMOVE_CONTACT,
    buckyos_api::METHOD_CONTACT_UNBLOCK_CONTACT,
];

/// The scope a kRPC method opens to restricted sessions: group and Session
/// paths (`msg.post_send` only into a hosted group), the caller's own address
/// book; nothing else.
fn method_scope(method: &str) -> Option<SessionScope> {
    if method.starts_with("group.") || method == "msg.post_send" {
        Some(SessionScope::MsgGroup)
    } else if OWN_CONTACT_METHODS.contains(&method) {
        Some(SessionScope::AccountSelf)
    } else {
        None
    }
}

/// Serve the kRPC `method` with the scope it opens to restricted sessions.
pub(crate) async fn serve_method<F: std::future::Future>(method: &str, request: F) -> F::Output {
    let gate = RequestGate {
        method: method.to_string(),
        scope: method_scope(method),
    };
    REQUEST_GATE.scope(gate, request).await
}

/// Serve a route (group HTTP, object read) opened to restricted sessions of
/// `scope`.
pub(crate) async fn serve_opened<F: std::future::Future>(
    method: &str,
    scope: SessionScope,
    request: F,
) -> F::Output {
    let gate = RequestGate {
        method: method.to_string(),
        scope: Some(scope),
    };
    REQUEST_GATE.scope(gate, request).await
}

/// A D-4 denial of the method being served; logged without the token.
pub(crate) fn gate_denial(reason: DenyReason, subject: &str) -> RPCErrors {
    let method = request_gate().method;
    warn!(
        "msg-center denied {} on {}: {}",
        subject,
        method,
        reason.as_str()
    );
    AccessDenial::new(reason)
        .at(MSG_CENTER_SERVICE_NAME, method)
        .into_rpc_error()
}

/// A group ACL refusal of a restricted visitor: missing and not granted read
/// the same (`resource_grant_missing`); the group's own reason is only logged,
/// it would tell an existing group from a missing one.
pub(crate) fn restricted_group_denial(subject: &str, reason: String) -> RPCErrors {
    let method = request_gate().method;
    warn!("msg-center denied {} on {}: {}", subject, method, reason);
    AccessDenial::new(DenyReason::ResourceGrantMissing)
        .at(MSG_CENTER_SERVICE_NAME, method)
        .into_rpc_error()
}

fn recall<T>(read: impl FnOnce(&RequestIdentityFacts) -> Option<T>) -> Option<T> {
    REQUEST_IDENTITY
        .try_with(|memo| {
            read(
                &memo
                    .0
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            )
        })
        .ok()
        .flatten()
}

fn remember(write: impl FnOnce(&mut RequestIdentityFacts)) {
    let _ = REQUEST_IDENTITY.try_with(|memo| {
        write(
            &mut memo
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    });
}

struct RequestScopedVerifier(Arc<dyn SessionTokenVerifier>);

#[async_trait]
impl SessionTokenVerifier for RequestScopedVerifier {
    async fn authorize(
        &self,
        token: &str,
        resource: &str,
        action: &str,
    ) -> std::result::Result<(), RPCErrors> {
        self.0.authorize(token, resource, action).await
    }

    async fn authorize_opened(
        &self,
        token: &str,
        resource: &str,
        action: &str,
        scope: SessionScope,
    ) -> std::result::Result<(), RPCErrors> {
        self.0.authorize_opened(token, resource, action, scope).await
    }

    async fn verify(&self, token: &str) -> std::result::Result<RPCSessionToken, RPCErrors> {
        if let Some(verified) = recall(|facts| facts.verified_tokens.get(token).cloned()) {
            return Ok(verified);
        }
        let verified = self.0.verify(token).await?;
        remember(|facts| {
            facts
                .verified_tokens
                .insert(token.to_string(), verified.clone());
        });
        Ok(verified)
    }

    /// A remembered token was verified in this request; `caller_identity`
    /// checks its ceiling against the opened scope on every use.
    async fn verify_opened(
        &self,
        token: &str,
        scope: SessionScope,
    ) -> std::result::Result<RPCSessionToken, RPCErrors> {
        if let Some(verified) = recall(|facts| facts.verified_tokens.get(token).cloned()) {
            return Ok(verified);
        }
        let verified = self.0.verify_opened(token, scope).await?;
        remember(|facts| {
            facts
                .verified_tokens
                .insert(token.to_string(), verified.clone());
        });
        Ok(verified)
    }

    async fn resolve_user_did(&self, user_id: &str) -> std::result::Result<DID, RPCErrors> {
        if let Some(did) = recall(|facts| facts.user_dids.get(user_id).cloned()) {
            return Ok(did);
        }
        let did = self.0.resolve_user_did(user_id).await?;
        remember(|facts| {
            facts.user_dids.insert(user_id.to_string(), did.clone());
        });
        Ok(did)
    }

    async fn zone_agent(&self, did: &DID) -> std::result::Result<Option<ZoneAgentInfo>, RPCErrors> {
        if let Some(agent) = recall(|facts| facts.zone_agents.get(did).cloned()) {
            return Ok(agent);
        }
        let agent = self.0.zone_agent(did).await?;
        remember(|facts| {
            facts.zone_agents.insert(did.clone(), agent.clone());
        });
        Ok(agent)
    }

    async fn is_zone_agent(&self, did: &DID) -> std::result::Result<bool, RPCErrors> {
        let known = recall(|facts| {
            facts
                .agent_owners
                .get(did)
                .map(Option::is_some)
                .or_else(|| facts.zone_agents.get(did).map(Option::is_some))
        });
        match known {
            Some(known) => Ok(known),
            None => self.0.is_zone_agent(did).await,
        }
    }

    async fn agent_owner(&self, did: &DID) -> std::result::Result<Option<DID>, RPCErrors> {
        let known = recall(|facts| {
            facts.agent_owners.get(did).cloned().or_else(|| {
                facts
                    .zone_agents
                    .get(did)
                    .map(|agent| agent.as_ref().map(|agent| agent.owner.clone()))
            })
        });
        if let Some(owner) = known {
            return Ok(owner);
        }
        let owner = self.0.agent_owner(did).await?;
        remember(|facts| {
            facts.agent_owners.insert(did.clone(), owner.clone());
        });
        Ok(owner)
    }

    async fn sso_settings(&self) -> std::result::Result<SsoSettings, RPCErrors> {
        self.0.sso_settings().await
    }

    async fn account_settings(
        &self,
        user_id: &str,
    ) -> std::result::Result<Option<UserSettings>, RPCErrors> {
        self.0.account_settings(user_id).await
    }

    async fn user_groups(&self) -> std::result::Result<UserGroupDirectory, RPCErrors> {
        self.0.user_groups().await
    }
}

/// Holder so `MessageCenter` can stay `Clone + Debug` while carrying a
/// swappable verifier.
pub struct TokenVerifierSlot(RwLock<Arc<dyn SessionTokenVerifier>>);

impl TokenVerifierSlot {
    pub fn new(verifier: Arc<dyn SessionTokenVerifier>) -> Self {
        Self(RwLock::new(Arc::new(RequestScopedVerifier(verifier))))
    }

    pub(crate) fn get(&self) -> Arc<dyn SessionTokenVerifier> {
        self.0.read().unwrap().clone()
    }

    #[allow(dead_code)]
    fn set(&self, verifier: Arc<dyn SessionTokenVerifier>) {
        *self.0.write().unwrap() = Arc::new(RequestScopedVerifier(verifier));
    }
}

impl std::fmt::Debug for TokenVerifierSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenVerifierSlot")
    }
}

impl Default for TokenVerifierSlot {
    fn default() -> Self {
        Self::new(Arc::new(RuntimeSessionTokenVerifier::default()))
    }
}

/// Identity of the caller as established from the verified session token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallerIdentity {
    pub user_id: String,
    pub principal_kind: TokenPrincipalKind,
    pub user_did: Option<DID>,
    /// RBAC app subject of the token (`system:<service>` / `app:<app>`).
    pub authorization_key: String,
    /// A user token for a DID without an account record in this Zone: it
    /// only acts in group sessions.
    pub external: bool,
    /// The session's approved upper bound (restricted: Limited, External).
    pub ceiling: SessionCeiling,
    pub auth_method: Option<AuthMethod>,
    /// The client the session was issued to (token `appid`).
    pub appid: Option<String>,
}

impl CallerIdentity {
    pub fn restricted(&self) -> bool {
        self.ceiling.restricted
    }
}

/// A call made inside this process: no session token and no peer address.
pub(crate) fn is_in_process_call(ctx: &RPCContext) -> bool {
    ctx.from_ip.is_none()
        && ctx
            .token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .is_none()
}

fn permission_denied(reason: impl Into<String>) -> RPCErrors {
    RPCErrors::NoPermission(reason.into())
}

/// External users (no account record here) only take part in group sessions.
fn external_denied() -> RPCErrors {
    permission_denied("external-users-use-group-sessions-only")
}

fn is_activity_kind(kind: MsgObjKind) -> bool {
    matches!(
        kind,
        MsgObjKind::Chat | MsgObjKind::GroupMsg | MsgObjKind::Deliver | MsgObjKind::Notify
    )
}

fn normalize_session_id(raw: &str) -> std::result::Result<String, RPCErrors> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(RPCErrors::ParseRequestError(
            "session_id cannot be empty".to_string(),
        ));
    }
    if trimmed.chars().count() > MAX_SESSION_ID_CHARS {
        return Err(RPCErrors::ParseRequestError(
            "session_id is too long".to_string(),
        ));
    }
    MailboxAddress::validate_session_id(trimmed).map_err(RPCErrors::ParseRequestError)?;
    Ok(trimmed.to_string())
}

fn normalize_title(raw: Option<String>) -> std::result::Result<Option<String>, RPCErrors> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_SESSION_TITLE_CHARS {
        return Err(RPCErrors::ParseRequestError(format!(
            "title exceeds {} characters",
            MAX_SESSION_TITLE_CHARS
        )));
    }
    Ok(Some(trimmed.to_string()))
}

impl MessageCenter {
    #[allow(dead_code)]
    pub fn set_token_verifier(&self, verifier: Arc<dyn SessionTokenVerifier>) {
        self.token_verifier.set(verifier);
    }

    /// Resolve the caller from `ctx.token`.
    ///
    /// * `Ok(None)`: a trusted in-process call without a token.
    ///   Network calls must carry a verified session token.
    /// * `Ok(Some(_))`: a verified verify-hub session token.
    /// * `Err(_)`: a token was supplied but is invalid / expired / not trusted.
    pub(crate) async fn caller_identity(
        &self,
        ctx: &RPCContext,
    ) -> std::result::Result<Option<CallerIdentity>, RPCErrors> {
        if is_in_process_call(ctx) {
            return Ok(None);
        }
        let Some(token) = ctx
            .token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            return Err(permission_denied("session token required"));
        };
        let gate = request_gate();
        let verifier = self.token_verifier.get();
        let verified = match gate.scope {
            Some(scope) => verifier.verify_opened(token, scope).await,
            None => verifier.verify(token).await,
        }
        .map_err(|error| match AccessDenial::from_rpc_error(&error) {
            Some(denial) => gate_denial(denial.reason, "session"),
            None => {
                warn!("msg-center rejected session token: {}", error);
                permission_denied(format!("invalid session token: {}", error))
            }
        })?;
        let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session)
            .map_err(|error| permission_denied(format!("invalid session claims: {}", error)))?;
        let user_id = verified
            .sub
            .clone()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| permission_denied("session token has no subject"))?;
        let ceiling = token_session_ceiling(&verified)
            .map_err(|error| permission_denied(format!("invalid session ceiling: {}", error)))?;
        if ceiling.restricted {
            match gate.scope {
                Some(scope) if ceiling.allows(scope) => {}
                Some(_) => return Err(gate_denial(DenyReason::SessionCeilingExceeded, &user_id)),
                None => return Err(gate_denial(DenyReason::SessionKindNotAllowed, &user_id)),
            }
        }
        let external = claims.principal_kind == TokenPrincipalKind::User
            && buckyos_api::token_is_external(&verified);
        let user_did = if external {
            // No account record here: the token subject is the DID itself.
            Some(DID::from_str(&user_id).map_err(|error| {
                permission_denied(format!("External subject is not a DID: {}", error))
            })?)
        } else if claims.principal_kind == TokenPrincipalKind::User {
            Some(
                self.token_verifier
                    .get()
                    .resolve_user_did(&user_id)
                    .await
                    .map_err(|error| {
                        permission_denied(format!("cannot resolve user identity: {}", error))
                    })?,
            )
        } else {
            None
        };
        Ok(Some(CallerIdentity {
            user_id,
            authorization_key: claims.target.authorization_key(),
            principal_kind: claims.principal_kind,
            user_did,
            external,
            ceiling,
            auth_method: token_auth_method(&verified),
            appid: verified.appid.clone(),
        }))
    }

    /// Internal maintenance (`reload_settings`) is for the Zone's services
    /// and devices only.
    pub(crate) async fn require_service_caller(
        &self,
        ctx: &RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        match self.caller_identity(ctx).await? {
            None => Ok(()),
            Some(caller)
                if matches!(
                    caller.principal_kind,
                    TokenPrincipalKind::System | TokenPrincipalKind::Device
                ) =>
            {
                Ok(())
            }
            Some(_) => Err(permission_denied("service-only")),
        }
    }

    /// A zone user may observe a mailbox owner other than themselves only when
    /// the owner is a zone-hosted Agent of that user. Other users' mailboxes
    /// and their Agents' mailboxes are never readable.
    async fn user_may_observe(
        &self,
        viewer: Option<&DID>,
        owner: &DID,
    ) -> std::result::Result<bool, RPCErrors> {
        let Some(viewer) = viewer else {
            return Ok(false);
        };
        if !self.is_local_recipient(owner) {
            return Ok(false);
        }
        Ok(self.token_verifier.get().agent_owner(owner).await?.as_ref() == Some(viewer))
    }

    /// Read access of the caller to `owner`'s mailbox / sessions.
    async fn authorize_owner_read_identity(
        &self,
        ctx: &RPCContext,
        owner: &DID,
    ) -> std::result::Result<(), RPCErrors> {
        match self.caller_identity(ctx).await? {
            None => Ok(()),
            Some(caller) if caller.external => Err(external_denied()),
            Some(caller) if caller.restricted() => Err(gate_denial(
                DenyReason::SessionKindNotAllowed,
                &caller.user_id,
            )),
            Some(caller) => match caller.principal_kind {
                TokenPrincipalKind::User => {
                    if caller.user_did.as_ref() == Some(owner)
                        || self
                            .user_may_observe(caller.user_did.as_ref(), owner)
                            .await?
                    {
                        Ok(())
                    } else {
                        Err(permission_denied(format!(
                            "user {} may not read mailbox of {}",
                            caller.user_id,
                            owner.to_string()
                        )))
                    }
                }
                _ => Ok(()),
            },
        }
    }

    /// Write access: a zone user may only act as themselves. Agent observation
    /// is read-only by design (`Message Center.md` §5.6).
    async fn authorize_owner_write_identity(
        &self,
        ctx: &RPCContext,
        owner: &DID,
    ) -> std::result::Result<(), RPCErrors> {
        match self.caller_identity(ctx).await? {
            None => Ok(()),
            Some(caller) if caller.external => Err(external_denied()),
            Some(caller) if caller.restricted() => Err(gate_denial(
                DenyReason::SessionKindNotAllowed,
                &caller.user_id,
            )),
            Some(caller) => match caller.principal_kind {
                TokenPrincipalKind::User => {
                    if caller.user_did.as_ref() == Some(owner) {
                        Ok(())
                    } else {
                        Err(permission_denied(format!(
                            "user {} may not write as {}",
                            caller.user_id,
                            owner.to_string()
                        )))
                    }
                }
                _ => Ok(()),
            },
        }
    }

    pub(crate) async fn authorize_resource(
        &self,
        ctx: &RPCContext,
        resource: &str,
        action: &str,
    ) -> std::result::Result<(), RPCErrors> {
        let Some(caller) = self.caller_identity(ctx).await? else {
            return Ok(());
        };
        let token = ctx.token.as_deref().unwrap();
        let verifier = self.token_verifier.get();
        match request_gate().scope.filter(|_| caller.restricted()) {
            Some(scope) => {
                verifier
                    .authorize_opened(token, resource, action, scope)
                    .await
            }
            None => verifier.authorize(token, resource, action).await,
        }
    }

    pub(crate) async fn authorize_owner_read(
        &self,
        ctx: &RPCContext,
        owner: &DID,
    ) -> std::result::Result<(), RPCErrors> {
        self.authorize_owner_read_identity(ctx, owner).await?;
        self.authorize_resource(
            ctx,
            &MailboxAddress::new(owner.clone(), None)
                .map_err(RPCErrors::ParseRequestError)?
                .owner_resource(),
            "read",
        )
        .await
    }

    pub(crate) async fn authorize_owner_write(
        &self,
        ctx: &RPCContext,
        owner: &DID,
    ) -> std::result::Result<(), RPCErrors> {
        self.authorize_owner_write_identity(ctx, owner).await?;
        self.authorize_resource(
            ctx,
            &MailboxAddress::new(owner.clone(), None)
                .map_err(RPCErrors::ParseRequestError)?
                .owner_resource(),
            "write",
        )
        .await
    }

    /// Contact stores are owner scoped: a zone user reads and changes only the
    /// store named by their own DID (reads also cover observable agents). The
    /// system scope (no owner) is readable, but only services may change it.
    pub(crate) async fn authorize_contact_scope(
        &self,
        ctx: &RPCContext,
        owner: Option<&DID>,
        write: bool,
    ) -> std::result::Result<(), RPCErrors> {
        let contacts_resource = |owner: &DID| {
            MailboxAddress::new(owner.clone(), None)
                .map(|address| address.contacts_resource())
                .map_err(RPCErrors::ParseRequestError)
        };
        if let Some(caller) = self
            .caller_identity(ctx)
            .await?
            .filter(CallerIdentity::restricted)
        {
            // A restricted session reaches its own address book only.
            let Some(owner) = owner else {
                return Err(gate_denial(
                    DenyReason::SessionKindNotAllowed,
                    &caller.user_id,
                ));
            };
            if caller.external || caller.user_did.as_ref() != Some(owner) {
                return Err(gate_denial(
                    DenyReason::ResourceGrantMissing,
                    &caller.user_id,
                ));
            }
            self.authorize_resource(
                ctx,
                &contacts_resource(owner)?,
                if write { "write" } else { "read" },
            )
            .await?;
            // The edit right as the account is now, not as the last zone sync
            // saw it (a new Limited account is not in that snapshot yet).
            if write {
                let settings = self
                    .token_verifier
                    .get()
                    .account_settings(&caller.user_id)
                    .await
                    .map_err(|error| {
                        warn!("read account {} failed: {}", caller.user_id, error);
                        gate_denial(DenyReason::AuthorizationUnavailable, &caller.user_id)
                    })?;
                if !settings.is_some_and(|settings| settings.can_edit_contacts()) {
                    return Err(permission_denied("contact-edit-not-allowed"));
                }
            }
            return Ok(());
        }
        match owner {
            Some(owner) if write => {
                self.authorize_owner_write_identity(ctx, owner).await?;
                self.authorize_resource(ctx, &contacts_resource(owner)?, "write")
                    .await?;
                // A user without the right to edit its address book sees and
                // keeps the projection only (UserType.md, dimension 3).
                let caller = self.caller_identity(ctx).await?;
                if caller.is_some_and(|caller| caller.principal_kind == TokenPrincipalKind::User)
                    && !self.contact_mgr.owner_can_edit(owner)
                {
                    return Err(permission_denied("contact-edit-not-allowed"));
                }
                Ok(())
            }
            Some(owner) => {
                self.authorize_owner_read_identity(ctx, owner).await?;
                self.authorize_resource(ctx, &contacts_resource(owner)?, "read")
                    .await
            }
            None => match self.caller_identity(ctx).await? {
                Some(caller) if caller.external => Err(external_denied()),
                Some(caller) if write && caller.principal_kind == TokenPrincipalKind::User => {
                    Err(permission_denied(format!(
                        "user {} may not change system contacts",
                        caller.user_id
                    )))
                }
                _ => Ok(()),
            },
        }
    }

    pub(crate) async fn authorize_mailbox(
        &self,
        ctx: &RPCContext,
        mailbox: &MailboxAddress,
        kind: MailboxKind,
        action: &str,
    ) -> std::result::Result<(), RPCErrors> {
        if kind == MailboxKind::GroupInbox {
            let actor = self.group_actor(ctx).await?;
            self.authorize_group_mailbox(&actor, mailbox).await?;
            if action != "read" {
                return Err(crate::group_types::denied("group-inbox-is-authoritative"));
            }
            self.authorize_resource(ctx, &mailbox.resource(kind), action)
                .await?;
            return Ok(());
        }
        if action == "read" {
            self.authorize_owner_read_identity(ctx, mailbox.owner())
                .await?;
        } else if let Err(error) = self
            .authorize_owner_write_identity(ctx, mailbox.owner())
            .await
        {
            if mailbox.session_id().is_none() {
                return Err(error);
            }
            self.authorize_owner_read_identity(ctx, mailbox.owner())
                .await?;
            self.authorize_resource(ctx, &mailbox.resource(kind), "delegate")
                .await?;
        }
        self.authorize_resource(ctx, &mailbox.resource(kind), action)
            .await
    }

    pub(crate) async fn authorize_record(
        &self,
        ctx: &RPCContext,
        record_id: &str,
        action: &str,
    ) -> std::result::Result<MailboxRecord, RPCErrors> {
        let owner = Self::owner_from_record_id(record_id)?;
        let record = self
            .msg_box_db
            .get_record(&owner, record_id)
            .await?
            .ok_or_else(|| permission_denied("mailbox record unavailable"))?;
        self.authorize_mailbox(ctx, &record.mailbox, record.box_kind, action)
            .await?;
        if record.box_kind == MailboxKind::GroupInbox {
            self.authorize_group_message(&self.group_actor(ctx).await?, &record.msg_id)
                .await?;
        }
        Ok(record)
    }

    pub(crate) async fn authorize_message(
        &self,
        ctx: &RPCContext,
        msg_id: &ndn_lib::ObjId,
    ) -> std::result::Result<(), RPCErrors> {
        if self.caller_identity(ctx).await?.is_none() {
            if self.groups.object(msg_id).await?.is_some() {
                return Err(crate::group_types::denied("authentication-required"));
            }
            return Ok(());
        }
        if self
            .groups
            .object(msg_id)
            .await?
            .is_some_and(|(_, body, _)| body.is_none())
        {
            return Err(crate::group_types::missing());
        }
        for record in self.msg_box_db.list_message_records(msg_id).await? {
            if record.box_kind == MailboxKind::GroupInbox {
                if self
                    .authorize_group_message(&self.group_actor(ctx).await?, msg_id)
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                continue;
            }
            if self
                .authorize_mailbox(ctx, &record.mailbox, record.box_kind, "read")
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
        Err(permission_denied("message unavailable"))
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    fn new_owner_session_state(owner: DID, session_id: String, now_ms: u64) -> OwnerSessionState {
        OwnerSessionState {
            owner,
            session_id,
            lifecycle: SessionLifecycle::Active,
            registered: false,
            origin: None,
            peer_did: None,
            binding: None,
            title: None,
            archived_at_ms: None,
            delete_watermark_sort_key: None,
            delete_watermark_record_id: None,
            deleted_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }

    pub(crate) async fn create_session_internal(
        &self,
        req: MsgCenterCreateSessionReq,
    ) -> std::result::Result<OwnerSessionState, RPCErrors> {
        let now_ms = Self::now_ms();
        let session_id = match req.session_id.as_deref() {
            Some(raw) => normalize_session_id(raw)?,
            None => uuid::Uuid::new_v4().to_string(),
        };
        let title = normalize_title(req.title)?;
        if let Some(existing) = self
            .msg_box_db
            .get_owner_session(&req.owner, &session_id)
            .await?
        {
            if existing.registered && existing.peer_did.as_ref() == Some(&req.peer_did) {
                // Idempotent retry with a caller-chosen id.
                return Ok(existing);
            }
            if existing.registered {
                return Err(RPCErrors::ReasonError(format!(
                    "session {} already exists for another peer",
                    session_id
                )));
            }
            // A previously deleted or lifecycle-only row: re-register while
            // keeping its delete watermark so old history stays hidden.
            let state = OwnerSessionState {
                registered: true,
                lifecycle: SessionLifecycle::Active,
                origin: Some(req.origin.unwrap_or_else(|| "manual".to_string())),
                peer_did: Some(req.peer_did),
                binding: req.binding,
                title,
                archived_at_ms: None,
                updated_at_ms: now_ms,
                ..existing
            };
            self.msg_box_db.upsert_owner_session(&state).await?;
            return Ok(state);
        }
        let mut state = Self::new_owner_session_state(req.owner, session_id, now_ms);
        state.registered = true;
        state.origin = Some(req.origin.unwrap_or_else(|| "manual".to_string()));
        state.peer_did = Some(req.peer_did);
        state.binding = req.binding;
        state.title = title;
        self.msg_box_db.upsert_owner_session(&state).await?;
        Ok(state)
    }

    async fn load_or_default_state(
        &self,
        owner: &DID,
        session_id: &str,
    ) -> std::result::Result<OwnerSessionState, RPCErrors> {
        if let Some(state) = self.msg_box_db.get_owner_session(owner, session_id).await? {
            return Ok(state);
        }
        // Lifecycle rows are created lazily; a session only known from the
        // mailbox index must have at least one visible record.
        let has_records = !self
            .msg_box_db
            .list_visible_session_records(owner, session_id, 1, None, None, true, None)
            .await?
            .is_empty();
        if !has_records {
            return Err(RPCErrors::ReasonError(format!(
                "session {} not found for owner {}",
                session_id,
                owner.to_string()
            )));
        }
        Ok(Self::new_owner_session_state(
            owner.clone(),
            session_id.to_string(),
            Self::now_ms(),
        ))
    }

    pub(crate) async fn set_session_lifecycle_internal(
        &self,
        owner: DID,
        session_id: String,
        lifecycle: SessionLifecycle,
    ) -> std::result::Result<OwnerSessionState, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let mut state = self.load_or_default_state(&owner, &session_id).await?;
        if state.lifecycle == lifecycle {
            return Ok(state);
        }
        let now_ms = Self::now_ms();
        state.lifecycle = lifecycle;
        state.archived_at_ms = match lifecycle {
            SessionLifecycle::Archived => Some(now_ms),
            SessionLifecycle::Active => None,
        };
        state.updated_at_ms = now_ms;
        self.msg_box_db.upsert_owner_session(&state).await?;
        Ok(state)
    }

    /// Delete for this owner only: move the visibility watermark past every
    /// record that exists now, drop registration and the owner's UI state.
    /// Per-record `RecipientState`, other owners' references and the shared
    /// `MsgObject`s are untouched.
    pub(crate) async fn delete_session_internal(
        &self,
        owner: DID,
        session_id: String,
    ) -> std::result::Result<OwnerSessionState, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let now_ms = Self::now_ms();
        let mut state = match self
            .msg_box_db
            .get_owner_session(&owner, &session_id)
            .await?
        {
            Some(state) => state,
            None => Self::new_owner_session_state(owner.clone(), session_id.clone(), now_ms),
        };
        let (max_sort_key, max_record_id) = self
            .msg_box_db
            .session_max_record_key(&owner, &session_id)
            .await?
            .unwrap_or((0, String::new()));
        // Records are keyed by the message creation time; a replay of an old
        // message carries the same sort_key and therefore stays hidden.
        let watermark_sort_key = max_sort_key.max(now_ms);
        let watermark_record_id = if watermark_sort_key == max_sort_key {
            max_record_id
        } else {
            String::new()
        };
        state.registered = false;
        state.lifecycle = SessionLifecycle::Active;
        state.archived_at_ms = None;
        state.delete_watermark_sort_key = Some(watermark_sort_key);
        state.delete_watermark_record_id = Some(watermark_record_id);
        state.deleted_at_ms = Some(now_ms);
        state.title = None;
        state.updated_at_ms = now_ms;
        self.msg_box_db.upsert_owner_session(&state).await?;
        self.msg_box_db
            .delete_owner_ui_session_states(&owner, &session_id)
            .await?;
        Ok(state)
    }

    pub(crate) async fn get_session_state_internal(
        &self,
        owner: DID,
        session_id: String,
    ) -> std::result::Result<Option<OwnerSessionState>, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        self.msg_box_db.get_owner_session(&owner, &session_id).await
    }

    /// Called after mailbox records are committed: an effective new message
    /// (chat / group message / deliver) newer than the delete watermark
    /// re-activates an archived session. Runtime state and action logs never
    /// reach this path with an activity kind, so they cannot un-archive.
    pub(crate) async fn note_records_committed(&self, records: &[MailboxRecord]) {
        for record in records {
            if !is_activity_kind(record.msg_kind) {
                continue;
            }
            let Some(session_id) = record.session_id.as_deref() else {
                continue;
            };
            let state = match self
                .msg_box_db
                .get_owner_session(&record.owner, session_id)
                .await
            {
                Ok(Some(state)) => state,
                Ok(None) => continue,
                Err(error) => {
                    warn!(
                        "msg-center lifecycle lookup failed for {}/{}: {}",
                        record.owner.to_string(),
                        session_id,
                        error
                    );
                    continue;
                }
            };
            if state.lifecycle != SessionLifecycle::Archived {
                continue;
            }
            if let Some(watermark) = DeleteWatermark::from_state(&state) {
                if !watermark.allows(record) {
                    continue;
                }
            }
            let now_ms = Self::now_ms();
            let next = OwnerSessionState {
                lifecycle: SessionLifecycle::Active,
                archived_at_ms: None,
                updated_at_ms: now_ms,
                ..state
            };
            if let Err(error) = self.msg_box_db.upsert_owner_session(&next).await {
                warn!(
                    "msg-center failed to re-activate archived session {}/{}: {}",
                    record.owner.to_string(),
                    session_id,
                    error
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // Projection
    // ------------------------------------------------------------------

    fn clamp_limit_value(limit: Option<usize>, default: usize, max: usize) -> usize {
        limit.unwrap_or(default).clamp(1, max)
    }

    pub(crate) async fn list_sessions_scoped(
        &self,
        owner: DID,
        limit: Option<usize>,
        cursor_value: Option<u64>,
        cursor_session_id: Option<String>,
        with_object: Option<bool>,
        lifecycle: Option<SessionListLifecycleFilter>,
        order_by: Option<SessionListOrder>,
    ) -> std::result::Result<SessionSummaryPage, RPCErrors> {
        let limit =
            Self::clamp_limit_value(limit, DEFAULT_SESSION_LIST_LIMIT, MAX_SESSION_LIST_LIMIT);
        let lifecycle = lifecycle.unwrap_or_default();
        let order_by = order_by.unwrap_or_default();

        let states: HashMap<String, OwnerSessionState> = self
            .msg_box_db
            .list_owner_sessions(&owner)
            .await?
            .into_iter()
            .map(|state| (state.session_id.clone(), state))
            .collect();
        let mut entries: HashMap<String, VisibleSessionIndexEntry> = self
            .msg_box_db
            .list_visible_session_index(&owner)
            .await?
            .into_iter()
            .map(|entry| (entry.session_id.clone(), entry))
            .collect();
        // Registered empty sessions are listed with their registration time
        // as activity so the UI can show "created, no messages yet".
        for state in states.values() {
            if state.registered && !entries.contains_key(&state.session_id) {
                entries.insert(
                    state.session_id.clone(),
                    VisibleSessionIndexEntry {
                        session_id: state.session_id.clone(),
                        updated_at_ms: state.updated_at_ms,
                        last_activity_ms: state.created_at_ms,
                        unread_count: 0,
                        request_count: 0,
                    },
                );
            }
        }

        let mut rows: Vec<VisibleSessionIndexEntry> = entries
            .into_values()
            .filter(|entry| {
                let session_lifecycle = states
                    .get(&entry.session_id)
                    .map(|state| state.lifecycle)
                    .unwrap_or_default();
                match lifecycle {
                    SessionListLifecycleFilter::Active => {
                        session_lifecycle == SessionLifecycle::Active
                    }
                    SessionListLifecycleFilter::Archived => {
                        session_lifecycle == SessionLifecycle::Archived
                    }
                    SessionListLifecycleFilter::All => true,
                }
            })
            .collect();
        let sort_value = |entry: &VisibleSessionIndexEntry| match order_by {
            SessionListOrder::Updated => entry.updated_at_ms,
            SessionListOrder::Activity => entry.last_activity_ms,
        };
        rows.sort_by(|a, b| {
            sort_value(b)
                .cmp(&sort_value(a))
                .then_with(|| b.session_id.cmp(&a.session_id))
        });
        if let Some(cursor_value) = cursor_value {
            let cursor_session_id = cursor_session_id.unwrap_or_default();
            rows.retain(|entry| {
                let value = sort_value(entry);
                value < cursor_value
                    || (value == cursor_value && entry.session_id < cursor_session_id)
            });
        }
        let has_more = rows.len() > limit;
        let page_entries = rows.into_iter().take(limit).collect::<Vec<_>>();

        let mut items = Vec::with_capacity(page_entries.len());
        for entry in page_entries {
            let state = states.get(&entry.session_id).cloned();
            let watermark = state.as_ref().and_then(DeleteWatermark::from_state);
            let last_record = self
                .msg_box_db
                .list_visible_session_records(
                    &owner,
                    &entry.session_id,
                    1,
                    None,
                    None,
                    true,
                    watermark.as_ref(),
                )
                .await?
                .into_iter()
                .next();
            let last_record = match last_record {
                Some(record) => Some(
                    self.build_record_view(record, Some(with_object.unwrap_or(false)))
                        .await?,
                ),
                None => None,
            };
            items.push(SessionSummary {
                session_id: entry.session_id,
                last_record,
                unread_count: entry.unread_count,
                updated_at_ms: entry.updated_at_ms,
                last_activity_ms: entry.last_activity_ms,
                request_count: entry.request_count,
                lifecycle: state.as_ref().map(|s| s.lifecycle).unwrap_or_default(),
                state,
            });
        }

        let (next_cursor_updated_at_ms, next_cursor_session_id) = if has_more {
            items
                .last()
                .map(|item| {
                    let value = match order_by {
                        SessionListOrder::Updated => item.updated_at_ms,
                        SessionListOrder::Activity => item.last_activity_ms,
                    };
                    (Some(value), Some(item.session_id.clone()))
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        Ok(SessionSummaryPage {
            items,
            next_cursor_updated_at_ms,
            next_cursor_session_id,
        })
    }

    pub(crate) async fn list_session_scoped(
        &self,
        owner: DID,
        session_id: String,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        with_object: Option<bool>,
    ) -> std::result::Result<SessionMessagePage, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let descending = descending.unwrap_or(true);
        let limit = Self::clamp_limit_value(limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT);
        let watermark = self
            .msg_box_db
            .get_owner_session(&owner, &session_id)
            .await?
            .as_ref()
            .and_then(DeleteWatermark::from_state);
        let records = self
            .msg_box_db
            .list_visible_session_records(
                &owner,
                &session_id,
                limit + 1,
                cursor_sort_key,
                cursor_record_id.as_deref(),
                descending,
                watermark.as_ref(),
            )
            .await?;
        let has_more = records.len() > limit;
        let page_records = records.into_iter().take(limit).collect::<Vec<_>>();

        let mut items = Vec::with_capacity(page_records.len());
        for record in page_records {
            items.push(
                self.build_session_item(record, with_object.unwrap_or(false))
                    .await?,
            );
        }

        let (next_cursor_sort_key, next_cursor_record_id) = if has_more {
            items
                .last()
                .map(|item| (Some(item.sort_key), Some(item.record_id.clone())))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        Ok(SessionMessagePage {
            items,
            next_cursor_sort_key,
            next_cursor_record_id,
        })
    }

    // ------------------------------------------------------------------
    // Owner-scoped UI state
    // ------------------------------------------------------------------

    pub(crate) async fn update_owner_ui_session_state_internal(
        &self,
        owner: DID,
        session_id: String,
        key: String,
        value: Value,
    ) -> std::result::Result<UiSessionStateEntry, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let key = normalize_session_id(&key)?;
        self.msg_box_db
            .upsert_owner_ui_session_state(&owner, &session_id, &key, &value, Self::now_ms())
            .await
    }

    pub(crate) async fn get_owner_ui_session_state_internal(
        &self,
        owner: DID,
        session_id: String,
        key: String,
    ) -> std::result::Result<Option<UiSessionStateEntry>, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let key = normalize_session_id(&key)?;
        self.msg_box_db
            .get_owner_ui_session_state(&owner, &session_id, &key)
            .await
    }

    pub(crate) async fn list_owner_ui_session_state_internal(
        &self,
        owner: DID,
        session_id: String,
    ) -> std::result::Result<Vec<UiSessionStateEntry>, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        self.msg_box_db
            .list_owner_ui_session_state(&owner, &session_id)
            .await
    }
}
