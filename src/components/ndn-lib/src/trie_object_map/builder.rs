use super::object_map::{TrieObjectMap, TrieObjectMapBody};
use super::storage::{TrieObjectMapInnerStorage, TrieObjectMapStorageType};
use super::storage_factory::{
    TrieObjectMapStorageOpenMode, GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY,
};
use crate::coll::CollectionStorageMode;
use crate::{Base32Codec, HashMethod, NdnError, NdnResult, ObjId};

pub struct TrieObjectMapBuilder {
    hash_method: HashMethod,
    total_count: u64,
    storage: Box<dyn TrieObjectMapInnerStorage>,
}

impl TrieObjectMapBuilder {
    pub async fn new(
        hash_method: HashMethod,
        coll_mode: Option<CollectionStorageMode>,
    ) -> NdnResult<Self> {
        let storage_type = TrieObjectMapStorageType::select_storage_type(coll_mode);

        let storage = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open_by_hash_method(
                None,
                false,
                Some(storage_type),
                hash_method,
                TrieObjectMapStorageOpenMode::CreateNew,
            )
            .await?;

        Ok(Self {
            hash_method,
            total_count: 0,
            storage,
        })
    }

    pub async fn open(obj_data: serde_json::Value) -> NdnResult<Self> {
        let body: TrieObjectMapBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding trie object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let (obj_id, _) = body.calc_obj_id();

        let storage = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open_by_hash_method(
                Some((&obj_id, body.root_hash.as_str())),
                false,
                Some(body.get_storage_type()),
                body.hash_method,
                TrieObjectMapStorageOpenMode::OpenExisting,
            )
            .await
            .map_err(|e| {
                let msg = format!(
                    "Error opening trie object map storage: {}, {}",
                    obj_id.to_base32(), e
                );
                error!("{}", msg);
                e
            })?;

        Ok(Self {
            hash_method: body.hash_method,
            total_count: body.total_count,
            storage,
        })
    }

    pub async fn from_trie_object_map(trie_object_map: &TrieObjectMap) -> NdnResult<Self> {
        let storage = trie_object_map.clone_storage_for_modify().await?; // Clone in read-write mode

        let ret = Self {
            hash_method: trie_object_map.hash_method(),
            total_count: trie_object_map.len(),
            storage,
        };

        Ok(ret)
    }

    // Get the storage type of current using storage, maybe changed after build
    pub fn storage_type(&self) -> TrieObjectMapStorageType {
        self.storage.get_type()
    }

    pub fn len(&self) -> u64 {
        self.total_count
    }

    pub fn put_object(&mut self, key: &str, obj_id: &ObjId) -> NdnResult<Option<ObjId>> {
        let ret = self.storage.put(key, &obj_id)?;
        if ret.is_none() {
            self.total_count += 1; // Increment total count only if a new object is added
        }

        Ok(ret)
    }

    pub fn get_object(&self, key: &str) -> NdnResult<Option<ObjId>> {
        self.storage.get(key)
    }

    pub fn remove_object(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        let ret = self.storage.remove(key)?;
        if ret.is_some() {
            self.total_count -= 1; // Decrement total count only if an object is removed
        }

        Ok(ret)
    }

    pub fn is_object_exist(&self, key: &str) -> NdnResult<bool> {
        self.storage.is_exist(key)
    }

    pub fn iter<'a>(&'a self) -> NdnResult<Box<dyn Iterator<Item = (String, ObjId)> + 'a>> {
        Ok(Box::new(self.storage.iter()?))
    }

    pub fn traverse(
        &self,
        callback: &mut dyn FnMut(String, ObjId) -> NdnResult<()>,
    ) -> NdnResult<()> {
        self.storage.traverse(callback)
    }

    pub async fn build(self) -> NdnResult<TrieObjectMap> {
        let root_hash = self.storage.root();
        let root_hash_str = Base32Codec::to_base32(&root_hash);

        // First regenerate the merkle tree and get the root hash
        let body = TrieObjectMapBody {
            hash_method: self.hash_method,
            root_hash: root_hash_str,
            total_count: self.total_count,
        };

        let obj_id = body.calc_obj_id().0;

        // Check if the collection storage mode is matched
        let storage_mode = CollectionStorageMode::select_mode(Some(self.total_count));
        let storage_type = TrieObjectMapStorageType::select_storage_type(Some(storage_mode));

        let storage = if self.storage.get_type() != storage_type {
            GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
                .get()
                .unwrap()
                .switch_storage_type(
                    (&obj_id, &body.root_hash),
                    self.storage,
                    self.hash_method,
                    storage_type,
                )
                .await?
        } else {
            self.storage
        };

        // Create the TrieObjectMap with the new body and storage
        let trie_object_map = TrieObjectMap::new(obj_id, body, storage);

        Ok(trie_object_map)
    }
}
