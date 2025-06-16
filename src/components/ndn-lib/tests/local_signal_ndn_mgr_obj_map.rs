use std::{
    collections::{HashMap, HashSet},
    io::SeekFrom,
    ops::{Deref, Index},
    path::PathBuf,
};

use buckyos_kit::*;
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use hex::ToHex;
use jsonwebtoken::EncodingKey;
use log::*;
use ndn_lib::*;
use rand::{Rng, RngCore};
use serde_json::json;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

fn generate_random_bytes(size: u64) -> Vec<u8> {
    let mut rng = rand::rng();
    let mut buffer = vec![0u8; size as usize];
    rng.fill_bytes(&mut buffer);
    buffer
}

fn generate_random_chunk_mix(size: u64) -> (ChunkId, Vec<u8>) {
    let chunk_data = generate_random_bytes(size);
    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&chunk_data);
    let chunk_id = ChunkId::mix_from_hash_result(size, &hash, HashMethod::Sha256);
    info!("chunk_id: {}", chunk_id.to_string());
    (chunk_id, chunk_data)
}

fn generate_random_chunk(size: u64) -> (ChunkId, Vec<u8>) {
    let chunk_data = generate_random_bytes(size);
    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&chunk_data);
    let chunk_id = ChunkId::from_hash_result(&hash, HashMethod::Sha256);
    info!("chunk_id: {}", chunk_id.to_string());
    (chunk_id, chunk_data)
}

fn generate_random_chunk_list(count: usize, fix_size: Option<u64>) -> Vec<(ChunkId, Vec<u8>)> {
    let mut chunk_list = Vec::with_capacity(count);
    for _ in 0..count {
        let (chunk_id, chunk_data) = if let Some(size) = fix_size {
            generate_random_chunk_mix(size)
        } else {
            generate_random_chunk_mix(rand::rng().random_range(1024u64..1024 * 1024 * 10))
        };
        chunk_list.push((chunk_id, chunk_data));
    }
    chunk_list
}

async fn write_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId, chunk_data: &[u8]) {
    let (mut chunk_writer, _progress_info) =
        NamedDataMgr::open_chunk_writer(Some(ndn_mgr_id), chunk_id, chunk_data.len() as u64, 0)
            .await
            .expect("open chunk writer failed");
    chunk_writer
        .write_all(chunk_data)
        .await
        .expect("write chunk to ndn-mgr failed");
    NamedDataMgr::complete_chunk_writer(Some(ndn_mgr_id), chunk_id)
        .await
        .expect("wait chunk writer complete failed.");
}

async fn read_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId) -> Vec<u8> {
    let (mut chunk_reader, len) =
        NamedDataMgr::open_chunk_reader(Some(ndn_mgr_id), chunk_id, SeekFrom::Start(0), false)
            .await
            .expect("open reader from ndn-mgr failed.");

    let mut buffer = vec![0u8; len as usize];
    chunk_reader
        .read_exact(&mut buffer)
        .await
        .expect("read chunk from ndn-mgr failed");

    buffer
}

type NdnServerHost = String;

async fn init_ndn_server(ndn_mgr_id: &str) -> (NdnClient, NdnServerHost) {
    let mut rng = rand::rng();
    let tls_port = rng.random_range(10000u16..20000u16);
    let http_port = rng.random_range(10000u16..20000u16);
    let test_server_config = json!({
        "tls_port": tls_port,
        "http_port": http_port,
        "hosts": {
            "*": {
                "enable_cors": true,
                "routes": {
                    "/ndn/": {
                        "named_mgr": {
                            "named_data_mgr_id": ndn_mgr_id,
                            "read_only": false,
                            "guest_access": true,
                            "is_chunk_id_in_path": true,
                            "enable_mgr_file_path": true
                        }
                    }
                }
            }
        }
    });

    let test_server_config: WarpServerConfig = serde_json::from_value(test_server_config).unwrap();

    tokio::spawn(async move {
        info!("start test ndn server(powered by cyfs-warp)...");
        start_cyfs_warp_server(test_server_config)
            .await
            .expect("start cyfs warp server failed.");
    });
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let temp_dir = tempfile::tempdir()
        .unwrap()
        .path()
        .join("ndn-test")
        .join(ndn_mgr_id);

    fs::create_dir_all(temp_dir.as_path())
        .await
        .expect("create temp dir failed.");

    let config = NamedDataMgrConfig {
        local_stores: vec![temp_dir.to_str().unwrap().to_string()],
        local_cache: None,
        mmap_cache_dir: None,
    };

    let named_mgr =
        NamedDataMgr::from_config(Some(ndn_mgr_id.to_string()), temp_dir.to_path_buf(), config)
            .await
            .expect("init NamedDataMgr failed.");

    NamedDataMgr::set_mgr_by_id(Some(ndn_mgr_id), named_mgr)
        .await
        .expect("set named data manager by id failed.");

    let host = format!("localhost:{}", http_port);
    let client = NdnClient::new(
        format!("http://{}/ndn/", host),
        None,
        Some(ndn_mgr_id.to_string()),
    );

    (client, host)
}

async fn init_obj_map_storage_factory() -> PathBuf {
    let data_path = std::env::temp_dir().join("test_ndn_obj_map_data");
    if GLOBAL_OBJECT_MAP_STORAGE_FACTORY.get().is_some() {
        info!("Object map storage factory already initialized");
        return data_path;
    }
    if !data_path.exists() {
        fs::create_dir_all(&data_path)
            .await
            .expect("create data path failed");
    }

    GLOBAL_OBJECT_MAP_STORAGE_FACTORY
        .set(ObjectMapStorageFactory::new(
            &data_path,
            Some(ObjectMapStorageType::JSONFile),
        ))
        .map_err(|_| ())
        .expect("Object array storage factory already initialized");
    data_path
}

#[tokio::test]
async fn ndn_local_obj_map_basic() {
    init_logging("ndn_local_obj_map_basic", false);

    info!("ndn_local_obj_map_basic test start...");
    init_obj_map_storage_factory().await;

    let mut rng = rand::rng();
    let chunk_fix_size: u64 = 1024 * 1024 + 513; // 1MB + x bytes

    let chunk_size1: u64 = chunk_fix_size;
    let (chunk_id1, chunk_data1) = generate_random_chunk_mix(chunk_fix_size);

    let chunk_size2: u64 = chunk_fix_size;
    let (chunk_id2, chunk_data2) = generate_random_chunk_mix(chunk_fix_size);

    let chunk_size3: u64 = chunk_fix_size;
    let (chunk_id3, chunk_data3) = generate_random_chunk_mix(chunk_fix_size);

    let chunk_size4: u64 = chunk_fix_size;
    let (chunk_id4, chunk_data4) = generate_random_chunk_mix(chunk_fix_size);

    let chunk_size5: u64 = chunk_fix_size;
    let (chunk_id5, chunk_data5) = generate_random_chunk_mix(chunk_fix_size);

    let chunks = HashMap::from([
        ("chunk1", (chunk_id1, chunk_data1)),
        ("chunk2", (chunk_id2, chunk_data2)),
        ("chunk3", (chunk_id3, chunk_data3)),
        ("chunk4", (chunk_id4, chunk_data4)),
        ("chunk5", (chunk_id5, chunk_data5)),
    ]);

    let mut obj_map = ObjectMap::new(HashMethod::Sha256, Some(ObjectMapStorageType::JSONFile))
        .await
        .expect("create ObjectMap failed");

    for (name, (chunk_id, chunk_data)) in chunks.iter() {
        obj_map
            .put_object(*name, chunk_id.to_obj_id())
            .await
            .expect(&format!("put {} to obj-map failed", name));
    }

    obj_map.flush().await.expect("flush obj-map failed");

    assert_eq!(
        obj_map.len().await.expect("get obj-map len failed"),
        5,
        "obj-map total size check failed"
    );

    let verifier = ObjectMapProofVerifier::new(HashMethod::Sha256);
    for (name, (chunk_id, chunk_data)) in chunks.iter() {
        let obj_id = obj_map
            .get_object(*name)
            .await
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
            .verify(
                &obj_map
                    .get_obj_id()
                    .expect("obj-map id should be calc finish"),
                &proof,
            )
            .expect("verify object map failed");
        assert!(is_ok, "verify object map failed for object {}", name);
    }

    let notexist_obj_id = obj_map
        .get_object("notexist")
        .await
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
    let is_ok = verifier
        .verify(
            &obj_map.get_obj_id().expect("obj-map id should calc finish"),
            &fake_obj_proof,
        )
        .expect("verify chunk list should success for exclude object");
    assert!(!is_ok, "verify chunk list should fail for fake object");

    let mut fake_key_proof = proof.clone();
    fake_obj_proof.item.key = "fake-key".to_string();
    let is_ok = verifier
        .verify(
            &obj_map.get_obj_id().expect("obj-map id should calc finish"),
            &fake_obj_proof,
        )
        .expect("verify chunk list should success for exclude object");
    assert!(!is_ok, "verify chunk list should fail for fake key");

    let mut fake_key_obj_proof = proof.clone();
    fake_key_obj_proof.item.key = "fake-key".to_string();
    fake_key_obj_proof.item.obj_id = new_chunk.0.to_obj_id(); // change the object id to new chunk id
    let is_ok = verifier
        .verify(
            &obj_map.get_obj_id().expect("obj-map id should calc finish"),
            &fake_key_obj_proof,
        )
        .expect("verify chunk list should success for exclude object");
    assert!(!is_ok, "verify chunk list should fail for fake key and obj");

    info!("ndn_local_obj_map_basic test end.");
}

#[tokio::test]
async fn ndn_local_obj_map_ok() {
    init_logging("ndn_local_obj_map_ok", false);

    info!("ndn_local_obj_map_ok test start...");
    init_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(5, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map = ObjectMap::new(HashMethod::Sha256, Some(ObjectMapStorageType::JSONFile))
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map
            .put_object(chunk_id.to_string().as_str(), chunk_id.to_obj_id())
            .await
            .expect("put chunk to obj-map failed");
    }

    obj_map.save().await.expect("save obj-map failed");

    let obj_map_id = obj_map
        .get_obj_id()
        .expect("obj-map id should be calc finish");

    let mut got_obj_map = ObjectMap::open(&obj_map_id, true, Some(ObjectMapStorageType::JSONFile))
        .await
        .expect("open obj-map from obj-map id failed");

    let got_obj_map_id = got_obj_map
        .calc_obj_id()
        .await
        .expect("calc obj-map id failed for got obj-map");
    assert_eq!(got_obj_map_id, obj_map_id, "obj-map id check failed");
    assert_eq!(
        got_obj_map.len().await.expect("get obj-map len failed"),
        chunks.len(),
        "obj-map total size check failed"
    );

    for (key, obj_id, _x) in got_obj_map.iter() {
        assert_eq!(
            got_obj_map
                .get_object(key.as_str())
                .await
                .expect("get item failed")
                .expect("object should be some"),
            obj_map
                .get_object(key.as_str())
                .await
                .expect("get item from obj-map failed")
                .expect("object should be some"),
            "item {} object check failed",
            key
        );

        assert_eq!(
            obj_id,
            obj_map
                .get_object(key.as_str())
                .await
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

        let (chunk_id, chunk_data) = chunks
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
        assert_eq!(&buffer, chunk_data, "chunk_data content check failed");
    }

    info!("ndn_local_obj_map_ok test end.");
}

#[tokio::test]
async fn ndn_local_obj_map_not_found() {
    init_logging("ndn_local_obj_map_not_found", false);

    info!("ndn_local_obj_map_not_found test start...");
    let storage_dir = init_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(5, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map = ObjectMap::new(HashMethod::Sha256, Some(ObjectMapStorageType::JSONFile))
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map
            .put_object(chunk_id.to_string().as_str(), chunk_id.to_obj_id())
            .await
            .expect("put chunk to obj-map failed");
    }

    obj_map.save().await.expect("save obj-map failed");

    let obj_map_id = obj_map
        .get_obj_id()
        .expect("obj-map id should be calc finish");

    // delete the chunk list storage file
    let remove_json_ret = std::fs::remove_file(storage_dir.join(obj_map_id.to_base32() + ".json"));
    let remove_arrow_ret =
        std::fs::remove_file(storage_dir.join(obj_map_id.to_base32() + ".arrow"));

    assert!(
        remove_json_ret.is_ok() || remove_arrow_ret.is_ok(),
        "remove chunk list storage file failed"
    );

    ObjectMap::open(&obj_map_id, true, Some(ObjectMapStorageType::JSONFile))
        .await
        .map(|_| ())
        .expect_err("open obj-map from obj-map id should failed for the storage is removed");

    info!("ndn_local_obj_map_not_found test end.");
}

#[tokio::test]
async fn ndn_local_obj_map_verify_failed() {
    init_logging("ndn_local_obj_map_verify_failed", false);

    info!("ndn_local_obj_map_verify_failed test start...");
    let storage_dir = init_obj_map_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let verifier = ObjectMapProofVerifier::new(HashMethod::Sha256);

    let chunks = generate_random_chunk_list(5, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut obj_map = ObjectMap::new(HashMethod::Sha256, Some(ObjectMapStorageType::JSONFile))
        .await
        .expect("create ObjectMap failed");

    for (chunk_id, chunk_data) in chunks.iter() {
        write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        obj_map
            .put_object(chunk_id.to_string().as_str(), chunk_id.to_obj_id())
            .await
            .expect("put chunk to obj-map failed");
    }

    obj_map.save().await.expect("save obj-map failed");

    let obj_map_id = obj_map
        .get_obj_id()
        .expect("obj-map id should be calc finish");

    let (append_chunk_id, append_chunk_data) = generate_random_chunk(1024 * 1024);
    let mut append_obj_map = obj_map.clone(false).await.expect("clone obj-map failed");

    append_obj_map
        .put_object(
            append_chunk_id.to_string().as_str(),
            append_chunk_id.to_obj_id(),
        )
        .await
        .expect("put append chunk to obj-map failed");
    append_obj_map
        .save()
        .await
        .expect("save append obj-map failed");
    let append_obj_map_id = append_obj_map
        .get_obj_id()
        .expect("append obj-map id should be calc finish");
    // instead the chunk list storage file
    let remove_json_ret = std::fs::remove_file(storage_dir.join(obj_map_id.to_base32() + ".json"));
    let copy_json_ret = std::fs::copy(
        storage_dir.join(append_obj_map_id.to_base32() + ".json"),
        storage_dir.join(obj_map_id.to_base32() + ".json"),
    );
    let remove_arrow_ret =
        std::fs::remove_file(storage_dir.join(obj_map_id.to_base32() + ".arrow"));
    let copy_arrow_ret = std::fs::copy(
        storage_dir.join(append_obj_map_id.to_base32() + ".arrow"),
        storage_dir.join(obj_map_id.to_base32() + ".arrow"),
    );

    assert!(
        copy_json_ret.is_ok()
            || copy_arrow_ret.is_ok(),
        "instead append chunk list storage file failed, remove-json: {:?}, copy-json: {:?}, remove-arrow: {:?}, copy-arrow: {:?}", remove_json_ret, copy_json_ret, remove_arrow_ret, copy_arrow_ret
    );

    let mut fake_obj_map = ObjectMap::open(&obj_map_id, true, Some(ObjectMapStorageType::JSONFile))
        .await
        .expect("build chunk list from ndn-mgr should success for object-array has been replaced");
    assert_eq!(
        fake_obj_map.calc_obj_id().await.unwrap(),
        append_obj_map_id,
        "obj-map id check failed after replace"
    );

    for (chunk_id, _chunk_data) in chunks.iter() {
        let key = chunk_id.to_string();
        let obj_id = obj_map
            .get_object(key.as_str())
            .await
            .expect("get object from obj-map failed")
            .expect("object should be some");

        let fake_chunk_id = fake_obj_map
            .get_object(key.as_str())
            .await
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
            .verify(&append_obj_map_id, &proof)
            .expect_err("should failed for chunk_list has been replaced");
        let fake_proof = fake_obj_map
            .get_object_proof_path(key.as_str())
            .await
            .expect("get_object_proof_path should success for chunk_list has been replaced")
            .expect("get_object_proof_path should return object");
        verifier
            .verify(&obj_map_id, &fake_proof)
            .expect_err("should failed for chunk_list has been replaced");
    }

    let fake_proof = fake_obj_map
        .get_object_proof_path(append_chunk_id.to_string().as_str())
        .await
        .expect("get_object_proof_path should success for chunk_list has been replaced")
        .expect("get_object_proof_path should return object");
    let is_ok = verifier
        .verify(&append_obj_map_id, &fake_proof)
        .expect("should success for chunk_list has been replaced");
    assert!(is_ok, "should success for item is in fake chunk_list");

    verifier
        .verify(&obj_map_id, &fake_proof)
        .expect_err("should failed for chunk_list has been replaced");

    info!("ndn_local_obj_map_verify_failed test end.");
}
