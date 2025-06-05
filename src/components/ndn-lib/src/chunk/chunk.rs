use crate::hash::*;
use crate::{object::ObjId, NdnError, NdnResult};
use async_trait::async_trait;
use base58::{FromBase58, ToBase58};
use crypto_common::hazmat::{SerializableState, SerializedState};
use hex;
use log::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use std::{future::Future, io::SeekFrom, ops::Range, path::PathBuf, pin::Pin};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite};

pub const CALC_HASH_PIECE_SIZE: u64 = 1024 * 1024;
pub const QCID_HASH_PIECE_SIZE: u64 = 4096;
pub const MAX_CHUNK_SIZE: u64 = 1024 * 1024 * 1024 * 2;
pub const COPY_CHUNK_BUFFER_SIZE: usize = CALC_HASH_PIECE_SIZE as usize;

pub type ChunkReader = Pin<Box<dyn AsyncRead + Unpin + Send>>;
pub type ChunkWriter = Pin<Box<dyn AsyncWrite + Unpin + Send>>;

pub struct ChunkIdHashHelper;

impl ChunkIdHashHelper {
    pub fn get_length(hash_type: &str, hash_result: &[u8]) -> Option<u64> {
        // Decode varint length from the beginning of the hash result
        if hash_result.is_empty() {
            return None;
        }

        // Check if the hash type is "mix" to handle special case
        match HashMethod::parse(hash_type) {
            Ok((hash_method, is_mix)) => {
                if is_mix {
                    match unsigned_varint::decode::u64(&hash_result) {
                        Ok((length, _)) => Some(length),
                        Err(_) => None, // If decoding fails, return None
                    }
                } else {
                    // For none-mix hash types, we assume no length encoding
                    None
                }
            }
            _ => {
                // For other hash types, we assume no length encoding
                None
            }
        }
    }

    pub fn get_hash<'a>(hash_type: &str, hash_result: &'a [u8]) -> &'a [u8] {
        //mix hash can get length from hash_hex_string
        if hash_result.is_empty() {
            return hash_result;
        }

        match HashMethod::parse(hash_type) {
            Ok((hash_method, is_mix)) => {
                if is_mix {
                    match unsigned_varint::decode::u64(&hash_result) {
                        Ok((_length, hash)) => hash,
                        Err(_) => &hash_result, // If decoding fails, directly return the whole hash result
                    }
                } else {
                    // For none-mix hash types, we assume no length encoding
                    &hash_result
                }
            }
            _ => {
                // For other hash types, we assume no length encoding
                &hash_result
            }
        }
    }
}

//We support 3 types of chunktype:qcid, sha256, mix at this time
//单个
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChunkId {
    pub hash_type: String,
    pub hash_result: Vec<u8>,
}

//TODO: add mix hash support
impl ChunkId {
    pub fn new(chunk_id_str: &str) -> NdnResult<Self> {
        let obj_id = ObjId::new(chunk_id_str)?;
        if !obj_id.is_chunk() {
            return Err(NdnError::InvalidId(format!(
                "invalid chunk id:{}",
                chunk_id_str
            )));
        }
        Ok(Self {
            hash_type: obj_id.obj_type,
            hash_result: obj_id.obj_hash,
        })
    }

    pub fn as_chunk_id_ref(&self) -> ChunkIdRef {
        ChunkIdRef {
            hash_type: &self.hash_type,
            hash_result: &self.hash_result,
        }
    }

    pub fn to_obj_id(&self) -> ObjId {
        ObjId {
            obj_type: self.hash_type.clone(),
            obj_hash: self.hash_result.clone(),
        }
    }

    pub fn from_obj_id(obj_id: &ObjId) -> Self {
        Self {
            hash_type: obj_id.obj_type.clone(),
            hash_result: obj_id.obj_hash.clone(),
        }
    }

    // Create a new ChunkId without length encoding
    pub fn from_hash_result(hash_result: &[u8], hash_method: HashMethod) -> Self {
        Self {
            hash_type: hash_method.to_string(),
            hash_result: hash_result.to_vec(),
        }
    }

    // Create a new ChunkId with length encoding, in mix mode
    pub fn mix_from_hash_result(
        data_length: u64,
        hash_result: &[u8],
        hash_method: HashMethod,
    ) -> Self {
        let mut length_buf = unsigned_varint::encode::u64_buffer();
        let length_encoded = unsigned_varint::encode::u64(data_length, &mut length_buf);

        let mut encoded = Vec::with_capacity(length_encoded.len() + hash_result.len());
        encoded.extend_from_slice(length_encoded);
        encoded.extend_from_slice(hash_result);

        Self {
            hash_type: hash_method.as_mix_str().to_string(),
            hash_result: encoded.to_vec(),
        }
    }

    pub fn to_string(&self) -> String {
        let hex_str = hex::encode(self.hash_result.clone());
        format!("{}:{}", self.hash_type, hex_str)
    }

    pub fn to_base32(&self) -> String {
        let mut vec_result: Vec<u8> = Vec::new();
        vec_result.extend_from_slice(self.hash_type.as_bytes());
        vec_result.push(b':');
        vec_result.extend_from_slice(&self.hash_result);

        base32::encode(
            base32::Alphabet::Rfc4648Lower { padding: false },
            &vec_result,
        )
    }

    pub fn to_did_string(&self) -> String {
        let hex_str = hex::encode(self.hash_result.clone());
        format!("did:{}:{}", self.hash_type, hex_str)
    }

    pub fn get_length(&self) -> Option<u64> {
        ChunkIdHashHelper::get_length(&self.hash_type, &self.hash_result)
    }

    pub fn get_hash(&self) -> &[u8] {
        ChunkIdHashHelper::get_hash(&self.hash_type, &self.hash_result)
    }

    pub fn equal(&self, hash_bytes: &[u8]) -> bool {
        self.hash_result == hash_bytes
    }
}

impl Into<ObjId> for ChunkId {
    fn into(self) -> ObjId {
        ObjId {
            obj_type: self.hash_type,
            obj_hash: self.hash_result,
        }
    }
}

impl From<ObjId> for ChunkId {
    fn from(obj_id: ObjId) -> Self {
        Self {
            hash_type: obj_id.obj_type,
            hash_result: obj_id.obj_hash,
        }
    }
}

pub struct ChunkIdRef<'a> {
    pub hash_type: &'a str,
    pub hash_result: &'a [u8],
}

impl<'a> ChunkIdRef<'a> {
    pub fn new(chunk_id: &'a ChunkId) -> Self {
        Self {
            hash_type: &chunk_id.hash_type,
            hash_result: &chunk_id.hash_result,
        }
    }

    pub fn from_obj_id(obj_id: &'a ObjId) -> Self {
        Self {
            hash_type: &obj_id.obj_type,
            hash_result: &obj_id.obj_hash,
        }
    }

    pub fn get_length(&self) -> Option<u64> {
        ChunkIdHashHelper::get_length(self.hash_type, self.hash_result)
    }

    pub fn get_hash(&self) -> &[u8] {
        ChunkIdHashHelper::get_hash(self.hash_type, self.hash_result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn test_var_length() {
        let mut buffer = vec![0u8; 2048];
        let mut rng = rand::thread_rng();
        rng.fill(&mut buffer[..]);

        let mut length_buf = unsigned_varint::encode::u64_buffer();
        let length_encoded = unsigned_varint::encode::u64(2048, &mut length_buf);
        println!("length_encoded: {:?}", length_encoded);

        // Decode length
        let (decoded_length, rest) = unsigned_varint::decode::u64(&length_encoded).unwrap();
        println!("decoded_length: {}, rest: {:?}", decoded_length, rest);
        assert_eq!(decoded_length, 2048);

        let chunk_id = ChunkId::mix_from_hash_result(2048, &buffer, HashMethod::sha256);
        println!("chunk_id: {}", chunk_id.to_string());

        let length = chunk_id.get_length().unwrap_or(0);
        println!("chunk_id length: {}", length);
        assert_eq!(length, 2048);
    }
}
