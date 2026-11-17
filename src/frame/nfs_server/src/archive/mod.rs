//! Archive browsing (notepads/archive-preview-files-todo.md, and
//! archive-preview-more-formats-todo.md for 7z / tar / compressed files): the
//! members of an archive as a read-only tree, addressed as
//! `<file>/@/archive/<member>` (path sources: DFS files and projected bucket
//! files) or as `ObjectRef { obj_id, inner_path: "/archive/<member>" }`
//! (object sources such as chat attachments). Nothing is extracted by
//! default: listings come from the archive's directory, members are read by
//! range (stored), decoded as a stream, or materialized into a bounded cache
//! (D9); compressed tars and solid 7z blocks are decoded once into the cache
//! (E3, E8).

pub mod cache;
pub mod format;
pub mod names;
pub mod password;
pub mod sevenz;
pub mod source;
pub mod tar;
pub mod zip;

use crate::acl::{Actor, Level, HOME_ROOT, HOST_ROOT};
use crate::bucket::projection::ProjAt;
use crate::error::*;
use crate::fsutil::{join_root, lstat, normalize_rel, validate_name, NodeMeta};
use crate::namespace::{canonical_path, current_zone_cyfs_path, EntryRow, Node};
use crate::state::{AppState, SharedState};
use crate::types::{Capabilities, Envelope, WantMask, WireRef};
use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use cache::Index;
use names::{Tree, TreeNode, ROOT};
use password::Origin;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use format::{Compression, Kind, Member};
use source::{unsupported, Stamp};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub const SEGMENT: &str = "@";
pub const BRANCH: &str = "archive";
pub const INNER_ROOT: &str = "/archive";

/// Limits and sizes (archive TODO T2: initial values, tuned after T8).
#[derive(Debug, Clone)]
pub struct ArchiveConfig {
    pub max_central_directory_bytes: u64,
    pub max_entries: u64,
    pub max_indexes: usize,
    pub disk_budget_bytes: u64,
    pub max_materialize_bytes: u64,
    pub workers: usize,
    pub idle_close_secs: u64,
    /// Dictionary / window memory one 7z or xz decoder may use (E11).
    pub max_decoder_memory: u64,
}

impl Default for ArchiveConfig {
    fn default() -> Self {
        let budget = 4 * 1024 * 1024 * 1024;
        ArchiveConfig {
            max_central_directory_bytes: 64 * 1024 * 1024,
            max_entries: 200_000,
            max_indexes: 64,
            disk_budget_bytes: budget,
            max_materialize_bytes: budget,
            workers: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2),
            idle_close_secs: 60,
            max_decoder_memory: 256 * 1024 * 1024,
        }
    }
}

/// The archive file a member lives in.
#[derive(Debug, Clone)]
pub enum Outer {
    Native { root: String, rel: String, meta: NodeMeta },
    Projection { rel: String, obj_id: String, etag: String },
    Object { obj_id: String },
}

#[derive(Debug, Clone)]
pub struct ArchiveNode {
    pub outer: Outer,
    pub index: Arc<Index>,
    pub tree: Arc<Tree>,
    pub node: usize,
}

impl ArchiveNode {
    pub fn tnode(&self) -> &TreeNode {
        &self.tree.nodes[self.node]
    }

    pub fn is_root(&self) -> bool {
        self.node == ROOT
    }

    pub fn is_dir(&self) -> bool {
        self.tnode().dir
    }

    pub fn kind(&self) -> &'static str {
        if self.is_dir() {
            "dir"
        } else {
            "file"
        }
    }

    pub fn member(&self) -> Option<&Member> {
        self.tnode().entry.map(|e| &self.index.members[e])
    }

    pub fn name(&self) -> String {
        if self.is_root() {
            self.index.name.clone()
        } else {
            self.tnode().name.clone()
        }
    }

    pub fn inner_path(&self) -> String {
        let path = &self.tnode().path;
        if path.is_empty() {
            INNER_ROOT.to_string()
        } else {
            format!("{INNER_ROOT}/{path}")
        }
    }

    pub fn outer_canonical(&self) -> Option<String> {
        match &self.outer {
            Outer::Native { root, rel, .. } => Some(canonical_path(root, rel)),
            Outer::Projection { rel, .. } => Some(canonical_path(HOME_ROOT, rel)),
            Outer::Object { .. } => None,
        }
    }

    pub fn canonical(&self) -> Option<String> {
        let path = &self.tnode().path;
        self.outer_canonical().map(|o| if path.is_empty() { format!("{o}/{SEGMENT}/{BRANCH}") } else { format!("{o}/{SEGMENT}/{BRANCH}/{path}") })
    }

    pub fn revision(&self) -> String {
        format!("a-{}", &hex::encode(Sha256::digest(self.index.key.as_bytes()))[..16])
    }

    pub fn container_key(&self) -> String {
        format!("{}\u{0}{}", self.index.key, hex::encode(&self.tnode().raw))
    }

    pub fn child(&self, node: usize) -> ArchiveNode {
        ArchiveNode { outer: self.outer.clone(), index: self.index.clone(), tree: self.tree.clone(), node }
    }

    pub fn object_id(&self) -> Option<&str> {
        match &self.outer {
            Outer::Object { obj_id } => Some(obj_id),
            _ => None,
        }
    }

    /// Per-member etag: strong for objects (they never change).
    fn etag(&self, m: &Member) -> (String, String) {
        match &self.outer {
            Outer::Object { obj_id } => {
                let v = format!("{obj_id}-{}", m.tag);
                (v.clone(), format!("\"{v}\""))
            }
            Outer::Native { meta, .. } => {
                let v = format!("{}-{}-{}", meta.size, meta.mtime, m.tag);
                (v.clone(), format!("W/\"{v}\""))
            }
            Outer::Projection { etag, .. } => {
                let v = format!("{etag}-{}", m.tag);
                (v.clone(), format!("W/\"{v}\""))
            }
        }
    }

    fn mtime(&self) -> i64 {
        match self.member() {
            Some(m) if m.mtime != 0 => m.mtime,
            _ => match &self.outer {
                Outer::Native { meta, .. } => meta.mtime,
                _ => 0,
            },
        }
    }
}

pub fn archive_dir_capabilities() -> Capabilities {
    Capabilities { list: true, read: false, accepts_content: false, accepts_references: false, remove_semantics: "none", ordered: false }
}

pub fn archive_file_capabilities() -> Capabilities {
    Capabilities { list: false, read: true, accepts_content: false, accepts_references: false, remove_semantics: "none", ordered: false }
}

pub fn read_only() -> NfsError {
    NfsError::new(ErrorCode::PermissionDenied, "files in an archive are read-only").with("reason", json!("archive_read_only"))
}

fn password_required() -> NfsError {
    NfsError::new(ErrorCode::PasswordRequired, "this archive is encrypted; unlock it first")
}

// ---------- request context ----------

/// What a request brings to the synchronous namespace: the encoding
/// preference of its `Accept-Language`, the objects its principal may read,
/// who it is (for users' views of encrypted-header archives), and failures
/// of the asynchronous preparation.
#[derive(Debug, Default)]
pub struct RequestCtx {
    pub pref: Option<&'static encoding_rs::Encoding>,
    granted: Mutex<HashSet<String>>,
    failed: Mutex<HashMap<String, NfsError>>,
    user: Mutex<Option<String>>,
}

impl RequestCtx {
    pub fn from_headers(headers: &HeaderMap) -> Arc<RequestCtx> {
        let pref = headers.get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()).and_then(names::preference);
        Arc::new(RequestCtx { pref, ..Default::default() })
    }

    pub fn grant(&self, obj_id: &str) {
        self.granted.lock().unwrap().insert(obj_id.to_string());
    }
}

tokio::task_local! {
    static CTX: Arc<RequestCtx>;
}

pub async fn scope<F: std::future::Future>(ctx: Arc<RequestCtx>, f: F) -> F::Output {
    CTX.scope(ctx, f).await
}

pub fn current() -> Option<Arc<RequestCtx>> {
    CTX.try_with(|c| c.clone()).ok()
}

fn pref() -> Option<&'static encoding_rs::Encoding> {
    current().and_then(|c| c.pref)
}

pub fn granted(obj_id: &str) -> bool {
    current().is_some_and(|c| c.granted.lock().unwrap().contains(obj_id))
}

fn failure(key: &str) -> Option<NfsError> {
    current().and_then(|c| c.failed.lock().unwrap().get(key).cloned())
}

fn ctx_user() -> Option<String> {
    current().and_then(|c| c.user.lock().unwrap().clone())
}

// ---------- handles ----------

#[derive(Debug, Serialize, Deserialize)]
struct NaHandle {
    r: String,
    p: String,
    #[serde(default)]
    i: u64,
    #[serde(default)]
    d: u64,
    #[serde(default)]
    b: Option<std::time::SystemTime>,
    #[serde(default)]
    s: u64,
    #[serde(default)]
    m: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    t: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    o: Option<String>,
    e: String,
    k: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct NoHandle {
    o: String,
    e: String,
    k: String,
}

fn native_key(root: &str, rel: &str, meta: &NodeMeta) -> String {
    let birth = meta.birth.and_then(|b| b.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos().to_string()).unwrap_or_else(|| "-".into());
    format!("n:{root}/{rel}:{}:{}:{birth}:{}:{}", meta.id.ino, meta.id.dev, meta.size, meta.mtime)
}

fn native_prefix(root: &str, rel: &str) -> String {
    format!("n:{root}/{rel}:")
}

fn object_key(obj_id: &str) -> String {
    format!("o:{obj_id}")
}

fn stamp_of(meta: &NodeMeta) -> Stamp {
    Stamp { ino: meta.id.ino, dev: meta.id.dev, size: meta.size, mtime: meta.mtime }
}

fn user_key(actor: &Actor) -> String {
    actor.user_id().map(String::from).unwrap_or_else(|| actor.label())
}

/// What a path names before its `@` segment.
enum Found {
    Outer(Outer, Vec<String>),
    Nothing,
}

impl AppState {
    fn find_outer(&self, root_id: &str, rel: &str) -> NfsResult<Found> {
        let segs: Vec<&str> = rel.split('/').collect();
        let Some(cfg) = self.config.root(root_id) else { return Ok(Found::Nothing) };
        for i in 1..segs.len() {
            if segs[i] != SEGMENT {
                continue;
            }
            let outer_rel = segs[..i].join("/");
            let outer = match lstat(&join_root(&cfg.path, &outer_rel)) {
                Ok(m) if m.kind == "file" && !m.special => Outer::Native { root: root_id.to_string(), rel: outer_rel, meta: m },
                Ok(m) if m.kind == "symlink" => return Err(unsupported("symlink", "an archive reached through a symbolic link cannot be opened")),
                Ok(_) => continue,
                Err(e) if e.code == ErrorCode::NotFound => match self.projection_at(root_id, &outer_rel) {
                    Ok(Some(p)) => match &p.at {
                        ProjAt::File { binding, .. } => Outer::Projection {
                            rel: p.rel.clone(),
                            obj_id: binding.obj_id.clone().ok_or_else(|| unsupported("not_file_object", "the projected file has no content object"))?,
                            etag: binding.etag.clone(),
                        },
                        _ => continue,
                    },
                    _ => return Ok(Found::Nothing),
                },
                Err(_) => return Ok(Found::Nothing),
            };
            return Ok(Found::Outer(outer, segs[i + 1..].iter().map(|s| s.to_string()).collect()));
        }
        Ok(Found::Nothing)
    }

    /// `resolve_native` found nothing at `rel`: an address inside an archive (§2.2).
    pub(crate) fn archive_split(&self, root_id: &str, rel: &str) -> NfsResult<Option<Node>> {
        match self.find_outer(root_id, rel)? {
            Found::Nothing => Ok(None),
            Found::Outer(outer, rest) => {
                if rest.first().map(String::as_str) != Some(BRANCH) {
                    return Err(not_found(format!("'/{root_id}/{rel}' not found (archive members are under '@/{BRANCH}')")));
                }
                self.archive_at(outer, &rest[1..].join("/")).map(Some)
            }
        }
    }

    /// Host-realm paths into a file (D15: no archives there).
    pub(crate) fn host_archive_unsupported(rel: &str) -> Option<NfsError> {
        rel.split('/').any(|s| s == SEGMENT).then(|| unsupported("host_realm", "archives on this computer's files are not supported"))
    }

    fn archive_index(&self, outer: &Outer) -> NfsResult<Arc<Index>> {
        match outer {
            Outer::Native { root, rel, meta } => {
                let key = native_key(root, rel, meta);
                if let Some(index) = self.archive.cached(&key) {
                    return Ok(index);
                }
                if let Some(e) = failure(&key) {
                    return Err(e);
                }
                let cfg = self.config.root(root).ok_or_else(|| stale("root gone"))?;
                let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
                let build = cache::Build { mgr: &self.archive, unpack: false, cancel: &AtomicBool::new(false) };
                let index = Arc::new(Index::build(build, key.clone(), join_root(&cfg.path, rel), Some(stamp_of(meta)), name, None)?);
                self.archive.forget_versions(&native_prefix(root, rel), &key);
                self.archive.insert(index.clone());
                Ok(index)
            }
            Outer::Projection { obj_id, .. } | Outer::Object { obj_id } => {
                let key = object_key(obj_id);
                if let Some(index) = self.archive.cached(&key) {
                    return Ok(index);
                }
                if let Some(e) = failure(&key) {
                    return Err(e);
                }
                if matches!(outer, Outer::Object { .. }) && !granted(obj_id) {
                    return Err(NfsError::new(ErrorCode::PermissionDenied, format!("not allowed to read {obj_id}")));
                }
                Err(unsupported("archive_not_prepared", "this request cannot open archives held as objects"))
            }
        }
    }

    /// A locked index is replaced by the requester's view of it once they unlocked it (E7).
    fn archive_view(&self, base: Arc<Index>) -> Arc<Index> {
        if !base.locked() {
            return base;
        }
        ctx_user().and_then(|u| self.archive.cached(&cache::view_key(&base.key, &u))).unwrap_or(base)
    }

    fn archive_at(&self, outer: Outer, member: &str) -> NfsResult<Node> {
        for seg in member.split('/').filter(|s| !s.is_empty()) {
            validate_name(seg)?;
        }
        let member = member.trim_matches('/');
        let index = self.archive_view(self.archive_index(&outer)?);
        if index.locked() && !member.is_empty() {
            return Err(password_required());
        }
        let trees = index.naming.trees_for(pref());
        for tree in &trees {
            if let Some(idx) = tree.by_path(member) {
                return Ok(Node::Archive(Box::new(ArchiveNode { outer, index: index.clone(), tree: tree.clone(), node: idx })));
            }
        }
        if trees.iter().any(|t| t.is_ambiguous(member)) {
            return Err(NfsError::new(ErrorCode::AmbiguousEntry, format!("more than one member is named '{member}'")));
        }
        let segs: Vec<&str> = member.split('/').collect();
        for k in 1..segs.len() {
            let prefix = segs[..k].join("/");
            if let Some(t) = trees.iter().find(|t| t.by_path(&prefix).is_some()) {
                if !t.nodes[t.by_path(&prefix).unwrap()].dir {
                    if segs[k] == SEGMENT {
                        return Err(unsupported("nested_archive", "archives inside archives cannot be opened"));
                    }
                    return Err(NfsError::new(ErrorCode::NotAContainer, format!("'{prefix}' is a file")));
                }
            }
        }
        Err(not_found(format!("no member '{member}' in the archive")))
    }

    pub(crate) fn resolve_archive_object(&self, obj_id: &str, inner_path: &str) -> NfsResult<Node> {
        let member = inner_path
            .strip_prefix(INNER_ROOT)
            .filter(|r| r.is_empty() || r.starts_with('/'))
            .ok_or_else(|| unsupported("inner_path", "object refs resolve only under /archive"))?;
        self.archive_at(Outer::Object { obj_id: obj_id.to_string() }, member)
    }

    pub(crate) fn resolve_archive_handle(&self, node_id: &str) -> NfsResult<Node> {
        let (outer, raw, kind) = if node_id.starts_with("no_") {
            let h: NoHandle = serde_json::from_slice(&self.handles.open("no_", node_id)?).map_err(|_| stale("malformed handle payload"))?;
            (Outer::Object { obj_id: h.o }, h.e, h.k)
        } else {
            let h: NaHandle = serde_json::from_slice(&self.handles.open("na_", node_id)?).map_err(|_| stale("malformed handle payload"))?;
            let outer = match h.o {
                Some(obj_id) => {
                    let p = self.projection_at(HOME_ROOT, &h.p).map_err(|e| if e.code == ErrorCode::NotFound { stale(e.message) } else { e })?;
                    match p.map(|p| p.at) {
                        Some(ProjAt::File { binding, .. }) if binding.obj_id.as_deref() == Some(&obj_id) && Some(&binding.etag) == h.t.as_ref() => Outer::Projection { rel: h.p.clone(), obj_id, etag: binding.etag },
                        _ => return Err(stale("the archive changed; open it again")),
                    }
                }
                None => {
                    let cfg = self.config.root(&h.r).ok_or_else(|| stale(format!("export root '{}' no longer exists", h.r)))?;
                    let meta = lstat(&join_root(&cfg.path, &h.p)).map_err(|e| if e.code == ErrorCode::NotFound { stale("the archive is gone") } else { e })?;
                    if meta.kind != "file" || meta.id.ino != h.i || meta.id.dev != h.d || h.b.is_some_and(|b| Some(b) != meta.birth) || meta.size != h.s || meta.mtime != h.m {
                        return Err(stale("the archive changed; open it again"));
                    }
                    Outer::Native { root: h.r, rel: h.p, meta }
                }
            };
            (outer, h.e, h.k)
        };
        let raw = B64.decode(raw).map_err(|_| stale("malformed handle payload"))?;
        let index = self.archive_view(self.archive_index(&outer)?);
        if index.locked() {
            return Err(password_required());
        }
        for tree in index.naming.trees_for(pref()) {
            if let Some(idx) = tree.by_raw(&raw) {
                if (kind == "d") != tree.nodes[idx].dir {
                    return Err(stale("the member changed kind"));
                }
                return Ok(Node::Archive(Box::new(ArchiveNode { outer, index: index.clone(), tree, node: idx })));
            }
        }
        Err(stale("the member is no longer addressable"))
    }

    pub fn archive_handle(&self, a: &ArchiveNode) -> String {
        let e = B64.encode(&a.tnode().raw);
        let k = if a.is_dir() { "d" } else { "f" }.to_string();
        match &a.outer {
            Outer::Object { obj_id } => self.handles.seal("no_", &serde_json::to_vec(&NoHandle { o: obj_id.clone(), e, k }).unwrap()),
            Outer::Native { root, rel, meta } => self.handles.seal(
                "na_",
                &serde_json::to_vec(&NaHandle { r: root.clone(), p: rel.clone(), i: meta.id.ino, d: meta.id.dev, b: meta.birth, s: meta.size, m: meta.mtime, t: None, o: None, e, k }).unwrap(),
            ),
            Outer::Projection { rel, obj_id, etag } => self.handles.seal(
                "na_",
                &serde_json::to_vec(&NaHandle { r: HOME_ROOT.into(), p: rel.clone(), i: 0, d: 0, b: None, s: 0, m: 0, t: Some(etag.clone()), o: Some(obj_id.clone()), e, k }).unwrap(),
            ),
        }
    }

    pub fn archive_ref(&self, a: &ArchiveNode) -> WireRef {
        match &a.outer {
            Outer::Object { obj_id } => WireRef::Object { obj_id: obj_id.clone(), inner_path: Some(a.inner_path()) },
            _ => WireRef::live(self.archive_handle(a), 0),
        }
    }

    /// Path sources need `read` on the archive file itself; object sources a
    /// readable object (`ndm::may_read`, checked per request).
    pub fn archive_level(&self, actor: &Actor, a: &ArchiveNode) -> Level {
        let ok = match &a.outer {
            Outer::Native { root, rel, .. } => self.acls.level(actor, root, rel) >= Level::Read,
            Outer::Projection { rel, .. } => self.acls.level(actor, HOME_ROOT, rel) >= Level::Read,
            Outer::Object { obj_id } => granted(obj_id),
        };
        if ok {
            Level::Read
        } else {
            Level::None
        }
    }

    pub(crate) fn archive_require(&self, actor: &Actor, a: &ArchiveNode, min: Level, op: &str) -> NfsResult<Level> {
        if min >= Level::Write || op != "read" {
            return Err(read_only());
        }
        let level = self.archive_level(actor, a);
        if level < Level::Read {
            return Err(crate::access::denied("read", actor));
        }
        Ok(level)
    }

    fn archive_attrs(&self, actor: &Actor, a: &ArchiveNode, want: &WantMask) -> serde_json::Map<String, Value> {
        let mut attrs = serde_json::Map::new();
        let m = a.member().cloned();
        if want.has("base") {
            attrs.insert("size".into(), json!(if a.is_dir() { 0 } else { m.as_ref().map(|m| m.size).unwrap_or(0) }));
            attrs.insert("mtime".into(), json!(a.mtime()));
            attrs.insert("flags".into(), json!(["read_only"]));
            if let (Some(m), false) = (&m, a.is_dir()) {
                if let Some(c) = m.compressed {
                    attrs.insert("compressed_size".into(), json!(c));
                }
                attrs.insert("encrypted".into(), json!(m.encrypted));
                attrs.insert("method".into(), json!(m.method));
            }
        }
        if want.has("ident") {
            if let (Some(m), false) = (&m, a.is_dir()) {
                attrs.insert("etag".into(), json!(a.etag(m).0));
            }
        }
        if want.has("access") {
            attrs.insert("access_urls".into(), json!(self.archive_access_urls(a)));
        }
        let _ = actor;
        attrs
    }

    fn archive_access_urls(&self, a: &ArchiveNode) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(url) = a.canonical() {
            out.push(json!({"kind": "fs", "url": url, "primary": true}));
        }
        if !a.is_dir() {
            out.push(json!({"kind": "read", "url": format!("/nfs/v1/read/{}", self.archive_handle(a))}));
        }
        out
    }

    pub fn archive_summary(&self, a: &ArchiveNode) -> Value {
        let tree = &a.tree;
        let f = &a.index.format;
        let mut out = json!({
            "format": f.kind.name(),
            "entries": a.index.members.len(),
            "hidden_entries": tree.hidden,
            "encrypted_entries": a.index.encrypted_entries(),
            "name_encoding": {"encoding": tree.codec.name(), "by": tree.by},
        });
        if let Some(c) = f.compression {
            out["compression"] = json!(c.name());
        }
        if f.kind == Kind::SevenZ {
            out["solid"] = json!(f.solid);
            out["header_encrypted"] = json!(f.header_encrypted);
            out["name_encoding"] = sevenz::summary_encoding();
        }
        out
    }

    pub fn archive_info(&self, actor: &Actor, a: &ArchiveNode, want: &WantMask) -> NfsResult<Value> {
        if a.index.locked() {
            return Err(password_required());
        }
        let level = self.archive_level(actor, a);
        let caps = if a.is_dir() { archive_dir_capabilities() } else { archive_file_capabilities() };
        let mut out = json!({
            "kind": a.kind(),
            "state": "live",
            "ref": self.archive_ref(a),
            "copy_ref": (!a.is_dir()).then(|| self.archive_ref(a)),
            "capabilities": caps,
            "access_level": level,
            "archive_member": true,
        });
        if a.is_dir() {
            out["revision"] = json!(a.revision());
        }
        out["locations"] = json!(a.canonical().map(|url| vec![json!({"url": url, "via": "direct"})]).unwrap_or_default());
        if let Some(c) = a.canonical() {
            out["canonical_path"] = json!(c);
        }
        let mut attrs = self.archive_attrs(actor, a, want);
        if a.is_root() {
            attrs.insert("archive".into(), self.archive_summary(a));
        }
        if want.has("base") {
            out["name"] = json!(a.name());
            out["size"] = attrs.get("size").cloned().unwrap_or(json!(0));
            out["mtime"] = json!(a.mtime());
            out["ctime"] = json!(a.mtime());
            out["flags"] = json!(["read_only"]);
        }
        if want.has("ident") {
            out["node_id"] = json!(self.archive_handle(a));
            out["gen"] = json!(0);
            if let Some(etag) = attrs.get("etag") {
                out["etag"] = etag.clone();
            }
        }
        if want.has("access") {
            out["access_urls"] = json!(self.archive_access_urls(a));
        }
        out["attrs"] = Value::Object(attrs);
        Ok(out)
    }

    /// List rows; their JSON is filled in for the emitted page only.
    pub fn archive_rows(&self, a: &ArchiveNode) -> NfsResult<Vec<EntryRow>> {
        if a.index.locked() {
            return Err(password_required());
        }
        Ok(a.tnode()
            .children
            .iter()
            .map(|&c| {
                let n = &a.tree.nodes[c];
                let m = n.entry.map(|e| &a.index.members[e]);
                EntryRow {
                    name: n.name.clone(),
                    kind: if n.dir { "dir" } else { "file" }.to_string(),
                    mtime: m.map(|m| m.mtime).unwrap_or(0),
                    size: if n.dir { 0 } else { m.map(|m| m.size).unwrap_or(0) },
                    order_index: c as i64,
                    json: Value::Null,
                }
            })
            .collect())
    }

    pub fn archive_entry_json(&self, actor: &Actor, a: &ArchiveNode, child: usize, want: &WantMask) -> Value {
        let c = a.child(child);
        let mut entry = json!({
            "entry_ref": format!("ae_{}", &hex::encode(Sha256::digest(&c.tnode().raw))[..24]),
            "name": c.tnode().name,
            "binding": "native",
            "target": {
                "ref": self.archive_ref(&c),
                "copy_ref": (!c.is_dir()).then(|| self.archive_ref(&c)),
                "kind": c.kind(),
                "attrs": Value::Object(self.archive_attrs(actor, &c, want)),
                "access_level": self.archive_level(actor, &c),
            },
        });
        if let Some(cp) = c.canonical() {
            entry["canonical_path"] = json!(cp);
        }
        entry
    }

    pub fn archive_walk(&self, a: &ArchiveNode, name: &str) -> NfsResult<Node> {
        validate_name(name)?;
        if a.index.locked() {
            return Err(password_required());
        }
        if !a.is_dir() {
            return Err(NfsError::new(ErrorCode::NotAContainer, "cannot walk into a file"));
        }
        match a.tree.child(a.node, name) {
            Some(idx) => Ok(Node::Archive(Box::new(a.child(idx)))),
            None => {
                let path = if a.tnode().path.is_empty() { name.to_string() } else { format!("{}/{name}", a.tnode().path) };
                if a.tree.is_ambiguous(&path) {
                    return Err(NfsError::new(ErrorCode::AmbiguousEntry, format!("more than one member is named '{path}'")));
                }
                Err(not_found(format!("no member '{path}' in the archive")))
            }
        }
    }
}

// ---------- preparation (async half of resolution) ----------

#[derive(Default)]
struct Wanted {
    objects: Vec<String>,
    handles: Vec<String>,
    paths: Vec<String>,
}

fn scan(v: &Value, out: &mut Wanted, budget: &mut usize) {
    if *budget == 0 {
        return;
    }
    match v {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("object") {
                if let (Some(obj), Some(inner)) = (map.get("obj_id").and_then(Value::as_str), map.get("inner_path").and_then(Value::as_str)) {
                    if inner == INNER_ROOT || inner.starts_with("/archive/") {
                        out.objects.push(obj.to_string());
                        *budget -= 1;
                    }
                }
            }
            for value in map.values() {
                scan(value, out, budget);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| scan(i, out, budget)),
        Value::String(s) if s.starts_with("no_") || s.starts_with("na_") => {
            out.handles.push(s.clone());
            *budget -= 1;
        }
        Value::String(s) if s.contains("/@/") => {
            out.paths.push(s.clone());
            *budget -= 1;
        }
        _ => {}
    }
}

/// Builds what the synchronous resolution of these values will need: object
/// outers landed (after the principal's read check) and indexes parsed off
/// the async threads. Failures are kept for the resolution to report.
pub async fn prepare(state: &SharedState, actor: &Actor, headers: &HeaderMap, values: &[&Value]) {
    let Some(ctx) = current() else { return };
    let user = user_key(actor);
    *ctx.user.lock().unwrap() = Some(user.clone());
    let mut wanted = Wanted::default();
    let mut budget = 32;
    for v in values {
        scan(v, &mut wanted, &mut budget);
    }
    let mut principal: Option<Option<crate::ndm::Principal>> = None;
    let mut objects: Vec<String> = wanted.objects;
    let mut natives: Vec<(String, String)> = Vec::new();
    let mut projections: Vec<(String, String)> = Vec::new();
    for h in &wanted.handles {
        if h.starts_with("no_") {
            if let Ok(p) = state.handles.open("no_", h).and_then(|b| serde_json::from_slice::<NoHandle>(&b).map_err(|_| stale("bad handle"))) {
                objects.push(p.o);
            }
        } else if let Ok(p) = state.handles.open("na_", h).and_then(|b| serde_json::from_slice::<NaHandle>(&b).map_err(|_| stale("bad handle"))) {
            match p.o {
                Some(obj) => projections.push((p.p, obj)),
                None => natives.push((p.r, p.p)),
            }
        }
    }
    for path in &wanted.paths {
        let path = match current_zone_cyfs_path(path) {
            Ok(Some(p)) => p,
            Ok(None) if path.starts_with('/') => path.clone(),
            _ => continue,
        };
        let Ok(rel_all) = normalize_rel(&path) else { continue };
        let Some((root, rel)) = rel_all.split_once('/') else { continue };
        if root == HOST_ROOT {
            continue;
        }
        match state.find_outer(root, rel) {
            Ok(Found::Outer(Outer::Native { root, rel, .. }, _)) => natives.push((root, rel)),
            Ok(Found::Outer(Outer::Projection { rel, obj_id, .. }, _)) => projections.push((rel, obj_id)),
            _ => {}
        }
    }
    for (root, rel) in natives {
        if state.acls.level(actor, &root, &rel) < Level::Read {
            continue;
        }
        let Some(cfg) = state.config.root(&root) else { continue };
        let Ok(meta) = lstat(&join_root(&cfg.path, &rel)) else { continue };
        if meta.kind != "file" {
            continue;
        }
        let key = native_key(&root, &rel, &meta);
        let built = match state.archive.cached(&key) {
            Some(index) => Ok(index),
            None => {
                let (st, k, (r, l)) = (state.clone(), key.clone(), (root.clone(), rel.clone()));
                state
                    .archive
                    .indexing
                    .run(&key, move |cancel| async move {
                        let path = join_root(&st.config.root(&r).ok_or_else(|| stale("root gone"))?.path, &l);
                        let name = l.rsplit('/').next().unwrap_or(&l).to_string();
                        let stamp = stamp_of(&meta);
                        let s2 = st.clone();
                        let index = tokio::task::spawn_blocking(move || Index::build(cache::Build { mgr: &s2.archive, unpack: true, cancel: &cancel }, k, path, Some(stamp), name, None))
                            .await
                            .map_err(|e| internal(format!("index task: {e}")))??;
                        let index = Arc::new(index);
                        st.archive.forget_versions(&native_prefix(&r, &l), &index.key);
                        st.archive.insert(index.clone());
                        Ok(index)
                    })
                    .await
            }
        };
        match built {
            Ok(index) if index.locked() => open_view(state, &user, &index, Some((root, rel))).await,
            Ok(_) => {}
            Err(e) => {
                ctx.failed.lock().unwrap().insert(key, e);
            }
        }
    }
    for (rel, obj) in projections {
        if state.acls.level(actor, HOME_ROOT, &rel) < Level::Read {
            continue;
        }
        match prepare_object(state, &obj).await {
            Ok(index) if index.locked() => open_view(state, &user, &index, None).await,
            Ok(_) => {}
            Err(e) => {
                ctx.failed.lock().unwrap().insert(object_key(&obj), e);
            }
        }
    }
    objects.sort();
    objects.dedup();
    for obj in objects {
        if !granted(&obj) {
            let p = match &principal {
                Some(p) => p.clone(),
                None => {
                    let p = crate::ndm::principal(state, headers).await.ok();
                    principal = Some(p.clone());
                    p
                }
            };
            let Some(p) = p else { continue };
            if !crate::ndm::may_read(state, &p, &obj).await {
                continue;
            }
            ctx.grant(&obj);
        }
        match prepare_object(state, &obj).await {
            Ok(index) if index.locked() => open_view(state, &user, &index, None).await,
            Ok(_) => {}
            Err(e) => {
                ctx.failed.lock().unwrap().insert(object_key(&obj), e);
            }
        }
    }
}

/// Opens `user`'s view of a locked index with a password they gave before
/// (this session's, or a remembered one); otherwise it stays locked (E7).
async fn open_view(state: &SharedState, user: &str, base: &Arc<Index>, native: Option<(String, String)>) {
    if state.archive.cached(&cache::view_key(&base.key, user)).is_some() {
        return;
    }
    let (mut candidates, _) = state.archive.passwords.session_candidates(user, base.password_key());
    let sha = match (&base.sha256, native) {
        (Some(sha), _) => Some(sha.clone()),
        (None, Some((root, rel))) if password::has_any(&state.db, user) => native_sha(state, &root, &rel).await,
        _ => None,
    };
    if let Some(sha) = sha {
        candidates.extend(password::stored(&state.db, user, &sha).unwrap_or_default());
    }
    for pw in candidates {
        let (b, u, p) = (base.clone(), user.to_string(), pw.clone());
        if let Ok(Ok(view)) = tokio::task::spawn_blocking(move || b.unlocked(&u, &p)).await {
            state.archive.insert(Arc::new(view));
            state.archive.passwords.promote(user, base.password_key(), &pw);
            return;
        }
    }
}

/// Lands an object outer and parses it (D16); callers checked the read right.
pub async fn prepare_object(state: &SharedState, obj_id: &str) -> NfsResult<Arc<Index>> {
    let key = object_key(obj_id);
    if let Some(index) = state.archive.cached(&key) {
        return Ok(index);
    }
    let (st, obj) = (state.clone(), obj_id.to_string());
    state
        .archive
        .indexing
        .run(&key, move |cancel| async move {
            let (path, sha, name) = cache::object_outer(&st, &obj).await?;
            let key = object_key(&obj);
            let s2 = st.clone();
            let index = tokio::task::spawn_blocking(move || Index::build(cache::Build { mgr: &s2.archive, unpack: true, cancel: &cancel }, key, path, None, name, Some(sha)))
                .await
                .map_err(|e| internal(format!("index task: {e}")))??;
            let index = Arc::new(index);
            st.archive.insert(index.clone());
            Ok(index)
        })
        .await
}

// ---------- passwords ----------

async fn native_sha(state: &SharedState, root: &str, rel: &str) -> Option<String> {
    let path = join_root(&state.config.root(root)?.path, rel);
    crate::preview::native_obj_id(&state.db, &format!("{root}/{rel}"), &path).await.ok().map(|id| id.trim_start_matches("sha256:").to_string())
}

/// Whole-file sha256 (hex) of the outer, when it can be had.
async fn outer_sha(state: &SharedState, a: &ArchiveNode) -> Option<String> {
    if let Some(sha) = &a.index.sha256 {
        return Some(sha.clone());
    }
    let Outer::Native { root, rel, .. } = &a.outer else { return None };
    native_sha(state, root, rel).await
}

async fn quick_check(index: &Arc<Index>, m: &Member, password: &str) -> bool {
    let (index, m, pw) = (index.clone(), m.clone(), password.to_string());
    tokio::task::spawn_blocking(move || format::quick_check(&index, &m, &pw)).await.unwrap_or(false)
}

/// The password this user reads `m` with: verified in the session, stored, then pending.
async fn password_for(state: &SharedState, actor: &Actor, a: &ArchiveNode, m: &Member) -> NfsResult<(String, Origin)> {
    let user = user_key(actor);
    let (verified, pending) = state.archive.passwords.session_candidates(&user, a.index.password_key());
    for pw in verified {
        if quick_check(&a.index, m, &pw).await {
            return Ok((pw, Origin::Verified));
        }
    }
    if let Some(sha) = outer_sha(state, a).await {
        for pw in password::stored(&state.db, &user, &sha)? {
            if quick_check(&a.index, m, &pw).await {
                return Ok((pw, Origin::Stored));
            }
        }
    }
    for pw in pending {
        if quick_check(&a.index, m, &pw).await {
            return Ok((pw, Origin::Pending));
        }
    }
    Err(NfsError::new(ErrorCode::PasswordRequired, "this member is encrypted; unlock the archive first"))
}

/// After a complete read: a password that decoded it is verified (and kept
/// once the archive's hash is known); one that did not is no longer pending.
async fn settle(state: &SharedState, user: &str, a: &ArchiveNode, pw: &(String, Origin), ok: bool) {
    let sessions = &state.archive.passwords;
    let key = a.index.password_key();
    if !ok {
        if pw.1 == Origin::Pending {
            sessions.drop_pending(user, key, &pw.0);
        }
        return;
    }
    sessions.promote(user, key, &pw.0);
    if pw.1 != Origin::Stored {
        if let Some(sha) = outer_sha(state, a).await {
            if let Err(e) = password::store(&state.db, user, &sha, &pw.0) {
                log::warn!("archive: keeping a verified password failed: {}", e.message);
            }
        }
    }
}

pub async fn op_unlock(state: &SharedState, actor: &Actor, env: &Envelope) -> NfsResult<Value> {
    let at = env.at.as_ref().ok_or_else(|| invalid("archive.unlock requires at"))?;
    let node = state.resolve_for(actor, at, Level::Read, "read")?;
    let Node::Archive(a) = node else { return Err(invalid("archive.unlock needs an archive or a member")) };
    let password = env.args["password"].as_str().filter(|p| !p.is_empty()).ok_or_else(|| invalid("archive.unlock requires args.password"))?;
    let user = user_key(actor);
    let pkey = a.index.password_key().to_string();
    let lock = state.archive.passwords.unlock_lock(&user, &pkey);
    let _held = lock.lock().await;
    if a.index.locked() {
        // The header carries a CRC: opening it is a complete check (E7).
        let (b, u, p) = (a.index.clone(), user.clone(), password.to_string());
        let view = tokio::task::spawn_blocking(move || b.unlocked(&u, &p)).await.map_err(|e| internal(format!("unlock task: {e}")))??;
        state.archive.insert(Arc::new(view));
        state.archive.passwords.promote(&user, &pkey, password);
        if let Some(sha) = outer_sha(state, &a).await {
            if let Err(e) = password::store(&state.db, &user, &sha, password) {
                log::warn!("archive: keeping a verified password failed: {}", e.message);
            }
        }
        return Ok(json!({"ok": true, "encrypted": true}));
    }
    let target = match a.member().filter(|m| !a.is_dir() && m.encrypted) {
        Some(m) => Some(m.clone()),
        None => a.index.members.iter().find(|m| m.encrypted && m.readable && m.supported().is_ok()).cloned(),
    };
    let Some(m) = target else { return Ok(json!({"ok": true, "encrypted": false})) };
    if !quick_check(&a.index, &m, password).await {
        return Err(NfsError::new(ErrorCode::PasswordInvalid, "the password is not right for this archive"));
    }
    state.archive.passwords.add_pending(&user, &pkey, password);
    Ok(json!({"ok": true, "encrypted": true}))
}

pub async fn op_forget(state: &SharedState, actor: &Actor, env: &Envelope) -> NfsResult<Value> {
    let user = user_key(actor);
    if env.args["all"].as_bool() == Some(true) {
        state.archive.passwords.forget(&user, None);
        state.archive.remove_views(&user);
        let n = password::forget(&state.db, &user, None)?;
        return Ok(json!({"forgotten": n}));
    }
    let at = env.at.as_ref().ok_or_else(|| invalid("archive.forget requires at or args.all"))?;
    let node = state.resolve_for(actor, at, Level::Read, "read")?;
    let Node::Archive(a) = node else { return Err(invalid("archive.forget needs an archive")) };
    state.archive.passwords.forget(&user, Some(a.index.password_key()));
    state.archive.remove(&cache::view_key(a.index.password_key(), &user));
    let n = match outer_sha(state, &a).await {
        Some(sha) => password::forget(&state.db, &user, Some(&sha))?,
        None => 0,
    };
    Ok(json!({"forgotten": n}))
}

/// `(tar extensions, single compressed file extensions)`.
fn stream_extensions() -> (Vec<&'static str>, Vec<&'static str>) {
    let mut tar_ext = vec!["tar"];
    let mut single_ext = Vec::new();
    for comp in Compression::ALL {
        tar_ext.extend_from_slice(comp.tar_extensions());
        single_ext.extend_from_slice(comp.extensions());
    }
    (tar_ext, single_ext)
}

pub fn describe(state: &AppState) -> Value {
    let c = &state.archive.config;
    let (tar_ext, single_ext) = stream_extensions();
    let compression: Vec<&str> = std::iter::once("none").chain(Compression::ALL.iter().map(|c| c.name())).collect();
    json!({
        "available": true,
        "formats": [
            {"format": "zip", "extensions": ["zip"], "methods": zip::VERIFIED_METHODS, "encryption": zip::ENCRYPTION},
            {"format": "tar", "extensions": tar_ext, "compression": compression},
            {"format": "7z", "extensions": ["7z"], "methods": sevenz::METHODS, "filters": sevenz::FILTERS, "encryption": sevenz::ENCRYPTION},
            {"format": "compressed", "extensions": single_ext, "compression": &compression[1..]},
        ],
        "limits": {
            "max_central_directory_bytes": c.max_central_directory_bytes,
            "max_entries": c.max_entries,
            "max_materialize_bytes": c.max_materialize_bytes,
            "max_decoder_memory": c.max_decoder_memory,
        },
    })
}

// ---------- data plane ----------

/// Cache file of a member: encrypted members are kept per user, so nobody
/// reads plaintext another user's password produced.
fn cache_path(state: &AppState, actor: &Actor, a: &ArchiveNode, m: &Member) -> PathBuf {
    let entry = a.tnode().entry.unwrap_or(usize::MAX);
    let user = m.encrypted.then(|| user_key(actor));
    state.archive.member_path(&a.index, entry, user.as_deref())
}

/// Decodes a member into the cache (once for concurrent callers) and returns the file.
pub async fn materialize(state: &SharedState, actor: &Actor, a: &ArchiveNode) -> NfsResult<PathBuf> {
    let m = a.member().filter(|_| !a.is_dir()).ok_or_else(|| invalid("not a file in the archive"))?.clone();
    m.supported()?;
    let password = if m.encrypted { Some(password_for(state, actor, a, &m).await?) } else { None };
    let dest = cache_path(state, actor, a, &m);
    if state.archive.touch(&dest) {
        return Ok(dest);
    }
    if m.solid().is_some() {
        return block_member(state, actor, a, &m, password).await;
    }
    state.archive.reserve(m.size)?;
    let key = dest.to_string_lossy().into_owned();
    let (st, node, user) = (state.clone(), a.clone(), user_key(actor));
    state
        .archive
        .members
        .run(&key, move |cancel| async move {
            let permit = st.archive.workers.clone().acquire_owned().await.map_err(|e| internal(e.to_string()))?;
            let tmp = st.archive.tmp_path(&st.archive.members_dir());
            let (index, mm, pw, d) = (node.index.clone(), m.clone(), password.clone(), dest.clone());
            let done = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                cache::materialize_blocking(&index, &mm, pw.as_ref().map(|p| p.0.as_str()), &tmp, &d, &cancel)
            })
            .await
            .map_err(|e| internal(format!("materialize task: {e}")))?;
            if let Some(pw) = &password {
                settle(&st, &user, &node, pw, matches!(done, Ok(true))).await;
            }
            match done? {
                true => {
                    st.archive.record(&dest, m.size);
                    Ok(dest)
                }
                false => Err(internal("cancelled")),
            }
        })
        .await
}

// ---------- solid 7z blocks (E8) ----------

fn block_key(index: &Index, block: usize, user: Option<&str>) -> String {
    format!("{}#b{block}#{}", index.key, user.unwrap_or(""))
}

/// Joins the extraction of a solid block, starting it when none runs: one
/// pass that writes every file of the block into the member cache, in
/// order, each as soon as its CRC checked out. It runs to the block's end
/// whoever still waits (the next and previous files are likely wanted soon).
fn join_block(state: &SharedState, actor: &Actor, a: &ArchiveNode, block: usize, password: Option<String>) -> tokio::sync::watch::Receiver<cache::BlockState> {
    let encrypted = password.is_some();
    let user = encrypted.then(|| user_key(actor));
    let key = block_key(&a.index, block, user.as_deref());
    let mut jobs = state.archive.blocks.lock().unwrap();
    if let Some(rx) = jobs.get(&key) {
        if rx.borrow().end.is_none() {
            return rx.clone();
        }
    }
    let (tx, rx) = tokio::sync::watch::channel(cache::BlockState::default());
    jobs.insert(key.clone(), rx.clone());
    drop(jobs);
    let (st, index) = (state.clone(), a.index.clone());
    tokio::spawn(async move {
        let result = async {
            let permit = st.archive.workers.clone().acquire_owned().await.map_err(|e| internal(e.to_string()))?;
            let s2 = st.clone();
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                extract_block(&s2.archive, &index, block, password.as_deref(), user.as_deref(), &tx)
            })
            .await
            .map_err(|e| internal(format!("block task: {e}")))?
        }
        .await;
        let mut jobs = st.archive.blocks.lock().unwrap();
        if let Some(rx) = jobs.get(&key) {
            if rx.borrow().end.is_none() {
                jobs.remove(&key);
            }
        }
        drop(jobs);
        if let Err(e) = &result {
            log::info!("archive: extracting block {block} of {key} stopped: {}", e.message);
        }
        result
    });
    rx
}

fn extract_block(mgr: &cache::ArchiveMgr, index: &Index, block: usize, password: Option<&str>, user: Option<&str>, tx: &tokio::sync::watch::Sender<cache::BlockState>) -> NfsResult<()> {
    use std::io::Write;
    let cancel = AtomicBool::new(false);
    let result = sevenz::for_block(index, block, password, &mut |file, r| {
        tx.send_modify(|s| s.ready_before = file);
        let m = &index.members[file];
        if m.size == 0 || m.solid().is_none() {
            return Ok(true);
        }
        let dest = mgr.member_path(index, file, m.encrypted.then_some(user).flatten());
        if mgr.touch(&dest) || mgr.reserve(m.size).is_err() {
            return Ok(true);
        }
        let tmp = mgr.tmp_path(&mgr.members_dir());
        let mut out = std::io::BufWriter::new(cache::create_private_sync(&tmp)?);
        let done = source::pump(r, m.size, None, &cancel, &mut |chunk| out.write_all(chunk).map(|_| true));
        let kept = match done {
            Ok(true) => out.into_inner().map_err(|e| NfsError::from(e.into_error())).and_then(|f| Ok(f.sync_all()?)).and_then(|_| Ok(std::fs::rename(&tmp, &dest)?)),
            Ok(false) => Err(internal("cancelled")),
            Err(e) => Err(e),
        };
        if let Err(e) = kept {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        mgr.record(&dest, m.size);
        Ok(true)
    });
    tx.send_modify(|s| {
        if result.is_ok() {
            s.ready_before = usize::MAX;
        }
        s.end = Some(result.clone());
    });
    result
}

/// A solid member: from the cache, else once its block's extraction got past it.
async fn block_member(state: &SharedState, actor: &Actor, a: &ArchiveNode, m: &Member, password: Option<(String, Origin)>) -> NfsResult<PathBuf> {
    let (block, file) = m.solid().ok_or_else(|| internal("not a solid member"))?;
    let dest = cache_path(state, actor, a, m);
    for _ in 0..2 {
        if state.archive.touch(&dest) {
            break;
        }
        let mut rx = join_block(state, actor, a, block, password.as_ref().map(|p| p.0.clone()));
        let state_now = rx.wait_for(|s| s.ready_before > file || s.end.is_some()).await.map_err(|_| internal("block extraction ended without a result"))?.clone();
        if state.archive.touch(&dest) {
            break;
        }
        if let Some(Err(e)) = state_now.end {
            if let Some(pw) = &password {
                settle(state, &user_key(actor), a, pw, false).await;
            }
            return Err(e);
        }
    }
    if !state.archive.touch(&dest) {
        return Err(NfsError::new(ErrorCode::QuotaExceeded, "the archive cache cannot keep this file").with("limit", json!(state.archive.config.disk_budget_bytes)));
    }
    if let Some(pw) = &password {
        settle(state, &user_key(actor), a, pw, true).await;
    }
    Ok(dest)
}

fn error_response(e: NfsError) -> Response {
    let status = StatusCode::from_u16(e.code.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    (status, axum::Json(json!({"ok": false, "error": e.to_json()}))).into_response()
}

pub async fn read_member(state: &SharedState, actor: &Actor, a: &ArchiveNode, headers: &HeaderMap, q: &HashMap<String, String>) -> Response {
    match read_member_inner(state, actor, a, headers, q).await {
        Ok(r) => r,
        Err(e) => error_response(e),
    }
}

async fn read_member_inner(state: &SharedState, actor: &Actor, a: &ArchiveNode, headers: &HeaderMap, q: &HashMap<String, String>) -> NfsResult<Response> {
    let m = a.member().filter(|_| !a.is_dir()).ok_or_else(|| invalid("read target must be a file"))?.clone();
    m.supported()?;
    let name = a.name();
    let (_, etag) = a.etag(&m);
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }
    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let range = match crate::server::parse_range(range_header, m.size) {
        Ok(r) => r,
        Err(_) => return Ok((StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{}", m.size))]).into_response()),
    };
    let view = FileView { etag, name, q: q.clone(), immutable: a.object_id().is_some() };
    if m.direct() {
        let (index, mm) = (a.index.clone(), m.clone());
        let (start, file) = tokio::task::spawn_blocking(move || -> NfsResult<(u64, std::fs::File)> { Ok((format::data_start(&index, &mm)?, index.data.std_file().map_err(source::io_error)?)) })
            .await
            .map_err(|e| internal(e.to_string()))??;
        return Ok(crate::server::serve_file(tokio::fs::File::from_std(file), start, m.size, range, &view.etag, &view.name, &view.q, view.immutable).await);
    }
    let password = if m.encrypted { Some(password_for(state, actor, a, &m).await?) } else { None };
    let cached = cache_path(state, actor, a, &m);
    if state.archive.touch(&cached) {
        let file = tokio::fs::File::open(&cached).await?;
        return Ok(crate::server::serve_file(file, 0, m.size, range, &view.etag, &view.name, &view.q, view.immutable).await);
    }
    if m.solid().is_some() {
        let path = block_member(state, actor, a, &m, password).await?;
        let file = tokio::fs::File::open(&path).await?;
        return Ok(crate::server::serve_file(file, 0, m.size, range, &view.etag, &view.name, &view.q, view.immutable).await);
    }
    match range {
        None => stream(state, actor, a, &m, password, None, view).await,
        Some((0, end)) => stream(state, actor, a, &m, password, Some(end + 1), view).await,
        Some(_) => {
            let path = materialize(state, actor, a).await?;
            let file = tokio::fs::File::open(&path).await?;
            Ok(crate::server::serve_file(file, 0, m.size, range, &view.etag, &view.name, &view.q, view.immutable).await)
        }
    }
}

struct FileView {
    etag: String,
    name: String,
    q: HashMap<String, String>,
    immutable: bool,
}

/// Decodes on a worker into a bounded channel (backpressure; a client that
/// leaves stops it). Headers go out once the first bytes decoded (or the
/// member turned out empty); a failure later can only cut the connection (§7).
#[allow(clippy::too_many_arguments)]
async fn stream(state: &SharedState, actor: &Actor, a: &ArchiveNode, m: &Member, password: Option<(String, Origin)>, limit: Option<u64>, view: FileView) -> NfsResult<Response> {
    let permit = state.archive.workers.clone().acquire_owned().await.map_err(|e| internal(e.to_string()))?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(4);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<NfsResult<()>>();
    let (index, mm, pw) = (a.index.clone(), m.clone(), password.as_ref().map(|p| p.0.clone()));
    let worker = tokio::task::spawn_blocking(move || -> NfsResult<bool> {
        let _permit = permit;
        let mut ready = Some(ready_tx);
        let cancel = AtomicBool::new(false);
        let done = format::decode(&index, &mm, pw.as_deref(), limit, &cancel, &mut |chunk| {
            if let Some(r) = ready.take() {
                let _ = r.send(Ok(()));
            }
            Ok(tx.blocking_send(Ok(Bytes::copy_from_slice(chunk))).is_ok())
        });
        match (ready.take(), &done) {
            (Some(r), Err(e)) => {
                let _ = r.send(Err(e.clone()));
            }
            (Some(r), Ok(_)) => {
                let _ = r.send(Ok(()));
            }
            (None, Err(e)) => {
                let _ = tx.blocking_send(Err(std::io::Error::other(e.message.clone())));
            }
            (None, Ok(_)) => {}
        }
        done
    });
    if let Ok(Err(e)) = ready_rx.await {
        if let Some(pw) = &password {
            if e.code == ErrorCode::ArchiveCorrupt || e.code == ErrorCode::PasswordInvalid {
                settle(state, &user_key(actor), a, pw, false).await;
            }
        }
        return Err(e);
    }
    if let Some(pw) = password {
        let (st, node, user, partial) = (state.clone(), a.clone(), user_key(actor), limit.is_some_and(|l| l < m.size));
        tokio::spawn(async move {
            match worker.await {
                Ok(Ok(true)) if !partial => settle(&st, &user, &node, &pw, true).await,
                Ok(Err(_)) => settle(&st, &user, &node, &pw, false).await,
                _ => {}
            }
        });
    }
    let len = limit.map(|l| l.min(m.size)).unwrap_or(m.size);
    let body = Body::from_stream(futures::stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let mut builder = axum::http::Response::builder()
        .header(header::CONTENT_LENGTH, len)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, &view.etag)
        .header(header::CACHE_CONTROL, if view.immutable { "private, max-age=31536000, immutable" } else { "private, no-cache" });
    builder = if len < m.size {
        builder.status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes 0-{}/{}", len.saturating_sub(1), m.size))
    } else {
        builder.status(StatusCode::OK)
    };
    let mut response = builder.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    let want_download = view.q.contains_key("download");
    let file_name = view.q.get("name").filter(|_| want_download).unwrap_or(&view.name);
    ndn_lib::apply_content_policy(response.headers_mut(), crate::fsutil::content_type_for(&view.name), Some(file_name), want_download);
    Ok(response)
}

/// Writes a whole member to `out` (copy tasks): streamed, password checked
/// like a read, settled at the end; a solid member comes from its block's extraction.
pub async fn copy_member_to(state: &SharedState, actor: &Actor, a: &ArchiveNode, out: std::fs::File, cancel: Arc<AtomicBool>, progress: Arc<std::sync::atomic::AtomicU64>) -> NfsResult<()> {
    let m = a.member().filter(|_| !a.is_dir()).ok_or_else(|| invalid("only files can be copied out of an archive"))?.clone();
    m.supported()?;
    let password = if m.encrypted { Some(password_for(state, actor, a, &m).await?) } else { None };
    let from_cache = if m.solid().is_some() { Some(block_member(state, actor, a, &m, password.clone()).await?) } else { None };
    let permit = state.archive.workers.clone().acquire_owned().await.map_err(|e| internal(e.to_string()))?;
    let (index, mm, pw) = (a.index.clone(), m.clone(), password.as_ref().map(|p| p.0.clone()));
    let done = tokio::task::spawn_blocking(move || -> NfsResult<bool> {
        use std::io::Write;
        let _permit = permit;
        let mut w = std::io::BufWriter::new(out);
        let mut sink = |chunk: &[u8]| {
            progress.fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
            w.write_all(chunk).map(|_| true)
        };
        let ok = match &from_cache {
            Some(path) => source::pump(std::fs::File::open(path)?, mm.size, None, &cancel, &mut sink)?,
            None => format::decode(&index, &mm, pw.as_deref(), None, &cancel, &mut sink)?,
        };
        w.into_inner().map_err(|e| NfsError::from(e.into_error()))?.sync_all()?;
        Ok(ok)
    })
    .await
    .map_err(|e| internal(format!("copy task: {e}")))?;
    if let (Some(pw), None) = (&password, m.solid()) {
        settle(state, &user_key(actor), a, pw, matches!(done, Ok(true))).await;
    }
    match done? {
        true => Ok(()),
        false => Err(internal("copy cancellation requested")),
    }
}

pub async fn maintenance_loop(state: SharedState) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        state.archive.sweep();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn desktop_knows_every_extension_hello_lists() {
        let urls = include_str!("../../../desktop/src/app/filebrowser/data/urls.ts");
        let start = urls.find("export const ARCHIVE_EXTENSIONS = [").expect("Desktop's ARCHIVE_EXTENSIONS");
        let list = &urls[start..start + urls[start..].find(']').unwrap()];
        let mut desktop: Vec<&str> = list.split('\'').skip(1).step_by(2).collect();
        let (tar_ext, single_ext) = super::stream_extensions();
        let mut server: Vec<&str> = ["zip", "7z"].into_iter().chain(tar_ext).chain(single_ext).collect();
        desktop.sort();
        server.sort();
        assert_eq!(desktop, server);
    }
}
