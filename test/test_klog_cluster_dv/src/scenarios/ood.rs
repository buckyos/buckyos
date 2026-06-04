async fn run_local_gateway_ood_membership_three_to_four(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-membership-3-4-dv";
    let setup = prepare_local_gateway_setup(harness, OOD_MEMBERSHIP_MODE, route_prefix, 4).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let added_ood = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing fourth OOD node".to_string())?;
    let seed = base_voters
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

    for node in &base_voters {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
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
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[0],
        &base_voters[1],
        &base_voters,
        "three-voters-before-add",
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config = write_klog_config(harness, &added_ood, &added_options)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before add OOD", leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &added_ood,
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

    let promote_leader = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &base_voters[0],
        &nodes,
        "four-voters-after-add",
    )
    .await?;

    let demote_leader = change_voters_via_current_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
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
        Duration::from_secs(70),
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before remove OOD", leader_id))?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove fourth OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[2],
        &base_voters[0],
        &base_voters,
        "three-voters-after-remove",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood membership 3<->4 ok: promote_leader={}, demote_leader={}, removed_ood={}",
        promote_leader, demote_leader, added_ood.name
    );
    Ok(())
}

async fn run_local_gateway_ood_membership_one_to_two(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-membership-1-2-dv";
    let setup = prepare_local_gateway_setup(harness, OOD_MEMBERSHIP_MODE, route_prefix, 2).await?;
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
        .cloned()
        .ok_or_else(|| "missing single OOD seed node".to_string())?;
    let added_ood = nodes
        .get(1)
        .cloned()
        .ok_or_else(|| "missing second OOD node".to_string())?;
    let seed_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let config = write_klog_config(harness, &seed, &seed_config)?;
    spawn_klog(harness, &klog_daemon_bin, &config, &seed)?;
    wait_tcp("127.0.0.1", seed.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", seed.ports.inter, Duration::from_secs(12)).await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1],
        &[],
        Duration::from_secs(30),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &seed,
        std::slice::from_ref(&seed),
        "one-voter-before-add",
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config = write_klog_config(harness, &added_ood, &added_options)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(&seed, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        &added_ood,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1],
        &[2],
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1, 2],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &seed,
        &nodes,
        "two-voters-after-add",
    )
    .await?;

    let demote_leader =
        change_voters_via_current_leader(&client, &nodes, ingress_port, route_prefix, &[1], true)
            .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1],
        &[2],
        Duration::from_secs(70),
    )
    .await?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(&seed, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove second OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &seed,
        std::slice::from_ref(&seed),
        "one-voter-after-remove",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood membership 1<->2 ok: promote_leader={}, demote_leader={}, removed_ood={}",
        promote_leader, demote_leader, added_ood.name
    );
    Ok(())
}

async fn run_local_gateway_ood_membership_two_to_three(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-membership-2-3-dv";
    let setup = prepare_local_gateway_setup(harness, OOD_MEMBERSHIP_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(2).cloned().collect::<Vec<_>>();
    let added_ood = nodes
        .get(2)
        .cloned()
        .ok_or_else(|| "missing third OOD node".to_string())?;
    let seed = base_voters
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

    for node in &base_voters {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
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
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[0],
        &base_voters[1],
        &base_voters,
        "two-voters-before-add-third",
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config = write_klog_config(harness, &added_ood, &added_options)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before add third OOD", leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &added_ood,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[3],
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &base_voters[0],
        &nodes,
        "three-voters-after-add-third",
    )
    .await?;

    let demote_leader = change_voters_via_current_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[3],
        Duration::from_secs(70),
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| {
            format!(
                "leader node {} not found before remove third OOD",
                leader_id
            )
        })?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove third OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[1],
        &base_voters[0],
        &base_voters,
        "two-voters-after-remove-third",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood membership 2<->3 ok: promote_leader={}, demote_leader={}, removed_ood={}",
        promote_leader, demote_leader, added_ood.name
    );
    Ok(())
}

async fn run_local_gateway_ood_membership() -> Result<(), String> {
    {
        let mut harness = LocalHarness::new()?;
        let result = run_local_gateway_ood_membership_three_to_four(&mut harness).await;
        if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
            harness.keep_temp = true;
            eprintln!(
                "[klog-cluster-dv] keeping temp root for diagnostics: {}",
                harness.root.display()
            );
        }
        result?;
    }

    {
        let mut harness = LocalHarness::new()?;
        let result = run_local_gateway_ood_membership_two_to_three(&mut harness).await;
        if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
            harness.keep_temp = true;
            eprintln!(
                "[klog-cluster-dv] keeping temp root for diagnostics: {}",
                harness.root.display()
            );
        }
        result?;
    }

    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_membership_one_to_two(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_leader_failover_shrink_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-leader-failover-shrink-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_LEADER_FAILOVER_SHRINK_MODE, route_prefix, 3)
            .await?;
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
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
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
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
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
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let before_failover = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes[0],
        &nodes[1],
        &nodes,
        "three-voters-before-leader-failover",
    )
    .await?;

    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .ok_or_else(|| format!("leader node {} not found", old_leader_id))?;
    harness.stop(format!("klog-{}", old_leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();

    let new_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &before_failover,
        Duration::from_secs(40),
    )
    .await?;

    let failover_writer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or_else(|| alive_nodes.first().unwrap());
    let failover_target = alive_nodes
        .iter()
        .find(|node| node.id == new_leader_id)
        .unwrap_or(failover_writer);
    let after_failover = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        failover_writer,
        failover_target,
        &alive_nodes,
        "two-voters-after-leader-failover",
    )
    .await?;

    let alive_voters = alive_nodes.iter().map(|node| node.id).collect::<Vec<_>>();
    let shrink_leader_id = change_voters_via_current_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        alive_voters.as_slice(),
        false,
    )
    .await?;
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        alive_voters.as_slice(),
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let stable_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;

    for witness in [&before_failover, &after_failover] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &alive_nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(40),
        )
        .await?;
    }

    let post_shrink_writer = alive_nodes
        .iter()
        .find(|node| node.id != stable_leader_id)
        .unwrap_or_else(|| alive_nodes.first().unwrap());
    let post_shrink_target = alive_nodes
        .iter()
        .find(|node| node.id == stable_leader_id)
        .unwrap_or(post_shrink_writer);
    let after_shrink = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        post_shrink_writer,
        post_shrink_target,
        &alive_nodes,
        "two-voters-after-shrink",
    )
    .await?;

    for witness in [&before_failover, &after_failover, &after_shrink] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &alive_nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(40),
        )
        .await?;
    }

    println!(
        "[klog-cluster-dv] ood leader failover shrink ok: old_leader={}, new_leader={}, shrink_leader={}, stable_leader={}, alive_voters={:?}, log_ids=[{},{},{}]",
        old_leader_id,
        new_leader_id,
        shrink_leader_id,
        stable_leader_id,
        alive_voters,
        before_failover.log_id,
        after_failover.log_id,
        after_shrink.log_id
    );
    Ok(())
}

async fn run_local_gateway_ood_leader_failover_shrink() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_leader_failover_shrink_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_seed_unavailable_join_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-seed-unavailable-join-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_SEED_UNAVAILABLE_JOIN_MODE, route_prefix, 4)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing seed OOD node".to_string())?;
    let survivors = base_voters
        .iter()
        .filter(|node| node.id != seed.id)
        .cloned()
        .collect::<Vec<_>>();
    if survivors.len() != 2 {
        return Err(format!(
            "expected two survivor OOD nodes, got {}",
            survivors.len()
        ));
    }
    let added_ood = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing fourth OOD node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &base_voters {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
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
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let before_seed_stop = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &survivors[0],
        &base_voters,
        "seed-unavailable-before-stop",
    )
    .await?;

    harness.stop(format!("klog-{}", seed.name).as_str())?;
    let survivor_leader = wait_consistent_leader(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        Some(seed.id),
        Duration::from_secs(70),
    )
    .await?;
    wait_membership(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(40),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        &before_seed_stop,
        Duration::from_secs(40),
    )
    .await?;
    let after_seed_stop = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &survivors[0],
        &survivors[1],
        &survivors,
        "seed-unavailable-after-stop",
    )
    .await?;

    let join_targets = base_voters
        .iter()
        .map(|target| gateway_admin_join_target(&added_ood, ingress_port, route_prefix, target))
        .collect::<Vec<_>>();
    println!(
        "[klog-cluster-dv] fourth OOD join_targets={}",
        join_targets.join(",")
    );
    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "voter",
    };
    let added_config =
        write_klog_config_with_join_targets(harness, &added_ood, &added_options, &join_targets)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let online_after_join = survivors
        .iter()
        .cloned()
        .chain(std::iter::once(added_ood.clone()))
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &online_after_join,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    for witness in [&before_seed_stop, &after_seed_stop] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &online_after_join,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(50),
        )
        .await?;
    }
    let after_join = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &survivors[0],
        &online_after_join,
        "seed-unavailable-after-fourth-join",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood seed-unavailable auto-join ok: stopped_seed={}, survivor_leader={}, added_ood={}, log_ids=[{},{},{}]",
        seed.id,
        survivor_leader,
        added_ood.id,
        before_seed_stop.log_id,
        after_seed_stop.log_id,
        after_join.log_id
    );
    Ok(())
}

async fn run_local_gateway_ood_seed_unavailable_join() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_seed_unavailable_join_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_single_to_two_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-single-to-two-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_SINGLE_TO_TWO_MODE, route_prefix, 2).await?;
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
        .cloned()
        .ok_or_else(|| "missing single OOD seed node".to_string())?;
    let added_ood = nodes
        .get(1)
        .cloned()
        .ok_or_else(|| "missing second OOD node".to_string())?;

    let seed_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let config = write_klog_config(harness, &seed, &seed_config)?;
    spawn_klog(harness, &klog_daemon_bin, &config, &seed)?;
    wait_tcp("127.0.0.1", seed.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", seed.ports.inter, Duration::from_secs(12)).await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1],
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let before_join = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &seed,
        std::slice::from_ref(&seed),
        "single-voter-before-learner-join",
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config = write_klog_config(harness, &added_ood, &added_options)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(&seed, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        &added_ood,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1],
        &[2],
        Duration::from_secs(60),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &before_join,
        Duration::from_secs(40),
    )
    .await?;

    let learner_phase = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &seed,
        &nodes,
        "single-voter-plus-learner",
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1, 2],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    for witness in [&before_join, &learner_phase] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(40),
        )
        .await?;
    }
    let post_promote = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &seed,
        &nodes,
        "two-voters-after-single-promote",
    )
    .await?;
    for witness in [&before_join, &learner_phase, &post_promote] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(40),
        )
        .await?;
    }

    println!(
        "[klog-cluster-dv] ood single-to-two ok: promote_leader={}, log_ids=[{},{},{}]",
        promote_leader, before_join.log_id, learner_phase.log_id, post_promote.log_id
    );
    Ok(())
}

async fn run_local_gateway_ood_single_to_two() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_single_to_two_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_two_voter_loss_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-two-voter-loss-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_TWO_VOTER_LOSS_MODE, route_prefix, 2).await?;
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
        .cloned()
        .ok_or_else(|| "missing two-voter seed node".to_string())?;
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
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
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
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let before_loss = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes[0],
        &nodes[1],
        &nodes,
        "two-voters-before-loss",
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
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    harness.stop(format!("klog-{}", leader.name).as_str())?;
    let survivor = nodes
        .iter()
        .find(|node| node.id != leader_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "survivor node not found after stopping leader {}",
                leader_id
            )
        })?;

    if wait_consistent_leader(
        &client,
        std::slice::from_ref(&survivor),
        ingress_port,
        route_prefix,
        Some(leader_id),
        Duration::from_secs(12),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "two-voter cluster unexpectedly elected a replacement leader after node {} stopped",
            leader_id
        ));
    }

    if append_via_cluster_inter_route(
        &client,
        gateway_addr(&survivor, ingress_port).as_str(),
        route_prefix,
        survivor.name.as_str(),
        "test/two-voter-loss-unavailable",
        "write should fail without two-voter quorum",
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "surviving voter {} unexpectedly accepted append without quorum",
            survivor.id
        ));
    }

    if query_via_cluster_inter_route(
        &client,
        gateway_addr(&survivor, ingress_port).as_str(),
        route_prefix,
        survivor.name.as_str(),
        before_loss.log_id,
        before_loss.log_source.as_str(),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "surviving voter {} unexpectedly served strong log query without quorum",
            survivor.id
        ));
    }

    if put_meta_via_cluster_inter_route(
        &client,
        gateway_addr(&survivor, ingress_port).as_str(),
        route_prefix,
        survivor.name.as_str(),
        "test/two_voter_loss/unavailable_meta",
        "should-not-commit",
        Some(0),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "surviving voter {} unexpectedly accepted meta put without quorum",
            survivor.id
        ));
    }

    println!(
        "[klog-cluster-dv] ood two-voter loss ok: stopped_leader={}, survivor={}, pre_loss_log_id={}",
        leader_id, survivor.id, before_loss.log_id
    );
    Ok(())
}

async fn run_local_gateway_ood_two_voter_loss() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_two_voter_loss_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

fn ood_snapshot_membership_raft_patch() -> KLogRaftPatch<'static> {
    KLogRaftPatch {
        install_snapshot_timeout_ms: Some(15_000),
        max_payload_entries: Some(16),
        replication_lag_threshold: Some(10),
        snapshot_policy: Some("since_last:25"),
        snapshot_max_chunk_size_bytes: Some(512 * 1024),
        max_in_snapshot_log_to_keep: Some(5),
        purge_batch_size: Some(50),
    }
}

fn raft_snapshot_install_crash_raft_patch(chunk_bytes: usize) -> KLogRaftPatch<'static> {
    KLogRaftPatch {
        install_snapshot_timeout_ms: Some(30_000),
        max_payload_entries: Some(8),
        replication_lag_threshold: Some(5),
        snapshot_policy: Some("since_last:20"),
        snapshot_max_chunk_size_bytes: Some(chunk_bytes as u64),
        max_in_snapshot_log_to_keep: Some(2),
        purge_batch_size: Some(20),
    }
}

async fn run_local_gateway_ood_snapshot_membership_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let item_count = parse_env_usize(
        ENV_OOD_SNAPSHOT_MEMBERSHIP_ITEMS,
        DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_ITEMS,
    )?;
    let value_bytes = parse_env_usize(
        ENV_OOD_SNAPSHOT_MEMBERSHIP_VALUE_BYTES,
        DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_VALUE_BYTES,
    )?;
    let route_prefix = "/.cluster/klog-it-ood-snapshot-membership-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_SNAPSHOT_MEMBERSHIP_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(2).cloned().collect::<Vec<_>>();
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing snapshot membership seed node".to_string())?;
    let second = base_voters
        .get(1)
        .cloned()
        .ok_or_else(|| "missing second snapshot membership voter".to_string())?;
    let added_ood = nodes
        .get(2)
        .cloned()
        .ok_or_else(|| "missing snapshot membership third OOD".to_string())?;
    let raft_patch = ood_snapshot_membership_raft_patch();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &base_voters {
        let config = write_klog_config_with_raft_patch(harness, node, &voter_config, raft_patch)?;
        spawn_klog_with_log_level(harness, &klog_daemon_bin, &config, node, "info")?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
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
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(50),
    )
    .await?;

    let pre_add_witness = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &second,
        "pre-add",
        item_count,
        value_bytes,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &base_voters,
        &pre_add_witness,
        Duration::from_secs(40),
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let snapshot_leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("snapshot leader {} not found", leader_id))?;
    let leader_snapshot_count =
        wait_snapshot_file_count(harness, snapshot_leader, 1, Duration::from_secs(70)).await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config =
        write_klog_config_with_raft_patch(harness, &added_ood, &added_options, raft_patch)?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &added_config, &added_ood, "info")?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before snapshot add", leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &added_ood,
        false,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[3],
        Duration::from_secs(80),
    )
    .await?;
    let added_snapshot_count =
        wait_snapshot_file_count(harness, &added_ood, 1, Duration::from_secs(80)).await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes,
        &pre_add_witness,
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let post_promote_count = (item_count / 5).max(20);
    let post_promote_witness = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &seed,
        "post-promote",
        post_promote_count,
        value_bytes,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes,
        &post_promote_witness,
        Duration::from_secs(60),
    )
    .await?;

    let demote_leader = change_voters_via_current_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[3],
        Duration::from_secs(80),
    )
    .await?;

    let remaining_voters = base_voters.clone();
    let leader_id = wait_consistent_leader(
        &client,
        &remaining_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader = remaining_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before snapshot remove", leader_id))?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove snapshot-added OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &remaining_voters,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;

    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &remaining_voters,
        &pre_add_witness,
        Duration::from_secs(60),
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &remaining_voters,
        &post_promote_witness,
        Duration::from_secs(60),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &second,
        &remaining_voters,
        "snapshot-two-voters-after-remove-added",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood snapshot membership ok: items={}, value_bytes={}, leader_snapshot_count={}, added_snapshot_count={}, promote_leader={}, demote_leader={}, removed_ood={}",
        item_count,
        value_bytes,
        leader_snapshot_count,
        added_snapshot_count,
        promote_leader,
        demote_leader,
        added_ood.name
    );
    Ok(())
}

async fn run_local_gateway_ood_snapshot_membership() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_snapshot_membership_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

