use crate::kevent_ringbuffer::SharedKEventRingBuffer;
use crate::{AppDoc, AppType, SelectorType};
use async_trait::async_trait;
use log::warn;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::{oneshot, Mutex, Notify, RwLock};
use tokio::time::Instant;

pub const KEVENT_SERVICE_UNIQUE_ID: &str = "kevent";
pub const KEVENT_SERVICE_NAME: &str = "kevent";
pub const KEVENT_SERVICE_MAIN_PORT: u16 = 4041;
pub const DEFAULT_READER_CAPACITY: usize = 1024;
pub const MAX_EVENT_DATA_SIZE_BYTES: usize = 64 * 1024;
const SHARED_RING_DRAIN_BATCH: usize = 128;
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
    // Full SDK semantics. Global patterns/events use daemon bridge when provided.
    Full,
    // Light SDK semantics. Only global pub is supported.
    Light,
    // Local publish-only mode.
    LocalPubOnly,
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

#[async_trait]
pub trait KEventDaemonBridge: Send + Sync {
    async fn register_reader(&self, reader_id: &str, patterns: &[String]) -> KEventResult<()>;
    async fn unregister_reader(&self, reader_id: &str) -> KEventResult<()>;
    async fn publish_global(&self, event: &Event) -> KEventResult<()>;
}

#[derive(Clone)]
pub struct KEventClient {
    mode: KEventClientMode,
    source_node: String,
    bridge: Option<Arc<dyn KEventDaemonBridge>>,
    inner: Arc<KEventClientInner>,
}

struct KEventClientInner {
    readers: RwLock<HashMap<String, Arc<ReaderState>>>,
    timers: RwLock<HashMap<TimerId, oneshot::Sender<()>>>,
    shared_ring: Option<Arc<SharedKEventRingBuffer>>,
    reader_seq: AtomicU64,
    timer_seq: AtomicU64,
    reader_capacity: usize,
    /// Signaled by ShmDispatch after dispatching shared-ring events to
    /// reader queues.  `pull_event` waits on this instead of polling.
    shm_dispatch_notify: Notify,
    /// Set to true when the client is being dropped, to stop the
    /// ShmDispatch background thread.
    shm_dispatch_stop: AtomicBool,
}

struct ReaderState {
    patterns: Vec<String>,
    queue: Mutex<VecDeque<Event>>,
    notify: Notify,
    capacity: usize,
}

impl ReaderState {
    fn new(patterns: Vec<String>, capacity: usize) -> Self {
        Self {
            patterns,
            queue: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            capacity,
        }
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

impl KEventClientInner {
    async fn dispatch_event(&self, event: &Event) {
        let snapshot: Vec<Arc<ReaderState>> = self.readers.read().await.values().cloned().collect();
        for reader in snapshot {
            if reader_match_event(&reader.patterns, &event.eventid) {
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
            if reader_match_event(&reader.patterns, &event.eventid) {
                reader.push_sync(event.clone());
            }
        }
    }

    /// Drain events from the shared ring buffer and dispatch to matching
    /// readers (synchronous, for ShmDispatch thread).
    /// Returns the number of events dispatched.
    fn import_shared_events_sync(&self, max_events: usize) -> usize {
        let Some(shared_ring) = &self.shared_ring else {
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
    bridge: Option<Arc<dyn KEventDaemonBridge>>,
    mode: KEventClientMode,
    has_global_patterns: bool,
    closed: AtomicBool,
}

impl KEventClient {
    pub fn new_local(source_node: impl Into<String>) -> Self {
        Self::new_with_mode(
            source_node,
            KEventClientMode::Local,
            None,
            DEFAULT_READER_CAPACITY,
        )
    }

    pub fn new_full(
        source_node: impl Into<String>,
        bridge: Option<Arc<dyn KEventDaemonBridge>>,
    ) -> Self {
        Self::new_with_mode(
            source_node,
            KEventClientMode::Full,
            bridge,
            DEFAULT_READER_CAPACITY,
        )
    }

    pub fn new_light(source_node: impl Into<String>, bridge: Arc<dyn KEventDaemonBridge>) -> Self {
        Self::new_with_mode(
            source_node,
            KEventClientMode::Light,
            Some(bridge),
            DEFAULT_READER_CAPACITY,
        )
    }

    pub fn new_local_pub_only(source_node: impl Into<String>) -> Self {
        Self::new_with_mode(
            source_node,
            KEventClientMode::LocalPubOnly,
            None,
            DEFAULT_READER_CAPACITY,
        )
    }

    pub fn new_with_mode(
        source_node: impl Into<String>,
        mode: KEventClientMode,
        bridge: Option<Arc<dyn KEventDaemonBridge>>,
        reader_capacity: usize,
    ) -> Self {
        let shared_ring = if mode == KEventClientMode::Full {
            match SharedKEventRingBuffer::open() {
                Ok(shared_ring) => Some(Arc::new(shared_ring)),
                Err(err) => {
                    warn!("kevent shared ringbuffer is unavailable: {}", err);
                    None
                }
            }
        } else {
            None
        };

        let inner = Arc::new(KEventClientInner {
            readers: RwLock::new(HashMap::new()),
            timers: RwLock::new(HashMap::new()),
            shared_ring,
            reader_seq: AtomicU64::new(0),
            timer_seq: AtomicU64::new(0),
            reader_capacity: reader_capacity.max(1),
            shm_dispatch_notify: Notify::new(),
            shm_dispatch_stop: AtomicBool::new(false),
        });

        // Launch the ShmDispatch background thread when we have a shared ring.
        // This thread blocks on the futex/ulock in shared memory, wakes up on
        // new events, drains them, dispatches to reader queues, and notifies
        // pull_event waiters.  It replaces the old 5ms polling approach.
        //
        // We capture the tokio Handle here (on the caller's thread, which
        // is inside a tokio runtime) so the background OS thread can use
        // block_on to call async dispatch_event.
        if inner.shared_ring.is_some() {
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
            bridge,
            inner,
        }
    }

    pub fn mode(&self) -> KEventClientMode {
        self.mode
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

        if self.mode == KEventClientMode::Full
            && has_global_patterns
            && self.bridge.is_none()
            && self.inner.shared_ring.is_none()
        {
            return Err(KEventError::DaemonUnavailable(
                "global reader requires daemon bridge or shared ringbuffer in full mode"
                    .to_string(),
            ));
        }

        let reader_id = format!(
            "r_{}",
            self.inner.reader_seq.fetch_add(1, Ordering::Relaxed) + 1
        );
        let state = Arc::new(ReaderState::new(
            patterns.clone(),
            self.inner.reader_capacity.max(1),
        ));
        self.inner
            .readers
            .write()
            .await
            .insert(reader_id.clone(), state);

        if self.mode == KEventClientMode::Full && has_global_patterns {
            if let Some(shared_ring) = &self.inner.shared_ring {
                shared_ring.prime_cursors();
            }
        }

        if self.mode == KEventClientMode::Full && has_global_patterns {
            if let Some(bridge) = &self.bridge {
                if let Err(err) = bridge.register_reader(&reader_id, &patterns).await {
                    self.inner.readers.write().await.remove(&reader_id);
                    return Err(err);
                }
            }
        }

        Ok(EventReader {
            reader_id,
            inner: Arc::downgrade(&self.inner),
            bridge: self.bridge.clone(),
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
        self.dispatch_local(&event).await;

        match self.mode {
            KEventClientMode::Local => Ok(()),
            KEventClientMode::LocalPubOnly => Ok(()),
            KEventClientMode::Full => {
                if is_global {
                    let mut delivered_to_local_host = false;
                    if let Some(shared_ring) = &self.inner.shared_ring {
                        match shared_ring.publish_event(&event) {
                            Ok(_) => {
                                delivered_to_local_host = true;
                            }
                            Err(err) => {
                                warn!("publish global event to shared ringbuffer failed: {}", err);
                            }
                        }
                    }

                    if let Some(bridge) = &self.bridge {
                        bridge.publish_global(&event).await?;
                    } else if !delivered_to_local_host {
                        return Err(KEventError::DaemonUnavailable(
                            "global event requires daemon bridge or shared ringbuffer in full mode"
                                .to_string(),
                        ));
                    }
                }
                Ok(())
            }
            KEventClientMode::Light => {
                if !is_global {
                    return Err(KEventError::NotSupported(
                        "light mode only supports global event publishing".to_string(),
                    ));
                }
                let bridge = self.bridge.as_ref().ok_or_else(|| {
                    KEventError::DaemonUnavailable("light mode requires daemon bridge".to_string())
                })?;
                bridge.publish_global(&event).await?;
                Ok(())
            }
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

        let shared_ring = match &inner.shared_ring {
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

    pub async fn close(&self) -> KEventResult<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let Some(inner) = self.inner.upgrade() else {
            return Ok(());
        };
        let removed = inner.readers.write().await.remove(&self.reader_id);
        if removed.is_none() {
            return Ok(());
        }

        if self.mode == KEventClientMode::Full && self.has_global_patterns {
            if let Some(bridge) = &self.bridge {
                bridge.unregister_reader(&self.reader_id).await?;
            }
        }
        Ok(())
    }
}

impl Drop for EventReader {
    fn drop(&mut self) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let reader_id = self.reader_id.clone();
        let bridge = self.bridge.clone();
        let mode = self.mode;
        let has_global_patterns = self.has_global_patterns;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                inner.readers.write().await.remove(&reader_id);
                if mode == KEventClientMode::Full && has_global_patterns {
                    if let Some(bridge) = bridge {
                        let _ = bridge.unregister_reader(&reader_id).await;
                    }
                }
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

fn reader_match_event(patterns: &[String], eventid: &str) -> bool {
    for pattern in patterns {
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
    use crate::kevent_ringbuffer::DEFAULT_RINGBUFFER_PATH_ENV;
    use std::sync::Arc;
    use std::sync::Once;

    struct MockBridge {
        published: Arc<Mutex<Vec<Event>>>,
    }

    fn init_test_ringbuffer_path() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let path = std::env::temp_dir().join(format!(
                "buckyos_kevent_ringbuffer_test_{}.shm",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            std::env::set_var(DEFAULT_RINGBUFFER_PATH_ENV, &path);
        });
    }

    #[async_trait]
    impl KEventDaemonBridge for MockBridge {
        async fn register_reader(
            &self,
            _reader_id: &str,
            _patterns: &[String],
        ) -> KEventResult<()> {
            Ok(())
        }

        async fn unregister_reader(&self, _reader_id: &str) -> KEventResult<()> {
            Ok(())
        }

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
        init_test_ringbuffer_path();
        let client = KEventClient::new_full("node_a", None);
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
        init_test_ringbuffer_path();
        let publisher = KEventClient::new_full("node_a", None);
        let subscriber = KEventClient::new_full("node_a", None);
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
}
