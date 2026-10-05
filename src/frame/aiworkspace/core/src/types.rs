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

const CELL_KEYS: &[&str] = &["source_ref", "view", "title", "fields", "filter", "sorts", "group", "manual_order", "options"];

fn view_source_type(view_type: &str) -> Option<&'static str> {
    Some(match view_type {
        "table" => TYPE_TABLE,
        "richtext" => TYPE_RICHTEXT,
        "record" => TYPE_RECORD,
        "asset" => TYPE_ASSET,
        _ => return None,
    })
}

fn validate_cell(p: &Planner, e: &mut EntityRow, changed: Option<&[String]>) -> WsResult<()> {
    check_keys(&e.payload, CELL_KEYS, "cell")?;
    // a package may legitimately carry dangling view configuration; it is preserved, not re-judged
    let importing = p.env.import;
    let is_changed = |k: &str| !importing && changed.map_or(true, |c| c.iter().any(|x| x == k));
    let view_type = e
        .payload
        .get("view")
        .and_then(|v| v.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| bad("cell needs view.type"))?
        .to_string();
    let source_type = view_source_type(&view_type).ok_or_else(|| bad(format!("unknown view.type {view_type}")))?;
    let source_ref = normalize_reference(e.payload.get("source_ref").ok_or_else(|| bad("cell needs source_ref"))?)?;
    e.payload.insert("source_ref".into(), source_ref.clone());
    let source = if importing || is_changed("source_ref") || is_changed("view") {
        p.check_ref_target(&source_ref, Some(&[source_type]))?
    } else {
        // dangling configuration must stay editable: do not re-require the source here
        match reference_entity_id(&source_ref).filter(|_| reference_is_local(&source_ref)) {
            Some(id) => p.ov.entity(id)?,
            None => None,
        }
    };
    opt_text(&e.payload, "title", 256)?;
    if e.payload.get("options").is_some_and(|o| !o.is_object()) {
        return Err(bad("options must be an object"));
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

fn validate_container(p: &Planner, e: &mut EntityRow, before: Option<&EntityRow>) -> WsResult<()> {
    check_keys(&e.payload, &["kind", "layout", "title"], "container")?;
    let kind = e.payload.get("kind").and_then(Value::as_str).ok_or_else(|| bad("container needs kind"))?;
    match (kind, before) {
        ("page" | "group", None) => {}
        ("root", None) if p.env.internal => {}
        (_, None) => return Err(bad("container kind must be page or group")),
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
    opt_text(&e.payload, "title", 256)
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

/// `resolved` or `target_deleted`: anchors hold stable ids only and never re-attach.
pub fn anchor_state(ctx: &dyn ReadCtx, target: &Value) -> WsResult<&'static str> {
    let Some(id) = reference_entity_id(target) else { return Ok("target_deleted") };
    let Some(e) = ctx.entity(id)?.filter(|e| e.alive()) else { return Ok("target_deleted") };
    let s = &target["selector"];
    let record_ok = |rid: &str| -> WsResult<bool> { Ok(ctx.record(&e.entity_id, rid)?.is_some_and(|r| r.alive())) };
    let field_ok = |fid: &str| -> WsResult<bool> { Ok(ctx.field(&e.entity_id, fid)?.is_some_and(|f| f.alive())) };
    let ok = match s.get("kind").and_then(Value::as_str).unwrap_or("entity") {
        "entity" => true,
        "table_record" => record_ok(s["record_id"].as_str().unwrap_or(""))?,
        "table_field" => field_ok(s["field_id"].as_str().unwrap_or(""))?,
        "table_cell" => record_ok(s["record_id"].as_str().unwrap_or(""))? && field_ok(s["field_id"].as_str().unwrap_or(""))?,
        "richtext_block" => ctx
            .richtext(&e.entity_id)?
            .is_some_and(|rt| rt.meta.block_index.contains_key(s["block_id"].as_str().unwrap_or(""))),
        _ => false,
    };
    Ok(if ok { "resolved" } else { "target_deleted" })
}

fn validate_annotation(p: &Planner, e: &mut EntityRow, before: Option<&EntityRow>) -> WsResult<()> {
    check_keys(&e.payload, &["target", "kind", "body", "style", "author"], "annotation")?;
    match before {
        None => {
            let target = e.payload.get("target").ok_or_else(|| bad("annotation needs target"))?;
            let mut norm = normalize_reference(&json!({ "entity_id": target.get("entity_id").cloned().unwrap_or(Value::Null),
                                                         "selector": target.get("selector").cloned().unwrap_or(json!({ "kind": "entity" })) }))?;
            norm.as_object_mut().unwrap().remove("version");
            let kind = norm.get("selector").and_then(|s| s["kind"].as_str()).unwrap_or("entity").to_string();
            if !matches!(kind.as_str(), "entity" | "table_record" | "table_cell" | "table_field" | "richtext_block") {
                return Err(bad(format!("annotation anchor kind {kind} is not supported")));
            }
            p.check_ref_target(&json!({ "entity_id": norm["entity_id"] }), None)?;
            if !p.env.import && anchor_state(&p.ov, &norm)? != "resolved" {
                return Err(WsError::new(Code::ReferenceBroken, "annotation target does not exist"));
            }
            e.payload.insert("target".into(), norm);
            e.payload.insert("author".into(), json!(p.env.principal));
        }
        Some(b) => {
            if e.payload.get("target") != b.payload.get("target") || e.payload.get("author") != b.payload.get("author") {
                return Err(WsError::invalid_op("annotation target and author cannot be changed"));
            }
        }
    }
    match e.payload.get("kind").and_then(Value::as_str) {
        Some("note") | Some("highlight") => {}
        _ => return Err(bad("annotation kind must be note or highlight")),
    }
    opt_text(&e.payload, "body", 4000)?;
    if e.payload.get("style").is_some_and(|s| !s.is_object()) {
        return Err(bad("style must be an object"));
    }
    Ok(())
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
            validate_annotation(p, row, None)?;
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
        TYPE_ANNOTATION => validate_annotation(p, e, Some(before)),
        TYPE_TABLE => {
            // a URL table's query definition is local document state; its remote rows are not
            if changed.iter().any(|k| k == "source_ref") {
                p.require(&e.entity_id, Cap::Structure)?;
            }
            validate_table_meta(p, e, Some(before))
        }
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
    match e.type_id.as_str() {
        TYPE_CELL => {
            if let Some(r) = e.payload.get("source_ref") {
                out.insert(ref_edge(&e.entity_id, "", "bind", r));
            }
        }
        TYPE_ANNOTATION => {
            if let Some(t) = e.payload.get("target") {
                out.insert(ref_edge(&e.entity_id, "", "anchor", t));
            }
        }
        TYPE_ASSET => {
            if let Some(o) = e.payload.get("object_id").and_then(Value::as_str) {
                let mut edge = RefEdge::local(&e.entity_id, "", "asset", "");
                edge.dst_object_id = o.to_string();
                out.insert(edge);
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
