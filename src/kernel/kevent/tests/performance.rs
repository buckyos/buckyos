use buckyos_api::{KEventClient, KEventClientMode, DEFAULT_RINGBUFFER_PATH_ENV};
use kevent::KEventService;
use serde_json::json;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static RING_ENV_LOCK: Mutex<()> = Mutex::new(());

fn set_unique_ring_path(test_name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "buckyos_perf_{}_{}_{}.shm",
        test_name,
        std::process::id(),
        nanos
    ));
    let _ = std::fs::remove_file(&path);
    std::env::set_var(DEFAULT_RINGBUFFER_PATH_ENV, &path);
    path
}

#[tokio::test]
#[ignore]
async fn local_pub_sub_baseline() {
    let client = KEventClient::new_local("node_a");
    let reader = client
        .create_event_reader(vec!["perf_tick".to_string()])
        .await
        .unwrap();

    let start = Instant::now();
    for seq in 0..10_000 {
        client
            .pub_event("perf_tick", json!({ "seq": seq }))
            .await
            .unwrap();
    }
    let publish_elapsed = start.elapsed();

    let start = Instant::now();
    let mut consumed = 0usize;
    let mut last_seq = None;
    while let Some(event) = reader.pull_event(Some(0)).await.unwrap() {
        consumed += 1;
        last_seq = event.data["seq"].as_u64();
    }
    let consume_elapsed = start.elapsed();
    assert!(consumed > 0);
    assert_eq!(last_seq, Some(9_999));

    eprintln!(
        "{{\"kevent_local_publish_10k_ms\":{},\"kevent_local_consume_retained_ms\":{},\"kevent_local_retained\":{}}}",
        publish_elapsed.as_millis(),
        consume_elapsed.as_millis(),
        consumed
    );
}

#[tokio::test]
#[ignore]
async fn service_publish_pull_baseline() {
    let service = KEventService::new_with_capacity("node_a", 10_000);
    service
        .register_reader("perf", vec!["/perf/**".to_string()])
        .await
        .unwrap();

    let start = Instant::now();
    for seq in 0..10_000 {
        service
            .publish_local_global("/perf/event", json!({ "seq": seq }))
            .await
            .unwrap();
    }
    let publish_elapsed = start.elapsed();

    let start = Instant::now();
    for seq in 0..10_000 {
        let event = service
            .pull_event("perf", Some(1000))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.data["seq"], json!(seq));
    }
    let consume_elapsed = start.elapsed();

    eprintln!(
        "{{\"kevent_service_publish_10k_ms\":{},\"kevent_service_consume_10k_ms\":{}}}",
        publish_elapsed.as_millis(),
        consume_elapsed.as_millis()
    );
}

#[tokio::test]
#[ignore]
async fn shared_ring_full_client_latency_baseline() {
    let _guard = RING_ENV_LOCK.lock().unwrap();
    let path = set_unique_ring_path("shared_ring_latency");

    let publisher = KEventClient::new_with_mode("node_a", KEventClientMode::Full, None, 4096);
    let subscriber = KEventClient::new_with_mode("node_a", KEventClientMode::Full, None, 4096);
    let reader = subscriber
        .create_event_reader(vec!["/perf/shared_ring/**".to_string()])
        .await
        .unwrap();

    let start = Instant::now();
    for seq in 0..2_000 {
        publisher
            .pub_event("/perf/shared_ring/event", json!({ "seq": seq }))
            .await
            .unwrap();
        let event = reader.pull_event(Some(1000)).await.unwrap().unwrap();
        assert_eq!(event.data["seq"], json!(seq));
    }
    let roundtrip_elapsed = start.elapsed();

    let timeout_start = Instant::now();
    while timeout_start.elapsed() < Duration::from_millis(100) {
        if reader.pull_event(Some(0)).await.unwrap().is_none() {
            break;
        }
    }

    eprintln!(
        "{{\"kevent_shared_ring_roundtrip_2k_ms\":{},\"kevent_shared_ring_roundtrips\":{}}}",
        roundtrip_elapsed.as_millis(),
        2_000
    );

    let _ = std::fs::remove_file(path);
}
