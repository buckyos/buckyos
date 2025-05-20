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
                            "read_only": true,
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
    init_logging("ndn_same_zone_chunk_ok", false);

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
    init_logging("ndn_same_zone_chunk_ok", false);

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
    init_logging("ndn_same_zone_chunk_ok", false);

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
async fn ndn_same_zone_chunk_ok() {
    init_logging("ndn_same_zone_chunk_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 515;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    // push the chunk using the NdnClient
    ndn_client
        .push_chunk(chunk_id.clone(), None)
        .await
        .expect("push chunk from ndn-mgr failed");

    // Pull the chunk using the NdnClient
    ndn_client
        .pull_chunk(chunk_id.clone(), Some(ndn_mgr_id.as_str()))
        .await
        .expect("pull chunk from ndn-mgr failed");

    let buffer = read_chunk(ndn_mgr_id.as_str(), &chunk_id).await;

    assert_eq!(buffer, chunk_data);
}

#[tokio::test]
async fn ndn_same_zone_chunk_not_found() {
    init_logging("ndn_same_zone_chunk_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 515;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    // Pull the chunk using the NdnClient
    let ret = ndn_client
        .pull_chunk(chunk_id.clone(), Some(ndn_mgr_id.as_str()))
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
    init_logging("ndn_same_zone_chunk_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let ndn_client = init_handles(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 567;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    let mut fake_chunk_data = chunk_data.clone();
    fake_chunk_data.splice(0..10, 0..10);

    write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    // Pull the chunk using the NdnClient
    ndn_client
        .pull_chunk(chunk_id.clone(), Some(ndn_mgr_id.as_str()))
        .await
        .expect("pull chunk from local-zone failed");

    let buffer = read_chunk(ndn_mgr_id.as_str(), &chunk_id).await;

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
