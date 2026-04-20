use crate::logs::{MemoryLogStorage, RocksDbLogStorage, SqliteLogStorage};
use crate::state_machine::{KLogStateMachine, SnapshotManager};
use crate::state_store::{
    KLogStateSnapshot, KLogStateSnapshotData, KLogStateStore, KLogStateStoreManager,
    MemoryStateStore,
};
use crate::{KLogEntry, KNode, KNodeId, KTypeConfig, StorageResult};
use openraft::entry::EntryPayload;
use openraft::testing::StoreBuilder;
use openraft::{CommittedLeaderId, Entry, LogId, Membership};
use simplelog::{ColorChoice, Config, LevelFilter, SimpleLogger, TermLogger, TerminalMode};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing_subscriber::{EnvFilter, fmt};

pub(crate) struct TestMemoryContext {
    pub(crate) log_storage: MemoryLogStorage,
    pub(crate) state_machine: KLogStateMachine,
}

impl TestMemoryContext {
    pub(crate) async fn new() -> Self {
        let log_storage = MemoryLogStorage::new();

        let state_store = MemoryStateStore::new();
        let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);

        let state_store_manager = KLogStateStoreManager::new(state_store.clone())
            .await
            .unwrap();
        let state_store_manager = Arc::new(state_store_manager);

        let data_dir = unique_test_path("memory_snapshot");
        std::fs::create_dir_all(&data_dir).unwrap();
        info!("Using data dir for snapshot manager: {:?}", data_dir);

        let snapshot_manager = SnapshotManager::new(data_dir);
        let snapshot_manager = Arc::new(snapshot_manager);
        snapshot_manager.clean_all_snapshots().await.unwrap();

        let state_machine = KLogStateMachine::new(state_store_manager, snapshot_manager)
            .await
            .unwrap();

        Self {
            log_storage,
            state_machine,
        }
    }
}

pub(crate) struct TestMemoryStoreBuilder;

impl TestMemoryStoreBuilder {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl StoreBuilder<KTypeConfig, MemoryLogStorage, KLogStateMachine, ()> for TestMemoryStoreBuilder {
    async fn build(&self) -> StorageResult<((), MemoryLogStorage, KLogStateMachine)> {
        let context = TestMemoryContext::new().await;
        Ok(((), context.log_storage, context.state_machine))
    }
}

pub(crate) struct TestSqliteContext {
    pub(crate) log_storage: SqliteLogStorage,
    pub(crate) state_machine: KLogStateMachine,
}

impl TestSqliteContext {
    pub(crate) async fn new() -> StorageResult<Self> {
        let log_storage = SqliteLogStorage::open(unique_test_path("sqlite_store.db"))
            .map_err(to_storage_error)?;

        let state_store = MemoryStateStore::new();
        let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);

        let state_store_manager = KLogStateStoreManager::new(state_store.clone())
            .await
            .map_err(to_storage_error)?;
        let state_store_manager = Arc::new(state_store_manager);

        let data_dir = unique_test_path("sqlite_snapshot");
        std::fs::create_dir_all(&data_dir).map_err(to_storage_error)?;
        info!("Using data dir for sqlite snapshot manager: {:?}", data_dir);

        let snapshot_manager = SnapshotManager::new(data_dir);
        let snapshot_manager = Arc::new(snapshot_manager);
        snapshot_manager.clean_all_snapshots().await?;

        let state_machine = KLogStateMachine::new(state_store_manager, snapshot_manager).await?;

        Ok(Self {
            log_storage,
            state_machine,
        })
    }
}

pub(crate) struct TestSqliteStoreBuilder;

impl TestSqliteStoreBuilder {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl StoreBuilder<KTypeConfig, SqliteLogStorage, KLogStateMachine, ()> for TestSqliteStoreBuilder {
    async fn build(&self) -> StorageResult<((), SqliteLogStorage, KLogStateMachine)> {
        let context = TestSqliteContext::new().await?;
        Ok(((), context.log_storage, context.state_machine))
    }
}

pub(crate) struct TestRocksDbContext {
    pub(crate) log_storage: RocksDbLogStorage,
    pub(crate) state_machine: KLogStateMachine,
}

impl TestRocksDbContext {
    pub(crate) async fn new() -> StorageResult<Self> {
        let log_storage = RocksDbLogStorage::open(unique_test_path("rocksdb_log_store"))
            .map_err(to_storage_error)?;

        let state_store = MemoryStateStore::new();
        let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);

        let state_store_manager = KLogStateStoreManager::new(state_store.clone())
            .await
            .map_err(to_storage_error)?;
        let state_store_manager = Arc::new(state_store_manager);

        let data_dir = unique_test_path("rocksdb_log_snapshot");
        std::fs::create_dir_all(&data_dir).map_err(to_storage_error)?;
        info!(
            "Using data dir for rocksdb log snapshot manager: {:?}",
            data_dir
        );

        let snapshot_manager = SnapshotManager::new(data_dir);
        let snapshot_manager = Arc::new(snapshot_manager);
        snapshot_manager.clean_all_snapshots().await?;

        let state_machine = KLogStateMachine::new(state_store_manager, snapshot_manager).await?;

        Ok(Self {
            log_storage,
            state_machine,
        })
    }
}

pub(crate) struct TestRocksDbStoreBuilder;

impl TestRocksDbStoreBuilder {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl StoreBuilder<KTypeConfig, RocksDbLogStorage, KLogStateMachine, ()>
    for TestRocksDbStoreBuilder
{
    async fn build(&self) -> StorageResult<((), RocksDbLogStorage, KLogStateMachine)> {
        let context = TestRocksDbContext::new().await?;
        Ok(((), context.log_storage, context.state_machine))
    }
}

static LOG_INIT_ONCE: Once = Once::new();
static TEST_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn init_test_logging() {
    LOG_INIT_ONCE.call_once(|| {
        // Set RUST_LOG=trace to see more logs
        unsafe {
            std::env::set_var("RUST_LOG", "trace");
            std::env::set_var("openraft", "trace");
        }

        let _ = TermLogger::init(
            LevelFilter::Debug,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        )
        .or_else(|_| SimpleLogger::init(LevelFilter::Info, Config::default()));

        let subscriber = fmt::Subscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .finish();

        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

pub(crate) fn unique_test_path(name: &str) -> std::path::PathBuf {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "buckyos_klog_{}_{}_{}_{}",
        std::process::id(),
        id,
        nanos,
        name
    ))
}

pub(crate) fn to_storage_error<E: std::fmt::Display>(err: E) -> openraft::StorageError<KNodeId> {
    let io_err = std::io::Error::other(err.to_string());
    openraft::StorageError::IO {
        source: openraft::StorageIOError::write(&io_err),
    }
}

pub(crate) fn blank_entry(term: u64, index: u64) -> Entry<KTypeConfig> {
    Entry {
        log_id: LogId::new(CommittedLeaderId::new(term, 0), index),
        payload: EntryPayload::Blank,
    }
}

pub(crate) fn sample_state_entries() -> Vec<KLogEntry> {
    vec![
        KLogEntry {
            id: 11,
            timestamp: 100,
            node_id: 1,
            request_id: None,
            level: Default::default(),
            source: None,
            attrs: Default::default(),
            message: "kernel-boot".to_string(),
        },
        KLogEntry {
            id: 12,
            timestamp: 101,
            node_id: 1,
            request_id: None,
            level: Default::default(),
            source: None,
            attrs: Default::default(),
            message: "driver-online".to_string(),
        },
    ]
}

pub(crate) fn sample_membership(node_id: KNodeId) -> Membership<KNodeId, KNode> {
    let mut voters = BTreeSet::new();
    voters.insert(node_id);

    let mut nodes = BTreeMap::new();
    nodes.insert(
        node_id,
        KNode {
            id: node_id,
            addr: "127.0.0.1".to_string(),
            port: 3000,
            inter_port: 3002,
            admin_port: 3003,
            rpc_port: 3001,
            node_name: None,
        },
    );

    Membership::new(vec![voters], nodes)
}

pub(crate) fn decode_entry_ids(snapshot: &KLogStateSnapshot) -> anyhow::Result<Vec<u64>> {
    let decoded_new: Result<(KLogStateSnapshotData, usize), _> =
        bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy());
    if let Ok((snapshot_data, _)) = decoded_new {
        return Ok(snapshot_data.entries.into_iter().map(|e| e.id).collect());
    }

    let (decoded, _): (Vec<KLogEntry>, usize) =
        bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy())?;
    Ok(decoded.into_iter().map(|e| e.id).collect())
}
