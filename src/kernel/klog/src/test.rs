use crate::logs::{MemoryLogStorage, SqliteLogStorage};
use crate::state_machine::{KLogMemoryStateMachine, SnapshotManager};
use crate::storage::{KLogStorage, KLogStorageManager, SimpleLogStorage};
use crate::{KNodeId, KTypeConfig, StorageResult};
use openraft::entry::EntryPayload;
use openraft::storage::RaftLogStorage;
use openraft::testing::StoreBuilder;
use openraft::{CommittedLeaderId, Entry, LogId, RaftLogReader, Vote};
use simplelog::{ColorChoice, Config, LevelFilter, SimpleLogger, TermLogger, TerminalMode};
use std::sync::Arc;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing_subscriber::{EnvFilter, fmt};

struct TestMemoryContext {
    log_storage: MemoryLogStorage,
    state_machine: KLogMemoryStateMachine,
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
            state_machine,
        }
    }
}

struct TestMemoryStoreBuilder;

impl TestMemoryStoreBuilder {
    pub fn new() -> Self {
        Self
    }
}

impl StoreBuilder<KTypeConfig, MemoryLogStorage, KLogMemoryStateMachine, ()>
    for TestMemoryStoreBuilder
{
    async fn build(&self) -> StorageResult<((), MemoryLogStorage, KLogMemoryStateMachine)> {
        let context = TestMemoryContext::new().await;
        Ok(((), context.log_storage, context.state_machine))
    }
}

struct TestSqliteContext {
    log_storage: SqliteLogStorage,
    state_machine: KLogMemoryStateMachine,
}

impl TestSqliteContext {
    pub async fn new() -> StorageResult<Self> {
        let log_storage = SqliteLogStorage::open(unique_test_path("sqlite_store.db"))
            .map_err(to_storage_error)?;

        let klog_storage = SimpleLogStorage::new();
        let klog_storage = Arc::new(Box::new(klog_storage) as Box<dyn KLogStorage>);

        let klog_storage_manager = KLogStorageManager::new(klog_storage.clone());
        let klog_storage_manager = Arc::new(klog_storage_manager);

        let data_dir = unique_test_path("sqlite_snapshot");
        std::fs::create_dir_all(&data_dir).map_err(to_storage_error)?;
        info!("Using data dir for sqlite snapshot manager: {:?}", data_dir);

        let snapshot_manager = SnapshotManager::new(data_dir);
        let snapshot_manager = Arc::new(snapshot_manager);
        snapshot_manager.clean_all_snapshots().await?;

        let state_machine =
            KLogMemoryStateMachine::new(klog_storage_manager.clone(), snapshot_manager.clone());

        Ok(Self {
            log_storage,
            state_machine,
        })
    }
}

struct TestSqliteStoreBuilder;

impl TestSqliteStoreBuilder {
    pub fn new() -> Self {
        Self
    }
}

impl StoreBuilder<KTypeConfig, SqliteLogStorage, KLogMemoryStateMachine, ()>
    for TestSqliteStoreBuilder
{
    async fn build(&self) -> StorageResult<((), SqliteLogStorage, KLogMemoryStateMachine)> {
        let context = TestSqliteContext::new().await?;
        Ok(((), context.log_storage, context.state_machine))
    }
}

static LOG_INIT_ONCE: Once = Once::new();
static TEST_ID: AtomicU64 = AtomicU64::new(1);

fn init_test_logging() {
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

fn unique_test_path(name: &str) -> std::path::PathBuf {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "buckyos_klog_{}_{}_{}",
        std::process::id(),
        id,
        name
    ))
}

fn to_storage_error<E: std::fmt::Display>(err: E) -> openraft::StorageError<KNodeId> {
    let io_err = std::io::Error::other(err.to_string());
    openraft::StorageError::IO {
        source: openraft::StorageIOError::write(&io_err),
    }
}

fn blank_entry(term: u64, index: u64) -> Entry<KTypeConfig> {
    Entry {
        log_id: LogId::new(CommittedLeaderId::new(term, 0), index),
        payload: EntryPayload::Blank,
    }
}

#[test]
pub fn test_mem_store() -> anyhow::Result<()> {
    init_test_logging();
    openraft::testing::Suite::test_all(TestMemoryStoreBuilder::new()).unwrap();
    Ok(())
}

#[test]
pub fn test_sqlite_store() -> anyhow::Result<()> {
    init_test_logging();
    openraft::testing::Suite::test_all(TestSqliteStoreBuilder::new()).unwrap();
    Ok(())
}

#[tokio::test]
async fn test_sqlite_and_memory_storage_equivalence() -> anyhow::Result<()> {
    init_test_logging();

    let memory = MemoryLogStorage::new();
    let sqlite =
        SqliteLogStorage::open(unique_test_path("equivalence.db")).map_err(anyhow::Error::msg)?;

    let entries = vec![
        blank_entry(1, 1),
        blank_entry(1, 2),
        blank_entry(1, 3),
        blank_entry(2, 4),
    ];

    memory.append_entries_for_test(entries.clone()).await?;
    sqlite.append_entries_for_test(entries).await?;

    let vote = Vote::<u64>::new(3, 9);

    let mut mem_store = memory.clone();
    mem_store.save_vote(&vote).await?;
    mem_store
        .truncate(LogId::new(CommittedLeaderId::new(2, 0), 4))
        .await?;
    mem_store
        .purge(LogId::new(CommittedLeaderId::new(1, 0), 1))
        .await?;

    let mut sqlite_store = sqlite.clone();
    sqlite_store.save_vote(&vote).await?;
    sqlite_store
        .truncate(LogId::new(CommittedLeaderId::new(2, 0), 4))
        .await?;
    sqlite_store
        .purge(LogId::new(CommittedLeaderId::new(1, 0), 1))
        .await?;

    let mut mem_reader = memory.clone();
    let mut sqlite_reader = sqlite.clone();

    let mem_entries = mem_reader.try_get_log_entries(0..100).await?;
    let sqlite_entries = sqlite_reader.try_get_log_entries(0..100).await?;
    let mem_log_ids: Vec<_> = mem_entries.iter().map(|e| e.log_id).collect();
    let sqlite_log_ids: Vec<_> = sqlite_entries.iter().map(|e| e.log_id).collect();
    assert_eq!(mem_log_ids, sqlite_log_ids);

    let mem_vote = mem_store.read_vote().await?;
    let sqlite_vote = sqlite_store.read_vote().await?;
    assert_eq!(mem_vote, sqlite_vote);

    let mem_state = mem_store.get_log_state().await?;
    let sqlite_state = sqlite_store.get_log_state().await?;
    assert_eq!(mem_state, sqlite_state);

    Ok(())
}
