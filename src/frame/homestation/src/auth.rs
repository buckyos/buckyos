//! Caller identity. Owner methods take zone session tokens; protocol reads take a session
//! token (same zone) or a self-signed reader proof (other zones, §4.4 读者身份), or nothing
//! (anonymous). Only what this module verified counts as identity.

use crate::audience::Reader;
use crate::directory::Directory;
use crate::sign::{decode_unverified, kid_did, verify_signature, Signer};
use crate::{new_id, now_s};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct Caller {
    /// Zone user id (session `sub`).
    pub principal: String,
    pub did: Option<String>,
    pub app_id: Option<String>,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, token: &str) -> Result<Caller, String>;
}

#[derive(Default, Deserialize)]
pub struct StaticTokens {
    /// `token → { principal, did?, app_id? }`
    pub tokens: HashMap<String, StaticIdentity>,
}

#[derive(Clone, Deserialize)]
pub struct StaticIdentity {
    pub principal: String,
    #[serde(default)]
    pub did: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
}

#[async_trait]
impl Authenticator for StaticTokens {
    async fn authenticate(&self, token: &str) -> Result<Caller, String> {
        match self.tokens.get(token) {
            Some(id) => Ok(Caller { principal: id.principal.clone(), did: id.did.clone(), app_id: id.app_id.clone() }),
            None => Err("invalid session token".to_string()),
        }
    }
}

/// System mode: verify-hub session tokens; the user's DID comes from `users/<id>/profile`.
#[derive(Default)]
pub struct RuntimeAuth {
    dids: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl Authenticator for RuntimeAuth {
    async fn authenticate(&self, token: &str) -> Result<Caller, String> {
        use buckyos_api::{get_buckyos_api_runtime, validate_verify_hub_token_claims, TokenUse};
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("runtime unavailable: {e}"))?;
        let verified = runtime.verify_trusted_session_token(token).await.map_err(|e| format!("invalid session token: {e}"))?;
        let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session).map_err(|e| format!("invalid session token: {e}"))?;
        let principal = verified.sub.clone().filter(|s| !s.trim().is_empty()).ok_or("session token has no subject")?;
        let cached = self.dids.lock().unwrap().get(&principal).cloned();
        let did = match cached {
            Some(did) => Some(did),
            None => {
                let did = user_did(&principal).await;
                if let Some(did) = &did {
                    self.dids.lock().unwrap().insert(principal.clone(), did.clone());
                }
                did
            }
        };
        Ok(Caller { principal, did, app_id: Some(claims.target.canonical_key()) })
    }
}

async fn user_did(user_id: &str) -> Option<String> {
    let runtime = buckyos_api::get_buckyos_api_runtime().ok()?;
    let client = runtime.get_system_config_client().await.ok()?;
    let value = client.get(&format!("users/{user_id}/profile")).await.ok()?;
    let profile: Value = serde_json::from_str(&value.value).ok()?;
    profile.get("did").and_then(Value::as_str).map(str::to_string)
}

pub const READER_PROOF_TTL: u64 = 300;

/// A short-lived statement "I am `iss` reading `aud`", signed by a key `iss` authorizes.
pub fn make_reader_proof(signer: &Signer, reader: &str, target_zone: &str) -> Result<String, String> {
    let iat = now_s();
    let claims = json!({
        "iss": reader,
        "aud": format!("cyfs://{target_zone}/home"),
        "iat": iat,
        "exp": iat + READER_PROOF_TTL,
        "jti": new_id("r"),
    });
    signer.sign(&claims)
}

pub async fn verify_reader_proof(directory: &dyn Directory, jwt: &str, own_zone: &str) -> Result<String, String> {
    let decoded = decode_unverified(jwt)?;
    let iss = decoded.claims.get("iss").and_then(Value::as_str).ok_or("reader proof has no iss")?.to_string();
    let aud = decoded.claims.get("aud").and_then(Value::as_str).unwrap_or_default();
    if aud != format!("cyfs://{own_zone}/home") {
        return Err("reader proof is for another zone".into());
    }
    let iat = decoded.claims.get("iat").and_then(Value::as_u64).ok_or("reader proof has no iat")?;
    let exp = decoded.claims.get("exp").and_then(Value::as_u64).ok_or("reader proof has no exp")?;
    let now = now_s();
    if exp < now || iat > now + 60 || exp.saturating_sub(iat) > 2 * READER_PROOF_TTL {
        return Err("reader proof expired or too long-lived".into());
    }
    let signer = kid_did(&decoded.kid).to_string();
    if !directory.authorizes(&iss, &signer, &decoded.kid).await.map_err(|e| e.to_string())? {
        return Err(format!("{signer} may not sign for {iss}"));
    }
    let key = directory.signer_key(&signer, &decoded.kid).await.map_err(|e| e.to_string())?;
    verify_signature(jwt, &key)?;
    Ok(iss)
}

/// Reader of a protocol request: owner session, another zone user, a reader proof, or anonymous.
pub async fn reader_from_request(
    auth: &dyn Authenticator,
    directory: &dyn Directory,
    owner: &str,
    zone: &str,
    authorization: Option<&str>,
    access_token: Option<&str>,
) -> Result<Reader, String> {
    if let Some(value) = authorization {
        if let Some(proof) = value.strip_prefix("DID ") {
            let did = verify_reader_proof(directory, proof.trim(), zone).await?;
            return Ok(if did == owner { Reader::Owner } else { Reader::Did(did) });
        }
        if let Some(token) = value.strip_prefix("Bearer ") {
            return session_reader(auth, owner, token.trim()).await;
        }
        return Err("unsupported authorization scheme".into());
    }
    if let Some(token) = access_token {
        return session_reader(auth, owner, token).await;
    }
    Ok(Reader::Anonymous)
}

async fn session_reader(auth: &dyn Authenticator, owner: &str, token: &str) -> Result<Reader, String> {
    let caller = auth.authenticate(token).await?;
    Ok(match caller.did {
        Some(did) if did == owner => Reader::Owner,
        Some(did) => Reader::Did(did),
        None => Reader::Anonymous,
    })
}
