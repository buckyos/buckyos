use crate::{NdnError, NdnResult};
use hash_db::{AsHashDB, HashDB, HashDBRef, Hasher as KeyHasher, Prefix};
use memory_db::KeyFunction;
use rusqlite::types::{FromSql, ToSql, ValueRef};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use std::borrow::Borrow;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct TrieObjectMapSqliteStorage<H, KF, T>
where
    H: KeyHasher,
    KF: KeyFunction<H>,
{
    // data: Map<KF::Key, (T, i32)>,
    conn: Arc<Mutex<Connection>>,

    hashed_null_node: H::Out,
    null_node_data: T,
    _kf: PhantomData<KF>,
}

impl<H, KF, T> TrieObjectMapSqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: for<'a> From<&'a [u8]> + Default + AsRef<[u8]> + Clone + Send + Sync,
    KF: KeyFunction<H>,
    KF::Key: Borrow<[u8]>,
{
    pub fn new(db_path: &Path) -> NdnResult<Self> {
        let conn = Connection::open(db_path).map_err(|e| {
            let msg = format!("Failed to open SQLite database: {:?}, {}", db_path, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Self::init_tables(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            hashed_null_node: H::hash(&[0u8][..]),
            null_node_data: [0u8][..].into(),
            _kf: PhantomData,
        })
    }

    fn init_tables(conn: &Connection) -> NdnResult<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS trie_data (
                key BLOB NOT NULL PRIMARY KEY,
                value BLOB NOT NULL,
                ref_count INTEGER NOT NULL
            );
            ",
        )
        .map_err(|e| {
            let msg = format!("Failed to create tables: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })
    }

    fn get_value(&self, key: &KF::Key) -> NdnResult<Option<T>> {
        let conn = self.conn.lock().unwrap();

        let result = conn
            .query_row(
                "SELECT value, ref_count FROM trie_data WHERE key = ?1",
                params![key.borrow()],
                |row| {
                    let ref_count: i32 = row.get(1)?;
                    if ref_count > 0 {
                        let value: Vec<u8> = row.get(0)?;
                        let value = T::from(value.as_ref());
                        Ok(Some(value))
                    } else {
                        Ok(None)
                    }
                },
            )
            .optional()
            .map_err(|e| {
                let msg = format!("Failed to query row: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        Ok(result.flatten())
    }

    fn emplace_value(&self, key: &KF::Key, value: T) -> NdnResult<()> {
        // Use transaction to ensure atomicity
        let mut conn = self.conn.lock().unwrap();

        // Use transaction to ensure atomicity
        let tx = conn.transaction().map_err(|e| {
            let msg = format!("Failed to begin transaction: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        // Get current value and reference count
        let mut existing: Option<(Vec<u8>, i32)> = None;
        tx.query_row(
            "SELECT value, ref_count FROM trie_data WHERE key = ?1",
            params![key.borrow()],
            |row| {
                let value: Vec<u8> = row.get(0)?;
                let ref_count: i32 = row.get(1)?;
                existing = Some((value, ref_count));

                Ok(())
            },
        )
        .optional()
        .map_err(|e| {
            let msg = format!("Failed to query row: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        match existing {
            Some((old_value, ref_count)) => {
                if ref_count <= 0 {
                    // Update value and reference count
                    tx.execute(
                        "UPDATE trie_data SET value = ?1, ref_count = ?2 WHERE key = ?3",
                        params![value.as_ref(), ref_count + 1, key.borrow()],
                    )
                    .map_err(|e| {
                        let msg = format!("Failed to update: {}", e);
                        error!("{}", msg);
                        NdnError::DbError(msg)
                    })?;
                } else {
                    // Only increase reference count
                    tx.execute(
                        "UPDATE trie_data SET ref_count = ref_count + 1 WHERE key = ?1",
                        params![key.borrow()],
                    )
                    .map_err(|e| {
                        let msg = format!("Failed to update ref_count: {}", e);
                        error!("{}", msg);
                        NdnError::DbError(msg)
                    })?;
                }
            }
            None => {
                // Insert new record
                tx.execute(
                    "INSERT INTO trie_data (key, value, ref_count) VALUES (?1, ?2, 1)",
                    params![key.borrow(), value.as_ref()],
                )
                .map_err(|e| {
                    let msg = format!("Failed to insert: {}", e);
                    error!("{}", msg);
                    NdnError::DbError(msg)
                })?;
            }
        }

        // Commit the transaction
        tx.commit().map_err(|e| {
            let msg = format!("Failed to commit transaction: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(())
    }

    fn remove_value(&self, key: &KF::Key) -> NdnResult<()> {
        // Use transaction to ensure atomicity
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| {
            let msg = format!("Failed to begin transaction: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        // First check the reference count
        let ref_count: Option<i32> = tx
            .query_row(
                "SELECT ref_count FROM trie_data WHERE key = ?1",
                params![key.borrow()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| {
                let msg = format!("Failed to query row: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        match ref_count {
            Some(rc) => {
                // Decrease the reference count
                tx.execute(
                    "UPDATE trie_data SET ref_count = ref_count - 1 WHERE key = ?1",
                    params![key.borrow()],
                )
                .map_err(|e| {
                    let msg = format!("Failed to update ref_count: {}", e);
                    error!("{}", msg);
                    NdnError::DbError(msg)
                })?;
            }
            None => {
                // Insert a new row with ref_count = -1
                let default_value: Vec<u8> = T::default().as_ref().to_vec();
                tx.execute(
                    "INSERT INTO trie_data (key, value, ref_count) VALUES (?1, ?2, -1)",
                    params![key.borrow(), default_value],
                )
                .map_err(|e| {
                    let msg = format!("Failed to insert new row: {}", e);
                    error!("{}", msg);
                    NdnError::DbError(msg)
                })?;
            }
        };

        // Commit the transaction
        tx.commit().map_err(|e| {
            let msg = format!("Failed to commit transaction: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(())
    }
}

impl<H, KF, T> HashDB<H, T> for TrieObjectMapSqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: Borrow<[u8]>,
{
    fn get(&self, key: &H::Out, prefix: Prefix) -> Option<T> {
        if key == &self.hashed_null_node {
            return Some(self.null_node_data.clone());
        }

        let key = KF::key(key, prefix);
        match self.get_value(&key) {
            Ok(Some(value)) => Some(value),
            Ok(None) => None,
            Err(e) => {
                error!("Failed to get value: {}", e);
                None
            }
        }
    }

    fn contains(&self, key: &H::Out, prefix: Prefix) -> bool {
        if key == &self.hashed_null_node {
            return true;
        }

        let key = KF::key(key, prefix);
        match self.get_value(&key) {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(e) => {
                error!("Failed to check existence: {}", e);
                false
            }
        }
    }

    fn emplace(&mut self, key: H::Out, prefix: Prefix, value: T) {
        if value == self.null_node_data {
            return;
        }

        let key = KF::key(&key, prefix);
        if let Err(e) = self.emplace_value(&key, value) {
            error!("Failed to remove value: {}", e);
        }
    }

    fn insert(&mut self, prefix: Prefix, value: &[u8]) -> H::Out {
        if T::from(value) == self.null_node_data {
            return self.hashed_null_node;
        }

        let key = H::hash(value);
        HashDB::emplace(self, key, prefix, value.into());
        key
    }

    fn remove(&mut self, key: &H::Out, prefix: Prefix) {
        if key == &self.hashed_null_node {
            return;
        }

        let key = KF::key(key, prefix);
        match self.remove_value(&key) {
            Ok(()) => (),
            Err(e) => {
                error!("Failed to remove value: {}", e);
            }
        }
    }
}

impl<H, KF, T> HashDBRef<H, T> for TrieObjectMapSqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: Borrow<[u8]>,
{
    fn get(&self, key: &H::Out, prefix: Prefix) -> Option<T> {
        HashDB::get(self, key, prefix)
    }
    fn contains(&self, key: &H::Out, prefix: Prefix) -> bool {
        HashDB::contains(self, key, prefix)
    }
}

impl<H, KF, T> AsHashDB<H, T> for TrieObjectMapSqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: Borrow<[u8]>,
{
    fn as_hash_db(&self) -> &dyn HashDB<H, T> {
        self
    }
    fn as_hash_db_mut(&mut self) -> &mut dyn HashDB<H, T> {
        self
    }
}

#[cfg(test)]
mod test {
    use crate::trie_object_map::storage;

    use super::super::hash::Sha256Hasher;
    use super::*;
    use hash_db::HashDB;
    use memory_db::{HashKey, MemoryDB};
    use std::{hash::Hash, path::PathBuf, vec};

    #[test]
    fn test_trie_object_map_sqlite_storage() {
        type TestStorage = TrieObjectMapSqliteStorage<Sha256Hasher, HashKey<Sha256Hasher>, Vec<u8>>;
        type H = Sha256Hasher;
        type TestMemoryDB = MemoryDB<Sha256Hasher, HashKey<Sha256Hasher>, Vec<u8>>;

        buckyos_kit::init_logging("test-trie-object-map", false);

        let data_dir = std::env::temp_dir().join("ndn-test-trie-object-map");
        let db_path = PathBuf::from("test.db");
        if db_path.exists() {
            println!("Removing existing test database file: {:?}", db_path);
            std::fs::remove_file(&db_path).unwrap();
        }
        let mut storage = TestStorage::new(&db_path).unwrap();
        //let mut storage = TestMemoryDB::default();

        // Test as HashDB
        let value = b"test_value".to_vec();
        let key = H::hash(&value);
        let node = vec![0u8; 32];
        let prefix = (node.as_ref(), None);

        HashDB::insert(&mut storage, prefix, &value);
        let retrieved_value = HashDB::get(&storage, &key, prefix).unwrap();
        assert_eq!(retrieved_value, value);
        assert!(HashDB::contains(&storage, &key, prefix));

        storage.remove(&key, prefix);
        assert!(!HashDB::contains(&storage, &key, prefix));
        assert!(HashDB::get(&storage, &key, prefix).is_none());
        assert!(HashDB::get(&storage, &H::hash(b"non_existent_key"), prefix).is_none());

        // Insert one value twice and then should be removed twice before it is really removed
        {
            let value = b"test_value1".to_vec();
            let key = H::hash(&value);
            let node = vec![0u8; 32];
            let prefix = (node.as_ref(), None);

            HashDB::insert(&mut storage, prefix, &value);
            HashDB::insert(&mut storage, prefix, &value);

            HashDB::remove(&mut storage, &key, prefix);

            // Get the value again, it should be existing
            let retrieved_value = HashDB::get(&storage, &key, prefix).unwrap();
            assert_eq!(retrieved_value, value);

            assert!(HashDB::contains(&storage, &key, prefix));

            // Remove the value again, it should be removed
            HashDB::remove(&mut storage, &key, prefix);

            assert!(!HashDB::contains(&storage, &key, prefix));
            assert!(HashDB::get(&storage, &key, prefix).is_none());
        }

        // Test first remove and then insert again
        {
            let value = b"test_value2".to_vec();
            let key = H::hash(&value);
            let node = vec![0u8; 32];
            let prefix = (node.as_ref(), None);

            // First remove the value
            HashDB::remove(&mut storage, &key, prefix);
            assert!(!HashDB::contains(&storage, &key, prefix));
            assert!(HashDB::get(&storage, &key, prefix).is_none());

            // First insert the value will not be existing
            HashDB::insert(&mut storage, prefix, &value);
            assert!(!HashDB::contains(&storage, &key, prefix));
            assert!(HashDB::get(&storage, &key, prefix).is_none());

            // Insert the value again, it should be existing
            HashDB::insert(&mut storage, prefix, &value);
            assert!(HashDB::contains(&storage, &key, prefix));
            let retrieved_value = HashDB::get(&storage, &key, prefix).unwrap();
            assert_eq!(retrieved_value, value);
        }
    }
}
