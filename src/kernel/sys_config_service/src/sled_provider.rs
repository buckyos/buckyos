use crate::kv_provider::*;
use async_trait::async_trait;
use buckyos_kit::*;
use log::*;
use serde_json::Value;
use sled::{
    transaction::{ConflictableTransactionError, TransactionError, TransactionalTree},
    Db,
};
use std::{collections::HashMap, sync::Arc};

type TxResult<T> = std::result::Result<T, ConflictableTransactionError<KVStoreErrors>>;

pub struct SledStore {
    db: Arc<Db>,
}

impl SledStore {
    const INTERNAL_META_PREFIX: &'static str = "__meta/";
    const REVISION_PREFIX: &'static str = "__meta/revision/";

    fn from_db(db: Db) -> Self {
        SledStore { db: Arc::new(db) }
    }

    pub fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let data_path = get_buckyos_service_local_data_dir("system_config");
        //let path = root_path.join("data").join("system_config");
        let db = sled::open(data_path)?;
        Ok(Self::from_db(db))
    }

    fn revision_key(key: &str) -> String {
        format!("{}{}", Self::REVISION_PREFIX, key)
    }

    fn is_internal_key(key: &str) -> bool {
        key.starts_with(Self::INTERNAL_META_PREFIX)
    }

    fn current_revision(db: &TransactionalTree, key: &str) -> TxResult<u64> {
        let revision_key = Self::revision_key(key);
        let raw_revision = db.get(revision_key.as_bytes())?;
        let Some(raw_revision) = raw_revision else {
            return Ok(0);
        };

        let revision = String::from_utf8(raw_revision.to_vec()).map_err(|err| {
            ConflictableTransactionError::Abort(KVStoreErrors::InternalError(err.to_string()))
        })?;

        revision.parse::<u64>().map_err(|err| {
            ConflictableTransactionError::Abort(KVStoreErrors::InternalError(err.to_string()))
        })
    }

    fn next_revision(db: &TransactionalTree, key: &str) -> TxResult<u64> {
        let current_revision = Self::current_revision(db, key)?;
        current_revision.checked_add(1).ok_or_else(|| {
            ConflictableTransactionError::Abort(KVStoreErrors::InternalError(format!(
                "revision overflow for key: {}",
                key
            )))
        })
    }

    fn write_revision(db: &TransactionalTree, key: &str, revision: u64) -> TxResult<()> {
        let revision_key = Self::revision_key(key);
        db.insert(revision_key.as_bytes(), revision.to_string().as_bytes())?;
        Ok(())
    }
}

#[async_trait]
impl KVStoreProvider for SledStore {
    async fn get(&self, key: String) -> Result<Option<String>> {
        Ok(self
            .get_with_revision(key)
            .await?
            .map(|(value, _revision)| value))
    }

    async fn get_with_revision(&self, key: String) -> Result<Option<(String, u64)>> {
        let tx_result = self.db.transaction(|db| {
            let raw_value = match db.get(key.as_bytes())? {
                Some(value) => value,
                None => return Ok(None),
            };

            let value = String::from_utf8(raw_value.to_vec()).map_err(|_err| {
                ConflictableTransactionError::Abort(KVStoreErrors::InternalError(
                    "Invalid UTF-8 sequence".to_string(),
                ))
            })?;
            let revision = Self::current_revision(db, &key)?;

            Ok(Some((value, revision)))
        });

        match tx_result {
            Ok(Some((value, revision))) => {
                debug!(
                    "Sled Get key:[{}] value length:[{}] revision:[{}]",
                    key,
                    value.len(),
                    revision
                );
                Ok(Some((value, revision)))
            }
            Ok(None) => Ok(None),
            Err(TransactionError::Abort(err)) => Err(err),
            Err(TransactionError::Storage(err)) => {
                Err(KVStoreErrors::InternalError(err.to_string()))
            }
        }
    }

    async fn set(&self, key: String, value: String) -> Result<()> {
        let tx_result = self.db.transaction(|db| {
            let next_revision = Self::next_revision(db, &key)?;
            db.insert(key.as_bytes(), value.as_bytes())?;
            Self::write_revision(db, &key, next_revision)?;
            Ok(())
        });

        match tx_result {
            Ok(_) => {
                self.db
                    .flush()
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
            }
            Err(TransactionError::Abort(err)) => return Err(err),
            Err(TransactionError::Storage(err)) => {
                return Err(KVStoreErrors::InternalError(err.to_string()))
            }
        }
        debug!("Sled Set key:[{}] to value:[{}]", key, value);
        Ok(())
    }

    async fn set_by_path(&self, key: String, json_path: String, value: &Value) -> Result<()> {
        let tx_result = self.db.transaction(|db| {
            // Get the current value
            let current_value = match db.get(key.as_bytes())? {
                Some(val) => val,
                None => {
                    return Err(sled::transaction::ConflictableTransactionError::Abort(
                        KVStoreErrors::KeyNotFound(key.clone()),
                    ))
                }
            };

            // Parse the current value as JSON
            let mut current_value: Value =
                serde_json::from_slice(&current_value).map_err(|err| {
                    sled::transaction::ConflictableTransactionError::Abort(
                        KVStoreErrors::InternalError(err.to_string()),
                    )
                })?;

            // Update the value using json_path
            set_json_by_path(&mut current_value, &json_path, Some(value));

            // Convert back to bytes
            let updated_value = serde_json::to_vec(&current_value).map_err(|err| {
                ConflictableTransactionError::Abort(KVStoreErrors::InternalError(err.to_string()))
            })?;

            let next_revision = Self::next_revision(db, &key)?;
            db.insert(key.as_bytes(), updated_value)?;
            Self::write_revision(db, &key, next_revision)?;
            Ok(())
        });

        match tx_result {
            Ok(_) => {
                self.db
                    .flush()
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                Ok(())
            }
            Err(sled::transaction::TransactionError::Abort(err)) => Err(err),
            Err(sled::transaction::TransactionError::Storage(err)) => {
                Err(KVStoreErrors::InternalError(err.to_string()))
            }
        }
    }

    async fn create(&self, key: &str, value: &str) -> Result<()> {
        let tx_result = self.db.transaction(|db| {
            if db.get(key.as_bytes())?.is_some() {
                return Err(ConflictableTransactionError::Abort(
                    KVStoreErrors::KeyExist(key.to_string()),
                ));
            }

            let next_revision = Self::next_revision(db, key)?;
            db.insert(key.as_bytes(), value.as_bytes())?;
            Self::write_revision(db, key, next_revision)?;
            Ok(())
        });

        match tx_result {
            Ok(_) => {
                self.db
                    .flush()
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                debug!("Sled Create key:[{}] to value:[{}]", key, value);
                Ok(())
            }
            Err(TransactionError::Abort(KVStoreErrors::KeyExist(_))) => {
                warn!(
                    "Sled Create key:[{}] to value:[{}] failed, key already exist",
                    key, value
                );
                Err(KVStoreErrors::KeyExist(key.to_string()))
            }
            Err(TransactionError::Abort(err)) => Err(err),
            Err(TransactionError::Storage(err)) => {
                Err(KVStoreErrors::InternalError(err.to_string()))
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let tx_result = self.db.transaction(|db| {
            if db.get(key.as_bytes())?.is_none() {
                return Err(ConflictableTransactionError::Abort(
                    KVStoreErrors::KeyNotFound(key.to_string()),
                ));
            }

            let next_revision = Self::next_revision(db, key)?;
            db.remove(key.as_bytes())?;
            Self::write_revision(db, key, next_revision)?;
            Ok(())
        });

        match tx_result {
            Ok(_) => {
                self.db
                    .flush()
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
            }
            Err(TransactionError::Abort(err)) => return Err(err),
            Err(TransactionError::Storage(err)) => {
                return Err(KVStoreErrors::InternalError(err.to_string()))
            }
        }
        debug!("Sled Delete key:[{}]", key);
        Ok(())
    }

    async fn list_data(&self, key_perfix: &str) -> Result<HashMap<String, String>> {
        let mut result = HashMap::new();
        let iter = self.db.scan_prefix(key_perfix.to_string());
        for item in iter {
            if item.is_ok() {
                let (key, value) = item.unwrap();
                let key_str = String::from_utf8(key.to_vec())
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                if Self::is_internal_key(&key_str) {
                    continue;
                }
                let value_str = String::from_utf8(value.to_vec())
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                result.insert(key_str, value_str);
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
                    if Self::is_internal_key(&key_str) {
                        continue;
                    }
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
                    if Self::is_internal_key(&key_str) {
                        continue;
                    }
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

    async fn exec_tx(
        &self,
        tx: HashMap<String, KVAction>,
        main_key: Option<(String, u64)>,
    ) -> Result<()> {
        let tx_result = self.db.transaction(|db| {
            let mut batch = sled::Batch::default();
            let mut revision_updates = HashMap::new();

            if let Some((key, expected_revision)) = main_key.as_ref() {
                let actual_revision = Self::current_revision(db, key)?;
                if actual_revision != *expected_revision {
                    return Err(ConflictableTransactionError::Abort(
                        KVStoreErrors::RevisionMismatch {
                            key: key.clone(),
                            expected: *expected_revision,
                            actual: actual_revision,
                        },
                    ));
                }

                let next_revision = actual_revision.checked_add(1).ok_or_else(|| {
                    ConflictableTransactionError::Abort(KVStoreErrors::InternalError(format!(
                        "revision overflow for key: {}",
                        key
                    )))
                })?;
                revision_updates.insert(key.clone(), next_revision);
            }

            for (key, action) in tx.iter() {
                match action {
                    KVAction::Create(value) => {
                        if db.get(key.as_bytes())?.is_some() {
                            return Err(ConflictableTransactionError::Abort(
                                KVStoreErrors::KeyExist(key.to_string()),
                            ));
                        }
                        batch.insert(key.as_bytes(), value.as_bytes());
                    }
                    KVAction::Update(value) => {
                        batch.insert(key.as_bytes(), value.as_bytes());
                    }
                    KVAction::Append(value) => {
                        let existing_value = match db.get(key.as_bytes())? {
                            Some(val) => val,
                            None => {
                                return Err(ConflictableTransactionError::Abort(
                                    KVStoreErrors::KeyNotFound(key.to_string()),
                                ))
                            }
                        };

                        let existing_value: String = String::from_utf8(existing_value.to_vec())
                            .map_err(|err| {
                                ConflictableTransactionError::Abort(KVStoreErrors::InternalError(
                                    err.to_string(),
                                ))
                            })?;

                        let updated_value = format!("{}{}", existing_value, value);
                        batch.insert(key.as_bytes(), updated_value.as_bytes());
                    }
                    KVAction::SetByJsonPath(value) => {
                        let existing_value = match db.get(key.as_bytes())? {
                            Some(val) => val,
                            None => {
                                return Err(ConflictableTransactionError::Abort(
                                    KVStoreErrors::KeyNotFound(key.to_string()),
                                ))
                            }
                        };

                        let mut existing_value: Value = serde_json::from_slice(&existing_value)
                            .map_err(|err| {
                                ConflictableTransactionError::Abort(KVStoreErrors::InternalError(
                                    err.to_string(),
                                ))
                            })?;

                        for (path, sub_value) in value.iter() {
                            if sub_value.is_some() {
                                set_json_by_path(
                                    &mut existing_value,
                                    path,
                                    Some(sub_value.as_ref().unwrap()),
                                );
                            } else {
                                set_json_by_path(&mut existing_value, path, None);
                            }
                        }

                        let updated_value = serde_json::to_vec(&existing_value).map_err(|err| {
                            ConflictableTransactionError::Abort(KVStoreErrors::InternalError(
                                err.to_string(),
                            ))
                        })?;

                        batch.insert(key.as_bytes(), updated_value);
                    }
                    KVAction::Remove => {
                        batch.remove(key.as_bytes());
                    }
                }

                if !revision_updates.contains_key(key) {
                    let next_revision = Self::next_revision(db, key)?;
                    revision_updates.insert(key.clone(), next_revision);
                }
            }

            for (key, revision) in revision_updates.into_iter() {
                let revision_key = Self::revision_key(&key);
                batch.insert(revision_key.as_bytes(), revision.to_string().as_bytes());
            }

            db.apply_batch(&batch)?;
            Ok(())
        });

        match tx_result {
            Ok(_) => {
                self.db
                    .flush()
                    .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                Ok(())
            }
            Err(TransactionError::Abort(err)) => Err(err),
            Err(TransactionError::Storage(err)) => {
                Err(KVStoreErrors::InternalError(err.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup_store() -> SledStore {
        let db = sled::Config::new()
            .temporary(true)
            .open()
            .expect("open temporary sled db");
        SledStore::from_db(db)
    }

    #[tokio::test]
    async fn tracks_revision_sidecar_across_writes() {
        let store = setup_store();

        store
            .create("users/alice/profile", r#"{"name":"alice"}"#)
            .await
            .expect("create key");
        assert_eq!(
            store
                .get("__meta/revision/users/alice/profile".to_string())
                .await
                .expect("get revision"),
            Some("1".to_string())
        );

        store
            .set(
                "users/alice/profile".to_string(),
                r#"{"name":"alice-2"}"#.to_string(),
            )
            .await
            .expect("set key");
        assert_eq!(
            store
                .get("__meta/revision/users/alice/profile".to_string())
                .await
                .expect("get revision"),
            Some("2".to_string())
        );

        store
            .set_by_path(
                "users/alice/profile".to_string(),
                "/name".to_string(),
                &json!("alice-3"),
            )
            .await
            .expect("set by path");
        assert_eq!(
            store
                .get("__meta/revision/users/alice/profile".to_string())
                .await
                .expect("get revision"),
            Some("3".to_string())
        );

        store
            .delete("users/alice/profile")
            .await
            .expect("delete key");
        assert_eq!(
            store
                .get("users/alice/profile".to_string())
                .await
                .expect("get deleted key"),
            None
        );
        assert_eq!(
            store
                .get("__meta/revision/users/alice/profile".to_string())
                .await
                .expect("get revision"),
            Some("4".to_string())
        );
    }

    #[tokio::test]
    async fn exec_tx_supports_optimistic_cas() {
        let store = setup_store();

        store
            .create("users/alice/guard", "v1")
            .await
            .expect("create guard");

        let mut first_tx = HashMap::new();
        first_tx.insert(
            "users/alice/data".to_string(),
            KVAction::Create("payload-1".to_string()),
        );
        store
            .exec_tx(first_tx, Some(("users/alice/guard".to_string(), 1)))
            .await
            .expect("first tx should pass");

        assert_eq!(
            store
                .get("users/alice/data".to_string())
                .await
                .expect("get payload"),
            Some("payload-1".to_string())
        );
        assert_eq!(
            store
                .get("__meta/revision/users/alice/guard".to_string())
                .await
                .expect("get guard revision"),
            Some("2".to_string())
        );

        let mut stale_tx = HashMap::new();
        stale_tx.insert(
            "users/alice/data".to_string(),
            KVAction::Update("payload-2".to_string()),
        );
        let err = store
            .exec_tx(stale_tx, Some(("users/alice/guard".to_string(), 1)))
            .await
            .expect_err("stale tx should fail");

        match err {
            KVStoreErrors::RevisionMismatch {
                key,
                expected,
                actual,
            } => {
                assert_eq!(key, "users/alice/guard");
                assert_eq!(expected, 1);
                assert_eq!(actual, 2);
            }
            other => panic!("unexpected error: {}", other),
        }

        assert_eq!(
            store
                .get("users/alice/data".to_string())
                .await
                .expect("get payload after failed tx"),
            Some("payload-1".to_string())
        );
    }

    #[tokio::test]
    async fn list_operations_hide_internal_meta_keys() {
        let store = setup_store();

        store
            .create("users/alice/profile", "v1")
            .await
            .expect("create profile");

        let root_children = store
            .list_direct_children("".to_string())
            .await
            .expect("list root");
        assert!(root_children.contains(&"users".to_string()));
        assert!(!root_children.contains(&"__meta".to_string()));

        let user_keys = store.list_keys("users/").await.expect("list users");
        assert_eq!(user_keys, vec!["users/alice/profile".to_string()]);

        let user_data = store.list_data("users/").await.expect("list user data");
        assert!(user_data.contains_key("users/alice/profile"));
        assert!(!user_data.contains_key("__meta/revision/users/alice/profile"));
    }
}
