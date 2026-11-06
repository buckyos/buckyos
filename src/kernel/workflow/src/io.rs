//! [`NativeIo`] implementations: an in-memory one for unit tests and the zone
//! one backed by the local Named Store and the workflow artifacts bucket.

use crate::native::{
    ArtifactDelivery, ArtifactRef, NativeIo, ToolInput, ToolInputLease, ToolInputRequest, FILE_PROTOCOL_WHITELIST, NDN_PROTOCOL_WHITELIST,
};
use crate::object_store::{InMemoryObjectStore, WorkflowObjectStore};
use crate::persistence::ArtifactIndex;
use async_trait::async_trait;
use buckyos_api::{error_codes, ThunkExecutionError};
use ndn_lib::{ChunkHasher, ChunkId, ChunkList, FileObject, NamedObject, ObjId};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Bytes -> content id (chunk or chunklist) + chunk pieces, using the same
/// chunking as `BucketClient::put_reader`, so identical bytes get identical
/// content ids no matter who stored them.
pub fn chunk_bytes(bytes: &[u8]) -> Result<(String, Vec<(ChunkId, Vec<u8>)>, Option<(ObjId, String)>), ThunkExecutionError> {
    let chunk_size = ndn_lib::CHUNK_DEFAULT_SIZE as usize;
    let mut pieces = Vec::new();
    let mut offset = 0usize;
    loop {
        let end = (offset + chunk_size).min(bytes.len());
        let piece = &bytes[offset..end];
        let hasher = ChunkHasher::new(None).map_err(|e| internal(format!("chunk hasher: {e}")))?;
        let method = hasher.hash_method;
        let hash = hasher.calc_from_bytes(piece);
        let chunk_id = ChunkId::from_mix_hash_result_by_hash_method(piece.len() as u64, &hash, method)
            .map_err(|e| internal(format!("chunk id: {e}")))?;
        pieces.push((chunk_id, piece.to_vec()));
        offset = end;
        if offset >= bytes.len() {
            break;
        }
    }
    if pieces.len() == 1 {
        return Ok((pieces[0].0.to_string(), pieces, None));
    }
    let list = ChunkList::from_chunk_list(pieces.iter().map(|(id, _)| id.clone()).collect())
        .map_err(|e| internal(format!("chunk list: {e}")))?;
    let (list_id, list_str) = list.gen_obj_id();
    Ok((list_id.to_string(), pieces, Some((list_id, list_str))))
}

/// Deterministic artifact FileObject: zero time fields, stable name, no owner.
pub fn artifact_file_object(name: &str, size: u64, content_id: &str) -> FileObject {
    let mut file = FileObject::default();
    file.content_obj.name = name.to_string();
    file.content_obj.create_time = 0;
    file.content_obj.last_update_time = 0;
    file.size = size;
    file.content = content_id.to_string();
    file
}

fn internal(message: String) -> ThunkExecutionError {
    ThunkExecutionError::new(error_codes::INTERNAL, message)
}

fn input_unavailable(content_id: &str, why: impl std::fmt::Display) -> ThunkExecutionError {
    ThunkExecutionError::new(
        error_codes::INPUT_UNAVAILABLE,
        format!("content `{content_id}` is not readable: {why}"),
    )
    .with_details(json!({"content_id": content_id}))
}

fn too_large(content_id: &str, size: u64, max: u64) -> ThunkExecutionError {
    ThunkExecutionError::new(
        error_codes::RESOURCE_LIMIT,
        format!("content `{content_id}` is {size} bytes, limit {max}"),
    )
    .with_details(json!({"content_id": content_id, "size": size, "max_input_bytes": max}))
}

/// Test / in-process IO: contents and artifacts live in memory, objects in an
/// [`InMemoryObjectStore`].
pub struct MemoryNativeIo {
    objects: Arc<InMemoryObjectStore>,
    contents: Mutex<HashMap<String, Vec<u8>>>,
    artifacts: Mutex<HashMap<String, ArtifactRef>>,
    deleted: Mutex<Vec<String>>,
}

impl MemoryNativeIo {
    pub fn new(objects: Arc<InMemoryObjectStore>) -> Self {
        Self {
            objects,
            contents: Mutex::new(HashMap::new()),
            artifacts: Mutex::new(HashMap::new()),
            deleted: Mutex::new(Vec::new()),
        }
    }

    /// Stores bytes as content; returns `(file_obj_id, content_id)` so tests
    /// can feed a FileObject id as run input.
    pub async fn add_file(&self, name: &str, bytes: &[u8]) -> (String, String) {
        let (content_id, _, _) = chunk_bytes(bytes).unwrap();
        self.contents
            .lock()
            .unwrap()
            .insert(content_id.clone(), bytes.to_vec());
        let mut file = artifact_file_object(name, bytes.len() as u64, &content_id);
        file.content_obj.create_time = 1;
        let (obj_id, obj_str) = file.gen_obj_id();
        let value: Value = serde_json::from_str(&obj_str).unwrap();
        self.objects
            .put_json_with_id(&obj_id.to_string(), &value)
            .await
            .unwrap();
        (obj_id.to_string(), content_id)
    }

    pub fn artifact_bytes(&self, obj_id: &str) -> Option<Vec<u8>> {
        let artifacts = self.artifacts.lock().unwrap();
        let content_id = artifacts.get(obj_id)?.content_id.clone();
        drop(artifacts);
        self.contents.lock().unwrap().get(&content_id).cloned()
    }

    pub fn artifacts(&self) -> Vec<ArtifactRef> {
        self.artifacts.lock().unwrap().values().cloned().collect()
    }

    /// Simulates an artifact expiring from the bucket.
    pub fn delete_artifact(&self, obj_id: &str) {
        self.deleted.lock().unwrap().push(obj_id.to_string());
    }

    pub fn objects(&self) -> Arc<InMemoryObjectStore> {
        self.objects.clone()
    }

    async fn store_bytes(
        &self,
        key: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<ArtifactRef, ThunkExecutionError> {
        let (content_id, _, _) = chunk_bytes(&bytes)?;
        let size = bytes.len() as u64;
        self.contents
            .lock()
            .unwrap()
            .insert(content_id.clone(), bytes);
        let file = artifact_file_object(name, size, &content_id);
        let (obj_id, obj_str) = file.gen_obj_id();
        let value: Value = serde_json::from_str(&obj_str).map_err(|e| internal(e.to_string()))?;
        self.objects
            .put_json_with_id(&obj_id.to_string(), &value)
            .await
            .map_err(|e| internal(e.to_string()))?;
        let artifact = ArtifactRef {
            obj_id: obj_id.to_string(),
            content_id,
            key: key.to_string(),
            size,
        };
        self.artifacts
            .lock()
            .unwrap()
            .insert(artifact.obj_id.clone(), artifact.clone());
        Ok(artifact)
    }
}

#[async_trait]
impl NativeIo for MemoryNativeIo {
    async fn content_size(&self, content_id: &str) -> Result<Option<u64>, ThunkExecutionError> {
        Ok(self
            .contents
            .lock()
            .unwrap()
            .get(content_id)
            .map(|b| b.len() as u64))
    }

    async fn read_content(&self, content_id: &str, max_bytes: u64) -> Result<Vec<u8>, ThunkExecutionError> {
        let bytes = self
            .contents
            .lock()
            .unwrap()
            .get(content_id)
            .cloned()
            .ok_or_else(|| input_unavailable(content_id, "not found"))?;
        if bytes.len() as u64 > max_bytes {
            return Err(too_large(content_id, bytes.len() as u64, max_bytes));
        }
        Ok(bytes)
    }

    async fn materialize(&self, content_id: &str, path: &Path, max_bytes: u64) -> Result<u64, ThunkExecutionError> {
        let bytes = self.read_content(content_id, max_bytes).await?;
        tokio::fs::write(path, &bytes)
            .await
            .map_err(|e| internal(format!("write {}: {e}", path.display())))?;
        Ok(bytes.len() as u64)
    }

    async fn open_tool_input(&self, request: &ToolInputRequest<'_>) -> Result<ToolInput, ThunkExecutionError> {
        let size = self.materialize(request.content_id, request.temp_path, request.max_bytes).await?;
        Ok(ToolInput {
            location: request.temp_path.to_string_lossy().into_owned(),
            size,
            protocols: FILE_PROTOCOL_WHITELIST,
            secrets: Vec::new(),
            lease: None,
        })
    }

    async fn release_tool_input(&self, _lease: &ToolInputLease) {}

    async fn put_artifact(&self, key: &str, name: &str, _content_type: &str, bytes: Vec<u8>) -> Result<ArtifactRef, ThunkExecutionError> {
        self.store_bytes(key, name, bytes).await
    }

    async fn put_artifact_file(&self, key: &str, name: &str, content_type: &str, path: &Path) -> Result<ArtifactRef, ThunkExecutionError> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| internal(format!("read {}: {e}", path.display())))?;
        self.put_artifact(key, name, content_type, bytes).await
    }

    async fn put_object(&self, obj_type: &str, value: &Value) -> Result<String, ThunkExecutionError> {
        self.objects
            .put_json(obj_type, value)
            .await
            .map_err(|e| internal(e.to_string()))
    }

    async fn get_object(&self, obj_id: &str) -> Result<Option<Value>, ThunkExecutionError> {
        self.objects
            .get_json(obj_id)
            .await
            .map_err(|e| internal(e.to_string()))
    }

    async fn artifact_available(&self, obj_id: &str) -> bool {
        if self.deleted.lock().unwrap().iter().any(|d| d == obj_id) {
            return false;
        }
        if self.artifacts.lock().unwrap().contains_key(obj_id) {
            return true;
        }
        self.objects.exists(obj_id).await.unwrap_or(false)
    }

    async fn deliver(&self, _principal: &str, obj_ids: &[String], expires_at: i64) -> Vec<ArtifactDelivery> {
        let artifacts = self.artifacts.lock().unwrap();
        obj_ids
            .iter()
            .map(|obj_id| ArtifactDelivery {
                obj_id: obj_id.clone(),
                key: artifacts.get(obj_id).map(|a| a.key.clone()),
                locator: None,
                path: None,
                token: None,
                expires_at,
                error: None,
            })
            .collect()
    }
}

/// Zone IO: inputs and artifacts in the local Named Store, artifact bindings
/// in the workflow artifacts bucket.
pub struct ZoneNativeIo {
    store: named_store::NamedDataMgr,
    objects: Arc<dyn WorkflowObjectStore>,
    bucket: bucket_store::types::BucketId,
    artifact_ttl_days: u32,
    index: Arc<dyn ArtifactIndex>,
}

impl ZoneNativeIo {
    pub fn new(
        store: named_store::NamedDataMgr,
        objects: Arc<dyn WorkflowObjectStore>,
        bucket: bucket_store::types::BucketId,
        artifact_ttl_days: u32,
        index: Arc<dyn ArtifactIndex>,
    ) -> Self {
        Self {
            store,
            objects,
            bucket,
            artifact_ttl_days,
            index,
        }
    }

    /// Holds the artifact FileObject and its content (chunk / chunklist +
    /// chunks) for the artifact TTL; the bucket binding is the second holder.
    async fn pin_artifact(&self, obj_id: &ObjId) -> Result<(), ThunkExecutionError> {
        let ttl = std::time::Duration::from_secs(u64::from(self.artifact_ttl_days) * 86_400);
        self.store
            .pin(obj_id, crate::object_store::WORKFLOW_PIN_OWNER, named_store::PinScope::Recursive, Some(ttl))
            .await
            .map_err(|e| internal(format!("pin artifact {obj_id}: {e}")))
    }

    async fn bucket_client(&self) -> Result<buckyos_api::BucketClient, ThunkExecutionError> {
        let runtime = buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| internal(format!("runtime: {e}")))?;
        runtime
            .get_bucket_client()
            .await
            .map_err(|e| internal(format!("bucket client: {e}")))
    }

    /// The FileObject a tool reads `content_id` through: the run's input when that is
    /// what this step works on, else a plain one over the content (a derived input).
    async fn tool_input_file(&self, source_obj_id: Option<&str>, content_id: &str, size: u64) -> Result<ObjId, ThunkExecutionError> {
        if let Some(id) = source_obj_id.and_then(|s| ObjId::new(s).ok()) {
            let file = self.store.get_object(&id).await.ok().and_then(|body| ndn_lib::load_named_obj::<FileObject>(&body).ok());
            if file.is_some_and(|f| f.content == content_id) {
                return Ok(id);
            }
        }
        let (id, body) = artifact_file_object("input", size, content_id).gen_obj_id();
        self.store.put_object(&id, &body).await.map_err(|e| internal(format!("put input object: {e}")))?;
        Ok(id)
    }

    async fn open(&self, content_id: &str) -> Result<(ndn_lib::ChunkReader, u64), ThunkExecutionError> {
        let obj_id = ObjId::new(content_id).map_err(|e| input_unavailable(content_id, e))?;
        if !obj_id.is_chunk() && !obj_id.is_chunk_list() {
            return Err(input_unavailable(content_id, "not a chunk or chunk list"));
        }
        self.store
            .open_reader(&obj_id, None)
            .await
            .map_err(|e| input_unavailable(content_id, e))
    }

    async fn bind(
        &self,
        key: &str,
        name: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<ArtifactRef, ThunkExecutionError> {
        let (content_id, pieces, list) = chunk_bytes(&bytes)?;
        let size = bytes.len() as u64;
        for (chunk_id, piece) in &pieces {
            if !self.store.have_chunk(chunk_id).await {
                self.store
                    .put_chunk(chunk_id, piece)
                    .await
                    .map_err(|e| internal(format!("put chunk: {e}")))?;
            }
        }
        if let Some((list_id, list_str)) = list {
            self.store
                .put_object(&list_id, &list_str)
                .await
                .map_err(|e| internal(format!("put chunk list: {e}")))?;
        }
        let file = artifact_file_object(name, size, &content_id);
        let (obj_id, obj_str) = file.gen_obj_id();
        self.store
            .put_object(&obj_id, &obj_str)
            .await
            .map_err(|e| internal(format!("put file object: {e}")))?;
        self.pin_artifact(&obj_id).await?;
        let client = self.bucket_client().await?;
        let expires_at = chrono::Utc::now().timestamp_millis() + i64::from(self.artifact_ttl_days) * 86_400_000;
        let req = bucket_store::types::PutRequest {
            key: key.to_string(),
            obj_id: obj_id.to_string(),
            content_type: Some(content_type.to_string()),
            expires_at: Some(expires_at),
            ..Default::default()
        };
        client
            .put(&self.bucket, &req)
            .await
            .map_err(|e| internal(format!("bind artifact {key}: {e}")))?;
        self.index
            .record_artifact(&obj_id.to_string(), key)
            .await
            .map_err(|e| internal(format!("index artifact {key}: {e}")))?;
        Ok(ArtifactRef {
            obj_id: obj_id.to_string(),
            content_id,
            key: key.to_string(),
            size,
        })
    }
}

#[async_trait]
impl NativeIo for ZoneNativeIo {
    async fn content_size(&self, content_id: &str) -> Result<Option<u64>, ThunkExecutionError> {
        let (_, size) = self.open(content_id).await?;
        Ok(Some(size))
    }

    async fn read_content(&self, content_id: &str, max_bytes: u64) -> Result<Vec<u8>, ThunkExecutionError> {
        let (mut reader, size) = self.open(content_id).await?;
        if size > max_bytes {
            return Err(too_large(content_id, size, max_bytes));
        }
        let mut bytes = Vec::with_capacity(size as usize);
        (&mut reader)
            .take(max_bytes + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| input_unavailable(content_id, e))?;
        if bytes.len() as u64 > max_bytes {
            return Err(too_large(content_id, bytes.len() as u64, max_bytes));
        }
        Ok(bytes)
    }

    async fn materialize(&self, content_id: &str, path: &Path, max_bytes: u64) -> Result<u64, ThunkExecutionError> {
        let (mut reader, size) = self.open(content_id).await?;
        if size > max_bytes {
            return Err(too_large(content_id, size, max_bytes));
        }
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|e| internal(format!("create {}: {e}", path.display())))?;
        let copied = tokio::io::copy(&mut (&mut reader).take(max_bytes + 1), &mut file)
            .await
            .map_err(|e| input_unavailable(content_id, e))?;
        file.flush().await.map_err(|e| internal(e.to_string()))?;
        if copied > max_bytes {
            return Err(too_large(content_id, copied, max_bytes));
        }
        Ok(copied)
    }

    /// `/ndn/<file_id>?hint={"token":...}` (bucket design §21 N17). The FileObject is
    /// bound at `inputs/<execution_id>`, which also holds it while the step runs, and
    /// the token reads that one key until the deadline.
    async fn open_tool_input(&self, request: &ToolInputRequest<'_>) -> Result<ToolInput, ThunkExecutionError> {
        let (_, size) = self.open(request.content_id).await?;
        if size > request.max_bytes {
            return Err(too_large(request.content_id, size, request.max_bytes));
        }
        let file_id = self.tool_input_file(request.source_obj_id, request.content_id, size).await?;
        let client = self.bucket_client().await?;
        let scratch = buckyos_api::workflow_scratch_bucket();
        let key = format!("inputs/{}", request.execution_id);
        let now = chrono::Utc::now().timestamp_millis();
        // Tokens live at most 7 days; a step's deadline is far shorter.
        let expires_at = request.expires_at_ms.clamp(now + 60_000, now + 7 * 86_400_000 - 60_000);
        let put = bucket_store::types::PutRequest {
            key: key.clone(),
            obj_id: file_id.to_string(),
            expires_at: Some(expires_at),
            ..Default::default()
        };
        client.put(&scratch, &put).await.map_err(|e| input_unavailable(request.content_id, format!("bind input: {e}")))?;
        let token = bucket_store::TokenRequest {
            prefix: None,
            key: Some(key.clone()),
            actions: [bucket_store::Action::Read].into_iter().collect(),
            expires_at,
            max_uses: None,
            bind_user_id: None,
            bind_app_id: None,
        };
        let issued = match client.create_access_token(&scratch, &token).await {
            Ok(issued) => issued,
            Err(e) => {
                let _ = client.delete(&scratch, &key, None).await;
                return Err(input_unavailable(request.content_id, format!("input token: {e}")));
            }
        };
        let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| internal(format!("runtime: {e}")))?;
        let hint = ndn_lib::NdnHint { path: None, token: Some(issued.token.clone()) };
        let url = runtime.ndn_read_url(None, &file_id, Some(&hint));
        Ok(ToolInput {
            secrets: vec![url.clone(), issued.token],
            location: url,
            size,
            protocols: NDN_PROTOCOL_WHITELIST,
            lease: Some(ToolInputLease { key, token_id: issued.token_id }),
        })
    }

    async fn release_tool_input(&self, lease: &ToolInputLease) {
        let client = match self.bucket_client().await {
            Ok(client) => client,
            Err(e) => {
                log::warn!("release input {}: {}", lease.key, e.message);
                return;
            }
        };
        if let Err(e) = client.revoke_access_token(&lease.token_id).await {
            log::warn!("revoke input token of {}: {e}", lease.key);
        }
        if let Err(e) = client.delete(&buckyos_api::workflow_scratch_bucket(), &lease.key, None).await {
            log::warn!("unbind input {}: {e}", lease.key);
        }
    }

    async fn put_artifact(&self, key: &str, name: &str, content_type: &str, bytes: Vec<u8>) -> Result<ArtifactRef, ThunkExecutionError> {
        self.bind(key, name, content_type, bytes).await
    }

    async fn put_artifact_file(&self, key: &str, name: &str, content_type: &str, path: &Path) -> Result<ArtifactRef, ThunkExecutionError> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| internal(format!("read {}: {e}", path.display())))?;
        self.bind(key, name, content_type, bytes).await
    }

    async fn put_object(&self, obj_type: &str, value: &Value) -> Result<String, ThunkExecutionError> {
        self.objects
            .put_json(obj_type, value)
            .await
            .map_err(|e| internal(e.to_string()))
    }

    async fn get_object(&self, obj_id: &str) -> Result<Option<Value>, ThunkExecutionError> {
        self.objects
            .get_json(obj_id)
            .await
            .map_err(|e| internal(e.to_string()))
    }

    async fn artifact_available(&self, obj_id: &str) -> bool {
        let Ok(id) = ObjId::new(obj_id) else {
            return false;
        };
        let Ok(body) = self.store.get_object(&id).await else {
            return false;
        };
        if !id.is_file_object() {
            return true;
        }
        let Ok(file) = ndn_lib::load_named_obj::<FileObject>(&body) else {
            return false;
        };
        let Ok(content) = ObjId::new(&file.content) else {
            return file.size == 0;
        };
        let available = self
            .store
            .is_object_stored(&content, None)
            .await
            .unwrap_or(false);
        if available {
            let _ = self.pin_artifact(&id).await;
        }
        available
    }

    async fn deliver(&self, principal: &str, obj_ids: &[String], expires_at: i64) -> Vec<ArtifactDelivery> {
        let (user_id, app_id) = principal.split_once('/').unwrap_or((principal, ""));
        let client = self.bucket_client().await;
        let mut out = Vec::with_capacity(obj_ids.len());
        for obj_id in obj_ids {
            let mut delivery = ArtifactDelivery {
                obj_id: obj_id.clone(),
                key: None,
                locator: None,
                path: None,
                token: None,
                expires_at,
                error: None,
            };
            let key = match self.index.artifact_key(obj_id).await {
                Ok(Some(key)) => key,
                Ok(None) => {
                    out.push(delivery);
                    continue;
                }
                Err(err) => {
                    delivery.error = Some(err.to_string());
                    out.push(delivery);
                    continue;
                }
            };
            delivery.locator = Some(buckyos_api::bucket_obj_url(&self.bucket, &key));
            delivery.key = Some(key.clone());
            match &client {
                Ok(client) => {
                    let request = bucket_store::TokenRequest {
                        prefix: None,
                        key: Some(key),
                        actions: [bucket_store::Action::Read].into_iter().collect(),
                        expires_at: expires_at.saturating_mul(1000),
                        max_uses: None,
                        bind_user_id: Some(user_id.to_string()),
                        bind_app_id: (!app_id.is_empty()).then(|| app_id.to_string()),
                    };
                    match client.create_access_token(&self.bucket, &request).await {
                        Ok(issued) => {
                            delivery.path = Some(issued.path);
                            delivery.token = Some(issued.token);
                            delivery.expires_at = issued.expires_at / 1000;
                        }
                        Err(err) => delivery.error = Some(err.to_string()),
                    }
                }
                Err(err) => delivery.error = Some(err.to_string()),
            }
            out.push(delivery);
        }
        out
    }
}
