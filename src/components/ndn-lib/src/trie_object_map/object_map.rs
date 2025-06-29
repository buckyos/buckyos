pub use super::storage::TrieObjectMapProofVerifyResult;
use super::storage::{
    TrieObjectMapInnerStorage, TrieObjectMapInnerStorageRef, TrieObjectMapProofVerifierRef,
    TrieObjectMapStorageType,
};
use super::storage_factory::{
    TrieObjectMapStorageFactory, TrieObjectMapStorageOpenMode,
    GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY,
};
use crate::coll::CollectionStorageMode;
use crate::hash::HashMethod;
use crate::object::ObjId;
use crate::{build_named_object_by_json, PathObject, OBJ_TYPE_TRIE_SIMPLE};
use crate::{Base32Codec, NdnError, NdnResult, OBJ_TYPE_TRIE};
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
    pub total_count: u64, // Total number of objects in the map
}

impl TrieObjectMapBody {
    pub fn calc_obj_id(&self) -> (ObjId, String) {
        let obj_type = match CollectionStorageMode::select_mode(Some(self.total_count)) {
            CollectionStorageMode::Simple => OBJ_TYPE_TRIE_SIMPLE,
            CollectionStorageMode::Normal => OBJ_TYPE_TRIE,
        };

        let body = serde_json::to_value(self).expect("Failed to serialize TrieObjectMapBody");
        build_named_object_by_json(obj_type, &body)
    }

    pub fn get_storage_type(&self) -> TrieObjectMapStorageType {
        let mode = CollectionStorageMode::select_mode(Some(self.total_count));
        TrieObjectMapStorageType::select_storage_type(Some(mode))
    }
}

pub struct TrieObjectMap {
    obj_id: ObjId,
    body: TrieObjectMapBody,
    storage: Arc<Box<dyn TrieObjectMapInnerStorage>>,
}

impl TrieObjectMap {
    pub fn new(
        obj_id: ObjId,
        body: TrieObjectMapBody,
        storage: Box<dyn TrieObjectMapInnerStorage>,
    ) -> Self {
        Self {
            obj_id,
            body,
            storage: Arc::new(storage),
        }
    }

    // Load object map from storage
    pub async fn open(obj_data: serde_json::Value, read_only: bool) -> NdnResult<Self> {
        let (obj_id, s) = build_named_object_by_json(OBJ_TYPE_TRIE, &obj_data);

        let body: TrieObjectMapBody = serde_json::from_value(obj_data).map_err(|e| {
            let msg = format!("Error deserializing TrieObjectMapBody: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let storage = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .open_by_hash_method(
                Some((&obj_id, body.root_hash.as_str())),
                true,
                Some(body.get_storage_type()),
                body.hash_method,
                TrieObjectMapStorageOpenMode::OpenExisting,
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

        Ok(Self {
            obj_id,
            body,
            storage: Arc::new(storage),
        })
    }

    pub fn storage_type(&self) -> TrieObjectMapStorageType {
        self.body.get_storage_type()
    }

    pub fn len(&self) -> u64 {
        self.body.total_count
    }

    pub fn get_root_hash(&self) -> Vec<u8> {
        self.storage.root()
    }

    fn get_root_hash_str(&self) -> String {
        Base32Codec::to_base32(&self.storage.root())
    }

    pub fn get_obj_id(&self) -> &ObjId {
        &self.obj_id
    }

    pub fn calc_obj_id(&self) -> (ObjId, String) {
        self.body.calc_obj_id()
    }

    pub fn hash_method(&self) -> HashMethod {
        self.body.hash_method
    }

    pub fn get_object(&self, key: &str) -> NdnResult<Option<ObjId>> {
        self.storage.get(key)
    }

    pub fn is_object_exist(&self, key: &str) -> NdnResult<bool> {
        self.storage.is_exist(key)
    }

    pub fn iter<'a>(&'a self) -> NdnResult<Box<dyn Iterator<Item = (String, ObjId)> + 'a>> {
        Ok(Box::new(self.storage.iter()?))
    }

    pub fn traverse(
        &self,
        callback: &mut dyn FnMut(String, ObjId) -> NdnResult<()>,
    ) -> NdnResult<()> {
        self.storage.traverse(callback)
    }

    pub fn get_storage_file_path(&self) -> Option<PathBuf> {
        if self.storage_type() == TrieObjectMapStorageType::Memory {
            return None; // Memory storage does not have a file path
        }

        let obj_id = self.get_obj_id();

        let factory = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY.get().unwrap();
        let file_path = factory.get_file_path_by_id(Some(&obj_id), self.storage_type());
        Some(file_path)
    }

    pub fn clone(&self) -> Self {
        let obj_id = self.get_obj_id().clone();
        let body = self.body.clone();
        let storage = self.storage.clone();

        Self {
            obj_id,
            body,
            storage,
        }
    }

    pub(crate) async fn clone_storage_for_modify(
        &self,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>> {
        let obj_id = self.get_obj_id();

        let new_storage = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
            .get()
            .unwrap()
            .clone(&obj_id, &**self.storage, false)
            .await
            .map_err(|e| {
                let msg = format!("Error cloning trie object map storage: {}, {}", obj_id.to_base32(), e);
                error!("{}", msg);
                e
            })?;

        Ok(new_storage)
    }

    pub fn get_object_proof_path(&self, key: &str) -> NdnResult<Option<TrieObjectMapItemProof>> {
        let proof_nodes = self.storage.generate_proof(key)?;
        let root_hash = self.storage.root();

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
        value: Option<&[u8]>,
        proof: &TrieObjectMapItemProof,
    ) -> NdnResult<TrieObjectMapProofVerifyResult> {
        let key_bytes = key.as_bytes();
        self.verifier
            .verify(&proof.proof_nodes, &proof.root_hash, key.as_bytes(), value)
    }

    pub fn verify_object(
        &self,
        key: &str,
        obj_id: Option<&ObjId>,
        proof: &TrieObjectMapItemProof,
    ) -> NdnResult<TrieObjectMapProofVerifyResult> {
        let obj_id = obj_id
            .map(|id| bincode::serialize(id))
            .transpose()
            .map_err(|e| {
                let msg = format!("Error serializing ObjId: {:?}, {}", obj_id, e);
                error!("{}", msg);
                NdnError::InvalidData(msg)
            })?;

        self.verify(key, obj_id.as_deref(), proof)
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
