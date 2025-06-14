use super::super::storage::HashDBWithFile;
use crate::{NdnError, NdnResult, TrieObjectMapStorageType};
use hash_db::{AsHashDB, HashDB, HashDBRef, Hasher as KeyHasher, Prefix};
use memory_db::KeyFunction;
use rusqlite::types::{FromSql, ToSql, ValueRef};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use std::borrow::Borrow;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

struct TrieObjectMapSqliteStorageChunkIterator<T> {
    conn: Arc<Mutex<Option<Connection>>>,
    buffer: VecDeque<(Vec<u8>, T)>,

    // Use last_key instead of offset to avoid large offset performance issue
    last_key: Option<Vec<u8>>,
    chunk_size: usize,

    finished: bool,
}

impl<T> TrieObjectMapSqliteStorageChunkIterator<T>
where
    T: for<'a> From<&'a [u8]> + Default + AsRef<[u8]> + Clone + Send + Sync,
{
    fn new(conn: Arc<Mutex<Option<Connection>>>, chunk_size: usize) -> Self {
        Self {
            conn,
            buffer: VecDeque::new(),
            last_key: None,
            chunk_size,
            finished: false,
        }
    }

    fn fetch_next_chunk(&mut self) -> NdnResult<()> {
        if self.finished {
            return Ok(());
        }

        let mut lock = self.conn.lock().unwrap();
        let conn = lock.as_mut().ok_or_else(|| {
            let msg = "Connection is not initialized".to_string();
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let query = if let Some(ref last_key) = self.last_key {
            "SELECT key, value, ref_count FROM trie_data WHERE key > ?1 ORDER BY key LIMIT ?2"
        } else {
            "SELECT key, value, ref_count FROM trie_data ORDER BY key LIMIT ?1 OFFSET ?2"
        };

        let mut stmt = conn.prepare(&query).map_err(|e| {
            let msg = format!("Failed to prepare next statement: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let params = if self.last_key.is_some() {
            params![self.last_key.as_ref(), self.chunk_size as u64]
        } else {
            params![self.chunk_size as u64, 0u64]
        };

        let rows = stmt
            .query_map(params, |r| {
                let key: Vec<u8> = r.get(0)?;
                let value: Vec<u8> = r.get(1)?;
                let value: T = T::from(value.as_ref());
                let ref_count: i32 = r.get(2)?;
                Ok((key, value, ref_count))
            })
            .map_err(|e| {
                let msg = format!("Failed to query object_map: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        for row in rows {
            let (key, value, ref_count) = row.map_err(|e| {
                let msg = format!("Failed to map row: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

            if ref_count <= 0 {
                // Skip rows with ref_count <= 0
                continue;
            }

            self.buffer.push_back((key, value));
        }

        // Update last_key to the last key in the current buffer
        if let Some(last) = self.buffer.back() {
            self.last_key = Some(last.0.clone());
        } else {
            self.last_key = None;
            self.finished = true;
        }

        Ok(())
    }
}

impl<T> Iterator for TrieObjectMapSqliteStorageChunkIterator<T>
where
    T: for<'a> From<&'a [u8]> + Default + AsRef<[u8]> + Clone + Send + Sync,
{
    type Item = (Vec<u8>, T);

    fn next(&mut self) -> Option<Self::Item> {
        if self.buffer.is_empty() {
            if let Err(e) = self.fetch_next_chunk() {
                error!("Failed to fetch next chunk: {}", e);
                return None;
            }
        }

        self.buffer.pop_front()
    }
}

pub(crate) struct SqliteStorage<H, KF, T>
where
    H: KeyHasher,
    KF: KeyFunction<H>,
{
    file: PathBuf,
    read_only: bool,

    // data: Map<KF::Key, (T, i32)>,
    conn: Arc<Mutex<Option<Connection>>>,

    hashed_null_node: H::Out,
    null_node_data: T,
    _kf: PhantomData<KF>,
}

impl<H, KF, T> Default for SqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: for<'a> From<&'a [u8]>,
    KF: KeyFunction<H>,
{
    fn default() -> Self {
        unimplemented!(
            "Default is not implemented for SqliteStorage, please use SqliteStorage::new() instead"
        )
    }
}

impl<H, KF, T> SqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: for<'a> From<&'a [u8]> + Default + AsRef<[u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: AsRef<[u8]>,
{
    pub fn new(db_path: PathBuf, read_only: bool) -> NdnResult<Self> {
        let conn = Connection::open(&db_path).map_err(|e| {
            let msg = format!("Failed to open SQLite database: {:?}, {}", db_path, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Self::init_tables(&conn)?;

        Ok(Self::new_with_connection(db_path, conn, read_only))
    }

    fn new_with_connection(db_path: PathBuf, conn: Connection, read_only: bool) -> Self {
        Self {
            read_only,
            file: db_path,
            conn: Arc::new(Mutex::new(Some(conn))),
            hashed_null_node: H::hash(&[0u8][..]),
            null_node_data: [0u8][..].into(),
            _kf: PhantomData,
        }
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
        let conn = conn.as_ref().unwrap();

        let result = conn
            .query_row(
                "SELECT value, ref_count FROM trie_data WHERE key = ?1",
                params![key.as_ref()],
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
        let conn = conn.as_mut().unwrap();

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
            params![key.as_ref()],
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
                        params![value.as_ref(), ref_count + 1, key.as_ref()],
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
                        params![key.as_ref()],
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
                    params![key.as_ref(), value.as_ref()],
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
        let conn = conn.as_mut().unwrap();

        let tx = conn.transaction().map_err(|e| {
            let msg = format!("Failed to begin transaction: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        // First check the reference count
        let ref_count: Option<i32> = tx
            .query_row(
                "SELECT ref_count FROM trie_data WHERE key = ?1",
                params![key.as_ref()],
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
                    params![key.as_ref()],
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
                    params![key.as_ref(), default_value],
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

    async fn clone_for_modify(&self, target: &Path) -> NdnResult<Self> {
        // First check if target is same as current file
        if target == self.file {
            let msg = format!("Target file is same as current file: {}", target.display());
            error!("{}", msg);
            return Err(NdnError::AlreadyExists(msg));
        }

        // Open new connection to target file
        let mut new_conn = Connection::open(target).map_err(|e| {
            let msg = format!("Failed to open SQLite database: {:?}, {}", target, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let mut lock = self.conn.lock().unwrap();
        let mut conn = lock.as_ref().unwrap();
        let backup = rusqlite::backup::Backup::new(&conn, &mut new_conn).map_err(|e| {
            let msg = format!(
                "Failed to create backup: {:?} -> {:?}, {}",
                self.file, target, e
            );
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        backup
            .run_to_completion(64, std::time::Duration::from_millis(5), None)
            .map_err(|e| {
                let msg = format!(
                    "Failed to run backup: {:?} -> {:?}, {}",
                    self.file, target, e
                );
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        drop(backup);
        drop(lock);

        let new_storage = Self::new_with_connection(target.to_path_buf(), new_conn, false);
        Ok(new_storage)
    }

    async fn save(&mut self, file: &Path) -> NdnResult<()> {
        // Check if file is same as current file
        if file == self.file {
            warn!("Target file is same as current file: {}", file.display());
            return Ok(());
        }

        // Check if target file exists
        if file.exists() {
            warn!(
                "Target object map storage file already exists: {}, now will overwrite it",
                file.display()
            );
        }

        // First close the current connection, then try to rename the file, and then open a new connection to the file.
        let mut lock = self.conn.lock().unwrap();
        let mut conn = lock.take().unwrap();
        conn.close().map_err(|e| {
            let msg = format!("Failed to close SQLite database: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        std::fs::rename(&self.file, file).map_err(|e| {
            let msg = format!(
                "Failed to rename SQLite database: {:?} -> {:?}, {}",
                self.file, file, e
            );
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        info!("Renamed SQLite database: {:?} -> {:?}", self.file, file);

        // Open a new connection to the file
        let new_conn = Connection::open(file).map_err(|e| {
            let msg = format!("Failed to open SQLite database: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        *lock = Some(new_conn);
        self.file = file.to_path_buf();

        Ok(())
    }
}

impl<H, KF, T> HashDB<H, T> for SqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: AsRef<[u8]>,
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

impl<H, KF, T> HashDBRef<H, T> for SqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: AsRef<[u8]>,
{
    fn get(&self, key: &H::Out, prefix: Prefix) -> Option<T> {
        HashDB::get(self, key, prefix)
    }
    fn contains(&self, key: &H::Out, prefix: Prefix) -> bool {
        HashDB::contains(self, key, prefix)
    }
}

impl<H, KF, T> AsHashDB<H, T> for SqliteStorage<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
    KF::Key: AsRef<[u8]>,
{
    fn as_hash_db(&self) -> &dyn HashDB<H, T> {
        self
    }
    fn as_hash_db_mut(&mut self) -> &mut dyn HashDB<H, T> {
        self
    }
}

#[async_trait::async_trait]
impl<H, KF, T> HashDBWithFile<H, T> for SqliteStorage<H, KF, T>
where
    H: KeyHasher + 'static,
    T: Default
        + PartialEq<T>
        + AsRef<[u8]>
        + for<'a> From<&'a [u8]>
        + Clone
        + Send
        + Sync
        + 'static,
    KF: KeyFunction<H> + Send + Sync + 'static,
    KF::Key: AsRef<[u8]>,
{
    fn get_type(&self) -> TrieObjectMapStorageType {
        TrieObjectMapStorageType::SQLite
    }

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Vec<u8>, T)> + 'a> {
        let conn = self.conn.clone();
        let chunk_size: usize = 64; // Adjust chunk size as needed
        let iter = TrieObjectMapSqliteStorageChunkIterator::new(conn, chunk_size);
        Box::new(iter)
    }

    // Clone the storage to a new file.
    // If the target file exists, it will be failed.
    async fn clone(
        &self,
        target: &Path,
        read_only: bool,
    ) -> NdnResult<Box<dyn HashDBWithFile<H, T>>> {
        if read_only {
            let ret = Self::new(target.to_path_buf(), read_only)?;
            Ok(Box::new(ret) as Box<dyn HashDBWithFile<H, T>>)
        } else {
            let ret = self.clone_for_modify(target).await?;
            Ok(Box::new(ret) as Box<dyn HashDBWithFile<H, T>>)
        }
    }

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, file: &Path) -> NdnResult<()> {
        self.save(file).await
    }
}

#[cfg(test)]
mod test {
    use crate::trie_object_map::storage;

    use super::super::super::hash::Sha256Hasher;
    use super::*;
    use hash_db::HashDB;
    use memory_db::{HashKey, MemoryDB};
    use std::{hash::Hash, path::PathBuf, vec};

    #[test]
    fn test_trie_object_map_sqlite_storage() {
        type TestStorage = SqliteStorage<Sha256Hasher, HashKey<Sha256Hasher>, Vec<u8>>;
        type H = Sha256Hasher;
        type TestMemoryDB = MemoryDB<Sha256Hasher, HashKey<Sha256Hasher>, Vec<u8>>;

        buckyos_kit::init_logging("test-trie-object-map", false);

        // Get system temp directory
        let data_dir = std::env::temp_dir().join("ndn-test-trie-object-map");
        if !data_dir.exists() {
            println!("Creating test data directory: {:?}", data_dir);
            std::fs::create_dir_all(&data_dir).unwrap();
        } else {
            println!("Using existing test data directory: {:?}", data_dir);
        }

        let db_path = data_dir.join("test_trie_object_map.sqlite");
        if db_path.exists() {
            println!("Removing existing test database file: {:?}", db_path);
            std::fs::remove_file(&db_path).unwrap();
        }
        let mut storage = TestStorage::new(db_path, false).unwrap();
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

// use super::super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use memory_db::HashKey;

pub type TrieObjectMapSqliteStorage<H> = SqliteStorage<H, HashKey<H>, Vec<u8>>;
// pub type TrieObjectMapSqliteSha256Storage = TrieObjectMapSqliteStorage<Sha256Hasher>;
// pub type TrieObjectMapSqliteSha512Storage = TrieObjectMapSqliteStorage<Sha512Hasher>;
// pub type TrieObjectMapSqliteBlake2s256Storage = TrieObjectMapSqliteStorage<Blake2s256Hasher>;
// pub type TrieObjectMapSqliteKeccak256Storage = TrieObjectMapSqliteStorage<Keccak256Hasher>;
