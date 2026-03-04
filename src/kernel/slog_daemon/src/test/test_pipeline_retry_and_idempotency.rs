use crate::client::LogDaemonClient;
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::server::LogHttpServer;
use slog_server::storage::{
    LogQueryRequest, LogRecords, LogStorage, LogStorageRef, LogStorageType,
    create_log_storage_with_dir,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, Instant};

fn new_temp_root(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "buckyos/slog_pipeline_tests/{}_{}_{}",
        prefix,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn allocate_bind_addr() -> Result<String, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("failed to bind test listener on loopback: {}", e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("failed to read local address: {}", e))?;
    Ok(format!("127.0.0.1:{}", addr.port()))
}

fn make_record(service: &str, time: u64, content: &str) -> SystemLogRecord {
    SystemLogRecord {
        level: LogLevel::Info,
        target: service.to_string(),
        time,
        file: Some("pipeline_retry_test.rs".to_string()),
        line: Some(1),
        content: content.to_string(),
    }
}

fn prepare_service_logs(
    log_root: &Path,
    service: &str,
    records: &[SystemLogRecord],
) -> Result<PathBuf, String> {
    let service_dir = log_root.join(service);
    std::fs::create_dir_all(&service_dir).map_err(|e| {
        format!(
            "failed to create service log dir {}: {}",
            service_dir.display(),
            e
        )
    })?;

    let meta = LogMeta::open(&service_dir)?;
    let file_name = format!("{}.1.log", service);
    meta.append_new_file(&file_name)
        .map_err(|e| format!("append_new_file failed: {}", e))?;

    let mut content = String::new();
    for record in records {
        content.push_str(&SystemLogRecordLineFormatter::format_record(record));
    }

    let log_file = service_dir.join(&file_name);
    std::fs::write(&log_file, &content)
        .map_err(|e| format!("failed to write log file {}: {}", log_file.display(), e))?;
    meta.update_current_write_index(content.len() as u64)
        .map_err(|e| format!("update_current_write_index failed: {}", e))?;

    Ok(service_dir)
}

async fn wait_for_uploaded_logs(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
    expected_count: usize,
    timeout: Duration,
) -> Result<Vec<LogRecords>, String> {
    let deadline = Instant::now() + timeout;

    loop {
        let result = storage
            .query_logs(LogQueryRequest {
                node: Some(node.to_string()),
                service: Some(service.to_string()),
                level: None,
                start_time: None,
                end_time: None,
                limit: Some(1000),
            })
            .await?;
        let count: usize = result.iter().map(|records| records.logs.len()).sum();
        if count >= expected_count {
            return Ok(result);
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting for uploaded logs, expected >= {}, got {}",
                expected_count, count
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_append_calls(
    append_calls: &AtomicUsize,
    expected_min_calls: usize,
    timeout: Duration,
) -> Result<usize, String> {
    let deadline = Instant::now() + timeout;

    loop {
        let current = append_calls.load(Ordering::SeqCst);
        if current >= expected_min_calls {
            return Ok(current);
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting append calls, expected >= {}, got {}",
                expected_min_calls, current
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct FalseNegativeOnceStorage {
    inner: LogStorageRef,
    injected_failure: AtomicBool,
    append_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl LogStorage for FalseNegativeOnceStorage {
    async fn append_logs(&self, records: LogRecords) -> Result<(), String> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.append_logs(records).await?;

        // Simulate "stored but returned failure once": uploader will retry the same batch.
        if !self.injected_failure.swap(true, Ordering::SeqCst) {
            return Err("injected false-negative append failure".to_string());
        }

        Ok(())
    }

    async fn query_logs(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
        self.inner.query_logs(request).await
    }
}

#[tokio::test]
async fn test_pipeline_retry_and_idempotency_no_duplicate_records() {
    let root = new_temp_root("retry_idempotency");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_retry_and_idempotency test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-retry";
    let service = "svc_retry";

    let input_records = vec![
        make_record(service, 1722000100001, "retry-idempotency-1"),
        make_record(service, 1722000100002, "retry-idempotency-2"),
        make_record(service, 1722000100003, "retry-idempotency-3"),
    ];
    let service_dir = prepare_service_logs(&root, service, &input_records).unwrap();

    let real_storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();
    let append_calls = Arc::new(AtomicUsize::new(0));
    let flaky_storage: LogStorageRef = Arc::new(Box::new(FalseNegativeOnceStorage {
        inner: real_storage.clone(),
        injected_failure: AtomicBool::new(false),
        append_calls: append_calls.clone(),
    }));

    let server = LogHttpServer::new(flaky_storage);
    let server_handle = tokio::spawn({
        let bind_addr = bind_addr.clone();
        async move {
            let _ = server.run(&bind_addr).await;
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let daemon = LogDaemonClient::new(
        node.to_string(),
        endpoint,
        3,
        &root,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();

    let result = wait_for_uploaded_logs(
        real_storage.as_ref().as_ref(),
        node,
        service,
        input_records.len(),
        Duration::from_secs(12),
    )
    .await
    .unwrap();

    // Must retry at least once due the injected false-negative failure.
    let append_count = wait_for_append_calls(append_calls.as_ref(), 2, Duration::from_secs(12))
        .await
        .unwrap();
    assert!(append_count >= 2);

    let mut uploaded_contents = Vec::new();
    for item in result {
        for log in item.logs {
            uploaded_contents.push(log.content);
        }
    }
    uploaded_contents.sort();

    let mut expected_contents: Vec<String> = input_records
        .iter()
        .map(|record| record.content.clone())
        .collect();
    expected_contents.sort();
    assert_eq!(uploaded_contents, expected_contents);

    // Validate read position eventually flushed to end after retry success.
    let meta = LogMeta::open(&service_dir).unwrap();
    let write_info = meta.get_active_write_file().unwrap().unwrap();
    let file_info = meta.get_file_info(write_info.id).unwrap().unwrap();
    assert_eq!(file_info.read_index, file_info.write_index);

    daemon.shutdown().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;

    std::fs::remove_dir_all(&root).unwrap();
}
