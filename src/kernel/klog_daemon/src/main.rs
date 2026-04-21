mod cluster;
mod config;
mod constants;
mod lifecycle;
mod logging;

use buckyos_api::{
    BuckyOSRuntime, BuckyOSRuntimeType, KLOG_CLUSTER_ADMIN_PORT, KLOG_CLUSTER_INTER_PORT,
    KLOG_CLUSTER_RAFT_PORT, KLOG_SERVICE_PORT, KLOG_SERVICE_UNIQUE_ID,
    get_session_token_env_key, init_buckyos_api_runtime, set_buckyos_api_runtime,
};
use cluster::{initialize_cluster_if_needed, spawn_auto_join_task};
use config::{
    BuckyosKlogConfig, KLogClusterConfigPatch, KLogNetworkConfigPatch, KLogRuntimeConfig,
    KLogRuntimeConfigSource,
};
use klog::logs::RocksDbLogStorage;
use klog::network::{KNetworkFactory, KNetworkServer};
use klog::rpc::KRpcServer;
use klog::state_machine::{KLogStateMachine, SnapshotManager};
use klog::state_store::{
    KLogStateStore, KLogStateStoreManager, RocksDbSnapshotMode, RocksDbStateStore,
};
use klog::{KClusterTransportConfig, KClusterTransportMode};
use lifecycle::run_server_lifecycle;
use log::{error, info, warn};
use logging::init_logging;
use std::env;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    init_logging();

    let (mut cfg, source, runtime) = match load_runtime_config().await {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to load runtime config: {}", e);
            std::process::exit(1);
        }
    };
    info!("klog runtime config source: {}", source);

    if let Err(e) = init_buckyos_runtime_if_needed(&mut cfg, runtime).await {
        error!("Failed to initialize BuckyOS runtime integration: {}", e);
        std::process::exit(1);
    }

    if let Err(e) = run(cfg).await {
        error!("klog startup failed: {}", e);
        std::process::exit(1);
    }
}

async fn load_runtime_config() -> Result<
    (
        KLogRuntimeConfig,
        KLogRuntimeConfigSource,
        Option<BuckyOSRuntime>,
    ),
    String,
> {
    match KLogRuntimeConfig::load() {
        Ok((cfg, source)) => Ok((cfg, source, None)),
        Err(err) => {
            let session_token_env_key = get_session_token_env_key(KLOG_SERVICE_UNIQUE_ID, false);
            if env::var_os(&session_token_env_key).is_none() {
                return Err(err);
            }
            if is_explicit_klog_config_requested() {
                return Err(err);
            }

            warn!(
                "Klog explicit env/file config is absent under BuckyOS runtime; fallback to services/{}/settings and runtime identity after load failure: {}",
                KLOG_SERVICE_UNIQUE_ID, err
            );
            let runtime = init_logged_in_buckyos_runtime().await?;
            let (cfg, source) = load_runtime_config_from_buckyos(&runtime).await?;
            Ok((cfg, source, Some(runtime)))
        }
    }
}

fn is_explicit_klog_config_requested() -> bool {
    let session_token_env_key = get_session_token_env_key(KLOG_SERVICE_UNIQUE_ID, false);
    env::vars_os().any(|(key, _)| {
        let key = key.to_string_lossy();
        key.starts_with("KLOG_") && key.as_ref() != session_token_env_key
    })
}

async fn init_logged_in_buckyos_runtime() -> Result<BuckyOSRuntime, String> {
    let mut runtime = init_buckyos_api_runtime(
        KLOG_SERVICE_UNIQUE_ID,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await
    .map_err(|e| {
        let msg = format!(
            "Failed to initialize BuckyOS runtime for {}: {}",
            KLOG_SERVICE_UNIQUE_ID, e
        );
        error!("{}", msg);
        msg
    })?;
    runtime.login().await.map_err(|e| {
        let msg = format!(
            "Failed to login BuckyOS runtime for {}: {}",
            KLOG_SERVICE_UNIQUE_ID, e
        );
        error!("{}", msg);
        msg
    })?;
    Ok(runtime)
}

async fn load_runtime_config_from_buckyos(
    runtime: &BuckyOSRuntime,
) -> Result<(KLogRuntimeConfig, KLogRuntimeConfigSource), String> {
    let mut patch = match runtime.get_my_settings().await {
        Ok(settings) => serde_json::from_value::<BuckyosKlogConfig>(settings).map_err(|e| {
            format!(
                "Failed to parse services/{}/settings as klog runtime patch: {}",
                KLOG_SERVICE_UNIQUE_ID, e
            )
        })?,
        Err(err) => {
            warn!(
                "services/{}/settings is unavailable; use runtime-derived klog defaults: {}",
                KLOG_SERVICE_UNIQUE_ID, err
            );
            BuckyosKlogConfig::default()
        }
    };
    apply_buckyos_runtime_defaults(&mut patch, runtime)?;
    KLogRuntimeConfig::load_from_buckyos(&patch)
}

fn apply_buckyos_runtime_defaults(
    patch: &mut BuckyosKlogConfig,
    runtime: &BuckyOSRuntime,
) -> Result<(), String> {
    let device = runtime.device_config.as_ref().ok_or_else(|| {
        format!(
            "Missing device_config while deriving BuckyOS runtime config for {}",
            KLOG_SERVICE_UNIQUE_ID
        )
    })?;
    let node_name = device.name.trim();
    if node_name.is_empty() {
        return Err(format!(
            "Missing runtime device name while deriving BuckyOS runtime config for {}",
            KLOG_SERVICE_UNIQUE_ID
        ));
    }

    let zone_host = runtime.zone_id.to_host_name();
    if zone_host.trim().is_empty() {
        return Err(format!(
            "Missing zone host name while deriving BuckyOS runtime config for {}",
            KLOG_SERVICE_UNIQUE_ID
        ));
    }

    if patch.node_id.is_none() {
        patch.node_id = Some(derive_buckyos_raft_node_id(runtime, node_name)?);
    }

    let network = patch.network.get_or_insert_with(KLogNetworkConfigPatch::default);
    network
        .listen_addr
        .get_or_insert_with(|| format!("0.0.0.0:{}", KLOG_CLUSTER_RAFT_PORT));
    network
        .inter_node_listen_addr
        .get_or_insert_with(|| format!("0.0.0.0:{}", KLOG_CLUSTER_INTER_PORT));
    network
        .admin_listen_addr
        .get_or_insert_with(|| format!("127.0.0.1:{}", KLOG_CLUSTER_ADMIN_PORT));
    network
        .rpc_listen_addr
        .get_or_insert_with(|| format!("127.0.0.1:{}", KLOG_SERVICE_PORT));
    network
        .advertise_addr
        .get_or_insert_with(|| "127.0.0.1".to_string());
    network.advertise_port.get_or_insert(KLOG_CLUSTER_RAFT_PORT);
    network
        .advertise_inter_port
        .get_or_insert(KLOG_CLUSTER_INTER_PORT);
    network
        .advertise_admin_port
        .get_or_insert(KLOG_CLUSTER_ADMIN_PORT);
    network.rpc_advertise_port.get_or_insert(KLOG_SERVICE_PORT);
    network
        .advertise_node_name
        .get_or_insert_with(|| node_name.to_string());
    network.enable_rpc_server.get_or_insert(true);

    let cluster = patch.cluster.get_or_insert_with(KLogClusterConfigPatch::default);
    cluster.name.get_or_insert_with(|| zone_host.clone());
    cluster.id.get_or_insert(zone_host);
    cluster
        .auto_bootstrap
        .get_or_insert_with(|| derive_buckyos_auto_bootstrap(runtime, node_name));

    Ok(())
}

fn derive_buckyos_raft_node_id(runtime: &BuckyOSRuntime, node_name: &str) -> Result<u64, String> {
    if let Some(zone_config) = runtime.zone_config.as_ref()
        && let Some(index) = zone_config.oods.iter().position(|ood| ood.name == node_name)
    {
        return Ok((index + 1) as u64);
    }

    derive_raft_node_id_from_node_name(node_name)
}

fn derive_buckyos_auto_bootstrap(runtime: &BuckyOSRuntime, node_name: &str) -> bool {
    runtime
        .zone_config
        .as_ref()
        .and_then(|zone_config| zone_config.oods.first())
        .map(|ood| ood.name.as_str() == node_name)
        .unwrap_or(true)
}

fn derive_raft_node_id_from_node_name(node_name: &str) -> Result<u64, String> {
    let digits = node_name
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if digits.is_empty() {
        return Err(format!(
            "Failed to derive raft node_id from BuckyOS node name '{}': missing numeric suffix and zone_config ordering",
            node_name
        ));
    }

    let node_id = digits.parse::<u64>().map_err(|e| {
        format!(
            "Failed to parse raft node_id from BuckyOS node name '{}': {}",
            node_name, e
        )
    })?;
    if node_id == 0 {
        return Err(format!(
            "Invalid derived raft node_id=0 from BuckyOS node name '{}'",
            node_name
        ));
    }
    Ok(node_id)
}

async fn init_buckyos_runtime_if_needed(
    cfg: &mut KLogRuntimeConfig,
    runtime: Option<BuckyOSRuntime>,
) -> Result<(), String> {
    let session_token_env_key = get_session_token_env_key(KLOG_SERVICE_UNIQUE_ID, false);
    if env::var_os(&session_token_env_key).is_none() {
        info!(
            "BuckyOS runtime integration disabled because env {} is not set; running in standalone mode",
            session_token_env_key
        );
        return Ok(());
    }

    if !cfg.enable_rpc_server {
        let msg = format!(
            "Invalid config for BuckyOS runtime: enable_rpc_server=false is not allowed when {} is set",
            session_token_env_key
        );
        error!("{}", msg);
        return Err(msg);
    }

    let rpc_port = parse_port_from_addr(&cfg.rpc_listen_addr).ok_or_else(|| {
        let msg = format!(
            "Invalid rpc_listen_addr for BuckyOS runtime integration: {}",
            cfg.rpc_listen_addr
        );
        error!("{}", msg);
        msg
    })?;

    let runtime = match runtime {
        Some(runtime) => runtime,
        None => init_logged_in_buckyos_runtime().await?,
    };
    validate_buckyos_cluster_transport_identity(
        cfg.cluster_network.mode,
        cfg.advertise_node_name.as_deref(),
        runtime
            .device_config
            .as_ref()
            .map(|device| device.name.as_str()),
    )?;
    runtime.set_main_service_port(rpc_port).await;

    let runtime_data_dir = runtime.get_data_folder().map_err(|e| {
        let msg = format!(
            "Failed to resolve BuckyOS data dir for {}: {}",
            KLOG_SERVICE_UNIQUE_ID, e
        );
        error!("{}", msg);
        msg
    })?;

    if cfg.data_dir != runtime_data_dir {
        info!(
            "Overriding klog data dir with BuckyOS runtime data dir: old={}, new={}",
            cfg.data_dir.display(),
            runtime_data_dir.display()
        );
        cfg.data_dir = runtime_data_dir;
    }

    set_buckyos_api_runtime(runtime).map_err(|e| {
        let msg = format!(
            "Failed to register BuckyOS runtime for {}: {}",
            KLOG_SERVICE_UNIQUE_ID, e
        );
        error!("{}", msg);
        msg
    })?;

    info!(
        "BuckyOS runtime integration enabled: service_name={}, service_port={}, data_dir={}",
        KLOG_SERVICE_UNIQUE_ID,
        rpc_port,
        cfg.data_dir.display()
    );

    if rpc_port != KLOG_SERVICE_PORT {
        warn!(
            "klog-service is running with a non-default rpc port: configured={}, default={}",
            rpc_port, KLOG_SERVICE_PORT
        );
    }

    Ok(())
}

fn parse_port_from_addr(addr: &str) -> Option<u16> {
    let (_, port_str) = addr.rsplit_once(':')?;
    port_str.parse::<u16>().ok()
}

fn validate_buckyos_cluster_transport_identity(
    transport_mode: KClusterTransportMode,
    advertise_node_name: Option<&str>,
    runtime_node_name: Option<&str>,
) -> Result<(), String> {
    if transport_mode == KClusterTransportMode::Direct {
        return Ok(());
    }

    let runtime_node_name = runtime_node_name.ok_or_else(|| {
        let msg = format!(
            "Missing BuckyOS runtime node identity for cluster_network.mode={}",
            transport_mode
        );
        error!("{}", msg);
        msg
    })?;
    let advertise_node_name = advertise_node_name.ok_or_else(|| {
        let msg = format!(
            "Missing network.advertise_node_name (BuckyOS node name) for cluster_network.mode={} under BuckyOS runtime",
            transport_mode
        );
        error!("{}", msg);
        msg
    })?;

    if advertise_node_name != runtime_node_name {
        let msg = format!(
            "Invalid cluster transport identity: cluster_network.mode={}, advertise_node_name(BuckyOS node name)={} must equal runtime_node_name={}",
            transport_mode, advertise_node_name, runtime_node_name
        );
        error!("{}", msg);
        return Err(msg);
    }

    info!(
        "Validated BuckyOS cluster transport identity: cluster_network.mode={}, advertise_node_name(BuckyOS node name)={}, runtime_node_name={}",
        transport_mode, advertise_node_name, runtime_node_name
    );
    Ok(())
}

async fn run(cfg: KLogRuntimeConfig) -> Result<(), String> {
    std::fs::create_dir_all(&cfg.data_dir).map_err(|e| {
        format!(
            "Failed to create data dir {}: {}",
            cfg.data_dir.display(),
            e
        )
    })?;

    info!(
        "klog startup config: raft_node_id={}, raft_listen_addr={}, inter_node_listen_addr={}, admin_listen_addr={}, rpc_enabled={}, rpc_listen_addr={}, advertise_addr={}, advertise_port={}, advertise_inter_port={}, advertise_admin_port={}, rpc_advertise_port={}, advertise_node_name(BuckyOS node name)={:?}, data_dir={}, cluster_name={}, cluster_id={}, auto_bootstrap={}, state_store_sync_write={}, cluster_network_mode={}, cluster_gateway_addr={}, cluster_gateway_route_prefix={}, join_targets={:?}, join_blocking={}, join_target_role={}, join_retry(strategy={}, initial_interval_ms={}, max_interval_ms={}, multiplier={}, jitter_ratio={}, max_attempts={}, request_timeout_ms={}, shuffle_targets_each_round={}, config_change_conflict_extra_backoff_ms={}), raft(election_timeout_min_ms={}, election_timeout_max_ms={}, heartbeat_interval_ms={}, install_snapshot_timeout_ms={}, max_payload_entries={}, replication_lag_threshold={}, snapshot_policy={}, snapshot_max_chunk_size_bytes={}, max_in_snapshot_log_to_keep={}, purge_batch_size={}), admin_local_only={}, rpc_append(timeout_ms={}, body_limit_bytes={}, concurrency={}), rpc_query(timeout_ms={}, body_limit_bytes={}, concurrency={}), rpc_jsonrpc(timeout_ms={}, body_limit_bytes={}, concurrency={})",
        cfg.node_id,
        cfg.listen_addr,
        cfg.inter_node_listen_addr,
        cfg.admin_listen_addr,
        cfg.enable_rpc_server,
        cfg.rpc_listen_addr,
        cfg.advertise_addr,
        cfg.advertise_port,
        cfg.advertise_inter_port,
        cfg.advertise_admin_port,
        cfg.rpc_advertise_port,
        cfg.advertise_node_name.as_deref(),
        cfg.data_dir.display(),
        cfg.cluster_name,
        cfg.cluster_id,
        cfg.auto_bootstrap,
        cfg.state_store_sync_write,
        cfg.cluster_network.mode,
        cfg.cluster_network.gateway_addr,
        cfg.cluster_network.gateway_route_prefix,
        cfg.join_targets,
        cfg.join_blocking,
        cfg.join_target_role,
        cfg.join_retry.strategy,
        cfg.join_retry.initial_interval_ms,
        cfg.join_retry.max_interval_ms,
        cfg.join_retry.multiplier,
        cfg.join_retry.jitter_ratio,
        cfg.join_retry.max_attempts,
        cfg.join_retry.request_timeout_ms,
        cfg.join_retry.shuffle_targets_each_round,
        cfg.join_retry.config_change_conflict_extra_backoff_ms,
        cfg.raft.election_timeout_min_ms,
        cfg.raft.election_timeout_max_ms,
        cfg.raft.heartbeat_interval_ms,
        cfg.raft.install_snapshot_timeout_ms,
        cfg.raft.max_payload_entries,
        cfg.raft.replication_lag_threshold,
        cfg.raft.snapshot_policy,
        cfg.raft.snapshot_max_chunk_size_bytes,
        cfg.raft.max_in_snapshot_log_to_keep,
        cfg.raft.purge_batch_size,
        cfg.admin_local_only,
        cfg.rpc.append.timeout_ms,
        cfg.rpc.append.body_limit_bytes,
        cfg.rpc.append.concurrency,
        cfg.rpc.query.timeout_ms,
        cfg.rpc.query.body_limit_bytes,
        cfg.rpc.query.concurrency,
        cfg.rpc.jsonrpc.timeout_ms,
        cfg.rpc.jsonrpc.body_limit_bytes,
        cfg.rpc.jsonrpc.concurrency
    );
    if cfg.admin_local_only {
        warn!(
            "Admin APIs are restricted to loopback clients; remote cluster join/management requests will be rejected"
        );
    }

    let raft_log_path = cfg.data_dir.join("raft_log.rocks");
    let log_storage = RocksDbLogStorage::open_with_sync(&raft_log_path, cfg.state_store_sync_write)
        .map_err(|e| {
            format!(
                "Failed to open rocksdb raft log storage at {}: {}",
                raft_log_path.display(),
                e
            )
        })?;
    info!(
        "Raft log storage ready: path={}, engine=rocksdb, sync_write={}",
        raft_log_path.display(),
        cfg.state_store_sync_write
    );

    let state_store_path = cfg.data_dir.join("state_store.rocks");
    let state_store = RocksDbStateStore::open_with_mode_and_sync(
        &state_store_path,
        RocksDbSnapshotMode::BackupEngine,
        cfg.state_store_sync_write,
    )
    .map_err(|e| {
        format!(
            "Failed to open state store at {}: {}",
            state_store_path.display(),
            e
        )
    })?;
    info!(
        "State store ready: path={}, snapshot_mode={:?}, sync_write={}",
        state_store_path.display(),
        RocksDbSnapshotMode::BackupEngine,
        cfg.state_store_sync_write
    );
    let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);
    let state_store_manager = Arc::new(
        KLogStateStoreManager::new(state_store)
            .await
            .map_err(|e| format!("Failed to initialize state store manager: {}", e))?,
    );

    let snapshot_manager = Arc::new(SnapshotManager::new(cfg.data_dir.clone()));
    let state_machine = KLogStateMachine::new(state_store_manager.clone(), snapshot_manager)
        .await
        .map_err(|e| format!("Failed to initialize state machine: {}", e))?;

    let raft_config = cfg
        .raft
        .to_openraft_config(cfg.cluster_name.clone())
        .map_err(|e| format!("Invalid openraft config: {}", e))?;
    info!(
        "OpenRaft config ready: cluster_name={}, election_timeout={}..{}, heartbeat_interval={}",
        raft_config.cluster_name,
        raft_config.election_timeout_min,
        raft_config.election_timeout_max,
        raft_config.heartbeat_interval
    );
    let cluster_transport = KClusterTransportConfig {
        mode: cfg.cluster_network.mode,
        gateway_addr: cfg.cluster_network.gateway_addr.clone(),
        gateway_route_prefix: cfg.cluster_network.gateway_route_prefix.clone(),
        ..Default::default()
    };

    let raft = openraft::Raft::new(
        cfg.node_id,
        Arc::new(raft_config),
        KNetworkFactory::new(cfg.node_id, cluster_transport.clone()),
        log_storage,
        state_machine,
    )
    .await
    .map_err(|e| {
        format!(
            "Failed to create raft node raft_node_id={}: {}",
            cfg.node_id, e
        )
    })?;
    let raft = Arc::new(raft);
    info!("Raft node created: raft_node_id={}", cfg.node_id);

    initialize_cluster_if_needed(&cfg, &raft).await;
    let join_task = spawn_auto_join_task(&cfg);

    let network_server = KNetworkServer::new(cfg.listen_addr.clone(), raft.clone())
        .with_inter_node_addr(cfg.inter_node_listen_addr.clone())
        .with_admin_addr(cfg.admin_listen_addr.clone())
        .with_state_store_manager(state_store_manager.clone())
        .with_admin_local_only(cfg.admin_local_only)
        .with_cluster_transport_config(cluster_transport.clone())
        .with_cluster_identity(cfg.cluster_name.clone(), cfg.cluster_id.clone());
    info!(
        "Starting network server: raft_listen_addr={}, inter_node_listen_addr={}, admin_listen_addr={}",
        cfg.listen_addr, cfg.inter_node_listen_addr, cfg.admin_listen_addr
    );

    let rpc_server = if cfg.enable_rpc_server {
        info!(
            "Starting client RPC server: rpc_listen_addr={}",
            cfg.rpc_listen_addr
        );
        Some(
            KRpcServer::new(cfg.rpc_listen_addr.clone(), raft, state_store_manager)
                .with_policy(cfg.rpc.into())
                .with_cluster_transport_config(cluster_transport),
        )
    } else {
        warn!("Client RPC server is disabled by config");
        None
    };

    run_server_lifecycle(network_server, rpc_server, join_task).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_raft_node_id_from_node_name_suffix() {
        assert_eq!(
            derive_raft_node_id_from_node_name("ood12").expect("derive node id from suffix"),
            12
        );
    }

    #[test]
    fn test_derive_raft_node_id_from_node_name_requires_digits() {
        let err = derive_raft_node_id_from_node_name("ood")
            .expect_err("missing numeric suffix should fail");
        assert!(err.contains("missing numeric suffix"));
    }

    #[test]
    fn test_validate_buckyos_cluster_transport_identity_direct_skips_check() {
        validate_buckyos_cluster_transport_identity(KClusterTransportMode::Direct, None, None)
            .expect("direct mode should skip node identity validation");
    }

    #[test]
    fn test_validate_buckyos_cluster_transport_identity_non_direct_requires_match() {
        validate_buckyos_cluster_transport_identity(
            KClusterTransportMode::GatewayProxy,
            Some("ood1"),
            Some("ood1"),
        )
        .expect("matching advertise_node_name should be accepted");

        let err = validate_buckyos_cluster_transport_identity(
            KClusterTransportMode::Hybrid,
            Some("ood2"),
            Some("ood1"),
        )
        .expect_err("mismatched advertise_node_name should be rejected");
        assert!(err.contains("advertise_node_name(BuckyOS node name)=ood2"));
        assert!(err.contains("runtime_node_name=ood1"));
    }
}
