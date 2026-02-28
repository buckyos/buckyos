mod config;

use config::KLogRuntimeConfig;
use klog::KNode;
use klog::logs::SqliteLogStorage;
use klog::network::{KNetworkFactory, KNetworkServer};
use klog::state_machine::{KLogMemoryStateMachine, SnapshotManager};
use klog::state_store::{
    KLogStateStore, KLogStateStoreManager, RocksDbSnapshotMode, RocksDbStateStore,
};
use log::{error, info, warn};
use openraft::Config;
use simplelog::{ColorChoice, Config as SimpleLogConfig, LevelFilter, TermLogger, TerminalMode};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, fmt};

fn init_logging() {
    let _ = TermLogger::init(
        LevelFilter::Info,
        SimpleLogConfig::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    );

    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

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
        "klog startup config: node_id={}, listen_addr={}, advertise_addr={}:{}, data_dir={}, cluster_name={}, auto_bootstrap={}",
        cfg.node_id,
        cfg.listen_addr,
        cfg.advertise_addr,
        cfg.advertise_port,
        cfg.data_dir.display(),
        cfg.cluster_name,
        cfg.auto_bootstrap
    );

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
    let state_store =
        RocksDbStateStore::open_with_mode(&state_store_path, RocksDbSnapshotMode::BackupEngine)
            .map_err(|e| {
                format!(
                    "Failed to open state store at {}: {}",
                    state_store_path.display(),
                    e
                )
            })?;
    info!(
        "State store ready: path={}, snapshot_mode={:?}",
        state_store_path.display(),
        RocksDbSnapshotMode::BackupEngine
    );
    let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);
    let state_store_manager = Arc::new(
        KLogStateStoreManager::new(state_store)
            .await
            .map_err(|e| format!("Failed to initialize state store manager: {}", e))?,
    );

    let snapshot_manager = Arc::new(SnapshotManager::new(cfg.data_dir.clone()));
    let state_machine = KLogMemoryStateMachine::new(state_store_manager, snapshot_manager);

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

    if cfg.auto_bootstrap {
        let mut members = BTreeMap::new();
        members.insert(
            cfg.node_id,
            KNode {
                id: cfg.node_id,
                addr: cfg.advertise_addr.clone(),
                port: cfg.advertise_port,
            },
        );
        match raft.initialize(members).await {
            Ok(()) => {
                info!(
                    "Raft cluster initialized: node_id={}, cluster_name={}",
                    cfg.node_id, cfg.cluster_name
                );
            }
            Err(e) => {
                warn!(
                    "Raft initialize skipped/failed (possibly already initialized): {}",
                    e
                );
            }
        }
    } else {
        info!("Skip raft initialize because KLOG_AUTO_BOOTSTRAP=false");
    }

    let network_server = KNetworkServer::new(cfg.listen_addr.clone(), raft);
    info!("Starting raft RPC server: listen_addr={}", cfg.listen_addr);
    network_server.run().await
}
