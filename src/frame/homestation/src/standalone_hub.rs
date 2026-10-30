//! Standalone mode stands in for the zone's verify-hub on `/kapi/verify-hub`: other zones' stations
//! sign their owners in with a cross-zone assertion (doc/arch/verify-hub/跨Zone登录断言契约.md §4),
//! judged by the same custody rule against the declared directory, and get a session of this zone
//! for HomeStation. Tokens are JWTs signed with the zone key.

use crate::directory::StaticDirectory;
use crate::sign::{decoding_key_from_x, verify_signature, Signer};
use buckyos_api::{cross_zone_target_scopes, verify_signed_assertion, SessionScope, CROSS_ZONE_LOGIN_METHOD};
use jsonwebtoken::DecodingKey;
use serde_json::{json, Value};
use std::sync::Arc;

const ISSUER: &str = "homestation-standalone";
const SESSION_TTL_SECS: u64 = 15 * 60;
const REFRESH_TTL_SECS: u64 = 7 * 24 * 3600;

pub struct StandaloneHub {
    zone: String,
    signer: Signer,
    key: DecodingKey,
    directory: Arc<StaticDirectory>,
}

impl StandaloneHub {
    /// `public_x`: the zone key's public half, which checks the tokens this hub signed.
    pub fn new(zone: &str, signer: Signer, public_x: &str, directory: Arc<StaticDirectory>) -> Result<Self, String> {
        Ok(Self { zone: zone.to_string(), signer, key: decoding_key_from_x(public_x)?, directory })
    }

    fn mint(&self, sub: &str, token_use: &str, ttl: u64) -> Result<String, String> {
        let now = crate::now_s();
        self.signer.sign(&json!({ "iss": ISSUER, "zone": self.zone, "sub": sub, "token_use": token_use, "iat": now, "exp": now + ttl }))
    }

    /// The subject of a token this hub signed for `token_use`, still valid.
    pub fn subject(&self, token: &str, token_use: &str) -> Option<String> {
        let claims = verify_signature(token, &self.key).ok()?;
        let valid = claims["iss"] == ISSUER
            && claims["zone"] == self.zone.as_str()
            && claims["token_use"] == token_use
            && claims["exp"].as_u64().is_some_and(|exp| exp > crate::now_s());
        valid.then(|| claims["sub"].as_str().map(str::to_string)).flatten()
    }

    fn pair(&self, sub: &str) -> Result<Value, String> {
        Ok(json!({ "session_token": self.mint(sub, "session", SESSION_TTL_SECS)?, "refresh_token": self.mint(sub, "refresh", REFRESH_TTL_SECS)? }))
    }

    /// One kRPC call (`login_by_cross_zone_assertion`, `refresh_token`); the kRPC answer.
    pub async fn rpc(&self, body: &[u8]) -> Value {
        let call: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let seq = call["sys"][0].clone();
        let answer = match call["method"].as_str() {
            Some(CROSS_ZONE_LOGIN_METHOD) => self.login(call["params"]["assertion"].as_str().unwrap_or_default()).await,
            Some("refresh_token") => match self.subject(call["params"]["refresh_token"].as_str().unwrap_or_default(), "refresh") {
                Some(sub) => self.pair(&sub).map_err(|e| ("internal".to_string(), e)),
                None => Err(("invalid_token".into(), "refresh token refused".into())),
            },
            _ => Err(("unknown_method".into(), "unknown method".into())),
        };
        match answer {
            Ok(result) => json!({ "result": result, "sys": [seq] }),
            Err((code, error)) => json!({ "error": error, "code": code, "sys": [seq] }),
        }
    }

    async fn login(&self, assertion: &str) -> Result<Value, (String, String)> {
        let zone = name_lib::DID::new("web", &self.zone);
        let verified = verify_signed_assertion(self.directory.as_ref(), assertion, &zone, &self.zone, crate::now_s())
            .await
            .map_err(|failure| (failure.code.as_str().to_string(), failure.detail))?;
        let allowed = cross_zone_target_scopes(&verified.target).unwrap_or(&[]);
        if !allowed.contains(&SessionScope::HomeStation) {
            return Err(("target_not_allowed".into(), verified.target.canonical_key()));
        }
        self.pair(&verified.requester.to_string()).map_err(|e| ("internal".into(), e))
    }
}
