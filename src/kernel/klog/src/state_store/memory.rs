use super::store::{
    KLogQuery, KLogQueryOrder, KLogStateMachineMeta, KLogStateSnapshot, KLogStateStore,
};
use crate::{KLogEntry, KLogError, KResult};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex as AsyncMutex;

/// A simple in-memory state store implementation.
pub struct MemoryStateStore {
    logs: Arc<AsyncMutex<Vec<KLogEntry>>>,
    next_log_id: AtomicU64,
    state_machine_meta: Arc<AsyncMutex<Option<KLogStateMachineMeta>>>,
}

impl MemoryStateStore {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(AsyncMutex::new(Vec::new())),
            next_log_id: AtomicU64::new(1),
            state_machine_meta: Arc::new(AsyncMutex::new(None)),
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

    async fn query(&self, query: KLogQuery) -> KResult<Vec<KLogEntry>> {
        let logs = self.logs.lock().await;
        let mut entries = logs
            .iter()
            .filter(|e| {
                query.start_id.map(|start| e.id >= start).unwrap_or(true)
                    && query.end_id.map(|end| e.id <= end).unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        drop(logs);

        entries.sort_by_key(|e| e.id);
        if query.order == KLogQueryOrder::Desc {
            entries.reverse();
        }

        if entries.len() > query.limit {
            entries.truncate(query.limit);
        }

        Ok(entries)
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

    async fn load_state_machine_meta(&self) -> KResult<Option<KLogStateMachineMeta>> {
        let meta = self.state_machine_meta.lock().await;
        Ok(meta.clone())
    }

    async fn save_state_machine_meta(&self, meta: KLogStateMachineMeta) -> KResult<()> {
        let mut guard = self.state_machine_meta.lock().await;
        *guard = Some(meta);
        Ok(())
    }
}
