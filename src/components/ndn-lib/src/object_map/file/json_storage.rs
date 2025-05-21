use super::super::storage::{
    ObjectMapInnerStorage, ObjectMapInnerStorageStat, ObjectMapStorageType,
};
use crate::{NdnError, NdnResult, ObjId};
use http_types::content;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod serde_objid_as_base32_helper {
    use super::ObjId;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &ObjId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_base32())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ObjId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        ObjId::new(&s).map_err(serde::de::Error::custom)
    }
}

mod serde_u64_as_string_helper {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(val: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match val {
            Some(num) => serializer.serialize_some(&num.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Deserialize::deserialize(deserializer)?;
        if let Some(value_str) = s {
            value_str
                .parse()
                .map(Some)
                .map_err(serde::de::Error::custom)
        } else {
            Ok(None)
        }
    }
}

mod base64_serde {
    use base64::{engine::general_purpose, Engine as _};
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(val: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match val {
            Some(bytes) => serializer.serialize_some(&general_purpose::STANDARD.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Deserialize::deserialize(deserializer)?;
        if let Some(value_str) = s {
            general_purpose::STANDARD
                .decode(value_str)
                .map_err(serde::de::Error::custom)
                .map(|v| Some(v))
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JSONStorageItem {
    #[serde(with = "serde_objid_as_base32_helper")]
    value: ObjId,

    #[serde(with = "serde_u64_as_string_helper")]
    mtree_index: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JSONStorageData {
    content: BTreeMap<String, JSONStorageItem>,

    #[serde(with = "base64_serde")]
    meta: Option<Vec<u8>>,

    #[serde(with = "base64_serde")]
    mtree_data: Option<Vec<u8>>,
}

impl JSONStorageData {
    fn new() -> Self {
        Self {
            content: BTreeMap::new(),
            meta: None,
            mtree_data: None,
        }
    }
}

pub struct ObjectMapJSONStorage {
    read_only: bool,
    file: PathBuf,

    data: Option<JSONStorageData>,
    is_dirty: bool,
}

impl ObjectMapJSONStorage {
    pub fn new(file: PathBuf, read_only: bool) -> NdnResult<Self> {
        let data = if file.exists() {
            let data = Self::load(&file)?;
            Some(data)
        } else {
            None
        };

        Ok(Self {
            read_only,
            file,
            data,
            is_dirty: false,
        })
    }

    fn load(file_path: &Path) -> NdnResult<JSONStorageData> {
        if !file_path.exists() {
            let msg = format!("File does not exist: {:?}", file_path);
            error!("{}", msg);
            return Err(NdnError::IoError(msg));
        }

        let file = std::fs::File::open(&file_path).map_err(|e| {
            let msg = format!("Failed to open file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        // Deserialize the data from JSON
        let data: JSONStorageData = serde_json::from_reader(file).map_err(|e| {
            let msg = format!("Failed to read data from file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        Ok(data)
    }

    fn check_read_only(&self) -> NdnResult<()> {
        if self.read_only {
            let msg = format!("Storage is read-only: {}", self.file.display());
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }
        Ok(())
    }

    async fn clone_for_modify(&self, target: &Path) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        // First check if target is same as current file
        if target == self.file {
            let msg = format!("Target file is same as current file: {}", target.display());
            error!("{}", msg);
            return Err(NdnError::AlreadyExists(msg));
        }

        if self.is_dirty {
            // Clone the current storage to a new file
            let mut new_storage = Self {
                read_only: false,
                file: target.to_path_buf(),
                data: None,
                is_dirty: false,
            };

            if let Some(data) = &self.data {
                new_storage.data = Some(data.clone());
            }

            new_storage.save(&target).await?;
            Ok(Box::new(new_storage))
        } else {
            // If the storage is not dirty, just copy the file
            std::fs::copy(&self.file, target).map_err(|e| {
                let msg = format!("Failed to copy file: {:?}, {}", target, e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;

            // Create a new storage instance
            let new_storage = Self {
                read_only: false,
                file: target.to_path_buf(),
                data: self.data.clone(),
                is_dirty: false,
            };

            // Return the new storage
            Ok(Box::new(new_storage))
        }
    }

    /*
    fn get_node(&mut self, name: &str, auto_create: bool) -> Option<&mut serde_json::Value> {
        let mut storage = if let Some(storage) = &mut self.storage {
            storage
        }  else {
            if !auto_create {
                return None;
            }

            // Initialize the storage if it's None
            let node = serde_json::Map::new();
            self.storage = Some(node);

            self.storage.as_mut().unwrap()
        };

        if let Some(node) = storage.get_mut(name) {
            Some(node)
        } else {
            if !auto_create {
                return None;
            }

            // Create a new node if it doesn't exist
            let node = serde_json::Value::Object(serde_json::Map::new());
            storage.insert(name.to_string(), node);
            Some(storage.get_mut(name).unwrap())
        }
    }
    */
}

#[async_trait::async_trait]
impl ObjectMapInnerStorage for ObjectMapJSONStorage {
    fn get_type(&self) -> ObjectMapStorageType {
        ObjectMapStorageType::JSONFile
    }

    fn is_readonly(&self) -> bool {
        self.read_only
    }

    async fn put(&mut self, key: &str, value: &ObjId) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        // Modify the JSON node
        if self.data.is_none() {
            self.data = Some(JSONStorageData::new());
        }

        let data = self.data.as_mut().unwrap();
        data.content.insert(
            key.to_string(),
            JSONStorageItem {
                value: value.clone(),
                mtree_index: None,
            },
        );

        // Mark the storage as dirty
        self.is_dirty = true;

        Ok(())
    }

    async fn get(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>> {
        if let Some(data) = &self.data {
            if let Some(item) = data.content.get(key) {
                Ok(Some((item.value.clone(), item.mtree_index)))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        // Check if the storage is read-only
        self.check_read_only()?;

        if let Some(data) = &mut self.data {
            if let Some(item) = data.content.remove(key) {
                // Mark the storage as dirty
                self.is_dirty = true;

                Ok(Some(item.value))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn is_exist(&self, key: &str) -> NdnResult<bool> {
        if let Some(data) = &self.data {
            Ok(data.content.contains_key(key))
        } else {
            Ok(false)
        }
    }

    async fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>> {
        if let Some(data) = &self.data {
            let start = page_index * page_size;
            let end = start + page_size;
            let list = data
                .content
                .iter() // Get an iterator of (&String, &Vec<u8>)
                .map(|(k, _)| k.clone()) // We only need the keys (paths)
                .skip(start) // Skip the first 'start' paths
                .take(page_size) // Take the next 'page_size' paths
                .collect(); // Collect into a Vec<String>

            Ok(list)
        } else {
            Ok(vec![])
        }
    }
    async fn stat(&self) -> NdnResult<ObjectMapInnerStorageStat> {
        if let Some(data) = &self.data {
            Ok(ObjectMapInnerStorageStat {
                total_count: data.content.len() as u64,
            })
        } else {
            Ok(ObjectMapInnerStorageStat { total_count: 0 })
        }
    }

    // Use to store meta data
    async fn put_meta(&mut self, value: &[u8]) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        // Modify the JSON node
        if self.data.is_none() {
            self.data = Some(JSONStorageData::new());
        }

        let data = self.data.as_mut().unwrap();
        data.meta = Some(value.to_vec());

        // Mark the storage as dirty
        self.is_dirty = true;

        Ok(())
    }

    async fn get_meta(&self) -> NdnResult<Option<Vec<u8>>> {
        if let Some(data) = &self.data {
            Ok(data.meta.clone())
        } else {
            Ok(None)
        }
    }

    // Use to store the index of the mtree node
    async fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        // Modify the JSON node
        if self.data.is_none() {
            self.data = Some(JSONStorageData::new());
        }

        let data = self.data.as_mut().unwrap();
        if let Some(item) = data.content.get_mut(key) {
            item.mtree_index = Some(index);

            // Mark the storage as dirty
            self.is_dirty = true;

            return Ok(());
        }

        let msg = format!("No such key: {}", key);
        Err(NdnError::NotFound(msg))
    }

    async fn get_mtree_index(&self, key: &str) -> NdnResult<Option<u64>> {
        if let Some(data) = &self.data {
            if let Some(item) = data.content.get(key) {
                Ok(item.mtree_index)
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
    async fn put_mtree_data(&mut self, value: &[u8]) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        // Modify the JSON node
        if self.data.is_none() {
            self.data = Some(JSONStorageData::new());
        }

        let data = self.data.as_mut().unwrap();
        data.mtree_data = Some(value.to_vec());

        // Mark the storage as dirty
        self.is_dirty = true;

        Ok(())
    }

    async fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>> {
        if let Some(data) = &self.data {
            Ok(data.mtree_data.clone())
        } else {
            Ok(None)
        }
    }

    // Clone the storage to a new file.
    // If the target file exists, it will be failed.
    async fn clone(
        &self,
        target: &Path,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        if read_only {
            let ret = Self {
                read_only,
                file: target.to_path_buf(),
                data: self.data.clone(),
                is_dirty: self.is_dirty,
            };

            Ok(Box::new(ret))
        } else {
            self.clone_for_modify(target).await
        }
    }

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, file: &Path) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        if !self.is_dirty {
            // No changes to save
            return Ok(());
        }

        // Serialize the data to JSON and write it to the file
        let f = std::fs::File::create(&file).map_err(|e| {
            let msg = format!("Failed to create file: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        serde_json::to_writer(f, &self.data).map_err(|e| {
            let msg = format!("Failed to write data to file: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        // Mark the storage as clean
        self.is_dirty = false;
        self.file = file.to_path_buf();

        Ok(())
    }
}
