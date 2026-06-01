use super::common::unique_test_path;
use crate::state_store::{
    KLogMetaChangeCursor, KLogMetaChangeQuery, KLogMetaPutResult, KLogMetaTxResult, KLogStateStore,
    KLogStateStoreManager, MemoryStateStore, RocksDbSnapshotMode, RocksDbStateStore,
};
use crate::{KLogMetaEntry, KLogMetaTxAction, KLogMetaTxRequest, KLogMetaVersion};
use std::collections::BTreeMap;
use std::sync::Arc;

#[tokio::test]
async fn test_meta_mvcc_store_matrix_memory_and_rocksdb() -> anyhow::Result<()> {
    run_meta_mvcc_store_matrix(new_memory_manager().await?).await?;
    run_meta_mvcc_store_matrix(new_rocksdb_manager("meta_mvcc_store_matrix.rocks").await?).await?;
    Ok(())
}

async fn new_memory_manager() -> anyhow::Result<KLogStateStoreManager> {
    let state_store = MemoryStateStore::new();
    let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);
    Ok(KLogStateStoreManager::new(state_store).await?)
}

async fn new_rocksdb_manager(name: &str) -> anyhow::Result<KLogStateStoreManager> {
    let rocks =
        RocksDbStateStore::open_with_mode(unique_test_path(name), RocksDbSnapshotMode::Enumerate)
            .map_err(anyhow::Error::msg)?;
    let state_store = Arc::new(Box::new(rocks) as Box<dyn KLogStateStore>);
    Ok(KLogStateStoreManager::new(state_store).await?)
}

async fn run_meta_mvcc_store_matrix(manager: KLogStateStoreManager) -> anyhow::Result<()> {
    let prefix = "system/mvcc/matrix/";
    let key_a = format!("{prefix}a");
    let key_b = format!("{prefix}b");
    let key_c = format!("{prefix}c");
    let key_d = format!("{prefix}d");

    let tx1 = exec_tx(
        &manager,
        [
            (key_a.clone(), put_action(&key_a, "a-v1", 1000, Some(0))),
            (key_b.clone(), put_action(&key_b, "b-v1", 1001, Some(0))),
        ],
    )
    .await?;
    assert_meta_version(tx1.meta_versions.get(&key_a), 1, 1, 1, false);
    assert_meta_version(tx1.meta_versions.get(&key_b), 1, 1, 1, false);
    assert_eq!(manager.meta_revision().await?, 1);

    let a_v2 = manager
        .put_meta_entry_with_expected_revision(meta_entry(&key_a, "a-v2", 1002), Some(1))
        .await?;
    assert!(matches!(
        a_v2,
        KLogMetaPutResult::Stored(KLogMetaEntry {
            revision: 2,
            create_revision: 1,
            mod_revision: 2,
            version: 2,
            ..
        })
    ));

    let deleted_b = manager
        .delete_meta_key(&key_b)
        .await?
        .expect("key_b should exist before delete");
    assert_eq!(deleted_b.revision, 1);
    assert_eq!(manager.current_meta_revision(&key_b).await?, Some(3));
    assert!(manager.get_meta_entry(&key_b).await?.is_none());

    let stale_b = manager
        .put_meta_entry_with_expected_revision(meta_entry(&key_b, "stale", 1003), Some(1))
        .await?;
    assert!(matches!(
        stale_b,
        KLogMetaPutResult::VersionConflict {
            expected_revision: 1,
            current_revision: Some(3),
        }
    ));

    let b_v2 = manager
        .put_meta_entry_with_expected_revision(meta_entry(&key_b, "b-v2", 1004), Some(0))
        .await?;
    assert!(matches!(
        b_v2,
        KLogMetaPutResult::Stored(KLogMetaEntry {
            revision: 4,
            create_revision: 4,
            mod_revision: 4,
            version: 1,
            ..
        })
    ));

    let tx5 = exec_tx(
        &manager,
        [
            (key_a.clone(), put_action(&key_a, "a-v3", 1005, Some(2))),
            (key_c.clone(), put_action(&key_c, "c-v1", 1005, Some(0))),
            (key_d.clone(), put_action(&key_d, "d-v1", 1005, Some(0))),
        ],
    )
    .await?;
    assert_meta_version(tx5.meta_versions.get(&key_a), 1, 5, 3, false);
    assert_meta_version(tx5.meta_versions.get(&key_c), 5, 5, 1, false);
    assert_meta_version(tx5.meta_versions.get(&key_d), 5, 5, 1, false);
    assert_eq!(manager.meta_revision().await?, 5);

    let rev1 = manager
        .list_meta_entries_at_revision(Some(prefix), None, 10, 1)
        .await?;
    assert_meta_values(&rev1, [(&key_a, "a-v1"), (&key_b, "b-v1")]);

    let rev3 = manager
        .list_meta_entries_at_revision(Some(prefix), None, 10, 3)
        .await?;
    assert_meta_values(&rev3, [(&key_a, "a-v2")]);

    let rev5 = manager
        .list_meta_entries_at_revision(Some(prefix), None, 10, 5)
        .await?;
    assert_meta_values(
        &rev5,
        [
            (&key_a, "a-v3"),
            (&key_b, "b-v2"),
            (&key_c, "c-v1"),
            (&key_d, "d-v1"),
        ],
    );

    let first_page = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            prefix: Some(prefix.to_string()),
            limit: 4,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_changes(
        &first_page,
        [
            (1, &key_a, "a-v1", false),
            (1, &key_b, "b-v1", false),
            (2, &key_a, "a-v2", false),
            (3, &key_b, "b-v1", true),
        ],
    );

    let second_page = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            prefix: Some(prefix.to_string()),
            cursor: first_page.last().map(|item| item.change_cursor()),
            limit: 3,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_changes(
        &second_page,
        [
            (4, &key_b, "b-v2", false),
            (5, &key_a, "a-v3", false),
            (5, &key_c, "c-v1", false),
        ],
    );

    let third_page = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            prefix: Some(prefix.to_string()),
            cursor: second_page.last().map(|item| item.change_cursor()),
            limit: 3,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_changes(&third_page, [(5, &key_d, "d-v1", false)]);

    assert_eq!(manager.compact_meta(4).await?, 4);
    assert_eq!(manager.meta_compacted_revision().await?, 4);

    let a_after_compact = manager
        .get_meta_entry_at_revision(&key_a, 5)
        .await?
        .expect("key_a should remain visible after compaction");
    assert_eq!(a_after_compact.value, "a-v3");
    assert_eq!(
        (
            a_after_compact.create_revision,
            a_after_compact.mod_revision,
            a_after_compact.version,
        ),
        (1, 5, 3)
    );

    let b_after_compact = manager
        .get_meta_entry_at_revision(&key_b, 5)
        .await?
        .expect("key_b baseline should remain visible after compaction");
    assert_eq!(b_after_compact.value, "b-v2");
    assert_eq!(
        (
            b_after_compact.create_revision,
            b_after_compact.mod_revision,
            b_after_compact.version,
        ),
        (4, 4, 1)
    );

    let changes_after_compact = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            prefix: Some(prefix.to_string()),
            limit: 10,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_changes(
        &changes_after_compact,
        [
            (5, &key_a, "a-v3", false),
            (5, &key_c, "c-v1", false),
            (5, &key_d, "d-v1", false),
        ],
    );

    let changes_after_old_cursor = manager
        .list_meta_changes(KLogMetaChangeQuery {
            start_revision: 1,
            prefix: Some(prefix.to_string()),
            cursor: Some(KLogMetaChangeCursor {
                revision: 3,
                key: key_b.clone(),
            }),
            limit: 10,
            include_deleted: true,
            ..KLogMetaChangeQuery::default()
        })
        .await?;
    assert_changes(
        &changes_after_old_cursor,
        [
            (5, &key_a, "a-v3", false),
            (5, &key_c, "c-v1", false),
            (5, &key_d, "d-v1", false),
        ],
    );

    let current = manager.list_meta_entries(Some(prefix), None, 10).await?;
    assert_meta_values(
        &current,
        [
            (&key_a, "a-v3"),
            (&key_b, "b-v2"),
            (&key_c, "c-v1"),
            (&key_d, "d-v1"),
        ],
    );

    Ok(())
}

fn meta_entry(key: &str, value: &str, updated_at: u64) -> KLogMetaEntry {
    KLogMetaEntry {
        key: key.to_string(),
        value: value.to_string(),
        updated_at,
        updated_by_node_name: "test-node".to_string(),
        ..KLogMetaEntry::default()
    }
}

fn put_action(
    key: &str,
    value: &str,
    updated_at: u64,
    expected_revision: Option<u64>,
) -> KLogMetaTxAction {
    KLogMetaTxAction::Put {
        item: meta_entry(key, value, updated_at),
        expected_revision,
    }
}

async fn exec_tx<const N: usize>(
    manager: &KLogStateStoreManager,
    actions: [(String, KLogMetaTxAction); N],
) -> anyhow::Result<crate::KLogMetaTxResponse> {
    let result = manager
        .exec_meta_tx(KLogMetaTxRequest {
            actions: BTreeMap::from(actions),
            guard: None,
        })
        .await?;
    match result {
        KLogMetaTxResult::Committed(response) => Ok(response),
        other => panic!("unexpected meta tx result: {other:?}"),
    }
}

fn assert_meta_version(
    version: Option<&KLogMetaVersion>,
    create_revision: u64,
    mod_revision: u64,
    item_version: u64,
    deleted: bool,
) {
    let version = version.expect("meta version should exist");
    assert_eq!(
        (
            version.create_revision,
            version.mod_revision,
            version.version,
            version.deleted,
        ),
        (create_revision, mod_revision, item_version, deleted)
    );
}

fn assert_meta_values<const N: usize>(items: &[KLogMetaEntry], expected: [(&str, &str); N]) {
    let actual = items
        .iter()
        .map(|item| (item.key.as_str(), item.value.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn assert_changes<const N: usize>(
    items: &[crate::state_store::KLogMetaHistoryRecord],
    expected: [(u64, &str, &str, bool); N],
) {
    let actual = items
        .iter()
        .map(|item| {
            (
                item.mod_revision,
                item.key.as_str(),
                item.value.as_str(),
                item.deleted,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}
