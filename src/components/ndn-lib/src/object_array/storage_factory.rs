use std::path::{PathBuf, Path};
use crate::{NdnResult, ObjId, NdnError};
use super::storage::{
    ObjectArrayStorageWriter,
    ObjectArrayStorageReader,
    ObjectArrayStorageType,
};
use super::file::*;

pub struct ObjectArrayStorage {
    data_path: PathBuf,
    storage_type: ObjectArrayStorageType,
}

impl ObjectArrayStorage {
    pub fn new(data_path: &Path, storage_type: Option<ObjectArrayStorageType>) -> Self {
        Self {
            data_path: data_path.to_path_buf(),
            storage_type: storage_type.unwrap_or(ObjectArrayStorageType::default()),
        }
    }

    pub async fn open_writer(&self, id: &ObjId, len: Option<usize>) -> NdnResult<Box<dyn ObjectArrayStorageWriter>> {
        // First make sure the directory exists
        if !self.data_path.exists() {
            std::fs::create_dir_all(&self.data_path).map_err(|e| {
                let msg = format!("Error creating directory {}: {}", self.data_path.display(), e);
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

    pub async fn open_reader(&self, id: &ObjId) -> NdnResult<Box<dyn ObjectArrayStorageReader>> {
        match self.storage_type {
            ObjectArrayStorageType::Arrow => {
                let file_path = self.data_path.join(format!("{}.arrow", id.to_base32()));
                let reader = ObjectArrayArrowReader::open(&file_path).await?;
                Ok(Box::new(reader))
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