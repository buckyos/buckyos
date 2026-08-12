use openraft::entry::RaftPayload;
use openraft::storage::{LogFlushed, RaftLogStorage};
use openraft::{Entry, LogId, OptionalSend, RaftLogReader, Vote};
use rocksdb::{
    ColumnFamilyDescriptor, DB, Direction, IteratorMode, Options, WriteBatch, WriteOptions,
};
use std::fmt::Debug;
use std::ops::{Bound, RangeBounds};
use std::path::Path;
use std::sync::Arc;

use crate::util::persist_format::{PersistPayloadType, decode_with_header, encode_with_header};
use crate::{KNodeId, KTypeConfig, StorageResult};

type LogEntry = Entry<KTypeConfig>;

const META_VOTE: &[u8] = b"vote";
const META_LAST_PURGED: &[u8] = b"last_purged";
const META_COMMITTED: &[u8] = b"committed";
const CF_LOGS: &str = "raft_logs";
const CF_META: &str = "raft_meta";

#[derive(Debug, Clone)]
pub struct RocksDbLogStorage {
    db: Arc<DB>,
    sync_write: bool,
}

impl RocksDbLogStorage {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        Self::open_with_sync(path, true)
    }

    pub fn open_with_sync<P: AsRef<Path>>(path: P, sync_write: bool) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create rocksdb parent dir {}: {}",
                    parent.display(),
                    e
                )
            })?;
        }

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_atomic_flush(true);

        let mut logs_cf_opts = Options::default();
        logs_cf_opts.set_write_buffer_size(64 * 1024 * 1024);
        let mut meta_cf_opts = Options::default();
        meta_cf_opts.set_write_buffer_size(4 * 1024 * 1024);

        let cfs = vec![
            ColumnFamilyDescriptor::new(CF_LOGS, logs_cf_opts),
            ColumnFamilyDescriptor::new(CF_META, meta_cf_opts),
        ];

        let db = DB::open_cf_descriptors(&opts, path.as_ref(), cfs).map_err(|e| {
            format!(
                "Failed to open rocksdb raft log at {}: {}",
                path.as_ref().display(),
                e
            )
        })?;

        Ok(Self {
            db: Arc::new(db),
            sync_write,
        })
    }

    fn write_options(&self) -> WriteOptions {
        let mut opts = WriteOptions::default();
        opts.set_sync(self.sync_write);
        opts
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

    fn db_read_err(err: rocksdb::Error) -> openraft::StorageError<KNodeId> {
        let io_err = std::io::Error::other(format!("RocksDB read error: {}", err));
        openraft::StorageError::IO {
            source: openraft::StorageIOError::read(&io_err),
        }
    }

    fn db_write_err(err: rocksdb::Error) -> openraft::StorageError<KNodeId> {
        let io_err = std::io::Error::other(format!("RocksDB write error: {}", err));
        openraft::StorageError::IO {
            source: openraft::StorageIOError::write(&io_err),
        }
    }

    fn cf_handle_err(cf: &str) -> openraft::StorageError<KNodeId> {
        let io_err = std::io::Error::other(format!("Missing rocksdb column family '{}'", cf));
        openraft::StorageError::IO {
            source: openraft::StorageIOError::read(&io_err),
        }
    }

    fn entry_key(index: u64) -> [u8; 8] {
        index.to_be_bytes()
    }

    fn decode_entry_key(key: &[u8]) -> StorageResult<u64> {
        if key.len() != 8 {
            let io_err = std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid raft log key length: {}", key.len()),
            );
            return Err(openraft::StorageError::IO {
                source: openraft::StorageIOError::read(&io_err),
            });
        }

        let mut buf = [0u8; 8];
        buf.copy_from_slice(key);
        Ok(u64::from_be_bytes(buf))
    }

    fn read_meta_value(&self, key: &[u8]) -> StorageResult<Option<Vec<u8>>> {
        let meta_cf = self
            .db
            .cf_handle(CF_META)
            .ok_or_else(|| Self::cf_handle_err(CF_META))?;

        self.db.get_cf(&meta_cf, key).map_err(Self::db_read_err)
    }

    fn write_meta_value(&self, key: &[u8], value: &[u8]) -> StorageResult<()> {
        let meta_cf = self
            .db
            .cf_handle(CF_META)
            .ok_or_else(|| Self::cf_handle_err(CF_META))?;

        self.db
            .put_cf_opt(&meta_cf, key, value, &self.write_options())
            .map_err(Self::db_write_err)
    }

    fn delete_meta_value(&self, key: &[u8]) -> StorageResult<()> {
        let meta_cf = self
            .db
            .cf_handle(CF_META)
            .ok_or_else(|| Self::cf_handle_err(CF_META))?;

        self.db
            .delete_cf_opt(&meta_cf, key, &self.write_options())
            .map_err(Self::db_write_err)
    }

    #[cfg(test)]
    pub async fn append_entries_for_test<I>(&self, entries: I) -> StorageResult<()>
    where
        I: IntoIterator<Item = LogEntry>,
    {
        let logs_cf = self
            .db
            .cf_handle(CF_LOGS)
            .ok_or_else(|| Self::cf_handle_err(CF_LOGS))?;
        let mut batch = WriteBatch::default();
        for entry in entries {
            let key = Self::entry_key(entry.log_id.index);
            let encoded = Self::ser(PersistPayloadType::RocksDbLogEntry, &entry)?;
            batch.put_cf(&logs_cf, key, encoded);
        }

        self.db
            .write_opt(batch, &self.write_options())
            .map_err(Self::db_write_err)
    }

    fn in_end_bound<RB: RangeBounds<u64>>(range: &RB, idx: u64) -> bool {
        match range.end_bound() {
            Bound::Included(v) => idx <= *v,
            Bound::Excluded(v) => idx < *v,
            Bound::Unbounded => true,
        }
    }

    fn in_start_bound<RB: RangeBounds<u64>>(range: &RB, idx: u64) -> bool {
        match range.start_bound() {
            Bound::Included(v) => idx >= *v,
            Bound::Excluded(v) => idx > *v,
            Bound::Unbounded => true,
        }
    }
}

impl RaftLogReader<KTypeConfig> for RocksDbLogStorage {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> StorageResult<Vec<LogEntry>> {
        debug!("rocksdb::try_get_log_entries: range={:?}", range);

        let logs_cf = self
            .db
            .cf_handle(CF_LOGS)
            .ok_or_else(|| Self::cf_handle_err(CF_LOGS))?;

        let start_key = match range.start_bound() {
            Bound::Included(v) | Bound::Excluded(v) => Some(Self::entry_key(*v)),
            Bound::Unbounded => None,
        };
        let iter_mode = match start_key.as_ref() {
            Some(key) => IteratorMode::From(key, Direction::Forward),
            None => IteratorMode::Start,
        };

        let mut out = Vec::new();
        for item in self.db.iterator_cf(&logs_cf, iter_mode) {
            let (k, v) = item.map_err(Self::db_read_err)?;
            let idx = Self::decode_entry_key(k.as_ref())?;

            if !Self::in_start_bound(&range, idx) {
                continue;
            }
            if !Self::in_end_bound(&range, idx) {
                break;
            }

            let entry: LogEntry = Self::de(
                PersistPayloadType::RocksDbLogEntry,
                v.as_ref(),
                "raft log entry",
            )?;
            if entry.get_membership().is_some() {
                debug!(
                    "rocksdb::try_get_log_entries found membership entry: {:?}",
                    entry
                );
            }
            out.push(entry);
        }

        Ok(out)
    }
}

impl RaftLogStorage<KTypeConfig> for RocksDbLogStorage {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> StorageResult<openraft::LogState<KTypeConfig>> {
        let logs_cf = self
            .db
            .cf_handle(CF_LOGS)
            .ok_or_else(|| Self::cf_handle_err(CF_LOGS))?;

        let mut last_log_id = None;
        for item in self.db.iterator_cf(&logs_cf, IteratorMode::End) {
            let (_k, v) = item.map_err(Self::db_read_err)?;
            let entry: LogEntry = Self::de(
                PersistPayloadType::RocksDbLogEntry,
                v.as_ref(),
                "last raft log entry",
            )?;
            last_log_id = Some(entry.log_id);
            break;
        }

        let last_purged_log_id = match self.read_meta_value(META_LAST_PURGED)? {
            Some(v) => Some(Self::de::<LogId<KNodeId>>(
                PersistPayloadType::RocksDbLastPurgedLogId,
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
        debug!("rocksdb::save_vote: {:?}", vote);
        let encoded = Self::ser(PersistPayloadType::RocksDbVote, vote)?;
        self.write_meta_value(META_VOTE, &encoded)
    }

    async fn read_vote(&mut self) -> StorageResult<Option<Vote<KNodeId>>> {
        let v = self.read_meta_value(META_VOTE)?;
        match v {
            Some(bytes) => Ok(Some(Self::de::<Vote<KNodeId>>(
                PersistPayloadType::RocksDbVote,
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
                    "rocksdb::save_committed ignore rollback: current={}, incoming={}",
                    cur, new
                );
                return Ok(());
            }
        }

        match committed {
            Some(log_id) => {
                debug!("rocksdb::save_committed: {}", log_id);
                let encoded = Self::ser(PersistPayloadType::RocksDbCommittedLogId, &log_id)?;
                self.write_meta_value(META_COMMITTED, &encoded)
            }
            None => {
                debug!("rocksdb::save_committed clear committed");
                self.delete_meta_value(META_COMMITTED)
            }
        }
    }

    async fn read_committed(&mut self) -> StorageResult<Option<LogId<KNodeId>>> {
        let v = self.read_meta_value(META_COMMITTED)?;
        match v {
            Some(bytes) => Ok(Some(Self::de::<LogId<KNodeId>>(
                PersistPayloadType::RocksDbCommittedLogId,
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
        let logs_cf = self
            .db
            .cf_handle(CF_LOGS)
            .ok_or_else(|| Self::cf_handle_err(CF_LOGS))?;

        let mut batch = WriteBatch::default();
        for entry in entries {
            debug!("rocksdb::append raft log entry: {:?}", entry);
            let key = Self::entry_key(entry.log_id.index);
            let encoded = Self::ser(PersistPayloadType::RocksDbLogEntry, &entry)?;
            batch.put_cf(&logs_cf, key, encoded);
        }

        self.db
            .write_opt(batch, &self.write_options())
            .map_err(Self::db_write_err)?;
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<KNodeId>) -> StorageResult<()> {
        info!("rocksdb::truncate raft logs from index {}", log_id.index);
        let logs_cf = self
            .db
            .cf_handle(CF_LOGS)
            .ok_or_else(|| Self::cf_handle_err(CF_LOGS))?;

        let start_key = Self::entry_key(log_id.index);
        let mut batch = WriteBatch::default();
        for item in self
            .db
            .iterator_cf(&logs_cf, IteratorMode::From(&start_key, Direction::Forward))
        {
            let (k, _) = item.map_err(Self::db_read_err)?;
            batch.delete_cf(&logs_cf, k.as_ref());
        }

        self.db
            .write_opt(batch, &self.write_options())
            .map_err(Self::db_write_err)?;
        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<KNodeId>) -> StorageResult<()> {
        info!("rocksdb::purge raft logs up to index {}", log_id.index);
        let logs_cf = self
            .db
            .cf_handle(CF_LOGS)
            .ok_or_else(|| Self::cf_handle_err(CF_LOGS))?;

        let mut batch = WriteBatch::default();
        for item in self.db.iterator_cf(&logs_cf, IteratorMode::Start) {
            let (k, _) = item.map_err(Self::db_read_err)?;
            let idx = Self::decode_entry_key(k.as_ref())?;
            if idx > log_id.index {
                break;
            }
            batch.delete_cf(&logs_cf, k.as_ref());
        }

        let encoded = Self::ser(PersistPayloadType::RocksDbLastPurgedLogId, &log_id)?;
        let meta_cf = self
            .db
            .cf_handle(CF_META)
            .ok_or_else(|| Self::cf_handle_err(CF_META))?;
        batch.put_cf(&meta_cf, META_LAST_PURGED, encoded);

        self.db
            .write_opt(batch, &self.write_options())
            .map_err(Self::db_write_err)?;

        Ok(())
    }
}
