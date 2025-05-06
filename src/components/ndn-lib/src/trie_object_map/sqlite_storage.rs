use crate::{NdnError, NdnResult};
use hash_db::{AsHashDB, HashDB, HashDBRef, Hasher as KeyHasher, Prefix};
use memory_db::KeyFunction;
use rusqlite::types::{FromSql, ToSql, ValueRef};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use std::borrow::Borrow;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct SqliteDB<H, KF, T>
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

impl<H, KF, T> SqliteDB<H, KF, T>
where
    H: KeyHasher,
    T: for<'a> From<&'a [u8]> + Default + AsRef<[u8]> + Clone + Send + Sync,
    KF: KeyFunction<H>,
    KF::Key: Borrow<[u8]> + for<'a> From<&'a [u8]>,
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
                key BLOB NOT NULL,
                value BLOB NOT NULL,
                ref_count INTEGER NOT NULL,
                PRIMARY KEY key
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

    fn replace(&self, key: &KF::Key, value: T) -> NdnResult<()> {
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
                        params![value.as_ref(), ref_count, key.borrow()],
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
                    "INSERT INTO trie_data (key, value, ref_count) VALUES (?1, ?2, ?3, -1)",
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

impl<H, KF, T> HashDB<H, T> for SqliteDB<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: Borrow<[u8]> + for<'a> From<&'a [u8]>,
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

        match self.remove_value(&key) {
            Ok(()) => (),
            Err(e) => {
                error!("Failed to remove value: {}", e);
            }
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

impl<H, KF, T> HashDBRef<H, T> for SqliteDB<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: Borrow<[u8]> + for<'a> From<&'a [u8]>,
{
    fn get(&self, key: &H::Out, prefix: Prefix) -> Option<T> {
        HashDB::get(self, key, prefix)
    }
    fn contains(&self, key: &H::Out, prefix: Prefix) -> bool {
        HashDB::contains(self, key, prefix)
    }
}

impl<H, KF, T> AsHashDB<H, T> for SqliteDB<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: Borrow<[u8]> + for<'a> From<&'a [u8]>,
{
    fn as_hash_db(&self) -> &dyn HashDB<H, T> {
        self
    }
    fn as_hash_db_mut(&mut self) -> &mut dyn HashDB<H, T> {
        self
    }
}
