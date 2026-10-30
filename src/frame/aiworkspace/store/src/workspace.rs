//! One Workspace folder: `doc.sqlite` (the portable document) + `local.sqlite`
//! (grants, locks, runs — this deployment only). A `Workspace` value is the
//! single writer of its folder: every Commit is planned, checked and applied
//! serially through it (design doc §2.5.2, §4.3).

use crate::docdb::{apply_changes, db_err, meta_get, meta_set, DocCache, SqlCtx, WriteStats};
use crate::objects::{bucket_id, ObjectStore, ASSETS, OBJECTS};
use crate::schema::{DOC_DDL, LOCAL_DDL, STORAGE_SCHEMA_VERSION};
use aiworkspace_core::access::{Access, Cap, CapSet};
use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use aiworkspace_core::id::{check_id, check_idempotency_key, is_prefixed_id, prefixed_id};
use aiworkspace_core::model::*;
use aiworkspace_core::plan::{plan_commit, CommitFailure, CommitRequest, Limits, PlanEnv, PlannedOp, Touched};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{richtext, Code, Deny, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Wish blocks use the workspace owner's AICC (default).
pub const WISH_AICC_WORKSPACE_OWNER: &str = "workspace_owner";
/// Wish blocks use the AICC of the user who starts the run.
pub const WISH_AICC_CURRENT_USER: &str = "current_user";

pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const LOCK_LEASE_MS: i64 = 60_000;
pub const LOCK_IDLE_MS: i64 = 600_000;
pub const COMPACT_UPDATES: i64 = 200;
pub const COMPACT_BYTES: i64 = 1024 * 1024;

/// What kind of caller the service verified (hybrid SSO contract §2.1, PRE-03). Grant matching and the right to
/// own Workspaces follow the class, never the shape of the principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerClass {
    /// A local account of the Zone; the principal is its user id. `restricted`: a Limited account or a session
    /// with a ceiling. `did` and `groups` are filled by the service when a grant on the Workspace needs them.
    Account { restricted: bool, did: Option<String>, groups: Vec<String> },
    /// An authenticated DID without an account in this Zone; the principal is the DID.
    External { did: String },
    /// A link's visitor without a session (`guest:` principal).
    Guest,
    /// App, agent, system and device principals.
    Service,
}

/// The identity the service runtime verified. Nothing a client writes into a
/// request body is ever used as identity.
#[derive(Debug, Clone)]
pub struct Caller {
    pub principal: String,
    pub app_id: Option<String>,
    pub class: CallerClass,
    /// The service principal's kind as the token names it (`agent`, `app`, `system`, `device`); `None` for
    /// accounts, Externals and guests. Decides the `agent` origin of a Commit (D08). `run` is a run token's
    /// holder (M3): the run's execution identity, limited to that run.
    pub kind: Option<String>,
    /// The run a run token speaks for.
    pub run_scope: Option<String>,
    /// How the session was established (`password`, `exclusive_invite`, `did_federation`, `assertion`).
    pub auth_method: Option<String>,
    /// `contact:<owner_did>#<tag>` subjects the service confirmed the caller belongs to.
    pub contacts: BTreeSet<String>,
    /// `group:` / `contact:` subjects whose membership could not be checked: a decision depending on one is
    /// refused with `authorization_unavailable`, never taken from a stale answer.
    pub unconfirmed: BTreeSet<String>,
}

/// Grant subject that applies to everyone, signed in or not (a public link).
pub const PUBLIC_SUBJECT: &str = "public";
/// Grant subject that applies to every unrestricted account of the Zone.
pub const ANY_AUTHENTICATED_SUBJECT: &str = "*";
pub const GUEST_PREFIX: &str = "guest:";
pub const GUEST_TOKEN_PREFIX: &str = "gt";
/// `group:<id>`: the members of a system user group.
pub const GROUP_PREFIX: &str = "group:";
/// `contact:<owner_did>#<tag>`: the DIDs in an owner's contact group (projections included).
pub const CONTACT_PREFIX: &str = "contact:";

impl Caller {
    fn with_class(principal: &str, class: CallerClass) -> Caller {
        Caller { principal: principal.to_string(), app_id: None, class, kind: None, run_scope: None, auth_method: None, contacts: BTreeSet::new(), unconfirmed: BTreeSet::new() }
    }
    /// The holder of a run token (M3): acts as the run's principal, for that run only.
    pub fn run_token(principal: &str, run_id: &str) -> Caller {
        Caller { kind: Some("run".into()), run_scope: Some(run_id.to_string()), ..Self::with_class(principal, CallerClass::Service) }
    }
    /// An Agent principal (its own DID): its Commits carry origin `agent`.
    pub fn agent(principal: &str) -> Caller {
        Caller { kind: Some("agent".into()), ..Self::with_class(principal, CallerClass::Service) }
    }
    pub fn is_agent(&self) -> bool {
        self.kind.as_deref() == Some("agent")
    }
    /// An unrestricted account (Admin / User).
    pub fn user(principal: &str) -> Caller {
        Self::account(principal, false)
    }
    pub fn account(principal: &str, restricted: bool) -> Caller {
        Self::with_class(principal, CallerClass::Account { restricted, did: None, groups: Vec::new() })
    }
    pub fn external(did: &str) -> Caller {
        Self::with_class(did, CallerClass::External { did: did.to_string() })
    }
    pub fn service(principal: &str) -> Caller {
        Self::with_class(principal, CallerClass::Service)
    }
    /// An unauthenticated visitor: the principal is derived from a bearer token the browser made up
    /// (`name_lib::bearer_token` form, prefix `gt`) and keeps, so the same browser stays the same guest
    /// (its own annotations stay "mine") without the token ever being stored or shown.
    pub fn guest(token: &str) -> Option<Caller> {
        if !name_lib::bearer_token::has_form(GUEST_TOKEN_PREFIX, token) {
            return None;
        }
        let digest = name_lib::bearer_token::digest_bytes(token);
        Some(Self::with_class(&prefixed_id(GUEST_PREFIX, digest[..17].try_into().unwrap()), CallerClass::Guest))
    }
    pub fn is_guest(&self) -> bool {
        self.class == CallerClass::Guest
    }
    /// Limited accounts, External DIDs and guests: they never match `*`, own nothing, and a missing
    /// Workspace looks like one that was not shared with them.
    pub fn is_restricted(&self) -> bool {
        match &self.class {
            CallerClass::Account { restricted, .. } => *restricted,
            CallerClass::External { .. } | CallerClass::Guest => true,
            CallerClass::Service => false,
        }
    }
    /// Creates, imports and forks Workspaces and lists the Zone's principals: unrestricted accounts and services.
    pub fn may_own(&self) -> bool {
        matches!(self.class, CallerClass::Account { restricted: false, .. } | CallerClass::Service)
    }
    /// The same caller before the service settled its group and contact memberships (they are settled again).
    pub fn without_memberships(&self) -> Caller {
        let mut c = self.clone();
        c.contacts.clear();
        c.unconfirmed.clear();
        if let CallerClass::Account { groups, .. } = &mut c.class {
            groups.clear();
        }
        c
    }
    /// The DID contact grants are checked against, once known.
    pub fn did(&self) -> Option<&str> {
        match &self.class {
            CallerClass::Account { did, .. } => did.as_deref(),
            CallerClass::External { did } => Some(did),
            _ => None,
        }
    }
    /// Whether a grant naming `subject` applies: the exact principal, `public` (when `public` counts), `*` for
    /// unrestricted accounts only, the caller's system groups, confirmed contact groups.
    pub fn matches(&self, subject: &str, public: bool) -> bool {
        if subject == self.principal {
            return true;
        }
        if subject == PUBLIC_SUBJECT {
            return public;
        }
        if let CallerClass::Account { restricted, groups, .. } = &self.class {
            if subject == ANY_AUTHENTICATED_SUBJECT {
                return !restricted;
            }
            if let Some(group) = subject.strip_prefix(GROUP_PREFIX) {
                return groups.iter().any(|g| g == group);
            }
        }
        subject.starts_with(CONTACT_PREFIX) && self.contacts.contains(subject)
    }
}

pub fn is_guest_principal(principal: &str) -> bool {
    principal.starts_with(GUEST_PREFIX)
}

/// A grant subject: a principal, `public`, `*`, `group:<id>` or `contact:<owner_did>#<tag>`. A contact group can only
/// be named by its owner (`granter` is the caller's DID): nobody probes someone else's private groups through grants.
fn check_subject(subject: &str, granter: Option<&str>) -> WsResult<()> {
    let bad = |what: &str| Err(WsError::invalid_op(format!("invalid subject: {what}")));
    if subject.is_empty() || subject.len() > 256 || subject.chars().any(char::is_whitespace) {
        return bad("1–256 characters without spaces");
    }
    if let Some(group) = subject.strip_prefix(GROUP_PREFIX) {
        if group.is_empty() {
            return bad("group:<id>");
        }
    }
    if let Some(contact) = subject.strip_prefix(CONTACT_PREFIX) {
        match contact.split_once('#') {
            Some((owner, tag)) if owner.starts_with("did:") && !tag.is_empty() => {
                if granter != Some(owner) {
                    return bad("a contact group grant names one of the caller's own groups (contact:<your did>#<tag>)");
                }
            }
            _ => return bad("contact:<owner_did>#<tag>"),
        }
    }
    Ok(())
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
    /// The identity the Commit is written as (D08): a run's result is written as the run's execution
    /// identity, whoever applies it, so every apply of the same run meets the same idempotency record.
    /// Authorization is still the caller's.
    pub as_principal: Option<String>,
    /// The maintenance channel (D19): accepted over quota.
    pub maintenance: bool,
    /// A Source refresh (D12): the Commit's origin is `source`, the only path that writes sourced content.
    pub as_source: bool,
    /// An extension build or import (扩展控件 P0): the Commit's origin is `build`, the only one that may write a
    /// definition's `derived.build` record.
    pub as_build: bool,
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
    pub objects: Arc<ObjectStore>,
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
    /// Show locks of the service (third phase §8.1): a presented Workspace accepts no document writes.
    pub show_locks: Option<crate::show::ShowLocks>,
    /// Logical bytes this Workspace may occupy (D19); `None` = no quota.
    pub quota: Option<u64>,
    /// Bytes this Workspace's Sources may fetch (D24); `None` = unlimited.
    pub traffic_quota: Option<u64>,
    /// Retention period of history-only resources, in milliseconds (D06).
    pub history_retention_ms: i64,
}

/// Default retention of resources only the history references: 30 days (D06).
pub const HISTORY_RETENTION_MS: i64 = 30 * 24 * 3_600_000;

fn open_db(path: &Path, synchronous_full: bool) -> WsResult<Connection> {
    let conn = Connection::open(path).map_err(db_err)?;
    conn.pragma_update(None, "journal_mode", "WAL").map_err(db_err)?;
    // an answered commit must survive power loss
    conn.pragma_update(None, "synchronous", if synchronous_full { "FULL" } else { "NORMAL" }).map_err(db_err)?;
    conn.pragma_update(None, "foreign_keys", "ON").map_err(db_err)?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_err)?;
    Ok(conn)
}

/// The origin of a Commit as the server reads it (内核架构改进 D08): internal compensations are `system`,
/// packages `import`, a run's result `program`, an Agent's own writes `agent`, a Source refresh `source`, an extension build `build`, everything else `human`
/// (SDK scripts with a user token included).
pub fn derive_origin(caller: &Caller, opts: &CommitOpts, has_run: bool) -> &'static str {
    if opts.import {
        "import"
    } else if opts.internal {
        "system"
    } else if opts.as_source {
        "source"
    } else if opts.as_build {
        "build"
    } else if has_run {
        "program"
    } else if caller.is_agent() {
        "agent"
    } else {
        "human"
    }
}

/// Keep only the rows of entities `can_read` admits (the replica bootstrap applies the same rule).
fn filter_rows(rows: &mut Value, can_read: &mut dyn FnMut(&str) -> WsResult<bool>) -> WsResult<()> {
    let keep = |table: &str, key: &str, rows: &mut Value, can_read: &mut dyn FnMut(&str) -> WsResult<bool>| -> WsResult<()> {
        if let Some(a) = rows.get_mut(table).and_then(Value::as_array_mut) {
            let mut out = Vec::with_capacity(a.len());
            for r in a.drain(..) {
                if can_read(r[key].as_str().unwrap_or(""))? {
                    out.push(r);
                }
            }
            *a = out;
        }
        Ok(())
    };
    keep("entities", "entity_id", rows, can_read)?;
    keep("tree_edges", "child_id", rows, can_read)?;
    keep("table_fields", "source_id", rows, can_read)?;
    keep("table_records", "source_id", rows, can_read)?;
    keep("richtext_states", "entity_id", rows, can_read)?;
    keep("richtext_updates", "entity_id", rows, can_read)?;
    keep("refs", "src_entity_id", rows, can_read)?;
    keep("refs_deleted", "src_entity_id", rows, can_read)?;
    Ok(())
}

fn add_usage(conn: &Connection, category: &str, bytes: i64) -> WsResult<()> {
    if bytes == 0 {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO usage (category, bytes) VALUES (?1, ?2) ON CONFLICT(category) DO UPDATE SET bytes = MAX(0, bytes + excluded.bytes)",
        params![category, bytes],
    )
    .map_err(db_err)?;
    Ok(())
}

/// `(structured bytes delta, new asset bytes)` of applying `changes` over what the transaction sees now.
fn usage_delta(tx: &Connection, changes: &Changes) -> WsResult<(i64, i64)> {
    let mut delta: i64 = 0;
    for (id, e) in &changes.entities {
        let old: i64 = tx.query_row("SELECT LENGTH(payload_json) + LENGTH(key_revs_json) FROM entities WHERE entity_id = ?1", [id], |r| r.get(0)).optional().map_err(db_err)?.unwrap_or(0);
        delta += (serde_json::to_string(&e.payload).map(|s| s.len()).unwrap_or(0) + serde_json::to_string(&e.key_revs).map(|s| s.len()).unwrap_or(0)) as i64 - old;
    }
    for ((s, r), rec) in &changes.records {
        let old: i64 = tx.query_row("SELECT LENGTH(values_json) + LENGTH(meta_json) FROM table_records WHERE source_id = ?1 AND record_id = ?2", params![s, r], |x| x.get(0)).optional().map_err(db_err)?.unwrap_or(0);
        delta += (serde_json::to_string(&rec.values).map(|s| s.len()).unwrap_or(0) + serde_json::to_string(&rec.meta).map(|s| s.len()).unwrap_or(0)) as i64 - old;
    }
    for ((s, f), field) in &changes.fields {
        let old: i64 = tx.query_row("SELECT LENGTH(def_json) FROM table_fields WHERE source_id = ?1 AND field_id = ?2", params![s, f], |x| x.get(0)).optional().map_err(db_err)?.unwrap_or(0);
        delta += serde_json::to_string(&field.def).map(|s| s.len()).unwrap_or(0) as i64 - old;
    }
    for (id, bytes) in &changes.rt_updates {
        let _ = id;
        delta += bytes.len() as i64;
    }
    let mut assets: i64 = 0;
    for (id, a) in &changes.assets {
        let known: bool = tx.query_row("SELECT 1 FROM assets WHERE object_id = ?1", [id], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
        if !known {
            assets += a.size as i64;
        }
    }
    Ok((delta, assets))
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
    pub fn create(dir: &Path, workspace_id: &str, title: &str, owner: &str, objects: Arc<ObjectStore>, clock: Clock) -> WsResult<Workspace> {
        if !is_prefixed_id("ws_", workspace_id) {
            return Err(WsError::invalid_op("workspace_id must be ws_ + 26 base32 chars"));
        }
        std::fs::create_dir_all(dir).map_err(|e| WsError::io(e.to_string()))?;
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
            ("base_seq", "0".to_string()),
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

    pub fn open(dir: &Path, objects: Arc<ObjectStore>, clock: Clock) -> WsResult<Workspace> {
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
            show_locks: None,
            quota: None,
            traffic_quota: None,
            history_retention_ms: HISTORY_RETENTION_MS,
        };
        Ok(ws)
    }

    /// Admission under the quota (D19): refused when `needed` more bytes would pass it. `QUOTA_EXCEEDED`
    /// (retryable) when the maintenance channel can release enough — exports, clones, staged uploads and
    /// reclaimable history — `QUOTA_LIMIT` otherwise. Maintenance writes are never refused.
    pub fn check_quota(&self, needed: u64, maintenance: bool) -> WsResult<()> {
        let Some(quota) = self.quota else { return Ok(()) };
        if maintenance {
            return Ok(());
        }
        let usage = self.usage()?;
        let used = usage["total"].as_i64().unwrap_or(0).max(0) as u64;
        if used + needed <= quota {
            return Ok(());
        }
        let reclaimable: u64 = ["clones", "staged"].iter().map(|c| usage[c].as_i64().unwrap_or(0).max(0) as u64).sum();
        let over = used + needed - quota;
        let data = json!({ "needed_bytes": over, "reclaimable_bytes": reclaimable, "quota_bytes": quota, "used_bytes": used, "owner": self.owner()?.unwrap_or_default() });
        if reclaimable >= over {
            Err(WsError::new(Code::QuotaExceeded, format!("the workspace is {over} bytes over its quota; release space and retry")).with_data(data))
        } else {
            Err(WsError::new(Code::QuotaLimit, format!("the workspace is {over} bytes over its quota and nothing it holds can be released; raise the quota")).with_data(data))
        }
    }

    /// `ws.delete_checkpoint { snapshot_id }` (maintenance, `manage`): the snapshot row goes and its retention
    /// set is unbound; the named store reclaims what no other binding holds.
    pub fn delete_checkpoint(&mut self, caller: &Caller, snapshot_id: &str) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let released = self.release_snapshot(snapshot_id)?;
        let n = self.doc.execute("DELETE FROM snapshots WHERE snapshot_id = ?1", [snapshot_id]).map_err(db_err)?;
        Ok(json!({ "ok": true, "removed": n > 0, "released": released }))
    }

    /// `ws.clear_staged` (maintenance, `manage`): staged uploads nobody referenced go, expired or not.
    pub fn clear_staged(&mut self, caller: &Caller) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let (removed, bytes) = self.drop_staged(None)?;
        Ok(json!({ "ok": true, "removed": removed, "bytes": bytes }))
    }

    /// Unbind and forget the staged uploads no asset row took over (all, or those expired by `now`).
    fn drop_staged(&self, expired_by: Option<&str>) -> WsResult<(u64, i64)> {
        let mut st = self.local.prepare("SELECT object_id, size, bucket_key, expires_at FROM staged_assets").map_err(db_err)?;
        let rows: Vec<(String, i64, String, String)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        let (mut removed, mut bytes) = (0, 0i64);
        for (id, size, key, expires) in rows {
            if expired_by.is_some_and(|now| expires.as_str() > now) {
                continue;
            }
            let referenced: bool = self.doc.query_row("SELECT 1 FROM assets WHERE object_id = ?1 AND bucket_version IS NOT NULL", [&id], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
            if !referenced && self.own_key(&key) {
                self.objects.unbind(&self.bucket(ASSETS)?, &key)?;
            }
            self.local.execute("DELETE FROM staged_assets WHERE object_id = ?1", [&id]).map_err(db_err)?;
            if !referenced {
                removed += 1;
                bytes += size;
            }
        }
        self.add_usage("staged", -bytes)?;
        Ok((removed, bytes))
    }

    /// Unbind history-only assets: purged ones (`ended_seq = 0`) at once, the others once their last live
    /// reference ended `history_retention_ms` ago (D06). Their rows stay; an undo or restore that needs one answers
    /// `RESOURCE_RECLAIMED`.
    fn release_history(&self, now_ms: i64) -> WsResult<u64> {
        let mut st = self
            .doc
            .prepare("SELECT a.object_id, a.bucket_key, a.ended_seq, c.accepted_at FROM assets a LEFT JOIN commits c ON c.seq = a.ended_seq \
                      WHERE a.ended_seq IS NOT NULL AND a.bucket_version IS NOT NULL")
            .map_err(db_err)?;
        let rows: Vec<(String, Option<String>, i64, Option<String>)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        drop(st);
        let mut released = 0;
        for (id, key, ended, at) in rows {
            let expired = ended == 0 || at.and_then(|a| aiworkspace_core::value::parse_datetime_ms(&a)).is_some_and(|t| now_ms - t >= self.history_retention_ms);
            if !expired {
                continue;
            }
            let staged: bool = self.local.query_row("SELECT 1 FROM staged_assets WHERE object_id = ?1", [&id], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
            if let Some(key) = key.filter(|k| self.own_key(k) && !staged) {
                self.objects.unbind(&self.bucket(ASSETS)?, &key)?;
            }
            self.doc.execute("UPDATE assets SET bucket_version = NULL WHERE object_id = ?1", [&id]).map_err(db_err)?;
            released += 1;
        }
        Ok(released)
    }

    /// The periodic release of what nothing retains any more (the service's sweep): expired staged uploads and
    /// history-only assets past their retention. A show clone holds the source's bindings and releases nothing.
    pub fn release_expired(&mut self) -> WsResult<Value> {
        if self.purpose().is_some() {
            return Ok(json!({ "staged": 0, "history": 0 }));
        }
        let (staged, _) = self.drop_staged(Some(&self.now()))?;
        let history = self.release_history((self.clock)())?;
        Ok(json!({ "staged": staged, "history": history }))
    }

    /// `ws.purge_deleted { entity_ids? }` (maintenance, `manage`): deleted entities give up content and
    /// references for good (`entity.purge`), their resources become reclaimable at once, their undo and
    /// version restore answer `RESOURCE_RECLAIMED`, and their Block state is cleared. Without ids: every
    /// deleted entity.
    pub fn purge_deleted(&mut self, caller: &Caller, entity_ids: Option<&[String]>, session_id: Option<&str>) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let ids: Vec<String> = match entity_ids {
            Some(ids) => ids.to_vec(),
            None => {
                let mut st = self.doc.prepare("SELECT entity_id FROM entities WHERE deleted_seq IS NOT NULL AND payload_json NOT LIKE '%\"#purged\"%' ORDER BY entity_id").map_err(db_err)?;
                let rows: Vec<String> = st.query_map([], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
                rows
            }
        };
        if ids.is_empty() {
            return Ok(json!({ "ok": true, "purged": [] }));
        }
        // the objects they alone referenced, before the purge drops those references
        let mut objects: Vec<String> = Vec::new();
        for id in &ids {
            let mut st = self.doc.prepare_cached("SELECT dst_object_id FROM refs WHERE src_entity_id = ?1 AND dst_object_id <> ''").map_err(db_err)?;
            let rows: Vec<String> = st.query_map([id], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
            objects.extend(rows);
        }
        let ops: Vec<Value> = ids.iter().map(|id| json!({ "op": "entity.purge", "entity_id": id })).collect();
        let req = json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch,
                          "idempotency_key": format!("purge/{}", random_id("")), "session_id": session_id, "message": "purge deleted entities", "operations": ops });
        let opts = CommitOpts { internal: true, import: false, undoes: None, as_principal: None, maintenance: true, as_source: false, as_build: false };
        let r = self.commit(&req, caller, &opts);
        if r["status"] != json!("accepted") {
            return Err(WsError::new(Code::InvalidOperation, "purge was not accepted").with_data(r));
        }
        // their history-only resources are released now, not after the retention period
        for object_id in &objects {
            self.doc.execute("UPDATE assets SET ended_seq = 0 WHERE object_id = ?1 AND ended_seq IS NOT NULL", [object_id]).map_err(db_err)?;
        }
        self.release_history((self.clock)())?;
        self.clear_block_state(&ids)?;
        Ok(json!({ "ok": true, "purged": ids, "commit_id": r["commit_id"] }))
    }

    /// Object ids a set of operations would need that this Workspace once referenced but no longer retains
    /// (released after their retention or purged, D06): such an undo or restore is refused.
    pub fn reclaimed_objects(&self, ops: &[Value]) -> WsResult<Vec<String>> {
        fn walk(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::String(s) if s.starts_with("cyfile:") || s.starts_with("cydir:") || s.starts_with("jobj:") => out.push(s.clone()),
                Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                Value::Object(o) => o.values().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        let mut ids = Vec::new();
        for op in ops {
            walk(op, &mut ids);
            // restoring a deleted entity brings its references back: the objects they name must be there
            if op["op"] == json!("entity.restore") {
                if let Some(id) = op["entity_id"].as_str() {
                    let mut st = self.doc.prepare_cached("SELECT dst_object_id FROM refs WHERE src_entity_id = ?1 AND dst_object_id <> ''").map_err(db_err)?;
                    let rows: Vec<String> = st.query_map([id], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
                    ids.extend(rows);
                }
            }
        }
        ids.sort();
        ids.dedup();
        let mut gone = Vec::new();
        for id in ids {
            let released: bool = self.doc.query_row("SELECT 1 FROM assets WHERE object_id = ?1 AND bucket_version IS NULL", [&id], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
            let staged: bool = self.local.query_row("SELECT 1 FROM staged_assets WHERE object_id = ?1", [&id], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
            if released && !staged {
                gone.push(id);
            }
        }
        Ok(gone)
    }

    /// A show clone judges access by the grants of the Workspace it was cloned from, as they are now (D17):
    /// revoking there ends the show's audience here.
    fn show_source_grants(&self) -> WsResult<Option<Vec<(String, String, u16, Option<String>)>>> {
        let Some(source) = meta_get(&self.doc, "show_source")? else { return Ok(None) };
        let Some(parent) = self.dir.parent() else { return Ok(None) };
        let path = parent.join(&source).join("local.sqlite");
        if !path.exists() {
            return Ok(Some(Vec::new()));
        }
        let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(db_err)?;
        let mut st = conn.prepare("SELECT subject, scope_entity_id, capabilities, via_entry FROM grants ORDER BY subject, scope_entity_id").map_err(db_err)?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        Ok(Some(rows))
    }

    pub fn title(&self) -> String {
        meta_get(&self.doc, "title").ok().flatten().unwrap_or_default()
    }

    // ---- grants ----

    /// Effective grants of a caller: every grant row whose subject applies to it (see [`Caller::matches`]) and the
    /// templates of the active entries open to guests — for every caller, since anyone may come in through them
    /// anonymously (computed now, so closing one takes effect at the next request).
    pub fn access(&self, caller: &Caller) -> WsResult<Access> {
        self.access_with(caller, true)
    }

    fn access_with(&self, caller: &Caller, public: bool) -> WsResult<Access> {
        let mut access = Access { principal: caller.principal.clone(), ws_caps: CapSet::NONE, scoped: BTreeMap::new() };
        let mut add = |scope: String, caps: u16| {
            if scope.is_empty() {
                access.ws_caps = access.ws_caps.union(CapSet(caps));
            } else {
                let e = access.scoped.entry(scope).or_insert(CapSet::NONE);
                *e = e.union(CapSet(caps));
            }
        };
        let mut undecided = false;
        let rows = match self.show_source_grants()? {
            Some(source) => source,
            None => self.grant_rows()?,
        };
        for (subject, scope, caps, _) in rows {
            if caller.matches(&subject, public) {
                add(scope, caps);
            } else if caller.unconfirmed.contains(&subject) {
                undecided = true;
            }
        }
        if public {
            for (scope, caps) in self.guest_entry_grants()? {
                add(scope, caps);
            }
        }
        // an unconfirmed membership could only add something to a caller that does not already hold everything
        if undecided && access.ws_caps != CapSet::ALL {
            return Err(WsError::refused(Code::PermissionDenied, Deny::AuthorizationUnavailable, "a group or contact membership could not be checked; retry later"));
        }
        Ok(access)
    }

    /// What `ws.list` counts: a restricted caller lists only Workspaces granted to it (by name, group, contact or a
    /// joined entry), never those it merely could open through a public link.
    pub fn listing_access(&self, caller: &Caller) -> WsResult<Access> {
        self.access_with(caller, !caller.is_restricted())
    }

    fn grant_rows(&self) -> WsResult<Vec<(String, String, u16, Option<String>)>> {
        let mut st = self.local.prepare_cached("SELECT subject, scope_entity_id, capabilities, via_entry FROM grants ORDER BY subject, scope_entity_id").map_err(db_err)?;
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// Grant subjects whose membership the service settles per request: `group:` and `contact:`.
    pub fn dynamic_subjects(&self) -> WsResult<Vec<String>> {
        let mut st = self
            .local
            .prepare_cached("SELECT DISTINCT subject FROM grants WHERE subject LIKE 'group:%' OR subject LIKE 'contact:%' ORDER BY subject")
            .map_err(db_err)?;
        let rows = st.query_map([], |r| r.get(0)).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        Ok(rows)
    }

    pub fn owner(&self) -> WsResult<Option<String>> {
        self.local.query_row("SELECT value FROM local_meta WHERE key = 'owner'", [], |r| r.get(0)).optional().map_err(db_err)
    }

    /// Whose AICC the wish blocks use (AI Center Quota PRD §3.3): `workspace_owner` (default) or
    /// `current_user`.
    pub fn wish_aicc_mode(&self) -> WsResult<String> {
        let mode: Option<String> = self.local.query_row("SELECT value FROM local_meta WHERE key = 'wish_aicc_mode'", [], |r| r.get(0)).optional().map_err(db_err)?;
        Ok(mode.unwrap_or_else(|| WISH_AICC_WORKSPACE_OWNER.to_string()))
    }

    /// `ws.set_wish_aicc`: the owner or a manager chooses; only runs accepted afterwards follow it.
    pub fn set_wish_aicc_mode(&mut self, caller: &Caller, mode: &str) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        if mode != WISH_AICC_WORKSPACE_OWNER && mode != WISH_AICC_CURRENT_USER {
            return Err(WsError::invalid_op("mode must be workspace_owner or current_user"));
        }
        self.local.execute("INSERT OR REPLACE INTO local_meta (key, value) VALUES ('wish_aicc_mode', ?1)", [mode]).map_err(db_err)?;
        Ok(json!({ "ok": true, "wish_aicc_mode": mode }))
    }

    /// The Owner's hosted execution configuration (D22, T3-7): `{ enabled, agent_id, allowed_wishes?: [ids],
    /// triggerers?: [subjects], limits?: { max_runs_per_day?, max_tool_iterations? } }`; `digest` is what a run
    /// binds to (D16: a changed configuration makes older delegations `DELEGATION_STALE`).
    pub fn hosted_exec(&self) -> WsResult<Value> {
        let raw: Option<String> = self.local.query_row("SELECT value FROM local_meta WHERE key = 'hosted_exec'", [], |r| r.get(0)).optional().map_err(db_err)?;
        let mut v = raw.and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or_else(|| json!({ "enabled": false }));
        let digest = sha256_hex(aiworkspace_core::canonical::canonical_json(&v).unwrap_or_default().as_bytes())[..32].to_string();
        v["digest"] = json!(digest);
        Ok(v)
    }

    /// `ws.set_hosted_exec` (`manage`): the Owner's configuration; runs accepted afterwards bind to its digest.
    pub fn set_hosted_exec(&mut self, caller: &Caller, config: &Value) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        let o = config.as_object().ok_or_else(|| WsError::invalid_op("config must be an object"))?;
        for k in o.keys() {
            if !matches!(k.as_str(), "enabled" | "agent_id" | "allowed_wishes" | "triggerers" | "limits") {
                return Err(WsError::invalid_op(format!("unknown hosted_exec key {k}")));
            }
        }
        let enabled = o.get("enabled").and_then(Value::as_bool).unwrap_or(false);
        if enabled && !o.get("agent_id").and_then(Value::as_str).is_some_and(|a| !a.is_empty() && a.len() <= 256) {
            return Err(WsError::invalid_op("hosted execution needs the Owner's agent_id"));
        }
        for k in ["allowed_wishes", "triggerers"] {
            if o.get(k).is_some_and(|v| !v.is_null() && !v.as_array().is_some_and(|a| a.iter().all(Value::is_string))) {
                return Err(WsError::invalid_op(format!("{k} must be an array of strings")));
            }
        }
        if o.get("limits").is_some_and(|l| !l.is_object()) {
            return Err(WsError::invalid_op("limits must be an object"));
        }
        let text = Value::Object(o.clone()).to_string();
        if text.len() > 16 * 1024 {
            return Err(WsError::limit("hosted_exec configuration is limited to 16 KiB"));
        }
        self.local.execute("INSERT OR REPLACE INTO local_meta (key, value) VALUES ('hosted_exec', ?1)", [text]).map_err(db_err)?;
        let v = self.hosted_exec()?;
        Ok(json!({ "ok": true, "hosted_exec": v }))
    }

    /// May `caller` trigger a hosted run of `wish_id` under this configuration (D22)?
    pub fn hosted_exec_admits(&self, caller: &Caller, wish_id: &str) -> WsResult<Value> {
        let cfg = self.hosted_exec()?;
        if cfg["enabled"] != json!(true) {
            return Err(WsError::new(Code::ExecutionContextMissing, "hosted execution is not configured for this workspace: the owner sets it up (ws.set_hosted_exec)"));
        }
        if let Some(allowed) = cfg["allowed_wishes"].as_array() {
            if !allowed.is_empty() && !allowed.iter().any(|w| w == wish_id) {
                return Err(WsError::denied(format!("wish {wish_id} is not among the tasks the owner allows to run hosted")));
            }
        }
        if let Some(who) = cfg["triggerers"].as_array() {
            if !who.is_empty() && !who.iter().filter_map(Value::as_str).any(|s| caller.matches(s, true)) {
                return Err(WsError::denied("the owner's hosted execution configuration does not admit this caller"));
            }
        }
        Ok(cfg)
    }

    pub fn require_ws(&self, caller: &Caller, cap: Cap) -> WsResult<Access> {
        let access = self.access(caller)?;
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
        self.grant_with(caller, subject, scope, caps, None, None)
    }

    /// `ws.grant` with the delegation fields of a `run` grant (D16, D22): the digest of the task
    /// configuration the granter confirmed and the outputs it declared. A run whose configuration no longer
    /// matches the digest is `DELEGATION_STALE` (M3).
    pub fn grant_with(&mut self, caller: &Caller, subject: &str, scope: Option<&str>, caps: &[String], task_digest: Option<&str>, outputs: Option<&Value>) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        check_subject(subject, caller.did())?;
        let caps = CapSet::parse(caps)?;
        if (task_digest.is_some() || outputs.is_some()) && !caps.has(Cap::Run) {
            return Err(WsError::invalid_op("task_digest and outputs belong to a run grant"));
        }
        if let Some(d) = task_digest {
            if d.is_empty() || d.len() > 128 {
                return Err(WsError::invalid_op("task_digest must be a short string"));
            }
        }
        if let Some(o) = outputs {
            if !o.is_array() || o.to_string().len() > 16 * 1024 {
                return Err(WsError::invalid_op("outputs must be a small array of output declarations"));
            }
        }
        if subject == PUBLIC_SUBJECT && caps.has(Cap::Manage) {
            return Err(WsError::invalid_op("a public link cannot carry the manage capability"));
        }
        if let Some(s) = scope {
            let now = self.now();
            if self.ctx(&now).entity(s)?.is_none() {
                return Err(WsError::not_found(format!("entity {s} not found")));
            }
        }
        self.local
            .execute(
                "INSERT OR REPLACE INTO grants (subject, scope_entity_id, capabilities, via_entry, task_digest, outputs_json) VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
                params![subject, scope.unwrap_or(""), caps.0, task_digest, outputs.map(Value::to_string)],
            )
            .map_err(db_err)?;
        Ok(json!({ "ok": true }))
    }

    /// The delegation fields of the caller's `run` grants: `(scope, task_digest, outputs)`.
    pub fn run_grants(&self, caller: &Caller) -> WsResult<Vec<(String, Option<String>, Option<Value>)>> {
        let mut st = self.local.prepare_cached("SELECT subject, scope_entity_id, capabilities, task_digest, outputs_json FROM grants ORDER BY subject, scope_entity_id").map_err(db_err)?;
        let rows: Vec<(String, String, u16, Option<String>, Option<String>)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .filter(|(subject, _, caps, ..)| CapSet(*caps).has(Cap::Run) && caller.matches(subject, true))
            .map(|(_, scope, _, digest, outputs)| (scope, digest, outputs.and_then(|o| serde_json::from_str(&o).ok())))
            .collect())
    }

    /// The caller may start a run here: `run` or any editing right on the Workspace (D16).
    pub fn require_run(&self, caller: &Caller) -> WsResult<Access> {
        let access = self.require_ws_any(caller)?;
        if !access.ws_caps.may_run() {
            return Err(WsError::denied("run capability required"));
        }
        Ok(access)
    }

    pub fn revoke(&mut self, caller: &Caller, subject: &str, scope: Option<&str>) -> WsResult<Value> {
        self.require_ws(caller, Cap::Manage)?;
        if self.owner()?.as_deref() == Some(subject) && scope.is_none() {
            return Err(WsError::invalid_op("the owner's workspace grant cannot be revoked"));
        }
        let n = self
            .local
            .execute("DELETE FROM grants WHERE subject = ?1 AND scope_entity_id = ?2", params![subject, scope.unwrap_or("")])
            .map_err(db_err)?;
        Ok(json!({ "ok": true, "removed": n }))
    }

    /// `ws.list_grants`: the whole list for a manager; anyone else only sees the rows that apply to
    /// them (their own subject, `public`, and `*`, groups and contacts where they match), never the complete name
    /// list (phase two §6.3). A row a join wrote names its entry (`via_entry`).
    pub fn list_grants(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let manage = access.ws_caps.has(Cap::Manage);
        let rows: Vec<Value> = self
            .grant_rows()?
            .into_iter()
            .filter(|(subject, ..)| manage || caller.matches(subject, true))
            .map(|(subject, scope, caps, via)| {
                let mut row = json!({ "subject": subject, "scope_entity_id": if scope.is_empty() { Value::Null } else { json!(scope) },
                        "capabilities": CapSet(caps).names(), "via_entry": via.map(|v| self.entry_public_id(&v)) });
                if let Ok((digest, outputs)) = self.local.query_row("SELECT task_digest, outputs_json FROM grants WHERE subject = ?1 AND scope_entity_id = ?2", params![subject, scope], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))) {
                    if let Some(d) = digest {
                        row["task_digest"] = json!(d);
                    }
                    if let Some(o) = outputs.and_then(|o| serde_json::from_str::<Value>(&o).ok()) {
                        row["outputs"] = o;
                    }
                }
                row
            })
            .collect();
        let owner = if manage { self.owner()? } else { None };
        Ok(json!({ "ok": true, "grants": rows, "complete": manage, "owner": owner, "principal": caller.principal }))
    }

    // ---- user work state (phase two §4.4): per subject, per Workspace, in local.sqlite ----

    /// `ws.get_user_state`: every entry of the caller.
    pub fn get_user_state(&self, caller: &Caller) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let mut st = self.local.prepare("SELECT key, value_json, updated_at, revision FROM user_state WHERE subject = ?1 ORDER BY key").map_err(db_err)?;
        let rows: Vec<(String, String, String, i64)> = st
            .query_map([&caller.principal], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        let mut entries = serde_json::Map::new();
        let mut updated = serde_json::Map::new();
        let mut revisions = serde_json::Map::new();
        for (k, v, at, rev) in rows {
            entries.insert(k.clone(), serde_json::from_str(&v).unwrap_or(Value::Null));
            updated.insert(k.clone(), json!(at));
            revisions.insert(k, json!(rev));
        }
        Ok(json!({ "ok": true, "entries": entries, "updated_at": updated, "revisions": revisions }))
    }

    /// `ws.set_user_state`: `entries: { key: value | null }` — null removes; `expect: { key: revision }`
    /// makes a write conditional (D13: `REVISION_CONFLICT` with the current revision when it moved; a new
    /// key expects 0). Each write bumps the entry's revision. Keys `block:<entity_id>/…` are a Block's private
    /// state. Not a document Commit: no history, no change-stream event, no undo.
    pub fn set_user_state(&mut self, caller: &Caller, entries: &Value, expect: Option<&Value>) -> WsResult<Value> {
        self.require_ws_any(caller)?;
        let o = entries.as_object().ok_or_else(|| WsError::invalid_op("entries must be an object"))?;
        if o.len() > 500 {
            return Err(WsError::limit("at most 500 entries per call"));
        }
        let expect = match expect {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(_) => return Err(WsError::invalid_op("expect must be { key: revision }")),
        };
        let now = self.now();
        let tx = self.local.unchecked_transaction().map_err(db_err)?;
        let mut revisions = serde_json::Map::new();
        for (k, v) in o {
            if k.is_empty() || k.len() > 256 {
                return Err(WsError::invalid_op("invalid user state key"));
            }
            let current: i64 = tx
                .query_row("SELECT revision FROM user_state WHERE subject = ?1 AND key = ?2", params![caller.principal, k], |r| r.get(0))
                .optional()
                .map_err(db_err)?
                .unwrap_or(0);
            if let Some(want) = expect.get(k) {
                let want = want.as_i64().ok_or_else(|| WsError::invalid_op("expect revisions must be integers"))?;
                if want != current {
                    return Err(WsError::new(Code::RevisionConflict, format!("user state {k} changed")).with_data(json!({ "key": k, "expected": want, "current": current })));
                }
            }
            if v.is_null() {
                tx.execute("DELETE FROM user_state WHERE subject = ?1 AND key = ?2", params![caller.principal, k]).map_err(db_err)?;
                revisions.insert(k.clone(), json!(0));
            } else {
                let text = v.to_string();
                if text.len() > 256 * 1024 {
                    return Err(WsError::limit("a user state entry is limited to 256 KiB"));
                }
                tx.execute(
                    "INSERT OR REPLACE INTO user_state (subject, key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![caller.principal, k, text, current + 1, now],
                )
                .map_err(db_err)?;
                revisions.insert(k.clone(), json!(current + 1));
            }
        }
        tx.commit().map_err(db_err)?;
        Ok(json!({ "ok": true, "updated_at": now, "revisions": revisions }))
    }

    /// Drop every user's `block:<entity_id>/…` state of entities that are gone for good (D13).
    pub fn clear_block_state(&mut self, entity_ids: &[String]) -> WsResult<u64> {
        let mut n = 0;
        for id in entity_ids {
            n += self.local.execute("DELETE FROM user_state WHERE key LIKE ?1", [format!("block:{id}/%")]).map_err(db_err)? as u64;
        }
        Ok(n)
    }

    /// The history floor (D06): requests below it are `BASE_TOO_OLD`. 0 until history is truncated.
    pub fn base_seq(&self) -> u64 {
        meta_get(&self.doc, "base_seq").ok().flatten().and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    /// Raise the history floor (truncation, D06 / D07): requests below it are refused from now on.
    pub fn set_base_seq(&self, seq: u64) -> WsResult<()> {
        meta_set(&self.doc, "base_seq", &seq.to_string())
    }

    fn check_base(&self, seq: u64) -> WsResult<()> {
        if seq < self.base_seq() {
            return Err(WsError::new(Code::BaseTooOld, format!("history below seq {} is no longer available; prepare the replica again", self.base_seq())));
        }
        Ok(())
    }

    /// The usage ledger (D19): logical bytes per category, kept by every transaction.
    pub fn usage(&self) -> WsResult<Value> {
        let mut st = self.doc.prepare_cached("SELECT category, bytes FROM usage ORDER BY category").map_err(db_err)?;
        let rows: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let mut m = serde_json::Map::new();
        let mut total = 0;
        for (c, b) in rows {
            total += b;
            m.insert(c, json!(b));
        }
        m.insert("total".into(), json!(total));
        Ok(Value::Object(m))
    }

    /// Add `bytes` (negative to release) to a usage category, outside a Commit (exports, clones, staged uploads).
    pub fn add_usage(&self, category: &str, bytes: i64) -> WsResult<()> {
        add_usage(&self.doc, category, bytes)
    }

    /// A bucket of this Workspace's owner (doc/workspace `BuckyOS AiWorkspace Durable Data Schema.md`).
    pub fn bucket(&self, name: &str) -> WsResult<String> {
        bucket_id(&self.owner()?.unwrap_or_default(), name)
    }

    /// Keys under this Workspace's own prefix: a show clone also sees the source's keys and never releases them.
    pub(crate) fn own_key(&self, key: &str) -> bool {
        key.strip_prefix(self.workspace_id.as_str()).is_some_and(|rest| rest.starts_with('/'))
    }

    /// `ws.release_retention { kind, ref }` (maintenance): unbind what holds a snapshot (`snapshot`), an export
    /// package (`export`) or an unreferenced upload (`staged`); `true` when a binding went.
    pub fn release_retention(&self, kind: &str, reference: &str) -> WsResult<bool> {
        match kind {
            "snapshot" => self.release_snapshot(reference),
            "export" => {
                if !is_prefixed_id("ex_", reference) {
                    return Ok(false);
                }
                let key = crate::export::export_key(&self.workspace_id, reference);
                let bucket = self.bucket(crate::objects::EXPORTS)?;
                let held = self.objects.binding(&bucket, &key)?.is_some();
                self.objects.unbind(&bucket, &key)?;
                Ok(held)
            }
            "staged" => {
                let key: Option<String> = self.local.query_row("SELECT bucket_key FROM staged_assets WHERE object_id = ?1", [reference], |r| r.get(0)).optional().map_err(db_err)?;
                let Some(key) = key else { return Ok(false) };
                let referenced: bool = self.doc.query_row("SELECT 1 FROM assets WHERE object_id = ?1 AND bucket_version IS NOT NULL", [reference], |_| Ok(true)).optional().map_err(db_err)?.unwrap_or(false);
                if referenced {
                    return Ok(false);
                }
                if self.own_key(&key) {
                    self.objects.unbind(&self.bucket(ASSETS)?, &key)?;
                }
                let size: i64 = self.local.query_row("SELECT size FROM staged_assets WHERE object_id = ?1", [reference], |r| r.get(0)).map_err(db_err)?;
                self.local.execute("DELETE FROM staged_assets WHERE object_id = ?1", [reference]).map_err(db_err)?;
                self.add_usage("staged", -size)?;
                Ok(true)
            }
            "clone" => Ok(false),
            _ => Err(WsError::invalid_op("kind must be snapshot, export or staged")),
        }
    }

    /// Unbind a snapshot's retention set; the row stays as history.
    pub fn release_snapshot(&self, snapshot_id: &str) -> WsResult<bool> {
        let key: Option<Option<String>> = self
            .doc
            .query_row("SELECT bucket_key FROM snapshots WHERE snapshot_id = ?1 AND bucket_version IS NOT NULL", [snapshot_id], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        let Some(key) = key.flatten() else { return Ok(false) };
        if self.own_key(&key) {
            self.objects.unbind(&self.bucket(OBJECTS)?, &key)?;
        }
        self.doc.execute("UPDATE snapshots SET bucket_version = NULL WHERE snapshot_id = ?1", [snapshot_id]).map_err(db_err)?;
        Ok(true)
    }

    /// Every binding this Workspace's folder records as its own: `(bucket name, key)` (the release of a folder
    /// that goes for good, `Service`).
    pub fn held_keys(&self) -> WsResult<BTreeSet<(String, String)>> {
        let mut out = BTreeSet::new();
        let doc_rows = |sql: &str, name: &str, out: &mut BTreeSet<(String, String)>| -> WsResult<()> {
            let mut st = self.doc.prepare(sql).map_err(db_err)?;
            for k in st.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)? {
                out.insert((name.to_string(), k.map_err(db_err)?));
            }
            Ok(())
        };
        doc_rows("SELECT bucket_key FROM assets WHERE bucket_version IS NOT NULL AND bucket_key IS NOT NULL", ASSETS, &mut out)?;
        doc_rows("SELECT bucket_key FROM snapshots WHERE bucket_version IS NOT NULL AND bucket_key IS NOT NULL", OBJECTS, &mut out)?;
        doc_rows("SELECT DISTINCT bucket_key FROM entity_versions WHERE bucket_version IS NOT NULL AND bucket_key IS NOT NULL", OBJECTS, &mut out)?;
        let mut st = self.local.prepare("SELECT bucket_key FROM staged_assets").map_err(db_err)?;
        for k in st.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)? {
            out.insert((ASSETS.to_string(), k.map_err(db_err)?));
        }
        Ok(out)
    }

    pub fn info(&self, caller: &Caller) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        Ok(json!({ "ok": true, "workspace_id": self.workspace_id, "title": self.title(), "epoch": self.epoch,
                   "head_seq": self.head_seq, "head_commit_id": self.head_commit_id, "base_seq": self.base_seq(), "format_version": FORMAT_VERSION,
                   "protocol_version": PROTOCOL_VERSION, "planner_version": aiworkspace_core::plan::PLANNER_VERSION,
                   "richtext_schema": aiworkspace_core::richtext::schema()["id"], "usage": self.usage()?,
                   "capabilities": access.ws_caps.names(), "principal": caller.principal,
                   "forked_from": meta_get(&self.doc, "forked_from")?.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
                   "show_lock": self.show_lock().map(|l| l.to_json()), "purpose": self.purpose(),
                   "wish_aicc_mode": self.wish_aicc_mode()?, "hosted_exec": self.hosted_exec()? }))
    }

    /// Any grant at all (workspace-level or scoped) is enough to know the Workspace exists.
    pub fn require_ws_any(&self, caller: &Caller) -> WsResult<Access> {
        let access = self.access(caller)?;
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
        // the identity the Commit is written as (D08): the caller, or a run's execution identity
        let writer = opts.as_principal.clone().unwrap_or_else(|| caller.principal.clone());
        // idempotency comes before anything that depends on document state: a
        // successful create whose response was lost must not hit its own ID_CONFLICT
        let prior: Option<(String, String)> = self
            .doc
            .query_row(
                "SELECT request_digest, result_json FROM commits WHERE principal = ?1 AND idem_key = ?2",
                params![writer, req.idempotency_key],
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
        let access = self.access(caller).map_err(reject)?;
        if access.is_empty() {
            return Err(reject(WsError::not_found("workspace not found")));
        }
        // the quota is checked before anything is planned (D19); the request size is the reservation
        self.check_quota(req_json.to_string().len() as u64, opts.maintenance).map_err(reject)?;
        // a presented Workspace is closed for writes, from every session (third phase §8.1)
        if let Some(lock) = self.show_lock() {
            return Err(reject(Self::show_locked_error(&lock)));
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
        // origin is the server's reading of who writes (D08); what the client wrote is ignored
        let origin = derive_origin(caller, opts, req.run_id.is_some());
        let mut req = req;
        req.origin = origin.to_string();
        let req = &req;
        let env = PlanEnv {
            seq,
            principal: writer.clone(),
            origin: origin.to_string(),
            run_id: req.run_id.clone(),
            now: now.clone(),
            internal: opts.internal,
            import: opts.import,
            schema_checks: true,
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
            .write_commit(req, caller, &writer, opts, &digest, seq, &commit_id, &now, &ops, &changes, &result)
            .map_err(reject)?;
        // durable from here on; a crash before publishing is recovered from the database
        for id in changes.assets.keys() {
            // a referenced upload is no longer staged
            let _ = self.local.execute("DELETE FROM staged_assets WHERE object_id = ?1", [id]);
        }
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
        writer: &str,
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
        // the usage ledger moves with the transaction (D19): structured bytes by their delta, assets by size
        let (content_delta, asset_delta) = usage_delta(&tx, changes)?;
        let stats = apply_changes(&tx, changes, seq)?;
        add_usage(&tx, "content", content_delta)?;
        add_usage(&tx, "assets", asset_delta)?;
        // a staged upload this Commit references is an asset now: its binding moves to the asset row, its usage ends
        let mut staged_bytes: i64 = 0;
        for (id, a) in &changes.assets {
            let staged: Option<(String, i64)> =
                self.local.query_row("SELECT bucket_key, bucket_version FROM staged_assets WHERE object_id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(db_err)?;
            if let Some((key, version)) = staged {
                tx.execute("UPDATE assets SET bucket_key = ?2, bucket_version = ?3 WHERE object_id = ?1", params![id, key, version]).map_err(db_err)?;
                staged_bytes += a.size as i64;
            }
        }
        add_usage(&tx, "staged", -staged_bytes)?;

        // a generated result gets an addressable version right away (phase two §7.4): its content is
        // materialized and listed in entity_versions together with the dependency record
        let versioned: Vec<String> = ops
            .iter()
            .filter(|o| o.op["op"] == json!("entity.set_derived"))
            .filter_map(|o| o.op["entity_id"].as_str().map(str::to_string))
            .collect();
        // the versions of this Commit are retained together: one retention set under `<ws>/version/<epoch>/<seq>`,
        // bound before the transaction commits and unbound when it does not
        let mut version_key: Option<String> = None;
        if !versioned.is_empty() {
            let ctx = SqlCtx { conn: &tx, local: Some(&self.local), cache: &self.cache, now };
            let mut sink = crate::objects::StoreSink::new(&self.objects);
            let mut members = BTreeMap::new();
            let mut rows = Vec::new();
            for id in versioned {
                let Some(e) = changes.entities.get(&id) else { continue };
                let object_id = aiworkspace_core::materialize::content_object(&ctx, e, &mut sink)?;
                self.objects.entity_members(&object_id, &e.type_id, &mut members)?;
                rows.push((id, e.content_rev, object_id, e.derived.as_ref().map(Value::to_string)));
            }
            let key = format!("{}/version/{}/{seq}", self.workspace_id, self.epoch);
            let (_, version) = self.objects.retain(&self.bucket(OBJECTS)?, &key, &members)?;
            version_key = Some(key.clone());
            for (id, rev, object_id, derived) in rows {
                tx.execute(
                    "INSERT INTO entity_versions (entity_id, content_rev, object_id, derived_json, kind, created_at, bucket_key, bucket_version) VALUES (?1, ?2, ?3, ?4, 'generated', ?5, ?6, ?7) \
                     ON CONFLICT(entity_id, content_rev) DO UPDATE SET object_id = excluded.object_id, derived_json = excluded.derived_json, kind = 'generated', created_at = excluded.created_at, \
                     bucket_key = excluded.bucket_key, bucket_version = excluded.bucket_version",
                    params![id, rev, object_id, derived, now, key, version],
                )
                .map_err(db_err)?;
            }
        }
        let unbind_versions = |objects: &ObjectStore, bucket: WsResult<String>| {
            if let (Some(key), Ok(bucket)) = (&version_key, bucket) {
                let _ = objects.unbind(&bucket, key);
            }
        };
        tx.execute(
            "INSERT INTO commits (seq, commit_id, principal, app_id, session_id, origin, run_id, undo_group, undoes, idem_key, \
             request_digest, message, accepted_at, result_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                seq, commit_id, writer, caller.app_id, req.session_id, req.origin, req.run_id, req.undo_group,
                opts.undoes, req.idempotency_key, digest, req.message, now, result.to_string()
            ],
        )
        .map_err(db_err)?;
        // the pure-application delta replicas apply (D20)
        let rows = aiworkspace_core::replica::changes_to_rows(changes)?;
        tx.execute("INSERT INTO commit_deltas (seq, rows_json) VALUES (?1, ?2)", params![seq, rows.to_string()]).map_err(db_err)?;
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
            unbind_versions(&self.objects, self.bucket(OBJECTS));
            self.fire(FailAt::InTxn)?;
            return Err(WsError::io("injected failure"));
        }
        if let Err(e) = tx.commit() {
            unbind_versions(&self.objects, self.bucket(OBJECTS));
            return Err(db_err(e));
        }
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
        self.check_base(seq)?;
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
        let gone = self.reclaimed_objects(&inverse)?;
        if !gone.is_empty() {
            return Err(WsError::new(Code::ResourceReclaimed, "the undo needs resources reclaimed after their retention period").with_data(json!({ "object_ids": gone })));
        }
        let (workspace_id, epoch) = (self.workspace_id.clone(), self.epoch.clone());
        let build = |ops: &[Value]| -> Value {
            json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": workspace_id, "epoch": epoch,
                    "idempotency_key": idem, "session_id": req.get("session_id").cloned().unwrap_or(Value::Null),
                    "origin": "human", "message": format!("undo {target}"), "operations": ops })
        };
        let opts = CommitOpts { internal: true, import: false, undoes: Some(target.to_string()), as_principal: None, maintenance: false, as_source: false, as_build: false };
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
        self.check_base(after_seq)?;
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
                    // the row delta, restricted to what the caller may read (D20)
                    let delta: Option<String> = self.doc.query_row("SELECT rows_json FROM commit_deltas WHERE seq = ?1", [seq], |r| r.get(0)).optional().map_err(db_err)?;
                    if let Some(d) = delta {
                        let mut rows: Value = serde_json::from_str(&d).map_err(|e| WsError::io(e.to_string()))?;
                        bytes += d.len();
                        filter_rows(&mut rows, &mut can_read)?;
                        ev["rows"] = rows;
                    }
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
