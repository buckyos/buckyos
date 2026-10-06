//! One Workspace folder: `doc.sqlite` (the portable document) + `local.sqlite`
//! (grants, locks, runs — this deployment only). A `Workspace` value is the
//! single writer of its folder: every Commit is planned, checked and applied
//! serially through it (design doc §2.5.2, §4.3).

use crate::docdb::{apply_changes, db_err, meta_get, meta_set, DocCache, SqlCtx, WriteStats};
use crate::objects::FsObjectStore;
use crate::schema::{DOC_DDL, LOCAL_DDL, STORAGE_SCHEMA_VERSION};
use aiworkspace_core::access::{Access, Cap, CapSet};
use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use aiworkspace_core::id::{check_id, check_idempotency_key, is_prefixed_id, prefixed_id};
use aiworkspace_core::model::*;
use aiworkspace_core::plan::{plan_commit, CommitFailure, CommitRequest, Limits, PlanEnv, PlannedOp, Touched};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{richtext, Code, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const LOCK_LEASE_MS: i64 = 60_000;
pub const LOCK_IDLE_MS: i64 = 600_000;
pub const COMPACT_UPDATES: i64 = 200;
pub const COMPACT_BYTES: i64 = 1024 * 1024;

/// The identity the service runtime verified. Nothing a client writes into a
/// request body is ever used as identity.
#[derive(Debug, Clone)]
pub struct Caller {
    pub principal: String,
    pub app_id: Option<String>,
}

impl Caller {
    pub fn user(principal: &str) -> Caller {
        Caller { principal: principal.to_string(), app_id: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailAt {
    BeforeTxn,
    InTxn,
    /// Durable, but nothing published and no response sent.
    AfterCommit,
}

/// Fault injection for atomicity/crash tests. `abort` kills the process.
#[derive(Debug, Clone)]
pub struct FailPoint {
    pub at: FailAt,
    pub abort: bool,
    /// Fire on the n-th commit reaching the point (1 = next).
    pub countdown: u32,
}

impl FailPoint {
    /// `before_txn|in_txn|after_commit[:abort][:N]`
    pub fn parse(s: &str) -> Option<FailPoint> {
        let mut parts = s.split(':');
        let at = match parts.next()? {
            "before_txn" => FailAt::BeforeTxn,
            "in_txn" => FailAt::InTxn,
            "after_commit" => FailAt::AfterCommit,
            _ => return None,
        };
        let (mut abort, mut countdown) = (false, 1);
        for p in parts {
            match p.parse::<u32>() {
                Ok(n) => countdown = n.max(1),
                Err(_) => abort = p == "abort",
            }
        }
        Some(FailPoint { at, abort, countdown })
    }
}

#[derive(Debug, Clone, Default)]
pub struct CommitOpts {
    /// Undo path / import: internal operations and `expect: "any"` are allowed.
    pub internal: bool,
    pub import: bool,
    /// `commit_id` this commit compensates.
    pub undoes: Option<String>,
}

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    })
}

pub fn random_id(prefix: &str) -> String {
    prefixed_id(prefix, &rand::random::<[u8; 17]>())
}

pub struct Workspace {
    pub dir: PathBuf,
    pub workspace_id: String,
    pub epoch: String,
    pub(crate) doc: Connection,
    pub(crate) local: Connection,
    pub(crate) cache: RefCell<DocCache>,
    pub objects: Arc<FsObjectStore>,
    pub head_seq: u64,
    pub head_commit_id: String,
    pub(crate) peer_id: u64,
    pub limits: Limits,
    pub clock: Clock,
    pub failpoint: Option<FailPoint>,
    /// Row counts written by the last accepted commit.
    pub last_stats: WriteStats,
    /// Called after a commit became durable and was published in memory.
    pub on_commit: Option<Box<dyn Fn(u64) + Send>>,
    pub on_lock_change: Option<Box<dyn Fn() + Send>>,
}

fn open_db(path: &Path, synchronous_full: bool) -> WsResult<Connection> {
    let conn = Connection::open(path).map_err(db_err)?;
    conn.pragma_update(None, "journal_mode", "WAL").map_err(db_err)?;
    // an answered commit must survive power loss
    conn.pragma_update(None, "synchronous", if synchronous_full { "FULL" } else { "NORMAL" }).map_err(db_err)?;
    conn.pragma_update(None, "foreign_keys", "ON").map_err(db_err)?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_err)?;
    Ok(conn)
}

pub fn err_json(e: &WsError) -> Value {
    json!({ "ok": false, "error": e.to_json() })
}

impl Workspace {
    pub fn now(&self) -> String {
        format_utc_ms((self.clock)())
    }

    pub(crate) fn ctx<'a>(&'a self, now: &'a str) -> SqlCtx<'a> {
        SqlCtx { conn: &self.doc, local: Some(&self.local), cache: &self.cache, now }
    }

    /// Build a new Workspace folder at `dir` (a temporary name; the caller
    /// renames it into place once complete).
    pub fn create(dir: &Path, workspace_id: &str, title: &str, owner: &str, objects: Arc<FsObjectStore>, clock: Clock) -> WsResult<Workspace> {
        if !is_prefixed_id("ws_", workspace_id) {
            return Err(WsError::invalid_op("workspace_id must be ws_ + 26 base32 chars"));
        }
        std::fs::create_dir_all(dir.join("staging")).map_err(|e| WsError::io(e.to_string()))?;
        let doc = open_db(&dir.join("doc.sqlite"), true)?;
        let local = open_db(&dir.join("local.sqlite"), false)?;
        doc.execute_batch(DOC_DDL).map_err(db_err)?;
        local.execute_batch(LOCAL_DDL).map_err(db_err)?;
        let now = format_utc_ms(clock());
        let epoch = random_id("ep_");
        let peer_id: u64 = rand::random::<u64>() >> 1;
        for (k, v) in [
            ("workspace_id", workspace_id.to_string()),
            ("epoch", epoch.clone()),
            ("format_version", FORMAT_VERSION.to_string()),
            ("storage_schema_version", STORAGE_SCHEMA_VERSION.to_string()),
            ("head_seq", "0".to_string()),
            ("head_commit_id", String::new()),
            ("created_at", now.clone()),
            ("title", title.to_string()),
            ("peer_id", peer_id.to_string()),
        ] {
            meta_set(&doc, k, &v)?;
        }
        // the root and the system nodes of the two trees (phase two §4.5) exist from `seq` 0
        let mut changes = Overlay::new(&MemStore::default()).into_changes();
        for (e, edge) in system_entities(Some(title)) {
            changes.entities.insert(e.entity_id.clone(), e);
            if let Some(edge) = edge {
                changes.edges.insert(edge.child_id.clone(), edge);
            }
        }
        apply_changes(&doc, &changes, 0)?;
        local
            .execute("INSERT INTO grants (subject, scope_entity_id, capabilities) VALUES (?1, '', ?2)", params![owner, CapSet::ALL.0])
            .map_err(db_err)?;
        local.execute("INSERT INTO local_meta (key, value) VALUES ('owner', ?1)", [owner]).map_err(db_err)?;
        drop(doc);
        drop(local);
        Workspace::open(dir, objects, clock)
    }

    pub fn open(dir: &Path, objects: Arc<FsObjectStore>, clock: Clock) -> WsResult<Workspace> {
        if !dir.join("doc.sqlite").exists() {
            return Err(WsError::not_found("workspace not found"));
        }
        let doc = open_db(&dir.join("doc.sqlite"), true)?;
        let local = open_db(&dir.join("local.sqlite"), false)?;
        let get = |k: &str| -> WsResult<String> {
            meta_get(&doc, k)?.ok_or_else(|| WsError::io(format!("workspace_meta.{k} missing")))
        };
        // unknown storage or format versions are refused; the file is never modified or emptied
        let storage: u32 = get("storage_schema_version")?.parse().unwrap_or(0);
        if storage != STORAGE_SCHEMA_VERSION {
            return Err(WsError::new(Code::UnsupportedVersion, format!("storage_schema_version {storage} is not supported")));
        }
        if get("format_version")? != FORMAT_VERSION {
            return Err(WsError::new(Code::UnsupportedVersion, "format_version is not supported"));
        }
        let ws = Workspace {
            dir: dir.to_path_buf(),
            workspace_id: get("workspace_id")?,
            epoch: get("epoch")?,
            head_seq: get("head_seq")?.parse().unwrap_or(0),
            head_commit_id: get("head_commit_id")?,
            peer_id: get("peer_id")?.parse().unwrap_or(1),
            doc,
            local,
            cache: RefCell::new(DocCache::default()),
            objects,
            limits: Limits::default(),
            clock,
            failpoint: None,
            last_stats: WriteStats::default(),
            on_commit: None,
            on_lock_change: None,
        };
        Ok(ws)
    }

    pub fn title(&self) -> String {
        meta_get(&self.doc, "title").ok().flatten().unwrap_or_default()
    }

    // ---- grants ----

    /// Effective grants of a principal: its own rows plus `*` (any authenticated principal).
    pub fn access(&self, principal: &str) -> WsResult<Access> {
        let mut st = self
            .local
            .prepare_cached("SELECT scope_entity_id, capabilities FROM grants WHERE subject = ?1 OR subject = '*'")
            .map_err(db_err)?;
        let rows: Vec<(String, u16)> =
            st.query_map([principal], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let mut access = Access { principal: principal.to_string(), ws_caps: CapSet::NONE, scoped: BTreeMap::new() };
        for (scope, caps) in rows {
            if scope.is_empty() {
                access.ws_caps = access.ws_caps.union(CapSet(caps));
            } else {
                let e = access.scoped.entry(scope).or_insert(CapSet::NONE);
                *e = e.union(CapSet(caps));
            }
        }
        Ok(access)
    }

    pub fn require_ws(&self, caller: &Caller, cap: Cap) -> WsResult<Access> {
        let access = self.access(&caller.principal)?;
        if access.is_empty() {
            // no grant at all: the Workspace does not exist for this principal
            return Err(WsError::not_found("workspace not found"));
        }
        if !access.ws_caps.has(cap) {
            return Err(WsError::denied(format!("{} capability required", aiworkspace_core::access::cap_name(cap))));
        }
        Ok(access)
    }

    pub fn grant(&mut self, caller: &Caller, subject: &str, scope: Option<&str>, caps: &[String]) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        if subject.is_empty() || subject.len() > 256 {
            return Err(WsError::invalid_op("invalid subject"));
        }
        let caps = CapSet::parse(caps)?;
        if let Some(s) = scope {
            let now = self.now();
            if self.ctx(&now).entity(s)?.is_none() {
                return Err(WsError::not_found(format!("entity {s} not found")));
            }
        }
        self.local
            .execute(
                "INSERT OR REPLACE INTO grants (subject, scope_entity_id, capabilities) VALUES (?1, ?2, ?3)",
                params![subject, scope.unwrap_or(""), caps.0],
            )
            .map_err(db_err)?;
        Ok(json!({ "ok": true }))
    }

    pub fn revoke(&mut self, caller: &Caller, subject: &str, scope: Option<&str>) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let owner: Option<String> =
            self.local.query_row("SELECT value FROM local_meta WHERE key = 'owner'", [], |r| r.get(0)).optional().map_err(db_err)?;
        if owner.as_deref() == Some(subject) && scope.is_none() {
            return Err(WsError::invalid_op("the owner's workspace grant cannot be revoked"));
        }
        let n = self
            .local
            .execute("DELETE FROM grants WHERE subject = ?1 AND scope_entity_id = ?2", params![subject, scope.unwrap_or("")])
            .map_err(db_err)?;
        Ok(json!({ "ok": true, "removed": n }))
    }

    /// `ws.list_grants`: the whole list for a manager; anyone else only sees the rows that apply to
    /// them (their own subject and `*`), never the complete name list (phase two §6.3).
    pub fn list_grants(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let manage = access.ws_caps.has(Cap::Manage);
        let sql = if manage {
            "SELECT subject, scope_entity_id, capabilities FROM grants ORDER BY subject, scope_entity_id"
        } else {
            "SELECT subject, scope_entity_id, capabilities FROM grants WHERE subject = ?1 OR subject = '*' ORDER BY subject, scope_entity_id"
        };
        let mut st = self.local.prepare(sql).map_err(db_err)?;
        let params: Vec<&dyn rusqlite::ToSql> = if manage { vec![] } else { vec![&caller.principal] };
        let owner: Option<String> =
            self.local.query_row("SELECT value FROM local_meta WHERE key = 'owner'", [], |r| r.get(0)).optional().map_err(db_err)?;
        let rows: Vec<Value> = st
            .query_map(params.as_slice(), |r| {
                let scope: String = r.get(1)?;
                Ok(json!({ "subject": r.get::<_, String>(0)?, "scope_entity_id": if scope.is_empty() { Value::Null } else { json!(scope) },
                           "capabilities": CapSet(r.get::<_, u16>(2)?).names() }))
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        Ok(json!({ "ok": true, "grants": rows, "complete": manage, "owner": if manage { owner } else { None }, "principal": caller.principal }))
    }

    // ---- user work state (phase two §4.4): per subject, per Workspace, in local.sqlite ----

    /// `ws.get_user_state`: every entry of the caller.
    pub fn get_user_state(&self, caller: &Caller) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let mut st = self.local.prepare("SELECT key, value_json, updated_at FROM user_state WHERE subject = ?1 ORDER BY key").map_err(db_err)?;
        let rows: Vec<(String, String, String)> = st
            .query_map([&caller.principal], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        let mut entries = serde_json::Map::new();
        let mut updated = serde_json::Map::new();
        for (k, v, at) in rows {
            entries.insert(k.clone(), serde_json::from_str(&v).unwrap_or(Value::Null));
            updated.insert(k, json!(at));
        }
        Ok(json!({ "ok": true, "entries": entries, "updated_at": updated }))
    }

    /// `ws.set_user_state`: `entries: { key: value | null }` — null removes. Last writer wins per
    /// entry. Not a document Commit: no history, no change-stream event, no undo.
    pub fn set_user_state(&mut self, caller: &Caller, entries: &Value) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let o = entries.as_object().ok_or_else(|| WsError::invalid_op("entries must be an object"))?;
        if o.len() > 500 {
            return Err(WsError::limit("at most 500 entries per call"));
        }
        let now = self.now();
        let tx = self.local.unchecked_transaction().map_err(db_err)?;
        for (k, v) in o {
            if k.is_empty() || k.len() > 256 {
                return Err(WsError::invalid_op("invalid user state key"));
            }
            if v.is_null() {
                tx.execute("DELETE FROM user_state WHERE subject = ?1 AND key = ?2", params![caller.principal, k]).map_err(db_err)?;
            } else {
                let text = v.to_string();
                if text.len() > 256 * 1024 {
                    return Err(WsError::limit("a user state entry is limited to 256 KiB"));
                }
                tx.execute(
                    "INSERT OR REPLACE INTO user_state (subject, key, value_json, updated_at) VALUES (?1, ?2, ?3, ?4)",
                    params![caller.principal, k, text, now],
                )
                .map_err(db_err)?;
            }
        }
        tx.commit().map_err(db_err)?;
        Ok(json!({ "ok": true, "updated_at": now }))
    }

    pub fn info(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        Ok(json!({ "ok": true, "workspace_id": self.workspace_id, "title": self.title(), "epoch": self.epoch,
                   "head_seq": self.head_seq, "head_commit_id": self.head_commit_id, "format_version": FORMAT_VERSION,
                   "protocol_version": PROTOCOL_VERSION, "capabilities": access.ws_caps.names(),
                   "forked_from": meta_get(&self.doc, "forked_from")?.and_then(|s| serde_json::from_str::<Value>(&s).ok()) }))
    }

    /// Any grant at all (workspace-level or scoped) is enough to know the Workspace exists.
    pub fn require_ws_any(&self, caller: &Caller) -> WsResult<Access> {
        let access = self.access(&caller.principal)?;
        if access.is_empty() {
            return Err(WsError::not_found("workspace not found"));
        }
        Ok(access)
    }

    fn check_epoch(&self, epoch: &str) -> WsResult<()> {
        if epoch != self.epoch {
            return Err(WsError::new(Code::EpochMismatch, "the workspace history was replaced; resynchronize"));
        }
        Ok(())
    }

    // ---- commit pipeline ----

    fn fire(&mut self, at: FailAt) -> WsResult<()> {
        let Some(fp) = self.failpoint.as_mut().filter(|f| f.at == at) else { return Ok(()) };
        if fp.countdown > 1 {
            fp.countdown -= 1;
            return Ok(());
        }
        if fp.abort {
            log::error!("failpoint {:?}: aborting process", at);
            std::process::abort();
        }
        self.failpoint = None;
        Err(WsError::io(format!("injected failure at {at:?}")))
    }

    /// `doc.commit`: always answers with the three-state result.
    pub fn commit(&mut self, req: &Value, caller: &Caller, opts: &CommitOpts) -> Value {
        match self.commit_inner(req, caller, opts, false) {
            Ok(v) => v,
            Err(f) => f.to_json(),
        }
    }

    /// `doc.prepare`: steps 1–3 and the read-only part of step 4. Nothing is stored.
    pub fn prepare(&mut self, req: &Value, caller: &Caller) -> Value {
        match self.commit_inner(req, caller, &CommitOpts::default(), true) {
            Ok(v) => v,
            Err(f) => {
                let mut v = f.to_json();
                if let Ok(d) = CommitRequest::digest(req) {
                    v["request_digest"] = json!(d);
                }
                v
            }
        }
    }

    fn commit_inner(&mut self, req_json: &Value, caller: &Caller, opts: &CommitOpts, dry_run: bool) -> Result<Value, CommitFailure> {
        let reject = |error: WsError| CommitFailure::Rejected { error, op_index: None };
        // 1. entry checks
        let req = CommitRequest::parse(req_json).map_err(reject)?;
        if req.workspace_id != self.workspace_id {
            return Err(reject(WsError::invalid_op("workspace_id does not match")));
        }
        self.check_epoch(&req.epoch).map_err(reject)?;
        if req_json.to_string().len() > MAX_REQUEST_BYTES {
            return Err(reject(WsError::limit("commit request exceeds 8 MiB")));
        }
        let digest = CommitRequest::digest(req_json).map_err(reject)?;
        // idempotency comes before anything that depends on document state: a
        // successful create whose response was lost must not hit its own ID_CONFLICT
        let prior: Option<(String, String)> = self
            .doc
            .query_row(
                "SELECT request_digest, result_json FROM commits WHERE principal = ?1 AND idem_key = ?2",
                params![caller.principal, req.idempotency_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| reject(db_err(e)))?;
        if let Some((prior_digest, result)) = prior {
            if prior_digest != digest {
                return Err(reject(WsError::new(Code::IdempotencyMismatch, "idempotency_key was used with a different request")));
            }
            let mut v: Value = serde_json::from_str(&result).map_err(|e| reject(WsError::io(e.to_string())))?;
            v["replayed"] = json!(true);
            return Ok(v);
        }
        let access = self.access(&caller.principal).map_err(reject)?;
        if access.is_empty() {
            return Err(reject(WsError::not_found("workspace not found")));
        }
        if let Some(run_id) = &req.run_id {
            let state: Option<String> = self
                .local
                .query_row("SELECT state FROM runs WHERE run_id = ?1", [run_id], |r| r.get(0))
                .optional()
                .map_err(|e| reject(db_err(e)))?;
            match state.as_deref() {
                Some("cancelled") => return Err(reject(WsError::new(Code::RunCancelled, "the run was cancelled"))),
                Some(_) => {}
                None => return Err(reject(WsError::invalid_op("unknown run_id"))),
            }
        }
        let seq = self.head_seq + 1;
        let now = self.now();
        let env = PlanEnv {
            seq,
            principal: caller.principal.clone(),
            origin: req.origin.clone(),
            run_id: req.run_id.clone(),
            now: now.clone(),
            internal: opts.internal,
            import: opts.import,
            replay: false,
            peer_id: self.peer_id,
            limits: self.limits.clone(),
        };
        // 2–3. plan on an overlay; the authoritative state is not touched
        let (ops, changes, lock_targets, reports, warnings, read_denied) = {
            let ctx = self.ctx(&now);
            let plan = plan_commit(&ctx, &req, &env, &access)?;
            (plan.ops, plan.overlay.into_changes(), plan.lock_targets, plan.reports, plan.warnings, plan.read_denied)
        };
        // 4. final checks inside the single writer: locks (authorization was evaluated on current grants above)
        self.check_locks(&lock_targets, caller, req.session_id.as_deref(), &now).map_err(reject)?;
        let commit_id = random_id("cm_");
        let touched: Vec<&Touched> = ops.iter().flat_map(|o| o.touched.iter()).collect();
        let visible_touched: Vec<Value> = {
            let ctx = self.ctx(&now);
            let overlay_free = |t: &&&Touched| -> bool {
                // the response only reports what the caller may read; an append-only
                // principal sees just the records it inserted itself
                let readable = changes
                    .entities
                    .get(&t.entity_id)
                    .map(|e| access.sees_scope(e))
                    .unwrap_or(true)
                    && access.can(&ctx, &t.entity_id, Cap::Read).unwrap_or(false);
                readable
                    || t.selector.as_ref().is_some_and(|s| {
                        s["kind"] == json!("table_record") && s["record_id"].as_str().is_some_and(|r| read_denied.contains(r))
                    })
            };
            touched.iter().filter(overlay_free).map(|t| t.to_json()).collect()
        };
        let server_ops: Vec<Value> = ops
            .iter()
            .enumerate()
            .filter(|(_, o)| o.op.get("server_update").is_some())
            .map(|(i, o)| json!({ "op_index": i, "entity_id": o.op["entity_id"], "lineage_id": o.op["lineage_id"], "update": o.op["server_update"] }))
            .collect();
        let mut result = json!({ "status": "accepted", "commit_id": commit_id, "seq": seq, "replayed": false,
                                 "touched": visible_touched, "server_ops": server_ops });
        if !warnings.is_empty() {
            result["warnings"] = json!(warnings);
        }
        if dry_run {
            let mut v = json!({ "status": "ok", "would_be_seq": seq, "touched": result["touched"], "request_digest": digest });
            if !reports.is_empty() {
                v["reports"] = json!(reports);
            }
            return Ok(v);
        }
        self.fire(FailAt::BeforeTxn).map_err(reject)?;
        // one SQLite transaction: data, references, history, idempotency record, head
        let stats = self
            .write_commit(&req, caller, opts, &digest, seq, &commit_id, &now, &ops, &changes, &result)
            .map_err(reject)?;
        // durable from here on; a crash before publishing is recovered from the database
        self.fire(FailAt::AfterCommit).map_err(reject)?;
        // 5. publish
        self.publish(seq, &commit_id, changes, stats, &lock_targets, caller, req.session_id.as_deref(), &now);
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_commit(
        &mut self,
        req: &CommitRequest,
        caller: &Caller,
        opts: &CommitOpts,
        digest: &str,
        seq: u64,
        commit_id: &str,
        now: &str,
        ops: &[PlannedOp],
        changes: &Changes,
        result: &Value,
    ) -> WsResult<WriteStats> {
        // does the in-transaction failpoint fire on this very commit?
        let fail_in_txn = match self.failpoint.as_mut().filter(|f| f.at == FailAt::InTxn) {
            Some(f) if f.countdown > 1 => {
                f.countdown -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        let tx = self.doc.unchecked_transaction().map_err(db_err)?;
        let stats = apply_changes(&tx, changes, seq)?;
        // a generated result gets an addressable version right away (phase two §7.4): its content is
        // materialized and listed in entity_versions together with the dependency record
        let versioned: Vec<String> = ops
            .iter()
            .filter(|o| o.op["op"] == json!("entity.set_derived"))
            .filter_map(|o| o.op["entity_id"].as_str().map(str::to_string))
            .collect();
        if !versioned.is_empty() {
            let ctx = SqlCtx { conn: &tx, local: Some(&self.local), cache: &self.cache, now };
            let mut sink = crate::objects::StoreSink::new(&self.objects);
            for id in versioned {
                let Some(e) = changes.entities.get(&id) else { continue };
                let object_id = aiworkspace_core::materialize::content_object(&ctx, e, &mut sink)?;
                tx.execute(
                    "INSERT INTO entity_versions (entity_id, content_rev, object_id, derived_json, kind, created_at) VALUES (?1, ?2, ?3, ?4, 'generated', ?5) \
                     ON CONFLICT(entity_id, content_rev) DO UPDATE SET object_id = excluded.object_id, derived_json = excluded.derived_json, kind = 'generated', created_at = excluded.created_at",
                    params![id, e.content_rev, object_id, e.derived.as_ref().map(Value::to_string), now],
                )
                .map_err(db_err)?;
            }
        }
        tx.execute(
            "INSERT INTO commits (seq, commit_id, principal, app_id, session_id, origin, run_id, undo_group, undoes, idem_key, \
             request_digest, message, accepted_at, result_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                seq, commit_id, caller.principal, caller.app_id, req.session_id, req.origin, req.run_id, req.undo_group,
                opts.undoes, req.idempotency_key, digest, req.message, now, result.to_string()
            ],
        )
        .map_err(db_err)?;
        {
            let mut st = tx
                .prepare_cached("INSERT INTO commit_ops (seq, op_index, op_json, inverse_json, touched_json) VALUES (?1,?2,?3,?4,?5)")
                .map_err(db_err)?;
            for (i, o) in ops.iter().enumerate() {
                let touched: Vec<Value> = o.touched.iter().map(Touched::to_json).collect();
                st.execute(params![seq, i, o.op.to_string(), o.inverse.as_ref().map(|v| json!(v).to_string()), json!(touched).to_string()])
                    .map_err(db_err)?;
            }
        }
        meta_set(&tx, "head_seq", &seq.to_string())?;
        meta_set(&tx, "head_commit_id", commit_id)?;
        if fail_in_txn {
            drop(tx); // rollback: neither data nor history nor the idempotency record exists
            self.fire(FailAt::InTxn)?;
            return Err(WsError::io("injected failure"));
        }
        tx.commit().map_err(db_err)?;
        Ok(stats)
    }

    #[allow(clippy::too_many_arguments)]
    fn publish(&mut self, seq: u64, commit_id: &str, changes: Changes, stats: WriteStats, locks: &std::collections::BTreeSet<String>,
               caller: &Caller, session: Option<&str>, now: &str) {
        self.head_seq = seq;
        self.head_commit_id = commit_id.to_string();
        self.last_stats = stats;
        let updated: Vec<String> = changes.richtexts.keys().cloned().collect();
        {
            // only now does a candidate document become the authoritative one in memory
            let mut cache = self.cache.borrow_mut();
            for (id, st) in changes.richtexts {
                cache.docs.insert(id, st);
            }
        }
        for id in locks {
            let _ = self.local.execute(
                "UPDATE locks SET last_write_at = ?1 WHERE entity_id = ?2 AND principal = ?3 AND session_id = ?4",
                params![now, id, caller.principal, session.unwrap_or("")],
            );
        }
        for id in updated {
            if let Err(e) = self.compact_richtext(&id, seq) {
                log::warn!("rich text compaction of {id} failed: {e}");
            }
        }
        if let Some(cb) = &self.on_commit {
            cb(seq);
        }
    }

    /// Fold accumulated updates into the stored snapshot. Storage shape only:
    /// full history is kept (no shallow snapshot), no new `seq`, one transaction.
    fn compact_richtext(&mut self, id: &str, seq: u64) -> WsResult<()> {
        let (n, bytes): (i64, i64) = self
            .doc
            .query_row("SELECT COUNT(*), COALESCE(SUM(LENGTH(update_bytes)), 0) FROM richtext_updates WHERE entity_id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(db_err)?;
        if n <= COMPACT_UPDATES && bytes <= COMPACT_BYTES {
            return Ok(());
        }
        let Some(state) = self.cache.borrow().docs.get(id).cloned() else { return Ok(()) };
        let snapshot = richtext::export_snapshot(&state.doc)?;
        let tx = self.doc.unchecked_transaction().map_err(db_err)?;
        tx.execute(
            "UPDATE richtext_states SET snapshot = ?2, snapshot_seq = ?3, engine_version = ?4 WHERE entity_id = ?1",
            params![id, snapshot, seq, richtext::engine_version()],
        )
        .map_err(db_err)?;
        tx.execute("DELETE FROM richtext_updates WHERE entity_id = ?1 AND seq <= ?2", params![id, seq]).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    /// `doc.get_submission`: `accepted` with the original result, or `not_found`
    /// ("this epoch never accepted that key" — safe to resend unchanged).
    pub fn get_submission(&self, caller: &Caller, epoch: &str, idem_key: &str) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        self.check_epoch(epoch)?;
        check_idempotency_key(idem_key)?;
        let row: Option<String> = self
            .doc
            .query_row("SELECT result_json FROM commits WHERE principal = ?1 AND idem_key = ?2", params![caller.principal, idem_key], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        Ok(match row {
            Some(r) => json!({ "ok": true, "status": "accepted", "result": serde_json::from_str::<Value>(&r).unwrap_or(Value::Null) }),
            None => json!({ "ok": true, "status": "not_found" }),
        })
    }

    // ---- undo ----

    /// `doc.undo`: compensate a commit with its stored inverse operations. Redo
    /// is the undo of the compensating commit.
    pub fn undo(&mut self, caller: &Caller, req: &Value) -> Value {
        match self.undo_inner(caller, req) {
            Ok(v) => v,
            Err(e) => CommitFailure::Rejected { error: e, op_index: None }.to_json(),
        }
    }

    fn undo_inner(&mut self, caller: &Caller, req: &Value) -> WsResult<Value> {
        let get = |k: &str| req.get(k).and_then(Value::as_str);
        let target = get("commit_id").ok_or_else(|| WsError::invalid_op("commit_id required"))?;
        let idem = get("idempotency_key").ok_or_else(|| WsError::invalid_op("idempotency_key required"))?;
        self.check_epoch(get("epoch").unwrap_or(""))?;
        let partial = match get("mode").unwrap_or("all_or_nothing") {
            "all_or_nothing" => false,
            "partial" => true,
            _ => return Err(WsError::invalid_op("mode must be all_or_nothing or partial")),
        };
        let access = self.require_ws_any(caller)?;
        let row: Option<(u64, String)> = self
            .doc
            .query_row("SELECT seq, principal FROM commits WHERE commit_id = ?1", [target], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(db_err)?;
        let (seq, author) = row.ok_or_else(|| WsError::not_found("commit not found"))?;
        if author != caller.principal && !access.ws_caps.has(Cap::Manage) {
            return Err(WsError::denied("only the author or a manager may undo this commit"));
        }
        let mut st = self.doc.prepare("SELECT op_index, inverse_json FROM commit_ops WHERE seq = ?1 ORDER BY op_index DESC").map_err(db_err)?;
        let rows: Vec<(u64, Option<String>)> =
            st.query_map([seq], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        let not_undoable: Vec<u64> = rows.iter().filter(|(_, inv)| inv.is_none()).map(|(i, _)| *i).collect();
        if !not_undoable.is_empty() {
            return Err(WsError::new(Code::NotUndoable, "the commit contains operations only its editor session can undo")
                .with_data(json!({ "op_indexes": not_undoable })));
        }
        // inverse operations of later ops first; each op's own inverse list keeps its order
        let mut inverse: Vec<Value> = Vec::new();
        for (_, inv) in &rows {
            let list: Vec<Value> = serde_json::from_str(inv.as_deref().unwrap_or("[]")).map_err(|e| WsError::io(e.to_string()))?;
            inverse.extend(list);
        }
        if inverse.is_empty() {
            return Err(WsError::new(Code::NotUndoable, "nothing to undo"));
        }
        let (workspace_id, epoch) = (self.workspace_id.clone(), self.epoch.clone());
        let build = |ops: &[Value]| -> Value {
            json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": workspace_id, "epoch": epoch,
                    "idempotency_key": idem, "session_id": req.get("session_id").cloned().unwrap_or(Value::Null),
                    "origin": "human", "message": format!("undo {target}"), "operations": ops })
        };
        let opts = CommitOpts { internal: true, import: false, undoes: Some(target.to_string()) };
        // applicability is the ordinary pipeline: every inverse carries `expect: { rev: S }`
        let dry = self.commit_inner(&build(&inverse), caller, &opts, true);
        let dry = match dry {
            Ok(v) if v["status"] == json!("ok") => None,
            Ok(v) => return Ok(v), // idempotent replay of an already accepted undo
            Err(f) => Some(f),
        };
        let Some(failure) = dry else {
            return Ok(self.commit(&build(&inverse), caller, &opts));
        };
        // which inverse operations still apply on their own?
        let (mut applicable, mut blocked) = (Vec::new(), Vec::new());
        for op in &inverse {
            let mut probe = build(std::slice::from_ref(op));
            probe["idempotency_key"] = json!(format!("{idem}#probe"));
            match self.commit_inner(&probe, caller, &opts, true) {
                Ok(_) => applicable.push(op.clone()),
                Err(f) => blocked.push(json!({ "operation": op, "reason": f.to_json() })),
            }
        }
        let plan_digest = sha256_hex(canonical_json(&json!(applicable))?.as_bytes());
        if partial {
            if get("plan_digest") != Some(plan_digest.as_str()) {
                return Err(WsError::invalid_op("plan_digest missing or stale; request the undo plan again"));
            }
            if applicable.is_empty() {
                return Err(WsError::new(Code::NotUndoable, "no part of the commit can be undone any more"));
            }
            return Ok(self.commit(&build(&applicable), caller, &opts));
        }
        let mut out = failure.to_json();
        out["undoable"] = json!(applicable);
        out["blocked"] = json!(blocked);
        out["plan_digest"] = json!(plan_digest);
        Ok(out)
    }

    // ---- change stream ----

    /// `doc.get_changes`: the only authoritative propagation mechanism. Entries
    /// are filtered by the caller's *current* read permission; a fully filtered
    /// commit still yields an empty event so `seq` stays contiguous.
    pub fn get_changes(&self, caller: &Caller, epoch: &str, after_seq: u64, limit: usize, with_ops: bool) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        self.check_epoch(epoch)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let limit = limit.clamp(1, 500);
        let mut st = self
            .doc
            .prepare_cached(
                "SELECT seq, commit_id, principal, origin, run_id, undoes, idem_key, accepted_at, undo_group FROM commits \
                 WHERE seq > ?1 ORDER BY seq LIMIT ?2",
            )
            .map_err(db_err)?;
        #[allow(clippy::type_complexity)]
        let commits: Vec<(u64, String, String, String, Option<String>, Option<String>, String, String, Option<String>)> = st
            .query_map(params![after_seq, limit as i64 + 1], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        let mut more = commits.len() > limit;
        let mut readable: BTreeMap<String, bool> = BTreeMap::new();
        let mut can_read = |id: &str| -> WsResult<bool> {
            if let Some(b) = readable.get(id) {
                return Ok(*b);
            }
            let ok = match ctx.entity(id)? {
                Some(e) => access.can_read(&ctx, &e)?,
                None => false,
            };
            readable.insert(id.to_string(), ok);
            Ok(ok)
        };
        let mut ops_st = self.doc.prepare_cached("SELECT op_json, touched_json FROM commit_ops WHERE seq = ?1 ORDER BY op_index").map_err(db_err)?;
        let (mut events, mut bytes) = (Vec::new(), 0usize);
        for (seq, commit_id, principal, origin, run_id, undoes, idem_key, accepted_at, undo_group) in commits.into_iter().take(limit) {
            let rows: Vec<(String, String)> =
                ops_st.query_map([seq], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
            let (mut touched_out, mut ops_out) = (Vec::new(), Vec::new());
            for (op_json, touched_json) in rows {
                let touched: Vec<Value> = serde_json::from_str(&touched_json).map_err(|e| WsError::io(e.to_string()))?;
                let op: Value = serde_json::from_str(&op_json).map_err(|e| WsError::io(e.to_string()))?;
                let primary = op.get("entity_id").or_else(|| op.get("source_id")).and_then(Value::as_str).unwrap_or("").to_string();
                let mut any = false;
                for t in touched {
                    if can_read(t["entity_id"].as_str().unwrap_or(""))? {
                        touched_out.push(t);
                        any = true;
                    }
                }
                if with_ops && any && can_read(&primary)? {
                    bytes += op_json.len();
                    ops_out.push(op);
                }
            }
            let mut ev = json!({ "seq": seq });
            if !touched_out.is_empty() {
                ev = json!({ "seq": seq, "commit_id": commit_id, "origin": origin, "run_id": run_id, "undoes": undoes,
                             "undo_group": undo_group, "author": principal, "accepted_at": accepted_at, "touched": touched_out });
                if with_ops {
                    ev["ops"] = json!(ops_out);
                }
            }
            if principal == caller.principal {
                // the idempotency key is scoped to its author and meaningless (and private) to anyone else
                ev["idempotency_key"] = json!(idem_key);
                ev["commit_id"] = json!(commit_id);
            }
            events.push(ev);
            if bytes > MAX_REQUEST_BYTES {
                more = true;
                break;
            }
        }
        if let Some(last) = events.last().and_then(|e| e["seq"].as_u64()) {
            more = more || last < self.head_seq;
        }
        Ok(json!({ "ok": true, "epoch": self.epoch, "head_seq": self.head_seq, "changes": events, "more": more }))
    }

    // ---- write locks (design doc §2.11) ----

    fn lock_event(&self, entity: &str, event: &str, principal: &str, session: Option<&str>, by: Option<&str>, now: &str) {
        let _ = self.local.execute(
            "INSERT INTO lock_events (entity_id, event, principal, session_id, by_principal, at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![entity, event, principal, session, by, now],
        );
    }

    fn lock_row(&self, entity: &str) -> WsResult<Option<(String, String, String, String, String, Option<String>)>> {
        self.local
            .query_row(
                "SELECT lock_id, principal, session_id, acquired_at, expires_at, last_write_at FROM locks WHERE entity_id = ?1",
                [entity],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()
            .map_err(db_err)
    }

    /// Checked in the same serial writer as the write itself, so there is no
    /// window between "holds the lock" and "wrote".
    fn check_locks(&self, targets: &std::collections::BTreeSet<String>, caller: &Caller, session: Option<&str>, now: &str) -> WsResult<()> {
        let mut problems = Vec::new();
        let mut worst: Option<Code> = None;
        for id in targets {
            let row = self.lock_row(id)?;
            let mine = |p: &str, s: &str| p == caller.principal && Some(s) == session;
            let code = match &row {
                Some((_, p, s, _, expires, _)) if expires.as_str() > now && mine(p, s) => continue,
                Some((_, p, s, acquired, expires, _)) if expires.as_str() > now => {
                    problems.push(json!({ "entity_id": id, "code": "LOCK_HELD", "holder": p, "session_id": s, "acquired_at": acquired, "expires_at": expires }));
                    if self.believed_holder(id, caller, session)? { Code::LockLost } else { Code::LockHeld }
                }
                _ => {
                    if self.believed_holder(id, caller, session)? { Code::LockLost } else { Code::LockRequired }
                }
            };
            if !matches!(problems.last(), Some(p) if p["entity_id"] == json!(id)) {
                problems.push(json!({ "entity_id": id, "code": code.as_str() }));
            } else if let Some(p) = problems.last_mut() {
                p["code"] = json!(code.as_str());
            }
            worst = Some(match (worst, code) {
                (Some(Code::LockLost), _) | (_, Code::LockLost) => Code::LockLost,
                (Some(Code::LockHeld), _) | (_, Code::LockHeld) => Code::LockHeld,
                _ => Code::LockRequired,
            });
        }
        match worst {
            None => Ok(()),
            Some(code) => Err(WsError::new(code, "write lock check failed").with_data(json!({ "locks": problems }))),
        }
    }

    /// The caller's session acquired this lock and never released it itself.
    fn believed_holder(&self, entity: &str, caller: &Caller, session: Option<&str>) -> WsResult<bool> {
        let last: Option<String> = self
            .local
            .query_row(
                "SELECT event FROM lock_events WHERE entity_id = ?1 AND principal = ?2 AND session_id IS ?3 ORDER BY id DESC LIMIT 1",
                params![entity, caller.principal, session],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        Ok(matches!(last.as_deref(), Some("acquired") | Some("taken_over") | Some("broken") | Some("idle_released")))
    }

    pub fn lock_acquire(&mut self, caller: &Caller, entity_ids: &[String], session_id: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        check_id("session_id", session_id)?;
        let now = self.now();
        let expires = format_utc_ms((self.clock)() + LOCK_LEASE_MS);
        let mut plan = Vec::new();
        {
            let ctx = self.ctx(&now);
            let mut held = Vec::new();
            for id in entity_ids {
                let e = aiworkspace_core::read::readable_entity(&ctx, &access, id)?;
                if !e.alive() {
                    return Err(WsError::deleted(format!("entity {id} is deleted")));
                }
                if !access.caps(&ctx, id)?.any_write() {
                    return Err(WsError::denied("a write capability is required to take a lock"));
                }
                if e.write_policy != POLICY_LOCK {
                    plan.push((id.clone(), None)); // locking an open entity is a no-op
                    continue;
                }
                match self.lock_row(id)? {
                    Some((lock_id, p, s, _, exp, _)) if exp.as_str() > now.as_str() => {
                        if p == caller.principal && s == session_id {
                            plan.push((id.clone(), Some((lock_id, false, None))));
                        } else {
                            held.push(json!({ "entity_id": id, "holder": p, "session_id": s, "expires_at": exp }));
                        }
                    }
                    Some((_, p, s, ..)) => plan.push((id.clone(), Some((random_id("lk_"), true, Some((p, s)))))),
                    None => plan.push((id.clone(), Some((random_id("lk_"), true, None)))),
                }
            }
            if !held.is_empty() {
                // all or nothing
                return Err(WsError::new(Code::LockHeld, "lock is held by another session").with_data(json!({ "locks": held })));
            }
        }
        let mut out = Vec::new();
        for (id, lock) in plan {
            match lock {
                None => out.push(json!({ "entity_id": id, "lock_id": null, "write_policy": POLICY_OPEN })),
                Some((lock_id, fresh, previous)) => {
                    if fresh {
                        if let Some((p, s)) = previous {
                            self.lock_event(&id, "taken_over", &p, Some(&s), Some(&caller.principal), &now);
                        }
                        self.local
                            .execute(
                                "INSERT OR REPLACE INTO locks (entity_id, lock_id, principal, session_id, acquired_at, expires_at, last_write_at) \
                                 VALUES (?1,?2,?3,?4,?5,?6,NULL)",
                                params![id, lock_id, caller.principal, session_id, now, expires],
                            )
                            .map_err(db_err)?;
                        self.lock_event(&id, "acquired", &caller.principal, Some(session_id), None, &now);
                    } else {
                        self.local.execute("UPDATE locks SET expires_at = ?1 WHERE lock_id = ?2", params![expires, lock_id]).map_err(db_err)?;
                    }
                    out.push(json!({ "entity_id": id, "lock_id": lock_id, "expires_at": expires }));
                }
            }
        }
        self.locks_changed();
        Ok(json!({ "ok": true, "locks": out }))
    }

    fn locks_changed(&self) {
        if let Some(cb) = &self.on_lock_change {
            cb();
        }
    }

    pub fn lock_renew(&mut self, caller: &Caller, lock_ids: &[String]) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let now_ms = (self.clock)();
        let now = format_utc_ms(now_ms);
        let expires = format_utc_ms(now_ms + LOCK_LEASE_MS);
        let idle_before = format_utc_ms(now_ms - LOCK_IDLE_MS);
        let (mut out, mut lost) = (Vec::new(), Vec::new());
        for lock_id in lock_ids {
            let row: Option<(String, String, String, String, String, Option<String>)> = self
                .local
                .query_row(
                    "SELECT entity_id, principal, session_id, acquired_at, expires_at, last_write_at FROM locks WHERE lock_id = ?1",
                    [lock_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .optional()
                .map_err(db_err)?;
            match row {
                Some((entity, p, s, acquired, exp, last_write)) if p == caller.principal && exp > now => {
                    // idle release: nobody may hold a lock for long without writing
                    if last_write.unwrap_or(acquired) < idle_before {
                        self.local.execute("DELETE FROM locks WHERE lock_id = ?1", [lock_id]).map_err(db_err)?;
                        self.lock_event(&entity, "idle_released", &p, Some(&s), None, &now);
                        lost.push(json!({ "lock_id": lock_id, "entity_id": entity, "reason": "idle" }));
                    } else {
                        self.local.execute("UPDATE locks SET expires_at = ?1 WHERE lock_id = ?2", params![expires, lock_id]).map_err(db_err)?;
                        out.push(json!({ "lock_id": lock_id, "entity_id": entity, "expires_at": expires }));
                    }
                }
                _ => lost.push(json!({ "lock_id": lock_id, "reason": "expired_or_released" })),
            }
        }
        if !lost.is_empty() {
            self.locks_changed();
            return Err(WsError::new(Code::LockLost, "lease is gone").with_data(json!({ "lost": lost, "renewed": out })));
        }
        Ok(json!({ "ok": true, "locks": out }))
    }

    pub fn lock_release(&mut self, caller: &Caller, lock_ids: &[String]) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let now = self.now();
        for lock_id in lock_ids {
            let row: Option<(String, String)> = self
                .local
                .query_row("SELECT entity_id, session_id FROM locks WHERE lock_id = ?1 AND principal = ?2", params![lock_id, caller.principal], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()
                .map_err(db_err)?;
            if let Some((entity, session)) = row {
                self.local.execute("DELETE FROM locks WHERE lock_id = ?1", [lock_id]).map_err(db_err)?;
                self.lock_event(&entity, "released", &caller.principal, Some(&session), None, &now);
            }
        }
        self.locks_changed();
        Ok(json!({ "ok": true }))
    }

    pub fn lock_break(&mut self, caller: &Caller, entity_id: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        if !access.can(&self.ctx(&now), entity_id, Cap::Manage)? {
            return Err(WsError::denied("manage capability required"));
        }
        if let Some((_, p, s, ..)) = self.lock_row(entity_id)? {
            self.local.execute("DELETE FROM locks WHERE entity_id = ?1", [entity_id]).map_err(db_err)?;
            self.lock_event(entity_id, "broken", &p, Some(&s), Some(&caller.principal), &now);
        }
        self.locks_changed();
        Ok(json!({ "ok": true }))
    }

    /// Currently valid locks on entities the caller may read.
    pub fn lock_list(&self, caller: &Caller, entity_ids: Option<&[String]>) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut st = self
            .local
            .prepare("SELECT entity_id, lock_id, principal, session_id, acquired_at, expires_at FROM locks WHERE expires_at > ?1 ORDER BY entity_id")
            .map_err(db_err)?;
        let rows: Vec<(String, String, String, String, String, String)> = st
            .query_map([&now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        let mut out = Vec::new();
        for (entity, lock_id, principal, session, acquired, expires) in rows {
            if entity_ids.is_some_and(|ids| !ids.contains(&entity)) || !access.can(&ctx, &entity, Cap::Read)? {
                continue;
            }
            let mut l = json!({ "entity_id": entity, "principal": principal, "session_id": session, "acquired_at": acquired, "expires_at": expires });
            if principal == caller.principal {
                l["lock_id"] = json!(lock_id);
            }
            out.push(l);
        }
        Ok(json!({ "ok": true, "locks": out }))
    }

    pub(crate) fn lock_holder(&self, entity: &str, now: &str) -> Option<Value> {
        match self.lock_row(entity) {
            Ok(Some((_, p, s, acquired, expires, _))) if expires.as_str() > now => {
                Some(json!({ "principal": p, "session_id": s, "acquired_at": acquired, "expires_at": expires }))
            }
            _ => None,
        }
    }
}
