use crate::client::LogDaemonClient;
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::server::LogHttpServer;
use slog_server::storage::{
    LogQueryRequest, LogStorage, LogStorageType, create_log_storage_with_dir,
};
use std::fs::OpenOptions;
use std::io::Write;
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
        file: Some("pipeline_append_runtime_test.rs".to_string()),
        line: Some(1),
        content: content.to_string(),
    }
}

fn prepare_service_with_initial_records(
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
    let log_file = service_dir.join(&file_name);

    let meta = LogMeta::open(&service_dir)?;
    meta.append_new_file(&file_name)
        .map_err(|e| format!("append_new_file failed: {}", e))?;

    let mut content = String::new();
    for record in records {
        content.push_str(&SystemLogRecordLineFormatter::format_record(record));
    }
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

async fn wait_for_uploaded_count(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
    expected_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let count = query_uploaded_contents(storage, node, service).await?.len();
        if count >= expected_count {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting uploaded count >= {}, current={}",
                expected_count, count
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_pipeline_append_logs_during_runtime() {
    let root = new_temp_root("append_during_runtime");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_append_during_runtime test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-append";
    let service = "svc_append_runtime";

    let initial_records = vec![make_record(service, 1722000900001, "append-initial-1")];
    let runtime_appended_records = vec![
        make_record(service, 1722000900002, "append-runtime-2"),
        make_record(service, 1722000900003, "append-runtime-3"),
    ];

    let (service_dir, file_name) =
        prepare_service_with_initial_records(&root, service, &initial_records).unwrap();

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

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        initial_records.len(),
        Duration::from_secs(8),
    )
    .await
    .unwrap();

    append_records_to_active_file(&service_dir, &file_name, &runtime_appended_records).unwrap();

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        initial_records.len() + runtime_appended_records.len(),
        Duration::from_secs(10),
    )
    .await
    .unwrap();

    let mut uploaded = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    let mut expected: Vec<String> = initial_records
        .iter()
        .chain(runtime_appended_records.iter())
        .map(|r| r.content.clone())
        .collect();
    uploaded.sort();
    expected.sort();
    assert_eq!(uploaded, expected);

    daemon.shutdown().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;
    std::fs::remove_dir_all(&root).unwrap();
}
