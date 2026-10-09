//! Locating other HomeStations and the keys that may sign for a DID (§4.4 "发现", §5.4, §16.5).
//! The protocol only needs: DID → home (zone and user segment: `cyfs://<zone>/home/<user>/feed`
//! and `/inbox`), zone → HTTP origin, and "may this signer sign for that DID".

use crate::protocol::HomeRef;
use crate::sign::{decoding_key_from_x, kid_fragment};
use crate::users::UserRegistry;
use async_trait::async_trait;
use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Zone hostname hosting `did`.
    async fn zone_of(&self, did: &str) -> Result<String, DirError>;
    /// The HomeStation of `did` (§4.4 发现); `NotFound` when that DID has none.
    async fn home_of(&self, did: &str) -> Result<HomeRef, DirError>;
    /// Known without asking anyone: `did` has no HomeStation (an agent of this zone, a cached
    /// negative answer). Push skips such recipients instead of failing on them.
    async fn known_without_home(&self, _did: &str) -> bool {
        false
    }
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
    /// User segment of the identity's HomeStation in its zone; none = no HomeStation.
    #[serde(default)]
    pub user: Option<String>,
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

    async fn known_without_home(&self, did: &str) -> bool {
        self.identity(did).is_some_and(|i| i.user.is_none())
    }

    async fn home_of(&self, did: &str) -> Result<HomeRef, DirError> {
        let identity = self.identity(did);
        match (identity.and_then(|i| i.zone.as_deref()), identity.and_then(|i| i.user.as_deref())) {
            (Some(zone), Some(user)) => Ok(HomeRef::new(zone, user)),
            _ => Err(DirError::NotFound(format!("{did} has no HomeStation"))),
        }
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

/// System mode: the name service, plus the facts of this zone (its users and devices) and
/// declared origins.
pub struct NameDirectory {
    pub local_zone: String,
    pub local_devices: Vec<String>,
    pub local_users: Arc<UserRegistry>,
    /// Origins declared in settings (`peers`), for zones not reachable by their hostname; the
    /// own zone maps to the local listener so in-zone traffic never leaves the node.
    pub origins: HashMap<String, String>,
    pub http: reqwest::Client,
    /// DID → zone, kept for a while: retries and syncs would otherwise ask the name service each time.
    pub zones: std::sync::Mutex<HashMap<String, (String, std::time::Instant)>>,
    /// DID → home of other zones (`GET /home/?did=`), positive and negative answers.
    pub homes: std::sync::Mutex<HashMap<String, (Result<HomeRef, DirError>, std::time::Instant)>>,
}

const ZONE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

impl NameDirectory {
    pub fn new(local_zone: &str, local_devices: Vec<String>, local_users: Arc<UserRegistry>, origins: HashMap<String, String>) -> Self {
        Self {
            local_zone: local_zone.to_string(),
            local_devices,
            local_users,
            origins,
            http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().expect("http client"),
            zones: Default::default(),
            homes: Default::default(),
        }
    }

    /// The DID names a host inside this zone (`did:web:<x>.<zone>`): a user or agent of ours.
    fn hosted_here(&self, value: &str) -> bool {
        let Ok(parsed) = name_lib::DID::from_str(value) else { return false };
        let host = parsed.to_host_name().trim_end_matches('.').to_ascii_lowercase();
        host == self.local_zone || host.ends_with(&format!(".{}", self.local_zone))
    }

    async fn locate_remote(&self, value: &str, zone: &str) -> Result<HomeRef, DirError> {
        let origin = self.origin_of_zone(zone).await?;
        let url = format!("{}/home/?did={}", origin.trim_end_matches('/'), crate::api::url_encode(value));
        let response = self
            .http
            .get(&url)
            .header("host", zone)
            .send()
            .await
            .map_err(|e| DirError::Unavailable(format!("locate {value}: {e}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(DirError::NotFound(format!("{zone} hosts no HomeStation for {value}")));
        }
        if !response.status().is_success() {
            return Err(DirError::Unavailable(format!("locate {value}: HTTP {}", response.status())));
        }
        let body: Value = response.json().await.map_err(|e| DirError::Unavailable(format!("locate {value}: {e}")))?;
        let user = body.get("user").and_then(Value::as_str).filter(|u| crate::protocol::valid_user_segment(u));
        match user {
            Some(user) if body.get("did").and_then(Value::as_str) == Some(value) => Ok(HomeRef::new(zone, user)),
            _ => Err(DirError::NotFound(format!("{zone} answered no home for {value}"))),
        }
    }
}

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
        if self.local_users.by_did(value).await.is_some() || self.hosted_here(value) {
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

    async fn home_of(&self, value: &str) -> Result<HomeRef, DirError> {
        let local = if self.hosted_here(value) { self.local_users.by_did_fresh(value).await } else { self.local_users.by_did(value).await };
        if let Some(user) = local {
            return Ok(HomeRef::new(&self.local_zone, &user.user));
        }
        let zone = self.zone_of(value).await?;
        if zone == self.local_zone {
            return Err(DirError::NotFound(format!("{value} is not a user of this zone")));
        }
        if let Some((answer, at)) = self.homes.lock().unwrap().get(value) {
            if at.elapsed() < ZONE_CACHE_TTL {
                return answer.clone();
            }
        }
        let answer = self.locate_remote(value, &zone).await;
        if !matches!(answer, Err(DirError::Unavailable(_))) {
            self.homes.lock().unwrap().insert(value.to_string(), (answer.clone(), std::time::Instant::now()));
        }
        answer
    }

    async fn known_without_home(&self, value: &str) -> bool {
        if self.local_users.by_did(value).await.is_some() {
            return false;
        }
        if self.hosted_here(value) {
            return self.local_users.by_did_fresh(value).await.is_none();
        }
        matches!(self.homes.lock().unwrap().get(value), Some((Err(DirError::NotFound(_)), at)) if at.elapsed() < ZONE_CACHE_TTL)
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

    /// The DID itself, a key its document lists, a device it owns, or — zone custody — a
    /// device of the zone that hosts it: a zone signs for all of its users (known risk, §5.4).
    async fn authorizes(&self, principal: &str, signer: &str, kid: &str) -> Result<bool, DirError> {
        if principal == signer {
            return Ok(true);
        }
        if self.local_devices.iter().any(|d| d == signer) && self.local_users.by_did(principal).await.is_some() {
            return Ok(true);
        }
        if let Ok(doc) = resolve_doc(principal).await {
            if lists_signer(&doc, signer, kid) {
                return Ok(true);
            }
        }
        let signer_doc = resolve_doc(signer).await?;
        if signer_doc.get("owner").and_then(Value::as_str) == Some(principal) {
            return Ok(true);
        }
        let device_zone = signer_doc
            .get("zone_did")
            .and_then(Value::as_str)
            .and_then(|z| name_lib::DID::from_str(z).ok())
            .map(|z| z.to_host_name().trim_end_matches('.').to_ascii_lowercase());
        match device_zone {
            Some(zone) => Ok(self.zone_of(principal).await? == zone),
            None => Ok(false),
        }
    }

    async fn content_did_owner(&self, value: &str) -> Result<Option<String>, DirError> {
        let doc = resolve_doc(value).await?;
        Ok(doc.get("owner").and_then(Value::as_str).map(str::to_string))
    }
}
