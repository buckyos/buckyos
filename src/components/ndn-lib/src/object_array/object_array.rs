use super::storage::{
    ObjectArrayStorageReader, ObjectArrayStorageWriter,
};
use crate::mtree::{
    MerkleTreeObject, MerkleTreeObjectGenerator, MtreeReadSeek, MtreeReadWriteSeekWithSharedBuffer,
    MtreeWriteSeek, SharedBuffer,
};
use crate::{HashMethod, ObjId, OBJ_TYPE_LIST};
use crate::{NdnError, NdnResult};
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use super::proof::ObjectArrayItemProof;

#[derive(Clone, Debug)]
pub struct ObjectArrayItem {
    pub obj_id: ObjId,
    pub proof: ObjectArrayItemProof,
}

pub struct ObjectArray {
    hash_method: HashMethod,
    data: Vec<ObjId>,
    is_dirty: bool,
    mtree: Option<MerkleTreeObject>,
}

impl ObjectArray {
    pub fn new(hash_method: HashMethod) -> Self {
        Self {
            hash_method,
            data: Vec::new(),
            is_dirty: false,
            mtree: None,
        }
    }

    pub fn new_from_reader(
        hash_method: HashMethod,
        reader: Box<dyn ObjectArrayStorageReader>,
    ) -> NdnResult<Self> {
        todo!("Implement ObjectArray::new_from_reader");
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

        self.data.push(obj_id.clone());
        self.is_dirty = true;
        Ok(())
    }

    pub fn insert_object(&mut self, index: usize, obj_id: &ObjId) -> NdnResult<()> {
        if index > self.data.len() {
            let msg = format!("Index out of bounds: {}", index);
            error!("{}", msg);
            return Err(NdnError::OffsetTooLarge(msg));
        }

        // Check if obj_id.obj_hash has the same length as hash_method
        if obj_id.obj_hash.len() != self.hash_method.hash_bytes() {
            let msg = format!(
                "Object hash length does not match hash method: {}",
                obj_id.obj_hash.len()
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        self.data.insert(index, obj_id.clone());
        self.is_dirty = true;
        Ok(())
    }

    pub fn get_object(&self, index: usize) -> NdnResult<Option<&ObjId>> {
        if index >= self.data.len() {
            return Ok(None);
        }

        Ok(Some(&self.data[index]))
    }

    // Get the object ID and proof for the object at the given index, the mtree must be exists
    pub async fn get_object_with_proof(
        &mut self,
        index: usize,
    ) -> NdnResult<Option<ObjectArrayItem>> {
        if index >= self.data.len() {
            return Ok(None);
        }

        assert!(self.mtree.is_some(), "Mtree is not initialized");

        let obj_id = self.data[index].clone();
        let mtree_proof = self.mtree.as_mut().unwrap().get_proof_path_by_leaf_index(index as u64).await?;
        let proof = ObjectArrayItemProof { proof: mtree_proof };

        Ok(Some(ObjectArrayItem { obj_id, proof }))
    }

    pub async fn batch_get_object_with_proof(
        &mut self,
        indices: &[usize],
    ) -> NdnResult<Vec<Option<ObjectArrayItem>>> {
        let mut ret = Vec::with_capacity(indices.len());
        for index in indices {
            let item = self.get_object_with_proof(*index).await?;
            ret.push(item);
        }
        Ok(ret)
    }

    pub async fn range_get_object_with_proof(
        &mut self,
        start: usize,
        end: usize,
    ) -> NdnResult<Vec<Option<ObjectArrayItem>>> {
        if start >= self.data.len() || end > self.data.len() || start >= end {
            return Ok(vec![]);
        }

        let mut ret = Vec::with_capacity(end - start);
        for index in start..end {
            let item = self.get_object_with_proof(index).await?;
            ret.push(item);
        }
        Ok(ret)
    }

    pub fn remove_object(&mut self, index: usize) -> NdnResult<Option<ObjId>> {
        if index >= self.data.len() {
            return Ok(None);
        }

        let obj_id = self.data.remove(index);
        self.is_dirty = true;
        Ok(Some(obj_id))
    }

    pub fn pop_object(&mut self) -> NdnResult<Option<ObjId>> {
        if self.data.is_empty() {
            return Ok(None);
        }

        let obj_id = self.data.pop().unwrap();
        self.is_dirty = true;
        Ok(Some(obj_id))
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
        if self.data.is_empty() {
            return Err(NdnError::InvalidData("No objects in the array".to_string()));
        }

        // Check if the mtree is dirty
        if self.is_dirty {
            self.regenerate_merkle_tree().await?;
        }

        Ok(self.get_obj_id().unwrap())
    }

    // Regenerate the merkle tree without checking the dirty flag
    pub async fn regenerate_merkle_tree(&mut self) -> NdnResult<()> {
        let count = self.data.len() as u64;
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

        for obj_id in &self.data {
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

    
}
