mod common;

use common::{
    allocate_bind_addr, build_binaries_for_e2e, make_record, new_temp_root, prepare_service_logs,
    spawn_daemon_process, spawn_server_process, wait_for_tcp_ready_or_process_exit,
    wait_for_uploaded_count,
};
use slog_server::storage::{LogStorageType, create_log_storage_with_dir};
use tokio::time::Duration;

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level multi-node e2e skeleton; run manually when needed"]
async fn test_process_multi_node_pipeline_skeleton() {
    let root = new_temp_root("multi_node_pipeline");
    let node_a_root = root.join("node_a_logs");
    let node_b_root = root.join("node_b_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&node_a_root).unwrap();
    std::fs::create_dir_all(&node_b_root).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_process_e2e";
    let node_a = "node-process-a";
    let node_b = "node-process-b";

    let records_a = vec![
        make_record(service, 1723000000001, "process-a-1"),
        make_record(service, 1723000000002, "process-a-2"),
    ];
    let records_b = vec![
        make_record(service, 1723000001001, "process-b-1"),
        make_record(service, 1723000001002, "process-b-2"),
        make_record(service, 1723000001003, "process-b-3"),
    ];
    prepare_service_logs(&node_a_root, service, &records_a).unwrap();
    prepare_service_logs(&node_b_root, service, &records_b).unwrap();

    let mut server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();

    let mut daemon_a = spawn_daemon_process(node_a, &endpoint, &node_a_root, 3).unwrap();
    let mut daemon_b = spawn_daemon_process(node_b, &endpoint, &node_b_root, 3).unwrap();

    let storage = create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir).unwrap();

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node_a,
        service,
        records_a.len(),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node_b,
        service,
        records_b.len(),
        Duration::from_secs(20),
    )
    .await
    .unwrap();

    daemon_a.stop();
    daemon_b.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
