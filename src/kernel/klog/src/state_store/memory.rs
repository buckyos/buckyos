use super::store::{KLogStateSnapshot, KLogStateStore};
use crate::{KLogEntry, KLogError, KResult};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex as AsyncMutex;

/// A simple in-memory state store implementation.
pub struct MemoryStateStore {
    logs: Arc<AsyncMutex<Vec<KLogEntry>>>,
    next_log_id: AtomicU64,
}

impl MemoryStateStore {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(AsyncMutex::new(Vec::new())),
            next_log_id: AtomicU64::new(1),
        }
    }
}

#[async_trait::async_trait]
impl KLogStateStore for MemoryStateStore {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let candidate_next = entries
            .iter()
            .map(|e| e.id.saturating_add(1))
            .max()
            .unwrap_or(0);
        let mut logs = self.logs.lock().await;
        logs.extend(entries);
        drop(logs);

        if candidate_next > 0 {
            let mut current = self.next_log_id.load(Ordering::SeqCst);
            while candidate_next > current {
                match self.next_log_id.compare_exchange(
                    current,
                    candidate_next,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => current = actual,
                }
            }
        }

        Ok(())
    }

    async fn build_snapshot(&self) -> KResult<KLogStateSnapshot> {
        let logs = self.logs.lock().await;
        let data =
            bincode::serde::encode_to_vec(&*logs, bincode::config::legacy()).map_err(|e| {
                let msg = format!("Failed to serialize logs for snapshot: {}", e);
                error!("{}", msg);
                KLogError::InvalidFormat(msg)
            })?;

        Ok(KLogStateSnapshot { data })
    }

    async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()> {
        let (entries, _): (Vec<KLogEntry>, usize) =
            bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy()).map_err(
                |e| {
                    let msg = format!("Failed to decode state snapshot: {}", e);
                    error!("{}", msg);
                    KLogError::InvalidFormat(msg)
                },
            )?;

        let candidate_next = entries
            .iter()
            .map(|e| e.id.saturating_add(1))
            .max()
            .unwrap_or(1);
        let mut logs = self.logs.lock().await;
        *logs = entries;
        self.next_log_id.store(candidate_next, Ordering::SeqCst);
        debug!(
            "MemoryStateStore install_snapshot reset next_log_id={}",
            candidate_next
        );
        Ok(())
    }

    async fn load_next_log_id(&self) -> KResult<Option<u64>> {
        Ok(Some(self.next_log_id.load(Ordering::SeqCst)))
    }

    async fn save_next_log_id(&self, next_log_id: u64) -> KResult<()> {
        self.next_log_id.store(next_log_id, Ordering::SeqCst);
        Ok(())
    }
}
