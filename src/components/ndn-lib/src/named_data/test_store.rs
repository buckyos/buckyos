
use crate::{build_named_object_by_json, ChunkId, ChunkHasher, NdnError, NdnResult, ObjId};
use super::*;
use buckyos_kit::*;
use tempfile::tempdir;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

//use rand::distributions::{Alphanumeric, DistString};
// Helper function to create a test ChunkStore

async fn create_test_store() -> NdnResult<NamedDataStore> {
    init_logging("ndn-lib test", false);
    //let random_str = Alphanumeric.sample_string(&mut rand::thread_rng(), 6);
    let random_str = format!("{:x}", rand::random::<u32>());
    let temp_dir = format!("/opt/ndn_test/{}", random_str);
    let temp_dir = tempdir().unwrap().path().to_str().unwrap().to_string();
    let result_store = NamedDataStore::new(temp_dir.clone()).await;
    if result_store.is_err() {
        let err = result_store.err().unwrap();
        warn!("create_test_store: create store failed! {:?}", &err);
        return Err(err);
    }
    info!("create_test_store: store created! {}", temp_dir.as_str());
    result_store
}

#[tokio::test]
async fn test_put_and_get_chunk() -> NdnResult<()> {
    let store = create_test_store().await?;
    let data = b"test data".to_vec();
    let mut chunk_hasher = ChunkHasher::new(None).unwrap();
    let chunk_id = chunk_hasher.calc_chunk_id_from_bytes(&data);

    // Test putting chunk
    store.put_chunk(&chunk_id, &data, false).await?;
    info!("put chunk ok! {}", chunk_id.to_string());
    // Verify chunk exists
    let (is_exist, size) = store.is_chunk_exist(&chunk_id, None).await?;
    assert!(is_exist);
    assert_eq!(size, data.len() as u64);

    let (mut reader, chunk_size) = store
        .open_chunk_reader(&chunk_id, SeekFrom::Start(0))
        .await?;
    let mut buffer = vec![0u8; data.len()];
    reader.read_exact(&mut buffer).await.unwrap();
    assert_eq!(buffer, data);
    Ok(())
}

#[tokio::test]
async fn test_chunk_linking() -> NdnResult<()> {
    let store = create_test_store().await?;
    let original_id = ChunkId::new("sha256:1234567890abcdef").unwrap();
    let linked_id = ChunkId::new("qcid:2223232323232323").unwrap();

    // Create original chunk
    let data = b"original data".to_vec();
    store.put_chunk(&original_id, &data, false).await?;

    // Create link
    let src_id = linked_id.to_obj_id();
    let target_id = original_id.to_obj_id();
    store.link_object(&src_id, &target_id).await?;
    info!("link object ok! {}", src_id.to_string());
    // Verify both chunks exist
    let (is_exist, size) = store.is_chunk_exist(&original_id, None).await?;
    assert!(is_exist);
    assert_eq!(size, data.len() as u64);
    let (is_exist, size) = store.is_chunk_exist(&linked_id, None).await?;
    assert!(is_exist);
    assert_eq!(size, data.len() as u64);
    Ok(())
}

//测试 open_chunk_writer
#[tokio::test]
async fn test_open_chunk_writer() -> NdnResult<()> {
    let store = create_test_store().await?;
    let chunk_id = ChunkId::new("sha256:abcdef1234567890").unwrap();
    let data = b"chunk writer test data".to_vec();

    let mut writer = store
        .open_new_chunk_writer(&chunk_id, data.len() as u64)
        .await?;
    // Open chunk writer

    // Write data to chunk
    writer.write_all(&data).await.map_err(|e| {
        warn!(
            "test_open_chunk_writer: write data failed! {}",
            e.to_string()
        );
        NdnError::IoError(e.to_string())
    })?;
    writer.flush().await.map_err(|e| {
        warn!(
            "test_open_chunk_writer: flush data failed! {}",
            e.to_string()
        );
        NdnError::IoError(e.to_string())
    })?;
    info!("test_open_chunk_writer: write data ok!");
    drop(writer);
    store.complete_chunk_writer(&chunk_id).await?;
    // Verify chunk exists and data is correct
    let (is_exist, size) = store.is_chunk_exist(&chunk_id, Some(false)).await?;
    assert!(is_exist);
    assert_eq!(size, data.len() as u64);

    let (mut reader, chunk_size) = store
        .open_chunk_reader(&chunk_id, SeekFrom::Start(0))
        .await?;
    let mut buffer = vec![0u8; data.len()];
    reader.read_exact(&mut buffer).await.unwrap();
    assert_eq!(buffer, data);

    Ok(())
}

#[tokio::test]
async fn test_object_operations() -> NdnResult<()> {
    let store = create_test_store().await?;
    let obj_json = serde_json::json!({"name": "test object", "data": "test data"});
    let (obj_id, obj_str) = build_named_object_by_json("myobj", &obj_json);
    //let obj_id = ObjId::new("object1".to_string());

    // Test putting object
    store.put_object(&obj_id, &obj_str, false).await?;
    info!("put object ok! {}", obj_id.to_string());

    // Verify object exists
    assert!(store.is_object_exist(&obj_id).await?);

    // Test getting object
    let retrieved_obj = store.get_object(&obj_id).await?;
    assert_eq!(retrieved_obj.to_string(), obj_str);

    // Test object linking
    let linked_id = ObjId::new("test:2222222222222222").unwrap();
    //let link = ObjectLink { link_obj_id: linked_id.clone() };
    store.link_object(&linked_id, &obj_id).await?;
    info!(
        "link object ok! {}->{}",
        linked_id.to_string(),
        obj_id.to_string()
    );
    // Verify linked object exists
    assert!(store.is_object_exist(&linked_id).await?);

    let ref_obj_ids = store.query_link_refs(&obj_id).await?;
    assert_eq!(ref_obj_ids.len(), 1);
    for ref_obj_id in ref_obj_ids {
        info!("query_link_refs ok! {}", ref_obj_id.to_string());
    }

    Ok(())
}
