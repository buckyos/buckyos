use crate::{NdnError, NdnResult};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::to_string;

#[derive(Debug, Clone)]
pub struct ObjectArrayItemProof {
    pub proof: Vec<(u64, Vec<u8>)>,
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
