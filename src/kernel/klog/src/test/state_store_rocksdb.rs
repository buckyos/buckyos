use super::common::{decode_entry_ids, sample_membership, sample_state_entries, unique_test_path};
use crate::state_store::{
    KLogStateMachineMeta, KLogStateSnapshot, KLogStateStore, KLogStateStoreManager,
    MemoryStateStore, RocksDbSnapshotMode, RocksDbStateStore,
};
use crate::KLogEntry;
use openraft::{CommittedLeaderId, LogId};
use std::sync::Arc;

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
async fn test_rocksdb_state_store_state_machine_meta_persistence_after_reopen() -> anyhow::Result<()>
{
    let path = unique_test_path("state_store_sm_meta_reopen.rocks");
    let store = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;

    let log_id = LogId::new(CommittedLeaderId::new(9, 2), 88);
    let membership = openraft::StoredMembership::new(Some(log_id), sample_membership(1));
    let meta = KLogStateMachineMeta {
        last_applied_log_id: Some(log_id),
        last_membership: membership.clone(),
    };

    store.save_state_machine_meta(meta.clone()).await?;
    drop(store);

    let reopened = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let loaded = reopened.load_state_machine_meta().await?;

    assert_eq!(loaded, Some(meta));

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
