use async_trait::async_trait;
use sled::{Db, IVec};
use std::sync::Arc;
use crate::kv_provider::*;

pub struct SledStore {
    db: Arc<Db>,
}

impl SledStore {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let db = sled::open(path)?;
        Ok(SledStore { db: Arc::new(db) })
    }
}

#[async_trait]
impl KVStoreProvider for SledStore {
    async fn get(&self, key: String) -> Result<Option<String>, Box<dyn std::error::Error>> {
        match self.db.get(key)? {
            Some(value) => Ok(Some(String::from_utf8(value.to_vec())?)),
            None => Ok(None),
        }
    }

    async fn set(&self, key: String, value: String) -> Result<(), Box<dyn std::error::Error>> {
        self.db.insert(key, value.into_bytes())?;
        self.db.flush()?;
        Ok(())
    }
}
