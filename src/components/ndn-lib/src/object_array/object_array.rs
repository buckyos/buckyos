use super::proof::ObjectArrayItemProof;
use super::storage::{
    ObjectArrayInnerCache, ObjectArrayStorageType,
    ObjectArrayStorageWriter,
};
use super::storage_factory::{ObjectArrayCacheFactory, ObjectArrayStorageFactory};
use crate::mtree::{
    self, MerkleTreeObject, MerkleTreeObjectGenerator, MtreeReadSeek,
    MtreeReadWriteSeekWithSharedBuffer, MtreeWriteSeek, SharedBuffer,
};
use crate::{HashMethod, ObjId, OBJ_TYPE_LIST};
use crate::{NdnError, NdnResult};
use http_types::cache;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

#[derive(Clone, Debug)]
pub struct ObjectArrayItem {
    pub obj_id: ObjId,
    pub proof: ObjectArrayItemProof,
}

pub struct ObjectArray {
    hash_method: HashMethod,
    cache: Box<dyn ObjectArrayInnerCache>,
    is_dirty: bool,
    mtree: Option<MerkleTreeObject>,
}

impl ObjectArray {
    pub fn new(storage_type: ObjectArrayStorageType, hash_method: HashMethod) -> Self {
        let cache: Box<dyn ObjectArrayInnerCache> =
            ObjectArrayCacheFactory::create_cache(storage_type);
        Self {
            hash_method,
            cache,
            is_dirty: false,
            mtree: None,
        }
    }

    pub fn new_from_cache(
        hash_method: HashMethod,
        cache: Box<dyn ObjectArrayInnerCache>,
    ) -> NdnResult<Self> {
        let obj_array = Self {
            hash_method,
            cache,
            is_dirty: false,
            mtree: None,
        };

        Ok(obj_array)
    }

    pub fn is_readonly(&self) -> bool {
        self.cache.is_readonly()
    }

    pub fn append_object(&mut self, obj_id: &ObjId) -> NdnResult<()> {
        // Check if obj_id.obj_hash has the same length as hash_method
        if obj_id.obj_hash.len() != self.hash_method.hash_bytes() {
            let msg = format!(
                "Object hash length does not match hash method: {}",
                obj_id.obj_hash.len()
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        self.cache.append(obj_id)?;
        self.is_dirty = true;
        Ok(())
    }

    pub fn get_object(&self, index: usize) -> NdnResult<Option<ObjId>> {
        self.cache.get(index)
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
            let proof = ObjectArrayItemProof { proof: mtree_proof };

            let obj_id = obj_list[i].clone();
            let obj_id = ObjId::new_by_raw(obj_id.obj_type.clone(), obj_id.obj_hash.clone());
            ret.push(Some(ObjectArrayItem { obj_id, proof }));
        }

        Ok(ret)
    }

    // Get the object ID for the array if mtree is not None, otherwise return None
    // WARNING: This method don't check if the mtree is dirty
    pub fn get_obj_id(&self) -> Option<ObjId> {
        if self.mtree.is_none() {
            return None;
        }

        // Get the root hash from the mtree
        let root_hash = self.mtree.as_ref().unwrap().get_root_hash();
        let obj_id = ObjId::new_by_raw(OBJ_TYPE_LIST.to_string(), root_hash);
        Some(obj_id)
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
        if self.is_dirty {
            self.regenerate_merkle_tree().await?;
        }

        Ok(self.get_obj_id().unwrap())
    }

    // Regenerate the merkle tree without checking the dirty flag
    pub async fn regenerate_merkle_tree(&mut self) -> NdnResult<()> {
        let count = self.cache.len() as u64;
        let leaf_size = self.hash_method.hash_bytes() as u64;
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

        for i in 0..self.cache.len() {
            let obj_id = self.cache.get(i)?.unwrap();

            mtree_generator
                .append_leaf_hashes(&vec![obj_id.obj_hash.clone()])
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

        self.regenerate_merkle_tree().await?;

        self.is_dirty = false;

        Ok(())
    }

    pub async fn save(&mut self, writer: &mut Box<dyn ObjectArrayStorageWriter>) -> NdnResult<()> {
        // Write the object array to the storage
        // TODO: use batch read and write to improve performance
        for i in 0..self.cache.len() {
            let obj_id = self.cache.get(i)?.unwrap();
            writer.append(&obj_id).await?;
        }

        writer.flush().await?;

        Ok(())
    }
}
