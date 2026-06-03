async fn run_local_gateway_mvcc_failover_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-failover-dv";
    let setup = prepare_local_gateway_setup(harness, MVCC_FAILOVER_MODE, route_prefix, 3).await?;
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

    let source = nodes
        .iter()
        .find(|node| node.id != old_leader_id)
        .ok_or_else(|| format!("missing non-leader source node: leader={}", old_leader_id))?;
    let target = nodes
        .iter()
        .find(|node| node.id != old_leader_id && node.id != source.id)
        .or_else(|| nodes.iter().find(|node| node.id == old_leader_id))
        .ok_or_else(|| format!("missing target node: leader={}", old_leader_id))?;
    let observer = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .unwrap_or(target);
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let observer_gateway_addr = gateway_addr(observer, ingress_port);
    let suffix = unique_suffix("mvcc-failover");
    let prefix = format!("test/klog_mvcc_failover_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);
    let key_c = format!("{}c", prefix);
    let key_d = format!("{}d", prefix);

    let tx1 = exec_meta_tx_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        BTreeMap::from([
            (
                key_a.clone(),
                meta_tx_put_action(&key_a, "a-v1", target.name.as_str(), Some(0)),
            ),
            (
                key_b.clone(),
                meta_tx_put_action(&key_b, "b-v1", target.name.as_str(), Some(0)),
            ),
        ]),
    )
    .await?;
    let r1 = tx1
        .revisions
        .get(&key_a)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing failover tx1 revision for {}", key_a))?;
    if tx1.revisions.get(&key_b).and_then(|revision| *revision) != Some(r1) {
        return Err(format!(
            "failover tx1 keys did not share revision: {:?}",
            tx1
        ));
    }
    require_meta_version(tx1.meta_versions.get(&key_a), r1, r1, 1, false)?;
    require_meta_version(tx1.meta_versions.get(&key_b), r1, r1, 1, false)?;

    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        key_a.as_str(),
        "a-v2",
        Some(r1),
    )
    .await?;
    let r2 = a_v2.mod_revision;
    if a_v2.create_revision != r1 || a_v2.version != 2 || r2 != r1 + 1 {
        return Err(format!("unexpected failover a_v2 response: {:?}", a_v2));
    }

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let rev1 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r1),
        )
        .await?;
        require_meta_values(
            &rev1,
            &[(&key_a, "a-v1", r1, r1, 1), (&key_b, "b-v1", r1, r1, 1)],
        )?;
    }

    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .ok_or_else(|| format!("old leader node {} not found", old_leader_id))?;
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
    let new_leader_gateway = gateway_addr(new_leader, ingress_port);

    let deleted_b = delete_meta_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_b.as_str(),
    )
    .await?;
    let delete_version = deleted_b.meta_version.as_ref().ok_or_else(|| {
        format!(
            "missing failover key_b delete meta_version: {:?}",
            deleted_b
        )
    })?;
    require_meta_version(Some(delete_version), r1, r2 + 1, 0, true)?;
    let r3 = delete_version.mod_revision;

    expect_meta_put_status_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        MetaPutRequest {
            key: key_b.clone(),
            value: "stale-b".to_string(),
            node_name: Some(new_leader.name.clone()),
            expected_revision: Some(r1),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let b_v2 = put_meta_via_cluster_inter_route(
        &client,
        new_leader_gateway.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        key_b.as_str(),
        "b-v2",
        Some(0),
    )
    .await?;
    let r4 = b_v2.mod_revision;
    if b_v2.create_revision != r4 || b_v2.version != 1 || r4 != r3 + 1 {
        return Err(format!("unexpected failover b_v2 response: {:?}", b_v2));
    }

    let tx5 = exec_meta_tx_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        BTreeMap::from([
            (
                key_a.clone(),
                meta_tx_put_action(&key_a, "a-v3", failover_writer.name.as_str(), Some(r2)),
            ),
            (
                key_c.clone(),
                meta_tx_put_action(&key_c, "c-v1", failover_writer.name.as_str(), Some(0)),
            ),
            (
                key_d.clone(),
                meta_tx_put_action(&key_d, "d-v1", failover_writer.name.as_str(), Some(0)),
            ),
        ]),
    )
    .await?;
    let r5 = tx5
        .revisions
        .get(&key_a)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing failover tx5 revision for {}", key_a))?;
    if r5 != r4 + 1 {
        return Err(format!(
            "unexpected failover tx5 revision: r4={}, r5={}",
            r4, r5
        ));
    }
    require_meta_version(tx5.meta_versions.get(&key_a), r1, r5, 3, false)?;
    require_meta_version(tx5.meta_versions.get(&key_c), r5, r5, 1, false)?;
    require_meta_version(tx5.meta_versions.get(&key_d), r5, r5, 1, false)?;

    for node in &alive_nodes {
        let gateway = gateway_addr(node, ingress_port);
        let rev4 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r4),
        )
        .await?;
        require_meta_values(
            &rev4,
            &[(&key_a, "a-v2", r1, r2, 2), (&key_b, "b-v2", r4, r4, 1)],
        )?;

        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
        )
        .await?;
        require_meta_values(
            &current,
            &[
                (&key_a, "a-v3", r1, r5, 3),
                (&key_b, "b-v2", r4, r4, 1),
                (&key_c, "c-v1", r5, r5, 1),
                (&key_d, "d-v1", r5, r5, 1),
            ],
        )?;
    }

    let compacted = post_meta_compact_via_admin_route(
        &client,
        new_leader_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        r3,
    )
    .await?;
    if compacted.compacted_revision != r3 || compacted.current_revision != r5 {
        return Err(format!(
            "unexpected failover compaction response: {:?}, expected compacted={}, current={}",
            compacted, r3, r5
        ));
    }

    expect_meta_query_status_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        None,
        Some(prefix.as_str()),
        Some(r1),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        new_leader_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        prefix.as_str(),
        r1,
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    for node in &alive_nodes {
        let gateway = gateway_addr(node, ingress_port);
        let rev4 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r4),
        )
        .await?;
        require_meta_values(
            &rev4,
            &[(&key_a, "a-v2", r1, r2, 2), (&key_b, "b-v2", r4, r4, 1)],
        )?;
    }

    let post_compact_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        prefix.as_str(),
        r4,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[
            (r4, &key_b, "b-v2", false, r4, 1),
            (r5, &key_a, "a-v3", false, r1, 3),
            (r5, &key_c, "c-v1", false, r5, 1),
            (r5, &key_d, "d-v1", false, r5, 1),
        ],
    )?;

    println!(
        "[klog-cluster-dv] MVCC failover ok: old_leader={}, new_leader={}, writer={}, revisions=[{},{},{},{},{}], prefix={}",
        old_leader_id, new_leader_id, failover_writer.name, r1, r2, r3, r4, r5, prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_failover() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_failover_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_auto_compact_failover_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-auto-compact-failover-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_AUTO_COMPACT_FAILOVER_MODE, route_prefix, 3)
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
    let meta_compaction = KLogMetaCompactionPatch {
        retention_revisions: 6,
        check_interval_ms: 200,
        min_compact_gap: 2,
    };

    for node in &nodes {
        let config =
            write_klog_config_with_meta_compaction(harness, node, &voter_config, meta_compaction)?;
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
    let initial_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;

    let suffix = unique_suffix("mvcc-auto-compact-failover");
    let prefix = format!("test/klog_mvcc_auto_compact_failover_dv/{}/", suffix);
    let mut expected_current = Vec::new();
    let phase1_count = 14usize;
    for index in 0..phase1_count {
        let source = &nodes[index % nodes.len()];
        let target = &nodes[(index + 1) % nodes.len()];
        let key = format!("{}phase1-{:03}", prefix, index);
        let value = format!("phase1-value-{:03}", index);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            gateway_addr(source, ingress_port).as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected auto-compact phase1 put response: {:?}",
                stored
            ));
        }
        expected_current.push(ExpectedMetaChange {
            revision: stored.mod_revision,
            key,
            value,
            deleted: false,
            create_revision: stored.create_revision,
            version: stored.version,
        });
    }
    let first_revision = expected_current
        .first()
        .ok_or_else(|| "missing auto-compact first revision".to_string())?
        .revision;
    let phase1_last = expected_current
        .last()
        .cloned()
        .ok_or_else(|| "missing auto-compact phase1 last revision".to_string())?;

    let observer = nodes
        .iter()
        .find(|node| node.id != initial_leader_id)
        .unwrap_or(&nodes[0]);
    wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        gateway_addr(observer, ingress_port).as_str(),
        route_prefix,
        observer.name.as_str(),
        Some(expected_current[0].key.as_str()),
        None,
        first_revision,
        Duration::from_secs(40),
    )
    .await?;

    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(20),
    )
    .await?;
    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .ok_or_else(|| format!("old leader node {} not found", old_leader_id))?;
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

    let phase2_count = 14usize;
    for index in 0..phase2_count {
        let source = &alive_nodes[index % alive_nodes.len()];
        let target = &alive_nodes[(index + 1) % alive_nodes.len()];
        let key = format!("{}phase2-{:03}", prefix, index);
        let value = format!("phase2-value-{:03}", index);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            gateway_addr(source, ingress_port).as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected auto-compact phase2 put response: {:?}",
                stored
            ));
        }
        expected_current.push(ExpectedMetaChange {
            revision: stored.mod_revision,
            key,
            value,
            deleted: false,
            create_revision: stored.create_revision,
            version: stored.version,
        });
    }

    let alive_observer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or(&alive_nodes[0]);
    wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        gateway_addr(alive_observer, ingress_port).as_str(),
        route_prefix,
        alive_observer.name.as_str(),
        Some(phase1_last.key.as_str()),
        None,
        phase1_last.revision,
        Duration::from_secs(40),
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        gateway_addr(alive_observer, ingress_port).as_str(),
        route_prefix,
        alive_observer.name.as_str(),
        prefix.as_str(),
        phase1_last.revision,
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    for node in &alive_nodes {
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            expected_current.len() + 8,
        )
        .await?;
        require_expected_current_meta_values(&current, expected_current.as_slice())?;
    }

    let latest = expected_current
        .last()
        .ok_or_else(|| "missing auto-compact latest revision".to_string())?;
    let post_compact_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        gateway_addr(alive_observer, ingress_port).as_str(),
        route_prefix,
        alive_observer.name.as_str(),
        prefix.as_str(),
        latest.revision,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[(
            latest.revision,
            latest.key.as_str(),
            latest.value.as_str(),
            false,
            latest.create_revision,
            latest.version,
        )],
    )?;

    println!(
        "[klog-cluster-dv] MVCC auto-compact failover ok: initial_leader={}, stopped_leader={}, new_leader={}, first_revision={}, phase1_last_revision={}, latest_revision={}, keys={}, prefix={}",
        initial_leader_id,
        old_leader_id,
        new_leader_id,
        first_revision,
        phase1_last.revision,
        latest.revision,
        expected_current.len(),
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_auto_compact_failover() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_auto_compact_failover_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_compaction_leader_switch_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-compaction-leader-switch-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_COMPACTION_LEADER_SWITCH_MODE, route_prefix, 3)
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
    let meta_compaction = KLogMetaCompactionPatch {
        retention_revisions: 8,
        check_interval_ms: 1500,
        min_compact_gap: 1,
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config =
            write_klog_config_with_meta_compaction(harness, node, &voter_config, meta_compaction)?;
        configs.insert(node.id, config.clone());
        spawn_klog_with_log_level(harness, &klog_daemon_bin, &config, node, "info")?;
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

    let suffix = unique_suffix("mvcc-compaction-leader-switch");
    let prefix = format!("test/klog_mvcc_compaction_leader_switch_dv/{}/", suffix);
    let mut expected_current = Vec::new();
    for index in 0..6usize {
        let source = &nodes[index % nodes.len()];
        let target = &nodes[(index + 1) % nodes.len()];
        let key = format!("{}manual-{:03}", prefix, index);
        let value = format!("manual-value-{:03}", index);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            gateway_addr(source, ingress_port).as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected manual phase put response: {:?}",
                stored
            ));
        }
        expected_current.push(ExpectedMetaChange {
            revision: stored.mod_revision,
            key,
            value,
            deleted: false,
            create_revision: stored.create_revision,
            version: stored.version,
        });
    }
    let manual_compact_revision = expected_current
        .get(2)
        .ok_or_else(|| "missing manual compact target revision".to_string())?
        .revision;
    let manual_retained_revision = expected_current
        .get(3)
        .ok_or_else(|| "missing manual retained revision".to_string())?
        .revision;
    let manual_current_revision = expected_current
        .last()
        .ok_or_else(|| "missing manual current revision".to_string())?
        .revision;

    let manual_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let manual_leader = nodes
        .iter()
        .find(|node| node.id == manual_leader_id)
        .cloned()
        .ok_or_else(|| format!("manual compact leader {} not found", manual_leader_id))?;
    let manual_url = cluster_route_url(
        gateway_addr(&manual_leader, ingress_port).as_str(),
        route_prefix,
        manual_leader.name.as_str(),
        "admin",
        "/meta-compact",
    );
    let manual_client = client.clone();
    let manual_task = tokio::spawn(async move {
        let response = manual_client
            .post(manual_url.as_str())
            .json(&MetaCompactRequest {
                revision: manual_compact_revision,
            })
            .send()
            .await
            .map_err(|err| format!("manual in-flight meta-compact request failed: {}", err))?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
            return Err(format!(
                "manual in-flight meta-compact returned {}: {}",
                status, body
            ));
        }
        response
            .json::<MetaCompactResponse>()
            .await
            .map_err(|err| format!("manual in-flight meta-compact decode failed: {}", err))
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    harness.stop(format!("klog-{}", manual_leader.name).as_str())?;
    let manual_result = manual_task
        .await
        .map_err(|err| format!("manual in-flight compact task join failed: {}", err));

    let alive_after_manual = nodes
        .iter()
        .filter(|node| node.id != manual_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_after_manual,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let manual_failover_leader_id = wait_consistent_leader(
        &client,
        &alive_after_manual,
        ingress_port,
        route_prefix,
        Some(manual_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    let manual_observer = alive_after_manual
        .first()
        .ok_or_else(|| "missing manual alive observer".to_string())?;
    let manual_observer_gateway = gateway_addr(manual_observer, ingress_port);
    let manual_already_compacted = wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        manual_observer_gateway.as_str(),
        route_prefix,
        manual_observer.name.as_str(),
        None,
        Some(prefix.as_str()),
        manual_compact_revision,
        Duration::from_secs(8),
    )
    .await
    .is_ok();
    if !manual_already_compacted {
        let new_leader = alive_after_manual
            .iter()
            .find(|node| node.id == manual_failover_leader_id)
            .ok_or_else(|| {
                format!(
                    "manual failover leader {} not found",
                    manual_failover_leader_id
                )
            })?;
        let compacted = post_meta_compact_via_admin_route(
            &client,
            gateway_addr(new_leader, ingress_port).as_str(),
            route_prefix,
            new_leader.name.as_str(),
            manual_compact_revision,
        )
        .await?;
        if compacted.compacted_revision != manual_compact_revision
            || compacted.current_revision < manual_current_revision
        {
            return Err(format!(
                "unexpected manual failover compaction response: {:?}, expected compacted={}, current>={}",
                compacted, manual_compact_revision, manual_current_revision
            ));
        }
    } else if let Ok(Ok(compacted)) = manual_result
        && (compacted.compacted_revision != manual_compact_revision
            || compacted.current_revision < manual_current_revision)
    {
        return Err(format!(
            "unexpected in-flight manual compaction response: {:?}, expected compacted={}, current>={}",
            compacted, manual_compact_revision, manual_current_revision
        ));
    }

    for node in &alive_after_manual {
        wait_meta_query_compacted_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            None,
            Some(prefix.as_str()),
            manual_compact_revision,
            Duration::from_secs(20),
        )
        .await?;
        let retained = query_meta_at_revision_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            expected_current[3].key.as_str(),
            Some(manual_retained_revision),
        )
        .await?;
        require_meta_values(
            &retained,
            &[(
                expected_current[3].key.as_str(),
                expected_current[3].value.as_str(),
                expected_current[3].create_revision,
                expected_current[3].revision,
                expected_current[3].version,
            )],
        )?;
    }

    let manual_commit_pattern = format!(
        "StateMachine meta-compact request committed: compacted_revision={},",
        manual_compact_revision
    );
    for node in &nodes {
        let count = count_klog_out_log_occurrences(harness, node, manual_commit_pattern.as_str())?;
        if count > 1 {
            return Err(format!(
                "manual compact target committed more than once on {}: target={}, count={}",
                node.name, manual_compact_revision, count
            ));
        }
    }

    let manual_leader_config = configs
        .get(&manual_leader_id)
        .ok_or_else(|| format!("missing config for manual leader {}", manual_leader_id))?;
    spawn_klog_with_log_level(
        harness,
        &klog_daemon_bin,
        manual_leader_config,
        &manual_leader,
        "info",
    )?;
    wait_tcp(
        "127.0.0.1",
        manual_leader.ports.admin,
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
        Duration::from_secs(90),
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
    for node in &nodes {
        wait_meta_query_compacted_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            None,
            Some(prefix.as_str()),
            manual_compact_revision,
            Duration::from_secs(30),
        )
        .await?;
    }

    for index in 0..14usize {
        let source = &nodes[index % nodes.len()];
        let target = &nodes[(index + 1) % nodes.len()];
        let key = format!("{}auto-{:03}", prefix, index);
        let value = format!("auto-value-{:03}", index);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            gateway_addr(source, ingress_port).as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!("unexpected auto phase put response: {:?}", stored));
        }
        expected_current.push(ExpectedMetaChange {
            revision: stored.mod_revision,
            key,
            value,
            deleted: false,
            create_revision: stored.create_revision,
            version: stored.version,
        });
    }
    let auto_compact_probe_revision = manual_retained_revision;
    let auto_latest = expected_current
        .last()
        .cloned()
        .ok_or_else(|| "missing auto phase latest revision".to_string())?;
    let auto_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let auto_leader = nodes
        .iter()
        .find(|node| node.id == auto_leader_id)
        .cloned()
        .ok_or_else(|| format!("auto compact leader {} not found", auto_leader_id))?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    harness.stop(format!("klog-{}", auto_leader.name).as_str())?;

    let alive_after_auto = nodes
        .iter()
        .filter(|node| node.id != auto_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_after_auto,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let auto_failover_leader_id = wait_consistent_leader(
        &client,
        &alive_after_auto,
        ingress_port,
        route_prefix,
        Some(auto_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    let auto_observer = alive_after_auto
        .first()
        .ok_or_else(|| "missing auto alive observer".to_string())?;
    wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        gateway_addr(auto_observer, ingress_port).as_str(),
        route_prefix,
        auto_observer.name.as_str(),
        None,
        Some(prefix.as_str()),
        auto_compact_probe_revision,
        Duration::from_secs(70),
    )
    .await?;
    for node in &alive_after_auto {
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            expected_current.len() + 8,
        )
        .await?;
        require_expected_current_meta_values(&current, expected_current.as_slice())?;
        let latest_page = query_meta_changes_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            auto_latest.revision,
            8,
            None,
        )
        .await?;
        require_meta_changes(
            &latest_page,
            &[(
                auto_latest.revision,
                auto_latest.key.as_str(),
                auto_latest.value.as_str(),
                false,
                auto_latest.create_revision,
                auto_latest.version,
            )],
        )?;
    }

    let auto_leader_config = configs
        .get(&auto_leader_id)
        .ok_or_else(|| format!("missing config for auto leader {}", auto_leader_id))?;
    spawn_klog_with_log_level(
        harness,
        &klog_daemon_bin,
        auto_leader_config,
        &auto_leader,
        "info",
    )?;
    wait_tcp(
        "127.0.0.1",
        auto_leader.ports.admin,
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
        Duration::from_secs(90),
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
    for node in &nodes {
        wait_meta_query_compacted_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            None,
            Some(prefix.as_str()),
            auto_compact_probe_revision,
            Duration::from_secs(40),
        )
        .await?;
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            expected_current.len() + 8,
        )
        .await?;
        require_expected_current_meta_values(&current, expected_current.as_slice())?;
    }

    println!(
        "[klog-cluster-dv] MVCC compaction leader switch ok: manual_leader={}, manual_failover_leader={}, auto_switch_leader={}, auto_failover_leader={}, manual_compacted={}, auto_probe_compacted={}, latest_revision={}, keys={}, prefix={}",
        manual_leader_id,
        manual_failover_leader_id,
        auto_leader_id,
        auto_failover_leader_id,
        manual_compact_revision,
        auto_compact_probe_revision,
        auto_latest.revision,
        expected_current.len(),
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_compaction_leader_switch() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_compaction_leader_switch_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_crash_recovery_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-crash-recovery-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_CRASH_RECOVERY_MODE, route_prefix, 3).await?;
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
    let leader_before_crash = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;

    let suffix = unique_suffix("mvcc-crash-recovery");
    let prefix = format!("test/klog_mvcc_crash_recovery_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);
    let key_c = format!("{}c", prefix);
    let key_d = format!("{}d", prefix);
    let key_e = format!("{}e", prefix);
    let source = nodes
        .iter()
        .find(|node| node.id != leader_before_crash)
        .unwrap_or(&nodes[0]);
    let target = nodes
        .iter()
        .find(|node| node.id != source.id)
        .unwrap_or(source);
    let source_gateway = gateway_addr(source, ingress_port);

    let a_v1 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_a.as_str(),
        "a-v1",
        Some(0),
    )
    .await?;
    let r1 = a_v1.mod_revision;
    if a_v1.create_revision != r1 || a_v1.version != 1 {
        return Err(format!("unexpected crash recovery a_v1: {:?}", a_v1));
    }

    let b_v1 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_b.as_str(),
        "b-v1",
        Some(0),
    )
    .await?;
    let r2 = b_v1.mod_revision;
    if b_v1.create_revision != r2 || b_v1.version != 1 || r2 != r1 + 1 {
        return Err(format!("unexpected crash recovery b_v1: {:?}", b_v1));
    }

    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_a.as_str(),
        "a-v2",
        Some(r1),
    )
    .await?;
    let r3 = a_v2.mod_revision;
    if a_v2.create_revision != r1 || a_v2.version != 2 || r3 != r2 + 1 {
        return Err(format!("unexpected crash recovery a_v2: {:?}", a_v2));
    }

    let b_deleted = delete_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_b.as_str(),
    )
    .await?;
    let b_delete_version = b_deleted.meta_version.as_ref().ok_or_else(|| {
        format!(
            "missing crash recovery b delete meta_version: {:?}",
            b_deleted
        )
    })?;
    require_meta_version(Some(b_delete_version), r2, r3 + 1, 0, true)?;
    let r4 = b_delete_version.mod_revision;

    let b_v2 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_b.as_str(),
        "b-v2",
        Some(0),
    )
    .await?;
    let r5 = b_v2.mod_revision;
    if b_v2.create_revision != r5 || b_v2.version != 1 || r5 != r4 + 1 {
        return Err(format!("unexpected crash recovery b_v2: {:?}", b_v2));
    }

    let tx6 = exec_meta_tx_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        BTreeMap::from([
            (
                key_c.clone(),
                meta_tx_put_action(&key_c, "c-v1", target.name.as_str(), Some(0)),
            ),
            (
                key_d.clone(),
                meta_tx_put_action(&key_d, "d-v1", target.name.as_str(), Some(0)),
            ),
        ]),
    )
    .await?;
    let r6 = tx6
        .revisions
        .get(&key_c)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing crash recovery tx6 revision for {}", key_c))?;
    if tx6.revisions.get(&key_d).and_then(|revision| *revision) != Some(r6) || r6 != r5 + 1 {
        return Err(format!("unexpected crash recovery tx6 response: {:?}", tx6));
    }
    require_meta_version(tx6.meta_versions.get(&key_c), r6, r6, 1, false)?;
    require_meta_version(tx6.meta_versions.get(&key_d), r6, r6, 1, false)?;

    let leader = nodes
        .iter()
        .find(|node| node.id == leader_before_crash)
        .ok_or_else(|| format!("leader node {} not found", leader_before_crash))?;
    let leader_gateway = gateway_addr(leader, ingress_port);
    let compacted = post_meta_compact_via_admin_route(
        &client,
        leader_gateway.as_str(),
        route_prefix,
        leader.name.as_str(),
        r4,
    )
    .await?;
    if compacted.compacted_revision != r4 || compacted.current_revision != r6 {
        return Err(format!(
            "unexpected crash recovery compaction response: {:?}, expected compacted={}, current={}",
            compacted, r4, r6
        ));
    }
    for node in &nodes {
        expect_meta_query_status_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            Some(key_a.as_str()),
            None,
            Some(r1),
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
    }

    harness.stop(format!("klog-{}", leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != leader_before_crash)
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
        Some(leader_before_crash),
        Duration::from_secs(70),
    )
    .await?;
    let failover_writer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or(&alive_nodes[0]);
    let new_leader = alive_nodes
        .iter()
        .find(|node| node.id == new_leader_id)
        .ok_or_else(|| format!("new leader node {} not found", new_leader_id))?;
    let failover_gateway = gateway_addr(failover_writer, ingress_port);

    let a_v3 = put_meta_via_cluster_inter_route(
        &client,
        failover_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_a.as_str(),
        "a-v3",
        Some(r3),
    )
    .await?;
    let r7 = a_v3.mod_revision;
    if a_v3.create_revision != r1 || a_v3.version != 3 || r7 != r6 + 1 {
        return Err(format!("unexpected crash recovery a_v3: {:?}", a_v3));
    }

    let c_deleted = delete_meta_via_cluster_inter_route(
        &client,
        failover_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_c.as_str(),
    )
    .await?;
    let c_delete_version = c_deleted.meta_version.as_ref().ok_or_else(|| {
        format!(
            "missing crash recovery c delete meta_version: {:?}",
            c_deleted
        )
    })?;
    require_meta_version(Some(c_delete_version), r6, r7 + 1, 0, true)?;
    let r8 = c_delete_version.mod_revision;

    let c_v2 = put_meta_via_cluster_inter_route(
        &client,
        failover_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_c.as_str(),
        "c-v2",
        Some(0),
    )
    .await?;
    let r9 = c_v2.mod_revision;
    if c_v2.create_revision != r9 || c_v2.version != 1 || r9 != r8 + 1 {
        return Err(format!("unexpected crash recovery c_v2: {:?}", c_v2));
    }

    let e_v1 = put_meta_via_cluster_inter_route(
        &client,
        failover_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_e.as_str(),
        "e-v1",
        Some(0),
    )
    .await?;
    let r10 = e_v1.mod_revision;
    if e_v1.create_revision != r10 || e_v1.version != 1 || r10 != r9 + 1 {
        return Err(format!("unexpected crash recovery e_v1: {:?}", e_v1));
    }

    let old_leader_config = configs
        .get(&leader_before_crash)
        .ok_or_else(|| format!("missing config for old leader {}", leader_before_crash))?;
    spawn_klog(harness, &klog_daemon_bin, old_leader_config, leader)?;
    wait_tcp("127.0.0.1", leader.ports.admin, Duration::from_secs(12)).await?;
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
    let leader_after_recovery = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;

    wait_meta_prefix_count_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        5,
        Duration::from_secs(60),
    )
    .await?;
    let expected_current = vec![
        ExpectedMetaChange {
            revision: r7,
            key: key_a.clone(),
            value: "a-v3".to_string(),
            deleted: false,
            create_revision: r1,
            version: 3,
        },
        ExpectedMetaChange {
            revision: r5,
            key: key_b.clone(),
            value: "b-v2".to_string(),
            deleted: false,
            create_revision: r5,
            version: 1,
        },
        ExpectedMetaChange {
            revision: r9,
            key: key_c.clone(),
            value: "c-v2".to_string(),
            deleted: false,
            create_revision: r9,
            version: 1,
        },
        ExpectedMetaChange {
            revision: r6,
            key: key_d.clone(),
            value: "d-v1".to_string(),
            deleted: false,
            create_revision: r6,
            version: 1,
        },
        ExpectedMetaChange {
            revision: r10,
            key: key_e.clone(),
            value: "e-v1".to_string(),
            deleted: false,
            create_revision: r10,
            version: 1,
        },
    ];
    for node in &nodes {
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
        )
        .await?;
        require_expected_current_meta_values(&current, expected_current.as_slice())?;
        expect_meta_query_status_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            Some(key_a.as_str()),
            None,
            Some(r4),
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        expect_meta_changes_status_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            r1,
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
    }

    let changes = query_meta_changes_via_cluster_inter_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        prefix.as_str(),
        r5,
        16,
        None,
    )
    .await?;
    require_meta_changes(
        &changes,
        &[
            (r5, &key_b, "b-v2", false, r5, 1),
            (r6, &key_c, "c-v1", false, r6, 1),
            (r6, &key_d, "d-v1", false, r6, 1),
            (r7, &key_a, "a-v3", false, r1, 3),
            (r8, &key_c, "c-v1", true, r6, 0),
            (r9, &key_c, "c-v2", false, r9, 1),
            (r10, &key_e, "e-v1", false, r10, 1),
        ],
    )?;

    println!(
        "[klog-cluster-dv] MVCC crash recovery ok: crashed_leader={}, failover_leader={}, recovered_leader={}, compacted_revision={}, latest_revision={}, prefix={}",
        leader_before_crash, new_leader_id, leader_after_recovery, r4, r10, prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_crash_recovery() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_crash_recovery_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

