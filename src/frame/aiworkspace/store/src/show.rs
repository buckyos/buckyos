//! Non-public shows (third phase §8): the show lock that closes a Workspace for document writes while it is
//! presented, and the temporary clone a show with operable Blocks writes into.
//!
//! The lock lives in memory, as the show itself (the relay is in memory too, §11.3): a service restart ends
//! every show, and with it the lock; the clones left behind are deleted when the service starts again.

use crate::docdb::{db_err, meta_get, meta_set};
use crate::service::Service;
use crate::workspace::{random_id, Workspace};
use aiworkspace_core::id::is_prefixed_id;
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A show holding a Workspace: nobody writes its document until `expires_ms` (renewed by the stage) or the end.
#[derive(Debug, Clone)]
pub struct ShowLock {
    pub show_id: String,
    pub principal: String,
    pub started_at: String,
    pub expires_ms: i64,
}

impl ShowLock {
    pub fn to_json(&self) -> Value {
        json!({ "show_id": self.show_id, "presenter": self.principal, "started_at": self.started_at })
    }
}

/// `workspace_id → lock`, shared by the service and every Workspace writer it opened.
pub type ShowLocks = Arc<Mutex<HashMap<String, ShowLock>>>;

/// `workspace_meta.purpose` of a show clone.
pub const PURPOSE_SHOW: &str = "show";

/// Run states a worker owns: a clone's copy of such a run has no worker (the original's keeps running).

impl Workspace {
    /// The show holding this Workspace now, if any.
    pub fn show_lock(&self) -> Option<ShowLock> {
        let locks = self.show_locks.as_ref()?.lock().unwrap();
        locks.get(&self.workspace_id).filter(|l| l.expires_ms > (self.clock)()).cloned()
    }

    pub(crate) fn show_locked_error(lock: &ShowLock) -> WsError {
        WsError::new(Code::ShowLocked, format!("the workspace is being presented by {} since {}: writes are closed until the show ends", lock.principal, lock.started_at))
            .with_data(lock.to_json())
    }

    /// `show` for a temporary show clone, `None` for an ordinary Workspace.
    pub fn purpose(&self) -> Option<String> {
        meta_get(&self.doc, "purpose").ok().flatten()
    }
}

/// Concurrent show clones of one Workspace (D17).
pub const MAX_CLONES: usize = 4;

/// Hard-link every file of `src` into `dst` (same names): the runs of a clone share their working files.
fn link_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            link_tree(&entry.path(), &to)?;
        } else if kind.is_file() {
            if std::fs::hard_link(entry.path(), &to).is_err() {
                std::fs::copy(entry.path(), &to)?;
            }
        }
    }
    Ok(())
}

fn io(e: std::io::Error) -> WsError {
    WsError::io(e.to_string())
}

impl Service {
    /// Put the show lock on `workspace_id`; a lock of another live show is refused with `SHOW_LOCKED`.
    pub fn show_lock(&self, workspace_id: &str, lock: ShowLock) -> WsResult<()> {
        let now = (self.clock)();
        let mut locks = self.show_locks.lock().unwrap();
        if let Some(held) = locks.get(workspace_id).filter(|l| l.expires_ms > now && l.show_id != lock.show_id) {
            return Err(Workspace::show_locked_error(held));
        }
        locks.insert(workspace_id.to_string(), lock);
        Ok(())
    }

    /// Extend the lock of `show_id` (nothing happens when it is not that show's any more).
    pub fn show_renew(&self, workspace_id: &str, show_id: &str, expires_ms: i64) {
        if let Some(l) = self.show_locks.lock().unwrap().get_mut(workspace_id).filter(|l| l.show_id == show_id) {
            l.expires_ms = expires_ms;
        }
    }

    pub fn show_unlock(&self, workspace_id: &str, show_id: &str) {
        let mut locks = self.show_locks.lock().unwrap();
        if locks.get(workspace_id).is_some_and(|l| l.show_id == show_id) {
            locks.remove(workspace_id);
        }
    }

    /// The temporary clone of a show (§8.3): both databases copied inside the source's writer, so the copy is one
    /// consistent state; entity ids, `seq` / epoch, rich text lineages, grants and wish runs are kept (run folders
    /// hard-linked), user state, locks and uploads are not. Objects are in the named store and the clone reads them
    /// through the source's bindings, sharing every asset, snapshot and program without copying a byte. Returns its id.
    pub fn clone_for_show(&self, source_id: &str, show_id: &str, expires_at: &str) -> WsResult<String> {
        let id = random_id("ws_");
        let tmp = self.tmp_dir();
        let built = (|| -> WsResult<()> {
            std::fs::create_dir_all(&tmp).map_err(io)?;
            {
                let handle = self.workspace(source_id)?;
                let ws = handle.lock().unwrap();
                // clones count on the source (D17): its quota, and at most MAX_CLONES at once
                ws.check_quota(0, false)?;
                let live = self.live_clones(&ws)?;
                if live.len() >= MAX_CLONES {
                    return Err(WsError::limit(format!("at most {MAX_CLONES} shows of this workspace may run at once")));
                }
                for name in ["doc.sqlite", "local.sqlite"] {
                    let conn = if name == "doc.sqlite" { &ws.doc } else { &ws.local };
                    conn.execute("VACUUM INTO ?1", [tmp.join(name).to_string_lossy().as_ref()]).map_err(db_err)?;
                }
                link_tree(&ws.dir.join("runs"), &tmp.join("runs")).map_err(io)?;
                link_tree(&ws.dir.join("cache"), &tmp.join("cache")).map_err(io)?;
                // the clone reads the source's current content for the show (D06); it counts on the source (D17)
                let mut clones = live;
                clones.push(id.clone());
                ws.local.execute("INSERT OR REPLACE INTO local_meta (key, value) VALUES ('show_clones', ?1)", [json!(clones).to_string()]).map_err(db_err)?;
                let bytes = std::fs::metadata(tmp.join("doc.sqlite")).map(|m| m.len()).unwrap_or(0) as i64;
                ws.add_usage("clones", bytes)?;
            }
            let doc = Connection::open(tmp.join("doc.sqlite")).map_err(db_err)?;
            meta_set(&doc, "workspace_id", &id)?;
            meta_set(&doc, "purpose", PURPOSE_SHOW)?;
            meta_set(&doc, "show_source", source_id)?;
            meta_set(&doc, "show_id", show_id)?;
            meta_set(&doc, "show_expires_at", expires_at)?;
            drop(doc);
            let local = Connection::open(tmp.join("local.sqlite")).map_err(db_err)?;
            local
                .execute_batch(&format!(
                    "DELETE FROM locks; DELETE FROM lock_events; DELETE FROM staged_assets; DELETE FROM user_state; DELETE FROM local_meta WHERE key = 'show_clones'; \
                     UPDATE runs SET state = 'interrupted' WHERE state IN ({});", crate::wish::runs::active_states_sql()
                ))
                .map_err(db_err)?;
            Ok(())
        })();
        if let Err(e) = built {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
        std::fs::rename(&tmp, self.ws_dir(&id)).map_err(io)?;
        Ok(id)
    }

    /// Delete a show clone (never anything else): it is closed and its folder removed.
    pub fn drop_clone(&self, id: &str) -> WsResult<()> {
        if !is_clone_dir(&self.ws_dir(id)) {
            return Err(WsError::invalid_op(format!("{id} is not a show clone")));
        }
        // what the clone bound itself goes; the source's clone usage with it
        if let Err(e) = self.release_folder(&self.ws_dir(id)) {
            log::warn!("release bindings of show clone {id}: {e}");
        }
        self.close(id);
        if let Ok(doc) = Connection::open_with_flags(self.ws_dir(id).join("doc.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX) {
            if let Ok(Some(source)) = meta_get(&doc, "show_source") {
                let bytes = std::fs::metadata(self.ws_dir(id).join("doc.sqlite")).map(|m| m.len()).unwrap_or(0) as i64;
                drop(doc);
                if let Ok(h) = self.workspace(&source) {
                    let ws = h.lock().unwrap();
                    let _ = ws.add_usage("clones", -bytes);
                }
            }
        }
        std::fs::remove_dir_all(self.ws_dir(id)).map_err(io)?;
        // a request that opened it again meanwhile must not keep a writer of the removed folder
        self.close(id);
        Ok(())
    }

    /// The source's show clones whose folders still exist.
    fn live_clones(&self, ws: &Workspace) -> WsResult<Vec<String>> {
        let list: Option<String> = ws.local.query_row("SELECT value FROM local_meta WHERE key = 'show_clones'", [], |r| r.get(0)).optional().map_err(db_err)?;
        let ids: Vec<String> = list.and_then(|l| serde_json::from_str(&l).ok()).unwrap_or_default();
        Ok(ids.into_iter().filter(|id| is_prefixed_id("ws_", id) && self.ws_dir(id).exists()).collect())
    }

    /// Clones a crashed or restarted service left behind (their shows ended with it).
    pub(crate) fn drop_stale_clones(&self) -> WsResult<()> {
        for entry in std::fs::read_dir(self.data_dir.join("workspaces")).map_err(io)? {
            let path = entry.map_err(io)?.path();
            if is_clone_dir(&path) {
                log::info!("removing the clone of an ended show: {}", path.display());
                self.release_folder(&path)?;
                let _ = std::fs::remove_dir_all(&path);
            }
        }
        Ok(())
    }
}

/// Is the Workspace folder at `dir` a show clone (read without opening a writer)?
pub fn is_clone_dir(dir: &Path) -> bool {
    let Ok(conn) = Connection::open_with_flags(dir.join("doc.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return false };
    meta_get(&conn, "purpose").ok().flatten().as_deref() == Some(PURPOSE_SHOW)
}
