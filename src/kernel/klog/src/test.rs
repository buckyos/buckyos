use crate::logs::MemoryLogStorage;
use crate::state_machine::{KLogMemoryStateMachine, SnapshotManager, SnapshotManagerRef};
use crate::storage::{
    KLogStorage, KLogStorageManager, KLogStorageManagerRef, KLogStorageRef, SimpleLogStorage,
};
use crate::{KTypeConfig, StorageResult};
use openraft::testing::StoreBuilder;
use simplelog::{ColorChoice, Config, LevelFilter, SimpleLogger, TermLogger, TerminalMode};
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, fmt};

struct TestMemoryContext {
    log_storage: MemoryLogStorage,
    klog_storage: KLogStorageRef,
    klog_storage_manager: KLogStorageManagerRef,
    state_machine: KLogMemoryStateMachine,
    snapshot_manager: SnapshotManagerRef,
}

impl TestMemoryContext {
    pub async fn new() -> Self {
        let log_storage = MemoryLogStorage::new();

        let klog_storage = SimpleLogStorage::new();
        let klog_storage = Arc::new(Box::new(klog_storage) as Box<dyn KLogStorage>);

        let klog_storage_manager = KLogStorageManager::new(klog_storage.clone());
        let klog_storage_manager = Arc::new(klog_storage_manager);

        let data_dir = std::env::temp_dir().join("buckyos_klog_test");
        std::fs::create_dir_all(&data_dir).unwrap();
        info!("Using data dir for snapshot manager: {:?}", data_dir);

        let snapshot_manager = SnapshotManager::new(data_dir);
        let snapshot_manager = Arc::new(snapshot_manager);
        snapshot_manager.clean_all_snapshots().await.unwrap();

        let state_machine =
            KLogMemoryStateMachine::new(klog_storage_manager.clone(), snapshot_manager.clone());

        Self {
            log_storage,
            klog_storage,
            klog_storage_manager,
            state_machine,
            snapshot_manager,
        }
    }
}

struct TestStoreBuilder {}

impl TestStoreBuilder {
    pub fn new() -> Self {
        Self {}
    }
}

impl StoreBuilder<KTypeConfig, MemoryLogStorage, KLogMemoryStateMachine, ()> for TestStoreBuilder {
    async fn build(&self) -> StorageResult<((), MemoryLogStorage, KLogMemoryStateMachine)> {
        let context = TestMemoryContext::new().await;
        Ok(((), context.log_storage, context.state_machine))
    }
}

#[test]
pub fn test_mem_store() -> anyhow::Result<()> {
    // Set RUST_LOG=trace to see more logs
    unsafe {
        std::env::set_var("RUST_LOG", "trace");
        std::env::set_var("openraft", "trace");
    }

    TermLogger::init(
        LevelFilter::Debug,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .unwrap_or_else(|_| {
        // If TermLogger is not available (e.g., in some environments), fall back to SimpleLogger
        let _ = SimpleLogger::init(LevelFilter::Info, Config::default());
    });

    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set a global tracing subscriber");

    openraft::testing::Suite::test_all(TestStoreBuilder::new()).unwrap();

    Ok(())
}
