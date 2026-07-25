use async_trait::async_trait;
use buckyos_api::{
    is_global_eventid, is_global_pattern, match_event_patterns, normalize_patterns,
    validate_event_data_size, validate_eventid, validate_pattern, Event, KEventDaemonRequest,
    KEventDaemonResponse, KEventError, KEventResult, SharedKEventRingBuffer,
};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::time::{timeout, Instant};

pub const DEFAULT_DAEMON_READER_CAPACITY: usize = 1024;

#[async_trait]
pub trait KEventPeerPublisher: Send + Sync {
    async fn broadcast(&self, event: &Event) -> KEventResult<()>;
}

/// Namespace a reader belongs to.
///
/// Readers registered over a native TCP connection live in that connection's
/// own namespace: `reader_id` only has to be unique *within* the connection,
/// and every reader of a connection is dropped when the connection ends.
/// This is what makes reader ids collision-free across independent client
/// processes (each container registers its own `r_1`) and what stops a
/// crashed client from leaving a reader queue behind forever.
///
/// Stateless callers (the HTTP facade) share [`KEventSessionId::SHARED`] and
/// must therefore pick process-unique reader ids themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KEventSessionId(u64);

impl KEventSessionId {
    /// Namespace used by connection-less callers (HTTP facade, in-process
    /// service API, tests).
    pub const SHARED: KEventSessionId = KEventSessionId(0);

    pub fn value(&self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for KEventSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "s{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReaderKey {
    session: KEventSessionId,
    reader_id: String,
}

impl ReaderKey {
    fn new(session: KEventSessionId, reader_id: &str) -> Self {
        Self {
            session,
            reader_id: reader_id.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct KEventService {
    source_node: String,
    reader_capacity: usize,
    readers: Arc<RwLock<HashMap<ReaderKey, Arc<ServiceReaderState>>>>,
    peers: Arc<RwLock<Vec<Arc<dyn KEventPeerPublisher>>>>,
    shared_ring: Arc<RwLock<Option<Arc<SharedKEventRingBuffer>>>>,
    session_seq: Arc<AtomicU64>,
}

struct ServiceReaderState {
    patterns: StdRwLock<Vec<String>>,
    queue: Mutex<VecDeque<Event>>,
    notify: Notify,
    capacity: usize,
}

impl ServiceReaderState {
    fn new(patterns: Vec<String>, capacity: usize) -> Self {
        Self {
            patterns: StdRwLock::new(patterns),
            queue: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            capacity,
        }
    }

    fn matches(&self, eventid: &str) -> bool {
        let patterns = self.patterns.read().expect("patterns lock poisoned");
        match_event_patterns(&patterns, eventid)
    }

    async fn push(&self, event: Event) {
        let mut queue = self.queue.lock().await;
        if queue.len() >= self.capacity {
            queue.pop_front();
        }
        queue.push_back(event);
        drop(queue);
        self.notify.notify_one();
    }

    async fn pop(&self) -> Option<Event> {
        let mut queue = self.queue.lock().await;
        queue.pop_front()
    }
}

impl KEventService {
    pub fn new(source_node: impl Into<String>) -> Self {
        Self::new_with_capacity(source_node, DEFAULT_DAEMON_READER_CAPACITY)
    }

    pub fn new_with_capacity(source_node: impl Into<String>, reader_capacity: usize) -> Self {
        Self {
            source_node: source_node.into(),
            reader_capacity: reader_capacity.max(1),
            readers: Arc::new(RwLock::new(HashMap::new())),
            peers: Arc::new(RwLock::new(Vec::new())),
            shared_ring: Arc::new(RwLock::new(None)),
            session_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn source_node(&self) -> &str {
        &self.source_node
    }

    /// Allocate a reader namespace for one connection. The caller must pair
    /// this with [`close_session`] on every exit path so readers left behind
    /// by a disconnecting client are reclaimed.
    pub fn open_session(&self) -> KEventSessionId {
        KEventSessionId(self.session_seq.fetch_add(1, Ordering::Relaxed) + 1)
    }

    /// Drop every reader registered in `session`. Returns how many were
    /// reclaimed.
    pub async fn close_session(&self, session: KEventSessionId) -> usize {
        if session == KEventSessionId::SHARED {
            return 0;
        }
        let mut readers = self.readers.write().await;
        let before = readers.len();
        readers.retain(|key, _| key.session != session);
        before - readers.len()
    }

    /// Number of live readers; exposed for observability and tests.
    pub async fn reader_count(&self) -> usize {
        self.readers.read().await.len()
    }

    pub async fn add_peer_publisher(&self, peer: Arc<dyn KEventPeerPublisher>) {
        self.peers.write().await.push(peer);
    }

    pub async fn set_shared_ring(&self, shared_ring: Arc<SharedKEventRingBuffer>) {
        *self.shared_ring.write().await = Some(shared_ring);
    }

    pub async fn register_reader(
        &self,
        reader_id: &str,
        patterns: Vec<String>,
    ) -> KEventResult<()> {
        self.register_reader_in(KEventSessionId::SHARED, reader_id, patterns)
            .await
    }

    /// Idempotent full-set registration: an existing reader keeps its queue
    /// and only swaps its pattern set. Clients always resend the complete
    /// pattern set (never a diff), so this doubles as the reconnect path.
    pub async fn register_reader_in(
        &self,
        session: KEventSessionId,
        reader_id: &str,
        patterns: Vec<String>,
    ) -> KEventResult<()> {
        if reader_id.is_empty() {
            return Err(KEventError::InvalidPattern(
                "reader_id must not be empty".to_string(),
            ));
        }
        if patterns.is_empty() {
            return Err(KEventError::InvalidPattern(
                "patterns must not be empty".to_string(),
            ));
        }
        for pattern in &patterns {
            validate_pattern(pattern)?;
            if !is_global_pattern(pattern) {
                return Err(KEventError::InvalidPattern(
                    "daemon only supports global patterns".to_string(),
                ));
            }
        }

        let normalized = normalize_patterns(patterns);
        let key = ReaderKey::new(session, reader_id);
        let mut readers = self.readers.write().await;
        if let Some(existing) = readers.get(&key) {
            // Preserve queue / notify across re-register; just swap patterns.
            *existing.patterns.write().expect("patterns lock poisoned") = normalized;
        } else {
            readers.insert(
                key,
                Arc::new(ServiceReaderState::new(normalized, self.reader_capacity)),
            );
        }
        Ok(())
    }

    pub async fn unregister_reader(&self, reader_id: &str) {
        self.unregister_reader_in(KEventSessionId::SHARED, reader_id)
            .await
    }

    pub async fn unregister_reader_in(&self, session: KEventSessionId, reader_id: &str) {
        self.readers
            .write()
            .await
            .remove(&ReaderKey::new(session, reader_id));
    }

    /// Incremental pattern edit. Kept for protocol compatibility only —
    /// clients recover by resending the full set via `register_reader`.
    pub async fn update_reader(
        &self,
        reader_id: &str,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> KEventResult<()> {
        self.update_reader_in(KEventSessionId::SHARED, reader_id, add, remove)
            .await
    }

    pub async fn update_reader_in(
        &self,
        session: KEventSessionId,
        reader_id: &str,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> KEventResult<()> {
        if reader_id.is_empty() {
            return Err(KEventError::InvalidPattern(
                "reader_id must not be empty".to_string(),
            ));
        }
        for pattern in add.iter().chain(remove.iter()) {
            validate_pattern(pattern)?;
            if !is_global_pattern(pattern) {
                return Err(KEventError::InvalidPattern(
                    "daemon only supports global patterns".to_string(),
                ));
            }
        }

        let reader = {
            let readers = self.readers.read().await;
            readers.get(&ReaderKey::new(session, reader_id)).cloned()
        };
        let Some(reader) = reader else {
            return Err(KEventError::ReaderClosed(reader_id.to_string()));
        };

        let mut patterns = reader.patterns.write().expect("patterns lock poisoned");
        let mut next: Vec<String> = patterns
            .iter()
            .filter(|p| !remove.iter().any(|r| r == *p))
            .cloned()
            .collect();
        if !add.is_empty() {
            next.extend(add);
            next = normalize_patterns(next);
        }
        if next.is_empty() {
            return Err(KEventError::InvalidPattern(
                "reader must keep at least one pattern".to_string(),
            ));
        }
        *patterns = next;
        Ok(())
    }

    pub async fn publish_local_global(&self, eventid: &str, data: Value) -> KEventResult<()> {
        if !is_global_eventid(eventid) {
            return Err(KEventError::InvalidEventId(
                "daemon only accepts global eventid".to_string(),
            ));
        }
        validate_eventid(eventid)?;
        validate_event_data_size(&data)?;

        let event = Event {
            eventid: eventid.to_string(),
            source_node: self.source_node.clone(),
            source_pid: std::process::id(),
            ingress_node: Some(self.source_node.clone()),
            timestamp: now_millis(),
            data,
        };
        // Mirror to shared ring so other local processes (full-mode SDK
        // readers that mmap the region) observe daemon-originated events
        // via the same fast path as peer/http-originated events.
        self.mirror_to_shared_ring(&event).await?;
        self.distribute(&event).await;
        if should_broadcast_to_peers(&event, &self.source_node) {
            self.broadcast_to_peers(&event).await
        } else {
            Ok(())
        }
    }

    pub async fn publish_http_global(&self, eventid: &str, data: Value) -> KEventResult<()> {
        let event = Event {
            eventid: eventid.to_string(),
            source_node: self.source_node.clone(),
            source_pid: std::process::id(),
            ingress_node: Some(self.source_node.clone()),
            timestamp: now_millis(),
            data,
        };
        self.accept_external_global(event).await
    }

    pub async fn publish_external_global(&self, mut event: Event) -> KEventResult<()> {
        if !is_global_eventid(&event.eventid) {
            return Err(KEventError::InvalidEventId(
                "daemon only accepts global eventid".to_string(),
            ));
        }
        validate_eventid(&event.eventid)?;
        validate_event_data_size(&event.data)?;
        if event.ingress_node.is_none() {
            event.ingress_node = Some(self.source_node.clone());
        }
        self.distribute(&event).await;
        if should_broadcast_to_peers(&event, &self.source_node) {
            self.broadcast_to_peers(&event).await
        } else {
            Ok(())
        }
    }

    pub async fn accept_external_global(&self, mut event: Event) -> KEventResult<()> {
        if !is_global_eventid(&event.eventid) {
            return Err(KEventError::InvalidEventId(
                "daemon only accepts global eventid".to_string(),
            ));
        }
        validate_eventid(&event.eventid)?;
        validate_event_data_size(&event.data)?;
        event.ingress_node = Some(self.source_node.clone());
        self.mirror_to_shared_ring(&event).await?;
        self.distribute(&event).await;
        if should_broadcast_to_peers(&event, &self.source_node) {
            self.broadcast_to_peers(&event).await
        } else {
            Ok(())
        }
    }

    pub async fn publish_from_peer(&self, mut event: Event) -> KEventResult<()> {
        if !is_global_eventid(&event.eventid) {
            return Err(KEventError::InvalidEventId(
                "peer event must be global eventid".to_string(),
            ));
        }
        validate_eventid(&event.eventid)?;
        validate_event_data_size(&event.data)?;
        if event.ingress_node.is_none() {
            event.ingress_node = Some(event.source_node.clone());
        }
        self.mirror_to_shared_ring(&event).await?;
        self.distribute(&event).await;
        Ok(())
    }

    pub async fn pull_event(
        &self,
        reader_id: &str,
        timeout_ms: Option<u64>,
    ) -> KEventResult<Option<Event>> {
        self.pull_event_in(KEventSessionId::SHARED, reader_id, timeout_ms)
            .await
    }

    /// `Ok(None)` means "no event within the timeout"; a reader the daemon
    /// does not know about is reported as `ReaderClosed` so a client can tell
    /// "lost my registration" apart from "nothing happened" and re-register
    /// instead of silently pulling into the void.
    pub async fn pull_event_in(
        &self,
        session: KEventSessionId,
        reader_id: &str,
        timeout_ms: Option<u64>,
    ) -> KEventResult<Option<Event>> {
        let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
        let key = ReaderKey::new(session, reader_id);
        loop {
            let reader = {
                let readers = self.readers.read().await;
                readers.get(&key).cloned()
            };

            let Some(reader) = reader else {
                return Err(KEventError::ReaderClosed(reader_id.to_string()));
            };

            if let Some(event) = reader.pop().await {
                return Ok(Some(event));
            }

            if timeout_ms == Some(0) {
                return Ok(None);
            }

            match deadline {
                None => {
                    reader.notify.notified().await;
                }
                Some(deadline_at) => {
                    let now = Instant::now();
                    if now >= deadline_at {
                        return Ok(None);
                    }
                    let remain = deadline_at - now;
                    if timeout(remain, reader.notify.notified()).await.is_err() {
                        return Ok(None);
                    }
                }
            }
        }
    }

    pub async fn handle_protocol_request(&self, req: KEventDaemonRequest) -> KEventDaemonResponse {
        self.handle_protocol_request_in(KEventSessionId::SHARED, req)
            .await
    }

    /// Serve one protocol request inside `session`'s reader namespace.
    /// Connection-oriented transports pass their own session so reader ids
    /// cannot collide across clients and die with the connection.
    pub async fn handle_protocol_request_in(
        &self,
        session: KEventSessionId,
        req: KEventDaemonRequest,
    ) -> KEventDaemonResponse {
        match req {
            KEventDaemonRequest::RegisterReader {
                reader_id,
                patterns,
            } => match self
                .register_reader_in(session, &reader_id, patterns)
                .await
            {
                Ok(_) => KEventDaemonResponse::Ok { event: None },
                Err(err) => err_to_response(err),
            },
            KEventDaemonRequest::UnregisterReader { reader_id } => {
                self.unregister_reader_in(session, &reader_id).await;
                KEventDaemonResponse::Ok { event: None }
            }
            KEventDaemonRequest::UpdateReader {
                reader_id,
                add,
                remove,
            } => match self.update_reader_in(session, &reader_id, add, remove).await {
                Ok(_) => KEventDaemonResponse::Ok { event: None },
                Err(err) => err_to_response(err),
            },
            KEventDaemonRequest::PublishGlobal { event } => {
                match self.accept_external_global(event).await {
                    Ok(_) => KEventDaemonResponse::Ok { event: None },
                    Err(err) => err_to_response(err),
                }
            }
            KEventDaemonRequest::PullEvent {
                reader_id,
                timeout_ms,
            } => match self.pull_event_in(session, &reader_id, timeout_ms).await {
                Ok(event) => KEventDaemonResponse::Ok { event },
                Err(err) => err_to_response(err),
            },
        }
    }

    async fn distribute(&self, event: &Event) {
        let snapshot: Vec<Arc<ServiceReaderState>> =
            self.readers.read().await.values().cloned().collect();
        for reader in snapshot {
            if reader.matches(&event.eventid) {
                reader.push(event.clone()).await;
            }
        }
    }

    async fn broadcast_to_peers(&self, event: &Event) -> KEventResult<()> {
        let peers = self.peers.read().await.clone();
        let mut last_error: Option<KEventError> = None;
        for peer in peers {
            if let Err(err) = peer.broadcast(event).await {
                last_error = Some(err);
            }
        }
        match last_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    async fn mirror_to_shared_ring(&self, event: &Event) -> KEventResult<()> {
        let shared_ring = self.shared_ring.read().await.clone();
        let Some(shared_ring) = shared_ring else {
            return Ok(());
        };
        shared_ring
            .publish_event(event)
            .map_err(KEventError::Internal)
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn err_to_response(err: KEventError) -> KEventDaemonResponse {
    KEventDaemonResponse::Err {
        code: err.code().to_string(),
        message: err.to_string(),
    }
}

fn should_broadcast_to_peers(event: &Event, local_node: &str) -> bool {
    match &event.ingress_node {
        Some(ingress_node) => ingress_node == local_node,
        None => true,
    }
}

pub fn protocol_ok() -> Value {
    json!({ "status": "ok" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    fn init_test_ringbuffer_path() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let path = std::env::temp_dir().join(format!(
                "kevent_service_ringbuffer_test_{}.shm",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            std::env::set_var("BUCKYOS_KEVENT_RINGBUFFER_PATH", &path);
        });
    }

    #[tokio::test]
    async fn test_daemon_register_publish_pull() {
        let service = KEventService::new("node_a");
        service
            .register_reader("r1", vec!["/taskmgr/**".to_string()])
            .await
            .unwrap();
        service
            .publish_local_global("/taskmgr/new/task_001", json!({"ok": true}))
            .await
            .unwrap();
        let event = service.pull_event("r1", Some(100)).await.unwrap();
        assert!(event.is_some());
        assert_eq!(event.unwrap().eventid, "/taskmgr/new/task_001");
    }

    #[tokio::test]
    async fn test_protocol_request() {
        let service = KEventService::new("node_a");
        let resp = service
            .handle_protocol_request(KEventDaemonRequest::RegisterReader {
                reader_id: "r1".to_string(),
                patterns: vec!["/system/**".to_string()],
            })
            .await;
        assert!(matches!(resp, KEventDaemonResponse::Ok { .. }));
    }

    #[tokio::test]
    async fn test_update_reader_preserves_queue_and_changes_routing() {
        let service = KEventService::new("node_a");
        service
            .register_reader("r1", vec!["/sys/node/online".to_string()])
            .await
            .unwrap();

        // Publish an event matched by the original pattern; do NOT pull.
        service
            .publish_local_global("/sys/node/online", json!({"a": 1}))
            .await
            .unwrap();

        // Add a broader pattern; the finer one should be swallowed by the
        // daemon's normalize_patterns step.
        service
            .update_reader("r1", vec!["/sys/**".to_string()], vec![])
            .await
            .unwrap();

        // Queued event must survive the patterns swap.
        let ev = service.pull_event("r1", Some(20)).await.unwrap().unwrap();
        assert_eq!(ev.eventid, "/sys/node/online");

        // New events covered by the broader pattern now flow through.
        service
            .publish_local_global("/sys/foo/bar", json!({"a": 2}))
            .await
            .unwrap();
        let ev = service.pull_event("r1", Some(50)).await.unwrap().unwrap();
        assert_eq!(ev.eventid, "/sys/foo/bar");

        // Remove the only remaining pattern → must error and not corrupt state.
        let err = service
            .update_reader("r1", vec![], vec!["/sys/**".to_string()])
            .await
            .unwrap_err();
        assert!(matches!(err, KEventError::InvalidPattern(_)));

        // Reader should still route /sys/** events.
        service
            .publish_local_global("/sys/baz", json!({"a": 3}))
            .await
            .unwrap();
        let ev = service.pull_event("r1", Some(50)).await.unwrap().unwrap();
        assert_eq!(ev.eventid, "/sys/baz");
    }

    #[tokio::test]
    async fn test_update_reader_unknown_id() {
        let service = KEventService::new("node_a");
        let err = service
            .update_reader("ghost", vec!["/sys/**".to_string()], vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, KEventError::ReaderClosed(_)));
    }

    #[tokio::test]
    async fn test_accept_external_global_mirrors_to_shared_ring() {
        init_test_ringbuffer_path();
        let consumer = SharedKEventRingBuffer::open().unwrap();
        consumer.prime_cursors();

        let service = KEventService::new("node_a");
        service
            .set_shared_ring(Arc::new(SharedKEventRingBuffer::open().unwrap()))
            .await;

        service
            .accept_external_global(Event {
                eventid: "/system/node/online".to_string(),
                source_node: "light_client".to_string(),
                source_pid: 7,
                ingress_node: Some("light_client".to_string()),
                timestamp: 1,
                data: json!({ "ok": true }),
            })
            .await
            .unwrap();

        let events = consumer.drain_events::<Event>(8);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ingress_node.as_deref(), Some("node_a"));
        assert_eq!(events[0].source_node, "light_client");
    }
}
