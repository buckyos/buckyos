use slog::{FileLogReader, SystemLogRecord};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// Log records read from a log directory, identified by id as service name(directory name)
#[derive(Clone)]
pub struct LogRecordItem {
    pub records: Vec<SystemLogRecord>,
    pub id: String,
}

struct LogDirItem {
    id: String, // Unique ID for the log directory， could be the directory name
    path: PathBuf,
    reader: FileLogReader,
}

pub struct LogDirReader {
    dir: PathBuf,
    excluded: Vec<String>,
    list: Mutex<Vec<LogDirItem>>,
}

impl LogDirReader {
    pub fn open(log_dir: &Path, excluded: Vec<String>) -> Result<Self, String> {
        info!("opening log dir reader at dir: {}, excluded: {:?}", log_dir.display(), excluded);
        
        let reader = LogDirReader {
            dir: log_dir.to_path_buf(),
            excluded,
            list: Mutex::new(Vec::new()),
        };

        reader.update_dir()?;

        Ok(reader)
    }

    pub fn try_read_records(&self, batch_size: usize) -> Result<Vec<LogRecordItem>, String> {
        let mut result = Vec::new();

        let mut list_lock = self.list.lock().unwrap();
        let mut remain_size = batch_size;
        for item in list_lock.iter_mut() {
            match item.reader.try_read_next_records(remain_size) {
                Ok(records) => {
                    if !records.is_empty() {
                        assert!(remain_size >= records.len());
                        remain_size -= records.len();

                        result.push(LogRecordItem {
                            records,
                            id: item.id.clone(),
                        });

                        if remain_size == 0 {
                            break;
                        }
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

        Ok(result)
    }

    pub fn flush_read_pos(&self, id: &str) -> Result<(), String> {
        let mut list_lock = self.list.lock().unwrap();
        for item in list_lock.iter_mut() {
            if item.id == id {
                return item.reader.flush_read_index();
            }
        }

        let msg = format!("log dir id not found when updating read pos: {}", id);
        error!("{}", msg);
        Err(msg)
    }

    pub fn update_dir(&self) -> Result<(), String> {
        let log_dirs = self.scan_dir(&self.dir)?;

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

        // TODO: remove deleted dirs

        Ok(())
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
