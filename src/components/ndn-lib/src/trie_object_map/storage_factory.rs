use super::file::{TrieObjectMapJSONFileStorage, TrieObjectMapSqliteStorage};
use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use super::inner_storage::TrieObjectMapInnerStorageWrapper;
use super::storage::{
    GenericTrieObjectMapProofVerifier, HashFromSlice, TrieObjectMapProofVerifier,
};
use super::storage::{TrieObjectMapInnerStorage, TrieObjectMapStorageType};
use crate::{Base32Codec, HashDBWithFile, HashMethod, NdnError, NdnResult, ObjId};
use hash_db::{HashDB, Hasher};
use once_cell::sync::OnceCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TrieObjectMapStorageFactory {
    data_dir: PathBuf,
    default_storage_type: TrieObjectMapStorageType,
    temp_file_index: AtomicU64,
}

impl TrieObjectMapStorageFactory {
    pub fn new(data_dir: PathBuf, default_storage_type: Option<TrieObjectMapStorageType>) -> Self {
        Self {
            data_dir,
            default_storage_type: default_storage_type
                .unwrap_or(TrieObjectMapStorageType::default()),
            temp_file_index: AtomicU64::new(0),
        }
    }

    pub fn get_file_path_by_id(
        &self,
        root_hash: Option<&str>,
        storage_type: TrieObjectMapStorageType,
    ) -> PathBuf {
        let file_name = match storage_type {
            TrieObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file path");
            }
            TrieObjectMapStorageType::SQLite => {
                if let Some(id) = root_hash {
                    id.to_string()
                } else {
                    self.get_temp_file_name(storage_type)
                }
            }
            TrieObjectMapStorageType::JSONFile => {
                if let Some(id) = root_hash {
                    id.to_string()
                } else {
                    self.get_temp_file_name(storage_type)
                }
            }
        };

        self.get_file_path(&file_name, storage_type)
    }

    fn get_file_path(&self, file_name: &str, storage_type: TrieObjectMapStorageType) -> PathBuf {
        match storage_type {
            TrieObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file path");
            }
            TrieObjectMapStorageType::SQLite => {
                let file_name = format!("{}.sqlite", file_name);
                self.data_dir.join(file_name)
            }
            TrieObjectMapStorageType::JSONFile => {
                let file_name = format!("{}.json", file_name);

                self.data_dir.join(file_name)
            }
        }
    }

    pub async fn open_by_hash_method(
        &self,
        root_hash: Option<&str>,
        read_only: bool,
        storage_type: Option<TrieObjectMapStorageType>,
        hash_method: HashMethod,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>> {
        match hash_method {
            HashMethod::Sha256 => {
                self.open::<Sha256Hasher>(root_hash, read_only, storage_type)
                    .await
            }
            HashMethod::Sha512 => {
                self.open::<Sha512Hasher>(root_hash, read_only, storage_type)
                    .await
            }
            HashMethod::Blake2s256 => {
                self.open::<Blake2s256Hasher>(root_hash, read_only, storage_type)
                    .await
            }
            HashMethod::Keccak256 => {
                self.open::<Keccak256Hasher>(root_hash, read_only, storage_type)
                    .await
            }
            HashMethod::QCID => {
                let msg = "QCID hash method is not supported for TrieObjectMap".to_string();
                error!("{}", msg);
                Err(NdnError::Unsupported(msg))
            }
        }
    }

    pub async fn open<H>(
        &self,
        root_hash: Option<&str>,
        read_only: bool,
        storage_type: Option<TrieObjectMapStorageType>,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>>
    where
        H: Hasher + Send + Sync + 'static,
        H::Out: HashFromSlice + AsRef<[u8]>,
    {
        if !self.data_dir.exists() {
            std::fs::create_dir_all(&self.data_dir).map_err(|e| {
                let msg = format!(
                    "Error creating directory {}: {}",
                    self.data_dir.display(),
                    e
                );
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;
        }

        let storage_type = storage_type.unwrap_or(self.default_storage_type);
        let root: Option<<H as Hasher>::Out> = if let Some(id) = root_hash {
            // Decode the root hash from string to the appropriate type.
            let root_hash = Base32Codec::from_base32(id)?;
            Some(H::Out::from_slice(root_hash.as_slice())?)
        } else {
            None
        };

        match storage_type {
            TrieObjectMapStorageType::Memory => {
                let msg = "Memory storage is not supported for open operation".to_string();
                error!("{}", msg);
                Err(NdnError::PermissionDenied(msg))
            }
            TrieObjectMapStorageType::SQLite => {
                let file = self.get_file_path_by_id(root_hash, storage_type);
                info!(
                    "Opening TrieObjectMap SQLite storage at: {}",
                    file.display()
                );

                let db = TrieObjectMapSqliteStorage::<H>::new(file, read_only)?;
                let storage = TrieObjectMapInnerStorageWrapper::<H>::new(
                    db.get_type(),
                    Box::new(db) as Box<dyn HashDBWithFile<H, Vec<u8>>>,
                    root,
                    read_only,
                );
                let ret = Box::new(storage) as Box<dyn TrieObjectMapInnerStorage>;

                Ok(ret)
            }
            TrieObjectMapStorageType::JSONFile => {
                let file = self.get_file_path_by_id(root_hash, storage_type);
                info!("Opening TrieObjectMap JSON storage at: {}", file.display());

                let db = TrieObjectMapJSONFileStorage::<H>::new(file, read_only)?;
                let storage = TrieObjectMapInnerStorageWrapper::<H>::new(
                    db.get_type(),
                    Box::new(db) as Box<dyn HashDBWithFile<H, Vec<u8>>>,
                    root,
                    read_only,
                );
                let ret = Box::new(storage) as Box<dyn TrieObjectMapInnerStorage>;

                Ok(ret)
            }
        }
    }

    pub async fn save(
        &self,
        root_hash: &str,
        storage: &mut dyn TrieObjectMapInnerStorage,
    ) -> NdnResult<()> {
        let file = self.get_file_path_by_id(Some(root_hash), storage.get_type());

        storage.save(&file).await
    }

    pub async fn clone(
        &self,
        root_hash: &str,
        storage: &dyn TrieObjectMapInnerStorage,
        read_only: bool,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>> {
        let file_name = if read_only {
            root_hash.to_string()
        } else {
            let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);
            format!(
                "clone_{}_{}_{}.{}",
                root_hash,
                index,
                chrono::Utc::now().timestamp(),
                Self::get_file_ext(storage.get_type()),
            )
        };

        let file = self.get_file_path(&file_name, storage.get_type());
        storage.clone(&file, read_only).await
    }

    fn get_temp_file_name(&self, storage_type: TrieObjectMapStorageType) -> String {
        // Use index and time tick to create a unique file name.
        let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);

        let ext = Self::get_file_ext(storage_type);
        format!("temp_{}_{}.{}", chrono::Utc::now().timestamp(), index, ext)
    }

    fn get_file_ext(storage_type: TrieObjectMapStorageType) -> &'static str {
        match storage_type {
            TrieObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file extension")
            }
            TrieObjectMapStorageType::SQLite => "sqlite",
            TrieObjectMapStorageType::JSONFile => "json",
        }
    }

    /*
    pub fn create_storage<H>(
        &self,
        storage_type: TrieObjectMapStorageType,
        read_only: bool,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>>
    where
        H: Hasher + Send + Sync + 'static,
        H::Out: HashFromSlice + std::borrow::Borrow<[u8]>,
    {
        let db = match storage_type {
            TrieObjectMapStorageType::Memory => {
                let db = TrieObjectMapMemoryStorage::<H>::default();
                Box::new(db) as Box<dyn HashDBWithFile<H, Vec<u8>>>
            }
            TrieObjectMapStorageType::SQLite => {
                // For SQLite storage, we can use a SQLite-based implementation
                let file = self.data_dir.join("trie_object_map.sqlite");
                Box::new(TrieObjectMapSqliteStorage::<H>::new(file, read_only)?)
                    as Box<dyn HashDBWithFile<H, Vec<u8>>>
            }
            TrieObjectMapStorageType::JSONFile => {
                // For JSON file storage, we can use a JSON-based implementation
                todo!("JSON file storage is not implemented yet");
            }
        };

        Ok(Box::new(TrieObjectMapInnerStorageWrapper::<H>::new(
            storage_type,
            db,
            read_only,
        )))
    }

    pub fn create_storage_by_hash_method(
        &self,
        storage_type: TrieObjectMapStorageType,
        read_only: bool,
        hash_method: HashMethod,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>> {
        match hash_method {
            HashMethod::Sha256 => self.create_storage::<Sha256Hasher>(storage_type, read_only),
            HashMethod::Sha512 => self.create_storage::<Sha512Hasher>(storage_type, read_only),
            HashMethod::Blake2s256 => {
                self.create_storage::<Blake2s256Hasher>(storage_type, read_only)
            }
            HashMethod::Keccak256 => {
                self.create_storage::<Keccak256Hasher>(storage_type, read_only)
            }
        }
    }
    */

    pub fn create_verifier<H>() -> Box<dyn TrieObjectMapProofVerifier>
    where
        H: Hasher + Send + Sync + 'static,
        H::Out: HashFromSlice + AsRef<[u8]>,
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
            HashMethod::QCID => {
                let msg = "QCID hash method is not supported for TrieObjectMap".to_string();
                error!("{}", msg);
                panic!("{}", NdnError::Unsupported(msg));
            }
        }
    }
}

pub static GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY: OnceCell<TrieObjectMapStorageFactory> =
    OnceCell::new();
