//! Locating other HomeStations and the keys that may sign for a DID (§4.4 "发现", §5.4, §16.5).
//! The protocol only needs: DID → home (zone and user segment: `https://<zone>/home/<user>/feed`
//! and `/inbox`), zone → HTTP origin, and "may this signer sign for that DID".

use crate::protocol::HomeRef;
use crate::sign::decoding_key_from_x;
use buckyos_api::{
    canonical_zone_of, check_custody, resolve_signer_key, zone_hostname, CustodyError, CustodyResolver, NameServiceResolver,
    ResolveFailure,
};
use crate::users::UserRegistry;
use async_trait::async_trait;
use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use serde_json::{json, Value};
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

    async fn authorizes(&self, principal: &str, signer: &str, kid: &str) -> Result<bool, DirError> {
        custody_authorizes(self, principal, signer, kid, None).await
    }

    async fn content_did_owner(&self, did: &str) -> Result<Option<String>, DirError> {
        Ok(self.identity(did).and_then(|i| i.owner.clone()))
    }
}

/// The declared identities as DID documents, so the one custody rule (`buckyos_api::check_custody`)
/// judges standalone zones too: a key in `#main_key`, the devices that sign for it listed, its zone.
#[async_trait]
impl CustodyResolver for StaticDirectory {
    async fn resolve(&self, did: &name_lib::DID, _doc_type: Option<name_lib::DidDocType>) -> Result<name_lib::EncodedDocument, ResolveFailure> {
        let id = did.to_string();
        let identity = self.identity(&id).ok_or_else(|| ResolveFailure::NotFound(id.clone()))?;
        let mut authentication: Vec<Value> = identity.devices.iter().map(|d| Value::String(format!("{d}#main_key"))).collect();
        let mut methods = Vec::new();
        if let Some(x) = &identity.public_key_x {
            authentication.push(Value::String("#main_key".into()));
            methods.push(json!({"type": "Ed25519VerificationKey2020", "id": "#main_key", "controller": id,
                "publicKeyJwk": {"kty": "OKP", "crv": "Ed25519", "x": x}}));
        }
        let zone = identity.zone.as_ref().map(|zone| format!("did:web:{zone}"));
        Ok(name_lib::EncodedDocument::JsonLd(json!({
            "id": id, "verificationMethod": methods, "authentication": authentication,
            "exp": 4102444800u64, "iat": 1, "owner": id, "device_type": "server", "name": id,
            "zone_did": zone, "binded_zone_list": zone.into_iter().collect::<Vec<_>>(),
        })))
    }
}

/// `buckyos_api::check_custody` answered as `Directory::authorizes`: `local_zone` is this zone
/// when the principal is known to live here.
async fn custody_authorizes(
    resolver: &dyn CustodyResolver,
    principal: &str,
    signer: &str,
    kid: &str,
    local_zone: Option<name_lib::DID>,
) -> Result<bool, DirError> {
    let principal = did(principal)?;
    let kid = if kid.starts_with(signer) { kid.to_string() } else { format!("{signer}#main_key") };
    let key = match resolve_signer_key(resolver, &kid).await {
        Ok(key) => key,
        Err(CustodyError::Unavailable(e)) => return Err(DirError::Unavailable(e.to_string())),
        Err(_) => return Ok(false),
    };
    match check_custody(resolver, &key, &principal, local_zone).await {
        Ok(_) => Ok(true),
        Err(CustodyError::Unavailable(e)) => Err(DirError::Unavailable(e.to_string())),
        Err(_) => Ok(false),
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
    /// DID documents for keys and custody: the name service.
    pub custody: Arc<dyn CustodyResolver>,
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
            custody: Arc::new(NameServiceResolver),
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
        // OwnerDocument → canonical Zone → ZoneDocument hostname, as every service resolves it.
        let parsed = did(value)?;
        let resolver = self.custody.as_ref();
        let unavailable = |e: ResolveFailure| match e {
            ResolveFailure::NotFound(m) => DirError::NotFound(m),
            e => DirError::Unavailable(e.to_string()),
        };
        let zone_did = canonical_zone_of(resolver, &parsed, None)
            .await
            .map_err(unavailable)?
            .ok_or_else(|| DirError::NotFound(format!("{value} names no zone")))?;
        let zone = zone_hostname(resolver, &zone_did).await.map_err(unavailable)?;
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
        let kid = if kid.starts_with(signer) { kid.to_string() } else { format!("{signer}#main_key") };
        match resolve_signer_key(self.custody.as_ref(), &kid).await {
            Ok(key) => Ok(key.key),
            Err(CustodyError::Unavailable(e)) => Err(DirError::Unavailable(format!("document of {signer}: {e}"))),
            Err(_) => Err(DirError::NotFound(format!("{kid} is not a key of {signer}"))),
        }
    }

    /// The one custody rule (`buckyos_api::check_custody`): the DID itself, a key its document
    /// lists, or — zone custody — a device of the zone that hosts it (known risk, §5.4). A user
    /// of this zone is hosted here whatever the name service says.
    async fn authorizes(&self, principal: &str, signer: &str, kid: &str) -> Result<bool, DirError> {
        let local = (self.local_users.by_did(principal).await.is_some() || self.hosted_here(principal))
            .then(|| name_lib::DID::new("web", &self.local_zone));
        custody_authorizes(self.custody.as_ref(), principal, signer, kid, local).await
    }

    async fn content_did_owner(&self, value: &str) -> Result<Option<String>, DirError> {
        let doc = resolve_doc(value).await?;
        Ok(doc.get("owner").and_then(Value::as_str).map(str::to_string))
    }
}

#[cfg(test)]
mod custody_tests {
    use super::*;
    use buckyos_api::custody_cases::{cases, resolver, Outcome};

    /// V-11: HomeStation judges signers by the shared custody table.
    #[tokio::test]
    async fn signers_follow_the_shared_custody_table() {
        let users = Arc::new(UserRegistry::new(
            Box::new(crate::users::StaticUsers(std::sync::Mutex::new(vec![]))),
            std::time::Duration::from_secs(60),
        ));
        let mut directory = NameDirectory::new("target.example", vec![], users, HashMap::new());
        directory.custody = Arc::new(resolver());
        for case in cases() {
            let kid = format!("{}#main_key", case.device);
            let outcome = match directory.authorizes(case.principal, case.device, &kid).await {
                Ok(true) => Outcome::Allowed,
                Ok(false) => Outcome::Refused,
                Err(DirError::Unavailable(_)) => Outcome::Retry,
                Err(DirError::NotFound(_)) => Outcome::Refused,
            };
            assert_eq!(outcome, case.outcome, "{} for {}", case.device, case.principal);
        }
    }
}
