use crate::client::LogDaemonClient;
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::server::LogHttpServer;
use slog_server::storage::{
    LogQueryRequest, LogRecords, LogStorage, LogStorageType, create_log_storage_with_dir,
};
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

fn reserve_bind_addr() -> Result<(std::net::TcpListener, String), String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("failed to bind reserve listener on loopback: {}", e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("failed to read local reserve listener address: {}", e))?;
    Ok((listener, format!("127.0.0.1:{}", addr.port())))
}

fn make_record(service: &str, time: u64, content: &str) -> SystemLogRecord {
    SystemLogRecord {
        level: LogLevel::Info,
        target: service.to_string(),
        time,
        file: Some("pipeline_recovery_test.rs".to_string()),
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

#[tokio::test]
async fn test_pipeline_server_unavailable_then_recover() {
    let root = new_temp_root("server_recovery");
    let storage_dir = root.join("server_storage");
    let (reserve_listener, bind_addr) = match reserve_bind_addr() {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "skip pipeline_server_recovery test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-recovery";
    let service = "svc_recovery";

    let input_records = vec![
        make_record(service, 1722000200001, "recovery-1"),
        make_record(service, 1722000200002, "recovery-2"),
        make_record(service, 1722000200003, "recovery-3"),
    ];
    let service_dir = prepare_service_logs(&root, service, &input_records).unwrap();

    let daemon = LogDaemonClient::new(
        node.to_string(),
        endpoint,
        1,
        &root,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();

    // Keep server unavailable for one cycle, read position must not be flushed on failed uploads.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let meta_before = LogMeta::open(&service_dir).unwrap();
    let write_info_before = meta_before.get_active_write_file().unwrap().unwrap();
    let file_info_before = meta_before
        .get_file_info(write_info_before.id)
        .unwrap()
        .unwrap();
    assert!(file_info_before.read_index < file_info_before.write_index);

    // Release reserved port and start real server.
    drop(reserve_listener);

    let storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();
    let server = LogHttpServer::new(storage.clone());
    let server_handle = tokio::spawn({
        let bind_addr = bind_addr.clone();
        async move {
            let _ = server.run(&bind_addr).await;
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = wait_for_uploaded_logs(
        storage.as_ref().as_ref(),
        node,
        service,
        input_records.len(),
        Duration::from_secs(16),
    )
    .await
    .unwrap();

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

    wait_for_read_index_catch_up(&service_dir, Duration::from_secs(8))
        .await
        .unwrap();

    daemon.shutdown().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;

    std::fs::remove_dir_all(&root).unwrap();
}
