mod common;

use common::{
    allocate_bind_addr, append_service_logs, build_binaries_for_e2e, make_record, new_temp_root,
    prepare_service_logs, query_uploaded_contents, query_uploaded_count, send_sigint,
    spawn_daemon_process, spawn_server_process, wait_for_process_exit,
    wait_for_tcp_ready_or_process_exit, wait_for_uploaded_count,
};
use slog_server::storage::{LogStorageType, create_log_storage_with_dir};
use std::collections::HashSet;
use tokio::time::Duration;

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level graceful shutdown drain; run manually when needed"]
async fn test_process_graceful_shutdown_drain() {
    let root = new_temp_root("graceful_shutdown_drain");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&log_root).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_graceful_shutdown";
    let node = "node-graceful-shutdown";

    let mut expected_contents = HashSet::new();

    let phase1 = vec![
        make_record(service, 1723050000001, "phase1-1"),
        make_record(service, 1723050000002, "phase1-2"),
        make_record(service, 1723050000003, "phase1-3"),
        make_record(service, 1723050000004, "phase1-4"),
        make_record(service, 1723050000005, "phase1-5"),
    ];
    for rec in &phase1 {
        expected_contents.insert(rec.content.clone());
    }

    let mut backlog = Vec::new();
    for i in 0..60u64 {
        let content = format!("backlog-{}", i + 1);
        backlog.push(make_record(service, 1723050001000 + i, &content));
        expected_contents.insert(content);
    }

    prepare_service_logs(&log_root, service, &phase1).unwrap();

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
        phase1.len(),
        Duration::from_secs(20),
    )
    .await
    .unwrap();

    append_service_logs(&log_root, service, &backlog).unwrap();

    // Let daemon consume part of backlog, then trigger graceful shutdown.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    send_sigint(&daemon).unwrap();
    let exit_status = wait_for_process_exit(&mut daemon, Duration::from_secs(15))
        .await
        .unwrap();
    assert!(
        exit_status.success(),
        "daemon did not exit successfully after SIGINT: {}",
        exit_status
    );

    let expected_total = phase1.len() + backlog.len();
    let count_after_shutdown = query_uploaded_count(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();

    // At least one backlog chunk should be drained before exit.
    assert!(
        count_after_shutdown > phase1.len(),
        "no backlog records drained before graceful exit, got {}",
        count_after_shutdown
    );

    if count_after_shutdown < expected_total {
        daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
        wait_for_uploaded_count(
            storage.as_ref().as_ref(),
            node,
            service,
            expected_total,
            Duration::from_secs(80),
        )
        .await
        .unwrap();
    }

    let final_count = query_uploaded_count(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    assert_eq!(final_count, expected_total);

    let contents = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    let unique: HashSet<String> = contents.iter().cloned().collect();
    assert_eq!(unique.len(), expected_total);
    assert_eq!(unique, expected_contents);

    daemon.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
