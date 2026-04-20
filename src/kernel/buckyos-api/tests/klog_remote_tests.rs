use buckyos_api::{
    BuckyOSRuntimeType, KLogAppendRequest, KLogLevel, KLogMetaDeleteRequest, KLogMetaPutRequest,
    KLogMetaQueryRequest, KLogQueryRequest, init_buckyos_api_runtime,
};
use klog::network::KLogClusterStateResponse;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TEST_APP_ID: &str = "buckycli";
const TEST_TIMEOUT_SECS: u64 = 15;
const TEST_NODE_GATEWAY_BASE_URL: &str = "http://127.0.0.1:3180";

fn unique_suffix(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{}", prefix, now)
}

async fn init_logged_in_runtime()
-> std::result::Result<buckyos_api::BuckyOSRuntime, kRPC::RPCErrors> {
    let mut runtime =
        init_buckyos_api_runtime(TEST_APP_ID, None, BuckyOSRuntimeType::AppClient).await?;
    runtime.login().await?;
    Ok(runtime)
}

fn require_runtime_node_name(runtime: &buckyos_api::BuckyOSRuntime) -> String {
    runtime
        .device_config
        .as_ref()
        .map(|device| device.name.clone())
        .expect("runtime missing device_config.name")
}

#[tokio::test]
#[ignore = "requires real BuckyOS runtime login context and running klog-service route"]
async fn runtime_klog_01_append_and_strong_query_roundtrip() {
    let runtime = init_logged_in_runtime()
        .await
        .expect("init/login runtime for klog append/query");
    let client = runtime
        .get_klog_client()
        .await
        .expect("get klog runtime client")
        .with_timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    let request_node_name = require_runtime_node_name(&runtime);

    let suffix = unique_suffix("append-query");
    let source = format!("buckyos-api/tests/klog-{}", suffix);
    let message = format!("runtime-klog-message-{}", suffix);
    let mut attrs = BTreeMap::new();
    attrs.insert("case".to_string(), suffix.clone());
    attrs.insert("suite".to_string(), "klog_remote_tests".to_string());

    let appended = client
        .append_log(KLogAppendRequest {
            message: message.clone(),
            timestamp: None,
            node_name: Some(request_node_name),
            level: Some(KLogLevel::Warn),
            source: Some(source.clone()),
            attrs: Some(attrs.clone()),
            request_id: Some(format!("runtime-append-{}", suffix)),
        })
        .await
        .expect("append log through runtime client");

    let queried = client
        .query_log(KLogQueryRequest {
            start_id: Some(appended.id),
            end_id: Some(appended.id),
            limit: Some(8),
            desc: Some(false),
            level: Some(KLogLevel::Warn),
            source: Some(source.clone()),
            attr_key: Some("case".to_string()),
            attr_value: Some(suffix.clone()),
            strong_read: Some(true),
        })
        .await
        .expect("query log through runtime client");

    assert_eq!(queried.items.len(), 1, "unexpected query item count");
    let item = &queried.items[0];
    assert_eq!(item.id, appended.id, "query returned unexpected log id");
    assert_eq!(item.message, message, "query returned unexpected message");
    assert_eq!(
        item.level,
        KLogLevel::Warn,
        "query returned unexpected level"
    );
    assert_eq!(item.source.as_deref(), Some(source.as_str()));
    assert_eq!(
        item.attrs.get("case").map(String::as_str),
        Some(suffix.as_str())
    );
}

#[tokio::test]
#[ignore = "requires real BuckyOS runtime login context and running klog-service route"]
async fn runtime_klog_02_meta_roundtrip() {
    let runtime = init_logged_in_runtime()
        .await
        .expect("init/login runtime for klog meta");
    let client = runtime
        .get_klog_client()
        .await
        .expect("get klog runtime client")
        .with_timeout(Duration::from_secs(TEST_TIMEOUT_SECS));

    let suffix = unique_suffix("meta");
    let key = format!("tests/klog/runtime/{}", suffix);
    let value = format!("meta-value-{}", suffix);

    let put = client
        .put_meta(KLogMetaPutRequest {
            key: key.clone(),
            value: value.clone(),
            node_name: None,
            expected_revision: None,
        })
        .await
        .expect("put meta through runtime client");
    assert_eq!(put.key, key, "put_meta returned unexpected key");
    assert!(put.revision >= 1, "put_meta returned invalid revision");

    let queried = client
        .query_meta(KLogMetaQueryRequest {
            key: Some(key.clone()),
            prefix: None,
            limit: Some(4),
            strong_read: Some(true),
        })
        .await
        .expect("query meta through runtime client");
    assert_eq!(queried.items.len(), 1, "unexpected meta query item count");
    let item = &queried.items[0];
    assert_eq!(item.key, key, "meta query returned unexpected key");
    assert_eq!(item.value, value, "meta query returned unexpected value");
    assert_eq!(
        item.revision, put.revision,
        "meta query returned unexpected revision"
    );

    let deleted = client
        .delete_meta(KLogMetaDeleteRequest { key: key.clone() })
        .await
        .expect("delete meta through runtime client");
    assert!(deleted.existed, "delete_meta should report existed=true");
    let prev = deleted.prev_meta.expect("delete_meta missing prev_meta");
    assert_eq!(prev.key, key, "prev_meta returned unexpected key");
    assert_eq!(prev.value, value, "prev_meta returned unexpected value");
    assert_eq!(
        prev.revision, put.revision,
        "prev_meta returned unexpected revision"
    );
}

#[tokio::test]
#[ignore = "requires real BuckyOS runtime login context and running klog-service route"]
async fn runtime_klog_03_request_id_dedup() {
    let runtime = init_logged_in_runtime()
        .await
        .expect("init/login runtime for klog dedup");
    let client = runtime
        .get_klog_client()
        .await
        .expect("get klog runtime client")
        .with_timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    let request_node_name = require_runtime_node_name(&runtime);

    let suffix = unique_suffix("dedup");
    let source = format!("buckyos-api/tests/klog-dedup-{}", suffix);
    let request_id = format!("runtime-dedup-{}", suffix);

    let first = client
        .append_log(KLogAppendRequest {
            message: format!("runtime-dedup-original-{}", suffix),
            timestamp: None,
            node_name: Some(request_node_name.clone()),
            level: Some(KLogLevel::Info),
            source: Some(source.clone()),
            attrs: None,
            request_id: Some(request_id.clone()),
        })
        .await
        .expect("first append with request_id");
    let retry = client
        .append_log(KLogAppendRequest {
            message: format!("runtime-dedup-retry-{}", suffix),
            timestamp: None,
            node_name: Some(request_node_name),
            level: Some(KLogLevel::Info),
            source: Some(source.clone()),
            attrs: None,
            request_id: Some(request_id.clone()),
        })
        .await
        .expect("retry append with same request_id");

    assert_eq!(first.id, retry.id, "request_id dedup should return same id");

    let queried = client
        .query_log(KLogQueryRequest {
            start_id: Some(first.id),
            end_id: Some(first.id),
            limit: Some(4),
            desc: Some(false),
            level: Some(KLogLevel::Info),
            source: Some(source),
            attr_key: None,
            attr_value: None,
            strong_read: Some(true),
        })
        .await
        .expect("query dedup result");

    assert_eq!(queried.items.len(), 1, "dedup query should return one item");
    let item = &queried.items[0];
    assert_eq!(item.id, first.id, "dedup query returned unexpected id");
    assert_eq!(
        item.request_id.as_deref(),
        Some(request_id.as_str()),
        "dedup query returned unexpected request_id"
    );
    assert!(
        item.message.contains("runtime-dedup-original"),
        "dedup should preserve the original append content"
    );
}

#[tokio::test]
#[ignore = "requires real BuckyOS runtime login context and node-gateway cluster route on 127.0.0.1:3180"]
async fn runtime_klog_04_cluster_state_via_node_gateway() {
    let runtime = init_logged_in_runtime()
        .await
        .expect("init/login runtime for node-gateway cluster-state");
    let node_name = require_runtime_node_name(&runtime);
    let url = format!(
        "{}/.cluster/klog/{}/admin/cluster-state",
        TEST_NODE_GATEWAY_BASE_URL, node_name
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TEST_TIMEOUT_SECS))
        .build()
        .expect("build reqwest client");
    let response = client
        .get(&url)
        .send()
        .await
        .expect("send cluster-state request through node gateway");
    let status = response.status();
    let body = response.bytes().await.expect("read cluster-state response");
    assert!(
        status.is_success(),
        "cluster-state request failed: status={}, body={}",
        status,
        String::from_utf8_lossy(&body)
    );

    let state: KLogClusterStateResponse =
        serde_json::from_slice(&body).expect("parse cluster-state response");
    assert_eq!(state.cluster_name, "test.buckyos.io");
    assert_eq!(state.cluster_id, "test.buckyos.io");
    assert_eq!(state.current_leader, Some(state.node_id));
    assert!(
        state.voters.contains(&state.node_id),
        "cluster-state voters should contain local node_id"
    );
    assert!(
        state.nodes.contains_key(&state.node_id),
        "cluster-state nodes should contain local node entry"
    );
}
