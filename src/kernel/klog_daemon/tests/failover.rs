mod common;

use common::*;
use std::time::Duration;

#[tokio::test]
async fn test_three_node_cluster_and_failover() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip three-node cluster test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_cluster_{}_{}_{}", port1, port2, port3);
    let join_seed = vec![format!("127.0.0.1:{}", port1)];

    let mut nodes = Vec::new();
    nodes.push(spawn_node(1, port1, &cluster_name, true, &[], "voter").await?);
    wait_single_node_leader(port1, 1, Duration::from_secs(20)).await?;

    // Join one by one to avoid concurrent membership-change races that may leave
    // the cluster in a hard-to-elect transitional state.
    nodes.push(spawn_node(2, port2, &cluster_name, false, &join_seed, "voter").await?);
    let _ = wait_cluster_voters(&[port1, port2], &[1, 2], Duration::from_secs(40)).await?;

    nodes.push(spawn_node(3, port3, &cluster_name, false, &join_seed, "voter").await?);

    let states =
        wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(40)).await?;
    let leader = states
        .iter()
        .find_map(|s| s.current_leader)
        .ok_or_else(|| "cluster has no leader after convergence".to_string())?;
    if ![1_u64, 2_u64, 3_u64].contains(&leader) {
        for n in &mut nodes {
            n.stop().await;
        }
        return Err(format!("unexpected leader id: {}", leader));
    }

    let leader_index = nodes
        .iter()
        .position(|n| n.node_id == leader)
        .ok_or_else(|| format!("cannot find leader node process for id={}", leader))?;
    nodes[leader_index].stop().await;

    let remaining_ports = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            if i == leader_index {
                None
            } else {
                Some(n.port)
            }
        })
        .collect::<Vec<_>>();
    let new_leader =
        wait_new_leader_on_ports(&remaining_ports, leader, Duration::from_secs(40)).await?;

    for n in &mut nodes {
        n.stop().await;
    }

    if new_leader == leader {
        return Err(format!(
            "new leader did not change: old={}, new={}",
            leader, new_leader
        ));
    }

    Ok(())
}

#[tokio::test]
async fn test_request_id_dedup_survives_leader_restart() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip leader-restart dedup test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_leader_restart_dedup_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_rpc_port = rpc_port_by_node_id(&nodes, leader_id)?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let request_id = format!("leader-restart-dedup-{}", leader_id);
        let first = append_log_with_request_id(
            &client,
            leader_rpc_port,
            "leader-restart-dedup-original",
            Some(500),
            Some(leader_id),
            Some(request_id.as_str()),
        )
        .await?;

        let leader_idx = nodes
            .iter()
            .position(|n| n.node_id == leader_id)
            .ok_or_else(|| format!("cannot find leader node process for id={}", leader_id))?;
        restart_node(&mut nodes[leader_idx]).await?;

        let _ = wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(60))
            .await?;
        let current_leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let current_leader_rpc_port = rpc_port_by_node_id(&nodes, current_leader)?;
        let restarted_node_rpc_port = nodes[leader_idx].rpc_port;

        let retry = append_log_with_request_id(
            &client,
            restarted_node_rpc_port,
            "leader-restart-dedup-retry",
            Some(501),
            Some(nodes[leader_idx].node_id),
            Some(request_id.as_str()),
        )
        .await?;
        if retry.id != first.id {
            return Err(format!(
                "request_id dedup failed after leader restart: first_id={}, retry_id={}",
                first.id, retry.id
            ));
        }

        let queried = query_logs(
            &client,
            current_leader_rpc_port,
            Some(first.id),
            Some(first.id),
            Some(10),
            Some(false),
        )
        .await?;
        if queried.items.len() != 1 {
            return Err(format!(
                "unexpected dedup query len after leader restart: expected=1, got={}",
                queried.items.len()
            ));
        }

        let item = &queried.items[0];
        if item.id != first.id || item.message != "leader-restart-dedup-original" {
            return Err(format!(
                "unexpected dedup item after leader restart: id={}, message={}",
                item.id, item.message
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

#[tokio::test]
async fn test_request_id_dedup_survives_leader_failover() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip leader-failover dedup test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_leader_failover_dedup_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_rpc_port = rpc_port_by_node_id(&nodes, leader_id)?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let request_id = format!("leader-failover-dedup-{}", leader_id);
        let first = append_log_with_request_id(
            &client,
            leader_rpc_port,
            "leader-failover-dedup-original",
            Some(600),
            Some(leader_id),
            Some(request_id.as_str()),
        )
        .await?;

        let old_leader_idx = nodes
            .iter()
            .position(|n| n.node_id == leader_id)
            .ok_or_else(|| format!("cannot find leader node process for id={}", leader_id))?;
        nodes[old_leader_idx].stop().await;

        let remaining_ports = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                if i == old_leader_idx {
                    None
                } else {
                    Some(n.port)
                }
            })
            .collect::<Vec<_>>();
        let new_leader =
            wait_new_leader_on_ports(&remaining_ports, leader_id, Duration::from_secs(45)).await?;

        let retry_node_id = nodes
            .iter()
            .enumerate()
            .find_map(|(i, n)| {
                if i != old_leader_idx && n.node_id != new_leader {
                    Some(n.node_id)
                } else {
                    None
                }
            })
            .unwrap_or(new_leader);
        let retry_rpc_port = rpc_port_by_node_id(&nodes, retry_node_id)?;
        let new_leader_rpc_port = rpc_port_by_node_id(&nodes, new_leader)?;

        let retry = append_log_with_request_id(
            &client,
            retry_rpc_port,
            "leader-failover-dedup-retry",
            Some(601),
            Some(retry_node_id),
            Some(request_id.as_str()),
        )
        .await?;
        if retry.id != first.id {
            return Err(format!(
                "request_id dedup failed after leader failover: first_id={}, retry_id={}",
                first.id, retry.id
            ));
        }

        let queried = query_logs(
            &client,
            new_leader_rpc_port,
            Some(first.id),
            Some(first.id),
            Some(10),
            Some(false),
        )
        .await?;
        if queried.items.len() != 1 {
            return Err(format!(
                "unexpected dedup query len after leader failover: expected=1, got={}",
                queried.items.len()
            ));
        }

        let item = &queried.items[0];
        if item.id != first.id || item.message != "leader-failover-dedup-original" {
            return Err(format!(
                "unexpected dedup item after leader failover: id={}, message={}",
                item.id, item.message
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
