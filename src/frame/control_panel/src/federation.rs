//! DID login (SSO-09, contract §9). Target side: `auth.did.begin` sends the
//! browser to the DID's canonical zone with a PKCE-bound pending login, and
//! `GET /sso/did_callback` turns the returned code into a session through
//! verify-hub. Canonical side: `auth.federation.*` shows the consent to the
//! signed-in owner of the DID and asks verify-hub for the one-time code.

use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::exclusive_invite::random_token;
use crate::sso_settings::load_sso_settings;
use crate::sys_auth_backend::ResolvedSsoAuthTarget;
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buckyos_api::{
    get_buckyos_api_runtime, owner_canonical_zone, zone_document_hostname, AccessDenial,
    AuthTarget, DenyReason, FederationProfile, IssueFederationCodeRequest,
    LoginByRemoteVerifyHubRequest, LoginByRemoteVerifyHubResult, SsoSettings, UserPrivateProfile,
    UserSettings, CONTROL_PANEL_SERVICE_UNIQUE_ID,
};
use buckyos_http_server::{server_err, ServerError, ServerErrorCode, ServerResult};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use bytes::Bytes;
use http::header::{CACHE_CONTROL, LOCATION, SET_COOKIE};
use http_body_util::combinators::BoxBody;
use log::{info, warn};
use name_lib::{DIDDocumentTrait, DidDocType, OwnerDocument, ZoneDocument, DID};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

pub(crate) const DID_CALLBACK_PATH: &str = "/sso/did_callback";
const PENDING_DID_LOGIN_TTL_SECS: u64 = 600;
/// Live pending DID logins kept at most; beyond it new logins are refused.
const MAX_PENDING_DID_LOGINS: usize = 4096;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
/// Q-05: DID, both zones, public display name and avatar.
const PROFILE_SCOPE: u32 = 1;
const MAX_FEDERATION_PARAM_LEN: usize = 256;

/// A DID login begun here, waiting for the canonical zone's code (10 minutes,
/// in memory, taken once by `state`).
#[derive(Clone, Debug)]
pub(crate) struct PendingDidLogin {
    did: String,
    code_verifier: String,
    nonce: String,
    redirect_uri: String,
    target: ResolvedSsoAuthTarget,
    created_at: u64,
    invitation: Option<(String,String)>,
    outbound: Option<String>,
    browser: String,
}

/// The name-service facts DID login relies on.
#[async_trait]
pub(crate) trait ZoneDirectory: Send + Sync {
    /// Hostname of the canonical (default) zone a person's DID names.
    async fn canonical_host(&self, did: &DID) -> Result<String, RPCErrors>;
    /// Hostname a Zone DID publishes.
    async fn zone_host(&self, zone_did: &DID) -> Result<String, RPCErrors>;
}

pub(crate) struct NameServiceDirectory;

pub(super) async fn resolve(did: &DID, doc_type: DidDocType) -> Result<name_lib::EncodedDocument, RPCErrors> {
    tokio::time::timeout(
        RESOLVE_TIMEOUT,
        name_client::resolve_did(did, Some(doc_type)),
    )
    .await
    .map_err(|_| RPCErrors::ReasonError(format!("resolving {} timed out", did.to_string())))?
    .map_err(|error| RPCErrors::ReasonError(format!("resolve {}: {error}", did.to_string())))
}

#[async_trait]
impl ZoneDirectory for NameServiceDirectory {
    async fn canonical_host(&self, did: &DID) -> Result<String, RPCErrors> {
        let owner = buckyos_api::resolve_federation_owner(did).await
            .map_err(|err| RPCErrors::ReasonError(format!("resolve {did:?}: {err}")))?;
        self.zone_host(&owner_canonical_zone(&owner)).await
    }

    async fn zone_host(&self, zone_did: &DID) -> Result<String, RPCErrors> {
        let zone = ZoneDocument::decode(&resolve(zone_did, DidDocType::Zone).await?, None)
            .map_err(|error| {
                RPCErrors::ReasonError(format!("zone document {}: {error}", zone_did.to_string()))
            })?;
        if &zone.id != zone_did {
            return Err(RPCErrors::InvalidToken(
                "ZoneDocument does not match requested Zone".into(),
            ));
        }
        let host = zone_document_hostname(&zone)
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if host.is_empty() {
            return Err(RPCErrors::ReasonError(format!(
                "{} publishes no hostname",
                zone_did.to_string()
            )));
        }
        Ok(host)
    }
}

fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The canonical zone's consent page for this login, and the pending login
/// that keeps the PKCE verifier on this side.
fn plan_did_login(
    did: &DID,
    canonical_host: &str,
    client_zone: &DID,
    zone_origin: &str,
    target: ResolvedSsoAuthTarget,
    http: bool,
    now: u64,
) -> Result<(String, PendingDidLogin, String), RPCErrors> {
    let state = random_token(32);
    let nonce = random_token(16);
    let code_verifier = random_token(32);
    let redirect_uri = format!("{zone_origin}{DID_CALLBACK_PATH}");
    let scheme = if http { "http" } else { "https" };
    let mut url = url::Url::parse(&format!("{scheme}://{canonical_host}/sso/federate"))
        .map_err(|error| RPCErrors::ReasonError(format!("canonical host: {error}")))?;
    url.query_pairs_mut()
        .append_pair("did", &did.to_string())
        .append_pair("client_zone", &client_zone.to_string())
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("state", &state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &code_challenge(&code_verifier));
    let pending = PendingDidLogin {
        did: did.to_string(),
        code_verifier,
        nonce,
        redirect_uri,
        target,
        created_at: now, invitation: None, outbound: None, browser: random_token(32),
    };
    Ok((state, pending, url.to_string()))
}

/// `did_error` of a callback whose browser already holds another subject's session.
const IDENTITY_CONFLICT: &str = "identity_conflict";

/// Whether a session cookie the browser holds speaks for someone else.
fn conflicting_subject(held: &[String], issued: &str) -> bool {
    held.iter().any(|subject| subject != issued)
}

/// Where a failed DID callback sends the browser: the sign-in page with the
/// reason (a D-4 reason code, `canonical_unavailable`, or `access_denied` when the
/// visitor declined at the canonical zone) and the original page when it is
/// known. A stale state, a missing code or a callback on another origin read
/// `authorization_unavailable`; a browser signed in as someone else reads
/// `identity_conflict`. No cookie is set.
pub(crate) fn did_error_location(reason: &str, redirect_url: Option<&str>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("did_error", reason);
    if let Some(redirect_url) = redirect_url {
        query.append_pair("redirect_url", redirect_url);
    }
    format!("/login?{}", query.finish())
}

/// Keep a verified invitation attempt on its own page so retry starts the same purpose.
pub(crate) fn callback_error_location(reason: &str, redirect_url: &str, invitation: bool) -> String {
    if invitation {
        if let Ok(mut url) = url::Url::parse(redirect_url) {
            let error = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("did_error", reason)
                .finish();
            let mut query: Vec<_> = url.query().into_iter().flat_map(|raw| raw.split('&'))
                .filter(|pair| !url::form_urlencoded::parse(pair.as_bytes()).any(|(key, _)| key == "did_error"))
                .collect();
            query.push(&error);
            url.set_query(Some(&query.join("&")));
            return url.into();
        }
    }
    did_error_location(reason, Some(redirect_url))
}

/// `redirect_uri` must be the DID callback on the host `client_zone` publishes;
/// plain http only where the Zone's development setting allows it.
fn validate_redirect_uri(
    redirect_uri: &str,
    client_host: &str,
    allow_http: bool,
) -> Result<url::Url, RPCErrors> {
    let invalid = || {
        RPCErrors::ParseRequestError(
            "redirect_uri is not the client zone's DID callback".to_string(),
        )
    };
    let url = url::Url::parse(redirect_uri).map_err(|_| invalid())?;
    let scheme_ok = match url.scheme() {
        "https" => url.port().is_none(),
        "http" => allow_http,
        _ => false,
    };
    let host_ok = url
        .host_str()
        .is_some_and(|host| host.trim_end_matches('.').eq_ignore_ascii_case(client_host));
    if !scheme_ok
        || !host_ok
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != DID_CALLBACK_PATH
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct FederationConsent {
    granted_at: u64,
    profile_scope: u32,
}

/// `users/<id>/federation_consents`: client zone DID → consent.
type FederationConsents = BTreeMap<String, FederationConsent>;

fn consents_key(user_id: &str) -> String {
    format!("users/{user_id}/federation_app_consents")
}

/// The canonical account as the consent checks see it.
struct FederationAccount {
    settings: UserSettings,
    owner: OwnerDocument,
    profile: Option<UserPrivateProfile>,
}

impl FederationAccount {
    fn profile(&self) -> FederationProfile {
        let show_name = self
            .profile
            .as_ref()
            .and_then(|profile| {
                profile
                    .display_name
                    .clone()
                    .or_else(|| profile.name.clone())
            })
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.settings.user_id.clone());
        FederationProfile {
            show_name,
            avatar: self
                .profile
                .as_ref()
                .and_then(|profile| profile.avatar.clone()),
        }
    }
}

async fn load_federation_account(
    kv: &dyn AgentKv,
    user_id: &str,
) -> Result<FederationAccount, RPCErrors> {
    let read = |key: String| async move {
        kv.get(&key)
            .await?
            .map(|(raw, _)| raw)
            .ok_or_else(|| RPCErrors::ReasonError(format!("`{key}` is missing")))
    };
    let settings = serde_json::from_str(&read(format!("users/{user_id}/settings")).await?)
        .map_err(|error| RPCErrors::ReasonError(format!("account {user_id}: {error}")))?;
    let owner = serde_json::from_str(&read(format!("users/{user_id}/doc")).await?)
        .map_err(|error| RPCErrors::ReasonError(format!("DID document of {user_id}: {error}")))?;
    let profile = match kv.get(&format!("users/{user_id}/profile")).await? {
        Some((raw, _)) => serde_json::from_str(&raw).ok(),
        None => None,
    };
    Ok(FederationAccount {
        settings,
        owner,
        profile,
    })
}

/// D-7 / T-6: only the signed-in Admin or User owning the DID, whose canonical
/// zone is this one, vouches for it.
fn federation_check(
    principal: &RpcAuthPrincipal,
    did: &str,
    account: &FederationAccount,
    this_zone: &DID,
) -> Result<(), AccessDenial> {
    if principal.is_restricted() || !principal.is_control_panel_session {
        return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed));
    }
    if !account.settings.is_active() {
        return Err(AccessDenial::new(DenyReason::AccountState));
    }
    if !account.settings.user_type.can_federate_out() {
        return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed).with_detail("user_type"));
    }
    if account.owner.id.to_string() != did {
        return Err(
            AccessDenial::new(DenyReason::SessionCeilingExceeded).with_detail("did_mismatch")
        );
    }
    if owner_canonical_zone(&account.owner) != *this_zone {
        return Err(
            AccessDenial::new(DenyReason::SessionKindNotAllowed).with_detail("not_canonical_zone")
        );
    }
    Ok(())
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(super) struct FederationParams {
    #[serde(default)]
    pub(super) did: String,
    pub(super) client_zone: String,
    pub(super) redirect_uri: String,
    pub(super) state: String,
    #[serde(default)]
    pub(super) nonce: String,
    #[serde(default)]
    pub(super) code_challenge: String,
    #[serde(default)]
    pub(super) remember: bool,
    #[serde(default)]
    pub(super) app_context: String,
}

impl FederationParams {
    fn require_login_fields(&self) -> Result<(), RPCErrors> {
        for (name, value) in [
            ("did", &self.did),
            ("state", &self.state),
            ("nonce", &self.nonce),
            ("code_challenge", &self.code_challenge),
        ] {
            if value.is_empty() || value.len() > MAX_FEDERATION_PARAM_LEN {
                return Err(RPCErrors::ParseRequestError(format!("invalid {name}")));
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum DidBeginResult {
    Redirect { url: String },
    Denied { denial: AccessDenial },
    CanonicalUnavailable,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum FederationResult {
    Consent {
        client_zone: String,
        client_host: String,
        app_host: String,
        profile: FederationProfile,
        already_granted: bool,
    },
    Redirect {
        url: String,
    },
    Denied {
        denial: AccessDenial,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DidBeginParams {
    purpose: Option<String>,
    invite_id: Option<String>,
    did: String,
    redirect_url: String,
}

fn account_missing() -> RPCErrors {
    RPCErrors::ReasonError("consent context has no account".to_string())
}

fn success(value: impl Serialize, seq: u64) -> Result<RPCResponse, RPCErrors> {
    Ok(RPCResponse::new(RPCResult::Success(json!(value)), seq))
}

/// What the consent page needs once every check passed; the account is read
/// for describe / approve only.
struct FederationContext {
    account: Option<FederationAccount>,
    app: AuthTarget,
    app_host: String,
    client_zone: DID,
    client_host: String,
    redirect_uri: url::Url,
    settings: SsoSettings,
}

/// False when the cache already holds the most live logins it keeps.
fn store_pending(
    cache: &mut HashMap<String, PendingDidLogin>,
    state: String,
    pending: PendingDidLogin,
    now: u64,
) -> bool {
    cache.retain(|_, value| now.saturating_sub(value.created_at) <= PENDING_DID_LOGIN_TTL_SECS);
    if cache.len() >= MAX_PENDING_DID_LOGINS {
        return false;
    }
    cache.insert(state, pending);
    true
}

/// Single use: the entry leaves the cache whether or not it is still fresh.
fn take_pending(
    cache: &mut HashMap<String, PendingDidLogin>,
    state: &str,
    now: u64,
) -> Option<PendingDidLogin> {
    let pending = cache.remove(state)?;
    (now.saturating_sub(pending.created_at) <= PENDING_DID_LOGIN_TTL_SECS).then_some(pending)
}

impl ControlPanelServer {
    async fn store_pending_did_login(&self, state: String, pending: PendingDidLogin) -> bool {
        let mut cache = self.pending_did_logins.lock().await;
        store_pending(&mut cache, state, pending, buckyos_get_unix_timestamp())
    }

    /// `auth.did.begin {did, redirect_url}` (public) → `{status:"redirect", url}` |
    /// `{status:"denied", denial}` | `{status:"canonical_unavailable"}`. `redirect_url`
    /// is the page on this Zone's root to return to after the login.
    pub(crate) async fn handle_auth_did_begin(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let params: DidBeginParams = Self::typed_params(&req)?;
        let settings = load_sso_settings(&ZoneKv::connect().await?).await?;
        if !settings.did_login_enabled {
            return success(
                DidBeginResult::Denied {
                    denial: AccessDenial::new(DenyReason::ZoneMethodDisabled)
                        .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "auth.did.begin"),
                },
                req.seq,
            );
        }
        if buckyos_api::external_source_for_did(&settings, params.did.trim()).is_some() {
            return Err(RPCErrors::ParseRequestError(
                "Choose the enabled OAuth source for this identity".into(),
            ));
        }
        let did = DID::from_str(params.did.trim())
            .map_err(|error| RPCErrors::ParseRequestError(format!("invalid did: {error}")))?;
        let runtime = get_buckyos_api_runtime()?;
        let zone_host = Self::sso_zone_host(runtime)?;
        let target = Self::resolve_sso_auth_target(
            &params.redirect_url,
            &zone_host,
            !runtime.force_https,
            runtime.node_gateway_port,
        )?;
        if !matches!(&target.auth_target, AuthTarget::System { service_id }
            if service_id.as_str() == CONTROL_PANEL_SERVICE_UNIQUE_ID)
        {
            return Err(RPCErrors::ParseRequestError(
                "DID login returns to a page on the Zone root".to_string(),
            ));
        }
        let canonical_host = match NameServiceDirectory.canonical_host(&did).await {
            Ok(host) => host,
            Err(error) => {
                warn!(
                    "DID login cannot locate the canonical zone of {}: {error}",
                    did.to_string()
                );
                return success(DidBeginResult::CanonicalUnavailable, req.seq);
            }
        };
        let (state, mut pending, url) = plan_did_login(
            &did,
            &canonical_host,
            &runtime.zone_id,
            &Self::zone_root_origin(&req, &zone_host),
            target,
            settings.dev_federation_http,
            buckyos_get_unix_timestamp(),
        )?;
        let return_url = url::Url::parse(&pending.target.canonical_redirect_url)
            .map_err(|_| RPCErrors::ParseRequestError("invalid return URL".into()))?;
        let (app, app_host) = match (params.purpose.as_deref(), params.invite_id.as_deref()) {
            (Some("share"), Some(id)) if return_url.path() == format!("/sso/share/{id}") => {
                let share =
                    crate::resource_share::load_share(&ZoneKv::connect().await?, id).await?;
                if matches!(&share.audience, buckyos_api::ShareAudience::Directed { did: recipient } if recipient != &pending.did)
                {
                    return Err(
                        AccessDenial::new(DenyReason::ResourceGrantMissing).into_rpc_error()
                    );
                }
                let resolved = Self::resolve_sso_auth_target(
                    &share.target_url,
                    &zone_host,
                    !runtime.force_https,
                    runtime.node_gateway_port,
                )?;
                if resolved.auth_target != share.target {
                    return Err(AccessDenial::new(DenyReason::EntryUnavailable).into_rpc_error());
                }
                let host = url::Url::parse(&resolved.canonical_origin)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .ok_or_else(|| RPCErrors::ReasonError("missing share host".into()))?;
                pending.invitation = Some(("share".into(), id.into()));
                (share.target, host)
            }
            (Some("user_invite"), Some(id))
                if return_url.path() == format!("/users/invite/{id}") =>
            {
                pending.invitation = Some(("user_invite".into(), id.into()));
                (pending.target.auth_target.clone(), zone_host.clone())
            }
            (None | Some("login"), None) => (pending.target.auth_target.clone(), zone_host.clone()),
            _ => {
                return Err(RPCErrors::ParseRequestError(
                    "invitation purpose and return page must match".into(),
                ))
            }
        };
        let signer = runtime
            .device_signer()
            .ok_or_else(|| RPCErrors::ReasonError("federation signing key unavailable".into()))?;
        let fields = FederationParams {
            did: pending.did.clone(),
            client_zone: runtime.zone_id.to_string(),
            redirect_uri: pending.redirect_uri.clone(),
            state: state.clone(),
            nonce: pending.nonce.clone(),
            code_challenge: code_challenge(&pending.code_verifier),
            remember: false,
            app_context: String::new(),
        };
        let signed = crate::federation_context::sign(
            &signer,
            &fields,
            app,
            app_host,
            buckyos_get_unix_timestamp(),
        )?;
        let mut outbound =
            url::Url::parse(&url).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        outbound
            .query_pairs_mut()
            .append_pair("app_context", &signed);
        pending.outbound = Some(outbound.to_string());
        let url = format!(
            "{}/sso/did_start?state={}",
            Self::zone_root_origin(&req, &zone_host),
            state
        );
        if !self.store_pending_did_login(state, pending).await {
            warn!("DID login refused: too many pending DID logins");
            return success(
                DidBeginResult::Denied {
                    denial: AccessDenial::new(DenyReason::AuthorizationUnavailable)
                        .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "auth.did.begin")
                        .with_detail("rate_limited"),
                },
                req.seq,
            );
        }
        info!("DID login of {} sent to {canonical_host}", did.to_string());
        success(DidBeginResult::Redirect { url }, req.seq)
    }

    pub(crate) async fn serve_did_start(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let state = Self::http_query_param(&req, "state").unwrap_or_default();
        let mut logins = self.pending_did_logins.lock().await;
        let runtime = get_buckyos_api_runtime().map_err(Self::rpc_to_server_error)?;
        let origin = Self::request_origin(&req, !runtime.force_https, runtime.node_gateway_port)
            .map_err(Self::rpc_to_server_error)?;
        if let Some(p) = logins.get_mut(&state).filter(|p| {
            p.target.canonical_origin == origin
                && buckyos_get_unix_timestamp().saturating_sub(p.created_at)
                    < PENDING_DID_LOGIN_TTL_SECS
        }) {
            if let Some(url) = p.outbound.take() {
                let secure = if origin.starts_with("https://") {
                    "; Secure"
                } else {
                    ""
                };
                let name = did_browser_cookie(&state, &origin);
                let cookie = format!("{name}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={PENDING_DID_LOGIN_TTL_SECS}{secure}",p.browser);
                return Self::did_redirect(&url, vec![cookie]);
            }
        }
        Self::did_redirect(
            &did_error_location("authorization_unavailable", None),
            vec![],
        )
    }

    pub(crate) fn did_redirect(
        location: &str,
        cookies: Vec<String>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let mut response = http::Response::builder()
            .status(http::StatusCode::FOUND)
            .header(CACHE_CONTROL, "no-store")
            .header(http::header::REFERRER_POLICY, "no-referrer")
            .header(LOCATION, location);
        for cookie in cookies {
            response = response.header(SET_COOKIE, cookie);
        }
        response
            .body(Self::boxed_http_body(Vec::new()))
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build DID callback redirect: {}",
                    error
                )
            })
    }

    /// `GET /sso/did_callback?code=..&state=..` (or `?error=access_denied&state=..`):
    /// sets the Zone root cookies and returns to the page the login began on, or
    /// sends the browser to `/login?did_error=<reason>&redirect_url=<page>`
    /// (see [`did_error_location`]).
    pub(crate) async fn serve_did_callback(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let state = Self::http_query_param(&req, "state").unwrap_or_default();
        let pending = {
            let mut cache = self.pending_did_logins.lock().await;
            if cache
                .get(&state)
                .is_some_and(|p| p.outbound.is_none() && Self::extract_http_cookie(&req, &did_browser_cookie(&state, &p.target.canonical_origin)).as_deref() == Some(&p.browser))
            {
                take_pending(&mut cache, &state, buckyos_get_unix_timestamp())
            } else {
                None
            }
        };
        let Some(pending) = pending else {
            return Self::did_redirect(
                &did_error_location(DenyReason::AuthorizationUnavailable.as_str(), None),
                Vec::new(),
            );
        };
        let fail = |reason: &str| {
            Self::did_redirect(
                &callback_error_location(reason, &pending.target.canonical_redirect_url, pending.invitation.is_some()),
                Vec::new(),
            )
        };
        if Self::http_query_param(&req, "error").is_some() {
            return fail("access_denied");
        }
        let Some(code) = Self::http_query_param(&req, "code") else {
            return fail(DenyReason::AuthorizationUnavailable.as_str());
        };
        let runtime = get_buckyos_api_runtime().map_err(Self::rpc_to_server_error)?;
        let same_origin =
            Self::request_origin(&req, !runtime.force_https, runtime.node_gateway_port)
                .is_ok_and(|origin| origin == pending.target.canonical_origin);
        if !same_origin {
            return fail(DenyReason::AuthorizationUnavailable.as_str());
        }
        let verify_hub = runtime
            .get_verify_hub_client()
            .await
            .map_err(Self::rpc_to_server_error)?;
        let result = verify_hub
            .login_by_remote_verify_hub(LoginByRemoteVerifyHubRequest {
                proof_only: pending.invitation.is_some(),
                did: pending.did.clone(),
                code,
                code_verifier: pending.code_verifier.clone(),
                nonce: pending.nonce.clone(),
                redirect_uri: pending.redirect_uri.clone(),
                target: pending.target.auth_target.clone(),
            })
            .await;
        let (user_id, token_pair, profile) = match result {
            Ok(LoginByRemoteVerifyHubResult::Verified {
                did,
                user_id,
                profile,
            }) => {
                let Some((purpose, id)) = pending.invitation.as_ref() else {
                    return fail("authorization_unavailable");
                };
                if did != pending.did
                    || conflicting_subject(
                        &Self::sso_cookie_subjects(&req),
                        user_id.as_deref().unwrap_or(&did),
                    )
                {
                    return fail(IDENTITY_CONFLICT);
                }
                self.remember_external_profile(&did, profile);
                let cookie = self
                    .save_native_invite_identity(purpose, id, did, &pending.target.canonical_origin)
                    .await
                    .map_err(Self::rpc_to_server_error)?;
                return Self::did_redirect(&pending.target.canonical_redirect_url, vec![cookie]);
            }
            Ok(LoginByRemoteVerifyHubResult::Issued {
                user_id,
                token_pair,
                profile,
            }) => (user_id, token_pair, profile),
            Ok(LoginByRemoteVerifyHubResult::Denied { denial }) => {
                return fail(denial.reason.as_str());
            }
            Ok(LoginByRemoteVerifyHubResult::CanonicalUnavailable) => {
                return fail("canonical_unavailable");
            }
            Err(error) => {
                warn!("DID login of {} failed: {error}", pending.did);
                return fail(DenyReason::AuthorizationUnavailable.as_str());
            }
        };
        let expected_subject = user_id.clone().unwrap_or_else(|| pending.did.clone());
        let checked = Self::validate_sso_token_pair(
            &token_pair.session_token,
            &token_pair.refresh_token,
            &pending.target.auth_target,
        )
        .and_then(|_| RPCSessionToken::from_string(&token_pair.session_token))
        .is_ok_and(|session| session.sub.as_deref() == Some(expected_subject.as_str()));
        if !checked {
            let _ = verify_hub.logout(&token_pair.refresh_token).await;
            return fail(DenyReason::AuthorizationUnavailable.as_str());
        }
        // A callback URL someone else completed must not replace the browser's own
        // session (login swap): refuse when a different subject is signed in here.
        let held = Self::sso_cookie_subjects(&req);
        if conflicting_subject(&held, &expected_subject) {
            let _ = verify_hub.logout(&token_pair.refresh_token).await;
            warn!(
                "DID login of {} refused: the browser holds another session",
                pending.did
            );
            return fail(IDENTITY_CONFLICT);
        }
        if user_id.is_none() {
            self.remember_external_profile(&pending.did, profile);
        }
        info!(
            "DID login of {} delivered as {}",
            pending.did, expected_subject
        );
        let cookies = Self::build_sso_cookie_headers(
            &req,
            &token_pair.session_token,
            &token_pair.refresh_token,
        )?;
        Self::did_redirect(&pending.target.canonical_redirect_url, cookies)
    }

    /// Runs every canonical-side check; a refusal the page shows is `Ok(Err)`.
    async fn federation_context(
        principal: &RpcAuthPrincipal,
        params: &FederationParams,
        login: bool,
    ) -> Result<Result<FederationContext, AccessDenial>, RPCErrors> {
        if login {
            params.require_login_fields()?;
        } else if params.state.is_empty() || params.state.len() > MAX_FEDERATION_PARAM_LEN {
            return Err(RPCErrors::ParseRequestError("invalid state".to_string()));
        }
        let client_zone = DID::from_str(&params.client_zone).map_err(|error| {
            RPCErrors::ParseRequestError(format!("invalid client_zone: {error}"))
        })?;
        let runtime = get_buckyos_api_runtime()?;
        let kv = ZoneKv::connect().await?;
        let settings = load_sso_settings(&kv).await?;
        if buckyos_api::external_source_for_did(&settings, &params.did).is_some() {
            return Ok(Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)));
        }
        let account = if login {
            if principal.is_restricted() {
                return Ok(Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)));
            }
            let account = load_federation_account(&kv, &principal.username).await?;
            if let Err(denial) =
                federation_check(principal, &params.did, &account, &runtime.zone_id)
            {
                return Ok(Err(denial));
            }
            Some(account)
        } else {
            None
        };
        let encoded = resolve(&client_zone, DidDocType::Zone).await?;
        let mut doc = ZoneDocument::decode(&encoded, None)
            .map_err(|e| RPCErrors::InvalidToken(e.to_string()))?;
        let client_host = zone_document_hostname(&doc)
            .trim_end_matches('.')
            .to_ascii_lowercase();
        crate::federation_context::load_signing_ood(params, &mut doc).await?;
        let (app, app_host) = crate::federation_context::verify(
            params,
            &doc,
            &client_host,
            buckyos_get_unix_timestamp(),
        )?;
        let redirect_uri = validate_redirect_uri(
            &params.redirect_uri,
            &client_host,
            settings.dev_federation_http,
        )?;
        Ok(Ok(FederationContext {
            account,
            app,
            app_host,
            client_zone,
            client_host,
            redirect_uri,
            settings,
        }))
    }

    pub(crate) async fn handle_federation_consents(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        if !principal.is_user_session
            || !principal.is_control_panel_session
            || principal.is_restricted()
        {
            return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed).into_rpc_error());
        }
        let kv = ZoneKv::connect().await?;
        let (mut consents, version) = load_consents(&kv, &principal.username).await?;
        if req.method == "auth.federation.forget" {
            let id = Self::require_param_str(&req, "id")?;
            if consents.remove(&id).is_some() {
                let key = consents_key(&principal.username);
                let raw = serde_json::to_string(&consents)
                    .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
                kv.exec_tx(
                    HashMap::from([(key.clone(), KVAction::Update(raw))]),
                    version.map(|n| (key, n)),
                )
                .await?;
            }
        }
        let items = consents.into_iter().filter_map(|(id, consent)| {
            let (zone, target) = id.split_once('|')?;
            Some(json!({"id": id, "client_zone": zone, "target":target, "granted_at":consent.granted_at}))
        }).collect::<Vec<_>>();
        success(json!({"consents":items}), req.seq)
    }

    /// `auth.federation.describe {did, client_zone, redirect_uri, state, nonce,
    /// code_challenge}` → `{status:"consent", client_zone, client_host, profile:
    /// {show_name, avatar?}, already_granted}` | `{status:"denied", denial}`.
    pub(crate) async fn handle_auth_federation_describe(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let params: FederationParams = Self::typed_params(&req)?;
        let context = match Self::federation_context(principal, &params, true).await? {
            Ok(context) => context,
            Err(denial) => {
                return success(
                    FederationResult::Denied {
                        denial: denial
                            .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "auth.federation.describe"),
                    },
                    req.seq,
                )
            }
        };
        let account = context.account.ok_or_else(account_missing)?;
        let consents = load_consents(&ZoneKv::connect().await?, &principal.username).await?;
        let already_granted = consents
            .0
            .get(&consent_id(&context.client_zone, &context.app))
            .is_some_and(|consent| consent.profile_scope >= PROFILE_SCOPE);
        success(
            FederationResult::Consent {
                client_zone: context.client_zone.to_string(),
                client_host: context.client_host,
                app_host: context.app_host,
                profile: account.profile(),
                already_granted,
            },
            req.seq,
        )
    }

    /// `auth.federation.approve {…describe params, remember}` →
    /// `{status:"redirect", url:"<redirect_uri>?code=..&state=.."}` | `{status:"denied", denial}`.
    pub(crate) async fn handle_auth_federation_approve(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let params: FederationParams = Self::typed_params(&req)?;
        let context = match Self::federation_context(principal, &params, true).await? {
            Ok(context) => context,
            Err(denial) => {
                return success(
                    FederationResult::Denied {
                        denial: denial
                            .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "auth.federation.approve"),
                    },
                    req.seq,
                )
            }
        };
        let account = context.account.as_ref().ok_or_else(account_missing)?;
        if params.remember {
            save_consent(
                &ZoneKv::connect().await?,
                &principal.username,
                &context.client_zone,
                &context.app,
                buckyos_get_unix_timestamp(),
            )
            .await?;
        }
        let issued = get_buckyos_api_runtime()?
            .get_verify_hub_client()
            .await?
            .issue_federation_code(IssueFederationCodeRequest {
                user_id: principal.username.clone(),
                did: params.did.clone(),
                client_zone: context.client_zone.to_string(),
                redirect_uri: params.redirect_uri.clone(),
                nonce: params.nonce.clone(),
                code_challenge: params.code_challenge.clone(),
                profile: account.profile(),
            })
            .await?;
        let mut url = context.redirect_uri;
        url.query_pairs_mut()
            .append_pair("code", &issued.code)
            .append_pair("state", &params.state);
        info!(
            "'{}' vouched for {} towards {} (dev_http={})",
            principal.username,
            params.did,
            context.client_zone.to_string(),
            context.settings.dev_federation_http
        );
        success(
            FederationResult::Redirect {
                url: url.to_string(),
            },
            req.seq,
        )
    }

    /// `auth.federation.deny {client_zone, redirect_uri, state, …}` →
    /// `{status:"redirect", url:"<redirect_uri>?error=access_denied&state=.."}`.
    pub(crate) async fn handle_auth_federation_deny(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let params: FederationParams = Self::typed_params(&req)?;
        let context = match Self::federation_context(principal, &params, false).await? {
            Ok(context) => context,
            Err(denial) => {
                return success(
                    FederationResult::Denied {
                        denial: denial.at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "auth.federation.deny"),
                    },
                    req.seq,
                )
            }
        };
        let mut url = context.redirect_uri;
        url.query_pairs_mut()
            .append_pair("error", "access_denied")
            .append_pair("state", &params.state);
        success(
            FederationResult::Redirect {
                url: url.to_string(),
            },
            req.seq,
        )
    }
}

async fn load_consents(
    kv: &dyn AgentKv,
    user_id: &str,
) -> Result<(FederationConsents, Option<u64>), RPCErrors> {
    match kv.get(&consents_key(user_id)).await? {
        Some((raw, version)) => serde_json::from_str(&raw)
            .map(|consents| (consents, Some(version)))
            .map_err(|error| RPCErrors::ReasonError(format!("consents of {user_id}: {error}"))),
        None => Ok((FederationConsents::new(), None)),
    }
}

fn did_browser_cookie(state: &str, origin: &str) -> String {
    let prefix = if origin.starts_with("https://") { "__Host-" } else { "" };
    format!("{prefix}buckyos_did_{state}")
}

fn consent_id(zone: &DID, app: &AuthTarget) -> String {
    format!("{}|{}", zone.to_string(), app.canonical_key())
}

async fn save_consent(
    kv: &dyn AgentKv,
    user_id: &str,
    client_zone: &DID,
    app: &AuthTarget,
    now: u64,
) -> Result<(), RPCErrors> {
    let (mut consents, version) = load_consents(kv, user_id).await?;
    consents.insert(
        consent_id(client_zone,app),
        FederationConsent {
            granted_at: now,
            profile_scope: PROFILE_SCOPE,
        },
    );
    let raw = serde_json::to_string(&consents)
        .map_err(|error| RPCErrors::ReasonError(format!("serialize consents: {error}")))?;
    let key = consents_key(user_id);
    let (action, main_key) = match version {
        Some(version) => (KVAction::Update(raw), Some((key.clone(), version))),
        None => (KVAction::Create(raw), None),
    };
    kv.exec_tx(HashMap::from([(key, action)]), main_key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclusive_invite::tests::{zone, MemKv};
    use crate::PrincipalSession;
    use buckyos_api::{PasswordState, SessionCeiling, UserState, UserType};

    fn root_target() -> ResolvedSsoAuthTarget {
        ResolvedSsoAuthTarget {
            auth_target: AuthTarget::system("control-panel".parse().unwrap()),
            canonical_origin: "https://b.example".to_string(),
            canonical_redirect_url: "https://b.example/sso/entry?service=aiworkspace&entry=se-1"
                .to_string(),
        }
    }

    #[test]
    fn begin_binds_the_callback_and_keeps_the_verifier_here() {
        let did = DID::from_str("did:bns:alice").unwrap();
        let client_zone = DID::from_str("did:bns:b").unwrap();
        let (state, pending, url) = plan_did_login(
            &did,
            "a.example",
            &client_zone,
            "https://b.example",
            root_target(),
            false,
            100,
        )
        .unwrap();
        let url = url::Url::parse(&url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("a.example"));
        assert_eq!(url.path(), "/sso/federate");
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(query["did"], "did:bns:alice");
        assert_eq!(query["client_zone"], "did:bns:b");
        assert_eq!(query["redirect_uri"], "https://b.example/sso/did_callback");
        assert_eq!(query["state"], state);
        assert_eq!(query["nonce"], pending.nonce);
        assert_eq!(
            query["code_challenge"],
            code_challenge(&pending.code_verifier)
        );
        assert!(!url.as_str().contains(&pending.code_verifier));
        assert_eq!(URL_SAFE_NO_PAD.decode(&state).unwrap().len(), 32);
        assert_eq!(pending.redirect_uri, "https://b.example/sso/did_callback");
        // RFC 7636 appendix B.
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn pending_did_logins_are_single_use_and_expire() {
        let now = 10_000;
        let (state, pending, _) = plan_did_login(
            &DID::from_str("did:bns:alice").unwrap(),
            "a.example",
            &zone(),
            "https://b.example",
            root_target(),
            false,
            now,
        )
        .unwrap();
        let mut cache = HashMap::new();
        assert!(store_pending(
            &mut cache,
            state.clone(),
            pending.clone(),
            now
        ));
        assert!(take_pending(&mut cache, &state, now + 1).is_some());
        assert!(take_pending(&mut cache, &state, now + 1).is_none());
        assert!(store_pending(
            &mut cache,
            state.clone(),
            pending.clone(),
            now
        ));
        assert!(take_pending(&mut cache, &state, now + PENDING_DID_LOGIN_TTL_SECS + 1).is_none());
        assert!(cache.is_empty());
        // Bounded: a full cache refuses until entries expire.
        for index in 0..MAX_PENDING_DID_LOGINS {
            assert!(store_pending(
                &mut cache,
                index.to_string(),
                pending.clone(),
                now
            ));
        }
        assert!(!store_pending(
            &mut cache,
            "one-more".to_string(),
            pending.clone(),
            now
        ));
        assert!(store_pending(
            &mut cache,
            "later".to_string(),
            pending,
            now + PENDING_DID_LOGIN_TTL_SECS + 1
        ));
        assert_eq!(cache.len(), 1);
        assert_eq!(
            did_error_location("access_denied", Some("https://b.example/x?y=1")),
            "/login?did_error=access_denied&redirect_url=https%3A%2F%2Fb.example%2Fx%3Fy%3D1"
        );
    }

    #[test]
    fn a_callback_never_replaces_another_signed_in_subject() {
        assert!(!conflicting_subject(&[], "did:bns:alice"));
        assert!(!conflicting_subject(
            &["did:bns:alice".to_string(), "did:bns:alice".to_string()],
            "did:bns:alice"
        ));
        assert!(conflicting_subject(
            &["mallory".to_string()],
            "did:bns:alice"
        ));
        assert!(conflicting_subject(
            &["did:bns:alice".to_string(), "mallory".to_string()],
            "did:bns:alice"
        ));
    }

    #[test]
    fn redirect_uri_must_be_the_client_zones_callback() {
        assert!(
            validate_redirect_uri("https://b.example/sso/did_callback", "b.example", false).is_ok()
        );
        assert!(
            validate_redirect_uri("https://B.example./sso/did_callback", "b.example", false)
                .is_ok()
        );
        for bad in [
            "https://evil.example/sso/did_callback",
            "https://b.example/sso/other",
            "https://b.example/sso/did_callback?x=1",
            "https://b.example/sso/did_callback#x",
            "https://u:p@b.example/sso/did_callback",
            "https://b.example:8443/sso/did_callback",
            "http://b.example/sso/did_callback",
            "javascript:alert(1)",
        ] {
            assert!(
                validate_redirect_uri(bad, "b.example", false).is_err(),
                "{bad}"
            );
        }
        assert!(
            validate_redirect_uri("http://b.example:3180/sso/did_callback", "b.example", true)
                .is_ok()
        );
    }

    fn account(user_type: UserType, default_zone: Option<DID>) -> FederationAccount {
        let mut owner = OwnerDocument::new(
            DID::from_str("did:bns:alice").unwrap(),
            "alice".to_string(),
            "Alice".to_string(),
            serde_json::from_value(json!({
                "kty": "OKP", "crv": "Ed25519",
                "x": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            }))
            .unwrap(),
        );
        if let Some(zone) = default_zone {
            owner.set_default_zone_did(zone);
        }
        FederationAccount {
            settings: UserSettings {
                user_id: "alice".to_string(),
                user_type,
                password: "x".to_string(),
                state: UserState::Active,
                res_pool_id: "default".to_string(),
                is_local: true,
                allow_password_change: None,
                allow_contact_edit: None,
                groups: Vec::new(),
                password_state: PasswordState::Set,
            },
            owner,
            profile: None,
        }
    }

    fn principal(ceiling: SessionCeiling) -> RpcAuthPrincipal {
        RpcAuthPrincipal {
            username: "alice".to_string(),
            owner_user_id: "alice".to_string(),
            authenticated_app_id: "control-panel".to_string(),
            user_type: UserType::User,
            owner_did: String::new(),
            is_user_session: true,
            is_control_panel_session: true,
            principal_kind: buckyos_api::TokenPrincipalKind::User,
            agent_permission_mode: None,
            session: PrincipalSession {
                ceiling,
                ..Default::default()
            },
        }
    }

    #[test]
    fn only_the_canonical_owner_of_the_did_vouches_for_it() {
        let here = zone();
        let user = principal(SessionCeiling::unrestricted());
        let ok = account(UserType::User, Some(here.clone()));
        assert!(federation_check(&user, "did:bns:alice", &ok, &here).is_ok());
        assert_eq!(ok.profile().show_name, "alice");
        let reason = |result: Result<(), AccessDenial>| result.unwrap_err().reason;
        assert_eq!(
            reason(federation_check(&user, "did:bns:bob", &ok, &here)),
            DenyReason::SessionCeilingExceeded
        );
        assert_eq!(
            reason(federation_check(
                &user,
                "did:bns:alice",
                &account(UserType::Limited, Some(here.clone())),
                &here
            )),
            DenyReason::SessionKindNotAllowed
        );
        assert_eq!(
            reason(federation_check(
                &user,
                "did:bns:alice",
                &account(UserType::User, None),
                &here
            )),
            DenyReason::SessionKindNotAllowed
        );
        // A self-hosted Zone: the owner DID is the Zone DID and binds no other zone.
        let self_hosted = DID::from_str("did:bns:alice").unwrap();
        assert!(federation_check(
            &user,
            "did:bns:alice",
            &account(UserType::User, None),
            &self_hosted
        )
        .is_ok());
        let mut suspended = account(UserType::User, Some(here.clone()));
        suspended.settings.state = UserState::Suspended("x".to_string());
        assert_eq!(
            reason(federation_check(&user, "did:bns:alice", &suspended, &here)),
            DenyReason::AccountState
        );
        let limited = principal(SessionCeiling::for_user_type(UserType::Limited));
        assert_eq!(
            reason(federation_check(&limited, "did:bns:alice", &ok, &here)),
            DenyReason::SessionKindNotAllowed
        );
    }

    #[tokio::test]
    async fn consent_is_kept_per_client_zone_and_app() {
        let kv = MemKv::default();
        let b = DID::from_str("did:bns:b").unwrap();
        let c = DID::from_str("did:bns:c").unwrap();
        save_consent(&kv, "alice", &b, &root_target().auth_target, 10)
            .await
            .unwrap();
        save_consent(&kv, "alice", &c, &root_target().auth_target, 20)
            .await
            .unwrap();
        save_consent(&kv, "alice", &b, &root_target().auth_target, 30)
            .await
            .unwrap();
        let workspace = AuthTarget::from_canonical_key("system:aiworkspace").unwrap();
        save_consent(&kv, "alice", &b, &workspace, 40)
            .await
            .unwrap();
        let (consents, _) = load_consents(&kv, "alice").await.unwrap();
        assert!(consents.contains_key(&consent_id(&b, &workspace)));
        assert!(!consents.contains_key(&consent_id(&c, &workspace)));
        assert_eq!(consents.len(), 3);
        assert_eq!(
            consents[&consent_id(&b, &root_target().auth_target)],
            FederationConsent {
                granted_at: 30,
                profile_scope: 1
            }
        );
        assert!(load_consents(&kv, "bob").await.unwrap().0.is_empty());
    }
}
