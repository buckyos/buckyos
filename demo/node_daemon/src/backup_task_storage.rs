use std::path::Path;

use rusqlite::{Connection, Result};

pub struct BackupTaskInfo {
    pub task_id: i64,
    pub zone_id: String,
    pub key: String,
    pub version: u32,
    pub prev_version: Option<u32>,
    pub meta: String,
    pub dir_path: String,
    pub chunk_count: u32,
    pub is_all_chunks_ready: bool,
    pub is_all_chunks_backup_done: bool,
}

#[derive(Debug, Clone)]
pub struct BackupChunkInfo {
    pub task_id: i64,
    pub chunk_seq: u32,
    pub file_path: String,
    pub hash: String,
    pub chunk_size: u32,
}

pub struct BackupTaskStorage {
    connection: Connection,
}

impl BackupTaskStorage {
    pub fn new_with_path(db_path: &str) -> Result<Self> {
        let connection = Connection::open(db_path)?;
        // Create the upload_tasks table if it doesn't exist
        connection.execute(
            "CREATE TABLE IF NOT EXISTS upload_tasks (
            task_id INTEGER PRIMARY KEY,
            zone_id TEXT NOT NULL,
            key TEXT NOT NULL,
            version INTEGER NOT NULL,
            prev_version INTEGER DEFAULT NULL,
            meta TEXT DEFAULT '',
            is_all_chunks_ready TINYINT DEFAULT 0,
            dir_path TEXT NOT NULL,
            create_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            update_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE (zone_id, key, version)
            )",
            [],
        )?;

        // Create the upload_chunks table if it doesn't exist
        connection.execute(
            "CREATE TABLE IF NOT EXISTS upload_chunks (
            task_id INTEGER NOT NULL,
            chunk_seq INTEGER NOT NULL,
            file_path TEXT NOT NULL,
            hash TEXT NOT NULL,
            chunk_size INTEGER NOT NULL,
            create_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            finish_at DATETIME DEFAULT NULL,
            FOREIGN KEY (task_id) REFERENCES upload_tasks (task_id),
            PRIMARY KEY (task_id, chunk_seq)
            )",
            [],
        )?;
        Ok(Self { connection })
    }

    pub fn insert_upload_task(
        &self,
        zone_id: &str,
        key: &str,
        version: u32,
        prev_version: Option<u32>,
        meta: Option<&str>,
        dir_path: &str,
    ) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO upload_tasks (zone_id, key, version, prev_version, meta, dir_path) VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![zone_id, key, version, prev_version, meta, dir_path],
        )?;

        let task_id = self.connection.last_insert_rowid();
        Ok(task_id)
    }

    pub fn update_upload_task_meta(&self, task_id: i64, meta: &str) -> Result<usize> {
        self.connection.execute(
            "UPDATE upload_tasks SET meta = ? WHERE task_id = ?",
            rusqlite::params![meta, task_id],
        )
    }

    pub fn task_ready(&self, task_id: i64, meta: Option<&str>) -> Result<usize> {
        if let Some(meta) = meta {
            self.connection.execute(
                "UPDATE upload_tasks SET is_all_chunks_ready = 1, meta = ? WHERE task_id = ?",
                rusqlite::params![meta, task_id],
            )
        } else {
            self.connection.execute(
                "UPDATE upload_tasks SET is_all_chunks_ready = 1 WHERE task_id = ?",
                rusqlite::params![task_id],
            )
        }
    }

    pub fn add_upload_chunk(
        &self,
        task_id: i64,
        chunk_seq: u32,
        file_path: &str,
        hash: &str,
        chunk_size: u32,
    ) -> Result<usize> {
        self.connection.execute(
            "INSERT INTO upload_chunks (task_id, chunk_seq, file_path, hash, chunk_size) VALUES (?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![task_id, chunk_seq, file_path, hash, chunk_size],
        )
    }

    pub fn set_finish_time(&self, task_id: i64, chunk_seq: u32) -> Result<usize> {
        self.connection.execute(
            "UPDATE upload_chunks SET finish_at = CURRENT_TIMESTAMP WHERE task_id = ? AND chunk_seq = ?",
            rusqlite::params![task_id, chunk_seq],
        )
    }

    pub fn get_incomplete_tasks(&self) -> Result<Vec<BackupTaskInfo>> {
        let mut stmt = self.connection.prepare(
            "SELECT task_id, zone_id, key, version, prev_version, meta, COUNT(*) as chunk_count, dir_path
            FROM upload_tasks
            LEFT JOIN upload_chunks ON upload_tasks.task_id = upload_chunks.task_id
            WHERE is_all_chunks_ready = 0 AND task_id NOT IN (
                SELECT task_id FROM upload_chunks WHERE finish_at IS NULL
            )
            GROUP BY upload_tasks.task_id",
        )?;

        let task_infos: Result<Vec<BackupTaskInfo>> = stmt
            .query_map([], |row| {
                Ok(BackupTaskInfo {
                    task_id: row.get(0)?,
                    zone_id: row.get(1)?,
                    key: row.get(2)?,
                    version: row.get(3)?,
                    prev_version: row.get(4)?,
                    meta: row.get(5)?,
                    chunk_count: row.get(6)?,
                    dir_path: row.get(7)?,
                    is_all_chunks_ready: false,
                    is_all_chunks_backup_done: false,
                })
            })?
            .collect();
        task_infos
    }

    pub fn get_incomplete_chunks(
        &self,
        task_id: i64,
        limit: usize,
    ) -> Result<Vec<BackupChunkInfo>> {
        // stmt = stmt;
        let mut stmt = self.connection.prepare(
            "SELECT task_id, chunk_seq, file_path, hash, chunk_size
            FROM upload_chunks
            WHERE task_id = ? AND finish_at IS NULL
            ORDER BY chunk_seq ASC
            LIMIT ?",
        )?;

        let chunk_infos: Result<Vec<BackupChunkInfo>> = stmt
            .query_map(rusqlite::params![task_id, limit], |row| {
                Ok(BackupChunkInfo {
                    task_id: row.get(0)?,
                    chunk_seq: row.get(1)?,
                    file_path: row.get(2)?,
                    hash: row.get(3)?,
                    chunk_size: row.get(4)?,
                })
            })?
            .collect();
        chunk_infos
    }
}
