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
