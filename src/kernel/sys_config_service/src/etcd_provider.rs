use async_trait::async_trait;
use etcd_client::*;
use std::{borrow::{Borrow, BorrowMut}, sync::{Arc, Mutex}};
use crate::kv_provider::*;

pub struct EtcdStore {
    client: Arc<Mutex<Client>>,
}

impl EtcdStore {
    pub async fn new(endpoints: &[&str]) -> Result<Self, Box<dyn std::error::Error>> {
        let client = Arc::new(Mutex::new(Client::connect(endpoints, None).await?));
        Ok(EtcdStore {
            client:client,
        })
    }
}

#[async_trait]
impl KVStoreProvider for EtcdStore {
    async fn get(&self, key: String) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut client = self.client.lock().unwrap();
        let resp = client.get(key, None).await?;
        if let Some(kv) = resp.kvs().first() {
            Ok(Some(String::from_utf8(kv.value().to_vec())?))
        } else {
            Ok(None)
        }
    }

    async fn set(&self, key: String, value: String) -> Result<(), Box<dyn std::error::Error>> {
        self.client.lock().unwrap().put(key, value, None).await?;
        Ok(())
    }
}
