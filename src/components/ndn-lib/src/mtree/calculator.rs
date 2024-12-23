use super::locator::{HashNode, HashNodeLocator};
use super::stream::{MtreeReadSeek, MtreeWriteSeek};
use crate::hash::{HashHelper, HashMethod};
use crate::{NdnError, NdnResult};
use std::collections::{HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct SerializeHashCalculator {
    hash_method: HashMethod,

    // Record the total leaf count and pending leaf count
    leaf_count: u64,
    append_count: u64,

    // Used to locate the node in the stream, from bottom to top, from left to right
    locator: HashNodeLocator,

    // Used to calculate the hash of all the nodes
    stack: VecDeque<HashNode>,

    // If set, used to save the hash when append leaf nodes or load leaf nodes from reader
    writer: Option<Box<dyn MtreeWriteSeek>>,

    // If set, used to verify the hash when append leaf nodes or load leaf nodes from reader
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

    // Get root hash from reader, the reader must be set
    pub async fn get_root_hash(&mut self) -> NdnResult<Vec<u8>> {
        assert!(self.reader.is_some());

        // The root hash is the last hash in the reader
        let reader = self.reader.as_mut().unwrap();
        let hash_bytes = self.hash_method.hash_bytes();
        let pos = (self.leaf_count - 1) * hash_bytes as u64;
        reader.seek(SeekFrom::Start(pos)).await.map_err(|e| {
            let msg = format!("Error seeking to position {}: {}", pos, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        let mut hash = vec![0u8; hash_bytes];
        reader.read_exact(&mut hash).await.map_err(|e| {
            let msg = format!("Error reading hash: {}", e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        Ok(hash)
    }

    // Get proof path of the leaf node by the leaf index, the reader must be set
    pub async fn get_proof_path_by_leaf_index(
        &mut self,
        leaf_index: u64,
    ) -> NdnResult<Vec<(u64, Vec<u8>)>> {
        assert!(self.reader.is_some());

        let indexes = self.locator.get_proof_path_by_leaf_index(leaf_index)?;

        let reader = self.reader.as_mut().unwrap();
        let hash_bytes = self.hash_method.hash_bytes();

        let mut ret = Vec::with_capacity(indexes.len());
        for (_depth, index) in indexes {
            let pos = hash_bytes as u64 * index;
            reader.seek(SeekFrom::Start(pos)).await.map_err(|e| {
                let msg = format!("Error seeking to position {}: {}", index, e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;

            let mut hash = vec![0u8; hash_bytes];
            reader.read_exact(&mut hash).await.map_err(|e| {
                let msg = format!("Error reading hash: {}", e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;

            ret.push((index, hash));
        }

        Ok(ret)
    }

    // Load all leaf hashed from reader, then append the leaf hashes to the writer or verifier
    pub async fn load_leaf_hashes_from_reader(&mut self) -> NdnResult<Vec<u8>> {
        assert!(self.reader.is_some());

        // Load leaf node hash from the reader, start from index 0
        let hash_bytes = self.hash_method.hash_bytes();
        for i in 0..self.leaf_count {
            // Must seek to the right position before read, because the reader may be used for verify on append leaf nodes!
            let pos = hash_bytes as u64 * i;
            self.reader
                .as_mut()
                .unwrap()
                .seek(SeekFrom::Start(pos))
                .await
                .map_err(|e| {
                    let msg = format!("Error seeking to position {}: {}", pos, e);
                    error!("{}", msg);
                    NdnError::IoError(msg)
                })?;

            let mut hash = vec![0u8; hash_bytes];
            let len = self
                .reader
                .as_mut()
                .unwrap()
                .read_exact(&mut hash)
                .await
                .map_err(|e| {
                    let msg = format!("Error reading leaf hash: {}", e);
                    error!("{}", msg);
                    NdnError::IoError(msg)
                })?;

            assert!(len == hash_bytes);

            self.append_leaf_hashes(&vec![hash]).await?;
        }

        let root_hash = self.finalize().await?;

        Ok(root_hash)
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
        assert!(hash.len() == self.hash_method.hash_bytes());

        // println!("Write hash to index: {}, {:?}", index, hash);
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

    // Append leaf hashes to the writer or verifier, the hash length must match the hash method!
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
            assert!(hash.len() == self.hash_method.hash_bytes());

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
            let node2 = self.stack.get(self.stack.len() - 1).unwrap();
            let node1 = self.stack.get(self.stack.len() - 2).unwrap();
            if node1.depth == node2.depth {
                let node2 = self.stack.pop_back().unwrap();
                let node1 = self.stack.pop_back().unwrap();
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
            let mut node2;

            // Check if the last node is same depth with node1
            let parent_hash = if self.stack.back().unwrap().depth == node1.depth {
                node2 = self.stack.pop_back().unwrap();
                self.calc_parent_hash(&node2.hash, &node1.hash)
            } else {
                // Clone the node1 and set the index to the next on the right
                node2 = node1.clone();
                node2.index = node1.index + 1;

                self.calc_parent_hash(&node1.hash, &node2.hash)
            };

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
        HashHelper::calc_parent_hash(self.hash_method, left, right)
    }
}
