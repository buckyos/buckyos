//对mix-hash提供原生支持
use super::calculator::SerializeHashCalculator;
use super::locator::HashNodeLocator;
use super::meta::*;
use super::stream::*;
use crate::hash::{HashHelper, HashMethod};
use crate::NdnError;
use crate::{NdnResult, ObjId, OBJ_TYPE_MTREE};
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

    pub fn get_obj_id(&self) -> ObjId {
        return ObjId::new_by_raw(OBJ_TYPE_MTREE.to_string(), self.root_hash.clone());
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
        for (i, item) in proof_path.iter().enumerate() {
            if i == proof_path.len() - 1 {
                // We reach the root node hash, so check if it is the same as the calculated root hash
                return Ok(current.as_ref() == Some(&item.1));
            }

            let (index, sibling_hash) = item;
            match current {
                Some(hash) => {
                    // println!("calc parent hash: {:?} {:?}", hash, sibling_hash);
                    let ret = if index % 2 == 0 {
                        self.calc_parent_hash(sibling_hash, &hash)
                    } else {
                        self.calc_parent_hash(&hash, sibling_hash)
                    };

                    current = Some(ret);
                }
                None => {
                    // The first one is the leaf node hash
                    current = Some(sibling_hash.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use http_types::{cache, proxies};
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;
    use tokio::test;

    #[test]
    async fn test_locator() {
        let total_depth = HashNodeLocator::calc_depth(6);
        println!("Total depth: {}", total_depth);
        assert!(total_depth == 3);

        let total_depth = HashNodeLocator::calc_depth(5);
        println!("Total depth: {}", total_depth);
        assert!(total_depth == 3);

        let total_depth = HashNodeLocator::calc_depth(4);
        println!("Total depth: {}", total_depth);
        assert!(total_depth == 2);

        let total_depth = HashNodeLocator::calc_depth(2);
        println!("Total depth: {}", total_depth);
        assert!(total_depth == 1);

        let counts = HashNodeLocator::calc_count_per_depth(3);
        println!("Counts: {:?}", counts);

        let counts = HashNodeLocator::calc_count_per_depth(6);
        println!("Counts: {:?}", counts);

        let counts = HashNodeLocator::calc_prev_count_per_depth(6);
        println!("Prev counts: {:?}", counts);

        let counts = HashNodeLocator::calc_count_per_depth(10);
        println!("Counts: {:?}", counts);

        let counts = HashNodeLocator::calc_prev_count_per_depth(10);
        println!("Prev counts: {:?}", counts);

        let locator = HashNodeLocator::new(6);
        let indexes = locator.get_proof_path_by_leaf_index(0).unwrap();
        println!("Indexes: {:?}", indexes);

        let indexes = locator.get_proof_path_by_leaf_index(5).unwrap();
        println!("Indexes: {:?}", indexes);

        assert_eq!(HashNodeLocator::calc_total_count(2), 3);
        assert_eq!(HashNodeLocator::calc_total_count(3), 7);
        assert_eq!(HashNodeLocator::calc_total_count(4), 7);
        assert_eq!(HashNodeLocator::calc_total_count(5), 13);
        assert_eq!(HashNodeLocator::calc_total_count(6), 13);

        assert_eq!(HashNodeLocator::calc_total_count(10), 23);
    }

    // Get file size and then calc leaf count of the file
    async fn get_leaf_count_of_file(file: &File, chunk_size: usize) -> u64 {
        // Get file size and then calc leaf count of the file
        let file_size = file.metadata().await.unwrap().len();
        assert!(file_size > 0);

        let mut leaf_count = file_size / chunk_size as u64;
        if file_size % chunk_size as u64 != 0 {
            leaf_count += 1;
        }

        println!("File size: {}, Leaf count: {}", file_size, leaf_count);
        leaf_count
    }

    async fn read_chunk(file: &mut File, chunk_size: usize) -> Vec<u8> {
        let mut buf = vec![0u8; chunk_size];

        let mut total_read = 0;
        while total_read < chunk_size {
            match file.read(&mut buf[total_read..]).await.unwrap() {
                0 => {
                    // EOF
                    break;
                }
                n => {
                    total_read += n;
                }
            }
        }

        // println!("Read {} bytes", total_read);
        // Truncate the buffer to the actual read size
        if total_read < chunk_size {
            buf.truncate(total_read);
        }

        buf
    }

    #[test]
    async fn test_generator() {
        let test_file: &str = "D:\\test";

        let chunk_size = 1024 * 1024 * 4;

        let stream;
        let root_hash;
        {
            let mut file = tokio::fs::File::open(test_file).await.unwrap();

            let data_size = file.metadata().await.unwrap().len();

            let total = MerkleTreeObjectGenerator::estimate_output_bytes(
                data_size,
                chunk_size,
                Some(HashMethod::Sha256),
            );
            println!("Estimated output bytes: {}", total);

            let buf = SharedBuffer::with_size(total as usize);
            stream = MtreeReadWriteSeekWithSharedBuffer::new(buf);
            let writer = Box::new(stream.clone()) as Box<dyn MtreeWriteSeek>;

            let mut gen = MerkleTreeObjectGenerator::new(
                data_size,
                chunk_size as u64,
                Some(HashMethod::Sha256),
                writer,
            )
            .await
            .unwrap();

            let leaf_count = get_leaf_count_of_file(&file, chunk_size as usize).await;
            let mut hash_list = Vec::new();
            loop {
                let buf = read_chunk(&mut file, chunk_size as usize).await;
                if buf.len() == 0 {
                    break;
                }

                let hash = sha2::Sha256::digest(&buf);
                hash_list.push(hash.to_vec());
            }

            assert!(hash_list.len() == leaf_count as usize);
            gen.append_leaf_hashes(&hash_list).await.unwrap();

            root_hash = gen.finalize().await.unwrap();
            println!("Root hash: {:?}", root_hash);
        }

        {
            // Create mtree object and load from buf previously
            let mut stream = stream.clone();
            stream.seek(SeekFrom::Start(0)).await.unwrap();
            let reader = Box::new(stream.clone()) as Box<dyn MtreeReadSeek>;
            let mut obj = MerkleTreeObject::load_from_reader(reader, true)
                .await
                .unwrap();

            let root_hash1 = obj.get_root_hash();
            println!("Root hash: {:?}", root_hash1);
            assert_eq!(root_hash, root_hash1);

            // Verify the proof path for the leaf node
            let proof_verify = MerkleTreeProofPathVerifier::new(HashMethod::Sha256);
            let mut proof = obj.get_proof_path_by_leaf_index(0).await.unwrap();

            // Proof last node must be the root node hash
            assert_eq!(proof[proof.len() - 1].1, root_hash);

            assert_eq!(proof_verify.verify(&proof).unwrap(), true);

            // Replace leaf node hash with error hash, then verify will failed!
            println!("Proof leaf node: {:?}", proof[0]);
            proof[0].1[0] = !proof[0].1[0];
            println!("Error proof leaf node: {:?}", proof[0]);
            assert_eq!(proof_verify.verify(&proof).unwrap(), false);
        }
    }

    #[test]
    async fn test_serialize_hash_calculator() {
        let test_file: &str = "D:\\test";

        let chunk_size = 1024 * 1024 * 4;

        let mut root_hash1;
        let mut root_hash2;
        {
            let mut file = tokio::fs::File::open(test_file).await.unwrap();
            let leaf_count = get_leaf_count_of_file(&file, chunk_size).await;

            // Read the file by chunk and calculate the leaf node hashes
            let mut calculator =
                SerializeHashCalculator::new(leaf_count, HashMethod::Sha256, None, None);
            let mut buf = vec![0u8; chunk_size];
            let mut hash_list = Vec::new();
            loop {
                let buf = read_chunk(&mut file, chunk_size).await;
                if buf.len() == 0 {
                    break;
                }

                let hash = sha2::Sha256::digest(&buf);
                hash_list.push(hash.to_vec());
            }

            assert!(hash_list.len() == leaf_count as usize);
            calculator.append_leaf_hashes(&hash_list).await.unwrap();
            root_hash1 = calculator.finalize().await.unwrap();
            println!("Root hash: {:?}", root_hash1);
        }

        {
            let mut file = tokio::fs::File::open(test_file).await.unwrap();
            let leaf_count = get_leaf_count_of_file(&file, chunk_size).await;

            let size =
                SerializeHashCalculator::estimate_output_bytes(leaf_count, HashMethod::Sha256)
                    as usize;

            let data = SharedBuffer::with_size(size);
            let buffer = MtreeReadWriteSeekWithSharedBuffer::new(data);

            let mut writer = Box::new(buffer.clone()) as Box<dyn MtreeWriteSeek>;

            // Read the file by chunk and calculate the leaf node hashes
            let mut calculator =
                SerializeHashCalculator::new(leaf_count, HashMethod::Sha256, Some(writer), None);
            let mut buf = vec![0u8; chunk_size];

            loop {
                let buf = read_chunk(&mut file, chunk_size).await;
                if buf.len() == 0 {
                    break;
                }

                let hash = sha2::Sha256::digest(&buf);
                calculator
                    .append_leaf_hashes(&vec![hash.to_vec()])
                    .await
                    .unwrap();
            }

            root_hash2 = calculator.finalize().await.unwrap();
            println!("Root hash: {:?}", root_hash2);

            // print the whole buffer
            // println!("Buffer: {:?}", buffer.buffer().lock().unwrap());

            // Clone the buf from writer and then create a reader from it
            let mut reader = Box::new(buffer) as Box<dyn MtreeReadSeek>;
            reader.seek(SeekFrom::Start(0)).await.unwrap();

            // Calc with reader verify
            let mut calculator =
                SerializeHashCalculator::new(leaf_count, HashMethod::Sha256, None, Some(reader));
            let mut buf = vec![0u8; chunk_size];

            // Read the file at beginning
            file.seek(SeekFrom::Start(0)).await.unwrap();
            loop {
                let buf = read_chunk(&mut file, chunk_size).await;
                if buf.len() == 0 {
                    break;
                }

                let hash = sha2::Sha256::digest(&buf);
                calculator
                    .append_leaf_hashes(&vec![hash.to_vec()])
                    .await
                    .unwrap();
            }

            let root_hash3 = calculator.finalize().await.unwrap();
            assert_eq!(root_hash2, root_hash3);
            // println!("Root hash: {:?}", root_hash3);
        }

        assert_eq!(root_hash1, root_hash2);
    }
}
