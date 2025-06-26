use super::iter::{ObjectArrayIter, ObjectArrayOwnedIter};
use super::proof::ObjectArrayItemProof;
use super::storage::{
    ObjectArrayCacheType, ObjectArrayInnerCache, ObjectArrayStorageType, ObjectArrayStorageWriter,
};
use super::storage_factory::{ObjectArrayCacheFactory, ObjectArrayStorageFactory};
use super::GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY;
use crate::mtree::MerkleTreeProofPathVerifier;
use crate::mtree::{
    self, MerkleTreeObject, MerkleTreeObjectGenerator, MtreeReadSeek,
    MtreeReadWriteSeekWithSharedBuffer, MtreeWriteSeek, SharedBuffer,
};
use crate::{build_named_object_by_json, Base32Codec, HashMethod, ObjId, OBJ_TYPE_LIST};
use crate::{NdnError, NdnResult};
use core::hash;
use http_types::cache;
use serde::{Deserialize, Serialize};
use std::hash::Hash;
use std::io::SeekFrom;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

// Because the object may be ObjectId or ChunkId, which maybe have mix mode, so we need to check the hash length
fn get_obj_hash<'a>(obj_id: &'a ObjId, hash_method: HashMethod) -> NdnResult<&'a [u8]> {
    if obj_id.obj_hash.len() < hash_method.hash_bytes() {
        let msg = format!(
            "Object hash length does not match hash method: {}",
            obj_id.obj_hash.len()
        );
        error!("{}", msg);
        return Err(NdnError::InvalidData(msg));
    }

    // We use the last hash bytes as the object hash
    if obj_id.obj_hash.len() > hash_method.hash_bytes() {
        // obj_id is a chunk id with mix mode, we need to get the last hash bytes
        // FIXME: Should we check if the hash type is valid?
        let start = obj_id.obj_hash.len() - hash_method.hash_bytes();
        return Ok(&obj_id.obj_hash[start..]);
    } else {
        // If the hash length is equal, we can return the whole hash
        return Ok(&obj_id.obj_hash);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectArrayBody {
    pub root_hash: String, // The root hash of the merkle tree, encoded as base32 string
    pub hash_method: HashMethod,
}

#[derive(Clone, Debug)]
pub struct ObjectArrayItem {
    pub obj_id: ObjId,
    pub proof: ObjectArrayItemProof,
}

pub struct ObjectArray {
    hash_method: HashMethod,
    storage_type: ObjectArrayStorageType,
    cache: Box<dyn ObjectArrayInnerCache>,
    is_dirty: bool,
    mtree: Option<MerkleTreeObject>,
    obj_id: Option<ObjId>, // The object ID of the array, can be None if not calculated yet
}

impl ObjectArray {
    pub fn new(hash_method: HashMethod, storage_type: Option<ObjectArrayStorageType>) -> Self {
        let cache: Box<dyn ObjectArrayInnerCache> =
            ObjectArrayCacheFactory::create_cache(ObjectArrayCacheType::Memory);

        Self {
            hash_method,
            storage_type: storage_type.unwrap_or(ObjectArrayStorageType::default()),
            cache,
            obj_id: None, // The object ID is not calculated ye
            is_dirty: false,
            mtree: None,
        }
    }

    fn new_from_cache(
        obj_id: ObjId,
        hash_method: HashMethod,
        cache: Box<dyn ObjectArrayInnerCache>,
        storage_type: ObjectArrayStorageType,
    ) -> NdnResult<Self> {
        let obj_array = Self {
            hash_method,
            storage_type,
            cache,
            obj_id: Some(obj_id),
            is_dirty: false,
            mtree: None,
        };

        Ok(obj_array)
    }

    pub fn is_readonly(&self) -> bool {
        self.cache.is_readonly()
    }

    pub fn hash_method(&self) -> HashMethod {
        self.hash_method
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn storage_type(&self) -> ObjectArrayStorageType {
        self.storage_type
    }

    pub fn iter(&self) -> ObjectArrayIter<'_> {
        ObjectArrayIter::new(&*self.cache)
    }

    pub fn clone(&self, read_only: bool) -> NdnResult<Self> {
        let cache = self.cache.clone_cache(read_only)?;
        let ret = Self {
            hash_method: self.hash_method.clone(),
            storage_type: self.storage_type.clone(),
            cache,
            obj_id: self.obj_id.clone(),
            is_dirty: self.is_dirty,
            mtree: None, // FIXME: Should we clone the mtree result if exists?
        };

        Ok(ret)
    }

    pub fn append_object(&mut self, obj_id: &ObjId) -> NdnResult<()> {
        // Check if obj_id.obj_hash is valid
        get_obj_hash(obj_id, self.hash_method)?;

        self.cache.append(obj_id)?;
        self.is_dirty = true;

        Ok(())
    }

    pub fn insert_object(&mut self, index: usize, obj_id: &ObjId) -> NdnResult<()> {
        // Check if obj_id.obj_hash is valid
        get_obj_hash(obj_id, self.hash_method)?;

        self.cache.insert(index, obj_id)?;
        self.is_dirty = true;

        Ok(())
    }

    pub fn remove_object(&mut self, index: usize) -> NdnResult<Option<ObjId>> {
        let ret = self.cache.remove(index)?;

        if ret.is_some() {
            self.is_dirty = true;
        }

        Ok(ret)
    }

    pub fn pop_object(&mut self) -> NdnResult<Option<ObjId>> {
        let ret = self.cache.pop()?;

        if ret.is_some() {
            self.is_dirty = true;
        }

        Ok(ret)
    }

    pub fn clear(&mut self) -> NdnResult<()> {
        if self.cache.len() == 0 {
            return Ok(());
        }

        self.cache.clear()?;
        self.is_dirty = true;

        Ok(())
    }

    pub fn get_object(&self, index: usize) -> NdnResult<Option<ObjId>> {
        self.cache.get(index)
    }

    // Change the storage type for the object array, this will not change the cache
    // When the storage type is changed, you should call `save()` to persist the changes
    // to the new storage type.
    pub async fn change_storage_type(
        &mut self,
        storage_type: ObjectArrayStorageType,
    ) -> NdnResult<()> {
        if self.storage_type == storage_type {
            return Ok(());
        }

        info!(
            "Changing storage type from {:?} to {:?}, {:?}",
            self.storage_type,
            storage_type,
            self.get_obj_id(),
        );

        // Change the storage type
        self.storage_type = storage_type;

        Ok(())
    }

    // Get the object ID and proof for the object at the given index, the mtree must be exists
    pub async fn get_object_with_proof(
        &mut self,
        index: usize,
    ) -> NdnResult<Option<ObjectArrayItem>> {
        if self.mtree.is_none() {
            let msg = "Mtree is not initialized".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        assert!(self.mtree.is_some(), "Mtree is not initialized");

        let obj_id = self.cache.get(index)?;
        if obj_id.is_none() {
            return Ok(None);
        }
        let obj_id = obj_id.unwrap();

        let mtree_proof = self
            .mtree
            .as_mut()
            .unwrap()
            .get_proof_path_by_leaf_index(index as u64)
            .await?;
        let proof = ObjectArrayItemProof { proof: mtree_proof };

        Ok(Some(ObjectArrayItem { obj_id, proof }))
    }

    pub async fn batch_get_object_with_proof(
        &mut self,
        indices: &[usize],
    ) -> NdnResult<Vec<Option<ObjectArrayItem>>> {
        if self.mtree.is_none() {
            let msg = "Mtree is not initialized".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        let mut ret = Vec::with_capacity(indices.len());
        for index in indices {
            let obj_id = self.cache.get(*index)?;
            if obj_id.is_none() {
                ret.push(None);
                continue;
            }
            let obj_id = obj_id.unwrap();

            let mtree_proof = self
                .mtree
                .as_mut()
                .unwrap()
                .get_proof_path_by_leaf_index(*index as u64)
                .await?;
            let proof = ObjectArrayItemProof { proof: mtree_proof };

            ret.push(Some(ObjectArrayItem { obj_id, proof }));
        }

        Ok(ret)
    }

    pub async fn range_get_object_with_proof(
        &mut self,
        start: usize,
        end: usize,
    ) -> NdnResult<Vec<Option<ObjectArrayItem>>> {
        if self.mtree.is_none() {
            let msg = "Mtree is not initialized".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        let obj_list = self.cache.get_range(start, end)?;
        if obj_list.is_empty() {
            return Ok(vec![]);
        }

        let mut ret = Vec::with_capacity(end - start);
        let mtree = self.mtree.as_mut().unwrap();
        let mut i = 0;
        for index in start..end {
            if i >= obj_list.len() {
                ret.push(None);
                continue;
            }

            let mtree_proof = mtree.get_proof_path_by_leaf_index(index as u64).await?;
            let proof: ObjectArrayItemProof = ObjectArrayItemProof { proof: mtree_proof };

            let obj_id = obj_list[i].clone();
            ret.push(Some(ObjectArrayItem { obj_id, proof }));
            i += 1;
        }

        Ok(ret)
    }

    pub fn get_body(&self) -> Option<ObjectArrayBody> {
        let root_hash = self.get_root_hash_str();
        if root_hash.is_none() {
            return None;
        }

        Some(ObjectArrayBody {
            root_hash: root_hash.unwrap(),
            hash_method: self.hash_method.clone(),
        })
    }

    // Get the object ID for the array if mtree is not None, otherwise return None
    // WARNING: This method don't check if the mtree is dirty
    pub fn get_obj_id(&self) -> Option<ObjId> {
        self.obj_id.clone()
    }

    // Calculate the object ID for the array
    pub fn calc_obj_id(&self) -> Option<(ObjId, String)> {
        if self.is_dirty {
            let msg = "Object map is dirty, should call flush_mtree at first".to_string();
            warn!("{}", msg);
            return None;
        }
        
        let body = self.get_body();
        if body.is_none() {
            return None;
        }

        let body = body.unwrap();
        let (obj_id, s) = build_named_object_by_json(
            OBJ_TYPE_LIST,
            &serde_json::to_value(&body).expect("Failed to serialize ObjectMapBody"),
        );

        Some((obj_id, s))
    }

    pub fn get_root_hash(&self) -> Option<Vec<u8>> {
        if self.mtree.is_none() {
            return None;
        }

        let root_hash = self.mtree.as_ref().unwrap().get_root_hash();
        Some(root_hash)
    }

    pub fn get_root_hash_str(&self) -> Option<String> {
        self.get_root_hash()
            .map(|hash| Base32Codec::to_base32(&hash))
    }

    // Regenerate the merkle tree without checking the dirty flag
    async fn regenerate_merkle_tree(&mut self) -> NdnResult<()> {
        let count = self.cache.len() as u64;
        let leaf_size = self.hash_method().hash_bytes() as u64;
        let data_size = count as u64 * leaf_size;

        // TODO now use the memory buffer to store the merkle tree, need to optimize for further usage
        let buf_size = MerkleTreeObjectGenerator::estimate_output_bytes(
            data_size,
            leaf_size,
            Some(self.hash_method()),
        );
        let buf = SharedBuffer::with_size(buf_size as usize);
        let stream = MtreeReadWriteSeekWithSharedBuffer::new(buf);
        let mtree_writer = Box::new(stream.clone()) as Box<dyn MtreeWriteSeek>;

        let mut mtree_generator = MerkleTreeObjectGenerator::new(
            data_size,
            leaf_size,
            Some(self.hash_method()),
            mtree_writer,
        )
        .await?;

        for i in 0..self.cache.len() {
            let obj_id = self.cache.get(i)?.unwrap();

            mtree_generator
                .append_leaf_hashes(&vec![get_obj_hash(&obj_id, self.hash_method())?.to_vec()])
                .await
                .map_err(|e| {
                    let msg = format!("Error appending leaf hashes: {}", e);
                    error!("{}", msg);
                    e
                })?;
        }

        // Finalize the merkle tree and get the root hash
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

    pub async fn flush_mtree(&mut self) -> NdnResult<()> {
        if !self.is_dirty && self.mtree.is_some() {
            return Ok(());
        }

        self.flush_mtree_impl().await
    }

    async fn flush_mtree_impl(&mut self) -> NdnResult<()> {
        self.regenerate_merkle_tree().await?;
        self.is_dirty = false;

        let (obj_id, _) = self.calc_obj_id().ok_or_else(|| {
            let msg = "Failed to calculate object ID".to_string();
            error!("{}", msg);
            NdnError::InvalidState(msg)
        })?;

        self.obj_id = Some(obj_id);

        Ok(())
    }

    // Get the storage file path for the object array
    // This will return None if the object ID is not generated yet
    // The target file must be created by the `save()` method
    pub fn get_storage_file_path(&self) -> Option<PathBuf> {
        let factory = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().unwrap();
        let id = self.get_obj_id();
        if id.is_none() {
            return None;
        }
        let id = id.unwrap();

        let file_path = factory.get_file_path(&id, self.storage_type.clone());
        Some(file_path)
    }

    pub async fn save(&self) -> NdnResult<()> {
        let factory = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().unwrap();
        let mut writer = factory
            .open_writer(
                &self.get_obj_id().unwrap(),
                None,
                Some(self.storage_type.clone()),
            )
            .await?;

        // Write the object array to the storage
        // TODO: use batch read and write to improve performance
        for i in 0..self.cache.len() {
            let obj_id = self.cache.get(i)?.unwrap();
            writer.append(&obj_id).await?;
        }

        writer.flush().await?;

        Ok(())
    }

    pub async fn open(obj_data: serde_json::Value, read_only: bool) -> NdnResult<Self> {
        // First calc obj id with body
        let (obj_id, _) = build_named_object_by_json(OBJ_TYPE_LIST, &obj_data);

        let body: ObjectArrayBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object array body: {} {}", e, obj_id);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let factory = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().unwrap();
        let (cache, storage_type) = factory.open(&obj_id, read_only).await?;

        let obj_array = Self::new_from_cache(obj_id, body.hash_method, cache, storage_type)?;

        Ok(obj_array)
    }
}

impl IntoIterator for ObjectArray {
    type Item = ObjId;
    type IntoIter = ObjectArrayOwnedIter;

    fn into_iter(self) -> Self::IntoIter {
        ObjectArrayOwnedIter::new(self.cache)
    }
}

pub struct ObjectArrayProofVerifier {
    hash_method: HashMethod,
}

impl ObjectArrayProofVerifier {
    pub fn new(hash_method: HashMethod) -> Self {
        Self { hash_method }
    }

    pub fn verify_with_obj_data_str(
        &self,
        obj_data: &str,
        obj_id: &ObjId,
        proof: &ObjectArrayItemProof,
    ) -> NdnResult<bool> {
        // Parse the object data as JSON
        let body: ObjectArrayBody = serde_json::from_str(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let root_hash = body.root_hash;
        self.verify(&root_hash, obj_id, proof)
    }

    pub fn verify_with_obj_data(
        &self,
        obj_data: serde_json::Value,
        obj_id: &ObjId,
        proof: &ObjectArrayItemProof,
    ) -> NdnResult<bool> {
        // Get the root hash from the object data
        let body: ObjectArrayBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object array body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let root_hash = body.root_hash;
        self.verify(&root_hash, obj_id, proof)
    }

    pub fn verify(
        &self,
        root_hash: &str,
        obj_id: &ObjId,
        proof: &ObjectArrayItemProof,
    ) -> NdnResult<bool> {
        if proof.proof.len() < 2 {
            let msg = format!("Invalid proof path length: {}", proof.proof.len());
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        // The first item is the leaf node, which is the item itself
        if proof.proof[0].1 != get_obj_hash(obj_id, self.hash_method)? {
            let msg = format!(
                "Unmatched object array leaf hash: expected {:?}, got {:?}",
                obj_id, proof.proof[0].1
            );
            warn!("{}", msg);
            return Ok(false);
        }

        let root_hash = Base32Codec::from_base32(root_hash).map_err(|e| {
            let msg = format!("Error decoding root hash: {}, {}", root_hash, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        // The last item is the root node, which is obj_id.obj_hash field
        if proof.proof[proof.proof.len() - 1].1 != root_hash {
            let msg = format!(
                "Unmatched object array root hash: expected {:?}, got {:?}",
                root_hash,
                proof.proof[proof.proof.len() - 1].1
            );
            warn!("{}", msg);
            return Ok(false);
        }

        let mtree_verifier = MerkleTreeProofPathVerifier::new(self.hash_method);
        mtree_verifier.verify(&proof.proof)
    }
}
