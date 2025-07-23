use buckyos_kit::*;
use hex::ToHex;
use ndn_lib::*;
use test_ndn::*;

#[tokio::test]
async fn ndn_local_chunk_ok() {
    init_logging("ndn_local_chunk_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 515;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);

    NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    let _buffer = NamedDataMgrTest::read_chunk_with_check(
        ndn_mgr_id.as_str(),
        &chunk_id,
        chunk_data.as_slice(),
    )
    .await;
}

#[tokio::test]
async fn ndn_local_chunk_not_found() {
    init_logging("ndn_local_chunk_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 + 154;
    let (chunk_id, _chunk_data) = generate_random_chunk(chunk_size);

    // Pull the chunk using the NdnClient
    NamedDataMgrTest::open_chunk_reader_not_found(ndn_mgr_id.as_str(), &chunk_id).await;
}

#[tokio::test]
async fn ndn_local_chunk_verify_failed() {
    init_logging("ndn_local_chunk_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunk_size: u64 = 1024 * 1024 + 567;
    let (chunk_id, chunk_data) = generate_random_chunk(chunk_size);
    let mut fake_chunk_data = chunk_data.clone();
    fake_chunk_data.splice(0..10, 0..10);

    NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, &fake_chunk_data).await;

    let buffer = NamedDataMgrTest::read_chunk_with_check(
        ndn_mgr_id.as_str(),
        &chunk_id,
        fake_chunk_data.as_slice(),
    )
    .await;

    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&buffer);
    let fake_chunk_id = ChunkId::from_sha256_result(&hash);
    assert_ne!(fake_chunk_id, chunk_id, "chunk-id should mismatch");
}
