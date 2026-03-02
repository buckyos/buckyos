use crate::{KLogEntry, KLogId, KNode, KNodeId, KResult};
use openraft::StoredMembership;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct KLogStateSnapshot {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KLogQueryOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KLogQuery {
    pub start_id: Option<u64>,
    pub end_id: Option<u64>,
    pub limit: usize,
    pub order: KLogQueryOrder,
}

impl Default for KLogQuery {
    fn default() -> Self {
        Self {
            start_id: None,
            end_id: None,
            limit: 100,
            order: KLogQueryOrder::Asc,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KLogStateMachineMeta {
    pub last_applied_log_id: Option<KLogId>,
    pub last_membership: StoredMembership<KNodeId, KNode>,
}

#[async_trait::async_trait]
pub trait KLogStateStore: Send + Sync {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()>;

    async fn query(&self, query: KLogQuery) -> KResult<Vec<KLogEntry>>;

    async fn build_snapshot(&self) -> KResult<KLogStateSnapshot>;

    async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()>;

    /// Load persisted next-log-id cursor.
    /// Return `Ok(None)` only when the store has not initialized this metadata yet.
    async fn load_next_log_id(&self) -> KResult<Option<u64>>;

    /// Persist next-log-id cursor.
    async fn save_next_log_id(&self, next_log_id: u64) -> KResult<()>;

    /// Load persisted state-machine metadata.
    async fn load_state_machine_meta(&self) -> KResult<Option<KLogStateMachineMeta>>;

    /// Persist state-machine metadata.
    async fn save_state_machine_meta(&self, meta: KLogStateMachineMeta) -> KResult<()>;
}

pub type KLogStateStoreRef = Arc<Box<dyn KLogStateStore>>;

pub struct KLogStateStoreManager {
    state_store: KLogStateStoreRef,

    // The kernel state: next id to assign to the next state entry
    next_log_id: AtomicU64,
}

impl std::fmt::Debug for KLogStateStoreManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KLogStateStoreManager")
            .field("next_log_id", &self.next_log_id.load(Ordering::SeqCst))
            .finish()
    }
}

impl KLogStateStoreManager {
    pub async fn new(state_store: KLogStateStoreRef) -> KResult<Self> {
        let recovered_next = state_store.load_next_log_id().await?.unwrap_or(1);
        info!(
            "KLogStateStoreManager init next_log_id from store: {}",
            recovered_next
        );

        Ok(Self {
            state_store,
            next_log_id: AtomicU64::new(recovered_next),
        })
    }

    pub async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let committed_next = entries
            .iter()
            .map(|e| e.id.saturating_add(1))
            .max()
            .unwrap_or(0);

        self.state_store.append(entries).await?;
        self.advance_next_log_id(committed_next).await
    }

    /// Allocate a deterministic id on leader before writing to raft log.
    pub fn alloc_log_id(&self) -> u64 {
        self.next_log_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn peek_next_log_id(&self) -> u64 {
        self.next_log_id.load(Ordering::SeqCst)
    }

    /// Prepare an append entry on leader side.
    /// If client did not provide id(0), assign one here.
    pub fn prepare_append_entry(&self, mut item: KLogEntry) -> KLogEntry {
        if item.id == 0 {
            item.id = self.alloc_log_id();
        }
        item
    }

    /// Append an already prepared entry.
    /// This is used by state machine apply path to avoid re-assigning ids on followers.
    pub async fn append_prepared_entry(&self, item: KLogEntry) -> KResult<u64> {
        let id = item.id;
        self.append(vec![item]).await?;
        Ok(id)
    }

    pub async fn build_snapshot(&self) -> KResult<KLogStateSnapshot> {
        self.state_store.build_snapshot().await
    }

    pub async fn query_entries(&self, query: KLogQuery) -> KResult<Vec<KLogEntry>> {
        self.state_store.query(query).await
    }

    pub async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()> {
        self.state_store.install_snapshot(snapshot).await?;
        let recovered_next = self.state_store.load_next_log_id().await?.unwrap_or(1);
        self.next_log_id.store(recovered_next, Ordering::SeqCst);
        info!(
            "KLogStateStoreManager install_snapshot reload next_log_id from store: {}",
            recovered_next
        );
        Ok(())
    }

    pub async fn load_state_machine_meta(&self) -> KResult<Option<KLogStateMachineMeta>> {
        self.state_store.load_state_machine_meta().await
    }

    pub async fn save_state_machine_meta(&self, meta: KLogStateMachineMeta) -> KResult<()> {
        self.state_store.save_state_machine_meta(meta).await
    }

    async fn advance_next_log_id(&self, candidate_next: u64) -> KResult<()> {
        if candidate_next == 0 {
            return Ok(());
        }

        let mut current = self.next_log_id.load(Ordering::SeqCst);
        while candidate_next > current {
            match self.next_log_id.compare_exchange(
                current,
                candidate_next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    self.state_store.save_next_log_id(candidate_next).await?;
                    debug!(
                        "KLogStateStoreManager advanced next_log_id: {} -> {}",
                        current, candidate_next
                    );
                    return Ok(());
                }
                Err(actual) => current = actual,
            }
        }

        Ok(())
    }
}

pub type KLogStateStoreManagerRef = Arc<KLogStateStoreManager>;
