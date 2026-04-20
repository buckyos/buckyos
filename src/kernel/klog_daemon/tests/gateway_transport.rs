mod common;

use common::*;
use klog::KClusterTransportMode;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

fn make_transport_options(
    mode: KClusterTransportMode,
    gateway_addr: &str,
    route_prefix: &str,
    node_names: &[&str; 3],
) -> [TestNodeSpawnOptions; 3] {
    std::array::from_fn(|idx| TestNodeSpawnOptions {
        advertise_node_name: Some(node_names[idx].to_string()),
        cluster_network_mode: mode,
        cluster_gateway_addr: gateway_addr.to_string(),
        cluster_gateway_route_prefix: route_prefix.to_string(),
        ..TestNodeSpawnOptions::default()
    })
}

fn pick_unused_port(exclude: &HashSet<u16>) -> Result<u16, String> {
    for _ in 0..200 {
        let candidate =
            choose_free_port().map_err(|e| format!("choose free port failed: {}", e))?;
        if !exclude.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "failed to pick unused port after retries; exclude_len={}",
        exclude.len()
    ))
}

fn make_hybrid_options(
    gateway_addr: &str,
    route_prefix: &str,
    node_names: &[&str; 3],
    nodes: &[(u16, u16, u16, u16); 3],
) -> Result<[TestNodeSpawnOptions; 3], String> {
    let mut exclude = HashSet::new();
    for (raft_port, inter_port, admin_port, rpc_port) in nodes {
        exclude.insert(*raft_port);
        exclude.insert(*inter_port);
        exclude.insert(*admin_port);
        exclude.insert(*rpc_port);
    }

    let mut options = make_transport_options(
        KClusterTransportMode::Hybrid,
        gateway_addr,
        route_prefix,
        node_names,
    );
    for option in &mut options {
        option.advertise_port = Some(pick_unused_port(&exclude)?);
        exclude.insert(option.advertise_port.unwrap());
        option.advertise_inter_port = Some(pick_unused_port(&exclude)?);
        exclude.insert(option.advertise_inter_port.unwrap());
        option.advertise_admin_port = Some(pick_unused_port(&exclude)?);
        exclude.insert(option.advertise_admin_port.unwrap());
    }

    Ok(options)
}

fn build_node_defs(raft_ports: &[u16]) -> Result<[(u16, u16, u16, u16); 3], String> {
    let mut node_defs = [(0_u16, 0_u16, 0_u16, 0_u16); 3];
    let mut exclude = HashSet::new();
    for raft_port in raft_ports {
        exclude.insert(*raft_port);
        exclude.insert(common::derive_admin_port(*raft_port));
    }

    for (idx, raft_port) in raft_ports.iter().enumerate() {
        let admin_port = common::derive_admin_port(*raft_port);
        let rpc_port = pick_unused_port(&exclude)?;
        exclude.insert(rpc_port);
        let inter_port = pick_unused_port(&exclude)?;
        exclude.insert(inter_port);
        node_defs[idx] = (*raft_port, inter_port, admin_port, rpc_port);
    }

    Ok(node_defs)
}

#[tokio::test]
async fn test_three_node_gateway_proxy_cluster_replication_and_forwarding() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip gateway transport test: localhost bind is not available");
        return Ok(());
    }

    let raft_ports = choose_unique_ports(3)?;
    let node_defs = build_node_defs(&raft_ports)?;
    let cluster_name = format!(
        "klog_gateway_proxy_{}_{}_{}",
        node_defs[0].0, node_defs[1].0, node_defs[2].0
    );
    let node_names = ["ood1", "ood2", "ood3"];
    let route_prefix = "/.cluster/klog-it-proxy";

    let gateway = spawn_gateway_proxy(
        route_prefix,
        HashMap::from([
            (
                "ood1".to_string(),
                TestGatewayNodeTarget {
                    raft_port: node_defs[0].0,
                    inter_port: node_defs[0].1,
                    admin_port: node_defs[0].2,
                },
            ),
            (
                "ood2".to_string(),
                TestGatewayNodeTarget {
                    raft_port: node_defs[1].0,
                    inter_port: node_defs[1].1,
                    admin_port: node_defs[1].2,
                },
            ),
            (
                "ood3".to_string(),
                TestGatewayNodeTarget {
                    raft_port: node_defs[2].0,
                    inter_port: node_defs[2].1,
                    admin_port: node_defs[2].2,
                },
            ),
        ]),
    )
    .await?;
    let options = make_transport_options(
        KClusterTransportMode::GatewayProxy,
        gateway.addr.as_str(),
        route_prefix,
        &node_names,
    );

    let join_seed = vec![format!("127.0.0.1:{}", node_defs[0].0)];
    let mut nodes = Vec::new();
    nodes.push(
        spawn_node_on_ports_with_options(
            1,
            node_defs[0].0,
            node_defs[0].1,
            node_defs[0].2,
            node_defs[0].3,
            &cluster_name,
            true,
            &[],
            "voter",
            &options[0],
        )
        .await?,
    );
    wait_single_node_leader(node_defs[0].0, 1, Duration::from_secs(20)).await?;
    nodes.push(
        spawn_node_on_ports_with_options(
            2,
            node_defs[1].0,
            node_defs[1].1,
            node_defs[1].2,
            node_defs[1].3,
            &cluster_name,
            false,
            &join_seed,
            "voter",
            &options[1],
        )
        .await?,
    );
    let _ = wait_cluster_voters(
        &[node_defs[0].0, node_defs[1].0],
        &[1, 2],
        Duration::from_secs(40),
    )
    .await?;
    nodes.push(
        spawn_node_on_ports_with_options(
            3,
            node_defs[2].0,
            node_defs[2].1,
            node_defs[2].2,
            node_defs[2].3,
            &cluster_name,
            false,
            &join_seed,
            "voter",
            &options[2],
        )
        .await?,
    );
    let _ = wait_cluster_voters(
        &[node_defs[0].0, node_defs[1].0, node_defs[2].0],
        &[1, 2, 3],
        Duration::from_secs(50),
    )
    .await?;

    let result = async {
        let leader_id = wait_consistent_leader_on_ports(
            &[nodes[0].port, nodes[1].port, nodes[2].port],
            Duration::from_secs(40),
        )
        .await?;
        let follower = nodes
            .iter()
            .find(|n| n.node_id != leader_id)
            .ok_or_else(|| format!("failed to choose follower; leader_id={}", leader_id))?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let appended = append_log(
            &client,
            follower.rpc_port,
            "gateway-proxy-forwarded-write",
            Some(5_001),
            Some(follower.node_id),
        )
        .await?;

        let expected = BTreeSet::from([appended.id]);
        for node in &nodes {
            let queried = query_logs_with_strong_read(
                &client,
                node.rpc_port,
                Some(appended.id),
                Some(appended.id),
                Some(4),
                Some(false),
                Some(true),
            )
            .await?;
            let got = queried.items.iter().map(|item| item.id).collect::<BTreeSet<_>>();
            if got != expected {
                return Err(format!(
                    "gateway_proxy query mismatch on node_id={}, rpc_port={}, expected={:?}, got={:?}",
                    node.node_id, node.rpc_port, expected, got
                ));
            }
        }

        Ok(())
    }
    .await;

    for node in &mut nodes {
        node.stop().await;
    }
    drop(gateway);

    result
}

#[tokio::test]
async fn test_three_node_hybrid_cluster_falls_back_to_gateway_when_direct_unreachable()
-> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip hybrid gateway transport test: localhost bind is not available");
        return Ok(());
    }

    let raft_ports = choose_unique_ports(3)?;
    let node_defs = build_node_defs(&raft_ports)?;

    let cluster_name = format!(
        "klog_gateway_hybrid_{}_{}_{}",
        node_defs[0].0, node_defs[1].0, node_defs[2].0
    );
    let node_names = ["ood1", "ood2", "ood3"];
    let route_prefix = "/.cluster/klog-it-hybrid";

    let gateway = spawn_gateway_proxy(
        route_prefix,
        HashMap::from([
            (
                "ood1".to_string(),
                TestGatewayNodeTarget {
                    raft_port: node_defs[0].0,
                    inter_port: node_defs[0].1,
                    admin_port: node_defs[0].2,
                },
            ),
            (
                "ood2".to_string(),
                TestGatewayNodeTarget {
                    raft_port: node_defs[1].0,
                    inter_port: node_defs[1].1,
                    admin_port: node_defs[1].2,
                },
            ),
            (
                "ood3".to_string(),
                TestGatewayNodeTarget {
                    raft_port: node_defs[2].0,
                    inter_port: node_defs[2].1,
                    admin_port: node_defs[2].2,
                },
            ),
        ]),
    )
    .await?;
    let options =
        make_hybrid_options(gateway.addr.as_str(), route_prefix, &node_names, &node_defs)?;

    let mut nodes = Vec::new();
    let join_seed = vec![format!("127.0.0.1:{}", node_defs[0].0)];
    nodes.push(
        spawn_node_on_ports_with_options(
            1,
            node_defs[0].0,
            node_defs[0].1,
            node_defs[0].2,
            node_defs[0].3,
            &cluster_name,
            true,
            &[],
            "voter",
            &options[0],
        )
        .await?,
    );
    wait_single_node_leader(node_defs[0].0, 1, Duration::from_secs(20)).await?;
    nodes.push(
        spawn_node_on_ports_with_options(
            2,
            node_defs[1].0,
            node_defs[1].1,
            node_defs[1].2,
            node_defs[1].3,
            &cluster_name,
            false,
            &join_seed,
            "voter",
            &options[1],
        )
        .await?,
    );
    let _ = wait_cluster_voters(
        &[node_defs[0].0, node_defs[1].0],
        &[1, 2],
        Duration::from_secs(40),
    )
    .await?;
    nodes.push(
        spawn_node_on_ports_with_options(
            3,
            node_defs[2].0,
            node_defs[2].1,
            node_defs[2].2,
            node_defs[2].3,
            &cluster_name,
            false,
            &join_seed,
            "voter",
            &options[2],
        )
        .await?,
    );
    let _ = wait_cluster_voters(
        &[node_defs[0].0, node_defs[1].0, node_defs[2].0],
        &[1, 2, 3],
        Duration::from_secs(50),
    )
    .await?;

    let result = async {
        let leader_id = wait_consistent_leader_on_ports(
            &[node_defs[0].0, node_defs[1].0, node_defs[2].0],
            Duration::from_secs(50),
        )
        .await?;
        let follower = nodes
            .iter()
            .find(|n| n.node_id != leader_id)
            .ok_or_else(|| format!("failed to choose follower; leader_id={}", leader_id))?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let appended = append_log(
            &client,
            follower.rpc_port,
            "hybrid-fallback-forwarded-write",
            Some(5_101),
            Some(follower.node_id),
        )
        .await?;

        let queried = query_logs_with_strong_read(
            &client,
            follower.rpc_port,
            Some(appended.id),
            Some(appended.id),
            Some(4),
            Some(false),
            Some(true),
        )
        .await?;
        if queried.items.len() != 1 || queried.items[0].id != appended.id {
            return Err(format!(
                "hybrid query mismatch on follower node_id={}, rpc_port={}, expected_id={}, queried={:?}",
                follower.node_id, follower.rpc_port, appended.id, queried.items
            ));
        }

        Ok(())
    }
    .await;

    for node in &mut nodes {
        node.stop().await;
    }
    drop(gateway);

    result
}
