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
}

#[derive(Clone)]
struct TrustedKey {
    key: DecodingKey,
    issuer_kind: TrustedIssuerKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionPrincipalKind {
    User,
    Device,
    Service,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppTokenScope {
    app_instance_id: String,
    owner_user_id: Option<String>,
}

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
    // Cache for valid refresh tokens, keyed by session_key
    // When a refresh token is used, the old one is invalidated and replaced with new one
    static ref REFRESH_TOKEN_CACHE: Arc<Mutex<HashMap<String, RPCSessionToken>>> =
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
    let value = match principal_kind {
        SessionPrincipalKind::User => TOKEN_PRINCIPAL_KIND_USER,
        SessionPrincipalKind::Device => TOKEN_PRINCIPAL_KIND_DEVICE,
        SessionPrincipalKind::Service => TOKEN_PRINCIPAL_KIND_SERVICE,
    };
    token.extra.insert(
        TOKEN_PRINCIPAL_KIND_CLAIM.to_string(),
        Value::String(value.to_string()),
    );
}

fn get_token_principal_kind(token: &RPCSessionToken) -> Result<SessionPrincipalKind> {
    match token
        .extra
        .get(TOKEN_PRINCIPAL_KIND_CLAIM)
        .and_then(Value::as_str)
    {
        Some(TOKEN_PRINCIPAL_KIND_USER) => Ok(SessionPrincipalKind::User),
        Some(TOKEN_PRINCIPAL_KIND_DEVICE) => Ok(SessionPrincipalKind::Device),
        Some(TOKEN_PRINCIPAL_KIND_SERVICE) => Ok(SessionPrincipalKind::Service),
        _ => Err(RPCErrors::InvalidToken(
            "Missing or invalid principal_kind".to_string(),
        )),
    }
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
    appid: &str,
    userid: &str,
    jti: u64,
    session: u64,
    duration: u64,
    aud: Option<String>,
    sudo: bool,
    principal_kind: SessionPrincipalKind,
    app_scope: Option<&AppTokenScope>,
) -> Result<RPCSessionToken> {
    reject_root_session_subject(userid)?;
    let now = buckyos_get_unix_timestamp();
    let exp = now + duration;

    let mut session_token = RPCSessionToken {
        token_type: RPCSessionTokenType::Normal,
        appid: Some(appid.to_string()),
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
    if let Some(app_scope) = app_scope {
        bind_token_app_instance(
            &mut session_token,
            &app_scope.app_instance_id,
            app_scope.owner_user_id.as_deref(),
        );
    }

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
    appid: &str,
    userid: &str,
    jti: u64,
    session: u64,
    duration: u64,
    principal_kind: SessionPrincipalKind,
    app_scope: Option<&AppTokenScope>,
) -> Result<RPCSessionToken> {
    reject_root_session_subject(userid)?;
    let now = buckyos_get_unix_timestamp();
    let exp = now + duration;

    let mut refresh_token = RPCSessionToken {
        token_type: RPCSessionTokenType::Normal,
        appid: Some(appid.to_string()),
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
    if let Some(app_scope) = app_scope {
        bind_token_app_instance(
            &mut refresh_token,
            &app_scope.app_instance_id,
            app_scope.owner_user_id.as_deref(),
        );
    }

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
    appid: &str,
    userid: &str,
    session_id: u64,
    principal_kind: SessionPrincipalKind,
    app_scope: Option<&AppTokenScope>,
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
        appid,
        userid,
        session_jti,
        session_id,
        SESSION_TOKEN_EXPIRE_SECONDS,
        None,
        false,
        principal_kind,
        app_scope,
    )
    .await?;

    // Generate long-lived refresh token
    let refresh_token = generate_refresh_token(
        appid,
        userid,
        refresh_jti,
        session_id,
        REFRESH_TOKEN_EXPIRE_SECONDS,
        principal_kind,
        app_scope,
    )
    .await?;

    let token_pair = TokenPair {
        session_token: session_token.to_string(),
        refresh_token: refresh_token.to_string(),
    };

    Ok((token_pair, session_token, refresh_token))
}

/// Cache refresh token for validation during refresh flow
async fn cache_refresh_token(key: &str, token: RPCSessionToken) {
    REFRESH_TOKEN_CACHE
        .lock()
        .await
        .insert(key.to_string(), token);
}

async fn gc_token_caches() {
    let now = buckyos_get_unix_timestamp();

    {
        let mut cache = TOKEN_CACHE.lock().await;
        cache.retain(|_, token| token.exp.map(|exp| exp > now).unwrap_or(false));
    }

    {
        let mut cache = REFRESH_TOKEN_CACHE.lock().await;
        cache.retain(|_, token| token.exp.map(|exp| exp > now).unwrap_or(false));
    }
}

async fn revoke_session_tokens(session_key: &str) {
    TOKEN_CACHE.lock().await.remove(session_key);
    REFRESH_TOKEN_CACHE.lock().await.remove(session_key);
}

/// Load refresh token from cache for validation
async fn load_refresh_token_from_cache(key: &str) -> Option<RPCSessionToken> {
    let cache = REFRESH_TOKEN_CACHE.lock().await;
    cache.get(key).cloned()
}

/// Invalidate (remove) a refresh token from cache
/// Called when a refresh token is used to ensure it cannot be reused
async fn invalidate_refresh_token(key: &str) {
    REFRESH_TOKEN_CACHE.lock().await.remove(key);
}

async fn validate_active_refresh_token(refresh_jwt: &str) -> Result<(RPCSessionToken, String)> {
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
    let appid = rpc_session_token
        .appid
        .clone()
        .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
    let cache_scope = rpc_session_token
        .extra
        .get(APP_INSTANCE_ID_CLAIM)
        .and_then(Value::as_str)
        .unwrap_or(appid.as_str());
    let session_id = get_token_session_id(&rpc_session_token)?;
    let session_key = format!("{}_{}_{}", userid, cache_scope, session_id);
    let refresh_jti = rpc_session_token
        .jti
        .clone()
        .ok_or(RPCErrors::ReasonError("Missing jti".to_string()))?;

    let cached_refresh = load_refresh_token_from_cache(session_key.as_str()).await;
    if cached_refresh.is_none() {
        warn!(
            "Refresh token not found in cache for session: {}",
            session_key
        );
        warn!(
            "Refresh reuse detected (cache-miss), revoking session: {}",
            session_key
        );
        revoke_session_tokens(session_key.as_str()).await;
        return Err(RPCErrors::ReasonError(
            "Refresh token not found or already invalidated".to_string(),
        ));
    }

    let cached_refresh = cached_refresh.unwrap();
    if cached_refresh.jti.as_deref() != Some(refresh_jti.as_str()) {
        warn!(
            "Invalid refresh token jti. Expected: {:?}, Got: {:?}",
            cached_refresh.jti, rpc_session_token.jti
        );
        warn!(
            "Refresh reuse detected (jti-mismatch), revoking session: {}",
            session_key
        );
        revoke_session_tokens(session_key.as_str()).await;
        return Err(RPCErrors::ReasonError(
            "Invalid refresh token jti".to_string(),
        ));
    }

    Ok((rpc_session_token, session_key))
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
        SessionPrincipalKind::Service => Ok(()),
    }
}

async fn resolve_user_app_scope(
    user_id: &str,
    appid: &str,
    app_instance_id: &str,
) -> Result<AppTokenScope> {
    let (instance_app_id, owner_user_id) = parse_app_instance_id(app_instance_id)?;
    if instance_app_id != appid {
        return Err(RPCErrors::NoPermission("AppAccessDenied".to_string()));
    }

    let resolver = AppAvailabilityResolver::new(
        Arc::new(get_system_config_client().await?),
        env!("CARGO_PKG_VERSION"),
        get_buckyos_api_runtime()?.zone_id.clone(),
    );
    if owner_user_id == SYSTEM_APP_OWNER_ID && is_system_login_target(appid) {
        if find_system_builtin_app(appid).is_none() {
            resolver.get_user_settings(user_id).await.map_err(|error| {
                warn!(
                    "system app availability check failed user={} app_instance_id={}: {}",
                    user_id, app_instance_id, error
                );
                RPCErrors::NoPermission("AppAccessDenied".to_string())
            })?;
            return Ok(AppTokenScope {
                app_instance_id: app_instance_id.to_string(),
                owner_user_id: None,
            });
        }
    }

    let decision = resolver
        .check_user(user_id, app_instance_id)
        .await
        .map_err(|error| {
            warn!(
                "app availability check failed user={} app_instance_id={}: {}",
                user_id, app_instance_id, error
            );
            RPCErrors::NoPermission("AppAccessDenied".to_string())
        })?;
    if !decision.allowed {
        warn!(
            "app availability denied user={} app_instance_id={} reason={}",
            user_id, app_instance_id, decision.reason
        );
        return Err(RPCErrors::NoPermission("AppAccessDenied".to_string()));
    }
    Ok(AppTokenScope {
        app_instance_id: decision.app_instance_id,
        owner_user_id: if decision.app_class == AppClass::SystemBuiltin {
            None
        } else {
            Some(decision.owner_user_id)
        },
    })
}

fn login_param_app_instance_id(login_params: Option<&Value>) -> Option<String> {
    login_params
        .and_then(Value::as_object)
        .and_then(|params| params.get(APP_INSTANCE_ID_CLAIM))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
        appid: Some("verify-hub".to_string()),
        jti: None,
        aud: None,
        sub: Some(device_id),
        token: None,
        iss: Some(VERIFY_HUB_ISSUER.to_string()),
        exp: Some(exp),
        sudo: false,
        extra: HashMap::new(),
    };

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

    let instance_info = ServiceInstanceReportInfo {
        instance_id: format!("{}-{}", VERIFY_HUB_UNIQUE_ID, service_config.device_id),
        node_id: service_config.device_id.clone(),
        node_did: service_config.node_did.clone(),
        state: ServiceInstanceState::Started,
        service_ports,
        last_update_time: buckyos_get_unix_timestamp(),
        start_time: service_config.start_time,
        pid: std::process::id(),
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
    if iss == "root" {
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
                let device_config = control_panel_client.get_device_config(iss).await;
                if device_config.is_err() {
                    warn!(
                        "load user/device {} config from system config service failed",
                        iss
                    );
                    return Err(RPCErrors::ReasonError(
                        "User or device config not found".to_string(),
                    ));
                }
                let device_config = device_config.unwrap();
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
    appid: &str,
    app_instance_id: &str,
    login_nonce: u64,
) -> Result<(UserSettings, String)> {
    let now = buckyos_get_unix_timestamp() * 1000;
    let abs_diff = now.abs_diff(login_nonce);
    debug!(
        "{} login nonce and now abs_diff:{}, from:{}",
        username, abs_diff, appid
    );
    if abs_diff > MAX_LOGIN_NONCE_AGE_SECONDS {
        warn!(
            "{} login nonce is too old, abs_diff:{}, this is a possible ATTACK?",
            username, abs_diff
        );
        return Err(RPCErrors::ParseRequestError("Invalid nonce".to_string()));
    }

    let session_key = format!("{}_{}_{}", username, app_instance_id, login_nonce);
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
    async fn handle_login_by_jwt(
        &self,
        jwt: &str,
        login_params: Option<Value>,
    ) -> Result<TokenPair> {
        gc_token_caches().await;

        // Step 1: Verify JWT signature (include exp) and extract payload
        // The incoming JWT is signed by a trusted entity (device/owner)
        let (jwt_payload, issuer_kind) = verify_trusted_jwt(jwt).await?;

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
            .ok_or(RPCErrors::ReasonError("Missing sub".to_string()))?;
        reject_root_session_subject(userid.as_str())?;
        let appid = rpc_session_token
            .appid
            .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
        let token_jti = rpc_session_token
            .jti
            .ok_or(RPCErrors::ReasonError("Missing jti".to_string()))?;
        let principal_kind = if issuer_kind == TrustedIssuerKind::Device {
            if rpc_session_token.iss.as_deref() == Some(userid.as_str()) {
                SessionPrincipalKind::Device
            } else {
                SessionPrincipalKind::Service
            }
        } else {
            SessionPrincipalKind::User
        };
        let app_scope = if principal_kind == SessionPrincipalKind::User {
            let app_instance_id = login_param_app_instance_id(login_params.as_ref())
                .or_else(|| {
                    rpc_session_token
                        .extra
                        .get(APP_INSTANCE_ID_CLAIM)
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| format!("{}@{}", appid, SYSTEM_APP_OWNER_ID));
            Some(resolve_user_app_scope(&userid, &appid, &app_instance_id).await?)
        } else {
            None
        };
        //let token_jti = rpc_session_token.jti.ok_or(RPCErrors::ReasonError("Missing jti".to_string()))?;

        // ============================================================
        // FIRST LOGIN FLOW: Using trusted device/owner JWT
        // The incoming JWT is signed by a trusted entity (device/owner)
        // ============================================================
        info!("Handle login by JWT for sub: {}, appid: {}", userid, appid);

        let cache_scope = app_scope
            .as_ref()
            .map(|scope| scope.app_instance_id.as_str())
            .unwrap_or(appid.as_str());
        let session_key = format!("{}_{}_{}", userid, cache_scope, token_jti);

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
        let new_session_key = format!("{}_{}_{}", userid, cache_scope, session_id);

        // Step 6: Generate new token pair (session_token + refresh_token)
        let (token_pair, session_token, refresh_token) = generate_token_pair(
            appid.as_str(),
            userid.as_str(),
            session_id,
            principal_kind,
            app_scope.as_ref(),
        )
        .await?;

        // Step 7: Cache both tokens
        // Cache by original session_key to mark login JWT as used
        cache_token(session_key.as_str(), session_token.clone()).await;
        // Cache by new session_key for future refresh operations
        cache_token(new_session_key.as_str(), session_token).await;
        cache_refresh_token(new_session_key.as_str(), refresh_token).await;

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
        let (rpc_session_token, session_key) = validate_active_refresh_token(refresh_jwt).await?;
        let userid = rpc_session_token
            .sub
            .clone()
            .ok_or(RPCErrors::ReasonError("Missing sub".to_string()))?;
        let appid = rpc_session_token
            .appid
            .clone()
            .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
        let session_id = get_token_session_id(&rpc_session_token)?;
        let principal_kind = get_token_principal_kind(&rpc_session_token)?;
        validate_refresh_principal(userid.as_str(), principal_kind).await?;
        let app_scope = if principal_kind == SessionPrincipalKind::User {
            let app_instance_id = token_app_instance_id(&rpc_session_token)?;
            Some(resolve_user_app_scope(&userid, &appid, app_instance_id).await?)
        } else {
            None
        };

        info!("Handle refresh token request for session: {}", session_key);

        // Step 7: IMPORTANT - Invalidate the old refresh token immediately
        // This ensures the old refresh token cannot be reused (one-time use)
        invalidate_refresh_token(session_key.as_str()).await;
        info!("Old refresh token invalidated for session: {}", session_key);

        // Step 8: Generate new token pair (session_token + refresh_token)
        let (token_pair, session_token, refresh_token) = generate_token_pair(
            appid.as_str(),
            userid.as_str(),
            session_id,
            principal_kind,
            app_scope.as_ref(),
        )
        .await?;

        // Step 9: Cache the new tokens
        cache_token(session_key.as_str(), session_token).await;
        cache_refresh_token(session_key.as_str(), refresh_token).await;

        info!(
            "Refresh successful for session: {}. New token pair generated.",
            session_key
        );
        Ok(buckyos_api::TokenPair {
            session_token: token_pair.session_token,
            refresh_token: token_pair.refresh_token,
        })
    }

    async fn handle_logout(&self, refresh_jwt: &str) -> Result<bool> {
        gc_token_caches().await;
        let (_rpc_session_token, session_key) = validate_active_refresh_token(refresh_jwt).await?;

        info!("Handle logout request for session: {}", session_key);
        revoke_session_tokens(session_key.as_str()).await;
        info!("Logout successful for session: {}", session_key);

        Ok(true)
    }

    async fn handle_login_by_password(
        &self,
        username: &str,
        password: &str,
        appid: &str,
        app_instance_id: &str,
        login_nonce: u64,
    ) -> Result<LoginByPasswordResponse> {
        gc_token_caches().await;
        reject_root_session_subject(username)?;

        let session_id = login_nonce;
        let (user_settings, session_key) =
            validate_password_login(username, password, appid, app_instance_id, login_nonce)
                .await?;
        reject_root_user_settings(&user_settings)?;
        require_active_user_settings(&user_settings)?;
        let app_scope = resolve_user_app_scope(username, appid, app_instance_id).await?;

        info!(
            "Password login successful for user: {}. Generating token pair.",
            username
        );

        // Step 5: Generate token pair (session_token + refresh_token)
        // session_token: short-lived (15 minutes) for API requests
        // refresh_token: long-lived (7 days) for obtaining new token pairs
        let (token_pair, session_token, refresh_token) = generate_token_pair(
            appid,
            username,
            session_id,
            SessionPrincipalKind::User,
            Some(&app_scope),
        )
        .await?;

        // Step 6: Cache both tokens
        cache_token(session_key.as_str(), session_token).await;
        cache_refresh_token(session_key.as_str(), refresh_token).await;

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
        appid: &str,
        app_instance_id: &str,
        aud: Option<String>,
        login_nonce: u64,
    ) -> Result<SudoByPasswordResponse> {
        gc_token_caches().await;
        reject_root_session_subject(username)?;

        let session_id = login_nonce;
        let (user_settings, session_key) =
            validate_password_login(username, password, appid, app_instance_id, login_nonce)
                .await?;
        reject_root_user_settings(&user_settings)?;
        require_active_user_settings(&user_settings)?;
        let app_scope = resolve_user_app_scope(username, appid, app_instance_id).await?;

        let session_jti: u64;
        {
            let mut rng = rand::thread_rng();
            session_jti = rng.gen::<u64>();
        }

        let session_token = generate_session_token(
            appid,
            username,
            session_jti,
            session_id,
            SUDO_SESSION_TOKEN_EXPIRE_SECONDS,
            aud,
            true,
            SessionPrincipalKind::User,
            Some(&app_scope),
        )
        .await?;

        cache_token(session_key.as_str(), session_token.clone()).await;
        info!("Sudo session token cached for session: {}", session_key);

        Ok(SudoByPasswordResponse {
            session_token: session_token.to_string(),
        })
    }

    async fn handle_verify_token(
        &self,
        session_token: &str,
        appid: Option<String>,
        app_instance_id: Option<String>,
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

            if rpc_session_token.aud.as_deref() == Some(VERIFY_HUB_UNIQUE_ID) {
                return Err(RPCErrors::InvalidToken(
                    "refresh token cannot be used as session token".to_string(),
                ));
            }
            if let Some(userid) = rpc_session_token.sub.as_deref() {
                reject_root_session_subject(userid)?;
            }

            let principal_kind = get_token_principal_kind(&rpc_session_token)?;
            if principal_kind == SessionPrincipalKind::User {
                let token_instance_id = token_app_instance_id(&rpc_session_token)?;
                let (instance_app_id, owner_user_id) = parse_app_instance_id(token_instance_id)?;
                if rpc_session_token.appid.as_deref() != Some(instance_app_id.as_str()) {
                    return Err(RPCErrors::InvalidToken(
                        "appid and app_instance_id claims do not match".to_string(),
                    ));
                }
                let owner_claim = rpc_session_token
                    .extra
                    .get(APP_OWNER_USER_ID_CLAIM)
                    .and_then(Value::as_str);
                if owner_user_id == SYSTEM_APP_OWNER_ID {
                    if owner_claim.is_some() && owner_claim != Some(SYSTEM_APP_OWNER_ID) {
                        return Err(RPCErrors::InvalidToken(
                            "app owner claim does not match app_instance_id".to_string(),
                        ));
                    }
                } else if owner_claim != Some(owner_user_id.as_str()) {
                    return Err(RPCErrors::InvalidToken(
                        "app owner claim does not match app_instance_id".to_string(),
                    ));
                }
            }

            if let Some(expected_appid) = appid {
                let token_appid = rpc_session_token
                    .appid
                    .as_deref()
                    .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
                if token_appid != expected_appid {
                    return Err(RPCErrors::InvalidToken("appid mismatch".to_string()));
                }
            }
            if let Some(expected_app_instance_id) = app_instance_id {
                let token_app_instance_id = token_app_instance_id(&rpc_session_token)?;
                if token_app_instance_id != expected_app_instance_id {
                    return Err(RPCErrors::InvalidToken(
                        "app_instance_id mismatch".to_string(),
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
            generate_service_login_jwt("alice", "control-panel", "ood1", &private_key).unwrap();

        let token_pair = VerifyHubServer::new()
            .handle_login_by_jwt(login_jwt.as_str(), None)
            .await
            .unwrap();
        let session_token = RPCSessionToken::from_string(&token_pair.session_token).unwrap();
        let refresh_token = RPCSessionToken::from_string(&token_pair.refresh_token).unwrap();

        assert_eq!(session_token.sub.as_deref(), Some("alice"));
        assert_eq!(session_token.appid.as_deref(), Some("control-panel"));
        assert_eq!(refresh_token.sub.as_deref(), Some("alice"));
        assert_eq!(refresh_token.appid.as_deref(), Some("control-panel"));
        assert_eq!(
            get_token_principal_kind(&refresh_token).unwrap(),
            SessionPrincipalKind::Service
        );
    }

    #[tokio::test]
    async fn device_service_login_preserves_device_subject() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_jwt("ood1", "node-daemon", "ood1", &private_key).unwrap();

        let token_pair = VerifyHubServer::new()
            .handle_login_by_jwt(login_jwt.as_str(), None)
            .await
            .unwrap();
        let session_token = RPCSessionToken::from_string(&token_pair.session_token).unwrap();
        let refresh_token = RPCSessionToken::from_string(&token_pair.refresh_token).unwrap();

        assert_eq!(session_token.sub.as_deref(), Some("ood1"));
        assert_eq!(session_token.appid.as_deref(), Some("node-daemon"));
        assert_eq!(
            get_token_principal_kind(&session_token).unwrap(),
            SessionPrincipalKind::Device
        );
        assert_eq!(
            get_token_principal_kind(&refresh_token).unwrap(),
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
            .login_by_jwt(test_jwt.as_str(), None)
            .await
            .expect("login_by_jwt should succeed");
        assert!(!token_pair.session_token.is_empty());
        assert!(!token_pair.refresh_token.is_empty());

        let verify_ok = verify_hub_client
            .verify_token(&token_pair.session_token, Some("kernel"), None)
            .await;
        assert!(
            verify_ok.is_ok(),
            "verify_token should succeed for correct appid"
        );

        let verify_bad = verify_hub_client
            .verify_token(&token_pair.session_token, Some("not-kernel"), None)
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
        let login_result = handler.handle_login_by_jwt(test_jwt.as_str(), None).await;

        assert!(
            matches!(login_result, Err(RPCErrors::NoPermission(_))),
            "verify-hub must reject root login JWT"
        );
    }

    #[tokio::test]
    async fn test_verify_hub_rejects_root_password_entrypoints() {
        let handler = VerifyHubServer::new();

        let login_result = handler
            .handle_login_by_password(
                "root",
                "not-used",
                "control-panel",
                "control-panel@system",
                9002,
            )
            .await;
        assert!(
            matches!(login_result, Err(RPCErrors::NoPermission(_))),
            "verify-hub must reject root password login"
        );

        let sudo_result = handler
            .handle_sudo_by_password(
                "root",
                "not-used",
                "control-panel",
                "control-panel@system",
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
        let session_result = generate_session_token(
            "control-panel",
            "root",
            5678,
            12345,
            SESSION_TOKEN_EXPIRE_SECONDS,
            None,
            false,
            SessionPrincipalKind::User,
            None,
        )
        .await;
        assert!(
            matches!(session_result, Err(RPCErrors::NoPermission(_))),
            "root session token generation must fail"
        );

        let refresh_result = generate_refresh_token(
            "control-panel",
            "root",
            5679,
            12345,
            REFRESH_TOKEN_EXPIRE_SECONDS,
            SessionPrincipalKind::User,
            None,
        )
        .await;
        assert!(
            matches!(refresh_result, Err(RPCErrors::NoPermission(_))),
            "root refresh token generation must fail"
        );

        let token_pair_result = generate_token_pair(
            "control-panel",
            "root",
            12345,
            SessionPrincipalKind::User,
            None,
        )
        .await;
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
            .handle_verify_token(token.to_string().as_str(), None, None)
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

        let login_result = handler.handle_login_by_jwt(test_jwt.as_str(), None).await;

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
            .handle_verify_token(token_pair.session_token.as_str(), None, None)
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
            .handle_verify_token(new_token_pair.session_token.as_str(), None, None)
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
            .handle_login_by_jwt(expired_jwt.as_str(), None)
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

        let first_use = handler.handle_login_by_jwt(replay_jwt.as_str(), None).await;
        assert!(first_use.is_ok(), "First use of login JWT should succeed");

        // Try to use the same JWT again
        let second_use = handler.handle_login_by_jwt(replay_jwt.as_str(), None).await;
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

        let app_scope = AppTokenScope {
            app_instance_id: "test-app@test-owner".to_string(),
            owner_user_id: Some("test-owner".to_string()),
        };
        let (token_pair, session_token, refresh_token) = generate_token_pair(
            "test-app",
            "test-user",
            12345,
            SessionPrincipalKind::User,
            Some(&app_scope),
        )
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
            "test-app@test-owner"
        );
        assert_eq!(
            token_app_instance_id(&refresh_token).unwrap(),
            "test-app@test-owner"
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
            .handle_verify_token(
                &token_pair.session_token,
                Some("test-app".to_string()),
                Some("test-app@test-owner".to_string()),
            )
            .await
            .is_ok());
        assert!(handler
            .handle_verify_token(
                &token_pair.session_token,
                Some("test-app".to_string()),
                Some("test-app@other-owner".to_string()),
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
        let session_token = generate_session_token(
            "control-panel",
            "alice",
            5678,
            12345,
            SUDO_SESSION_TOKEN_EXPIRE_SECONDS,
            Some("system-config".to_string()),
            true,
            SessionPrincipalKind::User,
            Some(&AppTokenScope {
                app_instance_id: "control-panel@system".to_string(),
                owner_user_id: None,
            }),
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

    /// Test refresh token cache operations
    #[tokio::test]
    async fn test_refresh_token_cache() {
        println!("\n=== Test: Refresh token cache operations ===");

        let mut test_token = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            jti: Some("123456".to_string()),
            appid: Some("test-app".to_string()),
            aud: None,
            sub: Some("test-user".to_string()),
            token: Some("test-token-value".to_string()),
            iss: Some("verify-hub".to_string()),
            exp: Some(buckyos_get_unix_timestamp() + 3600),
            sudo: false,
            extra: HashMap::new(),
        };
        set_token_session_id(&mut test_token, 789);

        let cache_key = "test_cache_key";

        // Test cache is initially empty
        let initial = load_refresh_token_from_cache(cache_key).await;
        assert!(initial.is_none(), "Cache should be initially empty");

        // Test caching token
        cache_refresh_token(cache_key, test_token.clone()).await;
        let cached = load_refresh_token_from_cache(cache_key).await;
        assert!(cached.is_some(), "Token should be cached");
        assert_eq!(
            cached.unwrap().jti,
            test_token.jti,
            "Cached token should match"
        );

        // Test invalidating token
        invalidate_refresh_token(cache_key).await;
        let after_invalidate = load_refresh_token_from_cache(cache_key).await;
        assert!(after_invalidate.is_none(), "Token should be invalidated");

        println!("Refresh token cache operations work correctly!");
    }
}
