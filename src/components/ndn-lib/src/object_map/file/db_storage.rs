use super::super::storage::{
    ObjectMapInnerStorage, ObjectMapInnerStorageStat, ObjectMapStorageType,
};
use crate::{NdnError, NdnResult, ObjId};
use futures::TryFutureExt;
use rusqlite::types::{FromSql, ToSql, ValueRef};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use std::collections::VecDeque;
use std::error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

struct ObjectMapSqliteStorageChunkIterator {
    conn: Arc<Mutex<Option<Connection>>>,
    buffer: VecDeque<(String, ObjId, Option<u64>)>,

    // Use last_key instead of offset to avoid large offset performance issue
    last_key: Option<String>,
    chunk_size: usize,

    finished: bool,
}

impl ObjectMapSqliteStorageChunkIterator {
    fn new(conn: Arc<Mutex<Option<Connection>>>, chunk_size: usize) -> Self {
        Self {
            conn,
            buffer: VecDeque::new(),
            last_key: None,
            chunk_size,
            finished: false,
        }
    }

    fn fetch_next_chunk(&mut self) -> NdnResult<()> {
        if self.finished {
            return Ok(());
        }

        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_mut().ok_or_else(|| {
            let msg = "Connection is not initialized".to_string();
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let query = if let Some(ref last_key) = self.last_key {
            format!(
                "SELECT key, value, mtree_index FROM object_map WHERE key > ?1 ORDER BY key LIMIT ?2"
            )
        } else {
            "SELECT key, value, mtree_index FROM object_map ORDER BY key LIMIT ?1 OFFSET ?2"
                .to_string()
        };

        let mut stmt = conn.prepare(&query).map_err(|e| {
            let msg = format!("Failed to prepare next statement: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let params = if self.last_key.is_some() {
            params![self.last_key.as_ref(), self.chunk_size as u64]
        } else {
            params![self.chunk_size as u64, 0u64]
        };

        let rows = stmt
            .query_map(params, |r| {
                let key: String = r.get(0)?;
                let value: String = r.get(1)?;
                let mtree_index: Option<i64> = r.get(2)?;
                Ok((key, value, mtree_index.map(|i| i as u64)))
            })
            .map_err(|e| {
                let msg = format!("Failed to query object_map: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        for row in rows {
            let row = row
                .map(|(key, value, mtree_index)| {
                    let obj_id = ObjId::new(&value).map_err(|e| {
                        let msg = format!("Failed to parse ObjId from value: {}, {}", value, e);
                        error!("{}", msg);
                        NdnError::DbError(msg)
                    })?;
                    Ok((key, obj_id, mtree_index))
                })
                .map_err(|e| {
                    let msg = format!("Failed to map row: {}", e);
                    error!("{}", msg);
                    NdnError::DbError(msg)
                })?;

            self.buffer.push_back(row?);
        }

        // Update last_key to the last key in the current buffer
        if let Some(last) = self.buffer.back() {
            self.last_key = Some(last.0.clone());
        } else {
            self.last_key = None;
            self.finished = true;
        }

        Ok(())
    }
}

impl Iterator for ObjectMapSqliteStorageChunkIterator {
    type Item = (String, ObjId, Option<u64>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.buffer.is_empty() {
            if let Err(e) = self.fetch_next_chunk() {
                error!("Failed to fetch next chunk: {}", e);
                return None;
            }
        }

        self.buffer.pop_front()
    }
}

#[derive(Clone)]
pub struct ObjectMapSqliteStorage {
    read_only: bool,
    file: PathBuf,

    conn: Arc<Mutex<Option<Connection>>>,
}

impl ObjectMapSqliteStorage {
    pub fn new(db_path: PathBuf, read_only: bool) -> NdnResult<Self> {
        let conn = Connection::open(&db_path).map_err(|e| {
            let msg = format!("Failed to open SQLite database: {:?}, {}", db_path, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Self::init_tables(&conn)?;

        Ok(Self::new_with_connection(db_path, conn, read_only))
    }

    fn new_with_connection(db_path: PathBuf, conn: Connection, read_only: bool) -> Self {
        Self {
            read_only,
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

    fn check_read_only(&self) -> NdnResult<()> {
        if self.read_only {
            let msg = format!("Storage is read-only: {}", self.file.display());
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }
        Ok(())
    }

    async fn clone_for_modify(&self, target: &Path) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        // First check if target is same as current file
        if target == self.file {
            let msg = format!("Target file is same as current file: {}", target.display());
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
            let msg = format!(
                "Failed to create backup: {:?} -> {:?}, {}",
                self.file, target, e
            );
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        backup
            .run_to_completion(64, std::time::Duration::from_millis(5), None)
            .map_err(|e| {
                let msg = format!(
                    "Failed to run backup: {:?} -> {:?}, {}",
                    self.file, target, e
                );
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        drop(backup);
        drop(lock);

        let new_storage =
            ObjectMapSqliteStorage::new_with_connection(target.to_path_buf(), new_conn, false);
        Ok(Box::new(new_storage))
    }
}

#[async_trait::async_trait]
impl ObjectMapInnerStorage for ObjectMapSqliteStorage {
    fn get_type(&self) -> ObjectMapStorageType {
        ObjectMapStorageType::SQLite
    }

    fn is_readonly(&self) -> bool {
        self.read_only
    }

    fn put(&mut self, key: &str, value: &ObjId) -> NdnResult<()> {
        self.check_read_only()?;

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

    fn put_with_index(
        &mut self,
        key: &str,
        value: &ObjId,
        index: Option<u64>,
    ) -> NdnResult<()> {
        self.check_read_only()?;

        let mut lock = self.conn.lock().unwrap();
        let mut conn = lock.as_mut().unwrap();

        let ret = match index {
            Some(i) => conn.execute(
                "INSERT OR REPLACE INTO object_map (key, value, mtree_index) VALUES (?1, ?2, ?3)",
                params![key, value.to_base32(), i],
            ),
            None => conn.execute(
                "INSERT OR REPLACE INTO object_map (key, value, mtree_index) VALUES (?1, ?2, NULL)",
                params![key, value.to_base32()],
            ),
        };

        ret.map_err(|e| {
            let msg = format!("Failed to insert into object_map: {}, {}", key, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(())
    }

    fn get(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>> {
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

    fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        self.check_read_only()?;

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

        let result: Option<String> = conn
            .query_row(
                "DELETE FROM object_map WHERE key=?1 RETURNING value",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| {
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

    fn is_exist(&self, key: &str) -> NdnResult<bool> {
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

    fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>> {
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

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (String, ObjId, Option<u64>)> + 'a> {
        let iter = ObjectMapSqliteStorageChunkIterator::new(self.conn.clone(), 64);
        Box::new(iter)
    }

    fn stat(&self) -> NdnResult<ObjectMapInnerStorageStat> {
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

    fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()> {
        self.check_read_only()?;

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

    fn get_mtree_index(&self, key: &str) -> NdnResult<Option<u64>> {
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

    fn put_mtree_data(&mut self, value: &[u8]) -> NdnResult<()> {
        self.check_read_only()?;

        let lock = self.conn.lock().unwrap();
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

    fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>> {
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

    async fn clone(
        &self,
        target: &Path,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        if read_only {
            let ret = Self::new(target.to_path_buf(), read_only)?;
            Ok(Box::new(ret))
        } else {
            self.clone_for_modify(target).await
        }
    }

    async fn save(&mut self, file: &Path) -> NdnResult<()> {
        self.check_read_only()?;

        // Check if file is same as current file
        if file == self.file {
            warn!("Target file is same as current file: {}", file.display());
            return Ok(());
        }

        // Check if target file exists
        if file.exists() {
            warn!(
                "Target object map storage file already exists: {}, now will overwrite it",
                file.display()
            );
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
            NdnError::IoError(msg)
        })?;

        info!("Renamed SQLite database: {:?} -> {:?}", self.file, file);

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
        let version: String = conn
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .unwrap();
        println!("SQLite version: {}", version);
    }
}
