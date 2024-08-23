use async_trait::async_trait;
use sled::{Db, IVec};
use std::sync::Arc;
use crate::kv_provider::*;
use log::*;

pub struct SledStore {
    db: Arc<Db>,
}

impl SledStore {
    pub fn new(path: &str) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let db = sled::open(path)?;
        Ok(SledStore { db: Arc::new(db) })
    }
}

#[async_trait]
impl KVStoreProvider for SledStore {
    async fn get(&self, key: String) -> Result<String> {
        match self.db.get(key.clone()).map_err(|error| KVStoreErrors::InternalError(error.to_string()))? {
            Some(value) => {
                let result = String::from_utf8(value.to_vec())
                    .map_err(|err| KVStoreErrors::InternalError("Invalid UTF-8 sequence".to_string()))?;
                info!("Sled Get key:[{}] value:[{}]", key, result);
                Ok(result)
            },
            None => Err(KVStoreErrors::KeyNotFound(key)),
        }
    }

    async fn set(&self, key: String, value: String) -> Result<()> {
        self.db.insert(key.clone(), value.clone().into_bytes())
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        self.db.flush().map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        info!("Sled Set key:[{}] to value:[{}]", key, value);
        Ok(())
    }
}
