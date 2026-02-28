use super::store::{KLogStateSnapshot, KLogStateStore};
use crate::{KLogEntry, KLogError, KResult};
use rocksdb::backup::{BackupEngine, BackupEngineOptions, RestoreOptions};
use rocksdb::checkpoint::Checkpoint;
use rocksdb::{DB, Env, IteratorMode, Options, WriteBatch};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const KEY_PREFIX_ENTRY: u8 = b'e';
const KEY_NEXT_LOG_ID_META: &[u8] = b"m:next_log_id";
const CHECKPOINT_SNAPSHOT_MAGIC: &str = "klog-rdb-checkpoint-v1";
const CHECKPOINT_SNAPSHOT_PREFIX: &[u8] = b"KLOG_RDB_CP1";
const BACKUP_ENGINE_SNAPSHOT_MAGIC: &str = "klog-rdb-backup-v1";
const BACKUP_ENGINE_SNAPSHOT_PREFIX: &[u8] = b"KLOG_RDB_BK1";
static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

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

fn create_rocksdb(path: &Path) -> Result<DB, String> {
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

    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_atomic_flush(true);

    DB::open(&opts, path).map_err(|e| {
        let msg = format!("Failed to open rocksdb at {}: {}", path.display(), e);
        error!("{}", msg);
        msg
    })
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
    snapshot_builder: Arc<dyn RocksDbSnapshotStrategy>,
    snapshot_installers: Vec<Arc<dyn RocksDbSnapshotStrategy>>,
}

impl std::fmt::Debug for RocksDbStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocksDbStateStore")
            .field("snapshot_mode", &self.snapshot_mode)
            .finish()
    }
}

impl RocksDbStateStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        Self::open_with_mode(path, RocksDbSnapshotMode::Checkpoint)
    }

    pub fn open_with_mode<P: AsRef<Path>>(
        path: P,
        snapshot_mode: RocksDbSnapshotMode,
    ) -> Result<Self, String> {
        info!(
            "RocksDbStateStore open_with_mode: path={}, snapshot_mode={:?}",
            path.as_ref().display(),
            snapshot_mode
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
            snapshot_builder,
            snapshot_installers,
        })
    }

    fn read_persisted_next_log_id(&self) -> KResult<Option<u64>> {
        let value = self
            .db
            .get(KEY_NEXT_LOG_ID_META)
            .map_err(|e| klog_err_with_context("Failed to read rocksdb next_log_id metadata", e))?;
        let Some(raw) = value else {
            return Ok(None);
        };
        let next_log_id = decode_u64_be(raw.as_ref())?;
        Ok(Some(next_log_id))
    }

    fn scan_next_log_id_from_entries(&self) -> KResult<u64> {
        let mut max_id = 0u64;
        for item in self.db.iterator(IteratorMode::Start) {
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
        self.db
            .put(KEY_NEXT_LOG_ID_META, next_log_id.to_be_bytes())
            .map_err(|e| klog_err_with_context("Failed to persist rocksdb next_log_id metadata", e))
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
        let mut entries = Vec::new();
        for item in self.db.iterator(IteratorMode::Start) {
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

    fn clear_entries_in_batch(&self, batch: &mut WriteBatch) -> KResult<()> {
        debug!(
            "RocksDbStateStore clear_entries_in_batch start: snapshot_mode={:?}",
            self.snapshot_mode
        );
        let mut deleted = 0usize;
        for item in self.db.iterator(IteratorMode::Start) {
            let (k, _) = item.map_err(|e| {
                klog_err_with_context("Failed to iterate rocksdb while clearing entries", e)
            })?;
            if decode_entry_key(&k).is_some() {
                batch.delete(k);
                deleted += 1;
            }
        }
        debug!(
            "RocksDbStateStore clear_entries_in_batch done: deleted_entries={}",
            deleted
        );
        Ok(())
    }

    fn replace_with_entries(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        info!(
            "RocksDbStateStore replace_with_entries start: snapshot_mode={:?}, incoming_entries={}",
            self.snapshot_mode,
            summarize_entry_ids(&entries)
        );
        let mut batch = WriteBatch::default();
        self.clear_entries_in_batch(&mut batch)?;

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
            batch.put(key, value);
        }
        let next_log_id = max_id.saturating_add(1).max(1);
        batch.put(KEY_NEXT_LOG_ID_META, next_log_id.to_be_bytes());

        self.db.write(batch).map_err(|e| {
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

        let mut copied = 0usize;
        let mut max_id = 0u64;
        for item in source_db.iterator(IteratorMode::Start) {
            let (k, v) = item
                .map_err(|e| klog_err_with_context("Failed to iterate source checkpoint db", e))?;
            let Some(id) = decode_entry_key(&k) else {
                continue;
            };
            batch.put(k, v);
            copied += 1;
            if id > max_id {
                max_id = id;
            }
        }
        let next_log_id = max_id.saturating_add(1).max(1);
        batch.put(KEY_NEXT_LOG_ID_META, next_log_id.to_be_bytes());

        self.db.write(batch).map_err(|e| {
            klog_err_with_context("Failed to apply checkpoint snapshot into rocksdb", e)
        })?;
        info!(
            "RocksDbStateStore replace_with_db completed: copied_entries={}, next_log_id={}",
            copied, next_log_id
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

            let mut opts = Options::default();
            opts.create_if_missing(false);
            let checkpoint_db = DB::open(&opts, &checkpoint_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to open restored checkpoint db {}",
                        checkpoint_dir.display()
                    ),
                    e,
                )
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

            let mut opts = Options::default();
            opts.create_if_missing(false);
            let restored_db = DB::open(&opts, &restored_db_dir).map_err(|e| {
                klog_err_with_context(
                    format!(
                        "Failed to open restored backup db {}",
                        restored_db_dir.display()
                    ),
                    e,
                )
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
        let data =
            bincode::serde::encode_to_vec(&entries, bincode::config::legacy()).map_err(|e| {
                klog_err_with_context("Failed to serialize rocksdb enumerate snapshot", e)
            })?;
        info!(
            "RocksDb enumerate build_snapshot done: entries={}, payload_bytes={}",
            entries.len(),
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

        let decoded: Result<(Vec<KLogEntry>, usize), _> =
            bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy());

        let Ok((entries, _)) = decoded else {
            warn!(
                "RocksDb enumerate install_snapshot decode failed: payload_bytes={}",
                snapshot.data.len()
            );
            return Ok(false);
        };

        info!(
            "RocksDb enumerate install_snapshot apply: entries={}",
            summarize_entry_ids(&entries)
        );
        store.replace_with_entries(entries)?;
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
        let mut batch = WriteBatch::default();
        for entry in entries {
            let key = entry_key(entry.id);
            let value =
                bincode::serde::encode_to_vec(&entry, bincode::config::legacy()).map_err(|e| {
                    klog_err_with_context("Failed to serialize state entry for rocksdb", e)
                })?;
            batch.put(key, value);
        }

        self.db
            .write(batch)
            .map_err(|e| klog_err_with_context("Failed to write state entries to rocksdb", e))?;
        debug!(
            "RocksDbStateStore append done: mode={:?}",
            self.snapshot_mode
        );
        Ok(())
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
}
