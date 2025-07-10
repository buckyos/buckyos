use std::{io::SeekFrom, path::PathBuf};

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

fn generate_random_chunk_mix(size: u64) -> (ChunkId, Vec<u8>) {
    let chunk_data = generate_random_bytes(size);
    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&chunk_data);
    let chunk_id = ChunkId::mix_from_hash_result(size, &hash, HashMethod::Sha256);
    info!("chunk_id: {}", chunk_id.to_string());
    (chunk_id, chunk_data)
}

fn _generate_random_chunk(size: u64) -> (ChunkId, Vec<u8>) {
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

async fn _read_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId) -> Vec<u8> {
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
async fn ndn_local_file_chunklist_rechunk_split() {
    init_logging("ndn_local_file_chunklist_rechunk_split", false);

    info!("ndn_local_file_chunklist_rechunk_split test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(10, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let (chunk0_id, chunk0_data) = chunks.get(0).unwrap();
    write_chunk(ndn_mgr_id.as_str(), chunk0_id, chunk0_data.as_slice()).await;

    // File(chunk0)
    let file0 = FileObject::new(
        "ndn_local_file_chunklist_rechunk_split_v0".to_string(),
        chunk0_data.len() as u64,
        chunk0_id.to_string(),
    );

    let (file0_id, file0_str) = file0.gen_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file0_id, file0_str.as_str())
        .await
        .expect("put file0 to ndn-mgr failed");

    info!("file0_id: {}", file0_id.to_string());

    let mut chunk_list_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(total_size);

    chunk_list_builder
        .append(chunk0_id.clone())
        .expect("append chunk to chunk_arr failed");
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

    // File([chunk0]) -> file0
    let mut file1 = FileObject::new(
        "ndn_local_file_chunklist_rechunk_split_v1".to_string(),
        chunk0_data.len() as u64,
        chunk_list_id.to_string(),
    );

    file1.links = Some(vec![LinkData::SameAs(file0_id.clone())]);

    let (file1_id, file1_str) = file1.gen_obj_id();
    info!("file1_id: {}", file1_id.to_string());
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file1_id, file1_str.as_str())
        .await
        .expect("put file1 to ndn-mgr failed");

    let file1_content_url = format!("http://{}/ndn/{}/content", ndn_host, file1_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file1_content_url.as_str(), None, None)
        .await
        .expect("open file1 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len,
        chunk0_data.len() as u64,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(chunk_list_id),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file1_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );
    assert_eq!(
        buffer.as_slice(),
        chunk0_data.as_slice(),
        "chunk range mismatch"
    );

    // File([chunk0[0], chunk0[1], chunk0[2]]) -> file0
    let part0_len = rand::rng().random_range(1u64..chunk0_data.len() as u64 - 5);
    let part1_len = rand::rng().random_range(1u64..chunk0_data.len() as u64 - part0_len - 2);
    let part2_len = chunk0_data.len() as u64 - part0_len - part1_len;
    let part_lens = vec![
        (0, part0_len),
        (part0_len, part1_len),
        (part0_len + part1_len, part2_len),
    ];
    let part_chunks = part_lens
        .iter()
        .map(|(start_pos, len)| {
            let hasher = ChunkHasher::new(None).expect("hash failed.");
            let hash = hasher.calc_from_bytes(
                &chunk0_data.as_slice()[*start_pos as usize..(*start_pos + *len) as usize],
            );
            let chunk_id = ChunkId::mix_from_hash_result(*len, &hash, HashMethod::Sha256);
            info!("chunk_id: {}", chunk_id.to_string());
            chunk_id
        })
        .collect::<Vec<_>>();
    let mut chunk_list_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(chunk0_data.len() as u64);

    for (_idx, chunk_id) in part_chunks.iter().enumerate() {
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
    let mut file2 = FileObject::new(
        "ndn_local_file_chunklist_rechunk_split_v2".to_string(),
        chunk0_data.len() as u64,
        chunk_list_id.to_string(),
    );

    file2.links = Some(vec![LinkData::SameAs(file0_id.clone())]);

    let (file2_id, file2_str) = file2.gen_obj_id();
    info!("file2_id: {}", file2_id.to_string());
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file2_id, file2_str.as_str())
        .await
        .expect("put file2 to ndn-mgr failed");

    let file2_content_url = format!("http://{}/ndn/{}/content", ndn_host, file2_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file2_content_url.as_str(), None, None)
        .await
        .expect("open file2 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len,
        chunk0_data.len() as u64,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(chunk_list_id.clone()),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file2_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );
    assert_eq!(
        buffer.as_slice(),
        chunk0_data.as_slice(),
        "chunk range mismatch"
    );

    // File([chunk0[0], chunk0[1], chunk0[2]]) -> file1
    let mut file3 = FileObject::new(
        "ndn_local_file_chunklist_rechunk_split_v3".to_string(),
        chunk0_data.len() as u64,
        chunk_list_id.to_string(),
    );

    file3.links = Some(vec![LinkData::SameAs(file1_id.clone())]);

    let (file3_id, file3_str) = file3.gen_obj_id();
    info!("file3_id: {}", file3_id.to_string());
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file3_id, file3_str.as_str())
        .await
        .expect("put file3 to ndn-mgr failed");

    let file3_content_url = format!("http://{}/ndn/{}/content", ndn_host, file3_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file3_content_url.as_str(), None, None)
        .await
        .expect("open file3 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len,
        chunk0_data.len() as u64,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(chunk_list_id.clone()),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file3_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );
    assert_eq!(
        buffer.as_slice(),
        chunk0_data.as_slice(),
        "chunk range mismatch"
    );

    // File([chunk0[0], chunk0[1], chunk0[2]]) -> file0 & file1
    let mut file4 = FileObject::new(
        "ndn_local_file_chunklist_rechunk_split_v4".to_string(),
        chunk0_data.len() as u64,
        chunk_list_id.to_string(),
    );

    file4.links = Some(vec![
        LinkData::SameAs(file1_id.clone()),
        LinkData::SameAs(file0_id.clone()),
    ]);

    let (file4_id, file4_str) = file4.gen_obj_id();
    info!("file4_id: {}", file4_id.to_string());
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file4_id, file4_str.as_str())
        .await
        .expect("put file4 to ndn-mgr failed");

    let file4_content_url = format!("http://{}/ndn/{}/content", ndn_host, file4_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file4_content_url.as_str(), None, None)
        .await
        .expect("open file4 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len,
        chunk0_data.len() as u64,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(chunk_list_id),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file4_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );
    assert_eq!(
        buffer.as_slice(),
        chunk0_data.as_slice(),
        "chunk range mismatch"
    );

    info!("ndn_local_chunklist_ok test end.");
}

#[tokio::test]
async fn ndn_local_file_chunklist_rechunk_combine() {
    init_logging("ndn_local_file_chunklist_rechunk_combine", false);

    info!("ndn_local_file_chunklist_rechunk_combine test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(10, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut chunk_list_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(total_size);

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

    // File([chunk0, chunk1, chunk2, chunk3, chunk4 ... chunk9])
    let file0 = FileObject::new(
        "ndn_local_file_chunklist_rechunk_combine_v0".to_string(),
        total_size,
        chunk_list_id.to_string(),
    );

    let (file0_id, file0_str) = file0.gen_obj_id();
    info!("file0_id: {}", file0_id.to_string());
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file0_id, file0_str.as_str())
        .await
        .expect("put file0 to ndn-mgr failed");

    let combine_chunk_data = chunks
        .iter()
        .map(|c| c.1.as_slice())
        .collect::<Vec<_>>()
        .concat();
    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(combine_chunk_data.as_slice());
    let combine_chunk_id =
        ChunkId::mix_from_hash_result(combine_chunk_data.len() as u64, &hash, HashMethod::Sha256);
    info!("combine_chunk_id: {}", combine_chunk_id.to_string());

    // File(chunk0 + chunk1 + ... + chunk9) -> file0
    let mut file1 = FileObject::new(
        "ndn_local_file_chunklist_rechunk_combine_v0".to_string(),
        combine_chunk_data.len() as u64,
        combine_chunk_id.to_string(),
    );

    file1.links = Some(vec![LinkData::SameAs(file0_id.clone())]);

    let (file1_id, file1_str) = file1.gen_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file1_id, file1_str.as_str())
        .await
        .expect("put file1 to ndn-mgr failed");

    info!("file1_id: {}", file1_id.to_string());

    let file1_content_url = format!("http://{}/ndn/{}/content", ndn_host, file1_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file1_content_url.as_str(), None, None)
        .await
        .expect("open file1 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len,
        combine_chunk_data.len() as u64,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(combine_chunk_id.to_obj_id()),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file1_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );
    assert_eq!(
        buffer.as_slice(),
        combine_chunk_data.as_slice(),
        "chunk range mismatch"
    );

    // File([chunk0 + chunk1 + ... + chunk9]) -> file0
    let mut chunk_list_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(total_size);

    chunk_list_builder
        .append(combine_chunk_id.clone())
        .expect("append chunk to chunk_arr failed");

    let combine_chunk_list = chunk_list_builder
        .build()
        .await
        .expect("build chunk list failed");

    let (combine_chunk_list_id, combine_chunk_list_str) = combine_chunk_list.calc_obj_id();
    NamedDataMgr::put_object(
        Some(ndn_mgr_id.as_str()),
        &combine_chunk_list_id,
        combine_chunk_list_str.as_str(),
    )
    .await
    .expect("put chunk_list to ndn-mgr failed");

    let mut file2 = FileObject::new(
        "ndn_local_file_chunklist_rechunk_combine_v2".to_string(),
        combine_chunk_data.len() as u64,
        combine_chunk_list_id.to_string(),
    );

    file2.links = Some(vec![LinkData::SameAs(file0_id.clone())]);

    let (file2_id, file2_str) = file2.gen_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file2_id, file2_str.as_str())
        .await
        .expect("put file2 to ndn-mgr failed");

    info!("file2_id: {}", file2_id.to_string());

    let file2_content_url = format!("http://{}/ndn/{}/content", ndn_host, file2_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file2_content_url.as_str(), None, None)
        .await
        .expect("open file1 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len,
        combine_chunk_data.len() as u64,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(combine_chunk_list_id.clone()),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file2_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );
    assert_eq!(
        buffer.as_slice(),
        combine_chunk_data.as_slice(),
        "chunk range mismatch"
    );

    info!("ndn_local_file_chunklist_rechunk_combine test end.");
}

#[tokio::test]
async fn ndn_local_file_chunklist_delta() {
    init_logging("ndn_local_file_chunklist_delta", false);

    info!("ndn_local_file_chunklist_delta test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks_0_3 = generate_random_chunk_list(3, None);
    let chunks_3_6 = generate_random_chunk_list(3, None);
    let chunks_6_9 = generate_random_chunk_list(3, None);
    let chunks_9_12 = generate_random_chunk_list(3, None);

    // File0(chunks_3_6.concat())
    let combine_chunks_3_6 = chunks_3_6
        .iter()
        .map(|c| c.1.as_slice())
        .collect::<Vec<_>>()
        .concat();

    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(combine_chunks_3_6.as_slice());
    let combine_chunk_3_6_id =
        ChunkId::mix_from_hash_result(combine_chunks_3_6.len() as u64, &hash, HashMethod::Sha256);

    write_chunk(
        ndn_mgr_id.as_str(),
        &combine_chunk_3_6_id,
        combine_chunks_3_6.as_slice(),
    )
    .await;

    let file0 = FileObject::new(
        "ndn_local_file_chunklist_delta_v0".to_string(),
        combine_chunks_3_6.len() as u64,
        combine_chunk_3_6_id.to_string(),
    );

    let (file0_id, file0_str) = file0.gen_obj_id();
    info!("file0_id: {}", file0_id.to_string());
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file0_id, file0_str.as_str())
        .await
        .expect("put file0 to ndn-mgr failed");

    // File1(chunks_3_6) -> File0
    let mut chunk_list_3_6_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(combine_chunks_3_6.len() as u64);
    for (chunk_id, _chunk_data) in chunks_3_6.iter() {
        chunk_list_3_6_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    let chunk_list_3_6 = chunk_list_3_6_builder
        .build()
        .await
        .expect("build chunk list failed");
    let (chunk_list_3_6_id, chunk_list_3_6_str) = chunk_list_3_6.calc_obj_id();
    NamedDataMgr::put_object(
        Some(ndn_mgr_id.as_str()),
        &chunk_list_3_6_id,
        chunk_list_3_6_str.as_str(),
    )
    .await
    .expect("put chunk_list to ndn-mgr failed");

    let mut file1 = FileObject::new(
        "ndn_local_file_chunklist_delta_v1".to_string(),
        combine_chunks_3_6.len() as u64,
        chunk_list_3_6_id.to_string(),
    );

    file1.links = Some(vec![LinkData::SameAs(file0_id.clone())]);

    let (file1_id, file1_str) = file1.gen_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file1_id, file1_str.as_str())
        .await
        .expect("put file1 to ndn-mgr failed");

    info!("file1_id: {}", file1_id.to_string());

    let file1_content_url = format!("http://{}/ndn/{}/content", ndn_host, file1_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file1_content_url.as_str(), None, None)
        .await
        .expect("open file1 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len,
        combine_chunks_3_6.len() as u64,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(chunk_list_3_6_id.clone()),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file1_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );
    assert_eq!(
        buffer.as_slice(),
        combine_chunks_3_6.as_slice(),
        "chunk range mismatch"
    );

    // File2([...chunks_0_3, ...chunks_3_6]) // insert head
    let file2_len = chunks_0_3.iter().map(|(_, d)| d.len() as u64).sum::<u64>()
        + chunks_3_6.iter().map(|(_, d)| d.len() as u64).sum::<u64>();
    let mut chunk_list_0_6_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(file2_len);
    for (chunk_id, chunk_data) in chunks_0_3.iter() {
        write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        chunk_list_0_6_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    for (chunk_id, _chunk_data) in chunks_3_6.iter() {
        chunk_list_0_6_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    let chunk_list_0_6 = chunk_list_0_6_builder
        .build()
        .await
        .expect("build chunk list failed");
    let (chunk_list_0_6_id, chunk_list_0_6_str) = chunk_list_0_6.calc_obj_id();
    NamedDataMgr::put_object(
        Some(ndn_mgr_id.as_str()),
        &chunk_list_0_6_id,
        chunk_list_0_6_str.as_str(),
    )
    .await
    .expect("put chunk_list to ndn-mgr failed");

    let file2 = FileObject::new(
        "ndn_local_file_chunklist_delta_v2".to_string(),
        file2_len,
        chunk_list_0_6_id.to_string(),
    );

    let (file2_id, file2_str) = file2.gen_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file2_id, file2_str.as_str())
        .await
        .expect("put file2 to ndn-mgr failed");

    info!("file2_id: {}", file2_id.to_string());

    let file2_content_url = format!("http://{}/ndn/{}/content", ndn_host, file2_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file2_content_url.as_str(), None, None)
        .await
        .expect("open file1 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len, file2_len,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(chunk_list_0_6_id.clone()),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file2_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );

    let mut pos = 0;
    for chunk_data in [
        chunks_0_3
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
        chunks_3_6
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
    ]
    .concat()
    {
        let chunk_len = chunk_data.len() as u64;
        assert_eq!(
            &buffer.as_slice()[pos as usize..(pos + chunk_len as u64) as usize],
            chunk_data,
            "chunk range mismatch for chunk_id",
        );
        pos += chunk_len;
    }

    // File3([...chunks_0_3, ...chunks_3_6, ...chunks_9_12]) // insert tail
    let file3_len = chunks_0_3.iter().map(|(_, d)| d.len() as u64).sum::<u64>()
        + chunks_3_6.iter().map(|(_, d)| d.len() as u64).sum::<u64>()
        + chunks_9_12.iter().map(|(_, d)| d.len() as u64).sum::<u64>();
    let mut chunk_list_0_6_9_12_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(file3_len);
    for (chunk_id, _chunk_data) in chunks_0_3.iter() {
        chunk_list_0_6_9_12_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    for (chunk_id, _chunk_data) in chunks_3_6.iter() {
        chunk_list_0_6_9_12_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    for (chunk_id, chunk_data) in chunks_9_12.iter() {
        write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        chunk_list_0_6_9_12_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    let chunk_list_0_6_9_12 = chunk_list_0_6_9_12_builder
        .build()
        .await
        .expect("build chunk list failed");
    let (chunk_list_0_6_9_12_id, chunk_list_0_6_9_12_str) = chunk_list_0_6_9_12.calc_obj_id();
    NamedDataMgr::put_object(
        Some(ndn_mgr_id.as_str()),
        &chunk_list_0_6_9_12_id,
        chunk_list_0_6_9_12_str.as_str(),
    )
    .await
    .expect("put chunk_list to ndn-mgr failed");

    let file3 = FileObject::new(
        "ndn_local_file_chunklist_delta_v3".to_string(),
        file3_len,
        chunk_list_0_6_9_12_id.to_string(),
    );

    let (file3_id, file3_str) = file3.gen_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file3_id, file3_str.as_str())
        .await
        .expect("put file3 to ndn-mgr failed");

    info!("file3_id: {}", file3_id.to_string());

    let file3_content_url = format!("http://{}/ndn/{}/content", ndn_host, file3_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file3_content_url.as_str(), None, None)
        .await
        .expect("open file3 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len, file3_len,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(chunk_list_0_6_9_12_id.clone()),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file3_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );

    let mut pos = 0;
    for chunk_data in [
        chunks_0_3
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
        chunks_3_6
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
        chunks_9_12
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
    ]
    .concat()
    {
        let chunk_len = chunk_data.len() as u64;
        assert_eq!(
            &buffer.as_slice()[pos as usize..(pos + chunk_len as u64) as usize],
            chunk_data,
            "chunk range mismatch for chunk_id",
        );
        pos += chunk_len;
    }

    // File4([...chunks_0_3, ...chunks_3_6, ...chunks_6_9, ...chunks_9_12]) // insert middle
    let file4_len = chunks_0_3.iter().map(|(_, d)| d.len() as u64).sum::<u64>()
        + chunks_3_6.iter().map(|(_, d)| d.len() as u64).sum::<u64>()
        + chunks_6_9.iter().map(|(_, d)| d.len() as u64).sum::<u64>()
        + chunks_9_12.iter().map(|(_, d)| d.len() as u64).sum::<u64>();
    let mut chunk_list_0_12_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(file4_len);
    for (chunk_id, _chunk_data) in chunks_0_3.iter() {
        chunk_list_0_12_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    for (chunk_id, _chunk_data) in chunks_3_6.iter() {
        chunk_list_0_12_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    for (chunk_id, chunk_data) in chunks_6_9.iter() {
        write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
        chunk_list_0_12_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    for (chunk_id, _chunk_data) in chunks_9_12.iter() {
        chunk_list_0_12_builder
            .append(chunk_id.clone())
            .expect("append chunk to chunk_arr failed");
    }
    let chunk_list_0_12 = chunk_list_0_12_builder
        .build()
        .await
        .expect("build chunk list failed");
    let (chunk_list_0_12_id, chunk_list_0_12_str) = chunk_list_0_12.calc_obj_id();
    NamedDataMgr::put_object(
        Some(ndn_mgr_id.as_str()),
        &chunk_list_0_12_id,
        chunk_list_0_12_str.as_str(),
    )
    .await
    .expect("put chunk_list to ndn-mgr failed");

    let file4 = FileObject::new(
        "ndn_local_file_chunklist_delta_v4".to_string(),
        file4_len,
        chunk_list_0_12_id.to_string(),
    );

    let (file4_id, file4_str) = file4.gen_obj_id();
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file4_id, file4_str.as_str())
        .await
        .expect("put file4 to ndn-mgr failed");

    info!("file3_id: {}", file3_id.to_string());

    let file4_content_url = format!("http://{}/ndn/{}/content", ndn_host, file4_id.to_string());
    let (mut reader, resp_headers) = ndn_client
        .open_chunk_reader_by_url(file4_content_url.as_str(), None, None)
        .await
        .expect("open file3 content reader failed");

    let content_len = resp_headers
        .obj_size
        .expect("content-length should exist in http-headers");
    assert_eq!(
        content_len, file4_len,
        "content-length in http-header should equal with read_len"
    );
    assert_eq!(
        resp_headers.obj_id,
        Some(chunk_list_0_12_id.clone()),
        "obj-id in http-header should equal with chunk-id"
    );
    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file4_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );

    let mut buffer = vec![0u8, 0];
    let len = reader
        .read_to_end(&mut buffer)
        .await
        .expect("read chunk failed");
    assert_eq!(
        len as u64, content_len,
        "length of data in http-body should equal with content-length"
    );
    assert_eq!(
        len,
        buffer.len(),
        "length of read data should equal with content-length"
    );

    let mut pos = 0;
    for chunk_data in [
        chunks_0_3
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
        chunks_3_6
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
        chunks_6_9
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
        chunks_9_12
            .iter()
            .map(|(_, d)| d.as_slice())
            .collect::<Vec<_>>(),
    ]
    .concat()
    {
        let chunk_len = chunk_data.len() as u64;
        assert_eq!(
            &buffer.as_slice()[pos as usize..(pos + chunk_len as u64) as usize],
            chunk_data,
            "chunk range mismatch for chunk_id",
        );
        pos += chunk_len;
    }

    info!("ndn_local_file_chunklist_delta test end.");
}
