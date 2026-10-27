//! Local named-object storage and the receiver-side checks of §5.4 / §16.6: JWT signature
//! against the publisher's authorized keys, ObjId from the claims, entry namespace ownership.

use crate::db::{Conn, Db};
use crate::directory::{DirError, Directory};
use crate::error::{bad, HsError, HsResult};
use crate::protocol::*;
use crate::sign::{decode_unverified, kid_did, verify_signature};
use buckyos_api::bucket_client::{BucketClient, BucketId, Origin, PutRequest};
use named_store::NamedDataMgr;
use ndn_lib::{ChunkHasher, ChunkId, NdnError, ObjId};
use ndn_toolkit::{NamedObjectStore, NdmClient, NdmClientConfig};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct StoredObject {
    pub obj_id: String,
    pub obj_type: String,
    pub body: Value,
    pub jwt: Option<String>,
    pub signer: Option<String>,
    pub publisher: Option<String>,
    pub verified: bool,
    pub local_only: bool,
}

impl StoredObject {
    /// The form handed to other nodes: the signed JWT when there is one.
    pub fn wire(&self) -> (String, &'static str) {
        match &self.jwt {
            Some(jwt) => (jwt.clone(), "application/cyfs-named-object+jwt"),
            None => (canonical(&self.body), "application/cyfs-named-object+json"),
        }
    }
}

pub fn canonical(value: &Value) -> String {
    ndn_lib::build_named_object_by_json("x", value).1
}

pub fn put_object(conn: &Conn, obj: &StoredObject, origin: Option<&str>, now: i64) -> HsResult<()> {
    conn.execute(
        "INSERT INTO objects(owner, obj_id, obj_type, body, jwt, signer, publisher, verified, local_only, origin, received_at)
         VALUES (?11, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(owner, obj_id) DO UPDATE SET
           jwt = COALESCE(objects.jwt, excluded.jwt),
           signer = COALESCE(objects.signer, excluded.signer),
           verified = MAX(objects.verified, excluded.verified),
           local_only = MIN(objects.local_only, excluded.local_only)",
        params![
            obj.obj_id,
            obj.obj_type,
            canonical(&obj.body),
            obj.jwt,
            obj.signer,
            obj.publisher,
            obj.verified as i64,
            obj.local_only as i64,
            origin,
            now,
            conn.owner
        ],
    )?;
    Ok(())
}

pub fn get_object(conn: &Conn, obj_id: &str) -> HsResult<Option<StoredObject>> {
    let obj_id = normalize_obj_id(obj_id);
    let row = conn
        .query_row(
            "SELECT obj_id, obj_type, body, jwt, signer, publisher, verified, local_only FROM objects WHERE obj_id=?1 AND owner=?2",
            params![obj_id, conn.owner],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()?;
    Ok(match row {
        Some((obj_id, obj_type, body, jwt, signer, publisher, verified, local_only)) => Some(StoredObject {
            obj_id,
            obj_type,
            body: serde_json::from_str(&body)?,
            jwt,
            signer,
            publisher,
            verified: verified != 0,
            local_only: local_only != 0,
        }),
        None => None,
    })
}

pub fn has_object(conn: &Conn, obj_id: &str) -> HsResult<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM objects WHERE obj_id=?1 AND owner=?2", params![normalize_obj_id(obj_id), conn.owner], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn get_feed(conn: &Conn, obj_id: &str) -> HsResult<Option<FeedObject>> {
    match get_object(conn, obj_id)? {
        Some(obj) if obj.obj_type == OBJ_TYPE_FEED => Ok(serde_json::from_value(obj.body).ok()),
        _ => Ok(None),
    }
}

/// Searchable facts of a Feed Object; `entry_valid` is the namespace check result (A46).
pub fn index_feed(conn: &Conn, obj_id: &str, obj: &FeedObject, entry_valid: bool) -> HsResult<()> {
    conn.execute(
        "INSERT INTO feed_index(owner, obj_id, publisher, kind, comment_type, content_type, entry, entry_valid, target, wraps, iat, category, text, tags)
         VALUES (?14, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(owner, obj_id) DO UPDATE SET entry_valid = MAX(feed_index.entry_valid, excluded.entry_valid)",
        params![
            obj_id,
            obj.publisher,
            match obj.kind {
                FeedKind::Post => "post",
                FeedKind::Comment => "comment",
            },
            obj.comment_type.map(|c| c.as_str()),
            obj.content.as_ref().map(|c| c.content_type.as_str()),
            obj.entry,
            entry_valid as i64,
            obj.comment_target(),
            obj.wraps.as_deref().map(normalize_obj_id),
            obj.iat as i64,
            obj.publication_category,
            obj.search_text(),
            serde_json::to_string(&obj.tags)?,
            conn.owner,
        ],
    )?;
    Ok(())
}

pub fn entry_valid(conn: &Conn, obj_id: &str) -> HsResult<bool> {
    Ok(conn
        .query_row("SELECT entry_valid FROM feed_index WHERE obj_id=?1 AND owner=?2", [obj_id, conn.owner], |r| r.get::<_, i64>(0))
        .optional()?
        .is_some_and(|v| v != 0))
}

/// Object type of a JWT/JSON body from its `kind` claim.
pub fn obj_type_for_claims(claims: &Value) -> Option<&'static str> {
    match claims.get("kind").and_then(Value::as_str)? {
        "post" | "comment" => Some(OBJ_TYPE_FEED),
        HEAD_KIND => Some(OBJ_TYPE_HEAD),
        FOLLOW_KIND => Some(OBJ_TYPE_FOLLOW),
        CONSUMPTION_KIND => Some(OBJ_TYPE_CONSUMPTION),
        "evaluation" => Some(OBJ_TYPE_EVALUATION),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct Verified {
    pub obj_id: String,
    pub obj_type: String,
    pub claims: Value,
    pub jwt: String,
    pub signer: String,
    pub publisher: String,
}

#[derive(Debug)]
pub enum VerifyError {
    Invalid(String),
    Unauthorized(String),
    KeyUnavailable(String),
}

impl From<VerifyError> for HsError {
    fn from(e: VerifyError) -> Self {
        match e {
            VerifyError::Invalid(m) => HsError::BadRequest(m),
            VerifyError::Unauthorized(m) => HsError::Forbidden(m),
            VerifyError::KeyUnavailable(m) => HsError::Unavailable(m),
        }
    }
}

fn dir_err(e: DirError) -> VerifyError {
    match e {
        DirError::NotFound(m) => VerifyError::Unauthorized(m),
        DirError::Unavailable(m) => VerifyError::KeyUnavailable(m),
    }
}

/// Signature of a named-object JWT by a key the claimed `publisher` authorizes.
pub async fn verify_jwt(directory: &dyn Directory, jwt: &str, expected_type: Option<&str>) -> Result<Verified, VerifyError> {
    let decoded = decode_unverified(jwt).map_err(VerifyError::Invalid)?;
    let obj_type = obj_type_for_claims(&decoded.claims).ok_or_else(|| VerifyError::Invalid("unknown object kind".into()))?;
    if let Some(expected) = expected_type {
        if expected != obj_type {
            return Err(VerifyError::Invalid(format!("expected a {expected} object")));
        }
    }
    let publisher = decoded
        .claims
        .get("publisher")
        .and_then(Value::as_str)
        .ok_or_else(|| VerifyError::Invalid("object has no publisher".into()))?
        .to_string();
    let signer = kid_did(&decoded.kid).to_string();
    if !directory.authorizes(&publisher, &signer, &decoded.kid).await.map_err(dir_err)? {
        return Err(VerifyError::Unauthorized(format!("{signer} may not sign for {publisher}")));
    }
    let key = directory.signer_key(&signer, &decoded.kid).await.map_err(dir_err)?;
    let claims = verify_signature(jwt, &key).map_err(VerifyError::Unauthorized)?;
    let (obj_id, _) = obj_id_of(obj_type, &claims).map_err(VerifyError::Invalid)?;
    Ok(Verified { obj_id, obj_type: obj_type.to_string(), claims, jwt: jwt.to_string(), signer, publisher })
}

/// §5.4 rule 2: the entry lies in the publisher's namespace (its home path or an owned
/// content DID).
pub async fn entry_belongs_to(directory: &dyn Directory, publisher: &str, entry: &str) -> Result<bool, DirError> {
    match EntryRef::parse(entry) {
        Ok(EntryRef::Path { home, .. }) => match directory.home_of(publisher).await {
            Ok(own) => Ok(own == home),
            Err(DirError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        },
        Ok(EntryRef::Did(did)) => Ok(directory.content_did_owner(&did).await?.as_deref() == Some(publisher)),
        Err(_) => Ok(false),
    }
}

pub fn verify_chunk(chunk_id: &str, data: &[u8]) -> bool {
    let Ok(id) = ChunkId::new(chunk_id) else { return false };
    let Ok(method) = id.chunk_type.to_hash_method() else { return false };
    let Ok(hasher) = ChunkHasher::new_with_hash_method(method) else { return false };
    let computed = if id.chunk_type.is_mix() {
        match hasher.calc_mix_chunk_id_from_bytes(data) {
            Ok(c) => c,
            Err(_) => return false,
        }
    } else {
        hasher.calc_chunk_id_from_bytes(data)
    };
    computed == id
}

pub fn chunk_id_of(data: &[u8]) -> HsResult<String> {
    let hasher = ChunkHasher::new(None).map_err(|e| HsError::Internal(e.to_string()))?;
    Ok(hasher.calc_mix_chunk_id_from_bytes(data).map_err(|e| HsError::Internal(e.to_string()))?.to_string())
}

pub fn is_chunk_id(value: &str) -> bool {
    ChunkId::new(value).is_ok()
}

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

enum Named {
    /// The zone named store (kernel service).
    Local(NamedDataMgr),
    /// A named store behind an nfs-server's NDM proxy (standalone).
    Remote(NdmClient),
}

macro_rules! named_call {
    ($named:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match $named {
            Named::Local(store) => store.$method($($arg),*).await,
            Named::Remote(store) => store.$method($($arg),*).await,
        }
    };
}

enum Buckets {
    /// `bucket.*` of the zone's nfs-server, with a fresh service session per call.
    Runtime,
    Fixed(Arc<BucketClient>),
}

/// File contents and objects of the zone, and the users' HomeStation buckets that retain them
/// (doc/homestation/HomeStation Durable Data Schema.md). Writes to the named store happen
/// only together with a bucket binding.
pub struct Storage {
    named: Named,
    buckets: Buckets,
}

fn named_err(e: NdnError) -> HsError {
    HsError::Unavailable(format!("named store: {e}"))
}

pub fn bucket_err(e: kRPC::RPCErrors) -> HsError {
    HsError::Unavailable(format!("bucket: {e}"))
}

impl Storage {
    pub fn zone(ndm: NamedDataMgr) -> Self {
        Self { named: Named::Local(ndm), buckets: Buckets::Runtime }
    }

    /// A standalone nfs-server (`--bucket-specs` declaring `homestation`): its NDM proxy and `bucket.*`.
    pub fn nfs_server(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        Self {
            named: Named::Remote(NdmClient::new(NdmClientConfig { base_url: base.to_string() })),
            buckets: Buckets::Fixed(Arc::new(BucketClient::new(kRPC::kRPC::new(&format!("{base}/kapi/nfs-server"), None)))),
        }
    }

    pub fn named(&self) -> &dyn NamedObjectStore {
        match &self.named {
            Named::Local(store) => store,
            Named::Remote(store) => store,
        }
    }

    pub async fn buckets(&self) -> HsResult<Arc<BucketClient>> {
        match &self.buckets {
            Buckets::Fixed(client) => Ok(client.clone()),
            Buckets::Runtime => {
                let runtime = buckyos_api::get_buckyos_api_runtime().map_err(bucket_err)?;
                Ok(Arc::new(runtime.get_bucket_client().await.map_err(bucket_err)?))
            }
        }
    }

    /// Binds a stored data object (FileObject / DirObject) under `key`; `origin` for ambient buckets.
    pub async fn bind(&self, bucket: &BucketId, key: &str, obj_id: &str, content_type: Option<String>, origin: Option<Origin>) -> HsResult<()> {
        let req = PutRequest { key: key.to_string(), obj_id: obj_id.to_string(), content_type, origin, ..Default::default() };
        self.buckets().await?.put(bucket, &req).await.map_err(bucket_err)?;
        Ok(())
    }

    /// Stores a JSON object (an `object` bucket's value) and binds it under `key`.
    pub async fn put_bound_object(&self, bucket: &BucketId, key: &str, obj_id: &str, body: &str, origin: Option<Origin>) -> HsResult<()> {
        let id = ObjId::new(obj_id).map_err(|e| bad(format!("invalid id: {e}")))?;
        self.named().put_object(&id, body).await.map_err(named_err)?;
        let req = PutRequest { key: key.to_string(), obj_id: obj_id.to_string(), origin, ..Default::default() };
        self.buckets().await?.put(bucket, &req).await.map_err(bucket_err)?;
        Ok(())
    }

    /// Stores a single-chunk FileObject (`body`, hashing to `obj_id`) with its bytes and binds it under `key`.
    #[allow(clippy::too_many_arguments)]
    pub async fn put_bound_file(
        &self,
        bucket: &BucketId,
        key: &str,
        chunk_id: &str,
        data: &[u8],
        obj_id: &str,
        body: &str,
        content_type: Option<String>,
        origin: Option<Origin>,
    ) -> HsResult<()> {
        let chunk = ChunkId::new(chunk_id).map_err(|e| bad(format!("invalid chunk id: {e}")))?;
        self.named().put_chunk(&chunk, data).await.map_err(named_err)?;
        let id = ObjId::new(obj_id).map_err(|e| bad(format!("invalid id: {e}")))?;
        self.named().put_object(&id, body).await.map_err(named_err)?;
        self.bind(bucket, key, obj_id, content_type, origin).await
    }

    /// Puts a fetched chunk or descriptor object; the caller binds the root right after.
    pub(crate) async fn put_fetched_chunk(&self, chunk_id: &str, data: &[u8]) -> HsResult<()> {
        if !verify_chunk(chunk_id, data) {
            return Err(bad("chunk data does not match its id"));
        }
        let id = ChunkId::new(chunk_id).map_err(|e| bad(format!("invalid chunk id: {e}")))?;
        self.named().put_chunk(&id, data).await.map_err(named_err)
    }

    pub(crate) async fn put_fetched_object(&self, obj_id: &str, body: &str) -> HsResult<()> {
        let id = ObjId::new(obj_id).map_err(|e| bad(format!("invalid id: {e}")))?;
        self.named().put_object(&id, body).await.map_err(named_err)
    }

    pub async fn get_chunk(&self, chunk_id: &str) -> HsResult<Option<Vec<u8>>> {
        let id = ChunkId::new(chunk_id).map_err(|e| bad(format!("invalid chunk id: {e}")))?;
        if !self.named().have_chunk(&id).await {
            return Ok(None);
        }
        match named_call!(&self.named, get_chunk_data(&id)) {
            Ok(data) => Ok(Some(data)),
            Err(NdnError::NotFound(_)) => Ok(None),
            Err(e) => Err(named_err(e)),
        }
    }

    pub async fn has_chunk(&self, chunk_id: &str) -> bool {
        match ChunkId::new(chunk_id) {
            Ok(id) => self.named().have_chunk(&id).await,
            Err(_) => false,
        }
    }

    /// Named objects stored by other zone apps (FileObjects uploaded through NDM).
    pub async fn get_named_object(&self, obj_id: &str) -> HsResult<Option<String>> {
        let id = ObjId::new(obj_id).map_err(|e| bad(format!("invalid id: {e}")))?;
        match self.named().get_object(&id).await {
            Ok(body) => Ok(Some(body)),
            Err(NdnError::NotFound(_)) => Ok(None),
            Err(e) => {
                log::debug!("named store get {obj_id}: {e}");
                Ok(None)
            }
        }
    }

    /// The object and everything it references are in the store.
    pub async fn is_complete(&self, obj_id: &str) -> HsResult<bool> {
        let id = ObjId::new(obj_id).map_err(|e| bad(e.to_string()))?;
        named_call!(&self.named, is_object_stored(&id, None)).map_err(named_err)
    }

    pub async fn read_file(&self, obj_id: &str) -> HsResult<Option<Vec<u8>>> {
        use tokio::io::AsyncReadExt;
        let id = ObjId::new(obj_id).map_err(|e| bad(e.to_string()))?;
        let (mut reader, size) = match named_call!(&self.named, open_reader(&id, None)) {
            Ok(value) => value,
            Err(NdnError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(named_err(e)),
        };
        if size > MAX_FILE_BYTES {
            return Err(bad("file exceeds 64 MiB download limit"));
        }
        let mut bytes = Vec::with_capacity(size as usize);
        reader.read_to_end(&mut bytes).await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        Ok(Some(bytes))
    }
}

/// Read an object from the local store, or, for FileObjects, from the zone named store.
pub async fn load_local(db: &Db, store: &Storage, obj_id: &str) -> HsResult<Option<StoredObject>> {
    let id = normalize_obj_id(obj_id);
    let id2 = id.clone();
    if let Some(obj) = db.call(move |c| get_object(c, &id2)).await? {
        return Ok(Some(obj));
    }
    let kind = obj_type_of(&id).unwrap_or_default();
    if kind != OBJ_TYPE_FILE && kind != OBJ_TYPE_DIR && kind != ndn_lib::OBJ_TYPE_CHUNK_LIST {
        return Ok(None);
    }
    let Some(body) = store.get_named_object(&id).await? else { return Ok(None) };
    let value: Value = serde_json::from_str(&body).map_err(|_| bad("named store returned invalid JSON"))?;
    let computed = object_id(&kind, &body)?;
    if computed != id {
        return Err(HsError::Internal(format!("named store object {id} does not match its id")));
    }
    let obj = StoredObject {
        obj_id: id.clone(),
        obj_type: kind,
        body: value,
        jwt: None,
        signer: None,
        publisher: None,
        verified: true,
        local_only: false,
    };
    let stored = obj.clone();
    let now = crate::now_ms();
    db.call(move |c| put_object(c, &stored, Some("named_store"), now)).await?;
    Ok(Some(obj))
}

pub fn object_id(kind: &str, body: &str) -> HsResult<String> {
    if kind == OBJ_TYPE_DIR {
        let dir: ndn_lib::DirObject = ndn_lib::load_named_obj(body).map_err(|e| bad(e.to_string()))?;
        return dir.gen_obj_id().map(|(id, _)| id.to_string()).map_err(|e| bad(e.to_string()));
    }
    if kind == ndn_lib::OBJ_TYPE_CHUNK_LIST {
        return ndn_lib::ChunkList::from_json(body).map(|list| list.gen_obj_id().0.to_string()).map_err(|e| bad(e.to_string()));
    }
    obj_id_of(kind, &serde_json::from_str(body)?).map(|(id, _)| id).map_err(bad)
}

pub fn object_dependencies(id: &str, body: &str) -> HsResult<Vec<String>> {
    let id = ndn_lib::ObjId::new(id).map_err(|e| bad(e.to_string()))?;
    if id.is_file_object() {
        let file: ndn_lib::FileObject = ndn_lib::load_named_obj(body).map_err(|e| bad(e.to_string()))?;
        return Ok(vec![file.content]);
    }
    if id.is_dir_object() {
        let dir: ndn_lib::DirObject = ndn_lib::load_named_obj(body).map_err(|e| bad(e.to_string()))?;
        return dir.object_map.body.values().map(|item| item.get_obj_id().map(|(id, _)| id.to_string()).map_err(|e| bad(e.to_string()))).collect();
    }
    if id.obj_type == ndn_lib::OBJ_TYPE_CHUNK_LIST {
        return Ok(ndn_lib::ChunkList::from_json(body).map_err(|e| bad(e.to_string()))?.body.into_iter().map(|id| id.to_string()).collect());
    }
    Err(bad("unsupported attachment object type"))
}

pub fn import_metadata(conn: &Conn, id: &str) -> HsResult<Value> {
    let value: Option<String> =
        conn.query_row("SELECT metadata FROM import_metadata WHERE obj_id=?1 AND owner=?2", [id, conn.owner], |row| row.get(0)).optional()?;
    Ok(value.and_then(|value| serde_json::from_str(&value).ok()).unwrap_or(Value::Null))
}
