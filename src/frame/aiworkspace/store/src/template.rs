//! Workspace templates (扩展控件 P3, Q32; README “Workspace templates”): a template is one `materialize` of chosen
//! content — Surfaces with what their Blocks reference, the control definitions they use, chosen data — carried in a
//! control package as `templates/<id>/{snapshot.json, objects/, chunks/}` and loaded as a new Workspace, never merged
//! into an existing one. Ids are kept: the snapshot is a whole Workspace of its own.

use crate::ext::content_digest;
use crate::workspace::{random_id, Caller, CommitOpts, Workspace};
use aiworkspace_core::access::Cap;
use aiworkspace_core::canonical::{obj_id_from_filename, obj_id_to_filename, parse_strict};
use aiworkspace_core::materialize::{materialize_with, ObjectSource};
use aiworkspace_core::model::*;
use aiworkspace_core::{Code, WsError, WsResult};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// `snapshot.json` `format`.
pub const SNAPSHOT_FORMAT: &str = "aiws-template/1";
const PARAM_TYPES: &[&str] = &["string", "number", "boolean"];

fn invalid(detail: impl Into<String>) -> WsError {
    WsError::new(Code::LibraryPackageInvalid, detail)
}

/// A template id: `[a-z0-9][a-z0-9-]{0,63}`.
pub fn is_template_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 64 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// `params { <k>: { type, title?, default? } }` of a template.
pub fn check_params(v: &Value) -> WsResult<()> {
    let o = v.as_object().filter(|o| o.len() <= 32).ok_or_else(|| WsError::invalid_op("params: at most 32 { type, title?, default? }"))?;
    for (k, p) in o {
        let ty = p.get("type").and_then(Value::as_str).filter(|t| PARAM_TYPES.contains(t));
        let ok = aiworkspace_core::types::is_binding_name(k)
            && ty.is_some()
            && p.as_object().is_some_and(|po| po.keys().all(|x| matches!(x.as_str(), "type" | "title" | "default")))
            && p.get("title").is_none_or(|t| t.as_str().is_some_and(|t| t.chars().count() <= 256))
            && p.get("default").is_none_or(|d| match ty {
                Some("string") => d.is_string(),
                Some("number") => d.is_number(),
                _ => d.is_boolean(),
            });
        if !ok {
            return Err(WsError::invalid_op(format!("params.{k}: a name [a-z][a-z0-9_]{{0,31}} with {{ type: string | number | boolean, title?, default? }}")));
        }
    }
    Ok(())
}

/// The value `{{params.<k>}}` stands for: the given one, else the declared default, else an empty text.
fn param_value(k: &str, given: &Map<String, Value>, declared: &Map<String, Value>) -> Value {
    given.get(k).cloned().or_else(|| declared.get(k).and_then(|d| d.get("default")).cloned()).unwrap_or(json!(""))
}

/// `v` with every `{{params.<k>}}` replaced; a string that is exactly one placeholder takes the value's type.
pub fn substitute(v: &Value, given: &Map<String, Value>, declared: &Map<String, Value>) -> Value {
    match v {
        Value::String(t) if t.contains("{{params.") => {
            let trimmed = t.trim();
            if let Some(k) = trimmed.strip_prefix("{{params.").and_then(|r| r.strip_suffix("}}")).filter(|k| !k.contains('}')) {
                return param_value(k.trim(), given, declared);
            }
            let mut out = String::new();
            let mut rest = t.as_str();
            while let Some(i) = rest.find("{{params.") {
                out.push_str(&rest[..i]);
                let tail = &rest[i + 9..];
                let Some(end) = tail.find("}}") else {
                    out.push_str(&rest[i..]);
                    rest = "";
                    break;
                };
                match param_value(tail[..end].trim(), given, declared) {
                    Value::String(s) => out.push_str(&s),
                    other => out.push_str(&other.to_string()),
                }
                rest = &tail[end + 2..];
            }
            out.push_str(rest);
            Value::String(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, given, declared)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), substitute(x, given, declared))).collect()),
        other => other.clone(),
    }
}

/// One template of a bundle, checked: its manifest item and its files by name relative to `templates/<id>/`.
pub struct TemplateFiles {
    pub item: Value,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl TemplateFiles {
    /// The content root its snapshot names.
    pub fn content_root(&self) -> WsResult<String> {
        let snap: Value = serde_json::from_slice(self.files.get("snapshot.json").ok_or_else(|| invalid("snapshot.json missing"))?).map_err(|e| invalid(format!("snapshot.json: {e}")))?;
        if snap["format"] != json!(SNAPSHOT_FORMAT) {
            return Err(invalid(format!("snapshot.json: format must be {SNAPSHOT_FORMAT}")));
        }
        snap["content_root"].as_str().map(str::to_string).ok_or_else(|| invalid("snapshot.json: content_root missing"))
    }

    /// Put its NamedObjects and chunks into `store` (each verified against its name): what `load_workspace` reads.
    pub fn load_into(&self, store: &crate::objects::ObjectStore) -> WsResult<()> {
        for (name, bytes) in &self.files {
            if let Some(f) = name.strip_prefix("objects/") {
                let id = obj_id_from_filename(f).ok_or_else(|| invalid(format!("bad object file name {f}")))?;
                store.put_verified(&id, std::str::from_utf8(bytes).map_err(|_| invalid(format!("{name} is not UTF-8")))?).map_err(|e| invalid(format!("{name}: {}", e.detail)))?;
            } else if let Some(f) = name.strip_prefix("chunks/") {
                let id = obj_id_from_filename(f).ok_or_else(|| invalid(format!("bad chunk file name {f}")))?;
                store.put_chunk_verified(&id, bytes).map_err(|e| invalid(format!("{name}: {}", e.detail)))?;
            }
        }
        Ok(())
    }
}

/// Check the `workspace_templates` of a bundle (README “Workspace templates”): each item's shape, its digest over the
/// files under `templates/<id>/`, the snapshot naming a content root whose object is present, and `requires.controls`
/// among `controls` (the manifest's identities). Answers each template with its files.
pub fn verify_templates(items: &[Value], files: &BTreeMap<String, Vec<u8>>, controls: &[&str]) -> WsResult<Vec<TemplateFiles>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for t in items {
        let o = t.as_object().ok_or_else(|| invalid("a workspace_templates entry is an object"))?;
        if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "id" | "title" | "description" | "snapshot" | "digest" | "requires" | "params" | "reset")) {
            return Err(invalid(format!("workspace_templates: unknown key {k}")));
        }
        let id = o.get("id").and_then(Value::as_str).filter(|i| is_template_id(i)).ok_or_else(|| invalid("a template id is [a-z0-9][a-z0-9-]{0,63}"))?;
        if !seen.insert(id.to_string()) {
            return Err(invalid(format!("template {id} is listed twice")));
        }
        if !o.get("title").and_then(Value::as_str).is_some_and(|s| !s.is_empty() && s.chars().count() <= 256) || o.get("description").is_some_and(|d| !d.as_str().is_some_and(|d| d.chars().count() <= 4000)) {
            return Err(invalid(format!("template {id}: title 1…256 characters, description at most 4000")));
        }
        let dir = format!("templates/{id}/");
        if o.get("snapshot").and_then(Value::as_str) != Some(format!("{dir}snapshot.json").as_str()) {
            return Err(invalid(format!("template {id}: snapshot must be {dir}snapshot.json")));
        }
        if let Some(p) = o.get("params") {
            check_params(p).map_err(|e| invalid(format!("template {id}: {}", e.detail)))?;
        }
        let reset_ok = o.get("reset").is_none_or(|r| {
            r.as_object().is_some_and(|ro| ro.keys().all(|k| k == "tables")) && r.get("tables").is_none_or(|t| t.as_array().is_some_and(|a| a.len() <= 64 && a.iter().all(|x| x.as_str().is_some_and(aiworkspace_core::id::is_valid_id))))
        });
        if !reset_ok {
            return Err(invalid(format!("template {id}: reset is {{ tables: [entity ids] }}")));
        }
        let requires = o.get("requires").and_then(Value::as_object).ok_or_else(|| invalid(format!("template {id}: requires {{ controls, host_api? }} required")))?;
        for c in requires.get("controls").and_then(Value::as_array).ok_or_else(|| invalid(format!("template {id}: requires.controls required")))? {
            if !c.as_str().is_some_and(|c| controls.contains(&c)) {
                return Err(invalid(format!("template {id} requires {c}, which the package does not carry")));
            }
        }
        let mine: BTreeMap<String, Vec<u8>> = files.range(dir.clone()..).take_while(|(p, _)| p.starts_with(&dir)).map(|(p, b)| (p[dir.len()..].to_string(), b.clone())).collect();
        let digest = content_digest(mine.iter().map(|(n, b)| (n.as_str(), b.as_slice())));
        if o.get("digest").and_then(Value::as_str) != Some(digest.as_str()) {
            return Err(invalid(format!("{dir}: the files do not match the template digest (tampered or incomplete)")));
        }
        if let Some(n) = mine.keys().find(|n| *n != "snapshot.json" && !n.starts_with("objects/") && !n.starts_with("chunks/")) {
            return Err(invalid(format!("{dir}{n} is neither the snapshot, an object nor a chunk")));
        }
        let tf = TemplateFiles { item: t.clone(), files: mine };
        let root = tf.content_root()?;
        let root_file = format!("objects/{}", obj_id_to_filename(&root).ok_or_else(|| invalid(format!("template {id}: bad content root")))?);
        if !tf.files.contains_key(&root_file) {
            return Err(invalid(format!("template {id}: its content root is not among its objects")));
        }
        out.push(tf);
    }
    Ok(out)
}

/// Reference kinds a template follows: what an import would refuse to leave dangling (strong local references).
fn follows(r: &RefEdge) -> bool {
    r.blocks_delete() && r.dst_workspace_id.is_empty() && !r.dst_entity_id.is_empty()
}

impl Workspace {
    /// `ext.export_template { surfaces, data?, template: { id, title, description?, params?, reset? } }` (README “Workspace
    /// templates”): the template's manifest item, the control definitions it needs (`controls`: entity ids, for the
    /// package) and its files (`templates/<id>/…`) by bundle path.
    pub fn ext_export_template(&self, caller: &Caller, surfaces: &[String], data: &[String], spec: &Value) -> WsResult<(Value, Vec<String>, BTreeMap<String, Vec<u8>>)> {
        let access = self.require_ws_any(caller)?;
        let id = spec.get("id").and_then(Value::as_str).filter(|i| is_template_id(i)).ok_or_else(|| WsError::invalid_op("template.id: [a-z0-9][a-z0-9-]{0,63}"))?;
        let title = spec.get("title").and_then(Value::as_str).filter(|t| !t.is_empty() && t.chars().count() <= 256).ok_or_else(|| WsError::invalid_op("template.title: 1…256 characters"))?;
        if let Some(k) = spec.as_object().into_iter().flatten().map(|(k, _)| k).find(|k| !matches!(k.as_str(), "id" | "title" | "description" | "params" | "reset")) {
            return Err(WsError::invalid_op(format!("template: unknown key {k}")));
        }
        if let Some(p) = spec.get("params") {
            check_params(p)?;
        }
        if surfaces.is_empty() || surfaces.len() > 64 || data.len() > 256 {
            return Err(WsError::invalid_op("surfaces: 1…64 Surfaces; data: at most 256 entities"));
        }
        let now = self.now();
        let ctx = self.ctx(&now);
        let alive = |id: &str| -> WsResult<EntityRow> {
            ctx.entity(id)?.filter(|e| e.alive() && e.scope == SCOPE_SHARED && access.can_read(&ctx, e).unwrap_or(false)).ok_or_else(|| WsError::not_found(format!("{id} not found")))
        };
        let mut queue: Vec<String> = Vec::new();
        for s in surfaces {
            let e = alive(s)?;
            if e.type_id != TYPE_CONTAINER || e.payload.get("kind").and_then(Value::as_str) != Some("surface") {
                return Err(WsError::invalid_op(format!("{s} is not a Surface")));
            }
            if !access.can(&ctx, s, Cap::Export)? {
                return Err(WsError::denied(format!("export capability required on {s}")));
            }
            queue.push(s.clone());
            if let Some(f) = e.payload.get("content_folder_id").and_then(Value::as_str) {
                queue.push(f.to_string());
            }
        }
        for d in data {
            if is_system_id(d) {
                return Err(WsError::invalid_op(format!("{d} is a system folder: choose what is in it")));
            }
            alive(d)?;
            queue.push(d.clone());
        }
        // what the chosen content holds and references (strongly), never the presentation paths or a session
        let mut set: BTreeSet<String> = BTreeSet::new();
        while let Some(cur) = queue.pop() {
            if cur == SESSION_ID || set.contains(&cur) {
                continue;
            }
            let Some(e) = ctx.entity(&cur)?.filter(|e| e.alive() && e.scope == SCOPE_SHARED) else { continue };
            if !access.can_read(&ctx, &e)? {
                continue;
            }
            set.insert(cur.clone());
            for edge in ctx.children(&cur)? {
                queue.push(edge.child_id);
            }
            for r in ctx.refs_from(&cur)? {
                if follows(&r) {
                    queue.push(r.dst_entity_id.clone());
                }
            }
        }
        // the system nodes and the parents of everything chosen, without the rest of their content
        for sys in [ROOT_ID, DATA_ID, SURFACES_ID, SHOWS_ID, CANVAS_CONTENT_ID] {
            if ctx.entity(sys)?.is_some_and(|e| e.alive()) {
                set.insert(sys.to_string());
            }
        }
        for id in set.clone() {
            let mut cur = id;
            while let Some(edge) = ctx.edge(&cur)? {
                if edge.parent_id == SHOWS_ID || edge.parent_id == SESSION_ID {
                    break;
                }
                set.insert(edge.parent_id.clone());
                cur = edge.parent_id;
            }
        }
        set.retain(|id| !self.under(&ctx, id, SHOWS_ID).unwrap_or(false) || id == SHOWS_ID);
        let mut sink = crate::objects::StoreSink::new(&self.objects);
        let visible = |e: &EntityRow| Ok(set.contains(&e.entity_id));
        let m = materialize_with(&ctx, &mut sink, &visible, &mut |_, _| None, true)?;
        let (objects, chunks, missing) = crate::export::closure(&self.objects, &m.content_root, true)?;
        if !missing.is_empty() {
            return Err(WsError::new(Code::DependencyUnavailable, format!("{} asset(s) of the template are not available here", missing.len())).with_data(json!({ "missing": missing })));
        }
        let mut rel: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for oid in &objects {
            rel.insert(format!("objects/{}", obj_id_to_filename(oid).ok_or_else(|| WsError::io("object id without a file name"))?), self.objects.get_object(oid)?.into_bytes());
        }
        for cid in &chunks {
            rel.insert(format!("chunks/{}", obj_id_to_filename(cid).ok_or_else(|| WsError::io("chunk id without a file name"))?), self.objects.get_chunk(cid)?);
        }
        let mut snapshot = serde_json::to_string_pretty(&json!({ "format": SNAPSHOT_FORMAT, "content_root": m.content_root, "entities": m.objects.len() })).map_err(|e| WsError::io(e.to_string()))?;
        snapshot.push('\n');
        rel.insert("snapshot.json".into(), snapshot.into_bytes());
        // what the snapshot's definitions are, by identity (the package must carry them), and the host APIs they speak
        let (mut controls, mut idents) = (Vec::new(), Vec::new());
        let (mut html, mut decl) = (0u64, 0u64);
        for eid in m.objects.keys() {
            let Some(e) = ctx.entity(eid)? else { continue };
            if e.type_id != TYPE_BLOCK_DEF {
                continue;
            }
            match e.payload.get("kind").and_then(Value::as_str) {
                Some("html") => html = html.max(e.payload.get("html").and_then(|h| h.get("api_version")).and_then(Value::as_u64).unwrap_or(aiworkspace_core::types::HTML_API_VERSION)),
                Some("declarative") => decl = decl.max(crate::ext::DECLARATIVE_API_VERSION),
                _ => {}
            }
            if let Some(ident) = e.payload.get("ident").and_then(|i| aiworkspace_core::types::ident_string(i).ok()) {
                controls.push(e.entity_id.clone());
                idents.push(ident);
            }
        }
        idents.sort();
        let mut host_api = Map::new();
        if html > 0 {
            host_api.insert("html".into(), json!(html));
        }
        if decl > 0 {
            host_api.insert("declarative".into(), json!(decl));
        }
        let mut item = json!({ "id": id, "title": title, "snapshot": format!("templates/{id}/snapshot.json"),
                               "digest": content_digest(rel.iter().map(|(n, b)| (n.as_str(), b.as_slice()))), "requires": { "host_api": host_api, "controls": idents } });
        for k in ["description", "params"] {
            if let Some(v) = spec.get(k) {
                item[k] = v.clone();
            }
        }
        if let Some(tables) = spec.get("reset").and_then(|r| r.get("tables")).and_then(Value::as_array) {
            for t in tables {
                let tid = t.as_str().unwrap_or("");
                if !set.contains(tid) || ctx.entity(tid)?.is_none_or(|e| e.type_id != TYPE_TABLE) {
                    return Err(WsError::invalid_op(format!("reset.tables: {tid} is not a table of the template")));
                }
            }
            item["reset"] = json!({ "tables": tables });
        }
        let files = rel.into_iter().map(|(n, b)| (format!("templates/{id}/{n}"), b)).collect();
        Ok((item, controls, files))
    }

    fn under(&self, ctx: &dyn ReadCtx, id: &str, ancestor: &str) -> WsResult<bool> {
        let mut cur = id.to_string();
        for _ in 0..4096 {
            match ctx.edge(&cur)? {
                Some(e) if e.parent_id == ancestor => return Ok(true),
                Some(e) => cur = e.parent_id,
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    /// The first Commit of a Workspace created from a template (internal): `{{params.<k>}}` replaced in titles, Block
    /// and container `config`, record properties and table values; the rows of `reset.tables` removed.
    pub fn template_finish(&mut self, caller: &Caller, item: &Value, given: &Map<String, Value>) -> WsResult<()> {
        let declared = item.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
        let reset: Vec<String> = item.get("reset").and_then(|r| r.get("tables")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let now = self.now();
        let mut ops = Vec::new();
        {
            let ctx = self.ctx(&now);
            for (e, _) in aiworkspace_core::materialize::shared_entities(&ctx)? {
                if is_system_id(&e.entity_id) {
                    continue;
                }
                let mut keys = Vec::new();
                for (k, v) in &e.payload {
                    let replaceable = k == "title" || (k == "config" && matches!(e.type_id.as_str(), TYPE_CELL | TYPE_CONTAINER)) || (e.type_id == TYPE_RECORD && k.starts_with("p:"));
                    if !replaceable || e.type_id == TYPE_RICHTEXT {
                        continue;
                    }
                    let next = substitute(v, given, &declared);
                    if &next != v {
                        keys.push(json!({ "key": k, "value": next, "expect": "any" }));
                    }
                }
                if !keys.is_empty() {
                    ops.push(json!({ "op": "entity.set_keys", "entity_id": e.entity_id, "keys": keys }));
                }
                if e.type_id != TYPE_TABLE {
                    continue;
                }
                let mut rows = Vec::new();
                let mut values = Vec::new();
                ctx.scan_records(&e.entity_id, &mut |r| {
                    if !r.alive() {
                        return Ok(true);
                    }
                    if reset.contains(&e.entity_id) {
                        rows.push(json!({ "record_id": r.record_id, "expect": "any" }));
                        return Ok(true);
                    }
                    for (fid, v) in &r.values {
                        if v.as_str().is_some_and(|t| t.contains("{{params.")) {
                            values.push(json!({ "record_id": r.record_id, "field_id": fid, "value": substitute(v, given, &declared), "expect": "any" }));
                        }
                    }
                    Ok(true)
                })?;
                if !rows.is_empty() {
                    ops.push(json!({ "op": "table.delete_records", "source_id": e.entity_id, "records": rows }));
                }
                if !values.is_empty() {
                    ops.push(json!({ "op": "table.set_values", "source_id": e.entity_id, "values": values }));
                }
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        let req = json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch, "idempotency_key": random_id("template/"),
                          "origin": "system", "message": "create from template", "operations": ops });
        let r = self.commit(&req, caller, &CommitOpts { internal: true, ..Default::default() });
        if r["status"] != json!("accepted") {
            return Err(WsError::invalid_op("the template's parameters could not be applied").with_data(r));
        }
        Ok(())
    }
}

/// Parse the item and files a client or the library hands over for creation.
pub fn parse_template(item: &Value, files: BTreeMap<String, Vec<u8>>) -> WsResult<TemplateFiles> {
    let tf = TemplateFiles { item: item.clone(), files };
    tf.content_root()?;
    Ok(tf)
}

/// What a content root is made of, read back for `load_workspace`: present in `store`.
pub fn root_present(store: &dyn ObjectSource, root: &str) -> bool {
    store.get_object(root).ok().and_then(|t| parse_strict(&t).ok()).is_some()
}
