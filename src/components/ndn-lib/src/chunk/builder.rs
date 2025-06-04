use super::chunk::ChunkId;
use super::chunk_list::{ChunkList, ChunkListBody, ChunkListMeta, CHUNK_LIST_MODE_THRESHOLD};
use crate::hash::HashMethod;
use crate::object_array::{ObjectArray, ObjectArrayStorageType};
use crate::NdnResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkListMode {
    Simple, // Simple mode, used for small chunk lists
    Normal, // Normal mode, used for larger chunk lists
}

pub struct ChunkListBuilder {
    meta: ChunkListMeta,
    list: ObjectArray,
}

impl ChunkListBuilder {
    pub fn new(hash_method: HashMethod, count: Option<usize>) -> Self {
        let mode = Self::select_mode(count);
        let storage_type = match mode {
            ChunkListMode::Simple => crate::ObjectArrayStorageType::JSONFile,
            ChunkListMode::Normal => crate::ObjectArrayStorageType::Arrow,
        };

        Self {
            meta: ChunkListMeta {
                total_size: 0,
                fix_size: None,
            },
            list: ObjectArray::new(hash_method, Some(storage_type)),
        }
    }

    pub async fn open(obj_data: serde_json::Value) -> NdnResult<Self> {
        let body: ChunkListBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Failed to parse chunk list body: {}", e);
            error!("{}", msg);
            crate::NdnError::InvalidData(msg)
        })?;

        let chunk_list_imp = ObjectArray::open(&body.object_array, false).await?;
        let meta = ChunkListMeta {
            total_size: body.total_size,
            fix_size: body.fix_size,
        };

        let ret = ChunkListBuilder {
            meta,
            list: chunk_list_imp,
        };

        Ok(ret)
    }

    pub fn from_chunk_list(chunk_list: &ChunkList) -> NdnResult<Self> {
        let chunk_list = chunk_list.clone(false)?; // Clone in read-write mode
        let (meta, chunk_list) = chunk_list.into_parts();

        let ret = Self {
            meta,
            list: chunk_list,
        };

        Ok(ret)
    }

    pub fn from_chunk_list_owned(chunk_list: ChunkList) -> NdnResult<Self> {
        let (meta, chunk_list) = chunk_list.into_parts();
        let list = if chunk_list.is_readonly() {
            chunk_list.clone(false)?
        } else {
            chunk_list
        };

        let ret = Self { meta, list };

        Ok(ret)
    }

    pub fn select_mode(count: Option<usize>) -> ChunkListMode {
        if let Some(c) = count {
            if c <= CHUNK_LIST_MODE_THRESHOLD {
                ChunkListMode::Simple
            } else {
                ChunkListMode::Normal
            }
        } else {
            ChunkListMode::Normal // Default to Normal if count is None
        }
    }

    pub fn with_total_size(mut self, size: u64) -> Self {
        self.meta.total_size = size;
        self
    }

    pub fn with_fixed_size(mut self, size: u64) -> Self {
        self.meta.fix_size = Some(size);
        self
    }

    pub fn with_var_size(mut self) -> Self {
        self.meta.fix_size = None;
        self
    }

    // Just append the chunk id to the list, no need to increment total size.
    pub fn append(&mut self, chunk_id: ChunkId) -> NdnResult<()> {
        let obj_id = chunk_id.into();
        self.list.append_object(&obj_id)?;

        Ok(())
    }

    // Just insert the chunk id at the specified index, no need to increment total size.
    pub fn insert(&mut self, index: usize, chunk_id: ChunkId) -> NdnResult<()> {
        let obj_id = chunk_id.into();
        self.list.insert_object(index, &obj_id)?;

        Ok(())
    }

    // Append a chunk with its size, and update the total size.
    pub fn append_with_size(&mut self, chunk_id: ChunkId, size: u64) -> NdnResult<()> {
        let obj_id = chunk_id.into();
        self.list.append_object(&obj_id)?;

        // Update total size
        self.meta.total_size += size;

        Ok(())
    }

    // Insert a chunk with its size at the specified index, and update the total size.
    pub fn insert_with_size(
        &mut self,
        index: usize,
        chunk_id: ChunkId,
        size: u64,
    ) -> NdnResult<()> {
        let obj_id = chunk_id.into();
        self.list.insert_object(index, &obj_id)?;

        // Update total size
        self.meta.total_size += size;

        Ok(())
    }

    pub async fn build(mut self) -> NdnResult<ChunkList> {
        let meta_str = serde_json::to_string(&self.meta).map_err(|e| {
            let msg = format!("Failed to serialize chunk list meta: {}", e);
            error!("{}", msg);
            crate::NdnError::InvalidData(msg)
        })?;

        self.list.set_meta(Some(meta_str));

        // First, flush the list to ensure the mtree is built and the object id is calculated.
        self.list.flush().await?;

        // Ensure the object mode is set correctly
        let len = self.list.len();
        match Self::select_mode(Some(len)) {
            ChunkListMode::Simple => {
                self.list
                    .change_storage_type(crate::ObjectArrayStorageType::JSONFile);
            }
            ChunkListMode::Normal => {
                self.list
                    .change_storage_type(crate::ObjectArrayStorageType::Arrow);
            }
        }

        // Save
        self.list.save().await?;

        let ret = ChunkList::new(self.meta, self.list);

        Ok(ret)
    }
}
