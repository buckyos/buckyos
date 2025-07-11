use buckyos_kit::*;
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use hex::ToHex;
use log::*;
use ndn_lib::*;
use rand::{Rng, RngCore};
use serde_json::json;
use tokio::fs;

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
                    "get object {:?} with innser-path {:?} failed, error: {:?}",
                    obj_id, inner_path, err
                ),
            },
            None => match &got_ret {
                Ok(got_obj) => {
                    assert!(
                        got_obj.is_null(),
                        "should no object found: {}, inner-path: {:?}",
                        got_obj.to_string(),
                        inner_path
                    )
                }
                Err(err) => match err {
                    NdnError::NotFound(_) => {
                        info!("Chunk not found as expected");
                    }
                    _ => {
                        assert!(false, "Unexpected error type: {:?}", err);
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
                    "get object {:?} with innser-path {:?} failed, error: {:?}",
                    obj_id, inner_path, err
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

#[tokio::test]
async fn ndn_local_object_ok() {
    init_logging("ndn_local_object_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let (obj_id, obj) = generate_random_obj();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
        "non-test-obj",
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
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let (obj_id, _obj) = generate_random_obj();

    check_obj_inner_path(
        ndn_mgr_id.as_str(),
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
async fn ndn_local_object_verify_failed() {
    init_logging("ndn_local_object_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_ndn_server(ndn_mgr_id.as_str()).await;

    let (obj_id, obj) = generate_random_obj();
    let (fake_obj_id, fake_obj) = generate_random_obj();

    let (_, fake_obj_str) = build_named_object_by_json("non-test-obj", &fake_obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, fake_obj_str.as_str())
        .await
        .expect("put object in local failed");

    check_obj_inner_path(
        ndn_mgr_id.as_str(),
        &obj_id,
        "non-test-obj",
        None,
        Some(Some(&fake_obj)),
        Some(Some(&obj)),
        Some(&fake_obj_id),
    )
    .await;
}

// http://{host}/ndn/{obj-id}
#[tokio::test]
async fn ndn_local_o_link_ok() {
    init_logging("ndn_local_o_link_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // get object using the NdnClient
    let o_link = format!("http://{}/ndn/{}", ndn_host, obj_id_base32);
    let (got_obj_id, got_obj) = ndn_client
        .get_obj_by_url(o_link.as_str(), None)
        .await
        .expect("get obj from ndn-mgr failed");

    assert_eq!(got_obj_id, obj_id, "got obj-id mismatch");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    let (_, expect_obj_str) = build_named_object_by_json("non-test-obj", &obj);
    assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

#[tokio::test]
async fn ndn_local_o_link_not_found() {
    init_logging("ndn_local_o_link_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let (_, _obj_str) = build_named_object_by_json("non-test-obj", &obj);
    // NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
    //     .await
    //     .expect("put object in local failed");

    // get object using the NdnClient
    let o_link = format!("http://{}/ndn/{}", ndn_host, obj_id_base32);
    let ret = ndn_client.get_obj_by_url(o_link.as_str(), None).await;

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
async fn ndn_local_o_link_verify_failed() {
    init_logging("ndn_local_o_link_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

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

    // get object using the NdnClient
    let o_link = format!("http://{}/ndn/{}", ndn_host, obj_id_base32);
    let ret = ndn_client.get_obj_by_url(o_link.as_str(), None).await;

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
async fn ndn_local_o_link_innerpath_ok() {
    init_logging("ndn_local_o_link_innerpath_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // get object using the NdnClient
    // todo: how to get sub-object from remote
    // let inner_path = "obj";
    // let o_link_inner_path = format!("http://{}/ndn/{}/{}", ndn_host, obj_id_base32, inner_path);
    // let (_got_obj_id, got_obj) = ndn_client
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
async fn ndn_local_o_link_innerpath_not_found() {
    init_logging("ndn_local_o_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // get object using the NdnClient
    let inner_path = "notexist";
    let o_link_inner_path = format!("http://{}/ndn/{}/{}", ndn_host, obj_id_base32, inner_path);
    let ret = ndn_client
        .get_obj_by_url(o_link_inner_path.as_str(), None)
        .await;

    match ret {
        Ok(_) => assert!(false, "sub obj id should not found"),
        Err(err) => assert!(true, "sub obj id should not found"),
    }
}

#[tokio::test]
async fn ndn_local_o_link_innerpath_verify_failed() {
    init_logging("ndn_local_o_link_innerpath_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

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

    // get object using the NdnClient
    let inner_path = "string";
    let o_link_inner_path = format!("http://{}/ndn/{}/{}", ndn_host, obj_id_base32, inner_path);
    let ret = ndn_client
        .get_obj_by_url(o_link_inner_path.as_str(), None)
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

// http://{host}/ndn/{obj-path}
#[tokio::test]
async fn ndn_local_r_link_ok() {
    init_logging("ndn_local_r_link_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

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
    let r_link = format!("http://{}/ndn{}", ndn_host, obj_path);
    let (got_obj_id, got_obj) = ndn_client
        .get_obj_by_url(r_link.as_str(), Some(obj_id.clone()))
        .await
        .expect("get obj from ndn-mgr failed");

    assert_eq!(got_obj_id, obj_id, "got obj-id mismatch");

    let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    let (_, expect_obj_str) = build_named_object_by_json("non-test-obj", &obj);
    assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

#[tokio::test]
async fn ndn_local_r_link_not_found() {
    init_logging("ndn_local_r_link_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, _obj) = generate_random_obj();

    let obj_path = "/test_obj_path";
    // no pub
    // NamedDataMgr::pub_object_to_file(
    //     Some(ndn_mgr_id.as_str()),
    //     obj.clone(),
    //     "non-test-obj",
    //     obj_path,
    //     "test_non_obj_user_id",
    //     "test_non_obj_app_id",
    // )
    // .await
    // .expect("pub object to file failed");

    // get object using the NdnClient
    let r_link = format!("http://{}/ndn{}", ndn_host, obj_path);
    let ret = ndn_client
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
async fn ndn_local_r_link_verify_failed() {
    init_logging("ndn_local_r_link_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

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

    // get object using the NdnClient
    let r_link = format!("http://{}/ndn{}", ndn_host, obj_path);
    let ret = ndn_client
        .get_obj_by_url(r_link.as_str(), Some(obj_id.clone()))
        .await;

    match ret {
        Ok(_) => assert!(false, "should obj id verify failed"),
        Err(err) => {
            if let NdnError::InvalidId(_) = err {
            } else {
                assert!(
                    false,
                    "unexpect error, should obj id verify failed. {:?}",
                    err
                );
            }
        }
    }
}

// http://{host}/ndn/{obj-path}/inner-path
#[tokio::test]
async fn ndn_local_r_link_innerpath_ok() {
    std::env::set_var("BUCKY_LOG", "debug");
    init_logging("ndn_local_r_link_innerpath_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

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
    let _r_link_inner_path = format!("http://{}/ndn{}/{}", ndn_host, obj_path, inner_path);
    //TODO：fix test,url is not target to a object
    // let (got_obj_id, got_obj) = ndn_client
    //     .get_obj_by_url(r_link_inner_path.as_str(), None)
    //     .await
    //     .expect("get obj from ndn-mgr failed");

    // assert_eq!(got_obj_id, obj_id, "got obj-id mismatch");

    // let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    // let (_, expect_obj_str) =
    //     build_named_object_by_json("non-test-obj", obj.get(inner_path).unwrap());
    // assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

#[tokio::test]
async fn ndn_local_r_link_innerpath_not_found() {
    init_logging("ndn_local_r_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (_obj_id, obj) = generate_random_obj();

    let obj_path = "/test-obj-path";
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
    let r_link_inner_path = format!("http://{}/ndn/{}/{}", ndn_host, obj_path, inner_path);
    let ret = ndn_client
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
}

#[tokio::test]
async fn ndn_local_r_link_innerpath_verify_failed() {
    init_logging("ndn_local_r_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (_obj_id, mut obj) = generate_random_obj();

    // modify 'obj.string'.
    obj.as_object_mut().unwrap().insert(
        "string".to_string(),
        serde_json::Value::String("fake string".to_string()),
    );

    let obj_path = "/test-obj-path";
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
    let r_link_inner_path = format!("http://{}/ndn{}/{}", ndn_host, obj_path, inner_path);
    let ret = ndn_client
        .get_obj_by_url(r_link_inner_path.as_str(), None)
        .await;

    match ret {
        Ok(_) => assert!(false, "should obj id verify failed"),
        Err(err) => {
            if let NdnError::InvalidId(_) = err {
            } else {
                assert!(
                    false,
                    "unexpect error, should obj id verify failed. {:?}",
                    err
                )
            }
        }
    }
}
