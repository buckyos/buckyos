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

const TEST_DIR: &str = "ndn-test";
const DOWNLOAD_DIR: &str = "download";

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

fn generate_random_file_obj_with_len(
    name_len: u32,
    content_len: u64,
) -> (ObjId, FileObject, ChunkId, Vec<u8>) {
    let mut buffer = vec![0u8; name_len as usize];
    {
        let mut rng = rand::rng();
        rng.fill_bytes(&mut buffer);
    }
    let name = buffer.encode_hex();

    let (chunk_id, chunk_data) = generate_random_chunk(content_len as u64);

    let obj = FileObject::new(name, content_len as u64, chunk_id.to_string());

    (obj.gen_obj_id().0, obj, chunk_id, chunk_data)
}

fn generate_random_file_obj() -> (ObjId, FileObject, ChunkId, Vec<u8>) {
    let name_len = {
        let mut rng = rand::rng();
        rng.random_range(15u32..31u32)
    };
    let content_len = {
        let mut rng = rand::rng();
        rng.random_range(
            0u32..(5 * 1024 * 1024u32 + {
                let mut rng2 = rand::rng();
                rng2.random_range(0u32..1024 * 1024u32)
            }),
        )
    };

    generate_random_file_obj_with_len(name_len, content_len as u64)
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

async fn check_file_obj(
    ndn_mgr_id: &str,
    file_obj_id: &ObjId,
    expect_value: Option<Option<&FileObject>>,
    unexpect_value: Option<Option<&FileObject>>,
) {
    let got_ret = NamedDataMgr::get_object(Some(ndn_mgr_id), file_obj_id, None).await;

    if let Some(expect_value) = &expect_value {
        match expect_value {
            Some(expect_value) => match &got_ret {
                Ok(got_obj) => {
                    let (expect_obj_id, expect_obj_str) = expect_value.gen_obj_id();
                    let (got_obj_id, got_obj_str) =
                        build_named_object_by_json(OBJ_TYPE_FILE, got_obj);

                    assert_eq!(
                        got_obj_id, expect_obj_id,
                        "got file-id should be same as file-id"
                    );
                    assert_eq!(
                        got_obj_str, expect_obj_str,
                        "got file obj json-str should be same as expect"
                    );

                    // 检查FileObject里所有字段都完全匹配
                    // got_obj 是 serde_json::Value，需要转为 FileObject
                    let got_file_obj: FileObject = serde_json::from_value(got_obj.clone())
                        .expect("deserialize got_obj to FileObject failed");
                    // 参照FileObject结构原型逐个字段断言got_file_obj和expect_value
                    assert_eq!(
                        got_file_obj.name, expect_value.name,
                        "FileObject.name mismatch"
                    );
                    assert_eq!(
                        got_file_obj.size, expect_value.size,
                        "FileObject.size mismatch"
                    );
                    assert_eq!(
                        got_file_obj.content, expect_value.content,
                        "FileObject.content mismatch"
                    );
                    assert_eq!(
                        got_file_obj.exp, expect_value.exp,
                        "FileObject.exp mismatch"
                    );
                    assert_eq!(
                        got_file_obj.meta, expect_value.meta,
                        "FileObject.meta mismatch"
                    );
                    assert_eq!(
                        got_file_obj.mime, expect_value.mime,
                        "FileObject.mime mismatch"
                    );
                    assert_eq!(
                        got_file_obj.owner, expect_value.owner,
                        "FileObject.owner mismatch"
                    );
                    assert_eq!(
                        got_file_obj.create_time, expect_value.create_time,
                        "FileObject.create_time mismatch"
                    );
                    assert_eq!(
                        got_file_obj.chunk_list, expect_value.chunk_list,
                        "FileObject.chunk_list mismatch"
                    );
                    assert_eq!(
                        got_file_obj.links, expect_value.links,
                        "FileObject.links mismatch"
                    );
                    assert_eq!(
                        got_file_obj.extra_info, expect_value.extra_info,
                        "FileObject.extra_info mismatch"
                    );
                }
                Err(err) => assert!(false, "get file object {:?} failed", file_obj_id),
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
                    let (_unexpect_obj_id, unexpect_obj_str) = unexpect_value.gen_obj_id();
                    let (_got_obj_id, got_obj_str) =
                        build_named_object_by_json(OBJ_TYPE_FILE, got_obj);

                    assert_ne!(
                        got_obj_str, unexpect_obj_str,
                        "file-obj check failed same as unexpect",
                    );
                }
                Err(err) => assert!(false, "get file-object {:?}", file_obj_id),
            },
            None => assert!(got_ret.is_ok(), "get object {:?} failed", file_obj_id,),
        }
    }
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
        .join(TEST_DIR)
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

#[tokio::test]
async fn ndn_local_file_ok() {
    init_logging("ndn_local_file_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

    write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
    assert_eq!(file_id, cal_file_id, "file-id mismatch");

    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
        .await
        .expect("put object in local failed");

    check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(Some(&file_obj)), None).await;

    let buffer = read_chunk(ndn_mgr_id.as_str(), &chunk_id).await;
    assert_eq!(buffer, chunk_data, "file chunk-content check failed");
}

#[tokio::test]
async fn ndn_local_file_not_found() {
    init_logging("ndn_local_file_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    {
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(None), None).await;

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

    {
        // write chunk only
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(None), None).await;

        let buffer = read_chunk(ndn_mgr_id.as_str(), &chunk_id).await;
        assert_eq!(buffer, chunk_data, "file chunk-content check failed");
    }

    {
        // put file-obj only
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(Some(&file_obj)), None).await;

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
}

#[tokio::test]
async fn ndn_local_file_verify_failed() {
    init_logging("ndn_local_file_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // fake file.content
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        write_chunk(
            ndn_mgr_id.as_str(),
            &fake_chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        check_file_obj(
            ndn_mgr_id.as_str(),
            &file_id,
            Some(Some(&fake_file_obj)),
            Some(Some(&file_obj)),
        )
        .await;

        let buffer = read_chunk(ndn_mgr_id.as_str(), &fake_chunk_id).await;
        assert_eq!(buffer, fake_chunk_data, "file chunk-content check failed");

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

    {
        // fake file.name
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        check_file_obj(
            ndn_mgr_id.as_str(),
            &file_id,
            Some(Some(&fake_file_obj)),
            Some(Some(&file_obj)),
        )
        .await;

        let buffer = read_chunk(ndn_mgr_id.as_str(), &chunk_id).await;
        assert_eq!(buffer, chunk_data, "file chunk-content check failed");
    }

    {
        // fake chunk
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(Some(&file_obj)), None).await;

        let buffer = read_chunk(ndn_mgr_id.as_str(), &chunk_id).await;
        assert_eq!(buffer, fake_chunk_data, "file chunk-content check failed");
    }
}

// http://{host}/ndn/{obj-id}/inner-path
#[tokio::test]
async fn ndn_local_o_link_innerpath_file_ok() {
    init_logging("ndn_local_o_link_innerpath_file_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let ndn_url = format!("http://{}/ndn/", ndn_host);

    {
        // 1. get chunk of file
        // 2. get name of file
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let (mut reader, resp_headers) = ndn_client
            .open_chunk_reader_by_url(o_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await
            .expect("open chunk-reader failed");

        let content_len = resp_headers
            .obj_size
            .expect("content-length should exist in http-headers");
        assert_eq!(
            content_len,
            chunk_data.len() as u64,
            "content-length in http-header should equal with chunk.len"
        );
        assert_eq!(
            resp_headers.obj_id,
            Some(chunk_id.to_obj_id()),
            "obj-id in http-header should equal with chunk-id"
        );
        assert!(
            resp_headers.path_obj.is_none(),
            "path-obj should be None for o-link"
        );
        assert_eq!(
            resp_headers.root_obj_id,
            Some(file_id.clone()),
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
        assert_eq!(buffer, chunk_data, "chunk content mismatch");

        // todo: verify chunk with mtree

        let o_link_inner_path = format!("http://{}/ndn/{}/name", ndn_host, file_id.to_string(),);
        let (name_obj_id, name_json) = ndn_client
            .get_obj_by_url(o_link_inner_path.as_str(), None)
            .await
            .expect("get name of file with o-link failed");

        let name = name_json.as_str().expect("name should be string");
        assert_eq!(name, file_obj.name.as_str(), "name mismatch");
    }

    {
        // 1. get name of file
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/name", ndn_host, file_id.to_string());
        let (name_obj_id, name_json) = ndn_client
            .get_obj_by_url(o_link_inner_path.as_str(), None)
            .await
            .expect("get name of file with o-link failed");

        let name = name_json.as_str().expect("name should be string");
        assert_eq!(name, file_obj.name.as_str(), "name mismatch");
    }

    {
        // 1. get chunk range by range
        let (file_id, file_obj, chunk_id, chunk_data) =
            generate_random_file_obj_with_len(16, 5 * 1024 * 1024);

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let mut read_pos = 0;
        let mut read_buffers = vec![];

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string());

        loop {
            let read_len = {
                let mut rng = rand::rng();
                rng.random_range(1u64..chunk_data.len() as u64 - read_pos)
            };
            let end_pos = read_pos + read_len;

            let (mut reader, resp_headers) = ndn_client
                .open_chunk_reader_by_url(
                    o_link_inner_path.as_str(),
                    Some(chunk_id.clone()),
                    Some(read_pos..end_pos),
                )
                .await
                .expect("open chunk-reader failed");

            let content_len = resp_headers
                .obj_size
                .expect("content-length should exist in http-headers");
            assert_eq!(
                content_len, read_len,
                "content-length in http-header should equal with read_len"
            );
            assert_eq!(
                resp_headers.obj_id,
                Some(chunk_id.to_obj_id()),
                "obj-id in http-header should equal with chunk-id"
            );
            assert!(
                resp_headers.path_obj.is_none(),
                "path-obj should be None for o-link"
            );
            assert_eq!(
                resp_headers.root_obj_id,
                Some(file_id.clone()),
                "root-obj-id in http-header should equal with file-id"
            );

            let mut buffer = vec![0u8, 0];
            let len = reader
                .read_to_end(&mut buffer)
                .await
                .expect("read chunk failed");
            assert_eq!(
                len as u64, read_len,
                "length of data in http-body should equal with content-length"
            );
            assert_eq!(
                len,
                buffer.len(),
                "length of read data should equal with content-length"
            );
            assert_eq!(
                buffer.as_slice(),
                &chunk_data.as_slice()[read_pos as usize..end_pos as usize],
                "chunk range mismatch"
            );
            read_buffers.push(buffer);

            // todo: verify chunk with mtree

            read_pos += read_len;
            if read_pos >= chunk_data.len() as u64 {
                break;
            }
        }

        let read_chunk = read_buffers.concat();
        assert_eq!(read_chunk, chunk_data, "chunk data mismatch");
    }

    {
        // download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await
            .expect("download chunk to local failed");

        assert_eq!(
            download_chunk_id, chunk_id,
            "download chunk-id should equal with chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            chunk_data.len() as u64,
            "download chunk-size should equal with chunk-data len"
        );

        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, chunk_data,
            "should be same as chunk-content"
        );

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await
            .expect("download chunk should success without verify");

        assert_eq!(
            download_chunk_id, chunk_id,
            "download chunk-id should equal with chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            chunk_data.len() as u64,
            "download chunk-size should equal with chunk-data len"
        );

        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, chunk_data,
            "should be same as chunk-content"
        );
    }
}

#[tokio::test]
async fn ndn_local_o_link_innerpath_file_not_found() {
    init_logging("ndn_local_o_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // no chunk saved
        // 1. get chunk of file
        // 2. get name of file
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .open_chunk_reader_by_url(o_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "chunk should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(false, "unexpect error, chunk should not found. {:?}", err)
                }
            }
        }

        let o_link_inner_path =
            format!("http://{}/ndn/{}/notexist", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .get_obj_by_url(o_link_inner_path.as_str(), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "notexist field should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(
                        false,
                        "unexpect error, notexist field should not found. {:?}",
                        err
                    )
                }
            }
        }
    }

    {
        // no write chunk for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "notexist field should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(
                        false,
                        "unexpect error, notexist field should not found. {:?}",
                        err
                    )
                }
            }
        }

        assert!(
            !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
            "chunk should removed for verify failed"
        );

        let ret = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "notexist field should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(
                        false,
                        "unexpect error, notexist field should not found. {:?}",
                        err
                    )
                }
            }
        }
    }

    {
        // field not exist
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path =
            format!("http://{}/ndn/{}/notexist", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .get_obj_by_url(o_link_inner_path.as_str(), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "notexist field should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(
                        false,
                        "unexpect error, notexist field should not found. {:?}",
                        err
                    )
                }
            }
        }
    }
}

#[tokio::test]
async fn ndn_local_o_link_innerpath_file_verify_failed() {
    init_logging("ndn_local_o_link_innerpath_file_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // fake file.content
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        write_chunk(
            ndn_mgr_id.as_str(),
            &fake_chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .open_chunk_reader_by_url(o_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "chunk should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("Chunk verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }
    }

    {
        // fake file.content for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        write_chunk(
            ndn_mgr_id.as_str(),
            &fake_chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "chunk-content should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("chunk-content verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }

        assert!(
            !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
            "chunk should removed for verify failed"
        );

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await
            .expect("download chunk should success without verify");

        assert_eq!(
            download_chunk_id, fake_chunk_id,
            "should be same as fake chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            fake_chunk_data.len() as u64,
            "should be same as fake chunk.len"
        );
        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, fake_chunk_data,
            "should be same as fake chunk-content"
        );
    }

    {
        // fake chunk
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");
        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let (mut reader, resp_headers) = ndn_client
            .open_chunk_reader_by_url(o_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await
            .expect("open chunk-reader failed");

        let content_len = resp_headers
            .obj_size
            .expect("content-length should exist in http-headers");
        assert_eq!(
            content_len,
            fake_chunk_data.len() as u64,
            "content-length in http-header should equal with chunk.len"
        );
        assert_eq!(
            resp_headers.obj_id,
            Some(chunk_id.to_obj_id()),
            "obj-id in http-header should equal with chunk-id"
        );
        assert!(
            resp_headers.path_obj.is_none(),
            "path-obj should be None for o-link"
        );
        assert_eq!(
            resp_headers.root_obj_id,
            Some(file_id.clone()),
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
        assert_eq!(buffer, fake_chunk_data, "chunk content mismatch");

        // todo: verify chunk with mtree
    }

    {
        // fake chunk for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "chunk-content should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("chunk-content verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }

        assert!(
            !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
            "chunk should removed for verify failed"
        );

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await
            .expect("download chunk should success without verify");

        assert_eq!(
            download_chunk_id, fake_chunk_id,
            "should be same as fake chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            fake_chunk_data.len() as u64,
            "should be same as fake chunk.len"
        );
        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, fake_chunk_data,
            "should be same as fake chunk-content"
        );
    }

    {
        // fake file.name
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .open_chunk_reader_by_url(o_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "file-obj should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("file-obj verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }
    }

    {
        // fake file.name for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let ret = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "file-obj should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("file-obj verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }

        assert!(
            !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
            "chunk should removed for verify failed"
        );

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                o_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await
            .expect("download chunk should success without verify");

        assert_eq!(download_chunk_id, chunk_id, "should be same as chunk-id");
        assert_eq!(
            download_chunk_len,
            chunk_data.len() as u64,
            "should be same as chunk.len"
        );
        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, chunk_data,
            "should be same as chunk-content"
        );
    }
}

// http://{host}/ndn/{obj-path}/inner-path
#[tokio::test]
async fn ndn_local_r_link_innerpath_file_ok() {
    init_logging("ndn_local_r_link_innerpath_file_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let ndn_url = format!("http://{}/ndn/", ndn_host);

    {
        // 1. get chunk of file
        // 2. get name of file
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let local_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(local_path.as_path());

        // 创建local_path目录
        std::fs::create_dir_all(local_path.parent().unwrap())
            .expect("create local path parent dir failed");
        // 把chunk_data写入到local_path
        std::fs::write(local_path.as_path(), chunk_data.as_slice())
            .expect("write chunk data to local failed");

        let obj_path = "/test_file_path";
        let content_ndn_path = format!("test_file_content_{}", chunk_id.to_base32());

        let mut file_obj = FileObject::new(
            "non_test_file".to_string(),
            chunk_data.len() as u64,
            chunk_id.to_string(),
        );

        NamedDataMgr::pub_local_file_as_fileobj(
            Some(ndn_mgr_id.as_str()),
            &local_path,
            obj_path,
            content_ndn_path.as_str(),
            &mut file_obj,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        assert_eq!(
            file_obj.content,
            chunk_id.to_string(),
            "file content should be same as ndn-path"
        );
        assert_eq!(
            file_obj.size,
            chunk_data.len() as u64,
            "file content-ndn-path should be same as ndn-path"
        );

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);
        let (mut reader, resp_headers) = ndn_client
            .open_chunk_reader_by_url(r_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await
            .map_err(|err| {
                log::error!("Failed to open chunk reader: {:?}", err);
                err
            })
            .expect("open chunk-reader failed");

        let content_len = resp_headers
            .obj_size
            .expect("content-length should exist in http-headers");
        assert_eq!(
            content_len,
            chunk_data.len() as u64,
            "content-length in http-header should equal with chunk.len"
        );
        assert_eq!(
            resp_headers.obj_id,
            Some(chunk_id.to_obj_id()),
            "obj-id in http-header should equal with chunk-id"
        );
        assert!(
            resp_headers.path_obj.is_none(),
            "path-obj should be None for o-link"
        );
        assert_eq!(
            resp_headers.root_obj_id,
            Some(file_id.clone()),
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
        assert_eq!(buffer, chunk_data, "chunk content mismatch");

        // todo: verify chunk with mtree

        let r_link_inner_path = format!("http://{}/ndn/{}/name", ndn_host, obj_path);
        let (name_obj_id, name_json) = ndn_client
            .get_obj_by_url(r_link_inner_path.as_str(), None)
            .await
            .expect("get name of file with o-link failed");

        let name = name_json.as_str().expect("name should be string");
        assert_eq!(name, file_obj.name.as_str(), "name mismatch");

        std::fs::remove_file(local_path.as_path()).expect("remove local file failed");
    }

    {
        // 1. get name of file
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/name", ndn_host, file_id.to_string());
        let (name_obj_id, name_json) = ndn_client
            .get_obj_by_url(r_link_inner_path.as_str(), None)
            .await
            .expect("get name of file with o-link failed");

        let name = name_json.as_str().expect("name should be string");
        assert_eq!(name, file_obj.name.as_str(), "name mismatch");
    }

    {
        // 1. get chunk range by range
        let (file_id, file_obj, chunk_id, chunk_data) =
            generate_random_file_obj_with_len(16, 5 * 1024 * 1024);

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);

        let mut read_pos = 0;
        let mut read_buffers = vec![];

        loop {
            let read_len = {
                let mut rng = rand::rng();
                rng.random_range(1u64..chunk_data.len() as u64 - read_pos)
            };
            let end_pos = read_pos + read_len;

            let (mut reader, resp_headers) = ndn_client
                .open_chunk_reader_by_url(
                    r_link_inner_path.as_str(),
                    Some(chunk_id.clone()),
                    Some(read_pos..end_pos),
                )
                .await
                .expect("open chunk-reader failed");

            let content_len = resp_headers
                .obj_size
                .expect("content-length should exist in http-headers");
            assert_eq!(
                content_len, read_len,
                "content-length in http-header should equal with read_len"
            );
            assert_eq!(
                resp_headers.obj_id,
                Some(chunk_id.to_obj_id()),
                "obj-id in http-header should equal with chunk-id"
            );
            assert!(
                resp_headers.path_obj.is_none(),
                "path-obj should be None for o-link"
            );
            assert_eq!(
                resp_headers.root_obj_id,
                Some(file_id.clone()),
                "root-obj-id in http-header should equal with file-id"
            );

            let mut buffer = vec![0u8, 0];
            let len = reader
                .read_to_end(&mut buffer)
                .await
                .expect("read chunk failed");
            assert_eq!(
                len as u64, read_len,
                "length of data in http-body should equal with content-length"
            );
            assert_eq!(
                len,
                buffer.len(),
                "length of read data should equal with content-length"
            );
            assert_eq!(
                buffer.as_slice(),
                &chunk_data.as_slice()[read_pos as usize..end_pos as usize],
                "chunk range mismatch"
            );
            read_buffers.push(buffer);

            // todo: verify chunk with mtree

            read_pos += read_len;
            if read_pos >= chunk_data.len() as u64 {
                break;
            }
        }

        let read_chunk = read_buffers.concat();
        assert_eq!(read_chunk, chunk_data, "chunk data mismatch");
    }

    {
        // download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);
        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await
            .expect("download chunk to local failed");

        assert_eq!(
            download_chunk_id, chunk_id,
            "download chunk-id should equal with chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            chunk_data.len() as u64,
            "download chunk-size should equal with chunk-data len"
        );

        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, chunk_data,
            "should be same as chunk-content"
        );

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await
            .expect("download chunk should success without verify");

        assert_eq!(
            download_chunk_id, chunk_id,
            "download chunk-id should equal with chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            chunk_data.len() as u64,
            "download chunk-size should equal with chunk-data len"
        );

        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, chunk_data,
            "should be same as chunk-content"
        );

        std::fs::remove_file(download_path.as_path()).expect("remove download chunk file failed");
    }
}

#[tokio::test]
async fn ndn_local_r_link_innerpath_file_not_found() {
    init_logging("ndn_local_r_link_innerpath_file_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // no chunk saved
        // 1. get chunk of file
        // 2. get name of file
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);
        let ret = ndn_client
            .open_chunk_reader_by_url(r_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "chunk should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(false, "unexpect error, chunk should not found. {:?}", err)
                }
            }
        }

        let r_link_inner_path = format!("http://{}/ndn/{}/notexist", ndn_host, obj_path);
        let ret = ndn_client
            .get_obj_by_url(r_link_inner_path.as_str(), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "notexist field should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(
                        false,
                        "unexpect error, notexist field should not found. {:?}",
                        err
                    )
                }
            }
        }
    }

    {
        // no write chunk for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let ret = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "notexist field should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(
                        false,
                        "unexpect error, notexist field should not found. {:?}",
                        err
                    )
                }
            }
        }

        assert!(
            !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
            "chunk should removed for verify failed"
        );

        let ret = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "notexist field should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(
                        false,
                        "unexpect error, notexist field should not found. {:?}",
                        err
                    )
                }
            }
        }
    }

    {
        // field not exist
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/notexist", ndn_host, obj_path,);

        let ret = ndn_client
            .get_obj_by_url(r_link_inner_path.as_str(), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "notexist field should not found"),
            Err(err) => {
                if let NdnError::NotFound(_) = err {
                } else {
                    assert!(
                        false,
                        "unexpect error, notexist field should not found. {:?}",
                        err
                    )
                }
            }
        }
    }
}

#[tokio::test]
async fn ndn_local_r_link_innerpath_file_verify_failed() {
    init_logging("ndn_local_r_link_innerpath_file_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // fake file.content
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        write_chunk(
            ndn_mgr_id.as_str(),
            &fake_chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id should not match");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);

        let ret = ndn_client
            .open_chunk_reader_by_url(r_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "chunk should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("Chunk verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }
    }

    {
        // fake file.content for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        write_chunk(
            ndn_mgr_id.as_str(),
            &fake_chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let ret = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "chunk-content should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("chunk-content verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }

        assert!(
            !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
            "chunk should removed for verify failed"
        );

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await
            .expect("download chunk should success without verify");

        assert_eq!(
            download_chunk_id, fake_chunk_id,
            "should be same as fake chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            fake_chunk_data.len() as u64,
            "should be same as fake chunk.len"
        );
        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, fake_chunk_data,
            "should be same as fake chunk-content"
        );
    }

    {
        // fake chunk
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);

        let (mut reader, resp_headers) = ndn_client
            .open_chunk_reader_by_url(r_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await
            .expect("open chunk-reader failed");

        let content_len = resp_headers
            .obj_size
            .expect("content-length should exist in http-headers");
        assert_eq!(
            content_len,
            fake_chunk_data.len() as u64,
            "content-length in http-header should equal with chunk.len"
        );
        assert_eq!(
            resp_headers.obj_id,
            Some(chunk_id.to_obj_id()),
            "obj-id in http-header should equal with chunk-id"
        );
        assert!(
            resp_headers.path_obj.is_none(),
            "path-obj should be None for o-link"
        );
        assert_eq!(
            resp_headers.root_obj_id,
            Some(file_id.clone()),
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
        assert_eq!(buffer, fake_chunk_data, "chunk content mismatch");

        // todo: verify chunk with mtree
    }

    {
        // fake chunk for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let ret = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "chunk-content should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("chunk-content verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }

        assert!(
            !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
            "chunk should removed for verify failed"
        );

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await
            .expect("download chunk should success without verify");

        assert_eq!(
            download_chunk_id, fake_chunk_id,
            "should be same as fake chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            fake_chunk_data.len() as u64,
            "should be same as fake chunk.len"
        );
        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, fake_chunk_data,
            "should be same as fake chunk-content"
        );
    }

    {
        // fake file.name
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);
        let ret = ndn_client
            .open_chunk_reader_by_url(r_link_inner_path.as_str(), Some(chunk_id.clone()), None)
            .await;

        match ret {
            Ok(_) => assert!(false, "file-obj should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("file-obj verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }
    }

    {
        // fake file.name for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path";
        NamedDataMgr::pub_object_to_file(
            Some(ndn_mgr_id.as_str()),
            serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
            OBJ_TYPE_FILE,
            obj_path,
            "test_non_file_obj_user_id",
            "test_non_file_obj_app_id",
        )
        .await
        .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, obj_path,);

        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());

        let ret = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(true),
            )
            .await;

        match ret {
            Ok(_) => assert!(false, "file-obj should verify error"),
            Err(err) => match err {
                NdnError::VerifyError(_) => {
                    info!("file-obj verify error as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type");
                }
            },
        }

        assert!(
            !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
            "chunk should removed for verify failed"
        );

        let (download_chunk_id, download_chunk_len) = ndn_client
            .download_chunk_to_local(
                r_link_inner_path.as_str(),
                chunk_id.clone(),
                &download_path,
                Some(false),
            )
            .await
            .expect("download chunk should success without verify");

        assert_eq!(download_chunk_id, chunk_id, "should be same as chunk-id");
        assert_eq!(
            download_chunk_len,
            chunk_data.len() as u64,
            "should be same as chunk.len"
        );
        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            download_chunk, chunk_data,
            "should be same as chunk-content"
        );
    }
}
