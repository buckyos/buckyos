use std::io::SeekFrom;

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

const LOCAL_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;

const NODE_B_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;

fn generate_random_bytes(size: u64) -> Vec<u8> {
    let mut rng = rand::rng();
    let mut buffer = vec![0u8; size as usize];
    rng.fill_bytes(&mut buffer);
    buffer
}

fn generate_random_chunk(size: u64) -> (ChunkId, Vec<u8>) {
    let chunk_data = generate_random_bytes(size);
    let hasher = ChunkHasher::new(None).expect("hash failed.");
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

type NdnServerHost = String;

async fn init_local_ndn_server(ndn_mgr_id: &str) -> (NdnClient, NdnServerHost) {
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

async fn init_ndn_client(ndn_mgr_id: &str, private_key: &str, target_ndn_host: &str) -> NdnClient {
    let session_token = kRPC::RPCSessionToken {
        token_type: kRPC::RPCSessionTokenType::JWT,
        token: None,
        appid: Some("ndn".to_string()),
        exp: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600 * 24 * 7,
        ),
        iss: None,
        nonce: None,
        userid: None,
        session: None,
    };

    let private_key = EncodingKey::from_ed_pem(private_key.as_bytes()).unwrap();

    let target_ndn_client = NdnClient::new(
        format!("http://{}/ndn/", target_ndn_host),
        Some(
            session_token
                .generate_jwt(None, &private_key)
                .expect("generate jwt failed."),
        ),
        Some(ndn_mgr_id.to_string()),
    );

    target_ndn_client
}

#[tokio::test]
async fn ndn_2_zone_chunk_ok() {
    init_logging("ndn_2_zone_chunk_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, _) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "test.buckyos.io",
    )
    .await;

    // 1. write the chunk to local ndn-mgr
    let chunk_size: u64 = 1024 * 1024 + 515;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);
    write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    // 2. push the chunk using the NdnClient to zone_a
    local_ndn_client
        .push_chunk(
            chunk_id.clone(),
            Some(zone_a_client.gen_chunk_url(&chunk_id, None)),
        )
        .await
        .expect("push chunk from ndn-mgr failed");

    // 3. Pull the chunk using the NdnClient from zone_a with private key of zone_b
    zone_b_client
        .pull_chunk(chunk_id.clone(), Some(target_ndn_mgr_id.as_str()))
        .await
        .expect("pull chunk from ndn-mgr failed");

    let buffer = read_chunk(target_ndn_mgr_id.as_str(), &chunk_id).await;
    assert_eq!(buffer, chunk_data);
}

#[tokio::test]
async fn ndn_2_zone_chunk_not_found() {
    init_logging("ndn_2_zone_chunk_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, _) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let _zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "test.buckyos.io",
    )
    .await;

    let chunk_size: u64 = 1024 * 1024 + 515;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    // 1. write the chunk to local ndn-mgr
    write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    // 2. push the chunk using the NdnClient to zone_a
    local_ndn_client
        .push_chunk(
            chunk_id.clone(),
            Some(zone_a_client.gen_chunk_url(&chunk_id, None)),
        )
        .await
        .expect("push chunk from ndn-mgr failed");

    // 3. Pull the chunk using the NdnClient from zone_a with private key of zone_b
    // zone_b_client
    //     .pull_chunk(chunk_id.clone(), Some(target_ndn_mgr_id.as_str()))
    //     .await
    //     .expect("pull chunk from ndn-mgr failed");

    let ret = NamedDataMgr::open_chunk_reader(
        Some(target_ndn_mgr_id.as_str()),
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
                assert!(false, "Unexpected error type, {:?}", err);
            }
        },
    }
}

#[tokio::test]
async fn ndn_2_zone_chunk_verify_failed() {
    init_logging("ndn_2_zone_chunk_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, _) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "test.buckyos.io",
    )
    .await;

    let chunk_size: u64 = 1024 * 1024 + 567;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    let mut fake_chunk_data = chunk_data.clone();
    fake_chunk_data.splice(0..10, 0..10);

    write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice()).await;

    // 2. push the chunk using the NdnClient to zone_a
    local_ndn_client
        .push_chunk(
            chunk_id.clone(),
            Some(zone_a_client.gen_chunk_url(&chunk_id, None)),
        )
        .await
        .expect("push chunk from ndn-mgr failed");

    // 3. Pull the chunk using the NdnClient from zone_a with private key of zone_b
    zone_b_client
        .pull_chunk(chunk_id.clone(), Some(target_ndn_mgr_id.as_str()))
        .await
        .expect("pull chunk from ndn-mgr failed");

    let buffer = read_chunk(target_ndn_mgr_id.as_str(), &chunk_id).await;

    assert_eq!(buffer, fake_chunk_data, "chunk-content check failed");

    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&buffer);
    let fake_chunk_id = ChunkId::from_sha256_result(&hash);
    assert_ne!(fake_chunk_id, chunk_id, "chunk-id should mismatch");
}
