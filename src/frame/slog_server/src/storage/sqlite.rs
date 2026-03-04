use super::storage::{LogQueryRequest, LogRecords, LogStorage};
use rusqlite::Connection;
use slog::SystemLogRecord;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

struct SystemLogRecordResult {
    pub level: u32,
    pub target: String,
    pub time: u64,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub content: String,
}

impl TryInto<SystemLogRecord> for SystemLogRecordResult {
    type Error = String;

    fn try_into(self) -> Result<SystemLogRecord, Self::Error> {
        Ok(SystemLogRecord {
            level: slog::LogLevel::try_from(self.level)?,
            target: self.target,
            time: self.time,
            file: self.file,
            line: self.line,
            content: self.content,
        })
    }
}
pub struct SqliteLogStorage {
    db_path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl SqliteLogStorage {
    fn ensure_logs_column(conn: &Connection, col_def: &str) -> Result<(), String> {
        let sql = format!("ALTER TABLE logs ADD COLUMN {}", col_def);
        match conn.execute_batch(&sql) {
            Ok(_) => Ok(()),
            Err(e) => {
                let err_text = e.to_string();
                if err_text.contains("duplicate column name") {
                    Ok(())
                } else {
                    let msg = format!("Failed to alter logs table with '{}': {}", col_def, e);
                    error!("{}", msg);
                    Err(msg)
                }
            }
        }
    }

    pub fn open(db_path: &Path) -> Result<Self, String> {
        // First initialize the database
        let conn = Connection::open(db_path).map_err(|e| {
            let msg = format!("Failed to open database at {:?}: {}", db_path, e);
            error!("{}", msg);
            msg
        })?;

        // Then set journal mode to WAL, and improve concurrency for reads and writes
        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(|e| {
                let msg = format!("Failed to set WAL mode: {}", e);
                error!("{}", msg);
                msg
            })?;

        // Create log_sources table (normalized)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS log_sources (
                source_id     INTEGER PRIMARY KEY, 
                node_id       TEXT NOT NULL,
                service_name  TEXT NOT NULL,
                UNIQUE(node_id, service_name)
            );",
        )
        .map_err(|e| {
            let msg = format!("Failed to create log_sources table: {}", e);
            error!("{}", msg);
            msg
        })?;

        // Create the main logs table
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS logs (
                log_id      INTEGER PRIMARY KEY,
                source_fk   INTEGER NOT NULL,
                timestamp   INTEGER NOT NULL,
                level       INTEGER NOT NULL,
                target      TEXT NOT NULL,
                file        TEXT,
                line        INTEGER,
                content     TEXT NOT NULL,
                FOREIGN KEY(source_fk) REFERENCES log_sources(source_id)
            );",
        )
        .map_err(|e| {
            let msg = format!("Failed to create logs table: {}", e);
            error!("{}", msg);
            msg
        })?;

        // Backward-compatible schema extension for idempotent append.
        Self::ensure_logs_column(&conn, "batch_id TEXT")?;
        Self::ensure_logs_column(&conn, "record_index INTEGER")?;
        Self::ensure_logs_column(&conn, "record_id TEXT")?;

        // Create index on (source_fk, timestamp DESC) for efficient querying by source and time
        // Create index on (timestamp DESC) for efficient time-based queries
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_logs_source_time 
             ON logs (source_fk, timestamp DESC);
             
             CREATE INDEX IF NOT EXISTS idx_logs_time 
             ON logs (timestamp DESC);

             CREATE UNIQUE INDEX IF NOT EXISTS idx_logs_source_batch_record
             ON logs (source_fk, batch_id, record_index)
             WHERE batch_id IS NOT NULL;

             CREATE UNIQUE INDEX IF NOT EXISTS idx_logs_source_record_id
             ON logs (source_fk, record_id)
             WHERE record_id IS NOT NULL;",
        )
        .map_err(|e| {
            let msg = format!("Failed to create indexes on logs table: {}", e);
            error!("{}", msg);
            msg
        })?;

        info!("Initialized SQLite log storage at {:?}", db_path);

        let ret = Self {
            db_path: db_path.to_path_buf(),
            conn: Arc::new(Mutex::new(conn)),
        };

        Ok(ret)
    }

    fn append(&self, logs: LogRecords) -> Result<(), String> {
        let LogRecords {
            node,
            service,
            batch_id,
            record_ids,
            logs,
        } = logs;
        let mut conn_lock = self.conn.lock().unwrap();

        // Do all operations in a transaction!
        let tx = conn_lock.transaction().map_err(|e| {
            let msg = format!("Failed to start transaction: {}", e);
            error!("{}", msg);
            msg
        })?;

        // Prepare statements
        let mut source_stmt = tx
            .prepare(
                "INSERT OR IGNORE INTO log_sources (node_id, service_name) 
             VALUES (?1, ?2);",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare source insert statement: {}", e);
                error!("{}", msg);
                msg
            })?;

        let mut source_id_stmt = tx
            .prepare(
                "SELECT source_id FROM log_sources 
             WHERE node_id = ?1 AND service_name = ?2;",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare source select statement: {}", e);
                error!("{}", msg);
                msg
            })?;

        let mut log_insert_stmt = tx
            .prepare(
                "INSERT OR IGNORE INTO logs (
                    source_fk, timestamp, level, target, file, line, content, batch_id, record_index, record_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10);",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare log insert statement: {}", e);
                error!("{}", msg);
                msg
            })?;

        // Insert or get source_id
        source_stmt
            .execute(rusqlite::params![&node, &service,])
            .map_err(|e| {
                let msg = format!("Failed to insert log source: {}", e);
                error!("{}", msg);
                msg
            })?;

        let source_id: i64 = source_id_stmt
            .query_row(rusqlite::params![&node, &service,], |row| row.get(0))
            .map_err(|e| {
                let msg = format!("Failed to get source_id: {}", e);
                error!("{}", msg);
                msg
            })?;

        // Insert log record list
        for (record_index, record) in logs.into_iter().enumerate() {
            let record_id = record_ids.get(record_index).map(|s| s.as_str());
            log_insert_stmt
                .execute(rusqlite::params![
                    source_id,
                    record.time as i64,
                    record.level as i32,
                    record.target,
                    record.file,
                    record.line.map(|v| v as i64),
                    record.content,
                    batch_id.as_deref(),
                    record_index as i64,
                    record_id,
                ])
                .map_err(|e| {
                    let msg = format!("Failed to insert log record: {}", e);
                    error!("{}", msg);
                    msg
                })?;
        }

        drop(log_insert_stmt);
        drop(source_id_stmt);
        drop(source_stmt);

        // Commit transaction
        tx.commit().map_err(|e| {
            let msg = format!("Failed to commit transaction: {}", e);
            error!("{}", msg);
            msg
        })?;

        Ok(())
    }

    fn query(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
        let conn_lock = self.conn.lock().unwrap();

        // Build the query dynamically based on the request parameters
        let mut query = String::from(
            "SELECT ls.node_id, ls.service_name, l.timestamp, l.level, l.target, l.file, l.line, l.content
             FROM logs l
             JOIN log_sources ls ON l.source_fk = ls.source_id
             WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(node) = request.node {
            query.push_str(" AND ls.node_id = ? ");
            params.push(Box::new(node));
        }
        if let Some(service) = request.service {
            query.push_str(" AND ls.service_name = ? ");
            params.push(Box::new(service));
        }
        if let Some(level) = request.level {
            query.push_str(" AND l.level = ? ");
            params.push(Box::new(level as i32));
        }
        if let Some(start_time) = request.start_time {
            query.push_str(" AND l.timestamp >= ? ");
            params.push(Box::new(start_time as i64));
        }
        if let Some(end_time) = request.end_time {
            query.push_str(" AND l.timestamp <= ? ");
            params.push(Box::new(end_time as i64));
        }

        query.push_str(" ORDER BY l.timestamp DESC ");

        if let Some(limit) = request.limit {
            query.push_str(" LIMIT ? ");
            params.push(Box::new(limit as i64));
        }

        let mut stmt = conn_lock.prepare(&query).map_err(|e| {
            let msg = format!("Failed to prepare log query statement: {}", e);
            error!("{}", msg);
            msg
        })?;

        let log_iter = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter().map(|p| &**p)),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        SystemLogRecordResult {
                            time: row.get::<_, i64>(2)? as u64,
                            level: row.get::<_, i32>(3)? as u32,
                            target: row.get::<_, String>(4)?,
                            file: row.get::<_, Option<String>>(5)?,
                            line: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
                            content: row.get::<_, String>(7)?,
                        },
                    ))
                },
            )
            .map_err(|e| {
                let msg = format!("Failed to execute log query: {}", e);
                error!("{}", msg);
                msg
            })?;
        let mut records_map: std::collections::HashMap<(String, String), Vec<SystemLogRecord>> =
            std::collections::HashMap::new();
        for log_result in log_iter {
            let (node, service, record) = log_result.map_err(|e| {
                let msg = format!("Failed to map log row: {}", e);
                error!("{}", msg);
                msg
            })?;
            let key = (node.clone(), service.clone());
            match record.try_into() {
                Ok(rec) => {
                    records_map.entry(key).or_insert_with(Vec::new).push(rec);
                }
                Err(e) => {
                    let msg = format!("Failed to convert log record: {}", e);
                    warn!("{}", msg);
                    continue;
                }
            }
        }

        let mut result = Vec::new();
        for ((node, service), logs) in records_map {
            result.push(LogRecords {
                node,
                service,
                batch_id: None,
                record_ids: vec![],
                logs,
            });
        }

        Ok(result)
    }
}

#[async_trait::async_trait]
impl LogStorage for SqliteLogStorage {
    async fn append_logs(&self, logs: LogRecords) -> Result<(), String> {
        self.append(logs)
    }

    async fn query_logs(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
        self.query(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slog::{LogLevel, SystemLogRecord};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "buckyos/slog_server_tests/{}_{}_{}",
            prefix,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("test_logs.db")
    }

    fn sample_record(level: LogLevel, time: u64, content: &str) -> SystemLogRecord {
        SystemLogRecord {
            level,
            target: "test_target".to_string(),
            time,
            file: None,
            line: None,
            content: content.to_string(),
        }
    }

    #[test]
    fn test_sqlite_storage_append_and_query_with_filters() {
        let db_path = temp_db_path("append_query");
        let storage = SqliteLogStorage::open(&db_path).unwrap();

        storage
            .append(LogRecords {
                node: "node-1".to_string(),
                service: "svc-a".to_string(),
                batch_id: Some("batch-a-1".to_string()),
                record_ids: vec!["rid-a-1".to_string(), "rid-a-2".to_string()],
                logs: vec![
                    sample_record(LogLevel::Info, 1000, "a-1"),
                    sample_record(LogLevel::Error, 1010, "a-2"),
                ],
            })
            .unwrap();

        storage
            .append(LogRecords {
                node: "node-2".to_string(),
                service: "svc-b".to_string(),
                batch_id: Some("batch-b-1".to_string()),
                record_ids: vec!["rid-b-1".to_string()],
                logs: vec![sample_record(LogLevel::Warn, 1020, "b-1")],
            })
            .unwrap();

        let all = storage
            .query(LogQueryRequest {
                node: None,
                service: None,
                level: None,
                start_time: None,
                end_time: None,
                limit: None,
            })
            .unwrap();
        assert_eq!(all.len(), 2);

        let only_error = storage
            .query(LogQueryRequest {
                node: Some("node-1".to_string()),
                service: Some("svc-a".to_string()),
                level: Some(LogLevel::Error),
                start_time: None,
                end_time: None,
                limit: Some(10),
            })
            .unwrap();
        assert_eq!(only_error.len(), 1);
        assert_eq!(only_error[0].node, "node-1");
        assert_eq!(only_error[0].service, "svc-a");
        assert_eq!(only_error[0].logs.len(), 1);
        assert_eq!(only_error[0].logs[0].content, "a-2");
        assert_eq!(only_error[0].logs[0].level, LogLevel::Error);

        std::fs::remove_file(&db_path).unwrap();
        std::fs::remove_dir_all(db_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn test_sqlite_storage_append_is_idempotent_for_same_batch_id() {
        let db_path = temp_db_path("idempotent_batch");
        let storage = SqliteLogStorage::open(&db_path).unwrap();

        let payload = LogRecords {
            node: "node-1".to_string(),
            service: "svc-a".to_string(),
            batch_id: Some("batch-dup-1".to_string()),
            record_ids: vec!["rid-dup-1".to_string(), "rid-dup-2".to_string()],
            logs: vec![
                sample_record(LogLevel::Info, 1000, "dup-1"),
                sample_record(LogLevel::Info, 1010, "dup-2"),
            ],
        };

        storage.append(payload.clone()).unwrap();
        storage.append(payload).unwrap();

        let queried = storage
            .query(LogQueryRequest {
                node: Some("node-1".to_string()),
                service: Some("svc-a".to_string()),
                level: None,
                start_time: None,
                end_time: None,
                limit: None,
            })
            .unwrap();
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].logs.len(), 2);

        std::fs::remove_file(&db_path).unwrap();
        std::fs::remove_dir_all(db_path.parent().unwrap()).unwrap();
    }
}
