//! Write operations (NFSP §5.3/§5.5): mkdir, move, delete, bind_ref, unlink,
//! open_write / commit_file, probe, meta, grant/revoke.
//!
//! All container structure writes take an optional opaque `expected_revision`
//! (CAS). File content writes go through the lease manager. Bypass writers
//! (server-local processes) are detected at commit via the (size, mtime)
//! snapshot taken at open_write (nfs_server.md §4.2: 劝告性租约, 显式冲突).

use crate::error::*;
use crate::fsutil::*;
use crate::handle::native_entry_ref;
use crate::namespace::{canonical_path, Node};
use crate::state::AppState;
use crate::types::{Locator, WireRef};
use crate::watch::ContainerKey;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

impl AppState {
    // ---------- helpers ----------

    /// Resolves the parent directory for a write op from `at` or args.parent_ref.
    fn parent_dir(&self, at: Option<&Locator>, args: &Value) -> NfsResult<Node> {
        let node = if args.get("parent_ref").map(|v| !v.is_null()).unwrap_or(false) {
            let r: WireRef = serde_json::from_value(args["parent_ref"].clone())
                .map_err(|_| invalid("bad parent_ref"))?;
            self.resolve_ref(&r)?
        } else if let Some(at) = at {
            self.resolve_locator(at)?
        } else {
            return Err(invalid("missing parent (at or parent_ref)"));
        };
        match &node {
            Node::Root => Err(NfsError::new(
                ErrorCode::PermissionDenied,
                "the namespace root is read-only (export roots are static config)",
            )),
            Node::Native { meta, .. } if meta.kind == "dir" => Ok(node),
            Node::Native { .. } => Err(invalid("parent is not a directory")),
            _ => Err(invalid("parent must be a native directory")),
        }
    }

    fn dir_parts(&self, node: &Node) -> NfsResult<(String, String, PathBuf)> {
        match node {
            Node::Native { root, rel, .. } => {
                let cfg = self.config.root(root).ok_or_else(|| stale("root gone"))?;
                Ok((root.clone(), rel.clone(), join_root(&cfg.path, rel)))
            }
            _ => Err(invalid("not a native node")),
        }
    }

    fn check_dir_cas(&self, key: &ContainerKey, expected: Option<&str>) -> NfsResult<()> {
        if let Some(exp) = expected {
            let cur = self.revisions.current(key);
            if exp != cur {
                return Err(rev_mismatch(exp, &cur));
            }
        }
        Ok(())
    }

    /// Bumps a dir's revision and emits container_changed. Returns the revision.
    fn bump_dir(&self, node: &Node, reason: &str, hint: Option<Value>) -> String {
        let key = match node.container_key() {
            Some(k) => k,
            None => return String::new(),
        };
        let revision = self.revisions.bump(&key);
        self.bus.emit_container_changed(
            &key,
            json!(self.node_ref(node)),
            "dir",
            &revision,
            reason,
            hint,
        );
        revision
    }

    /// Checks that `name` does not collide with a virtual binding in this dir.
    fn check_no_binding(&self, parent: &Node, name: &str) -> NfsResult<()> {
        if let Node::Native { anchored: Some(e), .. } = parent {
            for b in self.db.bindings_by_parent(e.node_id)? {
                if b.name == name {
                    return Err(NfsError::new(
                        ErrorCode::NamespaceConflict,
                        format!("'{}' is a reference entry; unlink it first", name),
                    ));
                }
            }
        }
        Ok(())
    }

    // ---------- mkdir ----------

    pub fn op_mkdir(&self, at: Option<&Locator>, args: &Value) -> NfsResult<Value> {
        // Path form: mkdir -p along the whole path (idempotent, no CAS).
        let has_path = at.map(|l| l.r#ref.is_none() && (l.path.is_some() || l.uri.is_some()))
            .unwrap_or(false);
        if has_path && args.get("name").is_none() {
            return self.mkdir_path(at.unwrap());
        }
        // Ref form: single child under parent, optional CAS.
        let parent = self.parent_dir(at, args)?;
        let name = args["name"].as_str().ok_or_else(|| invalid("mkdir requires name"))?;
        validate_name(name)?;
        self.check_dir_cas(
            &parent.container_key().unwrap(),
            args["expected_revision"].as_str(),
        )?;
        self.check_no_binding(&parent, name)?;
        let (root, rel, full) = self.dir_parts(&parent)?;
        let target = full.join(name);
        let child_rel = rel_join(&rel, name);
        let existed = match std::fs::symlink_metadata(&target) {
            Ok(m) if m.is_dir() => true,
            Ok(_) => {
                return Err(NfsError::new(
                    ErrorCode::NamespaceConflict,
                    format!("a non-directory named '{}' exists", name),
                ))
            }
            Err(_) => {
                std::fs::create_dir(&target)?;
                false
            }
        };
        let revision = if existed {
            self.revisions.current(&parent.container_key().unwrap())
        } else {
            self.bump_dir(&parent, "entries_changed", Some(json!({"added":[name]})))
        };
        let node = self.resolve_dfs_path(&format!("{}/{}", root, child_rel))?;
        Ok(json!({"ref": self.node_ref(&node), "existed": existed, "revision": revision}))
    }

    fn mkdir_path(&self, at: &Locator) -> NfsResult<Value> {
        let path = at
            .path
            .clone()
            .or_else(|| at.uri.as_ref()?.strip_prefix("dfs://").map(|s| s.to_string()))
            .ok_or_else(|| invalid("mkdir path form requires a dfs path"))?;
        let rel_all = normalize_rel(&path)?;
        let (root_id, rel) = match rel_all.split_once('/') {
            Some((r, rest)) => (r.to_string(), rest.to_string()),
            None if !rel_all.is_empty() => (rel_all.clone(), String::new()),
            _ => return Err(invalid("cannot mkdir the namespace root")),
        };
        let root = self
            .config
            .root(&root_id)
            .ok_or_else(|| not_found(format!("export root '{}' not found", root_id)))?;
        let mut existed = true;
        if !rel.is_empty() {
            let mut cur = String::new();
            for seg in rel.split('/') {
                let next = rel_join(&cur, seg);
                let p = join_root(&root.path, &next);
                match std::fs::symlink_metadata(&p) {
                    Ok(m) if m.is_dir() => {}
                    Ok(_) => {
                        return Err(NfsError::new(
                            ErrorCode::NamespaceConflict,
                            format!("a non-directory exists at '{}'", next),
                        ))
                    }
                    Err(_) => {
                        std::fs::create_dir(&p)?;
                        existed = false;
                        let parent = self.resolve_dfs_path(&format!(
                            "{}/{}",
                            root_id,
                            cur
                        ))?;
                        self.bump_dir(&parent, "entries_changed", Some(json!({"added":[seg]})));
                    }
                }
                cur = next;
            }
        }
        let node = self.resolve_dfs_path(&rel_all)?;
        Ok(json!({"ref": self.node_ref(&node), "existed": existed}))
    }

    // ---------- move ----------

    pub fn op_move(&self, args: &Value) -> NfsResult<Value> {
        let from = args.get("from").ok_or_else(|| invalid("move requires from"))?;
        let to = args.get("to").ok_or_else(|| invalid("move requires to"))?;
        let from_parent = self.parent_dir(None, from)?;
        let to_parent = self.parent_dir(None, to)?;
        let from_name =
            from["name"].as_str().ok_or_else(|| invalid("move.from requires name"))?;
        let to_name = to["name"].as_str().ok_or_else(|| invalid("move.to requires name"))?;
        validate_name(to_name)?;
        // 跨目录双 revision CAS (Ops_v3 §3.5).
        self.check_dir_cas(
            &from_parent.container_key().unwrap(),
            args["expected_from_revision"].as_str(),
        )?;
        if from_parent.container_key() != to_parent.container_key() {
            self.check_dir_cas(
                &to_parent.container_key().unwrap(),
                args["expected_to_revision"].as_str(),
            )?;
        }
        let (from_root, from_rel, from_full) = self.dir_parts(&from_parent)?;
        let (to_root, to_rel, to_full) = self.dir_parts(&to_parent)?;
        let src_rel = rel_join(&from_rel, from_name);
        let dst_rel = rel_join(&to_rel, to_name);
        if from_root == to_root && src_rel == dst_rel {
            return Err(invalid("source and destination are the same"));
        }
        let src = from_full.join(from_name);
        let dst = to_full.join(to_name);
        let src_meta = std::fs::symlink_metadata(&src)
            .map_err(|_| not_found(format!("'{}' not found", from_name)))?;
        // Moving a dir into its own subtree is a cycle in the strict tree.
        if src_meta.is_dir()
            && from_root == to_root
            && (dst_rel == src_rel || dst_rel.starts_with(&format!("{}/", src_rel)))
        {
            return Err(invalid("cannot move a directory into itself"));
        }
        if std::fs::symlink_metadata(&dst).is_ok() {
            return Err(NfsError::new(
                ErrorCode::TargetMismatch,
                format!("destination '{}' already exists", to_name),
            ));
        }
        self.check_no_binding(&to_parent, to_name)?;
        std::fs::rename(&src, &dst).map_err(|e| {
            if e.raw_os_error() == Some(18) {
                // EXDEV
                internal("cross-device move is not supported in v1")
            } else {
                NfsError::from(e).ctx("move")
            }
        })?;
        // Anchors and the content cache follow the move (meta/LiveRefs survive).
        self.db.anchors_move_prefix(&from_root, &src_rel, &to_root, &dst_rel)?;
        self.db.content_index_move_prefix(&from_root, &src_rel, &to_root, &dst_rel)?;
        let from_revision =
            self.bump_dir(&from_parent, "entries_changed", Some(json!({"removed":[from_name]})));
        let to_revision = if from_parent.container_key() == to_parent.container_key() {
            from_revision.clone()
        } else {
            self.bump_dir(&to_parent, "entries_changed", Some(json!({"added":[to_name]})))
        };
        Ok(json!({"from_revision": from_revision, "to_revision": to_revision}))
    }

    // ---------- delete / unlink ----------

    pub fn op_delete(&self, at: Option<&Locator>, args: &Value) -> NfsResult<Value> {
        let parent = self.parent_dir(at, args)?;
        let name = args["name"].as_str().ok_or_else(|| invalid("delete requires name"))?;
        validate_name(name)?;
        let recursive = args["recursive"].as_bool().unwrap_or(false);
        self.check_dir_cas(
            &parent.container_key().unwrap(),
            args["expected_revision"].as_str(),
        )?;
        let (root, rel, full) = self.dir_parts(&parent)?;
        let target = full.join(name);
        let target_rel = rel_join(&rel, name);
        let meta = match std::fs::symlink_metadata(&target) {
            Ok(m) => m,
            Err(_) => {
                // Not a native entry: a reference entry must use unlink (§5.3).
                if let Node::Native { anchored: Some(e), .. } = &parent {
                    if self
                        .db
                        .bindings_by_parent(e.node_id)?
                        .iter()
                        .any(|b| b.name == name)
                    {
                        return Err(invalid(format!(
                            "'{}' is a reference entry; use unlink, not delete",
                            name
                        )));
                    }
                }
                return Err(not_found(format!("'{}' not found", name)));
            }
        };
        if meta.is_dir() {
            if !recursive {
                let empty = std::fs::read_dir(&target)?.next().is_none();
                if !empty {
                    return Err(NfsError::new(
                        ErrorCode::NotEmpty,
                        format!("directory '{}' is not empty (pass recursive:true)", name),
                    ));
                }
                std::fs::remove_dir(&target)?;
            } else {
                std::fs::remove_dir_all(&target)?;
            }
        } else {
            std::fs::remove_file(&target)?;
        }
        // Destroyed natives: anchors marked deleted, inbound references go stale.
        self.db.anchors_delete_subtree(&root, &target_rel)?;
        let revision =
            self.bump_dir(&parent, "entries_changed", Some(json!({"removed":[name]})));
        Ok(json!({ "revision": revision }))
    }

    pub fn op_bind_ref(&self, args: &Value) -> NfsResult<Value> {
        let parent = self.parent_dir(None, args)?;
        let name = args["name"].as_str().ok_or_else(|| invalid("bind_ref requires name"))?;
        validate_name(name)?;
        let target_ref: WireRef = serde_json::from_value(args["target_ref"].clone())
            .map_err(|_| invalid("bind_ref requires target_ref"))?;
        self.check_dir_cas(
            &parent.container_key().unwrap(),
            args["expected_revision"].as_str(),
        )?;
        let (root, rel, full) = self.dir_parts(&parent)?;
        if std::fs::symlink_metadata(full.join(name)).is_ok() {
            return Err(NfsError::new(
                ErrorCode::NamespaceConflict,
                format!("a native entry named '{}' exists", name),
            ));
        }
        // Anchor the parent dir (lazy anchoring trigger: binding parent).
        let parent_meta = lstat(&full)?;
        let parent_ent =
            self.db
                .ensure_anchor("dir", &root, &rel, parent_meta.id.ino, parent_meta.id.dev, 0, parent_meta.mtime)?;
        let entry_id = match &target_ref {
            WireRef::Object { obj_id, .. } => {
                self.db
                    .binding_create(parent_ent.node_id, name, "object", None, Some(obj_id))?
            }
            live => {
                let target = self.resolve_ref(live)?;
                let (target_id, _) = self.require_stable_node(&target)?;
                self.db
                    .binding_create(parent_ent.node_id, name, "live", Some(target_id), None)?
            }
        };
        // Re-resolve the parent so the bump sees its anchored identity.
        let parent = self.resolve_dfs_path(&format!("{}/{}", root, rel))?;
        let revision =
            self.bump_dir(&parent, "entries_changed", Some(json!({"added":[name]})));
        Ok(json!({"entry_ref": format!("be_{}", entry_id), "revision": revision}))
    }

    pub fn op_unlink(&self, args: &Value) -> NfsResult<Value> {
        let entry_ref = args["entry_ref"]
            .as_str()
            .ok_or_else(|| invalid("unlink requires entry_ref"))?;
        if entry_ref.starts_with("ce_") {
            return Err(invalid("collection entries are unlinked via collection_patch"));
        }
        if entry_ref.starts_with("ne_") {
            return Err(invalid("native entries are destroyed via delete, not unlink"));
        }
        let id: i64 = entry_ref
            .strip_prefix("be_")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| invalid(format!("'{}' is not a binding entry_ref", entry_ref)))?;
        let b = self
            .db
            .binding_get(id)?
            .ok_or_else(|| stale(format!("entry '{}' no longer exists", entry_ref)))?;
        let parent_anchor = self
            .db
            .anchor_get(b.parent_node_id)?
            .ok_or_else(|| internal("binding parent has no anchor"))?;
        let parent_key = ContainerKey::Dir {
            root: parent_anchor.root_id.clone(),
            rel: parent_anchor.path.clone(),
        };
        self.check_dir_cas(&parent_key, args["expected_revision"].as_str())?;
        // Unlink removes only the entry, never the target (NFSP §3.1.1).
        self.db.binding_delete(id)?;
        let revision = self.revisions.bump(&parent_key);
        if parent_anchor.state == "ok" {
            if let Ok(parent) = self
                .resolve_dfs_path(&format!("{}/{}", parent_anchor.root_id, parent_anchor.path))
            {
                self.bus.emit_container_changed(
                    &parent_key,
                    json!(self.node_ref(&parent)),
                    "dir",
                    &revision,
                    "entries_changed",
                    Some(json!({"removed":[b.name]})),
                );
            }
        }
        Ok(json!({ "revision": revision }))
    }

    // ---------- open_write / commit_file ----------

    pub fn op_open_write(&self, args: &Value, session: &str) -> NfsResult<Value> {
        let (root, rel, base) = if args.get("ref").map(|v| !v.is_null()).unwrap_or(false) {
            let r: WireRef = serde_json::from_value(args["ref"].clone())
                .map_err(|_| invalid("bad ref"))?;
            match self.resolve_ref(&r)? {
                Node::Native { root, rel, meta, .. } if meta.kind == "file" => {
                    (root, rel, Some((meta.size, meta.mtime)))
                }
                _ => return Err(invalid("open_write ref must be a file")),
            }
        } else {
            let parent = self.parent_dir(None, args)?;
            let name =
                args["name"].as_str().ok_or_else(|| invalid("open_write requires name"))?;
            validate_name(name)?;
            self.check_no_binding(&parent, name)?;
            let (root, rel, full) = self.dir_parts(&parent)?;
            let target_rel = rel_join(&rel, name);
            let base = match std::fs::symlink_metadata(full.join(name)) {
                Ok(m) if m.is_file() => {
                    let nm = node_meta(&m);
                    Some((nm.size, nm.mtime))
                }
                Ok(_) => {
                    return Err(NfsError::new(
                        ErrorCode::NamespaceConflict,
                        format!("'{}' exists and is not a file", name),
                    ))
                }
                Err(_) => None,
            };
            (root, target_rel, base)
        };
        let expected_len = args["size"].as_u64();
        let fb = self.uploads.create(expected_len)?;
        let lease = match self.leases.acquire(&root, &rel, session, base, fb.clone()) {
            Ok(l) => l,
            Err(e) => {
                self.uploads.abort(&fb);
                return Err(e);
            }
        };
        Ok(json!({
            "fb_handle": fb,
            "upload_url": format!("/nfs/v1/uploads/{}", fb),
            "lease": {
                "lease_id": lease.lease_id,
                "seq": lease.seq,
                "ttl_ms": self.config.lease_ttl_secs * 1000,
            },
            "target": { "path": canonical_path(&root, &rel), "exists": base.is_some() },
        }))
    }

    pub fn op_commit_file(&self, at: Option<&Locator>, args: &Value, session: &str) -> NfsResult<Value> {
        let parent = self.parent_dir(at, args)?;
        let name = args["name"].as_str().ok_or_else(|| invalid("commit_file requires name"))?;
        validate_name(name)?;
        self.check_no_binding(&parent, name)?;
        let (root, rel, full) = self.dir_parts(&parent)?;
        let target_rel = rel_join(&rel, name);
        let target_path = full.join(name);
        let overwrite = args["overwrite"].as_bool().unwrap_or(false);
        self.check_dir_cas(
            &parent.container_key().unwrap(),
            args["expected_revision"].as_str(),
        )?;

        // Lease & bypass-writer checks.
        let lease = self.leases.get(&root, &target_rel);
        let held = match (&lease, args["lease_id"].as_str()) {
            (Some(_), Some(lid)) => Some(self.leases.validate(&root, &target_rel, session, lid)?),
            (Some(l), None) => {
                return Err(NfsError::new(
                    ErrorCode::LeaseConflict,
                    "target has an active write lease; pass lease_id",
                )
                .with("holder_session", json!(l.session.clone())))
            }
            (None, _) => None,
        };
        let current = std::fs::symlink_metadata(&target_path).ok().map(|m| node_meta(&m));
        if let Some(h) = &held {
            // Re-check against the open_write snapshot (bypass edits conflict loudly).
            match (&h.base, &current) {
                (Some((bs, bm)), Some(cur)) => {
                    if cur.size != *bs || cur.mtime != *bm {
                        if !overwrite {
                            return Err(NfsError::new(
                                ErrorCode::TargetMismatch,
                                "target was modified outside the protocol since open_write",
                            )
                            .with("reason", json!("bypass_modified")));
                        }
                    }
                }
                (None, Some(_)) if !overwrite => {
                    return Err(NfsError::new(
                        ErrorCode::TargetMismatch,
                        "target appeared since open_write",
                    )
                    .with("reason", json!("target_appeared")));
                }
                _ => {}
            }
        } else if current.is_some() && !overwrite {
            return Err(NfsError::new(
                ErrorCode::TargetMismatch,
                format!("'{}' already exists (open_write it, or pass overwrite:true)", name),
            ));
        }
        if current.as_ref().map(|m| m.kind == "dir").unwrap_or(false) {
            return Err(NfsError::new(
                ErrorCode::NamespaceConflict,
                format!("'{}' is a directory", name),
            ));
        }

        // Obtain content: staged upload, or dedup by hash (秒传).
        let (staged_tmp, size, hash) = if let Some(fb) = args["fb_handle"].as_str() {
            if let Some(h) = &held {
                if h.fb_handle != fb {
                    return Err(invalid("fb_handle does not belong to the held lease"));
                }
            }
            let fin = self.uploads.finish(fb)?;
            (fin.path, fin.size, fin.sha256)
        } else if let Some(hash_arg) = args["hash"].as_str().or(args["obj_id"].as_str()) {
            let hex_hash = normalize_hash(hash_arg)?;
            let (src_root, src_rel) = match self.db.content_index_get(&hex_hash)? {
                Some((r, p, isize, imtime)) => {
                    match self.verify_content_source(&r, &p, isize, imtime, &hex_hash)? {
                        true => (r, p),
                        false => {
                            self.db.content_index_remove(&hex_hash)?;
                            return Err(NfsError::new(
                                ErrorCode::NeedPull,
                                "content is not available locally; upload it",
                            )
                            .with("obj_id", json!(format!("sha256:{}", hex_hash))));
                        }
                    }
                }
                None => {
                    return Err(NfsError::new(
                        ErrorCode::NeedPull,
                        "content is not available locally; upload it",
                    )
                    .with("obj_id", json!(format!("sha256:{}", hex_hash))))
                }
            };
            let src_cfg = self.config.root(&src_root).ok_or_else(|| stale("root gone"))?;
            let src_path = join_root(&src_cfg.path, &src_rel);
            let tmp = self.config.staging_dir().join(format!("dedup_{}", uuid::Uuid::new_v4().simple()));
            std::fs::copy(&src_path, &tmp)?;
            let size = std::fs::metadata(&tmp)?.len();
            (tmp, size, hex_hash)
        } else {
            return Err(invalid("commit_file requires fb_handle or hash"));
        };

        // Atomic placement: rename into a temp in the target dir, then rename over.
        place_file(&staged_tmp, &target_path)?;
        let final_meta = lstat(&target_path)?;

        // Refresh anchor if this path is anchored (edit keeps node identity).
        if let Some(a) = self.db.anchor_get_by_path(&root, &target_rel)? {
            self.db.anchor_rebind(
                a.node_id,
                final_meta.id.ino,
                final_meta.id.dev,
                final_meta.size as i64,
                final_meta.mtime,
            )?;
            self.db.anchor_set_hash(a.node_id, final_meta.size as i64, final_meta.mtime, &hash)?;
        }
        self.db
            .content_index_put(&hash, &root, &target_rel, size as i64, final_meta.mtime)?;
        if let Some(h) = &held {
            self.leases.release(&root, &target_rel, &h.lease_id);
        }
        let revision =
            self.bump_dir(&parent, "entries_changed", Some(json!({"added":[name]})));
        let node = self.resolve_dfs_path(&format!("{}/{}", root, target_rel))?;
        Ok(json!({
            "ref": self.node_ref(&node),
            "entry_ref": native_entry_ref(&root, &rel, name, final_meta.id.ino),
            "revision": revision,
            "obj": {"sha256": hash, "size": size},
        }))
    }

    /// Verifies a content_index hit: sizes must match; a changed mtime triggers
    /// a full hash recompute (the honest slow path — no qcid infra in v1).
    fn verify_content_source(
        &self,
        root: &str,
        rel: &str,
        idx_size: i64,
        idx_mtime: i64,
        expect_hash: &str,
    ) -> NfsResult<bool> {
        let cfg = match self.config.root(root) {
            Some(c) => c,
            None => return Ok(false),
        };
        let path = join_root(&cfg.path, rel);
        let meta = match lstat(&path) {
            Ok(m) if m.kind == "file" => m,
            _ => return Ok(false),
        };
        if meta.size as i64 != idx_size {
            return Ok(false);
        }
        if meta.mtime == idx_mtime {
            return Ok(true);
        }
        let actual = sha256_file(&path)?;
        if actual == expect_hash {
            self.db.content_index_put(expect_hash, root, rel, idx_size, meta.mtime)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ---------- probe ----------

    pub fn op_probe(&self, args: &Value) -> NfsResult<Value> {
        let digests = args["digests"]
            .as_array()
            .ok_or_else(|| invalid("probe requires digests[]"))?;
        if digests.len() > 4096 {
            return Err(invalid("too many digests (max 4096)"));
        }
        let mut missing = Vec::new();
        for d in digests {
            let hash = d["hash"].as_str().ok_or_else(|| invalid("digest requires hash"))?;
            let size = d["size"].as_i64();
            let hex_hash = normalize_hash(hash)?;
            let present = match self.db.content_index_get(&hex_hash)? {
                Some((root, rel, isize, imtime)) => {
                    if size.map(|s| s != isize).unwrap_or(false) {
                        false
                    } else {
                        // Fast path only: (size, mtime) unchanged. A changed
                        // mtime is honestly reported missing; commit-by-hash
                        // still recovers via full re-verification.
                        self.config
                            .root(&root)
                            .and_then(|cfg| lstat(&join_root(&cfg.path, &rel)).ok())
                            .map(|m| m.kind == "file" && m.size as i64 == isize && m.mtime == imtime)
                            .unwrap_or(false)
                    }
                }
                None => false,
            };
            if !present {
                missing.push(d.clone());
            }
        }
        Ok(json!({ "missing": missing }))
    }

    // ---------- meta ----------

    pub fn op_get_meta(&self, at: Option<&Locator>, args: &Value) -> NfsResult<Value> {
        let node = self.meta_target(at, args)?;
        let ns: Vec<String> = args["ns"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let anchors = self.meta_anchors(&node)?;
        let records: Vec<Value> = self
            .db
            .meta_get(&anchors, &ns)?
            .into_iter()
            .map(|m| {
                json!({
                    "ns": m.ns, "key": m.key, "value": m.value,
                    "source": m.source, "confidence": m.confidence,
                    "anchor": m.anchor, "visibility": m.visibility,
                })
            })
            .collect();
        Ok(json!({ "records": records }))
    }

    pub fn op_set_meta(&self, at: Option<&Locator>, args: &Value) -> NfsResult<Value> {
        let node = self.meta_target(at, args)?;
        let records = args["records"]
            .as_array()
            .ok_or_else(|| invalid("set_meta requires records[]"))?;
        for r in records {
            let ns = r["ns"].as_str().ok_or_else(|| invalid("record requires ns"))?;
            if ns != "user" {
                // ai.* requires a pipeline cap — no cap infrastructure in v1.
                return Err(NfsError::new(
                    ErrorCode::PermissionDenied,
                    format!("only the 'user' ns is writable (got '{}')", ns),
                )
                .with("required_op", json!(format!("meta.write.{}", ns))));
            }
        }
        // Writing meta anchors the node (惰性锚定 trigger).
        let node = match node {
            Node::Native { anchored: None, .. } => {
                let (_, _) = self.require_stable_node(&node)?;
                self.re_resolve(&node)?
            }
            n => n,
        };
        let anchor = self
            .meta_anchors(&node)?
            .into_iter()
            .next()
            .ok_or_else(|| invalid("node cannot carry meta"))?;
        let mut count = 0;
        for r in records {
            let key = r["key"].as_str().ok_or_else(|| invalid("record requires key"))?;
            let visibility = r["visibility"].as_str().unwrap_or("private");
            let source = json!({"kind": "user", "at": unix_now()});
            self.db.meta_set(&anchor, "user", key, &r["value"], &source, visibility)?;
            count += 1;
        }
        self.bus.emit(
            "meta_changed",
            None,
            json!({"anchor": anchor, "ns": ["user"]}),
        );
        Ok(json!({ "updated": count }))
    }

    fn meta_target(&self, at: Option<&Locator>, args: &Value) -> NfsResult<Node> {
        if args.get("ref").map(|v| !v.is_null()).unwrap_or(false) {
            let r: WireRef =
                serde_json::from_value(args["ref"].clone()).map_err(|_| invalid("bad ref"))?;
            self.resolve_ref(&r)
        } else if let Some(at) = at {
            self.resolve_locator(at)
        } else {
            Err(invalid("missing target (at or ref)"))
        }
    }

    fn re_resolve(&self, node: &Node) -> NfsResult<Node> {
        match node {
            Node::Native { root, rel, .. } => {
                self.resolve_dfs_path(&format!("{}/{}", root, rel))
            }
            n => Ok(n.clone()),
        }
    }

    // ---------- grant / revoke ----------

    pub fn op_grant(&self, args: &Value) -> NfsResult<Value> {
        let subtree = if let Some(s) = args.get("subtree").filter(|v| !v.is_null()) {
            let loc: Locator = serde_json::from_value(s.clone())
                .map_err(|_| invalid("bad subtree locator"))?;
            match self.resolve_locator(&loc)? {
                Node::Native { root, rel, .. } => canonical_path(&root, &rel),
                Node::Root => "dfs://".to_string(),
                Node::View(v) => format!("view://{}", v.view_id),
                Node::Collection(c) => format!("collection://{}", c.collection_id),
                _ => return Err(invalid("unsupported grant subtree")),
            }
        } else if args.get("ref").map(|v| !v.is_null()).unwrap_or(false) {
            let r: WireRef =
                serde_json::from_value(args["ref"].clone()).map_err(|_| invalid("bad ref"))?;
            match self.resolve_ref(&r)? {
                Node::Native { root, rel, .. } => canonical_path(&root, &rel),
                Node::View(v) => format!("view://{}", v.view_id),
                Node::Collection(c) => format!("collection://{}", c.collection_id),
                _ => return Err(invalid("unsupported grant subtree")),
            }
        } else {
            return Err(invalid("grant requires subtree or ref"));
        };
        let ops: Vec<String> = args["ops"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_else(|| vec!["read".to_string(), "list".to_string()]);
        let expires_at = args["ttl"].as_i64().map(|ttl| unix_now() + ttl);
        let audience = args["audience"].as_str();
        let max_uses = args["max_uses"].as_i64();
        let cap_id = format!("cap_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let token_bytes: [u8; 32] = rand::random();
        let token = {
            use base64::Engine;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token_bytes)
        };
        let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
        self.db
            .grant_create(&cap_id, &token_hash, &subtree, &ops, audience, expires_at, max_uses)?;
        let mut out = json!({"cap_id": cap_id, "token": token, "subtree": subtree, "ops": ops});
        if let Some(e) = expires_at {
            out["expires_at"] = json!(e);
        }
        Ok(out)
    }

    pub fn op_revoke(&self, args: &Value) -> NfsResult<Value> {
        let cap_id =
            args["cap_id"].as_str().ok_or_else(|| invalid("revoke requires cap_id"))?;
        if !self.db.grant_revoke(cap_id)? {
            return Err(not_found(format!("cap '{}' not found", cap_id)));
        }
        Ok(json!({ "revoked": cap_id }))
    }
}

fn normalize_hash(h: &str) -> NfsResult<String> {
    let hex_part = h.strip_prefix("sha256:").unwrap_or(h).to_ascii_lowercase();
    if hex_part.len() != 64 || !hex_part.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(format!("bad sha256 hash '{}'", h)));
    }
    Ok(hex_part)
}

pub fn sha256_file(path: &Path) -> NfsResult<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Moves `staged` over `target` atomically: rename within the target dir; a
/// cross-device staged file is first copied to a temp sibling of the target.
fn place_file(staged: &Path, target: &Path) -> NfsResult<()> {
    match std::fs::rename(staged, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            let dir = target.parent().ok_or_else(|| internal("target has no parent"))?;
            let tmp = dir.join(format!(".nfs_tmp_{}", uuid::Uuid::new_v4().simple()));
            std::fs::copy(staged, &tmp).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                NfsError::from(e).ctx("stage copy")
            })?;
            let _ = std::fs::remove_file(staged);
            std::fs::rename(&tmp, target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                NfsError::from(e).ctx("final rename")
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_normalization() {
        let h = "A".repeat(64);
        assert_eq!(normalize_hash(&h).unwrap(), "a".repeat(64));
        assert_eq!(
            normalize_hash(&format!("sha256:{}", "b".repeat(64))).unwrap(),
            "b".repeat(64)
        );
        assert!(normalize_hash("xyz").is_err());
        assert!(normalize_hash(&"g".repeat(64)).is_err());
    }

    #[test]
    fn sha256_file_streams() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"hello world").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            hex::encode(Sha256::digest(b"hello world"))
        );
    }

    #[test]
    fn place_file_same_dir() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged");
        let target = dir.path().join("sub").join("out.txt");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&staged, b"data").unwrap();
        place_file(&staged, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"data");
        assert!(!staged.exists());
    }
}
