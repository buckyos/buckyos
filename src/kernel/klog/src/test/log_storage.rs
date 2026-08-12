use super::common::{blank_entry, init_test_logging, unique_test_path};
use crate::logs::{MemoryLogStorage, RocksDbLogStorage, SqliteLogStorage};
use openraft::storage::{RaftLogReader, RaftLogStorage};
use openraft::{CommittedLeaderId, LogId, Vote};

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
async fn test_sqlite_committed_log_id_persistence_after_reopen() -> anyhow::Result<()> {
    let path = unique_test_path("sqlite_committed_persistence.db");
    let mut sqlite = SqliteLogStorage::open(&path).map_err(anyhow::Error::msg)?;

    let committed = LogId::new(CommittedLeaderId::new(7, 1), 42);
    sqlite.save_committed(Some(committed)).await?;
    assert_eq!(sqlite.read_committed().await?, Some(committed));
    drop(sqlite);

    let mut reopened = SqliteLogStorage::open(&path).map_err(anyhow::Error::msg)?;
    assert_eq!(reopened.read_committed().await?, Some(committed));

    Ok(())
}

#[tokio::test]
async fn test_log_storage_committed_log_id_is_monotonic() -> anyhow::Result<()> {
    let mut memory = MemoryLogStorage::new();
    let mut sqlite = SqliteLogStorage::open(unique_test_path("sqlite_committed_monotonic.db"))
        .map_err(anyhow::Error::msg)?;
    let mut rocksdb =
        RocksDbLogStorage::open(unique_test_path("rocksdb_committed_monotonic.rocks"))
            .map_err(anyhow::Error::msg)?;

    let high = LogId::new(CommittedLeaderId::new(3, 2), 18);
    let low = LogId::new(CommittedLeaderId::new(2, 2), 15);

    memory.save_committed(Some(high)).await?;
    memory.save_committed(Some(low)).await?;
    assert_eq!(memory.read_committed().await?, Some(high));

    sqlite.save_committed(Some(high)).await?;
    sqlite.save_committed(Some(low)).await?;
    assert_eq!(sqlite.read_committed().await?, Some(high));

    rocksdb.save_committed(Some(high)).await?;
    rocksdb.save_committed(Some(low)).await?;
    assert_eq!(rocksdb.read_committed().await?, Some(high));

    Ok(())
}

#[tokio::test]
async fn test_sqlite_log_reader_range_bounds_match_memory() -> anyhow::Result<()> {
    let memory = MemoryLogStorage::new();
    let sqlite = SqliteLogStorage::open(unique_test_path("sqlite_range_bounds.db"))
        .map_err(anyhow::Error::msg)?;

    let entries = vec![
        blank_entry(1, 1),
        blank_entry(1, 2),
        blank_entry(1, 3),
        blank_entry(1, 4),
        blank_entry(1, 5),
    ];

    memory.append_entries_for_test(entries.clone()).await?;
    sqlite.append_entries_for_test(entries).await?;

    let mut mem_reader = memory.clone();
    let mut sqlite_reader = sqlite.clone();

    let mem_excluded = mem_reader.try_get_log_entries(2..5).await?;
    let sqlite_excluded = sqlite_reader.try_get_log_entries(2..5).await?;
    assert_eq!(
        mem_excluded
            .iter()
            .map(|e| e.log_id.index)
            .collect::<Vec<_>>(),
        sqlite_excluded
            .iter()
            .map(|e| e.log_id.index)
            .collect::<Vec<_>>()
    );

    let mem_included = mem_reader.try_get_log_entries(2..=4).await?;
    let sqlite_included = sqlite_reader.try_get_log_entries(2..=4).await?;
    assert_eq!(
        mem_included
            .iter()
            .map(|e| e.log_id.index)
            .collect::<Vec<_>>(),
        sqlite_included
            .iter()
            .map(|e| e.log_id.index)
            .collect::<Vec<_>>()
    );

    let mem_to_max = mem_reader
        .try_get_log_entries((u64::MAX - 1)..u64::MAX)
        .await?;
    let sqlite_to_max = sqlite_reader
        .try_get_log_entries((u64::MAX - 1)..u64::MAX)
        .await?;
    assert_eq!(mem_to_max.len(), sqlite_to_max.len());
    assert!(sqlite_to_max.is_empty());

    let mem_over_max = mem_reader.try_get_log_entries(u64::MAX..).await?;
    let sqlite_over_max = sqlite_reader.try_get_log_entries(u64::MAX..).await?;
    assert_eq!(mem_over_max.len(), sqlite_over_max.len());
    assert!(sqlite_over_max.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_and_memory_storage_equivalence() -> anyhow::Result<()> {
    init_test_logging();

    let memory = MemoryLogStorage::new();
    let rocksdb = RocksDbLogStorage::open(unique_test_path("raft_log_equivalence.rocks"))
        .map_err(anyhow::Error::msg)?;

    let entries = vec![
        blank_entry(1, 1),
        blank_entry(1, 2),
        blank_entry(1, 3),
        blank_entry(2, 4),
    ];

    memory.append_entries_for_test(entries.clone()).await?;
    rocksdb.append_entries_for_test(entries).await?;

    let vote = Vote::<u64>::new(3, 9);

    let mut mem_store = memory.clone();
    mem_store.save_vote(&vote).await?;
    mem_store
        .truncate(LogId::new(CommittedLeaderId::new(2, 0), 4))
        .await?;
    mem_store
        .purge(LogId::new(CommittedLeaderId::new(1, 0), 1))
        .await?;

    let mut rocks_store = rocksdb.clone();
    rocks_store.save_vote(&vote).await?;
    rocks_store
        .truncate(LogId::new(CommittedLeaderId::new(2, 0), 4))
        .await?;
    rocks_store
        .purge(LogId::new(CommittedLeaderId::new(1, 0), 1))
        .await?;

    let mut mem_reader = memory.clone();
    let mut rocks_reader = rocksdb.clone();

    let mem_entries = mem_reader.try_get_log_entries(0..100).await?;
    let rocks_entries = rocks_reader.try_get_log_entries(0..100).await?;
    let mem_log_ids: Vec<_> = mem_entries.iter().map(|e| e.log_id).collect();
    let rocks_log_ids: Vec<_> = rocks_entries.iter().map(|e| e.log_id).collect();
    assert_eq!(mem_log_ids, rocks_log_ids);

    let mem_vote = mem_store.read_vote().await?;
    let rocks_vote = rocks_store.read_vote().await?;
    assert_eq!(mem_vote, rocks_vote);

    let mem_state = mem_store.get_log_state().await?;
    let rocks_state = rocks_store.get_log_state().await?;
    assert_eq!(mem_state, rocks_state);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_committed_log_id_persistence_after_reopen() -> anyhow::Result<()> {
    let path = unique_test_path("raft_log_committed_persistence.rocks");
    let mut rocks = RocksDbLogStorage::open(&path).map_err(anyhow::Error::msg)?;

    let committed = LogId::new(CommittedLeaderId::new(7, 1), 42);
    rocks.save_committed(Some(committed)).await?;
    assert_eq!(rocks.read_committed().await?, Some(committed));
    drop(rocks);

    let mut reopened = RocksDbLogStorage::open(&path).map_err(anyhow::Error::msg)?;
    assert_eq!(reopened.read_committed().await?, Some(committed));

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_log_reader_range_bounds_match_memory() -> anyhow::Result<()> {
    let memory = MemoryLogStorage::new();
    let rocksdb = RocksDbLogStorage::open(unique_test_path("raft_log_range_bounds.rocks"))
        .map_err(anyhow::Error::msg)?;

    let entries = vec![
        blank_entry(1, 1),
        blank_entry(1, 2),
        blank_entry(1, 3),
        blank_entry(1, 4),
        blank_entry(1, 5),
    ];

    memory.append_entries_for_test(entries.clone()).await?;
    rocksdb.append_entries_for_test(entries).await?;

    let mut mem_reader = memory.clone();
    let mut rocks_reader = rocksdb.clone();

    let mem_excluded = mem_reader.try_get_log_entries(2..5).await?;
    let rocks_excluded = rocks_reader.try_get_log_entries(2..5).await?;
    assert_eq!(
        mem_excluded
            .iter()
            .map(|e| e.log_id.index)
            .collect::<Vec<_>>(),
        rocks_excluded
            .iter()
            .map(|e| e.log_id.index)
            .collect::<Vec<_>>()
    );

    let mem_included = mem_reader.try_get_log_entries(2..=4).await?;
    let rocks_included = rocks_reader.try_get_log_entries(2..=4).await?;
    assert_eq!(
        mem_included
            .iter()
            .map(|e| e.log_id.index)
            .collect::<Vec<_>>(),
        rocks_included
            .iter()
            .map(|e| e.log_id.index)
            .collect::<Vec<_>>()
    );

    let mem_to_max = mem_reader
        .try_get_log_entries((u64::MAX - 1)..u64::MAX)
        .await?;
    let rocks_to_max = rocks_reader
        .try_get_log_entries((u64::MAX - 1)..u64::MAX)
        .await?;
    assert_eq!(mem_to_max.len(), rocks_to_max.len());
    assert!(rocks_to_max.is_empty());

    let mem_over_max = mem_reader.try_get_log_entries(u64::MAX..).await?;
    let rocks_over_max = rocks_reader.try_get_log_entries(u64::MAX..).await?;
    assert_eq!(mem_over_max.len(), rocks_over_max.len());
    assert!(rocks_over_max.is_empty());

    Ok(())
}
