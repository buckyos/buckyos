use super::chunk::ChunkId;
use super::chunk_list::{ChunkList, ChunkListBody, ChunkListMeta};
use crate::coll::CollectionStorageMode;
use crate::hash::HashMethod;
use crate::object_array::{ObjectArray, ObjectArrayBuilder, ObjectArrayStorageType};
use crate::{object, NdnResult};

pub struct ChunkListBuilder {
    meta: ChunkListMeta,
    array_builder: ObjectArrayBuilder,
}

impl ChunkListBuilder {
    pub fn new(hash_method: HashMethod) -> Self {
        Self {
            meta: ChunkListMeta {
                total_size: 0,
                fix_size: None,
            },
            array_builder: ObjectArrayBuilder::new(hash_method),
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

        let object_array_builder = ObjectArrayBuilder::open(obj_array).await?;
        let meta = ChunkListMeta {
            total_size: body.total_size,
            fix_size: body.fix_size,
        };

        let ret = ChunkListBuilder {
            meta,
            array_builder: object_array_builder,
        };

        Ok(ret)
    }

    pub fn from_chunk_list(chunk_list: &ChunkList) -> NdnResult<Self> {
        let chunk_list = chunk_list.clone_for_modify()?; // Clone in read-write mode
        let (body, object_array) = chunk_list.into_parts();
        let objet_array_builder = ObjectArrayBuilder::from_object_array_owned(object_array);

        let ret = Self {
            meta: body.meta(),
            array_builder: objet_array_builder,
        };

        Ok(ret)
    }

    pub fn from_chunk_list_owned(chunk_list: ChunkList) -> NdnResult<Self> {
        let (body, object_array) = chunk_list.into_parts();
        let object_array_builder = ObjectArrayBuilder::from_object_array_owned(object_array);

        let ret = Self {
            meta: body.meta(),
            array_builder: object_array_builder,
        };

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
        self.array_builder.append_object(&obj_id)?;

        Ok(())
    }

    // Just insert the chunk id at the specified index, no need to increment total size.
    pub fn insert(&mut self, index: usize, chunk_id: ChunkId) -> NdnResult<()> {
        let obj_id = chunk_id.into();
        self.array_builder.insert_object(index, &obj_id)?;

        Ok(())
    }

    // Append a chunk with its size, and update the total size.
    pub fn append_with_size(&mut self, chunk_id: ChunkId, size: u64) -> NdnResult<()> {
        let obj_id = chunk_id.into();
        self.array_builder.append_object(&obj_id)?;

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
        self.array_builder.insert_object(index, &obj_id)?;

        // Update total size
        self.meta.total_size += size;

        Ok(())
    }

    pub async fn build(mut self) -> NdnResult<ChunkList> {
        // First, flush the list to ensure the mtree is built and the object id is calculated.
        let object_array = self.array_builder.build().await?;

        let ret = ChunkList::new(self.meta, object_array)?;

        Ok(ret)
    }
}
