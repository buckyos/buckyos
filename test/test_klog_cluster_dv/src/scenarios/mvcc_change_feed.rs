async fn run_local_gateway_mvcc_change_feed_failover_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-change-feed-failover-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_CHANGE_FEED_FAILOVER_MODE, route_prefix, 3)
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
        .cloned()
        .ok_or_else(|| format!("old leader node {} not found", old_leader_id))?;
    let source = nodes
        .iter()
        .find(|node| node.id != old_leader_id)
        .ok_or_else(|| format!("missing non-leader source node: leader={}", old_leader_id))?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let old_leader_gateway_addr = gateway_addr(&old_leader, ingress_port);
    let suffix = unique_suffix("mvcc-change-feed-failover");
    let prefix = format!("test/klog_mvcc_change_feed_failover_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);
    let key_c = format!("{}c", prefix);

    let a_v1 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        key_a.as_str(),
        "a-v1",
        Some(0),
    )
    .await?;
    let r1 = a_v1.mod_revision;
    if a_v1.create_revision != r1 || a_v1.version != 1 {
        return Err(format!(
            "unexpected change-feed failover a_v1 response: {:?}",
            a_v1
        ));
    }

    let waiter_client = client.clone();
    let waiter_gateway_addr = old_leader_gateway_addr.clone();
    let waiter_route_prefix = route_prefix.to_string();
    let waiter_node_name = old_leader.name.clone();
    let waiter_prefix = prefix.clone();
    let wait_started = std::time::Instant::now();
    let wait_task = tokio::spawn(async move {
        query_meta_changes_with_wait_via_cluster_inter_route(
            &waiter_client,
            waiter_gateway_addr.as_str(),
            waiter_route_prefix.as_str(),
            waiter_node_name.as_str(),
            waiter_prefix.as_str(),
            r1 + 1,
            8,
            None,
            Some(1_800),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
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
    let new_leader_gateway_addr = gateway_addr(new_leader, ingress_port);
    let failover_writer_gateway_addr = gateway_addr(failover_writer, ingress_port);

    let b_v1 = put_meta_via_cluster_inter_route(
        &client,
        failover_writer_gateway_addr.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_b.as_str(),
        "b-v1",
        Some(0),
    )
    .await?;
    let r2 = b_v1.mod_revision;
    if b_v1.create_revision != r2 || b_v1.version != 1 || r2 != r1 + 1 {
        return Err(format!(
            "unexpected change-feed failover b_v1 response: {:?}",
            b_v1
        ));
    }

    let wait_outcome = match wait_task
        .await
        .map_err(|err| format!("change-feed failover long-poll task join failed: {}", err))?
    {
        Ok(waited) => {
            require_meta_changes(&waited, &[(r2, &key_b, "b-v1", false, r2, 1)])?;
            "continued"
        }
        Err(err) => {
            let retried = query_meta_changes_with_wait_via_cluster_inter_route(
                &client,
                new_leader_gateway_addr.as_str(),
                route_prefix,
                failover_writer.name.as_str(),
                prefix.as_str(),
                r1 + 1,
                8,
                None,
                Some(1_500),
            )
            .await
            .map_err(|retry_err| {
                format!(
                    "long-poll failed during leader switch and resume also failed: initial={}, retry={}",
                    err, retry_err
                )
            })?;
            require_meta_changes(&retried, &[(r2, &key_b, "b-v1", false, r2, 1)])?;
            "resumed"
        }
    };
    let wait_elapsed = wait_started.elapsed();

    let cursor_page = query_meta_changes_via_cluster_inter_route(
        &client,
        new_leader_gateway_addr.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        prefix.as_str(),
        r1,
        1,
        None,
    )
    .await?;
    if !cursor_page.has_more || cursor_page.next_cursor.is_none() {
        return Err(format!(
            "change-feed failover cursor page did not produce cursor: {:?}",
            cursor_page
        ));
    }
    require_meta_changes(&cursor_page, &[(r1, &key_a, "a-v1", false, r1, 1)])?;
    let compacted_cursor = cursor_page
        .next_cursor
        .clone()
        .ok_or_else(|| "missing compacted change-feed cursor".to_string())?;

    let compacted = post_meta_compact_via_admin_route(
        &client,
        new_leader_gateway_addr.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        r1,
    )
    .await?;
    if compacted.compacted_revision != r1 || compacted.current_revision != r2 {
        return Err(format!(
            "unexpected change-feed failover compaction response: {:?}, expected compacted={}, current={}",
            compacted, r1, r2
        ));
    }

    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        failover_writer_gateway_addr.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        prefix.as_str(),
        r1,
        Some(&compacted_cursor),
        Some(600),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        new_leader_gateway_addr.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        prefix.as_str(),
        r1,
        None,
        Some(600),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let post_compact_client = client.clone();
    let post_compact_gateway_addr = failover_writer_gateway_addr.clone();
    let post_compact_route_prefix = route_prefix.to_string();
    let post_compact_node_name = failover_writer.name.clone();
    let post_compact_prefix = prefix.clone();
    let post_compact_wait_started = std::time::Instant::now();
    let post_compact_wait_task = tokio::spawn(async move {
        query_meta_changes_with_wait_via_cluster_inter_route(
            &post_compact_client,
            post_compact_gateway_addr.as_str(),
            post_compact_route_prefix.as_str(),
            post_compact_node_name.as_str(),
            post_compact_prefix.as_str(),
            r2 + 1,
            8,
            None,
            Some(1_800),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    let c_v1 = put_meta_via_cluster_inter_route(
        &client,
        new_leader_gateway_addr.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        key_c.as_str(),
        "c-v1",
        Some(0),
    )
    .await?;
    let r3 = c_v1.mod_revision;
    if c_v1.create_revision != r3 || c_v1.version != 1 || r3 != r2 + 1 {
        return Err(format!(
            "unexpected change-feed failover c_v1 response: {:?}",
            c_v1
        ));
    }
    let post_compact_waited = post_compact_wait_task
        .await
        .map_err(|err| format!("post-compact long-poll task join failed: {}", err))??;
    let post_compact_wait_elapsed = post_compact_wait_started.elapsed();
    if post_compact_wait_elapsed >= Duration::from_millis(1_600) {
        return Err(format!(
            "post-compact long-poll did not return promptly after write: elapsed_ms={}, response={:?}",
            post_compact_wait_elapsed.as_millis(),
            post_compact_waited
        ));
    }
    require_meta_changes(&post_compact_waited, &[(r3, &key_c, "c-v1", false, r3, 1)])?;

    for node in &alive_nodes {
        let gateway = gateway_addr(node, ingress_port);
        expect_meta_changes_status_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            r1,
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        let changes = query_meta_changes_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            r2,
            8,
            None,
        )
        .await?;
        require_meta_changes(
            &changes,
            &[
                (r2, &key_b, "b-v1", false, r2, 1),
                (r3, &key_c, "c-v1", false, r3, 1),
            ],
        )?;
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
                (&key_a, "a-v1", r1, r1, 1),
                (&key_b, "b-v1", r2, r2, 1),
                (&key_c, "c-v1", r3, r3, 1),
            ],
        )?;
    }

    let old_leader_config = configs
        .get(&old_leader_id)
        .ok_or_else(|| format!("missing config for old leader {}", old_leader_id))?;
    spawn_klog(harness, &klog_daemon_bin, old_leader_config, &old_leader)?;
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
        let gateway = gateway_addr(node, ingress_port);
        expect_meta_changes_status_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            r1,
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
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
                (&key_a, "a-v1", r1, r1, 1),
                (&key_b, "b-v1", r2, r2, 1),
                (&key_c, "c-v1", r3, r3, 1),
            ],
        )?;
    }

    println!(
        "[klog-cluster-dv] MVCC change-feed failover ok: old_leader={}, new_leader={}, wait_outcome={}, wait_ms={}, post_compact_wait_ms={}, compacted={}, revisions=[{},{},{}], prefix={}",
        old_leader_id,
        new_leader_id,
        wait_outcome,
        wait_elapsed.as_millis(),
        post_compact_wait_elapsed.as_millis(),
        compacted.compacted_revision,
        r1,
        r2,
        r3,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_change_feed_failover() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_change_feed_failover_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

#[derive(Debug, Clone)]
struct StressKeyState {
    key: String,
    value: String,
    create_revision: u64,
    mod_revision: u64,
    version: u64,
}

async fn run_local_gateway_mvcc_change_feed_stress_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let key_count = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_KEYS,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_KEYS,
    )?;
    let concurrency = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_CONCURRENCY,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_CONCURRENCY,
    )?;
    let rounds = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_ROUNDS,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_ROUNDS,
    )?;
    let page_limit = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT,
    )?;
    let round_delay_ms = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_ROUND_DELAY_MS,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_ROUND_DELAY_MS,
    )?;
    if key_count < 8 {
        return Err(format!(
            "{} must be at least 8, got {}",
            ENV_MVCC_CHANGE_FEED_STRESS_KEYS, key_count
        ));
    }
    if concurrency == 0 {
        return Err(format!(
            "{} must be greater than 0",
            ENV_MVCC_CHANGE_FEED_STRESS_CONCURRENCY
        ));
    }
    if rounds == 0 {
        return Err(format!(
            "{} must be greater than 0",
            ENV_MVCC_CHANGE_FEED_STRESS_ROUNDS
        ));
    }
    if page_limit < 2 {
        return Err(format!(
            "{} must be at least 2, got {}",
            ENV_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT, page_limit
        ));
    }

    let route_prefix = "/.cluster/klog-it-mvcc-change-feed-stress-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_CHANGE_FEED_STRESS_MODE, route_prefix, 3).await?;
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
    let suffix = unique_suffix("mvcc-change-feed-stress");
    let prefix = format!("test/klog_mvcc_change_feed_stress_dv/{}/", suffix);
    let stress_started = std::time::Instant::now();

    let mut expected_changes = Vec::new();
    let mut states: Vec<Option<StressKeyState>> = vec![None; key_count];
    for batch_start in (0..key_count).step_by(concurrency) {
        let batch_end = (batch_start + concurrency).min(key_count);
        let mut tasks = Vec::new();
        for index in batch_start..batch_end {
            let client = client.clone();
            let source = nodes[index % nodes.len()].clone();
            let target = nodes[(index + 1) % nodes.len()].clone();
            let gateway_addr = gateway_addr(&source, ingress_port);
            let route_prefix = route_prefix.to_string();
            let key = format!("{}key-{:04}", prefix, index);
            let value = format!("create-{:04}", index);
            tasks.push(tokio::spawn(async move {
                let stored = put_meta_via_cluster_inter_route(
                    &client,
                    gateway_addr.as_str(),
                    route_prefix.as_str(),
                    target.name.as_str(),
                    key.as_str(),
                    value.as_str(),
                    Some(0),
                )
                .await?;
                Ok::<_, String>((index, key, value, stored))
            }));
        }

        for task in tasks {
            let (index, key, value, stored) = task
                .await
                .map_err(|err| format!("stress create task join failed: {}", err))??;
            if stored.create_revision != stored.mod_revision || stored.version != 1 {
                return Err(format!("unexpected stress create response: {:?}", stored));
            }
            expected_changes.push(ExpectedMetaChange {
                revision: stored.mod_revision,
                key: key.clone(),
                value: value.clone(),
                deleted: false,
                create_revision: stored.create_revision,
                version: stored.version,
            });
            states[index] = Some(StressKeyState {
                key,
                value,
                create_revision: stored.create_revision,
                mod_revision: stored.mod_revision,
                version: stored.version,
            });
        }
    }
    let mut states = states
        .into_iter()
        .map(|state| state.ok_or_else(|| "missing stress key state after create".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let first_revision = expected_changes
        .iter()
        .map(|change| change.revision)
        .min()
        .ok_or_else(|| "missing first stress revision".to_string())?;

    for round in 1..=rounds {
        for batch_start in (0..key_count).step_by(concurrency) {
            let batch_end = (batch_start + concurrency).min(key_count);
            let mut tasks = Vec::new();
            for (index, state) in states
                .iter()
                .enumerate()
                .skip(batch_start)
                .take(batch_end - batch_start)
            {
                let client = client.clone();
                let source = nodes[(index + round) % nodes.len()].clone();
                let target = nodes[(index + round + 1) % nodes.len()].clone();
                let gateway_addr = gateway_addr(&source, ingress_port);
                let route_prefix = route_prefix.to_string();
                let key = state.key.clone();
                let value = format!("round-{:02}-key-{:04}", round, index);
                let expected_revision = state.mod_revision;
                tasks.push(tokio::spawn(async move {
                    let stored = put_meta_via_cluster_inter_route(
                        &client,
                        gateway_addr.as_str(),
                        route_prefix.as_str(),
                        target.name.as_str(),
                        key.as_str(),
                        value.as_str(),
                        Some(expected_revision),
                    )
                    .await?;
                    Ok::<_, String>((index, value, stored))
                }));
            }

            for task in tasks {
                let (index, value, stored) = task
                    .await
                    .map_err(|err| format!("stress update task join failed: {}", err))??;
                let state = states
                    .get_mut(index)
                    .ok_or_else(|| format!("missing stress state for update index {}", index))?;
                if stored.create_revision != state.create_revision
                    || stored.version != state.version + 1
                {
                    return Err(format!(
                        "unexpected stress update response: state={:?}, stored={:?}",
                        state, stored
                    ));
                }
                state.value = value.clone();
                state.mod_revision = stored.mod_revision;
                state.version = stored.version;
                expected_changes.push(ExpectedMetaChange {
                    revision: stored.mod_revision,
                    key: state.key.clone(),
                    value,
                    deleted: false,
                    create_revision: state.create_revision,
                    version: state.version,
                });
            }
        }

        if round_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(round_delay_ms as u64)).await;
        }
    }

    let delete_count = (key_count / 4).max(2);
    for batch_start in (0..delete_count).step_by(concurrency) {
        let batch_end = (batch_start + concurrency).min(delete_count);
        let mut tasks = Vec::new();
        for (index, state) in states
            .iter()
            .enumerate()
            .take(delete_count)
            .skip(batch_start)
            .take(batch_end - batch_start)
        {
            let client = client.clone();
            let source = nodes[(index + rounds + 1) % nodes.len()].clone();
            let target = nodes[(index + rounds + 2) % nodes.len()].clone();
            let gateway_addr = gateway_addr(&source, ingress_port);
            let route_prefix = route_prefix.to_string();
            let key = state.key.clone();
            tasks.push(tokio::spawn(async move {
                let deleted = delete_meta_via_cluster_inter_route(
                    &client,
                    gateway_addr.as_str(),
                    route_prefix.as_str(),
                    target.name.as_str(),
                    key.as_str(),
                )
                .await?;
                Ok::<_, String>((index, deleted))
            }));
        }

        for task in tasks {
            let (index, deleted) = task
                .await
                .map_err(|err| format!("stress delete task join failed: {}", err))??;
            let state = states
                .get(index)
                .ok_or_else(|| format!("missing stress state for delete index {}", index))?;
            let version = deleted
                .meta_version
                .as_ref()
                .ok_or_else(|| format!("missing stress delete meta_version: {:?}", deleted))?;
            require_meta_version(
                Some(version),
                state.create_revision,
                version.mod_revision,
                0,
                true,
            )?;
            expected_changes.push(ExpectedMetaChange {
                revision: version.mod_revision,
                key: state.key.clone(),
                value: state.value.clone(),
                deleted: true,
                create_revision: state.create_revision,
                version: 0,
            });
        }
    }

    for batch_start in (0..delete_count).step_by(concurrency) {
        let batch_end = (batch_start + concurrency).min(delete_count);
        let mut tasks = Vec::new();
        for (index, state) in states
            .iter()
            .enumerate()
            .take(delete_count)
            .skip(batch_start)
            .take(batch_end - batch_start)
        {
            let client = client.clone();
            let source = nodes[(index + rounds + 2) % nodes.len()].clone();
            let target = nodes[(index + rounds) % nodes.len()].clone();
            let gateway_addr = gateway_addr(&source, ingress_port);
            let route_prefix = route_prefix.to_string();
            let key = state.key.clone();
            let value = format!("recreate-key-{:04}", index);
            tasks.push(tokio::spawn(async move {
                let stored = put_meta_via_cluster_inter_route(
                    &client,
                    gateway_addr.as_str(),
                    route_prefix.as_str(),
                    target.name.as_str(),
                    key.as_str(),
                    value.as_str(),
                    Some(0),
                )
                .await?;
                Ok::<_, String>((index, value, stored))
            }));
        }

        for task in tasks {
            let (index, value, stored) = task
                .await
                .map_err(|err| format!("stress recreate task join failed: {}", err))??;
            if stored.create_revision != stored.mod_revision || stored.version != 1 {
                return Err(format!("unexpected stress recreate response: {:?}", stored));
            }
            let state = states
                .get_mut(index)
                .ok_or_else(|| format!("missing stress state for recreate index {}", index))?;
            state.value = value.clone();
            state.create_revision = stored.create_revision;
            state.mod_revision = stored.mod_revision;
            state.version = stored.version;
            expected_changes.push(ExpectedMetaChange {
                revision: stored.mod_revision,
                key: state.key.clone(),
                value,
                deleted: false,
                create_revision: state.create_revision,
                version: state.version,
            });
        }
    }

    expected_changes.sort_by(|left, right| {
        left.revision
            .cmp(&right.revision)
            .then_with(|| left.key.cmp(&right.key))
    });

    let source = nodes
        .first()
        .ok_or_else(|| "missing stress source node".to_string())?;
    let target = nodes
        .get(1)
        .ok_or_else(|| "missing stress target node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let target_gateway_addr = gateway_addr(target, ingress_port);
    let mut cursor = None;
    let mut page_sizes = Vec::new();
    let mut collected = Vec::with_capacity(expected_changes.len());
    loop {
        let page = query_meta_changes_via_cluster_inter_route(
            &client,
            source_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            prefix.as_str(),
            first_revision,
            page_limit,
            cursor.as_ref(),
        )
        .await?;
        if page.items.is_empty() && page.has_more {
            return Err(format!(
                "stress change-feed returned empty page with has_more=true: {:?}",
                page
            ));
        }
        page_sizes.push(page.items.len());
        collected.extend(page.items.iter().cloned());
        if !page.has_more {
            if page.next_start_revision
                <= expected_changes
                    .last()
                    .ok_or_else(|| "missing last expected change".to_string())?
                    .revision
            {
                return Err(format!(
                    "stress change-feed next_start_revision did not advance: page={:?}",
                    page
                ));
            }
            break;
        }
        let next_cursor = page
            .next_cursor
            .ok_or_else(|| "stress change-feed missing next_cursor".to_string())?;
        if cursor.as_ref() == Some(&next_cursor) {
            return Err(format!(
                "stress change-feed cursor did not advance: {:?}",
                next_cursor
            ));
        }
        cursor = Some(next_cursor);
        if page_sizes.len() > expected_changes.len() + 2 {
            return Err(format!(
                "stress change-feed pagination exceeded expected pages: sizes={:?}",
                page_sizes
            ));
        }
    }
    require_expected_meta_changes(&collected, &expected_changes)?;

    let cursor_page = query_meta_changes_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        first_revision,
        page_limit,
        None,
    )
    .await?;
    if !cursor_page.has_more {
        return Err(format!(
            "stress change-feed first cursor page unexpectedly has no more pages: {:?}",
            cursor_page
        ));
    }
    let compact_cursor = cursor_page
        .next_cursor
        .clone()
        .ok_or_else(|| "stress change-feed first page missing cursor".to_string())?;
    let compact_revision = compact_cursor.revision;
    let compacted = post_meta_compact_via_admin_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        compact_revision,
    )
    .await?;
    let last_revision = expected_changes
        .last()
        .ok_or_else(|| "missing last stress revision".to_string())?
        .revision;
    if compacted.compacted_revision != compact_revision
        || compacted.current_revision < last_revision
    {
        return Err(format!(
            "unexpected stress compaction response: {:?}, compact_revision={}, last_revision={}",
            compacted, compact_revision, last_revision
        ));
    }
    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        first_revision,
        Some(&compact_cursor),
        Some(500),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let expected_after_compact = expected_changes
        .iter()
        .filter(|change| change.revision > compact_revision)
        .cloned()
        .collect::<Vec<_>>();
    let mut post_compact_cursor = None;
    let mut post_compact_collected = Vec::with_capacity(expected_after_compact.len());
    loop {
        let page = query_meta_changes_with_wait_via_cluster_inter_route(
            &client,
            source_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            prefix.as_str(),
            compact_revision + 1,
            page_limit,
            post_compact_cursor.as_ref(),
            Some(500),
        )
        .await?;
        post_compact_collected.extend(page.items.iter().cloned());
        if !page.has_more {
            break;
        }
        post_compact_cursor = page.next_cursor;
        if post_compact_cursor.is_none() {
            return Err("stress post-compact page missing cursor".to_string());
        }
    }
    require_expected_meta_changes(&post_compact_collected, &expected_after_compact)?;

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            key_count + 8,
        )
        .await?;
        if current.items.len() != key_count {
            return Err(format!(
                "stress current key count mismatch on {}: expected={}, actual={}, items={:?}",
                node.name,
                key_count,
                current.items.len(),
                current.items
            ));
        }
        let samples = [
            0usize,
            delete_count.saturating_sub(1),
            delete_count,
            key_count / 2,
            key_count - 1,
        ];
        let mut expected_samples = Vec::new();
        for index in samples {
            let state = states
                .get(index)
                .ok_or_else(|| format!("missing stress sample state {}", index))?;
            expected_samples.push((
                state.key.as_str(),
                state.value.as_str(),
                state.create_revision,
                state.mod_revision,
                state.version,
            ));
        }
        require_meta_selected_values(&current, expected_samples.as_slice())?;
    }

    println!(
        "[klog-cluster-dv] MVCC change-feed stress ok: leader={}, keys={}, concurrency={}, rounds={}, delete_recreate={}, changes={}, pages={:?}, compact_revision={}, post_compact_changes={}, elapsed_ms={}, prefix={}",
        leader_id,
        key_count,
        concurrency,
        rounds,
        delete_count,
        expected_changes.len(),
        page_sizes,
        compact_revision,
        expected_after_compact.len(),
        stress_started.elapsed().as_millis(),
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_change_feed_stress() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_change_feed_stress_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

