use crate::kv_provider::*;
use async_trait::async_trait;
use buckyos_kit::*;
use log::*;
use sled::{Db, IVec};
use std::sync::Arc;
pub struct SledStore {
    db: Arc<Db>,
}

impl SledStore {
    pub fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let root_path = get_buckyos_root_dir();
        let path = root_path.join("data").join("sys_config");

        let db = sled::open(path)?;
        Ok(SledStore { db: Arc::new(db) })
    }
}

#[async_trait]
impl KVStoreProvider for SledStore {
    async fn get(&self, key: String) -> Result<Option<String>> {
        match self
            .db
            .get(key.clone())
            .map_err(|error| KVStoreErrors::InternalError(error.to_string()))?
        {
            Some(value) => {
                let result = String::from_utf8(value.to_vec()).map_err(|_err| {
                    KVStoreErrors::InternalError("Invalid UTF-8 sequence".to_string())
                })?;
                info!("Sled Get key:[{}] value length:[{}]", key, result.len());
                Ok(Some(result))
            }
            None => Ok(None),
        }
    }

    async fn set(&self, key: String, value: String) -> Result<()> {
        self.db
            .insert(key.clone(), value.clone().into_bytes())
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        self.db
            .flush()
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        info!("Sled Set key:[{}] to value:[{}]", key, value);
        Ok(())
    }

    async fn create(&self, key: &str, value: &str) -> Result<()> {
        let create_result = self
            .db
            .compare_and_swap(
                key.to_string(),
                None as Option<IVec>,
                Some(value.to_string().into_bytes()),
            )
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()));

        match create_result {
            Ok(Ok(_)) => {
                self.db
                    .flush()
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                info!("Sled Create key:[{}] to value:[{}]", key, value);
                return Ok(());
            }
            Ok(Err(_)) => {
                warn!(
                    "Sled Create key:[{}] to value:[{}] failed, key already exist",
                    key, value
                );
                return Err(KVStoreErrors::KeyExist(key.to_string()));
            }
            Err(err) => {
                return Err(KVStoreErrors::InternalError(err.to_string()));
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let result = self
            .db
            .remove(key.to_string())
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        self.db
            .flush()
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        if result.is_none() {
            return Err(KVStoreErrors::KeyNotFound(key.to_string()));
        }
        info!("Sled Delete key:[{}]", key);
        Ok(())
    }
}
