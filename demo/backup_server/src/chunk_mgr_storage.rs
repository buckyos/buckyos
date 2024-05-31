use std::path::{Path, PathBuf};
use std::os::unix::ffi::OsStringExt;
use backup_lib::{CheckPointVersion, ChunkId, ChunkServerType, FileId, FileServerType, TaskId, TaskInfo, TaskKey, TaskServerType};
use rusqlite::{params, Connection, Result};

pub struct ChunkStorageSqlite {
    connection: Connection,
}

impl ChunkStorageSqlite {
    pub(crate) fn new_with_path(db_path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("will open sqlite db: {}", db_path);
        let connection = Connection::open(db_path)?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS file_chunks (
                file_server_type INTEGER NOT NULL,
                file_server_name TEXT NOT NULL,
                chunk_hash TEXT NOT NULL,
                FOREIGN KEY (chunk_hash) REFERENCES chunks (chunk_hash),
                PRIMARY KEY (file_server_type, file_server_name, chunk_hash)
            )",
            [],
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS chunks (
                chunk_id INTEGER PRIMARY KEY AUTOINCREMENT,
                chunk_hash TEXT NOT NULL,
                chunk_size INTEGER NOT NULL,
                save_path BLOB DEFAULT NULL,
                UNIQUE(chunk_hash)
            )",
            [],
        )?;

        Ok(Self { connection })
    }

    pub fn insert_chunk(
        &mut self,
        file_server_type: FileServerType,
        file_server_name: &str,
        chunk_hash: &str,
        chunk_size: u32,
    ) -> Result<ChunkId, Box<dyn std::error::Error + Send + Sync>> {
        let tx = self.connection.transaction()?;

        let result = tx.query_row(
            "INSERT INTO chunks (chunk_hash, chunk_size)
             VALUES (?, ?)
             ON CONFLICT (chunk_hash) DO NOTHING
             RETURNING chunk_id",
            params![
                chunk_hash,
                chunk_size,
            ],
            |row| {
                Ok(ChunkId::from(row.get::<usize, u64>(0)? as u128))
            }
        );
        
        // Insert into "file_chunks" table
        tx.execute(
            "INSERT INTO file_chunks (file_server_type, file_server_name, chunk_hash)
             VALUES (?, ?, ?)
             ON CONFLICT (file_server_type, file_server_name, chunk_hash) DO NOTHING",
            params![
                Into::<u32>::into(file_server_type),
                file_server_name,
                chunk_hash
            ],
        )?;
        
        tx.commit()?;
        
        match result {
            Ok(n) => Ok(n),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let query = "SELECT chunk_id FROM chunks WHERE chunk_hash = ?";
                let mut stmt = self.connection.prepare(query)?;
                let result = stmt.query_row(params![chunk_hash], |row| {
                    Ok(ChunkId::from(row.get::<usize, u64>(0)? as u128))
                });

                match result {
                    Ok(n) => Ok(n),
                    Err(err) => Err(Box::new(err)),
                }
            },
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn update_chunk(
        &mut self,
        chunk_hash: &str,
        save_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.connection.execute(
            "UPDATE chunks SET save_path = ? WHERE chunk_hash = ?",
            params![save_path.as_os_str().as_encoded_bytes(), chunk_hash],
        )?;
        Ok(())
    }

    pub fn query_chunk_by_hash(
        &self,
        chunk_hash: &str,
    ) -> Result<Option<(ChunkId, u32, Option<PathBuf>)>, Box<dyn std::error::Error + Send + Sync>> {
        let query = "SELECT chunk_id, chunk_size, save_path FROM chunks WHERE chunk_hash = ?";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![chunk_hash], |row| {
            Ok((
                ChunkId::from(row.get::<usize, u64>(0)? as u128),
                row.get::<usize, u32>(1)?,
                row.get::<usize, Option<Vec<u8>>>(2)?.map(|v| PathBuf::from(std::ffi::OsString::from_vec(v))),
            ))
        });
        match result {
            Ok(chunk_info) => Ok(Some(chunk_info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }

    pub fn update_chunk_save_path(
        &mut self,
        chunk_hash: &str,
        save_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.connection.execute(
            "UPDATE chunks SET save_path = ? WHERE chunk_hash = ?",
            params![save_path.as_os_str().as_encoded_bytes(), chunk_hash],
        )?;
        Ok(())
    }

    pub fn get_chunk_by_id(
        &self,
        chunk_id: ChunkId,
    ) -> Result<Option<(String, u32, Option<PathBuf>)>, Box<dyn std::error::Error + Send + Sync>> {
        let query = "SELECT chunk_hash, chunk_size, save_path FROM chunks WHERE chunk_id = ?";
        let mut stmt = self.connection.prepare(query)?;
        let result = stmt.query_row(params![Into::<u128>::into(chunk_id) as u64], |row| {
            Ok((
                row.get::<usize, String>(0)?,
                row.get::<usize, u32>(1)?,
                row.get::<usize, Option<Vec<u8>>>(2)?.map(|v| PathBuf::from(std::ffi::OsString::from_vec(v))),
            ))
        });
        match result {
            Ok(chunk_info) => Ok(Some(chunk_info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(Box::new(err)),
        }
    }
}