use super::storage::{LogQueryRequest, LogRecords, LogStorage};
use rusqlite::{Connection, params};
use slog::SystemLogRecord;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const DAY_MILLIS: u64 = 24 * 60 * 60 * 1000;
const SECOND_THRESHOLD: u64 = 10_000_000_000;
const DEFAULT_PARTITION_MAX_ROWS: u64 = 5_000_000;
const DEFAULT_PARTITION_MAX_SIZE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartitionBucket {
    Day,
}

impl PartitionBucket {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "day" | "daily" => Some(Self::Day),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Day => "day",
        }
    }

    fn bucket_key(&self, raw_timestamp: u64) -> String {
        match self {
            Self::Day => {
                let epoch_day = normalize_to_millis(raw_timestamp) / DAY_MILLIS;
                format!("day-{}", epoch_day)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlitePartitionedConfig {
    pub bucket: PartitionBucket,
    pub max_rows_per_partition: u64,
    pub max_partition_size_bytes: u64,
}

impl Default for SqlitePartitionedConfig {
    fn default() -> Self {
        Self {
            bucket: PartitionBucket::Day,
            max_rows_per_partition: DEFAULT_PARTITION_MAX_ROWS,
            max_partition_size_bytes: DEFAULT_PARTITION_MAX_SIZE_BYTES,
        }
    }
}

#[derive(Clone, Debug)]
struct PartitionMeta {
    bucket_key: String,
    part_seq: i64,
    file_name: String,
    row_count: u64,
    size_bytes: u64,
}

#[derive(Debug, Clone)]
struct PartitionStats {
    start_time: u64,
    end_time: u64,
    row_count: u64,
    size_bytes: u64,
}

#[derive(Clone)]
struct IndexedRecord {
    record_index: usize,
    record_id: Option<String>,
    record: SystemLogRecord,
}

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

pub struct SqlitePartitionedLogStorage {
    partitions_dir: PathBuf,
    manifest_conn: Arc<Mutex<Connection>>,
    config: SqlitePartitionedConfig,
}

impl SqlitePartitionedLogStorage {
    pub fn open(storage_dir: &Path, config: SqlitePartitionedConfig) -> Result<Self, String> {
        std::fs::create_dir_all(storage_dir).map_err(|e| {
            let msg = format!(
                "Failed to create sqlite partitioned storage dir {}: {}",
                storage_dir.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        let partitions_dir = storage_dir.join("partitions");
        std::fs::create_dir_all(&partitions_dir).map_err(|e| {
            let msg = format!(
                "Failed to create sqlite partitioned data dir {}: {}",
                partitions_dir.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        let manifest_path = storage_dir.join("manifest.db");
        let manifest_conn = Connection::open(&manifest_path).map_err(|e| {
            let msg = format!(
                "Failed to open sqlite partitioned manifest {}: {}",
                manifest_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        manifest_conn
            .execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(|e| {
                let msg = format!("Failed to set WAL for partition manifest db: {}", e);
                error!("{}", msg);
                msg
            })?;

        manifest_conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS partitions (
                    partition_id INTEGER PRIMARY KEY,
                    bucket_key   TEXT NOT NULL,
                    part_seq     INTEGER NOT NULL,
                    file_name    TEXT NOT NULL,
                    start_time   INTEGER NOT NULL,
                    end_time     INTEGER NOT NULL,
                    row_count    INTEGER NOT NULL DEFAULT 0,
                    size_bytes   INTEGER NOT NULL DEFAULT 0,
                    created_at   INTEGER NOT NULL,
                    updated_at   INTEGER NOT NULL,
                    UNIQUE(bucket_key, part_seq),
                    UNIQUE(file_name)
                );

                CREATE INDEX IF NOT EXISTS idx_partitions_time
                ON partitions (start_time, end_time);

                CREATE TABLE IF NOT EXISTS batch_partition_map (
                    node_id      TEXT NOT NULL,
                    service_name TEXT NOT NULL,
                    batch_id     TEXT NOT NULL,
                    bucket_key   TEXT NOT NULL,
                    file_name    TEXT NOT NULL,
                    created_at   INTEGER NOT NULL,
                    PRIMARY KEY (node_id, service_name, batch_id, bucket_key)
                );",
            )
            .map_err(|e| {
                let msg = format!("Failed to initialize partition manifest schema: {}", e);
                error!("{}", msg);
                msg
            })?;

        Self::reconcile_manifest_with_partitions(&manifest_conn, &partitions_dir)?;

        info!(
            "Initialized sqlite partitioned storage at {}, bucket={}, max_rows_per_partition={}, max_partition_size_bytes={}",
            storage_dir.display(),
            config.bucket.as_str(),
            config.max_rows_per_partition,
            config.max_partition_size_bytes
        );

        Ok(Self {
            partitions_dir,
            manifest_conn: Arc::new(Mutex::new(manifest_conn)),
            config,
        })
    }

    fn ensure_partition_schema(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS logs (
                log_id        INTEGER PRIMARY KEY,
                node_id       TEXT NOT NULL,
                service_name  TEXT NOT NULL,
                timestamp     INTEGER NOT NULL,
                level         INTEGER NOT NULL,
                target        TEXT NOT NULL,
                file          TEXT,
                line          INTEGER,
                content       TEXT NOT NULL,
                batch_id      TEXT,
                record_index  INTEGER,
                record_id     TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_logs_node_service_time
            ON logs (node_id, service_name, timestamp DESC);

            CREATE INDEX IF NOT EXISTS idx_logs_time
            ON logs (timestamp DESC);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_logs_batch_record
            ON logs (node_id, service_name, batch_id, record_index)
            WHERE batch_id IS NOT NULL;

            CREATE UNIQUE INDEX IF NOT EXISTS idx_logs_record_id
            ON logs (node_id, service_name, record_id)
            WHERE record_id IS NOT NULL;",
        )
        .map_err(|e| {
            let msg = format!("Failed to initialize partition logs schema: {}", e);
            error!("{}", msg);
            msg
        })?;
        Ok(())
    }

    fn ensure_partition_database(partition_path: &Path) -> Result<(), String> {
        let conn = Connection::open(partition_path).map_err(|e| {
            let msg = format!(
                "Failed to open partition database {}: {}",
                partition_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(|e| {
                let msg = format!(
                    "Failed to set WAL for partition database {}: {}",
                    partition_path.display(),
                    e
                );
                error!("{}", msg);
                msg
            })?;
        Self::ensure_partition_schema(&conn)?;
        Ok(())
    }

    fn parse_partition_file_name(file_name: &str) -> Option<(String, i64)> {
        if !file_name.starts_with("logs_") || !file_name.ends_with(".db") {
            return None;
        }

        let core = &file_name["logs_".len()..file_name.len().saturating_sub(".db".len())];
        let split_idx = core.rfind("_p")?;
        let bucket_key = core[..split_idx].to_string();
        if bucket_key.is_empty() {
            return None;
        }

        let part_seq = core[split_idx + 2..].parse::<i64>().ok()?;
        Some((bucket_key, part_seq))
    }

    fn collect_partition_stats(partition_path: &Path) -> Result<PartitionStats, String> {
        Self::ensure_partition_database(partition_path)?;

        let conn = Connection::open(partition_path).map_err(|e| {
            let msg = format!(
                "Failed to open partition database {} for stats collection: {}",
                partition_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        let (min_ts, max_ts, row_count): (Option<i64>, Option<i64>, i64) = conn
            .query_row(
                "SELECT MIN(timestamp), MAX(timestamp), COUNT(*) FROM logs",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| {
                let msg = format!(
                    "Failed to query partition stats from {}: {}",
                    partition_path.display(),
                    e
                );
                error!("{}", msg);
                msg
            })?;

        let row_count = row_count.max(0) as u64;
        let (start_time, end_time) = if row_count == 0 {
            (0, 0)
        } else {
            (
                min_ts.unwrap_or(0).max(0) as u64,
                max_ts.unwrap_or(0).max(0) as u64,
            )
        };
        let size_bytes = std::fs::metadata(partition_path)
            .map(|m| m.len())
            .unwrap_or_default();

        Ok(PartitionStats {
            start_time,
            end_time,
            row_count,
            size_bytes,
        })
    }

    fn update_manifest_partition_stats(
        manifest: &Connection,
        file_name: &str,
        stats: &PartitionStats,
    ) -> Result<(), String> {
        let updated = manifest
            .execute(
                "UPDATE partitions
                 SET start_time = ?1,
                     end_time = ?2,
                     row_count = ?3,
                     size_bytes = ?4,
                     updated_at = ?5
                 WHERE file_name = ?6",
                params![
                    stats.start_time as i64,
                    stats.end_time as i64,
                    stats.row_count as i64,
                    stats.size_bytes as i64,
                    now_unix_secs(),
                    file_name
                ],
            )
            .map_err(|e| {
                let msg = format!(
                    "Failed to update manifest partition stats for {}: {}",
                    file_name, e
                );
                error!("{}", msg);
                msg
            })?;

        if updated != 1 {
            let msg = format!(
                "Unexpected manifest partition stats update count={}, file_name={}",
                updated, file_name
            );
            error!("{}", msg);
            return Err(msg);
        }

        Ok(())
    }

    fn reconcile_manifest_with_partitions(
        manifest: &Connection,
        partitions_dir: &Path,
    ) -> Result<(), String> {
        let mut manifest_partitions: HashMap<String, (String, i64)> = HashMap::new();
        let mut stmt = manifest
            .prepare("SELECT file_name, bucket_key, part_seq FROM partitions")
            .map_err(|e| {
                let msg = format!("Failed to prepare manifest partition scan query: {}", e);
                error!("{}", msg);
                msg
            })?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| {
                let msg = format!("Failed to scan manifest partitions: {}", e);
                error!("{}", msg);
                msg
            })?;

        for row in rows {
            let (file_name, bucket_key, part_seq) = row.map_err(|e| {
                let msg = format!("Failed to map manifest partition row: {}", e);
                error!("{}", msg);
                msg
            })?;
            manifest_partitions.insert(file_name, (bucket_key, part_seq));
        }
        drop(stmt);

        let dir_entries = std::fs::read_dir(partitions_dir).map_err(|e| {
            let msg = format!(
                "Failed to scan partition directory {}: {}",
                partitions_dir.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;
        let mut partition_files = Vec::new();
        for entry in dir_entries {
            let entry = entry.map_err(|e| {
                let msg = format!("Failed to read partition directory entry: {}", e);
                error!("{}", msg);
                msg
            })?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|v| v.to_str()) != Some("db") {
                continue;
            }
            if let Some(file_name) = path.file_name().and_then(|v| v.to_str()) {
                partition_files.push(file_name.to_string());
            }
        }

        let partition_file_set: HashSet<String> = partition_files.iter().cloned().collect();
        let mut removed_missing = 0usize;
        let manifest_file_names: Vec<String> = manifest_partitions.keys().cloned().collect();
        for file_name in manifest_file_names {
            if partition_file_set.contains(&file_name) {
                continue;
            }

            warn!(
                "partition file missing on disk, remove stale manifest entry: {}",
                file_name
            );
            manifest
                .execute(
                    "DELETE FROM partitions WHERE file_name = ?1",
                    params![file_name.as_str()],
                )
                .map_err(|e| {
                    let msg = format!(
                        "Failed to delete stale partition manifest entry {}: {}",
                        file_name, e
                    );
                    error!("{}", msg);
                    msg
                })?;
            manifest
                .execute(
                    "DELETE FROM batch_partition_map WHERE file_name = ?1",
                    params![file_name.as_str()],
                )
                .map_err(|e| {
                    let msg = format!(
                        "Failed to delete stale batch mapping for {}: {}",
                        file_name, e
                    );
                    error!("{}", msg);
                    msg
                })?;
            manifest_partitions.remove(&file_name);
            removed_missing += 1;
        }

        let mut recovered_new = 0usize;
        let mut refreshed_stats = 0usize;
        for file_name in partition_files {
            let partition_path = partitions_dir.join(&file_name);
            let stats = Self::collect_partition_stats(&partition_path)?;

            if manifest_partitions.contains_key(&file_name) {
                Self::update_manifest_partition_stats(manifest, &file_name, &stats)?;
                refreshed_stats += 1;
                continue;
            }

            let Some((bucket_key, part_seq)) = Self::parse_partition_file_name(&file_name) else {
                warn!(
                    "skip unmanaged partition file (invalid name pattern): {}",
                    file_name
                );
                continue;
            };

            manifest
                .execute(
                    "INSERT OR IGNORE INTO partitions (
                        bucket_key, part_seq, file_name, start_time, end_time, row_count, size_bytes, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        bucket_key,
                        part_seq,
                        &file_name,
                        stats.start_time as i64,
                        stats.end_time as i64,
                        stats.row_count as i64,
                        stats.size_bytes as i64,
                        now_unix_secs()
                    ],
                )
                .map_err(|e| {
                    let msg = format!(
                        "Failed to recover manifest entry for partition file {}: {}",
                        file_name, e
                    );
                    error!("{}", msg);
                    msg
                })?;

            Self::update_manifest_partition_stats(manifest, &file_name, &stats)?;
            recovered_new += 1;
        }

        if removed_missing > 0 || recovered_new > 0 {
            warn!(
                "reconciled partition manifest: removed_missing={}, recovered_new={}, refreshed_stats={}",
                removed_missing, recovered_new, refreshed_stats
            );
        } else {
            info!(
                "partition manifest reconciliation completed, refreshed_stats={}",
                refreshed_stats
            );
        }

        Ok(())
    }

    fn partition_path(&self, file_name: &str) -> PathBuf {
        self.partitions_dir.join(file_name)
    }

    fn should_rollover(
        &self,
        current: &PartitionMeta,
        incoming_rows: u64,
        incoming_bytes: u64,
    ) -> bool {
        let rows_over_limit = self.config.max_rows_per_partition > 0
            && current.row_count.saturating_add(incoming_rows) > self.config.max_rows_per_partition;

        let size_over_limit = self.config.max_partition_size_bytes > 0
            && current.size_bytes > 0
            && current.size_bytes.saturating_add(incoming_bytes)
                > self.config.max_partition_size_bytes;

        rows_over_limit || size_over_limit
    }

    fn estimate_records_bytes(records: &[IndexedRecord]) -> u64 {
        records.iter().fold(0_u64, |acc, item| {
            let record = &item.record;
            let content_len = record.content.len() as u64;
            let target_len = record.target.len() as u64;
            let file_len = record.file.as_ref().map(|s| s.len() as u64).unwrap_or(0);
            acc.saturating_add(content_len + target_len + file_len + 96)
        })
    }

    fn get_latest_partition(
        manifest: &Connection,
        bucket_key: &str,
    ) -> Result<Option<PartitionMeta>, String> {
        let mut stmt = manifest
            .prepare(
                "SELECT bucket_key, part_seq, file_name, row_count, size_bytes
                 FROM partitions
                 WHERE bucket_key = ?1
                 ORDER BY part_seq DESC
                 LIMIT 1",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare latest partition query: {}", e);
                error!("{}", msg);
                msg
            })?;

        match stmt.query_row(params![bucket_key], |row| {
            Ok(PartitionMeta {
                bucket_key: row.get::<_, String>(0)?,
                part_seq: row.get::<_, i64>(1)?,
                file_name: row.get::<_, String>(2)?,
                row_count: row.get::<_, i64>(3)? as u64,
                size_bytes: row.get::<_, i64>(4)? as u64,
            })
        }) {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => {
                let msg = format!("Failed to query latest partition metadata: {}", e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }

    fn get_partition_by_file(
        manifest: &Connection,
        bucket_key: &str,
        file_name: &str,
    ) -> Result<Option<PartitionMeta>, String> {
        let mut stmt = manifest
            .prepare(
                "SELECT bucket_key, part_seq, file_name, row_count, size_bytes
                 FROM partitions
                 WHERE bucket_key = ?1 AND file_name = ?2
                 LIMIT 1",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare partition-by-file query: {}", e);
                error!("{}", msg);
                msg
            })?;

        match stmt.query_row(params![bucket_key, file_name], |row| {
            Ok(PartitionMeta {
                bucket_key: row.get::<_, String>(0)?,
                part_seq: row.get::<_, i64>(1)?,
                file_name: row.get::<_, String>(2)?,
                row_count: row.get::<_, i64>(3)? as u64,
                size_bytes: row.get::<_, i64>(4)? as u64,
            })
        }) {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => {
                let msg = format!("Failed to query partition metadata by file: {}", e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }

    fn create_partition(
        &self,
        manifest: &Connection,
        bucket_key: &str,
        part_seq: i64,
        start_time: u64,
        end_time: u64,
    ) -> Result<PartitionMeta, String> {
        let now = now_unix_secs();
        let file_name = format!("logs_{}_p{}.db", bucket_key, part_seq);
        let partition_path = self.partition_path(&file_name);
        Self::ensure_partition_database(&partition_path)?;
        let size_bytes = std::fs::metadata(&partition_path)
            .map(|m| m.len())
            .unwrap_or_default();

        manifest
            .execute(
                "INSERT INTO partitions (
                    bucket_key, part_seq, file_name, start_time, end_time, row_count, size_bytes, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?7)",
                params![
                    bucket_key,
                    part_seq,
                    &file_name,
                    start_time as i64,
                    end_time as i64,
                    size_bytes as i64,
                    now,
                ],
            )
            .map_err(|e| {
                let msg = format!("Failed to insert partition metadata: {}", e);
                error!("{}", msg);
                msg
            })?;

        Ok(PartitionMeta {
            bucket_key: bucket_key.to_string(),
            part_seq,
            file_name,
            row_count: 0,
            size_bytes,
        })
    }

    fn read_batch_partition_mapping(
        manifest: &Connection,
        node: &str,
        service: &str,
        batch_id: &str,
        bucket_key: &str,
    ) -> Result<Option<String>, String> {
        let mut stmt = manifest
            .prepare(
                "SELECT file_name
                 FROM batch_partition_map
                 WHERE node_id = ?1 AND service_name = ?2 AND batch_id = ?3 AND bucket_key = ?4
                 LIMIT 1",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare batch mapping query: {}", e);
                error!("{}", msg);
                msg
            })?;

        match stmt.query_row(params![node, service, batch_id, bucket_key], |row| {
            row.get(0)
        }) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => {
                let msg = format!("Failed to query batch partition mapping: {}", e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }

    fn write_batch_partition_mapping(
        manifest: &Connection,
        node: &str,
        service: &str,
        batch_id: &str,
        bucket_key: &str,
        file_name: &str,
    ) -> Result<(), String> {
        manifest
            .execute(
                "INSERT OR IGNORE INTO batch_partition_map (
                    node_id, service_name, batch_id, bucket_key, file_name, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    node,
                    service,
                    batch_id,
                    bucket_key,
                    file_name,
                    now_unix_secs()
                ],
            )
            .map_err(|e| {
                let msg = format!("Failed to write batch partition mapping: {}", e);
                error!("{}", msg);
                msg
            })?;
        Ok(())
    }

    fn clear_batch_partition_mapping(
        manifest: &Connection,
        node: &str,
        service: &str,
        batch_id: &str,
        bucket_key: &str,
    ) -> Result<(), String> {
        manifest
            .execute(
                "DELETE FROM batch_partition_map
                 WHERE node_id = ?1 AND service_name = ?2 AND batch_id = ?3 AND bucket_key = ?4",
                params![node, service, batch_id, bucket_key],
            )
            .map_err(|e| {
                let msg = format!("Failed to delete stale batch partition mapping: {}", e);
                error!("{}", msg);
                msg
            })?;
        Ok(())
    }

    fn resolve_target_partition(
        &self,
        manifest: &Connection,
        node: &str,
        service: &str,
        batch_id: Option<&str>,
        bucket_key: &str,
        min_time: u64,
        max_time: u64,
        incoming_rows: u64,
        incoming_bytes: u64,
    ) -> Result<PartitionMeta, String> {
        if let Some(batch_id) = batch_id {
            if let Some(mapped_file) =
                Self::read_batch_partition_mapping(manifest, node, service, batch_id, bucket_key)?
            {
                if let Some(partition) =
                    Self::get_partition_by_file(manifest, bucket_key, &mapped_file)?
                {
                    return Ok(partition);
                }

                warn!(
                    "Stale batch partition mapping found for node={}, service={}, batch_id={}, bucket_key={}, file_name={}; will remap",
                    node, service, batch_id, bucket_key, mapped_file
                );
                Self::clear_batch_partition_mapping(manifest, node, service, batch_id, bucket_key)?;
            }
        }

        let target = match Self::get_latest_partition(manifest, bucket_key)? {
            Some(current) => {
                if self.should_rollover(&current, incoming_rows, incoming_bytes) {
                    self.create_partition(
                        manifest,
                        bucket_key,
                        current.part_seq + 1,
                        min_time,
                        max_time,
                    )?
                } else {
                    current
                }
            }
            None => self.create_partition(manifest, bucket_key, 0, min_time, max_time)?,
        };

        if let Some(batch_id) = batch_id {
            Self::write_batch_partition_mapping(
                manifest,
                node,
                service,
                batch_id,
                bucket_key,
                &target.file_name,
            )?;
        }

        Ok(target)
    }

    fn append_records_to_partition(
        partition_path: &Path,
        node: &str,
        service: &str,
        batch_id: Option<&str>,
        records: &[IndexedRecord],
    ) -> Result<u64, String> {
        Self::ensure_partition_database(partition_path)?;
        let mut conn = Connection::open(partition_path).map_err(|e| {
            let msg = format!(
                "Failed to open partition database {} for append: {}",
                partition_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        let tx = conn.transaction().map_err(|e| {
            let msg = format!(
                "Failed to start partition append transaction {}: {}",
                partition_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        let mut stmt = tx
            .prepare(
                "INSERT OR IGNORE INTO logs (
                    node_id, service_name, timestamp, level, target, file, line, content, batch_id, record_index, record_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare partition log insert statement: {}", e);
                error!("{}", msg);
                msg
            })?;

        let mut inserted_rows = 0_u64;
        for item in records {
            let changed = stmt
                .execute(params![
                    node,
                    service,
                    item.record.time as i64,
                    item.record.level as i32,
                    item.record.target.as_str(),
                    item.record.file.as_deref(),
                    item.record.line.map(|v| v as i64),
                    item.record.content.as_str(),
                    batch_id,
                    item.record_index as i64,
                    item.record_id.as_deref(),
                ])
                .map_err(|e| {
                    let msg = format!("Failed to append log row to partition db: {}", e);
                    error!("{}", msg);
                    msg
                })?;
            inserted_rows = inserted_rows.saturating_add(changed as u64);
        }

        drop(stmt);
        tx.commit().map_err(|e| {
            let msg = format!(
                "Failed to commit partition append transaction {}: {}",
                partition_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        Ok(inserted_rows)
    }

    fn refresh_partition_stats(
        &self,
        manifest: &Connection,
        partition: &PartitionMeta,
        min_time: u64,
        max_time: u64,
        inserted_rows: u64,
    ) -> Result<(), String> {
        let partition_path = self.partition_path(&partition.file_name);
        let size_bytes = std::fs::metadata(&partition_path)
            .map(|m| m.len())
            .unwrap_or(partition.size_bytes);
        let now = now_unix_secs();

        let updated = manifest
            .execute(
                "UPDATE partitions
                 SET row_count = row_count + ?1,
                     size_bytes = ?2,
                     start_time = MIN(start_time, ?3),
                     end_time = MAX(end_time, ?4),
                     updated_at = ?5
                 WHERE bucket_key = ?6 AND file_name = ?7",
                params![
                    inserted_rows as i64,
                    size_bytes as i64,
                    min_time as i64,
                    max_time as i64,
                    now,
                    &partition.bucket_key,
                    &partition.file_name
                ],
            )
            .map_err(|e| {
                let msg = format!("Failed to update partition metadata after append: {}", e);
                error!("{}", msg);
                msg
            })?;

        if updated != 1 {
            let msg = format!(
                "Unexpected partition metadata update count after append: {}",
                updated
            );
            error!("{}", msg);
            return Err(msg);
        }

        Ok(())
    }

    fn append(&self, logs: LogRecords) -> Result<(), String> {
        let LogRecords {
            node,
            service,
            batch_id,
            record_ids,
            logs,
        } = logs;

        if logs.is_empty() {
            return Ok(());
        }

        let mut grouped: BTreeMap<String, Vec<IndexedRecord>> = BTreeMap::new();
        for (record_index, record) in logs.into_iter().enumerate() {
            let bucket_key = self.config.bucket.bucket_key(record.time);
            grouped.entry(bucket_key).or_default().push(IndexedRecord {
                record_index,
                record_id: record_ids.get(record_index).cloned(),
                record,
            });
        }

        let manifest_lock = self.manifest_conn.lock().map_err(|e| {
            let msg = format!("Failed to lock partition manifest db mutex: {}", e);
            error!("{}", msg);
            msg
        })?;
        let manifest = &*manifest_lock;

        for (bucket_key, records) in grouped {
            let incoming_rows = records.len() as u64;
            let incoming_bytes = Self::estimate_records_bytes(&records);
            let min_time = records.iter().map(|r| r.record.time).min().unwrap_or(0);
            let max_time = records.iter().map(|r| r.record.time).max().unwrap_or(0);

            let target_partition = self.resolve_target_partition(
                manifest,
                &node,
                &service,
                batch_id.as_deref(),
                &bucket_key,
                min_time,
                max_time,
                incoming_rows,
                incoming_bytes,
            )?;

            let partition_path = self.partition_path(&target_partition.file_name);
            let inserted_rows = Self::append_records_to_partition(
                &partition_path,
                &node,
                &service,
                batch_id.as_deref(),
                &records,
            )?;

            self.refresh_partition_stats(
                manifest,
                &target_partition,
                min_time,
                max_time,
                inserted_rows,
            )?;
        }

        Ok(())
    }

    fn list_candidate_partitions(
        manifest: &Connection,
        start_time: Option<u64>,
        end_time: Option<u64>,
    ) -> Result<Vec<PartitionMeta>, String> {
        let mut stmt = manifest
            .prepare(
                "SELECT bucket_key, part_seq, file_name, row_count, size_bytes
                 FROM partitions
                 WHERE (?1 IS NULL OR end_time >= ?1)
                   AND (?2 IS NULL OR start_time <= ?2)
                 ORDER BY end_time DESC, part_seq DESC",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare candidate partitions query: {}", e);
                error!("{}", msg);
                msg
            })?;

        let start_time = start_time.map(|v| v as i64);
        let end_time = end_time.map(|v| v as i64);
        let rows = stmt
            .query_map(params![start_time, end_time], |row| {
                Ok(PartitionMeta {
                    bucket_key: row.get::<_, String>(0)?,
                    part_seq: row.get::<_, i64>(1)?,
                    file_name: row.get::<_, String>(2)?,
                    row_count: row.get::<_, i64>(3)? as u64,
                    size_bytes: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(|e| {
                let msg = format!("Failed to execute candidate partitions query: {}", e);
                error!("{}", msg);
                msg
            })?;

        let mut ret = Vec::new();
        for row in rows {
            let meta = row.map_err(|e| {
                let msg = format!("Failed to map candidate partition row: {}", e);
                error!("{}", msg);
                msg
            })?;
            ret.push(meta);
        }
        Ok(ret)
    }

    fn query_single_partition(
        &self,
        partition_file_name: &str,
        node: Option<&str>,
        service: Option<&str>,
        level: Option<slog::LogLevel>,
        start_time: Option<u64>,
        end_time: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<(String, String, SystemLogRecord)>, String> {
        let partition_path = self.partition_path(partition_file_name);
        if !partition_path.exists() {
            warn!(
                "Skipping missing partition database file during query: {}",
                partition_path.display()
            );
            return Ok(vec![]);
        }

        let conn = Connection::open(&partition_path).map_err(|e| {
            let msg = format!(
                "Failed to open partition database {} for query: {}",
                partition_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        let mut query = String::from(
            "SELECT node_id, service_name, timestamp, level, target, file, line, content
             FROM logs
             WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(node) = node {
            query.push_str(" AND node_id = ? ");
            params.push(Box::new(node.to_string()));
        }
        if let Some(service) = service {
            query.push_str(" AND service_name = ? ");
            params.push(Box::new(service.to_string()));
        }
        if let Some(level) = level {
            query.push_str(" AND level = ? ");
            params.push(Box::new(level as i32));
        }
        if let Some(start_time) = start_time {
            query.push_str(" AND timestamp >= ? ");
            params.push(Box::new(start_time as i64));
        }
        if let Some(end_time) = end_time {
            query.push_str(" AND timestamp <= ? ");
            params.push(Box::new(end_time as i64));
        }

        query.push_str(" ORDER BY timestamp DESC ");
        if let Some(limit) = limit {
            query.push_str(" LIMIT ? ");
            params.push(Box::new(limit as i64));
        }

        let mut stmt = conn.prepare(&query).map_err(|e| {
            let msg = format!(
                "Failed to prepare partition query statement for {}: {}",
                partition_path.display(),
                e
            );
            error!("{}", msg);
            msg
        })?;

        let rows = stmt
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
                let msg = format!(
                    "Failed to execute partition query for {}: {}",
                    partition_path.display(),
                    e
                );
                error!("{}", msg);
                msg
            })?;

        let mut output = Vec::new();
        for row in rows {
            let (node, service, record) = row.map_err(|e| {
                let msg = format!("Failed to map partition query row: {}", e);
                error!("{}", msg);
                msg
            })?;
            match record.try_into() {
                Ok(record) => output.push((node, service, record)),
                Err(e) => warn!("Failed to convert queried log row into record: {}", e),
            }
        }

        Ok(output)
    }

    fn query(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
        let LogQueryRequest {
            node,
            service,
            level,
            start_time,
            end_time,
            limit,
        } = request;

        let candidate_partitions = {
            let manifest_lock = self.manifest_conn.lock().map_err(|e| {
                let msg = format!(
                    "Failed to lock partition manifest db mutex for query: {}",
                    e
                );
                error!("{}", msg);
                msg
            })?;
            Self::list_candidate_partitions(&manifest_lock, start_time, end_time)?
        };

        if candidate_partitions.is_empty() {
            return Ok(vec![]);
        }

        let mut rows: Vec<(String, String, SystemLogRecord)> = Vec::new();
        for partition in candidate_partitions {
            let mut chunk = self.query_single_partition(
                &partition.file_name,
                node.as_deref(),
                service.as_deref(),
                level,
                start_time,
                end_time,
                limit,
            )?;
            rows.append(&mut chunk);
        }

        rows.sort_by(|a, b| b.2.time.cmp(&a.2.time));

        if let Some(limit) = limit {
            rows.truncate(limit);
        }

        let mut records_map: HashMap<(String, String), Vec<SystemLogRecord>> = HashMap::new();
        for (node, service, record) in rows {
            records_map.entry((node, service)).or_default().push(record);
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
impl LogStorage for SqlitePartitionedLogStorage {
    async fn append_logs(&self, logs: LogRecords) -> Result<(), String> {
        self.append(logs)
    }

    async fn query_logs(&self, request: LogQueryRequest) -> Result<Vec<LogRecords>, String> {
        self.query(request)
    }
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn normalize_to_millis(timestamp: u64) -> u64 {
    if timestamp < SECOND_THRESHOLD {
        timestamp.saturating_mul(1000)
    } else {
        timestamp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slog::{LogLevel, SystemLogRecord};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_storage_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "buckyos/slog_server_partitioned_tests/{}_{}_{}",
            prefix,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup_storage_dir(path: &Path) {
        if path.exists() {
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    fn record(time: u64, content: &str) -> SystemLogRecord {
        SystemLogRecord {
            level: LogLevel::Info,
            target: "test-target".to_string(),
            time,
            file: None,
            line: None,
            content: content.to_string(),
        }
    }

    fn payload(
        node: &str,
        service: &str,
        batch_id: &str,
        rows: Vec<SystemLogRecord>,
    ) -> LogRecords {
        let record_ids = rows
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("{}-rid-{}", batch_id, idx))
            .collect::<Vec<_>>();
        LogRecords {
            node: node.to_string(),
            service: service.to_string(),
            batch_id: Some(batch_id.to_string()),
            record_ids,
            logs: rows,
        }
    }

    fn query_partition_count(storage: &SqlitePartitionedLogStorage) -> i64 {
        storage
            .manifest_conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM partitions", [], |row| row.get(0))
            .unwrap()
    }

    fn query_first_partition_file_name(storage: &SqlitePartitionedLogStorage) -> String {
        storage
            .manifest_conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT file_name FROM partitions ORDER BY part_seq ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn test_partitioned_storage_rollover_by_row_limit() {
        let storage_dir = temp_storage_dir("row_rollover");
        let storage = SqlitePartitionedLogStorage::open(
            &storage_dir,
            SqlitePartitionedConfig {
                bucket: PartitionBucket::Day,
                max_rows_per_partition: 3,
                max_partition_size_bytes: 1024 * 1024 * 1024,
            },
        )
        .unwrap();

        storage
            .append(payload(
                "node-1",
                "svc-a",
                "batch-1",
                vec![
                    record(1_721_000_000_000, "a-1"),
                    record(1_721_000_000_010, "a-2"),
                ],
            ))
            .unwrap();
        storage
            .append(payload(
                "node-1",
                "svc-a",
                "batch-2",
                vec![
                    record(1_721_000_000_020, "a-3"),
                    record(1_721_000_000_030, "a-4"),
                ],
            ))
            .unwrap();

        assert_eq!(query_partition_count(&storage), 2);

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
        assert_eq!(queried[0].logs.len(), 4);

        cleanup_storage_dir(&storage_dir);
    }

    #[test]
    fn test_partitioned_storage_batch_mapping_keeps_idempotency_across_rollover() {
        let storage_dir = temp_storage_dir("batch_mapping_idempotent");
        let storage = SqlitePartitionedLogStorage::open(
            &storage_dir,
            SqlitePartitionedConfig {
                bucket: PartitionBucket::Day,
                max_rows_per_partition: 1,
                max_partition_size_bytes: 1024 * 1024 * 1024,
            },
        )
        .unwrap();

        storage
            .append(payload(
                "node-1",
                "svc-a",
                "batch-a",
                vec![record(1_721_000_100_000, "seed")],
            ))
            .unwrap();

        let retry_payload = payload(
            "node-1",
            "svc-a",
            "batch-b",
            vec![record(1_721_000_100_010, "dup-me")],
        );
        storage.append(retry_payload.clone()).unwrap();
        storage.append(retry_payload).unwrap();

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

        assert_eq!(query_partition_count(&storage), 2);

        cleanup_storage_dir(&storage_dir);
    }

    #[test]
    fn test_partitioned_storage_query_cross_day_with_global_limit() {
        let storage_dir = temp_storage_dir("cross_day_limit");
        let storage = SqlitePartitionedLogStorage::open(
            &storage_dir,
            SqlitePartitionedConfig {
                bucket: PartitionBucket::Day,
                max_rows_per_partition: 100,
                max_partition_size_bytes: 1024 * 1024 * 1024,
            },
        )
        .unwrap();

        let day1 = 1_721_000_000_000_u64;
        let day2 = day1 + DAY_MILLIS;
        storage
            .append(payload(
                "node-1",
                "svc-a",
                "batch-day-1",
                vec![
                    record(day1 + 10, "d1-1"),
                    record(day1 + 20, "d1-2"),
                    record(day1 + 30, "d1-3"),
                ],
            ))
            .unwrap();
        storage
            .append(payload(
                "node-1",
                "svc-a",
                "batch-day-2",
                vec![
                    record(day2 + 10, "d2-1"),
                    record(day2 + 20, "d2-2"),
                    record(day2 + 30, "d2-3"),
                ],
            ))
            .unwrap();

        let queried = storage
            .query(LogQueryRequest {
                node: Some("node-1".to_string()),
                service: Some("svc-a".to_string()),
                level: None,
                start_time: Some(day1),
                end_time: Some(day2 + 40),
                limit: Some(3),
            })
            .unwrap();
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].logs.len(), 3);
        assert_eq!(queried[0].logs[0].content, "d2-3");
        assert_eq!(queried[0].logs[1].content, "d2-2");
        assert_eq!(queried[0].logs[2].content, "d2-1");

        cleanup_storage_dir(&storage_dir);
    }

    #[test]
    fn test_partitioned_storage_rollover_by_size_limit() {
        let storage_dir = temp_storage_dir("size_rollover");
        let storage = SqlitePartitionedLogStorage::open(
            &storage_dir,
            SqlitePartitionedConfig {
                bucket: PartitionBucket::Day,
                max_rows_per_partition: 10_000,
                max_partition_size_bytes: 1,
            },
        )
        .unwrap();

        storage
            .append(payload(
                "node-1",
                "svc-size",
                "batch-size-1",
                vec![record(1_721_100_000_000, "first")],
            ))
            .unwrap();
        storage
            .append(payload(
                "node-1",
                "svc-size",
                "batch-size-2",
                vec![record(1_721_100_000_010, "second")],
            ))
            .unwrap();

        assert_eq!(query_partition_count(&storage), 2);

        cleanup_storage_dir(&storage_dir);
    }

    #[test]
    fn test_partitioned_storage_seconds_timestamp_creates_expected_day_buckets() {
        let storage_dir = temp_storage_dir("seconds_bucket");
        let storage = SqlitePartitionedLogStorage::open(
            &storage_dir,
            SqlitePartitionedConfig {
                bucket: PartitionBucket::Day,
                max_rows_per_partition: 100,
                max_partition_size_bytes: 1024 * 1024 * 1024,
            },
        )
        .unwrap();

        let day1_secs = 1_721_200_000_u64;
        let day2_secs = day1_secs + 86_400;
        storage
            .append(payload(
                "node-1",
                "svc-seconds",
                "batch-seconds-1",
                vec![record(day1_secs, "s-day1")],
            ))
            .unwrap();
        storage
            .append(payload(
                "node-1",
                "svc-seconds",
                "batch-seconds-2",
                vec![record(day2_secs, "s-day2")],
            ))
            .unwrap();

        assert_eq!(query_partition_count(&storage), 2);

        let queried = storage
            .query(LogQueryRequest {
                node: Some("node-1".to_string()),
                service: Some("svc-seconds".to_string()),
                level: None,
                start_time: Some(day1_secs),
                end_time: Some(day2_secs + 60),
                limit: None,
            })
            .unwrap();
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].logs.len(), 2);

        cleanup_storage_dir(&storage_dir);
    }

    #[test]
    fn test_partitioned_storage_query_isolated_by_node_and_service() {
        let storage_dir = temp_storage_dir("query_isolation");
        let storage = SqlitePartitionedLogStorage::open(
            &storage_dir,
            SqlitePartitionedConfig {
                bucket: PartitionBucket::Day,
                max_rows_per_partition: 100,
                max_partition_size_bytes: 1024 * 1024 * 1024,
            },
        )
        .unwrap();

        let day1 = 1_721_300_000_000_u64;
        let day2 = day1 + DAY_MILLIS;
        storage
            .append(payload(
                "node-a",
                "svc-shared",
                "batch-a",
                vec![record(day1 + 10, "a-1"), record(day2 + 10, "a-2")],
            ))
            .unwrap();
        storage
            .append(payload(
                "node-b",
                "svc-shared",
                "batch-b",
                vec![record(day2 + 20, "b-1")],
            ))
            .unwrap();

        let node_a = storage
            .query(LogQueryRequest {
                node: Some("node-a".to_string()),
                service: Some("svc-shared".to_string()),
                level: None,
                start_time: None,
                end_time: None,
                limit: None,
            })
            .unwrap();
        assert_eq!(node_a.len(), 1);
        assert_eq!(node_a[0].logs.len(), 2);
        assert!(
            node_a[0]
                .logs
                .iter()
                .all(|log| log.content.starts_with("a-"))
        );

        let node_b = storage
            .query(LogQueryRequest {
                node: Some("node-b".to_string()),
                service: Some("svc-shared".to_string()),
                level: None,
                start_time: None,
                end_time: None,
                limit: None,
            })
            .unwrap();
        assert_eq!(node_b.len(), 1);
        assert_eq!(node_b[0].logs.len(), 1);
        assert_eq!(node_b[0].logs[0].content, "b-1");

        cleanup_storage_dir(&storage_dir);
    }

    #[test]
    fn test_partitioned_storage_reconciles_stale_manifest_time_range_on_reopen() {
        let storage_dir = temp_storage_dir("reconcile_stale_time_range");
        let first_time = 1_721_400_000_000_u64;
        let second_time = first_time + 100;

        let storage = SqlitePartitionedLogStorage::open(
            &storage_dir,
            SqlitePartitionedConfig {
                bucket: PartitionBucket::Day,
                max_rows_per_partition: 100,
                max_partition_size_bytes: 1024 * 1024 * 1024,
            },
        )
        .unwrap();
        storage
            .append(payload(
                "node-1",
                "svc-reconcile",
                "batch-r-1",
                vec![record(first_time, "r-first")],
            ))
            .unwrap();
        storage
            .append(payload(
                "node-1",
                "svc-reconcile",
                "batch-r-2",
                vec![record(second_time, "r-second")],
            ))
            .unwrap();

        let file_name = query_first_partition_file_name(&storage);
        storage
            .manifest_conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE partitions
                 SET start_time = ?1, end_time = ?2, row_count = 1
                 WHERE file_name = ?3",
                params![first_time as i64, first_time as i64, &file_name],
            )
            .unwrap();
        drop(storage);

        let storage_reopened =
            SqlitePartitionedLogStorage::open(&storage_dir, SqlitePartitionedConfig::default())
                .unwrap();
        let queried = storage_reopened
            .query(LogQueryRequest {
                node: Some("node-1".to_string()),
                service: Some("svc-reconcile".to_string()),
                level: None,
                start_time: Some(second_time),
                end_time: Some(second_time),
                limit: None,
            })
            .unwrap();
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].logs.len(), 1);
        assert_eq!(queried[0].logs[0].content, "r-second");

        let repaired_end_time: i64 = storage_reopened
            .manifest_conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT end_time FROM partitions WHERE file_name = ?1",
                params![&file_name],
                |row| row.get(0),
            )
            .unwrap();
        assert!(repaired_end_time as u64 >= second_time);

        cleanup_storage_dir(&storage_dir);
    }

    #[test]
    fn test_partitioned_storage_recovers_missing_manifest_entry_on_reopen() {
        let storage_dir = temp_storage_dir("reconcile_missing_manifest_entry");
        let storage = SqlitePartitionedLogStorage::open(
            &storage_dir,
            SqlitePartitionedConfig {
                bucket: PartitionBucket::Day,
                max_rows_per_partition: 100,
                max_partition_size_bytes: 1024 * 1024 * 1024,
            },
        )
        .unwrap();
        storage
            .append(payload(
                "node-1",
                "svc-recover",
                "batch-rm-1",
                vec![record(1_721_500_000_000, "recover-me")],
            ))
            .unwrap();

        let file_name = query_first_partition_file_name(&storage);
        storage
            .manifest_conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM partitions WHERE file_name = ?1",
                params![&file_name],
            )
            .unwrap();
        drop(storage);

        let storage_reopened =
            SqlitePartitionedLogStorage::open(&storage_dir, SqlitePartitionedConfig::default())
                .unwrap();
        assert_eq!(query_partition_count(&storage_reopened), 1);

        let queried = storage_reopened
            .query(LogQueryRequest {
                node: Some("node-1".to_string()),
                service: Some("svc-recover".to_string()),
                level: None,
                start_time: None,
                end_time: None,
                limit: None,
            })
            .unwrap();
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].logs.len(), 1);
        assert_eq!(queried[0].logs[0].content, "recover-me");

        cleanup_storage_dir(&storage_dir);
    }
}
