mod common;

use common::*;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_bootstrap_late_start_converges() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip bootstrap-late-start cluster test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_cluster_bootstrap_late_{}_{}_{}", port1, port2, port3);
    let join_seed = vec![format!("127.0.0.1:{}", port1)];

    let (node2, node3) = tokio::try_join!(
        spawn_node(2, port2, &cluster_name, false, &join_seed, "voter"),
        spawn_node(3, port3, &cluster_name, false, &join_seed, "voter"),
    )?;

    let mut nodes = vec![node2, node3];
    // Simulate unpredictable startup order: non-bootstrap nodes start first.
    sleep(Duration::from_millis(1500)).await;
    nodes.push(spawn_node(1, port1, &cluster_name, true, &[], "voter").await?);

    let result = async {
        let _ = wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(70))
            .await?;
        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!("unexpected leader id: {}", leader));
        }
        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}
