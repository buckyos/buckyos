mod cluster;
mod config;
mod constants;
mod lifecycle;
mod logging;

use cluster::{initialize_cluster_if_needed, spawn_auto_join_task};
use config::KLogRuntimeConfig;
use klog::logs::SqliteLogStorage;
use klog::network::{KNetworkFactory, KNetworkServer};
use klog::rpc::KRpcServer;
use klog::state_machine::{KLogStateMachine, SnapshotManager};
use klog::state_store::{
    KLogStateStore, KLogStateStoreManager, RocksDbSnapshotMode, RocksDbStateStore,
};
use lifecycle::run_server_lifecycle;
use log::{error, info, warn};
use logging::init_logging;
use openraft::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    init_logging();

    let (cfg, source) = match KLogRuntimeConfig::load() {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to load runtime config: {}", e);
            std::process::exit(1);
        }
    };
    info!("klog runtime config source: {}", source);

    if let Err(e) = run(cfg).await {
        error!("klog startup failed: {}", e);
        std::process::exit(1);
    }
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
        "klog startup config: node_id={}, raft_listen_addr={}, inter_node_listen_addr={}, admin_listen_addr={}, rpc_enabled={}, rpc_listen_addr={}, advertise_addr={}, advertise_port={}, advertise_inter_port={}, advertise_admin_port={}, rpc_advertise_port={}, data_dir={}, cluster_name={}, cluster_id={}, auto_bootstrap={}, state_store_sync_write={}, join_targets={:?}, join_retry_interval_ms={}, join_max_attempts={}, join_blocking={}, join_target_role={}, admin_local_only={}, rpc_append(timeout_ms={}, body_limit_bytes={}, concurrency={}), rpc_query(timeout_ms={}, body_limit_bytes={}, concurrency={}), rpc_jsonrpc(timeout_ms={}, body_limit_bytes={}, concurrency={})",
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
        cfg.data_dir.display(),
        cfg.cluster_name,
        cfg.cluster_id,
        cfg.auto_bootstrap,
        cfg.state_store_sync_write,
        cfg.join_targets,
        cfg.join_retry_interval_ms,
        cfg.join_max_attempts,
        cfg.join_blocking,
        cfg.join_target_role,
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

    let raft_log_path = cfg.data_dir.join("raft_log.sqlite");
    let log_storage = SqliteLogStorage::open(&raft_log_path).map_err(|e| {
        format!(
            "Failed to open raft log storage at {}: {}",
            raft_log_path.display(),
            e
        )
    })?;
    info!("Raft log storage ready: {}", raft_log_path.display());

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

    let raft_config = Config {
        cluster_name: cfg.cluster_name.clone(),
        ..Default::default()
    }
    .validate()
    .map_err(|e| format!("Invalid openraft config: {}", e))?;
    info!(
        "OpenRaft config ready: cluster_name={}, election_timeout={}..{}, heartbeat_interval={}",
        raft_config.cluster_name,
        raft_config.election_timeout_min,
        raft_config.election_timeout_max,
        raft_config.heartbeat_interval
    );

    let raft = openraft::Raft::new(
        cfg.node_id,
        Arc::new(raft_config),
        KNetworkFactory::new(cfg.node_id),
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
                .with_policy(cfg.rpc.into()),
        )
    } else {
        warn!("Client RPC server is disabled by config");
        None
    };

    run_server_lifecycle(network_server, rpc_server, join_task).await
}
