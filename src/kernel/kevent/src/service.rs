use async_trait::async_trait;
use buckyos_api::{
    is_global_eventid, is_global_pattern, match_event_patterns, validate_event_data_size,
    validate_eventid, validate_pattern, Event, KEventDaemonRequest, KEventDaemonResponse,
    KEventError, KEventResult,
};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::time::{timeout, Instant};

pub const DEFAULT_DAEMON_READER_CAPACITY: usize = 1024;

#[async_trait]
pub trait KEventPeerPublisher: Send + Sync {
    async fn broadcast(&self, event: &Event) -> KEventResult<()>;
}

#[derive(Clone)]
pub struct KEventService {
    source_node: String,
    reader_capacity: usize,
    readers: Arc<RwLock<HashMap<String, Arc<ServiceReaderState>>>>,
    peers: Arc<RwLock<Vec<Arc<dyn KEventPeerPublisher>>>>,
}

struct ServiceReaderState {
    patterns: Vec<String>,
    queue: Mutex<VecDeque<Event>>,
    notify: Notify,
    capacity: usize,
}

impl ServiceReaderState {
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
        }
    }

    pub fn source_node(&self) -> &str {
        &self.source_node
    }

    pub async fn add_peer_publisher(&self, peer: Arc<dyn KEventPeerPublisher>) {
        self.peers.write().await.push(peer);
    }

    pub async fn register_reader(
        &self,
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

        self.readers.write().await.insert(
            reader_id.to_string(),
            Arc::new(ServiceReaderState::new(patterns, self.reader_capacity)),
        );
        Ok(())
    }

    pub async fn unregister_reader(&self, reader_id: &str) {
        self.readers.write().await.remove(reader_id);
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
        self.publish_external_global(event).await
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
        self.distribute(&event).await;
        Ok(())
    }

    pub async fn pull_event(
        &self,
        reader_id: &str,
        timeout_ms: Option<u64>,
    ) -> KEventResult<Option<Event>> {
        let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
        loop {
            let reader = {
                let readers = self.readers.read().await;
                readers.get(reader_id).cloned()
            };

            let Some(reader) = reader else {
                return Ok(None);
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
        match req {
            KEventDaemonRequest::RegisterReader {
                reader_id,
                patterns,
            } => match self.register_reader(&reader_id, patterns).await {
                Ok(_) => KEventDaemonResponse::Ok { event: None },
                Err(err) => err_to_response(err),
            },
            KEventDaemonRequest::UnregisterReader { reader_id } => {
                self.unregister_reader(&reader_id).await;
                KEventDaemonResponse::Ok { event: None }
            }
            KEventDaemonRequest::PublishGlobal { event } => {
                match self.publish_external_global(event).await {
                    Ok(_) => KEventDaemonResponse::Ok { event: None },
                    Err(err) => err_to_response(err),
                }
            }
            KEventDaemonRequest::PullEvent {
                reader_id,
                timeout_ms,
            } => match self.pull_event(&reader_id, timeout_ms).await {
                Ok(event) => KEventDaemonResponse::Ok { event },
                Err(err) => err_to_response(err),
            },
        }
    }

    async fn distribute(&self, event: &Event) {
        let snapshot: Vec<Arc<ServiceReaderState>> =
            self.readers.read().await.values().cloned().collect();
        for reader in snapshot {
            if match_event_patterns(&reader.patterns, &event.eventid) {
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
}
