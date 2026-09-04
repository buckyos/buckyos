#![allow(dead_code)]

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use buckyos_api::{AiArtifact, AiccError, AiccErrorCode, ResourceRef};
use futures_util::StreamExt;
use named_store::NamedDataMgr;
use ndn_lib::{load_named_obj_and_verify, ChunkHasher, FileObject, NamedObject, ObjId};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{Cursor, Read};
use std::net::IpAddr;
use std::path::Component;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::AsyncReadExt;

const ZIP_MIME_TYPES: [&str; 2] = ["application/zip", "application/x-zip-compressed"];
const REJECTED_ARCHIVE_MIME_TYPES: [&str; 5] = [
    "application/gzip",
    "application/x-gzip",
    "application/x-tar",
    "application/x-7z-compressed",
    "application/vnd.rar",
];
const RESERVED_ARTIFACT_META_KEYS: [&str; 6] = [
    "mime_type",
    "file_name",
    "digest",
    "rows",
    "dimensions",
    "space",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceAccessContext {
    pub tenant_id: String,
    pub caller_id: String,
    pub request_id: String,
}

impl ResourceAccessContext {
    pub fn new(
        tenant_id: impl Into<String>,
        caller_id: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Result<Self, ResourceError> {
        let context = Self {
            tenant_id: tenant_id.into(),
            caller_id: caller_id.into(),
            request_id: request_id.into(),
        };
        if context.tenant_id.trim().is_empty()
            || context.caller_id.trim().is_empty()
            || context.request_id.trim().is_empty()
        {
            return Err(ResourceError::new(
                ResourceFailure::Unauthorized,
                "resource access context is incomplete",
            ));
        }
        Ok(context)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceAccessOperation {
    Inspect,
    ReadContent,
    FetchUrl,
    WriteArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResourceTarget {
    Url { scheme: String, host: String },
    Inline,
    NamedObject { obj_id: ObjId },
    Artifact,
}

#[async_trait]
pub(crate) trait ResourceAuthorizer: Send + Sync {
    async fn authorize(
        &self,
        context: &ResourceAccessContext,
        target: &ResourceTarget,
        operation: ResourceAccessOperation,
    ) -> Result<(), ResourceError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceLimits {
    pub max_resources: usize,
    pub max_single_bytes: u64,
    pub max_total_bytes: u64,
    pub max_inline_bytes: u64,
    pub max_archive_depth: usize,
    pub max_archive_files: usize,
    pub max_archive_expanded_bytes: u64,
    pub max_archive_expansion_ratio: u64,
    pub allowed_mime_types: Vec<String>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_resources: 16,
            max_single_bytes: 32 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
            max_inline_bytes: 4 * 1024 * 1024,
            max_archive_depth: 3,
            max_archive_files: 1_000,
            max_archive_expanded_bytes: 128 * 1024 * 1024,
            max_archive_expansion_ratio: 100,
            allowed_mime_types: Vec::new(),
        }
    }
}

impl ResourceLimits {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.max_resources == 0
            || self.max_single_bytes == 0
            || self.max_total_bytes == 0
            || self.max_inline_bytes == 0
            || self.max_archive_depth == 0
            || self.max_archive_files == 0
            || self.max_archive_expanded_bytes == 0
            || self.max_archive_expansion_ratio == 0
        {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "resource limits must be greater than zero",
            ));
        }
        if self.max_single_bytes > self.max_total_bytes {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "single resource limit exceeds batch limit",
            ));
        }
        for mime in &self.allowed_mime_types {
            if let Some(kind) = mime.strip_suffix("/*") {
                if kind.is_empty() || !kind.bytes().all(mime_token_byte) {
                    return Err(ResourceError::new(
                        ResourceFailure::MimeInvalid,
                        "resource MIME policy is invalid",
                    ));
                }
            } else {
                normalize_mime(mime)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResourceKind {
    Url,
    Base64,
    NamedObject,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ResourceKey(String);

impl ResourceKey {
    pub fn from_ref(resource: &ResourceRef) -> Self {
        match resource {
            ResourceRef::Url { url, mime_hint } => {
                Self::hashed("url", [Some(url.as_str()), mime_hint.as_deref()])
            }
            ResourceRef::Base64 { mime, data_base64 } => {
                Self::hashed("base64", [Some(mime.as_str()), Some(data_base64.as_str())])
            }
            ResourceRef::NamedObject { obj_id } => {
                let value = obj_id.to_string();
                Self::hashed("named_object", [Some(value.as_str())])
            }
        }
    }

    fn hashed<'a>(kind: &str, fields: impl IntoIterator<Item = Option<&'a str>>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"aicc-resource-key-v1\0");
        update_key_part(&mut hasher, Some(kind));
        for field in fields {
            update_key_part(&mut hasher, field);
        }
        Self(format!(
            "aicc-resource-v1:{kind}:{}",
            hex_digest(hasher.finalize())
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for ResourceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ResourceKey").field(&self.0).finish()
    }
}

impl fmt::Display for ResourceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ResourceMetadata {
    pub kind: ResourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obj_id: Option<ObjId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub attributes: Map<String, Value>,
}

impl fmt::Debug for ResourceMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceMetadata")
            .field("kind", &self.kind)
            .field("obj_id", &self.obj_id)
            .field("mime", &self.mime)
            .field("size_bytes", &self.size_bytes)
            .field("has_digest", &self.digest.is_some())
            .field("file_name", &self.file_name)
            .field(
                "attribute_keys",
                &self.attributes.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

struct InspectedResource {
    source: ResourceRef,
    metadata: ResourceMetadata,
}

pub(crate) struct InspectedResourceBatch {
    resources: Vec<InspectedResource>,
}

impl InspectedResourceBatch {
    pub fn metadata(&self) -> Vec<ResourceMetadata> {
        self.resources
            .iter()
            .map(|resource| resource.metadata.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.resources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }
}

impl fmt::Debug for InspectedResourceBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InspectedResourceBatch")
            .field("metadata", &self.metadata())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct MaterializedResource {
    key: ResourceKey,
    metadata: ResourceMetadata,
    bytes: Vec<u8>,
}

impl MaterializedResource {
    pub fn key(&self) -> &ResourceKey {
        &self.key
    }

    pub fn metadata(&self) -> &ResourceMetadata {
        &self.metadata
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn into_codec_parts(self) -> Result<CodecResourceParts, ResourceError> {
        let mime = self.metadata.mime.ok_or_else(|| {
            ResourceError::new(
                ResourceFailure::MimeInvalid,
                "materialized resource MIME is required by protocol codecs",
            )
        })?;
        validate_resource_file_name(self.metadata.file_name.as_deref())?;
        Ok(CodecResourceParts {
            key: self.key,
            bytes: self.bytes,
            mime,
            file_name: self.metadata.file_name,
        })
    }

    pub fn multipart_part(&self) -> Result<reqwest::multipart::Part, ResourceError> {
        let mut part = reqwest::multipart::Part::bytes(self.bytes.clone());
        if let Some(mime) = &self.metadata.mime {
            part = part.mime_str(mime).map_err(|_| {
                ResourceError::new(ResourceFailure::MimeInvalid, "invalid multipart MIME")
            })?;
        }
        if let Some(file_name) = &self.metadata.file_name {
            part = part.file_name(file_name.clone());
        }
        Ok(part)
    }
}

impl fmt::Debug for MaterializedResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaterializedResource")
            .field("key", &self.key)
            .field("metadata", &self.metadata)
            .field("content", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CodecResourceParts {
    pub key: ResourceKey,
    pub bytes: Vec<u8>,
    pub mime: String,
    pub file_name: Option<String>,
}

impl fmt::Debug for CodecResourceParts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodecResourceParts")
            .field("key", &self.key)
            .field("byte_len", &self.bytes.len())
            .field("mime", &self.mime)
            .field("file_name", &self.file_name)
            .finish()
    }
}

pub(crate) fn require_materialized<'a, T>(
    resources: &'a BTreeMap<String, T>,
    resource: &ResourceRef,
) -> Result<&'a T, ResourceError> {
    let key = ResourceKey::from_ref(resource);
    resources.get(key.as_str()).ok_or_else(|| {
        ResourceError::new(
            ResourceFailure::PhaseViolation,
            "resource was not materialized before protocol encoding",
        )
    })
}

pub(crate) fn multipart_form(
    fields: impl IntoIterator<Item = (String, MaterializedResource)>,
) -> Result<reqwest::multipart::Form, ResourceError> {
    let mut form = reqwest::multipart::Form::new();
    for (field_name, resource) in fields {
        if field_name.trim().is_empty() || field_name.contains(['\r', '\n']) {
            return Err(ResourceError::new(
                ResourceFailure::InvalidReference,
                "invalid multipart field name",
            ));
        }
        form = form.part(field_name, resource.multipart_part()?);
    }
    Ok(form)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingArtifactMetadata {
    pub rows: u64,
    pub dimensions: u64,
    pub space: String,
}

impl EmbeddingArtifactMetadata {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.rows == 0 || self.dimensions == 0 || self.space.trim().is_empty() {
            return Err(ResourceError::new(
                ResourceFailure::ArtifactInvalid,
                "embedding artifact metadata is incomplete",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct ArtifactSpec {
    pub name: String,
    pub mime: String,
    pub attributes: Map<String, Value>,
    pub embedding: Option<EmbeddingArtifactMetadata>,
}

impl fmt::Debug for ArtifactSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactSpec")
            .field("name", &self.name)
            .field("mime", &self.mime)
            .field(
                "attribute_keys",
                &self.attributes.keys().collect::<Vec<_>>(),
            )
            .field("embedding", &self.embedding)
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct StoredFile {
    obj_id: ObjId,
    file_name: Option<String>,
    mime: Option<String>,
    size_bytes: u64,
    digest: Option<String>,
    attributes: Map<String, Value>,
}

#[async_trait]
pub(crate) trait ResourceStore: Send + Sync {
    async fn inspect(&self, obj_id: &ObjId) -> Result<StoredFile, ResourceError>;
    async fn read(&self, obj_id: &ObjId, max_bytes: u64) -> Result<Vec<u8>, ResourceError>;
    async fn write_artifact(
        &self,
        bytes: &[u8],
        spec: &ArtifactSpec,
    ) -> Result<StoredFile, ResourceError>;
}

#[derive(Clone)]
pub(crate) struct NamedDataMgrResourceStore {
    store: NamedDataMgr,
}

impl NamedDataMgrResourceStore {
    pub fn new(store: NamedDataMgr) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ResourceStore for NamedDataMgrResourceStore {
    async fn inspect(&self, obj_id: &ObjId) -> Result<StoredFile, ResourceError> {
        if !obj_id.is_file_object() {
            return Err(ResourceError::new(
                ResourceFailure::InvalidReference,
                "named resource must reference a FileObject",
            ));
        }
        let encoded = self.store.get_object(obj_id).await.map_err(store_error)?;
        let file: FileObject = load_named_obj_and_verify(obj_id, &encoded).map_err(|_| {
            ResourceError::new(
                ResourceFailure::NamedObjectInvalid,
                "FileObject verification failed",
            )
        })?;
        stored_file_from_object(obj_id.clone(), file)
    }

    async fn read(&self, obj_id: &ObjId, max_bytes: u64) -> Result<Vec<u8>, ResourceError> {
        let (reader, declared_size) = self
            .store
            .open_reader(obj_id, None)
            .await
            .map_err(store_error)?;
        if declared_size > max_bytes {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "named resource exceeds byte limit",
            ));
        }
        let mut limited = reader.take(max_bytes.saturating_add(1));
        let mut bytes = Vec::with_capacity(declared_size.min(max_bytes) as usize);
        limited.read_to_end(&mut bytes).await.map_err(|_| {
            ResourceError::new(ResourceFailure::Unavailable, "resource read failed")
        })?;
        if bytes.len() as u64 > max_bytes {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "named resource exceeds byte limit",
            ));
        }
        Ok(bytes)
    }

    async fn write_artifact(
        &self,
        bytes: &[u8],
        spec: &ArtifactSpec,
    ) -> Result<StoredFile, ResourceError> {
        let chunk_id = ChunkHasher::new(None)
            .map_err(|_| ResourceError::new(ResourceFailure::Unavailable, "hasher unavailable"))?
            .calc_chunk_id_from_bytes(bytes);
        self.store
            .put_chunk(&chunk_id, bytes)
            .await
            .map_err(store_error)?;

        let mut file = FileObject::new(spec.name.clone(), bytes.len() as u64, chunk_id.to_string());
        file.meta.insert("mime_type".to_string(), json!(spec.mime));
        file.meta.insert("file_name".to_string(), json!(spec.name));
        file.meta.insert(
            "digest".to_string(),
            json!(format!("sha256:{}", sha256_hex(bytes))),
        );
        for (key, value) in &spec.attributes {
            file.meta.insert(key.clone(), value.clone());
        }
        if let Some(embedding) = &spec.embedding {
            file.meta.insert("rows".to_string(), json!(embedding.rows));
            file.meta
                .insert("dimensions".to_string(), json!(embedding.dimensions));
            file.meta
                .insert("space".to_string(), json!(embedding.space));
        }
        let (obj_id, encoded) = file.gen_obj_id();
        self.store
            .put_object(&obj_id, &encoded)
            .await
            .map_err(store_error)?;
        stored_file_from_object(obj_id, file)
    }
}

#[derive(Clone)]
pub(crate) struct FetchedResource {
    pub bytes: Vec<u8>,
    pub mime: Option<String>,
    pub file_name: Option<String>,
}

#[async_trait]
pub(crate) trait UrlResourceFetcher: Send + Sync {
    async fn fetch(
        &self,
        url: &reqwest::Url,
        max_bytes: u64,
    ) -> Result<FetchedResource, ResourceError>;
}

#[derive(Clone)]
pub(crate) struct ReqwestUrlResourceFetcher {
    client: reqwest::Client,
}

impl ReqwestUrlResourceFetcher {
    pub fn new() -> Result<Self, ResourceError> {
        Self::from_builder(reqwest::Client::builder())
    }

    pub fn from_builder(builder: reqwest::ClientBuilder) -> Result<Self, ResourceError> {
        let client = builder
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                ResourceError::new(ResourceFailure::Unavailable, "URL client setup failed")
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl UrlResourceFetcher for ReqwestUrlResourceFetcher {
    async fn fetch(
        &self,
        url: &reqwest::Url,
        max_bytes: u64,
    ) -> Result<FetchedResource, ResourceError> {
        validate_url(url)?;
        let response =
            self.client.get(url.clone()).send().await.map_err(|_| {
                ResourceError::new(ResourceFailure::Unavailable, "URL fetch failed")
            })?;
        if !response.status().is_success() {
            return Err(ResourceError::new(
                ResourceFailure::Unavailable,
                "URL resource is unavailable",
            ));
        }
        validate_url(response.url())?;
        if response
            .content_length()
            .is_some_and(|size| size > max_bytes)
        {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "URL resource exceeds byte limit",
            ));
        }
        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(normalize_mime)
            .transpose()?;
        let file_name = response
            .url()
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                ResourceError::new(ResourceFailure::Unavailable, "URL resource read failed")
            })?;
            if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(ResourceError::new(
                    ResourceFailure::LimitExceeded,
                    "URL resource exceeds byte limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(FetchedResource {
            bytes,
            mime,
            file_name,
        })
    }
}

pub(crate) struct ResourceManager {
    authorizer: Arc<dyn ResourceAuthorizer>,
    store: Arc<dyn ResourceStore>,
    url_fetcher: Arc<dyn UrlResourceFetcher>,
    limits: ResourceLimits,
}

impl ResourceManager {
    pub fn new(
        authorizer: Arc<dyn ResourceAuthorizer>,
        store: Arc<dyn ResourceStore>,
        url_fetcher: Arc<dyn UrlResourceFetcher>,
        limits: ResourceLimits,
    ) -> Result<Self, ResourceError> {
        limits.validate()?;
        Ok(Self {
            authorizer,
            store,
            url_fetcher,
            limits,
        })
    }

    pub async fn inspect(
        &self,
        context: &ResourceAccessContext,
        resources: &[ResourceRef],
    ) -> Result<InspectedResourceBatch, ResourceError> {
        if resources.len() > self.limits.max_resources {
            return Err(ResourceError::new(
                ResourceFailure::CountExceeded,
                "resource count exceeds limit",
            ));
        }
        let mut inspected = Vec::with_capacity(resources.len());
        let mut known_total = 0u64;
        for resource in resources {
            let (target, metadata) = match resource {
                ResourceRef::Url { url, mime_hint } => {
                    let parsed = reqwest::Url::parse(url).map_err(|_| {
                        ResourceError::new(
                            ResourceFailure::InvalidReference,
                            "invalid resource URL",
                        )
                    })?;
                    validate_url(&parsed)?;
                    let host = parsed.host_str().unwrap_or_default().to_string();
                    let mime = mime_hint.as_deref().map(normalize_mime).transpose()?;
                    self.ensure_mime_allowed(mime.as_deref())?;
                    (
                        ResourceTarget::Url {
                            scheme: parsed.scheme().to_string(),
                            host,
                        },
                        ResourceMetadata {
                            kind: ResourceKind::Url,
                            obj_id: None,
                            mime,
                            size_bytes: None,
                            digest: None,
                            file_name: parsed
                                .path_segments()
                                .and_then(|mut segments| segments.next_back())
                                .filter(|name| !name.is_empty())
                                .map(str::to_string),
                            attributes: Map::new(),
                        },
                    )
                }
                ResourceRef::Base64 { mime, data_base64 } => {
                    let mime = normalize_mime(mime)?;
                    self.ensure_mime_allowed(Some(&mime))?;
                    let estimated = decoded_base64_size(data_base64)?;
                    if estimated > self.limits.max_inline_bytes
                        || estimated > self.limits.max_single_bytes
                    {
                        return Err(ResourceError::new(
                            ResourceFailure::LimitExceeded,
                            "inline resource exceeds byte limit",
                        ));
                    }
                    (
                        ResourceTarget::Inline,
                        ResourceMetadata {
                            kind: ResourceKind::Base64,
                            obj_id: None,
                            mime: Some(mime),
                            size_bytes: Some(estimated),
                            digest: None,
                            file_name: None,
                            attributes: Map::new(),
                        },
                    )
                }
                ResourceRef::NamedObject { obj_id } => {
                    let target = ResourceTarget::NamedObject {
                        obj_id: obj_id.clone(),
                    };
                    self.authorizer
                        .authorize(context, &target, ResourceAccessOperation::Inspect)
                        .await?;
                    let file = self.store.inspect(obj_id).await?;
                    self.ensure_mime_allowed(file.mime.as_deref())?;
                    if file.size_bytes > self.limits.max_single_bytes {
                        return Err(ResourceError::new(
                            ResourceFailure::LimitExceeded,
                            "named resource exceeds byte limit",
                        ));
                    }
                    let metadata = ResourceMetadata {
                        kind: ResourceKind::NamedObject,
                        obj_id: Some(file.obj_id),
                        mime: file.mime,
                        size_bytes: Some(file.size_bytes),
                        digest: file.digest,
                        file_name: file.file_name,
                        attributes: file.attributes,
                    };
                    known_total = known_total.checked_add(file.size_bytes).ok_or_else(|| {
                        ResourceError::new(ResourceFailure::LimitExceeded, "resource size overflow")
                    })?;
                    inspected.push(InspectedResource {
                        source: resource.clone(),
                        metadata,
                    });
                    continue;
                }
            };
            self.authorizer
                .authorize(context, &target, ResourceAccessOperation::Inspect)
                .await?;
            if let Some(size) = metadata.size_bytes {
                known_total = known_total.checked_add(size).ok_or_else(|| {
                    ResourceError::new(ResourceFailure::LimitExceeded, "resource size overflow")
                })?;
            }
            inspected.push(InspectedResource {
                source: resource.clone(),
                metadata,
            });
        }
        if known_total > self.limits.max_total_bytes {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "resource batch exceeds byte limit",
            ));
        }
        Ok(InspectedResourceBatch {
            resources: inspected,
        })
    }

    pub async fn materialize_after_provider_selected(
        &self,
        context: &ResourceAccessContext,
        provider_call_id: &str,
        inspected: InspectedResourceBatch,
    ) -> Result<Vec<MaterializedResource>, ResourceError> {
        if provider_call_id.trim().is_empty() {
            return Err(ResourceError::new(
                ResourceFailure::PhaseViolation,
                "provider must be selected before resource materialization",
            ));
        }
        let mut materialized = Vec::with_capacity(inspected.resources.len());
        let mut total = 0u64;
        for inspected_resource in inspected.resources {
            let key = ResourceKey::from_ref(&inspected_resource.source);
            let mut metadata = inspected_resource.metadata;
            let bytes = match inspected_resource.source {
                ResourceRef::Url { url, .. } => {
                    let parsed = reqwest::Url::parse(&url).map_err(|_| {
                        ResourceError::new(
                            ResourceFailure::InvalidReference,
                            "invalid resource URL",
                        )
                    })?;
                    let target = ResourceTarget::Url {
                        scheme: parsed.scheme().to_string(),
                        host: parsed.host_str().unwrap_or_default().to_string(),
                    };
                    self.authorizer
                        .authorize(context, &target, ResourceAccessOperation::FetchUrl)
                        .await?;
                    let fetched = self
                        .url_fetcher
                        .fetch(&parsed, self.limits.max_single_bytes)
                        .await?;
                    if let (Some(expected), Some(actual)) = (&metadata.mime, &fetched.mime) {
                        if expected != actual {
                            return Err(ResourceError::new(
                                ResourceFailure::MimeMismatch,
                                "URL MIME does not match its hint",
                            ));
                        }
                    }
                    metadata.mime = fetched.mime.or(metadata.mime);
                    metadata.file_name = fetched.file_name.or(metadata.file_name);
                    fetched.bytes
                }
                ResourceRef::Base64 { data_base64, .. } => {
                    self.authorizer
                        .authorize(
                            context,
                            &ResourceTarget::Inline,
                            ResourceAccessOperation::ReadContent,
                        )
                        .await?;
                    BASE64_STANDARD.decode(data_base64).map_err(|_| {
                        ResourceError::new(
                            ResourceFailure::Base64Invalid,
                            "inline resource is not valid base64",
                        )
                    })?
                }
                ResourceRef::NamedObject { obj_id } => {
                    let target = ResourceTarget::NamedObject {
                        obj_id: obj_id.clone(),
                    };
                    self.authorizer
                        .authorize(context, &target, ResourceAccessOperation::ReadContent)
                        .await?;
                    self.store
                        .read(&obj_id, self.limits.max_single_bytes)
                        .await?
                }
            };
            self.validate_materialized(&mut metadata, &bytes)?;
            total = total.checked_add(bytes.len() as u64).ok_or_else(|| {
                ResourceError::new(ResourceFailure::LimitExceeded, "resource size overflow")
            })?;
            if total > self.limits.max_total_bytes {
                return Err(ResourceError::new(
                    ResourceFailure::LimitExceeded,
                    "resource batch exceeds byte limit",
                ));
            }
            materialized.push(MaterializedResource {
                key,
                metadata,
                bytes,
            });
        }
        Ok(materialized)
    }

    pub async fn write_artifact(
        &self,
        context: &ResourceAccessContext,
        bytes: &[u8],
        spec: ArtifactSpec,
    ) -> Result<AiArtifact, ResourceError> {
        validate_artifact_spec(&spec)?;
        if bytes.len() as u64 > self.limits.max_single_bytes {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "artifact exceeds byte limit",
            ));
        }
        let mime = normalize_mime(&spec.mime)?;
        if sniff_mime(bytes).is_some_and(|detected| !mime_matches(&mime, detected)) {
            return Err(ResourceError::new(
                ResourceFailure::MimeMismatch,
                "artifact content does not match its MIME",
            ));
        }
        self.ensure_mime_allowed(Some(&mime))?;
        if let Some(embedding) = &spec.embedding {
            embedding.validate()?;
        }
        self.authorizer
            .authorize(
                context,
                &ResourceTarget::Artifact,
                ResourceAccessOperation::WriteArtifact,
            )
            .await?;
        let stored = self.store.write_artifact(bytes, &spec).await?;
        let mut metadata = stored.attributes;
        metadata.insert("size_bytes".to_string(), json!(stored.size_bytes));
        if let Some(digest) = stored.digest {
            metadata.insert("digest".to_string(), json!(digest));
        }
        if let Some(embedding) = spec.embedding {
            metadata.insert("rows".to_string(), json!(embedding.rows));
            metadata.insert("dimensions".to_string(), json!(embedding.dimensions));
            metadata.insert("space".to_string(), json!(embedding.space));
        }
        Ok(AiArtifact {
            name: spec.name,
            resource: ResourceRef::NamedObject {
                obj_id: stored.obj_id,
            },
            mime: stored.mime,
            metadata: Some(Value::Object(metadata)),
        })
    }

    fn validate_materialized(
        &self,
        metadata: &mut ResourceMetadata,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        if bytes.len() as u64 > self.limits.max_single_bytes {
            return Err(ResourceError::new(
                ResourceFailure::LimitExceeded,
                "resource exceeds byte limit",
            ));
        }
        if metadata
            .size_bytes
            .is_some_and(|expected| expected != bytes.len() as u64)
        {
            return Err(ResourceError::new(
                ResourceFailure::SizeMismatch,
                "resource size does not match metadata",
            ));
        }
        let detected_mime = sniff_mime(bytes);
        if let (Some(declared), Some(detected)) = (metadata.mime.as_deref(), detected_mime) {
            if !mime_matches(declared, detected) {
                return Err(ResourceError::new(
                    ResourceFailure::MimeMismatch,
                    "resource content does not match its MIME",
                ));
            }
        }
        if metadata.mime.is_none() {
            metadata.mime = detected_mime.map(str::to_string);
        }
        self.ensure_mime_allowed(metadata.mime.as_deref())?;
        validate_resource_file_name(metadata.file_name.as_deref())?;
        metadata.size_bytes = Some(bytes.len() as u64);
        metadata.digest = Some(format!("sha256:{}", sha256_hex(bytes)));
        inspect_archive(bytes, metadata, &self.limits)?;
        Ok(())
    }

    fn ensure_mime_allowed(&self, mime: Option<&str>) -> Result<(), ResourceError> {
        let Some(mime) = mime else {
            if self.limits.allowed_mime_types.is_empty() {
                return Ok(());
            }
            return Err(ResourceError::new(
                ResourceFailure::MimeInvalid,
                "resource MIME is required by policy",
            ));
        };
        let mime = normalize_mime(mime)?;
        if self.limits.allowed_mime_types.is_empty()
            || self.limits.allowed_mime_types.iter().any(|allowed| {
                allowed == &mime
                    || allowed
                        .strip_suffix("/*")
                        .is_some_and(|prefix| mime.starts_with(&format!("{prefix}/")))
            })
        {
            Ok(())
        } else {
            Err(ResourceError::new(
                ResourceFailure::MimeNotAllowed,
                "resource MIME is not allowed",
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResourceFailure {
    Unauthorized,
    InvalidReference,
    Unavailable,
    NamedObjectInvalid,
    Base64Invalid,
    MimeInvalid,
    MimeMismatch,
    MimeNotAllowed,
    CountExceeded,
    LimitExceeded,
    SizeMismatch,
    ArchiveUnsupported,
    ArchiveEncrypted,
    ArchivePathTraversal,
    ArchiveDepthExceeded,
    ArchiveFileCountExceeded,
    ArchiveExpansionExceeded,
    ArtifactInvalid,
    PhaseViolation,
}

#[derive(Clone, Error, PartialEq, Eq)]
#[error("resource_invalid: {message}")]
pub(crate) struct ResourceError {
    pub failure: ResourceFailure,
    message: String,
}

impl ResourceError {
    pub fn new(failure: ResourceFailure, message: impl Into<String>) -> Self {
        Self {
            failure,
            message: message.into(),
        }
    }

    pub fn to_aicc_error(&self) -> AiccError {
        AiccError {
            code: AiccErrorCode::ResourceInvalid,
            message: self.message.clone(),
            provider_code: None,
            retriable: false,
            details: Some(json!({"failure": self.failure})),
        }
    }
}

impl fmt::Debug for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceError")
            .field("failure", &self.failure)
            .field("message", &self.message)
            .finish()
    }
}

fn stored_file_from_object(obj_id: ObjId, file: FileObject) -> Result<StoredFile, ResourceError> {
    let mut attributes: Map<String, Value> = file.meta.into_iter().collect();
    let mime = take_string(&mut attributes, &["media_type", "mime_type", "mime"])
        .map(|mime| normalize_mime(&mime))
        .transpose()?;
    let file_name = take_string(&mut attributes, &["file_name", "filename"])
        .or_else(|| (!file.content_obj.name.is_empty()).then_some(file.content_obj.name));
    let digest = take_string(&mut attributes, &["digest"])
        .or_else(|| (!file.content.is_empty()).then_some(file.content));
    Ok(StoredFile {
        obj_id,
        file_name,
        mime,
        size_bytes: file.size,
        digest,
        attributes,
    })
}

fn take_string(attributes: &mut Map<String, Value>, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(Value::String(value)) = attributes.remove(*name) {
            return Some(value);
        }
    }
    None
}

fn validate_url(url: &reqwest::Url) -> Result<(), ResourceError> {
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() {
        return Err(ResourceError::new(
            ResourceFailure::InvalidReference,
            "resource URL must be an HTTPS URL without credentials",
        ));
    }
    if url.password().is_some() || url.fragment().is_some() {
        return Err(ResourceError::new(
            ResourceFailure::InvalidReference,
            "resource URL contains forbidden components",
        ));
    }
    if let Some(host) = url.host_str() {
        if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
            return Err(ResourceError::new(
                ResourceFailure::InvalidReference,
                "local resource URLs are forbidden",
            ));
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            if !ip_is_public(ip) {
                return Err(ResourceError::new(
                    ResourceFailure::InvalidReference,
                    "non-public resource URL is forbidden",
                ));
            }
        }
    }
    Ok(())
}

fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast())
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
    }
}

fn normalize_mime(value: &str) -> Result<String, ResourceError> {
    let value = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let Some((kind, subtype)) = value.split_once('/') else {
        return Err(ResourceError::new(
            ResourceFailure::MimeInvalid,
            "resource MIME is invalid",
        ));
    };
    if kind.is_empty()
        || subtype.is_empty()
        || !kind.bytes().all(mime_token_byte)
        || !subtype.bytes().all(mime_token_byte)
    {
        return Err(ResourceError::new(
            ResourceFailure::MimeInvalid,
            "resource MIME is invalid",
        ));
    }
    Ok(value)
}

fn mime_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
        )
}

fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        Some("application/zip")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        Some("audio/wav")
    } else {
        None
    }
}

fn mime_matches(declared: &str, detected: &str) -> bool {
    declared == detected
        || declared == "application/octet-stream"
        || matches!(
            (declared, detected),
            ("application/x-zip-compressed", "application/zip") | ("image/jpg", "image/jpeg")
        )
}

fn validate_resource_file_name(file_name: Option<&str>) -> Result<(), ResourceError> {
    if file_name.is_some_and(|name| {
        name.trim().is_empty()
            || name.len() > 255
            || name == "."
            || name == ".."
            || name
                .chars()
                .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    }) {
        return Err(ResourceError::new(
            ResourceFailure::InvalidReference,
            "resource file name is invalid",
        ));
    }
    Ok(())
}

fn update_key_part(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
        None => hasher.update([0]),
    }
}

fn decoded_base64_size(value: &str) -> Result<u64, ResourceError> {
    if value.is_empty() {
        return Ok(0);
    }
    if value.bytes().any(|byte| byte.is_ascii_whitespace()) || value.len() % 4 != 0 {
        return Err(ResourceError::new(
            ResourceFailure::Base64Invalid,
            "inline resource is not canonical base64",
        ));
    }
    let padding = value.bytes().rev().take_while(|byte| *byte == b'=').count();
    if padding > 2 || value[..value.len().saturating_sub(padding)].contains('=') {
        return Err(ResourceError::new(
            ResourceFailure::Base64Invalid,
            "inline resource is not canonical base64",
        ));
    }
    let groups = u64::try_from(value.len() / 4).map_err(|_| {
        ResourceError::new(
            ResourceFailure::LimitExceeded,
            "inline resource is too large",
        )
    })?;
    groups
        .checked_mul(3)
        .and_then(|size| size.checked_sub(padding as u64))
        .ok_or_else(|| ResourceError::new(ResourceFailure::Base64Invalid, "invalid base64 size"))
}

fn inspect_archive(
    bytes: &[u8],
    metadata: &ResourceMetadata,
    limits: &ResourceLimits,
) -> Result<(), ResourceError> {
    let mime = metadata.mime.as_deref();
    let is_zip = bytes.starts_with(b"PK\x03\x04")
        || mime.is_some_and(|mime| ZIP_MIME_TYPES.contains(&mime))
        || metadata
            .file_name
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".zip"));
    if !is_zip {
        if mime.is_some_and(|mime| REJECTED_ARCHIVE_MIME_TYPES.contains(&mime)) {
            return Err(ResourceError::new(
                ResourceFailure::ArchiveUnsupported,
                "archive format is not supported for safe inspection",
            ));
        }
        return Ok(());
    }
    let mut counters = ArchiveCounters::default();
    inspect_zip(bytes, 1, &mut counters, limits)
}

#[derive(Default)]
struct ArchiveCounters {
    files: usize,
    expanded_bytes: u64,
    compressed_bytes: u64,
}

fn inspect_zip(
    bytes: &[u8],
    depth: usize,
    counters: &mut ArchiveCounters,
    limits: &ResourceLimits,
) -> Result<(), ResourceError> {
    if depth > limits.max_archive_depth {
        return Err(ResourceError::new(
            ResourceFailure::ArchiveDepthExceeded,
            "archive nesting depth exceeds limit",
        ));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| {
        ResourceError::new(ResourceFailure::ArchiveUnsupported, "invalid ZIP archive")
    })?;
    for index in 0..archive.len() {
        {
            let raw = archive.by_index_raw(index).map_err(|_| {
                ResourceError::new(ResourceFailure::ArchiveUnsupported, "invalid ZIP entry")
            })?;
            if raw.encrypted() {
                return Err(ResourceError::new(
                    ResourceFailure::ArchiveEncrypted,
                    "encrypted archives are forbidden",
                ));
            }
        }
        let mut file = archive.by_index(index).map_err(|_| {
            ResourceError::new(ResourceFailure::ArchiveUnsupported, "invalid ZIP entry")
        })?;
        let enclosed = file.enclosed_name().ok_or_else(|| {
            ResourceError::new(
                ResourceFailure::ArchivePathTraversal,
                "archive entry escapes its root",
            )
        })?;
        if enclosed.is_absolute()
            || enclosed.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(ResourceError::new(
                ResourceFailure::ArchivePathTraversal,
                "archive entry escapes its root",
            ));
        }
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(ResourceError::new(
                ResourceFailure::ArchivePathTraversal,
                "archive links are forbidden",
            ));
        }
        if file.is_dir() {
            continue;
        }
        counters.files += 1;
        if counters.files > limits.max_archive_files {
            return Err(ResourceError::new(
                ResourceFailure::ArchiveFileCountExceeded,
                "archive file count exceeds limit",
            ));
        }
        let expanded = file.size();
        let compressed = file.compressed_size();
        if expanded
            > compressed
                .max(1)
                .saturating_mul(limits.max_archive_expansion_ratio)
        {
            return Err(ResourceError::new(
                ResourceFailure::ArchiveExpansionExceeded,
                "archive entry expansion ratio exceeds limit",
            ));
        }
        counters.expanded_bytes =
            counters
                .expanded_bytes
                .checked_add(expanded)
                .ok_or_else(|| {
                    ResourceError::new(
                        ResourceFailure::ArchiveExpansionExceeded,
                        "archive expanded size overflow",
                    )
                })?;
        counters.compressed_bytes = counters
            .compressed_bytes
            .checked_add(compressed)
            .ok_or_else(|| {
                ResourceError::new(
                    ResourceFailure::ArchiveExpansionExceeded,
                    "archive compressed size overflow",
                )
            })?;
        if counters.expanded_bytes > limits.max_archive_expanded_bytes
            || counters.expanded_bytes
                > counters
                    .compressed_bytes
                    .max(1)
                    .saturating_mul(limits.max_archive_expansion_ratio)
        {
            return Err(ResourceError::new(
                ResourceFailure::ArchiveExpansionExceeded,
                "archive expansion exceeds limit",
            ));
        }
        if enclosed
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            if depth == limits.max_archive_depth {
                return Err(ResourceError::new(
                    ResourceFailure::ArchiveDepthExceeded,
                    "archive nesting depth exceeds limit",
                ));
            }
            if expanded > limits.max_archive_expanded_bytes {
                return Err(ResourceError::new(
                    ResourceFailure::ArchiveExpansionExceeded,
                    "nested archive exceeds expanded byte limit",
                ));
            }
            let capacity = usize::try_from(expanded).map_err(|_| {
                ResourceError::new(
                    ResourceFailure::ArchiveExpansionExceeded,
                    "nested archive is too large",
                )
            })?;
            let mut nested = Vec::with_capacity(capacity);
            file.read_to_end(&mut nested).map_err(|_| {
                ResourceError::new(
                    ResourceFailure::ArchiveUnsupported,
                    "nested archive cannot be read",
                )
            })?;
            inspect_zip(&nested, depth + 1, counters, limits)?;
        }
    }
    Ok(())
}

fn validate_artifact_spec(spec: &ArtifactSpec) -> Result<(), ResourceError> {
    if spec.name.trim().is_empty()
        || spec.name.len() > 255
        || spec.name.contains(['/', '\\', '\0', '\r', '\n'])
        || spec.name == "."
        || spec.name == ".."
    {
        return Err(ResourceError::new(
            ResourceFailure::ArtifactInvalid,
            "artifact name is invalid",
        ));
    }
    if spec
        .attributes
        .keys()
        .any(|key| key.trim().is_empty() || RESERVED_ARTIFACT_META_KEYS.contains(&key.as_str()))
    {
        return Err(ResourceError::new(
            ResourceFailure::ArtifactInvalid,
            "artifact attributes contain a reserved metadata key",
        ));
    }
    normalize_mime(&spec.mime)?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes))
}

fn hex_digest(bytes: impl IntoIterator<Item = u8>) -> String {
    let bytes = bytes.into_iter();
    let mut encoded = String::with_capacity(bytes.size_hint().0 * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn store_error(error: ndn_lib::NdnError) -> ResourceError {
    let failure = match error {
        ndn_lib::NdnError::PermissionDenied(_) => ResourceFailure::Unauthorized,
        ndn_lib::NdnError::NotFound(_) => ResourceFailure::Unavailable,
        ndn_lib::NdnError::InvalidId(_)
        | ndn_lib::NdnError::InvalidObjType(_)
        | ndn_lib::NdnError::DecodeError(_)
        | ndn_lib::NdnError::VerifyError(_) => ResourceFailure::NamedObjectInvalid,
        _ => ResourceFailure::Unavailable,
    };
    ResourceError::new(failure, "named resource operation failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;
    use zip::write::SimpleFileOptions;

    #[derive(Default)]
    struct RecordingAuthorizer {
        operations: Mutex<Vec<ResourceAccessOperation>>,
        denied: Option<ResourceAccessOperation>,
    }

    #[async_trait]
    impl ResourceAuthorizer for RecordingAuthorizer {
        async fn authorize(
            &self,
            _context: &ResourceAccessContext,
            _target: &ResourceTarget,
            operation: ResourceAccessOperation,
        ) -> Result<(), ResourceError> {
            self.operations.lock().unwrap().push(operation);
            if self.denied == Some(operation) {
                Err(ResourceError::new(
                    ResourceFailure::Unauthorized,
                    "resource access denied",
                ))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct FakeStore {
        files: Mutex<BTreeMap<String, (StoredFile, Vec<u8>)>>,
    }

    impl FakeStore {
        fn insert(&self, file: StoredFile, bytes: Vec<u8>) {
            self.files
                .lock()
                .unwrap()
                .insert(file.obj_id.to_string(), (file, bytes));
        }
    }

    #[async_trait]
    impl ResourceStore for FakeStore {
        async fn inspect(&self, obj_id: &ObjId) -> Result<StoredFile, ResourceError> {
            self.files
                .lock()
                .unwrap()
                .get(&obj_id.to_string())
                .map(|entry| entry.0.clone())
                .ok_or_else(|| ResourceError::new(ResourceFailure::Unavailable, "not found"))
        }

        async fn read(&self, obj_id: &ObjId, max_bytes: u64) -> Result<Vec<u8>, ResourceError> {
            let bytes = self
                .files
                .lock()
                .unwrap()
                .get(&obj_id.to_string())
                .map(|entry| entry.1.clone())
                .ok_or_else(|| ResourceError::new(ResourceFailure::Unavailable, "not found"))?;
            if bytes.len() as u64 > max_bytes {
                return Err(ResourceError::new(
                    ResourceFailure::LimitExceeded,
                    "too large",
                ));
            }
            Ok(bytes)
        }

        async fn write_artifact(
            &self,
            bytes: &[u8],
            spec: &ArtifactSpec,
        ) -> Result<StoredFile, ResourceError> {
            let mut file = FileObject::new(
                spec.name.clone(),
                bytes.len() as u64,
                "sha256:artifact-content".to_string(),
            );
            file.meta.insert("mime_type".to_string(), json!(spec.mime));
            file.meta.insert(
                "digest".to_string(),
                json!(format!("sha256:{}", sha256_hex(bytes))),
            );
            let (obj_id, _) = file.gen_obj_id();
            let stored = stored_file_from_object(obj_id, file)?;
            self.insert(stored.clone(), bytes.to_vec());
            Ok(stored)
        }
    }

    struct FakeFetcher {
        response: FetchedResource,
    }

    #[async_trait]
    impl UrlResourceFetcher for FakeFetcher {
        async fn fetch(
            &self,
            _url: &reqwest::Url,
            max_bytes: u64,
        ) -> Result<FetchedResource, ResourceError> {
            if self.response.bytes.len() as u64 > max_bytes {
                return Err(ResourceError::new(
                    ResourceFailure::LimitExceeded,
                    "too large",
                ));
            }
            Ok(self.response.clone())
        }
    }

    fn context() -> ResourceAccessContext {
        ResourceAccessContext::new("tenant", "caller", "request").unwrap()
    }

    fn obj_id_for(file: &FileObject) -> ObjId {
        file.gen_obj_id().0
    }

    fn stored_named(bytes: &[u8], mime: &str) -> StoredFile {
        let mut file = FileObject::new(
            "input.bin".to_string(),
            bytes.len() as u64,
            "sha256:content".to_string(),
        );
        file.meta.insert("mime_type".to_string(), json!(mime));
        file.meta.insert("width".to_string(), json!(640));
        stored_file_from_object(obj_id_for(&file), file).unwrap()
    }

    fn manager(
        authorizer: Arc<RecordingAuthorizer>,
        store: Arc<FakeStore>,
        limits: ResourceLimits,
    ) -> ResourceManager {
        ResourceManager::new(
            authorizer,
            store,
            Arc::new(FakeFetcher {
                response: FetchedResource {
                    bytes: b"url-data".to_vec(),
                    mime: Some("image/png".to_string()),
                    file_name: Some("image.png".to_string()),
                },
            }),
            limits,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn inspect_named_object_reads_metadata_only_then_materializes_after_selection() {
        let authorizer = Arc::new(RecordingAuthorizer::default());
        let store = Arc::new(FakeStore::default());
        let bytes = b"named-content".to_vec();
        let stored = stored_named(&bytes, "image/png");
        let obj_id = stored.obj_id.clone();
        store.insert(stored, bytes.clone());
        let manager = manager(authorizer.clone(), store, ResourceLimits::default());

        let source = ResourceRef::NamedObject {
            obj_id: obj_id.clone(),
        };
        let inspected = manager
            .inspect(&context(), std::slice::from_ref(&source))
            .await
            .unwrap();
        assert_eq!(inspected.metadata()[0].size_bytes, Some(bytes.len() as u64));
        assert_eq!(
            authorizer.operations.lock().unwrap().as_slice(),
            &[ResourceAccessOperation::Inspect]
        );

        let materialized = manager
            .materialize_after_provider_selected(&context(), "provider-call", inspected)
            .await
            .unwrap();
        assert_eq!(materialized[0].bytes(), bytes);
        assert_eq!(materialized[0].key(), &ResourceKey::from_ref(&source));
        let parts = materialized[0].clone().into_codec_parts().unwrap();
        assert_eq!(parts.key, ResourceKey::from_ref(&source));
        assert_eq!(parts.bytes, bytes);
        assert_eq!(parts.mime, "image/png");
        assert_eq!(parts.file_name.as_deref(), Some("input.bin"));
        assert_eq!(
            authorizer.operations.lock().unwrap().as_slice(),
            &[
                ResourceAccessOperation::Inspect,
                ResourceAccessOperation::ReadContent
            ]
        );
        assert!(format!("{:?}", materialized[0]).contains("<redacted>"));
        assert!(!format!("{:?}", materialized[0]).contains("named-content"));
    }

    #[tokio::test]
    async fn all_resource_kinds_require_authorization_and_mime_validation() {
        let authorizer = Arc::new(RecordingAuthorizer::default());
        let manager = manager(
            authorizer.clone(),
            Arc::new(FakeStore::default()),
            ResourceLimits {
                allowed_mime_types: vec!["image/*".to_string()],
                ..ResourceLimits::default()
            },
        );
        let inspected = manager
            .inspect(
                &context(),
                &[
                    ResourceRef::url(
                        "https://assets.example/image.png".to_string(),
                        Some("image/png".to_string()),
                    ),
                    ResourceRef::base64("image/png".to_string(), "aW1hZ2U=".to_string()),
                ],
            )
            .await
            .unwrap();
        let materialized = manager
            .materialize_after_provider_selected(&context(), "selected", inspected)
            .await
            .unwrap();
        assert_eq!(materialized.len(), 2);
        assert_eq!(
            authorizer.operations.lock().unwrap().as_slice(),
            &[
                ResourceAccessOperation::Inspect,
                ResourceAccessOperation::Inspect,
                ResourceAccessOperation::FetchUrl,
                ResourceAccessOperation::ReadContent,
            ]
        );

        let error = manager
            .inspect(
                &context(),
                &[ResourceRef::base64(
                    "text/plain".to_string(),
                    "dGV4dA==".to_string(),
                )],
            )
            .await
            .unwrap_err();
        assert_eq!(error.failure, ResourceFailure::MimeNotAllowed);
    }

    #[tokio::test]
    async fn url_materialization_produces_safe_codec_handoff() {
        let manager = manager(
            Arc::new(RecordingAuthorizer::default()),
            Arc::new(FakeStore::default()),
            ResourceLimits {
                allowed_mime_types: vec!["image/*".to_string()],
                ..ResourceLimits::default()
            },
        );
        let source = ResourceRef::url(
            "https://assets.example/image.png".to_string(),
            Some("image/png".to_string()),
        );
        let inspected = manager
            .inspect(&context(), std::slice::from_ref(&source))
            .await
            .unwrap();
        let materialized = manager
            .materialize_after_provider_selected(&context(), "selected", inspected)
            .await
            .unwrap();
        let parts = materialized
            .into_iter()
            .next()
            .unwrap()
            .into_codec_parts()
            .unwrap();
        assert_eq!(parts.key, ResourceKey::from_ref(&source));
        assert_eq!(parts.bytes, b"url-data");
        assert_eq!(parts.mime, "image/png");
        assert_eq!(parts.file_name.as_deref(), Some("image.png"));
        assert!(!format!("{parts:?}").contains("url-data"));
    }

    #[test]
    fn stable_keys_cover_all_refs_and_missing_resources_fail_closed_without_secrets() {
        let credential = "credential-super-secret";
        let base64_content = "YmFzZTY0LXN1cGVyLXNlY3JldA==";
        let refs = [
            ResourceRef::url(
                "https://assets.example/private-name.png".to_string(),
                Some("image/png".to_string()),
            ),
            ResourceRef::base64("image/png".to_string(), base64_content.to_string()),
            ResourceRef::NamedObject {
                obj_id: obj_id_for(&FileObject::new(
                    "secret-name".to_string(),
                    1,
                    "sha256:content".to_string(),
                )),
            },
        ];
        assert_eq!(
            ResourceKey::from_ref(&refs[0]).as_str(),
            "aicc-resource-v1:url:6e4aeb88c12e7bdc80c938d118eab0de4980479b0bff852221cc250a9295c2ab"
        );
        assert_eq!(
            ResourceKey::from_ref(&refs[1]).as_str(),
            "aicc-resource-v1:base64:9b3239901c49146e7642a71a4d3f6686d969f98a563d57386686ad65844caf65"
        );
        for resource in &refs {
            let key = ResourceKey::from_ref(resource);
            assert_eq!(key, ResourceKey::from_ref(resource));
            assert!(!key.as_str().contains(base64_content));
            assert!(!key.as_str().contains("private-name"));
            let error =
                require_materialized::<CodecResourceParts>(&BTreeMap::new(), resource).unwrap_err();
            let rendered = format!("{error:?} {error}");
            assert_eq!(error.failure, ResourceFailure::PhaseViolation);
            assert!(!rendered.contains(base64_content));
            assert!(!rendered.contains(credential));
        }
        assert_ne!(
            ResourceKey::from_ref(&refs[0]),
            ResourceKey::from_ref(&refs[1])
        );
        assert_ne!(
            ResourceKey::from_ref(&refs[1]),
            ResourceKey::from_ref(&refs[2])
        );
    }

    #[tokio::test]
    async fn rejects_invalid_urls_base64_counts_sizes_and_phase_violation() {
        let limits = ResourceLimits {
            max_resources: 1,
            max_single_bytes: 8,
            max_total_bytes: 8,
            max_inline_bytes: 4,
            ..ResourceLimits::default()
        };
        let manager = manager(
            Arc::new(RecordingAuthorizer::default()),
            Arc::new(FakeStore::default()),
            limits,
        );
        let local = manager
            .inspect(
                &context(),
                &[ResourceRef::url("https://127.0.0.1/a".to_string(), None)],
            )
            .await
            .unwrap_err();
        assert_eq!(local.failure, ResourceFailure::InvalidReference);

        let base64 = manager
            .inspect(
                &context(),
                &[ResourceRef::base64(
                    "image/png".to_string(),
                    "not-base64".to_string(),
                )],
            )
            .await
            .unwrap_err();
        assert_eq!(base64.failure, ResourceFailure::Base64Invalid);

        let count = manager
            .inspect(
                &context(),
                &[
                    ResourceRef::base64("image/png".to_string(), "YQ==".to_string()),
                    ResourceRef::base64("image/png".to_string(), "Yg==".to_string()),
                ],
            )
            .await
            .unwrap_err();
        assert_eq!(count.failure, ResourceFailure::CountExceeded);

        let inspected = manager
            .inspect(
                &context(),
                &[ResourceRef::base64(
                    "image/png".to_string(),
                    "YQ==".to_string(),
                )],
            )
            .await
            .unwrap();
        let phase = manager
            .materialize_after_provider_selected(&context(), "", inspected)
            .await
            .unwrap_err();
        assert_eq!(phase.failure, ResourceFailure::PhaseViolation);
    }

    #[tokio::test]
    async fn authorization_failure_is_stable_resource_invalid() {
        let authorizer = Arc::new(RecordingAuthorizer {
            operations: Mutex::new(Vec::new()),
            denied: Some(ResourceAccessOperation::ReadContent),
        });
        let manager = manager(
            authorizer,
            Arc::new(FakeStore::default()),
            ResourceLimits::default(),
        );
        let inspected = manager
            .inspect(
                &context(),
                &[ResourceRef::base64(
                    "image/png".to_string(),
                    "YQ==".to_string(),
                )],
            )
            .await
            .unwrap();
        let error = manager
            .materialize_after_provider_selected(&context(), "selected", inspected)
            .await
            .unwrap_err();
        assert_eq!(error.to_aicc_error().code, AiccErrorCode::ResourceInvalid);
        assert!(!error.to_aicc_error().retriable);
    }

    #[tokio::test]
    async fn rejects_content_that_conflicts_with_declared_mime() {
        let manager = manager(
            Arc::new(RecordingAuthorizer::default()),
            Arc::new(FakeStore::default()),
            ResourceLimits::default(),
        );
        let png = BASE64_STANDARD.encode(b"\x89PNG\r\n\x1a\ncontent");
        let inspected = manager
            .inspect(
                &context(),
                &[ResourceRef::base64("image/jpeg".to_string(), png)],
            )
            .await
            .unwrap();
        let error = manager
            .materialize_after_provider_selected(&context(), "selected", inspected)
            .await
            .unwrap_err();
        assert_eq!(error.failure, ResourceFailure::MimeMismatch);
    }

    #[test]
    fn zip_archive_limits_path_traversal_file_count_ratio_and_depth() {
        let safe = zip_bytes(&[("safe/file.txt", b"hello")]);
        let metadata = ResourceMetadata {
            kind: ResourceKind::Base64,
            obj_id: None,
            mime: Some("application/zip".to_string()),
            size_bytes: Some(safe.len() as u64),
            digest: None,
            file_name: Some("safe.zip".to_string()),
            attributes: Map::new(),
        };
        inspect_archive(&safe, &metadata, &ResourceLimits::default()).unwrap();

        let traversal = zip_bytes(&[("../escape.txt", b"bad")]);
        assert_eq!(
            inspect_archive(&traversal, &metadata, &ResourceLimits::default())
                .unwrap_err()
                .failure,
            ResourceFailure::ArchivePathTraversal
        );

        let two_files = zip_bytes(&[("one", b"1"), ("two", b"2")]);
        assert_eq!(
            inspect_archive(
                &two_files,
                &metadata,
                &ResourceLimits {
                    max_archive_files: 1,
                    ..ResourceLimits::default()
                }
            )
            .unwrap_err()
            .failure,
            ResourceFailure::ArchiveFileCountExceeded
        );

        let bomb = zip_bytes(&[("bomb", &[0u8; 4096])]);
        assert_eq!(
            inspect_archive(
                &bomb,
                &metadata,
                &ResourceLimits {
                    max_archive_expansion_ratio: 2,
                    ..ResourceLimits::default()
                }
            )
            .unwrap_err()
            .failure,
            ResourceFailure::ArchiveExpansionExceeded
        );

        let nested = zip_bytes(&[("inner.zip", &safe)]);
        assert_eq!(
            inspect_archive(
                &nested,
                &metadata,
                &ResourceLimits {
                    max_archive_depth: 1,
                    ..ResourceLimits::default()
                }
            )
            .unwrap_err()
            .failure,
            ResourceFailure::ArchiveDepthExceeded
        );

        let encrypted = mark_zip_encrypted(zip_bytes(&[("secret.txt", b"secret")]));
        assert_eq!(
            inspect_archive(&encrypted, &metadata, &ResourceLimits::default())
                .unwrap_err()
                .failure,
            ResourceFailure::ArchiveEncrypted
        );
    }

    #[tokio::test]
    async fn writes_named_artifact_with_embedding_metadata() {
        let authorizer = Arc::new(RecordingAuthorizer::default());
        let store = Arc::new(FakeStore::default());
        let manager = manager(authorizer.clone(), store, ResourceLimits::default());
        let artifact = manager
            .write_artifact(
                &context(),
                b"embedding-data",
                ArtifactSpec {
                    name: "embedding.bin".to_string(),
                    mime: "application/octet-stream".to_string(),
                    attributes: Map::new(),
                    embedding: Some(EmbeddingArtifactMetadata {
                        rows: 2,
                        dimensions: 1536,
                        space: "cosine".to_string(),
                    }),
                },
            )
            .await
            .unwrap();
        assert!(matches!(artifact.resource, ResourceRef::NamedObject { .. }));
        let metadata = artifact.metadata.unwrap();
        assert_eq!(metadata["rows"], json!(2));
        assert_eq!(metadata["dimensions"], json!(1536));
        assert_eq!(metadata["space"], json!("cosine"));
        assert_eq!(
            authorizer.operations.lock().unwrap().as_slice(),
            &[ResourceAccessOperation::WriteArtifact]
        );

        let reserved = manager
            .write_artifact(
                &context(),
                b"data",
                ArtifactSpec {
                    name: "bad.bin".to_string(),
                    mime: "application/octet-stream".to_string(),
                    attributes: Map::from_iter([("digest".to_string(), json!("forged"))]),
                    embedding: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(reserved.failure, ResourceFailure::ArtifactInvalid);
    }

    #[test]
    fn multipart_and_debug_never_expose_content() {
        let source = ResourceRef::base64("text/plain".to_string(), "c3VwZXItc2VjcmV0".to_string());
        let resource = MaterializedResource {
            key: ResourceKey::from_ref(&source),
            metadata: ResourceMetadata {
                kind: ResourceKind::Base64,
                obj_id: None,
                mime: Some("text/plain".to_string()),
                size_bytes: Some(12),
                digest: Some("secret-digest".to_string()),
                file_name: Some("input.txt".to_string()),
                attributes: Map::new(),
            },
            bytes: b"super-secret".to_vec(),
        };
        let debug = format!("{resource:?}");
        assert!(!debug.contains("super-secret"));
        assert!(!debug.contains("secret-digest"));
        multipart_form([("file".to_string(), resource)]).unwrap();
    }

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn mark_zip_encrypted(mut bytes: Vec<u8>) -> Vec<u8> {
        let mut offset = 0;
        while offset + 10 <= bytes.len() {
            let flag_offset = if bytes[offset..].starts_with(b"PK\x03\x04") {
                Some(offset + 6)
            } else if bytes[offset..].starts_with(b"PK\x01\x02") {
                Some(offset + 8)
            } else {
                None
            };
            if let Some(flag_offset) = flag_offset {
                let flags = u16::from_le_bytes([bytes[flag_offset], bytes[flag_offset + 1]]) | 1;
                bytes[flag_offset..flag_offset + 2].copy_from_slice(&flags.to_le_bytes());
            }
            offset += 1;
        }
        bytes
    }
}
