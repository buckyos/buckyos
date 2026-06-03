async fn run_local_gateway_failover_smoke_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-dv";
    let setup = prepare_local_gateway_setup(harness, MULTI_NODE_MODE, route_prefix, 3).await?;
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
    wait_voters(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let follower = nodes
        .iter()
        .find(|node| node.id != leader_id)
        .ok_or_else(|| format!("failed to choose follower; leader_id={}", leader_id))?;
    let first_source = format!(
        "test/test_klog_cluster_dv-gateway-{}",
        unique_suffix("write")
    );
    let first_append = append_via_cluster_inter_route(
        &client,
        gateway_addr(follower, ingress_port).as_str(),
        route_prefix,
        follower.name.as_str(),
        first_source.as_str(),
        "gateway cluster transport write before failover",
    )
    .await?;
    wait_log_visible_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        first_append.id,
        first_source.as_str(),
        Duration::from_secs(20),
    )
    .await?;
    println!(
        "[klog-cluster-dv] gateway transport write replicated before failover: id={}, leader_id={}, follower={}",
        first_append.id, leader_id, follower.name
    );

    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    harness.stop(format!("klog-{}", leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != leader_id)
        .cloned()
        .collect::<Vec<_>>();
    let new_leader = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(leader_id),
        Duration::from_secs(60),
    )
    .await?;
    let write_node = alive_nodes
        .iter()
        .find(|node| node.id != new_leader)
        .unwrap_or_else(|| alive_nodes.first().unwrap());
    let failover_source = format!(
        "test/test_klog_cluster_dv-failover-{}",
        unique_suffix("write")
    );
    let failover_append = append_via_cluster_inter_route(
        &client,
        gateway_addr(write_node, ingress_port).as_str(),
        route_prefix,
        write_node.name.as_str(),
        failover_source.as_str(),
        "gateway cluster transport write after failover",
    )
    .await?;
    wait_log_visible_on_nodes(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        failover_append.id,
        failover_source.as_str(),
        Duration::from_secs(25),
    )
    .await?;
    println!(
        "[klog-cluster-dv] failover write replicated: old_leader={}, new_leader={}, write_node={}, id={}",
        leader_id, new_leader, write_node.name, failover_append.id
    );
    println!("[klog-cluster-dv] local gateway failover smoke success");
    Ok(())
}

async fn run_local_gateway_failover_smoke() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_failover_smoke_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_membership_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-membership-dv";
    let setup = prepare_local_gateway_setup(harness, MEMBERSHIP_MODE, route_prefix, 4).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let voter_nodes = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let learner = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing learner node".to_string())?;
    let seed = voter_nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &voter_nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
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

    let learner_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let learner_config = write_klog_config(harness, &learner, &learner_options)?;
    spawn_klog(harness, &klog_daemon_bin, &learner_config, &learner)?;
    wait_tcp("127.0.0.1", learner.ports.admin, Duration::from_secs(12)).await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = voter_nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &learner,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(60),
    )
    .await?;
    println!(
        "[klog-cluster-dv] gateway admin add-learner ok: leader={}, learner={}",
        leader.name, learner.name
    );

    let leader_id = wait_consistent_leader(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = voter_nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found after add-learner", leader_id))?;
    let follower = voter_nodes
        .iter()
        .find(|node| node.id != leader_id)
        .ok_or_else(|| "failed to choose follower for admin semantics check".to_string())?;
    let (status_change, body_change) = post_change_membership_via_admin_route(
        &client,
        gateway_addr(follower, ingress_port).as_str(),
        route_prefix,
        follower.name.as_str(),
        &[1, 2, 3],
        true,
    )
    .await?;
    if status_change != reqwest::StatusCode::CONFLICT {
        return Err(format!(
            "follower change-membership should return 409 via gateway, got status={}, body={}",
            status_change, body_change
        ));
    }

    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        learner.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove-learner via gateway returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;

    let (repeat_status, repeat_body) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        learner.id,
    )
    .await?;
    if repeat_status != reqwest::StatusCode::OK
        && repeat_status != reqwest::StatusCode::CONFLICT
        && repeat_status != reqwest::StatusCode::INTERNAL_SERVER_ERROR
    {
        return Err(format!(
            "unexpected repeated remove-learner status via gateway: status={}, body={}",
            repeat_status, repeat_body
        ));
    }
    wait_membership(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(30),
    )
    .await?;
    println!(
        "[klog-cluster-dv] gateway admin remove-learner semantics ok: repeat_status={}",
        repeat_status
    );
    println!("[klog-cluster-dv] local gateway membership DV success");
    Ok(())
}

async fn run_local_gateway_membership() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_membership_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

