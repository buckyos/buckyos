mod common;

use common::*;
use std::time::Duration;

#[tokio::test]
async fn test_single_node_smoke() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip single-node smoke: localhost bind is not available");
        return Ok(());
    }

    let port = choose_free_port().map_err(|e| format!("choose free port failed: {}", e))?;
    let cluster_name = format!("klog_smoke_{}", port);
    let mut node = spawn_node(1, port, &cluster_name, true, &[], "voter").await?;

    let wait_result = wait_single_node_leader(port, 1, Duration::from_secs(20)).await;
    node.stop().await;
    wait_result
}

#[tokio::test]
async fn test_single_node_business_log_append_and_query() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip single-node business-log test: localhost bind is not available");
        return Ok(());
    }

    let port = choose_free_port().map_err(|e| format!("choose free port failed: {}", e))?;
    let cluster_name = format!("klog_business_log_{}", port);
    let mut node = spawn_node(1, port, &cluster_name, true, &[], "voter").await?;

    let result = async {
        wait_single_node_leader(port, 1, Duration::from_secs(20)).await?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let append1 = append_log(&client, port, "kernel-init", Some(100), Some(1)).await?;
        let append2 = append_log(&client, port, "driver-up", Some(101), Some(1)).await?;
        let append3 = append_log(&client, port, "service-ready", Some(102), Some(1)).await?;

        if !(append1.id < append2.id && append2.id < append3.id) {
            return Err(format!(
                "append ids are not strictly increasing: [{}, {}, {}]",
                append1.id, append2.id, append3.id
            ));
        }

        let asc = query_logs(
            &client,
            port,
            Some(append1.id),
            Some(append3.id),
            Some(10),
            Some(false),
        )
        .await?;
        let asc_ids = asc.items.iter().map(|e| e.id).collect::<Vec<_>>();
        if asc_ids != vec![append1.id, append2.id, append3.id] {
            return Err(format!("unexpected asc ids: {:?}", asc_ids));
        }

        let desc = query_logs(
            &client,
            port,
            Some(append1.id),
            Some(append3.id),
            Some(2),
            Some(true),
        )
        .await?;
        let desc_ids = desc.items.iter().map(|e| e.id).collect::<Vec<_>>();
        if desc_ids != vec![append3.id, append2.id] {
            return Err(format!("unexpected desc ids: {:?}", desc_ids));
        }

        if asc.items[0].message != "kernel-init"
            || asc.items[1].message != "driver-up"
            || asc.items[2].message != "service-ready"
        {
            return Err(format!(
                "unexpected query messages: [{}, {}, {}]",
                asc.items[0].message, asc.items[1].message, asc.items[2].message
            ));
        }

        if asc.items[0].timestamp != 100
            || asc.items[1].timestamp != 101
            || asc.items[2].timestamp != 102
            || asc.items.iter().any(|e| e.node_id != 1)
        {
            return Err("unexpected query timestamps or node_id".to_string());
        }

        Ok(())
    }
    .await;

    node.stop().await;
    result
}
