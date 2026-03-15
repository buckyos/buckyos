mod common;

use common::{
    allocate_bind_addr, append_service_logs, build_binaries_for_e2e, make_record, new_temp_root,
    open_process_e2e_storage, prepare_service_logs, query_uploaded_contents, query_uploaded_count,
    spawn_daemon_process, spawn_server_process, wait_for_tcp_ready_or_process_exit,
    wait_for_uploaded_count,
};
use std::collections::HashSet;
use tokio::time::Duration;

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level daemon kill/restart around random points; run manually when needed"]
async fn test_process_daemon_kill_at_random_points() {
    let root = new_temp_root("daemon_kill_random_points");
    let log_root = root.join("node_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&log_root).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_daemon_kill_random";
    let node = "node-daemon-kill-random";

    let initial = vec![
        make_record(service, 1723040000001, "initial-1"),
        make_record(service, 1723040000002, "initial-2"),
        make_record(service, 1723040000003, "initial-3"),
        make_record(service, 1723040000004, "initial-4"),
    ];
    prepare_service_logs(&log_root, service, &initial).unwrap();

    let mut expected_content_set: HashSet<String> =
        initial.iter().map(|r| r.content.clone()).collect();
    let mut expected_total = initial.len();

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
        initial.len(),
        Duration::from_secs(20),
    )
    .await
    .unwrap();

    let kill_delays_ms: [u64; 6] = [120, 260, 80, 310, 150, 220];
    for (round, delay_ms) in kill_delays_ms.iter().enumerate() {
        let base_time = 1723040001000 + (round as u64) * 100;
        let batch = vec![
            make_record(
                service,
                base_time + 1,
                &format!("round-{}-record-1", round + 1),
            ),
            make_record(
                service,
                base_time + 2,
                &format!("round-{}-record-2", round + 1),
            ),
            make_record(
                service,
                base_time + 3,
                &format!("round-{}-record-3", round + 1),
            ),
        ];

        if round % 2 == 0 {
            // Append while daemon is running, then kill soon after append.
            append_service_logs(&log_root, service, &batch).unwrap();
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            daemon.stop();
            tokio::time::sleep(Duration::from_millis(100)).await;
            daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
        } else {
            // Kill first, append during downtime, then restart daemon.
            daemon.stop();
            append_service_logs(&log_root, service, &batch).unwrap();
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            daemon = spawn_daemon_process(node, &endpoint, &log_root, 3).unwrap();
        }

        expected_total += batch.len();
        for rec in &batch {
            expected_content_set.insert(rec.content.clone());
        }
    }

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node,
        service,
        expected_total,
        Duration::from_secs(80),
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
    assert_eq!(unique, expected_content_set);

    daemon.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
