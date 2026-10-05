//! NamedObject materialization and package loading (design doc §2.10, §4.4).
//!
//! What gets hashed is content only: no entity id, name, tree position, `rev`,
//! author or time. So renaming or moving an entity keeps its content ObjectId,
//! equal content gives equal ids, and a Fork has the same ContentRoot as its source.

use crate::canonical::{canonical_json, named_object, parse_strict, ObjId, OBJ_TYPE_JSON};
use crate::error::{WsError, WsResult};
use crate::filter::live_fields;
use crate::model::*;
use crate::richtext;
use crate::types;
use crate::value::{reference_is_local, FieldDef, FieldType};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Rich text ASTs above this canonical size move into a file.
pub const INLINE_AST_LIMIT: usize = 256 * 1024;

pub trait ObjectSink {
    /// Store canonical JSON text of a NamedObject; returns its id.
    fn put_object(&mut self, obj_type: &str, canonical: &str) -> WsResult<ObjId>;
    /// Store file bytes; returns the FileObject id.
    fn put_file(&mut self, bytes: &[u8]) -> WsResult<ObjId>;
}

pub trait ObjectSource {
    fn get_object(&self, id: &str) -> WsResult<String>;
    /// Bytes of the file a FileObject id names.
    fn get_file(&self, file_object_id: &str) -> WsResult<Vec<u8>>;
}

fn put_json(sink: &mut dyn ObjectSink, v: &Value) -> WsResult<ObjId> {
    let (id, text) = named_object(OBJ_TYPE_JSON, v)?;
    let stored = sink.put_object(OBJ_TYPE_JSON, &text)?;
    debug_assert_eq!(id, stored);
    Ok(stored)
}

fn jsonl(lines: &[Value]) -> WsResult<Vec<u8>> {
    let mut out = Vec::new();
    for l in lines {
        out.extend_from_slice(canonical_json(l)?.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

fn file_ref(sink: &mut dyn ObjectSink, lines: &[Value]) -> WsResult<Value> {
    let mut v = json!({ "encoding": "jsonl+jcs", "count": lines.len() });
    if !lines.is_empty() {
        v["file"] = json!(sink.put_file(&jsonl(lines)?)?);
    }
    Ok(v)
}

/// The hashed `content` of one entity (design doc §3.x "物化").
pub fn entity_content(ctx: &dyn ReadCtx, e: &EntityRow, sink: &mut dyn ObjectSink) -> WsResult<Value> {
    if e.degraded().is_some() {
        return Ok(Value::Object(e.payload.clone())); // unknown content is carried verbatim
    }
    Ok(match e.type_id.as_str() {
        TYPE_RECORD => types::record_nested(&e.payload),
        TYPE_TABLE => {
            let fields = live_fields(ctx, &e.entity_id)?;
            let live: BTreeSet<&str> = fields.iter().map(|f| f.field_id.as_str()).collect();
            let mut content = e.payload.clone();
            // array order is the default column order; order keys are working state
            content.insert("fields".into(), Value::Array(fields.iter().map(|f| f.def.to_value()).collect()));
            let mut lines = Vec::new();
            ctx.scan_records(&e.entity_id, &mut |r| {
                if r.alive() {
                    let keep = |m: &JsonMap| -> JsonMap {
                        m.iter().filter(|(k, _)| live.contains(k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
                    };
                    let mut line = json!({ "record_id": r.record_id, "values": keep(&r.values) });
                    let meta = keep(&r.meta);
                    if !meta.is_empty() {
                        line["meta"] = Value::Object(meta);
                    }
                    if let Some(b) = &r.body_entity_id {
                        line["body_ref"] = json!({ "entity_id": b, "version": { "mode": "live_head" } });
                    }
                    lines.push(line);
                }
                Ok(true)
            })?;
            content.insert("records".into(), file_ref(sink, &lines)?);
            Value::Object(content)
        }
        TYPE_RICHTEXT => {
            let rt = ctx.richtext(&e.entity_id)?.ok_or_else(|| WsError::io("rich text state missing"))?;
            let text = canonical_json(&rt.meta.ast)?;
            let doc = if text.len() > INLINE_AST_LIMIT {
                json!({ "encoding": "json+jcs", "file": sink.put_file(text.as_bytes())? })
            } else {
                rt.meta.ast.clone()
            };
            json!({ "editor_schema": richtext::EDITOR_SCHEMA, "doc": doc })
        }
        _ => Value::Object(e.payload.clone()),
    })
}

pub fn content_object(ctx: &dyn ReadCtx, e: &EntityRow, sink: &mut dyn ObjectSink) -> WsResult<ObjId> {
    let content = entity_content(ctx, e, sink)?;
    put_json(sink, &json!({ "ws_type": e.type_id, "schema_version": e.schema_version, "content": content }))
}

#[derive(Debug, Clone)]
pub struct Materialized {
    pub content_root: ObjId,
    /// `entity_id → content ObjectId` of everything included.
    pub objects: BTreeMap<String, ObjId>,
    /// Entities left out because the caller may not read them.
    pub excluded: usize,
    /// URL query sources and cross-Workspace live references: not fixed by this snapshot.
    pub unresolved: Vec<Value>,
}

/// All alive shared entities in parent-before-child order.
pub fn shared_entities(ctx: &dyn ReadCtx) -> WsResult<Vec<(EntityRow, Option<TreeEdge>)>> {
    fn rec(ctx: &dyn ReadCtx, id: &str, out: &mut Vec<(EntityRow, Option<TreeEdge>)>) -> WsResult<()> {
        for edge in ctx.children(id)? {
            if let Some(e) = ctx.entity(&edge.child_id)? {
                if e.alive() && e.scope == SCOPE_SHARED {
                    let cid = edge.child_id.clone();
                    out.push((e, Some(edge)));
                    rec(ctx, &cid, out)?;
                }
            }
        }
        Ok(())
    }
    let root = ctx.entity(ROOT_ID)?.ok_or_else(|| WsError::io("workspace has no root"))?;
    let mut out = vec![(root, None)];
    rec(ctx, ROOT_ID, &mut out)?;
    Ok(out)
}

/// Materialize the current shared content. `visible` filters by read permission
/// (a filtered entity takes its subtree with it); `cached` short-circuits
/// entities whose `(entity_id, content_rev)` was materialized before.
pub fn materialize(
    ctx: &dyn ReadCtx,
    sink: &mut dyn ObjectSink,
    visible: &dyn Fn(&EntityRow) -> WsResult<bool>,
    cached: &mut dyn FnMut(&EntityRow, Option<&ObjId>) -> Option<ObjId>,
) -> WsResult<Materialized> {
    let mut included: BTreeSet<String> = BTreeSet::new();
    let (mut lines, mut objects, mut unresolved, mut excluded) = (Vec::new(), BTreeMap::new(), Vec::new(), 0);
    for (e, edge) in shared_entities(ctx)? {
        let parent_ok = edge.as_ref().map_or(true, |x| included.contains(&x.parent_id));
        if !parent_ok || !visible(&e)? {
            excluded += 1;
            continue;
        }
        included.insert(e.entity_id.clone());
        let object_id = match cached(&e, None) {
            Some(id) => id,
            None => {
                let id = content_object(ctx, &e, sink)?;
                cached(&e, Some(&id));
                id
            }
        };
        let mut line = Map::new();
        line.insert("entity_id".into(), json!(e.entity_id));
        line.insert("type_id".into(), json!(e.type_id));
        line.insert("schema_version".into(), json!(e.schema_version));
        if let Some(n) = &e.name {
            line.insert("name".into(), json!(n));
        }
        if let Some(edge) = &edge {
            line.insert("parent_id".into(), json!(edge.parent_id));
            line.insert("order_key".into(), json!(edge.order_key));
            if let Some(p) = &edge.placement {
                line.insert("placement".into(), p.clone());
            }
        }
        if e.write_policy != POLICY_OPEN {
            line.insert("write_policy".into(), json!(e.write_policy));
        }
        line.insert("object_id".into(), json!(object_id));
        lines.push(Value::Object(line));
        objects.insert(e.entity_id.clone(), object_id);
        for r in ctx.refs_from(&e.entity_id)? {
            if !r.dst_query_json.is_empty() {
                unresolved.push(json!({ "from": e.entity_id, "source_ref": parse_strict(&r.dst_query_json)? }));
            } else if !r.dst_workspace_id.is_empty() && r.dst_object_id.is_empty() {
                unresolved.push(json!({ "from": e.entity_id, "to": { "workspace_id": r.dst_workspace_id, "entity_id": r.dst_entity_id } }));
            }
        }
    }
    lines.sort_by(|a, b| a["entity_id"].as_str().cmp(&b["entity_id"].as_str()));
    unresolved.sort_by_key(|v| v.to_string());
    unresolved.dedup();
    let root = json!({
        "ws_type": "buckyos.workspace-content",
        "format_version": FORMAT_VERSION,
        "root_entity_id": ROOT_ID,
        "entities": file_ref(sink, &lines)?,
        "resolved_refs": [],
        "unresolved_refs": unresolved,
    });
    Ok(Materialized { content_root: put_json(sink, &root)?, objects, excluded, unresolved })
}

pub fn snapshot_root(workspace_id: &str, seq: u64, commit_id: &str, content_root: &str, forked_from: Option<&Value>) -> Value {
    let mut v = json!({ "ws_type": "buckyos.workspace-snapshot", "format_version": FORMAT_VERSION,
                        "workspace_id": workspace_id, "seq": seq, "commit_id": commit_id, "content": content_root });
    if let Some(f) = forked_from {
        v["lineage"] = json!({ "forked_from": f });
    }
    v
}

fn parse_lines(bytes: &[u8]) -> WsResult<Vec<Value>> {
    let text = std::str::from_utf8(bytes).map_err(|_| WsError::invalid_schema("file is not UTF-8"))?;
    text.lines().filter(|l| !l.is_empty()).map(parse_strict).collect()
}

fn read_file_ref(src: &dyn ObjectSource, v: &Value) -> WsResult<Vec<Value>> {
    if v["encoding"] != json!("jsonl+jcs") {
        return Err(WsError::invalid_schema("unsupported file encoding"));
    }
    let lines = match v.get("file").and_then(Value::as_str) {
        Some(f) => parse_lines(&src.get_file(f)?)?,
        None => vec![],
    };
    if v["count"].as_u64() != Some(lines.len() as u64) {
        return Err(WsError::invalid_schema("file line count does not match"));
    }
    Ok(lines)
}

pub struct LoadPlan {
    /// Operations that rebuild the content as one `origin: "import"` commit.
    pub ops: Vec<Value>,
    pub entity_count: usize,
    /// `type_id → schema_version` seen in the package.
    pub types: BTreeMap<String, u32>,
    pub asset_objects: Vec<ObjId>,
}

/// Turn a ContentRoot into the operation batch that recreates it. The batch
/// runs through the ordinary planner (import mode), so every content object
/// passes the same schema validation as any other write. `collab` supplies
/// original CRDT snapshots for a personal-backup restore.
pub fn load_ops(src: &dyn ObjectSource, content_root: &str, collab: &dyn Fn(&str) -> Option<(String, Vec<u8>)>) -> WsResult<LoadPlan> {
    use base64::Engine;
    let root = parse_strict(&src.get_object(content_root)?)?;
    if root["ws_type"] != json!("buckyos.workspace-content") {
        return Err(WsError::invalid_schema("not a workspace content root"));
    }
    if root["format_version"] != json!(FORMAT_VERSION) {
        return Err(WsError::new(crate::Code::UnsupportedVersion, format!("format_version {} not supported", root["format_version"])));
    }
    let entries = read_file_ref(src, &root["entities"])?;
    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for e in &entries {
        let id = e["entity_id"].as_str().ok_or_else(|| WsError::invalid_schema("entity entry without id"))?;
        if by_id.insert(id.to_string(), e.clone()).is_some() {
            return Err(WsError::invalid_schema(format!("duplicate entity {id}")));
        }
    }
    if !by_id.contains_key(ROOT_ID) {
        return Err(WsError::invalid_schema("package has no root entity"));
    }
    // structural checks: every parent exists, no cycles
    let mut depth: BTreeMap<String, usize> = BTreeMap::new();
    for id in by_id.keys() {
        let (mut cur, mut d) = (id.clone(), 0usize);
        while cur != ROOT_ID {
            let parent = by_id[&cur]["parent_id"].as_str().ok_or_else(|| WsError::invalid_schema(format!("entity {cur} has no parent")))?;
            if !by_id.contains_key(parent) {
                return Err(WsError::invalid_schema(format!("entity {cur} has a dangling parent {parent}")));
            }
            cur = parent.to_string();
            d += 1;
            if d > by_id.len() {
                return Err(WsError::invalid_schema("cycle in the structural tree"));
            }
        }
        depth.insert(id.clone(), d);
    }
    // creation order: containers by depth, then data, then things that reference data
    let rank = |t: &str| match t {
        TYPE_CONTAINER => 0,
        TYPE_TABLE => 1,
        TYPE_RICHTEXT => 2,
        TYPE_RECORD | TYPE_ASSET => 3,
        TYPE_CELL => 5,
        TYPE_ANNOTATION => 6,
        _ => 4,
    };
    let mut order: Vec<&String> = by_id.keys().filter(|id| id.as_str() != ROOT_ID).collect();
    order.sort_by_key(|id| (rank(by_id[*id]["type_id"].as_str().unwrap_or("")), depth[*id], (*id).clone()));
    let (mut ops, mut inserts, mut later, mut types, mut assets) = (Vec::new(), Vec::new(), Vec::new(), BTreeMap::new(), Vec::new());
    for id in order {
        let entry = &by_id[id];
        let type_id = entry["type_id"].as_str().unwrap_or("");
        let object = parse_strict(&src.get_object(entry["object_id"].as_str().unwrap_or(""))?)?;
        if object["ws_type"] != entry["type_id"] || object["schema_version"] != entry["schema_version"] {
            return Err(WsError::invalid_schema(format!("content object of {id} does not match its entry")));
        }
        let schema_version = entry["schema_version"].as_u64().unwrap_or(0) as u32;
        types.insert(type_id.to_string(), schema_version);
        let known = is_known_type(type_id) && schema_version == 1;
        let mut content = object["content"].as_object().cloned().unwrap_or_default();
        let mut payload = content.clone();
        if known {
            match type_id {
                TYPE_TABLE => {
                    let records = content.remove("records").unwrap_or(Value::Null);
                    payload.remove("records");
                    let lines = if records.is_null() { vec![] } else { read_file_ref(src, &records)? };
                    for chunk in lines.chunks(5000) {
                        inserts.push(json!({ "op": "table.insert_records", "source_id": id, "records": chunk }));
                    }
                }
                TYPE_RICHTEXT => {
                    payload = Map::new();
                    match collab(id) {
                        Some((lineage, bytes)) => {
                            payload.insert("lineage_id".into(), json!(lineage));
                            payload.insert("snapshot".into(), json!(base64::engine::general_purpose::STANDARD.encode(bytes)));
                        }
                        None => {
                            let doc = &content["doc"];
                            let ast = match doc.get("file").and_then(Value::as_str) {
                                Some(f) if doc.get("encoding").is_some() => {
                                    parse_strict(std::str::from_utf8(&src.get_file(f)?).map_err(|_| WsError::invalid_schema("ast file is not UTF-8"))?)?
                                }
                                _ => doc.clone(),
                            };
                            // a plain import builds a new lineage: no CRDT history travels with a share package
                            payload.insert("content".into(), ast);
                        }
                    }
                }
                TYPE_ASSET => {
                    if let Some(o) = content.get("object_id").and_then(Value::as_str) {
                        assets.push(o.to_string());
                    }
                }
                _ => {}
            }
        }
        let mut op = json!({ "op": "entity.create", "entity_id": id, "type_id": type_id, "schema_version": schema_version,
                             "parent_id": entry["parent_id"], "order_key": entry["order_key"], "payload": payload });
        for k in ["name", "placement"] {
            if let Some(v) = entry.get(k) {
                op[k] = v.clone();
            }
        }
        ops.push(op);
        if let Some(policy) = entry.get("write_policy") {
            later.push(json!({ "op": "entity.set_write_policy", "entity_id": id, "policy": policy, "expect": "any" }));
        }
    }
    // the root's own keys (layout/title) and policy
    let root_entry = &by_id[ROOT_ID];
    let root_obj = parse_strict(&src.get_object(root_entry["object_id"].as_str().unwrap_or(""))?)?;
    let keys: Vec<Value> = root_obj["content"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| k.as_str() != "kind")
        .map(|(k, v)| json!({ "key": k, "value": v, "expect": "any" }))
        .collect();
    if !keys.is_empty() {
        later.push(json!({ "op": "entity.set_keys", "entity_id": ROOT_ID, "keys": keys }));
    }
    let entity_count = by_id.len();
    ops.extend(inserts);
    ops.extend(later);
    Ok(LoadPlan { ops, entity_count, types, asset_objects: assets })
}

/// Rebuild the reference index from scratch (design doc §2.4): the index is
/// derived data and must equal what incremental maintenance produced.
pub fn rebuild_refs(ctx: &dyn ReadCtx, entity_ids: &[String]) -> WsResult<BTreeSet<RefEdge>> {
    let mut out = BTreeSet::new();
    for id in entity_ids {
        let Some(e) = ctx.entity(id)? else { continue };
        out.extend(types::entity_refs(&e)?);
        match e.type_id.as_str() {
            TYPE_TABLE => {
                let refs: Vec<FieldDef> =
                    live_fields(ctx, id)?.into_iter().map(|f| f.def).filter(|d| d.ty == FieldType::ObjectRef).collect();
                ctx.scan_records(id, &mut |r| {
                    if r.alive() {
                        for d in &refs {
                            if let Some(v) = r.values.get(&d.field_id).filter(|v| v.is_object()) {
                                out.insert(types::value_ref_edge(id, &r.record_id, &d.field_id, v));
                            }
                        }
                        if let Some(b) = &r.body_entity_id {
                            out.insert(types::body_ref_edge(id, &r.record_id, b));
                        }
                    }
                    Ok(true)
                })?;
            }
            TYPE_RICHTEXT => {
                if let Some(rt) = ctx.richtext(id)? {
                    let v = richtext::validate_ast(&rt.meta.ast, &richtext::Limits::default())?;
                    for r in v.refs {
                        out.insert(types::richtext_ref_edge(id, &r.block_id, &r.reference));
                    }
                }
            }
            _ => {}
        }
    }
    let _ = reference_is_local;
    Ok(out)
}
