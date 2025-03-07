use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_root_dir, get_by_json_path};
use serde::{Serialize,Deserialize};
//chunk_mgr默认是机器级别的，即多个进程可以共享同一个chunk_mgr
//chunk_mgr使用共享内存/filemap等技术来实现跨进程的读数据共享，比使用127.0.0.1的http协议更高效
//从实现简单的角度考虑，先用http协议实现写数据？
use tokio::{
    fs::{self, File,OpenOptions}, 
    io::{self, AsyncRead,AsyncWrite, AsyncReadExt, AsyncWriteExt, AsyncSeek, AsyncSeekExt}, 
};
use log::*;
use crate::{build_named_object_by_json, ChunkHasher, ChunkId, ChunkReadSeek, ChunkState, FileObject, NamedDataStore, NdnError, NdnResult};
use memmap::Mmap;
use std::{path::PathBuf, pin::Pin};
use std::io::SeekFrom;
use std::sync::Mutex;
use std::sync::Arc;
use std::collections::HashMap;
use rusqlite::{Connection};
use lazy_static::lazy_static;

use buckyos_kit::get_buckyos_chunk_data_dir;

use crate::{ChunkReader,ChunkWriter,ObjId};

pub struct NamedDataMgrDB {
    db_path: String,
    conn: Mutex<Connection>,
}

impl NamedDataMgrDB {
    pub fn new(db_path: String) -> NdnResult<Self> {
        let conn = Connection::open(&db_path).map_err(|e| {
            warn!("NamedDataMgrDB: open db failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        
        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS objs (
                obj_id TEXT PRIMARY KEY,
                ref_count INTEGER NOT NULL DEFAULT 0,
                access_time INTEGER NOT NULL,
                size INTEGER NOT NULL DEFAULT 0
            )",
            [],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: create objs table failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS paths (
                path TEXT PRIMARY KEY,
                obj_id TEXT NOT NULL,
                app_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                FOREIGN KEY(obj_id) REFERENCES objs(obj_id)
            )",
            [],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: create paths table failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        Ok(Self {
            db_path,
            conn: Mutex::new(conn),
        })
    }

    pub fn get_path_target_objid(&self, path: &str)->NdnResult<ObjId> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT obj_id FROM paths WHERE path = ?1").map_err(|e| {
            warn!("NamedDataMgrDB: prepare statement failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        let obj_id: String = stmt.query_row([path], |row| row.get(0)).map_err(|e| {
            warn!("NamedDataMgrDB: query path target obj failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        ObjId::new(&obj_id).map_err(|e| {
            warn!("NamedDataMgrDB: invalid obj_id format! {}", e.to_string());
            NdnError::Internal(e.to_string())
        })
    }

    pub fn update_obj_access_time(&self, obj_id: &ObjId, access_time: u64) -> NdnResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE objs SET access_time = ?1 WHERE obj_id = ?2",
            [&access_time.to_string(), &obj_id.to_string()],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: update obj access time failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn create_path(&self, obj_id: &ObjId, path: String,app_id:&str,user_id:&str) -> NdnResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("NamedDataMgrDB: create path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        
        // Insert or update the chunk entry
        tx.execute(
            "INSERT OR IGNORE INTO objs (obj_id, ref_count, access_time) 
             VALUES (?1, 0, strftime('%s','now'))",
            [&obj_id.to_string()],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: create path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Insert the path and increment ref_count
        tx.execute(
            "INSERT INTO paths (path, obj_id, app_id, user_id) VALUES (?1, ?2, ?3, ?4)",
            [&path, &obj_id.to_string(), app_id, user_id],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: create path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        tx.execute(
            "UPDATE objs SET ref_count = ref_count + 1 WHERE obj_id = ?1",
            [&obj_id.to_string()],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: create path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        tx.commit().map_err(|e| {
            warn!("NamedDataMgrDB: create path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn set_path(&self, path: &str,new_obj_id:&ObjId,app_id:&str,user_id:&str) -> NdnResult<()> {
        //如果不存在路径则创建，否则更新已经存在的路径指向的chunk
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Check if the path exists
        let existing_obj_id: Result<String, _> = tx.query_row(
            "SELECT obj_id FROM paths WHERE path = ?1",
            [&path],
            |row| row.get(0),
        );

        let obj_id_str = new_obj_id.to_string();

        match existing_obj_id {
            Ok(obj_id) => {
                // Path exists, update the obj_id
                tx.execute(
                    "UPDATE paths SET obj_id = ?1, app_id = ?2, user_id = ?3 WHERE path = ?4",
                    [obj_id_str.as_str(), app_id, user_id, &path],
                ).map_err(|e| {
                    warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
                    NdnError::DbError(e.to_string())
                })?;

                // Decrease ref_count of the old chunk
                tx.execute(
                    "UPDATE objs SET ref_count = ref_count - 1 WHERE obj_id = ?1",
                    [obj_id_str.as_str()],
                ).map_err(|e| {
                    warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
                    NdnError::DbError(e.to_string())
                })?;

                // Remove the old chunk if ref_count becomes 0
                tx.execute(
                    "DELETE FROM objs WHERE obj_id = ?1 AND ref_count <= 0",
                    [obj_id_str.as_str()],
                ).map_err(|e| {
                    warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
                    NdnError::DbError(e.to_string())
                })?;
            },
            Err(_) => {
                // Path does not exist, create a new path
                tx.execute(
                    "INSERT INTO paths (path, obj_id, app_id, user_id) VALUES (?1, ?2, ?3, ?4)",
                    [&path, obj_id_str.as_str(), app_id, user_id],
                ).map_err(|e| {
                    warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
                    NdnError::DbError(e.to_string())
                })?;
            }
        }

        // Increase ref_count of the new chunk
        tx.execute(
            "INSERT OR IGNORE INTO objs (obj_id, ref_count, access_time) 
             VALUES (?1, 0, strftime('%s','now'))",
            [&new_obj_id.to_string()],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        tx.execute(
            "UPDATE objs SET ref_count = ref_count + 1 WHERE obj_id = ?1",
            [&new_obj_id.to_string()],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        tx.commit().map_err(|e| {
            warn!("NamedDataMgrDB: set path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn remove_path(&self, path: &str) -> NdnResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Get the chunk_id for this path
        let obj_id: String = tx.query_row(
            "SELECT obj_id FROM paths WHERE path = ?1",
            [&path],
            |row| row.get(0),
        ).map_err(|e| {
            warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Remove the path
        tx.execute("DELETE FROM paths WHERE path = ?1", [&path])
        .map_err(|e| {
            warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Decrease ref_count and remove chunk if ref_count becomes 0
        tx.execute(
            "UPDATE objs SET ref_count = ref_count - 1 WHERE obj_id = ?1",
            [&obj_id],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        tx.execute(
            "DELETE FROM objs WHERE obj_id = ?1 AND ref_count <= 0",
            [&obj_id],
        ).map_err(|e| {
            warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        tx.commit().map_err(|e| {
            warn!("NamedDataMgrDB: remove path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        Ok(())
    }

    pub fn remove_dir_path(&self, path: &str) -> NdnResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        // Get all paths and their chunk_ids that start with the given directory path
        let mut stmt = tx.prepare(
            "SELECT path, obj_id FROM paths WHERE path LIKE ?1"
        ).map_err(|e| {
            warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;
        
        let rows = stmt.query_map(
            [format!("{}%", path)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|e| {
            warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        let path_objs: Vec<(String, String)> = rows.filter_map(Result::ok).collect();

        // Remove paths and update chunk ref counts within the transaction
        for (path, obj_id) in path_objs {
            // Remove the path
            tx.execute("DELETE FROM paths WHERE path = ?1", [&path])
            .map_err(|e| {
                warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

            // Decrease ref_count and remove chunk if ref_count becomes 0
            tx.execute(
                "UPDATE objs SET ref_count = ref_count - 1 WHERE obj_id = ?1",
                [&obj_id],
            ).map_err(|e| {
                warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;

            tx.execute(
                "DELETE FROM objs WHERE obj_id = ?1 AND ref_count <= 0",
                [&obj_id],
            ).map_err(|e| {
                warn!("NamedDataMgrDB: remove dir path failed! {}", e.to_string());
                NdnError::DbError(e.to_string())
            })?;
        }

        drop(stmt);
        tx.commit().map_err(|e| {
            warn!("ChunkMgrDB: remove dir path failed! {}", e.to_string());
            NdnError::DbError(e.to_string())
        })?;

        Ok(())
    }
}



lazy_static! {
    pub static ref NAMED_DATA_MGR_MAP:Arc<tokio::sync::Mutex<HashMap<String,Arc<tokio::sync::Mutex<NamedDataMgr>>>>> = {
        Arc::new(tokio::sync::Mutex::new(HashMap::new()))
    };
}


#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct NamedDataMgrConfig {
    pub local_stores:Vec<String>,
    pub local_cache:Option<String>,
    pub mmap_cache_dir:Option<String>,
}

pub struct NamedDataMgr {
    local_store_list:Vec<NamedDataStore>,//Real chunk store
    local_cache:Option<NamedDataStore>,//Cache at local disk
    mmap_cache_dir:Option<String>,//Cache at memory
    mgr_id:Option<String>,
    db:NamedDataMgrDB,
}

impl NamedDataMgr {
    pub async fn set_mgr_by_id(named_data_mgr_id:Option<&str>,mgr:NamedDataMgr) -> NdnResult<()> {
        let named_data_mgr_key = named_data_mgr_id.unwrap_or("default").to_string();
        let mut named_data_mgr_map = NAMED_DATA_MGR_MAP.lock().await;
        named_data_mgr_map.insert(named_data_mgr_key,Arc::new(tokio::sync::Mutex::new(mgr)));
        Ok(())
    }

    pub async fn get_named_data_mgr_by_id(named_data_mgr_id:Option<&str>)->Option<Arc<tokio::sync::Mutex<Self>>> {
        let named_mgr_key = named_data_mgr_id.unwrap_or("default").to_string();
        let mut named_data_mgr_map = NAMED_DATA_MGR_MAP.lock().await;

        let named_data_mgr = named_data_mgr_map.get(&named_mgr_key);
        if named_data_mgr.is_some() {
            return Some(named_data_mgr.unwrap().clone());
        }

        info!("NamedDataMgr: auto create new named data mgr for mgr_id:{}", named_mgr_key);
        let root_path = get_buckyos_chunk_data_dir(named_data_mgr_id);
        //make sure the root path dir exists
        if !root_path.exists() {
            fs::create_dir_all(root_path.clone()).await.unwrap();
        }
        let mgr_config;
        let mgr_json_file = root_path.join("chunk_mgr.json");
        if !mgr_json_file.exists() {
            mgr_config = NamedDataMgrConfig {
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
            let mgr_config_result = serde_json::from_str::<NamedDataMgrConfig>(&mgr_json_str);
            if mgr_config_result.is_err() {
                warn!("ChunkMgr: parse mgr config failed! {}", mgr_config_result.err().unwrap().to_string());
                return None;
            }
            mgr_config = mgr_config_result.unwrap();
    
        } 

        let result_mgr = Self::from_config(named_data_mgr_id.map(|s|s.to_string()),root_path,mgr_config).await;
        if result_mgr.is_err() {
            warn!("ChunkMgr: create mgr failed! {}", result_mgr.err().unwrap().to_string());
            return None;
        }
        let result_mgr = Arc::new(tokio::sync::Mutex::new(result_mgr.unwrap()));
        named_data_mgr_map.insert(named_mgr_key,result_mgr.clone());
        return Some(result_mgr);
    }

    pub async fn from_config(mgr_id:Option<String>,root_path:PathBuf,config:NamedDataMgrConfig)->NdnResult<Self> {
        let db_path = root_path.join("chunk_mgr.db").to_str().unwrap().to_string();
        let db = NamedDataMgrDB::new(db_path)?;
        let mut local_store_list = vec![];
        for local_store_path in config.local_stores.iter() {
            let real_local_store_path;
            if local_store_path.starts_with(".") {
                real_local_store_path = root_path.join(local_store_path);
            } else {
                real_local_store_path = PathBuf::from(local_store_path);
            }
            let local_store = NamedDataStore::new(real_local_store_path.to_str().unwrap().to_string()).await?;
            local_store_list.push(local_store);
        }
        let local_cache;
        if config.local_cache.is_some() {
            local_cache = Some(NamedDataStore::new(config.local_cache.as_ref().unwrap().clone()).await?);
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

    pub async fn get_obj_id_by_path(&self, path:String)->NdnResult<ObjId> {
        let obj_id = self.db.get_path_target_objid(&path)?;
        Ok(obj_id)
    }

    pub async fn get_object(&self, obj_id:&ObjId,inner_obj_path:Option<String>)->NdnResult<serde_json::Value> {
        if obj_id.is_chunk() {
            return Err(NdnError::InvalidObjType(obj_id.to_string()));
        }

        let mut obj_body = None;
        if self.local_cache.is_some() {
            let local_cache = self.local_cache.as_ref().unwrap();
            let obj_result = local_cache.get_object(&obj_id).await;
            if obj_result.is_ok() {
                obj_body = Some(obj_result.unwrap());
            }
        }

        for local_store in self.local_store_list.iter() {
            let obj_result = local_store.get_object(&obj_id).await;
            if obj_result.is_ok() {
                obj_body = Some(obj_result.unwrap());
                break;
            }
        }        

        if obj_body.is_some() {
            let obj_body = obj_body.unwrap();
            let obj_body = obj_body.to_json_value()
                .map_err(|e| {
                    warn!("get_object: decode obj body failed! {}", e.to_string());
                    NdnError::DecodeError(e.to_string())
                })?;

            if inner_obj_path.is_some() {
                let obj_path = inner_obj_path.unwrap();
                let obj_filed = get_by_json_path(&obj_body, &obj_path);
                if obj_filed.is_some() {
                    return Ok(obj_filed.unwrap());
                }
            }
            return Ok(obj_body);
        }

        Err(NdnError::NotFound(obj_id.to_string()))
    }

    pub async fn put_object(&self, obj_id:&ObjId,obj_data:&str)->NdnResult<()> {
        for local_store in self.local_store_list.iter() {
            //TODO: select best local store to write?
            local_store.put_object(obj_id, obj_data, false).await?;
            break;
        }
        Ok(())
    }

    pub async fn get_chunk_reader_by_path(&self, path:String,user_id:&str,app_id:&str,seek_from:SeekFrom)->NdnResult<(ChunkReader,u64,ChunkId)> {
        let obj_id = self.db.get_path_target_objid(&path);
        if obj_id.is_err() {
            warn!("get_chunk_reader_by_path: no chunk_id for path:{}", path);
            return Err(NdnError::NotFound(path));
        }
        let obj_id = obj_id.unwrap();
        if !obj_id.is_chunk() {
            warn!("get_chunk_reader_by_path: path:{} , obj type is not chunk:{}", path, obj_id.to_string());
            return Err(NdnError::InvalidObjType(obj_id.to_string()));
        }
        let chunk_id = ChunkId::from_obj_id(&obj_id);
        let (chunk_reader,chunk_size) = self.open_chunk_reader(&chunk_id, seek_from, true).await?;
        let access_time = buckyos_get_unix_timestamp();
        self.db.update_obj_access_time(&obj_id, access_time)?;
        Ok((chunk_reader,chunk_size,chunk_id))
    }

    pub async fn create_file(&self, path:String,obj_id:&ObjId,app_id:&str,user_id:&str)->NdnResult<()> {
        self.db.create_path(obj_id, path.clone(), app_id, user_id).map_err(|e| {
            warn!("create_file: create path failed! {}", e.to_string());
            e
        })?;
        info!("create path:{} ==> {}", path, obj_id.to_string());
        Ok(())
    }

    pub async fn set_file(&self, path:String,new_obj_id:&ObjId,app_id:&str,user_id:&str)->NdnResult<()> {
        self.db.set_path(&path, &new_obj_id, app_id, user_id).map_err(|e| {
            warn!("update_file: update path failed! {}", e.to_string());
            e
        })?;
        info!("update path:{} ==> {}", path, new_obj_id.to_string());
        Ok(())
    }

    pub async fn remove_file(&self, path:&str)->NdnResult<()> {
        self.db.remove_path(path).map_err(|e| {
            warn!("remove_file: remove path failed! {}", e.to_string());
            e
        })?;
        info!("remove path:{}", path);
        Ok(())

        //TODO: 这里不立刻删除chunk,而是等统一的GC来删除
    }

    pub async fn remove_dir(&self, path:&str)->NdnResult<()> {
        self.db.remove_dir_path(path).map_err(|e| {
            warn!("remove_dir: remove dir path failed! {}", e.to_string());
            e
        })?;
        info!("remove dir path:{}", path);
        Ok(())
    }

    pub async fn is_chunk_exist(&self, chunk_id:&ChunkId)->NdnResult<bool> {
        for local_store in self.local_store_list.iter() {
            let (is_exist,chunk_size) = local_store.is_chunk_exist(chunk_id,None).await?;
            if is_exist {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn have_chunk(chunk_id:&ChunkId,mgr_id:Option<&str>)->bool {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return false;
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        let is_exist = named_mgr.is_chunk_exist(chunk_id).await;
        if is_exist.is_err() {
            return false;
        }
        is_exist.unwrap()
    }

    pub async fn query_chunk_state(&self, chunk_id: &ChunkId) -> NdnResult<(ChunkState,u64,String)> {
        for local_store in self.local_store_list.iter() {
            let (chunk_state,chunk_size,progress) = local_store.query_chunk_state(chunk_id).await?;
            if chunk_state != ChunkState::NotExist {
                return Ok((chunk_state,chunk_size,progress));
            }
        }
        Ok((ChunkState::NotExist,0,"".to_string()))
    }

  
    pub async fn open_chunk_reader(&self, chunk_id:&ChunkId,seek_from:SeekFrom,auto_cache:bool)->NdnResult<(ChunkReader,u64)> {
        // memroy cache ==> local disk cache ==> local store
        //at first ,do access control
        let mcache_file_path = self.get_cache_mmap_path(chunk_id);
        if mcache_file_path.is_some() {
            let mcache_file_path = mcache_file_path.unwrap();

            let mut file = OpenOptions::new()
                .read(true)  // 设置只读模式
                .open(&mcache_file_path)
                .await;
            
            if file.is_ok() {
                let mut file = file.unwrap();
                let file_meta = file.metadata().await.unwrap();
                if seek_from != SeekFrom::Start(0) {
                    file.seek(seek_from).await.map_err(|e| {
                        warn!("get_chunk_reader: seek cache file failed! {}", e.to_string());
                        NdnError::IoError(e.to_string())
                    })?;
                }
                //info!("get_chunk_reader:return tmpfs cache file:{}", mcache_file_path);
                return Ok((Box::pin(file),file_meta.len()));
            }
        }

        warn!("get_chunk_reader: CACHE MISS :{}", chunk_id.to_string());
        if self.local_cache.is_some() {
            let local_cache = self.local_cache.as_ref().unwrap();
            let local_reader = local_cache.open_chunk_reader(chunk_id,seek_from).await;
            if local_reader.is_ok() {
                info!("get_chunk_reader:return local cache file:{}", chunk_id.to_string());
                return local_reader;
            }
        }

        warn!("get_chunk_reader: no cache file:{}", chunk_id.to_string());
        for local_store in self.local_store_list.iter() {
            let local_reader = local_store.open_chunk_reader(chunk_id,seek_from).await;
            if local_reader.is_ok() {
                //TODO:将结果数据添加到自动cache管理中
                //caceh是完整的，还是可以支持部分？
                return local_reader;
            }
        }

        Err(NdnError::NotFound(chunk_id.to_string()))
    }

    //return chunk_id,progress_info 
    pub async fn open_chunk_writer(&self, chunk_id: &ChunkId,chunk_size:u64,offset:u64)
        ->NdnResult<(ChunkWriter,String)> 
    {
        let default_store = self.local_store_list.first().unwrap();
        let (writer,chunk_progress_info) = default_store.open_chunk_writer(chunk_id,chunk_size,offset).await?;
        Ok((writer,chunk_progress_info))
    }

    pub async fn update_chunk_progress(&self, chunk_id:&ChunkId, progress:String)->NdnResult<()> {
        let default_store = self.local_store_list.first().unwrap();
        default_store.update_chunk_progress(chunk_id, progress).await
    }

    pub async fn complete_chunk_writer(&self, chunk_id:&ChunkId)->NdnResult<()> {
        let default_store = self.local_store_list.first().unwrap();
        default_store.complete_chunk_writer(chunk_id).await
    }


    //下面是一些helper函数
    pub async fn pub_object(mgr_id:Option<&str>,will_pub_obj:serde_json::Value,obj_type:&str,ndn_path:&str,
                            user_id:&str,app_id:&str)->NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let (obj_id,obj_str) = build_named_object_by_json(obj_type,&will_pub_obj);
        let mut named_mgr = named_mgr.lock().await;
        named_mgr.put_object(&obj_id, &obj_str).await?;
        named_mgr.set_file(ndn_path.to_string(), &obj_id, app_id, user_id).await?;
        Ok(())
    }
    
    pub async fn sign_obj(mgr_id:Option<&str>,will_sign_obj_id:ObjId,obj_jwt:String,
                          user_id:&str,app_id:&str)->NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let mut named_mgr = named_mgr.lock().await;
        named_mgr.put_object(&will_sign_obj_id, &obj_jwt).await?;
        Ok(())
    }

    //会写入两个ndn_path
    pub async fn pub_local_file_as_fileobj(mgr_id:Option<&str>,local_file_path:&PathBuf,ndn_path:&str,ndn_content_path:&str,
        fileobj_template:&mut FileObject,user_id:&str,app_id:&str)->NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        //TODO：优化，边算边传，支持断点续传
        let mut file_reader =tokio::fs::File::open(local_file_path).await
            .map_err(|e| {
                error!("open local_file_path failed, err:{}", e);
                NdnError::IoError(format!("open local_file_path failed, err:{}", e))
            })?;
        
        let mut chunk_hasher = ChunkHasher::new(None).unwrap();
        let (chunk_raw_id,chunk_size) = chunk_hasher.calc_from_reader(&mut file_reader).await.unwrap();
        let chunk_id = ChunkId::from_sha256_result(&chunk_raw_id);
        let named_mgr = named_mgr.lock().await;
        let is_exist = named_mgr.is_chunk_exist(&chunk_id).await.unwrap();
        if !is_exist {
            let (mut chunk_writer, _) = named_mgr.open_chunk_writer(&chunk_id, chunk_size, 0).await?;
            file_reader.seek(std::io::SeekFrom::Start(0)).await.unwrap();
            tokio::io::copy(&mut file_reader, &mut chunk_writer).await
                .map_err(|e| {
                    error!("copy local_file {:?} to named-mgr failed, err:{}", local_file_path, e);
                    NdnError::IoError(format!("copy local_file to named-mgr failed, err:{}", e))
                })?;
                
            named_mgr.complete_chunk_writer(&chunk_id).await?;
        }

        fileobj_template.content = chunk_id.to_string();
        fileobj_template.size = chunk_size;
        let (file_obj_id,file_obj_str) = fileobj_template.gen_obj_id();
        named_mgr.put_object(&file_obj_id, file_obj_str.as_str()).await?;
        named_mgr.set_file(ndn_path.to_string(), &file_obj_id, app_id, user_id).await?;

        let chunk_obj_id = chunk_id.to_obj_id();
        named_mgr.set_file(ndn_content_path.to_string(), &chunk_obj_id, app_id, user_id).await?;
        Ok(())
    }

    
    pub async fn pub_local_file_as_chunk(mgr_id:Option<&str>,local_file_path:String,ndn_path:&str,
                                            user_id:&str,app_id:&str)->NdnResult<()> {
        unimplemented!()
    }
    
}


#[cfg(test)]
mod tests {
    use crate::FileObject;

    use super::*;
    use tempfile::tempdir;
    use std::io::SeekFrom;
    use buckyos_kit::*;
    #[tokio::test]
    async fn test_basic_chunk_operations() -> NdnResult<()> {
        // Create a temporary directory for testing
        let test_dir = tempdir().unwrap();
        let config = NamedDataMgrConfig {
            local_stores: vec![test_dir.path().to_str().unwrap().to_string()],
            local_cache: None,
            mmap_cache_dir: None,
        };

        let chunk_mgr = NamedDataMgr::from_config(
            Some("test".to_string()),
            test_dir.path().to_path_buf(),
            config
        ).await?;

        // Create test data
        let test_data = b"Hello, World!";
        let chunk_id = ChunkId::new("sha256:1234567890abcdef").unwrap();
        
        // Write chunk
        let (mut writer, _) = chunk_mgr.open_chunk_writer(&chunk_id, test_data.len() as u64, 0).await.unwrap();
        writer.write_all(test_data).await.unwrap();
        chunk_mgr.complete_chunk_writer(&chunk_id).await.unwrap();

        // Read and verify chunk
        let (mut reader, size) = chunk_mgr.open_chunk_reader(&chunk_id, SeekFrom::Start(0), true).await.unwrap();
        assert_eq!(size, test_data.len() as u64);

        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).await.unwrap();
        assert_eq!(&buffer, test_data);

        Ok(())
    }

    #[tokio::test]
    async fn test_base_operations() -> NdnResult<()> {
        // Create a temporary directory for testing
        init_logging("ndn-lib test");
        let test_dir = tempdir().unwrap();
        let config = NamedDataMgrConfig {
            local_stores: vec![test_dir.path().to_str().unwrap().to_string()],
            local_cache: None,
            mmap_cache_dir: None,
        };

        let chunk_mgr = NamedDataMgr::from_config(
            Some("test".to_string()),
            test_dir.path().to_path_buf(),
            config
        ).await?;

        // Create test data
        let test_data = b"Hello, Path Test!";
        let chunk_id = ChunkId::new("sha256:1234567890abcdef").unwrap();
        let test_path = "/test/file.txt".to_string();
        
        // Write chunk
        let (mut writer, _) = chunk_mgr.open_chunk_writer(&chunk_id, test_data.len() as u64, 0).await?;
        writer.write_all(test_data).await.unwrap();
        chunk_mgr.complete_chunk_writer(&chunk_id).await.unwrap();

        // Bind chunk to path
        chunk_mgr.create_file(
            test_path.clone(),
            &chunk_id.to_obj_id(),
            "test_app",
            "test_user"
        ).await?;


        // Read through path and verify
        let (mut reader, size, retrieved_chunk_id) = chunk_mgr.get_chunk_reader_by_path(
            test_path.clone(),
            "test_user",
            "test_app",
            SeekFrom::Start(0)
        ).await?;
        
        assert_eq!(size, test_data.len() as u64);
        assert_eq!(retrieved_chunk_id, chunk_id);

        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).await.unwrap();
        assert_eq!(&buffer, test_data);

        //test fileobj
        let path2 = "/test/file2.txt".to_string();
        let file_obj = FileObject::new(path2.clone(),test_data.len() as u64,chunk_id.to_string());
        let (file_obj_id,file_obj_str) = file_obj.gen_obj_id();
        info!("file_obj_id:{}",file_obj_id.to_string());
        //file-obj is soft linke to chunk-obj
        chunk_mgr.put_object(&file_obj_id, &file_obj_str).await?;

        let obj_content = chunk_mgr.get_object(&file_obj_id,Some("/content".to_string())).await?;
        info!("obj_content:{}",obj_content);
        assert_eq!(obj_content.as_str().unwrap(),chunk_id.to_string().as_str());
        // Test remove file
        chunk_mgr.remove_file(&test_path).await.unwrap();

        // Verify path is removed
        let result = chunk_mgr.get_chunk_reader_by_path(
            test_path.clone(),
            "test_user",
            "test_app",
            SeekFrom::Start(0)
        ).await;
        assert!(result.is_err());

        Ok(())
    }


    //test get_chunk_mgr_by_id，然后再创建并写入一个chunk，再读取
    #[tokio::test]
    async fn test_get_chunk_mgr_by_id() -> NdnResult<()> {
        // Get ChunkMgr by id
        let chunk_mgr_id = None;
        let chunk_mgr = NamedDataMgr::get_named_data_mgr_by_id(chunk_mgr_id).await;
        assert!(chunk_mgr.is_some());
        let chunk_mgr = chunk_mgr.unwrap();

        // Create test data
        let test_data = b"Hello, ChunkMgr Test!";
        let chunk_id = ChunkId::new("sha256:abcdef1234567890").unwrap();
        
        // Write chunk
        {
            let mut chunk_mgr = chunk_mgr.lock().await;
            let (mut writer, _) = chunk_mgr.open_chunk_writer(&chunk_id, test_data.len() as u64, 0).await.unwrap();
            writer.write_all(test_data).await.unwrap();
            chunk_mgr.complete_chunk_writer(&chunk_id).await.unwrap();
        }

        // Read chunk and verify
        {
            let chunk_mgr = chunk_mgr.lock().await;
            let (mut reader, size) = chunk_mgr.open_chunk_reader(&chunk_id, SeekFrom::Start(0), true).await?;
            assert_eq!(size, test_data.len() as u64);

            let mut buffer = Vec::new();
            reader.read_to_end(&mut buffer).await.unwrap();
            assert_eq!(&buffer, test_data);
        }

        Ok(())
    }
}

