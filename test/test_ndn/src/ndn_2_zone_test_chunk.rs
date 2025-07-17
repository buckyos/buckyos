use std::io::SeekFrom;

use crate::common::*;
use hex::ToHex;
use log::*;
use ndn_lib::*;

//#[tokio::test]
pub async fn ndn_2_zone_chunk_ok() {
    info!("ndn_2_zone_chunk_ok");

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

//#[tokio::test]
pub async fn ndn_2_zone_chunk_not_found() {
    info!("ndn_2_zone_chunk_not_found");

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

//#[tokio::test]
pub async fn ndn_2_zone_chunk_verify_failed() {
    info!("ndn_2_zone_chunk_verify_failed");

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
