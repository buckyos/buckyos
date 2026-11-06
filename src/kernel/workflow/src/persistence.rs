//! Service-local SQLite state (TODO T1d): the persistent step cache index, the
//! process execution epoch and the ledger of local native executions that a
//! restarted instance must fail and clean up.

use crate::cache_index::{CacheEntry, CacheIndex};
use crate::error::{WorkflowError, WorkflowResult};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveExecutionRecord {
    pub execution_id: String,
    pub run_id: String,
    pub node_id: String,
    pub epoch: u64,
    pub temp_dir: String,
    pub task_id: Option<String>,
    pub started_at: i64,
}

/// Records which executions this process owns, so a successor can mark them
/// `interrupted` and remove their temp dirs.
#[async_trait]
pub trait ExecutionLedger: Send + Sync {
    async fn record_started(&self, record: &ActiveExecutionRecord) -> WorkflowResult<()>;
    async fn record_finished(&self, execution_id: &str) -> WorkflowResult<()>;
    async fn stale_executions(&self, current_epoch: u64) -> WorkflowResult<Vec<ActiveExecutionRecord>>;
}

#[derive(Default)]
pub struct MemoryExecutionLedger {
    inner: Mutex<Vec<ActiveExecutionRecord>>,
}

#[async_trait]
impl ExecutionLedger for MemoryExecutionLedger {
    async fn record_started(&self, record: &ActiveExecutionRecord) -> WorkflowResult<()> {
        self.inner.lock().unwrap().push(record.clone());
        Ok(())
    }

    async fn record_finished(&self, execution_id: &str) -> WorkflowResult<()> {
        self.inner
            .lock()
            .unwrap()
            .retain(|r| r.execution_id != execution_id);
        Ok(())
    }

    async fn stale_executions(&self, current_epoch: u64) -> WorkflowResult<Vec<ActiveExecutionRecord>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.epoch < current_epoch)
            .cloned()
            .collect())
    }
}

pub struct WorkflowDb {
    conn: Mutex<Connection>,
}

fn db_err(err: rusqlite::Error) -> WorkflowError {
    WorkflowError::Persistence(err.to_string())
}

impl WorkflowDb {
    pub fn open(path: &Path) -> WorkflowResult<Arc<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| WorkflowError::Persistence(err.to_string()))?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> WorkflowResult<Arc<Self>> {
        let conn = Connection::open_in_memory().map_err(db_err)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> WorkflowResult<Arc<Self>> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS cache_index (
                 cache_key TEXT PRIMARY KEY,
                 result_obj_id TEXT NOT NULL,
                 artifact_refs TEXT NOT NULL,
                 fingerprint TEXT NOT NULL,
                 expires_at INTEGER,
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS service_epoch (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 epoch INTEGER NOT NULL,
                 started_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS active_executions (
                 execution_id TEXT PRIMARY KEY,
                 run_id TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 epoch INTEGER NOT NULL,
                 temp_dir TEXT NOT NULL,
                 task_id TEXT,
                 started_at INTEGER NOT NULL
             );",
        )
        .map_err(db_err)?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
        }))
    }

    /// Increments and returns the process epoch.
    pub fn next_epoch(&self) -> WorkflowResult<u64> {
        let conn = self.conn.lock().unwrap();
        let current: Option<i64> = conn
            .query_row("SELECT epoch FROM service_epoch WHERE id = 1", [], |row| row.get(0))
            .optional()
            .map_err(db_err)?;
        let next = current.unwrap_or(0) + 1;
        conn.execute(
            "INSERT INTO service_epoch(id, epoch, started_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET epoch = excluded.epoch, started_at = excluded.started_at",
            params![next, chrono::Utc::now().timestamp()],
        )
        .map_err(db_err)?;
        Ok(next as u64)
    }

    pub fn current_epoch(&self) -> WorkflowResult<u64> {
        let conn = self.conn.lock().unwrap();
        let current: Option<i64> = conn
            .query_row("SELECT epoch FROM service_epoch WHERE id = 1", [], |row| row.get(0))
            .optional()
            .map_err(db_err)?;
        Ok(current.unwrap_or(0) as u64)
    }

    pub fn purge_expired_cache(&self, now: i64) -> WorkflowResult<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM cache_index WHERE expires_at IS NOT NULL AND expires_at < ?1",
            params![now],
        )
        .map_err(db_err)
    }
}

#[async_trait]
impl CacheIndex for WorkflowDb {
    async fn get(&self, key: &str) -> WorkflowResult<Option<CacheEntry>> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(String, String, String, Option<i64>, i64)> = conn
            .query_row(
                "SELECT result_obj_id, artifact_refs, fingerprint, expires_at, created_at
                 FROM cache_index WHERE cache_key = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()
            .map_err(db_err)?;
        Ok(row.map(|(result_obj_id, refs, fingerprint, expires_at, created_at)| CacheEntry {
            result_obj_id,
            artifact_refs: serde_json::from_str(&refs).unwrap_or_default(),
            fingerprint: serde_json::from_str::<BTreeMap<String, String>>(&fingerprint)
                .unwrap_or_default(),
            expires_at,
            created_at,
        }))
    }

    async fn put(&self, key: &str, entry: &CacheEntry) -> WorkflowResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cache_index(cache_key, result_obj_id, artifact_refs, fingerprint, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(cache_key) DO UPDATE SET result_obj_id = excluded.result_obj_id,
                 artifact_refs = excluded.artifact_refs, fingerprint = excluded.fingerprint,
                 expires_at = excluded.expires_at, created_at = excluded.created_at",
            params![
                key,
                entry.result_obj_id,
                serde_json::to_string(&entry.artifact_refs).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&entry.fingerprint).unwrap_or_else(|_| "{}".into()),
                entry.expires_at,
                entry.created_at,
            ],
        )
        .map_err(db_err)?;
        Ok(())
    }

    async fn remove(&self, key: &str) -> WorkflowResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM cache_index WHERE cache_key = ?1", params![key])
            .map_err(db_err)?;
        Ok(())
    }

    async fn len(&self) -> WorkflowResult<usize> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM cache_index", [], |row| row.get::<_, i64>(0))
            .map(|n| n as usize)
            .map_err(db_err)
    }
}

#[async_trait]
impl ExecutionLedger for WorkflowDb {
    async fn record_started(&self, record: &ActiveExecutionRecord) -> WorkflowResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO active_executions(execution_id, run_id, node_id, epoch, temp_dir, task_id, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(execution_id) DO UPDATE SET epoch = excluded.epoch, temp_dir = excluded.temp_dir,
                 task_id = excluded.task_id, started_at = excluded.started_at",
            params![
                record.execution_id,
                record.run_id,
                record.node_id,
                record.epoch as i64,
                record.temp_dir,
                record.task_id,
                record.started_at,
            ],
        )
        .map_err(db_err)?;
        Ok(())
    }

    async fn record_finished(&self, execution_id: &str) -> WorkflowResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM active_executions WHERE execution_id = ?1",
            params![execution_id],
        )
        .map_err(db_err)?;
        Ok(())
    }

    async fn stale_executions(&self, current_epoch: u64) -> WorkflowResult<Vec<ActiveExecutionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT execution_id, run_id, node_id, epoch, temp_dir, task_id, started_at
                 FROM active_executions WHERE epoch < ?1",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![current_epoch as i64], |row| {
                Ok(ActiveExecutionRecord {
                    execution_id: row.get(0)?,
                    run_id: row.get(1)?,
                    node_id: row.get(2)?,
                    epoch: row.get::<_, i64>(3)? as u64,
                    temp_dir: row.get(4)?,
                    task_id: row.get(5)?,
                    started_at: row.get(6)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cache_index_round_trips_and_epoch_advances() {
        let db = WorkflowDb::open_in_memory().unwrap();
        assert_eq!(db.next_epoch().unwrap(), 1);
        assert_eq!(db.next_epoch().unwrap(), 2);
        let entry = CacheEntry {
            result_obj_id: "res:abc".into(),
            artifact_refs: vec!["cyfile:1".into()],
            expires_at: Some(10),
            created_at: 1,
            fingerprint: BTreeMap::from([("cmd/pdftoppm".into(), "24.02".into())]),
        };
        db.put("k", &entry).await.unwrap();
        assert_eq!(db.get("k").await.unwrap(), Some(entry.clone()));
        assert_eq!(db.len().await.unwrap(), 1);
        assert_eq!(db.purge_expired_cache(11).unwrap(), 1);
        assert_eq!(db.get("k").await.unwrap(), None);

        let record = ActiveExecutionRecord {
            execution_id: "run-1/a/1".into(),
            run_id: "run-1".into(),
            node_id: "a".into(),
            epoch: 1,
            temp_dir: "/tmp/x".into(),
            task_id: Some("t-1".into()),
            started_at: 0,
        };
        db.record_started(&record).await.unwrap();
        assert_eq!(db.stale_executions(2).await.unwrap(), vec![record.clone()]);
        assert!(db.stale_executions(1).await.unwrap().is_empty());
        db.record_finished(&record.execution_id).await.unwrap();
        assert!(db.stale_executions(2).await.unwrap().is_empty());
    }
}
