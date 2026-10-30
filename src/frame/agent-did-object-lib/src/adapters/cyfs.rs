//! Zone files and content objects (agent-object-url TODO §1.2, §6 第 3 段): `cyfs:///<path>` is the DFS file
//! `$BUCKYOS_ROOT/data/<path>`, read through the local mount when this process has it (an Agent's container mounts
//! its owner's home and its AgentRoot at the same paths) and otherwise as its DFS object. `file://<device>/<path>` is
//! that DFS file when the path is under the data root; another device's other files are not reachable from here.
//! `cyfs://<objid>/` is read from the Zone's object store.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::ObjectUrl;
use serde_json::json;

use super::{
    unsupported_event_subscription, unsupported_event_unsubscription, AdapterEventSubscription,
    AdapterReadRequest, AdapterReadResponse, AdapterSubscribeEventRequest,
    AdapterUnsubscribeEventRequest, AdapterXCallRequest, AdapterXCallResponse, AgentObjectAdapter,
    DidObjectProtocolAdapter,
};
use crate::error::AgentDIDObjectError;
use crate::types::{ObjectRef, ReadMeta};
use crate::zone::{RuntimeZoneAccess, ZoneAccess};

pub struct CyfsAdapter {
    id: String,
    zone: Arc<dyn ZoneAccess>,
    objects: DidObjectProtocolAdapter,
}

impl CyfsAdapter {
    pub fn new(id: String) -> Self {
        Self::with_zone(id, Arc::new(RuntimeZoneAccess))
    }

    pub fn with_zone(id: String, zone: Arc<dyn ZoneAccess>) -> Self {
        Self {
            objects: DidObjectProtocolAdapter::with_zone(id.clone(), zone.clone()),
            id,
            zone,
        }
    }

    fn unsupported(message: String) -> AgentDIDObjectError {
        AgentDIDObjectError::UnsupportedObjectRef(message)
    }

    /// A DFS path of this Zone: the local file or directory when mounted here, else its DFS object.
    async fn read_dfs(
        &self,
        dfs_path: &str,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        let canonical = format!("cyfs://{dfs_path}");
        let local = buckyos_api::dfs_to_data_path(&self.zone.data_root(), dfs_path)
            .ok_or_else(|| Self::unsupported(format!("{canonical}: not a plain DFS path")))?;
        match tokio::fs::metadata(&local).await {
            Ok(meta) if meta.is_dir() => {
                let content = list_dir(&local).await?;
                Ok(self.response(&canonical, &local, content, "inode/directory", None, req))
            }
            Ok(meta) => {
                let bytes = tokio::fs::read(&local).await?;
                let text = String::from_utf8(bytes).map_err(|_| {
                    AgentDIDObjectError::UnsupportedObjectRef(format!(
                        "{canonical} is not text; use llm_understand_media to look at media files"
                    ))
                })?;
                Ok(self.response(&canonical, &local, text, "text/plain", Some(meta.len()), req))
            }
            Err(_) => {
                if let Some((text, source)) = self.dfs_file_text(&canonical).await? {
                    let size = text.len() as u64;
                    let mut response = self.response(&canonical, Path::new(&source), text, "text/plain", Some(size), req);
                    response.adapt_meta = json!({ "dfs_read": source });
                    return Ok(response);
                }
                let mut dfs_req = req;
                dfs_req.object_ref = ObjectRef::parse(&canonical)?;
                self.objects.read(dfs_req).await
            }
        }
    }

    /// A DFS file not mounted here: its bytes through nfs-server (`/nfs/v1/read/<node_id>` of the DFS object's
    /// `info`), with the caller's session. `None` when it is no file (a directory reads as its DFS object).
    async fn dfs_file_text(&self, canonical: &str) -> Result<Option<(String, String)>, AgentDIDObjectError> {
        let Some(origin) = self.zone.gateway_origin() else {
            return Ok(None);
        };
        let url = buckyos_api::parse_object_url(canonical).map_err(|e| Self::unsupported(e.to_string()))?;
        let Some(endpoint) = buckyos_api::object_endpoint(&url, &origin, self.zone.zone_host().as_deref()) else {
            return Ok(None);
        };
        let http = crate::zone::client_for(self.zone.as_ref(), &endpoint).await;
        let objects = name_client::DIDObjectClient::with_http_client(http.clone());
        let Ok(resolved) = objects.resolve(&endpoint).await else {
            return Ok(None);
        };
        let Ok(info) = objects.read_property_from_resolved(&resolved, "info").await else {
            return Ok(None);
        };
        if info["kind"] != "file" {
            return Ok(None);
        }
        let node_id = info["ref"]["node_id"].as_str().or_else(|| info["copy_ref"]["node_id"].as_str()).unwrap_or_default();
        if node_id.is_empty() {
            return Ok(None);
        }
        let source = format!("{}/nfs/v1/read/{node_id}", origin.trim_end_matches('/'));
        let response = http.get(&source).send().await?;
        if !response.status().is_success() {
            return Err(AgentDIDObjectError::HttpError(format!("{canonical}: GET {source} returned {}", response.status())));
        }
        let bytes = response.bytes().await?;
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
            Self::unsupported(format!("{canonical} is not text; use llm_understand_media to look at media files"))
        })?;
        Ok(Some((text, source)))
    }

    fn response(
        &self,
        canonical: &str,
        local: &Path,
        content: String,
        content_type: &str,
        size: Option<u64>,
        req: AdapterReadRequest,
    ) -> AdapterReadResponse {
        AdapterReadResponse {
            object: canonical.to_string(),
            object_did: None,
            content: Some(content),
            meta: ReadMeta {
                title: None,
                content_type: Some(content_type.to_string()),
                size,
                updated_at: None,
                source: Some(local.display().to_string()),
                extra: json!({}),
            },
            prompt_guidance: vec![],
            trust_guidance: vec![],
            errors: vec![],
            cache_key: Some(canonical.to_string()),
            version: None,
            route: req.route_trace,
            adapt_meta: json!({ "local_path": local.display().to_string() }),
        }
    }
}

async fn list_dir(dir: &Path) -> Result<String, AgentDIDObjectError> {
    let mut names = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
        names.push(if is_dir { format!("{name}/") } else { name });
    }
    names.sort();
    Ok(names.join("\n"))
}

#[async_trait]
impl AgentObjectAdapter for CyfsAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    async fn read(
        &self,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        let url = buckyos_api::parse_object_url(&req.object_ref.normalized)
            .map_err(|err| Self::unsupported(err.to_string()))?;
        let zone_host = self.zone.zone_host();
        match &url {
            ObjectUrl::CyfsPath { zone, path } if zone.is_none() || *zone == zone_host => {
                let path = path.clone();
                self.read_dfs(&path, req).await
            }
            ObjectUrl::CyfsPath { .. } => Err(Self::unsupported(format!(
                "{url}: files of another Zone are not readable from here"
            ))),
            ObjectUrl::File { device_id, path } => {
                match buckyos_api::data_path_to_dfs(&self.zone.data_root(), Path::new(path)) {
                    Some(dfs_path) => self.read_dfs(&dfs_path, req).await,
                    None => Err(Self::unsupported(format!(
                        "{url}: a file of device {} outside the Zone's data; another device's files are read through its node executor, which read does not reach yet",
                        device_id.as_deref().unwrap_or("(the source runtime)")
                    ))),
                }
            }
            ObjectUrl::CyfsObject { obj_id, inner_path, .. } => {
                if !inner_path.is_empty() && inner_path != "/" {
                    return Err(Self::unsupported(format!(
                        "{url}: reading inside a container object is not supported yet; read {obj_id} itself"
                    )));
                }
                let content = agent_tool::read_tool::read_named_object(obj_id)
                    .await
                    .map_err(AgentDIDObjectError::AdapterError)?;
                let text = String::from_utf8(content.bytes).map_err(|_| {
                    Self::unsupported(format!("{obj_id} is not text; use llm_understand_media to look at media objects"))
                })?;
                let canonical = url.to_string();
                Ok(AdapterReadResponse {
                    object: canonical.clone(),
                    object_did: None,
                    content: Some(text),
                    meta: ReadMeta {
                        content_type: content.mime,
                        ..ReadMeta::default()
                    },
                    prompt_guidance: vec![],
                    trust_guidance: vec![],
                    errors: vec![],
                    cache_key: Some(canonical),
                    version: Some(obj_id.to_string()),
                    route: req.route_trace,
                    adapt_meta: json!({}),
                })
            }
            other => Err(Self::unsupported(format!("{other} is not a Zone file or content object"))),
        }
    }

    async fn x_call(
        &self,
        req: AdapterXCallRequest,
    ) -> Result<AdapterXCallResponse, AgentDIDObjectError> {
        Err(AgentDIDObjectError::UnsupportedMethod(format!(
            "{} is a file; it has no actions",
            req.object_ref.normalized
        )))
    }

    async fn subscribe_event(
        &self,
        req: AdapterSubscribeEventRequest,
    ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
        unsupported_event_subscription(&self.id, &req)
    }

    async fn unsubscribe_event(
        &self,
        _req: AdapterUnsubscribeEventRequest,
    ) -> Result<(), AgentDIDObjectError> {
        unsupported_event_unsubscription(&self.id)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::{AdapterConfig, AdapterType, ObjectRoute};
    use crate::router::{RouteMatchType, RouteMethod, RouteTrace};
    use crate::types::ReadInput;
    use crate::zone::tests::TestZone;

    fn request(url: &str) -> AdapterReadRequest {
        AdapterReadRequest {
            object_ref: ObjectRef::parse(url).unwrap(),
            input: ReadInput {
                object: url.to_string(),
                purpose: None,
                session_id: None,
                content_only: true,
                range: None,
                max_tokens: None,
                options: json!({}),
            },
            route: ObjectRoute {
                id: "cyfs".into(),
                priority: 0,
                match_type: RouteMatchType::Scheme,
                pattern: "cyfs".into(),
                adapter: "cyfs".into(),
                methods: vec![],
                options: json!({}),
            },
            route_trace: RouteTrace { route_id: "cyfs".into(), adapter: "cyfs".into(), method: RouteMethod::Read },
            adapter_config: AdapterConfig {
                id: "cyfs".into(),
                adapter_type: AdapterType::Cyfs,
                endpoint: None,
                auth_token_env: None,
                options: json!({}),
            },
        }
    }

    #[tokio::test]
    async fn dfs_paths_read_the_mounted_data_and_other_devices_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        std::fs::create_dir_all(root.join("home/alice/docs")).unwrap();
        std::fs::write(root.join("home/alice/docs/a b.md"), "hello\n").unwrap();
        let zone = Arc::new(TestZone { origin: None, data_root: root.clone() });
        let adapter = CyfsAdapter::with_zone("cyfs".into(), zone);

        let file = adapter.read(request("cyfs:///home/alice/docs/a%20b.md")).await.unwrap();
        assert_eq!(file.content.as_deref(), Some("hello\n"));
        assert_eq!(file.object, "cyfs:///home/alice/docs/a%20b.md");
        let listing = adapter.read(request("cyfs://test.buckyos.io/home/alice/")).await.unwrap();
        assert_eq!(listing.content.as_deref(), Some("docs/"));

        let data_file = format!("file://ood1{}", root.join("home/alice/docs/a%20b.md").display());
        assert_eq!(adapter.read(request(&data_file)).await.unwrap().content.as_deref(), Some("hello\n"));

        let err = adapter.read(request("file://node2/tmp/a")).await.unwrap_err();
        assert!(err.to_string().contains("node executor"), "{err}");
        let err = adapter.read(request("cyfs://bob.example.org/home/bob/a.md")).await.unwrap_err();
        assert!(err.to_string().contains("another Zone"), "{err}");
        // Not mounted here and no gateway: its DFS object cannot be reached either.
        let err = adapter.read(request("cyfs:///home/bob/x.md")).await.unwrap_err();
        assert!(err.to_string().contains("no BuckyOS runtime"), "{err}");
    }
}
