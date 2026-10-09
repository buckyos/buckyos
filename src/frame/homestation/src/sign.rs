//! JWT form of named objects (§5.3): the ObjId is computed from the claims only, the
//! signature is carried in the JWT header/kid and verified against the signer's DID document.

use base64::Engine;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Validation};
use serde_json::Value;

#[derive(Clone)]
pub struct Signer {
    key: EncodingKey,
    /// `did#fragment` written into the JWT header.
    pub kid: String,
    /// DID part of `kid`: the publisher itself or a device it authorizes.
    pub signer_did: String,
}

impl Signer {
    pub fn new(key: EncodingKey, kid: impl Into<String>) -> Self {
        let kid = kid.into();
        let signer_did = kid_did(&kid).to_string();
        Self { key, kid, signer_did }
    }

    pub fn from_pem(pem: &[u8], kid: impl Into<String>) -> Result<Self, String> {
        let key = EncodingKey::from_ed_pem(pem).map_err(|e| format!("invalid ed25519 key: {e}"))?;
        Ok(Self::new(key, kid))
    }

    pub fn sign(&self, claims: &Value) -> Result<String, String> {
        ndn_lib::named_obj_to_jwt(claims, &self.key, Some(self.kid.clone())).map_err(|e| format!("sign failed: {e}"))
    }
}

pub fn kid_did(kid: &str) -> &str {
    kid.split('#').next().unwrap_or(kid)
}

pub fn kid_fragment(kid: &str) -> Option<String> {
    kid.split_once('#').map(|(_, f)| format!("#{f}"))
}

pub struct DecodedJwt {
    pub claims: Value,
    pub kid: String,
}

/// Header and claims without checking the signature (the caller verifies next).
pub fn decode_unverified(jwt: &str) -> Result<DecodedJwt, String> {
    let header = jsonwebtoken::decode_header(jwt).map_err(|e| format!("invalid JWT header: {e}"))?;
    if header.alg != Algorithm::EdDSA {
        return Err("only EdDSA signatures are accepted".into());
    }
    let kid = header.kid.filter(|k| !k.is_empty()).ok_or("JWT has no kid")?;
    let payload = jwt.split('.').nth(1).ok_or("malformed JWT")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| "malformed JWT payload")?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|_| "JWT payload is not JSON")?;
    if !claims.is_object() {
        return Err("JWT claims must be an object".into());
    }
    Ok(DecodedJwt { claims, kid })
}

/// Named objects carry no `exp`; reader proofs check their own time window.
pub fn verify_signature(jwt: &str, key: &DecodingKey) -> Result<Value, String> {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    jsonwebtoken::decode::<Value>(jwt, key, &validation)
        .map(|data| data.claims)
        .map_err(|e| format!("invalid signature: {e}"))
}

/// A fresh ed25519 key: PKCS#8 PEM and the base64url public key (`x`), for standalone
/// identities and tests.
pub fn generate_key() -> (String, String) {
    use ed25519_dalek::pkcs8::EncodePrivateKey;
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let pem = key.to_pkcs8_pem(Default::default()).expect("pkcs8 encode").to_string();
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes());
    (pem, x)
}

pub fn decoding_key_from_x(x: &str) -> Result<DecodingKey, String> {
    DecodingKey::from_ed_components(x).map_err(|e| format!("invalid public key: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sign_and_verify() {
        let (pem, x) = generate_key();
        let signer = Signer::from_pem(pem.as_bytes(), "did:test:alice#main_key").unwrap();
        assert_eq!(signer.signer_did, "did:test:alice");
        let claims = json!({ "kind": "post", "iat": 1 });
        let jwt = signer.sign(&claims).unwrap();
        let decoded = decode_unverified(&jwt).unwrap();
        assert_eq!(decoded.kid, "did:test:alice#main_key");
        assert_eq!(decoded.claims, claims);
        assert_eq!(verify_signature(&jwt, &decoding_key_from_x(&x).unwrap()).unwrap(), claims);
        let (_, other) = generate_key();
        assert!(verify_signature(&jwt, &decoding_key_from_x(&other).unwrap()).is_err());
    }
}
