use crate::client::LogDaemonClient;
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::server::LogHttpServer;
use slog_server::storage::{
    LogQueryRequest, LogRecords, LogStorage, LogStorageRef, LogStorageType,
    create_log_storage_with_dir,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
        file: Some("pipeline_graceful_shutdown_test.rs".to_string()),
        line: Some(1),
        content: content.to_string(),
    }
}

fn generate_records(service: &str, start_time: u64, count: usize) -> Vec<SystemLogRecord> {
    (0..count)
        .map(|i| {
            make_record(
                service,
                start_time + i as u64,
                &format!("graceful-shutdown-record-{}", i),
            )
        })
        .collect()
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

async fn query_uploaded_contents(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
) -> Result<Vec<String>, String> {
    let result = storage
        .query_logs(LogQueryRequest {
            node: Some(node.to_string()),
            service: Some(service.to_string()),
            level: None,
            start_time: None,
            end_time: None,
            limit: Some(10_000),
        })
        .await?;

    let mut contents = Vec::new();
    for item in result {
        for log in item.logs {
            contents.push(log.content);
        }
    }
    contents.sort();
    Ok(contents)
}

async fn wait_for_append_started(
    started: &AtomicUsize,
    expected_min: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if started.load(Ordering::SeqCst) >= expected_min {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting append started count >= {}, current={}",
                expected_min,
                started.load(Ordering::SeqCst)
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_uploaded_count(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
    expected_count: usize,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let contents = query_uploaded_contents(storage, node, service).await?;
        if contents.len() >= expected_count {
            return Ok(contents);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting uploaded count >= {}, current={}",
                expected_count,
                contents.len()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_read_index_catch_up(service_dir: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let meta = LogMeta::open(service_dir)?;

    loop {
        let write_info = meta
            .get_active_write_file()
            .map_err(|e| format!("failed to get active write file: {}", e))?
            .ok_or_else(|| "no active write file found while waiting read flush".to_string())?;
        let file_info = meta
            .get_file_info(write_info.id)
            .map_err(|e| format!("failed to get file info for {}: {}", write_info.id, e))?
            .ok_or_else(|| format!("file info not found for file_id={}", write_info.id))?;

        if file_info.read_index == file_info.write_index {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting read_index catch up, read_index={}, write_index={}",
                file_info.read_index, file_info.write_index
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct SlowAppendStorage {
    inner: LogStorageRef,
    append_started: Arc<AtomicUsize>,
    append_completed: Arc<AtomicUsize>,
    delay: Duration,
}

#[async_trait::async_trait]
impl LogStorage for SlowAppendStorage {
    async fn append_logs(&self, records: LogRecords) -> Result<(), String> {
        self.append_started.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        let ret = self.inner.append_logs(records).await;
        if ret.is_ok() {
            self.append_completed.fetch_add(1, Ordering::SeqCst);
        }
        ret
    }

    async fn query_logs(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
        self.inner.query_logs(request).await
    }
}

#[tokio::test]
async fn test_pipeline_graceful_shutdown_drains_inflight_and_resume_completes() {
    let root = new_temp_root("graceful_shutdown_drain");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_graceful_shutdown_drain test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-shutdown";
    let service = "svc_shutdown";
    let input_records = generate_records(service, 1722000400000, 30);

    let service_dir = prepare_service_logs(&root, service, &input_records).unwrap();

    let real_storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();
    let append_started = Arc::new(AtomicUsize::new(0));
    let append_completed = Arc::new(AtomicUsize::new(0));
    let slow_storage: LogStorageRef = Arc::new(Box::new(SlowAppendStorage {
        inner: real_storage.clone(),
        append_started: append_started.clone(),
        append_completed: append_completed.clone(),
        delay: Duration::from_millis(300),
    }));

    let server = LogHttpServer::new(slow_storage);
    let server_handle = tokio::spawn({
        let bind_addr = bind_addr.clone();
        async move {
            let _ = server.run(&bind_addr).await;
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let daemon = LogDaemonClient::new(
        node.to_string(),
        endpoint.clone(),
        3,
        &root,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();

    wait_for_append_started(append_started.as_ref(), 1, Duration::from_secs(6))
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(8), daemon.shutdown())
        .await
        .unwrap()
        .unwrap();

    assert!(append_completed.load(Ordering::SeqCst) >= 1);

    let partial_uploaded = query_uploaded_contents(real_storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    assert!(!partial_uploaded.is_empty());
    assert!(partial_uploaded.len() <= input_records.len());

    // Restart daemon and verify it can resume from persisted read index and finish all logs.
    let daemon_resumed = LogDaemonClient::new(
        node.to_string(),
        endpoint,
        3,
        &root,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();

    let mut uploaded_contents = wait_for_uploaded_count(
        real_storage.as_ref().as_ref(),
        node,
        service,
        input_records.len(),
        Duration::from_secs(16),
    )
    .await
    .unwrap();
    let mut expected_contents: Vec<String> = input_records
        .iter()
        .map(|record| record.content.clone())
        .collect();
    uploaded_contents.sort();
    expected_contents.sort();
    assert_eq!(uploaded_contents, expected_contents);

    wait_for_read_index_catch_up(&service_dir, Duration::from_secs(8))
        .await
        .unwrap();

    daemon_resumed.shutdown().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;

    std::fs::remove_dir_all(&root).unwrap();
}
