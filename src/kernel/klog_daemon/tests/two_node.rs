mod common;

use common::*;
use klog::KClusterTransportMode;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::time::{Instant, sleep};

async fn wait_log_visible_on_port(
    client: &reqwest::Client,
    port: u16,
    log_id: u64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let queried = query_logs(
            client,
            port,
            Some(log_id),
            Some(log_id),
            Some(10),
            Some(false),
        )
        .await;
        if let Ok(resp) = queried
            && resp.items.iter().any(|item| item.id == log_id)
        {
            return Ok(());
        }

        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting log_id={} visible on rpc_port={}",
                log_id, port
            ));
        }
        sleep(Duration::from_millis(200)).await;
    }
}

async fn send_add_learner_with_node_name(
    port: u16,
    node_id: u64,
    node_name: &str,
    addr: &str,
    node_port: u16,
    blocking: bool,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1200))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let admin_port = common::derive_admin_port(port);
    let url = format!(
        "http://127.0.0.1:{}/klog/admin/add-learner?node_id={}&node_name={}&addr={}&port={}&blocking={}",
        admin_port,
        node_id,
        node_name,
        addr,
        node_port,
        if blocking { "true" } else { "false" }
    );
    let resp = client
        .post(&url)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_else(|_| String::new());
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("request {} returned {}: {}", url, status, body))
    }
}

async fn add_learner_with_retry_node_name(
    voter_ports: &[u16],
    node_id: u64,
    node_name: &str,
    node_port: u16,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout adding learner node_id={}, node_name={}, node_port={}, voter_ports={:?}, last_err={}",
                node_id, node_name, node_port, voter_ports, last_err
            ));
        }

        let mut states = Vec::with_capacity(voter_ports.len());
        let mut all_ok = true;
        for port in voter_ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => states.push(state),
                Err(e) => {
                    all_ok = false;
                    last_err = format!("fetch cluster state failed on port {}: {}", port, e);
                    break;
                }
            }
        }

        if all_ok && states.len() == voter_ports.len() {
            let all_have_learner = states
                .iter()
                .all(|s| s.learners.iter().any(|id| *id == node_id));
            if all_have_learner {
                return Ok(());
            }

            let mut leader_port = None;
            if let Some(leader_id) = states.iter().find_map(|s| s.current_leader)
                && let Some(idx) = states.iter().position(|s| s.node_id == leader_id)
            {
                leader_port = voter_ports.get(idx).copied();
            }

            let mut candidate_ports = Vec::new();
            if let Some(p) = leader_port {
                candidate_ports.push(p);
            }
            for p in voter_ports {
                if !candidate_ports.contains(p) {
                    candidate_ports.push(*p);
                }
            }

            let mut errs = Vec::new();
            for p in candidate_ports {
                match send_add_learner_with_node_name(
                    p,
                    node_id,
                    node_name,
                    "127.0.0.1",
                    node_port,
                    true,
                )
                .await
                {
                    Ok(_) => {
                        errs.clear();
                        break;
                    }
                    Err(e) => errs.push(format!("port={}, err={}", p, e)),
                }
            }
            if errs.is_empty() {
                last_err = format!(
                    "add-learner accepted for node_id={}, node_name={}, waiting membership propagation",
                    node_id, node_name
                );
            } else {
                last_err = format!(
                    "add-learner failed for node_id={}, node_name={}: {}",
                    node_id,
                    node_name,
                    errs.join(" | ")
                );
            }
        }

        sleep(Duration::from_millis(250)).await;
    }
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

fn build_two_node_defs(raft_ports: &[u16; 2]) -> Result<[(u16, u16, u16, u16); 2], String> {
    let mut node_defs = [(0_u16, 0_u16, 0_u16, 0_u16); 2];
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

fn make_two_node_transport_options(
    mode: KClusterTransportMode,
    gateway_addr: &str,
    route_prefix: &str,
    node_names: &[&str; 2],
) -> [TestNodeSpawnOptions; 2] {
    std::array::from_fn(|idx| TestNodeSpawnOptions {
        advertise_node_name: Some(node_names[idx].to_string()),
        cluster_network_mode: mode,
        cluster_gateway_addr: gateway_addr.to_string(),
        cluster_gateway_route_prefix: route_prefix.to_string(),
        ..TestNodeSpawnOptions::default()
    })
}

fn make_two_node_hybrid_options(
    gateway_addr: &str,
    route_prefix: &str,
    node_names: &[&str; 2],
    nodes: &[(u16, u16, u16, u16); 2],
) -> Result<[TestNodeSpawnOptions; 2], String> {
    let mut exclude = HashSet::new();
    for (raft_port, inter_port, admin_port, rpc_port) in nodes {
        exclude.insert(*raft_port);
        exclude.insert(*inter_port);
        exclude.insert(*admin_port);
        exclude.insert(*rpc_port);
    }

    let mut options = make_two_node_transport_options(
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

#[tokio::test]
async fn test_two_node_voter_and_learner_cluster_runs_but_is_not_ha() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip two-node voter+learner test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(2)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let cluster_name = format!("klog_two_node_voter_learner_{}_{}", port1, port2);
    let mut nodes = Vec::new();

    nodes.push(spawn_node(1, port1, &cluster_name, true, &[], "voter").await?);
    wait_single_node_leader(port1, 1, Duration::from_secs(20)).await?;
    nodes.push(spawn_node(2, port2, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1], 2, port2, Duration::from_secs(45)).await?;
        let _ =
            wait_cluster_membership(&[port1, port2], &[1], &[2], Duration::from_secs(50)).await?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let leader_append = append_log(
            &client,
            nodes[0].rpc_port,
            "two-node-1v1l",
            Some(700),
            Some(1),
        )
        .await?;
        wait_log_visible_on_port(
            &client,
            nodes[1].rpc_port,
            leader_append.id,
            Duration::from_secs(20),
        )
        .await?;

        let learner_logs = query_logs(
            &client,
            nodes[1].rpc_port,
            Some(leader_append.id),
            Some(leader_append.id),
            Some(10),
            Some(false),
        )
        .await?;
        if learner_logs.items.len() != 1 || learner_logs.items[0].message != "two-node-1v1l" {
            return Err(format!(
                "unexpected learner query result after replication: {:?}",
                learner_logs
                    .items
                    .iter()
                    .map(|item| format!("id={}, msg={}", item.id, item.message))
                    .collect::<Vec<_>>()
            ));
        }

        let voter_idx = nodes
            .iter()
            .position(|n| n.node_id == 1)
            .ok_or_else(|| "voter node(1) process not found".to_string())?;
        nodes[voter_idx].stop().await;

        if wait_new_leader_on_ports(&[port2], 1, Duration::from_secs(10))
            .await
            .is_ok()
        {
            return Err("learner-only topology unexpectedly elected a new leader".to_string());
        }

        if append_log(
            &client,
            nodes[1].rpc_port,
            "two-node-1v1l-after-voter-down",
            Some(701),
            Some(2),
        )
        .await
        .is_ok()
        {
            return Err(
                "learner unexpectedly accepted append after sole voter stopped".to_string(),
            );
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
async fn test_two_node_two_voter_cluster_loses_availability_after_one_failure() -> Result<(), String>
{
    if !can_bind_localhost() {
        eprintln!("skip two-node two-voter test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(2)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let cluster_name = format!("klog_two_node_two_voter_{}_{}", port1, port2);
    let join_seed = vec![format!("127.0.0.1:{}", port1)];
    let mut nodes = Vec::new();

    nodes.push(spawn_node(1, port1, &cluster_name, true, &[], "voter").await?);
    wait_single_node_leader(port1, 1, Duration::from_secs(20)).await?;
    nodes.push(spawn_node(2, port2, &cluster_name, false, &join_seed, "voter").await?);

    let result = async {
        let _ = wait_cluster_voters(&[port1, port2], &[1, 2], Duration::from_secs(45)).await?;
        let leader =
            wait_consistent_leader_on_ports(&[port1, port2], Duration::from_secs(30)).await?;
        let leader_rpc_port = rpc_port_by_node_id(&nodes, leader)?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let before_failure = append_log(
            &client,
            leader_rpc_port,
            "two-node-2v-before-failure",
            Some(800),
            Some(leader),
        )
        .await?;
        let survivor_id = if leader == 1 { 2 } else { 1 };
        let survivor_rpc_port = rpc_port_by_node_id(&nodes, survivor_id)?;
        wait_log_visible_on_port(
            &client,
            survivor_rpc_port,
            before_failure.id,
            Duration::from_secs(20),
        )
        .await?;

        let leader_idx = nodes
            .iter()
            .position(|n| n.node_id == leader)
            .ok_or_else(|| format!("cannot find leader node process for id={}", leader))?;
        nodes[leader_idx].stop().await;

        if wait_new_leader_on_ports(
            &[nodes
                .iter()
                .find(|n| n.node_id == survivor_id)
                .ok_or_else(|| format!("survivor node process not found for id={}", survivor_id))?
                .port],
            leader,
            Duration::from_secs(12),
        )
        .await
        .is_ok()
        {
            return Err(format!(
                "two-voter cluster unexpectedly elected a replacement leader after node {} stopped",
                leader
            ));
        }

        if append_log(
            &client,
            survivor_rpc_port,
            "two-node-2v-after-failure",
            Some(801),
            Some(survivor_id),
        )
        .await
        .is_ok()
        {
            return Err(format!(
                "surviving voter {} unexpectedly accepted append without quorum",
                survivor_id
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
async fn test_two_node_gateway_proxy_voter_and_learner_runs_but_is_not_ha() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!(
            "skip two-node gateway_proxy voter+learner test: localhost bind is not available"
        );
        return Ok(());
    }

    let raft_ports = choose_unique_ports(2)?;
    let raft_ports = [raft_ports[0], raft_ports[1]];
    let node_defs = build_two_node_defs(&raft_ports)?;
    let cluster_name = format!(
        "klog_two_node_gateway_proxy_{}_{}",
        node_defs[0].0, node_defs[1].0
    );
    let node_names = ["ood1", "ood2"];
    let route_prefix = "/.cluster/klog-it-two-node-proxy";
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
        ]),
    )
    .await?;
    let options = make_two_node_transport_options(
        KClusterTransportMode::GatewayProxy,
        gateway.addr.as_str(),
        route_prefix,
        &node_names,
    );
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
            &[],
            "learner",
            &options[1],
        )
        .await?,
    );

    let result = async {
        add_learner_with_retry_node_name(
            &[node_defs[0].0],
            2,
            node_names[1],
            node_defs[1].0,
            Duration::from_secs(45),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[node_defs[0].0, node_defs[1].0],
            &[1],
            &[2],
            Duration::from_secs(50),
        )
        .await?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let appended = append_log(
            &client,
            nodes[1].rpc_port,
            "two-node-gateway-proxy-1v1l",
            Some(900),
            Some(2),
        )
        .await?;
        wait_log_visible_on_port(
            &client,
            nodes[0].rpc_port,
            appended.id,
            Duration::from_secs(20),
        )
        .await?;

        let voter_idx = nodes
            .iter()
            .position(|n| n.node_id == 1)
            .ok_or_else(|| "voter node(1) process not found".to_string())?;
        nodes[voter_idx].stop().await;

        if wait_new_leader_on_ports(&[node_defs[1].0], 1, Duration::from_secs(10))
            .await
            .is_ok()
        {
            return Err(
                "gateway_proxy learner-only topology unexpectedly elected a new leader".to_string(),
            );
        }

        if append_log(
            &client,
            nodes[1].rpc_port,
            "two-node-gateway-proxy-1v1l-after-voter-down",
            Some(901),
            Some(2),
        )
        .await
        .is_ok()
        {
            return Err(
                "gateway_proxy learner unexpectedly accepted append after sole voter stopped"
                    .to_string(),
            );
        }

        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    drop(gateway);
    result
}

#[tokio::test]
async fn test_two_node_hybrid_two_voter_falls_back_to_gateway_but_still_loses_quorum()
-> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip two-node hybrid two-voter test: localhost bind is not available");
        return Ok(());
    }

    let raft_ports = choose_unique_ports(2)?;
    let raft_ports = [raft_ports[0], raft_ports[1]];
    let node_defs = build_two_node_defs(&raft_ports)?;
    let cluster_name = format!("klog_two_node_hybrid_{}_{}", node_defs[0].0, node_defs[1].0);
    let node_names = ["ood1", "ood2"];
    let route_prefix = "/.cluster/klog-it-two-node-hybrid";
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
        ]),
    )
    .await?;
    let options =
        make_two_node_hybrid_options(gateway.addr.as_str(), route_prefix, &node_names, &node_defs)?;
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

    let result = async {
        let _ = wait_cluster_voters(&[node_defs[0].0, node_defs[1].0], &[1, 2], Duration::from_secs(45))
            .await?;
        let leader =
            wait_consistent_leader_on_ports(&[node_defs[0].0, node_defs[1].0], Duration::from_secs(30))
                .await?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let follower = nodes
            .iter()
            .find(|n| n.node_id != leader)
            .ok_or_else(|| format!("failed to choose follower; leader_id={}", leader))?;
        let appended = append_log(
            &client,
            follower.rpc_port,
            "two-node-hybrid-2v-before-failure",
            Some(950),
            Some(follower.node_id),
        )
        .await?;
        wait_log_visible_on_port(
            &client,
            rpc_port_by_node_id(&nodes, leader)?,
            appended.id,
            Duration::from_secs(20),
        )
        .await?;

        let leader_idx = nodes
            .iter()
            .position(|n| n.node_id == leader)
            .ok_or_else(|| format!("cannot find leader node process for id={}", leader))?;
        nodes[leader_idx].stop().await;

        let survivor_id = if leader == 1 { 2 } else { 1 };
        if wait_new_leader_on_ports(
            &[nodes
                .iter()
                .find(|n| n.node_id == survivor_id)
                .ok_or_else(|| format!("survivor node process not found for id={}", survivor_id))?
                .port],
            leader,
            Duration::from_secs(12),
        )
        .await
        .is_ok()
        {
            return Err(format!(
                "two-node hybrid cluster unexpectedly elected replacement leader after node {} stopped",
                leader
            ));
        }

        if append_log(
            &client,
            rpc_port_by_node_id(&nodes, survivor_id)?,
            "two-node-hybrid-2v-after-failure",
            Some(951),
            Some(survivor_id),
        )
        .await
        .is_ok()
        {
            return Err(format!(
                "surviving voter {} unexpectedly accepted append without quorum in hybrid mode",
                survivor_id
            ));
        }

        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    drop(gateway);
    result
}
