use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use super::layout::GenericLayout;
use super::storage::{HashDBWithFile, TrieObjectMapInnerStorage, TrieObjectMapStorageType, HashFromSlice};
use crate::{HashMethod, NdnError, NdnResult, ObjId};
use generic_array::{ArrayLength, GenericArray};
use hash_db::{HashDB, HashDBRef, Hasher};
use memory_db::{HashKey, MemoryDB};
use std::borrow::Borrow;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use trie_db::proof::generate_proof;
use trie_db::{NodeCodec, Trie, TrieLayout, TrieMut, Value};

type GenericTrieDB<'a, 'cache, H> = trie_db::TrieDB<'a, 'cache, GenericLayout<H>>;
type GenericTrieDBBuilder<'a, 'cache, H> = trie_db::TrieDBBuilder<'a, 'cache, GenericLayout<H>>;
type GenericTrieDBMut<'a, H> = trie_db::TrieDBMut<'a, GenericLayout<H>>;
type GenericTrieDBMutBuilder<'a, H> = trie_db::TrieDBMutBuilder<'a, GenericLayout<H>>;

pub struct TrieObjectMapInnerStorageWrapper<H: Hasher> {
    storage_type: TrieObjectMapStorageType,
    read_only: bool,
    db: Arc<RwLock<Box<dyn HashDBWithFile<H, Vec<u8>>>>>,
    root: Arc<RwLock<<H as Hasher>::Out>>,
}

impl<H: Hasher + 'static> TrieObjectMapInnerStorageWrapper<H> {
    pub fn new(
        storage_type: TrieObjectMapStorageType,
        db: Box<dyn HashDBWithFile<H, Vec<u8>>>,
        read_only: bool,
    ) -> Self
    where
        <H as hash_db::Hasher>::Out: AsRef<[u8]>,
    {
        let root = <GenericLayout<H> as TrieLayout>::Codec::hashed_null_node();
        let db = Arc::new(RwLock::new(db));
        let root = Arc::new(RwLock::new(root));
        Self {
            storage_type,
            db,
            root,
            read_only,
        }
    }

    fn check_read_only(&self) -> NdnResult<()> {
        if self.read_only {
            let msg = format!("Storage is read-only");
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl<H> TrieObjectMapInnerStorage for TrieObjectMapInnerStorageWrapper<H>
where
    H: Hasher + Send + Sync + 'static,
    H::Out: Send + Sync + 'static + AsRef<[u8]>,
{
    fn get_type(&self) -> TrieObjectMapStorageType {
        self.storage_type
    }

    fn is_readonly(&self) -> bool {
        self.read_only
    }

    async fn put(&self, key: &[u8], value: &[u8]) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        let mut db_write = self.db.write().await;
        let mut root = self.root.write().await;

        let mut trie =
            GenericTrieDBMutBuilder::from_existing(db_write.as_hash_db_mut(), &mut root).build();
        trie.insert(key, value).map_err(|e| {
            let msg = format!("Failed to insert key-value pair: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        // The trie will auto commit when it goes out of scope, but we can also call commit explicitly if needed.
        // trie.commit();

        Ok(())
    }

    async fn get(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>> {
        let db_read = self.db.read().await;
        let db = db_read.as_ref().as_hash_db();
        let root = self.root.read().await;

        let trie = GenericTrieDBBuilder::new(&db as &dyn HashDBRef<H, Vec<u8>>, &root).build();
        let value = trie.get(key).map_err(|e| {
            let msg = format!("Failed to get value for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(value)
    }

    async fn remove(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>> {
        // Check if the storage is read-only
        self.check_read_only()?;

        let mut db_write = self.db.write().await;
        let mut root = self.root.write().await;
        let mut trie =
            GenericTrieDBMutBuilder::from_existing(db_write.as_hash_db_mut(), &mut root).build();

        // First get value
        let value = trie.get(key).map_err(|e| {
            let msg = format!("Failed to get value for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        if value.is_none() {
            warn!("Remove key not found: {:?}", key);
            return Ok(None);
        }
        let value = value.unwrap();

        let remove_value = trie.remove(key).map_err(|e| {
            let msg = format!("Failed to get value for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        assert!(
            remove_value.is_some(),
            "Value should be present after remove"
        );

        info!("Removed key: {:?}, value: {:?}", key, value);

        // The trie will auto commit when it goes out of scope, but we can also call commit explicitly if needed.
        // trie.commit();

        Ok(Some(value))
    }

    async fn is_exist(&self, key: &[u8]) -> NdnResult<bool> {
        let db_read = self.db.read().await;
        let db = db_read.as_ref().as_hash_db();
        let root = self.root.read().await;

        let trie = GenericTrieDBBuilder::new(&db as &dyn HashDBRef<H, Vec<u8>>, &root).build();
        let exists = trie.contains(key).map_err(|e| {
            let msg = format!("Failed to check existence for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(exists)
    }

    async fn commit(&self) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        let mut db_write = self.db.write().await;
        let mut root = self.root.write().await;

        let mut trie =
            GenericTrieDBMutBuilder::from_existing(db_write.as_hash_db_mut(), &mut root).build();
        trie.commit();

        // let new_root = trie.root_hash().to_vec();
        // info!("Root updated: {:?} -> {:?}", root, new_root);
        Ok(())
    }

    async fn root(&self) -> Vec<u8> {
        let root = *self.root.read().await;
        root.as_ref().to_vec()
    }

    async fn generate_proof(&self, key: &[u8]) -> NdnResult<Vec<Vec<u8>>> {
        let db_read = self.db.read().await;
        let db = db_read.as_ref();
        let root = self.root.read().await;
        // let trie = Sha256TrieDBBuilder::new(&*db_read, &*self.root.read().unwrap()).build();

        let proof =
            generate_proof::<_, GenericLayout<H>, _, &[u8]>(&db.as_hash_db(), &root, &vec![key])
                .map_err(|e| {
                    let msg = format!("Failed to generate proof for key: {:?}", e);
                    error!("{}", msg);
                    NdnError::DbError(msg)
                })?;

        Ok(proof)
    }

    async fn clone(
        &self,
        target: &Path,
        read_only: bool,
    ) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>> {
        let db_read = self.db.read().await;
        let db = db_read.as_ref();

        // Clone the database
        let cloned_db = db.clone(target, read_only).await?;

        // Create a new storage wrapper with the cloned database
        let new_storage =
            TrieObjectMapInnerStorageWrapper::<H>::new(self.storage_type, cloned_db, read_only);

        Ok(Box::new(new_storage))
    }

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&self, file: &Path) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        let mut db_write = self.db.write().await;
        let db = db_write.as_mut();

        // Save the database to the file
        db.save(file).await.map_err(|e| {
            let msg = format!("Failed to save database to file: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum TrieObjectMapProofVerifyResult {
    Ok,
    ExtraneousNode,
    ExtraneousValue,
    ExtraneousHashReference,
    InvalidChildReference,
    ValueMismatch,
    IncompleteProof,
    RootMismatch,
    Other,
}

pub trait TrieObjectMapProofVerifier: Send + Sync {
    fn verify(
        &self,
        proof_nodes: &Vec<Vec<u8>>,
        root_hash: &[u8],
        key: &[u8],
        value: &[u8],
    ) -> NdnResult<TrieObjectMapProofVerifyResult>;
}

pub type TrieObjectMapProofVerifierRef = Arc<Box<dyn TrieObjectMapProofVerifier>>;

pub struct GenericTrieObjectMapProofVerifier<H: Hasher> {
    _marker: std::marker::PhantomData<H>,
}

impl<H> GenericTrieObjectMapProofVerifier<H>
where
    H: Hasher + Send + Sync + 'static,
    H::Out: HashFromSlice,
{
    pub fn new() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

impl<H> TrieObjectMapProofVerifier for GenericTrieObjectMapProofVerifier<H>
where
    H: Hasher + Send + Sync + 'static,
    H::Out: HashFromSlice,
{
    fn verify(
        &self,
        proof_nodes: &Vec<Vec<u8>>,
        root_hash: &[u8],
        key: &[u8],
        value: &[u8],
    ) -> NdnResult<TrieObjectMapProofVerifyResult> {
        use trie_db::proof::{verify_proof, VerifyError};

        let root_hash: H::Out = H::Out::from_slice(root_hash)?;

        let ret = verify_proof::<GenericLayout<H>, _, _, &[u8]>(
            &root_hash,
            proof_nodes,
            &vec![(key, Some(value))], // The data to be verified, if the data is None, it means to check the existence of the key
        );

        // println!("Verify proof: key = {:?}, root = {:?}, ret = {:?}", key, root_hash, ret);
        let ret = match ret {
            Ok(_) => TrieObjectMapProofVerifyResult::Ok,
            Err(e) => match &e {
                VerifyError::ExtraneousNode => TrieObjectMapProofVerifyResult::ExtraneousNode,
                VerifyError::ExtraneousValue(_) => TrieObjectMapProofVerifyResult::ExtraneousValue,
                VerifyError::ExtraneousHashReference(_) => {
                    TrieObjectMapProofVerifyResult::ExtraneousHashReference
                }
                VerifyError::ValueMismatch(_) => TrieObjectMapProofVerifyResult::ValueMismatch,
                VerifyError::RootMismatch(_) => TrieObjectMapProofVerifyResult::RootMismatch,
                VerifyError::InvalidChildReference(_) => {
                    TrieObjectMapProofVerifyResult::InvalidChildReference
                }
                VerifyError::IncompleteProof => TrieObjectMapProofVerifyResult::IncompleteProof,
                _ => {
                    let msg = format!("Verification error: {:?}, {:?}", key, e);
                    info!("{}", msg);
                    TrieObjectMapProofVerifyResult::Other
                }
            },
        };

        Ok(ret)
    }
}
