use crate::kevent_bridge::{
    is_transport_error, BridgeReaderSink, KEventBridgeReaderLink, KEventDaemonBridgeTransport,
    KEventTransportStatus,
};
use crate::kevent_ringbuffer::SharedKEventRingBuffer;
use crate::{AppDoc, AppType, SelectorType};
use async_trait::async_trait;
use log::warn;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::{oneshot, Mutex, Notify, RwLock};
use tokio::time::Instant;

pub const KEVENT_SERVICE_UNIQUE_ID: &str = "kevent";
pub const KEVENT_SERVICE_NAME: &str = "kevent";
pub const KEVENT_SERVICE_MAIN_PORT: u16 = 3181;
pub const KEVENT_SERVICE_NATIVE_PORT: u16 = 3183;
pub const DEFAULT_READER_CAPACITY: usize = 1024;
pub const MAX_EVENT_DATA_SIZE_BYTES: usize = 64 * 1024;
const SHARED_RING_DRAIN_BATCH: usize = 128;
/// While a transport is down we log one drop summary per this interval
/// instead of one line per dropped event.
const PUBLISH_DROP_LOG_INTERVAL_MS: u64 = 60_000;
/// Maximum time the ShmDispatch thread blocks in futex/ulock before
/// re-checking (acts as a heartbeat / fallback interval).
///
/// On Linux, futex wakes are reliable for shared-memory pages and this
/// timeout only serves as a heartbeat.  On macOS, __ulock may not
/// reliably wake across separate file-backed mmaps, so this timeout
/// also acts as a polling fallback — we keep it short (1ms) to
/// bound the worst-case latency while remaining lightweight.
#[cfg(target_os = "linux")]
const SHM_DISPATCH_WAIT_TIMEOUT_MS: u64 = 500;
#[cfg(not(target_os = "linux"))]
const SHM_DISPATCH_WAIT_TIMEOUT_MS: u64 = 1;

pub type TimerId = String;

pub fn generate_kevent_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        KEVENT_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Kernel Event Bus")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum KEventError {
    #[error("INVALID_EVENTID: {0}")]
    InvalidEventId(String),
    #[error("INVALID_PATTERN: {0}")]
    InvalidPattern(String),
    #[error("DAEMON_UNAVAILABLE: {0}")]
    DaemonUnavailable(String),
    #[error("TIMER_INVALID_TARGET: {0}")]
    TimerInvalidTarget(String),
    #[error("TIMER_NOT_FOUND: {0}")]
    TimerNotFound(String),
    #[error("NOT_SUPPORTED: {0}")]
    NotSupported(String),
    #[error("READER_CLOSED: {0}")]
    ReaderClosed(String),
    #[error("INTERNAL: {0}")]
    Internal(String),
}

pub type KEventResult<T> = std::result::Result<T, KEventError>;

impl KEventError {
    pub fn code(&self) -> &'static str {
        match self {
            KEventError::InvalidEventId(_) => "INVALID_EVENTID",
            KEventError::InvalidPattern(_) => "INVALID_PATTERN",
            KEventError::DaemonUnavailable(_) => "DAEMON_UNAVAILABLE",
            KEventError::TimerInvalidTarget(_) => "TIMER_INVALID_TARGET",
            KEventError::TimerNotFound(_) => "TIMER_NOT_FOUND",
            KEventError::NotSupported(_) => "NOT_SUPPORTED",
            KEventError::ReaderClosed(_) => "READER_CLOSED",
            KEventError::Internal(_) => "INTERNAL",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub eventid: String,
    pub source_node: String,
    pub source_pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_node: Option<String>,
    pub timestamp: u64,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerOptions {
    pub interval_ms: u64,
    #[serde(default = "timer_repeat_default")]
    pub repeat: bool,
    #[serde(default)]
    pub initial_delay_ms: Option<u64>,
    #[serde(default)]
    pub data: Option<Value>,
}

const fn timer_repeat_default() -> bool {
    true
}

impl Default for TimerOptions {
    fn default() -> Self {
        Self {
            interval_ms: 1000,
            repeat: true,
            initial_delay_ms: None,
            data: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KEventClientMode {
    // Local pub/sub/timer, never talks to daemon.
    Local,
    // Full SDK semantics: local + global pub/sub over the chosen transport.
    Full,
    // Light SDK semantics. Only global pub is supported.
    Light,
    // Local publish-only mode.
    LocalPubOnly,
}

/// Which channel carries *global* events for this client.
///
/// The two real transports are mutually exclusive and are chosen when the
/// client is built — never by probing at runtime. The daemon mirrors every
/// global event it accepts into the shared ring, so a process that listened
/// on both would receive each event twice, and `Event` has no id to
/// deduplicate on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KEventTransportKind {
    /// No global channel at all (`Local` / `LocalPubOnly`).
    None,
    /// Same-host processes sharing node-daemon's ring buffer.
    SharedMemory,
    /// Containers and anything else that cannot reach the host's ring:
    /// a native TCP connection to node-daemon.
    DaemonBridge,
    /// Publish-only bridge injected by the caller (Light mode / in-process
    /// transports in tests).
    PublishOnlyBridge,
}

enum KEventTransport {
    None,
    SharedMemory(Arc<SharedKEventRingBuffer>),
    DaemonBridge(Arc<KEventDaemonBridgeTransport>),
    PublishOnlyBridge(Arc<dyn KEventDaemonBridge>),
}

impl KEventTransport {
    fn kind(&self) -> KEventTransportKind {
        match self {
            KEventTransport::None => KEventTransportKind::None,
            KEventTransport::SharedMemory(_) => KEventTransportKind::SharedMemory,
            KEventTransport::DaemonBridge(_) => KEventTransportKind::DaemonBridge,
            KEventTransport::PublishOnlyBridge(_) => KEventTransportKind::PublishOnlyBridge,
        }
    }

    fn shared_ring(&self) -> Option<&Arc<SharedKEventRingBuffer>> {
        match self {
            KEventTransport::SharedMemory(ring) => Some(ring),
            _ => None,
        }
    }

    fn daemon_bridge(&self) -> Option<&Arc<KEventDaemonBridgeTransport>> {
        match self {
            KEventTransport::DaemonBridge(bridge) => Some(bridge),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum KEventDaemonRequest {
    RegisterReader {
        reader_id: String,
        patterns: Vec<String>,
    },
    UnregisterReader {
        reader_id: String,
    },
    UpdateReader {
        reader_id: String,
        #[serde(default)]
        add: Vec<String>,
        #[serde(default)]
        remove: Vec<String>,
    },
    PublishGlobal {
        event: Event,
    },
    PullEvent {
        reader_id: String,
        timeout_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum KEventDaemonResponse {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        event: Option<Event>,
    },
    Err {
        code: String,
        message: String,
    },
}

/// Publish-only escape hatch for callers that already own a protocol client
/// (Light SDK, in-process transports in tests). Reader lifecycle is
/// deliberately *not* part of this trait: a reader needs its own connection
/// with register/pull/reconnect, which only the concrete transports provide.
#[async_trait]
pub trait KEventDaemonBridge: Send + Sync {
    async fn publish_global(&self, event: &Event) -> KEventResult<()>;
}

#[derive(Clone)]
pub struct KEventClient {
    mode: KEventClientMode,
    source_node: String,
    inner: Arc<KEventClientInner>,
}

struct KEventClientInner {
    readers: RwLock<HashMap<String, Arc<ReaderState>>>,
    timers: RwLock<HashMap<TimerId, oneshot::Sender<()>>>,
    transport: KEventTransport,
    /// Random per-instance tag so reader ids are unique across the processes
    /// sharing one daemon. (The daemon also namespaces readers per
    /// connection; this keeps ids readable and unambiguous in logs.)
    instance_nonce: String,
    reader_seq: AtomicU64,
    timer_seq: AtomicU64,
    reader_capacity: usize,
    /// Signaled by ShmDispatch after dispatching shared-ring events to
    /// reader queues.  `pull_event` waits on this instead of polling.
    shm_dispatch_notify: Notify,
    /// Set to true when the client is being dropped, to stop the
    /// ShmDispatch background thread.
    shm_dispatch_stop: AtomicBool,
    /// Best-effort publish bookkeeping for the shared-ring path (the bridge
    /// keeps its own counters).
    shm_publish_dropped: AtomicU64,
    shm_drop_last_log_ms: AtomicU64,
}

struct ReaderState {
    patterns: StdRwLock<Vec<String>>,
    queue: Mutex<VecDeque<Event>>,
    notify: Notify,
    capacity: usize,
}

impl ReaderState {
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

    fn snapshot_patterns(&self) -> Vec<String> {
        self.patterns
            .read()
            .expect("patterns lock poisoned")
            .clone()
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

    /// Synchronous push for use from the ShmDispatch OS thread.
    fn push_sync(&self, event: Event) {
        let mut queue = self.queue.blocking_lock();
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

/// Bridges one `EventReader` to its dedicated daemon connection: the pull
/// task asks for the reader's authoritative pattern set and hands back the
/// events it pulled.
struct ClientReaderSink {
    inner: Weak<KEventClientInner>,
    reader_id: String,
}

#[async_trait]
impl BridgeReaderSink for ClientReaderSink {
    async fn global_patterns(&self) -> Option<Vec<String>> {
        let inner = self.inner.upgrade()?;
        let readers = inner.readers.read().await;
        let state = readers.get(&self.reader_id)?;
        // An empty result is not the same as a missing reader: the caller may
        // have removed every global pattern and can add one back later.
        Some(
            state
                .snapshot_patterns()
                .into_iter()
                .filter(|p| is_global_pattern(p))
                .collect(),
        )
    }

    async fn deliver(&self, event: Event) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let state = {
            let readers = inner.readers.read().await;
            readers.get(&self.reader_id).cloned()
        };
        // Patterns are locally authoritative: re-check before queueing so a
        // pattern the caller just dropped can't leak through the window
        // before the daemon learns about it.
        if let Some(state) = state {
            if state.matches(&event.eventid) {
                state.push(event).await;
            }
        }
    }
}

impl KEventClientInner {
    /// Count a dropped global publish and log at most one line per minute so
    /// a long outage cannot flood the log.
    fn note_shm_publish_dropped(&self, eventid: &str, err: &dyn std::fmt::Display) {
        let dropped = self.shm_publish_dropped.fetch_add(1, Ordering::Relaxed) + 1;
        let now = now_millis();
        let last = self.shm_drop_last_log_ms.load(Ordering::Relaxed);
        if dropped == 1 || now.saturating_sub(last) >= PUBLISH_DROP_LOG_INTERVAL_MS {
            self.shm_drop_last_log_ms.store(now, Ordering::Relaxed);
            warn!(
                "kevent dropped global event {} ({} dropped so far): {}",
                eventid, dropped, err
            );
        }
    }

    async fn dispatch_event(&self, event: &Event) {
        let snapshot: Vec<Arc<ReaderState>> = self.readers.read().await.values().cloned().collect();
        for reader in snapshot {
            if reader.matches(&event.eventid) {
                reader.push(event.clone()).await;
            }
        }
    }

    /// Synchronous version of dispatch_event for use from the ShmDispatch
    /// OS thread.  Uses tokio's blocking_read/blocking_lock so we never
    /// need an async runtime on the calling thread.
    fn dispatch_event_sync(&self, event: &Event) {
        let snapshot: Vec<Arc<ReaderState>> =
            self.readers.blocking_read().values().cloned().collect();
        for reader in snapshot {
            if reader.matches(&event.eventid) {
                reader.push_sync(event.clone());
            }
        }
    }

    /// Drain events from the shared ring buffer and dispatch to matching
    /// readers (synchronous, for ShmDispatch thread).
    /// Returns the number of events dispatched.
    fn import_shared_events_sync(&self, max_events: usize) -> usize {
        let Some(shared_ring) = self.transport.shared_ring() else {
            return 0;
        };
        let events = shared_ring.drain_events::<Event>(max_events);
        let count = events.len();
        for event in events {
            if !is_global_eventid(&event.eventid) {
                continue;
            }
            self.dispatch_event_sync(&event);
        }
        count
    }
}

pub struct EventReader {
    reader_id: String,
    inner: Weak<KEventClientInner>,
    bridge_link: Option<Arc<KEventBridgeReaderLink>>,
    mode: KEventClientMode,
    has_global_patterns: bool,
    closed: AtomicBool,
}

impl KEventClient {
    pub fn new_local(source_node: impl Into<String>) -> Self {
        Self::build(
            source_node,
            KEventClientMode::Local,
            KEventTransport::None,
            DEFAULT_READER_CAPACITY,
        )
    }

    pub fn new_local_pub_only(source_node: impl Into<String>) -> Self {
        Self::build(
            source_node,
            KEventClientMode::LocalPubOnly,
            KEventTransport::None,
            DEFAULT_READER_CAPACITY,
        )
    }

    /// Full client whose global channel is node-daemon's shared ring buffer.
    /// Only valid for processes that share the host's `/tmp` — i.e. not
    /// containers.
    ///
    /// Configuration problems are fatal here on purpose: a client that cannot
    /// open the ring must not be built and quietly fall back to a private one
    /// that nobody else is attached to.
    pub fn new_shared_memory(source_node: impl Into<String>) -> KEventResult<Self> {
        Self::new_shared_memory_with_capacity(source_node, DEFAULT_READER_CAPACITY)
    }

    pub fn new_shared_memory_with_capacity(
        source_node: impl Into<String>,
        reader_capacity: usize,
    ) -> KEventResult<Self> {
        let shared_ring = SharedKEventRingBuffer::open().map_err(|err| {
            KEventError::DaemonUnavailable(format!(
                "kevent shared ringbuffer is unavailable: {}",
                err
            ))
        })?;
        Ok(Self::build(
            source_node,
            KEventClientMode::Full,
            KEventTransport::SharedMemory(Arc::new(shared_ring)),
            reader_capacity,
        ))
    }

    /// Full client whose global channel is a native TCP connection to
    /// node-daemon. The endpoint comes from deployment configuration
    /// (`BuckyOSRuntime::get_kevent_client` fills it in); the local ring is
    /// never used in this mode, even if it happens to be openable.
    ///
    /// Connecting is lazy and retried: a daemon that is down at startup is a
    /// runtime condition the client recovers from, not a construction error.
    pub fn new_daemon_bridge(
        source_node: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> KEventResult<Self> {
        Self::new_daemon_bridge_with_capacity(source_node, endpoint, DEFAULT_READER_CAPACITY)
    }

    pub fn new_daemon_bridge_with_capacity(
        source_node: impl Into<String>,
        endpoint: impl Into<String>,
        reader_capacity: usize,
    ) -> KEventResult<Self> {
        let transport = Arc::new(KEventDaemonBridgeTransport::new(endpoint)?);
        Ok(Self::build(
            source_node,
            KEventClientMode::Full,
            KEventTransport::DaemonBridge(transport),
            reader_capacity,
        ))
    }

    /// Light client (global publish only) over the native TCP bridge.
    pub fn new_light_daemon_bridge(
        source_node: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> KEventResult<Self> {
        let transport = Arc::new(KEventDaemonBridgeTransport::new(endpoint)?);
        Ok(Self::build(
            source_node,
            KEventClientMode::Light,
            KEventTransport::DaemonBridge(transport),
            DEFAULT_READER_CAPACITY,
        ))
    }

    /// Light client over a caller-provided publish channel.
    pub fn new_light(source_node: impl Into<String>, bridge: Arc<dyn KEventDaemonBridge>) -> Self {
        Self::build(
            source_node,
            KEventClientMode::Light,
            KEventTransport::PublishOnlyBridge(bridge),
            DEFAULT_READER_CAPACITY,
        )
    }

    fn build(
        source_node: impl Into<String>,
        mode: KEventClientMode,
        transport: KEventTransport,
        reader_capacity: usize,
    ) -> Self {
        let has_shared_ring = transport.shared_ring().is_some();
        let inner = Arc::new(KEventClientInner {
            readers: RwLock::new(HashMap::new()),
            timers: RwLock::new(HashMap::new()),
            transport,
            instance_nonce: format!("{:08x}", rand::random::<u32>()),
            reader_seq: AtomicU64::new(0),
            timer_seq: AtomicU64::new(0),
            reader_capacity: reader_capacity.max(1),
            shm_dispatch_notify: Notify::new(),
            shm_dispatch_stop: AtomicBool::new(false),
            shm_publish_dropped: AtomicU64::new(0),
            shm_drop_last_log_ms: AtomicU64::new(0),
        });

        // Launch the ShmDispatch background thread when we have a shared ring.
        // This thread blocks on the futex/ulock in shared memory, wakes up on
        // new events, drains them, dispatches to reader queues, and notifies
        // pull_event waiters.  It replaces the old 5ms polling approach.
        if has_shared_ring {
            let weak = Arc::downgrade(&inner);
            std::thread::Builder::new()
                .name("kevent-shm-dispatch".into())
                .spawn(move || {
                    shm_dispatch_thread(weak);
                })
                .ok();
        }

        Self {
            mode,
            source_node: source_node.into(),
            inner,
        }
    }

    pub fn mode(&self) -> KEventClientMode {
        self.mode
    }

    pub fn transport_kind(&self) -> KEventTransportKind {
        self.inner.transport.kind()
    }

    /// Queryable transport health for the daemon-bridge mode: last error,
    /// consecutive failure count, whether links are currently connected.
    /// `None` for transports that have no connection state.
    pub fn transport_status(&self) -> Option<KEventTransportStatus> {
        self.inner
            .transport
            .daemon_bridge()
            .map(|bridge| bridge.status())
    }

    pub async fn create_event_reader(&self, patterns: Vec<String>) -> KEventResult<EventReader> {
        if patterns.is_empty() {
            return Err(KEventError::InvalidPattern(
                "patterns must not be empty".to_string(),
            ));
        }
        if matches!(
            self.mode,
            KEventClientMode::Light | KEventClientMode::LocalPubOnly
        ) {
            return Err(KEventError::NotSupported(
                "current mode does not support create_event_reader".to_string(),
            ));
        }

        let mut has_global_patterns = false;
        for pattern in &patterns {
            validate_pattern(pattern)?;
            if is_global_pattern(pattern) {
                has_global_patterns = true;
            }
        }

        let normalized = normalize_patterns(patterns);
        let reader_id = format!(
            "r_{}_{}",
            self.inner.instance_nonce,
            self.inner.reader_seq.fetch_add(1, Ordering::Relaxed) + 1
        );
        let state = Arc::new(ReaderState::new(
            normalized.clone(),
            self.inner.reader_capacity.max(1),
        ));
        self.inner
            .readers
            .write()
            .await
            .insert(reader_id.clone(), state);

        // Global subscriptions need the transport wired up. Both paths are
        // optimistic: a daemon that is unreachable right now is a transient
        // condition the reader recovers from on its own, so creation succeeds
        // and the caller's pull loop keeps its normal cadence.
        let mut bridge_link = None;
        if self.mode == KEventClientMode::Full && has_global_patterns {
            match &self.inner.transport {
                KEventTransport::SharedMemory(shared_ring) => shared_ring.prime_cursors(),
                KEventTransport::DaemonBridge(bridge) => {
                    let sink = Arc::new(ClientReaderSink {
                        inner: Arc::downgrade(&self.inner),
                        reader_id: reader_id.clone(),
                    });
                    bridge_link = Some(bridge.spawn_reader(reader_id.clone(), sink));
                }
                KEventTransport::None | KEventTransport::PublishOnlyBridge(_) => {}
            }
        }

        Ok(EventReader {
            reader_id,
            inner: Arc::downgrade(&self.inner),
            bridge_link,
            mode: self.mode,
            has_global_patterns,
            closed: AtomicBool::new(false),
        })
    }

    pub async fn pub_event(&self, eventid: &str, data: Value) -> KEventResult<()> {
        validate_eventid(eventid)?;
        validate_event_data_size(&data)?;

        let event = Event {
            eventid: eventid.to_string(),
            source_node: self.source_node.clone(),
            source_pid: std::process::id(),
            ingress_node: if is_global_eventid(eventid) {
                Some(self.source_node.clone())
            } else {
                None
            },
            timestamp: now_millis(),
            data,
        };

        let is_global = is_global_eventid(eventid);

        // Loop-back rule. The shared ring skips the publisher's own ring on
        // drain, so a local dispatch is the only way we see our own global
        // events there. Over the bridge the daemon *does* route the event
        // back to us, so dispatching locally as well would deliver it twice —
        // and `Event` carries no id to dedupe on. Let the daemon be the single
        // ordering source instead.
        let dispatch_locally =
            !is_global || !matches!(self.inner.transport, KEventTransport::DaemonBridge(_));
        if dispatch_locally {
            self.dispatch_local(&event).await;
        }

        match self.mode {
            KEventClientMode::Local => Ok(()),
            KEventClientMode::LocalPubOnly => Ok(()),
            KEventClientMode::Full => {
                if is_global {
                    self.publish_global_best_effort(&event).await
                } else {
                    Ok(())
                }
            }
            KEventClientMode::Light => {
                if !is_global {
                    return Err(KEventError::NotSupported(
                        "light mode only supports global event publishing".to_string(),
                    ));
                }
                self.publish_global_best_effort(&event).await
            }
        }
    }

    /// KEvent is a lossy notification channel, so a transport that is down
    /// drops the event instead of failing the caller's business operation:
    /// input errors still surface as `Err`, transport errors become a counter
    /// plus a rate-limited log. Reliable data keeps flowing through kMsgQueue,
    /// the database, or the consumer's own sweep.
    async fn publish_global_best_effort(&self, event: &Event) -> KEventResult<()> {
        match &self.inner.transport {
            KEventTransport::SharedMemory(shared_ring) => {
                let payload = serde_json::to_vec(event).map_err(|err| {
                    KEventError::Internal(format!("failed to encode event: {}", err))
                })?;
                if payload.len() > SharedKEventRingBuffer::max_payload_size() {
                    // Deterministic input-size error, not an outage.
                    return Err(KEventError::InvalidEventId(format!(
                        "event too large for shared ring: {} bytes, max {}",
                        payload.len(),
                        SharedKEventRingBuffer::max_payload_size()
                    )));
                }
                if let Err(err) = shared_ring.publish_payload(&payload) {
                    self.inner.note_shm_publish_dropped(&event.eventid, &err);
                }
                Ok(())
            }
            KEventTransport::DaemonBridge(bridge) => match bridge.publish_global(event).await {
                Ok(_) => Ok(()),
                // The bridge already rate-limits its own outage logging.
                Err(err) if is_transport_error(&err) => {
                    bridge.note_publish_dropped();
                    Ok(())
                }
                Err(err) => Err(err),
            },
            KEventTransport::PublishOnlyBridge(bridge) => match bridge.publish_global(event).await {
                Ok(_) => Ok(()),
                Err(err) if is_transport_error(&err) => {
                    self.inner.note_shm_publish_dropped(&event.eventid, &err);
                    Ok(())
                }
                Err(err) => Err(err),
            },
            KEventTransport::None => Err(KEventError::NotSupported(
                "client has no global event transport".to_string(),
            )),
        }
    }

    // Called by external daemon bridge receiver when a remote global event arrives.
    pub async fn ingest_global_event(&self, mut event: Event) -> KEventResult<()> {
        if !is_global_eventid(&event.eventid) {
            return Err(KEventError::InvalidEventId(
                "ingest_global_event only accepts global eventid".to_string(),
            ));
        }
        validate_eventid(&event.eventid)?;
        if event.ingress_node.is_none() {
            event.ingress_node = Some(event.source_node.clone());
        }
        self.dispatch_local(&event).await;
        Ok(())
    }

    pub async fn create_timer(
        &self,
        eventid: &str,
        options: TimerOptions,
    ) -> KEventResult<TimerId> {
        if matches!(
            self.mode,
            KEventClientMode::Light | KEventClientMode::LocalPubOnly
        ) {
            return Err(KEventError::NotSupported(
                "current mode does not support create_timer".to_string(),
            ));
        }
        if is_global_eventid(eventid) {
            return Err(KEventError::TimerInvalidTarget(
                "timer target must be local eventid".to_string(),
            ));
        }
        validate_eventid(eventid)?;
        if options.interval_ms == 0 {
            return Err(KEventError::TimerInvalidTarget(
                "interval_ms must be > 0".to_string(),
            ));
        }

        let timer_id = format!(
            "t_{}",
            self.inner.timer_seq.fetch_add(1, Ordering::Relaxed) + 1
        );
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        self.inner
            .timers
            .write()
            .await
            .insert(timer_id.clone(), stop_tx);

        let initial_delay = options.initial_delay_ms.unwrap_or(options.interval_ms);
        let interval = options.interval_ms;
        let repeat = options.repeat;
        let eventid = eventid.to_string();
        let timer_id_for_task = timer_id.clone();
        let client = self.clone();
        let payload = options.data.clone();

        tokio::spawn(async move {
            let mut tick_count: u64 = 0;
            if initial_delay > 0 {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(initial_delay)) => {}
                    _ = &mut stop_rx => { return; }
                }
            }

            loop {
                tick_count += 1;
                let timer_data = build_timer_data(&timer_id_for_task, tick_count, payload.clone());
                if let Err(err) = client.pub_event(&eventid, timer_data).await {
                    warn!(
                        "publish timer event failed, timer_id={}, err={:?}",
                        timer_id_for_task, err
                    );
                }

                if !repeat {
                    break;
                }

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(interval)) => {}
                    _ = &mut stop_rx => { break; }
                }
            }

            client.inner.timers.write().await.remove(&timer_id_for_task);
        });

        Ok(timer_id)
    }

    pub async fn cancel_timer(&self, timer_id: &str) -> KEventResult<()> {
        let timer = self.inner.timers.write().await.remove(timer_id);
        match timer {
            Some(stop_tx) => {
                let _ = stop_tx.send(());
                Ok(())
            }
            None => Err(KEventError::TimerNotFound(timer_id.to_string())),
        }
    }

    async fn dispatch_local(&self, event: &Event) {
        self.inner.dispatch_event(event).await;
    }
}

// ---------------------------------------------------------------------------
// ShmDispatch background thread  (design doc §6)
//
// Runs on a dedicated OS thread (not a tokio task) so it can block on
// futex/ulock without occupying a tokio worker.  When woken by a producer
// writing to shared memory, it drains events, dispatches them to matching
// reader queues, and notifies `pull_event` waiters via `shm_dispatch_notify`.
// ---------------------------------------------------------------------------

fn shm_dispatch_thread(weak: Weak<KEventClientInner>) {
    loop {
        let Some(inner) = weak.upgrade() else {
            return;
        };

        if inner.shm_dispatch_stop.load(Ordering::Relaxed) {
            return;
        }

        let shared_ring = match inner.transport.shared_ring() {
            Some(sr) => sr.clone(),
            None => return,
        };

        // Snapshot notify_seq before draining, so we don't miss events
        // published between drain and wait.
        let seq_before = shared_ring.load_notify_seq();

        // Drain and dispatch synchronously (no tokio runtime needed).
        let dispatched = inner.import_shared_events_sync(SHARED_RING_DRAIN_BATCH);

        if dispatched > 0 {
            // Wake all pull_event waiters so they re-check their queues.
            inner.shm_dispatch_notify.notify_waiters();
        }

        // Drop the Arc before blocking so it doesn't keep the client alive.
        drop(inner);

        // Block on futex/ulock until notify_seq changes from seq_before,
        // or until the timeout expires (fallback heartbeat).
        shared_ring.wait_for_events(
            seq_before,
            Duration::from_millis(SHM_DISPATCH_WAIT_TIMEOUT_MS),
        );
    }
}

impl EventReader {
    pub fn reader_id(&self) -> &str {
        &self.reader_id
    }

    /// Returns the current pattern set for this reader. Useful for tests
    /// and observability; not part of the hot path.
    pub async fn patterns(&self) -> Vec<String> {
        let Some(inner) = self.inner.upgrade() else {
            return Vec::new();
        };
        let readers = inner.readers.read().await;
        match readers.get(&self.reader_id) {
            Some(state) => state.snapshot_patterns(),
            None => Vec::new(),
        }
    }

    /// Subscribe additional patterns. Patterns already covered by an existing
    /// pattern are silently swallowed; patterns that subsume existing ones
    /// replace them. Existing queued events are preserved. Future-only:
    /// events that arrived before this call are not retroactively delivered.
    pub async fn add_patterns(&self, patterns: Vec<String>) -> KEventResult<()> {
        if patterns.is_empty() {
            return Ok(());
        }
        let inner = self
            .inner
            .upgrade()
            .ok_or_else(|| KEventError::ReaderClosed(self.reader_id.clone()))?;

        let mut has_global = false;
        let mut has_local = false;
        for pattern in &patterns {
            validate_pattern(pattern)?;
            if is_global_pattern(pattern) {
                has_global = true;
            } else {
                has_local = true;
            }
        }
        if has_global && !self.has_global_patterns {
            return Err(KEventError::NotSupported(
                "cannot add global patterns to a reader with no global subscriptions; \
                 create a new reader instead"
                    .to_string(),
            ));
        }
        let _ = has_local;

        let state = {
            let readers = inner.readers.read().await;
            readers.get(&self.reader_id).cloned()
        }
        .ok_or_else(|| KEventError::ReaderClosed(self.reader_id.clone()))?;

        let (effective_added_globals, removed_redundant_globals) = {
            let mut guard = state.patterns.write().expect("patterns lock poisoned");
            let current = guard.clone();
            let mut combined = current.clone();
            combined.extend(patterns);
            let normalized = normalize_patterns(combined);

            let added: Vec<String> = normalized
                .iter()
                .filter(|p| !current.contains(p))
                .cloned()
                .collect();
            let removed: Vec<String> = current
                .iter()
                .filter(|p| !normalized.contains(p))
                .cloned()
                .collect();

            *guard = normalized;

            (
                added
                    .into_iter()
                    .filter(|p| is_global_pattern(p))
                    .collect::<Vec<_>>(),
                removed
                    .into_iter()
                    .filter(|p| is_global_pattern(p))
                    .collect::<Vec<_>>(),
            )
        };

        // The local pattern set is authoritative. Reaching the daemon is the
        // background link's job: it resends the *complete* set once the
        // in-flight long poll returns, so an edit made while the daemon is
        // down still lands on reconnect and the caller never sees a transport
        // error for a local state change.
        if !effective_added_globals.is_empty() || !removed_redundant_globals.is_empty() {
            if let Some(link) = &self.bridge_link {
                link.mark_dirty();
            }
        }

        Ok(())
    }

    /// Unsubscribe specific patterns by exact match. Patterns not currently
    /// in the set are silently ignored. Errors if the removal would leave
    /// the reader with no patterns. Future-only: already-queued events
    /// matching removed patterns remain pull-able.
    pub async fn remove_patterns(&self, patterns: Vec<String>) -> KEventResult<()> {
        if patterns.is_empty() {
            return Ok(());
        }
        let inner = self
            .inner
            .upgrade()
            .ok_or_else(|| KEventError::ReaderClosed(self.reader_id.clone()))?;

        let state = {
            let readers = inner.readers.read().await;
            readers.get(&self.reader_id).cloned()
        }
        .ok_or_else(|| KEventError::ReaderClosed(self.reader_id.clone()))?;

        let removed_globals: Vec<String> = {
            let mut guard = state.patterns.write().expect("patterns lock poisoned");
            let next: Vec<String> = guard
                .iter()
                .filter(|p| !patterns.iter().any(|r| r == *p))
                .cloned()
                .collect();
            if next.is_empty() {
                return Err(KEventError::InvalidPattern(
                    "reader must keep at least one pattern".to_string(),
                ));
            }
            let removed: Vec<String> = guard
                .iter()
                .filter(|p| !next.contains(p) && is_global_pattern(p))
                .cloned()
                .collect();
            *guard = next;
            removed
        };

        if !removed_globals.is_empty() {
            if let Some(link) = &self.bridge_link {
                link.mark_dirty();
            }
        }

        Ok(())
    }

    pub async fn pull_event(&self, timeout_ms: Option<u64>) -> KEventResult<Option<Event>> {
        let inner = self
            .inner
            .upgrade()
            .ok_or_else(|| KEventError::ReaderClosed(self.reader_id.clone()))?;

        let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));

        loop {
            let state = {
                let readers = inner.readers.read().await;
                readers.get(&self.reader_id).cloned()
            }
            .ok_or_else(|| KEventError::ReaderClosed(self.reader_id.clone()))?;

            if let Some(event) = state.pop().await {
                return Ok(Some(event));
            }

            if let Some(ms) = timeout_ms {
                if ms == 0 {
                    return Ok(None);
                }
            }

            // Wait for either:
            // - state.notify: local pub_event or timer delivered an event
            // - shm_dispatch_notify: ShmDispatch thread delivered shared-ring events
            // Both notifies will fire when there is something in our queue.
            let shm_notified = inner.shm_dispatch_notify.notified();
            let reader_notified = state.notify.notified();
            match deadline {
                None => {
                    tokio::select! {
                        _ = shm_notified => {}
                        _ = reader_notified => {}
                    }
                }
                Some(deadline_at) => {
                    let now = Instant::now();
                    if now >= deadline_at {
                        return Ok(None);
                    }
                    let remain = deadline_at - now;
                    tokio::select! {
                        _ = shm_notified => {}
                        _ = reader_notified => {}
                        _ = tokio::time::sleep(remain) => {
                            // Final drain attempt before returning timeout
                            if let Some(event) = state.pop().await {
                                return Ok(Some(event));
                            }
                            return Ok(None);
                        }
                    }
                }
            }
        }
    }

    /// Close the reader. Over the daemon bridge this drops the reader's
    /// connection rather than sending an unregister: the daemon reclaims
    /// every reader of a closed connection, so we don't have to wait for the
    /// in-flight long poll to come back first.
    pub async fn close(&self) -> KEventResult<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(link) = &self.bridge_link {
            link.stop();
        }

        let Some(inner) = self.inner.upgrade() else {
            return Ok(());
        };
        inner.readers.write().await.remove(&self.reader_id);
        Ok(())
    }
}

impl Drop for EventReader {
    fn drop(&mut self) {
        // Dropping the link aborts the pull task and closes its connection.
        if let Some(link) = &self.bridge_link {
            link.stop();
        }
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let reader_id = self.reader_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                inner.readers.write().await.remove(&reader_id);
            });
        } else if let Ok(mut readers) = inner.readers.try_write() {
            readers.remove(&reader_id);
        }
    }
}

pub fn validate_event_data_size(data: &Value) -> KEventResult<()> {
    let data_size = serde_json::to_vec(data)
        .map_err(|err| KEventError::Internal(format!("failed to encode event data: {}", err)))?
        .len();
    if data_size > MAX_EVENT_DATA_SIZE_BYTES {
        return Err(KEventError::InvalidEventId(format!(
            "event data too large: {} bytes, max {}",
            data_size, MAX_EVENT_DATA_SIZE_BYTES
        )));
    }
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn build_timer_data(timer_id: &str, tick_count: u64, data: Option<Value>) -> Value {
    let timer_meta = json!({
        "timer_id": timer_id,
        "tick_count": tick_count
    });

    match data {
        None => json!({ "_timer": timer_meta }),
        Some(Value::Object(mut map)) => {
            map.insert("_timer".to_string(), timer_meta);
            Value::Object(map)
        }
        Some(other) => {
            let mut map = Map::new();
            map.insert("payload".to_string(), other);
            map.insert("_timer".to_string(), timer_meta);
            Value::Object(map)
        }
    }
}

pub fn match_event_patterns<S: AsRef<str>>(patterns: &[S], eventid: &str) -> bool {
    for pattern in patterns {
        let pattern = pattern.as_ref();
        if is_global_pattern(pattern) {
            if is_global_eventid(eventid) && match_global_pattern(pattern, eventid) {
                return true;
            }
        } else if pattern == eventid {
            return true;
        }
    }
    false
}

pub fn is_global_eventid(eventid: &str) -> bool {
    eventid.starts_with('/')
}

pub fn is_global_pattern(pattern: &str) -> bool {
    pattern.starts_with('/')
}

pub fn validate_eventid(eventid: &str) -> KEventResult<()> {
    if eventid.is_empty() {
        return Err(KEventError::InvalidEventId("empty eventid".to_string()));
    }
    if is_global_eventid(eventid) {
        validate_global_path(eventid, false).map_err(KEventError::InvalidEventId)?;
    } else {
        validate_local_name(eventid, false).map_err(KEventError::InvalidEventId)?;
    }
    Ok(())
}

pub fn validate_pattern(pattern: &str) -> KEventResult<()> {
    if pattern.is_empty() {
        return Err(KEventError::InvalidPattern("empty pattern".to_string()));
    }
    if is_global_pattern(pattern) {
        validate_global_path(pattern, true).map_err(KEventError::InvalidPattern)?;
    } else {
        validate_local_name(pattern, true).map_err(KEventError::InvalidPattern)?;
        if pattern.contains('*') {
            return Err(KEventError::InvalidPattern(
                "local pattern does not support wildcard".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_global_path(path: &str, allow_wildcard: bool) -> std::result::Result<(), String> {
    if !path.starts_with('/') {
        return Err("global id/pattern must start with '/'".to_string());
    }
    if path.len() > 256 {
        return Err("global id/pattern length must be <= 256".to_string());
    }
    if path == "/" {
        return Err("global id/pattern must not be '/'".to_string());
    }

    let mut depth = 0usize;
    for seg in path.split('/').skip(1) {
        if seg.is_empty() {
            return Err("global id/pattern contains empty segment".to_string());
        }
        depth += 1;
        if depth > 8 {
            return Err("global id/pattern depth must be <= 8".to_string());
        }
        if allow_wildcard && (seg == "*" || seg == "**") {
            continue;
        }
        if seg.contains('*') {
            return Err("wildcard must be a full segment '*' or '**'".to_string());
        }
        if !seg.chars().all(is_valid_name_char) {
            return Err(format!("invalid segment '{}'", seg));
        }
    }
    Ok(())
}

fn validate_local_name(name: &str, _allow_wildcard: bool) -> std::result::Result<(), String> {
    if name.len() > 128 {
        return Err("local id/pattern length must be <= 128".to_string());
    }
    if name.contains('/') {
        return Err("local id/pattern must not contain '/'".to_string());
    }
    if !name.chars().all(is_valid_name_char) {
        return Err("local id/pattern has invalid char".to_string());
    }
    Ok(())
}

fn is_valid_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.'
}

pub fn match_global_pattern(pattern: &str, eventid: &str) -> bool {
    if !pattern.starts_with('/') || !eventid.starts_with('/') {
        return false;
    }

    let p_segments: Vec<&str> = pattern.split('/').skip(1).collect();
    let e_segments: Vec<&str> = eventid.split('/').skip(1).collect();
    match_global_segments(&p_segments, &e_segments)
}

/// Returns true iff every eventid that would match `narrow` also matches `broad`.
///
/// This is *pattern-vs-pattern* containment, not the eventid-vs-pattern check
/// done by `match_event_patterns`. It is conservative: cases that are difficult
/// to decide return false, so callers may keep two patterns that are actually
/// redundant (extra cost: one more entry in the match loop), but will never
/// silently drop a pattern whose match set is larger than the keeper's.
pub fn pattern_subsumes(broad: &str, narrow: &str) -> bool {
    if broad == narrow {
        return true;
    }
    let broad_global = is_global_pattern(broad);
    let narrow_global = is_global_pattern(narrow);
    if broad_global != narrow_global {
        return false;
    }
    if !broad_global {
        // Local patterns are literal names with no wildcards; only equality subsumes.
        return false;
    }
    let broad_segs: Vec<&str> = broad.split('/').skip(1).collect();
    let narrow_segs: Vec<&str> = narrow.split('/').skip(1).collect();
    pattern_subsumes_segments(&broad_segs, &narrow_segs)
}

fn pattern_subsumes_segments(broad: &[&str], narrow: &[&str]) -> bool {
    match (broad.first(), narrow.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(_), None) => broad.iter().all(|s| *s == "**"),
        (Some(&b0), Some(&n0)) => match (b0, n0) {
            ("**", "**") => {
                // L(Σ*·B) ⊇ L(Σ*·N) iff L(Σ*·B) ⊇ L(N): the leading Σ* on the
                // narrow side adds no constraints the broad's leading Σ* can't
                // already absorb, so peel ** from narrow and recurse.
                pattern_subsumes_segments(broad, &narrow[1..])
            }
            ("**", _) => {
                // broad's ** matches zero segments → rest-of-broad must subsume narrow
                if pattern_subsumes_segments(&broad[1..], narrow) {
                    return true;
                }
                // OR ** absorbs narrow's first single-segment match (n0 is "*"
                // or literal here, exactly one segment).
                pattern_subsumes_segments(broad, &narrow[1..])
            }
            (_, "**") => {
                // narrow's L is broader at this position than broad's; not subsumed
                // (conservative: there are corner cases like an all-** broad tail
                // that could still be true, but the recursion above handles those
                // via the ("**", "**") arm).
                false
            }
            ("*", _) => {
                // broad's "*" accepts any single segment; narrow's "*" or literal
                // is also a single segment → accepted. Compare the rest.
                pattern_subsumes_segments(&broad[1..], &narrow[1..])
            }
            (_, "*") => {
                // narrow's "*" can produce a segment broad's literal doesn't match.
                false
            }
            (b_lit, n_lit) => {
                if b_lit != n_lit {
                    return false;
                }
                pattern_subsumes_segments(&broad[1..], &narrow[1..])
            }
        },
    }
}

/// Normalize a pattern set: drop exact duplicates and patterns that are
/// subsumed by another pattern in the same set. Order is preserved among
/// the survivors.
pub fn normalize_patterns(patterns: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(patterns.len());
    for p in patterns {
        if out.iter().any(|kept| pattern_subsumes(kept, &p)) {
            continue;
        }
        out.retain(|kept| !pattern_subsumes(&p, kept));
        out.push(p);
    }
    out
}

fn match_global_segments(pattern: &[&str], event: &[&str]) -> bool {
    if pattern.is_empty() {
        return event.is_empty();
    }

    match pattern[0] {
        "**" => {
            if match_global_segments(&pattern[1..], event) {
                return true;
            }
            if event.is_empty() {
                return false;
            }
            match_global_segments(pattern, &event[1..])
        }
        "*" => {
            if event.is_empty() {
                return false;
            }
            match_global_segments(&pattern[1..], &event[1..])
        }
        literal => {
            if event.is_empty() {
                return false;
            }
            if literal != event[0] {
                return false;
            }
            match_global_segments(&pattern[1..], &event[1..])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct MockBridge {
        published: Arc<Mutex<Vec<Event>>>,
    }

    #[async_trait]
    impl KEventDaemonBridge for MockBridge {
        async fn publish_global(&self, event: &Event) -> KEventResult<()> {
            self.published.lock().await.push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn test_validate_eventid() {
        assert!(validate_eventid("/taskmgr/new/task_001").is_ok());
        assert!(validate_eventid("heartbeat_tick").is_ok());
        assert!(validate_eventid("/").is_err());
        assert!(validate_eventid("bad/name").is_err());
    }

    #[test]
    fn test_pattern_match() {
        assert!(match_global_pattern(
            "/taskmgr/*/task_001",
            "/taskmgr/new/task_001"
        ));
        assert!(!match_global_pattern(
            "/taskmgr/*/task_001",
            "/taskmgr/a/b/task_001"
        ));
        assert!(match_global_pattern("/taskmgr/**", "/taskmgr/new"));
        assert!(match_global_pattern("/taskmgr/**", "/taskmgr/new/task_001"));
    }

    #[test]
    fn test_match_event_patterns() {
        let patterns = ["heartbeat_tick", "/taskmgr/**"];

        assert!(match_event_patterns(&patterns, "heartbeat_tick"));
        assert!(match_event_patterns(&patterns, "/taskmgr/new/task_001"));
        assert!(!match_event_patterns(&patterns, "heartbeat_tock"));
        assert!(!match_event_patterns(&patterns, "/other/task_001"));
    }

    #[test]
    fn test_pattern_subsumes_basic() {
        assert!(pattern_subsumes("/sys/**", "/sys/**"));
        assert!(pattern_subsumes("/sys/**", "/sys/node/online"));
        assert!(pattern_subsumes("/sys/**", "/sys/node/*"));
        assert!(pattern_subsumes("/sys/**", "/sys/**/online"));
        assert!(pattern_subsumes("/sys/**", "/sys/*"));
        assert!(pattern_subsumes("/sys/**", "/sys"));

        assert!(!pattern_subsumes("/sys/node/*", "/sys/**"));
        assert!(!pattern_subsumes("/sys/*", "/sys/**"));
        assert!(!pattern_subsumes("/sys/*", "/sys/a/b"));
        assert!(!pattern_subsumes("/sys/node/online", "/sys/node/offline"));
        assert!(!pattern_subsumes("/sys/**", "/other/x"));

        // local patterns: only equality subsumes
        assert!(pattern_subsumes("heartbeat_tick", "heartbeat_tick"));
        assert!(!pattern_subsumes("heartbeat_tick", "heartbeat_tock"));
        // global vs local are disjoint
        assert!(!pattern_subsumes("/sys/**", "heartbeat_tick"));
        assert!(!pattern_subsumes("heartbeat_tick", "/sys/x"));
    }

    #[test]
    fn test_pattern_subsumes_star() {
        assert!(pattern_subsumes("/sys/*/online", "/sys/node/online"));
        assert!(pattern_subsumes("/sys/*/online", "/sys/*/online"));
        assert!(!pattern_subsumes("/sys/*/online", "/sys/*/offline"));
        assert!(!pattern_subsumes("/sys/*/online", "/sys/node/sub/online"));
        // ** absorbs star matches too
        assert!(pattern_subsumes("/**/online", "/sys/node/online"));
        assert!(pattern_subsumes("/**/online", "/sys/*/online"));
    }

    #[test]
    fn test_normalize_patterns() {
        // broad pattern swallows a later finer one
        let normalized = normalize_patterns(vec!["/sys/**".into(), "/sys/node/online".into()]);
        assert_eq!(normalized, vec!["/sys/**".to_string()]);

        // finer pattern arriving first is removed when a later broad pattern subsumes it
        let normalized = normalize_patterns(vec!["/sys/node/online".into(), "/sys/**".into()]);
        assert_eq!(normalized, vec!["/sys/**".to_string()]);

        // unrelated patterns coexist
        let normalized = normalize_patterns(vec!["/sys/**".into(), "/taskmgr/**".into()]);
        assert_eq!(
            normalized,
            vec!["/sys/**".to_string(), "/taskmgr/**".to_string()]
        );

        // exact duplicates dropped
        let normalized = normalize_patterns(vec!["/sys/**".into(), "/sys/**".into()]);
        assert_eq!(normalized, vec!["/sys/**".to_string()]);
    }

    #[tokio::test]
    async fn test_local_pub_sub() {
        let client = KEventClient::new_local("node_a");
        let reader = client
            .create_event_reader(vec![
                "heartbeat_tick".to_string(),
                "/taskmgr/**".to_string(),
            ])
            .await
            .unwrap();

        client
            .pub_event("heartbeat_tick", json!({"a": 1}))
            .await
            .unwrap();
        let event = reader.pull_event(Some(50)).await.unwrap().unwrap();
        assert_eq!(event.eventid, "heartbeat_tick");

        client
            .pub_event("/taskmgr/new/task_001", json!({"b": 2}))
            .await
            .unwrap();
        let event = reader.pull_event(Some(50)).await.unwrap().unwrap();
        assert_eq!(event.eventid, "/taskmgr/new/task_001");
    }

    #[tokio::test]
    async fn test_add_remove_patterns_local() {
        let client = KEventClient::new_local("node_a");
        let reader = client
            .create_event_reader(vec!["heartbeat_tick".to_string()])
            .await
            .unwrap();

        // Initially "tock" is not subscribed
        client
            .pub_event("heartbeat_tock", json!({"a": 1}))
            .await
            .unwrap();
        assert!(reader.pull_event(Some(20)).await.unwrap().is_none());

        // Add "tock" subscription
        reader
            .add_patterns(vec!["heartbeat_tock".to_string()])
            .await
            .unwrap();
        client
            .pub_event("heartbeat_tock", json!({"a": 2}))
            .await
            .unwrap();
        let ev = reader.pull_event(Some(50)).await.unwrap().unwrap();
        assert_eq!(ev.eventid, "heartbeat_tock");

        // Remove "tick"
        reader
            .remove_patterns(vec!["heartbeat_tick".to_string()])
            .await
            .unwrap();
        client
            .pub_event("heartbeat_tick", json!({"a": 3}))
            .await
            .unwrap();
        // Tick removed → no delivery
        assert!(reader.pull_event(Some(20)).await.unwrap().is_none());
        // Tock still works
        client
            .pub_event("heartbeat_tock", json!({"a": 4}))
            .await
            .unwrap();
        let ev = reader.pull_event(Some(50)).await.unwrap().unwrap();
        assert_eq!(ev.eventid, "heartbeat_tock");

        // Cannot remove the last pattern
        let err = reader
            .remove_patterns(vec!["heartbeat_tock".to_string()])
            .await
            .unwrap_err();
        assert!(matches!(err, KEventError::InvalidPattern(_)));
    }

    #[tokio::test]
    async fn test_add_patterns_subsumes_finer() {
        let client = KEventClient::new_local("node_a");
        let reader = client
            .create_event_reader(vec!["/sys/node/online".to_string()])
            .await
            .unwrap();

        // Add a broader pattern; the finer one should get swallowed.
        reader
            .add_patterns(vec!["/sys/**".to_string()])
            .await
            .unwrap();
        let patterns = reader.patterns().await;
        assert_eq!(patterns, vec!["/sys/**".to_string()]);

        // Adding a finer pattern that's already covered is a no-op.
        reader
            .add_patterns(vec!["/sys/node/offline".to_string()])
            .await
            .unwrap();
        assert_eq!(reader.patterns().await, vec!["/sys/**".to_string()]);

        // Both event flavors deliver because /sys/** matches them.
        client
            .pub_event("/sys/node/online", json!({"x": 1}))
            .await
            .unwrap();
        client
            .pub_event("/sys/foo/bar", json!({"x": 2}))
            .await
            .unwrap();
        let mut got = vec![];
        while let Some(ev) = reader.pull_event(Some(50)).await.unwrap() {
            got.push(ev.eventid);
        }
        assert!(got.contains(&"/sys/node/online".to_string()));
        assert!(got.contains(&"/sys/foo/bar".to_string()));
    }

    #[tokio::test]
    async fn test_add_patterns_preserves_queue() {
        let client = KEventClient::new_local("node_a");
        let reader = client
            .create_event_reader(vec!["heartbeat_tick".to_string()])
            .await
            .unwrap();

        client
            .pub_event("heartbeat_tick", json!({"a": 1}))
            .await
            .unwrap();
        // Add a new pattern WITHOUT pulling first → queued event must survive.
        reader
            .add_patterns(vec!["heartbeat_tock".to_string()])
            .await
            .unwrap();
        let ev = reader.pull_event(Some(20)).await.unwrap().unwrap();
        assert_eq!(ev.eventid, "heartbeat_tick");
    }

    #[tokio::test]
    async fn test_timer() {
        let client = KEventClient::new_local("node_a");
        let reader = client
            .create_event_reader(vec!["heartbeat_tick".to_string()])
            .await
            .unwrap();
        let timer_id = client
            .create_timer(
                "heartbeat_tick",
                TimerOptions {
                    interval_ms: 20,
                    repeat: false,
                    initial_delay_ms: Some(10),
                    data: Some(json!({"x": 1})),
                },
            )
            .await
            .unwrap();
        assert!(timer_id.starts_with("t_"));

        let event = reader.pull_event(Some(200)).await.unwrap().unwrap();
        assert_eq!(event.eventid, "heartbeat_tick");
        assert!(event.data.get("_timer").is_some());
    }

    #[tokio::test]
    async fn test_light_mode_publish_only() {
        let bridge = Arc::new(MockBridge {
            published: Arc::new(Mutex::new(Vec::new())),
        });
        let client = KEventClient::new_light("light_node", bridge.clone());
        client
            .pub_event("/system/node/online", json!({"ok": true}))
            .await
            .unwrap();
        let published = bridge.published.lock().await;
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].eventid, "/system/node/online");

        let err = client
            .create_event_reader(vec!["local_event".to_string()])
            .await
            .err()
            .unwrap();
        assert_eq!(err.code(), "NOT_SUPPORTED");
    }

    #[tokio::test]
    async fn test_full_mode_global_process_short_circuit_without_bridge() {
        let _ring_guard = crate::kevent_ringbuffer::test_support::lock_with_fresh_ring();
        let client = KEventClient::new_shared_memory("node_a").unwrap();
        let reader = client
            .create_event_reader(vec!["/system/node/online".to_string()])
            .await
            .unwrap();

        client
            .pub_event("/system/node/online", json!({"ok": true}))
            .await
            .unwrap();

        let event = reader.pull_event(Some(300)).await.unwrap().unwrap();
        assert_eq!(event.eventid, "/system/node/online");
        assert_eq!(event.ingress_node.as_deref(), Some("node_a"));
    }

    #[tokio::test]
    async fn test_full_mode_shared_ring_short_circuit_between_clients() {
        let _ring_guard = crate::kevent_ringbuffer::test_support::lock_with_fresh_ring();
        let publisher = KEventClient::new_shared_memory("node_a").unwrap();
        let subscriber = KEventClient::new_shared_memory("node_a").unwrap();
        let eventid = format!("/kevent/shared_ring/test_{}", now_millis());
        let reader = subscriber
            .create_event_reader(vec![eventid.clone()])
            .await
            .unwrap();

        publisher
            .pub_event(&eventid, json!({"path": "shared_ring"}))
            .await
            .unwrap();

        let event = reader.pull_event(Some(600)).await.unwrap().unwrap();
        assert_eq!(event.eventid, eventid);
        assert_eq!(event.data.get("path"), Some(&json!("shared_ring")));
    }

    #[tokio::test]
    async fn test_full_mode_shared_ring_first_event_from_late_producer() {
        let _ring_guard = crate::kevent_ringbuffer::test_support::lock_with_fresh_ring();
        let subscriber = KEventClient::new_shared_memory("node_a").unwrap();
        let eventid = format!("/kevent/shared_ring/late_producer_{}", now_millis());
        let reader = subscriber
            .create_event_reader(vec![eventid.clone()])
            .await
            .unwrap();

        let publisher = KEventClient::new_shared_memory("node_a").unwrap();
        publisher
            .pub_event(&eventid, json!({"path": "late_producer"}))
            .await
            .unwrap();

        let event = reader.pull_event(Some(600)).await.unwrap().unwrap();
        assert_eq!(event.eventid, eventid);
        assert_eq!(event.data.get("path"), Some(&json!("late_producer")));
    }
}
