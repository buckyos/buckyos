use super::memory_cache::ObjectArrayMemoryCache;
use super::storage::{
    ObjectArrayCacheType, ObjectArrayInnerCache, ObjectArrayStorageType, ObjectArrayStorageWriter,
};
use super::{file::*, ObjectArray};
use crate::{NdnError, NdnResult, ObjId};
use std::path::{Path, PathBuf};
use once_cell::sync::OnceCell;

pub struct ObjectArrayCacheFactory {}

impl ObjectArrayCacheFactory {
    /// Create a new cache based on the storage type.
    pub fn create_cache(cache_type: ObjectArrayCacheType) -> Box<dyn ObjectArrayInnerCache> {
        match cache_type {
            ObjectArrayCacheType::Memory => {
                let cache = ObjectArrayMemoryCache::new();
                Box::new(cache)
            }
            ObjectArrayCacheType::Arrow => {
                let cache = ObjectArrayArrowCache::new_empty();
                Box::new(cache)
            }
        }
    }
}

pub struct ObjectArrayStorageFactory {
    data_path: PathBuf,
}

impl ObjectArrayStorageFactory {
    pub fn new(data_path: &Path) -> Self {
        Self {
            data_path: data_path.to_path_buf(),
        }
    }

    pub fn get_file_path(&self, id: &ObjId, storage_type: ObjectArrayStorageType) -> PathBuf {
        match storage_type {
            ObjectArrayStorageType::Arrow => self.data_path.join(format!("{}.arrow", id.to_base32())),
            ObjectArrayStorageType::JSONFile => self.data_path.join(format!("{}.json", id.to_base32())),
        }
    }

    pub async fn open_writer(
        &self,
        id: &ObjId,
        len: Option<usize>,
        storage_type: Option<ObjectArrayStorageType>,
    ) -> NdnResult<Box<dyn ObjectArrayStorageWriter>> {
        // First make sure the directory exists
        if !self.data_path.exists() {
            std::fs::create_dir_all(&self.data_path).map_err(|e| {
                let msg = format!(
                    "Error creating directory {}: {}",
                    self.data_path.display(),
                    e
                );
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;
        }

        let storage_type = storage_type.unwrap_or(ObjectArrayStorageType::default());
        let file_path = self.get_file_path(id, storage_type);
    
        match storage_type {
            ObjectArrayStorageType::Arrow => {
                let writer = ObjectArrayArrowWriter::new(file_path, len);
                Ok(Box::new(writer))
            }
            ObjectArrayStorageType::JSONFile => {
                let writer = ObjectArrayJSONWriter::new(file_path);
                Ok(Box::new(writer))
            }
        }
    }

    // Search file in the data path with the given id and unknown file extension.
    fn search_file(&self, id: &ObjId) -> Option<(ObjectArrayStorageType, PathBuf)> {
        struct Item {
            storage_type: ObjectArrayStorageType,
            ext: &'static str,
        }

        const EXTENSIONS: &[Item] = &[
            Item {
                storage_type: ObjectArrayStorageType::Arrow,
                ext: "arrow",
            },
            Item {
                storage_type: ObjectArrayStorageType::JSONFile,
                ext: "json",
            },
        ];

        for item in EXTENSIONS {
            let file_path = self.data_path.join(format!("{}.{}", id.to_base32(), item.ext));
            if file_path.exists() {
                info!("Found file: {:?}", file_path);
                return Some((item.storage_type, file_path));
            }
        }

        warn!("File not found for ObjectArray: {:?}", id.to_base32());
        None
    }

    // Open the file with the given id and unknown file extension, will search the file in the data path.
    // If the file is not found, return an error.
    pub async fn open(
        &self,
        id: &ObjId,
        readonly: bool,
    ) -> NdnResult<(Box<dyn ObjectArrayInnerCache>, ObjectArrayStorageType)> {
        let (storage_type, file_path) = self.search_file(id).ok_or_else(|| {
            let msg = format!("File not found for ObjectArray: {:?}", id.to_base32());
            error!("{}", msg);
            NdnError::NotFound(msg)
        })?;

        let cache = self.open_inner(&file_path, readonly, storage_type).await?;

        Ok((cache, storage_type))
    }

    // Open with specific storage type
    pub async fn open_with_type(
        &self,
        id: &ObjId,
        readonly: bool,
        storage_type: ObjectArrayStorageType,
    ) -> NdnResult<Box<dyn ObjectArrayInnerCache>> {
        let file_path = self.get_file_path(id, storage_type);

        self.open_inner(&file_path, readonly, storage_type).await
    }

    async fn open_inner(
        &self,
        file_path: &Path,
        readonly: bool,
        storage_type: ObjectArrayStorageType,
    ) -> NdnResult<Box<dyn ObjectArrayInnerCache>> {
        match storage_type {
            ObjectArrayStorageType::Arrow => {
                let reader = ObjectArrayArrowReader::open(&file_path, readonly).await?;
                Ok(reader.into_cache())
            }
            ObjectArrayStorageType::JSONFile => {
                let reader = ObjectArrayJSONReader::open(&file_path, readonly).await?;
                Ok(reader.into_cache())
            }
        }
    }
}


pub static GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY: OnceCell<ObjectArrayStorageFactory> = OnceCell::new();