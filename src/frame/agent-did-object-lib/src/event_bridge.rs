use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use buckyos_api::KEventClient;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use name_lib::{EventFrame, EventSubscription};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use url::Url;
use uuid::Uuid;

use crate::adapters::{
    AdapterEventTransport, AdapterRegistry, AdapterSubscribeEventRequest,
    AdapterUnsubscribeEventRequest, AgentObjectAdapter,
};
use crate::error::AgentDIDObjectError;
use crate::types::{EventBridgeSubscription, UnsubscribeEventInput};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventBridgeKey {
    pub adapter_id: String,
    pub object: String,
    pub event: String,
    pub filter_hash: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeState {
    Connecting,
    Subscribing,
    Active,
    Renewing,
    Closing,
    Closed,
    Failed,
}

#[derive(Clone)]
pub struct EventBridgeStart {
    pub key: EventBridgeKey,
    pub subscription: EventBridgeSubscription,
    pub remote_subscription_id: String,
    pub transport: Option<AdapterEventTransport>,
    pub sink: EventBridgeSink,
}

pub struct EventTransportStarted {
    pub handle: Box<dyn EventTransportHandle>,
    pub remote_subscription_id: Option<String>,
    pub object_did: Option<String>,
    pub expires_at: Option<String>,
    pub cursor: Option<String>,
}

#[async_trait]
pub trait EventTransport: Send + Sync {
    async fn start(
        &self,
        start: EventBridgeStart,
    ) -> Result<EventTransportStarted, AgentDIDObjectError>;
}

#[async_trait]
pub trait EventTransportHandle: Send + Sync {
    async fn stop(&self) -> Result<(), AgentDIDObjectError>;
}

pub struct NoopEventTransport;

#[async_trait]
impl EventTransport for NoopEventTransport {
    async fn start(
        &self,
        _start: EventBridgeStart,
    ) -> Result<EventTransportStarted, AgentDIDObjectError> {
        Ok(EventTransportStarted {
            handle: Box::new(NoopEventTransportHandle),
            remote_subscription_id: None,
            object_did: None,
            expires_at: None,
            cursor: None,
        })
    }
}

struct NoopEventTransportHandle;

#[async_trait]
impl EventTransportHandle for NoopEventTransportHandle {
    async fn stop(&self) -> Result<(), AgentDIDObjectError> {
        Ok(())
    }
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWrite = SplitSink<WsStream, Message>;

pub struct WebSocketEventTransport {
    subscription_timeout: Duration,
}

impl Default for WebSocketEventTransport {
    fn default() -> Self {
        Self {
            subscription_timeout: Duration::from_secs(10),
        }
    }
}

impl WebSocketEventTransport {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl EventTransport for WebSocketEventTransport {
    async fn start(
        &self,
        start: EventBridgeStart,
    ) -> Result<EventTransportStarted, AgentDIDObjectError> {
        let Some(AdapterEventTransport::WebSocket {
            endpoint,
            subscribe,
            unsubscribe,
        }) = start.transport.clone()
        else {
            return NoopEventTransport.start(start).await;
        };

        let (stream, _) = connect_async(&endpoint)
            .await
            .map_err(|err| AgentDIDObjectError::EventBridgeError(err.to_string()))?;
        let (mut write, mut read) = stream.split();
        write_ws_json(&mut write, &subscribe).await?;
        let remote = read_subscription_frame(&mut read, self.subscription_timeout).await?;
        if remote.object != start.subscription.object || remote.event != start.subscription.event {
            return Err(AgentDIDObjectError::ProtocolError(format!(
                "event subscription response mismatch: expected {} {}, got {} {}",
                start.subscription.object, start.subscription.event, remote.object, remote.event
            )));
        }

        let sink = start.sink.clone();
        let read_task = tokio::spawn(async move {
            while let Some(message) = read.next().await {
                let Ok(message) = message else {
                    break;
                };
                let Ok(Some(value)) = ws_message_json(message) else {
                    continue;
                };
                if value.get("type").and_then(Value::as_str) != Some("event") {
                    continue;
                }
                let Ok(frame) = serde_json::from_value::<EventFrame>(value) else {
                    continue;
                };
                if sink.publish_frame(frame).await.is_err() {
                    break;
                }
            }
        });

        Ok(EventTransportStarted {
            handle: Box::new(WebSocketEventTransportHandle {
                write: Mutex::new(write),
                read_task: Mutex::new(Some(read_task)),
                remote_subscription_id: remote.subscription_id.clone(),
                unsubscribe,
            }),
            remote_subscription_id: Some(remote.subscription_id),
            object_did: remote.object_did.as_ref().map(|did| did.to_string()),
            expires_at: Some(remote.expires_at),
            cursor: remote.cursor,
        })
    }
}

struct WebSocketEventTransportHandle {
    write: Mutex<WsWrite>,
    read_task: Mutex<Option<JoinHandle<()>>>,
    remote_subscription_id: String,
    unsubscribe: Option<Value>,
}

#[async_trait]
impl EventTransportHandle for WebSocketEventTransportHandle {
    async fn stop(&self) -> Result<(), AgentDIDObjectError> {
        let unsubscribe = self.unsubscribe.clone().unwrap_or_else(|| {
            json!({
                "op": "unsubscribe",
                "subscription_id": self.remote_subscription_id,
            })
        });
        {
            let mut write = self.write.lock().await;
            write_ws_json(&mut write, &unsubscribe).await?;
            write
                .send(Message::Close(None))
                .await
                .map_err(|err| AgentDIDObjectError::EventBridgeError(err.to_string()))?;
        }
        if let Some(task) = self.read_task.lock().await.take() {
            task.abort();
        }
        Ok(())
    }
}

async fn write_ws_json(write: &mut WsWrite, value: &Value) -> Result<(), AgentDIDObjectError> {
    let text = serde_json::to_string(value)
        .map_err(|err| AgentDIDObjectError::ProtocolError(err.to_string()))?;
    write
        .send(Message::Text(text.into()))
        .await
        .map_err(|err| AgentDIDObjectError::EventBridgeError(err.to_string()))
}

async fn read_subscription_frame(
    read: &mut futures_util::stream::SplitStream<WsStream>,
    timeout: Duration,
) -> Result<EventSubscription, AgentDIDObjectError> {
    loop {
        let message = tokio::time::timeout(timeout, read.next())
            .await
            .map_err(|_| {
                AgentDIDObjectError::EventBridgeError(
                    "websocket event subscription timed out".to_string(),
                )
            })?
            .ok_or_else(|| {
                AgentDIDObjectError::EventBridgeError(
                    "websocket closed before subscription response".to_string(),
                )
            })?
            .map_err(|err| AgentDIDObjectError::EventBridgeError(err.to_string()))?;
        let Some(value) = ws_message_json(message)? else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("subscription") => {
                let subscription = serde_json::from_value::<EventSubscription>(value)
                    .map_err(|err| AgentDIDObjectError::ProtocolError(err.to_string()))?;
                return Ok(subscription);
            }
            Some("error") => {
                return Err(AgentDIDObjectError::ProtocolError(value.to_string()));
            }
            _ => {}
        }
    }
}

fn ws_message_json(message: Message) -> Result<Option<Value>, AgentDIDObjectError> {
    match message {
        Message::Text(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|err| AgentDIDObjectError::ProtocolError(err.to_string())),
        Message::Binary(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|err| AgentDIDObjectError::ProtocolError(err.to_string())),
        Message::Close(_) => Err(AgentDIDObjectError::EventBridgeError(
            "websocket closed".to_string(),
        )),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Ok(None),
    }
}

#[derive(Clone)]
pub struct EventBridgeSink {
    key: EventBridgeKey,
    kevent_pattern: String,
    local_subscription_id: String,
    kevent_client: Option<Arc<KEventClient>>,
}

impl EventBridgeSink {
    fn new(
        key: EventBridgeKey,
        kevent_pattern: String,
        local_subscription_id: String,
        kevent_client: Option<Arc<KEventClient>>,
    ) -> Self {
        Self {
            key,
            kevent_pattern,
            local_subscription_id,
            kevent_client,
        }
    }

    pub async fn publish_frame(&self, frame: EventFrame) -> Result<(), AgentDIDObjectError> {
        let client = self.kevent_client.as_ref().ok_or_else(|| {
            AgentDIDObjectError::KEventError(
                "event bridge requires a KEventClient to publish frames".to_string(),
            )
        })?;
        let payload = event_frame_payload(
            &self.key,
            &self.local_subscription_id,
            &self.kevent_pattern,
            frame,
        );
        client
            .pub_event(&self.kevent_pattern, payload)
            .await
            .map_err(|err| AgentDIDObjectError::KEventError(err.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventBridgeSnapshot {
    pub key: EventBridgeKey,
    pub state: BridgeState,
    pub ref_count: usize,
    pub subscription: EventBridgeSubscription,
    pub remote_subscription_id: String,
}

struct BridgeEntry {
    key: EventBridgeKey,
    state: BridgeState,
    ref_count: usize,
    subscription: EventBridgeSubscription,
    remote_subscription_id: String,
    transport_handle: Box<dyn EventTransportHandle>,
    unsubscribe_via_adapter: bool,
}

#[derive(Default)]
struct BridgeTables {
    entries: HashMap<EventBridgeKey, BridgeEntry>,
    subscription_index: HashMap<String, EventBridgeKey>,
}

pub struct EventBridgeManager {
    tables: Mutex<BridgeTables>,
    transport: Arc<dyn EventTransport>,
    kevent_client: Option<Arc<KEventClient>>,
}

impl EventBridgeManager {
    pub fn new(kevent_client: Option<Arc<KEventClient>>) -> Self {
        Self::with_transport(kevent_client, Arc::new(WebSocketEventTransport::new()))
    }

    pub fn with_transport(
        kevent_client: Option<Arc<KEventClient>>,
        transport: Arc<dyn EventTransport>,
    ) -> Self {
        Self {
            tables: Mutex::new(BridgeTables::default()),
            transport,
            kevent_client,
        }
    }

    pub async fn subscribe(
        &self,
        adapter: Arc<dyn AgentObjectAdapter>,
        req: AdapterSubscribeEventRequest,
    ) -> Result<EventBridgeSubscription, AgentDIDObjectError> {
        let key = bridge_key(&req);
        let mut tables = self.tables.lock().await;
        if let Some(entry) = tables.entries.get_mut(&key) {
            entry.ref_count += 1;
            return Ok(entry.subscription.clone());
        }

        let adapter_subscription = adapter.subscribe_event(req.clone()).await?;
        let unsubscribe_via_adapter = adapter_subscription.unsubscribe_via_adapter;
        let transport = adapter_subscription.transport.clone();
        let mut subscription = adapter_subscription.subscription;
        let remote_subscription_id = subscription.subscription_id.clone();
        subscription.subscription_id = new_subscription_id();
        subscription.kevent_pattern =
            encode_object_event_id(&subscription.object, &subscription.event);
        subscription.route = req.route_trace.clone();

        let sink = EventBridgeSink::new(
            key.clone(),
            subscription.kevent_pattern.clone(),
            subscription.subscription_id.clone(),
            self.kevent_client.clone(),
        );
        let start = EventBridgeStart {
            key: key.clone(),
            subscription: subscription.clone(),
            remote_subscription_id: remote_subscription_id.clone(),
            transport,
            sink,
        };
        let started = match self.transport.start(start).await {
            Ok(started) => started,
            Err(err) => {
                if unsubscribe_via_adapter {
                    let _ = adapter
                        .unsubscribe_event(AdapterUnsubscribeEventRequest {
                            input: UnsubscribeEventInput {
                                subscription_id: remote_subscription_id,
                            },
                        })
                        .await;
                }
                return Err(err);
            }
        };
        let remote_subscription_id = started
            .remote_subscription_id
            .unwrap_or(remote_subscription_id);
        if let Some(object_did) = started.object_did {
            subscription.object_did = Some(object_did);
        }
        if let Some(expires_at) = started.expires_at {
            subscription.expires_at = Some(expires_at);
        }
        if let Some(cursor) = started.cursor {
            subscription.cursor = Some(cursor);
        }

        tables
            .subscription_index
            .insert(subscription.subscription_id.clone(), key.clone());
        tables.entries.insert(
            key.clone(),
            BridgeEntry {
                key,
                state: BridgeState::Active,
                ref_count: 1,
                subscription: subscription.clone(),
                remote_subscription_id,
                transport_handle: started.handle,
                unsubscribe_via_adapter,
            },
        );
        Ok(subscription)
    }

    pub async fn unsubscribe(
        &self,
        registry: &AdapterRegistry,
        input: UnsubscribeEventInput,
    ) -> Result<(), AgentDIDObjectError> {
        let close_entry = {
            let mut tables = self.tables.lock().await;
            let Some(key) = tables
                .subscription_index
                .get(&input.subscription_id)
                .cloned()
            else {
                return Ok(());
            };
            let Some(entry) = tables.entries.get_mut(&key) else {
                tables.subscription_index.remove(&input.subscription_id);
                return Ok(());
            };
            if entry.ref_count > 1 {
                entry.ref_count -= 1;
                return Ok(());
            }
            entry.state = BridgeState::Closing;
            tables.subscription_index.remove(&input.subscription_id);
            tables.entries.remove(&key)
        };

        if let Some(mut entry) = close_entry {
            entry.transport_handle.stop().await?;
            entry.state = BridgeState::Closed;
            if entry.unsubscribe_via_adapter {
                let adapter = registry
                    .get(&entry.subscription.route.adapter)
                    .ok_or_else(|| {
                        AgentDIDObjectError::AdapterNotFound(
                            entry.subscription.route.adapter.clone(),
                        )
                    })?;
                adapter
                    .unsubscribe_event(AdapterUnsubscribeEventRequest {
                        input: UnsubscribeEventInput {
                            subscription_id: entry.remote_subscription_id,
                        },
                    })
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn snapshot(&self, key: &EventBridgeKey) -> Option<EventBridgeSnapshot> {
        let tables = self.tables.lock().await;
        tables.entries.get(key).map(|entry| EventBridgeSnapshot {
            key: entry.key.clone(),
            state: entry.state,
            ref_count: entry.ref_count,
            subscription: entry.subscription.clone(),
            remote_subscription_id: entry.remote_subscription_id.clone(),
        })
    }

    pub async fn active_count(&self) -> usize {
        self.tables.lock().await.entries.len()
    }
}

pub fn bridge_key(req: &AdapterSubscribeEventRequest) -> EventBridgeKey {
    EventBridgeKey {
        adapter_id: req.route.adapter.clone(),
        object: req.object_ref.normalized.clone(),
        event: req.input.event.clone(),
        filter_hash: filter_hash(&req.input.filter),
    }
}

pub fn filter_hash(filter: &Value) -> String {
    let encoded = serde_json::to_string(filter).unwrap_or_else(|_| "null".to_string());
    short_hash(&encoded)
}

pub fn event_frame_payload(
    key: &EventBridgeKey,
    local_subscription_id: &str,
    kevent_pattern: &str,
    frame: EventFrame,
) -> Value {
    let object = frame.object.clone();
    let object_did = frame
        .object_did
        .as_ref()
        .map(|did| serde_json::to_value(did).unwrap_or(Value::Null));
    let event = frame.event.clone();
    let cursor = frame.cursor.clone();
    let seq = frame.seq;
    let summary = frame.summary.clone();
    let remote_subscription_id = frame.subscription_id.clone();
    json!({
        "protocol": "agent-did-object-event-bridge/1",
        "bridge_key": key,
        "subscription_id": local_subscription_id,
        "remote_subscription_id": remote_subscription_id,
        "kevent_pattern": kevent_pattern,
        "object": object,
        "object_did": object_did,
        "event": event,
        "cursor": cursor,
        "seq": seq,
        "summary": summary,
        "received_at_ms": now_millis(),
        "frame": frame,
    })
}

pub fn encode_object_event_id(object: &str, event: &str) -> String {
    let event = safe_segment(event);
    if let Ok(url) = Url::parse(object) {
        if let Some(host) = url.host_str() {
            let mut segments = url
                .path_segments()
                .map(|segments| {
                    segments
                        .filter(|segment| !segment.trim().is_empty())
                        .map(safe_segment)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if segments.is_empty() {
                segments.push("by_hash".to_string());
                segments.push(short_hash(object));
            }
            return format!(
                "/obj/{}/{}/{}",
                safe_segment(&host.to_lowercase()),
                segments.join("/"),
                event
            );
        }
    }
    format!("/obj/by_hash/{}/{}", short_hash(object), event)
}

fn safe_segment(input: &str) -> String {
    let mut value = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let first = chars.peek().copied();
            if first.is_some_and(|value| value.is_ascii_hexdigit()) {
                chars.next();
                let second = chars.peek().copied();
                if second.is_some_and(|value| value.is_ascii_hexdigit()) {
                    chars.next();
                }
            }
            value.push('_');
        } else if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            value.push(ch);
        } else {
            value.push('_');
        }
    }
    if value.is_empty() {
        "_".to_string()
    } else {
        value
    }
}

fn short_hash(input: &str) -> String {
    let hash = Sha256::digest(input.as_bytes());
    hash.iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn new_subscription_id() -> String {
    format!("sub_{}", Uuid::new_v4().simple())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::accept_async;

    use crate::adapters::{
        AdapterEventSubscription, AdapterReadRequest, AdapterReadResponse,
        AdapterSubscribeEventRequest, AdapterXCallRequest, AdapterXCallResponse,
    };
    use crate::config::{AdapterConfig, AdapterType, ObjectRoute};
    use crate::router::{RouteMatchType, RouteMethod, RouteTrace};
    use crate::types::{ObjectRef, SubscribeEventInput};

    #[test]
    fn encodes_url_path_segments() {
        assert_eq!(
            encode_object_event_id("https://MyHome.com/devices/cam 01", "low battery"),
            "/obj/myhome.com/devices/cam_01/low_battery"
        );
    }

    #[test]
    fn falls_back_to_hash_for_empty_path() {
        let eventid = encode_object_event_id("https://example.com", "changed");
        assert!(eventid.starts_with("/obj/example.com/by_hash/"));
        assert!(eventid.ends_with("/changed"));
    }

    struct FakeAdapter {
        subscribe_count: Arc<AtomicUsize>,
        unsubscribe_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl AgentObjectAdapter for FakeAdapter {
        fn id(&self) -> &str {
            "fake"
        }

        async fn read(
            &self,
            _req: AdapterReadRequest,
        ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
            unreachable!()
        }

        async fn x_call(
            &self,
            _req: AdapterXCallRequest,
        ) -> Result<AdapterXCallResponse, AgentDIDObjectError> {
            unreachable!()
        }

        async fn subscribe_event(
            &self,
            req: AdapterSubscribeEventRequest,
        ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
            self.subscribe_count.fetch_add(1, Ordering::SeqCst);
            Ok(AdapterEventSubscription {
                subscription: EventBridgeSubscription {
                    subscription_id: "remote-sub".to_string(),
                    object: req.object_ref.normalized.clone(),
                    object_did: None,
                    event: req.input.event.clone(),
                    kevent_pattern: "remote-pattern".to_string(),
                    expires_at: Some("2026-06-07T13:00:00Z".to_string()),
                    cursor: req.input.cursor.clone(),
                    route: req.route_trace,
                },
                transport: None,
                unsubscribe_via_adapter: true,
            })
        }

        async fn unsubscribe_event(
            &self,
            req: AdapterUnsubscribeEventRequest,
        ) -> Result<(), AgentDIDObjectError> {
            assert_eq!(req.input.subscription_id, "remote-sub");
            self.unsubscribe_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeTransport {
        starts: AtomicUsize,
        stops: Arc<AtomicUsize>,
        sink: Mutex<Option<EventBridgeSink>>,
    }

    #[async_trait]
    impl EventTransport for FakeTransport {
        async fn start(
            &self,
            start: EventBridgeStart,
        ) -> Result<EventTransportStarted, AgentDIDObjectError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            *self.sink.lock().await = Some(start.sink);
            Ok(EventTransportStarted {
                handle: Box::new(FakeTransportHandle {
                    stops: self.stops.clone(),
                }),
                remote_subscription_id: None,
                object_did: None,
                expires_at: None,
                cursor: None,
            })
        }
    }

    struct FakeTransportHandle {
        stops: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EventTransportHandle for FakeTransportHandle {
        async fn stop(&self) -> Result<(), AgentDIDObjectError> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn subscribe_req() -> AdapterSubscribeEventRequest {
        let object = "https://example.com/devices/cam01".to_string();
        AdapterSubscribeEventRequest {
            object_ref: ObjectRef::parse(&object).unwrap(),
            input: SubscribeEventInput {
                object,
                event: "changed".to_string(),
                filter: json!({"level": "low"}),
                session_id: Some("s1".to_string()),
                ttl_ms: Some(300000),
                cursor: None,
                trace_id: Some("t1".to_string()),
            },
            route: ObjectRoute {
                id: "r".to_string(),
                priority: 0,
                match_type: RouteMatchType::Scheme,
                pattern: "https".to_string(),
                adapter: "fake".to_string(),
                methods: vec![RouteMethod::SubscribeEvent],
                options: json!({}),
            },
            route_trace: RouteTrace {
                route_id: "r".to_string(),
                adapter: "fake".to_string(),
                method: RouteMethod::SubscribeEvent,
            },
            adapter_config: AdapterConfig {
                id: "fake".to_string(),
                adapter_type: AdapterType::Web,
                endpoint: None,
                auth_token_env: None,
                options: json!({}),
            },
        }
    }

    #[tokio::test]
    async fn ref_count_starts_once_and_closes_on_last_unsubscribe() {
        let subscribe_count = Arc::new(AtomicUsize::new(0));
        let unsubscribe_count = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(FakeAdapter {
            subscribe_count: subscribe_count.clone(),
            unsubscribe_count: unsubscribe_count.clone(),
        });
        let transport = Arc::new(FakeTransport::default());
        let manager = EventBridgeManager::with_transport(None, transport.clone());
        let req = subscribe_req();
        let key = bridge_key(&req);

        let first = manager
            .subscribe(adapter.clone(), req.clone())
            .await
            .unwrap();
        let second = manager.subscribe(adapter, req).await.unwrap();
        assert_eq!(first.subscription_id, second.subscription_id);
        assert_eq!(subscribe_count.load(Ordering::SeqCst), 1);
        assert_eq!(transport.starts.load(Ordering::SeqCst), 1);
        assert_eq!(manager.snapshot(&key).await.unwrap().ref_count, 2);

        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(FakeAdapter {
            subscribe_count,
            unsubscribe_count: unsubscribe_count.clone(),
        }));
        manager
            .unsubscribe(
                &registry,
                UnsubscribeEventInput {
                    subscription_id: first.subscription_id.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(manager.snapshot(&key).await.unwrap().ref_count, 1);
        assert_eq!(unsubscribe_count.load(Ordering::SeqCst), 0);

        manager
            .unsubscribe(
                &registry,
                UnsubscribeEventInput {
                    subscription_id: first.subscription_id,
                },
            )
            .await
            .unwrap();
        assert!(manager.snapshot(&key).await.is_none());
        assert_eq!(unsubscribe_count.load(Ordering::SeqCst), 1);
        assert_eq!(transport.stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fake_transport_publishes_event_frame_to_kevent() {
        let subscribe_count = Arc::new(AtomicUsize::new(0));
        let unsubscribe_count = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(FakeAdapter {
            subscribe_count,
            unsubscribe_count,
        });
        let transport = Arc::new(FakeTransport::default());
        let kevent_client = Arc::new(KEventClient::new_local("agent-did-object-test"));
        let manager =
            EventBridgeManager::with_transport(Some(kevent_client.clone()), transport.clone());

        let subscription = manager
            .subscribe(adapter, subscribe_req())
            .await
            .expect("subscribe");
        let reader = kevent_client
            .create_event_reader(vec![subscription.kevent_pattern.clone()])
            .await
            .expect("reader");
        let sink = transport.sink.lock().await.clone().expect("transport sink");
        sink.publish_frame(EventFrame {
            frame_type: "event".to_string(),
            event_id: Some("evt_1".to_string()),
            subscription_id: "remote-sub".to_string(),
            object: subscription.object.clone(),
            object_did: None,
            event: subscription.event.clone(),
            seq: Some(42),
            cursor: Some("42".to_string()),
            timestamp: "2026-06-07T12:00:00Z".to_string(),
            summary: Some("changed".to_string()),
            data: json!({"battery": 12}),
            affected_objects: vec![subscription.object.clone()],
            invalidated_objects: vec![],
            refresh_hints: vec![subscription.object.clone()],
        })
        .await
        .expect("publish frame");

        let event = reader
            .pull_event(Some(1000))
            .await
            .expect("pull event")
            .expect("event");
        assert_eq!(event.eventid, subscription.kevent_pattern);
        assert_eq!(event.data["protocol"], "agent-did-object-event-bridge/1");
        assert_eq!(event.data["frame"]["event_id"], "evt_1");
        assert_eq!(event.data["frame"]["data"]["battery"], 12);
        assert_eq!(event.data["remote_subscription_id"], "remote-sub");
    }

    struct WebSocketAdapter {
        endpoint: String,
    }

    #[async_trait]
    impl AgentObjectAdapter for WebSocketAdapter {
        fn id(&self) -> &str {
            "ws"
        }

        async fn read(
            &self,
            _req: AdapterReadRequest,
        ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
            unreachable!()
        }

        async fn x_call(
            &self,
            _req: AdapterXCallRequest,
        ) -> Result<AdapterXCallResponse, AgentDIDObjectError> {
            unreachable!()
        }

        async fn subscribe_event(
            &self,
            req: AdapterSubscribeEventRequest,
        ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
            Ok(AdapterEventSubscription {
                subscription: EventBridgeSubscription {
                    subscription_id: "pending-remote".to_string(),
                    object: req.object_ref.normalized.clone(),
                    object_did: None,
                    event: req.input.event.clone(),
                    kevent_pattern: encode_object_event_id(
                        &req.object_ref.normalized,
                        &req.input.event,
                    ),
                    expires_at: None,
                    cursor: None,
                    route: req.route_trace,
                },
                transport: Some(AdapterEventTransport::WebSocket {
                    endpoint: self.endpoint.clone(),
                    subscribe: json!({
                        "op": "subscribe",
                        "object": req.object_ref.normalized,
                        "event": req.input.event,
                        "filter": req.input.filter,
                    }),
                    unsubscribe: None,
                }),
                unsubscribe_via_adapter: false,
            })
        }

        async fn unsubscribe_event(
            &self,
            _req: AdapterUnsubscribeEventRequest,
        ) -> Result<(), AgentDIDObjectError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn websocket_transport_subscribes_streams_and_unsubscribes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (send_event_tx, send_event_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let subscribe_msg = ws.next().await.unwrap().unwrap();
            let subscribe_json = ws_message_json(subscribe_msg).unwrap().unwrap();
            assert_eq!(subscribe_json["op"], "subscribe");
            assert_eq!(subscribe_json["event"], "changed");

            ws.send(Message::Text(
                json!({
                    "type": "subscription",
                    "subscription_id": "remote-ws-sub",
                    "object": "https://example.com/devices/cam01",
                    "event": "changed",
                    "expires_at": "2026-06-07T13:00:00Z",
                    "cursor": "41"
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

            send_event_rx.await.unwrap();
            ws.send(Message::Text(
                json!({
                    "type": "event",
                    "event_id": "evt-ws-1",
                    "subscription_id": "remote-ws-sub",
                    "object": "https://example.com/devices/cam01",
                    "event": "changed",
                    "seq": 42,
                    "cursor": "42",
                    "timestamp": "2026-06-07T12:00:00Z",
                    "summary": "changed",
                    "data": {"battery": 9},
                    "affected_objects": ["https://example.com/devices/cam01"],
                    "invalidated_objects": [],
                    "refresh_hints": ["https://example.com/devices/cam01"]
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

            let unsubscribe_msg = ws.next().await.unwrap().unwrap();
            let unsubscribe_json = ws_message_json(unsubscribe_msg).unwrap().unwrap();
            assert_eq!(unsubscribe_json["op"], "unsubscribe");
            assert_eq!(unsubscribe_json["subscription_id"], "remote-ws-sub");
        });

        let kevent_client = Arc::new(KEventClient::new_local("agent-did-object-ws-test"));
        let manager = EventBridgeManager::new(Some(kevent_client.clone()));
        let req = subscribe_req();
        let key = bridge_key(&req);
        let subscription = manager
            .subscribe(
                Arc::new(WebSocketAdapter {
                    endpoint: format!("ws://{addr}"),
                }),
                req,
            )
            .await
            .unwrap();
        assert_eq!(
            subscription.expires_at.as_deref(),
            Some("2026-06-07T13:00:00Z")
        );
        assert_eq!(
            manager.snapshot(&key).await.unwrap().remote_subscription_id,
            "remote-ws-sub"
        );

        let reader = kevent_client
            .create_event_reader(vec![subscription.kevent_pattern.clone()])
            .await
            .unwrap();
        send_event_tx.send(()).unwrap();
        let event = reader.pull_event(Some(1000)).await.unwrap().unwrap();
        assert_eq!(event.eventid, subscription.kevent_pattern);
        assert_eq!(event.data["frame"]["event_id"], "evt-ws-1");
        assert_eq!(event.data["frame"]["data"]["battery"], 9);

        manager
            .unsubscribe(
                &AdapterRegistry::new(),
                UnsubscribeEventInput {
                    subscription_id: subscription.subscription_id,
                },
            )
            .await
            .unwrap();
        server.await.unwrap();
    }
}
