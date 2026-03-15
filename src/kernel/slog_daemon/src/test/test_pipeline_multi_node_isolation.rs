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
        file: Some("pipeline_multi_node_test.rs".to_string()),
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

async fn query_uploaded_contents(
    storage: &dyn LogStorage,
    node: Option<&str>,
    service: &str,
) -> Result<HashMap<String, Vec<String>>, String> {
    let result = storage
        .query_logs(LogQueryRequest {
            node: node.map(|n| n.to_string()),
            service: Some(service.to_string()),
            level: None,
            start_time: None,
            end_time: None,
            limit: Some(10_000),
        })
        .await?;

    let mut by_node: HashMap<String, Vec<String>> = HashMap::new();
    for item in result {
        let mut contents: Vec<String> = item.logs.into_iter().map(|l| l.content).collect();
        contents.sort();
        by_node.insert(item.node, contents);
    }
    Ok(by_node)
}

async fn wait_for_node_service_count(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
    expected_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let by_node = query_uploaded_contents(storage, Some(node), service).await?;
        let count = by_node.get(node).map(|v| v.len()).unwrap_or(0);
        if count >= expected_count {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting node={}, service={}, expected_count={}, current={}",
                node, service, expected_count, count
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_pipeline_multi_node_same_service_isolated_by_node() {
    let root = new_temp_root("multi_node_isolation");
    let root_a = root.join("node_a_logs");
    let root_b = root.join("node_b_logs");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    let storage_dir = root.join("server_storage");

    let bind_addr = match allocate_bind_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "skip pipeline_multi_node_isolation test due socket restriction: {}",
                e
            );
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
    };

    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_shared";
    let node_a = "node-A";
    let node_b = "node-B";

    let records_a = vec![
        make_record(service, 1722001300001, "node-a-1"),
        make_record(service, 1722001300002, "node-a-2"),
    ];
    let records_b = vec![
        make_record(service, 1722001301001, "node-b-1"),
        make_record(service, 1722001301002, "node-b-2"),
        make_record(service, 1722001301003, "node-b-3"),
    ];
    prepare_service_logs(&root_a, service, &records_a).unwrap();
    prepare_service_logs(&root_b, service, &records_b).unwrap();

    let storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();
    let server = LogHttpServer::new(storage.clone());
    let server_handle = tokio::spawn({
        let bind_addr = bind_addr.clone();
        async move {
            let _ = server.run(&bind_addr).await;
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let daemon_a = LogDaemonClient::new(
        node_a.to_string(),
        endpoint.clone(),
        3,
        &root_a,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();
    let daemon_b = LogDaemonClient::new(
        node_b.to_string(),
        endpoint,
        3,
        &root_b,
        vec!["slog_daemon".to_string(), "slog_server".to_string()],
    )
    .unwrap();

    wait_for_node_service_count(
        storage.as_ref().as_ref(),
        node_a,
        service,
        records_a.len(),
        Duration::from_secs(12),
    )
    .await
    .unwrap();
    wait_for_node_service_count(
        storage.as_ref().as_ref(),
        node_b,
        service,
        records_b.len(),
        Duration::from_secs(12),
    )
    .await
    .unwrap();

    let all_nodes = query_uploaded_contents(storage.as_ref().as_ref(), None, service)
        .await
        .unwrap();
    let mut expected_a: Vec<String> = records_a.iter().map(|r| r.content.clone()).collect();
    let mut expected_b: Vec<String> = records_b.iter().map(|r| r.content.clone()).collect();
    expected_a.sort();
    expected_b.sort();
    assert_eq!(
        all_nodes.get(node_a).cloned().unwrap_or_default(),
        expected_a
    );
    assert_eq!(
        all_nodes.get(node_b).cloned().unwrap_or_default(),
        expected_b
    );

    daemon_a.shutdown().await.unwrap();
    daemon_b.shutdown().await.unwrap();
    server_handle.abort();
    let _ = server_handle.await;
    std::fs::remove_dir_all(&root).unwrap();
}
