use rocksdb::{DB, Options};
use std::sync::Arc;
use crate::kv_provider::*

pub struct RocksDBStore {
    db: Arc<DB>,
}

impl RocksDBStore {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, path)?;
        Ok(RocksDBStore { db: Arc::new(db) })
    }
}

#[async_trait]
impl KVStoreProvider for RocksDBStore {
    async fn get(&self, key: String) -> Result<Option<String>, Box<dyn std::error::Error>> {
        match self.db.get(key.as_bytes())? {
            Some(value) => Ok(Some(String::from_utf8(value)?)),
            None => Ok(None),
        }
    }

    async fn set(&self, key: String, value: String) -> Result<(), Box<dyn std::error::Error>> {
        self.db.put(key.as_bytes(), value.as_bytes())?;
        Ok(())
    }
}
