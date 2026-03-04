mod common;

use common::*;
use klog::error::KLogErrorCode;
use klog::network::{KLogMetaPutRequest, KLogMetaQueryRequest};
use klog::rpc::KLogClient;
use std::time::Duration;

fn client_for_rpc_port(rpc_port: u16, node_id: u64) -> KLogClient {
    KLogClient::from_daemon_addr(format!("127.0.0.1:{}", rpc_port).as_str(), node_id)
        .with_timeout(Duration::from_secs(3))
}

#[tokio::test]
async fn test_three_node_meta_revision_optional_cas_via_client() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip meta revision cas test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_meta_revision_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_rpc_port = rpc_port_by_node_id(&nodes, leader_id)?;
        let follower_id = [1_u64, 2_u64, 3_u64]
            .into_iter()
            .find(|id| *id != leader_id)
            .ok_or_else(|| format!("failed to choose follower id, leader_id={}", leader_id))?;
        let follower_rpc_port = rpc_port_by_node_id(&nodes, follower_id)?;

        let key = format!("cluster/meta/revision/{}", leader_id);
        let follower_client = client_for_rpc_port(follower_rpc_port, 9001);
        let leader_client = client_for_rpc_port(leader_rpc_port, 9002);

        let created = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v1".to_string(),
                updated_at: Some(1_001),
                updated_by: Some(follower_id),
                expected_revision: Some(0),
            })
            .await
            .map_err(|e| format!("create-if-absent put_meta failed: {}", e))?;
        if created.revision != 1 {
            return Err(format!(
                "unexpected create revision: expected=1, got={}",
                created.revision
            ));
        }

        let conflict_create = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v-create-conflict".to_string(),
                updated_at: Some(1_002),
                updated_by: Some(follower_id),
                expected_revision: Some(0),
            })
            .await
            .expect_err("expected create-if-absent conflict");
        if conflict_create.error_code != KLogErrorCode::VersionConflict {
            return Err(format!(
                "unexpected create conflict code: expected={:?}, got={:?}",
                KLogErrorCode::VersionConflict,
                conflict_create.error_code
            ));
        }

        let updated = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v2".to_string(),
                updated_at: Some(1_003),
                updated_by: Some(follower_id),
                expected_revision: Some(1),
            })
            .await
            .map_err(|e| format!("cas put_meta(expected=1) failed: {}", e))?;
        if updated.revision != 2 {
            return Err(format!(
                "unexpected cas update revision: expected=2, got={}",
                updated.revision
            ));
        }

        let conflict_stale = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v-stale".to_string(),
                updated_at: Some(1_004),
                updated_by: Some(follower_id),
                expected_revision: Some(1),
            })
            .await
            .expect_err("expected stale revision conflict");
        if conflict_stale.error_code != KLogErrorCode::VersionConflict {
            return Err(format!(
                "unexpected stale conflict code: expected={:?}, got={:?}",
                KLogErrorCode::VersionConflict,
                conflict_stale.error_code
            ));
        }

        let non_cas = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v3-non-cas".to_string(),
                updated_at: Some(1_005),
                updated_by: Some(follower_id),
                expected_revision: None,
            })
            .await
            .map_err(|e| format!("non-cas put_meta failed: {}", e))?;
        if non_cas.revision != 3 {
            return Err(format!(
                "unexpected non-cas revision: expected=3, got={}",
                non_cas.revision
            ));
        }

        let queried = leader_client
            .query_meta(KLogMetaQueryRequest {
                key: Some(key.clone()),
                prefix: None,
                limit: Some(1),
                strong_read: Some(true),
            })
            .await
            .map_err(|e| format!("query_meta failed: {}", e))?;
        if queried.items.len() != 1 {
            return Err(format!(
                "unexpected query_meta item len: expected=1, got={}",
                queried.items.len()
            ));
        }
        if queried.items[0].value != "v3-non-cas" || queried.items[0].revision != 3 {
            return Err(format!(
                "unexpected meta value/revision: value={}, revision={}",
                queried.items[0].value, queried.items[0].revision
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
async fn test_three_node_meta_revision_kept_after_leader_failover() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip meta revision failover test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_meta_revision_failover_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_rpc_port = rpc_port_by_node_id(&nodes, leader_id)?;
        let key = format!("cluster/meta/failover/{}", leader_id);

        let before_failover_client = client_for_rpc_port(leader_rpc_port, 9011);
        let first = before_failover_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "before-failover".to_string(),
                updated_at: Some(2_001),
                updated_by: Some(leader_id),
                expected_revision: Some(0),
            })
            .await
            .map_err(|e| format!("put_meta before failover failed: {}", e))?;
        if first.revision != 1 {
            return Err(format!(
                "unexpected revision before failover: expected=1, got={}",
                first.revision
            ));
        }

        let old_leader_idx = nodes
            .iter()
            .position(|n| n.node_id == leader_id)
            .ok_or_else(|| format!("cannot find leader node process for id={}", leader_id))?;
        nodes[old_leader_idx].stop().await;

        let remaining_ports = nodes
            .iter()
            .enumerate()
            .filter_map(|(idx, n)| {
                if idx == old_leader_idx {
                    None
                } else {
                    Some(n.port)
                }
            })
            .collect::<Vec<_>>();
        let new_leader_id =
            wait_new_leader_on_ports(&remaining_ports, leader_id, Duration::from_secs(45)).await?;
        let new_leader_rpc_port = rpc_port_by_node_id(&nodes, new_leader_id)?;

        let after_failover_client = client_for_rpc_port(new_leader_rpc_port, 9012);
        let queried = after_failover_client
            .query_meta(KLogMetaQueryRequest {
                key: Some(key.clone()),
                prefix: None,
                limit: Some(1),
                strong_read: Some(true),
            })
            .await
            .map_err(|e| format!("query_meta after failover failed: {}", e))?;
        if queried.items.len() != 1 {
            return Err(format!(
                "unexpected query_meta after failover len: expected=1, got={}",
                queried.items.len()
            ));
        }
        if queried.items[0].value != "before-failover" || queried.items[0].revision != 1 {
            return Err(format!(
                "unexpected meta after failover: value={}, revision={}",
                queried.items[0].value, queried.items[0].revision
            ));
        }

        let second = after_failover_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "after-failover".to_string(),
                updated_at: Some(2_002),
                updated_by: Some(new_leader_id),
                expected_revision: Some(1),
            })
            .await
            .map_err(|e| format!("put_meta after failover failed: {}", e))?;
        if second.revision != 2 {
            return Err(format!(
                "unexpected revision after failover update: expected=2, got={}",
                second.revision
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
