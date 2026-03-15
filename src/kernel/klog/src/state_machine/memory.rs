use super::snapshot::{KSnapshotMeta, SnapshotManager, SnapshotManagerRef};
use crate::state_machine::snapshot::KSnapshotData;
use crate::storage::KLogStorageManagerRef;
use crate::{KLogId, KLogRequest, KLogResponse, KNode, KNodeId, KTypeConfig, StorageResult};
use openraft::{
    Entry, EntryPayload, OptionalSend, RaftSnapshotBuilder, SnapshotMeta, StoredMembership,
    storage::RaftStateMachine,
};
use openraft::{Snapshot, StorageError, StorageIOError};
use std::sync::Arc;
use tokio::sync::RwLock as AsyncRwLock;

pub type KStoredMembership = StoredMembership<KNodeId, KNode>;
type KEntry = Entry<KTypeConfig>;
type KSnapshot = Snapshot<KTypeConfig>;

#[derive(Debug, Default)]
pub struct StateMachineData {
    last_applied_log_id: Option<KLogId>,
    last_membership: KStoredMembership,
}

#[derive(Debug, Clone)]
pub struct KLogMemoryStateMachine {
    data: Arc<AsyncRwLock<StateMachineData>>,

    log_storage: KLogStorageManagerRef,

    snapshot_manager: SnapshotManagerRef,
}

impl KLogMemoryStateMachine {
    pub fn new(log_storage: KLogStorageManagerRef, snapshot_manager: SnapshotManagerRef) -> Self {
        Self {
            data: Arc::new(AsyncRwLock::new(StateMachineData::default())),
            log_storage,
            snapshot_manager,
        }
    }

    async fn process_request(&self, req: KLogRequest) -> KLogResponse {
        match req {
            KLogRequest::AppendLog { item } => {
                match self.log_storage.process_append_request(item).await {
                    Ok(id) => KLogResponse::AppendOk { id },
                    Err(err) => KLogResponse::Err(err.to_string()),
                }
            }
        }
    }
}

impl RaftStateMachine<KTypeConfig> for KLogMemoryStateMachine {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> StorageResult<(Option<KLogId>, KStoredMembership)> {
        let data = self.data.read().await;
        Ok((
            data.last_applied_log_id.clone(),
            data.last_membership.clone(),
        ))
    }

    async fn apply<I>(&mut self, entries: I) -> StorageResult<Vec<KLogResponse>>
    where
        I: IntoIterator<Item = KEntry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let entries = entries.into_iter();
        let mut replies = Vec::with_capacity(entries.size_hint().0);

        let mut data = self.data.write().await;

        for entry in entries {
            data.last_applied_log_id = Some(entry.log_id);

            // we are using sync sends -> unbounded channels
            let resp_value = match entry.payload {
                EntryPayload::Blank => KLogResponse::Empty,

                EntryPayload::Normal(req) => self.process_request(req).await,

                EntryPayload::Membership(mem) => {
                    info!("Updating membership to: {:?}", mem);

                    data.last_membership = StoredMembership::new(Some(entry.log_id), mem);
                    KLogResponse::Empty
                }
            };

            replies.push(resp_value);
        }

        Ok(replies)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> StorageResult<Box<tokio::fs::File>> {
        self.snapshot_manager.begin_receiving_snapshot().await
    }

    async fn install_snapshot(
        &mut self,
        meta: &KSnapshotMeta,
        snapshot: Box<tokio::fs::File>,
    ) -> StorageResult<()> {
        let data = self
            .snapshot_manager
            .install_snapshot(meta, snapshot)
            .await?;

        // First, update the state machine data
        let mut state = self.data.write().await;
        state.last_applied_log_id = data.meta.last_log_id.clone();
        state.last_membership = data.meta.last_membership.clone();
        drop(state); // Release the lock before potentially long operations

        // Then, update the state machine's internal data structures
        // todo!();

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> StorageResult<Option<KSnapshot>> {
        let ret = self.snapshot_manager.load_current_snapshot().await?;
        if ret.is_none() {
            info!("No current snapshot available");
            return Ok(None);
        }

        let (path, snapshot) = ret.unwrap();
        let file = tokio::fs::File::open(&path).await.map_err(|err| {
            let msg = format!("Failed to open snapshot file: {:?}, {}", path, err);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::read(&err),
            }
        })?;

        let snapshot = KSnapshot {
            meta: snapshot.meta,
            snapshot: Box::new(file),
        };

        Ok(Some(snapshot))
    }
}

impl RaftSnapshotBuilder<KTypeConfig> for KLogMemoryStateMachine {
    async fn build_snapshot(&mut self) -> StorageResult<KSnapshot> {
        let meta = {
            let data = self.data.read().await;

            let snapshot_id =
                SnapshotManager::generate_snapshot_id(data.last_applied_log_id.as_ref());

            let meta = SnapshotMeta {
                last_log_id: data.last_applied_log_id,
                last_membership: data.last_membership.clone(),
                snapshot_id,
            };

            meta
        };

        let klog_data = self.log_storage.build_snapshot().await.map_err(|e| {
            let msg = format!("Failed to build log storage snapshot: {}", e);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::write_snapshot(None, &e),
            }
        })?;

        let snapshot = KSnapshotData::new(meta, klog_data.data);

        // First save the snapshot to disk
        let file = self
            .snapshot_manager
            .save_snapshot_to_file(&snapshot)
            .await?;

        // Then start a task to clean up old snapshots
        let snapshot_id = snapshot.meta.snapshot_id.clone();
        let snapshot_manager = self.snapshot_manager.clone();
        tokio::spawn(async move {
            if let Err(e) = snapshot_manager.clean_old_snapshots(&snapshot_id).await {
                error!("Failed to clean old snapshots: {}", e);
            }
        });

        let file = tokio::fs::File::open(&file).await.map_err(|err| {
            let msg = format!("Failed to open snapshot file: {:?}, {}", file, err);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::read(&err),
            }
        })?;

        let snapshot = Snapshot {
            meta: snapshot.meta,
            snapshot: Box::new(file),
        };

        Ok(snapshot)
    }
}
