async fn run_local_gateway_system_config_kv_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_KV_MODE, route_prefix, 3).await?;
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
    let source = nodes
        .first()
        .ok_or_else(|| "missing source gateway node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target gateway node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let suffix = unique_suffix("syscfg");
    let key_prefix = format!("test/system_config_kv/{}/", suffix);
    let boot_key = format!("{}boot/config", key_prefix);
    let node_config_key = format!("{}nodes/ood1/config", key_prefix);
    let device_info_key = format!("{}devices/ood1/info", key_prefix);
    let deleted_key = format!("{}nodes/ood2/config", key_prefix);

    let boot_value_v1 = r#"{"oods":["ood1","ood2","ood3"],"revision":1}"#;
    let boot_created = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        boot_key.as_str(),
        boot_value_v1,
        Some(0),
    )
    .await?;
    if boot_created.revision != 1 {
        return Err(format!(
            "system-config create expected revision 1, got {}",
            boot_created.revision
        ));
    }
    expect_meta_put_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        MetaPutRequest {
            key: boot_key.clone(),
            value: boot_value_v1.to_string(),
            node_name: Some(target.name.clone()),
            expected_revision: Some(0),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let boot_value_v2 = r#"{"oods":["ood1","ood2","ood3"],"revision":2}"#;
    let boot_updated = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        boot_key.as_str(),
        boot_value_v2,
        Some(boot_created.revision),
    )
    .await?;
    if boot_updated.revision != 2 {
        return Err(format!(
            "system-config update expected revision 2, got {}",
            boot_updated.revision
        ));
    }
    expect_meta_put_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        MetaPutRequest {
            key: boot_key.clone(),
            value: r#"{"stale":true}"#.to_string(),
            node_name: Some(target.name.clone()),
            expected_revision: Some(boot_created.revision),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let fetched = query_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        boot_key.as_str(),
    )
    .await?;
    require_meta_value(
        &fetched,
        boot_key.as_str(),
        boot_value_v2,
        boot_updated.revision,
    )?;

    let node_value = r#"{"kernel":{"scheduler":{},"verify-hub":{}}}"#;
    let device_value = r#"{"name":"ood1","device_type":"node"}"#;
    let deleted_value = r#"{"kernel":{"scheduler":{}}}"#;
    put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        node_config_key.as_str(),
        node_value,
        Some(0),
    )
    .await?;
    put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        device_info_key.as_str(),
        device_value,
        Some(0),
    )
    .await?;
    put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        deleted_key.as_str(),
        deleted_value,
        Some(0),
    )
    .await?;

    let listed = query_meta_prefix_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        key_prefix.as_str(),
        16,
    )
    .await?;
    require_meta_keys(
        &listed,
        &[
            boot_key.as_str(),
            node_config_key.as_str(),
            device_info_key.as_str(),
            deleted_key.as_str(),
        ],
    )?;

    let deleted = delete_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        deleted_key.as_str(),
    )
    .await?;
    if deleted.key != deleted_key
        || !deleted.existed
        || deleted.prev_meta.as_ref().map(|item| item.value.as_str()) != Some(deleted_value)
    {
        return Err(format!("unexpected meta delete result: {:?}", deleted));
    }
    let deleted_query = query_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        deleted_key.as_str(),
    )
    .await?;
    if !deleted_query.items.is_empty() {
        return Err(format!(
            "deleted system-config key still visible: items={:?}",
            deleted_query.items
        ));
    }

    println!(
        "[klog-cluster-dv] system-config kv semantics ok: leader={}, source_gateway={}, target_node={}, prefix={}",
        leader_id, source.name, target.name, key_prefix
    );
    Ok(())
}

async fn run_local_gateway_system_config_kv() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_kv_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

fn collect_used_ports(nodes: &[LocalNodeDef], ingress_port: u16) -> BTreeSet<u16> {
    let mut used_ports = BTreeSet::from([ingress_port]);
    for node in nodes {
        used_ports.insert(node.ports.raft);
        used_ports.insert(node.ports.inter);
        used_ports.insert(node.ports.admin);
        used_ports.insert(node.ports.rpc);
        used_ports.insert(node.ports.rtcp);
        used_ports.insert(node.ports.zone_http);
        used_ports.insert(node.ports.control);
    }
    used_ports
}

async fn run_local_gateway_system_config_service_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-service-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_SERVICE_MODE, route_prefix, 3).await?;
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
    let klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        leader.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );

    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let system_config_port = pick_local_port(&mut used_ports)?;
    spawn_system_config(
        harness,
        &system_config_bin,
        system_config_port,
        klog_endpoint.as_str(),
    )?;
    wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;

    let endpoint = format!(
        "http://127.0.0.1:{}{}",
        system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let user_token = system_config_jwt(TEST_DEVICE_NAME, "root", "scheduler")?;
    let scheduler_token = system_config_jwt(TEST_DEVICE_NAME, "alice", "scheduler")?;
    let suffix = unique_suffix("syscfg-service");
    let base = format!("users/alice/klog_service_dv/{}", suffix);
    let profile_key = format!("{}/profile", base);
    let notes_key = format!("{}/notes", base);
    let tx_key1 = format!("{}/tx/key1", base);
    let tx_key2 = format!("{}/tx/key2", base);
    let stale_key = format!("{}/tx/stale", base);

    let profile_v1 = r#"{"name":"v1","flags":{"enabled":false}}"#;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_create",
        json!({"key": profile_key, "value": profile_v1}),
    )
    .await?;
    let created = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": profile_key}),
    )
    .await?;
    require_system_config_value(&created, profile_v1, 1)?;
    expect_system_config_rpc_error(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_create",
        json!({"key": profile_key, "value": profile_v1}),
    )
    .await?;

    let profile_v2 = r#"{"name":"v2","flags":{"enabled":false}}"#;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_set",
        json!({"key": profile_key, "value": profile_v2}),
    )
    .await?;
    let set = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": profile_key}),
    )
    .await?;
    require_system_config_value(&set, profile_v2, 1)?;

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_set_by_json_path",
        json!({"key": profile_key, "json_path": "/flags/enabled", "value": "true"}),
    )
    .await?;
    let path_updated = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": profile_key}),
    )
    .await?;
    let path_updated_version = path_updated
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing version after json path update: {}", path_updated))?;
    let path_updated_value: Value = serde_json::from_str(
        path_updated
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing value after json path update: {}", path_updated))?,
    )
    .map_err(|err| format!("failed to parse profile json value: {}", err))?;
    if path_updated_value
        .pointer("/flags/enabled")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(format!(
            "json path update was not visible: {}",
            path_updated_value
        ));
    }

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_set",
        json!({"key": profile_key, "value": profile_v2}),
    )
    .await?;
    let mut stale_actions = serde_json::Map::new();
    stale_actions.insert(
        stale_key.clone(),
        json!({
            "action": "create",
            "value": "should-not-exist"
        }),
    );
    expect_system_config_rpc_error(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", profile_key, path_updated_version),
            "actions": stale_actions
        }),
    )
    .await?;
    let stale = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": stale_key}),
    )
    .await?;
    if !stale.is_null() {
        return Err(format!(
            "stale guarded exec_tx left partial state: {}",
            stale
        ));
    }

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_create",
        json!({"key": notes_key, "value": "hello"}),
    )
    .await?;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_append",
        json!({"key": notes_key, "append_value": " world"}),
    )
    .await?;
    let notes = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": notes_key}),
    )
    .await?;
    require_system_config_value(&notes, "hello world", 1)?;

    let mut tx_actions = serde_json::Map::new();
    tx_actions.insert(
        tx_key1.clone(),
        json!({
            "action": "create",
            "value": "tx-value-1"
        }),
    );
    tx_actions.insert(
        tx_key2.clone(),
        json!({
            "action": "create",
            "value": "tx-value-2"
        }),
    );
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_exec_tx",
        json!({"actions": tx_actions}),
    )
    .await?;
    let tx1 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1}),
    )
    .await?;
    require_system_config_value(&tx1, "tx-value-1", 1)?;
    let tx2 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": tx_key2}),
    )
    .await?;
    require_system_config_value(&tx2, "tx-value-2", 1)?;

    let listed = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_list",
        json!({"key": base}),
    )
    .await?;
    let listed = listed
        .as_array()
        .ok_or_else(|| format!("system_config list result is not array: {}", listed))?;
    for expected_child in ["profile", "notes", "tx"] {
        if !listed
            .iter()
            .any(|value| value.as_str() == Some(expected_child))
        {
            return Err(format!(
                "system_config list missing child {}: {:?}",
                expected_child, listed
            ));
        }
    }

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_delete",
        json!({"key": notes_key}),
    )
    .await?;
    let deleted = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": notes_key}),
    )
    .await?;
    if !deleted.is_null() {
        return Err(format!("deleted key is still visible: {}", deleted));
    }

    let scheduler_dump = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        scheduler_token.as_str(),
        "dump_configs_for_scheduler",
        json!({}),
    )
    .await?;
    if scheduler_dump.get(profile_key.as_str()).is_none()
        || scheduler_dump.get(tx_key1.as_str()).is_none()
    {
        return Err(format!(
            "scheduler dump missing klog-backed system_config keys: {}",
            scheduler_dump
        ));
    }

    println!(
        "[klog-cluster-dv] system_config service klog backend ok: leader={}, endpoint={}, prefix={}",
        leader_id, endpoint, base
    );
    Ok(())
}

async fn run_local_gateway_system_config_service() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_service_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_leader_failover_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-leader-failover-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_LEADER_FAILOVER_MODE, route_prefix, 3)
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
        .ok_or_else(|| "missing system_config failover seed node".to_string())?;
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
        Duration::from_secs(60),
    )
    .await?;
    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .cloned()
        .ok_or_else(|| format!("system_config failover leader {} not found", old_leader_id))?;
    let endpoint_node = nodes
        .iter()
        .find(|node| node.id != old_leader_id)
        .cloned()
        .ok_or_else(|| "missing non-leader klog RPC endpoint node".to_string())?;
    let survivors = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();

    let klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        endpoint_node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let system_config_port = pick_local_port(&mut used_ports)?;
    spawn_system_config(
        harness,
        &system_config_bin,
        system_config_port,
        klog_endpoint.as_str(),
    )?;
    wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;

    let endpoint = format!(
        "http://127.0.0.1:{}{}",
        system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let token = system_config_jwt(TEST_DEVICE_NAME, "root", "scheduler")?;
    let suffix = unique_suffix("syscfg-leader-failover");
    let base = format!("users/alice/klog_leader_failover_dv/{}", suffix);
    let prefix = format!("{}/", base);
    let profile_key = format!("{}profile", prefix);
    let tx_key = format!("{}tx/key", prefix);
    let profile_v1 = "profile-before-failover-v1";
    let profile_v2 = "profile-before-failover-v2";
    let profile_during_failover = "profile-during-failover";
    let profile_v3 = "profile-after-failover-v3";
    let profile_v4 = "profile-after-rejoin-v4";

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_create",
        json!({"key": profile_key.as_str(), "value": profile_v1}),
    )
    .await?;
    let created = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r1) = system_config_value_and_version(&created)?;
    require_system_config_value(&created, profile_v1, r1)?;

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set",
        json!({"key": profile_key.as_str(), "value": profile_v2}),
    )
    .await?;
    let before_failover = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r2) = system_config_value_and_version(&before_failover)?;
    if r2 <= r1 {
        return Err(format!(
            "system_config pre-failover set revision did not advance: r1={}, r2={}",
            r1, r2
        ));
    }
    require_system_config_value(&before_failover, profile_v2, r2)?;

    harness.stop(format!("klog-{}", old_leader.name).as_str())?;
    let failover_err = expect_system_config_rpc_error(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set",
        json!({"key": profile_key.as_str(), "value": profile_during_failover}),
    )
    .await?;
    require_system_config_klog_failover_error(failover_err.as_str())?;

    let new_leader_id = wait_consistent_leader(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(90),
    )
    .await?;
    wait_membership(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let after_failed_write = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    require_system_config_value(&after_failed_write, profile_v2, r2)?;

    wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set",
        json!({"key": profile_key.as_str(), "value": profile_v3}),
        Duration::from_secs(40),
    )
    .await?;
    let after_retry = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    let (_, r3) = system_config_value_and_version(&after_retry)?;
    if r3 <= r2 {
        return Err(format!(
            "system_config post-failover retry revision did not advance: r2={}, r3={}",
            r2, r3
        ));
    }
    require_system_config_value(&after_retry, profile_v3, r3)?;

    let mut tx_actions = serde_json::Map::new();
    tx_actions.insert(
        profile_key.clone(),
        json!({
            "action": "update",
            "value": profile_v4
        }),
    );
    tx_actions.insert(
        tx_key.clone(),
        json!({
            "action": "create",
            "value": "tx-after-failover"
        }),
    );
    wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", profile_key, r3),
            "actions": tx_actions
        }),
        Duration::from_secs(40),
    )
    .await?;
    let after_tx = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    let (_, r4) = system_config_value_and_version(&after_tx)?;
    if r4 <= r3 {
        return Err(format!(
            "system_config post-failover tx revision did not advance: r3={}, r4={}",
            r3, r4
        ));
    }
    require_system_config_value(&after_tx, profile_v4, r4)?;
    let tx_after_failover = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    require_system_config_value(&tx_after_failover, "tx-after-failover", r4)?;

    let old_leader_config = configs
        .get(&old_leader.id)
        .ok_or_else(|| format!("missing old leader config {}", old_leader.id))?;
    spawn_klog(harness, &klog_daemon_bin, old_leader_config, &old_leader)?;
    wait_tcp("127.0.0.1", old_leader.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", old_leader.ports.rpc, Duration::from_secs(12)).await?;
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
    let final_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    let after_rejoin = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    require_system_config_value(&after_rejoin, profile_v4, r4)?;

    for node in &nodes {
        let response = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            8,
        )
        .await?;
        require_meta_selected_values(
            &response,
            &[
                (profile_key.as_str(), profile_v4, r1, r4, 4),
                (tx_key.as_str(), "tx-after-failover", r4, r4, 1),
            ],
        )?;
    }

    println!(
        "[klog-cluster-dv] system_config leader failover ok: old_leader={}, new_leader={}, final_leader={}, endpoint_node={}, endpoint={}, failover_error_len={}, revisions=[{},{},{},{}], prefix={}",
        old_leader_id,
        new_leader_id,
        final_leader_id,
        endpoint_node.id,
        endpoint,
        failover_err.len(),
        r1,
        r2,
        r3,
        r4,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_system_config_leader_failover() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_leader_failover_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_abnormal_inner(harness: &mut LocalHarness) -> Result<(), String> {
    if LocalGatewayRouteMode::from_env()? != LocalGatewayRouteMode::TargetGateway {
        return Err(format!(
            "{} requires {}=target-gateway",
            GATEWAY_ABNORMAL_MODE, KLOG_CLUSTER_DV_ROUTE_MODE_ENV
        ));
    }

    let route_prefix = "/.cluster/klog-it-gateway-abnormal-dv";
    let setup =
        prepare_local_gateway_setup(harness, GATEWAY_ABNORMAL_MODE, route_prefix, 3).await?;
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
        .ok_or_else(|| "missing gateway abnormal seed node".to_string())?;
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
    let source = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .cloned()
        .ok_or_else(|| format!("gateway abnormal leader {} not found", leader_id))?;
    let mut non_leaders = nodes
        .iter()
        .filter(|node| node.id != leader_id)
        .cloned()
        .collect::<Vec<_>>();
    if non_leaders.len() < 2 {
        return Err("gateway abnormal requires two non-leader nodes".to_string());
    }
    let stopped_victim = non_leaders.remove(0);
    let healthy_target = non_leaders.remove(0);
    let source_gateway_addr = gateway_addr(&source, ingress_port);
    let healthy_gateway_addr = gateway_addr(&healthy_target, ingress_port);
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let cyfs_gateway_bin = resolve_cyfs_gateway_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let stale_source = LocalNodeDef {
        id: 99,
        name: "client".to_string(),
        device_id: "did:dv:client".to_string(),
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
    if !reserve_port(stale_source.gateway_host.as_str(), ingress_port) {
        return Err(format!(
            "gateway abnormal stale source ingress is not free: {}:{}",
            stale_source.gateway_host, ingress_port
        ));
    }
    let stale_source_gateway_addr = gateway_addr(&stale_source, ingress_port);

    let suffix = unique_suffix("gateway-abnormal");
    let base = format!("gateway_abnormal_dv/{}/", suffix);
    let baseline_key = format!("{}baseline", base);
    let stopped_key = format!("{}target-gateway-stopped", base);
    let stale_key = format!("{}stale-route", base);
    let stale_recovery_key = format!("{}stale-route-recovered", base);

    let baseline = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        baseline_key.as_str(),
        "baseline-v1",
        None,
    )
    .await?;
    let baseline_query = query_meta_via_cluster_inter_route(
        &client,
        healthy_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        baseline_key.as_str(),
    )
    .await?;
    require_meta_value(
        &baseline_query,
        baseline_key.as_str(),
        "baseline-v1",
        baseline.revision,
    )?;

    harness.stop(format!("gateway-{}", stopped_victim.name).as_str())?;
    let stopped_err = expect_meta_put_error_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        stopped_victim.name.as_str(),
        stopped_key.as_str(),
        "must-not-write-while-target-gateway-stopped",
    )
    .await?;
    require_gateway_diagnostic_error(stopped_err.as_str(), "target gateway stopped data route")?;
    let stopped_query = query_meta_via_cluster_inter_route(
        &client,
        healthy_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        stopped_key.as_str(),
    )
    .await?;
    require_meta_key_absent(&stopped_query, stopped_key.as_str())?;

    let stale_gateway_options = GatewayRuntimeOptions {
        all_nodes: &nodes,
        ingress_port,
        route_prefix,
        route_mode: LocalGatewayRouteMode::TargetGateway,
    };
    let stale_gateway_config = write_gateway_runtime(
        harness,
        &repo_root,
        &buckyos_root,
        &stale_source,
        &stale_gateway_options,
    )?;
    patch_gateway_direct_route(
        harness,
        &stale_source,
        healthy_target.name.as_str(),
        "tcp:///127.0.0.250",
    )?;
    spawn_gateway(
        harness,
        &cyfs_gateway_bin,
        stale_gateway_config.as_path(),
        &stale_source,
    )?;
    wait_tcp(
        stale_source.gateway_host.as_str(),
        ingress_port,
        Duration::from_secs(8),
    )
    .await?;

    let stale_err = expect_meta_put_error_via_cluster_inter_route(
        &client,
        stale_source_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        stale_key.as_str(),
        "must-not-write-through-stale-route",
    )
    .await?;
    require_gateway_diagnostic_error(stale_err.as_str(), "stale route data route")?;
    let admin_err = match fetch_cluster_state_via_admin_route(
        &client,
        stale_source_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
    )
    .await
    {
        Ok(value) => Err(format!(
            "stale gateway admin route unexpectedly succeeded: {}",
            value
        )),
        Err(err) => Ok(err),
    }?;
    require_gateway_diagnostic_error(admin_err.as_str(), "stale route admin route")?;

    let stale_query = query_meta_via_cluster_inter_route(
        &client,
        healthy_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        stale_key.as_str(),
    )
    .await?;
    require_meta_key_absent(&stale_query, stale_key.as_str())?;
    wait_consistent_leader(
        &client,
        &[source.clone(), healthy_target.clone()],
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(60),
    )
    .await?;
    let stale_recovery = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        stale_recovery_key.as_str(),
        "stale-route-recovered-v1",
        None,
    )
    .await?;
    let current = query_meta_prefix_via_cluster_inter_route(
        &client,
        healthy_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        base.as_str(),
        16,
    )
    .await?;
    require_meta_key_absent(&current, stopped_key.as_str())?;
    require_meta_key_absent(&current, stale_key.as_str())?;
    require_meta_selected_values(
        &current,
        &[
            (
                baseline_key.as_str(),
                "baseline-v1",
                baseline.revision,
                baseline.revision,
                1,
            ),
            (
                stale_recovery_key.as_str(),
                "stale-route-recovered-v1",
                stale_recovery.revision,
                stale_recovery.revision,
                1,
            ),
        ],
    )?;

    println!(
        "[klog-cluster-dv] gateway abnormal ok: leader={}, source={}, stopped_victim={}, healthy_target={}, stale_source={}, stopped_error_len={}, stale_error_len={}, admin_error_len={}, prefix={}",
        leader_id,
        source.name,
        stopped_victim.name,
        healthy_target.name,
        stale_source.name,
        stopped_err.len(),
        stale_err.len(),
        admin_err.len(),
        base
    );
    Ok(())
}

async fn run_local_gateway_abnormal() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_abnormal_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_stale_config_rejoin_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-stale-config-rejoin-dv";
    let setup = prepare_local_gateway_setup(
        harness,
        SYSTEM_CONFIG_STALE_CONFIG_REJOIN_MODE,
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
        .ok_or_else(|| "missing system_config stale-config seed node".to_string())?;
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
    let initial_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let removed_node = nodes
        .iter()
        .find(|node| node.id != initial_leader_id)
        .cloned()
        .ok_or_else(|| {
            format!("failed to pick stale-config removed node, leader={initial_leader_id}")
        })?;
    let active_nodes = nodes
        .iter()
        .filter(|node| node.id != removed_node.id)
        .cloned()
        .collect::<Vec<_>>();
    if active_nodes.len() != 2 {
        return Err(format!(
            "expected two active nodes after removing {}, got {}",
            removed_node.id,
            active_nodes.len()
        ));
    }
    let active_voters = active_nodes.iter().map(|node| node.id).collect::<Vec<_>>();
    let active_system_node = active_nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing active system_config node".to_string())?;

    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let active_system_config_port = pick_local_port(&mut used_ports)?;
    let stale_system_config_port = pick_local_port(&mut used_ports)?;
    let active_root = harness.root.join("system-config-active-root");
    let stale_root = harness.root.join("system-config-stale-root");
    let active_klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        active_system_node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let stale_klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        removed_node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let active_token = system_config_jwt(active_system_node.name.as_str(), "root", "scheduler")?;
    let stale_token = system_config_jwt(removed_node.name.as_str(), "root", "scheduler")?;

    spawn_system_config_with_options(
        harness,
        "system-config-active-klog",
        &system_config_bin,
        active_root.as_path(),
        active_system_config_port,
        Some(active_klog_endpoint.as_str()),
        active_system_node.name.as_str(),
        false,
    )?;
    wait_tcp(
        "127.0.0.1",
        active_system_config_port,
        Duration::from_secs(15),
    )
    .await?;
    let active_endpoint = format!(
        "http://127.0.0.1:{}{}",
        active_system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let suffix = unique_suffix("syscfg-stale-config");
    let base = format!("users/alice/klog_stale_config_dv/{}", suffix);
    let before_key = format!("{}/before_shrink", base);
    let after_shrink_key = format!("{}/after_shrink", base);
    let stale_key = format!("{}/stale_write", base);
    let active_after_stale_key = format!("{}/active_after_stale", base);

    call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_create",
        json!({"key": before_key.as_str(), "value": "before-shrink"}),
    )
    .await?;
    let before_value = call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_get",
        json!({"key": before_key.as_str()}),
    )
    .await?;
    let (_, before_revision) = system_config_value_and_version(&before_value)?;
    require_system_config_value(&before_value, "before-shrink", before_revision)?;

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
    call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_create",
        json!({"key": after_shrink_key.as_str(), "value": "after-shrink"}),
    )
    .await?;

    let removed_config = configs
        .get(&removed_node.id)
        .ok_or_else(|| format!("missing stale removed node config {}", removed_node.id))?;
    spawn_klog(harness, &klog_daemon_bin, removed_config, &removed_node)?;
    wait_tcp(
        "127.0.0.1",
        removed_node.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp("127.0.0.1", removed_node.ports.rpc, Duration::from_secs(12)).await?;
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

    spawn_system_config_with_options(
        harness,
        "system-config-stale-klog",
        &system_config_bin,
        stale_root.as_path(),
        stale_system_config_port,
        Some(stale_klog_endpoint.as_str()),
        removed_node.name.as_str(),
        false,
    )?;
    wait_tcp(
        "127.0.0.1",
        stale_system_config_port,
        Duration::from_secs(15),
    )
    .await?;
    let stale_endpoint = format!(
        "http://127.0.0.1:{}{}",
        stale_system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let stale_write_err = expect_system_config_rpc_error(
        &client,
        stale_endpoint.as_str(),
        stale_token.as_str(),
        "sys_config_create",
        json!({"key": stale_key.as_str(), "value": "must-not-land-from-stale-config"}),
    )
    .await?;
    require_system_config_klog_failover_error(stale_write_err.as_str())?;
    tokio::time::sleep(Duration::from_secs(2)).await;
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
    let stale_from_active = call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_get",
        json!({"key": stale_key.as_str()}),
    )
    .await?;
    require_system_config_null(&stale_from_active, stale_key.as_str())?;

    call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_create",
        json!({"key": active_after_stale_key.as_str(), "value": "active-after-stale"}),
    )
    .await?;
    let active_after_stale = call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_get",
        json!({"key": active_after_stale_key.as_str()}),
    )
    .await?;
    let (_, active_after_stale_revision) = system_config_value_and_version(&active_after_stale)?;
    require_system_config_value(
        &active_after_stale,
        "active-after-stale",
        active_after_stale_revision,
    )?;

    let active_prefix = query_meta_prefix_via_cluster_inter_route(
        &client,
        gateway_addr(&active_system_node, ingress_port).as_str(),
        route_prefix,
        active_system_node.name.as_str(),
        format!("{}/", base).as_str(),
        16,
    )
    .await?;
    require_meta_key_absent(&active_prefix, stale_key.as_str())?;
    require_meta_keys(
        &active_prefix,
        &[
            before_key.as_str(),
            after_shrink_key.as_str(),
            active_after_stale_key.as_str(),
        ],
    )?;

    println!(
        "[klog-cluster-dv] system_config stale config rejoin ok: initial_leader={}, removed_node={}, shrink_leader={}, active_voters={:?}, active_endpoint={}, stale_endpoint={}, stale_error_len={}, prefix={}",
        initial_leader_id,
        removed_node.id,
        shrink_leader_id,
        active_voters,
        active_endpoint,
        stale_endpoint,
        stale_write_err.len(),
        base
    );
    Ok(())
}

async fn run_local_gateway_system_config_stale_config_rejoin() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_stale_config_rejoin_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_mvcc_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-mvcc-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_MVCC_MODE, route_prefix, 3).await?;
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
        .ok_or_else(|| "missing source gateway node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target gateway node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);

    let klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        leader.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let system_config_port = pick_local_port(&mut used_ports)?;
    spawn_system_config(
        harness,
        &system_config_bin,
        system_config_port,
        klog_endpoint.as_str(),
    )?;
    wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;

    let endpoint = format!(
        "http://127.0.0.1:{}{}",
        system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let token = system_config_jwt(TEST_DEVICE_NAME, "root", "scheduler")?;
    let suffix = unique_suffix("syscfg-mvcc");
    let base = format!("users/alice/klog_mvcc_dv/{}", suffix);
    let prefix = format!("{}/", base);
    let profile_key = format!("{}profile", prefix);
    let tx_key1 = format!("{}tx/key1", prefix);
    let tx_key2 = format!("{}tx/key2", prefix);
    let stale_key = format!("{}tx/stale", prefix);

    let profile_v1 = r#"{"name":"v1","flags":{"enabled":false}}"#;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_create",
        json!({"key": profile_key.as_str(), "value": profile_v1}),
    )
    .await?;
    let created = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r1) = system_config_value_and_version(&created)?;
    require_system_config_value(&created, profile_v1, r1)?;

    let profile_v2 = r#"{"name":"v2","flags":{"enabled":false}}"#;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set",
        json!({"key": profile_key.as_str(), "value": profile_v2}),
    )
    .await?;
    let set = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r2) = system_config_value_and_version(&set)?;
    if r2 <= r1 {
        return Err(format!(
            "system_config set revision did not advance: r1={r1}, r2={r2}"
        ));
    }
    require_system_config_value(&set, profile_v2, r2)?;

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set_by_json_path",
        json!({"key": profile_key.as_str(), "json_path": "/flags/enabled", "value": "true"}),
    )
    .await?;
    let path_updated = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (profile_v3, r3) = system_config_value_and_version(&path_updated)?;
    if r3 <= r2 {
        return Err(format!(
            "system_config json-path revision did not advance: r2={r2}, r3={r3}"
        ));
    }
    let profile_v3_json: Value = serde_json::from_str(profile_v3.as_str())
        .map_err(|err| format!("failed to decode json-path profile value: {}", err))?;
    if profile_v3_json
        .pointer("/flags/enabled")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(format!(
            "system_config json-path update not visible: {}",
            profile_v3_json
        ));
    }

    let mut stale_actions = serde_json::Map::new();
    stale_actions.insert(
        profile_key.clone(),
        json!({
            "action": "update",
            "value": "stale-profile-value"
        }),
    );
    stale_actions.insert(
        stale_key.clone(),
        json!({
            "action": "create",
            "value": "should-not-exist"
        }),
    );
    let stale_err = expect_system_config_rpc_error(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", profile_key, r2),
            "actions": stale_actions
        }),
    )
    .await?;
    if !stale_err.contains("revision mismatch") {
        return Err(format!(
            "stale system_config exec_tx returned unexpected error: {}",
            stale_err
        ));
    }
    let stale = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": stale_key.as_str()}),
    )
    .await?;
    if !stale.is_null() {
        return Err(format!(
            "stale system_config exec_tx left partial create: {}",
            stale
        ));
    }
    let after_stale = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (after_stale_value, after_stale_revision) = system_config_value_and_version(&after_stale)?;
    if after_stale_value != profile_v3 || after_stale_revision != r3 {
        return Err(format!(
            "stale system_config exec_tx changed guarded key: before=({}, {}), after=({}, {})",
            profile_v3, r3, after_stale_value, after_stale_revision
        ));
    }

    let profile_v4 = "profile-v4";
    let mut good_actions = serde_json::Map::new();
    good_actions.insert(
        profile_key.clone(),
        json!({
            "action": "update",
            "value": profile_v4
        }),
    );
    good_actions.insert(
        tx_key1.clone(),
        json!({
            "action": "create",
            "value": "tx-value-1"
        }),
    );
    good_actions.insert(
        tx_key2.clone(),
        json!({
            "action": "create",
            "value": "tx-value-2"
        }),
    );
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", profile_key, r3),
            "actions": good_actions
        }),
    )
    .await?;
    let profile_after_tx = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r4) = system_config_value_and_version(&profile_after_tx)?;
    if r4 <= r3 {
        return Err(format!(
            "system_config tx revision did not advance: r3={r3}, r4={r4}"
        ));
    }
    require_system_config_value(&profile_after_tx, profile_v4, r4)?;
    let tx1 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    let (_, tx1_revision) = system_config_value_and_version(&tx1)?;
    let tx2 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key2.as_str()}),
    )
    .await?;
    let (_, tx2_revision) = system_config_value_and_version(&tx2)?;
    if tx1_revision != r4 || tx2_revision != r4 {
        return Err(format!(
            "system_config exec_tx keys did not share one klog revision: profile={}, tx1={}, tx2={}",
            r4, tx1_revision, tx2_revision
        ));
    }

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_delete",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    let deleted_tx1 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    if !deleted_tx1.is_null() {
        return Err(format!(
            "deleted system_config key still visible: {}",
            deleted_tx1
        ));
    }
    let delete_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r4 + 1,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &delete_changes,
        &[(
            delete_changes
                .items
                .first()
                .ok_or_else(|| "missing delete change".to_string())?
                .mod_revision,
            &tx_key1,
            "tx-value-1",
            true,
            r4,
            0,
        )],
    )?;
    let r5 = delete_changes.items[0].mod_revision;

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_create",
        json!({"key": tx_key1.as_str(), "value": "tx-value-1-recreated"}),
    )
    .await?;
    let recreated_tx1 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    let (_, r6) = system_config_value_and_version(&recreated_tx1)?;
    if r6 <= r5 {
        return Err(format!(
            "system_config recreate revision did not advance: r5={r5}, r6={r6}"
        ));
    }
    require_system_config_value(&recreated_tx1, "tx-value-1-recreated", r6)?;

    let rev1 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        16,
        None,
        Some(r1),
    )
    .await?;
    require_meta_values(&rev1, &[(&profile_key, profile_v1, r1, r1, 1)])?;

    let rev3 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        16,
        None,
        Some(r3),
    )
    .await?;
    require_meta_values(&rev3, &[(&profile_key, profile_v3.as_str(), r1, r3, 3)])?;

    let rev5 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        16,
        None,
        Some(r5),
    )
    .await?;
    require_meta_values(
        &rev5,
        &[
            (&profile_key, profile_v4, r1, r4, 4),
            (&tx_key2, "tx-value-2", r4, r4, 1),
        ],
    )?;

    let current = query_meta_prefix_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        16,
    )
    .await?;
    require_meta_values(
        &current,
        &[
            (&profile_key, profile_v4, r1, r4, 4),
            (&tx_key1, "tx-value-1-recreated", r6, r6, 1),
            (&tx_key2, "tx-value-2", r4, r4, 1),
        ],
    )?;

    let all_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r1,
        16,
        None,
    )
    .await?;
    if all_changes.has_more {
        return Err(format!(
            "system_config MVCC change-feed unexpectedly paginated: {:?}",
            all_changes
        ));
    }
    require_meta_changes(
        &all_changes,
        &[
            (r1, &profile_key, profile_v1, false, r1, 1),
            (r2, &profile_key, profile_v2, false, r1, 2),
            (r3, &profile_key, profile_v3.as_str(), false, r1, 3),
            (r4, &profile_key, profile_v4, false, r1, 4),
            (r4, &tx_key1, "tx-value-1", false, r4, 1),
            (r4, &tx_key2, "tx-value-2", false, r4, 1),
            (r5, &tx_key1, "tx-value-1", true, r4, 0),
            (r6, &tx_key1, "tx-value-1-recreated", false, r6, 1),
        ],
    )?;

    let compacted = post_meta_compact_via_admin_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        r5,
    )
    .await?;
    if compacted.compacted_revision != r5 || compacted.current_revision < r6 {
        return Err(format!(
            "unexpected system_config MVCC compaction response: {:?}, expected compacted={}, current>={}",
            compacted, r5, r6
        ));
    }

    expect_meta_query_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        Some(profile_key.as_str()),
        None,
        Some(r1),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r1,
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let profile_after_compact = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    require_system_config_value(&profile_after_compact, profile_v4, r4)?;
    let recreated_after_compact = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    require_system_config_value(&recreated_after_compact, "tx-value-1-recreated", r6)?;

    let post_compact_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r6,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[(r6, &tx_key1, "tx-value-1-recreated", false, r6, 1)],
    )?;

    println!(
        "[klog-cluster-dv] system_config MVCC ok: leader={}, endpoint={}, revisions=[{},{},{},{},{},{}], prefix={}",
        leader_id, endpoint, r1, r2, r3, r4, r5, r6, prefix
    );
    Ok(())
}

async fn run_local_gateway_system_config_mvcc() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_mvcc_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_multi_ood_mvcc_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-multi-ood-mvcc-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_MULTI_OOD_MVCC_MODE, route_prefix, 3)
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
    let source = nodes
        .first()
        .ok_or_else(|| "missing source gateway node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target gateway node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);

    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let mut endpoints = Vec::new();
    for node in &nodes {
        let system_config_port = pick_local_port(&mut used_ports)?;
        let klog_endpoint = format!(
            "http://127.0.0.1:{}{}",
            node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
        );
        let process_name = format!("system-config-{}", node.name);
        let system_config_root = harness.root.join(process_name.as_str());
        spawn_system_config_with_options(
            harness,
            process_name.as_str(),
            &system_config_bin,
            system_config_root.as_path(),
            system_config_port,
            Some(klog_endpoint.as_str()),
            node.name.as_str(),
            false,
        )?;
        wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;
        endpoints.push(SystemConfigRpcEndpoint {
            node_name: node.name.clone(),
            endpoint: format!(
                "http://127.0.0.1:{}{}",
                system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
            ),
            token: system_config_jwt(node.name.as_str(), "root", "scheduler")?,
        });
    }

    let suffix = unique_suffix("syscfg-multi-ood-mvcc");
    let base = format!("users/alice/klog_multi_ood_mvcc_dv/{}", suffix);
    let prefix = format!("{}/", base);
    let items_per_ood = 8usize;
    let mut create_tasks = Vec::new();
    for endpoint in &endpoints {
        for index in 0..items_per_ood {
            let client = client.clone();
            let endpoint_url = endpoint.endpoint.clone();
            let token = endpoint.token.clone();
            let node_name = endpoint.node_name.clone();
            let key = format!("{}{}/item-{:02}", prefix, endpoint.node_name, index);
            let value = format!("value-{}-{:02}", endpoint.node_name, index);
            create_tasks.push(tokio::spawn(async move {
                call_system_config_rpc(
                    &client,
                    endpoint_url.as_str(),
                    token.as_str(),
                    "sys_config_create",
                    json!({"key": key.as_str(), "value": value.as_str()}),
                )
                .await?;
                let got = call_system_config_rpc(
                    &client,
                    endpoint_url.as_str(),
                    token.as_str(),
                    "sys_config_get",
                    json!({"key": key.as_str()}),
                )
                .await?;
                let (_, revision) = system_config_value_and_version(&got)?;
                Ok::<_, String>((node_name, key, value, revision))
            }));
        }
    }

    let mut created_records = Vec::new();
    for task in create_tasks {
        let record = task
            .await
            .map_err(|err| format!("system_config create task join failed: {}", err))??;
        created_records.push(record);
    }
    created_records.sort_by(|left, right| left.1.cmp(&right.1));

    for (_, key, value, revision) in &created_records {
        for endpoint in &endpoints {
            let got = call_system_config_rpc(
                &client,
                endpoint.endpoint.as_str(),
                endpoint.token.as_str(),
                "sys_config_get",
                json!({"key": key.as_str()}),
            )
            .await?;
            require_system_config_value(&got, value.as_str(), *revision)?;
        }
    }

    for endpoint in &endpoints {
        let node_base = format!("{}/{}", base, endpoint.node_name);
        let listed = call_system_config_rpc(
            &client,
            endpoint.endpoint.as_str(),
            endpoint.token.as_str(),
            "sys_config_list",
            json!({"key": node_base.as_str()}),
        )
        .await?;
        let listed = listed.as_array().ok_or_else(|| {
            format!(
                "system_config multi-OOD list result is not array for {}: {}",
                endpoint.node_name, listed
            )
        })?;
        if listed.len() != items_per_ood {
            return Err(format!(
                "system_config multi-OOD list length mismatch on {}: expected={}, actual={}, value={}",
                endpoint.node_name,
                items_per_ood,
                listed.len(),
                Value::Array(listed.clone())
            ));
        }
    }

    let initial_current = query_meta_prefix_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        128,
    )
    .await?;
    if initial_current.items.len() != created_records.len() {
        return Err(format!(
            "system_config multi-OOD initial current count mismatch: expected={}, actual={}, items={:?}",
            created_records.len(),
            initial_current.items.len(),
            initial_current.items
        ));
    }

    let shared_key = format!("{}shared/profile", prefix);
    let shared_v1 = "shared-v1";
    call_system_config_rpc(
        &client,
        endpoints[0].endpoint.as_str(),
        endpoints[0].token.as_str(),
        "sys_config_create",
        json!({"key": shared_key.as_str(), "value": shared_v1}),
    )
    .await?;
    let shared_created = call_system_config_rpc(
        &client,
        endpoints[1].endpoint.as_str(),
        endpoints[1].token.as_str(),
        "sys_config_get",
        json!({"key": shared_key.as_str()}),
    )
    .await?;
    let (_, shared_r1) = system_config_value_and_version(&shared_created)?;

    let mut cas_tasks = Vec::new();
    for endpoint in &endpoints {
        let client = client.clone();
        let endpoint_url = endpoint.endpoint.clone();
        let token = endpoint.token.clone();
        let node_name = endpoint.node_name.clone();
        let shared_key_for_task = shared_key.clone();
        let attempt_key = format!("{}shared/attempt-{}", prefix, endpoint.node_name);
        let candidate_value = format!("shared-by-{}", endpoint.node_name);
        cas_tasks.push(tokio::spawn(async move {
            let mut actions = serde_json::Map::new();
            actions.insert(
                shared_key_for_task.clone(),
                json!({
                    "action": "update",
                    "value": candidate_value.as_str()
                }),
            );
            actions.insert(
                attempt_key.clone(),
                json!({
                    "action": "create",
                    "value": candidate_value.as_str()
                }),
            );
            let result = call_system_config_rpc(
                &client,
                endpoint_url.as_str(),
                token.as_str(),
                "sys_config_exec_tx",
                json!({
                    "main_key": format!("{}:{}", shared_key_for_task, shared_r1),
                    "actions": actions
                }),
            )
            .await;
            Ok::<_, String>((node_name, candidate_value, attempt_key, result))
        }));
    }

    let mut cas_results = Vec::new();
    for task in cas_tasks {
        let record = task
            .await
            .map_err(|err| format!("system_config CAS task join failed: {}", err))??;
        cas_results.push(record);
    }
    let winners = cas_results
        .iter()
        .filter(|(_, _, _, result)| result.is_ok())
        .collect::<Vec<_>>();
    if winners.len() != 1 {
        return Err(format!(
            "system_config multi-OOD CAS expected exactly one winner, got {}: {:?}",
            winners.len(),
            cas_results
        ));
    }
    for (node_name, _, _, result) in &cas_results {
        if let Err(err) = result
            && !err.contains("revision mismatch")
        {
            return Err(format!(
                "system_config CAS loser {} returned unexpected error: {}",
                node_name, err
            ));
        }
    }
    let winner_value = winners[0].1.as_str();
    let winner_attempt_key = winners[0].2.as_str();
    let final_shared = call_system_config_rpc(
        &client,
        endpoints[2].endpoint.as_str(),
        endpoints[2].token.as_str(),
        "sys_config_get",
        json!({"key": shared_key.as_str()}),
    )
    .await?;
    let (_, shared_r2) = system_config_value_and_version(&final_shared)?;
    if shared_r2 <= shared_r1 {
        return Err(format!(
            "system_config shared CAS revision did not advance: before={}, after={}",
            shared_r1, shared_r2
        ));
    }
    require_system_config_value(&final_shared, winner_value, shared_r2)?;
    for endpoint in &endpoints {
        let shared = call_system_config_rpc(
            &client,
            endpoint.endpoint.as_str(),
            endpoint.token.as_str(),
            "sys_config_get",
            json!({"key": shared_key.as_str()}),
        )
        .await?;
        require_system_config_value(&shared, winner_value, shared_r2)?;
    }
    for (_, candidate_value, attempt_key, result) in &cas_results {
        let got = call_system_config_rpc(
            &client,
            endpoints[0].endpoint.as_str(),
            endpoints[0].token.as_str(),
            "sys_config_get",
            json!({"key": attempt_key.as_str()}),
        )
        .await?;
        if result.is_ok() {
            require_system_config_value(&got, candidate_value.as_str(), shared_r2)?;
        } else {
            require_system_config_null(&got, attempt_key.as_str())?;
        }
    }

    let stale_key = format!("{}shared/stale-partial", prefix);
    let mut stale_actions = serde_json::Map::new();
    stale_actions.insert(
        shared_key.clone(),
        json!({
            "action": "update",
            "value": "stale-shared-value"
        }),
    );
    stale_actions.insert(
        stale_key.clone(),
        json!({
            "action": "create",
            "value": "should-not-exist"
        }),
    );
    let stale_error = expect_system_config_rpc_error(
        &client,
        endpoints[1].endpoint.as_str(),
        endpoints[1].token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", shared_key, shared_r1),
            "actions": stale_actions
        }),
    )
    .await?;
    if !stale_error.contains("revision mismatch") {
        return Err(format!(
            "system_config stale multi-OOD tx returned unexpected error: {}",
            stale_error
        ));
    }
    let stale = call_system_config_rpc(
        &client,
        endpoints[2].endpoint.as_str(),
        endpoints[2].token.as_str(),
        "sys_config_get",
        json!({"key": stale_key.as_str()}),
    )
    .await?;
    require_system_config_null(&stale, stale_key.as_str())?;

    let delete_key = format!("{}delete-recreate/item", prefix);
    call_system_config_rpc(
        &client,
        endpoints[0].endpoint.as_str(),
        endpoints[0].token.as_str(),
        "sys_config_create",
        json!({"key": delete_key.as_str(), "value": "delete-v1"}),
    )
    .await?;
    let delete_created = call_system_config_rpc(
        &client,
        endpoints[1].endpoint.as_str(),
        endpoints[1].token.as_str(),
        "sys_config_get",
        json!({"key": delete_key.as_str()}),
    )
    .await?;
    let (_, delete_r1) = system_config_value_and_version(&delete_created)?;
    call_system_config_rpc(
        &client,
        endpoints[1].endpoint.as_str(),
        endpoints[1].token.as_str(),
        "sys_config_delete",
        json!({"key": delete_key.as_str()}),
    )
    .await?;
    for endpoint in &endpoints {
        let deleted = call_system_config_rpc(
            &client,
            endpoint.endpoint.as_str(),
            endpoint.token.as_str(),
            "sys_config_get",
            json!({"key": delete_key.as_str()}),
        )
        .await?;
        require_system_config_null(&deleted, delete_key.as_str())?;
    }
    call_system_config_rpc(
        &client,
        endpoints[2].endpoint.as_str(),
        endpoints[2].token.as_str(),
        "sys_config_create",
        json!({"key": delete_key.as_str(), "value": "delete-v2"}),
    )
    .await?;
    let delete_recreated = call_system_config_rpc(
        &client,
        endpoints[0].endpoint.as_str(),
        endpoints[0].token.as_str(),
        "sys_config_get",
        json!({"key": delete_key.as_str()}),
    )
    .await?;
    let (_, delete_r3) = system_config_value_and_version(&delete_recreated)?;
    if delete_r3 <= delete_r1 {
        return Err(format!(
            "system_config delete/recreate revision did not advance: before={}, after={}",
            delete_r1, delete_r3
        ));
    }
    require_system_config_value(&delete_recreated, "delete-v2", delete_r3)?;

    let delete_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        delete_key.as_str(),
        delete_r1,
        8,
        None,
    )
    .await?;
    if delete_changes.items.len() != 3 {
        return Err(format!(
            "system_config delete/recreate changes mismatch: {:?}",
            delete_changes
        ));
    }
    let delete_tombstone_revision = delete_changes.items[1].mod_revision;
    require_meta_changes(
        &delete_changes,
        &[
            (delete_r1, &delete_key, "delete-v1", false, delete_r1, 1),
            (
                delete_tombstone_revision,
                &delete_key,
                "delete-v1",
                true,
                delete_r1,
                0,
            ),
            (delete_r3, &delete_key, "delete-v2", false, delete_r3, 1),
        ],
    )?;

    let compacted = post_meta_compact_via_admin_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        delete_tombstone_revision,
    )
    .await?;
    if compacted.compacted_revision != delete_tombstone_revision
        || compacted.current_revision < delete_r3
    {
        return Err(format!(
            "unexpected system_config multi-OOD compaction response: {:?}, expected compacted={}, current>={}",
            compacted, delete_tombstone_revision, delete_r3
        ));
    }
    expect_meta_query_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        Some(delete_key.as_str()),
        None,
        Some(delete_r1),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    for endpoint in &endpoints {
        let current = call_system_config_rpc(
            &client,
            endpoint.endpoint.as_str(),
            endpoint.token.as_str(),
            "sys_config_get",
            json!({"key": delete_key.as_str()}),
        )
        .await?;
        require_system_config_value(&current, "delete-v2", delete_r3)?;
    }

    let final_expected_count = created_records.len() + 3;
    let final_current = query_meta_prefix_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        128,
    )
    .await?;
    if final_current.items.len() != final_expected_count {
        return Err(format!(
            "system_config multi-OOD final current count mismatch: expected={}, actual={}, winner_attempt={}, items={:?}",
            final_expected_count,
            final_current.items.len(),
            winner_attempt_key,
            final_current.items
        ));
    }

    let scheduler_dump = call_system_config_rpc(
        &client,
        endpoints[0].endpoint.as_str(),
        endpoints[0].token.as_str(),
        "dump_configs_for_scheduler",
        json!({}),
    )
    .await?;
    for (_, key, value, _) in created_records.iter().step_by(items_per_ood) {
        if scheduler_dump.get(key.as_str()).and_then(Value::as_str) != Some(value.as_str()) {
            return Err(format!(
                "scheduler dump missing multi-OOD key {} in {}",
                key, scheduler_dump
            ));
        }
    }
    if scheduler_dump
        .get(stale_key.as_str())
        .and_then(Value::as_str)
        .is_some()
    {
        return Err(format!(
            "scheduler dump contains stale partial key {} in {}",
            stale_key, scheduler_dump
        ));
    }

    println!(
        "[klog-cluster-dv] system_config multi-OOD MVCC ok: leader={}, endpoints={}, created={}, shared_revisions=[{},{}], delete_revisions=[{},{},{}], prefix={}",
        leader_id,
        endpoints.len(),
        created_records.len(),
        shared_r1,
        shared_r2,
        delete_r1,
        delete_tombstone_revision,
        delete_r3,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_system_config_multi_ood_mvcc() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_multi_ood_mvcc_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_pagination_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-pagination-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_PAGINATION_MODE, route_prefix, 3)
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

    let item_count = 45usize;
    let page_limit = 17usize;
    let suffix = unique_suffix("syscfg-pagination");
    let base = format!("users/alice/klog_pagination_dv/{}", suffix);
    let prefix = format!("{}/", base);
    let source = nodes
        .first()
        .ok_or_else(|| "missing source gateway node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target gateway node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let expected_keys = (0..item_count)
        .map(|idx| format!("{}item-{:04}", prefix, idx))
        .collect::<Vec<_>>();

    for (idx, key) in expected_keys.iter().enumerate() {
        let value = format!("value-{:04}", idx);
        put_meta_via_cluster_inter_route(
            &client,
            source_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
    }

    let mut cursor = None;
    let mut page_sizes = Vec::new();
    let mut collected_keys = Vec::new();
    loop {
        let page = query_meta_prefix_page_via_cluster_inter_route(
            &client,
            source_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            prefix.as_str(),
            page_limit,
            cursor.as_deref(),
        )
        .await?;
        if page.items.is_empty() && page.has_more {
            return Err("meta pagination returned empty page with has_more=true".to_string());
        }
        page_sizes.push(page.items.len());
        collected_keys.extend(page.items.iter().map(|item| item.key.clone()));
        if !page.has_more {
            break;
        }
        let Some(next_cursor) = page.next_cursor else {
            return Err("meta pagination missing next_cursor while has_more=true".to_string());
        };
        if cursor.as_ref() == Some(&next_cursor) {
            return Err(format!(
                "meta pagination cursor did not advance: {}",
                next_cursor
            ));
        }
        cursor = Some(next_cursor);
    }
    if collected_keys != expected_keys {
        return Err(format!(
            "meta pagination keys mismatch: expected_len={}, actual_len={}, page_sizes={:?}",
            expected_keys.len(),
            collected_keys.len(),
            page_sizes
        ));
    }
    if page_sizes != vec![17, 17, 11] {
        return Err(format!(
            "unexpected meta pagination page sizes: {:?}",
            page_sizes
        ));
    }

    let klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        leader.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let system_config_port = pick_local_port(&mut used_ports)?;
    let page_limit_env = page_limit.to_string();
    spawn_system_config_with_extra_env(
        harness,
        &system_config_bin,
        system_config_port,
        klog_endpoint.as_str(),
        &[(
            ENV_SYSTEM_CONFIG_KLOG_META_QUERY_LIMIT,
            page_limit_env.as_str(),
        )],
    )?;
    wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;

    let endpoint = format!(
        "http://127.0.0.1:{}{}",
        system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let scheduler_token = system_config_jwt(TEST_DEVICE_NAME, "root", "scheduler")?;
    let listed = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        scheduler_token.as_str(),
        "sys_config_list",
        json!({"key": base}),
    )
    .await?;
    let listed = listed
        .as_array()
        .ok_or_else(|| format!("system_config paginated list is not array: {}", listed))?;
    if listed.len() != item_count {
        return Err(format!(
            "system_config paginated list length mismatch: expected={}, actual={}, value={}",
            item_count,
            listed.len(),
            Value::Array(listed.clone())
        ));
    }
    for idx in [0usize, 16, 17, 34, 44] {
        let expected_child = format!("item-{:04}", idx);
        if !listed
            .iter()
            .any(|value| value.as_str() == Some(expected_child.as_str()))
        {
            return Err(format!(
                "system_config paginated list missing child {}: {:?}",
                expected_child, listed
            ));
        }
    }

    let scheduler_dump = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        scheduler_token.as_str(),
        "dump_configs_for_scheduler",
        json!({}),
    )
    .await?;
    for idx in [0usize, 17, 44] {
        let key = format!("{}item-{:04}", prefix, idx);
        if scheduler_dump.get(key.as_str()).and_then(Value::as_str)
            != Some(format!("value-{:04}", idx).as_str())
        {
            return Err(format!(
                "scheduler dump missing paginated key {} in {}",
                key, scheduler_dump
            ));
        }
    }

    println!(
        "[klog-cluster-dv] system_config pagination ok: leader={}, endpoint={}, prefix={}, items={}, page_sizes={:?}",
        leader_id, endpoint, prefix, item_count, page_sizes
    );
    Ok(())
}

async fn run_local_gateway_system_config_pagination() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_pagination_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_rollout_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-rollout-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_ROLLOUT_MODE, route_prefix, 3).await?;
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
    let reader_node = nodes
        .get(1)
        .ok_or_else(|| "missing second OOD node".to_string())?
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
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;

    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let bootstrap_sled_port = pick_local_port(&mut used_ports)?;
    let reader_sled_port = pick_local_port(&mut used_ports)?;
    let bootstrap_klog_port = pick_local_port(&mut used_ports)?;
    let reader_klog_port = pick_local_port(&mut used_ports)?;
    let bootstrap_root = harness.root.join("system-config-ood1-root");
    let reader_root = harness.root.join("system-config-ood2-root");
    let bootstrap_token = system_config_jwt(seed.name.as_str(), "root", "scheduler")?;
    let reader_token = system_config_jwt(reader_node.name.as_str(), "root", "scheduler")?;
    let suffix = unique_suffix("syscfg-rollout");
    let base = format!("users/alice/klog_rollout_dv/{}", suffix);
    let migrated_key = format!("{}/migrated", base);
    let local_only_key = format!("{}/local_only", base);
    let reader_write_key = format!("{}/reader_write", base);

    spawn_system_config_with_options(
        harness,
        "system-config-ood1-sled",
        &system_config_bin,
        bootstrap_root.as_path(),
        bootstrap_sled_port,
        None,
        seed.name.as_str(),
        false,
    )?;
    wait_tcp("127.0.0.1", bootstrap_sled_port, Duration::from_secs(15)).await?;
    let bootstrap_sled_endpoint = format!(
        "http://127.0.0.1:{}{}",
        bootstrap_sled_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    call_system_config_rpc(
        &client,
        bootstrap_sled_endpoint.as_str(),
        bootstrap_token.as_str(),
        "sys_config_create",
        json!({"key": migrated_key, "value": "from-ood1-sled"}),
    )
    .await?;
    harness.stop("system-config-ood1-sled")?;

    spawn_system_config_with_options(
        harness,
        "system-config-ood2-sled",
        &system_config_bin,
        reader_root.as_path(),
        reader_sled_port,
        None,
        reader_node.name.as_str(),
        false,
    )?;
    wait_tcp("127.0.0.1", reader_sled_port, Duration::from_secs(15)).await?;
    let reader_sled_endpoint = format!(
        "http://127.0.0.1:{}{}",
        reader_sled_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    call_system_config_rpc(
        &client,
        reader_sled_endpoint.as_str(),
        reader_token.as_str(),
        "sys_config_create",
        json!({"key": local_only_key, "value": "from-ood2-local-sled"}),
    )
    .await?;
    harness.stop("system-config-ood2-sled")?;

    let bootstrap_klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        seed.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    spawn_system_config_with_options(
        harness,
        "system-config-ood1-klog-bootstrap",
        &system_config_bin,
        bootstrap_root.as_path(),
        bootstrap_klog_port,
        Some(bootstrap_klog_endpoint.as_str()),
        seed.name.as_str(),
        true,
    )?;
    wait_tcp("127.0.0.1", bootstrap_klog_port, Duration::from_secs(15)).await?;
    let bootstrap_klog_service_endpoint = format!(
        "http://127.0.0.1:{}{}",
        bootstrap_klog_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let migrated = call_system_config_rpc(
        &client,
        bootstrap_klog_service_endpoint.as_str(),
        bootstrap_token.as_str(),
        "sys_config_get",
        json!({"key": migrated_key}),
    )
    .await?;
    require_system_config_value(&migrated, "from-ood1-sled", 1)?;
    let local_only_after_bootstrap = call_system_config_rpc(
        &client,
        bootstrap_klog_service_endpoint.as_str(),
        bootstrap_token.as_str(),
        "sys_config_get",
        json!({"key": local_only_key}),
    )
    .await?;
    if !local_only_after_bootstrap.is_null() {
        return Err(format!(
            "non-bootstrap OOD local sled key was unexpectedly migrated before reader start: {}",
            local_only_after_bootstrap
        ));
    }

    let reader_klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        reader_node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    spawn_system_config_with_options(
        harness,
        "system-config-ood2-klog-reader",
        &system_config_bin,
        reader_root.as_path(),
        reader_klog_port,
        Some(reader_klog_endpoint.as_str()),
        reader_node.name.as_str(),
        false,
    )?;
    wait_tcp("127.0.0.1", reader_klog_port, Duration::from_secs(15)).await?;
    let reader_klog_service_endpoint = format!(
        "http://127.0.0.1:{}{}",
        reader_klog_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let migrated_from_reader = call_system_config_rpc(
        &client,
        reader_klog_service_endpoint.as_str(),
        reader_token.as_str(),
        "sys_config_get",
        json!({"key": migrated_key}),
    )
    .await?;
    require_system_config_value(&migrated_from_reader, "from-ood1-sled", 1)?;
    let local_only_from_reader = call_system_config_rpc(
        &client,
        reader_klog_service_endpoint.as_str(),
        reader_token.as_str(),
        "sys_config_get",
        json!({"key": local_only_key}),
    )
    .await?;
    if !local_only_from_reader.is_null() {
        return Err(format!(
            "non-bootstrap OOD copied its local sled state without bootstrap flag: {}",
            local_only_from_reader
        ));
    }

    call_system_config_rpc(
        &client,
        reader_klog_service_endpoint.as_str(),
        reader_token.as_str(),
        "sys_config_create",
        json!({"key": reader_write_key, "value": "from-ood2-klog"}),
    )
    .await?;
    let reader_write_from_bootstrap = call_system_config_rpc(
        &client,
        bootstrap_klog_service_endpoint.as_str(),
        bootstrap_token.as_str(),
        "sys_config_get",
        json!({"key": reader_write_key}),
    )
    .await?;
    require_system_config_value(&reader_write_from_bootstrap, "from-ood2-klog", 1)?;

    println!(
        "[klog-cluster-dv] system_config rollout ok: leader={}, bootstrap_ood={}, reader_ood={}, prefix={}",
        leader_id, seed.name, reader_node.name, base
    );
    Ok(())
}

async fn run_local_gateway_system_config_rollout() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_rollout_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_installed_runtime_smoke() -> Result<(), String> {
    let buckyos_root = get_buckyos_root();
    let local_node_name = load_local_node_name(&buckyos_root)?;
    let cluster_route = load_klog_cluster_route(&buckyos_root)?;
    let local_route = require_node_route(&cluster_route, local_node_name.as_str())?;
    let gateway_addr = gateway_addr_from_route(&cluster_route);
    let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
    let http_client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;

    println!("[klog-cluster-dv] BUCKYOS_ROOT={}", buckyos_root.display());
    println!("[klog-cluster-dv] node_name={}", local_node_name);
    println!("[klog-cluster-dv] node_gateway_addr={}", gateway_addr);
    println!(
        "[klog-cluster-dv] route_prefix={}",
        cluster_route.route_prefix
    );
    println!(
        "[klog-cluster-dv] local_cluster_ports={:?}",
        local_route.ports
    );

    let suffix = unique_suffix("cluster-dv");

    let service_source = format!("test/test_klog_cluster_dv-service-{}", suffix);
    let service_append = append_via_service_route(
        &http_client,
        gateway_addr.as_str(),
        local_node_name.as_str(),
        service_source.as_str(),
        format!("cluster dv service append {}", suffix).as_str(),
    )
    .await?;
    let cluster_query = query_via_cluster_inter_route(
        &http_client,
        gateway_addr.as_str(),
        cluster_route.route_prefix.as_str(),
        local_node_name.as_str(),
        service_append.id,
        service_source.as_str(),
    )
    .await?;
    require_query_match(&cluster_query, service_append.id, service_source.as_str())?;
    println!(
        "[klog-cluster-dv] service append visible via cluster inter route: id={}",
        service_append.id
    );

    let cluster_source = format!("test/test_klog_cluster_dv-cluster-{}", suffix);
    let cluster_append = append_via_cluster_inter_route(
        &http_client,
        gateway_addr.as_str(),
        cluster_route.route_prefix.as_str(),
        local_node_name.as_str(),
        cluster_source.as_str(),
        format!("cluster dv inter append {}", suffix).as_str(),
    )
    .await?;
    let service_query = query_via_service_route(
        &http_client,
        gateway_addr.as_str(),
        cluster_append.id,
        cluster_source.as_str(),
    )
    .await?;
    require_query_match(&service_query, cluster_append.id, cluster_source.as_str())?;
    println!(
        "[klog-cluster-dv] cluster inter append visible via service route: id={}",
        cluster_append.id
    );

    let cluster_state = fetch_cluster_state_via_admin_route(
        &http_client,
        gateway_addr.as_str(),
        cluster_route.route_prefix.as_str(),
        local_node_name.as_str(),
    )
    .await?;
    require_cluster_state(&cluster_state, local_node_name.as_str())?;
    println!("[klog-cluster-dv] admin cluster-state ok via cluster route");

    println!("[klog-cluster-dv] smoke test success");
    Ok(())
}

pub(super) async fn run() -> Result<(), String> {
    match std::env::var("KLOG_CLUSTER_DV_MODE")
        .unwrap_or_default()
        .trim()
    {
        "" => run_installed_runtime_smoke().await,
        MULTI_NODE_MODE => run_local_gateway_failover_smoke().await,
        MEMBERSHIP_MODE => run_local_gateway_membership().await,
        OOD_MEMBERSHIP_MODE => run_local_gateway_ood_membership().await,
        OOD_LEADER_FAILOVER_SHRINK_MODE => run_local_gateway_ood_leader_failover_shrink().await,
        OOD_SEED_UNAVAILABLE_JOIN_MODE => run_local_gateway_ood_seed_unavailable_join().await,
        OOD_SINGLE_TO_TWO_MODE => run_local_gateway_ood_single_to_two().await,
        OOD_TWO_VOTER_LOSS_MODE => run_local_gateway_ood_two_voter_loss().await,
        OOD_SNAPSHOT_MEMBERSHIP_MODE => run_local_gateway_ood_snapshot_membership().await,
        RESTART_RECOVERY_MODE => run_local_gateway_restart_recovery().await,
        MVCC_CLUSTER_MODE => run_local_gateway_mvcc_cluster().await,
        MVCC_CHANGE_FEED_MODE => run_local_gateway_mvcc_change_feed().await,
        MVCC_CHANGE_FEED_FAILOVER_MODE => run_local_gateway_mvcc_change_feed_failover().await,
        MVCC_CHANGE_FEED_STRESS_MODE => run_local_gateway_mvcc_change_feed_stress().await,
        MVCC_FAILOVER_MODE => run_local_gateway_mvcc_failover().await,
        MVCC_AUTO_COMPACT_FAILOVER_MODE => run_local_gateway_mvcc_auto_compact_failover().await,
        MVCC_COMPACTION_LEADER_SWITCH_MODE => {
            run_local_gateway_mvcc_compaction_leader_switch().await
        }
        MVCC_CRASH_RECOVERY_MODE => run_local_gateway_mvcc_crash_recovery().await,
        MVCC_COMPACT_DURING_SNAPSHOT_MODE => run_local_gateway_mvcc_compact_during_snapshot().await,
        RAFT_OLD_LEADER_REJOIN_MODE => run_local_gateway_raft_old_leader_rejoin().await,
        RAFT_FOLLOWER_LAG_SNAPSHOT_INSTALL_MODE => {
            run_local_gateway_raft_follower_lag_snapshot_install().await
        }
        RAFT_QUORUM_LOSS_RECOVERY_MODE => run_local_gateway_raft_quorum_loss_recovery().await,
        RAFT_MEMBERSHIP_CHANGE_REJOIN_MODE => {
            run_local_gateway_raft_membership_change_rejoin().await
        }
        RAFT_CONCURRENT_MEMBERSHIP_MODE => run_local_gateway_raft_concurrent_membership().await,
        RAFT_JOIN_RETRY_IDEMPOTENCY_MODE => run_local_gateway_raft_join_retry_idempotency().await,
        RAFT_SNAPSHOT_INSTALL_CRASH_MODE => run_local_gateway_raft_snapshot_install_crash().await,
        NODE_ID_REUSE_MODE => run_local_gateway_node_id_reuse().await,
        MVCC_SNAPSHOT_MEMBERSHIP_MODE => run_local_gateway_mvcc_snapshot_membership().await,
        SYSTEM_CONFIG_KV_MODE => run_local_gateway_system_config_kv().await,
        SYSTEM_CONFIG_SERVICE_MODE => run_local_gateway_system_config_service().await,
        SYSTEM_CONFIG_LEADER_FAILOVER_MODE => {
            run_local_gateway_system_config_leader_failover().await
        }
        GATEWAY_ABNORMAL_MODE => run_local_gateway_abnormal().await,
        SYSTEM_CONFIG_STALE_CONFIG_REJOIN_MODE => {
            run_local_gateway_system_config_stale_config_rejoin().await
        }
        SYSTEM_CONFIG_MVCC_MODE => run_local_gateway_system_config_mvcc().await,
        SYSTEM_CONFIG_MULTI_OOD_MVCC_MODE => run_local_gateway_system_config_multi_ood_mvcc().await,
        SYSTEM_CONFIG_PAGINATION_MODE => run_local_gateway_system_config_pagination().await,
        SYSTEM_CONFIG_ROLLOUT_MODE => run_local_gateway_system_config_rollout().await,
        other => {
            let supported = [
                "",
                MULTI_NODE_MODE,
                MEMBERSHIP_MODE,
                OOD_MEMBERSHIP_MODE,
                OOD_LEADER_FAILOVER_SHRINK_MODE,
                OOD_SEED_UNAVAILABLE_JOIN_MODE,
                OOD_SINGLE_TO_TWO_MODE,
                OOD_TWO_VOTER_LOSS_MODE,
                OOD_SNAPSHOT_MEMBERSHIP_MODE,
                RESTART_RECOVERY_MODE,
                MVCC_CLUSTER_MODE,
                MVCC_CHANGE_FEED_MODE,
                MVCC_CHANGE_FEED_FAILOVER_MODE,
                MVCC_CHANGE_FEED_STRESS_MODE,
                MVCC_FAILOVER_MODE,
                MVCC_AUTO_COMPACT_FAILOVER_MODE,
                MVCC_COMPACTION_LEADER_SWITCH_MODE,
                MVCC_CRASH_RECOVERY_MODE,
                MVCC_COMPACT_DURING_SNAPSHOT_MODE,
                RAFT_OLD_LEADER_REJOIN_MODE,
                RAFT_FOLLOWER_LAG_SNAPSHOT_INSTALL_MODE,
                RAFT_QUORUM_LOSS_RECOVERY_MODE,
                RAFT_MEMBERSHIP_CHANGE_REJOIN_MODE,
                RAFT_CONCURRENT_MEMBERSHIP_MODE,
                RAFT_JOIN_RETRY_IDEMPOTENCY_MODE,
                RAFT_SNAPSHOT_INSTALL_CRASH_MODE,
                NODE_ID_REUSE_MODE,
                MVCC_SNAPSHOT_MEMBERSHIP_MODE,
                SYSTEM_CONFIG_KV_MODE,
                SYSTEM_CONFIG_SERVICE_MODE,
                SYSTEM_CONFIG_LEADER_FAILOVER_MODE,
                GATEWAY_ABNORMAL_MODE,
                SYSTEM_CONFIG_STALE_CONFIG_REJOIN_MODE,
                SYSTEM_CONFIG_MVCC_MODE,
                SYSTEM_CONFIG_MULTI_OOD_MVCC_MODE,
                SYSTEM_CONFIG_PAGINATION_MODE,
                SYSTEM_CONFIG_ROLLOUT_MODE,
            ]
            .join("', '");
            Err(format!(
                "unsupported KLOG_CLUSTER_DV_MODE='{}'; supported values: '{}'",
                other, supported
            ))
        }
    }
}
