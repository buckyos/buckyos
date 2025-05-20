use std::{io::SeekFrom, iter};

use buckyos_kit::*;
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use hex::ToHex;
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

fn generate_random_chunk(size: u64) -> (ChunkId, Vec<u8>) {
    let chunk_data = generate_random_bytes(size);
    let mut hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&chunk_data);
    let chunk_id = ChunkId::from_sha256_result(&hash);
    info!("chunk_id: {}", chunk_id.to_string());
    (chunk_id, chunk_data)
}

fn generate_random_obj() -> (ObjId, serde_json::Value) {
    let int_value = rand::random::<u32>();
    let str_value: String = generate_random_bytes(7).encode_hex();
    let test_obj_base = json!({
        "int": int_value,
        "string": str_value.clone(),
    });

    let test_obj = json!({
        "int": int_value,
        "string": str_value.clone(),
        "obj": test_obj_base.clone(),
        "array": [int_value, str_value.clone(), test_obj_base.clone()]
    });
    let (obj_id, _obj_str) = build_named_object_by_json("non-test", &test_obj);
    (obj_id, test_obj)
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

async fn check_obj_inner_path(
    ndn_mgr_id: &str,
    obj_id: &ObjId,
    obj_type: &str,
    inner_path: Option<&str>,
    expect_value: Option<Option<&serde_json::Value>>,
    unexpect_value: Option<Option<&serde_json::Value>>,
    expect_obj_id: Option<&ObjId>,
) {
    let got_ret =
        NamedDataMgr::get_object(Some(ndn_mgr_id), obj_id, inner_path.map(|p| p.to_string())).await;

    log::info!("ndn_local_object_ok test inner-path {:?}.", inner_path);

    if let Some(expect_value) = &expect_value {
        match expect_value {
            Some(expect_value) => match &got_ret {
                Ok(got_obj) => {
                    let (_expect_obj_id, expect_obj_str) =
                        build_named_object_by_json(obj_type, *expect_value);
                    let (got_obj_id, got_obj_str) = build_named_object_by_json(obj_type, got_obj);

                    if inner_path.is_none() {
                        assert_eq!(
                            &got_obj_id,
                            expect_obj_id.unwrap_or(obj_id),
                            "object-id mismatch"
                        );
                    }

                    // log::info!(
                    //     "ndn_local_object_ok test inner-path {:?} check object, expect: {}, got: {}.",
                    //     inner_path, expect_obj_str, got_obj_str
                    // );

                    assert_eq!(
                        got_obj_str, expect_obj_str,
                        "obj['{:?}'] check failed",
                        inner_path
                    );
                }
                Err(err) => assert!(
                    false,
                    "get object {:?} with innser-path {:?} failed",
                    obj_id, inner_path
                ),
            },
            None => match &got_ret {
                Ok(got_obj) => {
                    assert!(got_obj.is_null(), "should no object found")
                }
                Err(err) => match err {
                    NdnError::NotFound(_) => {
                        info!("Chunk not found as expected");
                    }
                    _ => {
                        assert!(false, "Unexpected error type");
                    }
                },
            },
        }
    }

    if let Some(unexpect_value) = &unexpect_value {
        match unexpect_value {
            Some(unexpect_value) => match &got_ret {
                Ok(got_obj) => {
                    let (_unexpect_obj_id, unexpect_obj_str) =
                        build_named_object_by_json(obj_type, *unexpect_value);
                    let (got_obj_id, got_obj_str) = build_named_object_by_json(obj_type, got_obj);

                    if inner_path.is_none() {
                        assert_eq!(
                            &got_obj_id,
                            expect_obj_id.unwrap_or(obj_id),
                            "object-id mismatch"
                        );
                    }
                    assert_ne!(
                        got_obj_str, unexpect_obj_str,
                        "obj['{:?}'] check failed",
                        inner_path
                    );
                }
                Err(err) => assert!(
                    false,
                    "get object {:?} with innser-path {:?} failed",
                    obj_id, inner_path
                ),
            },
            None => assert!(
                got_ret.is_ok(),
                "get object {:?} with innser-path {:?} failed",
                obj_id,
                inner_path
            ),
        }
    }
}

async fn init_handles(ndn_mgr_id: &str) -> NdnClient {
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

    let client = NdnClient::new(
        format!("http://localhost:{}/ndn/", http_port),
        None,
        Some(ndn_mgr_id.to_string()),
    );

    client
}

#[tokio::test]
async fn ndn_local_chunk_ok() {
    init_logging("ndn_local_chunk_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 515;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    let buffer = read_chunk(ndn_mgr_id.as_str(), &chunk_id).await;

    assert_eq!(buffer, chunk_data, "chunk-content check failed");
}

#[tokio::test]
async fn ndn_local_chunk_not_found() {
    init_logging("ndn_local_chunk_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 + 154;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    // Pull the chunk using the NdnClient
    let ret = NamedDataMgr::open_chunk_reader(
        Some(ndn_mgr_id.as_str()),
        &chunk_id,
        SeekFrom::Start(0),
        false,
    )
    .await;

    match ret {
        Ok(_) => assert!(false, "should no chunk found"),
        Err(err) => match err {
            NdnError::NotFound(_) => {
                info!("Chunk not found as expected");
            }
            _ => {
                assert!(false, "Unexpected error type");
            }
        },
    }
}

#[tokio::test]
async fn ndn_local_chunk_verify_failed() {
    init_logging("ndn_local_chunk_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 567;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);
    let mut fake_chunk_data = chunk_data.clone();
    fake_chunk_data.splice(0..10, 0..10);

    write_chunk(ndn_mgr_id.as_str(), &chunk_id, &fake_chunk_data).await;

    let buffer = read_chunk(ndn_mgr_id.as_str(), &chunk_id).await;

    assert_eq!(buffer, fake_chunk_data, "chunk-content check failed");

    let mut hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&buffer);
    let fake_chunk_id = ChunkId::from_sha256_result(&hash);
    assert_ne!(fake_chunk_id, chunk_id, "chunk-id should mismatch");
}

#[tokio::test]
async fn ndn_local_object_ok() {
    init_logging("ndn_local_object_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let (obj_id, obj) = generate_random_obj();

    let (_, obj_str) = build_named_object_by_json("non-test", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        None,
        Some(Some(&obj)),
        None,
        None,
    )
    .await;

    let inner_path = "string";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(obj.get(inner_path).expect(
            format!("inner-path '{}' not exist", inner_path).as_str(),
        ))),
        None,
        None,
    )
    .await;

    let inner_path = "int";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(obj.get(inner_path).expect(
            format!("inner-path '{}' not exist", inner_path).as_str(),
        ))),
        None,
        None,
    )
    .await;

    let inner_path = "obj";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(obj.get(inner_path).expect(
            format!("inner-path '{}' not exist", inner_path).as_str(),
        ))),
        None,
        None,
    )
    .await;

    let inner_path = "obj/string";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(
            obj.get("obj")
                .expect("inner-path 'obj' not exist")
                .get("string")
                .expect("inner-path 'obj/string' not exist"),
        )),
        None,
        None,
    )
    .await;

    let inner_path = "obj/int";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(
            obj.get("obj")
                .expect("inner-path 'obj' not exist")
                .get("int")
                .expect("inner-path 'obj/int' not exist"),
        )),
        None,
        None,
    )
    .await;

    let inner_path = "array";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(obj.get(inner_path).expect(
            format!("inner-path '{}' not exist", inner_path).as_str(),
        ))),
        None,
        None,
    )
    .await;

    let inner_path = "array/0";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(
            obj.get("array")
                .expect("inner-path 'array' not exist")
                .get(0)
                .expect("inner-path 'array/0' not exist"),
        )),
        None,
        None,
    )
    .await;

    let inner_path = "array/1";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(
            obj.get("array")
                .expect("inner-path 'array' not exist")
                .get(1)
                .expect("inner-path 'array/0' not exist"),
        )),
        None,
        None,
    )
    .await;

    let inner_path = "array/2";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(Some(
            obj.get("array")
                .expect("inner-path 'array' not exist")
                .get(2)
                .expect("inner-path 'array/0' not exist"),
        )),
        None,
        None,
    )
    .await;

    let inner_path = "not-exist";
    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        Some(inner_path),
        Some(None),
        None,
        None,
    )
    .await;
}

#[tokio::test]
async fn ndn_local_object_not_found() {
    init_logging("ndn_local_object_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let (obj_id, obj) = generate_random_obj();

    // no put
    // let (_, obj_str) = build_named_object_by_json("non-test", &obj);
    // NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
    //     .await
    //     .expect("put object in local failed");

    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        None,
        Some(None),
        None,
        None,
    )
    .await;
}

#[tokio::test]
async fn ndn_local_object_verify_failed() {
    init_logging("ndn_local_object_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let (obj_id, obj) = generate_random_obj();
    let (fake_obj_id, fake_obj) = generate_random_obj();

    let (_, fake_obj_str) = build_named_object_by_json("non-test", &fake_obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, fake_obj_str.as_str())
        .await
        .expect("put object in local failed");

    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test",
        None,
        Some(Some(&fake_obj)),
        Some(Some(&obj)),
        Some(&fake_obj_id),
    )
    .await;
}

// 暂时先起两个不同的NamedDataMgr模拟相同zone内的两个Device
#[tokio::test]
async fn ndn_same_zone_chunk_ok() {
    init_logging("ndn_same_zone_chunk_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk1_size: u64 = 1024 * 1024 + 515;
    let (chunk1_id, chunk1_data) = generate_random_chunk(chunk1_size);
    write_chunk(ndn_mgr_id.as_str(), &chunk1_id, chunk1_data.as_slice()).await;

    let chunk2_size: u64 = 1024 * 1024 + 515;
    let (chunk2_id, chunk2_data) = generate_random_chunk(chunk2_size);
    write_chunk(ndn_mgr_id.as_str(), &chunk2_id, chunk2_data.as_slice()).await;

    let remote_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let remote_ndn_client = init_handles(remote_ndn_mgr_id.as_str()).await;

    // push the chunk using the NdnClient
    ndn_client
        .push_chunk(
            chunk1_id.clone(),
            Some(remote_ndn_client.gen_chunk_url(&chunk1_id, None)),
        )
        .await
        .expect("push chunk from ndn-mgr failed");

    let buffer = read_chunk(remote_ndn_mgr_id.as_str(), &chunk1_id).await;
    assert_eq!(buffer, chunk1_data);

    // Pull the chunk using the NdnClient
    ndn_client
        .pull_chunk(chunk2_id.clone(), Some(remote_ndn_mgr_id.as_str()))
        .await
        .expect("pull chunk from ndn-mgr failed");

    let buffer = read_chunk(remote_ndn_mgr_id.as_str(), &chunk2_id).await;
    assert_eq!(buffer, chunk2_data);
}

#[tokio::test]
async fn ndn_same_zone_chunk_not_found() {
    init_logging("ndn_same_zone_chunk_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 515;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    let remote_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _remote_ndn_client = init_handles(remote_ndn_mgr_id.as_str()).await;

    // Pull the chunk using the NdnClient
    // let ret = ndn_client
    //     .pull_chunk(chunk_id.clone(), Some(remote_ndn_mgr_id.as_str()))
    //     .await;

    let ret = NamedDataMgr::open_chunk_reader(
        Some(remote_ndn_mgr_id.as_str()),
        &chunk_id,
        SeekFrom::Start(0),
        false,
    )
    .await;

    match ret {
        Ok(_) => assert!(false, "should no chunk found"),
        Err(err) => match err {
            NdnError::NotFound(_) => {
                info!("Chunk not found as expected");
            }
            _ => {
                assert!(false, "Unexpected error type");
            }
        },
    }
}

#[tokio::test]
async fn ndn_same_zone_chunk_verify_failed() {
    init_logging("ndn_same_zone_chunk_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 567;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    let mut fake_chunk_data = chunk_data.clone();
    fake_chunk_data.splice(0..10, 0..10);

    write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice()).await;

    let remote_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _remote_ndn_client = init_handles(remote_ndn_mgr_id.as_str()).await;

    // Pull the chunk using the NdnClient
    ndn_client
        .pull_chunk(chunk_id.clone(), Some(remote_ndn_mgr_id.as_str()))
        .await
        .expect("pull chunk from local-zone failed");

    let buffer = read_chunk(remote_ndn_mgr_id.as_str(), &chunk_id).await;

    assert_eq!(buffer, fake_chunk_data, "chunk-content check failed");

    let mut hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&buffer);
    let fake_chunk_id = ChunkId::from_sha256_result(&hash);
    assert_ne!(fake_chunk_id, chunk_id, "chunk-id should mismatch");
}

#[tokio::test]
async fn ndn_same_zone_o_link_innerpath_ok() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_same_zone_o_link_innerpath_not_found() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_same_zone_o_link_innerpath_verify_failed() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_same_zone_r_link_innerpath_ok() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_same_zone_r_link_innerpath_not_found() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_same_zone_r_link_innerpath_verify_failed() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_diff_zone_o_link_innerpath_ok() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_diff_zone_o_link_innerpath_not_found() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_diff_zone_o_link_innerpath_verify_failed() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_diff_zone_r_link_innerpath_ok() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_diff_zone_r_link_innerpath_not_found() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_diff_zone_r_link_innerpath_verify_failed() {
    unimplemented!()
}
