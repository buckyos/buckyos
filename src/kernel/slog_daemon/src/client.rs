use super::read_manager::{LogReaderManager, LogRecordLoad};
use super::upload::LogUploader;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub struct LogDaemonClient {
    node: String,
    service_endpoint: String,
    log_dir: PathBuf, // The log root directory
    reader_manager: LogReaderManager,
    upload_task: JoinHandle<()>,
    shutdown_token: CancellationToken,
}

impl LogDaemonClient {
    pub fn new(
        node: String,
        service_endpoint: String,
        upload_timeout_secs: u64,
        log_dir: &Path,
        excluded: Vec<String>,
    ) -> Result<Self, String> {
        let shutdown_token = CancellationToken::new();
        let reader_shutdown = shutdown_token.child_token();
        let uploader_shutdown = shutdown_token.child_token();

        let uploader =
            LogUploader::new(node.clone(), service_endpoint.clone(), upload_timeout_secs);

        let (tx, rx) = mpsc::channel::<LogRecordLoad>(100);
        let reader_manager = LogReaderManager::open(log_dir, excluded, tx, reader_shutdown)
            .map_err(|e| {
                let msg = format!("failed to open log reader manager: {}", e);
                error!("{}", msg);
                msg
            })?;

        let upload_task = tokio::task::spawn(async move {
            Self::run_upload_processor(rx, uploader, uploader_shutdown).await;
        });

        Ok(Self {
            node,
            service_endpoint,
            log_dir: log_dir.to_path_buf(),
            reader_manager,
            upload_task,
            shutdown_token,
        })
    }

    pub async fn shutdown(self) -> Result<(), String> {
        let LogDaemonClient {
            node,
            service_endpoint,
            log_dir,
            reader_manager,
            upload_task,
            shutdown_token,
        } = self;

        info!(
            "shutting down slog daemon client, node={}, endpoint={}, log_dir={}",
            node,
            service_endpoint,
            log_dir.display()
        );

        shutdown_token.cancel();

        tokio::task::spawn_blocking(move || reader_manager.shutdown())
            .await
            .map_err(|e| {
                let msg = format!("log reader manager shutdown task join failed: {}", e);
                error!("{}", msg);
                msg
            })??;

        upload_task.await.map_err(|e| {
            let msg = format!("log upload processor task join failed: {}", e);
            error!("{}", msg);
            msg
        })?;

        info!("slog daemon client shutdown completed");
        Ok(())
    }

    async fn run_upload_processor(
        mut rx: mpsc::Receiver<LogRecordLoad>,
        uploader: LogUploader,
        shutdown: CancellationToken,
    ) {
        let mut draining = false;

        loop {
            tokio::select! {
                _ = shutdown.cancelled(), if !draining => {
                    draining = true;
                    info!("upload processor received shutdown signal, entering drain mode");
                }
                recv_result = rx.recv() => {
                    match recv_result {
                        Some(load) => {
                            // Upload the records
                            let mut ret = true;
                            if let Err(e) = uploader
                                .upload_logs(&load.id, &load.batch_id, load.record_ids, load.records)
                                .await
                            {
                                let msg = format!("failed to upload log records: {}", e);
                                error!("{}", msg);
                                ret = false;
                            }

                            // Send ack
                            if let Err(e) = load.ack.send(ret) {
                                let msg = format!("failed to send ack for uploaded log records: {}", e);
                                error!("{}", msg);
                            }
                        }
                        None => {
                            // Channel closed after reader thread exits, all queued records are drained.
                            info!("log upload processor channel closed, exiting...");
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LogDaemonClient;
    use super::LogUploader;
    use tokio::sync::mpsc;
    use tokio::time::{Duration, timeout};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn test_upload_processor_exits_after_shutdown_and_channel_close() {
        let uploader = LogUploader::new(
            "node-001".to_string(),
            "http://127.0.0.1:22001/logs".to_string(),
            1,
        );
        let (tx, rx) = mpsc::channel(1);
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();

        let handle = tokio::spawn(async move {
            LogDaemonClient::run_upload_processor(rx, uploader, task_shutdown.child_token()).await;
        });

        shutdown.cancel();
        drop(tx);

        let join_result = timeout(Duration::from_secs(2), handle).await;
        assert!(join_result.is_ok());
        assert!(join_result.unwrap().is_ok());
    }
}
