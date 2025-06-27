use crate::{NdnError, NdnResult, HashMethod, ObjId};

// Because the object may be ObjectId or ChunkId, which maybe have mix mode, so we need to check the hash length
pub fn get_obj_hash<'a>(obj_id: &'a ObjId, hash_method: HashMethod) -> NdnResult<&'a [u8]> {
    if obj_id.obj_hash.len() < hash_method.hash_bytes() {
        let msg = format!(
            "Object hash length does not match hash method: {}",
            obj_id.obj_hash.len()
        );
        error!("{}", msg);
        return Err(NdnError::InvalidData(msg));
    }

    // We use the last hash bytes as the object hash
    if obj_id.obj_hash.len() > hash_method.hash_bytes() {
        // obj_id is a chunk id with mix mode, we need to get the last hash bytes
        // FIXME: Should we check if the hash type is valid?
        let start = obj_id.obj_hash.len() - hash_method.hash_bytes();
        return Ok(&obj_id.obj_hash[start..]);
    } else {
        // If the hash length is equal, we can return the whole hash
        return Ok(&obj_id.obj_hash);
    }
}
