use super::object_map::{
    TrieObjectMap, TrieObjectMapProofNodesCodec, TrieObjectMapProofVerifierHelper,
    TrieObjectMapProofVerifyResult,
};
use super::storage_factory::{TrieObjectMapStorageFactory, GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY};
use crate::hash::{HashHelper, HashMethod};
use crate::ObjId;
use crate::OBJ_TYPE_FILE;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::sync::Arc;
use tokio::test;

fn generate_random_buf(rng: &mut StdRng, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];

    rng.fill(&mut buf[..]);
    buf
}

// Generate a random path key of segments, like /a/bb/cc/dd
fn generate_random_path_key(rng: &mut StdRng) -> String {
    let mut path = String::new();
    let seg_size = rng.random_range(1..=10);
    for _ in 0..seg_size {
        let seg_len = rng.random_range(1..=10);
        let seg: String = (0..seg_len)
            .map(|_| rng.random_range(b'a'..=b'z') as char)
            .collect();
        path.push('/');
        path.push_str(&seg);
    }
    path
}

fn generate_key_value_pair(seed: &str, count: usize) -> (String, ObjId) {
    let seed = HashHelper::calc_hash(HashMethod::Sha256, seed.as_bytes());
    let mut rng: StdRng = SeedableRng::from_seed(seed.try_into().unwrap());

    let hash = generate_random_buf(&mut rng, HashMethod::Keccak256.hash_bytes());
    let obj_id = ObjId::new_by_raw(OBJ_TYPE_FILE.to_owned(), hash);

    (generate_random_path_key(&mut rng), obj_id)
}

fn generate_key_value_pairs(seed: &str, count: usize) -> Vec<(String, ObjId)> {
    let mut pairs = Vec::new();
    for i in 0..count {
        pairs.push(generate_key_value_pair(&format!("{}-{}", seed, i), count));
    }
    pairs
}

async fn test_op_and_proof(key_pairs: &[(String, ObjId)]) {
    let mut obj_map = TrieObjectMap::new(HashMethod::Keccak256, None)
        .await
        .unwrap();
    println!("Object map created");

    let count = 100;
    for i in 0..count {
        println!("Test object map: {}/{}", i, count);
        let key = key_pairs[i].0.as_ref();
        let obj_id = key_pairs[i].1.clone();

        obj_map.put_object(key, &obj_id).await.unwrap();

        println!("Put object success: {}", key);

        // Test get object
        let ret = obj_map.get_object(key).await.unwrap();
        if ret.is_none() {
            panic!("Object not found for key: {}, {}", i, key);
        }
        let ret = ret.unwrap();
        assert_eq!(ret, obj_id);

        println!("Get object success: {}", key);

        // Test exist
        let ret = obj_map.is_object_exist(&key).await.unwrap();
        assert_eq!(ret, true);

        println!("Object exist: {}", key);

        // Test proof path
        let proof = obj_map.get_object_proof_path(key).await.unwrap();
        assert!(proof.is_some());
        let mut proof = proof.unwrap();
        assert_eq!(proof.proof_nodes.len() > 0, true);

        assert_eq!(proof.root_hash.len() > 0, true);
        println!("Get object proof path success: {}", key);

        // println!("Proof nodes: {:?}", proof.proof_nodes);
        // println!("Root hash: {:?}", proof.root_hash);

        // Test proof path with invalid key
        let key1 = format!("{}/1000", key);
        let proof1 = obj_map.get_object_proof_path(&key1).await.unwrap();
        assert!(proof1.is_some());
        println!("Get object proof path with invalid key success: {}", key1);
        let proof1 = proof1.unwrap();

        // Test verification
        let verifier = TrieObjectMapProofVerifierHelper::new(obj_map.hash_method());

        // First test verify with right value
        let ret = verifier.verify_object(&key, &obj_id, &proof).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::Ok);

        // Test verification with invalid value
        let ret = verifier.verify_object(&key1, &obj_id, &proof1).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::ValueMismatch);

        // Test remove
        let prev_root_hash = obj_map.get_root_hash().await;
        assert!(proof.root_hash == prev_root_hash);
        if i % 2 == 0 {
            let ret = obj_map.remove_object(&key).await.unwrap().unwrap();
            assert_eq!(ret, obj_id);

            println!("Remove object success: {}", key);
        } else {
            continue;
        }

        // Test root hash after remove
        let root_hash = obj_map.get_root_hash().await;
        assert_ne!(prev_root_hash, root_hash);
        println!(
            "Root hash changed after remove: {:?} -> {:?}",
            prev_root_hash, root_hash
        );

        // Test verification after remove
        proof.root_hash = root_hash.clone();
        let ret = verifier.verify_object(&key, &obj_id, &proof).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::RootMismatch);
        println!("Verify after remove success: {}", key);

        // Test codec
        let s = TrieObjectMapProofNodesCodec::encode(&proof.proof_nodes).unwrap();
        println!("Proof nodes encoded: {}", s);
        let proof_nodes = TrieObjectMapProofNodesCodec::decode(&s).unwrap();
        assert_eq!(proof_nodes.len(), proof.proof_nodes.len());
    }
}


#[test]
async fn test_trie_object_map() {
    let temp_dir = std::env::temp_dir();
    let data_dir = temp_dir.join("ndn_test_trie_object_map");
    if !data_dir.exists() {
        std::fs::create_dir_all(&data_dir).expect("Failed to create test data directory");
    } else {
        // Clean up the directory if it already exists
        // std::fs::remove_dir_all(&data_dir).expect("Failed to remove existing test data directory");
        // std::fs::create_dir_all(&data_dir).expect("Failed to recreate test data directory");
    }

    GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY.set(TrieObjectMapStorageFactory::new(data_dir, None));

    println!("Test object map");
    let key_pairs = generate_key_value_pairs("test", 100);
    println!("Key pairs generated");

    test_op_and_proof(key_pairs.as_slice()).await;
}