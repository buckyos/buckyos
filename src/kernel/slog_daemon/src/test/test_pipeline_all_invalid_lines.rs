use crate::client::LogDaemonClient;
use slog::LogMeta;
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

fn prepare_invalid_only_logs(
    log_root: &Path,
    service: &str,
    invalid_lines: &[String],
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
    for line in invalid_lines {
        content.push_str(line);
        if !line.ends_with('\n') {
            content.push('\n');
        }
    }

    let log_file = service_dir.join(&file_name);
    std::fs::write(&log_file, &content)
        .map_err(|e| format!("failed to write log file {}: {}", log_file.display(), e))?;
    meta.update_current_write_index(content.len() as u64)
        .map_err(|e| format!("update_current_write_index failed: {}", e))?;

    Ok(service_dir)
}

async fn wait_for_corrupt_log_lines(
    service_dir: &Path,
    min_line_count: usize,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let path = service_dir.join("corrupt.log");

    loop {
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
            if text.lines().count() >= min_line_count {
                return Ok(text);
            }
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting corrupt.log lines >= {}, path={}",
                min_line_count,
                path.display()
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

async fn query_uploaded_count(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
) -> Result<usize, String> {
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
    Ok(result.iter().map(|records| records.logs.len()).sum())
}

#[tokio::test]
async fn test_pipeline_all_invalid_lines_flushes_progress_and_sidecars_corrupt() {
    let root = new_temp_root("all_invalid_lines");
    let storage_dir = root.join("server_storage");
    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_all_invalid_lines test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-invalid";
    let service = "svc_invalid_only";

    let invalid_lines: Vec<String> = (0..22)
        .map(|i| format!("invalid-payload-line-{}: no valid formatter fields", i))
        .collect();
    let service_dir = prepare_invalid_only_logs(&root, service, &invalid_lines).unwrap();

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

    let corrupt_text =
        wait_for_corrupt_log_lines(&service_dir, invalid_lines.len(), Duration::from_secs(10))
            .await
            .unwrap();
    let corrupt_line_count = corrupt_text.lines().count();
    assert_eq!(corrupt_line_count, invalid_lines.len());

    wait_for_read_index_catch_up(&service_dir, Duration::from_secs(8))
        .await
        .unwrap();

    // Invalid-only input should never produce uploaded rows.
    let uploaded_count = query_uploaded_count(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    assert_eq!(uploaded_count, 0);

    daemon.shutdown().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;
    std::fs::remove_dir_all(&root).unwrap();
}
