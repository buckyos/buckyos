use async_trait::async_trait;
use sled::{Db, IVec};
use std::{collections::HashMap, sync::Arc};
use crate::kv_provider::*;
use log::*;
use buckyos_kit::*;
use serde_json::Value;
pub struct SledStore {
    db: Arc<Db>,
}


impl SledStore {
    pub fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let root_path  = get_buckyos_root_dir();
        let path = root_path.join("data").join("system_config");

        let db = sled::open(path)?;
        Ok(SledStore { db: Arc::new(db) })
    }
}

#[async_trait]
impl KVStoreProvider for SledStore {
    async fn get(&self, key: String) -> Result< Option<String> > {
        match self.db.get(key.clone()).map_err(|error| KVStoreErrors::InternalError(error.to_string()))? {
            Some(value) => {
                let result = String::from_utf8(value.to_vec())
                    .map_err(|_err| KVStoreErrors::InternalError("Invalid UTF-8 sequence".to_string()))?;
                info!("Sled Get key:[{}] value length:[{}]", key, result.len());
                Ok(Some(result))
            },
            None => Ok(None)
        }
    }

    async fn set(&self, key: String, value: String) -> Result<()> {
        self.db.insert(key.clone(), value.clone().into_bytes())
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        self.db.flush().map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        info!("Sled Set key:[{}] to value:[{}]", key, value);
        Ok(())
    }

    async fn set_by_path(&self, key: String, json_path: String, value: &Value) -> Result<()> {
        let tx_result = self.db.transaction(|db| {
            // Get the current value
            let current_value = match db.get(key.as_bytes())? {
                Some(val) => val,
                None => return Err(sled::transaction::ConflictableTransactionError::Abort(
                    KVStoreErrors::KeyNotFound(key.clone())
                )),
            };

            // Parse the current value as JSON
            let mut current_value: Value = serde_json::from_slice(&current_value)
                .map_err(|err| sled::transaction::ConflictableTransactionError::Abort(
                    KVStoreErrors::InternalError(err.to_string())
                ))?;

            // Update the value using json_path
            set_json_by_path(&mut current_value, &json_path, Some(value));

            // Convert back to bytes
            let updated_value = serde_json::to_vec(&current_value)
                .map_err(|err| sled::transaction::ConflictableTransactionError::Abort(
                    KVStoreErrors::InternalError(err.to_string())
                ))?;

            // Update in the transaction
            db.insert(key.as_bytes(), updated_value)?;
            Ok(())
        });

        match tx_result {
            Ok(_) => {
                self.db.flush().map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                Ok(())
            },
            Err(sled::transaction::TransactionError::Abort(err)) => Err(err),
            Err(sled::transaction::TransactionError::Storage(err)) => {
                Err(KVStoreErrors::InternalError(err.to_string()))
            }
        }
    }

    

    async fn create(&self, key: &str, value: &str) -> Result<()> {
        let create_result =  self.db.compare_and_swap(key.to_string(),
            None as Option<IVec>,Some(value.to_string().into_bytes()))
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()));

        match create_result {
            Ok(Ok(_)) => {
                self.db.flush().map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                info!("Sled Create key:[{}] to value:[{}]", key, value);
                return Ok(())
            },
            Ok(Err(_)) => {
                warn!("Sled Create key:[{}] to value:[{}] failed, key already exist", key, value);
                return Err(KVStoreErrors::KeyExist(key.to_string()));
            },
            Err(err) => {
                return Err(KVStoreErrors::InternalError(err.to_string()));
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let result = self.db.remove(key.to_string())
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        self.db.flush().map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        if result.is_none() {
            return Err(KVStoreErrors::KeyNotFound(key.to_string()));
        }
        info!("Sled Delete key:[{}]", key);
        Ok(())
    }

    async fn list_data(&self,key_perfix:&str) -> Result<HashMap<String,String>> {
        let mut result = HashMap::new();
        let iter = self.db.scan_prefix(key_perfix.to_string());
        for item in iter {
            if item.is_ok() {
                let (key,value) = item.unwrap();
                let key_str = String::from_utf8(key.to_vec())
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                let value_str = String::from_utf8(value.to_vec())
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                result.insert(key_str,value_str);
            }
        }
        Ok(result)
    }

    async fn list_keys(&self, key_prefix: &str) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let iter = self.db.scan_prefix(key_prefix.to_string()).keys();
        for key in iter {
            if let Ok(key) = key {
                if let Ok(key_str) = String::from_utf8(key.to_vec()) {
                    result.push(key_str);
                }
            }
        }
        Ok(result)
    }

    async fn list_direct_children(&self, prefix: String) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let prefix = if prefix.eq("") || prefix.ends_with("/") {
            prefix
        } else {
            format!("{}/", prefix)
        };
        let iter = self.db.scan_prefix(prefix.clone()).keys();
        for key in iter {
            if let Ok(key) = key {
                if let Ok(key_str) = String::from_utf8(key.to_vec()) {
                    let suffix = key_str.trim_start_matches(prefix.as_str());
                    let splite_result: Vec<_> = if suffix.ends_with("/") {
                        suffix[1..].split("/").collect()
                    } else {
                        suffix.split("/").collect()
                    };
                    let child = splite_result[0];
                    if !result.contains(&child.to_string()) {
                        result.push(child.to_string());
                    }
                }
            }
        }
        Ok(result)
    }

    async fn exec_tx(&self, tx: HashMap<String, KVAction>, main_key: Option<(String, u64)>) -> Result<()> {
        let tx_result = self.db.transaction(|db| {
            let mut batch = sled::Batch::default();

            for (key, action) in tx.iter() {
                match action {
                    KVAction::Create(value) => {
                        if db.get(key)?.is_some() {
                            return Err(sled::transaction::ConflictableTransactionError::Abort(
                                KVStoreErrors::KeyExist(key.to_string())
                            ));
                        }
                        batch.insert(key.as_bytes(), value.as_bytes());
                    }
                    KVAction::Update(value) => {
                        batch.insert(key.as_bytes(), value.as_bytes());
                    }
                    KVAction::Append(value) => {
                        let existing_value = match db.get(key)? {
                            Some(val) => val,
                            None => return Err(sled::transaction::ConflictableTransactionError::Abort(
                                KVStoreErrors::KeyNotFound(key.to_string()) 
                            )),
                        };

                        let existing_value: String = String::from_utf8(existing_value.to_vec())
                            .map_err(|err| sled::transaction::ConflictableTransactionError::Abort(
                                KVStoreErrors::InternalError(err.to_string())
                            ))?;

                        let updated_value = format!("{}{}", existing_value, value);
                        batch.insert(key.as_bytes(), updated_value.as_bytes());
                    }
                    KVAction::SetByJsonPath(value) => {
                        let existing_value = match db.get(key)? {
                            Some(val) => val,
                            None => return Err(sled::transaction::ConflictableTransactionError::Abort(
                                KVStoreErrors::KeyNotFound(key.to_string())
                            )),
                        };

                        let mut existing_value: Value = serde_json::from_slice(&existing_value)
                            .map_err(|err| sled::transaction::ConflictableTransactionError::Abort(  
                                KVStoreErrors::InternalError(err.to_string())
                            ))?;

                        for (path, sub_value) in value.iter() {
                            if sub_value.is_some() {
                                set_json_by_path(&mut existing_value, path, Some(sub_value.as_ref().unwrap()));
                            } else {
                                set_json_by_path(&mut existing_value, path, None);
                            }
                        }

                        let updated_value = serde_json::to_vec(&existing_value)
                            .map_err(|err| sled::transaction::ConflictableTransactionError::Abort(
                                KVStoreErrors::InternalError(err.to_string())
                            ))?;

                        batch.insert(key.as_bytes(), updated_value);
                    }
                    KVAction::Remove => {
                        batch.remove(key.as_bytes());
                    }
                }
            }

            db.apply_batch(&batch)?;

            if let Some((key, revision)) = main_key.as_ref() {
                let revision_key = format!("{}:revision", key);
                db.insert(revision_key.as_bytes(), revision.to_string().as_bytes())?;
            }

            Ok(())
        });

        match tx_result {
            Ok(_) => Ok(()),
            Err(sled::transaction::TransactionError::Abort(err)) => Err(err),
            Err(sled::transaction::TransactionError::Storage(err)) => {
                Err(KVStoreErrors::InternalError(err.to_string()))
            }
        }
    }
}

