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
    let mut builder = ObjectArrayBuilder::new(HashMethod::Sha256);

    for i in 0..100 {
        let obj_id = gen_random_obj_id(&format!("test-{}", i));
        builder.append_object(&obj_id).unwrap();

        // Test get
        let ret = builder.get_object(i).unwrap();
        assert_eq!(ret.as_ref(), Some(&obj_id), "Get object failed");
    }

    // Generate object array and save to file
    let ar = builder.build().await.unwrap();

    // Test Iterator
    {
        let mut iter = ar.iter();
        for i in 0..100 {
            let obj_id = gen_random_obj_id(&format!("test-{}", i));
            let ret = iter.next().unwrap();
            assert_eq!(ret, obj_id, "Iterator get object failed");
        }
        assert!(iter.next().is_none(), "Iterator should be empty after all items");

        let mut iter = ar.iter().rev();
        for i in (0..100).rev() {
            let obj_id = gen_random_obj_id(&format!("test-{}", i));
            let ret = iter.next().unwrap();
            assert_eq!(ret, obj_id, "Reverse iterator get object failed");
        }
        assert!(iter.next().is_none(), "Reverse iterator should be empty after all items");
    }

    // Flush object id, and will gen mtree inner
    let id = ar.get_obj_id();
    
    let (id, body) = ar.calc_obj_id();
    println!("Object ID: {}", id.to_string());
    assert_eq!(id, *ar.get_obj_id(), "Get object ID failed");

    // Save to file


    // Test load from file, in read-only mode
    let body = ar.body();
    let body = serde_json::to_value(&body).unwrap();
    let mut reader = ObjectArray::open(body).await.unwrap();

    let id2 = reader.get_obj_id();
    assert_eq!(id, *id2, "Load object ID unmatch");

    let (id2, _) = reader.calc_obj_id();
    assert_eq!(id, id2, "Load object ID unmatch");

    // Test get object and verify the value with the original object array
    for i in 0..100 {
        let obj_id = gen_random_obj_id(&format!("test-{}", i));
        let ret = reader.get_object(i).unwrap();
        assert_eq!(ret.as_ref(), Some(&obj_id), "Get object failed");
    }


    // Test get with proof path
    let item = reader.get_object_with_proof(0).await.unwrap().unwrap();
    let(_, reader_body) = reader.calc_obj_id();

    let verifier = ObjectArrayProofVerifier::new(HashMethod::Sha256);
    let vret = verifier.verify_with_obj_data_str(
        &reader_body,
        &item.obj_id,
        &item.proof,
    ).unwrap();
    assert!(vret, "Verify proof failed");

    // Test unmatch proof
    let item1 = reader.get_object_with_proof(1).await.unwrap().unwrap();
    let vret = verifier.verify_with_obj_data_str(
        &reader_body,
        &item1.obj_id,
        &item1.proof,
    ).unwrap();
    assert!(vret, "Verify proof failed");

    let vret = verifier.verify_with_obj_data_str(
        &reader_body,
        &item.obj_id,
        &item1.proof,
    ).unwrap();

    assert!(!vret, "Verify proof failed");

    let vret = verifier.verify_with_obj_data_str(
        &reader_body,
        &item1.obj_id,
        &item.proof,
    ).unwrap();

    assert!(!vret, "Verify proof failed");


    // Try modify, reader is in read-only mode, so we should clone it with read-write mode
    let mut ar2_builder = ObjectArrayBuilder::from_object_array(&reader).unwrap();

    // Remove first item
    let item0 = ar2_builder.remove_object(0).unwrap().unwrap();
    assert_eq!(item0, item.obj_id, "Remove object failed");

    // Pop last item
    let item99 = ar2_builder.pop_object().unwrap().unwrap();

    // Regenerate object array
    let ar2 = ar2_builder.build().await.unwrap();
    
    let (id3, _) = ar2.calc_obj_id();
    assert_ne!(id, id3, "Object ID should be different after remove");

    info!("Object ID Updated: {} -> {}", id.to_string(), id3.to_string());

    // Insert item0 at index 0
    let mut ar3_builder = ObjectArrayBuilder::from_object_array_owned(ar2);
    ar3_builder.insert_object(0, &item0).unwrap();
    assert_eq!(item0, ar3_builder.get_object(0).unwrap().unwrap(), "Insert object failed");

    // Insert item99 at index 99
    ar3_builder.insert_object(99, &item99).unwrap();
    assert_eq!(item99, ar3_builder.get_object(99).unwrap().unwrap(), "Insert object failed");

    // Test get object and verify the value with the original object array
    for i in 0..100 {
        let obj_id = gen_random_obj_id(&format!("test-{}", i));
        let ret = ar3_builder.get_object(i).unwrap();
        assert_eq!(ret.as_ref(), Some(&obj_id), "Get object failed");
    }

    // Regenerate object array
    let ar4 = ar3_builder.build().await.unwrap();

    let (id4, _) = ar4.calc_obj_id();
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