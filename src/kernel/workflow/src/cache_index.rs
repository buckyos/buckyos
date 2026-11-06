//! Step cache index (TODO D11): `cache_key -> result_obj_id / artifact_refs /
//! expires_at`. The index is a persistent lookup table, not a named object;
//! result objects and artifacts live in the object store / artifacts bucket.

use crate::error::WorkflowResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheEntry {
    pub result_obj_id: String,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub fingerprint: BTreeMap<String, String>,
}

#[async_trait]
pub trait CacheIndex: Send + Sync {
    async fn get(&self, key: &str) -> WorkflowResult<Option<CacheEntry>>;
    async fn put(&self, key: &str, entry: &CacheEntry) -> WorkflowResult<()>;
    async fn remove(&self, key: &str) -> WorkflowResult<()>;
    async fn len(&self) -> WorkflowResult<usize>;
}

#[derive(Default)]
pub struct MemoryCacheIndex {
    inner: Mutex<HashMap<String, CacheEntry>>,
}

impl MemoryCacheIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CacheIndex for MemoryCacheIndex {
    async fn get(&self, key: &str) -> WorkflowResult<Option<CacheEntry>> {
        Ok(self.inner.lock().unwrap().get(key).cloned())
    }

    async fn put(&self, key: &str, entry: &CacheEntry) -> WorkflowResult<()> {
        self.inner
            .lock()
            .unwrap()
            .insert(key.to_string(), entry.clone());
        Ok(())
    }

    async fn remove(&self, key: &str) -> WorkflowResult<()> {
        self.inner.lock().unwrap().remove(key);
        Ok(())
    }

    async fn len(&self) -> WorkflowResult<usize> {
        Ok(self.inner.lock().unwrap().len())
    }
}

/// `step_cache_key = hash(thunk_obj_id, relevant_execution_fingerprint)`.
pub fn step_cache_key(thunk_obj_id: &str, fingerprint: &BTreeMap<String, String>) -> String {
    let value = serde_json::json!({
        "thunk_obj_id": thunk_obj_id,
        "fingerprint": fingerprint,
    });
    let (id, _) = ndn_lib::build_named_object_by_json("stepcache", &value);
    hex::encode(&id.obj_hash)
}
