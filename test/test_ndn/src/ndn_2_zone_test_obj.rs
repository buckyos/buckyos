use hex::ToHex;
use log::*;
use ndn_lib::*;

use crate::{common::*, ndn_client_test::NdnClientTest};

//#[tokio::test]
pub async fn ndn_2_zone_object_ok() {
    info!("ndn_2_zone_object_ok");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
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

    // 2. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let (_got_obj_id, _got_obj) = zone_b_client
        .get_obj_by_url_with_check(
            format!("http://test.buckyos.io/ndn/{}", obj_id.to_base32()).as_str(),
            Some(&obj_id),
            Some(&obj),
            Some((obj_str.as_str(), "non-test-obj")),
        )
        .await;
}

//#[tokio::test]
pub async fn ndn_2_zone_object_not_found() {
    info!("ndn_2_zone_object_not_found");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, _obj) = generate_random_obj();

    // 1. no put
    // let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    // NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
    //     .await
    //     .expect("put object in local failed");

    // 2. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    zone_b_client
        .get_obj_by_url_not_found(
            format!("http://test.buckyos.io/ndn/{}", obj_id.to_base32()).as_str(),
        )
        .await;
}

//#[tokio::test]
pub async fn ndn_2_zone_object_verify_failed() {
    info!("ndn_2_zone_object_verify_failed");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, _obj) = generate_random_obj();
    let (_fake_obj_id, fake_obj) = generate_random_obj();

    let (_, fake_obj_str) = build_named_object_by_json("non-test-obj", &fake_obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, fake_obj_str.as_str())
        .await
        .expect("put object in local failed");

    // 2. push the object using the NdnClient to zone_b
    zone_b_client
        .get_obj_by_url_invalid_id(
            format!("http://{}/ndn/{}", "test.buckyos.io", obj_id.to_base32()).as_str(),
            Some(&obj_id),
        )
        .await;
}

// http://{host}/ndn/{obj-id}/inner-path
//#[tokio::test]
pub async fn ndn_2_zone_o_link_innerpath_ok() {
    info!("ndn_2_zone_o_link_innerpath_ok");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let _zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    let _inner_path = "obj";

    // 2. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    // todo: how to get the sub-obj with inner-path?
    // let o_link_inner_path = format!(
    //     "http://test.buckyos.io/ndn/{}/{}",
    //     obj_id_base32, inner_path
    // );
    // let (_got_obj_id, got_obj) = zone_b_client
    //     .get_obj_by_url(o_link_inner_path.as_str(), None)
    //     .await
    //     .expect("get obj from zone-a failed");

    // let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    // let (_, expect_obj_str) =
    //     build_named_object_by_json("non-test-obj", obj.get(inner_path).unwrap());
    // assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

//#[tokio::test]
pub async fn ndn_2_zone_o_link_innerpath_not_found() {
    info!("ndn_2_zone_o_link_innerpath_not_found");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let _zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

    let (_, obj_str) = build_named_object_by_json("non-test-obj", &obj);
    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &obj_id, obj_str.as_str())
        .await
        .expect("put object in local failed");

    let _inner_path = "notexist";

    // 2. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    // todo: how to get the sub-obj with inner-path?
    // let o_link_inner_path = format!(
    //     "http://test.buckyos.io/ndn/{}/{}",
    //     obj_id_base32, inner_path
    // );
    // let ret = zone_b_client
    //     .get_obj_by_url(o_link_inner_path.as_str(), None)
    //     .await;

    // match ret {
    //     Ok(_) => assert!(false, "should obj id verify failed"),
    //     Err(err) => {
    //         if let NdnError::NotFound(_) = err {
    //         } else {
    //             assert!(false, "unexpect error, should obj id verify failed.")
    //         }
    //     }
    // }
}

//#[tokio::test]
pub async fn ndn_2_zone_o_link_innerpath_verify_failed() {
    info!("ndn_2_zone_o_link_innerpath_verify_failed");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let _zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, mut obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

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
    // todo: how to get the sub-obj with inner-path?
    // get object using the NdnClient
    // let inner_path = "string";
    // let o_link_inner_path = format!(
    //     "http://{}/ndn/{}/{}",
    //     "test.buckyos.io", obj_id_base32, inner_path
    // );

    // let ret = zone_b_client
    //     .get_obj_by_url(o_link_inner_path.as_str(), None)
    //     .await;

    // match ret {
    //     Ok(_) => assert!(false, "sub obj id should not found"),
    //     Err(err) => {
    //         if let NdnError::VerifyError(_) = err {
    //         } else {
    //             assert!(
    //                 false,
    //                 "unexpect error, sub obj id should not found. {:?}",
    //                 err
    //             )
    //         }
    //     }
    // }
}

// http://{obj-id}.{host}/ndn/{obj-id}/inner-path
//#[tokio::test]
pub async fn ndn_2_zone_o_link_in_host_innerpath_ok() {
    // info!("ndn_2_zone_o_link_innerpath_ok");

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

//#[tokio::test]
pub async fn ndn_2_zone_o_link_in_host_innerpath_not_found() {
    // unimplemented!()
}

//#[tokio::test]
pub async fn ndn_2_zone_o_link_in_host_innerpath_verify_failed() {
    // unimplemented!()
}

// http://{host}/ndn/{obj-path}
//#[tokio::test]
pub async fn ndn_2_zone_r_link_ok() {
    info!("ndn_2_zone_r_link_ok");

    let ndn_mgr_id: String = "default".to_string();
    let _zone_a_client =
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
    let _obj_id_base32 = obj_id.to_base32();

    let obj_path: String = format!(
        "/test_obj_path-ok/{}",
        generate_random_bytes(8).encode_hex::<String>()
    );
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path.as_str(),
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let r_link = format!("http://test.buckyos.io/ndn{}", obj_path);
    let (_, expect_obj_str) = build_named_object_by_json("non-test-obj", &obj);
    let (_got_obj_id, _got_obj) = zone_b_client
        .get_obj_by_url_with_check(
            r_link.as_str(),
            Some(&obj_id),
            Some(&obj),
            Some((expect_obj_str.as_str(), "non-test-obj")),
        )
        .await;
}

//#[tokio::test]
pub async fn ndn_2_zone_r_link_not_found() {
    info!("ndn_2_zone_r_link_not_found");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, _obj) = generate_random_obj();

    let obj_path: String = format!(
        "/test_obj_path-not-found/{}",
        generate_random_bytes(8).encode_hex::<String>()
    );

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

    // 3. Pull the object using the NdnClient from zone_a with private key of zone_b
    // get object using the NdnClient
    let r_link = format!("http://test.buckyos.io/ndn{}", obj_path);
    zone_b_client
        .get_obj_by_url_not_found(r_link.as_str())
        .await;
}

//#[tokio::test]
pub async fn ndn_2_zone_r_link_verify_failed() {
    info!("ndn_2_zone_r_link_verify_failed");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
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

    let obj_path: String = format!(
        "/test_obj_path-r-verify/{}",
        generate_random_bytes(8).encode_hex::<String>()
    );
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path.as_str(),
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // 2. push the object using the NdnClient to zone_a
    // get object using the NdnClient
    let r_link = format!("http://{}/ndn{}", "test.buckyos.io", obj_path);

    zone_b_client
        .get_obj_by_url_invalid_id(r_link.as_str(), Some(&obj_id))
        .await;
}

// http://{host}/ndn/{obj-path}/inner-path
//#[tokio::test]
pub async fn ndn_2_zone_r_link_innerpath_ok() {
    info!("ndn_2_zone_r_link_innerpath_ok");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let _zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (obj_id, obj) = generate_random_obj();
    let _obj_id_base32 = obj_id.to_base32();

    let obj_path: String = format!(
        "/test_obj_path-innerpath-ok/{}",
        generate_random_bytes(8).encode_hex::<String>()
    );
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path.as_str(),
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // get object using the NdnClient
    // todo: who to get sub-obj from remote node
    // let inner_path = "obj";
    // let r_link_inner_path = format!(
    //     "http://{}/ndn{}/{}",
    //     local_ndn_server_host, obj_path, inner_path
    // );
    // let (_got_obj_id, got_obj) = zone_a_client
    //     .get_obj_by_url(r_link_inner_path.as_str(), None)
    //     .await
    //     .expect("get obj from ndn-mgr failed");

    // assert_eq!(got_obj_id, obj_id, "got obj-id mismatch");

    // let (_, got_obj_str) = build_named_object_by_json("non-test-obj", &got_obj);
    // let (_, expect_obj_str) =
    //     build_named_object_by_json("non-test-obj", obj.get(inner_path).unwrap());
    // assert_eq!(got_obj_str, expect_obj_str, "got obj mismatch");
}

//#[tokio::test]
pub async fn ndn_2_zone_r_link_innerpath_not_found() {
    info!("ndn_2_zone_r_link_innerpath_not_found");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (_obj_id, obj) = generate_random_obj();

    let obj_path: String = format!(
        "/test-obj-path-innerpath-not-found/{}",
        generate_random_bytes(8).encode_hex::<String>()
    );
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path.as_str(),
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // get object using the NdnClient
    let inner_path = "notexist";
    let r_link_inner_path = format!(
        "http://{}/ndn{}/{}",
        "test.buckyos.io", obj_path, inner_path
    );
    let _ret = zone_b_client
        .get_obj_by_url_err(r_link_inner_path.as_str(), None)
        .await;
}

//#[tokio::test]
pub async fn ndn_2_zone_r_link_innerpath_verify_failed() {
    info!("ndn_2_zone_r_link_innerpath_verify_failed");

    let ndn_mgr_id: String = "default".to_string();

    let _zone_a_client =
        init_ndn_client(ndn_mgr_id.as_str(), LOCAL_PRIVATE_KEY, "test.buckyos.io").await;

    let target_ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (_local_ndn_target_client, _) = init_local_ndn_server(target_ndn_mgr_id.as_str()).await;

    let _zone_b_client = init_ndn_client(
        target_ndn_mgr_id.as_str(),
        NODE_B_PRIVATE_KEY,
        "bob.web3.buckyos.io",
    )
    .await;

    let (_obj_id, mut obj) = generate_random_obj();

    // modify 'obj.string'.
    obj.as_object_mut().unwrap().insert(
        "string".to_string(),
        serde_json::Value::String("fake string".to_string()),
    );

    let obj_path: String = format!(
        "/test-obj-path-innerpath-verify/{}",
        generate_random_bytes(8).encode_hex::<String>()
    );
    NamedDataMgr::pub_object_to_file(
        Some(ndn_mgr_id.as_str()),
        obj.clone(),
        "non-test-obj",
        obj_path.as_str(),
        "test_non_obj_user_id",
        "test_non_obj_app_id",
    )
    .await
    .expect("pub object to file failed");

    // get object using the NdnClient
    // todo: how to get the sub-obj with inner-path?
    // let inner_path = "string";
    // let r_link_inner_path = format!(
    //     "http://{}/ndn{}/{}",
    //     "test.buckyos.io", obj_path, inner_path
    // );
    // let ret = zone_b_client
    //     .get_obj_by_url(r_link_inner_path.as_str(), None)
    //     .await;

    // match ret {
    //     Ok(_) => assert!(false, "should obj id verify failed"),
    //     Err(err) => {
    //         if let NdnError::InvalidId(_) = err {
    //         } else {
    //             assert!(
    //                 false,
    //                 "unexpect error, should obj id verify failed. {:?}",
    //                 err
    //             )
    //         }
    //     }
    // }
}
