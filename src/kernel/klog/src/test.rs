use crate::logs::{MemoryLogStorage, SqliteLogStorage};
use crate::state_machine::{KLogMemoryStateMachine, SnapshotManager};
use crate::state_store::{
    KLogStateSnapshot, KLogStateStore, KLogStateStoreManager, MemoryStateStore,
    RocksDbSnapshotMode, RocksDbStateStore,
};
use crate::{KLogEntry, KLogRequest, KLogResponse, KNodeId, KTypeConfig, StorageResult};
use openraft::entry::EntryPayload;
use openraft::storage::{RaftLogStorage, RaftStateMachine};
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

        let state_store = MemoryStateStore::new();
        let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);

        let state_store_manager = KLogStateStoreManager::new(state_store.clone())
            .await
            .unwrap();
        let state_store_manager = Arc::new(state_store_manager);

        let data_dir = std::env::temp_dir().join("buckyos_klog_test");
        std::fs::create_dir_all(&data_dir).unwrap();
        info!("Using data dir for snapshot manager: {:?}", data_dir);

        let snapshot_manager = SnapshotManager::new(data_dir);
        let snapshot_manager = Arc::new(snapshot_manager);
        snapshot_manager.clean_all_snapshots().await.unwrap();

        let state_machine =
            KLogMemoryStateMachine::new(state_store_manager.clone(), snapshot_manager.clone());

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

        let state_machine =
            KLogMemoryStateMachine::new(state_store_manager.clone(), snapshot_manager.clone());

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

fn sample_state_entries() -> Vec<KLogEntry> {
    vec![
        KLogEntry {
            id: 11,
            timestamp: 100,
            node_id: 1,
            message: "kernel-boot".to_string(),
        },
        KLogEntry {
            id: 12,
            timestamp: 101,
            node_id: 1,
            message: "driver-online".to_string(),
        },
    ]
}

fn decode_entry_ids(snapshot: &KLogStateSnapshot) -> anyhow::Result<Vec<u64>> {
    let (decoded, _): (Vec<KLogEntry>, usize) =
        bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy())?;
    Ok(decoded.into_iter().map(|e| e.id).collect())
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

#[tokio::test]
async fn test_prepare_append_entry_assigns_id_on_leader_only() -> anyhow::Result<()> {
    let state_store = MemoryStateStore::new();
    let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;

    let no_id_entry = KLogEntry {
        id: 0,
        timestamp: 100,
        node_id: 1,
        message: "leader-alloc-id".to_string(),
    };

    let prepared = manager.prepare_append_entry(no_id_entry);
    assert_ne!(prepared.id, 0);
    let allocated_id = prepared.id;

    let persisted_id = manager.append_prepared_entry(prepared.clone()).await?;
    assert_eq!(persisted_id, allocated_id);

    let fixed_id_entry = KLogEntry {
        id: 42,
        timestamp: 101,
        node_id: 1,
        message: "already-has-id".to_string(),
    };
    let prepared_fixed = manager.prepare_append_entry(fixed_id_entry.clone());
    assert_eq!(prepared_fixed.id, 42);

    let snapshot = manager.build_snapshot().await?;
    let (decoded, _): (Vec<KLogEntry>, usize) =
        bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy())?;
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].id, allocated_id);

    Ok(())
}

#[tokio::test]
async fn test_state_machine_apply_keeps_prepared_id() -> anyhow::Result<()> {
    let context = TestMemoryContext::new().await;
    let mut sm = context.state_machine;

    let prepared_id = 777;
    let entry = Entry {
        log_id: LogId::new(CommittedLeaderId::new(2, 0), 1),
        payload: EntryPayload::Normal(KLogRequest::AppendLog {
            item: KLogEntry {
                id: prepared_id,
                timestamp: 200,
                node_id: 1,
                message: "already-prepared".to_string(),
            },
        }),
    };

    let resps = sm.apply(vec![entry]).await?;
    assert_eq!(resps.len(), 1);
    match &resps[0] {
        KLogResponse::AppendOk { id } => assert_eq!(*id, prepared_id),
        other => panic!("unexpected response: {:?}", other),
    }

    Ok(())
}

#[tokio::test]
async fn test_manager_recovers_next_log_id_after_rocksdb_reopen() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_next_id_reopen.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    manager.append(sample_state_entries()).await?;
    drop(manager);

    let reopened = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let reopened = Arc::new(Box::new(reopened) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(reopened).await?;
    assert_eq!(manager.peek_next_log_id(), 13);

    let prepared = manager.prepare_append_entry(KLogEntry {
        id: 0,
        timestamp: 300,
        node_id: 1,
        message: "after-reopen".to_string(),
    });
    assert_eq!(prepared.id, 13);

    Ok(())
}

#[tokio::test]
async fn test_manager_recovers_next_log_id_from_entries_without_meta() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_next_id_from_entries.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    rocks.append(sample_state_entries()).await?;

    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    assert_eq!(manager.peek_next_log_id(), 13);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_state_store_snapshot_roundtrip() -> anyhow::Result<()> {
    let rocks = RocksDbStateStore::open_with_mode(
        unique_test_path("state_store_roundtrip.rocks"),
        RocksDbSnapshotMode::Enumerate,
    )
    .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;

    manager.append(sample_state_entries()).await?;
    let snapshot = manager.build_snapshot().await?;
    let ids = decode_entry_ids(&snapshot)?;
    assert_eq!(ids, vec![11, 12]);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_state_store_install_snapshot() -> anyhow::Result<()> {
    let src = MemoryStateStore::new();
    let src = Arc::new(Box::new(src) as Box<dyn KLogStateStore>);
    let src_mgr = KLogStateStoreManager::new(src).await?;
    src_mgr.append(sample_state_entries()).await?;
    let src_snapshot = src_mgr.build_snapshot().await?;

    let rocks = RocksDbStateStore::open_with_mode(
        unique_test_path("state_store_install.rocks"),
        RocksDbSnapshotMode::Enumerate,
    )
    .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;

    dst_mgr
        .append(vec![KLogEntry {
            id: 999,
            timestamp: 1,
            node_id: 7,
            message: "old-data".to_string(),
        }])
        .await?;

    dst_mgr
        .install_snapshot(KLogStateSnapshot {
            data: src_snapshot.data.clone(),
        })
        .await?;
    let prepared = dst_mgr.prepare_append_entry(KLogEntry {
        id: 0,
        timestamp: 500,
        node_id: 1,
        message: "after-install-snapshot".to_string(),
    });
    assert_eq!(prepared.id, 13);

    let restored = dst_mgr.build_snapshot().await?;
    let ids = decode_entry_ids(&restored)?;
    assert_eq!(ids, vec![11, 12]);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_checkpoint_mode_snapshot_roundtrip() -> anyhow::Result<()> {
    let src_path = unique_test_path("state_store_checkpoint_roundtrip_src.rocks");
    let src = RocksDbStateStore::open_with_mode(&src_path, RocksDbSnapshotMode::Checkpoint)
        .map_err(anyhow::Error::msg)?;
    let src = Arc::new(Box::new(src) as Box<dyn KLogStateStore>);
    let src_mgr = KLogStateStoreManager::new(src).await?;
    src_mgr.append(sample_state_entries()).await?;
    let snapshot = src_mgr.build_snapshot().await?;

    let dst_path = unique_test_path("state_store_checkpoint_roundtrip_dst.rocks");
    let dst = RocksDbStateStore::open_with_mode(&dst_path, RocksDbSnapshotMode::Checkpoint)
        .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(dst) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;
    dst_mgr
        .append(vec![KLogEntry {
            id: 999,
            timestamp: 1,
            node_id: 7,
            message: "old-data".to_string(),
        }])
        .await?;
    dst_mgr.install_snapshot(snapshot).await?;
    drop(dst_mgr);

    // Reopen in enumerate mode to decode and assert entry ids.
    let verify = RocksDbStateStore::open_with_mode(&dst_path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let verify = Arc::new(Box::new(verify) as Box<dyn KLogStateStore>);
    let verify_mgr = KLogStateStoreManager::new(verify).await?;
    let restored = verify_mgr.build_snapshot().await?;
    let ids = decode_entry_ids(&restored)?;
    assert_eq!(ids, vec![11, 12]);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_checkpoint_mode_install_enumerate_snapshot() -> anyhow::Result<()> {
    let src = MemoryStateStore::new();
    let src = Arc::new(Box::new(src) as Box<dyn KLogStateStore>);
    let src_mgr = KLogStateStoreManager::new(src).await?;
    src_mgr.append(sample_state_entries()).await?;
    let src_snapshot = src_mgr.build_snapshot().await?;

    let dst_path = unique_test_path("state_store_checkpoint_install_enumerate.rocks");
    let dst = RocksDbStateStore::open_with_mode(&dst_path, RocksDbSnapshotMode::Checkpoint)
        .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(dst) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;
    dst_mgr
        .append(vec![KLogEntry {
            id: 500,
            timestamp: 2,
            node_id: 9,
            message: "stale-data".to_string(),
        }])
        .await?;
    dst_mgr.install_snapshot(src_snapshot).await?;
    drop(dst_mgr);

    let verify = RocksDbStateStore::open_with_mode(&dst_path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let verify = Arc::new(Box::new(verify) as Box<dyn KLogStateStore>);
    let verify_mgr = KLogStateStoreManager::new(verify).await?;
    let restored = verify_mgr.build_snapshot().await?;
    let ids = decode_entry_ids(&restored)?;
    assert_eq!(ids, vec![11, 12]);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_backup_engine_mode_snapshot_roundtrip() -> anyhow::Result<()> {
    let src_path = unique_test_path("state_store_backup_roundtrip_src.rocks");
    let src = RocksDbStateStore::open_with_mode(&src_path, RocksDbSnapshotMode::BackupEngine)
        .map_err(anyhow::Error::msg)?;
    let src = Arc::new(Box::new(src) as Box<dyn KLogStateStore>);
    let src_mgr = KLogStateStoreManager::new(src).await?;
    src_mgr.append(sample_state_entries()).await?;
    let snapshot = src_mgr.build_snapshot().await?;

    let dst_path = unique_test_path("state_store_backup_roundtrip_dst.rocks");
    let dst = RocksDbStateStore::open_with_mode(&dst_path, RocksDbSnapshotMode::BackupEngine)
        .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(dst) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;
    dst_mgr
        .append(vec![KLogEntry {
            id: 999,
            timestamp: 1,
            node_id: 7,
            message: "old-data".to_string(),
        }])
        .await?;
    dst_mgr.install_snapshot(snapshot).await?;
    drop(dst_mgr);

    let verify = RocksDbStateStore::open_with_mode(&dst_path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let verify = Arc::new(Box::new(verify) as Box<dyn KLogStateStore>);
    let verify_mgr = KLogStateStoreManager::new(verify).await?;
    let restored = verify_mgr.build_snapshot().await?;
    let ids = decode_entry_ids(&restored)?;
    assert_eq!(ids, vec![11, 12]);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_backup_engine_mode_install_enumerate_snapshot() -> anyhow::Result<()> {
    let src = MemoryStateStore::new();
    let src = Arc::new(Box::new(src) as Box<dyn KLogStateStore>);
    let src_mgr = KLogStateStoreManager::new(src).await?;
    src_mgr.append(sample_state_entries()).await?;
    let src_snapshot = src_mgr.build_snapshot().await?;

    let dst_path = unique_test_path("state_store_backup_install_enumerate.rocks");
    let dst = RocksDbStateStore::open_with_mode(&dst_path, RocksDbSnapshotMode::BackupEngine)
        .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(dst) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;
    dst_mgr
        .append(vec![KLogEntry {
            id: 501,
            timestamp: 2,
            node_id: 9,
            message: "stale-data".to_string(),
        }])
        .await?;
    dst_mgr.install_snapshot(src_snapshot).await?;
    drop(dst_mgr);

    let verify = RocksDbStateStore::open_with_mode(&dst_path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let verify = Arc::new(Box::new(verify) as Box<dyn KLogStateStore>);
    let verify_mgr = KLogStateStoreManager::new(verify).await?;
    let restored = verify_mgr.build_snapshot().await?;
    let ids = decode_entry_ids(&restored)?;
    assert_eq!(ids, vec![11, 12]);

    Ok(())
}
