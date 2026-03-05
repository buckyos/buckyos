mod common;

use common::{
    allocate_bind_addr, build_binaries_for_e2e, make_record, new_temp_root, query_uploaded_contents,
    query_uploaded_count, spawn_daemon_process, spawn_server_process, wait_for_tcp_not_ready,
    wait_for_tcp_ready_or_process_exit, wait_for_uploaded_count,
};
use slog::{FileLogTarget, LogMeta, SystemLogTarget};
use slog_server::storage::{LogStorageType, create_log_storage_with_dir};
use std::collections::HashSet;
use tokio::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
struct MetaStats {
    file_count: usize,
    sealed_count: usize,
    all_caught_up: bool,
}

fn collect_meta_stats(meta: &LogMeta) -> Result<MetaStats, String> {
    let last_sealed_id = meta
        .get_last_sealed_file()
        .map_err(|e| format!("failed to get last sealed file: {}", e))?
        .map(|f| f.id)
        .unwrap_or(0);
    let active_write_id = meta
        .get_active_write_file()
        .map_err(|e| format!("failed to get active write file: {}", e))?
        .map(|f| f.id)
        .unwrap_or(0);
    let max_id = std::cmp::max(last_sealed_id, active_write_id);

    let mut file_count = 0usize;
    let mut sealed_count = 0usize;
    let mut all_caught_up = true;
    for id in 1..=max_id {
        let info = meta
            .get_file_info(id)
            .map_err(|e| format!("failed to get file info for id={}: {}", id, e))?;
        let Some(info) = info else {
            continue;
        };
        file_count += 1;
        if info.read_index != info.write_index {
            all_caught_up = false;
        }
        if info.is_sealed {
            sealed_count += 1;
            if !info.is_read_complete {
                all_caught_up = false;
            }
        }
    }

    Ok(MetaStats {
        file_count,
        sealed_count,
        all_caught_up,
    })
}

async fn wait_for_meta_caught_up(
    service_dir: &std::path::Path,
    timeout: Duration,
) -> Result<MetaStats, String> {
    let meta = LogMeta::open(service_dir)?;
    let deadline = Instant::now() + timeout;
    loop {
        let stats = collect_meta_stats(&meta)?;
        if stats.all_caught_up {
            return Ok(stats);
        }
        if Instant::now() >= deadline {
            return Err(format!("timeout waiting meta caught up, last_stats={:?}", stats));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level rotation boundary with daemon/server restart; run manually when needed"]
async fn test_process_rotation_boundary_restart() {
    let root = new_temp_root("rotation_boundary_restart");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&log_root).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_rotation_boundary";
    let node = "node-rotation-boundary";
    let service_dir = log_root.join(service);
    std::fs::create_dir_all(&service_dir).unwrap();

    // Small file size and short flush interval to force frequent rotation/seal.
    let file_target = FileLogTarget::new(&service_dir, service.to_string(), 512, 50).unwrap();

    let mut server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();

    let mut daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
    let storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();

    let mut expected_contents = HashSet::new();
    let total_records = 180usize;
    let base_ts = 1723060000000u64;

    // Phase 1: write first third and let daemon start consuming.
    for i in 0..60usize {
        let content = format!("phase1-record-{}", i + 1);
        let record = make_record(service, base_ts + i as u64, &content);
        file_target.log(&record);
        expected_contents.insert(content);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Restart daemon between rotation boundaries.
    daemon.stop();
    daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();

    // Phase 2: continue writing while daemon is up.
    for i in 60..120usize {
        let content = format!("phase2-record-{}", i + 1);
        let record = make_record(service, base_ts + i as u64, &content);
        file_target.log(&record);
        expected_contents.insert(content);
    }
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Restart server in the middle; keep writing during outage.
    server.stop();
    wait_for_tcp_not_ready(&bind_addr, Duration::from_secs(5))
        .await
        .unwrap();

    for i in 120..180usize {
        let content = format!("phase3-record-{}", i + 1);
        let record = make_record(service, base_ts + i as u64, &content);
        file_target.log(&record);
        expected_contents.insert(content);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;

    server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();

    // Another daemon restart to cross upload/read-index boundaries.
    daemon.stop();
    daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        total_records,
        Duration::from_secs(120),
    )
    .await
    .unwrap();

    let final_count = query_uploaded_count(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    assert_eq!(final_count, total_records);

    let contents = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    let unique: HashSet<String> = contents.into_iter().collect();
    assert_eq!(unique.len(), total_records);
    assert_eq!(unique, expected_contents);

    let meta_stats = wait_for_meta_caught_up(&service_dir, Duration::from_secs(20))
        .await
        .unwrap();
    assert!(meta_stats.file_count >= 3);
    assert!(meta_stats.sealed_count >= 2);
    assert!(meta_stats.all_caught_up);

    daemon.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
