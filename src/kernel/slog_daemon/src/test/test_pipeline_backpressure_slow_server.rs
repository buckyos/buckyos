use crate::client::LogDaemonClient;
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::server::LogHttpServer;
use slog_server::storage::{
    LogQueryRequest, LogRecords, LogStorage, LogStorageRef, LogStorageType,
    create_log_storage_with_dir,
};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
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
        file: Some("pipeline_backpressure_test.rs".to_string()),
        line: Some(1),
        content: content.to_string(),
    }
}

fn prepare_service_logs(
    log_root: &Path,
    service: &str,
    records: &[SystemLogRecord],
) -> Result<(), String> {
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

    Ok(())
}

fn prepare_service_with_records(
    log_root: &Path,
    service: &str,
    records: &[SystemLogRecord],
) -> Result<(PathBuf, String), String> {
    let service_dir = log_root.join(service);
    std::fs::create_dir_all(&service_dir).map_err(|e| {
        format!(
            "failed to create service log dir {}: {}",
            service_dir.display(),
            e
        )
    })?;
    let file_name = format!("{}.1.log", service);

    let meta = LogMeta::open(&service_dir)?;
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

    Ok((service_dir, file_name))
}

fn append_records_to_active_file(
    service_dir: &Path,
    file_name: &str,
    records: &[SystemLogRecord],
) -> Result<(), String> {
    let log_file = service_dir.join(file_name);
    let mut file = OpenOptions::new()
        .append(true)
        .open(&log_file)
        .map_err(|e| format!("failed to open append file {}: {}", log_file.display(), e))?;

    for record in records {
        let line = SystemLogRecordLineFormatter::format_record(record);
        file.write_all(line.as_bytes())
            .map_err(|e| format!("failed to append line to {}: {}", log_file.display(), e))?;
    }
    file.flush()
        .map_err(|e| format!("failed to flush {}: {}", log_file.display(), e))?;
    drop(file);

    let total_size = std::fs::metadata(&log_file)
        .map_err(|e| format!("failed to read metadata {}: {}", log_file.display(), e))?
        .len();
    let meta = LogMeta::open(service_dir)?;
    meta.update_current_write_index(total_size)
        .map_err(|e| format!("update_current_write_index after append failed: {}", e))?;
    Ok(())
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
    for item in result {
        for log in item.logs {
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
        for (service, expected) in expected_counts {
            let count = query_service_contents(storage, node, service).await?.len();
            if count < *expected {
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

struct SlowAppendStorage {
    inner: LogStorageRef,
    delay: Duration,
    append_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl LogStorage for SlowAppendStorage {
    async fn append_logs(&self, records: LogRecords) -> Result<(), String> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.inner.append_logs(records).await
    }

    async fn query_logs(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
        self.inner.query_logs(request).await
    }
}

#[tokio::test]
async fn test_pipeline_backpressure_with_slow_server_still_completes() {
    let root = new_temp_root("backpressure_slow_server");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_backpressure_slow_server test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-backpressure";

    let services = vec![
        "svc_backpressure_a".to_string(),
        "svc_backpressure_b".to_string(),
        "svc_backpressure_c".to_string(),
        "svc_backpressure_d".to_string(),
    ];
    let per_service_count = 50usize;

    let mut expected_contents_by_service: HashMap<String, Vec<String>> = HashMap::new();
    let mut expected_counts: HashMap<String, usize> = HashMap::new();

    for (svc_idx, service) in services.iter().enumerate() {
        let base_time = 1722001000000 + svc_idx as u64 * 1000;
        let records: Vec<SystemLogRecord> = (0..per_service_count)
            .map(|i| {
                make_record(
                    service,
                    base_time + i as u64,
                    &format!("{}-record-{}", service, i),
                )
            })
            .collect();
        prepare_service_logs(&root, service, &records).unwrap();

        expected_counts.insert(service.clone(), records.len());
        expected_contents_by_service.insert(
            service.clone(),
            records.iter().map(|r| r.content.clone()).collect(),
        );
    }

    let real_storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();
    let append_calls = Arc::new(AtomicUsize::new(0));
    let slow_storage: LogStorageRef = Arc::new(Box::new(SlowAppendStorage {
        inner: real_storage.clone(),
        delay: Duration::from_millis(200),
        append_calls: append_calls.clone(),
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
        endpoint,
        5,
        &root,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();

    wait_for_expected_counts(
        real_storage.as_ref().as_ref(),
        node,
        &expected_counts,
        Duration::from_secs(30),
    )
    .await
    .unwrap();

    for service in &services {
        let mut uploaded = query_service_contents(real_storage.as_ref().as_ref(), node, service)
            .await
            .unwrap();
        let mut expected = expected_contents_by_service.get(service).unwrap().clone();
        uploaded.sort();
        expected.sort();
        assert_eq!(uploaded, expected);
    }

    // Slow path should have involved many append calls across multiple batches.
    assert!(append_calls.load(Ordering::SeqCst) >= services.len() as usize);

    tokio::time::timeout(Duration::from_secs(8), daemon.shutdown())
        .await
        .unwrap()
        .unwrap();
    server_handle.abort();
    let _ = server_handle.await;
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn test_pipeline_backpressure_with_continuous_appending_eventually_catches_up() {
    let root = new_temp_root("backpressure_continuous_append");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_backpressure_continuous test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-backpressure-continuous";

    let services = vec![
        "svc_bp_cont_a".to_string(),
        "svc_bp_cont_b".to_string(),
        "svc_bp_cont_c".to_string(),
        "svc_bp_cont_d".to_string(),
    ];
    let initial_per_service = 10usize;
    let append_rounds = 4usize;
    let append_per_round = 8usize;

    let mut expected_contents_by_service: HashMap<String, Vec<String>> = HashMap::new();
    let mut expected_counts: HashMap<String, usize> = HashMap::new();
    let mut service_paths: HashMap<String, (PathBuf, String)> = HashMap::new();

    for (svc_idx, service) in services.iter().enumerate() {
        let base_time = 1722001200000 + svc_idx as u64 * 10000;
        let initial_records: Vec<SystemLogRecord> = (0..initial_per_service)
            .map(|i| {
                make_record(
                    service,
                    base_time + i as u64,
                    &format!("{}-init-{}", service, i),
                )
            })
            .collect();

        let (dir, file_name) =
            prepare_service_with_records(&root, service, &initial_records).unwrap();
        service_paths.insert(service.clone(), (dir, file_name));
        expected_contents_by_service.insert(
            service.clone(),
            initial_records.iter().map(|r| r.content.clone()).collect(),
        );
    }

    let real_storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();
    let append_calls = Arc::new(AtomicUsize::new(0));
    let slow_storage: LogStorageRef = Arc::new(Box::new(SlowAppendStorage {
        inner: real_storage.clone(),
        delay: Duration::from_millis(250),
        append_calls: append_calls.clone(),
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
        endpoint,
        5,
        &root,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();

    for round in 0..append_rounds {
        for (svc_idx, service) in services.iter().enumerate() {
            let base_time = 1722001200000 + svc_idx as u64 * 10000;
            let start = initial_per_service + round * append_per_round;
            let round_records: Vec<SystemLogRecord> = (0..append_per_round)
                .map(|i| {
                    let idx = start + i;
                    make_record(
                        service,
                        base_time + idx as u64,
                        &format!("{}-append-{}", service, idx),
                    )
                })
                .collect();
            let (service_dir, file_name) = service_paths.get(service).unwrap();
            append_records_to_active_file(service_dir, file_name, &round_records).unwrap();
            expected_contents_by_service
                .get_mut(service)
                .unwrap()
                .extend(round_records.into_iter().map(|r| r.content));
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }

    for service in &services {
        expected_counts.insert(
            service.clone(),
            expected_contents_by_service.get(service).unwrap().len(),
        );
    }

    wait_for_expected_counts(
        real_storage.as_ref().as_ref(),
        node,
        &expected_counts,
        Duration::from_secs(40),
    )
    .await
    .unwrap();

    for service in &services {
        let mut uploaded = query_service_contents(real_storage.as_ref().as_ref(), node, service)
            .await
            .unwrap();
        let mut expected = expected_contents_by_service.get(service).unwrap().clone();
        uploaded.sort();
        expected.sort();
        assert_eq!(uploaded, expected);
    }

    assert!(append_calls.load(Ordering::SeqCst) >= services.len() * 4);

    tokio::time::timeout(Duration::from_secs(10), daemon.shutdown())
        .await
        .unwrap()
        .unwrap();
    server_handle.abort();
    let _ = server_handle.await;
    std::fs::remove_dir_all(&root).unwrap();
}
