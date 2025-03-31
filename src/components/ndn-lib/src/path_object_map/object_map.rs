use super::storage::{
    PathObjectMapInnerStorage, PathObjectMapInnerStorageFactory, PathObjectMapInnerStorageRef,
    PathObjectMapProofVerifierRef,
};
use crate::hash::HashMethod;
use crate::object::ObjId;
use crate::OBJ_TYPE_OBJMAPT;
use crate::{NdnError, NdnResult};
use bincode::de;
use crypto_common::Key;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathObjectMapItem {
    pub obj_id: ObjId,
    pub meta: Option<Vec<u8>>,
}

impl PathObjectMapItem {
    pub fn new(obj_id: ObjId, meta: Option<Vec<u8>>) -> Self {
        Self { obj_id, meta }
    }

    pub fn encode(&self) -> NdnResult<Vec<u8>> {
        let bytes = bincode::serialize(self).map_err(|e| {
            let msg = format!("Error serializing PathObjectMapItem: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(bytes)
    }

    pub fn decode(data: &[u8]) -> NdnResult<Self> {
        let ret = bincode::deserialize(data).map_err(|e| {
            let msg = format!("Error deserializing PathObjectMapItem: {}", e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(ret)
    }
}

#[derive(Debug, Clone)]
pub struct PathObjectMapItemProof {
    // The proof nodes are the MPT nodes on the path to the root node
    pub proof_nodes: Vec<Vec<u8>>,

    // The root hash of the MPT tree
    pub root_hash: Vec<u8>,
    // TODO: should we add the item to the proof?
    // pub item: Option<PathObjectMapItem>,
}

#[derive(Clone)]
pub struct PathObjectMap {
    hash_method: HashMethod,
    db: PathObjectMapInnerStorageRef,
}

impl PathObjectMap {
    pub async fn new(hash_method: HashMethod) -> Self {
        let db =
            PathObjectMapInnerStorageFactory::create_memory_storage_by_hash_method(hash_method);
        let db = Arc::new(db);
        Self { hash_method, db }
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
        let item = PathObjectMapItem::new(obj_id, meta);
        let value = item.encode()?;
        self.db.put(key.as_bytes(), &value).await?;
        self.db.commit().await?;
        Ok(())
    }

    pub async fn get_object(&self, key: &str) -> NdnResult<Option<PathObjectMapItem>> {
        match self.db.get(key.as_bytes()).await? {
            Some(value) => {
                let item = PathObjectMapItem::decode(&value)?;
                Ok(Some(item))
            }
            None => Ok(None),
        }
    }

    pub async fn remove_object(&self, key: &str) -> NdnResult<Option<(ObjId, Option<Vec<u8>>)>> {
        let value = self.db.remove(key.as_bytes()).await?;
        if let Some(value) = value {
            let item = PathObjectMapItem::decode(&value)?;
            Ok(Some((item.obj_id, item.meta)))
        } else {
            Ok(None)
        }
    }

    pub async fn get_object_proof_path(
        &self,
        key: &str,
    ) -> NdnResult<Option<PathObjectMapItemProof>> {
        let proof_nodes = self.db.generate_proof(key.as_bytes()).await?;
        let root_hash = self.db.root().await;

        Ok(Some(PathObjectMapItemProof {
            proof_nodes,
            root_hash,
        }))
    }
}

#[derive(Clone)]
pub struct ObjectMapProofVerifier {
    hash_method: HashMethod,
    verifier: PathObjectMapProofVerifierRef,
}

impl ObjectMapProofVerifier {
    pub fn new(hash_method: HashMethod) -> Self {
        let verifier =
            PathObjectMapInnerStorageFactory::create_verifier_by_hash_method(hash_method);
        Self {
            hash_method,
            verifier: Arc::new(verifier),
        }
    }

    pub fn verify(
        &self,
        key: &str,
        value: Option<&[u8]>,
        proof: &PathObjectMapItemProof,
    ) -> NdnResult<bool> {
        let key_bytes = key.as_bytes();
        self.verifier
            .verify(&proof.proof_nodes, &proof.root_hash, key.as_bytes(), value)
    }
}
