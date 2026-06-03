use crate::config::{
    KLogJoinRetryConfig, KLogJoinRetryStrategy, KLogJoinTargetRole, KLogRuntimeConfig,
};
use klog::network::{KLogAdminRequestType, KLogClusterStateResponse};
use klog::{KClusterTransportMode, KNode, KRaftRef};
use log::{error, info, warn};
use rand::Rng;
use rand::seq::SliceRandom;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::task::JoinHandle;

const CONFIG_CHANGE_CONFLICT_MARKER: &str = "undergoing a configuration change";
const AUTO_JOIN_REFUSED_LOCAL_STATE_MARKER: &str =
    "auto-join refused by local persisted membership";

pub async fn initialize_cluster_if_needed(cfg: &KLogRuntimeConfig, raft: &KRaftRef) {
    if cfg.auto_bootstrap {
        let mut members = BTreeMap::new();
        members.insert(
            cfg.node_id,
            KNode {
                id: cfg.node_id,
                addr: cfg.advertise_addr.clone(),
                port: cfg.advertise_port,
                inter_port: cfg.advertise_inter_port,
                admin_port: cfg.advertise_admin_port,
                rpc_port: if cfg.enable_rpc_server {
                    cfg.rpc_advertise_port
                } else {
                    0
                },
                node_name: cfg.advertise_node_name.clone(),
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

pub fn spawn_auto_join_task(cfg: &KLogRuntimeConfig, raft: &KRaftRef) -> Option<JoinHandle<()>> {
    if !cfg.auto_bootstrap && !cfg.join_targets.is_empty() {
        let join_cfg = cfg.clone();
        let join_raft = raft.clone();
        Some(tokio::spawn(async move {
            run_auto_join_loop(join_cfg, join_raft).await;
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

async fn run_auto_join_loop(cfg: KLogRuntimeConfig, raft: KRaftRef) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(cfg.join_retry.request_timeout_ms))
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
        if cfg.join_retry.max_attempts > 0 && attempts >= cfg.join_retry.max_attempts {
            warn!(
                "Auto-join reached max attempts without success: attempts={}, node_id={}, targets={:?}, join_target_role={}",
                cfg.join_retry.max_attempts, cfg.node_id, cfg.join_targets, cfg.join_target_role
            );
            return;
        }
        attempts += 1;

        match try_join_once(&client, &cfg, &raft).await {
            Ok(msg) => {
                info!(
                    "Auto-join succeeded: node_id={}, attempt={}, join_target_role={}, {}",
                    cfg.node_id, attempts, cfg.join_target_role, msg
                );
                return;
            }
            Err(e) => {
                if e.contains(AUTO_JOIN_REFUSED_LOCAL_STATE_MARKER) {
                    warn!(
                        "Auto-join stopped because local persisted membership requires explicit admin rejoin: node_id={}, err={}",
                        cfg.node_id, e
                    );
                    return;
                }
                let sleep_ms = compute_retry_delay_ms(&cfg.join_retry, attempts, &e);
                warn!(
                    "Auto-join attempt failed: node_id={}, attempt={}, join_target_role={}, err={}, next_retry_in_ms={}",
                    cfg.node_id, attempts, cfg.join_target_role, e, sleep_ms
                );
                tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
            }
        }
    }
}

async fn try_join_once(
    client: &reqwest::Client,
    cfg: &KLogRuntimeConfig,
    raft: &KRaftRef,
) -> Result<String, String> {
    let cluster_state_path = KLogAdminRequestType::ClusterState.klog_path();
    let mut errors = Vec::new();

    let mut seed_targets = cfg.join_targets.clone();
    if cfg.join_retry.shuffle_targets_each_round {
        let mut rng = rand::thread_rng();
        seed_targets.shuffle(&mut rng);
    }

    for target in &seed_targets {
        let seed_state = match fetch_cluster_state(client, target, &cluster_state_path).await {
            Ok(state) => state,
            Err(err) => {
                errors.push(format!("target='{}': {}", target, err));
                continue;
            }
        };
        if let Err(err) =
            ensure_cluster_identity_matches(cfg, &seed_state, target, "seed-cluster-state")
        {
            errors.push(format!("target='{}': {}", target, err));
            continue;
        }

        // Prefer leader endpoint for admin write APIs if follower can report it.
        let mut admin_targets = Vec::new();
        if let Some(leader_id) = seed_state.current_leader
            && let Some(leader_node) = seed_state.nodes.get(&leader_id)
        {
            admin_targets.push(admin_target_from_node(cfg, leader_node));
        }
        admin_targets.push(target.clone());
        dedup_targets(&mut admin_targets);

        for admin_target in admin_targets {
            match join_and_promote_once(client, cfg, raft, &admin_target).await {
                Ok(msg) => {
                    return Ok(format!(
                        "seed_target='{}', admin_target='{}', {}",
                        target, admin_target, msg
                    ));
                }
                Err(err) => {
                    warn!(
                        "Auto-join admin target failed: seed_target='{}', admin_target='{}', node_id={}, err={}",
                        target, admin_target, cfg.node_id, err
                    );
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

fn compute_retry_delay_ms(retry: &KLogJoinRetryConfig, attempts: u32, last_err: &str) -> u64 {
    let base = match retry.strategy {
        KLogJoinRetryStrategy::Fixed => retry.initial_interval_ms,
        KLogJoinRetryStrategy::Exponential => {
            let power = attempts.saturating_sub(1) as i32;
            let factor = retry.multiplier.powi(power);
            ((retry.initial_interval_ms as f64) * factor).round() as u64
        }
    }
    .clamp(1, retry.max_interval_ms);

    let jittered = apply_jitter(base, retry.jitter_ratio);
    let mut total = jittered;
    if last_err.contains(CONFIG_CHANGE_CONFLICT_MARKER) {
        total = total.saturating_add(retry.config_change_conflict_extra_backoff_ms);
    }
    total.max(1)
}

fn apply_jitter(base_ms: u64, jitter_ratio: f64) -> u64 {
    if jitter_ratio <= 0.0 || base_ms == 0 {
        return base_ms.max(1);
    }

    let r = jitter_ratio.clamp(0.0, 1.0);
    let low = 1.0 - r;
    let high = 1.0 + r;
    let mut rng = rand::thread_rng();
    let factor: f64 = rng.gen_range(low..=high);
    ((base_ms as f64) * factor).round() as u64
}

async fn join_and_promote_once(
    client: &reqwest::Client,
    cfg: &KLogRuntimeConfig,
    raft: &KRaftRef,
    admin_target: &str,
) -> Result<String, String> {
    let cluster_state_path = KLogAdminRequestType::ClusterState.klog_path();
    let add_learner_path = KLogAdminRequestType::AddLearner.klog_path();
    let change_membership_path = KLogAdminRequestType::ChangeMembership.klog_path();

    let state_before = fetch_cluster_state(client, admin_target, &cluster_state_path).await?;
    ensure_cluster_identity_matches(cfg, &state_before, admin_target, "before-change")?;
    info!(
        "Auto-join state before change: admin_target={}, cluster_name={}, cluster_id={}, node_id={}, leader={:?}, voters={:?}, learners={:?}",
        admin_target,
        state_before.cluster_name,
        state_before.cluster_id,
        cfg.node_id,
        state_before.current_leader,
        state_before.voters,
        state_before.learners
    );

    if state_before.voters.contains(&cfg.node_id) {
        ensure_existing_remote_node_matches_config(
            cfg,
            &state_before,
            admin_target,
            "already-voter",
        )?;
        if cfg.join_target_role == KLogJoinTargetRole::Learner {
            return Ok(format!(
                "node is already voter, target_role=learner does not downgrade existing voter: node_id={}, voters={:?}",
                cfg.node_id, state_before.voters
            ));
        }
        return Ok(format!(
            "node already voter: node_id={}, voters={:?}",
            cfg.node_id, state_before.voters
        ));
    }
    ensure_auto_join_allowed_by_local_state(cfg, raft, &state_before, admin_target)?;

    if !state_before.learners.contains(&cfg.node_id) {
        let advertised_rpc_port = if cfg.enable_rpc_server {
            cfg.rpc_advertise_port
        } else {
            0
        };
        let mut add_url = build_admin_url(admin_target, &add_learner_path)?;
        {
            let mut q = add_url.query_pairs_mut();
            q.append_pair("node_id", &cfg.node_id.to_string());
            q.append_pair("addr", &cfg.advertise_addr);
            q.append_pair("port", &cfg.advertise_port.to_string());
            q.append_pair("inter_port", &cfg.advertise_inter_port.to_string());
            q.append_pair("admin_port", &cfg.advertise_admin_port.to_string());
            q.append_pair("rpc_port", &advertised_rpc_port.to_string());
            if let Some(node_name) = cfg.advertise_node_name.as_deref() {
                q.append_pair("node_name", node_name);
            }
            q.append_pair("blocking", if cfg.join_blocking { "true" } else { "false" });
        }

        info!(
            "Auto-join add-learner: admin_target={}, node_id={}, node_name={:?}, addr={}, raft_port={}, inter_port={}, admin_port={}, rpc_port={}, blocking={}",
            admin_target,
            cfg.node_id,
            cfg.advertise_node_name.as_deref(),
            cfg.advertise_addr,
            cfg.advertise_port,
            cfg.advertise_inter_port,
            cfg.advertise_admin_port,
            advertised_rpc_port,
            cfg.join_blocking
        );
        let add_result = send_admin_post(client, add_url, "add-learner").await?;
        info!(
            "Auto-join add-learner succeeded: admin_target={}, node_id={}, response={}",
            admin_target, cfg.node_id, add_result
        );
    } else {
        ensure_existing_remote_node_matches_config(
            cfg,
            &state_before,
            admin_target,
            "already-learner",
        )?;
        info!(
            "Auto-join skip add-learner because node is already learner: admin_target={}, node_id={}",
            admin_target, cfg.node_id
        );
    }

    let state_after_add = fetch_cluster_state(client, admin_target, &cluster_state_path).await?;
    ensure_cluster_identity_matches(cfg, &state_after_add, admin_target, "after-add-learner")?;
    if cfg.join_target_role == KLogJoinTargetRole::Learner {
        return Ok(format!(
            "node joined as learner: node_id={}, voters={:?}, learners={:?}",
            cfg.node_id, state_after_add.voters, state_after_add.learners
        ));
    }

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
    ensure_cluster_identity_matches(
        cfg,
        &state_after_change,
        admin_target,
        "after-change-membership",
    )?;
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

fn ensure_existing_remote_node_matches_config(
    cfg: &KLogRuntimeConfig,
    state: &KLogClusterStateResponse,
    admin_target: &str,
    stage: &str,
) -> Result<(), String> {
    let Some(remote) = state.nodes.get(&cfg.node_id) else {
        let msg = format!(
            "node identity mismatch at stage={}: remote membership contains node_id={} but cluster-state nodes map has no entry: admin_target={}, remote_voters={:?}, remote_learners={:?}",
            stage, cfg.node_id, admin_target, state.voters, state.learners
        );
        error!("{}", msg);
        return Err(msg);
    };

    let expected_rpc_port = if cfg.enable_rpc_server {
        cfg.rpc_advertise_port
    } else {
        0
    };
    let expected_node_name = cfg.advertise_node_name.as_deref().unwrap_or("");
    let remote_node_name = remote.node_name.as_deref().unwrap_or("");
    let mut mismatches = Vec::new();
    if remote.addr != cfg.advertise_addr {
        mismatches.push(format!(
            "addr expected={} remote={}",
            cfg.advertise_addr, remote.addr
        ));
    }
    if remote.port != cfg.advertise_port {
        mismatches.push(format!(
            "raft_port expected={} remote={}",
            cfg.advertise_port, remote.port
        ));
    }
    if remote.inter_port != cfg.advertise_inter_port {
        mismatches.push(format!(
            "inter_port expected={} remote={}",
            cfg.advertise_inter_port, remote.inter_port
        ));
    }
    if remote.admin_port != cfg.advertise_admin_port {
        mismatches.push(format!(
            "admin_port expected={} remote={}",
            cfg.advertise_admin_port, remote.admin_port
        ));
    }
    if remote.rpc_port != expected_rpc_port {
        mismatches.push(format!(
            "rpc_port expected={} remote={}",
            expected_rpc_port, remote.rpc_port
        ));
    }
    if remote_node_name != expected_node_name {
        mismatches.push(format!(
            "node_name expected={} remote={}",
            expected_node_name, remote_node_name
        ));
    }

    if mismatches.is_empty() {
        return Ok(());
    }

    let msg = format!(
        "node identity mismatch at stage={}: node_id={} is already present in remote membership but local advertised identity differs: admin_target={}, mismatches=[{}], remote_node={:?}",
        stage,
        cfg.node_id,
        admin_target,
        mismatches.join("; "),
        remote
    );
    error!("{}", msg);
    Err(msg)
}

fn ensure_auto_join_allowed_by_local_state(
    cfg: &KLogRuntimeConfig,
    raft: &KRaftRef,
    remote_state: &KLogClusterStateResponse,
    admin_target: &str,
) -> Result<(), String> {
    let (local_voters, local_learners) = local_membership_ids(raft);
    if should_block_auto_join_from_local_membership(
        &local_voters,
        &local_learners,
        remote_state,
        cfg.node_id,
    ) {
        let msg = format!(
            "{}: local raft state is initialized but remote membership does not contain this node: node_id={}, admin_target={}, local_voters={:?}, local_learners={:?}, remote_voters={:?}, remote_learners={:?}",
            AUTO_JOIN_REFUSED_LOCAL_STATE_MARKER,
            cfg.node_id,
            admin_target,
            local_voters,
            local_learners,
            remote_state.voters,
            remote_state.learners
        );
        warn!("{}", msg);
        return Err(msg);
    }
    Ok(())
}

fn local_membership_ids(raft: &KRaftRef) -> (Vec<u64>, Vec<u64>) {
    let metrics = raft.metrics();
    let metrics = metrics.borrow().clone();
    let membership = metrics.membership_config.membership();
    let voters = membership.voter_ids().collect::<Vec<_>>();
    let learners = membership.learner_ids().collect::<Vec<_>>();
    (voters, learners)
}

fn should_block_auto_join_from_local_membership(
    local_voters: &[u64],
    local_learners: &[u64],
    remote_state: &KLogClusterStateResponse,
    node_id: u64,
) -> bool {
    let local_initialized = !local_voters.is_empty() || !local_learners.is_empty();
    let remote_contains_node =
        remote_state.voters.contains(&node_id) || remote_state.learners.contains(&node_id);
    local_initialized && !remote_contains_node
}

fn ensure_cluster_identity_matches(
    cfg: &KLogRuntimeConfig,
    state: &KLogClusterStateResponse,
    target: &str,
    stage: &str,
) -> Result<(), String> {
    if state.cluster_id != cfg.cluster_id {
        let msg = format!(
            "cluster identity mismatch at stage={}: target='{}', expected_cluster_id='{}', got_cluster_id='{}'",
            stage, target, cfg.cluster_id, state.cluster_id
        );
        error!("{}", msg);
        return Err(msg);
    }

    if state.cluster_name != cfg.cluster_name {
        warn!(
            "cluster name mismatch but cluster_id matched at stage={}: target='{}', expected_cluster_name='{}', got_cluster_name='{}'",
            stage, target, cfg.cluster_name, state.cluster_name
        );
    }

    Ok(())
}

fn admin_target_from_node(cfg: &KLogRuntimeConfig, node: &KNode) -> String {
    if cfg.cluster_network.mode != KClusterTransportMode::Direct
        && let Some(node_name) = node.node_name.as_deref()
        && !node_name.trim().is_empty()
    {
        return gateway_admin_target(
            &cfg.cluster_network.gateway_addr,
            &cfg.cluster_network.gateway_route_prefix,
            node_name,
        );
    }

    let admin_port = if node.admin_port > 0 {
        node.admin_port
    } else if node.inter_port > 0 {
        node.inter_port
    } else {
        node.port
    };
    format!("{}:{}", node.addr, admin_port)
}

fn gateway_admin_target(gateway_addr: &str, route_prefix: &str, node_name: &str) -> String {
    let gateway_addr = gateway_addr.trim().trim_end_matches('/');
    let gateway_base =
        if gateway_addr.starts_with("http://") || gateway_addr.starts_with("https://") {
            gateway_addr.to_string()
        } else {
            format!("http://{}", gateway_addr)
        };
    let route_prefix = normalize_gateway_route_prefix(route_prefix);
    let route_base = if route_prefix == "/" {
        String::new()
    } else {
        route_prefix
    };
    format!("{gateway_base}{route_base}/{node_name}/admin")
}

fn normalize_gateway_route_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim().trim_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    format!("/{trimmed}")
}

fn admin_path_suffix(path: &str) -> &str {
    path.strip_prefix("/klog/admin/")
        .unwrap_or_else(|| path.trim_start_matches('/'))
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
    let base_path = url.path().trim_end_matches('/');
    let next_path = if base_path.is_empty() || base_path == "/" {
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    } else {
        format!("{}/{}", base_path, admin_path_suffix(path))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{
        admin_target_from_node, build_admin_url, build_promote_voters_csv, dedup_targets,
        ensure_cluster_identity_matches, ensure_existing_remote_node_matches_config,
        should_block_auto_join_from_local_membership,
    };
    use crate::config::{
        KLogJoinRetryConfig, KLogJoinTargetRole, KLogRaftConfig, KLogRuntimeConfig,
    };
    use klog::network::KLogClusterStateResponse;
    use klog::{KClusterTransportConfig, KClusterTransportMode, KNode, KNodeId};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

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
    fn test_build_admin_url_preserves_gateway_admin_base_path() {
        let path = klog::network::KLogAdminRequestType::ClusterState.klog_path();
        let url = build_admin_url("http://127.0.0.1:3180/.cluster/klog/ood1/admin", &path)
            .expect("build admin url");
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:3180/.cluster/klog/ood1/admin/cluster-state"
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
        let cfg = sample_cfg("cluster_a", "cluster_a_id");
        let node = KNode {
            id: 10 as KNodeId,
            addr: "127.0.0.1".to_string(),
            port: 21001,
            inter_port: 21002,
            admin_port: 21003,
            rpc_port: 31001,
            node_name: None,
        };
        let target = admin_target_from_node(&cfg, &node);
        assert_eq!(target, "127.0.0.1:21003");
    }

    #[test]
    fn test_admin_target_from_node_fallback_to_raft_port() {
        let cfg = sample_cfg("cluster_a", "cluster_a_id");
        let node = KNode {
            id: 10 as KNodeId,
            addr: "127.0.0.1".to_string(),
            port: 21001,
            inter_port: 0,
            admin_port: 0,
            rpc_port: 31001,
            node_name: None,
        };
        let target = admin_target_from_node(&cfg, &node);
        assert_eq!(target, "127.0.0.1:21001");
    }

    #[test]
    fn test_admin_target_from_node_uses_gateway_proxy_node_name() {
        let mut cfg = sample_cfg("cluster_a", "cluster_a_id");
        cfg.cluster_network.mode = KClusterTransportMode::GatewayProxy;
        cfg.cluster_network.gateway_addr = "127.0.0.1:3180".to_string();
        cfg.cluster_network.gateway_route_prefix = "/.cluster/klog".to_string();
        let node = KNode {
            id: 10 as KNodeId,
            addr: "10.0.0.8".to_string(),
            port: 21001,
            inter_port: 21002,
            admin_port: 21003,
            rpc_port: 31001,
            node_name: Some("ood2".to_string()),
        };
        let target = admin_target_from_node(&cfg, &node);
        assert_eq!(target, "http://127.0.0.1:3180/.cluster/klog/ood2/admin");
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

    fn sample_cfg(cluster_name: &str, cluster_id: &str) -> KLogRuntimeConfig {
        KLogRuntimeConfig {
            node_id: 1,
            listen_addr: "0.0.0.0:21001".to_string(),
            inter_node_listen_addr: "0.0.0.0:21002".to_string(),
            admin_listen_addr: "127.0.0.1:21003".to_string(),
            enable_rpc_server: true,
            rpc_listen_addr: "127.0.0.1:21101".to_string(),
            advertise_addr: "127.0.0.1".to_string(),
            advertise_port: 21001,
            advertise_inter_port: 21002,
            advertise_admin_port: 21003,
            rpc_advertise_port: 21101,
            advertise_node_name: None,
            data_dir: PathBuf::from("/tmp/klog_cluster_test"),
            cluster_name: cluster_name.to_string(),
            cluster_id: cluster_id.to_string(),
            auto_bootstrap: false,
            state_store_sync_write: true,
            join_targets: vec![],
            join_blocking: false,
            join_target_role: KLogJoinTargetRole::Voter,
            join_retry: KLogJoinRetryConfig::default(),
            raft: KLogRaftConfig::default(),
            cluster_network: KClusterTransportConfig::default(),
            admin_local_only: true,
            rpc: Default::default(),
            meta_compaction: Default::default(),
        }
    }

    fn sample_state(cluster_name: &str, cluster_id: &str) -> KLogClusterStateResponse {
        KLogClusterStateResponse {
            node_id: 2,
            cluster_name: cluster_name.to_string(),
            cluster_id: cluster_id.to_string(),
            server_state: "Follower".to_string(),
            current_leader: Some(1),
            voters: vec![1, 2, 3],
            learners: vec![],
            nodes: BTreeMap::new(),
        }
    }

    #[test]
    fn test_cluster_identity_match_ok() {
        let cfg = sample_cfg("cluster_a", "cluster_a_id");
        let state = sample_state("cluster_a", "cluster_a_id");
        ensure_cluster_identity_matches(&cfg, &state, "127.0.0.1:21001", "test")
            .expect("identity should match");
    }

    #[test]
    fn test_cluster_id_mismatch_rejected() {
        let cfg = sample_cfg("cluster_a", "cluster_a_id");
        let state = sample_state("cluster_b", "cluster_b_id");
        let err = ensure_cluster_identity_matches(&cfg, &state, "127.0.0.1:21001", "test")
            .expect_err("identity mismatch should fail");
        assert!(err.contains("cluster identity mismatch"));
        assert!(err.contains("expected_cluster_id='cluster_a_id'"));
    }

    #[test]
    fn test_cluster_name_mismatch_allowed_when_id_matches() {
        let cfg = sample_cfg("cluster_a", "cluster_a_id");
        let state = sample_state("renamed_cluster", "cluster_a_id");
        ensure_cluster_identity_matches(&cfg, &state, "127.0.0.1:21001", "test")
            .expect("name mismatch should be allowed when cluster_id matches");
    }

    fn insert_cfg_node(state: &mut KLogClusterStateResponse, cfg: &KLogRuntimeConfig) {
        state.nodes.insert(
            cfg.node_id,
            KNode {
                id: cfg.node_id,
                addr: cfg.advertise_addr.clone(),
                port: cfg.advertise_port,
                inter_port: cfg.advertise_inter_port,
                admin_port: cfg.advertise_admin_port,
                rpc_port: cfg.rpc_advertise_port,
                node_name: cfg.advertise_node_name.clone(),
            },
        );
    }

    #[test]
    fn test_existing_remote_node_identity_match_ok() {
        let mut cfg = sample_cfg("cluster_a", "cluster_a_id");
        cfg.advertise_node_name = Some("ood1".to_string());
        let mut state = sample_state("cluster_a", "cluster_a_id");
        insert_cfg_node(&mut state, &cfg);

        ensure_existing_remote_node_matches_config(&cfg, &state, "127.0.0.1:21001", "test")
            .expect("existing node identity should match");
    }

    #[test]
    fn test_existing_remote_node_identity_mismatch_rejected() {
        let mut cfg = sample_cfg("cluster_a", "cluster_a_id");
        cfg.advertise_node_name = Some("replacement-ood".to_string());
        let mut state = sample_state("cluster_a", "cluster_a_id");
        state.nodes.insert(
            cfg.node_id,
            KNode {
                id: cfg.node_id,
                addr: cfg.advertise_addr.clone(),
                port: cfg.advertise_port + 1,
                inter_port: cfg.advertise_inter_port,
                admin_port: cfg.advertise_admin_port,
                rpc_port: cfg.rpc_advertise_port,
                node_name: Some("ood1".to_string()),
            },
        );

        let err = ensure_existing_remote_node_matches_config(
            &cfg,
            &state,
            "127.0.0.1:21001",
            "already-voter",
        )
        .expect_err("identity mismatch should fail");
        assert!(err.contains("node identity mismatch"));
        assert!(err.contains("node_id=1"));
        assert!(err.contains("raft_port"));
        assert!(err.contains("node_name"));
    }

    #[test]
    fn test_existing_remote_node_missing_spec_rejected() {
        let cfg = sample_cfg("cluster_a", "cluster_a_id");
        let state = sample_state("cluster_a", "cluster_a_id");

        let err = ensure_existing_remote_node_matches_config(
            &cfg,
            &state,
            "127.0.0.1:21001",
            "already-voter",
        )
        .expect_err("missing remote node spec should fail");
        assert!(err.contains("node identity mismatch"));
        assert!(err.contains("nodes map has no entry"));
    }

    #[test]
    fn test_auto_join_blocks_initialized_removed_node() {
        let mut state = sample_state("cluster_a", "cluster_a_id");
        state.voters = vec![1, 3];
        state.learners = vec![];

        assert!(should_block_auto_join_from_local_membership(
            &[1, 2, 3],
            &[],
            &state,
            2
        ));
    }

    #[test]
    fn test_auto_join_allows_fresh_node() {
        let mut state = sample_state("cluster_a", "cluster_a_id");
        state.voters = vec![1, 3];
        state.learners = vec![];

        assert!(!should_block_auto_join_from_local_membership(
            &[],
            &[],
            &state,
            2
        ));
    }

    #[test]
    fn test_auto_join_allows_existing_learner_resume() {
        let mut state = sample_state("cluster_a", "cluster_a_id");
        state.voters = vec![1, 3];
        state.learners = vec![2];

        assert!(!should_block_auto_join_from_local_membership(
            &[1, 2, 3],
            &[],
            &state,
            2
        ));
    }
}
