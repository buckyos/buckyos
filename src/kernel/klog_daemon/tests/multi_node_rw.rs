mod common;

use common::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::task::JoinSet;

#[derive(Debug, Serialize)]
struct RichAppendLogBody {
    message: String,
    timestamp: Option<u64>,
    node_name: Option<String>,
    level: Option<String>,
    source: Option<String>,
    attrs: Option<BTreeMap<String, String>>,
    request_id: Option<String>,
}

async fn append_log_rich(
    client: &reqwest::Client,
    port: u16,
    message: &str,
    timestamp: Option<u64>,
    node_id: Option<u64>,
    level: Option<&str>,
    source: Option<&str>,
    attrs: Option<BTreeMap<String, String>>,
) -> Result<AppendLogResponse, String> {
    let url = format!("http://127.0.0.1:{}/klog/data/append", port);
    let body = RichAppendLogBody {
        message: message.to_string(),
        timestamp,
        node_name: node_id.map(|v| format!("node-{}", v)),
        level: level.map(|v| v.to_string()),
        source: source.map(|v| v.to_string()),
        attrs,
        request_id: None,
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(format!("request {} returned {}: {}", url, status, text));
    }

    resp.json::<AppendLogResponse>()
        .await
        .map_err(|e| format!("decode {} failed: {}", url, e))
}

async fn query_logs_with_filters(
    client: &reqwest::Client,
    port: u16,
    filters: &[(&str, &str)],
) -> Result<QueryLogResponse, String> {
    let mut url = reqwest::Url::parse(&format!("http://127.0.0.1:{}/klog/data/query", port))
        .map_err(|e| format!("invalid query url: {}", e))?;
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in filters {
            q.append_pair(k, v);
        }
    }

    let resp = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(format!("request {} returned {}: {}", url, status, text));
    }

    resp.json::<QueryLogResponse>()
        .await
        .map_err(|e| format!("decode {} failed: {}", url, e))
}

#[tokio::test]
async fn test_three_node_multi_writer_and_filtered_query_interactions() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip multi-node rw test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_multi_rw_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;

        let rpc_by_node = nodes
            .iter()
            .map(|n| (n.node_id, n.rpc_port))
            .collect::<BTreeMap<_, _>>();

        let leader_rpc_port = rpc_by_node
            .get(&leader_id)
            .copied()
            .ok_or_else(|| format!("leader rpc port missing for node_id={}", leader_id))?;
        let follower_id = [1_u64, 2_u64, 3_u64]
            .into_iter()
            .find(|id| *id != leader_id)
            .ok_or_else(|| format!("failed to choose follower id, leader_id={}", leader_id))?;
        let follower_rpc_port = rpc_by_node
            .get(&follower_id)
            .copied()
            .ok_or_else(|| format!("follower rpc port missing for node_id={}", follower_id))?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let mut attrs1 = BTreeMap::new();
        attrs1.insert("service".to_string(), "kmsg".to_string());
        attrs1.insert("node".to_string(), "node1".to_string());
        let append1 = append_log_rich(
            &client,
            rpc_by_node[&1],
            "node1-warn-kmsg",
            Some(1_001),
            Some(1),
            Some("WARN"),
            Some("kernel/kmsg"),
            Some(attrs1),
        )
        .await?;

        let mut attrs2 = BTreeMap::new();
        attrs2.insert("service".to_string(), "net".to_string());
        attrs2.insert("node".to_string(), "node2".to_string());
        let append2 = append_log_rich(
            &client,
            rpc_by_node[&2],
            "node2-error-net",
            Some(1_002),
            Some(2),
            Some("ERROR"),
            Some("kernel/net"),
            Some(attrs2),
        )
        .await?;

        let mut attrs3 = BTreeMap::new();
        attrs3.insert("service".to_string(), "fs".to_string());
        attrs3.insert("node".to_string(), "node3".to_string());
        let append3 = append_log_rich(
            &client,
            rpc_by_node[&3],
            "node3-info-fs",
            Some(1_003),
            Some(3),
            Some("INFO"),
            Some("kernel/fs"),
            Some(attrs3),
        )
        .await?;

        let mut attrs4 = BTreeMap::new();
        attrs4.insert("service".to_string(), "kmsg".to_string());
        attrs4.insert("node".to_string(), "node2".to_string());
        let append4 = append_log_rich(
            &client,
            rpc_by_node[&2],
            "node2-warn-kmsg",
            Some(1_004),
            Some(2),
            Some("WARN"),
            Some("kernel/kmsg"),
            Some(attrs4),
        )
        .await?;

        let ids = [append1.id, append2.id, append3.id, append4.id];
        let min_id = *ids
            .iter()
            .min()
            .ok_or_else(|| "failed to get min id".to_string())?;
        let max_id = *ids
            .iter()
            .max()
            .ok_or_else(|| "failed to get max id".to_string())?;
        let expected_ids = ids.into_iter().collect::<BTreeSet<_>>();

        for rpc_port in rpc_by_node.values() {
            let queried = query_logs_with_strong_read(
                &client,
                *rpc_port,
                Some(min_id),
                Some(max_id),
                Some(16),
                Some(false),
                Some(true),
            )
            .await?;
            if queried.items.len() != 4 {
                return Err(format!(
                    "unexpected item len on rpc_port={}: expected=4, got={}",
                    rpc_port,
                    queried.items.len()
                ));
            }

            let got_ids = queried.items.iter().map(|e| e.id).collect::<BTreeSet<_>>();
            if got_ids != expected_ids {
                return Err(format!(
                    "unexpected ids on rpc_port={}: expected={:?}, got={:?}",
                    rpc_port, expected_ids, got_ids
                ));
            }
        }

        let source_kmsg = query_logs_with_filters(
            &client,
            follower_rpc_port,
            &[
                ("source", "kernel/kmsg"),
                ("strong_read", "true"),
                ("desc", "false"),
                ("limit", "16"),
            ],
        )
        .await?;
        let source_kmsg_msgs = source_kmsg
            .items
            .iter()
            .map(|e| e.message.as_str())
            .collect::<BTreeSet<_>>();
        let expected_kmsg_msgs = BTreeSet::from(["node1-warn-kmsg", "node2-warn-kmsg"]);
        if source_kmsg_msgs != expected_kmsg_msgs {
            return Err(format!(
                "unexpected source filter result on follower rpc_port={}: expected={:?}, got={:?}",
                follower_rpc_port, expected_kmsg_msgs, source_kmsg_msgs
            ));
        }

        let level_error = query_logs_with_filters(
            &client,
            leader_rpc_port,
            &[("level", "ERROR"), ("strong_read", "true"), ("limit", "16")],
        )
        .await?;
        if level_error.items.len() != 1 || level_error.items[0].message != "node2-error-net" {
            return Err(format!(
                "unexpected level filter result on leader rpc_port={}: len={}, first={:?}",
                leader_rpc_port,
                level_error.items.len(),
                level_error.items.first().map(|e| e.message.clone())
            ));
        }

        let attr_service_fs = query_logs_with_filters(
            &client,
            follower_rpc_port,
            &[
                ("attr_key", "service"),
                ("attr_value", "fs"),
                ("strong_read", "true"),
                ("limit", "16"),
            ],
        )
        .await?;
        if attr_service_fs.items.len() != 1 || attr_service_fs.items[0].message != "node3-info-fs" {
            return Err(format!(
                "unexpected attr filter result on follower rpc_port={}: len={}, first={:?}",
                follower_rpc_port,
                attr_service_fs.items.len(),
                attr_service_fs.items.first().map(|e| e.message.clone())
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
async fn test_three_node_concurrent_multi_writer_converges_for_all_nodes() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip concurrent multi-node rw test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_multi_rw_concurrent_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let _leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;

        let rpc_by_node = nodes
            .iter()
            .map(|n| (n.node_id, n.rpc_port))
            .collect::<BTreeMap<_, _>>();

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let writes_per_node = 5_u64;
        let total_writes = writes_per_node * 3;
        let mut writes = JoinSet::new();

        for node_id in [1_u64, 2_u64, 3_u64] {
            let rpc_port = rpc_by_node
                .get(&node_id)
                .copied()
                .ok_or_else(|| format!("rpc port missing for node_id={}", node_id))?;
            for seq in 0..writes_per_node {
                let client = client.clone();
                let message = format!("concurrent-node{}-seq{}", node_id, seq);
                let source = format!("kernel/concurrent/node{}", node_id);
                writes.spawn(async move {
                    let mut attrs = BTreeMap::new();
                    attrs.insert("service".to_string(), "concurrent".to_string());
                    attrs.insert("node".to_string(), node_id.to_string());
                    attrs.insert("seq".to_string(), seq.to_string());
                    let resp = append_log_rich(
                        &client,
                        rpc_port,
                        &message,
                        Some(20_000 + node_id * 100 + seq),
                        Some(node_id),
                        Some(if seq % 2 == 0 { "INFO" } else { "WARN" }),
                        Some(source.as_str()),
                        Some(attrs),
                    )
                    .await;
                    (message, node_id, resp)
                });
            }
        }

        let mut expected_messages = BTreeSet::new();
        let mut expected_ids = BTreeSet::new();
        let mut completed = 0_u64;

        while let Some(res) = writes.join_next().await {
            let (message, _node_id, append_res) = res.map_err(|e| format!("join write task failed: {}", e))?;
            let append = append_res?;
            expected_messages.insert(message);
            expected_ids.insert(append.id);
            completed += 1;
        }

        if completed != total_writes {
            return Err(format!(
                "unexpected completed write tasks: expected={}, got={}",
                total_writes, completed
            ));
        }
        if expected_ids.len() != total_writes as usize {
            return Err(format!(
                "unexpected unique ids after concurrent writes: expected={}, got={}",
                total_writes,
                expected_ids.len()
            ));
        }

        let min_id = *expected_ids
            .iter()
            .next()
            .ok_or_else(|| "expected ids should not be empty".to_string())?;
        let max_id = *expected_ids
            .iter()
            .next_back()
            .ok_or_else(|| "expected ids should not be empty".to_string())?;

        for rpc_port in rpc_by_node.values() {
            let queried = query_logs_with_strong_read(
                &client,
                *rpc_port,
                Some(min_id),
                Some(max_id),
                Some(total_writes as usize + 8),
                Some(false),
                Some(true),
            )
            .await?;
            if queried.items.len() != total_writes as usize {
                return Err(format!(
                    "unexpected concurrent query len on rpc_port={}: expected={}, got={}",
                    rpc_port,
                    total_writes,
                    queried.items.len()
                ));
            }

            let got_messages = queried
                .items
                .iter()
                .map(|e| e.message.clone())
                .collect::<BTreeSet<_>>();
            if got_messages != expected_messages {
                return Err(format!(
                    "unexpected concurrent query messages on rpc_port={}: expected_count={}, got_count={}",
                    rpc_port,
                    expected_messages.len(),
                    got_messages.len()
                ));
            }
        }

        for node_id in [1_u64, 2_u64, 3_u64] {
            let source = format!("kernel/concurrent/node{}", node_id);
            let source_items = query_logs_with_filters(
                &client,
                rpc_by_node[&node_id],
                &[
                    ("source", source.as_str()),
                    ("strong_read", "true"),
                    ("limit", "32"),
                    ("desc", "false"),
                ],
            )
            .await?;
            if source_items.items.len() != writes_per_node as usize {
                return Err(format!(
                    "unexpected source-filtered len for node_id={}: expected={}, got={}",
                    node_id,
                    writes_per_node,
                    source_items.items.len()
                ));
            }
            let expected_node_name = format!("node-{}", node_id);
            if source_items
                .items
                .iter()
                .any(|e| e.node_name != expected_node_name)
            {
                return Err(format!(
                    "source-filtered query contains unexpected node_name for node_id={}",
                    node_id
                ));
            }
        }

        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}
