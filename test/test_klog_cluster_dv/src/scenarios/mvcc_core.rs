async fn run_local_gateway_mvcc_cluster_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-cluster-dv";
    let setup = prepare_local_gateway_setup(harness, MVCC_CLUSTER_MODE, route_prefix, 3).await?;
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

    let source = nodes
        .first()
        .ok_or_else(|| "missing source node".to_string())?;
    let target = nodes
        .get(1)
        .ok_or_else(|| "missing target node".to_string())?;
    let observer = nodes
        .get(2)
        .ok_or_else(|| "missing observer node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let target_gateway_addr = gateway_addr(target, ingress_port);
    let observer_gateway_addr = gateway_addr(observer, ingress_port);
    let suffix = unique_suffix("mvcc-cluster");
    let prefix = format!("test/klog_mvcc_cluster_dv/{}/", suffix);
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
        .ok_or_else(|| format!("missing tx1 revision for {}", key_a))?;
    if tx1.revisions.get(&key_b).and_then(|revision| *revision) != Some(r1) {
        return Err(format!("tx1 keys did not share revision: {:?}", tx1));
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
        return Err(format!("unexpected a_v2 MVCC response: {:?}", a_v2));
    }

    let deleted_b = delete_meta_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        key_b.as_str(),
    )
    .await?;
    if deleted_b.key != key_b
        || !deleted_b.existed
        || deleted_b.prev_meta.as_ref().map(|item| item.revision) != Some(r1)
    {
        return Err(format!("unexpected key_b delete response: {:?}", deleted_b));
    }
    let delete_version = deleted_b
        .meta_version
        .as_ref()
        .ok_or_else(|| format!("missing key_b delete meta_version: {:?}", deleted_b))?;
    require_meta_version(Some(delete_version), r1, r2 + 1, 0, true)?;
    let r3 = delete_version.mod_revision;

    expect_meta_put_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        MetaPutRequest {
            key: key_b.clone(),
            value: "stale-b".to_string(),
            node_name: Some(target.name.clone()),
            expected_revision: Some(r1),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let b_v2 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        key_b.as_str(),
        "b-v2",
        Some(0),
    )
    .await?;
    let r4 = b_v2.mod_revision;
    if b_v2.create_revision != r4 || b_v2.version != 1 || r4 != r3 + 1 {
        return Err(format!("unexpected b_v2 MVCC response: {:?}", b_v2));
    }

    let tx5 = exec_meta_tx_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        BTreeMap::from([
            (
                key_a.clone(),
                meta_tx_put_action(&key_a, "a-v3", target.name.as_str(), Some(r2)),
            ),
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
    let r5 = tx5
        .revisions
        .get(&key_a)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing tx5 revision for {}", key_a))?;
    if r5 != r4 + 1 {
        return Err(format!("unexpected tx5 revision: r4={}, r5={}", r4, r5));
    }
    require_meta_version(tx5.meta_versions.get(&key_a), r1, r5, 3, false)?;
    require_meta_version(tx5.meta_versions.get(&key_c), r5, r5, 1, false)?;
    require_meta_version(tx5.meta_versions.get(&key_d), r5, r5, 1, false)?;

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

        let rev3 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r3),
        )
        .await?;
        require_meta_values(&rev3, &[(&key_a, "a-v2", r1, r2, 2)])?;

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

    let page1 = query_meta_changes_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        r1,
        4,
        None,
    )
    .await?;
    if !page1.has_more || page1.next_cursor.is_none() || page1.current_revision < r5 {
        return Err(format!(
            "unexpected first changes page metadata: {:?}",
            page1
        ));
    }
    require_meta_changes(
        &page1,
        &[
            (r1, &key_a, "a-v1", false, r1, 1),
            (r1, &key_b, "b-v1", false, r1, 1),
            (r2, &key_a, "a-v2", false, r1, 2),
            (r3, &key_b, "b-v1", true, r1, 0),
        ],
    )?;

    let page2 = query_meta_changes_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        r1,
        4,
        page1.next_cursor.as_ref(),
    )
    .await?;
    if page2.has_more || page2.next_start_revision <= r5 {
        return Err(format!(
            "unexpected second changes page metadata: {:?}",
            page2
        ));
    }
    require_meta_changes(
        &page2,
        &[
            (r4, &key_b, "b-v2", false, r4, 1),
            (r5, &key_a, "a-v3", false, r1, 3),
            (r5, &key_c, "c-v1", false, r5, 1),
            (r5, &key_d, "d-v1", false, r5, 1),
        ],
    )?;

    let leader_node = nodes
        .iter()
        .find(|node| node.id == leader_before)
        .ok_or_else(|| format!("leader node {} not found", leader_before))?;
    let leader_gateway_addr = gateway_addr(leader_node, ingress_port);
    let compacted = post_meta_compact_via_admin_route(
        &client,
        leader_gateway_addr.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        r4,
    )
    .await?;
    if compacted.compacted_revision != r4 || compacted.current_revision != r5 {
        return Err(format!(
            "unexpected compaction response: {:?}, expected compacted={}, current={}",
            compacted, r4, r5
        ));
    }

    expect_meta_query_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        None,
        Some(prefix.as_str()),
        Some(r1),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        r1,
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let post_compact_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r4 + 1,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[
            (r5, &key_a, "a-v3", false, r1, 3),
            (r5, &key_c, "c-v1", false, r5, 1),
            (r5, &key_d, "d-v1", false, r5, 1),
        ],
    )?;

    for node in &nodes {
        harness.stop(format!("klog-{}", node.name).as_str())?;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    for node in &nodes {
        let config = configs
            .get(&node.id)
            .ok_or_else(|| format!("restart config for node {} not found", node.id))?;
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

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
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

    let restarted_observer_gateway = gateway_addr(observer, ingress_port);
    expect_meta_query_status_via_cluster_inter_route(
        &client,
        restarted_observer_gateway.as_str(),
        route_prefix,
        observer.name.as_str(),
        Some(key_a.as_str()),
        None,
        Some(r4),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    let after_restart_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        restarted_observer_gateway.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r5,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &after_restart_changes,
        &[
            (r5, &key_a, "a-v3", false, r1, 3),
            (r5, &key_c, "c-v1", false, r5, 1),
            (r5, &key_d, "d-v1", false, r5, 1),
        ],
    )?;

    println!(
        "[klog-cluster-dv] mvcc cluster ok: leader_before={}, leader_after={}, revisions=[{},{},{},{},{}], prefix={}",
        leader_before, leader_after, r1, r2, r3, r4, r5, prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_cluster() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_cluster_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_change_feed_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-change-feed-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_CHANGE_FEED_MODE, route_prefix, 3).await?;
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
    let source = nodes
        .first()
        .ok_or_else(|| "missing source node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target node".to_string())?;
    let observer = nodes
        .iter()
        .find(|node| node.name != source.name && node.name != target.name)
        .unwrap_or(source);
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let target_gateway_addr = gateway_addr(target, ingress_port);
    let observer_gateway_addr = gateway_addr(observer, ingress_port);
    let suffix = unique_suffix("mvcc-change-feed");
    let prefix = format!("test/klog_mvcc_change_feed_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);

    let empty_started = std::time::Instant::now();
    let empty = query_meta_changes_with_wait_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        1,
        8,
        None,
        Some(350),
    )
    .await?;
    let empty_elapsed = empty_started.elapsed();
    if !empty.items.is_empty()
        || empty.has_more
        || empty.next_cursor.is_some()
        || empty.next_start_revision != 1
    {
        return Err(format!(
            "unexpected empty long-poll response: elapsed_ms={}, response={:?}",
            empty_elapsed.as_millis(),
            empty
        ));
    }
    if empty_elapsed < Duration::from_millis(200) {
        return Err(format!(
            "empty long-poll returned too early: elapsed_ms={}, response={:?}",
            empty_elapsed.as_millis(),
            empty
        ));
    }

    let waiter_client = client.clone();
    let waiter_gateway_addr = observer_gateway_addr.clone();
    let waiter_route_prefix = route_prefix.to_string();
    let waiter_node_name = observer.name.clone();
    let waiter_prefix = prefix.clone();
    let wait_started = std::time::Instant::now();
    let wait_task = tokio::spawn(async move {
        query_meta_changes_with_wait_via_cluster_inter_route(
            &waiter_client,
            waiter_gateway_addr.as_str(),
            waiter_route_prefix.as_str(),
            waiter_node_name.as_str(),
            waiter_prefix.as_str(),
            1,
            8,
            None,
            Some(1_500),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    let a_v1 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        key_a.as_str(),
        "a-v1",
        Some(0),
    )
    .await?;
    let r1 = a_v1.mod_revision;
    let waited = wait_task
        .await
        .map_err(|err| format!("long-poll change-feed task join failed: {}", err))??;
    let wait_elapsed = wait_started.elapsed();
    if wait_elapsed >= Duration::from_millis(1_450) {
        return Err(format!(
            "long-poll did not return promptly after write: elapsed_ms={}, response={:?}",
            wait_elapsed.as_millis(),
            waited
        ));
    }
    require_meta_changes(&waited, &[(r1, &key_a, "a-v1", false, r1, 1)])?;
    if waited.next_start_revision != r1 + 1 || waited.current_revision < r1 {
        return Err(format!(
            "unexpected long-poll next revision after write: response={:?}",
            waited
        ));
    }

    let b_v1 = put_meta_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        key_b.as_str(),
        "b-v1",
        Some(0),
    )
    .await?;
    let r2 = b_v1.mod_revision;
    if r2 != r1 + 1 {
        return Err(format!("unexpected key_b revision: r1={}, r2={}", r1, r2));
    }
    let deleted_a = delete_meta_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        key_a.as_str(),
    )
    .await?;
    let delete_version = deleted_a
        .meta_version
        .as_ref()
        .ok_or_else(|| format!("missing key_a delete meta_version: {:?}", deleted_a))?;
    require_meta_version(Some(delete_version), r1, r2 + 1, 0, true)?;
    let r3 = delete_version.mod_revision;
    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        key_a.as_str(),
        "a-v2",
        Some(0),
    )
    .await?;
    let r4 = a_v2.mod_revision;
    if a_v2.create_revision != r4 || a_v2.version != 1 || r4 != r3 + 1 {
        return Err(format!("unexpected key_a recreate response: {:?}", a_v2));
    }

    let page1 = query_meta_changes_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r1,
        1,
        None,
    )
    .await?;
    if !page1.has_more || page1.next_cursor.is_none() {
        return Err(format!(
            "change-feed cursor page did not return cursor: {:?}",
            page1
        ));
    }
    require_meta_changes(&page1, &[(r1, &key_a, "a-v1", false, r1, 1)])?;
    let resume_cursor = page1
        .next_cursor
        .clone()
        .ok_or_else(|| "missing change-feed resume cursor".to_string())?;

    let compacted = post_meta_compact_via_admin_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        r1,
    )
    .await?;
    if compacted.compacted_revision != r1 || compacted.current_revision != r4 {
        return Err(format!(
            "unexpected change-feed compaction response: {:?}, expected compacted={}, current={}",
            compacted, r1, r4
        ));
    }

    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r1,
        Some(&resume_cursor),
        Some(500),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        r1,
        None,
        Some(500),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let post_compact_changes = query_meta_changes_with_wait_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r2,
        8,
        None,
        Some(500),
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[
            (r2, &key_b, "b-v1", false, r2, 1),
            (r3, &key_a, "a-v1", true, r1, 0),
            (r4, &key_a, "a-v2", false, r4, 1),
        ],
    )?;
    if post_compact_changes.next_start_revision != r4 + 1 {
        return Err(format!(
            "unexpected post-compact next_start_revision: {:?}",
            post_compact_changes
        ));
    }

    let after_current_started = std::time::Instant::now();
    let after_current = query_meta_changes_with_wait_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r4 + 1,
        8,
        None,
        Some(350),
    )
    .await?;
    let after_current_elapsed = after_current_started.elapsed();
    if !after_current.items.is_empty()
        || after_current.next_start_revision != r4 + 1
        || after_current_elapsed < Duration::from_millis(200)
    {
        return Err(format!(
            "unexpected post-current empty long-poll: elapsed_ms={}, response={:?}",
            after_current_elapsed.as_millis(),
            after_current
        ));
    }

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
            &[(&key_a, "a-v2", r4, r4, 1), (&key_b, "b-v1", r2, r2, 1)],
        )?;
    }

    println!(
        "[klog-cluster-dv] MVCC change-feed long-poll ok: leader={}, empty_wait_ms={}, wake_wait_ms={}, revisions=[{},{},{},{}], prefix={}",
        leader_id,
        empty_elapsed.as_millis(),
        wait_elapsed.as_millis(),
        r1,
        r2,
        r3,
        r4,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_change_feed() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_change_feed_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

