mod common;

use common::{
    allocate_bind_addr, append_service_logs, build_binaries_for_e2e, make_record, new_temp_root,
    open_process_e2e_storage, prepare_service_logs, query_uploaded_contents, spawn_daemon_process,
    spawn_server_process, wait_for_tcp_ready_or_process_exit, wait_for_uploaded_count,
};
use slog_server::storage::LogQueryRequest;
use std::collections::{HashMap, HashSet};
use tokio::time::Duration;

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

#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level multi-node same-service isolation; run manually when needed"]
async fn test_process_multi_node_same_service_name() {
    let root = new_temp_root("multi_node_same_service_name");
    let node_a_root = root.join("node_a_logs");
    let node_b_root = root.join("node_b_logs");
    let storage_dir = root.join("server_storage");
    std::fs::create_dir_all(&node_a_root).unwrap();
    std::fs::create_dir_all(&node_b_root).unwrap();

    build_binaries_for_e2e().unwrap();

    let bind_addr = allocate_bind_addr().unwrap();
    let endpoint = format!("http://{}/logs", bind_addr);
    let service = "svc_same_name";
    let node_a = "node-same-service-a";
    let node_b = "node-same-service-b";

    let phase1_a: Vec<_> = (0..6usize)
        .map(|i| {
            make_record(
                service,
                1723090000000 + i as u64,
                &format!("A-phase1-{}", i + 1),
            )
        })
        .collect();
    let phase1_b: Vec<_> = (0..9usize)
        .map(|i| {
            make_record(
                service,
                1723090001000 + i as u64,
                &format!("B-phase1-{}", i + 1),
            )
        })
        .collect();

    let mut expected_a: HashSet<String> = phase1_a.iter().map(|r| r.content.clone()).collect();
    let mut expected_b: HashSet<String> = phase1_b.iter().map(|r| r.content.clone()).collect();

    prepare_service_logs(&node_a_root, service, &phase1_a).unwrap();
    prepare_service_logs(&node_b_root, service, &phase1_b).unwrap();

    let mut server = spawn_server_process(&bind_addr, &storage_dir).unwrap();
    wait_for_tcp_ready_or_process_exit(&bind_addr, &mut server, Duration::from_secs(10))
        .await
        .unwrap();

    let mut daemon_a = spawn_daemon_process(node_a, &endpoint, &node_a_root, 3).unwrap();
    let mut daemon_b = spawn_daemon_process(node_b, &endpoint, &node_b_root, 3).unwrap();
    let storage = open_process_e2e_storage(&storage_dir).unwrap();

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node_a,
        service,
        expected_a.len(),
        Duration::from_secs(25),
    )
    .await
    .unwrap();
    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node_b,
        service,
        expected_b.len(),
        Duration::from_secs(25),
    )
    .await
    .unwrap();

    let phase2_a: Vec<_> = (0..5usize)
        .map(|i| {
            make_record(
                service,
                1723090002000 + i as u64,
                &format!("A-phase2-{}", i + 1),
            )
        })
        .collect();
    let phase2_b: Vec<_> = (0..4usize)
        .map(|i| {
            make_record(
                service,
                1723090003000 + i as u64,
                &format!("B-phase2-{}", i + 1),
            )
        })
        .collect();
    for rec in &phase2_a {
        expected_a.insert(rec.content.clone());
    }
    for rec in &phase2_b {
        expected_b.insert(rec.content.clone());
    }
    append_service_logs(&node_a_root, service, &phase2_a).unwrap();
    append_service_logs(&node_b_root, service, &phase2_b).unwrap();

    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node_a,
        service,
        expected_a.len(),
        Duration::from_secs(35),
    )
    .await
    .unwrap();
    wait_for_uploaded_count(
        storage.as_ref().as_ref(),
        node_b,
        service,
        expected_b.len(),
        Duration::from_secs(35),
    )
    .await
    .unwrap();

    ensure_process_alive(&mut daemon_a, "slog_daemon_node_a").unwrap();
    ensure_process_alive(&mut daemon_b, "slog_daemon_node_b").unwrap();

    let contents_a = query_uploaded_contents(storage.as_ref().as_ref(), node_a, service)
        .await
        .unwrap();
    let contents_b = query_uploaded_contents(storage.as_ref().as_ref(), node_b, service)
        .await
        .unwrap();
    let set_a: HashSet<String> = contents_a.into_iter().collect();
    let set_b: HashSet<String> = contents_b.into_iter().collect();

    assert_eq!(set_a, expected_a);
    assert_eq!(set_b, expected_b);
    assert!(
        set_a.is_disjoint(&set_b),
        "node A/B content should not overlap for this test dataset"
    );

    let rows = storage
        .as_ref()
        .as_ref()
        .query_logs(LogQueryRequest {
            node: None,
            service: Some(service.to_string()),
            level: None,
            start_time: None,
            end_time: None,
            limit: Some(5000),
        })
        .await
        .unwrap();

    let mut by_node = HashMap::<String, HashSet<String>>::new();
    for row in rows {
        by_node
            .entry(row.node)
            .or_default()
            .extend(row.logs.into_iter().map(|r| r.content));
    }

    assert_eq!(by_node.len(), 2);
    assert_eq!(by_node.get(node_a).cloned().unwrap_or_default(), expected_a);
    assert_eq!(by_node.get(node_b).cloned().unwrap_or_default(), expected_b);

    daemon_a.stop();
    daemon_b.stop();
    server.stop();
    std::fs::remove_dir_all(&root).unwrap();
}
