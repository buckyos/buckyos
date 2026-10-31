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
    if let Some(s) = &e.schema_id {
        m.insert("schema_id".into(), json!(s));
    }
    m.insert("scope".into(), json!(if e.owner().is_some() { "personal" } else if e.scope == SCOPE_PRESENTER { "presenter" } else if e.participant().is_some() { "participant" } else { "shared" }));
    if let Some(p) = e.participant() {
        m.insert("participant".into(), json!(p));
    }
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
    m.insert("capabilities".into(), json!(access.entity_caps(ctx, e)?.names()));
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
    // a sourced entity (D12): where it comes from and whether it follows or is pinned; content is read-only
    if let Some(s) = e.payload.get("source").filter(|s| s.is_object()) {
        env["source"] = json!({ "kind": s["kind"], "mode": s.get("mode").cloned().unwrap_or(json!("live")), "url": s.get("url").cloned().unwrap_or(Value::Null),
                                "workspace_id": s.get("workspace_id").cloned().unwrap_or(Value::Null), "entity_id": s.get("entity_id").cloned().unwrap_or(Value::Null),
                                "zone": s.get("zone").cloned().unwrap_or(Value::Null), "refresh": s.get("refresh").cloned().unwrap_or(Value::Null), "auth_mode": s.get("auth_mode").cloned().unwrap_or(json!("none")) });
        // a session feed (第四期规划 §5.3): which one
        if let Some(f) = s.get("feed") {
            env["source"]["feed"] = f.clone();
        }
    }
    match e.type_id.as_str() {
        TYPE_CONTAINER => {
            env["kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
            env["layout"] = e.payload.get("layout").cloned().unwrap_or(Value::Null);
            for k in ["system", "surface_id", "content_folder_id", "icon", "locked"] {
                if let Some(v) = e.payload.get(k) {
                    env[k] = v.clone();
                }
            }
            // a control's attributes drive its layout, computed from the outline alone (脑图方案 §5.4); a composite
            // names its definition, and a Block container inside one its slot (扩展控件 P1)
            if env["kind"].as_str().is_some_and(types::is_control_kind) {
                if let Some(c) = e.payload.get("config") {
                    env["config"] = c.clone();
                }
            }
            if let Some(d) = e.payload.get("def_ref").and_then(|r| r.get("entity_id")) {
                env["def_id"] = d.clone();
            }
            if let Some(v) = e.payload.get("slot") {
                env["slot"] = v.clone();
            }
        }
        TYPE_CELL => {
            env["view_type"] = e.payload.get("view").and_then(|v| v.get("type")).cloned().unwrap_or(Value::Null);
            env["view_version"] = e.payload.get("view").and_then(|v| v.get("version")).cloned().unwrap_or(Value::Null);
            env["source_id"] = e.payload.get("source_ref").and_then(|r| r.get("entity_id")).cloned().unwrap_or(Value::Null);
            env["def_id"] = e.payload.get("def_ref").and_then(|r| r.get("entity_id")).cloned().unwrap_or(Value::Null);
            if let Some(v) = e.payload.get("locked") {
                env["locked"] = v.clone();
            }
            // a Block's role inside its control (R22), and the layout hints of a mind map node (its side under a
            // two-sided root, its branch colour): the layout reads the outline only
            if let Some(v) = e.payload.get("slot") {
                env["slot"] = v.clone();
            }
            let config = e.payload.get("config");
            if let Some(side) = config.and_then(|c| c.get("side")).filter(|s| matches!(s.as_str(), Some("left" | "right"))) {
                env["side"] = side.clone();
            }
            if let Some(color) = config.and_then(|c| c.get("branch_color")).filter(|c| types::is_color(c)) {
                env["branch_color"] = color.clone();
            }
            // what a show needs before reading any content: which Blocks may be operated on stage (third phase §10.2)
            if e.payload.get("presentation").and_then(|p| p.get("live")) == Some(&json!(true)) {
                env["live"] = json!(true);
            }
            // the geometry a canvas needs to draw a line (连接线方案 §9.2); appearance stays in `config`
            if types::is_connector(&e.type_id, &e.payload) {
                let mut c = json!({ "start": e.payload.get("start").cloned().unwrap_or(Value::Null),
                                    "end": e.payload.get("end").cloned().unwrap_or(Value::Null) });
                for k in ["flip", "route", "controls", "label"] {
                    if let Some(v) = e.payload.get(k) {
                        c[k] = v.clone();
                    }
                }
                env["connector"] = c;
            }
            // the logical stroke a pen segment belongs to: its segments are selected, erased and copied together
            if let Some(stroke) = types::ink_stroke(&e.payload) {
                env["ink_stroke"] = json!(stroke);
            }
        }
        TYPE_WISH => {
            env["executor"] = e.payload.get("executor").cloned().unwrap_or(Value::Null);
            env["output_mode"] = e.payload.get("output_mode").cloned().unwrap_or(Value::Null);
        }
        TYPE_BLOCK_DEF => {
            env["def_id"] = e.payload.get("def_id").cloned().unwrap_or(Value::Null);
            env["def_kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
            // the control identity, for the catalogue and deduplication (扩展控件 P0)
            if let Some(ident) = e.payload.get("ident").and_then(|i| types::ident_string(i).ok()) {
                env["ident"] = json!(ident);
            }
            for (key, out) in [("manifest_version", "manifest_version"), ("version", "def_version")] {
                if let Some(v) = e.payload.get(key) {
                    env[out] = v.clone();
                }
            }
            if let Some(pkg) = e.payload.get("provenance").and_then(|p| p.get("pkg_id")) {
                env["provenance_pkg"] = pkg.clone();
            }
            if let Some(dir) = e.payload.get("source_dir").and_then(|d| d.get("entity_id")) {
                env["source_dir_id"] = dir.clone();
            }
            // what it builds on (P1, Q16): the identities, for the catalogue and exports
            if let Some(deps) = e.payload.get("deps").and_then(Value::as_array) {
                env["deps"] = json!(deps.iter().filter_map(|d| d.get("ident").cloned()).collect::<Vec<_>>());
            }
            // a composite's instances are laid out from the outline alone: its composition and the defaults its
            // `{{config.<key>}}` placeholders fall back to
            if e.payload.get("kind").and_then(Value::as_str) == Some(types::COMPOSITE) {
                if let Some(c) = e.payload.get("composition") {
                    env["composition"] = c.clone();
                }
                let defaults: Map<String, Value> = e.payload.get("config_schema").and_then(|s| s.get("properties")).and_then(Value::as_object).into_iter().flatten()
                    .filter_map(|(k, p)| p.get("default").map(|d| (k.clone(), d.clone()))).collect();
                env["config_defaults"] = Value::Object(defaults);
            }
        }
        TYPE_ANNOTATION => {
            env["target_id"] = e.payload.get("target").and_then(|t| t.get("entity_id")).cloned().unwrap_or(Value::Null);
            env["annotation_kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
        }
        TYPE_ASSET => {
            env["media_type"] = e.payload.get("media_type").cloned().unwrap_or(Value::Null);
        }
        // enough to draw a Viewport's frame and list the paths; steps, notes and captions are read with `doc.read`
        TYPE_VIEWPORT => {
            env["viewport"] = json!({
                "surface_id": e.payload.get("surface_ref").and_then(|r| r.get("entity_id")).cloned().unwrap_or(Value::Null),
                "center": e.payload.get("center").cloned().unwrap_or(Value::Null),
                "zoom": e.payload.get("zoom").cloned().unwrap_or(Value::Null),
            });
        }
        TYPE_SHOW_PATH => {
            env["show_path"] = json!({
                "purpose": e.payload.get("purpose").cloned().unwrap_or(Value::Null),
                "stage": e.payload.get("stage").cloned().unwrap_or(Value::Null),
                "steps": e.payload.get("steps").and_then(Value::as_array).map_or(0, Vec::len),
            });
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
