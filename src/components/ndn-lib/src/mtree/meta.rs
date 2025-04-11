use super::stream::{MtreeReadSeek, MtreeWriteSeek};
use crate::hash::HashMethod;
use crate::{NdnError, NdnResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

//meta data of the mtree object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTreeMetaData {
    pub data_size: u64,
    pub leaf_size: u64,
    pub hash_method: HashMethod, // Default is HashMethod::default()
}

impl MerkleTreeMetaData {
    pub fn leaf_count(&self) -> u64 {
        assert!(self.leaf_size > 0);

        let mut leaf_count = self.data_size / self.leaf_size;
        if self.data_size % self.leaf_size != 0 {
            leaf_count += 1;
        }

        leaf_count
    }

    pub fn estimate_output_bytes(&self) -> u64 {
        let meta_data = bincode::serialize(&self).unwrap();
        let meta_bytes = meta_data.len() + 4;

        meta_bytes as u64
    }

    pub async fn write(&self, body_writer: &mut Box<dyn MtreeWriteSeek>) -> NdnResult<usize> {
        let meta_data = bincode::serialize(&self).map_err(|e| {
            let msg = format!("Error serializing meta data: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        body_writer
            .write_all(&(meta_data.len() as u32).to_le_bytes())
            .await
            .map_err(|e| {
                let msg = format!("Error writing meta data length: {}", e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;
        body_writer.write_all(&meta_data).await.map_err(|e| {
            let msg = format!("Error writing meta data: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        Ok(meta_data.len() + 4)
    }

    pub async fn read(body_reader: &mut Box<dyn MtreeReadSeek>) -> NdnResult<(Self, usize)> {
        // Read the meta data from the reader, u32 length + data
        let mut meta_data = [0u8; 4];
        body_reader.read_exact(&mut meta_data).await.map_err(|e| {
            let msg = format!("Error reading meta data length: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let meta_len = u32::from_le_bytes(meta_data) as usize;
        let mut meta_data = vec![0u8; meta_len];
        body_reader.read_exact(&mut meta_data).await.map_err(|e| {
            let msg = format!("Error reading meta data: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let meta: MerkleTreeMetaData = bincode::deserialize(&meta_data).map_err(|e| {
            let msg = format!("Error deserializing meta data: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok((meta, meta_len + 4)) // 4 is the length of the meta data length
    }
}
