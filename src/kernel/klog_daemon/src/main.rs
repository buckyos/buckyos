mod cluster;
mod config;
mod constants;
mod lifecycle;
mod logging;

use buckyos_api::{
    BuckyOSRuntimeType, KLOG_SERVICE_PORT, KLOG_SERVICE_UNIQUE_ID, get_session_token_env_key,
    init_buckyos_api_runtime, set_buckyos_api_runtime,
};
use cluster::{initialize_cluster_if_needed, spawn_auto_join_task};
use config::KLogRuntimeConfig;
use klog::KClusterTransportConfig;
use klog::logs::RocksDbLogStorage;
use klog::network::{KNetworkFactory, KNetworkServer};
use klog::rpc::KRpcServer;
use klog::state_machine::{KLogStateMachine, SnapshotManager};
use klog::state_store::{
    KLogStateStore, KLogStateStoreManager, RocksDbSnapshotMode, RocksDbStateStore,
};
use lifecycle::run_server_lifecycle;
use log::{error, info, warn};
use logging::init_logging;
use std::env;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    init_logging();

    let (mut cfg, source) = match KLogRuntimeConfig::load() {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to load runtime config: {}", e);
            std::process::exit(1);
        }
    };
    info!("klog runtime config source: {}", source);

    if let Err(e) = init_buckyos_runtime_if_needed(&mut cfg).await {
        error!("Failed to initialize BuckyOS runtime integration: {}", e);
        std::process::exit(1);
    }

    if let Err(e) = run(cfg).await {
        error!("klog startup failed: {}", e);
        std::process::exit(1);
    }
}

async fn init_buckyos_runtime_if_needed(cfg: &mut KLogRuntimeConfig) -> Result<(), String> {
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

async fn run(cfg: KLogRuntimeConfig) -> Result<(), String> {
    std::fs::create_dir_all(&cfg.data_dir).map_err(|e| {
        format!(
            "Failed to create data dir {}: {}",
            cfg.data_dir.display(),
            e
        )
    })?;

    info!(
        "klog startup config: node_id={}, raft_listen_addr={}, inter_node_listen_addr={}, admin_listen_addr={}, rpc_enabled={}, rpc_listen_addr={}, advertise_addr={}, advertise_port={}, advertise_inter_port={}, advertise_admin_port={}, rpc_advertise_port={}, advertise_node_name={:?}, data_dir={}, cluster_name={}, cluster_id={}, auto_bootstrap={}, state_store_sync_write={}, cluster_network_mode={}, cluster_gateway_addr={}, cluster_gateway_route_prefix={}, join_targets={:?}, join_blocking={}, join_target_role={}, join_retry(strategy={}, initial_interval_ms={}, max_interval_ms={}, multiplier={}, jitter_ratio={}, max_attempts={}, request_timeout_ms={}, shuffle_targets_each_round={}, config_change_conflict_extra_backoff_ms={}), raft(election_timeout_min_ms={}, election_timeout_max_ms={}, heartbeat_interval_ms={}, install_snapshot_timeout_ms={}, max_payload_entries={}, replication_lag_threshold={}, snapshot_policy={}, snapshot_max_chunk_size_bytes={}, max_in_snapshot_log_to_keep={}, purge_batch_size={}), admin_local_only={}, rpc_append(timeout_ms={}, body_limit_bytes={}, concurrency={}), rpc_query(timeout_ms={}, body_limit_bytes={}, concurrency={}), rpc_jsonrpc(timeout_ms={}, body_limit_bytes={}, concurrency={})",
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
    .map_err(|e| format!("Failed to create raft node {}: {}", cfg.node_id, e))?;
    let raft = Arc::new(raft);
    info!("Raft node created: node_id={}", cfg.node_id);

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
