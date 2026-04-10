mod common;

use common::*;
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_wrong_seed_cluster_is_rejected_then_correct_seed_joins() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip cluster-identity wrong-seed test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port_a = ports[0];
    let port_b = ports[1];
    let port_joiner = ports[2];
    let cluster_a = format!("klog_cluster_identity_a_{}", port_a);
    let cluster_b = format!("klog_cluster_identity_b_{}", port_b);

    let mut node_a = spawn_node(1, port_a, &cluster_a, true, &[], "voter").await?;
    let mut node_b = spawn_node(10, port_b, &cluster_b, true, &[], "voter").await?;
    wait_single_node_leader(port_a, 1, Duration::from_secs(20)).await?;
    wait_single_node_leader(port_b, 10, Duration::from_secs(20)).await?;

    // Joiner belongs to cluster B but points to cluster A seed on purpose.
    let wrong_seed = vec![format!("127.0.0.1:{}", port_a)];
    let mut node_joiner =
        spawn_node(11, port_joiner, &cluster_b, false, &wrong_seed, "voter").await?;

    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let _ = wait_cluster_voters(&[port_a], &[1], Duration::from_secs(25)).await?;
        let _ = wait_cluster_voters(&[port_b], &[10], Duration::from_secs(25)).await?;

        // Give wrong-seed auto-join some time to retry and keep rejecting mismatched cluster id.
        sleep(Duration::from_secs(3)).await;
        let joiner_state = fetch_cluster_state(&client, port_joiner).await?;
        if joiner_state.voters.contains(&11) || joiner_state.learners.contains(&11) {
            return Err(format!(
                "joiner unexpectedly joined with wrong seed: leader={:?}, voters={:?}, learners={:?}",
                joiner_state.current_leader, joiner_state.voters, joiner_state.learners
            ));
        }

        let state_a = fetch_cluster_state(&client, port_a).await?;
        if state_a.voters.iter().copied().collect::<BTreeSet<_>>() != BTreeSet::from([1])
            || state_a.voters.contains(&11)
            || state_a.learners.contains(&11)
        {
            return Err(format!(
                "cluster A was unexpectedly changed by wrong-seed joiner: voters={:?}, learners={:?}",
                state_a.voters, state_a.learners
            ));
        }

        // Remediation: restart joiner with correct seed and verify it can join cluster B.
        node_joiner.stop().await;
        sleep(Duration::from_millis(200)).await;
        let correct_seed = vec![format!("127.0.0.1:{}", port_b)];
        node_joiner =
            spawn_node(11, port_joiner, &cluster_b, false, &correct_seed, "voter").await?;

        let _ = wait_cluster_voters(&[port_b, port_joiner], &[10, 11], Duration::from_secs(70))
            .await?;
        let leader = wait_consistent_leader_on_ports(&[port_b, port_joiner], Duration::from_secs(40))
            .await?;
        if ![10_u64, 11_u64].contains(&leader) {
            return Err(format!(
                "unexpected leader after correct-seed join: {}",
                leader
            ));
        }

        Ok(())
    }
    .await;

    node_a.stop().await;
    node_b.stop().await;
    node_joiner.stop().await;
    result
}
