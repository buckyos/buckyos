use super::common::{TestMemoryContext, sample_membership, unique_test_path};
use crate::state_machine::{KLogStateMachine, SnapshotManager};
use crate::state_store::{
    KLogQuery, KLogStateSnapshotData, KLogStateStore, KLogStateStoreManager, MemoryStateStore,
    RocksDbSnapshotMode, RocksDbStateStore,
};
use crate::{KLogEntry, KLogMetaEntry, KLogRequest, KLogResponse};
use openraft::entry::EntryPayload;
use openraft::storage::RaftStateMachine;
use openraft::{CommittedLeaderId, Entry, LogId};
use std::sync::Arc;

#[tokio::test]
async fn test_prepare_append_entry_assigns_id_on_leader_only() -> anyhow::Result<()> {
    let state_store = MemoryStateStore::new();
    let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;

    let no_id_entry = KLogEntry {
        id: 0,
        timestamp: 100,
        node_name: "node-1".to_string(),
        request_id: Some("sm-prepare-1".to_string()),
        level: Default::default(),
        source: None,
        attrs: Default::default(),
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
        node_name: "node-1".to_string(),
        request_id: Some("sm-prepare-2".to_string()),
        level: Default::default(),
        source: None,
        attrs: Default::default(),
        message: "already-has-id".to_string(),
    };
    let prepared_fixed = manager.prepare_append_entry(fixed_id_entry.clone());
    assert_eq!(prepared_fixed.id, 42);

    let snapshot = manager.build_snapshot().await?;
    let (decoded, _): (KLogStateSnapshotData, usize) =
        bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy())?;
    assert_eq!(decoded.entries.len(), 1);
    assert_eq!(decoded.entries[0].id, allocated_id);

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
                node_name: "node-1".to_string(),
                request_id: Some("sm-apply-1".to_string()),
                level: Default::default(),
                source: None,
                attrs: Default::default(),
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
async fn test_state_store_manager_request_id_dedup() -> anyhow::Result<()> {
    let state_store = MemoryStateStore::new();
    let state_store = Arc::new(Box::new(state_store) as Box<dyn KLogStateStore>);
    let manager = KLogStateStoreManager::new(state_store).await?;

    let first = manager.prepare_append_entry(KLogEntry {
        id: 0,
        timestamp: 300,
        node_name: "node-1".to_string(),
        request_id: Some("idem-1".to_string()),
        level: Default::default(),
        source: None,
        attrs: Default::default(),
        message: "first-write".to_string(),
    });
    let first_id = manager.append_prepared_entry(first).await?;

    let retry = manager.prepare_append_entry(KLogEntry {
        id: 0,
        timestamp: 301,
        node_name: "node-1".to_string(),
        request_id: Some("idem-1".to_string()),
        level: Default::default(),
        source: None,
        attrs: Default::default(),
        message: "retry-write-should-dedup".to_string(),
    });
    let retry_id = manager.append_prepared_entry(retry).await?;

    assert_eq!(retry_id, first_id);

    let entries = manager.query_entries(KLogQuery::default()).await?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, first_id);
    assert_eq!(entries[0].message, "first-write");
    assert_eq!(entries[0].request_id.as_deref(), Some("idem-1"));

    Ok(())
}

#[tokio::test]
async fn test_state_machine_recovers_persisted_meta_after_restart() -> anyhow::Result<()> {
    let state_store_path = unique_test_path("state_machine_meta_restart.rocks");
    let snapshot_dir = unique_test_path("state_machine_meta_restart_snapshots");
    std::fs::create_dir_all(&snapshot_dir)?;

    let expected_log_id = LogId::new(CommittedLeaderId::new(3, 1), 2);
    let expected_membership =
        openraft::StoredMembership::new(Some(expected_log_id), sample_membership(1));

    {
        let store =
            RocksDbStateStore::open_with_mode(&state_store_path, RocksDbSnapshotMode::Enumerate)
                .map_err(anyhow::Error::msg)?;
        let store = Arc::new(Box::new(store) as Box<dyn KLogStateStore>);
        let manager = Arc::new(KLogStateStoreManager::new(store).await?);
        let snapshot_manager = Arc::new(SnapshotManager::new(snapshot_dir.clone()));
        let mut sm = KLogStateMachine::new(manager, snapshot_manager).await?;

        let membership = sample_membership(1);
        let entries = vec![
            Entry {
                log_id: LogId::new(CommittedLeaderId::new(3, 1), 1),
                payload: EntryPayload::Blank,
            },
            Entry {
                log_id: expected_log_id,
                payload: EntryPayload::Membership(membership.clone()),
            },
        ];
        sm.apply(entries).await?;

        let (last_applied, last_membership) = sm.applied_state().await?;
        assert_eq!(last_applied, Some(expected_log_id));
        assert_eq!(last_membership, expected_membership);
    }

    let reopened_store =
        RocksDbStateStore::open_with_mode(&state_store_path, RocksDbSnapshotMode::Enumerate)
            .map_err(anyhow::Error::msg)?;
    let reopened_store = Arc::new(Box::new(reopened_store) as Box<dyn KLogStateStore>);
    let reopened_manager = Arc::new(KLogStateStoreManager::new(reopened_store).await?);
    let reopened_snapshot_manager = Arc::new(SnapshotManager::new(snapshot_dir));
    let mut reopened_sm =
        KLogStateMachine::new(reopened_manager, reopened_snapshot_manager).await?;

    let (last_applied, last_membership) = reopened_sm.applied_state().await?;
    assert_eq!(last_applied, Some(expected_log_id));
    assert_eq!(last_membership, expected_membership);

    Ok(())
}

#[tokio::test]
async fn test_state_machine_apply_meta_put_and_delete() -> anyhow::Result<()> {
    let context = TestMemoryContext::new().await;
    let mut sm = context.state_machine;

    let put = Entry {
        log_id: LogId::new(CommittedLeaderId::new(4, 0), 1),
        payload: EntryPayload::Normal(KLogRequest::PutMeta {
            item: KLogMetaEntry {
                key: "cluster/config/epoch".to_string(),
                value: "42".to_string(),
                updated_at: 5000,
                updated_by_node_name: "node-1".to_string(),
                revision: 0,
            },
            expected_revision: None,
        }),
    };
    let del = Entry {
        log_id: LogId::new(CommittedLeaderId::new(4, 0), 2),
        payload: EntryPayload::Normal(KLogRequest::DeleteMeta {
            key: "cluster/config/epoch".to_string(),
        }),
    };

    let responses = sm.apply(vec![put, del]).await?;
    assert_eq!(responses.len(), 2);
    assert!(matches!(
        responses[0],
        KLogResponse::MetaPutOk { revision: 1, .. }
    ));
    assert!(matches!(
        responses[1],
        KLogResponse::MetaDeleteOk {
            existed: true,
            prev_meta: Some(KLogMetaEntry { revision: 1, .. }),
            ..
        }
    ));

    Ok(())
}

#[tokio::test]
async fn test_state_machine_apply_meta_put_with_optional_cas() -> anyhow::Result<()> {
    let context = TestMemoryContext::new().await;
    let mut sm = context.state_machine;

    let create = Entry {
        log_id: LogId::new(CommittedLeaderId::new(5, 0), 1),
        payload: EntryPayload::Normal(KLogRequest::PutMeta {
            item: KLogMetaEntry {
                key: "cluster/config/name".to_string(),
                value: "alpha".to_string(),
                updated_at: 6000,
                updated_by_node_name: "node-1".to_string(),
                revision: 0,
            },
            expected_revision: Some(0),
        }),
    };
    let update = Entry {
        log_id: LogId::new(CommittedLeaderId::new(5, 0), 2),
        payload: EntryPayload::Normal(KLogRequest::PutMeta {
            item: KLogMetaEntry {
                key: "cluster/config/name".to_string(),
                value: "beta".to_string(),
                updated_at: 6001,
                updated_by_node_name: "node-1".to_string(),
                revision: 0,
            },
            expected_revision: Some(1),
        }),
    };
    let conflict = Entry {
        log_id: LogId::new(CommittedLeaderId::new(5, 0), 3),
        payload: EntryPayload::Normal(KLogRequest::PutMeta {
            item: KLogMetaEntry {
                key: "cluster/config/name".to_string(),
                value: "gamma".to_string(),
                updated_at: 6002,
                updated_by_node_name: "node-1".to_string(),
                revision: 0,
            },
            expected_revision: Some(1),
        }),
    };

    let responses = sm.apply(vec![create, update, conflict]).await?;
    assert_eq!(responses.len(), 3);
    assert!(matches!(
        responses[0],
        KLogResponse::MetaPutOk { revision: 1, .. }
    ));
    assert!(matches!(
        responses[1],
        KLogResponse::MetaPutOk { revision: 2, .. }
    ));
    assert!(matches!(
        responses[2],
        KLogResponse::MetaPutConflict {
            expected_revision: 1,
            current_revision: Some(2),
            ..
        }
    ));

    Ok(())
}
