mod common;

use common::{
    allocate_bind_addr, append_service_logs, build_binaries_for_e2e, make_record, new_temp_root,
    prepare_service_logs, spawn_daemon_process, spawn_server_process, wait_for_tcp_ready_or_process_exit,
    wait_for_uploaded_count,
};
use slog_server::storage::{LogQueryRequest, LogStorage, LogStorageType, create_log_storage_with_dir};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, Instant, MissedTickBehavior};

fn parse_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn parse_env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn ensure_process_alive(process: &mut common::ChildGuard, name: &str) -> Result<(), String> {
    match process
        .child
        .try_wait()
        .map_err(|e| format!("failed to poll {} status: {}", name, e))?
    {
        Some(status) => Err(format!(
            "{} exited unexpectedly: status={}, stderr_tail={}",
            name,
            status,
            process.read_stderr_tail(8192)
        )),
        None => Ok(()),
    }
}

async fn query_records(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
    start_time: Option<u64>,
    end_time: Option<u64>,
    limit: Option<usize>,
) -> Result<Vec<(u64, String)>, String> {
    let rows = storage
        .query_logs(LogQueryRequest {
            node: Some(node.to_string()),
            service: Some(service.to_string()),
            level: None,
            start_time,
            end_time,
            limit,
        })
        .await?;

    let mut logs = Vec::new();
    for row in rows {
        for log in row.logs {
            logs.push((log.time, log.content));
        }
    }
    logs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(logs)
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level query visibility under concurrent writes; run manually when needed"]
async fn test_process_query_visibility() {
    let write_records = parse_env_usize("SLOG_E2E_QUERY_VISIBILITY_RECORDS", 140);
    let write_interval_ms = parse_env_u64("SLOG_E2E_QUERY_VISIBILITY_WRITE_INTERVAL_MS", 80);
    let monitor_interval_ms = parse_env_u64("SLOG_E2E_QUERY_VISIBILITY_MONITOR_INTERVAL_MS", 300);

    let root = new_temp_root("query_visibility");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&log_root).unwrap();
    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_query_visibility";
    let node = "node-query-visibility";
    let base_ts = 1723100000000u64;
    let init_record = make_record(service, base_ts, "init-1");
    prepare_service_logs(&log_root, service, &[init_record]).unwrap();

    let mut server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();
    let mut daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
    let storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        1,
        Duration::from_secs(20),
    )
    .await
    .unwrap();

    let produced = Arc::new(Mutex::new(vec![(base_ts, "init-1".to_string())]));
    let writer_done = Arc::new(AtomicBool::new(false));
    let writer_log_root = log_root.clone();
    let writer_produced = produced.clone();
    let writer_done_flag = writer_done.clone();
    let writer = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(write_interval_ms));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        for i in 0..write_records {
            ticker.tick().await;
            let ts = base_ts + 1 + i as u64;
            let content = format!("qv-{}", i + 1);
            append_service_logs(
                &writer_log_root,
                service,
                &[make_record(service, ts, &content)],
            )?;
            writer_produced
                .lock()
                .map_err(|e| format!("lock produced failed: {}", e))?
                .push((ts, content));
        }
        writer_done_flag.store(true, Ordering::Relaxed);
        Ok::<(), String>(())
    });

    let mut prev_visible_count = 0usize;
    let monitor_deadline = Instant::now() + Duration::from_secs(300);
    while !writer_done.load(Ordering::Relaxed) {
        assert!(
            Instant::now() < monitor_deadline,
            "monitor timeout while writer still running"
        );
        ensure_process_alive(&mut daemon, "slog_daemon").unwrap();

        let snapshot = produced.lock().unwrap().clone();
        let watermark = snapshot.last().map(|x| x.0).unwrap_or(base_ts);
        let snapshot_set: HashSet<String> = snapshot.iter().map(|(_, c)| c.clone()).collect();

        let visible = query_records(
            storage.as_ref().as_ref(),
            node,
            service,
            None,
            Some(watermark),
            None,
        )
        .await
        .unwrap();

        assert!(
            visible.len() >= prev_visible_count,
            "query visibility regressed: prev={}, now={}",
            prev_visible_count,
            visible.len()
        );
        prev_visible_count = visible.len();

        for (ts, content) in &visible {
            assert!(*ts <= watermark);
            assert!(
                snapshot_set.contains(content),
                "query returned unknown content during live write: {}",
                content
            );
        }

        tokio::time::sleep(Duration::from_millis(monitor_interval_ms)).await;
    }

    writer.await.unwrap().unwrap();
    ensure_process_alive(&mut daemon, "slog_daemon after writer").unwrap();

    let snapshot = produced.lock().unwrap().clone();
    let expected_total = snapshot.len();
    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        expected_total,
        Duration::from_secs(90),
    )
    .await
    .unwrap();

    let max_ts = snapshot.last().map(|x| x.0).unwrap_or(base_ts);
    let all = query_records(
        storage.as_ref().as_ref(),
        node,
        service,
        None,
        Some(max_ts),
        None,
    )
    .await
    .unwrap();
    assert_eq!(all.len(), expected_total);

    let limited = query_records(
        storage.as_ref().as_ref(),
        node,
        service,
        None,
        Some(max_ts),
        Some(10),
    )
    .await
    .unwrap();
    assert_eq!(limited.len(), 10usize.min(expected_total));
    assert_eq!(limited, all[..limited.len()].to_vec());
    for _ in 0..4 {
        let again = query_records(
            storage.as_ref().as_ref(),
            node,
            service,
            None,
            Some(max_ts),
            Some(10),
        )
        .await
        .unwrap();
        assert_eq!(again, limited);
    }

    let range_start = max_ts.saturating_sub(60);
    let range_end = max_ts.saturating_sub(20);
    let expected_in_range = snapshot
        .iter()
        .filter(|(ts, _)| *ts >= range_start && *ts <= range_end)
        .count();
    let range_base = query_records(
        storage.as_ref().as_ref(),
        node,
        service,
        Some(range_start),
        Some(range_end),
        None,
    )
    .await
    .unwrap();
    assert_eq!(range_base.len(), expected_in_range);
    for (ts, _) in &range_base {
        assert!(*ts >= range_start && *ts <= range_end);
    }
    for _ in 0..4 {
        let again = query_records(
            storage.as_ref().as_ref(),
            node,
            service,
            Some(range_start),
            Some(range_end),
            None,
        )
        .await
        .unwrap();
        assert_eq!(again, range_base);
    }

    daemon.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
