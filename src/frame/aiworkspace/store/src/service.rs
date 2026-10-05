//! The set of Workspaces of one deployment (design doc §4.1): a folder per
//! Workspace, no global database. The folder list *is* the catalogue.

use crate::export::{load_workspace, open_package, LoadSpec};
use crate::objects::FsObjectStore;
use crate::urlsource::SourceRegistry;
use crate::workspace::{random_id, system_clock, Caller, Clock, FailPoint, Workspace};
use aiworkspace_core::access::{Access, Cap};
use aiworkspace_core::id::is_prefixed_id;
use aiworkspace_core::{Code, WsError, WsResult};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type WsHandle = Arc<Mutex<Workspace>>;

pub struct Service {
    pub data_dir: PathBuf,
    pub objects: Arc<FsObjectStore>,
    pub clock: Clock,
    pub sources: SourceRegistry,
    pub failpoint: Option<FailPoint>,
    /// Called for every Workspace this service opens (the server hooks its notifiers here).
    pub on_open: Option<Box<dyn Fn(&mut Workspace) + Send + Sync>>,
    open: Mutex<HashMap<String, WsHandle>>,
}

fn io(e: std::io::Error) -> WsError {
    WsError::io(e.to_string())
}

impl Service {
    pub fn open(data_dir: &Path) -> WsResult<Service> {
        Self::open_with_clock(data_dir, system_clock())
    }

    pub fn open_with_clock(data_dir: &Path, clock: Clock) -> WsResult<Service> {
        let root = data_dir.join("workspaces");
        std::fs::create_dir_all(&root).map_err(io)?;
        std::fs::create_dir_all(data_dir.join("trash")).map_err(io)?;
        std::fs::create_dir_all(data_dir.join("staging")).map_err(io)?;
        // half-built folders never carry a real name; drop what a crash left behind
        for entry in std::fs::read_dir(&root).map_err(io)? {
            let entry = entry.map_err(io)?;
            if entry.file_name().to_string_lossy().starts_with(".tmp-") {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        Ok(Service {
            data_dir: data_dir.to_path_buf(),
            objects: Arc::new(FsObjectStore::open(&data_dir.join("objects"))?),
            clock,
            sources: SourceRegistry::default(),
            failpoint: None,
            on_open: None,
            open: Mutex::new(HashMap::new()),
        })
    }

    fn ws_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("workspaces").join(id)
    }

    fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("workspaces").join(format!(".tmp-{}", random_id("")))
    }

    fn register(&self, mut ws: Workspace) -> WsHandle {
        ws.failpoint = self.failpoint.clone();
        if let Some(hook) = &self.on_open {
            hook(&mut ws);
        }
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

    /// `ws.create`: built under a temporary name, renamed into place when complete.
    pub fn create_workspace(&self, caller: &Caller, title: &str, workspace_id: Option<&str>) -> WsResult<Value> {
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

    /// `ws.list`: Workspaces on which the caller holds any grant with `read`.
    pub fn list_workspaces(&self, caller: &Caller) -> WsResult<Value> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(self.data_dir.join("workspaces")).map_err(io)? {
            let name = entry.map_err(io)?.file_name().to_string_lossy().to_string();
            if !is_prefixed_id("ws_", &name) {
                continue;
            }
            let Ok(handle) = self.workspace(&name) else { continue };
            let ws = handle.lock().unwrap();
            let access: Access = ws.access(&caller.principal)?;
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
                    let allowed = handle.lock().unwrap().access(&caller.principal)?.ws_caps.has(Cap::Manage);
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
        let tmp = self.tmp_dir();
        let spec = LoadSpec { workspace_id: &id, title, content_root, collab, personal, forked_from };
        let ws = match load_workspace(&tmp, spec, caller, self.objects.clone(), self.clock.clone()) {
            Ok(ws) => ws,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(e); // the existing Workspace, if any, is untouched
            }
        };
        let info = json!({ "ok": true, "workspace_id": id, "epoch": ws.epoch, "head_seq": ws.head_seq, "content_root": content_root,
                           "semantics": semantics, "replaced": existing, "missing": m["missing"], "external_sources": m["external_sources"] });
        drop(ws);
        if existing {
            // restoring content does not change who may access it: grants move over, runs and uploads do not
            let old_local = self.ws_dir(&id).join("local.sqlite");
            self.close(&id);
            let conn = rusqlite::Connection::open(tmp.join("local.sqlite")).map_err(crate::docdb::db_err)?;
            conn.execute("ATTACH DATABASE ?1 AS old", [old_local.to_string_lossy().as_ref()]).map_err(crate::docdb::db_err)?;
            conn.execute_batch("DELETE FROM grants; INSERT INTO grants SELECT * FROM old.grants; \
                                INSERT OR REPLACE INTO local_meta SELECT * FROM old.local_meta; DETACH DATABASE old;")
                .map_err(crate::docdb::db_err)?;
            drop(conn);
            self.trash(&id)?;
        }
        std::fs::rename(&tmp, self.ws_dir(&id)).map_err(io)?;
        Ok(info)
    }

    /// `ws.fork`: materialize what the caller may read, then load it as a new
    /// Workspace. No grants, runs, uploads or personal entities are copied;
    /// rich texts start new lineages.
    pub fn fork(&self, caller: &Caller, source_id: &str) -> WsResult<Value> {
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
        let ws = match load_workspace(&tmp, spec, caller, self.objects.clone(), self.clock.clone()) {
            Ok(ws) => ws,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
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
