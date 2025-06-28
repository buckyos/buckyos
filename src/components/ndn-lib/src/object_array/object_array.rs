use super::iter::{ObjectArrayIter, ObjectArrayOwnedIter};
use super::proof::ObjectArrayItemProof;
use super::storage::{
    ObjectArrayCacheType, ObjectArrayInnerCache, ObjectArrayStorageType, ObjectArrayStorageWriter,
};
use super::storage_factory::{ObjectArrayCacheFactory, ObjectArrayStorageFactory};
use super::GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY;
use crate::mtree::{
    self, MerkleTreeObject, MerkleTreeObjectGenerator, MtreeReadSeek,
    MtreeReadWriteSeekWithSharedBuffer, MtreeWriteSeek, SharedBuffer,
};
use crate::{
    build_named_object_by_json, Base32Codec, CollectionStorageMode, HashMethod, ObjId,
    OBJ_TYPE_LIST, OBJ_TYPE_LIST_SIMPLE,
};
use crate::{get_obj_hash, NdnError, NdnResult};
use core::hash;
use http_types::cache;
use serde::{Deserialize, Serialize};
use std::hash::Hash;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectArrayBody {
    pub root_hash: String, // The root hash of the merkle tree, encoded as base32 string
    pub hash_method: HashMethod,
    pub total_count: u64, // The number of objects in the array
}

impl ObjectArrayBody {
    pub fn calc_obj_id(&self) -> (ObjId, String) {
        let obj_type = match CollectionStorageMode::select_mode(Some(self.total_count)) {
            CollectionStorageMode::Simple => OBJ_TYPE_LIST_SIMPLE,
            CollectionStorageMode::Normal => OBJ_TYPE_LIST,
        };

        build_named_object_by_json(
            obj_type,
            &serde_json::to_value(self).expect("Failed to serialize ObjectArrayBody"),
        )
    }

    pub fn get_storage_type(&self) -> ObjectArrayStorageType {
        match CollectionStorageMode::select_mode(Some(self.total_count)) {
            CollectionStorageMode::Simple => ObjectArrayStorageType::JSONFile,
            CollectionStorageMode::Normal => ObjectArrayStorageType::Arrow,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ObjectArrayItem {
    pub obj_id: ObjId,
    pub proof: ObjectArrayItemProof,
}

pub struct ObjectArray {
    obj_id: ObjId,
    body: ObjectArrayBody,
    cache: Box<dyn ObjectArrayInnerCache>,
    mtree: Arc<Mutex<MerkleTreeObject>>,
}

impl ObjectArray {
    pub fn new(
        obj_id: ObjId,
        body: ObjectArrayBody,
        cache: Box<dyn ObjectArrayInnerCache>,
        mtree: MerkleTreeObject,
    ) -> Self {
        assert_eq!(
            body.total_count,
            cache.len() as u64,
            "ObjectArrayBody total_count must match cache length"
        );

        Self {
            obj_id,
            body,
            cache,
            mtree: Arc::new(Mutex::new(mtree)),
        }
    }

    pub fn cache(&self) -> &Box<dyn ObjectArrayInnerCache> {
        &self.cache
    }

    pub fn into_cache(self) -> Box<dyn ObjectArrayInnerCache> {
        self.cache
    }

    pub fn hash_method(&self) -> HashMethod {
        self.body.hash_method
    }

    pub fn len(&self) -> usize {
        self.body.total_count as usize
    }

    pub fn storage_type(&self) -> ObjectArrayStorageType {
        self.body.get_storage_type()
    }

    pub fn iter(&self) -> ObjectArrayIter<'_> {
        ObjectArrayIter::new(&*self.cache)
    }

    pub fn clone(&self) -> NdnResult<Self> {
        let cache = self.cache.clone_cache(true)?;
        let ret = Self {
            obj_id: self.obj_id.clone(),
            body: self.body.clone(),
            cache,
            mtree: self.mtree.clone(),
        };

        Ok(ret)
    }

    pub(crate) fn clone_for_modify(&self) -> NdnResult<Self> {
        let cache = self.cache.clone_cache(false)?;
        let ret = Self {
            obj_id: self.obj_id.clone(),
            body: self.body.clone(),
            cache,
            mtree: self.mtree.clone(),
        };

        Ok(ret)
    }

    pub fn get_object(&self, index: usize) -> NdnResult<Option<ObjId>> {
        self.cache.get(index)
    }

    // Get the object ID and proof for the object at the given index, the mtree must be exists
    pub async fn get_object_with_proof(
        &mut self,
        index: usize,
    ) -> NdnResult<Option<ObjectArrayItem>> {
        let obj_id = self.cache.get(index)?;
        if obj_id.is_none() {
            return Ok(None);
        }
        let obj_id = obj_id.unwrap();

        let mut mtree = self.mtree.lock().await;
        let mtree_proof = mtree.get_proof_path_by_leaf_index(index as u64).await?;
        let proof = ObjectArrayItemProof { proof: mtree_proof };

        Ok(Some(ObjectArrayItem { obj_id, proof }))
    }

    pub async fn batch_get_object_with_proof(
        &mut self,
        indices: &[usize],
    ) -> NdnResult<Vec<Option<ObjectArrayItem>>> {
        let mut ret = Vec::with_capacity(indices.len());
        let mut mtree = self.mtree.lock().await;
        for index in indices {
            let obj_id = self.cache.get(*index)?;
            if obj_id.is_none() {
                ret.push(None);
                continue;
            }
            let obj_id = obj_id.unwrap();

            let mtree_proof = mtree.get_proof_path_by_leaf_index(*index as u64).await?;
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
        let obj_list = self.cache.get_range(start, end)?;
        if obj_list.is_empty() {
            return Ok(vec![]);
        }

        let mut ret = Vec::with_capacity(end - start);
        let mut mtree = self.mtree.lock().await;
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

    pub fn body(&self) -> &ObjectArrayBody {
        &self.body
    }

    // Get the object ID for the array if mtree is not None, otherwise return None
    // WARNING: This method don't check if the mtree is dirty
    pub fn get_obj_id(&self) -> &ObjId {
        &self.obj_id
    }

    // Calculate the object ID for the array
    pub fn calc_obj_id(&self) -> (ObjId, String) {
        self.body.calc_obj_id()
    }

    // Get the storage file path for the object array
    // This will return None if the object ID is not generated yet
    // The target file must be created by the `save()` method
    pub fn get_storage_file_path(&self) -> Option<PathBuf> {
        let factory = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().unwrap();

        let file_path = factory.get_file_path(&self.obj_id, self.storage_type());
        Some(file_path)
    }

    pub async fn open(obj_data: serde_json::Value) -> NdnResult<Self> {
        let body: ObjectArrayBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object array body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let (obj_id, _) = body.calc_obj_id();

        let factory = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().unwrap();
        let (cache, storage_type) = factory.open(&obj_id, true).await?;

        let mtree = Self::regenerate_merkle_tree(&cache, body.hash_method).await?;

        let obj_array = Self::new(obj_id, body, cache, mtree);

        Ok(obj_array)
    }

    // Regenerate the merkle tree without checking the dirty flag
    pub(crate) async fn regenerate_merkle_tree(
        cache: &Box<dyn ObjectArrayInnerCache>,
        hash_method: HashMethod,
    ) -> NdnResult<MerkleTreeObject> {
        let count = cache.len() as u64;
        let leaf_size = hash_method.hash_bytes() as u64;
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

        for i in 0..cache.len() {
            let obj_id = cache.get(i)?.unwrap();

            mtree_generator
                .append_leaf_hashes(&vec![get_obj_hash(&obj_id, hash_method)?.to_vec()])
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

        Ok(object)
    }
}

impl IntoIterator for ObjectArray {
    type Item = ObjId;
    type IntoIter = ObjectArrayOwnedIter;

    fn into_iter(self) -> Self::IntoIter {
        ObjectArrayOwnedIter::new(self.cache)
    }
}
