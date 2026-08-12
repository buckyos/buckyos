mod common;

use common::*;
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_dual_auto_bootstrap_split_then_manual_converges() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip dual-bootstrap conflict test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(2)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let cluster_name = format!("klog_dual_bootstrap_conflict_{}_{}", port1, port2);

    let (mut node1, mut node2) = tokio::try_join!(
        spawn_node(1, port1, &cluster_name, true, &[], "voter"),
        spawn_node(2, port2, &cluster_name, true, &[], "voter"),
    )?;

    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        // Conflict behavior: both bootstrap nodes can elect themselves and form two isolated
        // single-node clusters if started concurrently.
        wait_single_node_leader(port1, 1, Duration::from_secs(20)).await?;
        wait_single_node_leader(port2, 2, Duration::from_secs(20)).await?;
        let state1 = fetch_cluster_state(&client, port1).await?;
        let state2 = fetch_cluster_state(&client, port2).await?;
        if state1.current_leader != Some(1)
            || state1.voters.iter().copied().collect::<BTreeSet<_>>() != BTreeSet::from([1])
        {
            return Err(format!(
                "unexpected node1 bootstrap state: leader={:?}, voters={:?}, learners={:?}",
                state1.current_leader, state1.voters, state1.learners
            ));
        }
        if state2.current_leader != Some(2)
            || state2.voters.iter().copied().collect::<BTreeSet<_>>() != BTreeSet::from([2])
        {
            return Err(format!(
                "unexpected node2 bootstrap state: leader={:?}, voters={:?}, learners={:?}",
                state2.current_leader, state2.voters, state2.learners
            ));
        }

        // Convergence path: restart one node as joiner and let it join the other bootstrap node.
        node2.stop().await;
        sleep(Duration::from_millis(200)).await;

        let join_seed = vec![format!("127.0.0.1:{}", port1)];
        node2 = spawn_node(2, port2, &cluster_name, false, &join_seed, "voter").await?;

        let _ = wait_cluster_voters(&[port1, port2], &[1, 2], Duration::from_secs(70)).await?;
        let leader =
            wait_consistent_leader_on_ports(&[port1, port2], Duration::from_secs(40)).await?;
        if ![1_u64, 2_u64].contains(&leader) {
            return Err(format!(
                "unexpected leader after dual-bootstrap convergence: {}",
                leader
            ));
        }

        Ok(())
    }
    .await;

    node1.stop().await;
    node2.stop().await;
    result
}
