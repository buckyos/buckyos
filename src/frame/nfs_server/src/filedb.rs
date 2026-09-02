//! filedb: the persistent truth source for virtual Nodes / Bindings and anchors
//! (nfs_server.md §3.2). Design philosophy: filedb only stores what the local
//! filesystem cannot express. Entity files/dirs get rows lazily (惰性锚定).
//!
//! Persistent tables: entities, anchors, namespace_bindings, views, view_groups,
//! view_base, view_patch, collections, collection_nodes, meta_records, grants.
//! Cache tables (rebuildable, may be lost): content_index.

use crate::error::{internal, not_found, NfsResult};
use crate::fsutil::unix_now;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

pub struct FileDb {
    conn: Mutex<Connection>,
}

// ---------- row types ----------

#[derive(Debug, Clone)]
pub struct Entity {
    pub node_id: i64,
    pub gen: u64,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub struct Anchor {
    pub node_id: i64,
    pub root_id: String,
    pub ino: u64,
    pub dev: u64,
    pub path: String,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub full_hash: Option<String>,
    pub state: String, // ok | stale | deleted
}

#[derive(Debug, Clone)]
pub struct BindingRow {
    pub entry_id: i64,
    pub parent_node_id: i64,
    pub name: String,
    pub target_type: String, // live | object
    pub target_node_id: Option<i64>,
    pub target_obj_id: Option<String>,
    pub binding_type: String,
    pub state: String, // ok | conflict | stale
}

#[derive(Debug, Clone)]
pub struct ViewRow {
    pub node_id: i64,
    pub view_id: String,
    pub origin: String,
    pub title: String,
    pub generation: i64,
    pub stale: bool,
}

#[derive(Debug, Clone)]
pub struct ViewGroupRow {
    pub node_id: i64,
    pub view_node_id: i64,
    pub label: String,
    pub by_dim: Option<String>,
    pub ord: i64,
}

#[derive(Debug, Clone)]
pub struct ViewMemberRow {
    pub id: i64,
    pub group_node_id: Option<i64>,
    pub target_type: String,
    pub target_node_id: Option<i64>,
    pub target_obj_id: Option<String>,
    pub provenance: Option<serde_json::Value>,
    pub ord: i64,
    pub pinned: bool,
    /// "base" | "patch"
    pub layer: &'static str,
}

#[derive(Debug, Clone)]
pub struct CollectionRow {
    pub node_id: i64,
    pub collection_id: String,
    pub title: String,
    pub description: Option<String>,
    pub generation: i64,
}

#[derive(Debug, Clone)]
pub struct CollectionEntryRow {
    pub entry_id: i64,
    pub collection_node_id: i64,
    pub parent_group_node_id: Option<i64>,
    pub node_type: String, // ref | group
    pub target_type: Option<String>,
    pub target_node_id: Option<i64>,
    pub target_obj_id: Option<String>,
    pub name: Option<String>,
    pub manual_order: i64,
}

#[derive(Debug, Clone)]
pub struct MetaRow {
    pub anchor: String,
    pub ns: String,
    pub key: String,
    pub value: serde_json::Value,
    pub source: serde_json::Value,
    pub confidence: Option<f64>,
    pub visibility: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct GrantRow {
    pub cap_id: String,
    pub subtree: String,
    pub ops: Vec<String>,
    pub audience: Option<String>,
    pub expires_at: Option<i64>,
    pub revoked: bool,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS kv(
  k TEXT PRIMARY KEY, v TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entities(
  node_id INTEGER PRIMARY KEY AUTOINCREMENT,
  gen INTEGER NOT NULL DEFAULT 1,
  kind TEXT NOT NULL,
  owner TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS anchors(
  node_id INTEGER PRIMARY KEY REFERENCES entities(node_id),
  root_id TEXT NOT NULL,
  ino INTEGER NOT NULL,
  dev INTEGER NOT NULL DEFAULT 0,
  path TEXT NOT NULL,
  size INTEGER,
  mtime INTEGER,
  qcid TEXT,
  full_hash TEXT,
  state TEXT NOT NULL DEFAULT 'ok'
);
CREATE INDEX IF NOT EXISTS idx_anchors_path ON anchors(root_id, path);
CREATE INDEX IF NOT EXISTS idx_anchors_ino ON anchors(root_id, ino);
CREATE TABLE IF NOT EXISTS namespace_bindings(
  entry_id INTEGER PRIMARY KEY AUTOINCREMENT,
  parent_node_id INTEGER NOT NULL REFERENCES entities(node_id),
  name TEXT NOT NULL,
  target_type TEXT NOT NULL,
  target_node_id INTEGER,
  target_obj_id TEXT,
  binding_type TEXT NOT NULL DEFAULT 'reference',
  state TEXT NOT NULL DEFAULT 'ok',
  created_at INTEGER NOT NULL,
  UNIQUE(parent_node_id, name)
);
CREATE TABLE IF NOT EXISTS views(
  node_id INTEGER PRIMARY KEY REFERENCES entities(node_id),
  view_id TEXT UNIQUE NOT NULL,
  origin TEXT NOT NULL DEFAULT 'auto',
  title TEXT NOT NULL,
  generation INTEGER NOT NULL DEFAULT 1,
  stale INTEGER NOT NULL DEFAULT 0,
  query_json TEXT
);
CREATE TABLE IF NOT EXISTS view_groups(
  node_id INTEGER PRIMARY KEY REFERENCES entities(node_id),
  view_node_id INTEGER NOT NULL REFERENCES views(node_id),
  label TEXT NOT NULL,
  by_dim TEXT,
  ord INTEGER NOT NULL DEFAULT 0,
  UNIQUE(view_node_id, label)
);
CREATE TABLE IF NOT EXISTS view_base(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  view_node_id INTEGER NOT NULL,
  group_node_id INTEGER,
  target_type TEXT NOT NULL,
  target_node_id INTEGER,
  target_obj_id TEXT,
  provenance_json TEXT,
  ord INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_view_base ON view_base(view_node_id);
CREATE TABLE IF NOT EXISTS view_patch(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  view_node_id INTEGER NOT NULL,
  op TEXT NOT NULL,
  target_type TEXT,
  target_node_id INTEGER,
  target_obj_id TEXT,
  base_id INTEGER,
  ord INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS collections(
  node_id INTEGER PRIMARY KEY REFERENCES entities(node_id),
  collection_id TEXT UNIQUE NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  generation INTEGER NOT NULL DEFAULT 1,
  owner TEXT
);
CREATE TABLE IF NOT EXISTS collection_nodes(
  entry_id INTEGER PRIMARY KEY AUTOINCREMENT,
  collection_node_id INTEGER NOT NULL REFERENCES collections(node_id),
  parent_group_node_id INTEGER,
  node_type TEXT NOT NULL,
  target_type TEXT,
  target_node_id INTEGER,
  target_obj_id TEXT,
  name TEXT,
  manual_order INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_coll_nodes ON collection_nodes(collection_node_id, parent_group_node_id);
CREATE TABLE IF NOT EXISTS meta_records(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  anchor TEXT NOT NULL,
  ns TEXT NOT NULL,
  key TEXT NOT NULL,
  value_json TEXT NOT NULL,
  source_json TEXT NOT NULL,
  confidence REAL,
  visibility TEXT NOT NULL DEFAULT 'private',
  updated_at INTEGER NOT NULL,
  UNIQUE(anchor, ns, key)
);
CREATE INDEX IF NOT EXISTS idx_meta_anchor ON meta_records(anchor);
CREATE TABLE IF NOT EXISTS grants(
  cap_id TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL,
  subtree TEXT NOT NULL,
  ops_json TEXT NOT NULL,
  audience TEXT,
  expires_at INTEGER,
  max_uses INTEGER,
  revoked INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS content_index(
  hash TEXT PRIMARY KEY,
  root_id TEXT NOT NULL,
  path TEXT NOT NULL,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
"#;

impl FileDb {
    pub fn open(path: &Path) -> NfsResult<FileDb> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(FileDb { conn: Mutex::new(conn) })
    }

    pub fn open_memory() -> NfsResult<FileDb> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(FileDb { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- kv ----------

    pub fn kv_get(&self, k: &str) -> NfsResult<Option<String>> {
        let conn = self.lock();
        Ok(conn
            .query_row("SELECT v FROM kv WHERE k=?1", params![k], |r| r.get(0))
            .optional()?)
    }

    pub fn kv_set(&self, k: &str, v: &str) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=?2",
            params![k, v],
        )?;
        Ok(())
    }

    /// Returns the persistent handle-signing key, creating it on first use.
    pub fn handle_key(&self) -> NfsResult<Vec<u8>> {
        if let Some(hexkey) = self.kv_get("handle_key")? {
            return hex::decode(&hexkey).map_err(|e| internal(format!("bad handle_key: {}", e)));
        }
        let key: [u8; 32] = rand::random();
        self.kv_set("handle_key", &hex::encode(key))?;
        Ok(key.to_vec())
    }

    // ---------- entities ----------

    pub fn entity_get(&self, node_id: i64) -> NfsResult<Option<Entity>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT node_id, gen, kind FROM entities WHERE node_id=?1",
                params![node_id],
                |r| {
                    Ok(Entity {
                        node_id: r.get(0)?,
                        gen: r.get::<_, i64>(1)? as u64,
                        kind: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    fn entity_create_conn(conn: &Connection, kind: &str) -> NfsResult<i64> {
        conn.execute(
            "INSERT INTO entities(kind, created_at) VALUES(?1, ?2)",
            params![kind, unix_now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn entity_create(&self, kind: &str) -> NfsResult<i64> {
        Self::entity_create_conn(&self.lock(), kind)
    }

    // ---------- anchors ----------

    fn row_anchor(r: &rusqlite::Row<'_>) -> rusqlite::Result<Anchor> {
        Ok(Anchor {
            node_id: r.get(0)?,
            root_id: r.get(1)?,
            ino: r.get::<_, i64>(2)? as u64,
            dev: r.get::<_, i64>(3)? as u64,
            path: r.get(4)?,
            size: r.get(5)?,
            mtime: r.get(6)?,
            full_hash: r.get(7)?,
            state: r.get(8)?,
        })
    }

    const ANCHOR_COLS: &'static str =
        "node_id, root_id, ino, dev, path, size, mtime, full_hash, state";

    pub fn anchor_get(&self, node_id: i64) -> NfsResult<Option<Anchor>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!("SELECT {} FROM anchors WHERE node_id=?1", Self::ANCHOR_COLS),
                params![node_id],
                Self::row_anchor,
            )
            .optional()?)
    }

    pub fn anchor_get_by_path(&self, root: &str, rel: &str) -> NfsResult<Option<Anchor>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {} FROM anchors WHERE root_id=?1 AND path=?2 AND state != 'deleted'",
                    Self::ANCHOR_COLS
                ),
                params![root, rel],
                Self::row_anchor,
            )
            .optional()?)
    }

    pub fn anchors_for_root(&self, root: &str) -> NfsResult<Vec<Anchor>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM anchors WHERE root_id=?1 AND state != 'deleted'",
            Self::ANCHOR_COLS
        ))?;
        let rows = stmt.query_map(params![root], Self::row_anchor)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Batch lookup of anchors for entries of one directory (avoids N+1 in list).
    pub fn anchors_by_paths(&self, root: &str, rels: &[String]) -> NfsResult<Vec<Anchor>> {
        if rels.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let placeholders = vec!["?"; rels.len()].join(",");
        let sql = format!(
            "SELECT {} FROM anchors WHERE root_id=? AND state != 'deleted' AND path IN ({})",
            Self::ANCHOR_COLS,
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![&root];
        for r in rels {
            params_vec.push(r);
        }
        let rows = stmt.query_map(params_vec.as_slice(), Self::row_anchor)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Lazy anchoring: returns the node_id for a native file/dir, creating the
    /// entity+anchor rows on first need. If an anchor exists at this path with a
    /// different native id, it is rebound (editor-overwrite heuristic §3.6).
    pub fn ensure_anchor(
        &self,
        kind: &str,
        root: &str,
        rel: &str,
        ino: u64,
        dev: u64,
        size: i64,
        mtime: i64,
    ) -> NfsResult<Entity> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(|e| internal(e.to_string()))?;
        let existing = tx
            .query_row(
                &format!(
                    "SELECT {} FROM anchors WHERE root_id=?1 AND path=?2 AND state != 'deleted'",
                    Self::ANCHOR_COLS
                ),
                params![root, rel],
                Self::row_anchor,
            )
            .optional()?;
        let ent = if let Some(a) = existing {
            if a.ino != ino || a.state != "ok" {
                tx.execute(
                    "UPDATE anchors SET ino=?1, dev=?2, size=?3, mtime=?4, full_hash=NULL, state='ok' WHERE node_id=?5",
                    params![ino as i64, dev as i64, size, mtime, a.node_id],
                )?;
            } else if a.size != Some(size) || a.mtime != Some(mtime) {
                tx.execute(
                    "UPDATE anchors SET size=?1, mtime=?2, full_hash=NULL WHERE node_id=?3",
                    params![size, mtime, a.node_id],
                )?;
            }
            tx.query_row(
                "SELECT node_id, gen, kind FROM entities WHERE node_id=?1",
                params![a.node_id],
                |r| {
                    Ok(Entity {
                        node_id: r.get(0)?,
                        gen: r.get::<_, i64>(1)? as u64,
                        kind: r.get(2)?,
                    })
                },
            )?
        } else {
            let node_id = Self::entity_create_conn(&tx, kind)?;
            tx.execute(
                "INSERT INTO anchors(node_id, root_id, ino, dev, path, size, mtime) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![node_id, root, ino as i64, dev as i64, rel, size, mtime],
            )?;
            Entity { node_id, gen: 1, kind: kind.to_string() }
        };
        tx.commit().map_err(|e| internal(e.to_string()))?;
        Ok(ent)
    }

    pub fn anchor_update_path(&self, node_id: i64, root: &str, rel: &str) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE anchors SET root_id=?1, path=?2, state='ok' WHERE node_id=?3",
            params![root, rel, node_id],
        )?;
        Ok(())
    }

    pub fn anchor_rebind(
        &self,
        node_id: i64,
        ino: u64,
        dev: u64,
        size: i64,
        mtime: i64,
    ) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE anchors SET ino=?1, dev=?2, size=?3, mtime=?4, full_hash=NULL, state='ok' WHERE node_id=?5",
            params![ino as i64, dev as i64, size, mtime, node_id],
        )?;
        Ok(())
    }

    pub fn anchor_touch(&self, node_id: i64, size: i64, mtime: i64, clear_hash: bool) -> NfsResult<()> {
        let conn = self.lock();
        if clear_hash {
            conn.execute(
                "UPDATE anchors SET size=?1, mtime=?2, full_hash=NULL WHERE node_id=?3",
                params![size, mtime, node_id],
            )?;
        } else {
            conn.execute(
                "UPDATE anchors SET size=?1, mtime=?2 WHERE node_id=?3",
                params![size, mtime, node_id],
            )?;
        }
        Ok(())
    }

    pub fn anchor_set_hash(&self, node_id: i64, size: i64, mtime: i64, hash: &str) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE anchors SET size=?1, mtime=?2, full_hash=?3, state='ok' WHERE node_id=?4",
            params![size, mtime, hash, node_id],
        )?;
        Ok(())
    }

    pub fn anchor_set_state(&self, node_id: i64, state: &str) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute("UPDATE anchors SET state=?1 WHERE node_id=?2", params![state, node_id])?;
        // Bindings pointing at a dead target become stale (honest, not silent).
        if state == "stale" || state == "deleted" {
            conn.execute(
                "UPDATE namespace_bindings SET state='stale' WHERE target_type='live' AND target_node_id=?1 AND state='ok'",
                params![node_id],
            )?;
        }
        Ok(())
    }

    /// Rewrites anchor paths under a moved directory (protocol `move`).
    pub fn anchors_move_prefix(
        &self,
        root: &str,
        old_rel: &str,
        new_root: &str,
        new_rel: &str,
    ) -> NfsResult<usize> {
        let conn = self.lock();
        let like = format!("{}/%", old_rel.replace('%', "\\%").replace('_', "\\_"));
        let n = conn.execute(
            "UPDATE anchors SET root_id=?1, path = ?2 || substr(path, ?3) WHERE root_id=?4 AND path LIKE ?5 ESCAPE '\\' AND state != 'deleted'",
            params![new_root, new_rel, old_rel.len() as i64 + 1, root, like],
        )?;
        let m = conn.execute(
            "UPDATE anchors SET root_id=?1, path=?2 WHERE root_id=?3 AND path=?4 AND state != 'deleted'",
            params![new_root, new_rel, root, old_rel],
        )?;
        Ok(n + m)
    }

    /// Marks anchors under a deleted subtree as deleted; returns affected node ids.
    pub fn anchors_delete_subtree(&self, root: &str, rel: &str) -> NfsResult<Vec<i64>> {
        let conn = self.lock();
        let like = format!("{}/%", rel.replace('%', "\\%").replace('_', "\\_"));
        let mut ids: Vec<i64> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT node_id FROM anchors WHERE root_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\') AND state != 'deleted'",
            )?;
            let rows = stmt.query_map(params![root, rel, like], |r| r.get::<_, i64>(0))?;
            for r in rows {
                ids.push(r?);
            }
        }
        conn.execute(
            "UPDATE anchors SET state='deleted' WHERE root_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\')",
            params![root, rel, like],
        )?;
        for id in &ids {
            conn.execute(
                "UPDATE namespace_bindings SET state='stale' WHERE target_type='live' AND target_node_id=?1 AND state='ok'",
                params![id],
            )?;
            // Entries owned by a destroyed dir are destroyed with it (targets untouched).
            conn.execute("DELETE FROM namespace_bindings WHERE parent_node_id=?1", params![id])?;
        }
        Ok(ids)
    }

    // ---------- namespace bindings ----------

    fn row_binding(r: &rusqlite::Row<'_>) -> rusqlite::Result<BindingRow> {
        Ok(BindingRow {
            entry_id: r.get(0)?,
            parent_node_id: r.get(1)?,
            name: r.get(2)?,
            target_type: r.get(3)?,
            target_node_id: r.get(4)?,
            target_obj_id: r.get(5)?,
            binding_type: r.get(6)?,
            state: r.get(7)?,
        })
    }

    const BINDING_COLS: &'static str =
        "entry_id, parent_node_id, name, target_type, target_node_id, target_obj_id, binding_type, state";

    pub fn binding_create(
        &self,
        parent_node_id: i64,
        name: &str,
        target_type: &str,
        target_node_id: Option<i64>,
        target_obj_id: Option<&str>,
    ) -> NfsResult<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO namespace_bindings(parent_node_id, name, target_type, target_node_id, target_obj_id, binding_type, created_at)
             VALUES(?1,?2,?3,?4,?5,'reference',?6)",
            params![parent_node_id, name, target_type, target_node_id, target_obj_id, unix_now()],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                crate::error::NfsError::new(
                    crate::error::ErrorCode::NamespaceConflict,
                    format!("binding name '{}' already exists", name),
                )
            }
            e => internal(format!("filedb: {}", e)),
        })?;
        Ok(conn.last_insert_rowid())
    }

    pub fn binding_get(&self, entry_id: i64) -> NfsResult<Option<BindingRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!("SELECT {} FROM namespace_bindings WHERE entry_id=?1", Self::BINDING_COLS),
                params![entry_id],
                Self::row_binding,
            )
            .optional()?)
    }

    pub fn bindings_by_parent(&self, parent_node_id: i64) -> NfsResult<Vec<BindingRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM namespace_bindings WHERE parent_node_id=?1 ORDER BY name",
            Self::BINDING_COLS
        ))?;
        let rows = stmt.query_map(params![parent_node_id], Self::row_binding)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn bindings_by_target(&self, target_node_id: i64) -> NfsResult<Vec<BindingRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM namespace_bindings WHERE target_type='live' AND target_node_id=?1",
            Self::BINDING_COLS
        ))?;
        let rows = stmt.query_map(params![target_node_id], Self::row_binding)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn binding_delete(&self, entry_id: i64) -> NfsResult<bool> {
        let conn = self.lock();
        let n = conn.execute("DELETE FROM namespace_bindings WHERE entry_id=?1", params![entry_id])?;
        Ok(n > 0)
    }

    // ---------- views ----------

    fn row_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<ViewRow> {
        Ok(ViewRow {
            node_id: r.get(0)?,
            view_id: r.get(1)?,
            origin: r.get(2)?,
            title: r.get(3)?,
            generation: r.get(4)?,
            stale: r.get::<_, i64>(5)? != 0,
        })
    }

    const VIEW_COLS: &'static str = "node_id, view_id, origin, title, generation, stale";

    pub fn view_create(&self, view_id: &str, title: &str, origin: &str) -> NfsResult<ViewRow> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(|e| internal(e.to_string()))?;
        let node_id = Self::entity_create_conn(&tx, "view")?;
        tx.execute(
            "INSERT INTO views(node_id, view_id, origin, title) VALUES(?1,?2,?3,?4)",
            params![node_id, view_id, origin, title],
        )?;
        tx.commit().map_err(|e| internal(e.to_string()))?;
        Ok(ViewRow {
            node_id,
            view_id: view_id.to_string(),
            origin: origin.to_string(),
            title: title.to_string(),
            generation: 1,
            stale: false,
        })
    }

    pub fn view_by_id(&self, view_id: &str) -> NfsResult<Option<ViewRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!("SELECT {} FROM views WHERE view_id=?1", Self::VIEW_COLS),
                params![view_id],
                Self::row_view,
            )
            .optional()?)
    }

    pub fn view_by_node(&self, node_id: i64) -> NfsResult<Option<ViewRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!("SELECT {} FROM views WHERE node_id=?1", Self::VIEW_COLS),
                params![node_id],
                Self::row_view,
            )
            .optional()?)
    }

    pub fn view_group_create(
        &self,
        view_node_id: i64,
        label: &str,
        by_dim: Option<&str>,
        ord: i64,
    ) -> NfsResult<i64> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(|e| internal(e.to_string()))?;
        let node_id = Self::entity_create_conn(&tx, "group")?;
        tx.execute(
            "INSERT INTO view_groups(node_id, view_node_id, label, by_dim, ord) VALUES(?1,?2,?3,?4,?5)",
            params![node_id, view_node_id, label, by_dim, ord],
        )?;
        tx.commit().map_err(|e| internal(e.to_string()))?;
        Ok(node_id)
    }

    pub fn view_groups(&self, view_node_id: i64) -> NfsResult<Vec<ViewGroupRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT node_id, view_node_id, label, by_dim, ord FROM view_groups WHERE view_node_id=?1 ORDER BY ord, node_id",
        )?;
        let rows = stmt.query_map(params![view_node_id], |r| {
            Ok(ViewGroupRow {
                node_id: r.get(0)?,
                view_node_id: r.get(1)?,
                label: r.get(2)?,
                by_dim: r.get(3)?,
                ord: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn view_group_get(&self, group_node_id: i64) -> NfsResult<Option<ViewGroupRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT node_id, view_node_id, label, by_dim, ord FROM view_groups WHERE node_id=?1",
                params![group_node_id],
                |r| {
                    Ok(ViewGroupRow {
                        node_id: r.get(0)?,
                        view_node_id: r.get(1)?,
                        label: r.get(2)?,
                        by_dim: r.get(3)?,
                        ord: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn view_base_add(
        &self,
        view_node_id: i64,
        group_node_id: Option<i64>,
        target_node_id: i64,
        provenance: Option<&serde_json::Value>,
        ord: i64,
    ) -> NfsResult<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO view_base(view_node_id, group_node_id, target_type, target_node_id, provenance_json, ord)
             VALUES(?1,?2,'live',?3,?4,?5)",
            params![view_node_id, group_node_id, target_node_id, provenance.map(|v| v.to_string()), ord],
        )?;
        conn.execute("UPDATE views SET generation = generation + 1 WHERE node_id=?1", params![view_node_id])?;
        Ok(conn.last_insert_rowid())
    }

    /// Base + Upper merge (NFSP §3.5 / I2): base minus removed, plus patch adds,
    /// pinned first, then by (ord, id).
    pub fn view_members(
        &self,
        view_node_id: i64,
        group_node_id: Option<i64>,
    ) -> NfsResult<Vec<ViewMemberRow>> {
        let conn = self.lock();
        let mut members: Vec<ViewMemberRow> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, group_node_id, target_type, target_node_id, target_obj_id, provenance_json, ord FROM view_base
                 WHERE view_node_id=?1 AND (?2 IS NULL AND group_node_id IS NULL OR group_node_id=?2)
                 ORDER BY ord, id",
            )?;
            let rows = stmt.query_map(params![view_node_id, group_node_id], |r| {
                Ok(ViewMemberRow {
                    id: r.get(0)?,
                    group_node_id: r.get(1)?,
                    target_type: r.get(2)?,
                    target_node_id: r.get(3)?,
                    target_obj_id: r.get(4)?,
                    provenance: r
                        .get::<_, Option<String>>(5)?
                        .and_then(|s| serde_json::from_str(&s).ok()),
                    ord: r.get(6)?,
                    pinned: false,
                    layer: "base",
                })
            })?;
            for r in rows {
                members.push(r?);
            }
        }
        // Apply patch layer: removes, pins, adds (adds only at top level).
        let mut removed: Vec<i64> = Vec::new();
        let mut pinned: Vec<i64> = Vec::new();
        let mut adds: Vec<ViewMemberRow> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, op, target_type, target_node_id, target_obj_id, base_id, ord FROM view_patch WHERE view_node_id=?1 ORDER BY id",
            )?;
            let rows = stmt.query_map(params![view_node_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })?;
            for r in rows {
                let (id, op, tt, tn, to, base_id, ord) = r?;
                match op.as_str() {
                    "remove" => {
                        if let Some(b) = base_id {
                            removed.push(b)
                        }
                    }
                    "pin" => {
                        if let Some(b) = base_id {
                            pinned.push(b)
                        }
                    }
                    "add" if group_node_id.is_none() => adds.push(ViewMemberRow {
                        id,
                        group_node_id: None,
                        target_type: tt.unwrap_or_else(|| "live".into()),
                        target_node_id: tn,
                        target_obj_id: to,
                        provenance: None,
                        ord,
                        pinned: false,
                        layer: "patch",
                    }),
                    _ => {}
                }
            }
        }
        members.retain(|m| !removed.contains(&m.id));
        for m in members.iter_mut() {
            if pinned.contains(&m.id) {
                m.pinned = true;
            }
        }
        members.extend(adds);
        members.sort_by_key(|m| (!m.pinned, m.ord, m.id));
        Ok(members)
    }

    // ---------- collections ----------

    fn row_collection(r: &rusqlite::Row<'_>) -> rusqlite::Result<CollectionRow> {
        Ok(CollectionRow {
            node_id: r.get(0)?,
            collection_id: r.get(1)?,
            title: r.get(2)?,
            description: r.get(3)?,
            generation: r.get(4)?,
        })
    }

    const COLLECTION_COLS: &'static str = "node_id, collection_id, title, description, generation";

    pub fn collection_create(&self, collection_id: &str, title: &str) -> NfsResult<CollectionRow> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(|e| internal(e.to_string()))?;
        let node_id = Self::entity_create_conn(&tx, "collection")?;
        tx.execute(
            "INSERT INTO collections(node_id, collection_id, title) VALUES(?1,?2,?3)",
            params![node_id, collection_id, title],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                crate::error::NfsError::new(
                    crate::error::ErrorCode::NamespaceConflict,
                    format!("collection_id '{}' already exists", collection_id),
                )
            }
            e => internal(format!("filedb: {}", e)),
        })?;
        tx.commit().map_err(|e| internal(e.to_string()))?;
        Ok(CollectionRow {
            node_id,
            collection_id: collection_id.to_string(),
            title: title.to_string(),
            description: None,
            generation: 1,
        })
    }

    pub fn collection_by_id(&self, collection_id: &str) -> NfsResult<Option<CollectionRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!("SELECT {} FROM collections WHERE collection_id=?1", Self::COLLECTION_COLS),
                params![collection_id],
                Self::row_collection,
            )
            .optional()?)
    }

    pub fn collection_by_node(&self, node_id: i64) -> NfsResult<Option<CollectionRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!("SELECT {} FROM collections WHERE node_id=?1", Self::COLLECTION_COLS),
                params![node_id],
                Self::row_collection,
            )
            .optional()?)
    }

    fn row_coll_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<CollectionEntryRow> {
        Ok(CollectionEntryRow {
            entry_id: r.get(0)?,
            collection_node_id: r.get(1)?,
            parent_group_node_id: r.get(2)?,
            node_type: r.get(3)?,
            target_type: r.get(4)?,
            target_node_id: r.get(5)?,
            target_obj_id: r.get(6)?,
            name: r.get(7)?,
            manual_order: r.get(8)?,
        })
    }

    const COLL_ENTRY_COLS: &'static str =
        "entry_id, collection_node_id, parent_group_node_id, node_type, target_type, target_node_id, target_obj_id, name, manual_order";

    pub fn collection_entries(
        &self,
        collection_node_id: i64,
        parent_group: Option<i64>,
    ) -> NfsResult<Vec<CollectionEntryRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM collection_nodes WHERE collection_node_id=?1
             AND (?2 IS NULL AND parent_group_node_id IS NULL OR parent_group_node_id=?2)
             ORDER BY manual_order, entry_id",
            Self::COLL_ENTRY_COLS
        ))?;
        let rows = stmt.query_map(params![collection_node_id, parent_group], Self::row_coll_entry)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn collection_entry_get(&self, entry_id: i64) -> NfsResult<Option<CollectionEntryRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                &format!("SELECT {} FROM collection_nodes WHERE entry_id=?1", Self::COLL_ENTRY_COLS),
                params![entry_id],
                Self::row_coll_entry,
            )
            .optional()?)
    }

    /// Finds which collection a group entity belongs to (for list(group_ref)).
    pub fn collection_group_home(&self, group_node_id: i64) -> NfsResult<Option<i64>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT collection_node_id FROM collection_nodes WHERE node_type='group' AND target_node_id=?1",
                params![group_node_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Applies collection_patch ops atomically; returns the new generation.
    /// `ops` are pre-validated closures over the tx to keep SQL in one place.
    pub fn collection_patch(
        &self,
        collection_node_id: i64,
        expected_generation: Option<i64>,
        ops: &[CollectionPatchOp],
    ) -> NfsResult<i64> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(|e| internal(e.to_string()))?;
        let current: i64 = tx
            .query_row(
                "SELECT generation FROM collections WHERE node_id=?1",
                params![collection_node_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| not_found("collection not found"))?;
        if let Some(exp) = expected_generation {
            if exp != current {
                return Err(crate::error::rev_mismatch(
                    &format!("g-{}", exp),
                    &format!("g-{}", current),
                ));
            }
        }
        for op in ops {
            Self::apply_collection_op(&tx, collection_node_id, op)?;
        }
        // Renumber manual_order densely per parent group.
        Self::renumber_collection(&tx, collection_node_id)?;
        tx.execute(
            "UPDATE collections SET generation = generation + 1 WHERE node_id=?1",
            params![collection_node_id],
        )?;
        tx.commit().map_err(|e| internal(e.to_string()))?;
        Ok(current + 1)
    }

    fn apply_collection_op(
        tx: &Connection,
        cid: i64,
        op: &CollectionPatchOp,
    ) -> NfsResult<()> {
        match op {
            CollectionPatchOp::AddRef {
                parent_group,
                target_type,
                target_node_id,
                target_obj_id,
                name,
                position,
            } => {
                let pos = Self::position_order(tx, cid, *parent_group, *position)?;
                tx.execute(
                    "INSERT INTO collection_nodes(collection_node_id, parent_group_node_id, node_type, target_type, target_node_id, target_obj_id, name, manual_order)
                     VALUES(?1,?2,'ref',?3,?4,?5,?6,?7)",
                    params![cid, parent_group, target_type, target_node_id, target_obj_id, name, pos],
                )?;
            }
            CollectionPatchOp::CreateGroup { name, position } => {
                let node_id = Self::entity_create_conn(tx, "group")?;
                let pos = Self::position_order(tx, cid, None, *position)?;
                tx.execute(
                    "INSERT INTO collection_nodes(collection_node_id, parent_group_node_id, node_type, target_type, target_node_id, name, manual_order)
                     VALUES(?1,NULL,'group','live',?2,?3,?4)",
                    params![cid, node_id, name, pos],
                )?;
            }
            CollectionPatchOp::RemoveEntry { entry_id } => {
                let row = tx
                    .query_row(
                        "SELECT node_type, target_node_id FROM collection_nodes WHERE entry_id=?1 AND collection_node_id=?2",
                        params![entry_id, cid],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
                    )
                    .optional()?
                    .ok_or_else(|| not_found(format!("entry ce_{} not found", entry_id)))?;
                if row.0 == "group" {
                    if let Some(gid) = row.1 {
                        // Unlink group members (targets untouched), then the group.
                        tx.execute(
                            "DELETE FROM collection_nodes WHERE collection_node_id=?1 AND parent_group_node_id=?2",
                            params![cid, gid],
                        )?;
                    }
                }
                tx.execute("DELETE FROM collection_nodes WHERE entry_id=?1", params![entry_id])?;
            }
            CollectionPatchOp::MoveEntries { entry_ids, to_index } => {
                // All moved entries must share a parent; `to_index` is the target
                // position in the final sibling list (clamped to its length).
                let mut parent: Option<Option<i64>> = None;
                for id in entry_ids {
                    let p: Option<Option<i64>> = tx
                        .query_row(
                            "SELECT parent_group_node_id FROM collection_nodes WHERE entry_id=?1 AND collection_node_id=?2",
                            params![id, cid],
                            |r| r.get(0),
                        )
                        .optional()?;
                    let p = p.ok_or_else(|| not_found(format!("entry ce_{} not found", id)))?;
                    match &parent {
                        None => parent = Some(p),
                        Some(prev) if *prev == p => {}
                        _ => {
                            return Err(crate::error::invalid(
                                "move_entries requires entries with the same parent",
                            ))
                        }
                    }
                }
                let parent = parent.unwrap_or(None);
                let mut siblings: Vec<i64> = {
                    let mut stmt = tx.prepare(
                        "SELECT entry_id FROM collection_nodes WHERE collection_node_id=?1
                         AND (?2 IS NULL AND parent_group_node_id IS NULL OR parent_group_node_id=?2)
                         ORDER BY manual_order, entry_id",
                    )?;
                    let rows = stmt.query_map(params![cid, parent], |r| r.get(0))?;
                    rows.collect::<Result<Vec<_>, _>>()?
                };
                siblings.retain(|id| !entry_ids.contains(id));
                let at = (*to_index).min(siblings.len());
                for (i, id) in entry_ids.iter().enumerate() {
                    siblings.insert(at + i, *id);
                }
                for (i, id) in siblings.iter().enumerate() {
                    tx.execute(
                        "UPDATE collection_nodes SET manual_order=?1 WHERE entry_id=?2",
                        params![i as i64, id],
                    )?;
                }
            }
            CollectionPatchOp::RenameGroup { entry_id, name } => {
                let n = tx.execute(
                    "UPDATE collection_nodes SET name=?1 WHERE entry_id=?2 AND collection_node_id=?3 AND node_type='group'",
                    params![name, entry_id, cid],
                )?;
                if n == 0 {
                    return Err(not_found(format!("group entry ce_{} not found", entry_id)));
                }
            }
        }
        Ok(())
    }

    fn position_order(
        tx: &Connection,
        cid: i64,
        parent: Option<i64>,
        position: Option<usize>,
    ) -> NfsResult<i64> {
        Ok(match position {
            // Doubling leaves odd slots free for insertion before renumbering.
            Some(p) => {
                tx.execute(
                    "UPDATE collection_nodes SET manual_order = manual_order * 2 + 2 WHERE collection_node_id=?1
                     AND (?2 IS NULL AND parent_group_node_id IS NULL OR parent_group_node_id=?2)
                     AND manual_order >= ?3",
                    params![cid, parent, p as i64],
                )?;
                tx.execute(
                    "UPDATE collection_nodes SET manual_order = manual_order * 2 WHERE collection_node_id=?1
                     AND (?2 IS NULL AND parent_group_node_id IS NULL OR parent_group_node_id=?2)
                     AND manual_order < ?3",
                    params![cid, parent, p as i64],
                )?;
                (p as i64) * 2 + 1
            }
            None => {
                let max: Option<i64> = tx.query_row(
                    "SELECT MAX(manual_order) FROM collection_nodes WHERE collection_node_id=?1
                     AND (?2 IS NULL AND parent_group_node_id IS NULL OR parent_group_node_id=?2)",
                    params![cid, parent],
                    |r| r.get(0),
                )?;
                max.unwrap_or(-1) + 1
            }
        })
    }

    fn renumber_collection(tx: &Connection, cid: i64) -> NfsResult<()> {
        let mut parents: Vec<Option<i64>> = vec![None];
        {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT parent_group_node_id FROM collection_nodes WHERE collection_node_id=?1 AND parent_group_node_id IS NOT NULL",
            )?;
            let rows = stmt.query_map(params![cid], |r| r.get::<_, Option<i64>>(0))?;
            for r in rows {
                parents.push(r?);
            }
        }
        for parent in parents {
            let ids: Vec<i64> = {
                let mut stmt = tx.prepare(
                    "SELECT entry_id FROM collection_nodes WHERE collection_node_id=?1
                     AND (?2 IS NULL AND parent_group_node_id IS NULL OR parent_group_node_id=?2)
                     ORDER BY manual_order, entry_id",
                )?;
                let rows = stmt.query_map(params![cid, parent], |r| r.get(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            for (i, id) in ids.iter().enumerate() {
                tx.execute(
                    "UPDATE collection_nodes SET manual_order=?1 WHERE entry_id=?2",
                    params![i as i64, id],
                )?;
            }
        }
        Ok(())
    }

    // ---------- meta ----------

    pub fn meta_set(
        &self,
        anchor: &str,
        ns: &str,
        key: &str,
        value: &serde_json::Value,
        source: &serde_json::Value,
        visibility: &str,
    ) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO meta_records(anchor, ns, key, value_json, source_json, visibility, updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(anchor, ns, key) DO UPDATE SET value_json=?4, source_json=?5, visibility=?6, updated_at=?7",
            params![anchor, ns, key, value.to_string(), source.to_string(), visibility, unix_now()],
        )?;
        Ok(())
    }

    /// ns filter supports exact names and trailing-wildcard patterns like "ai.*".
    pub fn meta_get(&self, anchors: &[String], ns_filter: &[String]) -> NfsResult<Vec<MetaRow>> {
        if anchors.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let placeholders = vec!["?"; anchors.len()].join(",");
        let sql = format!(
            "SELECT anchor, ns, key, value_json, source_json, confidence, visibility, updated_at
             FROM meta_records WHERE anchor IN ({}) ORDER BY ns, key",
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let params_vec: Vec<&dyn rusqlite::ToSql> =
            anchors.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params_vec.as_slice(), |r| {
            Ok(MetaRow {
                anchor: r.get(0)?,
                ns: r.get(1)?,
                key: r.get(2)?,
                value: serde_json::from_str(&r.get::<_, String>(3)?)
                    .unwrap_or(serde_json::Value::Null),
                source: serde_json::from_str(&r.get::<_, String>(4)?)
                    .unwrap_or(serde_json::Value::Null),
                confidence: r.get(5)?,
                visibility: r.get(6)?,
                updated_at: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            let row = r?;
            if ns_filter.is_empty() || ns_filter.iter().any(|f| ns_matches(f, &row.ns)) {
                out.push(row);
            }
        }
        Ok(out)
    }

    pub fn meta_summary(&self, anchors: &[String]) -> NfsResult<Vec<(String, i64)>> {
        if anchors.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let placeholders = vec!["?"; anchors.len()].join(",");
        let sql = format!(
            "SELECT ns, COUNT(*) FROM meta_records WHERE anchor IN ({}) GROUP BY ns",
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let params_vec: Vec<&dyn rusqlite::ToSql> =
            anchors.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params_vec.as_slice(), |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ---------- grants ----------

    pub fn grant_create(
        &self,
        cap_id: &str,
        token_hash: &str,
        subtree: &str,
        ops: &[String],
        audience: Option<&str>,
        expires_at: Option<i64>,
        max_uses: Option<i64>,
    ) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO grants(cap_id, token_hash, subtree, ops_json, audience, expires_at, max_uses, created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                cap_id,
                token_hash,
                subtree,
                serde_json::to_string(ops).unwrap(),
                audience,
                expires_at,
                max_uses,
                unix_now()
            ],
        )?;
        Ok(())
    }

    pub fn grant_revoke(&self, cap_id: &str) -> NfsResult<bool> {
        let conn = self.lock();
        let n = conn.execute("UPDATE grants SET revoked=1 WHERE cap_id=?1", params![cap_id])?;
        Ok(n > 0)
    }

    pub fn grant_by_token_hash(&self, token_hash: &str) -> NfsResult<Option<GrantRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT cap_id, subtree, ops_json, audience, expires_at, revoked FROM grants WHERE token_hash=?1",
                params![token_hash],
                |r| {
                    Ok(GrantRow {
                        cap_id: r.get(0)?,
                        subtree: r.get(1)?,
                        ops: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                        audience: r.get(3)?,
                        expires_at: r.get(4)?,
                        revoked: r.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .optional()?)
    }

    // ---------- content index (cache, rebuildable) ----------

    pub fn content_index_put(
        &self,
        hash: &str,
        root: &str,
        rel: &str,
        size: i64,
        mtime: i64,
    ) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO content_index(hash, root_id, path, size, mtime, updated_at) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(hash) DO UPDATE SET root_id=?2, path=?3, size=?4, mtime=?5, updated_at=?6",
            params![hash, root, rel, size, mtime, unix_now()],
        )?;
        Ok(())
    }

    pub fn content_index_get(&self, hash: &str) -> NfsResult<Option<(String, String, i64, i64)>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT root_id, path, size, mtime FROM content_index WHERE hash=?1",
                params![hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?)
    }

    pub fn content_index_remove(&self, hash: &str) -> NfsResult<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM content_index WHERE hash=?1", params![hash])?;
        Ok(())
    }

    pub fn content_index_move_prefix(
        &self,
        root: &str,
        old_rel: &str,
        new_root: &str,
        new_rel: &str,
    ) -> NfsResult<()> {
        let conn = self.lock();
        let like = format!("{}/%", old_rel.replace('%', "\\%").replace('_', "\\_"));
        conn.execute(
            "UPDATE content_index SET root_id=?1, path = ?2 || substr(path, ?3) WHERE root_id=?4 AND path LIKE ?5 ESCAPE '\\'",
            params![new_root, new_rel, old_rel.len() as i64 + 1, root, like],
        )?;
        conn.execute(
            "UPDATE content_index SET root_id=?1, path=?2 WHERE root_id=?3 AND path=?4",
            params![new_root, new_rel, root, old_rel],
        )?;
        Ok(())
    }
}

fn ns_matches(filter: &str, ns: &str) -> bool {
    if let Some(prefix) = filter.strip_suffix(".*") {
        ns == prefix || ns.starts_with(&format!("{}.", prefix))
    } else {
        filter == ns
    }
}

/// Ops for `collection_patch`, decoded from the wire in containers.rs.
#[derive(Debug, Clone)]
pub enum CollectionPatchOp {
    AddRef {
        parent_group: Option<i64>,
        target_type: String,
        target_node_id: Option<i64>,
        target_obj_id: Option<String>,
        name: Option<String>,
        position: Option<usize>,
    },
    CreateGroup {
        name: String,
        position: Option<usize>,
    },
    RemoveEntry {
        entry_id: i64,
    },
    MoveEntries {
        entry_ids: Vec<i64>,
        to_index: usize,
    },
    RenameGroup {
        entry_id: i64,
        name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> FileDb {
        FileDb::open_memory().unwrap()
    }

    #[test]
    fn handle_key_persists() {
        let d = db();
        let k1 = d.handle_key().unwrap();
        let k2 = d.handle_key().unwrap();
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32);
    }

    #[test]
    fn lazy_anchor_and_rebind() {
        let d = db();
        let e1 = d.ensure_anchor("file", "home", "a.txt", 100, 1, 10, 1000).unwrap();
        let e2 = d.ensure_anchor("file", "home", "a.txt", 100, 1, 10, 1000).unwrap();
        assert_eq!(e1.node_id, e2.node_id);
        // Same path, new ino → overwrite heuristic keeps node identity.
        let e3 = d.ensure_anchor("file", "home", "a.txt", 200, 1, 12, 1001).unwrap();
        assert_eq!(e3.node_id, e1.node_id);
        let a = d.anchor_get(e1.node_id).unwrap().unwrap();
        assert_eq!(a.ino, 200);
        assert_eq!(a.full_hash, None);
    }

    #[test]
    fn anchor_move_prefix() {
        let d = db();
        let e1 = d.ensure_anchor("dir", "home", "docs", 1, 1, 0, 0).unwrap();
        let e2 = d.ensure_anchor("file", "home", "docs/a.txt", 2, 1, 5, 0).unwrap();
        let e3 = d.ensure_anchor("file", "home", "docs2/b.txt", 3, 1, 5, 0).unwrap();
        d.anchors_move_prefix("home", "docs", "home", "archive/docs").unwrap();
        assert_eq!(d.anchor_get(e1.node_id).unwrap().unwrap().path, "archive/docs");
        assert_eq!(d.anchor_get(e2.node_id).unwrap().unwrap().path, "archive/docs/a.txt");
        assert_eq!(d.anchor_get(e3.node_id).unwrap().unwrap().path, "docs2/b.txt");
    }

    #[test]
    fn delete_subtree_stales_bindings() {
        let d = db();
        let parent = d.ensure_anchor("dir", "home", "keep", 1, 1, 0, 0).unwrap();
        let victim = d.ensure_anchor("file", "home", "gone/x.txt", 2, 1, 0, 0).unwrap();
        d.binding_create(parent.node_id, "link-to-x", "live", Some(victim.node_id), None)
            .unwrap();
        let ids = d.anchors_delete_subtree("home", "gone").unwrap();
        assert!(ids.contains(&victim.node_id));
        let b = &d.bindings_by_parent(parent.node_id).unwrap()[0];
        assert_eq!(b.state, "stale");
        assert_eq!(d.anchor_get(victim.node_id).unwrap().unwrap().state, "deleted");
    }

    #[test]
    fn binding_name_conflict() {
        let d = db();
        let p = d.ensure_anchor("dir", "home", "d", 1, 1, 0, 0).unwrap();
        let t = d.ensure_anchor("file", "home", "t.txt", 2, 1, 0, 0).unwrap();
        d.binding_create(p.node_id, "x", "live", Some(t.node_id), None).unwrap();
        let err = d.binding_create(p.node_id, "x", "live", Some(t.node_id), None).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::NamespaceConflict);
    }

    #[test]
    fn collection_patch_order_and_cas() {
        let d = db();
        let c = d.collection_create("reading", "Reading List").unwrap();
        let t1 = d.ensure_anchor("file", "home", "1.pdf", 11, 1, 0, 0).unwrap();
        let t2 = d.ensure_anchor("file", "home", "2.pdf", 12, 1, 0, 0).unwrap();
        let g = d
            .collection_patch(
                c.node_id,
                Some(1),
                &[
                    CollectionPatchOp::AddRef {
                        parent_group: None,
                        target_type: "live".into(),
                        target_node_id: Some(t1.node_id),
                        target_obj_id: None,
                        name: Some("one".into()),
                        position: None,
                    },
                    CollectionPatchOp::AddRef {
                        parent_group: None,
                        target_type: "live".into(),
                        target_node_id: Some(t2.node_id),
                        target_obj_id: None,
                        name: Some("two".into()),
                        position: Some(0),
                    },
                    CollectionPatchOp::CreateGroup { name: "papers".into(), position: None },
                ],
            )
            .unwrap();
        assert_eq!(g, 2);
        let entries = d.collection_entries(c.node_id, None).unwrap();
        assert_eq!(entries.len(), 3);
        // "two" inserted at position 0.
        assert_eq!(entries[0].name.as_deref(), Some("two"));
        assert_eq!(entries[1].name.as_deref(), Some("one"));
        assert_eq!(entries[2].node_type, "group");
        assert_eq!(entries.iter().map(|e| e.manual_order).collect::<Vec<_>>(), vec![0, 1, 2]);
        // CAS failure
        let err = d
            .collection_patch(c.node_id, Some(1), &[])
            .unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::RevMismatch);
        // Remove the group cascades member unlink.
        let gid = entries[2].target_node_id.unwrap();
        d.collection_patch(
            c.node_id,
            None,
            &[CollectionPatchOp::AddRef {
                parent_group: Some(gid),
                target_type: "live".into(),
                target_node_id: Some(t1.node_id),
                target_obj_id: None,
                name: Some("in-group".into()),
                position: None,
            }],
        )
        .unwrap();
        assert_eq!(d.collection_entries(c.node_id, Some(gid)).unwrap().len(), 1);
        let group_entry = entries[2].entry_id;
        d.collection_patch(
            c.node_id,
            None,
            &[CollectionPatchOp::RemoveEntry { entry_id: group_entry }],
        )
        .unwrap();
        assert_eq!(d.collection_entries(c.node_id, Some(gid)).unwrap().len(), 0);
        assert_eq!(d.collection_entries(c.node_id, None).unwrap().len(), 2);
    }

    #[test]
    fn move_entries_reorders() {
        let d = db();
        let c = d.collection_create("c", "C").unwrap();
        let mut ids = Vec::new();
        for i in 0..4 {
            let t = d
                .ensure_anchor("file", "home", &format!("{}.txt", i), 20 + i, 1, 0, 0)
                .unwrap();
            d.collection_patch(
                c.node_id,
                None,
                &[CollectionPatchOp::AddRef {
                    parent_group: None,
                    target_type: "live".into(),
                    target_node_id: Some(t.node_id),
                    target_obj_id: None,
                    name: Some(format!("{}", i)),
                    position: None,
                }],
            )
            .unwrap();
            ids.push(d.collection_entries(c.node_id, None).unwrap().last().unwrap().entry_id);
        }
        // Move entries [0,1] to index 3 (after current 2,3).
        d.collection_patch(
            c.node_id,
            None,
            &[CollectionPatchOp::MoveEntries { entry_ids: vec![ids[0], ids[1]], to_index: 3 }],
        )
        .unwrap();
        let names: Vec<_> = d
            .collection_entries(c.node_id, None)
            .unwrap()
            .into_iter()
            .map(|e| e.name.unwrap())
            .collect();
        assert_eq!(names, vec!["2", "3", "0", "1"]);
    }

    #[test]
    fn view_overlay_merge() {
        let d = db();
        let v = d.view_create("topic/x", "X", "auto").unwrap();
        let t1 = d.ensure_anchor("file", "home", "m1", 31, 1, 0, 0).unwrap();
        let t2 = d.ensure_anchor("file", "home", "m2", 32, 1, 0, 0).unwrap();
        let b1 = d.view_base_add(v.node_id, None, t1.node_id, None, 0).unwrap();
        let _b2 = d.view_base_add(v.node_id, None, t2.node_id, None, 1).unwrap();
        // Patch: remove b1, pin b2.
        {
            let conn = d.lock();
            conn.execute(
                "INSERT INTO view_patch(view_node_id, op, base_id) VALUES(?1,'remove',?2)",
                params![v.node_id, b1],
            )
            .unwrap();
        }
        let members = d.view_members(v.node_id, None).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].target_node_id, Some(t2.node_id));
    }

    #[test]
    fn meta_ns_filter() {
        let d = db();
        let src = serde_json::json!({"kind":"user"});
        d.meta_set("live:n_1", "user", "rating", &serde_json::json!(5), &src, "private").unwrap();
        d.meta_set("live:n_1", "ai.vision.v1", "caption", &serde_json::json!("x"), &src, "zone")
            .unwrap();
        let all = d.meta_get(&["live:n_1".into()], &[]).unwrap();
        assert_eq!(all.len(), 2);
        let ai = d.meta_get(&["live:n_1".into()], &["ai.*".into()]).unwrap();
        assert_eq!(ai.len(), 1);
        assert_eq!(ai[0].ns, "ai.vision.v1");
        // Upsert
        d.meta_set("live:n_1", "user", "rating", &serde_json::json!(4), &src, "private").unwrap();
        let user = d.meta_get(&["live:n_1".into()], &["user".into()]).unwrap();
        assert_eq!(user.len(), 1);
        assert_eq!(user[0].value, serde_json::json!(4));
    }

    #[test]
    fn grants_lifecycle() {
        let d = db();
        d.grant_create("cap_1", "hash1", "cyfs:///home/x", &["read".into()], None, Some(9999999999), None)
            .unwrap();
        let g = d.grant_by_token_hash("hash1").unwrap().unwrap();
        assert!(!g.revoked);
        assert!(d.grant_revoke("cap_1").unwrap());
        assert!(d.grant_by_token_hash("hash1").unwrap().unwrap().revoked);
        assert!(!d.grant_revoke("cap_nope").unwrap());
    }
}
