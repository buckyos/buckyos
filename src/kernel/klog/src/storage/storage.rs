use crate::{KLogEntry, KResult};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct KLogStorageSnapshot {
    pub data: Vec<u8>,
}

#[async_trait::async_trait]
pub trait KLogStorage: Send + Sync {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()>;

    async fn build_snapshot(&self) -> KResult<KLogStorageSnapshot>;
}

pub type KLogStorageRef = Arc<Box<dyn KLogStorage>>;

pub struct KLogStorageManager {
    storage: KLogStorageRef,

    // The kernel state: next id to assign to the next log entry
    next_log_id: AtomicU64,
}

impl std::fmt::Debug for KLogStorageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KLogStorageManager")
            .field("next_log_id", &self.next_log_id.load(Ordering::SeqCst))
            .finish()
    }
}

impl KLogStorageManager {
    pub fn new(storage: KLogStorageRef) -> Self {
        Self {
            storage,
            next_log_id: AtomicU64::new(1),
        }
    }

    pub async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        self.storage.append(entries).await
    }

    pub async fn process_append_request(&self, item: KLogEntry) -> KResult<u64> {
        let mut entry = item;
        let id = self.next_log_id.fetch_add(1, Ordering::SeqCst); // Assign and increment
        entry.id = id;

        self.append(vec![entry]).await?;
        Ok(id)
    }

    pub async fn build_snapshot(&self) -> KResult<KLogStorageSnapshot> {
        self.storage.build_snapshot().await
    }
}

pub type KLogStorageManagerRef = Arc<KLogStorageManager>;
