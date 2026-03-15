mod common;

use common::{
    allocate_bind_addr, build_binaries_for_e2e, make_record, new_temp_root,
    open_process_e2e_storage, prepare_service_logs, query_uploaded_contents,
    query_uploaded_counts_by_service, spawn_daemon_process_with_concurrency, spawn_server_process,
    wait_for_tcp_ready_or_process_exit,
};
use slog::LogMeta;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tokio::time::{Duration, Instant};

fn parse_env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

async fn wait_for_all_expected_counts(
    storage: &dyn slog_server::storage::LogStorage,
    node: &str,
    expected_counts: &HashMap<String, usize>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let uploaded = query_uploaded_counts_by_service(storage, node).await?;
        let all_ready = expected_counts
            .iter()
            .all(|(service, expected)| uploaded.get(service).copied().unwrap_or(0) >= *expected);
        if all_ready {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting expected counts for node={}, expected={:?}, current={:?}",
                node, expected_counts, uploaded
            ));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
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
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level concurrent multi-service integrity test; run manually when needed"]
async fn test_process_concurrent_multi_service_integrity() {
    let service_count = parse_env_usize("SLOG_E2E_CONCURRENT_SERVICES", 24).clamp(12, 80);
    let per_service_records = parse_env_usize("SLOG_E2E_CONCURRENT_PER_SERVICE", 30).clamp(10, 80);
    let upload_concurrency = parse_env_usize("SLOG_E2E_CONCURRENT_UPLOAD", 12).clamp(2, 32);

    let root = new_temp_root("concurrent_multi_service_integrity");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&log_root).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let node = "node-e2e-concurrent-integrity";

    let mut expected_counts = HashMap::<String, usize>::new();
    let mut expected_contents = HashMap::<String, Vec<String>>::new();

    for svc_idx in 0..service_count {
        let service = format!("svc_e2e_concurrent_{:03}", svc_idx + 1);
        let base_time = 1723100000000 + svc_idx as u64 * 10_000;
        let records = (0..per_service_records)
            .map(|i| {
                make_record(
                    &service,
                    base_time + i as u64,
                    &format!("{}-record-{}", service, i),
                )
            })
            .collect::<Vec<_>>();

        prepare_service_logs(&log_root, &service, &records).unwrap();
        expected_counts.insert(service.clone(), records.len());
        expected_contents.insert(
            service.clone(),
            records.into_iter().map(|r| r.content).collect(),
        );
    }

    let mut server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(12))
        .await
        .unwrap();

    let mut daemon = spawn_daemon_process_with_concurrency(
        node,
        &endpoint,
        &log_root,
        5,
        Some(upload_concurrency),
    )
    .unwrap();

    let storage = open_process_e2e_storage(&storage_dir).unwrap();
    wait_for_all_expected_counts(
        storage.as_ref().as_ref(),
        node,
        &expected_counts,
        Duration::from_secs(90),
    )
    .await
    .unwrap();

    for (service, expected) in &expected_contents {
        let mut uploaded = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
            .await
            .unwrap();
        let mut expected = expected.clone();
        uploaded.sort();
        expected.sort();

        let unique: HashSet<String> = uploaded.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            uploaded.len(),
            "duplicate logs detected for service {}",
            service
        );
        assert_eq!(
            uploaded, expected,
            "content mismatch for service {}",
            service
        );
    }

    for service in expected_counts.keys() {
        let service_dir = log_root.join(service);
        wait_for_read_index_catch_up(&service_dir, Duration::from_secs(30))
            .await
            .unwrap();
    }

    daemon.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
