use std::path::Path;
use std::sync::Arc;

use backup_lib::{
    CheckPointVersion, ChunkServerType, ChunkStorage, ChunkStorageQuerier, FileInfo,
    FileServerType, FileStorageQuerier, ListOffset, TaskId, TaskInfo as TaskInfoServer, TaskKey,
    TaskStorageDelete, TaskStorageInStrategy, TaskStorageQuerier, Transaction,
};
use backup_lib::{ChunkId, FileId};
use std::os::unix::ffi::OsStringExt;
use tokio::sync::Mutex;

use crate::backup_task::TaskInfo;
use crate::task_storage::{
    ChunkStorageClient, FileStorageClient, FilesReadyState, TaskStorageClient,
};
use rusqlite::params;
use rusqlite::Connection;
use std::convert::Into; // Add this line to import the Into trait

pub struct TaskStorageSqlite {
    zone_id: String,
    connection: Arc<Mutex<Connection>>,
}

impl TaskStorageSqlite {
    pub(crate) fn new_with_path<P: AsRef<Path>>(
        zone_id: String,
        db_path: P,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let connection = Connection::open(db_path)?;

        // Create the upload_tasks table if it doesn't exist
        connection.execute(
            "CREATE TABLE IF NOT EXISTS upload_tasks (
            task_id INTEGER PRIMARY KEY AUTOINCREMENT,
            zone_id TEXT NOT NULL,
            key TEXT NOT NULL,
            version INTEGER NOT NULL,
            prev_version INTEGER DEFAULT NULL,
            meta TEXT DEFAULT '',
            priority INTEGER NOT NULL,
            is_manual TINYINT NOT NULL,
            remote_task_id INTEGER DEFAULT NULL,
            is_all_files_ready TINYINT DEFAULT 0,
            dir_path BLOB NOT NULL,
            last_fail_at INTEGER DEFAULT NULL,
            create_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
            update_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
            UNIQUE (zone_id, key, version)
            )",
            [],
        )?;

        // Create the upload_chunks table if it doesn't exist
        connection.execute(
            "CREATE TABLE IF NOT EXISTS upload_files (
            task_id INTEGER NOT NULL,
            file_seq INTEGER NOT NULL,
            file_path BLOB NOT NULL,
            file_hash TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            chunk_size INTEGER DEFAULT NULL,
            server_type INTEGER DEFAULT NULL,
            server_name TEXT DEFAULT NULL,
            remote_file_id INTEGER DEFAULT NULL,
            last_fail_at INTEGER DEFAULT NULL,
            create_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
            finish_at INTEGER DEFAULT NULL,
            FOREIGN KEY (task_id) REFERENCES upload_tasks (task_id),
            PRIMARY KEY (task_id, file_seq),
            UNIQUE (task_id, file_path)
            )",
            [],
        )?;

        // Create the upload_chunks table if it doesn't exist
        connection.execute(
            "CREATE TABLE IF NOT EXISTS upload_chunks (
            task_id INTEGER NOT NULL,
            file_seq INTEGER NOT NULL,
            chunk_seq INTEGER NOT NULL,
            chunk_hash TEXT NOT NULL,
            server_type INTEGER DEFAULT NULL,
            server_name TEXT DEFAULT NULL,
            remote_chunk_id INTEGER DEFAULT NULL,
            is_uploaded TINYINT DEFAULT 0,
            last_fail_at INTEGER DEFAULT NULL,
            create_at INTEGER DEFAULT (STRFTIME('%s', 'now')),
            finish_at INTEGER DEFAULT NULL,
            FOREIGN KEY (task_id, file_seq) REFERENCES upload_files (task_id, file_seq),
            PRIMARY KEY (task_id, file_seq, chunk_seq)
            )",
            [],
        )?;

        Ok(Self {
            zone_id,
            connection: Arc::new(Mutex::new(connection)),
        })
    }
}

#[async_trait::async_trait]
impl Transaction for TaskStorageSqlite {
    async fn begin_transaction(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.connection
            .lock()
            .await
            .execute("BEGIN TRANSACTION", [])?;
        Ok(())
    }

    async fn commit_transaction(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.connection.lock().await.execute("COMMIT", [])?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl TaskStorageQuerier for TaskStorageSqlite {
    async fn get_last_check_point_version(
        &self,
        task_key: &TaskKey,
        is_restorable_only: bool,
    ) -> Result<Option<TaskInfoServer>, Box<dyn std::error::Error + Send + Sync>> {
        let task_info =
            TaskStorageClient::get_last_check_point_version(self, task_key, is_restorable_only)
                .await?;
        Ok(task_info.map(|t| TaskInfoServer {
            task_id: t.task_id,
            task_key: t.task_key,
            check_point_version: t.check_point_version,
            prev_check_point_version: t.prev_check_point_version,
            meta: t.meta,
            dir_path: t.dir_path,
            is_all_files_ready: if let FilesReadyState::RemoteReady = t.is_all_files_ready {
                true
            } else {
                false
            },
            complete_file_count: t.complete_file_count,
            file_count: t.file_count,
            create_time: t.create_time,
        }))
    }

    async fn get_check_point_version_list(
        &self,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfoServer>, Box<dyn std::error::Error + Send + Sync>> {
        let task_infos = TaskStorageClient::get_check_point_version_list(
            self,
            task_key,
            offset,
            limit,
            is_restorable_only,
        )
        .await?;
        Ok(task_infos
            .into_iter()
            .map(|t| TaskInfoServer {
                task_id: t.task_id,
                task_key: t.task_key,
                check_point_version: t.check_point_version,
                prev_check_point_version: t.prev_check_point_version,
                meta: t.meta,
                dir_path: t.dir_path,
                is_all_files_ready: if let FilesReadyState::RemoteReady = t.is_all_files_ready {
                    true
                } else {
                    false
                },
                complete_file_count: t.complete_file_count,
                file_count: t.file_count,
                create_time: t.create_time,
            })
            .collect())
    }

    async fn get_check_point_version_list_in_range(
        &self,
        task_key: &TaskKey,
        min_version: Option<CheckPointVersion>,
        max_version: Option<CheckPointVersion>,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfoServer>, Box<dyn std::error::Error + Send + Sync>> {
        let task_infos = TaskStorageClient::get_check_point_version_list_in_range(
            self,
            task_key,
            min_version,
            max_version,
            limit,
            is_restorable_only,
        )
        .await?;
        Ok(task_infos
            .into_iter()
            .map(|t| TaskInfoServer {
                task_id: t.task_id,
                task_key: t.task_key,
                check_point_version: t.check_point_version,
                prev_check_point_version: t.prev_check_point_version,
                meta: t.meta,
                dir_path: t.dir_path,
                is_all_files_ready: if let FilesReadyState::RemoteReady = t.is_all_files_ready {
                    true
                } else {
                    false
                },
                complete_file_count: t.complete_file_count,
                file_count: t.file_count,
                create_time: t.create_time,
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl TaskStorageClient for TaskStorageSqlite {
    async fn create_task(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<&str>,
        dir_path: &Path,
        priority: u32,
        is_manual: bool,
    ) -> Result<TaskId, Box<dyn std::error::Error + Send + Sync>> {
        let connection = self.connection.lock().await;
        let sql = "INSERT INTO upload_tasks (zone_id, key, version, prev_version, meta, dir_path, priority, is_manual) VALUES (?, ?, ?, ?, ?, ?, ?, ?)";
        connection.execute(
            sql,
            params![
                self.zone_id.as_str(),
                task_key.as_str(),
                Into::<u128>::into(check_point_version) as u64,
                prev_check_point_version.map(|v| Into::<u128>::into(v) as u64),
                meta,
                dir_path.as_os_str().as_encoded_bytes(),
                priority,
                is_manual as u8,
            ],
        )?;

        let task_id = connection.last_insert_rowid();
        Ok(TaskId::from(task_id as u128))
    }

    async fn add_file(
        &self,
        task_id: TaskId,
        file_path: &Path,
        hash: &str,
        file_size: u64,
        file_seq: u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let connection = self.connection.lock().await;
        let sql = "INSERT INTO upload_files (task_id, file_seq, file_path, file_hash, file_size) VALUES (?, ?, ?, ?, ?)";
        connection.execute(
            sql,
            params![
                Into::<u128>::into(task_id) as u64,
                file_seq,
                file_path.as_os_str().as_encoded_bytes(),
                hash,
                file_size,
            ],
        )?;

        Ok(())
    }

    async fn set_files_prepare_ready(
        &self,
        task_id: TaskId,
        state: FilesReadyState,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let sql = "UPDATE upload_tasks SET is_all_files_ready = ? WHERE task_id = ? AND zone_id = ? AND is_all_files_ready < ?";
        let connection = self.connection.lock().await;
        connection.execute(
            sql,
            params![
                Into::<u32>::into(state),
                Into::<u128>::into(task_id) as u64,
                self.zone_id.as_str(),
                Into::<u32>::into(state)
            ],
        )?;
        Ok(())
    }

    async fn get_incomplete_tasks(
        &self,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let sql = "SELECT *, 
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id AND 
                (upload_files.chunk_size IS NOT NULL AND upload_files.file_size <= upload_files.chunk_size * (SELECT COUNT(*) FROM upload_chunks WHERE upload_chunks.task_id = upload_files.task_id AND upload_chunks.file_seq = upload_files.file_seq AND upload_chunks.is_uploaded = 1))
            ) AS completed_files,
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id) AS total_files
            FROM upload_tasks
            WHERE zone_id = ? AND 
                (is_all_files_ready <> 2 OR completed_files < total_files)
            ORDER BY version DESC LIMIT ? OFFSET ?";

        let connection = self.connection.lock().await;
        let mut stmt = connection.prepare(sql)?;

        let task_infos = stmt
            .query_map(params![self.zone_id.as_str(), limit, offset], |row| {
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
                    is_all_files_ready: FilesReadyState::try_from(
                        row.get::<&str, u32>("is_all_files_ready")?,
                    )
                    .expect("is_all_files_ready should be valid"),
                    complete_file_count: row.get("completed_files")?,
                    file_count: row.get("total_files")?,
                    priority: row.get("priority")?,
                    is_manual: row.get::<&str, u8>("is_manual")? == 1,
                    last_fail_at: row
                        .get::<&str, Option<u64>>("last_fail_at")?
                        .map(|t| std::time::UNIX_EPOCH + std::time::Duration::from_secs(t as u64)),
                    create_time: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(row.get::<&str, u64>("create_at")?),
                })
            })?
            .map(|t| t.unwrap())
            .collect::<Vec<_>>();

        Ok(task_infos)
    }

    async fn get_incomplete_files(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        min_file_seq: usize,
        limit: usize,
    ) -> Result<Vec<FileInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let sql = "SELECT task_id, file_path, file_hash, file_size, file_seq
            FROM upload_files
            WHERE file_seq >= ? AND task_id IN (SELECT task_id FROM upload_tasks WHERE key = ? AND version = ? AND zone_id = ?) AND
                (
                    chunk_size IS NULL OR
                    file_size > chunk_size * (SELECT COUNT(*) FROM upload_chunks WHERE upload_chunks.task_id = upload_files.task_id AND upload_chunks.file_seq = upload_files.file_seq AND upload_chunks.is_uploaded = 1)
                )
            ORDER BY file_seq ASC
            LIMIT ?";
        let connection = self.connection.lock().await;
        let mut stmt = connection.prepare(sql)?;
        let file_infos = stmt
            .query_map(
                params![
                    min_file_seq,
                    task_key.as_str(),
                    Into::<u128>::into(version) as u64,
                    self.zone_id.as_str(),
                    limit
                ],
                |row| {
                    Ok(FileInfo {
                        task_id: TaskId::from(row.get::<&str, u64>("task_id")? as u128),
                        file_path: std::path::PathBuf::from(std::ffi::OsString::from_vec(
                            row.get::<&str, Vec<u8>>("file_path")?,
                        )),
                        hash: row.get("file_hash")?,
                        file_size: row.get("file_size")?,
                        file_seq: row.get("file_seq")?,
                        file_server: None,
                    })
                },
            )?
            .map(|f| f.unwrap())
            .collect::<Vec<_>>();
        Ok(file_infos)
    }

    async fn is_task_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<Option<TaskId>, Box<dyn std::error::Error + Send + Sync>> {
        let sql =
            "SELECT remote_task_id FROM upload_tasks WHERE key = ? AND version = ? AND zone_id = ?";
        let connection = self.connection.lock().await;
        let mut stmt = connection.prepare(sql)?;
        let remote_task_id: Option<u64> = stmt.query_row(
            params![
                task_key.as_str(),
                Into::<u128>::into(check_point_version) as u64,
                self.zone_id.as_str()
            ],
            |row| row.get(0),
        )?;
        Ok(remote_task_id.map(|id| TaskId::from(id as u128)))
    }

    async fn set_task_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        remote_task_id: TaskId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let sql = "UPDATE upload_tasks SET remote_task_id = ? WHERE key = ? AND version = ? AND zone_id = ?";
        let connection = self.connection.lock().await;
        connection.execute(
            sql,
            params![
                Into::<u128>::into(remote_task_id) as u64,
                task_key.as_str(),
                Into::<u128>::into(check_point_version) as u64,
                self.zone_id.as_str()
            ],
        )?;
        Ok(())
    }

    async fn is_file_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_path: &Path,
    ) -> Result<
        Option<(FileServerType, String, FileId, u32)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // chunk_size INTEGER DEFAULT NULL,
        // server_type TEXT DEFAULT NULL,
        // server_name TEXT DEFAULT NULL,
        let sql = "SELECT upload_files.chunk_size, upload_files.server_type, upload_files.server_name, upload_files.remote_file_id
            FROM upload_files, upload_tasks 
            WHERE upload_tasks.task_id = upload_files.task_id AND zone_id = ? AND
                key = ? AND version = ? AND file_path = ?";

        let connection = self.connection.lock().await;

        let mut stmt = connection.prepare(sql)?;
        let (chunk_size, server_type, server_name, remote_file_id) = stmt.query_row(
            params![
                self.zone_id.as_str(),
                task_key.as_str(),
                Into::<u128>::into(check_point_version) as u64,
                file_path.as_os_str().as_encoded_bytes()
            ],
            |row| {
                Ok((
                    row.get::<usize, Option<u32>>(0)?,
                    row.get::<usize, Option<u32>>(1)?,
                    row.get::<usize, Option<String>>(2)?,
                    row.get::<usize, Option<u64>>(3)?,
                ))
            },
        )?;

        Ok(match chunk_size {
            Some(chunk_size) => {
                let server_type = server_type.expect("chunk-size, file-server-type, file-server-name, remote-file-id should all exist");
                let server_type = FileServerType::try_from(server_type)
                    .expect("file-server-type should be valid");
                let server_name = server_name.expect("chunk-size, file-server-type, file-server-name, remote-file-id should all exist");
                let remote_file_id = remote_file_id.expect("chunk-size, file-server-type, file-server-name, remote-file-id should all exist");
                Some((
                    server_type,
                    server_name,
                    FileId::from(remote_file_id as u128),
                    chunk_size,
                ))
            }
            None => None,
        })
    }

    async fn set_file_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_path: &Path,
        server_type: FileServerType,
        server_name: &str,
        remote_file_id: FileId,
        chunk_size: u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let sql = "UPDATE upload_files SET chunk_size = ?, server_type = ?, server_name = ?, remote_file_id = ? WHERE task_id IN (SELECT task_id FROM upload_tasks WHERE key = ? AND version = ? AND zone_id = ?) AND file_path = ?";

        let connection = self.connection.lock().await;
        connection.execute(
            sql,
            params![
                chunk_size,
                Into::<u32>::into(server_type),
                server_name,
                Into::<u128>::into(remote_file_id) as u64,
                task_key.as_str(),
                Into::<u128>::into(check_point_version) as u64,
                self.zone_id.as_str(),
                file_path.as_os_str().as_encoded_bytes()
            ],
        )?;

        Ok(())
    }
    async fn get_last_check_point_version(
        &self,
        task_key: &TaskKey,
        is_restorable_only: bool,
    ) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        // is_restorable_only = true 有几个条件:
        //     1. is_all_files_ready = 2
        //     2. 每个相关文件的所有chunk都已经上传:
        //         - 文件关联所有chunk的is_uploaded都为true
        //         - 文件关联chunk数 * 文件chunk大小(chunk_size) >= 文件大小(file_size)

        let sql = if is_restorable_only {
            "SELECT *, 
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id AND 
                (upload_files.chunk_size IS NOT NULL AND upload_files.file_size <= upload_files.chunk_size * (SELECT COUNT(*) FROM upload_chunks WHERE upload_chunks.task_id = upload_files.task_id AND upload_chunks.file_seq = upload_files.file_seq AND upload_chunks.is_uploaded = 1))
            ) AS completed_files,
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id) AS total_files
            FROM upload_tasks
            WHERE zone_id = ? AND key = ? AND is_all_files_ready = 2 
                AND 
                completed_files = total_files
            ORDER BY version DESC LIMIT 1"
        } else {
            "SELECT *, 
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id AND 
                (upload_files.chunk_size IS NOT NULL AND upload_files.file_size <= upload_files.chunk_size * (SELECT COUNT(*) FROM upload_chunks WHERE upload_chunks.task_id = upload_files.task_id AND upload_chunks.file_seq = upload_files.file_seq AND upload_chunks.is_uploaded = 1))
            ) AS completed_files,
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id) AS total_files
            FROM upload_tasks
            WHERE zone_id = ? AND key = ?
            ORDER BY version DESC LIMIT 1"
        };

        let connection = self.connection.lock().await;
        let mut stmt = connection.prepare(sql)?;
        let mut task_infos = stmt
            .query_map(params![self.zone_id.as_str(), task_key.as_str()], |row| {
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
                    is_all_files_ready: FilesReadyState::try_from(
                        row.get::<&str, u32>("is_all_files_ready")?,
                    )
                    .expect("is_all_files_ready should be valid"),
                    complete_file_count: row.get("completed_files")?,
                    file_count: row.get("total_files")?,
                    priority: row.get("priority")?,
                    is_manual: row.get::<&str, u8>("is_manual")? == 1,
                    last_fail_at: row
                        .get::<&str, Option<u64>>("last_fail_at")?
                        .map(|t| std::time::UNIX_EPOCH + std::time::Duration::from_secs(t as u64)),
                    create_time: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(row.get::<&str, u64>("create_at")?),
                })
            })?
            .map(|t| t.unwrap())
            .collect::<Vec<_>>();

        if task_infos.len() > 0 {
            Ok(Some(task_infos.remove(0)))
        } else {
            Ok(None)
        }
    }

    async fn get_check_point_version_list(
        &self,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let (ord_sql, offset, limit) = match offset {
            ListOffset::FromFirst(offset) => (
                "ORDER BY upload_tasks.version ASC
                    LIMIT ? OFFSET ?",
                offset,
                limit,
            ),
            ListOffset::FromLast(offset) => (
                "ORDER BY upload_tasks.version DESC
                    LIMIT ? OFFSET ?",
                (std::cmp::max((offset as i32) - (limit as i32), 0) as u32),
                std::cmp::min(offset, limit),
            ),
        };

        let sql = if is_restorable_only {
            "SELECT *, 
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id AND 
                (upload_files.chunk_size IS NOT NULL AND upload_files.file_size <= upload_files.chunk_size * (SELECT COUNT(*) FROM upload_chunks WHERE upload_chunks.task_id = upload_files.task_id AND upload_chunks.file_seq = upload_files.file_seq AND upload_chunks.is_uploaded = 1))
            ) AS completed_files,
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id) AS total_files
            FROM upload_tasks
            WHERE zone_id = ? AND key = ? AND is_all_files_ready = 2 
                AND 
                completed_files = total_files"
        } else {
            "SELECT *, 
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id AND 
                (upload_files.chunk_size IS NOT NULL AND upload_files.file_size <= upload_files.chunk_size * (SELECT COUNT(*) FROM upload_chunks WHERE upload_chunks.task_id = upload_files.task_id AND upload_chunks.file_seq = upload_files.file_seq AND upload_chunks.is_uploaded = 1))
            ) AS completed_files,
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id) AS total_files
            FROM upload_tasks
            WHERE zone_id = ? AND key = ?"
        };

        let sql = format!("{} {}", sql, ord_sql);
        let connection = self.connection.lock().await;
        let mut stmt = connection.prepare(sql.as_str())?;

        let task_infos = stmt
            .query_map(
                params![self.zone_id.as_str(), task_key.as_str(), limit, offset],
                |row| {
                    Ok(TaskInfo {
                        task_id: TaskId::from(row.get::<&str, u64>("task_id")? as u128),
                        task_key: TaskKey::from(row.get::<&str, String>("key")?),
                        check_point_version: CheckPointVersion::from(
                            row.get::<&str, u64>("version")? as u128,
                        ),
                        prev_check_point_version: row
                            .get::<&str, Option<u64>>("prev_version")?
                            .map(|v| CheckPointVersion::from(v as u128)),
                        meta: row.get("meta")?,
                        dir_path: std::path::PathBuf::from(std::ffi::OsString::from_vec(
                            row.get::<&str, Vec<u8>>("dir_path")?,
                        )),
                        is_all_files_ready: FilesReadyState::try_from(
                            row.get::<&str, u32>("is_all_files_ready")?,
                        )
                        .expect("is_all_files_ready should be valid"),
                        complete_file_count: row.get("completed_files")?,
                        file_count: row.get("total_files")?,
                        priority: row.get("priority")?,
                        is_manual: row.get::<&str, u8>("is_manual")? == 1,
                        last_fail_at: row.get::<&str, Option<u64>>("last_fail_at")?.map(|t| {
                            std::time::UNIX_EPOCH + std::time::Duration::from_secs(t as u64)
                        }),
                        create_time: std::time::UNIX_EPOCH
                            + std::time::Duration::from_secs(row.get::<&str, u64>("create_at")?),
                    })
                },
            )?
            .map(|t| t.unwrap())
            .collect::<Vec<_>>();

        Ok(task_infos)
    }

    async fn get_check_point_version_list_in_range(
        &self,
        task_key: &TaskKey,
        min_version: Option<CheckPointVersion>,
        max_version: Option<CheckPointVersion>,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let min_version = min_version
            .map(|v| Into::<u128>::into(v) as u64)
            .unwrap_or(0);
        let max_version = max_version
            .map(|v| Into::<u128>::into(v) as u64)
            .unwrap_or(std::u64::MAX);

        let sql = if is_restorable_only {
            "SELECT *, 
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id AND 
                (upload_files.chunk_size IS NOT NULL AND upload_files.file_size <= upload_files.chunk_size * (SELECT COUNT(*) FROM upload_chunks WHERE upload_chunks.task_id = upload_files.task_id AND upload_chunks.file_seq = upload_files.file_seq AND upload_chunks.is_uploaded = 1))
            ) AS completed_files,
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id) AS total_files
            FROM upload_tasks
            WHERE zone_id = ? AND key = ? AND version >=? AND version <=? AND is_all_files_ready = 2 
                AND 
                completed_files = total_files
            ORDER BY version DESC LIMIT ?"
        } else {
            "SELECT *, 
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id AND 
                (upload_files.chunk_size IS NOT NULL AND upload_files.file_size <= upload_files.chunk_size * (SELECT COUNT(*) FROM upload_chunks WHERE upload_chunks.task_id = upload_files.task_id AND upload_chunks.file_seq = upload_files.file_seq AND upload_chunks.is_uploaded = 1))
            ) AS completed_files,
            (SELECT COUNT(*) FROM upload_files WHERE upload_files.task_id = upload_tasks.task_id) AS total_files
            FROM upload_tasks
            WHERE zone_id = ? AND key = ? AND version >=? AND version <=?
            ORDER BY version DESC LIMIT ?"
        };

        let connection = self.connection.lock().await;
        let mut stmt = connection.prepare(sql)?;

        let task_infos = stmt
            .query_map(
                params![
                    self.zone_id.as_str(),
                    task_key.as_str(),
                    min_version,
                    max_version,
                    limit
                ],
                |row| {
                    Ok(TaskInfo {
                        task_id: TaskId::from(row.get::<&str, u64>("task_id")? as u128),
                        task_key: TaskKey::from(row.get::<&str, String>("key")?),
                        check_point_version: CheckPointVersion::from(
                            row.get::<&str, u64>("version")? as u128,
                        ),
                        prev_check_point_version: row
                            .get::<&str, Option<u64>>("prev_version")?
                            .map(|v| CheckPointVersion::from(v as u128)),
                        meta: row.get("meta")?,
                        dir_path: std::path::PathBuf::from(std::ffi::OsString::from_vec(
                            row.get::<&str, Vec<u8>>("dir_path")?,
                        )),
                        is_all_files_ready: FilesReadyState::try_from(
                            row.get::<&str, u32>("is_all_files_ready")?,
                        )
                        .expect("is_all_files_ready should be valid"),
                        complete_file_count: row.get("completed_files")?,
                        file_count: row.get("total_files")?,
                        priority: row.get("priority")?,
                        is_manual: row.get::<&str, u8>("is_manual")? == 1,
                        last_fail_at: row.get::<&str, Option<u64>>("last_fail_at")?.map(|t| {
                            std::time::UNIX_EPOCH + std::time::Duration::from_secs(t as u64)
                        }),
                        create_time: std::time::UNIX_EPOCH
                            + std::time::Duration::from_secs(row.get::<&str, u64>("create_at")?),
                    })
                },
            )?
            .map(|t| t.unwrap())
            .collect::<Vec<_>>();

        Ok(task_infos)
    }

    async fn set_task_last_try_fail_time(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let current_time = std::time::SystemTime::now();
        let sql = "UPDATE upload_tasks SET last_fail_at = ? WHERE key = ? AND version = ? AND zone_id = ?";
        let connection = self.connection.lock().await;
        let execute = connection.execute(
            sql,
            params![
                current_time
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as u64,
                task_key.as_str(),
                Into::<u128>::into(version) as u64,
                self.zone_id
            ],
        )?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl TaskStorageDelete for TaskStorageSqlite {
    async fn delete_tasks_by_id(
        &self,
        _todo_task_id: &[TaskId],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // unimplemented!()
        Ok(())
    }
}

#[async_trait::async_trait]
impl TaskStorageInStrategy for TaskStorageSqlite {
    // Implement the methods of TaskStorageInStrategy trait here
}

#[async_trait::async_trait]
impl FileStorageQuerier for TaskStorageSqlite {}

#[async_trait::async_trait]
impl FileStorageClient for TaskStorageSqlite {
    // Ok((chunk-server-type, chunk-server-name, chunk-hash))
    async fn is_chunk_info_pushed(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        file_path: &Path,
        chunk_seq: u64,
    ) -> Result<
        Option<(ChunkServerType, String, String, ChunkId)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let sql = "SELECT upload_chunks.server_type, upload_chunks.server_name, upload_chunks.chunk_hash, upload_chunks.remote_chunk_id
            FROM upload_chunks, upload_files, upload_tasks
            WHERE upload_tasks.task_id = upload_files.task_id AND upload_tasks.task_id = upload_chunks.task_id
                AND upload_files.file_seq = upload_chunks.file_seq
                AND upload_tasks.zone_id = ?
                AND upload_tasks.key = ?
                AND upload_tasks.version = ?
                AND upload_files.file_path = ?
                AND upload_chunks.chunk_seq = ?";
        let connection = self.connection.lock().await;
        let mut stmt = connection.prepare(sql)?;
        let mut rows = stmt
            .query_map(
                params![
                    self.zone_id.as_str(),
                    task_key.as_str(),
                    Into::<u128>::into(version) as u64,
                    file_path.as_os_str().as_encoded_bytes(),
                    chunk_seq
                ],
                |row| {
                    Ok((
                        row.get::<usize, Option<u32>>(0)?,
                        row.get::<usize, Option<String>>(1)?,
                        row.get::<usize, Option<String>>(2)?,
                        row.get::<usize, Option<u64>>(3)?,
                    ))
                },
            )?
            .collect::<Vec<_>>();

        if rows.len() == 0 {
            return Ok(None);
        }

        let (server_type, server_name, chunk_hash, remote_chunk_id) = rows.remove(0)?;

        Ok(match chunk_hash {
            Some(chunk_hash) => {
                let server_type = server_type.expect("chunk-size, file-server-type, file-server-name, remote-chunk-id should all exist");
                let server_type = ChunkServerType::try_from(server_type)
                    .expect("file-server-type should be valid");
                let server_name = server_name.expect("chunk-size, file-server-type, file-server-name, remote-chunk-id should all exist");
                let remote_chunk_id = remote_chunk_id.expect("chunk-size, file-server-type, file-server-name, remote-chunk-id should all exist");
                Some((
                    server_type,
                    server_name,
                    chunk_hash,
                    ChunkId::from(remote_chunk_id as u128),
                ))
            }
            None => None,
        })
    }

    async fn set_chunk_info_pushed(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        file_path: &Path,
        chunk_seq: u64,
        chunk_server_type: ChunkServerType,
        server_name: &str,
        chunk_hash: &str,
        remote_chunk_id: ChunkId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let sql = "INSERT INTO upload_chunks (task_id, file_seq, chunk_seq, server_type, server_name, chunk_hash, remote_chunk_id)
            SELECT upload_files.task_id, upload_files.file_seq, ?, ?, ?, ?, ?
            FROM upload_files
            JOIN upload_tasks ON upload_files.task_id = upload_tasks.task_id
            WHERE upload_files.file_path = ? 
            AND upload_tasks.key = ? 
            AND upload_tasks.version = ? 
            AND upload_tasks.zone_id = ?
            ON CONFLICT (task_id, file_seq, chunk_seq) DO UPDATE
            SET server_type = ?, server_name = ?, chunk_hash = ?, remote_chunk_id = ?";

        let connection = self.connection.lock().await;
        connection.execute(
            sql,
            params![
                chunk_seq,
                Into::<u32>::into(chunk_server_type),
                server_name,
                chunk_hash,
                Into::<u128>::into(remote_chunk_id) as u64,
                file_path.as_os_str().as_encoded_bytes(),
                task_key.as_str(),
                Into::<u128>::into(version) as u64,
                self.zone_id.as_str(),
                Into::<u32>::into(chunk_server_type),
                server_name,
                chunk_hash,
                Into::<u128>::into(remote_chunk_id) as u64,
            ],
        )?;
        // let sql = "UPDATE upload_chunks
        //     SET server_type = ?, server_name = ?, chunk_hash = ?, remote_chunk_id = ?
        //     WHERE chunk_seq = ?
        //         AND EXISTS (
        //             SELECT 1
        //             FROM upload_files
        //             JOIN upload_tasks ON upload_files.task_id = upload_tasks.task_id
        //             WHERE upload_files.file_path = ?
        //             AND upload_tasks.key = ?
        //             AND upload_tasks.version = ?
        //             AND upload_tasks.zone_id = ?
        //             AND upload_chunks.task_id = upload_files.task_id
        //             AND upload_chunks.file_seq = upload_files.file_seq
        //         )
        //     ";

        // connection.execute(sql, params![Into::<u32>::into(chunk_server_type), server_name, chunk_hash, Into::<u128>::into(remote_chunk_id) as u64, chunk_seq, file_path.as_os_str().as_encoded_bytes(), task_key.as_str(), Into::<u128>::into(version) as u64, self.zone_id.as_str()])?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl ChunkStorageQuerier for TaskStorageSqlite {}

impl ChunkStorage for TaskStorageSqlite {}

#[async_trait::async_trait]
impl ChunkStorageClient for TaskStorageSqlite {
    // Ok(is_uploaded)
    async fn is_chunk_uploaded(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        file_path: &Path,
        chunk_seq: u64,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let sql = "SELECT is_uploaded
            FROM upload_chunks
            WHERE chunk_seq = ?
                AND EXISTS (
                    SELECT 1
                    FROM upload_files
                    JOIN upload_tasks ON upload_files.task_id = upload_tasks.task_id
                    WHERE upload_files.file_path = ? 
                    AND upload_tasks.key = ? 
                    AND upload_tasks.version = ? 
                    AND upload_tasks.zone_id = ?
                    AND upload_chunks.task_id = upload_files.task_id
                    AND upload_chunks.file_seq = upload_files.file_seq
                )
            ";
        let connection = self.connection.lock().await;
        let mut stmt = connection.prepare(sql)?;
        let is_uploaded: bool = stmt.query_row(
            params![
                chunk_seq,
                file_path.as_os_str().as_encoded_bytes(),
                task_key.as_str(),
                Into::<u128>::into(version) as u64,
                self.zone_id.as_str()
            ],
            |row| Ok(row.get::<usize, u8>(0)? == 1),
        )?;
        Ok(is_uploaded)
    }

    async fn set_chunk_uploaded(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        file_path: &Path,
        chunk_seq: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let sql = "UPDATE upload_chunks
                    SET is_uploaded = 1
                    WHERE chunk_seq = ?
                    AND EXISTS (
                        SELECT 1
                        FROM upload_files
                        JOIN upload_tasks ON upload_files.task_id = upload_tasks.task_id
                        WHERE upload_files.file_path = ? 
                        AND upload_tasks.key = ? 
                        AND upload_tasks.version = ? 
                        AND upload_tasks.zone_id = ?
                        AND upload_chunks.task_id = upload_files.task_id
                        AND upload_chunks.file_seq = upload_files.file_seq
                    )
                ";
        let connection = self.connection.lock().await;
        connection.execute(
            sql,
            params![
                chunk_seq,
                file_path.as_os_str().as_encoded_bytes(),
                task_key.as_str(),
                Into::<u128>::into(version) as u64,
                self.zone_id.as_str()
            ],
        )?;

        Ok(())
    }
}
