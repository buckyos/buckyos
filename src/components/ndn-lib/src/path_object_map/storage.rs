use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use crate::{HashMethod, NdnError, NdnResult, ObjId};
use generic_array::{ArrayLength, GenericArray};
use hash_db::Hasher;
use memory_db::{HashKey, MemoryDB};
use reference_trie::{RefTrieDB, RefTrieDBMut, ReferenceNodeCodec};
use std::sync::{Arc, RwLock};
use trie_db::proof::generate_proof;
use trie_db::{NodeCodec, Trie, TrieLayout, TrieMut, Value};

#[async_trait::async_trait]
pub trait PathObjectMapInnerStorage: Send + Sync {
    async fn put(&self, key: &[u8], value: &[u8]) -> NdnResult<()>;
    async fn get(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>>;
    async fn remove(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>>;
    async fn commit(&self) -> NdnResult<()>;
    async fn root(&self) -> Vec<u8>;
    async fn generate_proof(&self, key: &[u8]) -> NdnResult<Vec<Vec<u8>>>;
}

pub type PathObjectMapInnerStorageRef = Arc<Box<dyn PathObjectMapInnerStorage>>;

type GenericMemoryDB<H> = MemoryDB<H, HashKey<H>, Vec<u8>>;
type Sha256DB = GenericMemoryDB<Sha256Hasher>;
type Sha512DB = GenericMemoryDB<Sha512Hasher>;
type Blake2s256DB = GenericMemoryDB<Blake2s256Hasher>;
type Keccak256DB = GenericMemoryDB<Keccak256Hasher>;

pub struct GenericLayout<H: Hasher>(std::marker::PhantomData<H>);

impl<H: Hasher> TrieLayout for GenericLayout<H> {
    const USE_EXTENSION: bool = true;
    const ALLOW_EMPTY: bool = false;
    const MAX_INLINE_VALUE: Option<u32> = None;

    type Hash = H;
    type Codec = ReferenceNodeCodec<H>;
}

type GenericTrieDB<'a, 'cache, H> = trie_db::TrieDB<'a, 'cache, GenericLayout<H>>;
type GenericTrieDBBuilder<'a, 'cache, H> = trie_db::TrieDBBuilder<'a, 'cache, GenericLayout<H>>;
type GenericTrieDBMut<'a, H> = trie_db::TrieDBMut<'a, GenericLayout<H>>;
type GenericTrieDBMutBuilder<'a, H> = trie_db::TrieDBMutBuilder<'a, GenericLayout<H>>;

pub struct GenericMemoryStorage<H: Hasher> {
    db: Arc<RwLock<GenericMemoryDB<H>>>,
    root: Arc<RwLock<<H as Hasher>::Out>>,
}

impl<H: Hasher> GenericMemoryStorage<H> {
    pub fn new() -> Self {
        let root = Default::default();
        let db = Arc::new(RwLock::new(GenericMemoryDB::<H>::default()));
        let root = Arc::new(RwLock::new(root));
        Self { db, root }
    }
}

#[async_trait::async_trait]
impl<H> PathObjectMapInnerStorage for GenericMemoryStorage<H>
where
    H: Hasher + Send + Sync + 'static,
    H::Out: Send + Sync + 'static,
{
    async fn put(&self, key: &[u8], value: &[u8]) -> NdnResult<()> {
        let mut db_write = self.db.write().unwrap();
        let mut root = self.root.write().unwrap();
        let mut trie = GenericTrieDBMutBuilder::new(&mut *db_write, &mut root).build();
        trie.insert(key, value).map_err(|e| {
            let msg = format!("Failed to insert key-value pair: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(())
    }

    async fn get(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>> {
        let db_read = self.db.read().unwrap();
        let root = self.root.read().unwrap();
        let trie = GenericTrieDBBuilder::new(&*db_read, &root).build();
        let value = trie.get(key).map_err(|e| {
            let msg = format!("Failed to get value for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(value)
    }

    async fn remove(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>> {
        let mut db_write = self.db.write().unwrap();
        let mut root = self.root.write().unwrap();
        let mut trie = GenericTrieDBMutBuilder::new(&mut *db_write, &mut root).build();
        let value = trie.remove(key).map_err(|e| {
            let msg = format!("Failed to get value for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        drop(trie); // Explicitly drop the trie to release the lock before using it again

        if value.is_none() {
            warn!("Remove key not found: {:?}", key);
            return Ok(None);
        }

        let value = value.unwrap();
        let value = match value {
            Value::Inline(v) | Value::NewNode(_, v) => v.to_vec(),
            Value::Node(hash) => {
                use hash_db::HashDBRef;

                let db = self.db.read().unwrap();
                let value = db.get(&hash, hash_db::EMPTY_PREFIX);
                if value.is_none() {
                    error!("Remove key but hash not found: {:?}, {:?}", key, hash);
                    return Ok(None);
                }

                let value = value.unwrap();
                value
            }
        };

        info!("Removed key: {:?}, value: {:?}", key, value);

        Ok(Some(value))
    }

    async fn commit(&self) -> NdnResult<()> {
        let mut db_write = self.db.write().unwrap();
        let mut root = self.root.write().unwrap();

        let mut trie = GenericTrieDBMutBuilder::new(&mut *db_write, &mut root).build();
        trie.commit();

        // let new_root = trie.root_hash().to_vec();
        // info!("Root updated: {:?} -> {:?}", root, new_root);
        Ok(())
    }

    async fn root(&self) -> Vec<u8> {
        let root = *self.root.read().unwrap();
        root.as_ref().to_vec()
    }

    async fn generate_proof(&self, key: &[u8]) -> NdnResult<Vec<Vec<u8>>> {
        let db_read = self.db.read().unwrap();
        let root = self.root.read().unwrap();
        // let trie = Sha256TrieDBBuilder::new(&*db_read, &*self.root.read().unwrap()).build();

        let proof = generate_proof::<_, GenericLayout<H>, _, &[u8]>(&*db_read, &root, &vec![key])
            .map_err(|e| {
            let msg = format!("Failed to generate proof for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(proof)
    }
}

pub type Sha256MemoryStorage = GenericMemoryStorage<Sha256Hasher>;
pub type Sha512MemoryStorage = GenericMemoryStorage<Sha512Hasher>;
pub type Blake2s256MemoryStorage = GenericMemoryStorage<Blake2s256Hasher>;
pub type Keccak256MemoryStorage = GenericMemoryStorage<Keccak256Hasher>;

pub trait PathObjectMapProofVerifier: Send + Sync {
    fn verify(
        &self,
        proof_nodes: &Vec<Vec<u8>>,
        root_hash: &[u8],
        key: &[u8],
        value: Option<&[u8]>,
    ) -> NdnResult<bool>;
}

pub type PathObjectMapProofVerifierRef = Arc<Box<dyn PathObjectMapProofVerifier>>;

pub trait HashFromSlice: Sized {
    fn from_slice(bytes: &[u8]) -> NdnResult<Self>;
}

impl<N> HashFromSlice for GenericArray<u8, N>
where
    N: ArrayLength<u8>,
{
    fn from_slice(data: &[u8]) -> NdnResult<Self> {
        if data.len() != N::to_usize() {
            let msg = format!(
                "Invalid length for GenericArray<u8, {}>: expected {}, got {}",
                std::any::type_name::<N>(),
                N::to_usize(),
                data.len()
            );
            error!("{}", msg);
            return Err(NdnError::InvalidData(msg));
        }

        let mut array = GenericArray::<u8, N>::default();
        array.clone_from_slice(data);
        Ok(array)
    }
}

pub struct GenericPathObjectMapProofVerifier<H: Hasher> {
    _marker: std::marker::PhantomData<H>,
}

impl<H> GenericPathObjectMapProofVerifier<H>
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

impl<H> PathObjectMapProofVerifier for GenericPathObjectMapProofVerifier<H>
where
    H: Hasher + Send + Sync + 'static,
    H::Out: HashFromSlice,
{
    fn verify(
        &self,
        proof_nodes: &Vec<Vec<u8>>,
        root_hash: &[u8],
        key: &[u8],
        value: Option<&[u8]>,
    ) -> NdnResult<bool> {
        use trie_db::proof::{verify_proof, VerifyError};

        let root_hash: H::Out = H::Out::from_slice(root_hash)?;

        let ret = verify_proof::<GenericLayout<H>, _, _, &[u8]>(
            &root_hash,
            proof_nodes,
            &vec![(key, value)], // The data to be verified, if the data is None, it means to check the existence of the key
        );

        match ret {
            Ok(_) => Ok(true),
            Err(e) => match &e {
                VerifyError::DecodeError(_) => {
                    let msg = format!("Error decoding proof: {:?}", e);
                    error!("{}", msg);
                    Err(NdnError::InvalidData(msg))
                }
                VerifyError::DuplicateKey(_key) => {
                    let msg = format!("Error verifying proof: {:?}", e);
                    error!("{}", msg);
                    Err(NdnError::InvalidData(msg))
                }
                _ => {
                    let msg = format!("Verification error: {:?}, {:?}", key, e);
                    info!("{}", msg);
                    Ok(false)
                }
            },
        }
    }
}

pub struct PathObjectMapInnerStorageFactory {}

impl PathObjectMapInnerStorageFactory {
    pub fn new() -> Self {
        Self {}
    }

    pub fn create_memory_storage<H: Hasher + Send + Sync + 'static>(
    ) -> Box<dyn PathObjectMapInnerStorage> {
        Box::new(GenericMemoryStorage::<H>::new())
    }

    pub fn create_memory_storage_by_hash_method(
        hash_method: HashMethod,
    ) -> Box<dyn PathObjectMapInnerStorage> {
        match hash_method {
            HashMethod::Sha256 => Self::create_memory_storage::<Sha256Hasher>(),
            HashMethod::Sha512 => Self::create_memory_storage::<Sha512Hasher>(),
            HashMethod::Blake2s256 => Self::create_memory_storage::<Blake2s256Hasher>(),
            HashMethod::Keccak256 => Self::create_memory_storage::<Keccak256Hasher>(),
        }
    }

    pub fn create_verifier<H>() -> Box<dyn PathObjectMapProofVerifier>
    where
        H: Hasher + Send + Sync + 'static,
        H::Out: HashFromSlice,
    {
        Box::new(GenericPathObjectMapProofVerifier::<H>::new())
    }

    pub fn create_verifier_by_hash_method(
        hash_method: HashMethod,
    ) -> Box<dyn PathObjectMapProofVerifier> {
        match hash_method {
            HashMethod::Sha256 => Self::create_verifier::<Sha256Hasher>(),
            HashMethod::Sha512 => Self::create_verifier::<Sha512Hasher>(),
            HashMethod::Blake2s256 => Self::create_verifier::<Blake2s256Hasher>(),
            HashMethod::Keccak256 => Self::create_verifier::<Keccak256Hasher>(),
        }
    }
}
