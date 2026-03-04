use super::format::SystemLogRecordLineFormatter;
use super::meta::LogMeta;
use crate::system_log::{SystemLogRecord, SystemLogTarget};
use std::collections::VecDeque;
use std::io::Write;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

// Cache log records before flushing to file, then we flush log every interval
struct LogRecordCache {
    records: Mutex<VecDeque<SystemLogRecord>>,
}

impl LogRecordCache {
    pub fn new() -> Self {
        LogRecordCache {
            records: Mutex::new(VecDeque::new()),
        }
    }

    pub fn add_record(&self, record: SystemLogRecord) {
        let mut records = self.records.lock().unwrap();
        records.push_back(record);
    }

    pub fn fetch_all(&self) -> Vec<SystemLogRecord> {
        let mut records = self.records.lock().unwrap();
        std::mem::take(&mut *records).into_iter().collect()
    }

    // Requeue failed flush records to the front, so old records are retried first.
    pub fn add_records_front(&self, failed_records: Vec<SystemLogRecord>) {
        if failed_records.is_empty() {
            return;
        }

        let mut records = self.records.lock().unwrap();
        let mut pending = VecDeque::from(failed_records);
        pending.append(&mut records);
        *records = pending;
    }
}

struct FileInfo {
    size: u64,
    file: File,
}

struct FileLogTargetInner {
    log_dir: PathBuf,
    service_name: String,
    max_file_size: u64,
    flush_interval_ms: u64,

    meta: LogMeta,

    current_file: Mutex<Option<FileInfo>>,
    pending_write_index: Mutex<Option<u64>>,
    cache: LogRecordCache,

    #[cfg(test)]
    force_next_meta_sync_fail: AtomicBool,
    #[cfg(test)]
    force_write_fail_at_record: Mutex<Option<usize>>,
}

#[derive(Clone)]
pub struct FileLogTarget {
    inner: Arc<FileLogTargetInner>,
}

impl FileLogTarget {
    pub fn new(
        log_dir: &Path,
        service_name: String,
        max_file_size: u64,
        flush_interval_ms: u64,
    ) -> Result<Self, String> {
        let meta = LogMeta::open(std::path::Path::new(&log_dir))?;
        let cache = LogRecordCache::new();

        let inner = FileLogTargetInner {
            log_dir: log_dir.to_path_buf(),
            service_name,
            max_file_size,
            flush_interval_ms,
            meta,
            current_file: Mutex::new(None),
            pending_write_index: Mutex::new(None),
            cache,
            #[cfg(test)]
            force_next_meta_sync_fail: AtomicBool::new(false),
            #[cfg(test)]
            force_write_fail_at_record: Mutex::new(None),
        };

        let ret = Self {
            inner: Arc::new(inner),
        };
        ret.start();

        Ok(ret)
    }

    fn start(&self) {
        let this = self.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(
                    this.inner.flush_interval_ms,
                ));
                this.flush_to_file();
            }
        });
    }

    fn flush_to_file(&self) {
        self.try_sync_pending_write_index();

        let records = self.inner.cache.fetch_all();
        if records.is_empty() {
            return;
        }

        let result = self.log_to_file(&records);
        if let Some(e) = result.error {
            error!("failed to flush logs to file: {}", e);
        }

        if result.written_count < records.len() {
            let failed_records = records[result.written_count..].to_vec();
            let count = failed_records.len();
            self.inner.cache.add_records_front(failed_records);
            warn!(
                "requeued {} unwritten log records after flush failure, written_count={}",
                count, result.written_count
            );
        }

        if !result.meta_synced {
            warn!(
                "write index meta sync is pending for service {}",
                self.inner.service_name
            );
        }
    }

    fn try_sync_pending_write_index(&self) {
        let pending = {
            let pending_lock = self.inner.pending_write_index.lock().unwrap();
            *pending_lock
        };

        if let Some(index) = pending {
            if self.sync_write_index_or_mark_pending(index) {
                info!("synced pending write index to meta: {}", index);
            } else {
                warn!("failed to sync pending write index to meta: {}", index);
            }
        }
    }

    fn has_pending_write_index(&self) -> bool {
        let pending_lock = self.inner.pending_write_index.lock().unwrap();
        pending_lock.is_some()
    }

    fn sync_write_index_or_mark_pending(&self, new_index: u64) -> bool {
        #[cfg(test)]
        if self
            .inner
            .force_next_meta_sync_fail
            .swap(false, Ordering::SeqCst)
        {
            let mut pending_lock = self.inner.pending_write_index.lock().unwrap();
            *pending_lock = Some(pending_lock.unwrap_or(0).max(new_index));
            error!(
                "forced meta sync failure for test, pending write index={}",
                new_index
            );
            return false;
        }

        match self.inner.meta.update_current_write_index(new_index) {
            Ok(_) => {
                let mut pending_lock = self.inner.pending_write_index.lock().unwrap();
                *pending_lock = None;
                true
            }
            Err(e) => {
                let mut pending_lock = self.inner.pending_write_index.lock().unwrap();
                *pending_lock = Some(pending_lock.unwrap_or(0).max(new_index));
                error!(
                    "failed to update write index of current log file to {}, mark pending: {}",
                    new_index, e
                );
                false
            }
        }
    }

    fn get_current_log_file(&self) -> Result<PathBuf, String> {
        let write_info = self.inner.meta.get_active_write_file().map_err(|e| {
            let msg = format!("failed to get active write log file: {}", e);
            error!("{}", msg);
            msg
        })?;

        let file_name = if write_info.is_none() {
            let last_file = self.inner.meta.get_last_sealed_file().map_err(|e| {
                let msg = format!("failed to get last sealed log file: {}", e);
                error!("{}", msg);
                msg
            })?;

            let next_id = match last_file {
                Some(f) => f.id + 1,
                None => 1,
            };

            let file_name = format!("{}.{}.log", self.inner.service_name, next_id);
            self.inner.meta.append_new_file(&file_name).map_err(|e| {
                let msg = format!("failed to append new log file meta: {}", e);
                error!("{}", msg);
                msg
            })?;

            info!("created new log file: {}", file_name);
            file_name
        } else {
            let info = write_info.unwrap();
            info!("continue using existing log file: {}", info.name);
            info.name
        };

        let full_path = self.inner.log_dir.join(&file_name);
        Ok(full_path)
    }

    fn open_current_log_file(&self) -> Result<FileInfo, String> {
        let log_file = self.get_current_log_file()?;
        let file = File::options()
            .create(true)
            .append(true)
            .open(&log_file)
            .map_err(|e| {
                let msg = format!("failed to open log file {}: {}", log_file.display(), e);
                error!("{}", msg);
                msg
            })?;

        // Stat the file to get its size
        let metadata = file.metadata().map_err(|e| {
            let msg = format!(
                "failed to get metadata of log file {}: {}",
                log_file.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        let size = metadata.len();
        if size > 0 {
            let _ = self.sync_write_index_or_mark_pending(size);
        }

        let file_info = FileInfo { size, file };

        println!("Opened log file: {}", log_file.display());
        Ok(file_info)
    }

    fn log_to_file(&self, records: &[SystemLogRecord]) -> FlushToFileResult {
        // First format all records to lines
        let mut lines = Vec::with_capacity(records.len());

        for record in records {
            let line = SystemLogRecordLineFormatter::format_record(record);
            lines.push(line);
        }

        // Try to open current log file and check size
        let mut current_file = self.inner.current_file.lock().unwrap();
        if current_file.is_none() {
            let file_info = match self.open_current_log_file() {
                Ok(file) => file,
                Err(e) => {
                    return FlushToFileResult {
                        written_count: 0,
                        meta_synced: true,
                        error: Some(e),
                    };
                }
            };
            *current_file = Some(file_info);
        } else {
            let file_info = current_file.as_ref().unwrap();
            if file_info.size >= self.inner.max_file_size {
                if self.has_pending_write_index() {
                    warn!(
                        "skip sealing current file due pending write index sync, service={}",
                        self.inner.service_name
                    );
                } else {
                    let seal_result = self.inner.meta.seal_current_write_file().map_err(|e| {
                        let msg = format!("failed to seal current write log file: {}", e);
                        error!("{}", msg);
                        msg
                    });
                    if let Err(e) = seal_result {
                        return FlushToFileResult {
                            written_count: 0,
                            meta_synced: true,
                            error: Some(e),
                        };
                    }

                    let file_info = match self.open_current_log_file() {
                        Ok(file) => file,
                        Err(e) => {
                            return FlushToFileResult {
                                written_count: 0,
                                meta_synced: true,
                                error: Some(e),
                            };
                        }
                    };
                    *current_file = Some(file_info);
                }
            }
        }

        let file_info = current_file.as_mut().unwrap();

        // Get current pos of the file
        use std::io::Seek;
        let pos = match file_info
            .file
            .seek(std::io::SeekFrom::Current(0))
            .map_err(|e| {
                let msg = format!("failed to seek to end of log file: {}", e);
                error!("{}", msg);
                msg
            }) {
            Ok(pos) => pos,
            Err(e) => {
                return FlushToFileResult {
                    written_count: 0,
                    meta_synced: true,
                    error: Some(e),
                };
            }
        };

        let mut written_count = 0usize;
        let mut write_error: Option<String> = None;
        for line_index in 0..lines.len() {
            #[cfg(test)]
            if self.should_force_write_fail_at_record(line_index) {
                write_error = Some(format!("forced write failure at record {}", line_index));
                break;
            }

            let line = &lines[line_index];
            match file_info.file.write_all(line.as_bytes()) {
                Ok(_) => {
                    file_info.size += line.len() as u64;
                    written_count += 1;
                }
                Err(e) => {
                    let msg = format!("failed to write log to file: {}", e);
                    error!("{}", msg);
                    write_error = Some(msg);
                    break;
                }
            }
        }

        let new_pos = match file_info.file.seek(std::io::SeekFrom::Current(0)) {
            Ok(pos) => pos,
            Err(e) => {
                let msg = format!("failed to seek log file after writing: {}", e);
                error!("{}", msg);
                file_info.size
            }
        };

        // If any logs were written, update the write index in meta
        let mut meta_synced = true;
        if new_pos > pos {
            meta_synced = self.sync_write_index_or_mark_pending(new_pos);
        }

        FlushToFileResult {
            written_count,
            meta_synced,
            error: write_error,
        }
    }

    #[cfg(test)]
    fn should_force_write_fail_at_record(&self, current_index: usize) -> bool {
        let mut fail_lock = self.inner.force_write_fail_at_record.lock().unwrap();
        if let Some(target_index) = *fail_lock {
            if target_index == current_index {
                *fail_lock = None;
                return true;
            }
        }
        false
    }

    #[cfg(test)]
    fn force_next_meta_sync_fail_for_test(&self) {
        self.inner
            .force_next_meta_sync_fail
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn force_write_fail_at_record_for_test(&self, record_index: usize) {
        let mut fail_lock = self.inner.force_write_fail_at_record.lock().unwrap();
        *fail_lock = Some(record_index);
    }

    #[cfg(test)]
    fn pending_write_index_for_test(&self) -> Option<u64> {
        let pending_lock = self.inner.pending_write_index.lock().unwrap();
        *pending_lock
    }
}

struct FlushToFileResult {
    written_count: usize,
    meta_synced: bool,
    error: Option<String>,
}

impl SystemLogTarget for FileLogTarget {
    fn log(&self, record: &SystemLogRecord) {
        // println!("Caching log record to file log target...");
        self.inner.cache.add_record(record.clone());
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{FileLogTarget, LogRecordCache, SystemLogRecordLineFormatter};
    use crate::system_log::{LogLevel, SystemLogRecord, SystemLogTarget};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_record(content: &str) -> SystemLogRecord {
        SystemLogRecord {
            level: LogLevel::Info,
            target: "test".to_string(),
            time: 1,
            file: None,
            line: None,
            content: content.to_string(),
        }
    }

    fn new_temp_log_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "buckyos/slog_tests/{}_{}_{}",
            prefix,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read_records_from_current_file(
        target: &FileLogTarget,
        log_dir: &Path,
    ) -> Vec<SystemLogRecord> {
        let info = target.inner.meta.get_active_write_file().unwrap().unwrap();
        let content = std::fs::read_to_string(log_dir.join(info.name)).unwrap();
        content
            .lines()
            .map(|line| SystemLogRecordLineFormatter::parse_record(line).unwrap())
            .collect()
    }

    #[test]
    fn test_log_record_cache_fetch_all_keeps_order() {
        let cache = LogRecordCache::new();
        cache.add_record(make_record("a"));
        cache.add_record(make_record("b"));

        let first = cache.fetch_all();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].content, "a");
        assert_eq!(first[1].content, "b");

        let second = cache.fetch_all();
        assert!(second.is_empty());
    }

    #[test]
    fn test_log_record_cache_requeue_front_preserves_old_first() {
        let cache = LogRecordCache::new();
        cache.add_record(make_record("new-1"));
        cache.add_record(make_record("new-2"));

        cache.add_records_front(vec![make_record("old-1"), make_record("old-2")]);

        let all = cache.fetch_all();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].content, "old-1");
        assert_eq!(all[1].content, "old-2");
        assert_eq!(all[2].content, "new-1");
        assert_eq!(all[3].content, "new-2");
    }

    #[test]
    fn test_flush_requeues_only_unwritten_records_when_partial_write_fails() {
        let log_dir = new_temp_log_dir("file_log_partial_write");
        let target = FileLogTarget::new(
            &log_dir,
            "test_service".to_string(),
            1024 * 1024,
            60 * 60 * 1000,
        )
        .unwrap();

        target.log(&make_record("a"));
        target.log(&make_record("b"));
        target.log(&make_record("c"));
        target.force_write_fail_at_record_for_test(1);

        target.flush_to_file();

        let requeued = target.inner.cache.fetch_all();
        assert_eq!(requeued.len(), 2);
        assert_eq!(requeued[0].content, "b");
        assert_eq!(requeued[1].content, "c");

        let written = read_records_from_current_file(&target, &log_dir);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].content, "a");
    }

    #[test]
    fn test_flush_meta_sync_failure_does_not_requeue_written_records() {
        let log_dir = new_temp_log_dir("file_log_meta_pending");
        let target = FileLogTarget::new(
            &log_dir,
            "test_service".to_string(),
            1024 * 1024,
            60 * 60 * 1000,
        )
        .unwrap();

        target.log(&make_record("meta-fail-once"));
        target.force_next_meta_sync_fail_for_test();

        target.flush_to_file();
        assert!(target.inner.cache.fetch_all().is_empty());
        assert!(target.pending_write_index_for_test().is_some());

        let after_first_flush = read_records_from_current_file(&target, &log_dir);
        assert_eq!(after_first_flush.len(), 1);
        assert_eq!(after_first_flush[0].content, "meta-fail-once");

        target.flush_to_file();
        assert!(target.pending_write_index_for_test().is_none());

        let after_second_flush = read_records_from_current_file(&target, &log_dir);
        assert_eq!(after_second_flush.len(), 1);
        assert_eq!(after_second_flush[0].content, "meta-fail-once");
    }
}
