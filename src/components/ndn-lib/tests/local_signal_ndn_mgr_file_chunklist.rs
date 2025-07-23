use buckyos_kit::*;
use hex::ToHex;
use log::*;
use ndn_lib::*;
use rand::Rng;
use test_ndn::*;
use tokio::io::AsyncReadExt;

//#[tokio::test]
async fn ndn_local_file_chunklist_rechunk_split() {
    init_logging("ndn_local_file_chunklist_rechunk_split", false);

    info!("ndn_local_file_chunklist_rechunk_split test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(10, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let (chunk0_id, chunk0_data) = chunks.get(0).unwrap();
    NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk0_id, chunk0_data.as_slice()).await;

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
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file1_content_url.as_str(),
            Some(&chunk0_id),
            chunk0_data.as_slice(),
            &file1_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
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
            let chunk_id =
                ChunkId::from_mix_hash_result_by_hash_method(*len, &hash, HashMethod::Sha256)
                    .unwrap();
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
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file2_content_url.as_str(),
            Some(&chunk0_id),
            chunk0_data.as_slice(),
            &file2_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
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
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file3_content_url.as_str(),
            Some(&chunk0_id),
            chunk0_data.as_slice(),
            &file3_id,
        )
        .await;

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
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file4_content_url.as_str(),
            Some(&chunk0_id),
            chunk0_data.as_slice(),
            &file4_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );

    info!("ndn_local_chunklist_ok test end.");
}

//#[tokio::test]
async fn ndn_local_file_chunklist_rechunk_combine() {
    init_logging("ndn_local_file_chunklist_rechunk_combine", false);

    info!("ndn_local_file_chunklist_rechunk_combine test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

    let chunks = generate_random_chunk_list(10, None);
    let total_size: u64 = chunks.iter().map(|c| c.1.len() as u64).sum();

    let mut chunk_list_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(total_size);

    for (chunk_id, chunk_data) in chunks.iter() {
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
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
    let combine_chunk_id = ChunkId::from_mix_hash_result_by_hash_method(
        combine_chunk_data.len() as u64,
        &hash,
        HashMethod::Sha256,
    )
    .unwrap();
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
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file1_content_url.as_str(),
            Some(&combine_chunk_id),
            combine_chunk_data.as_slice(),
            &file1_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
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
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file2_content_url.as_str(),
            Some(&combine_chunk_id),
            combine_chunk_data.as_slice(),
            &file2_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );

    info!("ndn_local_file_chunklist_rechunk_combine test end.");
}

//#[tokio::test]
async fn ndn_local_file_chunklist_delta() {
    init_logging("ndn_local_file_chunklist_delta", false);

    info!("ndn_local_file_chunklist_delta test start...");
    init_obj_array_storage_factory().await;
    let ndn_mgr_id: String = generate_random_bytes(16).encode_hex();
    let (ndn_client, ndn_host) = init_local_ndn_server(ndn_mgr_id.as_str()).await;

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
    let combine_chunk_3_6_id = ChunkId::from_mix_hash_result_by_hash_method(
        combine_chunks_3_6.len() as u64,
        &hash,
        HashMethod::Sha256,
    )
    .unwrap();

    NamedDataMgrTest::write_chunk(
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
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file1_content_url.as_str(),
            Some(&combine_chunk_3_6_id),
            combine_chunks_3_6.as_slice(),
            &file1_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );

    // File2([...chunks_0_3, ...chunks_3_6]) // insert head
    let file2_len = chunks_0_3.iter().map(|(_, d)| d.len() as u64).sum::<u64>()
        + chunks_3_6.iter().map(|(_, d)| d.len() as u64).sum::<u64>();
    let mut chunk_list_0_6_builder =
        ChunkListBuilder::new(HashMethod::Sha256).with_total_size(file2_len);
    for (chunk_id, chunk_data) in chunks_0_3.iter() {
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
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
    let file2_chunk_data = [
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
    .concat();
    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(file2_chunk_data.as_slice());
    let file2_chunk_id = ChunkId::from_mix_hash_result_by_hash_method(
        file2_chunk_data.len() as u64,
        &hash,
        HashMethod::Sha256,
    )
    .unwrap();

    let file2_content_url = format!("http://{}/ndn/{}/content", ndn_host, file2_id.to_string());
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file2_content_url.as_str(),
            Some(&file2_chunk_id),
            file2_chunk_data.as_slice(),
            &file2_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );
    assert_eq!(
        resp_headers.root_obj_id,
        Some(file2_id.clone()),
        "root-obj-id in http-header should equal with file-id"
    );
    drop(file2_chunk_data);

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
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
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

    let file3_chunk_data = [
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
    .concat();

    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(file3_chunk_data.as_slice());
    let file3_chunk_id = ChunkId::from_mix_hash_result_by_hash_method(
        file3_chunk_data.len() as u64,
        &hash,
        HashMethod::Sha256,
    )
    .unwrap();

    let file3_content_url = format!("http://{}/ndn/{}/content", ndn_host, file3_id.to_string());
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file3_content_url.as_str(),
            Some(&file3_chunk_id),
            file3_chunk_data.as_slice(),
            &file3_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );

    drop(file3_chunk_data);

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
        NamedDataMgrTest::write_chunk(ndn_mgr_id.as_str(), chunk_id, chunk_data.as_slice()).await;
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

    info!("file4_id: {}", file4_id.to_string());
    let file4_chunk_data = [
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
    .concat();

    let hasher = ChunkHasher::new(None).expect("hash failed.");
    let hash = hasher.calc_from_bytes(file4_chunk_data.as_slice());
    let file4_chunk_id = ChunkId::from_mix_hash_result_by_hash_method(
        file4_chunk_data.len() as u64,
        &hash,
        HashMethod::Sha256,
    )
    .unwrap();

    let file4_content_url = format!("http://{}/ndn/{}/content", ndn_host, file4_id.to_string());
    let resp_headers = ndn_client
        .open_chunk_reader_by_url_with_check(
            file4_content_url.as_str(),
            Some(&file4_chunk_id),
            file4_chunk_data.as_slice(),
            &file4_id,
        )
        .await;

    assert!(
        resp_headers.path_obj.is_none(),
        "path-obj should be None for o-link"
    );

    info!("ndn_local_file_chunklist_delta test end.");
}
