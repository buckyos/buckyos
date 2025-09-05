use openraft::entry::RaftPayload;
use openraft::{LogId, OptionalSend, Vote, Entry};
use openraft::{RaftLogReader, storage::{RaftLogStorage, LogFlushed}};
use tracing_subscriber::field::debug;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::RangeBounds;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;

use crate::{KNodeId, KTypeConfig, StorageResult};

type LogEntry = Entry<KTypeConfig>;

#[derive(Debug, Clone)]
struct MemoryLogState {
    last_purged: Option<LogId<KNodeId>>,
    last_applied: Option<LogId<KNodeId>>,
    vote: Option<Vote<KNodeId>>,
}

impl MemoryLogState {
    fn new() -> Self {
        Self {
            last_purged: None,
            last_applied: None,
            vote: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryLogStorage {
    state: Arc<AsyncMutex<MemoryLogState>>,
    logs: Arc<AsyncMutex<BTreeMap<u64, LogEntry>>>,
}

impl MemoryLogStorage {
    pub fn new() -> Self {
        let logs = BTreeMap::new();
        let logs = Arc::new(AsyncMutex::new(logs));

        let state = MemoryLogState::new();
        let state = Arc::new(AsyncMutex::new(state));

        Self { logs, state }
    }
}

impl RaftLogReader<KTypeConfig> for MemoryLogStorage {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> StorageResult<Vec<LogEntry>> {
        debug!("try_get_log_entries: range={:?}", range);

        let logs = self.logs.lock().await;
        let entries: Vec<LogEntry> = logs.range(range).map(|(_, entry)| entry.clone()).collect();

        for entry in &entries {
            if entry.get_membership().is_some() {
                debug!("Found membership entry: {:?}", entry);
            }
        }
        
        Ok(entries)
    }
}

impl RaftLogStorage<KTypeConfig> for MemoryLogStorage {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> StorageResult<openraft::LogState<KTypeConfig>> {
        let last_log_id = {
            let logs = self.logs.lock().await;
            logs.iter().last().map(|(_, item)| item.log_id.clone())
        };

        let last_purged_log_id = {
            let state = self.state.lock().await;
            state.last_purged.clone()
        };
        let last_log_id = match last_log_id {
            Some(id) => Some(id),
            None => last_purged_log_id,
        };

        Ok(openraft::LogState {
            last_log_id,
            last_purged_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote<KNodeId>) -> StorageResult<()> {
        let mut state = self.state.lock().await;
        debug!("save_vote: {:?}", vote);
        state.vote = Some(vote.clone());

        Ok(())
    }

    async fn read_vote(&mut self) -> StorageResult<Option<Vote<KNodeId>>> {
        let state = self.state.lock().await;
        Ok(state.vote.clone())
    }

    async fn append<I>(&mut self, entries: I, callback: LogFlushed<KTypeConfig>) -> StorageResult<()>
    where
        I: IntoIterator<Item = LogEntry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut logs = self.logs.lock().await;
        for entry in entries {
            debug!("Appending raft log entry: {:?}", entry);
            logs.insert(entry.log_id.index, entry);
        }

        callback.log_io_completed(Ok(()));

        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<KNodeId>) -> StorageResult<()> {
        info!("Truncating raft logs from index {}", log_id.index);

        let mut logs = self.logs.lock().await;

        // Remove all entries with index >= log_id.index
        logs.split_off(&log_id.index);
        Ok(())
    }

    /// Remove all log entries that index <= `log_id.index`
    async fn purge(&mut self, log_id: LogId<KNodeId>) -> StorageResult<()> {
        info!("Purging raft logs up to index {}", log_id.index);

        {
            let mut logs = self.logs.lock().await;
            let new_logs=  logs.split_off(&(log_id.index + 1));
            *logs = new_logs;
        }

        let mut state = self.state.lock().await;
        state.last_purged = Some(log_id);

        Ok(())
    }
}
