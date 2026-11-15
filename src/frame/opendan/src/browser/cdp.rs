//! A Chrome DevTools Protocol client over the browser's debugging WebSocket:
//! requests paired with their answers by id, events broadcast to whoever
//! subscribed, tabs reached through flat sessions (`sessionId` on the same
//! connection).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

/// How long a command waits for its answer unless the caller says otherwise.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_BUFFER: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub enum CdpError {
    /// The connection is gone (the browser stopped or crashed).
    Closed,
    Timeout(String),
    /// The browser refused the command.
    Remote { code: i64, message: String },
    Transport(String),
}

impl std::fmt::Display for CdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CdpError::Closed => write!(f, "the browser connection is closed"),
            CdpError::Timeout(method) => write!(f, "{method} was not answered in time"),
            CdpError::Remote { code, message } => write!(f, "{message} ({code})"),
            CdpError::Transport(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CdpEvent {
    /// `None`: a browser level event.
    pub session_id: Option<String>,
    pub method: String,
    pub params: Value,
}

/// Which events a subscriber takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Any,
    /// Events of the browser connection itself (`Target.*`, `Browser.*`).
    Browser,
    /// Events of one flat session (one tab).
    Session(String),
}

/// A subscriber's view of the event broadcast. Events that arrived before
/// the subscription are not seen; a subscriber that falls more than the
/// buffer behind loses the oldest ones (`lagged` counts them).
pub struct EventStream {
    rx: broadcast::Receiver<Arc<CdpEvent>>,
    closed: watch::Receiver<bool>,
    scope: Scope,
    methods: Vec<String>,
    pub lagged: u64,
}

impl EventStream {
    fn takes(&self, ev: &CdpEvent) -> bool {
        let scope = match &self.scope {
            Scope::Any => true,
            Scope::Browser => ev.session_id.is_none(),
            Scope::Session(s) => ev.session_id.as_deref() == Some(s.as_str()),
        };
        scope && (self.methods.is_empty() || self.methods.iter().any(|m| m == &ev.method))
    }

    /// The next event of the subscription; `None` once the connection is closed.
    pub async fn next(&mut self) -> Option<Arc<CdpEvent>> {
        loop {
            let received = tokio::select! {
                biased;
                received = self.rx.recv() => received,
                _ = self.closed.wait_for(|c| *c) => return None,
            };
            match received {
                Ok(ev) if self.takes(&ev) => return Some(ev),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => self.lagged += n,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>;

pub struct CdpClient {
    next_id: AtomicU64,
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    events: broadcast::Sender<Arc<CdpEvent>>,
    closed: watch::Receiver<bool>,
}

impl std::fmt::Debug for CdpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpClient").field("closed", &self.is_closed()).finish()
    }
}

impl CdpClient {
    /// Connect to a debugging endpoint (`ws://127.0.0.1:<port>/devtools/browser/<id>`).
    pub async fn connect(url: &str) -> Result<Arc<Self>, CdpError> {
        // Screenshots and screencast frames travel as one message each.
        let mut config = WebSocketConfig::default();
        config.max_message_size = None;
        config.max_frame_size = None;
        let (socket, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), true)
            .await
            .map_err(|e| CdpError::Transport(format!("connect {url}: {e}")))?;
        let (mut sink, mut stream) = socket.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<String>();
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let (closed_tx, closed) = watch::channel(false);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        tokio::spawn(async move {
            while let Some(text) = outgoing.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });
        let (answers, broadcast_to) = (pending.clone(), events.clone());
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let Message::Text(text) = message else { continue };
                let Ok(mut v) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                match v.get("id").and_then(Value::as_u64) {
                    Some(id) => {
                        let Some(waiter) = answers.lock().expect("cdp pending").remove(&id) else { continue };
                        let answer = match v.get("error") {
                            Some(e) => Err(CdpError::Remote {
                                code: e["code"].as_i64().unwrap_or(0),
                                message: e["message"].as_str().unwrap_or("error").to_string(),
                            }),
                            None => Ok(v.get_mut("result").map(Value::take).unwrap_or(Value::Null)),
                        };
                        let _ = waiter.send(answer);
                    }
                    None => {
                        let Some(method) = v["method"].as_str().map(str::to_string) else { continue };
                        let _ = broadcast_to.send(Arc::new(CdpEvent {
                            session_id: v["sessionId"].as_str().map(str::to_string),
                            method,
                            params: v.get_mut("params").map(Value::take).unwrap_or(Value::Null),
                        }));
                    }
                }
            }
            for (_, waiter) in answers.lock().expect("cdp pending").drain() {
                let _ = waiter.send(Err(CdpError::Closed));
            }
            let _ = closed_tx.send(true);
        });
        Ok(Arc::new(Self { next_id: AtomicU64::new(1), out, pending, events, closed }))
    }

    fn write(&self, session: Option<&str>, method: &str, params: Value, waiter: Option<oneshot::Sender<Result<Value, CdpError>>>) -> Result<u64, CdpError> {
        if self.is_closed() {
            return Err(CdpError::Closed);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        if let Some(waiter) = waiter {
            self.pending.lock().expect("cdp pending").insert(id, waiter);
        }
        if self.out.send(message.to_string()).is_err() {
            self.pending.lock().expect("cdp pending").remove(&id);
            return Err(CdpError::Closed);
        }
        Ok(id)
    }

    /// Send a command to the browser (`session = None`) or to a tab's flat
    /// session, and wait for its answer.
    pub async fn send(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, CdpError> {
        self.send_within(session, method, params, COMMAND_TIMEOUT).await
    }

    pub async fn send_within(&self, session: Option<&str>, method: &str, params: Value, within: Duration) -> Result<Value, CdpError> {
        let (waiter, answer) = oneshot::channel();
        let id = self.write(session, method, params, Some(waiter))?;
        let mut closed = self.closed.clone();
        tokio::select! {
            answer = answer => answer.unwrap_or(Err(CdpError::Closed)),
            _ = closed.wait_for(|c| *c) => Err(CdpError::Closed),
            _ = tokio::time::sleep(within) => {
                self.pending.lock().expect("cdp pending").remove(&id);
                Err(CdpError::Timeout(method.to_string()))
            }
        }
    }

    /// Send a command without waiting for its answer (an input that opens a
    /// JS dialog is not answered until the dialog is closed).
    pub fn post(&self, session: Option<&str>, method: &str, params: Value) -> Result<(), CdpError> {
        self.write(session, method, params, None).map(|_| ())
    }

    /// Subscribe to events from now on: of `scope`, named by `methods`
    /// (empty: every method).
    pub fn subscribe(&self, scope: Scope, methods: &[&str]) -> EventStream {
        EventStream {
            rx: self.events.subscribe(),
            closed: self.closed.clone(),
            scope,
            methods: methods.iter().map(|m| m.to_string()).collect(),
            lagged: 0,
        }
    }

    /// The raw broadcast (a subscriber that must notice lost events).
    pub fn events(&self) -> broadcast::Receiver<Arc<CdpEvent>> {
        self.events.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    /// Returns once the connection is closed.
    pub async fn closed(&self) {
        let _ = self.closed.clone().wait_for(|c| *c).await;
    }
}
