//对mix-hash提供原生支持
use super::hash::HashMethod;
use super::mtree_stream::*;
use crate::NdnError;
use crate::{NdnResult, ObjId, OBJ_TYPE_MTREE};
use core::{error, hash};
use futures::stream;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::fmt::format;
use std::hash::Hash;
use std::io::{Read, Seek, SeekFrom};
use std::pin::Pin;
use std::str::FromStr;
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

// mtree object version, begin at 0
pub const OBJ_MTREE_VERSION: u8 = 0;

//meta data of the mtree object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTreeMetaData {
    data_size: u64,
    leaf_size: u64,
    hash_method: HashMethod, // Default is HashMethod::default()
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
}

pub struct MerkleTreeObject {
    meta: MerkleTreeMetaData,
    root_hash: Vec<u8>,
    body_reader: Box<dyn MtreeReadSeek>,
    locator: HashNodeLocator,
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
    pub async fn load_from_reader(mut body_reader: Box<dyn MtreeReadSeek>) -> NdnResult<Self> {
        // Read the meta data from the reader
        let mut meta = Self::load_meta_from_reader(&mut body_reader).await?;

        // Load leaf hash lsit and calc the root hash from the reader
        let (root_hash, locator) =
            Self::load_leaf_hash_list_from_reader(&mut body_reader, &meta).await?;

        Ok(Self {
            meta,
            root_hash,
            body_reader: body_reader,
            locator: locator,
        })
    }

    async fn load_meta_from_reader(
        body_reader: &mut Box<dyn MtreeReadSeek>,
    ) -> NdnResult<MerkleTreeMetaData> {
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

        Ok(meta)
    }

    async fn load_leaf_hash_list_from_reader(
        body_reader: &mut Box<dyn MtreeReadSeek>,
        meta: &MerkleTreeMetaData,
    ) -> NdnResult<(Vec<u8>, HashNodeLocator)> {
        assert!(meta.leaf_size > 0);

        let mut leaf_count = meta.leaf_count();
        let hash_bytes = meta.hash_method.hash_bytes();

        let mut calc = SerializeHashCalculator::new(leaf_count, meta.hash_method, None, None);

        for _ in 0..leaf_count {
            let mut hash = vec![0u8; hash_bytes];
            let len = body_reader.read_exact(&mut hash).await.map_err(|e| {
                let msg = format!("Error reading leaf hash: {}", e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;

            assert!(len == hash_bytes);

            calc.append_leaf_hashes(&vec![hash]).await?;
        }

        let root_hash = calc.finalize().await?;

        Ok((root_hash, calc.into_locator()))
    }

    //result is a map, key is the index of the leaf, value is the hash of the leaf
    pub async fn get_verify_path_by_leaf_index(
        &mut self,
        leaf_index: u64,
    ) -> NdnResult<Vec<(u64, Vec<u8>)>> {
        let indexes = self.locator.get_verify_path_by_leaf_index(leaf_index)?;

        let hash_bytes = self.meta.hash_method.hash_bytes();

        let mut ret = Vec::with_capacity(indexes.len());
        for (_depth, index) in indexes {
            let pos = hash_bytes as u64 * index;
            self.body_reader
                .seek(SeekFrom::Start(pos))
                .await
                .map_err(|e| {
                    let msg = format!("Error seeking to position {}: {}", index, e);
                    error!("{}", msg);
                    NdnError::IoError(msg)
                })?;

            let mut hash = vec![0u8; hash_bytes];
            self.body_reader.read_exact(&mut hash).await.map_err(|e| {
                let msg = format!("Error reading hash: {}", e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;

            ret.push((index, hash));
        }

        Ok(ret)
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

pub struct MerkleTreeObjectGenerator {
    meta: MerkleTreeMetaData,

    calc: SerializeHashCalculator,
}

impl MerkleTreeObjectGenerator {
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
        let write_bytes = Self::write_meta_data(&mut body_writer, &meta).await?;

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

    async fn write_meta_data(
        body_writer: &mut Box<dyn MtreeWriteSeek>,
        meta: &MerkleTreeMetaData,
    ) -> NdnResult<usize> {
        let meta_data = bincode::serialize(&meta).map_err(|e| {
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

    pub async fn append_leaf_hashes(&mut self, leaf_hashes: &Vec<Vec<u8>>) -> NdnResult<()> {
        self.calc.append_leaf_hashes(&leaf_hashes).await
    }

    pub async fn finalize(&mut self) -> NdnResult<Vec<u8>> {
        let root_hash = self.calc.finalize().await?;

        Ok(root_hash)
    }
}

#[derive(Debug, Clone)]
struct HashNode {
    hash: Vec<u8>,
    depth: u32, // The depth of the node in the tree, start from 0, and from bottom to top
    index: u64, // The hash index in current depth, start from 0, and from left to right
}

pub struct HashNodeLocator {
    // The total leaf count of the tree
    leaf_count: u64,

    // The total depth of the tree, start from 0, and from bottom to top
    total_depth: u32,

    // The prev count of nodes in previous depth, from bottom to top
    prev_count_per_depth: Vec<u64>,
}

impl HashNodeLocator {
    pub fn new(leaf_count: u64) -> Self {
        Self {
            leaf_count,
            total_depth: Self::calc_depth(leaf_count),
            prev_count_per_depth: Self::calc_prev_count_per_depth(leaf_count),
        }
    }

    pub fn total_depth(&self) -> u32 {
        self.total_depth
    }

    // Start at zero, and from top to bottom
    pub fn calc_depth(leaf_count: u64) -> u32 {
        (leaf_count as f64).log2().ceil() as u32
    }

    pub fn calc_total_count(leaf_count: u64) -> u64 {
        let counts = Self::calc_count_per_depth(leaf_count);
        counts.iter().sum()
    }

    pub fn calc_count_per_depth(leaf_count: u64) -> Vec<u64> {
        let total_depth = Self::calc_depth(leaf_count);
        let mut count_per_depth = Vec::with_capacity(total_depth as usize + 1);
        let mut count = leaf_count;
        for i in 0..total_depth + 1 {
            if i != total_depth {
                // If the count is odd, we should make it even, expect the root node
                if count % 2 != 0 {
                    count += 1;
                }
            }

            count_per_depth.push(count);

            count = count / 2;
        }

        assert!(count_per_depth[total_depth as usize] == 1);
        count_per_depth
    }

    pub fn calc_prev_count_per_depth(leaf_count: u64) -> Vec<u64> {
        let counts = Self::calc_count_per_depth(leaf_count);
        let prev_counts = counts
            .iter()
            .scan(0, |state, &x| {
                let ret = *state;
                *state += x;
                Some(ret)
            })
            .collect();

        prev_counts
    }

    // Depth start from 0, and from bottom to top
    // Index start from 0, and from left to right
    pub fn calc_index_in_stream(&self, depth: u32, index: u64) -> u64 {
        assert!(depth <= self.total_depth);
        self.prev_count_per_depth[depth as usize] + index
    }

    // Get the verify path of the leaf node by the leaf index
    // The result is a vector of (depth, index) tuple, depth start 0, and from bottom to top
    // Index is the index of the node node in the stream, start from 0, and from left to right
    pub fn get_verify_path_by_leaf_index(&self, leaf_index: u64) -> NdnResult<Vec<(u32, u64)>> {
        if leaf_index >= self.leaf_count {
            let msg = format!(
                "Leaf index out of range: {} vs {}",
                leaf_index, self.leaf_count
            );
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        let mut ret = Vec::new();
        let mut index = leaf_index;
        for depth in 0..self.total_depth {
            // Get sibling index of the node in the current depth
            let sibling_index = if index % 2 == 0 { index + 1 } else { index - 1 };
            let stream_index = self.calc_index_in_stream(depth, sibling_index);
            ret.push((depth, stream_index));

            index = index / 2;
        }

        // Finally, add the root node
        let stream_index = self.calc_index_in_stream(self.total_depth, 0);
        ret.push((self.total_depth, stream_index));

        Ok(ret)
    }
}

pub struct SerializeHashCalculator {
    hash_method: HashMethod,

    // Record the total leaf count and pending leaf count
    leaf_count: u64,
    append_count: u64,

    // Used to locate the node in the stream, from bottom to top, from left to right
    locator: HashNodeLocator,

    stack: VecDeque<HashNode>,

    writer: Option<Box<dyn MtreeWriteSeek>>,
    reader: Option<Box<dyn MtreeReadSeek>>,

    #[cfg_attr(debug_assertions, allow(dead_code))]
    writer_tracker: HashSet<u64>,
}

impl SerializeHashCalculator {
    pub fn new(
        leaf_count: u64,
        hash_method: HashMethod,
        writer: Option<Box<dyn MtreeWriteSeek>>, // Used to save the hash
        reader: Option<Box<dyn MtreeReadSeek>>,  // Used to verify the hash
    ) -> Self {
        let total_depth = (leaf_count as f64).log2().ceil() as u32;
        Self {
            hash_method,
            leaf_count,
            append_count: 0,
            locator: HashNodeLocator::new(leaf_count),
            stack: VecDeque::new(),
            writer,
            reader,

            #[cfg_attr(debug_assertions, allow(dead_code))]
            writer_tracker: HashSet::new(),
        }
    }

    pub fn estimate_output_bytes(leaf_count: u64, hash_method: HashMethod) -> u64 {
        HashNodeLocator::calc_total_count(leaf_count) * HashMethod::Sha256.hash_bytes() as u64
    }

    pub fn get_leaf_count(&self) -> u64 {
        self.leaf_count
    }

    pub fn get_append_count(&self) -> u64 {
        self.append_count
    }

    pub fn into_locator(self) -> HashNodeLocator {
        self.locator
    }

    pub fn detach_writer(&mut self) -> Option<Box<dyn MtreeWriteSeek>> {
        self.writer.take()
    }

    pub fn detach_reader(&mut self) -> Option<Box<dyn MtreeReadSeek>> {
        self.reader.take()
    }

    async fn verify_hash(&mut self, index: usize, hash: &Vec<u8>) -> NdnResult<()> {
        assert!(self.reader.is_some());

        // println!("Write hash to index: {}, {:?}", index, hash);
        let reader = self.reader.as_mut().unwrap();
        let pos = (index * self.hash_method.hash_bytes()) as u64;
        reader.seek(SeekFrom::Start(pos)).await.map_err(|e| {
            let msg = format!("Error seeking to position {}: {}", pos, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let mut read_hash = vec![0u8; self.hash_method.hash_bytes()];
        reader.read_exact(&mut read_hash).await.map_err(|e| {
            let msg = format!("Error reading hash: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        if read_hash != *hash {
            let msg = format!("Hash not match: {} {:?} vs {:?}", index, read_hash, hash);
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        Ok(())
    }

    async fn write_hash(&mut self, index: usize, hash: &Vec<u8>) -> NdnResult<()> {
        assert!(self.writer.is_some());

        println!("Write hash to index: {}, {:?}", index, hash);
        #[cfg(debug_assertions)]
        {
            assert!(!self.writer_tracker.contains(&(index as u64)));
            self.writer_tracker.insert(index as u64);
        }

        // println!("Write hash to index: {}, {:?}", index, hash);
        let writer = self.writer.as_mut().unwrap();
        let pos = (index * self.hash_method.hash_bytes()) as u64;
        writer.seek(SeekFrom::Start(pos)).await.map_err(|e| {
            let msg = format!("Error seeking to position {}: {}", pos, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        writer.write_all(hash).await.map_err(|e| {
            let msg = format!("Error writing hash: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        Ok(())
    }

    async fn write_node(&mut self, node: &HashNode) -> NdnResult<()> {
        let index = self.locator.calc_index_in_stream(node.depth, node.index);

        if self.writer.is_some() {
            self.write_hash(index as usize, &node.hash).await?;
        }

        if self.reader.is_some() {
            self.verify_hash(index as usize, &node.hash).await?;
        }

        Ok(())
    }

    pub async fn append_leaf_hashes(&mut self, leaf_hashes: &Vec<Vec<u8>>) -> NdnResult<()> {
        if leaf_hashes.len() as u64 + self.append_count > self.leaf_count {
            let msg = format!(
                "Leaf count out of range: {} + {} > {}",
                leaf_hashes.len(),
                self.append_count,
                self.leaf_count
            );
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        for hash in leaf_hashes {
            let node = HashNode {
                hash: hash.clone(),
                depth: 0,
                index: self.append_count as u64,
            };
            self.stack.push_back(node);

            self.append_count += 1;
            assert!(self.append_count <= self.leaf_count);

            // If the last two nodes have the same depth, merge them
            self.check_and_merge().await?;
        }

        Ok(())
    }

    async fn check_and_merge(&mut self) -> NdnResult<()> {
        while self.stack.len() > 1 {
            let node1 = self.stack.get(self.stack.len() - 1).unwrap();
            let node2 = self.stack.get(self.stack.len() - 2).unwrap();
            if node1.depth == node2.depth {
                let node1 = self.stack.pop_back().unwrap();
                let node2 = self.stack.pop_back().unwrap();
                let parent_hash = self.calc_parent_hash(&node1.hash, &node2.hash);
                self.stack.push_back(HashNode {
                    hash: parent_hash.clone(),
                    depth: node1.depth + 1,
                    index: node1.index / 2,
                });

                // Save node1 and node2 to the writer
                self.write_node(&node1).await?;
                self.write_node(&node2).await?;
            } else {
                break;
            }
        }

        Ok(())
    }

    // Should be called after all leaf nodes are appended
    pub async fn finalize(&mut self) -> NdnResult<Vec<u8>> {
        if self.stack.len() == 0 {
            let msg = "No leaf node appended".to_string();
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        // We should have exactly leaf_count leaf nodes appended
        if self.leaf_count != self.append_count {
            let msg = format!(
                "Leaf count not match: {} vs {}",
                self.leaf_count, self.append_count
            );
            error!("{}", msg);
            return Err(NdnError::InvalidState(msg));
        }

        // Merge the node in same depth, if there is only one node left in the same depth, we should copy it once more then merge with itself
        loop {
            if self.stack.len() == 1 {
                break;
            }

            let node1 = self.stack.pop_back().unwrap();

            // Check if the last node is same depth with node1
            let node2 = if self.stack.back().unwrap().depth == node1.depth {
                self.stack.pop_back().unwrap()
            } else {
                // Clone the node1 and set the index to the next on the right
                let mut node2 = node1.clone();
                node2.index = node1.index + 1;

                node2
            };

            let parent_hash = self.calc_parent_hash(&node1.hash, &node2.hash);
            self.stack.push_back(HashNode {
                hash: parent_hash.clone(),
                depth: node1.depth + 1,
                index: node1.index / 2,
            });

            // Save node1 and node2 to the writer
            self.write_node(&node1).await?;
            self.write_node(&node2).await?;
        }

        assert_eq!(self.stack.len(), 1);
        let root = self.stack.pop_back().unwrap();
        assert!(root.depth == self.locator.total_depth());
        assert!(root.index == 0);

        // Save the root hash to the writer
        self.write_node(&root).await?;

        // At last, we should have check if all nodes in the writer
        #[cfg(debug_assertions)]
        if self.writer.is_some() {
            let total = HashNodeLocator::calc_total_count(self.leaf_count);
            assert_eq!(self.writer_tracker.len(), total as usize);
            for i in 0..total {
                assert!(self.writer_tracker.contains(&(i as u64)));
            }
        }

        Ok(root.hash)
    }

    fn calc_parent_hash(&self, left: &[u8], right: &[u8]) -> Vec<u8> {
        match self.hash_method {
            HashMethod::Sha256 => {
                let mut hasher = sha2::Sha256::new();
                hasher.update(left);
                hasher.update(right);
                hasher.finalize().to_vec()
            }
            HashMethod::Sha512 => {
                let mut hasher = sha2::Sha512::new();
                hasher.update(left);
                hasher.update(right);
                hasher.finalize().to_vec()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_types::cache;
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
        let indexes = locator.get_verify_path_by_leaf_index(0).unwrap();
        println!("Indexes: {:?}", indexes);

        let indexes = locator.get_verify_path_by_leaf_index(5).unwrap();
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

        let chunk_size = 1024 * 4;

        {
            let mut file = tokio::fs::File::open(test_file).await.unwrap();

            let data_size = file.metadata().await.unwrap().len();

            let total = MerkleTreeObjectGenerator::estimate_output_bytes(
                data_size,
                chunk_size,
                Some(HashMethod::Sha256),
            );
            println!("Estimated output bytes: {}", total);

            let mut buf = vec![0u8; total as usize];
            let mut writer = std::io::Cursor::new(buf);
            let mut writer = Box::new(writer) as Box<dyn MtreeWriteSeek>;

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
            let root_hash1 = gen.finalize().await.unwrap();
            println!("Root hash: {:?}", root_hash1);
        }

        {}
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
