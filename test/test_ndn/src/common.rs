use std::io::SeekFrom;

use cyfs_gateway_lib::WarpServerConfig;
use cyfs_warp::start_cyfs_warp_server;
use hex::ToHex;
use jsonwebtoken::EncodingKey;
use log::*;
use ndn_lib::*;
use rand::{Rng, RngCore};
use serde_json::*;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

pub const LOCAL_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;

pub const NODE_B_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;

pub const TEST_DIR: &str = "ndn-test";
pub const DOWNLOAD_DIR: &str = "download";

pub fn generate_random_bytes(size: u64) -> Vec<u8> {
    let mut rng = rand::rng();
    let mut buffer = vec![0u8; size as usize];
    rng.fill_bytes(&mut buffer);
    buffer
}

pub fn generate_random_chunk(size: u64) -> (ChunkId, Vec<u8>) {
    let chunk_data = generate_random_bytes(size);
    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(&chunk_data);
    let chunk_id = ChunkId::from_sha256_result(&hash);
    info!("chunk_id: {}", chunk_id.to_string());
    (chunk_id, chunk_data)
}

pub async fn write_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId, chunk_data: &[u8]) {
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

pub async fn read_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId) -> Vec<u8> {
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

pub type NdnServerHost = String;

pub async fn init_local_ndn_server(ndn_mgr_id: &str) -> (NdnClient, NdnServerHost) {
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

pub async fn init_ndn_client(
    ndn_mgr_id: &str,
    private_key: &str,
    target_ndn_host: &str,
) -> NdnClient {
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

pub fn generate_random_obj() -> (ObjId, serde_json::Value) {
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
    let (obj_id, _obj_str) = build_named_object_by_json("non-test-obj", &test_obj);
    (obj_id, test_obj)
}

pub fn generate_random_file_obj_with_len(
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

pub fn generate_random_file_obj() -> (ObjId, FileObject, ChunkId, Vec<u8>) {
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

pub async fn check_file_obj(
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
                Err(err) => assert!(
                    false,
                    "get file object {:?} failed, err: {:?}",
                    file_obj_id, err
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
                    let (_unexpect_obj_id, unexpect_obj_str) = unexpect_value.gen_obj_id();
                    let (_got_obj_id, got_obj_str) =
                        build_named_object_by_json(OBJ_TYPE_FILE, got_obj);

                    assert_ne!(
                        got_obj_str, unexpect_obj_str,
                        "file-obj check failed same as unexpect",
                    );
                }
                Err(err) => assert!(false, "get file-object {:?}, {:?}", file_obj_id, err),
            },
            None => assert!(got_ret.is_ok(), "get object {:?} failed", file_obj_id,),
        }
    }
}

pub async fn pub_object_to_file_with_str(
    ndn_mgr_id: &str,
    obj_path: &str,
    obj_id: &ObjId,
    obj_str: &str,
) {
    NamedDataMgr::put_object(Some(ndn_mgr_id), obj_id, obj_str)
        .await
        .expect("put object in local failed");
    NamedDataMgr::create_file(
        Some(ndn_mgr_id),
        obj_path,
        obj_id,
        "test_non_file_obj_app_id",
        "test_non_file_obj_user_id",
    )
    .await
    .expect("create file failed");
}