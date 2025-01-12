use crate::{NdnError, NdnResult};


#[derive(Debug, Clone)]
pub struct HashNode {
    pub hash: Vec<u8>,
    pub depth: u32, // The depth of the node in the tree, start from 0, and from bottom to top
    pub index: u64, // The hash index in current depth, start from 0, and from left to right
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

    pub fn depth(&self) -> u32 {
        self.total_depth
    }

    pub fn calc_total_count(leaf_count: u64) -> u64 {
        let counts = Self::calc_count_per_depth(leaf_count);
        counts.iter().sum()
    }

    pub fn total_count(&self) -> u64 {
        Self::calc_total_count(self.leaf_count)
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
    pub fn get_proof_path_by_leaf_index(&self, leaf_index: u64) -> NdnResult<Vec<(u32, u64)>> {
        if leaf_index >= self.leaf_count {
            let msg = format!(
                "Leaf index out of range: {} vs {}",
                leaf_index, self.leaf_count
            );
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        let mut ret = Vec::new();

        // First push the leaf node
        ret.push((0, leaf_index));

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
