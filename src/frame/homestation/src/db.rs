//! SQLite storage (schema in `doc/homestation/HomeStation 协议与实现.md` §6). All access goes
//! through `Db::call`, which runs on the blocking pool.

use crate::error::{HsError, HsResult};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS objects (
  obj_id TEXT PRIMARY KEY,
  obj_type TEXT NOT NULL,
  body TEXT NOT NULL,
  jwt TEXT,
  signer TEXT,
  publisher TEXT,
  verified INTEGER NOT NULL DEFAULT 0,
  local_only INTEGER NOT NULL DEFAULT 0,
  origin TEXT,
  received_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS feed_index (
  obj_id TEXT PRIMARY KEY,
  publisher TEXT NOT NULL,
  kind TEXT NOT NULL,
  comment_type TEXT,
  content_type TEXT,
  entry TEXT,
  entry_valid INTEGER NOT NULL DEFAULT 0,
  target TEXT,
  wraps TEXT,
  iat INTEGER NOT NULL,
  category TEXT,
  text TEXT NOT NULL DEFAULT '',
  tags TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS feed_index_target ON feed_index(target);
CREATE INDEX IF NOT EXISTS feed_index_entry ON feed_index(entry);
CREATE INDEX IF NOT EXISTS feed_index_publisher ON feed_index(publisher, iat);
CREATE INDEX IF NOT EXISTS feed_index_wraps ON feed_index(wraps);

CREATE TABLE IF NOT EXISTS heads (
  entry TEXT PRIMARY KEY,
  publisher TEXT NOT NULL,
  seq INTEGER NOT NULL,
  state TEXT NOT NULL,
  current TEXT,
  head_obj_id TEXT NOT NULL,
  conflict_obj_id TEXT,
  restricted INTEGER NOT NULL DEFAULT 0,
  tier TEXT,
  updated_at_ms INTEGER NOT NULL,
  verified_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS entry_versions (
  entry TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  first_seen INTEGER NOT NULL,
  PRIMARY KEY (entry, obj_id)
);

CREATE TABLE IF NOT EXISTS published (
  entry TEXT PRIMARY KEY,
  key TEXT NOT NULL,
  namespace TEXT NOT NULL,
  kind TEXT NOT NULL,
  audience TEXT NOT NULL,
  category TEXT,
  target TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS published_target ON published(target);

CREATE TABLE IF NOT EXISTS head_history (
  entry TEXT NOT NULL,
  seq INTEGER NOT NULL,
  head_obj_id TEXT NOT NULL,
  state TEXT NOT NULL,
  current TEXT,
  at INTEGER NOT NULL,
  PRIMARY KEY (entry, seq)
);

CREATE TABLE IF NOT EXISTS stream_changes (
  cursor INTEGER PRIMARY KEY AUTOINCREMENT,
  entry TEXT NOT NULL,
  kind TEXT NOT NULL,
  seq INTEGER NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS stream_changes_entry ON stream_changes(entry);

CREATE TABLE IF NOT EXISTS object_grants (
  obj_id TEXT NOT NULL,
  entry TEXT NOT NULL,
  PRIMARY KEY (obj_id, entry)
);

CREATE TABLE IF NOT EXISTS publish_tasks (
  key TEXT PRIMARY KEY,
  stage TEXT NOT NULL,
  entry TEXT,
  obj_id TEXT,
  input TEXT NOT NULL,
  error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS outbox (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  obj_id TEXT NOT NULL,
  recipient TEXT NOT NULL,
  target TEXT,
  entry TEXT,
  task_key TEXT,
  purpose TEXT NOT NULL,
  restricted INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL,
  admission TEXT,
  attempts INTEGER NOT NULL DEFAULT 0,
  next_at INTEGER NOT NULL,
  last_result TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (obj_id, recipient)
);
CREATE INDEX IF NOT EXISTS outbox_due ON outbox(state, next_at);
CREATE INDEX IF NOT EXISTS outbox_entry ON outbox(entry);

CREATE TABLE IF NOT EXISTS inbox_receipts (
  obj_id TEXT PRIMARY KEY,
  sender TEXT NOT NULL,
  admission TEXT,
  received_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS inbox_receipts_sender ON inbox_receipts(sender, received_at);

CREATE TABLE IF NOT EXISTS sources (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  did TEXT,
  url TEXT,
  name TEXT NOT NULL,
  description TEXT,
  basis TEXT NOT NULL DEFAULT '[]',
  paused INTEGER NOT NULL DEFAULT 0,
  intent_id TEXT,
  notify TEXT NOT NULL,
  follow_entry TEXT,
  cursor INTEGER,
  last_success_at INTEGER,
  last_error TEXT,
  last_fetch_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS sources_did ON sources(did) WHERE did IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS sources_url ON sources(url) WHERE url IS NOT NULL;

CREATE TABLE IF NOT EXISTS intents (
  id TEXT PRIMARY KEY,
  text TEXT NOT NULL,
  status TEXT NOT NULL,
  source_ids TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS followers (
  did TEXT PRIMARY KEY,
  entry TEXT NOT NULL,
  state TEXT NOT NULL,
  seq INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS candidates (
  obj_id TEXT PRIMARY KEY,
  publisher TEXT NOT NULL,
  selection TEXT NOT NULL,
  reason TEXT,
  rule_ids TEXT NOT NULL DEFAULT '[]',
  source_paths TEXT NOT NULL DEFAULT '[]',
  private_capture INTEGER NOT NULL DEFAULT 0,
  arrived_at INTEGER NOT NULL,
  score REAL,
  resources TEXT NOT NULL DEFAULT 'unknown',
  replaced_by TEXT,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS candidates_selection ON candidates(selection, arrived_at);

CREATE TABLE IF NOT EXISTS admission_history (
  obj_id TEXT PRIMARY KEY,
  first_admitted_at INTEGER,
  opened_at INTEGER
);

CREATE TABLE IF NOT EXISTS reading (
  obj_id TEXT PRIMARY KEY,
  admitted_at INTEGER NOT NULL,
  order_key INTEGER NOT NULL,
  reason TEXT NOT NULL,
  resources TEXT NOT NULL,
  topics TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS reading_order ON reading(order_key);

CREATE TABLE IF NOT EXISTS view_cursors (
  view_key TEXT PRIMARY KEY,
  anchor TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS personal (
  obj_id TEXT PRIMARY KEY,
  like_entry TEXT,
  repost_entry TEXT,
  bookmark_on INTEGER NOT NULL DEFAULT 0,
  bookmark_private TEXT,
  bookmark_entry TEXT,
  saved_at INTEGER,
  read_later INTEGER NOT NULL DEFAULT 0,
  read_later_at INTEGER,
  dislike INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS comment_sources (
  comment_id TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  source TEXT NOT NULL,
  seen_at INTEGER NOT NULL,
  PRIMARY KEY (comment_id, source_kind, source)
);

CREATE TABLE IF NOT EXISTS author_list (
  target TEXT NOT NULL,
  comment_id TEXT NOT NULL,
  listed INTEGER NOT NULL,
  position INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (target, comment_id)
);

CREATE TABLE IF NOT EXISTS tracked (
  target TEXT PRIMARY KEY,
  author TEXT NOT NULL,
  reason TEXT NOT NULL,
  paused INTEGER NOT NULL DEFAULT 0,
  last_sync_at INTEGER,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS participants (
  target TEXT NOT NULL,
  did TEXT NOT NULL,
  seen_at INTEGER NOT NULL,
  PRIMARY KEY (target, did)
);

CREATE TABLE IF NOT EXISTS view_snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  view TEXT NOT NULL,
  target TEXT NOT NULL,
  complete INTEGER NOT NULL,
  comment_ids TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS view_snapshots_target ON view_snapshots(view, target, at);

CREATE TABLE IF NOT EXISTS eval_records (
  result_id TEXT PRIMARY KEY,
  target_key TEXT NOT NULL,
  target_kind TEXT NOT NULL,
  profile_id TEXT NOT NULL,
  profile_revision TEXT NOT NULL,
  dimensions TEXT NOT NULL,
  scope TEXT NOT NULL,
  state TEXT NOT NULL,
  iat INTEGER NOT NULL,
  refresh_after INTEGER,
  result TEXT NOT NULL,
  superseded_by TEXT
);
CREATE INDEX IF NOT EXISTS eval_records_target ON eval_records(target_key, profile_id);

CREATE TABLE IF NOT EXISTS eval_tasks (
  task_id TEXT PRIMARY KEY,
  request TEXT NOT NULL,
  scope TEXT NOT NULL,
  state TEXT NOT NULL,
  result_ids TEXT NOT NULL DEFAULT '[]',
  error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tag_overrides (
  id TEXT PRIMARY KEY,
  target_key TEXT NOT NULL,
  dimension TEXT NOT NULL,
  tag TEXT NOT NULL,
  action TEXT NOT NULL,
  replacement TEXT,
  scope TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  revoked_at INTEGER
);
CREATE INDEX IF NOT EXISTS tag_overrides_target ON tag_overrides(target_key);

CREATE TABLE IF NOT EXISTS eval_changes (
  cursor INTEGER PRIMARY KEY AUTOINCREMENT,
  target_key TEXT NOT NULL,
  result_id TEXT,
  kind TEXT NOT NULL,
  scope TEXT NOT NULL,
  at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS behavior_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  obj_id TEXT NOT NULL,
  event TEXT NOT NULL,
  value INTEGER,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS behavior_events_at ON behavior_events(at);

CREATE TABLE IF NOT EXISTS agreements (
  id TEXT PRIMARY KEY,
  target TEXT NOT NULL,
  receiver TEXT NOT NULL,
  action TEXT NOT NULL,
  terms TEXT,
  joined INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS consumption_proofs (
  obj_id TEXT PRIMARY KEY,
  agreement_id TEXT NOT NULL,
  target TEXT NOT NULL,
  receiver TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS capture_shares (
  capture_id TEXT PRIMARY KEY,
  shared_id TEXT NOT NULL,
  entry TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS collector_index (
  obj_id TEXT PRIMARY KEY,
  submitted_by TEXT NOT NULL,
  received_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS collector_index_at ON collector_index(received_at);
"#;

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> HsResult<Self> {
        // The runtime names the service data folder but does not create it on a fresh install.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HsError::Internal(format!("create {}: {e}", parent.display())))?;
        }
        let conn = Connection::open(path).map_err(HsError::from)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        let version: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0)).optional()?;
        match version {
            None => {
                conn.execute("INSERT INTO meta(key, value) VALUES ('schema_version', ?1)", [SCHEMA_VERSION.to_string()])?;
            }
            Some(v) if v == SCHEMA_VERSION.to_string() => {}
            Some(v) => return Err(HsError::Internal(format!("unsupported schema version {v}"))),
        }
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    pub async fn call<T, F>(&self, f: F) -> HsResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> HsResult<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().map_err(|_| HsError::Internal("database lock poisoned".into()))?;
            f(&mut guard)
        })
        .await
        .map_err(|e| HsError::Internal(format!("database task failed: {e}")))?
    }

    /// Synchronous access for tests and startup.
    pub fn with<T>(&self, f: impl FnOnce(&mut Connection) -> HsResult<T>) -> HsResult<T> {
        let mut guard = self.conn.lock().map_err(|_| HsError::Internal("database lock poisoned".into()))?;
        f(&mut guard)
    }
}

pub fn get_setting(conn: &Connection, key: &str) -> HsResult<Option<String>> {
    Ok(conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> HsResult<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [key, value],
    )?;
    Ok(())
}

pub fn get_meta(conn: &Connection, key: &str) -> HsResult<Option<String>> {
    Ok(conn.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0)).optional()?)
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> HsResult<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_a_missing_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("homestation").join("homestation.db");
        let db = Db::open(&path).unwrap();
        let version = db.with(|c| get_meta(c, "schema_version")).unwrap();
        assert_eq!(version.as_deref(), Some("1"));
        drop(db);
        Db::open(&path).unwrap();
    }
}
