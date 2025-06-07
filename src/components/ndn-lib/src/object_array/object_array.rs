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
use crate::{HashMethod, ObjId, OBJ_TYPE_LIST};
use crate::{NdnError, NdnResult};
use arrow::csv::writer;
use core::hash;
use http_types::cache;
use serde::{Deserialize, Serialize};
use std::io::SeekFrom;
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
pub struct ObjectArrayMeta {
    pub hash_method: HashMethod,
    pub ext: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ObjectArrayItem {
    pub obj_id: ObjId,
    pub proof: ObjectArrayItemProof,
}

pub struct ObjectArray {
    meta: ObjectArrayMeta,
    storage_type: ObjectArrayStorageType,
    cache: Box<dyn ObjectArrayInnerCache>,
    obj_id: Option<ObjId>, // The object ID of the array, can be None if not calculated yet
    is_dirty: bool,
    mtree: Option<MerkleTreeObject>,
}

impl ObjectArray {
    pub fn new(hash_method: HashMethod, storage_type: Option<ObjectArrayStorageType>) -> Self {
        let cache: Box<dyn ObjectArrayInnerCache> =
            ObjectArrayCacheFactory::create_cache(ObjectArrayCacheType::Memory);

        Self {
            meta: ObjectArrayMeta {
                hash_method: hash_method.clone(),
                ext: None, // We can add more metadata in the future
            },
            storage_type: storage_type.unwrap_or(ObjectArrayStorageType::default()),
            cache,
            obj_id: None, // The object ID is not calculated ye
            is_dirty: false,
            mtree: None,
        }
    }

    pub fn new_from_cache(
        meta: ObjectArrayMeta,
        cache: Box<dyn ObjectArrayInnerCache>,
        storage_type: ObjectArrayStorageType,
    ) -> NdnResult<Self> {
        let obj_array = Self {
            meta,
            storage_type,
            cache,
            obj_id: None, // The object ID is not calculated yet
            is_dirty: false,
            mtree: None,
        };

        Ok(obj_array)
    }

    pub fn is_readonly(&self) -> bool {
        self.cache.is_readonly()
    }

    pub fn hash_method(&self) -> HashMethod {
        self.meta.hash_method
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
            meta: self.meta.clone(),
            storage_type: self.storage_type.clone(),
            cache,
            obj_id: self.obj_id.clone(),
            is_dirty: self.is_dirty,
            mtree: None, // FIXME: Should we clone the mtree result if exists?
        };

        Ok(ret)
    }

    pub fn set_meta(&mut self, meta: Option<String>) -> NdnResult<()> {
        self.meta.ext = meta;

        Ok(())
    }

    pub fn get_meta(&self) -> NdnResult<Option<&str>> {
        Ok(self.meta.ext.as_deref())
    }

    pub fn append_object(&mut self, obj_id: &ObjId) -> NdnResult<()> {
        // Check if obj_id.obj_hash is valid
        get_obj_hash(obj_id, self.meta.hash_method)?;

        self.cache.append(obj_id)?;
        self.is_dirty = true;

        Ok(())
    }

    pub fn insert_object(&mut self, index: usize, obj_id: &ObjId) -> NdnResult<()> {
        // Check if obj_id.obj_hash is valid
        get_obj_hash(obj_id, self.meta.hash_method)?;

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
        }

        Ok(ret)
    }

    // Get the object ID for the array if mtree is not None, otherwise return None
    // WARNING: This method don't check if the mtree is dirty
    pub fn get_obj_id(&self) -> Option<ObjId> {
        self.obj_id.clone()
    }

    // Calculate the object ID for the array
    // This is the same as the mtree root hash, but we need to check if the mtree is dirty
    pub async fn calc_obj_id(&mut self) -> NdnResult<ObjId> {
        if self.cache.len() == 0 {
            let msg = "No objects in the array".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        // Check if the mtree is dirty
        if self.is_dirty || self.mtree.is_none() {
            // If the mtree is dirty or first loaded, we need to regenerate it
            self.flush_impl().await?;
        }

        Ok(self.get_obj_id().unwrap())
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

    pub async fn flush(&mut self) -> NdnResult<()> {
        if !self.is_dirty && self.mtree.is_some() {
            return Ok(());
        }

        self.flush_impl().await
    }

    async fn flush_impl(&mut self) -> NdnResult<()> {
        self.regenerate_merkle_tree().await?;
        self.is_dirty = false;

        let root_hash = self.mtree.as_ref().unwrap().get_root_hash();
        let obj_id = ObjId::new_by_raw(OBJ_TYPE_LIST.to_string(), root_hash);
        self.obj_id = Some(obj_id);

        Ok(())
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

        // Write the meta to the storage
        let meta = serde_json::to_string(&self.meta).map_err(|e| {
            let msg = format!("Error serializing meta: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        writer.put_meta(Some(meta)).await?;

        // Write the object array to the storage
        // TODO: use batch read and write to improve performance
        for i in 0..self.cache.len() {
            let obj_id = self.cache.get(i)?.unwrap();
            writer.append(&obj_id).await?;
        }

        writer.flush().await?;

        Ok(())
    }

    pub async fn open(container_id: &ObjId, read_only: bool) -> NdnResult<Self> {
        let factory = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().unwrap();
        let (cache, storage_type) = factory.open(container_id, read_only).await?;

        // First load meta from the cache and deserialize it, so we can get the hash method
        let meta = cache.get_meta()?;
        let meta: ObjectArrayMeta = match meta {
            Some(meta) => serde_json::from_slice(meta.as_bytes()).map_err(|e| {
                let msg = format!("Error deserializing meta: {}", e);
                error!("{}", msg);
                NdnError::InvalidData(msg)
            })?,
            None => {
                let msg = format!("Object array meta not found for: {:?}", container_id);
                error!("{}", msg);
                return Err(NdnError::InvalidData(msg));
            }
        };

        let obj_array = Self::new_from_cache(meta, cache, storage_type)?;

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

    pub fn verify(
        &self,
        container_id: &ObjId,
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

        // The last item is the root node, which is obj_id.obj_hash field
        if proof.proof[proof.proof.len() - 1].1 != container_id.obj_hash {
            let msg = format!(
                "Unmatched object array root hash: expected {:?}, got {:?}",
                container_id.obj_hash,
                proof.proof[proof.proof.len() - 1].1
            );
            warn!("{}", msg);
            return Ok(false);
        }

        let mtree_verifier = MerkleTreeProofPathVerifier::new(self.hash_method);
        mtree_verifier.verify(&proof.proof)
    }
}
