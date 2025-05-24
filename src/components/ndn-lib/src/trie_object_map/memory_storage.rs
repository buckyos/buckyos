use super::hash::{Blake2s256Hasher, Keccak256Hasher, Sha256Hasher, Sha512Hasher};
use memory_db::{HashKey, MemoryDB};

pub type TrieObjectMapMemoryStorage<H> = MemoryDB<H, HashKey<H>, Vec<u8>>;
pub type TrieObjectMapMemorySha256Storage = TrieObjectMapMemoryStorage<Sha256Hasher>;
pub type TrieObjectMapMemorySha512Storage = TrieObjectMapMemoryStorage<Sha512Hasher>;
pub type TrieObjectMapMemoryBlake2s256Storage = TrieObjectMapMemoryStorage<Blake2s256Hasher>;
pub type TrieObjectMapMemoryKeccak256Storage = TrieObjectMapMemoryStorage<Keccak256Hasher>;
