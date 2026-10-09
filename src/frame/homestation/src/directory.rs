//! Locating other HomeStations and the keys that may sign for a DID (§4.4 "发现", §5.4, §16.5).
//! The protocol only needs: DID → zone (namespace owner and default `cyfs://<zone>/home/feed`
//! and `/home/inbox`), zone → HTTP origin, and "may this signer sign for that DID".

use crate::sign::{decoding_key_from_x, kid_fragment};
use async_trait::async_trait;
use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum DirError {
    NotFound(String),
    /// Resolution may succeed later (name service unreachable, ...).
    Unavailable(String),
}

impl std::fmt::Display for DirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) => write!(f, "not found: {m}"),
            Self::Unavailable(m) => write!(f, "unavailable: {m}"),
        }
    }
}

#[async_trait]
pub trait Directory: Send + Sync {
    /// Zone hostname whose `home/` namespace belongs to `did`.
    async fn zone_of(&self, did: &str) -> Result<String, DirError>;
    /// HTTP origin serving `zone` (`https://<zone>` unless declared otherwise).
    async fn origin_of_zone(&self, zone: &str) -> Result<String, DirError>;
    async fn signer_key(&self, signer: &str, kid: &str) -> Result<DecodingKey, DirError>;
    /// The signer is the DID itself, a key its document lists, or a device it owns.
    async fn authorizes(&self, principal: &str, signer: &str, kid: &str) -> Result<bool, DirError>;
    /// Holder of a content DID used as an entry (E08).
    async fn content_did_owner(&self, did: &str) -> Result<Option<String>, DirError>;
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StaticIdentity {
    pub zone: Option<String>,
    /// Base64url ed25519 public key; `did:dev` DIDs carry theirs in the DID.
    pub public_key_x: Option<String>,
    /// Device DIDs allowed to sign for this identity.
    #[serde(default)]
    pub devices: Vec<String>,
    /// Owner of a content DID.
    pub owner: Option<String>,
}

/// Standalone mode: everything is declared in the config file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StaticDirectory {
    #[serde(default)]
    pub identities: HashMap<String, StaticIdentity>,
    /// zone → origin, e.g. `"bob.local": "http://127.0.0.1:4132"`.
    #[serde(default)]
    pub zones: HashMap<String, String>,
}

impl StaticDirectory {
    fn identity(&self, did: &str) -> Option<&StaticIdentity> {
        self.identities.get(did)
    }
}

fn did_dev_key(did: &str) -> Option<DecodingKey> {
    let x = did.strip_prefix("did:dev:")?;
    decoding_key_from_x(x).ok()
}

#[async_trait]
impl Directory for StaticDirectory {
    async fn zone_of(&self, did: &str) -> Result<String, DirError> {
        self.identity(did)
            .and_then(|i| i.zone.clone())
            .ok_or_else(|| DirError::NotFound(format!("no zone known for {did}")))
    }

    async fn origin_of_zone(&self, zone: &str) -> Result<String, DirError> {
        Ok(self.zones.get(zone).cloned().unwrap_or_else(|| format!("https://{zone}")))
    }

    async fn signer_key(&self, signer: &str, _kid: &str) -> Result<DecodingKey, DirError> {
        if let Some(key) = did_dev_key(signer) {
            return Ok(key);
        }
        let x = self
            .identity(signer)
            .and_then(|i| i.public_key_x.clone())
            .ok_or_else(|| DirError::NotFound(format!("no key known for {signer}")))?;
        decoding_key_from_x(&x).map_err(DirError::NotFound)
    }

    async fn authorizes(&self, principal: &str, signer: &str, _kid: &str) -> Result<bool, DirError> {
        if principal == signer {
            return Ok(true);
        }
        Ok(self.identity(principal).map_or(false, |i| i.devices.iter().any(|d| d == signer)))
    }

    async fn content_did_owner(&self, did: &str) -> Result<Option<String>, DirError> {
        Ok(self.identity(did).and_then(|i| i.owner.clone()))
    }
}

/// System mode: the name service, plus the facts of this zone and declared origins.
pub struct NameDirectory {
    pub local_owner: String,
    pub local_zone: String,
    pub local_devices: Vec<String>,
    /// Origins declared in settings (`peers`), for zones not reachable by their hostname.
    pub origins: HashMap<String, String>,
    /// DID → zone, kept for a while: retries and syncs would otherwise ask the name service each time.
    pub zones: std::sync::Mutex<HashMap<String, (String, std::time::Instant)>>,
}

const ZONE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

fn did(value: &str) -> Result<name_lib::DID, DirError> {
    name_lib::DID::from_str(value).map_err(|e| DirError::NotFound(format!("invalid DID {value}: {e}")))
}

async fn resolve_doc(value: &str) -> Result<Value, DirError> {
    let doc = name_client::resolve_did(&did(value)?, None)
        .await
        .map_err(|e| DirError::Unavailable(format!("resolve {value}: {e}")))?;
    doc.to_json_value().map_err(|e| DirError::NotFound(format!("decode document of {value}: {e}")))
}

fn lists_signer(doc: &Value, signer: &str, kid: &str) -> bool {
    let fragment = kid_fragment(kid);
    doc.get("authentication").and_then(Value::as_array).is_some_and(|items| {
        items.iter().any(|item| {
            let id = item.as_str().or_else(|| item.get("id").and_then(Value::as_str)).unwrap_or("");
            id == kid || id == signer || fragment.as_deref() == Some(id)
        })
    })
}

#[async_trait]
impl Directory for NameDirectory {
    async fn zone_of(&self, value: &str) -> Result<String, DirError> {
        if value == self.local_owner {
            return Ok(self.local_zone.clone());
        }
        if let Some((zone, at)) = self.zones.lock().unwrap().get(value) {
            if at.elapsed() < ZONE_CACHE_TTL {
                return Ok(zone.clone());
            }
        }
        let parsed = did(value)?;
        let zone = match name_client::resolve_owner_document(&parsed).await {
            Ok(doc) => doc.get_default_zone_did().map(|z| z.to_host_name()),
            Err(e) => {
                log::debug!("owner document of {value} unavailable: {e}");
                None
            }
        }
        .unwrap_or_else(|| parsed.to_host_name())
        .trim_end_matches('.')
        .to_ascii_lowercase();
        self.zones.lock().unwrap().insert(value.to_string(), (zone.clone(), std::time::Instant::now()));
        Ok(zone)
    }

    async fn origin_of_zone(&self, zone: &str) -> Result<String, DirError> {
        Ok(self.origins.get(zone).cloned().unwrap_or_else(|| format!("https://{zone}")))
    }

    async fn signer_key(&self, signer: &str, kid: &str) -> Result<DecodingKey, DirError> {
        if let Some(key) = did_dev_key(signer) {
            return Ok(key);
        }
        let fragment = kid_fragment(kid);
        name_client::resolve_auth_key(&did(signer)?, fragment.as_deref())
            .await
            .map_err(|e| DirError::Unavailable(format!("key of {signer}: {e}")))
    }

    async fn authorizes(&self, principal: &str, signer: &str, kid: &str) -> Result<bool, DirError> {
        if principal == signer {
            return Ok(true);
        }
        if principal == self.local_owner && self.local_devices.iter().any(|d| d == signer) {
            return Ok(true);
        }
        if let Ok(doc) = resolve_doc(principal).await {
            if lists_signer(&doc, signer, kid) {
                return Ok(true);
            }
        }
        let signer_doc = resolve_doc(signer).await?;
        Ok(signer_doc.get("owner").and_then(Value::as_str) == Some(principal))
    }

    async fn content_did_owner(&self, value: &str) -> Result<Option<String>, DirError> {
        let doc = resolve_doc(value).await?;
        Ok(doc.get("owner").and_then(Value::as_str).map(str::to_string))
    }
}
