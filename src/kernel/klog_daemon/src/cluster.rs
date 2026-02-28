use crate::config::KLogRuntimeConfig;
use klog::network::{KLogAdminRequestType, KLogClusterStateResponse};
use klog::{KNode, KRaftRef};
use log::{error, info, warn};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::task::JoinHandle;

pub async fn initialize_cluster_if_needed(cfg: &KLogRuntimeConfig, raft: &KRaftRef) {
    if cfg.auto_bootstrap {
        let mut members = BTreeMap::new();
        members.insert(
            cfg.node_id,
            KNode {
                id: cfg.node_id,
                addr: cfg.advertise_addr.clone(),
                port: cfg.advertise_port,
            },
        );
        match raft.initialize(members).await {
            Ok(()) => {
                info!(
                    "Raft cluster initialized: node_id={}, cluster_name={}",
                    cfg.node_id, cfg.cluster_name
                );
            }
            Err(e) => {
                warn!(
                    "Raft initialize skipped/failed (possibly already initialized): {}",
                    e
                );
            }
        }
    } else {
        info!("Skip raft initialize because KLOG_AUTO_BOOTSTRAP=false");
    }
}

pub fn spawn_auto_join_task(cfg: &KLogRuntimeConfig) -> Option<JoinHandle<()>> {
    if !cfg.auto_bootstrap && !cfg.join_targets.is_empty() {
        let join_cfg = cfg.clone();
        Some(tokio::spawn(async move {
            run_auto_join_loop(join_cfg).await;
        }))
    } else {
        if !cfg.auto_bootstrap {
            warn!(
                "KLOG_AUTO_BOOTSTRAP=false but no join targets configured; daemon will run without auto-join"
            );
        }
        None
    }
}

async fn run_auto_join_loop(cfg: KLogRuntimeConfig) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build();
    let client = match client {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to create join http client: {}", e);
            return;
        }
    };

    let mut attempts: u32 = 0;
    loop {
        if cfg.join_max_attempts > 0 && attempts >= cfg.join_max_attempts {
            warn!(
                "Auto-join reached max attempts without success: attempts={}, node_id={}, targets={:?}",
                cfg.join_max_attempts, cfg.node_id, cfg.join_targets
            );
            return;
        }
        attempts += 1;

        match try_join_once(&client, &cfg).await {
            Ok(msg) => {
                info!(
                    "Auto-join succeeded: node_id={}, attempt={}, {}",
                    cfg.node_id, attempts, msg
                );
                return;
            }
            Err(e) => {
                warn!(
                    "Auto-join attempt failed: node_id={}, attempt={}, err={}",
                    cfg.node_id, attempts, e
                );
            }
        }

        tokio::time::sleep(Duration::from_millis(cfg.join_retry_interval_ms)).await;
    }
}

async fn try_join_once(
    client: &reqwest::Client,
    cfg: &KLogRuntimeConfig,
) -> Result<String, String> {
    let cluster_state_path = KLogAdminRequestType::ClusterState.klog_path();
    let mut errors = Vec::new();

    for target in &cfg.join_targets {
        let seed_state = match fetch_cluster_state(client, target, &cluster_state_path).await {
            Ok(state) => state,
            Err(err) => {
                errors.push(format!("target='{}': {}", target, err));
                continue;
            }
        };

        // Prefer leader endpoint for admin write APIs if follower can report it.
        let mut admin_targets = Vec::new();
        if let Some(leader_id) = seed_state.current_leader
            && let Some(leader_node) = seed_state.nodes.get(&leader_id)
        {
            admin_targets.push(admin_target_from_node(leader_node));
        }
        admin_targets.push(target.clone());
        dedup_targets(&mut admin_targets);

        for admin_target in admin_targets {
            match join_and_promote_once(client, cfg, &admin_target).await {
                Ok(msg) => {
                    return Ok(format!(
                        "seed_target='{}', admin_target='{}', {}",
                        target, admin_target, msg
                    ));
                }
                Err(err) => {
                    errors.push(format!(
                        "seed_target='{}', admin_target='{}': {}",
                        target, admin_target, err
                    ));
                }
            }
        }
    }

    Err(errors.join(" | "))
}

async fn join_and_promote_once(
    client: &reqwest::Client,
    cfg: &KLogRuntimeConfig,
    admin_target: &str,
) -> Result<String, String> {
    let cluster_state_path = KLogAdminRequestType::ClusterState.klog_path();
    let add_learner_path = KLogAdminRequestType::AddLearner.klog_path();
    let change_membership_path = KLogAdminRequestType::ChangeMembership.klog_path();

    let state_before = fetch_cluster_state(client, admin_target, &cluster_state_path).await?;
    info!(
        "Auto-join state before change: admin_target={}, node_id={}, leader={:?}, voters={:?}, learners={:?}",
        admin_target,
        cfg.node_id,
        state_before.current_leader,
        state_before.voters,
        state_before.learners
    );

    if state_before.voters.contains(&cfg.node_id) {
        return Ok(format!(
            "node already voter: node_id={}, voters={:?}",
            cfg.node_id, state_before.voters
        ));
    }

    if !state_before.learners.contains(&cfg.node_id) {
        let mut add_url = build_admin_url(admin_target, &add_learner_path)?;
        {
            let mut q = add_url.query_pairs_mut();
            q.append_pair("node_id", &cfg.node_id.to_string());
            q.append_pair("addr", &cfg.advertise_addr);
            q.append_pair("port", &cfg.advertise_port.to_string());
            q.append_pair("blocking", if cfg.join_blocking { "true" } else { "false" });
        }

        info!(
            "Auto-join add-learner: admin_target={}, node_id={}, addr={}, port={}, blocking={}",
            admin_target, cfg.node_id, cfg.advertise_addr, cfg.advertise_port, cfg.join_blocking
        );
        let add_result = send_admin_post(client, add_url, "add-learner").await?;
        info!(
            "Auto-join add-learner succeeded: admin_target={}, node_id={}, response={}",
            admin_target, cfg.node_id, add_result
        );
    } else {
        info!(
            "Auto-join skip add-learner because node is already learner: admin_target={}, node_id={}",
            admin_target, cfg.node_id
        );
    }

    let state_after_add = fetch_cluster_state(client, admin_target, &cluster_state_path).await?;
    if state_after_add.voters.contains(&cfg.node_id) {
        return Ok(format!(
            "node became voter without explicit promote: node_id={}, voters={:?}",
            cfg.node_id, state_after_add.voters
        ));
    }

    let voters_csv = build_promote_voters_csv(&state_after_add.voters, cfg.node_id);
    let mut change_url = build_admin_url(admin_target, &change_membership_path)?;
    {
        let mut q = change_url.query_pairs_mut();
        q.append_pair("voters", &voters_csv);
        q.append_pair("retain", "true");
    }

    info!(
        "Auto-join promote learner to voter: admin_target={}, node_id={}, voters={}",
        admin_target, cfg.node_id, voters_csv
    );
    let change_result = send_admin_post(client, change_url, "change-membership").await?;
    info!(
        "Auto-join change-membership succeeded: admin_target={}, node_id={}, response={}",
        admin_target, cfg.node_id, change_result
    );

    let state_after_change = fetch_cluster_state(client, admin_target, &cluster_state_path).await?;
    if state_after_change.voters.contains(&cfg.node_id) {
        return Ok(format!(
            "node promoted to voter: node_id={}, voters={:?}, learners={:?}",
            cfg.node_id, state_after_change.voters, state_after_change.learners
        ));
    }

    Err(format!(
        "change-membership succeeded but node is still not voter: node_id={}, voters={:?}, learners={:?}",
        cfg.node_id, state_after_change.voters, state_after_change.learners
    ))
}

async fn fetch_cluster_state(
    client: &reqwest::Client,
    target: &str,
    cluster_state_path: &str,
) -> Result<KLogClusterStateResponse, String> {
    let url = build_admin_url(target, cluster_state_path)?;
    let response = client.get(url.clone()).send().await.map_err(|e| {
        format!(
            "cluster-state request send failed: target='{}', url={}, err={}",
            target, url, e
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read body: {}>", e));
        return Err(format!(
            "cluster-state request failed: target='{}', url={}, status={}, body={}",
            target, url, status, body
        ));
    }

    response
        .json::<KLogClusterStateResponse>()
        .await
        .map_err(|e| {
            format!(
                "cluster-state response decode failed: target='{}', url={}, err={}",
                target, url, e
            )
        })
}

async fn send_admin_post(
    client: &reqwest::Client,
    url: reqwest::Url,
    action: &str,
) -> Result<String, String> {
    let response = client.post(url.clone()).send().await.map_err(|e| {
        format!(
            "{} request send failed: url={}, err={}",
            action,
            url.as_str(),
            e
        )
    })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|e| format!("<failed to read body: {}>", e));

    if status.is_success() {
        Ok(body)
    } else {
        Err(format!(
            "{} request failed: url={}, status={}, body={}",
            action,
            url.as_str(),
            status,
            body
        ))
    }
}

fn build_promote_voters_csv(existing_voters: &[u64], node_id: u64) -> String {
    let mut voters = existing_voters.iter().copied().collect::<BTreeSet<_>>();
    voters.insert(node_id);
    voters
        .into_iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn admin_target_from_node(node: &KNode) -> String {
    format!("{}:{}", node.addr, node.port)
}

fn dedup_targets(targets: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    targets.retain(|target| {
        let normalized = target.trim().to_string();
        if normalized.is_empty() {
            return false;
        }
        if seen.contains(&normalized) {
            return false;
        }
        seen.insert(normalized);
        true
    });
}

fn build_admin_url(target: &str, path: &str) -> Result<reqwest::Url, String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err("empty join target".to_string());
    }

    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{}", trimmed)
    };

    let mut url = reqwest::Url::parse(&with_scheme)
        .map_err(|e| format!("invalid join target url '{}': {}", trimmed, e))?;
    url.set_path(path);
    url.set_query(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{admin_target_from_node, build_admin_url, build_promote_voters_csv, dedup_targets};
    use klog::{KNode, KNodeId};

    #[test]
    fn test_build_admin_url_adds_scheme_and_path() {
        let path = klog::network::KLogAdminRequestType::AddLearner.klog_path();
        let url = build_admin_url("127.0.0.1:21001", &path).expect("build admin url");
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:21001/klog/admin/add-learner"
        );
    }

    #[test]
    fn test_build_admin_url_rejects_empty() {
        let path = klog::network::KLogAdminRequestType::AddLearner.klog_path();
        let err = build_admin_url("  ", &path).expect_err("should fail");
        assert!(err.contains("empty join target"));
    }

    #[test]
    fn test_build_promote_voters_csv_sorted_and_deduped() {
        let csv = build_promote_voters_csv(&[3, 1, 3, 2], 2);
        assert_eq!(csv, "1,2,3");
    }

    #[test]
    fn test_admin_target_from_node() {
        let node = KNode {
            id: 10 as KNodeId,
            addr: "127.0.0.1".to_string(),
            port: 21001,
        };
        let target = admin_target_from_node(&node);
        assert_eq!(target, "127.0.0.1:21001");
    }

    #[test]
    fn test_dedup_targets() {
        let mut targets = vec![
            "127.0.0.1:21001".to_string(),
            "127.0.0.1:21001".to_string(),
            "".to_string(),
            "127.0.0.1:21002".to_string(),
        ];
        dedup_targets(&mut targets);
        assert_eq!(
            targets,
            vec!["127.0.0.1:21001".to_string(), "127.0.0.1:21002".to_string()]
        );
    }
}
