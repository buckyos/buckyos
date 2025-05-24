use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use super::storage::HashDBWithFile;
use hash_db::Hasher as KeyHasher;
use memory_db::{HashKey, KeyFunction, MemoryDB};

pub type TrieObjectMapMemoryStorage<H> = MemoryDB<H, HashKey<H>, Vec<u8>>;
pub type TrieObjectMapMemorySha256Storage = TrieObjectMapMemoryStorage<Sha256Hasher>;
pub type TrieObjectMapMemorySha512Storage = TrieObjectMapMemoryStorage<Sha512Hasher>;
pub type TrieObjectMapMemoryBlake2s256Storage = TrieObjectMapMemoryStorage<Blake2s256Hasher>;
pub type TrieObjectMapMemoryKeccak256Storage = TrieObjectMapMemoryStorage<Keccak256Hasher>;

#[async_trait::async_trait]
impl<H, KF, T> HashDBWithFile<H, T> for MemoryDB<H, KF, T>
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
    KF::Key: std::borrow::Borrow<[u8]>,
{
    fn get_type(&self) -> super::storage::TrieObjectMapStorageType {
        super::storage::TrieObjectMapStorageType::Memory
    }

    async fn clone(
        &self,
        _target: &std::path::Path,
        _read_only: bool,
    ) -> crate::NdnResult<Box<dyn HashDBWithFile<H, T>>> {
        let ret = Clone::clone(self);
        Ok(Box::new(ret))
    }

    async fn save(&mut self, _file: &std::path::Path) -> crate::NdnResult<()> {
        // Memory storage does not support saving to file, so we just return Ok.
        Ok(())
    }
}
