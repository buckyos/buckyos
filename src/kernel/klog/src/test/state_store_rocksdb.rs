use super::common::{decode_entry_ids, sample_membership, sample_state_entries, unique_test_path};
use crate::state_store::{
    KLogMetaChangeCursor, KLogMetaChangeQuery, KLogMetaPutResult, KLogQuery, KLogQueryOrder,
    KLogStateMachineMeta, KLogStateSnapshot, KLogStateStore, KLogStateStoreManager,
    MemoryStateStore, RocksDbSnapshotMode, RocksDbStateStore,
};
use crate::{KLogEntry, KLogLevel, KLogMetaEntry, KLogMetaTxAction, KLogMetaTxRequest};
use openraft::{CommittedLeaderId, LogId};
use std::collections::BTreeMap;
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
        node_name: "node-1".to_string(),
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
        node_name: "node-1".to_string(),
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
        node_name: "node-1".to_string(),
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
            level: None,
            source: None,
            attr_key: None,
            attr_value: None,
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
            updated_by_node_name: "node-1".to_string(),
            ..KLogMetaEntry::default()
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
    assert_eq!(item.updated_by_node_name, "node-1");
    assert_eq!(item.revision, 1);

    let second = manager
        .put_meta_entry(KLogMetaEntry {
            key: "cluster/config/max_clients".to_string(),
            value: "128".to_string(),
            updated_at: 1001,
            updated_by_node_name: "node-1".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    assert_eq!(second.revision, 2);

    let listed = manager
        .list_meta_entries(Some("cluster/config"), None, 10)
        .await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "cluster/config/max_clients");
    assert_eq!(listed[0].revision, 2);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_list_uses_cursor() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_meta_cursor.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;

    for key in [
        "cluster/config/a",
        "cluster/config/b",
        "cluster/config/c",
        "cluster/other/d",
    ] {
        manager
            .put_meta_entry(KLogMetaEntry {
                key: key.to_string(),
                value: key.to_string(),
                updated_at: 1000,
                updated_by_node_name: "node-1".to_string(),
                ..KLogMetaEntry::default()
            })
            .await?;
    }

    let first = manager
        .list_meta_entries(Some("cluster/config/"), None, 2)
        .await?;
    assert_eq!(
        first
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["cluster/config/a", "cluster/config/b"]
    );

    let second = manager
        .list_meta_entries(Some("cluster/config/"), Some("cluster/config/b"), 2)
        .await?;
    assert_eq!(
        second
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["cluster/config/c"]
    );

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
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
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
    assert_eq!(item.updated_by_node_name, "node-2");
    assert_eq!(item.revision, 1);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_delete_recreate_uses_tombstone_revision() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_meta_tombstone.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    let key = "cluster/config/tombstone";

    let created = manager
        .put_meta_entry_with_expected_revision(
            KLogMetaEntry {
                key: key.to_string(),
                value: "v1".to_string(),
                updated_at: 2100,
                updated_by_node_name: "node-2".to_string(),
                ..KLogMetaEntry::default()
            },
            Some(0),
        )
        .await?;
    assert!(matches!(
        created,
        KLogMetaPutResult::Stored(KLogMetaEntry { revision: 1, .. })
    ));

    let prev = manager
        .delete_meta_key(key)
        .await?
        .expect("delete should return previous entry");
    assert_eq!(prev.revision, 1);
    assert_eq!(manager.current_meta_revision(key).await?, Some(2));
    assert!(manager.get_meta_entry(key).await?.is_none());

    let stale = manager
        .put_meta_entry_with_expected_revision(
            KLogMetaEntry {
                key: key.to_string(),
                value: "stale".to_string(),
                updated_at: 2101,
                updated_by_node_name: "node-2".to_string(),
                ..KLogMetaEntry::default()
            },
            Some(1),
        )
        .await?;
    assert!(matches!(
        stale,
        KLogMetaPutResult::VersionConflict {
            expected_revision: 1,
            current_revision: Some(2)
        }
    ));

    let recreated = manager
        .put_meta_entry_with_expected_revision(
            KLogMetaEntry {
                key: key.to_string(),
                value: "v2".to_string(),
                updated_at: 2102,
                updated_by_node_name: "node-2".to_string(),
                ..KLogMetaEntry::default()
            },
            Some(0),
        )
        .await?;
    assert!(matches!(
        recreated,
        KLogMetaPutResult::Stored(KLogMetaEntry { revision: 3, .. })
    ));

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_history_query_by_revision() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_meta_history.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    let key_a = "cluster/config/history/a";
    let key_b = "cluster/config/history/b";

    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_a.to_string(),
            value: "a-v1".to_string(),
            updated_at: 2300,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_b.to_string(),
            value: "b-v1".to_string(),
            updated_at: 2301,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_a.to_string(),
            value: "a-v2".to_string(),
            updated_at: 2302,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager.delete_meta_key(key_b).await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_b.to_string(),
            value: "b-v2".to_string(),
            updated_at: 2303,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;

    let a_rev1 = manager
        .get_meta_entry_at_revision(key_a, 1)
        .await?
        .expect("key_a should exist at rev 1");
    assert_eq!(a_rev1.value, "a-v1");
    assert_eq!(
        (a_rev1.create_revision, a_rev1.mod_revision, a_rev1.version),
        (1, 1, 1)
    );

    let a_rev3 = manager
        .get_meta_entry_at_revision(key_a, 3)
        .await?
        .expect("key_a should exist at rev 3");
    assert_eq!(a_rev3.value, "a-v2");
    assert_eq!(
        (a_rev3.create_revision, a_rev3.mod_revision, a_rev3.version),
        (1, 3, 2)
    );

    assert!(
        manager
            .get_meta_entry_at_revision(key_b, 4)
            .await?
            .is_none()
    );
    let b_rev5 = manager
        .get_meta_entry_at_revision(key_b, 5)
        .await?
        .expect("key_b should exist after recreate");
    assert_eq!(b_rev5.value, "b-v2");
    assert_eq!(
        (b_rev5.create_revision, b_rev5.mod_revision, b_rev5.version),
        (5, 5, 1)
    );

    let listed_rev2 = manager
        .list_meta_entries_at_revision(Some("cluster/config/history/"), None, 10, 2)
        .await?;
    assert_eq!(
        listed_rev2
            .iter()
            .map(|item| (item.key.as_str(), item.value.as_str()))
            .collect::<Vec<_>>(),
        vec![(key_a, "a-v1"), (key_b, "b-v1")]
    );

    let listed_rev4 = manager
        .list_meta_entries_at_revision(Some("cluster/config/history/"), None, 10, 4)
        .await?;
    assert_eq!(
        listed_rev4
            .iter()
            .map(|item| (item.key.as_str(), item.value.as_str()))
            .collect::<Vec<_>>(),
        vec![(key_a, "a-v2")]
    );

    let listed_after_cursor = manager
        .list_meta_entries_at_revision(Some("cluster/config/history/"), Some(key_a), 10, 5)
        .await?;
    assert_eq!(
        listed_after_cursor
            .iter()
            .map(|item| (item.key.as_str(), item.value.as_str()))
            .collect::<Vec<_>>(),
        vec![(key_b, "b-v2")]
    );

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_change_feed_uses_revision_major_index() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_meta_change_feed.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    let key_a = "cluster/config/change/a";
    let key_b = "cluster/config/change/b";
    let key_aa = "cluster/config/change/aa";
    let key_c = "cluster/config/change/c";
    let key_d = "cluster/config/change/d";

    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_a.to_string(),
            value: "a-v1".to_string(),
            updated_at: 2500,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_b.to_string(),
            value: "b-v1".to_string(),
            updated_at: 2501,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_a.to_string(),
            value: "a-v2".to_string(),
            updated_at: 2502,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager.delete_meta_key(key_b).await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_b.to_string(),
            value: "b-v2".to_string(),
            updated_at: 2503,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;

    let mut actions = BTreeMap::new();
    actions.insert(
        key_aa.to_string(),
        KLogMetaTxAction::Put {
            item: KLogMetaEntry {
                key: key_aa.to_string(),
                value: "aa-v1".to_string(),
                updated_at: 2504,
                updated_by_node_name: "node-2".to_string(),
                ..KLogMetaEntry::default()
            },
            expected_revision: Some(0),
        },
    );
    actions.insert(
        key_c.to_string(),
        KLogMetaTxAction::Put {
            item: KLogMetaEntry {
                key: key_c.to_string(),
                value: "c-v1".to_string(),
                updated_at: 2504,
                updated_by_node_name: "node-2".to_string(),
                ..KLogMetaEntry::default()
            },
            expected_revision: Some(0),
        },
    );
    actions.insert(
        key_d.to_string(),
        KLogMetaTxAction::Put {
            item: KLogMetaEntry {
                key: key_d.to_string(),
                value: "d-v1".to_string(),
                updated_at: 2504,
                updated_by_node_name: "node-2".to_string(),
                ..KLogMetaEntry::default()
            },
            expected_revision: Some(0),
        },
    );
    manager
        .exec_meta_tx(KLogMetaTxRequest {
            actions,
            guard: None,
        })
        .await?;

    let changes = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            limit: 10,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_eq!(
        changes
            .iter()
            .map(|item| (
                item.mod_revision,
                item.key.as_str(),
                item.value.as_str(),
                item.deleted
            ))
            .collect::<Vec<_>>(),
        vec![
            (1, key_a, "a-v1", false),
            (2, key_b, "b-v1", false),
            (3, key_a, "a-v2", false),
            (4, key_b, "b-v1", true),
            (5, key_b, "b-v2", false),
            (6, key_aa, "aa-v1", false),
            (6, key_c, "c-v1", false),
            (6, key_d, "d-v1", false),
        ]
    );

    let live_only = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            limit: 10,
            include_deleted: false,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_eq!(
        live_only
            .iter()
            .map(|item| (item.mod_revision, item.key.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (1, key_a),
            (2, key_b),
            (3, key_a),
            (5, key_b),
            (6, key_aa),
            (6, key_c),
            (6, key_d)
        ]
    );

    let b_changes = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            key: Some(key_b.to_string()),
            limit: 10,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_eq!(
        b_changes
            .iter()
            .map(|item| (item.mod_revision, item.deleted))
            .collect::<Vec<_>>(),
        vec![(2, false), (4, true), (5, false)]
    );

    let page_after_cursor = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            cursor: Some(KLogMetaChangeCursor {
                revision: 3,
                key: key_a.to_string(),
            }),
            limit: 3,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_eq!(
        page_after_cursor
            .iter()
            .map(|item| (item.mod_revision, item.key.as_str()))
            .collect::<Vec<_>>(),
        vec![(4, key_b), (5, key_b), (6, key_aa)]
    );

    let bounded = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 2,
            end_revision: Some(4),
            prefix: Some("cluster/config/change/".to_string()),
            limit: 10,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_eq!(
        bounded
            .iter()
            .map(|item| (item.mod_revision, item.key.as_str(), item.deleted))
            .collect::<Vec<_>>(),
        vec![(2, key_b, false), (3, key_a, false), (4, key_b, true)]
    );

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_compaction_keeps_baselines_and_drops_change_index() -> anyhow::Result<()>
{
    let path = unique_test_path("state_store_meta_compaction.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;
    let key_a = "cluster/config/compact/a";
    let key_b = "cluster/config/compact/b";
    let key_c = "cluster/config/compact/c";

    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_a.to_string(),
            value: "a-v1".to_string(),
            updated_at: 2600,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_b.to_string(),
            value: "b-v1".to_string(),
            updated_at: 2601,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_b.to_string(),
            value: "b-v2".to_string(),
            updated_at: 2602,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    manager
        .put_meta_entry(KLogMetaEntry {
            key: key_c.to_string(),
            value: "c-v1".to_string(),
            updated_at: 2603,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;

    assert_eq!(manager.meta_revision().await?, 4);
    assert_eq!(manager.compact_meta(3).await?, 3);
    assert_eq!(manager.meta_compacted_revision().await?, 3);

    let a_at_rev4 = manager
        .get_meta_entry_at_revision(key_a, 4)
        .await?
        .expect("baseline key_a must survive compaction");
    assert_eq!(
        (a_at_rev4.value.as_str(), a_at_rev4.mod_revision),
        ("a-v1", 1)
    );
    let b_at_rev4 = manager
        .get_meta_entry_at_revision(key_b, 4)
        .await?
        .expect("latest pre-compaction key_b baseline must survive compaction");
    assert_eq!(
        (b_at_rev4.value.as_str(), b_at_rev4.mod_revision),
        ("b-v2", 3)
    );

    let changes = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            limit: 10,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_eq!(
        changes
            .iter()
            .map(|item| (item.mod_revision, item.key.as_str(), item.value.as_str()))
            .collect::<Vec<_>>(),
        vec![(4, key_c, "c-v1")]
    );

    let snapshot = manager.build_snapshot().await?;
    let dst = RocksDbStateStore::open_with_mode(
        unique_test_path("state_store_meta_compaction_snapshot_dst.rocks"),
        RocksDbSnapshotMode::Enumerate,
    )
    .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(dst) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;
    dst_mgr.install_snapshot(snapshot).await?;
    assert_eq!(dst_mgr.meta_compacted_revision().await?, 3);
    assert_eq!(
        dst_mgr
            .get_meta_entry_at_revision(key_a, 4)
            .await?
            .expect("snapshot baseline key_a")
            .value,
        "a-v1"
    );
    assert_eq!(
        dst_mgr
            .list_meta_changes(KLogMetaChangeQuery {
                start_revision: 1,
                limit: 10,
                include_deleted: true,
                ..KLogMetaChangeQuery::default()
            })
            .await?
            .iter()
            .map(|item| item.mod_revision)
            .collect::<Vec<_>>(),
        vec![4]
    );

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_history_survives_enumerate_snapshot() -> anyhow::Result<()> {
    let src = RocksDbStateStore::open_with_mode(
        unique_test_path("state_store_meta_history_snapshot_src.rocks"),
        RocksDbSnapshotMode::Enumerate,
    )
    .map_err(anyhow::Error::msg)?;
    let src = Arc::new(Box::new(src) as Box<dyn KLogStateStore>);
    let src_mgr = KLogStateStoreManager::new(src).await?;
    let key = "cluster/config/history/snapshot";

    src_mgr
        .put_meta_entry(KLogMetaEntry {
            key: key.to_string(),
            value: "v1".to_string(),
            updated_at: 2400,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    src_mgr
        .put_meta_entry(KLogMetaEntry {
            key: key.to_string(),
            value: "v2".to_string(),
            updated_at: 2401,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    src_mgr.delete_meta_key(key).await?;
    src_mgr
        .put_meta_entry(KLogMetaEntry {
            key: key.to_string(),
            value: "v3".to_string(),
            updated_at: 2402,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;

    let snapshot = src_mgr.build_snapshot().await?;
    let dst = RocksDbStateStore::open_with_mode(
        unique_test_path("state_store_meta_history_snapshot_dst.rocks"),
        RocksDbSnapshotMode::Enumerate,
    )
    .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(dst) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;
    dst_mgr.install_snapshot(snapshot).await?;

    let rev1 = dst_mgr
        .get_meta_entry_at_revision(key, 1)
        .await?
        .expect("rev1 must survive snapshot");
    assert_eq!(rev1.value, "v1");
    let rev2 = dst_mgr
        .get_meta_entry_at_revision(key, 2)
        .await?
        .expect("rev2 must survive snapshot");
    assert_eq!(rev2.value, "v2");
    assert!(dst_mgr.get_meta_entry_at_revision(key, 3).await?.is_none());
    let rev4 = dst_mgr
        .get_meta_entry_at_revision(key, 4)
        .await?
        .expect("rev4 must survive snapshot");
    assert_eq!(rev4.value, "v3");
    assert_eq!(
        (rev4.create_revision, rev4.mod_revision, rev4.version),
        (4, 4, 1)
    );
    let changes = dst_mgr
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            limit: 10,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_eq!(
        changes
            .iter()
            .map(|item| (item.mod_revision, item.value.as_str(), item.deleted))
            .collect::<Vec<_>>(),
        vec![
            (1, "v1", false),
            (2, "v2", false),
            (3, "v2", true),
            (4, "v3", false)
        ]
    );

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_meta_tombstone_survives_enumerate_snapshot() -> anyhow::Result<()> {
    let src = RocksDbStateStore::open_with_mode(
        unique_test_path("state_store_meta_tombstone_src.rocks"),
        RocksDbSnapshotMode::Enumerate,
    )
    .map_err(anyhow::Error::msg)?;
    let src = Arc::new(Box::new(src) as Box<dyn KLogStateStore>);
    let src_mgr = KLogStateStoreManager::new(src).await?;
    let key = "cluster/config/tombstone_snapshot";

    src_mgr
        .put_meta_entry(KLogMetaEntry {
            key: key.to_string(),
            value: "v1".to_string(),
            updated_at: 2200,
            updated_by_node_name: "node-2".to_string(),
            ..KLogMetaEntry::default()
        })
        .await?;
    src_mgr.delete_meta_key(key).await?;
    let snapshot = src_mgr.build_snapshot().await?;

    let dst = RocksDbStateStore::open_with_mode(
        unique_test_path("state_store_meta_tombstone_dst.rocks"),
        RocksDbSnapshotMode::Enumerate,
    )
    .map_err(anyhow::Error::msg)?;
    let dst = Arc::new(Box::new(dst) as Box<dyn KLogStateStore>);
    let dst_mgr = KLogStateStoreManager::new(dst).await?;
    dst_mgr.install_snapshot(snapshot).await?;

    assert_eq!(dst_mgr.current_meta_revision(key).await?, Some(2));
    let stale = dst_mgr
        .put_meta_entry_with_expected_revision(
            KLogMetaEntry {
                key: key.to_string(),
                value: "stale".to_string(),
                updated_at: 2201,
                updated_by_node_name: "node-2".to_string(),
                ..KLogMetaEntry::default()
            },
            Some(1),
        )
        .await?;
    assert!(matches!(
        stale,
        KLogMetaPutResult::VersionConflict {
            expected_revision: 1,
            current_revision: Some(2)
        }
    ));

    let recreated = dst_mgr
        .put_meta_entry_with_expected_revision(
            KLogMetaEntry {
                key: key.to_string(),
                value: "v2".to_string(),
                updated_at: 2202,
                updated_by_node_name: "node-2".to_string(),
                ..KLogMetaEntry::default()
            },
            Some(0),
        )
        .await?;
    assert!(matches!(
        recreated,
        KLogMetaPutResult::Stored(KLogMetaEntry { revision: 3, .. })
    ));

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
            node_name: "node-7".to_string(),
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
        node_name: "node-1".to_string(),
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
            node_name: "node-7".to_string(),
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
            node_name: "node-9".to_string(),
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
            node_name: "node-7".to_string(),
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
            node_name: "node-9".to_string(),
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
                node_name: "node-1".to_string(),
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m10".to_string(),
            },
            KLogEntry {
                id: 11,
                timestamp: 2,
                node_name: "node-1".to_string(),
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m11".to_string(),
            },
            KLogEntry {
                id: 12,
                timestamp: 3,
                node_name: "node-1".to_string(),
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m12".to_string(),
            },
            KLogEntry {
                id: 13,
                timestamp: 4,
                node_name: "node-1".to_string(),
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
            level: None,
            source: None,
            attr_key: None,
            attr_value: None,
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
                node_name: "node-1".to_string(),
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m20".to_string(),
            },
            KLogEntry {
                id: 21,
                timestamp: 2,
                node_name: "node-1".to_string(),
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m21".to_string(),
            },
            KLogEntry {
                id: 22,
                timestamp: 3,
                node_name: "node-1".to_string(),
                request_id: None,
                level: Default::default(),
                source: None,
                attrs: Default::default(),
                message: "m22".to_string(),
            },
            KLogEntry {
                id: 23,
                timestamp: 4,
                node_name: "node-1".to_string(),
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
            level: None,
            source: None,
            attr_key: None,
            attr_value: None,
        })
        .await?;
    let ids = items.into_iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(ids, vec![23, 22]);

    Ok(())
}

#[tokio::test]
async fn test_rocksdb_query_entries_with_source_level_and_attrs() -> anyhow::Result<()> {
    let path = unique_test_path("state_store_query_source_level_attrs.rocks");
    let rocks = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;

    let mut attrs_a = BTreeMap::new();
    attrs_a.insert("service".to_string(), "kmsg".to_string());
    attrs_a.insert("pid".to_string(), "42".to_string());
    let mut attrs_b = BTreeMap::new();
    attrs_b.insert("service".to_string(), "kmsg".to_string());
    attrs_b.insert("pid".to_string(), "43".to_string());
    let mut attrs_c = BTreeMap::new();
    attrs_c.insert("service".to_string(), "net".to_string());

    manager
        .append(vec![
            KLogEntry {
                id: 100,
                timestamp: 1,
                node_name: "node-1".to_string(),
                request_id: None,
                level: KLogLevel::Info,
                source: Some("kernel/kmsg".to_string()),
                attrs: attrs_a,
                message: "a".to_string(),
            },
            KLogEntry {
                id: 101,
                timestamp: 2,
                node_name: "node-1".to_string(),
                request_id: None,
                level: KLogLevel::Error,
                source: Some("kernel/kmsg".to_string()),
                attrs: attrs_b,
                message: "b".to_string(),
            },
            KLogEntry {
                id: 102,
                timestamp: 3,
                node_name: "node-1".to_string(),
                request_id: None,
                level: KLogLevel::Warn,
                source: Some("kernel/net".to_string()),
                attrs: attrs_c,
                message: "c".to_string(),
            },
        ])
        .await?;
    drop(manager);

    let reopened = RocksDbStateStore::open_with_mode(&path, RocksDbSnapshotMode::Enumerate)
        .map_err(anyhow::Error::msg)?;
    let reopened = Arc::new(Box::new(reopened) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(reopened).await?;

    let source_items = manager
        .query_entries(KLogQuery {
            start_id: None,
            end_id: None,
            limit: 10,
            order: KLogQueryOrder::Asc,
            level: None,
            source: Some("kernel/kmsg".to_string()),
            attr_key: None,
            attr_value: None,
        })
        .await?;
    assert_eq!(
        source_items.into_iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![100, 101]
    );

    let level_items = manager
        .query_entries(KLogQuery {
            start_id: None,
            end_id: None,
            limit: 10,
            order: KLogQueryOrder::Desc,
            level: Some(KLogLevel::Warn),
            source: None,
            attr_key: None,
            attr_value: None,
        })
        .await?;
    assert_eq!(
        level_items.into_iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![102]
    );

    let attr_items = manager
        .query_entries(KLogQuery {
            start_id: None,
            end_id: None,
            limit: 10,
            order: KLogQueryOrder::Asc,
            level: None,
            source: Some("kernel/kmsg".to_string()),
            attr_key: Some("pid".to_string()),
            attr_value: Some("43".to_string()),
        })
        .await?;
    assert_eq!(
        attr_items.into_iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![101]
    );

    Ok(())
}
