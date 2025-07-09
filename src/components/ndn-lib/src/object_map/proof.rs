use super::object_map::{ObjectMapBody, ObjectMapItem};
use crate::mtree::MerkleTreeProofPathVerifier;
use crate::{Base32Codec, HashMethod, NdnError, NdnResult};

#[derive(Debug, Clone)]
pub struct ObjectMapItemProof {
    pub item: ObjectMapItem,
    pub proof: Vec<(u64, Vec<u8>)>,
}

impl ObjectMapItemProof {
    pub fn get_leaf_node(&self) -> NdnResult<(u64, Vec<u8>)> {
        if self.proof.is_empty() {
            return Err(NdnError::InvalidData("Proof is empty".to_string()));
        }

        // The first item is the leaf node
        Ok(self.proof[0].clone())
    }

    pub fn get_root_node(&self) -> NdnResult<(u64, Vec<u8>)> {
        if self.proof.is_empty() {
            return Err(NdnError::InvalidData("Proof is empty".to_string()));
        }

        // The last item is the root node
        Ok(self.proof[self.proof.len() - 1].clone())
    }
}

pub struct ObjectMapProofVerifier {
    hash_method: HashMethod,
}

impl ObjectMapProofVerifier {
    pub fn new(hash_method: HashMethod) -> Self {
        Self { hash_method }
    }

    pub fn verify_with_obj_data_str(
        &self,
        obj_data: &str,
        proof: &ObjectMapItemProof,
    ) -> NdnResult<bool> {
        // Parse the object data as JSON
        let body: ObjectMapBody = serde_json::from_str(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let root_hash = body.root_hash;
        self.verify(&root_hash, proof)
    }

    pub fn verify_with_obj_data(
        &self,
        obj_data: serde_json::Value,
        proof: &ObjectMapItemProof,
    ) -> NdnResult<bool> {
        // Get the root hash from the object data
        let body: ObjectMapBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let root_hash = body.root_hash;
        self.verify(&root_hash, proof)
    }

    // root_hash is the object map's root hash, which is the body.root_hash field, encoded as base32
    // proof is the ObjectMapItemProof, which contains the item and the proof path
    pub fn verify(&self, root_hash: &str, proof: &ObjectMapItemProof) -> NdnResult<bool> {
        if proof.proof.len() < 2 {
            let msg = format!("Invalid proof path length: {}", proof.proof.len());
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        // First calculate the hash of the item
        let item_hash = proof.item.calc_hash(self.hash_method);

        // The first item is the leaf node, which is the item itself
        if proof.proof[0].1 != item_hash {
            let msg = format!(
                "Unmatched object map leaf hash: expected {:?}, got {:?}",
                item_hash, proof.proof[0].1
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        let root_hash = Base32Codec::from_base32(root_hash).map_err(|e| {
            let msg = format!("Error decoding root hash: {}, {}", root_hash, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        // The last item is the root node, which is the expected root hash
        if proof.proof[proof.proof.len() - 1].1 != root_hash {
            let msg = format!(
                "Unmatched object map root hash: expected {:?}, got {:?}",
                root_hash,
                proof.proof[proof.proof.len() - 1].1
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        let mtree_verifier = MerkleTreeProofPathVerifier::new(self.hash_method);
        mtree_verifier.verify(&proof.proof)
    }
}
