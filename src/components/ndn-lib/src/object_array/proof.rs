use super::object_array::ObjectArrayBody;
use crate::mtree::MerkleTreeProofPathVerifier;
use crate::{get_obj_hash, Base32Codec, HashMethod, NdnError, NdnResult, ObjId};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::to_string;

#[derive(Debug, Clone)]
pub struct ObjectArrayItemProof {
    pub proof: Vec<(u64, Vec<u8>)>,
}

impl ObjectArrayItemProof {
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
// Proof nodes in base64 string format
#[derive(Serialize, Deserialize)]
struct ProofNode {
    i: String,
    v: String,
}

#[derive(Serialize, Deserialize)]
struct ProofNodes {
    nodes: Vec<ProofNode>,
}

pub struct ObjectArrayItemProofCodec {}

impl ObjectArrayItemProofCodec {
    pub fn encode(proof: &ObjectArrayItemProof) -> NdnResult<String> {
        let mut ret = Vec::with_capacity(proof.proof.len());

        // Encode each proof node to base64 string
        for item in proof.proof.iter() {
            let s = BASE64_STANDARD.encode(&item.1);
            ret.push(ProofNode {
                i: item.0.to_string(),
                v: s,
            });
        }

        // Encode to json string
        let ret = serde_json::to_string(&ret).map_err(|e| {
            let msg = format!("Error serializing ProofNodes: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(ret)
    }

    pub fn decode(proof: &str) -> NdnResult<ObjectArrayItemProof> {
        let mut ret = Vec::new();

        // First decode from json string
        let nodes: Vec<ProofNode> = serde_json::from_str(proof).map_err(|e| {
            let msg = format!("Error deserializing ProofNodes: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        for node in nodes {
            let v = BASE64_STANDARD.decode(node.v).map_err(|e| {
                let msg = format!("Error decoding base64 string: {}", e);
                error!("{}", msg);
                NdnError::InvalidData(msg)
            })?;

            let i = node.i.parse::<u64>().map_err(|e| {
                let msg = format!("Error parsing index: {}", e);
                error!("{}", msg);
                NdnError::InvalidData(msg)
            })?;

            ret.push((i, v));
        }

        Ok(ObjectArrayItemProof { proof: ret })
    }
}

pub struct ObjectArrayProofVerifier {
    hash_method: HashMethod,
}

impl ObjectArrayProofVerifier {
    pub fn new(hash_method: HashMethod) -> Self {
        Self { hash_method }
    }

    pub fn verify_with_obj_data_str(
        &self,
        obj_data: &str,
        obj_id: &ObjId,
        proof: &ObjectArrayItemProof,
    ) -> NdnResult<bool> {
        // Parse the object data as JSON
        let body: ObjectArrayBody = serde_json::from_str(obj_data).map_err(|e| {
            let msg = format!("Error decoding object map body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let root_hash = body.root_hash;
        self.verify(&root_hash, obj_id, proof)
    }

    pub fn verify_with_obj_data(
        &self,
        obj_data: serde_json::Value,
        obj_id: &ObjId,
        proof: &ObjectArrayItemProof,
    ) -> NdnResult<bool> {
        // Get the root hash from the object data
        let body: ObjectArrayBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error decoding object array body: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let root_hash = body.root_hash;
        self.verify(&root_hash, obj_id, proof)
    }

    pub fn verify(
        &self,
        root_hash: &str,
        obj_id: &ObjId,
        proof: &ObjectArrayItemProof,
    ) -> NdnResult<bool> {
        if proof.proof.len() < 2 {
            let msg = format!("Invalid proof path length: {}", proof.proof.len());
            error!("{}", msg);
            return Err(NdnError::InvalidParam(msg));
        }

        // The first item is the leaf node, which is the item itself
        if proof.proof[0].1 != get_obj_hash(obj_id, self.hash_method)? {
            let msg = format!(
                "Unmatched object array leaf hash: expected {:?}, got {:?}",
                obj_id, proof.proof[0].1
            );
            warn!("{}", msg);
            return Ok(false);
        }

        let root_hash = Base32Codec::from_base32(root_hash).map_err(|e| {
            let msg = format!("Error decoding root hash: {}, {}", root_hash, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        // The last item is the root node, which is obj_id.obj_hash field
        if proof.proof[proof.proof.len() - 1].1 != root_hash {
            let msg = format!(
                "Unmatched object array root hash: expected {:?}, got {:?}",
                root_hash,
                proof.proof[proof.proof.len() - 1].1
            );
            warn!("{}", msg);
            return Ok(false);
        }

        let mtree_verifier = MerkleTreeProofPathVerifier::new(self.hash_method);
        mtree_verifier.verify(&proof.proof)
    }
}
