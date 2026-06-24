use async_trait::async_trait;
use buckyos_api::{Event, KEventError, KEventResult};
use kevent::{InProcessPeerPublisher, KEventPeerPublisher, KEventService};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct CountingPeer {
    inner: InProcessPeerPublisher,
    count: AtomicUsize,
}

impl CountingPeer {
    fn new(target: Arc<KEventService>) -> Self {
        Self {
            inner: InProcessPeerPublisher::new(target),
            count: AtomicUsize::new(0),
        }
    }

    fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl KEventPeerPublisher for CountingPeer {
    async fn broadcast(&self, event: &Event) -> KEventResult<()> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.inner.broadcast(event).await
    }
}

#[tokio::test]
async fn service_register_publish_pull_and_invalid_inputs() {
    let service = KEventService::new("node_a");
    service
        .register_reader("r1", vec!["/system/**".to_string()])
        .await
        .unwrap();
    service
        .publish_local_global("/system/node/online", json!({"ok": true}))
        .await
        .unwrap();
    let event = service.pull_event("r1", Some(50)).await.unwrap().unwrap();
    assert_eq!(event.eventid, "/system/node/online");
    assert_eq!(event.source_node, "node_a");

    assert!(service
        .pull_event("missing", Some(0))
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        service
            .register_reader("", vec!["/system/**".to_string()])
            .await,
        Err(KEventError::InvalidPattern(_))
    ));
    assert!(matches!(
        service
            .register_reader("local", vec!["heartbeat_tick".to_string()])
            .await,
        Err(KEventError::InvalidPattern(_))
    ));
    assert!(matches!(
        service
            .publish_local_global("heartbeat_tick", json!({}))
            .await,
        Err(KEventError::InvalidEventId(_))
    ));
}

#[tokio::test]
async fn reader_queue_overflow_drops_oldest_events() {
    let service = KEventService::new_with_capacity("node_a", 3);
    service
        .register_reader("r1", vec!["/overflow/**".to_string()])
        .await
        .unwrap();

    for seq in 0..5 {
        service
            .publish_local_global("/overflow/test", json!({ "seq": seq }))
            .await
            .unwrap();
    }

    let mut got = Vec::new();
    while let Some(event) = service.pull_event("r1", Some(0)).await.unwrap() {
        got.push(event.data["seq"].as_i64().unwrap());
    }
    assert_eq!(got, vec![2, 3, 4]);
}

#[tokio::test]
async fn unregister_reader_closes_update_path_and_drops_queue() {
    let service = KEventService::new("node_a");
    service
        .register_reader("r1", vec!["/lifecycle/**".to_string()])
        .await
        .unwrap();
    service
        .publish_local_global("/lifecycle/queued", json!({"seq": 1}))
        .await
        .unwrap();

    service.unregister_reader("r1").await;

    assert!(service.pull_event("r1", Some(0)).await.unwrap().is_none());
    assert!(matches!(
        service
            .update_reader("r1", vec!["/lifecycle/**".to_string()], vec![])
            .await,
        Err(KEventError::ReaderClosed(_))
    ));

    service.unregister_reader("r1").await;
    service
        .register_reader("r1", vec!["/lifecycle/**".to_string()])
        .await
        .unwrap();
    assert!(service.pull_event("r1", Some(0)).await.unwrap().is_none());
}

#[tokio::test]
async fn peer_publish_delivers_once_and_does_not_rebroadcast_peer_events() {
    let service_a = Arc::new(KEventService::new("node_a"));
    let service_b = Arc::new(KEventService::new("node_b"));
    let a_to_b = Arc::new(CountingPeer::new(service_b.clone()));
    let b_to_a = Arc::new(CountingPeer::new(service_a.clone()));

    service_a.add_peer_publisher(a_to_b.clone()).await;
    service_b.add_peer_publisher(b_to_a.clone()).await;
    service_b
        .register_reader("b_reader", vec!["/peer/**".to_string()])
        .await
        .unwrap();

    service_a
        .publish_local_global("/peer/event", json!({"ok": true}))
        .await
        .unwrap();

    let event = service_b
        .pull_event("b_reader", Some(50))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.eventid, "/peer/event");
    assert_eq!(event.ingress_node.as_deref(), Some("node_a"));
    assert_eq!(a_to_b.count(), 1);
    assert_eq!(b_to_a.count(), 0);
}
