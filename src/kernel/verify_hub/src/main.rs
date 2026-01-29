#[allow(unused_braces)]
use base64::{engine::general_purpose::STANDARD, Engine as _};
use lazy_static::lazy_static;
use log::*;
use rand::prelude::*;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use async_trait::async_trait;

use ::kRPC::*;
use buckyos_api::*;
use buckyos_kit::*;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use name_lib::*;

// Token expiration time constants
// Session token: short-lived, used for API requests
const SESSION_TOKEN_EXPIRE_SECONDS: u64 = 15 * 60; // 15 minutes
// Refresh token: long-lived, used to obtain new token pairs
const REFRESH_TOKEN_EXPIRE_SECONDS: u64 = 7 * 24 * 3600; // 7 days
use cyfs_gateway_lib::{HttpServer, ServerError, ServerResult, StreamInfo, serve_http_by_rpc_handler, server_err, ServerErrorCode};
use server_runner::*;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;

type Result<T> = std::result::Result<T, RPCErrors>;

#[derive(Clone, Debug, PartialEq)]
struct VerifyServiceConfig {
    zone_config: ZoneConfig,
    device_id: String,
}



const VERIFY_HUB_ISSUER: &str = "verify-hub";
const TOKEN_USE_SESSION: &str = "session";
const TOKEN_USE_REFRESH: &str = "refresh";
const TOKEN_USE_LOGIN: &str = "login";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct VerifyHubJwtClaims {
    #[serde(skip_serializing_if = "Option::is_none")]
    jti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<u64>,
    iss: String,
    exp: u64,
    token_use: String,
}

fn sign_verify_hub_jwt<T: serde::Serialize>(claims: &T, private_key: &EncodingKey) -> Result<String> {
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = None;
    header.kid = Some(VERIFY_HUB_ISSUER.to_string());
    jsonwebtoken::encode(&header, claims, private_key)
        .map_err(|e| RPCErrors::ReasonError(format!("JWT encode error: {e}")))
}

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
    static ref TRUSTKEY_CACHE: Arc<Mutex<HashMap<String, DecodingKey>>> =
        Arc::new(Mutex::new(HashMap::new()));
    static ref VERIFY_SERVICE_CONFIG: Arc<Mutex<Option<VerifyServiceConfig>>> =
        Arc::new(Mutex::new(None));
    static ref MY_RPC_TOKEN: Arc<Mutex<Option<RPCSessionToken>>> =  Arc::new(Mutex::new(None)) ;
}

/// Generate a session token with specified parameters
/// Session token is short-lived and used for API requests
async fn generate_session_token(
    appid: &str,
    userid: &str,
    jti: u64,
    session: u64,
    duration: u64,
) -> Result<RPCSessionToken> {
    let now = buckyos_get_unix_timestamp();
    let exp = now + duration;

    let mut session_token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        jti: Some(jti.to_string()),
        aud: Some(appid.to_string()),
        sub: Some(userid.to_string()),
        token: None,
        session: Some(session),
        iss: Some(VERIFY_HUB_ISSUER.to_string()),
        exp: Some(exp),
    };

    {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let claims = VerifyHubJwtClaims {
            jti: Some(jti.to_string()),
            aud: Some(appid.to_string()),
            sub: Some(userid.to_string()),
            session: Some(session),
            iss: VERIFY_HUB_ISSUER.to_string(),
            exp,
            token_use: TOKEN_USE_SESSION.to_string(),
        };
        session_token.token = Some(sign_verify_hub_jwt(&claims, &private_key)?);
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
) -> Result<RPCSessionToken> {
    let now = buckyos_get_unix_timestamp();
    let exp = now + duration;

    let mut refresh_token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        jti: Some(jti.to_string()),
        aud: Some(appid.to_string()),
        sub: Some(userid.to_string()),
        token: None,
        session: Some(session),
        iss: Some(VERIFY_HUB_ISSUER.to_string()),
        exp: Some(exp),
    };

    {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let claims = VerifyHubJwtClaims {
            jti: Some(jti.to_string()),
            aud: Some(appid.to_string()),
            sub: Some(userid.to_string()),
            session: Some(session),
            iss: VERIFY_HUB_ISSUER.to_string(),
            exp,
            token_use: TOKEN_USE_REFRESH.to_string(),
        };
        refresh_token.token = Some(sign_verify_hub_jwt(&claims, &private_key)?);
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
    )
    .await?;

    // Generate long-lived refresh token
    let refresh_token = generate_refresh_token(
        appid,
        userid,
        refresh_jti,
        session_id,
        REFRESH_TOKEN_EXPIRE_SECONDS,
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
    REFRESH_TOKEN_CACHE.lock().await.insert(key.to_string(), token);
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
    if my_rpc_token.is_some() {
        let token = my_rpc_token.as_ref().unwrap();
        if token.exp.is_some() {
            if token.exp.unwrap() - 30 > now {
                return Ok(token.clone());
            }
        }
    }
    drop(my_rpc_token);

    let exp = now + VERIFY_HUB_TOKEN_EXPIRE_TIME;

    let mut session_token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        jti: None,
        aud: Some("verify-hub".to_string()),
        sub: Some(device_id),
        token: None,
        session: None,
        iss: Some(VERIFY_HUB_ISSUER.to_string()),
        exp: Some(exp),
    };

    {
        let private_key = VERIFY_HUB_PRIVATE_KEY.read().await;
        let claims = VerifyHubJwtClaims {
            jti: None,
            aud: Some(VERIFY_HUB_ISSUER.to_string()),
            sub: session_token.sub.clone(),
            session: None,
            iss: VERIFY_HUB_ISSUER.to_string(),
            exp,
            token_use: TOKEN_USE_SESSION.to_string(),
        };
        session_token.token = Some(sign_verify_hub_jwt(&claims, &private_key)?);
    }

    let mut my_rpc_token = MY_RPC_TOKEN.lock().await;
    *my_rpc_token = Some(session_token.clone());
    Ok(session_token)
}

async fn load_token_from_cache(key: &str) -> Option<RPCSessionToken> {
    let cache = TOKEN_CACHE.lock().await;
    let token = cache.get(key);
    if token.is_none() {
        return None;
    } else {
        return Some(token.unwrap().clone());
    }
}

async fn cache_token(key: &str, token: RPCSessionToken) {
    TOKEN_CACHE.lock().await.insert(key.to_string(), token);
}

async fn load_trustkey_from_cache(kid: &str) -> Option<DecodingKey> {
    let cache = TRUSTKEY_CACHE.lock().await;
    let decoding_key = cache.get(kid);
    if decoding_key.is_none() {
        return None;
    }
    return Some(decoding_key.unwrap().clone());
}

async fn cache_trustkey(kid: &str, key: DecodingKey) {
    TRUSTKEY_CACHE.lock().await.insert(kid.to_string(), key);
}

async fn get_trust_public_key_from_kid(kid: &Option<String>) -> Result<DecodingKey> {
    //turst keys include : zone's owner, admin users, server device
    //kid : {owner}
    //kid : #device_id

    let kid = kid.clone().unwrap_or("verify-hub".to_string());
    let cached_key = load_trustkey_from_cache(&kid).await;
    if cached_key.is_some() {
        return Ok(cached_key.unwrap());
    }

    //not found in trustkey_cache, try load from system config service
    let result_key: DecodingKey;
    if kid == "root" {
        //load zone config from system config service
        let owner_auth_key = VERIFY_SERVICE_CONFIG
            .lock()
            .await
            .as_ref()
            .unwrap()
            .zone_config
            .get_auth_key(None)
            .ok_or(RPCErrors::ReasonError(
                "Owner public key not found".to_string(),
            ))?;
        result_key = owner_auth_key.0;
        info!("load owner public key from zone config");
    } else {
        //load device config from system config service(not from name-lib)
        let _zone_config = VERIFY_SERVICE_CONFIG
            .lock()
            .await
            .as_ref()
            .unwrap()
            .zone_config
            .clone();
        let rpc_token = get_my_krpc_token().await?;
        let rpc_token_str = rpc_token.to_string();
        let system_config_client = SystemConfigClient::new(None, Some(rpc_token_str.as_str()));
        let control_panel_client = ControlPanelClient::new(system_config_client);
        let device_config = control_panel_client.get_device_config(&kid).await;
        if device_config.is_err() {
            warn!(
                "load device {} config from system config service failed",
                kid
            );
            return Err(RPCErrors::ReasonError(
                "Device config not found".to_string(),
            ));
        }
        let device_config = device_config.unwrap();
        let result_device_key = device_config
            .get_auth_key(None)
            .ok_or(RPCErrors::ReasonError(
                "Device public key not found".to_string(),
            ))?;
        result_key = result_device_key.0;
    }

    //kid is device_id,try load device config from system config service
    cache_trustkey(&kid, result_key.clone()).await;

    return Ok(result_key);
}

// return (kid, payload)
async fn verify_trusted_jwt(jwt: &str) -> Result<(String, Value)> {
    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        error!("JWT decode header error: {}", error);
        RPCErrors::ReasonError("JWT decode header error".to_string())
    })?;

    if header.alg != Algorithm::EdDSA {
        return Err(RPCErrors::ReasonError("JWT algorithm not allowed".to_string()));
    }

    let mut validation = Validation::new(Algorithm::EdDSA);
    // We don't have an expected audience for generic trusted JWTs.
    validation.validate_aud = false;

    // try get public key from header.kid
    let public_key = get_trust_public_key_from_kid(&header.kid).await?;

    // verify jwt
    let decoded_token =
        jsonwebtoken::decode::<Value>(jwt, &public_key, &validation).map_err(|error| {
            error!("JWT verify error: {}", error);
            RPCErrors::ReasonError("JWT verify error".to_string())
        })?;

    let kid = header.kid.unwrap_or(VERIFY_HUB_ISSUER.to_string());
    Ok((kid, decoded_token.claims))
}

async fn verify_verify_hub_jwt(
    jwt: &str,
    expected_token_use: &str,
    expected_audience: Option<&str>,
) -> Result<Value> {
    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        error!("JWT decode header error: {}", error);
        RPCErrors::ReasonError("JWT decode header error".to_string())
    })?;

    if header.alg != Algorithm::EdDSA {
        return Err(RPCErrors::ReasonError("JWT algorithm not allowed".to_string()));
    }

    if header.kid.as_deref() != Some(VERIFY_HUB_ISSUER) {
        return Err(RPCErrors::ReasonError("JWT kid not allowed".to_string()));
    }

    // Always verify verify-hub issued tokens by verify-hub public key
    let public_key = get_trust_public_key_from_kid(&Some(VERIFY_HUB_ISSUER.to_string())).await?;

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[VERIFY_HUB_ISSUER]);
    validation.set_required_spec_claims(&["exp", "iss"]);

    if let Some(aud) = expected_audience {
        validation.set_audience(&[aud]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    } else {
        // If we don't have an expected audience from the caller, don't enable aud validation.
        // Otherwise tokens that contain aud would be rejected with no configured audience set.
        validation.validate_aud = false;
    }

    let decoded_token =
        jsonwebtoken::decode::<Value>(jwt, &public_key, &validation).map_err(|error| {
            error!("JWT verify error: {}", error);
            RPCErrors::ReasonError("JWT verify error".to_string())
        })?;

    let token_use = decoded_token
        .claims
        .get("token_use")
        .and_then(|v| v.as_str())
        .ok_or(RPCErrors::ReasonError("Missing token_use".to_string()))?;
    if token_use != expected_token_use {
        return Err(RPCErrors::ReasonError("Invalid token_use".to_string()));
    }

    Ok(decoded_token.claims)
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
        _login_params: Option<Value>,
    ) -> Result<buckyos_api::TokenPair> {
        gc_token_caches().await;

        // Step 2: Verify JWT signature and extract payload
        let (_kid, jwt_payload) = verify_trusted_jwt(jwt).await?;
    
        let token_use = jwt_payload
            .get("token_use")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| TOKEN_USE_LOGIN.to_string());
    
        // Refresh token must be verified by verify-hub key, not any trust-chain key.
        let jwt_payload = if token_use == TOKEN_USE_REFRESH {
            verify_verify_hub_jwt(jwt, TOKEN_USE_REFRESH, None).await?
        } else {
            jwt_payload
        };
    
        // Step 3: Extract required fields from JWT payload
        // Support both old (userid/appid/nonce) and new (sub/aud/jti) field names for compatibility
        let userid = jwt_payload
            .get("sub")
            .or_else(|| jwt_payload.get("userid"))
            .ok_or(RPCErrors::ReasonError("Missing sub/userid".to_string()))?;
        let userid = userid
            .as_str()
            .ok_or(RPCErrors::ReasonError("Invalid sub/userid".to_string()))?;
        let appid = jwt_payload
            .get("aud")
            .or_else(|| jwt_payload.get("appid"))
            .ok_or(RPCErrors::ReasonError("Missing aud/appid".to_string()))?;
        let appid = appid
            .as_str()
            .ok_or(RPCErrors::ReasonError("Invalid aud/appid".to_string()))?;
    
        let exp = jwt_payload
            .get("exp")
            .ok_or(RPCErrors::ReasonError("Missing exp".to_string()))?;
        let exp = exp
            .as_u64()
            .ok_or(RPCErrors::ReasonError("Invalid exp".to_string()))?;
    
        // Support both old (nonce) and new (jti) field names
        let login_jti = jwt_payload.get("jti").or_else(|| jwt_payload.get("nonce"));
        let mut token_jti: u64 = 0;
        if login_jti.is_some() {
            // jti can be string or u64
            let jti_val = login_jti.unwrap();
            if let Some(jti_str) = jti_val.as_str() {
                token_jti = jti_str.parse::<u64>()
                    .map_err(|_| RPCErrors::ReasonError("Invalid jti/nonce".to_string()))?;
            } else {
                token_jti = jti_val
                    .as_u64()
                    .ok_or(RPCErrors::ReasonError("Invalid jti/nonce".to_string()))?;
            }
        }
    
        if token_use == TOKEN_USE_REFRESH {
            // ============================================================
            // REFRESH FLOW: Using refresh_token to get new token pair
            // The incoming JWT is a refresh_token issued by verify-hub
            // ============================================================
            
            // Step 4a: Extract session_id from refresh token
            let session_id = jwt_payload.get("session");
            if session_id.is_none() {
                return Err(RPCErrors::ReasonError("Missing session_id".to_string()));
            }
            let session_id = session_id
                .unwrap()
                .as_u64()
                .ok_or(RPCErrors::ReasonError("Invalid session_id".to_string()))?;
            if session_id == 0 {
                return Err(RPCErrors::ReasonError("Invalid session_id".to_string()));
            }
            
            let session_key = format!("{}_{}_{}", userid, appid, session_id);
            info!("Handle refresh token request for session: {}", session_key);
                
            // Step 5a: Validate the refresh token exists in cache
            // This ensures the refresh token hasn't been invalidated
            let cached_refresh = load_refresh_token_from_cache(session_key.as_str()).await;
            if cached_refresh.is_none() {
                warn!("Refresh token not found in cache for session: {}", session_key);
                warn!("Refresh reuse detected (cache-miss), revoking session: {}", session_key);
                revoke_session_tokens(session_key.as_str()).await;
                return Err(RPCErrors::ReasonError(
                    "Refresh token not found or already invalidated".to_string(),
                ));
            }
                
            // Step 6a: Verify the jti matches the cached refresh token
            // This prevents replay attacks with old refresh tokens
            let old_refresh = cached_refresh.unwrap();
            let old_jti = old_refresh.jti.as_ref()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            if old_jti != token_jti {
                warn!(
                    "Invalid refresh token jti. Expected: {:?}, Got: {}",
                    old_refresh.jti, token_jti
                );
                warn!("Refresh reuse detected (jti-mismatch), revoking session: {}", session_key);
                revoke_session_tokens(session_key.as_str()).await;
                return Err(RPCErrors::ReasonError(
                    "Invalid refresh token jti".to_string(),
                ));
            }
                
            // Step 7a: Check if refresh token is expired
            if buckyos_get_unix_timestamp() > exp {
                // Invalidate the expired refresh token
                invalidate_refresh_token(session_key.as_str()).await;
                return Err(RPCErrors::ReasonError("Refresh token expired".to_string()));
            }
            
            // Step 8a: IMPORTANT - Invalidate the old refresh token immediately
            // This ensures the old refresh token cannot be reused (one-time use)
            invalidate_refresh_token(session_key.as_str()).await;
            info!("Old refresh token invalidated for session: {}", session_key);
            
            // Step 9a: Generate new token pair (session_token + refresh_token)
            let (token_pair, session_token, refresh_token) =
                generate_token_pair(appid, userid, session_id).await?;
            
            // Step 10a: Cache the new tokens
            cache_token(session_key.as_str(), session_token).await;
            cache_refresh_token(session_key.as_str(), refresh_token).await;
            
            info!(
                "Refresh successful for session: {}. New token pair generated.",
                session_key
            );
            return Ok(buckyos_api::TokenPair {
                session_token: token_pair.session_token,
                refresh_token: token_pair.refresh_token,
            });
        } else {
                // ============================================================
                // FIRST LOGIN FLOW: Using trusted device/owner JWT
                // The incoming JWT is signed by a trusted entity (device/owner)
                // ============================================================
                info!("Handle first login by JWT for user: {}", userid);
                
                let session_key = format!("{}_{}_{}", userid, appid, token_jti);
    
                // Step 4b: Check if JWT has expired
                if buckyos_get_unix_timestamp() > exp {
                    return Err(RPCErrors::ReasonError("Login JWT expired".to_string()));
                }
    
                // Step 5b: Check if this login JWT has already been used (replay protection)
                let cache_result = load_token_from_cache(session_key.as_str()).await;
                if cache_result.is_some() {
                    return Err(RPCErrors::ReasonError("Login JWT already used".to_string()));
                }
    
                // Step 6b: Generate new session_id for this login session
                let session_id: u64;
                {
                    let mut rng = rand::thread_rng();
                    session_id = rng.gen::<u64>();
                }
                let new_session_key = format!("{}_{}_{}", userid, appid, session_id);
                
                // Step 7b: Generate new token pair (session_token + refresh_token)
                let (token_pair, session_token, refresh_token) =
                    generate_token_pair(appid, userid, session_id).await?;
                
                // Step 8b: Cache both tokens
                // Cache by original session_key to mark login JWT as used
                cache_token(session_key.as_str(), session_token.clone()).await;
                // Cache by new session_key for future refresh operations
                cache_token(new_session_key.as_str(), session_token).await;
                cache_refresh_token(new_session_key.as_str(), refresh_token).await;
                
                info!(
                    "Login successful for user: {}. Session: {}. Token pair generated.",
                    userid, new_session_key
                );
                return Ok(buckyos_api::TokenPair {
                    session_token: token_pair.session_token,
                    refresh_token: token_pair.refresh_token,
                });
        }

    }

    async fn handle_login_by_password(
        &self,
        username: &str,
        password: &str,
        appid: &str,
        login_nonce: u64,
    ) -> Result<Value> {
        gc_token_caches().await;

        // TODO: verify appid matches the target domain
        // The logic for verifying that appid matches the target domain is an operational logic,
        // planned to be placed in the cyfs-gateway configuration file for easy adjustment through configuration
    
        // Step 2: Validate login nonce (prevent replay attacks)
        let now = buckyos_get_unix_timestamp() * 1000;
        let abs_diff = now.abs_diff(login_nonce);
        debug!(
            "{} login nonce and now abs_diff:{}, from:{}",
            username, abs_diff, appid
        );
        if now.abs_diff(login_nonce) > 3600 * 1000 * 8 {
            warn!(
                "{} login nonce is too old, abs_diff:{}, this is a possible ATTACK?",
                username, abs_diff
            );
            return Err(RPCErrors::ParseRequestError("Invalid nonce".to_string()));
        }
    
        // Step 3: Load user info from system config service
        let user_info_path = format!("users/{}/settings", username);
        let rpc_token = get_my_krpc_token().await?;
        let rpc_token_str = rpc_token.to_string();
        let system_config_client = SystemConfigClient::new(None, Some(rpc_token_str.as_str()));
        let user_info_result = system_config_client.get(user_info_path.as_str()).await;
        if user_info_result.is_err() {
            warn!(
                "handle_login_by_password: user not found {}",
                user_info_path
            );
            return Err(RPCErrors::UserNotFound(username.to_string()));
        }
        let user_info = user_info_result.unwrap().value;
        let user_info: serde_json::Value = serde_json::from_str(&user_info)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let store_password = user_info.get("password").ok_or(RPCErrors::ReasonError(
            "password not set, can't login by password".to_string(),
        ))?;
        let store_password = store_password
            .as_str()
            .ok_or(RPCErrors::ReasonError("Invalid password".to_string()))?;
        let user_type = user_info.get("type").ok_or(RPCErrors::ReasonError(
            "user type not set, can't login by password".to_string(),
        ))?;
        let user_type = user_type
            .as_str()
            .ok_or(RPCErrors::ReasonError("Invalid user type".to_string()))?;
    
        // Step 4: Verify password
        // Password is hashed with nonce on client side: SHA256(stored_password + nonce)
        let password_hash_input = STANDARD
            .decode(password)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    
        let salt = format!("{}{}", store_password, login_nonce);
        let hash = Sha256::digest(salt.clone()).to_vec();
        if hash != password_hash_input {
            warn!(
                "{} login by password failed, password is wrong!",
                username
            );
            return Err(RPCErrors::InvalidPassword);
        }
    
        // Step 5: Generate new session_id for this login session
        let session_id: u64;
        {
            let mut rng = rand::thread_rng();
            session_id = rng.gen::<u64>();
        }
        let session_key = format!("{}_{}_{}", username, appid, session_id);
        
        info!(
            "Password login successful for user: {}. Generating token pair.",
            username
        );
    
        // Step 6: Generate token pair (session_token + refresh_token)
        // session_token: short-lived (15 minutes) for API requests
        // refresh_token: long-lived (7 days) for obtaining new token pairs
        let (token_pair, session_token, refresh_token) =
            generate_token_pair(appid, username, session_id).await?;
        
        // Step 7: Cache both tokens
        cache_token(session_key.as_str(), session_token).await;
        cache_refresh_token(session_key.as_str(), refresh_token).await;
        
        info!(
            "Token pair cached for session: {}",
            session_key
        );
    
        // Step 8: Return account info with dual tokens
        let result_account_info = json!({
            "user_name": username,
            "user_id": username,
            "user_type": user_type,
            "session_token": token_pair.session_token,
            "refresh_token": token_pair.refresh_token
        });
        return Ok(result_account_info);
    }

    async fn handle_verify_token(
        &self,
        session_token: &str,
        appid: Option<String>,
    ) -> Result<Value> {
        gc_token_caches().await;
        let expected_audience = appid.as_deref();
        let first_dot = session_token.find('.');
        if first_dot.is_none() {
            //this is not a jwt token, use token-store to verify
            return Err(RPCErrors::InvalidToken("not a jwt token".to_string()));
        } else {
            let json_body =
                verify_verify_hub_jwt(session_token, TOKEN_USE_SESSION, expected_audience).await?;
            Ok(json_body)
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
        return Err(server_err!(ServerErrorCode::BadRequest, "Method not allowed"));
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
    let private_key_str = system_config_client.get("system/verify-hub/key").await;
    if private_key_str.is_ok() {
        let private_key = private_key_str.unwrap().value;
        let private_key = EncodingKey::from_ed_pem(private_key.as_bytes());
        if private_key.is_ok() {
            let private_key = private_key.unwrap();
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
    cache_trustkey("verify-hub", verify_hub_pub_key).await;
    info!("verify_hub public key loaded from system config service OK!");

    let new_service_config = VerifyServiceConfig {
        zone_config: zone_config,
        device_id: device_id,
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
    if let Err(error) = load_service_config().await {
        error!("load service config failed:{}", error);
        if !cfg!(test) {
            return -1;
        }
        warn!("cfg(test) enabled: continue running with test defaults");
    }
    //load cache from service_cache@dfs:// and service_local_cache@fs://

    let server = VerifyHubServer::new();
    const VERIFY_HUB_SERVICE_MAIN_PORT: u16 = 3300;
    info!("verify_hub service initialized, running on port {}", VERIFY_HUB_SERVICE_MAIN_PORT);
    let runner = Runner::new(VERIFY_HUB_SERVICE_MAIN_PORT);
    let _ = runner.add_http_server("/kapi/verify-hub".to_string(), Arc::new(server));
    let _ = runner.run().await;
    return 0;
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
        cache_trustkey("verify-hub", test_pk.clone()).await;
        cache_trustkey("root", test_pk).await;

        let test_owner_private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;
        EncodingKey::from_ed_pem(test_owner_private_key_pem.as_bytes()).unwrap()
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
            token_type: RPCSessionTokenType::JWT,
            jti: Some(jti.to_string()),
            aud: Some(appid.to_string()),
            sub: Some(userid.to_string()),
            token: None,
            session: Some(jti), // Use jti as session for first login
            iss: Some("root".to_string()),
            exp: Some(exp),
        };

        test_login_token
            .generate_jwt(Some("root".to_string()), private_key)
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
            .verify_token(&token_pair.session_token, Some("kernel"))
            .await;
        assert!(verify_ok.is_ok(), "verify_token should succeed for correct appid");

        let verify_bad = verify_hub_client
            .verify_token(&token_pair.session_token, Some("not-kernel"))
            .await;
        assert!(verify_bad.is_err(), "verify_token should reject wrong appid");

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
            .handle_login_by_jwt(token_pair.refresh_token.as_str(), None)
            .await;
        
        assert!(refresh_result.is_ok(), "Refresh should succeed");
        let new_token_pair = refresh_result.unwrap();
        
        println!("Refresh successful!");
        println!("  new session_token: {}...", &new_token_pair.session_token[..50]);
        println!("  new refresh_token: {}...", &new_token_pair.refresh_token[..50]);

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
        
        assert!(verify_new_result.is_ok(), "New session token should be valid");
        println!("New session token verified successfully!");

        // ============================================================
        // Test 5: Try to reuse old refresh token (should fail)
        // Expected: Old refresh token is invalidated, reuse should fail
        // ============================================================
        println!("\n=== Test 5: Reuse old refresh token (should fail) ===");
        
        let reuse_result = handler
            .handle_login_by_jwt(token_pair.refresh_token.as_str(), None)
            .await;
        
        assert!(reuse_result.is_err(), "Reusing old refresh token should fail");
        println!("Old refresh token correctly rejected: {:?}", reuse_result.err());

        // ============================================================
        // Test 6: Second refresh with new refresh token
        // Expected: Should fail because reuse detection revokes the session
        // ============================================================
        println!("\n=== Test 6: Second refresh with new refresh token ===");
        
        let second_refresh_result = handler
            .handle_login_by_jwt(new_token_pair.refresh_token.as_str(), None)
            .await;
        
        assert!(second_refresh_result.is_err(), "Second refresh should fail after reuse detection");
        println!("Second refresh correctly rejected: {:?}", second_refresh_result.err());

        // ============================================================
        // Test 7: Login with expired JWT (should fail)
        // Expected: Expired login JWT should be rejected
        // ============================================================
        println!("\n=== Test 7: Login with expired JWT (should fail) ===");
        
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
        
        let (token_pair, session_token, refresh_token) =
            generate_token_pair("test-app", "test-user", 12345).await.unwrap();
        
        // Verify token pair contains both tokens
        assert!(!token_pair.session_token.is_empty(), "Session token should not be empty");
        assert!(!token_pair.refresh_token.is_empty(), "Refresh token should not be empty");
        
        // Verify tokens are different
        assert_ne!(
            token_pair.session_token, token_pair.refresh_token,
            "Session and refresh tokens should be different"
        );
        
        // Verify session token has correct expiration (short-lived)
        assert!(
            session_token.exp.unwrap() <= buckyos_get_unix_timestamp() + SESSION_TOKEN_EXPIRE_SECONDS + 1,
            "Session token expiration should be short"
        );
        
        // Verify refresh token has correct expiration (long-lived)
        assert!(
            refresh_token.exp.unwrap() <= buckyos_get_unix_timestamp() + REFRESH_TOKEN_EXPIRE_SECONDS + 1,
            "Refresh token expiration should be long"
        );
        assert!(
            refresh_token.exp.unwrap() > session_token.exp.unwrap(),
            "Refresh token should expire later than session token"
        );
        
        println!("Token pair generated correctly:");
        println!("  Session token exp: {} (in {} seconds)", 
            session_token.exp.unwrap(),
            session_token.exp.unwrap() - buckyos_get_unix_timestamp()
        );
        println!("  Refresh token exp: {} (in {} seconds)", 
            refresh_token.exp.unwrap(),
            refresh_token.exp.unwrap() - buckyos_get_unix_timestamp()
        );
    }

    /// Test refresh token cache operations
    #[tokio::test]
    async fn test_refresh_token_cache() {
        println!("\n=== Test: Refresh token cache operations ===");
        
        let test_token = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            jti: Some("123456".to_string()),
            aud: Some("test-app".to_string()),
            sub: Some("test-user".to_string()),
            token: Some("test-token-value".to_string()),
            session: Some(789),
            iss: Some("verify-hub".to_string()),
            exp: Some(buckyos_get_unix_timestamp() + 3600),
        };
        
        let cache_key = "test_cache_key";
        
        // Test cache is initially empty
        let initial = load_refresh_token_from_cache(cache_key).await;
        assert!(initial.is_none(), "Cache should be initially empty");
        
        // Test caching token
        cache_refresh_token(cache_key, test_token.clone()).await;
        let cached = load_refresh_token_from_cache(cache_key).await;
        assert!(cached.is_some(), "Token should be cached");
        assert_eq!(cached.unwrap().jti, test_token.jti, "Cached token should match");
        
        // Test invalidating token
        invalidate_refresh_token(cache_key).await;
        let after_invalidate = load_refresh_token_from_cache(cache_key).await;
        assert!(after_invalidate.is_none(), "Token should be invalidated");
        
        println!("Refresh token cache operations work correctly!");
    }
}
