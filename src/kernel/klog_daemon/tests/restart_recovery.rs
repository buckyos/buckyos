mod common;

use common::*;
use klog::network::{KLogMetaPutRequest, KLogMetaQueryRequest};
use klog::rpc::KLogClient;
use std::time::Duration;
use tokio::time::sleep;

fn rpc_client(rpc_port: u16, request_node_id: u64) -> KLogClient {
    KLogClient::from_daemon_addr(format!("127.0.0.1:{}", rpc_port).as_str(), request_node_id)
        .with_timeout(Duration::from_secs(3))
}

fn find_node_index(nodes: &[TestNode], node_id: u64) -> Result<usize, String> {
    nodes
        .iter()
        .position(|n| n.node_id == node_id)
        .ok_or_else(|| format!("node index not found for node_id={}", node_id))
}

#[tokio::test]
async fn test_three_voter_full_restart_recovers_membership_leader_and_writes() -> Result<(), String>
{
    if !can_bind_localhost() {
        eprintln!("skip full-restart recovery test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_full_restart_recovery_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_before =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_rpc_before = rpc_port_by_node_id(&nodes, leader_before)?;

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;
        let meta_key = format!("cluster/restart/meta/{}", port1);
        let meta_client_before = rpc_client(leader_rpc_before, 9201);

        let append_before_1 = append_log(
            &http,
            leader_rpc_before,
            "before-full-restart-1",
            Some(3001),
            Some(leader_before),
        )
        .await?;
        let append_before_2 = append_log(
            &http,
            leader_rpc_before,
            "before-full-restart-2",
            Some(3002),
            Some(leader_before),
        )
        .await?;
        if append_before_2.id <= append_before_1.id {
            return Err(format!(
                "append id not increasing before restart: first_id={}, second_id={}",
                append_before_1.id, append_before_2.id
            ));
        }

        let meta_before = meta_client_before
            .put_meta(KLogMetaPutRequest {
                key: meta_key.clone(),
                value: "before-restart".to_string(),
                updated_at: Some(3003),
                updated_by: Some(leader_before),
                expected_revision: Some(0),
            })
            .await
            .map_err(|e| format!("put_meta before full restart failed: {}", e))?;
        if meta_before.revision != 1 {
            return Err(format!(
                "unexpected meta revision before restart: expected=1, got={}",
                meta_before.revision
            ));
        }

        // Stop all voters first (full cluster downtime), then restart in non-deterministic order.
        for n in &mut nodes {
            n.stop().await;
        }
        sleep(Duration::from_millis(300)).await;

        for id in [2_u64, 3_u64, 1_u64] {
            let idx = find_node_index(&nodes, id)?;
            restart_node(&mut nodes[idx]).await?;
        }

        let _ = wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(90))
            .await?;
        let leader_after =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(60))
                .await?;
        let leader_rpc_after = rpc_port_by_node_id(&nodes, leader_after)?;

        let queried_before = query_logs(
            &http,
            leader_rpc_after,
            Some(append_before_1.id),
            Some(append_before_2.id),
            Some(10),
            Some(false),
        )
        .await?;
        let queried_ids = queried_before
            .items
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>();
        if queried_before.items.len() != 2
            || queried_ids != vec![append_before_1.id, append_before_2.id]
        {
            return Err(format!(
                "unexpected pre-restart logs after full restart: len={}, ids={:?}",
                queried_before.items.len(),
                queried_ids
            ));
        }

        let meta_client_after = rpc_client(leader_rpc_after, 9202);
        let meta_queried = meta_client_after
            .query_meta(KLogMetaQueryRequest {
                key: Some(meta_key.clone()),
                prefix: None,
                limit: Some(1),
                strong_read: Some(true),
            })
            .await
            .map_err(|e| format!("query_meta after full restart failed: {}", e))?;
        if meta_queried.items.len() != 1
            || meta_queried.items[0].value != "before-restart"
            || meta_queried.items[0].revision != 1
        {
            return Err(format!(
                "unexpected meta after full restart: items={:?}",
                meta_queried
                    .items
                    .iter()
                    .map(|i| format!("key={}, value={}, revision={}", i.key, i.value, i.revision))
                    .collect::<Vec<_>>()
            ));
        }

        let append_after = append_log(
            &http,
            leader_rpc_after,
            "after-full-restart",
            Some(3004),
            Some(leader_after),
        )
        .await?;
        let queried_after = query_logs(
            &http,
            leader_rpc_after,
            Some(append_after.id),
            Some(append_after.id),
            Some(1),
            Some(false),
        )
        .await?;
        if queried_after.items.len() != 1
            || queried_after.items[0].id != append_after.id
            || queried_after.items[0].message != "after-full-restart"
        {
            return Err(format!(
                "unexpected post-restart append/query result: items={:?}",
                queried_after
                    .items
                    .iter()
                    .map(|i| format!("id={}, message={}", i.id, i.message))
                    .collect::<Vec<_>>()
            ));
        }

        let meta_after = meta_client_after
            .put_meta(KLogMetaPutRequest {
                key: meta_key.clone(),
                value: "after-restart".to_string(),
                updated_at: Some(3005),
                updated_by: Some(leader_after),
                expected_revision: Some(1),
            })
            .await
            .map_err(|e| format!("cas put_meta after full restart failed: {}", e))?;
        if meta_after.revision != 2 {
            return Err(format!(
                "unexpected meta revision after full restart update: expected=2, got={}",
                meta_after.revision
            ));
        }

        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}
