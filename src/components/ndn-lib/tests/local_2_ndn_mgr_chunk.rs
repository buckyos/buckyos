use buckyos_kit::*;
use hex::ToHex;
use test_ndn::*;

// 暂时先起两个不同的NamedDataMgr模拟相同zone内的两个Device
#[tokio::test]
async fn ndn_local_2_mgr_chunk_ok() {
    init_logging("ndn_local_2_mgr_chunk_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, _) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunk1_size: u64 = 1024 * 1024 + 515;
    let (chunk1_id, chunk1_data) = generate_random_chunk(chunk1_size);
    NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk1_id, chunk1_data.as_slice()).await;

    let chunk2_size: u64 = 1024 * 1024 + 515;
    let (chunk2_id, chunk2_data) = generate_random_chunk(chunk2_size);
    NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk2_id, chunk2_data.as_slice()).await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // push the chunk using the NdnClient
    ndn_client
        .push_chunk(
            chunk1_id.clone(),
            Some(target_ndn_client.gen_chunk_url(&chunk1_id, None)),
        )
        .await
        .expect("push chunk from ndn-mgr failed");

    let _buffer = NamedDataMgrTest::read_chunk_with_check(
        target_ndn_mgr_id.as_str(),
        &chunk1_id,
        chunk1_data.as_slice(),
    )
    .await;

    // Pull the chunk using the NdnClient
    ndn_client
        .pull_chunk(chunk2_id.clone(), Some(target_ndn_mgr_id.as_str()))
        .await
        .expect("pull chunk from ndn-mgr failed");

    let _buffer = NamedDataMgrTest::read_chunk_with_check(
        target_ndn_mgr_id.as_str(),
        &chunk2_id,
        chunk2_data.as_slice(),
    )
    .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_chunk_not_found() {
    init_logging("ndn_local_2_mgr_chunk_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 515;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _target_ndn_client = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // Pull the chunk using the NdnClient
    // let ret = ndn_client
    //     .pull_chunk(chunk_id.clone(), Some(target_ndn_mgr_id.as_str()))
    //     .await;

    NamedDataMgrTest::open_chunk_reader_not_found(target_ndn_mgr_id.as_str(), &chunk_id).await;
}

#[tokio::test]
async fn ndn_local_2_mgr_chunk_verify_failed() {
    init_logging("ndn_local_2_mgr_chunk_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, _) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 567;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    let mut fake_chunk_data = chunk_data.clone();
    fake_chunk_data.splice(0..10, 0..10);

    NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice()).await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _target_ndn_client = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // Pull the chunk using the NdnClient
    ndn_client
        .pull_chunk(chunk_id.clone(), Some(target_ndn_mgr_id.as_str()))
        .await
        .expect("pull chunk from local-zone failed");

    let buffer = NamedDataMgrTest::read_chunk_with_check(
        target_ndn_mgr_id.as_str(),
        &chunk_id,
        fake_chunk_data.as_slice(),
    )
    .await;

    // //assert_eq!(buffer, fake_chunk_data, "chunk-content check failed");

    // let hasher = ChunkHasher::new(None).expect("hash failed.");
    // let hash = hasher.calc_from_bytes(&buffer);
    // let fake_chunk_id = ChunkId::from_sha256_result(&hash);
    // assert_ne!(fake_chunk_id, chunk_id, "chunk-id should mismatch");
}
