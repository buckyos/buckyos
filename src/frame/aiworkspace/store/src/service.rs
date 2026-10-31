//! The set of Workspaces of one deployment (design doc §4.1): a folder per
//! Workspace, no global database. The folder list *is* the catalogue.

use crate::export::{load_workspace, open_package, LoadSpec};
use crate::objects::ObjectStore;
use crate::urlsource::SourceRegistry;
use crate::docdb::db_err;
use crate::workspace::{random_id, system_clock, Caller, Clock, FailPoint, Workspace};
use aiworkspace_core::value::format_utc_ms;
use rusqlite::Connection;
use aiworkspace_core::access::Cap;
use aiworkspace_core::id::is_prefixed_id;
use aiworkspace_core::{Code, Deny, WsError, WsResult};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type WsHandle = Arc<Mutex<Workspace>>;

pub struct Service {
    pub data_dir: PathBuf,
    pub objects: Arc<ObjectStore>,
    pub clock: Clock,
    pub sources: SourceRegistry,
    pub failpoint: Option<FailPoint>,
    /// Called for every Workspace this service opens (the server hooks its notifiers here).
    pub on_open: Option<Box<dyn Fn(&mut Workspace) + Send + Sync>>,
    /// Show locks of this deployment's Workspaces (third phase §8.1); in memory, a restart ends every show.
    pub show_locks: crate::show::ShowLocks,
    /// Logical bytes a Workspace may occupy (D19); `None` = no quota.
    pub quota: Option<u64>,
    /// Bytes a Workspace's Sources may fetch (D24); `None` = unlimited.
    pub traffic_quota: Option<u64>,
    /// Retention of history-only resources (D06), in milliseconds.
    pub history_retention_ms: i64,
    /// The presentation sessions of this deployment (第四期规划 §3): `sessions.sqlite` in the data folder.
    pub sessions: crate::session::registry::Sessions,
    /// How long an ended, unsaved session is kept (Q10).
    pub session_retention_ms: i64,
    open: Mutex<HashMap<String, WsHandle>>,
}

/// A deleted Workspace stays in `trash/`, its bindings kept, this long before the sweep releases them (D06).
pub const TRASH_RETENTION_MS: i64 = 30 * 24 * 3_600_000;

fn io(e: std::io::Error) -> WsError {
    WsError::io(e.to_string())
}

impl Service {
    pub fn open(data_dir: &Path, objects: Arc<ObjectStore>) -> WsResult<Service> {
        Self::open_with_clock(data_dir, objects, system_clock())
    }

    pub fn open_with_clock(data_dir: &Path, objects: Arc<ObjectStore>, clock: Clock) -> WsResult<Service> {
        let root = data_dir.join("workspaces");
        std::fs::create_dir_all(&root).map_err(io)?;
        std::fs::create_dir_all(data_dir.join("trash")).map_err(io)?;
        for entry in std::fs::read_dir(&root).map_err(io)? {
            let entry = entry.map_err(io)?;
            if entry.file_name().to_string_lossy().starts_with(".tmp-") {
                let intent = std::fs::read(entry.path().join("restore-intent.json")).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
                if let Some(intent) = intent {
                    let workspace = intent["workspace_id"].as_str().unwrap_or_default();
                    let backup = intent["backup"].as_str().unwrap_or_default();
                    if is_prefixed_id("ws_", workspace) && is_prefixed_id("restore_", backup)
                        && !root.join(workspace).exists() && data_dir.join("trash").join(backup).is_dir() {
                        std::fs::rename(entry.path(), root.join(workspace)).map_err(io)?;
                        let _ = std::fs::remove_file(root.join(workspace).join("restore-intent.json"));
                        continue;
                    }
                }
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        let svc = Service {
            data_dir: data_dir.to_path_buf(),
            objects,
            clock,
            sources: SourceRegistry::default(),
            failpoint: None,
            on_open: None,
            show_locks: Default::default(),
            quota: None,
            traffic_quota: None,
            history_retention_ms: crate::workspace::HISTORY_RETENTION_MS,
            sessions: crate::session::registry::Sessions::open(&data_dir.join("sessions.sqlite"))?,
            session_retention_ms: crate::session::DEFAULT_RETENTION_MS,
            open: Mutex::new(HashMap::new()),
        };
        // no connection survived a restart
        svc.sessions.all_offline()?;
        svc.drop_stale_clones()?;
        svc.mark_interrupted_runs()?;
        Ok(svc)
    }

    /// Service start (D15): no worker survived, so every run a worker owned is `interrupted` now — before
    /// anyone asks `proc.list` — and never restarted on its own. `applying` runs are reconciled through their
    /// idempotency record when they are next read.
    fn mark_interrupted_runs(&self) -> WsResult<()> {
        for id in self.workspace_ids()? {
            let path = self.ws_dir(&id).join("local.sqlite");
            let Ok(local) = Connection::open(&path) else { continue };
            let states = crate::wish::runs::active_states_sql().replace("'applying'", "''");
            let mut st = local.prepare(&format!("SELECT run_id, result_json FROM runs WHERE state IN ({states})")).map_err(db_err)?;
            let rows: Vec<(String, Option<String>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
            drop(st);
            for (run_id, result) in rows {
                let mut detail: Value = result.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}));
                detail["error"] = json!({ "code": "INTERRUPTED", "detail": "The service restarted during the run; it does not restart automatically" });
                local
                    .execute("UPDATE runs SET state = 'interrupted', result_json = ?2, updated_at = ?3 WHERE run_id = ?1", rusqlite::params![run_id, detail.to_string(), format_utc_ms((self.clock)())])
                    .map_err(db_err)?;
            }
        }
        Ok(())
    }

    /// The periodic release (D06), hourly: every Workspace unbinds its expired staged uploads and history-only
    /// assets past their retention; a folder in `trash/` older than `TRASH_RETENTION_MS` releases its bindings
    /// and goes. What the buckets no longer hold the named store reclaims.
    pub fn sweep(&self) -> WsResult<Value> {
        let now = (self.clock)();
        let (mut workspaces, mut staged, mut history) = (0, 0, 0);
        for id in self.workspace_ids()? {
            let Ok(handle) = self.workspace(&id) else { continue };
            let released = handle.lock().unwrap().release_expired()?;
            staged += released["staged"].as_u64().unwrap_or(0);
            history += released["history"].as_u64().unwrap_or(0);
            workspaces += 1;
        }
        let mut trashed = 0;
        for entry in std::fs::read_dir(self.data_dir.join("trash")).map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name().to_string_lossy().to_string();
            let at: Option<i64> = name.rsplit('-').next().and_then(|s| s.parse().ok());
            let restore_backup = is_prefixed_id("restore_", &name);
            let modified = entry.metadata().ok().and_then(|m| m.modified().ok()).map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0));
            let expired = match (at, restore_backup) {
                (Some(t), false) => now - t > TRASH_RETENTION_MS,
                (_, true) => modified.is_some_and(|t| now - t > TRASH_RETENTION_MS),
                _ => false,
            };
            if expired {
                self.release_folder(&entry.path())?;
                let _ = std::fs::remove_dir_all(entry.path());
                trashed += 1;
            }
        }
        Ok(json!({ "ok": true, "workspaces": workspaces, "staged": staged, "history": history, "trash_dropped": trashed }))
    }

    /// Unbind what a folder that goes for good (trash, a show clone, a failed import) holds under its own key
    /// prefix, except what the live Workspace of the same id still holds.
    pub(crate) fn release_folder(&self, dir: &Path) -> WsResult<()> {
        let Ok(folder) = Workspace::open(dir, self.objects.clone(), self.clock.clone()) else { return Ok(()) };
        let held = folder.held_keys()?;
        let live = match self.ws_dir(&folder.workspace_id) {
            path if path != dir && path.exists() => self.workspace(&folder.workspace_id)?.lock().unwrap().held_keys()?,
            _ => Default::default(),
        };
        for (name, key) in held {
            if folder.own_key(&key) && !live.contains(&(name.clone(), key.clone())) {
                self.objects.unbind(&folder.bucket(&name)?, &key)?;
            }
        }
        Ok(())
    }

    pub(crate) fn ws_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("workspaces").join(id)
    }

    /// Drop a folder that never became a Workspace (a failed import or fork), its bindings with it.
    pub(crate) fn discard(&self, dir: &Path) {
        if let Err(e) = self.release_folder(dir) {
            log::warn!("release bindings of {}: {e}", dir.display());
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    pub(crate) fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("workspaces").join(format!(".tmp-{}", random_id("")))
    }

    fn configure(&self, ws: &mut Workspace) {
        ws.failpoint = self.failpoint.clone();
        ws.show_locks = Some(self.show_locks.clone());
        ws.quota = self.quota;
        ws.traffic_quota = self.traffic_quota;
        ws.history_retention_ms = self.history_retention_ms;
        if let Some(hook) = &self.on_open {
            hook(ws);
        }
    }

    fn register(&self, mut ws: Workspace) -> WsHandle {
        self.configure(&mut ws);
        let id = ws.workspace_id.clone();
        let handle = Arc::new(Mutex::new(ws));
        self.open.lock().unwrap().insert(id, handle.clone());
        handle
    }

    /// The single writer of a Workspace. Opens the folder on first use.
    pub fn workspace(&self, id: &str) -> WsResult<WsHandle> {
        if !is_prefixed_id("ws_", id) {
            return Err(WsError::not_found("workspace not found"));
        }
        if let Some(h) = self.open.lock().unwrap().get(id) {
            return Ok(h.clone());
        }
        let ws = Workspace::open(&self.ws_dir(id), self.objects.clone(), self.clock.clone())?;
        // two racing opens must not create two writers
        let open = self.open.lock().unwrap();
        if let Some(h) = open.get(id) {
            return Ok(h.clone());
        }
        drop(open);
        Ok(self.register(ws))
    }

    /// Release the writer of an idle Workspace (its WAL is checkpointed on close).
    pub fn close(&self, id: &str) {
        self.open.lock().unwrap().remove(id);
    }

    pub fn is_current_workspace(&self, id: &str, handle: &WsHandle) -> bool {
        self.open.lock().unwrap().get(id).is_some_and(|current| Arc::ptr_eq(current, handle))
    }

    /// `ws.create`: built under a temporary name, renamed into place when complete.
    pub fn create_workspace(&self, caller: &Caller, title: &str, workspace_id: Option<&str>) -> WsResult<Value> {
        require_may_own(caller)?;
        let id = match workspace_id {
            Some(id) if is_prefixed_id("ws_", id) => id.to_string(),
            Some(_) => return Err(WsError::invalid_op("workspace_id must be ws_ + 26 base32 chars")),
            None => random_id("ws_"),
        };
        if title.chars().count() > 256 {
            return Err(WsError::invalid_op("title too long"));
        }
        if self.ws_dir(&id).exists() {
            return Err(WsError::sub(Code::InvalidOperation, "ID_CONFLICT", "workspace already exists"));
        }
        let tmp = self.tmp_dir();
        let ws = Workspace::create(&tmp, &id, title, &caller.principal, self.objects.clone(), self.clock.clone())?;
        let info = json!({ "ok": true, "workspace_id": id, "epoch": ws.epoch, "head_seq": 0, "title": title });
        drop(ws);
        std::fs::rename(&tmp, self.ws_dir(&id)).map_err(io)?;
        Ok(info)
    }

    /// Every Workspace of this service (the timers' list).
    pub fn all_workspace_ids(&self) -> WsResult<Vec<String>> {
        self.workspace_ids()
    }

    fn workspace_ids(&self) -> WsResult<Vec<String>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(self.data_dir.join("workspaces")).map_err(io)? {
            let name = entry.map_err(io)?.file_name().to_string_lossy().to_string();
            if is_prefixed_id("ws_", &name) {
                out.push(name);
            }
        }
        Ok(out)
    }

    /// `group:` / `contact:` subjects granted anywhere in this deployment (what `ws.list` may depend on).
    pub fn dynamic_subjects(&self) -> WsResult<Vec<String>> {
        let mut out = std::collections::BTreeSet::new();
        for name in self.workspace_ids()? {
            let Ok(handle) = self.workspace(&name) else { continue };
            out.extend(handle.lock().unwrap().dynamic_subjects()?);
        }
        Ok(out.into_iter().collect())
    }

    /// `ws.list`: Workspaces on which the caller holds any grant with `read`. A guest reaches public Workspaces by
    /// their links only, never by enumeration; other restricted callers list what was granted to them (no `public`,
    /// no `*`).
    pub fn list_workspaces(&self, caller: &Caller) -> WsResult<Value> {
        let mut out = Vec::new();
        if caller.is_guest() {
            return Ok(json!({ "ok": true, "workspaces": out }));
        }
        for name in self.workspace_ids()? {
            let Ok(handle) = self.workspace(&name) else { continue };
            let ws = handle.lock().unwrap();
            // a show clone belongs to its stage only (third phase §8.3)
            if ws.purpose().is_some() {
                continue;
            }
            // a membership that cannot be checked leaves the Workspace out of the list (opening it says why)
            let Ok(access) = ws.listing_access(caller) else { continue };
            let reads = access.ws_caps.has(Cap::Read) || access.scoped.values().any(|c| c.has(Cap::Read));
            if reads {
                out.push(json!({ "workspace_id": name, "title": ws.title(), "head_seq": ws.head_seq, "epoch": ws.epoch,
                                 "capabilities": access.ws_caps.names() }));
            }
        }
        out.sort_by(|a, b| a["workspace_id"].as_str().cmp(&b["workspace_id"].as_str()));
        Ok(json!({ "ok": true, "workspaces": out }))
    }

    fn trash(&self, id: &str) -> WsResult<()> {
        self.close(id);
        let dst = self.data_dir.join("trash").join(format!("{id}-{}", (self.clock)()));
        std::fs::rename(self.ws_dir(id), dst).map_err(io)
    }

    /// `ws.delete`: moved to the trash folder; nothing is physically removed in this phase.
    pub fn delete_workspace(&self, caller: &Caller, id: &str) -> WsResult<Value> {
        {
            let handle = self.workspace(id)?;
            let ws = handle.lock().unwrap();
            ws.require_ws(caller, Cap::Manage)?;
        }
        self.trash(id)?;
        Ok(json!({ "ok": true }))
    }

    /// `ws.import`. The semantics must be chosen explicitly:
    /// `new` (= Fork: fresh id, importer becomes owner) or `restore` (keeps the
    /// package's id; replacing an existing Workspace needs `replace: true` *and*
    /// `manage` on the existing one — nothing inside the package grants anything).
    pub fn import(&self, caller: &Caller, package: &Path, semantics: &str, replace: bool) -> WsResult<Value> {
        require_may_own(caller)?;
        let pkg = open_package(package, &self.objects)?;
        let m = &pkg.manifest;
        let snapshot = m["snapshot"].as_str().ok_or_else(|| WsError::invalid_schema("manifest.snapshot missing"))?;
        let content_root = m["content_root"].as_str().ok_or_else(|| WsError::invalid_schema("manifest.content_root missing"))?;
        use aiworkspace_core::materialize::ObjectSource;
        let snap = aiworkspace_core::canonical::parse_strict(&self.objects.get_object(snapshot)?)?;
        if snap["content"] != json!(content_root) || snap["ws_type"] != json!("buckyos.workspace-snapshot") {
            return Err(WsError::invalid_schema("snapshot root does not match the manifest"));
        }
        let source_id = snap["workspace_id"].as_str().unwrap_or("").to_string();
        let title = m["title"].as_str().unwrap_or("");
        let personal_backup = m["export_mode"] == json!("personal_backup");
        let (id, forked_from, existing) = match semantics {
            "new" => (random_id("ws_"), Some(json!({ "workspace_id": source_id, "snapshot": snapshot })), false),
            "restore" => {
                if !is_prefixed_id("ws_", &source_id) {
                    return Err(WsError::invalid_schema("package has no valid workspace_id"));
                }
                let exists = self.ws_dir(&source_id).exists();
                if exists {
                    // judged on the existing folder only; indistinguishable from "cannot create" when denied
                    let denied = || WsError::denied("restore is not permitted");
                    let handle = self.workspace(&source_id).map_err(|_| denied())?;
                    let allowed = handle.lock().unwrap().access(caller)?.ws_caps.has(Cap::Manage);
                    if !allowed || !replace {
                        return Err(denied());
                    }
                }
                (source_id.clone(), snap.get("lineage").and_then(|l| l.get("forked_from")).cloned(), exists)
            }
            _ => return Err(WsError::invalid_op("semantics must be \"restore\" or \"new\"")),
        };
        // only a restore of a personal backup continues the original collaboration lineages
        let mut collab = std::collections::BTreeMap::new();
        if personal_backup && semantics == "restore" {
            for (entity_id, bytes) in pkg.collab {
                if let Some(lineage) = m["collab"][&entity_id]["lineage_id"].as_str() {
                    collab.insert(entity_id, (lineage.to_string(), bytes));
                }
            }
        }
        let personal = if personal_backup { pkg.personal } else { vec![] };
        // a restore over an existing Workspace keeps its owner, whose buckets hold its objects
        let owner = match existing {
            true => self.workspace(&id)?.lock().unwrap().owner()?.unwrap_or_else(|| caller.principal.clone()),
            false => caller.principal.clone(),
        };
        let tmp = self.tmp_dir();
        let spec = LoadSpec { workspace_id: &id, title, content_root, collab, personal, forked_from };
        let ws = match load_workspace(&tmp, spec, caller, &owner, self.objects.clone(), self.clock.clone()) {
            Ok(ws) => ws,
            Err(e) => {
                self.discard(&tmp);
                return Err(e); // the existing Workspace, if any, is untouched
            }
        };
        let info = json!({ "ok": true, "workspace_id": id, "epoch": ws.epoch, "head_seq": ws.head_seq, "content_root": content_root,
                           "semantics": semantics, "replaced": existing, "missing": m["missing"], "external_sources": m["external_sources"] });
        drop(ws);
        if existing {
            let handle = self.workspace(&id)?;
            let mut active = handle.lock().unwrap();
            if !self.is_current_workspace(&id, &handle) {
                return Err(WsError::new(Code::WriterBusy, "workspace writer was replaced; retry"));
            }
            active.require_ws(caller, Cap::Manage)?;
            // restoring content does not change who may access it: grants and entries move over, runs and uploads do not
            let old_local = self.ws_dir(&id).join("local.sqlite");
            let conn = rusqlite::Connection::open(tmp.join("local.sqlite")).map_err(crate::docdb::db_err)?;
            conn.execute("ATTACH DATABASE ?1 AS old", [old_local.to_string_lossy().as_ref()]).map_err(crate::docdb::db_err)?;
            conn.execute_batch("DELETE FROM grants; INSERT INTO grants SELECT * FROM old.grants; \
                                INSERT INTO share_entries SELECT * FROM old.share_entries; \
                                INSERT OR REPLACE INTO local_meta SELECT * FROM old.local_meta; DETACH DATABASE old;")
                .map_err(crate::docdb::db_err)?;
            drop(conn);
            let backup = random_id("restore_");
            write_atomic(&tmp.join("restore-intent.json"), json!({"workspace_id": id, "backup": backup}).to_string().as_bytes())?;
            let backup_path = self.data_dir.join("trash").join(&backup);
            let doc = rusqlite::Connection::open_in_memory().map_err(crate::docdb::db_err)?;
            let local = rusqlite::Connection::open_in_memory().map_err(crate::docdb::db_err)?;
            drop(std::mem::replace(&mut active.doc, doc));
            drop(std::mem::replace(&mut active.local, local));
            let replaced = (|| {
                std::fs::rename(self.ws_dir(&id), &backup_path).map_err(io)?;
                if let Err(error) = std::fs::rename(&tmp, self.ws_dir(&id)) {
                    let _ = std::fs::rename(&backup_path, self.ws_dir(&id));
                    return Err(io(error));
                }
                let _ = std::fs::remove_file(self.ws_dir(&id).join("restore-intent.json"));
                Ok(())
            })();
            let mut replacement = match Workspace::open(&self.ws_dir(&id), self.objects.clone(), self.clock.clone()) {
                Ok(workspace) => workspace,
                Err(error) => { self.close(&id); return Err(error); }
            };
            self.configure(&mut replacement);
            *active = replacement;
            replaced?;
        } else {
            std::fs::rename(&tmp, self.ws_dir(&id)).map_err(io)?;
        }
        Ok(info)
    }

    /// `ws.fork`: materialize what the caller may read, then load it as a new
    /// Workspace. No grants, runs, uploads or personal entities are copied;
    /// rich texts start new lineages.
    pub fn fork(&self, caller: &Caller, source_id: &str) -> WsResult<Value> {
        require_may_own(caller)?;
        let (cp, title) = {
            let handle = self.workspace(source_id)?;
            let ws = handle.lock().unwrap();
            let access = ws.require_ws(caller, Cap::Export)?;
            (ws.materialize_for(&access, "fork_source", &caller.principal)?, ws.title())
        };
        let id = random_id("ws_");
        let tmp = self.tmp_dir();
        let spec = LoadSpec {
            workspace_id: &id,
            title: &title,
            content_root: &cp.content_root,
            collab: Default::default(),
            personal: vec![],
            forked_from: Some(json!({ "workspace_id": source_id, "snapshot": cp.snapshot_id })),
        };
        let ws = match load_workspace(&tmp, spec, caller, &caller.principal, self.objects.clone(), self.clock.clone()) {
            Ok(ws) => ws,
            Err(e) => {
                self.discard(&tmp);
                return Err(e);
            }
        };
        let info = json!({ "ok": true, "workspace_id": id, "epoch": ws.epoch, "head_seq": ws.head_seq, "content_root": cp.content_root,
                           "forked_from": { "workspace_id": source_id, "snapshot": cp.snapshot_id }, "excluded_entities": cp.excluded });
        drop(ws);
        std::fs::rename(&tmp, self.ws_dir(&id)).map_err(io)?;
        Ok(info)
    }
}

/// Workspaces are created, imported and forked — and the Zone's principals listed — by unrestricted accounts and
/// services only: a guest, an External DID or a restricted (Limited) account owns nothing (PRE-03).
/// Write-then-rename so a crash never leaves a half-written file under its name.
fn write_atomic(path: &Path, bytes: &[u8]) -> WsResult<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", rand::random::<u64>()));
    let mut f = std::fs::File::create(&tmp).map_err(io)?;
    f.write_all(bytes).map_err(io)?;
    f.sync_all().map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

pub fn require_may_own(caller: &Caller) -> WsResult<()> {
    if !caller.may_own() {
        return Err(WsError::refused(Code::PermissionDenied, Deny::SessionKindNotAllowed, "this kind of account cannot own workspaces"));
    }
    Ok(())
}
