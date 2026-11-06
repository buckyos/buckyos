use crate::error::{WorkflowError, WorkflowResult};
use async_trait::async_trait;
use named_store::NamedDataMgr;
use ndn_lib::ObjId;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait WorkflowObjectStore: Send + Sync {
    async fn put_json(&self, object_type: &str, value: &Value) -> WorkflowResult<String>;
    async fn put_json_with_id(&self, object_id: &str, value: &Value) -> WorkflowResult<()>;
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

    async fn put_json_with_id(&self, object_id: &str, value: &Value) -> WorkflowResult<()> {
        self.inner
            .lock()
            .await
            .insert(object_id.to_string(), value.clone());
        Ok(())
    }

    async fn get_json(&self, object_id: &str) -> WorkflowResult<Option<Value>> {
        Ok(self.inner.lock().await.get(object_id).cloned())
    }

    async fn exists(&self, object_id: &str) -> WorkflowResult<bool> {
        Ok(self.inner.lock().await.contains_key(object_id))
    }
}

/// Pin owner of every object the workflow service writes; the named store
/// GC evicts unpinned objects, so each write renews a lease of `pin_ttl`.
pub const WORKFLOW_PIN_OWNER: &str = "workflow";

#[derive(Clone)]
pub struct NamedStoreObjectStore {
    store: NamedDataMgr,
    pin_ttl: std::time::Duration,
}

impl NamedStoreObjectStore {
    pub fn new(store: NamedDataMgr, pin_ttl: std::time::Duration) -> Self {
        Self { store, pin_ttl }
    }

    async fn put_pinned(&self, obj_id: &ObjId, json: &str) -> WorkflowResult<()> {
        self.store
            .put_object(obj_id, json)
            .await
            .map_err(|err| WorkflowError::ObjectStore(err.to_string()))?;
        self.store
            .pin(obj_id, WORKFLOW_PIN_OWNER, named_store::PinScope::Lease, Some(self.pin_ttl))
            .await
            .map_err(|err| WorkflowError::ObjectStore(format!("pin {obj_id}: {err}")))
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
        self.put_pinned(&obj_id, &json).await?;
        Ok(object_id)
    }

    async fn put_json_with_id(&self, object_id: &str, value: &Value) -> WorkflowResult<()> {
        let obj_id =
            ObjId::new(object_id).map_err(|err| WorkflowError::ObjectStore(err.to_string()))?;
        let json = serde_json::to_string(value)
            .map_err(|err| WorkflowError::Serialization(err.to_string()))?;
        self.put_pinned(&obj_id, &json).await
    }

    async fn get_json(&self, object_id: &str) -> WorkflowResult<Option<Value>> {
        let obj_id =
            ObjId::new(object_id).map_err(|err| WorkflowError::ObjectStore(err.to_string()))?;
        match self.store.get_object(&obj_id).await {
            Ok(value) => {
                let value = serde_json::from_str(&value)
                    .map_err(|err| WorkflowError::Serialization(err.to_string()))?;
                // A read renews the lease so a reused result outlives its first writer.
                let _ = self
                    .store
                    .pin(&obj_id, WORKFLOW_PIN_OWNER, named_store::PinScope::Lease, Some(self.pin_ttl))
                    .await;
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

/// NDN content id of a JSON value: `<object_type>:<hex>` over the JCS
/// canonical encoding, the same rule every named object uses.
pub fn deterministic_object_id<T: Serialize>(
    object_type: &str,
    value: &T,
) -> WorkflowResult<String> {
    let value =
        serde_json::to_value(value).map_err(|err| WorkflowError::Serialization(err.to_string()))?;
    let (obj_id, _) = ndn_lib::try_build_named_object_by_json(object_type, &value)
        .map_err(|err| WorkflowError::Serialization(err.to_string()))?;
    Ok(obj_id.to_string())
}

/// Reads a named object and verifies its content against the id.
pub async fn get_verified_json<O: WorkflowObjectStore + ?Sized>(
    store: &O,
    object_id: &str,
) -> WorkflowResult<Option<Value>> {
    let Some(value) = store.get_json(object_id).await? else {
        return Ok(None);
    };
    let obj_id = ObjId::new(object_id).map_err(|err| WorkflowError::ObjectStore(err.to_string()))?;
    if !ndn_lib::verify_named_object(&obj_id, &value) {
        return Err(WorkflowError::ObjectStore(format!(
            "object `{object_id}` content does not match its id"
        )));
    }
    Ok(Some(value))
}
