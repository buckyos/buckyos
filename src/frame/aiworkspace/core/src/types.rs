//! Built-in keyed-document types (design doc §3.1, §3.2, §3.5–§3.7): payload
//! validation, normalization and the references each payload holds. Concurrency
//! is not implemented here — every type shares the keyed-document version cells.

use crate::access::Cap;
use crate::error::{Code, WsError, WsResult};
use crate::filter;
use crate::id::is_valid_id;
use crate::model::*;
use crate::order_key::{check_order_key, order_key_between};
use crate::plan::{selector_string, Planner};
use crate::value::{normalize_reference, reference_entity_id, reference_is_local, FieldDef, FieldType};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn change_class(type_id: &str) -> &'static str {
    match type_id {
        TYPE_CELL | TYPE_CONTAINER => "view",
        TYPE_TABLE => "schema",
        _ => "value",
    }
}

/// `^[a-z0-9][a-z0-9._-]{0,63}$`: Renderer / Block definition identifiers (D6: the backend checks the
/// format only; whether a Renderer supports a data type is the front-end registry's judgement).
pub fn is_renderer_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

fn canonical_len(v: &Value) -> usize {
    crate::canonical::canonical_json(v).map(|s| s.len()).unwrap_or(usize::MAX)
}

pub fn annotation_author(e: &EntityRow) -> Option<&str> {
    e.payload.get("author").and_then(Value::as_str)
}

fn bad(detail: impl Into<String>) -> WsError {
    WsError::invalid_schema(detail)
}

fn check_keys(payload: &JsonMap, allowed: &[&str], what: &str) -> WsResult<()> {
    for k in payload.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(bad(format!("{what}: unknown key {k}")));
        }
    }
    Ok(())
}

fn opt_text(payload: &JsonMap, key: &str, max: usize) -> WsResult<()> {
    match payload.get(key) {
        None => Ok(()),
        Some(Value::String(s)) if s.chars().count() <= max => Ok(()),
        Some(_) => Err(bad(format!("{key} must be a string of at most {max} chars"))),
    }
}

// ---- RecordObject ----

/// `schema.properties[]` uses `key` where table fields use `field_id`.
pub fn record_schema(payload: &JsonMap) -> WsResult<BTreeMap<String, FieldDef>> {
    let props = payload
        .get("schema")
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_array)
        .ok_or_else(|| bad("record needs schema.properties"))?;
    if payload["schema"].as_object().map_or(0, |o| o.len()) != 1 {
        return Err(bad("record schema has unknown keys"));
    }
    let mut out = BTreeMap::new();
    for prop in props {
        let mut o = prop.as_object().cloned().ok_or_else(|| bad("property must be an object"))?;
        let key = o.remove("key").ok_or_else(|| bad("property needs key"))?;
        o.insert("field_id".into(), key);
        let def = FieldDef::from_value(&Value::Object(o))?;
        if def.unique {
            return Err(bad("unique is not supported on record properties"));
        }
        if out.insert(def.field_id.clone(), def).is_some() {
            return Err(bad("duplicate property key"));
        }
    }
    Ok(out)
}

fn validate_record(p: &Planner, e: &mut EntityRow, changed: Option<&[String]>) -> WsResult<()> {
    let schema = record_schema(&e.payload)?;
    let keys: Vec<String> = e.payload.keys().cloned().collect();
    for k in keys {
        if k == "schema" {
            continue;
        }
        let Some(prop) = k.strip_prefix("p:") else { return Err(bad(format!("record: unknown key {k}"))) };
        let schema_changed = changed.map_or(true, |c| c.iter().any(|x| x == "schema"));
        let Some(def) = schema.get(prop) else {
            return Err(if schema_changed && changed.is_some_and(|c| !c.contains(&k)) {
                bad(format!("property {prop} was removed from the schema; unset p:{prop} in the same operation"))
            } else {
                bad(format!("property {prop} is not declared in the schema"))
            });
        };
        // re-validate values touched now, and all values whenever the schema itself changed
        if schema_changed || changed.map_or(true, |c| c.contains(&k)) {
            let v = def.normalize(&e.payload[&k]).map_err(|err| {
                if changed.is_some_and(|c| !c.contains(&k)) {
                    WsError::new(Code::SchemaConflict, format!("existing value of {prop} is invalid under the new schema: {}", err.detail))
                } else {
                    err
                }
            })?;
            if def.ty == FieldType::ObjectRef && !v.is_null() {
                p.check_ref_target(&v, def.target_types.as_ref().map(|t| t.iter().map(String::as_str).collect::<Vec<_>>()).as_deref())?;
            }
            e.payload.insert(k, v);
        }
    }
    for (key, def) in &schema {
        if def.required && !e.payload.contains_key(&format!("p:{key}")) {
            return Err(bad(format!("property {key} is required")));
        }
    }
    Ok(())
}

/// Nested read form `{ schema, props }` of a stored record payload.
pub fn record_nested(payload: &JsonMap) -> Value {
    let props: JsonMap =
        payload.iter().filter_map(|(k, v)| k.strip_prefix("p:").map(|p| (p.to_string(), v.clone()))).collect();
    json!({ "schema": payload.get("schema").cloned().unwrap_or(Value::Null), "props": props })
}

// ---- Cell / TableView ----

const CELL_KEYS: &[&str] = &["source_ref", "view", "title", "fields", "filter", "sorts", "group", "manual_order", "options", "config", "def_ref", "bindings"];
/// Named data bindings of a Block (`aiws` v2): at most this many names.
pub const MAX_BINDINGS: usize = 32;
/// Largest canonical size of a Block's `config` / a definition's body.
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;
pub const MAX_BLOCK_DEF_BYTES: usize = 512 * 1024;
/// Version of the `window.aiws` Block host API (`aiws` v2) the wish planner writes into html results.
pub const HTML_API_VERSION: u64 = 2;

/// The four built-in views bind one fixed data type each; any other Renderer id is accepted with a
/// known data type as source, or without a source (D6: the registry decides what it supports).
fn view_source_type(view_type: &str) -> Option<&'static str> {
    Some(match view_type {
        "table" => TYPE_TABLE,
        "richtext" => TYPE_RICHTEXT,
        "record" => TYPE_RECORD,
        "asset" => TYPE_ASSET,
        _ => return None,
    })
}

const DATA_SOURCE_TYPES: &[&str] = &[TYPE_TABLE, TYPE_RICHTEXT, TYPE_RECORD, TYPE_ASSET, TYPE_WISH, TYPE_ANNOTATION, TYPE_BLOCK_DEF];

fn validate_cell(p: &mut Planner, e: &mut EntityRow, changed: Option<&[String]>) -> WsResult<()> {
    check_keys(&e.payload, CELL_KEYS, "cell")?;
    // a package may legitimately carry dangling view configuration; it is preserved, not re-judged
    let importing = p.env.import;
    let is_changed = |k: &str| !importing && changed.map_or(true, |c| c.iter().any(|x| x == k));
    let view = e.payload.get("view").cloned().ok_or_else(|| bad("cell needs view"))?;
    let view_type = view.get("type").and_then(Value::as_str).ok_or_else(|| bad("cell needs view.type"))?.to_string();
    if !is_renderer_id(&view_type) {
        return Err(bad(format!("view.type {view_type:?} is not a valid renderer id")));
    }
    if view.as_object().is_some_and(|o| o.keys().any(|k| !matches!(k.as_str(), "type" | "version"))) {
        return Err(bad("view: unknown key"));
    }
    if view.get("version").is_some_and(|v| !v.as_u64().is_some_and(|n| n >= 1 && n <= 1_000_000)) {
        return Err(bad("view.version must be a positive integer"));
    }
    let builtin = view_source_type(&view_type);
    let source_ref = match e.payload.get("source_ref") {
        Some(r) if !r.is_null() => Some(normalize_reference(r)?),
        _ if builtin.is_some() => return Err(bad("cell needs source_ref")),
        _ => None,
    };
    if let Some(r) = &source_ref {
        e.payload.insert("source_ref".into(), r.clone());
    } else {
        e.payload.remove("source_ref");
    }
    let source = match &source_ref {
        None => None,
        Some(source_ref) if importing || is_changed("source_ref") || is_changed("view") => {
            p.check_ref_target(source_ref, Some(builtin.map(|t| vec![t]).unwrap_or_else(|| DATA_SOURCE_TYPES.to_vec()).as_slice()))?
        }
        // dangling configuration must stay editable: do not re-require the source here
        Some(source_ref) => match reference_entity_id(source_ref).filter(|_| reference_is_local(source_ref)) {
            Some(id) => p.ov.entity(id)?,
            None => None,
        },
    };
    match e.payload.get("bindings").cloned() {
        None => {}
        Some(Value::Null) => {
            e.payload.remove("bindings");
        }
        Some(Value::Object(b)) => {
            if b.len() > MAX_BINDINGS {
                return Err(WsError::limit(format!("at most {MAX_BINDINGS} bindings")));
            }
            let mut norm = Map::new();
            for (name, target) in b {
                if !crate::wish::is_input_name(&name) || name == "source" {
                    return Err(bad(format!("binding name {name:?} must be an identifier other than source")));
                }
                let t = target.as_object().ok_or_else(|| bad("binding must be { entity_id, selector? }"))?;
                if t.keys().any(|k| !matches!(k.as_str(), "entity_id" | "selector")) {
                    return Err(bad("binding: unknown key"));
                }
                if t.get("selector").is_some_and(|s| !s.is_object() || s.get("kind").and_then(Value::as_str).is_none()) {
                    return Err(bad("binding.selector must be an object with kind"));
                }
                let r = normalize_reference(&json!({ "entity_id": t.get("entity_id").cloned().unwrap_or(Value::Null) }))?;
                if importing || is_changed("bindings") {
                    p.check_ref_target(&r, Some(DATA_SOURCE_TYPES))?;
                }
                let mut nt = Map::new();
                nt.insert("entity_id".into(), r["entity_id"].clone());
                if let Some(sel) = t.get("selector") {
                    nt.insert("selector".into(), sel.clone());
                }
                norm.insert(name, Value::Object(nt));
            }
            e.payload.insert("bindings".into(), Value::Object(norm));
        }
        Some(_) => return Err(bad("bindings must be an object")),
    }
    if let Some(d) = e.payload.get("def_ref").filter(|v| !v.is_null()) {
        let d = normalize_reference(d)?;
        if is_changed("def_ref") || importing {
            p.check_ref_target(&d, Some(&[TYPE_BLOCK_DEF]))?;
        }
        e.payload.insert("def_ref".into(), d);
    } else {
        e.payload.remove("def_ref");
    }
    opt_text(&e.payload, "title", 256)?;
    if e.payload.get("options").is_some_and(|o| !o.is_object()) {
        return Err(bad("options must be an object"));
    }
    if let Some(c) = e.payload.get("config") {
        if !c.is_object() {
            return Err(bad("config must be an object"));
        }
        if let Some(snapshot) = c.get("snapshot") {
            let mut asset = e.clone();
            asset.payload = snapshot.as_object().cloned().ok_or_else(|| bad("config.snapshot must be an asset object"))?;
            if importing || is_changed("config") {
                validate_asset(p, &mut asset)?;
                e.payload.get_mut("config").unwrap().as_object_mut().unwrap().insert("snapshot".into(), Value::Object(asset.payload));
            }
        }
        let c = &e.payload["config"];
        if canonical_len(c) > MAX_CONFIG_BYTES {
            return Err(WsError::limit(format!("config is limited to {MAX_CONFIG_BYTES} bytes")));
        }
    }
    if view_type != "table" {
        for k in ["filter", "sorts", "group", "manual_order"] {
            if e.payload.get(k).is_some_and(|v| !v.is_null()) {
                return Err(bad(format!("{k} is only valid on table views")));
            }
        }
        if view_type != "record" && e.payload.get("fields").is_some_and(|v| !v.is_null()) {
            return Err(bad("fields is only valid on table and record views"));
        }
    }
    let live: BTreeMap<String, FieldDef> = match (&source, view_type.as_str()) {
        (Some(s), "table") => filter::live_fields(&p.ov, &s.entity_id)?.into_iter().map(|f| (f.field_id, f.def)).collect(),
        (Some(s), "record") => record_schema(&s.payload).unwrap_or_default(),
        _ => BTreeMap::new(),
    };
    if let Some(fields) = e.payload.get("fields").filter(|v| !v.is_null()) {
        let list = fields.as_array().ok_or_else(|| bad("fields must be an array"))?;
        let mut seen = BTreeSet::new();
        for f in list {
            let id = f.get("field_id").and_then(Value::as_str).filter(|s| is_valid_id(s)).ok_or_else(|| bad("fields[].field_id required"))?;
            if !seen.insert(id) {
                return Err(bad(format!("duplicate field {id} in fields")));
            }
            if f.as_object().is_some_and(|o| o.keys().any(|k| !matches!(k.as_str(), "field_id" | "width" | "hidden"))) {
                return Err(bad("fields[]: unknown key"));
            }
            if f.get("width").is_some_and(|w| !w.as_f64().is_some_and(|x| x > 0.0 && x.is_finite())) {
                return Err(bad("fields[].width must be a positive number"));
            }
            if is_changed("fields") && source.is_some() && !live.contains_key(id) {
                return Err(WsError::invalid_op(format!("view references unknown field {id}")));
            }
        }
    }
    if is_changed("filter") {
        if let Some(f) = e.payload.get("filter").filter(|v| !v.is_null()) {
            filter::compile(f, &live)?;
        }
    }
    if let Some(sorts) = e.payload.get("sorts").filter(|v| !v.is_null()) {
        let list = sorts.as_array().ok_or_else(|| bad("sorts must be an array"))?;
        if list.len() > filter::MAX_SORTS {
            return Err(WsError::invalid_op("at most 4 sorts"));
        }
        for s in list {
            let id = s.get("field_id").and_then(Value::as_str).ok_or_else(|| bad("sorts[].field_id required"))?;
            if !matches!(s.get("direction").and_then(Value::as_str), Some("asc") | Some("desc")) {
                return Err(bad("sorts[].direction must be asc or desc"));
            }
            if is_changed("sorts") {
                match live.get(id) {
                    Some(d) if d.ty.sortable() => {}
                    _ => return Err(WsError::invalid_op(format!("cannot sort by {id}"))),
                }
            }
        }
    }
    if let Some(g) = e.payload.get("group").filter(|v| !v.is_null()) {
        let id = g.get("field_id").and_then(Value::as_str).ok_or_else(|| bad("group.field_id required"))?;
        if is_changed("group") {
            match live.get(id) {
                Some(d) if matches!(d.ty, FieldType::Select | FieldType::Boolean | FieldType::Date) => {}
                _ => return Err(WsError::invalid_op(format!("cannot group by {id}"))),
            }
        }
    }
    if let Some(m) = e.payload.get("manual_order").filter(|v| !v.is_null()) {
        let o = m.as_object().ok_or_else(|| bad("manual_order must be an object"))?;
        for (rid, key) in o {
            if !is_valid_id(rid) {
                return Err(bad("manual_order: invalid record_id"));
            }
            check_order_key(key.as_str().unwrap_or(""))?;
        }
    }
    Ok(())
}

// ---- Container ----

/// Container kinds (phase two §4.5): `folder` in the data tree, `surface` under `surfaces`, `group`
/// in a BlockTree. `root` / `data` / `surfaces` exist only as system nodes. A Surface names its
/// canvas content folder (`content_folder_id`, a folder under `canvas-content`); the folder may
/// point back (`surface_id`, informative).
fn validate_container(p: &Planner, e: &mut EntityRow, before: Option<&EntityRow>) -> WsResult<()> {
    check_keys(&e.payload, &["kind", "layout", "title", "content_folder_id", "system", "surface_id"], "container")?;
    let kind = e.payload.get("kind").and_then(Value::as_str).ok_or_else(|| bad("container needs kind"))?.to_string();
    match (kind.as_str(), before) {
        ("folder" | "surface" | "group", None) => {}
        ("root" | "data" | "surfaces", None) if p.env.internal => {}
        (_, None) => return Err(bad("container kind must be folder, surface or group")),
        (k, Some(b)) if b.payload.get("kind").and_then(Value::as_str) == Some(k) => {}
        _ => return Err(WsError::invalid_op("container kind cannot be changed")),
    }
    let mode = match e.payload.get("layout") {
        None => "flow".to_string(),
        Some(l) => {
            let m = l.get("mode").and_then(Value::as_str).unwrap_or("");
            if l.as_object().map_or(0, |o| o.len()) != 1 || !matches!(m, "flow" | "free") {
                return Err(bad("layout must be { mode: flow | free }"));
            }
            m.to_string()
        }
    };
    e.payload.insert("layout".into(), json!({ "mode": mode }));
    opt_text(&e.payload, "title", 256)?;
    match e.payload.get("system") {
        None => {}
        Some(Value::String(s)) if kind == "folder" && matches!(s.as_str(), "canvas_content" | "surface_content") => {}
        Some(_) => return Err(bad("system must be canvas_content or surface_content on a folder")),
    }
    if let Some(sid) = e.payload.get("surface_id") {
        if kind != "folder" || !sid.as_str().is_some_and(is_valid_id) {
            return Err(bad("surface_id must be an entity id on a folder"));
        }
    }
    match e.payload.get("content_folder_id") {
        None if kind == "surface" => return Err(bad("a surface needs content_folder_id (its folder under canvas-content)")),
        None => {}
        Some(v) => {
            let fid = v.as_str().filter(|s| is_valid_id(s)).ok_or_else(|| bad("content_folder_id must be an entity id"))?;
            if kind != "surface" {
                return Err(bad("content_folder_id is only valid on a surface"));
            }
            // a package creates surfaces before their (deeper) folders: judged once the whole batch is staged
            if !p.env.import && before.is_none_or(|b| b.payload.get("content_folder_id") != Some(v)) {
                let folder = p.ov.entity(fid)?.filter(EntityRow::alive).ok_or_else(|| bad(format!("content folder {fid} does not exist")))?;
                let is_folder = folder.type_id == TYPE_CONTAINER && folder.payload.get("kind").and_then(Value::as_str) == Some("folder");
                let under_content = p.ov.edge(fid)?.is_some_and(|edge| edge.parent_id == CANVAS_CONTENT_ID);
                if !is_folder || !under_content {
                    return Err(bad(format!("content folder {fid} must be a folder under {CANVAS_CONTENT_ID}")));
                }
            }
        }
    }
    Ok(())
}

// ---- AssetRef ----

fn validate_asset(p: &mut Planner, e: &mut EntityRow) -> WsResult<()> {
    check_keys(&e.payload, &["object_id", "media_type", "size", "file_name", "image"], "asset")?;
    let object_id = e.payload.get("object_id").and_then(Value::as_str).ok_or_else(|| bad("asset needs object_id"))?.to_string();
    if !crate::canonical::is_obj_id(&object_id) {
        return Err(bad("asset object_id must be a type:hex object id"));
    }
    let Some(info) = p.ov.asset(&object_id)? else {
        if p.env.import && e.payload.get("media_type").is_some_and(Value::is_string) && e.payload.get("size").is_some_and(Value::is_u64) {
            return Ok(()); // a package may list an asset as not included; it reads back as `missing`
        }
        return Err(WsError::new(Code::DependencyUnavailable, format!("asset {object_id} has not been uploaded")));
    };
    // media type and size come from the verified content, never from the client
    e.payload.insert("media_type".into(), json!(info.media_type));
    e.payload.insert("size".into(), json!(info.size));
    p.ov.assets.insert(object_id, info);
    opt_text(&e.payload, "file_name", 255)?;
    if let Some(img) = e.payload.get("image") {
        let dim = |k: &str| img.get(k).and_then(Value::as_u64).is_some_and(|n| n > 0 && n <= 1 << 20);
        if img.as_object().map_or(0, |o| o.len()) != 2 || !dim("width") || !dim("height") {
            return Err(bad("image must be { width, height }"));
        }
    }
    Ok(())
}

// ---- Annotation ----

const ANCHOR_KEYS: &[&str] = &["target", "range", "context"];

/// Anchor keys (`target`, `range`, `context`) are checked by `anchor`; changing them later
/// re-anchors the annotation, which only its author may do.
fn validate_annotation(p: &Planner, e: &mut EntityRow, before: Option<&EntityRow>, changed: Option<&[String]>) -> WsResult<()> {
    // replay keeps what a newer backend accepted
    let strict = !p.env.import;
    if strict {
        check_keys(&e.payload, &["target", "range", "context", "kind", "body", "style", "author"], "annotation")?;
    }
    let anchor_changed = match before {
        None => true,
        Some(b) => {
            if e.payload.get("author") != b.payload.get("author") {
                return Err(WsError::invalid_op("annotation author cannot be changed"));
            }
            let changed = changed.unwrap_or(&[]);
            let has = |k: &str| changed.iter().any(|c| c == k);
            if !ANCHOR_KEYS.iter().any(|k| has(k)) {
                false
            } else {
                if !p.env.internal && annotation_author(b) != Some(p.env.principal.as_str()) {
                    return Err(WsError::denied("only the author can re-anchor an annotation"));
                }
                // a range or quote of the old target means nothing on a new one
                if e.payload.get("target") != b.payload.get("target") {
                    if let Some(k) = ["range", "context"].into_iter().find(|k| b.payload.contains_key(*k) && !has(k)) {
                        return Err(WsError::invalid_op(format!("re-anchoring to another target must restate {k} (null clears it)")));
                    }
                }
                true
            }
        }
    };
    if anchor_changed {
        for k in ["target", "range", "context"] {
            if e.payload.get(k).is_some_and(Value::is_null) {
                e.payload.remove(k);
            }
        }
        if e.payload.contains_key("target") {
            if let Some(id) = e.payload.get("target").and_then(|t| t.get("entity_id")).and_then(Value::as_str) {
                p.check_ref_target(&json!({ "entity_id": id }), None)?;
            }
            crate::anchor::check(&p.ov, &mut e.payload, strict)?;
        } else if strict && (e.payload.contains_key("range") || e.payload.contains_key("context")) {
            // a free note (phase two §4.2) has nothing to range over or quote
            return Err(bad("range and context need a target"));
        }
    }
    if before.is_none() {
        e.payload.insert("author".into(), json!(p.env.principal));
    }
    match e.payload.get("kind").and_then(Value::as_str) {
        Some("note") | Some("highlight") => {}
        _ if !strict => {}
        _ => return Err(bad("annotation kind must be note or highlight")),
    }
    opt_text(&e.payload, "body", 4000)?;
    if e.payload.get("style").is_some_and(|s| !s.is_object()) {
        return Err(bad("style must be an object"));
    }
    Ok(())
}

// ---- Wish (phase two §7.1) ----

const WISH_KEYS: &[&str] = &["title", "prompt", "knowledge", "refinements", "analysis", "inputs", "executor", "output", "output_mode", "executor_config", "program", "last_run"];
pub const MAX_PROMPT_CHARS: usize = 20_000;

/// One input reference of a wish or of a dependency record:
/// `{ entity_id, selector?, version: { mode: follow | fixed, rev? }, label?, name?, appended_by? }`.
/// A wish input (`wish`) selects *what* is read — the whole entity, a saved table view
/// (`table_view { cell_id }`) or a table query (`table_query { filter?, sorts?, fields? }`); a
/// dependency record lists the version cells that were read.
fn check_input(p: &Planner, v: &Value, require_target: bool, wish: bool) -> WsResult<Value> {
    let o = v.as_object().ok_or_else(|| bad("input must be an object"))?;
    for k in o.keys() {
        if !matches!(k.as_str(), "entity_id" | "selector" | "version" | "label" | "name" | "appended_by") {
            return Err(bad(format!("input: unknown key {k}")));
        }
    }
    let id = o.get("entity_id").and_then(Value::as_str).filter(|s| is_valid_id(s)).ok_or_else(|| bad("input.entity_id required"))?;
    let mut out = Map::new();
    out.insert("entity_id".into(), json!(id));
    if let Some(sel) = o.get("selector").filter(|s| !s.is_null()) {
        if !sel.is_object() || sel.get("kind").and_then(Value::as_str).is_none() || canonical_len(sel) > 4096 {
            return Err(bad("input.selector must be an object with kind"));
        }
        if wish {
            match sel["kind"].as_str().unwrap_or("") {
                "entity" => {}
                "table_view" => {
                    let cell = sel.get("cell_id").and_then(Value::as_str).filter(|s| is_valid_id(s)).ok_or_else(|| bad("table_view selector needs cell_id"))?;
                    if require_target {
                        match p.ov.entity(cell)?.filter(|c| c.alive()) {
                            // a package stages the Surfaces after the data tree: the view comes later
                            // (a view that never comes is reported by freshness as `view_missing`)
                            None if p.env.import => {}
                            None => return Err(WsError::invalid_op(format!("view {cell} does not exist"))),
                            Some(c) => {
                                let bound = c.payload.get("source_ref").and_then(reference_entity_id);
                                if c.type_id != TYPE_CELL || c.payload.get("view").and_then(|v| v["type"].as_str()) != Some("table") || bound != Some(id) {
                                    return Err(WsError::invalid_op(format!("{cell} is not a table view of {id}")));
                                }
                            }
                        }
                    }
                }
                "table_query" => {
                    let so = sel.as_object().unwrap();
                    if so.keys().any(|k| !matches!(k.as_str(), "kind" | "filter" | "sorts" | "fields")) {
                        return Err(bad("table_query selector: unknown key"));
                    }
                    if so.get("sorts").is_some_and(|x| !x.is_array()) || so.get("fields").is_some_and(|x| !x.as_array().is_some_and(|a| a.iter().all(Value::is_string))) {
                        return Err(bad("table_query: sorts / fields must be arrays"));
                    }
                }
                k => return Err(bad(format!("a wish input selects entity, table_view or table_query, not {k}"))),
            }
        }
        out.insert("selector".into(), sel.clone());
    }
    let version = match o.get("version") {
        None => json!({ "mode": "follow" }),
        Some(ver) => {
            let mode = ver.get("mode").and_then(Value::as_str).unwrap_or("");
            let keys_ok = ver.as_object().is_some_and(|m| m.keys().all(|k| matches!(k.as_str(), "mode" | "rev" | "hash" | "object_id")));
            if !keys_ok || !matches!(mode, "follow" | "fixed") {
                return Err(bad("input.version must be { mode: follow | fixed, rev? | hash? }"));
            }
            ver.clone()
        }
    };
    out.insert("version".into(), version);
    if let Some(l) = o.get("label") {
        if !l.as_str().is_some_and(|s| s.chars().count() <= 256) {
            return Err(bad("input.label must be a short string"));
        }
        out.insert("label".into(), l.clone());
    }
    if let Some(n) = o.get("name") {
        if !n.as_str().is_some_and(crate::wish::is_input_name) {
            return Err(bad("input.name must be an identifier ([A-Za-z_][A-Za-z0-9_]*)"));
        }
        out.insert("name".into(), n.clone());
    }
    if let Some(r) = o.get("appended_by") {
        if !r.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 128) {
            return Err(bad("input.appended_by must be a run id"));
        }
        out.insert("appended_by".into(), r.clone());
    }
    if require_target {
        p.check_ref_target(&json!({ "entity_id": id }), None)?;
    }
    Ok(Value::Object(out))
}

fn validate_wish(p: &mut Planner, e: &mut EntityRow, changed: Option<&[String]>) -> WsResult<()> {
    check_keys(&e.payload, WISH_KEYS, "wish")?;
    let importing = p.env.import;
    let is_changed = |k: &str| !importing && changed.map_or(true, |c| c.iter().any(|x| x == k));
    opt_text(&e.payload, "title", 256)?;
    opt_text(&e.payload, "prompt", MAX_PROMPT_CHARS)?;
    if !e.payload.get("prompt").is_some_and(Value::is_string) {
        return Err(bad("wish needs prompt"));
    }
    match e.payload.get("executor").and_then(Value::as_str) {
        Some("mock" | "xllm" | "agent-work-session") => {}
        _ => return Err(bad("executor must be mock, xllm or agent-work-session")),
    }
    match e.payload.get("output_mode") {
        None => {
            e.payload.insert("output_mode".into(), json!("overwrite"));
        }
        Some(Value::String(m)) if m == "overwrite" || m == "new" => {}
        Some(_) => return Err(bad("output_mode must be overwrite or new")),
    }
    for k in ["knowledge", "refinements", "analysis", "program"] {
        if e.payload.get(k).is_some_and(Value::is_null) {
            e.payload.remove(k);
        }
    }
    if let Some(k) = e.payload.get("knowledge") {
        crate::wish::check_knowledge(k)?;
    }
    if let Some(r) = e.payload.get("refinements") {
        crate::wish::check_refinements(r)?;
    }
    if let Some(prog) = e.payload.get("program").cloned() {
        let source = crate::wish::check_program(&prog)?.to_string();
        if is_changed("program") || importing {
            match p.ov.asset(&source)? {
                Some(info) => {
                    p.ov.assets.insert(source, info);
                }
                None if importing => {}
                None => return Err(WsError::new(Code::DependencyUnavailable, format!("program source {source} has not been uploaded"))),
            }
        }
    }
    if let Some(inputs) = e.payload.get("inputs").filter(|v| !v.is_null()).cloned() {
        let list = inputs.as_array().ok_or_else(|| bad("inputs must be an array"))?;
        if list.len() > 200 {
            return Err(WsError::limit("at most 200 inputs"));
        }
        let mut norm = Vec::new();
        let mut names = BTreeSet::new();
        for item in list {
            let input = check_input(p, item, is_changed("inputs") || importing, true)?;
            if let Some(n) = input.get("name").and_then(Value::as_str) {
                if !names.insert(n.to_string()) {
                    return Err(bad(format!("duplicate input name {n}")));
                }
            }
            norm.push(input);
        }
        e.payload.insert("inputs".into(), Value::Array(norm));
    } else {
        e.payload.remove("inputs");
    }
    if let Some(a) = e.payload.get("analysis") {
        crate::wish::check_analysis(a)?;
    }
    if e.payload.contains_key("analysis") && is_changed("analysis") {
        // the basis is the core's statement of what the analysis was made from, never the writer's
        crate::wish::with_basis(&mut e.payload);
    }
    if let Some(out) = e.payload.get("output").filter(|v| !v.is_null()) {
        let o = out.as_object().ok_or_else(|| bad("output must be an object"))?;
        for k in o.keys() {
            if !matches!(k.as_str(), "container_id" | "name" | "type" | "surface_id") {
                return Err(bad(format!("output: unknown key {k}")));
            }
        }
        if let Some(c) = o.get("container_id") {
            if !c.as_str().is_some_and(is_valid_id) {
                return Err(bad("output.container_id must be an entity id"));
            }
        }
        if let Some(sid) = o.get("surface_id") {
            if !sid.as_str().is_some_and(is_valid_id) {
                return Err(bad("output.surface_id must be an entity id"));
            }
        }
        if !o.get("name").is_some_and(|n| n.as_str().is_some_and(|s| !s.is_empty() && s.chars().count() <= 128 && !s.contains('/'))) {
            return Err(bad("output.name required (1-128 chars, no '/')"));
        }
        if o.get("type").is_some_and(|t| !t.as_str().is_some_and(|s| s.chars().count() <= 64)) {
            return Err(bad("output.type must be a short string"));
        }
    } else {
        e.payload.remove("output");
    }
    if e.payload.get("executor_config").is_some_and(|c| !c.is_object() || canonical_len(c) > MAX_CONFIG_BYTES) {
        return Err(bad("executor_config must be a small object"));
    }
    if e.payload.get("last_run").is_some_and(|c| !c.is_object() || canonical_len(c) > MAX_CONFIG_BYTES) {
        return Err(WsError::limit("last_run must be an object of at most 64 KiB"));
    }
    if e.payload.get("analysis").is_some_and(|c| canonical_len(c) > MAX_CONFIG_BYTES * 2) {
        return Err(WsError::limit("analysis is limited to 128 KiB"));
    }
    Ok(())
}

// ---- Block definition (phase two §10.3) ----

fn validate_block_def(e: &mut EntityRow) -> WsResult<()> {
    check_keys(&e.payload, &["def_id", "version", "kind", "title", "accepts", "allow_no_source", "default_size", "declarative", "html", "config_schema", "actions", "inspector", "description"], "block_def")?;
    let def_id = e.payload.get("def_id").and_then(Value::as_str).ok_or_else(|| bad("block_def needs def_id"))?;
    if !is_renderer_id(def_id) {
        return Err(bad("def_id must be a renderer id"));
    }
    match e.payload.get("version") {
        None => {
            e.payload.insert("version".into(), json!(1));
        }
        Some(v) if v.as_u64().is_some_and(|n| n >= 1 && n <= 1_000_000) => {}
        Some(_) => return Err(bad("version must be a positive integer")),
    }
    let kind = e.payload.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "declarative" => {
            if !e.payload.get("declarative").is_some_and(Value::is_object) {
                return Err(bad("a declarative definition needs `declarative`"));
            }
        }
        "html" => {
            let h = e.payload.get("html").and_then(Value::as_object).ok_or_else(|| bad("an html definition needs `html`"))?;
            if !h.get("html").is_some_and(Value::is_string) {
                return Err(bad("html.html (the markup) required"));
            }
            for k in h.keys() {
                if !matches!(k.as_str(), "html" | "css" | "js" | "api_version") {
                    return Err(bad(format!("html: unknown key {k}")));
                }
            }
            // format only: whether a version runs is the Block host's decision (D6), so a definition
            // written for a newer host still travels and falls back locally
            if h.get("api_version").is_some_and(|v| v.as_u64().is_none_or(|n| n == 0)) {
                return Err(bad("html.api_version must be a positive integer"));
            }
        }
        _ => return Err(bad("kind must be declarative or html")),
    }
    opt_text(&e.payload, "title", 256)?;
    opt_text(&e.payload, "description", 4000)?;
    if let Some(a) = e.payload.get("accepts") {
        let ok = a.as_array().is_some_and(|l| l.iter().all(|t| t.as_str().is_some_and(is_data_type)));
        if !ok {
            return Err(bad("accepts must list known data types"));
        }
    }
    if e.payload.get("allow_no_source").is_some_and(|v| !v.is_boolean()) {
        return Err(bad("allow_no_source must be a boolean"));
    }
    if let Some(d) = e.payload.get("default_size") {
        let dim = |k: &str| d.get(k).and_then(Value::as_f64).is_some_and(|n| n > 0.0 && n.is_finite());
        if !(dim("w") && dim("h")) {
            return Err(bad("default_size must be { w, h }"));
        }
    }
    for k in ["config_schema", "actions", "inspector"] {
        if e.payload.get(k).is_some_and(|v| !(v.is_object() || v.is_array())) {
            return Err(bad(format!("{k} must be an object or array")));
        }
    }
    if canonical_len(&Value::Object(e.payload.clone())) > MAX_BLOCK_DEF_BYTES {
        return Err(WsError::limit(format!("a block definition is limited to {MAX_BLOCK_DEF_BYTES} bytes")));
    }
    Ok(())
}

// ---- dependency record (phase two §7.3) ----

/// `{ wish_id, run_id, executor, inputs: [{ entity_id, selector?, version: { mode, rev? | hash? } }], generated_rev }`.
/// The version cells are the read set the application was guarded with; `generated_rev` is set here.
pub fn check_derived(p: &Planner, v: &Value, content_rev: u64) -> WsResult<Value> {
    let o = v.as_object().ok_or_else(|| bad("derived must be an object"))?;
    for k in o.keys() {
        if !matches!(
            k.as_str(),
            "wish_id" | "run_id" | "executor" | "inputs" | "generated_rev" | "simulated" | "at" | "output_mode" | "group" | "stale" | "stale_at_import" | "kept_manual"
                | "config_digest" | "result_key" | "approach" | "program_digest" | "model_judgment" | "external_data" | "mode"
        ) {
            return Err(bad(format!("derived: unknown key {k}")));
        }
    }
    let mut out = Map::new();
    let wish = o.get("wish_id").and_then(Value::as_str).filter(|s| is_valid_id(s)).ok_or_else(|| bad("derived.wish_id required"))?;
    out.insert("wish_id".into(), json!(wish));
    let run = o.get("run_id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 128).ok_or_else(|| bad("derived.run_id required"))?;
    out.insert("run_id".into(), json!(run));
    let executor = o.get("executor").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 64).ok_or_else(|| bad("derived.executor required"))?;
    out.insert("executor".into(), json!(executor));
    let inputs = o.get("inputs").and_then(Value::as_array).ok_or_else(|| bad("derived.inputs must be an array"))?;
    if inputs.len() > 200 {
        return Err(WsError::limit("at most 200 derived inputs"));
    }
    let mut norm = Vec::new();
    for i in inputs {
        let mut input = check_input(p, i, false, false)?;
        if p.env.import {
            // a package's revisions belong to another deployment (§7.5 rule 4): the record is rebased onto
            // the versions the package itself carries, which were captured together with the result
            let target = json!({ "entity_id": input["entity_id"], "selector": input.get("selector").cloned().unwrap_or(json!({ "kind": "entity" })) });
            if let Some(ver) = input.get_mut("version").and_then(Value::as_object_mut) {
                ver.remove("rev");
                ver.remove("hash");
                if let Ok(cur) = crate::plan::resolve_cell(&p.ov, &target) {
                    ver.insert(if cur.is_string() { "hash".into() } else { "rev".into() }, cur);
                }
            }
        }
        norm.push(input);
    }
    out.insert("inputs".into(), Value::Array(norm));
    out.insert("generated_rev".into(), json!(content_rev));
    for k in ["simulated", "at", "output_mode", "group", "model_judgment", "external_data"] {
        if let Some(x) = o.get(k) {
            out.insert(k.into(), x.clone());
        }
    }
    for k in ["config_digest", "result_key", "program_digest", "mode"] {
        if let Some(x) = o.get(k) {
            if !x.as_str().is_some_and(|s| !s.is_empty() && s.chars().count() <= 128) {
                return Err(bad(format!("derived.{k} must be a short string")));
            }
            out.insert(k.into(), x.clone());
        }
    }
    if let Some(a) = o.get("approach") {
        if !matches!(a.as_str(), Some("program" | "direct")) {
            return Err(bad("derived.approach must be program or direct"));
        }
        out.insert("approach".into(), a.clone());
    }
    if o.get("kept_manual") == Some(&json!(true)) {
        out.insert("kept_manual".into(), json!(true));
    }
    if p.env.import {
        // what was stale when exported stays stale until the wish runs again
        if o.get("stale") == Some(&json!(true)) || o.get("stale_at_import") == Some(&json!(true)) {
            out.insert("stale_at_import".into(), json!(true));
        }
    } else if o.get("stale_at_import") == Some(&json!(true)) {
        out.insert("stale_at_import".into(), json!(true));
    }
    if canonical_len(&Value::Object(out.clone())) > MAX_CONFIG_BYTES {
        return Err(WsError::limit("derived record too large"));
    }
    Ok(Value::Object(out))
}

// ---- TableSource metadata ----

pub fn is_url_table(e: &EntityRow) -> bool {
    e.payload.get("data_mode").and_then(Value::as_str) == Some("url_query")
}

/// Validate a `QueryReference` (design doc §2.4). Credentials never belong in it.
pub fn check_query_reference(v: &Value) -> WsResult<()> {
    let o = v.as_object().ok_or_else(|| bad("source_ref must be an object"))?;
    for k in o.keys() {
        if !matches!(k.as_str(), "kind" | "source_url" | "query" | "version" | "consistency") {
            return Err(bad(format!("source_ref: unknown key {k}")));
        }
    }
    if o.get("kind").and_then(Value::as_str) != Some("url_query") {
        return Err(bad("source_ref.kind must be url_query"));
    }
    let url = o.get("source_url").and_then(Value::as_str).unwrap_or("");
    let (scheme, rest) = url.split_once("://").ok_or_else(|| bad("source_url must be an absolute URL"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if scheme.is_empty() || authority.is_empty() || authority.contains('@') || url.chars().any(|c| c.is_control() || c == ' ') {
        return Err(bad("source_url must be an absolute URL without credentials"));
    }
    if o.get("query").is_some_and(|q| !q.is_object()) {
        return Err(bad("source_ref.query must be an object"));
    }
    if o.get("query").is_some_and(|q| q.get("cursor").is_some()) {
        return Err(bad("a persisted query must not contain a cursor"));
    }
    match o.get("version").map(|v| v["mode"].as_str().unwrap_or("")) {
        None | Some("live_head") => {}
        Some("fixed_revision") if o["version"].get("source_revision").is_some_and(Value::is_string) => {}
        _ => return Err(bad("source_ref.version must be live_head or fixed_revision with source_revision")),
    }
    match o.get("consistency").and_then(Value::as_str) {
        None | Some("best_effort") | Some("snapshot") => Ok(()),
        _ => Err(bad("consistency must be snapshot or best_effort")),
    }
}

fn validate_table_meta(p: &Planner, e: &mut EntityRow, before: Option<&EntityRow>) -> WsResult<()> {
    check_keys(&e.payload, &["title_field_id", "description", "data_mode", "source_ref"], "table")?;
    opt_text(&e.payload, "description", 4000)?;
    let mode = e.payload.get("data_mode").and_then(Value::as_str).unwrap_or("embedded").to_string();
    if let Some(b) = before {
        if b.payload.get("data_mode") != e.payload.get("data_mode") {
            return Err(WsError::invalid_op("data_mode cannot be changed"));
        }
    }
    match mode.as_str() {
        "embedded" => {
            if e.payload.contains_key("source_ref") {
                return Err(bad("source_ref is only valid with data_mode url_query"));
            }
            e.payload.remove("data_mode"); // default is omitted
        }
        "url_query" => check_query_reference(e.payload.get("source_ref").ok_or_else(|| bad("url_query table needs source_ref"))?)?,
        _ => return Err(bad("data_mode must be embedded or url_query")),
    }
    if let Some(t) = e.payload.get("title_field_id") {
        let fid = t.as_str().ok_or_else(|| bad("title_field_id must be a string"))?;
        match p.ov.field(&e.entity_id, fid)? {
            Some(f) if f.alive() && f.def.ty == FieldType::Text => {}
            _ => return Err(bad("title_field_id must name an existing text field")),
        }
    }
    Ok(())
}

// ---- entry points used by the planner ----

/// Initialize a newly created entity of a known type from the caller's
/// payload. Returns the payload as recorded in history (replayable form).
pub fn init_entity(p: &mut Planner, row: &mut EntityRow, mut payload: JsonMap) -> WsResult<Value> {
    match row.type_id.as_str() {
        TYPE_CONTAINER => {
            row.payload = payload;
            validate_container(p, row, None)?;
        }
        TYPE_RECORD => {
            check_keys(&payload, &["schema", "props"], "record")?;
            let mut stored = Map::new();
            stored.insert("schema".into(), payload.remove("schema").ok_or_else(|| bad("record needs schema"))?);
            if let Some(props) = payload.remove("props") {
                for (k, v) in props.as_object().ok_or_else(|| bad("props must be an object"))? {
                    stored.insert(format!("p:{k}"), v.clone());
                }
            }
            row.payload = stored;
            validate_record(p, row, None)?;
            return Ok(record_nested(&row.payload));
        }
        TYPE_CELL => {
            row.payload = payload;
            validate_cell(p, row, None)?;
        }
        TYPE_ASSET => {
            row.payload = payload;
            validate_asset(p, row)?;
        }
        TYPE_ANNOTATION => {
            if payload.contains_key("author") && !p.env.internal {
                return Err(bad("author is set by the backend"));
            }
            let author = payload.remove("author");
            row.payload = payload;
            validate_annotation(p, row, None, None)?;
            if let (Some(a), true) = (author, p.env.internal) {
                row.payload.insert("author".into(), a); // import keeps the original author
            }
        }
        TYPE_TABLE => {
            let fields = payload.remove("fields");
            let title = payload.remove("title_field_id");
            row.payload = payload;
            let mut prev: Option<String> = None;
            let mut recorded = Vec::new();
            for f in fields.as_ref().and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
                let mut o = f.as_object().cloned().ok_or_else(|| bad("field must be an object"))?;
                let order_key = match o.remove("order_key") {
                    Some(Value::String(k)) => {
                        check_order_key(&k)?;
                        k
                    }
                    None => order_key_between(prev.as_deref(), None)?,
                    Some(_) => return Err(bad("field order_key must be a string")),
                };
                let def = FieldDef::from_value(&Value::Object(o))?;
                if p.ov.field(&row.entity_id, &def.field_id)?.is_some() {
                    return Err(WsError::sub(Code::InvalidOperation, "ID_CONFLICT", format!("duplicate field_id {}", def.field_id)));
                }
                if p.ov.fields(&row.entity_id)?.iter().any(|x| x.def.name == def.name) {
                    return Err(WsError::sub(Code::InvalidOperation, "NAME_CONFLICT", format!("duplicate field name {}", def.name)));
                }
                let mut rec = def.to_value();
                rec["order_key"] = json!(order_key);
                recorded.push(rec);
                prev = Some(order_key.clone());
                p.ov.put_field(FieldRow {
                    source_id: row.entity_id.clone(),
                    field_id: def.field_id.clone(),
                    def,
                    order_key,
                    def_rev: p.seq(),
                    type_rev: p.seq(),
                    values_rev: 0,
                    deleted_seq: None,
                });
            }
            if let Some(t) = title {
                row.payload.insert("title_field_id".into(), t);
            }
            validate_table_meta(p, row, None)?;
            row.key_revs.insert(MEMBERS_KEY.to_string(), 0);
            let mut out = row.payload.clone();
            if !recorded.is_empty() {
                out.insert("fields".into(), Value::Array(recorded));
            }
            return Ok(Value::Object(out));
        }
        TYPE_RICHTEXT => return crate::plan_richtext::create(p, row, payload),
        TYPE_WISH => {
            row.payload = payload;
            validate_wish(p, row, None)?;
        }
        TYPE_BLOCK_DEF => {
            row.payload = payload;
            validate_block_def(row)?;
        }
        _ => row.payload = payload,
    }
    Ok(Value::Object(row.payload.clone()))
}

/// Validate a keyed document after `set_keys`/`unset_keys` changed `changed`.
pub fn validate_update(p: &mut Planner, before: &EntityRow, e: &mut EntityRow, changed: &[String]) -> WsResult<()> {
    match e.type_id.as_str() {
        TYPE_CONTAINER => validate_container(p, e, Some(before)),
        TYPE_RECORD => validate_record(p, e, Some(changed)),
        TYPE_CELL => validate_cell(p, e, Some(changed)),
        TYPE_ASSET => {
            // new content: dimensions of the old image do not describe it
            let has = |k: &str| changed.iter().any(|c| c == k);
            if has("object_id") && !has("image") && before.payload.get("object_id") != e.payload.get("object_id") {
                e.payload.remove("image");
            }
            validate_asset(p, e)
        }
        TYPE_ANNOTATION => validate_annotation(p, e, Some(before), Some(changed)),
        TYPE_TABLE => {
            // a URL table's query definition is local document state; its remote rows are not
            if changed.iter().any(|k| k == "source_ref") {
                p.require(&e.entity_id, Cap::Structure)?;
            }
            validate_table_meta(p, e, Some(before))
        }
        TYPE_WISH => validate_wish(p, e, Some(changed)),
        TYPE_BLOCK_DEF => validate_block_def(e),
        _ => Err(WsError::invalid_op("this type has no keyed content")),
    }
}

fn ref_edge(src: &str, selector: &str, kind: &str, reference: &Value) -> RefEdge {
    let mut edge = RefEdge::local(src, selector, kind, reference_entity_id(reference).unwrap_or(""));
    if let Some(ws) = reference.get("workspace_id").and_then(Value::as_str) {
        edge.dst_workspace_id = ws.to_string();
    }
    if let Some(o) = reference.get("version").and_then(|v| v.get("object_id")).and_then(Value::as_str) {
        edge.dst_object_id = o.to_string();
    }
    edge
}

/// The references held by an entity's keyed payload. (Table values, record
/// bodies and rich text references are maintained by their own planners.)
pub fn entity_refs(e: &EntityRow) -> WsResult<BTreeSet<RefEdge>> {
    let mut out = BTreeSet::new();
    if let Some(object_id) = asset_object_id(&e.type_id, &e.payload) {
        let mut edge = RefEdge::local(&e.entity_id, "", "asset", "");
        edge.dst_object_id = object_id.to_string();
        out.insert(edge);
    }
    // generation dependencies: result → each input it read, and result → the wish that produced it
    // (neither blocks deletion)
    if let Some(d) = &e.derived {
        for (i, input) in d.get("inputs").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let s = selector_string(&json!({ "kind": "derived_input", "index": i }));
            out.insert(ref_edge(&e.entity_id, &s, "derived", input));
        }
        if let Some(w) = d.get("wish_id").and_then(Value::as_str) {
            out.insert(RefEdge::local(&e.entity_id, "", "produced", w));
        }
    }
    match e.type_id.as_str() {
        TYPE_CELL => {
            if let Some(r) = e.payload.get("source_ref") {
                out.insert(ref_edge(&e.entity_id, "", "bind", r));
            }
            if let Some(d) = e.payload.get("def_ref") {
                out.insert(ref_edge(&e.entity_id, "", "def", d));
            }
            for (name, target) in e.payload.get("bindings").and_then(Value::as_object).into_iter().flatten() {
                let s = selector_string(&json!({ "kind": "binding", "name": name }));
                out.insert(ref_edge(&e.entity_id, &s, "bind", target));
            }
        }
        TYPE_WISH => {
            for (i, input) in e.payload.get("inputs").and_then(Value::as_array).into_iter().flatten().enumerate() {
                let s = selector_string(&json!({ "kind": "wish_input", "index": i }));
                out.insert(ref_edge(&e.entity_id, &s, "input", input));
            }
        }
        TYPE_ANNOTATION => {
            if let Some(t) = e.payload.get("target") {
                out.insert(ref_edge(&e.entity_id, "", "anchor", t));
            }
        }
        TYPE_RECORD => {
            if let Ok(schema) = record_schema(&e.payload) {
                for (key, def) in schema {
                    if def.ty == FieldType::ObjectRef {
                        if let Some(v) = e.payload.get(&format!("p:{key}")).filter(|v| v.is_object()) {
                            let s = selector_string(&json!({ "kind": "doc_key", "key": format!("p:{key}") }));
                            out.insert(ref_edge(&e.entity_id, &s, "value", v));
                        }
                    }
                }
            }
        }
        TYPE_TABLE => {
            if let Some(q) = e.payload.get("source_ref") {
                let mut edge = RefEdge::local(&e.entity_id, "", "bind", "");
                edge.dst_query_json = crate::canonical::canonical_json(q)?;
                out.insert(edge);
            }
        }
        _ => {}
    }
    Ok(out)
}

pub fn asset_object_id<'a>(type_id: &str, payload: &'a JsonMap) -> Option<&'a str> {
    match type_id {
        TYPE_ASSET => payload.get("object_id").and_then(Value::as_str),
        TYPE_CELL => payload.get("config")?.get("snapshot")?.get("object_id")?.as_str(),
        TYPE_WISH => payload.get("program")?.get("source")?.as_str(),
        _ => None,
    }
}

pub fn value_ref_edge(source_id: &str, record_id: &str, field_id: &str, reference: &Value) -> RefEdge {
    let s = selector_string(&json!({ "kind": "table_cell", "record_id": record_id, "field_id": field_id }));
    ref_edge(source_id, &s, "value", reference)
}

pub fn body_ref_edge(source_id: &str, record_id: &str, body_entity_id: &str) -> RefEdge {
    let s = selector_string(&json!({ "kind": "table_record", "record_id": record_id }));
    RefEdge::local(source_id, &s, "body", body_entity_id)
}

pub fn richtext_ref_edge(entity_id: &str, block_id: &str, reference: &Value) -> RefEdge {
    let s = selector_string(&json!({ "kind": "richtext_block", "block_id": block_id }));
    ref_edge(entity_id, &s, "embed", reference)
}
