//! Content snapshots (design §3.4): a readable file or directory becomes an
//! immutable FileObject / DirObject whose bytes live in the named store
//! (store mode — never links to the mutable source file).
//!
//! Planning walks the source once (ACL per entry, default exclusions, no
//! symlink following inside directories); storing re-reads every planned file
//! in 32 MiB ranges, hashing and storing each range from the same buffer, and
//! reports `source_changed` when a file differs from the plan.

use crate::acl::{Actor, Level};
use crate::error::*;
use crate::fsutil::{content_type_for, join_root};
use crate::namespace::{canonical_path, Node};
use crate::share::db::reason;
use crate::state::AppState;
use crate::types::Locator;
use named_store::NamedDataMgr;
use ndn_lib::{
    load_named_obj, ChunkHasher, ChunkId, ChunkList, DirObject, FileObject, NamedObject, ObjId, SimpleMapItem,
    CHUNK_DEFAULT_SIZE, OBJ_TYPE_DIR, OBJ_TYPE_FILE,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum PlanNode {
    File { name: String, disk: PathBuf, size: u64, mtime: i64 },
    Dir { name: String, children: Vec<PlanNode> },
}

#[derive(Debug, Clone)]
pub struct SourcePlan {
    /// `cyfs:///<root>/<rel>` of the source (private, never put in objects).
    pub source_path: String,
    pub name: String,
    pub kind: &'static str,
    pub mime: Option<String>,
    pub root: PlanNode,
    pub total_size: u64,
    pub item_count: u64,
    pub excluded: Vec<Value>,
    pub mtime: i64,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct SnapshotResult {
    pub obj_id: String,
    pub obj_type: &'static str,
    pub total_size: u64,
    pub item_count: u64,
    pub source_changed: bool,
}

/// Progress shared with the operation runner.
#[derive(Default)]
pub struct Progress {
    pub done_bytes: AtomicU64,
    pub cancel: AtomicBool,
}

const EXCLUDED_DIRS: &[&str] = &[".git", "node_modules"];

/// Default exclusions for directory shares (`share.plan` reports each one).
fn excluded_reason(name: &str, is_dir: bool) -> Option<&'static str> {
    if is_dir {
        return EXCLUDED_DIRS.contains(&name).then_some("excluded_by_default");
    }
    let lower = name.to_ascii_lowercase();
    if lower == ".ds_store" || lower.starts_with(".nfs_tmp_") {
        return Some("excluded_by_default");
    }
    if lower.starts_with(".env") || lower.ends_with(".key") || lower.ends_with(".pem") {
        return Some("secret");
    }
    None
}

pub fn base_mime(name: &str) -> String {
    content_type_for(name).split(';').next().unwrap_or("application/octet-stream").trim().to_string()
}

fn mtime_of(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

struct Walk<'a> {
    state: &'a AppState,
    actor: &'a Actor,
    root: &'a str,
    total: u64,
    items: u64,
    max_mtime: i64,
    excluded: Vec<Value>,
}

impl Walk<'_> {
    fn check_limits(&self) -> NfsResult<()> {
        let cfg = &self.state.config.share;
        if self.total > cfg.max_total_size {
            return Err(reason(ErrorCode::QuotaExceeded, "too_large", format!("share exceeds {} bytes", cfg.max_total_size)));
        }
        if self.items > cfg.max_items {
            return Err(reason(ErrorCode::QuotaExceeded, "too_many_items", format!("share exceeds {} items", cfg.max_items)));
        }
        Ok(())
    }

    fn dir(&mut self, disk: &std::path::Path, rel: &str, inner: &str, name: String) -> NfsResult<PlanNode> {
        let mut entries: Vec<(String, std::fs::FileType)> = Vec::new();
        for entry in std::fs::read_dir(disk)? {
            let entry = entry?;
            let Ok(n) = entry.file_name().into_string() else {
                self.excluded.push(json!({"path": format!("{inner}{}", entry.file_name().to_string_lossy()), "reason": "bad_name"}));
                continue;
            };
            entries.push((n, entry.file_type()?));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut children = Vec::new();
        for (n, ft) in entries {
            let child_rel = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
            let child_inner = format!("{inner}{n}");
            if ft.is_symlink() {
                self.excluded.push(json!({"path": child_inner, "reason": "symlink"}));
                continue;
            }
            if !ft.is_dir() && !ft.is_file() {
                self.excluded.push(json!({"path": child_inner, "reason": "special_file"}));
                continue;
            }
            if let Some(why) = excluded_reason(&n, ft.is_dir()) {
                self.excluded.push(json!({"path": if ft.is_dir() { format!("{child_inner}/") } else { child_inner }, "reason": why}));
                continue;
            }
            if self.state.acls.level(self.actor, self.root, &child_rel) < Level::Read {
                self.excluded.push(json!({"path": child_inner, "reason": "no_permission"}));
                continue;
            }
            let path = disk.join(&n);
            if ft.is_dir() {
                let node = self.dir(&path, &child_rel, &format!("{child_inner}/"), n)?;
                children.push(node);
            } else {
                let m = std::fs::metadata(&path)?;
                self.total += m.len();
                self.items += 1;
                self.max_mtime = self.max_mtime.max(mtime_of(&m));
                self.check_limits()?;
                children.push(PlanNode::File { name: n, disk: path, size: m.len(), mtime: mtime_of(&m) });
            }
        }
        Ok(PlanNode::Dir { name, children })
    }
}

impl AppState {
    /// Resolves and walks a share source (Read access required), without
    /// touching the named store.
    pub fn share_plan_source(&self, actor: &Actor, source: &Locator) -> NfsResult<SourcePlan> {
        let node = self.resolve_for(actor, source, Level::Read, "read").map_err(|e| {
            if e.code == ErrorCode::PermissionDenied {
                e.with("reason", json!("source_not_readable"))
            } else {
                e
            }
        })?;
        let Node::Native { root, rel, meta, .. } = &node else {
            return Err(reason(ErrorCode::Unsupported, "source_unsupported", "only files and folders can be shared"));
        };
        let cfg = self.config.root(root).ok_or_else(|| stale("root gone"))?;
        let disk = join_root(&cfg.path, rel);
        let name = node.display_name();
        let source_path = canonical_path(root, rel);
        let mut walk = Walk { state: self, actor, root, total: 0, items: 0, max_mtime: 0, excluded: Vec::new() };
        let (kind, plan_root, mime) = match meta.kind {
            "dir" => {
                if rel.is_empty() && !actor.is_admin() {
                    return Err(reason(ErrorCode::Unsupported, "source_unsupported", "an export root cannot be shared"));
                }
                let node = walk.dir(&disk, rel, "", name.clone())?;
                ("dir", node, None)
            }
            "file" | "symlink" => {
                if meta.kind == "symlink" {
                    self.require_symlink_target(actor, &disk)?;
                }
                let m = std::fs::metadata(&disk)?;
                if !m.is_file() {
                    return Err(reason(ErrorCode::Unsupported, "source_unsupported", "a link to a folder cannot be shared"));
                }
                walk.total = m.len();
                walk.items = 1;
                walk.max_mtime = mtime_of(&m);
                walk.check_limits()?;
                let file = PlanNode::File { name: name.clone(), disk: disk.clone(), size: m.len(), mtime: mtime_of(&m) };
                ("file", file, Some(base_mime(&name)))
            }
            _ => return Err(reason(ErrorCode::Unsupported, "source_unsupported", "only files and folders can be shared")),
        };
        let fingerprint = {
            let body = json!([source_path, kind, walk.total, walk.items, walk.max_mtime, walk.excluded.len()]).to_string();
            hex::encode(&Sha256::digest(body.as_bytes())[..12])
        };
        Ok(SourcePlan {
            source_path,
            name,
            kind,
            mime,
            root: plan_root,
            total_size: walk.total,
            item_count: walk.items,
            excluded: walk.excluded,
            mtime: walk.max_mtime,
            fingerprint,
        })
    }
}

fn ndn_err(e: impl std::fmt::Display) -> NfsError {
    internal(format!("named store: {e}"))
}

/// Stores one file in 32 MiB ranges (a single chunk up to 32 MiB, as HomeStation
/// attachments require; a chunk list beyond). Returns the FileObject and
/// whether the file no longer matches the plan.
async fn store_file(
    store: &NamedDataMgr,
    disk: &std::path::Path,
    name: &str,
    planned: (u64, i64),
    progress: &Progress,
) -> NfsResult<(FileObject, ObjId, bool)> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(disk).await?;
    let before = file.metadata().await?;
    let mut changed = (before.len(), mtime_of(&before)) != planned;
    let mut chunk_ids = Vec::new();
    let mut size = 0u64;
    loop {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(reason(ErrorCode::Internal, "canceled", "snapshot canceled"));
        }
        let mut buf = Vec::with_capacity(CHUNK_DEFAULT_SIZE.min(before.len().max(1)) as usize);
        let n = (&mut file).take(CHUNK_DEFAULT_SIZE).read_to_end(&mut buf).await?;
        if n == 0 && !chunk_ids.is_empty() {
            break;
        }
        let (chunk_id, buf) = tokio::task::spawn_blocking(move || -> NfsResult<(ChunkId, Vec<u8>)> {
            let hasher = ChunkHasher::new(None).map_err(ndn_err)?;
            let method = hasher.hash_method;
            let raw = hasher.calc_from_bytes(&buf);
            let id = ChunkId::from_mix_hash_result_by_hash_method(buf.len() as u64, &raw, method).map_err(ndn_err)?;
            Ok((id, buf))
        })
        .await
        .map_err(|e| internal(format!("hash task: {e}")))??;
        if !store.have_chunk(&chunk_id).await {
            store.put_chunk(&chunk_id, &buf).await.map_err(ndn_err)?;
        }
        chunk_ids.push(chunk_id);
        size += n as u64;
        progress.done_bytes.fetch_add(n as u64, Ordering::Relaxed);
        if (n as u64) < CHUNK_DEFAULT_SIZE {
            break;
        }
    }
    let after = tokio::fs::metadata(disk).await?;
    changed |= (after.len(), mtime_of(&after)) != (before.len(), mtime_of(&before)) || size != before.len();
    let content = if chunk_ids.len() > 1 {
        let list = ChunkList::from_chunk_list(chunk_ids).map_err(ndn_err)?;
        let (list_id, list_str) = list.gen_obj_id();
        store.put_object(&list_id, &list_str).await.map_err(ndn_err)?;
        list_id.to_string()
    } else {
        chunk_ids[0].to_string()
    };
    let mut obj = FileObject::new(name.to_string(), size, content);
    obj.meta.insert("mime".into(), json!(base_mime(name)));
    let (obj_id, obj_str) = obj.gen_obj_id();
    store.put_object(&obj_id, &obj_str).await.map_err(ndn_err)?;
    Ok((obj, obj_id, changed))
}

fn store_dir<'a>(
    store: &'a NamedDataMgr,
    name: &'a str,
    children: &'a [PlanNode],
    progress: &'a Progress,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = NfsResult<(DirObject, ObjId, bool)>> + Send + 'a>> {
    Box::pin(async move {
        let mut dir = DirObject::new(Some(name.to_string()));
        let mut changed = false;
        for child in children {
            match child {
                PlanNode::File { name, disk, size, mtime } => {
                    let (obj, _, c) = store_file(store, disk, name, (*size, *mtime), progress).await?;
                    changed |= c;
                    let size = obj.size;
                    dir.add_file(name.clone(), serde_json::to_value(&obj)?, size).map_err(ndn_err)?;
                }
                PlanNode::Dir { name, children } => {
                    let (sub, sub_id, c) = store_dir(store, name, children, progress).await?;
                    changed |= c;
                    dir.add_directory(name.clone(), sub_id, sub.total_size).map_err(ndn_err)?;
                }
            }
        }
        let (dir_id, _) = dir.gen_obj_id().map_err(ndn_err)?;
        let body = serde_json::to_string(&dir)?;
        store.put_object(&dir_id, &body).await.map_err(ndn_err)?;
        Ok((dir, dir_id, changed))
    })
}

/// Stores the planned content and returns the root object.
pub async fn run_snapshot(store: &NamedDataMgr, plan: &SourcePlan, progress: &Progress) -> NfsResult<SnapshotResult> {
    match &plan.root {
        PlanNode::File { name, disk, size, mtime } => {
            let (obj, id, changed) = store_file(store, disk, name, (*size, *mtime), progress).await?;
            Ok(SnapshotResult {
                obj_id: id.to_string(),
                obj_type: OBJ_TYPE_FILE,
                total_size: obj.size,
                item_count: 1,
                source_changed: changed,
            })
        }
        PlanNode::Dir { name, children } => {
            let (dir, id, changed) = store_dir(store, name, children, progress).await?;
            Ok(SnapshotResult {
                obj_id: id.to_string(),
                obj_type: OBJ_TYPE_DIR,
                total_size: dir.total_size,
                item_count: plan.item_count,
                source_changed: changed,
            })
        }
    }
}

// ---------- reading snapshots back (data plane) ----------

pub enum Resolved {
    File(FileObject),
    Dir(DirObject),
}

fn item_to_resolved(item: &SimpleMapItem) -> NfsResult<(ObjId, Option<Resolved>)> {
    match item {
        SimpleMapItem::ObjId(id) => Ok((id.clone(), None)),
        SimpleMapItem::Object(t, v) if t == OBJ_TYPE_FILE => {
            let obj: FileObject = serde_json::from_value(v.clone())?;
            let (id, _) = item.get_obj_id().map_err(ndn_err)?;
            Ok((id, Some(Resolved::File(obj))))
        }
        _ => Err(not_found("unsupported directory item")),
    }
}

async fn load(store: &NamedDataMgr, id: &ObjId) -> NfsResult<Resolved> {
    let body = store.get_object(id).await.map_err(|e| not_found(format!("snapshot object unavailable: {e}")))?;
    if id.is_dir_object() {
        Ok(Resolved::Dir(load_named_obj(&body).map_err(ndn_err)?))
    } else if id.is_file_object() {
        Ok(Resolved::File(load_named_obj(&body).map_err(ndn_err)?))
    } else {
        Err(not_found("unsupported snapshot object"))
    }
}

/// Walks `segments` (already percent-decoded names) from the share root.
pub async fn resolve(store: &NamedDataMgr, root: &str, segments: &[String]) -> NfsResult<Resolved> {
    let root_id = ObjId::new(root).map_err(ndn_err)?;
    let mut cur = load(store, &root_id).await?;
    for seg in segments {
        let Resolved::Dir(dir) = &cur else {
            return Err(not_found("path not found"));
        };
        let item = dir.get(seg).ok_or_else(|| not_found("path not found"))?;
        let (id, embedded) = item_to_resolved(item)?;
        cur = match embedded {
            Some(r) => r,
            None => load(store, &id).await?,
        };
    }
    Ok(cur)
}

/// Directory listing for the visitor page: name, kind, size, mime.
pub async fn list_dir(store: &NamedDataMgr, dir: &DirObject) -> Vec<Value> {
    let mut out = Vec::new();
    for (name, item) in dir.iter() {
        match item_to_resolved(item) {
            Ok((_, Some(Resolved::File(f)))) => {
                let mime = f.meta.get("mime").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| base_mime(name));
                out.push(json!({"name": name, "kind": "file", "size": f.size, "mime": mime}));
            }
            Ok((id, None)) if id.is_dir_object() => {
                let size = match load(store, &id).await {
                    Ok(Resolved::Dir(d)) => d.total_size,
                    _ => 0,
                };
                out.push(json!({"name": name, "kind": "dir", "size": size}));
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| {
        (a["kind"] != "dir", a["name"].as_str().unwrap_or("")).cmp(&(b["kind"] != "dir", b["name"].as_str().unwrap_or("")))
    });
    out
}

/// A reader positioned at `offset` of a FileObject's content.
pub async fn open_content(store: &NamedDataMgr, file: &FileObject, offset: u64) -> NfsResult<ndn_lib::ChunkReader> {
    let content = ObjId::new(&file.content).map_err(ndn_err)?;
    let (reader, _) = if content.is_chunk_list() {
        store.open_chunklist_reader(&content, offset).await
    } else {
        store.open_chunk_reader(&ChunkId::from_obj_id(&content), offset).await
    }
    .map_err(|e| not_found(format!("snapshot content unavailable: {e}")))?;
    Ok(reader)
}

/// Whether the object and everything it references is still in the store.
pub async fn is_stored(store: &NamedDataMgr, obj_id: &str) -> bool {
    match ObjId::new(obj_id) {
        Ok(id) => store.is_object_stored(&id, None).await.unwrap_or(false),
        Err(_) => false,
    }
}

pub fn shared_progress() -> Arc<Progress> {
    Arc::new(Progress::default())
}
