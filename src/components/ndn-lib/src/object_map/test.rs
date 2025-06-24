use super::memory_storage::MemoryStorage;
use super::*;
use crate::hash::HashHelper;
use crate::{HashMethod, ObjId, OBJ_TYPE_FILE};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::test;

fn generate_random_buf(seed: &str, len: usize) -> Vec<u8> {
    let seed = HashHelper::calc_hash(HashMethod::Sha256, seed.as_bytes());
    let mut rng: StdRng = SeedableRng::from_seed(seed.try_into().unwrap());
    let mut buf = vec![0u8; len];
    rng.fill(&mut buf[..]);
    buf
}

async fn test_object_map() {
    let mut obj_map = ObjectMap::new(HashMethod::Sha256, None).await.unwrap();

    let count = 100;
    for i in 0..count {
        let key = format!("key{}", i);
        let hash = generate_random_buf(&i.to_string(), HashMethod::Sha256.hash_bytes());
        let obj_id = ObjId::new_by_raw(OBJ_TYPE_FILE.to_owned(), hash);

        obj_map.put_object(&key, &obj_id).await.unwrap();

        // Test get object
        let ret = obj_map.get_object(&key).await.unwrap().unwrap();
        assert_eq!(ret, obj_id);

        // Test exist
        let ret = obj_map.is_object_exist(&key).await.unwrap();
        assert_eq!(ret, true);

        // Test remove
        if i % 2 == 0 {
            let ret = obj_map.remove_object(&key).await.unwrap().unwrap();
            assert_eq!(ret, obj_id);
        }
    }

    obj_map.flush().await.unwrap();

    let (objid, obj_content) = obj_map.calc_obj_id().unwrap();
    println!("objid: {}", objid.to_string());

    obj_map.save().await.unwrap();

    for i in 0..count {
        let key = format!("key{}", i);
        if i % 2 == 0 {
            let ret = obj_map.get_object(&key).await.unwrap();
            assert_eq!(ret.is_none(), true);
        } else {
            let ret = obj_map.get_object(&key).await.unwrap().unwrap();

            let proof = obj_map.get_object_proof_path(&key).await.unwrap();
            assert!(proof.is_some());
            let proof = proof.unwrap();

            let verifier = ObjectMapProofVerifier::new(obj_map.hash_method());
            let ret = verifier
                .verify_with_obj_data_str(&obj_content, &proof)
                .unwrap();
            assert_eq!(ret, true);
        }
    }

    // Test reopen object map for read
    let obj_content = serde_json::from_str(&obj_content).unwrap();
    let mut obj_map2 = ObjectMap::open(obj_content, true).await.unwrap();
    obj_map2.flush().await.unwrap();

    let objid2 = obj_map2.get_obj_id().unwrap();
    assert_eq!(objid, objid2, "Object ID unmatch");

    // Test clone for modify
    let mut obj_map3 = obj_map2.clone(false).await.unwrap();

    // Remove some objects
    let obj_item1 = obj_map3.remove_object("key0").await.unwrap();
    let obj_item2 = obj_map3.remove_object("key1").await.unwrap();

    println!("obj_item1: {:?}", obj_item1);
    println!("obj_item2: {:?}", obj_item2);
    assert!(obj_item1.is_none(), "Remove object failed");
    assert!(obj_item2.is_some(), "Remove object failed");

    // Regenerate container ID
    obj_map3.flush().await.unwrap();
    let (objid3, _content) = obj_map3.calc_obj_id().unwrap();
    assert_ne!(objid, objid3, "Object ID unmatch");

    // Then save it to new file
    obj_map3.save().await.unwrap();

    // Clone for read-only
    let obj_map_read = obj_map3.clone(true).await.unwrap();
    let objid4 = obj_map_read.get_obj_id().unwrap();
    assert_eq!(objid3, objid4, "Object ID unmatch");

    // Then reinsert key1
    obj_map3
        .put_object("key1", &obj_item2.unwrap())
        .await
        .unwrap();

    obj_map3.flush().await.unwrap();
    let objid4 = obj_map3.get_obj_id().unwrap();
    assert_eq!(objid, objid4, "Object ID unmatch");

    // Test Iterator
    let mut iter = obj_map3.iter();
    let mut count = 0;
    while let Some((key, obj_id, _)) = iter.next() {
        println!("key: {}, obj_id: {}", key, obj_id.to_string());
        count += 1;
    }
}

#[test]
async fn test_object_map_main() {
    buckyos_kit::init_logging("test-object-map", false);

    // First init global object map storage factory
    let data_dir = std::env::temp_dir().join("ndn-test-object-map");
    println!("data_dir: {}", data_dir.display());
    let factory = ObjectMapStorageFactory::new(&data_dir, Some(ObjectMapStorageType::JSONFile));

    GLOBAL_OBJECT_MAP_STORAGE_FACTORY
        .set(factory)
        .unwrap_or_else(|_| panic!("Failed to set global object map storage factory"));

    test_object_map().await;
}
