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
