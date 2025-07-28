use buckyos_kit::*;
use hex::ToHex;
use ndn_lib::*;
use test_ndn::*;

#[tokio::test]
async fn ndn_local_2_mgr_object_ok() {
    init_logging("ndn_local_2_mgr_object_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;
    let ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let (_got_obj_id, _got_obj) = target_ndn_client
        .get_obj_by_url_with_check(
            format!("{}{}", ndn_url, obj_id.to_base32()).as_str(),
            Some(&obj_id),
            Some(&obj),
            Some((obj_str.as_str(), "non-test-obj")),
        )
        .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_object_not_found() {
    init_logging("ndn_local_2_mgr_object_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;
    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // no get
    // let (got_obj_id, got_obj) = target_ndn_client
    //     .get_obj_by_url(
    //         format!("{}{}", ndn_url, obj_id.to_base32()).as_str(),
    //         Some(obj_id.clone()),
    //     )
    //     .await
    //     .expect("get obj from ndn-mgr failed");

    check_obj_inner_path(
        target_ndn_mgr_id.as_str(),
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
async fn ndn_local_2_mgr_object_verify_failed() {
    init_logging("ndn_local_object_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;
    let ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, _obj) = generate_random_obj();
    let (_fake_obj_id, fake_obj) = generate_random_obj();

    let (_, fake_obj_str) = build_named_object_by_json("non-test-obj", &fake_obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, fake_obj_str.as_str())
        .await
        .expect("put object in local failed");

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_host) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    target_ndn_client
        .get_obj_by_url_invalid_id(
            format!("{}{}", ndn_url, obj_id.to_base32()).as_str(),
            Some(&obj_id),
        )
        .await;
}

// http://{host}/ndn/{obj-id}/
#[tokio::test]
async fn ndn_local_2_mgr_o_link_ok() {
    init_logging("ndn_local_2_mgr_o_link_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let o_link = format!("http://{}/ndn/{}", ndn_host, obj_id_base32);
    let (_got_obj_id, _got_obj) = target_ndn_client
        .get_obj_by_url_with_check(
            o_link.as_str(),
            Some(&obj_id),
            Some(&obj),
            Some((obj_str.as_str(), "non-test-obj")),
        )
        .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_o_link_not_found() {
    init_logging("ndn_local_2_mgr_o_link_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;
    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, target_ndn_host) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let o_link = format!("http://{}/ndn/{}", target_ndn_host, obj_id_base32);
    target_ndn_client
        .get_obj_by_url_not_found(o_link.as_str())
        .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_o_link_verify_failed() {
    init_logging("ndn_local_2_mgr_o_link_innerpath_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;
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

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let _inner_path = "string";
    let o_link = format!("http://{}/ndn/{}", ndn_host, obj_id_base32);
    target_ndn_client
        .get_obj_by_url_invalid_id(o_link.as_str(), Some(&obj_id))
        .await;
}

// http://{host}/ndn/{obj-id}/inner-path
#[tokio::test]
async fn ndn_local_2_mgr_o_link_innerpath_ok() {
    init_logging("ndn_local_o_link_innerpath_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    // todo: how to get sub-object that not make in Object
    // let inner_path = "obj";
    // let o_link_inner_path = format!("http://{}/ndn/{}/{}", ndn_host, obj_id_base32, inner_path);
    // let (got_obj_id, got_obj) = target_ndn_client
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
async fn ndn_local_2_mgr_o_link_innerpath_not_found() {
    init_logging("ndn_local_2_mgr_o_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;
    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let inner_path = "notexist";
    let o_link_inner_path = format!("http://{}/ndn/{}/{}", ndn_host, obj_id_base32, inner_path);
    let _err = target_ndn_client
        .get_obj_by_url_err(o_link_inner_path.as_str(), None)
        .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_o_link_innerpath_verify_failed() {
    init_logging("ndn_local_2_mgr_o_link_innerpath_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;
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

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let inner_path = "string";
    let o_link_inner_path = format!("http://{}/ndn/{}/{}", ndn_host, obj_id_base32, inner_path);
    target_ndn_client
        .get_obj_by_url_invalid_id(o_link_inner_path.as_str(), None)
        .await;
}

// http://{obj-id}.{host}/ndn/{obj-id}/inner-path
#[tokio::test]
async fn ndn_local_2_mgr_o_link_in_host_innerpath_ok() {
    init_logging("ndn_local_2_mgr_o_link_innerpath_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    let (obj_id, obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    // get object using the NdnClient
    // todo: how to get sub-object that not make in Object
    // let inner_path = "obj";
    // let o_link_inner_path = format!("http://{}.{}/ndn/{}", obj_id_base32, ndn_host, inner_path);
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

//#[tokio::test]
async fn ndn_local_2_mgr_o_link_in_host_innerpath_not_found() {
    unimplemented!()
}

//#[tokio::test]
async fn ndn_local_2_mgr_o_link_in_host_innerpath_verify_failed() {
    unimplemented!()
}

// http://{host}/ndn/{obj-path}
#[tokio::test]
async fn ndn_local_2_mgr_r_link_ok() {
    init_logging("ndn_local_2_mgr_r_link_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

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

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let (_, expect_obj_str) = build_named_object_by_json("non-test-obj", &obj);
    // get object using the NdnClient
    let r_link = format!("http://{}/ndn{}", ndn_host, obj_path);
    let (_got_obj_id, _got_obj) = target_ndn_client
        .get_obj_by_url_with_check(
            r_link.as_str(),
            Some(&obj_id),
            Some(&obj),
            Some((expect_obj_str.as_str(), "non-test-obj")),
        )
        .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_r_link_not_found() {
    init_logging("ndn_local_2_mgr_r_link_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

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

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let r_link = format!("http://{}/ndn{}", ndn_host, obj_path);
    target_ndn_client
        .get_obj_by_url_not_found(r_link.as_str())
        .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_r_link_verify_failed() {
    init_logging("ndn_local_r_link_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

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

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let r_link = format!("http://{}/ndn{}", ndn_host, obj_path);
    target_ndn_client
        .get_obj_by_url_invalid_id(r_link.as_str(), Some(&obj_id))
        .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_r_link_innerpath_ok() {
    init_logging("ndn_local_2_mgr_r_link_innerpath_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

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

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // todo: how to get sub-object that not make in Object
    // get object using the NdnClient
    // let inner_path = "obj";
    // let r_link_inner_path = format!("http://{}/ndn{}/{}", ndn_host, obj_path, inner_path);
    // let (_got_obj_id, got_obj) = target_ndn_client
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
async fn ndn_local_2_mgr_r_link_innerpath_not_found() {
    init_logging("ndn_local_2_mgr_r_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

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

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let inner_path = "notexist";
    let r_link_inner_path = format!("http://{}/ndn{}/{}", ndn_host, obj_path, inner_path);
    let _err = target_ndn_client
        .get_obj_by_url_err(r_link_inner_path.as_str(), None)
        .await;
}

#[tokio::test]
async fn ndn_local_2_mgr_r_link_innerpath_verify_failed() {
    init_logging("ndn_local_2_mgr_r_link_innerpath_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

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

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (target_ndn_client, _target_ndn_url) =
        init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    // get object using the NdnClient
    let inner_path = "string";
    let r_link_inner_path = format!("http://{}/ndn{}/{}", ndn_host, obj_path, inner_path);
    target_ndn_client
        .get_obj_by_url_invalid_id(r_link_inner_path.as_str(), None)
        .await;
}
