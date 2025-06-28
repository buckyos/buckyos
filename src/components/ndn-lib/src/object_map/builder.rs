use super::object_map::{ObjectMap, ObjectMapBody};
use super::storage::{ObjectMapInnerStorage, ObjectMapStorageType};
use super::storage_factory::{ObjectMapStorageOpenMode, GLOBAL_OBJECT_MAP_STORAGE_FACTORY};
use crate::coll::CollectionStorageMode;
use crate::{Base32Codec, HashMethod, NdnError, NdnResult, ObjId};

pub struct ObjectMapBuilder {
    hash_method: HashMethod,
    storage: Box<dyn ObjectMapInnerStorage>,
}

impl ObjectMapBuilder {
    pub async fn new(
        hash_method: HashMethod,
        coll_mode: Option<CollectionStorageMode>,
    ) -> NdnResult<Self> {
        let storage_type = ObjectMapStorageType::select_storage_type(coll_mode);

        let mut storage = GLOBAL_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open(
                None,
                false,
                Some(storage_type),
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
            storage,
        })
    }

    pub async fn open(obj_data: serde_json::Value) -> NdnResult<Self> {
        let body: ObjectMapBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let (obj_id, _) = body.calc_obj_id();

        let storage = GLOBAL_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open(
                Some(&obj_id),
                false,
                Some(body.get_storage_type()),
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

        Ok(Self {
            hash_method: body.hash_method,
            storage,
        })
    }

    // Always clone the storage for modify
    // This is to ensure that the original object map file is not modified
    pub async fn from_object_map(object_map: &ObjectMap) -> NdnResult<Self> {
        let storage = object_map.clone_storage_for_modify().await?;
        Ok(Self {
            hash_method: object_map.hash_method(),
            storage,
        })
    }

    pub async fn put_object(&mut self, key: &str, obj_id: &ObjId) -> NdnResult<()> {
        self.storage.put(&key, &obj_id).await
    }

    pub async fn get_object(&self, key: &str) -> NdnResult<Option<ObjId>> {
        let ret = self.storage.get(key).await?;
        if ret.is_none() {
            return Ok(None);
        }

        Ok(Some(ret.unwrap().0))
    }

    // Try to remove the object from the map, return the object id
    pub async fn remove_object(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        self.storage.remove(key).await
    }

    pub async fn is_object_exist(&self, key: &str) -> NdnResult<bool> {
        self.storage.is_exist(&key).await
    }

    pub fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (String, ObjId, Option<u64>)> + 'a> {
        let iter = self.storage.iter();
        Box::new(iter)
    }

    pub async fn build(mut self) -> NdnResult<ObjectMap> {
        let mtree =
            ObjectMap::regenerate_merkle_tree(&mut self.storage, self.hash_method, false).await?;

        let root_hash = mtree.get_root_hash();
        let root_hash_str = Base32Codec::to_base32(&root_hash);
        let total_count = self.storage.stat().await?.total_count;

        let body = ObjectMapBody {
            hash_method: self.hash_method.clone(),
            root_hash: root_hash_str,
            total_count,
        };

        let obj_id = body.calc_obj_id().0;

        // Save the object map to storage
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

        let object_map = ObjectMap::new(obj_id, body, self.storage, mtree);
        Ok(object_map)
    }
}
