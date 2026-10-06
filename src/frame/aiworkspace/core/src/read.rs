//! Authorized reads (design doc §2.5.4, §2.9). Every writable unit in a result
//! carries its `rev`, so clients can build `expect` from what they read.

use crate::access::{Access, Cap};
use crate::anchor;
use crate::error::{Code, WsError, WsResult};
use crate::filter::live_fields;
use crate::model::*;
use crate::richtext;
use crate::types;
use crate::value::FieldDef;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Look an entity up for reading. Without `read`: `PERMISSION_DENIED` if the
/// caller can see the parent (so it already knows the entity exists),
/// otherwise `NOT_FOUND` — existence is not leaked.
pub fn readable_entity(ctx: &dyn ReadCtx, access: &Access, id: &str) -> WsResult<EntityRow> {
    let not_found = || WsError::not_found(format!("entity {id} not found"));
    let e = ctx.entity(id)?.filter(|e| access.sees_scope(e)).ok_or_else(not_found)?;
    if !access.can(ctx, id, Cap::Read)? {
        let parent_readable = match ctx.edge(id)? {
            Some(edge) => access.can(ctx, &edge.parent_id, Cap::Read)?,
            None => false,
        };
        return Err(if parent_readable { WsError::denied("read capability required") } else { not_found() });
    }
    Ok(e)
}

/// The common envelope: identity, type, versions and tree position. No content.
pub fn envelope(ctx: &dyn ReadCtx, access: &Access, e: &EntityRow) -> WsResult<Value> {
    let mut m = Map::new();
    m.insert("entity_id".into(), json!(e.entity_id));
    m.insert("type_id".into(), json!(e.type_id));
    m.insert("schema_version".into(), json!(e.schema_version));
    m.insert("scope".into(), json!(if e.owner().is_some() { "personal" } else { "shared" }));
    m.insert("name".into(), json!(e.name));
    m.insert("write_policy".into(), json!(e.write_policy));
    m.insert("content_rev".into(), json!(e.content_rev));
    m.insert("meta_rev".into(), json!(e.meta_rev));
    m.insert("life_rev".into(), json!(e.life_rev));
    m.insert("deleted".into(), json!(!e.alive()));
    if let Some(d) = e.degraded() {
        m.insert("degraded".into(), json!(d));
    }
    if let Some(d) = &e.derived {
        m.insert("derived".into(), d.clone());
    }
    if let Some(edge) = ctx.edge(&e.entity_id)? {
        m.insert("parent_id".into(), json!(edge.parent_id));
        m.insert("order_key".into(), json!(edge.order_key));
        m.insert("struct_rev".into(), json!(edge.struct_rev));
        if let Some(p) = edge.placement {
            m.insert("placement".into(), p);
        }
    }
    m.insert("capabilities".into(), json!(access.caps(ctx, &e.entity_id)?.names()));
    Ok(Value::Object(m))
}

fn field_json(f: &FieldRow) -> Value {
    let mut v = f.def.to_value();
    v["order_key"] = json!(f.order_key);
    v["def_rev"] = json!(f.def_rev);
    v["type_rev"] = json!(f.type_rev);
    v["values_rev"] = json!(f.values_rev);
    v
}

/// Diagnostics for view configuration that references deleted fields/options.
/// The configuration itself is never rewritten by the backend.
pub fn cell_diagnostics(ctx: &dyn ReadCtx, cell: &EntityRow) -> WsResult<Vec<Value>> {
    let mut out = Vec::new();
    if cell.payload.get("view").and_then(|v| v["type"].as_str()) != Some("table") {
        return Ok(out);
    }
    let Some(source_id) = cell.payload.get("source_ref").and_then(crate::value::reference_entity_id) else { return Ok(out) };
    match ctx.entity(source_id)? {
        Some(s) if s.alive() => {}
        _ => {
            out.push(json!({ "key": "source_ref", "code": "SOURCE_DELETED" }));
            return Ok(out);
        }
    }
    let live: BTreeMap<String, FieldDef> = live_fields(ctx, source_id)?.into_iter().map(|f| (f.field_id, f.def)).collect();
    if let Some(f) = cell.payload.get("filter").filter(|v| !v.is_null()) {
        if let Err(e) = crate::filter::compile(f, &live) {
            let mut d = e.data.clone().unwrap_or_else(|| json!({ "code": "INVALID" }));
            d["key"] = json!("filter");
            out.push(d);
        }
    }
    let mut missing = |key: &str, id: Option<&str>| {
        if let Some(id) = id.filter(|id| !live.contains_key(*id)) {
            out.push(json!({ "key": key, "code": "FIELD_DELETED", "field_id": id }));
        }
    };
    for f in cell.payload.get("fields").and_then(Value::as_array).into_iter().flatten() {
        missing("fields", f["field_id"].as_str());
    }
    for s in cell.payload.get("sorts").and_then(Value::as_array).into_iter().flatten() {
        missing("sorts", s["field_id"].as_str());
    }
    missing("group", cell.payload.get("group").and_then(|g| g["field_id"].as_str()));
    Ok(out)
}

fn record_json(r: &RecordRow, live: &BTreeMap<String, FieldDef>) -> Value {
    let keep = |k: &String| live.contains_key(k);
    let values: JsonMap = r.values.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let revs: JsonMap = r.revs.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k.clone(), json!(v))).collect();
    let meta: JsonMap = r.meta.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut out = json!({ "record_id": r.record_id, "rev": r.rev, "values": values, "revs": revs, "meta": meta });
    if let Some(b) = &r.body_entity_id {
        out["body_ref"] = json!({ "entity_id": b, "version": { "mode": "live_head" } });
    }
    out
}

/// Content of an entity or of a part of it named by `selector`.
pub fn read(ctx: &dyn ReadCtx, access: &Access, entity_id: &str, selector: Option<&Value>) -> WsResult<Value> {
    let e = readable_entity(ctx, access, entity_id)?;
    if !e.alive() {
        return Err(WsError::deleted(format!("entity {entity_id} is deleted")));
    }
    let mut out = envelope(ctx, access, &e)?;
    let kind = selector.and_then(|s| s.get("kind")).and_then(Value::as_str).unwrap_or("entity");
    let s = selector.cloned().unwrap_or(Value::Null);
    let need = |key: &str| s.get(key).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("selector.{key} required")));
    let content = match kind {
        "entity" => entity_content(ctx, &e)?,
        "doc_key" => {
            let key = need("key")?;
            json!({ "key": key, "value": e.payload.get(key), "rev": e.key_rev(key) })
        }
        "table_field" | "table_record" | "table_cell" if e.type_id == TYPE_TABLE => {
            let live: BTreeMap<String, FieldDef> = live_fields(ctx, entity_id)?.into_iter().map(|f| (f.field_id, f.def)).collect();
            let field = |id: &str| match ctx.field(entity_id, id)? {
                Some(f) if f.alive() => Ok(f),
                Some(_) => Err(WsError::deleted(format!("field {id} is deleted"))),
                None => Err(WsError::not_found(format!("field {id} not found"))),
            };
            let record = |id: &str| match ctx.record(entity_id, id)? {
                Some(r) if r.alive() => Ok(r),
                Some(_) => Err(WsError::deleted(format!("record {id} is deleted"))),
                None => Err(WsError::not_found(format!("record {id} not found"))),
            };
            match kind {
                "table_field" => field_json(&field(need("field_id")?)?),
                "table_record" => record_json(&record(need("record_id")?)?, &live),
                _ => {
                    let f = field(need("field_id")?)?;
                    let r = record(need("record_id")?)?;
                    json!({ "record_id": r.record_id, "field_id": f.field_id, "value": r.values.get(&f.field_id),
                            "is_set": r.values.contains_key(&f.field_id), "rev": r.value_rev(&f.field_id),
                            "meta": r.meta.get(&f.field_id), "type_rev": f.type_rev })
                }
            }
        }
        "richtext_block" if e.type_id == TYPE_RICHTEXT => {
            let rt = ctx.richtext(entity_id)?.ok_or_else(|| WsError::io("rich text state missing"))?;
            let b = need("block_id")?;
            let info = rt.meta.block_index.get(b).ok_or_else(|| WsError::deleted(format!("block {b} not found")))?;
            let loc = richtext::index_blocks(&rt.meta.ast).remove(b);
            json!({ "block_id": b, "hash": info.hash, "struct_rev": info.struct_rev,
                    "node": loc.as_ref().map(|l| l.node.clone()), "position": loc.as_ref().map(|l| richtext::position_of(l).to_json()) })
        }
        "table_field" | "table_record" | "table_cell" | "richtext_block" => {
            return Err(WsError::invalid_op(format!("selector {kind} does not apply to {}", e.type_id)))
        }
        // unknown kinds are kept verbatim wherever they are stored; resolving them needs the extension
        _ => return Err(WsError::new(Code::MissingExtension, format!("unknown selector kind {kind}"))),
    };
    out["content"] = content;
    Ok(out)
}

fn entity_content(ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<Value> {
    let revs = json!(e.key_revs.iter().filter(|(k, _)| !k.starts_with('#')).collect::<BTreeMap<_, _>>());
    if e.degraded().is_some() {
        // unknown extension or newer schema: the raw payload, read-only (V20)
        return Ok(json!({ "payload": e.payload, "key_revs": revs }));
    }
    Ok(match e.type_id.as_str() {
        TYPE_RECORD => {
            let mut v = types::record_nested(&e.payload);
            v["key_revs"] = revs;
            v
        }
        TYPE_CELL => json!({ "payload": e.payload, "key_revs": revs, "diagnostics": cell_diagnostics(ctx, e)? }),
        TYPE_ANNOTATION => json!({ "payload": e.payload, "key_revs": revs, "anchor": anchor::resolve(ctx, &e.payload)?.to_json() }),
        TYPE_TABLE => {
            let mut count = 0u64;
            ctx.scan_records(&e.entity_id, &mut |r| {
                count += r.alive() as u64;
                Ok(true)
            })?;
            json!({ "payload": e.payload, "key_revs": revs,
                    "fields": live_fields(ctx, &e.entity_id)?.iter().map(field_json).collect::<Vec<_>>(),
                    "record_count": count, "members_rev": e.key_rev(MEMBERS_KEY) })
        }
        TYPE_RICHTEXT => {
            let rt = ctx.richtext(&e.entity_id)?.ok_or_else(|| WsError::io("rich text state missing"))?;
            let blocks: BTreeMap<&String, Value> =
                rt.meta.block_index.iter().map(|(k, b)| (k, json!({ "hash": b.hash, "struct_rev": b.struct_rev }))).collect();
            json!({ "editor_schema": richtext::EDITOR_SCHEMA, "content": rt.meta.ast, "blocks": blocks, "lineage_id": rt.meta.lineage_id })
        }
        _ => json!({ "payload": e.payload, "key_revs": revs }),
    })
}

/// Readable annotations anchored to any of `targets` or placed under `parent_id`, in creation
/// order, each with its content as `read` returns it (incl. `anchor`).
pub fn list_annotations(ctx: &dyn ReadCtx, access: &Access, targets: &[String], parent_id: Option<&str>) -> WsResult<Vec<Value>> {
    let mut ids = BTreeSet::new();
    for t in targets {
        ids.extend(ctx.refs_to(t)?.into_iter().filter(|r| r.kind == "anchor").map(|r| r.src_entity_id));
    }
    if let Some(parent) = parent_id {
        readable_entity(ctx, access, parent)?;
        ids.extend(ctx.children(parent)?.into_iter().map(|edge| edge.child_id));
    }
    let mut rows = Vec::new();
    for id in ids {
        if let Some(e) = ctx.entity(&id)? {
            if e.type_id == TYPE_ANNOTATION && e.alive() && access.can_read(ctx, &e)? {
                rows.push(e);
            }
        }
    }
    rows.sort_by(|a, b| (a.created_seq, &a.entity_id).cmp(&(b.created_seq, &b.entity_id)));
    rows.iter()
        .map(|e| {
            let mut v = envelope(ctx, access, e)?;
            v["content"] = entity_content(ctx, e)?;
            Ok(v)
        })
        .collect()
}

/// Outline decoration shared by the service and the replica: enough of the content to draw the two
/// trees without N further reads (container kind / layout / system role, a Block's renderer, source
/// and definition, a wish's executor).
pub fn outline_extras(env: &mut Value, e: &EntityRow) {
    env["title"] = e.payload.get("title").cloned().unwrap_or(Value::Null);
    match e.type_id.as_str() {
        TYPE_CONTAINER => {
            env["kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
            env["layout"] = e.payload.get("layout").cloned().unwrap_or(Value::Null);
            for k in ["system", "surface_id", "content_folder_id"] {
                if let Some(v) = e.payload.get(k) {
                    env[k] = v.clone();
                }
            }
        }
        TYPE_CELL => {
            env["view_type"] = e.payload.get("view").and_then(|v| v.get("type")).cloned().unwrap_or(Value::Null);
            env["view_version"] = e.payload.get("view").and_then(|v| v.get("version")).cloned().unwrap_or(Value::Null);
            env["source_id"] = e.payload.get("source_ref").and_then(|r| r.get("entity_id")).cloned().unwrap_or(Value::Null);
            env["def_id"] = e.payload.get("def_ref").and_then(|r| r.get("entity_id")).cloned().unwrap_or(Value::Null);
        }
        TYPE_WISH => {
            env["executor"] = e.payload.get("executor").cloned().unwrap_or(Value::Null);
            env["output_mode"] = e.payload.get("output_mode").cloned().unwrap_or(Value::Null);
        }
        TYPE_BLOCK_DEF => {
            env["def_id"] = e.payload.get("def_id").cloned().unwrap_or(Value::Null);
            env["def_kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
        }
        TYPE_ANNOTATION => {
            env["target_id"] = e.payload.get("target").and_then(|t| t.get("entity_id")).cloned().unwrap_or(Value::Null);
            env["annotation_kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
        }
        TYPE_ASSET => {
            env["media_type"] = e.payload.get("media_type").cloned().unwrap_or(Value::Null);
        }
        _ => {}
    }
}

/// All readable, alive entities in tree order (the "outline"), decorated with `outline_extras`.
pub fn outline(ctx: &dyn ReadCtx, access: &Access) -> WsResult<Vec<Value>> {
    let mut out = Vec::new();
    let mut stack = vec![ROOT_ID.to_string()];
    while let Some(id) = stack.pop() {
        let Some(e) = ctx.entity(&id)? else { continue };
        if !e.alive() || !access.can_read(ctx, &e)? {
            continue;
        }
        let mut env = envelope(ctx, access, &e)?;
        outline_extras(&mut env, &e);
        out.push(env);
        for edge in ctx.children(&id)?.into_iter().rev() {
            stack.push(edge.child_id);
        }
    }
    Ok(out)
}

/// Child envelopes (no content) of a container, in sibling order.
pub fn list_children(ctx: &dyn ReadCtx, access: &Access, parent_id: &str, include_deleted: bool) -> WsResult<Vec<Value>> {
    readable_entity(ctx, access, parent_id)?;
    let mut out = Vec::new();
    for edge in ctx.children(parent_id)? {
        if let Some(e) = ctx.entity(&edge.child_id)? {
            if (e.alive() || include_deleted) && access.can_read(ctx, &e)? {
                out.push(envelope(ctx, access, &e)?);
            }
        }
    }
    Ok(out)
}

/// `/data/entity name` → entity id (the system nodes are named `data` / `surfaces`). Paths are for
/// lookup only; anything persistent stores the id.
pub fn resolve_path(ctx: &dyn ReadCtx, access: &Access, path: &str) -> WsResult<String> {
    let mut cur = ROOT_ID.to_string();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        let mut next = None;
        for edge in ctx.children(&cur)? {
            if let Some(e) = ctx.entity(&edge.child_id)? {
                if e.alive() && e.name.as_deref() == Some(seg) && access.can_read(ctx, &e)? {
                    next = Some(e.entity_id);
                    break;
                }
            }
        }
        cur = next.ok_or_else(|| WsError::not_found(format!("path segment {seg:?} not found")))?;
    }
    Ok(cur)
}
