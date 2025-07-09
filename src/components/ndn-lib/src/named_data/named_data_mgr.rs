use super::named_data_mgr_db::NamedDataMgrDB;
use crate::{
    build_named_object_by_json, ChunkHasher, ChunkId, ChunkListReader, ChunkReadSeek, ChunkState,
    FileObject, NamedDataStore, NdnError, NdnResult, PathObject,
};
use crate::{ChunkList, ChunkReader, ChunkWriter, ObjId};
use buckyos_kit::get_buckyos_named_data_dir;
use buckyos_kit::{
    buckyos_get_unix_timestamp, get_buckyos_root_dir, get_by_json_path, get_relative_path,
};
use futures_util::stream;
use futures_util::stream::StreamExt;
use lazy_static::lazy_static;
use log::*;
use memmap::Mmap;
use name_lib::decode_jwt_claim_without_verify;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::io as std_io;
use std::io::SeekFrom;
use std::sync::Arc;
use std::sync::Mutex;
use std::{path::PathBuf, pin::Pin};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{self, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt},
};
use tokio_util::bytes::BytesMut;
use tokio_util::io::StreamReader;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedDataMgrConfig {
    pub local_stores: Vec<String>,
    pub local_cache: Option<String>,
    pub mmap_cache_dir: Option<String>,
}

pub struct NamedDataMgr {
    local_store_list: Vec<NamedDataStore>, //Real chunk store
    local_cache: Option<NamedDataStore>,   //Cache at local disk
    mmap_cache_dir: Option<String>,        //Cache at memory
    mgr_id: Option<String>,
    db: NamedDataMgrDB,
}

impl NamedDataMgr {
    pub(crate) fn db(&self) -> &NamedDataMgrDB {
        &self.db
    }

    pub async fn set_mgr_by_id(
        named_data_mgr_id: Option<&str>,
        mgr: NamedDataMgr,
    ) -> NdnResult<()> {
        let named_data_mgr_key = named_data_mgr_id.unwrap_or("default").to_string();
        let mut named_data_mgr_map = NAMED_DATA_MGR_MAP.lock().await;
        named_data_mgr_map.insert(named_data_mgr_key, Arc::new(tokio::sync::Mutex::new(mgr)));
        Ok(())
    }

    pub async fn get_named_data_mgr_by_id(
        named_data_mgr_id: Option<&str>,
    ) -> Option<Arc<tokio::sync::Mutex<Self>>> {
        let named_mgr_key = named_data_mgr_id.unwrap_or("default").to_string();
        let mut named_data_mgr_map = NAMED_DATA_MGR_MAP.lock().await;

        let named_data_mgr = named_data_mgr_map.get(&named_mgr_key);
        if named_data_mgr.is_some() {
            debug!("NamedDataMgr: get named data mgr by id:{}", named_mgr_key);
            return Some(named_data_mgr.unwrap().clone());
        }

        info!(
            "NamedDataMgr: auto create new named data mgr for mgr_id:{}",
            named_mgr_key
        );
        let root_path = get_buckyos_named_data_dir(named_mgr_key.as_str());
        //make sure the root path dir exists
        if !root_path.exists() {
            fs::create_dir_all(root_path.clone()).await.unwrap();
        }
        let mgr_config;
        let mgr_json_file = root_path.join("ndn_mgr.json");
        if !mgr_json_file.exists() {
            mgr_config = NamedDataMgrConfig {
                local_stores: vec!["./".to_string()],
                local_cache: None,
                mmap_cache_dir: None,
            };

            let mgr_json_str = serde_json::to_string(&mgr_config).unwrap();
            let mut mgr_json_file = File::create(mgr_json_file.clone()).await.unwrap();
            mgr_json_file
                .write_all(mgr_json_str.as_bytes())
                .await
                .unwrap();
        } else {
            let mgr_json_str = fs::read_to_string(mgr_json_file).await;
            if mgr_json_str.is_err() {
                warn!(
                    "NamedDataMgr: read mgr config failed! {}",
                    mgr_json_str.err().unwrap().to_string()
                );
                return None;
            }
            let mgr_json_str = mgr_json_str.unwrap();
            let mgr_config_result = serde_json::from_str::<NamedDataMgrConfig>(&mgr_json_str);
            if mgr_config_result.is_err() {
                warn!(
                    "NamedDataMgr: parse mgr config failed! {}",
                    mgr_config_result.err().unwrap().to_string()
                );
                return None;
            }
            mgr_config = mgr_config_result.unwrap();
        }

        let result_mgr = Self::from_config(
            named_data_mgr_id.map(|s| s.to_string()),
            root_path,
            mgr_config,
        )
        .await;
        if result_mgr.is_err() {
            warn!(
                "NamedDataMgr: create mgr failed! {}",
                result_mgr.err().unwrap().to_string()
            );
            return None;
        }
        let result_mgr = Arc::new(tokio::sync::Mutex::new(result_mgr.unwrap()));
        named_data_mgr_map.insert(named_mgr_key, result_mgr.clone());
        return Some(result_mgr);
    }

    pub async fn from_config(
        mgr_id: Option<String>,
        root_path: PathBuf,
        config: NamedDataMgrConfig,
    ) -> NdnResult<Self> {
        let db_path = root_path.join("ndn_mgr.db").to_str().unwrap().to_string();
        let db = NamedDataMgrDB::new(db_path)?;
        let mut local_store_list = vec![];
        for local_store_path in config.local_stores.iter() {
            let real_local_store_path;
            if local_store_path.starts_with(".") {
                real_local_store_path = root_path.join(local_store_path);
            } else {
                real_local_store_path = PathBuf::from(local_store_path);
            }
            let local_store =
                NamedDataStore::new(real_local_store_path.to_str().unwrap().to_string()).await?;
            local_store_list.push(local_store);
        }
        let local_cache;
        if config.local_cache.is_some() {
            local_cache =
                Some(NamedDataStore::new(config.local_cache.as_ref().unwrap().clone()).await?);
        } else {
            local_cache = None;
        }
        Ok(Self {
            local_store_list: local_store_list,
            local_cache: local_cache,
            mmap_cache_dir: config.mmap_cache_dir,
            mgr_id: mgr_id,
            db: db,
        })
    }

    fn get_cache_mmap_path(&self, chunk_id: &ChunkId) -> Option<String> {
        None
    }

    pub fn get_cache_path_obj(&self, url: &str) -> Option<PathObject> {
        None
    }

    pub fn update_cache_path_obj(&self, url: &str, path_obj: PathObject) -> NdnResult<()> {
        Ok(())
    }

    //return path_obj_jwt
    async fn get_path_obj(&self, path: &str) -> NdnResult<Option<String>> {
        let (_obj_id, path_obj_jwt) = self.db.get_path_target_objid(path)?;
        if path_obj_jwt.is_some() {
            return Ok(Some(path_obj_jwt.unwrap()));
        }
        Ok(None)
    }

    pub async fn get_obj_id_by_path_impl(&self, path: &str) -> NdnResult<(ObjId, Option<String>)> {
        let (obj_id, path_obj_jwt) = self.db.get_path_target_objid(path)?;
        //info!("get_obj_id_by_path_impl: path:{},obj_id:{}",path,obj_id.to_string());
        Ok((obj_id, path_obj_jwt))
    }

    pub async fn get_obj_id_by_path(
        mgr_id: Option<&str>,
        path: &str,
    ) -> NdnResult<(ObjId, Option<String>)> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.get_obj_id_by_path_impl(path).await
    }

    //返回obj_id,path_obj_jwt和relative_path(如有)
    pub async fn select_obj_id_by_path_impl(
        &self,
        path: &str,
    ) -> NdnResult<(ObjId, Option<String>, Option<String>)> {
        let (root_path, obj_id, path_obj_jwt, relative_path) =
            self.db.find_longest_matching_path(path)?;
        Ok((obj_id, path_obj_jwt, relative_path))
    }

    pub async fn select_obj_id_by_path(
        mgr_id: Option<&str>,
        path: &str,
    ) -> NdnResult<(ObjId, Option<String>, Option<String>)> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.select_obj_id_by_path_impl(path).await
    }

    pub async fn get_object_impl(
        &self,
        obj_id: &ObjId,
        inner_obj_path: Option<String>,
    ) -> NdnResult<serde_json::Value> {
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
            let obj_body = obj_body.to_json_value().map_err(|e| {
                warn!("get_object: decode obj body failed! {}", e.to_string());
                NdnError::DecodeError(e.to_string())
            })?;

            if inner_obj_path.is_some() {
                let obj_path = inner_obj_path.unwrap();
                let obj_filed = get_by_json_path(&obj_body, &obj_path);
                if obj_filed.is_some() {
                    return Ok(obj_filed.unwrap());
                } else {
                    return Ok(serde_json::Value::Null);
                }
            } else {
                return Ok(obj_body);
            }
        }

        Err(NdnError::NotFound(obj_id.to_string()))
    }

    pub async fn get_object(
        mgr_id: Option<&str>,
        obj_id: &ObjId,
        inner_obj_path: Option<String>,
    ) -> NdnResult<serde_json::Value> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.get_object_impl(obj_id, inner_obj_path).await
    }

    pub async fn put_object_impl(&self, obj_id: &ObjId, obj_data: &str) -> NdnResult<()> {
        for local_store in self.local_store_list.iter() {
            //TODO: select best local store to write?
            local_store.put_object(obj_id, obj_data, false).await?;
            break;
        }
        Ok(())
    }

    pub async fn put_object(mgr_id: Option<&str>, obj_id: &ObjId, obj_data: &str) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.put_object_impl(obj_id, obj_data).await
    }

    pub async fn get_chunk_reader_by_path_impl(
        &self,
        path: &str,
        user_id: &str,
        app_id: &str,
        seek_from: SeekFrom,
    ) -> NdnResult<(ChunkReader, u64, ChunkId)> {
        let obj_id = self.db.get_path_target_objid(path)?;

        // Check if obj_id is a valid chunk id
        if !obj_id.0.is_chunk() {
            warn!(
                "get_chunk_reader_by_path: obj_id is not a chunk_id:{}",
                obj_id.0.to_string()
            );
            return Err(NdnError::InvalidParam(format!(
                "obj_id is not a chunk_id:{}",
                obj_id.0.to_string()
            )));
        }

        let chunk_id = ChunkId::from_obj_id(&obj_id.0);
        let (chunk_reader, chunk_size) = self
            .open_chunk_reader_impl(&chunk_id, seek_from, true)
            .await?;
        //let access_time = buckyos_get_unix_timestamp();
        //self.db.update_obj_access_time(&obj_id.0, access_time)?;
        Ok((chunk_reader, chunk_size, chunk_id))
    }

    pub async fn get_chunk_reader_by_path(
        mgr_id: Option<&str>,
        path: &str,
        user_id: &str,
        app_id: &str,
        seek_from: SeekFrom,
    ) -> NdnResult<(ChunkReader, u64, ChunkId)> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr
            .get_chunk_reader_by_path_impl(path, user_id, app_id, seek_from)
            .await
    }

    pub async fn create_file_impl(
        &self,
        path: &str,
        obj_id: &ObjId,
        app_id: &str,
        user_id: &str,
    ) -> NdnResult<()> {
        self.db
            .create_path(obj_id, path, app_id, user_id)
            .map_err(|e| {
                warn!("create_file: create path failed! {}", e.to_string());
                e
            })?;
        info!("create ndn path:{} ==> {}", path, obj_id.to_string());
        Ok(())
    }

    pub async fn create_file(
        mgr_id: Option<&str>,
        path: &str,
        obj_id: &ObjId,
        app_id: &str,
        user_id: &str,
    ) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr
            .create_file_impl(path, obj_id, app_id, user_id)
            .await
    }

    pub async fn set_file_impl(
        &self,
        path: &str,
        new_obj_id: &ObjId,
        app_id: &str,
        user_id: &str,
    ) -> NdnResult<()> {
        let path_obj = PathObject::new(path.to_string(), new_obj_id.clone());
        let path_obj_str = serde_json::to_string(&path_obj).unwrap();
        self.db
            .set_path(path, &new_obj_id, path_obj_str, app_id, user_id)
            .map_err(|e| {
                warn!("update_file: update path failed! {}", e.to_string());
                e
            })?;
        info!("update ndn path:{} ==> {}", path, new_obj_id.to_string());
        Ok(())
    }

    pub async fn set_file(
        mgr_id: Option<&str>,
        path: &str,
        new_obj_id: &ObjId,
        app_id: &str,
        user_id: &str,
    ) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr
            .set_file_impl(path, new_obj_id, app_id, user_id)
            .await
    }

    pub async fn remove_file_impl(&self, path: &str) -> NdnResult<()> {
        self.db.remove_path(path).map_err(|e| {
            warn!("remove_file: remove path failed! {}", e.to_string());
            e
        })?;
        info!("remove ndn path:{}", path);
        Ok(())

        //TODO: 这里不立刻删除chunk,而是等统一的GC来删除
    }

    pub async fn remove_file(mgr_id: Option<&str>, path: &str) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.remove_file_impl(path).await
    }

    pub async fn remove_dir_impl(&self, path: &str) -> NdnResult<()> {
        self.db.remove_dir_path(path).map_err(|e| {
            warn!("remove_dir: remove dir path failed! {}", e.to_string());
            e
        })?;
        info!("remove ndn dir path:{}", path);
        Ok(())
    }

    pub async fn remove_dir(mgr_id: Option<&str>, path: &str) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.remove_dir_impl(path).await
    }

    pub async fn is_chunk_exist_impl(&self, chunk_id: &ChunkId) -> NdnResult<bool> {
        for local_store in self.local_store_list.iter() {
            let (is_exist, chunk_size) = local_store.is_chunk_exist(chunk_id, None).await?;
            if is_exist {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn have_chunk(chunk_id: &ChunkId, mgr_id: Option<&str>) -> bool {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return false;
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        let is_exist = named_mgr.is_chunk_exist_impl(chunk_id).await;
        if is_exist.is_err() {
            return false;
        }
        is_exist.unwrap()
    }

    pub async fn query_chunk_state_impl(
        &self,
        chunk_id: &ChunkId,
    ) -> NdnResult<(ChunkState, u64, String)> {
        for local_store in self.local_store_list.iter() {
            let (chunk_state, chunk_size, progress) =
                local_store.query_chunk_state(chunk_id).await?;
            if chunk_state != ChunkState::NotExist {
                return Ok((chunk_state, chunk_size, progress));
            }
        }
        Ok((ChunkState::NotExist, 0, "".to_string()))
    }

    pub async fn query_chunk_state(
        mgr_id: Option<&str>,
        chunk_id: &ChunkId,
    ) -> NdnResult<(ChunkState, u64, String)> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.query_chunk_state_impl(chunk_id).await
    }

    pub async fn open_chunk_reader_impl(
        &self,
        chunk_id: &ChunkId,
        seek_from: SeekFrom,
        auto_cache: bool,
    ) -> NdnResult<(ChunkReader, u64)> {
        // memroy cache ==> local disk cache ==> local store
        //at first ,do access control
        let mcache_file_path = self.get_cache_mmap_path(chunk_id);
        if mcache_file_path.is_some() {
            let mcache_file_path = mcache_file_path.unwrap();

            let mut file = OpenOptions::new()
                .read(true) // 设置只读模式
                .open(&mcache_file_path)
                .await;

            if file.is_ok() {
                let mut file = file.unwrap();
                let file_meta = file.metadata().await.unwrap();
                if seek_from != SeekFrom::Start(0) {
                    file.seek(seek_from).await.map_err(|e| {
                        warn!(
                            "get_chunk_reader: seek cache file failed! {}",
                            e.to_string()
                        );
                        NdnError::IoError(e.to_string())
                    })?;
                }
                //info!("get_chunk_reader:return tmpfs cache file:{}", mcache_file_path);
                return Ok((Box::pin(file), file_meta.len()));
            }
        }

        debug!("get_chunk_reader: CACHE MISS :{}", chunk_id.to_string());
        if self.local_cache.is_some() {
            let local_cache = self.local_cache.as_ref().unwrap();
            let local_reader = local_cache.open_chunk_reader(chunk_id, seek_from).await;
            if local_reader.is_ok() {
                info!(
                    "get_chunk_reader:return local cache file:{}",
                    chunk_id.to_string()
                );
                return local_reader;
            }
        }

        debug!("get_chunk_reader: no cache file:{}", chunk_id.to_string());
        for local_store in self.local_store_list.iter() {
            let local_reader = local_store.open_chunk_reader(chunk_id, seek_from).await;
            if local_reader.is_ok() {
                //TODO:将结果数据添加到自动cache管理中
                //caceh是完整的，还是可以支持部分？
                return local_reader;
            }
        }

        Err(NdnError::NotFound(chunk_id.to_string()))
    }

    pub async fn open_chunk_reader(
        mgr_id: Option<&str>,
        chunk_id: &ChunkId,
        seek_from: SeekFrom,
        auto_cache: bool,
    ) -> NdnResult<(ChunkReader, u64)> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!(
                "named data mgr {} not found",
                mgr_id.unwrap()
            )));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr
            .open_chunk_reader_impl(chunk_id, seek_from, auto_cache)
            .await
    }

    //return chunklist_reader,chunklist_size(total_size)
    pub async fn open_chunklist_reader(
        mgr_id: Option<&str>,
        chunklist_id: &ObjId,
        seek_from: SeekFrom,
        auto_cache: bool,
    ) -> NdnResult<(ChunkReader, u64)> {
        // 1. Get named data manager by id
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id)
            .await
            .ok_or_else(|| {
                NdnError::NotFound(format!(
                    "named data mgr {} not found",
                    mgr_id.unwrap_or("default")
                ))
            })?;

        // 2. Get chunklist object
        let obj_data = {
            let mgr = named_mgr.lock().await;
            mgr.get_object_impl(chunklist_id, None).await?
        };

        let chunk_list = ChunkList::open(obj_data).await?;
        let total_size = chunk_list.total_size();

        let reader = ChunkListReader::new(named_mgr, chunk_list, seek_from, auto_cache).await?;

        Ok((Box::pin(reader), total_size))
    }

    //return chunk_id,progress_info
    pub async fn open_chunk_writer_impl(
        &self,
        chunk_id: &ChunkId,
        chunk_size: u64,
        offset: u64,
    ) -> NdnResult<(ChunkWriter, String)> {
        let default_store = self.local_store_list.first().unwrap();
        let (writer, chunk_progress_info) = default_store
            .open_chunk_writer(chunk_id, chunk_size, offset)
            .await?;
        Ok((writer, chunk_progress_info))
    }

    pub async fn open_chunk_writer(
        mgr_id: Option<&str>,
        chunk_id: &ChunkId,
        chunk_size: u64,
        offset: u64,
    ) -> NdnResult<(ChunkWriter, String)> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr
            .open_chunk_writer_impl(chunk_id, chunk_size, offset)
            .await
    }

    pub async fn update_chunk_progress_impl(
        &self,
        chunk_id: &ChunkId,
        progress: String,
    ) -> NdnResult<()> {
        let default_store = self.local_store_list.first().unwrap();
        default_store
            .update_chunk_progress(chunk_id, progress)
            .await
    }

    pub async fn update_chunk_progress(
        mgr_id: Option<&str>,
        chunk_id: &ChunkId,
        progress: String,
    ) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr
            .update_chunk_progress_impl(chunk_id, progress)
            .await
    }

    pub async fn complete_chunk_writer_impl(&self, chunk_id: &ChunkId) -> NdnResult<()> {
        let default_store = self.local_store_list.first().unwrap();
        default_store.complete_chunk_writer(chunk_id).await
    }

    pub async fn complete_chunk_writer(mgr_id: Option<&str>, chunk_id: &ChunkId) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.complete_chunk_writer_impl(chunk_id).await
    }

    //下面是一些helper函数
    pub async fn pub_object_to_file(
        mgr_id: Option<&str>,
        will_pub_obj: serde_json::Value,
        obj_type: &str,
        ndn_path: &str,
        user_id: &str,
        app_id: &str,
    ) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let (obj_id, obj_str) = build_named_object_by_json(obj_type, &will_pub_obj);
        let mut named_mgr = named_mgr.lock().await;
        named_mgr.put_object_impl(&obj_id, &obj_str).await?;
        named_mgr
            .create_file_impl(ndn_path, &obj_id, app_id, user_id)
            .await?;
        Ok(())
    }

    pub async fn sign_obj(
        mgr_id: Option<&str>,
        will_sign_obj_id: ObjId,
        obj_jwt: String,
        user_id: &str,
        app_id: &str,
    ) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let mut named_mgr = named_mgr.lock().await;
        named_mgr
            .put_object_impl(&will_sign_obj_id, &obj_jwt)
            .await?;
        Ok(())
    }

    pub async fn sigh_path_obj_impl(&self, path: &str, path_obj_jwt: &str) -> NdnResult<()> {
        let path_obj_json: serde_json::Value = decode_jwt_claim_without_verify(path_obj_jwt)
            .map_err(|e| {
                warn!(
                    "sigh_path_obj: decode path obj jwt failed! {}",
                    e.to_string()
                );
                NdnError::DecodeError(e.to_string())
            })?;

        let path_obj: PathObject = serde_json::from_value(path_obj_json).map_err(|e| {
            warn!("sigh_path_obj: parse path obj failed! {}", e.to_string());
            NdnError::DecodeError(e.to_string())
        })?;

        if path_obj.path != path {
            return Err(NdnError::InvalidParam(format!(
                "path_obj.path != path:{}",
                path
            )));
        }

        self.db.set_path_obj_jwt(path, path_obj_jwt).map_err(|e| {
            warn!("sigh_path_obj: set path obj jwt failed! {}", e.to_string());
            e
        })?;
        Ok(())
    }

    pub async fn sigh_path_obj(
        mgr_id: Option<&str>,
        path: &str,
        path_obj_jwt: &str,
    ) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        named_mgr.sigh_path_obj_impl(path, path_obj_jwt).await
    }

    //会写入两个ndn_path
    pub async fn pub_local_file_as_fileobj(
        mgr_id: Option<&str>,
        local_file_path: &PathBuf,
        ndn_path: &str,
        ndn_content_path: &str,
        fileobj_template: &mut FileObject,
        user_id: &str,
        app_id: &str,
    ) -> NdnResult<()> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::NotFound(format!("named data mgr not found")));
        }
        let named_mgr = named_mgr.unwrap();
        //TODO：优化，边算边传，支持断点续传
        debug!(
            "start pub local_file_as_fileobj, local_file_path:{}",
            local_file_path.display()
        );
        let mut file_reader = tokio::fs::File::open(local_file_path).await.map_err(|e| {
            error!("open local_file_path failed, err:{}", e);
            NdnError::IoError(format!("open local_file_path failed, err:{}", e))
        })?;
        debug!("open local_file_path success");
        let mut chunk_hasher = ChunkHasher::new(None).unwrap();
        let chunk_type = chunk_hasher.hash_type.clone();

        file_reader.seek(SeekFrom::Start(0)).await;
        let (chunk_raw_id, chunk_size) = chunk_hasher
            .calc_from_reader(&mut file_reader)
            .await
            .unwrap();

        let chunk_id = ChunkId::mix_from_hash_result(chunk_size, &chunk_raw_id, chunk_type);
        info!(
            "pub_local_file_as_fileobj:calc chunk_id success,chunk_id:{},chunk_size:{}",
            chunk_id.to_string(),
            chunk_size
        );
        let real_named_mgr = named_mgr.lock().await;
        let is_exist = real_named_mgr.is_chunk_exist_impl(&chunk_id).await.unwrap();
        if !is_exist {
            let (mut chunk_writer, _) = real_named_mgr
                .open_chunk_writer_impl(&chunk_id, chunk_size, 0)
                .await?;
            drop(real_named_mgr);
            file_reader.seek(std::io::SeekFrom::Start(0)).await.unwrap();
            let copy_bytes = tokio::io::copy(&mut file_reader, &mut chunk_writer)
                .await
                .map_err(|e| {
                    error!(
                        "copy local_file {:?} to named-mgr failed, err:{}",
                        local_file_path, e
                    );
                    NdnError::IoError(format!("copy local_file to named-mgr failed, err:{}", e))
                })?;

            info!("pub_local_file_as_fileobj:copy local_file {:?} to named-mgr's chunk success,copy_bytes:{}", local_file_path, copy_bytes);
            let real_named_mgr = named_mgr.lock().await;
            real_named_mgr.complete_chunk_writer_impl(&chunk_id).await?;
        } else {
            drop(real_named_mgr);
        }

        fileobj_template.content = chunk_id.to_string();
        fileobj_template.size = chunk_size;
        fileobj_template.create_time = Some(buckyos_get_unix_timestamp());

        let (file_obj_id, file_obj_str) = fileobj_template.gen_obj_id();
        let chunk_obj_id = chunk_id.to_obj_id();
        let real_named_mgr = named_mgr.lock().await;
        real_named_mgr
            .put_object_impl(&file_obj_id, file_obj_str.as_str())
            .await?;
        real_named_mgr
            .set_file_impl(ndn_path, &file_obj_id, app_id, user_id)
            .await?;
        real_named_mgr
            .set_file_impl(ndn_content_path, &chunk_obj_id, app_id, user_id)
            .await?;
        Ok(())
    }

    pub async fn pub_local_file_as_chunk(
        mgr_id: Option<&str>,
        local_file_path: String,
        ndn_path: &str,
        user_id: &str,
        app_id: &str,
    ) -> NdnResult<()> {
        unimplemented!()
    }
}

pub type NamedDataMgrRef = Arc<tokio::sync::Mutex<NamedDataMgr>>;

lazy_static! {
    pub static ref NAMED_DATA_MGR_MAP: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<NamedDataMgr>>>>> =
        { Arc::new(tokio::sync::Mutex::new(HashMap::new())) };
}
