use super::object_array::{ObjectArray, ObjectArrayBody};
use super::storage::{ObjectArrayCacheType, ObjectArrayInnerCache, ObjectArrayStorageType};
use super::storage_factory::{
    ObjectArrayCacheFactory, ObjectArrayStorageFactory, GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY,
};
use crate::coll::{get_obj_hash, CollectionStorageMode};
use crate::mtree::{
    MerkleTreeObject, MerkleTreeObjectGenerator, MtreeReadSeek, MtreeReadWriteSeekWithSharedBuffer,
    MtreeWriteSeek, SharedBuffer,
};
use crate::{
    build_named_object_by_json, mtree, Base32Codec, HashMethod, NdnError, NdnResult, ObjId,
    OBJ_TYPE_LIST,
};
use std::io::SeekFrom;

pub struct ObjectArrayBuilder {
    hash_method: HashMethod,
    cache: Box<dyn ObjectArrayInnerCache>,
}

impl ObjectArrayBuilder {
    pub fn new(hash_method: HashMethod) -> Self {
        // Always use memory cache for object array builder
        let cache: Box<dyn ObjectArrayInnerCache> =
            ObjectArrayCacheFactory::create_cache(ObjectArrayCacheType::Memory);

        Self { hash_method, cache }
    }

    pub async fn open(obj_data: serde_json::Value) -> NdnResult<Self> {
        let body: ObjectArrayBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object array body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let (obj_id, _) = body.calc_obj_id();

        let factory = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().unwrap();
        let (cache, storage_type) = factory.open(&obj_id, false).await?;

        Ok(Self {
            hash_method: body.hash_method,
            cache,
        })
    }

    pub fn from_object_array(obj_array: &ObjectArray) -> NdnResult<Self> {
        let cache = obj_array.cache().clone_cache(false)?;

        Ok(Self {
            hash_method: obj_array.hash_method(),
            cache,
        })
    }

    pub fn from_object_array_owned(obj_array: ObjectArray) -> Self {
        Self {
            hash_method: obj_array.hash_method(),
            cache: obj_array.into_cache(),
        }
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn get_object(&self, index: usize) -> NdnResult<Option<ObjId>> {
        self.cache.get(index)
    }

    pub fn append_object(&mut self, obj_id: &ObjId) -> NdnResult<()> {
        // Check if obj_id.obj_hash is valid
        get_obj_hash(obj_id, self.hash_method)?;

        self.cache.append(obj_id)
    }

    pub fn insert_object(&mut self, index: usize, obj_id: &ObjId) -> NdnResult<()> {
        // Check if obj_id.obj_hash is valid
        get_obj_hash(obj_id, self.hash_method)?;

        self.cache.insert(index, obj_id)
    }

    pub fn remove_object(&mut self, index: usize) -> NdnResult<Option<ObjId>> {
        self.cache.remove(index)
    }

    pub fn pop_object(&mut self) -> NdnResult<Option<ObjId>> {
        self.cache.pop()
    }

    pub fn clear(&mut self) -> NdnResult<()> {
        self.cache.clear()
    }

    pub async fn build(self) -> NdnResult<ObjectArray> {
        // First regenerate the merkle tree and get the root hash
        let mtree = ObjectArray::regenerate_merkle_tree(&self.cache, self.hash_method).await?;

        let root_hash = mtree.get_root_hash();
        let root_hash_str = Base32Codec::to_base32(&root_hash);

        let total_count = self.cache.len() as u64;
        let storage_mode = CollectionStorageMode::select_mode(Some(total_count));
        let storage_type = match storage_mode {
            CollectionStorageMode::Simple => ObjectArrayStorageType::JSONFile,
            CollectionStorageMode::Normal => ObjectArrayStorageType::Arrow,
        };

        // Then build the object array body and calculate the object id
        let body = ObjectArrayBody {
            root_hash: root_hash_str,
            hash_method: self.hash_method,
            total_count: self.cache.len() as u64,
        };

        let (obj_id, s) = body.calc_obj_id();

        // Then we save the object array to the storage
        Self::save(&self, &obj_id, storage_type).await?;

        let obj_array = ObjectArray::new(obj_id, body, self.cache, mtree);

        Ok(obj_array)
    }

    async fn save(&self, obj_id: &ObjId, storage_type: ObjectArrayStorageType) -> NdnResult<()> {
        let factory = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().unwrap();
        let mut writer = factory
            .open_writer(&obj_id, None, Some(storage_type))
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
}
