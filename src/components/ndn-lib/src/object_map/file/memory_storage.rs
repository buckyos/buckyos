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
    #[serde(with = "serde_objid_as_base32_helper", rename = "v")]
    value: ObjId,

    #[serde(with = "serde_u64_as_string_helper", rename = "i")]
    mtree_index: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct JSONStorageData {
    content: BTreeMap<String, JSONStorageItem>,

    #[serde(with = "base64_serde")]
    mtree_data: Option<Vec<u8>>,
}

impl JSONStorageData {
    fn new() -> Self {
        Self {
            content: BTreeMap::new(),
            mtree_data: None,
        }
    }
}

pub struct ObjectMapMemoryStorage {
    read_only: bool,

    data: Option<JSONStorageData>,
    is_dirty: bool,
}

impl ObjectMapMemoryStorage {
    pub fn new(data: serde_json::Value, read_only: bool) -> NdnResult<Self> {
        let data = serde_json::from_value(data).map_err(|e| {
            let msg = format!("Failed to parse JSON data: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(Self {
            read_only,
            data: Some(data),
            is_dirty: false,
        })
    }


    pub fn new_empty(read_only: bool) -> Self {
        Self {
            read_only,
            data: Some(JSONStorageData::new()),
            is_dirty: false,
        }
    }

    pub fn new_raw(data: Option<JSONStorageData>, read_only: bool) -> Self {
        Self {
            read_only,
            data,
            is_dirty: false,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    pub(crate) fn set_dirty(&mut self, dirty: bool) {
        self.is_dirty = dirty;
    }

    pub(crate) fn get_data(&self) -> Option<&JSONStorageData> {
        self.data.as_ref()
    }

    pub fn check_read_only(&self) -> NdnResult<()> {
        if self.read_only {
            let msg = format!("Storage is read-only");
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }
        Ok(())
    }

    pub fn clone_for_read(&self) -> Self {
        // Create a new instance with the same data but different read-only status
        let new_storage = Self {
            read_only: true,
            data: self.data.clone(),
            is_dirty: self.is_dirty,
        };

        new_storage
    }

    pub fn clone_for_modify(&self) -> Self {
        // Create a new instance with the same data but different read-only status
        let new_storage = Self {
            read_only: false,
            data: self.data.clone(),
            is_dirty: self.is_dirty,
        };

        new_storage
    }
}

#[async_trait::async_trait]
impl ObjectMapInnerStorage for ObjectMapMemoryStorage {
    fn get_type(&self) -> ObjectMapStorageType {
        ObjectMapStorageType::Memory
    }

    fn is_readonly(&self) -> bool {
        self.read_only
    }

    fn put(&mut self, key: &str, value: &ObjId) -> NdnResult<()> {
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

    fn put_with_index(&mut self, key: &str, value: &ObjId, index: Option<u64>) -> NdnResult<()> {
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
                mtree_index: index,
            },
        );

        // Mark the storage as dirty
        self.is_dirty = true;

        Ok(())
    }

    fn get(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>> {
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

    fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
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

    fn is_exist(&self, key: &str) -> NdnResult<bool> {
        if let Some(data) = &self.data {
            Ok(data.content.contains_key(key))
        } else {
            Ok(false)
        }
    }

    fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>> {
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

    fn stat(&self) -> NdnResult<ObjectMapInnerStorageStat> {
        if let Some(data) = &self.data {
            Ok(ObjectMapInnerStorageStat {
                total_count: data.content.len() as u64,
            })
        } else {
            Ok(ObjectMapInnerStorageStat { total_count: 0 })
        }
    }

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (String, ObjId, Option<u64>)> + 'a> {
        if let Some(data) = &self.data {
            Box::new(
                data.content
                    .iter()
                    .map(|(k, v)| (k.clone(), v.value.clone(), v.mtree_index)),
            )
        } else {
            Box::new(std::iter::empty())
        }
    }

    // Use to store the index of the mtree node
    fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()> {
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

    fn get_mtree_index(&self, key: &str) -> NdnResult<Option<u64>> {
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

    fn put_mtree_data(&mut self, value: &[u8]) -> NdnResult<()> {
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

    fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>> {
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
        _target: &Path,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        let ret = Self {
            read_only,
            data: self.data.clone(),
            is_dirty: self.is_dirty,
        };

        Ok(Box::new(ret))
    }

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, file: &Path) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        // Mark the storage as clean
        self.is_dirty = false;

        Ok(())
    }

    async fn dump(&self) -> NdnResult<Option<serde_json::Value>> {
        if let Some(data) = &self.data {
            let json_value = serde_json::to_value(data).map_err(|e| {
                let msg = format!("Error encoding JSON storage data: {}", e);
                error!("{}", msg);
                NdnError::InvalidData(msg)
            })?;
            Ok(Some(json_value))
        } else {
            Ok(None)
        }
    }
}
