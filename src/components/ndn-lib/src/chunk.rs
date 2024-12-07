use async_trait::async_trait;
use base58::{ToBase58, FromBase58};
use sha2::{Sha256, Digest};
use crypto_common::hazmat::{SerializedState, SerializableState};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite};
use std::{io::SeekFrom, ops::Range, pin::Pin};
use serde_json::{json, Value};
use hex;
use log::*;
use crate::{object::{ObjId}, NdnError, NdnResult};

pub const CACL_HASH_PIECE_SIZE: u64 = 1024*1024;
pub const QCID_HASH_PIECE_SIZE: u64 = 4096;
pub const MAX_CHUNK_SIZE: u64 = 1024*1024*1024*4;
pub const COPY_CHUNK_BUFFER_SIZE: usize = CACL_HASH_PIECE_SIZE as usize;

type ChunkReader = Pin<Box<dyn AsyncRead + Unpin>>;
type ChunkWriter = Pin<Box<dyn AsyncWrite + Unpin>>;
//We support 3 types of chunktype:qcid, sha256, mix at this time
//单个
#[derive(Debug, Clone,Eq, PartialEq)]
pub struct ChunkId {
    pub hash_type:String,
    pub hash_hex_string: String,
}

//TODO: add mix hash support
impl ChunkId {
    pub fn new(chunk_id_str:&str) -> NdnResult<Self> {
        let split = chunk_id_str.split(":").collect::<Vec<&str>>();
        if split.len() != 2 {
            return Err(NdnError::InvalidId(chunk_id_str.to_string()));
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

    pub fn to_did_string(&self) -> String {
        format!("did:{}:{}", self.hash_type, self.hash_hex_string)
    }
    

    pub fn to_hostname(&self) -> String {
        format!("{}-{}", self.hash_hex_string, self.hash_type)
    }

    pub fn from_hostname(hostname: &str) -> NdnResult<Self> {
        let sub_host = hostname.split(".").collect::<Vec<&str>>();
        let first_part = sub_host[0];

        let pos = first_part.rfind("-").unwrap();
        let hash_hex_string = &first_part[..pos];
        let hash_type = &first_part[pos+1..];
        Ok(Self { hash_hex_string:hash_hex_string.to_string(), hash_type:hash_type.to_string() })   
    }

    pub fn from_url_path(path:&str) -> NdnResult<Self> {
        let path_parts = path.split("/").collect::<Vec<&str>>();
        let first_part = path_parts[0];
        return Self::new(first_part);
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
    hash_type:String,
    sha_hasher: Option<Sha256>,
    pos: u64,
    //can extend other hash type in the future
}



impl ChunkHasher {

    pub fn new(hash_type: Option<&str>) -> NdnResult<Self> {
        //default is sha256
        let hasher = match hash_type {
            Some("sha256") => Sha256::new(),
            None => Sha256::new(),
            _ => return Err(NdnError::Internal(format!("invalid hash type:{}",hash_type.unwrap_or("")))),
        };

        Ok(Self {
            hash_type:hash_type.unwrap_or("sha256").to_string(),
            sha_hasher: Some(hasher),
            pos: 0,
        })
    }

    pub fn restore_from_state(state_json:serde_json::Value) -> NdnResult<Self> {
        let hash_type = state_json["hash_type"].as_str().unwrap_or("sha256");
        let pos = state_json["pos"].as_u64().unwrap_or(0);
        let serialized_state = hex::decode(
            &state_json["state"].as_str().ok_or(NdnError::Internal("invalid hasher state json".to_string()))?)
            .map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?;

        let hasher = Sha256::deserialize(
            &SerializedState::<Sha256>::try_from(&serialized_state[..]).map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?)
            .map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?;
        Ok(Self {
            hash_type: hash_type.to_string(),
            sha_hasher: Some(hasher),
            pos: pos,
        })
    }

    pub fn save_state(&self) -> serde_json::Value {
        if let Some(hasher) = &self.sha_hasher {
            let will_save = json!({
                "hash_type": self.hash_type,
                "pos": self.pos,
                "state": hex::encode(hasher.serialize()),
            });
            will_save
        } else {
            serde_json::Value::Null
        }
    }

    pub async fn calc_from_reader<T: AsyncRead + Unpin>(&mut self, reader: &mut T) -> NdnResult<Vec<u8>> {
        //TODO: add other hash type support
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; CACL_HASH_PIECE_SIZE as usize];
        loop {
            let n = reader.read(&mut buffer).await
            .map_err(|e| {
                warn!("ChunkHasher: read failed! {}", e.to_string());
                NdnError::IoError(e.to_string())
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
        let mut hasher = self.sha_hasher.as_mut().unwrap();
        hasher.update(bytes);
        self.pos += bytes.len() as u64;
    }

    pub fn finalize(self) -> Vec<u8> {
        let hasher = self.sha_hasher.unwrap();
        hasher.finalize().to_vec()
    }

    pub fn finalize_chunk_id(self) -> ChunkId {
        let hash_result = self.finalize();
        ChunkId::from_sha256_result(&hash_result)
    }
}

//quick hash is only for qcid
pub async fn calc_quick_hash<T: AsyncRead + AsyncSeek + Unpin>(reader: &mut T, length: Option<u64>) -> NdnResult<ChunkId> {
    let length = if let Some(length) = length {
        length
    } else {
        let length = reader.seek(SeekFrom::End(0)).await
            .map_err(|e| {
                warn!("calc_quick_hash: seek file failed! {}",e.to_string());
                NdnError::IoError(e.to_string())
            })?;
        reader.seek(SeekFrom::Start(0)).await
            .map_err(|e| {
                warn!("calc_quick_hash: seek file failed! {}",e.to_string());
                NdnError::IoError(e.to_string())
            })?;
        length
    };

    if length < QCID_HASH_PIECE_SIZE*3 {
        return Err(NdnError::Internal(format!("quick hash error: item size is too small")));
    }

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; QCID_HASH_PIECE_SIZE as usize];
    let mut offset = 0;
    reader.read_exact(&mut buffer).await
        .map_err(|e| {
            warn!("calc_quick_hash: read file failed! {}",e.to_string());
            NdnError::IoError(e.to_string())
        })?;
    hasher.update(&buffer);
    offset = length/2;
    reader.seek(SeekFrom::Start(offset)).await
        .map_err(|e| {
            warn!("calc_quick_hash: seek file failed! {}",e.to_string());
            NdnError::IoError(e.to_string())
        })?;    
    reader.read_exact(&mut buffer).await
        .map_err(|e| {
            warn!("calc_quick_hash: read file failed! {}",e.to_string());
            NdnError::IoError(e.to_string())
        })?;
    hasher.update(&buffer);
    let hash_result = hasher.finalize();

    Ok( ChunkId{
        hash_hex_string:hex::encode(hash_result),
        hash_type:"qcid".to_string(),
    })
}

pub async fn calc_quick_hash_by_buffer(buffer_begin: &[u8],buffer_mid: &[u8],buffer_end: &[u8]) -> NdnResult<ChunkId> {
    let mut hasher = Sha256::new();
    let limit_size = QCID_HASH_PIECE_SIZE as usize;
    if buffer_begin.len() != limit_size || buffer_mid.len() != limit_size || buffer_end.len() != limit_size {
        return Err(NdnError::Internal(format!("cacl quick hash buffer part length must be 4096")));
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


pub enum ObjectState {
    Exist,
    NotCompleted,
    NotExist,
    Object(String),//json_str
    Reader(ChunkReader,u64),//u64 is the chunk size
    Link(LinkData),
}


pub enum LinkData {
    SameAs(ObjId),//Same ChunkId
    //ComposedBy(ChunkId,ObjMapId),// Base ChunkId + Diff Action Items
    PartOf(ChunkId,Range<u64>), //Object Id + Range
    //IndexOf(ObjId,u64),//Object Id + Index
}

impl LinkData {
    pub fn to_string(&self)->String {
        unimplemented!()
    }

    pub fn from_string(link_str:&str)->NdnResult<Self> {
        unimplemented!()
    }
}

pub async fn copy_chunk<R, W, F>(
    chunk_id: ChunkId,
    mut chunk_reader: R,
    mut chunk_writer: W,
    mut hasher: Option<ChunkHasher>,
    mut progress_callback: Option<F>
) -> NdnResult<u64> 
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnMut(&ChunkId, u64, &Option<ChunkHasher>) -> futures::future::BoxFuture<'static, NdnResult<()>>,
{
    let mut total_copied: u64 = 0;
    let mut buffer = vec![0u8; COPY_CHUNK_BUFFER_SIZE]; 

    loop {
        let n = tokio::io::AsyncReadExt::read(&mut chunk_reader, &mut buffer).await
            .map_err(|e| NdnError::IoError(e.to_string()))?;
        if n == 0 {
            break;
        }

        if let Some(ref mut hasher) = hasher {
            hasher.update_from_bytes(&buffer);
        }

        tokio::io::AsyncWriteExt::write_all(&mut chunk_writer, &buffer[..n]).await
            .map_err(|e| NdnError::IoError(e.to_string()))?;
        total_copied += n as u64;

        if let Some(ref mut progress_callback) = progress_callback {
            progress_callback(&chunk_id, total_copied, &hasher).await?;
        }
    }

    if let Some(hasher) = hasher {
        let result_chunk_id = hasher.finalize_chunk_id();
        if result_chunk_id != chunk_id {
            return Err(NdnError::VerifyError(format!("copy chunk hash mismatch:{}",result_chunk_id.to_string())));
        }
    }

    Ok(total_copied)
}



pub struct ObjectLink {
    pub obj_id:Option<ObjId>,
    pub link_data:LinkData,
}

impl ObjectLink {
    pub fn to_string(&self)->String {
        unimplemented!()
    }

    pub fn from_string(link_str:&str)->NdnResult<Self> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn test_chunk_id_from_hostname() {
        let chunk_id = ChunkId::from_hostname("1234567890abcdef-sha256.ndn.buckyos.org").unwrap();
        assert_eq!(chunk_id.to_string(), "sha256:1234567890abcdef");

        let chunk_id = ChunkId::new("sha256:1234567890abcdef").unwrap();
        assert_eq!(chunk_id.to_hostname(), "1234567890abcdef-sha256");
    }

    #[test]
    fn test_chunk_hasher_save_state() {
        let mut buffer = vec![0u8; 2048];
        let mut rng = rand::thread_rng();
        rng.fill(&mut buffer[..]);

        let mut chunk_hasher = ChunkHasher::new(None).unwrap();
        let hash_result = chunk_hasher.calc_from_bytes(&buffer);

        let hash_result_restored = {
            let mut chunk_hasher = ChunkHasher::new(None).unwrap();
            chunk_hasher.update_from_bytes(&buffer[..1024]);
            let state_json = chunk_hasher.save_state();
            println!("state_json:{}",state_json.to_string());


            let mut chunk_hasher_restored = ChunkHasher::restore_from_state(state_json).unwrap();
            chunk_hasher_restored.update_from_bytes(&buffer[1024..]);
            chunk_hasher_restored.finalize()
        };

        assert_eq!(hash_result, hash_result_restored);
       
    }
}