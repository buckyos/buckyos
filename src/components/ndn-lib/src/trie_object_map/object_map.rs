pub use super::storage::TrieObjectMapProofVerifyResult;
use super::storage::{
    TrieObjectMapInnerStorage, TrieObjectMapInnerStorageRef, TrieObjectMapProofVerifierRef,
    TrieObjectMapStorageType,
};
use super::storage_factory::{TrieObjectMapStorageFactory, GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY};
use crate::hash::HashMethod;
use crate::object::ObjId;
use crate::{NdnError, NdnResult};
use crate::{PathObject, OBJ_TYPE_MTREE, OBJ_TYPE_OBJMAPT};
use bincode::de;
use crypto_common::Key;
use log::kv::value;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

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

pub struct TrieObjectMap {
    hash_method: HashMethod,
    db: Box<dyn TrieObjectMapInnerStorage>,
}

impl TrieObjectMap {
    pub async fn new(
        hash_method: HashMethod,
        storage_type: Option<TrieObjectMapStorageType>,
    ) -> NdnResult<Self> {
        let db = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open_by_hash_method(None, false, storage_type, hash_method)
            .await?;

        Ok(Self { hash_method, db })
    }

    // Load object map from storage
    pub async fn open(
        container_id: &ObjId,
        read_only: bool,
        hash_method: HashMethod,
        storage_type: Option<TrieObjectMapStorageType>,
    ) -> NdnResult<Self> {
        let db = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open_by_hash_method(Some(container_id), read_only, storage_type, hash_method)
            .await
            .map_err(|e| {
                let msg = format!(
                    "Error opening trie object map storage: {}, {}",
                    container_id, e
                );
                error!("{}", msg);
                e
            })?;

        Ok(Self { hash_method, db })
    }

    pub fn is_read_only(&self) -> bool {
        self.db.is_readonly()
    }

    pub fn get_storage_type(&self) -> TrieObjectMapStorageType {
        self.db.get_type()
    }

    pub fn get_root_hash(&self) -> Vec<u8> {
        self.db.root()
    }

    pub fn get_obj_id(&self) -> ObjId {
        let root_hash = self.db.root();
        ObjId::new_by_raw(OBJ_TYPE_OBJMAPT.to_owned(), root_hash)
    }

    pub fn hash_method(&self) -> HashMethod {
        self.hash_method
    }

    pub fn put_object(&mut self, key: &str, obj_id: &ObjId) -> NdnResult<()> {
        self.db.put(key, &obj_id)
    }

    pub fn get_object(&self, key: &str) -> NdnResult<Option<ObjId>> {
        self.db.get(key)
    }

    pub fn remove_object(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        self.db.remove(key)
    }

    pub fn is_object_exist(&self, key: &str) -> NdnResult<bool> {
        self.db.is_exist(key)
    }

    pub fn iter<'a>(&'a self) -> NdnResult<Box<dyn Iterator<Item = (String, ObjId)> + 'a>> {
        Ok(Box::new(self.db.iter()?))
    }

    pub fn traverse(
        &self,
        callback: &mut dyn FnMut(String, ObjId) -> NdnResult<()>,
    ) -> NdnResult<()> {
        self.db.traverse(callback)
    }

    pub fn get_storage_file_path(&self) -> Option<PathBuf> {
        let id = self.get_obj_id();

        if self.get_storage_type() == TrieObjectMapStorageType::Memory {
            return None; // Memory storage does not have a file path
        }

        let factory = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY.get().unwrap();
        let file_path = factory.get_file_path_by_id(Some(&id), self.get_storage_type());
        Some(file_path)
    }

    // Should not call this function if in read-only mode
    pub async fn save(&mut self) -> NdnResult<()> {
        if self.is_read_only() {
            let msg = "Trie Object map is read-only".to_string();
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }

        let obj_id = self.get_obj_id();

        GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .save(&obj_id, self.db.as_mut())
            .await
            .map_err(|e| {
                let msg = format!("Error saving object map: {}", e);
                error!("{}", msg);
                e
            })?;

        info!("Saved trie object map to storage: {}", obj_id);

        Ok(())
    }

    pub async fn clone(&self, read_only: bool) -> NdnResult<Self> {
        let obj_id = self.get_obj_id();

        let mut new_storage = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .clone(&obj_id, &*self.db, read_only)
            .await
            .map_err(|e| {
                let msg = format!("Error cloning object map storage: {}", e);
                error!("{}", msg);
                e
            })?;

        let ret = TrieObjectMap {
            hash_method: self.hash_method.clone(),
            db: new_storage,
        };

        Ok(ret)
    }

    pub fn get_object_proof_path(
        &self,
        key: &str,
    ) -> NdnResult<Option<TrieObjectMapItemProof>> {
        let proof_nodes = self.db.generate_proof(key)?;
        let root_hash = self.db.root();

        Ok(Some(TrieObjectMapItemProof {
            proof_nodes,
            root_hash,
        }))
    }
}

#[derive(Clone)]
pub struct TrieObjectMapProofVerifierHelper {
    hash_method: HashMethod,
    verifier: TrieObjectMapProofVerifierRef,
}

impl TrieObjectMapProofVerifierHelper {
    pub fn new(hash_method: HashMethod) -> Self {
        let verifier = TrieObjectMapStorageFactory::create_verifier_by_hash_method(hash_method);
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
        proof: &TrieObjectMapItemProof,
    ) -> NdnResult<TrieObjectMapProofVerifyResult> {
        let value = bincode::serialize(obj_id).map_err(|e| {
            let msg = format!("Error serializing ObjId: {}, {}", obj_id, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

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
