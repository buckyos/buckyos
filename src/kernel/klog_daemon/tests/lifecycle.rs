mod common;

use common::*;
use klog::network::KLogAdminRequestType;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::{Instant, sleep, timeout};

#[cfg(unix)]
async fn send_sigterm(node: &mut TestNode) -> Result<(), String> {
    let pid = node
        .child
        .id()
        .ok_or_else(|| format!("missing child pid for node_id={}", node.node_id))?;
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .await
        .map_err(|e| format!("failed to run kill -TERM {}: {}", pid, e))?;
    if !status.success() {
        return Err(format!("kill -TERM {} returned status {}", pid, status));
    }
    Ok(())
}

#[cfg(not(unix))]
async fn send_sigterm(_node: &mut TestNode) -> Result<(), String> {
    Err("SIGTERM lifecycle tests require unix platform".to_string())
}

async fn wait_node_exit(node: &mut TestNode, timeout_dur: Duration) -> Result<(), String> {
    timeout(timeout_dur, node.child.wait())
        .await
        .map_err(|_| {
            format!(
                "timeout waiting node process exit: node_id={}, timeout={:?}",
                node.node_id, timeout_dur
            )
        })?
        .map_err(|e| format!("wait process failed for node {}: {}", node.node_id, e))?;
    Ok(())
}

#[tokio::test]
async fn test_sigterm_stops_daemon_with_autojoin_loop_running() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip lifecycle auto-join shutdown test: localhost bind is not available");
        return Ok(());
    }

    // Keep one healthy node for baseline, and start joiner with unreachable seed so auto-join
    // loop keeps running in background.
    let ports = choose_unique_ports(2)?;
    let seed_port = ports[0];
    let joiner_port = ports[1];
    let cluster_name = format!("klog_lifecycle_autojoin_{}_{}", seed_port, joiner_port);
    let mut seed = spawn_node(1, seed_port, &cluster_name, true, &[], "voter").await?;
    let wrong_seed = vec!["127.0.0.1:9".to_string()];
    let mut joiner = spawn_node(2, joiner_port, &cluster_name, false, &wrong_seed, "voter").await?;

    let result = async {
        wait_single_node_leader(seed_port, 1, Duration::from_secs(20)).await?;
        sleep(Duration::from_millis(1200)).await;

        send_sigterm(&mut joiner).await?;
        wait_node_exit(&mut joiner, Duration::from_secs(10)).await?;

        // Baseline node still healthy after joiner shutdown.
        let _ = wait_cluster_voters(&[seed_port], &[1], Duration::from_secs(20)).await?;
        Ok(())
    }
    .await;

    joiner.stop().await;
    seed.stop().await;
    result
}

#[tokio::test]
async fn test_sigterm_during_admin_request_does_not_stall_cluster() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip lifecycle in-flight admin shutdown test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_lifecycle_admin_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_idx = nodes
            .iter()
            .position(|n| n.node_id == leader_id)
            .ok_or_else(|| format!("cannot find leader process for id={}", leader_id))?;
        let leader_port = nodes[leader_idx].port;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;
        let admin_path = KLogAdminRequestType::ChangeMembership.klog_path();
        let admin_url = format!(
            "http://127.0.0.1:{}{}?voters=1,2,3&retain=true",
            leader_port, admin_path
        );

        let request_task = tokio::spawn(async move {
            let start = Instant::now();
            let resp = client.post(&admin_url).send().await;
            (start.elapsed(), resp)
        });

        sleep(Duration::from_millis(30)).await;
        send_sigterm(&mut nodes[leader_idx]).await?;
        wait_node_exit(&mut nodes[leader_idx], Duration::from_secs(10)).await?;

        let (elapsed, result) = request_task
            .await
            .map_err(|e| format!("admin request task join failed: {}", e))?;
        if elapsed > Duration::from_secs(10) {
            return Err(format!(
                "admin request blocked too long during shutdown: elapsed={:?}",
                elapsed
            ));
        }
        match result {
            Ok(_resp) => {}
            Err(_e) => {}
        }

        let remaining_ports = nodes
            .iter()
            .enumerate()
            .filter_map(|(idx, n)| (idx != leader_idx).then_some(n.port))
            .collect::<Vec<_>>();
        let _ =
            wait_new_leader_on_ports(&remaining_ports, leader_id, Duration::from_secs(45)).await?;

        let remaining_rpc_port = nodes
            .iter()
            .enumerate()
            .find_map(|(idx, n)| (idx != leader_idx).then_some(n.rpc_port))
            .ok_or_else(|| "remaining rpc port not found".to_string())?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build verify http client: {}", e))?;
        let appended = append_log(
            &http,
            remaining_rpc_port,
            "lifecycle-post-shutdown",
            Some(4001),
            None,
        )
        .await?;
        if appended.id == 0 {
            return Err("append id should not be zero after leader shutdown".to_string());
        }

        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}
