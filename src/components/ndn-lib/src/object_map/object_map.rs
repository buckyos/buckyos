use super::storage::{ObjectMapInnerStorage, ObjectMapStorageType};
use super::GLOBAL_OBJECT_MAP_STORAGE_FACTORY;
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
use crate::{Base32Codec, MerkleTreeProofPathVerifier, ObjectMapStorageOpenMode, OBJ_TYPE_OBJMAP};
use core::hash;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::SeekFrom;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

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

    /*
    pub fn encode(&self) -> NdnResult<Vec<u8>> {
        let bytes = bincode::serialize(self).map_err(|e| {
            let msg = format!("Error serializing ObjectMapItem: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(bytes)
    }

    pub fn decode(data: &[u8]) -> NdnResult<Self> {
        let ret = bincode::deserialize(data).map_err(|e| {
            let msg = format!("Error deserializing ObjectMapItem: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(ret)
    }
    */
}

#[derive(Debug, Clone)]
pub struct ObjectMapItemProof {
    pub item: ObjectMapItem,
    pub proof: Vec<(u64, Vec<u8>)>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ObjectMapBody {
    pub root_hash: String, // The root hash of the merkle tree, encode as base32
    pub hash_method: HashMethod,
    pub storage_type: ObjectMapStorageType,
}

pub struct ObjectMap {
    pub hash_method: HashMethod,
    pub is_dirty: bool,
    pub storage: Box<dyn ObjectMapInnerStorage>,
    pub mtree: Option<MerkleTreeObject>,
    pub obj_id: Option<ObjId>, // Cache the object ID for the object map, updated on flush
}

impl ObjectMap {
    // Create empty object map
    pub async fn new(
        hash_method: HashMethod,
        storage_type: Option<ObjectMapStorageType>,
    ) -> NdnResult<Self> {
        let mut storage = GLOBAL_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open(
                None,
                false,
                storage_type,
                ObjectMapStorageOpenMode::CreateNew,
            )
            .await
            .map_err(|e| {
                let msg = format!("Error opening object map storage: {}", e);
                error!("{}", msg);
                e
            })?;

        Ok(Self {
            hash_method,
            is_dirty: false,
            storage,
            mtree: None,
            obj_id: None,
        })
    }

    pub fn is_read_only(&self) -> bool {
        self.storage.is_readonly()
    }

    pub fn get_storage_type(&self) -> ObjectMapStorageType {
        self.storage.get_type()
    }

    // Load object map from storage
    pub async fn open(obj_data: serde_json::Value, read_only: bool) -> NdnResult<Self> {
        // First calc obj id with body
        let (obj_id, _) = build_named_object_by_json(OBJ_TYPE_OBJMAP, &obj_data);

        let body: ObjectMapBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {} {}", e, obj_id);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let storage = GLOBAL_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open(
                Some(&obj_id),
                read_only,
                Some(body.storage_type),
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
        let ret = storage.load_mtree_data().await.map_err(|e| {
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

            Some(mtree)
        } else {
            None
        };

        let mut map = Self {
            hash_method: body.hash_method,
            is_dirty: false,
            storage,
            mtree,
            obj_id: Some(obj_id),
        };

        if map.mtree.is_none() {
            map.is_dirty = true;
        }

        Ok(map)
    }

    // If mtree exists, return the current obj id, otherwise return None
    // If mtree is dirty, then should call flush_mtree/calc_obj_id to regenerate the mtree first
    pub fn get_obj_id(&self) -> Option<ObjId> {
        self.obj_id.clone()
    }

    // Should call flush_mtree first to regenerate the mtree if it is dirty
    // This will return None if the root hash is not set or the mtree is not
    pub fn calc_obj_id(&self) -> Option<(ObjId, String)> {
        if self.is_dirty {
            let msg = "Object map is dirty, should call flush_mtree at first".to_string();
            warn!("{}", msg);
            return None;
        }
        
        let root_hash = self.get_root_hash_str();
        if root_hash.is_none() {
            return None;
        }

        let root_hash = root_hash.unwrap();
        let body = ObjectMapBody {
            root_hash,
            hash_method: self.hash_method.clone(),
            storage_type: self.get_storage_type(),
        };

        let (obj_id, s) = build_named_object_by_json(
            OBJ_TYPE_OBJMAP,
            &serde_json::to_value(&body).expect("Failed to serialize ObjectMapBody"),
        );

        Some((obj_id, s))
    }

    pub fn hash_method(&self) -> HashMethod {
        self.hash_method
    }

    pub async fn len(&self) -> NdnResult<usize> {
        let stat = self.storage.stat().await.map_err(|e| {
            let msg = format!("Error getting object map stat: {}", e);
            error!("{}", msg);
            e
        })?;

        Ok(stat.total_count as usize)
    }

    pub async fn put_object(&mut self, key: &str, obj_id: &ObjId) -> NdnResult<()> {
        self.storage.put(&key, &obj_id).await.map_err(|e| {
            let msg = format!("Error putting object map item: {}", e);
            error!("{}", msg);
            e
        })?;

        self.is_dirty = true;

        Ok(())
    }

    pub async fn get_object(&self, key: &str) -> NdnResult<Option<ObjId>> {
        let ret = self.get_object_inner(key).await?;
        if ret.is_none() {
            return Ok(None);
        }

        Ok(Some(ret.unwrap().0))
    }

    pub async fn get_object_proof_path(
        &mut self,
        key: &str,
    ) -> NdnResult<Option<ObjectMapItemProof>> {
        if self.mtree.is_none() {
            let msg = "Merkle tree is not initialized".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        if self.is_dirty {
            let msg = "Object map is dirty, should call flush_mtree at first".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        // Get object and mtree index from storage
        let ret = self.get_object_inner(key).await?;
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
        let mtree = self.mtree.as_mut().unwrap();
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

    async fn get_object_inner(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>> {
        let ret = self.storage.get(&key).await.map_err(|e| {
            let msg = format!("Error getting object map item: {}, {}", key, e);
            error!("{}", msg);
            e
        })?;

        Ok(ret)
    }

    // Try to remove the object from the map, return the object id and meta data
    pub async fn remove_object(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        let ret = self.storage.remove(&key).await.map_err(|e| {
            let msg = format!("Error removing object map item: {}", e);
            error!("{}", msg);
            e
        })?;

        if ret.is_some() {
            self.is_dirty = true;
        }

        Ok(ret)
    }

    pub async fn is_object_exist(&self, key: &str) -> NdnResult<bool> {
        let ret = self.storage.is_exist(&key).await.map_err(|e| {
            let msg = format!("Error checking object map item: {}", e);
            error!("{}", msg);
            e
        })?;

        Ok(ret)
    }

    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    pub fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (String, ObjId, Option<u64>)> + 'a> {
        let iter = self.storage.iter();
        Box::new(iter)
    }

    // Regenerate the merkle tree without checking the dirty flag
    async fn regenerate_merkle_tree(&mut self) -> NdnResult<()> {
        let count = self.storage.stat().await?.total_count;
        let leaf_size = 256u64;
        let data_size = count as u64 * leaf_size;

        // TODO now use the memory buffer to store the merkle tree, need to optimize for further usage
        let buf_size = MerkleTreeObjectGenerator::estimate_output_bytes(
            data_size,
            leaf_size,
            Some(self.hash_method),
        );
        let buf = SharedBuffer::with_size(buf_size as usize);
        let stream = MtreeReadWriteSeekWithSharedBuffer::new(buf);
        let mtree_writer = Box::new(stream.clone()) as Box<dyn MtreeWriteSeek>;

        let mut mtree_generator = MerkleTreeObjectGenerator::new(
            data_size,
            leaf_size,
            Some(self.hash_method),
            mtree_writer,
        )
        .await?;

        let read_only = self.is_read_only();
        let mut page_index = 0;
        let page_size = 128;
        let mut leaf_index = 0;
        loop {
            let list = self.storage.list(page_index, page_size).await?;
            if list.is_empty() {
                break;
            }
            page_index += 1;

            for key in list {
                let item = self.get_object(&key).await?;
                if item.is_none() {
                    let msg = format!("Error getting object map item: {}", key);
                    error!("{}", msg);
                    return Err(NdnError::InvalidState(msg));
                }

                let item = item.unwrap();
                let hash = HashHelper::calc_hash_list(
                    self.hash_method,
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
                    self.storage
                        .update_mtree_index(&key, leaf_index)
                        .await
                        .map_err(|e| {
                            let msg = format!(
                                "Error updating mtree index: {}, {}, {}",
                                key, leaf_index, e
                            );
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

        // Save the mtree data to object for further usage
        self.mtree = Some(object);

        Ok(())
    }

    // Regenerate the merkle tree and object id if the mtree is dirty
    pub async fn flush_mtree(&mut self) -> NdnResult<()> {
        if !self.is_dirty && !self.mtree.is_none() {
            return Ok(());
        }

        self.regenerate_merkle_tree().await?;
        self.is_dirty = false;

        let (obj_id, _content) = self.calc_obj_id().unwrap();
        info!(
            "Calculated object map root hash and id: {}, {}",
            self.get_root_hash_str().unwrap(),
            obj_id
        );

        self.obj_id = Some(obj_id);

        Ok(())
    }

    // Get the storage file path for the object map
    // This will return None if the object ID is not generated yet
    // The target file must be created by the `save()` method
    pub fn get_storage_file_path(&self) -> Option<PathBuf> {
        let obj_id= self.get_obj_id();
        if obj_id.is_none() {
            return None; // Object ID is not set, cannot get file path
        }

        let obj_id = obj_id.unwrap();

        if self.get_storage_type() == ObjectMapStorageType::Memory {
            return None; // Memory storage does not have a file path
        }

        let factory = GLOBAL_OBJECT_MAP_STORAGE_FACTORY.get().unwrap();
        let file_path =
            factory.get_file_path_by_id(Some(&obj_id), self.get_storage_type());
        Some(file_path)
    }

    // Should not call this function if in read-only mode
    pub async fn save(&mut self) -> NdnResult<()> {
        if self.is_read_only() {
            let msg = "Object map is read-only".to_string();
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }

        self.flush_mtree().await?;

        let obj_id = self.get_obj_id();
        if obj_id.is_none() {
            let msg = "Object map obj id is empty".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        let obj_id = obj_id.unwrap();

        GLOBAL_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .save(&obj_id, &mut *self.storage)
            .await
            .map_err(|e| {
                let msg = format!("Error saving object map: {}", e);
                error!("{}", msg);
                e
            })?;

        info!("Saved object map to storage: {}", obj_id.to_base32());

        Ok(())
    }

    pub async fn clone(&self, read_only: bool) -> NdnResult<Self> {
        if self.is_dirty || self.mtree.is_none() {
            let msg = "Object map is dirty, should call flush_mtree at first".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        let obj_id = self.get_obj_id();
        if obj_id.is_none() {
            let msg = "Object map obj id is empty".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }
        let obj_id = obj_id.unwrap();

        let mut new_storage = GLOBAL_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .clone(&obj_id, &*self.storage, read_only)
            .await
            .map_err(|e| {
                let msg = format!("Error cloning object map storage: {}", e);
                error!("{}", msg);
                e
            })?;

        let mut ret = Self {
            hash_method: self.hash_method.clone(),
            is_dirty: true,
            storage: new_storage,
            mtree: None,
            obj_id: self.obj_id.clone(),
        };

        if read_only {
            // If read-only, we don't need to regenerate the mtree
            ret.is_dirty = false;
        }

        Ok(ret)
    }

    pub fn get_root_hash(&self) -> Option<Vec<u8>> {
        if self.mtree.is_none() {
            return None;
        }

        let mtree = self.mtree.as_ref().unwrap();
        let root_hash = mtree.get_root_hash();
        Some(root_hash)
    }

    pub fn get_root_hash_str(&self) -> Option<String> {
        self.get_root_hash()
            .map(|hash| Base32Codec::to_base32(&hash))
    }
}

pub struct ObjectMapProofVerifier {
    hash_method: HashMethod,
}

impl ObjectMapProofVerifier {
    pub fn new(hash_method: HashMethod) -> Self {
        Self { hash_method }
    }

    pub fn verify_with_obj_data_str(
        &self,
        obj_data: &str,
        proof: &ObjectMapItemProof,
    ) -> NdnResult<bool> {
        // Parse the object data as JSON
        let body: ObjectMapBody = serde_json::from_str(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let root_hash = body.root_hash;
        self.verify(&root_hash, proof)
    }

    pub fn verify_with_obj_data(
        &self,
        obj_data: serde_json::Value,
        proof: &ObjectMapItemProof,
    ) -> NdnResult<bool> {
        // Get the root hash from the object data
        let body: ObjectMapBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let root_hash = body.root_hash;
        self.verify(&root_hash, proof)
    }

    // root_hash is the object map's root hash, which is the body.root_hash field, encoded as base32
    // proof is the ObjectMapItemProof, which contains the item and the proof path
    pub fn verify(&self, root_hash: &str, proof: &ObjectMapItemProof) -> NdnResult<bool> {
        if proof.proof.len() < 2 {
            let msg = format!("Invalid proof path length: {}", proof.proof.len());
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        // First calculate the hash of the item
        let item_hash = proof.item.calc_hash(self.hash_method);

        // The first item is the leaf node, which is the item itself
        if proof.proof[0].1 != item_hash {
            let msg = format!(
                "Unmatched object map leaf hash: expected {:?}, got {:?}",
                item_hash, proof.proof[0].1
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        let root_hash = Base32Codec::from_base32(root_hash).map_err(|e| {
            let msg = format!("Error decoding root hash: {}, {}", root_hash, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        // The last item is the root node, which is the expected root hash
        if proof.proof[proof.proof.len() - 1].1 != root_hash {
            let msg = format!(
                "Unmatched object map root hash: expected {:?}, got {:?}",
                root_hash,
                proof.proof[proof.proof.len() - 1].1
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        let mtree_verifier = MerkleTreeProofPathVerifier::new(self.hash_method);
        mtree_verifier.verify(&proof.proof)
    }
}
