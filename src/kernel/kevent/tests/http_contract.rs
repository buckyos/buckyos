use buckyos_api::{KEventDaemonRequest, SharedKEventRingBuffer, DEFAULT_RINGBUFFER_PATH_ENV};
use bytes::Bytes;
use http::StatusCode;
use http_body_util::{combinators::BoxBody, BodyExt};
use kevent::{KEventHttpServer, KEventService};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{timeout, Duration};

static RING_ENV_LOCK: Mutex<()> = Mutex::new(());

fn set_unique_ring_path(test_name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "buckyos_http_{}_{}_{}.shm",
        test_name,
        std::process::id(),
        nanos
    ));
    let _ = std::fs::remove_file(&path);
    std::env::set_var(DEFAULT_RINGBUFFER_PATH_ENV, &path);
    path
}

async fn response_json(
    response: http::Response<BoxBody<Bytes, buckyos_http_server::ServerError>>,
) -> Value {
    let collected = response.into_body().collect().await.unwrap();
    serde_json::from_slice(&collected.to_bytes()).unwrap()
}

async fn read_stream_line(body: &mut BoxBody<Bytes, buckyos_http_server::ServerError>) -> Value {
    let frame = timeout(Duration::from_millis(200), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_slice(frame.into_data().unwrap().as_ref()).unwrap()
}

#[tokio::test]
async fn native_endpoint_roundtrips_protocol_and_rejects_bad_json() {
    let service = Arc::new(KEventService::new("node_a"));
    let server = KEventHttpServer::new(service.clone());

    let response = server
        .handle_http_request(
            "/kapi/kevent",
            serde_json::to_vec(&KEventDaemonRequest::RegisterReader {
                reader_id: "r1".to_string(),
                patterns: vec!["/system/**".to_string()],
            })
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({ "status": "ok" }));

    service
        .publish_local_global("/system/node/online", json!({"ok": true}))
        .await
        .unwrap();
    let response = server
        .handle_http_request(
            "/kapi/kevent",
            serde_json::to_vec(&KEventDaemonRequest::PullEvent {
                reader_id: "r1".to_string(),
                timeout_ms: Some(50),
            })
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();
    let value = response_json(response).await;
    assert_eq!(value["status"], "ok");
    assert_eq!(value["event"]["eventid"], "/system/node/online");

    assert!(server
        .handle_http_request("/kapi/kevent", b"{bad-json")
        .await
        .is_err());
}

#[tokio::test]
async fn publish_endpoint_sets_global_event_metadata_and_rejects_local_eventid() {
    let service = Arc::new(KEventService::new("node_a"));
    let server = KEventHttpServer::new(service.clone());
    service
        .register_reader("r1", vec!["/taskmgr/**".to_string()])
        .await
        .unwrap();

    let response = server
        .handle_http_request(
            "/kapi/kevent/publish",
            serde_json::to_vec(&json!({
                "eventid": "/taskmgr/new/task_001",
                "data": { "ok": true }
            }))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({ "status": "ok" }));

    let event = service.pull_event("r1", Some(50)).await.unwrap().unwrap();
    assert_eq!(event.eventid, "/taskmgr/new/task_001");
    assert_eq!(event.source_node, "node_a");
    assert_eq!(event.ingress_node.as_deref(), Some("node_a"));
    assert!(event.timestamp > 0);

    let response = server
        .handle_http_request(
            "/kapi/kevent/publish",
            serde_json::to_vec(&json!({ "eventid": "local_event" }))
                .unwrap()
                .as_slice(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = response_json(response).await;
    assert_eq!(value["status"], "err");
}

#[tokio::test]
async fn publish_endpoint_reports_shared_ring_large_event_failure() {
    let _guard = RING_ENV_LOCK.lock().unwrap();
    let path = set_unique_ring_path("large_publish");

    let service = Arc::new(KEventService::new("node_a"));
    service
        .set_shared_ring(Arc::new(SharedKEventRingBuffer::open().unwrap()))
        .await;
    service
        .register_reader("r1", vec!["/taskmgr/**".to_string()])
        .await
        .unwrap();
    let server = KEventHttpServer::new(service.clone());

    let response = server
        .handle_http_request(
            "/kapi/kevent/publish",
            serde_json::to_vec(&json!({
                "eventid": "/taskmgr/large",
                "data": { "blob": "x".repeat(4096) }
            }))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let value = response_json(response).await;
    assert_eq!(value["status"], "err");
    assert_eq!(value["code"], "INTERNAL");
    assert!(service.pull_event("r1", Some(0)).await.unwrap().is_none());

    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn stream_endpoint_emits_ack_event_and_keepalive_frames() {
    let service = Arc::new(KEventService::new("node_a"));
    let server = KEventHttpServer::new(service.clone());

    let response = server
        .handle_http_request(
            "/kapi/kevent/stream",
            serde_json::to_vec(&json!({
                "patterns": ["/system/**"],
                "keepalive_ms": 10
            }))
            .unwrap()
            .as_slice(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(http::header::CONTENT_TYPE).unwrap(),
        "application/x-ndjson"
    );

    let mut body = response.into_body();
    let ack = read_stream_line(&mut body).await;
    assert_eq!(ack["type"], "ack");

    service
        .publish_local_global("/system/node/online", json!({"ok": true}))
        .await
        .unwrap();
    let event = read_stream_line(&mut body).await;
    assert_eq!(event["type"], "event");
    assert_eq!(event["event"]["eventid"], "/system/node/online");

    let keepalive = read_stream_line(&mut body).await;
    assert_eq!(keepalive["type"], "keepalive");
}

#[tokio::test]
async fn unsupported_path_returns_bad_request_response() {
    let server = KEventHttpServer::new(Arc::new(KEventService::new("node_a")));
    let response = server
        .handle_http_request("/kapi/kevent/missing", b"{}")
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response_json(response).await["error"]
        .as_str()
        .unwrap()
        .contains("Unsupported kevent path"));
}
