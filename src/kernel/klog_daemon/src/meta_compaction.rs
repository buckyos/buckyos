use crate::config::{KLogMetaCompactionConfig, KLogMetaCompactionPolicy, KLogRuntimeConfig};
use klog::state_store::KLogStateStoreManager;
use klog::{KLogRequest, KLogResponse, KRaftRef};
use log::{debug, info, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

pub fn spawn_auto_meta_compaction_task(
    cfg: &KLogRuntimeConfig,
    raft: &KRaftRef,
    state_store: Arc<KLogStateStoreManager>,
) -> Option<JoinHandle<()>> {
    if !cfg.meta_compaction.enabled {
        info!("Auto meta compaction disabled");
        return None;
    }

    let node_id = cfg.node_id;
    let compaction_cfg = cfg.meta_compaction;
    let raft = raft.clone();
    Some(tokio::spawn(async move {
        run_auto_meta_compaction_loop(node_id, compaction_cfg, raft, state_store).await;
    }))
}

async fn run_auto_meta_compaction_loop(
    node_id: u64,
    cfg: KLogMetaCompactionConfig,
    raft: KRaftRef,
    state_store: Arc<KLogStateStoreManager>,
) {
    info!(
        "Auto meta compaction started: node_id={}, policy={}, retention_revisions={}, check_interval_ms={}, min_compact_gap={}",
        node_id, cfg.policy, cfg.retention_revisions, cfg.check_interval_ms, cfg.min_compact_gap
    );

    let interval = Duration::from_millis(cfg.check_interval_ms);
    loop {
        tokio::time::sleep(interval).await;

        if !is_current_leader(&raft, node_id) {
            debug!(
                "Auto meta compaction skipped on non-leader: node_id={}",
                node_id
            );
            continue;
        }

        if let Err(err) = try_auto_meta_compaction_once(&cfg, &raft, &state_store).await {
            warn!("Auto meta compaction attempt failed: {}", err);
        }
    }
}

fn is_current_leader(raft: &KRaftRef, node_id: u64) -> bool {
    let metrics = raft.metrics().borrow().clone();
    metrics.current_leader == Some(node_id)
}

async fn try_auto_meta_compaction_once(
    cfg: &KLogMetaCompactionConfig,
    raft: &KRaftRef,
    state_store: &KLogStateStoreManager,
) -> Result<(), String> {
    let current_revision = state_store
        .meta_revision()
        .await
        .map_err(|err| format!("read meta_revision failed: {}", err))?;
    let compacted_revision = state_store
        .meta_compacted_revision()
        .await
        .map_err(|err| format!("read meta_compacted_revision failed: {}", err))?;

    let target_revision = match cfg.policy {
        KLogMetaCompactionPolicy::RevisionCount => compute_revision_count_target(
            current_revision,
            compacted_revision,
            cfg.retention_revisions,
            cfg.min_compact_gap,
        ),
    };
    let Some(target_revision) = target_revision else {
        debug!(
            "Auto meta compaction skipped: current_revision={}, compacted_revision={}, retention_revisions={}, min_compact_gap={}",
            current_revision, compacted_revision, cfg.retention_revisions, cfg.min_compact_gap
        );
        return Ok(());
    };

    info!(
        "Auto meta compaction submit: target_revision={}, current_revision={}, compacted_revision={}",
        target_revision, current_revision, compacted_revision
    );
    let response = raft
        .client_write(KLogRequest::CompactMeta {
            revision: target_revision,
        })
        .await
        .map_err(|err| format!("raft client_write CompactMeta failed: {}", err))?;

    match response.data {
        KLogResponse::MetaCompactOk {
            compacted_revision,
            current_revision,
        } => {
            info!(
                "Auto meta compaction succeeded: compacted_revision={}, current_revision={}",
                compacted_revision, current_revision
            );
            Ok(())
        }
        KLogResponse::MetaCompactRejected {
            revision,
            current_revision,
        } => {
            warn!(
                "Auto meta compaction rejected by state machine: revision={}, current_revision={}",
                revision, current_revision
            );
            Ok(())
        }
        KLogResponse::Err(err) => Err(format!("state machine CompactMeta failed: {}", err)),
        other => Err(format!(
            "unexpected CompactMeta response from state machine: {:?}",
            other
        )),
    }
}

fn compute_revision_count_target(
    current_revision: u64,
    compacted_revision: u64,
    retention_revisions: u64,
    min_compact_gap: u64,
) -> Option<u64> {
    let target_revision = current_revision.checked_sub(retention_revisions)?;
    if target_revision == 0 || target_revision <= compacted_revision {
        return None;
    }
    if target_revision.saturating_sub(compacted_revision) < min_compact_gap {
        return None;
    }
    Some(target_revision)
}

#[cfg(test)]
mod tests {
    use super::compute_revision_count_target;

    #[test]
    fn compute_revision_count_target_respects_retention_and_gap() {
        assert_eq!(compute_revision_count_target(100, 0, 80, 10), Some(20));
        assert_eq!(compute_revision_count_target(100, 15, 80, 10), None);
        assert_eq!(compute_revision_count_target(100, 10, 80, 10), Some(20));
        assert_eq!(compute_revision_count_target(80, 0, 80, 10), None);
        assert_eq!(compute_revision_count_target(79, 0, 80, 10), None);
    }
}
