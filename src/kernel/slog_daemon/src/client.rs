use super::read_manager::{LogReaderManager, LogRecordLoad};
use super::upload::LogUploader;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub struct LogDaemonClient {
    node: String,
    service_endpoint: String,
    log_dir: PathBuf, // The log root directory
    reader_manager: LogReaderManager,
}

impl LogDaemonClient {
    pub fn new(node: String, service_endpoint: String, log_dir: &Path) -> Result<Self, String> {
        let uploader = LogUploader::new(node.clone(), service_endpoint.clone());

        let (tx, rx) = mpsc::channel::<LogRecordLoad>(100);
        let reader_manager = LogReaderManager::open(log_dir, tx).map_err(|e| {
            let msg = format!("failed to open log reader manager: {}", e);
            error!("{}", msg);
            msg
        })?;

        let ret = Self {
            node,
            service_endpoint,
            log_dir: log_dir.to_path_buf(),
            reader_manager,
        };

        tokio::task::spawn(async move {
            Self::run_upload_processor(rx, uploader).await;
        });

        Ok(ret)
    }

    async fn run_upload_processor(mut rx: mpsc::Receiver<LogRecordLoad>, uploader: LogUploader) {
        loop {
            match rx.blocking_recv() {
                Some(load) => {
                    // Upload the records
                    let mut ret = true;
                    if let Err(e) = uploader.upload_logs(&load.id, load.records).await {
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
                    // Channel closed
                    info!("log upload processor channel closed, exiting...");
                    break;
                }
            }
        }
    }
}
