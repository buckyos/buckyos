use super::reader::{LogDirReader, LogRecordItem};
use slog::SystemLogRecord;
use std::path::Path;
use tokio::sync::{mpsc, oneshot};

const UPDATE_DIR_INTERVAL_SECS: u64 = 60;
const READ_RECORD_BATCH_SIZE: usize = 100;
const READ_RECORD_INTERVAL_MILLIS: u64 = 1000;
const MAX_UPLOAD_FAILED_RETRY_INTERVAL_SECS: u64 = 60 * 2; // 2 minutes for max retry interval

pub struct LogRecordLoad {
    pub id: String,
    pub records: Vec<SystemLogRecord>,
    pub ack: oneshot::Sender<bool>,
}

pub struct LogReaderManager {}

impl LogReaderManager {
    pub fn open(
        log_dir: &Path,
        excluded: Vec<String>,
        data_tx: mpsc::Sender<LogRecordLoad>,
    ) -> Result<Self, String> {
        let dir_reader = LogDirReader::open(log_dir, excluded)?;

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

        loop {
            let mut read_count = 0;

            match dir_reader.try_read_records(READ_RECORD_BATCH_SIZE) {
                Ok(items) => {
                    for item in items {
                        // Always return true here, the retry logic is handled in process_record_item_with_retry
                        Self::process_record_item_with_retry(&data_tx, item.clone()).unwrap();

                        read_count += item.records.len();

                        // Update last read pos
                        if let Err(e) = dir_reader.flush_read_pos(&item.id) {
                            let msg = format!("failed to flush read pos: {}", e);
                            error!("{}", msg);
                            // FIXME: What to do here if failed to flush read pos?
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

    fn process_record_item_with_retry(
        data_tx: &mpsc::Sender<LogRecordLoad>,
        item: LogRecordItem,
    ) -> Result<bool, String> {
        let mut retry_interval_in_secs = 2;
        loop {
            match Self::process_record_item(data_tx, item.clone()) {
                Ok(ret) => {
                    if ret {
                        return Ok(true);
                    } else {
                        warn!(
                            "log processor reported failure for item {}, retrying in {} seconds...",
                            item.id, retry_interval_in_secs
                        );
                    }
                }
                Err(e) => {
                    let msg = format!("failed to update record item, retrying...: {}", e);
                    error!("{}", msg);
                }
            }

            retry_interval_in_secs *= 2;
            if retry_interval_in_secs > MAX_UPLOAD_FAILED_RETRY_INTERVAL_SECS {
                retry_interval_in_secs = MAX_UPLOAD_FAILED_RETRY_INTERVAL_SECS;
            }

            std::thread::sleep(std::time::Duration::from_secs(retry_interval_in_secs));
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
