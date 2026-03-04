use crate::{KLogEntry, KLogId, KLogLevel, KLogMetaEntry, KNode, KNodeId, KResult};
use openraft::StoredMembership;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as AsyncMutex;

pub(crate) const REQUEST_DEDUP_WINDOW_MS: u64 = 5 * 60 * 1000;
pub(crate) const REQUEST_DEDUP_MAX_ITEMS: usize = 10_000;

pub struct KLogStateSnapshot {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KLogMetaPutResult {
    Stored(KLogMetaEntry),
    VersionConflict {
        expected_revision: u64,
        current_revision: Option<u64>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KLogStateSnapshotData {
    pub entries: Vec<KLogEntry>,
    pub meta_entries: Vec<KLogMetaEntry>,
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
    pub level: Option<KLogLevel>,
    pub source: Option<String>,
    pub attr_key: Option<String>,
    pub attr_value: Option<String>,
}

impl Default for KLogQuery {
    fn default() -> Self {
        Self {
            start_id: None,
            end_id: None,
            limit: 100,
            order: KLogQueryOrder::Asc,
            level: None,
            source: None,
            attr_key: None,
            attr_value: None,
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

    async fn put_meta(&self, item: KLogMetaEntry) -> KResult<KLogMetaEntry>;

    async fn delete_meta(&self, key: &str) -> KResult<Option<KLogMetaEntry>>;

    async fn get_meta(&self, key: &str) -> KResult<Option<KLogMetaEntry>>;

    async fn list_meta(&self, prefix: Option<&str>, limit: usize) -> KResult<Vec<KLogMetaEntry>>;

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

    /// Lookup recent request id for idempotency.
    /// Returns committed log id when request id is still valid in dedup window.
    async fn lookup_recent_request_id(&self, request_id: &str, now_ms: u64)
    -> KResult<Option<u64>>;
}

pub type KLogStateStoreRef = Arc<Box<dyn KLogStateStore>>;

#[derive(Debug, Clone, Copy)]
struct RequestDedupRecord {
    log_id: u64,
    seen_at_ms: u64,
}

#[derive(Debug, Default)]
struct RequestDedupCache {
    records: HashMap<String, RequestDedupRecord>,
    order: VecDeque<(u64, String)>,
}

impl RequestDedupCache {
    fn lookup(&mut self, request_id: &str, now_ms: u64) -> Option<u64> {
        self.cleanup(now_ms);
        self.records.get(request_id).map(|r| r.log_id)
    }

    fn remember(&mut self, request_id: String, log_id: u64, now_ms: u64) {
        self.cleanup(now_ms);

        self.records.insert(
            request_id.clone(),
            RequestDedupRecord {
                log_id,
                seen_at_ms: now_ms,
            },
        );
        self.order.push_back((now_ms, request_id));
        self.cleanup(now_ms);
    }

    fn clear(&mut self) {
        self.records.clear();
        self.order.clear();
    }

    fn cleanup(&mut self, now_ms: u64) {
        loop {
            let should_pop = match self.order.front() {
                Some((seen_at_ms, _)) => {
                    let expired = now_ms.saturating_sub(*seen_at_ms) > REQUEST_DEDUP_WINDOW_MS;
                    expired || self.records.len() > REQUEST_DEDUP_MAX_ITEMS
                }
                None => false,
            };

            if !should_pop {
                break;
            }

            let Some((seen_at_ms, request_id)) = self.order.pop_front() else {
                break;
            };

            let remove = self
                .records
                .get(request_id.as_str())
                .map(|v| v.seen_at_ms == seen_at_ms)
                .unwrap_or(false);
            if remove {
                self.records.remove(request_id.as_str());
            }
        }
    }
}

pub struct KLogStateStoreManager {
    state_store: KLogStateStoreRef,

    // The kernel state: next id to assign to the next state entry
    next_log_id: AtomicU64,
    request_dedup: AsyncMutex<RequestDedupCache>,
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
            request_dedup: AsyncMutex::new(RequestDedupCache::default()),
        })
    }

    pub async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let request_id_pairs = entries
            .iter()
            .filter_map(|e| {
                normalize_request_id(e.request_id.as_deref())
                    .map(|request_id| (request_id.to_string(), e.id))
            })
            .collect::<Vec<_>>();
        let committed_next = entries
            .iter()
            .map(|e| e.id.saturating_add(1))
            .max()
            .unwrap_or(0);

        self.state_store.append(entries).await?;
        self.advance_next_log_id(committed_next).await?;
        self.remember_request_ids(request_id_pairs).await;
        Ok(())
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

    pub async fn find_recent_request_id(&self, request_id: &str) -> Option<u64> {
        let request_id = normalize_request_id(Some(request_id))?;
        let now_ms = now_millis();
        {
            let mut dedup = self.request_dedup.lock().await;
            if let Some(existing_id) = dedup.lookup(request_id, now_ms) {
                return Some(existing_id);
            }
        }

        match self
            .state_store
            .lookup_recent_request_id(request_id, now_ms)
            .await
        {
            Ok(Some(existing_id)) => {
                let mut dedup = self.request_dedup.lock().await;
                dedup.remember(request_id.to_string(), existing_id, now_ms);
                Some(existing_id)
            }
            Ok(None) => None,
            Err(err) => {
                warn!(
                    "KLogStateStoreManager lookup request_id in state_store failed: request_id={}, err={}",
                    request_id, err
                );
                None
            }
        }
    }

    /// Append an already prepared entry.
    /// This is used by state machine apply path to avoid re-assigning ids on followers.
    pub async fn append_prepared_entry(&self, item: KLogEntry) -> KResult<u64> {
        if let Some(request_id) = normalize_request_id(item.request_id.as_deref())
            && let Some(existing_id) = self.find_recent_request_id(request_id).await
        {
            info!(
                "KLogStateStoreManager dedup hit in append_prepared_entry: request_id={}, existing_id={}, incoming_id={}",
                request_id, existing_id, item.id
            );
            return Ok(existing_id);
        }

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

    pub async fn put_meta_entry(&self, item: KLogMetaEntry) -> KResult<KLogMetaEntry> {
        self.state_store.put_meta(item).await
    }

    pub async fn put_meta_entry_with_expected_revision(
        &self,
        item: KLogMetaEntry,
        expected_revision: Option<u64>,
    ) -> KResult<KLogMetaPutResult> {
        if let Some(expected_revision) = expected_revision {
            let current = self.state_store.get_meta(item.key.as_str()).await?;
            let current_revision = current.as_ref().map(|v| v.revision);
            let matched = if expected_revision == 0 {
                current.is_none()
            } else {
                current_revision == Some(expected_revision)
            };

            if !matched {
                return Ok(KLogMetaPutResult::VersionConflict {
                    expected_revision,
                    current_revision,
                });
            }
        }

        let stored = self.state_store.put_meta(item).await?;
        Ok(KLogMetaPutResult::Stored(stored))
    }

    pub async fn delete_meta_key(&self, key: &str) -> KResult<Option<KLogMetaEntry>> {
        self.state_store.delete_meta(key).await
    }

    pub async fn get_meta_entry(&self, key: &str) -> KResult<Option<KLogMetaEntry>> {
        self.state_store.get_meta(key).await
    }

    pub async fn list_meta_entries(
        &self,
        prefix: Option<&str>,
        limit: usize,
    ) -> KResult<Vec<KLogMetaEntry>> {
        self.state_store.list_meta(prefix, limit).await
    }

    pub async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()> {
        self.state_store.install_snapshot(snapshot).await?;
        let recovered_next = self.state_store.load_next_log_id().await?.unwrap_or(1);
        self.next_log_id.store(recovered_next, Ordering::SeqCst);
        self.request_dedup.lock().await.clear();
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

    async fn remember_request_ids(&self, request_ids: Vec<(String, u64)>) {
        if request_ids.is_empty() {
            return;
        }

        let now_ms = now_millis();
        let mut dedup = self.request_dedup.lock().await;
        for (request_id, log_id) in request_ids {
            dedup.remember(request_id, log_id, now_ms);
        }
    }
}

pub type KLogStateStoreManagerRef = Arc<KLogStateStoreManager>;

fn normalize_request_id(request_id: Option<&str>) -> Option<&str> {
    request_id.map(|v| v.trim()).filter(|v| !v.is_empty())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
