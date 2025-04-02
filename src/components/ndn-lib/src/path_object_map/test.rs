use std::sync::Arc;
use crate::ObjId;
use super::object_map::PathObjectMap;
use crate::OBJ_TYPE_FILE;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::test;
use crate::hash::{HashMethod, HashHelper};

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
        let seg: String = (0..seg_len).map(|_| rng.random_range(b'a'..=b'z') as char).collect();
        path.push('/');
        path.push_str(&seg);
    }
    path
}

fn generate_key_value_pair(seed: &str, count: usize) -> (String, ObjId, Vec<u8>) {
    let seed = HashHelper::calc_hash(HashMethod::Sha256, seed.as_bytes());
    let mut rng: StdRng = SeedableRng::from_seed(seed.try_into().unwrap());

    let hash = generate_random_buf(&mut rng, HashMethod::Keccak256.hash_bytes());
    let obj_id = ObjId::new_by_raw(OBJ_TYPE_FILE.to_owned(), hash);

    let meta = generate_random_buf(&mut rng, 16);
    (generate_random_path_key(&mut rng), obj_id, meta)
}

fn generate_key_value_pairs(seed: &str, count: usize) -> Vec<(String, ObjId, Vec<u8>)> {
    let mut pairs = Vec::new();
    for i in 0..count {
        pairs.push(generate_key_value_pair(&format!("{}-{}", seed, i), count));
    }
    pairs
}

#[test]
async fn test_path_object_map() {
    println!("Test object map");
    let key_pairs = generate_key_value_pairs("test", 100);
    println!("Key pairs generated");
    let mut obj_map = PathObjectMap::new(HashMethod::Keccak256).await;
    println!("Object map created");

    let count = 100;
    for i in 0..count {
        println!("Test object map: {}/{}", i, count);
        let key = key_pairs[i].0.as_ref();
        let obj_id = key_pairs[i].1.clone();
        let meta = key_pairs[i].2.clone();

        obj_map.put_object(key, obj_id.clone(), Some(meta.clone())).await.unwrap();

        println!("Put object success: {}", key);

        // Test get object
        let ret = obj_map.get_object(key).await.unwrap();
        if ret.is_none() {
            panic!("Object not found for key: {}, {}", i, key);
        }
        let ret = ret.unwrap();
        assert_eq!(ret.obj_id, obj_id);
        assert_eq!(ret.meta, Some(meta.clone()));

        println!("Get object success: {}", key);

        // Test exist
        let ret = obj_map.is_object_exist(&key).await.unwrap();
        assert_eq!(ret, true);

        println!("Object exist: {}", key);

        // Test remove
        if i % 2 == 0 {
            let ret = obj_map.remove_object(&key).await.unwrap().unwrap();
            assert_eq!(ret.0, obj_id);
            assert_eq!(ret.1, Some(meta));

            println!("Remove object success: {}", key);
        }
    }
}