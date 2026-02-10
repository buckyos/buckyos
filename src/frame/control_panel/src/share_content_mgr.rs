use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::{
    AccessLogEntry, ContentMgrHandler, LogFilter, PublishRequest, RevisionMetadata, SharedItemInfo,
    TimeBucketStat,
};
use log::warn;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};

const DEFAULT_CONTENT_MGR_DB_PATH: &str = "/opt/buckyos/data/share_content_mgr.sqlite3";
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 1000;
const DEFAULT_LOG_LIMIT: usize = 200;
const MAX_LOG_LIMIT: usize = 2000;
const HOURLY_BUCKET_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Debug)]
pub struct ShareContentMgr {
    db_path: PathBuf,
}

impl ShareContentMgr {
    pub fn new() -> std::result::Result<Self, RPCErrors> {
        let db_path = std::env::var("BUCKYOS_CONTENT_MGR_DB_PATH")
            .unwrap_or_else(|_| DEFAULT_CONTENT_MGR_DB_PATH.to_string());
        Self::new_with_path(db_path)
    }

    pub fn new_with_path<P: AsRef<Path>>(db_path: P) -> std::result::Result<Self, RPCErrors> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "Failed to create content mgr db dir {}: {}",
                    parent.display(),
                    error
                ))
            })?;
        }
        let mgr = Self { db_path };
        mgr.init_db()?;
        Ok(mgr)
    }

    fn connect(&self) -> std::result::Result<Connection, RPCErrors> {
        let conn = Connection::open(&self.db_path).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to open content mgr db {}: {}",
                self.db_path.display(),
                error
            ))
        })?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "Failed to configure content mgr db {}: {}",
                    self.db_path.display(),
                    error
                ))
            })?;
        Ok(conn)
    }

    fn init_db(&self) -> std::result::Result<(), RPCErrors> {
        let conn = self.connect()?;
        conn.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS published_items (
    name TEXT PRIMARY KEY,
    current_obj_id TEXT NOT NULL,
    share_policy TEXT NOT NULL,
    share_policy_config TEXT,
    sequence INTEGER NOT NULL DEFAULT 1,
    enabled INTEGER NOT NULL DEFAULT 1,
    disabled_reason TEXT,
    disabled_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS item_revisions (
    name TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    obj_id TEXT NOT NULL,
    share_policy TEXT,
    share_policy_config TEXT,
    enabled INTEGER,
    committed_at INTEGER NOT NULL,
    op_device_id TEXT,
    PRIMARY KEY (name, sequence),
    FOREIGN KEY (name) REFERENCES published_items(name) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS access_logs (
    log_id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    req_ts INTEGER NOT NULL,
    source_device_id TEXT,
    bytes_sent INTEGER DEFAULT 0,
    status_code INTEGER DEFAULT 200,
    user_agent TEXT
);

CREATE TABLE IF NOT EXISTS access_stats (
    name TEXT NOT NULL,
    time_bucket INTEGER NOT NULL,
    request_count INTEGER DEFAULT 0,
    bytes_sent INTEGER DEFAULT 0,
    last_access_ts INTEGER,
    PRIMARY KEY (name, time_bucket)
);

CREATE INDEX IF NOT EXISTS idx_revisions_name ON item_revisions(name);
CREATE INDEX IF NOT EXISTS idx_logs_ts ON access_logs(req_ts);
CREATE INDEX IF NOT EXISTS idx_logs_name_ts ON access_logs(name, req_ts);
CREATE INDEX IF NOT EXISTS idx_stats_time ON access_stats(time_bucket);
"#,
        )
        .map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to initialize content mgr schema in {}: {}",
                self.db_path.display(),
                error
            ))
        })?;
        Ok(())
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn validate_name(name: &str) -> std::result::Result<(), RPCErrors> {
        if name.is_empty() || name.len() > 256 {
            return Err(RPCErrors::ReasonError(format!(
                "Invalid name length for '{}'",
                name
            )));
        }
        let valid = name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | '~'));
        if !valid {
            return Err(RPCErrors::ReasonError(format!(
                "Invalid name characters for '{}'",
                name
            )));
        }
        Ok(())
    }

    fn serialize_json_value(
        value: &Option<Value>,
    ) -> std::result::Result<Option<String>, RPCErrors> {
        match value {
            Some(value) => serde_json::to_string(value).map(Some).map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to serialize JSON value: {}", error))
            }),
            None => Ok(None),
        }
    }

    fn parse_json_value(value: Option<String>) -> Option<Value> {
        match value {
            Some(raw) => match serde_json::from_str::<Value>(&raw) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    warn!("Failed to parse share_policy_config '{}': {}", raw, error);
                    None
                }
            },
            None => None,
        }
    }

    fn i64_to_u64(field: &str, value: i64) -> std::result::Result<u64, RPCErrors> {
        if value < 0 {
            return Err(RPCErrors::ReasonError(format!(
                "Invalid negative value for {}: {}",
                field, value
            )));
        }
        Ok(value as u64)
    }

    fn i64_to_u64_saturating(value: i64) -> u64 {
        if value < 0 {
            0
        } else {
            value as u64
        }
    }

    fn normalize_limit(value: Option<usize>, default_limit: usize, max_limit: usize) -> usize {
        value.unwrap_or(default_limit).clamp(1, max_limit)
    }

    fn normalize_offset(value: Option<u64>) -> u64 {
        value.unwrap_or(0)
    }

    fn bool_to_i64(value: bool) -> i64 {
        if value {
            1
        } else {
            0
        }
    }
}

#[async_trait]
impl ContentMgrHandler for ShareContentMgr {
    async fn handle_publish(
        &self,
        request: PublishRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<u64, RPCErrors> {
        Self::validate_name(&request.name)?;
        let mut conn = self.connect()?;
        let tx = conn.transaction().map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to begin publish transaction: {}", error))
        })?;

        let now_ms = Self::now_ms();
        let obj_id = request.obj_id.to_string();
        let share_policy_config = Self::serialize_json_value(&request.share_policy_config)?;

        let existing = tx
            .query_row(
                r#"SELECT sequence, enabled
                FROM published_items
                WHERE name = ?1"#,
                params![request.name],
                |row| {
                    let sequence: i64 = row.get(0)?;
                    let enabled: i64 = row.get(1)?;
                    Ok((sequence, enabled))
                },
            )
            .optional()
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to query published item: {}", error))
            })?;

        let new_sequence: u64 = if let Some((current_sequence, enabled)) = existing {
            let current_sequence_u64 = Self::i64_to_u64("sequence", current_sequence)?;
            if let Some(expected) = request.expected_sequence {
                if expected != current_sequence_u64 {
                    return Err(RPCErrors::ReasonError(format!(
                        "CAS mismatch for '{}': expected {}, current {}",
                        request.name, expected, current_sequence_u64
                    )));
                }
            }
            let next_sequence = current_sequence_u64 + 1;
            tx.execute(
                r#"UPDATE published_items
                SET current_obj_id = ?1,
                    share_policy = ?2,
                    share_policy_config = ?3,
                    sequence = ?4,
                    updated_at = ?5
                WHERE name = ?6"#,
                params![
                    obj_id,
                    request.share_policy,
                    share_policy_config,
                    next_sequence as i64,
                    now_ms as i64,
                    request.name
                ],
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to update published item: {}", error))
            })?;

            tx.execute(
                r#"INSERT INTO item_revisions (
                    name, sequence, obj_id, share_policy, share_policy_config, enabled, committed_at, op_device_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                params![
                    request.name,
                    next_sequence as i64,
                    obj_id,
                    request.share_policy,
                    share_policy_config,
                    enabled,
                    now_ms as i64,
                    request.op_device_id
                ],
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to insert item revision: {}", error))
            })?;
            next_sequence
        } else {
            if let Some(expected) = request.expected_sequence {
                if expected != 0 {
                    return Err(RPCErrors::ReasonError(format!(
                        "CAS mismatch for new '{}': expected {}, current 0",
                        request.name, expected
                    )));
                }
            }
            let initial_sequence = 1_u64;
            tx.execute(
                r#"INSERT INTO published_items (
                    name, current_obj_id, share_policy, share_policy_config, sequence,
                    enabled, disabled_reason, disabled_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, 1, NULL, NULL, ?6, ?6)"#,
                params![
                    request.name,
                    obj_id,
                    request.share_policy,
                    share_policy_config,
                    initial_sequence as i64,
                    now_ms as i64
                ],
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to create published item: {}", error))
            })?;

            tx.execute(
                r#"INSERT INTO item_revisions (
                    name, sequence, obj_id, share_policy, share_policy_config, enabled, committed_at, op_device_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)"#,
                params![
                    request.name,
                    initial_sequence as i64,
                    obj_id,
                    request.share_policy,
                    share_policy_config,
                    now_ms as i64,
                    request.op_device_id
                ],
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to insert first item revision: {}", error))
            })?;
            initial_sequence
        };

        tx.commit().map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to commit publish transaction: {}", error))
        })?;
        Ok(new_sequence)
    }

    async fn handle_resolve(
        &self,
        name: &str,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<String>, RPCErrors> {
        Self::validate_name(name)?;
        let conn = self.connect()?;
        let row = conn
            .query_row(
                r#"SELECT current_obj_id, enabled
                FROM published_items
                WHERE name = ?1"#,
                params![name],
                |row| {
                    let obj_id: String = row.get(0)?;
                    let enabled: i64 = row.get(1)?;
                    Ok((obj_id, enabled))
                },
            )
            .optional()
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to resolve published item: {}", error))
            })?;

        match row {
            Some((obj_id, enabled)) if enabled != 0 => Ok(Some(obj_id)),
            _ => Ok(None),
        }
    }

    async fn handle_resolve_version(
        &self,
        name: &str,
        sequence: u64,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<String>, RPCErrors> {
        Self::validate_name(name)?;
        let conn = self.connect()?;
        let row = conn
            .query_row(
                r#"SELECT r.obj_id, p.enabled
                FROM item_revisions r
                JOIN published_items p ON p.name = r.name
                WHERE r.name = ?1 AND r.sequence = ?2"#,
                params![name, sequence as i64],
                |row| {
                    let obj_id: String = row.get(0)?;
                    let enabled: i64 = row.get(1)?;
                    Ok((obj_id, enabled))
                },
            )
            .optional()
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to resolve version: {}", error))
            })?;

        match row {
            Some((obj_id, enabled)) if enabled != 0 => Ok(Some(obj_id)),
            _ => Ok(None),
        }
    }

    async fn handle_get_item(
        &self,
        name: &str,
        _ctx: RPCContext,
    ) -> std::result::Result<Option<SharedItemInfo>, RPCErrors> {
        Self::validate_name(name)?;
        let conn = self.connect()?;
        let item = conn
            .query_row(
                r#"
SELECT
    p.name,
    p.current_obj_id,
    p.share_policy,
    p.share_policy_config,
    p.enabled,
    p.disabled_reason,
    p.disabled_at,
    p.sequence,
    (SELECT COUNT(*) FROM item_revisions r WHERE r.name = p.name) AS history_count,
    p.created_at,
    p.updated_at
FROM published_items p
WHERE p.name = ?1
"#,
                params![name],
                |row| {
                    let share_policy_config_raw: Option<String> = row.get(3)?;
                    let enabled_i64: i64 = row.get(4)?;
                    let disabled_at_i64: Option<i64> = row.get(6)?;
                    let sequence_i64: i64 = row.get(7)?;
                    let history_count_i64: i64 = row.get(8)?;
                    let created_at_i64: i64 = row.get(9)?;
                    let updated_at_i64: i64 = row.get(10)?;
                    Ok(SharedItemInfo {
                        name: row.get(0)?,
                        current_obj_id: row.get(1)?,
                        share_policy: row.get(2)?,
                        share_policy_config: Self::parse_json_value(share_policy_config_raw),
                        enabled: enabled_i64 != 0,
                        disabled_reason: row.get(5)?,
                        disabled_at: match disabled_at_i64 {
                            Some(value) => Some(Self::i64_to_u64_saturating(value)),
                            None => None,
                        },
                        sequence: Self::i64_to_u64_saturating(sequence_i64),
                        history_count: Self::i64_to_u64_saturating(history_count_i64),
                        created_at: Self::i64_to_u64_saturating(created_at_i64),
                        updated_at: Self::i64_to_u64_saturating(updated_at_i64),
                    })
                },
            )
            .optional()
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to get item '{}': {}", name, error))
            })?;

        Ok(item)
    }

    async fn handle_set_item_enabled(
        &self,
        name: &str,
        enabled: bool,
        reason: Option<&str>,
        _ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        Self::validate_name(name)?;
        let mut conn = self.connect()?;
        let tx = conn.transaction().map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to begin enable transaction: {}", error))
        })?;

        let row = tx
            .query_row(
                r#"SELECT current_obj_id, share_policy, share_policy_config, sequence
                FROM published_items
                WHERE name = ?1"#,
                params![name],
                |row| {
                    let current_obj_id: String = row.get(0)?;
                    let share_policy: String = row.get(1)?;
                    let share_policy_config: Option<String> = row.get(2)?;
                    let sequence: i64 = row.get(3)?;
                    Ok((current_obj_id, share_policy, share_policy_config, sequence))
                },
            )
            .optional()
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "Failed to read item '{}' for enable: {}",
                    name, error
                ))
            })?;

        let (current_obj_id, share_policy, share_policy_config, current_sequence) =
            row.ok_or_else(|| RPCErrors::ReasonError(format!("Item '{}' not found", name)))?;
        let current_sequence = Self::i64_to_u64("sequence", current_sequence)?;
        let next_sequence = current_sequence + 1;
        let now_ms = Self::now_ms() as i64;

        let (disabled_reason, disabled_at): (Option<String>, Option<i64>) = if enabled {
            (None, None)
        } else {
            (reason.map(|value| value.to_string()), Some(now_ms))
        };

        tx.execute(
            r#"UPDATE published_items
            SET enabled = ?1,
                disabled_reason = ?2,
                disabled_at = ?3,
                sequence = ?4,
                updated_at = ?5
            WHERE name = ?6"#,
            params![
                Self::bool_to_i64(enabled),
                disabled_reason,
                disabled_at,
                next_sequence as i64,
                now_ms,
                name
            ],
        )
        .map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to update enabled for '{}': {}",
                name, error
            ))
        })?;

        tx.execute(
            r#"INSERT INTO item_revisions (
                name, sequence, obj_id, share_policy, share_policy_config, enabled, committed_at, op_device_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)"#,
            params![
                name,
                next_sequence as i64,
                current_obj_id,
                share_policy,
                share_policy_config,
                Self::bool_to_i64(enabled),
                now_ms
            ],
        )
        .map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to create enable revision for '{}': {}",
                name, error
            ))
        })?;

        tx.commit().map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to commit enable transaction: {}", error))
        })?;
        Ok(())
    }

    async fn handle_list_items(
        &self,
        prefix: Option<&str>,
        limit: Option<usize>,
        offset: Option<u64>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<SharedItemInfo>, RPCErrors> {
        if let Some(prefix) = prefix {
            if !prefix.is_empty() {
                Self::validate_name(prefix)?;
            }
        }

        let limit = Self::normalize_limit(limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT);
        let offset = Self::normalize_offset(offset);

        let conn = self.connect()?;
        let prefix_value = prefix.map(|value| value.to_string());
        let like_value = prefix_value.as_ref().map(|value| format!("{}%", value));

        let mut stmt = conn
            .prepare(
                r#"
SELECT
    p.name,
    p.current_obj_id,
    p.share_policy,
    p.share_policy_config,
    p.enabled,
    p.disabled_reason,
    p.disabled_at,
    p.sequence,
    (SELECT COUNT(*) FROM item_revisions r WHERE r.name = p.name) AS history_count,
    p.created_at,
    p.updated_at
FROM published_items p
WHERE (?1 IS NULL OR p.name LIKE ?2)
ORDER BY p.updated_at DESC, p.name ASC
LIMIT ?3 OFFSET ?4
"#,
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to prepare list_items query: {}", error))
            })?;

        let rows = stmt
            .query_map(
                params![prefix_value, like_value, limit as i64, offset as i64],
                |row| {
                    let share_policy_config_raw: Option<String> = row.get(3)?;
                    let enabled_i64: i64 = row.get(4)?;
                    let disabled_at_i64: Option<i64> = row.get(6)?;
                    let sequence_i64: i64 = row.get(7)?;
                    let history_count_i64: i64 = row.get(8)?;
                    let created_at_i64: i64 = row.get(9)?;
                    let updated_at_i64: i64 = row.get(10)?;

                    Ok(SharedItemInfo {
                        name: row.get(0)?,
                        current_obj_id: row.get(1)?,
                        share_policy: row.get(2)?,
                        share_policy_config: Self::parse_json_value(share_policy_config_raw),
                        enabled: enabled_i64 != 0,
                        disabled_reason: row.get(5)?,
                        disabled_at: match disabled_at_i64 {
                            Some(value) => Some(Self::i64_to_u64_saturating(value)),
                            None => None,
                        },
                        sequence: Self::i64_to_u64_saturating(sequence_i64),
                        history_count: Self::i64_to_u64_saturating(history_count_i64),
                        created_at: Self::i64_to_u64_saturating(created_at_i64),
                        updated_at: Self::i64_to_u64_saturating(updated_at_i64),
                    })
                },
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to execute list_items query: {}", error))
            })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to parse list_items row: {}", error))
            })?);
        }
        Ok(result)
    }

    async fn handle_list_history(
        &self,
        name: &str,
        limit: Option<usize>,
        offset: Option<u64>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<RevisionMetadata>, RPCErrors> {
        Self::validate_name(name)?;
        let limit = Self::normalize_limit(limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT);
        let offset = Self::normalize_offset(offset);

        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
SELECT sequence, obj_id, share_policy, share_policy_config, enabled, committed_at, op_device_id
FROM item_revisions
WHERE name = ?1
ORDER BY sequence DESC
LIMIT ?2 OFFSET ?3
"#,
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to prepare list_history query: {}", error))
            })?;

        let rows = stmt
            .query_map(params![name, limit as i64, offset as i64], |row| {
                let sequence_i64: i64 = row.get(0)?;
                let share_policy_config_raw: Option<String> = row.get(3)?;
                let enabled_i64: Option<i64> = row.get(4)?;
                let committed_at_i64: i64 = row.get(5)?;
                Ok(RevisionMetadata {
                    name: name.to_string(),
                    sequence: Self::i64_to_u64_saturating(sequence_i64),
                    obj_id: row.get(1)?,
                    share_policy: row.get(2)?,
                    share_policy_config: Self::parse_json_value(share_policy_config_raw),
                    enabled: enabled_i64.map(|value| value != 0),
                    committed_at: Self::i64_to_u64_saturating(committed_at_i64),
                    op_device_id: row.get(6)?,
                })
            })
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to execute list_history query: {}", error))
            })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to parse list_history row: {}", error))
            })?);
        }
        Ok(result)
    }

    async fn handle_record_batch(
        &self,
        logs: Vec<AccessLogEntry>,
        _ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        if logs.is_empty() {
            return Ok(());
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction().map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to begin record_batch transaction: {}",
                error
            ))
        })?;

        for log_entry in logs {
            Self::validate_name(&log_entry.name)?;
            let req_ts_i64 = log_entry.req_ts as i64;
            let bytes_sent_i64 = log_entry.bytes_sent as i64;
            let status_code_i64 = log_entry.status_code as i64;
            tx.execute(
                r#"INSERT INTO access_logs (
                    name, req_ts, source_device_id, bytes_sent, status_code, user_agent
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                params![
                    log_entry.name,
                    req_ts_i64,
                    log_entry.source_device_id,
                    bytes_sent_i64,
                    status_code_i64,
                    log_entry.user_agent
                ],
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to insert access log: {}", error))
            })?;

            let bucket = ((log_entry.req_ts / HOURLY_BUCKET_MS) * HOURLY_BUCKET_MS) as i64;
            tx.execute(
                r#"INSERT INTO access_stats (
                    name, time_bucket, request_count, bytes_sent, last_access_ts
                ) VALUES (?1, ?2, 1, ?3, ?4)
                ON CONFLICT(name, time_bucket) DO UPDATE SET
                    request_count = access_stats.request_count + 1,
                    bytes_sent = access_stats.bytes_sent + excluded.bytes_sent,
                    last_access_ts = CASE
                        WHEN access_stats.last_access_ts IS NULL THEN excluded.last_access_ts
                        WHEN excluded.last_access_ts > access_stats.last_access_ts THEN excluded.last_access_ts
                        ELSE access_stats.last_access_ts
                    END"#,
                params![log_entry.name, bucket, bytes_sent_i64, req_ts_i64],
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to upsert access_stats: {}", error))
            })?;
        }

        tx.commit().map_err(|error| {
            RPCErrors::ReasonError(format!(
                "Failed to commit record_batch transaction: {}",
                error
            ))
        })?;
        Ok(())
    }

    async fn handle_get_stats(
        &self,
        name: &str,
        start_ts: u64,
        end_ts: u64,
        bucket_size: Option<u64>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<TimeBucketStat>, RPCErrors> {
        Self::validate_name(name)?;
        if end_ts < start_ts {
            return Err(RPCErrors::ReasonError(format!(
                "Invalid range: end_ts({}) < start_ts({})",
                end_ts, start_ts
            )));
        }
        let bucket_size = bucket_size.unwrap_or(HOURLY_BUCKET_MS).max(1);
        let conn = self.connect()?;

        if bucket_size == HOURLY_BUCKET_MS {
            let start_bucket = (start_ts / HOURLY_BUCKET_MS) * HOURLY_BUCKET_MS;
            let end_bucket = (end_ts / HOURLY_BUCKET_MS) * HOURLY_BUCKET_MS;
            let mut stmt = conn
                .prepare(
                    r#"
SELECT time_bucket, request_count, bytes_sent, last_access_ts
FROM access_stats
WHERE name = ?1
  AND time_bucket >= ?2
  AND time_bucket <= ?3
ORDER BY time_bucket ASC
"#,
                )
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("Failed to prepare stats query: {}", error))
                })?;

            let rows = stmt
                .query_map(
                    params![name, start_bucket as i64, end_bucket as i64],
                    |row| {
                        let time_bucket_i64: i64 = row.get(0)?;
                        let request_count_i64: i64 = row.get(1)?;
                        let bytes_sent_i64: i64 = row.get(2)?;
                        let last_access_ts_i64: Option<i64> = row.get(3)?;
                        Ok(TimeBucketStat {
                            name: name.to_string(),
                            time_bucket: Self::i64_to_u64_saturating(time_bucket_i64),
                            request_count: Self::i64_to_u64_saturating(request_count_i64),
                            bytes_sent: Self::i64_to_u64_saturating(bytes_sent_i64),
                            last_access_ts: match last_access_ts_i64 {
                                Some(value) => Some(Self::i64_to_u64_saturating(value)),
                                None => None,
                            },
                        })
                    },
                )
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("Failed to execute stats query: {}", error))
                })?;

            let mut result = Vec::new();
            for row in rows {
                result.push(row.map_err(|error| {
                    RPCErrors::ReasonError(format!("Failed to parse stats row: {}", error))
                })?);
            }
            return Ok(result);
        }

        let mut stmt = conn
            .prepare(
                r#"
SELECT
    ((req_ts / ?1) * ?1) AS bucket_start,
    COUNT(*) AS request_count,
    SUM(bytes_sent) AS bytes_sent,
    MAX(req_ts) AS last_access_ts
FROM access_logs
WHERE name = ?2
  AND req_ts >= ?3
  AND req_ts <= ?4
GROUP BY bucket_start
ORDER BY bucket_start ASC
"#,
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to prepare grouped stats query: {}", error))
            })?;

        let rows = stmt
            .query_map(
                params![bucket_size as i64, name, start_ts as i64, end_ts as i64],
                |row| {
                    let time_bucket_i64: i64 = row.get(0)?;
                    let request_count_i64: i64 = row.get(1)?;
                    let bytes_sent_i64: i64 = row.get(2)?;
                    let last_access_ts_i64: Option<i64> = row.get(3)?;
                    Ok(TimeBucketStat {
                        name: name.to_string(),
                        time_bucket: Self::i64_to_u64_saturating(time_bucket_i64),
                        request_count: Self::i64_to_u64_saturating(request_count_i64),
                        bytes_sent: Self::i64_to_u64_saturating(bytes_sent_i64),
                        last_access_ts: match last_access_ts_i64 {
                            Some(value) => Some(Self::i64_to_u64_saturating(value)),
                            None => None,
                        },
                    })
                },
            )
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to execute grouped stats query: {}", error))
            })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to parse grouped stats row: {}", error))
            })?);
        }
        Ok(result)
    }

    async fn handle_query_logs(
        &self,
        filter: LogFilter,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<AccessLogEntry>, RPCErrors> {
        if let Some(name) = filter.name.as_deref() {
            Self::validate_name(name)?;
        }
        let limit = Self::normalize_limit(filter.limit, DEFAULT_LOG_LIMIT, MAX_LOG_LIMIT);
        let offset = Self::normalize_offset(filter.offset);
        let conn = self.connect()?;

        let mut sql = String::from(
            r#"SELECT name, req_ts, source_device_id, bytes_sent, status_code, user_agent
FROM access_logs
WHERE 1 = 1"#,
        );
        let mut params_vec: Vec<SqlValue> = Vec::new();

        if let Some(name) = filter.name {
            sql.push_str(" AND name = ?");
            params_vec.push(SqlValue::Text(name));
        }
        if let Some(source_device_id) = filter.source_device_id {
            sql.push_str(" AND source_device_id = ?");
            params_vec.push(SqlValue::Text(source_device_id));
        }
        if let Some(status_code) = filter.status_code {
            sql.push_str(" AND status_code = ?");
            params_vec.push(SqlValue::Integer(status_code as i64));
        }
        if let Some(start_ts) = filter.start_ts {
            sql.push_str(" AND req_ts >= ?");
            params_vec.push(SqlValue::Integer(start_ts as i64));
        }
        if let Some(end_ts) = filter.end_ts {
            sql.push_str(" AND req_ts <= ?");
            params_vec.push(SqlValue::Integer(end_ts as i64));
        }

        sql.push_str(" ORDER BY req_ts DESC, log_id DESC LIMIT ? OFFSET ?");
        params_vec.push(SqlValue::Integer(limit as i64));
        params_vec.push(SqlValue::Integer(offset as i64));

        let mut stmt = conn.prepare(&sql).map_err(|error| {
            RPCErrors::ReasonError(format!("Failed to prepare query_logs SQL: {}", error))
        })?;

        let rows = stmt
            .query_map(params_from_iter(params_vec), |row| {
                let req_ts_i64: i64 = row.get(1)?;
                let bytes_sent_i64: i64 = row.get(3)?;
                let status_code_i64: i64 = row.get(4)?;
                Ok(AccessLogEntry {
                    name: row.get(0)?,
                    req_ts: Self::i64_to_u64_saturating(req_ts_i64),
                    source_device_id: row.get(2)?,
                    bytes_sent: Self::i64_to_u64_saturating(bytes_sent_i64),
                    status_code: status_code_i64 as i32,
                    user_agent: row.get(5)?,
                })
            })
            .map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to execute query_logs SQL: {}", error))
            })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|error| {
                RPCErrors::ReasonError(format!("Failed to parse query_logs row: {}", error))
            })?);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndn_lib::ObjId;
    use serde_json::json;

    fn make_test_db_path() -> PathBuf {
        let mut path = std::env::temp_dir();
        let name = format!(
            "share_content_mgr_test_{}.sqlite3",
            ShareContentMgr::now_ms()
        );
        path.push(name);
        path
    }

    #[tokio::test]
    async fn test_publish_resolve_and_toggle() {
        let db_path = make_test_db_path();
        let mgr = ShareContentMgr::new_with_path(&db_path).unwrap();
        let ctx = RPCContext::default();

        let sequence = mgr
            .handle_publish(
                PublishRequest::new(
                    "home/docs/readme.md".to_string(),
                    ObjId::new("chunk:1234").unwrap(),
                    "public".to_string(),
                    Some(json!({"ttl_seconds": 120})),
                    None,
                    Some("dev-a".to_string()),
                ),
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(sequence, 1);

        let resolved = mgr
            .handle_resolve("home/docs/readme.md", ctx.clone())
            .await
            .unwrap();
        assert_eq!(resolved, Some("chunk:1234".to_string()));

        mgr.handle_set_item_enabled(
            "home/docs/readme.md",
            false,
            Some("manual disable"),
            ctx.clone(),
        )
        .await
        .unwrap();

        let resolved_disabled = mgr
            .handle_resolve("home/docs/readme.md", ctx.clone())
            .await
            .unwrap();
        assert!(resolved_disabled.is_none());

        let item = mgr
            .handle_get_item("home/docs/readme.md", ctx.clone())
            .await
            .unwrap()
            .unwrap();
        assert!(!item.enabled);
        assert_eq!(item.disabled_reason, Some("manual disable".to_string()));

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_record_batch_and_stats() {
        let db_path = make_test_db_path();
        let mgr = ShareContentMgr::new_with_path(&db_path).unwrap();
        let ctx = RPCContext::default();

        let logs = vec![
            AccessLogEntry {
                name: "home/docs/readme.md".to_string(),
                req_ts: 1_000,
                source_device_id: Some("dev-a".to_string()),
                bytes_sent: 100,
                status_code: 200,
                user_agent: Some("ua1".to_string()),
            },
            AccessLogEntry {
                name: "home/docs/readme.md".to_string(),
                req_ts: 2_000,
                source_device_id: Some("dev-b".to_string()),
                bytes_sent: 200,
                status_code: 200,
                user_agent: Some("ua2".to_string()),
            },
        ];
        mgr.handle_record_batch(logs, ctx.clone()).await.unwrap();

        let queried = mgr
            .handle_query_logs(
                LogFilter {
                    name: Some("home/docs/readme.md".to_string()),
                    ..Default::default()
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(queried.len(), 2);

        let stats = mgr
            .handle_get_stats("home/docs/readme.md", 0, 10_000, Some(1_000), ctx)
            .await
            .unwrap();
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].request_count, 1);
        assert_eq!(stats[1].request_count, 1);

        let _ = std::fs::remove_file(db_path);
    }
}
