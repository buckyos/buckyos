use crate::util::persist_format::{PersistPayloadType, decode_with_header, encode_with_header};
use crate::{KLogId, StorageResult};
use crate::{KNode, KNodeId};
use openraft::{StorageError, StorageIOError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

pub type KSnapshotMeta = openraft::SnapshotMeta<KNodeId, KNode>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KSnapshotData {
    pub meta: KSnapshotMeta,
    pub klog_data: Vec<u8>,
}

impl KSnapshotData {
    pub fn new(meta: KSnapshotMeta, klog_data: Vec<u8>) -> Self {
        Self { meta, klog_data }
    }

    pub fn serialize(&self) -> Result<Vec<u8>, String> {
        let buf = encode_with_header(PersistPayloadType::SnapshotData, self).map_err(|e| {
            let msg = format!("Failed to serialize KSnapshotData with header: {}", e);
            error!("{}", msg);
            msg
        })?;

        Ok(buf)
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        let snapshot = decode_with_header(PersistPayloadType::SnapshotData, data).map_err(|e| {
            let msg = format!("Failed to deserialize KSnapshotData with header: {}", e);
            error!("{}", msg);
            msg
        })?;

        Ok(snapshot)
    }
}

#[derive(Debug)]
pub struct SnapshotManager {
    data_dir: PathBuf,
}

impl SnapshotManager {
    pub fn new(parent_dir: PathBuf) -> Self {
        let data_dir = parent_dir.join("snapshots");
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            error!("Failed to create snapshot directory: {}", e);
        }

        info!("Snapshot directory set to: {:?}", data_dir);

        Self { data_dir }
    }

    // Generate a unique snapshot ID based on the current timestamp and last log id
    pub fn generate_snapshot_id(last_log_id: Option<&KLogId>) -> String {
        let now = chrono::Utc::now();
        match last_log_id {
            Some(log_id) => {
                format!("{}_{}_{}", now.timestamp(), log_id.leader_id, log_id.index)
            }
            None => {
                format!("{}_0_0", now.timestamp())
            }
        }
    }

    // Parse a snapshot ID into its timestamp and log id components
    fn parse_snapshot_id(sid: &str) -> Option<(i64, i64)> {
        // First part is the timestamp, last part is the log id
        let Some((ts, _)) = sid.split_once('_') else {
            return None;
        };

        let Ok(ts) = ts.parse::<i64>() else {
            return None;
        };

        let Some((_, log_id)) = sid.rsplit_once('_') else {
            return None;
        };

        let Ok(log_id) = log_id.parse::<i64>() else {
            return None;
        };

        Some((ts, log_id))
    }

    fn parse_snapshot_file_name(file_name: &str) -> Option<(i64, i64)> {
        if !file_name.starts_with("snapshot_") {
            return None;
        }

        // First trim the "snapshot_" prefix
        let sid = &file_name["snapshot_".len()..];

        Self::parse_snapshot_id(sid)
    }

    fn get_temp_snapshot_path(&self) -> PathBuf {
        self.data_dir.join("snapshot.temp")
    }

    fn make_atomic_temp_path(&self, snapshot_id: &str, reason: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        self.data_dir.join(format!(
            ".snapshot_{}_{}_{}_{}",
            snapshot_id,
            reason,
            std::process::id(),
            nanos
        ))
    }

    async fn sync_snapshot_dir(&self) -> std::io::Result<()> {
        let dir = self.data_dir.clone();
        tokio::task::spawn_blocking(move || {
            let dir_handle = std::fs::File::open(&dir)?;
            dir_handle.sync_all()
        })
        .await
        .map_err(|e| std::io::Error::other(format!("Failed to join dir fsync task: {}", e)))?
    }

    async fn write_stream_to_temp<R>(&self, tmp: &Path, src: &mut R) -> std::io::Result<u64>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let mut tmp_file = tokio::fs::File::create_new(tmp).await?;
        let copied = tokio::io::copy(src, &mut tmp_file).await?;
        tmp_file.flush().await?;
        tmp_file.sync_all().await?;
        Ok(copied)
    }

    async fn write_bytes_to_temp(&self, tmp: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut tmp_file = tokio::fs::File::create_new(tmp).await?;
        tmp_file.write_all(bytes).await?;
        tmp_file.flush().await?;
        tmp_file.sync_all().await?;
        Ok(())
    }

    async fn commit_temp_as_snapshot(&self, tmp: &Path, dest: &Path) -> std::io::Result<()> {
        tokio::fs::rename(tmp, dest).await?;
        self.sync_snapshot_dir().await
    }

    pub async fn begin_receiving_snapshot(&self) -> StorageResult<Box<tokio::fs::File>> {
        let path = self.get_temp_snapshot_path();
        info!("Saving incoming snapshot to {:?}", path);

        // Clean up possible existing old data
        if path.exists() {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                error!("Failed to remove existing snapshot file: {}", e);
            } else {
                info!("Removed existing snapshot file: {:?}", path);
            }
        }

        match tokio::fs::File::create(&path).await {
            Ok(file) => Ok(Box::new(file)),
            Err(err) => {
                error!("Failed to create snapshot file: {}", err);
                Err(StorageError::IO {
                    source: StorageIOError::write(&err),
                })
            }
        }
    }

    pub async fn install_snapshot(
        &self,
        meta: &KSnapshotMeta,
        mut snapshot: Box<tokio::fs::File>,
    ) -> StorageResult<KSnapshotData> {
        // TODO Should we remove the temp snapshot file after installation?
        // let src = self.get_temp_snapshot_path();

        let dest = self.data_dir.join(format!("snapshot_{}", meta.snapshot_id));
        let tmp = self.make_atomic_temp_path(&meta.snapshot_id, "install");
        info!("Installing snapshot {} to {:?}", meta.snapshot_id, dest);
        if dest.exists() {
            warn!(
                "Snapshot file already exists: {:?}, replacing with atomic rename",
                dest
            );
        }

        snapshot
            .seek(std::io::SeekFrom::Start(0))
            .await
            .map_err(|err| {
                let msg = format!(
                    "Failed to rewind incoming snapshot stream for {}: {}",
                    meta.snapshot_id, err
                );
                error!("{}", msg);
                let io_err = std::io::Error::other(msg);
                StorageError::IO {
                    source: StorageIOError::write_snapshot(Some(meta.signature()), &io_err),
                }
            })?;

        self.write_stream_to_temp(&tmp, &mut snapshot)
            .await
            .map_err(|err| {
                let _ = std::fs::remove_file(&tmp);
                let msg = format!(
                    "Failed to persist temp snapshot file {:?} before atomic rename: {}",
                    tmp, err
                );
                error!("{}", msg);
                let io_err = std::io::Error::other(msg);
                StorageError::IO {
                    source: StorageIOError::write_snapshot(Some(meta.signature()), &io_err),
                }
            })?;

        self.commit_temp_as_snapshot(&tmp, &dest)
            .await
            .map_err(|err| {
                let _ = std::fs::remove_file(&tmp);
                let msg = format!(
                    "Failed to atomically replace snapshot file {:?} with {:?}: {}",
                    dest, tmp, err
                );
                error!("{}", msg);
                let io_err = std::io::Error::other(msg);
                StorageError::IO {
                    source: StorageIOError::write_snapshot(Some(meta.signature()), &io_err),
                }
            })?;

        /*
        info!("Installing snapshot from {:?} to {:?}", src, dest);
        tokio::fs::copy(&src, &dest).await.map_err(|err| {
            let msg = format!("Failed to copy snapshot file: {}", err);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::write(&err),
            }
        })?;

        // Remove the temp file after successful copy
        tokio::fs::remove_file(src).await.map_err(|err| {
            let msg = format!("Failed to remove temp snapshot file: {}", err);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::write(&err),
            }
        })?;
        */

        let snapshot = self.load_snapshot_from_file(Some(meta), &dest).await?;

        // Check that the loaded snapshot matches the meta
        debug_assert_eq!(meta.snapshot_id, snapshot.meta.snapshot_id);
        debug_assert_eq!(meta.last_log_id, snapshot.meta.last_log_id);
        debug_assert_eq!(meta.last_membership, snapshot.meta.last_membership);

        Ok(snapshot)
    }

    pub async fn load_snapshot_from_file(
        &self,
        meta: Option<&KSnapshotMeta>,
        path: &Path,
    ) -> StorageResult<KSnapshotData> {
        assert!(path.exists(), "Snapshot file does not exist: {:?}", path);

        let bytes = tokio::fs::read(&path).await.map_err(|e| {
            let msg = format!("Failed to read snapshot file {:?}: {}", path, e);
            error!("{}", msg);
            StorageIOError::read_snapshot(meta.map(|m| m.signature()), &e)
        })?;

        let snapshot = KSnapshotData::deserialize(&bytes).map_err(|e| {
            let msg = format!("Failed to deserialize snapshot data: {}", e);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::read_snapshot(
                    meta.map(|m| m.signature()),
                    &std::io::Error::new(std::io::ErrorKind::InvalidData, msg),
                ),
            }
        })?;

        Ok(snapshot)
    }

    pub async fn save_snapshot_to_file(&self, snapshot: &KSnapshotData) -> StorageResult<PathBuf> {
        let path = self
            .data_dir
            .join(format!("snapshot_{}", snapshot.meta.snapshot_id));
        let tmp = self.make_atomic_temp_path(&snapshot.meta.snapshot_id, "save");
        info!("Saving snapshot to file {:?}", path);

        let bytes = snapshot.serialize().map_err(|e| {
            let msg = format!("Failed to serialize snapshot data: {}", e);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::write_state_machine(&std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    msg,
                )),
            }
        })?;

        // Keep previous behavior: snapshot_id collision is treated as error.
        if path.exists() {
            let msg = format!(
                "Snapshot file already exists, refusing overwrite: {:?}",
                path
            );
            error!("{}", msg);
            let io_err = std::io::Error::other(msg);
            return Err(StorageError::IO {
                source: StorageIOError::write_state_machine(&io_err),
            });
        }

        self.write_bytes_to_temp(&tmp, &bytes).await.map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            let msg = format!(
                "Failed to persist temp snapshot file {:?} before atomic rename: {}",
                tmp, e
            );
            error!("{}", msg);
            let io_err = std::io::Error::other(msg);
            StorageError::IO {
                source: StorageIOError::write_state_machine(&io_err),
            }
        })?;

        self.commit_temp_as_snapshot(&tmp, &path)
            .await
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                let msg = format!(
                    "Failed to atomically publish snapshot file {:?} from {:?}: {}",
                    path, tmp, e
                );
                error!("{}", msg);
                let io_err = std::io::Error::other(msg);
                StorageError::IO {
                    source: StorageIOError::write_state_machine(&io_err),
                }
            })?;

        Ok(path)
    }

    // Load the most recent snapshot from the snapshots directory
    pub async fn load_current_snapshot(&self) -> StorageResult<Option<(PathBuf, KSnapshotData)>> {
        if !self.data_dir.exists() {
            warn!("Snapshots directory does not exist: {:?}", self.data_dir);
            return Ok(None);
        }

        // Read the snapshots directory and find the latest snapshot file
        let mut list = tokio::fs::read_dir(&self.data_dir).await.map_err(|err| {
            let msg = format!("Failed to read snapshots directory: {}", err);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::read(&err),
            }
        })?;

        let mut latest_ts: Option<i64> = None;
        let mut latest_file_name = None;
        while let Ok(Some(entry)) = list.next_entry().await {
            let file_name = entry.file_name();
            let name = file_name.to_str().unwrap_or_default();
            if !name.starts_with("snapshot_") {
                info!("Ignoring non-snapshot file in snapshots dir: {}", name);
                continue;
            }

            let meta = entry.metadata().await.map_err(|err| {
                let msg = format!("Failed to get metadata for snapshot file {}: {}", name, err);
                error!("{}", msg);
                StorageError::IO {
                    source: StorageIOError::read(&err),
                }
            })?;
            if meta.is_dir() {
                warn!("Ignoring directory in snapshots dir: {}", name);
                continue;
            }

            let (ts, log_id) = match Self::parse_snapshot_file_name(name) {
                Some((ts, log_id)) => (ts, log_id),
                None => {
                    warn!("Invalid filename in snapshots dir: {}", name);
                    continue;
                }
            };

            if let Some(latest) = latest_ts {
                if ts > latest {
                    latest_ts = Some(ts);
                    latest_file_name = Some(name.to_string());
                } else if ts == latest {
                    // Maybe two different nodes created snapshot at the same second
                    // Then we pick the one with the larger log id

                    let last_name = latest_file_name.as_deref().unwrap_or_default();
                    let last_log_id = match Self::parse_snapshot_file_name(last_name) {
                        Some((_, id)) => id,
                        None => {
                            warn!("Invalid filename in snapshots dir: {}", last_name);
                            continue;
                        }
                    };

                    if log_id > last_log_id {
                        latest_ts = Some(ts);
                        latest_file_name = Some(name.to_string());
                    } else {
                        // keep the existing one
                        info!("Keeping existing snapshot file {} over {}", last_name, name);
                        continue;
                    }
                }
            } else {
                latest_ts = Some(ts);
                latest_file_name = Some(name.to_string());
            }
        }

        if latest_ts.is_none() {
            warn!(
                "No valid snapshot files found in snapshots dir {}",
                self.data_dir.display()
            );
            return Ok(None);
        }

        assert!(latest_file_name.is_some());
        let path = self.data_dir.join(latest_file_name.unwrap());
        info!("Loading latest snapshot from file {:?}", path);

        let data = self.load_snapshot_from_file(None, &path).await?;

        Ok(Some((path, data)))
    }

    /// Clean up old snapshots, keeping only the latest one with id == `last_snapshot_id`
    pub async fn clean_old_snapshots(&self, last_snapshot_id: &str) -> StorageResult<()> {
        if !self.data_dir.exists() {
            warn!("Snapshots directory does not exist: {:?}", self.data_dir);
            return Ok(());
        }

        // Read the snapshots directory and find all snapshot files
        let mut list = tokio::fs::read_dir(&self.data_dir).await.map_err(|err| {
            let msg = format!("Failed to read snapshots directory: {}", err);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::read(&err),
            }
        })?;

        let mut snapshots = vec![];
        while let Ok(Some(entry)) = list.next_entry().await {
            let file_name = entry.file_name();
            let name = file_name.to_str().unwrap_or_default();
            if !name.starts_with("snapshot_") {
                info!("Ignoring non-snapshot file in snapshots dir: {}", name);
                continue;
            }

            let meta = entry.metadata().await.map_err(|err| {
                let msg = format!("Failed to get metadata for snapshot file {}: {}", name, err);
                error!("{}", msg);
                StorageError::IO {
                    source: StorageIOError::read(&err),
                }
            })?;
            if meta.is_dir() {
                warn!("Ignoring directory in snapshots dir: {}", name);
                continue;
            }

            let sid = &name["snapshot_".len()..];
            if sid != last_snapshot_id {
                snapshots.push(entry.path());
            }
        }

        // Delete all old snapshots
        for path in snapshots {
            info!("Removing old snapshot file {:?}", path);
            if let Err(e) = tokio::fs::remove_file(&path).await {
                error!("Failed to remove old snapshot file {:?}: {}", path, e);
            }
        }

        info!("Old snapshots cleanup completed.");
        Ok(())
    }

    /// Remove all snapshots in the snapshots directory
    pub async fn clean_all_snapshots(&self) -> StorageResult<()> {
        if !self.data_dir.exists() {
            warn!("Snapshots directory does not exist: {:?}", self.data_dir);
            return Ok(());
        }

        // Remove the snapshots directory and all its contents
        info!("Removing all snapshots in directory {:?}", self.data_dir);
        if let Err(e) = tokio::fs::remove_dir_all(&self.data_dir).await {
            error!(
                "Failed to remove snapshots directory {:?}: {}",
                self.data_dir, e
            );
            return Err(StorageError::IO {
                source: StorageIOError::write(&e),
            });
        }

        // Recreate the snapshots directory
        if let Err(e) = tokio::fs::create_dir_all(&self.data_dir).await {
            error!(
                "Failed to recreate snapshots directory {:?}: {}",
                self.data_dir, e
            );
            return Err(StorageError::IO {
                source: StorageIOError::write(&e),
            });
        }

        info!("All snapshots removed and directory recreated.");
        Ok(())
    }
}

pub type SnapshotManagerRef = Arc<SnapshotManager>;
