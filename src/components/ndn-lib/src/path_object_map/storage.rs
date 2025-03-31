use crate::{NdnError, NdnResult, ObjId};
use hash_db::{Hasher};
use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha512Hasher, Sha256Hasher};
use memory_db::{HashKey, MemoryDB};
use std::sync::{Arc, RwLock};
use reference_trie::{RefTrieDBMut, RefTrieDB, ReferenceNodeCodec};
use trie_db::proof::generate_proof;
use trie_db::{TrieMut, Trie, TrieLayout, NodeCodec, Value};

#[async_trait::async_trait]
pub trait DBStorage: Send + Sync {
    type Hasher: Hasher;

    async fn put(&self, key: &[u8], value: &[u8]) -> NdnResult<()>;
    async fn get(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>>;
    async fn remove(&self, key: &[u8]) -> NdnResult<Option<Vec<u8>>>;
    async fn commit(&self) -> NdnResult<()>;
    async fn root(&self) -> <Self::Hasher as Hasher>::Out;
    async fn generate_proof(&self, key: &[u8]) -> NdnResult<Vec<Vec<u8>>>;
}


type Sha256DB = MemoryDB<Sha256Hasher, HashKey<Sha256Hasher>, Vec<u8>>;
type Sha512DB = MemoryDB<Sha512Hasher, HashKey<Sha512Hasher>, Vec<u8>>;
type Blake2s256DB = MemoryDB<Blake2s256Hasher, HashKey<Blake2s256Hasher>, Vec<u8>>;
type Keccak256DB = MemoryDB<Keccak256Hasher, HashKey<Keccak256Hasher>, Vec<u8>>;


pub struct Sha256Layout;

impl TrieLayout for Sha256Layout {
    const USE_EXTENSION: bool = true;
    const ALLOW_EMPTY: bool = false;
    const MAX_INLINE_VALUE: Option<u32> = None;

    type Hash = Sha256Hasher; 
    type Codec = ReferenceNodeCodec<Sha256Hasher>;
}

type Sha256TrieDB<'a, 'cache> = trie_db::TrieDB<'a, 'cache, Sha256Layout>;
type Sha256TrieDBBuilder<'a, 'cache> = trie_db::TrieDBBuilder<'a, 'cache, Sha256Layout>;
type Sha256TrieDBMut<'a> = trie_db::TrieDBMut<'a, Sha256Layout>;
type Sha256TrieDBMutBuilder<'a> = trie_db::TrieDBMutBuilder<'a, Sha256Layout>;

pub struct Sha256MemoryStorage {
    db: Arc<RwLock<Sha256DB>>,
    root: Arc<RwLock<<Sha256Hasher as Hasher>::Out>>,
}

#[async_trait::async_trait ]
impl DBStorage for Sha256MemoryStorage {
    type Hasher = Sha256Hasher;

    async fn put(&self, key: &[u8], value: &[u8]) -> NdnResult<()> {
        let mut db_write = self.db.write().unwrap();
        let mut root = self.root.write().unwrap();
        let mut trie = Sha256TrieDBMutBuilder::new(&mut *db_write, &mut root).build();
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
        let trie = Sha256TrieDBBuilder::new(&*db_read, &root).build();
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
        let mut trie = Sha256TrieDBMutBuilder::new(&mut *db_write, &mut root).build();
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
            Value::Inline(v) | Value::NewNode(_,v) => {
                v.to_vec()
            }
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

        let mut trie = Sha256TrieDBMutBuilder::new(&mut *db_write, &mut root).build();
        trie.commit();

        // let new_root = trie.root_hash().to_vec();
        // info!("Root updated: {:?} -> {:?}", root, new_root);
        Ok(())
    }

    async fn root(&self) -> <Self::Hasher as Hasher>::Out {
        *self.root.read().unwrap()
    }

    async fn generate_proof(&self, key: &[u8]) -> NdnResult<Vec<Vec<u8>>> {
        let db_read = self.db.read().unwrap();
        let root = self.root.read().unwrap();
        // let trie = Sha256TrieDBBuilder::new(&*db_read, &*self.root.read().unwrap()).build();

        let proof = generate_proof::<_, Sha256Layout, _, &[u8]>(&*db_read, &root, &vec![key]).map_err(|e| {
            let msg = format!("Failed to generate proof for key: {:?}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;
       
        Ok(proof)
    }
}