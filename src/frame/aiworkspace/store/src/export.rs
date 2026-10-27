//! Checkpoints, export packages, package loading, replica bootstrap and
//! asset staging (design doc §2.10, §3.6, §4.4, §6.2).

use crate::docdb::{db_err, meta_set};
use crate::objects::{sniff_media_type, sniff_prefix, ObjectStore, StoreSink, ASSETS, EXPORTS, OBJECTS};
use crate::schema::{DOC_DDL, REPLICA_TABLES};
use crate::workspace::{random_id, Caller, CommitOpts, Workspace};
use aiworkspace_core::access::{Access, Cap};
use aiworkspace_core::canonical::{self, named_object, obj_id_from_filename, obj_id_to_filename, parse_strict, ObjId, OBJ_TYPE_JSON};
use aiworkspace_core::materialize::{self, load_ops, materialize_with, snapshot_root, ObjectSink, ObjectSource};
use aiworkspace_core::model::*;
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{richtext, Code, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const PACKAGE_FORMAT: &str = "buckyos.ai-workspace";
pub const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_PACKAGE_ENTRIES: usize = 200_000;
pub const STAGED_ASSET_TTL_MS: i64 = 24 * 3600 * 1000;
/// How long a download token of an export or a replica is good for.
pub const DOWNLOAD_TOKEN_MS: i64 = 3_600_000;

fn io(e: std::io::Error) -> WsError {
    let code = if e.raw_os_error() == Some(28) { Code::StorageFull } else { Code::StorageIoError };
    WsError::new(code, e.to_string())
}

/// Key of an export package in the owner's `exports` bucket.
pub fn export_key(workspace_id: &str, export_id: &str) -> String {
    format!("{workspace_id}/export/{export_id}.bcanvas")
}

/// A file of this process while it is built or read (packages, replicas): removed when dropped.
pub struct TempFile(pub PathBuf);

impl TempFile {
    pub fn new(name: &str) -> WsResult<TempFile> {
        let dir = std::env::temp_dir().join("buckyos").join("aiworkspace");
        std::fs::create_dir_all(&dir).map_err(io)?;
        Ok(TempFile(dir.join(format!("{}-{name}", random_id("")))))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct Checkpoint {
    pub snapshot_id: ObjId,
    pub content_root: ObjId,
    pub seq: u64,
    pub excluded: usize,
    pub unresolved: Vec<Value>,
    pub objects: BTreeMap<String, ObjId>,
}

impl Checkpoint {
    pub fn to_json(&self) -> Value {
        json!({ "ok": true, "snapshot": self.snapshot_id, "content_root": self.content_root, "seq": self.seq,
                "excluded_entities": self.excluded, "unresolved_refs": self.unresolved })
    }
}

/// Everything reachable from a content root: NamedObjects and chunks.
/// `assets` decides whether asset bytes belong to the closure.
pub fn closure(store: &ObjectStore, content_root: &str, assets: bool) -> WsResult<(BTreeSet<ObjId>, BTreeSet<ObjId>, Vec<ObjId>)> {
    let (mut objects, mut chunks, mut missing) = (BTreeSet::new(), BTreeSet::new(), Vec::new());
    let add_file = |file: &str, objects: &mut BTreeSet<ObjId>, chunks: &mut BTreeSet<ObjId>| -> WsResult<bool> {
        if !store.has_object(file) {
            return Ok(false);
        }
        let chunk = store.file_chunk(file)?;
        if store.get_chunk(&chunk).is_err() {
            return Ok(false);
        }
        objects.insert(file.to_string());
        chunks.insert(chunk);
        Ok(true)
    };
    let root = parse_strict(&store.get_object(content_root)?)?;
    objects.insert(content_root.to_string());
    let mut entries = Vec::new();
    if let Some(f) = root["entities"]["file"].as_str() {
        add_file(f, &mut objects, &mut chunks)?;
        let text = String::from_utf8(store.get_file(f)?).map_err(|_| WsError::invalid_schema("entities file is not UTF-8"))?;
        for line in text.lines().filter(|l| !l.is_empty()) {
            entries.push(parse_strict(line)?);
        }
    }
    for e in entries {
        let Some(obj) = e["object_id"].as_str() else { continue };
        objects.insert(obj.to_string());
        let content = parse_strict(&store.get_object(obj)?)?;
        for f in [content["content"]["records"]["file"].as_str(), content["content"]["doc"]["file"].as_str()].into_iter().flatten() {
            add_file(f, &mut objects, &mut chunks)?;
        }
        if let Some(payload) = content["content"].as_object() {
            for a in aiworkspace_core::types::asset_object_ids(e["type_id"].as_str().unwrap_or(""), payload) {
                if !assets || !add_file(a, &mut objects, &mut chunks)? {
                    missing.push(a.to_string());
                }
            }
        }
    }
    Ok((objects, chunks, missing))
}

impl Workspace {
    /// Materialize the content `access` may read as immutable NamedObjects.
    pub(crate) fn materialize_for(&self, access: &Access, kind: &str, by: &str) -> WsResult<Checkpoint> {
        self.materialize_notes(access, kind, by, true)
    }

    /// [`Self::materialize_for`]; without `notes` the speaker notes are left out (third phase §6.4).
    pub(crate) fn materialize_notes(&self, access: &Access, kind: &str, by: &str, notes: bool) -> WsResult<Checkpoint> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut sink = StoreSink::new(&self.objects);
        let visible = |e: &EntityRow| access.can_read(&ctx, e);
        // (entity_id, content_rev) → object id is a pure, rebuildable cache
        let mut cached = |e: &EntityRow, fresh: Option<&ObjId>| -> Option<ObjId> {
            match fresh {
                Some(id) => {
                    let _ = self.doc.execute(
                        "INSERT INTO entity_versions (entity_id, content_rev, object_id, derived_json, kind, created_at) VALUES (?1, ?2, ?3, ?4, 'checkpoint', ?5) \
                         ON CONFLICT(entity_id, content_rev) DO UPDATE SET object_id = excluded.object_id",
                        params![e.entity_id, e.content_rev, id, e.derived.as_ref().map(Value::to_string), now],
                    );
                    None
                }
                None => self
                    .doc
                    .query_row(
                        "SELECT object_id FROM entity_versions WHERE entity_id = ?1 AND content_rev = ?2",
                        params![e.entity_id, e.content_rev],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
                    .filter(|id| self.objects.has_object(id)),
            }
        };
        let m = materialize_with(&ctx, &mut sink, &visible, &mut cached, !notes)?;
        let forked: Option<Value> = crate::docdb::meta_get(&self.doc, "forked_from")?.and_then(|s| serde_json::from_str(&s).ok());
        let snap = snapshot_root(&self.workspace_id, self.head_seq, &self.head_commit_id, &m.content_root, forked.as_ref());
        let (snapshot_id, text) = named_object(OBJ_TYPE_JSON, &snap)?;
        self.objects.put_verified(&snapshot_id, &text)?;
        // every snapshot is retained (D06) by its retention set until `ws.release_retention` / `ws.delete_checkpoint`
        let mut members = BTreeMap::new();
        self.objects.add_member(&snapshot_id, &mut members)?;
        if let Some(source) = forked.as_ref().and_then(|f| f["snapshot"].as_str()) {
            self.objects.add_member(source, &mut members)?;
        }
        self.objects.content_members(&m.content_root, &mut members)?;
        let key = format!("{}/snapshot/{snapshot_id}", self.workspace_id);
        let (_, version) = self.objects.retain(&self.bucket(OBJECTS)?, &key, &members)?;
        self.doc
            .execute(
                "INSERT OR REPLACE INTO snapshots (snapshot_id, content_root, seq, kind, created_at, created_by, bucket_key, bucket_version) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![snapshot_id, m.content_root, self.head_seq, kind, now, by, key, version],
            )
            .map_err(db_err)?;
        Ok(Checkpoint { snapshot_id, content_root: m.content_root, seq: self.head_seq, excluded: m.excluded, unresolved: m.unresolved, objects: m.objects })
    }

    /// Content root of the current state as seen by its owner, without recording a snapshot row (tests).
    pub fn checkpoint_ro(&self) -> String {
        self.materialize_for(&Access::full("system"), "pin", "system").map(|c| c.content_root).unwrap_or_default()
    }

    /// `doc.checkpoint`
    pub fn checkpoint(&mut self, caller: &Caller) -> WsResult<Checkpoint> {
        let access = self.require_ws(caller, Cap::Export)?;
        self.materialize_for(&access, "checkpoint", &caller.principal)
    }

    /// `doc.export`: build a package into the owner's `exports` bucket and return its manifest and download.
    ///
    /// `share`: a new content snapshot of what the caller may read — no drafts,
    /// no history, no CRDT bytes, no credentials. `personal_backup`: additionally
    /// the original CRDT states and the caller's personal annotations. `notes`: whether speaker notes travel
    /// (default: in a personal backup only; third phase §6.4).
    pub fn export(&mut self, caller: &Caller, mode: &str, self_contained: bool, notes: Option<bool>) -> WsResult<Value> {
        let access = self.require_ws(caller, Cap::Export)?;
        let personal = match mode {
            "share" => false,
            "personal_backup" => true,
            _ => return Err(WsError::invalid_op("mode must be share or personal_backup")),
        };
        if personal && !access.ws_caps.has(Cap::Read) {
            return Err(WsError::denied("a personal backup needs workspace-level read"));
        }
        let notes = notes.unwrap_or(personal);
        self.check_quota(0, false)?;
        let cp = self.materialize_notes(&access, "export", &caller.principal, notes)?;
        let (mut objects, chunks, missing_assets) = closure(&self.objects, &cp.content_root, self_contained)?;
        objects.insert(cp.snapshot_id.clone());
        let now = self.now();
        let ctx = self.ctx(&now);
        let export_id = random_id("ex_");
        let tmp = TempFile::new(&format!("{export_id}.bcanvas"))?;
        let file = std::fs::File::create(&tmp.0).map_err(io)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let zerr = |e: zip::result::ZipError| WsError::io(format!("package: {e}"));
        let (mut object_list, mut chunk_list, mut types) = (Vec::new(), Vec::new(), BTreeMap::new());
        for id in &objects {
            let text = self.objects.get_object(id)?;
            zip.start_file(format!("objects/{}", obj_id_to_filename(id).unwrap()), opts).map_err(zerr)?;
            zip.write_all(text.as_bytes()).map_err(io)?;
            object_list.push(json!({ "id": id, "size": text.len() }));
        }
        for id in &chunks {
            let data = self.objects.get_chunk(id)?;
            zip.start_file(format!("chunks/{}", obj_id_to_filename(id).unwrap()), opts).map_err(zerr)?;
            zip.write_all(&data).map_err(io)?;
            chunk_list.push(json!({ "id": id, "size": data.len() }));
        }
        let mut collab = serde_json::Map::new();
        let mut has_richtext = false;
        for entity_id in cp.objects.keys() {
            let Some(e) = ctx.entity(entity_id)? else { continue };
            types.insert(e.type_id.clone(), e.schema_version);
            if e.type_id == TYPE_RICHTEXT {
                has_richtext = true;
                if personal {
                    if let Some(rt) = ctx.richtext(entity_id)? {
                        // full-history snapshot: lets the restored workspace keep merging the old lineage
                        zip.start_file(format!("collab/{entity_id}.loro"), opts).map_err(zerr)?;
                        zip.write_all(&richtext::export_snapshot(&rt.doc)?).map_err(io)?;
                        collab.insert(entity_id.clone(), json!({ "lineage_id": rt.meta.lineage_id, "engine": rt.meta.engine, "encoding": rt.meta.encoding }));
                    }
                }
            }
        }
        let mut personal_count = 0;
        if personal {
            let mut lines = String::new();
            let scope = format!("user:{}", caller.principal);
            let mut st = self.doc.prepare("SELECT entity_id FROM entities WHERE scope = ?1 AND deleted_seq IS NULL ORDER BY entity_id").map_err(db_err)?;
            let ids: Vec<String> = st.query_map([&scope], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
            for id in ids {
                if let (Some(e), Some(edge)) = (ctx.entity(&id)?, ctx.edge(&id)?) {
                    let mut payload = e.payload.clone();
                    payload.remove("author");
                    lines.push_str(&canonical::canonical_json(&json!({ "entity_id": id, "type_id": e.type_id, "parent_id": edge.parent_id,
                        "order_key": edge.order_key, "payload": payload }))?);
                    lines.push('\n');
                    personal_count += 1;
                }
            }
            zip.start_file("personal/entities.jsonl", opts).map_err(zerr)?;
            zip.write_all(lines.as_bytes()).map_err(io)?;
        }
        let external: Vec<Value> = cp
            .unresolved
            .iter()
            .filter(|u| u.get("source_ref").is_some())
            .map(|u| json!({ "from": u["from"], "source_ref": u["source_ref"],
                             "fixed_version": u["source_ref"]["version"]["mode"] == json!("fixed_revision"), "slice_included": false }))
            .collect();
        let mut missing: Vec<Value> = missing_assets
            .iter()
            .map(|a| json!({ "id": a, "reason": if self_contained { "unavailable" } else { "external" } }))
            .collect();
        missing.extend(cp.unresolved.iter().filter(|u| u.get("to").is_some()).map(|u| json!({ "ref": u["to"], "reason": "external" })));
        let manifest = json!({
            "format": PACKAGE_FORMAT, "format_version": FORMAT_VERSION, "export_mode": mode,
            // only true when everything in the declared scope really is inside the package
            "self_contained": self_contained && missing.is_empty() && external.is_empty(),
            "workspace_id": self.workspace_id, "title": self.title(), "snapshot": cp.snapshot_id, "content_root": cp.content_root,
            "seq": cp.seq, "objects": object_list, "chunks": chunk_list,
            "types": types.iter().map(|(t, v)| json!({ "type_id": t, "schema_version": v })).collect::<Vec<_>>(),
            "editor_schemas": if has_richtext { json!([richtext::EDITOR_SCHEMA]) } else { json!([]) },
            "missing": missing, "external_sources": external, "excluded_entities": cp.excluded,
            "collab": collab, "personal_entities": personal_count, "notes_included": notes,
            "exported_at": now, "exported_by": caller.principal,
        });
        zip.start_file("manifest.json", opts).map_err(zerr)?;
        zip.write_all(serde_json::to_string_pretty(&manifest).unwrap().as_bytes()).map_err(io)?;
        zip.finish().map_err(zerr)?.sync_all().map_err(io)?;
        let download = self.store_download(&tmp, &export_key(&self.workspace_id, &export_id))?;
        Ok(json!({ "ok": true, "export_id": export_id, "manifest": manifest, "download": download }))
    }

    /// Put a file this Workspace built into the owner's `exports` bucket (7 days) and say how to fetch it: the
    /// bucket, the key and a read token for that key alone (the caller need not be the owner).
    fn store_download(&self, file: &TempFile, key: &str) -> WsResult<Value> {
        let name = key.rsplit('/').next().unwrap_or(key);
        let object_id = self.objects.store_file(&file.0, name)?;
        let bucket = self.bucket(EXPORTS)?;
        let version = self.objects.bind(&bucket, key, &object_id)?;
        let token = self.objects.read_token(&bucket, key, DOWNLOAD_TOKEN_MS)?;
        Ok(json!({ "bucket_id": bucket, "key": key, "version": version, "obj_id": object_id, "token": token }))
    }

    /// Register uploaded bytes as an available asset (step 1 of §3.6): the
    /// content exists and is verified *before* any Commit may reference it.
    pub fn stage_asset(&mut self, caller: &Caller, data: &[u8]) -> WsResult<Value> {
        require_update(&self.require_ws_any(caller)?)?;
        self.check_quota(data.len() as u64, false)?;
        let object_id = StoreSink::new(&self.objects).put_file(data)?;
        self.stage_bound(caller, &object_id, sniff_media_type(data), data.len() as u64)
    }

    /// `asset.import_object`: a FileObject already in the named store becomes a staged asset without copying a
    /// byte. `binding` names where the uploader bound it (`bucket.get` proves it); without one the server binds the
    /// object itself (the caller's right to read it was checked at the NDM gateway).
    pub fn stage_object(&mut self, caller: &Caller, object_id: &str, max_bytes: u64) -> WsResult<Value> {
        require_update(&self.require_ws_any(caller)?)?;
        if !object_id.starts_with("cyfile:") {
            return Err(WsError::invalid_op("an asset must be a FileObject; import folders as workspace containers"));
        }
        if self.objects.availability(object_id) != crate::objects::Availability::Available {
            return Err(WsError::new(Code::DependencyUnavailable, "the imported object is incomplete"));
        }
        let size = self.objects.file_size(object_id)?;
        if size > max_bytes {
            return Err(WsError::limit(format!("workspace assets are limited to {max_bytes} bytes")));
        }
        self.check_quota(size, false)?;
        let media_type = sniff_prefix(&self.objects.file_head(object_id, 512)?);
        self.stage_bound(caller, object_id, media_type, size)
    }

    /// Bind a stored FileObject under `<ws>/<object_id>` in the owner's `assets` bucket and list it as staged.
    fn stage_bound(&mut self, caller: &Caller, object_id: &str, media_type: &str, size: u64) -> WsResult<Value> {
        let bucket = self.bucket(ASSETS)?;
        let key = format!("{}/{object_id}", self.workspace_id);
        let version = self.objects.bind(&bucket, &key, object_id)?;
        let expires = format_utc_ms((self.clock)() + STAGED_ASSET_TTL_MS);
        let fresh = self
            .local
            .query_row("SELECT 1 FROM staged_assets WHERE object_id = ?1", [object_id], |_| Ok(()))
            .optional()
            .map_err(db_err)?
            .is_none();
        self.local
            .execute(
                "INSERT OR REPLACE INTO staged_assets (object_id, media_type, size, principal, expires_at, bucket_key, bucket_version) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![object_id, media_type, size as i64, caller.principal, expires, key, version],
            )
            .map_err(db_err)?;
        if fresh {
            self.add_usage("staged", size as i64)?;
        }
        Ok(json!({ "ok": true, "object_id": object_id, "media_type": media_type, "size": size, "expires_at": expires,
                   "bucket_id": bucket, "key": key, "version": version }))
    }

    pub fn asset_info(&self, object_id: &str) -> WsResult<Option<AssetInfo>> {
        let now = self.now();
        self.ctx(&now).asset(object_id)
    }

    /// `diag.list_unretained`: staged content nobody references any more.
    /// Retention roots: every asset ever referenced by a commit (history can be
    /// undone), retained snapshots, and unexpired uploads. Nothing is collected
    /// automatically in this phase.
    pub fn list_unretained(&self, caller: &Caller) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let now = self.now();
        let mut st = self
            .local
            .prepare("SELECT object_id, size, expires_at FROM staged_assets WHERE expires_at <= ?1 ORDER BY object_id")
            .map_err(db_err)?;
        let staged: Vec<(String, i64, String)> =
            st.query_map([&now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let mut out = Vec::new();
        for (id, size, expired) in staged {
            let referenced: bool =
                self.doc.query_row("SELECT 1 FROM assets WHERE object_id = ?1", [&id], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
            if !referenced {
                out.push(json!({ "object_id": id, "size": size, "expired_at": expired }));
            }
        }
        Ok(json!({ "ok": true, "unretained": out }))
    }

    /// Compare the incrementally maintained reference index with a full rebuild.
    pub fn verify_refs(&self) -> WsResult<Value> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let alive: Vec<String> = {
            let mut st = self.doc.prepare("SELECT entity_id FROM entities").map_err(db_err)?;
            let v: Vec<String> = st.query_map([], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
            v
        };
        let rebuilt = materialize::rebuild_refs(&ctx, &alive)?;
        let stored: BTreeSet<RefEdge> = ctx.all_refs()?.into_iter().collect();
        let missing: Vec<&RefEdge> = rebuilt.difference(&stored).collect();
        let extra: Vec<&RefEdge> = stored.difference(&rebuilt).collect();
        Ok(json!({ "ok": missing.is_empty() && extra.is_empty(), "count": stored.len(), "missing": missing, "extra": extra }))
    }

    /// `replica.bootstrap`: a brand-new database built table by table from an
    /// allow-list inside one read transaction. Never "copy the file and delete":
    /// that leaves removed rows in free pages and the WAL.
    pub fn replica_bootstrap(&mut self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        if !access.ws_caps.has(Cap::Read) {
            return Err(WsError::denied("an offline replica needs workspace-level read"));
        }
        if self.purpose().is_some() {
            return Err(WsError::invalid_op("a show clone is not prepared for offline use"));
        }
        let now = self.now();
        let replica_id = random_id("rp_");
        let tmp = TempFile::new(&format!("{replica_id}.sqlite"))?;
        let path = tmp.0.clone();
        {
            let fresh = Connection::open(&path).map_err(db_err)?;
            fresh.execute_batch(DOC_DDL).map_err(db_err)?;
        }
        // rich text: fold pending updates into the snapshot so the replica needs no update rows
        let rich_ids: Vec<String> = {
            let mut st = self.doc.prepare("SELECT entity_id FROM richtext_states").map_err(db_err)?;
            let v: Vec<String> = st.query_map([], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
            v
        };
        let mut merged: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        {
            let ctx = self.ctx(&now);
            for id in &rich_ids {
                if let Some(rt) = ctx.richtext(id)? {
                    merged.insert(id.clone(), richtext::export_snapshot(&rt.doc)?);
                }
            }
        }
        let scope = format!("user:{}", caller.principal);
        self.doc.execute("ATTACH DATABASE ?1 AS replica", [path.to_string_lossy().as_ref()]).map_err(db_err)?;
        let result = (|| -> WsResult<()> {
            let tx = self.doc.unchecked_transaction().map_err(db_err)?;
            debug_assert!(REPLICA_TABLES.contains(&"entities"));
            tx.execute("INSERT INTO replica.workspace_meta SELECT key, value FROM main.workspace_meta WHERE key <> 'peer_id'", []).map_err(db_err)?;
            tx.execute("INSERT INTO replica.entities SELECT * FROM main.entities WHERE scope = 'shared' OR scope = ?1", [&scope]).map_err(db_err)?;
            tx.execute(
                "INSERT INTO replica.tree_edges SELECT * FROM main.tree_edges WHERE child_id IN (SELECT entity_id FROM replica.entities) \
                 AND parent_id IN (SELECT entity_id FROM replica.entities)",
                [],
            )
            .map_err(db_err)?;
            tx.execute(
                "INSERT INTO replica.refs SELECT * FROM main.refs WHERE src_entity_id IN (SELECT entity_id FROM replica.entities) \
                 AND (dst_entity_id = '' OR dst_workspace_id <> '' OR dst_entity_id IN (SELECT entity_id FROM replica.entities))",
                [],
            )
            .map_err(db_err)?;
            tx.execute("INSERT INTO replica.table_fields SELECT * FROM main.table_fields", []).map_err(db_err)?;
            // Residual values of deleted fields stay behind. They are filtered while
            // copying: an UPDATE afterwards would leave the old row bytes in free space.
            let mut dead: BTreeMap<String, Vec<String>> = BTreeMap::new();
            {
                let mut st = tx.prepare("SELECT source_id, field_id FROM main.table_fields WHERE deleted_seq IS NOT NULL").map_err(db_err)?;
                let rows: Vec<(String, String)> =
                    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
                for (source, field) in rows {
                    dead.entry(source).or_default().push(field);
                }
            }
            tx.execute(
                "INSERT INTO replica.table_records SELECT * FROM main.table_records \
                 WHERE source_id NOT IN (SELECT source_id FROM main.table_fields WHERE deleted_seq IS NOT NULL)",
                [],
            )
            .map_err(db_err)?;
            for (source, fields) in dead {
                // field ids are restricted to [a-z0-9_-], so they are safe inside a JSON path literal
                let paths: Vec<String> = fields.iter().map(|f| format!("'$.\"{f}\"'")).collect();
                let strip = |col: &str| format!("json_remove({col}, {})", paths.join(", "));
                tx.execute(
                    &format!(
                        "INSERT INTO replica.table_records SELECT source_id, record_id, {}, {}, {}, body_entity_id, created_seq, rev, deleted_seq \
                         FROM main.table_records WHERE source_id = ?1",
                        strip("values_json"),
                        strip("revs_json"),
                        strip("meta_json")
                    ),
                    [&source],
                )
                .map_err(db_err)?;
            }
            tx.execute(
                "INSERT INTO replica.richtext_states SELECT * FROM main.richtext_states WHERE entity_id IN (SELECT entity_id FROM replica.entities)",
                [],
            )
            .map_err(db_err)?;
            for (id, snapshot) in &merged {
                tx.execute("UPDATE replica.richtext_states SET snapshot = ?2, snapshot_seq = ?3 WHERE entity_id = ?1", params![id, snapshot, self.head_seq])
                    .map_err(db_err)?;
            }
            tx.execute("INSERT INTO replica.assets SELECT * FROM main.assets", []).map_err(db_err)?;
            tx.commit().map_err(db_err)
        })();
        self.doc.execute("DETACH DATABASE replica", []).map_err(db_err)?;
        result?;
        {
            // one self-contained file: no WAL sidecar travels separately
            let fresh = Connection::open(&path).map_err(db_err)?;
            fresh.pragma_update(None, "journal_mode", "DELETE").map_err(db_err)?;
        }
        let download = self.store_download(&tmp, &format!("{}/replica/{replica_id}.sqlite", self.workspace_id))?;
        Ok(json!({ "ok": true, "replica_id": replica_id, "download": download, "epoch": self.epoch,
                   "head_seq": self.head_seq, "principal": caller.principal, "prepared_at": now }))
    }
}

// ---------------------------------------------------------------------------
// package reading
// ---------------------------------------------------------------------------

pub struct OpenedPackage {
    pub manifest: Value,
    pub collab: BTreeMap<String, Vec<u8>>,
    pub personal: Vec<Value>,
}

/// Unpack and verify a package into the object store. Every object must be
/// canonical JSON hashing to its file name; every chunk must hash to its id.
/// Nothing becomes visible as a Workspace here.
pub fn open_package(path: &Path, store: &ObjectStore) -> WsResult<OpenedPackage> {
    let file = std::fs::File::open(path).map_err(io)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| WsError::invalid_schema(format!("not a package: {e}")))?;
    if zip.len() > MAX_PACKAGE_ENTRIES {
        return Err(WsError::limit("package has too many entries"));
    }
    let (mut manifest, mut collab, mut personal, mut total) = (None, BTreeMap::new(), Vec::new(), 0u64);
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| WsError::invalid_schema(format!("package entry: {e}")))?;
        if entry.is_dir() {
            continue;
        }
        // no `..`, no absolute paths, no symlinks
        let name = entry.enclosed_name().ok_or_else(|| WsError::invalid_schema("package entry escapes the package"))?;
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(WsError::invalid_schema("package contains a symbolic link"));
        }
        let name = name.to_string_lossy().replace('\\', "/");
        total += entry.size();
        if total > MAX_PACKAGE_BYTES {
            return Err(WsError::limit("package is too large"));
        }
        let mut data = Vec::with_capacity(entry.size().min(64 << 20) as usize);
        entry.by_ref().take(MAX_PACKAGE_BYTES).read_to_end(&mut data).map_err(io)?;
        match name.split_once('/') {
            None if name == "manifest.json" => {
                manifest = Some(parse_strict(std::str::from_utf8(&data).map_err(|_| WsError::invalid_schema("manifest is not UTF-8"))?)?);
            }
            Some(("objects", f)) => {
                let id = obj_id_from_filename(f).ok_or_else(|| WsError::invalid_schema(format!("bad object file name {f}")))?;
                let text = std::str::from_utf8(&data).map_err(|_| WsError::invalid_schema("object is not UTF-8"))?;
                store.put_verified(&id, text)?;
            }
            Some(("chunks", f)) => {
                let id = obj_id_from_filename(f).ok_or_else(|| WsError::invalid_schema(format!("bad chunk file name {f}")))?;
                store.put_chunk_verified(&id, &data)?;
            }
            Some(("collab", f)) => {
                if let Some(id) = f.strip_suffix(".loro").filter(|id| aiworkspace_core::id::is_valid_id(id)) {
                    collab.insert(id.to_string(), data);
                }
            }
            Some(("personal", "entities.jsonl")) => {
                for line in String::from_utf8_lossy(&data).lines().filter(|l| !l.is_empty()) {
                    personal.push(parse_strict(line)?);
                }
            }
            _ => return Err(WsError::invalid_schema(format!("unexpected package entry {name}"))),
        }
    }
    let manifest = manifest.ok_or_else(|| WsError::invalid_schema("package has no manifest.json"))?;
    if manifest["format"] != json!(PACKAGE_FORMAT) {
        return Err(WsError::invalid_schema("not an AI Workspace package"));
    }
    if manifest["format_version"] != json!(FORMAT_VERSION) {
        return Err(WsError::new(Code::UnsupportedVersion, format!("package format_version {} is not supported", manifest["format_version"])));
    }
    Ok(OpenedPackage { manifest, collab, personal })
}

pub struct LoadSpec<'a> {
    pub workspace_id: &'a str,
    pub title: &'a str,
    pub content_root: &'a str,
    /// Original CRDT snapshots (personal backup only): `entity_id → (lineage_id, bytes)`.
    pub collab: BTreeMap<String, (String, Vec<u8>)>,
    pub personal: Vec<Value>,
    pub forked_from: Option<Value>,
}

/// Build a complete Workspace folder at `dir` from a content root: one
/// `origin: "import"` commit at `seq = 1` under a fresh epoch, followed by the
/// assertion that re-materializing yields the very same content root. `owner`
/// is whose buckets retain it (the caller, or the owner of the Workspace a
/// restore replaces).
pub fn load_workspace(
    dir: &Path,
    spec: LoadSpec,
    caller: &Caller,
    owner: &str,
    objects: std::sync::Arc<ObjectStore>,
    clock: crate::workspace::Clock,
) -> WsResult<Workspace> {
    let mut ws = Workspace::create(dir, spec.workspace_id, spec.title, &caller.principal, objects.clone(), clock)?;
    if owner != caller.principal {
        ws.local.execute("UPDATE local_meta SET value = ?1 WHERE key = 'owner'", [owner]).map_err(db_err)?;
    }
    let collab = &spec.collab;
    let plan = load_ops(objects.as_ref(), spec.content_root, &|id| collab.get(id).cloned())?;
    // assets the package brought along are available for the import commit: bound for the new Workspace now
    for a in &plan.asset_objects {
        if objects.availability(a) == crate::objects::Availability::Available {
            let size = objects.file_size(a)?;
            let media_type = sniff_prefix(&objects.file_head(a, 512)?);
            ws.stage_bound(caller, a, media_type, size)?;
        }
    }
    let commit = |ws: &mut Workspace, key: &str, ops: Vec<Value>| -> WsResult<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let req = json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": ws.workspace_id, "epoch": ws.epoch,
                          "idempotency_key": key, "origin": "import", "operations": ops });
        let r = ws.commit(&req, caller, &CommitOpts { internal: true, import: true, undoes: None, as_principal: None, maintenance: false, as_source: false });
        if r["status"] != json!("accepted") {
            return Err(WsError::invalid_schema("package content was rejected").with_data(r));
        }
        Ok(())
    };
    commit(&mut ws, "import", plan.ops)?;
    // a mismatch here is an implementation defect; the package must not become visible
    let check = ws.materialize_for(&Access::full(&caller.principal), "import", &caller.principal)?;
    if check.content_root != spec.content_root {
        return Err(WsError::io(format!("import verification failed: content root {} != {}", check.content_root, spec.content_root)));
    }
    let personal: Vec<Value> = spec
        .personal
        .iter()
        .map(|p| json!({ "op": "entity.create", "entity_id": p["entity_id"], "type_id": p["type_id"], "parent_id": p["parent_id"],
                         "order_key": p["order_key"], "payload": p["payload"], "scope": "personal" }))
        .collect();
    commit(&mut ws, "import-personal", personal)?;
    if let Some(f) = &spec.forked_from {
        meta_set(&ws.doc, "forked_from", &f.to_string())?;
    }
    Ok(ws)
}

fn require_update(access: &Access) -> WsResult<()> {
    if !access.ws_caps.has(Cap::Update) && !access.ws_caps.has(Cap::Structure) {
        return Err(WsError::denied("update or structure capability required"));
    }
    Ok(())
}
