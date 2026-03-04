use crate::client::LogDaemonClient;
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::server::LogHttpServer;
use slog_server::storage::{
    LogQueryRequest, LogStorage, LogStorageType, create_log_storage_with_dir,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
        file: Some("pipeline_multi_service_test.rs".to_string()),
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
            limit: Some(10_000),
        })
        .await?;

    let mut contents = Vec::new();
    for records in result {
        for log in records.logs {
            contents.push(log.content);
        }
    }
    contents.sort();
    Ok(contents)
}

async fn wait_for_service_min_count(
    storage: &dyn LogStorage,
    node: &str,
    services: &[&str],
    min_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;

    loop {
        let mut all_ready = true;
        for service in services {
            let count = query_service_contents(storage, node, service).await?.len();
            if count < min_count {
                all_ready = false;
                break;
            }
        }
        if all_ready {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting all services reach min_count={}, services={:?}",
                min_count, services
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_expected_counts(
    storage: &dyn LogStorage,
    node: &str,
    expected_counts: &HashMap<&str, usize>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;

    loop {
        let mut all_match = true;
        for (service, expected_count) in expected_counts {
            let count = query_service_contents(storage, node, service).await?.len();
            if count != *expected_count {
                all_match = false;
                break;
            }
        }

        if all_match {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting expected counts, expected={:?}",
                expected_counts
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
                "timeout waiting read_index catch up for {}, read_index={}, write_index={}",
                service_dir.display(),
                file_info.read_index,
                file_info.write_index
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn generate_service_records(service: &str, start_time: u64, count: usize) -> Vec<SystemLogRecord> {
    (0..count)
        .map(|i| {
            make_record(
                service,
                start_time + i as u64,
                &format!("{}-record-{}", service, i),
            )
        })
        .collect()
}

#[tokio::test]
async fn test_pipeline_multi_service_fairness_and_consistency() {
    let root = new_temp_root("multi_service_fairness");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_multi_service_fairness test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };

    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-multi";
    let service_hot = "svc_hot";
    let service_a = "svc_a";
    let service_b = "svc_b";

    let records_hot = generate_service_records(service_hot, 1722000300000, 50);
    let records_a = generate_service_records(service_a, 1722000310000, 8);
    let records_b = generate_service_records(service_b, 1722000320000, 7);

    let dir_hot = prepare_service_logs(&root, service_hot, &records_hot).unwrap();
    let dir_a = prepare_service_logs(&root, service_a, &records_a).unwrap();
    let dir_b = prepare_service_logs(&root, service_b, &records_b).unwrap();

    let storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();
    let server = LogHttpServer::new(storage.clone());
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

    // Early fairness check: each service should be observed quickly rather than
    // being starved by the hot service.
    wait_for_service_min_count(
        storage.as_ref().as_ref(),
        node,
        &[service_hot, service_a, service_b],
        1,
        Duration::from_secs(6),
    )
    .await
    .unwrap();

    let mut expected_counts = HashMap::new();
    expected_counts.insert(service_hot, records_hot.len());
    expected_counts.insert(service_a, records_a.len());
    expected_counts.insert(service_b, records_b.len());

    wait_for_expected_counts(
        storage.as_ref().as_ref(),
        node,
        &expected_counts,
        Duration::from_secs(14),
    )
    .await
    .unwrap();

    let mut uploaded_hot = query_service_contents(storage.as_ref().as_ref(), node, service_hot)
        .await
        .unwrap();
    let mut uploaded_a = query_service_contents(storage.as_ref().as_ref(), node, service_a)
        .await
        .unwrap();
    let mut uploaded_b = query_service_contents(storage.as_ref().as_ref(), node, service_b)
        .await
        .unwrap();

    let mut expected_hot: Vec<String> = records_hot.iter().map(|r| r.content.clone()).collect();
    let mut expected_a: Vec<String> = records_a.iter().map(|r| r.content.clone()).collect();
    let mut expected_b: Vec<String> = records_b.iter().map(|r| r.content.clone()).collect();
    uploaded_hot.sort();
    uploaded_a.sort();
    uploaded_b.sort();
    expected_hot.sort();
    expected_a.sort();
    expected_b.sort();

    assert_eq!(uploaded_hot, expected_hot);
    assert_eq!(uploaded_a, expected_a);
    assert_eq!(uploaded_b, expected_b);

    wait_for_read_index_catch_up(&dir_hot, Duration::from_secs(8))
        .await
        .unwrap();
    wait_for_read_index_catch_up(&dir_a, Duration::from_secs(8))
        .await
        .unwrap();
    wait_for_read_index_catch_up(&dir_b, Duration::from_secs(8))
        .await
        .unwrap();

    daemon.shutdown().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;

    std::fs::remove_dir_all(&root).unwrap();
}
