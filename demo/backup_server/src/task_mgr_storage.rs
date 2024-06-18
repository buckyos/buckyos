use backup_lib::{
    CheckPointVersion, FileId, FileInfo, FileServerType, ListOffset, TaskId, TaskInfo, TaskKey,
};
use rusqlite::{params, Connection, Result};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

pub struct TaskStorageSqlite {
    connection: Connection,
}

impl TaskStorageSqlite {
    pub(crate) fn new_with_path(
        db_path: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("will open sqlite db: {}", db_path);
        let connection = Connection::open(db_path)?;

        // Create the "tasks" table
        connection.execute(
            "CREATE TABLE IF NOT EXISTS tasks (
                task_id INTEGER PRIMARY KEY AUTOINCREMENT,
                zone_id TEXT NOT NULL,
                key TEXT NOT NULL,
                version INTEGER NOT NULL,
                prev_version INTEGER DEFAULT NULL,
                meta TEXT DEFAULT '',
                is_all_files_ready TINYINT DEFAULT 0,
                dir_path BLOB NOT NULL,
                create_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
                update_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
                UNIQUE (zone_id, key, version)
            )",
            [],
        )?;

        // Create the "task-files" table
        connection.execute(
            "CREATE TABLE IF NOT EXISTS task_files (
                task_id INTEGER NOT NULL,
                file_seq INTEGER NOT NULL,
                file_path BLOB NOT NULL,
                file_hash TEXT NOT NULL,
                create_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
                FOREIGN KEY (task_id) REFERENCES tasks (task_id),
                FOREIGN KEY (file_hash) REFERENCES files (file_hash),
                PRIMARY KEY (task_id, file_path)
            )",
            [],
        )?;

        // Create the "files" table
        connection.execute(
            "CREATE TABLE IF NOT EXISTS files (
                file_hash TEXT NOT NULL PRIMARY KEY,
                file_size INTEGER NOT NULL,
                file_server_type INTEGER  NOT NULL,
                file_server_name TEXT  NOT NULL,
                chunk_size INTEGER DEFAULT NULL,
                remote_file_id INTEGER DEFAULT NULL,
                is_uploaded TINYINT DEFAULT 0,
                create_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
                finish_at INTEGER DEFAULT NULL
            )",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS strategy (
                zone_id TEXT NOT NULL,
                key TEXT NOT NULL,
                strategy TEXT NOT NULL,
                create_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
                update_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
                PRIMARY KEY (zone_id, key)
            )",
            [],
        )?;

        Ok(Self { connection })
    }

    pub fn insert_or_update_strategy(
        &mut self,
        zone_id: &str,
        key: &TaskKey,
        strategy: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let query = "INSERT INTO strategy (zone_id, key, strategy) VALUES (?, ?, ?)
                     ON CONFLICT (zone_id, key) DO UPDATE SET strategy = excluded.strategy";
        let mut stmt = self.connection.prepare(query)?;
        stmt.execute(params![zone_id, key.as_str(), strategy])?;
        Ok(())
    }

    pub fn query_strategy(
        &mut self,
        zone_id: &str,
        key: &TaskKey,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let query = "SELECT strategy FROM strategy WHERE zone_id = ? AND key = ?";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![zone_id, key.as_str()], |row| Ok(row.get(0)?));
        match result {
            Ok(strategy) => Ok(Some(strategy)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn insert_task(
        &mut self,
        zone_id: &str,
        key: &TaskKey,
        version: CheckPointVersion,
        prev_version: Option<CheckPointVersion>,
        meta: Option<&str>,
        dir_path: &Path,
    ) -> Result<TaskId, Box<dyn std::error::Error + Send + Sync>> {
        let query = "INSERT INTO tasks (zone_id, key, version, prev_version, meta, dir_path) VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT (zone_id, key, version) DO NOTHING
                     RETURNING task_id";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(
            params![
                zone_id,
                key.as_str(),
                Into::<u128>::into(version) as u64,
                prev_version.map(|v| Into::<u128>::into(v) as u64),
                meta,
                dir_path.as_os_str().as_encoded_bytes()
            ],
            |row| Ok(TaskId::from(row.get::<usize, u64>(0)? as u128)),
        );
        match result {
            Ok(task_id) => Ok(task_id),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let query =
                    "SELECT task_id FROM tasks WHERE zone_id = ? AND key = ? AND version = ?";
                let mut stmt = self.connection.prepare(query)?;
                let result = stmt.query_row(
                    params![zone_id, key.as_str(), Into::<u128>::into(version) as u64],
                    |row| Ok(TaskId::from(row.get::<usize, u64>(0)? as u128)),
                );
                match result {
                    Ok(task_id) => Ok(task_id),
                    Err(err) => Err(Box::new(err)),
                }
            }
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn query_task_info_without_files(
        &mut self,
        task_id: TaskId,
    ) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: 查询完成文件数和文件总数
        let query = "SELECT key, version, prev_version, meta, dir_path, is_all_files_ready, create_at FROM tasks WHERE task_id = ?";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![Into::<u128>::into(task_id) as u64], |row| {
            Ok(TaskInfo {
                task_id,
                task_key: TaskKey::from(row.get::<usize, String>(0)?),
                check_point_version: CheckPointVersion::from(row.get::<usize, u64>(1)? as u128),
                prev_check_point_version: row
                    .get::<usize, Option<u64>>(2)?
                    .map(|v| CheckPointVersion::from(v as u128)),
                meta: row.get::<usize, Option<String>>(3)?,
                dir_path: PathBuf::from(std::ffi::OsString::from_vec(
                    row.get::<usize, Vec<u8>>(4)?,
                )),
                is_all_files_ready: row.get::<usize, u8>(5)? == 1,
                create_time: std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(row.get::<usize, u64>(6)?),
                complete_file_count: 0,
                file_count: 0,
            })
        });
        match result {
            Ok(task_info) => Ok(Some(task_info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn insert_task_file(
        &mut self,
        task_id: TaskId,
        file_seq: u64,
        file_path: &Path,
        file_hash: &str,
        file_size: u64,
        file_server_type: FileServerType,
        file_server_name: &str,
    ) -> Result<
        (FileServerType, String, Option<(FileId, u32)>),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let tx = self.connection.transaction()?;

        let result = tx.query_row(
            "INSERT INTO files (file_hash, file_size, file_server_type, file_server_name)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (file_hash) DO NOTHING
             RETURNING chunk_size, file_server_type, file_server_name, remote_file_id",
            params![
                file_hash,
                file_size,
                Into::<u32>::into(file_server_type),
                file_server_name,
            ],
            |_todo_row| Ok((file_server_type, file_server_name.to_string(), None)),
        );

        // Insert into "task_files" table
        tx.execute(
            "INSERT INTO task_files (task_id, file_path, file_seq, file_hash)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (task_id, file_path) DO NOTHING",
            params![
                Into::<u128>::into(task_id) as u64,
                file_path.as_os_str().as_encoded_bytes(),
                file_seq,
                file_hash
            ],
        )?;

        tx.commit()?;

        match result {
            Ok(n) => Ok(n),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let query = "SELECT chunk_size, file_server_type, file_server_name, remote_file_id FROM files WHERE file_hash = ?";
                let mut stmt = self.connection.prepare(query)?;
                let result = stmt.query_row(params![file_hash], |row| {
                    Ok((
                        row.get::<usize, Option<u32>>(0)?,
                        row.get::<usize, u32>(1)?,
                        row.get::<usize, String>(2)?,
                        row.get::<usize, Option<u64>>(3)?,
                    ))
                });
                match result {
                    Ok((chunk_size, server_type, server_name, remote_file_id)) => {
                        if let Some(chunk_size) = chunk_size {
                            let server_type = FileServerType::try_from(server_type)
                                .expect("file-server-type should be valid");
                            let remote_file_id = remote_file_id.expect("chunk-size, file-server-type, file-server-name, remote-file-id should all exist");
                            Ok((
                                server_type,
                                server_name,
                                Some((FileId::from(remote_file_id as u128), chunk_size)),
                            ))
                        } else {
                            Ok((file_server_type, file_server_name.to_string(), None))
                        }
                    }
                    Err(_todo_err) => Ok((file_server_type, file_server_name.to_string(), None)),
                }
            }
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn update_file_info(
        &mut self,
        file_hash: &str,
        chunk_size: u32,
        remote_file_id: FileId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let query = "UPDATE files SET chunk_size = ?, remote_file_id = ? WHERE file_hash = ?";
        let mut stmt = self.connection.prepare(query)?;
        stmt.execute(params![
            chunk_size,
            Into::<u128>::into(remote_file_id) as u64,
            file_hash,
        ])?;
        Ok(())
    }

    pub fn update_all_files_ready(
        &mut self,
        task_id: TaskId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let query = "UPDATE tasks SET is_all_files_ready = 1 WHERE task_id = ?";
        let mut stmt = self.connection.prepare(query)?;
        stmt.execute(params![Into::<u128>::into(task_id) as u64])?;
        Ok(())
    }

    pub fn update_file_uploaded(
        &mut self,
        task_id: TaskId,
        file_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let query = "UPDATE files SET is_uploaded = 1 WHERE EXISTS (SELECT 1 FROM task_files WHERE task_id = ? AND file_path = ? AND files.file_hash = task_files.file_hash)";
        log::info!(
            "update_file_uploaded: task_id: {:?}, file_path: {:?}, sql: {}",
            task_id,
            file_path,
            query
        );
        let mut stmt = self.connection.prepare(query)?;
        stmt.execute(params![
            Into::<u128>::into(task_id) as u64,
            file_path.as_os_str().as_encoded_bytes()
        ])?;
        Ok(())
    }

    pub fn get_check_point_version_list(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let (ord_sql, offset, limit) = match offset {
            ListOffset::FromFirst(offset) => (
                "ORDER BY tasks.version ASC
                    LIMIT ? OFFSET ?",
                offset,
                limit,
            ),
            ListOffset::FromLast(offset) => (
                "ORDER BY tasks.version DESC
                    LIMIT ? OFFSET ?",
                (std::cmp::max(((offset + 1) as i32) - (limit as i32), 0) as u32),
                std::cmp::min(offset + 1, limit),
            ),
        };

        let sql = if is_restorable_only {
            "SELECT *, 
            (
                SELECT COUNT(*) FROM files, task_files WHERE task_files.task_id = tasks.task_id AND task_files.file_hash = files.file_hash AND files.is_uploaded = 1
            ) AS completed_files,
            (SELECT COUNT(*) FROM task_files WHERE task_files.task_id = tasks.task_id) AS total_files
            FROM tasks
            WHERE zone_id = ? AND key = ? AND is_all_files_ready = 1 
                AND 
                completed_files = total_files"
        } else {
            "SELECT *, 
            (
                SELECT COUNT(*) FROM files, task_files WHERE task_files.task_id = tasks.task_id AND task_files.file_hash = files.file_hash AND files.is_uploaded = 1
            ) AS completed_files,
            (SELECT COUNT(*) FROM task_files WHERE task_files.task_id = tasks.task_id) AS total_files
            FROM tasks
            WHERE zone_id = ? AND key = ?"
        };

        let sql = format!("{} {}", sql, ord_sql);

        log::info!(
            "sql: {}, zone_id: {}, task_key: {}, limit: {}, offset: {}",
            sql,
            zone_id,
            task_key.as_str(),
            limit,
            offset
        );

        let mut stmt = self.connection.prepare(sql.as_str())?;

        let task_infos = stmt
            .query_map(params![zone_id, task_key.as_str(), limit, offset], |row| {
                Ok(TaskInfo {
                    task_id: TaskId::from(row.get::<&str, u64>("task_id")? as u128),
                    task_key: TaskKey::from(row.get::<&str, String>("key")?),
                    check_point_version: CheckPointVersion::from(
                        row.get::<&str, u64>("version")? as u128
                    ),
                    prev_check_point_version: row
                        .get::<&str, Option<u64>>("prev_version")?
                        .map(|v| CheckPointVersion::from(v as u128)),
                    meta: row.get("meta")?,
                    dir_path: std::path::PathBuf::from(std::ffi::OsString::from_vec(
                        row.get::<&str, Vec<u8>>("dir_path")?,
                    )),
                    is_all_files_ready: row.get("is_all_files_ready")?,
                    complete_file_count: row.get("completed_files")?,
                    file_count: row.get("total_files")?,
                    create_time: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(row.get::<&str, u64>("create_at")?),
                })
            })?
            .map(|t| t.unwrap())
            .collect::<Vec<_>>();

        Ok(task_infos)
    }

    pub fn get_check_point_version(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let sql = "SELECT *, 
            (
                SELECT COUNT(*) FROM files, task_files WHERE task_files.task_id = tasks.task_id AND task_files.file_hash = files.file_hash AND files.is_uploaded = 1
            ) AS completed_files,
            (SELECT COUNT(*) FROM task_files WHERE task_files.task_id = tasks.task_id) AS total_files
            FROM tasks
            WHERE zone_id = ? AND key = ? AND version = ?";

        let mut stmt = self.connection.prepare(sql)?;

        let task_info = stmt.query_row(
            params![
                zone_id,
                task_key.as_str(),
                Into::<u128>::into(check_point_version) as u64
            ],
            |row| {
                Ok(TaskInfo {
                    task_id: TaskId::from(row.get::<&str, u64>("task_id")? as u128),
                    task_key: TaskKey::from(row.get::<&str, String>("key")?),
                    check_point_version: CheckPointVersion::from(
                        row.get::<&str, u64>("version")? as u128
                    ),
                    prev_check_point_version: row
                        .get::<&str, Option<u64>>("prev_version")?
                        .map(|v| CheckPointVersion::from(v as u128)),
                    meta: row.get("meta")?,
                    dir_path: std::path::PathBuf::from(std::ffi::OsString::from_vec(
                        row.get::<&str, Vec<u8>>("dir_path")?,
                    )),
                    is_all_files_ready: row.get("is_all_files_ready")?,
                    complete_file_count: row.get("completed_files")?,
                    file_count: row.get("total_files")?,
                    create_time: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(row.get::<&str, u64>("create_at")?),
                })
            },
        );

        match task_info {
            Ok(task_info) => Ok(Some(task_info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn get_file_info(
        &self,
        _todo_zone_id: &str,
        task_id: TaskId,
        file_seq: u64,
    ) -> Result<Option<FileInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let sql = "SELECT file_path, task_files.file_hash, file_size, file_server_type, file_server_name, remote_file_id, chunk_size
                   FROM task_files
                   JOIN files ON task_files.file_hash = files.file_hash
                   WHERE task_files.task_id = ? AND task_files.file_seq = ?";
        let mut stmt = self.connection.prepare(sql)?;
        let file_info = stmt.query_row((Into::<u128>::into(task_id) as u64, file_seq), |row| {
            let file_server_type = row.get::<&str, u32>("file_server_type")?;
            let file_server_name = row.get::<&str, String>("file_server_name")?;
            let remote_file_id = row.get::<&str, Option<u64>>("remote_file_id")?;
            let chunk_size = row.get::<&str, Option<u32>>("chunk_size")?;

            let server_type = FileServerType::try_from(file_server_type).expect("file-server-type should be valid");
            let file_server = match chunk_size {
                Some(chunk_size) => {
                    let remote_file_id = remote_file_id.expect("chunk-size, file-server-type, file-server-name, remote-file-id should all exist");
                    Some((FileId::from(remote_file_id as u128), chunk_size))
                }
                None => None,
            };

            Ok(FileInfo {
                file_path: std::path::PathBuf::from(std::ffi::OsString::from_vec(row.get::<&str, Vec<u8>>("file_path")?)),
                file_size: row.get("file_size")?,
                file_seq,
                task_id,
                hash: row.get("file_hash")?,
                file_server: Some((server_type, file_server_name, file_server)),
            })
        });
        match file_info {
            Ok(file_info) => Ok(Some(file_info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }
}
