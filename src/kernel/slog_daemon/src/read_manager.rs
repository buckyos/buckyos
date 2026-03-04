use super::reader::{FlushReadPosError, LogDirReader, LogRecordItem};
use crate::constants::{
    INITIAL_UPLOAD_FAILED_RETRY_INTERVAL_SECS, MAX_UPLOAD_FAILED_RETRY_INTERVAL_SECS,
    READ_RECORD_BATCH_SIZE, READ_RECORD_INTERVAL_MILLIS, UPDATE_DIR_INTERVAL_SECS,
};
use slog::SystemLogRecord;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub struct LogRecordLoad {
    pub id: String,
    pub batch_id: String,
    pub record_ids: Vec<String>,
    pub records: Vec<SystemLogRecord>,
    pub ack: oneshot::Sender<bool>,
}

pub struct LogReaderManager {
    shutdown_token: CancellationToken,
    thread_handle: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct RetryState {
    retry_interval_secs: u64,
    next_retry_at: Instant,
}

impl LogReaderManager {
    pub fn open(
        log_dir: &Path,
        excluded: Vec<String>,
        data_tx: mpsc::Sender<LogRecordLoad>,
        shutdown_token: CancellationToken,
    ) -> Result<Self, String> {
        let dir_reader = LogDirReader::open(log_dir, excluded)?;

        let thread_shutdown_token = shutdown_token.child_token();
        let thread_handle = std::thread::spawn({
            move || {
                info!("starting log reader manager thread...");
                Self::run(data_tx, dir_reader, thread_shutdown_token);
            }
        });

        Ok(LogReaderManager {
            shutdown_token,
            thread_handle: Some(thread_handle),
        })
    }

    pub fn shutdown(mut self) -> Result<(), String> {
        self.shutdown_token.cancel();
        if let Some(handle) = self.thread_handle.take() {
            match handle.join() {
                Ok(_) => {}
                Err(_) => {
                    let msg = "log reader manager thread panicked during shutdown".to_string();
                    error!("{}", msg);
                    return Err(msg);
                }
            }
        }
        Ok(())
    }

    fn run(
        data_tx: mpsc::Sender<LogRecordLoad>,
        dir_reader: LogDirReader,
        shutdown: CancellationToken,
    ) {
        // Update dir every while 1 minute
        let mut last_update_dir_tick = std::time::Instant::now();
        let mut upload_retry_states: HashMap<String, RetryState> = HashMap::new();
        let mut flush_retry_states: HashMap<String, RetryState> = HashMap::new();

        loop {
            if shutdown.is_cancelled() {
                info!("log reader manager received shutdown signal, exiting read loop");
                break;
            }

            Self::retry_pending_flush_read_pos(&dir_reader, &mut flush_retry_states);

            let mut read_count = 0;

            match dir_reader.try_read_records(READ_RECORD_BATCH_SIZE) {
                Ok(items) => {
                    for item in items {
                        if shutdown.is_cancelled() {
                            info!(
                                "stop reading new records due to shutdown signal, pending items stay on disk"
                            );
                            break;
                        }

                        if flush_retry_states.contains_key(&item.id) {
                            debug!(
                                "skip upload for item {} due pending read-pos flush retry",
                                item.id
                            );
                            continue;
                        }

                        if item.flush_only {
                            if let Err(e) = dir_reader.flush_read_pos(&item.id) {
                                Self::handle_flush_read_pos_error(
                                    &mut flush_retry_states,
                                    &item.id,
                                    e,
                                    false,
                                );
                                continue;
                            }

                            flush_retry_states.remove(&item.id);
                            continue;
                        }

                        if !Self::should_attempt_action(&upload_retry_states, &item.id, "upload") {
                            continue;
                        }

                        let upload_ok = match Self::process_record_item(&data_tx, item.clone()) {
                            Ok(ret) => ret,
                            Err(e) => {
                                let msg = format!("failed to process record item: {}", e);
                                error!("{}", msg);
                                false
                            }
                        };

                        if upload_ok {
                            upload_retry_states.remove(&item.id);

                            // Update last read pos
                            if let Err(e) = dir_reader.flush_read_pos(&item.id) {
                                Self::handle_flush_read_pos_error(
                                    &mut flush_retry_states,
                                    &item.id,
                                    e,
                                    false,
                                );
                                continue;
                            }

                            flush_retry_states.remove(&item.id);
                            read_count += item.records.len();
                        } else {
                            Self::record_upload_failure(&mut upload_retry_states, &item.id);
                        }
                    }
                }
                Err(e) => {
                    let msg = format!("failed to read log records: {}", e);
                    error!("{}", msg);
                }
            }

            // If we exactly read the batch size, try read more immediately
            if read_count == READ_RECORD_BATCH_SIZE {
                continue;
            }

            if last_update_dir_tick.elapsed()
                >= std::time::Duration::from_secs(UPDATE_DIR_INTERVAL_SECS)
            {
                if let Err(e) = dir_reader.update_dir() {
                    let msg = format!("failed to update log dir reader: {}", e);
                    error!("{}", msg);
                } else {
                    let active_ids: HashSet<String> =
                        dir_reader.get_active_ids().into_iter().collect();
                    Self::cleanup_retry_states_for_removed_services(
                        &mut upload_retry_states,
                        &active_ids,
                        "upload",
                    );
                    Self::cleanup_retry_states_for_removed_services(
                        &mut flush_retry_states,
                        &active_ids,
                        "flush_read_pos",
                    );
                }
                last_update_dir_tick = std::time::Instant::now();
            }

            // Sleep a while before next read
            if Self::sleep_with_cancellation(
                &shutdown,
                std::time::Duration::from_millis(READ_RECORD_INTERVAL_MILLIS),
            ) {
                info!("log reader manager interrupted during sleep by shutdown signal");
                break;
            }
        }
    }

    fn sleep_with_cancellation(shutdown: &CancellationToken, duration: Duration) -> bool {
        if shutdown.is_cancelled() {
            return true;
        }

        let step = Duration::from_millis(100);
        let started = Instant::now();
        while started.elapsed() < duration {
            if shutdown.is_cancelled() {
                return true;
            }
            let remaining = duration.saturating_sub(started.elapsed());
            std::thread::sleep(remaining.min(step));
        }

        shutdown.is_cancelled()
    }

    fn retry_pending_flush_read_pos(
        dir_reader: &LogDirReader,
        flush_retry_states: &mut HashMap<String, RetryState>,
    ) {
        let ids: Vec<String> = flush_retry_states.keys().cloned().collect();
        for id in ids {
            if !Self::should_attempt_action(flush_retry_states, &id, "flush_read_pos") {
                continue;
            }

            if let Err(e) = dir_reader.flush_read_pos(&id) {
                Self::handle_flush_read_pos_error(flush_retry_states, &id, e, true);
                continue;
            }

            flush_retry_states.remove(&id);
            info!("retry flush read pos succeeded for {}", id);
        }
    }

    fn should_attempt_action(
        retry_states: &HashMap<String, RetryState>,
        id: &str,
        action: &str,
    ) -> bool {
        match retry_states.get(id) {
            Some(state) => {
                if Instant::now() >= state.next_retry_at {
                    true
                } else {
                    let remain = state
                        .next_retry_at
                        .saturating_duration_since(Instant::now())
                        .as_secs();
                    debug!(
                        "skip {} for item {} due retry backoff, next attempt in {}s",
                        action, id, remain
                    );
                    false
                }
            }
            None => true,
        }
    }

    fn next_retry_interval_secs(current: Option<u64>) -> u64 {
        match current {
            None => INITIAL_UPLOAD_FAILED_RETRY_INTERVAL_SECS,
            Some(v) => {
                let doubled = v.saturating_mul(2);
                if doubled > MAX_UPLOAD_FAILED_RETRY_INTERVAL_SECS {
                    MAX_UPLOAD_FAILED_RETRY_INTERVAL_SECS
                } else {
                    doubled
                }
            }
        }
    }

    fn record_action_failure(
        retry_states: &mut HashMap<String, RetryState>,
        id: &str,
        action: &str,
    ) {
        let retry_interval_secs =
            Self::next_retry_interval_secs(retry_states.get(id).map(|s| s.retry_interval_secs));
        let next_retry_at = Instant::now() + Duration::from_secs(retry_interval_secs);

        retry_states.insert(
            id.to_string(),
            RetryState {
                retry_interval_secs,
                next_retry_at,
            },
        );

        warn!(
            "{} failed for item {}, schedule retry in {}s",
            action, id, retry_interval_secs
        );
    }

    fn record_upload_failure(retry_states: &mut HashMap<String, RetryState>, id: &str) {
        Self::record_action_failure(retry_states, id, "upload")
    }

    fn record_flush_failure(retry_states: &mut HashMap<String, RetryState>, id: &str) {
        Self::record_action_failure(retry_states, id, "flush_read_pos")
    }

    fn handle_flush_read_pos_error(
        flush_retry_states: &mut HashMap<String, RetryState>,
        id: &str,
        err: FlushReadPosError,
        is_retrying: bool,
    ) {
        match err {
            FlushReadPosError::NotFound { .. } => {
                flush_retry_states.remove(id);
                warn!(
                    "skip flush_read_pos for removed service {}, clear retry state",
                    id
                );
            }
            FlushReadPosError::FlushFailed { .. } => {
                if is_retrying {
                    error!("failed to retry flush read pos for {}: {}", id, err);
                } else {
                    error!("failed to flush read pos for {}: {}", id, err);
                }
                Self::record_flush_failure(flush_retry_states, id);
            }
        }
    }

    fn cleanup_retry_states_for_removed_services(
        retry_states: &mut HashMap<String, RetryState>,
        active_ids: &HashSet<String>,
        action: &str,
    ) {
        let before = retry_states.len();
        retry_states.retain(|id, _| active_ids.contains(id));
        let removed = before.saturating_sub(retry_states.len());
        if removed > 0 {
            info!(
                "cleaned {} stale {} retry states for removed services",
                removed, action
            );
        }
    }

    fn process_record_item(
        data_tx: &mpsc::Sender<LogRecordLoad>,
        item: LogRecordItem,
    ) -> Result<bool, String> {
        info!(
            "Sending {} log records for id {}",
            item.records.len(),
            item.id
        );

        let (ack_tx, ack_rx) = oneshot::channel::<bool>();
        let load = LogRecordLoad {
            id: item.id.clone(),
            batch_id: item.batch_id,
            record_ids: item.record_ids,
            records: item.records,
            ack: ack_tx,
        };
        if let Err(e) = data_tx.blocking_send(load) {
            let msg = format!("failed to send log records to processor: {}", e);
            error!("{}", msg);
            return Err(msg);
        }

        // Wait for ack
        match ack_rx.blocking_recv() {
            Ok(ret) => Ok(ret),
            Err(e) => {
                let msg = format!("failed to receive ack from processor: {}", e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LogReaderManager, RetryState};
    use crate::reader::FlushReadPosError;
    use std::collections::{HashMap, HashSet};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn test_next_retry_interval_secs_backoff_and_cap() {
        let first = LogReaderManager::next_retry_interval_secs(None);
        let second = LogReaderManager::next_retry_interval_secs(Some(first));
        let third = LogReaderManager::next_retry_interval_secs(Some(second));

        assert_eq!(first, 2);
        assert_eq!(second, 4);
        assert_eq!(third, 8);
    }

    #[test]
    fn test_next_retry_interval_secs_capped_to_max() {
        let max = LogReaderManager::next_retry_interval_secs(Some(90));
        assert_eq!(max, 120);

        let still_max = LogReaderManager::next_retry_interval_secs(Some(max));
        assert_eq!(still_max, 120);
    }

    #[test]
    fn test_should_attempt_action_respects_next_retry_at() {
        let mut retry_states = HashMap::new();
        retry_states.insert(
            "svc".to_string(),
            RetryState {
                retry_interval_secs: 2,
                next_retry_at: Instant::now() + Duration::from_secs(5),
            },
        );

        let should_skip = LogReaderManager::should_attempt_action(&retry_states, "svc", "upload");
        assert!(!should_skip);

        let should_run_for_unknown =
            LogReaderManager::should_attempt_action(&retry_states, "unknown", "upload");
        assert!(should_run_for_unknown);
    }

    #[test]
    fn test_record_flush_failure_backoff_and_cap() {
        let mut retry_states = HashMap::new();
        LogReaderManager::record_flush_failure(&mut retry_states, "svc");
        let first = retry_states.get("svc").unwrap().retry_interval_secs;
        assert_eq!(first, 2);

        LogReaderManager::record_flush_failure(&mut retry_states, "svc");
        let second = retry_states.get("svc").unwrap().retry_interval_secs;
        assert_eq!(second, 4);

        retry_states.insert(
            "svc".to_string(),
            RetryState {
                retry_interval_secs: 120,
                next_retry_at: Instant::now() + Duration::from_secs(120),
            },
        );
        LogReaderManager::record_flush_failure(&mut retry_states, "svc");
        let capped = retry_states.get("svc").unwrap().retry_interval_secs;
        assert_eq!(capped, 120);
    }

    #[test]
    fn test_handle_flush_read_pos_error_not_found_clears_retry_state() {
        let mut retry_states = HashMap::new();
        LogReaderManager::record_flush_failure(&mut retry_states, "svc");
        assert!(retry_states.contains_key("svc"));

        LogReaderManager::handle_flush_read_pos_error(
            &mut retry_states,
            "svc",
            FlushReadPosError::NotFound {
                id: "svc".to_string(),
            },
            true,
        );

        assert!(!retry_states.contains_key("svc"));
    }

    #[test]
    fn test_handle_flush_read_pos_error_flush_failed_records_retry() {
        let mut retry_states = HashMap::new();

        LogReaderManager::handle_flush_read_pos_error(
            &mut retry_states,
            "svc",
            FlushReadPosError::FlushFailed {
                id: "svc".to_string(),
                reason: "io failed".to_string(),
            },
            false,
        );

        let state = retry_states.get("svc").unwrap();
        assert_eq!(state.retry_interval_secs, 2);
        assert!(state.next_retry_at > Instant::now());
    }

    #[test]
    fn test_cleanup_retry_states_for_removed_services_keeps_only_active_ids() {
        let mut retry_states = HashMap::new();
        retry_states.insert(
            "svc_a".to_string(),
            RetryState {
                retry_interval_secs: 2,
                next_retry_at: Instant::now() + Duration::from_secs(2),
            },
        );
        retry_states.insert(
            "svc_b".to_string(),
            RetryState {
                retry_interval_secs: 4,
                next_retry_at: Instant::now() + Duration::from_secs(4),
            },
        );

        let active_ids = HashSet::from([String::from("svc_b"), String::from("svc_c")]);
        LogReaderManager::cleanup_retry_states_for_removed_services(
            &mut retry_states,
            &active_ids,
            "upload",
        );

        assert!(!retry_states.contains_key("svc_a"));
        assert!(retry_states.contains_key("svc_b"));
        assert_eq!(retry_states.len(), 1);
    }

    #[test]
    fn test_log_reader_manager_shutdown_stops_thread() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "buckyos/slog_daemon_shutdown_test/{}_{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&root).unwrap();

        let (tx, _rx) = mpsc::channel(1);
        let shutdown_token = CancellationToken::new();
        let manager = LogReaderManager::open(&root, vec![], tx, shutdown_token).unwrap();
        assert!(manager.shutdown().is_ok());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
