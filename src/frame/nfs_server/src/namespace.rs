//! Unified namespace: locator resolution, node info, and the unified Container
//! `list` over Dir / View / Collection / Group (NFSP §5.2, nfs_server.md §3.1).
//!
//! VisibleEntries(dir) = NativeFsEntries(dir) + VirtualBindings(dir); native
//! entries win name conflicts, conflicting bindings surface via `conflicts[]`
//! (never silently覆盖 or rebound — nfs_server.md §3.5).

use crate::error::*;
use crate::filedb::{Anchor, BindingRow, CollectionRow, Entity, ViewGroupRow, ViewRow};
use crate::fsutil::*;
use crate::handle::{native_entry_ref, watch_token, Cursor, NativeHandle, ROOT_NODE_ID};
use crate::state::AppState;
use crate::types::{Capabilities, Locator, WantMask, WireRef};
use crate::watch::ContainerKey;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub enum Node {
    Root,
    Native {
        root: String,
        rel: String,
        meta: NodeMeta,
        anchored: Option<Entity>,
    },
    View(ViewRow),
    Collection(CollectionRow),
    ViewGroup {
        group: ViewGroupRow,
        view: ViewRow,
    },
    CollectionGroup {
        entity: Entity,
        collection: CollectionRow,
        name: String,
    },
}

impl Node {
    pub fn kind(&self) -> &'static str {
        match self {
            Node::Root => "dir",
            Node::Native { meta, .. } => meta.kind,
            Node::View(_) => "view",
            Node::Collection(_) => "collection",
            Node::ViewGroup { .. } | Node::CollectionGroup { .. } => "group",
        }
    }

    pub fn is_container(&self) -> bool {
        !matches!(self, Node::Native { meta, .. } if meta.kind != "dir")
    }

    pub fn display_name(&self) -> String {
        match self {
            Node::Root => "".to_string(),
            Node::Native { root, rel, .. } => {
                rel_split(rel).map(|(_, n)| n).unwrap_or_else(|| root.clone())
            }
            Node::View(v) => v.title.clone(),
            Node::Collection(c) => c.title.clone(),
            Node::ViewGroup { group, .. } => group.label.clone(),
            Node::CollectionGroup { name, .. } => name.clone(),
        }
    }

    pub fn container_key(&self) -> Option<ContainerKey> {
        match self {
            Node::Root => Some(ContainerKey::Root),
            Node::Native { root, rel, meta, .. } if meta.kind == "dir" => {
                Some(ContainerKey::Dir { root: root.clone(), rel: rel.clone() })
            }
            Node::Native { .. } => None,
            Node::View(v) => Some(ContainerKey::Node(v.node_id)),
            Node::Collection(c) => Some(ContainerKey::Node(c.node_id)),
            Node::ViewGroup { group, .. } => Some(ContainerKey::Node(group.node_id)),
            Node::CollectionGroup { entity, .. } => Some(ContainerKey::Node(entity.node_id)),
        }
    }
}

pub fn canonical_path(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        format!("dfs://{}", root)
    } else {
        format!("dfs://{}/{}", root, rel)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListArgs {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub filter: Option<ListFilter>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListFilter {
    #[serde(default)]
    pub kind: Option<Vec<String>>,
    #[serde(default)]
    pub name_glob: Option<String>,
}

impl AppState {
    // ---------- resolution ----------

    pub fn resolve_locator(&self, at: &Locator) -> NfsResult<Node> {
        if let Some(r) = &at.r#ref {
            return self.resolve_ref(r);
        }
        if let Some(uri) = &at.uri {
            return self.resolve_uri(uri);
        }
        if let Some(path) = &at.path {
            let realm = at.realm.as_deref().unwrap_or("dfs");
            if realm != "dfs" {
                return Err(not_found(format!("unknown realm '{}'", realm)));
            }
            return self.resolve_dfs_path(path);
        }
        Err(invalid("locator requires ref, uri or path"))
    }

    fn resolve_uri(&self, uri: &str) -> NfsResult<Node> {
        let decoded: String = percent_encoding::percent_decode_str(uri)
            .decode_utf8()
            .map_err(|_| invalid("bad uri encoding"))?
            .into_owned();
        if let Some(rest) = decoded.strip_prefix("dfs://") {
            return self.resolve_dfs_path(rest);
        }
        if let Some(view_id) = decoded.strip_prefix("view://") {
            let v = self
                .db
                .view_by_id(view_id)?
                .ok_or_else(|| not_found(format!("view '{}' not found", view_id)))?;
            return Ok(Node::View(v));
        }
        if let Some(cid) = decoded.strip_prefix("collection://") {
            let c = self
                .db
                .collection_by_id(cid)?
                .ok_or_else(|| not_found(format!("collection '{}' not found", cid)))?;
            return Ok(Node::Collection(c));
        }
        Err(invalid(format!("unsupported uri scheme in '{}'", uri)))
    }

    pub fn resolve_dfs_path(&self, path: &str) -> NfsResult<Node> {
        let rel_all = normalize_rel(path)?;
        if rel_all.is_empty() {
            return Ok(Node::Root);
        }
        let (root_id, rel) = match rel_all.split_once('/') {
            Some((r, rest)) => (r.to_string(), rest.to_string()),
            None => (rel_all.clone(), String::new()),
        };
        let root = self
            .config
            .root(&root_id)
            .ok_or_else(|| not_found(format!("export root '{}' not found", root_id)))?;
        let full = join_root(&root.path, &rel);
        let meta = lstat(&full).map_err(|e| {
            if e.code == ErrorCode::NotFound {
                not_found(format!("path '/{}' not found", rel_all))
            } else {
                e
            }
        })?;
        let anchored = self.lookup_anchor_entity(&root_id, &rel, &meta)?;
        Ok(Node::Native { root: root_id, rel, meta, anchored })
    }

    /// Pure-read anchor lookup: attaches the entity only when the anchor row
    /// matches the live native id (no DB writes on the browse path).
    fn lookup_anchor_entity(
        &self,
        root: &str,
        rel: &str,
        meta: &NodeMeta,
    ) -> NfsResult<Option<Entity>> {
        if rel.is_empty() {
            return Ok(None);
        }
        match self.db.anchor_get_by_path(root, rel)? {
            Some(a) if a.state == "ok" && a.ino == meta.id.ino => {
                Ok(self.db.entity_get(a.node_id)?)
            }
            _ => Ok(None),
        }
    }

    pub fn resolve_ref(&self, r: &WireRef) -> NfsResult<Node> {
        match r {
            WireRef::Object { .. } => Err(NfsError::new(
                ErrorCode::Unsupported,
                "object refs are not resolvable in v1 (no frozen subtrees)",
            )),
            WireRef::Live { node_id, gen } => {
                if node_id == ROOT_NODE_ID {
                    return Ok(Node::Root);
                }
                if node_id.starts_with("nh_") {
                    return self.resolve_handle(node_id);
                }
                if let Some(idstr) = node_id.strip_prefix("n_") {
                    let id: i64 =
                        idstr.parse().map_err(|_| stale(format!("bad node id '{}'", node_id)))?;
                    return self.resolve_node_id(id, if *gen == 0 { None } else { Some(*gen) });
                }
                Err(stale(format!("unknown node id form '{}'", node_id)))
            }
        }
    }

    fn resolve_handle(&self, node_id: &str) -> NfsResult<Node> {
        let h: NativeHandle = self.handles.decode(node_id)?;
        let root = self
            .config
            .root(&h.r)
            .ok_or_else(|| stale(format!("export root '{}' no longer exists", h.r)))?;
        let full = join_root(&root.path, &h.p);
        let meta = lstat(&full).map_err(|e| {
            if e.code == ErrorCode::NotFound {
                stale(format!("'{}' is gone; re-resolve from a trusted locator", h.p))
            } else {
                e
            }
        })?;
        if h.i != 0 && meta.id.ino != 0 && meta.id.ino != h.i {
            return Err(stale(format!(
                "native id changed at '{}'; re-resolve from a trusted locator",
                h.p
            )));
        }
        let anchored = self.lookup_anchor_entity(&h.r, &h.p, &meta)?;
        Ok(Node::Native { root: h.r, rel: h.p, meta, anchored })
    }

    pub fn resolve_node_id(&self, id: i64, expect_gen: Option<u64>) -> NfsResult<Node> {
        let ent = self
            .db
            .entity_get(id)?
            .ok_or_else(|| stale(format!("node n_{} does not exist", id)))?;
        if let Some(g) = expect_gen {
            if g != ent.gen {
                return Err(stale(format!("gen mismatch for n_{} (identity replaced)", id)));
            }
        }
        match ent.kind.as_str() {
            "view" => {
                let v = self.db.view_by_node(id)?.ok_or_else(|| internal("view row missing"))?;
                Ok(Node::View(v))
            }
            "collection" => {
                let c = self
                    .db
                    .collection_by_node(id)?
                    .ok_or_else(|| internal("collection row missing"))?;
                Ok(Node::Collection(c))
            }
            "group" => {
                if let Some(g) = self.db.view_group_get(id)? {
                    let v = self
                        .db
                        .view_by_node(g.view_node_id)?
                        .ok_or_else(|| internal("view row missing"))?;
                    return Ok(Node::ViewGroup { group: g, view: v });
                }
                if let Some(cid) = self.db.collection_group_home(id)? {
                    let c = self
                        .db
                        .collection_by_node(cid)?
                        .ok_or_else(|| internal("collection row missing"))?;
                    // Group display name lives on its collection entry.
                    let name = self
                        .db
                        .collection_entries(cid, None)?
                        .into_iter()
                        .find(|e| e.node_type == "group" && e.target_node_id == Some(id))
                        .and_then(|e| e.name)
                        .unwrap_or_default();
                    return Ok(Node::CollectionGroup { entity: ent, collection: c, name });
                }
                Err(stale(format!("group n_{} is orphaned", id)))
            }
            "file" | "dir" => self.resolve_anchored_native(&ent),
            other => Err(internal(format!("unknown entity kind '{}'", other))),
        }
    }

    /// Resolves an anchored native node, applying the on-access half of the
    /// reconcile ladder (nfs_server.md §3.6): same path + new native id ⇒
    /// editor-overwrite heuristic rebinds the anchor; missing path ⇒ stale.
    fn resolve_anchored_native(&self, ent: &Entity) -> NfsResult<Node> {
        let a: Anchor = self
            .db
            .anchor_get(ent.node_id)?
            .ok_or_else(|| stale(format!("n_{} has no anchor", ent.node_id)))?;
        if a.state == "deleted" {
            return Err(stale(format!("n_{} was deleted", ent.node_id)));
        }
        if a.state == "stale" {
            return Err(stale(format!(
                "n_{} is stale (bypass change could not be reconciled)",
                ent.node_id
            )));
        }
        let root = self
            .config
            .root(&a.root_id)
            .ok_or_else(|| stale(format!("export root '{}' no longer exists", a.root_id)))?;
        let full = join_root(&root.path, &a.path);
        let meta = match lstat(&full) {
            Ok(m) => m,
            Err(e) if e.code == ErrorCode::NotFound => {
                // Renames are detected by the scan loop; here we can only be honest.
                self.db.anchor_set_state(ent.node_id, "stale")?;
                return Err(stale(format!("n_{} target path is gone", ent.node_id)));
            }
            Err(e) => return Err(e),
        };
        if meta.id.ino != a.ino {
            // Same path, new native id: the file was overwritten in place.
            self.db.anchor_rebind(
                ent.node_id,
                meta.id.ino,
                meta.id.dev,
                meta.size as i64,
                meta.mtime,
            )?;
        } else if a.size != Some(meta.size as i64) || a.mtime != Some(meta.mtime) {
            self.db.anchor_touch(ent.node_id, meta.size as i64, meta.mtime, true)?;
        }
        Ok(Node::Native {
            root: a.root_id,
            rel: a.path,
            meta,
            anchored: Some(ent.clone()),
        })
    }

    // ---------- refs & revisions ----------

    pub fn node_ref(&self, node: &Node) -> WireRef {
        match node {
            Node::Root => WireRef::live(ROOT_NODE_ID, 0),
            Node::Native { anchored: Some(e), .. } => {
                WireRef::live(format!("n_{}", e.node_id), e.gen)
            }
            Node::Native { root, rel, meta, .. } => {
                let h = NativeHandle {
                    r: root.clone(),
                    p: rel.clone(),
                    i: meta.id.ino,
                    k: match meta.kind {
                        "dir" => "d",
                        "symlink" => "l",
                        _ => "f",
                    }
                    .to_string(),
                };
                WireRef::live(self.handles.encode(&h), 0)
            }
            Node::View(v) => WireRef::live(format!("n_{}", v.node_id), 1),
            Node::Collection(c) => WireRef::live(format!("n_{}", c.node_id), 1),
            Node::ViewGroup { group, .. } => WireRef::live(format!("n_{}", group.node_id), 1),
            Node::CollectionGroup { entity, .. } => {
                WireRef::live(format!("n_{}", entity.node_id), entity.gen)
            }
        }
    }

    pub fn node_revision(&self, node: &Node) -> Option<String> {
        match node {
            Node::Root => Some(self.revisions.current(&ContainerKey::Root)),
            Node::Native { meta, .. } if meta.kind == "dir" => {
                Some(self.revisions.current(&node.container_key().unwrap()))
            }
            Node::Native { .. } => None,
            Node::View(v) => Some(format!("g-{}", v.generation)),
            Node::Collection(c) => Some(format!("g-{}", c.generation)),
            // Group revisions follow their parent container's generation.
            Node::ViewGroup { view, .. } => Some(format!("g-{}", view.generation)),
            Node::CollectionGroup { collection, .. } => {
                Some(format!("g-{}", collection.generation))
            }
        }
    }

    // ---------- node info (resolve/stat result) ----------

    pub fn node_info(&self, node: &Node, want: &WantMask) -> NfsResult<Value> {
        let kind = node.kind();
        let writable = !matches!(node, Node::Root | Node::View(_) | Node::ViewGroup { .. });
        let mut out = json!({
            "kind": kind,
            "state": "live",
            "ref": self.node_ref(node),
            "capabilities": Capabilities::for_kind(kind, writable),
        });
        if let Some(rev) = self.node_revision(node) {
            out["revision"] = json!(rev);
        }
        out["locations"] = json!(self.node_locations(node)?);
        match node {
            Node::View(v) => {
                out["view_id"] = json!(v.view_id);
                out["origin"] = json!(v.origin);
                out["title"] = json!(v.title);
                out["stale"] = json!(v.stale);
            }
            Node::Collection(c) => {
                out["collection_id"] = json!(c.collection_id);
                out["title"] = json!(c.title);
            }
            _ => {}
        }
        if want.has("base") {
            out["name"] = json!(node.display_name());
            if let Node::Native { meta, .. } = node {
                out["size"] = json!(meta.size);
                out["mtime"] = json!(meta.mtime);
                out["ctime"] = json!(meta.ctime);
            }
            out["flags"] = json!(self.node_flags(node));
        }
        if want.has("ident") {
            if let WireRef::Live { node_id, gen } = self.node_ref(node) {
                out["node_id"] = json!(node_id);
                out["gen"] = json!(gen);
            }
            if let Node::Native { meta, anchored, .. } = node {
                if meta.kind == "file" {
                    out["etag"] = json!(format!("{}-{}", meta.size, meta.mtime));
                }
                if let Some(e) = anchored {
                    if let Some(a) = self.db.anchor_get(e.node_id)? {
                        if let Some(h) = a.full_hash {
                            out["obj_id"] = json!(format!("sha256:{}", h));
                        }
                    }
                }
            }
        }
        if want.has("access") {
            out["access_urls"] = json!(self.access_urls(node));
        }
        if want.has("meta") {
            let anchors = self.meta_anchors(node)?;
            let mut summary = serde_json::Map::new();
            for (ns, count) in self.db.meta_summary(&anchors)? {
                summary.insert(ns, json!(count));
            }
            out["meta_summary"] = Value::Object(summary);
        }
        Ok(out)
    }

    pub fn node_flags(&self, node: &Node) -> Vec<&'static str> {
        let mut flags = Vec::new();
        match node {
            Node::Root | Node::View(_) | Node::ViewGroup { .. } => flags.push("read_only"),
            Node::Native { root, rel, meta, .. } if meta.kind == "file" => {
                if self.leases.get(root, rel).is_some() {
                    flags.push("writing");
                }
            }
            _ => {}
        }
        flags
    }

    /// Location entries: the direct locator plus any DFS reference bindings
    /// pointing at this node (NFSP §3.1.1 locations[]).
    fn node_locations(&self, node: &Node) -> NfsResult<Vec<Value>> {
        let mut out = Vec::new();
        let (direct, target_id): (Option<String>, Option<i64>) = match node {
            Node::Root => (Some("dfs://".to_string()), None),
            Node::Native { root, rel, anchored, .. } => (
                Some(canonical_path(root, rel)),
                anchored.as_ref().map(|e| e.node_id),
            ),
            Node::View(v) => (Some(format!("view://{}", v.view_id)), Some(v.node_id)),
            Node::Collection(c) => {
                (Some(format!("collection://{}", c.collection_id)), Some(c.node_id))
            }
            Node::ViewGroup { group, .. } => (None, Some(group.node_id)),
            Node::CollectionGroup { entity, .. } => (None, Some(entity.node_id)),
        };
        if let Some(url) = direct {
            out.push(json!({"url": url, "via": "direct"}));
        }
        if let Some(id) = target_id {
            for b in self.db.bindings_by_target(id)? {
                if b.state != "ok" {
                    continue;
                }
                if let Some(pa) = self.db.anchor_get(b.parent_node_id)? {
                    if pa.state == "ok" {
                        out.push(json!({
                            "url": format!("{}/{}", canonical_path(&pa.root_id, &pa.path), b.name),
                            "via": "reference",
                        }));
                    }
                }
            }
        }
        Ok(out)
    }

    fn access_urls(&self, node: &Node) -> Vec<Value> {
        match node {
            Node::Native { root, rel, meta, .. } if meta.kind != "dir" => {
                let r = self.node_ref(node);
                let node_id = match &r {
                    WireRef::Live { node_id, .. } => node_id.clone(),
                    _ => unreachable!(),
                };
                vec![
                    json!({"kind": "fs", "url": canonical_path(root, rel), "primary": true}),
                    json!({"kind": "read", "url": format!("/nfs/v1/read/{}", node_id)}),
                ]
            }
            Node::Native { root, rel, .. } => {
                vec![json!({"kind": "fs", "url": canonical_path(root, rel), "primary": true})]
            }
            _ => Vec::new(),
        }
    }

    /// Meta anchor strings for a node: `live:n_<id>` plus `obj:sha256:<hash>`
    /// when committed content is known.
    pub fn meta_anchors(&self, node: &Node) -> NfsResult<Vec<String>> {
        let mut out = Vec::new();
        match node {
            Node::Native { anchored: Some(e), .. } => {
                out.push(format!("live:n_{}", e.node_id));
                if let Some(a) = self.db.anchor_get(e.node_id)? {
                    if let Some(h) = a.full_hash {
                        out.push(format!("obj:sha256:{}", h));
                    }
                }
            }
            Node::View(v) => out.push(format!("live:n_{}", v.node_id)),
            Node::Collection(c) => out.push(format!("live:n_{}", c.node_id)),
            Node::ViewGroup { group, .. } => out.push(format!("live:n_{}", group.node_id)),
            Node::CollectionGroup { entity, .. } => out.push(format!("live:n_{}", entity.node_id)),
            _ => {}
        }
        Ok(out)
    }

    // ---------- unified list ----------

    pub fn list_node(&self, node: &Node, args: &ListArgs, want: &WantMask) -> NfsResult<Value> {
        if !node.is_container() {
            return Err(NfsError::new(
                ErrorCode::NotAContainer,
                format!("{} nodes cannot be listed", node.kind()),
            ));
        }
        let limit = args.limit.unwrap_or(200).clamp(1, self.config.max_list);
        let revision = self.node_revision(node).unwrap_or_default();
        let container_key = node.container_key().unwrap();

        let (default_order, mut rows, conflicts) = match node {
            Node::Root => ("name", self.root_entries(want)?, Vec::new()),
            Node::Native { .. } => {
                let (rows, conflicts) = self.dir_entries(node, want)?;
                ("name", rows, conflicts)
            }
            Node::View(v) => ("manual", self.view_entries(v, None, want)?, Vec::new()),
            Node::ViewGroup { group, view } => {
                ("manual", self.view_entries(view, Some(group.node_id), want)?, Vec::new())
            }
            Node::Collection(c) => {
                ("manual", self.collection_entries_json(c, None, want)?, Vec::new())
            }
            Node::CollectionGroup { entity, collection, .. } => (
                "manual",
                self.collection_entries_json(collection, Some(entity.node_id), want)?,
                Vec::new(),
            ),
        };
        let order = args.order.clone().unwrap_or_else(|| default_order.to_string());
        if order != "manual" {
            match order.as_str() {
                "name" | "mtime" | "size" => {}
                other => return Err(invalid(format!("unknown order '{}'", other))),
            }
        }

        // Filters apply before pagination.
        if let Some(f) = &args.filter {
            if let Some(kinds) = &f.kind {
                rows.retain(|r| kinds.iter().any(|k| k == &r.kind));
            }
            if let Some(g) = &f.name_glob {
                rows.retain(|r| glob_match(g, &r.name));
            }
        }

        // Server-side stable sort (byte order on names; manual keeps stored order).
        match order.as_str() {
            "name" => rows.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes())),
            "mtime" => rows.sort_by(|a, b| {
                (a.mtime, a.name.as_bytes()).cmp(&(b.mtime, b.name.as_bytes()))
            }),
            "size" => {
                rows.sort_by(|a, b| (a.size, a.name.as_bytes()).cmp(&(b.size, b.name.as_bytes())))
            }
            _ => {}
        }

        // Cursor: skip past the last emitted sort key (D10: never reset).
        let mut revision_changed = false;
        let mut start = 0usize;
        if let Some(cs) = &args.cursor {
            let cur = Cursor::decode(cs)?;
            if cur.order != order {
                return Err(invalid("cursor order does not match requested order"));
            }
            if cur.rev != revision {
                revision_changed = true;
            }
            start = rows
                .iter()
                .position(|r| cmp_keys(&r.sort_key(&order), &cur.key) == std::cmp::Ordering::Greater)
                .unwrap_or(rows.len());
        }
        let page: Vec<&EntryRow> = rows.iter().skip(start).take(limit).collect();
        let truncated = start + page.len() < rows.len();
        let next_cursor = if truncated {
            page.last().map(|last| {
                Cursor {
                    v: 1,
                    rev: if let Some(cs) = &args.cursor {
                        Cursor::decode(cs).map(|c| c.rev).unwrap_or_else(|_| revision.clone())
                    } else {
                        revision.clone()
                    },
                    order: order.clone(),
                    key: last.sort_key(&order),
                }
                .encode()
            })
        } else {
            None
        };

        let entries: Vec<Value> = page.iter().map(|r| r.json.clone()).collect();
        let kind = node.kind();
        let writable = !matches!(node, Node::Root | Node::View(_) | Node::ViewGroup { .. });
        let mut out = json!({
            "container": {
                "ref": self.node_ref(node),
                "kind": kind,
                "revision": revision,
                "capabilities": Capabilities::for_kind(kind, writable),
            },
            "entries": entries,
            "truncated": truncated,
            "conflicts": conflicts,
            "watch_token": watch_token(&self.watch_key, &container_key.canonical()),
        });
        if let Some(c) = next_cursor {
            out["next_cursor"] = json!(c);
        }
        if revision_changed {
            out["revision_changed"] = json!(true);
        }
        Ok(out)
    }

    fn root_entries(&self, want: &WantMask) -> NfsResult<Vec<EntryRow>> {
        let mut rows = Vec::new();
        for export in &self.config.exports {
            let meta = match lstat(&export.path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let node = Node::Native {
                root: export.id.clone(),
                rel: String::new(),
                meta: meta.clone(),
                anchored: None,
            };
            let target = self.target_json(&node, want)?;
            rows.push(EntryRow {
                name: export.id.clone(),
                kind: "dir".to_string(),
                mtime: meta.mtime,
                size: meta.size,
                order_index: 0,
                json: json!({
                    "entry_ref": native_entry_ref("", "", &export.id, meta.id.ino),
                    "name": export.id,
                    "binding": "native",
                    "target": target,
                }),
            });
        }
        Ok(rows)
    }

    fn dir_entries(&self, node: &Node, want: &WantMask) -> NfsResult<(Vec<EntryRow>, Vec<Value>)> {
        let (root, rel, anchored) = match node {
            Node::Native { root, rel, meta, anchored } if meta.kind == "dir" => {
                (root.clone(), rel.clone(), anchored.clone())
            }
            _ => return Err(NfsError::new(ErrorCode::NotAContainer, "not a directory")),
        };
        let root_cfg = self.config.root(&root).ok_or_else(|| stale("root gone"))?;
        let full = join_root(&root_cfg.path, &rel);
        let mut rows = Vec::new();
        let mut native_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for dent in std::fs::read_dir(&full)? {
            let dent = dent?;
            let name = match dent.file_name().into_string() {
                Ok(n) => n,
                Err(_) => continue, // non-UTF8 names are not representable in the protocol
            };
            let meta = match lstat(&dent.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            native_names.insert(name.clone());
            let child_rel = rel_join(&rel, &name);
            let child = Node::Native {
                root: root.clone(),
                rel: child_rel,
                meta: meta.clone(),
                anchored: None, // attached below in batch for the emitted page
            };
            let target = self.target_json(&child, want)?;
            rows.push(EntryRow {
                name: name.clone(),
                kind: meta.kind.to_string(),
                mtime: meta.mtime,
                size: meta.size,
                order_index: 0,
                json: json!({
                    "entry_ref": native_entry_ref(&root, &rel, &name, meta.id.ino),
                    "name": name,
                    "binding": "native",
                    "target": target,
                }),
            });
        }
        // Sidecar merge: virtual bindings of this dir (only when it is anchored).
        let mut conflicts = Vec::new();
        if let Some(ent) = &anchored {
            for b in self.db.bindings_by_parent(ent.node_id)? {
                if native_names.contains(&b.name) {
                    // Native item stays visible unchanged; binding surfaces as conflict.
                    conflicts.push(json!({
                        "name": b.name,
                        "entry_ref": format!("be_{}", b.entry_id),
                        "target": self.binding_target_json(&b, want)?,
                        "reason": "native_shadow",
                    }));
                    continue;
                }
                let mut target = self.binding_target_json(&b, want)?;
                // canonical_path lives on the entry (consistent with View /
                // Collection member envelopes).
                let canonical = target.as_object_mut().and_then(|o| o.remove("canonical_path"));
                let mut entry = json!({
                    "entry_ref": format!("be_{}", b.entry_id),
                    "name": b.name,
                    "binding": "reference",
                    "target": target,
                });
                if let Some(cp) = canonical {
                    entry["canonical_path"] = cp;
                }
                rows.push(EntryRow {
                    name: b.name.clone(),
                    kind: entry["target"]["kind"].as_str().unwrap_or("").to_string(),
                    mtime: 0,
                    size: 0,
                    order_index: 0,
                    json: entry,
                });
            }
        }
        // Attach ident info for anchored children in one batch query (no N+1).
        if want.has("ident") {
            let rels: Vec<String> = rows
                .iter()
                .filter(|r| r.json["binding"] == "native")
                .map(|r| rel_join(&rel, &r.name))
                .collect();
            let anchors = self.db.anchors_by_paths(&root, &rels)?;
            for a in anchors {
                if a.state != "ok" {
                    continue;
                }
                if let Some((_, name)) = rel_split(&a.path) {
                    if let Some(row) =
                        rows.iter_mut().find(|r| r.name == name && r.json["binding"] == "native")
                    {
                        // Only attach when the live ino still matches the anchor.
                        let ino = row.json["target"]["attrs"]["_ino"].as_u64().unwrap_or(0);
                        if ino == a.ino {
                            if let Some(e) = self.db.entity_get(a.node_id)? {
                                row.json["target"]["ref"] =
                                    json!(WireRef::live(format!("n_{}", e.node_id), e.gen));
                                if let Some(h) = &a.full_hash {
                                    row.json["target"]["attrs"]["obj_id"] =
                                        json!(format!("sha256:{}", h));
                                }
                            }
                        }
                    }
                }
            }
        }
        // Drop the internal _ino marker.
        for row in rows.iter_mut() {
            if let Some(attrs) = row.json["target"]["attrs"].as_object_mut() {
                attrs.remove("_ino");
            }
        }
        Ok((rows, conflicts))
    }

    /// target JSON for a native child (handles want-mask attrs).
    fn target_json(&self, node: &Node, want: &WantMask) -> NfsResult<Value> {
        let mut attrs = serde_json::Map::new();
        if let Node::Native { meta, .. } = node {
            if want.has("base") {
                attrs.insert("size".into(), json!(meta.size));
                attrs.insert("mtime".into(), json!(meta.mtime));
                attrs.insert("flags".into(), json!(self.node_flags(node)));
            }
            if want.has("ident") {
                if meta.kind == "file" {
                    attrs.insert("etag".into(), json!(format!("{}-{}", meta.size, meta.mtime)));
                }
                // Internal marker for the batch anchor attach (removed before output).
                attrs.insert("_ino".into(), json!(meta.id.ino));
            }
            if want.has("access") {
                attrs.insert("access_urls".into(), json!(self.access_urls(node)));
            }
        }
        Ok(json!({
            "ref": self.node_ref(node),
            "kind": node.kind(),
            "attrs": Value::Object(attrs),
        }))
    }

    /// target JSON for a virtual binding row (live target may be stale).
    pub fn binding_target_json(&self, b: &BindingRow, want: &WantMask) -> NfsResult<Value> {
        if b.target_type == "object" {
            return Ok(json!({
                "ref": WireRef::Object { obj_id: b.target_obj_id.clone().unwrap_or_default(), inner_path: None },
                "kind": "file",
                "attrs": {},
            }));
        }
        let id = b.target_node_id.unwrap_or(0);
        if b.state != "ok" {
            return Ok(json!({
                "ref": WireRef::live(format!("n_{}", id), 0),
                "kind": Value::Null,
                "attrs": {},
                "target_state": "stale",
            }));
        }
        match self.resolve_node_id(id, None) {
            Ok(target) => {
                let mut v = self.target_json(&target, want)?;
                if let Node::Native { root, rel, .. } = &target {
                    v["canonical_path"] = json!(canonical_path(root, rel));
                }
                Ok(v)
            }
            Err(e) if e.code == ErrorCode::Stale => Ok(json!({
                "ref": WireRef::live(format!("n_{}", id), 0),
                "kind": Value::Null,
                "attrs": {},
                "target_state": "stale",
            })),
            Err(e) => Err(e),
        }
    }

    // ---------- walk (batch cursor movement) ----------

    pub fn walk_child(&self, node: &Node, name: &str) -> NfsResult<Node> {
        match node {
            Node::Root => {
                if self.config.root(name).is_some() {
                    self.resolve_dfs_path(name)
                } else {
                    Err(not_found(format!("export root '{}' not found", name)))
                }
            }
            Node::Native { root, rel, meta, anchored } if meta.kind == "dir" => {
                validate_name(name)?;
                let child_rel = rel_join(rel, name);
                let root_cfg = self.config.root(root).ok_or_else(|| stale("root gone"))?;
                match lstat(&join_root(&root_cfg.path, &child_rel)) {
                    Ok(m) => {
                        let anchored_child = self.lookup_anchor_entity(root, &child_rel, &m)?;
                        Ok(Node::Native {
                            root: root.clone(),
                            rel: child_rel,
                            meta: m,
                            anchored: anchored_child,
                        })
                    }
                    Err(e) if e.code == ErrorCode::NotFound => {
                        // Fall through to virtual bindings.
                        if let Some(ent) = anchored {
                            for b in self.db.bindings_by_parent(ent.node_id)? {
                                if b.name == name && b.state == "ok" && b.target_type == "live" {
                                    return self.resolve_node_id(b.target_node_id.unwrap(), None);
                                }
                            }
                        }
                        Err(e)
                    }
                    Err(e) => Err(e),
                }
            }
            Node::Collection(c) => {
                let matches: Vec<_> = self
                    .db
                    .collection_entries(c.node_id, None)?
                    .into_iter()
                    .filter(|e| e.name.as_deref() == Some(name))
                    .collect();
                match matches.len() {
                    0 => Err(not_found(format!("no entry named '{}'", name))),
                    1 => self.collection_entry_target(&matches[0]),
                    _ => Err(NfsError::new(
                        ErrorCode::AmbiguousEntry,
                        format!("multiple entries named '{}'; walk by entry_ref", name),
                    )),
                }
            }
            Node::View(v) => {
                for g in self.db.view_groups(v.node_id)? {
                    if g.label == name {
                        return Ok(Node::ViewGroup { group: g, view: v.clone() });
                    }
                }
                Err(not_found(format!("no group '{}' in view", name)))
            }
            _ => Err(NfsError::new(
                ErrorCode::NotAContainer,
                format!("cannot walk into {}", node.kind()),
            )),
        }
    }

    pub fn walk_entry_ref(&self, node: &Node, entry_ref: &str) -> NfsResult<Node> {
        if let Some(id) = entry_ref.strip_prefix("ce_") {
            let id: i64 = id.parse().map_err(|_| invalid("bad entry_ref"))?;
            let e = self
                .db
                .collection_entry_get(id)?
                .ok_or_else(|| stale(format!("entry '{}' not found", entry_ref)))?;
            // Must belong to the collection being walked.
            let cid = match node {
                Node::Collection(c) => c.node_id,
                Node::CollectionGroup { collection, .. } => collection.node_id,
                _ => return Err(invalid("entry_ref walk requires a collection cursor")),
            };
            if e.collection_node_id != cid {
                return Err(stale("entry belongs to another collection"));
            }
            return self.collection_entry_target(&e);
        }
        Err(invalid(format!("walk by entry_ref not supported for '{}'", entry_ref)))
    }

    pub fn collection_entry_target(
        &self,
        e: &crate::filedb::CollectionEntryRow,
    ) -> NfsResult<Node> {
        match e.node_type.as_str() {
            "group" => self.resolve_node_id(
                e.target_node_id.ok_or_else(|| internal("group without node"))?,
                None,
            ),
            _ => match e.target_type.as_deref() {
                Some("live") => self.resolve_node_id(
                    e.target_node_id.ok_or_else(|| stale("ref without target"))?,
                    None,
                ),
                _ => Err(NfsError::new(
                    ErrorCode::Unsupported,
                    "object targets are not resolvable in v1",
                )),
            },
        }
    }
}

/// Internal row used for sorting/pagination before JSON emission.
pub struct EntryRow {
    pub name: String,
    pub kind: String,
    pub mtime: i64,
    pub size: u64,
    /// position for manual-ordered containers
    pub order_index: i64,
    pub json: Value,
}

impl EntryRow {
    pub fn sort_key(&self, order: &str) -> Vec<Value> {
        match order {
            "mtime" => vec![json!(self.mtime), json!(self.name)],
            "size" => vec![json!(self.size), json!(self.name)],
            "manual" => vec![json!(self.order_index), json!(self.name)],
            _ => vec![json!(self.name)],
        }
    }
}

pub fn cmp_keys(a: &[Value], b: &[Value]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for (x, y) in a.iter().zip(b.iter()) {
        let ord = match (x, y) {
            (Value::Number(nx), Value::Number(ny)) => {
                let fx = nx.as_f64().unwrap_or(0.0);
                let fy = ny.as_f64().unwrap_or(0.0);
                fx.partial_cmp(&fy).unwrap_or(Ordering::Equal)
            }
            (Value::String(sx), Value::String(sy)) => sx.as_bytes().cmp(sy.as_bytes()),
            _ => Ordering::Equal,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_comparison() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_keys(&[json!("a")], &[json!("b")]), Less);
        assert_eq!(cmp_keys(&[json!(2), json!("a")], &[json!(2), json!("a")]), Equal);
        assert_eq!(cmp_keys(&[json!(3), json!("a")], &[json!(2), json!("z")]), Greater);
        assert_eq!(cmp_keys(&[json!(1)], &[json!(1), json!("x")]), Less);
    }
}
