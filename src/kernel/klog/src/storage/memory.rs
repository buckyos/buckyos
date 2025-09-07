use super::storage::{KLogStorage, KLogStorageSnapshot};
use crate::{KResult, KLogEntry, KLogError};
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;

/// A simple in-memory log storage implementation
pub struct SimpleLogStorage {
    logs: Arc<AsyncMutex<Vec<KLogEntry>>>,
}

impl SimpleLogStorage {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(AsyncMutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl KLogStorage for SimpleLogStorage {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let mut logs = self.logs.lock().await;
        logs.extend(entries);

        Ok(())
    }

    async fn build_snapshot(&self) -> KResult<KLogStorageSnapshot> {
        let logs = self.logs.lock().await;
        let data =
            bincode::serde::encode_to_vec(&*logs, bincode::config::legacy()).map_err(|e| {
                let msg = format!("Failed to serialize logs for snapshot: {}", e);
                error!("{}", msg);
                KLogError::InvalidFormat(msg)
            })?;

        Ok(KLogStorageSnapshot { data })
    }
}
