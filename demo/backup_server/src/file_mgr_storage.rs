use std::path::{Path, PathBuf};
use std::os::unix::ffi::OsStringExt;
use backup_lib::{CheckPointVersion, ChunkId, ChunkInfo, ChunkServerType, FileId, FileServerType, TaskId, TaskInfo, TaskKey, TaskServerType};
use rusqlite::{params, Connection, Result};

pub struct FileStorageSqlite {
    connection: Connection,
}

impl FileStorageSqlite {
    pub(crate) fn new_with_path(db_path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("will open sqlite db: {}", db_path);
        let connection = Connection::open(db_path)?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS files (
                file_id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_server_type INTEGER NOT NULL,
                task_server_name TEXT NOT NULL,
                file_hash TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                chunk_size INTEGER NOT NULL,
                UNIQUE (task_server_type, task_server_name, file_hash)
            )",
            [],
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS file_chunks (
                file_hash TEXT NOT NULL,
                chunk_seq INTEGER NOT NULL,
                chunk_hash TEXT NOT NULL,
                chunk_size INTEGER NOT NULL,
                is_uploaded TINYINT DEFAULT 0,
                FOREIGN KEY (chunk_hash) REFERENCES chunks (chunk_hash),
                PRIMARY KEY (file_hash, chunk_seq)
            )",
            [],
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS chunks (
                chunk_hash TEXT NOT NULL PRIMARY KEY,
                chunk_server_type INTEGER NOT NULL,
                chunk_server_name TEXT NOT NULL,
                remote_chunk_id INTEGER DEFAULT NULL
            )",
            [],
        )?;

        Ok(Self { connection })
    }

    pub(crate) fn insert_file(
        &mut self,
        task_server_type: TaskServerType,
        task_server_name: &str,
        file_hash: &str,
        file_size: u64,
        chunk_size: u32,
    ) -> Result<(FileId, u32), Box<dyn std::error::Error + Send + Sync>>  {
        let result = self.connection.query_row(
            "INSERT INTO files (task_server_type, task_server_name, file_hash, file_size, chunk_size)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (task_server_type, task_server_name, file_hash) DO NOTHING
            RETURNING file_id, chunk_size",
            params![Into::<u32>::into(task_server_type), task_server_name, file_hash, file_size, chunk_size],
            |row| {
                Ok((FileId::from(row.get::<usize, u64>(0)? as u128), row.get::<usize, u32>(1)?))
            });

        match result {
            Ok(ret) => Ok(ret),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let query = "SELECT file_id, chunk_size FROM files WHERE task_server_type = ? AND task_server_name = ? AND file_hash = ?";
                let mut stmt = self.connection.prepare(query)?;
                let result = stmt.query_row(params![Into::<u32>::into(task_server_type), task_server_name, file_hash], |row| {
                    Ok((FileId::from(row.get::<usize, u64>(0)? as u128), row.get::<usize, u32>(1)?))
                });
                match result {
                    Ok(ret) => Ok(ret),
                    Err(err) => Err(Box::new(err)),
                }
            },
            Err(err) => Err(Box::new(err)),
        }
    }

    pub(crate) fn get_file_by_id(
        &mut self,
        file_id: FileId,
    ) -> Result<Option<(TaskServerType, String, String, u64, u32)>, Box<dyn std::error::Error + Send + Sync>> {
        let query = "SELECT task_server_type, task_server_name, file_hash, file_size, chunk_size FROM files WHERE file_id = ?";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![Into::<u128>::into(file_id) as u64], |row| {
            Ok((
                TaskServerType::try_from(row.get::<usize, u32>(0)?).expect("Invalid task_server_type"),
                row.get::<usize, String>(1)?,
                row.get::<usize, String>(2)?,
                row.get::<usize, u64>(3)?,
                row.get::<usize, u32>(4)?,
            ))
        });
        match result {
            Ok(ret) => Ok(Some(ret)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn insert_file_chunk(
        &mut self,
        file_hash: &str,
        chunk_seq: u64,
        chunk_hash: &str,
        chunk_size: u32,
        chunk_server_type: ChunkServerType,
        chunk_server_name: &str,
    ) -> Result<(ChunkServerType, String, Option<ChunkId>), Box<dyn std::error::Error + Send + Sync>> {
        let tx = self.connection.transaction()?;

        let result = tx.query_row(
            "INSERT INTO chunks (chunk_hash, chunk_server_type, chunk_server_name)
             VALUES (?, ?, ?)
             ON CONFLICT (chunk_hash) DO NOTHING
             RETURNING chunk_server_type, chunk_server_name, remote_chunk_id",
            params![
                chunk_hash,
                Into::<u32>::into(chunk_server_type),
                chunk_server_name,
            ],
            |row| {
                Ok((chunk_server_type, chunk_server_name.to_string(), None))
            }
        );
        
        // Insert into "file_chunks" table
        tx.execute(
            "INSERT INTO file_chunks (file_hash, chunk_seq, chunk_hash, chunk_size)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (file_hash, chunk_seq) DO NOTHING",
            params![file_hash, chunk_seq, chunk_hash, chunk_size],
        )?;
        
        tx.commit()?;
        
        match result {
            Ok(n) => Ok(n),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let query = "SELECT chunk_server_type, chunk_server_name, remote_chunk_id FROM chunks WHERE chunk_hash = ?";
                let mut stmt = self.connection.prepare(query)?;
                let result = stmt.query_row(params![file_hash], |row| {
                    Ok((row.get::<usize, u32>(0)?, row.get::<usize, String>(1)?, row.get::<usize, Option<u64>>(2)?))
                });
                match result {
                    Ok((server_type, server_name, remote_chunk_id)) => {
                        let server_type = ChunkServerType::try_from(server_type).expect("chunk-server-type should be valid");
                        if let Some(remote_chunk_id) = remote_chunk_id {
                            Ok((server_type, server_name, Some(ChunkId::from(remote_chunk_id as u128))))
                        } else {
                            Ok((server_type, server_name.to_string(), None))
                        }
                    },
                    Err(err) => Ok((chunk_server_type, chunk_server_name.to_string(), None)),
                }
            },
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn update_chunk(
        &mut self,
        chunk_hash: &str,
        remote_chunk_id: ChunkId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.connection.execute(
            "UPDATE chunks SET remote_chunk_id = ? WHERE chunk_hash = ?",
            params![Into::<u128>::into(remote_chunk_id) as u64, chunk_hash],
        )?;
        Ok(())
    }

    pub fn set_chunk_uploaded(
        &mut self,
        file_id: FileId,
        chunk_seq: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.connection.execute(
            "UPDATE file_chunks SET is_uploaded = 1 WHERE chunk_seq = ? AND EXISTS (SELECT 1 FROM files WHERE files.file_hash = file_chunks.file_hash AND file_id = ?)",
            params![chunk_seq, Into::<u128>::into(file_id) as u64],
        )?;
        Ok(())
    }

    pub fn get_chunk_info(
        &mut self,
        file_id: FileId,
        chunk_seq: u64,
    ) -> Result<Option<ChunkInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let query = "SELECT chunks.chunk_server_type, chunks.chunk_server_name, chunks.remote_chunk_id, file_chunks.chunk_hash, file_chunks.chunk_size
                     FROM chunks
                     INNER JOIN file_chunks ON chunks.chunk_hash = file_chunks.chunk_hash
                     INNER JOIN files ON file_chunks.file_hash = files.file_hash
                     WHERE files.file_id = ? AND file_chunks.chunk_seq = ?";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![Into::<u128>::into(file_id) as u64, chunk_seq], |row| {
            let chunk_server_type = ChunkServerType::try_from(row.get::<usize, u32>(0)?).expect("Invalid chunk_server_type");
            let chunk_server_name = row.get::<usize, String>(1)?;
            let remote_chunk_id = row.get::<usize, Option<u64>>(2)?.map(|v| ChunkId::from(v as u128));
            let chunk_hash = row.get::<usize, String>(3)?;
            let chunk_size = row.get::<usize, u32>(4)?;

            Ok(ChunkInfo {
                hash: chunk_hash,
                chunk_size,
                chunk_server: Some((chunk_server_type, chunk_server_name, remote_chunk_id)),
            })
        });
        match result {
            Ok(ret) => Ok(Some(ret)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }
}