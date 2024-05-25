use std::path::{Path, PathBuf};
use std::os::unix::ffi::OsStringExt;
use backup_lib::{CheckPointVersion, FileId, FileServerType, TaskId, TaskInfo, TaskKey};
use rusqlite::{params, Connection, Result};

pub struct TaskStorageSqlite {
    connection: Connection,
}

impl TaskStorageSqlite {
    pub(crate) fn new_with_path(db_path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
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
                create_at INTEGER DEFAULT STRFTIME('%S', 'NOW'),
                update_at INTEGER DEFAULT STRFTIME('%S', 'NOW'),
                UNIQUE (zone_id, key, version)
            ",
            [],
        )?;

        // Create the "task-files" table
        connection.execute(
            "CREATE TABLE task_files (
                task_id INTEGER NOT NULL,
                file_path BLOB NOT NULL,
                file_hash TEXT NOT NULL,
                create_at INTEGER DEFAULT STRFTIME('%S', 'NOW'),
                FOREIGN KEY (task_id) REFERENCES upload_tasks (task_id),
                FOREIGN KEY (file_hash) REFERENCES files (file_hash),
                PRIMARY KEY (task_id, file_path),
            )",
            [],
        )?;

        // Create the "files" table
        connection.execute(
            "CREATE TABLE files (
                file_hash TEXT NOT NULL PRIMARY KEY,
                file_size INTEGER NOT NULL,
                chunk_size INTEGER DEFAULT NULL,
                file_server_type TEXT DEFAULT NULL,
                file_server_name TEXT DEFAULT NULL,
                remote_file_id INTEGER DEFAULT NULL,
                is_uploaded TINYINT DEFAULT 0,
                create_at INTEGER DEFAULT STRFTIME('%S', 'NOW'),
                finish_at INTEGER DEFAULT NULL,
            )",
            [],
        )?;

        connection.execute(
            "CREATE TABLE strategy (
                zone_id TEXT NOT NULL,
                key TEXT NOT NULL,
                strategy TEXT NOT NULL,
                create_at INTEGER DEFAULT STRFTIME('%S', 'NOW'),
                update_at INTEGER DEFAULT STRFTIME('%S', 'NOW'),
                PRIMARY KEY (zone_id, key)
            )",
            [],
        )?;

        Ok(Self { connection })
    }

    pub fn insert_or_update_strategy(&mut self, zone_id: &str, key: &TaskKey, strategy: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let query = "INSERT INTO strategy (zone_id, key, strategy) VALUES (?, ?, ?)
                     ON CONFLICT (zone_id, key) DO UPDATE SET strategy = excluded.strategy";
        let mut stmt = self.connection.prepare(query)?;
        stmt.execute(params![zone_id, key.as_str(), strategy])?;
        Ok(())
    }

    pub fn query_strategy(&mut self, zone_id: &str, key: &TaskKey) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let query = "SELECT strategy FROM strategy WHERE zone_id = ? AND key = ?";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![zone_id, key.as_str()], |row| {
            Ok(row.get(0)?)
        });
        match result {
            Ok(strategy) => Ok(Some(strategy)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn insert_task(&mut self, zone_id: &str, key: &TaskKey, version: CheckPointVersion, prev_version: Option<CheckPointVersion>, meta: Option<&str>, dir_path: &Path) -> Result<TaskId, Box<dyn std::error::Error + Send + Sync>> {
        let query = "INSERT INTO tasks (zone_id, key, version, prev_version, meta, dir_path) VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT (zone_id, key, version) DO NOTHING
                     RETURNING task_id";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![zone_id, key.as_str(), Into::<u128>::into(version) as u64, prev_version.map(|v| Into::<u128>::into(v) as u64), meta, dir_path.as_os_str().as_encoded_bytes()], |row| {
            Ok(TaskId::from(row.get::<usize, u64>(0)? as u128))
        });
        match result {
            Ok(task_id) => Ok(task_id),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let query = "SELECT task_id FROM tasks WHERE zone_id = ? AND key = ? AND version = ?";
                let mut stmt = self.connection.prepare(query)?;
                let result = stmt.query_row(params![zone_id, key.as_str(), Into::<u128>::into(version) as u64], |row| {
                    Ok(TaskId::from(row.get::<usize, u64>(0)? as u128))
                });
                match result {
                    Ok(task_id) => Ok(task_id),
                    Err(err) => Err(Box::new(err)),
                }
            },
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn query_task_info_without_files(&mut self, task_id: TaskId) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: 查询完成文件数和文件总数
        let query = "SELECT key, version, prev_version, meta, dir_path, is_all_files_ready FROM tasks WHERE task_id = ?";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![Into::<u128>::into(task_id) as u64], |row| {
            Ok(TaskInfo {
                task_id,
                task_key: TaskKey::from(row.get::<usize, String>(0)?),
                check_point_version: CheckPointVersion::from(row.get::<usize, u64>(1)? as u128),
                prev_check_point_version: row.get::<usize, Option<u64>>(2)?.map(|v| CheckPointVersion::from(v as u128)),
                meta: row.get::<usize, Option<String>>(3)?,
                dir_path: PathBuf::from(std::ffi::OsString::from_vec(row.get::<usize, Vec<u8>>(4)?)),
                is_all_files_ready: row.get::<usize, u8>(5)? == 1,
                complete_file_count: 0,
                file_count: 0,
                priority: 0,
                is_manual: false,
                last_fail_at: None,
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
        file_path: &Path,
        file_hash: &str,
        file_size: u64,
    ) -> Result<Option<(FileServerType, String, FileId, u32)>, Box<dyn std::error::Error + Send + Sync>> {
        let tx = self.connection.transaction()?;

        let result = tx.query_row(
            "INSERT INTO files (file_hash, file_size)
             VALUES (?, ?)
             ON CONFLICT (file_hash) DO NOTHING
             RETURNING chunk_size, file_server_type, file_server_name, remote_file_id",
            params![
                file_hash,
                file_size,
            ],
            |row| {
                Ok(None)
            }
        );
        
        // Insert into "task_files" table
        tx.execute(
            "INSERT INTO task_files (task_id, file_path, file_hash)
             VALUES (?, ?, ?)
             ON CONFLICT (task_id, file_path) DO NOTHING",
            params![Into::<u128>::into(task_id) as u64, file_path.as_os_str().as_encoded_bytes(), file_hash],
        )?;
        
        tx.commit()?;
        
        match result {
            Ok(n) => Ok(n),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let query = "SELECT chunk_size, file_server_type, file_server_name, remote_file_id FROM files WHERE file_hash = ?";
                let mut stmt = self.connection.prepare(query)?;
                let result = stmt.query_row(params![file_hash], |row| {
                    Ok((row.get::<usize, Option<u32>>(0)?, row.get::<usize, Option<u32>>(1)?, row.get::<usize, Option<String>>(2)?, row.get::<usize, Option<u64>>(3)?))
                });
                match result {
                    Ok((chunk_size, server_type, server_name, remote_file_id)) => {
                        if let Some(chunk_size) = chunk_size {
                            let server_type = server_type.expect("chunk-size, file-server-type, file-server-name, remote-file-id should all exist");
                            let server_type = FileServerType::try_from(server_type).expect("file-server-type should be valid");
                            let server_name = server_name.expect("chunk-size, file-server-type, file-server-name, remote-file-id should all exist");
                            let remote_file_id = remote_file_id.expect("chunk-size, file-server-type, file-server-name, remote-file-id should all exist");
                            Ok(Some((server_type, server_name, FileId::from(remote_file_id as u128), chunk_size)))
                        } else {
                            Ok(None)
                        }
                    },
                    Err(err) => Ok(None),
                }
            },
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn update_file_info(
        &mut self,
        file_hash: &str,
        file_server_type: FileServerType,
        file_server_name: &str,
        chunk_size: u32,
        remote_file_id: FileId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let query = "UPDATE files SET chunk_size = ?, file_server_type = ?, file_server_name = ?, remote_file_id = ? WHERE file_hash = ?";
        let mut stmt = self.connection.prepare(query)?;
        stmt.execute(params![
            chunk_size,
            Into::<u32>::into(file_server_type),
            file_server_name,
            Into::<u128>::into(remote_file_id) as u64,
            file_hash,
        ])?;
        Ok(())
    }
}
