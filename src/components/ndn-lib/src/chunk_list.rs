use crate::NdnResult;
use crate::ObjectArray;
use crate::{ChunkId, ChunkIdRef, HashMethod, ObjId, OBJ_TYPE_CHUNK_LIST};
use core::hash;
use serde::{Deserialize, Serialize};
use std::io::SeekFrom;
use std::ops::{Deref, DerefMut};

pub const CHUNK_LIST_MODE_THRESHOLD: usize = 1024; // Threshold for chunk list normal and simple mode

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkListMeta {
    total_size: u64,       // Total size of the chunk list
    fix_size: Option<u64>, // Fixed size of each chunk, if None, it is a var chunk size list
}

pub struct ChunkList {
    meta: ChunkListMeta,
    chunk_list_imp: ObjectArray,
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
    pub fn new() -> Self {
        unimplemented!()
    }

    pub fn new_by_obj_data(obj_data: serde_json::Value) -> Self {
        unimplemented!()
    }

    // The objid should be flush and calculate when loaded or built.
    pub fn get_chunk_list_id(&self) -> ObjId {
        let id = self.chunk_list_imp.get_obj_id().unwrap();
        let chunk_list_id = ObjId {
            obj_type: OBJ_TYPE_CHUNK_LIST.to_owned(),
            obj_hash: id.obj_hash,
        };

        chunk_list_id
    }

    pub fn is_chunklist(obj_id: &ObjId) -> bool {
        unimplemented!()
    }

    pub fn is_simple_chunklist(&self) -> bool {
        let len = self.chunk_list_imp.len();
        if len <= CHUNK_LIST_MODE_THRESHOLD {
            true
        } else {
            false
        }
    }

    pub fn is_fixed_size_chunklist(&self) -> bool {
        if self.meta.fix_size.is_some() {
            true
        } else {
            false
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
                                self.get_chunk_list_id(),
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
                                self.get_chunk_list_id(),
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
                            self.get_chunk_list_id(),
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
                                self.get_chunk_list_id()
                            );
                            error!("{}", msg);
                            crate::NdnError::OffsetTooLarge(msg)
                        })?;

                        let absolute_pos = total_size as i64 + offset;
                        if absolute_pos < 0 {
                            let msg = format!(
                                "Offset {} from end is negative, cannot seek: {}",
                                offset,
                                self.get_chunk_list_id()
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
                                self.get_chunk_list_id(),
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
                                self.get_chunk_list_id()
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
                                self.get_chunk_list_id(),
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
                            if total_size > offset {
                                let chunk_index = total_chunks - index - 1; // Reverse index
                                let chunk_offset = length - (total_size - offset);
                                if chunk_offset >= length {
                                    let msg = format!(
                                        "Chunk offset {} exceeds chunk length {}, {}",
                                        chunk_offset,
                                        length,
                                        self.get_chunk_list_id(),
                                    );
                                    warn!("{}", msg);
                                    return Err(crate::NdnError::OffsetTooLarge(msg));
                                }

                                return Ok((index as u64, chunk_offset));
                            }
                        }

                        let msg = format!(
                            "Offset {} from end exceeds total size of chunk list {}, {}",
                            offset,
                            total_size,
                            self.get_chunk_list_id(),
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
                self.get_chunk_list_id(),
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
                        self.get_chunk_list_id(),
                    );
                    error!("{}", msg);
                    crate::NdnError::OffsetTooLarge(msg)
                })
            }
            None => {
                // Variable size chunks, need to calculate based on the chunk list
                let mut total_size: u64 = 0;
                for (i, obj_id) in self.chunk_list_imp.iter().enumerate() {
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
                            self.get_chunk_list_id(),
                        );
                        error!("{}", msg);
                        crate::NdnError::OffsetTooLarge(msg)
                    })?;

                    if i as u64 == index {
                        return Ok(total_size);
                    }
                }

                let msg = format!(
                    "Chunk index {} exceeds total chunks {}, {}",
                    index,
                    self.chunk_list_imp.len(),
                    self.get_chunk_list_id(),
                );
                warn!("{}", msg);
                Err(crate::NdnError::OffsetTooLarge(msg))
            }
        }
    }

    pub fn get_total_size(&self) -> u64 {
        self.meta.total_size
    }
}
