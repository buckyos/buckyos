use buckyos_kit::*;
use hex::ToHex;
use ndn_lib::*;
use test_ndn::*;

#[tokio::test]
async fn ndn_local_file_ok() {
    init_logging("ndn_local_file_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

    NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

    let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
    assert_eq!(file_id, cal_file_id, "file-id mismatch");

    NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
        .await
        .expect("put object in local failed");

    check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(Some(&file_obj)), None).await;

    let _buffer = NamedDataMgrTest::read_chunk_with_check(
        ndn_mgr_id.as_str(),
        &chunk_id,
        chunk_data.as_slice(),
    )
    .await;
}

#[tokio::test]
async fn ndn_local_file_not_found() {
    init_logging("ndn_local_file_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    {
        let (file_id, _file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(None), None).await;

        NamedDataMgrTest::open_chunk_reader_not_found(ndn_mgr_id.as_str(), &chunk_id).await;
    }

    {
        // write chunk only
        let (file_id, _file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(None), None).await;

        let _buffer = NamedDataMgrTest::read_chunk_with_check(
            ndn_mgr_id.as_str(),
            &chunk_id,
            chunk_data.as_slice(),
        )
        .await;
    }

    {
        // put file-obj only
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(Some(&file_obj)), None).await;

        NamedDataMgrTest::open_chunk_reader_not_found(ndn_mgr_id.as_str(), &chunk_id).await;
    }
}

#[tokio::test]
async fn ndn_local_file_verify_failed() {
    init_logging("ndn_local_file_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let _ndn_client = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // fake file.content
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        NamedDataMgrTest::write_chunk(
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

        NamedDataMgrTest::read_chunk_with_check(
            ndn_mgr_id.as_str(),
            &fake_chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;

        NamedDataMgrTest::open_chunk_reader_not_found(ndn_mgr_id.as_str(), &chunk_id).await;
    }

    {
        // fake file.name
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

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

        let _buffer = NamedDataMgrTest::read_chunk_with_check(
            ndn_mgr_id.as_str(),
            &chunk_id,
            chunk_data.as_slice(),
        )
        .await;
    }

    {
        // fake chunk
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        let (_fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice())
            .await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        check_file_obj(ndn_mgr_id.as_str(), &file_id, Some(Some(&file_obj)), None).await;

        let _buffer = NamedDataMgrTest::read_chunk_with_check(
            ndn_mgr_id.as_str(),
            &chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;
    }
}

// http://{host}/ndn/{obj-id}/inner-path
#[tokio::test]
async fn ndn_local_o_link_innerpath_file_ok() {
    init_logging("ndn_local_o_link_innerpath_file_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    {
        // 1. get chunk of file
        // 2. get name of file
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let resp_headers = ndn_client
            .open_chunk_reader_by_url_with_check(
                o_link_inner_path.as_str(),
                Some(&chunk_id),
                chunk_data.as_slice(),
                &file_id,
            )
            .await;
        assert!(
            resp_headers.path_obj.is_none(),
            "path-obj should be None for o-link"
        );

        // todo: verify chunk with mtree

        // todo: how to get fields with no object
        // let o_link_inner_path = format!("http://{}/ndn/{}/name", ndn_host, file_id.to_string(),);
        // let (name_obj_id, name_json) = ndn_client
        //     .get_obj_by_url(o_link_inner_path.as_str(), None)
        //     .await
        //     .expect("get name of file with o-link failed");

        // let name = name_json.as_str().expect("name should be string");
        // assert_eq!(name, file_obj.name.as_str(), "name mismatch");
    }

    {
        // 1. get name of file
        // todo: how to get fields with no object
        // let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        // let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        // assert_eq!(file_id, cal_file_id, "file-id mismatch");

        // NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
        //     .await
        //     .expect("put object in local failed");

        // let o_link_inner_path = format!("http://{}/ndn/{}/name", ndn_host, file_id.to_string());
        // let (name_obj_id, name_json) = ndn_client
        //     .get_obj_by_url(o_link_inner_path.as_str(), None)
        //     .await
        //     .expect("get name of file with o-link failed");

        // let name = name_json.as_str().expect("name should be string");
        // assert_eq!(name, file_obj.name.as_str(), "name mismatch");
    }

    {
        // 1. get chunk range by range
        let (file_id, file_obj, chunk_id, chunk_data) =
            generate_random_file_obj_with_len(16, 5 * 1024 * 1024);

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string());

        ndn_client
            .open_chunk_reader_by_url_range_with_check(
                o_link_inner_path.as_str(),
                Some(&chunk_id),
                chunk_data.as_slice(),
                &file_id,
            )
            .await;
    }

    {
        // download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let (_download_chunk_id, _download_chunk_len) = ndn_client
            .download_chunk_to_local_with_check(
                o_link_inner_path.as_str(),
                &chunk_id,
                true,
                chunk_data.as_slice(),
            )
            .await;

        let (_download_chunk_id, _download_chunk_len) = ndn_client
            .download_chunk_to_local_with_check(
                o_link_inner_path.as_str(),
                &chunk_id,
                false,
                chunk_data.as_slice(),
            )
            .await;
    }
}

#[tokio::test]
async fn ndn_local_o_link_innerpath_file_not_found() {
    init_logging("ndn_local_o_link_innerpath_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // no chunk saved
        // 1. get chunk of file
        // 2. get name of file
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        ndn_client
            .open_chunk_reader_by_url_not_found(o_link_inner_path.as_str(), Some(&chunk_id))
            .await;

        let o_link_inner_path =
            format!("http://{}/ndn/{}/notexist", ndn_host, file_id.to_string(),);
        let _err = ndn_client
            .get_obj_by_url_err(o_link_inner_path.as_str(), None)
            .await;
    }

    {
        // no write chunk for download to local
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        ndn_client
            .download_chunk_to_local_not_found(o_link_inner_path.as_str(), &chunk_id, false)
            .await;

        ndn_client
            .download_chunk_to_local_not_found(o_link_inner_path.as_str(), &chunk_id, true)
            .await;
    }

    {
        // field not exist
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path =
            format!("http://{}/ndn/{}/notexist", ndn_host, file_id.to_string(),);
        let _err = ndn_client
            .get_obj_by_url_err(o_link_inner_path.as_str(), None)
            .await;
    }
}

#[tokio::test]
async fn ndn_local_o_link_innerpath_file_verify_failed() {
    init_logging("ndn_local_o_link_innerpath_file_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // fake file.content
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        NamedDataMgrTest::write_chunk(
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
        //TODO:use copy chunk to verify reader
        // TODO: should verify root object
        ndn_client
            .open_chunk_reader_by_url_verify_error(o_link_inner_path.as_str(), Some(&chunk_id))
            .await;
    }

    {
        // fake file.content for download to local
        let (file_id, file_obj, _chunk_id, _chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        NamedDataMgrTest::write_chunk(
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

        // TODO: reserve invalid file?
        // let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        // let _err = ndn_client
        //     .download_chunk_to_local_err(o_link_inner_path.as_str(), &chunk_id, false)
        //     .await;

        // TODO: no_verify invalid
        // let _ = ndn_client
        //     .download_chunk_to_local_with_check(
        //         o_link_inner_path.as_str(),
        //         &chunk_id,
        //         true,
        //         fake_chunk_data.as_slice(),
        //     )
        //     .await;
    }

    {
        // fake chunk
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        let (_fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice())
            .await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");
        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        let resp_headers = ndn_client
            .open_chunk_reader_by_url_with_check(
                o_link_inner_path.as_str(),
                Some(&chunk_id),
                fake_chunk_data.as_slice(),
                &file_id,
            )
            .await;

        assert!(
            resp_headers.path_obj.is_none(),
            "path-obj should be None for o-link"
        );
    }

    {
        // fake chunk for download to local
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        let (_fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice())
            .await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        // TODO: reserve invalid file?
        // let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        // let _err = ndn_client
        //     .download_chunk_to_local_err(o_link_inner_path.as_str(), &chunk_id, false)
        //     .await;

        // TODO: no_verify invalid
        // let _ = ndn_client
        //     .download_chunk_to_local_with_check(
        //         o_link_inner_path.as_str(),
        //         &chunk_id,
        //         true,
        //         fake_chunk_data.as_slice(),
        //     )
        //     .await;
    }

    {
        // fake file.name
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        NamedDataMgr::put_object(Some(ndn_mgr_id.as_str()), &file_id, file_obj_str.as_str())
            .await
            .expect("put object in local failed");

        let o_link_inner_path = format!("http://{}/ndn/{}/content", ndn_host, file_id.to_string(),);
        // TODO: verify root object
        ndn_client
            .open_chunk_reader_by_url_verify_error(o_link_inner_path.as_str(), Some(&chunk_id))
            .await;
    }

    {
        // fake file.name for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

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
        // TODO: verify root object
        // ndn_client
        //     .download_chunk_to_local_verify_failed(o_link_inner_path.as_str(), &chunk_id, true)
        //     .await;

        let (_download_chunk_id, _download_chunk_len) = ndn_client
            .download_chunk_to_local_with_check(
                o_link_inner_path.as_str(),
                &chunk_id,
                false,
                chunk_data.as_slice(),
            )
            .await;
    }
}

// http://{host}/ndn/{obj-path}/inner-path
#[tokio::test]
async fn ndn_local_r_link_innerpath_file_ok() {
    init_logging("ndn_local_r_link_innerpath_file_ok", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let _ndn_url = format!("http://{}/ndn/", ndn_host);

    {
        // 1. get chunk of file
        // 2. get name of file
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();
        let mix_chunk_id = ChunkId::from_mix_hash_result_by_hash_method(
            chunk_data.len() as u64,
            chunk_id.hash_result.as_slice(),
            HashMethod::Sha256,
        )
        .unwrap();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, _file_obj_str) = file_obj.gen_obj_id();
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

        let obj_path = "/test_file_path-chunk-name";
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

        let (file_id, _file_obj_str) = file_obj.gen_obj_id();

        assert!(
            file_obj.content == chunk_id.to_string()
                || file_obj.content == mix_chunk_id.to_string(),
            "file content should be same as ndn-path"
        );
        assert_eq!(
            file_obj.size,
            chunk_data.len() as u64,
            "file content-ndn-path should be same as ndn-path"
        );

        let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path,);
        let resp_headers = ndn_client
            .open_chunk_reader_by_url_with_check(
                r_link_inner_path.as_str(),
                Some(&mix_chunk_id),
                chunk_data.as_slice(),
                &file_id,
            )
            .await;

        assert!(
            resp_headers.path_obj.is_none(),
            "path-obj should be None for o-link"
        );

        // todo: how to get field with no object
        // let r_link_inner_path = format!("http://{}/ndn{}/name", ndn_host, obj_path);
        // let (name_obj_id, name_json) = ndn_client
        //     .get_obj_by_url(r_link_inner_path.as_str(), None)
        //     .await
        //     .expect("get name of file with o-link failed");

        // let name = name_json.as_str().expect("name should be string");
        // assert_eq!(name, file_obj.name.as_str(), "name mismatch");

        // std::fs::remove_file(local_path.as_path()).expect("remove local file failed");
    }

    {
        // 1. get name of file
        // todo: how to get field with no object
        // let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        // let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        // assert_eq!(file_id, cal_file_id, "file-id mismatch");

        // let obj_path = "/test_file_path-name";
        // NamedDataMgr::pub_object_to_file(
        //     Some(ndn_mgr_id.as_str()),
        //     serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
        //     OBJ_TYPE_FILE,
        //     obj_path,
        //     "test_non_file_obj_user_id",
        //     "test_non_file_obj_app_id",
        // )
        // .await
        // .expect("pub object to file failed");

        // let r_link_inner_path = format!("http://{}/ndn{}/name", ndn_host, obj_path);
        // let (name_obj_id, name_json) = ndn_client
        //     .get_obj_by_url(r_link_inner_path.as_str(), None)
        //     .await
        //     .expect("get name of file with o-link failed");

        // let name = name_json.as_str().expect("name should be string");
        // assert_eq!(name, file_obj.name.as_str(), "name mismatch");
    }

    {
        // 1. get chunk range by range
        let (file_id, file_obj, chunk_id, chunk_data) =
            generate_random_file_obj_with_len(16, 5 * 1024 * 1024);

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, _file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path-range";
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

        let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path,);
        ndn_client
            .open_chunk_reader_by_url_range_with_check(
                r_link_inner_path.as_str(),
                Some(&chunk_id),
                chunk_data.as_slice(),
                &file_id,
            )
            .await;
    }

    {
        // download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, _file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path-download";
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

        let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path,);

        let (_download_chunk_id, _download_chunk_len) = ndn_client
            .download_chunk_to_local_with_check(
                r_link_inner_path.as_str(),
                &chunk_id,
                false,
                chunk_data.as_slice(),
            )
            .await;

        let (_download_chunk_id, _download_chunk_len) = ndn_client
            .download_chunk_to_local_with_check(
                r_link_inner_path.as_str(),
                &chunk_id,
                true,
                chunk_data.as_slice(),
            )
            .await;
    }
}

#[tokio::test]
async fn ndn_local_r_link_innerpath_file_not_found() {
    init_logging("ndn_local_r_link_innerpath_file_not_found", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // no chunk saved
        // 1. get chunk of file
        // 2. get name of file
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        // write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, _file_obj_str) = file_obj.gen_obj_id();
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

        let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path,);
        ndn_client
            .open_chunk_reader_by_url_not_found(r_link_inner_path.as_str(), Some(&chunk_id))
            .await;

        let r_link_inner_path = format!("http://{}/ndn{}/notexist", ndn_host, obj_path);
        let _err = ndn_client
            .get_obj_by_url_err(r_link_inner_path.as_str(), None)
            .await;
    }

    {
        // no pub file object for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, _file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path1";
        // NamedDataMgr::pub_object_to_file(
        //     Some(ndn_mgr_id.as_str()),
        //     serde_json::to_value(&file_obj).expect("Failed to serialize FileObject"),
        //     OBJ_TYPE_FILE,
        //     obj_path,
        //     "test_non_file_obj_user_id",
        //     "test_non_file_obj_app_id",
        // )
        // .await
        // .expect("pub object to file failed");

        let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path,);

        // when chunk already exists, download should success immediately
        let _ = ndn_client
            .download_chunk_to_local_with_check(
                r_link_inner_path.as_str(),
                &chunk_id,
                true,
                chunk_data.as_slice(),
            )
            .await;

        let chunk_id_not_exist = ChunkId::new("sha256:1234567890").expect("invalid chunk id");
        ndn_client
            .download_chunk_to_local_not_found(
                r_link_inner_path.as_str(),
                &chunk_id_not_exist,
                false,
            )
            .await;
    }

    {
        // field not exist
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, _file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id mismatch");

        let obj_path = "/test_file_path2";
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

        let r_link_inner_path = format!("http://{}/ndn{}/notexist", ndn_host, obj_path,);

        let _err = ndn_client
            .get_obj_by_url_err(r_link_inner_path.as_str(), None)
            .await;
    }
}

#[tokio::test]
async fn ndn_local_r_link_innerpath_file_verify_failed() {
    init_logging("ndn_local_r_link_innerpath_file_verify_failed", false);

    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    {
        // fake file.content
        let (file_id, file_obj, _chunk_id, _chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        NamedDataMgrTest::write_chunk(
            ndn_mgr_id.as_str(),
            &fake_chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;

        let (fake_file_id, fake_file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, fake_file_id, "file-id should not match");

        let obj_path = "/test_file_path";
        pub_object_to_file_with_str(
            ndn_mgr_id.as_str(),
            obj_path,
            &file_id,
            fake_file_obj_str.as_str(),
        )
        .await;

        let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path,);

        //TODO:use copy chunk to verify reader
        // TODO: should verify root object
        ndn_client
            .open_chunk_reader_by_url_verify_error(r_link_inner_path.as_str(), Some(&fake_chunk_id))
            .await;
    }

    {
        // fake file.content for download to local
        let (file_id, file_obj, _chunk_id, _chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        let (fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);
        fake_file_obj.content = fake_chunk_id.to_string();

        NamedDataMgrTest::write_chunk(
            ndn_mgr_id.as_str(),
            &fake_chunk_id,
            fake_chunk_data.as_slice(),
        )
        .await;

        let (fake_file_id, fake_file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, fake_file_id, "file-id should not match");

        let obj_path2 = "/test_file_path2";

        pub_object_to_file_with_str(
            ndn_mgr_id.as_str(),
            obj_path2,
            &file_id,
            fake_file_obj_str.as_str(),
        )
        .await;

        //TODO: no_verify invalid
        // let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path2);
        // let _err = ndn_client
        //     .download_chunk_to_local_err(r_link_inner_path.as_str(), &fake_chunk_id, false)
        //     .await;
    }

    {
        // fake chunk
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        let (_fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice())
            .await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        let obj_path3 = "/test_file_path3";
        pub_object_to_file_with_str(
            ndn_mgr_id.as_str(),
            obj_path3,
            &file_id,
            file_obj_str.as_str(),
        )
        .await;

        let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path3);

        let resp_headers = ndn_client
            .open_chunk_reader_by_url_with_check(
                r_link_inner_path.as_str(),
                Some(&chunk_id),
                fake_chunk_data.as_slice(),
                &file_id,
            )
            .await;

        assert!(
            resp_headers.path_obj.is_none(),
            "path-obj should be None for o-link"
        );
    }

    {
        // fake chunk for download to local
        let (file_id, file_obj, chunk_id, _chunk_data) = generate_random_file_obj();

        let (_fake_chunk_id, fake_chunk_data) = generate_random_chunk(5678);

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, fake_chunk_data.as_slice())
            .await;

        let (cal_file_id, file_obj_str) = file_obj.gen_obj_id();
        assert_eq!(file_id, cal_file_id, "file-id should not match");

        let obj_path4 = "/test_file_path4";
        pub_object_to_file_with_str(
            ndn_mgr_id.as_str(),
            obj_path4,
            &file_id,
            file_obj_str.as_str(),
        )
        .await;

        // TODO: reserve invalid file?
        // let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path4);
        // let _err = ndn_client
        //     .download_chunk_to_local_err(r_link_inner_path.as_str(), &chunk_id, false)
        //     .await;
    }

    {
        // fake file.name
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        let obj_path5 = "/test_file_path5";
        pub_object_to_file_with_str(
            ndn_mgr_id.as_str(),
            obj_path5,
            &file_id,
            file_obj_str.as_str(),
        )
        .await;

        let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path5);
        //todo: use copy chunk to verify reader
        // TODO: verify root object
        ndn_client
            .open_chunk_reader_by_url_verify_error(r_link_inner_path.as_str(), Some(&chunk_id))
            .await;
    }

    {
        // fake file.name for download to local
        let (file_id, file_obj, chunk_id, chunk_data) = generate_random_file_obj();

        let mut fake_file_obj = file_obj.clone();
        fake_file_obj.name = "fake-file-name".to_string();

        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), &chunk_id, chunk_data.as_slice()).await;

        let (cal_file_id, file_obj_str) = fake_file_obj.gen_obj_id();
        assert_ne!(file_id, cal_file_id, "file-id mismatch");

        let obj_path6 = "/test_file_path6";
        pub_object_to_file_with_str(
            ndn_mgr_id.as_str(),
            obj_path6,
            &file_id,
            file_obj_str.as_str(),
        )
        .await;

        // TODO: verify root object
        // let r_link_inner_path = format!("http://{}/ndn{}/content", ndn_host, obj_path6);
        // ndn_client
        //     .download_chunk_to_local_verify_failed(r_link_inner_path.as_str(), &chunk_id, false)
        //     .await;
    }
}
