//对mix-hash提供原生支持
use super::calculator::SerializeHashCalculator;
use super::locator::HashNodeLocator;
use super::meta::*;
use super::stream::*;
use crate::hash::{HashHelper, HashMethod};
use crate::NdnError;
use crate::{NdnResult, ObjId};
use core::{error, hash};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct MerkleTreeObject {
    meta: MerkleTreeMetaData,
    root_hash: Vec<u8>,
    calculator: SerializeHashCalculator,
}

impl MerkleTreeObject {
    /*
    pub fn new(
        data_size: u64,
        leaf_size: u64,
        body_reader: Box<dyn MtreeReadSeek>,
        hash_type: Option<String>,
    ) -> Self {
        assert!(leaf_size > 0);

        Self {
            meta: MerkleTreeMetaData {
                data_size,
                leaf_size,
                hash_type,
            },
            body_reader: None,
        }
    }
    */

    // Init from reader, reader is a reader of the body of the mtree
    pub async fn load_from_reader(
        mut body_reader: Box<dyn MtreeReadSeek>,
        verify: bool,
    ) -> NdnResult<Self> {
        // Read the meta data from the reader
        let (mut meta, len) = MerkleTreeMetaData::read(&mut body_reader).await?;

        // Create nodes reader with offset = len
        let nodes_reader = Box::new(MtreeReadSeekWithOffset::new(body_reader, len as u64))
            as Box<dyn MtreeReadSeek>;

        let mut calculator = SerializeHashCalculator::new(
            meta.leaf_count(),
            meta.hash_method,
            None,
            Some(nodes_reader),
        );

        let root_hash = if verify {
            // Load leaf hash list and calc the root hash from the reader
            calculator.load_leaf_hashes_from_reader().await?
        } else {
            calculator.get_root_hash().await?
        };

        Ok(Self {
            meta,
            root_hash,
            calculator,
        })
    }

    //result is a map, key is the index of the leaf, value is the hash of the leaf
    pub async fn get_proof_path_by_leaf_index(
        &mut self,
        leaf_index: u64,
    ) -> NdnResult<Vec<(u64, Vec<u8>)>> {
        self.calculator
            .get_proof_path_by_leaf_index(leaf_index)
            .await
    }

    pub fn get_hash_method(&self) -> HashMethod {
        self.meta.hash_method
    }

    pub fn get_leaf_count(&self) -> u64 {
        self.meta.leaf_count()
    }

    pub fn get_leaf_size(&self) -> u64 {
        self.meta.leaf_size
    }

    pub fn get_root_hash(&self) -> Vec<u8> {
        self.root_hash.clone()
    }

    pub fn get_data_size(&self) -> u64 {
        self.meta.data_size
    }
}

pub struct MerkleTreeProofPathVerifier {
    hash_method: HashMethod,
}

impl MerkleTreeProofPathVerifier {
    pub fn new(hash_method: HashMethod) -> Self {
        Self { hash_method }
    }

    pub fn verify(&self, proof_path: &Vec<(u64, Vec<u8>)>) -> NdnResult<bool> {
        // println!("verify proof path: {:?}", proof_path);

        // The first one is the leaf node hash, and the last one is the root node hash
        if proof_path.len() < 2 {
            let msg = format!("Invalid proof path length: {}", proof_path.len());
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        let mut current = None;
        let mut current_leaf_index = 0;
        for (i, item) in proof_path.iter().enumerate() {
            if i == proof_path.len() - 1 {
                if current_leaf_index != 0 {
                    let msg = format!(
                        "Invalid proof path: current_leaf_index should be 0, but got {}",
                        current_leaf_index
                    );
                    warn!("{}", msg);
                    return Ok(false);
                }

                // We reach the root node hash, so check if it is the same as the calculated root hash
                return Ok(current.as_ref() == Some(&item.1));
            }

            let (index, sibling_hash) = item;
            match current {
                Some(hash) => {
                    // println!("calc parent hash: {:?} {:?}", hash, sibling_hash);
                    let mut silbing_index = 0;
                    let ret = if index % 2 == 0 {
                        silbing_index = index + 1;
                        self.calc_parent_hash(sibling_hash, &hash)
                    } else {
                        silbing_index = index - 1;
                        self.calc_parent_hash(&hash, sibling_hash)
                    };

                    if silbing_index != current_leaf_index {
                        let msg = format!(
                            "Sibling index mismatch: {} vs {}",
                            silbing_index, current_leaf_index
                        );
                        warn!("{}", msg);
                        return Ok(false);
                    }

                    current = Some(ret);
                    current_leaf_index = current_leaf_index / 2;
                }
                None => {
                    // The first one is the leaf node hash
                    current = Some(sibling_hash.clone());
                    current_leaf_index = *index;
                }
            }
        }

        unreachable!();
    }

    fn calc_parent_hash(&self, left: &Vec<u8>, right: &Vec<u8>) -> Vec<u8> {
        let hash = HashHelper::calc_parent_hash(self.hash_method, left, right);
        // println!("calc parent hash: {:?} {:?} -> {:?}", left, right, hash);
        hash
    }
}

pub struct MerkleTreeObjectGenerator {
    meta: MerkleTreeMetaData,

    calc: SerializeHashCalculator,
}

impl MerkleTreeObjectGenerator {
    pub fn into_writer(mut self) -> Box<dyn MtreeWriteSeek> {
        self.calc.detach_writer().unwrap()
    }

    pub async fn new(
        data_size: u64,
        leaf_size: u64,
        hash_method: Option<HashMethod>,
        mut body_writer: Box<dyn MtreeWriteSeek>,
    ) -> NdnResult<Self> {
        assert!(leaf_size > 0);

        let hash_method = hash_method.unwrap_or_default();

        let meta = MerkleTreeMetaData {
            data_size,
            leaf_size,
            hash_method,
        };
        info!("New mtree with meta data: {:?}", meta);

        // Record the stream position of the meta data
        let meta_pos = body_writer.seek(SeekFrom::Current(0)).await.map_err(|e| {
            let msg = format!("Error getting current position: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        // First write the meta data to the writer
        let write_bytes = meta.write(&mut body_writer).await?;

        // Reseek to the start before the meta data
        body_writer
            .seek(SeekFrom::Start(meta_pos))
            .await
            .map_err(|e| {
                let msg = format!("Error seeking to position {}: {}", meta_pos, e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;

        // Make new stream for the body writer
        let writer = MtreeWriteSeekWithOffset::new(body_writer, meta_pos + write_bytes as u64);
        let body_writer = Box::new(writer) as Box<dyn MtreeWriteSeek>;

        let calc =
            SerializeHashCalculator::new(meta.leaf_count(), hash_method, Some(body_writer), None);

        let mut ret = Self { meta, calc };

        Ok(ret)
    }

    pub fn estimate_output_bytes(
        data_size: u64,
        leaf_size: u64,
        hash_method: Option<HashMethod>,
    ) -> u64 {
        let hash_method = hash_method.unwrap_or_default();
        let meta = MerkleTreeMetaData {
            data_size,
            leaf_size,
            hash_method,
        };

        let meta_data = bincode::serialize(&meta).unwrap();
        let meta_bytes = meta_data.len() + 4;

        let body_bytes =
            SerializeHashCalculator::estimate_output_bytes(meta.leaf_count(), hash_method);

        meta_bytes as u64 + body_bytes
    }

    pub async fn append_leaf_hashes(&mut self, leaf_hashes: &Vec<Vec<u8>>) -> NdnResult<()> {
        self.calc.append_leaf_hashes(&leaf_hashes).await
    }

    pub async fn finalize(&mut self) -> NdnResult<Vec<u8>> {
        let root_hash = self.calc.finalize().await?;

        Ok(root_hash)
    }
}