use super::def::{ChunkItem, ObjectState, ChunkState};
use super::named_data_db::NamedDataDb;
use crate::{
    ChunkHasher, ChunkId, ChunkReader, ChunkWriter, LinkData, NdnError, NdnResult, ObjId,
    ObjectLink,
};
use async_trait::async_trait;
use log::*;
use name_lib::EncodedDocument;
use rusqlite::types::{FromSql, ToSql, ValueRef};
use rusqlite::{params, Connection, Result as SqliteResult};
use serde_json::json;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, io::SeekFrom};
use tokio::sync::Mutex;
use tokio::{
    fs::{self, File, OpenOptions},
    io::{self, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt},
};

// 每个Local Chunk Store基于一个目录独立存在
// Chunk Manage由多个Local Chunk Store组成(目前版本先搞定单OOD)
pub struct NamedDataStore {
    pub store_id: String,
    pub store_desc: String,
    pub enable_symlink: bool, //是否启用符号链接，不同的文件系统对符号链接的支持不一样，默认不启用
    pub auto_add_to_db: bool, //是否自动将符合命名规范的chunkid添加到db中，默认不自动添加
    named_db: NamedDataDb,
    base_dir: String,
    read_only: bool,
}

impl NamedDataStore {
    pub async fn new(base_dir: String) -> NdnResult<Self> {
        // Create base directory if it doesn't exist
        tokio::fs::create_dir_all(&base_dir).await.map_err(|e| {
            warn!("NamedDataStore: create base dir failed! {}", e.to_string());
            NdnError::IoError(e.to_string())
        })?;

        let named_db_path = format!("{}/named_object_store.db", base_dir.clone());
        let named_db = NamedDataDb::new(named_db_path.clone())?;
        if !std::path::Path::new(&named_db_path).exists() {
            info!("NamedDataStore: Database file does not exist, creating it");
            tokio::fs::File::create(&named_db_path).await.map_err(|e| {
                warn!("NamedDataStore: create db file failed! {}", e.to_string());
                NdnError::IoError(e.to_string())
            })?;
            info!("NamedDataStore: Database file created successfully");
        }
        Ok(Self {
            store_id: "".to_string(),
            store_desc: "".to_string(),
            named_db,
            base_dir,
            enable_symlink: true,
            auto_add_to_db: true,
            read_only: false,
        })
    }

    fn get_chunk_path(&self, chunk_id: &ChunkId) -> String {
        //根据ChunkId的HashResult,产生一个三层的目录结构
        let hex_str = hex::encode(chunk_id.hash_result.clone());
        let dir1 = &hex_str[0..2];
        let dir2 = &hex_str[2..4];
        let file_name = &hex_str[4..];

        format!(
            "{}/{}/{}/{}.{}",
            self.base_dir, dir1, dir2, file_name, chunk_id.chunk_type.to_string()
        )
    }

    pub async fn is_object_exist(&self, obj_id: &ObjId) -> NdnResult<bool> {
        let obj_state = self.query_object_by_id(obj_id).await?;
        match obj_state {
            ObjectState::Exist => Ok(true),
            ObjectState::Link(_) => Ok(true),
            ObjectState::Object(_) => Ok(true),
            ObjectState::Reader(_, _) => Ok(true),
            _ => Ok(false),
        }
    }

    pub async fn query_object_by_id(&self, obj_id: &ObjId) -> NdnResult<ObjectState> {
        let real_obj_result = self.named_db.get_object(obj_id).await;
        if real_obj_result.is_ok() {
            let (obj_type, obj_str) = real_obj_result.unwrap();
            return Ok(ObjectState::Object(obj_str));
        }

        let link_obj_result = self.named_db.get_object_link(obj_id).await;
        if link_obj_result.is_ok() {
            let link_obj = link_obj_result.unwrap();
            let obj_link = LinkData::from_string(&link_obj)?;
            return Ok(ObjectState::Link(obj_link));
        }

        return Ok(ObjectState::NotExist);
    }

    pub async fn get_object(&self, obj_id: &ObjId) -> NdnResult<EncodedDocument> {
        let obj_state = self.query_object_by_id(obj_id).await?;
        match obj_state {
            ObjectState::Object(obj_str) => {
                let doc = EncodedDocument::from_str(obj_str).map_err(|e| {
                    warn!("get_object: decode object failed! {}", e.to_string());
                    NdnError::DecodeError(e.to_string())
                })?;
                Ok(doc)
            }
            ObjectState::Link(obj_link) => match obj_link {
                LinkData::SameAs(link_obj_id) => Box::pin(self.get_object(&link_obj_id)).await,
                _ => Err(NdnError::InvalidLink(format!(
                    "object link not supported! {}",
                    obj_id.to_string()
                ))),
            },
            _ => Err(NdnError::NotFound(format!(
                "object not found! {}",
                obj_id.to_string()
            ))),
        }
    }

    pub async fn put_object(
        &self,
        obj_id: &ObjId,
        obj_str: &str,
        need_verify: bool,
    ) -> NdnResult<()> {
        if need_verify {
            // Add verification logic here if needed
            let build_obj_id = crate::build_obj_id("sha256", obj_str);
            if obj_id.obj_hash != build_obj_id.obj_hash {
                return Err(NdnError::InvalidId(format!(
                    "object id not match! {}",
                    obj_id.to_string()
                )));
            }
        }
        self.named_db
            .set_object(obj_id, obj_id.obj_type.as_str(), obj_str)
            .await
    }

    pub async fn link_object(&self, obj_id: &ObjId, target_obj: &ObjId) -> NdnResult<()> {
        let link = LinkData::SameAs(target_obj.clone());
        self.named_db.set_object_link(obj_id, &link).await
    }

    pub async fn query_link_refs(&self, ref_obj_id: &ObjId) -> NdnResult<Vec<ObjId>> {
        let link_obj_ids = self.named_db.query_object_link_ref(ref_obj_id).await?;
        let mut ref_obj_ids = Vec::new();
        for link_obj_id in link_obj_ids {
            let ref_obj_id = ObjId::new(link_obj_id.as_str())?;
            ref_obj_ids.push(ref_obj_id);
        }
        Ok(ref_obj_ids)
    }

    async fn get_real_chunk_item(&self, link_data: LinkData) -> NdnResult<ChunkItem> {
        match link_data {
            LinkData::SameAs(link_obj_id) => {
                let real_chunk = ChunkId::from_obj_id(&link_obj_id);
                let real_chunk_item = self.named_db.get_chunk(&real_chunk).await;
                if real_chunk_item.is_ok() {
                    let real_chunk_item = real_chunk_item.unwrap();
                    return Ok(real_chunk_item);
                } else {
                    let link_obj = self.named_db.get_object_link(&link_obj_id).await;
                    if link_obj.is_ok() {
                        let link_obj = link_obj.unwrap();
                        let obj_link: LinkData = LinkData::from_string(&link_obj)?;
                        return Box::pin(self.get_real_chunk_item(obj_link)).await;
                    } else {
                        return Err(NdnError::NotFound(format!(
                            "real chunk not found! {}",
                            link_obj_id.to_string()
                        )));
                    }
                }
            }
            LinkData::PartOf(link_obj_id, range) => {
                unimplemented!();
            }
            _ => Err(NdnError::InvalidLink(format!(
                "link data not supported! {}",
                link_data.to_string()
            ))),
        }
    }

    async fn get_chunk_item(&self, chunk_id: &ChunkId) -> NdnResult<ChunkItem> {
        let chunk_item = self.named_db.get_chunk(chunk_id).await;
        if chunk_item.is_ok() {
            return Ok(chunk_item.unwrap());
        }

        let link_obj_result = self.named_db.get_object_link(&chunk_id.to_obj_id()).await;
        if link_obj_result.is_ok() {
            let link_obj = link_obj_result.unwrap();
            let obj_link = LinkData::from_string(&link_obj)?;
            return self.get_real_chunk_item(obj_link).await;
        }

        Err(NdnError::NotFound(format!(
            "chunk not found! {}",
            chunk_id.to_string()
        )))
    }

    //只有chunk完整准备好了，才是存在。写入到一半的chunk不会算存在。
    //通过get_chunk_state可以得到更准确的chunk状态
    pub async fn is_chunk_exist(
        &self,
        chunk_id: &ChunkId,
        is_auto_add: Option<bool>,
    ) -> NdnResult<(bool, u64)> {
        let chunk_state = self.query_chunk_by_id(chunk_id).await?;
        let (chunk_state, chunk_size) = chunk_state;
        match chunk_state {
            ChunkState::Completed => Ok((true, chunk_size)),
            ChunkState::Link(link_data) => {
                if chunk_size == 0 {
                    let real_chunk_item = self.get_real_chunk_item(link_data).await?;
                    return Ok((true, real_chunk_item.chunk_size));
                } else {
                    return Ok((true, chunk_size));
                }
            }
            _ => Ok((false, 0)),
        }
    }

    pub async fn query_chunk_state(
        &self,
        chunk_id: &ChunkId,
    ) -> NdnResult<(ChunkState, u64, String)> {
        let chunk_item_result = self.named_db.get_chunk(chunk_id).await;
        if chunk_item_result.is_ok() {
            let chunk_item = chunk_item_result.unwrap();
            return Ok((
                chunk_item.chunk_state,
                chunk_item.chunk_size,
                chunk_item.progress,
            ));
        } else {
            return Ok((ChunkState::NotExist, 0, "".to_string()));
        }
    }

    pub async fn query_chunk_by_id(&self, chunk_id: &ChunkId) -> NdnResult<(ChunkState, u64)> {
        let chunk_item_result = self.named_db.get_chunk(chunk_id).await;
        if chunk_item_result.is_ok() {
            let chunk_item = chunk_item_result.unwrap();
            return Ok((chunk_item.chunk_state, chunk_item.chunk_size));
        }

        let link_obj_result = self.named_db.get_object_link(&chunk_id.to_obj_id()).await;
        if link_obj_result.is_ok() {
            let link_obj = link_obj_result.unwrap();
            let obj_link = LinkData::from_string(&link_obj)?;
            let obj_link2 = obj_link.clone();
            match obj_link {
                LinkData::SameAs(link_obj_id) => {
                    return Ok((ChunkState::Link(obj_link2), 0));
                }
                LinkData::PartOf(link_obj_id, range) => {
                    return Ok((ChunkState::Link(obj_link2), range.end - range.start));
                }
                _ => {
                    warn!(
                        "query_chunk_by_id: link data not supported! {}",
                        chunk_id.to_string()
                    );
                    return Err(NdnError::InvalidLink(format!(
                        "link data not supported! {}",
                        chunk_id.to_string()
                    )));
                }
            }
        }

        return Ok((ChunkState::NotExist, 0));
    }

    //查询多个chunk的状态
    pub async fn query_chunk_state_by_list(
        &self,
        chunk_list: &mut Vec<ChunkItem>,
    ) -> NdnResult<()> {
        unimplemented!()
    }

    pub async fn open_chunk_reader(
        &self,
        chunk_id: &ChunkId,
        offset: SeekFrom,
    ) -> NdnResult<(ChunkReader, u64)> {
        let chunk_item = self.get_chunk_item(chunk_id).await?;
        if chunk_item.chunk_state != ChunkState::Completed {
            return Err(NdnError::InComplete(format!(
                "chunk not completed! {}",
                chunk_id.to_string()
            )));
        }
        let real_chunk_id = chunk_item.chunk_id;
        let chunk_size = chunk_item.chunk_size;

        let chunk_path = self.get_chunk_path(&real_chunk_id);
        let mut file = OpenOptions::new()
            .read(true) // 设置只读模式
            .open(&chunk_path)
            .await
            .map_err(|e| {
                warn!("open_chunk_reader: open file failed! {}", e.to_string());
                NdnError::IoError(e.to_string())
            })?;

        if offset != SeekFrom::Start(0) {
            file.seek(offset).await.map_err(|e| {
                warn!("open_chunk_reader: seek file failed! {}", e.to_string());
                NdnError::IoError(e.to_string())
            })?;
        }

        Ok((Box::pin(file), chunk_size))
    }

    //打开writer并允许writer已经存在
    pub async fn open_chunk_writer(
        &self,
        chunk_id: &ChunkId,
        chunk_size: u64,
        offset: u64,
    ) -> NdnResult<(ChunkWriter, String)> {
        let chunk_item = self.named_db.get_chunk(chunk_id).await;
        let chunk_path = self.get_chunk_path(chunk_id);
        if chunk_item.is_ok() {
            let chunk_item = chunk_item.unwrap();
            if chunk_item.chunk_state == ChunkState::Completed {
                warn!(
                    "open_chunk_writer: chunk completed! {} cannot write!",
                    chunk_id.to_string()
                );
                return Err(NdnError::AlreadyExists(format!(
                    "chunk completed! {} cannot write!",
                    chunk_id.to_string()
                )));
            }

            let file_meta = fs::metadata(&chunk_path).await.map_err(|e| {
                warn!("open_chunk_writer: get metadata failed! {}", e.to_string());
                NdnError::IoError(e.to_string())
            })?;

            if offset <= file_meta.len() {
                let mut file = OpenOptions::new()
                    .write(true)
                    .open(&chunk_path)
                    .await
                    .map_err(|e| {
                        warn!("open_chunk_writer: open file failed! {}", e.to_string());
                        NdnError::IoError(e.to_string())
                    })?;

                if offset != 0 {
                    file.seek(SeekFrom::Start(offset)).await.map_err(|e| {
                        warn!("open_chunk_writer: seek file failed! {}", e.to_string());
                        NdnError::IoError(e.to_string())
                    })?;
                } else {
                    file.seek(SeekFrom::End(0)).await.map_err(|e| {
                        warn!("open_chunk_writer: seek file failed! {}", e.to_string());
                        NdnError::IoError(e.to_string())
                    })?;
                }

                if chunk_item.progress.len() < 2 {
                    let progress = json!({
                        "pos":file_meta.len(),
                    })
                    .to_string();
                    return Ok((Box::pin(file), progress));
                }
                return Ok((Box::pin(file), chunk_item.progress));
            } else {
                warn!(
                    "open_chunk_writer: offset too large! {}",
                    chunk_id.to_string()
                );
                return Err(NdnError::OffsetTooLarge(chunk_id.to_string()));
            }
        } else {
            if offset != 0 {
                warn!("open_chunk_writer: offset not 0! {}", chunk_id.to_string());
                return Err(NdnError::Internal("offset not 0".to_string()));
            }
            // Create parent directories if they don't exist
            if let Some(parent) = std::path::Path::new(&chunk_path).parent() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    warn!("open_chunk_writer: create dir failed! {}", e.to_string());
                    NdnError::IoError(e.to_string())
                })?;
            }

            let file = File::create(&chunk_path).await.map_err(|e| {
                warn!("open_chunk_writer: create file failed! {}", e.to_string());
                NdnError::IoError(e.to_string())
            })?;

            //创建chunk_item
            let chunk_item = ChunkItem::new(&chunk_id, chunk_size, None);
            self.named_db.set_chunk_item(&chunk_item).await?;

            return Ok((Box::pin(file), "".to_string()));
        }
    }
    //打开writer,不允许writer已经存在
    pub async fn open_new_chunk_writer(
        &self,
        chunk_id: &ChunkId,
        chunk_size: u64,
    ) -> NdnResult<ChunkWriter> {
        let chunk_item = self.named_db.get_chunk(chunk_id).await;
        if chunk_item.is_ok() {
            return Err(NdnError::AlreadyExists(format!(
                "chunk already exists! {}",
                chunk_id.to_string()
            )));
        }
        let chunk_path = self.get_chunk_path(&chunk_id);

        // Create parent directories if they don't exist
        if let Some(parent) = std::path::Path::new(&chunk_path).parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                warn!(
                    "open_new_chunk_writer: create dir failed! {}",
                    e.to_string()
                );
                NdnError::IoError(e.to_string())
            })?;
        }

        let file = File::create(&chunk_path).await.map_err(|e| {
            warn!("open_chunk_writer: create file failed! {}", e.to_string());
            NdnError::IoError(e.to_string())
        })?;

        let chunk_item = ChunkItem::new(chunk_id, chunk_size, None);
        self.named_db.set_chunk_item(&chunk_item).await?;
        return Ok(Box::pin(file));
    }

    pub async fn update_chunk_progress(
        &self,
        chunk_id: &ChunkId,
        progress: String,
    ) -> NdnResult<()> {
        return self
            .named_db
            .update_chunk_progress(chunk_id, progress)
            .await;
    }

    //writer已经写入完成，此时可以进行一次可选的hash校验
    pub async fn complete_chunk_writer(&self, chunk_id: &ChunkId) -> NdnResult<()> {
        let mut chunk_item = self.named_db.get_chunk(chunk_id).await;
        if chunk_item.is_err() {
            return Err(NdnError::NotFound(format!(
                "chunk not found! {}",
                chunk_id.to_string()
            )));
        }
        let mut chunk_item = chunk_item.unwrap();
        chunk_item.chunk_state = ChunkState::Completed;
        chunk_item.progress = "".to_string();
        info!(
            "complete_chunk_writer: complete chunk {} success itemsize:{}",
            chunk_id.to_string(),
            chunk_item.chunk_size
        );
        self.named_db.set_chunk_item(&chunk_item).await?;
        Ok(())
    }

    //删除chunkid对应的文件,注意一定会带来文件的删除
    async fn remove_chunk_data(&self, chunk_list: Vec<ChunkId>) -> NdnResult<()> {
        for chunk_id in chunk_list {
            // Remove the physical file
            let chunk_path = self.get_chunk_path(&chunk_id);
            if let Err(e) = fs::remove_file(&chunk_path).await {
                warn!("Failed to remove chunk file {}: {}", chunk_path, e);
            }

            // Remove from database
            self.named_db.remove_chunk(&chunk_id).await?;
        }
        Ok(())
    }

    //=====================下面的都是helper函数了======================
    //针对小于1MB的 chunk,推荐直接返回内存
    pub async fn get_chunk_data(&self, chunk_id: &ChunkId) -> NdnResult<Vec<u8>> {
        let (mut chunk_reader, chunk_size) =
            self.open_chunk_reader(chunk_id, SeekFrom::Start(0)).await?;
        let mut buffer = Vec::with_capacity(chunk_size as usize);
        chunk_reader.read_to_end(&mut buffer).await.map_err(|e| {
            warn!("get_chunk_data: read file failed! {}", e.to_string());
            NdnError::IoError(e.to_string())
        })?;
        Ok(buffer)
    }

    pub async fn get_chunk_piece(
        &self,
        chunk_id: &ChunkId,
        offset: SeekFrom,
        piece_size: u32,
    ) -> NdnResult<Vec<u8>> {
        let (mut reader, chunk_size) = self.open_chunk_reader(chunk_id, offset).await?;
        let mut buffer = vec![0u8; piece_size as usize];
        reader.read_exact(&mut buffer).await.map_err(|e| {
            warn!("get_chunk_piece: read file failed! {}", e.to_string());
            NdnError::IoError(e.to_string())
        })?;
        Ok(buffer)
    }

    //一口气写入一组chunk(通常是小chunk)
    pub async fn put_chunklist(
        &self,
        chunk_list: HashMap<ChunkId, Vec<u8>>,
        need_verify: bool,
    ) -> NdnResult<()> {
        unimplemented!()
    }
    //写入一个在内存中的完整的chunk
    pub async fn put_chunk(
        &self,
        chunk_id: &ChunkId,
        chunk_data: &[u8],
        need_verify: bool,
    ) -> NdnResult<()> {
        if need_verify {
            let hash_method = chunk_id.chunk_type.to_hash_method()?;
            let mut chunk_hasher = ChunkHasher::new_with_hash_method(hash_method)?;
            let hash_bytes = chunk_hasher.calc_from_bytes(&chunk_data);
            if !chunk_id.equal(&hash_bytes) {
                warn!(
                    "put_chunk: chunk_id not equal hash_bytes! {}",
                    chunk_id.to_string()
                );
                return Err(NdnError::InvalidId(format!(
                    "chunk_id not equal hash_bytes! {}",
                    chunk_id.to_string()
                )));
            }
        }

        let mut chunk_writer = self
            .open_new_chunk_writer(chunk_id, chunk_data.len() as u64)
            .await?;
        chunk_writer.write_all(chunk_data).await.map_err(|e| {
            warn!("put_chunk: write file failed! {}", e.to_string());
            NdnError::IoError(e.to_string())
        })?;
        self.complete_chunk_writer(chunk_id).await?;

        Ok(())
    }
}
