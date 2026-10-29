//! Sources and publication by path (内核架构改进 D12, D24, D26; M4).
//!
//! A data entity with a `source` attribute is read-only for Blocks and people: its content is written only by
//! a *refresh* — a run (`source.refresh@1`) whose candidate is validated and then committed with origin `source`,
//! one Source at a time (each refresh is its own atomic Commit). Publishing an entity grants the `public`
//! subject read on it and registers a name; the link is the URL projection of the reference object (D21), and
//! another Workspace inserts it as a `source{kind: workspace}`. Referrers register themselves (a weak record:
//! it never blocks an unpublish; the consumer then reads `unavailable`).

use crate::docdb::db_err;
use crate::workspace::{random_id, Caller, CommitOpts, Workspace, PUBLIC_SUBJECT};
use aiworkspace_core::access::Cap;
use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use aiworkspace_core::id::check_idempotency_key;
use aiworkspace_core::materialize::{content_object, ObjectSource};
use aiworkspace_core::model::*;
use aiworkspace_core::read::readable_entity;
use aiworkspace_core::types::{has_source, record_nested};
use aiworkspace_core::value::{format_utc_ms, FieldDef};
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const SOURCE_PROGRAM: &str = "source.refresh@1";
pub const PUB_FORMAT: &str = "buckyos.ai-workspace/pub/1.0";
/// Types that can carry a Source and be published.
pub const SOURCED_TYPES: &[&str] = &[TYPE_TABLE, TYPE_RICHTEXT, TYPE_RECORD, TYPE_ASSET, TYPE_DIR];
const RECORDS_PER_OP: usize = 1000;

/// What a fetch produced for one entity.
#[derive(Debug, Clone)]
pub enum Fetched {
    /// `fields`: FieldDef values; `records`: `{ record_id, values }`.
    Table { fields: Vec<Value>, records: Vec<Value> },
    RichText { doc: Value },
    Asset { bytes: Vec<u8>, file_name: Option<String> },
}

/// A name in a published link: lower-case ASCII letters, digits and dashes.
/// First path segments a Workspace title may not take on its own host (see `workspace_slug`).
pub const RESERVED_WORKSPACE_SLUGS: &[&str] = &["home", "kapi", "api", "s3", "bucket", "ndm", "nfs", "assets", "aiworkspace", "messagehub"];

pub fn pub_name(s: &str) -> String {
    let ascii: String = s.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    ascii.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-").chars().take(64).collect()
}

fn public_reader() -> Caller {
    Caller::service(PUBLIC_SUBJECT)
}

impl Workspace {
    // ---- publishing (D26, T4-4)

    /// `ws.publish { entity_id, name? }` (`manage`): the `public` subject may read the entity; the name (default:
    /// the entity's own, slugified) becomes the last segment of the link. Publishing again under another name
    /// keeps the old name as an alias, so old links keep resolving.
    pub fn publish_entity(&mut self, caller: &Caller, entity_id: &str, name: Option<&str>) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let now = self.now();
        let e = {
            let ctx = self.ctx(&now);
            ctx.entity(entity_id)?.filter(|e| e.alive()).ok_or_else(|| WsError::not_found(format!("entity {entity_id} not found")))?
        };
        if !SOURCED_TYPES.contains(&e.type_id.as_str()) {
            return Err(WsError::invalid_op(format!("{} cannot be published (data only)", e.type_id)));
        }
        let name = match name {
            Some(n) => pub_name(n),
            None => pub_name(e.name.as_deref().or(e.payload.get("title").and_then(Value::as_str)).unwrap_or(entity_id)),
        };
        let name = if name.is_empty() { pub_name(entity_id) } else { name };
        if name.is_empty() {
            return Err(WsError::invalid_op("the publication needs a name"));
        }
        let taken: Option<String> = self.local.query_row("SELECT entity_id FROM publications WHERE name = ?1", [&name], |r| r.get(0)).optional().map_err(db_err)?;
        if taken.as_deref().is_some_and(|t| t != entity_id) {
            return Err(WsError::sub(Code::InvalidOperation, "NAME_CONFLICT", format!("{name} already names another publication")));
        }
        self.grant_with(caller, PUBLIC_SUBJECT, Some(entity_id), &["read".to_string()], None, None)?;
        self.local.execute("UPDATE publications SET current = 0 WHERE entity_id = ?1", [entity_id]).map_err(db_err)?;
        self.local
            .execute("INSERT OR REPLACE INTO publications (name, entity_id, created_at, created_by, current) VALUES (?1, ?2, ?3, ?4, 1)", params![name, entity_id, now, caller.principal])
            .map_err(db_err)?;
        Ok(self.publication_json(&name, entity_id, &now, &caller.principal)?)
    }

    /// `ws.unpublish { entity_id }` (`manage`): every name of the entity goes, and so does the public grant;
    /// referrers keep their records and read `unavailable` from now on.
    pub fn unpublish(&mut self, caller: &Caller, entity_id: &str) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let n = self.local.execute("DELETE FROM publications WHERE entity_id = ?1", [entity_id]).map_err(db_err)?;
        self.local.execute("DELETE FROM grants WHERE subject = ?1 AND scope_entity_id = ?2", params![PUBLIC_SUBJECT, entity_id]).map_err(db_err)?;
        Ok(json!({ "ok": true, "entity_id": entity_id, "removed": n }))
    }

    fn publication_json(&self, name: &str, entity_id: &str, at: &str, by: &str) -> WsResult<Value> {
        let referrers: i64 = self.local.query_row("SELECT COUNT(*) FROM referrers WHERE entity_id = ?1", [entity_id], |r| r.get(0)).map_err(db_err)?;
        Ok(json!({ "ok": true, "entity_id": entity_id, "name": name, "path": format!("/{}/{}", self.workspace_slug(), name),
                   "reference": { "workspace_id": self.workspace_id, "entity_id": entity_id, "mode": "live" },
                   "published_at": at, "published_by": by, "referrers": referrers }))
    }

    /// The workspace's own segment of a link (D21): its title, slugified; the id is in the host. Empty, or one of
    /// the first segments the Zone gateway routes elsewhere on every host (`/home/*` → HomeStation, `/kapi/*`,
    /// `/s3/*`, `/bucket/*`, `/ndm/*`, `/nfs/*`) or the Desktop build's own directories, becomes `workspace`.
    pub fn workspace_slug(&self) -> String {
        let s = pub_name(&self.title());
        if s.is_empty() || RESERVED_WORKSPACE_SLUGS.contains(&s.as_str()) { "workspace".into() } else { s }
    }

    /// `ws.list_publications` (any read): names, entities and how many referrers registered.
    pub fn list_publications(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let mut st = self.local.prepare_cached("SELECT name, entity_id, created_at, created_by, current FROM publications ORDER BY created_at").map_err(db_err)?;
        let rows: Vec<(String, String, String, String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for (name, entity_id, at, by, current) in rows {
            if !ctx.entity(&entity_id)?.is_some_and(|e| access.can_read(&ctx, &e).unwrap_or(false)) {
                continue;
            }
            let mut v = self.publication_json(&name, &entity_id, &at, &by)?;
            v.as_object_mut().unwrap().remove("ok");
            v["current"] = json!(current == 1);
            out.push(v);
        }
        Ok(json!({ "ok": true, "publications": out }))
    }

    /// The entity a link's last segment names: a current name, an alias, or the entity id of a published entity.
    pub fn publication(&self, name_or_id: &str) -> WsResult<Option<(String, String)>> {
        let by_name: Option<(String, String)> = self.local.query_row("SELECT name, entity_id FROM publications WHERE name = ?1", [name_or_id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(db_err)?;
        if by_name.is_some() {
            return Ok(by_name);
        }
        self.local.query_row("SELECT name, entity_id FROM publications WHERE entity_id = ?1 AND current = 1", [name_or_id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(db_err)
    }

    /// The published content of a name (no session: the `public` subject reads it). `fixed` also names the
    /// entity's content object (D03 `{ content_root }`), the version a consumer can pin.
    pub fn public_content(&self, name_or_id: &str, fixed: bool, with_bytes: bool) -> WsResult<Value> {
        let (name, entity_id) = self.publication(name_or_id)?.ok_or_else(|| WsError::not_found("no such publication"))?;
        let reader = public_reader();
        let access = self.require_ws_any(&reader)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let e = readable_entity(&ctx, &access, &entity_id)?;
        if !e.alive() {
            return Err(WsError::not_found("no such publication"));
        }
        let content = match e.type_id.as_str() {
            TYPE_TABLE => {
                let fields: Vec<Value> = ctx.fields(&entity_id)?.into_iter().filter(|f| f.alive()).map(|f| f.def.to_value()).collect();
                let mut records = Vec::new();
                ctx.scan_records(&entity_id, &mut |r| {
                    if r.alive() {
                        records.push(json!({ "record_id": r.record_id, "values": r.values }));
                    }
                    Ok(true)
                })?;
                json!({ "fields": fields, "records": records })
            }
            TYPE_RICHTEXT => {
                let rt = ctx.richtext(&entity_id)?.ok_or_else(|| WsError::io("rich text state missing"))?;
                json!({ "doc": rt.meta.ast })
            }
            TYPE_RECORD => record_nested(&e.payload),
            TYPE_ASSET => {
                let mut c = Value::Object(e.payload.clone());
                if with_bytes {
                    use base64::Engine;
                    let bytes = self.objects.get_file(e.payload.get("object_id").and_then(Value::as_str).unwrap_or(""))?;
                    c["bytes"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
                }
                c
            }
            _ => Value::Object(e.payload.clone()),
        };
        let mut v = json!({ "format": PUB_FORMAT, "workspace_id": self.workspace_id, "entity_id": entity_id, "name": name, "title": e.payload.get("title").cloned().unwrap_or_else(|| json!(e.name)),
                            "type_id": e.type_id, "content_rev": e.content_rev, "head_seq": self.head_seq, "epoch": self.epoch,
                            "reference": { "workspace_id": self.workspace_id, "entity_id": entity_id, "mode": "live" }, "content": content });
        if fixed {
            let mut sink = crate::objects::StoreSink::new(&self.objects);
            let root = content_object(&ctx, &e, &mut sink)?;
            // a consumer pins this version: the owner's `exports` bucket keeps it a week past its last fixed read
            let mut members = std::collections::BTreeMap::new();
            self.objects.entity_members(&root, &e.type_id, &mut members)?;
            self.objects.retain(&self.bucket(crate::objects::EXPORTS)?, &format!("{}/fixed/{root}", self.workspace_id), &members)?;
            v["content_root"] = json!(root);
            v["reference"] = json!({ "workspace_id": self.workspace_id, "entity_id": entity_id, "mode": "fixed", "version": { "content_root": root } });
        }
        Ok(v)
    }

    /// The content a consumer pinned (`version.content_root`), read back from the named store: valid as long as
    /// it is there (its `exports` binding keeps it seven days after the last fixed read).
    pub fn public_fixed_content(&self, content_root: &str) -> WsResult<Value> {
        if !self.objects.has_object(content_root) {
            return Err(WsError::new(Code::VersionNotRetained, "that fixed version is no longer retained"));
        }
        let obj = aiworkspace_core::canonical::parse_strict(&self.objects.get_object(content_root)?)?;
        let mut content = obj["content"].clone();
        if obj["ws_type"] == json!(TYPE_TABLE) {
            if let Some(file) = content.get("records").and_then(|r| r.get("file")).and_then(Value::as_str) {
                let bytes = self.objects.get_file(file)?;
                let lines: Vec<Value> = bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).map_err(|e| WsError::io(e.to_string()))).collect::<WsResult<_>>()?;
                content["records"] = Value::Array(lines);
            }
        }
        Ok(json!({ "format": PUB_FORMAT, "workspace_id": self.workspace_id, "type_id": obj["ws_type"], "content_root": content_root, "content": content }))
    }

    // ---- referrers (D26, T4-6)

    /// A consumer registered that it reads `entity_id` (same service: at insert; other Zones: over HTTP). Weak.
    pub fn register_referrer(&self, entity_id: &str, referrer: &Value) -> WsResult<()> {
        let key = sha256_hex(canonical_json(referrer)?.as_bytes())[..32].to_string();
        self.local
            .execute("INSERT OR REPLACE INTO referrers (entity_id, referrer_key, referrer_json, registered_at) VALUES (?1, ?2, ?3, ?4)", params![entity_id, key, referrer.to_string(), self.now()])
            .map_err(db_err)?;
        Ok(())
    }

    /// `ws.list_referrers { entity_id }` (read on the entity): who registered as reading it.
    pub fn list_referrers(&self, caller: &Caller, entity_id: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        readable_entity(&self.ctx(&now), &access, entity_id)?;
        let mut st = self.local.prepare_cached("SELECT referrer_json, registered_at FROM referrers WHERE entity_id = ?1 ORDER BY registered_at").map_err(db_err)?;
        let rows: Vec<Value> = st
            .query_map([entity_id], |r| {
                let (j, at): (String, String) = (r.get(0)?, r.get(1)?);
                Ok(json!({ "referrer": serde_json::from_str::<Value>(&j).unwrap_or(Value::Null), "registered_at": at }))
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        Ok(json!({ "ok": true, "entity_id": entity_id, "referrers": rows }))
    }

    // ---- refresh runs (D12, D24, T4-1)

    /// The entity and its Source, when it has one.
    pub fn source_of(&self, entity_id: &str) -> WsResult<(EntityRow, Value)> {
        let now = self.now();
        let e = self.ctx(&now).entity(entity_id)?.filter(|e| e.alive()).ok_or_else(|| WsError::not_found(format!("entity {entity_id} not found")))?;
        if !has_source(&e) {
            return Err(WsError::invalid_op(format!("{entity_id} has no Source")));
        }
        let s = e.payload["source"].clone();
        Ok((e, s))
    }

    /// `proc.start { program: source.refresh@1, params: { entity_id } }`: a refresh run, `update` on the entity.
    pub fn source_refresh_start(&mut self, caller: &Caller, entity_id: &str, idem_key: &str) -> WsResult<Value> {
        check_idempotency_key(idem_key)?;
        let access = self.require_ws_any(caller)?;
        let (_, source) = self.source_of(entity_id)?;
        let now = self.now();
        if !access.can(&self.ctx(&now), entity_id, Cap::Update)? {
            return Err(WsError::denied("refreshing a Source needs update on its entity"));
        }
        let params = json!({ "entity_id": entity_id, "source": source });
        let existing: Option<String> = self.local.query_row("SELECT run_id FROM runs WHERE principal = ?1 AND idem_key = ?2", params![caller.principal, idem_key], |r| r.get(0)).optional().map_err(db_err)?;
        if let Some(run_id) = existing {
            let mut v = self.run_json(&run_id)?;
            v["ok"] = json!(true);
            v["replayed"] = json!(true);
            return Ok(v);
        }
        let run_id = random_id("run_");
        self.local
            .execute(
                "INSERT INTO runs (run_id, program, principal, idem_key, state, params_json, created_at, updated_at) VALUES (?1,?2,?3,?4,'queued',?5,?6,?6)",
                params![run_id, SOURCE_PROGRAM, caller.principal, idem_key, params.to_string(), now],
            )
            .map_err(db_err)?;
        let mut v = self.run_json(&run_id)?;
        v["ok"] = json!(true);
        Ok(v)
    }

    pub fn source_run(&self, run_id: &str) -> WsResult<Value> {
        self.run_json(run_id)
    }

    pub fn source_run_set(&self, run_id: &str, state: &str, result: Option<&Value>) -> WsResult<()> {
        self.set_run(run_id, state, result)
    }

    /// The fetched content becomes the entity's content in one Commit of origin `source` (D12): the diff
    /// against the current content, written as the run's principal; on any failure nothing changes and the
    /// run says why (§11.2: a failed refresh keeps the last valid version).
    pub fn source_apply(&mut self, run_id: &str, fetched: &Fetched, bytes: u64, content_root: Option<&str>) -> WsResult<Value> {
        let run = self.run_json(run_id)?;
        if run["state"] != json!("queued") && run["state"] != json!("running") {
            return Err(WsError::invalid_op(format!("run is {}", run["state"].as_str().unwrap_or("?"))));
        }
        let principal = run["principal"].as_str().unwrap_or("").to_string();
        let entity_id = run["params"]["entity_id"].as_str().unwrap_or("").to_string();
        let caller = Caller::user(&principal);
        let ops = match self.source_ops(&caller, &entity_id, fetched) {
            Ok(ops) => ops,
            Err(e) => {
                self.set_run(run_id, "failed", Some(&json!({ "error": e.to_json() })))?;
                return Err(e);
            }
        };
        let now = self.now();
        if ops.is_empty() {
            let result = json!({ "status": "unchanged", "bytes": bytes, "content_root": content_root, "at": now });
            self.set_run(run_id, "succeeded", Some(&result))?;
            local_set(&self.local, &format!("source_refreshed:{entity_id}"), &now)?;
            return self.run_json(run_id);
        }
        let req = json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch, "idempotency_key": format!("source/{run_id}"),
                          "origin": "program", "run_id": run_id, "message": format!("refresh from source ({} bytes)", bytes), "operations": ops });
        let opts = CommitOpts { as_source: true, as_principal: Some(principal), ..CommitOpts::default() };
        let mut result = self.commit(&req, &caller, &opts);
        result["bytes"] = json!(bytes);
        if let Some(r) = content_root {
            result["content_root"] = json!(r);
        }
        let accepted = result["status"] == json!("accepted");
        self.set_run(run_id, if accepted { "succeeded" } else { "failed" }, Some(&result))?;
        if accepted {
            local_set(&self.local, &format!("source_refreshed:{entity_id}"), &now)?;
        }
        self.run_json(run_id)
    }

    pub fn source_fail(&self, run_id: &str, error: &WsError) -> WsResult<()> {
        self.set_run(run_id, "failed", Some(&json!({ "error": error.to_json() })))
    }

    /// Operations that turn the current content into `fetched`.
    fn source_ops(&mut self, caller: &Caller, entity_id: &str, fetched: &Fetched) -> WsResult<Vec<Value>> {
        let now = self.now();
        let (e, fields, records, ast) = {
            let ctx = self.ctx(&now);
            let e = ctx.entity(entity_id)?.filter(|e| e.alive()).ok_or_else(|| WsError::not_found(format!("entity {entity_id} not found")))?;
            let fields: Vec<FieldRow> = if e.type_id == TYPE_TABLE { ctx.fields(entity_id)?.into_iter().filter(|f| f.alive()).collect() } else { Vec::new() };
            let mut records: BTreeMap<String, RecordRow> = BTreeMap::new();
            if e.type_id == TYPE_TABLE {
                ctx.scan_records(entity_id, &mut |r| {
                    if r.alive() {
                        records.insert(r.record_id.clone(), r.clone());
                    }
                    Ok(true)
                })?;
            }
            let ast = if e.type_id == TYPE_RICHTEXT { ctx.richtext(entity_id)? } else { None };
            (e, fields, records, ast)
        };
        let mut ops = Vec::new();
        match (e.type_id.as_str(), fetched) {
            (TYPE_TABLE, Fetched::Table { fields: new_fields, records: new_records }) => {
                let current: BTreeMap<String, &FieldRow> = fields.iter().map(|f| (f.field_id.clone(), f)).collect();
                let mut wanted: BTreeMap<String, FieldDef> = BTreeMap::new();
                for f in new_fields {
                    let def = FieldDef::from_value(f)?;
                    wanted.insert(def.field_id.clone(), def);
                }
                for (id, f) in &current {
                    match wanted.get(id) {
                        Some(def) if def.ty == f.def.ty => {}
                        _ => ops.push(json!({ "op": "table.delete_field", "source_id": entity_id, "field_id": id, "expect": { "rev": f.def_rev } })),
                    }
                }
                for (id, def) in &wanted {
                    if !current.get(id).is_some_and(|f| f.def.ty == def.ty) {
                        ops.push(json!({ "op": "table.add_field", "source_id": entity_id, "field": def.to_value() }));
                    }
                }
                let mut seen = std::collections::BTreeSet::new();
                let (mut inserts, mut sets, mut unsets) = (Vec::new(), Vec::new(), Vec::new());
                for r in new_records {
                    let rid = r["record_id"].as_str().ok_or_else(|| WsError::invalid_schema("record_id required"))?.to_string();
                    let values = r["values"].as_object().cloned().unwrap_or_default();
                    let values: Map<String, Value> = values.into_iter().filter(|(k, _)| wanted.contains_key(k)).collect();
                    seen.insert(rid.clone());
                    match records.get(&rid) {
                        None => inserts.push(json!({ "record_id": rid, "values": values })),
                        Some(cur) => {
                            for (fid, v) in &values {
                                let same_type = current.get(fid).is_some_and(|f| f.def.ty == wanted[fid].ty);
                                if cur.values.get(fid) != Some(v) && same_type {
                                    sets.push(json!({ "record_id": rid, "field_id": fid, "value": v, "expect": { "rev": cur.value_rev(fid) } }));
                                }
                            }
                            for fid in cur.values.keys() {
                                if !values.contains_key(fid) && current.get(fid).is_some_and(|f| wanted.get(fid).is_some_and(|w| w.ty == f.def.ty)) {
                                    unsets.push(json!({ "record_id": rid, "field_id": fid, "expect": { "rev": cur.value_rev(fid) } }));
                                }
                            }
                        }
                    }
                }
                let deletes: Vec<Value> = records.values().filter(|r| !seen.contains(&r.record_id)).map(|r| json!({ "record_id": r.record_id, "expect": { "rev": r.rev } })).collect();
                for chunk in deletes.chunks(RECORDS_PER_OP) {
                    ops.push(json!({ "op": "table.delete_records", "source_id": entity_id, "records": chunk }));
                }
                for chunk in sets.chunks(RECORDS_PER_OP) {
                    ops.push(json!({ "op": "table.set_values", "source_id": entity_id, "values": chunk }));
                }
                for chunk in unsets.chunks(RECORDS_PER_OP) {
                    ops.push(json!({ "op": "table.unset_values", "source_id": entity_id, "values": chunk }));
                }
                for chunk in inserts.chunks(RECORDS_PER_OP) {
                    ops.push(json!({ "op": "table.insert_records", "source_id": entity_id, "records": chunk }));
                }
            }
            (TYPE_RICHTEXT, Fetched::RichText { doc }) => {
                // a diff by block id: blocks the publisher kept are replaced when they changed, new ones are inserted
                // in order, gone ones are deleted last (the document never becomes empty on the way)
                let rt = ast.as_ref();
                let old_order: Vec<String> = rt.map(|rt| rt.meta.ast["content"].as_array().into_iter().flatten().filter_map(|b| b["attrs"]["block_id"].as_str().map(str::to_string)).collect()).unwrap_or_default();
                let old_node = |id: &str| rt.and_then(|rt| rt.meta.ast["content"].as_array()).into_iter().flatten().find(|b| b["attrs"]["block_id"].as_str() == Some(id)).cloned();
                let expect_of = |id: &str| rt.and_then(|rt| rt.meta.block_index.get(id)).map(|info| json!({ "hash": info.hash, "struct_rev": info.struct_rev }));
                let mut new_blocks: Vec<Value> = doc["content"].as_array().cloned().unwrap_or_default();
                for (i, b) in new_blocks.iter_mut().enumerate() {
                    if b["attrs"]["block_id"].as_str().is_none() {
                        b["attrs"]["block_id"] = json!(format!("src-{i}"));
                    }
                }
                let new_ids: Vec<String> = new_blocks.iter().filter_map(|b| b["attrs"]["block_id"].as_str().map(str::to_string)).collect();
                let mut prev: Option<String> = None;
                for b in &new_blocks {
                    let id = b["attrs"]["block_id"].as_str().unwrap_or("").to_string();
                    match (old_order.contains(&id), expect_of(&id)) {
                        (true, Some(expect)) => {
                            if old_node(&id).as_ref() != Some(b) {
                                ops.push(json!({ "op": "richtext.replace_block", "entity_id": entity_id, "block_id": id, "node": b, "expect": expect }));
                            }
                        }
                        _ => {
                            let position = match &prev { Some(p) => json!({ "after": p }), None => json!({ "start": true }) };
                            ops.push(json!({ "op": "richtext.insert_blocks", "entity_id": entity_id, "position": position, "blocks": [b] }));
                        }
                    }
                    prev = Some(id);
                }
                let gone: Vec<Value> = old_order.iter().filter(|id| !new_ids.contains(id)).filter_map(|id| expect_of(id).map(|e| json!({ "block_id": id, "expect": e }))).collect();
                if !gone.is_empty() {
                    ops.push(json!({ "op": "richtext.delete_blocks", "entity_id": entity_id, "blocks": gone }));
                }
            }
            (TYPE_ASSET, Fetched::Asset { bytes, file_name }) => {
                let staged = self.stage_asset(caller, bytes)?;
                let object_id = staged["object_id"].clone();
                if e.payload.get("object_id") != Some(&object_id) {
                    let mut keys = vec![json!({ "key": "object_id", "value": object_id, "expect": { "rev": e.key_rev("object_id") } })];
                    if let Some(n) = file_name {
                        keys.push(json!({ "key": "file_name", "value": n, "expect": { "rev": e.key_rev("file_name") } }));
                    }
                    ops.push(json!({ "op": "entity.set_keys", "entity_id": entity_id, "keys": keys }));
                }
            }
            (t, _) => return Err(WsError::invalid_op(format!("fetched content does not fit a {t}"))),
        }
        Ok(ops)
    }

    /// Sourced entities whose `refresh.interval_s` elapsed since their last successful refresh (the timer's list).
    pub fn due_sources(&self) -> WsResult<Vec<String>> {
        let mut st = self.doc.prepare_cached("SELECT entity_id, payload_json FROM entities WHERE deleted_seq IS NULL AND payload_json LIKE '%\"refresh\"%'").map_err(db_err)?;
        let rows: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        let now_ms = (self.clock)();
        let mut due = Vec::new();
        for (id, payload) in rows {
            let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            let Some(secs) = p["source"]["refresh"]["interval_s"].as_u64() else { continue };
            let last = local_get(&self.local, &format!("source_refreshed:{id}"))?.and_then(|s| aiworkspace_core::value::parse_datetime_ms(&s));
            if last.map_or(true, |l| now_ms - l >= secs as i64 * 1000) {
                due.push(id);
            }
        }
        Ok(due)
    }

    // ---- traffic (D24, D19)

    /// Bytes fetched for this Workspace's Sources (cumulative); refused beyond `traffic_quota`.
    pub fn add_traffic(&self, bytes: u64) -> WsResult<u64> {
        let used = self.traffic()? + bytes;
        if let Some(q) = self.traffic_quota {
            if used > q {
                return Err(WsError::new(Code::QuotaLimit, format!("the workspace's Source traffic quota ({q} bytes) is used up"))
                    .with_data(json!({ "quota_bytes": q, "used_bytes": used, "owner": self.owner()?.unwrap_or_default() })));
            }
        }
        local_set(&self.local, "traffic_bytes", &used.to_string())?;
        Ok(used)
    }

    pub fn traffic(&self) -> WsResult<u64> {
        Ok(local_get(&self.local, "traffic_bytes")?.and_then(|s| s.parse().ok()).unwrap_or(0))
    }
}

impl Workspace {
    /// The Surface a link names (D21 `/<workspace_name>/<surface_name>`): by name, else by title.
    pub fn surface_by_name(&self, name: &str) -> WsResult<Option<String>> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let wanted = pub_name(name);
        for edge in ctx.children(SURFACES_ID)? {
            let Some(e) = ctx.entity(&edge.child_id)?.filter(|e| e.alive()) else { continue };
            let title = e.payload.get("title").and_then(Value::as_str).unwrap_or("");
            if e.name.as_deref() == Some(name) || e.entity_id == name || pub_name(title) == wanted || pub_name(e.name.as_deref().unwrap_or("")) == wanted {
                return Ok(Some(e.entity_id));
            }
        }
        Ok(None)
    }
}

fn local_get(conn: &rusqlite::Connection, key: &str) -> WsResult<Option<String>> {
    conn.query_row("SELECT value FROM local_meta WHERE key = ?1", [key], |r| r.get(0)).optional().map_err(db_err)
}

fn local_set(conn: &rusqlite::Connection, key: &str, value: &str) -> WsResult<()> {
    conn.execute("INSERT OR REPLACE INTO local_meta (key, value) VALUES (?1, ?2)", params![key, value]).map_err(db_err)?;
    Ok(())
}

/// `source_refreshed:<id>` is a `format_utc_ms` timestamp.
pub fn refreshed_at(ws: &Workspace, entity_id: &str) -> WsResult<Option<String>> {
    local_get(&ws.local, &format!("source_refreshed:{entity_id}"))
}

#[allow(dead_code)]
fn _format(ms: i64) -> String {
    format_utc_ms(ms)
}
