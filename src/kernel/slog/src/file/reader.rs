use super::format::SystemLogRecordLineFormatter;
use super::meta::LogFileReadInfo;
use super::meta::LogMeta;
use crate::system_log::SystemLogRecord;
use std::fs::{File, OpenOptions};
use std::io::Seek;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const CORRUPT_LOG_FILE_NAME: &str = "corrupt.log";
const PARSE_ERROR_ALERT_THRESHOLD_PER_BATCH: usize = 10;

struct ReadFileInfo {
    meta: LogFileReadInfo,
    file: File,
    last_read_index: usize, // The last read index in the current buffer, >= meta.read_index
    last_record_offsets: Vec<i64>,
    last_batch_parse_failures: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReadWindow {
    pub file_id: i64,
    pub start_index: i64,
    pub end_index: i64,
}

pub struct FileLogReader {
    dir: PathBuf,
    meta: LogMeta,
    file: Mutex<Option<ReadFileInfo>>,
}

impl FileLogReader {
    fn append_corrupt_line(&self, source_file: &Path, offset: u64, line: &str, err: &str) {
        let corrupt_path = self.dir.join(CORRUPT_LOG_FILE_NAME);
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let escaped_line = line.replace('\n', "\\n").replace('\r', "\\r");
        let entry = format!(
            "ts_ms={} source={} offset={} err={} line={}\n",
            timestamp_ms,
            source_file.display(),
            offset,
            err,
            escaped_line
        );

        if let Err(e) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&corrupt_path)
            .and_then(|mut f| f.write_all(entry.as_bytes()))
        {
            error!(
                "failed to append corrupt log line to {}: {}",
                corrupt_path.display(),
                e
            );
        }
    }

    pub fn open(log_dir: &Path) -> Result<Self, String> {
        let meta = LogMeta::open(std::path::Path::new(&log_dir)).map_err(|e| {
            let msg = format!(
                "failed to open file log reader at dir: {}, {}",
                log_dir.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        info!("opened file log reader at dir: {}", log_dir.display());

        Ok(FileLogReader {
            dir: log_dir.to_path_buf(),
            meta,
            file: Mutex::new(None),
        })
    }

    fn check_current_read_file(&self) -> Result<bool, String> {
        let mut current_file = self.file.lock().unwrap();

        if let Some(file_id) = current_file.as_ref().map(|cf| cf.meta.id) {
            let info = self.meta.get_file_info(file_id).map_err(|e| {
                let msg = format!("failed to get log file info: {}", e);
                error!("{}", msg);
                msg
            })?;
            if let Some(info) = info {
                if info.read_index >= info.write_index {
                    // Check if the file is sealed by writer
                    if info.is_sealed {
                        // Mark read complete
                        self.meta.mark_file_read_complete(info.id).map_err(|e| {
                            let msg = format!("failed to mark log file read complete: {}", e);
                            error!("{}", msg);
                            msg
                        })?;

                        // Close current file
                        *current_file = None;
                    } else {
                        // No new data to read, but the file is not sealed yet, we should wait
                        return Ok(false);
                    }
                } else {
                    // There is new data to read
                    return Ok(true);
                }
            } else {
                warn!(
                    "current read file metadata missing in db, reset local state: file_id={}",
                    file_id
                );
                *current_file = None;
            }
        }

        loop {
            // Get a new read file
            let read_info = self.meta.get_active_read_file().map_err(|e| {
                let msg = format!("failed to get active read log file: {}", e);
                error!("{}", msg);
                msg
            })?;

            if read_info.is_none() {
                // No new log file to read, so we should wait
                // info!("no active read log file, waiting for new logs");
                return Ok(false);
            }

            let mut read_info = read_info.unwrap();

            // Open the file for reading
            info!("opening log file for reading: {:?}", read_info);

            let file_path = self.dir.join(&read_info.name);
            // Check file exists
            if !file_path.exists() {
                let msg = format!(
                    "log file for reading does not exist: {}",
                    file_path.display()
                );
                warn!("{}", msg);

                // Mark read complete to skip this file
                self.meta
                    .mark_file_read_complete(read_info.id)
                    .map_err(|e| {
                        let msg = format!("failed to mark log file read complete: {}", e);
                        error!("{}", msg);
                        msg
                    })?;

                continue; // Try next file
            }

            let mut file = std::fs::File::open(&file_path).map_err(|e| {
                let msg = format!(
                    "failed to open log file for reading: {}, {}",
                    file_path.display(),
                    e
                );
                error!("{}", msg);
                msg
            })?;

            // Seek to the read index by pos
            if read_info.read_index > 0 {
                info!(
                    "seeking log file to read pos: {}, {}",
                    read_info.read_index,
                    file_path.display()
                );

                // Check if read_index is valid
                let metadata = file.metadata().map_err(|e| {
                    let msg = format!(
                        "failed to get metadata of log file {}: {}",
                        file_path.display(),
                        e
                    );
                    error!("{}", msg);
                    msg
                })?;

                // If read_index is beyond file size, adjust it
                if read_info.read_index as u64 > metadata.len() {
                    let msg = format!(
                        "invalid read index {} for log file {}, file size {}",
                        read_info.read_index,
                        file_path.display(),
                        metadata.len()
                    );
                    warn!("{}", msg);
                    self.meta
                        .update_file_read_index(read_info.id, metadata.len() as i64)
                        .map_err(|e| {
                            let msg = format!(
                                "failed to update read index of log file: {}, {}",
                                file_path.display(),
                                e
                            );
                            error!("{}", msg);
                            msg
                        })?;

                    read_info.read_index = metadata.len() as i64;
                }

                file.seek(std::io::SeekFrom::Start(read_info.read_index as u64))
                    .map_err(|e| {
                        let msg = format!(
                            "failed to seek log file for reading: {}, {}",
                            file_path.display(),
                            e
                        );
                        error!("{}", msg);
                        msg
                    })?;
            }

            *current_file = Some(ReadFileInfo {
                last_read_index: read_info.read_index as usize,
                last_record_offsets: Vec::new(),
                last_batch_parse_failures: 0,
                meta: read_info,
                file,
            });

            break;
        }

        Ok(true)
    }

    pub fn try_read_next_records(&self, batch_size: usize) -> Result<Vec<SystemLogRecord>, String> {
        // Get the active read file
        let has_lines = self.check_current_read_file()?;

        if !has_lines {
            let mut file_lock = self.file.lock().unwrap();
            if let Some(read_info) = file_lock.as_mut() {
                read_info.last_record_offsets.clear();
                read_info.last_batch_parse_failures = 0;
            }
            return Ok(Vec::new());
        }

        let mut file_lock = self.file.lock().unwrap();
        let read_info = file_lock.as_mut().unwrap();
        let file_path = self.dir.join(&read_info.meta.name);

        debug!(
            "reading log records from file: {}, read_index: {}, last_read_index: {}, batch_size: {}",
            file_path.display(),
            read_info.meta.read_index,
            read_info.last_read_index,
            batch_size
        );

        let mut records = Vec::new();
        let mut buf_reader = std::io::BufReader::new(&mut read_info.file);

        // Always seek to read_index, not the last_read_index
        buf_reader
            .seek(std::io::SeekFrom::Start(read_info.meta.read_index as u64))
            .map_err(|e| {
                let msg = format!(
                    "failed to seek log file for reading: {}, {}",
                    file_path.display(),
                    e
                );
                error!("{}", msg);
                msg
            })?;

        let mut line = String::new();
        let mut next_line_offset = read_info.meta.read_index as u64;
        let mut record_offsets = Vec::new();
        let mut parse_failures = 0usize;

        for _ in 0..batch_size {
            line.clear();
            let line_start_offset = next_line_offset;
            let bytes_read = buf_reader.read_line(&mut line).map_err(|e| {
                let msg = format!(
                    "failed to read line from log file: {}, {}",
                    file_path.display(),
                    e
                );
                error!("{}", msg);
                msg
            })?;

            /*
            let pos = buf_reader.seek(std::io::SeekFrom::Current(0)).map_err(|e| {
                let msg = format!(
                    "failed to get current position of log file: {}, {}",
                    file_path.display(),
                    e
                );
                error!("{}", msg);
                msg
            })?;
            debug!(
                "read line from log file: {}, bytes_read: {}, pos: {}",
                file_path.display(),
                bytes_read,
                pos,
            );
            */

            if bytes_read == 0 {
                break; // EOF
            }
            next_line_offset += bytes_read as u64;

            match SystemLogRecordLineFormatter::parse_record(line.trim_end()) {
                Ok(record) => {
                    records.push(record);
                    record_offsets.push(line_start_offset as i64);
                }
                Err(e) => {
                    parse_failures += 1;
                    self.append_corrupt_line(&file_path, line_start_offset, line.trim_end(), &e);
                    warn!(
                        "failed to parse log record line, file={}, offset={}, err={}",
                        file_path.display(),
                        line_start_offset,
                        e
                    );

                    // Skip invalid lines after writing them into corrupt.log
                    continue;
                }
            }
        }

        // Save last read index
        let current_pos = buf_reader.stream_position().map_err(|e| {
            let msg = format!(
                "failed to get current position of log file: {}, {}",
                file_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        info!(
            "read {} lines from log file: {}, current pos: {}",
            records.len(),
            file_path.display(),
            current_pos
        );
        if parse_failures > 0 {
            warn!(
                "encountered {} parse failures in batch for {}, bad lines are written to {}",
                parse_failures,
                file_path.display(),
                self.dir.join(CORRUPT_LOG_FILE_NAME).display()
            );
            if parse_failures >= PARSE_ERROR_ALERT_THRESHOLD_PER_BATCH {
                error!(
                    "high parse-failure rate in one batch: service_dir={}, file={}, parse_failures={}, threshold={}",
                    self.dir.display(),
                    file_path.display(),
                    parse_failures,
                    PARSE_ERROR_ALERT_THRESHOLD_PER_BATCH
                );
            }
        }
        // Just update last_read_index in memory, must call flush_read_index to persist to meta db
        read_info.last_read_index = current_pos as usize;
        read_info.last_record_offsets = record_offsets;
        read_info.last_batch_parse_failures = parse_failures;

        Ok(records)
    }

    pub fn flush_read_index(&self) -> Result<(), String> {
        let mut file_lock = self.file.lock().unwrap();

        if file_lock.is_none() {
            return Ok(());
        }

        let read_info = file_lock.as_mut().unwrap();

        debug!(
            "flushing read index for log file id: {}, last_read_index: {}",
            read_info.meta.id, read_info.last_read_index
        );
        // Update meta db
        self.meta
            .update_file_read_index(read_info.meta.id, read_info.last_read_index as i64)
            .map_err(|e| {
                let msg = format!(
                    "failed to update log file read index: {}, {}",
                    read_info.meta.id, e
                );
                error!("{}", msg);
                msg
            })?;

        // Update in-memory meta
        read_info.meta.read_index = read_info.last_read_index as i64;

        Ok(())
    }

    pub fn get_current_read_window(&self) -> Option<FileReadWindow> {
        let file_lock = self.file.lock().unwrap();
        let read_info = file_lock.as_ref()?;
        Some(FileReadWindow {
            file_id: read_info.meta.id,
            start_index: read_info.meta.read_index,
            end_index: read_info.last_read_index as i64,
        })
    }

    pub fn get_current_batch_record_offsets(&self) -> Option<Vec<i64>> {
        let file_lock = self.file.lock().unwrap();
        let read_info = file_lock.as_ref()?;
        Some(read_info.last_record_offsets.clone())
    }

    pub fn get_current_batch_parse_failures(&self) -> usize {
        let file_lock = self.file.lock().unwrap();
        file_lock
            .as_ref()
            .map(|read_info| read_info.last_batch_parse_failures)
            .unwrap_or(0)
    }
}

fn test_read() {
    let log_dir = crate::get_buckyos_log_root_dir().join("test_slog_service");
    let file = log_dir.join("test_slog_service.1.log");

    // Open the file and read lines
    let mut file = std::fs::File::open(file).unwrap();
    let mut buf_reader = std::io::BufReader::new(&file);

    for i in 0..100 {
        let mut line = String::new();
        let bytes_read = buf_reader.read_line(&mut line).unwrap();
        if bytes_read == 0 {
            break;
        }
        println!("line {}: {}", i, line.trim_end());
    }

    // Get current position
    let pos = buf_reader.seek(std::io::SeekFrom::Current(0)).unwrap();
    println!("current position: {}", pos);

    let pos = file.seek(std::io::SeekFrom::Current(0)).unwrap();
    println!("file current position: {}", pos);
}

#[cfg(test)]
mod tests {
    use super::{FileLogReader, LogMeta, SystemLogRecordLineFormatter};
    use crate::system_log::{LogLevel, SystemLogRecord};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn new_temp_log_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "buckyos/slog_reader_tests/{}_{}_{}",
            prefix,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_record(content: &str) -> SystemLogRecord {
        SystemLogRecord {
            level: LogLevel::Info,
            target: "reader_test".to_string(),
            time: 1721000000000,
            file: Some("reader.rs".to_string()),
            line: Some(42),
            content: content.to_string(),
        }
    }

    #[test]
    fn test_try_read_next_records_handles_missing_meta_entry_without_panic() {
        let log_dir = new_temp_log_dir("missing_meta_entry");
        let meta = LogMeta::open(&log_dir).unwrap();
        let file_name = "svc.1.log";
        meta.append_new_file(file_name).unwrap();

        let line = SystemLogRecordLineFormatter::format_record(&make_record("one-line"));
        let file_path = log_dir.join(file_name);
        std::fs::write(&file_path, &line).unwrap();
        meta.update_current_write_index(line.len() as u64).unwrap();

        let reader = FileLogReader::open(&log_dir).unwrap();
        let first_read = reader.try_read_next_records(10).unwrap();
        assert_eq!(first_read.len(), 1);

        let conn = rusqlite::Connection::open(log_dir.join("log_meta.db")).unwrap();
        conn.execute("DELETE FROM LogFiles WHERE name = ?1", [file_name])
            .unwrap();

        let second_read = reader.try_read_next_records(10);
        assert!(second_read.is_ok());
        assert!(second_read.unwrap().is_empty());

        std::fs::remove_dir_all(&log_dir).unwrap();
    }

    #[test]
    fn test_try_read_next_records_writes_invalid_lines_to_corrupt_log() {
        let log_dir = new_temp_log_dir("invalid_line_corrupt_log");
        let meta = LogMeta::open(&log_dir).unwrap();
        let file_name = "svc_invalid.1.log";
        meta.append_new_file(file_name).unwrap();

        let invalid_line = "invalid-log-line-without-required-format\n";
        let file_path = log_dir.join(file_name);
        std::fs::write(&file_path, invalid_line).unwrap();
        meta.update_current_write_index(invalid_line.len() as u64)
            .unwrap();

        let reader = FileLogReader::open(&log_dir).unwrap();
        let records = reader.try_read_next_records(10).unwrap();
        assert!(records.is_empty());
        assert_eq!(reader.get_current_batch_parse_failures(), 1);

        let window = reader.get_current_read_window().unwrap();
        assert!(window.end_index > window.start_index);

        let corrupt_path = log_dir.join(super::CORRUPT_LOG_FILE_NAME);
        let corrupt_content = std::fs::read_to_string(corrupt_path).unwrap();
        assert!(corrupt_content.contains("offset=0"));
        assert!(corrupt_content.contains("invalid-log-line-without-required-format"));

        reader.flush_read_index().unwrap();
        let second = reader.try_read_next_records(10).unwrap();
        assert!(second.is_empty());
        assert_eq!(reader.get_current_batch_parse_failures(), 0);

        std::fs::remove_dir_all(&log_dir).unwrap();
    }
}
