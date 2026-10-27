//! The HomeStation database: one RDB instance (`homestation-main`, opened through `rdb_mgr`
//! in a zone) for every user's home and the zone feed list. Each station works on its own
//! rows: [`Db::for_owner`] gives a handle whose [`Conn`] carries the owner (username) that
//! every user-table statement filters and writes. Schema: `buckyos_api::HOMESTATION_RDB_SCHEMA_SQLITE`,
//! doc/homestation/HomeStation Durable Data Schema.md. All access goes through `Db::call`, which
//! runs on the blocking pool.

use crate::error::{HsError, HsResult};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use std::ops::Deref;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const SCHEMA_VERSION: u64 = buckyos_api::HOMESTATION_RDB_SCHEMA_VERSION;

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    owner: Arc<str>,
}

/// The connection as one owner sees it.
pub struct Conn<'a> {
    conn: &'a Connection,
    pub owner: &'a str,
}

impl Deref for Conn<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl<'a> Conn<'a> {
    pub fn transaction(&self) -> HsResult<Tx<'a>> {
        let tx = rusqlite::Transaction::new_unchecked(self.conn, TransactionBehavior::Deferred)?;
        Ok(Tx { conn: Conn { conn: self.conn, owner: self.owner }, tx })
    }
}

/// A transaction of one owner; rolled back unless committed.
pub struct Tx<'a> {
    conn: Conn<'a>,
    tx: rusqlite::Transaction<'a>,
}

impl<'a> Deref for Tx<'a> {
    type Target = Conn<'a>;
    fn deref(&self) -> &Conn<'a> {
        &self.conn
    }
}

impl Tx<'_> {
    pub fn commit(self) -> HsResult<()> {
        Ok(self.tx.commit()?)
    }
}

/// The sqlite file behind an rdb_mgr connection string (`sqlite:<path>?mode=rwc`).
pub fn sqlite_path(connection: &str) -> Option<std::path::PathBuf> {
    let body = connection.strip_prefix("sqlite://").or_else(|| connection.strip_prefix("sqlite:"))?;
    let path = body.split('?').next()?;
    (!path.is_empty() && path != ":memory:").then(|| path.into())
}

impl Db {
    /// Opens the database file (the rdb_mgr instance's path in a zone, a local file
    /// standalone); the handle is the zone's own (owner `""`).
    pub fn open(path: &Path) -> HsResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HsError::Internal(format!("create {}: {e}", parent.display())))?;
        }
        let conn = Connection::open(path).map_err(HsError::from)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(buckyos_api::HOMESTATION_RDB_SCHEMA_SQLITE)?;
        let version: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0)).optional()?;
        match version {
            None => {
                conn.execute("INSERT INTO meta(key, value) VALUES ('schema_version', ?1)", [SCHEMA_VERSION.to_string()])?;
            }
            Some(v) if v == SCHEMA_VERSION.to_string() => {}
            Some(v) => return Err(HsError::Internal(format!("unsupported schema version {v}"))),
        }
        Ok(Self { conn: Arc::new(Mutex::new(conn)), owner: Arc::from("") })
    }

    /// The same database, scoped to one user's rows.
    pub fn for_owner(&self, owner: &str) -> Self {
        Self { conn: self.conn.clone(), owner: Arc::from(owner) }
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub async fn call<T, F>(&self, f: F) -> HsResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Conn) -> HsResult<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.with(f))
            .await
            .map_err(|e| HsError::Internal(format!("database task failed: {e}")))?
    }

    /// Synchronous access for tests and startup.
    pub fn with<T>(&self, f: impl FnOnce(&mut Conn) -> HsResult<T>) -> HsResult<T> {
        let guard = self.conn.lock().map_err(|_| HsError::Internal("database lock poisoned".into()))?;
        f(&mut Conn { conn: &guard, owner: &self.owner })
    }
}

pub fn get_setting(conn: &Conn, key: &str) -> HsResult<Option<String>> {
    Ok(conn.query_row("SELECT value FROM settings WHERE key=?1 AND owner=?2", [key, conn.owner], |r| r.get(0)).optional()?)
}

pub fn set_setting(conn: &Conn, key: &str, value: &str) -> HsResult<()> {
    conn.execute(
        "INSERT INTO settings(owner, key, value) VALUES (?3, ?1, ?2) ON CONFLICT(owner, key) DO UPDATE SET value=excluded.value",
        [key, value, conn.owner],
    )?;
    Ok(())
}

pub fn get_meta(conn: &Connection, key: &str) -> HsResult<Option<String>> {
    Ok(conn.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0)).optional()?)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_a_missing_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("homestation").join("homestation-main.db");
        let db = Db::open(&path).unwrap();
        let version = db.with(|c| get_meta(c, "schema_version")).unwrap();
        assert_eq!(version, Some(SCHEMA_VERSION.to_string()));
        drop(db);
        Db::open(&path).unwrap();
    }

    #[test]
    fn owners_see_their_own_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("homestation-main.db")).unwrap();
        let (alice, bob) = (db.for_owner("alice"), db.for_owner("bob"));
        alice.with(|c| set_setting(c, "k", "a")).unwrap();
        bob.with(|c| {
            let tx = c.transaction()?;
            set_setting(&tx, "k", "b")?;
            tx.commit()
        })
        .unwrap();
        bob.with(|c| {
            let tx = c.transaction()?;
            set_setting(&tx, "k", "rolled back")
        })
        .unwrap();
        assert_eq!(alice.with(|c| get_setting(c, "k")).unwrap().as_deref(), Some("a"));
        assert_eq!(bob.with(|c| get_setting(c, "k")).unwrap().as_deref(), Some("b"));
        assert_eq!(sqlite_path("sqlite:/data/homestation/homestation-main.db?mode=rwc"), Some("/data/homestation/homestation-main.db".into()));
    }
}
