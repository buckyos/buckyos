use openraft::entry::RaftPayload;
use openraft::storage::{LogFlushed, RaftLogStorage};
use openraft::{Entry, LogId, OptionalSend, RaftLogReader, Vote};
use rusqlite::{Connection, params};
use std::fmt::Debug;
use std::ops::{Bound, RangeBounds};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;

use crate::util::persist_format::{PersistPayloadType, decode_with_header, encode_with_header};
use crate::{KNodeId, KTypeConfig, StorageResult};

type LogEntry = Entry<KTypeConfig>;

const META_VOTE: &str = "vote";
const META_LAST_PURGED: &str = "last_purged";
const META_COMMITTED: &str = "committed";
const SQLITE_LOG_INDEX_MAX_U64: u64 = i64::MAX as u64;

#[derive(Debug, Clone)]
pub struct SqliteLogStorage {
    conn: Arc<AsyncMutex<Connection>>,
}

impl SqliteLogStorage {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create sqlite parent dir {}: {}",
                    parent.display(),
                    e
                )
            })?;
        }

        let conn = Connection::open(path.as_ref()).map_err(|e| {
            format!(
                "Failed to open sqlite db {}: {}",
                path.as_ref().display(),
                e
            )
        })?;

        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS raft_logs (
                log_index INTEGER PRIMARY KEY,
                entry BLOB NOT NULL
            );

            CREATE TABLE IF NOT EXISTS raft_meta (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );
            "#,
        )
        .map_err(|e| format!("Failed to initialize sqlite schema: {}", e))?;

        Ok(Self {
            conn: Arc::new(AsyncMutex::new(conn)),
        })
    }

    fn ser<T: serde::Serialize>(payload_type: PersistPayloadType, v: &T) -> StorageResult<Vec<u8>> {
        encode_with_header(payload_type, v).map_err(|e| {
            let io_err = std::io::Error::other(format!("Failed to serialize value: {}", e));
            openraft::StorageError::IO {
                source: openraft::StorageIOError::write(&io_err),
            }
        })
    }

    fn de<T: serde::de::DeserializeOwned>(
        expected_type: PersistPayloadType,
        bytes: &[u8],
        what: &str,
    ) -> StorageResult<T> {
        decode_with_header(expected_type, bytes).map_err(|e| {
            let io_err = std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to deserialize {}: {}", what, e),
            );
            openraft::StorageError::IO {
                source: openraft::StorageIOError::read(&io_err),
            }
        })
    }

    fn sql_read_err(err: rusqlite::Error) -> openraft::StorageError<KNodeId> {
        let io_err = std::io::Error::other(format!("SQLite read error: {}", err));
        openraft::StorageError::IO {
            source: openraft::StorageIOError::read(&io_err),
        }
    }

    fn sql_write_err(err: rusqlite::Error) -> openraft::StorageError<KNodeId> {
        let io_err = std::io::Error::other(format!("SQLite write error: {}", err));
        openraft::StorageError::IO {
            source: openraft::StorageIOError::write(&io_err),
        }
    }

    fn u64_to_i64(v: u64) -> StorageResult<i64> {
        i64::try_from(v).map_err(|_| {
            let io_err = std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("value {} exceeds sqlite INTEGER range", v),
            );
            openraft::StorageError::IO {
                source: openraft::StorageIOError::write(&io_err),
            }
        })
    }

    fn build_range_query<RB: RangeBounds<u64>>(
        range: &RB,
    ) -> Result<(Option<String>, Vec<i64>), openraft::StorageError<KNodeId>> {
        let mut clauses = Vec::new();
        let mut params = Vec::new();

        match range.start_bound() {
            Bound::Included(v) => {
                if *v > SQLITE_LOG_INDEX_MAX_U64 {
                    return Ok((None, Vec::new()));
                }
                clauses.push("log_index >= ?".to_string());
                params.push(Self::u64_to_i64(*v)?);
            }
            Bound::Excluded(v) => {
                if *v >= SQLITE_LOG_INDEX_MAX_U64 {
                    return Ok((None, Vec::new()));
                }
                clauses.push("log_index > ?".to_string());
                params.push(Self::u64_to_i64(*v)?);
            }
            Bound::Unbounded => {}
        }

        match range.end_bound() {
            Bound::Included(v) => {
                if *v <= SQLITE_LOG_INDEX_MAX_U64 {
                    clauses.push("log_index <= ?".to_string());
                    params.push(Self::u64_to_i64(*v)?);
                }
            }
            Bound::Excluded(v) => {
                if *v == 0 {
                    return Ok((None, Vec::new()));
                }
                if *v <= SQLITE_LOG_INDEX_MAX_U64 {
                    clauses.push("log_index < ?".to_string());
                    params.push(Self::u64_to_i64(*v)?);
                }
            }
            Bound::Unbounded => {}
        }

        let sql = if clauses.is_empty() {
            "SELECT entry FROM raft_logs ORDER BY log_index ASC".to_string()
        } else {
            format!(
                "SELECT entry FROM raft_logs WHERE {} ORDER BY log_index ASC",
                clauses.join(" AND ")
            )
        };

        Ok((Some(sql), params))
    }

    async fn read_meta_value(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare("SELECT value FROM raft_meta WHERE key = ?1")
            .map_err(Self::sql_read_err)?;

        let row = stmt.query_row(params![key], |row| row.get::<_, Vec<u8>>(0));
        match row {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Self::sql_read_err(e)),
        }
    }

    async fn write_meta_value(&self, key: &str, value: &[u8]) -> StorageResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO raft_meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(Self::sql_write_err)?;

        Ok(())
    }

    async fn delete_meta_value(&self, key: &str) -> StorageResult<()> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM raft_meta WHERE key = ?1", params![key])
            .map_err(Self::sql_write_err)?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn append_entries_for_test<I>(&self, entries: I) -> StorageResult<()>
    where
        I: IntoIterator<Item = LogEntry>,
    {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction().map_err(Self::sql_write_err)?;

        for entry in entries {
            let idx = Self::u64_to_i64(entry.log_id.index)?;
            let encoded = Self::ser(PersistPayloadType::SqliteLogEntry, &entry)?;
            tx.execute(
                "INSERT INTO raft_logs(log_index, entry) VALUES(?1, ?2)
                 ON CONFLICT(log_index) DO UPDATE SET entry = excluded.entry",
                params![idx, encoded],
            )
            .map_err(Self::sql_write_err)?;
        }

        tx.commit().map_err(Self::sql_write_err)?;
        Ok(())
    }
}

impl RaftLogReader<KTypeConfig> for SqliteLogStorage {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> StorageResult<Vec<LogEntry>> {
        debug!("sqlite::try_get_log_entries: range={:?}", range);
        let (sql, params_i64) = Self::build_range_query(&range)?;
        let Some(sql) = sql else {
            return Ok(Vec::new());
        };

        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(&sql).map_err(Self::sql_read_err)?;

        let mut rows = match params_i64.as_slice() {
            [] => stmt.query([]).map_err(Self::sql_read_err)?,
            [p1] => stmt.query(params![p1]).map_err(Self::sql_read_err)?,
            [p1, p2] => stmt.query(params![p1, p2]).map_err(Self::sql_read_err)?,
            _ => {
                let io_err = std::io::Error::other(format!(
                    "Unexpected sqlite range parameter count: {}",
                    params_i64.len()
                ));
                return Err(openraft::StorageError::IO {
                    source: openraft::StorageIOError::read(&io_err),
                });
            }
        };

        let mut entries = Vec::new();
        while let Some(row) = rows.next().map_err(Self::sql_read_err)? {
            let bytes: Vec<u8> = row.get(0).map_err(Self::sql_read_err)?;
            let entry: LogEntry =
                Self::de(PersistPayloadType::SqliteLogEntry, &bytes, "raft log entry")?;
            if entry.get_membership().is_some() {
                debug!(
                    "sqlite::try_get_log_entries found membership entry: {:?}",
                    entry
                );
            }
            entries.push(entry);
        }

        Ok(entries)
    }
}

impl RaftLogStorage<KTypeConfig> for SqliteLogStorage {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> StorageResult<openraft::LogState<KTypeConfig>> {
        let last_log_id = {
            let conn = self.conn.lock().await;
            let mut stmt = conn
                .prepare("SELECT entry FROM raft_logs ORDER BY log_index DESC LIMIT 1")
                .map_err(Self::sql_read_err)?;

            let row = stmt.query_row([], |row| row.get::<_, Vec<u8>>(0));
            match row {
                Ok(bytes) => Some(
                    Self::de::<LogEntry>(
                        PersistPayloadType::SqliteLogEntry,
                        &bytes,
                        "last raft log entry",
                    )?
                    .log_id,
                ),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(Self::sql_read_err(e)),
            }
        };

        let last_purged_log_id = match self.read_meta_value(META_LAST_PURGED).await? {
            Some(v) => Some(Self::de::<LogId<KNodeId>>(
                PersistPayloadType::SqliteLastPurgedLogId,
                &v,
                "last purged log id",
            )?),
            None => None,
        };

        let last_log_id = match last_log_id {
            Some(id) => Some(id),
            None => last_purged_log_id,
        };

        Ok(openraft::LogState {
            last_log_id,
            last_purged_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote<KNodeId>) -> StorageResult<()> {
        debug!("sqlite::save_vote: {:?}", vote);
        let encoded = Self::ser(PersistPayloadType::SqliteVote, vote)?;
        self.write_meta_value(META_VOTE, &encoded).await
    }

    async fn read_vote(&mut self) -> StorageResult<Option<Vote<KNodeId>>> {
        let v = self.read_meta_value(META_VOTE).await?;
        match v {
            Some(bytes) => Ok(Some(Self::de::<Vote<KNodeId>>(
                PersistPayloadType::SqliteVote,
                &bytes,
                "vote",
            )?)),
            None => Ok(None),
        }
    }

    async fn save_committed(&mut self, committed: Option<LogId<KNodeId>>) -> StorageResult<()> {
        let current = self.read_committed().await?;
        if current == committed {
            return Ok(());
        }

        if let (Some(cur), Some(new)) = (current, committed.clone()) {
            if new < cur {
                warn!(
                    "sqlite::save_committed ignore rollback: current={}, incoming={}",
                    cur, new
                );
                return Ok(());
            }
        }

        match committed {
            Some(log_id) => {
                debug!("sqlite::save_committed: {}", log_id);
                let encoded = Self::ser(PersistPayloadType::SqliteCommittedLogId, &log_id)?;
                self.write_meta_value(META_COMMITTED, &encoded).await
            }
            None => {
                debug!("sqlite::save_committed clear committed");
                self.delete_meta_value(META_COMMITTED).await
            }
        }
    }

    async fn read_committed(&mut self) -> StorageResult<Option<LogId<KNodeId>>> {
        let v = self.read_meta_value(META_COMMITTED).await?;
        match v {
            Some(bytes) => Ok(Some(Self::de::<LogId<KNodeId>>(
                PersistPayloadType::SqliteCommittedLogId,
                &bytes,
                "committed log id",
            )?)),
            None => Ok(None),
        }
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<KTypeConfig>,
    ) -> StorageResult<()>
    where
        I: IntoIterator<Item = LogEntry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction().map_err(Self::sql_write_err)?;

        for entry in entries {
            debug!("sqlite::append raft log entry: {:?}", entry);
            let idx = Self::u64_to_i64(entry.log_id.index)?;
            let encoded = Self::ser(PersistPayloadType::SqliteLogEntry, &entry)?;
            tx.execute(
                "INSERT INTO raft_logs(log_index, entry) VALUES(?1, ?2)
                 ON CONFLICT(log_index) DO UPDATE SET entry = excluded.entry",
                params![idx, encoded],
            )
            .map_err(Self::sql_write_err)?;
        }

        tx.commit().map_err(Self::sql_write_err)?;
        callback.log_io_completed(Ok(()));

        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<KNodeId>) -> StorageResult<()> {
        info!("sqlite::truncate raft logs from index {}", log_id.index);
        let idx = Self::u64_to_i64(log_id.index)?;

        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM raft_logs WHERE log_index >= ?1", params![idx])
            .map_err(Self::sql_write_err)?;

        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<KNodeId>) -> StorageResult<()> {
        info!("sqlite::purge raft logs up to index {}", log_id.index);
        let idx = Self::u64_to_i64(log_id.index)?;

        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM raft_logs WHERE log_index <= ?1", params![idx])
            .map_err(Self::sql_write_err)?;
        drop(conn);

        let encoded = Self::ser(PersistPayloadType::SqliteLastPurgedLogId, &log_id)?;
        self.write_meta_value(META_LAST_PURGED, &encoded).await?;

        Ok(())
    }
}
