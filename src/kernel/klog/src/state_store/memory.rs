use super::store::{KLogStateSnapshot, KLogStateStore};
use crate::{KLogEntry, KLogError, KResult};
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;

/// A simple in-memory state store implementation.
pub struct MemoryStateStore {
    logs: Arc<AsyncMutex<Vec<KLogEntry>>>,
}

impl MemoryStateStore {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(AsyncMutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl KLogStateStore for MemoryStateStore {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let mut logs = self.logs.lock().await;
        logs.extend(entries);

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

        let mut logs = self.logs.lock().await;
        *logs = entries;
        Ok(())
    }
}
