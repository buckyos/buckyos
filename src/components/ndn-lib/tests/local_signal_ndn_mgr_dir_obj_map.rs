use std::{
    collections::HashMap,
    fmt::Display,
    io::SeekFrom,
    path::{Path, PathBuf},
    sync::Arc,
    u64, vec,
};

use buckyos_kit::*;
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use hex::ToHex;
use log::*;
use ndn_lib::*;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DirObject {
    pub name: String,
    pub content: String, //ObjectMapId
    #[serde(default)]
    #[serde(skip_serializing_if = "is_default")]
    pub exp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<u64>,
    #[serde(flatten)]
    pub extra_info: HashMap<String, Value>,
}

impl DirObject {
    pub fn gen_obj_id(&self) -> (ObjId, String) {
        build_named_object_by_json(
            OBJ_TYPE_DIR,
            &serde_json::to_value(self).expect("json::value from DirObject failed"),
        )
    }
}

#[derive(Clone)]
pub struct FileStorageItem {
    pub obj: FileObject,
    pub chunk_size: Option<u64>,
}

impl std::fmt::Debug for FileStorageItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FileStorageItem: {}", self.obj.name)
    }
}

#[derive(Clone, Debug)]
pub struct ChunkItem {
    pub seq: u64,          // sequence number in file
    pub offset: SeekFrom,  // offset in the file
    pub chunk_id: ChunkId, // chunk id
}

#[derive(Clone, Debug)]
pub enum StorageItem {
    Dir(DirObject),
    File(FileStorageItem),
    Chunk(ChunkItem), // (seq, offset, ChunkId)
}

#[derive(Debug)]
pub enum StorageItemName {
    Name(String),
    ChunkSeq(u64), // seq
}

impl StorageItemName {
    fn check_name(&self) -> &str {
        match self {
            StorageItemName::Name(name) => name.as_str(),
            StorageItemName::ChunkSeq(_) => panic!("expect name"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StorageItemNameRef<'a> {
    Name(&'a str),
    ChunkSeq(u64), // seq
}

impl<'a> StorageItemNameRef<'a> {
    fn check_name(&self) -> &str {
        match self {
            StorageItemNameRef::Name(name) => *name,
            StorageItemNameRef::ChunkSeq(_) => panic!("expect name"),
        }
    }
}

impl StorageItem {
    fn is_dir(&self) -> bool {
        matches!(self, StorageItem::Dir(_))
    }
    fn is_file(&self) -> bool {
        matches!(self, StorageItem::File(_))
    }
    fn is_chunk(&self) -> bool {
        matches!(self, StorageItem::Chunk(_))
    }
    fn check_dir(&self) -> &DirObject {
        match self {
            StorageItem::Dir(dir) => dir,
            _ => panic!("expect dir"),
        }
    }
    fn check_file(&self) -> &FileStorageItem {
        match self {
            StorageItem::File(file) => file,
            _ => panic!("expect file"),
        }
    }
    fn check_chunk(&self) -> &ChunkItem {
        match self {
            StorageItem::Chunk(chunk) => chunk,
            _ => panic!("expect chunk"),
        }
    }
    fn name(&self) -> StorageItemNameRef<'_> {
        match self {
            StorageItem::Dir(dir) => StorageItemNameRef::Name(dir.name.as_str()),
            StorageItem::File(file) => StorageItemNameRef::Name(file.obj.name.as_str()),
            StorageItem::Chunk(chunk_item) => StorageItemNameRef::ChunkSeq(chunk_item.seq),
        }
    }
    fn item_type(&self) -> &str {
        match self {
            StorageItem::Dir(_) => "dir",
            StorageItem::File(_) => "file",
            StorageItem::Chunk(_) => "chunk",
        }
    }
}

pub type PathDepth = u64;

#[derive(Clone)]
pub enum ItemStatus {
    New,
    Scanning,
    Hashing,
    Transfer(ObjId),
    Complete(ObjId),
}

impl std::fmt::Debug for ItemStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemStatus::New => write!(f, "New"),
            ItemStatus::Scanning => write!(f, "Scanning"),
            ItemStatus::Hashing => write!(f, "Hashing"),
            ItemStatus::Transfer(obj_id) => write!(f, "Transfer({})", obj_id.to_string()),
            ItemStatus::Complete(obj_id) => write!(f, "Complete({})", obj_id.to_string()),
        }
    }
}

impl ItemStatus {
    pub fn is_new(&self) -> bool {
        matches!(self, ItemStatus::New)
    }
    pub fn is_scanning(&self) -> bool {
        matches!(self, ItemStatus::Scanning)
    }
    pub fn is_hashing_or_transfer(&self) -> bool {
        matches!(self, ItemStatus::Hashing | ItemStatus::Transfer(_))
    }
    pub fn is_complete(&self) -> bool {
        matches!(self, ItemStatus::Complete(_))
    }
    pub fn is_hashing(&self) -> bool {
        matches!(self, ItemStatus::Hashing)
    }
    pub fn is_transfer(&self) -> bool {
        matches!(self, ItemStatus::Transfer(_))
    }

    pub fn get_obj_id(&self) -> Option<&ObjId> {
        match self {
            ItemStatus::Transfer(obj_id) | ItemStatus::Complete(obj_id) => Some(obj_id),
            _ => None,
        }
    }
}

pub trait StrorageCreator<S: Storage<ItemId = Self::ItemId>>:
    AsyncFn(String) -> NdnResult<S> + Send + Sync + Sized
{
    type ItemId: Send + Sync + Clone + std::fmt::Debug + Eq + std::hash::Hash + Sized;
}

#[async_trait::async_trait]
pub trait Storage: Send + Sync + Sized + Clone {
    type ItemId: Send + Sync + Clone + std::fmt::Debug + Eq + std::hash::Hash + Sized;
    async fn create_new_item(
        &self,
        item: &StorageItem,
        depth: PathDepth,
        parent_path: &Path,
        parent_item_id: Option<Self::ItemId>,
    ) -> NdnResult<(Self::ItemId, StorageItem, ItemStatus)>;
    async fn remove_dir(&self, item_id: &Self::ItemId) -> NdnResult<u64>;
    async fn remove_children(&self, item_id: &Self::ItemId) -> NdnResult<u64>;
    async fn begin_hash(&self, item_id: &Self::ItemId) -> NdnResult<()>;
    async fn begin_transfer(&self, item_id: &Self::ItemId, content: &ObjId) -> NdnResult<()>;
    async fn complete(&self, item_id: &Self::ItemId) -> NdnResult<()>;
    async fn get_root(
        &self,
    ) -> NdnResult<(Self::ItemId, StorageItem, PathBuf, ItemStatus, PathDepth)>;
    async fn complete_chilren_exclude(
        &self,
        item_id: &Self::ItemId,
        exclude_item_obj_ids: &[ObjId],
    ) -> NdnResult<Vec<Self::ItemId>>; // complete all children except the exclude items, return the completed item ids
    async fn get_item(
        &self,
        item_id: &Self::ItemId,
    ) -> NdnResult<(StorageItem, ItemStatus, PathDepth)>;
    async fn select_dir_scan_or_new(
        &self,
    ) -> NdnResult<Option<(Self::ItemId, DirObject, PathBuf, ItemStatus, PathDepth)>>; // to continue scan
    async fn select_file_hashing_or_transfer(
        &self,
    ) -> NdnResult<
        Option<(
            Self::ItemId,
            FileStorageItem,
            PathBuf,
            ItemStatus,
            PathDepth,
        )>,
    >; // to continue transfer file after all child-dir hash and all child-file complete
    async fn select_dir_hashing_with_all_child_dir_transfer_and_file_complete(
        &self,
    ) -> NdnResult<Option<(Self::ItemId, DirObject, PathBuf, ItemStatus, PathDepth)>>; // to continue transfer dir after all children complete
    async fn select_dir_transfer(
        &self,
        depth: Option<PathDepth>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> NdnResult<Vec<(Self::ItemId, DirObject, PathBuf, ItemStatus, PathDepth)>>; // to transfer dir
    async fn select_item_transfer(
        &self,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> NdnResult<Vec<(Self::ItemId, StorageItem, PathBuf, ItemStatus, PathDepth)>>; // to transfer
    async fn list_children_order_by_name(
        &self,
        item_id: &Self::ItemId,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> NdnResult<(
        Vec<(Self::ItemId, StorageItem, ItemStatus, PathDepth)>,
        PathBuf,
    )>;
    async fn list_chunks_by_chunk_id(
        &self,
        chunk_ids: &[ChunkId],
    ) -> NdnResult<Vec<(Self::ItemId, ChunkItem, PathBuf, ItemStatus, PathDepth)>>;
}

pub enum FileSystemItem {
    Dir(DirObject),
    File(FileObject),
}

impl FileSystemItem {
    fn name(&self) -> &str {
        match self {
            FileSystemItem::Dir(dir_object) => dir_object.name.as_str(),
            FileSystemItem::File(file_object) => file_object.name.as_str(),
        }
    }
}

#[async_trait::async_trait]
pub trait FileSystemReader<D: FileSystemDirReader, F: FileSystemFileReader>:
    Send + Sync + Sized + Clone
{
    async fn info(&self, path: &std::path::Path) -> NdnResult<FileSystemItem>;
    async fn open_dir(&self, path: &std::path::Path) -> NdnResult<D>;
    async fn open_file(&self, path: &std::path::Path) -> NdnResult<F>;
}

#[async_trait::async_trait]
pub trait FileSystemDirReader: Send + Sync + Sized {
    async fn next(&self, limit: Option<u64>) -> NdnResult<Vec<FileSystemItem>>;
}

#[async_trait::async_trait]
pub trait FileSystemFileReader: Send + Sync + Sized {
    async fn read_chunk(&self, offset: SeekFrom, limit: Option<u64>) -> NdnResult<Vec<u8>>;
}

#[async_trait::async_trait]
pub trait FileSystemWriter<W: FileSystemFileWriter>: Send + Sync + Sized {
    async fn create_dir_all(&self, dir_path: &Path) -> NdnResult<()>;
    async fn create_dir(&self, dir: &DirObject, parent_path: &Path) -> NdnResult<()>;
    async fn open_file(&self, file: &FileObject, parent_path: &Path) -> NdnResult<W>;
}

#[async_trait::async_trait]
pub trait FileSystemFileWriter: Send + Sync + Sized {
    async fn length(&self) -> NdnResult<u64>;
    async fn write_chunk(&self, chunk_data: &[u8], offset: SeekFrom) -> NdnResult<()>;
}

#[async_trait::async_trait]
pub trait NdnWriter: Send + Sync + Sized + Clone {
    async fn push_object(&self, obj_id: &ObjId, obj_str: &str) -> NdnResult<Vec<ObjId>>; // lost child-obj-list
    async fn push_chunk(&self, chunk_id: &ChunkId, chunk_data: &[u8]) -> NdnResult<()>;
    async fn push_container(&self, container_id: &ObjId) -> NdnResult<Vec<ObjId>>; // lost child-obj-list
}

#[async_trait::async_trait]
pub trait NdnReader: Send + Sync + Sized {
    async fn get_object(&self, obj_id: &ObjId) -> NdnResult<Value>;
    async fn get_chunk(&self, chunk_id: &ChunkId) -> NdnResult<Vec<u8>>;
    async fn get_container(&self, container_id: &ObjId) -> NdnResult<Value>;
}

pub async fn file_system_to_ndn<
    S: Storage + 'static,
    FDR: FileSystemDirReader + 'static,
    FFR: FileSystemFileReader + 'static,
    F: FileSystemReader<FDR, FFR> + 'static,
    N: NdnWriter + 'static,
>(
    path: Option<&Path>, // None for continue
    writer: N,
    reader: F,
    storage: S,
    chunk_size: u64,
    ndn_mgr_id: &str,
) -> NdnResult<S::ItemId> {
    let is_finish_scan = Arc::new(tokio::sync::Mutex::new(false));
    let (continue_hash_emiter, continue_hash_handler) = tokio::sync::mpsc::channel::<()>(64);

    let scan_task_process = async |path: Option<&Path>,
                                   reader: F,
                                   storage: S,
                                   chunk_size: u64,
                                   emiter: tokio::sync::mpsc::Sender<()>|
           -> NdnResult<()> {
        let insert_new_item = async |item: FileSystemItem,
                                     parent_path: &Path,
                                     storage: &S,
                                     depth: u64,
                                     parent_item_id: Option<S::ItemId>|
               -> NdnResult<S::ItemId> {
            let item_id = match item {
                FileSystemItem::Dir(dir_object) => {
                    let (item_id, _, _) = storage
                        .create_new_item(
                            &StorageItem::Dir(dir_object),
                            depth,
                            parent_path,
                            parent_item_id,
                        )
                        .await?;
                    item_id
                }
                FileSystemItem::File(file_object) => {
                    let file_path = parent_path.join(file_object.name.as_str());
                    let file_size = file_object.size;
                    let file_reader = reader
                        .open_file(parent_path.join(file_object.name.as_str()).as_path())
                        .await?;
                    let (file_item_id, file_item, _item_status) = storage
                        .create_new_item(
                            &StorageItem::File(FileStorageItem {
                                obj: file_object,
                                chunk_size: Some(chunk_size),
                            }),
                            depth,
                            parent_path,
                            parent_item_id.clone(),
                        )
                        .await?;
                    let chunk_size = match file_item {
                        StorageItem::File(file_storage_item) => {
                            file_storage_item.chunk_size.unwrap_or(chunk_size)
                        }
                        _ => Err(NdnError::InvalidObjType(format!(
                            "expect file, got: {:?} in history storage",
                            file_item.item_type()
                        )))?,
                    };
                    for i in 0..(file_size + chunk_size - 1) / chunk_size {
                        let offet = SeekFrom::Start(i * chunk_size);
                        let chunk_data = file_reader.read_chunk(offet, Some(chunk_size)).await?;
                        let hasher = ChunkHasher::new(None).expect("hash failed.");
                        let hash = hasher.calc_from_bytes(&chunk_data);
                        let chunk_id = ChunkId::from_mix_hash_result_by_hash_method(
                            chunk_data.len() as u64,
                            &hash,
                            HashMethod::Sha256,
                        )?;
                        let (chunk_item_id, _, _) = storage
                            .create_new_item(
                                &StorageItem::Chunk(ChunkItem {
                                    seq: i as u64,
                                    offset: SeekFrom::Start(i * chunk_size),
                                    chunk_id: chunk_id.clone(),
                                }),
                                depth + 1,
                                file_path.as_path(),
                                Some(file_item_id.clone()),
                            )
                            .await?;
                        storage
                            .begin_transfer(&chunk_item_id, &chunk_id.to_obj_id())
                            .await?;
                    }

                    storage.begin_hash(&file_item_id).await?;
                    file_item_id
                }
            };
            Ok(item_id)
        };

        if let Some(path) = path {
            let none_path = PathBuf::from("");
            let fs_item = reader.info(path).await?;
            insert_new_item(
                fs_item,
                path.parent().unwrap_or(none_path.as_path()),
                &storage,
                0,
                None,
            )
            .await?;
        }

        let _ = emiter.send(()).await;

        loop {
            match storage.select_dir_scan_or_new().await? {
                Some((item_id, dir_object, parent_path, _item_status, depth)) => {
                    info!("scan dir: {}, item_id: {:?}", dir_object.name, item_id);
                    let dir_path = parent_path.join(dir_object.name);
                    let dir_reader = reader.open_dir(dir_path.as_path()).await?;

                    loop {
                        let items = dir_reader.next(Some(64)).await?;
                        let is_finish = items.len() < 64;
                        for item in items {
                            insert_new_item(
                                item,
                                dir_path.as_path(),
                                &storage,
                                depth + 1,
                                Some(item_id.clone()),
                            )
                            .await?;

                            let _ = emiter.send(()).await;
                        }
                        if is_finish {
                            break;
                        }
                    }
                    storage.begin_hash(&item_id).await?;
                }
                None => break,
            }
        }

        Ok(())
    };

    let scan_task = {
        let path = path.map(|p| p.to_path_buf());
        let reader = reader.clone();
        let storage = storage.clone();
        let is_finish_scan = is_finish_scan.clone();
        let emiter = continue_hash_emiter.clone();
        tokio::spawn(async move {
            let ret = scan_task_process(
                path.as_ref().map(|p| p.as_path()),
                reader,
                storage,
                chunk_size,
                emiter.clone(),
            )
            .await;
            *is_finish_scan.lock().await = true;
            let _ = emiter.send(()).await;
            ret
        })
    };

    let transfer_task_process = async |storage: S,
                                       reader: F,
                                       writer: N,
                                       is_finish_scan: Arc<tokio::sync::Mutex<bool>>,
                                       mut continue_hash_handler: tokio::sync::mpsc::Receiver<
        (),
    >,
                                       ndn_mgr_id: &str|
           -> NdnResult<()> {
        let mut is_hash_finish_pending = false;
        loop {
            loop {
                // hash files
                match storage.select_file_hashing_or_transfer().await? {
                    Some((item_id, mut file_item, parent_path, file_status, depth)) => {
                        info!("hashing file: {:?}, status: {:?}", item_id, file_status);
                        let file_path = parent_path.join(file_item.obj.name.as_str());
                        let (file_obj_id, file_obj_str, file_chunk_list_id, _file_chunk_list_str) =
                            if file_status.is_hashing() {
                                assert!(
                                    file_item.obj.content.is_empty(),
                                    "file content should be empty for before hashing."
                                );

                                let mut chunk_list_builder =
                                    ChunkListBuilder::new(HashMethod::Sha256)
                                        .with_total_size(file_item.obj.size);
                                let mut chunk_count_in_list = 0;
                                if let Some(chunk_size) = file_item.chunk_size {
                                    chunk_list_builder =
                                        chunk_list_builder.with_fixed_size(chunk_size);
                                }

                                let chunk_size = file_item
                                    .chunk_size
                                    .expect("chunk size should be fix for file");
                                let chunk_count =
                                    (file_item.obj.size + chunk_size - 1) / chunk_size;
                                let mut batch_count = 0;
                                let batch_limit = 64;
                                loop {
                                    let (chunk_items, _) = storage
                                        .list_children_order_by_name(
                                            &item_id,
                                            Some(batch_count * batch_limit),
                                            Some(batch_limit),
                                        )
                                        .await?;
                                    batch_count += 1;
                                    let is_file_ready = (chunk_items.len() as u64) < batch_limit;
                                    for (_, chunk_item, chunk_status, chunk_depth) in chunk_items {
                                        let chunk_seq = chunk_item.check_chunk().seq;
                                        assert_eq!(
                                            chunk_depth,
                                            depth + 1,
                                            "chunk depth should be one more than file depth."
                                        );
                                        assert!(
                                            chunk_status.is_transfer(),
                                            "chunk item status should be Hashing, but: {:?}.",
                                            chunk_status
                                        );
                                        // expect chunk item
                                        match chunk_item {
                                            StorageItem::Chunk(chunk_item) => {
                                                assert!(
                                                    (chunk_seq == chunk_count - 1
                                                        && chunk_item
                                                            .chunk_id
                                                            .get_length()
                                                            .expect("chunk id should fix size")
                                                            == file_item.obj.size % chunk_size)
                                                        || chunk_item
                                                            .chunk_id
                                                            .get_length()
                                                            .expect("chunk id should fix size")
                                                            == chunk_size
                                                );
                                                assert_eq!(chunk_item.seq, chunk_count_in_list);
                                                assert_eq!(
                                                    chunk_item.offset,
                                                    SeekFrom::Start(
                                                        chunk_count_in_list
                                                            * file_item.chunk_size.unwrap_or(0)
                                                    )
                                                );
                                                chunk_list_builder
                                                    .append(chunk_item.chunk_id.clone())
                                                    .expect("add chunk failed");
                                                chunk_count_in_list += 1;
                                            }
                                            _ => {
                                                unreachable!(
                                                "expect chunk item, got: {:?} in history storage",
                                                chunk_item.item_type()
                                            );
                                            }
                                        }
                                    }
                                    if is_file_ready {
                                        break;
                                    }
                                }

                                let file_chunk_list = chunk_list_builder.build().await?;
                                let (file_chunk_list_id, file_chunk_list_str) =
                                    file_chunk_list.calc_obj_id();
                                NamedDataMgr::put_object(
                                    Some(ndn_mgr_id),
                                    &file_chunk_list_id,
                                    file_chunk_list_str.as_str(),
                                )
                                .await?;

                                file_item.obj.content = file_chunk_list_id.to_string();
                                let (file_obj_id, file_obj_str) = file_item.obj.gen_obj_id();
                                NamedDataMgr::put_object(
                                    Some(ndn_mgr_id),
                                    &file_obj_id,
                                    file_obj_str.as_str(),
                                )
                                .await?;
                                storage.begin_transfer(&item_id, &file_obj_id).await?;

                                (
                                    file_obj_id,
                                    file_obj_str,
                                    file_chunk_list_id,
                                    file_chunk_list_str,
                                )
                            } else if file_status.is_transfer() {
                                info!("transfer file: {:?}, status: {:?}", item_id, file_status);
                                let file_obj = NamedDataMgr::get_object(
                                    Some(ndn_mgr_id),
                                    file_status
                                        .get_obj_id()
                                        .expect("file status should have obj id for transfer."),
                                    None,
                                )
                                .await?;
                                let (file_obj_id, file_obj_str) =
                                    build_named_object_by_json(OBJ_TYPE_FILE, &file_obj);
                                let file_obj: FileObject = serde_json::from_value(file_obj)
                                    .expect("file object should be valid json value.");
                                assert_eq!(
                                    file_status.get_obj_id(),
                                    Some(&file_obj_id),
                                    "file content should be set to chunk list id before transfer."
                                );

                                let file_chunk_list_id = ObjId::try_from(file_obj.content.as_str())
                                    .expect("file content should be a valid ObjId for chunk-list.");

                                let chunk_list_json_value = NamedDataMgr::get_object(
                                    Some(ndn_mgr_id),
                                    &file_chunk_list_id,
                                    None,
                                )
                                .await?;
                                let (chunk_list_id, chunk_list_str) = build_named_object_by_json(
                                    file_chunk_list_id.obj_type.as_str(),
                                    &chunk_list_json_value,
                                );
                                assert_eq!(
                                    chunk_list_id, file_chunk_list_id,
                                    "chunk list id should match the file content id."
                                );

                                (
                                    file_obj_id,
                                    file_obj_str,
                                    file_chunk_list_id,
                                    chunk_list_str,
                                )
                            } else {
                                unreachable!(
                                    "item status should be Hashing or Transfer, got: {:?}",
                                    file_status
                                );
                            };

                        let (lost_obj_ids) =
                            writer.push_object(&file_obj_id, &file_obj_str).await?;
                        if let Some(lost_obj_id) = lost_obj_ids.get(0) {
                            debug!(
                                "lost child objects when push file object: {}, lost: {:?}",
                                file_obj_id, lost_obj_ids
                            );
                            assert_eq!(lost_obj_id, &file_chunk_list_id);
                            let lost_chunk_ids = writer.push_container(&file_chunk_list_id).await?;
                            let limit = 16;
                            for i in 0..(lost_chunk_ids.len() + limit - 1) / limit {
                                let chunk_ids = &lost_chunk_ids[i * limit
                                    ..std::cmp::min((i + 1) * limit, lost_chunk_ids.len())];
                                let chunk_ids = chunk_ids
                                    .iter()
                                    .map(|id| ChunkId::from_obj_id(id))
                                    .collect::<Vec<_>>();
                                let chunk_items = storage
                                    .list_chunks_by_chunk_id(chunk_ids.as_slice())
                                    .await?;
                                assert_eq!(
                                    chunk_items.len(),
                                    chunk_ids.len(),
                                    "chunk items should match the chunk ids."
                                );
                                for ((_, chunk_item, chunk_file_path, _, _), chunk_id) in
                                    chunk_items.iter().zip(chunk_ids.iter())
                                {
                                    assert_eq!(
                                        chunk_file_path, &file_path,
                                        "chunk file path should match the file path."
                                    );
                                    assert_eq!(
                                        chunk_item.chunk_id, *chunk_id,
                                        "chunk item id should match the chunk id."
                                    );
                                    let chunk_reader =
                                        reader.open_file(file_path.as_path()).await?;
                                    let chunk_data = chunk_reader
                                        .read_chunk(
                                            chunk_item.offset,
                                            Some(
                                                chunk_item
                                                    .chunk_id
                                                    .get_length()
                                                    .expect("chunk id should have length"),
                                            ),
                                        )
                                        .await?;
                                    writer.push_chunk(chunk_id, chunk_data.as_slice()).await?;
                                }
                            }
                            if lost_chunk_ids.len() > 0 {
                                let lost_chunk_ids =
                                    writer.push_container(&file_chunk_list_id).await?;
                                assert!(
                                    lost_chunk_ids.is_empty(),
                                    "lost chunk ids should be empty after push container."
                                );
                            }
                            let lost_chunk_list =
                                writer.push_object(&file_obj_id, &file_obj_str).await?;
                            assert!(
                                lost_chunk_list.is_empty(),
                                "lost chunk list should be empty after push object."
                            );
                        }

                        storage.complete(&item_id).await?;

                        info!("transfer file: {:?}, status: {:?}", item_id, file_status);
                    }
                    None => {
                        break;
                    }
                }
            }

            loop {
                // hash dirs
                match storage
                    .select_dir_hashing_with_all_child_dir_transfer_and_file_complete()
                    .await?
                {
                    Some((item_id, mut dir_object, _parent_path, item_status, depth)) => {
                        info!("hashing dir: {:?}, status: {:?}", item_id, item_status);
                        assert!(item_status.is_hashing());

                        let mut dir_obj_map_builder =
                            TrieObjectMapBuilder::new(HashMethod::Sha256, None).await?;

                        let mut dir_children_batch_index = 0;
                        let dir_children_batch_limit = 64;

                        loop {
                            let (children, _parent_path) = storage
                                .list_children_order_by_name(
                                    &item_id,
                                    Some(dir_children_batch_index * dir_children_batch_limit),
                                    Some(dir_children_batch_limit),
                                )
                                .await?;
                            dir_children_batch_index += 1;
                            let is_dir_ready = (children.len() as u64) < dir_children_batch_limit;
                            for (child_item_id, child_item, child_status, child_depth) in children {
                                assert_eq!(
                                    child_depth,
                                    depth + 1,
                                    "child item depth should be one more than dir depth."
                                );

                                match child_item {
                                    StorageItem::Dir(dir_obj) => {
                                        assert!(
                                            child_status.is_transfer(),
                                            "child dir item should be complete, got: {:?}",
                                            child_status
                                        );
                                        dir_obj_map_builder.put_object(
                                            dir_obj.name.as_str(),
                                            child_status
                                                .get_obj_id()
                                                .expect("child dir item should have obj id."),
                                        )?;
                                    }
                                    StorageItem::File(file_item) => {
                                        assert!(
                                            child_status.is_complete(),
                                            "child file item should be complete, got: {:?}",
                                            child_status
                                        );
                                        dir_obj_map_builder.put_object(
                                            file_item.obj.name.as_str(),
                                            child_status
                                                .get_obj_id()
                                                .expect("child file item should have obj id."),
                                        )?;
                                    }
                                    StorageItem::Chunk(_) => {
                                        unreachable!("should not have chunk item in dir hashing.");
                                    }
                                }
                            }
                            if is_dir_ready {
                                break;
                            }
                        }

                        let dir_obj_map = dir_obj_map_builder.build().await?;
                        let (dir_obj_map_id, dir_obj_map_str) = dir_obj_map.calc_obj_id();

                        NamedDataMgr::put_object(
                            Some(ndn_mgr_id),
                            &dir_obj_map_id,
                            dir_obj_map_str.as_str(),
                        )
                        .await?;

                        dir_object.content = dir_obj_map_id.to_string();
                        let (dir_obj_id, dir_obj_str) = dir_object.gen_obj_id();
                        NamedDataMgr::put_object(
                            Some(ndn_mgr_id),
                            &dir_obj_id,
                            dir_obj_str.as_str(),
                        )
                        .await?;
                        storage.begin_transfer(&item_id, &dir_obj_id).await?;
                    }
                    None => {
                        break;
                    }
                }
            }

            if is_hash_finish_pending {
                info!("hash is in finish pending for last search.");
                break;
            }

            if *is_finish_scan.lock().await {
                info!("finish scan, look for hashing items again.");
                is_hash_finish_pending = true;
            } else {
                // wait new item found
                match continue_hash_handler.try_recv() {
                    Ok(_) => {
                        while let Ok(_) = continue_hash_handler.try_recv() {
                            // continue to wait for new item
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        let _ = continue_hash_handler.recv().await;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        unreachable!("continue hash channel closed.");
                    }
                }
            }
        }

        // transfer dirs
        let mut scan_depth = 0;
        let mut scan_batch_index = 0;
        let mut new_complete_count = 0;
        let scan_batch_limit = 64;
        loop {
            let dir_items = storage
                .select_dir_transfer(
                    Some(scan_depth),
                    Some(scan_batch_index * scan_batch_limit - new_complete_count),
                    Some(scan_batch_limit),
                )
                .await?;
            let is_depth_finish = (dir_items.len() as u64) < scan_batch_limit;
            scan_batch_index += 1;
            let find_count = dir_items.len();

            for (item_id, _dir_item, _parent_path, item_status, depth) in dir_items {
                info!("transfer dir: {:?}, status: {:?}", item_id, item_status);
                assert!(item_status.is_transfer());
                assert_eq!(
                    depth, scan_depth,
                    "dir item depth should match the scan depth."
                );

                let dir_obj_id = item_status
                    .get_obj_id()
                    .expect("dir item status should have obj id for transfer.");
                let dir_obj = NamedDataMgr::get_object(Some(ndn_mgr_id), &dir_obj_id, None).await?;
                let (dir_obj_id, dir_obj_str) = build_named_object_by_json(OBJ_TYPE_DIR, &dir_obj);
                assert_eq!(
                    &dir_obj_id,
                    item_status
                        .get_obj_id()
                        .expect("dir item should have obj id."),
                    "dir object id should match the item status obj id."
                );

                let lost_obj_ids = writer.push_object(&dir_obj_id, &dir_obj_str).await?;
                if let Some(lost_obj_map_id) = lost_obj_ids.get(0) {
                    debug!(
                        "lost child objects when push dir object: {}, lost: {:?}",
                        dir_obj_id, lost_obj_map_id
                    );
                    assert_eq!(
                        serde_json::from_value::<DirObject>(dir_obj)
                            .expect("DirObject from josn-value failed")
                            .content,
                        lost_obj_map_id.to_string(),
                        "dir object content should match the lost object map id."
                    );
                    let lost_children = writer.push_container(&lost_obj_map_id).await?;
                    //
                    let mut complete_items = storage
                        .complete_chilren_exclude(&item_id, lost_children.as_slice())
                        .await?;

                    loop {
                        if complete_items.is_empty() {
                            break;
                        }
                        let mut new_complete_items = vec![];
                        for complete_item in complete_items.iter() {
                            new_complete_items
                                .push(storage.complete_chilren_exclude(complete_item, &[]).await?);
                        }
                        complete_items = new_complete_items.concat()
                    }

                    if lost_children.is_empty() {
                        let lost_obj_ids = writer.push_object(&dir_obj_id, &dir_obj_str).await?;
                        assert_eq!(lost_obj_ids.len(), 0, "dir object map has pushed");
                        storage.complete(&item_id).await?;
                        if !is_depth_finish {
                            new_complete_count += 1;
                        }
                    }
                } else {
                    storage.complete(&item_id).await?;
                    if !is_depth_finish {
                        new_complete_count += 1;
                    }
                }

                info!("transfer dir: {:?}, status: {:?}", item_id, item_status);
            }

            if is_depth_finish {
                scan_depth += 1;
                if scan_batch_index == 1 && find_count == 0 {
                    info!("no more dir with more depth {}.", scan_depth);
                    break;
                }
                new_complete_count = 0;
                scan_batch_index = 0;
            }
        }

        new_complete_count = 0;
        scan_batch_index = 0;
        scan_depth -= 1;
        loop {
            if scan_depth == 0 {
                debug!("transfer root.");
            }

            let dir_items = storage
                .select_dir_transfer(
                    Some(scan_depth),
                    Some(scan_batch_index * scan_batch_limit - new_complete_count),
                    Some(scan_batch_limit),
                )
                .await?;
            let is_depth_finish = (dir_items.len() as u64) < scan_batch_limit;
            scan_batch_index += 1;

            for (item_id, _dir_item, _parent_path, item_status, depth) in dir_items {
                info!("transfer dir: {:?}, status: {:?}", item_id, item_status);
                assert!(item_status.is_transfer());
                assert_eq!(
                    depth, scan_depth,
                    "dir item depth should match the scan depth."
                );

                let dir_obj_id = item_status
                    .get_obj_id()
                    .expect("dir item status should have obj id for transfer.");
                let dir_obj = NamedDataMgr::get_object(Some(ndn_mgr_id), &dir_obj_id, None).await?;
                let (dir_obj_id, dir_obj_str) = build_named_object_by_json(OBJ_TYPE_DIR, &dir_obj);
                assert_eq!(
                    &dir_obj_id,
                    item_status
                        .get_obj_id()
                        .expect("dir item should have obj id."),
                    "dir object id should match the item status obj id."
                );
                let lost_obj_ids = writer.push_object(&dir_obj_id, &dir_obj_str).await?;
                if let Some(lost_obj_map_id) = lost_obj_ids.get(0) {
                    debug!(
                        "lost child objects when push dir object: {}, lost: {:?}",
                        dir_obj_id, lost_obj_map_id
                    );
                    let lost_children = writer.push_container(&lost_obj_map_id).await?;
                    if !lost_children.is_empty() {
                        panic!(
                            "all children should exist in remote, item_id: {:?}, lost: {:?}",
                            item_id,
                            lost_children
                                .iter()
                                .map(|id| id.to_string())
                                .collect::<Vec<_>>()
                        );
                    }
                    let lost_obj_ids = writer.push_object(&dir_obj_id, &dir_obj_str).await?;
                    assert_eq!(
                        lost_obj_ids.len(),
                        0,
                        "lost object ids should be empty after push object."
                    );
                }
                storage.complete(&item_id).await?;
                if !is_depth_finish {
                    new_complete_count += 1;
                }

                info!("transfer dir: {:?}, status: {:?}", item_id, item_status);
            }

            if is_depth_finish {
                if scan_depth > 0 {
                    scan_depth -= 1;
                    scan_batch_index = 0;
                    new_complete_count = 0;
                } else {
                    info!("no more dir with depth 0, transfer finished.");
                    break;
                }
            }
        }

        Ok(())
    };

    let transfer_task = {
        let reader = reader.clone();
        let writer = writer.clone();
        let storage = storage.clone();
        let is_finish_scan = is_finish_scan.clone();
        let ndn_mgr_id = ndn_mgr_id.to_string();
        tokio::spawn(async move {
            let ret = transfer_task_process(
                storage,
                reader,
                writer,
                is_finish_scan,
                continue_hash_handler,
                ndn_mgr_id.as_str(),
            )
            .await;
            ret
        })
    };

    transfer_task.await.expect("task run failed")?;
    scan_task.await.expect("task run failed")?;

    // TODO: should wait for the root item to be created.
    let (root_item_id, item, parent_path, _, depth) = storage.get_root().await?;
    assert_eq!(depth, 0, "root item depth should be 0.");
    if let Some(path) = path {
        match item.name() {
            StorageItemNameRef::Name(name) => assert_eq!(
                parent_path.join(name).as_path(),
                path,
                "root item parent path should match the scan path."
            ),
            StorageItemNameRef::ChunkSeq(_) => unreachable!(
                "root item name should not be ChunkSeq, got: {:?}",
                item.name()
            ),
        }
    }

    Ok(root_item_id)
}

async fn ndn_to_file_system<
    S: Storage + 'static,
    FFW: FileSystemFileWriter + 'static,
    F: FileSystemWriter<FFW> + 'static,
    R: NdnReader + 'static,
>(
    dir_path_root_obj_id: Option<(&Path, &ObjId)>, // None for continue
    writer: F,
    reader: R,
    storage: S,
) -> NdnResult<S::ItemId> {
    let task_handle = {
        let dir_path_root_obj_id = dir_path_root_obj_id
            .as_ref()
            .map(|(path, obj_id)| (path.to_path_buf(), (*obj_id).clone()));
        let storage = storage.clone();

        let proc = async move || -> NdnResult<()> {
            info!("start ndn to file system task...");

            let create_new_item = async |parent_path: &Path,
                                         obj_id: &ObjId,
                                         parent_item_id: Option<S::ItemId>,
                                         depth: u64|
                   -> NdnResult<()> {
                let obj_value = reader.get_object(obj_id).await?;
                if obj_id.obj_type.as_str() == OBJ_TYPE_DIR {
                    let dir_obj: DirObject = serde_json::from_value(obj_value).map_err(|err| {
                        let msg = format!(
                            "Failed to parse dir object from obj-id: {}, error: {}",
                            obj_id, err
                        );
                        error!("{}", msg);
                        crate::NdnError::DecodeError(msg)
                    })?;
                    info!("create dir: {}, obj id: {}", dir_obj.name, obj_id);
                    let (item_id, _, _) = storage
                        .create_new_item(
                            &StorageItem::Dir(dir_obj),
                            depth,
                            parent_path,
                            parent_item_id,
                        )
                        .await?;
                    storage.begin_transfer(&item_id, obj_id).await?;
                } else if obj_id.obj_type.as_str() == OBJ_TYPE_FILE {
                    let file_obj: FileObject =
                        serde_json::from_value(obj_value).map_err(|err| {
                            let msg = format!(
                                "Failed to parse file object from obj-id: {}, error: {:?}",
                                obj_id, err
                            );
                            error!("{}", msg);
                            crate::NdnError::DecodeError(msg)
                        })?;

                    info!("create file: {}, obj id: {}", file_obj.name, obj_id);

                    let (item_id, _, _) = storage
                        .create_new_item(
                            &StorageItem::File(FileStorageItem {
                                obj: file_obj,
                                chunk_size: None,
                            }),
                            depth,
                            parent_path,
                            parent_item_id,
                        )
                        .await?;
                    storage.begin_transfer(&item_id, obj_id).await?;
                } else {
                    unreachable!(
                        "expect dir or file object, got: {}, obj id: {}",
                        obj_id.obj_type, obj_id
                    );
                }

                Ok(())
            };

            let transfer_item = async |item_id: &S::ItemId,
                                       item: &StorageItem,
                                       parent_path: &Path,
                                       depth: u64|
                   -> NdnResult<()> {
                info!(
                    "transfer item: {:?}, parent path: {}",
                    item_id,
                    parent_path.display()
                );
                match item {
                    StorageItem::Dir(dir_obj) => {
                        info!("transfer dir: {:?}", dir_obj.name);
                        let dir_obj_map_id = ObjId::try_from(dir_obj.content.as_str())?;
                        let obj_map_json = reader.get_container(&dir_obj_map_id).await?;
                        let dir_obj_map = TrieObjectMap::open(obj_map_json).await?;

                        writer.create_dir(dir_obj, parent_path).await?;

                        let child_obj_ids = dir_obj_map
                            .iter()?
                            .map(|(_, child_obj_id)| child_obj_id)
                            .collect::<Vec<_>>();
                        for child_obj_id in child_obj_ids {
                            create_new_item(
                                &parent_path.join(dir_obj.name.as_str()).as_path(),
                                &child_obj_id,
                                Some(item_id.clone()),
                                depth + 1,
                            )
                            .await?;
                        }
                        storage.complete(item_id).await?;
                    }
                    StorageItem::File(file_storage_item) => {
                        info!("transfer file: {:?}", file_storage_item.obj.name);
                        let file_chunk_list_id =
                            ObjId::try_from(file_storage_item.obj.content.as_str())?;
                        let chunk_list = reader.get_container(&file_chunk_list_id).await?;
                        let chunk_list = ChunkListBuilder::open(chunk_list).await?.build().await?;

                        let file_writer = writer
                            .open_file(&file_storage_item.obj, parent_path)
                            .await?;
                        let file_length = file_writer.length().await?;
                        let (chunk_index, mut pos) = if file_length > 0 {
                            let (chunk_index, chunk_pos) = chunk_list
                                .get_chunk_index_by_offset(SeekFrom::Start(file_length - 1))?;
                            let chunk_id = chunk_list
                                .get_chunk(chunk_index as usize)?
                                .expect("chunk id should exist");
                            match chunk_pos
                                .cmp(&chunk_id.get_length().expect("chunk id should have length"))
                            {
                                std::cmp::Ordering::Less => {
                                    info!(
                                        "chunk pos: {}, file pos: {}, chunk index: {}",
                                        chunk_pos, file_length, chunk_index
                                    );
                                    (chunk_index, file_length - (chunk_pos + 1))
                                }
                                std::cmp::Ordering::Equal => {
                                    info!(
                                        "chunk pos: {}, file pos: {}, chunk index: {}",
                                        chunk_pos, file_length, chunk_index
                                    );
                                    (chunk_index + 1, file_length)
                                }
                                std::cmp::Ordering::Greater => {
                                    unreachable!(
                                        "chunk pos: {}, file pos: {}, chunk index: {}",
                                        chunk_pos, file_length, chunk_index
                                    );
                                }
                            }
                        } else {
                            (0, 0)
                        };

                        for chunk_index in chunk_index..chunk_list.len() {
                            let chunk_index = chunk_index as usize;
                            let chunk_id = chunk_list
                                .get_chunk(chunk_index)?
                                .expect("chunk id should exist");
                            let chunk_data = reader.get_chunk(&chunk_id).await?;
                            assert_eq!(
                                chunk_data.len() as u64,
                                chunk_id.get_length().expect("chunk id should have length"),
                                "chunk data length should match the chunk id length."
                            );

                            let hasher = ChunkHasher::new(None).expect("hash failed.");
                            let hash = hasher.calc_from_bytes(chunk_data.as_slice());
                            let calc_chunk_id = ChunkId::from_mix_hash_result_by_hash_method(
                                chunk_data.len() as u64,
                                &hash,
                                HashMethod::Sha256,
                            )?;
                            if calc_chunk_id != chunk_id {
                                error!(
                                    "chunk id mismatch, expected: {:?}, got: {:?}",
                                    calc_chunk_id, chunk_id
                                );
                                return Err(NdnError::InvalidData("chunk id mismatch".to_string()));
                            }

                            file_writer
                                .write_chunk(chunk_data.as_slice(), SeekFrom::Start(pos))
                                .await?;
                            pos += chunk_data.len() as u64;
                        }

                        storage.complete(item_id).await?;
                    }
                    StorageItem::Chunk(chunk_item) => {
                        unreachable!(
                            "should not have chunk item in dir transfer, got: {:?}",
                            chunk_item.chunk_id
                        );
                    }
                }
                Ok(())
            };

            if let Some((path, root_obj_id)) = dir_path_root_obj_id {
                info!(
                    "scan path: {}, root obj id: {}",
                    path.display(),
                    root_obj_id
                );
                writer.create_dir_all(path.as_path()).await?;
                create_new_item(path.as_path(), &root_obj_id, None, 0).await?;
            }

            loop {
                let items = storage.select_item_transfer(Some(0), Some(64)).await?;
                if items.is_empty() {
                    info!("no more items to transfer.");
                    break;
                }

                for (item_id, item, parent_path, item_status, depth) in items {
                    info!("transfer item: {:?}, status: {:?}", item_id, item_status);
                    assert!(item_status.is_transfer());
                    assert_eq!(depth, 0, "item depth should be 0 for root item transfer.");

                    transfer_item(&item_id, &item, parent_path.as_path(), depth).await?;
                }
            }

            Ok(())
        };

        tokio::spawn(async move { proc().await })
    };

    task_handle.await.expect("task abort")?;

    // TODO: should wait for the root item to be created.
    let (root_item_id, item, parent_path, _, depth) = storage.get_root().await?;
    assert_eq!(depth, 0, "root item depth should be 0.");
    if let Some((dir_path, _)) = dir_path_root_obj_id {
        assert_eq!(
            dir_path,
            parent_path.as_path(),
            "root item parent path should match the scan path."
        );
    }

    Ok(root_item_id)
}

fn generate_random_bytes(size: u64) -> Vec<u8> {
    let mut rng = rand::rng();
    let mut buffer = vec![0u8; size as usize];
    rng.fill_bytes(&mut buffer);
    buffer
}

fn generate_random_chunk_mix(size: u64) -> (ChunkId, Vec<u8>) {
    let chunk_data = generate_random_bytes(size);
    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&chunk_data);
    let chunk_id = ChunkId::from_mix_hash_result_by_hash_method(size, &hash, HashMethod::Sha256).unwrap();
    info!("chunk_id: {}", chunk_id.to_string());
    (chunk_id, chunk_data)
}

fn generate_random_chunk(size: u64) -> (ChunkId, Vec<u8>) {
    let chunk_data = generate_random_bytes(size);
    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&chunk_data);
    let chunk_id = ChunkId::from_hash_result(&hash, ChunkType::Sha256);
    info!("chunk_id: {}", chunk_id.to_string());
    (chunk_id, chunk_data)
}

fn generate_random_chunk_list(count: usize, fix_size: Option<u64>) -> Vec<(ChunkId, Vec<u8>)> {
    let mut chunk_list = Vec::with_capacity(count);
    for _ in 0..count {
        let (chunk_id, chunk_data) = if let Some(size) = fix_size {
            generate_random_chunk_mix(size)
        } else {
            generate_random_chunk_mix(rand::rng().random_range(1024u64..1024 * 1024 * 10))
        };
        chunk_list.push((chunk_id, chunk_data));
    }
    chunk_list
}

#[derive(Clone)]
struct SimulateFile {
    name: String,
    content: Vec<u8>,
}

impl std::fmt::Debug for SimulateFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimulateFile")
            .field("name", &self.name)
            .finish()
    }
}

#[derive(Clone, Debug)]
struct SimulateDir {
    name: String,
    children: HashMap<String, SimulateFsItem>,
}

#[derive(Clone, Debug)]
enum SimulateFsItem {
    File(SimulateFile),
    Dir(SimulateDir),
}

impl SimulateFsItem {
    fn name(&self) -> &str {
        match self {
            SimulateFsItem::File(file) => &file.name,
            SimulateFsItem::Dir(dir) => &dir.name,
        }
    }

    fn check_dir(&self) -> &SimulateDir {
        match self {
            SimulateFsItem::File(_) => panic!("expected a directory, got a file"),
            SimulateFsItem::Dir(dir) => dir,
        }
    }

    fn check_dir_mut(&mut self) -> &mut SimulateDir {
        match self {
            SimulateFsItem::File(_) => panic!("expected a directory, got a file"),
            SimulateFsItem::Dir(dir) => dir,
        }
    }

    fn check_file(&self) -> &SimulateFile {
        match self {
            SimulateFsItem::File(file) => file,
            SimulateFsItem::Dir(_) => panic!("expected a file, got a directory"),
        }
    }

    fn check_file_mut(&mut self) -> &mut SimulateFile {
        match self {
            SimulateFsItem::File(file) => file,
            SimulateFsItem::Dir(_) => panic!("expected a file, got a directory"),
        }
    }

    fn is_dir(&self) -> bool {
        matches!(self, SimulateFsItem::Dir(_))
    }
    fn is_file(&self) -> bool {
        matches!(self, SimulateFsItem::File(_))
    }

    fn find_child(&self, path: &Path) -> Option<&SimulateFsItem> {
        let paths = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let mut found = self;
        assert_eq!(
            paths.get(0).expect("Path must have at least one component"),
            found.name(),
            "Path does not match the root item name"
        );

        for path in paths.as_slice()[1..].iter() {
            match found {
                SimulateFsItem::File(_) => {
                    unreachable!("Expected a directory, but found a file at path: {}", path);
                }
                SimulateFsItem::Dir(dir) => {
                    if let Some(child) = dir.children.get(path) {
                        found = child;
                    } else {
                        unreachable!(
                            "Path component '{}' not found in directory '{}'",
                            path, dir.name
                        );
                    }
                }
            }
        }
        Some(found)
    }

    fn find_child_mut(&mut self, path: &Path) -> Option<&mut SimulateFsItem> {
        let paths = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let mut found = self;
        assert_eq!(
            paths.get(0).expect("Path must have at least one component"),
            found.name(),
            "Path does not match the root item name"
        );

        for path in paths.as_slice()[1..].iter() {
            match found {
                SimulateFsItem::File(_) => {
                    unreachable!("Expected a directory, but found a file at path: {}", path);
                }
                SimulateFsItem::Dir(dir) => {
                    if let Some(child) = dir.children.get_mut(path) {
                        found = child;
                    } else {
                        unreachable!(
                            "Path component '{}' not found in directory '{}'",
                            path, dir.name
                        );
                    }
                }
            }
        }
        Some(found)
    }
}

fn gen_random_simulate_dir(
    total_dir_count: usize,
    total_file_count: usize,
    max_file_size: u64,
) -> SimulateFsItem {
    let mut rng = rand::rng();

    let mut left_file_count = total_file_count;

    let avg_child_dir_count = |dir_count: usize| {
        std::cmp::min(
            std::cmp::max((dir_count as f32).sqrt() as usize, 1),
            dir_count,
        )
    };

    let max_child_file_count = |dir_count: usize, file_count: usize| {
        std::cmp::min(std::cmp::max((file_count / dir_count) * 2, 1), file_count)
    };

    let total_dir_count = std::cmp::max(total_dir_count, 1);

    let mut dir_list: Vec<SimulateDir> = (0..total_dir_count)
        .map(|i| SimulateDir {
            name: "".to_string(),
            children: HashMap::new(),
        })
        .collect();

    let mut free_pos = 1;

    dir_list.last_mut().unwrap().name = "0".to_string();

    let mut leaf_count = 1;
    for pos in 0..total_dir_count {
        let parent_name = dir_list[total_dir_count - pos - 1].name.clone();
        let child_dir_count = {
            let max_child_dir_count = total_dir_count - free_pos;
            let avg_child_dir_count = avg_child_dir_count(max_child_dir_count);
            if leaf_count == 1 {
                if max_child_dir_count <= 1 {
                    max_child_dir_count
                } else if avg_child_dir_count <= 1 {
                    1
                } else {
                    rng.random_range(1..avg_child_dir_count)
                }
            } else if leaf_count == 0 {
                assert_eq!(pos, 0, "should be last dir");
                0
            } else {
                if max_child_dir_count == 0 {
                    0
                } else {
                    rng.random_range(0..avg_child_dir_count)
                }
            }
        };
        let child_file_count =
            rng.random_range(0..max_child_file_count(total_dir_count - pos, left_file_count));
        let select_child_dir_begin_pos = total_dir_count - free_pos - child_dir_count;
        free_pos += child_dir_count;
        left_file_count -= child_file_count;
        leaf_count += child_dir_count;

        for seq in 0..child_dir_count {
            let dir_index = select_child_dir_begin_pos + seq;
            let child_dir_name = format!("{}_{}", parent_name, seq);
            dir_list[dir_index].name = child_dir_name;
        }

        for seq in 0..child_file_count {
            let file_name = format!("{}_file_{}", parent_name, seq);
            let file_size = rng.random_range(0..max_file_size);
            let file_content = generate_random_bytes(file_size);
            dir_list[total_dir_count - pos - 1].children.insert(
                file_name.clone(),
                SimulateFsItem::File(SimulateFile {
                    name: file_name,
                    content: file_content,
                }),
            );
        }
        leaf_count -= 1;
    }

    let mut root_dir = dir_list.pop().expect("should have root dir");
    for dir in dir_list.into_iter().rev() {
        let parent_paths = {
            let paths = dir.name.split('_').collect::<Vec<_>>();
            let mut count = 1;
            let mut pos = 0;
            let mut parent_paths = vec![];
            while count < paths.len() {
                let dir_name = paths[0..count].join("_");
                pos += count;
                count += 1;
                parent_paths.push(dir_name.clone());
            }
            parent_paths
        };
        let mut parent_dir = &mut root_dir;
        assert_eq!(
            parent_dir.name.as_str(),
            parent_paths.get(0).expect("should have root name")
        );
        for parent_name in parent_paths.as_slice()[1..].iter() {
            parent_dir = match parent_dir.children.get_mut(parent_name).unwrap() {
                SimulateFsItem::File(simulate_file) => unreachable!(
                    "parent dir should not be a file, got: {}",
                    simulate_file.name
                ),
                SimulateFsItem::Dir(simulate_dir) => {
                    assert_eq!(
                        simulate_dir.name.as_str(),
                        parent_name,
                        "parent dir name should match the path"
                    );
                    simulate_dir
                }
            };
        }
        parent_dir
            .children
            .insert(dir.name.clone(), SimulateFsItem::Dir(dir));
    }

    SimulateFsItem::Dir(root_dir)
}

#[derive(Debug, Copy, Clone)]
enum UpdateTypeDir {
    Remove,
    Add(usize, usize), // dir-count, file-count
    Move,
    Rename,
    None,
}

#[derive(Debug, Copy, Clone)]
enum UpdateFileMethod {
    Truncation,
    InsertHead,
    InsertMiddle,
    Rename,
}

#[derive(Debug, Clone)]
enum UpdateTypeFile {
    Remove,
    Update(Vec<UpdateFileMethod>),
    Move,
    None,
}

fn update_simulate_dir(
    dir: &SimulateFsItem,
    dir_update_types: &[UpdateTypeDir],
    file_update_types: &[UpdateTypeFile],
) -> SimulateFsItem {
    let update_item = |item: &SimulateFsItem| -> (Option<SimulateFsItem>, Option<SimulateFsItem>) {
        let mut rng = rand::rng();
        match item {
            SimulateFsItem::File(simulate_file) => {
                let op_type = rng.random_range(0..file_update_types.len());
                let op_type = file_update_types.get(op_type).unwrap();
                match op_type {
                    UpdateTypeFile::Remove => {
                        info!("remove file: {}", simulate_file.name);
                        (None, None)
                    }
                    UpdateTypeFile::Update(methods) => {
                        let update_method = rng.random_range(0..methods.len());
                        let method = methods.get(update_method).unwrap();
                        let mut new_file = SimulateFile {
                            name: "".to_string(),
                            content: vec![0u8; 0],
                        };
                        match method {
                            UpdateFileMethod::Truncation => {
                                let new_size = rng.random_range(0..simulate_file.content.len());
                                info!(
                                    "truncate file: {}, new size: {}",
                                    simulate_file.name, new_size
                                );
                                new_file.content =
                                    simulate_file.content.as_slice()[0..new_size].to_vec();
                                new_file.name = simulate_file.name.clone();
                            }
                            UpdateFileMethod::InsertHead => {
                                let insert_data = generate_random_bytes(rng.random_range(32..128));
                                info!(
                                    "insert head to file: {}, data size: {}",
                                    simulate_file.name,
                                    insert_data.len()
                                );
                                new_file.content =
                                    [insert_data.as_slice(), simulate_file.content.as_slice()]
                                        .concat();
                                new_file.name = simulate_file.name.clone();
                            }
                            UpdateFileMethod::InsertMiddle => {
                                let insert_pos = rng.random_range(0..new_file.content.len());
                                let insert_data = generate_random_bytes(rng.random_range(32..128));
                                info!(
                                    "insert middle to file: {}, pos: {}, data size: {}",
                                    simulate_file.name,
                                    insert_pos,
                                    insert_data.len()
                                );
                                new_file.content = [
                                    &simulate_file.content.as_slice()[0..insert_pos],
                                    insert_data.as_slice(),
                                    &simulate_file.content.as_slice()[insert_pos..],
                                ]
                                .concat();
                                new_file.name = simulate_file.name.clone();
                            }
                            UpdateFileMethod::Rename => {
                                let mut random_name = [0u8; 16];
                                rng.fill_bytes(&mut random_name);
                                let new_name = format!(
                                    "renamed_{}_{}",
                                    hex::encode(random_name),
                                    simulate_file.name
                                );
                                info!("rename file: {} to {}", simulate_file.name, new_name);
                                new_file.content = simulate_file.content.clone();
                                new_file.name = new_name;
                            }
                        }
                        (Some(SimulateFsItem::File(new_file)), None)
                    }
                    UpdateTypeFile::Move => {
                        info!("move file: {}", simulate_file.name);
                        (None, Some(item.clone()))
                    }
                    UpdateTypeFile::None => (Some(item.clone()), None),
                }
            }
            SimulateFsItem::Dir(simulate_dir) => {
                let op_type = rng.random_range(0..dir_update_types.len());
                let op_type = dir_update_types.get(op_type).unwrap();
                match op_type {
                    UpdateTypeDir::Remove => {
                        info!("remove dir: {}", simulate_dir.name);
                        (None, None)
                    }
                    UpdateTypeDir::Add(dir_count, file_count) => {
                        info!(
                            "add dir: {}, file: {} to dir: {}",
                            dir_count, file_count, simulate_dir.name
                        );
                        let mut new_dir = gen_random_simulate_dir(*dir_count, *file_count, 1024);

                        (
                            Some(SimulateFsItem::Dir(SimulateDir {
                                name: simulate_dir.name.clone(),
                                children: match new_dir {
                                    SimulateFsItem::File(_) => unreachable!(),
                                    SimulateFsItem::Dir(new_dir) => {
                                        let mut random_name = [0u8; 16];
                                        rng.fill_bytes(&mut random_name);
                                        let random_name =
                                            format!("insert_{}", hex::encode(random_name));
                                        let mut new_children = HashMap::new();
                                        for (name, mut child) in new_dir.children {
                                            let new_name = format!("{}_{}", random_name, name);
                                            match &mut child {
                                                SimulateFsItem::File(file) => {
                                                    file.name = new_name.clone();
                                                }
                                                SimulateFsItem::Dir(dir) => {
                                                    dir.name = new_name.clone();
                                                }
                                            }
                                            new_children.insert(new_name, child);
                                        }
                                        new_children
                                    }
                                },
                            })),
                            None,
                        )
                    }
                    UpdateTypeDir::Move => {
                        info!("move dir: {}", simulate_dir.name);
                        (None, Some(item.clone()))
                    }
                    UpdateTypeDir::Rename => {
                        let mut random_name = [0u8; 16];
                        rng.fill_bytes(&mut random_name);
                        let new_name =
                            format!("renamed_{}_{}", hex::encode(random_name), simulate_dir.name);
                        info!("rename dir: {} to {}", simulate_dir.name, new_name);
                        (
                            Some(SimulateFsItem::Dir(SimulateDir {
                                name: new_name,
                                children: HashMap::new(),
                            })),
                            None,
                        )
                    }
                    UpdateTypeDir::None => (
                        Some(SimulateFsItem::Dir(SimulateDir {
                            name: simulate_dir.name.clone(),
                            children: HashMap::new(),
                        })),
                        None,
                    ),
                }
            }
        }
    };

    let mut free_items = vec![];
    // Recursively update the directory structure
    let mut traverse_stack = vec![];
    let mut result_item_path = vec![];
    let (new_item, free_item) = update_item(&dir);
    let mut result_root_item = match new_item {
        Some(new_item) => {
            match dir {
                SimulateFsItem::File(simulate_file) => {}
                SimulateFsItem::Dir(simulate_dir) => {
                    traverse_stack.push(simulate_dir.children.iter());
                }
            }
            new_item
        }
        None => {
            info!("remove root item, return empty");
            SimulateFsItem::Dir(SimulateDir {
                name: "new_root".to_string(),
                children: HashMap::new(),
            })
        }
    };

    loop {
        if traverse_stack.is_empty() {
            break;
        }

        let child = traverse_stack.last_mut().unwrap().next();
        match child {
            Some((name, item)) => {
                let (new_item, free_item) = update_item(item);
                if let Some(new_item) = new_item {
                    match &mut result_root_item {
                        SimulateFsItem::File(_) => unreachable!(),
                        SimulateFsItem::Dir(root_result_dir) => {
                            let mut result_dir = root_result_dir;
                            for path in result_item_path.iter() {
                                result_dir = match result_dir.children.get_mut(path).unwrap() {
                                    SimulateFsItem::File(simulate_file) => {
                                        unreachable!(
                                            "result dir should not have file, got: {}",
                                            simulate_file.name
                                        );
                                    }
                                    SimulateFsItem::Dir(simulate_dir) => simulate_dir,
                                };
                            }

                            let new_item_name = new_item.name().to_string();
                            result_dir.children.insert(new_item_name.clone(), new_item);
                            match item {
                                SimulateFsItem::File(_) => {}
                                SimulateFsItem::Dir(item_dir) => {
                                    result_item_path.push(new_item_name);
                                    traverse_stack.push(item_dir.children.iter());
                                }
                            }
                        }
                    }
                } else if let Some(free_item) = free_item {
                    free_items.push(free_item);
                }
            }
            None => {
                result_item_path.pop();
                traverse_stack.pop();
            }
        }
    }

    for free_item in free_items {
        let mut insert_children = &mut result_root_item.check_dir_mut().children;
        loop {
            let child_dir_names = insert_children
                .iter()
                .filter(|(_, item)| item.is_dir())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            let insert_pos = rand::rng().random_range(0..(child_dir_names.len() + 1));
            match child_dir_names.get(insert_pos) {
                Some(name) => {
                    insert_children = &mut (insert_children
                        .get_mut(name)
                        .unwrap()
                        .check_dir_mut()
                        .children);
                }
                None => {
                    // Insert to the root directory
                    insert_children.insert(free_item.name().to_string(), free_item);
                    break;
                }
            }
        }
    }

    result_root_item
}

async fn write_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId, chunk_data: &[u8]) {
    let (mut chunk_writer, _progress_info) =
        NamedDataMgr::open_chunk_writer(Some(ndn_mgr_id), chunk_id, chunk_data.len() as u64, 0)
            .await
            .expect("open chunk writer failed");
    chunk_writer
        .write_all(chunk_data)
        .await
        .expect("write chunk to ndn-mgr failed");
    NamedDataMgr::complete_chunk_writer(Some(ndn_mgr_id), chunk_id)
        .await
        .expect("wait chunk writer complete failed.");
}

async fn read_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId) -> Vec<u8> {
    let (mut chunk_reader, len) =
        NamedDataMgr::open_chunk_reader(Some(ndn_mgr_id), chunk_id, SeekFrom::Start(0), false)
            .await
            .expect("open reader from ndn-mgr failed.");

    let mut buffer = vec![0u8; len as usize];
    chunk_reader
        .read_exact(&mut buffer)
        .await
        .expect("read chunk from ndn-mgr failed");

    buffer
}

type NdnServerHost = String;

async fn init_ndn_server(ndn_mgr_id: &str) -> (NdnClient, NdnServerHost) {
    let mut rng = rand::rng();
    let tls_port = rng.random_range(10000u16..20000u16);
    let http_port = rng.random_range(10000u16..20000u16);
    let test_server_config = json!({
        "tls_port": tls_port,
        "http_port": http_port,
        "hosts": {
            "*": {
                "enable_cors": true,
                "routes": {
                    "/ndn/": {
                        "named_mgr": {
                            "named_data_mgr_id": ndn_mgr_id,
                            "read_only": false,
                            "guest_access": true,
                            "is_chunk_id_in_path": true,
                            "enable_mgr_file_path": true
                        }
                    }
                }
            }
        }
    });

    let test_server_config: WarpServerConfig = serde_json::from_value(test_server_config).unwrap();

    tokio::spawn(async move {
        info!("start test ndn server(powered by cyfs-warp)...");
        start_cyfs_warp_server(test_server_config)
            .await
            .expect("start cyfs warp server failed.");
    });
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let temp_dir = tempfile::tempdir()
        .unwrap()
        .path()
        .join("ndn-test")
        .join(ndn_mgr_id);

    fs::create_dir_all(temp_dir.as_path())
        .await
        .expect("create temp dir failed.");

    let config = NamedDataMgrConfig {
        local_stores: vec![temp_dir.to_str().unwrap().to_string()],
        local_cache: None,
        mmap_cache_dir: None,
    };

    let named_mgr =
        NamedDataMgr::from_config(Some(ndn_mgr_id.to_string()), temp_dir.to_path_buf(), config)
            .await
            .expect("init NamedDataMgr failed.");

    NamedDataMgr::set_mgr_by_id(Some(ndn_mgr_id), named_mgr)
        .await
        .expect("set named data manager by id failed.");

    let host = format!("localhost:{}", http_port);
    let client = NdnClient::new(
        format!("http://{}/ndn/", host),
        None,
        Some(ndn_mgr_id.to_string()),
    );

    (client, host)
}

async fn init_obj_array_storage_factory() -> PathBuf {
    let data_path = std::env::temp_dir().join("test_ndn_chunklist_data");
    if GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().is_some() {
        info!("Object array storage factory already initialized");
        return data_path;
    }
    if !data_path.exists() {
        fs::create_dir_all(&data_path)
            .await
            .expect("create data path failed");
    }

    let _ = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.set(ObjectArrayStorageFactory::new(&data_path));
    data_path
}

async fn init_obj_map_storage_factory() -> PathBuf {
    let data_path = std::env::temp_dir().join("test_ndn_trie_obj_map_data");
    if GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY.get().is_some() {
        info!("Object map storage factory already initialized");
        return data_path;
    }
    if !data_path.exists() {
        fs::create_dir_all(&data_path)
            .await
            .expect("create data path failed");
    }

    GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
        .set(TrieObjectMapStorageFactory::new(
            data_path.clone(),
            Some(TrieObjectMapStorageType::JSONFile),
        ))
        .map_err(|_| ())
        .expect("Object array storage factory already initialized");
    data_path
}

#[derive(Clone)]
struct MemoryStorage {
    items: HashMap<u64, (StorageItem, PathBuf, ItemStatus, PathDepth, Vec<u64>)>,
    next_item_id: u64,
}

impl MemoryStorage {
    fn new() -> Self {
        MemoryStorage {
            items: HashMap::new(),
            next_item_id: 0,
        }
    }
}

#[async_trait::async_trait]
impl Storage for Arc<tokio::sync::Mutex<MemoryStorage>> {
    type ItemId = u64;
    async fn create_new_item(
        &self,
        item: &StorageItem,
        depth: PathDepth,
        parent_path: &Path,
        parent_item_id: Option<Self::ItemId>,
    ) -> NdnResult<(Self::ItemId, StorageItem, ItemStatus)> {
        let mut storage = self.lock().await;
        let item_id = storage.next_item_id;
        if let Some(parent_item_id) = parent_item_id {
            assert!(
                parent_item_id < storage.next_item_id,
                "parent item id out of range"
            );
            let parent_item = &storage
                .items
                .get(&parent_item_id)
                .expect("parent item must exist");
            assert_eq!(parent_item.3 + 1, depth, "parent item depth does not match");
            assert_eq!(
                parent_item.1.join(parent_item.0.name().check_name()),
                parent_path,
                "parent item path does not match"
            );
            assert!(
                parent_item.2.is_scanning(),
                "parent item status must be Scanning for new item creation"
            );

            match item {
                StorageItem::Dir(dir_object) => {
                    assert!(parent_item.0.is_dir(), "parent of dir must be dir");
                }
                StorageItem::File(file_storage_item) => {
                    assert!(parent_item.0.is_dir(), "parent of file must be dir");
                }
                StorageItem::Chunk(chunk_item) => {
                    assert!(parent_item.0.is_file(), "parent of chunk must be file");
                }
            }

            for child_item_id in parent_item.4.iter() {
                assert_ne!(*child_item_id, item_id, "item id must be unique");
                let child_item = &storage
                    .items
                    .get(child_item_id)
                    .expect("child item must exist");
                if child_item.0.name() == item.name() {
                    return Ok((*child_item_id, child_item.0.clone(), child_item.2.clone()));
                }
            }

            let parent_item = storage
                .items
                .get_mut(&parent_item_id)
                .expect("parent item must exist");
            parent_item.4.push(item_id);
        } else {
            assert_eq!(depth, 0, "root item depth must be 0");
            assert_eq!(item_id, 0, "root item is the first item");
        }

        info!(
            "create new item: {:?}, depth: {}, parent_path: {:?}",
            item, depth, parent_path
        );

        storage.items.insert(
            item_id,
            (
                item.clone(),
                parent_path.to_path_buf(),
                ItemStatus::Scanning,
                depth,
                vec![],
            ),
        );
        storage.next_item_id += 1;
        Ok((item_id, item.clone(), ItemStatus::Scanning))
    }

    async fn remove_dir(&self, item_id: &Self::ItemId) -> NdnResult<u64> {
        let mut storage = self.lock().await;
        if let Some((item, parent_path, status, depth, children)) = storage.items.remove(item_id) {
            info!(
                "remove dir: {:?}, parent_path: {:?}, depth: {}",
                item, parent_path, depth
            );
            assert!(item.is_dir(), "item must be a directory");
            assert!(children.is_empty(), "directory must be empty");
            Ok(1)
        } else {
            unreachable!("Item not found in storage");
        }
    }
    async fn remove_children(&self, item_id: &Self::ItemId) -> NdnResult<u64> {
        let mut storage = self.lock().await;
        let remove_children = if let Some((item, _parent_path, _status, _depth, children)) =
            storage.items.get_mut(item_id)
        {
            assert!(!item.is_chunk(), "item must not be a chunk");
            let mut remove_children = vec![];
            std::mem::swap(children, &mut remove_children);
            remove_children
        } else {
            unreachable!("Item not found in storage");
        };

        let children_count = remove_children.len();
        for child_id in remove_children {
            if let Some(child) = storage.items.remove(&child_id) {
                info!(
                    "remove child item: {:?}, parent_path: {:?}, depth: {}, parent_id: {}",
                    child.0, child.1, child.3, item_id
                );
            } else {
                unreachable!("Child item not found in storage");
            }
        }
        Ok(children_count as u64)
    }
    async fn begin_hash(&self, item_id: &Self::ItemId) -> NdnResult<()> {
        let mut storage = self.lock().await;
        if let Some((item, parent_path, status, depth, children)) = storage.items.get_mut(item_id) {
            assert!(
                status.is_scanning() || status.is_hashing(),
                "item must be in Scanning|Hashing status"
            );
            assert!(
                item.is_dir() || item.is_file(),
                "item must be a directory or file"
            );
            info!(
                "begin hashing item: {:?}, parent_path: {:?}, depth: {}, old_status: {:?}",
                item, parent_path, depth, status
            );
            *status = ItemStatus::Hashing;
            Ok(())
        } else {
            unreachable!("Item not found in storage");
        }
    }
    async fn begin_transfer(&self, item_id: &Self::ItemId, content: &ObjId) -> NdnResult<()> {
        let mut storage = self.lock().await;
        if let Some((item, parent_path, status, depth, children)) = storage.items.get_mut(item_id) {
            if item.is_chunk() {
                assert!(
                    status.is_scanning() || status.is_transfer(),
                    "chunk item must be in Scanning status"
                ); // chunk transfer immediately after scanning
            } else {
                assert!(
                    status.is_hashing() || status.is_transfer(),
                    "item must be in Scanning|Hashing status"
                );
            }

            info!(
                "begin transfer item: {:?}, parent_path: {:?}, depth: {}, old_status: {:?}, content: {}",
                item, parent_path, depth, status, content
            );
            *status = ItemStatus::Transfer(content.clone());
            Ok(())
        } else {
            unreachable!("Item not found in storage");
        }
    }
    async fn complete(&self, item_id: &Self::ItemId) -> NdnResult<()> {
        let mut storage_guard = self.lock().await;
        let storage = &mut *storage_guard;
        if let Some((_item, _parent_path, status, _depth, _children)) =
            storage.items.get_mut(item_id)
        {
            assert!(
                status.is_transfer() || status.is_complete(),
                "item must be in Transfer | Complete status"
            );

            let content = match status {
                ItemStatus::Transfer(content) => content.clone(),
                ItemStatus::Complete(_) => {
                    // already complete, nothing to do
                    return Ok(());
                }
                _ => unreachable!("item status must be Transfer or Complete"),
            };
            info!(
                "complete item: {:?}, parent_path: {:?}, depth: {}, old_status: {:?}, content: {}",
                _item, _parent_path, _depth, status, content
            );
            *status = ItemStatus::Complete(content);
            Ok(())
        } else {
            unreachable!("Item not found in storage");
        }
    }
    async fn get_root(
        &self,
    ) -> NdnResult<(Self::ItemId, StorageItem, PathBuf, ItemStatus, PathDepth)> {
        let storage_guard = self.lock().await;
        let storage = &*storage_guard;
        if storage.items.is_empty() {
            unreachable!("Storage is empty, no root item available");
        }
        let root_item_id = 0; // assuming root item is always the first item
        if let Some((item, parent_path, status, depth, _)) = storage.items.get(&root_item_id) {
            assert_eq!(depth, &0, "Root item depth should be 0");
            info!(
                "get root item: {:?}, parent_path: {:?}, depth: {}, status: {:?}",
                item, parent_path, depth, status
            );
            Ok((
                root_item_id,
                item.clone(),
                parent_path.to_path_buf(),
                status.clone(),
                *depth,
            ))
        } else {
            unreachable!("Root item not found in storage");
        }
    }
    async fn complete_chilren_exclude(
        &self,
        item_id: &Self::ItemId,
        exclude_item_obj_ids: &[ObjId],
    ) -> NdnResult<Vec<Self::ItemId>> {
        let mut storage = self.lock().await;
        let mut select_children_id = vec![];
        if let Some((_item, _parent_path, status, _depth, children)) = storage.items.get(item_id) {
            match status {
                ItemStatus::Complete(_) => {
                    // already complete, nothing to do
                    assert!(false, "Item already complete");
                    return Ok(vec![]);
                }
                _ => {
                    assert!(
                        status.is_hashing() || status.is_transfer(),
                        "item must be in Hashing | Transfer status {:?}",
                        status
                    );
                    for child_id in children.iter() {
                        let child_status = storage
                            .items
                            .get(child_id)
                            .map(|(_, _, status, _, _)| status)
                            .expect("Child item must exist in storage");
                        if !child_status.is_complete() {
                            let child_obj_id = child_status.get_obj_id();
                            if child_obj_id.is_none()
                                || !exclude_item_obj_ids.contains(child_obj_id.unwrap())
                            {
                                select_children_id.push(child_id.clone());
                            }
                        }
                    }
                }
            }
        } else {
            return Err(NdnError::NotFound("Item not found".to_string()));
        }

        for child_id in select_children_id.iter() {
            if let Some((_child, _parent_path, status, _depth, _children)) =
                storage.items.get_mut(child_id)
            {
                info!(
                    "complete child item: {:?}, old_status: {:?}, parent_path: {:?}, depth :{}",
                    _child, status, _parent_path, _depth
                );
                *status = ItemStatus::Complete(
                    status
                        .get_obj_id()
                        .expect("Item must have an ObjId")
                        .clone(),
                );
            } else {
                unreachable!("Child item not found in storage");
            }
        }

        Ok(select_children_id)
    }
    async fn get_item(
        &self,
        item_id: &Self::ItemId,
    ) -> NdnResult<(StorageItem, ItemStatus, PathDepth)> {
        let storage = self.lock().await;
        if let Some((item, _, status, depth, _)) = storage.items.get(item_id) {
            Ok((item.clone(), status.clone(), *depth))
        } else {
            unreachable!("Item not found in storage");
        }
    }
    async fn select_dir_scan_or_new(
        &self,
    ) -> NdnResult<Option<(Self::ItemId, DirObject, PathBuf, ItemStatus, PathDepth)>> {
        // to continue scan

        let storage = self.lock().await;
        for (item_id, (item, parent_path, status, depth, _)) in storage.items.iter() {
            if item.is_dir() && (status.is_scanning() || status.is_new()) {
                info!(
                    "select dir for scan or new: {:?}, parent_path: {:?}, depth: {}, status: {:?}",
                    item, parent_path, depth, status
                );
                return Ok(Some((
                    *item_id,
                    item.clone().check_dir().clone(),
                    parent_path.to_path_buf(),
                    status.clone(),
                    *depth,
                )));
            }
        }

        info!("No directory found for scanning or new creation");
        Ok(None)
    }
    async fn select_file_hashing_or_transfer(
        &self,
    ) -> NdnResult<
        Option<(
            Self::ItemId,
            FileStorageItem,
            PathBuf,
            ItemStatus,
            PathDepth,
        )>,
    > {
        // to continue transfer file after all child-dir hash and all child-file complete
        let storage = self.lock().await;
        for (item_id, (item, parent_path, status, depth, _)) in storage.items.iter() {
            if item.is_file() && (status.is_hashing() || status.is_transfer()) {
                info!(
                    "select file for hashing or transfer: {:?}, parent_path: {:?}, depth: {}, status: {:?}",
                    item, parent_path, depth, status
                );
                return Ok(Some((
                    *item_id,
                    item.clone().check_file().clone(),
                    parent_path.to_path_buf(),
                    status.clone(),
                    *depth,
                )));
            }
        }
        info!("No file found for hashing or transfer");
        Ok(None)
    }
    async fn select_dir_hashing_with_all_child_dir_transfer_and_file_complete(
        &self,
    ) -> NdnResult<Option<(Self::ItemId, DirObject, PathBuf, ItemStatus, PathDepth)>> {
        // to continue transfer dir after all children complete
        let storage = self.lock().await;
        for (item_id, (item, parent_path, status, depth, children)) in storage.items.iter() {
            if item.is_dir()
                && status.is_hashing()
                && children.iter().all(|child_id| {
                    let (item, _, status, _, _) =
                        storage.items.get(child_id).expect("child item must exist");
                    item.is_file() && status.is_complete() || {
                        item.is_dir() && status.is_transfer() || status.is_complete()
                    }
                })
            {
                info!(
                    "select dir for hashing with all child dir transfer and file complete: {:?}, parent_path: {:?}, depth: {}, status: {:?}",
                    item, parent_path, depth, status
                );
                return Ok(Some((
                    *item_id,
                    item.clone().check_dir().clone(),
                    parent_path.to_path_buf(),
                    status.clone(),
                    *depth,
                )));
            }
        }
        info!("No directory found for hashing with all child dir transfer and file complete");
        Ok(None)
    }
    async fn select_dir_transfer(
        &self,
        depth: Option<PathDepth>,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> NdnResult<Vec<(Self::ItemId, DirObject, PathBuf, ItemStatus, PathDepth)>> {
        // to transfer dir
        let storage = self.lock().await;
        let mut result = vec![];
        for (item_id, (item, parent_path, status, item_depth, _)) in storage.items.iter() {
            if item.is_dir()
                && status.is_transfer()
                && (depth.is_none() || item_depth == depth.as_ref().unwrap())
            {
                result.push((
                    *item_id,
                    item.clone().check_dir().clone(),
                    parent_path.to_path_buf(),
                    status.clone(),
                    *item_depth,
                ));
            }
        }

        result.sort_by(|l, r| l.0.cmp(&r.0));

        let offset = offset.unwrap_or(0);
        let limit = std::cmp::min(limit.unwrap_or(usize::MAX as u64), result.len() as u64);
        let end_pos = std::cmp::min(offset + limit, result.len() as u64);
        let result = result.as_slice()[offset as usize..end_pos as usize].to_vec();

        info!(
            "select dir for transfer: found {:?} items, offset: {}, limit: {}, depth: {:?}",
            result
                .iter()
                .map(|(item_id, _, _, _, _)| item_id)
                .collect::<Vec<&u64>>(),
            offset,
            limit,
            depth
        );
        Ok(result)
    }

    async fn select_item_transfer(
        &self,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> NdnResult<Vec<(Self::ItemId, StorageItem, PathBuf, ItemStatus, PathDepth)>> {
        // to transfer
        let storage = self.lock().await;
        let mut result = vec![];
        let mut found_count = 0;
        for (item_id, (item, parent_path, status, depth, _)) in storage.items.iter() {
            if status.is_transfer() {
                found_count += 1;
                if offset.as_ref().is_none_or(|offset| found_count > *offset) {
                    result.push((
                        *item_id,
                        item.clone(),
                        parent_path.to_path_buf(),
                        status.clone(),
                        *depth,
                    ));
                    if let Some(limit) = limit {
                        if result.len() as u64 >= limit {
                            break;
                        }
                    }
                }
            }
        }

        info!(
            "select item for transfer: found {:?} items, offset: {:?}, limit: {:?}",
            result
                .iter()
                .map(|(item_id, _, _, _, _)| item_id)
                .collect::<Vec<&u64>>(),
            offset,
            limit
        );
        Ok(result)
    }
    async fn list_children_order_by_name(
        &self,
        item_id: &Self::ItemId,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> NdnResult<(
        Vec<(Self::ItemId, StorageItem, ItemStatus, PathDepth)>,
        PathBuf,
    )> {
        let storage = self.lock().await;
        if let Some((_, parent_path, _, _depth, children)) = storage.items.get(item_id) {
            let mut result = vec![];
            let mut children = children.clone();
            children.sort();
            for child_id in children.as_slice()[offset.unwrap_or(0) as usize..].iter() {
                let (item, _parent_path, status, depth, _) =
                    storage.items.get(child_id).expect("Child item must exist");
                result.push((*child_id, item.clone(), status.clone(), *depth));
                if let Some(limit) = limit {
                    if result.len() as u64 >= limit {
                        break;
                    }
                }
            }

            info!(
                "list children for item: {}, found {:?} items, offset: {:?}, limit: {:?}",
                item_id,
                result
                    .iter()
                    .map(|(item_id, _, _, _)| item_id)
                    .collect::<Vec<&u64>>(),
                offset,
                limit
            );
            Ok((result, parent_path.to_path_buf()))
        } else {
            unreachable!("Item not found in storage");
        }
    }
    async fn list_chunks_by_chunk_id(
        &self,
        chunk_ids: &[ChunkId],
    ) -> NdnResult<Vec<(Self::ItemId, ChunkItem, PathBuf, ItemStatus, PathDepth)>> {
        let storage = self.lock().await;
        let mut result = HashMap::new();
        for (item_id, (item, parent_path, status, depth, _)) in storage.items.iter() {
            if let StorageItem::Chunk(item) = item {
                let chunk_id = item.chunk_id.clone();
                if chunk_ids.contains(&chunk_id) {
                    result.insert(
                        chunk_id.to_string(),
                        (
                            *item_id,
                            item.clone(),
                            parent_path.to_path_buf(),
                            status.clone(),
                            *depth,
                        ),
                    );
                }
            }
        }

        let result = chunk_ids
            .iter()
            .map(|chunk_id| {
                result
                    .remove(&chunk_id.to_string())
                    .expect("Chunk ID must exist")
            })
            .collect::<Vec<_>>();
        info!(
            "list chunks by chunk_id: found {:?} items",
            result
                .iter()
                .map(|(item_id, _, _, _, _)| item_id)
                .collect::<Vec<&u64>>()
        );
        Ok(result)
    }
}

#[async_trait::async_trait]
impl FileSystemReader<SimulateDirReader, SimulateFileReader>
    for Arc<tokio::sync::Mutex<SimulateFsItem>>
{
    async fn info(&self, path: &std::path::Path) -> NdnResult<FileSystemItem> {
        let root = self.lock().await;

        let found = root
            .find_child(path)
            .expect("Path must exist in the simulated file system");

        Ok(match found {
            SimulateFsItem::File(file) => FileSystemItem::File(FileObject::new(
                file.name.clone(),
                file.content.len() as u64,
                "".to_string(),
            )),
            SimulateFsItem::Dir(dir) => FileSystemItem::Dir(DirObject {
                name: dir.name.clone(),
                content: "".to_string(),
                exp: 0,
                meta: None,
                owner: None,
                create_time: None,
                extra_info: HashMap::new(),
            }),
        })
    }

    async fn open_dir(&self, path: &std::path::Path) -> NdnResult<SimulateDirReader> {
        Ok(SimulateDirReader {
            root: self.clone(),
            parent_path: path.to_path_buf(),
            last_child_name: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    async fn open_file(&self, path: &std::path::Path) -> NdnResult<SimulateFileReader> {
        Ok(SimulateFileReader {
            root: self.clone(),
            file_path: path.to_path_buf(),
        })
    }
}

struct SimulateDirReader {
    root: Arc<tokio::sync::Mutex<SimulateFsItem>>,
    parent_path: PathBuf,
    last_child_name: Arc<tokio::sync::Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl FileSystemDirReader for SimulateDirReader {
    async fn next(&self, limit: Option<u64>) -> NdnResult<Vec<FileSystemItem>> {
        let root = self.root.lock().await;
        let parent = root
            .find_child(self.parent_path.as_path())
            .expect("Parent path must exist");
        let mut child_names = parent.check_dir().children.keys().collect::<Vec<_>>();
        child_names.sort();
        let mut last_child_name = self.last_child_name.lock().await;
        let next_pos = match &*last_child_name {
            Some(last_child_name) => {
                child_names
                    .iter()
                    .position(|name| last_child_name == *name)
                    .expect("last_child lost")
                    + 1
            }
            None => 0,
        };

        if next_pos >= child_names.len() {
            // No more children to select
            info!(
                "No more children to select in directory: {}",
                self.parent_path.display()
            );
            return Ok(vec![]);
        }

        let select_child_names = &child_names.as_slice()[next_pos
            ..std::cmp::min(
                child_names.len(),
                next_pos + limit.unwrap_or((u32::MAX / 2) as u64) as usize,
            )];

        if select_child_names.is_empty() {
            info!(
                "No more children to select in directory: {}",
                self.parent_path.display()
            );
            return Ok(vec![]);
        }

        *last_child_name = Some(select_child_names.last().unwrap().to_string());

        let parent_dir = parent.check_dir();

        let childs = select_child_names
            .iter()
            .map(|name| {
                let child_item = parent_dir
                    .children
                    .get(*name)
                    .expect("Child item must exist");
                match child_item {
                    SimulateFsItem::File(file) => FileSystemItem::File(FileObject::new(
                        file.name.clone(),
                        file.content.len() as u64,
                        "".to_string(),
                    )),
                    SimulateFsItem::Dir(dir) => FileSystemItem::Dir(DirObject {
                        name: dir.name.clone(),
                        content: "".to_string(),
                        exp: 0,
                        meta: None,
                        owner: None,
                        create_time: None,
                        extra_info: HashMap::new(),
                    }),
                }
            })
            .collect::<Vec<_>>();

        info!(
            "Selected children from directory: {}, found: {:?}, last_child_name: {:?}, limit: {:?}",
            self.parent_path.display(),
            childs.iter().map(|child| child.name()).collect::<Vec<_>>(),
            *last_child_name,
            limit
        );

        Ok(childs)
    }
}

struct SimulateFileReader {
    root: Arc<tokio::sync::Mutex<SimulateFsItem>>,
    file_path: PathBuf,
}

#[async_trait::async_trait]
impl FileSystemFileReader for SimulateFileReader {
    async fn read_chunk(&self, offset: SeekFrom, limit: Option<u64>) -> NdnResult<Vec<u8>> {
        let root = self.root.lock().await;
        let file = root
            .find_child(self.file_path.as_path())
            .expect("expect a file")
            .check_file();
        let pos = match offset {
            SeekFrom::Start(pos) => pos,
            SeekFrom::End(_) => unimplemented!(),
            SeekFrom::Current(_) => unreachable!(),
        };
        let chunk = file.content.as_slice()[(pos as usize)
            ..std::cmp::min(
                file.content.len(),
                (pos + limit.unwrap_or((u32::MAX / 2) as u64)) as usize,
            )]
            .to_vec();
        Ok(chunk)
    }
}

#[async_trait::async_trait]
impl FileSystemWriter<SimulateFileWriter> for Arc<tokio::sync::Mutex<SimulateFsItem>> {
    async fn create_dir_all(&self, dir_path: &Path) -> NdnResult<()> {
        let paths = dir_path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let mut root = self.lock().await;
        let mut found = &mut *root;
        for child_name in paths {
            found = found
                .check_dir_mut()
                .children
                .entry(child_name.clone())
                .or_insert_with(|| {
                    info!("Creating directory: {}", child_name);
                    SimulateFsItem::Dir(SimulateDir {
                        name: child_name,
                        children: HashMap::new(),
                    })
                });
        }
        Ok(())
    }

    async fn create_dir(&self, dir: &DirObject, parent_path: &Path) -> NdnResult<()> {
        let mut root = self.lock().await;
        let parent_item = root
            .find_child_mut(parent_path)
            .expect("Parent path must exist");
        parent_item
            .check_dir_mut()
            .children
            .entry(dir.name.clone())
            .or_insert_with(|| {
                info!(
                    "Creating directory: {}, parent: {:?}",
                    dir.name, parent_path
                );
                // Create a new SimulateDir with the name from DirObject
                SimulateFsItem::Dir(SimulateDir {
                    name: dir.name.clone(),
                    children: HashMap::new(),
                })
            });
        Ok(())
    }

    async fn open_file(
        &self,
        file: &FileObject,
        parent_path: &Path,
    ) -> NdnResult<SimulateFileWriter> {
        let mut root = self.lock().await;
        let parent_item = root
            .find_child_mut(parent_path)
            .expect("Parent path must exist");
        parent_item
            .check_dir_mut()
            .children
            .entry(file.name.clone())
            .or_insert_with(|| {
                info!("Creating file: {}, parent: {:?}", file.name, parent_path);
                SimulateFsItem::File(SimulateFile {
                    name: file.name.clone(),
                    content: vec![],
                })
            });
        Ok(SimulateFileWriter {
            root: self.clone(),
            file_path: parent_path.join(file.name.as_str()),
        })
    }
}

struct SimulateFileWriter {
    root: Arc<tokio::sync::Mutex<SimulateFsItem>>,
    file_path: PathBuf,
}

#[async_trait::async_trait]
impl FileSystemFileWriter for SimulateFileWriter {
    async fn length(&self) -> NdnResult<u64> {
        let root = self.root.lock().await;
        let file = root
            .find_child(self.file_path.as_path())
            .expect("expect a file")
            .check_file();
        Ok(file.content.len() as u64)
    }
    async fn write_chunk(&self, chunk_data: &[u8], offset: SeekFrom) -> NdnResult<()> {
        let mut root = self.root.lock().await;
        let file = root
            .find_child_mut(self.file_path.as_path())
            .expect("expect a file")
            .check_file_mut();
        let pos = match offset {
            SeekFrom::Start(pos) => pos,
            SeekFrom::End(_) => unimplemented!(),
            SeekFrom::Current(_) => unreachable!(),
        };
        if pos as usize > file.content.len() {
            panic!("Write position out of bounds");
        }
        if pos as usize + chunk_data.len() > file.content.len() {
            file.content.resize(pos as usize + chunk_data.len(), 0);
        }
        file.content[pos as usize..(pos as usize + chunk_data.len())].copy_from_slice(chunk_data);
        Ok(())
    }
}

#[derive(Clone)]
struct Local2NdnWriter {
    local_named_mgr_id: String,
    target_named_mgr_id: String,
}

#[async_trait::async_trait]
impl NdnWriter for Local2NdnWriter {
    async fn push_object(&self, obj_id: &ObjId, obj_str: &str) -> NdnResult<Vec<ObjId>> {
        // lost child-obj-list
        match obj_id.obj_type.as_str() {
            OBJ_TYPE_FILE => {
                let file_obj: FileObject =
                    serde_json::from_str(obj_str).expect("Invalid file object");
                let chunk_list_id =
                    ObjId::try_from(file_obj.content.as_str()).expect("Invalid chunk list id");
                assert!(
                    chunk_list_id.obj_type.as_str() == OBJ_TYPE_CHUNK_LIST_FIX_SIZE
                        || chunk_list_id.obj_type.as_str() == OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE,
                    "Chunk list id must be of type ChunkList"
                );
                let lost_obj = NamedDataMgr::get_object(
                    Some(self.target_named_mgr_id.as_str()),
                    &chunk_list_id,
                    None,
                )
                .await;
                match lost_obj {
                    Ok(_) => {
                        info!(
                            "Chunklist {} already exists, push file-obj: {}",
                            chunk_list_id.to_string(),
                            obj_id.to_string()
                        );
                        NamedDataMgr::put_object(
                            Some(self.target_named_mgr_id.as_str()),
                            obj_id,
                            obj_str,
                        )
                        .await
                        .expect("Failed to put file object");
                        return Ok(vec![]);
                    }
                    Err(e) => match e {
                        NdnError::NotFound(_) => {
                            info!(
                                "Chunklist {} for file {} not found, please push it first",
                                chunk_list_id.to_string(),
                                obj_id.to_string()
                            );
                            return Ok(vec![chunk_list_id.clone()]);
                        }
                        _ => {
                            panic!("Failed to get object: {}", e);
                        }
                    },
                }
            }
            OBJ_TYPE_DIR => {
                let dir_obj: DirObject = serde_json::from_str(obj_str).expect("Invalid dir object");
                let dir_obj_map_id =
                    ObjId::try_from(dir_obj.content.as_str()).expect("Invalid dir object map id");

                let lost_obj_map = NamedDataMgr::get_object(
                    Some(self.target_named_mgr_id.as_str()),
                    &dir_obj_map_id,
                    None,
                )
                .await;
                let err = match lost_obj_map {
                    Ok(obj_map_json) => {
                        // todo: should check from target_named_mgr_id, but now it's global, so it will success always
                        let obj_map = TrieObjectMap::open(obj_map_json).await;
                        match obj_map {
                            Ok(_) => {
                                info!(
                                    "ObjectMap {} already exists, push directory {}",
                                    dir_obj_map_id.to_string(),
                                    obj_id.to_string()
                                );
                                NamedDataMgr::put_object(
                                    Some(self.target_named_mgr_id.as_str()),
                                    obj_id,
                                    obj_str,
                                )
                                .await
                                .expect("Failed to put dir object");
                                return Ok(vec![]);
                            }
                            Err(e) => e,
                        }
                    }
                    Err(e) => e,
                };

                match err {
                    NdnError::NotFound(_) => {
                        info!(
                            "ObjectMap {} for directory {} not found, please push it first",
                            dir_obj_map_id, obj_id
                        );
                        return Ok(vec![dir_obj_map_id.clone()]);
                    }
                    _ => {
                        panic!("Failed to get object: {}", err);
                    }
                }
            }
            _ => unreachable!("Unsupported object type: {}", obj_id.obj_type),
        }
    }
    async fn push_chunk(&self, chunk_id: &ChunkId, chunk_data: &[u8]) -> NdnResult<()> {
        write_chunk(self.target_named_mgr_id.as_str(), chunk_id, chunk_data).await;
        Ok(())
    }
    async fn push_container(&self, container_id: &ObjId) -> NdnResult<Vec<ObjId>> {
        // lost child-obj-list
        match container_id.obj_type.as_str() {
            OBJ_TYPE_CHUNK_LIST_FIX_SIZE | OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE => {
                let chunk_list_json = NamedDataMgr::get_object(
                    Some(self.local_named_mgr_id.as_str()),
                    container_id,
                    None,
                )
                .await
                .expect("Failed to get chunk list from local named manager");

                let chunk_list = ChunkListBuilder::open(chunk_list_json.clone())
                    .await
                    .expect("Failed to open chunk list")
                    .build()
                    .await
                    .expect("Failed to build chunk list");

                let mut lost_child_obj_ids = vec![];
                for item in chunk_list.iter() {
                    let obj_id = item.to_obj_id();
                    let ret = NamedDataMgr::open_chunk_reader(
                        Some(self.target_named_mgr_id.as_str()),
                        &item,
                        SeekFrom::Start(0),
                        false,
                    )
                    .await;
                    match ret {
                        Ok(_) => {
                            info!(
                                "Object already exists, skipping push: {:?}",
                                item.to_string()
                            );
                        }
                        Err(e) => match e {
                            NdnError::NotFound(_) => {
                                info!(
                                    "Object not found, pushing new object: {:?}",
                                    obj_id.to_string()
                                );
                                lost_child_obj_ids.push(obj_id);
                            }
                            _ => {
                                panic!("Failed to get object: {}", e);
                            }
                        },
                    }
                }

                if lost_child_obj_ids.is_empty() {
                    info!(
                        "all chunks for {} already exist, push it",
                        container_id.to_string()
                    );
                    let (recalc_container_id, container_str) = build_named_object_by_json(
                        container_id.obj_type.as_str(),
                        &chunk_list_json,
                    );
                    assert_eq!(
                        recalc_container_id,
                        *container_id,
                        "Chunk list ID mismatch: expected {}, got {}",
                        container_id.to_string(),
                        recalc_container_id.to_string()
                    );
                    NamedDataMgr::put_object(
                        Some(self.target_named_mgr_id.as_str()),
                        container_id,
                        container_str.as_str(),
                    )
                    .await
                    .expect("Failed to put chunk list");
                    // todo: should push object array to target_named_mgr_id, but now it's global, so it's no useable
                } else {
                    info!(
                        "{} chunk for {} lost, please push them first, {:?}",
                        lost_child_obj_ids.len(),
                        container_id.to_string(),
                        lost_child_obj_ids
                            .iter()
                            .map(|id| id.to_string())
                            .collect::<Vec<_>>()
                    );
                }

                Ok(lost_child_obj_ids)
            }
            OBJ_TYPE_TRIE | OBJ_TYPE_TRIE_SIMPLE => {
                let obj_map_json = NamedDataMgr::get_object(
                    Some(self.local_named_mgr_id.as_str()),
                    container_id,
                    None,
                )
                .await
                .expect("Failed to get object map from local named manager");

                let (got_obj_map_id, obj_map_str) =
                    build_named_object_by_json(container_id.obj_type.as_str(), &obj_map_json);
                assert_eq!(
                    got_obj_map_id,
                    *container_id,
                    "Object map ID mismatch: expected {}, got {}",
                    container_id.to_string(),
                    got_obj_map_id.to_string()
                );

                let obj_map = TrieObjectMap::open(obj_map_json)
                    .await
                    .expect("Failed to open object map");
                let all_obj_ids = obj_map
                    .iter()
                    .expect("Failed to iterate object map")
                    .map(|item| item.1.clone())
                    .collect::<Vec<_>>();
                let mut lost_child_obj_ids = vec![];
                for item in all_obj_ids {
                    let ret = NamedDataMgr::get_object(
                        Some(self.target_named_mgr_id.as_str()),
                        &item,
                        None,
                    )
                    .await;
                    match ret {
                        Ok(_) => {
                            info!("Object already exists, skipping push: {}", item.to_string());
                        }
                        Err(e) => match e {
                            NdnError::NotFound(_) => {
                                info!("Object not found, pushing new object: {}", item.to_string());
                                lost_child_obj_ids.push(item);
                            }
                            _ => {
                                panic!("Failed to get object: {}", e);
                            }
                        },
                    }
                }

                // todo: should push object map to target_named_mgr_id, but now it's global, so it's no useable
                if lost_child_obj_ids.is_empty() {
                    info!(
                        "No new objects to push for object map: {}, push it",
                        container_id
                    );
                    NamedDataMgr::put_object(
                        Some(self.target_named_mgr_id.as_str()),
                        container_id,
                        obj_map_str.as_str(),
                    )
                    .await
                    .expect("Failed to put object map");
                } else {
                    info!(
                        "{} sub-obj for {} lost, please push them first, {:?}",
                        lost_child_obj_ids.len(),
                        container_id,
                        lost_child_obj_ids
                            .iter()
                            .map(|id| id.to_string())
                            .collect::<Vec<_>>()
                    );
                }

                Ok(lost_child_obj_ids)
            }
            _ => unreachable!("Unsupported object type: {}", container_id.obj_type),
        }
    }
}

struct LocalNdnReader {
    ndn_mgr_id: String,
}

#[async_trait::async_trait]
impl NdnReader for LocalNdnReader {
    async fn get_object(&self, obj_id: &ObjId) -> NdnResult<Value> {
        NamedDataMgr::get_object(Some(self.ndn_mgr_id.as_str()), obj_id, None).await
    }
    async fn get_chunk(&self, chunk_id: &ChunkId) -> NdnResult<Vec<u8>> {
        Ok(read_chunk(self.ndn_mgr_id.as_str(), chunk_id).await)
    }
    async fn get_container(&self, container_id: &ObjId) -> NdnResult<Value> {
        match container_id.obj_type.as_str() {
            OBJ_TYPE_CHUNK_LIST_FIX_SIZE | OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE => {
                let chunk_list_json =
                    NamedDataMgr::get_object(Some(self.ndn_mgr_id.as_str()), container_id, None)
                        .await
                        .expect("Failed to get chunk list from NDN manager");
                return Ok(chunk_list_json);
            }
            OBJ_TYPE_TRIE => {
                let obj_map_json =
                    NamedDataMgr::get_object(Some(self.ndn_mgr_id.as_str()), container_id, None)
                        .await
                        .expect("Failed to get object map from NDN manager");
                return Ok(obj_map_json);
            }
            _ => unreachable!("Unsupported object type: {}", container_id.obj_type),
        }
    }
}

async fn check_simulate_fs_eq_object(
    fs_root_item: &SimulateFsItem,
    obj_id: &ObjId,
    ndn_mgr_id: &str,
    ndn_client: &NdnClient,
    obj_host_url: &str,
) {
    let mut traverse_stack = vec![];

    let check_item = async |item: &SimulateFsItem,
                            obj_id: &ObjId,
                            ndn_mgr_id: &str,
                            ndn_client: &NdnClient,
                            obj_host_url: &str|
           -> Option<ObjId> {
        info!(
            "check item: {}, obj_id: {}",
            item.name(),
            obj_id.to_string()
        );
        match item {
            SimulateFsItem::File(file) => {
                let obj_json = NamedDataMgr::get_object(Some(ndn_mgr_id), obj_id, None)
                    .await
                    .expect("Failed to get object");
                let file_obj =
                    serde_json::from_value::<FileObject>(obj_json).expect("Invalid file object");
                assert_eq!(file.name, file_obj.name, "File name mismatch");
                assert_eq!(
                    file.content.len() as u64,
                    file_obj.size,
                    "File size mismatch"
                );
                let o_link_url = format!("http://{}/ndn/{}/content", obj_host_url, obj_id);
                let chunk_list_id =
                    ObjId::try_from(file_obj.content.as_str()).expect("Invalid chunk list id");
                let (mut reader, resp_headers) = ndn_client
                    .open_chunk_reader_by_url(o_link_url.as_str(), None, None)
                    .await
                    .expect("Failed to open chunk reader");
                assert_eq!(
                    resp_headers.obj_size,
                    Some(file_obj.size),
                    "Response headers size mismatch"
                );
                assert_eq!(
                    resp_headers.obj_id,
                    Some(chunk_list_id),
                    "Response headers object ID mismatch"
                );
                assert_eq!(
                    resp_headers.root_obj_id,
                    Some(obj_id.clone()),
                    "Response headers root object ID mismatch"
                );
                let mut content = vec![];
                let read_len = reader
                    .read_to_end(&mut content)
                    .await
                    .expect("Failed to read content");
                assert_eq!(read_len as u64, file_obj.size, "Read content size mismatch");
                assert_eq!(content, file.content, "File content mismatch");
                None
            }
            SimulateFsItem::Dir(dir) => {
                let obj_json = NamedDataMgr::get_object(Some(ndn_mgr_id), obj_id, None)
                    .await
                    .expect("Failed to get dir object");
                let dir_obj =
                    serde_json::from_value::<DirObject>(obj_json).expect("Invalid dir object");
                assert_eq!(dir.name, dir_obj.name, "Directory name mismatch");
                let children_obj_map_id =
                    ObjId::try_from(dir_obj.content.as_str()).expect("Invalid dir object map id");

                let children_obj_map_json =
                    NamedDataMgr::get_object(Some(ndn_mgr_id), &children_obj_map_id, None)
                        .await
                        .expect("Failed to get children object map");

                let obj_map = TrieObjectMap::open(children_obj_map_json)
                    .await
                    .expect("Failed to open dir object map");
                assert_eq!(
                    obj_map
                        .iter()
                        .expect("Failed to iterate object map")
                        .count(),
                    dir.children.len(),
                    "Directory children count mismatch"
                );
                Some(children_obj_map_id)
            }
        }
    };

    match check_item(fs_root_item, obj_id, ndn_mgr_id, ndn_client, obj_host_url).await {
        Some(children_obj_map_id) => {
            traverse_stack.push((
                fs_root_item.check_dir().children.iter(),
                children_obj_map_id,
            ));
        }
        None => return,
    }

    while traverse_stack.len() > 0 {
        let stack_top = traverse_stack
            .last_mut()
            .expect("Traverse stack must not be empty");
        let dir_obj_map_id = stack_top.1.clone();
        let next_children = stack_top.0.next();
        match next_children {
            Some((_child_name, child_item)) => {
                let children_obj_map_json =
                    NamedDataMgr::get_object(Some(ndn_mgr_id), &dir_obj_map_id, None)
                        .await
                        .expect("Failed to get children object map");

                let obj_map = TrieObjectMap::open(children_obj_map_json)
                    .await
                    .expect("Failed to open dir object map");
                let next_obj_id = obj_map
                    .get_object(child_item.name())
                    .expect("Child item must exist in object map")
                    .expect("Child should exist in object map");

                if let Some(next_children_obj_map_id) = check_item(
                    child_item,
                    &next_obj_id,
                    ndn_mgr_id,
                    ndn_client,
                    obj_host_url,
                )
                .await
                {
                    traverse_stack.push((
                        child_item.check_dir().children.iter(),
                        next_children_obj_map_id,
                    ));
                }
            }
            None => {
                traverse_stack.pop(); // Finished with this item, pop it from the stack
                                      // No more children, continue with the next item in the stack
            }
        }
    }
}

fn check_simulate_fs_eq(left: &SimulateFsItem, right: &SimulateFsItem) {
    let mut traverse_stack = vec![];

    let check_item = |left: &SimulateFsItem, right: &SimulateFsItem| -> bool {
        match left {
            SimulateFsItem::File(file) => {
                let right = right.check_file();
                assert_eq!(file.name, right.name, "File name mismatch");
                assert_eq!(file.content, right.content, "File content mismatch");
                false // No children to traverse
            }
            SimulateFsItem::Dir(dir) => {
                let right = right.check_dir();
                assert_eq!(dir.name, right.name, "Directory name mismatch");
                assert_eq!(
                    right.children.len(),
                    dir.children.len(),
                    "Directory children count mismatch"
                );
                true
            }
        }
    };

    if check_item(left, right) {
        traverse_stack.push((left.check_dir().children.iter(), right));
    }

    while traverse_stack.len() > 0 {
        let stack_top = traverse_stack
            .last_mut()
            .expect("Traverse stack must not be empty");
        let right_dir = stack_top.1.check_dir();
        let left_next_child = stack_top.0.next();
        match left_next_child {
            Some((_child_name, child_item)) => {
                let right_child = right_dir
                    .children
                    .get(child_item.name())
                    .expect("Child item must exist in right dir");
                if check_item(child_item, right_child) {
                    traverse_stack.push((child_item.check_dir().children.iter(), right_child));
                }
            }
            None => {
                traverse_stack.pop(); // Finished with this item, pop it from the stack
                                      // No more children, continue with the next item in the stack
            }
        }
    }
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_build() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    let obj_array_dir = init_obj_array_storage_factory().await;
    let obj_map_dir = init_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;
    let backup_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (backup_ndn_client, backup_ndn_host) = init_ndn_server(backup_ndn_mgr_id.as_str()).await;

    // backup
    let simulate_dir = gen_random_simulate_dir(4, 4, 1000 * 1000);

    info!("Simulate dir: {:?}", simulate_dir);

    let simulate_dir = Arc::new(tokio::sync::Mutex::new(simulate_dir));
    let storage = Arc::new(tokio::sync::Mutex::new(MemoryStorage::new()));
    let root_path = PathBuf::from(simulate_dir.lock().await.name());
    let root_item_id = file_system_to_ndn(
        Some(root_path.as_path()),
        Local2NdnWriter {
            local_named_mgr_id: ndn_mgr_id.clone(),
            target_named_mgr_id: backup_ndn_mgr_id.clone(),
        },
        simulate_dir.clone(),
        storage.clone(),
        4000,
        ndn_mgr_id.as_str(),
    )
    .await
    .expect("Failed to build NDN local dir trie object map");

    let root_obj_id = {
        let storage_guard = storage.lock().await;
        assert!(
            storage_guard.items.contains_key(&root_item_id),
            "Root item must exist"
        );
        let (root_item, _, status, depth, _children) = storage_guard
            .items
            .get(&root_item_id)
            .expect("Root item must exist");
        assert!(root_item.is_dir(), "Root item must be a directory");
        assert_eq!(depth, &0, "Root item depth must be 0");
        assert!(
            status.is_complete(),
            "Root item status must be complete, {:?}",
            status
        );
        status
            .get_obj_id()
            .expect("Root item must have an ObjId")
            .clone()
    };

    check_simulate_fs_eq_object(
        &*simulate_dir.lock().await,
        &root_obj_id,
        backup_ndn_mgr_id.as_str(),
        &backup_ndn_client,
        &backup_ndn_host,
    )
    .await;

    // restore
    let restore_simulate_dir = Arc::new(tokio::sync::Mutex::new(gen_random_simulate_dir(1, 0, 0)));
    let restore_storage = Arc::new(tokio::sync::Mutex::new(MemoryStorage::new()));
    let restore_root_path = PathBuf::from(restore_simulate_dir.lock().await.name());
    let root_item_id = ndn_to_file_system(
        Some((restore_root_path.as_path(), &root_obj_id)),
        restore_simulate_dir.clone(),
        LocalNdnReader {
            ndn_mgr_id: backup_ndn_mgr_id.clone(),
        },
        restore_storage.clone(),
    )
    .await
    .expect("Failed to build NDN local dir trie object map");

    let restore_root_obj_id = {
        let storage_guard = storage.lock().await;
        assert!(
            storage_guard.items.contains_key(&root_item_id),
            "Root item must exist"
        );
        let (root_item, _, status, depth, _children) = storage_guard
            .items
            .get(&root_item_id)
            .expect("Root item must exist");
        assert!(root_item.is_dir(), "Root item must be a directory");
        assert_eq!(depth, &0, "Root item depth must be 0");
        assert!(status.is_complete(), "Root item status must be Scanning");
        status
            .get_obj_id()
            .expect("Root item must have an ObjId")
            .clone()
    };

    assert_eq!(
        restore_root_obj_id, root_obj_id,
        "Restored root object ID must match the original"
    );

    {
        let orignal_simulate_dir = simulate_dir.lock().await;
        let restore_simulate_dir_guard = restore_simulate_dir.lock().await;
        let restore_simulate_dir = restore_simulate_dir_guard
            .check_dir()
            .children
            .get(orignal_simulate_dir.name())
            .expect("Original simulate dir must exist");

        check_simulate_fs_eq(&*orignal_simulate_dir, restore_simulate_dir);
    }

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_add_file() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_remove_file() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_append_file() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_insert_head_file() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_insert_random_file() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_trancation_file() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_remove_head_file() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_remove_random_file() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_add_dir() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}

#[tokio::test]
async fn ndn_local_dir_trie_obj_map_remove_dir() {
    init_logging("ndn_local_dir_trie_obj_map_build", false);

    info!("ndn_local_dir_trie_obj_map_build test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    info!("ndn_local_dir_trie_obj_map_build test end.");
}
