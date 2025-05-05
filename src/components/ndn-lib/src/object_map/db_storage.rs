use super::storage::{ObjectMapInnerStorage, ObjectMapInnerStorageStat};
use crate::{NdnError, NdnResult, ObjId};
use rusqlite::types::{FromSql, ToSql, ValueRef};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use std::error;
use std::sync::{Arc, Mutex};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct ObjectMapSqliteStorage {
    file: PathBuf,

    conn: Arc<Mutex<Option<Connection>>>,
}

impl ObjectMapSqliteStorage {
    pub fn new(db_path: &Path) -> NdnResult<Self> {
        let conn = Connection::open(db_path).map_err(|e| {
            let msg = format!("Failed to open SQLite database: {:?}, {}", db_path, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Self::init_tables(&conn)?;

        Ok(Self::new_with_connection(db_path.to_path_buf(), conn))
    }

    fn new_with_connection(db_path: PathBuf, conn: Connection) -> Self {
        Self {
            file: db_path,
            conn: Arc::new(Mutex::new(Some(conn))),
        }
    }

    fn init_tables(conn: &Connection) -> NdnResult<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS object_map (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                mtree_index INTEGER
             );
             CREATE TABLE IF NOT EXISTS object_meta (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                value BLOB
             );
             CREATE TABLE IF NOT EXISTS mtree_meta (
                id I
                NTEGER PRIMARY KEY CHECK (id = 1),
                value BLOB
             );",
        )
        .map_err(|e| {
            let msg = format!("Failed to create tables: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })
    }
}

#[async_trait::async_trait]
impl ObjectMapInnerStorage for ObjectMapSqliteStorage {
    async fn put(&mut self, key: &str, value: &ObjId) -> NdnResult<()> {
        let mut lock = self.conn.lock().unwrap();
        let mut conn = lock.as_mut().unwrap();


        conn.execute(
            "INSERT OR REPLACE INTO object_map (key, value, mtree_index) VALUES (?1, ?2, NULL)",
            params![key, value.to_base32()],
        )
        .map_err(|e| {
            let msg = format!("Failed to insert into object_map: {}, {}", key, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(())
    }

    async fn get(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        let row: Option<(String, Option<u64>)> = conn
            .query_row(
                "SELECT value, mtree_index FROM object_map WHERE key=?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| {
                let msg = format!("Failed to query object_map: {}, {}", key, e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        match row {
            Some((v, m)) => {
                let obj_id = ObjId::new(&v)?;
                Ok(Some((obj_id, m)))
            }
            None => Ok(None),
        }
    }

    async fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        let mut lock = self.conn.lock().unwrap();
        let mut conn = lock.as_mut().unwrap();

        /* 
        let tx = conn.transaction().map_err(|e| {
            let msg = format!("Failed to begin transaction: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let row: Option<String> = tx
            .query_row(
                "SELECT value FROM object_map WHERE key=?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| {
                let msg = format!("Failed to query object_map: {}, {}", key, e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        tx.execute("DELETE FROM object_map WHERE key=?1", params![key])
            .map_err(|e| {
                let msg = format!("Failed to delete from object_map: {}, {}", key, e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        tx.commit().map_err(|e| {
            let msg = format!("Failed to commit transaction: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;
        */

        let result: Option<String> = conn.query_row(
            "DELETE FROM object_map WHERE key=?1 RETURNING value",
            [key],
            |row| row.get(0)
        ).optional().map_err(|e| {
            let msg = format!("Failed to delete from object_map: {}, {}", key, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let obj_id = match result {
            Some(v) => Some(ObjId::new(&v)?),
            None => None,
        };

        Ok(obj_id)
    }

    async fn is_exist(&self, key: &str) -> NdnResult<bool> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM object_map WHERE key=?1)",
                params![key],
                |r| r.get(0),
            )
            .map_err(|e| {
                let msg = format!("Failed to check existence in object_map: {}, {}", key, e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        Ok(exists)
    }

    async fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        let offset = page_index * page_size;
        let mut stmt = conn
            .prepare("SELECT key FROM object_map ORDER BY key LIMIT ?1 OFFSET ?2")
            .map_err(|e| {
                let msg = format!("Failed to prepare statement: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        let iter = stmt
            .query_map(params![page_size as u64, offset as u64], |r| r.get(0))
            .map_err(|e| {
                let msg = format!("Failed to query object_map: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        let mut keys = Vec::new();
        for k in iter {
            keys.push(k.map_err(|e| {
                let msg = format!("Failed to map key: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?);
        }

        Ok(keys)
    }

    async fn stat(&self) -> NdnResult<ObjectMapInnerStorageStat> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        let total_count: usize = conn
            .query_row("SELECT COUNT(*) FROM object_map", [], |r| r.get(0))
            .map_err(|e| {
                let msg = format!("Failed to count object_map: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;
        Ok(ObjectMapInnerStorageStat {
            total_count: total_count as u64,
        })
    }

    async fn put_meta(&mut self, value: &[u8]) -> NdnResult<()> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO object_meta (id, value) VALUES (1, ?1)",
            params![value],
        )
        .map_err(|e| {
            let msg = format!("Failed to insert object_meta: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(())
    }

    async fn get_meta(&self) -> NdnResult<Option<Vec<u8>>> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        let data: Option<Vec<u8>> = conn
            .query_row("SELECT value FROM object_meta WHERE id=1", [], |r| r.get(0))
            .optional()
            .map_err(|e| {
                let msg = format!("Failed to query object_meta: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        Ok(data)
    }

    async fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()> {
        let mut lock = self.conn.lock().unwrap();
        let mut conn = lock.as_mut().unwrap();

        conn.execute(
            "UPDATE object_map SET mtree_index=?1 WHERE key=?2",
            params![index, key],
        )
        .map_err(|e| {
            let msg = format!("Failed to update mtree_index: {}, {}", key, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        if conn.changes() == 0 {
            let msg = format!("No such key: {}", key);
            error!("{}", msg);
            return Err(NdnError::NotFound(msg));
        }

        Ok(())
    }

    async fn get_mtree_index(&self, key: &str) -> NdnResult<Option<u64>> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        conn.query_row(
            "SELECT mtree_index FROM object_map WHERE key=?1",
            params![key],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| {
            let msg = format!("Failed to query mtree_index: {}, {}", key, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })
        .map(|v| v.map(|i: i64| i as u64))
    }

    async fn put_mtree_data(&mut self, value: &[u8]) -> NdnResult<()> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO mtree_meta (id, value) VALUES (1, ?1)",
            params![value],
        )
        .map_err(|e| {
            let msg = format!("Failed to insert mtree_meta: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(())
    }

    async fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>> {
        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_ref().unwrap();

        conn.query_row("SELECT value FROM mtree_meta WHERE id=1", [], |r| r.get(0))
            .optional()
            .map_err(|e| {
                let msg = format!("Failed to query mtree_meta: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })
            .map(|v| v.map(|i: Vec<u8>| i))
    }

    async fn clone(&self, target: &Path) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        // First check if target is same as current file
        if target == self.file {
            let msg = format!(
                "Target file is same as current file: {}",
                target.display()
            );
            error!("{}", msg);
            return Err(NdnError::AlreadyExists(msg));
        }

        // Open new connection to target file
        let mut new_conn = Connection::open(target).map_err(|e| {
            let msg = format!("Failed to open SQLite database: {:?}, {}", target, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let mut lock = self.conn.lock().unwrap();
        let mut conn = lock.as_ref().unwrap();
        let backup = rusqlite::backup::Backup::new(&conn, &mut new_conn).map_err(|e| {
            let msg = format!("Failed to create backup: {:?} -> {:?}, {}", self.file, target, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        backup.run_to_completion(64, std::time::Duration::from_millis(5), None).map_err(|e| {
            let msg = format!(
                "Failed to run backup: {:?} -> {:?}, {}",
                self.file, target, e
            );
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        drop(backup);
        drop(lock);

        let new_storage = ObjectMapSqliteStorage::new_with_connection(
            target.to_path_buf(),
            new_conn,
        );
        Ok(Box::new(new_storage))
    }

    async fn save(&mut self, file: &Path) -> NdnResult<()> {
        // Check if file is same as current file
        if file == self.file {
            warn!("Target file is same as current file: {}", file.display());
            return Ok(());
        }

        // Check if target file exists
        if file.exists() {
            let msg = format!("Target file already exists: {}", file.display());
            error!("{}", msg);
            return Err(NdnError::AlreadyExists(msg));
        }

        // First close the current connection, then try to rename the file, and then open a new connection to the file.
        let mut lock = self.conn.lock().unwrap();
        let mut conn = lock.take().unwrap();
        conn.close().map_err(|e| {
            let msg = format!("Failed to close SQLite database: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        std::fs::rename(&self.file, file).map_err(|e| {
            let msg = format!(
                "Failed to rename SQLite database: {:?} -> {:?}, {}",
                self.file, file, e
            );
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        // Open a new connection to the file
        let new_conn = Connection::open(file).map_err(|e| {
            let msg = format!("Failed to open SQLite database: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        *lock = Some(new_conn);
        self.file = file.to_path_buf();

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use rusqlite::{Connection, Result};

    #[test]
    fn test_version() {
        let conn = Connection::open_in_memory().unwrap();
        let version: String = conn.query_row("SELECT sqlite_version()", [], |row| row.get(0)).unwrap();
        println!("SQLite version: {}", version);
    }
}
