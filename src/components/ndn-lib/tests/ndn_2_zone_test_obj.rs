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
    let (obj_id, _obj_str) = build_named_object_by_json("non-test-obj", &test_obj);
    (obj_id, test_obj)
}

async fn check_obj_inner_path(
    ndn_client: &NdnClient,
    url: &str,
    obj_id: &ObjId,
    obj_type: &str,
    inner_path: Option<&str>,
    expect_value: Option<Option<&serde_json::Value>>,
    unexpect_value: Option<Option<&serde_json::Value>>,
    expect_obj_id: Option<&ObjId>,
) {
    let got_ret = ndn_client
        .get_obj_by_url(url, expect_obj_id.map(|id| id.clone()))
        .await;

    if let Some(expect_value) = &expect_value {
        match expect_value {
            Some(expect_value) => match &got_ret {
                Ok((got_obj_id, got_obj)) => {
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
                Ok((_, got_obj)) => {
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
                Ok((got_obj_id, got_obj)) => {
                    let (_unexpect_obj_id, unexpect_obj_str) =
                        build_named_object_by_json(obj_type, *unexpect_value);
                    let (got_obj_id_cal, got_obj_str) =
                        build_named_object_by_json(obj_type, got_obj);
                    assert_eq!(got_obj_id, &got_obj_id_cal, "got obj-id mismatch");
                    if inner_path.is_none() {
                        assert_eq!(
                            got_obj_id,
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
async fn ndn_2_zone_object_ok() {
    init_logging("ndn_2_zone_object_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // 2. push the object using the NdnClient to zone_a
    let (got_obj_id, got_obj) = zone_a_client
        .get_obj_by_url(
            format!(
                "http://{}/ndn/{}",
                local_ndn_server_host,
                obj_id.to_base32()
            )
            .as_str(),
            Some(obj_id.clone()),
        )
        .await
        .expect("push object from ndn-mgr failed");

    assert_eq!(got_obj_id, obj_id, "got obj-id to zone-a mismatch");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    assert_eq!(got_obj_str, obj_str, "got obj to zone-a mismatch");

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let (got_obj_id, got_obj) = zone_b_client
        .get_obj_by_url(
            format!("http://test.buckyos.io/ndn/{}", obj_id.to_base32()).as_str(),
            Some(obj_id.clone()),
        )
        .await
        .expect("get obj from zone-a failed");

    assert_eq!(got_obj_id, obj_id, "got obj-id from zone-a mismatch");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    assert_eq!(got_obj_str, obj_str, "got obj from zone-a mismatch");
}

#[tokio::test]
async fn ndn_2_zone_object_not_found() {
    init_logging("ndn_2_zone_object_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // 2. push the object using the NdnClient to zone_a
    // let (got_obj_id, got_obj) = zone_a_client
    //     .get_obj_by_url(
    //         format!(
    //             "http://{}/ndn/{}",
    //             local_ndn_server_host,
    //             obj_id.to_base32()
    //         )
    //         .as_str(),
    //         Some(obj_id.clone()),
    //     )
    //     .await
    //     .expect("push object from ndn-mgr failed");

    // assert_eq!(got_obj_id, obj_id, "got obj-id to zone-a mismatch");

    // let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    // assert_eq!(got_obj_str, obj_str, "got obj to zone-a mismatch");

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let (got_obj_id, got_obj) = zone_b_client
        .get_obj_by_url(
            format!("http://test.buckyos.io/ndn/{}", obj_id.to_base32()).as_str(),
            Some(obj_id.clone()),
        )
        .await
        .expect("get obj from zone-a failed");

    check_obj_inner_path(
        &zone_b_client,
        format!("http://test.buckyos.io/ndn/{}", obj_id.to_base32()).as_str(),
        &obj_id,
        "non-test-obj",
        None,
        Some(None),
        None,
        None,
    )
    .await;
}

#[tokio::test]
async fn ndn_2_zone_object_verify_failed() {
    init_logging("ndn_2_zone_object_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();
    let (fake_obj_id, fake_obj) = generate_random_obj();

    let (_, fake_obj_str) = build_named_object_by_json("non-test-obj", &fake_obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, fake_obj_str.as_str())
        .await
        .expect("put object in local failed");

    // 2. push the object using the NdnClient to zone_a
    let ret = zone_a_client
        .get_obj_by_url(
            format!(
                "http://{}/ndn/{}",
                local_ndn_server_host,
                obj_id.to_base32()
            )
            .as_str(),
            Some(obj_id.clone()),
        )
        .await;

    match ret {
        Ok(_) => assert!(false, "should obj id verify failed"),
        Err(err) => {
            if let NdnError::InvalidId(_) = err {
            } else {
                assert!(false, "unexpect error, should obj id verify failed.")
            }
        }
    }
}

// http://{host}/ndn/{obj-id}/inner-path
#[tokio::test]
async fn ndn_2_zone_o_link_innerpath_ok() {
    init_logging("ndn_2_zone_o_link_innerpath_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // 2. push the object using the NdnClient to zone_a
    // get object using the NdnClient
    let inner_path = "obj";
    let o_link_inner_path = format!(
        "http://{}/ndn/{}/{}",
        local_ndn_server_host, obj_id_base32, inner_path
    );

    let (got_obj_id, got_obj) = zone_a_client
        .get_obj_by_url(o_link_inner_path.as_str(), Some(obj_id.clone()))
        .await
        .expect("push object from ndn-mgr failed");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    let (_, expect_obj_str) =
        build_named_object_by_json("non-test-obj", obj.get(inner_path).unwrap());
    assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let o_link_inner_path = format!(
        "http://test.buckyos.io/ndn/{}/{}",
        obj_id_base32, inner_path
    );
    let (got_obj_id, got_obj) = zone_b_client
        .get_obj_by_url(o_link_inner_path.as_str(), Some(obj_id.clone()))
        .await
        .expect("get obj from zone-a failed");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    let (_, expect_obj_str) =
        build_named_object_by_json("non-test-obj", obj.get(inner_path).unwrap());
    assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

#[tokio::test]
async fn ndn_2_zone_o_link_innerpath_not_found() {
    init_logging("ndn_2_zone_o_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // 2. push the object using the NdnClient to zone_a
    // get object using the NdnClient
    let inner_path = "notexist";
    let o_link_inner_path = format!(
        "http://{}/ndn/{}/{}",
        local_ndn_server_host, obj_id_base32, inner_path
    );

    let ret = zone_a_client
        .get_obj_by_url(o_link_inner_path.as_str(), Some(obj_id.clone()))
        .await;

    match ret {
        Ok(_) => assert!(false, "should obj id verify failed"),
        Err(err) => {
            if let NdnError::InvalidId(_) = err {
            } else {
                assert!(false, "unexpect error, should obj id verify failed.")
            }
        }
    }

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let o_link_inner_path = format!(
        "http://test.buckyos.io/ndn/{}/{}",
        obj_id_base32, inner_path
    );
    let ret = zone_b_client
        .get_obj_by_url(o_link_inner_path.as_str(), Some(obj_id.clone()))
        .await;

    match ret {
        Ok(_) => assert!(false, "should obj id verify failed"),
        Err(err) => {
            if let NdnError::InvalidId(_) = err {
            } else {
                assert!(false, "unexpect error, should obj id verify failed.")
            }
        }
    }
}

#[tokio::test]
async fn ndn_2_zone_o_link_innerpath_verify_failed() {
    init_logging("ndn_2_zone_o_link_innerpath_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, mut obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    // modify 'obj.string'.
    obj.as_object_mut().unwrap().insert(
        "string".to_string(),
        serde_json::Value::String("fake string".to_string()),
    );

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // 2. push the object using the NdnClient to zone_a
    // get object using the NdnClient
    let inner_path = "string";
    let o_link_inner_path = format!(
        "http://{}/ndn/{}/{}",
        local_ndn_server_host, obj_id_base32, inner_path
    );

    let ret = zone_a_client
        .get_obj_by_url(o_link_inner_path.as_str(), Some(obj_id.clone()))
        .await;

    match ret {
        Ok(_) => assert!(false, "sub obj id should not found"),
        Err(err) => {
            if let NdnError::NotFound(_) = err {
            } else {
                assert!(
                    false,
                    "unexpect error, sub obj id should not found. {:?}",
                    err
                )
            }
        }
    }
}

// http://{obj-id}.{host}/ndn/{obj-id}/inner-path
#[tokio::test]
async fn ndn_2_zone_o_link_in_host_innerpath_ok() {
    // init_logging("ndn_2_zone_o_link_innerpath_ok", false);

    // let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    // let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    // // let session_token = kRPC::RPCSessionToken {
    // //     token_type: kRPC::RPCSessionTokenType::JWT,
    // //     token: None,
    // //     appid: Some("ndn".to_string()),
    // //     exp: Some(
    // //         std::time::SystemTime::now()
    // //             .duration_since(std::time::UNIX_EPOCH)
    // //             .unwrap()
    // //             .as_secs()
    // //             + 3600 * 24 * 7,
    // //     ),
    // //     iss: None,
    // //     nonce: None,
    // //     userid: None,
    // // };

    // //     let private_key = {
    // //         let private_key_pem = r#"
    // // -----BEGIN PRIVATE KEY-----
    // // MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
    // // -----END PRIVATE KEY-----
    // // "#;
    // //         EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap()
    // //     };

    // //     let target_ndn_host = "bob.web3.buckyos.io";
    // //     let target_ndn_client = NdnClient::new(
    // //         format!("http://{}/ndn/", target_ndn_host),
    // //         Some(
    // //             session_token
    // //                 .generate_jwt(None, &private_key)
    // //                 .expect("generate jwt failed."),
    // //         ),
    // //         Some(ndn_mgr_id.clone()),
    // //     );
    // let ndn_url = format!("http://{}/ndn/", ndn_host);

    // let (obj_id, obj) = generate_random_obj();
    // let obj_id_base32 = obj_id.to_base32();

    // let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    // NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
    //     .await
    //     .expect("put object in local failed");

    // // get object using the NdnClient
    // let inner_path = "obj";
    // let o_link_inner_path = format!("http://{}.{}/ndn/{}", obj_id_base32, ndn_host, inner_path);
    // let (got_obj_id, got_obj) = ndn_client
    //     .get_obj_by_url(o_link_inner_path.as_str(), None)
    //     .await
    //     .expect("get obj from ndn-mgr failed");

    // // assert_eq!(got_obj_id, obj_id, "got obj-id mismatch");

    // let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    // let (_, expect_obj_str) =
    //     build_named_object_by_json("non-test-obj", obj.get(inner_path).unwrap());
    // assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

#[tokio::test]
async fn ndn_2_zone_o_link_in_host_innerpath_not_found() {
    unimplemented!()
}

#[tokio::test]
async fn ndn_2_zone_o_link_in_host_innerpath_verify_failed() {
    unimplemented!()
}

// http://{host}/ndn/{obj-path}
#[tokio::test]
async fn ndn_2_zone_r_link_ok() {
    init_logging("ndn_2_zone_r_link_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let obj_path = "/test_obj_path";
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path,
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // 2. push the object using the NdnClient to zone_a
    // get object using the NdnClient
    let r_link = format!("http://{}/ndn{}", local_ndn_server_host, obj_path);

    let (got_obj_id, got_obj) = zone_a_client
        .get_obj_by_url(r_link.as_str(), Some(obj_id.clone()))
        .await
        .expect("push object from ndn-mgr failed");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    let (_, expect_obj_str) = build_named_object_by_json("non-test-obj", &obj);
    assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let r_link = format!("http://test.buckyos.io/ndn{}", obj_path);
    let (got_obj_id, got_obj) = zone_b_client
        .get_obj_by_url(r_link.as_str(), Some(obj_id.clone()))
        .await
        .expect("get obj from zone-a failed");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    let (_, expect_obj_str) = build_named_object_by_json("non-test-obj", &obj);
    assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

#[tokio::test]
async fn ndn_2_zone_r_link_not_found() {
    init_logging("ndn_2_zone_r_link_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();

    let obj_path = "/test_obj_path";
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path,
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // no get object using the NdnClient
    // let r_link = format!("http://{}/ndn{}", local_ndn_server_host, obj_path);
    // let ret = zone_a_client
    //     .get_obj_by_url(r_link.as_str(), Some(obj_id.clone()))
    //     .await;

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let r_link = format!("http://test.buckyos.io/ndn{}", obj_path);
    let ret = zone_b_client
        .get_obj_by_url(r_link.as_str(), Some(obj_id.clone()))
        .await;

    match ret {
        Ok(_) => assert!(false, "sub obj id should not found"),
        Err(err) => {
            if let NdnError::NotFound(_) = err {
            } else {
                assert!(
                    false,
                    "unexpect error, sub obj id should not found. {:?}",
                    err
                )
            }
        }
    }
}

#[tokio::test]
async fn ndn_2_zone_r_link_verify_failed() {
    init_logging("ndn_2_zone_r_link_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, mut obj) = generate_random_obj();

    // modify 'obj.string'.
    obj.as_object_mut().unwrap().insert(
        "string".to_string(),
        serde_json::Value::String("fake string".to_string()),
    );

    let obj_path = "/test_obj_path";
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path,
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // 2. push the object using the NdnClient to zone_a
    // get object using the NdnClient
    let r_link = format!("http://{}/ndn{}", local_ndn_server_host, obj_path);

    let ret = zone_a_client
        .get_obj_by_url(r_link.as_str(), Some(obj_id.clone()))
        .await;

    match ret {
        Ok(_) => assert!(false, "should obj id verify failed"),
        Err(err) => {
            if let NdnError::InvalidId(_) = err {
            } else {
                assert!(false, "unexpect error, should obj id verify failed.")
            }
        }
    }
}

// http://{host}/ndn/{obj-path}/inner-path
#[tokio::test]
async fn ndn_2_zone_r_link_innerpath_ok() {
    init_logging("ndn_2_zone_r_link_innerpath_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let obj_path = "/test_obj_path";
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path,
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // get object using the NdnClient
    let inner_path = "obj";
    let r_link_inner_path = format!(
        "http://{}/ndn{}/{}",
        local_ndn_server_host, obj_path, inner_path
    );
    let (got_obj_id, got_obj) = zone_a_client
        .get_obj_by_url(r_link_inner_path.as_str(), None)
        .await
        .expect("get obj from ndn-mgr failed");

    // assert_eq!(got_obj_id, obj_id, "got obj-id mismatch");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    let (_, expect_obj_str) =
        build_named_object_by_json("non-test-obj", obj.get(inner_path).unwrap());
    assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

#[tokio::test]
async fn ndn_2_zone_r_link_innerpath_not_found() {
    init_logging("ndn_2_zone_r_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();

    let obj_path = "";
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path,
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // get object using the NdnClient
    let inner_path = "notexist";
    let r_link_inner_path = format!(
        "http://{}/ndn{}/{}",
        local_ndn_server_host, obj_path, inner_path
    );
    let ret = zone_a_client
        .get_obj_by_url(r_link_inner_path.as_str(), None)
        .await;

    match ret {
        Ok(_) => assert!(false, "sub obj id should not found"),
        Err(err) => {
            if let NdnError::NotFound(_) = err {
            } else {
                assert!(
                    false,
                    "unexpect error, sub obj id should not found. {:?}",
                    err
                )
            }
        }
    }

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let r_link = format!("http://test.buckyos.io/ndn{}/{}", obj_path, inner_path);
    let ret = zone_b_client
        .get_obj_by_url(r_link.as_str(), Some(obj_id.clone()))
        .await;

    match ret {
        Ok(_) => assert!(false, "sub obj id should not found"),
        Err(err) => {
            if let NdnError::NotFound(_) = err {
            } else {
                assert!(
                    false,
                    "unexpect error, sub obj id should not found. {:?}",
                    err
                )
            }
        }
    }
}

#[tokio::test]
async fn ndn_2_zone_r_link_innerpath_verify_failed() {
    init_logging("ndn_2_zone_r_link_innerpath_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (local_ndn_client, local_ndn_server_host) =
        init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, mut obj) = generate_random_obj();

    // modify 'obj.string'.
    obj.as_object_mut().unwrap().insert(
        "string".to_string(),
        serde_json::Value::String("fake string".to_string()),
    );

    let obj_path = "";
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path,
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // get object using the NdnClient
    let inner_path = "string";
    let r_link_inner_path = format!(
        "http://{}/ndn/{}{}",
        local_ndn_server_host, obj_path, inner_path
    );
    let ret = zone_a_client
        .get_obj_by_url(r_link_inner_path.as_str(), None)
        .await;

    match ret {
        Ok(_) => assert!(false, "should obj id verify failed"),
        Err(err) => {
            if let NdnError::InvalidId(_) = err {
            } else {
                assert!(false, "unexpect error, should obj id verify failed.")
            }
        }
    }
}
