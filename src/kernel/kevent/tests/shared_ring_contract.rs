use buckyos_api::{Event, KEventError, SharedKEventRingBuffer, DEFAULT_RINGBUFFER_PATH_ENV};
use kevent::KEventService;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

static RING_ENV_LOCK: Mutex<()> = Mutex::new(());

fn set_unique_ring_path(test_name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "buckyos_{}_{}_{}.shm",
        test_name,
        std::process::id(),
        nanos
    ));
    let _ = std::fs::remove_file(&path);
    std::env::set_var(DEFAULT_RINGBUFFER_PATH_ENV, &path);
    path
}

fn event(seq: u64) -> Event {
    Event {
        eventid: format!("/shared/ring/{}", seq),
        source_node: "node_a".to_string(),
        source_pid: std::process::id(),
        ingress_node: Some("node_a".to_string()),
        timestamp: seq,
        data: json!({ "seq": seq }),
    }
}

fn large_event() -> Event {
    Event {
        eventid: "/shared/ring/large".to_string(),
        source_node: "node_a".to_string(),
        source_pid: std::process::id(),
        ingress_node: Some("node_a".to_string()),
        timestamp: 1,
        data: json!({ "blob": "x".repeat(4096) }),
    }
}

#[test]
fn shared_ring_delivers_first_event_from_late_producer() {
    let _guard = RING_ENV_LOCK.lock().unwrap();
    let path = set_unique_ring_path("late_producer");

    let consumer = SharedKEventRingBuffer::open().unwrap();
    consumer.prime_cursors();
    let producer = SharedKEventRingBuffer::open().unwrap();
    producer.publish_event(&event(1)).unwrap();

    let events = consumer.drain_events::<Event>(8);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].eventid, "/shared/ring/1");
    assert_eq!(events[0].data["seq"], json!(1));

    drop(producer);
    drop(consumer);
    let _ = std::fs::remove_file(path);
}

#[test]
fn shared_ring_overrun_drops_oldest_without_corrupting_events() {
    let _guard = RING_ENV_LOCK.lock().unwrap();
    let path = set_unique_ring_path("overrun");

    let producer = SharedKEventRingBuffer::open().unwrap();
    let consumer = SharedKEventRingBuffer::open().unwrap();
    consumer.prime_cursors();

    for seq in 0..700 {
        producer.publish_event(&event(seq)).unwrap();
    }

    let events = consumer.drain_events::<Event>(800);
    assert!(!events.is_empty());
    assert!(events.len() <= 512);
    let mut previous = None;
    for event in &events {
        let seq = event.data["seq"].as_u64().unwrap();
        if let Some(previous) = previous {
            assert!(seq > previous);
        }
        previous = Some(seq);
        assert!(event.eventid.starts_with("/shared/ring/"));
    }
    assert!(previous.unwrap() >= 699);

    drop(producer);
    drop(consumer);
    let _ = std::fs::remove_file(path);
}

#[test]
fn shared_ring_rejects_events_larger_than_slot() {
    let _guard = RING_ENV_LOCK.lock().unwrap();
    let path = set_unique_ring_path("large_event");

    let producer = SharedKEventRingBuffer::open().unwrap();
    let err = producer.publish_event(&large_event()).unwrap_err();
    assert!(err.contains("payload too large"), "{err}");

    drop(producer);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn service_returns_error_when_shared_ring_mirror_rejects_event() {
    let _guard = RING_ENV_LOCK.lock().unwrap();
    let path = set_unique_ring_path("service_large_event");

    let service = KEventService::new("node_a");
    service
        .set_shared_ring(Arc::new(SharedKEventRingBuffer::open().unwrap()))
        .await;
    service
        .register_reader("r1", vec!["/shared/ring/**".to_string()])
        .await
        .unwrap();

    let err = service
        .publish_local_global("/shared/ring/large", json!({ "blob": "x".repeat(4096) }))
        .await
        .unwrap_err();
    assert!(matches!(err, KEventError::Internal(_)));
    assert!(service.pull_event("r1", Some(0)).await.unwrap().is_none());

    let _ = std::fs::remove_file(path);
}
