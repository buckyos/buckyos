
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_root_dir};
use serde::{Serialize,Deserialize};
//chunk_mgr默认是机器级别的，即多个进程可以共享同一个chunk_mgr
//chunk_mgr使用共享内存/filemap等技术来实现跨进程的读数据共享，比使用127.0.0.1的http协议更高效
//从实现简单的角度考虑，先用http协议实现写数据？
use tokio::{
    fs::{self, File,OpenOptions}, 
    io::{self, AsyncRead,AsyncWrite, AsyncReadExt, AsyncWriteExt, AsyncSeek, AsyncSeekExt}, 
};
use log::*;
use crate::{ChunkStore,ChunkId,ChunkResult,ChunkReadSeek,ChunkError};
use memmap::Mmap;
use std::{path::PathBuf, pin::Pin};
use std::sync::Mutex;
use std::sync::Arc;
use std::collections::HashMap;
use rusqlite::{Connection};
use lazy_static::lazy_static;

use buckyos_kit::get_buckyos_chunk_data_dir;

pub struct ChunkMgrDB {
    db_path: String,
    conn: Mutex<Connection>,
}

impl ChunkMgrDB {
    pub fn new(db_path: String) -> ChunkResult<Self> {
        let conn = Connection::open(&db_path).map_err(|e| {
            warn!("ChunkMgrDB: open db failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunks (
                chunk_id TEXT PRIMARY KEY,
                ref_count INTEGER NOT NULL DEFAULT 0,
                access_time INTEGER NOT NULL,
                size INTEGER NOT NULL DEFAULT 0
            )",
            [],
        ).map_err(|e| {
            warn!("ChunkMgrDB: create chunks table failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS paths (
                path TEXT PRIMARY KEY,
                chunk_id TEXT NOT NULL,
                app_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                FOREIGN KEY(chunk_id) REFERENCES chunks(chunk_id)
            )",
            [],
        ).map_err(|e| {
            warn!("ChunkMgrDB: create paths table failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        Ok(Self {
            db_path,
            conn: Mutex::new(conn),
        })
    }

    pub fn get_path_target_chunk(&self, path: &str)->ChunkResult<ChunkId> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT chunk_id FROM paths WHERE path = ?1").map_err(|e| {
            warn!("ChunkMgrDB: prepare statement failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        let chunk_id: String = stmt.query_row([path], |row| row.get(0)).map_err(|e| {
            warn!("ChunkMgrDB: query path target chunk failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        ChunkId::new(&chunk_id).map_err(|e| {
            warn!("ChunkMgrDB: invalid chunk_id format! {}", e.to_string());
            ChunkError::Internal(e.to_string())
        })
    }

    pub fn update_chunk_access_time(&self, chunk_id: &ChunkId, access_time: u64) -> ChunkResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE chunks SET access_time = ?1 WHERE chunk_id = ?2",
            [&access_time.to_string(), &chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkMgrDB: update chunk access time failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn create_path(&self, chunk_id: &ChunkId, path: String,app_id:&str,user_id:&str) -> ChunkResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkMgrDB: create path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        // Insert or update the chunk entry
        tx.execute(
            "INSERT OR IGNORE INTO chunks (chunk_id, ref_count, access_time) 
             VALUES (?1, 0, strftime('%s','now'))",
            [&chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkMgrDB: create path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        // Insert the path and increment ref_count
        tx.execute(
            "INSERT INTO paths (path, chunk_id, app_id, user_id) VALUES (?1, ?2, ?3, ?4)",
            [&path, &chunk_id.to_string(), app_id, user_id],
        ).map_err(|e| {
            warn!("ChunkMgrDB: create path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        tx.execute(
            "UPDATE chunks SET ref_count = ref_count + 1 WHERE chunk_id = ?1",
            [&chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkMgrDB: create path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        tx.commit().map_err(|e| {
            warn!("ChunkMgrDB: create path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn set_path(&self, path: String,new_chunk_id:&ChunkId,app_id:&str,user_id:&str) -> ChunkResult<()> {
        //如果不存在路径则创建，否则更新已经存在的路径指向的chunk
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkMgrDB: set path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        // Check if the path exists
        let existing_chunk_id: Result<String, _> = tx.query_row(
            "SELECT chunk_id FROM paths WHERE path = ?1",
            [&path],
            |row| row.get(0),
        );

        match existing_chunk_id {
            Ok(chunk_id) => {
                // Path exists, update the chunk_id
                tx.execute(
                    "UPDATE paths SET chunk_id = ?1, app_id = ?2, user_id = ?3 WHERE path = ?4",
                    [&new_chunk_id.to_string(), app_id, user_id, &path],
                ).map_err(|e| {
                    warn!("ChunkMgrDB: set path failed! {}", e.to_string());
                    ChunkError::DbError(e.to_string())
                })?;

                // Decrease ref_count of the old chunk
                tx.execute(
                    "UPDATE chunks SET ref_count = ref_count - 1 WHERE chunk_id = ?1",
                    [&chunk_id],
                ).map_err(|e| {
                    warn!("ChunkMgrDB: set path failed! {}", e.to_string());
                    ChunkError::DbError(e.to_string())
                })?;

                // Remove the old chunk if ref_count becomes 0
                tx.execute(
                    "DELETE FROM chunks WHERE chunk_id = ?1 AND ref_count <= 0",
                    [&chunk_id],
                ).map_err(|e| {
                    warn!("ChunkMgrDB: set path failed! {}", e.to_string());
                    ChunkError::DbError(e.to_string())
                })?;
            },
            Err(_) => {
                // Path does not exist, create a new path
                tx.execute(
                    "INSERT INTO paths (path, chunk_id, app_id, user_id) VALUES (?1, ?2, ?3, ?4)",
                    [&path, &new_chunk_id.to_string(), app_id, user_id],
                ).map_err(|e| {
                    warn!("ChunkMgrDB: set path failed! {}", e.to_string());
                    ChunkError::DbError(e.to_string())
                })?;
            }
        }

        // Increase ref_count of the new chunk
        tx.execute(
            "INSERT OR IGNORE INTO chunks (chunk_id, ref_count, access_time) 
             VALUES (?1, 0, strftime('%s','now'))",
            [&new_chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkMgrDB: set path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        tx.execute(
            "UPDATE chunks SET ref_count = ref_count + 1 WHERE chunk_id = ?1",
            [&new_chunk_id.to_string()],
        ).map_err(|e| {
            warn!("ChunkMgrDB: set path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        tx.commit().map_err(|e| {
            warn!("ChunkMgrDB: set path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn remove_path(&self, path: String) -> ChunkResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkMgrDB: remove path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        // Get the chunk_id for this path
        let chunk_id: String = tx.query_row(
            "SELECT chunk_id FROM paths WHERE path = ?1",
            [&path],
            |row| row.get(0),
        ).map_err(|e| {
            warn!("ChunkMgrDB: remove path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        // Remove the path
        tx.execute("DELETE FROM paths WHERE path = ?1", [&path])
        .map_err(|e| {
            warn!("ChunkMgrDB: remove path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        // Decrease ref_count and remove chunk if ref_count becomes 0
        tx.execute(
            "UPDATE chunks SET ref_count = ref_count - 1 WHERE chunk_id = ?1",
            [&chunk_id],
        ).map_err(|e| {
            warn!("ChunkMgrDB: remove path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        tx.execute(
            "DELETE FROM chunks WHERE chunk_id = ?1 AND ref_count <= 0",
            [&chunk_id],
        ).map_err(|e| {
            warn!("ChunkMgrDB: remove path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        tx.commit().map_err(|e| {
            warn!("ChunkMgrDB: remove path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn remove_dir_path(&self, path: String) -> ChunkResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        // Get all paths and their chunk_ids that start with the given directory path
        let mut stmt = tx.prepare(
            "SELECT path, chunk_id FROM paths WHERE path LIKE ?1"
        ).map_err(|e| {
            warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;
        
        let rows = stmt.query_map(
            [format!("{}%", path)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|e| {
            warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        let path_chunks: Vec<(String, String)> = rows.filter_map(Result::ok).collect();

        // Remove paths and update chunk ref counts within the transaction
        for (path, chunk_id) in path_chunks {
            // Remove the path
            tx.execute("DELETE FROM paths WHERE path = ?1", [&path])
            .map_err(|e| {
                warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
                ChunkError::DbError(e.to_string())
            })?;

            // Decrease ref_count and remove chunk if ref_count becomes 0
            tx.execute(
                "UPDATE chunks SET ref_count = ref_count - 1 WHERE chunk_id = ?1",
                [&chunk_id],
            ).map_err(|e| {
                warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
                ChunkError::DbError(e.to_string())
            })?;

            tx.execute(
                "DELETE FROM chunks WHERE chunk_id = ?1 AND ref_count <= 0",
                [&chunk_id],
            ).map_err(|e| {
                warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
                ChunkError::DbError(e.to_string())
            })?;
        }

        drop(stmt);
        tx.commit().map_err(|e| {
            warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
            ChunkError::DbError(e.to_string())
        })?;

        Ok(())
    }
}



lazy_static! {
    static ref CHUNK_MGR_MAP:Arc<tokio::sync::Mutex<HashMap<String,Arc<tokio::sync::Mutex<ChunkMgr>>>>> = {
        Arc::new(tokio::sync::Mutex::new(HashMap::new()))
    };
}


#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ChunkMgrConfig {
    local_stores:Vec<String>,
    local_cache:Option<String>,
    mmap_cache_dir:Option<String>,
}

pub struct ChunkMgr {
    local_store_list:Vec<ChunkStore>,//Real chunk store
    local_cache:Option<ChunkStore>,//Cache at local disk
    mmap_cache_dir:Option<String>,//Cache at memory
    mgr_id:Option<String>,
    db:ChunkMgrDB,
}

impl ChunkMgr {
    pub async fn get_chunk_mgr_by_id(chunk_mgr_id:Option<&str>)->Option<Arc<tokio::sync::Mutex<Self>>> {
        let chunk_mgr_key = chunk_mgr_id.unwrap_or("default").to_string();
        let mut chunk_mgr_map = CHUNK_MGR_MAP.lock().await;
        let chunk_mgr = chunk_mgr_map.get(&chunk_mgr_key);
        if chunk_mgr.is_some() {
            return Some(chunk_mgr.unwrap().clone());
        }

        let root_path = get_buckyos_chunk_data_dir(chunk_mgr_id);
        //make sure the root path dir exists
        if !root_path.exists() {
            fs::create_dir_all(root_path.clone()).await.unwrap();
        }
        let mgr_config;
        let mgr_json_file = root_path.join("chunk_mgr.json");
        if !mgr_json_file.exists() {
            mgr_config = ChunkMgrConfig {
                local_stores:vec!["./".to_string()],
                local_cache:None,
                mmap_cache_dir:None,
            };

            let mgr_json_str = serde_json::to_string(&mgr_config).unwrap();
            let mut mgr_json_file = File::create(mgr_json_file.clone()).await.unwrap();
            mgr_json_file.write_all(mgr_json_str.as_bytes()).await.unwrap();
        } else {
            let mgr_json_str = fs::read_to_string(mgr_json_file).await;
            if mgr_json_str.is_err() {
                warn!("ChunkMgr: read mgr config failed! {}", mgr_json_str.err().unwrap().to_string());
                return None;
            }
            let mgr_json_str = mgr_json_str.unwrap();
            let mgr_config_result = serde_json::from_str::<ChunkMgrConfig>(&mgr_json_str);
            if mgr_config_result.is_err() {
                warn!("ChunkMgr: parse mgr config failed! {}", mgr_config_result.err().unwrap().to_string());
                return None;
            }
            mgr_config = mgr_config_result.unwrap();
    
        } 

        let result_mgr = Self::from_config(chunk_mgr_id.map(|s|s.to_string()),root_path,mgr_config).await;
        if result_mgr.is_err() {
            warn!("ChunkMgr: create mgr failed! {}", result_mgr.err().unwrap().to_string());
            return None;
        }
        let result_mgr = Arc::new(tokio::sync::Mutex::new(result_mgr.unwrap()));
        chunk_mgr_map.insert(chunk_mgr_key,result_mgr.clone());
        return Some(result_mgr);
    }

    pub async fn from_config(mgr_id:Option<String>,root_path:PathBuf,config:ChunkMgrConfig)->ChunkResult<Self> {
        let db_path = root_path.join("chunk_mgr.db").to_str().unwrap().to_string();
        let db = ChunkMgrDB::new(db_path)?;
        let mut local_store_list = vec![];
        for local_store_path in config.local_stores.iter() {
            let local_store = ChunkStore::new(local_store_path.clone()).await?;
            local_store_list.push(local_store);
        }
        let local_cache;
        if config.local_cache.is_some() {
            local_cache = Some(ChunkStore::new(config.local_cache.as_ref().unwrap().clone()).await?);
        } else {
            local_cache = None;
        }
        Ok(Self {
            local_store_list:local_store_list,
            local_cache:local_cache,
            mmap_cache_dir:config.mmap_cache_dir,
            mgr_id:mgr_id,
            db:db,
        })
    }

    fn get_cache_mmap_path(&self, chunk_id:&ChunkId)->Option<String> {
        None
    }

    pub async fn get_chunk_reader_by_path(&self, path:String,user_id:&str,app_id:&str)->ChunkResult<(Pin<Box<dyn ChunkReadSeek + Send + Sync + Unpin>>,u64)> {
        let chunk_id = self.db.get_path_target_chunk(&path);
        if chunk_id.is_err() {
            warn!("get_chunk_reader_by_path: no chunk_id for path:{}", path);
            return Err(ChunkError::ChunkNotFound(path));
        }
        let chunk_id = chunk_id.unwrap();
        let access_time = buckyos_get_unix_timestamp();
        self.db.update_chunk_access_time(&chunk_id, access_time)?;
        self.get_chunk_reader(&chunk_id, false).await
    }

    pub async fn create_file(&self, path:String,chunk_id:&ChunkId,app_id:&str,user_id:&str)->ChunkResult<()> {
        self.db.create_path(chunk_id, path.clone(), app_id, user_id).map_err(|e| {
            warn!("create_file: create path failed! {}", e.to_string());
            e
        })?;
        info!("create path:{} ==> {}", path, chunk_id.to_string());
        Ok(())
    }

    pub async fn set_file(&self, path:String,new_chunk_id:&ChunkId,app_id:&str,user_id:&str)->ChunkResult<()> {
        self.db.set_path(path.clone(), new_chunk_id, app_id, user_id).map_err(|e| {
            warn!("update_file: update path failed! {}", e.to_string());
            e
        })?;
        info!("update path:{} ==> {}", path, new_chunk_id.to_string());
        Ok(())
    }

    pub async fn remove_file(&self, path:String)->ChunkResult<()> {
        self.db.remove_path(path.clone()).map_err(|e| {
            warn!("remove_file: remove path failed! {}", e.to_string());
            e
        })?;
        info!("remove path:{}", path);
        Ok(())

        //TODO: 这里不立刻删除chunk,而是等统一的GC来删除
    }

    pub async fn remove_dir(&self, path:String)->ChunkResult<()> {
        self.db.remove_dir_path(path.clone()).map_err(|e| {
            warn!("remove_dir: remove dir path failed! {}", e.to_string());
            e
        })?;
        info!("remove dir path:{}", path);
        Ok(())
    }

    //得到已经存在chunk的reader
    pub async fn get_chunk_reader(&self, chunk_id:&ChunkId,auto_cache:bool)->ChunkResult<(Pin<Box<dyn ChunkReadSeek + Send + Sync + Unpin>>,u64)> {
        //at first ,do access control
        let mcache_file_path = self.get_cache_mmap_path(chunk_id);
        if mcache_file_path.is_some() {
            let mcache_file_path = mcache_file_path.unwrap();
            let file = File::open(mcache_file_path.clone()).await;
            if file.is_ok() {
                let file = file.unwrap();
                let file_meta = file.metadata().await.unwrap();
                info!("get_chunk_reader:return tmpfs cache file:{}", mcache_file_path);
                return Ok((Box::pin(file),file_meta.len()));
            }
        }

        if self.local_cache.is_some() {
            let local_cache = self.local_cache.as_ref().unwrap();
            let local_reader = local_cache.get_chunk_reader(chunk_id).await;
            if local_reader.is_ok() {
                info!("get_chunk_reader:return local cache file:{}", chunk_id.to_string());
                return local_reader;
            }
        }

        warn!("get_chunk_reader: no cache file:{}", chunk_id.to_string());

        for local_store in self.local_store_list.iter() {
            let local_reader = local_store.get_chunk_reader(chunk_id).await;
            if local_reader.is_ok() {
                //TODO:将结果数据添加到自动cache管理中
                //caceh是完整的，还是可以支持部分？
                return local_reader;
            }
        }

        Err(ChunkError::ChunkNotFound(chunk_id.to_string()))
    }

    pub async fn open_chunk_writer(&self, chunk_id:&ChunkId,chunk_size:u64,append:bool)->ChunkResult<(Pin<Box<dyn AsyncWrite + Send + Sync + Unpin>>,u64)> {
        Err(ChunkError::Internal("no chunk mgr".to_string()))
    }

    pub async fn close_chunk_writer(&self, chunk_id:&ChunkId)->ChunkResult<()> {
        Err(ChunkError::Internal("no chunk mgr".to_string()))
    }


}


