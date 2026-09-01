//! View & Collection containers (NFSP §3.5/§3.5.1/§5.4).
//!
//! Views are read-only query-derived containers; v1 has no AI generator, so
//! view content enters filedb through the debug API or tests (nfs_server.md
//! §4.1: view 只读, view_patch 延后 — the base/patch merge logic is in filedb).
//! Collections are user-managed ordered reference containers; members persist
//! target Refs only, canonical_path is batch-resolved at read time (§3.2 红线).

use crate::error::*;
use crate::filedb::{CollectionPatchOp, CollectionRow, ViewRow};
use crate::namespace::{canonical_path, EntryRow, Node};
use crate::state::AppState;
use crate::types::{WantMask, WireRef};
use crate::watch::ContainerKey;
use serde_json::{json, Value};

impl AppState {
    // ---------- listing ----------

    pub(crate) fn view_entries(
        &self,
        view: &ViewRow,
        group: Option<i64>,
        want: &WantMask,
    ) -> NfsResult<Vec<EntryRow>> {
        let mut rows = Vec::new();
        let mut idx: i64 = 0;
        if group.is_none() {
            for g in self.db.view_groups(view.node_id)? {
                let member_count = self.db.view_members(view.node_id, Some(g.node_id))?.len();
                rows.push(EntryRow {
                    name: g.label.clone(),
                    kind: "group".to_string(),
                    mtime: 0,
                    size: 0,
                    order_index: idx,
                    json: json!({
                        "entry_ref": format!("vg_{}", g.node_id),
                        "name": g.label,
                        "binding": "derived",
                        "target": {
                            "ref": WireRef::live(format!("n_{}", g.node_id), 1),
                            "kind": "group",
                            "attrs": {},
                        },
                        "context": {"type": "view", "by": g.by_dim, "count": member_count},
                    }),
                });
                idx += 1;
            }
        }
        for m in self.db.view_members(view.node_id, group)? {
            let entry_ref = match m.layer {
                "patch" => format!("ve_p{}", m.id),
                _ => format!("ve_b{}", m.id),
            };
            let (target, name, canonical) = self.member_target_json(
                m.target_type.as_str(),
                m.target_node_id,
                m.target_obj_id.as_deref(),
                want,
            )?;
            let mut context = json!({"type": "view"});
            if let Some(p) = &m.provenance {
                context["provenance"] = p.clone();
            }
            if m.pinned {
                context["pinned"] = json!(true);
            }
            let mut entry = json!({
                "entry_ref": entry_ref,
                "name": name,
                "binding": "derived",
                "target": target,
                "context": context,
            });
            if let Some(c) = canonical {
                // canonical_path is protocol-mandatory for view members (PRD 11.5).
                entry["canonical_path"] = json!(c);
            }
            rows.push(EntryRow {
                name: entry["name"].as_str().unwrap_or("").to_string(),
                kind: entry["target"]["kind"].as_str().unwrap_or("").to_string(),
                mtime: 0,
                size: 0,
                order_index: idx,
                json: entry,
            });
            idx += 1;
        }
        Ok(rows)
    }

    pub(crate) fn collection_entries_json(
        &self,
        c: &CollectionRow,
        group: Option<i64>,
        want: &WantMask,
    ) -> NfsResult<Vec<EntryRow>> {
        let mut rows = Vec::new();
        for (idx, e) in self.db.collection_entries(c.node_id, group)?.iter().enumerate() {
            let entry_ref = format!("ce_{}", e.entry_id);
            let json_entry = if e.node_type == "group" {
                json!({
                    "entry_ref": entry_ref,
                    "name": e.name.clone().unwrap_or_default(),
                    "binding": "member",
                    "target": {
                        "ref": WireRef::live(format!("n_{}", e.target_node_id.unwrap_or(0)), 1),
                        "kind": "group",
                        "attrs": {},
                    },
                    "context": {"type": "collection", "order_index": idx},
                })
            } else {
                let (target, default_name, canonical) = self.member_target_json(
                    e.target_type.as_deref().unwrap_or("live"),
                    e.target_node_id,
                    e.target_obj_id.as_deref(),
                    want,
                )?;
                let mut entry = json!({
                    "entry_ref": entry_ref,
                    "name": e.name.clone().unwrap_or(default_name),
                    "binding": "reference",
                    "target": target,
                    "context": {"type": "collection", "order_index": idx},
                });
                if let Some(cp) = canonical {
                    entry["canonical_path"] = json!(cp);
                }
                entry
            };
            rows.push(EntryRow {
                name: json_entry["name"].as_str().unwrap_or("").to_string(),
                kind: json_entry["target"]["kind"].as_str().unwrap_or("").to_string(),
                mtime: 0,
                size: 0,
                order_index: idx as i64,
                json: json_entry,
            });
        }
        Ok(rows)
    }

    /// Resolves a stored member target into (target_json, display_name,
    /// canonical_path). Dead targets return target_state:"stale" — the entry is
    /// preserved, never silently dropped (NFSP §3.5.1).
    fn member_target_json(
        &self,
        target_type: &str,
        target_node_id: Option<i64>,
        target_obj_id: Option<&str>,
        want: &WantMask,
    ) -> NfsResult<(Value, String, Option<String>)> {
        if target_type == "object" {
            let obj = target_obj_id.unwrap_or_default().to_string();
            return Ok((
                json!({
                    "ref": WireRef::Object { obj_id: obj.clone(), inner_path: None },
                    "kind": "file",
                    "attrs": {},
                }),
                obj,
                None,
            ));
        }
        let id = target_node_id.unwrap_or(0);
        match self.resolve_node_id(id, None) {
            Ok(node) => {
                let mut attrs = serde_json::Map::new();
                if let Node::Native { meta, .. } = &node {
                    if want.has("base") {
                        attrs.insert("size".into(), json!(meta.size));
                        attrs.insert("mtime".into(), json!(meta.mtime));
                        attrs.insert("flags".into(), json!(self.node_flags(&node)));
                    }
                    if want.has("ident") && meta.kind == "file" {
                        attrs
                            .insert("etag".into(), json!(format!("{}-{}", meta.size, meta.mtime)));
                    }
                }
                let canonical = match &node {
                    Node::Native { root, rel, .. } => Some(canonical_path(root, rel)),
                    _ => None,
                };
                Ok((
                    json!({
                        "ref": self.node_ref(&node),
                        "kind": node.kind(),
                        "attrs": Value::Object(attrs),
                    }),
                    node.display_name(),
                    canonical,
                ))
            }
            Err(e) if e.code == ErrorCode::Stale => Ok((
                json!({
                    "ref": WireRef::live(format!("n_{}", id), 0),
                    "kind": Value::Null,
                    "attrs": {},
                    "target_state": "stale",
                }),
                format!("n_{}", id),
                None,
            )),
            Err(e) => Err(e),
        }
    }

    // ---------- ops ----------

    pub fn op_open_view(&self, args: &Value, want: &WantMask) -> NfsResult<Value> {
        let view_id = args["view_id"]
            .as_str()
            .ok_or_else(|| invalid("open_view requires view_id"))?;
        let v = self
            .db
            .view_by_id(view_id)?
            .ok_or_else(|| not_found(format!("view '{}' not found", view_id)))?;
        self.node_info(&Node::View(v), want)
    }

    pub fn op_create_collection(&self, args: &Value, want: &WantMask) -> NfsResult<Value> {
        let title = args["title"]
            .as_str()
            .ok_or_else(|| invalid("create_collection requires title"))?;
        let collection_id = match args["collection_id"].as_str() {
            Some(cid) => {
                if cid.is_empty() || cid.len() > 128 {
                    return Err(invalid("bad collection_id"));
                }
                cid.to_string()
            }
            None => format!("col_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
        };
        let c = self.db.collection_create(&collection_id, title)?;
        Ok(self.node_info(&Node::Collection(c), want)?)
    }

    pub fn op_open_collection(&self, args: &Value, want: &WantMask) -> NfsResult<Value> {
        let cid = args["collection_id"]
            .as_str()
            .ok_or_else(|| invalid("open_collection requires collection_id"))?;
        let c = self
            .db
            .collection_by_id(cid)?
            .ok_or_else(|| not_found(format!("collection '{}' not found", cid)))?;
        self.node_info(&Node::Collection(c), want)
    }

    pub fn op_collection_patch(&self, args: &Value) -> NfsResult<Value> {
        let r: WireRef = serde_json::from_value(args["ref"].clone())
            .map_err(|_| invalid("collection_patch requires ref"))?;
        let node = self.resolve_ref(&r)?;
        let c = match node {
            Node::Collection(c) => c,
            _ => return Err(invalid("ref is not a collection")),
        };
        let expected = match args["expected_revision"].as_str() {
            Some(rev) => Some(parse_generation(rev)?),
            None => None,
        };
        let ops_json = args["ops"]
            .as_array()
            .ok_or_else(|| invalid("collection_patch requires ops[]"))?;
        let mut ops = Vec::new();
        for op in ops_json {
            ops.push(self.decode_collection_op(&c, op)?);
        }
        let new_gen = self.db.collection_patch(c.node_id, expected, &ops)?;
        let revision = format!("g-{}", new_gen);
        let key = ContainerKey::Node(c.node_id);
        self.bus.emit_container_changed(
            &key,
            json!(WireRef::live(format!("n_{}", c.node_id), 1)),
            "collection",
            &revision,
            "entries_changed",
            None,
        );
        Ok(json!({ "revision": revision }))
    }

    fn decode_collection_op(&self, c: &CollectionRow, op: &Value) -> NfsResult<CollectionPatchOp> {
        if let Some(add) = op.get("add_ref") {
            let target_ref: WireRef = serde_json::from_value(add["target_ref"].clone())
                .map_err(|_| invalid("add_ref requires target_ref"))?;
            let position = add["position"].as_u64().map(|p| p as usize);
            let name = add["name"].as_str().map(|s| s.to_string());
            let parent_group = match add["parent_entry_ref"].as_str() {
                Some(er) => Some(self.collection_group_id(c, er)?),
                None => None,
            };
            return Ok(match target_ref {
                WireRef::Object { obj_id, .. } => CollectionPatchOp::AddRef {
                    parent_group,
                    target_type: "object".into(),
                    target_node_id: None,
                    target_obj_id: Some(obj_id),
                    name,
                    position,
                },
                live => {
                    // Anchor native targets so the reference survives renames
                    // (§3.2: 目标列只能保存 Ref, 不得保存 path 作为身份).
                    let node = self.resolve_ref(&live)?;
                    let (node_id, default_name) = self.require_stable_node(&node)?;
                    CollectionPatchOp::AddRef {
                        parent_group,
                        target_type: "live".into(),
                        target_node_id: Some(node_id),
                        target_obj_id: None,
                        name: name.or(Some(default_name)),
                        position,
                    }
                }
            });
        }
        if let Some(rm) = op.get("remove_entry") {
            let er = rm["entry_ref"]
                .as_str()
                .or_else(|| rm.as_str())
                .ok_or_else(|| invalid("remove_entry requires entry_ref"))?;
            return Ok(CollectionPatchOp::RemoveEntry { entry_id: parse_ce(er)? });
        }
        if let Some(mv) = op.get("move_entries") {
            let refs = mv["entry_refs"]
                .as_array()
                .ok_or_else(|| invalid("move_entries requires entry_refs"))?;
            let mut ids = Vec::new();
            for r in refs {
                ids.push(parse_ce(r.as_str().ok_or_else(|| invalid("bad entry_ref"))?)?);
            }
            let to_index = mv["to_index"]
                .as_u64()
                .ok_or_else(|| invalid("move_entries requires to_index"))?
                as usize;
            return Ok(CollectionPatchOp::MoveEntries { entry_ids: ids, to_index });
        }
        if let Some(cg) = op.get("create_group") {
            let name = cg["name"]
                .as_str()
                .ok_or_else(|| invalid("create_group requires name"))?
                .to_string();
            let position = cg["position"].as_u64().map(|p| p as usize);
            return Ok(CollectionPatchOp::CreateGroup { name, position });
        }
        if let Some(rg) = op.get("rename_group") {
            let er = rg["entry_ref"]
                .as_str()
                .ok_or_else(|| invalid("rename_group requires entry_ref"))?;
            let name = rg["name"]
                .as_str()
                .ok_or_else(|| invalid("rename_group requires name"))?
                .to_string();
            return Ok(CollectionPatchOp::RenameGroup { entry_id: parse_ce(er)?, name });
        }
        Err(invalid(format!("unknown collection_patch op: {}", op)))
    }

    fn collection_group_id(&self, c: &CollectionRow, entry_ref: &str) -> NfsResult<i64> {
        let e = self
            .db
            .collection_entry_get(parse_ce(entry_ref)?)?
            .ok_or_else(|| not_found(format!("entry '{}' not found", entry_ref)))?;
        if e.collection_node_id != c.node_id || e.node_type != "group" {
            return Err(invalid(format!("'{}' is not a group of this collection", entry_ref)));
        }
        e.target_node_id.ok_or_else(|| internal("group without node"))
    }

    /// Returns a stable (anchored/virtual) node id for a live ref target,
    /// lazily anchoring native nodes.
    pub fn require_stable_node(&self, node: &Node) -> NfsResult<(i64, String)> {
        match node {
            Node::Native { root, rel, meta, anchored } => {
                if let Some(e) = anchored {
                    return Ok((e.node_id, node.display_name()));
                }
                if rel.is_empty() {
                    return Err(invalid("export roots cannot be reference targets"));
                }
                let kind = if meta.kind == "dir" { "dir" } else { "file" };
                let e = self.db.ensure_anchor(
                    kind,
                    root,
                    rel,
                    meta.id.ino,
                    meta.id.dev,
                    meta.size as i64,
                    meta.mtime,
                )?;
                Ok((e.node_id, node.display_name()))
            }
            Node::View(v) => Ok((v.node_id, v.title.clone())),
            Node::Collection(c) => Ok((c.node_id, c.title.clone())),
            Node::ViewGroup { group, .. } => Ok((group.node_id, group.label.clone())),
            Node::CollectionGroup { entity, name, .. } => Ok((entity.node_id, name.clone())),
            Node::Root => Err(invalid("the namespace root cannot be a reference target")),
        }
    }
}

fn parse_ce(entry_ref: &str) -> NfsResult<i64> {
    entry_ref
        .strip_prefix("ce_")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| invalid(format!("'{}' is not a collection entry_ref", entry_ref)))
}

pub fn parse_generation(revision: &str) -> NfsResult<i64> {
    revision
        .strip_prefix("g-")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            NfsError::new(
                ErrorCode::RevMismatch,
                format!("revision '{}' is not from this container", revision),
            )
        })
}
