use super::reader::LogDirReader;
use slog::SystemLogRecord;
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

const UPDATE_DIR_INTERVAL_SECS: u64 = 60;
const READ_RECORD_BATCH_SIZE: usize = 100;
const READ_RECORD_INTERVAL_MILLIS: u64 = 1000;
const UPLOAD_FAILED_RETRY_INTERVAL_MILLIS: u64 = 1000 * 10; // 10 seconds retry on upload failed

pub struct LogRecordLoad {
    pub id: String,
    pub records: Vec<SystemLogRecord>,
    pub ack: oneshot::Sender<bool>,
}

pub struct LogReaderManager {}

impl LogReaderManager {
    pub fn open(log_dir: &Path, data_tx: mpsc::Sender<LogRecordLoad>) -> Result<Self, String> {
        let dir_reader = LogDirReader::open(log_dir)?;

        std::thread::spawn({
            move || {
                info!("starting log reader manager thread...");
                Self::run(data_tx, dir_reader);
            }
        });

        Ok(LogReaderManager {})
    }

    fn run(data_tx: mpsc::Sender<LogRecordLoad>, dir_reader: LogDirReader) {
        // Update dir every while 1 minute
        let mut last_update_dir_tick = std::time::Instant::now();
        let mut last_process_failed = false;

        loop {
            let mut read_count = 0;

            if last_process_failed {
                // If last process failed, sleep a while to avoid busy loop
                std::thread::sleep(std::time::Duration::from_secs(
                    UPLOAD_FAILED_RETRY_INTERVAL_MILLIS,
                ));
                last_process_failed = false;
            }

            match dir_reader.try_read_records(READ_RECORD_BATCH_SIZE) {
                Ok(items) => {
                    for item in items {
                        read_count += item.records.len();

                        let (ack_tx, ack_rx) = oneshot::channel::<bool>();
                        let load = LogRecordLoad {
                            id: item.id.clone(),
                            records: item.records,
                            ack: ack_tx,
                        };
                        if let Err(e) = data_tx.blocking_send(load) {
                            let msg = format!("failed to send log records to processor: {}", e);
                            error!("{}", msg);
                            continue;
                        }

                        // Wait for ack
                        match ack_rx.blocking_recv() {
                            Ok(ret) => {
                                if !ret {
                                    last_process_failed = true;
                                    let msg = format!("log processor reported failure");
                                    error!("{}", msg);
                                    continue;
                                }
                            }
                            Err(e) => {
                                let msg = format!("failed to receive ack from processor: {}", e);
                                error!("{}", msg);
                                continue;
                            }
                        }

                        // Update last read pos
                        if let Err(e) = dir_reader.flush_read_pos(&item.id) {
                            let msg = format!("failed to flush read pos: {}", e);
                            error!("{}", msg);
                            continue;
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
                }
                last_update_dir_tick = std::time::Instant::now();
            }

            // Sleep a while before next read
            std::thread::sleep(std::time::Duration::from_millis(
                READ_RECORD_INTERVAL_MILLIS,
            ));
        }
    }
}
