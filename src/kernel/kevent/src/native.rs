use crate::{KEventPeerPublisher, KEventService};
use async_trait::async_trait;
use buckyos_api::{
    Event, KEventDaemonBridge, KEventDaemonRequest, KEventDaemonResponse, KEventError, KEventResult,
};
use std::sync::Arc;

#[async_trait]
pub trait KEventProtocolTransport: Send + Sync {
    async fn call(&self, req: KEventDaemonRequest) -> KEventResult<KEventDaemonResponse>;
}

#[derive(Clone)]
pub struct KEventDaemonClient {
    transport: Arc<dyn KEventProtocolTransport>,
}

impl KEventDaemonClient {
    pub fn new(transport: Arc<dyn KEventProtocolTransport>) -> Self {
        Self { transport }
    }

    pub async fn call(&self, req: KEventDaemonRequest) -> KEventResult<KEventDaemonResponse> {
        self.transport.call(req).await
    }

    pub async fn register_reader(&self, reader_id: &str, patterns: &[String]) -> KEventResult<()> {
        map_response_unit(
            self.call(KEventDaemonRequest::RegisterReader {
                reader_id: reader_id.to_string(),
                patterns: patterns.to_vec(),
            })
            .await?,
        )
    }

    pub async fn unregister_reader(&self, reader_id: &str) -> KEventResult<()> {
        map_response_unit(
            self.call(KEventDaemonRequest::UnregisterReader {
                reader_id: reader_id.to_string(),
            })
            .await?,
        )
    }

    pub async fn update_reader(
        &self,
        reader_id: &str,
        add: &[String],
        remove: &[String],
    ) -> KEventResult<()> {
        map_response_unit(
            self.call(KEventDaemonRequest::UpdateReader {
                reader_id: reader_id.to_string(),
                add: add.to_vec(),
                remove: remove.to_vec(),
            })
            .await?,
        )
    }

    pub async fn publish_global(&self, event: &Event) -> KEventResult<()> {
        map_response_unit(
            self.call(KEventDaemonRequest::PublishGlobal {
                event: event.clone(),
            })
            .await?,
        )
    }

    pub async fn pull_event(
        &self,
        reader_id: &str,
        timeout_ms: Option<u64>,
    ) -> KEventResult<Option<Event>> {
        map_response_event(
            self.call(KEventDaemonRequest::PullEvent {
                reader_id: reader_id.to_string(),
                timeout_ms,
            })
            .await?,
        )
    }
}

#[async_trait]
impl KEventDaemonBridge for KEventDaemonClient {
    async fn register_reader(&self, reader_id: &str, patterns: &[String]) -> KEventResult<()> {
        self.register_reader(reader_id, patterns).await
    }

    async fn unregister_reader(&self, reader_id: &str) -> KEventResult<()> {
        self.unregister_reader(reader_id).await
    }

    async fn update_reader(
        &self,
        reader_id: &str,
        add: &[String],
        remove: &[String],
    ) -> KEventResult<()> {
        self.update_reader(reader_id, add, remove).await
    }

    async fn publish_global(&self, event: &Event) -> KEventResult<()> {
        self.publish_global(event).await
    }
}

#[derive(Clone)]
pub struct LocalKEventTransport {
    service: Arc<KEventService>,
}

impl LocalKEventTransport {
    pub fn new(service: Arc<KEventService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl KEventProtocolTransport for LocalKEventTransport {
    async fn call(&self, req: KEventDaemonRequest) -> KEventResult<KEventDaemonResponse> {
        Ok(self.service.handle_protocol_request(req).await)
    }
}

#[derive(Clone)]
pub struct InProcessPeerPublisher {
    target: Arc<KEventService>,
}

impl InProcessPeerPublisher {
    pub fn new(target: Arc<KEventService>) -> Self {
        Self { target }
    }
}

#[async_trait]
impl KEventPeerPublisher for InProcessPeerPublisher {
    async fn broadcast(&self, event: &Event) -> KEventResult<()> {
        self.target.publish_from_peer(event.clone()).await
    }
}

pub fn encode_daemon_request(req: &KEventDaemonRequest) -> KEventResult<Vec<u8>> {
    serde_json::to_vec(req)
        .map_err(|err| KEventError::Internal(format!("encode daemon request failed: {}", err)))
}

pub fn decode_daemon_request(data: &[u8]) -> KEventResult<KEventDaemonRequest> {
    serde_json::from_slice(data)
        .map_err(|err| KEventError::Internal(format!("decode daemon request failed: {}", err)))
}

pub fn encode_daemon_response(resp: &KEventDaemonResponse) -> KEventResult<Vec<u8>> {
    serde_json::to_vec(resp)
        .map_err(|err| KEventError::Internal(format!("encode daemon response failed: {}", err)))
}

pub fn decode_daemon_response(data: &[u8]) -> KEventResult<KEventDaemonResponse> {
    serde_json::from_slice(data)
        .map_err(|err| KEventError::Internal(format!("decode daemon response failed: {}", err)))
}

pub fn encode_peer_event(event: &Event) -> KEventResult<Vec<u8>> {
    serde_json::to_vec(event)
        .map_err(|err| KEventError::Internal(format!("encode peer event failed: {}", err)))
}

pub fn decode_peer_event(data: &[u8]) -> KEventResult<Event> {
    serde_json::from_slice(data)
        .map_err(|err| KEventError::Internal(format!("decode peer event failed: {}", err)))
}

pub fn map_response_unit(resp: KEventDaemonResponse) -> KEventResult<()> {
    match resp {
        KEventDaemonResponse::Ok { .. } => Ok(()),
        KEventDaemonResponse::Err { code, message } => Err(map_response_error(&code, &message)),
    }
}

pub fn map_response_event(resp: KEventDaemonResponse) -> KEventResult<Option<Event>> {
    match resp {
        KEventDaemonResponse::Ok { event } => Ok(event),
        KEventDaemonResponse::Err { code, message } => Err(map_response_error(&code, &message)),
    }
}

pub fn map_response_error(code: &str, message: &str) -> KEventError {
    match code {
        "INVALID_EVENTID" => KEventError::InvalidEventId(message.to_string()),
        "INVALID_PATTERN" => KEventError::InvalidPattern(message.to_string()),
        "DAEMON_UNAVAILABLE" => KEventError::DaemonUnavailable(message.to_string()),
        "TIMER_INVALID_TARGET" => KEventError::TimerInvalidTarget(message.to_string()),
        "TIMER_NOT_FOUND" => KEventError::TimerNotFound(message.to_string()),
        "NOT_SUPPORTED" => KEventError::NotSupported(message.to_string()),
        "READER_CLOSED" => KEventError::ReaderClosed(message.to_string()),
        _ => KEventError::Internal(message.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_local_transport_roundtrip() {
        let service = Arc::new(KEventService::new("node_a"));
        let client = KEventDaemonClient::new(Arc::new(LocalKEventTransport::new(service.clone())));

        client
            .register_reader("r1", &["/system/**".to_string()])
            .await
            .unwrap();
        service
            .publish_local_global("/system/node/online", json!({"ok": true}))
            .await
            .unwrap();

        let event = client.pull_event("r1", Some(50)).await.unwrap().unwrap();
        assert_eq!(event.eventid, "/system/node/online");
    }

    #[tokio::test]
    async fn test_in_process_peer_publisher() {
        let service_a = Arc::new(KEventService::new("node_a"));
        let service_b = Arc::new(KEventService::new("node_b"));

        service_b
            .register_reader("r1", vec!["/taskmgr/**".to_string()])
            .await
            .unwrap();
        service_a
            .add_peer_publisher(Arc::new(InProcessPeerPublisher::new(service_b.clone())))
            .await;

        service_a
            .publish_local_global("/taskmgr/new/task_001", json!({"ok": true}))
            .await
            .unwrap();

        let event = service_b
            .pull_event("r1", Some(100))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.eventid, "/taskmgr/new/task_001");
        assert_eq!(event.ingress_node.as_deref(), Some("node_a"));
    }

    #[test]
    fn test_protocol_codecs() {
        let req = KEventDaemonRequest::PullEvent {
            reader_id: "r1".to_string(),
            timeout_ms: Some(1000),
        };
        let req_bytes = encode_daemon_request(&req).unwrap();
        assert_eq!(decode_daemon_request(&req_bytes).unwrap(), req);

        let resp = KEventDaemonResponse::Ok { event: None };
        let resp_bytes = encode_daemon_response(&resp).unwrap();
        assert_eq!(decode_daemon_response(&resp_bytes).unwrap(), resp);
    }
}
