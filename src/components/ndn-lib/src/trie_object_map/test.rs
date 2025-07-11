use super::builder::TrieObjectMapBuilder;
use super::object_map::{
    TrieObjectMap, TrieObjectMapProofNodesCodec, TrieObjectMapProofVerifierHelper,
    TrieObjectMapProofVerifyResult,
};
use super::storage_factory::{TrieObjectMapStorageFactory, GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY};
use crate::hash::{HashHelper, HashMethod};
use crate::trie_object_map::file;
use crate::ObjId;
use crate::TrieObjectMapStorageType;
use crate::OBJ_TYPE_FILE;
use buckyos_kit::init_logging;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use trie_db::proof;
use std::clone;
use std::hash::Hash;
use std::sync::Arc;
use tokio::test;
use std::collections::HashMap;

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

fn generate_key_value_pair(seed: &str) -> (String, ObjId) {
    let seed = HashHelper::calc_hash(HashMethod::Sha256, seed.as_bytes());
    let mut rng: StdRng = SeedableRng::from_seed(seed.try_into().unwrap());

    let hash = generate_random_buf(&mut rng, HashMethod::Keccak256.hash_result_size());
    let obj_id = ObjId::new_by_raw(OBJ_TYPE_FILE.to_owned(), hash);

    (generate_random_path_key(&mut rng), obj_id)
}

fn generate_key_value_pairs(seed: &str, count: usize) -> Vec<(String, ObjId)> {
    let mut pairs = Vec::new();
    for i in 0..count {
        println!("Generating key-value pair: {}/{}", i, count);
        pairs.push(generate_key_value_pair(&format!("{}-{}", seed, i)));
    }
    pairs
}

async fn test_op_and_proof(key_pairs: &[(String, ObjId)]) {
    let mut obj_map_builder = TrieObjectMapBuilder::new(HashMethod::Keccak256, None)
        .await
        .unwrap();

    let storage_type = obj_map_builder.storage_type();
    assert_eq!(
        storage_type,
        TrieObjectMapStorageType::SQLite,
        "Default storage type should be SQLite"
    );

    let count = key_pairs.len();
    for i in 0..count {
        let key = key_pairs[i].0.as_ref();
        let obj_id = key_pairs[i].1.clone();

        println!("Test object map: {}/{} {}", i, count, key);
        
        let ret = obj_map_builder.put_object(key, &obj_id).unwrap();
        assert!(
            ret.is_none(),
            "Object should not exist before put for key: {}",
            key
        );

        // println!("Put object success: {}", key);

        // Test get object
        let ret = obj_map_builder.get_object(key).unwrap();
        if ret.is_none() {
            panic!("Object not found for key: {}, {}", i, key);
        }
        let ret = ret.unwrap();
        assert_eq!(ret, obj_id);

        // println!("Get object success: {}", key);

        // Test exist
        let ret = obj_map_builder.is_object_exist(&key).unwrap();
        assert_eq!(ret, true);

        // Remove object
        let ret = obj_map_builder.remove_object(&key).unwrap();
        assert!(ret.is_some(), "Object should be removed for key: {}", key);
        assert_eq!(ret.unwrap(), obj_id, "Object ID mismatch for key: {}", key);

        // Test exist after remove
        let ret = obj_map_builder.is_object_exist(&key).unwrap();
        assert_eq!(
            ret, false,
            "Object should not exist after remove for key: {}",
            key
        );

        // Test get object after remove
        let ret = obj_map_builder.get_object(&key).unwrap();
        assert!(
            ret.is_none(),
            "Object should not be found after remove for key: {}",
            key
        );

        // Reinsert object
        let ret = obj_map_builder.put_object(&key, &obj_id).unwrap();
        assert!(
            ret.is_none(),
            "Object should not exist after remove for key: {}",
            key
        );

        let ret = obj_map_builder.put_object(key, &obj_id).unwrap();
        assert!(
            ret.is_some(),
            "Prev object should exist after reinsert for key: {}",
            key
        );
        assert_eq!(
            ret.unwrap(),
            obj_id,
            "Object ID mismatch after reinsert for key: {}",
            key
        );
    }

    let obj_map = obj_map_builder.build().await.unwrap();
    println!("Object map built successfully");

    let len = obj_map.len();
    assert_eq!(len, count as u64, "Object map length mismatch");

    // Check the storage type
    let storage_type = obj_map.storage_type();
    assert_eq!(
        storage_type,
        TrieObjectMapStorageType::JSONFile,
        "Storage type should be JSONFile, not {:?}",
        storage_type
    );

    let mut proofs = HashMap::new();
    for (i, (key, obj_id)) in obj_map.iter().unwrap().enumerate() {
        // Test proof path
        let proof = obj_map.get_object_proof_path(&key).unwrap();
        assert!(proof.is_some());
        let mut proof = proof.unwrap();
        assert_eq!(proof.proof_nodes.len() > 0, true);

        proofs.insert(key.clone(), proof.clone());

        let root_hash = obj_map.get_root_hash();
        assert_eq!(proof.root_hash, root_hash, "Root hash mismatch for key: {}", key);

        assert_eq!(proof.root_hash.len() > 0, true);
        println!("Get object proof path success: {}", key);

        // println!("Proof nodes: {:?}", proof.proof_nodes);
        // println!("Root hash: {:?}", proof.root_hash);

        // Test proof path with invalid key
        let key1 = format!("{}/1000", key);
        let key2 = format!("{}/1001", key);
        let key3 = "error_key";
        let proof1 = obj_map.get_object_proof_path(&key1).unwrap();
        assert!(proof1.is_some());
        println!("Get object proof path with invalid key success: {}", key1);
        let proof1 = proof1.unwrap();

        // Test verification
        let verifier = TrieObjectMapProofVerifierHelper::new(obj_map.hash_method());

        // First test verify with right value
        let ret = verifier.verify_object(&key, Some(&obj_id), &proof).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::Ok);

        let ret = verifier.verify_object(&key, None, &proof).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::ValueMismatch);

        // Test verification with invalid value
        let ret = verifier
            .verify_object(&key1, Some(&obj_id), &proof1)
            .unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::ValueMismatch);

        let ret = verifier.verify_object(&key1, None, &proof1).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::Ok);

        // Because key2 starts with key1, it should be verified as Ok (not exist)
        let ret = verifier.verify_object(&key2, None, &proof1).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::Ok);

        let ret = verifier.verify_object(&key, None, &proof1).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::ExtraneousValue);
    }

    let prev_root_hash = obj_map.get_root_hash();

    let mut removed_keys = HashMap::new();
    let mut obj_map_builder = TrieObjectMapBuilder::from_trie_object_map(&obj_map).await.unwrap();
    for (i, (key, obj_id)) in obj_map.iter().unwrap().enumerate() {
        if i % 2 == 0 {
            let ret = obj_map_builder.remove_object(&key).unwrap().unwrap();
            assert_eq!(ret, obj_id);
            removed_keys.insert(key.clone(), obj_id);

            println!("Remove object success: {}", key);
        } else {
            continue;
        }
    }

    let obj_map2 = obj_map_builder.build().await.unwrap();
    let root_hash = obj_map2.get_root_hash();
    assert_ne!(prev_root_hash, root_hash);  

    let verifier = TrieObjectMapProofVerifierHelper::new(obj_map.hash_method());
    for (key, mut proof)  in proofs.into_iter() {
        let obj_id = obj_map.get_object(&key).unwrap().unwrap();

        if removed_keys.contains_key(&key) {
            assert!(obj_map2.get_object(&key).unwrap().is_none(), "Object should be removed for key: {}", key);
        } else {
            assert_eq!(obj_map2.get_object(&key).unwrap().unwrap(), obj_id, "Object ID mismatch for key: {}", key);
        }
        
        // Test verification after remove
        proof.root_hash = root_hash.clone();
        let ret = verifier.verify_object(&key, Some(&obj_id), &proof).unwrap();
        assert_eq!(ret, TrieObjectMapProofVerifyResult::RootMismatch);
        println!("Verify after remove success: {}", key);

        // Test codec
        let s = TrieObjectMapProofNodesCodec::encode(&proof.proof_nodes).unwrap();
        println!("Proof nodes encoded: {}", s);
        let proof_nodes = TrieObjectMapProofNodesCodec::decode(&s).unwrap();
    }
}

async fn test_iterator(key_pairs: &[(String, ObjId)]) {
    let mut obj_map_builder = TrieObjectMapBuilder::new(
        HashMethod::Keccak256,
        Some(crate::CollectionStorageMode::Simple),
    )
    .await
    .unwrap();
    println!("Object map created");

    for (key, obj_id) in key_pairs.iter() {
        obj_map_builder.put_object(key, obj_id).unwrap();
    }
    println!("All objects put");

    let obj_map = obj_map_builder.build().await.unwrap();

    let mut iter = obj_map.iter().unwrap();
    let mut count = 0;
    while let Some((key, obj_id)) = iter.next() {
        // Verify the key and object ID in the key_pairs
        let ret = key_pairs.contains(&(key.clone(), obj_id.clone()));
        assert!(ret, "Key not found in key_pairs: {}", key);

        // Check if object in object map
        let ret = obj_map.get_object(&key).unwrap();
        assert!(ret.is_some(), "Object not found for key: {}", key);
        assert_eq!(ret.unwrap(), obj_id, "Object ID mismatch for key: {}", key);

        println!("Iterated key: {}, obj_id: {}", key, obj_id);
        count += 1;
    }
    assert_eq!(count, key_pairs.len());
}

async fn test_traverse(key_pairs: &[(String, ObjId)]) {
    let mut obj_map_builder = TrieObjectMapBuilder::new(HashMethod::Keccak256, None)
        .await
        .unwrap();
    println!("Object map created");

    for (key, obj_id) in key_pairs.iter() {
        obj_map_builder.put_object(key, obj_id).unwrap();
    }
    println!("All objects put");

    let obj_map = obj_map_builder.build().await.unwrap();
    let mut count = 0;
    obj_map.traverse(&mut |key: String, obj_id: ObjId| {
        // Verify the key and object ID in the key_pairs
        let ret = key_pairs.contains(&(key.clone(), obj_id.clone()));
        assert!(ret, "Key not found in key_pairs: {}", key);

        println!("Traversed key: {}, obj_id: {}", key, obj_id);
        count += 1;

        Ok(())
    });

    assert_eq!(count, key_pairs.len());
    assert_eq!(
        count,
        obj_map.len() as usize,
        "Traverse count mismatch"
    );
}

async fn test_storage(key_pairs: &[(String, ObjId)]) {
    let mut obj_map_builder = TrieObjectMapBuilder::new(
        HashMethod::Keccak256,
        Some(crate::CollectionStorageMode::Normal),
    )
    .await
    .unwrap();
    println!("Object map created");

    for (i, (key, obj_id)) in key_pairs.iter().enumerate() {
        println!("Test object map storage: {}/{} {}", i, key_pairs.len(), key);

        let old_value = obj_map_builder.put_object(key, obj_id).unwrap();
        assert!(
            old_value.is_none(),
            "Object already exists for key: {}",
            key
        );

        // Reinsert
        let old_value = obj_map_builder.put_object(key, obj_id).unwrap();
        assert!(
            old_value.is_some(),
            "Object should exist after reinsert for key: {}",
            key
        );
        assert_eq!(
            old_value.unwrap(),
            *obj_id,
            "Object ID mismatch for key: {}",
            key
        );

        // Remove
        let ret = obj_map_builder.remove_object(key).unwrap();
        assert!(ret.is_some(), "Object should be removed for key: {}", key);

        // get object after remove
        let ret = obj_map_builder.get_object(key).unwrap();
        assert!(
            ret.is_none(),
            "Object should not be found after remove for key: {}",
            key
        );

        // Reinsert key
        let old_value = obj_map_builder.put_object(key, obj_id).unwrap();
        assert!(
            old_value.is_none(),
            "Object should not exist after remove for key: {}",
            key
        );

        // Insert sub-key
        let sub_key = key
            .split('/')
            .filter_map(|s| if s.is_empty() { None } else { Some(s) })
            .next();

        if let Some(sub_key) = sub_key {
            assert!(!sub_key.is_empty(), "Sub-key should not be empty");

            let sub_obj_id = ObjId::new_by_raw(OBJ_TYPE_FILE.to_owned(), obj_id.obj_hash.clone());
            let old_value = obj_map_builder.put_object(sub_key, &sub_obj_id).unwrap();
            assert!(
                old_value.is_none(),
                "Sub-object already exists for key: {}",
                sub_key
            );

            // Reinsert sub-key
            let old_value = obj_map_builder.put_object(sub_key, &sub_obj_id).unwrap();
            assert!(
                old_value.is_some(),
                "Sub-object should exist after reinsert for key: {}",
                sub_key
            );
            assert_eq!(
                old_value.unwrap(),
                sub_obj_id,
                "Sub-object ID mismatch for key: {}",
                sub_key
            );

            // Remove sub-key
            let ret = obj_map_builder.remove_object(sub_key).unwrap();
            assert!(
                ret.is_some(),
                "Sub-object should be removed for key: {}",
                sub_key
            );
            assert_eq!(
                ret.unwrap(),
                sub_obj_id,
                "Sub-object ID mismatch for key: {}",
                sub_key
            );
        }
    }

    let obj_map = obj_map_builder.build().await.unwrap();
    let (id, obj_content) = obj_map.calc_obj_id();
    println!("All objects put {}", id);

    // Test iterator
    let mut iter = obj_map.iter().unwrap();
    let mut count = 0;
    while let Some((key, obj_id)) = iter.next() {
        // Verify the key and object ID in the key_pairs
        let ret = key_pairs.contains(&(key.clone(), obj_id.clone()));
        assert!(ret, "Key not found in key_pairs: {}", key);

        // Check if object in object map
        let ret = obj_map.get_object(&key).unwrap();
        assert!(ret.is_some(), "Object not found for key: {}", key);
        assert_eq!(ret.unwrap(), obj_id, "Object ID mismatch for key: {}", key);

        // println!("Iterated key: {}, obj_id: {}", key, obj_id);
        count += 1;
    }
    assert_eq!(count, key_pairs.len());
    drop(iter);

    for (key, obj_id) in key_pairs.iter() {
        let ret: Option<ObjId> = obj_map.get_object(key).unwrap();
        assert!(ret.is_some(), "Object not found for key: {}", key);
        assert_eq!(ret.unwrap(), *obj_id, "Object ID mismatch for key: {}", key);
    }

    // Save the object map to storage
    let storage = GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY.get().unwrap();

    let file_path = obj_map.get_storage_file_path();
    assert!(file_path.is_some(), "Storage file path is None");
    println!("Object map saved to storage at: {:?}", file_path);

    // Load the object map from storage
    let content = serde_json::from_str(&obj_content).unwrap();
    let loaded_obj_map = TrieObjectMap::open(content).await.unwrap();
    println!("Object map loaded from storage");

    // List all objects in the loaded object map
    let mut iter = loaded_obj_map.iter().unwrap();
    let mut count = 0;
    while let Some((key, obj_id)) = iter.next() {
        // Verify the key and object ID in the key_pairs
        let ret = key_pairs.contains(&(key.clone(), obj_id.clone()));
        assert!(ret, "Key not found in key_pairs: {}", key);

        // Check if object in object map
        let ret = loaded_obj_map.get_object(&key).unwrap();
        assert!(ret.is_some(), "Object not found for key: {}", key);
        assert_eq!(ret.unwrap(), obj_id, "Object ID mismatch for key: {}", key);

        // println!("Loaded iterated key: {}, obj_id: {}", key, obj_id);
        count += 1;
    }
    println!("Total loaded objects: {}", count);
    assert_eq!(
        loaded_obj_map.len(),
        obj_map.len(),
        "Loaded object map length mismatch"
    );

    // Verify the loaded object map
    for (key, obj_id) in key_pairs.iter() {
        // println!("Verifying key: {}, obj_id: {}", key, obj_id);
        let ret: Option<ObjId> = loaded_obj_map.get_object(key).unwrap();
        assert!(ret.is_some(), "Object not found for key: {}", key);
        assert_eq!(ret.unwrap(), *obj_id, "Object ID mismatch for key: {}", key);
    }

    // Clone for modification
    let mut cloned_obj_map = TrieObjectMapBuilder::from_trie_object_map(&loaded_obj_map)
        .await
        .unwrap();

    println!("Object map cloned for modification");
    for (key, obj_id) in key_pairs.iter() {
        let ret = cloned_obj_map.get_object(key).unwrap();
        assert!(ret.is_some(), "Object not found for key: {}", key);
        assert_eq!(ret.unwrap(), *obj_id, "Object ID mismatch for key: {}", key);
    }

    // Remove first object
    if let Some((key, obj_id)) = key_pairs.first() {
        let ret = cloned_obj_map.remove_object(key).unwrap();
        assert!(ret.is_some(), "Object not found for key: {}", key);
        assert_eq!(ret.unwrap(), *obj_id, "Object ID mismatch for key: {}", key);
        println!("Removed object: {}, obj_id: {}", key, obj_id);
    } else {
        panic!("No key pairs to remove");
    }

    // Gen new object map
    let cloned_obj_map = cloned_obj_map.build().await.unwrap();
    let new_obj_id = cloned_obj_map.get_obj_id();
    println!("New object map ID: {}", new_obj_id);
    assert_ne!(
        *new_obj_id, id,
        "New object map ID should be different from original"
    );

    // Verify the removed object
    let ret: Option<ObjId> = cloned_obj_map
        .get_object(key_pairs.first().unwrap().0.as_ref())
        .unwrap();
    assert!(ret.is_none(), "Object should not be found after removal");
}

#[test]
async fn test_trie_object_map() {
    init_logging("test_trie_object_map", false);

    let temp_dir = std::env::temp_dir();
    let data_dir = temp_dir.join("ndn_test_trie_object_map");
    if !data_dir.exists() {
        std::fs::create_dir_all(&data_dir).expect("Failed to create test data directory");
    } else {
        // Clean up the directory if it already exists
        // std::fs::remove_dir_all(&data_dir).expect("Failed to remove existing test data directory");
        // std::fs::create_dir_all(&data_dir).expect("Failed to recreate test data directory");
    }

    GLOBAL_TRIE_OBJECT_MAP_STORAGE_FACTORY
        .set(TrieObjectMapStorageFactory::new(
            data_dir,
            Some(TrieObjectMapStorageType::JSONFile),
        ))
        .unwrap_or_else(|_| panic!("Failed to set global trie object map storage factory"));

    println!("Test object map");
    let key_pairs = generate_key_value_pairs("test", 5);
    println!("Key pairs generated");

    test_storage(key_pairs.as_slice()).await;
    println!("Test storage completed");

    // test_iterator(key_pairs.as_slice()).await;
    test_traverse(key_pairs.as_slice()).await;
    println!("Test iterator and traverse completed");

    test_op_and_proof(key_pairs.as_slice()).await;
    println!("Test operation and proof completed");

    // test_op_and_proof(key_pairs.as_slice()).await;
    tokio::task::spawn(async move {
        test_traverse(key_pairs.as_slice()).await;
    })
    .await
    .unwrap();
    println!("Test traverse in async task completed");

    println!("Test object map completed");
}
