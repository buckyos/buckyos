//! A consistent read view of one Workspace for the duration of a wish stage (许愿格 §6.1): its
//! own read-only connection inside one read transaction (WAL), so every map line, profile, tool
//! answer and materialized file comes from the same point in history while the Workspace writer
//! keeps accepting edits. Grants are captured once, when the stage starts.

use crate::docdb::{db_err, meta_get, DocCache, SqlCtx};
use crate::objects::FsObjectStore;
use crate::workspace::{Caller, Workspace};
use aiworkspace_core::access::Access;
use aiworkspace_core::model::*;
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{WsError, WsResult};
use rusqlite::{Connection, OpenFlags};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

pub struct WishSnapshot {
    conn: Connection,
    cache: RefCell<DocCache>,
    now: String,
    pub access: Access,
    pub principal: String,
    pub workspace_id: String,
    pub epoch: String,
    /// The head the snapshot sees (locates it; never a conflict condition by itself, §6.3).
    pub head_seq: u64,
    pub ws_dir: PathBuf,
    pub objects: Arc<FsObjectStore>,
}

impl WishSnapshot {
    pub fn ctx(&self) -> SqlCtx<'_> {
        SqlCtx { conn: &self.conn, local: None, cache: &self.cache, now: &self.now }
    }

    pub fn entity(&self, id: &str) -> WsResult<Option<EntityRow>> {
        self.ctx().entity(id)
    }

    /// A live entity the caller may read.
    pub fn readable(&self, id: &str) -> WsResult<EntityRow> {
        let e = aiworkspace_core::read::readable_entity(&self.ctx(), &self.access, id)?;
        if presentation_only(&e) {
            return Err(WsError::not_found(format!("entity {id} not found")));
        }
        if !e.alive() {
            return Err(WsError::deleted(format!("entity {id} is deleted")));
        }
        Ok(e)
    }

    pub fn can_read(&self, e: &EntityRow) -> WsResult<bool> {
        Ok(!presentation_only(e) && self.access.can_read(&self.ctx(), e)?)
    }

    /// Verified bytes of an asset.
    pub fn asset_bytes(&self, object_id: &str) -> WsResult<Vec<u8>> {
        use aiworkspace_core::materialize::ObjectSource;
        self.objects.get_file(object_id)
    }
}

/// Presentation paths, Viewports and their folder are not part of what a wish sees (third phase §6.2); a Frame's
/// speaker notes are never read out either (the read tools list a Block's keys explicitly).
fn presentation_only(e: &EntityRow) -> bool {
    e.entity_id == SHOWS_ID || is_show_type(&e.type_id)
}

impl Workspace {
    /// Open a stage snapshot for `caller`. The grants are evaluated now; the document view is fixed by
    /// the first read inside the transaction.
    pub fn wish_snapshot(&self, caller: &Caller) -> WsResult<WishSnapshot> {
        let access = self.require_ws_any(caller)?;
        let conn = Connection::open_with_flags(self.dir.join("doc.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_err)?;
        conn.execute_batch("BEGIN DEFERRED").map_err(db_err)?;
        let head_seq: u64 = meta_get(&conn, "head_seq")?.and_then(|s| s.parse().ok()).unwrap_or(0);
        let epoch = meta_get(&conn, "epoch")?.unwrap_or_default();
        Ok(WishSnapshot {
            conn,
            cache: RefCell::new(DocCache::default()),
            now: format_utc_ms((self.clock)()),
            access,
            principal: caller.principal.clone(),
            workspace_id: self.workspace_id.clone(),
            epoch,
            head_seq,
            ws_dir: self.dir.clone(),
            objects: self.objects.clone(),
        })
    }
}
