use crate::hash::*;
use crate::{
    object::{ObjId, ObjIdBytesCodec},
    NdnError, NdnResult,
};
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ChunkType {
    Sha256,
    Mix256,
    Sha512,
    Mix512,
    QCID,//QCID already include length encoding
    Blake2s256,
    MixBlake2s256,
    Keccak256,
    MixKeccak256,
    Unknown(String),
}

impl ChunkType {
    pub fn is_chunk_type(type_str: &str) -> bool {
        match type_str {
            "sha256" | "mix256" | "sha512" | "mix512" | "qcid" | "blake2s256" | "mixblake2s256" | "keccak256" | "mixkeccak256" => true,
            _ => false,
        }
    }

    pub fn is_mix(&self) -> bool {
        match self {
            ChunkType::Mix256 | ChunkType::Mix512 | ChunkType::MixBlake2s256 | ChunkType::MixKeccak256 | ChunkType::QCID => true,
            _ => false,
        }
    }

    pub fn from_hash_type(hash_type: HashMethod,is_mix: bool) -> NdnResult<Self> {
        match hash_type {
            HashMethod::Sha256 =>{
                if is_mix {
                    Ok(ChunkType::Mix256)
                } else {
                    Ok(ChunkType::Sha256)
                }
            }
            HashMethod::Sha512 =>{
                if is_mix {
                    Ok(ChunkType::Mix512)
                } else {
                    Ok(ChunkType::Sha512)
                }
            }
            HashMethod::QCID =>{
                if is_mix {
                    Ok(ChunkType::QCID)
                } else {
                    return Err(NdnError::InvalidObjType("QCID must be mix hash".to_string()));
                }
            }
            HashMethod::Blake2s256 =>{
                if is_mix {
                    Ok(ChunkType::MixBlake2s256)
                } else {
                    Ok(ChunkType::Blake2s256)
                }
            }
            HashMethod::Keccak256 =>{
                if is_mix {
                    Ok(ChunkType::MixKeccak256)
                } else {
                    Ok(ChunkType::Keccak256)
                }
            }
        }
    }

    pub fn to_hash_method(&self) -> NdnResult<HashMethod> {
        match self {
            ChunkType::Sha256 => Ok(HashMethod::Sha256),
            ChunkType::Mix256 => Ok(HashMethod::Sha256),
            ChunkType::Sha512 => Ok(HashMethod::Sha512),
            ChunkType::Mix512 => Ok(HashMethod::Sha512),
            ChunkType::QCID => Ok(HashMethod::QCID),
            ChunkType::Blake2s256 => Ok(HashMethod::Blake2s256),
            ChunkType::MixBlake2s256 => Ok(HashMethod::Blake2s256),
            ChunkType::Keccak256 => Ok(HashMethod::Keccak256),
            ChunkType::MixKeccak256 => Ok(HashMethod::Keccak256),
            ChunkType::Unknown(s) => Err(NdnError::InvalidObjType(format!("invalid chunk type:{}",s))),
        }
    }
}
impl FromStr for ChunkType {
    type Err = NdnError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sha256" => Ok(ChunkType::Sha256),
            "mix256" => Ok(ChunkType::Mix256),
            "sha512" => Ok(ChunkType::Sha512),
            "mix512" => Ok(ChunkType::Mix512),
            "qcid" => Ok(ChunkType::QCID),
            "blake2s256" => Ok(ChunkType::Blake2s256),
            "mixblake2s256" => Ok(ChunkType::MixBlake2s256),
            "keccak256" => Ok(ChunkType::Keccak256),
            "mixkeccak256" => Ok(ChunkType::MixKeccak256),
            _ => Ok(ChunkType::Unknown(s.to_string())),
        }
    }
}

impl ToString for ChunkType {
    fn to_string(&self) -> String {
        match self {
            ChunkType::Sha256 => "sha256".to_string(),
            ChunkType::Mix256 => "mix256".to_string(),
            ChunkType::Sha512 => "sha512".to_string(), 
            ChunkType::Mix512 => "mix512".to_string(),
            ChunkType::QCID => "qcid".to_string(),
            ChunkType::Blake2s256 => "blake2s256".to_string(),
            ChunkType::MixBlake2s256 => "mixblake2s256".to_string(),
            ChunkType::Keccak256 => "keccak256".to_string(),
            ChunkType::MixKeccak256 => "mixkeccak256".to_string(),
            ChunkType::Unknown(s) => s.clone(),
        }
    }
}

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
    pub chunk_type: ChunkType,
    pub hash_result: Vec<u8>,
}

//TODO: add mix hash support
impl ChunkId {
    pub fn default_chunk_type() -> ChunkType {
        ChunkType::Mix256
    }
    
    pub fn new(chunk_id_str: &str) -> NdnResult<Self> {
        let obj_id = ObjId::new(chunk_id_str)?;
        if !obj_id.is_chunk() {
            return Err(NdnError::InvalidId(format!(
                "invalid chunk id:{}",
                chunk_id_str
            )));
        }
        let chunk_type = ChunkType::from_str(&obj_id.obj_type)?;
        Ok(Self {
            chunk_type: chunk_type,
            hash_result: obj_id.obj_hash,
        })
    }



    pub fn to_obj_id(&self) -> ObjId {
        ObjId {
            obj_type: self.chunk_type.to_string(),
            obj_hash: self.hash_result.clone(),
        }
    }

    pub fn from_obj_id(obj_id: &ObjId) -> Self {
        Self {
            chunk_type: ChunkType::from_str(&obj_id.obj_type).unwrap(),
            hash_result: obj_id.obj_hash.clone(),
        }
    }

    // Create a new ChunkId without length encoding
    pub fn from_hash_result(hash_result: &[u8], chunk_type: ChunkType) -> Self {
        Self {
            chunk_type: chunk_type,
            hash_result: hash_result.to_vec(),
        }
    }

    pub fn from_mix_hash_result(data_length: u64,hash_result: &[u8], chunk_type: ChunkType) -> Self {
        let encoded = Self::mix_length_and_hash_result(data_length,hash_result);
        Self {
            chunk_type: chunk_type,
            hash_result: encoded.to_vec(),
        }
    }

        // Create a new ChunkId with length encoding, in mix mode
        pub fn from_mix_hash_result_by_hash_method(
            data_length: u64,
            hash_result: &[u8],
            hash_method: HashMethod,
        ) -> NdnResult<Self> {
            let chunk_type = ChunkType::from_hash_type(hash_method,true)?;
            let encoded = Self::mix_length_and_hash_result(data_length,hash_result);
    
            Ok(Self {
                chunk_type: chunk_type,
                hash_result: encoded.to_vec(),
            })
        }

    pub fn from_sha256_result(hash_result: &[u8]) -> Self {
        Self {
            chunk_type: ChunkType::Sha256,
            hash_result: hash_result.to_vec(),
        }
    }

    pub fn from_mix256_result(data_length: u64,hash_result: &[u8]) -> Self {
        let encoded = Self::mix_length_and_hash_result(data_length,hash_result);
        Self {
            chunk_type: ChunkType::Mix256,
            hash_result: encoded.to_vec(),
        }
    }

    pub fn mix_length_and_hash_result(data_length: u64,hash_result: &[u8]) -> Vec<u8> {
        let mut length_buf = unsigned_varint::encode::u64_buffer();
        let length_encoded = unsigned_varint::encode::u64(data_length, &mut length_buf);
        let mut encoded = Vec::with_capacity(length_encoded.len() + hash_result.len());
        encoded.extend_from_slice(length_encoded);
        encoded.extend_from_slice(hash_result);
        encoded
    }
    


    pub fn to_string(&self) -> String {
        let hex_str = hex::encode(self.hash_result.clone());
        format!("{}:{}", self.chunk_type.to_string(), hex_str)
    }

    pub fn to_base32(&self) -> String {
        let mut vec_result: Vec<u8> = Vec::new();
        let chunk_type_str = self.chunk_type.to_string();
        vec_result.extend_from_slice(chunk_type_str.as_bytes());
        vec_result.push(b':');
        vec_result.extend_from_slice(&self.hash_result);

        base32::encode(
            base32::Alphabet::Rfc4648Lower { padding: false },
            &vec_result,
        )
    }

    pub fn to_did_string(&self) -> String {
        let hex_str = hex::encode(self.hash_result.clone());
        format!("did:{}:{}", self.chunk_type.to_string(), hex_str)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        ObjIdBytesCodec::to_bytes(&self.chunk_type.to_string(), &self.hash_result)
    }

    pub fn from_bytes(chunk_id_bytes: &[u8]) -> NdnResult<Self> {
        let (hash_type_str, hash_result) = ObjIdBytesCodec::from_bytes(chunk_id_bytes)?;
        let chunk_type = ChunkType::from_str(&hash_type_str)?;
        Ok(Self {
            chunk_type: chunk_type,
            hash_result,
        })
    }

    pub fn get_length(&self) -> Option<u64> {

        if self.hash_result.is_empty() {
            return None;
        }
        let hash_result_slice = self.hash_result.as_slice();
        // Check if the hash type is "mix" to handle special case
        if self.chunk_type.is_mix() {
            let length = unsigned_varint::decode::u64(hash_result_slice);
            if length.is_ok() {
                return Some(length.unwrap().0);
            }
        }
        return None;
    }

    // pub fn get_hash(&self) -> &[u8] {
    //     ChunkIdHashHelper::get_hash(&self.chunk_type, &self.hash_result)
    // }

    pub fn equal(&self, hash_bytes: &[u8]) -> bool {
        self.hash_result == hash_bytes
    }
}

impl Into<ObjId> for ChunkId {
    fn into(self) -> ObjId {
        ObjId {
            obj_type: self.chunk_type.to_string(),
            obj_hash: self.hash_result,
        }
    }
}

impl From<ObjId> for ChunkId {
    fn from(obj_id: ObjId) -> Self {
        Self {
            chunk_type: ChunkType::from_str(&obj_id.obj_type).unwrap(),
            hash_result: obj_id.obj_hash,
        }
    }
}

impl From<ChunkId> for Vec<u8> {
    fn from(chunk_id: ChunkId) -> Self {
        chunk_id.to_bytes()
    }
}

impl TryFrom<&[u8]> for ChunkId {
    type Error = NdnError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(value)
    }
}

impl TryFrom<Vec<u8>> for ChunkId {
    type Error = NdnError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::from_bytes(&value)
    }
}

// pub struct ChunkIdRef<'a> {
//     pub hash_type: &'a ChunkType,
//     pub hash_result: &'a [u8],
// }

// impl<'a> ChunkIdRef<'a> {
//     pub fn new(chunk_id: &'a ChunkId) -> Self {
//         Self {
//             hash_type: &chunk_id.chunk_type,
//             hash_result: &chunk_id.hash_result,
//         }
//     }

//     pub fn get_length(&self) -> Option<u64> {
//         ChunkId::get_length(self.hash_type, self.hash_result)
//     }

//     pub fn get_hash(&self) -> &[u8] {
//         ChunkIdHashHelper::get_hash(self.hash_type, self.hash_result)
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn test_var_length() {
        let mut buffer = vec![0u8; 2048];
        let mut rng = rand::rng();
        rng.fill(&mut buffer[..]);

        let mut length_buf = unsigned_varint::encode::u64_buffer();
        let length_encoded = unsigned_varint::encode::u64(2048, &mut length_buf);
        println!("length_encoded: {:?}", length_encoded);

        // Decode length
        let (decoded_length, rest) = unsigned_varint::decode::u64(&length_encoded).unwrap();
        println!("decoded_length: {}, rest: {:?}", decoded_length, rest);
        assert_eq!(decoded_length, 2048);

        let chunk_id = ChunkId::from_mix_hash_result_by_hash_method(2048, &buffer, HashMethod::Sha256).unwrap();
        println!("chunk_id: {}", chunk_id.to_string());

        let length = chunk_id.get_length().unwrap_or(0);
        println!("chunk_id length: {}", length);
        assert_eq!(length, 2048);
    }
}
