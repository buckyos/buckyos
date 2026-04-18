use buckyos_api::{
    init_buckyos_api_runtime, BuckyOSRuntimeType, KLogAppendRequest, KLogLevel,
    KLogMetaDeleteRequest, KLogMetaPutRequest, KLogMetaQueryRequest, KLogQueryRequest,
};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TEST_APP_ID: &str = "buckycli";
const TEST_REQUEST_NODE_ID: u64 = 9_001;
const TEST_TIMEOUT_SECS: u64 = 15;

fn unique_suffix(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{}", prefix, now)
}

async fn init_logged_in_runtime(
) -> std::result::Result<buckyos_api::BuckyOSRuntime, kRPC::RPCErrors> {
    let mut runtime =
        init_buckyos_api_runtime(TEST_APP_ID, None, BuckyOSRuntimeType::AppClient).await?;
    runtime.login().await?;
    Ok(runtime)
}

#[tokio::test]
#[ignore = "requires real BuckyOS runtime login context and running klog-service route"]
async fn runtime_klog_01_append_and_strong_query_roundtrip() {
    let runtime = init_logged_in_runtime()
        .await
        .expect("init/login runtime for klog append/query");
    let client = runtime
        .get_klog_client(TEST_REQUEST_NODE_ID)
        .await
        .expect("get klog runtime client")
        .with_timeout(Duration::from_secs(TEST_TIMEOUT_SECS));

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
            node_id: Some(TEST_REQUEST_NODE_ID),
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
        .get_klog_client(TEST_REQUEST_NODE_ID)
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
        .get_klog_client(TEST_REQUEST_NODE_ID)
        .await
        .expect("get klog runtime client")
        .with_timeout(Duration::from_secs(TEST_TIMEOUT_SECS));

    let suffix = unique_suffix("dedup");
    let source = format!("buckyos-api/tests/klog-dedup-{}", suffix);
    let request_id = format!("runtime-dedup-{}", suffix);

    let first = client
        .append_log(KLogAppendRequest {
            message: format!("runtime-dedup-original-{}", suffix),
            timestamp: None,
            node_id: Some(TEST_REQUEST_NODE_ID),
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
            node_id: Some(TEST_REQUEST_NODE_ID),
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
