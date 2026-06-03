fn mvcc_snapshot_key(prefix: &str, index: usize) -> String {
    format!("{}key-{:04}", prefix, index)
}

fn mvcc_compact_snapshot_value(phase: &str, index: usize, value_bytes: usize) -> String {
    let label = format!("mvcc-compact-during-snapshot-{}", phase);
    fixed_payload(label.as_str(), index, value_bytes)
}

#[allow(clippy::too_many_arguments)]
async fn require_mvcc_snapshot_current_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    prefix: &str,
    expected_count: usize,
    expected_values: &[(&str, &str, u64, u64, u64)],
) -> Result<(), String> {
    for node in nodes {
        let response = query_meta_prefix_via_cluster_inter_route(
            client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix,
            expected_count + 8,
        )
        .await?;
        if response.items.len() != expected_count {
            return Err(format!(
                "unexpected MVCC snapshot current count on {}: expected={}, actual={}, items={:?}",
                node.name,
                expected_count,
                response.items.len(),
                response.items
            ));
        }
        require_meta_selected_values(&response, expected_values)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn require_meta_at_revision_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    key: &str,
    revision: u64,
    expected_value: &str,
    expected_create_revision: u64,
    expected_version: u64,
) -> Result<(), String> {
    for node in nodes {
        let response = query_meta_at_revision_via_cluster_inter_route(
            client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            key,
            Some(revision),
        )
        .await?;
        require_meta_selected_values(
            &response,
            &[(
                key,
                expected_value,
                expected_create_revision,
                revision,
                expected_version,
            )],
        )?;
    }
    Ok(())
}

async fn require_mvcc_snapshot_key_absent_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    key: &str,
) -> Result<(), String> {
    for node in nodes {
        let response = query_meta_via_cluster_inter_route(
            client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            key,
        )
        .await?;
        if !response.items.is_empty() {
            return Err(format!(
                "deleted MVCC snapshot key visible on {}: key={}, items={:?}",
                node.name, key, response.items
            ));
        }
    }
    Ok(())
}

async fn run_local_gateway_mvcc_snapshot_membership_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let item_count = parse_env_usize(
        ENV_MVCC_SNAPSHOT_MEMBERSHIP_KEYS,
        DEFAULT_MVCC_SNAPSHOT_MEMBERSHIP_KEYS,
    )?;
    if item_count < 30 {
        return Err(format!(
            "{} must be at least 30 for MVCC snapshot membership coverage, got {}",
            ENV_MVCC_SNAPSHOT_MEMBERSHIP_KEYS, item_count
        ));
    }

    let route_prefix = "/.cluster/klog-it-mvcc-snapshot-membership-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_SNAPSHOT_MEMBERSHIP_MODE, route_prefix, 4)
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
        .ok_or_else(|| "missing MVCC snapshot seed node".to_string())?;
    let target = base_voters
        .get(1)
        .cloned()
        .ok_or_else(|| "missing MVCC snapshot target voter".to_string())?;
    let added_ood = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing MVCC snapshot added OOD".to_string())?;
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
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("MVCC snapshot leader {} not found", leader_id))?;
    let leader_gateway_addr = gateway_addr(leader, ingress_port);
    let seed_gateway_addr = gateway_addr(&seed, ingress_port);

    let run_id = unique_suffix("mvcc-snapshot-membership");
    let prefix = format!("test/klog_mvcc_snapshot_membership/{}/", run_id);
    let key0 = mvcc_snapshot_key(&prefix, 0);
    let key1 = mvcc_snapshot_key(&prefix, 1);
    let key2 = mvcc_snapshot_key(&prefix, 2);
    let key3 = mvcc_snapshot_key(&prefix, 3);
    let key4 = mvcc_snapshot_key(&prefix, 4);
    let key5 = mvcc_snapshot_key(&prefix, 5);
    let key10 = mvcc_snapshot_key(&prefix, 10);
    let key25 = mvcc_snapshot_key(&prefix, 25);
    let key_last = mvcc_snapshot_key(&prefix, item_count - 1);
    let key_last_value = format!("v1-{:04}", item_count - 1);

    let mut create_revisions = Vec::with_capacity(item_count);
    for index in 0..item_count {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = format!("v1-{index:04}");
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected initial MVCC put response: {:?}",
                stored
            ));
        }
        create_revisions.push(stored.mod_revision);
    }

    let mut update_revisions = BTreeMap::new();
    let update_count = (item_count / 3).max(12);
    for (index, create_revision) in create_revisions.iter().enumerate().take(update_count) {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = format!("v2-{index:04}");
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(*create_revision),
        )
        .await?;
        if stored.create_revision != *create_revision || stored.version != 2 {
            return Err(format!("unexpected MVCC update response: {:?}", stored));
        }
        update_revisions.insert(index, stored.mod_revision);
    }

    let delete_count = 10usize;
    let mut delete_revisions = BTreeMap::new();
    for (index, create_revision) in create_revisions.iter().enumerate().take(delete_count) {
        let key = mvcc_snapshot_key(&prefix, index);
        let deleted = delete_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
        )
        .await?;
        let version = deleted
            .meta_version
            .as_ref()
            .ok_or_else(|| format!("missing delete meta_version: {:?}", deleted))?;
        if !version.deleted || version.version != 0 || version.create_revision != *create_revision {
            return Err(format!("unexpected MVCC delete response: {:?}", deleted));
        }
        delete_revisions.insert(index, version.mod_revision);
    }
    let compact_revision = *delete_revisions
        .get(&(delete_count - 1))
        .ok_or_else(|| "missing compact revision".to_string())?;

    let mut recreate_revisions = BTreeMap::new();
    for index in 0..5usize {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = format!("v3-{index:04}");
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!("unexpected MVCC recreate response: {:?}", stored));
        }
        recreate_revisions.insert(index, stored.mod_revision);
    }

    let current_revision = *recreate_revisions
        .get(&4)
        .ok_or_else(|| "missing recreated revision for key4".to_string())?;
    let compacted = post_meta_compact_via_admin_route(
        &client,
        leader_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        compact_revision,
    )
    .await?;
    if compacted.compacted_revision != compact_revision
        || compacted.current_revision < current_revision
    {
        return Err(format!(
            "unexpected MVCC snapshot compaction response: {:?}, expected_compacted={}, current>={}",
            compacted, compact_revision, current_revision
        ));
    }

    let leader_snapshot_count =
        wait_snapshot_file_count(harness, leader, 1, Duration::from_secs(80)).await?;
    let current_expected_count = item_count - 5;
    let key10_update_revision = *update_revisions
        .get(&10)
        .ok_or_else(|| "missing key10 update revision".to_string())?;

    require_mvcc_snapshot_current_on_nodes(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count,
        &[
            (
                key0.as_str(),
                "v3-0000",
                *recreate_revisions.get(&0).unwrap(),
                *recreate_revisions.get(&0).unwrap(),
                1,
            ),
            (
                key10.as_str(),
                "v2-0010",
                create_revisions[10],
                key10_update_revision,
                2,
            ),
            (
                key25.as_str(),
                "v1-0025",
                create_revisions[25],
                create_revisions[25],
                1,
            ),
            (
                key_last.as_str(),
                key_last_value.as_str(),
                create_revisions[item_count - 1],
                create_revisions[item_count - 1],
                1,
            ),
        ],
    )
    .await?;
    require_mvcc_snapshot_key_absent_on_nodes(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        key5.as_str(),
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
    let added_config =
        write_klog_config_with_raft_patch(harness, &added_ood, &added_options, raft_patch)?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &added_config, &added_ood, "info")?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| {
            format!(
                "leader node {} not found before MVCC snapshot add",
                leader_id
            )
        })?;
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
        &[1, 2, 3],
        &[4],
        Duration::from_secs(90),
    )
    .await?;
    let added_snapshot_count =
        wait_snapshot_file_count(harness, &added_ood, 1, Duration::from_secs(90)).await?;

    require_mvcc_snapshot_current_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count,
        &[
            (
                key0.as_str(),
                "v3-0000",
                *recreate_revisions.get(&0).unwrap(),
                *recreate_revisions.get(&0).unwrap(),
                1,
            ),
            (
                key10.as_str(),
                "v2-0010",
                create_revisions[10],
                key10_update_revision,
                2,
            ),
            (
                key25.as_str(),
                "v1-0025",
                create_revisions[25],
                create_revisions[25],
                1,
            ),
        ],
    )
    .await?;
    require_mvcc_snapshot_key_absent_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        key5.as_str(),
    )
    .await?;
    expect_meta_query_status_via_cluster_inter_route(
        &client,
        gateway_addr(&added_ood, ingress_port).as_str(),
        route_prefix,
        added_ood.name.as_str(),
        Some(key10.as_str()),
        None,
        Some(create_revisions[0]),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    let added_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        gateway_addr(&added_ood, ingress_port).as_str(),
        route_prefix,
        added_ood.name.as_str(),
        prefix.as_str(),
        compact_revision + 1,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &added_changes,
        &[
            (
                *recreate_revisions.get(&0).unwrap(),
                key0.as_str(),
                "v3-0000",
                false,
                *recreate_revisions.get(&0).unwrap(),
                1,
            ),
            (
                *recreate_revisions.get(&1).unwrap(),
                key1.as_str(),
                "v3-0001",
                false,
                *recreate_revisions.get(&1).unwrap(),
                1,
            ),
            (
                *recreate_revisions.get(&2).unwrap(),
                key2.as_str(),
                "v3-0002",
                false,
                *recreate_revisions.get(&2).unwrap(),
                1,
            ),
            (
                *recreate_revisions.get(&3).unwrap(),
                key3.as_str(),
                "v3-0003",
                false,
                *recreate_revisions.get(&3).unwrap(),
                1,
            ),
            (
                *recreate_revisions.get(&4).unwrap(),
                key4.as_str(),
                "v3-0004",
                false,
                *recreate_revisions.get(&4).unwrap(),
                1,
            ),
        ],
    )?;

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
        Duration::from_secs(90),
    )
    .await?;

    let post_promote_key = format!("{}post-promote", prefix);
    let post_promote_tx = exec_meta_tx_via_cluster_inter_route(
        &client,
        gateway_addr(&added_ood, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        BTreeMap::from([
            (
                key10.clone(),
                meta_tx_put_action(
                    key10.as_str(),
                    "v3-0010",
                    added_ood.name.as_str(),
                    Some(key10_update_revision),
                ),
            ),
            (
                post_promote_key.clone(),
                meta_tx_put_action(
                    post_promote_key.as_str(),
                    "post-promote-value",
                    added_ood.name.as_str(),
                    Some(0),
                ),
            ),
        ]),
    )
    .await?;
    let post_promote_revision = post_promote_tx
        .revisions
        .get(&key10)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing post-promote revision for {}", key10))?;
    if post_promote_tx
        .revisions
        .get(&post_promote_key)
        .and_then(|revision| *revision)
        != Some(post_promote_revision)
    {
        return Err(format!(
            "post-promote MVCC tx did not share revision: {:?}",
            post_promote_tx
        ));
    }
    require_mvcc_snapshot_current_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count + 1,
        &[
            (
                key10.as_str(),
                "v3-0010",
                create_revisions[10],
                post_promote_revision,
                3,
            ),
            (
                post_promote_key.as_str(),
                "post-promote-value",
                post_promote_revision,
                post_promote_revision,
                1,
            ),
        ],
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
        Duration::from_secs(90),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(60),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| {
            format!(
                "leader node {} not found before MVCC snapshot remove",
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
            "remove MVCC snapshot-added OOD learner returned status={}, body={}",
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
        Duration::from_secs(80),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;

    require_mvcc_snapshot_current_on_nodes(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count + 1,
        &[
            (
                key10.as_str(),
                "v3-0010",
                create_revisions[10],
                post_promote_revision,
                3,
            ),
            (
                post_promote_key.as_str(),
                "post-promote-value",
                post_promote_revision,
                post_promote_revision,
                1,
            ),
        ],
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        seed_gateway_addr.as_str(),
        route_prefix,
        seed.name.as_str(),
        prefix.as_str(),
        create_revisions[0],
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    println!(
        "[klog-cluster-dv] MVCC snapshot membership ok: items={}, current_count={}, leader_snapshot_count={}, added_snapshot_count={}, promote_leader={}, demote_leader={}, removed_ood={}, compacted={}, post_promote_revision={}",
        item_count,
        current_expected_count + 1,
        leader_snapshot_count,
        added_snapshot_count,
        promote_leader,
        demote_leader,
        added_ood.name,
        compact_revision,
        post_promote_revision
    );
    Ok(())
}

async fn run_local_gateway_mvcc_snapshot_membership() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_snapshot_membership_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_compact_during_snapshot_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let item_count = parse_env_usize(
        ENV_MVCC_COMPACT_DURING_SNAPSHOT_KEYS,
        DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_KEYS,
    )?;
    let value_bytes = parse_env_usize(
        ENV_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES,
        DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES,
    )?;
    let chunk_bytes = parse_env_usize(
        ENV_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES,
        DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES,
    )?;
    if item_count < 30 {
        return Err(format!(
            "{} must be at least 30 for compact-during-snapshot coverage, got {}",
            ENV_MVCC_COMPACT_DURING_SNAPSHOT_KEYS, item_count
        ));
    }
    if value_bytes == 0 || chunk_bytes == 0 {
        return Err(format!(
            "{}={} and {}={} must both be greater than 0",
            ENV_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES,
            value_bytes,
            ENV_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES,
            chunk_bytes
        ));
    }

    let route_prefix = "/.cluster/klog-it-mvcc-compact-during-snapshot-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_COMPACT_DURING_SNAPSHOT_MODE, route_prefix, 4)
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
        .ok_or_else(|| "missing MVCC compact snapshot seed node".to_string())?;
    let target = base_voters
        .get(1)
        .cloned()
        .ok_or_else(|| "missing MVCC compact snapshot target node".to_string())?;
    let learner = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing MVCC compact snapshot learner node".to_string())?;
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
    let seed_gateway_addr = gateway_addr(&seed, ingress_port);
    let run_id = unique_suffix("mvcc-compact-during-snapshot");
    let prefix = format!("test/klog_mvcc_compact_during_snapshot/{}/", run_id);
    let key0 = mvcc_snapshot_key(&prefix, 0);
    let key1 = mvcc_snapshot_key(&prefix, 1);
    let key2 = mvcc_snapshot_key(&prefix, 2);
    let key3 = mvcc_snapshot_key(&prefix, 3);
    let key4 = mvcc_snapshot_key(&prefix, 4);
    let key5 = mvcc_snapshot_key(&prefix, 5);
    let key10 = mvcc_snapshot_key(&prefix, 10);
    let key_last = mvcc_snapshot_key(&prefix, item_count - 1);

    let mut create_revisions = Vec::with_capacity(item_count);
    for index in 0..item_count {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = mvcc_compact_snapshot_value("v1", index, value_bytes);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected compact snapshot initial put response: {:?}",
                stored
            ));
        }
        create_revisions.push(stored.mod_revision);
    }

    let update_count = (item_count / 3).max(12);
    let mut update_revisions = BTreeMap::new();
    for (index, create_revision) in create_revisions.iter().enumerate().take(update_count) {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = mvcc_compact_snapshot_value("v2", index, value_bytes);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(*create_revision),
        )
        .await?;
        if stored.create_revision != *create_revision || stored.version != 2 {
            return Err(format!(
                "unexpected compact snapshot update response: {:?}",
                stored
            ));
        }
        update_revisions.insert(index, stored.mod_revision);
    }

    let delete_count = 10usize;
    let mut delete_revisions = BTreeMap::new();
    for (index, create_revision) in create_revisions.iter().enumerate().take(delete_count) {
        let key = mvcc_snapshot_key(&prefix, index);
        let deleted = delete_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
        )
        .await?;
        let version = deleted
            .meta_version
            .as_ref()
            .ok_or_else(|| format!("missing compact snapshot delete version: {:?}", deleted))?;
        if !version.deleted || version.version != 0 || version.create_revision != *create_revision {
            return Err(format!(
                "unexpected compact snapshot delete response: {:?}",
                deleted
            ));
        }
        delete_revisions.insert(index, version.mod_revision);
    }
    let compact_revision = *delete_revisions
        .get(&(delete_count - 1))
        .ok_or_else(|| "missing compact snapshot compact revision".to_string())?;

    let mut recreate_revisions = BTreeMap::new();
    for index in 0..5usize {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = mvcc_compact_snapshot_value("v3", index, value_bytes);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected compact snapshot recreate response: {:?}",
                stored
            ));
        }
        recreate_revisions.insert(index, stored.mod_revision);
    }
    let current_revision = *recreate_revisions
        .get(&4)
        .ok_or_else(|| "missing compact snapshot current revision".to_string())?;

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
    let leader_snapshot_count =
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
    let learner_snapshot_count_before_compact = snapshot_file_count(harness, &learner)?;

    let compact_leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let compact_leader = base_voters
        .iter()
        .find(|node| node.id == compact_leader_id)
        .ok_or_else(|| format!("compact leader node {} not found", compact_leader_id))?;
    let compacted = post_meta_compact_via_admin_route(
        &client,
        gateway_addr(compact_leader, ingress_port).as_str(),
        route_prefix,
        compact_leader.name.as_str(),
        compact_revision,
    )
    .await?;
    if compacted.compacted_revision != compact_revision
        || compacted.current_revision < current_revision
    {
        return Err(format!(
            "unexpected compact-during-snapshot response: {:?}, expected_compacted={}, current>={}",
            compacted, compact_revision, current_revision
        ));
    }

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
    let learner_snapshot_count =
        wait_snapshot_file_count(harness, &learner, 1, Duration::from_secs(160)).await?;
    wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        gateway_addr(&learner, ingress_port).as_str(),
        route_prefix,
        learner.name.as_str(),
        Some(key5.as_str()),
        None,
        create_revisions[5],
        Duration::from_secs(120),
    )
    .await?;

    let key0_v3 = mvcc_compact_snapshot_value("v3", 0, value_bytes);
    let key1_v3 = mvcc_compact_snapshot_value("v3", 1, value_bytes);
    let key2_v3 = mvcc_compact_snapshot_value("v3", 2, value_bytes);
    let key3_v3 = mvcc_compact_snapshot_value("v3", 3, value_bytes);
    let key4_v3 = mvcc_compact_snapshot_value("v3", 4, value_bytes);
    let key10_v2 = mvcc_compact_snapshot_value("v2", 10, value_bytes);
    let key_last_v1 = mvcc_compact_snapshot_value("v1", item_count - 1, value_bytes);
    let current_expected_count = item_count - 5;
    let key10_update_revision = *update_revisions
        .get(&10)
        .ok_or_else(|| "missing compact snapshot key10 update revision".to_string())?;
    require_mvcc_snapshot_current_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count,
        &[
            (
                key0.as_str(),
                key0_v3.as_str(),
                *recreate_revisions.get(&0).unwrap(),
                *recreate_revisions.get(&0).unwrap(),
                1,
            ),
            (
                key10.as_str(),
                key10_v2.as_str(),
                create_revisions[10],
                key10_update_revision,
                2,
            ),
            (
                key_last.as_str(),
                key_last_v1.as_str(),
                create_revisions[item_count - 1],
                create_revisions[item_count - 1],
                1,
            ),
        ],
    )
    .await?;
    require_mvcc_snapshot_key_absent_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        key5.as_str(),
    )
    .await?;
    require_meta_at_revision_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        key0.as_str(),
        *recreate_revisions.get(&0).unwrap(),
        key0_v3.as_str(),
        *recreate_revisions.get(&0).unwrap(),
        1,
    )
    .await?;

    for node in &nodes {
        let node_gateway_addr = gateway_addr(node, ingress_port);
        expect_meta_query_status_via_cluster_inter_route(
            &client,
            node_gateway_addr.as_str(),
            route_prefix,
            node.name.as_str(),
            Some(key5.as_str()),
            None,
            Some(create_revisions[5]),
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        expect_meta_changes_status_via_cluster_inter_route(
            &client,
            node_gateway_addr.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            create_revisions[0],
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        let changes = query_meta_changes_via_cluster_inter_route(
            &client,
            node_gateway_addr.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            compact_revision + 1,
            8,
            None,
        )
        .await?;
        require_meta_changes(
            &changes,
            &[
                (
                    *recreate_revisions.get(&0).unwrap(),
                    key0.as_str(),
                    key0_v3.as_str(),
                    false,
                    *recreate_revisions.get(&0).unwrap(),
                    1,
                ),
                (
                    *recreate_revisions.get(&1).unwrap(),
                    key1.as_str(),
                    key1_v3.as_str(),
                    false,
                    *recreate_revisions.get(&1).unwrap(),
                    1,
                ),
                (
                    *recreate_revisions.get(&2).unwrap(),
                    key2.as_str(),
                    key2_v3.as_str(),
                    false,
                    *recreate_revisions.get(&2).unwrap(),
                    1,
                ),
                (
                    *recreate_revisions.get(&3).unwrap(),
                    key3.as_str(),
                    key3_v3.as_str(),
                    false,
                    *recreate_revisions.get(&3).unwrap(),
                    1,
                ),
                (
                    *recreate_revisions.get(&4).unwrap(),
                    key4.as_str(),
                    key4_v3.as_str(),
                    false,
                    *recreate_revisions.get(&4).unwrap(),
                    1,
                ),
            ],
        )?;
    }

    let post_key = format!("{}post-recovery", prefix);
    let post_value = mvcc_compact_snapshot_value("post", 0, value_bytes);
    let post_tx = exec_meta_tx_via_cluster_inter_route(
        &client,
        gateway_addr(&learner, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        BTreeMap::from([
            (
                key10.clone(),
                meta_tx_put_action(
                    key10.as_str(),
                    key10_v2.as_str(),
                    learner.name.as_str(),
                    Some(key10_update_revision),
                ),
            ),
            (
                post_key.clone(),
                meta_tx_put_action(
                    post_key.as_str(),
                    post_value.as_str(),
                    learner.name.as_str(),
                    Some(0),
                ),
            ),
        ]),
    )
    .await?;
    let post_revision = post_tx
        .revisions
        .get(&post_key)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing compact snapshot post revision for {}", post_key))?;
    require_mvcc_snapshot_current_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count + 1,
        &[
            (
                post_key.as_str(),
                post_value.as_str(),
                post_revision,
                post_revision,
                1,
            ),
            (
                key10.as_str(),
                key10_v2.as_str(),
                create_revisions[10],
                post_revision,
                3,
            ),
        ],
    )
    .await?;

    println!(
        "[klog-cluster-dv] MVCC compact during snapshot ok: items={}, value_bytes={}, chunk_bytes={}, add_leader={}, compact_leader={}, snapshot_leader={}, leader_snapshots={}, learner_snapshots_before_compact={}, learner_snapshots_after={}, temp_bytes_before_compact={}, compacted={}, current_revision={}, post_revision={}, prefix={}",
        item_count,
        value_bytes,
        chunk_bytes,
        add_leader_id,
        compact_leader_id,
        snapshot_leader_id,
        leader_snapshot_count,
        learner_snapshot_count_before_compact,
        learner_snapshot_count,
        temp_bytes,
        compact_revision,
        compacted.current_revision,
        post_revision,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_compact_during_snapshot() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_compact_during_snapshot_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

