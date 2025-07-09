use super::chunk::{ChunkId, CALC_HASH_PIECE_SIZE, COPY_CHUNK_BUFFER_SIZE, QCID_HASH_PIECE_SIZE};
use crate::hash::DEFAULT_HASH_METHOD;
use crate::{HashHelper, HashMethod, Hasher, NdnError, NdnResult};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use std::{future::Future, io::SeekFrom, ops::Range, path::PathBuf, pin::Pin};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite};

pub struct ChunkHasher {
    pub hash_type: HashMethod,
    pub hash_length: u64,
    pub hasher: Box<dyn Hasher + Send + Sync>,
    //can extend other hash type in the future
}

impl ChunkHasher {
    pub fn new(hash_type: Option<&str>) -> NdnResult<Self> {
        // default is sha256
        let hash_method = match hash_type {
            Some(ht) => HashMethod::from_str(ht)?,
            None => HashMethod::default(),
        };
        
        let hasher = HashHelper::create_hasher(hash_method)?;

        Ok(Self {
            hash_type: hash_method,
            hash_length: 0,
            hasher: hasher,
        })
    }

    pub fn new_with_hash_type(hash_type: HashMethod) -> NdnResult<Self> {
        let hasher = HashHelper::create_hasher(hash_type)?;

        Ok(Self {
            hash_type,
            hash_length: 0,
            hasher,
        })
    }

    pub fn get_pos(&self) -> u64 {
        self.hasher.get_pos()
    }

    pub fn restore_from_state(state_json: serde_json::Value) -> NdnResult<Self> {
        let mut hash_str_type = DEFAULT_HASH_METHOD;
        let hash_type = state_json.get("hash_type");
        if hash_type.is_some() {
            hash_str_type = hash_type.unwrap().as_str().unwrap();
        }
        let hash_method = HashMethod::from_str(hash_str_type)?;

        // Load hash length
        let hash_length = state_json
            .get("hash_length")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Load hasher state
        let mut hasher = HashHelper::create_hasher(hash_method)?;
        hasher.restore_from_state(state_json)?;

        Ok(Self {
            hash_type: hash_method,
            hash_length,
            hasher,
        })
    }

    pub fn save_state(&self) -> NdnResult<serde_json::Value> {
        let mut v = self.hasher.save_state()?;

        // Add hash length
        v.as_object_mut().unwrap().insert(
            "hash_length".to_string(),
            serde_json::json!(self.hash_length),
        );

        Ok(v)
    }

    //return the hash result and the total read size
    pub async fn calc_from_reader<T: AsyncRead + Unpin>(
        mut self,
        reader: &mut T,
    ) -> NdnResult<(Vec<u8>, u64)> {
        //TODO: add other hash type support

        let mut buffer = vec![0u8; CALC_HASH_PIECE_SIZE as usize];
        let mut total_read = 0;
        loop {
            let n = reader.read(&mut buffer).await.map_err(|e| {
                warn!("ChunkHasher: read failed! {}", e.to_string());
                NdnError::IoError(e.to_string())
            })?;

            // 如果读取到0字节，表示已经到达EOF
            if n == 0 {
                break;
            }

            // 更新哈希计算器
            self.hasher.update_from_bytes(&buffer[..n]);
            total_read += n as u64;
        }

        self.hash_length += total_read;

        Ok((self.hasher.finalize().to_vec(), total_read))
    }

    pub fn calc_from_bytes(mut self, bytes: &[u8]) -> Vec<u8> {
        self.hash_length += bytes.len() as u64;
        self.hasher.update_from_bytes(bytes);
        self.hasher.finalize().to_vec()
    }

    pub fn calc_chunk_id_from_bytes(mut self, bytes: &[u8]) -> ChunkId {
        self.hash_length += bytes.len() as u64;
        self.hasher.update_from_bytes(bytes);
        self.finalize_chunk_id()
    }

    pub fn calc_mix_chunk_id_from_bytes(mut self, bytes: &[u8]) -> ChunkId {
        self.hash_length += bytes.len() as u64;
        self.hasher.update_from_bytes(bytes);
        self.finalize_mix_chunk_id()
    }

    pub fn update_from_bytes(&mut self, bytes: &[u8]) {
        self.hash_length += bytes.len() as u64;
        self.hasher.update_from_bytes(bytes);
    }

    pub fn finalize(self) -> Vec<u8> {
        self.hasher.finalize().to_vec()
    }

    pub fn finalize_chunk_id(self) -> ChunkId {
        let hash_result = self.hasher.finalize();
        ChunkId::from_hash_result(&hash_result, self.hash_type)
    }

    pub fn finalize_mix_chunk_id(self) -> ChunkId {
        let hash_result = self.hasher.finalize();
        ChunkId::mix_from_hash_result(self.hash_length, &hash_result, self.hash_type)
    }
}

//TODO: this function require a seekable reader
//quick hash is only for qcid
pub async fn calc_quick_hash<T: AsyncRead + AsyncSeek + Unpin>(
    reader: &mut T,
    length: Option<u64>,
) -> NdnResult<ChunkId> {
    let length = if let Some(length) = length {
        length
    } else {
        let length = reader.seek(SeekFrom::End(0)).await.map_err(|e| {
            warn!("calc_quick_hash: seek file failed! {}", e.to_string());
            NdnError::IoError(e.to_string())
        })?;
        reader.seek(SeekFrom::Start(0)).await.map_err(|e| {
            warn!("calc_quick_hash: seek file failed! {}", e.to_string());
            NdnError::IoError(e.to_string())
        })?;
        length
    };

    if length < QCID_HASH_PIECE_SIZE * 3 {
        return Err(NdnError::Internal(format!(
            "quick hash error: item size is too small"
        )));
    }

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; QCID_HASH_PIECE_SIZE as usize];
    let mut offset = 0;
    reader.read_exact(&mut buffer).await.map_err(|e| {
        warn!("calc_quick_hash: read file failed! {}", e.to_string());
        NdnError::IoError(e.to_string())
    })?;
    hasher.update(&buffer);
    offset = length / 2;
    reader.seek(SeekFrom::Start(offset)).await.map_err(|e| {
        warn!("calc_quick_hash: seek file failed! {}", e.to_string());
        NdnError::IoError(e.to_string())
    })?;
    reader.read_exact(&mut buffer).await.map_err(|e| {
        warn!("calc_quick_hash: read file failed! {}", e.to_string());
        NdnError::IoError(e.to_string())
    })?;
    hasher.update(&buffer);
    let hash_result = hasher.finalize();

    Ok(ChunkId {
        hash_type: "qcid".to_string(),
        hash_result: hash_result.to_vec(),
    })
}

pub async fn calc_quick_hash_by_buffer(
    buffer_begin: &[u8],
    buffer_mid: &[u8],
    buffer_end: &[u8],
) -> NdnResult<ChunkId> {
    let mut hasher = Sha256::new();
    let limit_size = QCID_HASH_PIECE_SIZE as usize;
    if buffer_begin.len() != limit_size
        || buffer_mid.len() != limit_size
        || buffer_end.len() != limit_size
    {
        return Err(NdnError::Internal(format!(
            "cacl quick hash buffer part length must be 4096"
        )));
    }

    hasher.update(buffer_begin);
    hasher.update(buffer_mid);
    hasher.update(buffer_end);
    let hash_result = hasher.finalize();
    Ok(ChunkId {
        hash_type: "qcid".to_string(),
        hash_result: hash_result.to_vec(),
    })
}

pub async fn calculate_file_chunk_id(
    file_path: &str,
    hash_method: HashMethod,
) -> NdnResult<(ChunkId, u64)> {
    let mut file_reader = tokio::fs::File::open(file_path).await.map_err(|err| {
        warn!(
            "calculate_file_chunk_id: open file failed! {}",
            err.to_string()
        );
        NdnError::IoError(err.to_string())
    })?;

    let mut hasher = ChunkHasher::new_with_hash_type(hash_method)?;
    let (hash_result, file_size) = hasher.calc_from_reader(&mut file_reader).await?;
    Ok((
        ChunkId::mix_from_hash_result(file_size, &hash_result, hash_method),
        file_size,
    ))
}

pub async fn copy_chunk<R, W, F>(
    chunk_id: ChunkId,
    mut chunk_reader: R,
    mut chunk_writer: W,
    mut hasher: Option<ChunkHasher>,
    mut progress_callback: Option<F>,
) -> NdnResult<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnMut(
        ChunkId,
        u64,
        &Option<ChunkHasher>,
    ) -> Pin<Box<dyn Future<Output = NdnResult<()>> + Send + 'static>>,
{
    let mut total_copied: u64 = 0;
    let mut buffer = vec![0u8; COPY_CHUNK_BUFFER_SIZE];

    loop {
        let n = tokio::io::AsyncReadExt::read(&mut chunk_reader, &mut buffer)
            .await
            .map_err(|e| NdnError::IoError(e.to_string()))?;
        if n == 0 {
            break;
        }

        if let Some(ref mut hasher) = hasher {
            if hasher.hash_type.as_str() == chunk_id.hash_type {
                hasher.update_from_bytes(&buffer[..n]);
            } else {
                return Err(NdnError::Internal(format!(
                    "hash type mismatch:{}",
                    hasher.hash_type.as_str()
                )));
            }
        }

        tokio::io::AsyncWriteExt::write_all(&mut chunk_writer, &buffer[..n])
            .await
            .map_err(|e| NdnError::IoError(e.to_string()))?;
        total_copied += n as u64;

        if let Some(ref mut progress_callback) = progress_callback {
            progress_callback(chunk_id.clone(), total_copied, &hasher).await?;
        }
    }

    if let Some(hasher) = hasher {
        let result_chunk_id = ChunkHasher::finalize_chunk_id(hasher);
        if result_chunk_id != chunk_id {
            return Err(NdnError::VerifyError(format!(
                "copy chunk hash mismatch:{}",
                result_chunk_id.to_string()
            )));
        }
    }

    Ok(total_copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn test_chunk_hasher_save_state() {
        let mut buffer = vec![0u8; 2048];
        let mut rng = rand::rng();
        rng.fill(&mut buffer[..]);

        let mut chunk_hasher = ChunkHasher::new(None).unwrap();
        let hash_result = chunk_hasher.calc_from_bytes(&buffer);

        let hash_result_restored = {
            let mut chunk_hasher = ChunkHasher::new(None).unwrap();
            chunk_hasher.update_from_bytes(&buffer[..1024]);
            let state_json = chunk_hasher.save_state().unwrap();
            println!("state_json:{}", state_json.to_string());

            let mut chunk_hasher_restored = ChunkHasher::restore_from_state(state_json).unwrap();
            chunk_hasher_restored.update_from_bytes(&buffer[1024..]);
            // let hash = chunk_hasher_restored.finalize();

            let chunk_id = chunk_hasher_restored.finalize_chunk_id();
            let length = chunk_id.get_length().unwrap_or(0);
            println!("chunk_id: {}, length: {}", chunk_id.to_string(), length);
            assert_eq!(length, 2048);

            chunk_id.get_hash().to_vec()
        };

        assert_eq!(hash_result, hash_result_restored);
    }
}
