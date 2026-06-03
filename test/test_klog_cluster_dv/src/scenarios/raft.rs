async fn run_local_gateway_raft_old_leader_rejoin_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-old-leader-rejoin-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_OLD_LEADER_REJOIN_MODE, route_prefix, 3).await?;
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
    let mut configs = BTreeMap::new();
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
        .timeout(Duration::from_secs(8))
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
        .ok_or_else(|| format!("old leader node {} not found", old_leader_id))?;
    let pre_writer = nodes
        .iter()
        .find(|node| node.id != old_leader_id)
        .unwrap_or(old_leader);
    let old_leader_gateway = gateway_addr(old_leader, ingress_port);
    let pre_writer_gateway = gateway_addr(pre_writer, ingress_port);
    let suffix = unique_suffix("raft-old-leader-rejoin");
    let log_source = format!("test/klog_raft_old_leader_rejoin_dv/log/{}", suffix);
    let prefix = format!("test/klog_raft_old_leader_rejoin_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);

    let before_log = append_via_cluster_inter_route(
        &client,
        pre_writer_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        log_source.as_str(),
        "old leader rejoin write before crash",
    )
    .await?;
    wait_log_visible_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        before_log.id,
        log_source.as_str(),
        Duration::from_secs(30),
    )
    .await?;

    let a_v1 = put_meta_via_cluster_inter_route(
        &client,
        pre_writer_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        key_a.as_str(),
        "a-before-crash",
        Some(0),
    )
    .await?;
    let r1 = a_v1.mod_revision;
    if a_v1.create_revision != r1 || a_v1.version != 1 {
        return Err(format!("unexpected old-leader-rejoin a_v1: {:?}", a_v1));
    }

    harness.stop(format!("klog-{}", old_leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let new_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    let new_leader = alive_nodes
        .iter()
        .find(|node| node.id == new_leader_id)
        .ok_or_else(|| format!("new leader node {} not found", new_leader_id))?;
    let failover_writer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or(new_leader);
    let failover_writer_gateway = gateway_addr(failover_writer, ingress_port);

    let after_log = append_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        log_source.as_str(),
        "old leader rejoin write after crash",
    )
    .await?;
    if after_log.id <= before_log.id {
        return Err(format!(
            "log id did not advance after leader crash: before={}, after={}",
            before_log.id, after_log.id
        ));
    }
    wait_log_visible_on_nodes(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        after_log.id,
        log_source.as_str(),
        Duration::from_secs(30),
    )
    .await?;

    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_a.as_str(),
        "a-after-crash",
        Some(r1),
    )
    .await?;
    let r2 = a_v2.mod_revision;
    if a_v2.create_revision != r1 || a_v2.version != 2 || r2 != r1 + 1 {
        return Err(format!("unexpected old-leader-rejoin a_v2: {:?}", a_v2));
    }
    let b_v1 = put_meta_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_b.as_str(),
        "b-after-crash",
        Some(0),
    )
    .await?;
    let r3 = b_v1.mod_revision;
    if b_v1.create_revision != r3 || b_v1.version != 1 || r3 != r2 + 1 {
        return Err(format!("unexpected old-leader-rejoin b_v1: {:?}", b_v1));
    }

    let old_leader_config = configs
        .get(&old_leader_id)
        .ok_or_else(|| format!("missing config for old leader {}", old_leader_id))?;
    spawn_klog(harness, &klog_daemon_bin, old_leader_config, old_leader)?;
    wait_tcp("127.0.0.1", old_leader.ports.admin, Duration::from_secs(12)).await?;
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
    let leader_after_rejoin = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;

    wait_log_visible_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        before_log.id,
        log_source.as_str(),
        Duration::from_secs(45),
    )
    .await?;
    wait_log_visible_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        after_log.id,
        log_source.as_str(),
        Duration::from_secs(45),
    )
    .await?;

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            8,
        )
        .await?;
        require_meta_values(
            &current,
            &[
                (&key_a, "a-after-crash", r1, r2, 2),
                (&key_b, "b-after-crash", r3, r3, 1),
            ],
        )?;
    }

    expect_meta_put_status_via_cluster_inter_route(
        &client,
        old_leader_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        MetaPutRequest {
            key: key_a.clone(),
            value: "stale-old-leader-write".to_string(),
            node_name: Some(old_leader.name.clone()),
            expected_revision: Some(r1),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let a_v3 = put_meta_via_cluster_inter_route(
        &client,
        old_leader_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        key_a.as_str(),
        "a-after-rejoin",
        Some(r2),
    )
    .await?;
    let r4 = a_v3.mod_revision;
    if a_v3.create_revision != r1 || a_v3.version != 3 || r4 != r3 + 1 {
        return Err(format!("unexpected old-leader-rejoin a_v3: {:?}", a_v3));
    }
    wait_meta_value_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        key_a.as_str(),
        "a-after-rejoin",
        r4,
        Duration::from_secs(45),
    )
    .await?;

    let changes = query_meta_changes_via_cluster_inter_route(
        &client,
        old_leader_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        prefix.as_str(),
        r1,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &changes,
        &[
            (r1, &key_a, "a-before-crash", false, r1, 1),
            (r2, &key_a, "a-after-crash", false, r1, 2),
            (r3, &key_b, "b-after-crash", false, r3, 1),
            (r4, &key_a, "a-after-rejoin", false, r1, 3),
        ],
    )?;

    println!(
        "[klog-cluster-dv] raft old leader rejoin ok: old_leader={}, new_leader={}, leader_after_rejoin={}, log_ids=[{},{}], revisions=[{},{},{},{}], prefix={}",
        old_leader_id,
        new_leader_id,
        leader_after_rejoin,
        before_log.id,
        after_log.id,
        r1,
        r2,
        r3,
        r4,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_raft_old_leader_rejoin() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_old_leader_rejoin_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_follower_lag_snapshot_install_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-follower-lag-snapshot-dv";
    let setup = prepare_local_gateway_setup(
        harness,
        RAFT_FOLLOWER_LAG_SNAPSHOT_INSTALL_MODE,
        route_prefix,
        3,
    )
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
    let raft_patch = ood_snapshot_membership_raft_patch();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config_with_raft_patch(harness, node, &voter_config, raft_patch)?;
        configs.insert(node.id, config.clone());
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
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let initial_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let lagged_follower = nodes
        .iter()
        .find(|node| node.id != initial_leader_id && node.id != seed.id)
        .or_else(|| nodes.iter().find(|node| node.id != initial_leader_id))
        .cloned()
        .ok_or_else(|| {
            format!(
                "failed to pick lagged follower: leader={}",
                initial_leader_id
            )
        })?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != lagged_follower.id)
        .cloned()
        .collect::<Vec<_>>();
    let writer = alive_nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing writer node after picking lagged follower".to_string())?;
    let target = alive_nodes
        .iter()
        .find(|node| node.id != writer.id)
        .cloned()
        .unwrap_or_else(|| writer.clone());

    let baseline = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &writer,
        &target,
        &nodes,
        "raft-follower-lag-before-stop",
    )
    .await?;

    harness.stop(format!("klog-{}", lagged_follower.name).as_str())?;
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let active_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;

    let bulk = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &writer,
        &target,
        "raft-follower-lag-snapshot-install",
        DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_ITEMS,
        DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_VALUE_BYTES,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &alive_nodes,
        &bulk,
        Duration::from_secs(60),
    )
    .await?;
    let snapshot_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let snapshot_leader = alive_nodes
        .iter()
        .find(|node| node.id == snapshot_leader_id)
        .ok_or_else(|| format!("snapshot leader node {} not found", snapshot_leader_id))?;
    let leader_snapshot_files =
        wait_snapshot_file_count(harness, snapshot_leader, 1, Duration::from_secs(90)).await?;

    let lagged_config = configs
        .get(&lagged_follower.id)
        .ok_or_else(|| format!("missing config for lagged follower {}", lagged_follower.id))?;
    spawn_klog_with_log_level(
        harness,
        &klog_daemon_bin,
        lagged_config,
        &lagged_follower,
        "info",
    )?;
    wait_tcp(
        "127.0.0.1",
        lagged_follower.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(120),
    )
    .await?;
    wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    let follower_snapshot_files =
        wait_snapshot_file_count(harness, &lagged_follower, 1, Duration::from_secs(120)).await?;

    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &baseline,
        Duration::from_secs(60),
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes,
        &bulk,
        Duration::from_secs(90),
    )
    .await?;

    let recovered_write = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &lagged_follower,
        snapshot_leader,
        &nodes,
        "raft-follower-lag-after-recovery",
    )
    .await?;

    println!(
        "[klog-cluster-dv] raft follower lag snapshot install ok: initial_leader={}, active_leader={}, snapshot_leader={}, lagged_follower={}, leader_snapshots={}, follower_snapshots={}, bulk_items={}, recovered_log_id={}, prefix={}",
        initial_leader_id,
        active_leader_id,
        snapshot_leader_id,
        lagged_follower.id,
        leader_snapshot_files,
        follower_snapshot_files,
        bulk.expected_meta_count,
        recovered_write.log_id,
        bulk.meta_prefix
    );
    Ok(())
}

async fn run_local_gateway_raft_follower_lag_snapshot_install() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_follower_lag_snapshot_install_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_snapshot_install_crash_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let item_count = parse_env_usize(
        ENV_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS,
        DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS,
    )?;
    let value_bytes = parse_env_usize(
        ENV_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES,
        DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES,
    )?;
    let chunk_bytes = parse_env_usize(
        ENV_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES,
        DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES,
    )?;
    if item_count == 0 || value_bytes == 0 || chunk_bytes == 0 {
        return Err(format!(
            "{}={}, {}={}, and {}={} must all be greater than 0",
            ENV_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS,
            item_count,
            ENV_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES,
            value_bytes,
            ENV_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES,
            chunk_bytes
        ));
    }

    let route_prefix = "/.cluster/klog-it-raft-snapshot-install-crash-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_SNAPSHOT_INSTALL_CRASH_MODE, route_prefix, 4)
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
    let learner = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing snapshot install crash learner node".to_string())?;
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing snapshot install crash seed node".to_string())?;
    let raft_patch = raft_snapshot_install_crash_raft_patch(chunk_bytes);
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
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let writer = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing snapshot install crash writer".to_string())?;
    let target = base_voters
        .get(1)
        .cloned()
        .ok_or_else(|| "missing snapshot install crash target".to_string())?;
    let bulk = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &writer,
        &target,
        "raft-snapshot-install-crash-prejoin",
        item_count,
        value_bytes,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &base_voters,
        &bulk,
        Duration::from_secs(80),
    )
    .await?;
    let snapshot_leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let snapshot_leader = base_voters
        .iter()
        .find(|node| node.id == snapshot_leader_id)
        .ok_or_else(|| format!("snapshot leader node {} not found", snapshot_leader_id))?;
    let leader_snapshot_files =
        wait_snapshot_file_count(harness, snapshot_leader, 1, Duration::from_secs(100)).await?;

    let learner_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let learner_config =
        write_klog_config_with_raft_patch(harness, &learner, &learner_options, raft_patch)?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &learner_config, &learner, "info")?;
    wait_tcp("127.0.0.1", learner.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", learner.ports.inter, Duration::from_secs(12)).await?;

    let add_leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let add_leader = base_voters
        .iter()
        .find(|node| node.id == add_leader_id)
        .ok_or_else(|| format!("add-learner leader node {} not found", add_leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(add_leader, ingress_port).as_str(),
        route_prefix,
        add_leader.name.as_str(),
        &learner,
        false,
    )
    .await?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(80),
    )
    .await?;
    let temp_bytes =
        wait_snapshot_temp_file_exists(harness, &learner, Duration::from_secs(120)).await?;
    harness.stop(format!("klog-{}", learner.name).as_str())?;
    let snapshot_files_before_restart = snapshot_file_count(harness, &learner)?;

    spawn_klog_with_log_level(harness, &klog_daemon_bin, &learner_config, &learner, "info")?;
    wait_tcp("127.0.0.1", learner.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", learner.ports.inter, Duration::from_secs(12)).await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(160),
    )
    .await?;
    wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(90),
    )
    .await?;
    let learner_snapshot_files =
        wait_snapshot_file_count(harness, &learner, 1, Duration::from_secs(160)).await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes,
        &bulk,
        Duration::from_secs(120),
    )
    .await?;

    let after_restart = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &learner,
        snapshot_leader,
        &nodes,
        "raft-snapshot-install-crash-after-restart",
    )
    .await?;

    println!(
        "[klog-cluster-dv] raft snapshot install crash ok: add_leader={}, snapshot_leader={}, learner={}, leader_snapshots={}, learner_snapshots_before_restart={}, learner_snapshots_after_restart={}, temp_bytes_before_kill={}, bulk_items={}, value_bytes={}, chunk_bytes={}, recovered_log_id={}, prefix={}",
        add_leader_id,
        snapshot_leader_id,
        learner.id,
        leader_snapshot_files,
        snapshot_files_before_restart,
        learner_snapshot_files,
        temp_bytes,
        bulk.expected_meta_count,
        value_bytes,
        chunk_bytes,
        after_restart.log_id,
        bulk.meta_prefix
    );
    Ok(())
}

async fn run_local_gateway_raft_snapshot_install_crash() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_snapshot_install_crash_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_quorum_loss_recovery_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-quorum-loss-recovery-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_QUORUM_LOSS_RECOVERY_MODE, route_prefix, 3)
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
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
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
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let survivor = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .cloned()
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let stopped_nodes = nodes
        .iter()
        .filter(|node| node.id != leader_id)
        .cloned()
        .collect::<Vec<_>>();
    if stopped_nodes.len() != 2 {
        return Err(format!(
            "expected two followers to stop, got {}",
            stopped_nodes.len()
        ));
    }
    let baseline = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &stopped_nodes[0],
        &survivor,
        &nodes,
        "raft-quorum-loss-before-loss",
    )
    .await?;

    for node in &stopped_nodes {
        harness.stop(format!("klog-{}", node.name).as_str())?;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;

    let survivor_gateway = gateway_addr(&survivor, ingress_port);
    if append_via_cluster_inter_route(
        &client,
        survivor_gateway.as_str(),
        route_prefix,
        survivor.name.as_str(),
        "test/raft-quorum-loss/unavailable-log",
        "write should fail without quorum",
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "single survivor {} unexpectedly accepted append without quorum",
            survivor.id
        ));
    }

    if query_via_cluster_inter_route(
        &client,
        survivor_gateway.as_str(),
        route_prefix,
        survivor.name.as_str(),
        baseline.log_id,
        baseline.log_source.as_str(),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "single survivor {} unexpectedly served strong read without quorum",
            survivor.id
        ));
    }

    let suffix = unique_suffix("raft-quorum-loss-recovery");
    let prefix = format!("test/klog_raft_quorum_loss_recovery_dv/{}/", suffix);
    let doomed_key = format!("{}doomed", prefix);
    if put_meta_via_cluster_inter_route(
        &client,
        survivor_gateway.as_str(),
        route_prefix,
        survivor.name.as_str(),
        doomed_key.as_str(),
        "should-not-commit-without-quorum",
        Some(0),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "single survivor {} unexpectedly accepted meta put without quorum",
            survivor.id
        ));
    }

    let first_restored = stopped_nodes[0].clone();
    let first_restored_config = configs
        .get(&first_restored.id)
        .ok_or_else(|| format!("missing config for restored node {}", first_restored.id))?;
    spawn_klog(
        harness,
        &klog_daemon_bin,
        first_restored_config,
        &first_restored,
    )?;
    wait_tcp(
        "127.0.0.1",
        first_restored.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp(
        "127.0.0.1",
        first_restored.ports.inter,
        Duration::from_secs(12),
    )
    .await?;
    let quorum_nodes = vec![survivor.clone(), first_restored.clone()];
    wait_membership(
        &client,
        &quorum_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let quorum_leader_id = wait_consistent_leader(
        &client,
        &quorum_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    let quorum_leader = quorum_nodes
        .iter()
        .find(|node| node.id == quorum_leader_id)
        .cloned()
        .ok_or_else(|| format!("quorum leader node {} not found", quorum_leader_id))?;
    let quorum_writer = quorum_nodes
        .iter()
        .find(|node| node.id != quorum_leader_id)
        .cloned()
        .unwrap_or_else(|| quorum_leader.clone());
    tokio::time::sleep(Duration::from_secs(3)).await;

    let pre_recovery_query = query_meta_via_cluster_inter_route(
        &client,
        gateway_addr(&quorum_writer, ingress_port).as_str(),
        route_prefix,
        quorum_leader.name.as_str(),
        doomed_key.as_str(),
    )
    .await?;
    if !pre_recovery_query.items.is_empty() {
        return Err(format!(
            "no-quorum meta write was applied after quorum recovery: key={}, items={:?}",
            doomed_key, pre_recovery_query.items
        ));
    }

    let recovered_meta = put_meta_via_cluster_inter_route(
        &client,
        gateway_addr(&quorum_writer, ingress_port).as_str(),
        route_prefix,
        quorum_leader.name.as_str(),
        doomed_key.as_str(),
        "committed-after-quorum-recovery",
        Some(0),
    )
    .await?;
    if recovered_meta.create_revision != recovered_meta.mod_revision || recovered_meta.version != 1
    {
        return Err(format!(
            "unexpected recovered quorum meta version: {:?}",
            recovered_meta
        ));
    }
    wait_meta_value_on_nodes(
        &client,
        &quorum_nodes,
        ingress_port,
        route_prefix,
        doomed_key.as_str(),
        "committed-after-quorum-recovery",
        recovered_meta.mod_revision,
        Duration::from_secs(40),
    )
    .await?;

    verify_log_and_meta_witness_on_nodes(
        &client,
        &quorum_nodes,
        ingress_port,
        route_prefix,
        &baseline,
        Duration::from_secs(40),
    )
    .await?;
    let after_quorum = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &quorum_writer,
        &quorum_leader,
        &quorum_nodes,
        "raft-quorum-loss-after-one-restore",
    )
    .await?;

    let second_restored = stopped_nodes[1].clone();
    let second_restored_config = configs
        .get(&second_restored.id)
        .ok_or_else(|| format!("missing config for restored node {}", second_restored.id))?;
    spawn_klog(
        harness,
        &klog_daemon_bin,
        second_restored_config,
        &second_restored,
    )?;
    wait_tcp(
        "127.0.0.1",
        second_restored.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp(
        "127.0.0.1",
        second_restored.ports.inter,
        Duration::from_secs(12),
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(100),
    )
    .await?;
    let final_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    let final_leader = nodes
        .iter()
        .find(|node| node.id == final_leader_id)
        .cloned()
        .ok_or_else(|| format!("final leader node {} not found", final_leader_id))?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &baseline,
        Duration::from_secs(50),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &after_quorum,
        Duration::from_secs(50),
    )
    .await?;
    wait_meta_value_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        doomed_key.as_str(),
        "committed-after-quorum-recovery",
        recovered_meta.mod_revision,
        Duration::from_secs(50),
    )
    .await?;
    let final_write = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &second_restored,
        &final_leader,
        &nodes,
        "raft-quorum-loss-after-all-recovered",
    )
    .await?;

    println!(
        "[klog-cluster-dv] raft quorum loss recovery ok: survivor={}, stopped=[{},{}], quorum_leader={}, final_leader={}, baseline_log_id={}, recovered_log_id={}, final_log_id={}, recovered_revision={}, prefix={}",
        survivor.id,
        stopped_nodes[0].id,
        stopped_nodes[1].id,
        quorum_leader_id,
        final_leader_id,
        baseline.log_id,
        after_quorum.log_id,
        final_write.log_id,
        recovered_meta.mod_revision,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_raft_quorum_loss_recovery() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_quorum_loss_recovery_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_membership_change_rejoin_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-membership-change-rejoin-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_MEMBERSHIP_CHANGE_REJOIN_MODE, route_prefix, 3)
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
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
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
    let initial_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let removed_node = nodes
        .iter()
        .find(|node| node.id != initial_leader_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "failed to choose non-leader voter, leader={}",
                initial_leader_id
            )
        })?;
    let active_nodes = nodes
        .iter()
        .filter(|node| node.id != removed_node.id)
        .cloned()
        .collect::<Vec<_>>();
    if active_nodes.len() != 2 {
        return Err(format!(
            "expected two active voters after picking removed node, got {}",
            active_nodes.len()
        ));
    }
    let active_voters = active_nodes.iter().map(|node| node.id).collect::<Vec<_>>();

    let before_remove = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &active_nodes[0],
        &removed_node,
        &nodes,
        "raft-membership-rejoin-before-remove",
    )
    .await?;

    harness.stop(format!("klog-{}", removed_node.name).as_str())?;
    let shrink_leader_id = change_voters_via_current_leader(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        false,
    )
    .await?;
    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let active_leader_id = wait_consistent_leader(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let active_leader = active_nodes
        .iter()
        .find(|node| node.id == active_leader_id)
        .cloned()
        .ok_or_else(|| format!("active leader node {} not found", active_leader_id))?;
    let active_writer = active_nodes
        .iter()
        .find(|node| node.id != active_leader_id)
        .cloned()
        .unwrap_or_else(|| active_leader.clone());
    verify_log_and_meta_witness_on_nodes(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        &before_remove,
        Duration::from_secs(40),
    )
    .await?;
    let after_shrink = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &active_writer,
        &active_leader,
        &active_nodes,
        "raft-membership-rejoin-after-shrink",
    )
    .await?;

    let removed_config = configs
        .get(&removed_node.id)
        .ok_or_else(|| format!("missing config for removed node {}", removed_node.id))?;
    spawn_klog(harness, &klog_daemon_bin, removed_config, &removed_node)?;
    wait_tcp(
        "127.0.0.1",
        removed_node.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp(
        "127.0.0.1",
        removed_node.ports.inter,
        Duration::from_secs(12),
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let (stale_status, stale_body) = post_change_membership_via_admin_route(
        &client,
        gateway_addr(&removed_node, ingress_port).as_str(),
        route_prefix,
        removed_node.name.as_str(),
        &[1, 2, 3],
        true,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let after_stale_restart = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &active_writer,
        &active_leader,
        &active_nodes,
        "raft-membership-rejoin-after-stale-restart",
    )
    .await?;

    post_add_learner_via_admin_route(
        &client,
        gateway_addr(&active_leader, ingress_port).as_str(),
        route_prefix,
        active_leader.name.as_str(),
        &removed_node,
        true,
    )
    .await?;
    let learner_ids = [removed_node.id];
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &learner_ids,
        Duration::from_secs(90),
    )
    .await?;
    for witness in [&before_remove, &after_shrink, &after_stale_restart] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(60),
        )
        .await?;
    }

    let mut promoted_voters = active_voters.clone();
    promoted_voters.push(removed_node.id);
    promoted_voters.sort_unstable();
    let promote_leader_id = change_voters_via_current_leader(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        promoted_voters.as_slice(),
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        promoted_voters.as_slice(),
        &[],
        Duration::from_secs(90),
    )
    .await?;
    let final_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(70),
    )
    .await?;
    let final_leader = nodes
        .iter()
        .find(|node| node.id == final_leader_id)
        .cloned()
        .ok_or_else(|| format!("final leader node {} not found", final_leader_id))?;
    let final_write = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &removed_node,
        &final_leader,
        &nodes,
        "raft-membership-rejoin-after-promote",
    )
    .await?;
    for witness in [
        &before_remove,
        &after_shrink,
        &after_stale_restart,
        &final_write,
    ] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(60),
        )
        .await?;
    }

    println!(
        "[klog-cluster-dv] raft membership change rejoin ok: initial_leader={}, removed_node={}, shrink_leader={}, active_leader={}, stale_admin_status={}, stale_admin_body_len={}, promote_leader={}, final_leader={}, active_voters={:?}, promoted_voters={:?}, log_ids=[{},{},{},{}]",
        initial_leader_id,
        removed_node.id,
        shrink_leader_id,
        active_leader_id,
        stale_status,
        stale_body.len(),
        promote_leader_id,
        final_leader_id,
        active_voters,
        promoted_voters,
        before_remove.log_id,
        after_shrink.log_id,
        after_stale_restart.log_id,
        final_write.log_id
    );
    Ok(())
}

async fn run_local_gateway_raft_membership_change_rejoin() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_membership_change_rejoin_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_node_id_reuse_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-node-id-reuse-dv";
    let setup = prepare_local_gateway_setup(harness, NODE_ID_REUSE_MODE, route_prefix, 3).await?;
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
        .ok_or_else(|| "missing node-id reuse seed node".to_string())?;
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
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
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
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader_node = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .cloned()
        .ok_or_else(|| format!("node-id reuse leader {} not found", leader_id))?;
    let reused_node = nodes
        .iter()
        .find(|node| node.id != seed.id)
        .cloned()
        .ok_or_else(|| "missing non-seed node for node-id reuse".to_string())?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let replacement = LocalNodeDef {
        id: reused_node.id,
        name: format!("{}-replacement", reused_node.name),
        device_id: format!("did:dv:{}-replacement", reused_node.name),
        gateway_host: "127.0.0.4".to_string(),
        ports: LocalNodePorts {
            raft: pick_local_port(&mut used_ports)?,
            inter: pick_local_port(&mut used_ports)?,
            admin: pick_local_port(&mut used_ports)?,
            rpc: pick_local_port(&mut used_ports)?,
            rtcp: pick_local_port(&mut used_ports)?,
            zone_http: pick_local_port(&mut used_ports)?,
            control: pick_local_port(&mut used_ports)?,
        },
    };

    let (duplicate_status, duplicate_body) = post_add_learner_via_admin_route_status(
        &client,
        gateway_addr(&leader_node, ingress_port).as_str(),
        route_prefix,
        leader_node.name.as_str(),
        &replacement,
        true,
    )
    .await?;
    if duplicate_status.is_success() {
        return Err(format!(
            "duplicate add-learner unexpectedly succeeded: reused_node_id={}, replacement={:?}, body={}",
            replacement.id, replacement, duplicate_body
        ));
    }
    require_node_id_reuse_error(
        format!(
            "duplicate add-learner node_id={} status={} body={}",
            replacement.id, duplicate_status, duplicate_body
        )
        .as_str(),
        "duplicate admin add-learner",
    )?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(30),
    )
    .await?;

    let join_targets = vec![format!("127.0.0.1:{}", leader_node.ports.admin)];
    let replacement_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "voter",
    };
    let replacement_retry = KLogJoinRetryPatch {
        initial_interval_ms: 200,
        max_interval_ms: 200,
        max_attempts: 1,
        request_timeout_ms: 1000,
    };
    let replacement_config = write_klog_config_with_join_targets_and_retry_patch(
        harness,
        &replacement,
        &replacement_options,
        join_targets.as_slice(),
        replacement_retry,
        KLogRaftPatch::default(),
    )?;
    spawn_klog_with_log_level(
        harness,
        &klog_daemon_bin,
        &replacement_config,
        &replacement,
        "info",
    )?;
    wait_tcp(
        "127.0.0.1",
        replacement.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp("127.0.0.1", replacement.ports.rpc, Duration::from_secs(12)).await?;
    let node_id_pattern = format!("node_id={}", replacement.id);
    let replacement_name_pattern = format!("expected={} remote=", replacement.name);
    let replacement_device_pattern = format!("expected={} remote=", replacement.device_id);
    let join_log = wait_klog_out_log_contains(
        harness,
        &replacement,
        &[
            "Auto-join reached max attempts without success",
            "node identity mismatch",
            node_id_pattern.as_str(),
            "node_name",
            replacement_name_pattern.as_str(),
            "device_id",
            replacement_device_pattern.as_str(),
        ],
        Duration::from_secs(12),
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let witness = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &leader_node,
        &reused_node,
        &nodes,
        "node-id-reuse-after-rejected-replacement",
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &witness,
        Duration::from_secs(40),
    )
    .await?;

    println!(
        "[klog-cluster-dv] node id reuse ok: leader={}, reused_node={}, replacement_name={}, duplicate_status={}, duplicate_body_len={}, join_log_len={}, log_id={}, meta_key={}",
        leader_id,
        reused_node.id,
        replacement.name,
        duplicate_status,
        duplicate_body.len(),
        join_log.len(),
        witness.log_id,
        witness.meta_key
    );
    Ok(())
}

async fn run_local_gateway_node_id_reuse() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_node_id_reuse_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_concurrent_membership_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-concurrent-membership-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_CONCURRENT_MEMBERSHIP_MODE, route_prefix, 5)
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
    let candidate_a = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing concurrent membership candidate A".to_string())?;
    let candidate_b = nodes
        .get(4)
        .cloned()
        .ok_or_else(|| "missing concurrent membership candidate B".to_string())?;
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing concurrent membership seed node".to_string())?;
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

    let candidate_config_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    for candidate in [&candidate_a, &candidate_b] {
        let config = write_klog_config(harness, candidate, &candidate_config_options)?;
        spawn_klog(harness, &klog_daemon_bin, &config, candidate)?;
        wait_tcp("127.0.0.1", candidate.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", candidate.ports.inter, Duration::from_secs(12)).await?;
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
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let before_concurrent = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[0],
        &base_voters[1],
        &base_voters,
        "raft-concurrent-membership-before",
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
        .cloned()
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let leader_gateway = gateway_addr(&leader, ingress_port);
    let add_a = post_add_learner_via_admin_route_status(
        &client,
        leader_gateway.as_str(),
        route_prefix,
        leader.name.as_str(),
        &candidate_a,
        true,
    );
    let add_b = post_add_learner_via_admin_route_status(
        &client,
        leader_gateway.as_str(),
        route_prefix,
        leader.name.as_str(),
        &candidate_b,
        true,
    );
    let ((status_a, body_a), (status_b, body_b)) = tokio::try_join!(add_a, add_b)?;
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for (candidate, status, body) in [
        (candidate_a.clone(), status_a, body_a),
        (candidate_b.clone(), status_b, body_b),
    ] {
        if status.is_success() {
            successes.push((candidate, body));
        } else {
            failures.push((candidate, status, body));
        }
    }
    if successes.len() != 1 || failures.len() != 1 {
        return Err(format!(
            "expected exactly one concurrent add-learner success and one failure, successes={}, failures={}",
            successes.len(),
            failures.len()
        ));
    }
    let (added_ood, add_success_body) = successes
        .pop()
        .ok_or_else(|| "missing successful concurrent add-learner result".to_string())?;
    let (rejected_ood, rejected_status, rejected_body) = failures
        .pop()
        .ok_or_else(|| "missing rejected concurrent add-learner result".to_string())?;
    if rejected_status != StatusCode::CONFLICT {
        return Err(format!(
            "concurrent add-learner for {} expected 409 Conflict, got status={}, body={}",
            rejected_ood.name, rejected_status, rejected_body
        ));
    }
    if !rejected_body.contains("membership change already in progress")
        && !rejected_body.contains("undergoing a configuration change")
    {
        return Err(format!(
            "concurrent add-learner rejection body missing conflict marker: node={}, body={}",
            rejected_ood.name, rejected_body
        ));
    }

    let mut member_nodes = base_voters.clone();
    member_nodes.push(added_ood.clone());
    let learner_ids = [added_ood.id];
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &learner_ids,
        Duration::from_secs(80),
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &learner_ids,
        Duration::from_secs(20),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &before_concurrent,
        Duration::from_secs(50),
    )
    .await?;

    let mut promoted_voters = vec![1, 2, 3, added_ood.id];
    promoted_voters.sort_unstable();
    let promote_leader_id = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        promoted_voters.as_slice(),
        true,
    )
    .await?;
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        promoted_voters.as_slice(),
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let after_promote = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &base_voters[0],
        &member_nodes,
        "raft-concurrent-membership-after-promote",
    )
    .await?;
    for witness in [&before_concurrent, &after_promote] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &member_nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(50),
        )
        .await?;
    }

    println!(
        "[klog-cluster-dv] raft concurrent membership ok: leader={}, added={}, rejected={}, rejected_status={}, promote_leader={}, voters={:?}, success_body_len={}, rejected_body_len={}, log_ids=[{},{}]",
        leader_id,
        added_ood.id,
        rejected_ood.id,
        rejected_status,
        promote_leader_id,
        promoted_voters,
        add_success_body.len(),
        rejected_body.len(),
        before_concurrent.log_id,
        after_promote.log_id
    );
    Ok(())
}

async fn run_local_gateway_raft_concurrent_membership() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_concurrent_membership_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_join_retry_idempotency_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-join-retry-idempotency-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_JOIN_RETRY_IDEMPOTENCY_MODE, route_prefix, 4)
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
    let added_ood = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing join retry learner node".to_string())?;
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing join retry seed node".to_string())?;
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
        .timeout(Duration::from_secs(8))
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

    let pre_join_witness = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[0],
        &base_voters[1],
        "join-retry-idempotency-prejoin",
        220,
        1024,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &base_voters,
        &pre_join_witness,
        Duration::from_secs(50),
    )
    .await?;

    let join_targets = base_voters
        .iter()
        .map(|target| gateway_admin_join_target(&added_ood, ingress_port, route_prefix, target))
        .collect::<Vec<_>>();
    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let retry_patch = KLogJoinRetryPatch {
        initial_interval_ms: 100,
        max_interval_ms: 100,
        max_attempts: 80,
        request_timeout_ms: 20,
    };
    let added_config = write_klog_config_with_join_targets_and_retry_patch(
        harness,
        &added_ood,
        &added_options,
        &join_targets,
        retry_patch,
        raft_patch,
    )?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &added_config, &added_ood, "info")?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let member_nodes = base_voters
        .iter()
        .cloned()
        .chain(std::iter::once(added_ood.clone()))
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(120),
    )
    .await?;
    let join_log = wait_klog_out_log_contains(
        harness,
        &added_ood,
        &[
            "add-learner request send failed",
            "Auto-join skip add-learner because node is already learner",
            "Auto-join succeeded",
        ],
        Duration::from_secs(20),
    )
    .await?;
    if join_log.contains("Auto-join promote learner to voter") {
        return Err("auto-join unexpectedly promoted learner target role to voter".to_string());
    }

    tokio::time::sleep(Duration::from_secs(3)).await;
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(30),
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &member_nodes,
        &pre_join_witness,
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader_id = change_voters_via_current_leader(
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
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let after_promote = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &base_voters[0],
        &member_nodes,
        "raft-join-retry-idempotency-after-promote",
    )
    .await?;

    println!(
        "[klog-cluster-dv] raft join retry idempotency ok: added={}, promote_leader={}, prejoin_meta_count={}, post_promote_log_id={}, timeout_ms={}, join_log_len={}",
        added_ood.id,
        promote_leader_id,
        pre_join_witness.expected_meta_count,
        after_promote.log_id,
        retry_patch.request_timeout_ms,
        join_log.len()
    );
    Ok(())
}

async fn run_local_gateway_raft_join_retry_idempotency() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_join_retry_idempotency_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

