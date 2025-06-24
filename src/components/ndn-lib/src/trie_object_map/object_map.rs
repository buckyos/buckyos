pub use super::storage::TrieObjectMapProofVerifyResult;
use super::storage::{
    TrieObjectMapInnerStorage, TrieObjectMapInnerStorageRef, TrieObjectMapProofVerifierRef,
    TrieObjectMapStorageType,
};
use super::storage_factory::{TrieObjectMapStorageFactory, GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY};
use crate::hash::HashMethod;
use crate::object::ObjId;
use crate::{Base32Codec, NdnError, NdnResult, OBJ_TYPE_TRIE};
use crate::{PathObject, build_named_object_by_json};
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

    pub fn root_hash(&self) -> &[u8] {
        &self.root_hash
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrieObjectMapBody {
    pub root_hash: String, // Encoded in base32 format
    pub hash_method: HashMethod,
    pub storage_type: TrieObjectMapStorageType,
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
    pub async fn open(obj_data: serde_json::Value, read_only: bool) -> NdnResult<Self> {
        let body: TrieObjectMapBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error deserializing TrieObjectMapBody: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let db = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open_by_hash_method(
                Some(body.root_hash.as_str()),
                read_only,
                Some(body.storage_type),
                body.hash_method,
            )
            .await
            .map_err(|e| {
                let msg = format!(
                    "Error opening trie object map storage: {}, {}",
                    body.root_hash, e
                );
                error!("{}", msg);
                e
            })?;

        Ok(Self { hash_method: body.hash_method, db })
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

    fn get_root_hash_str(&self) -> String {
        Base32Codec::to_base32(&self.db.root())
    }

    pub fn get_obj_id(&self) -> ObjId {
        self.calc_obj_id().0
    }

    pub fn calc_obj_id(&self) -> (ObjId, String) {
        let body = TrieObjectMapBody {
            root_hash: self.get_root_hash_str(),
            hash_method: self.hash_method.clone(),
            storage_type: self.get_storage_type(),
        };

        let (obj_id, s) = build_named_object_by_json(
            OBJ_TYPE_TRIE,
            &serde_json::to_value(&body).expect("Failed to serialize ChunkListBody"),
        );

        (obj_id, s)
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
        if self.get_storage_type() == TrieObjectMapStorageType::Memory {
            return None; // Memory storage does not have a file path
        }

        let root_hash = self.get_root_hash_str();

        let factory = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY.get().unwrap();
        let file_path = factory.get_file_path_by_id(Some(root_hash.as_str()), self.get_storage_type());
        Some(file_path)
    }

    // Should not call this function if in read-only mode
    pub async fn save(&mut self) -> NdnResult<()> {
        if self.is_read_only() {
            let msg = "Trie Object map is read-only".to_string();
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }

        let root_hash = self.get_root_hash_str();

        GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .save(&root_hash, self.db.as_mut())
            .await
            .map_err(|e| {
                let msg = format!("Error saving object map: {}", e);
                error!("{}", msg);
                e
            })?;

        info!("Saved trie object map to storage: {}", root_hash);

        Ok(())
    }

    pub async fn clone(&self, read_only: bool) -> NdnResult<Self> {
        let root_hash = self.get_root_hash_str();

        let mut new_storage = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .clone(&root_hash, &*self.db, read_only)
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

    pub fn get_object_proof_path(&self, key: &str) -> NdnResult<Option<TrieObjectMapItemProof>> {
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
