use base58::{ToBase58, FromBase58};
use sha2::{Sha256, Digest};
use tokio::io::{self, AsyncRead, AsyncSeek, AsyncSeekExt, AsyncReadExt};
use std::io::SeekFrom;
use hex;
use log::*;
use crate::{ChunkResult, ChunkError};

pub const CACL_HASH_PIECE_SIZE: u64 = 1024*1024;
pub const QCID_HASH_PIECE_SIZE: u64 = 4096;
pub const MAX_CHUNK_SIZE: u64 = 1024*1024*1024*4;

//We support 3 types of chunktype:qcid, sha256, mix at this time
//单个
#[derive(Debug, Clone,Eq, PartialEq)]
pub struct ChunkId {
    pub hash_type:String,
    pub hash_hex_string: String,
}

//TODO: add mix hash support
impl ChunkId {
    pub fn new(chunk_id_str:&str) -> ChunkResult<Self> {
        let split = chunk_id_str.split(":").collect::<Vec<&str>>();
        if split.len() != 2 {
            return Err(ChunkError::InvalidId(chunk_id_str.to_string()));
        }
        Ok(Self { hash_hex_string:split[1].to_string(), hash_type:split[0].to_string() })
    }

    pub fn from_sha256_result(hash_result: &[u8]) -> Self {
        let hex_string = hex::encode(hash_result);
        Self { hash_hex_string:hex_string, hash_type:"sha256".to_string() }
    }

    pub fn to_string(&self) -> String {
        format!("{}:{}", self.hash_type, self.hash_hex_string)
    }

    pub fn to_hostname(&self) -> String {
        format!("{}-{}", self.hash_hex_string, self.hash_type)
    }

    pub fn from_hostname(hostname: &str) -> ChunkResult<Self> {
        let sub_host = hostname.split(".").collect::<Vec<&str>>();
        let first_part = sub_host[0];

        let pos = first_part.rfind("-").unwrap();
        let hash_hex_string = &first_part[..pos];
        let hash_type = &first_part[pos+1..];
        Ok(Self { hash_hex_string:hash_hex_string.to_string(), hash_type:hash_type.to_string() })   
    }

    pub fn get_length(&self) -> Option<u64> {
        //mix hash can get length from hash_hex_string
        None
    }    

    pub fn is_equal(&self, hash_bytes: &[u8])->bool {
        self.hash_hex_string == hex::encode(hash_bytes)
    }
}


pub struct ChunkHasher {
    hasher: Sha256,
}

impl ChunkHasher {
    pub fn new(hash_type: Option<&str>) -> ChunkResult<Self> {
        //default is sha256
        let hasher = match hash_type {
            Some("sha256") => Sha256::new(),
            None => Sha256::new(),
            _ => return Err(ChunkError::Internal(format!("invalid hash type:{}",hash_type.unwrap_or("")))),
        };

        Ok(Self {
            hasher: hasher,
        })
    }

    pub async fn calc_from_reader<T: AsyncRead + Unpin>(&mut self, reader: &mut T) -> ChunkResult<Vec<u8>> {
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; CACL_HASH_PIECE_SIZE as usize];
        loop {
            let n = reader.read(&mut buffer).await
            .map_err(|e| {
                warn!("ChunkHasher: read failed! {}", e.to_string());
                ChunkError::IoError(e.to_string())
            })?;
            if n < CACL_HASH_PIECE_SIZE as usize {
                break;
            }
            hasher.update(&buffer[..n]);
            
        }

        Ok(hasher.finalize().to_vec())
    }

    pub fn calc_from_bytes(&mut self,bytes: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher.finalize().to_vec()
    }

    pub fn update_from_bytes(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    pub fn finalize(self) -> Vec<u8> {
        self.hasher.finalize().to_vec()
    }
}

pub async fn calc_quick_hash<T: AsyncRead + AsyncSeek + Unpin>(reader: &mut T, length: Option<u64>) -> ChunkResult<ChunkId> {
    let length = if let Some(length) = length {
        length
    } else {
        let length = reader.seek(SeekFrom::End(0)).await
            .map_err(|e| {
                warn!("calc_quick_hash: seek file failed! {}",e.to_string());
                ChunkError::IoError(e.to_string())
            })?;
        reader.seek(SeekFrom::Start(0)).await
            .map_err(|e| {
                warn!("calc_quick_hash: seek file failed! {}",e.to_string());
                ChunkError::IoError(e.to_string())
            })?;
        length
    };

    if length < QCID_HASH_PIECE_SIZE*3 {
        return Err(ChunkError::Internal(format!("quick hash error: item size is too small")));
    }

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; QCID_HASH_PIECE_SIZE as usize];
    let mut offset = 0;
    reader.read_exact(&mut buffer).await
        .map_err(|e| {
            warn!("calc_quick_hash: read file failed! {}",e.to_string());
            ChunkError::IoError(e.to_string())
        })?;
    hasher.update(&buffer);
    offset = length/2;
    reader.seek(SeekFrom::Start(offset)).await
        .map_err(|e| {
            warn!("calc_quick_hash: seek file failed! {}",e.to_string());
            ChunkError::IoError(e.to_string())
        })?;    
    reader.read_exact(&mut buffer).await
        .map_err(|e| {
            warn!("calc_quick_hash: read file failed! {}",e.to_string());
            ChunkError::IoError(e.to_string())
        })?;
    hasher.update(&buffer);
    let hash_result = hasher.finalize();

    Ok( ChunkId{
        hash_hex_string:hex::encode(hash_result),
        hash_type:"qcid".to_string(),
    })
}

pub async fn calc_quick_hash_by_buffer(buffer_begin: &[u8],buffer_mid: &[u8],buffer_end: &[u8]) -> ChunkResult<ChunkId> {
    let mut hasher = Sha256::new();
    let limit_size = QCID_HASH_PIECE_SIZE as usize;
    if buffer_begin.len() != limit_size || buffer_mid.len() != limit_size || buffer_end.len() != limit_size {
        return Err(ChunkError::Internal(format!("cacl quick hash buffer part length must be 4096")));
    }

    hasher.update(buffer_begin);
    hasher.update(buffer_mid);
    hasher.update(buffer_end);
    let hash_result = hasher.finalize();
    Ok( ChunkId{
        hash_hex_string:hex::encode(hash_result),
        hash_type:"qcid".to_string(),
    })
}

//strcut ChunkData ?

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn test_chunk_id_from_hostname() {
        let chunk_id = ChunkId::from_hostname("1234567890abcdef-sha256.ndn.buckyos.org").unwrap();
        assert_eq!(chunk_id.to_string(), "sha256:1234567890abcdef");

        let chunk_id = ChunkId::new("sha256:1234567890abcdef").unwrap();
        assert_eq!(chunk_id.to_hostname(), "1234567890abcdef-sha256");
    }
}