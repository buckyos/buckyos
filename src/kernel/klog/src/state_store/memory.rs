use super::store::{
    KLogQuery, KLogQueryOrder, KLogStateMachineMeta, KLogStateSnapshot, KLogStateSnapshotData,
    KLogStateStore, REQUEST_DEDUP_MAX_ITEMS, REQUEST_DEDUP_WINDOW_MS,
};
use crate::{KLogEntry, KLogError, KLogMetaEntry, KResult};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Debug, Clone, Copy)]
struct RequestDedupRecord {
    log_id: u64,
    seen_at_ms: u64,
}

#[derive(Debug, Default)]
struct RequestDedupIndex {
    records: HashMap<String, RequestDedupRecord>,
    order: VecDeque<(u64, String)>,
}

impl RequestDedupIndex {
    fn lookup(&mut self, request_id: &str, now_ms: u64) -> Option<u64> {
        self.cleanup(now_ms);
        self.records.get(request_id).map(|v| v.log_id)
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

/// A simple in-memory state store implementation.
pub struct MemoryStateStore {
    logs: Arc<AsyncMutex<Vec<KLogEntry>>>,
    metas: Arc<AsyncMutex<HashMap<String, KLogMetaEntry>>>,
    next_log_id: AtomicU64,
    state_machine_meta: Arc<AsyncMutex<Option<KLogStateMachineMeta>>>,
    request_dedup: Arc<AsyncMutex<RequestDedupIndex>>,
}

impl MemoryStateStore {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(AsyncMutex::new(Vec::new())),
            metas: Arc::new(AsyncMutex::new(HashMap::new())),
            next_log_id: AtomicU64::new(1),
            state_machine_meta: Arc::new(AsyncMutex::new(None)),
            request_dedup: Arc::new(AsyncMutex::new(RequestDedupIndex::default())),
        }
    }
}

#[async_trait::async_trait]
impl KLogStateStore for MemoryStateStore {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let now_ms = now_millis();
        let request_id_pairs = entries
            .iter()
            .filter_map(|entry| {
                normalize_request_id(entry.request_id.as_deref())
                    .map(|request_id| (request_id.to_string(), entry.id))
            })
            .collect::<Vec<_>>();
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

        if !request_id_pairs.is_empty() {
            let mut dedup = self.request_dedup.lock().await;
            for (request_id, log_id) in request_id_pairs {
                dedup.remember(request_id, log_id, now_ms);
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

    async fn put_meta(&self, item: KLogMetaEntry) -> KResult<()> {
        let mut metas = self.metas.lock().await;
        metas.insert(item.key.clone(), item);
        Ok(())
    }

    async fn delete_meta(&self, key: &str) -> KResult<bool> {
        let mut metas = self.metas.lock().await;
        Ok(metas.remove(key).is_some())
    }

    async fn get_meta(&self, key: &str) -> KResult<Option<KLogMetaEntry>> {
        let metas = self.metas.lock().await;
        Ok(metas.get(key).cloned())
    }

    async fn list_meta(&self, prefix: Option<&str>, limit: usize) -> KResult<Vec<KLogMetaEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let metas = self.metas.lock().await;
        let mut keys = metas.keys().cloned().collect::<Vec<_>>();
        keys.sort();

        let normalized_prefix = prefix.map(str::trim).filter(|v| !v.is_empty());
        let mut out = Vec::with_capacity(limit.min(keys.len()));
        for key in keys {
            if let Some(prefix) = normalized_prefix
                && !key.starts_with(prefix)
            {
                continue;
            }

            if let Some(item) = metas.get(&key) {
                out.push(item.clone());
                if out.len() >= limit {
                    break;
                }
            }
        }

        Ok(out)
    }

    async fn build_snapshot(&self) -> KResult<KLogStateSnapshot> {
        let logs = self.logs.lock().await;
        let metas = self.metas.lock().await;
        let mut meta_entries = metas.values().cloned().collect::<Vec<_>>();
        meta_entries.sort_by(|a, b| a.key.cmp(&b.key));
        let snapshot_data = KLogStateSnapshotData {
            entries: logs.clone(),
            meta_entries,
        };
        let data = bincode::serde::encode_to_vec(&snapshot_data, bincode::config::legacy())
            .map_err(|e| {
                let msg = format!("Failed to serialize logs for snapshot: {}", e);
                error!("{}", msg);
                KLogError::InvalidFormat(msg)
            })?;

        Ok(KLogStateSnapshot { data })
    }

    async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()> {
        let snapshot_data = decode_snapshot_data(&snapshot.data)?;
        let entries = snapshot_data.entries;
        let mut metas = HashMap::new();
        for item in snapshot_data.meta_entries {
            metas.insert(item.key.clone(), item);
        }

        let candidate_next = entries
            .iter()
            .map(|e| e.id.saturating_add(1))
            .max()
            .unwrap_or(1);
        let mut logs = self.logs.lock().await;
        *logs = entries;
        let mut stored_metas = self.metas.lock().await;
        *stored_metas = metas;
        self.next_log_id.store(candidate_next, Ordering::SeqCst);
        let mut dedup = self.request_dedup.lock().await;
        dedup.clear();
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

    async fn lookup_recent_request_id(
        &self,
        request_id: &str,
        now_ms: u64,
    ) -> KResult<Option<u64>> {
        let Some(request_id) = normalize_request_id(Some(request_id)) else {
            return Ok(None);
        };
        let mut dedup = self.request_dedup.lock().await;
        Ok(dedup.lookup(request_id, now_ms))
    }
}

fn normalize_request_id(request_id: Option<&str>) -> Option<&str> {
    request_id.map(|v| v.trim()).filter(|v| !v.is_empty())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn decode_snapshot_data(data: &[u8]) -> KResult<KLogStateSnapshotData> {
    let decoded_new: Result<(KLogStateSnapshotData, usize), _> =
        bincode::serde::decode_from_slice(data, bincode::config::legacy());
    if let Ok((snapshot_data, _)) = decoded_new {
        return Ok(snapshot_data);
    }

    // Temporary fallback for old test snapshots generated before meta support.
    let (entries, _): (Vec<KLogEntry>, usize) =
        bincode::serde::decode_from_slice(data, bincode::config::legacy()).map_err(|e| {
            let msg = format!("Failed to decode state snapshot: {}", e);
            error!("{}", msg);
            KLogError::InvalidFormat(msg)
        })?;
    Ok(KLogStateSnapshotData {
        entries,
        meta_entries: Vec::new(),
    })
}
