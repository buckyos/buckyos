async fn run_local_gateway_restart_recovery_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-restart-dv";
    let setup =
        prepare_local_gateway_setup(harness, RESTART_RECOVERY_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let mut configs = BTreeMap::new();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_before = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader_node = nodes
        .iter()
        .find(|node| node.id == leader_before)
        .ok_or_else(|| format!("leader node {} not found", leader_before))?;
    let leader_gateway_addr_before = gateway_addr(leader_node, ingress_port);
    let suffix = unique_suffix("restart");
    let source = format!("test/test_klog_restart_recovery_dv-{}", suffix);
    let first = append_via_cluster_inter_route(
        &client,
        leader_gateway_addr_before.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        source.as_str(),
        "restart recovery write before full stop 1",
    )
    .await?;
    let second = append_via_cluster_inter_route(
        &client,
        leader_gateway_addr_before.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        source.as_str(),
        "restart recovery write before full stop 2",
    )
    .await?;
    if second.id <= first.id {
        return Err(format!(
            "append id not increasing before restart: first_id={}, second_id={}",
            first.id, second.id
        ));
    }
    let meta_key = format!("test/test_klog_restart_recovery_dv/meta/{}", suffix);
    let meta_before = put_meta_via_cluster_inter_route(
        &client,
        leader_gateway_addr_before.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        meta_key.as_str(),
        "before-restart",
        Some(0),
    )
    .await?;
    if meta_before.key != meta_key || meta_before.revision != 1 {
        return Err(format!(
            "unexpected meta before restart: key={}, revision={}",
            meta_before.key, meta_before.revision
        ));
    }

    for node in &nodes {
        harness.stop(format!("klog-{}", node.name).as_str())?;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    for node_id in [2_u64, 3_u64, 1_u64] {
        let node = nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| format!("restart node {} not found", node_id))?;
        let config = configs
            .get(&node_id)
            .ok_or_else(|| format!("restart config for node {} not found", node_id))?;
        spawn_klog(harness, &klog_daemon_bin, config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
    }

    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    let leader_after = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(60),
    )
    .await?;
    let leader_node = nodes
        .iter()
        .find(|node| node.id == leader_after)
        .ok_or_else(|| format!("post-restart leader node {} not found", leader_after))?;
    let leader_gateway_addr_after = gateway_addr(leader_node, ingress_port);

    for log_id in [first.id, second.id] {
        let response = query_via_cluster_inter_route(
            &client,
            leader_gateway_addr_after.as_str(),
            route_prefix,
            leader_node.name.as_str(),
            log_id,
            source.as_str(),
        )
        .await?;
        require_query_match(&response, log_id, source.as_str())?;
    }

    let meta_after_restart = query_meta_via_cluster_inter_route(
        &client,
        leader_gateway_addr_after.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        meta_key.as_str(),
    )
    .await?;
    if meta_after_restart.items.len() != 1
        || meta_after_restart.items[0].key != meta_key
        || meta_after_restart.items[0].value != "before-restart"
        || meta_after_restart.items[0].revision != 1
    {
        return Err(format!(
            "unexpected meta after restart: items={:?}",
            meta_after_restart
                .items
                .iter()
                .map(|item| format!(
                    "key={}, value={}, revision={}",
                    item.key, item.value, item.revision
                ))
                .collect::<Vec<_>>()
        ));
    }

    let after = append_via_cluster_inter_route(
        &client,
        leader_gateway_addr_after.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        source.as_str(),
        "restart recovery write after full restart",
    )
    .await?;
    let response = query_via_cluster_inter_route(
        &client,
        leader_gateway_addr_after.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        after.id,
        source.as_str(),
    )
    .await?;
    require_query_match(&response, after.id, source.as_str())?;

    let meta_after_update = put_meta_via_cluster_inter_route(
        &client,
        leader_gateway_addr_after.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        meta_key.as_str(),
        "after-restart",
        Some(1),
    )
    .await?;
    if meta_after_update.revision != 2 {
        return Err(format!(
            "unexpected meta revision after restart update: expected=2, got={}",
            meta_after_update.revision
        ));
    }
    println!(
        "[klog-cluster-dv] restart recovery ok: leader_before={}, leader_after={}, log_ids=[{}, {}, {}], meta_revision={}",
        leader_before, leader_after, first.id, second.id, after.id, meta_after_update.revision
    );
    println!("[klog-cluster-dv] local gateway restart recovery DV success");
    Ok(())
}

async fn run_local_gateway_restart_recovery() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_restart_recovery_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

