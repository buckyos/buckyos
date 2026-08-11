use async_trait::async_trait;
use buckyos_api::{
    match_event_patterns, normalize_patterns, validate_event_data_size, validate_eventid,
    validate_pattern, Event, KEventClient, KEventDaemonBridge, KEventError, KEventResult,
    TimerOptions,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

struct RecordingBridge {
    published: Mutex<Vec<Event>>,
}

#[async_trait]
impl KEventDaemonBridge for RecordingBridge {
    async fn publish_global(&self, event: &Event) -> KEventResult<()> {
        self.published.lock().await.push(event.clone());
        Ok(())
    }
}

#[test]
fn eventid_and_pattern_validation_contract() {
    assert!(validate_eventid("/taskmgr/new/task_001").is_ok());
    assert!(validate_eventid("heartbeat_tick").is_ok());
    assert!(matches!(
        validate_eventid("/"),
        Err(KEventError::InvalidEventId(_))
    ));
    assert!(matches!(
        validate_eventid("bad/name"),
        Err(KEventError::InvalidEventId(_))
    ));
    assert!(matches!(
        validate_pattern("/taskmgr/bad*segment"),
        Err(KEventError::InvalidPattern(_))
    ));
    assert!(matches!(
        validate_pattern("local*wildcard"),
        Err(KEventError::InvalidPattern(_))
    ));
}

#[test]
fn pattern_match_and_normalize_contract() {
    let patterns = vec!["/a/*/c".to_string(), "/taskmgr/**".to_string()];
    assert!(match_event_patterns(&patterns, "/a/b/c"));
    assert!(!match_event_patterns(&patterns, "/a/b/d"));
    assert!(match_event_patterns(&patterns, "/taskmgr/new/task_001"));

    assert_eq!(
        normalize_patterns(vec![
            "/sys/node/online".to_string(),
            "/sys/**".to_string(),
            "/sys/node/offline".to_string(),
        ]),
        vec!["/sys/**".to_string()]
    );
}

#[tokio::test]
async fn local_pub_sub_timeout_and_dynamic_patterns() {
    let client = KEventClient::new_local("node_a");
    let reader = client
        .create_event_reader(vec!["heartbeat_tick".to_string()])
        .await
        .unwrap();

    assert!(reader.pull_event(Some(0)).await.unwrap().is_none());
    client
        .pub_event("heartbeat_tock", json!({"seq": 0}))
        .await
        .unwrap();
    assert!(reader.pull_event(Some(20)).await.unwrap().is_none());

    client
        .pub_event("heartbeat_tick", json!({"seq": 1}))
        .await
        .unwrap();
    reader
        .add_patterns(vec!["heartbeat_tock".to_string()])
        .await
        .unwrap();
    let event = reader.pull_event(Some(50)).await.unwrap().unwrap();
    assert_eq!(event.eventid, "heartbeat_tick");

    reader
        .remove_patterns(vec!["heartbeat_tick".to_string()])
        .await
        .unwrap();
    client
        .pub_event("heartbeat_tick", json!({"seq": 2}))
        .await
        .unwrap();
    assert!(reader.pull_event(Some(20)).await.unwrap().is_none());
}

#[tokio::test]
async fn local_pub_sub_accepts_large_data_within_sdk_limit() {
    let client = KEventClient::new_local("node_a");
    let reader = client
        .create_event_reader(vec!["large_local_event".to_string()])
        .await
        .unwrap();
    let data = json!({ "blob": "x".repeat(4096) });
    validate_event_data_size(&data).unwrap();

    client
        .pub_event("large_local_event", data.clone())
        .await
        .unwrap();
    let event = reader.pull_event(Some(50)).await.unwrap().unwrap();
    assert_eq!(event.eventid, "large_local_event");
    assert_eq!(event.data, data);
}

#[test]
fn event_data_larger_than_sdk_limit_is_rejected() {
    let data = json!({ "blob": "x".repeat(70 * 1024) });
    assert!(matches!(
        validate_event_data_size(&data),
        Err(KEventError::InvalidEventId(_))
    ));
}

#[tokio::test]
async fn timer_and_mode_boundaries_are_explicit() {
    let client = KEventClient::new_local("node_a");
    let reader = client
        .create_event_reader(vec!["timer_tick".to_string()])
        .await
        .unwrap();
    let timer_id = client
        .create_timer(
            "timer_tick",
            TimerOptions {
                interval_ms: 20,
                repeat: true,
                initial_delay_ms: Some(1),
                data: Some(json!({"kind": "timer"})),
            },
        )
        .await
        .unwrap();
    let event = reader.pull_event(Some(100)).await.unwrap().unwrap();
    assert_eq!(event.eventid, "timer_tick");
    assert!(event.data.get("_timer").is_some());
    client.cancel_timer(&timer_id).await.unwrap();
    assert!(reader.pull_event(Some(80)).await.unwrap().is_none());

    let local_pub_only = KEventClient::new_local_pub_only("node_a");
    assert!(matches!(
        local_pub_only
            .create_event_reader(vec!["local".to_string()])
            .await,
        Err(KEventError::NotSupported(_))
    ));

    let bridge = Arc::new(RecordingBridge {
        published: Mutex::new(Vec::new()),
    });
    let light = KEventClient::new_light("light_node", bridge.clone());
    assert!(matches!(
        light.pub_event("local_event", json!({})).await,
        Err(KEventError::NotSupported(_))
    ));
    light
        .pub_event("/system/node/online", json!({"ok": true}))
        .await
        .unwrap();
    assert_eq!(bridge.published.lock().await.len(), 1);
}
