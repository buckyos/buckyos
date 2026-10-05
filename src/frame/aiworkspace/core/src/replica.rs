//! The offline replica's engine (design doc §6.3, §6.4), shared by native
//! tests and the browser Worker (through the WASM facade).
//!
//! The replica keeps a **confirmed layer** (state the backend accepted, up to
//! `confirmed_seq`) and an ordered list of **pending submissions**. The working
//! view is always `confirmed + pending` replayed through the very same planner
//! the backend runs. Remote changes advance the confirmed layer and the pending
//! submissions are re-planned on top; one that no longer applies leaves the
//! working view but its request is kept for the user.
//!
//! A remote snapshot never overwrites the working view, and a provisional local
//! version is never treated as an accepted one.

use crate::access::Access;
use crate::error::{Code, WsError, WsResult};
use crate::model::*;
use crate::plan::{plan_commit, CommitFailure, CommitRequest, Limits, PlanEnv};
use crate::richtext;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub key: String,
    /// The Commit request. `expect` values that refer to a version produced by an
    /// earlier pending submission are stored as `{ "after": "<its key>" }`.
    pub request: Value,
    /// queued | sending | unknown | conflict | rejected | blocked_asset
    pub state: String,
    pub result: Option<Value>,
    /// Applied to the current working view (under this provisional `seq`)?
    pub provisional_seq: Option<u64>,
}

impl Pending {
    pub fn to_json(&self) -> Value {
        json!({ "idempotency_key": self.key, "request": self.request, "state": self.state, "result": self.result,
                "applied": self.provisional_seq.is_some() })
    }
    /// Still waiting for a verdict of the backend (and therefore part of the working view if it applies).
    fn live(&self) -> bool {
        matches!(self.state.as_str(), "queued" | "sending" | "unknown" | "blocked_asset")
    }
}

pub struct Replica {
    pub principal: String,
    pub workspace_id: String,
    pub epoch: String,
    pub confirmed: MemStore,
    pub confirmed_seq: u64,
    pub working: MemStore,
    pub pending: Vec<Pending>,
    /// Own submissions the backend accepted: `key → seq` (resolves `{ after }` placeholders).
    pub accepted: BTreeMap<String, u64>,
    pub peer_id: u64,
    pub limits: Limits,
    /// Rows of the confirmed layer changed by `apply_remote` and not yet handed out
    /// through `take_confirmed_delta` (what the durable form has to write).
    dirty: Dirty,
}

/// Keys of confirmed-layer rows that changed since the last `take_confirmed_delta`.
#[derive(Default)]
struct Dirty {
    entities: BTreeSet<String>,
    edges: BTreeSet<String>,
    fields: BTreeSet<(String, String)>,
    records: BTreeSet<(String, String)>,
    richtexts: BTreeSet<String>,
    assets: BTreeSet<String>,
    ref_add: BTreeSet<RefEdge>,
    ref_del: BTreeSet<RefEdge>,
}

impl Dirty {
    fn note(&mut self, c: &Changes) {
        self.entities.extend(c.entities.keys().cloned());
        self.edges.extend(c.edges.keys().cloned());
        self.fields.extend(c.fields.keys().cloned());
        self.records.extend(c.records.keys().cloned());
        self.richtexts.extend(c.richtexts.keys().cloned());
        self.assets.extend(c.assets.keys().cloned());
        // same order as `MemStore::apply`: deletions first, then additions
        for r in &c.ref_del {
            self.ref_add.remove(r);
            self.ref_del.insert(r.clone());
        }
        for r in &c.ref_add {
            self.ref_del.remove(r);
            self.ref_add.insert(r.clone());
        }
    }
}

fn env(seq: u64, principal: &str, origin: &str, run_id: Option<String>, now: &str, replay: bool, peer_id: u64, limits: &Limits) -> PlanEnv {
    PlanEnv {
        seq,
        principal: principal.to_string(),
        origin: origin.to_string(),
        run_id,
        now: now.to_string(),
        internal: replay,
        import: replay,
        replay,
        peer_id,
        limits: limits.clone(),
    }
}

/// Rewrite every `expect: { rev }` / `{ after }` in an operation tree.
fn map_expects(v: &mut Value, f: &mut dyn FnMut(&mut Value) -> WsResult<()>) -> WsResult<()> {
    match v {
        Value::Object(o) => {
            for (k, child) in o.iter_mut() {
                if k == "expect" && child.is_object() {
                    f(child)?;
                } else {
                    map_expects(child, f)?;
                }
            }
        }
        Value::Array(a) => {
            for child in a {
                map_expects(child, f)?;
            }
        }
        _ => {}
    }
    Ok(())
}

impl Replica {
    pub fn new(principal: &str, workspace_id: &str, epoch: &str, confirmed: MemStore, confirmed_seq: u64, peer_id: u64) -> Replica {
        let working = confirmed.clone();
        Replica {
            principal: principal.to_string(),
            workspace_id: workspace_id.to_string(),
            epoch: epoch.to_string(),
            confirmed,
            confirmed_seq,
            working,
            pending: vec![],
            accepted: BTreeMap::new(),
            peer_id,
            limits: Limits::default(),
            dirty: Dirty::default(),
        }
    }

    /// The replica plans with full capabilities: the backend is the only place
    /// authorization is decided, and it decides again on every submission.
    fn access(&self) -> Access {
        Access::full(&self.principal)
    }

    fn provisional_key(&self, seq: u64) -> Option<&str> {
        self.pending.iter().find(|p| p.provisional_seq == Some(seq)).map(|p| p.key.as_str())
    }

    /// `{ after: key }` → `{ rev }` of where that submission currently stands.
    fn resolve(&self, request: &Value, for_send: bool) -> WsResult<Value> {
        let mut out = request.clone();
        map_expects(&mut out, &mut |e| {
            if let Some(key) = e.get("after").and_then(Value::as_str) {
                let rev = match self.accepted.get(key) {
                    Some(seq) => *seq,
                    None if for_send => {
                        return Err(WsError::new(Code::DependencyUnavailable, format!("depends on unconfirmed submission {key}")))
                    }
                    None => self.pending.iter().find(|p| p.key == key).and_then(|p| p.provisional_seq).ok_or_else(|| {
                        WsError::new(Code::RevisionConflict, format!("depends on submission {key} which did not apply"))
                    })?,
                };
                *e = json!({ "rev": rev });
            }
            Ok(())
        })?;
        Ok(out)
    }

    fn plan_local(&self, base: &MemStore, request: &Value, seq: u64) -> Result<Changes, CommitFailure> {
        let reject = |error: WsError| CommitFailure::Rejected { error, op_index: None };
        let resolved = self.resolve(request, false).map_err(reject)?;
        let req = CommitRequest::parse(&resolved).map_err(reject)?;
        let e = env(seq, &self.principal, &req.origin, req.run_id.clone(), "1970-01-01T00:00:00.000Z", false, self.peer_id, &self.limits);
        let access = self.access();
        let plan = plan_commit(base, &req, &e, &access)?;
        if !plan.lock_targets.is_empty() {
            // lock_required objects are read-only while the backend is unreachable; online, the
            // session holds the lock and the backend checks it — the replica cannot know either way
            return Err(reject(WsError::new(Code::LockRequired, "this object needs a write lock; edit it online")
                .with_data(json!({ "entity_ids": plan.lock_targets }))));
        }
        Ok(plan.overlay.into_changes())
    }

    /// Recompute the working view: confirmed layer + every still-undecided
    /// submission, in order. A `queued` one that no longer applies becomes
    /// `conflict`; a `sending`/`unknown` one keeps its state (only the backend
    /// can say whether it was accepted) but is left out of the view.
    pub fn rebuild(&mut self) {
        let mut working = self.confirmed.clone();
        let mut seq = self.confirmed_seq;
        for i in 0..self.pending.len() {
            self.pending[i].provisional_seq = None;
            if !self.pending[i].live() {
                continue;
            }
            match self.plan_local(&working, &self.pending[i].request.clone(), seq + 1) {
                Ok(changes) => {
                    seq += 1;
                    working.apply(changes);
                    self.pending[i].provisional_seq = Some(seq);
                }
                Err(f) => {
                    if self.pending[i].state == "queued" {
                        self.pending[i].state = "conflict".into();
                        self.pending[i].result = Some(f.to_json());
                    }
                }
            }
        }
        self.working = working;
    }

    /// A local edit. Planned exactly like the backend would; on success it joins
    /// the pending list (the caller persists that row before reporting
    /// "saved on this device").
    pub fn submit_local(&mut self, request: &Value) -> Value {
        let mut stored = request.clone();
        // versions produced by earlier pending submissions are provisional: remember the dependency, not the number
        let confirmed = self.confirmed_seq;
        let mapped = map_expects(&mut stored, &mut |e| {
            if let Some(rev) = e.get("rev").and_then(Value::as_u64).filter(|r| *r > confirmed) {
                let key = self.provisional_key(rev).ok_or_else(|| WsError::invalid_op("expect refers to an unknown local version"))?;
                *e = json!({ "after": key });
            }
            Ok(())
        });
        if let Err(e) = mapped {
            return CommitFailure::Rejected { error: e, op_index: None }.to_json();
        }
        let key = match stored.get("idempotency_key").and_then(Value::as_str) {
            Some(k) if !self.pending.iter().any(|p| p.key == k) && !self.accepted.contains_key(k) => k.to_string(),
            _ => return CommitFailure::Rejected { error: WsError::invalid_op("a fresh idempotency_key is required"), op_index: None }.to_json(),
        };
        let seq = self.pending.iter().filter_map(|p| p.provisional_seq).max().unwrap_or(self.confirmed_seq) + 1;
        match self.plan_local(&self.working, &stored, seq) {
            Ok(changes) => {
                self.working.apply(changes);
                self.pending.push(Pending { key: key.clone(), request: stored, state: "queued".into(), result: None, provisional_seq: Some(seq) });
                json!({ "status": "saved_locally", "idempotency_key": key, "provisional_seq": seq })
            }
            Err(f) => f.to_json(),
        }
    }

    /// Advance the confirmed layer with change-stream events (already accepted:
    /// replayed, not re-judged). An event carrying one of our keys settles that
    /// submission — matching is by key only, never by content.
    pub fn apply_remote(&mut self, events: &[Value]) -> WsResult<Value> {
        let mut settled = Vec::new();
        for ev in events {
            let seq = ev["seq"].as_u64().ok_or_else(|| WsError::invalid_schema("event without seq"))?;
            if seq <= self.confirmed_seq {
                continue; // duplicate delivery
            }
            if seq != self.confirmed_seq + 1 {
                return Err(WsError::new(Code::BaseUnknown, format!("change stream gap: have {}, got {seq}", self.confirmed_seq)));
            }
            if let Some(ops) = ev.get("ops").and_then(Value::as_array).filter(|o| !o.is_empty()) {
                let req = CommitRequest::parse(&json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id,
                    "epoch": self.epoch, "idempotency_key": "remote", "origin": ev.get("origin").cloned().unwrap_or(json!("human")),
                    "run_id": ev.get("run_id").cloned().unwrap_or(Value::Null), "operations": ops }))?;
                let author = ev["author"].as_str().unwrap_or("");
                let e = env(seq, author, &req.origin, req.run_id.clone(), ev["accepted_at"].as_str().unwrap_or(""), true, self.peer_id, &self.limits);
                let access = Access::full(author);
                let changes = match plan_commit(&self.confirmed, &req, &e, &access) {
                    Ok(plan) => plan.overlay.into_changes(),
                    Err(f) => return Err(WsError::io(format!("replica cannot apply accepted commit {seq}")).with_data(f.to_json())),
                };
                self.dirty.note(&changes);
                self.confirmed.apply(changes);
            }
            self.confirmed_seq = seq;
            if let Some(key) = ev.get("idempotency_key").and_then(Value::as_str) {
                if let Some(pos) = self.pending.iter().position(|p| p.key == key) {
                    self.pending.remove(pos);
                    settled.push(key.to_string());
                }
                self.accepted.insert(key.to_string(), seq);
            }
        }
        self.rebuild();
        Ok(json!({ "confirmed_seq": self.confirmed_seq, "settled": settled, "pending": self.pending.iter().map(Pending::to_json).collect::<Vec<_>>() }))
    }

    /// The next submission to send, with its placeholders resolved to accepted versions.
    pub fn next_to_send(&self) -> WsResult<Option<Value>> {
        match self.pending.iter().find(|p| p.state == "queued") {
            Some(p) => Ok(Some(self.resolve(&p.request, true)?)),
            None => Ok(None),
        }
    }

    /// Record what happened to a submission. `conflict`/`rejected` leave the
    /// working view (the request stays available); later independent
    /// submissions keep going.
    pub fn mark(&mut self, key: &str, state: &str, result: Option<Value>) -> WsResult<()> {
        if !matches!(state, "queued" | "sending" | "unknown" | "conflict" | "rejected" | "blocked_asset") {
            return Err(WsError::invalid_op("unknown pending state"));
        }
        let p = self.pending.iter_mut().find(|p| p.key == key).ok_or_else(|| WsError::not_found("no such pending submission"))?;
        let was_live = p.live();
        p.state = state.to_string();
        if result.is_some() {
            p.result = result;
        }
        // queued → sending → unknown → queued: the submission stays in the working view exactly as
        // it was, so nothing has to be re-planned (the send loop does this twice per submission)
        if !(was_live && p.live() && p.provisional_seq.is_some()) {
            self.rebuild();
        }
        Ok(())
    }

    /// Remove a submission the user discarded (or undid before it was sent).
    pub fn discard(&mut self, key: &str) -> Option<Pending> {
        let pos = self.pending.iter().position(|p| p.key == key)?;
        let p = self.pending.remove(pos);
        self.rebuild();
        Some(p)
    }

    pub fn pending_json(&self) -> Value {
        json!(self.pending.iter().map(Pending::to_json).collect::<Vec<_>>())
    }

    /// Restore persisted pending submissions (after a restart of the tab/Worker).
    pub fn restore_pending(&mut self, rows: &[Value]) {
        for r in rows {
            self.pending.push(Pending {
                key: r["idempotency_key"].as_str().unwrap_or("").to_string(),
                request: r["request"].clone(),
                state: r["state"].as_str().unwrap_or("queued").to_string(),
                result: r.get("result").filter(|v| !v.is_null()).cloned(),
                provisional_seq: None,
            });
        }
        self.rebuild();
    }

    /// Rows of the confirmed layer that `apply_remote` changed since the last call, in the shape
    /// of `dump_rows` plus `refs_deleted`. Every row is an upsert by its primary key; the confirmed
    /// layer never deletes rows other than references. The durable form writes this delta, the new
    /// `confirmed_seq` and the pending list in one transaction.
    pub fn take_confirmed_delta(&mut self) -> WsResult<Value> {
        let d = std::mem::take(&mut self.dirty);
        let m = &self.confirmed;
        let mut rich = Vec::new();
        for id in &d.richtexts {
            if let Some(st) = m.richtexts.get(id) {
                rich.push(richtext_row(id, st)?);
            }
        }
        Ok(json!({
            "entities": d.entities.iter().filter_map(|k| m.entities.get(k)).map(entity_row).collect::<Vec<_>>(),
            "tree_edges": d.edges.iter().filter_map(|k| m.edges.get(k)).map(edge_row).collect::<Vec<_>>(),
            "table_fields": d.fields.iter().filter_map(|k| m.fields.get(k)).map(field_row).collect::<Vec<_>>(),
            "table_records": d.records.iter().filter_map(|k| m.records.get(k)).map(record_row).collect::<Vec<_>>(),
            "richtext_states": rich,
            "refs": d.ref_add.iter().map(ref_row).collect::<Vec<_>>(),
            "refs_deleted": d.ref_del.iter().map(ref_row).collect::<Vec<_>>(),
            "assets": d.assets.iter().filter_map(|k| m.assets.get(k)).map(asset_row).collect::<Vec<_>>(),
        }))
    }
}

// ---------------------------------------------------------------------------
// rows <-> MemStore: the shape of the replica database tables (design §4.2)
// ---------------------------------------------------------------------------

fn col<'v>(row: &'v Value, name: &str) -> &'v Value {
    row.get(name).unwrap_or(&Value::Null)
}
fn text(row: &Value, name: &str) -> WsResult<String> {
    col(row, name).as_str().map(str::to_string).ok_or_else(|| WsError::invalid_schema(format!("row column {name} missing")))
}
fn num(row: &Value, name: &str) -> u64 {
    col(row, name).as_u64().unwrap_or(0)
}
fn opt_num(row: &Value, name: &str) -> Option<u64> {
    col(row, name).as_u64()
}
fn parsed<T: serde::de::DeserializeOwned>(row: &Value, name: &str) -> WsResult<T> {
    let v = match col(row, name) {
        Value::String(s) => serde_json::from_str(s).map_err(|e| WsError::invalid_schema(format!("{name}: {e}")))?,
        other => other.clone(),
    };
    serde_json::from_value(v).map_err(|e| WsError::invalid_schema(format!("{name}: {e}")))
}
fn bytes(row: &Value, name: &str) -> WsResult<Vec<u8>> {
    match col(row, name) {
        Value::String(s) => B64.decode(s).map_err(|_| WsError::invalid_schema(format!("{name} must be base64"))),
        Value::Array(a) => Ok(a.iter().filter_map(|b| b.as_u64().map(|b| b as u8)).collect()),
        _ => Err(WsError::invalid_schema(format!("{name} missing"))),
    }
}

/// Build the confirmed layer from replica database rows:
/// `{ entities, tree_edges, table_fields, table_records, richtext_states, refs, assets }`,
/// each an array of row objects with the column names of the document DDL
/// (BLOB columns as base64 strings).
pub fn load_rows(tables: &Value) -> WsResult<MemStore> {
    let rows = |name: &str| tables.get(name).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut m = MemStore::default();
    for r in rows("entities") {
        let e = EntityRow {
            entity_id: text(&r, "entity_id")?,
            type_id: text(&r, "type_id")?,
            schema_version: num(&r, "schema_version") as u32,
            scope: text(&r, "scope")?,
            name: col(&r, "name").as_str().map(str::to_string),
            write_policy: text(&r, "write_policy")?,
            payload: parsed(&r, "payload_json")?,
            key_revs: parsed(&r, "key_revs_json")?,
            created_seq: num(&r, "created_seq"),
            meta_rev: num(&r, "meta_rev"),
            content_rev: num(&r, "content_rev"),
            life_rev: num(&r, "life_rev"),
            deleted_seq: opt_num(&r, "deleted_seq"),
        };
        m.entities.insert(e.entity_id.clone(), e);
    }
    for r in rows("tree_edges") {
        let placement = match col(&r, "placement_json") {
            Value::Null => None,
            _ => Some(parsed::<Value>(&r, "placement_json")?),
        };
        let e = TreeEdge { child_id: text(&r, "child_id")?, parent_id: text(&r, "parent_id")?, order_key: text(&r, "order_key")?, placement, struct_rev: num(&r, "struct_rev") };
        m.edges.insert(e.child_id.clone(), e);
    }
    for r in rows("table_fields") {
        let f = FieldRow {
            source_id: text(&r, "source_id")?,
            field_id: text(&r, "field_id")?,
            def: parsed(&r, "def_json")?,
            order_key: text(&r, "order_key")?,
            def_rev: num(&r, "def_rev"),
            type_rev: num(&r, "type_rev"),
            values_rev: num(&r, "values_rev"),
            deleted_seq: opt_num(&r, "deleted_seq"),
        };
        m.fields.insert((f.source_id.clone(), f.field_id.clone()), f);
    }
    for r in rows("table_records") {
        let rec = RecordRow {
            source_id: text(&r, "source_id")?,
            record_id: text(&r, "record_id")?,
            values: parsed(&r, "values_json")?,
            revs: parsed(&r, "revs_json")?,
            meta: parsed(&r, "meta_json")?,
            body_entity_id: col(&r, "body_entity_id").as_str().map(str::to_string),
            created_seq: num(&r, "created_seq"),
            rev: num(&r, "rev"),
            deleted_seq: opt_num(&r, "deleted_seq"),
        };
        m.records.insert((rec.source_id.clone(), rec.record_id.clone()), rec);
    }
    for r in rows("richtext_states") {
        let id = text(&r, "entity_id")?;
        let doc = richtext::load_doc(&bytes(&r, "snapshot")?, std::iter::empty())?;
        let meta = RichTextMeta {
            entity_id: id.clone(),
            lineage_id: text(&r, "lineage_id")?,
            engine: text(&r, "engine")?,
            engine_version: text(&r, "engine_version")?,
            encoding: text(&r, "encoding")?,
            ast: parsed(&r, "ast_json")?,
            block_index: parsed(&r, "block_index_json")?,
        };
        m.richtexts.insert(id, RichTextState { meta, doc });
    }
    for r in rows("refs") {
        m.refs.insert(RefEdge {
            src_entity_id: text(&r, "src_entity_id")?,
            src_selector: text(&r, "src_selector")?,
            kind: text(&r, "kind")?,
            dst_workspace_id: text(&r, "dst_workspace_id")?,
            dst_entity_id: text(&r, "dst_entity_id")?,
            dst_object_id: text(&r, "dst_object_id")?,
            dst_query_json: text(&r, "dst_query_json")?,
        });
    }
    for r in rows("assets") {
        let a = AssetInfo { object_id: text(&r, "object_id")?, media_type: text(&r, "media_type")?, size: num(&r, "size") };
        m.assets.insert(a.object_id.clone(), a);
    }
    Ok(m)
}

fn jtext(v: &impl serde::Serialize) -> Value {
    Value::String(serde_json::to_string(v).expect("row"))
}

fn entity_row(e: &EntityRow) -> Value {
    json!({
        "entity_id": e.entity_id, "type_id": e.type_id, "schema_version": e.schema_version, "scope": e.scope, "name": e.name,
        "write_policy": e.write_policy, "payload_json": jtext(&e.payload), "key_revs_json": jtext(&e.key_revs),
        "created_seq": e.created_seq, "meta_rev": e.meta_rev, "content_rev": e.content_rev, "life_rev": e.life_rev, "deleted_seq": e.deleted_seq
    })
}
fn edge_row(e: &TreeEdge) -> Value {
    json!({
        "child_id": e.child_id, "parent_id": e.parent_id, "order_key": e.order_key,
        "placement_json": e.placement.as_ref().map(jtext), "struct_rev": e.struct_rev
    })
}
fn field_row(f: &FieldRow) -> Value {
    json!({
        "source_id": f.source_id, "field_id": f.field_id, "def_json": jtext(&f.def), "order_key": f.order_key,
        "def_rev": f.def_rev, "type_rev": f.type_rev, "values_rev": f.values_rev, "deleted_seq": f.deleted_seq
    })
}
fn record_row(r: &RecordRow) -> Value {
    json!({
        "source_id": r.source_id, "record_id": r.record_id, "values_json": jtext(&r.values), "revs_json": jtext(&r.revs),
        "meta_json": jtext(&r.meta), "body_entity_id": r.body_entity_id, "created_seq": r.created_seq, "rev": r.rev, "deleted_seq": r.deleted_seq
    })
}
fn richtext_row(id: &str, st: &RichTextState) -> WsResult<Value> {
    Ok(json!({ "entity_id": id, "lineage_id": st.meta.lineage_id, "engine": st.meta.engine, "engine_version": st.meta.engine_version,
        "encoding": st.meta.encoding, "snapshot": B64.encode(richtext::export_snapshot(&st.doc)?), "snapshot_seq": 0,
        "ast_json": jtext(&st.meta.ast), "block_index_json": jtext(&st.meta.block_index) }))
}
fn ref_row(r: &RefEdge) -> Value {
    json!({
        "src_entity_id": r.src_entity_id, "src_selector": r.src_selector, "kind": r.kind, "dst_workspace_id": r.dst_workspace_id,
        "dst_entity_id": r.dst_entity_id, "dst_object_id": r.dst_object_id, "dst_query_json": r.dst_query_json
    })
}
fn asset_row(a: &AssetInfo) -> Value {
    json!({ "object_id": a.object_id, "media_type": a.media_type, "size": a.size, "first_seq": 0 })
}

/// The whole store as table rows (same shape `load_rows` reads).
pub fn dump_rows(m: &MemStore) -> WsResult<Value> {
    let mut rich = Vec::new();
    for (id, st) in &m.richtexts {
        rich.push(richtext_row(id, st)?);
    }
    Ok(json!({
        "entities": m.entities.values().map(entity_row).collect::<Vec<_>>(),
        "tree_edges": m.edges.values().map(edge_row).collect::<Vec<_>>(),
        "table_fields": m.fields.values().map(field_row).collect::<Vec<_>>(),
        "table_records": m.records.values().map(record_row).collect::<Vec<_>>(),
        "richtext_states": rich,
        "refs": m.refs.iter().map(ref_row).collect::<Vec<_>>(),
        "assets": m.assets.values().map(asset_row).collect::<Vec<_>>(),
    }))
}
