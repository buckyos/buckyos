use crate::client::LogDaemonClient;
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::server::LogHttpServer;
use slog_server::storage::{
    LogQueryRequest, LogRecords, LogStorage, LogStorageRef, LogStorageType,
    create_log_storage_with_dir,
};
use std::collections::{HashMap, HashSet};
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
        file: Some("pipeline_concurrent_integrity.rs".to_string()),
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

async fn query_service_contents(
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
            limit: Some(20_000),
        })
        .await?;

    let mut contents = Vec::new();
    for row in result {
        for log in row.logs {
            contents.push(log.content);
        }
    }
    contents.sort();
    Ok(contents)
}

async fn wait_for_expected_counts(
    storage: &dyn LogStorage,
    node: &str,
    expected_counts: &HashMap<String, usize>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut all_ready = true;
        for (service, expected_count) in expected_counts {
            let uploaded_count = query_service_contents(storage, node, service).await?.len();
            if uploaded_count < *expected_count {
                all_ready = false;
                break;
            }
        }

        if all_ready {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting expected counts for node={}, expected={:?}",
                node, expected_counts
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_read_index_catch_up(service_dir: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let meta = LogMeta::open(service_dir)?;
        let write_info = meta
            .get_active_write_file()
            .map_err(|e| format!("get_active_write_file failed: {}", e))?
            .ok_or_else(|| "missing active write file".to_string())?;
        let file_info = meta
            .get_file_info(write_info.id)
            .map_err(|e| format!("get_file_info failed: {}", e))?
            .ok_or_else(|| "missing file info".to_string())?;

        if file_info.read_index == file_info.write_index {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting read_index catch up for {}, read_index={}, write_index={}",
                service_dir.display(),
                file_info.read_index,
                file_info.write_index
            ));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

struct JitterAppendStorage {
    inner: LogStorageRef,
    append_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl LogStorage for JitterAppendStorage {
    async fn append_logs(&self, records: LogRecords) -> Result<(), String> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        let hash = records
            .service
            .bytes()
            .fold(0_u64, |acc, b| acc.wrapping_mul(131).wrapping_add(b as u64));
        let delay_ms = 10 + (hash % 40);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        self.inner.append_logs(records).await
    }

    async fn query_logs(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
        self.inner.query_logs(request).await
    }
}

#[tokio::test]
async fn test_pipeline_high_concurrency_multi_service_integrity_no_loss_no_duplicate() {
    let root = new_temp_root("concurrent_multi_service_integrity");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_concurrent_multi_service_integrity due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-concurrent-integrity";
    let service_count = 16usize;
    let per_service_records = 48usize;

    let mut expected_contents_by_service: HashMap<String, Vec<String>> = HashMap::new();
    let mut expected_counts: HashMap<String, usize> = HashMap::new();
    let mut service_dirs: HashMap<String, PathBuf> = HashMap::new();

    for svc_idx in 0..service_count {
        let service = format!("svc_concurrent_{:02}", svc_idx);
        let base_time = 1723090000000 + svc_idx as u64 * 10_000;
        let records: Vec<SystemLogRecord> = (0..per_service_records)
            .map(|i| {
                make_record(
                    &service,
                    base_time + i as u64,
                    &format!("{}-record-{}", service, i),
                )
            })
            .collect();

        let service_dir = prepare_service_logs(&root, &service, &records).unwrap();
        expected_counts.insert(service.clone(), records.len());
        expected_contents_by_service.insert(
            service.clone(),
            records.iter().map(|r| r.content.clone()).collect(),
        );
        service_dirs.insert(service, service_dir);
    }

    let real_storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();
    let append_calls = Arc::new(AtomicUsize::new(0));
    let jitter_storage: LogStorageRef = Arc::new(Box::new(JitterAppendStorage {
        inner: real_storage.clone(),
        append_calls: append_calls.clone(),
    }));

    let server = LogHttpServer::new(jitter_storage);
    let server_handle = tokio::spawn({
        let bind_addr = bind_addr.clone();
        async move {
            let _ = server.run(&bind_addr).await;
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let daemon = LogDaemonClient::new_with_upload_concurrency(
        node.to_string(),
        endpoint,
        5,
        12,
        &root,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();

    wait_for_expected_counts(
        real_storage.as_ref().as_ref(),
        node,
        &expected_counts,
        Duration::from_secs(45),
    )
    .await
    .unwrap();

    for (service, expected_contents) in &expected_contents_by_service {
        let uploaded = query_service_contents(real_storage.as_ref().as_ref(), node, service)
            .await
            .unwrap();
        let unique: HashSet<String> = uploaded.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            uploaded.len(),
            "duplicate logs detected for service {}",
            service
        );

        let mut expected = expected_contents.clone();
        expected.sort();
        assert_eq!(
            uploaded, expected,
            "uploaded log content mismatch for service {}",
            service
        );
    }

    for (service, service_dir) in &service_dirs {
        wait_for_read_index_catch_up(service_dir, Duration::from_secs(10))
            .await
            .unwrap_or_else(|e| panic!("read index catch-up failed for {}: {}", service, e));
    }

    assert!(append_calls.load(Ordering::SeqCst) >= service_count);

    tokio::time::timeout(Duration::from_secs(8), daemon.shutdown())
        .await
        .unwrap()
        .unwrap();
    server_handle.abort();
    let _ = server_handle.await;
    std::fs::remove_dir_all(&root).unwrap();
}
