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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrieObjectMapStorageOpenMode {
    CreateNew,
    OpenExisting,
}

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
        obj_id: Option<&ObjId>,
        storage_type: TrieObjectMapStorageType,
    ) -> PathBuf {
        let file_name = match storage_type {
            TrieObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file path");
            }
            TrieObjectMapStorageType::SQLite => {
                if let Some(id) = obj_id {
                    id.to_base32()
                } else {
                    self.get_temp_file_name()
                }
            }
            TrieObjectMapStorageType::JSONFile => {
                if let Some(id) = obj_id {
                    id.to_base32()
                } else {
                    self.get_temp_file_name()
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
        obj_info: Option<(&ObjId, &str)>,
        read_only: bool,
        storage_type: Option<TrieObjectMapStorageType>,
        hash_method: HashMethod,
        mode: TrieObjectMapStorageOpenMode,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>> {
        match hash_method {
            HashMethod::Sha256 => {
                self.open::<Sha256Hasher>(obj_info, read_only, storage_type, mode)
                    .await
            }
            HashMethod::Sha512 => {
                self.open::<Sha512Hasher>(obj_info, read_only, storage_type, mode)
                    .await
            }
            HashMethod::Blake2s256 => {
                self.open::<Blake2s256Hasher>(obj_info, read_only, storage_type, mode)
                    .await
            }
            HashMethod::Keccak256 => {
                self.open::<Keccak256Hasher>(obj_info, read_only, storage_type, mode)
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
        obj_info: Option<(&ObjId, &str)>,
        read_only: bool,
        storage_type: Option<TrieObjectMapStorageType>,
        mode: TrieObjectMapStorageOpenMode,
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
        if storage_type == TrieObjectMapStorageType::Memory {
            let msg = "Memory storage is not supported for open operation".to_string();
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }

        let (obj_id, root) = if let Some((id, root_hash)) = obj_info {
            // Decode the root hash from string to the appropriate type.
            let root_hash = Base32Codec::from_base32(root_hash)?;
            let root_hash: <H as Hasher>::Out =
                <H::Out as HashFromSlice>::from_slice(root_hash.as_slice())?;
            (Some(id), Some(root_hash))
        } else {
            (None, None)
        };

        let file = self.get_file_path_by_id(obj_id, storage_type);
        match mode {
            TrieObjectMapStorageOpenMode::CreateNew => {
                if file.exists() {
                    warn!(
                        "File {} already exists, removing it before creating new storage",
                        file.display()
                    );

                    tokio::fs::remove_file(&file).await.map_err(|e| {
                        let msg = format!("Error removing existing file {}: {}", file.display(), e);
                        error!("{}", msg);
                        NdnError::IoError(msg)
                    })?;
                }

                info!("Creating new TrieObjectMap storage at: {}", file.display());
            }
            TrieObjectMapStorageOpenMode::OpenExisting => {
                if !file.exists() {
                    let msg = format!(
                        "File {} does not exist, cannot open existing storage",
                        file.display()
                    );
                    error!("{}", msg);
                    return Err(NdnError::IoError(msg));
                }

                info!(
                    "Opening existing TrieObjectMap storage at: {}",
                    file.display()
                );
            }
        }

        match storage_type {
            TrieObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file path");
            }
            TrieObjectMapStorageType::SQLite => {
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
        obj_id: &ObjId,
        storage: &mut dyn TrieObjectMapInnerStorage,
    ) -> NdnResult<()> {
        let file = self.get_file_path_by_id(Some(obj_id), storage.get_type());

        storage.save(&file).await
    }

    pub async fn clone(
        &self,
        obj_id: &ObjId,
        storage: &dyn TrieObjectMapInnerStorage,
        read_only: bool,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>> {
        let file_name = if read_only {
            obj_id.to_base32()
        } else {
            let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);
            format!(
                "clone_{}_{}_{}.{}",
                obj_id.to_base32(),
                index,
                chrono::Utc::now().timestamp(),
                Self::get_file_ext(storage.get_type()),
            )
        };

        let file = self.get_file_path(&file_name, storage.get_type());
        storage.clone(&file, read_only).await
    }

    pub async fn switch_storage_type(
        &self,
        obj_info: (&ObjId, &str),
        storage: Box<dyn TrieObjectMapInnerStorage>,
        hash_method: HashMethod,
        new_storage_type: TrieObjectMapStorageType,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>>
    {
        let old_storage_type = storage.get_type();
        assert_ne!(
            old_storage_type, new_storage_type,
            "Cannot switch storage type with the same type",
        );

        // First create a new storage of the desired type.
        let mut new_storage = self
            .open_by_hash_method(
                None,
                false,
                Some(new_storage_type),
                hash_method,
                TrieObjectMapStorageOpenMode::CreateNew,
            )
            .await?;

        storage.traverse(&mut |key, obj_id| {
            new_storage.put(&key, &obj_id)?;
            Ok(())
        })?;

        self.save(&obj_info.0, new_storage.as_mut()).await?;

        // Drop the old storage and try to remove the file
        drop(storage);

        let old_file = self.get_file_path_by_id(Some(&obj_info.0), old_storage_type);
        if old_file.exists() {
            info!(
                "Removing old storage file: {}",
                old_file.display()
            );
            let ret = tokio::fs::remove_file(&old_file).await;
            if let Err(e) = ret {
                let msg = format!(
                    "Error removing old storage file {}: {}",
                    old_file.display(),
                    e
                );
                warn!("{}", msg);
                // FIXME: Should we return an error here? or we can remove the file later in GC?
            }
        }

        info!(
            "Switched trie object map storage for {} from {:?} to {:?}",
            obj_info.0.to_base32(),
            old_storage_type,
            new_storage_type,
        );

        Ok(new_storage)
    }

    fn get_temp_file_name(&self) -> String {
        // Use index and time tick to create a unique file name.
        let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);

        format!("temp_{}_{}", chrono::Utc::now().timestamp(), index)
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
