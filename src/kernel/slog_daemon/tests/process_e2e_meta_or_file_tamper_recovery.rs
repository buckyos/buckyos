mod common;

use common::{
    allocate_bind_addr, append_service_logs, build_binaries_for_e2e, make_record, new_temp_root,
    query_uploaded_contents, query_uploaded_count, spawn_daemon_process, spawn_server_process,
    wait_for_tcp_not_ready, wait_for_tcp_ready_or_process_exit,
};
use slog::{LogMeta, SystemLogRecord, SystemLogRecordLineFormatter};
use slog_server::storage::{LogStorage, LogStorageType, create_log_storage_with_dir};
use std::collections::HashSet;
use std::path::Path;
use tokio::time::{Duration, Instant};

fn write_new_file_with_records(
    service_dir: &Path,
    service: &str,
    records: &[SystemLogRecord],
    seal_after_write: bool,
) -> Result<String, String> {
    let meta = LogMeta::open(service_dir)?;
    let file_name = {
        let last_sealed_id = meta
            .get_last_sealed_file()
            .map_err(|e| format!("failed to get last sealed file: {}", e))?
            .map(|f| f.id)
            .unwrap_or(0);
        let active_id = meta
            .get_active_write_file()
            .map_err(|e| format!("failed to get active write file: {}", e))?
            .map(|f| f.id)
            .unwrap_or(0);
        format!("{}.{}.log", service, std::cmp::max(last_sealed_id, active_id) + 1)
    };

    meta.append_new_file(&file_name)
        .map_err(|e| format!("failed to append new file {}: {}", file_name, e))?;

    let mut content = String::new();
    for record in records {
        content.push_str(&SystemLogRecordLineFormatter::format_record(record));
    }

    let file_path = service_dir.join(&file_name);
    std::fs::write(&file_path, &content)
        .map_err(|e| format!("failed to write {}: {}", file_path.display(), e))?;
    meta.update_current_write_index(content.len() as u64)
        .map_err(|e| format!("failed to update write index for {}: {}", file_name, e))?;

    if seal_after_write {
        meta.seal_current_write_file()
            .map_err(|e| format!("failed to seal {}: {}", file_name, e))?;
    }

    Ok(file_name)
}

fn ensure_process_alive(
    process: &mut common::ChildGuard,
    name: &str,
) -> Result<(), String> {
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

fn reserve_file_ids(service_dir: &Path, service: &str, count: usize) -> Result<(), String> {
    let meta = LogMeta::open(service_dir)?;
    for i in 0..count {
        let file_name = format!("{}.reserved.{}.log", service, i + 1);
        meta.append_new_file(&file_name)
            .map_err(|e| format!("failed to append reserved file {}: {}", file_name, e))?;
        meta.seal_current_write_file()
            .map_err(|e| format!("failed to seal reserved file {}: {}", file_name, e))?;
    }
    Ok(())
}

async fn wait_for_contents_include(
    storage: &dyn LogStorage,
    node: &str,
    service: &str,
    expected_subset: &HashSet<String>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let all = query_uploaded_contents(storage, node, service).await?;
        let existing: HashSet<String> = all.into_iter().collect();
        if expected_subset.is_subset(&existing) {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting expected contents, expected_count={}, got_count={}",
                expected_subset.len(),
                existing.len()
            ));
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level tamper recovery; run manually when needed"]
async fn test_process_meta_or_file_tamper_recovery() {
    let root = new_temp_root("meta_or_file_tamper_recovery");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    let service = "svc_tamper_recovery";
    let node = "node-tamper-recovery";
    let service_dir = log_root.join(service);
    std::fs::create_dir_all(&service_dir).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);

    let mut expected_contents = HashSet::new();
    let base_ts = 1723070000000u64;

    // Initial files:
    //   file1 sealed (10), file2 sealed (10), file3 active (5).
    let baseline_1: Vec<SystemLogRecord> = (0..10usize)
        .map(|i| make_record(service, base_ts + i as u64, &format!("baseline-{}", i + 1)))
        .collect();
    let baseline_2: Vec<SystemLogRecord> = (10..20usize)
        .map(|i| make_record(service, base_ts + i as u64, &format!("baseline-{}", i + 1)))
        .collect();
    let baseline_3: Vec<SystemLogRecord> = (20..25usize)
        .map(|i| make_record(service, base_ts + i as u64, &format!("baseline-{}", i + 1)))
        .collect();
    for rec in baseline_1.iter().chain(baseline_2.iter()).chain(baseline_3.iter()) {
        expected_contents.insert(rec.content.clone());
    }

    write_new_file_with_records(&service_dir, service, &baseline_1, true).unwrap();
    write_new_file_with_records(&service_dir, service, &baseline_2, true).unwrap();
    write_new_file_with_records(&service_dir, service, &baseline_3, false).unwrap();

    let mut server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();
    let mut daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
    let storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();

    wait_for_contents_include(
        storage.as_ref().as_ref(),
        node,
        service,
        &expected_contents,
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    ensure_process_alive(&mut daemon, "slog_daemon").unwrap();

    // Tamper #1: delete one unread sealed file, ensure daemon keeps running and
    // can still upload new records from later files.
    daemon.stop();

    let meta = LogMeta::open(&service_dir).unwrap();
    if meta.get_active_write_file().unwrap().is_some() {
        meta.seal_current_write_file().unwrap();
    }

    let lost_records: Vec<SystemLogRecord> = (25..31usize)
        .map(|i| make_record(service, base_ts + i as u64, &format!("lost-{}", i + 1)))
        .collect();
    let kept_records: Vec<SystemLogRecord> = (31..36usize)
        .map(|i| make_record(service, base_ts + i as u64, &format!("kept-{}", i + 1)))
        .collect();

    let lost_file_name = write_new_file_with_records(&service_dir, service, &lost_records, true)
        .unwrap();
    write_new_file_with_records(&service_dir, service, &kept_records, false).unwrap();

    std::fs::remove_file(service_dir.join(&lost_file_name)).unwrap();
    for rec in &kept_records {
        expected_contents.insert(rec.content.clone());
    }

    daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
    wait_for_contents_include(
        storage.as_ref().as_ref(),
        node,
        service,
        &expected_contents,
        Duration::from_secs(90),
    )
    .await
    .unwrap();
    ensure_process_alive(&mut daemon, "slog_daemon").unwrap();
    expected_contents = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap()
        .into_iter()
        .collect();

    // Tamper #2: corrupt log_meta.db, daemon should not panic.
    daemon.stop();
    std::fs::write(service_dir.join("log_meta.db"), b"invalid sqlite data").unwrap();
    daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    ensure_process_alive(&mut daemon, "slog_daemon after meta corruption").unwrap();

    // Recover by recreating service directory and writing new records.
    daemon.stop();
    std::fs::remove_dir_all(&service_dir).unwrap();
    std::fs::create_dir_all(&service_dir).unwrap();
    reserve_file_ids(&service_dir, service, 32).unwrap();
    let recovered_from_meta_corruption: Vec<SystemLogRecord> = (200..206usize)
        .map(|i| make_record(service, base_ts + i as u64, &format!("meta-recover-{}", i + 1)))
        .collect();
    write_new_file_with_records(
        &service_dir,
        service,
        &recovered_from_meta_corruption,
        false,
    )
    .unwrap();
    for rec in &recovered_from_meta_corruption {
        expected_contents.insert(rec.content.clone());
    }

    daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
    wait_for_contents_include(
        storage.as_ref().as_ref(),
        node,
        service,
        &expected_contents,
        Duration::from_secs(40),
    )
    .await
    .unwrap();
    ensure_process_alive(&mut daemon, "slog_daemon after meta recovery").unwrap();
    expected_contents = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap()
        .into_iter()
        .collect();

    // Tamper #3: delete service directory while upload failures happen.
    // Then wait one update cycle to allow retry-state cleanup and recreate service.
    server.stop();
    wait_for_tcp_not_ready(&bind_addr, Duration::from_secs(5))
        .await
        .unwrap();

    let fail_batch: Vec<SystemLogRecord> = (300..306usize)
        .map(|i| make_record(service, base_ts + i as u64, &format!("fail-batch-{}", i + 1)))
        .collect();
    append_service_logs(&log_root, service, &fail_batch).unwrap();

    // Give daemon time to enter retry flow for this service.
    tokio::time::sleep(Duration::from_secs(3)).await;
    std::fs::remove_dir_all(&service_dir).unwrap();
    ensure_process_alive(&mut daemon, "slog_daemon after deleting service dir").unwrap();

    // Wait for directory rescan cycle to clear stale retry states for removed service.
    tokio::time::sleep(Duration::from_secs(65)).await;

    std::fs::create_dir_all(&service_dir).unwrap();
    reserve_file_ids(&service_dir, service, 64).unwrap();
    let recreated_after_delete: Vec<SystemLogRecord> = (400..405usize)
        .map(|i| make_record(service, base_ts + i as u64, &format!("dir-recover-{}", i + 1)))
        .collect();
    write_new_file_with_records(&service_dir, service, &recreated_after_delete, false).unwrap();
    for rec in &recreated_after_delete {
        expected_contents.insert(rec.content.clone());
    }

    server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();

    // Restart daemon to force immediate rescan of recovered service directory.
    daemon.stop();
    daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();

    wait_for_contents_include(
        storage.as_ref().as_ref(),
        node,
        service,
        &expected_contents,
        Duration::from_secs(40),
    )
    .await
    .unwrap();
    ensure_process_alive(&mut daemon, "slog_daemon after dir recovery").unwrap();

    let final_contents = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    let final_unique: HashSet<String> = final_contents.into_iter().collect();
    assert!(expected_contents.is_subset(&final_unique));
    let final_count = query_uploaded_count(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    assert_eq!(final_count, final_unique.len());

    daemon.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
