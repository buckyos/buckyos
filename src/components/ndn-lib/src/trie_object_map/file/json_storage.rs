use super::super::storage::HashFromSlice;
use super::super::storage::TrieObjectMapInnerStorage;
use super::super::storage::{HashDBWithFile, TrieObjectMapStorageType};
use crate::{NdnError, NdnResult};
use base64::{engine::general_purpose, Engine as _};
use hash_db::{
    AsHashDB, AsPlainDB, HashDB, HashDBRef, Hasher as KeyHasher, MaybeDebug, PlainDB, PlainDBRef,
    Prefix,
};
use memory_db::KeyFunction;
use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::{
    collections::hash_map::Entry, collections::HashMap as Map, hash, marker::PhantomData, mem,
};

// Fork of `memory_db` to support file-based storage with a memory database.
pub struct MemoryDBExt<H, KF, T>
where
    H: KeyHasher,
    KF: KeyFunction<H>,
{
    file: PathBuf,
    read_only: bool,
    is_dirty: bool,

    data: Map<KF::Key, (T, i32)>,
    hashed_null_node: H::Out,
    null_node_data: T,
    _kf: PhantomData<KF>,
}

impl<H, KF, T> Default for MemoryDBExt<H, KF, T>
where
    H: KeyHasher,
    T: for<'a> From<&'a [u8]>,
    KF: KeyFunction<H>,
{
    fn default() -> Self {
        unimplemented!("Default for MemoryDBExt requires a null key/data");
    }
}

impl<H, KF, T> MemoryDBExt<H, KF, T>
where
    H: KeyHasher,
    T: for<'a> From<&'a [u8]> + AsRef<[u8]> + Clone,
    KF::Key: AsRef<[u8]> + HashFromSlice,
    KF: KeyFunction<H>,
{
    pub fn new(file: PathBuf, read_only: bool) -> NdnResult<Self> {
        let hashed_null_node = H::hash(&[]);
        let null_node_data = T::from(&[]);

        let mut ret = Self {
            file: file.clone(),
            read_only,
            is_dirty: false,
            data: Map::new(),
            hashed_null_node,
            null_node_data,
            _kf: PhantomData,
        };

        if ret.file.exists() {
            info!("Loading memory database from file: {:?}", file);
            ret.load(&file)?;
        }

        Ok(ret)
    }

    async fn clone_for_modify(&self, target: &Path) -> NdnResult<Self> {
        // First check if target is same as current file
        if target == self.file {
            let msg = format!("Target file is same as current file: {}", target.display());
            error!("{}", msg);
            return Err(NdnError::AlreadyExists(msg));
        }

        if self.is_dirty {
            // Clone the current storage to a new file
            let mut new_storage = Self {
                read_only: false,
                file: target.to_path_buf(),
                data: self.data.clone(),
                is_dirty: false,

                hashed_null_node: self.hashed_null_node,
                null_node_data: self.null_node_data.clone(),
                _kf: PhantomData,
            };

            new_storage.save(&target)?;
            Ok(new_storage)
        } else {
            // If the storage is not dirty, just copy the file
            tokio::fs::copy(&self.file, target).await.map_err(|e| {
                let msg = format!("Failed to copy file: {:?}, {}", target, e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;

            // Create a new storage instance
            let new_storage = Self {
                read_only: false,
                file: target.to_path_buf(),
                data: self.data.clone(),
                is_dirty: false,

                hashed_null_node: self.hashed_null_node,
                null_node_data: self.null_node_data.clone(),
                _kf: PhantomData,
            };

            // Return the new storage
            Ok(new_storage)
        }
    }

    /// Clear all data from the database.
    pub fn clear(&mut self) {
        self.data.clear();
        self.is_dirty = true;
    }
}

impl<H, KF, T> HashDB<H, T> for MemoryDBExt<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
{
    fn get(&self, key: &H::Out, prefix: Prefix) -> Option<T> {
        if key == &self.hashed_null_node {
            return Some(self.null_node_data.clone());
        }

        let key = KF::key(key, prefix);
        match self.data.get(&key) {
            Some(&(ref d, rc)) if rc > 0 => Some(d.clone()),
            _ => None,
        }
    }

    fn contains(&self, key: &H::Out, prefix: Prefix) -> bool {
        if key == &self.hashed_null_node {
            return true;
        }

        let key = KF::key(key, prefix);
        match self.data.get(&key) {
            Some(&(_, x)) if x > 0 => true,
            _ => false,
        }
    }

    fn emplace(&mut self, key: H::Out, prefix: Prefix, value: T) {
        if value == self.null_node_data {
            return;
        }

        let key = KF::key(&key, prefix);
        match self.data.entry(key) {
            Entry::Occupied(mut entry) => {
                let &mut (ref mut old_value, ref mut rc) = entry.get_mut();
                if *rc <= 0 {
                    *old_value = value;
                }
                *rc += 1;
            }
            Entry::Vacant(entry) => {
                entry.insert((value, 1));
            }
        }

        self.is_dirty = true;
    }

    fn insert(&mut self, prefix: Prefix, value: &[u8]) -> H::Out {
        if T::from(value) == self.null_node_data {
            return self.hashed_null_node;
        }

        self.is_dirty = true;

        let key = H::hash(value);
        HashDB::emplace(self, key, prefix, value.into());
        key
    }

    fn remove(&mut self, key: &H::Out, prefix: Prefix) {
        if key == &self.hashed_null_node {
            return;
        }

        let key = KF::key(key, prefix);
        match self.data.entry(key) {
            Entry::Occupied(mut entry) => {
                let &mut (_, ref mut rc) = entry.get_mut();
                *rc -= 1;
            }
            Entry::Vacant(entry) => {
                let value = T::default();
                entry.insert((value, -1));
            }
        }

        self.is_dirty = true;
    }
}

impl<H, KF, T> HashDBRef<H, T> for MemoryDBExt<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
{
    fn get(&self, key: &H::Out, prefix: Prefix) -> Option<T> {
        HashDB::get(self, key, prefix)
    }
    fn contains(&self, key: &H::Out, prefix: Prefix) -> bool {
        HashDB::contains(self, key, prefix)
    }
}

impl<H, KF, T> AsHashDB<H, T> for MemoryDBExt<H, KF, T>
where
    H: KeyHasher,
    T: Default + PartialEq<T> + AsRef<[u8]> + for<'a> From<&'a [u8]> + Clone + Send + Sync,
    KF: KeyFunction<H> + Send + Sync,
{
    fn as_hash_db(&self) -> &dyn HashDB<H, T> {
        self
    }
    fn as_hash_db_mut(&mut self) -> &mut dyn HashDB<H, T> {
        self
    }
}

trait MemoryDBFileExt<H, KF, T>
where
    H: KeyHasher,
    KF: KeyFunction<H>,
{
    fn load(&mut self, file_path: &Path) -> NdnResult<()>;
    fn save(&self, file_path: &Path) -> NdnResult<()>;
}

impl<H, KF, T> MemoryDBFileExt<H, KF, T> for MemoryDBExt<H, KF, T>
where
    H: KeyHasher,
    T: AsRef<[u8]> + for<'a> From<&'a [u8]>,
    KF::Key: AsRef<[u8]> + HashFromSlice,
    KF: KeyFunction<H>,
{
    fn load(&mut self, file_path: &Path) -> NdnResult<()> {
        if !file_path.exists() {
            let msg = format!("File does not exist: {:?}", file_path);
            error!("{}", msg);
            return Err(NdnError::IoError(msg));
        }

        let file = std::fs::File::open(&file_path).map_err(|e| {
            let msg = format!("Failed to open file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        // Deserialize the data from JSON
        let data: HashMap<String, (String, i32)> = serde_json::from_reader(file).map_err(|e| {
            let msg = format!("Failed to read data from file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        self.data.clear();

        for (encoded_key, (encoded_value, index)) in data {
            let k: Vec<u8> = general_purpose::STANDARD
                .decode(&encoded_key)
                .map_err(|e| {
                    let msg = format!("Failed to decode key: {}, {}", encoded_key, e);
                    error!("{}", msg);
                    NdnError::IoError(msg)
                })?;

            let v: Vec<u8> = general_purpose::STANDARD
                .decode(&encoded_value)
                .map_err(|e| {
                    let msg = format!("Failed to decode value: {}, {}", encoded_value, e);
                    error!("{}", msg);
                    NdnError::IoError(msg)
                })?;

            self.data
                .insert(KF::Key::from_slice(&k)?, (v.as_slice().into(), index));
        }

        Ok(())
    }

    fn save(&self, file_path: &Path) -> NdnResult<()> {
        // Serialize the data to JSON and write it to the file
        let f = std::fs::File::create(&file_path).map_err(|e| {
            let msg = format!("Failed to create file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        use base64::{engine::general_purpose, Engine as _};

        let encoded = self
            .data
            .iter()
            .map(|(k, (v, i))| {
                let encoded_key = general_purpose::STANDARD.encode(k);
                let encoded_value = general_purpose::STANDARD.encode(v);
                (encoded_key, (encoded_value, *i))
            })
            .collect::<HashMap<String, (String, i32)>>();

        serde_json::to_writer(f, &encoded).map_err(|e| {
            let msg = format!("Failed to write data to file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        info!("Saved JSON storage to file: {:?}", file_path);

        Ok(())
    }
}

#[async_trait::async_trait]
impl<H, KF, T> HashDBWithFile<H, T> for MemoryDBExt<H, KF, T>
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
    KF::Key: AsRef<[u8]> + HashFromSlice + 'static,
{
    fn get_type(&self) -> TrieObjectMapStorageType {
        TrieObjectMapStorageType::JSONFile
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

use memory_db::HashKey;

pub type TrieObjectMapJSONFileStorage<H> = MemoryDBExt<H, HashKey<H>, Vec<u8>>;
