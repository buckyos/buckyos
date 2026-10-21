#[allow(unused_braces)]
use base64::{engine::general_purpose::STANDARD, Engine as _};
use lazy_static::lazy_static;
use log::*;
use rand::prelude::*;

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use ::kRPC::*;
use buckyos_api::*;
use buckyos_kit::*;
use jsonwebtoken::{errors::ErrorKind, Algorithm, DecodingKey, EncodingKey, Validation};
use name_lib::*;

// Token expiration time constants
// Session token: short-lived, used for API requests
const SESSION_TOKEN_EXPIRE_SECONDS: u64 = 15 * 60; // 15 minutes
const SUDO_SESSION_TOKEN_EXPIRE_SECONDS: u64 = 3 * 60;
// Refresh token: long-lived, used to obtain new token pairs
const REFRESH_TOKEN_EXPIRE_SECONDS: u64 = 7 * 24 * 3600; // 7 days
const MAX_LOGIN_NONCE_AGE_SECONDS: u64 = 3600 * 8; // 8 hours
                                                   // The refresh token rotated away last is still answered with the current pair for this long,
                                                   // so pages of one host refreshing over the same cookie at once do not revoke their family.
const REFRESH_GRACE_SECONDS: u64 = 10;
use buckyos_http_server::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;

type Result<T> = std::result::Result<T, RPCErrors>;
const SESSION_FIELD: &str = "session";

#[derive(Clone, Debug, PartialEq)]
struct VerifyServiceConfig {
    zone_document: ZoneDocument,
    device_id: String,
    node_did: DID,
    start_time: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrustedIssuerKind {
    VerifyHub,
    Root,
    User,
    Device,
    Agent,
}

#[derive(Clone)]
struct TrustedKey {
    key: DecodingKey,
    issuer_kind: TrustedIssuerKind,
}

#[derive(Clone, Debug)]
struct RotatedRefresh {
    jti: String,
    rotated_at: u64,
}

#[derive(Clone, Debug)]
struct RefreshFamily {
    refresh_token: RPCSessionToken,
    session_token: String,
    previous_refresh: Option<RotatedRefresh>,
}

impl RefreshFamily {
    fn new(session_token: &RPCSessionToken, refresh_token: RPCSessionToken) -> Self {
        Self {
            refresh_token,
            session_token: session_token.to_string(),
            previous_refresh: None,
        }
    }

    fn token_pair(&self) -> TokenPair {
        TokenPair {
            session_token: self.session_token.clone(),
            refresh_token: self.refresh_token.to_string(),
        }
    }

    fn is_live(&self, now: u64) -> bool {
        self.refresh_token.exp.map(|exp| exp > now).unwrap_or(false)
    }
}

#[derive(Debug)]
enum RefreshPresentation {
    Current,
    Grace(TokenPair),
    Missing,
    Reused,
}

fn classify_refresh_presentation(
    family: Option<&RefreshFamily>,
    jti: &str,
    now: u64,
) -> RefreshPresentation {
    let Some(family) = family else {
        return RefreshPresentation::Missing;
    };
    if family.refresh_token.jti.as_deref() == Some(jti) {
        return RefreshPresentation::Current;
    }
    match family.previous_refresh.as_ref() {
        Some(previous)
            if previous.jti == jti
                && now.saturating_sub(previous.rotated_at) <= REFRESH_GRACE_SECONDS =>
        {
            RefreshPresentation::Grace(family.token_pair())
        }
        _ => RefreshPresentation::Reused,
    }
}

#[derive(Debug)]
enum RefreshCommit {
    Committed,
    Grace(TokenPair),
    LoggedOut,
    Reused,
}

struct PresentedRefresh {
    token: RPCSessionToken,
    session_key: String,
    jti: String,
}

struct DelegationSource {
    user_id: String,
    session_key: String,
}

type SessionPrincipalKind = TokenPrincipalKind;

const VERIFY_HUB_ISSUER: &str = "verify-hub";
const VERIFY_HUB_SERVICE_MAIN_PORT: u16 = 3300;
const ROOT_USER_ID: &str = "root";

lazy_static! {
    static ref VERIFY_HUB_PRIVATE_KEY: Arc<RwLock<EncodingKey>> = {
        let private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;
        let private_key = EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
        Arc::new(RwLock::new(private_key))
    };
    // Cache for session tokens, keyed by session_key (userid_appid_session_id)
    static ref TOKEN_CACHE: Arc<Mutex<HashMap<String, RPCSessionToken>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Active refresh families, keyed by session_key. Refresh rotates the record in place;
    // logout and reuse detection remove it.
    static ref REFRESH_TOKEN_CACHE: Arc<Mutex<HashMap<String, RefreshFamily>>> =
        Arc::new(Mutex::new(HashMap::new()));
    static ref TRUSTKEY_CACHE: Arc<Mutex<HashMap<String, TrustedKey>>> =
        Arc::new(Mutex::new(HashMap::new()));
    static ref VERIFY_SERVICE_CONFIG: Arc<Mutex<Option<VerifyServiceConfig>>> =
        Arc::new(Mutex::new(None));
    static ref MY_RPC_TOKEN: Arc<Mutex<Option<RPCSessionToken>>> =  Arc::new(Mutex::new(None)) ;
}

fn set_token_session_id(token: &mut RPCSessionToken, session_id: u64) {
    token
        .extra
        .insert(SESSION_FIELD.to_string(), Value::from(session_id));
}

fn set_token_principal_kind(token: &mut RPCSessionToken, principal_kind: SessionPrincipalKind) {
    bind_token_principal_kind(token, principal_kind);
}

fn get_token_session_id(token: &RPCSessionToken) -> Result<u64> {
    let session = token
        .extra
        .get(SESSION_FIELD)
        .ok_or(RPCErrors::ReasonError("Missing session".to_string()))?;

    if let Some(session) = session.as_u64() {
        return Ok(session);
    }

    if let Some(session) = session.as_str() {
        return session
            .parse::<u64>()
            .map_err(|_| RPCErrors::ReasonError("Invalid session".to_string()));
    }

    Err(RPCErrors::ReasonError("Invalid session".to_string()))
}

fn is_root_userid(userid: &str) -> bool {
    userid.trim().eq_ignore_ascii_case(ROOT_USER_ID)
}

fn reject_root_session_subject(userid: &str) -> Result<()> {
    if is_root_userid(userid) {
        return Err(RPCErrors::NoPermission(
            "verify-hub does not issue session tokens for root".to_string(),
        ));
    }
    Ok(())
}

fn reject_root_user_settings(user_settings: &UserSettings) -> Result<()> {
    reject_root_session_subject(user_settings.user_id.as_str())?;
    if matches!(user_settings.user_type, UserType::Root) {
        return Err(RPCErrors::NoPermission(
            "verify-hub does not issue session tokens for root".to_string(),
        ));
    }
    Ok(())
}

fn require_active_user_settings(user_settings: &UserSettings) -> Result<()> {
    if user_is_active(user_settings) {
        Ok(())
    } else {
        Err(RPCErrors::NoPermission(format!(
            "user '{}' is not active",
            user_settings.user_id
        )))
    }
}

/// Generate a session token with specified parameters
/// Session token is short-lived and used for API requests
async fn generate_session_token(
    target: &AuthTarget,
    userid: &str,
    jti: u64,
    session: u64,
    duration: u64,
    aud: Option<String>,
    sudo: bool,
    principal_kind: SessionPrincipalKind,
) -> Result<RPCSessionToken> {
    reject_root_session_subject(userid)?;
    let now = buckyos_get_unix_timestamp();
    let exp = now + duration;

    let mut session_token = RPCSessionToken {
        token_type: RPCSessionTokenType::Normal,
        appid: None,
        jti: Some(jti.to_string()),
        aud: aud,
        sub: Some(userid.to_string()),
        token: None,
        iss: Some(VERIFY_HUB_ISSUER.to_string()),
        exp: Some(exp),
        sudo,
        extra: HashMap::new(),
    };
    set_token_session_id(&mut session_token, session);
    set_token_principal_kind(&mut session_token, principal_kind);
    bind_token_target(&mut session_token, target, TokenUse::Session)?;

    {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let jwt = session_token.generate_jwt(Some(VERIFY_HUB_ISSUER.to_string()), &private_key)?;
        session_token.token = Some(jwt);
    }

    Ok(session_token)
}

/// Generate a refresh token with specified parameters
/// Refresh token is long-lived and used to obtain new token pairs
async fn generate_refresh_token(
    target: &AuthTarget,
    userid: &str,
    jti: u64,
    session: u64,
    duration: u64,
    principal_kind: SessionPrincipalKind,
) -> Result<RPCSessionToken> {
    reject_root_session_subject(userid)?;
    let now = buckyos_get_unix_timestamp();
    let exp = now + duration;

    let mut refresh_token = RPCSessionToken {
        token_type: RPCSessionTokenType::Normal,
        appid: None,
        jti: Some(jti.to_string()),
        aud: Some(VERIFY_HUB_UNIQUE_ID.to_string()), //refresh token audience is verify-hub
        sub: Some(userid.to_string()),
        token: None,
        iss: Some(VERIFY_HUB_ISSUER.to_string()),
        exp: Some(exp),
        sudo: false,
        extra: HashMap::new(),
    };
    set_token_session_id(&mut refresh_token, session);
    set_token_principal_kind(&mut refresh_token, principal_kind);
    bind_token_target(&mut refresh_token, target, TokenUse::Refresh)?;

    {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let jwt = refresh_token.generate_jwt(Some(VERIFY_HUB_ISSUER.to_string()), &private_key)?;
        refresh_token.token = Some(jwt);
    }

    Ok(refresh_token)
}

/// Generate a token pair (session_token + refresh_token) for login
/// This is the core function that creates dual tokens as per SSO specification:
/// - session_token: short-lived (15 minutes), used for API requests
/// - refresh_token: long-lived (7 days), used to obtain new token pairs
async fn generate_token_pair(
    target: &AuthTarget,
    userid: &str,
    session_id: u64,
    principal_kind: SessionPrincipalKind,
) -> Result<(TokenPair, RPCSessionToken, RPCSessionToken)> {
    // Generate random jti (JWT ID) for both tokens
    let session_jti: u64;
    let refresh_jti: u64;
    {
        let mut rng = rand::thread_rng();
        session_jti = rng.gen::<u64>();
        refresh_jti = rng.gen::<u64>();
    }

    // Generate short-lived session token
    let session_token = generate_session_token(
        target,
        userid,
        session_jti,
        session_id,
        SESSION_TOKEN_EXPIRE_SECONDS,
        None,
        false,
        principal_kind,
    )
    .await?;

    // Generate long-lived refresh token
    let refresh_token = generate_refresh_token(
        target,
        userid,
        refresh_jti,
        session_id,
        REFRESH_TOKEN_EXPIRE_SECONDS,
        principal_kind,
    )
    .await?;

    let token_pair = TokenPair {
        session_token: session_token.to_string(),
        refresh_token: refresh_token.to_string(),
    };

    Ok((token_pair, session_token, refresh_token))
}

/// Start a refresh family for a freshly issued token pair
async fn cache_refresh_family(
    key: &str,
    session_token: &RPCSessionToken,
    refresh_token: RPCSessionToken,
) {
    REFRESH_TOKEN_CACHE.lock().await.insert(
        key.to_string(),
        RefreshFamily::new(session_token, refresh_token),
    );
}

async fn gc_token_caches() {
    let now = buckyos_get_unix_timestamp();

    {
        let mut cache = TOKEN_CACHE.lock().await;
        cache.retain(|_, token| token.exp.map(|exp| exp > now).unwrap_or(false));
    }

    {
        let mut cache = REFRESH_TOKEN_CACHE.lock().await;
        cache.retain(|_, family| family.is_live(now));
    }
}

async fn revoke_session_tokens(session_key: &str) {
    TOKEN_CACHE.lock().await.remove(session_key);
    REFRESH_TOKEN_CACHE.lock().await.remove(session_key);
}

async fn verify_presented_refresh(refresh_jwt: &str) -> Result<PresentedRefresh> {
    let jwt_payload = verify_verify_hub_jwt(refresh_jwt, None).await?;

    let rpc_session_token: RPCSessionToken =
        serde_json::from_value(jwt_payload).map_err(|error| {
            error!(
                "Failed to parse RPCSessionToken from JWT payload: {}",
                error
            );
            RPCErrors::ReasonError("Failed to parse RPCSessionToken from JWT payload".to_string())
        })?;

    let userid = rpc_session_token
        .sub
        .clone()
        .ok_or(RPCErrors::ReasonError("Missing sub".to_string()))?;
    reject_root_session_subject(userid.as_str())?;
    let claims = validate_verify_hub_token_claims(&rpc_session_token, TokenUse::Refresh)?;
    let session_id = get_token_session_id(&rpc_session_token)?;
    let session_key = format!(
        "{}_{}_{}",
        userid,
        claims.target.canonical_key(),
        session_id
    );
    let jti = rpc_session_token
        .jti
        .clone()
        .ok_or(RPCErrors::ReasonError("Missing jti".to_string()))?;

    Ok(PresentedRefresh {
        token: rpc_session_token,
        session_key,
        jti,
    })
}

/// Accepts the family's current refresh token, or the one it replaced within the grace
/// window (answered with the current pair). Any other presentation revokes the family.
async fn check_presented_refresh(presented: &PresentedRefresh) -> Result<Option<TokenPair>> {
    let now = buckyos_get_unix_timestamp();
    let presentation = {
        let cache = REFRESH_TOKEN_CACHE.lock().await;
        classify_refresh_presentation(
            cache.get(presented.session_key.as_str()),
            presented.jti.as_str(),
            now,
        )
    };
    match presentation {
        RefreshPresentation::Current => Ok(None),
        RefreshPresentation::Grace(token_pair) => Ok(Some(token_pair)),
        RefreshPresentation::Missing => {
            warn!(
                "Refresh reuse detected (cache-miss), revoking session: {}",
                presented.session_key
            );
            revoke_session_tokens(presented.session_key.as_str()).await;
            Err(RPCErrors::ReasonError(
                "Refresh token not found or already invalidated".to_string(),
            ))
        }
        RefreshPresentation::Reused => {
            warn!(
                "Refresh reuse detected (jti-mismatch), revoking session: {}",
                presented.session_key
            );
            revoke_session_tokens(presented.session_key.as_str()).await;
            Err(RPCErrors::ReasonError(
                "Invalid refresh token jti".to_string(),
            ))
        }
    }
}

/// Compare-and-replace of the family's refresh token: commits only while `old_jti` is still
/// current, so a logout or a competing rotation that happened meanwhile is never overwritten.
async fn commit_refresh_rotation(
    session_key: &str,
    old_jti: &str,
    session_token: &RPCSessionToken,
    refresh_token: RPCSessionToken,
) -> RefreshCommit {
    let now = buckyos_get_unix_timestamp();
    let mut cache = REFRESH_TOKEN_CACHE.lock().await;
    match classify_refresh_presentation(cache.get(session_key), old_jti, now) {
        RefreshPresentation::Current => {
            if let Some(family) = cache.get_mut(session_key) {
                family.previous_refresh = Some(RotatedRefresh {
                    jti: old_jti.to_string(),
                    rotated_at: now,
                });
                family.refresh_token = refresh_token;
                family.session_token = session_token.to_string();
            }
            RefreshCommit::Committed
        }
        RefreshPresentation::Grace(token_pair) => RefreshCommit::Grace(token_pair),
        RefreshPresentation::Missing => RefreshCommit::LoggedOut,
        RefreshPresentation::Reused => {
            cache.remove(session_key);
            RefreshCommit::Reused
        }
    }
}

async fn source_family_is_live(session_key: &str) -> bool {
    let now = buckyos_get_unix_timestamp();
    REFRESH_TOKEN_CACHE
        .lock()
        .await
        .get(session_key)
        .map(|family| family.is_live(now))
        .unwrap_or(false)
}

/// Inserts the delegated family only while the source family is still live, in the same
/// critical section, so nothing can be delegated from a session once its logout completed.
async fn commit_delegated_family(
    source_session_key: &str,
    session_key: &str,
    session_token: &RPCSessionToken,
    refresh_token: RPCSessionToken,
) -> bool {
    let now = buckyos_get_unix_timestamp();
    let mut cache = REFRESH_TOKEN_CACHE.lock().await;
    if !cache
        .get(source_session_key)
        .map(|family| family.is_live(now))
        .unwrap_or(false)
    {
        return false;
    }
    cache.insert(
        session_key.to_string(),
        RefreshFamily::new(session_token, refresh_token),
    );
    true
}

fn control_panel_target() -> Result<AuthTarget> {
    Ok(AuthTarget::system(
        SystemServiceId::parse(CONTROL_PANEL_SERVICE_UNIQUE_ID).map_err(RPCErrors::ReasonError)?,
    ))
}

/// Only the Control Panel service may exchange a user's session: /kapi/verify-hub is
/// reachable from browsers, and a delegated token pair must never be returned to them.
async fn authorize_delegate_caller(caller_token: Option<&str>) -> Result<()> {
    let denied =
        || RPCErrors::NoPermission("only control-panel may delegate user sessions".to_string());
    let caller_token = caller_token.ok_or_else(denied)?;
    let payload = verify_verify_hub_jwt(caller_token, None)
        .await
        .map_err(|error| {
            warn!("delegate_user_session caller token rejected: {}", error);
            denied()
        })?;
    let caller: RPCSessionToken = serde_json::from_value(payload).map_err(|_| denied())?;
    let claims =
        validate_verify_hub_token_claims(&caller, TokenUse::Session).map_err(|_| denied())?;
    if caller.sudo
        || !matches!(
            claims.principal_kind,
            SessionPrincipalKind::System | SessionPrincipalKind::Device
        )
        || claims.target != control_panel_target()?
    {
        return Err(denied());
    }
    Ok(())
}

/// `None` when the source cannot prove a session (bad signature, expired, malformed):
/// the caller should report it as an inactive source rather than an error.
async fn verify_delegation_source(source_session_token: &str) -> Result<Option<DelegationSource>> {
    let payload = match verify_verify_hub_jwt(source_session_token, None).await {
        Ok(payload) => payload,
        Err(error) => {
            info!("delegation source token rejected: {}", error);
            return Ok(None);
        }
    };
    let source: RPCSessionToken = serde_json::from_value(payload).map_err(|_| {
        RPCErrors::InvalidToken("delegation source is not a session token".to_string())
    })?;
    let claims = validate_verify_hub_token_claims(&source, TokenUse::Session)?;
    let control_panel = control_panel_target()?;
    if claims.principal_kind != SessionPrincipalKind::User
        || claims.target != control_panel
        || source.sudo
    {
        return Err(RPCErrors::InvalidToken(
            "delegation source must be a Control Panel user session".to_string(),
        ));
    }
    let user_id = source
        .sub
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RPCErrors::InvalidToken("delegation source missing sub".to_string()))?
        .to_string();
    reject_root_session_subject(user_id.as_str())?;
    let session_id = get_token_session_id(&source)
        .map_err(|_| RPCErrors::InvalidToken("delegation source missing session".to_string()))?;
    Ok(Some(DelegationSource {
        session_key: format!(
            "{}_{}_{}",
            user_id,
            control_panel.canonical_key(),
            session_id
        ),
        user_id,
    }))
}

async fn validate_refresh_principal(
    userid: &str,
    principal_kind: SessionPrincipalKind,
) -> Result<()> {
    match principal_kind {
        SessionPrincipalKind::User => {
            let control_panel_client = ControlPanelClient::new(get_system_config_client().await?);
            let user_settings = control_panel_client
                .get_user_settings_by_username(userid)
                .await?;
            reject_root_user_settings(&user_settings)?;
            require_active_user_settings(&user_settings)
        }
        SessionPrincipalKind::Device => {
            let control_panel_client = ControlPanelClient::new(get_system_config_client().await?);
            control_panel_client.get_device_config(userid).await?;
            Ok(())
        }
        SessionPrincipalKind::System | SessionPrincipalKind::App => Ok(()),
        SessionPrincipalKind::Agent => {
            let trusted = load_trust_public_key_from_source(userid).await?;
            if trusted.issuer_kind != TrustedIssuerKind::Agent {
                return Err(RPCErrors::InvalidToken(
                    "agent token subject is not an AgentDID".to_string(),
                ));
            }
            Ok(())
        }
    }
}

async fn check_app_principal_target(target: &AuthTarget) -> Result<()> {
    if let AuthTarget::App { app_instance_id } = target {
        buckyos_api::check_agent_runtime_session(&get_system_config_client().await?, app_instance_id)
            .await
            .map_err(|error| {
                warn!("App session refused for {}: {}", app_instance_id, error);
                error
            })?;
    }
    Ok(())
}

async fn resolve_user_auth_target(user_id: &str, target: &AuthTarget) -> Result<AuthTarget> {
    match target {
        AuthTarget::App { app_instance_id } => {
            let resolver =
                AppAvailabilityResolver::new(Arc::new(get_system_config_client().await?));
            let decision = resolver
                .check_user(user_id, app_instance_id)
                .await
                .map_err(|error| {
                    warn!(
                        "app availability check failed user={} app_instance_id={}: {}",
                        user_id, app_instance_id, error
                    );
                    match error {
                        RPCErrors::KeyNotExist(_) | RPCErrors::NoPermission(_) => {
                            RPCErrors::NoPermission("AppAccessDenied".to_string())
                        }
                        error => error,
                    }
                })?;
            if !decision.allowed {
                warn!(
                    "app availability denied user={} app_instance_id={} reason={}",
                    user_id, app_instance_id, decision.reason
                );
                return Err(RPCErrors::NoPermission("AppAccessDenied".to_string()));
            }
            Ok(AuthTarget::app(decision.app_instance_id))
        }
        AuthTarget::System { service_id } => {
            if !is_system_login_target(service_id.as_str()) {
                return Err(RPCErrors::NoPermission(format!(
                    "system service '{}' does not allow interactive user login",
                    service_id
                )));
            }
            Ok(target.clone())
        }
    }
}

async fn get_my_krpc_token() -> Result<RPCSessionToken> {
    let now = buckyos_get_unix_timestamp();
    let device_id = VERIFY_SERVICE_CONFIG
        .lock()
        .await
        .as_ref()
        .unwrap()
        .device_id
        .clone();

    let my_rpc_token = MY_RPC_TOKEN.lock().await;
    if let Some(token) = my_rpc_token.as_ref() {
        if let Some(exp) = token.exp {
            if exp - 30 > now {
                return Ok(token.clone());
            }
        }
    }
    drop(my_rpc_token);

    let exp = now + VERIFY_HUB_TOKEN_EXPIRE_TIME;

    let mut session_token = RPCSessionToken {
        token_type: RPCSessionTokenType::Normal,
        appid: None,
        jti: None,
        aud: None,
        sub: Some(device_id),
        token: None,
        iss: Some(VERIFY_HUB_ISSUER.to_string()),
        exp: Some(exp),
        sudo: false,
        extra: HashMap::new(),
    };
    set_token_principal_kind(&mut session_token, SessionPrincipalKind::System);
    bind_token_target(
        &mut session_token,
        &AuthTarget::system(
            SystemServiceId::parse(VERIFY_HUB_UNIQUE_ID).map_err(RPCErrors::ReasonError)?,
        ),
        TokenUse::Session,
    )?;

    {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let jwt = session_token.generate_jwt(Some(VERIFY_HUB_ISSUER.to_string()), &private_key)?;
        session_token.token = Some(jwt);
    }

    let mut my_rpc_token = MY_RPC_TOKEN.lock().await;
    *my_rpc_token = Some(session_token.clone());
    Ok(session_token)
}

async fn get_system_config_client() -> Result<SystemConfigClient> {
    let rpc_token = get_my_krpc_token().await?;
    let rpc_token_str = rpc_token.to_string();
    Ok(SystemConfigClient::new(None, Some(&rpc_token_str)))
}

async fn report_service_instance_info() -> Result<()> {
    let service_config =
        VERIFY_SERVICE_CONFIG
            .lock()
            .await
            .as_ref()
            .cloned()
            .ok_or(RPCErrors::ReasonError(
                "verify_hub service config not loaded".to_string(),
            ))?;

    let mut service_ports = HashMap::new();
    service_ports.insert("www".to_string(), VERIFY_HUB_SERVICE_MAIN_PORT);

    let now = buckyos_get_unix_timestamp();
    let instance_epoch = env::var("BUCKYOS_INSTANCE_EPOCH")
        .unwrap_or_else(|_| format!("process:{}", std::process::id()));
    let node_session_id = env::var("BUCKYOS_NODE_SESSION_ID")
        .unwrap_or_else(|_| format!("device:{}", service_config.device_id));
    let instance_info = ServiceInstanceReportInfo {
        node_id: service_config.device_id.clone(),
        node_did: service_config.node_did.clone(),
        state: ServiceInstanceState::Started,
        service_ports,
        last_update_time: now,
        start_time: service_config.start_time,
        pid: std::process::id(),
        deployment: None,
        instance_epoch,
        node_session_id,
        observed_at: now,
        expires_at: now.saturating_add(SERVICE_INSTANCE_INFO_UPDATE_INTERVAL * 3),
        health: DeploymentHealth::Healthy,
        deployment_error: None,
    };

    let system_config_client = get_system_config_client().await?;
    let control_panel_client = ControlPanelClient::new(system_config_client);
    control_panel_client
        .update_service_instance_info(
            VERIFY_HUB_UNIQUE_ID,
            service_config.device_id.as_str(),
            &instance_info,
        )
        .await?;
    info!(
        "verify_hub reported service instance info,node_id:{}",
        service_config.device_id
    );
    Ok(())
}

fn start_service_instance_reporter() {
    tokio::task::spawn(async move {
        if let Err(error) = report_service_instance_info().await {
            warn!(
                "verify_hub initial service instance report failed: {:?}",
                error
            );
        }

        let start = tokio::time::Instant::now()
            + std::time::Duration::from_secs(SERVICE_INSTANCE_INFO_UPDATE_INTERVAL);
        let mut timer = tokio::time::interval_at(
            start,
            std::time::Duration::from_secs(SERVICE_INSTANCE_INFO_UPDATE_INTERVAL),
        );
        loop {
            timer.tick().await;
            if let Err(error) = report_service_instance_info().await {
                warn!("verify_hub service instance report failed: {:?}", error);
            }
        }
    });
}

async fn load_token_from_cache(key: &str) -> Option<RPCSessionToken> {
    let cache = TOKEN_CACHE.lock().await;
    cache.get(key).cloned()
}

async fn cache_token(key: &str, token: RPCSessionToken) {
    TOKEN_CACHE.lock().await.insert(key.to_string(), token);
}

async fn load_trustkey_from_cache(kid: &str) -> Option<TrustedKey> {
    let cache = TRUSTKEY_CACHE.lock().await;
    cache.get(kid).cloned()
}

async fn cache_trustkey(kid: &str, key: DecodingKey, issuer_kind: TrustedIssuerKind) {
    TRUSTKEY_CACHE
        .lock()
        .await
        .insert(kid.to_string(), TrustedKey { key, issuer_kind });
}

async fn remove_trustkey_from_cache(kid: &str) {
    TRUSTKEY_CACHE.lock().await.remove(kid);
}

async fn load_trust_public_key_from_source(iss: &str) -> Result<TrustedKey> {
    let result_key: DecodingKey;
    let issuer_kind: TrustedIssuerKind;
    if iss.starts_with("did:") {
        let agent_did = DID::from_str(iss)
            .map_err(|error| RPCErrors::ReasonError(format!("invalid AgentDID issuer: {error}")))?;
        let agent_id = AgentId::from_agent_did(&agent_did).map_err(RPCErrors::ReasonError)?;
        let system_config_client = get_system_config_client().await?;
        let users = system_config_client
            .list("users")
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let mut resolved = None;
        for owner in users {
            let path = agent_spec_key(&owner, &agent_id);
            let Ok(value) = system_config_client.get(&path).await else {
                continue;
            };
            let spec: AgentSpec = serde_json::from_str(&value.value).map_err(|error| {
                RPCErrors::ReasonError(format!("invalid AgentSpec at {path}: {error}"))
            })?;
            spec.validate().map_err(RPCErrors::ReasonError)?;
            if spec.agent_did != agent_did {
                continue;
            }
            let jwk = spec.agent_doc.get_default_key().ok_or_else(|| {
                RPCErrors::ReasonError("AgentDocument public key not found".to_string())
            })?;
            resolved = Some(
                DecodingKey::from_jwk(&jwk)
                    .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
            );
            break;
        }
        result_key = resolved.ok_or_else(|| {
            RPCErrors::ReasonError(format!("AgentSpec not found for issuer {iss}"))
        })?;
        issuer_kind = TrustedIssuerKind::Agent;
    } else if iss == "root" {
        //load zone config from system config service
        let owner_auth_key = VERIFY_SERVICE_CONFIG
            .lock()
            .await
            .as_ref()
            .unwrap()
            .zone_document
            .get_auth_key(None)
            .ok_or(RPCErrors::ReasonError(
                "Owner public key not found".to_string(),
            ))?;
        result_key = owner_auth_key.0;
        issuer_kind = TrustedIssuerKind::Root;
        info!("load owner public key from zone config");
    } else {
        let system_config_client = get_system_config_client().await?;
        let control_panel_client = ControlPanelClient::new(system_config_client);
        match control_panel_client.get_user_config(iss).await {
            Ok(user_config) => {
                let owner_key = user_config.get_default_key().ok_or(RPCErrors::ReasonError(
                    "User public key not found".to_string(),
                ))?;
                result_key = DecodingKey::from_jwk(&owner_key)
                    .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
                issuer_kind = TrustedIssuerKind::User;
                info!("load user public key from system config for iss={}", iss);
            }
            Err(RPCErrors::KeyNotExist(_)) => {
                //load device config from system config service(not from name-lib)
                let device_config = match control_panel_client.get_device_config(iss).await {
                    Ok(device_config) => device_config,
                    Err(RPCErrors::KeyNotExist(_)) => {
                        warn!("user/device {} config not found in system config", iss);
                        return Err(RPCErrors::ReasonError(
                            "User or device config not found".to_string(),
                        ));
                    }
                    Err(err) => {
                        warn!(
                            "load device {} doc from system config service failed: {}",
                            iss, err
                        );
                        return Err(err);
                    }
                };
                let result_device_key =
                    device_config
                        .get_auth_key(None)
                        .ok_or(RPCErrors::ReasonError(
                            "Device public key not found".to_string(),
                        ))?;
                result_key = result_device_key.0;
                issuer_kind = TrustedIssuerKind::Device;
                info!("load device public key from system config for iss={}", iss);
            }
            Err(err) => {
                warn!(
                    "load user {} doc from system config service failed: {}",
                    iss, err
                );
                return Err(err);
            }
        }
    }

    let trusted_key = TrustedKey {
        key: result_key,
        issuer_kind,
    };
    cache_trustkey(iss, trusted_key.key.clone(), issuer_kind).await;
    Ok(trusted_key)
}

async fn get_trust_public_key(iss: &str, _kid: &Option<String>) -> Result<TrustedKey> {
    let cached_key = load_trustkey_from_cache(iss).await;
    if let Some(cached_key) = cached_key {
        return Ok(cached_key);
    }

    load_trust_public_key_from_source(iss).await
}

// return (kid, payload)
async fn verify_trusted_jwt(jwt: &str) -> Result<(Value, TrustedIssuerKind)> {
    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        error!("JWT decode header error: {}", error);
        RPCErrors::ReasonError("JWT decode header error".to_string())
    })?;

    if header.alg != Algorithm::EdDSA {
        return Err(RPCErrors::ReasonError(
            "JWT algorithm not allowed".to_string(),
        ));
    }

    let mut validation = Validation::new(Algorithm::EdDSA);
    // We don't have an expected audience for generic trusted JWTs.
    validation.validate_aud = false;

    // Get iss from claims by decoding the payload part (without signature verification)
    // JWT format: header.payload.signature
    let claims = decode_jwt_claim_without_verify(jwt).map_err(|error| {
        error!("decode_jwt_claim_without_verify error: {}", error);
        RPCErrors::ReasonError("decode_jwt_claim_without_verify error".to_string())
    })?;

    let iss = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .unwrap_or(VERIFY_HUB_ISSUER);

    // try get public key from header.kid
    let trusted_key = get_trust_public_key(iss, &header.kid).await?;

    // verify jwt
    let decoded_token = match jsonwebtoken::decode::<Value>(jwt, &trusted_key.key, &validation) {
        Ok(decoded_token) => decoded_token,
        Err(error) => {
            let should_retry = matches!(error.kind(), ErrorKind::InvalidSignature);
            if should_retry && iss != VERIFY_HUB_ISSUER {
                warn!(
                    "JWT verify failed for iss={}, retrying after trust-key reload: {}",
                    iss, error
                );
                remove_trustkey_from_cache(iss).await;
                let refreshed_key = load_trust_public_key_from_source(iss).await?;
                jsonwebtoken::decode::<Value>(jwt, &refreshed_key.key, &validation).map_err(
                    |retry_error| {
                        error!(
                            "JWT verify error after trust-key reload for iss={}: {}",
                            iss, retry_error
                        );
                        RPCErrors::ReasonError("JWT verify error".to_string())
                    },
                )?
            } else {
                error!("JWT verify error: {}", error);
                return Err(RPCErrors::ReasonError("JWT verify error".to_string()));
            }
        }
    };

    Ok((decoded_token.claims, trusted_key.issuer_kind))
}

async fn verify_verify_hub_jwt(jwt: &str, expected_audience: Option<&str>) -> Result<Value> {
    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        error!("JWT decode header error: {}", error);
        RPCErrors::ReasonError("JWT decode header error".to_string())
    })?;

    if header.alg != Algorithm::EdDSA {
        return Err(RPCErrors::ReasonError(
            "JWT algorithm not allowed".to_string(),
        ));
    }

    let claims = decode_jwt_claim_without_verify(jwt).map_err(|error| {
        error!("decode_jwt_claim_without_verify error: {}", error);
        RPCErrors::ReasonError("decode_jwt_claim_without_verify error".to_string())
    })?;

    let iss = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .unwrap_or(VERIFY_HUB_ISSUER);

    if iss != VERIFY_HUB_ISSUER {
        return Err(RPCErrors::ReasonError("JWT kid not allowed".to_string()));
    }

    // Always verify verify-hub issued tokens by verify-hub public key
    let public_key = get_trust_public_key(VERIFY_HUB_ISSUER, &None).await?;

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[VERIFY_HUB_ISSUER]);
    validation.set_required_spec_claims(&["exp", "iss"]);

    if let Some(aud) = expected_audience {
        validation.set_audience(&[aud]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    } else {
        validation.validate_aud = false;
    }

    let decoded_token =
        jsonwebtoken::decode::<Value>(jwt, &public_key.key, &validation).map_err(|error| {
            error!("JWT verify error: {}", error);
            RPCErrors::ReasonError("JWT verify error".to_string())
        })?;

    Ok(decoded_token.claims)
}

async fn validate_password_login(
    username: &str,
    password: &str,
    target: &AuthTarget,
    login_nonce: u64,
) -> Result<(UserSettings, String)> {
    let now = buckyos_get_unix_timestamp() * 1000;
    let abs_diff = now.abs_diff(login_nonce);
    debug!(
        "{} login nonce and now abs_diff:{}, from:{}",
        username,
        abs_diff,
        target.canonical_key()
    );
    if abs_diff > MAX_LOGIN_NONCE_AGE_SECONDS {
        warn!(
            "{} login nonce is too old, abs_diff:{}, this is a possible ATTACK?",
            username, abs_diff
        );
        return Err(RPCErrors::ParseRequestError("Invalid nonce".to_string()));
    }

    let session_key = format!("{}_{}_{}", username, target.canonical_key(), login_nonce);
    let cache_result = load_token_from_cache(session_key.as_str()).await;
    if cache_result.is_some() {
        warn!(
            "{} login nonce {} already used, this is a REPLAY ATTACK!",
            username, login_nonce
        );
        warn!(
            "Revoking session {} due to replay attack detection",
            session_key
        );
        revoke_session_tokens(session_key.as_str()).await;
        return Err(RPCErrors::ReasonError(
            "Login nonce already used".to_string(),
        ));
    }

    let system_config_client = get_system_config_client().await?;
    let control_panel_client = ControlPanelClient::new(system_config_client);
    let user_settings = control_panel_client
        .get_user_settings_by_username(username)
        .await?;

    let password_hash_input = STANDARD
        .decode(password)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

    let salt = format!("{}{}", user_settings.password, login_nonce);
    let hash = Sha256::digest(salt.clone()).to_vec();
    if hash != password_hash_input {
        warn!("{} login by password failed, password is wrong!", username);
        return Err(RPCErrors::InvalidPassword);
    }

    Ok((user_settings, session_key))
}

/**
curl -X POST http://127.0.0.1/kapi/verify_hub -H "Content-Type: application/json" -d '{"method": "login","params":{"type":"jwt","jwt":"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJodHRwczovL3d3dy53aGl0ZS5ib3Vjay5pbyIsImF1ZCI6Imh0dHBzOi8vd3d3LndoaXRlLmJvdWNrLmlvIiwiZXhwIjoxNzI3NzIwMDAwLCJpYXQiOjE3Mjc3MTY0MDAsInVzZXJpZCI6ImRpZDpleGFtcGxlOjEyMzQ1Njc4OTAiLCJhcHBpZCI6InN5c3RvbSIsInVzZXJuYW1lIjoiYWxpY2UifQ.6XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ5"}}'
curl -X POST http://127.0.0.1:3300/kapi/verify_hub -H "Content-Type: application/json" -d '{"method": "login","params":{"type":"password","username":"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJodHRwczovL3d3dy53aGl0ZS5ib3Vjay5pbyIsImF1ZCI6Imh0dHBzOi8vd3d3LndoaXRlLmJvdWNrLmlvIiwiZXhwIjoxNzI3NzIwMDAwLCJpYXQiOjE3Mjc3MTY0MDAsInVzZXJpZCI6ImRpZDpleGFtcGxlOjEyMzQ1Njc4OTAiLCJhcHBpZCI6InN5c3RvbSIsInVzZXJuYW1lIjoiYWxpY2UifQ.6XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ5"}}'
 */
#[derive(Clone)]
struct VerifyHubServer {}

impl VerifyHubServer {
    fn new() -> Self {
        VerifyHubServer {}
    }
}

#[async_trait]
impl VerifyHubApiHandler for VerifyHubServer {
    async fn handle_login_by_jwt(&self, jwt: &str, target: &AuthTarget) -> Result<TokenPair> {
        gc_token_caches().await;

        // Step 1: Verify JWT signature (include exp) and extract payload
        // The incoming JWT is signed by a trusted entity (device/owner)
        let (jwt_payload, issuer_kind) = verify_trusted_jwt(jwt).await?;
        if issuer_kind == TrustedIssuerKind::VerifyHub {
            return Err(RPCErrors::InvalidToken(
                "verify-hub session tokens cannot be exchanged as LoginAssertions".to_string(),
            ));
        }

        // Step 2: Extract required fields from JWT payload
        let rpc_session_token: RPCSessionToken =
            serde_json::from_value(jwt_payload).map_err(|error| {
                error!(
                    "Failed to parse RPCSessionToken from JWT payload: {}",
                    error
                );
                RPCErrors::ReasonError(
                    "Failed to parse RPCSessionToken from JWT payload".to_string(),
                )
            })?;
        let userid = rpc_session_token
            .sub
            .clone()
            .ok_or(RPCErrors::ReasonError("Missing sub".to_string()))?;
        reject_root_session_subject(userid.as_str())?;
        let appid = rpc_session_token
            .appid
            .clone()
            .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
        let token_jti = rpc_session_token
            .jti
            .clone()
            .ok_or(RPCErrors::ReasonError("Missing jti".to_string()))?;
        if rpc_session_token.extra.contains_key(TOKEN_USE_CLAIM)
            || rpc_session_token
                .extra
                .contains_key(TOKEN_TARGET_KIND_CLAIM)
        {
            return Err(RPCErrors::InvalidToken(
                "LoginAssertion cannot contain session target/use claims".to_string(),
            ));
        }
        if appid != target.appid_claim() {
            return Err(RPCErrors::InvalidToken(
                "LoginAssertion appid does not match requested target".to_string(),
            ));
        }
        let principal_kind = if issuer_kind == TrustedIssuerKind::Device {
            if rpc_session_token.iss.as_deref() == Some(userid.as_str()) {
                SessionPrincipalKind::Device
            } else if matches!(target, AuthTarget::App { .. }) {
                SessionPrincipalKind::App
            } else {
                SessionPrincipalKind::System
            }
        } else if issuer_kind == TrustedIssuerKind::Agent {
            if userid != rpc_session_token.iss.as_deref().unwrap_or_default() {
                return Err(RPCErrors::InvalidToken(
                    "Agent token sub and iss must both be the canonical AgentDID".to_string(),
                ));
            }
            SessionPrincipalKind::Agent
        } else {
            SessionPrincipalKind::User
        };
        let target = if principal_kind == SessionPrincipalKind::User {
            resolve_user_auth_target(&userid, target).await?
        } else {
            target.clone()
        };
        if principal_kind == SessionPrincipalKind::App {
            check_app_principal_target(&target).await?;
        }
        //let token_jti = rpc_session_token.jti.ok_or(RPCErrors::ReasonError("Missing jti".to_string()))?;

        // ============================================================
        // FIRST LOGIN FLOW: Using trusted device/owner JWT
        // The incoming JWT is signed by a trusted entity (device/owner)
        // ============================================================
        info!(
            "Handle LoginAssertion exchange for sub: {}, target: {}",
            userid,
            target.canonical_key()
        );

        let assertion_issuer = rpc_session_token
            .iss
            .as_deref()
            .ok_or_else(|| RPCErrors::InvalidToken("LoginAssertion missing issuer".to_string()))?;
        let session_key = format!("assertion:{assertion_issuer}:{userid}:{token_jti}");

        // Step 4: Check if this login JWT has already been used (replay protection)
        let cache_result = load_token_from_cache(session_key.as_str()).await;
        if cache_result.is_some() {
            return Err(RPCErrors::ReasonError("Login JWT already used".to_string()));
        }

        // Step 5: Generate new session_id for this login session
        let session_id: u64;
        {
            let mut rng = rand::thread_rng();
            session_id = rng.gen::<u64>();
        }
        let new_session_key = format!("{}_{}_{}", userid, target.canonical_key(), session_id);

        // Step 6: Generate new token pair (session_token + refresh_token)
        let (token_pair, session_token, refresh_token) =
            generate_token_pair(&target, userid.as_str(), session_id, principal_kind).await?;

        // Step 7: Cache both tokens
        // Cache by original session_key to mark login JWT as used
        cache_token(session_key.as_str(), session_token.clone()).await;
        // Cache by new session_key for future refresh operations
        cache_refresh_family(new_session_key.as_str(), &session_token, refresh_token).await;
        cache_token(new_session_key.as_str(), session_token).await;

        info!(
            "Login successful for user: {}. Session: {}. Token pair generated.",
            userid, new_session_key
        );

        Ok(buckyos_api::TokenPair {
            session_token: token_pair.session_token,
            refresh_token: token_pair.refresh_token,
        })
    }

    async fn handle_refresh_token(&self, refresh_jwt: &str) -> Result<TokenPair> {
        gc_token_caches().await;
        let presented = verify_presented_refresh(refresh_jwt).await?;
        if let Some(token_pair) = check_presented_refresh(&presented).await? {
            info!(
                "Refresh within grace period for session: {}, returning current token pair",
                presented.session_key
            );
            return Ok(token_pair);
        }
        let userid = presented
            .token
            .sub
            .clone()
            .ok_or(RPCErrors::ReasonError("Missing sub".to_string()))?;
        let session_id = get_token_session_id(&presented.token)?;
        let claims = validate_verify_hub_token_claims(&presented.token, TokenUse::Refresh)?;
        let principal_kind = claims.principal_kind;
        validate_refresh_principal(userid.as_str(), principal_kind).await?;
        let target = if principal_kind == SessionPrincipalKind::User {
            resolve_user_auth_target(&userid, &claims.target).await?
        } else {
            claims.target
        };
        if principal_kind == SessionPrincipalKind::App {
            check_app_principal_target(&target).await?;
        }

        info!(
            "Handle refresh token request for session: {}",
            presented.session_key
        );

        let (token_pair, session_token, refresh_token) =
            generate_token_pair(&target, userid.as_str(), session_id, principal_kind).await?;

        match commit_refresh_rotation(
            presented.session_key.as_str(),
            presented.jti.as_str(),
            &session_token,
            refresh_token,
        )
        .await
        {
            RefreshCommit::Committed => {
                cache_token(presented.session_key.as_str(), session_token).await;
                info!(
                    "Refresh successful for session: {}. New token pair generated.",
                    presented.session_key
                );
                Ok(token_pair)
            }
            RefreshCommit::Grace(current_pair) => {
                info!(
                    "Concurrent refresh for session: {} already rotated, returning current token pair",
                    presented.session_key
                );
                Ok(current_pair)
            }
            RefreshCommit::LoggedOut => {
                warn!(
                    "Refresh for session: {} lost to a logout, not committing",
                    presented.session_key
                );
                Err(RPCErrors::ReasonError(
                    "Refresh token not found or already invalidated".to_string(),
                ))
            }
            RefreshCommit::Reused => {
                warn!(
                    "Refresh reuse detected (concurrent rotation), revoking session: {}",
                    presented.session_key
                );
                revoke_session_tokens(presented.session_key.as_str()).await;
                Err(RPCErrors::ReasonError(
                    "Invalid refresh token jti".to_string(),
                ))
            }
        }
    }

    async fn handle_logout(&self, refresh_jwt: &str) -> Result<bool> {
        gc_token_caches().await;
        let presented = verify_presented_refresh(refresh_jwt).await?;
        check_presented_refresh(&presented).await?;

        info!(
            "Handle logout request for session: {}",
            presented.session_key
        );
        revoke_session_tokens(presented.session_key.as_str()).await;
        info!("Logout successful for session: {}", presented.session_key);

        Ok(true)
    }

    async fn handle_login_by_password(
        &self,
        username: &str,
        password: &str,
        target: &AuthTarget,
        login_nonce: u64,
    ) -> Result<LoginByPasswordResponse> {
        gc_token_caches().await;
        reject_root_session_subject(username)?;

        let session_id = login_nonce;
        let (user_settings, session_key) =
            validate_password_login(username, password, target, login_nonce).await?;
        reject_root_user_settings(&user_settings)?;
        require_active_user_settings(&user_settings)?;
        let target = resolve_user_auth_target(username, target).await?;

        info!(
            "Password login successful for user: {}. Generating token pair.",
            username
        );

        // Step 5: Generate token pair (session_token + refresh_token)
        // session_token: short-lived (15 minutes) for API requests
        // refresh_token: long-lived (7 days) for obtaining new token pairs
        let (token_pair, session_token, refresh_token) =
            generate_token_pair(&target, username, session_id, SessionPrincipalKind::User).await?;

        // Step 6: Cache both tokens
        cache_refresh_family(session_key.as_str(), &session_token, refresh_token).await;
        cache_token(session_key.as_str(), session_token).await;

        info!("Token pair cached for session: {}", session_key);

        let user_info = user_settings.to_user_info();

        // Step 8: Return account info with dual tokens
        let result_account_info = LoginByPasswordResponse {
            user_info,
            session_token: token_pair.session_token,
            refresh_token: token_pair.refresh_token,
        };
        return Ok(result_account_info);
    }

    async fn handle_sudo_by_password(
        &self,
        username: &str,
        password: &str,
        target: &AuthTarget,
        aud: Option<String>,
        login_nonce: u64,
    ) -> Result<SudoByPasswordResponse> {
        gc_token_caches().await;
        reject_root_session_subject(username)?;

        let session_id = login_nonce;
        let (user_settings, session_key) =
            validate_password_login(username, password, target, login_nonce).await?;
        reject_root_user_settings(&user_settings)?;
        require_active_user_settings(&user_settings)?;
        let target = resolve_user_auth_target(username, target).await?;

        let session_jti: u64;
        {
            let mut rng = rand::thread_rng();
            session_jti = rng.gen::<u64>();
        }

        let session_token = generate_session_token(
            &target,
            username,
            session_jti,
            session_id,
            SUDO_SESSION_TOKEN_EXPIRE_SECONDS,
            aud,
            true,
            SessionPrincipalKind::User,
        )
        .await?;

        cache_token(session_key.as_str(), session_token.clone()).await;
        info!("Sudo session token cached for session: {}", session_key);

        Ok(SudoByPasswordResponse {
            session_token: session_token.to_string(),
        })
    }

    async fn handle_delegate_user_session(
        &self,
        caller_token: Option<&str>,
        source_session_token: &str,
        target: &AuthTarget,
    ) -> Result<DelegateUserSessionResult> {
        gc_token_caches().await;
        authorize_delegate_caller(caller_token).await?;
        if !matches!(target, AuthTarget::App { .. }) {
            return Err(RPCErrors::NoPermission(
                "delegate_user_session only issues App targets".to_string(),
            ));
        }
        let Some(source) = verify_delegation_source(source_session_token).await? else {
            return Ok(DelegateUserSessionResult::SourceInactive);
        };
        if !source_family_is_live(source.session_key.as_str()).await {
            info!(
                "delegation source session is not active: {}",
                source.session_key
            );
            return Ok(DelegateUserSessionResult::SourceInactive);
        }

        let resolver = AppAvailabilityResolver::new(Arc::new(get_system_config_client().await?));
        match resolver.get_user_settings(source.user_id.as_str()).await {
            Ok(_) => {}
            Err(RPCErrors::KeyNotExist(_) | RPCErrors::NoPermission(_)) => {
                return Ok(DelegateUserSessionResult::SourceInactive);
            }
            Err(error) => return Err(error),
        }
        let target = match resolve_user_auth_target(source.user_id.as_str(), target).await {
            Ok(target) => target,
            Err(RPCErrors::NoPermission(_)) => {
                return Ok(DelegateUserSessionResult::AccessDenied);
            }
            Err(error) => return Err(error),
        };

        let session_id: u64 = rand::thread_rng().gen();
        let session_key = format!(
            "{}_{}_{}",
            source.user_id,
            target.canonical_key(),
            session_id
        );
        let (token_pair, session_token, refresh_token) = generate_token_pair(
            &target,
            source.user_id.as_str(),
            session_id,
            SessionPrincipalKind::User,
        )
        .await?;
        if !commit_delegated_family(
            source.session_key.as_str(),
            session_key.as_str(),
            &session_token,
            refresh_token,
        )
        .await
        {
            info!(
                "delegation source session ended before commit: {}",
                source.session_key
            );
            return Ok(DelegateUserSessionResult::SourceInactive);
        }
        cache_token(session_key.as_str(), session_token).await;
        info!(
            "Delegated session of {} to {}. Session: {}",
            source.session_key,
            target.canonical_key(),
            session_key
        );
        Ok(DelegateUserSessionResult::Issued(token_pair))
    }

    async fn handle_verify_token(
        &self,
        session_token: &str,
        expected_target: Option<AuthTarget>,
    ) -> Result<bool> {
        gc_token_caches().await;
        let first_dot = session_token.find('.');
        if first_dot.is_none() {
            //this is not a jwt token, use token-store to verify
            return Err(RPCErrors::InvalidToken("not a jwt token".to_string()));
        } else {
            let json_body = verify_verify_hub_jwt(session_token, None).await?;
            let rpc_session_token: RPCSessionToken =
                serde_json::from_value(json_body).map_err(|error| {
                    error!(
                        "Failed to parse RPCSessionToken from JWT payload: {}",
                        error
                    );
                    RPCErrors::ReasonError(
                        "Failed to parse RPCSessionToken from JWT payload".to_string(),
                    )
                })?;

            if let Some(userid) = rpc_session_token.sub.as_deref() {
                reject_root_session_subject(userid)?;
            }

            let claims = validate_verify_hub_token_claims(&rpc_session_token, TokenUse::Session)?;
            if let Some(expected_target) = expected_target {
                if claims.target != expected_target {
                    return Err(RPCErrors::InvalidToken(
                        "authentication target mismatch".to_string(),
                    ));
                }
            }

            Ok(true)
        }
    }
}

#[async_trait]
impl HttpServer for VerifyHubServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            let rpc_handler = VerifyHubRpcHandler::new(self.clone());
            return serve_http_by_rpc_handler(req, info, &rpc_handler).await;
        }
        return Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ));
    }

    fn id(&self) -> String {
        "verify-hub-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

async fn load_service_config() -> Result<()> {
    info!("start load config from system config service.");
    let session_token = env::var("VERIFY_HUB_SESSION_TOKEN")
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let device_rpc_token = RPCSessionToken::from_string(session_token.as_str())?;
    let device_id = device_rpc_token
        .sub
        .ok_or(RPCErrors::ReasonError("device id not found".to_string()))?;
    info!("This device_id:{}", device_id);

    let system_config_client = SystemConfigClient::new(None, Some(session_token.as_str()));

    //load verify-hub private key from system config service
    let private_key_str = system_config_client.get("security/verify-hub/key").await;
    if let Ok(private_key_str) = private_key_str {
        let private_key = private_key_str.value;
        let private_key = EncodingKey::from_ed_pem(private_key.as_bytes());
        if let Ok(private_key) = private_key {
            let mut verify_hub_private_key = VERIFY_HUB_PRIVATE_KEY.write().await;
            *verify_hub_private_key = private_key;
        } else {
            warn!("verify_hub private key format error!");
            return Err(RPCErrors::ReasonError(
                "verify_hub private key format error".to_string(),
            ));
        }
    } else {
        warn!("verify_hub private key cann't load from system config service!");
        return Err(RPCErrors::ReasonError(
            "verify_hub private key cann't load from system config service".to_string(),
        ));
    }
    info!("verify_hub private key loaded from system config service OK!");

    let control_panel_client = ControlPanelClient::new(system_config_client);
    let zone_config = control_panel_client.load_zone_config().await;
    if zone_config.is_err() {
        warn!(
            "zone config cann't load from system config service,use default zone config for test!"
        );
        return Err(RPCErrors::ReasonError(
            "zone config cann't load from system config service".to_string(),
        ));
    }
    let zone_config = zone_config.unwrap();
    if zone_config.verify_hub_info.is_none() {
        warn!("zone config verify_hub_info not found!");
        return Err(RPCErrors::ReasonError(
            "zone config verify_hub_info not found".to_string(),
        ));
    }
    let verify_hub_info = zone_config.verify_hub_info.as_ref().unwrap();
    let verify_hub_pub_key = DecodingKey::from_jwk(&verify_hub_info.public_key)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    cache_trustkey(
        "verify-hub",
        verify_hub_pub_key,
        TrustedIssuerKind::VerifyHub,
    )
    .await;
    info!("verify_hub public key loaded from system config service OK!");
    let zone_document = zone_config
        .zone_document()
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

    let device_info = control_panel_client
        .get_device_info(device_id.as_str())
        .await?;
    let new_service_config = VerifyServiceConfig {
        zone_document,
        device_id,
        node_did: device_info.id.clone(),
        start_time: buckyos_get_unix_timestamp(),
    };

    {
        let mut service_config = VERIFY_SERVICE_CONFIG.lock().await;
        if service_config.is_some() {
            return Ok(());
        }
        service_config.replace(new_service_config);
    }

    info!("verify_hub load_service_config success!");
    Ok(())
}

async fn service_main() -> i32 {
    init_logging("verify_hub", true);
    info!("Starting verify_hub service...");
    //init service config from system config service and env
    let mut service_config_loaded = true;
    if let Err(error) = load_service_config().await {
        service_config_loaded = false;
        error!("load service config failed:{}", error);
        if !cfg!(test) {
            return -1;
        }
        warn!("cfg(test) enabled: continue running with test defaults");
    }
    //load cache from service_cache@dfs:// and service_local_cache@fs://

    if service_config_loaded {
        start_service_instance_reporter();
    }

    let server = VerifyHubServer::new();
    info!(
        "verify_hub service initialized, running on port {}",
        VERIFY_HUB_SERVICE_MAIN_PORT
    );
    let runner = Runner::new(VERIFY_HUB_SERVICE_MAIN_PORT);
    let _ = runner.add_http_server("/kapi/verify-hub".to_string(), Arc::new(server));
    let _ = runner.run().await;
    0
}

#[tokio::main]
async fn main() {
    service_main().await;
}

#[cfg(test)]
mod test {
    use super::*;

    use serde_json::json;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tokio::task;
    use tokio::time::sleep;

    fn system_target(service_id: &str) -> AuthTarget {
        AuthTarget::system(service_id.parse().unwrap())
    }

    fn app_target(app_instance_id: &str) -> AuthTarget {
        AuthTarget::app(app_instance_id.parse().unwrap())
    }

    /// Helper function to setup test environment
    /// Initializes trust keys for verify-hub and root
    async fn setup_test_environment() -> EncodingKey {
        let test_jwk = json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc",
        });
        let public_key_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(test_jwk).unwrap();
        let test_pk = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        // Cache trust keys for verify-hub and root
        cache_trustkey("verify-hub", test_pk.clone(), TrustedIssuerKind::VerifyHub).await;
        cache_trustkey("root", test_pk.clone(), TrustedIssuerKind::Root).await;
        cache_trustkey("ood1", test_pk, TrustedIssuerKind::Device).await;

        let test_owner_private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;
        EncodingKey::from_ed_pem(test_owner_private_key_pem.as_bytes()).unwrap()
    }

    #[tokio::test]
    async fn service_login_preserves_owner_subject() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();

        let token_pair = VerifyHubServer::new()
            .handle_login_by_jwt(login_jwt.as_str(), &system_target("control-panel"))
            .await
            .unwrap();
        let session_token = RPCSessionToken::from_string(&token_pair.session_token).unwrap();
        let refresh_token = RPCSessionToken::from_string(&token_pair.refresh_token).unwrap();

        assert_eq!(session_token.sub.as_deref(), Some("alice"));
        assert_eq!(session_token.appid.as_deref(), Some("control-panel"));
        assert_eq!(refresh_token.sub.as_deref(), Some("alice"));
        assert_eq!(refresh_token.appid.as_deref(), Some("control-panel"));
        assert_eq!(
            token_principal_kind(&refresh_token).unwrap(),
            SessionPrincipalKind::System
        );
    }

    #[tokio::test]
    async fn device_service_login_preserves_device_subject() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("ood1", "node-daemon", "ood1", &private_key).unwrap();

        let token_pair = VerifyHubServer::new()
            .handle_login_by_jwt(login_jwt.as_str(), &system_target("node-daemon"))
            .await
            .unwrap();
        let session_token = RPCSessionToken::from_string(&token_pair.session_token).unwrap();
        let refresh_token = RPCSessionToken::from_string(&token_pair.refresh_token).unwrap();

        assert_eq!(session_token.sub.as_deref(), Some("ood1"));
        assert_eq!(session_token.appid.as_deref(), Some("node-daemon"));
        assert_eq!(
            token_principal_kind(&session_token).unwrap(),
            SessionPrincipalKind::Device
        );
        assert_eq!(
            token_principal_kind(&refresh_token).unwrap(),
            SessionPrincipalKind::Device
        );
    }

    /// Helper function to create a login JWT for testing
    fn create_login_jwt(
        private_key: &EncodingKey,
        userid: &str,
        appid: &str,
        jti: u64,
        exp: u64,
    ) -> String {
        let test_login_token = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            jti: Some(jti.to_string()),
            appid: Some(appid.to_string()),
            aud: None,
            sub: Some(userid.to_string()),
            token: None,
            iss: Some("ood1".to_string()),
            exp: Some(exp),
            sudo: false,
            extra: HashMap::new(),
        };

        test_login_token
            .generate_jwt(Some("ood1".to_string()), private_key)
            .unwrap()
    }

    #[tokio::test]
    async fn test_verify_hub_client_login_and_verify_token() {
        let private_key = setup_test_environment().await;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut rng = rand::thread_rng();
        let login_nonce = rng.gen::<u64>();

        let test_jwt = create_login_jwt(&private_key, "alice", "kernel", login_nonce, now + 3600);

        let handler = VerifyHubServer::new();
        let verify_hub_client = VerifyHubClient::new_in_process(Box::new(handler));

        let token_pair = verify_hub_client
            .login_by_jwt(test_jwt.as_str(), system_target("kernel"))
            .await
            .expect("login_by_jwt should succeed");
        assert!(!token_pair.session_token.is_empty());
        assert!(!token_pair.refresh_token.is_empty());

        let verify_ok = verify_hub_client
            .verify_token(&token_pair.session_token, Some(system_target("kernel")))
            .await;
        assert!(
            verify_ok.is_ok(),
            "verify_token should succeed for correct appid"
        );

        let verify_bad = verify_hub_client
            .verify_token(&token_pair.session_token, Some(system_target("not-kernel")))
            .await;
        assert!(
            verify_bad.is_err(),
            "verify_token should reject wrong appid"
        );
    }

    #[tokio::test]
    async fn test_verify_hub_rejects_root_login_jwt() {
        let private_key = setup_test_environment().await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let test_jwt = create_login_jwt(&private_key, "root", "kernel", 9001, now + 3600);
        let handler = VerifyHubServer::new();
        let login_result = handler
            .handle_login_by_jwt(test_jwt.as_str(), &system_target("kernel"))
            .await;

        assert!(
            matches!(login_result, Err(RPCErrors::NoPermission(_))),
            "verify-hub must reject root login JWT"
        );
    }

    #[tokio::test]
    async fn test_verify_hub_rejects_root_password_entrypoints() {
        let handler = VerifyHubServer::new();
        let target = system_target("control-panel");

        let login_result = handler
            .handle_login_by_password("root", "not-used", &target, 9002)
            .await;
        assert!(
            matches!(login_result, Err(RPCErrors::NoPermission(_))),
            "verify-hub must reject root password login"
        );

        let sudo_result = handler
            .handle_sudo_by_password(
                "root",
                "not-used",
                &target,
                Some("system-config".to_string()),
                9003,
            )
            .await;
        assert!(
            matches!(sudo_result, Err(RPCErrors::NoPermission(_))),
            "verify-hub must reject root sudo password login"
        );
    }

    #[test]
    fn only_active_users_can_receive_or_refresh_tokens() {
        let settings = |state| UserSettings {
            user_id: "alice".to_string(),
            user_type: UserType::User,
            password: "unused".to_string(),
            state,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: Some(true),
        };

        assert!(require_active_user_settings(&settings(UserState::Active)).is_ok());
        let mut guest = settings(UserState::Active);
        guest.user_type = UserType::Guest;
        assert!(matches!(
            require_active_user_settings(&guest),
            Err(RPCErrors::NoPermission(_))
        ));
        for state in [
            UserState::Pending,
            UserState::Suspended("test".to_string()),
            UserState::Deleted,
            UserState::Banned("test".to_string()),
        ] {
            assert!(matches!(
                require_active_user_settings(&settings(state)),
                Err(RPCErrors::NoPermission(_))
            ));
        }
    }

    #[tokio::test]
    async fn test_root_token_generation_rejected() {
        let target = system_target("control-panel");
        let session_result = generate_session_token(
            &target,
            "root",
            5678,
            12345,
            SESSION_TOKEN_EXPIRE_SECONDS,
            None,
            false,
            SessionPrincipalKind::User,
        )
        .await;
        assert!(
            matches!(session_result, Err(RPCErrors::NoPermission(_))),
            "root session token generation must fail"
        );

        let refresh_result = generate_refresh_token(
            &target,
            "root",
            5679,
            12345,
            REFRESH_TOKEN_EXPIRE_SECONDS,
            SessionPrincipalKind::User,
        )
        .await;
        assert!(
            matches!(refresh_result, Err(RPCErrors::NoPermission(_))),
            "root refresh token generation must fail"
        );

        let token_pair_result =
            generate_token_pair(&target, "root", 12345, SessionPrincipalKind::User).await;
        assert!(
            matches!(token_pair_result, Err(RPCErrors::NoPermission(_))),
            "root token pair generation must fail"
        );
    }

    #[tokio::test]
    async fn test_verify_token_rejects_verify_hub_root_token() {
        setup_test_environment().await;
        let mut token = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            jti: Some("root-session".to_string()),
            appid: Some("kernel".to_string()),
            aud: None,
            sub: Some("root".to_string()),
            token: None,
            iss: Some(VERIFY_HUB_ISSUER.to_string()),
            exp: Some(buckyos_get_unix_timestamp() + 3600),
            sudo: false,
            extra: HashMap::new(),
        };
        set_token_session_id(&mut token, 12345);
        {
            let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
            let jwt = token
                .generate_jwt(Some(VERIFY_HUB_ISSUER.to_string()), &private_key)
                .unwrap();
            token.token = Some(jwt);
        }

        let handler = VerifyHubServer::new();
        let verify_result = handler
            .handle_verify_token(token.to_string().as_str(), None)
            .await;
        assert!(
            matches!(verify_result, Err(RPCErrors::NoPermission(_))),
            "verify-hub issued root session token must not be accepted"
        );
    }

    //TEST login by password

    /// Test dual token login and refresh flow
    /// This test verifies:
    /// 1. First login returns token pair (session_token + refresh_token)
    /// 2. Session token can be verified
    /// 3. Refresh token can be used to get new token pair
    /// 4. Old refresh token is invalidated after use
    /// 5. Expired login JWT is rejected

    #[tokio::test]
    async fn test_login_and_verify() {
        // ============================================================
        // Setup test environment
        // ============================================================
        let server = task::spawn(async {
            service_main().await;
        });

        sleep(Duration::from_millis(100)).await;
        let private_key = setup_test_environment().await;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut rng = rand::thread_rng();
        let login_nonce = rng.gen::<u64>();

        let handler = VerifyHubServer::new();

        // ============================================================
        // Test 1: First login with trusted device JWT
        // Expected: Returns token pair {session_token, refresh_token}
        // ============================================================
        println!("\n=== Test 1: First login ===");

        let test_jwt = create_login_jwt(&private_key, "alice", "kernel", login_nonce, now + 3600);
        let kernel_target = system_target("kernel");

        let login_result = handler
            .handle_login_by_jwt(test_jwt.as_str(), &kernel_target)
            .await;

        assert!(login_result.is_ok(), "First login should succeed");
        let token_pair = login_result.unwrap();

        println!("First login successful!");
        println!("  session_token: {}...", &token_pair.session_token[..50]);
        println!("  refresh_token: {}...", &token_pair.refresh_token[..50]);

        // ============================================================
        // Test 2: Verify session token
        // Expected: Session token should be valid
        // ============================================================
        println!("\n=== Test 2: Verify session token ===");

        let verify_result = handler
            .handle_verify_token(token_pair.session_token.as_str(), None)
            .await;

        assert!(verify_result.is_ok(), "Session token should be valid");
        println!("Session token verified successfully!");
        println!("  payload: {:?}", verify_result.unwrap());

        // ============================================================
        // Test 3: Refresh using refresh_token
        // Expected: Returns new token pair, old refresh token invalidated
        // ============================================================
        println!("\n=== Test 3: Refresh using refresh_token ===");

        let refresh_result = handler
            .handle_refresh_token(token_pair.refresh_token.as_str())
            .await;

        assert!(refresh_result.is_ok(), "Refresh should succeed");
        let new_token_pair = refresh_result.unwrap();

        println!("Refresh successful!");
        println!(
            "  new session_token: {}...",
            &new_token_pair.session_token[..50]
        );
        println!(
            "  new refresh_token: {}...",
            &new_token_pair.refresh_token[..50]
        );

        // Verify new session token is different from old one
        assert_ne!(
            token_pair.session_token, new_token_pair.session_token,
            "New session token should be different"
        );
        assert_ne!(
            token_pair.refresh_token, new_token_pair.refresh_token,
            "New refresh token should be different"
        );

        // ============================================================
        // Test 4: Verify new session token
        // Expected: New session token should be valid
        // ============================================================
        println!("\n=== Test 4: Verify new session token ===");

        let verify_new_result = handler
            .handle_verify_token(new_token_pair.session_token.as_str(), None)
            .await;

        assert!(
            verify_new_result.is_ok(),
            "New session token should be valid"
        );
        println!("New session token verified successfully!");

        // ============================================================
        // Test 5: Logout using the current refresh token
        // Expected: Logout succeeds and the refresh token becomes unusable
        // ============================================================
        println!("\n=== Test 5: Logout using refresh token ===");

        let logout_result = handler
            .handle_logout(new_token_pair.refresh_token.as_str())
            .await;

        assert!(logout_result.is_ok(), "Logout should succeed");
        assert!(logout_result.unwrap(), "Logout should return true");
        println!("Logout successful!");

        // ============================================================
        // Test 6: Reuse old refresh token from before refresh (should fail)
        // Expected: Old refresh token is invalidated, reuse should fail
        // ============================================================
        println!("\n=== Test 6: Reuse old refresh token (should fail) ===");

        let reuse_result = handler
            .handle_refresh_token(token_pair.refresh_token.as_str())
            .await;

        assert!(
            reuse_result.is_err(),
            "Reusing old refresh token should fail"
        );
        println!(
            "Old refresh token correctly rejected: {:?}",
            reuse_result.err()
        );

        // ============================================================
        // Test 7: Refresh after logout with latest refresh token
        // Expected: Should fail because logout revoked the session refresh token
        // ============================================================
        println!("\n=== Test 7: Refresh after logout (should fail) ===");

        let second_refresh_result = handler
            .handle_refresh_token(new_token_pair.refresh_token.as_str())
            .await;

        assert!(
            second_refresh_result.is_err(),
            "Refresh should fail after logout"
        );
        println!(
            "Refresh after logout correctly rejected: {:?}",
            second_refresh_result.err()
        );

        // ============================================================
        // Test 8: Login with expired JWT (should fail)
        // Expected: Expired login JWT should be rejected
        // ============================================================
        println!("\n=== Test 8: Login with expired JWT (should fail) ===");

        let expired_nonce = rng.gen::<u64>();
        let expired_jwt = create_login_jwt(
            &private_key,
            "alice",
            "kernel",
            expired_nonce,
            now - 100, // Expired 100 seconds ago
        );

        let expired_result = handler
            .handle_login_by_jwt(expired_jwt.as_str(), &kernel_target)
            .await;

        assert!(expired_result.is_err(), "Expired JWT login should fail");
        println!("Expired JWT correctly rejected: {:?}", expired_result.err());

        // ============================================================
        // Test 8: Replay attack - reuse same login JWT (should fail)
        // Expected: Same login JWT cannot be used twice
        // ============================================================
        println!("\n=== Test 8: Replay attack - reuse login JWT (should fail) ===");

        // First use the JWT
        let replay_nonce = rng.gen::<u64>();
        let replay_jwt = create_login_jwt(&private_key, "bob", "kernel", replay_nonce, now + 3600);

        let first_use = handler
            .handle_login_by_jwt(replay_jwt.as_str(), &kernel_target)
            .await;
        assert!(first_use.is_ok(), "First use of login JWT should succeed");

        // Try to use the same JWT again
        let second_use = handler
            .handle_login_by_jwt(replay_jwt.as_str(), &kernel_target)
            .await;
        assert!(second_use.is_err(), "Replay of login JWT should fail");
        println!("Replay attack correctly prevented: {:?}", second_use.err());

        println!("\n=== All tests passed! ===");
        drop(server);
    }

    /// Test token pair generation
    #[tokio::test]
    async fn test_generate_token_pair() {
        println!("\n=== Test: Token pair generation ===");
        setup_test_environment().await;

        let target = app_target("test-app@test-owner");
        let (token_pair, session_token, refresh_token) =
            generate_token_pair(&target, "test-user", 12345, SessionPrincipalKind::User)
                .await
                .unwrap();

        // Verify token pair contains both tokens
        assert!(
            !token_pair.session_token.is_empty(),
            "Session token should not be empty"
        );
        assert!(
            !token_pair.refresh_token.is_empty(),
            "Refresh token should not be empty"
        );
        assert_eq!(
            token_app_instance_id(&session_token).unwrap(),
            "test-app@test-owner".parse().unwrap()
        );
        assert_eq!(
            token_app_instance_id(&refresh_token).unwrap(),
            "test-app@test-owner".parse().unwrap()
        );
        assert_eq!(
            session_token
                .extra
                .get(APP_OWNER_USER_ID_CLAIM)
                .and_then(Value::as_str),
            Some("test-owner")
        );
        let handler = VerifyHubServer::new();
        assert!(handler
            .handle_verify_token(&token_pair.session_token, Some(target.clone()),)
            .await
            .is_ok());
        assert!(handler
            .handle_verify_token(
                &token_pair.session_token,
                Some(app_target("test-app@other-owner")),
            )
            .await
            .is_err());

        // Verify tokens are different
        assert_ne!(
            token_pair.session_token, token_pair.refresh_token,
            "Session and refresh tokens should be different"
        );

        // Verify session token has correct expiration (short-lived)
        assert!(
            session_token.exp.unwrap()
                <= buckyos_get_unix_timestamp() + SESSION_TOKEN_EXPIRE_SECONDS + 1,
            "Session token expiration should be short"
        );

        // Verify refresh token has correct expiration (long-lived)
        assert!(
            refresh_token.exp.unwrap()
                <= buckyos_get_unix_timestamp() + REFRESH_TOKEN_EXPIRE_SECONDS + 1,
            "Refresh token expiration should be long"
        );
        assert!(
            refresh_token.exp.unwrap() > session_token.exp.unwrap(),
            "Refresh token should expire later than session token"
        );

        println!("Token pair generated correctly:");
        println!(
            "  Session token exp: {} (in {} seconds)",
            session_token.exp.unwrap(),
            session_token.exp.unwrap() - buckyos_get_unix_timestamp()
        );
        println!(
            "  Refresh token exp: {} (in {} seconds)",
            refresh_token.exp.unwrap(),
            refresh_token.exp.unwrap() - buckyos_get_unix_timestamp()
        );
    }

    #[tokio::test]
    async fn test_generate_sudo_session_token() {
        let target = system_target("control-panel");
        let session_token = generate_session_token(
            &target,
            "alice",
            5678,
            12345,
            SUDO_SESSION_TOKEN_EXPIRE_SECONDS,
            Some("system-config".to_string()),
            true,
            SessionPrincipalKind::User,
        )
        .await
        .unwrap();

        assert!(!session_token.to_string().is_empty());
        assert!(session_token.sudo);
        assert_eq!(session_token.appid.as_deref(), Some("control-panel"));
        assert_eq!(session_token.sub.as_deref(), Some("alice"));
        assert_eq!(session_token.aud.as_deref(), Some("system-config"));
        assert!(
            session_token.exp.unwrap()
                <= buckyos_get_unix_timestamp() + SUDO_SESSION_TOKEN_EXPIRE_SECONDS + 1,
            "Sudo session token expiration should be short"
        );

        let parsed = RPCSessionToken::from_string(session_token.to_string().as_str()).unwrap();
        assert!(parsed.sudo);
        assert_eq!(parsed.aud.as_deref(), Some("system-config"));
    }

    async fn signed_verify_hub_token(mut token: RPCSessionToken) -> String {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let jwt = token
            .generate_jwt(Some(VERIFY_HUB_ISSUER.to_string()), &private_key)
            .unwrap();
        token.token = Some(jwt);
        token.to_string()
    }

    fn unique_user(prefix: &str) -> String {
        format!("{}{}", prefix, rand::thread_rng().gen::<u32>())
    }

    /// A user's Control Panel session, issued through a root-signed LoginAssertion so no
    /// system-config lookup is needed.
    async fn control_panel_user_login(user: &str) -> TokenPair {
        let private_key = setup_test_environment().await;
        let assertion = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            jti: Some(rand::thread_rng().gen::<u64>().to_string()),
            appid: Some("control-panel".to_string()),
            aud: None,
            sub: Some(user.to_string()),
            token: None,
            iss: Some("root".to_string()),
            exp: Some(buckyos_get_unix_timestamp() + 600),
            sudo: false,
            extra: HashMap::new(),
        };
        let jwt = assertion
            .generate_jwt(Some("root".to_string()), &private_key)
            .unwrap();
        VerifyHubServer::new()
            .handle_login_by_jwt(jwt.as_str(), &system_target("control-panel"))
            .await
            .unwrap()
    }

    async fn service_session(service_id: &str, principal_kind: SessionPrincipalKind) -> String {
        generate_session_token(
            &system_target(service_id),
            "ood1",
            rand::thread_rng().gen(),
            rand::thread_rng().gen(),
            SESSION_TOKEN_EXPIRE_SECONDS,
            None,
            false,
            principal_kind,
        )
        .await
        .unwrap()
        .to_string()
    }

    fn session_key_of(token: &str) -> String {
        let token = RPCSessionToken::from_string(token).unwrap();
        let claims = validate_verify_hub_token_claims(&token, token_use(&token).unwrap()).unwrap();
        format!(
            "{}_{}_{}",
            token.sub.as_deref().unwrap(),
            claims.target.canonical_key(),
            get_token_session_id(&token).unwrap()
        )
    }

    fn test_family(refresh_jti: &str, previous: Option<(&str, u64)>) -> RefreshFamily {
        let token = |jti: &str| RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            jti: Some(jti.to_string()),
            appid: Some("test-app".to_string()),
            aud: None,
            sub: Some("test-user".to_string()),
            token: Some(format!("jwt-{jti}")),
            iss: Some(VERIFY_HUB_ISSUER.to_string()),
            exp: Some(buckyos_get_unix_timestamp() + 3600),
            sudo: false,
            extra: HashMap::new(),
        };
        RefreshFamily {
            refresh_token: token(refresh_jti),
            session_token: format!("session-{refresh_jti}"),
            previous_refresh: previous.map(|(jti, rotated_at)| RotatedRefresh {
                jti: jti.to_string(),
                rotated_at,
            }),
        }
    }

    #[test]
    fn refresh_presentation_accepts_current_and_recent_previous_only() {
        let now = 1_000_000;
        let family = test_family("r2", Some(("r1", now - REFRESH_GRACE_SECONDS)));
        assert!(matches!(
            classify_refresh_presentation(Some(&family), "r2", now),
            RefreshPresentation::Current
        ));
        match classify_refresh_presentation(Some(&family), "r1", now) {
            RefreshPresentation::Grace(pair) => {
                assert_eq!(pair.session_token, "session-r2");
                assert_eq!(pair.refresh_token, "jwt-r2");
            }
            other => panic!("expected grace, got {other:?}"),
        }
        assert!(matches!(
            classify_refresh_presentation(Some(&family), "r1", now + 1),
            RefreshPresentation::Reused
        ));
        assert!(matches!(
            classify_refresh_presentation(Some(&family), "r0", now),
            RefreshPresentation::Reused
        ));
        assert!(matches!(
            classify_refresh_presentation(None, "r2", now),
            RefreshPresentation::Missing
        ));
    }

    #[tokio::test]
    async fn refresh_rotation_commit_is_compare_and_replace() {
        let key = format!("cas_test_{}", rand::thread_rng().gen::<u64>());
        REFRESH_TOKEN_CACHE
            .lock()
            .await
            .insert(key.clone(), test_family("r1", None));
        let next = test_family("r2", None);
        let session = next.refresh_token.clone();

        assert!(matches!(
            commit_refresh_rotation(&key, "r1", &session, next.refresh_token.clone()).await,
            RefreshCommit::Committed
        ));
        let family = REFRESH_TOKEN_CACHE.lock().await.get(&key).cloned().unwrap();
        assert_eq!(family.refresh_token.jti.as_deref(), Some("r2"));
        assert_eq!(family.previous_refresh.as_ref().unwrap().jti, "r1");

        let losing = test_family("r2b", None);
        match commit_refresh_rotation(&key, "r1", &session, losing.refresh_token).await {
            RefreshCommit::Grace(pair) => assert_eq!(pair.refresh_token, "jwt-r2"),
            other => panic!("expected grace, got {other:?}"),
        }
        assert_eq!(
            REFRESH_TOKEN_CACHE
                .lock()
                .await
                .get(&key)
                .unwrap()
                .refresh_token
                .jti
                .as_deref(),
            Some("r2")
        );

        assert!(matches!(
            commit_refresh_rotation(&key, "r0", &session, test_family("r3", None).refresh_token)
                .await,
            RefreshCommit::Reused
        ));
        assert!(REFRESH_TOKEN_CACHE.lock().await.get(&key).is_none());

        assert!(matches!(
            commit_refresh_rotation(&key, "r2", &session, test_family("r4", None).refresh_token)
                .await,
            RefreshCommit::LoggedOut
        ));
        assert!(REFRESH_TOKEN_CACHE.lock().await.get(&key).is_none());
    }

    #[tokio::test]
    async fn concurrent_refresh_with_one_token_shares_one_pair() {
        // User principals read their settings on refresh; a System principal family needs no
        // system-config and exercises the same rotation path.
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();
        let service_pair = VerifyHubServer::new()
            .handle_login_by_jwt(login_jwt.as_str(), &system_target("control-panel"))
            .await
            .unwrap();
        let handler = VerifyHubServer::new();
        let (first, second) = tokio::join!(
            handler.handle_refresh_token(service_pair.refresh_token.as_str()),
            handler.handle_refresh_token(service_pair.refresh_token.as_str()),
        );
        let (first, second) = (first.unwrap(), second.unwrap());
        assert_eq!(first.refresh_token, second.refresh_token);
        assert_eq!(first.session_token, second.session_token);
        assert_ne!(first.refresh_token, service_pair.refresh_token);

        let late = handler
            .handle_refresh_token(service_pair.refresh_token.as_str())
            .await
            .unwrap();
        assert_eq!(late.refresh_token, first.refresh_token);
        let next = handler
            .handle_refresh_token(first.refresh_token.as_str())
            .await
            .unwrap();
        assert_ne!(next.refresh_token, first.refresh_token);
    }

    #[tokio::test]
    async fn stale_refresh_outside_grace_revokes_family() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();
        let handler = VerifyHubServer::new();
        let pair = handler
            .handle_login_by_jwt(login_jwt.as_str(), &system_target("control-panel"))
            .await
            .unwrap();
        let rotated = handler
            .handle_refresh_token(pair.refresh_token.as_str())
            .await
            .unwrap();
        let key = session_key_of(&pair.refresh_token);
        REFRESH_TOKEN_CACHE
            .lock()
            .await
            .get_mut(&key)
            .unwrap()
            .previous_refresh
            .as_mut()
            .unwrap()
            .rotated_at -= REFRESH_GRACE_SECONDS + 1;

        assert!(handler
            .handle_refresh_token(pair.refresh_token.as_str())
            .await
            .is_err());
        assert!(handler
            .handle_refresh_token(rotated.refresh_token.as_str())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn logout_cannot_be_undone_by_grace_or_inflight_refresh() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();
        let handler = VerifyHubServer::new();
        let pair = handler
            .handle_login_by_jwt(login_jwt.as_str(), &system_target("control-panel"))
            .await
            .unwrap();
        let rotated = handler
            .handle_refresh_token(pair.refresh_token.as_str())
            .await
            .unwrap();
        assert!(handler
            .handle_logout(rotated.refresh_token.as_str())
            .await
            .unwrap());
        assert!(handler
            .handle_refresh_token(pair.refresh_token.as_str())
            .await
            .is_err());
        let key = session_key_of(&rotated.refresh_token);
        let rotated_jti = RPCSessionToken::from_string(&rotated.refresh_token)
            .unwrap()
            .jti
            .unwrap();
        let session = RPCSessionToken::from_string(&rotated.session_token).unwrap();
        let refresh = RPCSessionToken::from_string(&rotated.refresh_token).unwrap();
        assert!(matches!(
            commit_refresh_rotation(&key, &rotated_jti, &session, refresh).await,
            RefreshCommit::LoggedOut
        ));
        assert!(REFRESH_TOKEN_CACHE.lock().await.get(&key).is_none());
    }

    #[tokio::test]
    async fn delegate_accepts_only_the_control_panel_service_as_caller() {
        setup_test_environment().await;
        let desktop = control_panel_user_login(&unique_user("caller")).await;
        let handler = VerifyHubServer::new();
        let notes = app_target("notes@bob");
        let private_key = setup_test_environment().await;
        let (device_assertion, _) =
            generate_service_login_assertion("ood1", "control-panel", "ood1", &private_key)
                .unwrap();
        let mut sudo_service = RPCSessionToken::from_string(
            &service_session("control-panel", SessionPrincipalKind::System).await,
        )
        .unwrap();
        sudo_service.sudo = true;
        let sudo_service = signed_verify_hub_token(sudo_service).await;
        let user_app_session = generate_session_token(
            &notes,
            "alice",
            1,
            2,
            SESSION_TOKEN_EXPIRE_SECONDS,
            None,
            false,
            SessionPrincipalKind::User,
        )
        .await
        .unwrap()
        .to_string();

        for caller in [
            None,
            Some(desktop.session_token.clone()),
            Some(desktop.refresh_token.clone()),
            Some(service_session("task-manager", SessionPrincipalKind::System).await),
            Some(service_session("control-panel", SessionPrincipalKind::App).await),
            Some(user_app_session),
            Some(device_assertion),
            Some(sudo_service),
            Some("not-a-jwt".to_string()),
        ] {
            let result = handler
                .handle_delegate_user_session(
                    caller.as_deref(),
                    desktop.session_token.as_str(),
                    &notes,
                )
                .await;
            assert!(
                matches!(result, Err(RPCErrors::NoPermission(_))),
                "caller {caller:?} must be refused, got {result:?}"
            );
        }

        for principal_kind in [SessionPrincipalKind::System, SessionPrincipalKind::Device] {
            let caller = service_session("control-panel", principal_kind).await;
            let result = handler
                .handle_delegate_user_session(Some(caller.as_str()), "not-a-jwt", &notes)
                .await;
            assert!(
                matches!(result, Ok(DelegateUserSessionResult::SourceInactive)),
                "{principal_kind:?} control-panel caller must pass, got {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn delegate_rejects_sources_that_are_not_live_control_panel_user_sessions() {
        setup_test_environment().await;
        let user = unique_user("source");
        let desktop = control_panel_user_login(&user).await;
        let caller = service_session("control-panel", SessionPrincipalKind::System).await;
        let handler = VerifyHubServer::new();
        let notes = app_target("notes@bob");
        let delegate = |source: String, target: AuthTarget| {
            let handler = handler.clone();
            let caller = caller.clone();
            async move {
                handler
                    .handle_delegate_user_session(Some(caller.as_str()), source.as_str(), &target)
                    .await
            }
        };

        assert!(matches!(
            delegate(
                desktop.session_token.clone(),
                system_target("control-panel")
            )
            .await,
            Err(RPCErrors::NoPermission(_))
        ));

        let mut expired = RPCSessionToken::from_string(&desktop.session_token).unwrap();
        expired.exp = Some(buckyos_get_unix_timestamp() - 600);
        let expired = signed_verify_hub_token(expired).await;
        let forged = {
            let mut parts = desktop.session_token.split('.');
            let (header, payload, signature) = (
                parts.next().unwrap(),
                parts.next().unwrap(),
                parts.next().unwrap(),
            );
            let mut claims: Value = serde_json::from_slice(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(payload)
                    .unwrap(),
            )
            .unwrap();
            claims["sub"] = Value::from("mallory");
            let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&claims).unwrap());
            format!("{header}.{payload}.{signature}")
        };
        for source in ["not-a-jwt".to_string(), expired, forged] {
            assert!(matches!(
                delegate(source, notes.clone()).await,
                Ok(DelegateUserSessionResult::SourceInactive)
            ));
        }

        let app_session = generate_session_token(
            &app_target("control-panel@alice"),
            &user,
            1,
            2,
            SESSION_TOKEN_EXPIRE_SECONDS,
            None,
            false,
            SessionPrincipalKind::User,
        )
        .await
        .unwrap()
        .to_string();
        let mut sudo = RPCSessionToken::from_string(&desktop.session_token).unwrap();
        sudo.sudo = true;
        let sudo = signed_verify_hub_token(sudo).await;
        for source in [
            desktop.refresh_token.clone(),
            app_session,
            sudo,
            service_session("control-panel", SessionPrincipalKind::System).await,
            service_session("control-panel", SessionPrincipalKind::Device).await,
        ] {
            let result = delegate(source, notes.clone()).await;
            assert!(
                matches!(result, Err(RPCErrors::InvalidToken(_))),
                "got {result:?}"
            );
        }

        assert!(handler
            .handle_logout(desktop.refresh_token.as_str())
            .await
            .unwrap());
        assert!(matches!(
            delegate(desktop.session_token.clone(), notes.clone()).await,
            Ok(DelegateUserSessionResult::SourceInactive)
        ));
    }

    #[tokio::test]
    async fn delegation_source_stays_live_across_refresh_and_ends_at_logout() {
        setup_test_environment().await;
        let desktop = control_panel_user_login(&unique_user("family")).await;
        let source = verify_delegation_source(&desktop.session_token)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(source.session_key, session_key_of(&desktop.refresh_token));
        assert!(source_family_is_live(&source.session_key).await);

        // A refresh keeps session_id and replaces the pair; the older access token still names
        // the same live family.
        let old_jti = RPCSessionToken::from_string(&desktop.refresh_token)
            .unwrap()
            .jti
            .unwrap();
        let (_, session, refresh) = generate_token_pair(
            &system_target("control-panel"),
            &source.user_id,
            get_token_session_id(&RPCSessionToken::from_string(&desktop.session_token).unwrap())
                .unwrap(),
            SessionPrincipalKind::User,
        )
        .await
        .unwrap();
        assert!(matches!(
            commit_refresh_rotation(&source.session_key, &old_jti, &session, refresh).await,
            RefreshCommit::Committed
        ));
        let again = verify_delegation_source(&desktop.session_token)
            .await
            .unwrap()
            .unwrap();
        assert!(source_family_is_live(&again.session_key).await);

        let (_, target_session, target_refresh) = generate_token_pair(
            &app_target("notes@bob"),
            &source.user_id,
            7,
            SessionPrincipalKind::User,
        )
        .await
        .unwrap();
        let target_key = format!("{}_app:notes@bob_7_test", source.user_id);
        assert!(
            commit_delegated_family(
                &source.session_key,
                &target_key,
                &target_session,
                target_refresh.clone(),
            )
            .await
        );
        assert!(REFRESH_TOKEN_CACHE.lock().await.contains_key(&target_key));

        revoke_session_tokens(&source.session_key).await;
        REFRESH_TOKEN_CACHE.lock().await.remove(&target_key);
        assert!(
            !commit_delegated_family(
                &source.session_key,
                &target_key,
                &target_session,
                target_refresh,
            )
            .await
        );
        assert!(!REFRESH_TOKEN_CACHE.lock().await.contains_key(&target_key));
    }
}
