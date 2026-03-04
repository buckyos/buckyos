mod common;

use common::*;
use klog::network::KLogAdminRequestType;
use reqwest::StatusCode;
use std::time::{Duration, Instant};
use tokio::time::sleep;

async fn post_admin(
    client: &reqwest::Client,
    port: u16,
    path: &str,
    query: &[(&str, String)],
) -> Result<(StatusCode, String), String> {
    let mut url = reqwest::Url::parse(&format!("http://127.0.0.1:{}{}", port, path))
        .map_err(|e| format!("invalid admin url: {}", e))?;
    {
        let mut qp = url.query_pairs_mut();
        for (k, v) in query {
            qp.append_pair(k, v);
        }
    }

    let resp = client
        .post(url.clone())
        .send()
        .await
        .map_err(|e| format!("admin request {} failed: {}", url, e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_else(|_| String::new());
    Ok((status, body))
}

async fn post_change_membership(
    client: &reqwest::Client,
    port: u16,
    voters: &[u64],
    retain: bool,
) -> Result<(StatusCode, String), String> {
    let voters_csv = voters
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",");
    post_admin(
        client,
        port,
        &KLogAdminRequestType::ChangeMembership.klog_path(),
        &[
            ("voters", voters_csv),
            ("retain", if retain { "true" } else { "false" }.to_string()),
        ],
    )
    .await
}

async fn post_remove_learner(
    client: &reqwest::Client,
    port: u16,
    node_id: u64,
) -> Result<(StatusCode, String), String> {
    post_admin(
        client,
        port,
        &KLogAdminRequestType::RemoveLearner.klog_path(),
        &[("node_id", node_id.to_string())],
    )
    .await
}

async fn retry_change_membership_until_success(
    nodes: &[TestNode],
    voters: &[u64],
    retain: bool,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| format!("failed to build retry http client: {}", e))?;
    let ports = nodes.iter().map(|n| n.port).collect::<Vec<_>>();
    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting change-membership success: voters={:?}, retain={}, ports={:?}, last_err={}",
                voters, retain, ports, last_err
            ));
        }

        let leader_id = match wait_consistent_leader_on_ports(&ports, Duration::from_secs(5)).await
        {
            Ok(id) => id,
            Err(e) => {
                last_err = format!("discover leader failed: {}", e);
                sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let leader_port = nodes
            .iter()
            .find(|n| n.node_id == leader_id)
            .map(|n| n.port)
            .ok_or_else(|| format!("leader port not found for leader_id={}", leader_id))?;

        let (status, body) = post_change_membership(&client, leader_port, voters, retain).await?;
        if status == StatusCode::OK {
            return Ok(());
        }

        if status == StatusCode::CONFLICT
            || (status == StatusCode::INTERNAL_SERVER_ERROR
                && body.contains("configuration change"))
        {
            last_err = format!(
                "transient change-membership failure: status={}, body={}",
                status, body
            );
            sleep(Duration::from_millis(250)).await;
            continue;
        }

        return Err(format!(
            "unexpected change-membership response: status={}, body={}",
            status, body
        ));
    }
}

#[tokio::test]
async fn test_admin_remove_learner_repeated_call_keeps_membership_stable() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip admin remove-learner idempotent test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(4)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let cluster_name = format!(
        "klog_admin_remove_learner_{}_{}_{}_{}",
        port1, port2, port3, port4
    );
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;
    nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(50),
        )
        .await?;

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(50),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;

        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_port = if leader == 1 {
            port1
        } else if leader == 2 {
            port2
        } else {
            port3
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let (status, body) = post_remove_learner(&client, leader_port, 4).await?;
        if status != StatusCode::OK
            && status != StatusCode::CONFLICT
            && status != StatusCode::INTERNAL_SERVER_ERROR
        {
            return Err(format!(
                "unexpected repeated remove-learner status: status={}, body={}",
                status, body
            ));
        }

        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}

#[tokio::test]
async fn test_admin_write_on_follower_returns_not_leader() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip admin non-leader test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_admin_non_leader_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let follower_port = [port1, port2, port3]
            .into_iter()
            .find(|p| {
                let id = if *p == port1 {
                    1
                } else if *p == port2 {
                    2
                } else {
                    3
                };
                id != leader
            })
            .ok_or_else(|| "no follower port found".to_string())?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let (status_change, body_change) =
            post_change_membership(&client, follower_port, &[1, 2, 3], true).await?;
        if status_change != StatusCode::CONFLICT {
            return Err(format!(
                "follower change-membership should return 409, got status={}, body={}",
                status_change, body_change
            ));
        }

        let (status_remove, body_remove) = post_remove_learner(&client, follower_port, 999).await?;
        if status_remove != StatusCode::CONFLICT {
            return Err(format!(
                "follower remove-learner should return 409, got status={}, body={}",
                status_remove, body_remove
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
async fn test_admin_change_membership_retry_after_transient_conflict() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip admin config-change retry test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(4)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let cluster_name = format!(
        "klog_admin_change_retry_{}_{}_{}_{}",
        port1, port2, port3, port4
    );
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;
    nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(55),
        )
        .await?;

        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_port = if leader == 1 {
            port1
        } else if leader == 2 {
            port2
        } else {
            port3
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let req1 = post_change_membership(&client, leader_port, &[1, 2, 3, 4], true);
        let req2 = post_change_membership(&client, leader_port, &[1, 2, 3, 4], true);
        let (r1, r2) = tokio::join!(req1, req2);
        let (s1, b1) = r1?;
        let (s2, b2) = r2?;
        if s1 != StatusCode::OK && s2 != StatusCode::OK {
            return Err(format!(
                "concurrent promote requests both failed: r1=({}, {}), r2=({}, {})",
                s1, b1, s2, b2
            ));
        }

        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3, 4],
            &[],
            Duration::from_secs(70),
        )
        .await?;

        retry_change_membership_until_success(&nodes, &[1, 2, 3], true, Duration::from_secs(45))
            .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(70),
        )
        .await?;

        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}
