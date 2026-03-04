use super::snapshot::{KSnapshotMeta, SnapshotManager, SnapshotManagerRef};
use crate::state_machine::snapshot::KSnapshotData;
use crate::state_store::KLogStateStoreManagerRef;
use crate::state_store::{KLogStateMachineMeta, KLogStateSnapshot};
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
pub struct KLogStateMachine {
    data: Arc<AsyncRwLock<StateMachineData>>,

    state_store: KLogStateStoreManagerRef,

    snapshot_manager: SnapshotManagerRef,
}

impl KLogStateMachine {
    pub async fn new(
        state_store: KLogStateStoreManagerRef,
        snapshot_manager: SnapshotManagerRef,
    ) -> StorageResult<Self> {
        let recovered = state_store.load_state_machine_meta().await.map_err(|e| {
            let msg = format!("Failed to load state machine metadata: {}", e);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::read(&std::io::Error::other(msg)),
            }
        })?;

        let data = if let Some(meta) = recovered {
            info!(
                "StateMachine metadata loaded from store: last_applied={:?}, membership={:?}",
                meta.last_applied_log_id, meta.last_membership
            );
            StateMachineData {
                last_applied_log_id: meta.last_applied_log_id,
                last_membership: meta.last_membership,
            }
        } else {
            info!("StateMachine metadata not found in store, using defaults");
            StateMachineData::default()
        };

        Ok(Self {
            data: Arc::new(AsyncRwLock::new(data)),
            state_store,
            snapshot_manager,
        })
    }

    async fn persist_state_machine_meta(
        &self,
        last_applied_log_id: Option<KLogId>,
        last_membership: KStoredMembership,
    ) -> StorageResult<()> {
        self.state_store
            .save_state_machine_meta(KLogStateMachineMeta {
                last_applied_log_id,
                last_membership,
            })
            .await
            .map_err(|e| {
                let msg = format!("Failed to persist state machine metadata: {}", e);
                error!("{}", msg);
                StorageError::IO {
                    source: StorageIOError::write(&std::io::Error::other(msg)),
                }
            })
    }

    async fn process_request(&self, req: KLogRequest) -> KLogResponse {
        match req {
            KLogRequest::AppendLog { item } => {
                // Id is expected to be assigned on leader before log replication.
                // State machine apply must be deterministic and should not mutate ids.
                debug!(
                    "StateMachine process append request: id={}, ts={}, node_id={}, msg_len={}",
                    item.id,
                    item.timestamp,
                    item.node_id,
                    item.message.len()
                );
                match self.state_store.append_prepared_entry(item).await {
                    Ok(id) => {
                        debug!("StateMachine append request committed: id={}", id);
                        KLogResponse::AppendOk { id }
                    }
                    Err(err) => {
                        error!("StateMachine append request failed: {}", err);
                        KLogResponse::Err(err.to_string())
                    }
                }
            }
            KLogRequest::PutMeta { item } => {
                debug!(
                    "StateMachine process put-meta request: key={}, value_len={}, updated_at={}, updated_by={}",
                    item.key,
                    item.value.len(),
                    item.updated_at,
                    item.updated_by
                );
                let key = item.key.clone();
                match self.state_store.put_meta_entry(item).await {
                    Ok(stored) => {
                        debug!(
                            "StateMachine put-meta request committed: key={}, revision={}",
                            key, stored.revision
                        );
                        KLogResponse::MetaPutOk {
                            key,
                            revision: stored.revision,
                        }
                    }
                    Err(err) => {
                        error!("StateMachine put-meta request failed: {}", err);
                        KLogResponse::Err(err.to_string())
                    }
                }
            }
            KLogRequest::DeleteMeta { key } => {
                debug!("StateMachine process delete-meta request: key={}", key);
                match self.state_store.delete_meta_key(&key).await {
                    Ok(prev_revision) => {
                        let existed = prev_revision.is_some();
                        debug!(
                            "StateMachine delete-meta request committed: key={}, existed={}, prev_revision={:?}",
                            key, existed, prev_revision
                        );
                        KLogResponse::MetaDeleteOk {
                            key,
                            existed,
                            prev_revision,
                        }
                    }
                    Err(err) => {
                        error!("StateMachine delete-meta request failed: {}", err);
                        KLogResponse::Err(err.to_string())
                    }
                }
            }
        }
    }
}

impl RaftStateMachine<KTypeConfig> for KLogStateMachine {
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

        let persisted_last_applied = data.last_applied_log_id;
        let persisted_membership = data.last_membership.clone();
        drop(data);
        self.persist_state_machine_meta(persisted_last_applied, persisted_membership)
            .await?;

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
        info!(
            "StateMachine install_snapshot start: snapshot_id={}, last_log_id={:?}, last_membership={:?}",
            meta.snapshot_id, meta.last_log_id, meta.last_membership
        );
        let data = self
            .snapshot_manager
            .install_snapshot(meta, snapshot)
            .await?;
        info!(
            "StateMachine install_snapshot persisted file: snapshot_id={}, klog_data_bytes={}",
            data.meta.snapshot_id,
            data.klog_data.len()
        );

        // First, restore state store from snapshot payload.
        self.state_store
            .install_snapshot(KLogStateSnapshot {
                data: data.klog_data,
            })
            .await
            .map_err(|e| {
                let msg = format!("Failed to install state store snapshot: {}", e);
                error!("{}", msg);
                StorageError::IO {
                    source: StorageIOError::write_snapshot(None, &std::io::Error::other(msg)),
                }
            })?;
        self.persist_state_machine_meta(
            data.meta.last_log_id.clone(),
            data.meta.last_membership.clone(),
        )
        .await?;

        // Then, update the in-memory state machine metadata.
        let mut state = self.data.write().await;
        state.last_applied_log_id = data.meta.last_log_id;
        state.last_membership = data.meta.last_membership;
        debug!(
            "StateMachine install_snapshot state updated: last_applied={:?}, membership={:?}",
            state.last_applied_log_id, state.last_membership
        );
        info!(
            "StateMachine install_snapshot completed: snapshot_id={}",
            meta.snapshot_id
        );

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

impl RaftSnapshotBuilder<KTypeConfig> for KLogStateMachine {
    async fn build_snapshot(&mut self) -> StorageResult<KSnapshot> {
        info!("StateMachine build_snapshot start");
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
        info!(
            "StateMachine build_snapshot meta prepared: snapshot_id={}, last_log_id={:?}, last_membership={:?}",
            meta.snapshot_id, meta.last_log_id, meta.last_membership
        );

        let klog_data = self.state_store.build_snapshot().await.map_err(|e| {
            let msg = format!("Failed to build state store snapshot: {}", e);
            error!("{}", msg);
            StorageError::IO {
                source: StorageIOError::write_snapshot(None, &e),
            }
        })?;
        info!(
            "StateMachine build_snapshot state_store ready: snapshot_id={}, klog_data_bytes={}",
            meta.snapshot_id,
            klog_data.data.len()
        );

        let snapshot = KSnapshotData::new(meta, klog_data.data);

        // First save the snapshot to disk
        let file = self
            .snapshot_manager
            .save_snapshot_to_file(&snapshot)
            .await?;
        info!(
            "StateMachine build_snapshot file saved: snapshot_id={}, path={}",
            snapshot.meta.snapshot_id,
            file.display()
        );

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
        info!(
            "StateMachine build_snapshot completed: snapshot_id={}",
            snapshot.meta.snapshot_id
        );

        Ok(snapshot)
    }
}
