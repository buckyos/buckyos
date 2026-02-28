use crate::{KLogEntry, KResult};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct KLogStateSnapshot {
    pub data: Vec<u8>,
}

#[async_trait::async_trait]
pub trait KLogStateStore: Send + Sync {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()>;

    async fn build_snapshot(&self) -> KResult<KLogStateSnapshot>;

    async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()>;
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
    pub fn new(state_store: KLogStateStoreRef) -> Self {
        Self {
            state_store,
            next_log_id: AtomicU64::new(1),
        }
    }

    pub async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        self.state_store.append(entries).await
    }

    /// Allocate a deterministic id on leader before writing to raft log.
    pub fn alloc_log_id(&self) -> u64 {
        self.next_log_id.fetch_add(1, Ordering::SeqCst)
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

    pub async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()> {
        self.state_store.install_snapshot(snapshot).await
    }
}

pub type KLogStateStoreManagerRef = Arc<KLogStateStoreManager>;
