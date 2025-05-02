use super::storage::{
    TrieObjectMapInnerStorage, TrieObjectMapInnerStorageFactory, TrieObjectMapInnerStorageRef,
    TrieObjectMapProofVerifierRef,
};
use crate::hash::HashMethod;
use crate::object::ObjId;
use crate::{NdnError, NdnResult};
use crate::{PathObject, OBJ_TYPE_MTREE, OBJ_TYPE_OBJMAPT};
use bincode::de;
use crypto_common::Key;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub use super::storage::TrieObjectMapProofVerifyResult;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrieObjectMapItem {
    pub obj_id: ObjId,
    pub meta: Option<Vec<u8>>,
}

impl TrieObjectMapItem {
    pub fn new(obj_id: ObjId, meta: Option<Vec<u8>>) -> Self {
        Self { obj_id, meta }
    }

    pub fn encode(&self) -> NdnResult<Vec<u8>> {
        let bytes = bincode::serialize(self).map_err(|e| {
            let msg = format!("Error serializing TrieObjectMapItem: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(bytes)
    }

    pub fn decode(data: &[u8]) -> NdnResult<Self> {
        let ret = bincode::deserialize(data).map_err(|e| {
            let msg = format!("Error deserializing TrieObjectMapItem: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(ret)
    }
}

#[derive(Debug, Clone)]
pub struct TrieObjectMapItemProof {
    // The proof nodes are the MPT nodes on the path to the root node
    pub proof_nodes: Vec<Vec<u8>>,

    // The root hash of the MPT tree
    pub root_hash: Vec<u8>,
    // TODO: should we add the item to the proof?
    // pub item: Option<TrieObjectMapItem>,
}

impl TrieObjectMapItemProof {
    // Only encode the nodes to a base64 string, ignore the root hash
    pub fn encode_nodes(&self) -> NdnResult<String> {
        let proof_nodes = TrieObjectMapProofNodesCodec::encode(&self.proof_nodes)?;

        Ok(proof_nodes)
    }

    pub fn decode_nodes(proof_nodes: &str, root_hash: &ObjId) -> NdnResult<Self> {
        let proof_nodes = TrieObjectMapProofNodesCodec::decode(proof_nodes)?;
        let root_hash = root_hash.obj_hash.clone();
        let proof = TrieObjectMapItemProof {
            proof_nodes,
            root_hash,
        };
        Ok(proof)
    }

    pub fn root_id(&self) -> ObjId {
        ObjId::new_by_raw(OBJ_TYPE_MTREE.to_owned(), self.root_hash.clone())
    }
}

#[derive(Clone)]
pub struct TrieObjectMap {
    hash_method: HashMethod,
    db: TrieObjectMapInnerStorageRef,
}

impl TrieObjectMap {
    pub async fn new(hash_method: HashMethod) -> Self {
        let db =
            TrieObjectMapInnerStorageFactory::create_memory_storage_by_hash_method(hash_method);
        let db = Arc::new(db);
        Self { hash_method, db }
    }

    pub async fn get_root_hash(&self) -> Vec<u8> {
        self.db.root().await
    }

    pub async fn get_obj_id(&self) -> ObjId {
        let root_hash = self.db.root().await;
        ObjId::new_by_raw(OBJ_TYPE_OBJMAPT.to_owned(), root_hash)
    }

    pub fn hash_method(&self) -> HashMethod {
        self.hash_method
    }

    pub async fn put_object(
        &self,
        key: &str,
        obj_id: ObjId,
        meta: Option<Vec<u8>>,
    ) -> NdnResult<()> {
        let item = TrieObjectMapItem::new(obj_id, meta);
        let value = item.encode()?;
        self.db.put(key.as_bytes(), &value).await?;

        Ok(())
    }

    pub async fn get_object(&self, key: &str) -> NdnResult<Option<TrieObjectMapItem>> {
        match self.db.get(key.as_bytes()).await? {
            Some(value) => {
                let item = TrieObjectMapItem::decode(&value)?;
                Ok(Some(item))
            }
            None => Ok(None),
        }
    }

    pub async fn remove_object(&self, key: &str) -> NdnResult<Option<(ObjId, Option<Vec<u8>>)>> {
        let value = self.db.remove(key.as_bytes()).await?;
        if let Some(value) = value {
            let item = TrieObjectMapItem::decode(&value)?;
            Ok(Some((item.obj_id, item.meta)))
        } else {
            Ok(None)
        }
    }

    pub async fn is_object_exist(&self, key: &str) -> NdnResult<bool> {
        self.db.is_exist(key.as_bytes()).await
    }

    pub async fn get_object_proof_path(
        &self,
        key: &str,
    ) -> NdnResult<Option<TrieObjectMapItemProof>> {
        let proof_nodes = self.db.generate_proof(key.as_bytes()).await?;
        let root_hash = self.db.root().await;

        Ok(Some(TrieObjectMapItemProof {
            proof_nodes,
            root_hash,
        }))
    }
}

#[derive(Clone)]
pub struct TrieObjectMapProofVerifier {
    hash_method: HashMethod,
    verifier: TrieObjectMapProofVerifierRef,
}

impl TrieObjectMapProofVerifier {
    pub fn new(hash_method: HashMethod) -> Self {
        let verifier =
            TrieObjectMapInnerStorageFactory::create_verifier_by_hash_method(hash_method);
        Self {
            hash_method,
            verifier: Arc::new(verifier),
        }
    }

    pub fn verify(
        &self,
        key: &str,
        value: &[u8],
        proof: &TrieObjectMapItemProof,
    ) -> NdnResult<TrieObjectMapProofVerifyResult> {
        let key_bytes = key.as_bytes();
        self.verifier
            .verify(&proof.proof_nodes, &proof.root_hash, key.as_bytes(), value)
    }

    pub fn verify_object(
        &self,
        key: &str,
        obj_id: &ObjId,
        meta: Option<&[u8]>,
        proof: &TrieObjectMapItemProof,
    ) -> NdnResult<TrieObjectMapProofVerifyResult> {
        let item = TrieObjectMapItem::new(obj_id.clone(), meta.map(|m| m.to_vec()));
        let value = item.encode()?;

        self.verify(key, value.as_ref(), proof)
    }
}

use base64::prelude::*;

// Proof nodes in base64 string format
#[derive(Serialize, Deserialize)]
struct ProofNodes {
    nodes: Vec<String>,
}

pub struct TrieObjectMapProofNodesCodec {}

impl TrieObjectMapProofNodesCodec {
    pub fn encode(proof_nodes: &[Vec<u8>]) -> NdnResult<String> {
        let mut ret = Vec::new();
        for node in proof_nodes {
            let s = BASE64_STANDARD.encode(node);
            ret.push(s);
        }

        let nodes = ProofNodes { nodes: ret };

        // Encode to json string
        let ret = serde_json::to_string(&nodes).map_err(|e| {
            let msg = format!("Error serializing ProofNodes: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(ret)
    }

    pub fn decode(proof_nodes: &str) -> NdnResult<Vec<Vec<u8>>> {
        let mut ret = Vec::new();

        // First decode from json string
        let nodes: ProofNodes = serde_json::from_str(proof_nodes).map_err(|e| {
            let msg = format!("Error deserializing ProofNodes: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        for node in nodes.nodes {
            let s = BASE64_STANDARD.decode(node).map_err(|e| {
                let msg = format!("Error decoding base64 string: {}", e);
                error!("{}", msg);
                NdnError::InvalidData(msg)
            })?;
            ret.push(s);
        }

        Ok(ret)
    }
}
