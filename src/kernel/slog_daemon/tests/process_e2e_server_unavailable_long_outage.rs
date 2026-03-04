mod common;

use common::{
    allocate_bind_addr, append_service_logs, build_binaries_for_e2e, make_record, new_temp_root,
    prepare_service_logs, query_uploaded_contents, query_uploaded_count, spawn_daemon_process,
    spawn_server_process, wait_for_tcp_not_ready, wait_for_tcp_ready_or_process_exit,
    wait_for_uploaded_count,
};
use slog_server::storage::{LogStorageType, create_log_storage_with_dir};
use std::collections::HashSet;
use tokio::time::Duration;

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level long outage recovery; run manually when needed"]
async fn test_process_server_unavailable_long_outage() {
    let root = new_temp_root("server_long_outage");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&log_root).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_long_outage";
    let node = "node-long-outage";

    let phase1 = vec![
        make_record(service, 1723030000001, "phase1-1"),
        make_record(service, 1723030000002, "phase1-2"),
        make_record(service, 1723030000003, "phase1-3"),
    ];
    let outage_batches = vec![
        vec![
            make_record(service, 1723030001001, "outage-1-1"),
            make_record(service, 1723030001002, "outage-1-2"),
            make_record(service, 1723030001003, "outage-1-3"),
        ],
        vec![
            make_record(service, 1723030002001, "outage-2-1"),
            make_record(service, 1723030002002, "outage-2-2"),
            make_record(service, 1723030002003, "outage-2-3"),
        ],
        vec![
            make_record(service, 1723030003001, "outage-3-1"),
            make_record(service, 1723030003002, "outage-3-2"),
            make_record(service, 1723030003003, "outage-3-3"),
        ],
    ];

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

    server.stop();
    wait_for_tcp_not_ready(&bind_addr, Duration::from_secs(5))
        .await
        .unwrap();

    for batch in &outage_batches {
        append_service_logs(&log_root, service, batch).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // Keep outage for a while to let daemon keep retrying in realistic way.
    tokio::time::sleep(Duration::from_secs(4)).await;

    let count_during_outage = query_uploaded_count(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    assert_eq!(count_during_outage, phase1.len());

    server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();

    let expected_total = phase1.len() + outage_batches.iter().map(|b| b.len()).sum::<usize>();
    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        expected_total,
        Duration::from_secs(60),
    )
    .await
    .unwrap();

    let final_count = query_uploaded_count(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    assert_eq!(final_count, expected_total);

    let contents = query_uploaded_contents(storage.as_ref().as_ref(), node, service)
        .await
        .unwrap();
    let unique: HashSet<String> = contents.iter().cloned().collect();
    assert_eq!(unique.len(), expected_total);

    daemon.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
