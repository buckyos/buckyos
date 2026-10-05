//! Read-side service methods: resolve, read, list, query, collaboration state.

use crate::docdb::db_err;
use crate::urlsource::{SourceQuery, SourceRegistry};
use crate::workspace::{Caller, Workspace};
use aiworkspace_core::access::Cap;
use aiworkspace_core::filter::{run_query, QuerySpec};
use aiworkspace_core::model::*;
use aiworkspace_core::read;
use aiworkspace_core::value::{normalize_reference, reference_entity_id, reference_is_local};
use aiworkspace_core::{richtext, types, Code, WsError, WsResult};
use base64::Engine;
use serde_json::{json, Value};

pub const QUERY_LIMIT_MAX: usize = 1000;

impl Workspace {
    fn decorate(&self, v: &mut Value, now: &str) {
        let id = v["entity_id"].as_str().unwrap_or("").to_string();
        if v["write_policy"] == json!(POLICY_LOCK) {
            v["lock_holder"] = self.lock_holder(&id, now).unwrap_or(Value::Null);
        }
        if v["type_id"] == json!(TYPE_ASSET) {
            if let Some(obj) = v["content"]["payload"]["object_id"].as_str().map(str::to_string) {
                // a read-time diagnosis; a missing asset never blocks opening the document
                v["content"]["availability"] = json!(self.objects.availability(&obj).as_str());
            }
        }
    }

    /// `doc.read`
    pub fn read(&self, caller: &Caller, entity_id: &str, selector: Option<&Value>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let mut v = read::read(&self.ctx(&now), &access, entity_id, selector)?;
        self.decorate(&mut v, &now);
        v["head_seq"] = json!(self.head_seq);
        Ok(v)
    }

    /// `doc.list_children`
    pub fn list_children(&self, caller: &Caller, parent_id: &str, include_deleted: bool) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let mut children = read::list_children(&self.ctx(&now), &access, parent_id, include_deleted)?;
        for c in &mut children {
            self.decorate(c, &now);
        }
        Ok(json!({ "ok": true, "parent_id": parent_id, "children": children, "head_seq": self.head_seq }))
    }

    /// The whole readable tree as envelopes (the "outline"): one call for UIs.
    pub fn outline(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        let mut stack = vec![ROOT_ID.to_string()];
        while let Some(id) = stack.pop() {
            let Some(e) = ctx.entity(&id)? else { continue };
            if !e.alive() || !access.can_read(&ctx, &e)? {
                continue;
            }
            let mut env = read::envelope(&ctx, &access, &e)?;
            self.decorate(&mut env, &now);
            // enough of the content to draw an outline without N further reads
            env["title"] = e.payload.get("title").cloned().unwrap_or(Value::Null);
            if e.type_id == TYPE_CONTAINER {
                env["kind"] = e.payload.get("kind").cloned().unwrap_or(Value::Null);
                env["layout"] = e.payload.get("layout").cloned().unwrap_or(Value::Null);
            }
            if e.type_id == TYPE_CELL {
                env["view_type"] = e.payload.get("view").and_then(|v| v.get("type")).cloned().unwrap_or(Value::Null);
                env["source_id"] = e.payload.get("source_ref").and_then(|r| r.get("entity_id")).cloned().unwrap_or(Value::Null);
            }
            out.push(env);
            for edge in ctx.children(&id)?.into_iter().rev() {
                stack.push(edge.child_id);
            }
        }
        Ok(json!({ "ok": true, "epoch": self.epoch, "head_seq": self.head_seq, "entities": out }))
    }

    /// `doc.resolve`: a reference or a path → resolved reference with the full
    /// `workspace_id`, type, version and the caller's capabilities on it.
    pub fn resolve(&self, caller: &Caller, reference: Option<&Value>, path: Option<&str>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let reference = match (reference, path) {
            (Some(r), _) => normalize_reference(r)?,
            (None, Some(p)) => json!({ "entity_id": read::resolve_path(&ctx, &access, p)?, "version": { "mode": "live_head" } }),
            _ => return Err(WsError::invalid_op("reference or path required")),
        };
        if !reference_is_local(&reference) && reference["workspace_id"] != json!(self.workspace_id) {
            return Err(WsError::new(Code::DependencyUnavailable, "cross-workspace references are resolved by their own workspace"));
        }
        match reference["version"]["mode"].as_str() {
            Some("live_head") => {}
            // kept as its own structure; never silently degraded to the live head
            Some("published_channel") => return Err(WsError::new(Code::UnsupportedVersion, "published channels are not available in this phase")),
            Some("fixed_revision") => {
                let obj = reference["version"]["object_id"].as_str().ok_or_else(|| {
                    WsError::new(Code::UnsupportedVersion, "fixed source revisions need a source that keeps history")
                })?;
                let id = reference_entity_id(&reference).unwrap_or("");
                read::readable_entity(&ctx, &access, id)?;
                let known: Option<u64> = self
                    .doc
                    .query_row("SELECT content_rev FROM entity_versions WHERE entity_id = ?1 AND object_id = ?2", [id, obj], |r| r.get(0))
                    .ok();
                if known.is_none() || !self.objects.has_object(obj) {
                    return Err(WsError::new(Code::ReferenceBroken, "that fixed version is not available here"));
                }
                use aiworkspace_core::materialize::ObjectSource;
                let content = aiworkspace_core::canonical::parse_strict(&self.objects.get_object(obj)?)?;
                return Ok(json!({ "ok": true, "reference": with_ws(&reference, &self.workspace_id), "object_id": obj,
                                  "content_rev": known, "fixed": true, "object": content }));
            }
            _ => return Err(WsError::invalid_op("unknown version mode")),
        }
        let id = reference_entity_id(&reference).unwrap_or("");
        let e = read::readable_entity(&ctx, &access, id)?;
        if !e.alive() {
            return Err(WsError::deleted(format!("entity {id} is deleted")));
        }
        Ok(json!({ "ok": true, "reference": with_ws(&reference, &self.workspace_id), "type_id": e.type_id,
                   "content_rev": e.content_rev, "capabilities": access.caps(&ctx, id)?.names() }))
    }

    /// `doc.get_collab_state`: the only way CRDT bytes leave the backend
    /// (never mixed into `read`, never part of a share export).
    pub fn get_collab_state(&self, caller: &Caller, entity_id: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let e = read::readable_entity(&ctx, &access, entity_id)?;
        if !e.alive() {
            return Err(WsError::deleted(format!("entity {entity_id} is deleted")));
        }
        let rt = ctx.richtext(entity_id)?.ok_or_else(|| WsError::invalid_op("not a rich text"))?;
        let snapshot = richtext::export_snapshot(&rt.doc)?;
        Ok(json!({ "ok": true, "entity_id": entity_id, "lineage_id": rt.meta.lineage_id, "engine": rt.meta.engine,
                   "engine_version": rt.meta.engine_version, "encoding": rt.meta.encoding,
                   "snapshot": base64::engine::general_purpose::STANDARD.encode(snapshot),
                   "seq": self.head_seq, "content_rev": e.content_rev }))
    }

    /// `doc.query`: by saved view (`view_id`, optionally narrowed by a session
    /// filter) or directly by `source_id`.
    pub fn query(&self, caller: &Caller, params: &Value, sources: &SourceRegistry) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut spec = QuerySpec {
            limit: params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize,
            cursor: params.get("cursor").and_then(Value::as_str).map(str::to_string),
            best_effort: params.get("consistency").and_then(Value::as_str) == Some("best_effort"),
            with_meta: params.get("with_meta").and_then(Value::as_bool).unwrap_or(true),
            ..Default::default()
        };
        if spec.limit == 0 || spec.limit > QUERY_LIMIT_MAX {
            return Err(WsError::limit(format!("limit must be 1..{QUERY_LIMIT_MAX}")));
        }
        let mut source_id = params.get("source_id").and_then(Value::as_str).map(str::to_string);
        if let Some(view_id) = params.get("view_id").and_then(Value::as_str) {
            let cell = read::readable_entity(&ctx, &access, view_id)?;
            if !cell.alive() || cell.type_id != TYPE_CELL || cell.payload.get("view").and_then(|v| v["type"].as_str()) != Some("table") {
                return Err(WsError::invalid_op("view_id must name a live table view"));
            }
            source_id = cell.payload.get("source_ref").and_then(reference_entity_id).map(str::to_string);
            spec.filter = cell.payload.get("filter").filter(|f| !f.is_null()).cloned();
            spec.sorts = cell.payload.get("sorts").and_then(Value::as_array).cloned().unwrap_or_default();
            spec.group = cell.payload.get("group").and_then(|g| g["field_id"].as_str()).map(str::to_string);
            spec.manual_order = cell.payload.get("manual_order").and_then(Value::as_object).cloned();
            spec.fields = cell
                .payload
                .get("fields")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|f| f["field_id"].as_str()).map(str::to_string).collect());
        }
        let source_id = source_id.ok_or_else(|| WsError::invalid_op("source_id or view_id required"))?;
        let source = read::readable_entity(&ctx, &access, &source_id)?;
        if !source.alive() {
            return Err(WsError::deleted(format!("table {source_id} is deleted")));
        }
        if source.type_id != TYPE_TABLE {
            return Err(WsError::invalid_op(format!("{source_id} is not a table source")));
        }
        // session-level additions on top of the saved view
        if let Some(extra) = params.get("filter").filter(|f| !f.is_null()) {
            spec.filter = Some(match spec.filter.take() {
                Some(f) => json!({ "op": "and", "args": [f, extra] }),
                None => extra.clone(),
            });
        }
        if let Some(s) = params.get("sorts").and_then(Value::as_array) {
            spec.sorts = s.clone();
        }
        if let Some(f) = params.get("fields").and_then(Value::as_array) {
            spec.fields = Some(f.iter().filter_map(Value::as_str).map(str::to_string).collect());
        }
        if let Some(g) = params.get("group").and_then(Value::as_str) {
            spec.group = Some(g.to_string());
        }
        let mut out = if types::is_url_table(&source) {
            let reference = source.payload.get("source_ref").cloned().unwrap_or(Value::Null);
            let q = SourceQuery {
                filter: spec.filter.clone(),
                sorts: spec.sorts.clone(),
                fields: spec.fields.clone(),
                limit: spec.limit,
                cursor: spec.cursor.clone(),
                require_snapshot: params.get("consistency").and_then(Value::as_str) == Some("snapshot"),
                snapshot_token: params.get("source_revision").and_then(Value::as_str).map(str::to_string),
            };
            // the definition's content_rev tracks the definition only, never the remote rows
            let mut page = sources.query(&reference, &q, &caller.principal)?;
            page["definition_rev"] = json!(source.content_rev);
            page["data_mode"] = json!("url_query");
            page
        } else {
            run_query(&ctx, &source, &spec)?
        };
        out["ok"] = json!(true);
        out["source_id"] = json!(source_id);
        Ok(out)
    }

    /// Capabilities of a URL table's source, for honest UI states.
    pub fn source_capabilities(&self, caller: &Caller, source_id: &str, sources: &SourceRegistry) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let e = read::readable_entity(&self.ctx(&now), &access, source_id)?;
        let reference = e.payload.get("source_ref").ok_or_else(|| WsError::invalid_op("not a URL query table"))?;
        Ok(match sources.capabilities(reference) {
            Ok(c) => json!({ "ok": true, "available": true, "capabilities": c }),
            // an unreachable or unknown source does not invalidate the stored definition
            Err(e) => json!({ "ok": true, "available": false, "reason": e.to_json() }),
        })
    }

    /// May `caller` fetch this asset? Authorized through an entity that references it.
    pub fn can_read_asset(&self, caller: &Caller, object_id: &str) -> WsResult<bool> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut st = self.doc.prepare_cached("SELECT src_entity_id FROM refs WHERE dst_object_id = ?1 AND kind = 'asset'").map_err(db_err)?;
        let ids: Vec<String> = st.query_map([object_id], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        for id in ids {
            if let Some(e) = ctx.entity(&id)? {
                if e.alive() && access.can_read(&ctx, &e)? {
                    return Ok(true);
                }
            }
        }
        // the uploader may read back what it staged
        let staged: Option<String> = self
            .local
            .query_row("SELECT principal FROM staged_assets WHERE object_id = ?1", [object_id], |r| r.get(0))
            .ok();
        Ok(staged.as_deref() == Some(caller.principal.as_str()) && access.ws_caps.has(Cap::Read))
    }
}

fn with_ws(reference: &Value, workspace_id: &str) -> Value {
    let mut r = reference.clone();
    r["workspace_id"] = json!(workspace_id);
    r
}
