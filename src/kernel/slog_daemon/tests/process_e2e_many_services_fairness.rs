mod common;

use common::{
    allocate_bind_addr, append_service_logs, build_binaries_for_e2e, make_record, new_temp_root,
    open_process_e2e_storage, prepare_service_logs, query_uploaded_counts_by_service,
    spawn_daemon_process, spawn_server_process, wait_for_process_exit,
    wait_for_tcp_ready_or_process_exit,
};
use std::collections::HashMap;
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

async fn wait_all_services_uploaded_min_count(
    storage: &dyn slog_server::storage::LogStorage,
    node: &str,
    services: &[String],
    min_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let counts = query_uploaded_counts_by_service(storage, node).await?;
        let ok = services
            .iter()
            .all(|svc| counts.get(svc).copied().unwrap_or(0) >= min_count);
        if ok {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting all services upload >= {}, currently have {} services",
                min_count,
                counts.len()
            ));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level many-services fairness test; run manually when needed"]
async fn test_process_many_services_fairness() {
    let total_services = parse_env_usize("SLOG_E2E_FAIRNESS_SERVICES", 60).clamp(50, 200);
    let hot_services = parse_env_usize("SLOG_E2E_FAIRNESS_HOT_SERVICES", 5).min(total_services);
    let hot_interval_ms = parse_env_u64("SLOG_E2E_FAIRNESS_HOT_INTERVAL_MS", 100);
    let non_hot_interval_ms = parse_env_u64("SLOG_E2E_FAIRNESS_NON_HOT_INTERVAL_MS", 2000);
    let stress_secs = parse_env_u64("SLOG_E2E_FAIRNESS_STRESS_SECS", 45);
    let warmup_secs = parse_env_u64("SLOG_E2E_FAIRNESS_WARMUP_SECS", 8).min(stress_secs);
    let monitor_interval_secs = parse_env_u64("SLOG_E2E_FAIRNESS_MONITOR_INTERVAL_SECS", 2);
    let max_stall_secs = parse_env_u64("SLOG_E2E_FAIRNESS_MAX_STALL_SECS", 20);
    let catch_up_secs = parse_env_u64("SLOG_E2E_FAIRNESS_CATCHUP_SECS", 90);

    let root = new_temp_root("many_services_fairness");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&log_root).unwrap();
    build_binaries_for_e2e().unwrap();

    let node = "node-many-services-fairness";
    let service_names: Vec<String> = (0..total_services)
        .map(|i| format!("svc_fair_{:03}", i + 1))
        .collect();
    let hot_set: std::collections::HashSet<String> =
        service_names.iter().take(hot_services).cloned().collect();
    let non_hot_services: Vec<String> = service_names
        .iter()
        .filter(|s| !hot_set.contains(*s))
        .cloned()
        .collect();

    let written_counts = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let mut base_ts = 1723080000000u64;
    for service in &service_names {
        let init = make_record(service, base_ts, &format!("{}-init-1", service));
        base_ts += 1000;
        prepare_service_logs(&log_root, service, &[init]).unwrap();
        written_counts.lock().unwrap().insert(service.clone(), 1);
    }

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);

    let mut server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();
    let mut daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
    let storage = open_process_e2e_storage(&storage_dir).unwrap();

    wait_all_services_uploaded_min_count(
        storage.as_ref().as_ref(),
        node,
        &service_names,
        1,
        Duration::from_secs(60),
    )
    .await
    .unwrap();

    let mut writer_tasks = Vec::new();
    for (idx, service) in service_names.iter().enumerate() {
        let service = service.clone();
        let interval_ms = if idx < hot_services {
            hot_interval_ms
        } else {
            non_hot_interval_ms
        };
        let local_log_root = log_root.clone();
        let local_stop = stop_flag.clone();
        let local_written = written_counts.clone();
        let task = tokio::spawn(async move {
            let mut seq = 2usize;
            let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if local_stop.load(Ordering::Relaxed) {
                    break;
                }
                let content = format!("{}-seq-{}", service, seq);
                let record = make_record(&service, 1723081000000 + seq as u64, &content);
                append_service_logs(&local_log_root, &service, &[record])?;
                let mut map = local_written
                    .lock()
                    .map_err(|e| format!("lock written_counts failed: {}", e))?;
                *map.entry(service.clone()).or_insert(0) += 1;
                seq += 1;
            }
            Ok::<(), String>(())
        });
        writer_tasks.push(task);
    }

    let start = Instant::now();
    let end = start + Duration::from_secs(stress_secs);
    let mut last_uploaded: HashMap<String, usize> = HashMap::new();
    let mut last_progress: HashMap<String, Instant> = HashMap::new();
    for svc in &non_hot_services {
        last_uploaded.insert(svc.clone(), 1);
        last_progress.insert(svc.clone(), Instant::now());
    }
    let mut warmup_uploaded: Option<HashMap<String, usize>> = None;

    while Instant::now() < end {
        tokio::time::sleep(Duration::from_secs(monitor_interval_secs)).await;
        ensure_process_alive(&mut daemon, "slog_daemon").unwrap();

        let uploaded = query_uploaded_counts_by_service(storage.as_ref().as_ref(), node)
            .await
            .unwrap();
        let written_snapshot = written_counts.lock().unwrap().clone();

        if warmup_uploaded.is_none() && Instant::now() >= start + Duration::from_secs(warmup_secs) {
            warmup_uploaded = Some(uploaded.clone());
        }

        for svc in &non_hot_services {
            let up_now = uploaded.get(svc).copied().unwrap_or(0);
            let up_prev = last_uploaded.get(svc).copied().unwrap_or(0);
            if up_now > up_prev {
                last_progress.insert(svc.clone(), Instant::now());
            }
            last_uploaded.insert(svc.clone(), up_now);

            let written = written_snapshot.get(svc).copied().unwrap_or(0);
            if written > up_now {
                let last = last_progress.get(svc).copied().unwrap_or(start);
                assert!(
                    Instant::now().duration_since(last).as_secs() <= max_stall_secs,
                    "service {} starved too long: written={}, uploaded={}, no progress_for={}s",
                    svc,
                    written,
                    up_now,
                    Instant::now().duration_since(last).as_secs()
                );
            }
        }
    }

    stop_flag.store(true, Ordering::Relaxed);
    for task in writer_tasks {
        let join = task.await.unwrap();
        join.unwrap();
    }

    let final_written = written_counts.lock().unwrap().clone();
    let expected_non_hot_after_warmup =
        ((stress_secs.saturating_sub(warmup_secs) * 1000) / non_hot_interval_ms) as usize;
    let min_non_hot_progress = std::cmp::max(3usize, expected_non_hot_after_warmup / 3);
    let warmup_snapshot = warmup_uploaded.unwrap_or_default();
    let uploaded_end = query_uploaded_counts_by_service(storage.as_ref().as_ref(), node)
        .await
        .unwrap();

    for svc in &non_hot_services {
        let end_count = uploaded_end.get(svc).copied().unwrap_or(0);
        let warmup_count = warmup_snapshot.get(svc).copied().unwrap_or(0);
        let progress = end_count.saturating_sub(warmup_count);
        assert!(
            progress >= min_non_hot_progress,
            "service {} throughput too low during stress: progress={}, min_required={}",
            svc,
            progress,
            min_non_hot_progress
        );
    }

    let catch_deadline = Instant::now() + Duration::from_secs(catch_up_secs);
    loop {
        let uploaded_now = query_uploaded_counts_by_service(storage.as_ref().as_ref(), node)
            .await
            .unwrap();
        let all_caught_up = service_names.iter().all(|svc| {
            uploaded_now.get(svc).copied().unwrap_or(0) >= *final_written.get(svc).unwrap_or(&0)
        });
        if all_caught_up {
            break;
        }
        assert!(
            Instant::now() < catch_deadline,
            "timeout waiting catch-up for all services"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    daemon.stop();
    let _ = wait_for_process_exit(&mut daemon, Duration::from_secs(1)).await;
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
