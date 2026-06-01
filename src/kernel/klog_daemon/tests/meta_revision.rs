mod common;

use common::*;
use klog::error::KLogErrorCode;
use klog::network::{KLogMetaDeleteRequest, KLogMetaPutRequest, KLogMetaQueryRequest};
use klog::rpc::KLogClient;
use std::time::Duration;

fn client_for_rpc_port(rpc_port: u16, node_id: u64) -> KLogClient {
    KLogClient::from_daemon_addr(
        format!("127.0.0.1:{}", rpc_port).as_str(),
        format!("node-{}", node_id),
    )
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
                node_name: None,
                expected_revision: Some(0),
            })
            .await
            .map_err(|e| format!("create-if-absent put_meta failed: {}", e))?;
        if (created.revision, created.create_revision, created.mod_revision, created.version)
            != (1, 1, 1, 1)
        {
            return Err(format!(
                "unexpected create revision fields: revision={}, create_revision={}, mod_revision={}, version={}",
                created.revision, created.create_revision, created.mod_revision, created.version
            ));
        }

        let conflict_create = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v-create-conflict".to_string(),
                node_name: None,
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
                node_name: None,
                expected_revision: Some(1),
            })
            .await
            .map_err(|e| format!("cas put_meta(expected=1) failed: {}", e))?;
        if (updated.revision, updated.create_revision, updated.mod_revision, updated.version)
            != (2, 1, 2, 2)
        {
            return Err(format!(
                "unexpected cas update revision fields: revision={}, create_revision={}, mod_revision={}, version={}",
                updated.revision, updated.create_revision, updated.mod_revision, updated.version
            ));
        }

        let conflict_stale = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v-stale".to_string(),
                node_name: None,
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
                node_name: None,
                expected_revision: None,
            })
            .await
            .map_err(|e| format!("non-cas put_meta failed: {}", e))?;
        if (non_cas.revision, non_cas.create_revision, non_cas.mod_revision, non_cas.version)
            != (3, 1, 3, 3)
        {
            return Err(format!(
                "unexpected non-cas revision fields: revision={}, create_revision={}, mod_revision={}, version={}",
                non_cas.revision, non_cas.create_revision, non_cas.mod_revision, non_cas.version
            ));
        }

        let deleted = follower_client
            .delete_meta(KLogMetaDeleteRequest { key: key.clone() })
            .await
            .map_err(|e| format!("delete_meta failed: {}", e))?;
        let deleted_version = deleted
            .meta_version
            .as_ref()
            .map(|v| (v.revision, v.create_revision, v.mod_revision, v.version, v.deleted));
        if !deleted.existed
            || deleted.prev_meta.as_ref().map(|item| {
                (
                    item.revision,
                    item.create_revision,
                    item.mod_revision,
                    item.version,
                )
            }) != Some((3, 1, 3, 3))
            || deleted_version != Some((4, 1, 4, 0, true))
        {
            return Err(format!(
                "unexpected delete response: existed={}, prev_revision={:?}, delete_meta_version={:?}",
                deleted.existed,
                deleted.prev_meta.as_ref().map(|item| {
                    (
                        item.revision,
                        item.create_revision,
                        item.mod_revision,
                        item.version,
                    )
                }),
                deleted_version
            ));
        }

        let stale_after_delete = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v-stale-after-delete".to_string(),
                node_name: None,
                expected_revision: Some(3),
            })
            .await
            .expect_err("expected stale revision conflict after delete");
        if stale_after_delete.error_code != KLogErrorCode::VersionConflict
            || !stale_after_delete
                .message
                .contains("current_revision=Some(4)")
        {
            return Err(format!(
                "unexpected stale-after-delete conflict: code={:?}, message={}",
                stale_after_delete.error_code, stale_after_delete.message
            ));
        }

        let recreated = follower_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "v4-recreated".to_string(),
                node_name: None,
                expected_revision: Some(0),
            })
            .await
            .map_err(|e| format!("recreate put_meta failed: {}", e))?;
        if (
            recreated.revision,
            recreated.create_revision,
            recreated.mod_revision,
            recreated.version,
        ) != (5, 5, 5, 1)
        {
            return Err(format!(
                "unexpected recreated revision fields: revision={}, create_revision={}, mod_revision={}, version={}",
                recreated.revision,
                recreated.create_revision,
                recreated.mod_revision,
                recreated.version
            ));
        }

        let queried = leader_client
            .query_meta(KLogMetaQueryRequest {
                key: Some(key.clone()),
                prefix: None,
                limit: Some(1),
                cursor: None,
                revision: None,
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
        if queried.items[0].value != "v4-recreated"
            || (
                queried.items[0].revision,
                queried.items[0].create_revision,
                queried.items[0].mod_revision,
                queried.items[0].version,
            ) != (5, 5, 5, 1)
        {
            return Err(format!(
                "unexpected meta value/revision fields: value={}, revision={}, create_revision={}, mod_revision={}, version={}",
                queried.items[0].value,
                queried.items[0].revision,
                queried.items[0].create_revision,
                queried.items[0].mod_revision,
                queried.items[0].version
            ));
        }

        let historical_v2 = leader_client
            .query_meta(KLogMetaQueryRequest {
                key: Some(key.clone()),
                prefix: None,
                limit: Some(1),
                cursor: None,
                revision: Some(2),
                strong_read: Some(true),
            })
            .await
            .map_err(|e| format!("query_meta revision=2 failed: {}", e))?;
        if historical_v2.items.len() != 1
            || historical_v2.items[0].value != "v2"
            || (
                historical_v2.items[0].revision,
                historical_v2.items[0].create_revision,
                historical_v2.items[0].mod_revision,
                historical_v2.items[0].version,
            ) != (2, 1, 2, 2)
        {
            return Err(format!(
                "unexpected historical rev2 query: len={}, first={:?}",
                historical_v2.items.len(),
                historical_v2.items.first()
            ));
        }

        let historical_deleted = leader_client
            .query_meta(KLogMetaQueryRequest {
                key: Some(key.clone()),
                prefix: None,
                limit: Some(1),
                cursor: None,
                revision: Some(4),
                strong_read: Some(true),
            })
            .await
            .map_err(|e| format!("query_meta revision=4 failed: {}", e))?;
        if !historical_deleted.items.is_empty() {
            return Err(format!(
                "deleted historical revision should be invisible: items={:?}",
                historical_deleted.items
            ));
        }

        let historical_recreated = leader_client
            .query_meta(KLogMetaQueryRequest {
                key: Some(key.clone()),
                prefix: None,
                limit: Some(1),
                cursor: None,
                revision: Some(5),
                strong_read: Some(true),
            })
            .await
            .map_err(|e| format!("query_meta revision=5 failed: {}", e))?;
        if historical_recreated.items.len() != 1
            || historical_recreated.items[0].value != "v4-recreated"
            || (
                historical_recreated.items[0].revision,
                historical_recreated.items[0].create_revision,
                historical_recreated.items[0].mod_revision,
                historical_recreated.items[0].version,
            ) != (5, 5, 5, 1)
        {
            return Err(format!(
                "unexpected historical rev5 query: len={}, first={:?}",
                historical_recreated.items.len(),
                historical_recreated.items.first()
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
                node_name: None,
                expected_revision: Some(0),
            })
            .await
            .map_err(|e| format!("put_meta before failover failed: {}", e))?;
        if (first.revision, first.create_revision, first.mod_revision, first.version) != (1, 1, 1, 1)
        {
            return Err(format!(
                "unexpected revision before failover: revision={}, create_revision={}, mod_revision={}, version={}",
                first.revision, first.create_revision, first.mod_revision, first.version
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
                cursor: None,
                revision: None,
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
        if queried.items[0].value != "before-failover"
            || (
                queried.items[0].revision,
                queried.items[0].create_revision,
                queried.items[0].mod_revision,
                queried.items[0].version,
            ) != (1, 1, 1, 1)
        {
            return Err(format!(
                "unexpected meta after failover: value={}, revision={}, create_revision={}, mod_revision={}, version={}",
                queried.items[0].value,
                queried.items[0].revision,
                queried.items[0].create_revision,
                queried.items[0].mod_revision,
                queried.items[0].version
            ));
        }

        let second = after_failover_client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value: "after-failover".to_string(),
                node_name: None,
                expected_revision: Some(1),
            })
            .await
            .map_err(|e| format!("put_meta after failover failed: {}", e))?;
        if (second.revision, second.create_revision, second.mod_revision, second.version)
            != (2, 1, 2, 2)
        {
            return Err(format!(
                "unexpected revision after failover update: revision={}, create_revision={}, mod_revision={}, version={}",
                second.revision, second.create_revision, second.mod_revision, second.version
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
