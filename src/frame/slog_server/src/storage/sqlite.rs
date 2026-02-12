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

        // Create index on (source_fk, timestamp DESC) for efficient querying by source and time
        // Create index on (timestamp DESC) for efficient time-based queries
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_logs_source_time 
             ON logs (source_fk, timestamp DESC);
             
             CREATE INDEX IF NOT EXISTS idx_logs_time 
             ON logs (timestamp DESC);",
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
                "INSERT INTO logs (source_fk, timestamp, level, target, file, line, content) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare log insert statement: {}", e);
                error!("{}", msg);
                msg
            })?;

        // Insert or get source_id
        source_stmt
            .execute(rusqlite::params![logs.node, logs.service,])
            .map_err(|e| {
                let msg = format!("Failed to insert log source: {}", e);
                error!("{}", msg);
                msg
            })?;

        let source_id: i64 = source_id_stmt
            .query_row(rusqlite::params![logs.node, logs.service,], |row| {
                row.get(0)
            })
            .map_err(|e| {
                let msg = format!("Failed to get source_id: {}", e);
                error!("{}", msg);
                msg
            })?;

        // Insert log record list
        for record in logs.logs {
            log_insert_stmt
                .execute(rusqlite::params![
                    source_id,
                    record.time as i64,
                    record.level as i32,
                    record.target,
                    record.file,
                    record.line.map(|v| v as i64),
                    record.content,
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
mod test {
    use super::*;
    use slog::{LogLevel, SystemLogRecord};
    use std::fs;

    #[test]
    fn test_sqlite_log_storage() {
        // Get a temporary database path
        let dir = std::env::temp_dir().join("buckyos/slog_test");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test_logs.db");
        if db_path.exists() {
            fs::remove_file(&db_path).unwrap();
        }

        let storage = SqliteLogStorage::open(&db_path).unwrap();

        let records = vec![
            SystemLogRecord {
                level: LogLevel::Info,
                target: "test_target".to_string(),
                time: 1625079600000,
                file: Some("test_file.rs".to_string()),
                line: Some(42),
                content: "This is a test log message.".to_string(),
            },
            SystemLogRecord {
                level: LogLevel::Error,
                target: "test_target".to_string(),
                time: 1625079660000,
                file: None,
                line: None,
                content: "This is another test log message.".to_string(),
            },
        ];

        let log_records = LogRecords {
            node: "test_node".to_string(),
            service: "test_service".to_string(),
            logs: records,
        };

        storage.append(log_records).unwrap();

        // For another node and service
        let records2 = vec![
            SystemLogRecord {
                level: LogLevel::Warn,
                target: "test_target_2".to_string(),
                time: 1625079720000,
                file: Some("test_file_2.rs".to_string()),
                line: Some(84),
                content: "This is a warning log message.".to_string(),
            },
            SystemLogRecord {
                level: LogLevel::Debug,
                target: "test_target_2".to_string(),
                time: 1625079780000,
                file: None,
                line: None,
                content: "This is a debug log message.".to_string(),
            },
        ];

        let log_records2 = LogRecords {
            node: "test_node_2".to_string(),
            service: "test_service_2".to_string(),
            logs: records2,
        };

        storage.append(log_records2).unwrap();

        drop(storage);

        // Clean up
        fs::remove_file(&db_path).unwrap();
    }
}
