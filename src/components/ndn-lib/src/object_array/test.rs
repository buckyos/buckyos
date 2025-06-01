use super::*;
use crate::hash::HashHelper;
use crate::hash::HashMethod;
use crate::{ObjId, OBJ_TYPE_FILE};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

struct TestNdnDataManager {
    
}

fn generate_random_buf(rng: &mut StdRng, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];

    rng.fill(&mut buf[..]);
    buf
}

fn gen_random_obj_id(seed: &str) -> ObjId {
    let seed = HashHelper::calc_hash(HashMethod::Sha256, seed.as_bytes());
    let mut rng: StdRng = SeedableRng::from_seed(seed.try_into().unwrap());

    let hash = generate_random_buf(&mut rng, HashMethod::Sha256.hash_bytes());
    let obj_id = ObjId::new_by_raw(OBJ_TYPE_FILE.to_owned(), hash);

    obj_id
}

async fn test_object_array() {
    let mut ar = ObjectArray::new(HashMethod::Sha256, Some(ObjectArrayStorageType::JSONFile));

    let meta = format!("Test object array with {} items", ar.len());
    ar.set_meta(Some(meta.clone())).unwrap();

    for i in 0..100 {
        let obj_id = gen_random_obj_id(&format!("test-{}", i));
        ar.append_object(&obj_id).unwrap();

        // Test get
        let ret = ar.get_object(i).unwrap();
        assert_eq!(ret.as_ref(), Some(&obj_id), "Get object failed");
    }

    // Flush object id, and will gen mtree inner
    let id = ar.get_obj_id();
    assert!(id.is_none());

    let id = ar.calc_obj_id().await.unwrap();
    println!("Object ID: {}", id.to_string());
    assert_eq!(id, ar.get_obj_id().unwrap(), "Get object ID failed");

    // Save to file
    let data_dir = std::env::temp_dir().join("ndn-test-object-array");

    // Set some meta data
    let meta1 = format!("Test object array with {} items again", ar.len());
    ar.set_meta(Some(meta1.clone())).unwrap();

    ar.save().await.unwrap();
    ar.flush().await.unwrap();


    // Test load from file, in read-only mode
    let mut reader = ObjectArray::open(&id, true).await.unwrap();

    let id2 = reader.calc_obj_id().await.unwrap();
    assert_eq!(id, id2, "Load object ID unmatch");

    // Test get object and verify the value with the original object array
    for i in 0..100 {
        let obj_id = gen_random_obj_id(&format!("test-{}", i));
        let ret = reader.get_object(i).unwrap();
        assert_eq!(ret.as_ref(), Some(&obj_id), "Get object failed");
    }

    // Test get meta
    let meta2 = reader.get_meta().unwrap().unwrap();
    assert_eq!(meta1, meta2, "Get meta failed");

    // Test get with proof path
    let item = reader.get_object_with_proof(0).await.unwrap().unwrap();

    let verifier = ObjectArrayProofVerifier::new(HashMethod::Sha256);
    let vret = verifier.verify(
        &id,
        &item.obj_id,
        &item.proof,
    ).unwrap();
    assert!(vret, "Verify proof failed");

    // Test unmatch proof
    let item1 = reader.get_object_with_proof(1).await.unwrap().unwrap();
    let vret = verifier.verify(
        &id,
        &item1.obj_id,
        &item1.proof,
    ).unwrap();
    assert!(vret, "Verify proof failed");

    let vret = verifier.verify(
        &id,
        &item.obj_id,
        &item1.proof,
    ).unwrap();

    assert!(!vret, "Verify proof failed");

    let vret = verifier.verify(
        &id,
        &item1.obj_id,
        &item.proof,
    ).unwrap();

    assert!(!vret, "Verify proof failed");


    // Try modify, reader is in read-only mode, so we should clone it with read-write mode
    let mut ar2 = reader.clone(false).unwrap();

    let id2 = ar2.calc_obj_id().await.unwrap();
    assert_eq!(id, id2, "Load object ID unmatch");

    // Remove first item
    let item0 = ar2.remove_object(0).unwrap().unwrap();
    assert_eq!(item0, item.obj_id, "Remove object failed");

    // Pop last item
    let item99 = ar2.pop_object().unwrap().unwrap();

    // Recalculate object ID
    let id3 = ar2.calc_obj_id().await.unwrap();
    assert_ne!(id, id3, "Object ID should be different after remove");

    info!("Object ID Updated: {} -> {}", id.to_string(), id3.to_string());

    // Insert item0 at index 0
    ar2.insert_object(0, &item0).unwrap();
    assert_eq!(item0, ar2.get_object(0).unwrap().unwrap(), "Insert object failed");

    // Insert item99 at index 99
    ar2.insert_object(99, &item99).unwrap();
    assert_eq!(item99, ar2.get_object(99).unwrap().unwrap(), "Insert object failed");

    // Test get object and verify the value with the original object array
    for i in 0..100 {
        let obj_id = gen_random_obj_id(&format!("test-{}", i));
        let ret = ar2.get_object(i).unwrap();
        assert_eq!(ret.as_ref(), Some(&obj_id), "Get object failed");
    }

    // Recalculate object ID
    let id4 = ar2.calc_obj_id().await.unwrap();
    assert_eq!(id, id4, "Object ID should be the same after insert");
    assert_ne!(id3, id4, "Object ID should be different after remove");
}


#[tokio::test]
async fn test_object_array_main() {
    buckyos_kit::init_logging("test-object-array", false);

    // First init object array factory
    let data_dir = std::env::temp_dir().join("ndn-test-object-array");
    let factory = ObjectArrayStorageFactory::new(&data_dir);
    if let Err(_) = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.set(factory) {
        error!("Object array storage factory already initialized");
    }

    test_object_array().await;
}