use crate::hash::{HashHelper, HashMethod};
use hash_db::Hasher;
use generic_array::GenericArray;
use typenum::{U64, U32};

pub(super) struct Sha256Hasher;

impl Hasher for Sha256Hasher {
    type Out = GenericArray<u8, U32>;
    type StdHasher = std::collections::hash_map::DefaultHasher;
    const LENGTH: usize = 32;

    fn hash(data: &[u8]) -> Self::Out {
        let value = HashHelper::calc_hash(HashMethod::Sha256, data);
        GenericArray::clone_from_slice(&value)
    }
}

pub(super) struct Sha512Hasher;

impl Hasher for Sha512Hasher {
    type Out = GenericArray<u8, U64>;
    type StdHasher = std::collections::hash_map::DefaultHasher;
    const LENGTH: usize = 64;

    fn hash(data: &[u8]) -> Self::Out {
        let value = HashHelper::calc_hash(HashMethod::Sha512, data);
        GenericArray::clone_from_slice(&value)
    }
}

#[derive(Default)]
pub(super) struct Blake2s256Hasher;

impl Hasher for Blake2s256Hasher {
    type Out = GenericArray<u8, U32>;
    type StdHasher = std::collections::hash_map::DefaultHasher;
    const LENGTH: usize = 32;

    fn hash(data: &[u8]) -> Self::Out {
        let value = HashHelper::calc_hash(HashMethod::Blake2s256, data);
        GenericArray::clone_from_slice(&value)
    }
}

#[derive(Default)]
pub(super) struct Keccak256Hasher;

impl Hasher for Keccak256Hasher {
    type Out = GenericArray<u8, U32>;
    type StdHasher = std::collections::hash_map::DefaultHasher;
    const LENGTH: usize = 32;

    fn hash(data: &[u8]) -> Self::Out {
        let value = HashHelper::calc_hash(HashMethod::Keccak256, data);
        GenericArray::clone_from_slice(&value)
    }
}