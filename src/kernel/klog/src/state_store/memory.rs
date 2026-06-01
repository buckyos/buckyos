use super::store::{
    KLogMetaChangeQuery, KLogMetaHistoryRecord, KLogMetaKeyState, KLogMetaTxResult, KLogQuery,
    KLogQueryOrder, KLogStateMachineMeta, KLogStateSnapshot, KLogStateSnapshotData, KLogStateStore,
    REQUEST_DEDUP_MAX_ITEMS, REQUEST_DEDUP_WINDOW_MS,
};
use crate::{
    KLogEntry, KLogError, KLogMetaEntry, KLogMetaTxAction, KLogMetaTxRequest, KLogMetaTxResponse,
    KLogMetaVersion, KResult,
};
use std::collections::{BTreeMap, HashMap, VecDeque};
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
    metas: Arc<AsyncMutex<MemoryMetaState>>,
    next_log_id: AtomicU64,
    state_machine_meta: Arc<AsyncMutex<Option<KLogStateMachineMeta>>>,
    request_dedup: Arc<AsyncMutex<RequestDedupIndex>>,
}

#[derive(Debug, Default)]
struct MemoryMetaState {
    entries: HashMap<String, KLogMetaEntry>,
    states: HashMap<String, KLogMetaKeyState>,
    history: BTreeMap<(String, u64), KLogMetaHistoryRecord>,
    changes: BTreeMap<(u64, String), KLogMetaHistoryRecord>,
    revision: u64,
}

impl MemoryMetaState {
    fn next_revision(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1).max(1);
        self.revision
    }

    fn current_revision(&self, key: &str) -> Option<u64> {
        self.states
            .get(key)
            .map(|state| state.mod_revision)
            .or_else(|| {
                self.entries
                    .get(key)
                    .map(KLogMetaEntry::effective_mod_revision)
            })
    }

    fn state_for_entry(&self, item: &KLogMetaEntry) -> KLogMetaKeyState {
        self.states
            .get(&item.key)
            .cloned()
            .unwrap_or_else(|| legacy_meta_state_from_entry(item, false))
    }

    fn insert_history(&mut self, record: KLogMetaHistoryRecord) {
        self.revision = self.revision.max(record.mod_revision);
        let history_key = (record.key.clone(), record.mod_revision);
        let change_key = (record.mod_revision, record.key.clone());
        self.history.insert(history_key, record.clone());
        self.changes.insert(change_key, record);
    }

    fn get_history_at_revision(&self, key: &str, revision: u64) -> Option<KLogMetaHistoryRecord> {
        self.history
            .values()
            .filter(|record| record.key == key && record.mod_revision <= revision)
            .max_by_key(|record| record.mod_revision)
            .cloned()
    }

    fn list_history_at_revision(
        &self,
        prefix: Option<&str>,
        cursor: Option<&str>,
        revision: u64,
    ) -> BTreeMap<String, KLogMetaHistoryRecord> {
        let mut latest = BTreeMap::new();
        for record in self.history.values() {
            if record.mod_revision > revision {
                continue;
            }
            if let Some(prefix) = prefix
                && !record.key.starts_with(prefix)
            {
                continue;
            }
            if let Some(cursor) = cursor
                && record.key.as_str() <= cursor
            {
                continue;
            }
            latest.insert(record.key.clone(), record.clone());
        }
        latest
    }

    fn list_changes(&self, query: &KLogMetaChangeQuery) -> Vec<KLogMetaHistoryRecord> {
        if query.limit == 0 {
            return Vec::new();
        }

        let key_filter = query
            .key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let prefix_filter = query
            .prefix
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let mut out = Vec::with_capacity(query.limit.min(self.changes.len()));
        for record in self.changes.values() {
            if record.mod_revision < query.start_revision {
                continue;
            }
            if let Some(end_revision) = query.end_revision
                && record.mod_revision > end_revision
            {
                break;
            }
            if let Some(cursor) = query.cursor.as_ref()
                && (record.mod_revision < cursor.revision
                    || (record.mod_revision == cursor.revision
                        && record.key.as_str() <= cursor.key.as_str()))
            {
                continue;
            }
            if let Some(key) = key_filter
                && record.key != key
            {
                continue;
            }
            if let Some(prefix) = prefix_filter
                && !record.key.starts_with(prefix)
            {
                continue;
            }
            if !query.include_deleted && record.deleted {
                continue;
            }

            out.push(record.clone());
            if out.len() >= query.limit {
                break;
            }
        }
        out
    }
}

impl MemoryStateStore {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(AsyncMutex::new(Vec::new())),
            metas: Arc::new(AsyncMutex::new(MemoryMetaState::default())),
            next_log_id: AtomicU64::new(1),
            state_machine_meta: Arc::new(AsyncMutex::new(None)),
            request_dedup: Arc::new(AsyncMutex::new(RequestDedupIndex::default())),
        }
    }
}

impl Default for MemoryStateStore {
    fn default() -> Self {
        Self::new()
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
        let source_filter = query
            .source
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let attr_key_filter = query
            .attr_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let attr_value_filter = query
            .attr_value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if attr_key_filter.is_none() && attr_value_filter.is_some() {
            return Ok(Vec::new());
        }
        let mut entries = logs
            .iter()
            .filter(|e| {
                query.start_id.map(|start| e.id >= start).unwrap_or(true)
                    && query.end_id.map(|end| e.id <= end).unwrap_or(true)
                    && query.level.map(|level| e.level == level).unwrap_or(true)
                    && source_filter
                        .map(|source| {
                            e.source
                                .as_deref()
                                .map(str::trim)
                                .filter(|v| !v.is_empty())
                                .map(|v| v == source)
                                .unwrap_or(false)
                        })
                        .unwrap_or(true)
                    && attr_key_filter
                        .map(|key| {
                            e.attrs.get(key).is_some_and(|value| {
                                attr_value_filter
                                    .map(|expect| value == expect)
                                    .unwrap_or(true)
                            })
                        })
                        .unwrap_or(true)
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

    async fn put_meta(&self, item: KLogMetaEntry) -> KResult<KLogMetaEntry> {
        let mut metas = self.metas.lock().await;
        let key = item.key.clone();
        let next_revision = metas.next_revision();
        let mut stored = item;
        let state = if let Some(existing) = metas.entries.get(&key) {
            let previous = metas.state_for_entry(existing);
            KLogMetaKeyState {
                key: key.clone(),
                create_revision: previous.create_revision,
                mod_revision: next_revision,
                version: previous.version.saturating_add(1).max(1),
                deleted: false,
            }
        } else {
            KLogMetaKeyState {
                key: key.clone(),
                create_revision: next_revision,
                mod_revision: next_revision,
                version: 1,
                deleted: false,
            }
        };
        state.apply_to_entry(&mut stored);
        metas.insert_history(KLogMetaHistoryRecord::from_entry(&stored, false));
        metas.states.insert(key.clone(), state);
        metas.entries.insert(key, stored.clone());
        Ok(stored)
    }

    async fn exec_meta_tx(&self, tx: KLogMetaTxRequest) -> KResult<KLogMetaTxResult> {
        let mut metas = self.metas.lock().await;

        if let Some(guard) = tx.guard.as_ref() {
            let current_revision = metas.current_revision(&guard.key);
            let actual_revision = current_revision.unwrap_or(0);
            if actual_revision != guard.expected_revision {
                return Ok(KLogMetaTxResult::VersionConflict {
                    key: guard.key.clone(),
                    expected_revision: guard.expected_revision,
                    current_revision,
                });
            }
        }

        for (action_key, action) in tx.actions.iter() {
            if action.key() != action_key {
                let msg = format!(
                    "meta tx action key mismatch: map_key={}, action_key={}",
                    action_key,
                    action.key()
                );
                error!("{}", msg);
                return Err(KLogError::InvalidFormat(msg));
            }

            let Some(expected_revision) = action.expected_revision() else {
                continue;
            };
            let current_revision = metas.current_revision(action_key);
            let matched = if expected_revision == 0 {
                !metas.entries.contains_key(action_key)
            } else {
                current_revision == Some(expected_revision)
            };
            if !matched {
                return Ok(KLogMetaTxResult::VersionConflict {
                    key: action_key.clone(),
                    expected_revision,
                    current_revision,
                });
            }
        }

        let guard_key = tx.guard.as_ref().map(|guard| guard.key.clone());
        let mut guard_touched = false;
        let mut revisions = BTreeMap::new();
        let mut meta_versions = BTreeMap::new();
        let mut has_mutation = false;
        for action in tx.actions.values() {
            match action {
                KLogMetaTxAction::Put { .. } => {
                    has_mutation = true;
                    break;
                }
                KLogMetaTxAction::Delete { key, .. } => {
                    if metas.entries.contains_key(key) {
                        has_mutation = true;
                        break;
                    }
                }
            }
        }
        if !has_mutation
            && let Some(guard_key) = guard_key.as_deref()
            && metas.entries.contains_key(guard_key)
            && !tx.actions.values().any(|action| action.key() == guard_key)
        {
            has_mutation = true;
        }
        let tx_revision = if has_mutation {
            Some(metas.next_revision())
        } else {
            None
        };

        for (action_key, action) in tx.actions.into_iter() {
            if guard_key.as_deref() == Some(action_key.as_str()) {
                guard_touched = true;
            }

            match action {
                KLogMetaTxAction::Put { mut item, .. } => {
                    let next_revision = tx_revision.expect("put action must allocate tx revision");
                    let state = if let Some(existing) = metas.entries.get(&item.key) {
                        let previous = metas.state_for_entry(existing);
                        KLogMetaKeyState {
                            key: item.key.clone(),
                            create_revision: previous.create_revision,
                            mod_revision: next_revision,
                            version: previous.version.saturating_add(1).max(1),
                            deleted: false,
                        }
                    } else {
                        KLogMetaKeyState {
                            key: item.key.clone(),
                            create_revision: next_revision,
                            mod_revision: next_revision,
                            version: 1,
                            deleted: false,
                        }
                    };
                    state.apply_to_entry(&mut item);
                    revisions.insert(item.key.clone(), Some(item.effective_mod_revision()));
                    meta_versions.insert(item.key.clone(), KLogMetaVersion::from_entry(&item));
                    metas.insert_history(KLogMetaHistoryRecord::from_entry(&item, false));
                    metas.states.insert(item.key.clone(), state);
                    metas.entries.insert(item.key.clone(), item);
                }
                KLogMetaTxAction::Delete { key, .. } => {
                    if let Some(prev) = metas.entries.remove(&key) {
                        let next_revision =
                            tx_revision.expect("existing delete must allocate tx revision");
                        let previous = metas.state_for_entry(&prev);
                        let state = KLogMetaKeyState {
                            key: key.clone(),
                            create_revision: previous.create_revision,
                            mod_revision: next_revision,
                            version: 0,
                            deleted: true,
                        };
                        metas.insert_history(KLogMetaHistoryRecord::from_tombstone(&prev, &state));
                        metas.states.insert(key.clone(), state);
                        meta_versions.insert(
                            key.clone(),
                            KLogMetaVersion::new(previous.create_revision, next_revision, 0, true),
                        );
                    }
                    revisions.insert(key, None);
                }
            }
        }

        if let Some(guard) = tx.guard
            && !guard_touched
            && let Some(mut item) = metas.entries.remove(&guard.key)
        {
            let next_revision = tx_revision.expect("guard side effect must allocate tx revision");
            let previous = metas.state_for_entry(&item);
            let state = KLogMetaKeyState {
                key: guard.key.clone(),
                create_revision: previous.create_revision,
                mod_revision: next_revision,
                version: previous.version.saturating_add(1).max(1),
                deleted: false,
            };
            state.apply_to_entry(&mut item);
            metas.states.insert(guard.key.clone(), state);
            revisions.insert(guard.key.clone(), Some(item.effective_mod_revision()));
            meta_versions.insert(guard.key, KLogMetaVersion::from_entry(&item));
            metas.insert_history(KLogMetaHistoryRecord::from_entry(&item, false));
            metas.entries.insert(item.key.clone(), item);
        }

        Ok(KLogMetaTxResult::Committed(KLogMetaTxResponse {
            revisions,
            meta_versions,
        }))
    }

    async fn delete_meta(&self, key: &str) -> KResult<Option<KLogMetaEntry>> {
        let mut metas = self.metas.lock().await;
        let Some(prev) = metas.entries.remove(key) else {
            return Ok(None);
        };
        let next_revision = metas.next_revision();
        let previous = metas.state_for_entry(&prev);
        let state = KLogMetaKeyState {
            key: key.to_string(),
            create_revision: previous.create_revision,
            mod_revision: next_revision,
            version: 0,
            deleted: true,
        };
        metas.insert_history(KLogMetaHistoryRecord::from_tombstone(&prev, &state));
        metas.states.insert(key.to_string(), state);
        Ok(Some(prev))
    }

    async fn get_meta(&self, key: &str) -> KResult<Option<KLogMetaEntry>> {
        let metas = self.metas.lock().await;
        Ok(metas.entries.get(key).cloned())
    }

    async fn current_meta_revision(&self, key: &str) -> KResult<Option<u64>> {
        let metas = self.metas.lock().await;
        Ok(metas.current_revision(key))
    }

    async fn get_meta_at_revision(
        &self,
        key: &str,
        revision: u64,
    ) -> KResult<Option<KLogMetaEntry>> {
        let metas = self.metas.lock().await;
        Ok(metas
            .get_history_at_revision(key, revision)
            .and_then(|record| record.to_live_entry()))
    }

    async fn list_meta(
        &self,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> KResult<Vec<KLogMetaEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let metas = self.metas.lock().await;
        let mut keys = metas.entries.keys().cloned().collect::<Vec<_>>();
        keys.sort();

        let normalized_prefix = prefix.map(str::trim).filter(|v| !v.is_empty());
        let normalized_cursor = cursor.map(str::trim).filter(|v| !v.is_empty());
        let mut out = Vec::with_capacity(limit.min(keys.len()));
        for key in keys {
            if let Some(prefix) = normalized_prefix
                && !key.starts_with(prefix)
            {
                continue;
            }
            if let Some(cursor) = normalized_cursor
                && key.as_str() <= cursor
            {
                continue;
            }

            if let Some(item) = metas.entries.get(&key) {
                out.push(item.clone());
                if out.len() >= limit {
                    break;
                }
            }
        }

        Ok(out)
    }

    async fn list_meta_at_revision(
        &self,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
        revision: u64,
    ) -> KResult<Vec<KLogMetaEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let normalized_prefix = prefix.map(str::trim).filter(|v| !v.is_empty());
        let normalized_cursor = cursor.map(str::trim).filter(|v| !v.is_empty());
        let metas = self.metas.lock().await;
        let latest = metas.list_history_at_revision(normalized_prefix, normalized_cursor, revision);
        let mut out = Vec::with_capacity(limit.min(latest.len()));
        for record in latest.values() {
            if let Some(item) = record.to_live_entry() {
                out.push(item);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    async fn list_meta_changes(
        &self,
        query: KLogMetaChangeQuery,
    ) -> KResult<Vec<KLogMetaHistoryRecord>> {
        if query.key.as_ref().is_some_and(|v| !v.trim().is_empty())
            && query.prefix.as_ref().is_some_and(|v| !v.trim().is_empty())
        {
            let msg = "meta change query can not set key and prefix together";
            error!("{}", msg);
            return Err(KLogError::InvalidFormat(msg.to_string()));
        }

        let metas = self.metas.lock().await;
        Ok(metas.list_changes(&query))
    }

    async fn build_snapshot(&self) -> KResult<KLogStateSnapshot> {
        let logs = self.logs.lock().await;
        let metas = self.metas.lock().await;
        let mut meta_entries = metas.entries.values().cloned().collect::<Vec<_>>();
        meta_entries.sort_by(|a, b| a.key.cmp(&b.key));
        let mut meta_states = metas.states.values().cloned().collect::<Vec<_>>();
        meta_states.sort_by(|a, b| a.key.cmp(&b.key));
        let meta_history = metas.history.values().cloned().collect::<Vec<_>>();
        let snapshot_data = KLogStateSnapshotData {
            entries: logs.clone(),
            meta_entries,
            meta_states,
            meta_history,
            meta_revision: metas.revision,
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
        let meta_history = snapshot_data.meta_history;
        let entries = snapshot_data.entries;
        let mut metas = MemoryMetaState {
            entries: HashMap::new(),
            states: snapshot_data
                .meta_states
                .into_iter()
                .map(|state| (state.key.clone(), state))
                .collect(),
            history: BTreeMap::new(),
            changes: BTreeMap::new(),
            revision: snapshot_data.meta_revision,
        };
        for item in snapshot_data.meta_entries {
            metas.revision = metas.revision.max(item.effective_mod_revision());
            metas
                .states
                .entry(item.key.clone())
                .or_insert_with(|| legacy_meta_state_from_entry(&item, false));
            let mut item = item;
            let state = metas.state_for_entry(&item);
            state.apply_to_entry(&mut item);
            metas.entries.insert(item.key.clone(), item);
        }
        if meta_history.is_empty() {
            let entries = metas.entries.values().cloned().collect::<Vec<_>>();
            for item in entries {
                metas.insert_history(KLogMetaHistoryRecord::from_entry(&item, false));
            }
        } else {
            for record in meta_history {
                metas.insert_history(record);
            }
        }
        for state in metas.states.values() {
            metas.revision = metas.revision.max(state.mod_revision);
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
        meta_states: Vec::new(),
        meta_history: Vec::new(),
        meta_revision: 0,
    })
}

fn legacy_meta_state_from_entry(item: &KLogMetaEntry, deleted: bool) -> KLogMetaKeyState {
    KLogMetaKeyState {
        key: item.key.clone(),
        create_revision: item.effective_create_revision(),
        mod_revision: item.effective_mod_revision(),
        version: if deleted { 0 } else { item.effective_version() },
        deleted,
    }
}
