use super::store::{
    KLogQuery, KLogQueryOrder, KLogStateMachineMeta, KLogStateSnapshot, KLogStateSnapshotData,
    KLogStateStore, REQUEST_DEDUP_WINDOW_MS,
};
use crate::{KLogEntry, KLogError, KLogLevel, KLogMetaEntry, KResult};
use rocksdb::backup::{BackupEngine, BackupEngineOptions, RestoreOptions};
use rocksdb::checkpoint::Checkpoint;
use rocksdb::{
    ColumnFamilyDescriptor, DB, DEFAULT_COLUMN_FAMILY_NAME, Direction, Env, IteratorMode, Options,
    WriteBatch, WriteOptions,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const KEY_PREFIX_ENTRY: u8 = b'e';
const KEY_NEXT_LOG_ID_META: &[u8] = b"m:next_log_id";
const KEY_STATE_MACHINE_META: &[u8] = b"m:state_machine_meta";
const KEY_REQUEST_DEDUP_PREFIX: &[u8] = b"m:req:";
const KEY_DATA_META_PREFIX: &[u8] = b"d:";
const CF_LOGS: &str = "logs";
const CF_META: &str = "meta";
const CF_INDEX_LEVEL: &str = "idx_level";
const CF_INDEX_SOURCE: &str = "idx_source";
const CHECKPOINT_SNAPSHOT_MAGIC: &str = "klog-rdb-checkpoint-v1";
const CHECKPOINT_SNAPSHOT_PREFIX: &[u8] = b"KLOG_RDB_CP1";
const BACKUP_ENGINE_SNAPSHOT_MAGIC: &str = "klog-rdb-backup-v1";
const BACKUP_ENGINE_SNAPSHOT_PREFIX: &[u8] = b"KLOG_RDB_BK1";
static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RequestDedupMeta {
    log_id: u64,
    seen_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyKLogMetaEntryV0 {
    key: String,
    value: String,
    updated_at: u64,
    updated_by: u64,
}

fn entry_key(id: u64) -> [u8; 9] {
    let mut key = [0u8; 9];
    key[0] = KEY_PREFIX_ENTRY;
    key[1..].copy_from_slice(&id.to_be_bytes());
    key
}

fn decode_entry_key(key: &[u8]) -> Option<u64> {
    if key.len() != 9 || key[0] != KEY_PREFIX_ENTRY {
        return None;
    }

    let mut id = [0u8; 8];
    id.copy_from_slice(&key[1..]);
    Some(u64::from_be_bytes(id))
}

fn klog_err(msg: impl Into<String>) -> KLogError {
    KLogError::InvalidFormat(msg.into())
}

fn klog_err_with_context<E: std::fmt::Display>(context: impl Into<String>, err: E) -> KLogError {
    let msg = format!("{}: {}", context.into(), err);
    error!("{}", msg);
    klog_err(msg)
}

fn decode_u64_be(bytes: &[u8]) -> KResult<u64> {
    if bytes.len() != 8 {
        let msg = format!("Invalid u64 bytes length: {}", bytes.len());
        error!("{}", msg);
        return Err(klog_err(msg));
    }
    let mut v = [0u8; 8];
    v.copy_from_slice(bytes);
    Ok(u64::from_be_bytes(v))
}

fn summarize_entry_ids(entries: &[KLogEntry]) -> String {
    let Some(first) = entries.first() else {
        return "none".to_string();
    };
    let Some(last) = entries.last() else {
        return "none".to_string();
    };

    format!("{}..={} ({} entries)", first.id, last.id, entries.len())
}

fn request_dedup_meta_key(request_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(KEY_REQUEST_DEDUP_PREFIX.len() + request_id.len());
    key.extend_from_slice(KEY_REQUEST_DEDUP_PREFIX);
    key.extend_from_slice(request_id.as_bytes());
    key
}

fn data_meta_key(key: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(KEY_DATA_META_PREFIX.len() + key.len());
    out.extend_from_slice(KEY_DATA_META_PREFIX);
    out.extend_from_slice(key.as_bytes());
    out
}

fn decode_data_meta_key(raw: &[u8]) -> Option<&str> {
    if !raw.starts_with(KEY_DATA_META_PREFIX) {
        return None;
    }
    std::str::from_utf8(&raw[KEY_DATA_META_PREFIX.len()..]).ok()
}

fn normalize_request_id(request_id: Option<&str>) -> Option<&str> {
    request_id.map(|v| v.trim()).filter(|v| !v.is_empty())
}

fn normalize_source(source: Option<&str>) -> Option<&str> {
    source.map(|v| v.trim()).filter(|v| !v.is_empty())
}

fn level_index_code(level: KLogLevel) -> u8 {
    match level {
        KLogLevel::Trace => 1,
        KLogLevel::Debug => 2,
        KLogLevel::Info => 3,
        KLogLevel::Warn => 4,
        KLogLevel::Error => 5,
        KLogLevel::Fatal => 6,
    }
}

fn level_index_key(level: KLogLevel, id: u64) -> [u8; 9] {
    let mut key = [0u8; 9];
    key[0] = level_index_code(level);
    key[1..].copy_from_slice(&id.to_be_bytes());
    key
}

fn decode_level_index_id(key: &[u8], level: KLogLevel) -> Option<u64> {
    if key.len() != 9 || key[0] != level_index_code(level) {
        return None;
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(&key[1..]);
    Some(u64::from_be_bytes(id))
}

fn source_index_prefix(source: &str) -> Vec<u8> {
    let source_bytes = source.as_bytes();
    let mut key = Vec::with_capacity(4 + source_bytes.len());
    key.extend_from_slice(&(source_bytes.len() as u32).to_be_bytes());
    key.extend_from_slice(source_bytes);
    key
}

fn source_index_key(source: &str, id: u64) -> Vec<u8> {
    let mut key = source_index_prefix(source);
    key.extend_from_slice(&id.to_be_bytes());
    key
}

fn decode_source_index_id(key: &[u8], source: &str) -> Option<u64> {
    let prefix = source_index_prefix(source);
    if key.len() != prefix.len() + 8 || !key.starts_with(&prefix) {
        return None;
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(&key[prefix.len()..]);
    Some(u64::from_be_bytes(id))
}

fn entry_matches_query(entry: &KLogEntry, query: &KLogQuery) -> bool {
    if let Some(level) = query.level
        && entry.level != level
    {
        return false;
    }
    if let Some(source) = query
        .source
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let entry_source = normalize_source(entry.source.as_deref());
        if entry_source != Some(source) {
            return false;
        }
    }

    let attr_key = query
        .attr_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let attr_value = query
        .attr_value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if attr_key.is_none() && attr_value.is_some() {
        return false;
    }
    if let Some(attr_key) = attr_key {
        let Some(actual) = entry.attrs.get(attr_key) else {
            return false;
        };
        if let Some(expected) = attr_value
            && actual != expected
        {
            return false;
        }
    }

    true
}

fn decode_meta_entry_with_legacy(raw: &[u8]) -> KResult<KLogMetaEntry> {
    let decoded_v1: Result<(KLogMetaEntry, usize), _> =
        bincode::serde::decode_from_slice(raw, bincode::config::legacy());
    if let Ok((item, _)) = decoded_v1 {
        return Ok(item);
    }

    let (legacy, _): (LegacyKLogMetaEntryV0, usize) =
        bincode::serde::decode_from_slice(raw, bincode::config::legacy())
            .map_err(|e| klog_err_with_context("Failed to decode rocksdb data meta entry", e))?;
    Ok(KLogMetaEntry {
        key: legacy.key,
        value: legacy.value,
        updated_at: legacy.updated_at,
        updated_by_node_name: legacy.updated_by.to_string(),
        revision: 1,
    })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);

    std::env::temp_dir().join(format!(
        "buckyos_klog_rdb_{}_{}_{}_{}",
        prefix,
        std::process::id(),
        nanos,
        seq
    ))
}

fn open_rocksdb_with_cfs(path: &Path, create_if_missing: bool) -> Result<DB, String> {
    if create_if_missing {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                let msg = format!(
                    "Failed to create rocksdb parent dir {}: {}",
                    parent.display(),
                    e
                );
                error!("{}", msg);
                msg
            })?;
        }
    }

    let mut opts = Options::default();
    opts.create_if_missing(create_if_missing);
    opts.create_missing_column_families(true);
    opts.set_atomic_flush(true);

    let mut logs_cf_opts = Options::default();
    logs_cf_opts.set_write_buffer_size(64 * 1024 * 1024);
    let mut meta_cf_opts = Options::default();
    meta_cf_opts.set_write_buffer_size(4 * 1024 * 1024);
    let mut idx_cf_opts = Options::default();
    idx_cf_opts.set_write_buffer_size(8 * 1024 * 1024);

    let cfs = vec![
        ColumnFamilyDescriptor::new(CF_LOGS, logs_cf_opts),
        ColumnFamilyDescriptor::new(CF_META, meta_cf_opts),
        ColumnFamilyDescriptor::new(CF_INDEX_LEVEL, idx_cf_opts.clone()),
        ColumnFamilyDescriptor::new(CF_INDEX_SOURCE, idx_cf_opts),
    ];

    DB::open_cf_descriptors(&opts, path, cfs).map_err(|e| {
        let msg = format!(
            "Failed to open rocksdb at {} with cfs: {}",
            path.display(),
            e
        );
        error!("{}", msg);
        msg
    })
}

fn build_write_options(sync_write: bool) -> WriteOptions {
    let mut opts = WriteOptions::default();
    opts.set_sync(sync_write);
    opts
}

fn migrate_legacy_default_cf_data(db: &DB) -> Result<(), String> {
    let default_cf = db.cf_handle(DEFAULT_COLUMN_FAMILY_NAME).ok_or_else(|| {
        let msg = "Missing default column family".to_string();
        error!("{}", msg);
        msg
    })?;
    let logs_cf = db.cf_handle(CF_LOGS).ok_or_else(|| {
        let msg = format!("Missing column family '{}'", CF_LOGS);
        error!("{}", msg);
        msg
    })?;
    let meta_cf = db.cf_handle(CF_META).ok_or_else(|| {
        let msg = format!("Missing column family '{}'", CF_META);
        error!("{}", msg);
        msg
    })?;

    let mut batch = WriteBatch::default();
    let mut migrated_logs = 0usize;
    let mut migrated_meta = 0usize;

    for item in db.iterator_cf(&default_cf, IteratorMode::Start) {
        let (k, v) = item.map_err(|e| {
            let msg = format!("Failed to iterate default CF for migration: {}", e);
            error!("{}", msg);
            msg
        })?;

        if decode_entry_key(&k).is_some() {
            batch.put_cf(&logs_cf, k.as_ref(), v.as_ref());
            batch.delete_cf(&default_cf, k.as_ref());
            migrated_logs += 1;
            continue;
        }

        if k.as_ref() == KEY_NEXT_LOG_ID_META || k.as_ref() == KEY_STATE_MACHINE_META {
            batch.put_cf(&meta_cf, k.as_ref(), v.as_ref());
            batch.delete_cf(&default_cf, k.as_ref());
            migrated_meta += 1;
        }
    }

    if migrated_logs > 0 || migrated_meta > 0 {
        let write_opts = build_write_options(true);
        db.write_opt(batch, &write_opts).map_err(|e| {
            let msg = format!("Failed to write legacy default-CF migration batch: {}", e);
            error!("{}", msg);
            msg
        })?;
        info!(
            "RocksDbStateStore migrated legacy default CF data: logs={}, meta={}",
            migrated_logs, migrated_meta
        );
    }

    Ok(())
}

fn rebuild_log_indexes_if_needed(db: &DB) -> Result<(), String> {
    let logs_cf = db.cf_handle(CF_LOGS).ok_or_else(|| {
        let msg = format!("Missing column family '{}'", CF_LOGS);
        error!("{}", msg);
        msg
    })?;
    let idx_level_cf = db.cf_handle(CF_INDEX_LEVEL).ok_or_else(|| {
        let msg = format!("Missing column family '{}'", CF_INDEX_LEVEL);
        error!("{}", msg);
        msg
    })?;
    let idx_source_cf = db.cf_handle(CF_INDEX_SOURCE).ok_or_else(|| {
        let msg = format!("Missing column family '{}'", CF_INDEX_SOURCE);
        error!("{}", msg);
        msg
    })?;

    let has_logs = db
        .iterator_cf(&logs_cf, IteratorMode::Start)
        .find_map(|item| match item {
            Ok((k, _)) if decode_entry_key(k.as_ref()).is_some() => Some(true),
            Ok(_) => None,
            Err(_) => Some(false),
        })
        .unwrap_or(false);
    if !has_logs {
        return Ok(());
    }

    let has_level_index = db
        .iterator_cf(&idx_level_cf, IteratorMode::Start)
        .next()
        .is_some();
    if has_level_index {
        return Ok(());
    }

    info!("RocksDbStateStore rebuilding secondary indexes from logs");
    let mut batch = WriteBatch::default();
    for item in db.iterator_cf(&idx_level_cf, IteratorMode::Start) {
        let (k, _) = item.map_err(|e| format!("Failed to iterate level index cf: {}", e))?;
        batch.delete_cf(&idx_level_cf, k.as_ref());
    }
    for item in db.iterator_cf(&idx_source_cf, IteratorMode::Start) {
        let (k, _) = item.map_err(|e| format!("Failed to iterate source index cf: {}", e))?;
        batch.delete_cf(&idx_source_cf, k.as_ref());
    }

    let mut rebuilt = 0usize;
    for item in db.iterator_cf(&logs_cf, IteratorMode::Start) {
        let (k, v) =
            item.map_err(|e| format!("Failed to iterate logs cf for index rebuild: {}", e))?;
        if decode_entry_key(k.as_ref()).is_none() {
            continue;
        }
        let (entry, _): (KLogEntry, usize) =
            bincode::serde::decode_from_slice(v.as_ref(), bincode::config::legacy())
                .map_err(|e| format!("Failed to decode entry while rebuilding indexes: {}", e))?;
        batch.put_cf(&idx_level_cf, level_index_key(entry.level, entry.id), []);
        if let Some(source) = normalize_source(entry.source.as_deref()) {
            batch.put_cf(&idx_source_cf, source_index_key(source, entry.id), []);
        }
        rebuilt += 1;
    }

    let write_opts = build_write_options(true);
    db.write_opt(batch, &write_opts)
        .map_err(|e| format!("Failed to persist rebuilt log indexes: {}", e))?;
    info!(
        "RocksDbStateStore rebuilt secondary indexes from logs: entries={}",
        rebuilt
    );
    Ok(())
}

fn create_rocksdb(path: &Path) -> Result<DB, String> {
    let db = open_rocksdb_with_cfs(path, true)?;
    migrate_legacy_default_cf_data(&db)?;
    rebuild_log_indexes_if_needed(&db)?;
    Ok(db)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RocksDbSnapshotMode {
    Enumerate,
    Checkpoint,
    BackupEngine,
}

pub trait RocksDbSnapshotStrategy: Send + Sync {
    fn mode(&self) -> RocksDbSnapshotMode;

    fn build_snapshot(&self, store: &RocksDbStateStore) -> KResult<KLogStateSnapshot>;

    fn try_install_snapshot(
        &self,
        store: &RocksDbStateStore,
        snapshot: &KLogStateSnapshot,
    ) -> KResult<bool>;
}

#[derive(Debug, Default)]
pub struct EnumerateSnapshotStrategy;

#[derive(Debug, Default)]
pub struct CheckpointSnapshotStrategy;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotFileBlob {
    relative_path: String,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointSnapshotArchive {
    magic: String,
    files: Vec<SnapshotFileBlob>,
}

#[derive(Debug, Default)]
pub struct BackupEngineSnapshotStrategy;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupEngineSnapshotArchive {
    magic: String,
    files: Vec<SnapshotFileBlob>,
}

/// RocksDB-backed state store for high-write kernel logs.
#[derive(Clone)]
pub struct RocksDbStateStore {
    db: Arc<DB>,
    snapshot_mode: RocksDbSnapshotMode,
    sync_write: bool,
    snapshot_builder: Arc<dyn RocksDbSnapshotStrategy>,
    snapshot_installers: Vec<Arc<dyn RocksDbSnapshotStrategy>>,
}

impl std::fmt::Debug for RocksDbStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocksDbStateStore")
            .field("snapshot_mode", &self.snapshot_mode)
            .field("sync_write", &self.sync_write)
            .finish()
    }
}

impl RocksDbStateStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        Self::open_with_mode_and_sync(path, RocksDbSnapshotMode::Checkpoint, true)
    }

    pub fn open_with_mode<P: AsRef<Path>>(
        path: P,
        snapshot_mode: RocksDbSnapshotMode,
    ) -> Result<Self, String> {
        Self::open_with_mode_and_sync(path, snapshot_mode, true)
    }

    pub fn open_with_mode_and_sync<P: AsRef<Path>>(
        path: P,
        snapshot_mode: RocksDbSnapshotMode,
        sync_write: bool,
    ) -> Result<Self, String> {
        info!(
            "RocksDbStateStore open_with_mode: path={}, snapshot_mode={:?}, sync_write={}",
            path.as_ref().display(),
            snapshot_mode,
            sync_write
        );
        let db = create_rocksdb(path.as_ref())?;

        let snapshot_builder = strategy_for_mode(snapshot_mode);
        let mut snapshot_installers = Vec::new();
        snapshot_installers.push(snapshot_builder.clone());

        for mode in [
            RocksDbSnapshotMode::Enumerate,
            RocksDbSnapshotMode::Checkpoint,
            RocksDbSnapshotMode::BackupEngine,
        ] {
            if mode != snapshot_mode {
                snapshot_installers.push(strategy_for_mode(mode));
            }
        }

        Ok(Self {
            db: Arc::new(db),
            snapshot_mode,
            sync_write,
            snapshot_builder,
            snapshot_installers,
        })
    }

    fn write_options(&self) -> WriteOptions {
        build_write_options(self.sync_write)
    }

    fn read_persisted_next_log_id(&self) -> KResult<Option<u64>> {
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let value = self
            .db
            .get_cf(&meta_cf, KEY_NEXT_LOG_ID_META)
            .map_err(|e| klog_err_with_context("Failed to read rocksdb next_log_id metadata", e))?;
        let Some(raw) = value else {
            return Ok(None);
        };
        let next_log_id = decode_u64_be(raw.as_ref())?;
        Ok(Some(next_log_id))
    }

    fn scan_next_log_id_from_entries(&self) -> KResult<u64> {
        let logs_cf = self.db.cf_handle(CF_LOGS).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_LOGS);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let mut max_id = 0u64;
        for item in self.db.iterator_cf(&logs_cf, IteratorMode::Start) {
            let (k, _) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate rocksdb while scanning next_log_id", e)
            })?;
            if let Some(id) = decode_entry_key(&k) {
                if id > max_id {
                    max_id = id;
                }
            }
        }

        Ok(max_id.saturating_add(1).max(1))
    }

    fn persist_next_log_id(&self, next_log_id: u64) -> KResult<()> {
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let write_opts = self.write_options();
        self.db
            .put_cf_opt(
                &meta_cf,
                KEY_NEXT_LOG_ID_META,
                next_log_id.to_be_bytes(),
                &write_opts,
            )
            .map_err(|e| klog_err_with_context("Failed to persist rocksdb next_log_id metadata", e))
    }

    fn read_persisted_state_machine_meta(&self) -> KResult<Option<KLogStateMachineMeta>> {
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;

        let value = self
            .db
            .get_cf(&meta_cf, KEY_STATE_MACHINE_META)
            .map_err(|e| {
                klog_err_with_context("Failed to read rocksdb state-machine metadata", e)
            })?;
        let Some(raw) = value else {
            return Ok(None);
        };

        let (meta, _): (KLogStateMachineMeta, usize) =
            bincode::serde::decode_from_slice(raw.as_ref(), bincode::config::legacy()).map_err(
                |e| klog_err_with_context("Failed to decode rocksdb state-machine metadata", e),
            )?;
        Ok(Some(meta))
    }

    fn persist_state_machine_meta(&self, meta: &KLogStateMachineMeta) -> KResult<()> {
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;

        let bytes =
            bincode::serde::encode_to_vec(meta, bincode::config::legacy()).map_err(|e| {
                klog_err_with_context("Failed to encode rocksdb state-machine metadata", e)
            })?;
        let write_opts = self.write_options();
        self.db
            .put_cf_opt(&meta_cf, KEY_STATE_MACHINE_META, bytes, &write_opts)
            .map_err(|e| {
                klog_err_with_context("Failed to persist rocksdb state-machine metadata", e)
            })
    }

    fn resolve_next_log_id(&self) -> KResult<u64> {
        let from_entries = self.scan_next_log_id_from_entries()?;
        let from_meta = self.read_persisted_next_log_id()?.unwrap_or(1);
        let resolved = from_entries.max(from_meta).max(1);

        if resolved != from_meta {
            self.persist_next_log_id(resolved)?;
            info!(
                "RocksDbStateStore next_log_id metadata reconciled: meta={}, entries={}, resolved={}",
                from_meta, from_entries, resolved
            );
        } else {
            debug!(
                "RocksDbStateStore next_log_id metadata loaded: {}",
                resolved
            );
        }

        Ok(resolved)
    }

    fn read_all_entries(&self) -> KResult<Vec<KLogEntry>> {
        debug!(
            "RocksDbStateStore read_all_entries start: snapshot_mode={:?}",
            self.snapshot_mode
        );
        let logs_cf = self.db.cf_handle(CF_LOGS).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_LOGS);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let mut entries = Vec::new();
        for item in self.db.iterator_cf(&logs_cf, IteratorMode::Start) {
            let (k, v) =
                item.map_err(|e| klog_err_with_context("Failed to iterate rocksdb entry", e))?;

            if decode_entry_key(&k).is_none() {
                continue;
            }

            let (entry, _): (KLogEntry, usize) =
                bincode::serde::decode_from_slice(v.as_ref(), bincode::config::legacy()).map_err(
                    |e| klog_err_with_context("Failed to deserialize state entry from rocksdb", e),
                )?;
            entries.push(entry);
        }
        debug!(
            "RocksDbStateStore read_all_entries done: snapshot_mode={:?}, entries={}",
            self.snapshot_mode,
            summarize_entry_ids(&entries)
        );

        Ok(entries)
    }

    fn read_all_meta_entries(&self) -> KResult<Vec<KLogMetaEntry>> {
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;

        let mut out = Vec::new();
        let iter = self.db.iterator_cf(
            &meta_cf,
            IteratorMode::From(KEY_DATA_META_PREFIX, Direction::Forward),
        );
        for item in iter {
            let (k, v) =
                item.map_err(|e| klog_err_with_context("Failed to iterate rocksdb meta entry", e))?;
            if !k.as_ref().starts_with(KEY_DATA_META_PREFIX) {
                break;
            }

            let Some(key) = decode_data_meta_key(k.as_ref()) else {
                continue;
            };
            let entry = decode_meta_entry_with_legacy(v.as_ref())?;
            if entry.key != key {
                warn!(
                    "RocksDbStateStore meta key mismatch, key_from_index='{}', key_in_value='{}'",
                    key, entry.key
                );
            }
            out.push(entry);
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    fn clear_entries_in_batch(&self, batch: &mut WriteBatch) -> KResult<()> {
        debug!(
            "RocksDbStateStore clear_entries_in_batch start: snapshot_mode={:?}",
            self.snapshot_mode
        );
        let logs_cf = self.db.cf_handle(CF_LOGS).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_LOGS);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let mut deleted = 0usize;
        for item in self.db.iterator_cf(&logs_cf, IteratorMode::Start) {
            let (k, _) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate rocksdb while clearing entries", e)
            })?;
            if decode_entry_key(&k).is_some() {
                batch.delete_cf(&logs_cf, k);
                deleted += 1;
            }
        }
        debug!(
            "RocksDbStateStore clear_entries_in_batch done: deleted_entries={}",
            deleted
        );
        Ok(())
    }

    fn clear_request_dedup_in_batch(&self, batch: &mut WriteBatch) -> KResult<()> {
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let mut deleted = 0usize;
        let iter = self.db.iterator_cf(
            &meta_cf,
            IteratorMode::From(KEY_REQUEST_DEDUP_PREFIX, Direction::Forward),
        );
        for item in iter {
            let (k, _) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate rocksdb while clearing request dedup", e)
            })?;
            if !k.as_ref().starts_with(KEY_REQUEST_DEDUP_PREFIX) {
                break;
            }
            batch.delete_cf(&meta_cf, k.as_ref());
            deleted += 1;
        }
        debug!(
            "RocksDbStateStore clear_request_dedup_in_batch done: deleted={}",
            deleted
        );
        Ok(())
    }

    fn clear_data_meta_in_batch(&self, batch: &mut WriteBatch) -> KResult<()> {
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let mut deleted = 0usize;
        let iter = self.db.iterator_cf(
            &meta_cf,
            IteratorMode::From(KEY_DATA_META_PREFIX, Direction::Forward),
        );
        for item in iter {
            let (k, _) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate rocksdb while clearing data meta", e)
            })?;
            if !k.as_ref().starts_with(KEY_DATA_META_PREFIX) {
                break;
            }
            batch.delete_cf(&meta_cf, k.as_ref());
            deleted += 1;
        }
        debug!(
            "RocksDbStateStore clear_data_meta_in_batch done: deleted={}",
            deleted
        );
        Ok(())
    }

    fn clear_indexes_in_batch(&self, batch: &mut WriteBatch) -> KResult<()> {
        let level_cf = self.db.cf_handle(CF_INDEX_LEVEL).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_INDEX_LEVEL);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let source_cf = self.db.cf_handle(CF_INDEX_SOURCE).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_INDEX_SOURCE);
            error!("{}", msg);
            klog_err(msg)
        })?;

        let mut deleted_level = 0usize;
        for item in self.db.iterator_cf(&level_cf, IteratorMode::Start) {
            let (k, _) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate rocksdb while clearing level index", e)
            })?;
            batch.delete_cf(&level_cf, k.as_ref());
            deleted_level += 1;
        }

        let mut deleted_source = 0usize;
        for item in self.db.iterator_cf(&source_cf, IteratorMode::Start) {
            let (k, _) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate rocksdb while clearing source index", e)
            })?;
            batch.delete_cf(&source_cf, k.as_ref());
            deleted_source += 1;
        }

        debug!(
            "RocksDbStateStore clear_indexes_in_batch done: deleted_level={}, deleted_source={}",
            deleted_level, deleted_source
        );
        Ok(())
    }

    fn replace_with_entries(
        &self,
        entries: Vec<KLogEntry>,
        meta_entries: Vec<KLogMetaEntry>,
    ) -> KResult<()> {
        info!(
            "RocksDbStateStore replace_with_entries start: snapshot_mode={:?}, incoming_entries={}, incoming_meta_entries={}",
            self.snapshot_mode,
            summarize_entry_ids(&entries),
            meta_entries.len()
        );
        let mut batch = WriteBatch::default();
        self.clear_entries_in_batch(&mut batch)?;
        self.clear_request_dedup_in_batch(&mut batch)?;
        self.clear_data_meta_in_batch(&mut batch)?;
        self.clear_indexes_in_batch(&mut batch)?;
        let logs_cf = self.db.cf_handle(CF_LOGS).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_LOGS);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let idx_level_cf = self.db.cf_handle(CF_INDEX_LEVEL).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_INDEX_LEVEL);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let idx_source_cf = self.db.cf_handle(CF_INDEX_SOURCE).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_INDEX_SOURCE);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;

        let mut max_id = 0u64;
        for entry in entries {
            if entry.id > max_id {
                max_id = entry.id;
            }
            let key = entry_key(entry.id);
            let value =
                bincode::serde::encode_to_vec(&entry, bincode::config::legacy()).map_err(|e| {
                    klog_err_with_context("Failed to serialize state entry for rocksdb install", e)
                })?;
            batch.put_cf(&logs_cf, key, value);
            batch.put_cf(&idx_level_cf, level_index_key(entry.level, entry.id), []);
            if let Some(source) = normalize_source(entry.source.as_deref()) {
                batch.put_cf(&idx_source_cf, source_index_key(source, entry.id), []);
            }
        }
        let next_log_id = max_id.saturating_add(1).max(1);
        batch.put_cf(&meta_cf, KEY_NEXT_LOG_ID_META, next_log_id.to_be_bytes());
        for item in meta_entries {
            let encoded = bincode::serde::encode_to_vec(&item, bincode::config::legacy())
                .map_err(|e| klog_err_with_context("Failed to encode data meta entry", e))?;
            let key = data_meta_key(item.key.as_str());
            batch.put_cf(&meta_cf, key, encoded);
        }

        let write_opts = self.write_options();
        self.db.write_opt(batch, &write_opts).map_err(|e| {
            klog_err_with_context("Failed to write entries during rocksdb snapshot install", e)
        })?;
        info!(
            "RocksDbStateStore replace_with_entries completed: snapshot_mode={:?}, next_log_id={}",
            self.snapshot_mode, next_log_id
        );

        Ok(())
    }

    fn replace_with_db(&self, source_db: &DB) -> KResult<()> {
        info!(
            "RocksDbStateStore replace_with_db start: snapshot_mode={:?}",
            self.snapshot_mode
        );
        let mut batch = WriteBatch::default();
        self.clear_entries_in_batch(&mut batch)?;
        self.clear_request_dedup_in_batch(&mut batch)?;
        self.clear_data_meta_in_batch(&mut batch)?;
        self.clear_indexes_in_batch(&mut batch)?;
        let logs_cf = self.db.cf_handle(CF_LOGS).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_LOGS);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let idx_level_cf = self.db.cf_handle(CF_INDEX_LEVEL).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_INDEX_LEVEL);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let idx_source_cf = self.db.cf_handle(CF_INDEX_SOURCE).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_INDEX_SOURCE);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let source_logs_cf = source_db.cf_handle(CF_LOGS).ok_or_else(|| {
            let msg = format!("Missing source column family '{}'", CF_LOGS);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let source_meta_cf = source_db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing source column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;

        let mut copied = 0usize;
        let mut copied_dedup = 0usize;
        let mut copied_meta = 0usize;
        let mut max_id = 0u64;
        for item in source_db.iterator_cf(&source_logs_cf, IteratorMode::Start) {
            let (k, v) = item
                .map_err(|e| klog_err_with_context("Failed to iterate source checkpoint db", e))?;
            let Some(id) = decode_entry_key(&k) else {
                continue;
            };
            let (entry, _): (KLogEntry, usize) =
                bincode::serde::decode_from_slice(v.as_ref(), bincode::config::legacy()).map_err(
                    |e| klog_err_with_context("Failed to decode source entry for index rebuild", e),
                )?;
            batch.put_cf(&logs_cf, k.as_ref(), v.as_ref());
            batch.put_cf(&idx_level_cf, level_index_key(entry.level, entry.id), []);
            if let Some(source) = normalize_source(entry.source.as_deref()) {
                batch.put_cf(&idx_source_cf, source_index_key(source, entry.id), []);
            }
            copied += 1;
            if id > max_id {
                max_id = id;
            }
        }
        let next_log_id = max_id.saturating_add(1).max(1);
        batch.put_cf(&meta_cf, KEY_NEXT_LOG_ID_META, next_log_id.to_be_bytes());

        let dedup_iter = source_db.iterator_cf(
            &source_meta_cf,
            IteratorMode::From(KEY_REQUEST_DEDUP_PREFIX, Direction::Forward),
        );
        for item in dedup_iter {
            let (k, v) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate source request dedup index", e)
            })?;
            if !k.as_ref().starts_with(KEY_REQUEST_DEDUP_PREFIX) {
                break;
            }
            batch.put_cf(&meta_cf, k.as_ref(), v.as_ref());
            copied_dedup += 1;
        }
        let meta_iter = source_db.iterator_cf(
            &source_meta_cf,
            IteratorMode::From(KEY_DATA_META_PREFIX, Direction::Forward),
        );
        for item in meta_iter {
            let (k, v) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate source data meta entries", e)
            })?;
            if !k.as_ref().starts_with(KEY_DATA_META_PREFIX) {
                break;
            }
            batch.put_cf(&meta_cf, k.as_ref(), v.as_ref());
            copied_meta += 1;
        }

        let write_opts = self.write_options();
        self.db.write_opt(batch, &write_opts).map_err(|e| {
            klog_err_with_context("Failed to apply checkpoint snapshot into rocksdb", e)
        })?;
        info!(
            "RocksDbStateStore replace_with_db completed: copied_entries={}, copied_dedup={}, copied_meta={}, next_log_id={}",
            copied, copied_dedup, copied_meta, next_log_id
        );

        Ok(())
    }

    fn build_checkpoint_archive(&self) -> KResult<CheckpointSnapshotArchive> {
        let checkpoint_dir = unique_temp_dir("checkpoint_build");
        info!(
            "RocksDbStateStore build_checkpoint_archive start: checkpoint_dir={}, mode={:?}",
            checkpoint_dir.display(),
            self.snapshot_mode
        );
        let result = (|| -> KResult<CheckpointSnapshotArchive> {
            let checkpoint = Checkpoint::new(self.db.as_ref()).map_err(|e| {
                klog_err_with_context("Failed to create rocksdb checkpoint object", e)
            })?;

            checkpoint.create_checkpoint(&checkpoint_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to create rocksdb checkpoint in {}",
                        checkpoint_dir.display()
                    ),
                    e,
                )
            })?;

            let files = collect_snapshot_files(&checkpoint_dir)?;
            info!(
                "RocksDbStateStore build_checkpoint_archive files collected: checkpoint_dir={}, files={}",
                checkpoint_dir.display(),
                files.len()
            );
            Ok(CheckpointSnapshotArchive {
                magic: CHECKPOINT_SNAPSHOT_MAGIC.to_string(),
                files,
            })
        })();

        let _ = fs::remove_dir_all(&checkpoint_dir);
        result
    }

    fn apply_checkpoint_archive(&self, archive: &CheckpointSnapshotArchive) -> KResult<()> {
        let checkpoint_dir = unique_temp_dir("checkpoint_install");
        info!(
            "RocksDbStateStore apply_checkpoint_archive start: checkpoint_dir={}, files={}",
            checkpoint_dir.display(),
            archive.files.len()
        );
        let result = (|| -> KResult<()> {
            fs::create_dir_all(&checkpoint_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to create checkpoint install dir {}",
                        checkpoint_dir.display()
                    ),
                    e,
                )
            })?;

            materialize_snapshot_files(&checkpoint_dir, &archive.files)?;

            let checkpoint_db = open_rocksdb_with_cfs(&checkpoint_dir, false)
                .map_err(|e| klog_err_with_context("Failed to open restored checkpoint db", e))?;
            migrate_legacy_default_cf_data(&checkpoint_db).map_err(|e| {
                klog_err_with_context("Failed to migrate legacy data in checkpoint db", e)
            })?;

            self.replace_with_db(&checkpoint_db)?;
            info!(
                "RocksDbStateStore apply_checkpoint_archive completed: checkpoint_dir={}",
                checkpoint_dir.display()
            );
            Ok(())
        })();

        let _ = fs::remove_dir_all(&checkpoint_dir);
        result
    }

    fn build_backup_engine_archive(&self) -> KResult<BackupEngineSnapshotArchive> {
        let backup_dir = unique_temp_dir("backup_engine_build");
        info!(
            "RocksDbStateStore build_backup_engine_archive start: backup_dir={}, mode={:?}",
            backup_dir.display(),
            self.snapshot_mode
        );
        let result = (|| -> KResult<BackupEngineSnapshotArchive> {
            fs::create_dir_all(&backup_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to create backup engine dir {}",
                        backup_dir.display()
                    ),
                    e,
                )
            })?;

            let backup_opts = BackupEngineOptions::new(&backup_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to create backup engine options for {}",
                        backup_dir.display()
                    ),
                    e,
                )
            })?;
            let env = Env::new().map_err(|e| {
                klog_err_with_context("Failed to create rocksdb env for backup engine", e)
            })?;
            let mut backup_engine = BackupEngine::open(&backup_opts, &env).map_err(|e| {
                klog_err_with_context(
                    format!("Failed to open backup engine at {}", backup_dir.display()),
                    e,
                )
            })?;
            backup_engine
                .create_new_backup_flush(self.db.as_ref(), true)
                .map_err(|e| {
                    klog_err_with_context("Failed to create rocksdb backup snapshot", e)
                })?;

            let files = collect_snapshot_files(&backup_dir)?;
            info!(
                "RocksDbStateStore build_backup_engine_archive files collected: backup_dir={}, files={}",
                backup_dir.display(),
                files.len()
            );
            Ok(BackupEngineSnapshotArchive {
                magic: BACKUP_ENGINE_SNAPSHOT_MAGIC.to_string(),
                files,
            })
        })();

        let _ = fs::remove_dir_all(&backup_dir);
        result
    }

    fn apply_backup_engine_archive(&self, archive: &BackupEngineSnapshotArchive) -> KResult<()> {
        let restore_root = unique_temp_dir("backup_engine_install");
        let backup_dir = restore_root.join("backup");
        let restored_db_dir = restore_root.join("restored_db");
        info!(
            "RocksDbStateStore apply_backup_engine_archive start: restore_root={}, files={}",
            restore_root.display(),
            archive.files.len()
        );
        let result = (|| -> KResult<()> {
            fs::create_dir_all(&backup_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to create backup restore dir {}",
                        backup_dir.display()
                    ),
                    e,
                )
            })?;
            fs::create_dir_all(&restored_db_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to create restored db dir {}",
                        restored_db_dir.display()
                    ),
                    e,
                )
            })?;

            materialize_snapshot_files(&backup_dir, &archive.files)?;

            let backup_opts = BackupEngineOptions::new(&backup_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to create backup engine options for restore {}",
                        backup_dir.display()
                    ),
                    e,
                )
            })?;
            let env = Env::new().map_err(|e| {
                klog_err_with_context("Failed to create rocksdb env for backup restore", e)
            })?;
            let mut backup_engine = BackupEngine::open(&backup_opts, &env).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to open backup engine for restore {}",
                        backup_dir.display()
                    ),
                    e,
                )
            })?;
            let mut restore_opts = RestoreOptions::default();
            restore_opts.set_keep_log_files(false);
            backup_engine
                .restore_from_latest_backup(&restored_db_dir, &restored_db_dir, &restore_opts)
                .map_err(|e| {
                    klog_err_with_context(
                        format!(
                            "Failed to restore latest backup to {}",
                            restored_db_dir.display()
                        ),
                        e,
                    )
                })?;

            let restored_db = open_rocksdb_with_cfs(&restored_db_dir, false)
                .map_err(|e| klog_err_with_context("Failed to open restored backup db", e))?;
            migrate_legacy_default_cf_data(&restored_db).map_err(|e| {
                klog_err_with_context("Failed to migrate legacy data in restored backup db", e)
            })?;
            self.replace_with_db(&restored_db)?;
            info!(
                "RocksDbStateStore apply_backup_engine_archive completed: restore_root={}",
                restore_root.display()
            );
            Ok(())
        })();

        let _ = fs::remove_dir_all(&restore_root);
        result
    }

    pub fn snapshot_mode(&self) -> RocksDbSnapshotMode {
        self.snapshot_mode
    }
}

fn strategy_for_mode(mode: RocksDbSnapshotMode) -> Arc<dyn RocksDbSnapshotStrategy> {
    match mode {
        RocksDbSnapshotMode::Enumerate => Arc::new(EnumerateSnapshotStrategy),
        RocksDbSnapshotMode::Checkpoint => Arc::new(CheckpointSnapshotStrategy),
        RocksDbSnapshotMode::BackupEngine => Arc::new(BackupEngineSnapshotStrategy),
    }
}

fn collect_snapshot_files(root: &Path) -> KResult<Vec<SnapshotFileBlob>> {
    debug!("Collect snapshot files start: root={}", root.display());
    let mut files = Vec::new();
    collect_snapshot_files_recursive(root, root, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    debug!(
        "Collect snapshot files done: root={}, files={}",
        root.display(),
        files.len()
    );
    Ok(files)
}

fn collect_snapshot_files_recursive(
    root: &Path,
    current: &Path,
    files: &mut Vec<SnapshotFileBlob>,
) -> KResult<()> {
    let entries = fs::read_dir(current).map_err(|e| {
        klog_err_with_context(
            format!("Failed to read snapshot dir {}", current.display()),
            e,
        )
    })?;

    for entry in entries {
        let entry =
            entry.map_err(|e| klog_err_with_context("Failed to read snapshot dir entry", e))?;

        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            klog_err_with_context(
                format!("Failed to read snapshot file type {}", path.display()),
                e,
            )
        })?;

        if file_type.is_dir() {
            collect_snapshot_files_recursive(root, &path, files)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let rel_path = path.strip_prefix(root).map_err(|e| {
            klog_err_with_context(
                format!(
                    "Failed to derive relative snapshot path {} from {}",
                    path.display(),
                    root.display()
                ),
                e,
            )
        })?;

        let relative_path = normalize_relative_path(rel_path)?;
        let data = fs::read(&path).map_err(|e| {
            klog_err_with_context(
                format!("Failed to read snapshot file {}", path.display()),
                e,
            )
        })?;

        files.push(SnapshotFileBlob {
            relative_path,
            data,
        });
    }

    Ok(())
}

fn normalize_relative_path(path: &Path) -> KResult<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(s) => {
                parts.push(s.to_string_lossy().to_string());
            }
            _ => {
                let msg = format!("Invalid checkpoint path component: {}", path.display());
                error!("{}", msg);
                return Err(klog_err(msg));
            }
        }
    }

    if parts.is_empty() {
        let msg = "Checkpoint file path is empty";
        error!("{}", msg);
        return Err(klog_err(msg));
    }

    Ok(parts.join("/"))
}

fn safe_join_relative(root: &Path, relative: &str) -> KResult<PathBuf> {
    let mut path = PathBuf::new();
    for component in Path::new(relative).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(s) => path.push(s),
            _ => {
                let msg = format!("Unsafe checkpoint relative path: {}", relative);
                error!("{}", msg);
                return Err(klog_err(msg));
            }
        }
    }

    if path.as_os_str().is_empty() {
        let msg = format!("Checkpoint relative path is empty: {}", relative);
        error!("{}", msg);
        return Err(klog_err(msg));
    }

    Ok(root.join(path))
}

fn materialize_snapshot_files(root: &Path, files: &[SnapshotFileBlob]) -> KResult<()> {
    debug!(
        "Materialize snapshot files start: root={}, files={}",
        root.display(),
        files.len()
    );
    for file in files {
        let dst = safe_join_relative(root, &file.relative_path)?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                klog_err_with_context(
                    format!("Failed to create snapshot restore dir {}", parent.display()),
                    e,
                )
            })?;
        }

        fs::write(&dst, &file.data).map_err(|e| {
            klog_err_with_context(
                format!("Failed to write snapshot file {}", dst.display()),
                e,
            )
        })?;
    }
    debug!(
        "Materialize snapshot files done: root={}, files={}",
        root.display(),
        files.len()
    );

    Ok(())
}

fn try_decode_checkpoint_archive(data: &[u8]) -> KResult<Option<CheckpointSnapshotArchive>> {
    if !data.starts_with(CHECKPOINT_SNAPSHOT_PREFIX) {
        debug!(
            "Checkpoint snapshot prefix not matched: payload_bytes={}",
            data.len()
        );
        return Ok(None);
    }

    let payload = &data[CHECKPOINT_SNAPSHOT_PREFIX.len()..];
    let decoded: Result<(CheckpointSnapshotArchive, usize), _> =
        bincode::serde::decode_from_slice(payload, bincode::config::legacy());

    let Ok((archive, _)) = decoded else {
        warn!(
            "Checkpoint snapshot payload decode failed: payload_bytes={}",
            payload.len()
        );
        return Ok(None);
    };

    if archive.magic != CHECKPOINT_SNAPSHOT_MAGIC {
        warn!(
            "Checkpoint snapshot magic mismatch: got={}, expected={}",
            archive.magic, CHECKPOINT_SNAPSHOT_MAGIC
        );
        return Ok(None);
    }

    debug!(
        "Checkpoint snapshot payload decoded: files={}",
        archive.files.len()
    );
    Ok(Some(archive))
}

fn try_decode_backup_engine_archive(data: &[u8]) -> KResult<Option<BackupEngineSnapshotArchive>> {
    if !data.starts_with(BACKUP_ENGINE_SNAPSHOT_PREFIX) {
        debug!(
            "BackupEngine snapshot prefix not matched: payload_bytes={}",
            data.len()
        );
        return Ok(None);
    }

    let payload = &data[BACKUP_ENGINE_SNAPSHOT_PREFIX.len()..];
    let decoded: Result<(BackupEngineSnapshotArchive, usize), _> =
        bincode::serde::decode_from_slice(payload, bincode::config::legacy());

    let Ok((archive, _)) = decoded else {
        warn!(
            "BackupEngine snapshot payload decode failed: payload_bytes={}",
            payload.len()
        );
        return Ok(None);
    };

    if archive.magic != BACKUP_ENGINE_SNAPSHOT_MAGIC {
        warn!(
            "BackupEngine snapshot magic mismatch: got={}, expected={}",
            archive.magic, BACKUP_ENGINE_SNAPSHOT_MAGIC
        );
        return Ok(None);
    }

    debug!(
        "BackupEngine snapshot payload decoded: files={}",
        archive.files.len()
    );
    Ok(Some(archive))
}

fn decode_snapshot_data(data: &[u8]) -> KResult<KLogStateSnapshotData> {
    let decoded_new: Result<(KLogStateSnapshotData, usize), _> =
        bincode::serde::decode_from_slice(data, bincode::config::legacy());
    if let Ok((snapshot_data, _)) = decoded_new {
        return Ok(snapshot_data);
    }

    // Temporary fallback for snapshots built before meta support.
    let (entries, _): (Vec<KLogEntry>, usize) =
        bincode::serde::decode_from_slice(data, bincode::config::legacy()).map_err(|e| {
            klog_err_with_context("Failed to decode rocksdb enumerate snapshot payload", e)
        })?;
    Ok(KLogStateSnapshotData {
        entries,
        meta_entries: Vec::new(),
    })
}

impl RocksDbSnapshotStrategy for EnumerateSnapshotStrategy {
    fn mode(&self) -> RocksDbSnapshotMode {
        RocksDbSnapshotMode::Enumerate
    }

    fn build_snapshot(&self, store: &RocksDbStateStore) -> KResult<KLogStateSnapshot> {
        info!(
            "RocksDb enumerate build_snapshot start: mode={:?}",
            store.snapshot_mode
        );
        let entries = store.read_all_entries()?;
        let meta_entries = store.read_all_meta_entries()?;
        let snapshot_data = KLogStateSnapshotData {
            entries,
            meta_entries,
        };
        let data = bincode::serde::encode_to_vec(&snapshot_data, bincode::config::legacy())
            .map_err(|e| {
                klog_err_with_context("Failed to serialize rocksdb enumerate snapshot", e)
            })?;
        info!(
            "RocksDb enumerate build_snapshot done: entries={}, meta_entries={}, payload_bytes={}",
            snapshot_data.entries.len(),
            snapshot_data.meta_entries.len(),
            data.len()
        );
        Ok(KLogStateSnapshot { data })
    }

    fn try_install_snapshot(
        &self,
        store: &RocksDbStateStore,
        snapshot: &KLogStateSnapshot,
    ) -> KResult<bool> {
        if snapshot.data.starts_with(CHECKPOINT_SNAPSHOT_PREFIX)
            || snapshot.data.starts_with(BACKUP_ENGINE_SNAPSHOT_PREFIX)
        {
            debug!(
                "RocksDb enumerate install_snapshot skipped by prefix: payload_bytes={}",
                snapshot.data.len()
            );
            return Ok(false);
        }

        let snapshot_data = match decode_snapshot_data(&snapshot.data) {
            Ok(v) => v,
            Err(_) => {
                warn!(
                    "RocksDb enumerate install_snapshot decode failed: payload_bytes={}",
                    snapshot.data.len()
                );
                return Ok(false);
            }
        };

        info!(
            "RocksDb enumerate install_snapshot apply: entries={}, meta_entries={}",
            summarize_entry_ids(&snapshot_data.entries),
            snapshot_data.meta_entries.len()
        );
        store.replace_with_entries(snapshot_data.entries, snapshot_data.meta_entries)?;
        Ok(true)
    }
}

impl RocksDbSnapshotStrategy for CheckpointSnapshotStrategy {
    fn mode(&self) -> RocksDbSnapshotMode {
        RocksDbSnapshotMode::Checkpoint
    }

    fn build_snapshot(&self, store: &RocksDbStateStore) -> KResult<KLogStateSnapshot> {
        info!(
            "RocksDb checkpoint build_snapshot start: mode={:?}",
            store.snapshot_mode
        );
        let archive = store.build_checkpoint_archive()?;
        let mut payload = bincode::serde::encode_to_vec(&archive, bincode::config::legacy())
            .map_err(|e| {
                klog_err_with_context("Failed to serialize checkpoint snapshot archive", e)
            })?;

        let mut data = Vec::with_capacity(CHECKPOINT_SNAPSHOT_PREFIX.len() + payload.len());
        data.extend_from_slice(CHECKPOINT_SNAPSHOT_PREFIX);
        data.append(&mut payload);
        info!(
            "RocksDb checkpoint build_snapshot done: files={}, payload_bytes={}",
            archive.files.len(),
            data.len()
        );

        Ok(KLogStateSnapshot { data })
    }

    fn try_install_snapshot(
        &self,
        store: &RocksDbStateStore,
        snapshot: &KLogStateSnapshot,
    ) -> KResult<bool> {
        let Some(archive) = try_decode_checkpoint_archive(&snapshot.data)? else {
            return Ok(false);
        };

        info!(
            "RocksDb checkpoint install_snapshot apply: files={}, payload_bytes={}",
            archive.files.len(),
            snapshot.data.len()
        );
        store.apply_checkpoint_archive(&archive)?;
        Ok(true)
    }
}

impl RocksDbSnapshotStrategy for BackupEngineSnapshotStrategy {
    fn mode(&self) -> RocksDbSnapshotMode {
        RocksDbSnapshotMode::BackupEngine
    }

    fn build_snapshot(&self, store: &RocksDbStateStore) -> KResult<KLogStateSnapshot> {
        info!(
            "RocksDb backup-engine build_snapshot start: mode={:?}",
            store.snapshot_mode
        );
        let archive = store.build_backup_engine_archive()?;
        let mut payload = bincode::serde::encode_to_vec(&archive, bincode::config::legacy())
            .map_err(|e| {
                klog_err_with_context("Failed to serialize backup engine snapshot archive", e)
            })?;

        let mut data = Vec::with_capacity(BACKUP_ENGINE_SNAPSHOT_PREFIX.len() + payload.len());
        data.extend_from_slice(BACKUP_ENGINE_SNAPSHOT_PREFIX);
        data.append(&mut payload);
        info!(
            "RocksDb backup-engine build_snapshot done: files={}, payload_bytes={}",
            archive.files.len(),
            data.len()
        );

        Ok(KLogStateSnapshot { data })
    }

    fn try_install_snapshot(
        &self,
        store: &RocksDbStateStore,
        snapshot: &KLogStateSnapshot,
    ) -> KResult<bool> {
        let Some(archive) = try_decode_backup_engine_archive(&snapshot.data)? else {
            return Ok(false);
        };

        info!(
            "RocksDb backup-engine install_snapshot apply: files={}, payload_bytes={}",
            archive.files.len(),
            snapshot.data.len()
        );
        store.apply_backup_engine_archive(&archive)?;
        Ok(true)
    }
}

#[async_trait::async_trait]
impl KLogStateStore for RocksDbStateStore {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        debug!(
            "RocksDbStateStore append start: mode={:?}, entries={}",
            self.snapshot_mode,
            summarize_entry_ids(&entries)
        );
        let logs_cf = self.db.cf_handle(CF_LOGS).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_LOGS);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let idx_level_cf = self.db.cf_handle(CF_INDEX_LEVEL).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_INDEX_LEVEL);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let idx_source_cf = self.db.cf_handle(CF_INDEX_SOURCE).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_INDEX_SOURCE);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let now_ms = now_millis();
        let mut batch = WriteBatch::default();
        for entry in entries {
            let key = entry_key(entry.id);
            let value =
                bincode::serde::encode_to_vec(&entry, bincode::config::legacy()).map_err(|e| {
                    klog_err_with_context("Failed to serialize state entry for rocksdb", e)
                })?;
            batch.put_cf(&logs_cf, key, value);
            batch.put_cf(&idx_level_cf, level_index_key(entry.level, entry.id), []);
            if let Some(source) = normalize_source(entry.source.as_deref()) {
                batch.put_cf(&idx_source_cf, source_index_key(source, entry.id), []);
            }
            if let Some(request_id) = normalize_request_id(entry.request_id.as_deref()) {
                let dedup_key = request_dedup_meta_key(request_id);
                let dedup_value = bincode::serde::encode_to_vec(
                    &RequestDedupMeta {
                        log_id: entry.id,
                        seen_at_ms: now_ms,
                    },
                    bincode::config::legacy(),
                )
                .map_err(|e| klog_err_with_context("Failed to encode request dedup index", e))?;
                batch.put_cf(&meta_cf, dedup_key, dedup_value);
            }
        }

        let write_opts = self.write_options();
        self.db
            .write_opt(batch, &write_opts)
            .map_err(|e| klog_err_with_context("Failed to write state entries to rocksdb", e))?;
        debug!(
            "RocksDbStateStore append done: mode={:?}",
            self.snapshot_mode
        );
        Ok(())
    }

    async fn query(&self, query: KLogQuery) -> KResult<Vec<KLogEntry>> {
        let logs_cf = self.db.cf_handle(CF_LOGS).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_LOGS);
            error!("{}", msg);
            klog_err(msg)
        })?;

        if query.limit == 0 {
            return Ok(Vec::new());
        }

        let mut out = Vec::with_capacity(query.limit.min(1024));
        let source_filter = query
            .source
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());

        if let Some(source) = source_filter.as_deref() {
            let idx_source_cf = self.db.cf_handle(CF_INDEX_SOURCE).ok_or_else(|| {
                let msg = format!("Missing column family '{}'", CF_INDEX_SOURCE);
                error!("{}", msg);
                klog_err(msg)
            })?;
            let source_prefix = source_index_prefix(source);
            match query.order {
                KLogQueryOrder::Asc => {
                    let start_id = query.start_id.unwrap_or(0);
                    let start_key = source_index_key(source, start_id);
                    let iter = self.db.iterator_cf(
                        &idx_source_cf,
                        IteratorMode::From(&start_key, Direction::Forward),
                    );
                    for item in iter {
                        let (k, _) = item.map_err(|e| {
                            klog_err_with_context("Failed to iterate rocksdb source index", e)
                        })?;
                        if !k.as_ref().starts_with(&source_prefix) {
                            break;
                        }
                        let Some(id) = decode_source_index_id(k.as_ref(), source) else {
                            continue;
                        };
                        if query.start_id.map(|start| id < start).unwrap_or(false) {
                            continue;
                        }
                        if query.end_id.map(|end| id > end).unwrap_or(false) {
                            break;
                        }
                        let Some(raw) =
                            self.db
                                .get_cf(&logs_cf, entry_key(id).as_ref())
                                .map_err(|e| {
                                    klog_err_with_context(
                                        "Failed to read state entry by source index from rocksdb",
                                        e,
                                    )
                                })?
                        else {
                            continue;
                        };
                        let (entry, _): (KLogEntry, usize) = bincode::serde::decode_from_slice(
                            raw.as_ref(),
                            bincode::config::legacy(),
                        )
                        .map_err(|e| {
                            klog_err_with_context(
                                "Failed to deserialize state entry from rocksdb",
                                e,
                            )
                        })?;
                        if !entry_matches_query(&entry, &query) {
                            continue;
                        }
                        out.push(entry);
                        if out.len() >= query.limit {
                            break;
                        }
                    }
                }
                KLogQueryOrder::Desc => {
                    let end_id = query.end_id.unwrap_or(u64::MAX);
                    let start_key = source_index_key(source, end_id);
                    let iter = self.db.iterator_cf(
                        &idx_source_cf,
                        IteratorMode::From(&start_key, Direction::Reverse),
                    );
                    for item in iter {
                        let (k, _) = item.map_err(|e| {
                            klog_err_with_context("Failed to iterate rocksdb source index", e)
                        })?;
                        if !k.as_ref().starts_with(&source_prefix) {
                            break;
                        }
                        let Some(id) = decode_source_index_id(k.as_ref(), source) else {
                            continue;
                        };
                        if query.end_id.map(|end| id > end).unwrap_or(false) {
                            continue;
                        }
                        if query.start_id.map(|start| id < start).unwrap_or(false) {
                            break;
                        }
                        let Some(raw) =
                            self.db
                                .get_cf(&logs_cf, entry_key(id).as_ref())
                                .map_err(|e| {
                                    klog_err_with_context(
                                        "Failed to read state entry by source index from rocksdb",
                                        e,
                                    )
                                })?
                        else {
                            continue;
                        };
                        let (entry, _): (KLogEntry, usize) = bincode::serde::decode_from_slice(
                            raw.as_ref(),
                            bincode::config::legacy(),
                        )
                        .map_err(|e| {
                            klog_err_with_context(
                                "Failed to deserialize state entry from rocksdb",
                                e,
                            )
                        })?;
                        if !entry_matches_query(&entry, &query) {
                            continue;
                        }
                        out.push(entry);
                        if out.len() >= query.limit {
                            break;
                        }
                    }
                }
            }
            return Ok(out);
        }

        if let Some(level) = query.level {
            let idx_level_cf = self.db.cf_handle(CF_INDEX_LEVEL).ok_or_else(|| {
                let msg = format!("Missing column family '{}'", CF_INDEX_LEVEL);
                error!("{}", msg);
                klog_err(msg)
            })?;
            match query.order {
                KLogQueryOrder::Asc => {
                    let start_id = query.start_id.unwrap_or(0);
                    let start_key = level_index_key(level, start_id);
                    let iter = self.db.iterator_cf(
                        &idx_level_cf,
                        IteratorMode::From(&start_key, Direction::Forward),
                    );
                    for item in iter {
                        let (k, _) = item.map_err(|e| {
                            klog_err_with_context("Failed to iterate rocksdb level index", e)
                        })?;
                        let Some(id) = decode_level_index_id(k.as_ref(), level) else {
                            break;
                        };
                        if query.start_id.map(|start| id < start).unwrap_or(false) {
                            continue;
                        }
                        if query.end_id.map(|end| id > end).unwrap_or(false) {
                            break;
                        }
                        let Some(raw) =
                            self.db
                                .get_cf(&logs_cf, entry_key(id).as_ref())
                                .map_err(|e| {
                                    klog_err_with_context(
                                        "Failed to read state entry by level index from rocksdb",
                                        e,
                                    )
                                })?
                        else {
                            continue;
                        };
                        let (entry, _): (KLogEntry, usize) = bincode::serde::decode_from_slice(
                            raw.as_ref(),
                            bincode::config::legacy(),
                        )
                        .map_err(|e| {
                            klog_err_with_context(
                                "Failed to deserialize state entry from rocksdb",
                                e,
                            )
                        })?;
                        if !entry_matches_query(&entry, &query) {
                            continue;
                        }
                        out.push(entry);
                        if out.len() >= query.limit {
                            break;
                        }
                    }
                }
                KLogQueryOrder::Desc => {
                    let end_id = query.end_id.unwrap_or(u64::MAX);
                    let start_key = level_index_key(level, end_id);
                    let iter = self.db.iterator_cf(
                        &idx_level_cf,
                        IteratorMode::From(&start_key, Direction::Reverse),
                    );
                    for item in iter {
                        let (k, _) = item.map_err(|e| {
                            klog_err_with_context("Failed to iterate rocksdb level index", e)
                        })?;
                        let Some(id) = decode_level_index_id(k.as_ref(), level) else {
                            break;
                        };
                        if query.end_id.map(|end| id > end).unwrap_or(false) {
                            continue;
                        }
                        if query.start_id.map(|start| id < start).unwrap_or(false) {
                            break;
                        }
                        let Some(raw) =
                            self.db
                                .get_cf(&logs_cf, entry_key(id).as_ref())
                                .map_err(|e| {
                                    klog_err_with_context(
                                        "Failed to read state entry by level index from rocksdb",
                                        e,
                                    )
                                })?
                        else {
                            continue;
                        };
                        let (entry, _): (KLogEntry, usize) = bincode::serde::decode_from_slice(
                            raw.as_ref(),
                            bincode::config::legacy(),
                        )
                        .map_err(|e| {
                            klog_err_with_context(
                                "Failed to deserialize state entry from rocksdb",
                                e,
                            )
                        })?;
                        if !entry_matches_query(&entry, &query) {
                            continue;
                        }
                        out.push(entry);
                        if out.len() >= query.limit {
                            break;
                        }
                    }
                }
            }
            return Ok(out);
        }

        match query.order {
            KLogQueryOrder::Asc => {
                let iter = if let Some(start_id) = query.start_id {
                    let start_key = entry_key(start_id);
                    self.db
                        .iterator_cf(&logs_cf, IteratorMode::From(&start_key, Direction::Forward))
                } else {
                    self.db.iterator_cf(&logs_cf, IteratorMode::Start)
                };

                for item in iter {
                    let (k, v) = item
                        .map_err(|e| klog_err_with_context("Failed to iterate rocksdb entry", e))?;
                    let Some(id) = decode_entry_key(&k) else {
                        continue;
                    };
                    if query.start_id.map(|start| id < start).unwrap_or(false) {
                        continue;
                    }
                    if query.end_id.map(|end| id > end).unwrap_or(false) {
                        break;
                    }

                    let (entry, _): (KLogEntry, usize) =
                        bincode::serde::decode_from_slice(v.as_ref(), bincode::config::legacy())
                            .map_err(|e| {
                                klog_err_with_context(
                                    "Failed to deserialize state entry from rocksdb",
                                    e,
                                )
                            })?;
                    if !entry_matches_query(&entry, &query) {
                        continue;
                    }
                    out.push(entry);
                    if out.len() >= query.limit {
                        break;
                    }
                }
            }
            KLogQueryOrder::Desc => {
                let iter = if let Some(end_id) = query.end_id {
                    let end_key = entry_key(end_id);
                    self.db
                        .iterator_cf(&logs_cf, IteratorMode::From(&end_key, Direction::Reverse))
                } else {
                    self.db.iterator_cf(&logs_cf, IteratorMode::End)
                };

                for item in iter {
                    let (k, v) = item
                        .map_err(|e| klog_err_with_context("Failed to iterate rocksdb entry", e))?;
                    let Some(id) = decode_entry_key(&k) else {
                        continue;
                    };
                    if query.end_id.map(|end| id > end).unwrap_or(false) {
                        continue;
                    }
                    if query.start_id.map(|start| id < start).unwrap_or(false) {
                        break;
                    }

                    let (entry, _): (KLogEntry, usize) =
                        bincode::serde::decode_from_slice(v.as_ref(), bincode::config::legacy())
                            .map_err(|e| {
                                klog_err_with_context(
                                    "Failed to deserialize state entry from rocksdb",
                                    e,
                                )
                            })?;
                    if !entry_matches_query(&entry, &query) {
                        continue;
                    }
                    out.push(entry);
                    if out.len() >= query.limit {
                        break;
                    }
                }
            }
        }

        Ok(out)
    }

    async fn put_meta(&self, item: KLogMetaEntry) -> KResult<KLogMetaEntry> {
        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let meta_key = data_meta_key(item.key.as_str());
        let prev = self
            .db
            .get_cf(&meta_cf, meta_key.as_slice())
            .map_err(|e| klog_err_with_context("Failed to read rocksdb data meta entry", e))?;
        let next_revision = match prev {
            Some(raw) => decode_meta_entry_with_legacy(raw.as_ref())?
                .revision
                .saturating_add(1),
            None => 1,
        };

        let mut stored = item;
        stored.revision = next_revision;

        let encoded = bincode::serde::encode_to_vec(&stored, bincode::config::legacy())
            .map_err(|e| klog_err_with_context("Failed to encode rocksdb data meta entry", e))?;
        let write_opts = self.write_options();
        self.db
            .put_cf_opt(&meta_cf, meta_key, encoded, &write_opts)
            .map_err(|e| klog_err_with_context("Failed to persist rocksdb data meta entry", e))?;
        Ok(stored)
    }

    async fn delete_meta(&self, key: &str) -> KResult<Option<KLogMetaEntry>> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(None);
        }

        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let meta_key = data_meta_key(key);
        let Some(raw) = self
            .db
            .get_cf(&meta_cf, meta_key.as_slice())
            .map_err(|e| klog_err_with_context("Failed to read rocksdb data meta entry", e))?
        else {
            return Ok(None);
        };
        let prev_meta = decode_meta_entry_with_legacy(raw.as_ref())?;
        let write_opts = self.write_options();
        self.db
            .delete_cf_opt(&meta_cf, meta_key.as_slice(), &write_opts)
            .map_err(|e| klog_err_with_context("Failed to delete rocksdb data meta entry", e))?;
        Ok(Some(prev_meta))
    }

    async fn get_meta(&self, key: &str) -> KResult<Option<KLogMetaEntry>> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(None);
        }

        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let meta_key = data_meta_key(key);
        let Some(raw) = self
            .db
            .get_cf(&meta_cf, meta_key.as_slice())
            .map_err(|e| klog_err_with_context("Failed to read rocksdb data meta entry", e))?
        else {
            return Ok(None);
        };
        let item = decode_meta_entry_with_legacy(raw.as_ref())?;
        Ok(Some(item))
    }

    async fn list_meta(&self, prefix: Option<&str>, limit: usize) -> KResult<Vec<KLogMetaEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let normalized_prefix = prefix.map(str::trim).filter(|v| !v.is_empty());
        let seek_key = normalized_prefix
            .map(data_meta_key)
            .unwrap_or_else(|| KEY_DATA_META_PREFIX.to_vec());

        let mut out = Vec::with_capacity(limit.min(1024));
        let iter = self.db.iterator_cf(
            &meta_cf,
            IteratorMode::From(seek_key.as_slice(), Direction::Forward),
        );
        for item in iter {
            let (k, v) =
                item.map_err(|e| klog_err_with_context("Failed to iterate rocksdb data meta", e))?;
            if !k.as_ref().starts_with(KEY_DATA_META_PREFIX) {
                break;
            }
            let Some(key) = decode_data_meta_key(k.as_ref()) else {
                continue;
            };
            if let Some(prefix) = normalized_prefix
                && !key.starts_with(prefix)
            {
                continue;
            }
            let item = decode_meta_entry_with_legacy(v.as_ref())?;
            out.push(item);
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    async fn build_snapshot(&self) -> KResult<KLogStateSnapshot> {
        info!(
            "RocksDbStateStore build_snapshot dispatch: mode={:?}",
            self.snapshot_mode
        );
        self.snapshot_builder.build_snapshot(self)
    }

    async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()> {
        info!(
            "RocksDbStateStore install_snapshot start: mode={:?}, payload_bytes={}, installers={}",
            self.snapshot_mode,
            snapshot.data.len(),
            self.snapshot_installers.len()
        );
        for installer in &self.snapshot_installers {
            if installer.try_install_snapshot(self, &snapshot)? {
                info!(
                    "RocksDbStateStore install_snapshot handled by mode={:?}",
                    installer.mode()
                );
                return Ok(());
            }
        }

        let msg = format!(
            "Unsupported rocksdb snapshot payload for mode {:?}, payload_bytes={}",
            self.snapshot_mode,
            snapshot.data.len()
        );
        error!("{}", msg);
        Err(klog_err(msg))
    }

    async fn load_next_log_id(&self) -> KResult<Option<u64>> {
        let next_log_id = self.resolve_next_log_id()?;
        Ok(Some(next_log_id))
    }

    async fn save_next_log_id(&self, next_log_id: u64) -> KResult<()> {
        let current = self.read_persisted_next_log_id()?.unwrap_or(1);
        let target = next_log_id.max(current).max(1);
        if target != current {
            self.persist_next_log_id(target)?;
            debug!(
                "RocksDbStateStore save_next_log_id updated: {} -> {}",
                current, target
            );
        }
        Ok(())
    }

    async fn load_state_machine_meta(&self) -> KResult<Option<KLogStateMachineMeta>> {
        self.read_persisted_state_machine_meta()
    }

    async fn save_state_machine_meta(&self, meta: KLogStateMachineMeta) -> KResult<()> {
        self.persist_state_machine_meta(&meta)
    }

    async fn lookup_recent_request_id(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> KResult<Option<u64>> {
        let Some(request_id) = normalize_request_id(Some(request_id)) else {
            return Ok(None);
        };

        let meta_cf = self.db.cf_handle(CF_META).ok_or_else(|| {
            let msg = format!("Missing column family '{}'", CF_META);
            error!("{}", msg);
            klog_err(msg)
        })?;
        let key = request_dedup_meta_key(request_id);
        let Some(raw) = self
            .db
            .get_cf(&meta_cf, key.as_slice())
            .map_err(|e| klog_err_with_context("Failed to read request dedup index", e))?
        else {
            return Ok(None);
        };
        let (record, _): (RequestDedupMeta, usize) =
            bincode::serde::decode_from_slice(raw.as_ref(), bincode::config::legacy())
                .map_err(|e| klog_err_with_context("Failed to decode request dedup index", e))?;

        if now_ms.saturating_sub(record.seen_at_ms) > REQUEST_DEDUP_WINDOW_MS {
            let write_opts = self.write_options();
            self.db
                .delete_cf_opt(&meta_cf, key.as_slice(), &write_opts)
                .map_err(|e| {
                    klog_err_with_context("Failed to delete expired request dedup index", e)
                })?;
            return Ok(None);
        }

        Ok(Some(record.log_id))
    }
}
