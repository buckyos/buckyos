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
            format!(
                "Failed to create rocksdb parent dir {}: {}",
                parent.display(),
                e
            )
        })?;
    }

    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_atomic_flush(true);

    DB::open(&opts, path)
        .map_err(|e| format!("Failed to open rocksdb at {}: {}", path.display(), e))
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

    fn read_all_entries(&self) -> KResult<Vec<KLogEntry>> {
        let mut entries = Vec::new();
        for item in self.db.iterator(IteratorMode::Start) {
            let (k, v) = item.map_err(|e| {
                let msg = format!("Failed to iterate rocksdb entry: {}", e);
                klog_err(msg)
            })?;

            if decode_entry_key(&k).is_none() {
                continue;
            }

            let (entry, _): (KLogEntry, usize) =
                bincode::serde::decode_from_slice(v.as_ref(), bincode::config::legacy()).map_err(
                    |e| {
                        let msg = format!("Failed to deserialize state entry from rocksdb: {}", e);
                        klog_err(msg)
                    },
                )?;
            entries.push(entry);
        }

        Ok(entries)
    }

    fn clear_entries_in_batch(&self, batch: &mut WriteBatch) -> KResult<()> {
        for item in self.db.iterator(IteratorMode::Start) {
            let (k, _) = item.map_err(|e| {
                let msg = format!("Failed to iterate rocksdb while clearing entries: {}", e);
                klog_err(msg)
            })?;
            if decode_entry_key(&k).is_some() {
                batch.delete(k);
            }
        }
        Ok(())
    }

    fn replace_with_entries(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let mut batch = WriteBatch::default();
        self.clear_entries_in_batch(&mut batch)?;

        for entry in entries {
            let key = entry_key(entry.id);
            let value =
                bincode::serde::encode_to_vec(&entry, bincode::config::legacy()).map_err(|e| {
                    let msg = format!("Failed to serialize state entry for rocksdb install: {}", e);
                    klog_err(msg)
                })?;
            batch.put(key, value);
        }

        self.db.write(batch).map_err(|e| {
            let msg = format!(
                "Failed to write entries during rocksdb snapshot install: {}",
                e
            );
            klog_err(msg)
        })?;

        Ok(())
    }

    fn replace_with_db(&self, source_db: &DB) -> KResult<()> {
        let mut batch = WriteBatch::default();
        self.clear_entries_in_batch(&mut batch)?;

        for item in source_db.iterator(IteratorMode::Start) {
            let (k, v) = item.map_err(|e| {
                let msg = format!("Failed to iterate source checkpoint db: {}", e);
                klog_err(msg)
            })?;
            if decode_entry_key(&k).is_none() {
                continue;
            }
            batch.put(k, v);
        }

        self.db.write(batch).map_err(|e| {
            let msg = format!("Failed to apply checkpoint snapshot into rocksdb: {}", e);
            klog_err(msg)
        })?;

        Ok(())
    }

    fn build_checkpoint_archive(&self) -> KResult<CheckpointSnapshotArchive> {
        let checkpoint_dir = unique_temp_dir("checkpoint_build");
        let result = (|| -> KResult<CheckpointSnapshotArchive> {
            let checkpoint = Checkpoint::new(self.db.as_ref()).map_err(|e| {
                let msg = format!("Failed to create rocksdb checkpoint object: {}", e);
                klog_err(msg)
            })?;

            checkpoint.create_checkpoint(&checkpoint_dir).map_err(|e| {
                let msg = format!(
                    "Failed to create rocksdb checkpoint in {}: {}",
                    checkpoint_dir.display(),
                    e
                );
                klog_err(msg)
            })?;

            let files = collect_snapshot_files(&checkpoint_dir)?;
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
        let result = (|| -> KResult<()> {
            fs::create_dir_all(&checkpoint_dir).map_err(|e| {
                let msg = format!(
                    "Failed to create checkpoint install dir {}: {}",
                    checkpoint_dir.display(),
                    e
                );
                klog_err(msg)
            })?;

            materialize_snapshot_files(&checkpoint_dir, &archive.files)?;

            let mut opts = Options::default();
            opts.create_if_missing(false);
            let checkpoint_db = DB::open(&opts, &checkpoint_dir).map_err(|e| {
                let msg = format!(
                    "Failed to open restored checkpoint db {}: {}",
                    checkpoint_dir.display(),
                    e
                );
                klog_err(msg)
            })?;

            self.replace_with_db(&checkpoint_db)?;
            Ok(())
        })();

        let _ = fs::remove_dir_all(&checkpoint_dir);
        result
    }

    fn build_backup_engine_archive(&self) -> KResult<BackupEngineSnapshotArchive> {
        let backup_dir = unique_temp_dir("backup_engine_build");
        let result = (|| -> KResult<BackupEngineSnapshotArchive> {
            fs::create_dir_all(&backup_dir).map_err(|e| {
                let msg = format!(
                    "Failed to create backup engine dir {}: {}",
                    backup_dir.display(),
                    e
                );
                klog_err(msg)
            })?;

            let backup_opts = BackupEngineOptions::new(&backup_dir).map_err(|e| {
                let msg = format!(
                    "Failed to create backup engine options for {}: {}",
                    backup_dir.display(),
                    e
                );
                klog_err(msg)
            })?;
            let env = Env::new().map_err(|e| {
                let msg = format!("Failed to create rocksdb env for backup engine: {}", e);
                klog_err(msg)
            })?;
            let mut backup_engine = BackupEngine::open(&backup_opts, &env).map_err(|e| {
                let msg = format!(
                    "Failed to open backup engine at {}: {}",
                    backup_dir.display(),
                    e
                );
                klog_err(msg)
            })?;
            backup_engine
                .create_new_backup_flush(self.db.as_ref(), true)
                .map_err(|e| {
                    let msg = format!("Failed to create rocksdb backup snapshot: {}", e);
                    klog_err(msg)
                })?;

            let files = collect_snapshot_files(&backup_dir)?;
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
        let result = (|| -> KResult<()> {
            fs::create_dir_all(&backup_dir).map_err(|e| {
                let msg = format!(
                    "Failed to create backup restore dir {}: {}",
                    backup_dir.display(),
                    e
                );
                klog_err(msg)
            })?;
            fs::create_dir_all(&restored_db_dir).map_err(|e| {
                let msg = format!(
                    "Failed to create restored db dir {}: {}",
                    restored_db_dir.display(),
                    e
                );
                klog_err(msg)
            })?;

            materialize_snapshot_files(&backup_dir, &archive.files)?;

            let backup_opts = BackupEngineOptions::new(&backup_dir).map_err(|e| {
                let msg = format!(
                    "Failed to create backup engine options for restore {}: {}",
                    backup_dir.display(),
                    e
                );
                klog_err(msg)
            })?;
            let env = Env::new().map_err(|e| {
                let msg = format!("Failed to create rocksdb env for backup restore: {}", e);
                klog_err(msg)
            })?;
            let mut backup_engine = BackupEngine::open(&backup_opts, &env).map_err(|e| {
                let msg = format!(
                    "Failed to open backup engine for restore {}: {}",
                    backup_dir.display(),
                    e
                );
                klog_err(msg)
            })?;
            let mut restore_opts = RestoreOptions::default();
            restore_opts.set_keep_log_files(false);
            backup_engine
                .restore_from_latest_backup(&restored_db_dir, &restored_db_dir, &restore_opts)
                .map_err(|e| {
                    let msg = format!(
                        "Failed to restore latest backup to {}: {}",
                        restored_db_dir.display(),
                        e
                    );
                    klog_err(msg)
                })?;

            let mut opts = Options::default();
            opts.create_if_missing(false);
            let restored_db = DB::open(&opts, &restored_db_dir).map_err(|e| {
                let msg = format!(
                    "Failed to open restored backup db {}: {}",
                    restored_db_dir.display(),
                    e
                );
                klog_err(msg)
            })?;
            self.replace_with_db(&restored_db)?;
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
    let mut files = Vec::new();
    collect_snapshot_files_recursive(root, root, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

fn collect_snapshot_files_recursive(
    root: &Path,
    current: &Path,
    files: &mut Vec<SnapshotFileBlob>,
) -> KResult<()> {
    let entries = fs::read_dir(current).map_err(|e| {
        let msg = format!("Failed to read checkpoint dir {}: {}", current.display(), e);
        klog_err(msg)
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            let msg = format!("Failed to read checkpoint dir entry: {}", e);
            klog_err(msg)
        })?;

        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            let msg = format!(
                "Failed to read checkpoint file type {}: {}",
                path.display(),
                e
            );
            klog_err(msg)
        })?;

        if file_type.is_dir() {
            collect_snapshot_files_recursive(root, &path, files)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let rel_path = path.strip_prefix(root).map_err(|e| {
            let msg = format!(
                "Failed to derive relative checkpoint path {} from {}: {}",
                path.display(),
                root.display(),
                e
            );
            klog_err(msg)
        })?;

        let relative_path = normalize_relative_path(rel_path)?;
        let data = fs::read(&path).map_err(|e| {
            let msg = format!("Failed to read checkpoint file {}: {}", path.display(), e);
            klog_err(msg)
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
                return Err(klog_err(msg));
            }
        }
    }

    if parts.is_empty() {
        return Err(klog_err("Checkpoint file path is empty"));
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
                return Err(klog_err(msg));
            }
        }
    }

    if path.as_os_str().is_empty() {
        return Err(klog_err(format!(
            "Checkpoint relative path is empty: {}",
            relative
        )));
    }

    Ok(root.join(path))
}

fn materialize_snapshot_files(root: &Path, files: &[SnapshotFileBlob]) -> KResult<()> {
    for file in files {
        let dst = safe_join_relative(root, &file.relative_path)?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                let msg = format!(
                    "Failed to create checkpoint restore dir {}: {}",
                    parent.display(),
                    e
                );
                klog_err(msg)
            })?;
        }

        fs::write(&dst, &file.data).map_err(|e| {
            let msg = format!("Failed to write checkpoint file {}: {}", dst.display(), e);
            klog_err(msg)
        })?;
    }

    Ok(())
}

fn try_decode_checkpoint_archive(data: &[u8]) -> KResult<Option<CheckpointSnapshotArchive>> {
    if !data.starts_with(CHECKPOINT_SNAPSHOT_PREFIX) {
        return Ok(None);
    }

    let payload = &data[CHECKPOINT_SNAPSHOT_PREFIX.len()..];
    let decoded: Result<(CheckpointSnapshotArchive, usize), _> =
        bincode::serde::decode_from_slice(payload, bincode::config::legacy());

    let Ok((archive, _)) = decoded else {
        return Ok(None);
    };

    if archive.magic != CHECKPOINT_SNAPSHOT_MAGIC {
        return Ok(None);
    }

    Ok(Some(archive))
}

fn try_decode_backup_engine_archive(data: &[u8]) -> KResult<Option<BackupEngineSnapshotArchive>> {
    if !data.starts_with(BACKUP_ENGINE_SNAPSHOT_PREFIX) {
        return Ok(None);
    }

    let payload = &data[BACKUP_ENGINE_SNAPSHOT_PREFIX.len()..];
    let decoded: Result<(BackupEngineSnapshotArchive, usize), _> =
        bincode::serde::decode_from_slice(payload, bincode::config::legacy());

    let Ok((archive, _)) = decoded else {
        return Ok(None);
    };

    if archive.magic != BACKUP_ENGINE_SNAPSHOT_MAGIC {
        return Ok(None);
    }

    Ok(Some(archive))
}

impl RocksDbSnapshotStrategy for EnumerateSnapshotStrategy {
    fn mode(&self) -> RocksDbSnapshotMode {
        RocksDbSnapshotMode::Enumerate
    }

    fn build_snapshot(&self, store: &RocksDbStateStore) -> KResult<KLogStateSnapshot> {
        let entries = store.read_all_entries()?;
        let data =
            bincode::serde::encode_to_vec(&entries, bincode::config::legacy()).map_err(|e| {
                let msg = format!("Failed to serialize rocksdb enumerate snapshot: {}", e);
                klog_err(msg)
            })?;
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
            return Ok(false);
        }

        let decoded: Result<(Vec<KLogEntry>, usize), _> =
            bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy());

        let Ok((entries, _)) = decoded else {
            return Ok(false);
        };

        store.replace_with_entries(entries)?;
        Ok(true)
    }
}

impl RocksDbSnapshotStrategy for CheckpointSnapshotStrategy {
    fn mode(&self) -> RocksDbSnapshotMode {
        RocksDbSnapshotMode::Checkpoint
    }

    fn build_snapshot(&self, store: &RocksDbStateStore) -> KResult<KLogStateSnapshot> {
        let archive = store.build_checkpoint_archive()?;
        let mut payload = bincode::serde::encode_to_vec(&archive, bincode::config::legacy())
            .map_err(|e| {
                let msg = format!("Failed to serialize checkpoint snapshot archive: {}", e);
                klog_err(msg)
            })?;

        let mut data = Vec::with_capacity(CHECKPOINT_SNAPSHOT_PREFIX.len() + payload.len());
        data.extend_from_slice(CHECKPOINT_SNAPSHOT_PREFIX);
        data.append(&mut payload);

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

        store.apply_checkpoint_archive(&archive)?;
        Ok(true)
    }
}

impl RocksDbSnapshotStrategy for BackupEngineSnapshotStrategy {
    fn mode(&self) -> RocksDbSnapshotMode {
        RocksDbSnapshotMode::BackupEngine
    }

    fn build_snapshot(&self, store: &RocksDbStateStore) -> KResult<KLogStateSnapshot> {
        let archive = store.build_backup_engine_archive()?;
        let mut payload = bincode::serde::encode_to_vec(&archive, bincode::config::legacy())
            .map_err(|e| {
                let msg = format!("Failed to serialize backup engine snapshot archive: {}", e);
                klog_err(msg)
            })?;

        let mut data = Vec::with_capacity(BACKUP_ENGINE_SNAPSHOT_PREFIX.len() + payload.len());
        data.extend_from_slice(BACKUP_ENGINE_SNAPSHOT_PREFIX);
        data.append(&mut payload);

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

        store.apply_backup_engine_archive(&archive)?;
        Ok(true)
    }
}

#[async_trait::async_trait]
impl KLogStateStore for RocksDbStateStore {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let mut batch = WriteBatch::default();
        for entry in entries {
            let key = entry_key(entry.id);
            let value =
                bincode::serde::encode_to_vec(&entry, bincode::config::legacy()).map_err(|e| {
                    let msg = format!("Failed to serialize state entry for rocksdb: {}", e);
                    klog_err(msg)
                })?;
            batch.put(key, value);
        }

        self.db.write(batch).map_err(|e| {
            let msg = format!("Failed to write state entries to rocksdb: {}", e);
            klog_err(msg)
        })?;
        Ok(())
    }

    async fn build_snapshot(&self) -> KResult<KLogStateSnapshot> {
        self.snapshot_builder.build_snapshot(self)
    }

    async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()> {
        for installer in &self.snapshot_installers {
            if installer.try_install_snapshot(self, &snapshot)? {
                return Ok(());
            }
        }

        Err(klog_err(format!(
            "Unsupported rocksdb snapshot payload for mode {:?}",
            self.snapshot_mode
        )))
    }
}
