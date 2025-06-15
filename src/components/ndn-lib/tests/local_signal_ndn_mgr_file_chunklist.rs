use std::{
    collections::HashSet,
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
            generate_random_chunk(rand::rng().random_range(1024u64..1024 * 1024 * 10))
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

async fn init_obj_array_storage_factory() -> PathBuf {
    let data_path = std::env::temp_dir().join("test_ndn_chunklist_data");
    if GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.get().is_some() {
        info!("Object array storage factory already initialized");
        return data_path;
    }
    if !data_path.exists() {
        fs::create_dir_all(&data_path)
            .await
            .expect("create data path failed");
    }

    GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY
        .set(ObjectArrayStorageFactory::new(&data_path))
        .map_err(|_| ())
        .expect("Object array storage factory already initialized");
    data_path
}

#[tokio::test]
async fn ndn_local_chunklist_basic_fix_len() {
    init_logging("ndn_local_chunklist_basic_fix_len", false);

    info!("ndn_local_chunklist_basic_fix_len test start...");
    init_obj_array_storage_factory().await;

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

    let mut fix_mix_chunk_list_builder = ChunkListBuilder::new(HashMethod::Sha256, None)
        .with_total_size(chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5)
        .with_fixed_size(chunk_fix_size);

    // [1]
    fix_mix_chunk_list_builder
        .append(chunk_id1.clone())
        .expect("append chunk_id1 to chunk_arr failed");
    // [1, 2]
    fix_mix_chunk_list_builder
        .append(chunk_id2.clone())
        .expect("append chunk_id1 to chunk_arr failed");
    // [1, 3, 2]
    fix_mix_chunk_list_builder
        .insert(1, chunk_id3.clone())
        .expect("insert chunk_id3 to chunk_arr failed");
    // [4, 1, 3, 2]
    fix_mix_chunk_list_builder
        .insert(0, chunk_id4.clone())
        .expect("insert chunk_id4 to chunk_arr failed");
    // [4, 1, 3, 2, 5]
    fix_mix_chunk_list_builder
        .insert(4, chunk_id5.clone())
        .expect("insert chunk_id4 to chunk_arr failed");
    fix_mix_chunk_list_builder
        .insert(6, chunk_id5.clone())
        .expect_err("insert pos 6 to chunk_arr should fail for out of range");

    let mut fix_mix_chunk_list = fix_mix_chunk_list_builder
        .build()
        .await
        .expect("build chunk list failed");

    assert_eq!(
        fix_mix_chunk_list.get_total_size(),
        chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5,
        "chunk_list total size check failed"
    );
    assert!(
        fix_mix_chunk_list.is_fixed_size_chunk_list(),
        "chunk_list fix size check failed"
    );
    assert_eq!(
        fix_mix_chunk_list.get_len(),
        5,
        "chunk_list length check failed"
    );

    assert_eq!(
        fix_mix_chunk_list
            .get_chunk(0)
            .expect("get chunk 0 failed")
            .expect("chunk_list first object check failed"),
        chunk_id4,
        "chunk_list first object check failed"
    );

    assert_eq!(
        fix_mix_chunk_list
            .get_chunk(1)
            .expect("get chunk 1 failed")
            .expect("chunk_list second object check failed"),
        chunk_id1,
        "chunk_list second object check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk(2)
            .expect("get chunk 2 failed")
            .expect("chunk_list third object check failed"),
        chunk_id3,
        "chunk_list third object check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk(3)
            .expect("get chunk 3 failed")
            .expect("chunk_list fourth object check failed"),
        chunk_id2,
        "chunk_list fourth object check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk(4)
            .expect("get chunk 4 failed")
            .expect("chunk_list fifth object check failed"),
        chunk_id5,
        "chunk_list fifth object check failed"
    );
    assert!(
        fix_mix_chunk_list
            .get_chunk(5)
            .expect("should Ok(None) for larger index")
            .is_none(),
        "chunk_list sixth object check failed"
    );
    assert_eq!(fix_mix_chunk_list.get_meta().fix_size, Some(chunk_fix_size));

    // from start
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(0))
            .expect("get chunk index by offset 0 failed"),
        (0, 0),
        "chunk_list first object index check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_offset_by_index(0)
            .expect("get chunk offset by index 0 failed"),
        0,
        "chunk_list first object offset check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(chunk_fix_size * 3))
            .expect("get chunk index by offset failed"),
        (3, 0),
        "chunk_list 3 object index check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_offset_by_index(3)
            .expect("get chunk offset by index 3 failed"),
        chunk_fix_size * 3,
        "chunk_list 3 object offset check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(chunk_fix_size * 4))
            .expect("get chunk index by offset failed"),
        (4, 0),
        "chunk_list 4 object index check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(chunk_fix_size * 5 - 1))
            .expect("get chunk index by offset failed"),
        (4, chunk_fix_size - 1),
        "chunk_list 4 object index check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_offset_by_index(4)
            .expect("get chunk offset by index 4 failed"),
        chunk_fix_size * 4,
        "chunk_list 4 object offset check failed"
    );
    fix_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(chunk_fix_size * 5))
        .expect_err("should fail for out of range");
    fix_mix_chunk_list
        .get_chunk_offset_by_index(5)
        .expect_err("should fail for out of range");
    fix_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(chunk_fix_size * 6))
        .expect_err("should fail for out of range");
    fix_mix_chunk_list
        .get_chunk_offset_by_index(6)
        .expect_err("should fail for out of range");

    let chunk_offset = rng.random_range(1..chunk_fix_size - 1);
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(chunk_fix_size + chunk_offset))
            .expect("get chunk index by offset failed"),
        (1, chunk_offset),
        "chunk_list 1.x object index check failed"
    );

    // from end
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-1))
            .expect("get chunk index by offset 0 failed"),
        (4, chunk_fix_size - 1),
        "chunk_list first object index check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-(chunk_fix_size as i64) * 3 - 1))
            .expect("get chunk index by offset failed"),
        (1, chunk_fix_size - 1),
        "chunk_list 3 object index check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-(chunk_fix_size as i64) * 4 - 1))
            .expect("get chunk index by offset failed"),
        (0, chunk_fix_size - 1),
        "chunk_list 4 object index check failed"
    );
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-(chunk_fix_size as i64) * 5))
            .expect("get chunk index by offset failed"),
        (0, 0),
        "chunk_list 0 object index check failed"
    );
    fix_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(-(chunk_fix_size as i64) * 5 - 1))
        .expect_err("should fail for out of range");
    fix_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(-(chunk_fix_size as i64) * 6))
        .expect_err("should fail for out of range");
    fix_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(1))
        .expect_err("should fail for out of range");

    let chunk_offset = rng.random_range(2..(chunk_fix_size as i64) - 1);
    assert_eq!(
        fix_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-(chunk_fix_size as i64) - chunk_offset))
            .expect("get chunk index by offset failed"),
        (3, (chunk_fix_size - chunk_offset as u64)),
        "chunk_list 3.x object index check failed"
    );

    fix_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Current(500))
        .expect_err("get chunk index by offset 500 should fail for not supported");

    // verify
    let chunk_array = fix_mix_chunk_list.deref();
    let chunk_array_id = chunk_array
        .get_obj_id()
        .expect("id for obj-array of chunklist should calc complete");
    let verifier = ObjectArrayProofVerifier::new(HashMethod::Sha256);
    for idx in 0..chunk_array.len() {
        // TODO: why mut needed?
        let item = fix_mix_chunk_list
            .get_object_with_proof(idx)
            .await
            .expect("get object with proof failed")
            .expect("object with proof should be some");
        assert_eq!(
            item.obj_id,
            fix_mix_chunk_list
                .get_chunk(idx)
                .expect("chunk_list object id check failed")
                .expect("chunk_list object should be some")
                .to_obj_id(),
            "chunk_list {} object id check failed",
            idx
        );
        verifier
            .verify(&chunk_array_id, &item.obj_id, &item.proof)
            .expect("verify chunk list failed");
    }

    let ret = fix_mix_chunk_list.get_object_with_proof(5).await;
    assert!(
        ret.is_err() || ret.unwrap().is_none(),
        "get object with proof for out of range should fail"
    );
    let ret = fix_mix_chunk_list.get_object_with_proof(100).await;
    assert!(
        ret.is_err() || ret.unwrap().is_none(),
        "get object with proof for out of range should fail"
    );

    let check_batch =
        async |chunk_list: &mut ChunkList, batch: &[usize], larger_index: &[usize]| -> () {
            info!("check batch: {:?}", batch);
            let larger_index = HashSet::<usize>::from_iter(larger_index.iter().cloned());
            let obj_item_vec = chunk_list
                .batch_get_object_with_proof(batch)
                .await
                .expect("batch get object with proof failed");
            assert_eq!(
                obj_item_vec.len(),
                batch.len(),
                "batch get object with proof length check failed for out of range"
            );

            for (idx, chunk_pos) in batch.iter().enumerate() {
                let item = obj_item_vec
                    .get(idx)
                    .expect("batch get object with proof item should be some");
                if larger_index.contains(&idx) {
                    assert!(
                        item.is_none(),
                        "batch get object with proof item {} should be none",
                        idx
                    );
                } else {
                    assert!(
                        item.is_some(),
                        "batch get object with proof item {} should be some",
                        idx
                    );
                    let item = item.as_ref().expect("item should be some");
                    assert_eq!(
                        item.obj_id,
                        chunk_list
                            .deref()
                            .get_object(*chunk_pos)
                            .expect("chunk_list object id check failed")
                            .expect("chunk_list object should be some"),
                        "chunk_list {} object id check failed for out of range",
                        chunk_pos
                    );
                    verifier
                        .verify(&chunk_array_id, &item.obj_id, &item.proof)
                        .expect("verify chunk list failed");
                }
            }
        };

    check_batch(&mut fix_mix_chunk_list, &[0], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[2], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[4], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[0, 1, 2, 3, 4], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[0, 1, 2], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[2, 3, 4], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[0, 2], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[2, 4], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[2, 3], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[1, 3], &[]).await;
    check_batch(&mut fix_mix_chunk_list, &[3, 0, 2, 4, 1], &[]).await; // random order
    check_batch(&mut fix_mix_chunk_list, &[3, 0, 3, 4, 1], &[]).await; // repeat

    // small large
    check_batch(&mut fix_mix_chunk_list, &[5], &[0]).await;
    check_batch(&mut fix_mix_chunk_list, &[0, 5], &[1]).await;
    check_batch(&mut fix_mix_chunk_list, &[2, 5], &[1]).await;
    check_batch(&mut fix_mix_chunk_list, &[4, 5], &[1]).await;
    check_batch(&mut fix_mix_chunk_list, &[5, 0], &[0]).await;
    check_batch(&mut fix_mix_chunk_list, &[5, 2], &[0]).await;
    check_batch(&mut fix_mix_chunk_list, &[5, 4], &[0]).await;
    check_batch(&mut fix_mix_chunk_list, &[0, 1, 5], &[2]).await;
    check_batch(&mut fix_mix_chunk_list, &[3, 1, 5], &[2]).await;

    // more large
    check_batch(&mut fix_mix_chunk_list, &[100], &[0]).await;
    check_batch(&mut fix_mix_chunk_list, &[0, 100], &[1]).await;
    check_batch(&mut fix_mix_chunk_list, &[2, 100], &[1]).await;
    check_batch(&mut fix_mix_chunk_list, &[4, 100], &[1]).await;
    check_batch(&mut fix_mix_chunk_list, &[100, 0], &[0]).await;
    check_batch(&mut fix_mix_chunk_list, &[100, 2], &[0]).await;
    check_batch(&mut fix_mix_chunk_list, &[100, 4], &[0]).await;
    check_batch(&mut fix_mix_chunk_list, &[0, 1, 100], &[2]).await;
    check_batch(&mut fix_mix_chunk_list, &[3, 1, 100], &[2]).await;

    let check_range = async |chunk_list: &mut ChunkList, start_pos: usize, end_pos: usize| -> () {
        info!("check range: {:?}, {:?}", start_pos, end_pos);
        let obj_item_vec = chunk_list
            .range_get_object_with_proof(start_pos, end_pos)
            .await
            .expect("batch get object with proof failed");
        if end_pos > chunk_list.get_len() {
            assert!(
                obj_item_vec.is_empty()
                    || obj_item_vec.len() == end_pos - start_pos
                    || obj_item_vec.len() == chunk_list.get_len() - start_pos,
                "batch get object with proof should be empty for out of range"
            );
            return;
        } else {
            assert_eq!(
                obj_item_vec.len(),
                end_pos - start_pos,
                "batch get object with proof length check failed for out of range"
            );
        }

        for (idx, chunk_pos) in (start_pos..end_pos).into_iter().enumerate() {
            if chunk_pos >= chunk_list.get_len() {
                let item = obj_item_vec.get(idx);
                assert!(
                    item.is_none() || item.as_ref().unwrap().is_none(),
                    "batch get object with proof item {} should be none",
                    idx
                );
            } else {
                let item = obj_item_vec
                    .get(idx)
                    .expect("batch get object with proof item should be some");
                assert!(
                    item.is_some(),
                    "batch get object with proof item {} should be some",
                    idx
                );
                let item = item.as_ref().expect("item should be some");
                assert_eq!(
                    item.obj_id,
                    chunk_list
                        .deref()
                        .get_object(chunk_pos)
                        .expect("chunk_list object id check failed")
                        .expect("chunk_list object should be some"),
                    "chunk_list {} object id check failed",
                    chunk_pos
                );
                verifier
                    .verify(&chunk_array_id, &item.obj_id, &item.proof)
                    .expect("verify chunk list failed");
            }
        }
    };

    check_range(&mut fix_mix_chunk_list, 0, 1).await;
    check_range(&mut fix_mix_chunk_list, 2, 3).await;
    check_range(&mut fix_mix_chunk_list, 4, 5).await;
    check_range(&mut fix_mix_chunk_list, 0, 5).await;
    check_range(&mut fix_mix_chunk_list, 0, 3).await;
    check_range(&mut fix_mix_chunk_list, 2, 5).await;
    check_range(&mut fix_mix_chunk_list, 1, 4).await;

    // small large
    check_range(&mut fix_mix_chunk_list, 0, 6).await;
    check_range(&mut fix_mix_chunk_list, 2, 6).await;
    check_range(&mut fix_mix_chunk_list, 4, 6).await;
    check_range(&mut fix_mix_chunk_list, 5, 6).await;

    // more large
    check_range(&mut fix_mix_chunk_list, 0, 100).await;
    check_range(&mut fix_mix_chunk_list, 2, 100).await;
    check_range(&mut fix_mix_chunk_list, 4, 100).await;
    check_range(&mut fix_mix_chunk_list, 5, 100).await;
    check_range(&mut fix_mix_chunk_list, 100, 150).await;

    let obj_item_vec_ret = fix_mix_chunk_list.range_get_object_with_proof(3, 2).await;
    assert!(
        obj_item_vec_ret.is_err()
            || obj_item_vec_ret.as_ref().unwrap().is_empty()
            || obj_item_vec_ret
                .unwrap()
                .iter()
                .filter(|item| item.is_some())
                .count()
                == 0,
        "should empty"
    );

    let obj_item_vec_ret = fix_mix_chunk_list.range_get_object_with_proof(6, 2).await;
    assert!(
        obj_item_vec_ret.is_err()
            || obj_item_vec_ret.as_ref().unwrap().is_empty()
            || obj_item_vec_ret
                .unwrap()
                .iter()
                .filter(|item| item.is_some())
                .count()
                == 0,
        "should empty"
    );

    let obj_item_vec_ret = fix_mix_chunk_list.range_get_object_with_proof(6, 5).await;
    assert!(
        obj_item_vec_ret.is_err()
            || obj_item_vec_ret.as_ref().unwrap().is_empty()
            || obj_item_vec_ret
                .unwrap()
                .iter()
                .filter(|item| item.is_some())
                .count()
                == 0,
        "should empty"
    );
    info!("ndn_local_chunklist_basic_fix_len test end.");
}

#[tokio::test]
async fn ndn_local_chunklist_basic_var_len() {
    init_logging("ndn_local_chunklist_basic_fix_len", false);

    info!("ndn_local_chunklist_basic_fix_len test start...");
    init_obj_array_storage_factory().await;

    let mut rng = rand::rng();

    let chunk_size1: u64 = 1024 * 1024 + 513;
    let (chunk_id1, chunk_data1) = generate_random_chunk_mix(chunk_size1);

    let chunk_size2: u64 = 1024 * 1024 * 3 + 5;
    let (chunk_id2, chunk_data2) = generate_random_chunk_mix(chunk_size2);

    let chunk_size3: u64 = 1024 + 13;
    let (chunk_id3, chunk_data3) = generate_random_chunk_mix(chunk_size3);

    let chunk_size4: u64 = 1024 * 2 + 113;
    let (chunk_id4, chunk_data4) = generate_random_chunk_mix(chunk_size4);

    let chunk_size5: u64 = 1024 * 1024 * 2 + 53;
    let (chunk_id5, chunk_data5) = generate_random_chunk_mix(chunk_size5);

    let mut var_mix_chunk_list_builder = ChunkListBuilder::new(HashMethod::Sha256, None)
        .with_total_size(chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5)
        .with_var_size();

    // [1]
    var_mix_chunk_list_builder
        .append(chunk_id1.clone())
        .expect("append chunk_id1 to chunk_arr failed");
    // [1, 2]
    var_mix_chunk_list_builder
        .append(chunk_id2.clone())
        .expect("append chunk_id1 to chunk_arr failed");
    // [1, 2, 3]
    var_mix_chunk_list_builder
        .append(chunk_id3.clone())
        .expect("insert chunk_id3 to chunk_arr failed");
    // [1, 2, 3, 4]
    var_mix_chunk_list_builder
        .append(chunk_id4.clone())
        .expect("insert chunk_id4 to chunk_arr failed");
    // [1, 2, 3, 4, 5]
    var_mix_chunk_list_builder
        .append(chunk_id5.clone())
        .expect("insert chunk_id4 to chunk_arr failed");

    let var_mix_chunk_list = var_mix_chunk_list_builder
        .build()
        .await
        .expect("build chunk list failed");

    assert_eq!(
        var_mix_chunk_list.get_total_size(),
        chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5,
        "var_mix_chunk_list total size check failed"
    );
    assert!(
        !var_mix_chunk_list.is_fixed_size_chunk_list(),
        "chunk_list fix size check failed"
    );
    assert_eq!(
        var_mix_chunk_list.get_len(),
        5,
        "chunk_list length check failed"
    );

    assert_eq!(
        var_mix_chunk_list
            .get_chunk(0)
            .expect("get chunk 0 failed")
            .expect("chunk_list first object check failed"),
        chunk_id1,
        "chunk_list first object check failed"
    );

    assert_eq!(
        var_mix_chunk_list
            .get_chunk(1)
            .expect("get chunk 1 failed")
            .expect("chunk_list second object check failed"),
        chunk_id2,
        "chunk_list second object check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk(2)
            .expect("get chunk 2 failed")
            .expect("chunk_list third object check failed"),
        chunk_id3,
        "chunk_list third object check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk(3)
            .expect("get chunk 3 failed")
            .expect("chunk_list fourth object check failed"),
        chunk_id4,
        "chunk_list fourth object check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk(4)
            .expect("get chunk 4 failed")
            .expect("chunk_list fifth object check failed"),
        chunk_id5,
        "chunk_list fifth object check failed"
    );
    assert!(
        var_mix_chunk_list
            .get_chunk(5)
            .expect("should Ok(None) for larger index")
            .is_none(),
        "chunk_list sixth object check failed"
    );
    assert_eq!(var_mix_chunk_list.get_meta().fix_size, None);

    // from start
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(0))
            .expect("get chunk index by offset 0 failed"),
        (0, 0),
        "chunk_list first object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_offset_by_index(0)
            .expect("get chunk offset by index 0 failed"),
        0,
        "chunk_list first object offset check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(chunk_size1 + chunk_size2 + chunk_size3))
            .expect("get chunk index by offset failed"),
        (3, 0),
        "chunk_list 3 object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_offset_by_index(3)
            .expect("get chunk offset by index 3 failed"),
        chunk_size1 + chunk_size2 + chunk_size3,
        "chunk_list 3 object offset check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(
                chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4
            ))
            .expect("get chunk index by offset failed"),
        (4, 0),
        "chunk_list 4 object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(
                chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5 - 1
            ))
            .expect("get chunk index by offset failed"),
        (4, chunk_size5 - 1),
        "chunk_list 4 object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_offset_by_index(4)
            .expect("get chunk offset by index 4 failed"),
        chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4,
        "chunk_list 4 object offset check failed"
    );
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(
            chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5,
        ))
        .expect_err("should fail for out of range");
    var_mix_chunk_list
        .get_chunk_offset_by_index(5)
        .expect_err("should fail for out of range");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(
            chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5 + chunk_size5,
        ))
        .expect_err("should fail for out of range");
    var_mix_chunk_list
        .get_chunk_offset_by_index(6)
        .expect_err("should fail for out of range");

    let chunk_offset = rng.random_range(1..chunk_size2 - 1);
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(chunk_size1 + chunk_offset))
            .expect("get chunk index by offset failed"),
        (1, chunk_offset),
        "chunk_list 1.x object index check failed"
    );

    // from end
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-1))
            .expect("get chunk index by offset 0 failed"),
        (4, chunk_size5 - 1),
        "chunk_list first object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(
                -((chunk_size4 + chunk_size5 + chunk_size3) as i64) - 1
            ))
            .expect("get chunk index by offset failed"),
        (1, chunk_size2 - 1),
        "chunk_list 1 object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(
                -((chunk_size3 + chunk_size4 + chunk_size5 + chunk_size2) as i64) - 1
            ))
            .expect("get chunk index by offset failed"),
        (0, chunk_size1 - 1),
        "chunk_list 0 object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(
                -((chunk_size1
                    + chunk_size2
                    + chunk_size3
                    + chunk_size4
                    + chunk_size5
                    + chunk_size5) as i64)
            ))
            .expect("get chunk index by offset failed"),
        (0, 0),
        "chunk_list 0 object index check failed"
    );
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(
            -((chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5 + chunk_size5)
                as i64)
                - 1,
        ))
        .expect_err("should fail for out of range");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(
            -((chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5 + chunk_size5)
                as i64)
                * 6,
        ))
        .expect_err("should fail for out of range");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(1))
        .expect_err("should fail for out of range");

    let chunk_offset = rng.random_range(2..(chunk_size5 as i64) - 1);
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-(chunk_size5 as i64) - chunk_offset))
            .expect("get chunk index by offset failed"),
        (3, (chunk_size4 - chunk_offset as u64)),
        "chunk_list 3.x object index check failed"
    );

    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Current(500))
        .expect_err("get chunk index by offset 500 should fail for not supported");

    info!("ndn_local_chunklist_basic_fix_len test end.");
}

#[tokio::test]
async fn ndn_local_chunklist_basic_var_no_mix_len() {
    init_logging("ndn_local_chunklist_basic_fix_len", false);

    info!("ndn_local_chunklist_basic_fix_len test start...");
    init_obj_array_storage_factory().await;

    let mut rng = rand::rng();

    let chunk_size1: u64 = 1024 * 1024 + 513;
    let (chunk_id1, chunk_data1) = generate_random_chunk_mix(chunk_size1);

    let chunk_size2: u64 = 1024 * 1024 * 3 + 5;
    let (chunk_id2, chunk_data2) = generate_random_chunk_mix(chunk_size2);

    let chunk_size3: u64 = 1024 + 13;
    let (chunk_id3, chunk_data3) = generate_random_chunk(chunk_size3);

    let chunk_size4: u64 = 1024 * 2 + 113;
    let (chunk_id4, chunk_data4) = generate_random_chunk_mix(chunk_size4);

    let chunk_size5: u64 = 1024 * 1024 * 2 + 53;
    let (chunk_id5, chunk_data5) = generate_random_chunk_mix(chunk_size5);

    let mut var_mix_chunk_list_builder = ChunkListBuilder::new(HashMethod::Sha256, None)
        .with_total_size(chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5)
        .with_var_size();

    // [1]
    var_mix_chunk_list_builder
        .append(chunk_id1.clone())
        .expect("append chunk_id1 to chunk_arr failed");
    // [1, 2]
    var_mix_chunk_list_builder
        .append(chunk_id2.clone())
        .expect("append chunk_id1 to chunk_arr failed");
    // [1, 2, 3]
    var_mix_chunk_list_builder
        .append(chunk_id3.clone())
        .expect("insert chunk_id3 to chunk_arr failed");
    // [1, 2, 3, 4]
    var_mix_chunk_list_builder
        .append(chunk_id4.clone())
        .expect("insert chunk_id4 to chunk_arr failed");
    // [1, 2, 3, 4, 5]
    var_mix_chunk_list_builder
        .append(chunk_id5.clone())
        .expect("insert chunk_id4 to chunk_arr failed");

    let var_mix_chunk_list = var_mix_chunk_list_builder
        .build()
        .await
        .expect("build chunk list failed");

    assert_eq!(
        var_mix_chunk_list.get_total_size(),
        chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5,
        "var_mix_chunk_list total size check failed"
    );
    assert!(
        !var_mix_chunk_list.is_fixed_size_chunk_list(),
        "chunk_list fix size check failed"
    );
    assert_eq!(
        var_mix_chunk_list.get_len(),
        5,
        "chunk_list length check failed"
    );

    assert_eq!(
        var_mix_chunk_list
            .get_chunk(0)
            .expect("get chunk 0 failed")
            .expect("chunk_list first object check failed"),
        chunk_id1,
        "chunk_list first object check failed"
    );

    assert_eq!(
        var_mix_chunk_list
            .get_chunk(1)
            .expect("get chunk 1 failed")
            .expect("chunk_list second object check failed"),
        chunk_id2,
        "chunk_list second object check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk(2)
            .expect("get chunk 2 failed")
            .expect("chunk_list third object check failed"),
        chunk_id3,
        "chunk_list third object check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk(3)
            .expect("get chunk 3 failed")
            .expect("chunk_list fourth object check failed"),
        chunk_id4,
        "chunk_list fourth object check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk(4)
            .expect("get chunk 4 failed")
            .expect("chunk_list fifth object check failed"),
        chunk_id5,
        "chunk_list fifth object check failed"
    );
    assert!(
        var_mix_chunk_list
            .get_chunk(5)
            .expect("should Ok(None) for larger index")
            .is_none(),
        "chunk_list sixth object check failed"
    );
    assert_eq!(var_mix_chunk_list.get_meta().fix_size, None);

    // from start
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(0))
            .expect("get chunk index by offset 0 failed"),
        (0, 0),
        "chunk_list first object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_offset_by_index(0)
            .expect("get chunk offset by index 0 failed"),
        0,
        "chunk_list first object offset check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(chunk_size1 + chunk_size2))
            .expect("get chunk index by offset failed"),
        (2, 0),
        "chunk_list 3 object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_offset_by_index(2)
            .expect("get chunk offset by index 3 failed"),
        chunk_size1 + chunk_size2,
        "chunk_list 3 object offset check failed"
    );
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(chunk_size1 + chunk_size2 + 1))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(chunk_size1 + chunk_size2 + chunk_size3))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(
            chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4,
        ))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(
            chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5 - 1,
        ))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(
            chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5,
        ))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Start(
            chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5 + 123456789,
        ))
        .expect_err("unknown offset for chunk[>=2]");

    var_mix_chunk_list
        .get_chunk_offset_by_index(3)
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_offset_by_index(4)
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_offset_by_index(5)
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_offset_by_index(123456)
        .expect_err("unknown offset for chunk[>=2]");

    let chunk_offset = rng.random_range(1..chunk_size2 - 1);
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::Start(chunk_size1 + chunk_offset))
            .expect("get chunk index by offset failed"),
        (1, chunk_offset),
        "chunk_list 1.x object index check failed"
    );

    // from end
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-1))
            .expect("get chunk index by offset 0 failed"),
        (4, chunk_size5 - 1),
        "chunk_list -1 object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-((chunk_size5) as i64) - 1))
            .expect("get chunk index by offset failed"),
        (3, chunk_size4 - 1),
        "chunk_list 3 object index check failed"
    );
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-((chunk_size4 + chunk_size5) as i64)))
            .expect("get chunk index by offset failed"),
        (3, 0),
        "chunk_list 3 object index check failed"
    );
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(-((chunk_size4 + chunk_size5) as i64) - 1))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(
            -((chunk_size3 + chunk_size4 + chunk_size5) as i64) - 1,
        ))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(
            -((chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5) as i64) - 1,
        ))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(
            -((chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5) as i64) - 1,
        ))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(
            -((chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5) as i64),
        ))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(
            -((chunk_size1 + chunk_size2 + chunk_size3 + chunk_size4 + chunk_size5) as i64)
                - 123456789,
        ))
        .expect_err("unknown offset for chunk[>=2]");
    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::End(1))
        .expect_err("should fail for out of range");

    let chunk_offset = rng.random_range(2..(chunk_size5 as i64) - 1);
    assert_eq!(
        var_mix_chunk_list
            .get_chunk_index_by_offset(SeekFrom::End(-(chunk_size5 as i64) - chunk_offset))
            .expect("get chunk index by offset failed"),
        (3, (chunk_size4 - chunk_offset as u64)),
        "chunk_list 3.x object index check failed"
    );

    var_mix_chunk_list
        .get_chunk_index_by_offset(SeekFrom::Current(500))
        .expect_err("get chunk index by offset 500 should fail for not supported");

    info!("ndn_local_chunklist_basic_fix_len test end.");
}

#[tokio::test]
async fn ndn_local_chunklist_ok() {
    init_logging("ndn_local_chunklist_ok", false);

    info!("ndn_local_chunklist_ok test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(5, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut chunk_list_builder =
        ChunkListBuilder::new(HashMethod::Sha256, None).with_total_size(total_size);

    for (chunk_id, chunk_data) in chunks.iter() {
        write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        chunk_list_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }

    let chunk_list = chunk_list_builder
        .build()
        .await
        .expect("build chunk list failed");

    let (chunk_list_id, chunk_list_str) = chunk_list.calc_obj_id();
    NamedDataMgr::put_object(
        Some(ndn_mgr_id.as_str()),
        &chunk_list_id,
        chunk_list_str.as_str(),
    )
    .await
    .expect("put chunk_list to ndn-mgr failed");
    info!("chunk_list_id: {}", chunk_list_id.to_string());

    let chunk_list_json = NamedDataMgr::get_object(Some(ndn_mgr_id.as_str()), &chunk_list_id, None)
        .await
        .expect("open chunk list reader from ndn-mgr failed.");

    let got_chunk_list = ChunkListBuilder::open(chunk_list_json)
        .await
        .expect("open chunk list from ndn-mgr failed")
        .build()
        .await
        .expect("build chunk list from ndn-mgr failed");

    let (got_chunk_list_id, got_chunk_list_str) = got_chunk_list.calc_obj_id();
    assert_eq!(
        got_chunk_list_id, chunk_list_id,
        "chunk_list id check failed"
    );
    assert_eq!(
        got_chunk_list_str, chunk_list_str,
        "chunk_list string check failed"
    );

    assert_eq!(
        got_chunk_list.get_total_size(),
        total_size,
        "chunk_list total size check failed"
    );
    assert!(
        !got_chunk_list.is_fixed_size_chunk_list(),
        "chunk_list fix size check failed"
    );
    assert_eq!(
        got_chunk_list.get_len(),
        chunks.len(),
        "chunk_list length check failed"
    );

    for idx in 0..got_chunk_list.get_len() {
        assert_eq!(
            got_chunk_list.get_chunk(idx).expect("get chunk failed"),
            chunks.get(idx).map(|(id, _)| id.clone()),
            "chunk_list {} object check failed",
            idx
        );

        let (mut chunk_reader, chunk_size) = NamedDataMgr::open_chunk_reader(
            Some(ndn_mgr_id.as_str()),
            &got_chunk_list.get_chunk(idx).unwrap().unwrap(),
            SeekFrom::Start(0),
            false,
        )
        .await
        .expect("open chunk list reader from ndn-mgr failed.");
        assert_eq!(
            chunk_size,
            chunks.get(idx).unwrap().1.len() as u64,
            "chunk_list first object size check failed"
        );

        let mut buffer = vec![0u8; chunk_size as usize];
        chunk_reader
            .read_exact(&mut buffer)
            .await
            .expect("read chunk list from ndn-mgr failed");
        assert_eq!(
            buffer,
            chunks.get(idx).unwrap().1,
            "chunk_list first object content check failed"
        );
    }
    assert!(
        got_chunk_list
            .get_chunk(chunks.len())
            .expect("should Ok(None) for larger index")
            .is_none(),
        "chunk_list sixth object check failed"
    );

    info!("ndn_local_chunklist_ok test end.");
}

#[tokio::test]
async fn ndn_local_chunklist_not_found() {
    init_logging("ndn_local_chunklist_not_found", false);

    info!("ndn_local_chunklist_not_found test start...");
    let storage_dir = init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(5, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut chunk_list_builder =
        ChunkListBuilder::new(HashMethod::Sha256, None).with_total_size(total_size);

    for (chunk_id, chunk_data) in chunks.iter() {
        write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        chunk_list_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }

    let chunk_list = chunk_list_builder
        .build()
        .await
        .expect("build chunk list failed");

    let (chunk_list_id, chunk_list_str) = chunk_list.calc_obj_id();
    NamedDataMgr::put_object(
        Some(ndn_mgr_id.as_str()),
        &chunk_list_id,
        chunk_list_str.as_str(),
    )
    .await
    .expect("put chunk_list to ndn-mgr failed");
    info!("chunk_list_id: {}", chunk_list_id.to_string());

    // delete the chunk list storage file
    let _ = std::fs::remove_file(
        storage_dir.join(
            chunk_list
                .deref()
                .get_obj_id()
                .expect("should calc obj-array id")
                .to_base32()
                + ".json",
        ),
    );
    let _ = std::fs::remove_file(
        storage_dir.join(
            chunk_list
                .deref()
                .get_obj_id()
                .expect("should calc obj-array id")
                .to_base32()
                + ".arrow",
        ),
    );

    info!(
        "ndn_local_chunklist_not_found chunk_list_id: {:?}",
        chunk_list_id
    );

    let chunk_list_json = NamedDataMgr::get_object(Some(ndn_mgr_id.as_str()), &chunk_list_id, None)
        .await
        .expect("open chunk list reader from ndn-mgr failed.");

    ChunkListBuilder::open(chunk_list_json)
        .await
        .map(|_| ())
        .expect_err(
            "build chunk list from ndn-mgr should failed for object-array has been deleted",
        );

    info!("ndn_local_chunklist_not_found test end.");
}
