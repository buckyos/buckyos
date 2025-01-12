use super::storage::InnerStorage;
use crate::mtree::{MerkleTreeObject, MerkleTreeObjectGenerator};
use crate::mtree::{
    MtreeReadSeek, MtreeReadWriteSeekWithSharedBuffer, MtreeWriteSeek, SharedBuffer,
};
use crate::object::ObjId;
use crate::{
    hash::{HashHelper, HashMethod},
    NdnError, NdnResult,
};
use crate::{MerkleTreeProofPathVerifier, OBJ_TYPE_OBJMAPT};
use core::hash;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ObjectMapItem {
    pub key: String,
    pub obj_id: ObjId,
    pub meta: Option<Vec<u8>>,
}

impl ObjectMapItem {
    pub fn new(key: impl Into<String>, obj_id: ObjId, meta: Option<Vec<u8>>) -> Self {
        Self {
            key: key.into(),
            obj_id,
            meta,
        }
    }

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
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ObjectMapMeta {
    // Default is Sha256
    pub hash_method: HashMethod,
}

#[derive(Debug, Clone)]
pub struct ObjectMapItemProof {
    pub item: ObjectMapItem,
    pub proof: Vec<(u64, Vec<u8>)>,
}

pub struct ObjectMap {
    pub meta: ObjectMapMeta,
    pub is_dirty: bool,
    pub storage: Box<dyn InnerStorage>,
    pub mtree: Option<MerkleTreeObject>,
}

impl ObjectMap {
    // Create empty object map
    pub async fn new(
        hash_method: HashMethod,
        mut storage: Box<dyn InnerStorage>,
    ) -> NdnResult<Self> {
        let meta = ObjectMapMeta { hash_method };

        // First save the meta to storage
        let data = bincode::serialize(&meta).unwrap();
        storage.put_meta(&data).await.map_err(|e| {
            let msg = format!("Error putting object map meta: {}", e);
            error!("{}", msg);
            e
        })?;

        Ok(Self {
            meta,
            is_dirty: false,
            storage,
            mtree: None,
        })
    }

    // Load object map from storage
    pub async fn load(storage: Box<dyn InnerStorage>) -> NdnResult<Self> {
        // First load meta from storage
        let ret = storage.get_meta().await.map_err(|e| {
            let msg = format!("Error getting object map meta: {}", e);
            error!("{}", msg);
            e
        })?;

        if ret.is_none() {
            let msg = "Object map meta is not found".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        let meta: ObjectMapMeta = bincode::deserialize(&ret.unwrap()).map_err(|e| {
            let msg = format!("Error decoding object map meta: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
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
            meta,
            is_dirty: false,
            storage,
            mtree,
        };

        if map.mtree.is_none() {
            map.is_dirty = true;
        }

        Ok(map)
    }

    // If mtree exists, return the current objid, otherwise return None
    // If mtree is dirty, then should call flush to regenerate the mtree first
    pub fn gen_obj_id(&self) -> Option<ObjId> {
        let root_hash = self.get_root_hash();
        if root_hash.is_none() {
            None
        } else {
            Some(ObjId::new_by_raw(
                OBJ_TYPE_OBJMAPT.to_owned(),
                root_hash.unwrap(),
            ))
        }
    }

    pub fn hash_method(&self) -> HashMethod {
        self.meta.hash_method
    }

    pub async fn put_object(
        &mut self,
        key: &str,
        obj_id: ObjId,
        meta: Option<Vec<u8>>,
    ) -> NdnResult<()> {
        let item = ObjectMapItem::new(key.to_owned(), obj_id, meta);
        let data = item.encode()?;

        self.storage.put(&key, &data).await.map_err(|e| {
            let msg = format!("Error putting object map item: {}", e);
            error!("{}", msg);
            e
        })?;

        self.is_dirty = true;

        Ok(())
    }

    pub async fn get_object(&self, key: &str) -> NdnResult<Option<ObjectMapItem>> {
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
            let msg = "Object map is dirty, should call flush at first".to_string();
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
            item: self.get_object(key).await?.unwrap(),
            proof,
        };

        Ok(Some(proof))
    }

    async fn get_object_inner(&self, key: &str) -> NdnResult<Option<(ObjectMapItem, Option<u64>)>> {
        let ret = self.storage.get(&key).await.map_err(|e| {
            let msg = format!("Error getting object map item: {}, {}", key, e);
            error!("{}", msg);
            e
        })?;

        if ret.is_none() {
            return Ok(None);
        }

        let (value, index) = ret.unwrap();
        let item = ObjectMapItem::decode(&value).map_err(|e| {
            let msg = format!("Error decoding object map item: {}, {}", key, e);
            error!("{}", msg);
            e
        })?;

        Ok(Some((item, index)))
    }

    // Try to remove the object from the map, return the object id and meta data
    pub async fn remove_object(
        &mut self,
        key: &str,
    ) -> NdnResult<Option<(ObjId, Option<Vec<u8>>)>> {
        let ret = self.storage.remove(&key).await.map_err(|e| {
            let msg = format!("Error removing object map item: {}", e);
            error!("{}", msg);
            e
        })?;

        if ret.is_none() {
            return Ok(None);
        }

        self.is_dirty = true;

        let item = ObjectMapItem::decode(&ret.unwrap()).map_err(|e| {
            let msg = format!("Error decoding object map item: {}, {}", key, e);
            error!("{}", msg);
            e
        })?;

        Ok(Some((item.obj_id, item.meta)))
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

    // Regenerate the merkle tree without checking the dirty flag
    pub async fn regenerate_merkle_tree(&mut self) -> NdnResult<()> {
        let count = self.storage.stat().await?.total_count;
        let leaf_size = 256u64;
        let data_size = count as u64 * leaf_size;

        // TODO now use the memory buffer to store the merkle tree, need to optimize for further usage
        let buf_size = MerkleTreeObjectGenerator::estimate_output_bytes(
            data_size,
            leaf_size,
            Some(self.meta.hash_method),
        );
        let buf = SharedBuffer::with_size(buf_size as usize);
        let stream = MtreeReadWriteSeekWithSharedBuffer::new(buf);
        let mtree_writer = Box::new(stream.clone()) as Box<dyn MtreeWriteSeek>;

        let mut mtree_generator = MerkleTreeObjectGenerator::new(
            data_size,
            leaf_size,
            Some(self.meta.hash_method),
            mtree_writer,
        )
        .await?;

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
                let hash = HashHelper::calc_hash(self.meta.hash_method, &item.encode().unwrap());
                mtree_generator
                    .append_leaf_hashes(&vec![hash])
                    .await
                    .map_err(|e| {
                        let msg = format!("Error appending leaf hashes: {}, {}", key, e);
                        error!("{}", msg);
                        e
                    })?;
                
                // Update the mtree index in storage
                self.storage
                    .update_mtree_index(&key, leaf_index)
                    .await
                    .map_err(|e| {
                        let msg = format!("Error updating mtree index: {}, {}, {}", key, leaf_index, e);
                        error!("{}", msg);
                        e
                    })?;

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

    pub async fn flush(&mut self) -> NdnResult<()> {
        if !self.is_dirty && self.mtree.is_some() {
            return Ok(());
        }

        self.regenerate_merkle_tree().await?;

        self.is_dirty = false;

        Ok(())
    }

    pub fn get_root_hash(&self) -> Option<Vec<u8>> {
        if self.mtree.is_none() {
            return None;
        }

        let mtree = self.mtree.as_ref().unwrap();
        let root_hash = mtree.get_root_hash();
        Some(root_hash)
    }
}

pub struct ObjectMapProofVerifier {
    hash_method: HashMethod,
}

impl ObjectMapProofVerifier {
    pub fn new(hash_method: HashMethod) -> Self {
        Self { hash_method }
    }

    pub fn verify(&self, object_map: &ObjId, proof: &ObjectMapItemProof) -> NdnResult<bool> {
        if proof.proof.len() < 2 {
            let msg = format!("Invalid proof path length: {}", proof.proof.len());
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        // First calculate the hash of the item
        let item_data = proof.item.encode().unwrap();
        let item_hash = HashHelper::calc_hash(self.hash_method, &item_data);

        // The first item is the leaf node, which is the item itself
        if proof.proof[0].1 != item_hash {
            let msg = format!(
                "Unmatched objectmap leaf hash: expected {:?}, got {:?}",
                item_hash, proof.proof[0].1
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        // The last item is the root node, which is objid.hash field
        if proof.proof[proof.proof.len() - 1].1 != object_map.obj_hash {
            let msg = format!(
                "Unmatched objectmap root hash: expected {:?}, got {:?}",
                object_map.obj_hash,
                proof.proof[proof.proof.len() - 1].1
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        let mtree_verifier = MerkleTreeProofPathVerifier::new(self.hash_method);
        mtree_verifier.verify(&proof.proof)
    }
}
