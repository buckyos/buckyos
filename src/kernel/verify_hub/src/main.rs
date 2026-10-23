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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use ::kRPC::*;
use buckyos_api::*;
use buckyos_kit::*;
use jsonwebtoken::{errors::ErrorKind, Algorithm, DecodingKey, EncodingKey, Validation};
use name_lib::*;

mod directory;
mod family_store;
mod federation;

use directory::{SystemConfigDirectory, ZoneDirectory};
use family_store::{
    classify_refresh_presentation, family_key, FamilyClaims, FamilyRecord, FamilyStore, PairStamp,
    RefreshCommit, RefreshPresentation, SessionSubject,
};
use federation::{
    FederationCode, FederationCodes, FederationPeer, NameFederationPeer,
    FEDERATION_CODE_TTL_SECONDS, FEDERATION_SESSION_MAX_SECONDS,
};

// Token expiration time constants
// Session token: short-lived, used for API requests
const SESSION_TOKEN_EXPIRE_SECONDS: u64 = 15 * 60; // 15 minutes
const SUDO_SESSION_TOKEN_EXPIRE_SECONDS: u64 = 3 * 60;
// Refresh token: long-lived, used to obtain new token pairs
const REFRESH_TOKEN_EXPIRE_SECONDS: u64 = 7 * 24 * 3600; // 7 days
const MAX_LOGIN_NONCE_AGE_SECONDS: u64 = 3600 * 8; // 8 hours
const FAMILY_GC_INTERVAL_SECONDS: u64 = 60;
/// Resolving the canonical zone and exchanging the code; a slower peer reads
/// as `canonical_unavailable`, well before Control Panel's own call times out.
const FEDERATION_PEER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
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

struct PresentedRefresh {
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
    // Login nonces and LoginAssertions already used, with the time the mark may be dropped.
    static ref TOKEN_CACHE: Arc<Mutex<HashMap<String, u64>>> =
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

fn account_state_denied() -> RPCErrors {
    AccessDenial::new(DenyReason::AccountState).into_rpc_error()
}

fn require_active_user_settings(user_settings: &UserSettings) -> Result<()> {
    if user_is_active(user_settings) {
        Ok(())
    } else if !user_settings.user_type.can_login() {
        Err(RPCErrors::NoPermission(format!(
            "user '{}' cannot log in",
            user_settings.user_id
        )))
    } else {
        Err(account_state_denied())
    }
}

fn refresh_credential_denied(detail: &str) -> RPCErrors {
    AccessDenial::new(DenyReason::CredentialState)
        .at(VERIFY_HUB_UNIQUE_ID, "refresh_token")
        .with_detail(detail)
        .into_rpc_error()
}

fn login_assertion_denied(detail: &str) -> RPCErrors {
    AccessDenial::new(DenyReason::CredentialState)
        .at(VERIFY_HUB_UNIQUE_ID, "login_by_jwt")
        .with_detail(detail)
        .into_rpc_error()
}

fn family_invalidated() -> RPCErrors {
    refresh_credential_denied("refresh_session_invalidated")
}

/// Sign one token of a family's pair, or a sudo token, for `session`.
async fn sign_session_token(
    session: &SessionSubject,
    token_use: TokenUse,
    jti: &str,
    exp: u64,
    aud: Option<String>,
    sudo: bool,
) -> Result<RPCSessionToken> {
    reject_root_session_subject(&session.subject)?;
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::Normal,
        appid: None,
        jti: Some(jti.to_string()),
        aud,
        sub: Some(session.subject.clone()),
        token: None,
        iss: Some(VERIFY_HUB_ISSUER.to_string()),
        exp: Some(exp),
        sudo,
        extra: HashMap::new(),
    };
    set_token_session_id(&mut token, session.session_id);
    set_token_principal_kind(&mut token, session.principal_kind);
    session.claims.bind(&mut token);
    bind_token_target(&mut token, &session.target, token_use)?;

    {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let jwt = token.generate_jwt(Some(VERIFY_HUB_ISSUER.to_string()), &private_key)?;
        token.token = Some(jwt);
    }

    Ok(token)
}

/// New identifiers for a pair: session token 15 minutes, refresh token 7 days, neither past
/// the family's absolute expiry.
fn fresh_stamp(now: u64, absolute_exp: Option<u64>) -> PairStamp {
    let cap = |exp: u64| absolute_exp.map_or(exp, |limit| exp.min(limit));
    let mut rng = rand::thread_rng();
    PairStamp {
        session_jti: rng.gen::<u64>().to_string(),
        session_exp: cap(now + SESSION_TOKEN_EXPIRE_SECONDS),
        refresh_jti: rng.gen::<u64>().to_string(),
        refresh_exp: cap(now + REFRESH_TOKEN_EXPIRE_SECONDS),
    }
}

/// The pair a family record describes. Re-signing the same record yields the same pair, which
/// is how a grace answer is rebuilt without storing tokens.
async fn sign_pair(session: &SessionSubject, stamp: &PairStamp) -> Result<TokenPair> {
    let session_token = sign_session_token(
        session,
        TokenUse::Session,
        &stamp.session_jti,
        stamp.session_exp,
        None,
        false,
    )
    .await?;
    let refresh_token = sign_session_token(
        session,
        TokenUse::Refresh,
        &stamp.refresh_jti,
        stamp.refresh_exp,
        Some(VERIFY_HUB_UNIQUE_ID.to_string()), //refresh token audience is verify-hub
        false,
    )
    .await?;
    Ok(TokenPair {
        session_token: session_token.to_string(),
        refresh_token: refresh_token.to_string(),
    })
}

async fn verify_presented_refresh(refresh_jwt: &str) -> Result<PresentedRefresh> {
    let jwt_payload = verify_verify_hub_jwt(refresh_jwt, None).await?;

    let rpc_session_token: RPCSessionToken =
        serde_json::from_value(jwt_payload).map_err(|error| {
            error!(
                "Failed to parse RPCSessionToken from JWT payload: {}",
                error
            );
            RPCErrors::InvalidToken("Failed to parse RPCSessionToken from JWT payload".to_string())
        })?;

    let userid = rpc_session_token
        .sub
        .clone()
        .ok_or(RPCErrors::InvalidToken("Missing sub".to_string()))?;
    reject_root_session_subject(userid.as_str())?;
    let claims = validate_verify_hub_token_claims(&rpc_session_token, TokenUse::Refresh)?;
    let session_id = get_token_session_id(&rpc_session_token)
        .map_err(|error| RPCErrors::InvalidToken(error.to_string()))?;
    let session_key = family_key(&userid, &claims.target, session_id);
    let jti = rpc_session_token
        .jti
        .clone()
        .ok_or(RPCErrors::InvalidToken("Missing jti".to_string()))?;

    Ok(PresentedRefresh { session_key, jti })
}

/// Family key of a verified session token of this hub.
/// The family a session token belongs to; an expired token still names it
/// (`revoke_families` spares the caller's family even if its access just expired).
async fn session_family_key(session_token: &str) -> Result<String> {
    let payload = decode_verify_hub_jwt(session_token, None, false).await?;
    let token: RPCSessionToken = serde_json::from_value(payload)
        .map_err(|_| RPCErrors::InvalidToken("not a session token".to_string()))?;
    let claims = validate_verify_hub_token_claims(&token, TokenUse::Session)?;
    let subject = token
        .sub
        .as_deref()
        .ok_or_else(|| RPCErrors::InvalidToken("session token missing sub".to_string()))?;
    Ok(family_key(
        subject,
        &claims.target,
        get_token_session_id(&token)?,
    ))
}

fn control_panel_target() -> Result<AuthTarget> {
    Ok(AuthTarget::system(
        SystemServiceId::parse(CONTROL_PANEL_SERVICE_UNIQUE_ID).map_err(RPCErrors::ReasonError)?,
    ))
}

/// Sessions issued on Control Panel's behalf (exclusive invite, DID login) open on its target.
fn require_control_panel_target(target: &AuthTarget, method: &str) -> Result<AuthTarget> {
    let control_panel = control_panel_target()?;
    if *target != control_panel {
        return Err(RPCErrors::ParseRequestError(format!(
            "{method} only issues the control-panel target"
        )));
    }
    Ok(control_panel)
}

/// Only the Control Panel service may call `method`: /kapi/verify-hub is reachable from
/// browsers, and these methods return token pairs or revoke sessions on CP's decision.
async fn authorize_control_panel_caller(caller_token: Option<&str>, method: &str) -> Result<()> {
    let denied = || RPCErrors::NoPermission(format!("only control-panel may call {method}"));
    let caller_token = caller_token.ok_or_else(denied)?;
    let payload = verify_verify_hub_jwt(caller_token, None)
        .await
        .map_err(|error| {
            warn!("{} caller token rejected: {}", method, error);
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
        session_key: family_key(&user_id, &control_panel, session_id),
        user_id,
    }))
}

/// Non-user principals still stand: a device still has its config, an agent its AgentDID key.
async fn validate_service_principal(
    subject: &str,
    principal_kind: SessionPrincipalKind,
) -> Result<()> {
    match principal_kind {
        SessionPrincipalKind::Device => {
            let control_panel_client = ControlPanelClient::new(get_system_config_client().await?);
            control_panel_client.get_device_config(subject).await?;
            Ok(())
        }
        SessionPrincipalKind::Agent => {
            let trusted = load_trust_public_key_from_source(subject).await?;
            if trusted.issuer_kind != TrustedIssuerKind::Agent {
                return Err(RPCErrors::InvalidToken(
                    "agent token subject is not an AgentDID".to_string(),
                ));
            }
            Ok(())
        }
        SessionPrincipalKind::User | SessionPrincipalKind::System | SessionPrincipalKind::App => {
            Ok(())
        }
    }
}

async fn check_app_principal_target(subject: &str, target: &AuthTarget) -> Result<()> {
    if let AuthTarget::App { app_instance_id } = target {
        if subject != app_instance_id.owner_user_id() {
            return Err(AccessDenial::new(DenyReason::EntryUnavailable)
                .with_detail("app_subject_owner_mismatch")
                .into_rpc_error());
        }
        buckyos_api::check_app_runtime_session(&get_system_config_client().await?, app_instance_id)
            .await
            .map_err(|error| {
                warn!("App session refused for {}: {}", app_instance_id, error);
                error
            })?;
    }
    Ok(())
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
        login_assertion_denied("login_assertion_invalid")
    })?;

    if header.alg != Algorithm::EdDSA {
        return Err(login_assertion_denied("login_assertion_invalid"));
    }

    let mut validation = Validation::new(Algorithm::EdDSA);
    // We don't have an expected audience for generic trusted JWTs.
    validation.validate_aud = false;

    // Get iss from claims by decoding the payload part (without signature verification)
    // JWT format: header.payload.signature
    let claims = decode_jwt_claim_without_verify(jwt).map_err(|error| {
        error!("decode_jwt_claim_without_verify error: {}", error);
        login_assertion_denied("login_assertion_invalid")
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
                        login_assertion_denied("login_assertion_invalid")
                    },
                )?
            } else {
                error!("JWT verify error: {}", error);
                return Err(login_assertion_denied(match error.kind() {
                    ErrorKind::ExpiredSignature => "login_assertion_expired",
                    _ => "login_assertion_invalid",
                }));
            }
        }
    };

    Ok((decoded_token.claims, trusted_key.issuer_kind))
}

async fn verify_verify_hub_jwt(jwt: &str, expected_audience: Option<&str>) -> Result<Value> {
    decode_verify_hub_jwt(jwt, expected_audience, true).await
}

/// `check_exp: false` only identifies a token this hub signed (e.g. which
/// family to spare); it never authorizes anything.
async fn decode_verify_hub_jwt(
    jwt: &str,
    expected_audience: Option<&str>,
    check_exp: bool,
) -> Result<Value> {
    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        error!("JWT decode header error: {}", error);
        RPCErrors::InvalidToken("JWT decode header error".to_string())
    })?;

    if header.alg != Algorithm::EdDSA {
        return Err(RPCErrors::InvalidToken(
            "JWT algorithm not allowed".to_string(),
        ));
    }

    let claims = decode_jwt_claim_without_verify(jwt).map_err(|error| {
        error!("decode_jwt_claim_without_verify error: {}", error);
        RPCErrors::InvalidToken("decode_jwt_claim_without_verify error".to_string())
    })?;

    let iss = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .unwrap_or(VERIFY_HUB_ISSUER);

    if iss != VERIFY_HUB_ISSUER {
        return Err(RPCErrors::InvalidToken("JWT kid not allowed".to_string()));
    }

    // Always verify verify-hub issued tokens by verify-hub public key
    let public_key = get_trust_public_key(VERIFY_HUB_ISSUER, &None).await?;

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[VERIFY_HUB_ISSUER]);
    validation.set_required_spec_claims(&["exp", "iss"]);
    validation.validate_exp = check_exp;

    if let Some(aud) = expected_audience {
        validation.set_audience(&[aud]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    } else {
        validation.validate_aud = false;
    }

    let decoded_token =
        jsonwebtoken::decode::<Value>(jwt, &public_key.key, &validation).map_err(|error| {
            error!("JWT verify error: {}", error);
            match error.kind() {
                ErrorKind::ExpiredSignature => RPCErrors::TokenExpired("JWT expired".to_string()),
                _ => RPCErrors::InvalidToken("JWT verify error".to_string()),
            }
        })?;

    Ok(decoded_token.claims)
}

async fn mark_used(key: &str, until: u64) {
    TOKEN_CACHE.lock().await.insert(key.to_string(), until);
}

async fn is_used(key: &str) -> bool {
    TOKEN_CACHE.lock().await.contains_key(key)
}

/**
curl -X POST http://127.0.0.1/kapi/verify_hub -H "Content-Type: application/json" -d '{"method": "login","params":{"type":"jwt","jwt":"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJodHRwczovL3d3dy53aGl0ZS5ib3Vjay5pbyIsImF1ZCI6Imh0dHBzOi8vd3d3LndoaXRlLmJvdWNrLmlvIiwiZXhwIjoxNzI3NzIwMDAwLCJpYXQiOjE3Mjc3MTY0MDAsInVzZXJpZCI6ImRpZDpleGFtcGxlOjEyMzQ1Njc4OTAiLCJhcHBpZCI6InN5c3RvbSIsInVzZXJuYW1lIjoiYWxpY2UifQ.6XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ5"}}'
curl -X POST http://127.0.0.1:3300/kapi/verify_hub -H "Content-Type: application/json" -d '{"method": "login","params":{"type":"password","username":"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJodHRwczovL3d3dy53aGl0ZS5ib3Vjay5pbyIsImF1ZCI6Imh0dHBzOi8vd3d3LndoaXRlLmJvdWNrLmlvIiwiZXhwIjoxNzI3NzIwMDAwLCJpYXQiOjE3Mjc3MTY0MDAsInVzZXJpZCI6ImRpZDpleGFtcGxlOjEyMzQ1Njc4OTAiLCJhcHBpZCI6InN5c3RvbSIsInVzZXJuYW1lIjoiYWxpY2UifQ.6XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ56XQ5"}}'
 */
struct HubState {
    families: FamilyStore,
    zone: Arc<dyn ZoneDirectory>,
    federation: Arc<dyn FederationPeer>,
    federation_codes: FederationCodes,
    last_family_gc: AtomicU64,
}

#[derive(Clone)]
struct VerifyHubServer {
    state: Arc<HubState>,
}

/// Whether a family may answer again after the D-5 recheck.
enum FamilyRecheck {
    Allowed(FamilyClaims),
    /// The family no longer stands and was removed; the error to report.
    Ended(RPCErrors),
}

enum DidAccount {
    Active(UserSettings),
    Inactive,
    Unknown,
}

impl VerifyHubServer {
    fn new(
        families: FamilyStore,
        zone: Arc<dyn ZoneDirectory>,
        federation: Arc<dyn FederationPeer>,
    ) -> Self {
        VerifyHubServer {
            state: Arc::new(HubState {
                families,
                zone,
                federation,
                federation_codes: FederationCodes::default(),
                last_family_gc: AtomicU64::new(0),
            }),
        }
    }

    fn families(&self) -> &FamilyStore {
        &self.state.families
    }

    async fn gc(&self) {
        let now = buckyos_get_unix_timestamp();
        TOKEN_CACHE.lock().await.retain(|_, until| *until > now);
        let last = self.state.last_family_gc.load(Ordering::Relaxed);
        if now >= last + FAMILY_GC_INTERVAL_SECONDS
            && self
                .state
                .last_family_gc
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            if let Err(error) = self.families().gc(now) {
                warn!("session family gc failed: {}", error);
            }
        }
    }

    async fn revoke_family(&self, session_key: &str) {
        TOKEN_CACHE.lock().await.remove(session_key);
        if let Err(error) = self.families().remove(session_key) {
            error!("revoke session family {} failed: {}", session_key, error);
        }
    }

    async fn issue_family(
        &self,
        session: SessionSubject,
        source: Option<SessionSource>,
        absolute_exp: Option<u64>,
    ) -> Result<TokenPair> {
        let now = buckyos_get_unix_timestamp();
        let stamp = fresh_stamp(now, absolute_exp);
        let token_pair = sign_pair(&session, &stamp).await?;
        self.families().insert(&FamilyRecord {
            session,
            stamp,
            source,
            parent_key: None,
            absolute_exp,
            created_at: now,
            previous: None,
        })?;
        Ok(token_pair)
    }

    /// The account a user session is issued for or refreshed as: it exists and is Active.
    async fn active_account(&self, user_id: &str) -> Result<UserSettings> {
        let user_settings = self
            .state
            .zone
            .user_settings(user_id)
            .await?
            .ok_or_else(account_state_denied)?;
        reject_root_user_settings(&user_settings)?;
        require_active_user_settings(&user_settings)?;
        Ok(user_settings)
    }

    async fn resolve_user_auth_target(
        &self,
        user_id: &str,
        target: &AuthTarget,
    ) -> Result<AuthTarget> {
        match target {
            AuthTarget::App { app_instance_id } => {
                let decision = self
                    .state
                    .zone
                    .app_availability(user_id, app_instance_id)
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

    async fn validate_password_login(
        &self,
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

        let session_key = family_key(username, target, login_nonce);
        if is_used(session_key.as_str()).await {
            warn!(
                "{} login nonce {} already used, this is a REPLAY ATTACK!",
                username, login_nonce
            );
            warn!(
                "Revoking session {} due to replay attack detection",
                session_key
            );
            self.revoke_family(session_key.as_str()).await;
            return Err(RPCErrors::ReasonError(
                "Login nonce already used".to_string(),
            ));
        }

        let user_settings = self
            .state
            .zone
            .user_settings(username)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("user info not found".to_string()))?;
        reject_root_user_settings(&user_settings)?;

        if user_settings.password_state == PasswordState::Unset {
            info!("{} has not set a password yet", username);
            return Err(AccessDenial::new(DenyReason::CredentialState)
                .with_detail("password_unset")
                .into_rpc_error());
        }
        if user_settings.password.is_empty() {
            warn!(
                "{} has no stored password and cannot log in by password",
                username
            );
            return Err(RPCErrors::InvalidPassword);
        }

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

    /// D-5: before a family answers again (refresh, grace, delegation) its principal, ceiling,
    /// source and parent are checked anew. A denial or an ended parent removes the family.
    async fn recheck_family(&self, family: &FamilyRecord) -> Result<FamilyRecheck> {
        let key = family.key();
        let now = buckyos_get_unix_timestamp();
        let parent_ended = match family.parent_key.as_deref() {
            Some(parent) => !self.families().is_live(parent, now)?,
            None => false,
        };
        if parent_ended || !family.is_live(now) {
            info!(
                "session family {} ended with its parent or absolute expiry",
                key
            );
            self.revoke_family(&key).await;
            return Ok(FamilyRecheck::Ended(family_invalidated()));
        }
        match self.recheck_principal(family).await {
            Ok(claims) => Ok(FamilyRecheck::Allowed(claims)),
            Err(error) if AccessDenial::from_rpc_error(&error).is_some() => {
                warn!("session family {} refused on recheck: {}", key, error);
                self.revoke_family(&key).await;
                Ok(FamilyRecheck::Ended(error))
            }
            Err(error) => Err(error),
        }
    }

    /// The claims the family may carry now: never wider than the account's current type.
    async fn recheck_principal(&self, family: &FamilyRecord) -> Result<FamilyClaims> {
        let session = &family.session;
        let mut claims = session.claims.clone();
        match session.principal_kind {
            SessionPrincipalKind::User if claims.external => {}
            SessionPrincipalKind::User => {
                let user_settings = self.active_account(&session.subject).await?;
                claims = claims.narrowed(&SessionCeiling::for_user_type(user_settings.user_type));
                self.resolve_user_auth_target(&session.subject, &session.target)
                    .await?;
            }
            principal_kind => {
                validate_service_principal(&session.subject, principal_kind).await?;
                if principal_kind == SessionPrincipalKind::App {
                    check_app_principal_target(&session.subject, &session.target).await?;
                }
            }
        }
        if let Some(SessionSource::ExclusiveInvite { invite_id, .. }) = &family.source {
            let active = self
                .state
                .zone
                .exclusive_invite(invite_id)
                .await?
                .is_some_and(|invite| invite.state == InviteState::Active);
            if !active {
                return Err(AccessDenial::new(DenyReason::SourceRevoked)
                    .with_detail("invite_revoked")
                    .into_rpc_error());
            }
        }
        Ok(claims)
    }

    async fn refuse_presented_refresh(
        &self,
        presented: &PresentedRefresh,
        presentation: RefreshPresentation,
    ) -> RPCErrors {
        if presentation == RefreshPresentation::Missing {
            warn!(
                "Refresh reuse detected (cache-miss), revoking session: {}",
                presented.session_key
            );
            self.revoke_family(presented.session_key.as_str()).await;
            family_invalidated()
        } else {
            warn!(
                "Refresh reuse detected (jti-mismatch), revoking session: {}",
                presented.session_key
            );
            self.revoke_family(presented.session_key.as_str()).await;
            refresh_credential_denied("refresh_token_reused")
        }
    }

    /// The local account `did` belongs to: `users/<id>/doc` names it. Root never maps.
    async fn account_for_did(&self, did: &DID) -> Result<DidAccount> {
        let mut inactive = false;
        for user_id in self.state.zone.list_users().await? {
            if is_root_userid(&user_id) {
                continue;
            }
            let Some(doc) = self.state.zone.user_doc(&user_id).await? else {
                continue;
            };
            if doc.did != *did {
                continue;
            }
            let Some(user_settings) = self.state.zone.user_settings(&user_id).await? else {
                continue;
            };
            if !user_settings.user_type.can_login() {
                continue;
            }
            if user_is_active(&user_settings) {
                return Ok(DidAccount::Active(user_settings));
            }
            inactive = true;
        }
        Ok(if inactive {
            DidAccount::Inactive
        } else {
            DidAccount::Unknown
        })
    }
}

/// Login decision for an exclusive invite link (contract §5.3), in order: invite missing or
/// revoked, account not Active (or not Limited), password already set, then the credential.
fn exclusive_invite_decision<'a>(
    invite: Option<&'a ExclusiveInvite>,
    account: Option<&'a UserSettings>,
    secret_digest: &str,
    now: u64,
) -> std::result::Result<(&'a ExclusiveInvite, &'a UserSettings), AccessDenial> {
    let Some(invite) = invite.filter(|invite| invite.state == InviteState::Active) else {
        return Err(InviteLoginRefusal::Revoked.denial());
    };
    let Some(account) =
        account.filter(|account| account.user_id == invite.user_id && user_is_active(account))
    else {
        return Err(InviteLoginRefusal::AccountInactive.denial());
    };
    if account.password_state == PasswordState::Set {
        return Err(InviteLoginRefusal::PasswordRequired.denial());
    }
    if account.user_type != UserType::Limited {
        return Err(InviteLoginRefusal::AccountInactive
            .denial()
            .with_detail("not_limited"));
    }
    invite
        .check_credential(secret_digest, now)
        .map_err(InviteLoginRefusal::denial)?;
    Ok((invite, account))
}

fn revoke_request_matches(request: &RevokeFamiliesRequest, family: &FamilyRecord) -> bool {
    let method_matches = request.auth_method.map_or(true, |method| {
        family.session.claims.auth_method == Some(method)
    });
    let source_matches = match (request.source_invite_id.as_deref(), &family.source) {
        (None, _) => true,
        (Some(id), Some(SessionSource::ExclusiveInvite { invite_id, .. })) => invite_id == id,
        (Some(_), _) => false,
    };
    method_matches && source_matches
}

fn valid_record_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn require_field(method: &str, name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(RPCErrors::ParseRequestError(format!(
            "{method}: {name} is required"
        )));
    }
    Ok(())
}

fn parse_did_field(method: &str, name: &str, value: &str) -> Result<DID> {
    DID::from_str(value)
        .map_err(|error| RPCErrors::ParseRequestError(format!("{method}: invalid {name}: {error}")))
}

#[async_trait]
impl VerifyHubApiHandler for VerifyHubServer {
    async fn handle_login_by_jwt(&self, jwt: &str, target: &AuthTarget) -> Result<TokenPair> {
        self.gc().await;

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
            if matches!(target, AuthTarget::App { .. }) {
                SessionPrincipalKind::App
            } else if rpc_session_token.iss.as_deref() == Some(userid.as_str()) {
                SessionPrincipalKind::Device
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
        let claims = if principal_kind == SessionPrincipalKind::User {
            let user_settings = self.active_account(&userid).await?;
            FamilyClaims::account(AuthMethod::Assertion, user_settings.user_type)
        } else {
            FamilyClaims::default()
        };
        let target = if principal_kind == SessionPrincipalKind::User {
            self.resolve_user_auth_target(&userid, target).await?
        } else {
            target.clone()
        };
        if principal_kind == SessionPrincipalKind::App {
            check_app_principal_target(&userid, &target).await?;
        }

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
        let assertion_key = format!("assertion:{assertion_issuer}:{userid}:{token_jti}");

        // Step 4: Check if this login JWT has already been used (replay protection)
        if is_used(assertion_key.as_str()).await {
            return Err(login_assertion_denied("login_assertion_used"));
        }

        // Step 5: Generate new session_id for this login session
        let session_id: u64 = rand::thread_rng().gen();
        let session = SessionSubject {
            subject: userid.clone(),
            principal_kind,
            target,
            session_id,
            claims,
        };
        let session_key = session.family_key();

        // Step 6: Mark the assertion used, then open the family
        let now = buckyos_get_unix_timestamp();
        mark_used(
            assertion_key.as_str(),
            rpc_session_token
                .exp
                .unwrap_or(0)
                .max(now + SESSION_TOKEN_EXPIRE_SECONDS),
        )
        .await;
        let token_pair = self.issue_family(session, None, None).await?;

        info!(
            "Login successful for user: {}. Session: {}. Token pair generated.",
            userid, session_key
        );

        Ok(token_pair)
    }

    async fn handle_refresh_token(&self, refresh_jwt: &str) -> Result<TokenPair> {
        self.gc().await;
        let presented =
            verify_presented_refresh(refresh_jwt)
                .await
                .map_err(|error| match error {
                    RPCErrors::TokenExpired(_) => {
                        refresh_credential_denied("refresh_token_expired")
                    }
                    RPCErrors::InvalidToken(_) => {
                        refresh_credential_denied("refresh_token_invalid")
                    }
                    other => other,
                })?;
        let now = buckyos_get_unix_timestamp();
        let family = self.families().get(presented.session_key.as_str())?;
        let presentation = classify_refresh_presentation(family.as_ref(), &presented.jti, now);
        let family = match (family, presentation) {
            (Some(family), RefreshPresentation::Current | RefreshPresentation::Grace) => family,
            (_, presentation) => {
                return Err(self
                    .refuse_presented_refresh(&presented, presentation)
                    .await)
            }
        };
        let claims = match self.recheck_family(&family).await? {
            FamilyRecheck::Allowed(claims) => claims,
            FamilyRecheck::Ended(error) => return Err(error),
        };

        if presentation == RefreshPresentation::Grace {
            if claims != family.session.claims {
                self.families()
                    .update_claims(presented.session_key.as_str(), &claims)?;
            }
            info!(
                "Refresh within grace period for session: {}, returning current token pair",
                presented.session_key
            );
            let session = SessionSubject {
                claims,
                ..family.session
            };
            return sign_pair(&session, &family.stamp).await;
        }

        info!(
            "Handle refresh token request for session: {}",
            presented.session_key
        );

        let session = SessionSubject {
            claims: claims.clone(),
            ..family.session.clone()
        };
        let stamp = fresh_stamp(now, family.absolute_exp);
        let token_pair = sign_pair(&session, &stamp).await?;

        match self.families().rotate(
            presented.session_key.as_str(),
            presented.jti.as_str(),
            &stamp,
            &claims,
            now,
        )? {
            RefreshCommit::Committed => {
                info!(
                    "Refresh successful for session: {}. New token pair generated.",
                    presented.session_key
                );
                Ok(token_pair)
            }
            RefreshCommit::Grace(current) => {
                info!(
                    "Concurrent refresh for session: {} already rotated, returning current token pair",
                    presented.session_key
                );
                sign_pair(&current.session, &current.stamp).await
            }
            RefreshCommit::LoggedOut => {
                warn!(
                    "Refresh for session: {} lost to a logout, not committing",
                    presented.session_key
                );
                Err(family_invalidated())
            }
            RefreshCommit::Reused => {
                warn!(
                    "Refresh reuse detected (concurrent rotation), revoking session: {}",
                    presented.session_key
                );
                self.revoke_family(presented.session_key.as_str()).await;
                Err(refresh_credential_denied("refresh_token_reused"))
            }
        }
    }

    async fn handle_logout(&self, refresh_jwt: &str) -> Result<bool> {
        self.gc().await;
        let presented = verify_presented_refresh(refresh_jwt).await?;
        let now = buckyos_get_unix_timestamp();
        let family = self.families().get(presented.session_key.as_str())?;
        match classify_refresh_presentation(family.as_ref(), &presented.jti, now) {
            RefreshPresentation::Current | RefreshPresentation::Grace => {}
            presentation => {
                return Err(self
                    .refuse_presented_refresh(&presented, presentation)
                    .await)
            }
        }

        info!(
            "Handle logout request for session: {}",
            presented.session_key
        );
        self.revoke_family(presented.session_key.as_str()).await;
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
        self.gc().await;
        reject_root_session_subject(username)?;

        let (user_settings, nonce_key) = self
            .validate_password_login(username, password, target, login_nonce)
            .await?;
        require_active_user_settings(&user_settings)?;
        let target = self.resolve_user_auth_target(username, target).await?;

        info!(
            "Password login successful for user: {}. Generating token pair.",
            username
        );

        let session = SessionSubject {
            subject: username.to_string(),
            principal_kind: SessionPrincipalKind::User,
            target,
            session_id: login_nonce,
            claims: FamilyClaims::account(AuthMethod::Password, user_settings.user_type),
        };
        // The nonce replay cache is memory only; a family of this nonce surviving
        // a restart still marks the login as already used.
        if self
            .families()
            .get(session.family_key().as_str())?
            .is_some()
        {
            warn!(
                "{} login nonce {} already opened a family",
                username, login_nonce
            );
            return Err(RPCErrors::ReasonError(
                "Login nonce already used".to_string(),
            ));
        }
        mark_used(
            nonce_key.as_str(),
            buckyos_get_unix_timestamp() + SESSION_TOKEN_EXPIRE_SECONDS,
        )
        .await;
        let token_pair = self.issue_family(session, None, None).await?;

        info!("Session family opened: {}", nonce_key);

        Ok(LoginByPasswordResponse {
            user_info: user_settings.to_user_info(),
            session_token: token_pair.session_token,
            refresh_token: token_pair.refresh_token,
        })
    }

    async fn handle_sudo_by_password(
        &self,
        username: &str,
        password: &str,
        target: &AuthTarget,
        aud: Option<String>,
        login_nonce: u64,
    ) -> Result<SudoByPasswordResponse> {
        self.gc().await;
        reject_root_session_subject(username)?;

        let (user_settings, nonce_key) = self
            .validate_password_login(username, password, target, login_nonce)
            .await?;
        require_active_user_settings(&user_settings)?;
        let ceiling = SessionCeiling::for_user_type(user_settings.user_type);
        if ceiling.restricted {
            return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)
                .with_detail("sudo")
                .into_rpc_error());
        }
        let target = self.resolve_user_auth_target(username, target).await?;

        let session = SessionSubject {
            subject: username.to_string(),
            principal_kind: SessionPrincipalKind::User,
            target,
            session_id: login_nonce,
            claims: FamilyClaims::account(AuthMethod::Password, user_settings.user_type),
        };
        let exp = buckyos_get_unix_timestamp() + SUDO_SESSION_TOKEN_EXPIRE_SECONDS;
        let jti = rand::thread_rng().gen::<u64>().to_string();
        let session_token =
            sign_session_token(&session, TokenUse::Session, &jti, exp, aud, true).await?;

        mark_used(nonce_key.as_str(), exp).await;
        info!("Sudo session token issued for session: {}", nonce_key);

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
        self.gc().await;
        authorize_control_panel_caller(caller_token, "delegate_user_session").await?;
        if !matches!(target, AuthTarget::App { .. }) {
            return Err(RPCErrors::NoPermission(
                "delegate_user_session only issues App targets".to_string(),
            ));
        }
        let Some(source) = verify_delegation_source(source_session_token).await? else {
            return Ok(DelegateUserSessionResult::SourceInactive);
        };
        let now = buckyos_get_unix_timestamp();
        let Some(source_family) = self
            .families()
            .get(source.session_key.as_str())?
            .filter(|family| family.is_live(now))
        else {
            info!(
                "delegation source session is not active: {}",
                source.session_key
            );
            return Ok(DelegateUserSessionResult::SourceInactive);
        };
        let claims = match self.recheck_family(&source_family).await? {
            FamilyRecheck::Allowed(claims) => claims,
            FamilyRecheck::Ended(_) => return Ok(DelegateUserSessionResult::SourceInactive),
        };
        if claims.external {
            return Ok(DelegateUserSessionResult::AccessDenied);
        }
        let target = match self
            .resolve_user_auth_target(source.user_id.as_str(), target)
            .await
        {
            Ok(target) => target,
            Err(RPCErrors::NoPermission(_)) => {
                return Ok(DelegateUserSessionResult::AccessDenied);
            }
            Err(error) => return Err(error),
        };

        // A restricted delegation keeps the source's ceiling and dies with the source family;
        // an unrestricted one stays independent. Both keep the source and its absolute expiry.
        let session = SessionSubject {
            subject: source.user_id.clone(),
            principal_kind: SessionPrincipalKind::User,
            target,
            session_id: rand::thread_rng().gen(),
            claims,
        };
        let session_key = session.family_key();
        let stamp = fresh_stamp(now, source_family.absolute_exp);
        let token_pair = sign_pair(&session, &stamp).await?;
        let parent_key = session
            .claims
            .restricted
            .then(|| source.session_key.clone());
        let family = FamilyRecord {
            session,
            stamp,
            source: source_family.source.clone(),
            parent_key,
            absolute_exp: source_family.absolute_exp,
            created_at: now,
            previous: None,
        };
        if !self
            .families()
            .insert_delegated(source.session_key.as_str(), &family, now)?
        {
            info!(
                "delegation source session ended before commit: {}",
                source.session_key
            );
            return Ok(DelegateUserSessionResult::SourceInactive);
        }
        info!(
            "Delegated session of {} to {}. Session: {}",
            source.session_key,
            family.session.target.canonical_key(),
            session_key
        );
        Ok(DelegateUserSessionResult::Issued(token_pair))
    }

    async fn handle_verify_token(
        &self,
        session_token: &str,
        expected_target: Option<AuthTarget>,
    ) -> Result<bool> {
        self.gc().await;
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
            token_session_ceiling(&rpc_session_token)?;
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

    async fn handle_login_by_exclusive_invite(
        &self,
        caller_token: Option<&str>,
        request: ExclusiveInviteLoginRequest,
    ) -> Result<ExclusiveInviteLoginResult> {
        const METHOD: &str = "login_by_exclusive_invite";
        self.gc().await;
        authorize_control_panel_caller(caller_token, METHOD).await?;
        let target = require_control_panel_target(&request.target, METHOD)?;

        let invite = if valid_record_id(&request.invite_id) {
            self.state.zone.exclusive_invite(&request.invite_id).await?
        } else {
            None
        };
        let account = match invite
            .as_ref()
            .filter(|invite| invite.state == InviteState::Active)
        {
            Some(invite) => self.state.zone.user_settings(&invite.user_id).await?,
            None => None,
        };
        let now = buckyos_get_unix_timestamp();
        let (invite, account) = match exclusive_invite_decision(
            invite.as_ref(),
            account.as_ref(),
            &invite_secret_digest(&request.secret),
            now,
        ) {
            Ok(granted) => granted,
            Err(denial) => {
                info!(
                    "exclusive invite {} refused: {:?}",
                    request.invite_id, denial.detail
                );
                return Ok(ExclusiveInviteLoginResult::Denied { denial });
            }
        };

        let session = SessionSubject {
            subject: account.user_id.clone(),
            principal_kind: SessionPrincipalKind::User,
            target,
            session_id: rand::thread_rng().gen(),
            claims: FamilyClaims::account(AuthMethod::ExclusiveInvite, UserType::Limited),
        };
        let token_pair = self
            .issue_family(
                session,
                Some(SessionSource::ExclusiveInvite {
                    invite_id: invite.invite_id.clone(),
                    version: invite.credential.version,
                }),
                None,
            )
            .await?;
        info!(
            "exclusive invite {} v{} signed in {}",
            invite.invite_id, invite.credential.version, account.user_id
        );
        Ok(ExclusiveInviteLoginResult::Issued {
            user_id: account.user_id.clone(),
            token_pair,
        })
    }

    async fn handle_revoke_families(
        &self,
        caller_token: Option<&str>,
        request: RevokeFamiliesRequest,
    ) -> Result<RevokeFamiliesResult> {
        const METHOD: &str = "revoke_families";
        self.gc().await;
        authorize_control_panel_caller(caller_token, METHOD).await?;
        require_field(METHOD, "subject", &request.subject)?;
        let keep = match request.except_session_token.as_deref() {
            Some(token) => Some(session_family_key(token).await?),
            None => None,
        };
        let revoked =
            self.families()
                .revoke_matching(&request.subject, keep.as_deref(), |family| {
                    revoke_request_matches(&request, family)
                })?;
        info!(
            "revoked {} session families of {} (auth_method {:?}, invite {:?})",
            revoked, request.subject, request.auth_method, request.source_invite_id
        );
        Ok(RevokeFamiliesResult { revoked })
    }

    async fn handle_issue_federation_code(
        &self,
        caller_token: Option<&str>,
        request: IssueFederationCodeRequest,
    ) -> Result<IssueFederationCodeResult> {
        const METHOD: &str = "issue_federation_code";
        authorize_control_panel_caller(caller_token, METHOD).await?;
        for (name, value) in [
            ("user_id", &request.user_id),
            ("redirect_uri", &request.redirect_uri),
            ("nonce", &request.nonce),
            ("code_challenge", &request.code_challenge),
        ] {
            require_field(METHOD, name, value)?;
        }
        let did = parse_did_field(METHOD, "did", &request.did)?;
        parse_did_field(METHOD, "client_zone", &request.client_zone)?;

        let user_settings = self
            .state
            .zone
            .user_settings(&request.user_id)
            .await?
            .filter(user_is_active)
            .ok_or_else(account_state_denied)?;
        if !user_settings.user_type.can_federate_out() {
            return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed).into_rpc_error());
        }
        let doc = self
            .state
            .zone
            .user_doc(&request.user_id)
            .await?
            .ok_or_else(account_state_denied)?;
        if doc.did != did {
            return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)
                .with_detail("did_mismatch")
                .into_rpc_error());
        }
        if doc.default_zone.as_ref() != Some(&self.state.zone.zone_did().await?) {
            return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)
                .with_detail("not_canonical_zone")
                .into_rpc_error());
        }

        let now = buckyos_get_unix_timestamp();
        let code = self.state.federation_codes.issue(
            FederationCode {
                did: request.did,
                client_zone: request.client_zone.clone(),
                redirect_uri: request.redirect_uri,
                nonce: request.nonce,
                code_challenge: request.code_challenge,
                profile: request.profile,
                issued_at: now,
            },
            now,
        );
        info!(
            "issued federation code for {} to {}",
            request.user_id, request.client_zone
        );
        Ok(IssueFederationCodeResult {
            code,
            expires_in: FEDERATION_CODE_TTL_SECONDS,
        })
    }

    async fn handle_exchange_federation_code(
        &self,
        request: ExchangeFederationCodeRequest,
    ) -> Result<ExchangeFederationCodeResult> {
        let result = self
            .state
            .federation_codes
            .exchange(&request, buckyos_get_unix_timestamp());
        if result == ExchangeFederationCodeResult::InvalidCode {
            info!(
                "federation code exchange refused for {}",
                request.client_zone
            );
        }
        Ok(result)
    }

    async fn handle_login_by_remote_verify_hub(
        &self,
        caller_token: Option<&str>,
        request: LoginByRemoteVerifyHubRequest,
    ) -> Result<LoginByRemoteVerifyHubResult> {
        const METHOD: &str = "login_by_remote_verify_hub";
        self.gc().await;
        authorize_control_panel_caller(caller_token, METHOD).await?;
        let target = require_control_panel_target(&request.target, METHOD)?;
        let did = parse_did_field(METHOD, "did", &request.did)?;
        let denied = |denial: AccessDenial| Ok(LoginByRemoteVerifyHubResult::Denied { denial });

        let sso_settings = self.state.zone.sso_settings().await?;
        if !sso_settings.did_login_enabled {
            return denied(AccessDenial::new(DenyReason::ZoneMethodDisabled));
        }
        let canonical_host = match tokio::time::timeout(
            FEDERATION_PEER_TIMEOUT,
            self.state.federation.canonical_host(&did),
        )
        .await
        .unwrap_or_else(|_| {
            Err(RPCErrors::ReasonError(
                "canonical zone resolution timed out".to_string(),
            ))
        }) {
            Ok(host) => host,
            Err(error) => {
                warn!(
                    "canonical zone of {} unavailable: {}",
                    did.to_string(),
                    error
                );
                return Ok(LoginByRemoteVerifyHubResult::CanonicalUnavailable);
            }
        };
        let zone_did = self.state.zone.zone_did().await?.to_string();
        let exchange = ExchangeFederationCodeRequest {
            code: request.code,
            code_verifier: request.code_verifier,
            client_zone: zone_did.clone(),
            redirect_uri: request.redirect_uri,
        };
        let assertion = match tokio::time::timeout(
            FEDERATION_PEER_TIMEOUT,
            self.state.federation.exchange(
                &canonical_host,
                sso_settings.dev_federation_http,
                exchange,
            ),
        )
        .await
        .unwrap_or_else(|_| {
            Err(RPCErrors::ReasonError(
                "federation code exchange timed out".to_string(),
            ))
        }) {
            Ok(ExchangeFederationCodeResult::Ok { assertion }) => assertion,
            Ok(ExchangeFederationCodeResult::InvalidCode) => {
                return denied(
                    AccessDenial::new(DenyReason::CredentialState).with_detail("invalid_code"),
                );
            }
            Err(error) => {
                warn!(
                    "federation code exchange with {} failed: {}",
                    canonical_host, error
                );
                return Ok(LoginByRemoteVerifyHubResult::CanonicalUnavailable);
            }
        };
        if DID::from_str(&assertion.did).ok().as_ref() != Some(&did)
            || assertion.client_zone != zone_did
            || assertion.nonce != request.nonce
            || assertion.purpose != FEDERATION_PURPOSE_LOGIN
        {
            warn!(
                "federation assertion from {} does not match the login of {}",
                canonical_host,
                did.to_string()
            );
            return denied(
                AccessDenial::new(DenyReason::CredentialState).with_detail("assertion_mismatch"),
            );
        }

        let (user_id, session) = match self.account_for_did(&did).await? {
            DidAccount::Active(user_settings) => (
                Some(user_settings.user_id.clone()),
                SessionSubject {
                    subject: user_settings.user_id,
                    principal_kind: SessionPrincipalKind::User,
                    target,
                    session_id: rand::thread_rng().gen(),
                    claims: FamilyClaims::account(
                        AuthMethod::DidFederation,
                        user_settings.user_type,
                    ),
                },
            ),
            DidAccount::Inactive => return denied(AccessDenial::new(DenyReason::AccountState)),
            DidAccount::Unknown => (
                None,
                SessionSubject {
                    subject: request.did.clone(),
                    principal_kind: SessionPrincipalKind::User,
                    target,
                    session_id: rand::thread_rng().gen(),
                    claims: FamilyClaims::external(AuthMethod::DidFederation),
                },
            ),
        };
        let absolute_exp = buckyos_get_unix_timestamp() + FEDERATION_SESSION_MAX_SECONDS;
        let token_pair = self
            .issue_family(
                session,
                Some(SessionSource::DidFederation {
                    canonical_host: canonical_host.clone(),
                }),
                Some(absolute_exp),
            )
            .await?;
        info!(
            "DID login of {} via {} as {}",
            did.to_string(),
            canonical_host,
            user_id.as_deref().unwrap_or("External")
        );
        Ok(LoginByRemoteVerifyHubResult::Issued {
            user_id,
            token_pair,
            profile: assertion.profile,
        })
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

fn load_web3_bridge_config() -> HashMap<String, String> {
    let path = get_buckyos_system_etc_dir().join("machine.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<BuckyOSMachineConfig>(&raw).ok())
        .unwrap_or_default()
        .web3_bridge
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

    if let Err(error) = name_client::init_name_lib(&load_web3_bridge_config()).await {
        warn!("name lib init failed, DID login cannot resolve canonical zones: {error}");
    }
    let families_path = get_buckyos_service_data_dir(VERIFY_HUB_SERVICE_NAME).join("sessions.db");
    let families = match FamilyStore::open(&families_path) {
        Ok(families) => families,
        Err(error) => {
            error!(
                "open session family store {} failed: {}",
                families_path.display(),
                error
            );
            return -1;
        }
    };

    let server = VerifyHubServer::new(
        families,
        Arc::new(SystemConfigDirectory),
        Arc::new(NameFederationPeer),
    );
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

    use directory::UserDoc;
    use family_store::REFRESH_GRACE_SECONDS;
    use federation::pkce_s256;
    use serde_json::json;
    use std::collections::HashSet;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn system_target(service_id: &str) -> AuthTarget {
        AuthTarget::system(service_id.parse().unwrap())
    }

    fn app_target(app_instance_id: &str) -> AuthTarget {
        AuthTarget::app(app_instance_id.parse().unwrap())
    }

    // ─── an in-memory Zone ────────────────────────────────────────────────

    #[derive(Default)]
    struct DirectoryData {
        zone_did: Option<DID>,
        users: HashMap<String, UserSettings>,
        docs: HashMap<String, UserDoc>,
        invites: HashMap<String, ExclusiveInvite>,
        sso: SsoSettings,
        apps: HashSet<(String, String)>,
    }

    struct MemoryDirectory(std::sync::Mutex<DirectoryData>);

    impl MemoryDirectory {
        fn new(zone_did: &str) -> Arc<Self> {
            Arc::new(Self(std::sync::Mutex::new(DirectoryData {
                zone_did: Some(DID::from_str(zone_did).unwrap()),
                ..Default::default()
            })))
        }

        fn data(&self) -> std::sync::MutexGuard<'_, DirectoryData> {
            self.0.lock().unwrap()
        }

        /// `password: None` is an account without a password (exclusive invite).
        fn add_user(&self, user_id: &str, user_type: UserType, password: Option<&str>) {
            self.data().users.insert(
                user_id.to_string(),
                UserSettings {
                    user_id: user_id.to_string(),
                    user_type,
                    password: password.unwrap_or_default().to_string(),
                    state: UserState::Active,
                    res_pool_id: "default".to_string(),
                    is_local: true,
                    allow_password_change: None,
                    allow_contact_edit: None,
                    groups: Vec::new(),
                    password_state: if password.is_some() {
                        PasswordState::Set
                    } else {
                        PasswordState::Unset
                    },
                },
            );
        }

        fn update_user(&self, user_id: &str, update: impl FnOnce(&mut UserSettings)) {
            update(self.data().users.get_mut(user_id).unwrap());
        }

        fn set_doc(&self, user_id: &str, did: &str, default_zone: Option<&str>) {
            self.data().docs.insert(
                user_id.to_string(),
                UserDoc {
                    did: DID::from_str(did).unwrap(),
                    default_zone: default_zone.map(|zone| DID::from_str(zone).unwrap()),
                },
            );
        }

        fn put_invite(&self, invite: ExclusiveInvite) {
            self.data().invites.insert(invite.invite_id.clone(), invite);
        }

        fn update_invite(&self, invite_id: &str, update: impl FnOnce(&mut ExclusiveInvite)) {
            update(self.data().invites.get_mut(invite_id).unwrap());
        }

        fn allow_app(&self, user_id: &str, app_instance_id: &str) {
            self.data()
                .apps
                .insert((user_id.to_string(), app_instance_id.to_string()));
        }
    }

    #[async_trait]
    impl ZoneDirectory for MemoryDirectory {
        async fn zone_did(&self) -> Result<DID> {
            Ok(self.data().zone_did.clone().unwrap())
        }

        async fn user_settings(&self, user_id: &str) -> Result<Option<UserSettings>> {
            Ok(self.data().users.get(user_id).cloned())
        }

        async fn user_doc(&self, user_id: &str) -> Result<Option<UserDoc>> {
            Ok(self.data().docs.get(user_id).cloned())
        }

        async fn list_users(&self) -> Result<Vec<String>> {
            Ok(self.data().users.keys().cloned().collect())
        }

        async fn exclusive_invite(&self, invite_id: &str) -> Result<Option<ExclusiveInvite>> {
            Ok(self.data().invites.get(invite_id).cloned())
        }

        async fn sso_settings(&self) -> Result<SsoSettings> {
            Ok(self.data().sso.clone())
        }

        async fn app_availability(
            &self,
            user_id: &str,
            app_instance_id: &AppInstanceId,
        ) -> Result<AppAvailabilityDecision> {
            let allowed = self
                .data()
                .apps
                .contains(&(user_id.to_string(), app_instance_id.to_string()));
            Ok(AppAvailabilityDecision {
                allowed,
                app_id: app_instance_id.app_id().clone(),
                app_instance_id: app_instance_id.clone(),
                owner_user_id: app_instance_id.owner_user_id().to_string(),
                availability_match: None,
                reason: if allowed { "test_allow" } else { "test_deny" }.to_string(),
            })
        }
    }

    struct UnreachableCanonical;

    #[async_trait]
    impl FederationPeer for UnreachableCanonical {
        async fn canonical_host(&self, did: &DID) -> Result<String> {
            Err(RPCErrors::ReasonError(format!(
                "no canonical zone for {}",
                did.to_string()
            )))
        }

        async fn exchange(
            &self,
            _canonical_host: &str,
            _use_http: bool,
            _request: ExchangeFederationCodeRequest,
        ) -> Result<ExchangeFederationCodeResult> {
            Err(RPCErrors::ReasonError("unreachable".to_string()))
        }
    }

    /// A canonical zone served in process: the exchange runs that hub's own handler.
    struct InProcessCanonical {
        hosts: HashMap<String, String>,
        hub: VerifyHubServer,
        exchanges: std::sync::Mutex<Vec<(String, bool)>>,
    }

    #[async_trait]
    impl FederationPeer for InProcessCanonical {
        async fn canonical_host(&self, did: &DID) -> Result<String> {
            self.hosts
                .get(&did.to_string())
                .cloned()
                .ok_or_else(|| RPCErrors::ReasonError("unresolvable".to_string()))
        }

        async fn exchange(
            &self,
            canonical_host: &str,
            use_http: bool,
            request: ExchangeFederationCodeRequest,
        ) -> Result<ExchangeFederationCodeResult> {
            self.exchanges
                .lock()
                .unwrap()
                .push((canonical_host.to_string(), use_http));
            self.hub.handle_exchange_federation_code(request).await
        }
    }

    fn hub_with(zone: Arc<MemoryDirectory>, peer: Arc<dyn FederationPeer>) -> VerifyHubServer {
        VerifyHubServer::new(FamilyStore::open_in_memory().unwrap(), zone, peer)
    }

    fn test_hub() -> (VerifyHubServer, Arc<MemoryDirectory>) {
        let zone = MemoryDirectory::new("did:bns:testzone");
        (hub_with(zone.clone(), Arc::new(UnreachableCanonical)), zone)
    }

    fn test_server() -> VerifyHubServer {
        test_hub().0
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

    fn decoded(token: &str) -> RPCSessionToken {
        RPCSessionToken::from_string(token).unwrap()
    }

    fn ceiling_of(token: &str) -> SessionCeiling {
        token_session_ceiling(&decoded(token)).unwrap()
    }

    fn denial_reason(error: &RPCErrors) -> Option<(DenyReason, Option<String>)> {
        AccessDenial::from_rpc_error(error).map(|denial| (denial.reason, denial.detail))
    }

    fn session_key_of(token: &str) -> String {
        let token = decoded(token);
        let claims = validate_verify_hub_token_claims(&token, token_use(&token).unwrap()).unwrap();
        family_key(
            token.sub.as_deref().unwrap(),
            &claims.target,
            get_token_session_id(&token).unwrap(),
        )
    }

    fn unique_user(prefix: &str) -> String {
        format!("{}{}", prefix, rand::thread_rng().gen::<u32>())
    }

    async fn signed_session(
        subject: &str,
        principal_kind: SessionPrincipalKind,
        target: AuthTarget,
    ) -> RPCSessionToken {
        let session = SessionSubject {
            subject: subject.to_string(),
            principal_kind,
            target,
            session_id: rand::thread_rng().gen(),
            claims: FamilyClaims::default(),
        };
        sign_session_token(
            &session,
            TokenUse::Session,
            &rand::thread_rng().gen::<u64>().to_string(),
            buckyos_get_unix_timestamp() + SESSION_TOKEN_EXPIRE_SECONDS,
            None,
            false,
        )
        .await
        .unwrap()
    }

    async fn service_session(service_id: &str, principal_kind: SessionPrincipalKind) -> String {
        signed_session("ood1", principal_kind, system_target(service_id))
            .await
            .to_string()
    }

    async fn cp_caller() -> String {
        service_session("control-panel", SessionPrincipalKind::System).await
    }

    async fn signed_verify_hub_token(mut token: RPCSessionToken) -> String {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let jwt = token
            .generate_jwt(Some(VERIFY_HUB_ISSUER.to_string()), &private_key)
            .unwrap();
        token.token = Some(jwt);
        token.to_string()
    }

    /// A user's Control Panel session from a root-signed LoginAssertion (auth method
    /// `assertion`); the account must exist in the hub's directory.
    async fn assertion_login(hub: &VerifyHubServer, user: &str) -> TokenPair {
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
        hub.handle_login_by_jwt(jwt.as_str(), &system_target("control-panel"))
            .await
            .unwrap()
    }

    fn password_proof(stored: &str, login_nonce: u64) -> String {
        STANDARD.encode(Sha256::digest(format!("{stored}{login_nonce}")))
    }

    fn login_nonce() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        buckyos_get_unix_timestamp() * 1000 + NEXT.fetch_add(1, Ordering::Relaxed) % 1000
    }

    fn invite_record(
        invite_id: &str,
        user_id: &str,
        secret: &str,
        issued_at: u64,
    ) -> ExclusiveInvite {
        ExclusiveInvite {
            invite_id: invite_id.to_string(),
            user_id: user_id.to_string(),
            issuer: InviteIssuer::User {
                user_id: "devtest".to_string(),
            },
            channel: InviteChannel {
                kind: "email".to_string(),
                target_hint: "a***@acme.com".to_string(),
            },
            resource: None,
            resource_state: InviteResourceState::None,
            return_path: "/messagehub".to_string(),
            state: InviteState::Active,
            credential: InviteCredential {
                version: 1,
                digest: invite_secret_digest(secret),
                issued_at,
                expires_at: issued_at + EXCLUSIVE_INVITE_TTL_SECS,
                state: InviteCredentialState::Active,
            },
            resend: None,
            created_at: issued_at,
            updated_at: issued_at,
        }
    }

    async fn invite_login(
        hub: &VerifyHubServer,
        caller: &str,
        invite_id: &str,
        secret: &str,
    ) -> Result<ExclusiveInviteLoginResult> {
        hub.handle_login_by_exclusive_invite(
            Some(caller),
            ExclusiveInviteLoginRequest {
                invite_id: invite_id.to_string(),
                secret: secret.to_string(),
                target: system_target("control-panel"),
            },
        )
        .await
    }

    fn issued_pair(result: Result<ExclusiveInviteLoginResult>) -> TokenPair {
        match result {
            Ok(ExclusiveInviteLoginResult::Issued { token_pair, .. }) => token_pair,
            other => panic!("expected an issued pair, got {other:?}"),
        }
    }

    fn invite_denial(result: Result<ExclusiveInviteLoginResult>) -> (DenyReason, Option<String>) {
        match result {
            Ok(ExclusiveInviteLoginResult::Denied { denial }) => (denial.reason, denial.detail),
            other => panic!("expected a denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn service_login_preserves_owner_subject() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();

        let token_pair = test_server()
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
        assert_eq!(token_auth_method(&session_token), None);
        assert!(!ceiling_of(&token_pair.session_token).restricted);
    }

    #[tokio::test]
    async fn device_service_login_preserves_device_subject() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("ood1", "node-daemon", "ood1", &private_key).unwrap();

        let token_pair = test_server()
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

        let verify_hub_client = VerifyHubClient::new_in_process(Box::new(test_server()));

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
        let login_result = test_server()
            .handle_login_by_jwt(test_jwt.as_str(), &system_target("kernel"))
            .await;

        assert!(
            matches!(login_result, Err(RPCErrors::NoPermission(_))),
            "verify-hub must reject root login JWT"
        );
    }

    #[tokio::test]
    async fn test_verify_hub_rejects_root_password_entrypoints() {
        let handler = test_server();
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
            allow_contact_edit: None,
            groups: Vec::new(),
            password_state: PasswordState::Set,
        };

        assert!(require_active_user_settings(&settings(UserState::Active)).is_ok());
        let mut limited = settings(UserState::Active);
        limited.user_type = UserType::Limited;
        assert!(require_active_user_settings(&limited).is_ok());
        let mut root = settings(UserState::Active);
        root.user_type = UserType::Root;
        assert!(matches!(
            require_active_user_settings(&root),
            Err(RPCErrors::NoPermission(_))
        ));

        for state in [
            UserState::Pending,
            UserState::Suspended("test".to_string()),
            UserState::Deleted,
            UserState::Banned("test".to_string()),
        ] {
            let error = require_active_user_settings(&settings(state)).unwrap_err();
            assert_eq!(
                denial_reason(&error),
                Some((DenyReason::AccountState, None))
            );
        }
    }

    #[tokio::test]
    async fn test_root_token_generation_rejected() {
        let session = SessionSubject {
            subject: "root".to_string(),
            principal_kind: SessionPrincipalKind::User,
            target: system_target("control-panel"),
            session_id: 12345,
            claims: FamilyClaims::default(),
        };
        let now = buckyos_get_unix_timestamp();
        let session_result = sign_session_token(
            &session,
            TokenUse::Session,
            "5678",
            now + SESSION_TOKEN_EXPIRE_SECONDS,
            None,
            false,
        )
        .await;
        assert!(
            matches!(session_result, Err(RPCErrors::NoPermission(_))),
            "root session token generation must fail"
        );

        let token_pair_result = sign_pair(&session, &fresh_stamp(now, None)).await;
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
        let token = signed_verify_hub_token(token).await;

        let verify_result = test_server()
            .handle_verify_token(token.as_str(), None)
            .await;
        assert!(
            matches!(verify_result, Err(RPCErrors::NoPermission(_))),
            "verify-hub issued root session token must not be accepted"
        );
    }

    /// Test dual token login and refresh flow
    /// This test verifies:
    /// 1. First login returns token pair (session_token + refresh_token)
    /// 2. Session token can be verified
    /// 3. Refresh token can be used to get new token pair
    /// 4. Old refresh token is invalidated after use
    /// 5. Expired login JWT is rejected
    #[tokio::test]
    async fn test_login_and_verify() {
        let private_key = setup_test_environment().await;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut rng = rand::thread_rng();
        let login_nonce = rng.gen::<u64>();

        let handler = test_server();

        // Test 1: First login with trusted device JWT
        let test_jwt = create_login_jwt(&private_key, "alice", "kernel", login_nonce, now + 3600);
        let kernel_target = system_target("kernel");

        let login_result = handler
            .handle_login_by_jwt(test_jwt.as_str(), &kernel_target)
            .await;

        assert!(login_result.is_ok(), "First login should succeed");
        let token_pair = login_result.unwrap();

        // Test 2: Verify session token
        let verify_result = handler
            .handle_verify_token(token_pair.session_token.as_str(), None)
            .await;
        assert!(verify_result.is_ok(), "Session token should be valid");

        // Test 3: Refresh using refresh_token
        let refresh_result = handler
            .handle_refresh_token(token_pair.refresh_token.as_str())
            .await;
        assert!(refresh_result.is_ok(), "Refresh should succeed");
        let new_token_pair = refresh_result.unwrap();
        assert_ne!(
            token_pair.session_token, new_token_pair.session_token,
            "New session token should be different"
        );
        assert_ne!(
            token_pair.refresh_token, new_token_pair.refresh_token,
            "New refresh token should be different"
        );

        // Test 4: Verify new session token
        let verify_new_result = handler
            .handle_verify_token(new_token_pair.session_token.as_str(), None)
            .await;
        assert!(
            verify_new_result.is_ok(),
            "New session token should be valid"
        );

        // Test 5: Logout using the current refresh token
        let logout_result = handler
            .handle_logout(new_token_pair.refresh_token.as_str())
            .await;
        assert!(logout_result.is_ok(), "Logout should succeed");
        assert!(logout_result.unwrap(), "Logout should return true");

        // Test 6: Reuse old refresh token from before refresh (should fail)
        let reuse_result = handler
            .handle_refresh_token(token_pair.refresh_token.as_str())
            .await;
        assert!(
            reuse_result.is_err(),
            "Reusing old refresh token should fail"
        );

        // Test 7: Refresh after logout with latest refresh token (should fail)
        let second_refresh_result = handler
            .handle_refresh_token(new_token_pair.refresh_token.as_str())
            .await;
        assert!(
            second_refresh_result.is_err(),
            "Refresh should fail after logout"
        );

        // Test 8: Login with expired JWT (should fail)
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

        // Test 9: Replay attack - reuse same login JWT (should fail)
        let replay_nonce = rng.gen::<u64>();
        let replay_jwt = create_login_jwt(&private_key, "bob", "kernel", replay_nonce, now + 3600);
        let first_use = handler
            .handle_login_by_jwt(replay_jwt.as_str(), &kernel_target)
            .await;
        assert!(first_use.is_ok(), "First use of login JWT should succeed");
        let second_use = handler
            .handle_login_by_jwt(replay_jwt.as_str(), &kernel_target)
            .await;
        assert!(second_use.is_err(), "Replay of login JWT should fail");
    }

    /// Test token pair generation
    #[tokio::test]
    async fn test_generate_token_pair() {
        setup_test_environment().await;

        let target = app_target("test-app@test-owner");
        let session = SessionSubject {
            subject: "test-user".to_string(),
            principal_kind: SessionPrincipalKind::User,
            target: target.clone(),
            session_id: 12345,
            claims: FamilyClaims::account(AuthMethod::Password, UserType::User),
        };
        let now = buckyos_get_unix_timestamp();
        let stamp = fresh_stamp(now, None);
        let token_pair = sign_pair(&session, &stamp).await.unwrap();
        let session_token = decoded(&token_pair.session_token);
        let refresh_token = decoded(&token_pair.refresh_token);

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
        assert_eq!(
            token_auth_method(&session_token),
            Some(AuthMethod::Password)
        );
        assert_eq!(
            token_auth_method(&refresh_token),
            Some(AuthMethod::Password)
        );
        assert_eq!(refresh_token.aud.as_deref(), Some(VERIFY_HUB_UNIQUE_ID));
        let handler = test_server();
        assert!(handler
            .handle_verify_token(&token_pair.session_token, Some(target.clone()))
            .await
            .is_ok());
        assert!(handler
            .handle_verify_token(
                &token_pair.session_token,
                Some(app_target("test-app@other-owner")),
            )
            .await
            .is_err());

        assert_ne!(token_pair.session_token, token_pair.refresh_token);
        assert_eq!(session_token.exp, Some(now + SESSION_TOKEN_EXPIRE_SECONDS));
        assert_eq!(refresh_token.exp, Some(now + REFRESH_TOKEN_EXPIRE_SECONDS));

        // The same record signs the same pair: grace answers need no stored tokens.
        assert_eq!(
            sign_pair(&session, &stamp).await.unwrap().refresh_token,
            token_pair.refresh_token
        );
        // An absolute expiry caps both tokens.
        let capped = fresh_stamp(now, Some(now + 60));
        assert_eq!(capped.session_exp, now + 60);
        assert_eq!(capped.refresh_exp, now + 60);
    }

    #[tokio::test]
    async fn test_generate_sudo_session_token() {
        let session = SessionSubject {
            subject: "alice".to_string(),
            principal_kind: SessionPrincipalKind::User,
            target: system_target("control-panel"),
            session_id: 12345,
            claims: FamilyClaims::account(AuthMethod::Password, UserType::Admin),
        };
        let session_token = sign_session_token(
            &session,
            TokenUse::Session,
            "5678",
            buckyos_get_unix_timestamp() + SUDO_SESSION_TOKEN_EXPIRE_SECONDS,
            Some("system-config".to_string()),
            true,
        )
        .await
        .unwrap();

        assert!(session_token.sudo);
        assert_eq!(session_token.appid.as_deref(), Some("control-panel"));
        assert_eq!(session_token.sub.as_deref(), Some("alice"));
        assert_eq!(session_token.aud.as_deref(), Some("system-config"));

        let parsed = decoded(session_token.to_string().as_str());
        assert!(parsed.sudo);
        assert_eq!(parsed.aud.as_deref(), Some("system-config"));
        assert!(
            sign_session_token(&session, TokenUse::Refresh, "5679", 1, None, true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn concurrent_refresh_with_one_token_shares_one_pair() {
        // User principals read their settings on refresh; a System principal family needs no
        // directory and exercises the same rotation path.
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();
        let handler = test_server();
        let service_pair = handler
            .handle_login_by_jwt(login_jwt.as_str(), &system_target("control-panel"))
            .await
            .unwrap();
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
    async fn app_service_subject_must_match_target_owner() {
        let private_key = setup_test_environment().await;
        let target = AuthTarget::app("demo@alice".parse().unwrap());
        let (jwt, _) =
            generate_service_login_assertion("ood1", "demo", "ood1", &private_key).unwrap();
        let error = test_server()
            .handle_login_by_jwt(&jwt, &target)
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((
                DenyReason::EntryUnavailable,
                Some("app_subject_owner_mismatch".into())
            ))
        );
        let error = check_app_principal_target("bob", &target)
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((
                DenyReason::EntryUnavailable,
                Some("app_subject_owner_mismatch".into())
            ))
        );
    }

    #[tokio::test]
    async fn used_and_expired_login_assertions_require_reauthorization() {
        let private_key = setup_test_environment().await;
        let handler = test_server();
        let target = system_target("control-panel");
        let (jwt, mut assertion) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();
        handler.handle_login_by_jwt(&jwt, &target).await.unwrap();
        let error = handler
            .handle_login_by_jwt(&jwt, &target)
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((
                DenyReason::CredentialState,
                Some("login_assertion_used".into())
            ))
        );
        assertion.exp = Some(buckyos_get_unix_timestamp() - 120);
        let expired = assertion
            .generate_jwt(Some("ood1".into()), &private_key)
            .unwrap();
        let error = handler
            .handle_login_by_jwt(&expired, &target)
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((
                DenyReason::CredentialState,
                Some("login_assertion_expired".into())
            ))
        );
    }

    #[tokio::test]
    async fn invalid_and_expired_refresh_have_structured_denials() {
        let private_key = setup_test_environment().await;
        let handler = test_server();
        let malformed = handler
            .handle_refresh_token("invalid.jwt")
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&malformed),
            Some((
                DenyReason::CredentialState,
                Some("refresh_token_invalid".into())
            ))
        );
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();
        let pair = handler
            .handle_login_by_jwt(&login_jwt, &system_target("control-panel"))
            .await
            .unwrap();
        let mut expired = decoded(&pair.refresh_token);
        expired.exp = Some(buckyos_get_unix_timestamp() - 120);
        let expired = signed_verify_hub_token(expired).await;
        let error = handler.handle_refresh_token(&expired).await.unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((
                DenyReason::CredentialState,
                Some("refresh_token_expired".into())
            ))
        );
        assert!(handler
            .handle_refresh_token(&pair.refresh_token)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn stale_refresh_outside_grace_revokes_family() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();
        let handler = test_server();
        let pair = handler
            .handle_login_by_jwt(login_jwt.as_str(), &system_target("control-panel"))
            .await
            .unwrap();
        let rotated = handler
            .handle_refresh_token(pair.refresh_token.as_str())
            .await
            .unwrap();
        let key = session_key_of(&pair.refresh_token);
        handler
            .families()
            .backdate_rotation(&key, REFRESH_GRACE_SECONDS + 1)
            .unwrap();

        let error = handler
            .handle_refresh_token(pair.refresh_token.as_str())
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((
                DenyReason::CredentialState,
                Some("refresh_token_reused".into())
            ))
        );
        assert!(handler.families().get(&key).unwrap().is_none());
        let error = handler
            .handle_refresh_token(rotated.refresh_token.as_str())
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((
                DenyReason::CredentialState,
                Some("refresh_session_invalidated".into())
            ))
        );
    }

    #[tokio::test]
    async fn logout_cannot_be_undone_by_grace_or_inflight_refresh() {
        let private_key = setup_test_environment().await;
        let (login_jwt, _) =
            generate_service_login_assertion("alice", "control-panel", "ood1", &private_key)
                .unwrap();
        let handler = test_server();
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
        let rotated_jti = decoded(&rotated.refresh_token).jti.unwrap();
        let now = buckyos_get_unix_timestamp();
        assert!(matches!(
            handler
                .families()
                .rotate(
                    &key,
                    &rotated_jti,
                    &fresh_stamp(now, None),
                    &FamilyClaims::default(),
                    now
                )
                .unwrap(),
            RefreshCommit::LoggedOut
        ));
        assert!(handler.families().get(&key).unwrap().is_none());
    }

    #[tokio::test]
    async fn restart_keeps_families_refreshable_and_grace_idempotent() {
        setup_test_environment().await;
        let zone = MemoryDirectory::new("did:bns:testzone");
        let user = unique_user("restart");
        zone.add_user(&user, UserType::Limited, Some("pw"));
        let dir = std::env::temp_dir().join(format!(
            "verify-hub-restart-{}-{}",
            std::process::id(),
            rand::thread_rng().gen::<u64>()
        ));
        let path = dir.join("sessions.db");
        let open = || {
            VerifyHubServer::new(
                FamilyStore::open(&path).unwrap(),
                zone.clone(),
                Arc::new(UnreachableCanonical),
            )
        };

        let first = open();
        let login = assertion_login(&first, &user).await;
        let rotated = first
            .handle_refresh_token(&login.refresh_token)
            .await
            .unwrap();
        drop(first);

        let second = open();
        let grace = second
            .handle_refresh_token(&login.refresh_token)
            .await
            .unwrap();
        for (rebuilt, original) in [
            (&grace.session_token, &rotated.session_token),
            (&grace.refresh_token, &rotated.refresh_token),
        ] {
            let (rebuilt, original) = (decoded(rebuilt), decoded(original));
            assert_eq!(rebuilt.jti, original.jti);
            assert_eq!(rebuilt.exp, original.exp);
            assert_eq!(rebuilt.extra, original.extra);
        }
        assert_eq!(grace.refresh_token, rotated.refresh_token);
        assert_eq!(
            ceiling_of(&grace.session_token),
            SessionCeiling::for_user_type(UserType::Limited)
        );
        let next = second
            .handle_refresh_token(&rotated.refresh_token)
            .await
            .unwrap();
        assert_ne!(next.refresh_token, rotated.refresh_token);
        drop(second);

        let third = open();
        assert!(third
            .handle_refresh_token(&next.refresh_token)
            .await
            .is_ok());
        drop(third);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn refresh_narrows_the_ceiling_and_never_widens_it() {
        setup_test_environment().await;
        let (hub, zone) = test_hub();
        let limited = SessionCeiling::for_user_type(UserType::Limited);

        let dora = unique_user("dora");
        zone.add_user(&dora, UserType::User, Some("pw"));
        let pair = assertion_login(&hub, &dora).await;
        assert!(!ceiling_of(&pair.session_token).restricted);
        assert_eq!(
            token_auth_method(&decoded(&pair.session_token)),
            Some(AuthMethod::Assertion)
        );

        zone.update_user(&dora, |user| user.user_type = UserType::Limited);
        let narrowed = hub.handle_refresh_token(&pair.refresh_token).await.unwrap();
        assert_eq!(ceiling_of(&narrowed.session_token), limited);
        assert_eq!(ceiling_of(&narrowed.refresh_token), limited);
        let key = session_key_of(&narrowed.refresh_token);
        assert_eq!(
            hub.families()
                .get(&key)
                .unwrap()
                .unwrap()
                .session
                .claims
                .ceiling(),
            limited
        );

        zone.update_user(&dora, |user| user.user_type = UserType::User);
        let upgraded = hub
            .handle_refresh_token(&narrowed.refresh_token)
            .await
            .unwrap();
        assert_eq!(ceiling_of(&upgraded.session_token), limited);

        let evan = unique_user("evan");
        zone.add_user(&evan, UserType::Limited, Some("pw"));
        let pair = assertion_login(&hub, &evan).await;
        assert_eq!(ceiling_of(&pair.session_token), limited);
        zone.update_user(&evan, |user| user.user_type = UserType::Admin);
        let refreshed = hub.handle_refresh_token(&pair.refresh_token).await.unwrap();
        assert_eq!(ceiling_of(&refreshed.session_token), limited);
    }

    #[tokio::test]
    async fn refresh_and_grace_recheck_the_account() {
        setup_test_environment().await;
        let (hub, zone) = test_hub();
        let user = unique_user("gina");
        zone.add_user(&user, UserType::User, Some("pw"));
        let pair = assertion_login(&hub, &user).await;
        let rotated = hub.handle_refresh_token(&pair.refresh_token).await.unwrap();
        let key = session_key_of(&rotated.refresh_token);

        zone.update_user(&user, |user| {
            user.state = UserState::Suspended("test".into())
        });
        // The grace branch rechecks before answering.
        let error = hub
            .handle_refresh_token(&pair.refresh_token)
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((DenyReason::AccountState, None))
        );
        assert!(hub.families().get(&key).unwrap().is_none());

        // Reactivation does not bring the revoked family back.
        zone.update_user(&user, |user| user.state = UserState::Active);
        assert!(hub
            .handle_refresh_token(&rotated.refresh_token)
            .await
            .is_err());

        // A deleted account is refused on login too.
        zone.update_user(&user, |user| user.state = UserState::Deleted);
        let private_key = setup_test_environment().await;
        let assertion = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            jti: Some("deleted-login".to_string()),
            appid: Some("control-panel".to_string()),
            aud: None,
            sub: Some(user.clone()),
            token: None,
            iss: Some("root".to_string()),
            exp: Some(buckyos_get_unix_timestamp() + 600),
            sudo: false,
            extra: HashMap::new(),
        };
        let jwt = assertion
            .generate_jwt(Some("root".to_string()), &private_key)
            .unwrap();
        let error = hub
            .handle_login_by_jwt(&jwt, &system_target("control-panel"))
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((DenyReason::AccountState, None))
        );
    }

    #[tokio::test]
    async fn password_login_checks_password_state_and_restricted_sudo() {
        setup_test_environment().await;
        let (hub, zone) = test_hub();
        let target = system_target("control-panel");

        let ivy = unique_user("ivy");
        zone.add_user(&ivy, UserType::Limited, Some("stored-ivy"));
        let nonce = login_nonce();
        let response = hub
            .handle_login_by_password(&ivy, &password_proof("stored-ivy", nonce), &target, nonce)
            .await
            .unwrap();
        assert_eq!(
            token_auth_method(&decoded(&response.session_token)),
            Some(AuthMethod::Password)
        );
        assert_eq!(
            ceiling_of(&response.session_token),
            SessionCeiling::for_user_type(UserType::Limited)
        );

        let nonce = login_nonce();
        let error = hub
            .handle_sudo_by_password(
                &ivy,
                &password_proof("stored-ivy", nonce),
                &target,
                Some("system-config".to_string()),
                nonce,
            )
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error).map(|(reason, _)| reason),
            Some(DenyReason::SessionKindNotAllowed)
        );

        let hal = unique_user("hal");
        zone.add_user(&hal, UserType::User, Some("stored-hal"));
        let nonce = login_nonce();
        let sudo = hub
            .handle_sudo_by_password(
                &hal,
                &password_proof("stored-hal", nonce),
                &target,
                Some("system-config".to_string()),
                nonce,
            )
            .await
            .unwrap();
        assert!(decoded(&sudo.session_token).sudo);
        let nonce = login_nonce();
        assert!(matches!(
            hub.handle_login_by_password(&hal, &password_proof("wrong", nonce), &target, nonce)
                .await,
            Err(RPCErrors::InvalidPassword)
        ));
        zone.update_user(&hal, |user| {
            user.state = UserState::Suspended("test".into())
        });
        let nonce = login_nonce();
        let error = hub
            .handle_login_by_password(&hal, &password_proof("stored-hal", nonce), &target, nonce)
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((DenyReason::AccountState, None))
        );

        // An account without a password is refused before any digest comparison, even when a
        // stale stored value would match.
        let fay = unique_user("fay");
        zone.add_user(&fay, UserType::Limited, None);
        let password_unset = Some((
            DenyReason::CredentialState,
            Some("password_unset".to_string()),
        ));
        let nonce = login_nonce();
        let error = hub
            .handle_login_by_password(&fay, &password_proof("", nonce), &target, nonce)
            .await
            .unwrap_err();
        assert_eq!(denial_reason(&error), password_unset);
        zone.update_user(&fay, |user| user.password = "leftover".to_string());
        let nonce = login_nonce();
        let error = hub
            .handle_sudo_by_password(
                &fay,
                &password_proof("leftover", nonce),
                &target,
                None,
                nonce,
            )
            .await
            .unwrap_err();
        assert_eq!(denial_reason(&error), password_unset);
    }

    #[test]
    fn exclusive_invite_decision_follows_the_contract_order() {
        let now = 1_800_000_000;
        let invite = invite_record("ei-1", "acme", "s3cret", now);
        let mut account = UserSettings {
            user_id: "acme".to_string(),
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
        let digest = invite_secret_digest("s3cret");
        let decide = |invite: Option<&ExclusiveInvite>, account: Option<&UserSettings>| {
            exclusive_invite_decision(invite, account, &digest, now)
                .map(|_| ())
                .map_err(|denial| (denial.reason, denial.detail.unwrap_or_default()))
        };
        assert_eq!(decide(Some(&invite), Some(&account)), Ok(()));
        assert_eq!(
            decide(None, Some(&account)),
            Err((DenyReason::SourceRevoked, "revoked".into()))
        );
        let mut revoked = invite.clone();
        revoked.state = InviteState::Revoked;
        assert_eq!(
            decide(Some(&revoked), Some(&account)),
            Err((DenyReason::SourceRevoked, "revoked".into()))
        );
        assert_eq!(
            decide(Some(&invite), None),
            Err((DenyReason::AccountState, "account_inactive".into()))
        );
        account.user_type = UserType::User;
        assert_eq!(
            decide(Some(&invite), Some(&account)),
            Err((DenyReason::AccountState, "not_limited".into()))
        );
        account.user_type = UserType::Limited;
        account.password_state = PasswordState::Set;
        // Account state is decided before the password state.
        account.state = UserState::Suspended("test".into());
        assert_eq!(
            decide(Some(&invite), Some(&account)),
            Err((DenyReason::AccountState, "account_inactive".into()))
        );
        account.state = UserState::Active;
        assert_eq!(
            decide(Some(&invite), Some(&account)),
            Err((DenyReason::CredentialState, "password_required".into()))
        );
        // An account upgraded after setting its password still gets the password answer.
        account.user_type = UserType::User;
        assert_eq!(
            decide(Some(&invite), Some(&account)),
            Err((DenyReason::CredentialState, "password_required".into()))
        );
    }

    #[tokio::test]
    async fn exclusive_invite_login_issues_restricted_sessions_for_control_panel_only() {
        setup_test_environment().await;
        let (hub, zone) = test_hub();
        let caller = cp_caller().await;
        let now = buckyos_get_unix_timestamp();
        let acme = unique_user("acme");
        zone.add_user(&acme, UserType::Limited, None);
        zone.put_invite(invite_record("ei-ok", &acme, "s3cret", now));

        let result = invite_login(&hub, &caller, "ei-ok", "s3cret").await;
        let Ok(ExclusiveInviteLoginResult::Issued {
            user_id,
            token_pair,
        }) = result
        else {
            panic!("expected an issued pair, got {result:?}");
        };
        assert_eq!(user_id, acme);
        let session = decoded(&token_pair.session_token);
        assert_eq!(session.sub.as_deref(), Some(acme.as_str()));
        assert_eq!(
            token_auth_method(&session),
            Some(AuthMethod::ExclusiveInvite)
        );
        assert_eq!(
            ceiling_of(&token_pair.session_token),
            SessionCeiling::for_user_type(UserType::Limited)
        );
        let family = hub
            .families()
            .get(&session_key_of(&token_pair.refresh_token))
            .unwrap()
            .unwrap();
        assert_eq!(
            family.source,
            Some(SessionSource::ExclusiveInvite {
                invite_id: "ei-ok".into(),
                version: 1
            })
        );
        // The credential stays usable within its lifetime.
        issued_pair(invite_login(&hub, &caller, "ei-ok", "s3cret").await);

        assert_eq!(
            invite_denial(invite_login(&hub, &caller, "ei-missing", "s3cret").await),
            (DenyReason::SourceRevoked, Some("revoked".into()))
        );
        assert_eq!(
            invite_denial(invite_login(&hub, &caller, "../settings", "s3cret").await),
            (DenyReason::SourceRevoked, Some("revoked".into()))
        );
        assert_eq!(
            invite_denial(invite_login(&hub, &caller, "ei-ok", "other").await),
            (DenyReason::CredentialState, Some("link_replaced".into()))
        );
        zone.put_invite(invite_record(
            "ei-old",
            &acme,
            "s3cret",
            now - EXCLUSIVE_INVITE_TTL_SECS - 10,
        ));
        assert_eq!(
            invite_denial(invite_login(&hub, &caller, "ei-old", "s3cret").await),
            (DenyReason::CredentialState, Some("expired".into()))
        );

        let typed = unique_user("typed");
        zone.add_user(&typed, UserType::User, None);
        zone.put_invite(invite_record("ei-user", &typed, "s3cret", now));
        assert_eq!(
            invite_denial(invite_login(&hub, &caller, "ei-user", "s3cret").await),
            (DenyReason::AccountState, Some("not_limited".into()))
        );

        zone.update_user(&acme, |user| {
            user.password_state = PasswordState::Set;
            user.password = "pw".into();
        });
        assert_eq!(
            invite_denial(invite_login(&hub, &caller, "ei-ok", "s3cret").await),
            (
                DenyReason::CredentialState,
                Some("password_required".into())
            )
        );
        zone.update_user(&acme, |user| {
            user.state = UserState::Suspended("test".into())
        });
        assert_eq!(
            invite_denial(invite_login(&hub, &caller, "ei-ok", "s3cret").await),
            (DenyReason::AccountState, Some("account_inactive".into()))
        );

        for caller in [
            None,
            Some(token_pair.session_token.clone()),
            Some(service_session("task-manager", SessionPrincipalKind::System).await),
        ] {
            let result = hub
                .handle_login_by_exclusive_invite(
                    caller.as_deref(),
                    ExclusiveInviteLoginRequest {
                        invite_id: "ei-ok".into(),
                        secret: "s3cret".into(),
                        target: system_target("control-panel"),
                    },
                )
                .await;
            assert!(
                matches!(result, Err(RPCErrors::NoPermission(_))),
                "{result:?}"
            );
        }
        let result = hub
            .handle_login_by_exclusive_invite(
                Some(&caller),
                ExclusiveInviteLoginRequest {
                    invite_id: "ei-ok".into(),
                    secret: "s3cret".into(),
                    target: app_target("notes@bob"),
                },
            )
            .await;
        assert!(matches!(result, Err(RPCErrors::ParseRequestError(_))));
    }

    #[tokio::test]
    async fn exclusive_invite_family_ends_when_its_invite_is_revoked() {
        setup_test_environment().await;
        let (hub, zone) = test_hub();
        let caller = cp_caller().await;
        let acme = unique_user("acme");
        zone.add_user(&acme, UserType::Limited, None);
        zone.put_invite(invite_record(
            "ei-r",
            &acme,
            "s3cret",
            buckyos_get_unix_timestamp(),
        ));
        let pair = issued_pair(invite_login(&hub, &caller, "ei-r", "s3cret").await);
        let pair = hub.handle_refresh_token(&pair.refresh_token).await.unwrap();

        // A resend (new credential version) does not end existing sessions.
        zone.update_invite("ei-r", |invite| {
            invite.credential.version = 2;
            invite.credential.digest = invite_secret_digest("new");
        });
        let pair = hub.handle_refresh_token(&pair.refresh_token).await.unwrap();

        zone.update_invite("ei-r", |invite| invite.state = InviteState::Revoked);
        let error = hub
            .handle_refresh_token(&pair.refresh_token)
            .await
            .unwrap_err();
        assert_eq!(
            denial_reason(&error),
            Some((DenyReason::SourceRevoked, Some("invite_revoked".into())))
        );
        assert!(hub
            .families()
            .get(&session_key_of(&pair.refresh_token))
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn revoke_families_spares_the_current_session() {
        setup_test_environment().await;
        let (hub, zone) = test_hub();
        let caller = cp_caller().await;
        let now = buckyos_get_unix_timestamp();
        let acme = unique_user("acme");
        zone.add_user(&acme, UserType::Limited, None);
        zone.put_invite(invite_record("ei-a", &acme, "a", now));
        zone.put_invite(invite_record("ei-b", &acme, "b", now));
        let current = issued_pair(invite_login(&hub, &caller, "ei-a", "a").await);
        let other = issued_pair(invite_login(&hub, &caller, "ei-a", "a").await);
        let third = issued_pair(invite_login(&hub, &caller, "ei-b", "b").await);

        // First password: the account now signs in by password too.
        zone.update_user(&acme, |user| {
            user.password_state = PasswordState::Set;
            user.password = "stored".into();
        });
        let nonce = login_nonce();
        let by_password = hub
            .handle_login_by_password(
                &acme,
                &password_proof("stored", nonce),
                &system_target("control-panel"),
                nonce,
            )
            .await
            .unwrap();

        let request = RevokeFamiliesRequest {
            subject: acme.clone(),
            auth_method: Some(AuthMethod::ExclusiveInvite),
            source_invite_id: None,
            except_session_token: Some(current.session_token.clone()),
        };
        for denied_caller in [None, Some(current.session_token.as_str())] {
            assert!(matches!(
                hub.handle_revoke_families(denied_caller, request.clone())
                    .await,
                Err(RPCErrors::NoPermission(_))
            ));
        }
        assert_eq!(
            hub.handle_revoke_families(Some(&caller), request)
                .await
                .unwrap(),
            RevokeFamiliesResult { revoked: 2 }
        );
        let current = hub
            .handle_refresh_token(&current.refresh_token)
            .await
            .unwrap();
        assert!(hub
            .handle_refresh_token(&other.refresh_token)
            .await
            .is_err());
        assert!(hub
            .handle_refresh_token(&third.refresh_token)
            .await
            .is_err());
        let by_password = hub
            .handle_refresh_token(&by_password.refresh_token)
            .await
            .unwrap();

        let by_invite = RevokeFamiliesRequest {
            subject: acme.clone(),
            source_invite_id: Some("ei-a".into()),
            ..Default::default()
        };
        assert_eq!(
            hub.handle_revoke_families(Some(&caller), by_invite)
                .await
                .unwrap(),
            RevokeFamiliesResult { revoked: 1 }
        );
        assert!(hub
            .handle_refresh_token(&current.refresh_token)
            .await
            .is_err());
        assert!(hub
            .handle_refresh_token(&by_password.refresh_token)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn delegate_accepts_only_the_control_panel_service_as_caller() {
        let private_key = setup_test_environment().await;
        let (handler, zone) = test_hub();
        let user = unique_user("caller");
        zone.add_user(&user, UserType::User, Some("pw"));
        let desktop = assertion_login(&handler, &user).await;
        let notes = app_target("notes@bob");
        let (device_assertion, _) =
            generate_service_login_assertion("ood1", "control-panel", "ood1", &private_key)
                .unwrap();
        let mut sudo_service = decoded(&cp_caller().await);
        sudo_service.sudo = true;
        let sudo_service = signed_verify_hub_token(sudo_service).await;
        let user_app_session = signed_session("alice", SessionPrincipalKind::User, notes.clone())
            .await
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
        let (handler, zone) = test_hub();
        let user = unique_user("source");
        zone.add_user(&user, UserType::User, Some("pw"));
        let desktop = assertion_login(&handler, &user).await;
        let caller = cp_caller().await;
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

        let mut expired = decoded(&desktop.session_token);
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

        let app_session = signed_session(
            &user,
            SessionPrincipalKind::User,
            app_target("control-panel@alice"),
        )
        .await
        .to_string();
        let mut sudo = decoded(&desktop.session_token);
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
    async fn delegation_keeps_the_source_ceiling_and_restricted_children_die_with_it() {
        setup_test_environment().await;
        let (hub, zone) = test_hub();
        let caller = cp_caller().await;
        let notes = app_target("notes@bob");
        let delegate = |source: String, target: AuthTarget| {
            let hub = hub.clone();
            let caller = caller.clone();
            async move {
                hub.handle_delegate_user_session(Some(caller.as_str()), source.as_str(), &target)
                    .await
                    .unwrap()
            }
        };
        let issued = |result: DelegateUserSessionResult| match result {
            DelegateUserSessionResult::Issued(pair) => pair,
            other => panic!("expected an issued pair, got {other:?}"),
        };

        // Unrestricted: independent of the source, which stays live across its refresh.
        let user = unique_user("ulla");
        zone.add_user(&user, UserType::User, Some("pw"));
        zone.allow_app(&user, "notes@bob");
        let desktop = assertion_login(&hub, &user).await;
        let refreshed = hub
            .handle_refresh_token(&desktop.refresh_token)
            .await
            .unwrap();
        let app = issued(delegate(desktop.session_token.clone(), notes.clone()).await);
        let child = hub
            .families()
            .get(&session_key_of(&app.refresh_token))
            .unwrap()
            .unwrap();
        assert_eq!(child.parent_key, None);
        assert!(!child.session.claims.restricted);
        assert_eq!(
            token_auth_method(&decoded(&app.session_token)),
            Some(AuthMethod::Assertion)
        );
        assert!(matches!(
            delegate(desktop.session_token.clone(), app_target("mail@bob")).await,
            DelegateUserSessionResult::AccessDenied
        ));
        assert!(hub.handle_logout(&refreshed.refresh_token).await.unwrap());
        assert!(hub.handle_refresh_token(&app.refresh_token).await.is_ok());
        assert!(matches!(
            delegate(desktop.session_token.clone(), notes.clone()).await,
            DelegateUserSessionResult::SourceInactive
        ));

        // Restricted: the child carries the source ceiling and depends on the source family.
        let acme = unique_user("acme");
        zone.add_user(&acme, UserType::Limited, None);
        zone.allow_app(&acme, "notes@bob");
        zone.put_invite(invite_record(
            "ei-d",
            &acme,
            "s3cret",
            buckyos_get_unix_timestamp(),
        ));
        let desktop = issued_pair(invite_login(&hub, &caller, "ei-d", "s3cret").await);
        let source_key = session_key_of(&desktop.refresh_token);
        let app = issued(delegate(desktop.session_token.clone(), notes.clone()).await);
        let app_key = session_key_of(&app.refresh_token);
        let child = hub.families().get(&app_key).unwrap().unwrap();
        assert_eq!(child.parent_key.as_deref(), Some(source_key.as_str()));
        assert_eq!(
            child.source,
            Some(SessionSource::ExclusiveInvite {
                invite_id: "ei-d".into(),
                version: 1
            })
        );
        assert_eq!(
            ceiling_of(&app.session_token),
            SessionCeiling::for_user_type(UserType::Limited)
        );
        assert_eq!(
            token_auth_method(&decoded(&app.session_token)),
            Some(AuthMethod::ExclusiveInvite)
        );
        let app = hub.handle_refresh_token(&app.refresh_token).await.unwrap();
        assert!(hub.handle_logout(&desktop.refresh_token).await.unwrap());
        assert!(hub.families().get(&app_key).unwrap().is_none());
        assert!(hub.handle_refresh_token(&app.refresh_token).await.is_err());

        // A downgraded source delegates a restricted, dependent child.
        let vic = unique_user("vic");
        zone.add_user(&vic, UserType::User, Some("pw"));
        zone.allow_app(&vic, "notes@bob");
        let desktop = assertion_login(&hub, &vic).await;
        zone.update_user(&vic, |user| user.user_type = UserType::Limited);
        let app = issued(delegate(desktop.session_token.clone(), notes.clone()).await);
        let child = hub
            .families()
            .get(&session_key_of(&app.refresh_token))
            .unwrap()
            .unwrap();
        assert!(child.session.claims.restricted);
        assert_eq!(
            child.parent_key,
            Some(session_key_of(&desktop.refresh_token))
        );

        // A restricted child whose parent row is gone is refused on refresh.
        let mut orphan = child.clone();
        orphan.session.session_id = rand::thread_rng().gen();
        orphan.parent_key = Some("missing-parent".into());
        let orphan_pair = sign_pair(&orphan.session, &orphan.stamp).await.unwrap();
        hub.families().insert(&orphan).unwrap();
        assert!(hub
            .handle_refresh_token(&orphan_pair.refresh_token)
            .await
            .is_err());
        assert!(hub.families().get(&orphan.key()).unwrap().is_none());
    }

    // ─── DID federation ───────────────────────────────────────────────────

    const REDIRECT_URI: &str = "https://b.example/sso/did_callback";

    struct Federation {
        canonical: VerifyHubServer,
        canonical_zone: Arc<MemoryDirectory>,
        target: VerifyHubServer,
        target_zone: Arc<MemoryDirectory>,
        peer: Arc<InProcessCanonical>,
        caller: String,
    }

    async fn federation() -> Federation {
        setup_test_environment().await;
        let canonical_zone = MemoryDirectory::new("did:bns:zonea");
        for (user, user_type, did) in [
            ("alice", UserType::User, "did:bns:alice"),
            ("carol", UserType::Limited, "did:bns:carol"),
        ] {
            canonical_zone.add_user(user, user_type, Some("pw"));
            canonical_zone.set_doc(user, did, Some("did:bns:zonea"));
        }
        let canonical = hub_with(canonical_zone.clone(), Arc::new(UnreachableCanonical));
        let peer = Arc::new(InProcessCanonical {
            hosts: HashMap::from([("did:bns:alice".to_string(), "a.example".to_string())]),
            hub: canonical.clone(),
            exchanges: std::sync::Mutex::new(Vec::new()),
        });
        let target_zone = MemoryDirectory::new("did:bns:zoneb");
        let target = hub_with(target_zone.clone(), peer.clone());
        Federation {
            canonical,
            canonical_zone,
            target,
            target_zone,
            peer,
            caller: cp_caller().await,
        }
    }

    impl Federation {
        async fn issue(
            &self,
            user_id: &str,
            did: &str,
            client_zone: &str,
            verifier: &str,
        ) -> Result<IssueFederationCodeResult> {
            self.canonical
                .handle_issue_federation_code(
                    Some(&self.caller),
                    IssueFederationCodeRequest {
                        user_id: user_id.to_string(),
                        did: did.to_string(),
                        client_zone: client_zone.to_string(),
                        redirect_uri: REDIRECT_URI.to_string(),
                        nonce: "n-1".to_string(),
                        code_challenge: pkce_s256(verifier),
                        profile: FederationProfile {
                            show_name: "Alice".to_string(),
                            avatar: None,
                        },
                    },
                )
                .await
        }

        async fn code(&self, verifier: &str) -> String {
            self.issue("alice", "did:bns:alice", "did:bns:zoneb", verifier)
                .await
                .unwrap()
                .code
        }

        async fn login(
            &self,
            did: &str,
            code: &str,
            verifier: &str,
        ) -> LoginByRemoteVerifyHubResult {
            self.target
                .handle_login_by_remote_verify_hub(
                    Some(&self.caller),
                    LoginByRemoteVerifyHubRequest {
                        did: did.to_string(),
                        code: code.to_string(),
                        code_verifier: verifier.to_string(),
                        nonce: "n-1".to_string(),
                        redirect_uri: REDIRECT_URI.to_string(),
                        target: system_target("control-panel"),
                    },
                )
                .await
                .unwrap()
        }
    }

    fn remote_denial(result: LoginByRemoteVerifyHubResult) -> (DenyReason, Option<String>) {
        match result {
            LoginByRemoteVerifyHubResult::Denied { denial } => (denial.reason, denial.detail),
            other => panic!("expected a denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn did_login_issues_an_external_session_through_a_single_use_code() {
        let fed = federation().await;
        let issued = fed
            .issue("alice", "did:bns:alice", "did:bns:zoneb", "v-1")
            .await
            .unwrap();
        assert!(issued.code.starts_with("fc-"));
        assert_eq!(issued.expires_in, FEDERATION_CODE_TTL_SECONDS);

        let LoginByRemoteVerifyHubResult::Issued {
            user_id,
            token_pair,
            profile,
        } = fed.login("did:bns:alice", &issued.code, "v-1").await
        else {
            panic!("expected an issued External session");
        };
        assert_eq!(user_id, None);
        assert_eq!(profile.show_name, "Alice");
        assert_eq!(
            fed.peer.exchanges.lock().unwrap().as_slice(),
            &[("a.example".to_string(), false)]
        );
        let session = decoded(&token_pair.session_token);
        assert_eq!(session.sub.as_deref(), Some("did:bns:alice"));
        assert!(token_is_external(&session));
        assert_eq!(
            token_principal_kind(&session).unwrap(),
            SessionPrincipalKind::User
        );
        assert_eq!(token_auth_method(&session), Some(AuthMethod::DidFederation));
        assert_eq!(
            ceiling_of(&token_pair.session_token),
            SessionCeiling::external()
        );
        let family = fed
            .target
            .families()
            .get(&session_key_of(&token_pair.refresh_token))
            .unwrap()
            .unwrap();
        assert_eq!(
            family.source,
            Some(SessionSource::DidFederation {
                canonical_host: "a.example".into()
            })
        );
        let absolute_exp = family.absolute_exp.unwrap();
        assert!(absolute_exp >= buckyos_get_unix_timestamp() + FEDERATION_SESSION_MAX_SECONDS - 5);
        assert!(family.stamp.refresh_exp <= absolute_exp);

        // External families refresh without an account and stay External.
        let refreshed = fed
            .target
            .handle_refresh_token(&token_pair.refresh_token)
            .await
            .unwrap();
        assert!(token_is_external(&decoded(&refreshed.session_token)));
        assert_eq!(
            ceiling_of(&refreshed.session_token),
            SessionCeiling::external()
        );

        let invalid_code = (
            DenyReason::CredentialState,
            Some("invalid_code".to_string()),
        );
        // Replay: the code was consumed.
        assert_eq!(
            remote_denial(fed.login("did:bns:alice", &issued.code, "v-1").await),
            invalid_code
        );
        // Wrong verifier, and the failed attempt burns the code.
        let code = fed.code("v-2").await;
        assert_eq!(
            remote_denial(fed.login("did:bns:alice", &code, "wrong").await),
            invalid_code
        );
        assert_eq!(
            remote_denial(fed.login("did:bns:alice", &code, "v-2").await),
            invalid_code
        );
        // A code issued for another target zone.
        let code = fed
            .issue("alice", "did:bns:alice", "did:bns:zonec", "v-3")
            .await
            .unwrap()
            .code;
        assert_eq!(
            remote_denial(fed.login("did:bns:alice", &code, "v-3").await),
            invalid_code
        );
        // The assertion must answer this login's nonce.
        let code = fed.code("v-4").await;
        let result = fed
            .target
            .handle_login_by_remote_verify_hub(
                Some(&fed.caller),
                LoginByRemoteVerifyHubRequest {
                    did: "did:bns:alice".into(),
                    code,
                    code_verifier: "v-4".into(),
                    nonce: "other".into(),
                    redirect_uri: REDIRECT_URI.into(),
                    target: system_target("control-panel"),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            remote_denial(result),
            (
                DenyReason::CredentialState,
                Some("assertion_mismatch".into())
            )
        );

        // Development HTTP exchange only when the Zone setting allows it.
        fed.target_zone.data().sso.dev_federation_http = true;
        let code = fed.code("v-5").await;
        assert!(matches!(
            fed.login("did:bns:alice", &code, "v-5").await,
            LoginByRemoteVerifyHubResult::Issued { .. }
        ));
        assert_eq!(
            fed.peer.exchanges.lock().unwrap().last(),
            Some(&("a.example".to_string(), true))
        );

        assert!(matches!(
            fed.login("did:bns:nobody", "fc-x", "v").await,
            LoginByRemoteVerifyHubResult::CanonicalUnavailable
        ));
        fed.target_zone.data().sso.did_login_enabled = false;
        assert_eq!(
            remote_denial(fed.login("did:bns:alice", "fc-x", "v").await).0,
            DenyReason::ZoneMethodDisabled
        );

        let request = LoginByRemoteVerifyHubRequest {
            did: "did:bns:alice".into(),
            code: "fc-x".into(),
            code_verifier: "v".into(),
            nonce: "n-1".into(),
            redirect_uri: REDIRECT_URI.into(),
            target: app_target("notes@bob"),
        };
        assert!(matches!(
            fed.target
                .handle_login_by_remote_verify_hub(Some(&fed.caller), request.clone())
                .await,
            Err(RPCErrors::ParseRequestError(_))
        ));
        assert!(matches!(
            fed.target
                .handle_login_by_remote_verify_hub(Some(&token_pair.session_token), request)
                .await,
            Err(RPCErrors::NoPermission(_))
        ));
    }

    #[tokio::test]
    async fn did_login_maps_onboarded_accounts_and_canonical_refuses_ineligible_ones() {
        let fed = federation().await;
        fed.target_zone
            .add_user("alice-b", UserType::Limited, Some("pw"));
        fed.target_zone
            .set_doc("alice-b", "did:bns:alice", Some("did:bns:zonea"));
        fed.target_zone.add_user("root", UserType::Root, Some("pw"));
        fed.target_zone.set_doc("root", "did:bns:alice", None);

        let code = fed.code("v-1").await;
        let LoginByRemoteVerifyHubResult::Issued {
            user_id,
            token_pair,
            ..
        } = fed.login("did:bns:alice", &code, "v-1").await
        else {
            panic!("expected an account session");
        };
        assert_eq!(user_id.as_deref(), Some("alice-b"));
        let session = decoded(&token_pair.session_token);
        assert_eq!(session.sub.as_deref(), Some("alice-b"));
        assert!(!token_is_external(&session));
        assert_eq!(token_auth_method(&session), Some(AuthMethod::DidFederation));
        assert_eq!(
            ceiling_of(&token_pair.session_token),
            SessionCeiling::for_user_type(UserType::Limited)
        );
        assert!(fed
            .target
            .handle_refresh_token(&token_pair.refresh_token)
            .await
            .is_ok());

        // A suspended account is not replaced by an External session.
        fed.target_zone.update_user("alice-b", |user| {
            user.state = UserState::Suspended("t".into())
        });
        let code = fed.code("v-2").await;
        assert_eq!(
            remote_denial(fed.login("did:bns:alice", &code, "v-2").await).0,
            DenyReason::AccountState
        );

        // Canonical side: only Active Admin / User accounts of this zone, for their own DID.
        let refused = |result: Result<IssueFederationCodeResult>| {
            denial_reason(&result.unwrap_err()).unwrap()
        };
        assert_eq!(
            refused(
                fed.issue("carol", "did:bns:carol", "did:bns:zoneb", "v")
                    .await
            ),
            (DenyReason::SessionKindNotAllowed, None)
        );
        assert_eq!(
            refused(
                fed.issue("alice", "did:bns:mallory", "did:bns:zoneb", "v")
                    .await
            ),
            (
                DenyReason::SessionKindNotAllowed,
                Some("did_mismatch".into())
            )
        );
        fed.canonical_zone
            .set_doc("alice", "did:bns:alice", Some("did:bns:zonec"));
        assert_eq!(
            refused(
                fed.issue("alice", "did:bns:alice", "did:bns:zoneb", "v")
                    .await
            ),
            (
                DenyReason::SessionKindNotAllowed,
                Some("not_canonical_zone".into())
            )
        );
        fed.canonical_zone.update_user("alice", |user| {
            user.state = UserState::Suspended("t".into())
        });
        assert_eq!(
            refused(
                fed.issue("alice", "did:bns:alice", "did:bns:zoneb", "v")
                    .await
            ),
            (DenyReason::AccountState, None)
        );
        assert!(matches!(
            fed.canonical
                .handle_issue_federation_code(
                    None,
                    IssueFederationCodeRequest {
                        user_id: "alice".into(),
                        did: "did:bns:alice".into(),
                        client_zone: "did:bns:zoneb".into(),
                        redirect_uri: REDIRECT_URI.into(),
                        nonce: "n".into(),
                        code_challenge: pkce_s256("v"),
                        profile: FederationProfile::default(),
                    },
                )
                .await,
            Err(RPCErrors::NoPermission(_))
        ));
    }
}
