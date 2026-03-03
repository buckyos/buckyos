mod common;

use common::*;
use std::time::Duration;

#[tokio::test]
async fn test_three_node_append_via_follower_auto_forwards_to_leader() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip follower-forward append test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_append_forward_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_port = match leader_id {
            1 => port1,
            2 => port2,
            3 => port3,
            _ => return Err(format!("unexpected leader id: {}", leader_id)),
        };
        let follower_port = [port1, port2, port3]
            .into_iter()
            .find(|p| *p != leader_port)
            .ok_or_else(|| "failed to choose follower port".to_string())?;
        let leader_rpc_port = rpc_port_by_node_id(&nodes, leader_id)?;
        let follower_id = match follower_port {
            p if p == port1 => 1,
            p if p == port2 => 2,
            p if p == port3 => 3,
            _ => return Err(format!("unexpected follower raft port: {}", follower_port)),
        };
        let follower_rpc_port = rpc_port_by_node_id(&nodes, follower_id)?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let appended = append_log(
            &client,
            follower_rpc_port,
            "forwarded-from-follower",
            Some(300),
            Some(2),
        )
        .await?;

        let queried = query_logs(
            &client,
            leader_rpc_port,
            Some(appended.id),
            Some(appended.id),
            Some(1),
            Some(false),
        )
        .await?;
        if queried.items.len() != 1 {
            return Err(format!(
                "unexpected query result len: expected=1, got={}",
                queried.items.len()
            ));
        }

        let item = &queried.items[0];
        if item.id != appended.id || item.message != "forwarded-from-follower" {
            return Err(format!(
                "unexpected queried item: id={}, message={}",
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
async fn test_three_node_retry_same_request_id_via_followers_dedups() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip follower-forward dedup retry test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_forward_dedup_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_port = match leader_id {
            1 => port1,
            2 => port2,
            3 => port3,
            _ => return Err(format!("unexpected leader id: {}", leader_id)),
        };
        let follower_ports = [port1, port2, port3]
            .into_iter()
            .filter(|p| *p != leader_port)
            .collect::<Vec<_>>();
        if follower_ports.len() != 2 {
            return Err(format!(
                "unexpected follower ports size: expected=2, got={}, ports={:?}",
                follower_ports.len(),
                follower_ports
            ));
        }

        let port_to_node_id = |p: u16| -> Result<u64, String> {
            if p == port1 {
                Ok(1)
            } else if p == port2 {
                Ok(2)
            } else if p == port3 {
                Ok(3)
            } else {
                Err(format!("unexpected raft port: {}", p))
            }
        };

        let follower1_id = port_to_node_id(follower_ports[0])?;
        let follower2_id = port_to_node_id(follower_ports[1])?;
        let follower1_rpc_port = rpc_port_by_node_id(&nodes, follower1_id)?;
        let follower2_rpc_port = rpc_port_by_node_id(&nodes, follower2_id)?;
        let leader_rpc_port = rpc_port_by_node_id(&nodes, leader_id)?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let request_id = format!("forward-dedup-{}", leader_port);
        let first = append_log_with_request_id(
            &client,
            follower1_rpc_port,
            "forward-dedup-original",
            Some(400),
            Some(follower1_id),
            Some(request_id.as_str()),
        )
        .await?;
        let retry = append_log_with_request_id(
            &client,
            follower2_rpc_port,
            "forward-dedup-retry",
            Some(401),
            Some(follower2_id),
            Some(request_id.as_str()),
        )
        .await?;

        if retry.id != first.id {
            return Err(format!(
                "request_id dedup via follower forwarding failed: first_id={}, retry_id={}",
                first.id, retry.id
            ));
        }

        let queried = query_logs(
            &client,
            leader_rpc_port,
            Some(first.id),
            Some(first.id),
            Some(10),
            Some(false),
        )
        .await?;
        if queried.items.len() != 1 {
            return Err(format!(
                "unexpected dedup query len via follower forwarding: expected=1, got={}",
                queried.items.len()
            ));
        }

        let item = &queried.items[0];
        if item.id != first.id || item.message != "forward-dedup-original" {
            return Err(format!(
                "unexpected dedup item via follower forwarding: id={}, message={}",
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
