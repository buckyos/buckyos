use crate::object::{build_named_object_by_json, ObjId};
use crate::NdnResult;
use crate::ObjectArrayOwnedIter;
use crate::{
    ChunkId, ChunkIdRef, HashMethod, OBJ_TYPE_CHUNK_LIST, OBJ_TYPE_CHUNK_LIST_FIX_SIZE,
    OBJ_TYPE_CHUNK_LIST_SIMPLE, OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE,
};
use crate::{ObjectArray, ObjectArrayBody};
use core::hash;
use serde::{Deserialize, Serialize};
use std::io::SeekFrom;
use std::ops::{Deref, DerefMut};

pub const CHUNK_LIST_MODE_THRESHOLD: usize = 1024; // Threshold for chunk list normal and simple mode

pub struct ChunkListId {}

impl ChunkListId {
    pub fn is_chunk_list(obj_id: &ObjId) -> bool {
        obj_id.obj_type == OBJ_TYPE_CHUNK_LIST
            || obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_FIX_SIZE
            || obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_SIMPLE
            || obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE
    }

    pub fn is_simple_chunk_list(obj_id: &ObjId) -> bool {
        obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_SIMPLE
            || obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE
    }

    pub fn is_normal_chunk_list(obj_id: &ObjId) -> bool {
        obj_id.obj_type == OBJ_TYPE_CHUNK_LIST || obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_FIX_SIZE
    }

    pub fn is_fixed_size_chunk_list(obj_id: &ObjId) -> bool {
        obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_FIX_SIZE
            || obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE
    }

    pub fn is_variable_size_chunk_list(obj_id: &ObjId) -> bool {
        obj_id.obj_type == OBJ_TYPE_CHUNK_LIST || obj_id.obj_type == OBJ_TYPE_CHUNK_LIST_SIMPLE
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkListMeta {
    pub total_size: u64,       // Total size of the chunk list
    pub fix_size: Option<u64>, // Fixed size of each chunk, if None, it is a var chunk size list
}

// Use to calculate the ChunkListId
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkListBody {
    pub object_array: ObjectArrayBody,
    pub total_count: u64, // Total number of chunks in the chunk list
    pub total_size: u64,  // Total size of the chunk list
    pub fix_size: Option<u64>,
}

impl ChunkListBody {
    pub fn is_simple_chunk_list(&self) -> bool {
        if self.total_count <= CHUNK_LIST_MODE_THRESHOLD as u64 {
            true
        } else {
            false
        }
    }

    pub fn is_fixed_size_chunk_list(&self) -> bool {
        if self.fix_size.is_some() {
            true
        } else {
            false
        }
    }

    pub fn get_chunk_list_type(&self) -> &str {
        if self.is_simple_chunk_list() {
            if self.is_fixed_size_chunk_list() {
                OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE
            } else {
                OBJ_TYPE_CHUNK_LIST_SIMPLE
            }
        } else {
            if self.is_fixed_size_chunk_list() {
                OBJ_TYPE_CHUNK_LIST_FIX_SIZE
            } else {
                OBJ_TYPE_CHUNK_LIST
            }
        }
    }

    pub fn calc_obj_id(&self) -> (ObjId, String) {
        let chunk_list_type = self.get_chunk_list_type();
        let (obj_id, s) = build_named_object_by_json(
            chunk_list_type,
            &serde_json::to_value(self).expect("Failed to serialize ChunkListBody"),
        );

        // Return the object id and its string representation
        (obj_id, s)
    }
}

pub struct ChunkList {
    meta: ChunkListMeta,
    chunk_list_imp: ObjectArray,
    obj_id: ObjId,
}

impl Deref for ChunkList {
    type Target = ObjectArray;
    fn deref(&self) -> &Self::Target {
        &self.chunk_list_imp
    }
}

impl DerefMut for ChunkList {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.chunk_list_imp
    }
}

impl ChunkList {
    pub fn new(meta: ChunkListMeta, list: ObjectArray) -> NdnResult<Self> {
        let body = ChunkListBody {
            object_array: list.get_body().ok_or_else(|| {
                let msg = "Object array body is None".to_string();
                error!("{}", msg);
                crate::NdnError::InvalidData(msg)
            })?,
            total_count: list.len() as u64,
            total_size: meta.total_size,
            fix_size: meta.fix_size,
        };

        let (obj_id, _) = body.calc_obj_id();

        let ret = Self {
            meta,
            chunk_list_imp: list,
            obj_id,
        };

        Ok(ret)
    }

    // Load an existing chunk list from the object id.
    pub async fn open(obj_data: serde_json::Value) -> NdnResult<Self> {
        let body: ChunkListBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Failed to parse chunk list body: {}", e);
            error!("{}", msg);
            crate::NdnError::InvalidData(msg)
        })?;

        let (obj_id, _) = body.calc_obj_id();

        let obj_array_body = serde_json::to_value(&body.object_array).map_err(|e| {
            let msg = format!("Failed to serialize object array body: {}", e);
            error!("{}", msg);
            crate::NdnError::InvalidData(msg)
        })?;

        let chunk_list_imp = ObjectArray::open(obj_array_body, true).await?;
        let meta = ChunkListMeta {
            total_size: body.total_size,
            fix_size: body.fix_size,
        };

        Ok(Self {
            meta,
            chunk_list_imp,
            obj_id,
        })
    }

    pub fn clone(&self, read_only: bool) -> NdnResult<Self> {
        let ret = Self {
            meta: self.meta.clone(),
            chunk_list_imp: self.chunk_list_imp.clone(read_only)?,
            obj_id: self.obj_id.clone(),
        };

        Ok(ret)
    }

    pub fn into_parts(self) -> (ChunkListMeta, ObjectArray) {
        (self.meta, self.chunk_list_imp)
    }

    // ChunkList always has an obj_id, so we can use get_obj_id() to get it.
    pub fn get_obj_id(&self) -> &ObjId {
        &self.obj_id
    }

    // The obj_id should be flush and calculate when loaded or built.
    pub fn calc_obj_id(&self) -> (ObjId, String) {
        let object_array = self.chunk_list_imp.get_body();
        if object_array.is_none() {
            unreachable!("Object array body should not be None");
        }

        let body = ChunkListBody {
            object_array: object_array.unwrap(),
            total_count: self.chunk_list_imp.len() as u64,
            total_size: self.meta.total_size,
            fix_size: self.meta.fix_size,
        };

        let (obj_id, s) = body.calc_obj_id();
        (obj_id, s)
    }

    pub fn get_hash_method(&self) -> HashMethod {
        self.chunk_list_imp.hash_method()
    }

    pub fn get_meta(&self) -> &ChunkListMeta {
        &self.meta
    }

    // Return the total number of chunks in the chunk list
    pub fn get_len(&self) -> usize {
        self.chunk_list_imp.len()
    }

    // Return the total size of the chunk list
    pub fn get_total_size(&self) -> u64 {
        self.meta.total_size
    }

    pub fn iter(&self) -> ChunkListIter<'_> {
        ChunkListIter::new(self)
    }

    pub fn is_chunk_list(obj_id: &ObjId) -> bool {
        if obj_id.obj_type == OBJ_TYPE_CHUNK_LIST {
            true
        } else {
            false
        }
    }

    pub fn is_simple_chunk_list(&self) -> bool {
        let len = self.chunk_list_imp.len();
        if len <= CHUNK_LIST_MODE_THRESHOLD {
            true
        } else {
            false
        }
    }

    pub fn is_fixed_size_chunk_list(&self) -> bool {
        if self.meta.fix_size.is_some() {
            true
        } else {
            false
        }
    }

    pub fn get_chunk(&self, index: usize) -> NdnResult<Option<ChunkId>> {
        let obj_id = self.chunk_list_imp.get_object(index)?;
        if let Some(obj_id) = obj_id {
            Ok(Some(ChunkId::from(obj_id)))
        } else {
            Ok(None)
        }
    }

    //return (chunk_index,chunk_offset)
    pub fn get_chunk_index_by_offset(&self, offset: SeekFrom) -> NdnResult<(u64, u64)> {
        match offset {
            SeekFrom::Start(pos) => {
                match self.meta.fix_size {
                    Some(fix_size) => {
                        if fix_size == 0 {
                            let msg = format!("Fixed size cannot be zero");
                            error!("{}", msg);
                            return Err(crate::NdnError::InvalidData(msg));
                        }

                        let total_chunks = self.chunk_list_imp.len() as u64;
                        let chunk_index = pos / fix_size;
                        let chunk_offset = pos % fix_size;

                        if chunk_index >= total_chunks {
                            let msg = format!(
                                "Chunk index {} exceeds total chunks {}, {}",
                                chunk_index,
                                total_chunks,
                                self.get_obj_id(),
                            );
                            warn!("{}", msg);
                            return Err(crate::NdnError::OffsetTooLarge(msg));
                        }

                        // FIXME : Check if chunk_offset is valid for the given chunk?
                        if chunk_offset >= fix_size {
                            let msg = format!(
                                "Chunk offset {} exceeds fixed size {}, {}",
                                chunk_offset,
                                fix_size,
                                self.get_obj_id(),
                            );
                            warn!("{}", msg);
                            return Err(crate::NdnError::OffsetTooLarge(msg));
                        }

                        Ok((chunk_index, chunk_offset))
                    }
                    None => {
                        // Variable size chunks, need to calculate based on the chunk list
                        let mut total_size = 0;
                        for (index, obj_id) in self.chunk_list_imp.iter().enumerate() {
                            // Check if we just at the beginning of current chunk
                            if total_size == pos {
                                return Ok((index as u64, 0));
                            }

                            let chunk_id = ChunkIdRef::from_obj_id(&obj_id);
                            let length = chunk_id.get_length().ok_or_else(|| {
                                let msg = format!("Failed to get length for chunk id: {}", obj_id);
                                error!("{}", msg);
                                crate::NdnError::InvalidData(msg)
                            })?;

                            total_size += length;
                            if total_size > pos {
                                let chunk_offset = pos - (total_size - length);
                                return Ok((index as u64, chunk_offset));
                            }
                        }

                        let msg = format!(
                            "Offset {} exceeds total size of chunk list {}, {}",
                            pos,
                            total_size,
                            self.get_obj_id(),
                        );
                        warn!("{}", msg);
                        Err(crate::NdnError::OffsetTooLarge(msg))
                    }
                }
            }
            SeekFrom::End(offset) => {
                match self.meta.fix_size {
                    Some(fix_size) => {
                        if fix_size == 0 {
                            let msg = format!("Fixed size cannot be zero");
                            error!("{}", msg);
                            return Err(crate::NdnError::InvalidData(msg));
                        }

                        let total_chunks = self.chunk_list_imp.len() as u64;
                        let total_size = fix_size.checked_mul(total_chunks).ok_or_else(|| {
                            let msg = format!(
                                "Total size overflow for chunk list: {}",
                                self.get_obj_id()
                            );
                            error!("{}", msg);
                            crate::NdnError::OffsetTooLarge(msg)
                        })?;

                        let absolute_pos = total_size as i64 + offset;
                        if absolute_pos < 0 {
                            let msg = format!(
                                "Offset {} from end is negative, cannot seek: {}",
                                offset,
                                self.get_obj_id()
                            );
                            error!("{}", msg);
                            return Err(crate::NdnError::OffsetTooLarge(msg));
                        }
                        let absolute_pos = absolute_pos as u64;
                        if absolute_pos >= total_size {
                            let msg = format!(
                                "Absolute position {} exceeds total size {}, {}",
                                absolute_pos,
                                total_size,
                                self.get_obj_id(),
                            );
                            warn!("{}", msg);
                            return Err(crate::NdnError::OffsetTooLarge(msg));
                        }

                        // Special case: when absolute_pos equals total_size
                        if absolute_pos == total_size && absolute_pos != 0 {
                            // When absolute_pos is at the end of the file, it points to the end of the last chunk
                            return Ok((total_chunks - 1, fix_size));
                        }

                        // Chunk index is the absolute position divided by chunk size
                        let chunk_index = absolute_pos / fix_size;
                        let chunk_offset = absolute_pos % fix_size;

                        Ok((chunk_index, chunk_offset))
                    }
                    None => {
                        // Variable size chunks, need to calculate based on the chunk list
                        let total_size = self.meta.total_size;
                        if total_size == 0 {
                            return Err(crate::NdnError::OffsetTooLarge(
                                "Chunk list is empty".to_string(),
                            ));
                        }

                        if offset >= 0 {
                            let msg = format!(
                                "Offset {} from end is not negative, cannot seek: {}",
                                offset,
                                self.get_obj_id()
                            );
                            error!("{}", msg);
                            return Err(crate::NdnError::OffsetTooLarge(msg));
                        }

                        // Convert to u64 for calculations
                        let offset = offset.abs() as u64;
                        if offset > total_size {
                            let msg = format!(
                                "Offset {} from end exceeds total size {}, {}",
                                offset,
                                total_size,
                                self.get_obj_id(),
                            );
                            warn!("{}", msg);
                            return Err(crate::NdnError::OffsetTooLarge(msg));
                        }

                        // When total_size is none zero, we must have at least one chunk
                        let total_chunks = self.chunk_list_imp.len();
                        assert!(total_chunks > 0, "Chunk list should not be empty");

                        let mut total_size = 0;
                        for (index, obj_id) in self.chunk_list_imp.iter().rev().enumerate() {
                            let chunk_id = ChunkIdRef::from_obj_id(&obj_id);
                            let length = chunk_id.get_length().ok_or_else(|| {
                                let msg = format!("Failed to get length for chunk id: {}", obj_id);
                                error!("{}", msg);
                                crate::NdnError::InvalidData(msg)
                            })?;

                            total_size += length;
                            if total_size >= offset {
                                let chunk_index = total_chunks - index - 1; // Reverse index
                                let chunk_offset = total_size - offset;
                                if chunk_offset >= length {
                                    let msg = format!(
                                        "Chunk offset {} exceeds chunk length {}, {}",
                                        chunk_offset,
                                        length,
                                        self.get_obj_id(),
                                    );
                                    warn!("{}", msg);
                                    return Err(crate::NdnError::OffsetTooLarge(msg));
                                }

                                return Ok((chunk_index as u64, chunk_offset));
                            }
                        }

                        let msg = format!(
                            "Offset {} from end exceeds total size of chunk list {}, {}",
                            offset,
                            total_size,
                            self.get_obj_id(),
                        );
                        warn!("{}", msg);
                        Err(crate::NdnError::OffsetTooLarge(msg))
                    }
                }
            }
            SeekFrom::Current(_) => {
                // Handle current offset if needed
                Err(crate::NdnError::Unsupported(
                    "Current offset not supported".to_string(),
                ))
            }
        }
    }

    pub fn get_chunk_offset_by_index(&self, index: u64) -> NdnResult<u64> {
        if index >= self.chunk_list_imp.len() as u64 {
            let msg = format!(
                "Chunk index {} exceeds total chunks {}, {}",
                index,
                self.chunk_list_imp.len(),
                self.get_obj_id(),
            );
            warn!("{}", msg);
            return Err(crate::NdnError::OffsetTooLarge(msg));
        }

        match self.meta.fix_size {
            Some(fix_size) => {
                if fix_size == 0 {
                    let msg = format!("Fixed size cannot be zero");
                    error!("{}", msg);
                    return Err(crate::NdnError::InvalidData(msg));
                }

                index.checked_mul(fix_size).ok_or_else(|| {
                    let msg = format!(
                        "Index {} multiplied by fixed size {} overflow, {}",
                        index,
                        fix_size,
                        self.get_obj_id(),
                    );
                    error!("{}", msg);
                    crate::NdnError::OffsetTooLarge(msg)
                })
            }
            None => {
                // Variable size chunks, need to calculate based on the chunk list
                let mut total_size: u64 = 0;
                for (i, obj_id) in self.chunk_list_imp.iter().enumerate() {
                    if i as u64 == index {
                        return Ok(total_size);
                    }

                    let chunk_id = ChunkIdRef::from_obj_id(&obj_id);
                    let length = chunk_id.get_length().ok_or_else(|| {
                        let msg = format!("Failed to get length for chunk id: {}", obj_id);
                        error!("{}", msg);
                        crate::NdnError::InvalidData(msg)
                    })?;

                    total_size = total_size.checked_add(length).ok_or_else(|| {
                        let msg = format!(
                            "Total size overflow when calculating chunk offset for index {}, {}",
                            index,
                            self.get_obj_id(),
                        );
                        error!("{}", msg);
                        crate::NdnError::OffsetTooLarge(msg)
                    })?;
                }

                let msg = format!(
                    "Chunk index {} exceeds total chunks {}, {}",
                    index,
                    self.chunk_list_imp.len(),
                    self.get_obj_id(),
                );
                warn!("{}", msg);
                Err(crate::NdnError::OffsetTooLarge(msg))
            }
        }
    }
}

pub struct ChunkListIter<'a> {
    iter: crate::ObjectArrayIter<'a>,
}

impl<'a> ChunkListIter<'a> {
    pub fn new(chunk_list: &'a ChunkList) -> Self {
        Self {
            iter: chunk_list.chunk_list_imp.iter(),
        }
    }
}

impl<'a> Iterator for ChunkListIter<'a> {
    type Item = ChunkId;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|obj_id| {
            obj_id.into() // Convert ObjId to ChunkId
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a> DoubleEndedIterator for ChunkListIter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iter.next_back().map(|obj_id| {
            obj_id.into() // Convert ObjId to ChunkId
        })
    }
}

impl<'a> ExactSizeIterator for ChunkListIter<'a> {}

pub struct ChunkListOwnedIter {
    iter: ObjectArrayOwnedIter,
}

impl ChunkListOwnedIter {
    pub fn new(chunk_list: ChunkList) -> Self {
        Self {
            iter: chunk_list.chunk_list_imp.into_iter(),
        }
    }
}

impl Iterator for ChunkListOwnedIter {
    type Item = ChunkId;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|obj_id| {
            obj_id.into() // Convert ObjId to ChunkId
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl ExactSizeIterator for ChunkListOwnedIter {}

impl DoubleEndedIterator for ChunkListOwnedIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iter.next_back().map(|obj_id| {
            obj_id.into() // Convert ObjId to ChunkId
        })
    }
}

impl IntoIterator for ChunkList {
    type Item = ChunkId;
    type IntoIter = ChunkListOwnedIter;

    fn into_iter(self) -> Self::IntoIter {
        ChunkListOwnedIter::new(self)
    }
}
