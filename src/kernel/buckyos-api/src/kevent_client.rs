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
use tokio::time::{timeout, Instant};

pub const KEVENT_SERVICE_UNIQUE_ID: &str = "kevent";
pub const KEVENT_SERVICE_NAME: &str = "kevent";
pub const KEVENT_SERVICE_MAIN_PORT: u16 = 4041;
pub const DEFAULT_READER_CAPACITY: usize = 1024;
pub const MAX_EVENT_DATA_SIZE_BYTES: usize = 64 * 1024;

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
    reader_seq: AtomicU64,
    timer_seq: AtomicU64,
    reader_capacity: usize,
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

    async fn pop(&self) -> Option<Event> {
        let mut queue = self.queue.lock().await;
        queue.pop_front()
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
        Self {
            mode,
            source_node: source_node.into(),
            bridge,
            inner: Arc::new(KEventClientInner {
                readers: RwLock::new(HashMap::new()),
                timers: RwLock::new(HashMap::new()),
                reader_seq: AtomicU64::new(0),
                timer_seq: AtomicU64::new(0),
                reader_capacity: reader_capacity.max(1),
            }),
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

        if self.mode == KEventClientMode::Full && has_global_patterns && self.bridge.is_none() {
            return Err(KEventError::DaemonUnavailable(
                "global reader requires daemon bridge in full mode".to_string(),
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
                    let bridge = self.bridge.as_ref().ok_or_else(|| {
                        KEventError::DaemonUnavailable(
                            "global event requires daemon bridge in full mode".to_string(),
                        )
                    })?;
                    bridge.publish_global(&event).await?;
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
    pub async fn ingest_global_event(&self, event: Event) -> KEventResult<()> {
        if !is_global_eventid(&event.eventid) {
            return Err(KEventError::InvalidEventId(
                "ingest_global_event only accepts global eventid".to_string(),
            ));
        }
        validate_eventid(&event.eventid)?;
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
        let snapshot: Vec<Arc<ReaderState>> =
            self.inner.readers.read().await.values().cloned().collect();
        for reader in snapshot {
            if reader_match_event(&reader.patterns, &event.eventid) {
                reader.push(event.clone()).await;
            }
        }
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

            match deadline {
                None => {
                    state.notify.notified().await;
                }
                Some(deadline_at) => {
                    let now = Instant::now();
                    if now >= deadline_at {
                        return Ok(None);
                    }
                    let remain = deadline_at - now;
                    if timeout(remain, state.notify.notified()).await.is_err() {
                        return Ok(None);
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
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
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
    use std::sync::Arc;

    struct MockBridge {
        published: Arc<Mutex<Vec<Event>>>,
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
}
