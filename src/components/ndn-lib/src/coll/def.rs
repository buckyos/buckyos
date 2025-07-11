use crate::{HashMethod, NdnError, NdnResult, ObjId};

// Because the object may be ObjectId or ChunkId, which maybe have mix mode, so we need to check the hash length
pub fn get_obj_hash<'a>(obj_id: &'a ObjId, hash_method: HashMethod) -> NdnResult<&'a [u8]> {
    if obj_id.obj_hash.len() < hash_method.hash_result_size() {
        let msg = format!(
            "Object hash length does not match hash method: {}",
            obj_id.obj_hash.len()
        );
        error!("{}", msg);
        return Err(NdnError::InvalidData(msg));
    }

    // We use the last hash bytes as the object hash
    if obj_id.obj_hash.len() > hash_method.hash_result_size() {
        // obj_id is a chunk id with mix mode, we need to get the last hash bytes
        // FIXME: Should we check if the hash type is valid?
        let start = obj_id.obj_hash.len() - hash_method.hash_result_size();
        return Ok(&obj_id.obj_hash[start..]);
    } else {
        // If the hash length is equal, we can return the whole hash
        return Ok(&obj_id.obj_hash);
    }
}

pub const COLLECTION_STORAGE_MODE_THRESHOLD: u64 = 1024; // Threshold for collection normal and simple mode

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionStorageMode {
    // Simple mode, used for small collections, always use JSON file storage
    Simple,

    // Normal mode, used for larger collections, always use Arrow/SQLITE storage
    Normal,
}

impl CollectionStorageMode {
    pub fn select_mode(count: Option<u64>) -> Self {
        match count {
            Some(c) if c <= COLLECTION_STORAGE_MODE_THRESHOLD => Self::Simple,
            _ => Self::Normal,
        }
    }

    pub fn is_simple(count: u64) -> bool {
        count <= COLLECTION_STORAGE_MODE_THRESHOLD
    }

    pub fn is_normal(count: u64) -> bool {
        !Self::is_simple(count)
    }
}
