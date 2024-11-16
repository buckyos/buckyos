// 每个Local Chunk Store基于一个目录独立存在
// Chunk Manage由多个Local Chunk Store组成(目前版本先搞定单OOD)
use std::{collections::HashMap, io::SeekFrom};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::{
    fs::{self, File,OpenOptions}, 
    io::{self, AsyncRead, AsyncSeek, AsyncSeekExt}, 
};
use log::*;
use rusqlite::{params, Connection, Result as SqliteResult};
use rusqlite::types::{ToSql, FromSql, ValueRef};
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{ChunkError, ChunkId, ChunkResult, ChunkHasher};

#[derive(Debug, Clone, PartialEq)]
pub enum ChunkState {
    New,//刚创建
    Completed,//完成
    Incompleted,//未完成
    Disabled,//禁用
    NotExist,//不存在
}

impl ChunkState {
    pub fn from_str(s: &str)->Self {
        match s {
            "new" => ChunkState::New,
            "completed" => ChunkState::Completed,
            "incompleted" => ChunkState::Incompleted,
            "disabled" => ChunkState::Disabled,
            "not_exist" => ChunkState::NotExist,
            _ => ChunkState::NotExist,
        }
    }
}

impl ToSql for ChunkState {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let s = match self {
            ChunkState::New => "new",
            ChunkState::Completed => "completed",
            ChunkState::Incompleted => "incompleted",
            ChunkState::Disabled => "disabled",
            ChunkState::NotExist => "not_exist",
        };
        Ok(s.into())   
    }
}

impl FromSql for ChunkState {
    fn column_result(value: ValueRef<'_>) ->  rusqlite::types::FromSqlResult<Self> {
        let s = value.as_str().unwrap();
        Ok(ChunkState::from_str(s))
    }
}

pub struct ChunkItem {
    pub chunk_id: ChunkId,
    pub chunk_size: u64,
    pub chunk_state:ChunkState,
    pub already_write_size: u64,//使用write操作时，已经写入的大小
    pub create_uid: String,
    pub create_appid: String,
    pub description: String,
    pub create_time: u64,
    pub update_time: u64,
}

impl ChunkItem {
    pub fn new(chunk_id: &ChunkId, chunk_size: u64,create_uid: Option<&str>,create_appid: Option<&str>,description: Option<&str>)->Self {
        let now_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
        Self { 
            chunk_id: chunk_id.clone(),
            chunk_size,
            chunk_state: ChunkState::New,
            already_write_size: 0,
            create_uid: create_uid.unwrap_or("").to_string(),
            create_appid: create_appid.unwrap_or("kernel").to_string(),
            description: description.unwrap_or("").to_string(),
            create_time: now_time,
            update_time: now_time,
        }
    }
}

struct ChunkDb {
    db_path: String,
    conn: Mutex<Connection>,
}

impl ChunkDb {
    fn new(db_path: String) -> ChunkResult<Self> {
        let conn = Connection::open(&db_path).map_err(|e| {
            warn!("ChunkDb: open db failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunk_items (
                chunk_id TEXT PRIMARY KEY,
                chunk_size INTEGER NOT NULL,
                chunk_state TEXT NOT NULL,
                already_write_size INTEGER NOT NULL,
                create_uid TEXT NOT NULL,
                create_appid TEXT NOT NULL,
                description TEXT NOT NULL,
                create_time INTEGER NOT NULL,
                update_time INTEGER NOT NULL
            )",
            [],
        ).map_err(|e| {
            warn!("ChunkDb: create table failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunk_links (
                link_chunk_id TEXT PRIMARY KEY,
                target_chunk_id TEXT NOT NULL,
                FOREIGN KEY(target_chunk_id) REFERENCES chunk_items(chunk_id)
            )",
            [],
        ).map_err(|e| {
            warn!("ChunkDb: create table failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        Ok(Self { 
            db_path,
            conn: Mutex::new(conn),
        })
    }

    async fn put_chunk(&self, chunk_item: &ChunkItem) -> ChunkResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO chunk_items 
            (chunk_id, chunk_size, chunk_state, already_write_size, create_uid, 
             create_appid, description, create_time, update_time)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                chunk_item.chunk_id.to_string(),
                chunk_item.chunk_size,
                format!("{:?}", chunk_item.chunk_state),
                chunk_item.already_write_size,
                chunk_item.create_uid,
                chunk_item.create_appid,
                chunk_item.description,
                chunk_item.create_time,
                chunk_item.update_time,
            ],
        ).map_err(|e| {
            warn!("ChunkDb: insert chunk failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    async fn get_chunk(&self, chunk_id: &ChunkId) -> ChunkResult<ChunkItem> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT * FROM chunk_items WHERE chunk_id = ?1"
        ).map_err(|e| {
            warn!("ChunkDb: query chunk failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        let chunk = stmt.query_row(params![chunk_id.to_string()], |row| {
            Ok(ChunkItem {
                chunk_id: chunk_id.clone(),
                chunk_size: row.get(1)?,
                chunk_state: row.get(2)?,
                already_write_size: row.get(3)?,
                create_uid: row.get(4)?,
                create_appid: row.get(5)?,
                description: row.get(6)?,
                create_time: row.get(7)?,
                update_time: row.get(8)?,
            })
        }).map_err(|e| {
            warn!("ChunkDb: query chunk failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        Ok(chunk)
    }

    async fn put_chunk_list(&self, chunk_list: Vec<ChunkItem>) -> ChunkResult<()> {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkDb: transaction failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        for chunk in chunk_list {
            tx.execute(
                "INSERT OR REPLACE INTO chunk_items 
                (chunk_id, chunk_size, chunk_state, already_write_size, create_uid,
                 create_appid, description, create_time, update_time)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    chunk.chunk_id.to_string(),
                    chunk.chunk_size,
                    chunk.chunk_state,
                    chunk.already_write_size,
                    chunk.create_uid,
                    chunk.create_appid,
                    chunk.description,
                    chunk.create_time,
                    chunk.update_time,
                ],
            ).map_err(|e| {
                warn!("ChunkDb: insert chunk failed! {}", e.to_string());
                ChunkError::DbError(e.to_string())
            })?;
        }
        
        tx.commit().map_err(|e| {
            warn!("ChunkDb: commit failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        Ok(())
    }

    async fn remove_chunk(&self, chunk_id: &ChunkId) -> ChunkResult<()> {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkDb: transaction failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        // First remove any links pointing to this chunk
        tx.execute(
            "DELETE FROM chunk_links WHERE target_chunk_id = ?1",
            params![chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkDb: delete link failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        // Then remove the chunk itself
        tx.execute(
            "DELETE FROM chunk_items WHERE chunk_id = ?1",
            params![chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkDb: delete chunk failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        tx.commit().map_err(|e| {
            warn!("ChunkDb: commit failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    async fn remove_chunk_list(&self, chunk_list: Vec<ChunkId>) -> ChunkResult<()> {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkDb: transaction failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        for chunk_id in chunk_list {
            tx.execute(
                "DELETE FROM chunk_links WHERE target_chunk_id = ?1",
                params![chunk_id.to_string()],
            ).map_err(|e| {
                warn!("ChunkDb: delete link failed! {}", e.to_string());
                ChunkError::DbError(e.to_string())
            })?;
            
            tx.execute(
                "DELETE FROM chunk_items WHERE chunk_id = ?1",
                params![chunk_id.to_string()],
            ).map_err(|e| {
                warn!("ChunkDb: delete chunk failed! {}", e.to_string());
                ChunkError::DbError(e.to_string())
            })?;
        }
        
        tx.commit().map_err(|e| {
            warn!("ChunkDb: commit failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    async fn link_chunk(&self, target_chunk_id: &ChunkId, new_chunk_id: &ChunkId) -> ChunkResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO chunk_links (link_chunk_id, target_chunk_id)
            VALUES (?1, ?2)",
            params![new_chunk_id.to_string(), target_chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkDb: link chunk failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    async fn remove_link(&self, link_chunk_id: &ChunkId) -> ChunkResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM chunk_links WHERE link_chunk_id = ?1",
            params![link_chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkDb: remove link failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    async fn get_link_target(&self, chunk_id: &ChunkId) -> ChunkResult<ChunkId> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT target_chunk_id FROM chunk_links WHERE link_chunk_id = ?1"
        ).map_err(|e| {
            warn!("ChunkDb: query link failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        let target_id = stmt.query_row(
            params![chunk_id.to_string()],
            |row| row.get::<_, String>(0)
        ).map_err(|e| {
            warn!("ChunkDb: query link failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        Ok(ChunkId::new(&target_id).unwrap())
    }
}

struct ChunkStore {
    store_id: String,
    store_desc: String,
    chunk_db: ChunkDb,
    base_dir: String,
    enable_symlink: bool,//是否启用符号链接，不同的文件系统对符号链接的支持不一样，默认不启用
    auto_add_to_db: bool,//是否自动将符合命名规范的chunkid添加到db中，默认不自动添加
}


impl ChunkStore {
    fn get_chunk_path(&self, chunk_id: &ChunkId)->String {
        //根据ChunkId的HashResult,产生一个三层的目录结构
        let dir1 = &chunk_id.hash_hex_string[0..3];
        let dir2 = &chunk_id.hash_hex_string[3..6];
        let dir3 = &chunk_id.hash_hex_string[6..9];
        let file_name = &chunk_id.hash_hex_string[6..];
        
        format!("{}/{}/{}/{}/{}.{}",
            self.base_dir,
            dir1,
            dir2,
            dir3,
            file_name,
            chunk_id.hash_type)
    }

    async fn is_real_chunk_exist(&self, chunk_id: &ChunkId)->ChunkResult<bool> {
        let chunk_item = self.chunk_db.get_chunk(chunk_id).await;
        if chunk_item.is_ok() {
            let chunk_item = chunk_item.unwrap();
            if chunk_item.chunk_state == ChunkState::Completed {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn is_chunk_exist(&self, chunk_id: &ChunkId,is_auto_add: Option<bool>)->ChunkResult<bool> {
        let chunk_item = self.chunk_db.get_chunk(chunk_id).await;
        if chunk_item.is_ok() {
            let chunk_item = chunk_item.unwrap();
            if chunk_item.chunk_state == ChunkState::Completed {
                return Ok(true);
            }
        }

        let link_target = self.chunk_db.get_link_target(chunk_id).await;
        if link_target.is_ok() {
            let link_target = link_target.unwrap();
            return self.is_real_chunk_exist(&link_target).await;
        }

        let is_auto_add = is_auto_add.unwrap_or(self.auto_add_to_db);
        if is_auto_add {
            let chunk_path = self.get_chunk_path(chunk_id);
            let file_meta = fs::metadata(&chunk_path).await;
            if file_meta.is_ok() {
                //进行文件校验
                let file_size = file_meta.unwrap().len();
                let mut reader = File::open(&chunk_path).await
                .map_err(|e| {
                    warn!("ChunkStore: open file failed! {}", e.to_string());
                    ChunkError::IoError(e.to_string())
                })?;

                let mut chunk_hasher = ChunkHasher::new(None);
                let hash_bytes = chunk_hasher.calc_from_reader(&mut reader).await?;
                if !chunk_id.is_equal(&hash_bytes) {
                    warn!("auto add chunk failed! chunk_id not equal file content! {} ", chunk_id.to_string());
                    return Ok(false);
                }
                let chunk_item = ChunkItem::new(&chunk_id, file_size, None, None, None);
                self.chunk_db.put_chunk(&chunk_item).await?;
                return Ok(true);
            }
        }

        warn!("chunk not exist! {}", chunk_id.to_string());
        Ok(false)
    }

    pub async fn get_chunk_state(&self, chunk_id: &ChunkId) -> ChunkResult<ChunkState> {
        unimplemented!()
    }
    //查询多个chunk的状态
    pub async fn query_chunk_state_by_list(&self, chunk_list: &mut Vec<ChunkItem>)->ChunkResult<()> {
        unimplemented!()
        
    }
    //一口气写入一组chunk(通常是小chunk)
    pub async fn put_chunklist(&self, chunk_list: HashMap<ChunkId, Vec<u8>>,need_verify: bool)->ChunkResult<()> {
        for (chunk_id, data) in chunk_list {
            self.put_chunk(&chunk_id, &data,need_verify).await?;
        }
        Ok(())
    }
    //写入一个在内存中的完整的chunk
    pub async fn put_chunk(&self, chunk_id: &ChunkId, chunk_data: &[u8],need_verify: bool)->ChunkResult<()> {
        let chunk_path = self.get_chunk_path(&chunk_id);
        
        if need_verify {
            let mut chunk_hasher = ChunkHasher::new(None);
            let hash_bytes = chunk_hasher.calc_from_bytes(&chunk_data);
            if !chunk_id.is_equal(&hash_bytes) {
                warn!("put_chunk: chunk_id not equal hash_bytes! {}",chunk_id.to_string());
                return Err(ChunkError::InvalidId(format!("chunk_id not equal hash_bytes! {}",chunk_id.to_string())));
            }
        }

        // Create parent directories if they don't exist
        if let Some(parent) = std::path::Path::new(&chunk_path).parent() {
            fs::create_dir_all(parent).await
                .map_err(|e| {
                    warn!("put_chunk: create dir failed! {}",e.to_string());
                    ChunkError::IoError(e.to_string())
                })?;
        }

        // Write the chunk data
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&chunk_path)
            .await
            .map_err(|e| {
                warn!("put_chunk: create file failed! {}", e.to_string());
                ChunkError::IoError(e.to_string())
            })?;

        tokio::io::copy(&mut chunk_data.as_ref(), &mut file).await
            .map_err(|e| {
                warn!("put_chunk: write file failed! {}",e.to_string());
                ChunkError::IoError(e.to_string())
            })?;

        // Create and store chunk metadata
        let chunk_item = ChunkItem::new(&chunk_id, chunk_data.len() as u64, None, None, None);
        self.chunk_db.put_chunk(&chunk_item).await?;

        Ok(())
    }

    //使用reader写入一个完整的chunk
    pub async fn put_by_reader<T>(&self, chunk_id: &ChunkId, mut chunk_reader: T)->ChunkResult<()>
        where T: AsyncRead + Unpin + Send + Sync + 'static
    {
        let chunk_path = self.get_chunk_path(&chunk_id);

        // Create parent directories
        if let Some(parent) = std::path::Path::new(&chunk_path).parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                warn!("put_chunk: create dir failed! {}",e.to_string());
                ChunkError::IoError(e.to_string())
            })?;
        }

        // Write the chunk data from reader
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&chunk_path)
            .await
            .map_err(|e| {
                warn!("put_chunk: create file failed! {}", e.to_string());
                ChunkError::IoError(e.to_string())
            })?;
        let bytes_written = tokio::io::copy(&mut chunk_reader, &mut file).await
            .map_err(|e| {
                warn!("put_chunk: write file failed! {}",e.to_string());
                ChunkError::IoError(e.to_string())
            })?;

        // Create and store chunk metadata
        let chunk_item = ChunkItem::new(&chunk_id, bytes_written, None, None, None);
        self.chunk_db.put_chunk(&chunk_item).await?;

        Ok(())
    }

    //写入chunk的一部分数据(可用作部分更新)
    pub async fn write_chunk_part(&self, chunk_id: &ChunkId, offset: SeekFrom, chunk_data: &[u8], is_completed: bool)->ChunkResult<()> {
        let chunk_path = self.get_chunk_path(&chunk_id);

        // Create parent directories if needed
        if let Some(parent) = std::path::Path::new(&chunk_path).parent() {
            fs::create_dir_all(parent).await
                .map_err(|e| {
                    warn!("put_chunk: create dir failed! {}",e.to_string());
                    ChunkError::IoError(e.to_string())
                })?;
        }

        // Write the chunk data from reader
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&chunk_path)
            .await
            .map_err(|e| {
                warn!("put_chunk: create file failed! {}", e.to_string());
                ChunkError::IoError(e.to_string())
            })?;

        // Seek to offset
        file.seek(offset).await
            .map_err(|e| {
                warn!("put_chunk: seek file failed! {}",e.to_string());
                ChunkError::IoError(e.to_string())
            })?;

        // Write data
        tokio::io::copy(&mut chunk_data.as_ref(), &mut file).await
            .map_err(|e| {
                warn!("put_chunk: write file failed! {}",e.to_string());
                ChunkError::IoError(e.to_string())
            })?;

        // Update chunk metadata
        let mut chunk_item = match self.chunk_db.get_chunk(&chunk_id).await {
            Ok(item) => item,
            Err(_) => ChunkItem::new(&chunk_id, 0, None, None, None),
        };

        chunk_item.already_write_size += chunk_data.len() as u64;
        if is_completed {
            chunk_item.chunk_state = ChunkState::Completed;
        }

        self.chunk_db.put_chunk(&chunk_item).await?;

        Ok(())
    }


        
    //path操作的核心是上传diff文件，并说明  chunkid2 = chunkid1 + diff_id, 该操作要成功的前提是local store中存在chunkid1
    //操作成功后，查询chunkid1和chunkid2和diff_id的chunk状态，应该都是exist
    //该函数是否应该上移到chunk_mgr中？
    // pub async fn patch<T>(&self, chunk_id: &str, chunk_reader:  T)->ChunkResult<()>
    //     where T: AsyncRead + Unpin + Send + Sync + 'static
    // {
    //     unimplemented!()
    // }

    //删除chunkid对应的文件（注意一定会带来文件的删除，即使这个chunkid有多个link指向，这些link也会被删除）
    async fn remove(&self, chunk_list: Vec<ChunkId>)->ChunkResult<()> {
        for chunk_id in chunk_list {
            // Remove the physical file
            let chunk_path = self.get_chunk_path(&chunk_id);
            if let Err(e) = fs::remove_file(&chunk_path).await {
                warn!("Failed to remove chunk file {}: {}", chunk_path, e);
            }

            // Remove from database
            self.chunk_db.remove_chunk(&chunk_id).await?;
        }
        Ok(())
    }
    //说明两个chunk id是同一个chunk.实现者可以自己决定是否校验
    //link成功后，查询target_chunk_id和new_chunk_id的状态，应该都是exist
    pub async fn link_chunkid(&self, target_chunk_id: &ChunkId, new_chunk_id: &ChunkId)->ChunkResult<()> {
        // Verify target chunk exists
        if !self.is_real_chunk_exist(&target_chunk_id).await? {
            return Err(ChunkError::ChunkNotFound(format!("target_chunk_id not exist! {}",target_chunk_id.to_string())));
        }

        // Create the link in database
        self.chunk_db.link_chunk(&target_chunk_id, &new_chunk_id).await?;

        // Create symlink if enabled
        if self.enable_symlink {
            let target_path = self.get_chunk_path(&target_chunk_id);
            let new_path = self.get_chunk_path(&new_chunk_id);
            
            if let Some(parent) = std::path::Path::new(&new_path).parent() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    warn!("link_chunkid: create dir failed! {}",e.to_string());
                    ChunkError::IoError(e.to_string())
                })?;
            }
            
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target_path, &new_path)
                .map_err(|e| {
                    warn!("link_chunkid: create symlink failed! {}",e.to_string());
                    ChunkError::IoError(e.to_string())
                })?;
            #[cfg(windows)] 
            std::os::windows::fs::symlink_file(&target_path, &new_path)
                .map_err(|e| {
                    warn!("link_chunkid: create symlink failed! {}",e.to_string());
                    ChunkError::IoError(e.to_string())
                })?;
        }

        Ok(())
    }

    pub async fn remove_chunk_link(&self, chunk_id: &ChunkId)->ChunkResult<()> {
        // Remove symlink if it exists

        let chunk_path = self.get_chunk_path(&chunk_id);
        if let Err(e) = fs::remove_file(&chunk_path).await {
            warn!("Failed to remove symlink {}: {}", chunk_path, e);
        }


        // Remove link from database
        self.chunk_db.remove_link(&chunk_id).await?;
        
        Ok(())
    }
}

