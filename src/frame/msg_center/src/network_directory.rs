//! What msg-center learns about DIDs outside its own records: the Zone a DID
//! lives in, its public profile, and the Zone of a signing device. Native
//! delivery routes by it (G1), lookup describes candidates with it, and the
//! Zone-custody signature rule (G3) compares Zones with it.

use async_trait::async_trait;
use buckyos_api::{
    CrossZoneClient, CustodyResolver, DeviceSigner, LookupKind, NameServiceResolver,
};
use name_lib::DID;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const DIRECTORY_CACHE_TTL: Duration = Duration::from_secs(600);
/// A name service that does not answer is asked again only after this.
const DIRECTORY_UNAVAILABLE_TTL: Duration = Duration::from_secs(30);
const DIRECTORY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryError {
    /// The name service answered: there is no such document.
    NotFound(String),
    /// No answer right now; worth retrying.
    Unavailable(String),
    /// The name service did not answer in time; worth retrying.
    Timeout,
}

impl std::fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(reason) => write!(f, "not found: {reason}"),
            Self::Unavailable(reason) => write!(f, "unavailable: {reason}"),
            Self::Timeout => f.write_str("timeout"),
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
    async fn describe(
        &self,
        did: &DID,
        refresh_negative: bool,
    ) -> Result<DidProfile, DirectoryError>;

    async fn zone_of(&self, did: &DID) -> Result<Option<String>, DirectoryError> {
        Ok(self.describe(did, false).await?.zone)
    }
}

/// Production directory: DID documents through name_client.
#[derive(Default)]
pub struct NameServiceDirectory {
    cache: Mutex<HashMap<String, (Result<DidProfile, DirectoryError>, Instant)>>,
    client: Option<Arc<name_client::NameClient>>,
}

fn host_of(did: &DID) -> String {
    did.to_host_name()
        .trim_end_matches('.')
        .to_ascii_lowercase()
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
    let context = doc
        .get("@context")
        .map(Value::to_string)
        .unwrap_or_default();
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
        LookupKind::User => match doc
            .get("binded_zone_list")
            .and_then(Value::as_array)
            .and_then(|zones| zones.first())
        {
            Some(zone) => zone_host(zone),
            None => Some(host_of(did)),
        },
        LookupKind::Zone => Some(host_of(did)),
        LookupKind::Group => doc.get("host").and_then(zone_host),
        _ => doc
            .get("zone_did")
            .or_else(|| doc.get("zone"))
            .and_then(zone_host),
    };
    let profile = if kind == LookupKind::Group {
        doc.get("profile")
            .filter(|value| value.is_object())
            .unwrap_or(doc)
    } else {
        doc
    };
    DidProfile {
        kind,
        name: text(
            profile,
            &["display_name", "full_name", "displayName", "name"],
        ),
        avatar: text(profile, &["avatar", "icon"]),
        zone,
    }
}

impl NameServiceDirectory {
    async fn fetch(&self, did: &DID, refresh_negative: bool) -> Result<DidProfile, DirectoryError> {
        let client = self
            .client
            .as_deref()
            .or_else(|| name_client::get_name_client())
            .ok_or_else(|| DirectoryError::NotFound("Name client not found".into()))?;
        let resolve = |doc_type| async move {
            let resolve = async {
                if refresh_negative {
                    let revision = name_client::Revision::current();
                    let opts = client.default_resolve_opts().with_force_authority_query();
                    let result = client
                        .resolve_did_ex(did, doc_type, revision.clone(), opts)
                        .await?;
                    name_client::select_or_error(&result, &revision)
                } else {
                    client.resolve_did(did, doc_type).await
                }
            };
            match tokio::time::timeout(DIRECTORY_TIMEOUT, resolve).await {
                Ok(Ok(doc)) => Ok(doc),
                Ok(Err(name_lib::NSError::NotFound(reason))) => {
                    Err(DirectoryError::NotFound(reason))
                }
                // Key-class DIDs (`did:dev`) and malformed ones have no document to wait for.
                Ok(Err(name_lib::NSError::InvalidDID(reason))) => {
                    Err(DirectoryError::NotFound(reason))
                }
                Ok(Err(name_lib::NSError::DeadlineExceeded(_))) | Err(_) => {
                    Err(DirectoryError::Timeout)
                }
                Ok(Err(error)) => Err(DirectoryError::Unavailable(error.to_string())),
            }
        };
        Self::fetch_with_resolver(did, resolve).await
    }

    async fn fetch_with_resolver<F, Fut>(
        did: &DID,
        mut resolve: F,
    ) -> Result<DidProfile, DirectoryError>
    where
        F: FnMut(Option<name_lib::DidDocType>) -> Fut,
        Fut: std::future::Future<Output = Result<name_lib::EncodedDocument, DirectoryError>>,
    {
        let profile = |doc: name_lib::EncodedDocument| {
            let doc = doc
                .to_json_value()
                .map_err(|error| DirectoryError::NotFound(error.to_string()))?;
            Ok(profile_from_document(did, &doc))
        };
        // A child name (`g1.alice`) usually publishes one document type: its did.json
        // answers in one query. A Zone answer still asks for the owner it may host.
        let mut default = None;
        if did.upper_did().is_some() {
            match resolve(None).await.and_then(profile) {
                Ok(found) if found.kind != LookupKind::Zone => return Ok(found),
                Err(DirectoryError::Timeout) => return Err(DirectoryError::Timeout),
                answer => default = Some(answer),
            }
        }
        for doc_type in [name_lib::DidDocType::Owner, name_lib::DidDocType::Device] {
            match resolve(Some(doc_type)).await {
                Ok(doc) => return profile(doc),
                Err(DirectoryError::Timeout) => return Err(DirectoryError::Timeout),
                Err(_) => {}
            }
        }
        let default = match default {
            Some(answer) => answer,
            None => resolve(None).await.and_then(profile),
        };
        let default_error = match default {
            Ok(found) => return Ok(found),
            Err(DirectoryError::Timeout) => return Err(DirectoryError::Timeout),
            Err(error) => error,
        };
        let doc = resolve(Some(name_lib::DidDocType::custom("group")))
            .await
            .map_err(|error| match (&error, &default_error) {
                (DirectoryError::NotFound(_), DirectoryError::Unavailable(_)) => default_error,
                _ => error,
            })?;
        profile(doc)
    }
}

#[async_trait]
impl NetworkDirectory for NameServiceDirectory {
    async fn describe(
        &self,
        did: &DID,
        refresh_negative: bool,
    ) -> Result<DidProfile, DirectoryError> {
        let key = did.to_string();
        if let Some((answer, at)) = self.cache.lock().unwrap().get(&key) {
            let ttl = match answer {
                Err(DirectoryError::Unavailable(_) | DirectoryError::Timeout) => {
                    DIRECTORY_UNAVAILABLE_TTL
                }
                _ => DIRECTORY_CACHE_TTL,
            };
            if at.elapsed() < ttl && !(refresh_negative && answer.is_err()) {
                return answer.clone();
            }
        }
        let answer = self.fetch(did, refresh_negative).await;
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
/// with (Zone custody, `Message Center.md` §4.5): the runtime's device signer.
#[derive(Default)]
pub struct ZoneSignerSlot(RwLock<Option<DeviceSigner>>);

impl ZoneSignerSlot {
    pub(crate) fn get(&self) -> Option<DeviceSigner> {
        self.0.read().unwrap().clone()
    }

    pub(crate) fn set(&self, signer: DeviceSigner) {
        *self.0.write().unwrap() = Some(signer);
    }
}

impl std::fmt::Debug for ZoneSignerSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ZoneSignerSlot")
    }
}

/// Where signing keys and custody facts come from: the name service.
pub struct CustodySlot(RwLock<Arc<dyn CustodyResolver>>);

impl CustodySlot {
    pub(crate) fn get(&self) -> Arc<dyn CustodyResolver> {
        self.0.read().unwrap().clone()
    }

    #[cfg(test)]
    pub(crate) fn set(&self, resolver: Arc<dyn CustodyResolver>) {
        *self.0.write().unwrap() = resolver;
    }
}

impl Default for CustodySlot {
    fn default() -> Self {
        Self(RwLock::new(Arc::new(NameServiceResolver)))
    }
}

impl std::fmt::Debug for CustodySlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustodySlot")
    }
}

/// The client into other Zones; none until the device key is loaded.
#[derive(Default)]
pub struct CrossZoneSlot(RwLock<Option<Arc<CrossZoneClient>>>);

impl CrossZoneSlot {
    pub(crate) fn get(&self) -> Option<Arc<CrossZoneClient>> {
        self.0.read().unwrap().clone()
    }

    pub(crate) fn set(&self, client: Arc<CrossZoneClient>) {
        *self.0.write().unwrap() = Some(client);
    }
}

impl std::fmt::Debug for CrossZoneSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CrossZoneSlot")
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use name_client::{
        Carrier, DefaultDocType, DocumentStatus, NameClient, NameClientConfig, PublishInfo,
        ResolveOpts, Revision, Source, SourceAnswer, SourceRef, SourceTier,
    };
    use name_lib::{DidDocType, EncodedDocument};

    #[derive(Default)]
    pub struct GroupDocumentSource {
        pub document: Mutex<Option<Value>>,
        pub calls: Mutex<Vec<bool>>,
    }

    impl GroupDocumentSource {
        pub fn directory(self: &Arc<Self>) -> (Arc<NameServiceDirectory>, Arc<NameClient>) {
            let client = Arc::new(NameClient::new(NameClientConfig::for_test()));
            client.set_method_authority("web", self.clone());
            let directory = Arc::new(NameServiceDirectory {
                client: Some(client.clone()),
                ..Default::default()
            });
            (directory, client)
        }
    }

    #[async_trait]
    impl Source for GroupDocumentSource {
        fn id(&self) -> String {
            "group-authority".into()
        }

        async fn default_doc_type(&self, _: &DID, _: &ResolveOpts) -> Option<DefaultDocType> {
            Some(DefaultDocType::Known {
                doc_type: DidDocType::custom("group"),
                reasons: vec![],
            })
        }

        async fn get_document(
            &self,
            did: &DID,
            doc_type: &DidDocType,
            revision: &Revision,
            opts: &ResolveOpts,
        ) -> SourceAnswer {
            self.calls.lock().unwrap().push(opts.force_authority_query);
            let source = SourceRef::provider(self.id(), SourceTier::Authority);
            if *doc_type == DidDocType::custom("group") {
                if let Some(document) = self.document.lock().unwrap().clone() {
                    return SourceAnswer::new(source)
                        .with_body(EncodedDocument::JsonLd(document), Carrier::Body);
                }
            }
            SourceAnswer::new(source).with_publish_info(
                PublishInfo::new(did.clone(), doc_type.clone(), revision.clone()).with_status(
                    &DocumentStatus::Missing,
                    None,
                    None,
                ),
            )
        }
    }

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
        async fn describe(&self, did: &DID, _: bool) -> Result<DidProfile, DirectoryError> {
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
    use name_lib::{DidDocType, EncodedDocument};
    use serde_json::json;

    #[tokio::test]
    async fn explicit_lookup_refreshes_negative_caches_and_keeps_positive_answers() {
        let did = DID::new("web", "club.host.example");
        for error in [
            DirectoryError::NotFound("not published".into()),
            DirectoryError::Unavailable("offline".into()),
            DirectoryError::Timeout,
        ] {
            let source = Arc::new(test_support::GroupDocumentSource::default());
            let (directory, client) = source.directory();
            assert!(matches!(
                directory.describe(&did, false).await,
                Err(DirectoryError::NotFound(_))
            ));
            directory
                .cache
                .lock()
                .unwrap()
                .insert(did.to_string(), (Err(error.clone()), Instant::now()));
            let mut doc = group_doc(&did).to_json_value().unwrap();
            doc["iat"] = json!(buckyos_kit::buckyos_get_unix_timestamp());
            doc["exp"] = json!(buckyos_kit::buckyos_get_unix_timestamp() + 3600);
            *source.document.lock().unwrap() = Some(doc);
            source.calls.lock().unwrap().clear();
            assert!(client.resolve_did(&did, None).await.is_err());
            assert_eq!(directory.describe(&did, false).await, Err(error));
            assert!(source.calls.lock().unwrap().is_empty());

            let profile = directory.describe(&did, true).await.unwrap();
            assert_eq!(profile.kind, LookupKind::Group);
            assert_eq!(profile.zone.as_deref(), Some("host.example"));
            assert_eq!(*source.calls.lock().unwrap(), vec![true]);
            source.calls.lock().unwrap().clear();
            assert_eq!(directory.describe(&did, false).await.unwrap(), profile);
            assert_eq!(directory.describe(&did, true).await.unwrap(), profile);
            assert!(client.resolve_did(&did, None).await.is_ok());
            assert!(source.calls.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn explicit_lookup_refreshes_lower_cache_without_a_directory_entry() {
        let did = DID::new("web", "club.host.example");
        let source = Arc::new(test_support::GroupDocumentSource::default());
        let (directory, client) = source.directory();
        assert!(client.resolve_did(&did, None).await.is_err());
        let mut doc = group_doc(&did).to_json_value().unwrap();
        doc["iat"] = json!(buckyos_kit::buckyos_get_unix_timestamp());
        doc["exp"] = json!(buckyos_kit::buckyos_get_unix_timestamp() + 3600);
        *source.document.lock().unwrap() = Some(doc);
        source.calls.lock().unwrap().clear();
        assert_eq!(
            directory.describe(&did, true).await.unwrap().kind,
            LookupKind::Group
        );
        assert_eq!(*source.calls.lock().unwrap(), vec![true]);
    }

    #[tokio::test]
    async fn explicit_lookup_still_rejects_a_document_for_another_did() {
        let did = DID::new("web", "club.host.example");
        let source = Arc::new(test_support::GroupDocumentSource::default());
        let (directory, _) = source.directory();
        assert!(directory.describe(&did, false).await.is_err());
        let mut doc = group_doc(&DID::new("web", "other.host.example"))
            .to_json_value()
            .unwrap();
        doc["iat"] = json!(buckyos_kit::buckyos_get_unix_timestamp());
        doc["exp"] = json!(buckyos_kit::buckyos_get_unix_timestamp() + 3600);
        *source.document.lock().unwrap() = Some(doc);
        assert!(matches!(
            directory.describe(&did, true).await,
            Err(DirectoryError::NotFound(_))
        ));
        assert!(directory.describe(&did, false).await.is_err());
    }

    fn group_doc(did: &DID) -> EncodedDocument {
        EncodedDocument::JsonLd(json!({
            "id": did,
            "entity_type": "group",
            "host": "did:web:host.example",
            "profile": {"name": "Club", "avatar": "https://host.example/club.png"}
        }))
    }

    #[tokio::test]
    async fn a_child_did_is_answered_by_its_did_json_in_one_query() {
        let did = DID::new("web", "club.host.example");
        let mut requested = Vec::new();
        let profile = NameServiceDirectory::fetch_with_resolver(&did, |doc_type| {
            requested.push(doc_type.clone());
            std::future::ready(if doc_type.is_none() {
                Ok(group_doc(&did))
            } else {
                Err(DirectoryError::Timeout)
            })
        })
        .await
        .unwrap();
        assert_eq!(requested, vec![None]);
        assert_eq!(profile.kind, LookupKind::Group);
        assert_eq!(profile.zone.as_deref(), Some("host.example"));
    }

    #[tokio::test]
    async fn a_child_without_did_json_still_discovers_its_typed_document() {
        let did = DID::new("web", "club.host.example");
        let mut requested = Vec::new();
        let profile = NameServiceDirectory::fetch_with_resolver(&did, |doc_type| {
            requested.push(doc_type.clone());
            std::future::ready(if doc_type == Some(DidDocType::custom("group")) {
                Ok(group_doc(&did))
            } else {
                Err(DirectoryError::NotFound("not published".into()))
            })
        })
        .await
        .unwrap();
        assert_eq!(
            requested,
            vec![
                None,
                Some(DidDocType::Owner),
                Some(DidDocType::Device),
                Some(DidDocType::custom("group")),
            ]
        );
        assert_eq!(profile.kind, LookupKind::Group);
        assert_eq!(profile.name.as_deref(), Some("Club"));
        assert_eq!(
            profile.avatar.as_deref(),
            Some("https://host.example/club.png")
        );
        assert_eq!(profile.zone.as_deref(), Some("host.example"));
    }

    #[tokio::test]
    async fn owner_document_keeps_precedence_over_other_document_types() {
        let did = DID::new("web", "owner.example");
        let mut requested = Vec::new();
        let profile = NameServiceDirectory::fetch_with_resolver(&did, |doc_type| {
            requested.push(doc_type);
            std::future::ready(Ok(EncodedDocument::JsonLd(json!({
                "id": did, "entity_type": "owner", "name": "Owner"
            }))))
        })
        .await
        .unwrap();
        assert_eq!(requested, vec![Some(DidDocType::Owner)]);
        assert_eq!(profile.kind, LookupKind::User);
        assert_eq!(profile.name.as_deref(), Some("Owner"));
    }

    #[tokio::test]
    async fn missing_typed_documents_still_allow_default_document_resolution() {
        let did = DID::new("web", "zone.example");
        let mut requested = Vec::new();
        let profile = NameServiceDirectory::fetch_with_resolver(&did, |doc_type| {
            requested.push(doc_type.clone());
            std::future::ready(if doc_type.is_none() {
                Ok(EncodedDocument::JsonLd(
                    json!({"id": did, "entity_type": "zone"}),
                ))
            } else {
                Err(DirectoryError::NotFound("not published".into()))
            })
        })
        .await
        .unwrap();
        assert_eq!(requested.last(), Some(&None));
        assert_eq!(requested.len(), 3);
        assert_eq!(profile.kind, LookupKind::Zone);
        assert_eq!(profile.zone.as_deref(), Some("zone.example"));
    }

    #[tokio::test]
    async fn a_timeout_stops_document_type_fallback() {
        let did = DID::new("web", "unavailable.example");
        for timeout_at in 0..4 {
            let mut calls = 0;
            let result = NameServiceDirectory::fetch_with_resolver(&did, |_| {
                let error = if calls == timeout_at {
                    DirectoryError::Timeout
                } else {
                    DirectoryError::NotFound("not published".into())
                };
                calls += 1;
                std::future::ready(Err(error))
            })
            .await;
            assert_eq!(result.unwrap_err(), DirectoryError::Timeout);
            assert_eq!(calls, timeout_at + 1);
        }
    }

    #[tokio::test]
    async fn missing_group_does_not_hide_unavailable_default_document() {
        let did = DID::new("web", "unavailable.example");
        let result = NameServiceDirectory::fetch_with_resolver(&did, |doc_type| {
            std::future::ready(Err(if doc_type.is_none() {
                DirectoryError::Unavailable("default endpoint unavailable".into())
            } else {
                DirectoryError::NotFound("not published".into())
            }))
        })
        .await;
        assert_eq!(
            result.unwrap_err(),
            DirectoryError::Unavailable("default endpoint unavailable".into())
        );
    }

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

    #[test]
    fn self_hosted_owners_route_to_their_own_zone() {
        let alice = DID::new("bns", "alice");
        for doc in [
            json!({"id":"did:bns:alice","@context":"https://www.buckyos.org/ns/owner"}),
            json!({"id":"did:bns:alice","binded_zone_list":[]}),
        ] {
            let profile = profile_from_document(&alice, &doc);
            assert_eq!(profile.kind, LookupKind::User);
            assert_eq!(profile.zone, Some(host_of(&alice)));
        }
    }

    #[test]
    fn a_declared_zone_must_be_valid_and_takes_precedence() {
        let alice = DID::new("bns", "alice");
        let profile = profile_from_document(
            &alice,
            &json!({"binded_zone_list":["did:web:charlie.me", "did:web:other.me"]}),
        );
        assert_eq!(profile.zone.as_deref(), Some("charlie.me"));
        let profile = profile_from_document(&alice, &json!({"binded_zone_list":[null]}));
        assert_eq!(profile.zone, None);
    }
}
