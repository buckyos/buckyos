use super::meta::LogMeta;
use crate::system_log::{SystemLogTarget, SystemLogRecord};
use std::{fs::File, path::{PathBuf, Path}};
use std::sync::{Mutex, Arc};
use super::format::SystemLogRecordLineFormatter;
use std::io::Write;

// Cache log records before flushing to file, then we flush log every interval
struct LogRecordCache {
    records: Mutex<Vec<SystemLogRecord>>,
}

impl LogRecordCache {
    pub fn new() -> Self {
        LogRecordCache {
            records: Mutex::new(Vec::new()),
        }
    }

    pub fn add_record(&self, record: SystemLogRecord) {
        let mut records = self.records.lock().unwrap();
        records.push(record);
    }

    pub fn fetch_all(&self) -> Vec<SystemLogRecord> {
        let mut records = self.records.lock().unwrap();
        let fetched = records.clone();
        records.clear();
        fetched
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
    cache: LogRecordCache,

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
            cache,
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
                std::thread::sleep(std::time::Duration::from_millis(this.inner.flush_interval_ms));
                this.flush_to_file();
            }
        });
    }

    fn flush_to_file(&self) {
        let records = self.inner.cache.fetch_all();
        if records.is_empty() {
            return;
        }

        // println!("Flushing {} log records to file...", records.len());
        // First try to log to file
        if let Err(e) = self.log_to_file(&records) {
            error!("failed to flush logs to file: {}", e);
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
            let msg = format!("failed to get metadata of log file {}: {}", log_file.display(), e);
            error!("{}", msg);
            msg
        })?;

        let size = metadata.len();

        let file_info = FileInfo { size, file };

        println!("Opened log file: {}", log_file.display());
        Ok(file_info)
    }

    fn log_to_file(&self, records: &[SystemLogRecord]) -> Result<(), String> {
        // First format all records to lines
        let mut lines  = Vec::with_capacity(records.len());

        for record in records {
            let line = SystemLogRecordLineFormatter::format_record(record);
            lines.push(line);
        }

        // Try to open current log file and check size
        let mut current_file = self.inner.current_file.lock().unwrap();
        if current_file.is_none() {
            let file_info = self.open_current_log_file()?;
            *current_file = Some(file_info);
        } else {
            let file_info = current_file.as_ref().unwrap();
            if file_info.size >= self.inner.max_file_size {
                self.inner.meta.seal_current_write_file().map_err(|e| {
                    let msg = format!("failed to seal current write log file: {}", e);
                    error!("{}", msg);
                    msg
                })?;
                let file_info = self.open_current_log_file()?;
                *current_file = Some(file_info);
            }
        }

        let file_info = current_file.as_mut().unwrap();
        
        // Get current pos of the file
        use std::io::Seek;
        let pos = file_info.file.seek(std::io::SeekFrom::Current(0)).map_err(|e| {
            let msg = format!("failed to seek to end of log file: {}", e);
            error!("{}", msg);
            msg
        })?;

        let mut i = 0;
        let ret = loop {
            if i >= lines.len() {
                break Ok(());
            }

            let line = &lines[i];
            i += 1;
            match  file_info.file.write_all(line.as_bytes()) {
                Ok(_) => {
                    file_info.size += line.len() as u64;
                },
                Err(e) => {
                    let msg = format!("failed to write log to file: {}", e);
                    error!("{}", msg);
                    break Err(msg);
                }
            }
        };

        let new_pos = file_info.file.seek(std::io::SeekFrom::Current(0)).map_err(|e| {
            let msg = format!("failed to seek log file after writing: {}", e);
            error!("{}", msg);
            msg
        })?;

        // If any logs were written, update the write index in meta
        if new_pos > pos {
            self.inner.meta.update_current_write_index(new_pos).map_err(|e| {
                let msg = format!("failed to update write index of current log file: {}", e);
                error!("{}", msg);
                msg
            })?;
        }

        ret
    }
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
