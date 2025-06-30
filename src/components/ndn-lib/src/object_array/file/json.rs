use std::path::{PathBuf, Path};
use serde::{Serialize, Deserialize};
use super::super::memory_cache::ObjectArrayMemoryCache;
use super::super::storage::{ObjectArrayStorageWriter, ObjectArrayInnerCache};
use crate::{ObjId, NdnResult, NdnError};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObjectArrayJSONData {
    data: Vec<ObjId>,
}

pub struct ObjectArrayJSONWriter {
    file_path: PathBuf,

    data: ObjectArrayJSONData,
}

impl ObjectArrayJSONWriter {
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            data: ObjectArrayJSONData {
                data: Vec::new(),
            },
        }
    }
}


#[async_trait::async_trait]
impl ObjectArrayStorageWriter for ObjectArrayJSONWriter {
    async fn file_path(&self) -> NdnResult<PathBuf> {
        Ok(self.file_path.clone())
    }

    async fn append(&mut self, value: &ObjId) -> NdnResult<()> {
        self.data.data.push(value.clone());
        Ok(())
    }

    async fn len(&self) -> NdnResult<usize> {
        Ok(self.data.data.len())
    }

    async fn flush(&mut self) -> NdnResult<()> {
        let file = std::fs::File::create(&self.file_path).map_err(|e| {
            let msg = format!("Failed to create file: {:?}, {}", self.file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        // Serialize the data to JSON and write it to the file
        serde_json::to_writer(file, &self.data).map_err(|e| {
            let msg = format!("Failed to write data to file: {:?}, {}", self.file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        Ok(())
    }
}

pub struct ObjectArrayJSONReader {
    cache: Box<dyn ObjectArrayInnerCache>,
}

impl ObjectArrayJSONReader {
    pub fn new(cache: Box<dyn ObjectArrayInnerCache>) -> Self {
        Self { cache }
    }

    pub fn into_cache(self) -> Box<dyn ObjectArrayInnerCache> {
        self.cache
    }

    pub async fn open(file_path: &Path, readonly: bool) -> NdnResult<Self> {
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
        let data: ObjectArrayJSONData = serde_json::from_reader(file).map_err(|e| {
            let msg = format!("Failed to read data from file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let cache = ObjectArrayMemoryCache::new_array(data.data);
        let ret = ObjectArrayJSONReader::new(Box::new(cache));

        Ok(ret)
    }
}
