mod common;

use common::{
    allocate_bind_addr, append_service_logs, build_binaries_for_e2e, make_record, new_temp_root,
    open_process_e2e_storage, prepare_service_logs, query_uploaded_contents, query_uploaded_count,
    spawn_daemon_process, spawn_server_process, wait_for_tcp_not_ready,
    wait_for_tcp_ready_or_process_exit, wait_for_uploaded_count,
};
use std::collections::HashSet;
use tokio::time::Duration;

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level server restart recovery skeleton; run manually when needed"]
async fn test_process_server_restart_recovery_skeleton() {
    let root = new_temp_root("server_restart_recovery");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&log_root).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_server_recovery";
    let node = "node-restart-recovery";

    let phase1 = vec![
        make_record(service, 1723010000001, "phase1-1"),
        make_record(service, 1723010000002, "phase1-2"),
        make_record(service, 1723010000003, "phase1-3"),
    ];
    let phase2 = vec![
        make_record(service, 1723010001001, "phase2-1"),
        make_record(service, 1723010001002, "phase2-2"),
        make_record(service, 1723010001003, "phase2-3"),
    ];

    prepare_service_logs(&log_root, service, &phase1).unwrap();

    let mut server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();

    let mut daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
    let storage = open_process_e2e_storage(&storage_dir).unwrap();

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        phase1.len(),
        Duration::from_secs(20),
    )
    .await
    .unwrap();

    server.stop();
    wait_for_tcp_not_ready(&bind_addr, Duration::from_secs(5))
        .await
        .unwrap();

    append_service_logs(&log_root, service, &phase2).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        phase1.len() + phase2.len(),
        Duration::from_secs(30),
    )
    .await
    .unwrap();

    let final_count = query_uploaded_count(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    assert_eq!(final_count, phase1.len() + phase2.len());

    let contents = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    let unique: HashSet<String> = contents.iter().cloned().collect();
    assert_eq!(unique.len(), phase1.len() + phase2.len());

    daemon.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
