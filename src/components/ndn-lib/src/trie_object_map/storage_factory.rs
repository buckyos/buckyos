use super::file::TrieObjectMapSqliteStorage;
use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use super::inner_storage::TrieObjectMapInnerStorageWrapper;
use super::memory_storage::TrieObjectMapMemoryStorage;
use super::storage::{
    GenericTrieObjectMapProofVerifier, HashFromSlice, TrieObjectMapProofVerifier,
};
use super::storage::{TrieObjectMapInnerStorage, TrieObjectMapStorageType};
use crate::{HashMethod, NdnError, NdnResult};
use hash_db::{HashDB, Hasher};
use once_cell::sync::OnceCell;
use std::path::{Path, PathBuf};

pub struct TrieObjectMapStorageFactory {
    data_dir: PathBuf,
    default_storage_type: TrieObjectMapStorageType,
}

impl TrieObjectMapStorageFactory {
    pub fn new(data_dir: &Path, default_storage_type: Option<TrieObjectMapStorageType>) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            default_storage_type: default_storage_type
                .unwrap_or(TrieObjectMapStorageType::default()),
        }
    }

    pub fn create_storage<H>(
        &self,
        storage_type: TrieObjectMapStorageType,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>>
    where
        H: Hasher + Send + Sync + 'static,
        H::Out: HashFromSlice + std::borrow::Borrow<[u8]>,
    {
        let db = match storage_type {
            TrieObjectMapStorageType::Memory => {
                let db = TrieObjectMapMemoryStorage::<H>::default();
                Box::new(db) as Box<dyn HashDB<H, Vec<u8>>>
            }
            TrieObjectMapStorageType::SQLite => {
                // For SQLite storage, we can use a SQLite-based implementation
                let file = self.data_dir.join("trie_object_map.sqlite");
                Box::new(TrieObjectMapSqliteStorage::<H>::new(file, false)?)
                    as Box<dyn HashDB<H, Vec<u8>>>
            }
        };

        Ok(Box::new(TrieObjectMapInnerStorageWrapper::<H>::new(db)))
    }

    pub fn create_storage_by_hash_method(
        &self,
        storage_type: TrieObjectMapStorageType,
        hash_method: HashMethod,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>> {
        match hash_method {
            HashMethod::Sha256 => self.create_storage::<Sha256Hasher>(storage_type),
            HashMethod::Sha512 => self.create_storage::<Sha512Hasher>(storage_type),
            HashMethod::Blake2s256 => self.create_storage::<Blake2s256Hasher>(storage_type),
            HashMethod::Keccak256 => self.create_storage::<Keccak256Hasher>(storage_type),
        }
    }

    pub fn create_verifier<H>() -> Box<dyn TrieObjectMapProofVerifier>
    where
        H: Hasher + Send + Sync + 'static,
        H::Out: HashFromSlice + std::borrow::Borrow<[u8]>,
    {
        Box::new(GenericTrieObjectMapProofVerifier::<H>::new())
    }

    pub fn create_verifier_by_hash_method(
        hash_method: HashMethod,
    ) -> Box<dyn TrieObjectMapProofVerifier> {
        match hash_method {
            HashMethod::Sha256 => Self::create_verifier::<Sha256Hasher>(),
            HashMethod::Sha512 => Self::create_verifier::<Sha512Hasher>(),
            HashMethod::Blake2s256 => Self::create_verifier::<Blake2s256Hasher>(),
            HashMethod::Keccak256 => Self::create_verifier::<Keccak256Hasher>(),
        }
    }
}

pub static GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY: OnceCell<TrieObjectMapStorageFactory> =
    OnceCell::new();
