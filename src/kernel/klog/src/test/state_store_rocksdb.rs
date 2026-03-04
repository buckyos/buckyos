use super::common::{decode_entry_ids, sample_membership, sample_state_entries, unique_test_path};
use crate::state_store::{
    KLogQuery, KLogQueryOrder, KLogStateMachineMeta, KLogStateSnapshot, KLogStateStore,
    KLogStateStoreManager, MemoryStateStore, RocksDbSnapshotMode, RocksDbStateStore,
};
use crate::{KLogEntry, KLogMetaEntry};
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
        request_id: None,
        level: Default::default(),
        source: None,
        attrs: Default::default(),
        message: "after-reopen".to_string(),
    });
    assert_eq!(prepared.id, 13);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_request_id_dedup_persists_after_reopen() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_request_dedup_reopen.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;

    let first = manager.prepare_append_entry(KLogEntry {
        id: 0,
        timestamp: 123,
        node_id: 1,
        request_id: Some("rk-dedup-1".to_string()),
        level: Default::default(),
        source: None,
        attrs: Default::default(),
        message: "first-write".to_string(),
    });
    let first_id = manager.append_prepared_entry(first).await?;
    drop(manager);

    let reopened = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let reopened = Arc::new(Box::new(reopened) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(reopened).await?;

    let found = manager.find_recent_request_id("rk-dedup-1").await;
    assert_eq!(found, Some(first_id));

    let retry = manager.prepare_append_entry(KLogEntry {
        id: 0,
        timestamp: 124,
        node_id: 1,
        request_id: Some("rk-dedup-1".to_string()),
        level: Default::default(),
        source: None,
        attrs: Default::default(),
        message: "retry-write".to_string(),
    });
    let retry_id = manager.append_prepared_entry(retry).await?;
    assert_eq!(retry_id, first_id);

    let items = manager
        .query_entries(KLogQuery {
            start_id: Some(first_id),
            end_id: Some(first_id),
            limit: 10,
            order: KLogQueryOrder::Asc,
        })
        .await?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, first_id);
    assert_eq!(items[0].message, "first-write");

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_persists_after_reopen() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_meta_reopen.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: "cluster/config/max_clients".to_string(),
            value: "64".to_string(),
            updated_at: 1000,
            updated_by: 1,
            revision: 0,
        })
        .await?;
    drop(manager);

    let reopened = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let reopened = Arc::new(Box::new(reopened) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(reopened).await?;
    let item = manager
        .get_meta_entry("cluster/config/max_clients")
        .await?
        .expect("meta must exist");
    assert_eq!(item.value, "64");
    assert_eq!(item.updated_by, 1);
    assert_eq!(item.revision, 1);

    let second = manager
        .put_meta_entry(KLogMetaEntry {
            key: "cluster/config/max_clients".to_string(),
            value: "128".to_string(),
            updated_at: 1001,
            updated_by: 1,
            revision: 0,
        })
        .await?;
    assert_eq!(second.revision, 2);

    let listed = manager
        .list_meta_entries(Some("cluster/config"), 10)
        .await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "cluster/config/max_clients");
    assert_eq!(listed[0].revision, 2);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_snapshot_roundtrip() -> anyhow::Result<()> {
    let src = MemoryStateStore::new();
    let src = Arc::new(Box::new(src) as Box<dyn KLogStateStore>);
    let src_mgr = KLogStateStoreManager::new(src).await?;
    src_mgr.append(sample_state_entries()).await?;
    src_mgr
        .put_meta_entry(KLogMetaEntry {
            key: "cluster/config/version".to_string(),
            value: "v1".to_string(),
            updated_at: 2000,
            updated_by: 2,
            revision: 0,
        })
        .await?;
    let src_snapshot = src_mgr.build_snapshot().await?;

    let dst = RocksDbStateStore::open_with_mode(
        unique_test_path("state_store_meta_snapshot.rocks"),
        RocksDbSnapshotMode::Enumerate,
    )
    .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(dst) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;
    dst_mgr.install_snapshot(src_snapshot).await?;

    let item = dst_mgr
        .get_meta_entry("cluster/config/version")
        .await?
        .expect("meta must exist after snapshot install");
    assert_eq!(item.value, "v1");
    assert_eq!(item.updated_by, 2);
    assert_eq!(item.revision, 1);

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
            request_id: None,
            level: Default::default(),
            source: None,
            attrs: Default::default(),
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
        request_id: None,
        level: Default::default(),
        source: None,
        attrs: Default::default(),
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
            request_id: None,
            level: Default::default(),
            source: None,
            attrs: Default::default(),
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
            request_id: None,
            level: Default::default(),
            source: None,
            attrs: Default::default(),
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
            request_id: None,
            level: Default::default(),
            source: None,
            attrs: Default::default(),
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
            request_id: None,
            level: Default::default(),
            source: None,
            attrs: Default::default(),
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
async fn test_rocksdb_query_entries_asc_range_limit() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_query_asc.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    manager
        .append(vec![
            KLogEntry {
                id: 10,
                timestamp: 1,
                node_id: 1,
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m10".to_string(),
            },
            KLogEntry {
                id: 11,
                timestamp: 2,
                node_id: 1,
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m11".to_string(),
            },
            KLogEntry {
                id: 12,
                timestamp: 3,
                node_id: 1,
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m12".to_string(),
            },
            KLogEntry {
                id: 13,
                timestamp: 4,
                node_id: 1,
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m13".to_string(),
            },
        ])
        .await?;

    let items = manager
        .query_entries(KLogQuery {
            start_id: Some(11),
            end_id: Some(13),
            limit: 2,
            order: KLogQueryOrder::Asc,
        })
        .await?;
    let ids = items.into_iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(ids, vec![11, 12]);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_query_entries_desc_range_limit() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_query_desc.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    manager
        .append(vec![
            KLogEntry {
                id: 20,
                timestamp: 1,
                node_id: 1,
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m20".to_string(),
            },
            KLogEntry {
                id: 21,
                timestamp: 2,
                node_id: 1,
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m21".to_string(),
            },
            KLogEntry {
                id: 22,
                timestamp: 3,
                node_id: 1,
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m22".to_string(),
            },
            KLogEntry {
                id: 23,
                timestamp: 4,
                node_id: 1,
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m23".to_string(),
            },
        ])
        .await?;

    let items = manager
        .query_entries(KLogQuery {
            start_id: Some(21),
            end_id: Some(23),
            limit: 2,
            order: KLogQueryOrder::Desc,
        })
        .await?;
    let ids = items.into_iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(ids, vec![23, 22]);

    Ok(())
}
