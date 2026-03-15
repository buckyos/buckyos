use crate::constants::READ_RECORD_PER_SERVICE_QUOTA;
use slog::{FileLogReader, FileReadWindow, SystemLogRecord};
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// Log records read from a log directory, identified by id as service name(directory name)
#[derive(Clone)]
pub struct LogRecordItem {
    pub records: Vec<SystemLogRecord>,
    pub id: String,
    pub batch_id: String,
    pub record_ids: Vec<String>,
    pub flush_only: bool,
}

struct LogDirItem {
    id: String, // Unique ID for the log directory， could be the directory name
    path: PathBuf,
    reader: FileLogReader,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushReadPosError {
    NotFound { id: String },
    FlushFailed { id: String, reason: String },
}

impl fmt::Display for FlushReadPosError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlushReadPosError::NotFound { id } => {
                write!(f, "log dir id not found when updating read pos: {}", id)
            }
            FlushReadPosError::FlushFailed { id, reason } => {
                write!(f, "failed to flush read pos for {}: {}", id, reason)
            }
        }
    }
}

pub struct LogDirReader {
    dir: PathBuf,
    excluded: Vec<String>,
    list: Mutex<Vec<LogDirItem>>,
    next_start_index: Mutex<usize>,
}

impl LogDirReader {
    fn make_batch_id(id: &str, window: &FileReadWindow) -> String {
        format!(
            "{}:{}:{}:{}",
            id, window.file_id, window.start_index, window.end_index
        )
    }

    fn make_fallback_batch_id(id: &str, records: &[SystemLogRecord]) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut hasher);
        for record in records {
            record.time.hash(&mut hasher);
            (record.level as u8).hash(&mut hasher);
            record.target.hash(&mut hasher);
            record.file.hash(&mut hasher);
            record.line.hash(&mut hasher);
            record.content.hash(&mut hasher);
        }
        format!("{}:fallback:{:x}", id, hasher.finish())
    }

    fn make_record_ids(id: &str, window: &FileReadWindow, offsets: &[i64]) -> Vec<String> {
        offsets
            .iter()
            .map(|offset| format!("{}:{}:{}", id, window.file_id, offset))
            .collect()
    }

    pub fn open(log_dir: &Path, excluded: Vec<String>) -> Result<Self, String> {
        info!(
            "opening log dir reader at dir: {}, excluded: {:?}",
            log_dir.display(),
            excluded
        );

        let reader = LogDirReader {
            dir: log_dir.to_path_buf(),
            excluded,
            list: Mutex::new(Vec::new()),
            next_start_index: Mutex::new(0),
        };

        reader.update_dir()?;

        Ok(reader)
    }

    pub fn try_read_records(&self, batch_size: usize) -> Result<Vec<LogRecordItem>, String> {
        let blocked_ids = HashSet::new();
        self.try_read_records_with_blocked(batch_size, &blocked_ids)
    }

    pub fn try_read_records_with_blocked(
        &self,
        batch_size: usize,
        blocked_ids: &HashSet<String>,
    ) -> Result<Vec<LogRecordItem>, String> {
        let mut result = Vec::new();
        if batch_size == 0 {
            return Ok(result);
        }

        let mut list_lock = self.list.lock().unwrap();
        if list_lock.is_empty() {
            return Ok(result);
        }

        let list_len = list_lock.len();
        let start_idx = {
            let mut idx_lock = self.next_start_index.lock().unwrap();
            if *idx_lock >= list_len {
                *idx_lock = 0;
            }
            *idx_lock
        };

        let mut remain_size = batch_size;
        let mut visited = 0usize;
        let mut offset = 0usize;
        let mut last_processed_idx: Option<usize> = None;

        while visited < list_len && remain_size > 0 {
            let idx = (start_idx + offset) % list_len;
            offset += 1;
            visited += 1;
            last_processed_idx = Some(idx);

            let item = &mut list_lock[idx];
            if blocked_ids.contains(&item.id) {
                continue;
            }
            let service_batch_size = remain_size.min(READ_RECORD_PER_SERVICE_QUOTA);
            match item.reader.try_read_next_records(service_batch_size) {
                Ok(records) => {
                    let window = item.reader.get_current_read_window();
                    let parse_failures = item.reader.get_current_batch_parse_failures();
                    if !records.is_empty() {
                        assert!(remain_size >= records.len());
                        remain_size -= records.len();
                        let (batch_id, record_ids) = if let Some(window) = window.as_ref() {
                            let offsets = item
                                .reader
                                .get_current_batch_record_offsets()
                                .unwrap_or_default();
                            if offsets.len() == records.len() {
                                (
                                    Self::make_batch_id(&item.id, &window),
                                    Self::make_record_ids(&item.id, &window, &offsets),
                                )
                            } else {
                                (Self::make_fallback_batch_id(&item.id, &records), Vec::new())
                            }
                        } else {
                            (Self::make_fallback_batch_id(&item.id, &records), Vec::new())
                        };

                        result.push(LogRecordItem {
                            records,
                            id: item.id.clone(),
                            batch_id,
                            record_ids,
                            flush_only: false,
                        });
                    } else if parse_failures > 0
                        && window
                            .as_ref()
                            .map(|w| w.end_index > w.start_index)
                            .unwrap_or(false)
                    {
                        warn!(
                            "all lines in batch failed parse for service {}, schedule flush-only read-index update",
                            item.id
                        );
                        result.push(LogRecordItem {
                            records: Vec::new(),
                            id: item.id.clone(),
                            batch_id: String::new(),
                            record_ids: Vec::new(),
                            flush_only: true,
                        });
                    }
                }
                Err(e) => {
                    let msg = format!(
                        "failed to read records from log dir {}: {}",
                        item.path.display(),
                        e
                    );
                    error!("{}", msg);
                    continue;
                }
            }
        }

        if let Some(last_idx) = last_processed_idx {
            let mut idx_lock = self.next_start_index.lock().unwrap();
            *idx_lock = (last_idx + 1) % list_len;
        }

        Ok(result)
    }

    pub fn flush_read_pos(&self, id: &str) -> Result<(), FlushReadPosError> {
        let mut list_lock = self.list.lock().unwrap();
        for item in list_lock.iter_mut() {
            if item.id == id {
                return item.reader.flush_read_index().map_err(|e| {
                    FlushReadPosError::FlushFailed {
                        id: id.to_string(),
                        reason: e,
                    }
                });
            }
        }

        let err = FlushReadPosError::NotFound { id: id.to_string() };
        warn!("{}", err);
        Err(err)
    }

    pub fn update_dir(&self) -> Result<(), String> {
        let log_dirs = self.scan_dir(&self.dir)?;
        let log_dir_set: HashSet<PathBuf> = log_dirs.iter().cloned().collect();

        let mut list_lock = self.list.lock().unwrap();

        for dir in log_dirs {
            let exists = list_lock.iter().any(|item| item.path == dir);
            if !exists {
                match FileLogReader::open(&dir) {
                    Ok(reader) => {
                        info!("opened log dir reader at dir: {}", dir.display());

                        let item = LogDirItem {
                            id: dir.file_name().unwrap().to_string_lossy().to_string(),
                            path: dir.clone(),
                            reader,
                        };
                        list_lock.push(item);
                    }
                    Err(e) => {
                        let msg = format!(
                            "failed to open log dir reader at dir {}: {}",
                            dir.display(),
                            e
                        );
                        error!("{}", msg);
                        continue;
                    }
                }
            }
        }

        let old_len = list_lock.len();
        list_lock.retain(|item| log_dir_set.contains(&item.path));
        let removed = old_len.saturating_sub(list_lock.len());
        if removed > 0 {
            info!("removed {} deleted log dir readers", removed);
        }

        let mut idx_lock = self.next_start_index.lock().unwrap();
        if list_lock.is_empty() {
            *idx_lock = 0;
        } else {
            *idx_lock %= list_lock.len();
        }

        Ok(())
    }

    pub fn get_active_ids(&self) -> Vec<String> {
        let list_lock = self.list.lock().unwrap();
        list_lock.iter().map(|item| item.id.clone()).collect()
    }

    // Scan the log directory for subdirectories containing log files, which contain a meta file name "log_meta.db"
    fn scan_dir(&self, root: &Path) -> Result<Vec<PathBuf>, String> {
        let mut log_dirs = Vec::new();
        let entries = std::fs::read_dir(root).map_err(|e| {
            let msg = format!("failed to read log root dir {}: {}", root.display(), e);
            error!("{}", msg);
            msg
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                let msg = format!("failed to read log root dir entry: {}", e);
                error!("{}", msg);
                msg
            })?;

            // Check if excluded
            let file_name = entry.file_name().to_string_lossy().to_string();
            if self.excluded.iter().any(|ex| ex == &file_name) {
                debug!("log dir {} is excluded, skip it", file_name);
                continue;
            }

            let path = entry.path();
            if path.is_dir() {
                let meta_path = path.join("log_meta.db");
                if meta_path.exists() {
                    // info!("found sub log dir: {}", path.display());
                    log_dirs.push(path);
                }
            }
        }

        Ok(log_dirs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slog::{LogLevel, LogMeta, SystemLogRecordLineFormatter};
    use std::collections::HashSet;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_service_dir(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = dir.join("log_meta.db");
        std::fs::File::create(meta).unwrap();
        dir
    }

    fn new_temp_root(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "buckyos/slog_daemon_reader_test/{}_{}_{}",
            prefix,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn create_service_dir_with_logs(root: &Path, name: &str, count: usize) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();

        let meta = LogMeta::open(&dir).unwrap();
        let file_name = format!("{}.1.log", name);
        meta.append_new_file(&file_name).unwrap();

        let mut content = String::new();
        for i in 0..count {
            let record = slog::SystemLogRecord {
                level: LogLevel::Info,
                target: name.to_string(),
                time: 1721000000000 + i as u64,
                file: Some(format!("{}.rs", name)),
                line: Some(i as u32 + 1),
                content: format!("{}-{}", name, i),
            };
            content.push_str(&SystemLogRecordLineFormatter::format_record(&record));
        }
        std::fs::write(dir.join(&file_name), &content).unwrap();
        meta.update_current_write_index(content.len() as u64)
            .unwrap();

        dir
    }

    #[test]
    fn test_update_dir_removes_deleted_dirs() {
        let base = new_temp_root("remove_deleted");

        let service_a = create_service_dir(&base, "service_a");
        let _service_b = create_service_dir(&base, "service_b");

        let reader = LogDirReader::open(&base, vec![]).unwrap();
        assert!(reader.flush_read_pos("service_a").is_ok());
        assert!(reader.flush_read_pos("service_b").is_ok());

        std::fs::remove_dir_all(&service_a).unwrap();
        reader.update_dir().unwrap();

        assert!(matches!(
            reader.flush_read_pos("service_a"),
            Err(FlushReadPosError::NotFound { .. })
        ));
        assert!(reader.flush_read_pos("service_b").is_ok());

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn test_try_read_records_respects_per_service_quota() {
        let base = new_temp_root("quota");
        create_service_dir_with_logs(&base, "service_a", 100);
        create_service_dir_with_logs(&base, "service_b", 100);
        create_service_dir_with_logs(&base, "service_c", 100);

        let reader = LogDirReader::open(&base, vec![]).unwrap();
        let items = reader
            .try_read_records(READ_RECORD_PER_SERVICE_QUOTA * 3)
            .unwrap();

        assert_eq!(items.len(), 3);
        for item in items {
            assert!(item.records.len() <= READ_RECORD_PER_SERVICE_QUOTA);
            assert!(!item.flush_only);
        }

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn test_try_read_records_rotates_start_service_between_batches() {
        let base = new_temp_root("round_robin");
        create_service_dir_with_logs(&base, "service_a", 30);
        create_service_dir_with_logs(&base, "service_b", 30);
        create_service_dir_with_logs(&base, "service_c", 30);

        let reader = LogDirReader::open(&base, vec![]).unwrap();
        let mut first_service_ids = Vec::new();

        for _ in 0..3 {
            let items = reader
                .try_read_records(READ_RECORD_PER_SERVICE_QUOTA)
                .unwrap();
            assert_eq!(items.len(), 1);
            assert!(!items[0].flush_only);
            first_service_ids.push(items[0].id.clone());
            reader.flush_read_pos(&items[0].id).unwrap();
        }

        let unique: HashSet<String> = first_service_ids.into_iter().collect();
        assert_eq!(unique.len(), 3);

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn test_try_read_records_with_blocked_skips_inflight_service() {
        let base = new_temp_root("blocked_service");
        create_service_dir_with_logs(&base, "service_a", 30);
        create_service_dir_with_logs(&base, "service_b", 30);

        let reader = LogDirReader::open(&base, vec![]).unwrap();
        let blocked = HashSet::from([String::from("service_a")]);

        let items = reader
            .try_read_records_with_blocked(READ_RECORD_PER_SERVICE_QUOTA * 2, &blocked)
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "service_b");
        assert!(!items[0].flush_only);

        let unblocked_items = reader
            .try_read_records(READ_RECORD_PER_SERVICE_QUOTA)
            .unwrap();
        assert_eq!(unblocked_items.len(), 1);
        assert_eq!(unblocked_items[0].id, "service_a");

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn test_try_read_records_emits_flush_only_item_for_invalid_lines() {
        let base = new_temp_root("invalid_line_flush_only");
        let dir = base.join("service_invalid");
        std::fs::create_dir_all(&dir).unwrap();

        let meta = LogMeta::open(&dir).unwrap();
        let file_name = "service_invalid.1.log";
        meta.append_new_file(file_name).unwrap();
        let invalid_line = "invalid-log-line-without-required-format\n";
        std::fs::write(dir.join(file_name), invalid_line).unwrap();
        meta.update_current_write_index(invalid_line.len() as u64)
            .unwrap();

        let reader = LogDirReader::open(&base, vec![]).unwrap();
        let first = reader
            .try_read_records(READ_RECORD_PER_SERVICE_QUOTA)
            .unwrap();
        assert_eq!(first.len(), 1);
        assert!(first[0].records.is_empty());
        assert!(first[0].flush_only);
        assert_eq!(first[0].id, "service_invalid");

        reader.flush_read_pos(&first[0].id).unwrap();

        let second = reader
            .try_read_records(READ_RECORD_PER_SERVICE_QUOTA)
            .unwrap();
        assert!(second.is_empty());

        std::fs::remove_dir_all(&base).unwrap();
    }
}
