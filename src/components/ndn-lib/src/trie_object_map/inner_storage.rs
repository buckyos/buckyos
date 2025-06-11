use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use super::layout::GenericLayout;
use super::storage::{
    HashDBWithFile, HashFromSlice, TrieObjectMapInnerStorage, TrieObjectMapStorageType,
};
use crate::{HashMethod, NdnError, NdnResult, ObjId};
use generic_array::{ArrayLength, GenericArray};
use hash_db::{HashDB, HashDBRef, Hasher};
use memory_db::{HashKey, MemoryDB};
use ouroboros::self_referencing;
use serde::de::value;
use std::borrow::Borrow;
use std::path::Path;
use std::sync::Arc;
use trie_db::proof::generate_proof;
use trie_db::{NodeCodec, Trie, TrieDB, TrieLayout, TrieMut, Value};

type GenericTrieDB<'a, 'cache, H> = trie_db::TrieDB<'a, 'cache, GenericLayout<H>>;
type GenericTrieDBBuilder<'a, 'cache, H> = trie_db::TrieDBBuilder<'a, 'cache, GenericLayout<H>>;
type GenericTrieDBMut<'a, H> = trie_db::TrieDBMut<'a, GenericLayout<H>>;
type GenericTrieDBMutBuilder<'a, H> = trie_db::TrieDBMutBuilder<'a, GenericLayout<H>>;

#[self_referencing]
pub struct TrieObjectMapInnerStorageWrapperIterator<'a, H: Hasher> {
    root: &'a <H as Hasher>::Out,
    trie_db: TrieDB<'a, 'a, GenericLayout<H>>,

    #[borrows(trie_db)]
    #[covariant]
    iter: Box<dyn Iterator<Item = (String, ObjId)> + 'this>,
}

impl<'a, H: Hasher> TrieObjectMapInnerStorageWrapperIterator<'a, H> {
    pub fn create(
        db: &'a Box<dyn HashDBWithFile<H, Vec<u8>>>,
        root: &'a <H as Hasher>::Out,
    ) -> NdnResult<Self> {
        let trie_db = GenericTrieDBBuilder::new(&*db as &dyn HashDBRef<H, Vec<u8>>, root).build();

        let mut ret = Self::try_new(root, trie_db, |trie_db| {
            // Create an iterator over the trie database
            let iter = trie_db.iter().map_err(|e| {
                let msg = format!("Failed to create iterator: {:?}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

            let iter = iter.filter_map(|item| {
                let (k, v) = item
                    .map_err(|e| {
                        let msg = format!("Failed to iterate over trie: {:?}", e);
                        error!("{}", msg);
                        NdnError::DbError(msg)
                    })
                    .ok()?;

                let key = String::from_utf8(k.to_vec())
                    .map_err(|e| {
                        let msg = format!("Failed to convert key to string: {:?}, {:?}", k, e);
                        error!("{}", msg);
                        NdnError::InvalidData(msg)
                    })
                    .ok()?;

                let value: ObjId = bincode::deserialize(&v)
                    .map_err(|e| {
                        let msg = format!("Failed to deserialize obj_id value: {:?}, {:?}", v, e);
                        error!("{}", msg);
                        NdnError::InvalidData(msg)
                    })
                    .ok()?;

                Some((key, value))
            });

            Ok(Box::new(iter))
        })?;

        Ok(ret)
    }
}

impl<'a, H: Hasher> Iterator for TrieObjectMapInnerStorageWrapperIterator<'a, H> {
    type Item = (String, ObjId);

    fn next(&mut self) -> Option<Self::Item> {
        self.with_iter_mut(|iter| iter.next())
    }
}

pub struct TrieObjectMapInnerStorageWrapper<H: Hasher> {
    storage_type: TrieObjectMapStorageType,
    read_only: bool,
    db: Box<dyn HashDBWithFile<H, Vec<u8>>>,
    root: <H as Hasher>::Out,
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

    async fn put(&mut self, key: &str, value: &ObjId) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        let value = bincode::serialize(value).map_err(|e| {
            let msg = format!("Failed to serialize obj_id value: {}, {:?}", value, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        let mut trie =
            GenericTrieDBMutBuilder::from_existing(self.db.as_hash_db_mut(), &mut self.root)
                .build();
        trie.insert(key.as_bytes(), &value).map_err(|e| {
            let msg = format!("Failed to insert key-value pair: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        // The trie will auto commit when it goes out of scope, but we can also call commit explicitly if needed.
        // trie.commit();

        Ok(())
    }

    async fn get(&self, key: &str) -> NdnResult<Option<ObjId>> {
        let db = &self.db.as_hash_db() as &dyn HashDBRef<H, Vec<u8>>;
        let trie = GenericTrieDBBuilder::new(db, &self.root).build();
        let value = trie.get(key.as_bytes()).map_err(|e| {
            let msg = format!("Failed to get value for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        if value.is_none() {
            return Ok(None);
        }

        let value = value.unwrap();
        let value: ObjId = bincode::deserialize(&value).map_err(|e| {
            let msg = format!("Failed to deserialize obj_id value: {:?}, {:?}", value, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(Some(value))
    }

    async fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        // Check if the storage is read-only
        self.check_read_only()?;

        let mut trie =
            GenericTrieDBMutBuilder::from_existing(self.db.as_hash_db_mut(), &mut self.root)
                .build();

        // First get value
        let value = trie.get(key.as_bytes()).map_err(|e| {
            let msg = format!("Failed to get value for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        if value.is_none() {
            warn!("Remove key not found: {:?}", key);
            return Ok(None);
        }
        let value = value.unwrap();

        let remove_value = trie.remove(key.as_bytes()).map_err(|e| {
            let msg = format!("Failed to get value for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        assert!(
            remove_value.is_some(),
            "Value should be present after remove"
        );

        info!("Removed key: {}, value: {:?}", key, value);

        // The trie will auto commit when it goes out of scope, but we can also call commit explicitly if needed.
        // trie.commit();

        let value: ObjId = bincode::deserialize(&value).map_err(|e| {
            let msg = format!("Failed to deserialize obj_id value: {:?}, {:?}", value, e);
            error!("{}", msg);
            NdnError::InvalidData(msg)
        })?;

        Ok(Some(value))
    }

    async fn is_exist(&self, key: &str) -> NdnResult<bool> {
        let db = &self.db.as_hash_db() as &dyn HashDBRef<H, Vec<u8>>;
        let trie = GenericTrieDBBuilder::new(db, &self.root).build();
        let exists = trie.contains(key.as_bytes()).map_err(|e| {
            let msg = format!("Failed to check existence for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(exists)
    }

    async fn commit(&mut self) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        let mut trie =
            GenericTrieDBMutBuilder::from_existing(self.db.as_hash_db_mut(), &mut self.root)
                .build();
        trie.commit();

        // let new_root = trie.root_hash().to_vec();
        // info!("Root updated: {:?} -> {:?}", root, new_root);
        Ok(())
    }

    async fn root(&self) -> Vec<u8> {
        self.root.as_ref().to_vec()
    }

    fn iter<'a>(&'a self) -> NdnResult<Box<dyn Iterator<Item = (String, ObjId)> + 'a>> {
        let mut iter = TrieObjectMapInnerStorageWrapperIterator::create(&self.db, &self.root)?;

        Ok(Box::new(iter))
    }

    async fn generate_proof(&self, key: &str) -> NdnResult<Vec<Vec<u8>>> {
        // let trie = Sha256TrieDBBuilder::new(&*db_read, &*self.root.read().unwrap()).build();

        let proof = generate_proof::<_, GenericLayout<H>, _, &[u8]>(
            &self.db.as_hash_db(),
            &self.root,
            &vec![key.as_bytes()],
        )
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
        // Clone the database
        let cloned_db = self.db.clone(target, read_only).await?;

        // Create a new storage wrapper with the cloned database
        let new_storage =
            TrieObjectMapInnerStorageWrapper::<H>::new(self.storage_type, cloned_db, read_only);

        Ok(Box::new(new_storage))
    }

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, file: &Path) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        // Save the database to the file
        self.db.save(file).await.map_err(|e| {
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
