use crate::error::{WorkflowError, WorkflowResult};
use async_trait::async_trait;
use named_store::NamedDataMgr as NamedStoreMgr;
use ndn_lib::ObjId;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait WorkflowObjectStore: Send + Sync {
    async fn put_json(&self, object_type: &str, value: &Value) -> WorkflowResult<String>;
    async fn get_json(&self, object_id: &str) -> WorkflowResult<Option<Value>>;
    async fn exists(&self, object_id: &str) -> WorkflowResult<bool>;
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryObjectStore {
    inner: Arc<Mutex<HashMap<String, Value>>>,
}

impl InMemoryObjectStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl WorkflowObjectStore for InMemoryObjectStore {
    async fn put_json(&self, object_type: &str, value: &Value) -> WorkflowResult<String> {
        let object_id = deterministic_object_id(object_type, value)?;
        self.inner
            .lock()
            .await
            .insert(object_id.clone(), value.clone());
        Ok(object_id)
    }

    async fn get_json(&self, object_id: &str) -> WorkflowResult<Option<Value>> {
        Ok(self.inner.lock().await.get(object_id).cloned())
    }

    async fn exists(&self, object_id: &str) -> WorkflowResult<bool> {
        Ok(self.inner.lock().await.contains_key(object_id))
    }
}

#[derive(Clone)]
pub struct NamedStoreObjectStore {
    store: NamedStoreMgr,
}

impl NamedStoreObjectStore {
    pub fn new(store: NamedStoreMgr) -> Self {
        Self { store }
    }
}

#[async_trait]
impl WorkflowObjectStore for NamedStoreObjectStore {
    async fn put_json(&self, object_type: &str, value: &Value) -> WorkflowResult<String> {
        let object_id = deterministic_object_id(object_type, value)?;
        let obj_id =
            ObjId::new(&object_id).map_err(|err| WorkflowError::ObjectStore(err.to_string()))?;
        let json = serde_json::to_string(value)
            .map_err(|err| WorkflowError::Serialization(err.to_string()))?;
        self.store
            .put_object(&obj_id, &json)
            .await
            .map_err(|err| WorkflowError::ObjectStore(err.to_string()))?;
        Ok(object_id)
    }

    async fn get_json(&self, object_id: &str) -> WorkflowResult<Option<Value>> {
        let obj_id =
            ObjId::new(object_id).map_err(|err| WorkflowError::ObjectStore(err.to_string()))?;
        match self.store.get_object(&obj_id).await {
            Ok(value) => {
                let value = serde_json::from_str(&value)
                    .map_err(|err| WorkflowError::Serialization(err.to_string()))?;
                Ok(Some(value))
            }
            Err(err) => {
                if err.to_string().contains("NotFound") || err.to_string().contains("not found") {
                    Ok(None)
                } else {
                    Err(WorkflowError::ObjectStore(err.to_string()))
                }
            }
        }
    }

    async fn exists(&self, object_id: &str) -> WorkflowResult<bool> {
        Ok(self.get_json(object_id).await?.is_some())
    }
}

pub fn deterministic_object_id<T: Serialize>(
    object_type: &str,
    value: &T,
) -> WorkflowResult<String> {
    let bytes =
        serde_json::to_vec(value).map_err(|err| WorkflowError::Serialization(err.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{object_type}:{}", hex::encode(hasher.finalize())))
}
