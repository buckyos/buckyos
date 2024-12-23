use super::storage::{InnerStorage, StoragePathGenerator};
use crate::mtree::{MerkleTreeObject, MerkleTreeObjectGenerator};
use crate::mtree::{
    MtreeReadSeek, MtreeReadWriteSeekWithSharedBuffer, MtreeWriteSeek, SharedBuffer,
};
use crate::object::ObjId;
use crate::{
    hash::{HashHelper, HashMethod},
    NdnError, NdnResult,
};
use core::hash;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::collections::VecDeque;

#[derive(Serialize, Deserialize, Clone)]
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

    pub fn calc_hash(&self, hash: &Option<Vec<u8>>) -> Vec<u8> {
        let mut hasher = sha2::Sha256::new();
        match hash {
            Some(h) => {
                hasher.update(h);
            }
            None => {
                hasher.update(self.encode().unwrap());
            }
        }
        hasher.finalize().to_vec()
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ObjectMapMeta {
    // Default is Sha256
    pub hash_method: HashMethod,

    // Default is 2
    pub name_len: usize,

    // Default is 1
    pub level: usize,

    // Total count of objects
    pub count: usize,
}

pub struct ObjectMap {
    pub meta: ObjectMapMeta,
    pub is_dirty: bool,
    pub storage: Box<dyn InnerStorage>,
}

impl ObjectMap {
    pub async fn put_object(
        &mut self,
        key: impl Into<String>,
        obj_id: ObjId,
        meta: Option<Vec<u8>>,
    ) -> NdnResult<()> {
        let key = key.into();
        let item = ObjectMapItem::new(key.clone(), obj_id, meta);
        let path = StoragePathGenerator::gen_path(&key, self.meta.name_len, self.meta.level);
        let data = item.encode()?;

        self.storage.put(&path, &data).await.map_err(|e| {
            let msg = format!("Error putting object map item: {}", e);
            error!("{}", msg);
            e
        })?;

        Ok(())
    }

    pub async fn get_object(&self, key: &str) -> NdnResult<Option<ObjectMapItem>> {
        let path = StoragePathGenerator::gen_path(key, self.meta.name_len, self.meta.level);
        let ret = self.storage.get(&path).await.map_err(|e| {
            let msg = format!("Error getting object map item: {}, {}", key, e);
            error!("{}", msg);
            e
        })?;

        if ret.is_none() {
            return Ok(None);
        }

        let item = ObjectMapItem::decode(&ret.unwrap()).map_err(|e| {
            let msg = format!("Error decoding object map item: {}, {}", key, e);
            error!("{}", msg);
            e
        })?;

        Ok(Some(item))
    }

    // Try to remove the object from the map, return the object id and meta data
    pub async fn remove_object(&mut self, key: &str) -> NdnResult<(ObjId, Option<Vec<u8>>)> {
        let key = key.as_ref();
        let path = StoragePathGenerator::gen_path(key, self.meta.name_len, self.meta.level);
        let ret = self.storage.remove(&path).await.map_err(|e| {
            let msg = format!("Error removing object map item: {}", e);
            error!("{}", msg);
            e
        })?;

        let item = ObjectMapItem::decode(&ret).map_err(|e| {
            let msg = format!("Error decoding object map item: {}, {}", key, e);
            error!("{}", msg);
            e
        })?;

        Ok((item.obj_id, item.meta))
    }

    pub async fn list_objects(&self, path: &str, depth: usize) -> NdnResult<()> {
        let count = self.meta.count;
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

        let mut stack = VecDeque::new();
        stack.push_back(("/".to_string(), 0)); // Start at root

        while let Some((current_path, depth)) = stack.pop_front() {
            let children = self.storage.list(&current_path).await?;
            if depth == self.meta.level {
                // Load all objects at current level
                for child in children {
                    // Skip reserved files such as meta data
                    if child.starts_with('.') {
                        continue;
                    }

                    let child_path = if current_path == "/" {
                        format!("/{}", child)
                    } else {
                        format!("{}/{}", current_path, child)
                    };

                    let item = self.storage.get(&child_path).await?;
                    if item.is_none() {
                        let msg = format!("Error getting object map item: {}", child_path);
                        error!("{}", msg);
                        return Err(NdnError::InvalidState(msg));
                    }

                    let item = item.unwrap();
                    let hash = HashHelper::calc_hash(self.meta.hash_method, &item);
                    mtree_generator
                        .append_leaf_hashes(&vec![hash])
                        .await
                        .map_err(|e| {
                            let msg = format!("Error appending leaf hashes: {}, {}", child_path, e);
                            error!("{}", msg);
                            e
                        })?;
                }
                continue;
            }

            // Add children dir to stack for further processing
            for child in children {
                if child.starts_with('.') {
                    continue;
                }

                let child_path = if current_path == "/" {
                    format!("/{}", child)
                } else {
                    format!("{}/{}", current_path, child)
                };

                stack.push_back((child_path, depth + 1));
            }
        }

        Ok(())
    }
}
