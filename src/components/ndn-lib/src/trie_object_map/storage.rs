use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use super::layout::GenericLayout;
use crate::{HashMethod, NdnError, NdnResult, ObjId};
use generic_array::{ArrayLength, GenericArray};
use hash_db::{HashDB, HashDBRef, Hasher};
use memory_db::{HashKey, MemoryDB};
use std::sync::{Arc, RwLock};
use trie_db::proof::generate_proof;
use trie_db::{NodeCodec, Trie, TrieLayout, TrieMut, Value};
use std::borrow::Borrow;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrieObjectMapStorageType {
    Memory,
    SQLite,
    JSONFile,
}

impl Default for TrieObjectMapStorageType {
    fn default() -> Self {
        Self::SQLite
    }
}

#[async_trait::async_trait]
pub trait HashDBWithFile<H: Hasher, T>: Send + Sync + HashDB<H, T> {
    fn get_type(&self) -> TrieObjectMapStorageType;

    // Clone the storage to a new file.
    // If the target file exists, it will be failed.
    async fn clone(&self, target: &Path, read_only: bool) -> NdnResult<Box<dyn HashDBWithFile<H, T>>>;

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, file: &Path) -> NdnResult<()>;
}

#[async_trait::async_trait]
pub trait TrieObjectMapInnerStorage: Send + Sync {
    fn is_readonly(&self) -> bool;
    fn get_type(&self) -> TrieObjectMapStorageType;

    async fn put(&self, key: &[u8], value: &[u8]) -> NdnResult<()>;
    async fn get(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>>;
    async fn remove(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>>;
    async fn is_exist(&self, key: &[u8]) -> NdnResult<bool>;
    async fn commit(&self) -> NdnResult<()>;
    async fn root(&self) -> Vec<u8>;
    async fn generate_proof(&self, key: &[u8]) -> NdnResult<Vec<Vec<u8>>>;

    // Clone the storage to a new file.
    // If the target file exists, it will be failed.
    async fn clone(&self, target: &Path, read_only: bool) -> NdnResult<Box<dyn TrieObjectMapInnerStorage>>;

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&self, file: &Path) -> NdnResult<()>;
}

pub type TrieObjectMapInnerStorageRef = Arc<Box<dyn TrieObjectMapInnerStorage>>;



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
