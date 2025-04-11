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

#[test]
async fn test_object_map() {
    let storage = Box::new(MemoryStorage::new()) as Box<dyn InnerStorage>;
    let mut obj_map = ObjectMap::new(HashMethod::Sha256, storage).await.unwrap();

    let count = 100;
    for i in 0..count {
        let key = format!("key{}", i);
        let hash = generate_random_buf(&i.to_string(), HashMethod::Sha256.hash_bytes());
        let meta = generate_random_buf(&key, 16);
        let obj_id = ObjId::new_by_raw(OBJ_TYPE_FILE.to_owned(), hash);

        obj_map.put_object(&key, obj_id.clone(), Some(meta.clone()))
            .await
            .unwrap();

        // Test get object
        let ret = obj_map.get_object(&key).await.unwrap().unwrap();
        assert_eq!(ret.obj_id, obj_id);
        assert_eq!(ret.meta, Some(meta.clone()));

        // Test exist
        let ret = obj_map.is_object_exist(&key).await.unwrap();
        assert_eq!(ret, true);

        // Test remove
        if i % 2 == 0 {
            let ret = obj_map.remove_object(&key).await.unwrap().unwrap();
            assert_eq!(ret.0, obj_id);
            assert_eq!(ret.1, Some(meta));
        }
    }

    obj_map.flush().await.unwrap();

    let objid = obj_map.gen_obj_id().unwrap();
    println!("objid: {}", objid.to_string());

    for i in 0..count {
        let key = format!("key{}", i);
        if i % 2 == 0 {
            let ret = obj_map.get_object(&key).await.unwrap();
            assert_eq!(ret.is_none(), true);
        } else {
            let ret = obj_map.get_object(&key).await.unwrap().unwrap();
            assert_eq!(ret.meta.is_some(), true);

            let proof = obj_map.get_object_proof_path(&key).await.unwrap();
            assert!(proof.is_some());
            let proof = proof.unwrap();

            let verifier = ObjectMapProofVerifier::new(obj_map.hash_method());
            let ret = verifier.verify(&objid, &proof).unwrap(); 
            assert_eq!(ret, true);
        }
    }
}
