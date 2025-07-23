use std::{collections::HashMap, io::SeekFrom};

use buckyos_kit::*;
use hex::ToHex;
use log::*;
use ndn_lib::*;
use test_ndn::*;
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn ndn_local_obj_map_basic() {
    init_logging("ndn_local_obj_map_basic", false);

    info!("ndn_local_obj_map_basic test start...");
    init_obj_map_storage_factory().await;

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

    let mut obj_map_builder = ObjectMapBuilder::new(HashMethod::Sha256, None, false)
        .await
        .expect("create ObjectMap failed");

    for (name, (chunk_id, _chunk_data)) in chunks.iter() {
        obj_map_builder
            .put_object(*name, &chunk_id.to_obj_id())
            .expect("put chunk to obj-map failed");
    }

    let obj_map = obj_map_builder
        .build()
        .await
        .expect("build ObjectMap failed");

    assert_eq!(obj_map.len(), 5, "obj-map total size check failed");

    let (_obj_map_id, obj_map_str) = obj_map.calc_obj_id();
    let obj_map_body: ObjectMapBody =
        serde_json::from_str(&obj_map_str).expect("parse obj-map body from str should success");
    let verifier = ObjectMapProofVerifier::new(HashMethod::Sha256);
    for (name, (chunk_id, _chunk_data)) in chunks.iter() {
        let obj_id = obj_map
            .get_object(*name)
            .expect(&format!("get {} from obj-map failed", name))
            .expect("object should be some");
        assert_eq!(
            obj_id,
            chunk_id.to_obj_id(),
            "obj-map {} object id check failed",
            name
        );
        let proof = obj_map
            .get_object_proof_path(name)
            .await
            .expect("get object with proof failed")
            .expect("object with proof should be some");
        assert_eq!(proof.item.key, *name, "proof item key check failed");
        assert_eq!(
            proof.item.obj_id, obj_id,
            "proof item object id check failed"
        );
        let is_ok = verifier
            .verify(obj_map_body.root_hash.as_str(), &proof)
            .expect("verify object map failed");
        assert!(is_ok, "verify object map failed for object {}", name);
    }

    let notexist_obj_id = obj_map
        .get_object("notexist")
        .expect("get notexist from obj-map failed");
    assert!(
        notexist_obj_id.is_none(),
        "obj-map[notexist] should be none",
    );
    let notexist_proof = obj_map
        .get_object_proof_path("notexist")
        .await
        .expect("get object with proof failed");
    assert!(
        notexist_proof.is_none(),
        "obj-map[notexist] proof should be none",
    );

    let new_chunk = generate_random_chunk(chunk_fix_size);
    let proof = obj_map
        .get_object_proof_path("chunk1")
        .await
        .expect("get object with proof failed")
        .expect("object with proof should be some");
    let mut fake_obj_proof = proof.clone();
    fake_obj_proof.item.obj_id = new_chunk.0.to_obj_id(); // change the object id to new chunk id
    let _is_ok = verifier
        .verify(obj_map_body.root_hash.as_str(), &fake_obj_proof)
        // .expect("verify chunk list should success for exclude object");
        .expect_err("verify chunk list should fail for fake object proof");
    // assert!(!is_ok, "verify chunk list should fail for fake object proof");

    let mut fake_key_proof = proof.clone();
    fake_key_proof.item.key = "fake-key".to_string();
    let _is_ok = verifier
        .verify(obj_map_body.root_hash.as_str(), &fake_key_proof)
        // .expect("verify chunk list should success for exclude object");
        .expect_err("verify chunk list should fail for fake object proof");
    // assert!(!is_ok, "verify chunk list should fail for fake key");

    let mut fake_key_obj_proof = proof.clone();
    fake_key_obj_proof.item.key = "fake-key".to_string();
    fake_key_obj_proof.item.obj_id = new_chunk.0.to_obj_id(); // change the object id to new chunk id
    let _is_ok = verifier
        .verify(obj_map_body.root_hash.as_str(), &fake_key_obj_proof)
        // .expect("verify chunk list should success for exclude object");
        .expect_err("verify chunk list should fail for fake object proof");
    // assert!(!is_ok, "verify chunk list should fail for fake key and obj");

    let mut fake_root_hash = Base32Codec::from_base32(obj_map_body.root_hash.as_str())
        .expect("parse base32 root hash should success");
    fake_root_hash.as_mut_slice()[0] ^= 1;
    let _is_ok = verifier
        .verify(
            Base32Codec::to_base32(fake_root_hash.as_slice()).as_str(),
            &proof,
        )
        // .expect("verify chunk list should success for exclude object");
        .expect_err("verify chunk list should fail for fake object proof");
    // assert!(!is_ok, "verify chunk list should fail for root obj");

    info!("ndn_local_obj_map_basic test end.");
}

#[tokio::test]
async fn ndn_local_obj_map_ok() {
    init_logging("ndn_local_obj_map_ok", false);

    info!("ndn_local_obj_map_ok test start...");
    init_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(5, None);
    let _total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map_builder = ObjectMapBuilder::new(HashMethod::Sha256, None, false)
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map_builder
            .put_object(chunk_id.to_string().as_str(), &chunk_id.to_obj_id())
            .expect("put chunk to obj-map failed");
    }

    let obj_map = obj_map_builder
        .build()
        .await
        .expect("build ObjectMap failed");

    let (obj_map_id, obj_map_str) = obj_map.calc_obj_id();

    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_map_id, obj_map_str.as_str())
        .await
        .expect("put obj-map to ndn-mgr failed");

    let obj_map_json = NamedDataMgr::get_object(Some(ndn_mgr_id.as_str()), &obj_map_id, None)
        .await
        .expect("get obj-map from ndn-mgr failed");

    let got_obj_map = ObjectMap::open(obj_map_json)
        .await
        .expect("open obj-map from obj-map id failed");

    let (got_obj_map_id, got_obj_map_str) = got_obj_map.calc_obj_id();
    assert_eq!(got_obj_map_id, obj_map_id, "obj-map id check failed");
    assert_eq!(got_obj_map_str, obj_map_str, "obj-map str check failed");
    assert_eq!(
        got_obj_map.len(),
        chunks.len() as u64,
        "obj-map total size check failed"
    );

    for (key, obj_id, _x) in got_obj_map.iter() {
        assert_eq!(
            got_obj_map
                .get_object(key.as_str())
                .expect("get item failed")
                .expect("object should be some"),
            obj_map
                .get_object(key.as_str())
                .expect("get item from obj-map failed")
                .expect("object should be some"),
            "item {} object check failed",
            key
        );

        assert_eq!(
            obj_id,
            obj_map
                .get_object(key.as_str())
                .expect("get item from obj-map failed")
                .expect("object should be some"),
            "item {} object check failed",
            key
        );

        let (mut chunk_reader, chunk_size) = NamedDataMgr::open_chunk_reader(
            Some(ndn_mgr_id.as_str()),
            &ChunkId::from_obj_id(&obj_id),
            SeekFrom::Start(0),
            false,
        )
        .await
        .expect("open chunk list reader from ndn-mgr failed.");

        let (_chunk_id, chunk_data) = chunks
            .iter()
            .find(|(id, _)| id.to_obj_id() == obj_id)
            .expect("should find chunk in chunks");
        assert_eq!(
            chunk_size,
            chunk_data.len() as u64,
            "chunk size check failed"
        );

        let mut buffer = vec![0u8; chunk_size as usize];
        chunk_reader
            .read_exact(&mut buffer)
            .await
            .expect("read chunk list from ndn-mgr failed");
        //assert_eq!(&buffer, chunk_data, "chunk_data content check failed");
    }

    info!("ndn_local_obj_map_ok test end.");
}

#[tokio::test]
async fn ndn_local_obj_map_not_found() {
    init_logging("ndn_local_obj_map_not_found", false);

    info!("ndn_local_obj_map_not_found test start...");
    let _storage_dir = init_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(5, None);
    let _total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map_builder = ObjectMapBuilder::new(HashMethod::Sha256, None, false)
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map_builder
            .put_object(chunk_id.to_string().as_str(), &chunk_id.to_obj_id())
            .expect("put chunk to obj-map failed");
    }

    let obj_map = obj_map_builder
        .build()
        .await
        .expect("build ObjectMap failed");

    let (obj_map_id, obj_map_str) = obj_map.calc_obj_id();

    // let _obj_map_root_hash = obj_map
    //     .get_root_hash_str()
    //     .expect("obj-map root hash should calc finish");

    // delete the chunk list storage file
    let remove_ret = std::fs::remove_file(
        obj_map
            .get_storage_file_path()
            .expect("get obj-map storage file path failed")
            .as_path(),
    );
    assert!(
        remove_ret.is_ok(),
        "remove chunk list storage file failed, remove: {:?}",
        remove_ret
    );

    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_map_id, obj_map_str.as_str())
        .await
        .expect("put obj-map to ndn-mgr failed");
    let obj_map_json = NamedDataMgr::get_object(Some(ndn_mgr_id.as_str()), &obj_map_id, None)
        .await
        .expect("get obj-map from ndn-mgr failed");

    ObjectMap::open(obj_map_json)
        .await
        .map(|_| ())
        .expect_err("open obj-map from obj-map id should failed for the storage is removed");

    info!("ndn_local_obj_map_not_found test end.");
}

#[tokio::test]
async fn ndn_local_obj_map_verify_failed() {
    init_logging("ndn_local_obj_map_verify_failed", false);

    info!("ndn_local_obj_map_verify_failed test start...");
    let _storage_dir = init_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let verifier = ObjectMapProofVerifier::new(HashMethod::Sha256);

    let chunks = generate_random_chunk_list(5, None);
    let _total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map_builder = ObjectMapBuilder::new(HashMethod::Sha256, None, false)
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map_builder
            .put_object(chunk_id.to_string().as_str(), &chunk_id.to_obj_id())
            .expect("put chunk to obj-map failed");
    }

    let obj_map = obj_map_builder
        .build()
        .await
        .expect("build ObjectMap failed");

    let (obj_map_id, obj_map_str) = obj_map.calc_obj_id();
    let obj_map_body: ObjectMapBody =
        serde_json::from_str(&obj_map_str).expect("parse obj-map body from str should success");

    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_map_id, obj_map_str.as_str())
        .await
        .expect("put obj-map to ndn-mgr failed");
    let obj_map_json = NamedDataMgr::get_object(Some(ndn_mgr_id.as_str()), &obj_map_id, None)
        .await
        .expect("get obj-map from ndn-mgr failed");

    let (append_chunk_id, _append_chunk_data) = generate_random_chunk(1024 * 1024);
    let mut append_obj_map_builder = ObjectMapBuilder::from_object_map(&obj_map)
        .await
        .expect("create append ObjectMap failed");

    append_obj_map_builder
        .put_object(
            append_chunk_id.to_string().as_str(),
            &append_chunk_id.to_obj_id(),
        )
        .expect("put append chunk to obj-map failed");
    let append_obj_map = append_obj_map_builder
        .build()
        .await
        .expect("build append ObjectMap failed");

    let (append_obj_map_id, append_obj_map_str) = append_obj_map.calc_obj_id();
    let append_obj_map_body: ObjectMapBody = serde_json::from_str(&append_obj_map_str)
        .expect("parse append obj-map body from str should success");
    // instead the chunk list storage file
    let obj_map_storage_file_path = obj_map
        .get_storage_file_path()
        .expect("get obj-map storage file path failed");
    let append_obj_map_storage_file_path = append_obj_map
        .get_storage_file_path()
        .expect("get append obj-map storage file path failed");
    let remove_ret = std::fs::remove_file(obj_map_storage_file_path.as_path());
    let copy_ret = std::fs::copy(
        append_obj_map_storage_file_path.as_path(),
        obj_map_storage_file_path.as_path(),
    );

    assert!(
        copy_ret.is_ok(),
        "instead append chunk list storage file failed, remove: {:?}, copy: {:?}",
        remove_ret,
        copy_ret
    );

    let fake_obj_map = ObjectMapBuilder::open(obj_map_json.clone())
        .await
        .expect("build chunk list from ndn-mgr should success for object-array has been replaced")
        .build()
        .await
        .expect("build fake ObjectMap failed");

    let (fake_obj_map_id, fake_obj_map_str) = fake_obj_map.calc_obj_id();
    let fake_obj_map_body: ObjectMapBody = serde_json::from_str(&fake_obj_map_str)
        .expect("parse obj-map body from str should success for fake obj-map");

    assert_eq!(
        fake_obj_map_body.root_hash, append_obj_map_body.root_hash,
        "obj-map id check failed after replace"
    );
    assert_eq!(
        fake_obj_map_id, append_obj_map_id,
        "obj-map id check failed after replace"
    );

    for (chunk_id, _chunk_data) in chunks.iter() {
        let key = chunk_id.to_string();
        let obj_id = obj_map
            .get_object(key.as_str())
            .expect("get object from obj-map failed")
            .expect("object should be some");

        let fake_chunk_id = fake_obj_map
            .get_object(key.as_str())
            .expect("get object from fake obj-map failed")
            .expect("object should be some");

        assert_eq!(
            fake_chunk_id, obj_id,
            "obj-map {} object check failed after replace",
            key
        );

        let proof = obj_map
            .get_object_proof_path(key.as_str())
            .await
            .expect("get_object_proof_path should success for chunk_list has been replaced")
            .expect("get_object_proof_path should return error");
        verifier
            .verify(append_obj_map_body.root_hash.as_str(), &proof)
            .expect_err("should failed for chunk_list has been replaced");
        let fake_proof = fake_obj_map
            .get_object_proof_path(key.as_str())
            .await
            .expect("get_object_proof_path should success for chunk_list has been replaced")
            .expect("get_object_proof_path should return object");
        verifier
            .verify(obj_map_body.root_hash.as_str(), &fake_proof)
            .expect_err("should failed for chunk_list has been replaced");
    }

    let mut fake_proof = fake_obj_map
        .get_object_proof_path(append_chunk_id.to_string().as_str())
        .await
        .expect("get_object_proof_path should success for chunk_list has been replaced")
        .expect("get_object_proof_path should return object");
    let is_ok = verifier
        .verify(append_obj_map_body.root_hash.as_str(), &fake_proof)
        .expect("should success for chunk_list has been replaced");
    assert!(is_ok, "should success for item is in fake chunk_list");

    verifier
        .verify(obj_map_body.root_hash.as_str(), &fake_proof)
        .expect_err("should failed for chunk_list has been replaced");

    let root_id_pos = fake_proof.proof.len() - 1;
    fake_proof.proof[root_id_pos].1 = obj_map_id.obj_hash.clone(); // change the last proof item to fake obj_map_id
    let _is_ok = verifier
        .verify(obj_map_body.root_hash.as_str(), &fake_proof)
        .expect_err("should failed for chunk_list has been replaced");
    // assert!(!is_ok, "should failed for item is in fake proof");

    info!("ndn_local_obj_map_verify_failed test end.");
}
