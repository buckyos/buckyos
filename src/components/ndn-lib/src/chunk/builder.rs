use super::chunk::ChunkId;
use super::chunk_list::{ChunkList, ChunkListBody, ChunkListMeta};
use crate::hash::HashMethod;
use crate::object_array::{ObjectArray, ObjectArrayStorageType};
use crate::NdnResult;
use crate::coll::CollectionStorageMode;


pub struct ChunkListBuilder {
    meta: ChunkListMeta,
    list: ObjectArray,
}

impl ChunkListBuilder {
    pub fn new(hash_method: HashMethod, count: Option<usize>) -> Self {
        let mode = CollectionStorageMode::select_mode(count.map(|c| c as u64));
        let storage_type = match mode {
            CollectionStorageMode::Simple => crate::ObjectArrayStorageType::JSONFile,
            CollectionStorageMode::Normal => crate::ObjectArrayStorageType::Arrow,
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

        let obj_array = serde_json::to_value(&body.object_array).map_err(|e| {
            let msg = format!("Failed to serialize object array: {}", e);
            error!("{}", msg);
            crate::NdnError::InvalidData(msg)
        })?;

        let chunk_list_imp = ObjectArray::open(obj_array, false).await?;
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
        // First, flush the list to ensure the mtree is built and the object id is calculated.
        self.list.flush_mtree().await?;

        // Ensure the object mode is set correctly
        let len = self.list.len();
        match CollectionStorageMode::select_mode(Some(len as u64)) {
            CollectionStorageMode::Simple => {
                self.list
                    .change_storage_type(crate::ObjectArrayStorageType::JSONFile);
            }
            CollectionStorageMode::Normal => {
                self.list
                    .change_storage_type(crate::ObjectArrayStorageType::Arrow);
            }
        }

        // Save
        self.list.save().await?;

        let ret = ChunkList::new(self.meta, self.list)?;

        Ok(ret)
    }
}
