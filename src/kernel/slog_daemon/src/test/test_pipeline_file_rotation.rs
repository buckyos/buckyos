use crate::client::LogDaemonClient;
use slog::{LogLevel, LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::server::LogHttpServer;
use slog_server::storage::{
    LogQueryRequest, LogStorage, LogStorageType, create_log_storage_with_dir,
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
        file: Some("pipeline_file_rotation_test.rs".to_string()),
        line: Some(1),
        content: content.to_string(),
    }
}

fn prepare_rotated_logs(
    log_root: &Path,
    service: &str,
    file1_records: &[SystemLogRecord],
    file2_records: &[SystemLogRecord],
) -> Result<(PathBuf, i64, i64), String> {
    let service_dir = log_root.join(service);
    std::fs::create_dir_all(&service_dir).map_err(|e| {
        format!(
            "failed to create service log dir {}: {}",
            service_dir.display(),
            e
        )
    })?;

    let meta = LogMeta::open(&service_dir)?;

    let file1_name = format!("{}.1.log", service);
    meta.append_new_file(&file1_name)
        .map_err(|e| format!("append_new_file(file1) failed: {}", e))?;
    let mut file1_content = String::new();
    for record in file1_records {
        file1_content.push_str(&SystemLogRecordLineFormatter::format_record(record));
    }
    std::fs::write(service_dir.join(&file1_name), &file1_content)
        .map_err(|e| format!("failed to write file1: {}", e))?;
    meta.update_current_write_index(file1_content.len() as u64)
        .map_err(|e| format!("update_current_write_index(file1) failed: {}", e))?;
    meta.seal_current_write_file()
        .map_err(|e| format!("seal_current_write_file(file1) failed: {}", e))?;
    let file1_id = meta.get_last_sealed_file().unwrap().unwrap().id;

    let file2_name = format!("{}.2.log", service);
    meta.append_new_file(&file2_name)
        .map_err(|e| format!("append_new_file(file2) failed: {}", e))?;
    let mut file2_content = String::new();
    for record in file2_records {
        file2_content.push_str(&SystemLogRecordLineFormatter::format_record(record));
    }
    std::fs::write(service_dir.join(&file2_name), &file2_content)
        .map_err(|e| format!("failed to write file2: {}", e))?;
    meta.update_current_write_index(file2_content.len() as u64)
        .map_err(|e| format!("update_current_write_index(file2) failed: {}", e))?;
    let file2_id = meta.get_active_write_file().unwrap().unwrap().id;

    Ok((service_dir, file1_id, file2_id))
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

async fn wait_for_sealed_file_read_complete(
    service_dir: &Path,
    file_id: i64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let meta = LogMeta::open(service_dir)?;

    loop {
        let file = meta
            .get_file_info(file_id)
            .map_err(|e| format!("failed to get file info {}: {}", file_id, e))?
            .ok_or_else(|| format!("file info missing for {}", file_id))?;
        if file.read_index == file.write_index && file.is_read_complete {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting sealed file complete for {}, read_index={}, write_index={}, is_read_complete={}",
                file_id, file.read_index, file.write_index, file.is_read_complete
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_active_file_catch_up(
    service_dir: &Path,
    file_id: i64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let meta = LogMeta::open(service_dir)?;

    loop {
        let file = meta
            .get_file_info(file_id)
            .map_err(|e| format!("failed to get file info {}: {}", file_id, e))?
            .ok_or_else(|| format!("file info missing for {}", file_id))?;
        if file.read_index == file.write_index {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting active file catch-up for {}, read_index={}, write_index={}",
                file_id, file.read_index, file.write_index
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_pipeline_file_rotation_reads_sealed_then_active_file() {
    let root = new_temp_root("file_rotation");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_file_rotation test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };

    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-rotation";
    let service = "svc_rotation";

    let file1_records = vec![
        make_record(service, 1722000700001, "rotation-f1-1"),
        make_record(service, 1722000700002, "rotation-f1-2"),
    ];
    let file2_records = vec![
        make_record(service, 1722000701001, "rotation-f2-1"),
        make_record(service, 1722000701002, "rotation-f2-2"),
        make_record(service, 1722000701003, "rotation-f2-3"),
    ];

    let (service_dir, file1_id, file2_id) =
        prepare_rotated_logs(&root, service, &file1_records, &file2_records).unwrap();

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
        file1_records.len() + file2_records.len(),
        Duration::from_secs(12),
    )
    .await
    .unwrap();

    let mut uploaded_contents = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    let mut expected_contents: Vec<String> = file1_records
        .iter()
        .chain(file2_records.iter())
        .map(|r| r.content.clone())
        .collect();
    uploaded_contents.sort();
    expected_contents.sort();
    assert_eq!(uploaded_contents, expected_contents);

    wait_for_sealed_file_read_complete(&service_dir, file1_id, Duration::from_secs(8))
        .await
        .unwrap();
    wait_for_active_file_catch_up(&service_dir, file2_id, Duration::from_secs(8))
        .await
        .unwrap();

    daemon.shutdown().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;
    std::fs::remove_dir_all(&root).unwrap();
}
