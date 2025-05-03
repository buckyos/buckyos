use super::{file::*, ObjectArray};
use super::storage::{
    ObjectArrayCacheType, ObjectArrayInnerCache, ObjectArrayStorageType,
    ObjectArrayStorageWriter,
};
use super::memory_cache::ObjectArrayMemoryCache;
use crate::{NdnError, NdnResult, ObjId};
use std::path::{Path, PathBuf};

pub struct ObjectArrayCacheFactory {}

impl ObjectArrayCacheFactory {
    /// Create a new cache based on the storage type.
    pub fn create_cache(
        cache_type: ObjectArrayCacheType,
    ) -> Box<dyn ObjectArrayInnerCache> {
    
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
    storage_type: ObjectArrayStorageType,
}

impl ObjectArrayStorageFactory {
    pub fn new(data_path: &Path, storage_type: Option<ObjectArrayStorageType>) -> Self {
        Self {
            data_path: data_path.to_path_buf(),
            storage_type: storage_type.unwrap_or(ObjectArrayStorageType::default()),
        }
    }

    pub async fn open_writer(
        &self,
        id: &ObjId,
        len: Option<usize>,
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

        match self.storage_type {
            ObjectArrayStorageType::Arrow => {
                let file_path = self.data_path.join(format!("{}.arrow", id.to_base32()));
                let writer = ObjectArrayArrowWriter::new(file_path, len);
                Ok(Box::new(writer))
            }
            ObjectArrayStorageType::SQLite => {
                unimplemented!("SQLite storage is not implemented yet");
            }
            ObjectArrayStorageType::SimpleFile => {
                unimplemented!("Simple file storage is not implemented yet");
            }
        }
    }

    pub async fn open(&self, id: &ObjId, readonly: bool) -> NdnResult<Box<dyn ObjectArrayInnerCache>> {
        match self.storage_type {
            ObjectArrayStorageType::Arrow => {
                let file_path = self.data_path.join(format!("{}.arrow", id.to_base32()));
                let reader = ObjectArrayArrowReader::open(&file_path, readonly).await?;
                Ok(reader.into_cache())
            }
            ObjectArrayStorageType::SQLite => {
                unimplemented!("SQLite storage is not implemented yet");
            }
            ObjectArrayStorageType::SimpleFile => {
                unimplemented!("Simple file storage is not implemented yet");
            }
        }
    }
}
