//! The Workspaces' objects: bytes and NamedObjects live in the zone's named store, and what a Workspace
//! keeps is a binding in one of the service's application object buckets (doc/workspace
//! `BuckyOS AiWorkspace Durable Data Schema.md`). Ids and encodings are those of ndn-lib (`type:hex`).
//!
//! The store is synchronous like the rest of this crate: every named-store or bucket call runs on one
//! process-wide runtime and the caller waits for it, so it can be used from blocking threads, plain test
//! threads and async workers alike.

use aiworkspace_core::canonical::{
    self, chunk_id, file_object, obj_id_to_filename, parse_strict, DirMember, ObjId, MAX_CHUNK_SIZE,
};
use aiworkspace_core::materialize::{ObjectSink, ObjectSource};
use aiworkspace_core::{Code, WsError, WsResult};
use bucket_store::{BucketDecl, BucketError, BucketId, BucketStore, Caller as BucketCaller, PutRequest, TokenRequest};
use named_store::NamedDataMgr;
use ndn_lib::{ChunkId, NdnError};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::AsyncReadExt;

/// The service's app id: its buckets are `user/aiworkspace/<owner>/<name>`.
pub const APPID: &str = "aiworkspace";
/// Assets (FileObjects) under `<ws>/<object_id>`.
pub const ASSETS: &str = "assets";
/// Retention DirObjects of snapshots (`<ws>/snapshot/<id>`) and generated versions (`<ws>/version/<epoch>/<seq>`).
pub const OBJECTS: &str = "objects";
/// Packages, replicas and fixed publications (7 days).
pub const EXPORTS: &str = "exports";

/// A bucket of the Workspace owner `owner`.
pub fn bucket_id(owner: &str, name: &str) -> WsResult<String> {
    BucketId::user(APPID, owner, name)
        .map(|id| id.to_string())
        .map_err(|_| WsError::new(Code::DependencyUnavailable, format!("{owner} has no {name} bucket")))
}

/// Bindings as the service sees them (kRPC `bucket.*` at nfs-server in a Zone, the lib in-process otherwise).
#[async_trait::async_trait]
pub trait Buckets: Send + Sync {
    /// Bind `obj_id` under `key`; the binding's version.
    async fn put(&self, bucket: &str, key: &str, obj_id: &str) -> WsResult<u64>;
    /// `(obj_id, version)` of the current binding, `None` when there is none.
    async fn get(&self, bucket: &str, key: &str) -> WsResult<Option<(String, u64)>>;
    /// Unbind; a missing key is fine.
    async fn delete(&self, bucket: &str, key: &str) -> WsResult<()>;
    /// A single-key read token (`bkt_…`) for the data plane, valid for `ttl_ms` from now (the bucket service's clock).
    async fn read_token(&self, bucket: &str, key: &str, ttl_ms: i64) -> WsResult<String>;
}

pub fn bucket_err(e: BucketError) -> WsError {
    let code = match &e {
        BucketError::QuotaExceeded(_) | BucketError::ObjectTooLarge { .. } => Code::StorageFull,
        BucketError::ContentNotAvailable(_) | BucketError::ContentEvicted(_) | BucketError::NotHeld(_) => Code::DependencyUnavailable,
        BucketError::BucketNotFound(_) | BucketError::NoSuchKey(_) => Code::NotFound,
        _ => Code::StorageIoError,
    };
    WsError::new(code, format!("bucket: {e}"))
}

/// The bucket lib in this process (standalone servers without nfs-server, tests): buckets are created from
/// the declarations on first use, like nfs-server does from the service spec.
pub struct LocalBuckets {
    store: Arc<BucketStore>,
    decls: HashMap<String, BucketDecl>,
    quota_mib: Option<u64>,
    ensured: Mutex<HashSet<String>>,
}

impl LocalBuckets {
    pub fn new(store: Arc<BucketStore>, decls: HashMap<String, BucketDecl>, quota_mib: Option<u64>) -> LocalBuckets {
        LocalBuckets { store, decls, quota_mib, ensured: Mutex::new(HashSet::new()) }
    }

    fn bucket(&self, id: &str) -> WsResult<BucketId> {
        let id: BucketId = id.parse().map_err(bucket_err)?;
        let owner_key = format!("{}@{}", id.appid, id.owner);
        if self.ensured.lock().unwrap().contains(&owner_key) {
            return Ok(id);
        }
        for (name, decl) in &self.decls {
            self.store.ensure_bucket(&BucketId::new(decl.scope, &id.appid, &id.owner, name).map_err(bucket_err)?, decl).map_err(bucket_err)?;
        }
        if self.decls.values().any(BucketDecl::needs_app_pool) {
            self.store.set_declared_pool_quota(&id.appid, &id.owner, self.quota_mib.map(|m| m * 1024 * 1024)).map_err(bucket_err)?;
        }
        self.ensured.lock().unwrap().insert(owner_key);
        Ok(id)
    }
}

#[async_trait::async_trait]
impl Buckets for LocalBuckets {
    async fn put(&self, bucket: &str, key: &str, obj_id: &str) -> WsResult<u64> {
        let id = self.bucket(bucket)?;
        let req = PutRequest { key: key.to_string(), obj_id: obj_id.to_string(), ..Default::default() };
        let r = self.store.put(&id, &BucketCaller::system(), req).await.map_err(bucket_err)?;
        log::info!("bucket bind {bucket} key={key} obj_id={obj_id} version={}", r.version);
        Ok(r.version)
    }

    async fn get(&self, bucket: &str, key: &str) -> WsResult<Option<(String, u64)>> {
        let id = self.bucket(bucket)?;
        match self.store.get(&id, &BucketCaller::system(), key, None).await {
            Ok(b) if b.is_delete_marker => Ok(None),
            Ok(b) => Ok(b.obj_id.map(|o| (o, b.version))),
            Err(BucketError::NoSuchKey(_)) => Ok(None),
            Err(e) => Err(bucket_err(e)),
        }
    }

    async fn delete(&self, bucket: &str, key: &str) -> WsResult<()> {
        let id = self.bucket(bucket)?;
        match self.store.delete(&id, &BucketCaller::system(), key, None, None).await {
            Ok(r) => {
                log::info!("bucket unbind {bucket} key={key} version={:?}", r.version);
                Ok(())
            }
            Err(BucketError::NoSuchKey(_)) => Ok(()),
            Err(e) => Err(bucket_err(e)),
        }
    }

    async fn read_token(&self, bucket: &str, key: &str, ttl_ms: i64) -> WsResult<String> {
        let id = self.bucket(bucket)?;
        let req = TokenRequest {
            prefix: None,
            key: Some(key.to_string()),
            actions: [bucket_store::Action::Read].into_iter().collect(),
            expires_at: bucket_store::now_ms() + ttl_ms,
            max_uses: None,
            bind_user_id: None,
            bind_app_id: None,
        };
        Ok(self.store.create_access_token(&id, &BucketCaller::system(), req).await.map_err(bucket_err)?.token)
    }
}

fn bridge() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("aiws-objects").enable_all().build().expect("object store runtime")
    })
}

/// Run `f` on the object store's runtime and wait for it (any thread).
pub fn block_on<T: Send + 'static>(f: impl Future<Output = T> + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    bridge().spawn(async move {
        let _ = tx.send(f.await);
    });
    rx.recv().expect("object store task failed")
}

fn ndn(e: NdnError) -> WsError {
    match e {
        NdnError::NotFound(m) => WsError::new(Code::DependencyUnavailable, format!("named store: {m} is missing")),
        e => WsError::new(Code::StorageIoError, format!("named store: {e}")),
    }
}

fn obj_id(id: &str) -> WsResult<ndn_lib::ObjId> {
    ndn_lib::ObjId::new(id).map_err(|_| WsError::invalid_schema(format!("bad object id {id}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Available,
    Missing,
}

impl Availability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Availability::Available => "available",
            Availability::Missing => "missing",
        }
    }
}

pub struct ObjectStore {
    ndm: NamedDataMgr,
    buckets: Arc<dyn Buckets>,
}

impl ObjectStore {
    pub fn new(ndm: NamedDataMgr, buckets: Arc<dyn Buckets>) -> Arc<ObjectStore> {
        Arc::new(ObjectStore { ndm, buckets })
    }

    /// A named store and a bucket index of its own under `dir` (standalone without nfs-server, tests).
    pub fn embedded(dir: &Path, decls: HashMap<String, BucketDecl>, quota_mib: Option<u64>) -> WsResult<Arc<ObjectStore>> {
        let dir = dir.to_path_buf();
        block_on(async move {
            std::fs::create_dir_all(&dir).map_err(|e| WsError::io(e.to_string()))?;
            let local = named_store::NamedLocalStore::get_named_store_by_path(dir.join("named-store")).await.map_err(ndn)?;
            let store_id = local.store_id().to_string();
            let ndm = NamedDataMgr::new();
            ndm.register_store(Arc::new(tokio::sync::Mutex::new(local))).await;
            let target = named_store::StoreTarget { store_id, device_did: String::new(), capacity: None, used: None, readonly: false, enabled: true, weight: 1 };
            ndm.add_layout(named_store::StoreLayout::new(1, vec![target], 0, 0)).await;
            let index = bucket_store::IndexDb::open(&dir.join("bucket-index.db")).map_err(bucket_err)?;
            let buckets = LocalBuckets::new(BucketStore::new(index, ndm.clone()), decls, quota_mib);
            Ok(ObjectStore::new(ndm, Arc::new(buckets)))
        })
    }

    pub fn named_store(&self) -> NamedDataMgr {
        self.ndm.clone()
    }

    pub fn has_object(&self, id: &str) -> bool {
        let Ok(id) = obj_id(id) else { return false };
        let ndm = self.ndm.clone();
        block_on(async move { ndm.is_object_exist(&id).await.unwrap_or(false) })
    }

    /// Read-time diagnosis of an asset: the FileObject and every chunk it names are in the store.
    pub fn availability(&self, file_object_id: &str) -> Availability {
        let Ok(id) = obj_id(file_object_id) else { return Availability::Missing };
        let ndm = self.ndm.clone();
        match block_on(async move { ndm.is_object_stored(&id, None).await.unwrap_or(false) }) {
            true => Availability::Available,
            false => Availability::Missing,
        }
    }

    /// Store canonical text under `id`, refusing content that does not hash to it.
    pub fn put_verified(&self, id: &str, text: &str) -> WsResult<()> {
        canonical::verify_named_object(id, text)?;
        self.put_text(id, text)
    }

    fn put_text(&self, id: &str, text: &str) -> WsResult<()> {
        let (oid, text, ndm) = (obj_id(id)?, text.to_string(), self.ndm.clone());
        block_on(async move { ndm.put_object(&oid, &text).await.map_err(ndn) })
    }

    pub fn put_chunk(&self, data: &[u8]) -> WsResult<ObjId> {
        if data.len() as u64 > MAX_CHUNK_SIZE {
            return Err(WsError::limit("files above 32 MiB need a chunk list, which this phase does not write"));
        }
        let id = chunk_id(data);
        let chunk = ChunkId::new(&id).map_err(|_| WsError::invalid_schema(format!("bad chunk id {id}")))?;
        let (data, ndm) = (data.to_vec(), self.ndm.clone());
        block_on(async move { ndm.put_chunk(&chunk, &data).await.map_err(ndn) })?;
        Ok(id)
    }

    /// Store a chunk received under a claimed id (package import).
    pub fn put_chunk_verified(&self, id: &str, data: &[u8]) -> WsResult<()> {
        if chunk_id(data) != id {
            return Err(WsError::invalid_schema(format!("chunk {id} content hash mismatch")));
        }
        self.put_chunk(data).map(|_| ())
    }

    pub fn get_chunk(&self, id: &str) -> WsResult<Vec<u8>> {
        let chunk = ChunkId::new(id).map_err(|_| WsError::invalid_schema(format!("bad chunk id {id}")))?;
        let ndm = self.ndm.clone();
        block_on(async move { ndm.get_chunk_data(&chunk).await.map_err(ndn) })
    }

    /// The content (chunk or chunk list) a FileObject points at.
    pub fn file_chunk(&self, file_object_id: &str) -> WsResult<ObjId> {
        let v = parse_strict(&self.get_object(file_object_id)?)?;
        v["content"].as_str().map(str::to_string).ok_or_else(|| WsError::invalid_schema("file object without content"))
    }

    /// Size a FileObject declares.
    pub fn file_size(&self, file_object_id: &str) -> WsResult<u64> {
        let v = parse_strict(&self.get_object(file_object_id)?)?;
        Ok(v["size"].as_u64().unwrap_or(0))
    }

    /// The first `n` bytes of a file (media type sniffing).
    pub fn file_head(&self, file_object_id: &str, n: u64) -> WsResult<Vec<u8>> {
        let (id, ndm) = (obj_id(file_object_id)?, self.ndm.clone());
        block_on(async move {
            let (reader, _) = ndm.open_reader(&id, None).await.map_err(ndn)?;
            let mut out = Vec::new();
            reader.take(n).read_to_end(&mut out).await.map_err(|e| WsError::io(e.to_string()))?;
            Ok(out)
        })
    }

    /// Copy a file's bytes to `path` (a temporary file of the caller).
    pub fn read_to_file(&self, file_object_id: &str, path: &Path) -> WsResult<u64> {
        let (id, ndm, path) = (obj_id(file_object_id)?, self.ndm.clone(), path.to_path_buf());
        block_on(async move {
            let (mut reader, _) = ndm.open_reader(&id, None).await.map_err(ndn)?;
            let mut file = tokio::fs::File::create(&path).await.map_err(|e| WsError::io(e.to_string()))?;
            let n = tokio::io::copy(&mut reader, &mut file).await.map_err(|e| WsError::io(e.to_string()))?;
            file.sync_all().await.map_err(|e| WsError::io(e.to_string()))?;
            Ok(n)
        })
    }

    /// Store a local file as a FileObject named `name` (chunk lists for large files).
    pub fn store_file(&self, path: &Path, name: &str) -> WsResult<ObjId> {
        let (path, name, ndm) = (path.to_path_buf(), name.to_string(), self.ndm.clone());
        block_on(async move {
            let mut template = ndn_lib::FileObject::default();
            template.content_obj.name = name;
            let store: &dyn ndn_toolkit::NamedObjectStore = &ndm;
            let (_, id, _) = ndn_toolkit::cacl_file_object(
                Some(store),
                &path,
                &template,
                true,
                &ndn_toolkit::CheckMode::ByFullHash,
                ndn_lib::StoreMode::StoreInNamedMgr,
                None,
            )
            .await
            .map_err(ndn)?;
            Ok(id.to_string())
        })
    }

    pub fn bind(&self, bucket: &str, key: &str, obj_id: &str) -> WsResult<u64> {
        let (b, bucket, key, obj_id) = (self.buckets.clone(), bucket.to_string(), key.to_string(), obj_id.to_string());
        block_on(async move { b.put(&bucket, &key, &obj_id).await })
    }

    pub fn binding(&self, bucket: &str, key: &str) -> WsResult<Option<(String, u64)>> {
        let (b, bucket, key) = (self.buckets.clone(), bucket.to_string(), key.to_string());
        block_on(async move { b.get(&bucket, &key).await })
    }

    pub fn unbind(&self, bucket: &str, key: &str) -> WsResult<()> {
        let (b, bucket, key) = (self.buckets.clone(), bucket.to_string(), key.to_string());
        block_on(async move { b.delete(&bucket, &key).await })
    }

    pub fn read_token(&self, bucket: &str, key: &str, ttl_ms: i64) -> WsResult<String> {
        let (b, bucket, key) = (self.buckets.clone(), bucket.to_string(), key.to_string());
        block_on(async move { b.read_token(&bucket, &key, ttl_ms).await })
    }

    /// Bind a retention set: one DirObject naming every member (`<hex>.<type>` → id, time fields 0), so the
    /// named store's cascade retains each of them — `jobj` members included, which a binding of their own
    /// would not reach beyond themselves. Returns the DirObject's id and the binding's version.
    pub fn retain(&self, bucket: &str, key: &str, members: &BTreeMap<ObjId, u64>) -> WsResult<(ObjId, u64)> {
        let list: Vec<DirMember> = members
            .iter()
            .filter_map(|(id, size)| {
                obj_id_to_filename(id).map(|name| DirMember { name, object_id: id.clone(), size: *size, is_file: id.starts_with("cyfile:") })
            })
            .collect();
        let (id, text) = canonical::dir_object(&list)?;
        self.put_verified(&id, &text)?;
        let version = self.bind(bucket, key, &id)?;
        Ok((id, version))
    }

    /// Bytes of a file a NamedObject names, as `members` of a retention set wants them sized.
    fn member_size(&self, id: &str) -> WsResult<u64> {
        let text = self.get_object(id)?;
        if id.starts_with("cyfile:") {
            return Ok(parse_strict(&text)?["size"].as_u64().unwrap_or(0));
        }
        Ok(text.len() as u64)
    }

    /// Add an object (when stored) and its size to a retention set.
    pub fn add_member(&self, id: &str, out: &mut BTreeMap<ObjId, u64>) -> WsResult<bool> {
        if out.contains_key(id) {
            return Ok(true);
        }
        if !self.has_object(id) {
            return Ok(false);
        }
        out.insert(id.to_string(), self.member_size(id)?);
        Ok(true)
    }

    /// The retention set of one entity content object: the object, its record / document files, the
    /// DirObject of a directory and the asset it references (when stored).
    pub fn entity_members(&self, object_id: &str, type_id: &str, out: &mut BTreeMap<ObjId, u64>) -> WsResult<()> {
        if !self.add_member(object_id, out)? {
            return Ok(());
        }
        let content = parse_strict(&self.get_object(object_id)?)?;
        let c = &content["content"];
        for f in [c["records"]["file"].as_str(), c["doc"]["file"].as_str(), c["dir_object"].as_str()].into_iter().flatten() {
            self.add_member(f, out)?;
        }
        if let Some(payload) = c.as_object() {
            for a in aiworkspace_core::types::asset_object_ids(type_id, payload) {
                self.add_member(a, out)?;
            }
        }
        Ok(())
    }

    /// The retention set of a content root: the root, its entities file and every entity it lists.
    pub fn content_members(&self, content_root: &str, out: &mut BTreeMap<ObjId, u64>) -> WsResult<()> {
        if !self.add_member(content_root, out)? {
            return Ok(());
        }
        let root = parse_strict(&self.get_object(content_root)?)?;
        if let Some(f) = root["entities"]["file"].as_str() {
            self.add_member(f, out)?;
            let text = String::from_utf8(self.get_file(f)?).map_err(|_| WsError::invalid_schema("entities file is not UTF-8"))?;
            for line in text.lines().filter(|l| !l.is_empty()) {
                let e = parse_strict(line)?;
                if let Some(obj) = e["object_id"].as_str() {
                    self.entity_members(obj, e["type_id"].as_str().unwrap_or(""), out)?;
                }
            }
        }
        Ok(())
    }
}

impl ObjectSource for ObjectStore {
    fn get_object(&self, id: &str) -> WsResult<String> {
        let (oid, ndm) = (obj_id(id)?, self.ndm.clone());
        block_on(async move { ndm.get_object(&oid).await.map_err(ndn) })
    }
    fn get_file(&self, file_object_id: &str) -> WsResult<Vec<u8>> {
        let (id, ndm) = (obj_id(file_object_id)?, self.ndm.clone());
        block_on(async move {
            let (mut reader, _) = ndm.open_reader(&id, None).await.map_err(ndn)?;
            let mut out = Vec::new();
            reader.read_to_end(&mut out).await.map_err(|e| WsError::new(Code::DependencyUnavailable, format!("file {}: {e}", id.to_string())))?;
            Ok(out)
        })
    }
}

/// A sink writing into the store. What it writes is retained by the binding its caller makes in the same
/// function (a snapshot's or a version's retention set, a staged asset).
pub struct StoreSink<'a> {
    pub store: &'a ObjectStore,
}

impl<'a> StoreSink<'a> {
    pub fn new(store: &'a ObjectStore) -> Self {
        StoreSink { store }
    }
}

impl ObjectSink for StoreSink<'_> {
    fn put_object(&mut self, obj_type: &str, canonical_text: &str) -> WsResult<ObjId> {
        let id = canonical::build_obj_id(obj_type, canonical_text);
        self.store.put_text(&id, canonical_text)?;
        Ok(id)
    }
    fn put_file(&mut self, bytes: &[u8]) -> WsResult<ObjId> {
        let chunk = self.store.put_chunk(bytes)?;
        let (id, text) = file_object(bytes.len() as u64, &chunk)?;
        self.put_object(canonical::OBJ_TYPE_FILE, &text)?;
        Ok(id)
    }
}

/// Media type from magic bytes — never from what the client claims.
pub fn sniff_media_type(data: &[u8]) -> &'static str {
    sniff(data, std::str::from_utf8(data).is_ok())
}

/// [`sniff_media_type`] of the first bytes of a longer file: a character cut at the end is still text.
pub fn sniff_prefix(head: &[u8]) -> &'static str {
    sniff(head, std::str::from_utf8(head).map(|_| true).unwrap_or_else(|e| e.error_len().is_none()))
}

fn sniff(data: &[u8], utf8: bool) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'B', b'M', ..] if data.len() >= 18 && matches!(u32::from_le_bytes([data[14], data[15], data[16], data[17]]), 12 | 40 | 52 | 56 | 64 | 108 | 124) => "image/bmp",
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => "image/tiff",
        [b'%', b'P', b'D', b'F', ..] => "application/pdf",
        [b'P', b'K', 3, 4, ..] => "application/zip",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => iso_media(data),
        [0x1a, 0x45, 0xdf, 0xa3, ..] => {
            if data.windows(4).take(64).any(|w| w == b"webm") {
                "video/webm"
            } else {
                "video/x-matroska"
            }
        }
        [b'O', b'g', b'g', b'S', ..] => {
            if data.windows(6).any(|w| w == b"theora") {
                "video/ogg"
            } else {
                "audio/ogg"
            }
        }
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'A', b'V', b'E', ..] => "audio/wav",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'A', b'V', b'I', b' ', ..] => "video/x-msvideo",
        [b'f', b'L', b'a', b'C', ..] => "audio/flac",
        [b'I', b'D', b'3', ..] => "audio/mpeg",
        _ if mpeg_audio_frame(data) => "audio/mpeg",
        [0xff, b, ..] if b & 0xf6 == 0xf0 => "audio/aac",
        _ if utf8 => {
            if data.iter().take(256).any(|b| *b == b'<') && String::from_utf8_lossy(&data[..data.len().min(256)]).contains("<svg") {
                "image/svg+xml"
            } else {
                "text/plain"
            }
        }
        _ => "application/octet-stream",
    }
}

/// The header of an MPEG-1/2 audio frame of layer II or III (MP3): a UTF-16 byte order mark is not one.
fn mpeg_audio_frame(h: &[u8]) -> bool {
    h.len() >= 3 && h[0] == 0xff && h[1] & 0xe0 == 0xe0 && (h[1] >> 3) & 3 != 1 && matches!((h[1] >> 1) & 3, 1 | 2) && h[2] >> 4 != 0xf && (h[2] >> 2) & 3 != 3
}

/// An ISO base media file (`ftyp` box) by its major and compatible brands: still images first (HEIF and AVIF are
/// ISO files too), then audio-only MPEG-4 and QuickTime, the rest is MPEG-4 video.
fn iso_media(data: &[u8]) -> &'static str {
    let end = (u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize).min(data.len());
    // the major brand, then (after the minor version) the compatible brands
    let brands: Vec<&[u8]> = data.get(8..end).unwrap_or_default().chunks_exact(4).enumerate().filter(|(i, _)| *i != 1).map(|(_, b)| b).collect();
    let has = |names: &[&[u8]]| brands.iter().any(|b| names.contains(b));
    match brands.first().copied().unwrap_or_default() {
        b"avif" | b"avis" => "image/avif",
        b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" if !has(&[b"avif".as_slice()]) => "image/heic",
        b"mif1" | b"msf1" => "image/avif",
        b"M4A " | b"M4B " | b"F4A " => "audio/mp4",
        b"qt  " => "video/quicktime",
        b"3gp4" | b"3gp5" | b"3gp6" | b"3g2a" => "video/3gpp",
        _ => "video/mp4",
    }
}

#[cfg(test)]
mod tests {
    use super::sniff_media_type as sniff;

    #[test]
    fn media_types_come_from_magic_bytes() {
        let ftyp = |major: &[u8], compatible: &[u8]| [&[0, 0, 0, (16 + compatible.len()) as u8][..], b"ftyp", major, &[0, 0, 0, 0], compatible, b"rest"].concat();
        assert_eq!(sniff(&ftyp(b"isom", b"isomavc1mp41")), "video/mp4");
        assert_eq!(sniff(&ftyp(b"qt  ", b"qt  ")), "video/quicktime");
        assert_eq!(sniff(&ftyp(b"M4A ", b"M4A mp42isom")), "audio/mp4");
        assert_eq!(sniff(&ftyp(b"heic", b"mif1heic")), "image/heic");
        assert_eq!(sniff(&ftyp(b"mif1", b"mif1avif")), "image/avif");
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x84webm\x42\x87"), "video/webm");
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x9f\x42\x82\x88matroska"), "video/x-matroska");
        assert_eq!(sniff(b"OggS\0\x02\0\0\0\0\0\0\0\0\x01\x02\x03\x04\0\0\0\0\0\0\0\0\x01\x13OpusHead"), "audio/ogg");
        assert_eq!(sniff(b"RIFF\x24\0\0\0WAVEfmt "), "audio/wav");
        assert_eq!(sniff(b"RIFF\x24\0\0\0AVI LIST"), "video/x-msvideo");
        assert_eq!(sniff(b"fLaC\0\0\0\x22"), "audio/flac");
        assert_eq!(sniff(b"ID3\x04\0\0\0\0\0\0"), "audio/mpeg");
        assert_eq!(sniff(&[0xff, 0xfb, 0x90, 0x64, 0, 0]), "audio/mpeg");
        assert_eq!(sniff(&[0xff, 0xf1, 0x50, 0x80, 0, 0]), "audio/aac");
        assert_eq!(sniff(&[b'B', b'M', 0, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0, 1, 0]), "image/bmp");
        assert_eq!(sniff(b"II*\0\x08\0\0\0"), "image/tiff");
        // look-alikes stay what they are: UTF-16 text starts with FF FE, a note may start with "BM"
        assert_eq!(sniff(&[0xff, 0xfe, b'h', 0, b'i', 0]), "application/octet-stream");
        assert_eq!(sniff(b"BMW sales figures for 2026"), "text/plain");
    }
}
