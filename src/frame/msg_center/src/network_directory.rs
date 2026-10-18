//! What msg-center learns about DIDs outside its own records: the Zone a DID
//! lives in, its public profile, and the Zone of a signing device. Native
//! delivery routes by it (G1), lookup describes candidates with it, and the
//! Zone-custody signature rule (G3) compares Zones with it.

use async_trait::async_trait;
use buckyos_api::LookupKind;
use jsonwebtoken::EncodingKey;
use name_lib::DID;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const DIRECTORY_CACHE_TTL: Duration = Duration::from_secs(600);
/// A name service that does not answer is asked again only after this.
const DIRECTORY_UNAVAILABLE_TTL: Duration = Duration::from_secs(30);
const DIRECTORY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryError {
    /// The name service answered: there is no such document.
    NotFound(String),
    /// No answer right now; worth retrying.
    Unavailable(String),
}

impl std::fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(reason) => write!(f, "not found: {reason}"),
            Self::Unavailable(reason) => write!(f, "unavailable: {reason}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidProfile {
    pub kind: LookupKind,
    pub name: Option<String>,
    pub avatar: Option<String>,
    /// Host name of the Zone the DID belongs to.
    pub zone: Option<String>,
}

#[async_trait]
pub trait NetworkDirectory: Send + Sync {
    async fn describe(&self, did: &DID) -> Result<DidProfile, DirectoryError>;

    async fn zone_of(&self, did: &DID) -> Result<Option<String>, DirectoryError> {
        Ok(self.describe(did).await?.zone)
    }
}

/// Production directory: DID documents through name_client, answers kept
/// for ten minutes (failures to answer are not kept).
#[derive(Default)]
pub struct NameServiceDirectory {
    cache: Mutex<HashMap<String, (Result<DidProfile, DirectoryError>, Instant)>>,
}

fn host_of(did: &DID) -> String {
    did.to_host_name().trim_end_matches('.').to_ascii_lowercase()
}

fn zone_host(value: &Value) -> Option<String> {
    value
        .as_str()
        .and_then(|raw| DID::from_str(raw).ok())
        .map(|did| host_of(&did))
}

fn text(doc: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| doc.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn profile_from_document(did: &DID, doc: &Value) -> DidProfile {
    let declared = doc
        .get("entity_type")
        .or_else(|| doc.get("type"))
        .or_else(|| doc.get("doc_type"))
        .and_then(Value::as_str);
    let context = doc.get("@context").map(Value::to_string).unwrap_or_default();
    let kind = match declared {
        Some("user" | "owner") => LookupKind::User,
        Some("agent") => LookupKind::Agent,
        Some("device") => LookupKind::Device,
        Some("group") => LookupKind::Group,
        Some("zone") => LookupKind::Zone,
        _ if context.contains("ns/owner") => LookupKind::User,
        _ if context.contains("ns/agent") => LookupKind::Agent,
        _ if context.contains("ns/device") => LookupKind::Device,
        _ if context.contains("ns/group") => LookupKind::Group,
        _ if context.contains("ns/zone") => LookupKind::Zone,
        _ if doc.get("binded_zone_list").is_some() => LookupKind::User,
        _ => LookupKind::Unknown,
    };
    let zone = match kind {
        LookupKind::User => doc
            .get("binded_zone_list")
            .and_then(Value::as_array)
            .and_then(|zones| zones.first())
            .and_then(zone_host),
        LookupKind::Zone => Some(host_of(did)),
        LookupKind::Group => doc.get("host").and_then(zone_host),
        _ => doc
            .get("zone_did")
            .or_else(|| doc.get("zone"))
            .and_then(zone_host),
    };
    DidProfile {
        kind,
        name: text(doc, &["display_name", "full_name", "displayName", "name"]),
        avatar: text(doc, &["avatar", "icon"]),
        zone,
    }
}

impl NameServiceDirectory {
    async fn fetch(did: &DID) -> Result<DidProfile, DirectoryError> {
        let resolve = |doc_type| async move {
            match tokio::time::timeout(DIRECTORY_TIMEOUT, name_client::resolve_did(did, doc_type)).await {
                Ok(Ok(doc)) => Ok(doc),
                Ok(Err(name_lib::NSError::NotFound(reason))) => Err(DirectoryError::NotFound(reason)),
                // Key-class DIDs (`did:dev`) and malformed ones have no document to wait for.
                Ok(Err(name_lib::NSError::InvalidDID(reason))) => Err(DirectoryError::NotFound(reason)),
                Ok(Err(error)) => Err(DirectoryError::Unavailable(error.to_string())),
                Err(_) => Err(DirectoryError::Unavailable("timeout".into())),
            }
        };
        // The owner document first; a name service that timed out is not
        // asked a second time.
        let doc = match resolve(Some(name_lib::DidDocType::Owner)).await {
            Ok(doc) => doc,
            Err(DirectoryError::Unavailable(reason)) if reason == "timeout" => {
                return Err(DirectoryError::Unavailable(reason))
            }
            Err(_) => resolve(None).await?,
        };
        let doc = doc
            .to_json_value()
            .map_err(|error| DirectoryError::NotFound(error.to_string()))?;
        Ok(profile_from_document(did, &doc))
    }
}

#[async_trait]
impl NetworkDirectory for NameServiceDirectory {
    async fn describe(&self, did: &DID) -> Result<DidProfile, DirectoryError> {
        let key = did.to_string();
        if let Some((answer, at)) = self.cache.lock().unwrap().get(&key) {
            let ttl = match answer {
                Err(DirectoryError::Unavailable(_)) => DIRECTORY_UNAVAILABLE_TTL,
                _ => DIRECTORY_CACHE_TTL,
            };
            if at.elapsed() < ttl {
                return answer.clone();
            }
        }
        let answer = Self::fetch(did).await;
        self.cache
            .lock()
            .unwrap()
            .insert(key, (answer.clone(), Instant::now()));
        answer
    }
}

pub struct DirectorySlot(RwLock<Arc<dyn NetworkDirectory>>);

impl DirectorySlot {
    pub(crate) fn get(&self) -> Arc<dyn NetworkDirectory> {
        self.0.read().unwrap().clone()
    }

    #[cfg(test)]
    pub(crate) fn set(&self, directory: Arc<dyn NetworkDirectory>) {
        *self.0.write().unwrap() = directory;
    }
}

impl Default for DirectorySlot {
    fn default() -> Self {
        Self(RwLock::new(Arc::new(NameServiceDirectory::default())))
    }
}

impl std::fmt::Debug for DirectorySlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DirectorySlot")
    }
}

/// The OOD device key msg-center signs native messages of its Zone's users
/// with (Zone custody, `Message Center.md` §4.5).
#[derive(Clone)]
pub struct ZoneSigner {
    pub key: EncodingKey,
    /// `<device_did>#<fragment>` written into the JWT header.
    pub kid: String,
}

#[derive(Default)]
pub struct ZoneSignerSlot(RwLock<Option<ZoneSigner>>);

impl ZoneSignerSlot {
    pub(crate) fn get(&self) -> Option<ZoneSigner> {
        self.0.read().unwrap().clone()
    }

    pub(crate) fn set(&self, signer: ZoneSigner) {
        *self.0.write().unwrap() = Some(signer);
    }
}

impl std::fmt::Debug for ZoneSignerSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ZoneSignerSlot")
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// DID → profile, as two test Zones would publish them.
    #[derive(Default)]
    pub struct StaticDirectory(pub Mutex<HashMap<String, DidProfile>>);

    impl StaticDirectory {
        pub fn with(entries: Vec<(DID, LookupKind, &str, &str)>) -> Arc<Self> {
            let map = entries
                .into_iter()
                .map(|(did, kind, name, zone)| {
                    (
                        did.to_string(),
                        DidProfile {
                            kind,
                            name: Some(name.to_string()),
                            avatar: None,
                            zone: Some(zone.to_string()),
                        },
                    )
                })
                .collect();
            Arc::new(Self(Mutex::new(map)))
        }
    }

    #[async_trait]
    impl NetworkDirectory for StaticDirectory {
        async fn describe(&self, did: &DID) -> Result<DidProfile, DirectoryError> {
            self.0
                .lock()
                .unwrap()
                .get(&did.to_string())
                .cloned()
                .ok_or_else(|| DirectoryError::NotFound(did.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documents_describe_kind_name_and_zone() {
        let alice = DID::new("bns", "alice");
        let profile = profile_from_document(
            &alice,
            &json!({"id":"did:bns:alice","name":"alice","display_name":"Alice",
                "binded_zone_list":["did:web:alice.example"],"avatar":"https://a/p.png"}),
        );
        assert_eq!(profile.kind, LookupKind::User);
        assert_eq!(profile.name.as_deref(), Some("Alice"));
        assert_eq!(profile.zone.as_deref(), Some("alice.example"));
        let device = DID::new("dev", "abc");
        let profile = profile_from_document(
            &device,
            &json!({"id":"did:dev:abc","entity_type":"device","zone_did":"did:web:alice.example"}),
        );
        assert_eq!(profile.kind, LookupKind::Device);
        assert_eq!(profile.zone.as_deref(), Some("alice.example"));
    }
}
