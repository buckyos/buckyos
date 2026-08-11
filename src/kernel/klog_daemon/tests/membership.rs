mod common;

use common::*;
use std::time::Duration;

#[tokio::test]
async fn test_three_node_concurrent_startup_converges() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip concurrent-startup cluster test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(5)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let port5 = ports[4];
    let cluster_name = format!(
        "klog_cluster_concurrent_{}_{}_{}_{}_{}",
        port1, port2, port3, port4, port5
    );
    let join_seed = vec![format!("127.0.0.1:{}", port1)];

    let (node1, node2, node3, node4, node5) = tokio::try_join!(
        spawn_node(1, port1, &cluster_name, true, &[], "voter"),
        spawn_node(2, port2, &cluster_name, false, &join_seed, "voter"),
        spawn_node(3, port3, &cluster_name, false, &join_seed, "voter"),
        spawn_node(4, port4, &cluster_name, false, &join_seed, "learner"),
        spawn_node(5, port5, &cluster_name, false, &join_seed, "learner"),
    )?;

    let mut nodes = vec![node1, node2, node3, node4, node5];
    let result = async {
        let states = wait_cluster_membership(
            &[port1, port2, port3, port4, port5],
            &[1, 2, 3],
            &[4, 5],
            Duration::from_secs(80),
        )
        .await?;
        for state in states
            .iter()
            .filter(|s| s.node_id == 4 || s.node_id == 5)
        {
            if state.voters.contains(&state.node_id) || !state.learners.contains(&state.node_id) {
                return Err(format!(
                    "learner node state mismatch: node_id={}, voters={:?}, learners={:?}, server_state={}",
                    state.node_id, state.voters, state.learners, state.server_state
                ));
            }
        }

        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!("unexpected leader id: {}", leader));
        }

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4, 5],
            Duration::from_secs(45),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}

#[tokio::test]
async fn test_remove_offline_learner_succeeds() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip remove-offline-learner test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(4)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let cluster_name = format!(
        "klog_remove_offline_learner_{}_{}_{}_{}",
        port1, port2, port3, port4
    );

    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;
    nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(50),
        )
        .await?;

        let learner_idx = nodes
            .iter()
            .position(|n| n.node_id == 4)
            .ok_or_else(|| "learner node(4) process not found".to_string())?;
        nodes[learner_idx].stop().await;

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(50),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;

        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!(
                "unexpected leader after offline learner removal: {}",
                leader
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
async fn test_remove_both_learners_when_one_offline() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip remove-two-learners test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(5)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let port5 = ports[4];
    let cluster_name = format!(
        "klog_remove_two_learners_{}_{}_{}_{}_{}",
        port1, port2, port3, port4, port5
    );

    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;
    nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);
    nodes.push(spawn_node(5, port5, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        add_learner_with_retry(&[port1, port2, port3], 5, port5, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4, 5],
            Duration::from_secs(60),
        )
        .await?;

        let learner4_idx = nodes
            .iter()
            .position(|n| n.node_id == 4)
            .ok_or_else(|| "learner node(4) process not found".to_string())?;
        nodes[learner4_idx].stop().await;

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4, 5],
            Duration::from_secs(55),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;

        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!(
                "unexpected leader after removing both learners: {}",
                leader
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
async fn test_offline_learner_rejoin_requires_add_learner_again() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip learner-rejoin test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(4)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let cluster_name = format!(
        "klog_learner_rejoin_requires_add_{}_{}_{}_{}",
        port1, port2, port3, port4
    );

    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;
    nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(50),
        )
        .await?;

        let learner_idx = nodes
            .iter()
            .position(|n| n.node_id == 4)
            .ok_or_else(|| "learner node(4) process not found".to_string())?;
        nodes[learner_idx].stop().await;

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(55),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;

        // Re-start the removed learner node without auto-join target.
        nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);
        ensure_learners_absent_for_duration(&[port1, port2, port3], &[4], Duration::from_secs(5))
            .await?;

        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}
