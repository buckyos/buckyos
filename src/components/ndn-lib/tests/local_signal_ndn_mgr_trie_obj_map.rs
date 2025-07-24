use std::collections::HashMap;

use buckyos_kit::*;
use hex::ToHex;
use log::*;
use ndn_lib::*;

use test_ndn::*;

const IS_IGNORE: bool = true;

#[tokio::test]
async fn ndn_local_trie_obj_map_basic() {
    init_logging("ndn_local_trie_obj_map_basic", false);

    info!("ndn_local_trie_obj_map_basic test start...");
    init_trie_obj_map_storage_factory().await;

    let _rng = rand::rng();
    let chunk_fix_size: u64 = 1024 * 1024 + 513; // 1MB + x bytes

    let _chunk_size1: u64 = chunk_fix_size;
    let (chunk_id1, chunk_data1) = generate_random_chunk_mix(chunk_fix_size);

    let _chunk_size2: u64 = chunk_fix_size;
    let (chunk_id2, chunk_data2) = generate_random_chunk_mix(chunk_fix_size);

    let _chunk_size3: u64 = chunk_fix_size;
    let (chunk_id3, chunk_data3) = generate_random_chunk_mix(chunk_fix_size);

    let _chunk_size4: u64 = chunk_fix_size;
    let (chunk_id4, chunk_data4) = generate_random_chunk_mix(chunk_fix_size);

    let _chunk_size5: u64 = chunk_fix_size;
    let (chunk_id5, chunk_data5) = generate_random_chunk_mix(chunk_fix_size);

    let chunks = HashMap::from([
        ("chunk1", (chunk_id1, chunk_data1)),
        ("chunk2", (chunk_id2, chunk_data2)),
        ("chunk3", (chunk_id3, chunk_data3)),
        ("chunk4", (chunk_id4, chunk_data4)),
        ("chunk5", (chunk_id5, chunk_data5)),
    ]);

    let mut obj_map = TrieObjectMapBuilder::new(HashMethod::Sha256, None)
        .await
        .expect("create ObjectMap failed");

    for (name, (chunk_id, _chunk_data)) in chunks.iter() {
        obj_map
            .put_object(*name, &chunk_id.to_obj_id())
            .expect(&format!("put {} to trie-obj-map failed", name));
    }

    let obj_map = obj_map.build().await.expect("build ObjectMap failed");

    assert_eq!(obj_map.len(), 5, "trie-obj-map total size check failed");

    let obj_map_root_hash = obj_map.get_root_hash();

    let verifier = TrieObjectMapProofVerifierHelper::new(HashMethod::Sha256);
    for (name, (chunk_id, _chunk_data)) in chunks.iter() {
        let obj_id = obj_map
            .get_object(*name)
            .expect(&format!("get {} from trie-obj-map failed", name))
            .expect("object should be some");
        assert_eq!(
            obj_id,
            chunk_id.to_obj_id(),
            "trie-obj-map {} object id check failed",
            name
        );
        let proof = obj_map
            .get_object_proof_path(name)
            .expect("get object with proof failed")
            .expect("object with proof should be some");
        assert_eq!(
            proof.root_hash(),
            obj_map_root_hash.as_slice(),
            "proof item object id check failed"
        );
        let verify_ret = verifier
            .verify_object(name, Some(&obj_id), &proof)
            .expect("verify object map failed");
        assert_eq!(
            verify_ret,
            TrieObjectMapProofVerifyResult::Ok,
            "verify object map failed for object {}",
            name
        );
    }

    let notexist_obj_id = obj_map
        .get_object("notexist")
        .expect("get notexist from trie-obj-map failed");
    assert!(
        notexist_obj_id.is_none(),
        "trie-obj-map[notexist] should be none",
    );
    let notexist_proof = obj_map
        .get_object_proof_path("notexist")
        .expect("get object with proof failed")
        .expect("proof for not exist item");
    let ret = verifier
        .verify("notexist", None, &notexist_proof)
        .expect("verify not exist item should success");
    assert_eq!(
        ret,
        TrieObjectMapProofVerifyResult::Ok,
        "verify not exist item should return Ok"
    );

    let new_chunk = generate_random_chunk(chunk_fix_size);
    let proof = obj_map
        .get_object_proof_path("chunk1")
        .expect("get object with proof failed")
        .expect("object with proof should be some");
    let verify_ret = verifier
        .verify_object("chunk1", Some(&new_chunk.0.to_obj_id()), &proof)
        .expect("verify chunk list should success for exclude object");
    assert_ne!(
        verify_ret,
        TrieObjectMapProofVerifyResult::Ok,
        "verify chunk list should fail for fake object"
    );

    let verify_ret = verifier
        .verify_object(
            "fake-key",
            Some(&chunks.get("chunk1").unwrap().0.to_obj_id()),
            &proof,
        )
        .expect("verify chunk list should success for exclude object");
    assert_ne!(
        verify_ret,
        TrieObjectMapProofVerifyResult::Ok,
        "verify chunk list should fail for fake key"
    );

    let verify_ret = verifier
        .verify_object("fake-key", Some(&new_chunk.0.to_obj_id()), &proof)
        .expect("verify chunk list should success for exclude object");
    assert_ne!(
        verify_ret,
        TrieObjectMapProofVerifyResult::Ok,
        "verify chunk list should fail for fake key and obj"
    );

    let mut fake_root_proof = proof.clone();
    fake_root_proof.root_hash.as_mut_slice()[0] ^= 1;
    let verify_ret = verifier
        .verify_object(
            "chunk1",
            Some(&chunks.get("chunk1").unwrap().0.to_obj_id()),
            &fake_root_proof,
        )
        .expect("verify chunk list should success for exclude object");
    assert_ne!(
        verify_ret,
        TrieObjectMapProofVerifyResult::Ok,
        "verify chunk list should fail for fake root"
    );

    info!("ndn_local_trie_obj_map_basic test end.");
}

#[tokio::test]
async fn ndn_local_trie_obj_map_ok() {
    init_logging("ndn_local_trie_obj_map_ok", false);

    info!("ndn_local_trie_obj_map_ok test start...");
    init_trie_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(5, None);
    let _total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map_builder = TrieObjectMapBuilder::new(HashMethod::Sha256, None)
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map_builder
            .put_object(chunk_id.to_string().as_str(), &chunk_id.to_obj_id())
            .expect("put chunk to trie-obj-map failed");
    }

    let obj_map = obj_map_builder
        .build()
        .await
        .expect("build ObjectMap failed");

    let (obj_map_id, obj_map_str) = obj_map.calc_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_map_id, obj_map_str.as_str())
        .await
        .expect("put obj-map failed");
    let obj_map_json = NamedDataMgr::get_object(Some(ndn_mgr_id.as_str()), &obj_map_id, None)
        .await
        .expect("get obj-map failed");

    let got_obj_map = TrieObjectMap::open(obj_map_json)
        .await
        .expect("open trie-obj-map from trie-obj-map id failed");

    let got_obj_map_id = got_obj_map.get_obj_id();
    assert_eq!(got_obj_map_id, &obj_map_id, "trie-obj-map id check failed");
    assert_eq!(
        got_obj_map
            .iter()
            .expect("iter got trie-obj-map failed")
            .map(|_| 1)
            .sum::<usize>(),
        chunks.len(),
        "trie-obj-map total size check failed"
    );

    for (key, obj_id) in got_obj_map.iter().expect("iter trie-obj-map failed") {
        let expect_obj_id = obj_map
            .get_object(key.as_str())
            .expect("get item from trie-obj-map failed")
            .expect("object should be some");
        assert_eq!(
            got_obj_map
                .get_object(key.as_str())
                .expect("get item failed")
                .expect("object should be some"),
            expect_obj_id,
            "item {} object check failed",
            key
        );

        assert_eq!(obj_id, expect_obj_id, "item {} object check failed", key);

        let (chunk_id, chunk_data) = chunks
            .iter()
            .find(|(id, _)| id.to_obj_id() == obj_id)
            .expect("should find chunk in chunks");
        let _got_chunk = NamedDataMgrTest::read_chunk_with_check(
            ndn_mgr_id.as_str(),
            &chunk_id,
            chunk_data.as_slice(),
        )
        .await;
    }

    info!("ndn_local_trie_obj_map_ok test end.");
}

#[tokio::test]
async fn ndn_local_trie_obj_map_not_found() {
    init_logging("ndn_local_trie_obj_map_not_found", false);

    info!("ndn_local_trie_obj_map_not_found test start...");
    let _storage_dir = init_trie_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(5, None);
    let _total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map_builder = TrieObjectMapBuilder::new(HashMethod::Sha256, None)
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map_builder
            .put_object(chunk_id.to_string().as_str(), &chunk_id.to_obj_id())
            .expect("put chunk to trie-obj-map failed");
    }

    let obj_map = obj_map_builder
        .build()
        .await
        .expect("build ObjectMap failed");

    let (obj_map_id, obj_map_str) = obj_map.calc_obj_id();

    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_map_id, obj_map_str.as_str())
        .await
        .expect("put obj-map failed");
    let obj_map_json = NamedDataMgr::get_object(Some(ndn_mgr_id.as_str()), &obj_map_id, None)
        .await
        .expect("get obj-map failed");

    // delete the object map storage file
    let remove_ret = std::fs::remove_file(
        obj_map
            .get_storage_file_path()
            .expect("get storage file path failed")
            .as_path(),
    );
    assert!(
        remove_ret.is_ok(),
        "remove object map storage file failed: {:?}",
        remove_ret
    );

    TrieObjectMap::open(obj_map_json)
        .await
        .map(|_| ())
        .expect_err(
            "open trie-obj-map from trie-obj-map id should failed for the storage is removed",
        );

    info!("ndn_local_trie_obj_map_not_found test end.");
}

#[tokio::test]
async fn ndn_local_trie_obj_map_verify_failed() {
    init_logging("ndn_local_trie_obj_map_verify_failed", false);

    info!("ndn_local_trie_obj_map_verify_failed test start...");
    let _storage_dir = init_trie_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let verifier = TrieObjectMapProofVerifierHelper::new(HashMethod::Sha256);

    let chunks = generate_random_chunk_list(5, None);
    let _total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map_builder = TrieObjectMapBuilder::new(HashMethod::Sha256, None)
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map_builder
            .put_object(chunk_id.to_string().as_str(), &chunk_id.to_obj_id())
            .expect("put chunk to trie-obj-map failed");
    }

    let obj_map = obj_map_builder
        .build()
        .await
        .expect("build ObjectMap failed");

    let (obj_map_id, obj_map_str) = obj_map.calc_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_map_id, obj_map_str.as_str())
        .await
        .expect("put obj-map failed");
    let obj_map_json = NamedDataMgr::get_object(Some(ndn_mgr_id.as_str()), &obj_map_id, None)
        .await
        .expect("get obj-map failed");

    let (append_chunk_id, _append_chunk_data) = generate_random_chunk(1024 * 1024);
    let mut append_obj_map_builder = TrieObjectMapBuilder::from_trie_object_map(&obj_map)
        .await
        .expect("create append ObjectMap failed");

    append_obj_map_builder
        .put_object(
            append_chunk_id.to_string().as_str(),
            &append_chunk_id.to_obj_id(),
        )
        .expect("put append chunk to trie-obj-map failed");

    let append_obj_map = append_obj_map_builder
        .build()
        .await
        .expect("build append ObjectMap failed");

    let (_append_obj_map_id, _append_obj_map_str) = append_obj_map.calc_obj_id();
    let obj_map_storage_file_path = obj_map
        .get_storage_file_path()
        .expect("get storage file path failed");
    let append_obj_map_storage_file_path = append_obj_map
        .get_storage_file_path()
        .expect("get append storage file path failed");
    // instead the chunk list storage file
    let remove_ret = std::fs::remove_file(obj_map_storage_file_path.as_path());
    assert!(
        remove_ret.is_ok(),
        "remove chunk list storage file failed: {:?}",
        remove_ret
    );
    let copy_ret = std::fs::copy(
        append_obj_map_storage_file_path.as_path(),
        obj_map_storage_file_path.as_path(),
    );

    assert!(
        copy_ret.is_ok(),
        "instead append chunk list storage file failed, remove: {:?}, copy: {:?}, {:?} -> {:?}",
        remove_ret,
        copy_ret,
        append_obj_map_storage_file_path,
        obj_map_storage_file_path
    );

    let fake_obj_map_builder = TrieObjectMapBuilder::open(obj_map_json)
        .await
        .expect("build chunk list from ndn-mgr should success for object-array has been replaced");

    let fake_obj_map = fake_obj_map_builder
        .build()
        .await
        .expect("build fake ObjectMap failed");

    for (chunk_id, _chunk_data) in chunks.iter() {
        let key = chunk_id.to_string();
        let obj_id = obj_map
            .get_object(key.as_str())
            .expect("get object from trie-obj-map failed")
            .expect("object should be some");

        let _fake_chunk_id = fake_obj_map.get_object(key.as_str()).expect_err(
            "get object from fake trie-obj-map should be failed for root hash has been replaced",
        );

        // assert_eq!(
        //     fake_chunk_id, obj_id,
        //     "trie-obj-map {} object check failed after replace",
        //     key
        // );

        let proof = obj_map
            .get_object_proof_path(key.as_str())
            .expect("get_object_proof_path should success for chunk_list has been replaced")
            .expect("get_object_proof_path should return error");
        assert_eq!(
            proof.root_hash,
            obj_map.get_root_hash(),
            "root-hash in proof should same with obj-map.root_hash"
        );
        let ret = verifier
            .verify_object(key.as_str(), Some(&obj_id), &proof)
            .expect("should verify success");
        assert_eq!(
            ret,
            TrieObjectMapProofVerifyResult::Ok,
            "should verify success"
        );
        let _fake_proof = fake_obj_map
            .get_object_proof_path(key.as_str())
            .expect_err("get_object_proof_path should failed for root hash has been replaced");
        // assert_eq!(
        //     fake_proof.root_hash,
        //     append_obj_map.get_root_hash(),
        //     "root-hash in fake-proof should same with append obj-map.root_hash"
        // );
        // verifier
        //     .verify_object(key.as_str(), &obj_id, &fake_proof)
        //     .expect_err("should failed for chunk_list has been replaced");
    }

    let _fake_proof = fake_obj_map
        .get_object_proof_path(append_chunk_id.to_string().as_str())
        .expect_err("get_object_proof_path should failed for root hash has been replaced");
    // let verify_ret = verifier
    //     .verify_object(
    //         append_chunk_id.to_string().as_str(),
    //         &append_chunk_id.to_obj_id(),
    //         &fake_proof,
    //     )
    //     .expect("should success for chunk_list has been replaced");
    // assert_ne!(
    //     verify_ret,
    //     TrieObjectMapProofVerifyResult::Ok,
    //     "should success for item is in fake chunk_list"
    // );

    let fake_proof = append_obj_map
        .get_object_proof_path(append_chunk_id.to_string().as_str())
        .expect("get_object_proof_path should success for append chunk in append obj-map")
        .expect("get_object_proof_path should return error");
    {
        let ret = verifier
            .verify_object(
                append_chunk_id.to_string().as_str(),
                Some(&append_chunk_id.to_obj_id()),
                &fake_proof,
            )
            .expect("should failed for chunk_list has been replaced");

        assert_eq!(
            ret,
            TrieObjectMapProofVerifyResult::Ok,
            "should verify success for fake root hash in proof"
        );
    }
    {
        let mut fake_proof = fake_proof.clone();
        fake_proof.root_hash = obj_map_id.obj_hash.clone();
        let ret = verifier
            .verify_object(
                append_chunk_id.to_string().as_str(),
                Some(&append_chunk_id.to_obj_id()),
                &fake_proof,
            )
            .expect("should failed for chunk_list has been replaced");

        assert_eq!(
            ret,
            TrieObjectMapProofVerifyResult::RootMismatch,
            "should verify failed for fake root hash in proof"
        );
    }

    {
        let ret = verifier
            .verify_object("fake-key", Some(&append_chunk_id.to_obj_id()), &fake_proof)
            .expect("should failed for chunk_list has been replaced");

        assert_eq!(
            ret,
            TrieObjectMapProofVerifyResult::ValueMismatch,
            "should verify failed for fake key in proof"
        );
    }

    {
        let ret = verifier
            .verify_object(
                append_chunk_id.to_string().as_str(),
                Some(&chunks.get(0).unwrap().0.to_obj_id()),
                &fake_proof,
            )
            .expect("should failed for chunk_list has been replaced");

        assert!(
            ret != TrieObjectMapProofVerifyResult::Ok,
            "should verify failed for fake value in proof"
        );

        if !IS_IGNORE {
            assert_eq!(
                ret,
                TrieObjectMapProofVerifyResult::ValueMismatch,
                "should verify failed for fake value in proof"
            );
        }
    }

    info!("ndn_local_trie_obj_map_verify_failed test end.");
}
