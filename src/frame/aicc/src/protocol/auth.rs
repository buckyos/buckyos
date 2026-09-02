use super::{ProtocolError, ProtocolErrorKind, ProtocolResultValue};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CredentialKind {
    Bearer,
    NamedHeader,
    FalKey,
    GlmJwt,
}

impl CredentialKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::NamedHeader => "named_header",
            Self::FalKey => "fal_key",
            Self::GlmJwt => "glm_jwt",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct AnonymousCredentialRef(String);

impl AnonymousCredentialRef {
    pub(crate) fn from_reference(reference: &str) -> ProtocolResultValue<Self> {
        if reference.trim().is_empty() {
            return Err(ProtocolError::invalid_configuration(
                "credential reference must not be empty",
            ));
        }
        let digest = Sha256::digest(reference.as_bytes());
        let anonymous = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(Self(anonymous))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for AnonymousCredentialRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("AnonymousCredentialRef")
            .field(&self.0)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CredentialAudit {
    pub kind: CredentialKind,
    pub anonymous_ref: AnonymousCredentialRef,
}

impl std::fmt::Display for CredentialAudit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "credential kind={} ref={}",
            self.kind.as_str(),
            self.anonymous_ref.as_str()
        )
    }
}

#[derive(Clone)]
enum CredentialMaterial {
    Bearer(String),
    NamedHeader { name: HeaderName, value: String },
    FalKey(String),
    GlmJwt(String),
}

#[derive(Clone)]
pub(crate) struct ResolvedCredential {
    audit: CredentialAudit,
    material: CredentialMaterial,
}

impl std::fmt::Debug for ResolvedCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedCredential")
            .field("audit", &self.audit)
            .field("material", &"[REDACTED]")
            .finish()
    }
}

impl ResolvedCredential {
    pub(crate) fn bearer(reference: &str, secret: impl Into<String>) -> ProtocolResultValue<Self> {
        Self::new(
            reference,
            CredentialKind::Bearer,
            CredentialMaterial::Bearer(secret.into()),
        )
    }

    pub(crate) fn named_header(
        reference: &str,
        name: &str,
        secret: impl Into<String>,
    ) -> ProtocolResultValue<Self> {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            ProtocolError::invalid_configuration("credential header name is invalid")
        })?;
        if name == AUTHORIZATION {
            return Err(ProtocolError::invalid_configuration(
                "use bearer or fal credential for the authorization header",
            ));
        }
        Self::new(
            reference,
            CredentialKind::NamedHeader,
            CredentialMaterial::NamedHeader {
                name,
                value: secret.into(),
            },
        )
    }

    pub(crate) fn fal_key(reference: &str, secret: impl Into<String>) -> ProtocolResultValue<Self> {
        Self::new(
            reference,
            CredentialKind::FalKey,
            CredentialMaterial::FalKey(secret.into()),
        )
    }

    pub(crate) fn glm_jwt(
        reference: &str,
        api_key: &str,
        issued_at: SystemTime,
        ttl: Duration,
    ) -> ProtocolResultValue<Self> {
        let token = generate_glm_jwt(api_key, issued_at, ttl)?;
        Self::new(
            reference,
            CredentialKind::GlmJwt,
            CredentialMaterial::GlmJwt(token),
        )
    }

    fn new(
        reference: &str,
        kind: CredentialKind,
        material: CredentialMaterial,
    ) -> ProtocolResultValue<Self> {
        let secret_is_empty = match &material {
            CredentialMaterial::Bearer(secret)
            | CredentialMaterial::FalKey(secret)
            | CredentialMaterial::GlmJwt(secret) => secret.is_empty(),
            CredentialMaterial::NamedHeader { value, .. } => value.is_empty(),
        };
        if secret_is_empty {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "credential material must not be empty",
            ));
        }
        Ok(Self {
            audit: CredentialAudit {
                kind,
                anonymous_ref: AnonymousCredentialRef::from_reference(reference)?,
            },
            material,
        })
    }

    pub(crate) fn audit(&self) -> &CredentialAudit {
        &self.audit
    }

    pub(crate) fn apply(&self, headers: &mut HeaderMap) -> ProtocolResultValue<()> {
        let (name, value) = match &self.material {
            CredentialMaterial::Bearer(secret) | CredentialMaterial::GlmJwt(secret) => {
                (AUTHORIZATION, format!("Bearer {secret}"))
            }
            CredentialMaterial::FalKey(secret) => (AUTHORIZATION, format!("Key {secret}")),
            CredentialMaterial::NamedHeader { name, value } => (name.clone(), value.clone()),
        };
        let value = HeaderValue::from_str(&value).map_err(|_| {
            ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "credential contains invalid header characters",
            )
        })?;
        headers.insert(name, value);
        Ok(())
    }
}

#[derive(Serialize)]
struct GlmJwtHeader<'a> {
    alg: &'a str,
    sign_type: &'a str,
}

#[derive(Serialize)]
struct GlmJwtClaims<'a> {
    api_key: &'a str,
    exp: u128,
    timestamp: u128,
}

fn generate_glm_jwt(
    api_key: &str,
    issued_at: SystemTime,
    ttl: Duration,
) -> ProtocolResultValue<String> {
    let (key_id, secret) = api_key.split_once('.').ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "GLM API key must use the `<id>.<secret>` format",
        )
    })?;
    if key_id.is_empty() || secret.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "GLM API key must contain a non-empty id and secret",
        ));
    }
    if ttl.is_zero() || ttl > Duration::from_secs(3600) {
        return Err(ProtocolError::invalid_configuration(
            "GLM JWT lifetime must be between 1 second and 1 hour",
        ));
    }
    let timestamp = issued_at
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProtocolError::invalid_configuration("GLM JWT time precedes Unix epoch"))?
        .as_millis();
    let expiration = timestamp
        .checked_add(ttl.as_millis())
        .ok_or_else(|| ProtocolError::invalid_configuration("GLM JWT expiration overflow"))?;
    let header = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&GlmJwtHeader {
            alg: "HS256",
            sign_type: "SIGN",
        })
        .map_err(|_| ProtocolError::invalid_configuration("failed to encode GLM JWT header"))?,
    );
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&GlmJwtClaims {
            api_key: key_id,
            exp: expiration,
            timestamp,
        })
        .map_err(|_| ProtocolError::invalid_configuration("failed to encode GLM JWT claims"))?,
    );
    let signing_input = format!("{header}.{claims}");
    let signature = hmac_sha256(secret.as_bytes(), signing_input.as_bytes());
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        inner_pad[index] ^= key_block[index];
        outer_pad[index] ^= key_block[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_debug_and_audit_never_expose_reference_or_secret() {
        let credential = ResolvedCredential::bearer("secret://tenant/key", "plain-secret").unwrap();
        let debug = format!("{credential:?}");
        assert!(!debug.contains("plain-secret"));
        assert!(!debug.contains("secret://tenant/key"));
        assert_eq!(credential.audit().kind, CredentialKind::Bearer);
        assert_eq!(credential.audit().anonymous_ref.as_str().len(), 16);
        let audit = credential.audit().to_string();
        assert!(!audit.contains("plain-secret"));
        assert!(!audit.contains("secret://tenant/key"));
        assert!(audit.contains("credential kind=bearer ref="));
    }

    #[test]
    fn applies_supported_header_schemes() {
        let cases = [
            (
                ResolvedCredential::bearer("ref:a", "token").unwrap(),
                AUTHORIZATION,
                "Bearer token",
            ),
            (
                ResolvedCredential::fal_key("ref:b", "token").unwrap(),
                AUTHORIZATION,
                "Key token",
            ),
            (
                ResolvedCredential::named_header("ref:c", "x-api-key", "token").unwrap(),
                HeaderName::from_static("x-api-key"),
                "token",
            ),
        ];
        for (credential, name, expected) in cases {
            let mut headers = HeaderMap::new();
            credential.apply(&mut headers).unwrap();
            assert_eq!(headers.get(name).unwrap(), expected);
        }
    }

    #[test]
    fn glm_jwt_has_expected_header_claims_and_signature() {
        let issued_at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let token = generate_glm_jwt("key-id.secret", issued_at, Duration::from_secs(300)).unwrap();
        let segments = token.split('.').collect::<Vec<_>>();
        assert_eq!(segments.len(), 3);
        let header: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segments[0]).unwrap()).unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segments[1]).unwrap()).unwrap();
        assert_eq!(
            header,
            serde_json::json!({"alg":"HS256","sign_type":"SIGN"})
        );
        assert_eq!(claims["api_key"], "key-id");
        assert_eq!(claims["timestamp"], 1_700_000_000_000_u64);
        assert_eq!(claims["exp"], 1_700_000_300_000_u64);
        let expected = hmac_sha256(
            b"secret",
            format!("{}.{}", segments[0], segments[1]).as_bytes(),
        );
        assert_eq!(URL_SAFE_NO_PAD.decode(segments[2]).unwrap(), expected);
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_vector() {
        assert_eq!(
            hmac_sha256(&[0x0b; 20], b"Hi There"),
            [
                0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
                0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
                0x2e, 0x32, 0xcf, 0xf7,
            ]
        );
    }
}
