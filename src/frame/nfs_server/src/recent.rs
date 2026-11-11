//! The built-in `view://recent`: the newest files in the caller's own home,
//! computed on every list (product/bucky_file/nfs_server.md §4.1). The view row is seeded once
//! so it has a stable node like any other View; its members are not stored.

use crate::acl::{Actor, Level, HOME_ROOT};
use crate::error::*;
use crate::filedb::FileDb;
use crate::fsutil::*;
use crate::handle::native_entry_ref;
use crate::namespace::{canonical_path, EntryRow, Node};
use crate::state::AppState;
use crate::types::{WantMask, WireRef};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const RECENT_VIEW_ID: &str = "recent";
pub const RECENT_LIMIT: usize = 50;
/// Entries looked at per list; past it the newest of what was seen is returned.
const MAX_VISITED: usize = 100_000;
/// Top-level home folders that hold system state rather than the user's files.
const SKIPPED_HOME_DIRS: &[&str] = &["agents"];

pub fn ensure_builtin_views(db: &FileDb) -> NfsResult<()> {
    if db.view_by_id(RECENT_VIEW_ID)?.is_none() {
        db.view_create(RECENT_VIEW_ID, "Recent", "system")?;
    }
    Ok(())
}

pub fn is_recent(node: &Node) -> bool {
    matches!(node, Node::View(v) if v.view_id == RECENT_VIEW_ID)
}

/// Changes whenever the members or their content change, so cached listings revalidate.
pub fn recent_revision(rows: &[EntryRow]) -> String {
    let mut h = Sha256::new();
    for r in rows {
        h.update(r.json["canonical_path"].as_str().unwrap_or("").as_bytes());
        h.update([0]);
        h.update(r.mtime.to_le_bytes());
        h.update(r.size.to_le_bytes());
    }
    format!("r-{}", hex::encode(&h.finalize()[..12]))
}

impl AppState {
    pub(crate) fn recent_entries(&self, actor: &Actor, want: &WantMask) -> NfsResult<Vec<EntryRow>> {
        let (Some(user), Some(cfg)) = (actor.user_id(), self.config.root(HOME_ROOT)) else {
            return Ok(Vec::new());
        };
        let home_rel = user.to_string();
        if !join_root(&cfg.path, &home_rel).is_dir() {
            return Ok(Vec::new());
        }
        // Newest first by when the file was last written or put here: a copy keeps its old mtime.
        let mut files: Vec<(i64, String, NodeMeta)> = Vec::new();
        let mut visited = 0usize;
        let mut dirs = vec![home_rel.clone()];
        'walk: while let Some(rel) = dirs.pop() {
            let Ok(rd) = std::fs::read_dir(join_root(&cfg.path, &rel)) else { continue };
            for dent in rd.flatten() {
                visited += 1;
                if visited > MAX_VISITED {
                    break 'walk;
                }
                let Ok(name) = dent.file_name().into_string() else { continue };
                if name.starts_with('.') || validate_name(&name).is_err() {
                    continue;
                }
                if rel == home_rel && SKIPPED_HOME_DIRS.contains(&name.as_str()) {
                    continue;
                }
                let child_rel = rel_join(&rel, &name);
                let Ok(meta) = lstat(&dent.path()) else { continue };
                let needed = if meta.kind == "dir" { Level::Traverse } else { Level::Read };
                if self.acls.level(actor, HOME_ROOT, &child_rel) < needed {
                    continue;
                }
                match meta.kind {
                    "dir" => dirs.push(child_rel),
                    "file" => files.push((meta.mtime.max(meta.ctime), child_rel, meta)),
                    _ => {}
                }
            }
        }
        files.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        files.truncate(RECENT_LIMIT);
        let picked = files;

        let mut rows = Vec::with_capacity(picked.len());
        for (idx, (_, rel, meta)) in picked.iter().enumerate() {
            let (parent, name) = rel_split(rel).unwrap_or_default();
            let node = Node::Native { root: HOME_ROOT.to_string(), rel: rel.clone(), meta: meta.clone(), anchored: None };
            let mut target = self.target_json(actor, &node, want)?;
            if let Some(attrs) = target["attrs"].as_object_mut() {
                attrs.remove("_ino");
            }
            let entry_ref = native_entry_ref(HOME_ROOT, &parent, &name, meta.id.ino);
            rows.push(EntryRow {
                name: name.clone(),
                kind: "file".to_string(),
                mtime: meta.mtime,
                size: meta.size,
                order_index: idx as i64,
                json: json!({
                    "entry_ref": format!("vr_{}", &entry_ref[3..]),
                    "name": name,
                    "binding": "derived",
                    "target": target,
                    "context": {"type": "view"},
                    "canonical_path": canonical_path(HOME_ROOT, rel),
                }),
            });
        }
        if want.has("ident") {
            let rels: Vec<String> = picked.iter().map(|(_, rel, _)| rel.clone()).collect();
            for a in self.db.anchors_by_paths(HOME_ROOT, &rels)? {
                let Some(i) = rels.iter().position(|rel| *rel == a.path) else { continue };
                if a.state != "ok" || picked[i].2.id.ino != a.ino {
                    continue;
                }
                if let Some(e) = self.db.entity_get(a.node_id)? {
                    let target: &mut Value = &mut rows[i].json["target"];
                    target["ref"] = json!(WireRef::live(format!("n_{}", e.node_id), e.gen));
                    if let Some(h) = &a.full_hash {
                        target["attrs"]["obj_id"] = json!(format!("sha256:{}", h));
                    }
                }
            }
        }
        Ok(rows)
    }
}
