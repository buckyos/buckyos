use super::proof::ObjectMapItemProof;
use super::storage::{ObjectMapInnerStorage, ObjectMapStorageType};
use super::GLOBAL_OBJECT_MAP_STORAGE_FACTORY;
use crate::coll::CollectionStorageMode;
use crate::mtree::{MerkleTreeObject, MerkleTreeObjectGenerator};
use crate::mtree::{
    MtreeReadSeek, MtreeReadWriteSeekWithSharedBuffer, MtreeWriteSeek, SharedBuffer,
};
use crate::object::ObjId;
use crate::{
    build_named_object_by_json,
    hash::{HashHelper, HashMethod},
    NdnError, NdnResult,
};
use crate::{
    Base32Codec, MerkleTreeProofPathVerifier, ObjectMapStorageOpenMode, OBJ_TYPE_OBJMAP,
    OBJ_TYPE_OBJMAP_SIMPLE,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ObjectMapItem {
    pub key: String,
    pub obj_id: ObjId,
}

impl ObjectMapItem {
    pub fn new(key: impl Into<String>, obj_id: ObjId) -> Self {
        Self {
            key: key.into(),
            obj_id,
        }
    }

    pub fn calc_hash(&self, hash_method: HashMethod) -> Vec<u8> {
        HashHelper::calc_hash_list(
            hash_method,
            &[self.key.as_bytes(), self.obj_id.obj_hash.as_slice()],
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ObjectMapBody {
    pub root_hash: String, // The root hash of the merkle tree, encode as base32
    pub hash_method: HashMethod,
    pub total_count: u64, // Total count of items in the object map
}

impl ObjectMapBody {
    pub fn calc_obj_id(&self) -> (ObjId, String) {
        let obj_type = match CollectionStorageMode::select_mode(Some(self.total_count)) {
            CollectionStorageMode::Simple => OBJ_TYPE_OBJMAP_SIMPLE,
            CollectionStorageMode::Normal => OBJ_TYPE_OBJMAP,
        };

        let body = serde_json::to_value(self).expect("Failed to serialize ObjectMapBody");
        build_named_object_by_json(obj_type, &body)
    }

    pub fn get_storage_type(&self) -> ObjectMapStorageType {
        let mode = CollectionStorageMode::select_mode(Some(self.total_count));
        ObjectMapStorageType::select_storage_type(Some(mode))
    }
}

pub struct ObjectMap {
    obj_id: ObjId,
    body: ObjectMapBody,
    storage: Arc<Box<dyn ObjectMapInnerStorage>>,
    mtree: Arc<Mutex<MerkleTreeObject>>,
}

impl ObjectMap {
    // Create object map from builder
    pub(crate) fn new(
        obj_id: ObjId,
        body: ObjectMapBody,
        storage: Box<dyn ObjectMapInnerStorage>,
        mtree: MerkleTreeObject,
    ) -> Self {
        Self {
            obj_id,
            body,
            storage: Arc::new(storage),
            mtree: Arc::new(Mutex::new(mtree)),
        }
    }

    pub fn storage_type(&self) -> ObjectMapStorageType {
        self.body.get_storage_type()
    }

    // Load object map from storage
    pub async fn open(obj_data: serde_json::Value) -> NdnResult<Self> {
        let body: ObjectMapBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let (obj_id, _) = body.calc_obj_id();

        let mut storage = GLOBAL_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open(
                Some(&obj_id),
                true, // Always open in read-only mode for object map
                Some(body.get_storage_type()),
                ObjectMapStorageOpenMode::OpenExisting,
            )
            .await
            .map_err(|e| {
                let msg = format!(
                    "Error opening object map storage: {}, {}",
                    body.root_hash, e
                );
                error!("{}", msg);
                e
            })?;

        // Try load mtree data from storage
        let ret = storage.load_mtree_data().map_err(|e| {
            let msg = format!("Error loading mtree data: {}", e);
            error!("{}", msg);
            e
        })?;

        let mtree = if let Some(data) = ret {
            let stream = std::io::Cursor::new(data);
            let reader = Box::new(stream) as Box<dyn MtreeReadSeek>;

            // TODO should we verify the mtree data on loading? and if error happens, should we regenerate the mtree or return error?
            let mtree = MerkleTreeObject::load_from_reader(reader, false)
                .await
                .map_err(|e| {
                    let msg = format!("Error loading mtree object: {}", e);
                    error!("{}", msg);
                    e
                })?;

            mtree
        } else {
            Self::regenerate_merkle_tree(&mut storage, body.hash_method, true).await?
        };

        Ok(Self {
            obj_id,
            body,
            storage: Arc::new(storage),
            mtree: Arc::new(Mutex::new(mtree)),
        })
    }

    pub fn get_obj_id(&self) -> &ObjId {
        &self.obj_id
    }

    pub fn calc_obj_id(&self) -> (ObjId, String) {
        self.body.calc_obj_id()
    }

    pub fn hash_method(&self) -> HashMethod {
        self.body.hash_method
    }

    pub fn len(&self) -> u64 {
        self.body.total_count
    }

    pub fn get_object(&self, key: &str) -> NdnResult<Option<ObjId>> {
        let ret = self.storage.get(key)?;
        if ret.is_none() {
            return Ok(None);
        }

        Ok(Some(ret.unwrap().0))
    }

    pub async fn get_object_proof_path(&self, key: &str) -> NdnResult<Option<ObjectMapItemProof>> {
        // Get object and mtree index from storage
        let ret = self.storage.get(key)?;
        if ret.is_none() {
            return Ok(None);
        }

        let (item, leaf_index) = ret.unwrap();
        if leaf_index.is_none() {
            let msg = format!("Object mtree leaf index is empty: {}", key);
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        let leaf_index = leaf_index.unwrap();
        let mut mtree = self.mtree.lock().await;

        let proof = mtree
            .get_proof_path_by_leaf_index(leaf_index)
            .await
            .map_err(|e| {
                let msg = format!("Error getting proof path: {}, {}", key, e);
                error!("{}", msg);
                e
            })?;

        let proof = ObjectMapItemProof {
            item: ObjectMapItem::new(key, item),
            proof,
        };

        Ok(Some(proof))
    }

    pub fn is_object_exist(&self, key: &str) -> NdnResult<bool> {
        self.storage.is_exist(&key)
    }

    pub fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (String, ObjId, Option<u64>)> + 'a> {
        let iter = self.storage.iter();
        Box::new(iter)
    }

    // Regenerate the merkle tree
    // If read_only is true, it will not update the mtree index in storage
    pub(crate) async fn regenerate_merkle_tree(
        storage: &mut Box<dyn ObjectMapInnerStorage>,
        hash_method: HashMethod,
        read_only: bool,
    ) -> NdnResult<MerkleTreeObject> {
        let count = storage.stat()?.total_count;
        let leaf_size = 256u64;
        let data_size = count as u64 * leaf_size;

        // TODO now use the memory buffer to store the merkle tree, need to optimize for further usage
        let buf_size = MerkleTreeObjectGenerator::estimate_output_bytes(
            data_size,
            leaf_size,
            Some(hash_method),
        );
        let buf = SharedBuffer::with_size(buf_size as usize);
        let stream = MtreeReadWriteSeekWithSharedBuffer::new(buf);
        let mtree_writer = Box::new(stream.clone()) as Box<dyn MtreeWriteSeek>;

        let mut mtree_generator =
            MerkleTreeObjectGenerator::new(data_size, leaf_size, Some(hash_method), mtree_writer)
                .await?;

        let mut page_index = 0;
        let page_size = 128;
        let mut leaf_index = 0;
        loop {
            let list = storage.list(page_index, page_size)?;
            if list.is_empty() {
                break;
            }
            page_index += 1;

            for key in list {
                let item = storage.get(&key)?;
                if item.is_none() {
                    let msg = format!("Error getting object map item: {}", key);
                    error!("{}", msg);
                    return Err(NdnError::InvalidState(msg));
                }

                let item = item.unwrap().0;
                let hash = HashHelper::calc_hash_list(
                    hash_method,
                    &[key.as_bytes(), item.obj_hash.as_slice()],
                );

                mtree_generator
                    .append_leaf_hashes(&vec![hash])
                    .await
                    .map_err(|e| {
                        let msg = format!("Error appending leaf hashes: {}, {}", key, e);
                        error!("{}", msg);
                        e
                    })?;

                if !read_only {
                    // Update the mtree index in storage
                    storage.update_mtree_index(&key, leaf_index).map_err(|e| {
                        let msg =
                            format!("Error updating mtree index: {}, {}, {}", key, leaf_index, e);
                        error!("{}", msg);
                        e
                    })?;
                }

                leaf_index += 1;
            }
        }

        let root_hash = mtree_generator.finalize().await?;
        info!("Regenerated merkle tree root hash: {:?}", root_hash);

        // Create the merkle tree object from the stream
        let mut stream = stream.clone();
        stream.seek(SeekFrom::Start(0)).await.map_err(|e| {
            let msg = format!("Error seeking to start: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let reader = Box::new(stream) as Box<dyn MtreeReadSeek>;
        let object = MerkleTreeObject::load_from_reader(reader, true).await?;
        let root_hash1 = object.get_root_hash();
        assert_eq!(root_hash, root_hash1);

        Ok(object)
    }

    // Get the storage file path for the object map
    // This will return None if the object ID is not generated yet
    // The target file must be created by the `save()` method
    pub fn get_storage_file_path(&self) -> Option<PathBuf> {
        let obj_id = self.get_obj_id();

        if self.storage_type() == ObjectMapStorageType::Memory {
            return None; // Memory storage does not have a file path
        }

        let factory = GLOBAL_OBJECT_MAP_STORAGE_FACTORY.get().unwrap();
        let file_path = factory.get_file_path_by_id(Some(&obj_id), self.storage_type());
        Some(file_path)
    }

    pub async fn clone(&self) -> Self {
        Self {
            obj_id: self.obj_id.clone(),
            body: self.body.clone(),
            storage: self.storage.clone(),
            mtree: self.mtree.clone(),
        }
    }

    pub(crate) async fn clone_storage_for_modify(
        &self,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        let obj_id = self.get_obj_id();

        let mut new_storage = GLOBAL_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .clone(&obj_id, &**self.storage, false)
            .await
            .map_err(|e| {
                let msg = format!("Error cloning object map storage: {}", e);
                error!("{}", msg);
                e
            })?;

        Ok(new_storage)
    }
}
