//! Read-side service methods: resolve, read, list, query, collaboration state.

use crate::docdb::db_err;
use crate::urlsource::{SourceQuery, SourceRegistry};
use crate::workspace::{Caller, Workspace};
use aiworkspace_core::model::ReadCtx;
use aiworkspace_core::access::Cap;
use aiworkspace_core::filter::QuerySpec;
use aiworkspace_core::model::*;
use aiworkspace_core::read;
use aiworkspace_core::value::{canonical_reference, normalize_reference, reference_entity_id, reference_from_canonical, reference_is_local};
use aiworkspace_core::{richtext, types, Code, WsError, WsResult};
use base64::Engine;
use rusqlite::{params, OptionalExtension};
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

    /// `doc.read`: the native content (`as` absent or `content`), the common `meta`, or the plain-text `body`. The
    /// target is `entity_id` or `path`; `version` (`{ epoch, seq }`, `{ object_id }`) reads a fixed version of the
    /// native content, or fails — never the head instead (R2).
    pub fn read_as(&self, caller: &Caller, params: &Value) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let id = self.target(&ctx, &access, params)?;
        let what = params.get("as").and_then(Value::as_str).unwrap_or("content");
        if let Some(version) = params.get("version").filter(|v| !v.is_null()) {
            if what != "content" {
                return Err(WsError::new(Code::UnsupportedVersion, format!("{what} of a fixed version is not available yet; read the head or the native content")));
            }
            if params.get("selector").is_some_and(|s| !s.is_null()) {
                return Err(WsError::new(Code::UnsupportedVersion, "a selector of a fixed version is not available yet"));
            }
            let reference = json!({ "workspace_id": self.workspace_id, "entity_id": id, "mode": "fixed", "version": version });
            let mut v = self.resolve(caller, Some(&reference), None)?;
            v["entity_id"] = json!(id);
            v["head_seq"] = json!(self.head_seq);
            return Ok(v);
        }
        match what {
            "content" => self.read(caller, &id, params.get("selector").filter(|s| !s.is_null())),
            "meta" | "body" => {
                let e = read::readable_entity(&ctx, &access, &id)?;
                if !e.alive() {
                    return Err(WsError::deleted(format!("entity {id} is deleted")));
                }
                let objects = self.objects.clone();
                let loader = move |obj: &str| -> WsResult<Option<Vec<u8>>> {
                    use aiworkspace_core::materialize::ObjectSource;
                    if !objects.has_object(obj) {
                        return Ok(None);
                    }
                    objects.get_file(obj).map(Some)
                };
                let mut v = if what == "meta" {
                    let mut m = aiworkspace_core::meta::meta(&ctx, &access, &e, Some(&loader))?;
                    m["ref"] = json!({ "workspace_id": self.workspace_id, "entity_id": id, "mode": "live" });
                    m["version"]["epoch"] = json!(self.epoch);
                    m["version"]["head_seq"] = json!(self.head_seq);
                    if e.write_policy == POLICY_LOCK {
                        m["lock_holder"] = self.lock_holder(&id, &now).unwrap_or(Value::Null);
                    }
                    if e.type_id == TYPE_ASSET {
                        if let Some(obj) = e.payload.get("object_id").and_then(Value::as_str) {
                            m["availability"] = json!(self.objects.availability(obj).as_str());
                        }
                    }
                    m
                } else {
                    aiworkspace_core::body::read_body(&ctx, &access, &e, params, Some(&loader))?
                };
                v["head_seq"] = json!(self.head_seq);
                Ok(v)
            }
            other => Err(WsError::invalid_op(format!("as must be content, meta or body, not {other}"))),
        }
    }

    /// The entity a request names: `entity_id`, or `path` (the one path syntax, every step readable).
    pub fn target(&self, ctx: &dyn ReadCtx, access: &aiworkspace_core::access::Access, params: &Value) -> WsResult<String> {
        match (params.get("entity_id").and_then(Value::as_str), params.get("path").and_then(Value::as_str)) {
            (Some(id), None) => Ok(id.to_string()),
            (None, Some(p)) => aiworkspace_core::path::resolve(ctx, access, p),
            (Some(_), Some(_)) => Err(WsError::invalid_op("give entity_id or path, not both")),
            (None, None) => Err(WsError::invalid_op("entity_id or path required")),
        }
    }

    /// `doc.read` of the native content.
    pub fn read(&self, caller: &Caller, entity_id: &str, selector: Option<&Value>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let mut v = read::read(&self.ctx(&now), &access, entity_id, selector)?;
        self.decorate(&mut v, &now);
        v["head_seq"] = json!(self.head_seq);
        Ok(v)
    }

    fn no_version(params: &Value, method: &str) -> WsResult<()> {
        if params.get("version").is_some_and(|v| !v.is_null()) {
            return Err(WsError::new(Code::UnsupportedVersion, format!("{method} reads the head only; a fixed version is not available yet")));
        }
        Ok(())
    }

    /// `doc.list { at?, type?, not_type?, schema?, limit?, cursor? }`: the readable direct children of one node.
    pub fn list(&self, caller: &Caller, params: &Value) -> WsResult<Value> {
        Self::no_version(params, "doc.list")?;
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        aiworkspace_core::path::list(&self.ctx(&now), &access, params, self.head_seq)
    }

    /// `doc.find { at?, depth?, name?, type?, not_type?, schema?, limit?, cursor? }`.
    pub fn find(&self, caller: &Caller, params: &Value) -> WsResult<Value> {
        Self::no_version(params, "doc.find")?;
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        aiworkspace_core::path::find(&self.ctx(&now), &access, params, self.head_seq)
    }

    /// `doc.list_annotations`: annotations anchored to `target_ids` and/or placed under `parent_id`.
    pub fn list_annotations(&self, caller: &Caller, target_ids: &[String], parent_id: Option<&str>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        if target_ids.len() > 1000 {
            return Err(WsError::limit("at most 1000 target_ids"));
        }
        let now = self.now();
        let annotations = read::list_annotations(&self.ctx(&now), &access, target_ids, parent_id)?;
        Ok(json!({ "ok": true, "annotations": annotations, "head_seq": self.head_seq }))
    }

    /// The whole readable tree as envelopes (the "outline"): one call for UIs.
    pub fn outline(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = read::outline(&ctx, &access)?;
        for env in &mut out {
            self.decorate(env, &now);
        }
        Ok(json!({ "ok": true, "epoch": self.epoch, "head_seq": self.head_seq, "entities": out }))
    }

    /// `doc.freshness`: dependency-based freshness of results and wishes (phase two §7.5).
    pub fn freshness(&self, caller: &Caller, entity_ids: &[String]) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        if entity_ids.len() > 1000 {
            return Err(WsError::limit("at most 1000 entity_ids"));
        }
        let now = self.now();
        let items = aiworkspace_core::freshness::freshness_many(&self.ctx(&now), &access, entity_ids)?;
        Ok(json!({ "ok": true, "head_seq": self.head_seq, "items": items }))
    }

    /// `doc.relations`: what an entity references / is referenced by / was generated from (phase two §6.2).
    pub fn relations(&self, caller: &Caller, entity_id: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let mut v = aiworkspace_core::freshness::relations(&self.ctx(&now), &access, entity_id)?;
        v["ok"] = json!(true);
        v["head_seq"] = json!(self.head_seq);
        Ok(v)
    }

    /// `doc.list_versions`: addressable versions of one entity (generated results and checkpoints),
    /// newest first, with the commit that produced each content version.
    pub fn list_versions(&self, caller: &Caller, entity_id: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let e = read::readable_entity(&self.ctx(&now), &access, entity_id)?;
        let mut st = self
            .doc
            .prepare(
                "SELECT v.content_rev, v.object_id, v.derived_json, v.kind, v.created_at, c.commit_id, c.principal, c.origin, c.run_id, c.message, c.accepted_at \
                 FROM entity_versions v LEFT JOIN commits c ON c.seq = v.content_rev WHERE v.entity_id = ?1 ORDER BY v.content_rev DESC",
            )
            .map_err(db_err)?;
        let rows: Vec<Value> = st
            .query_map([entity_id], |r| {
                let derived: Option<String> = r.get(2)?;
                Ok(json!({
                    "content_rev": r.get::<_, u64>(0)?, "object_id": r.get::<_, String>(1)?,
                    "derived": derived.and_then(|d| serde_json::from_str::<Value>(&d).ok()),
                    "kind": r.get::<_, String>(3)?, "created_at": r.get::<_, String>(4)?,
                    "commit_id": r.get::<_, Option<String>>(5)?, "author": r.get::<_, Option<String>>(6)?, "origin": r.get::<_, Option<String>>(7)?,
                    "run_id": r.get::<_, Option<String>>(8)?, "message": r.get::<_, Option<String>>(9)?, "accepted_at": r.get::<_, Option<String>>(10)?,
                }))
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        let rows: Vec<Value> = rows.into_iter().filter(|v| self.objects.has_object(v["object_id"].as_str().unwrap_or(""))).collect();
        Ok(json!({ "ok": true, "entity_id": entity_id, "content_rev": e.content_rev, "versions": rows, "head_seq": self.head_seq }))
    }

    /// The operations that turn the current content of `entity_id` into the stored version
    /// `content_rev` (a plan the caller submits as an ordinary, undoable Commit). The dependency record
    /// of that version comes back with it.
    pub fn restore_version_plan(&self, caller: &Caller, entity_id: &str, content_rev: u64) -> WsResult<Value> {
        use aiworkspace_core::materialize::ObjectSource;
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let e = read::readable_entity(&ctx, &access, entity_id)?;
        if !e.alive() {
            return Err(WsError::deleted(format!("entity {entity_id} is deleted")));
        }
        let row: Option<(String, Option<String>)> = self
            .doc
            .query_row("SELECT object_id, derived_json FROM entity_versions WHERE entity_id = ?1 AND content_rev = ?2", params![entity_id, content_rev], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
            .map_err(db_err)?;
        let (object_id, derived) = row.ok_or_else(|| WsError::not_found("that version is not available here"))?;
        if !self.objects.has_object(&object_id) {
            return Err(WsError::new(Code::ResourceReclaimed, "that version's content was reclaimed after its retention period").with_data(json!({ "object_ids": [object_id] })));
        }
        let object = aiworkspace_core::canonical::parse_strict(&self.objects.get_object(&object_id)?)?;
        if object["ws_type"] != json!(e.type_id) {
            return Err(WsError::invalid_schema("stored version has another type"));
        }
        let content = object["content"].clone();
        let mut ops = Vec::new();
        match e.type_id.as_str() {
            TYPE_RICHTEXT => {
                let rt = ctx.richtext(entity_id)?.ok_or_else(|| WsError::io("rich text state missing"))?;
                let doc = &content["doc"];
                let target = match doc.get("file").and_then(Value::as_str) {
                    Some(f) if doc.get("encoding").is_some() => {
                        aiworkspace_core::canonical::parse_strict(std::str::from_utf8(&self.objects.get_file(f)?).map_err(|_| WsError::invalid_schema("ast file is not UTF-8"))?)?
                    }
                    _ => doc.clone(),
                };
                let target = richtext::canonicalize(&target)?;
                ops.extend(richtext::diff_blocks(entity_id, &rt.meta.ast, &target, &rt.meta.block_index)?);
            }
            TYPE_RECORD => {
                // stored form is { schema, props }: compare key by key with what is in the payload now
                let mut want = serde_json::Map::new();
                want.insert("schema".into(), content["schema"].clone());
                for (k, v) in content["props"].as_object().into_iter().flatten() {
                    want.insert(format!("p:{k}"), v.clone());
                }
                ops.extend(keyed_diff(&e, &want));
            }
            TYPE_TABLE => return Err(WsError::new(Code::UnsupportedVersion, "restoring a table version is not supported in this phase")),
            _ => {
                let want = content.as_object().cloned().unwrap_or_default();
                ops.extend(keyed_diff(&e, &want));
            }
        }
        let derived: Option<Value> = derived.and_then(|d| serde_json::from_str::<Value>(&d).ok());
        ops.push(json!({ "op": "entity.set_derived", "entity_id": entity_id, "derived": derived.clone().unwrap_or(Value::Null) }));
        Ok(json!({ "ok": true, "entity_id": entity_id, "content_rev": content_rev, "operations": ops, "derived": derived, "head_seq": self.head_seq }))
    }

    /// `doc.resolve`: a reference or a path → resolved reference with the full
    /// `workspace_id`, type, version and the caller's capabilities on it.
    pub fn resolve(&self, caller: &Caller, reference: Option<&Value>, path: Option<&str>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let reference = match (reference, path) {
            // a retained point of this history (D03, T1-12): `{ epoch, seq }` reads through its checkpoint
            (Some(r), _) if r.get("mode").is_some() && r["version"].get("seq").is_some() => return self.resolve_retained(&access, &canonical_reference(r)?),
            // the frozen reference object (D10) or the persisted form it replaces
            (Some(r), _) if r.get("mode").is_some() => reference_from_canonical(&canonical_reference(r)?)?,
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
                return Ok(json!({ "ok": true, "reference": with_ws(&reference, &self.workspace_id), "canonical": canonical_reference(&with_ws(&reference, &self.workspace_id))?,
                                  "object_id": obj, "content_rev": known, "fixed": true, "object": content }));
            }
            _ => return Err(WsError::invalid_op("unknown version mode")),
        }
        let id = reference_entity_id(&reference).unwrap_or("");
        let e = read::readable_entity(&ctx, &access, id)?;
        if !e.alive() {
            return Err(WsError::deleted(format!("entity {id} is deleted")));
        }
        Ok(json!({ "ok": true, "reference": with_ws(&reference, &self.workspace_id), "canonical": canonical_reference(&with_ws(&reference, &self.workspace_id))?,
                   "type_id": e.type_id, "content_rev": e.content_rev, "capabilities": access.entity_caps(&ctx, &e)?.names(),
                   "path": aiworkspace_core::path::path_of(&ctx, &access, id)?, "name": e.name }))
    }

    /// A fixed version `{ epoch, seq }` of an entity (D03): readable only while a snapshot taken at that
    /// seq is still a retention root (checkpoint, export, clone — `ws.release_retention` ends it); then the
    /// entity's content object in that snapshot. Anything else is `VERSION_NOT_RETAINED`.
    fn resolve_retained(&self, access: &aiworkspace_core::access::Access, canon: &Value) -> WsResult<Value> {
        use aiworkspace_core::materialize::ObjectSource;
        let not_retained = |what: String| WsError::new(Code::VersionNotRetained, what);
        let epoch = canon["version"]["epoch"].as_str().unwrap_or("");
        let seq = canon["version"]["seq"].as_u64().unwrap_or(0);
        if epoch != self.epoch {
            return Err(not_retained(format!("epoch {epoch} is not this Workspace's history")));
        }
        let mut st = self.doc.prepare_cached("SELECT snapshot_id, content_root FROM snapshots WHERE seq = ?1 AND bucket_version IS NOT NULL ORDER BY created_at").map_err(db_err)?;
        let snaps: Vec<(String, String)> = st.query_map([seq], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let mut retained = None;
        for (id, root) in snaps {
            if self.objects.has_object(&root) {
                retained = Some((id, root));
                break;
            }
        }
        let Some((snapshot_id, content_root)) = retained else {
            return Err(not_retained(format!("no checkpoint or clone retains seq {seq} of epoch {epoch}")).with_data(json!({ "epoch": epoch, "seq": seq })));
        };
        let id = reference_entity_id(canon).unwrap_or("");
        let now = self.now();
        let ctx = self.ctx(&now);
        match ctx.entity(id)? {
            Some(e) if !access.can_read(&ctx, &e)? => return Err(WsError::not_found(format!("entity {id} not found"))),
            None if !access.ws_caps.has(Cap::Read) => return Err(WsError::not_found(format!("entity {id} not found"))),
            _ => {}
        }
        let root = aiworkspace_core::canonical::parse_strict(&self.objects.get_object(&content_root)?)?;
        let mut object_id: Option<String> = None;
        if let Some(file) = root["entities"]["file"].as_str() {
            let bytes = self.objects.get_file(file)?;
            for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                let entry: Value = serde_json::from_slice(line).map_err(|e| WsError::io(e.to_string()))?;
                if entry["entity_id"] == json!(id) {
                    object_id = entry["object_id"].as_str().map(str::to_string);
                    break;
                }
            }
        }
        let Some(object_id) = object_id else {
            return Err(WsError::not_found(format!("entity {id} is not part of seq {seq}")));
        };
        let object = aiworkspace_core::canonical::parse_strict(&self.objects.get_object(&object_id)?)?;
        let mut reference = canon.clone();
        reference["workspace_id"] = json!(self.workspace_id);
        Ok(json!({ "ok": true, "reference": reference, "canonical": reference, "fixed": true, "snapshot_id": snapshot_id,
                   "content_root": content_root, "object_id": object_id, "object": object }))
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

    /// `doc.query`: a text query (`text`) over names, titles and bodies, or an SQL query (`sql`) over the logical
    /// tables (核心 API 调整 R3, R11). Nothing else.
    pub fn query(&self, caller: &Caller, params: &Value) -> WsResult<Value> {
        for k in ["view_id", "source_id", "filter", "sorts", "fields", "group", "group_order", "group_limit", "consistency", "with_meta"] {
            if params.get(k).is_some() {
                return Err(WsError::invalid_op(format!("doc.query takes text or sql; {k} belongs to the old table query (write the SQL, or doc.source_query for a URL table)")));
            }
        }
        Self::no_version(params, "doc.query")?;
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        match (params.get("text"), params.get("sql")) {
            (Some(_), None) => {
                let started = std::time::Instant::now();
                let stop = move || started.elapsed() > std::time::Duration::from_secs(1);
                let objects = self.objects.clone();
                let loader = move |obj: &str| -> WsResult<Option<Vec<u8>>> {
                    use aiworkspace_core::materialize::ObjectSource;
                    if !objects.has_object(obj) {
                        return Ok(None);
                    }
                    objects.get_file(obj).map(Some)
                };
                let mut v = aiworkspace_core::search::text_query(&ctx, &access, params, self.head_seq, Some(&loader), &stop)?;
                v["ok"] = json!(true);
                Ok(v)
            }
            (None, Some(_)) => {
                if let Some(expect) = params.get("expect_head").filter(|v| !v.is_null()) {
                    if expect["epoch"] != json!(self.epoch) || expect["seq"].as_u64() != Some(self.head_seq) {
                        return Err(WsError::sub(Code::SnapshotExpired, "HEAD_MISMATCH", "the Workspace is not at the head the caller expects: synchronize, then query again")
                            .with_data(json!({ "epoch": self.epoch, "head_seq": self.head_seq })));
                    }
                }
                let conn = rusqlite::Connection::open_with_flags(self.dir.join("doc.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
                    .map_err(db_err)?;
                conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_err)?;
                conn.execute_batch("BEGIN DEFERRED").map_err(db_err)?;
                let seen: u64 = crate::docdb::meta_get(&conn, "head_seq")?.and_then(|s| s.parse().ok()).unwrap_or(0);
                if seen != self.head_seq {
                    return Err(WsError::new(Code::WriterBusy, "the Workspace is being written; retry"));
                }
                let mut v = crate::sql::execute(&conn, &ctx, &access, params, self.head_seq, None)?;
                v["ok"] = json!(true);
                Ok(v)
            }
            _ => Err(WsError::invalid_op("doc.query takes either text (a text query) or sql (an SQL query)")),
        }
    }

    /// `f` over one consistent read view of the document (internal callers and tests; no access check).
    pub fn read_ctx<T>(&self, f: impl FnOnce(&dyn ReadCtx) -> T) -> T {
        let now = self.now();
        f(&self.ctx(&now))
    }

    /// The live records of a local table in `record_id` order, as `doc.read` gives a record (internal callers, e.g.
    /// the session notes; the evaluator of the core, not SQL).
    pub fn table_rows(&self, caller: &Caller, source_id: &str, limit: usize) -> WsResult<Vec<Value>> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let source = read::readable_entity(&ctx, &access, source_id)?;
        if !source.alive() || source.type_id != TYPE_TABLE || types::is_url_table(&source) {
            return Err(WsError::invalid_op(format!("{source_id} is not a local table")));
        }
        let page = aiworkspace_core::filter::run_query(&ctx, &source, &QuerySpec { limit: limit.max(1), best_effort: true, ..Default::default() })?;
        Ok(page["rows"].as_array().cloned().unwrap_or_default())
    }

    /// `doc.source_query`: the rows of a URL table (`data_mode: url_query`), queried at its remote source with the
    /// table query parameters (`filter`, `sorts`, `fields`, `limit`, `cursor`, `consistency`, `source_revision`).
    pub fn source_query(&self, caller: &Caller, params: &Value, sources: &SourceRegistry) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let source_id = params.get("source_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("source_id required"))?;
        let source = read::readable_entity(&ctx, &access, source_id)?;
        if !source.alive() {
            return Err(WsError::deleted(format!("table {source_id} is deleted")));
        }
        if source.type_id != TYPE_TABLE || !types::is_url_table(&source) {
            return Err(WsError::invalid_op(format!("{source_id} is not a URL table: query local tables with doc.query {{ sql }}")));
        }
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
        if limit == 0 || limit > QUERY_LIMIT_MAX {
            return Err(WsError::limit(format!("limit must be 1..{QUERY_LIMIT_MAX}")));
        }
        let spec = QuerySpec {
            filter: params.get("filter").filter(|f| !f.is_null()).cloned(),
            sorts: params.get("sorts").and_then(Value::as_array).cloned().unwrap_or_default(),
            fields: params.get("fields").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()),
            ..Default::default()
        };
        let reference = source.payload.get("source_ref").cloned().unwrap_or(Value::Null);
        let q = SourceQuery {
            filter: spec.filter,
            sorts: spec.sorts,
            fields: spec.fields,
            limit,
            cursor: params.get("cursor").and_then(Value::as_str).map(str::to_string),
            require_snapshot: params.get("consistency").and_then(Value::as_str) == Some("snapshot"),
            snapshot_token: params.get("source_revision").and_then(Value::as_str).map(str::to_string),
        };
        // the definition's content_rev tracks the definition only, never the remote rows
        let mut page = sources.query(&reference, &q, &caller.principal)?;
        page["definition_rev"] = json!(source.content_rev);
        page["data_mode"] = json!("url_query");
        page["ok"] = json!(true);
        page["source_id"] = json!(source_id);
        Ok(page)
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

/// `entity.set_keys` / `entity.unset_keys` turning the keyed payload of `e` into `want`, each key
/// guarded by the version the caller is looking at.
fn keyed_diff(e: &EntityRow, want: &serde_json::Map<String, Value>) -> Vec<Value> {
    let mut set = Vec::new();
    let mut unset = Vec::new();
    for (k, v) in want {
        if e.payload.get(k) != Some(v) {
            set.push(json!({ "key": k, "value": v, "expect": { "rev": e.key_rev(k) } }));
        }
    }
    for k in e.payload.keys() {
        if !want.contains_key(k) {
            unset.push(json!({ "key": k, "expect": { "rev": e.key_rev(k) } }));
        }
    }
    let mut ops = Vec::new();
    if !set.is_empty() {
        ops.push(json!({ "op": "entity.set_keys", "entity_id": e.entity_id, "keys": set }));
    }
    if !unset.is_empty() {
        ops.push(json!({ "op": "entity.unset_keys", "entity_id": e.entity_id, "keys": unset }));
    }
    ops
}

fn with_ws(reference: &Value, workspace_id: &str) -> Value {
    let mut r = reference.clone();
    r["workspace_id"] = json!(workspace_id);
    r
}
