use super::*;
use crate::{ChunkId, FileObject, NamedDataMgr, NamedDataMgrConfig, NdnError, NdnResult, ObjId};
use buckyos_kit::*;
use std::io::SeekFrom;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn test_basic_chunk_operations() -> NdnResult<()> {
    // Create a temporary directory for testing
    let test_dir = tempdir().unwrap();
    let config = NamedDataMgrConfig {
        local_stores: vec![test_dir.path().to_str().unwrap().to_string()],
        local_cache: None,
        mmap_cache_dir: None,
    };

    let chunk_mgr = NamedDataMgr::from_config(
        Some("test".to_string()),
        test_dir.path().to_path_buf(),
        config,
    )
    .await?;

    // Create test data
    let test_data = b"Hello, World!";
    let chunk_id = ChunkId::new("sha256:1234567890abcdef").unwrap();

    // Write chunk
    let (mut writer, _) = chunk_mgr
        .open_chunk_writer_impl(&chunk_id, test_data.len() as u64, 0)
        .await
        .unwrap();
    writer.write_all(test_data).await.unwrap();
    chunk_mgr
        .complete_chunk_writer_impl(&chunk_id)
        .await
        .unwrap();

    // Read and verify chunk
    let (mut reader, size) = chunk_mgr
        .open_chunk_reader_impl(&chunk_id, SeekFrom::Start(0), true)
        .await
        .unwrap();
    assert_eq!(size, test_data.len() as u64);
    drop(chunk_mgr);

    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer).await.unwrap();
    assert_eq!(&buffer, test_data);

    Ok(())
}

#[tokio::test]
async fn test_base_operations() -> NdnResult<()> {
    // Create a temporary directory for testing
    init_logging("ndn-lib test", false);
    let test_dir = tempdir().unwrap();
    let config = NamedDataMgrConfig {
        local_stores: vec![test_dir.path().to_str().unwrap().to_string()],
        local_cache: None,
        mmap_cache_dir: None,
    };

    let named_mgr = NamedDataMgr::from_config(
        Some("test".to_string()),
        test_dir.path().to_path_buf(),
        config,
    )
    .await?;

    // Create test data
    let test_data = b"Hello, Path Test!";
    let chunk_id = ChunkId::new("sha256:1234567890abcdef").unwrap();
    let test_path = "/test/file.txt".to_string();

    // Write chunk
    let (mut writer, _) = named_mgr
        .open_chunk_writer_impl(&chunk_id, test_data.len() as u64, 0)
        .await?;
    writer.write_all(test_data).await.unwrap();
    named_mgr
        .complete_chunk_writer_impl(&chunk_id)
        .await
        .unwrap();

    // Bind chunk to path
    named_mgr
        .create_file_impl(
            test_path.as_str(),
            &chunk_id.to_obj_id(),
            "test_app",
            "test_user",
        )
        .await?;

    // Read through path and verify
    let (mut reader, size, retrieved_chunk_id) = named_mgr
        .get_chunk_reader_by_path_impl(
            test_path.as_str(),
            "test_user",
            "test_app",
            SeekFrom::Start(0),
        )
        .await?;

    assert_eq!(size, test_data.len() as u64);
    assert_eq!(retrieved_chunk_id, chunk_id);

    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer).await.unwrap();
    assert_eq!(&buffer, test_data);

    //test fileobj
    let path2 = "/test/file2.txt".to_string();
    let file_obj = FileObject::new(path2.clone(), test_data.len() as u64, chunk_id.to_string());
    let (file_obj_id, file_obj_str) = file_obj.gen_obj_id();
    info!("file_obj_id:{}", file_obj_id.to_string());
    //file-obj is soft linke to chunk-obj
    named_mgr
        .put_object_impl(&file_obj_id, &file_obj_str)
        .await?;

    let obj_content = named_mgr
        .get_object_impl(&file_obj_id, Some("/content".to_string()))
        .await?;
    info!("obj_content:{}", obj_content);
    assert_eq!(obj_content.as_str().unwrap(), chunk_id.to_string().as_str());

    let (the_chunk_id, path_obj_jwt, inner_obj_path) = named_mgr
        .select_obj_id_by_path_impl(test_path.as_str())
        .await?;
    info!("chunk_id:{}", chunk_id.to_string());
    info!("inner_obj_path:{}", inner_obj_path.unwrap());
    let obj_id_of_chunk = chunk_id.to_obj_id();
    assert_eq!(the_chunk_id, obj_id_of_chunk);

    // Test remove file
    named_mgr.remove_file_impl(&test_path).await.unwrap();

    // Verify path is removed
    let result = named_mgr
        .get_chunk_reader_by_path_impl(
            test_path.as_str(),
            "test_user",
            "test_app",
            SeekFrom::Start(0),
        )
        .await;
    assert!(result.is_err());

    Ok(())
}

//test get_chunk_mgr_by_id，然后再创建并写入一个chunk，再读取
#[tokio::test]
async fn test_get_chunk_mgr_by_id() -> NdnResult<()> {
    // Get ChunkMgr by id
    let random_mgr_id = rand::random::<u64>();
    //println!("random_mgr_id: {}", random_mgr_id);
    let chunk_mgr_id = format!("test_{}", random_mgr_id);
    let mgr_id = Some(chunk_mgr_id.as_str());
    let chunk_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
    assert!(chunk_mgr.is_some());
    let chunk_mgr = chunk_mgr.unwrap();

    // Create test data
    let test_data = b"Hello, ChunkMgr Test!";
    let chunk_id = ChunkId::new("sha256:abcdef1234567890AB").unwrap();

    // Write chunk
    {
        let mut chunk_mgr = chunk_mgr.lock().await;
        let (mut writer, _) = chunk_mgr
            .open_chunk_writer_impl(&chunk_id, test_data.len() as u64, 0)
            .await
            .unwrap();
        writer.write_all(test_data).await.unwrap();
        chunk_mgr
            .complete_chunk_writer_impl(&chunk_id)
            .await
            .unwrap();
    }

    // Read chunk and verify
    {
        let chunk_mgr = chunk_mgr.lock().await;
        let (mut reader, size) = chunk_mgr
            .open_chunk_reader_impl(&chunk_id, SeekFrom::Start(0), true)
            .await?;
        assert_eq!(size, test_data.len() as u64);
        drop(chunk_mgr);

        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).await.unwrap();
        assert_eq!(&buffer, test_data);
    }

    Ok(())
}

#[test]
fn test_path_normalization() {
    let test_cases = vec![
        ("//a//b//c", "/a/b/c"),
        ("./a/b/c", "a/b/c"),
        ("/a/b/c", "/a/b/c"),
        ("a/b/c", "a/b/c"),
    ];

    for (input, expected) in test_cases {
        let result = NamedDataMgrDB::normalize_path(input);
        assert_eq!(result, expected, "Failed to normalize path: {}", input);
    }
}

#[tokio::test]
async fn test_find_longest_matching_path_edge_cases() -> NdnResult<()> {
    // Create a temporary directory for testing
    init_logging("ndn-lib test", false);
    let test_dir = tempdir().unwrap();
    let config = NamedDataMgrConfig {
        local_stores: vec![test_dir.path().to_str().unwrap().to_string()],
        local_cache: None,
        mmap_cache_dir: None,
    };

    let chunk_mgr = NamedDataMgr::from_config(
        Some("test".to_string()),
        test_dir.path().to_path_buf(),
        config,
    )
    .await?;

    // Create test data and paths
    let test_data1 = b"Test data for path 1";
    let test_data2 = b"Test data for path 2";
    let test_data3 = b"Test data for path 3";

    let chunk_id1 = ChunkId::new("sha256:1111111111111111").unwrap();
    let chunk_id2 = ChunkId::new("sha256:2222222222222222").unwrap();
    let chunk_id3 = ChunkId::new("sha256:3333333333333333").unwrap();

    let base_path = "/test/path";
    let sub_path1 = "/test/path/file1.txt";
    let sub_path2 = "/test/path/subdir";
    let sub_path3 = "/test/path/subdir/file2.txt";

    // Write chunks
    let (mut writer1, _) = chunk_mgr
        .open_chunk_writer_impl(&chunk_id1, test_data1.len() as u64, 0)
        .await?;
    writer1.write_all(test_data1).await.unwrap();
    chunk_mgr
        .complete_chunk_writer_impl(&chunk_id1)
        .await
        .unwrap();

    let (mut writer2, _) = chunk_mgr
        .open_chunk_writer_impl(&chunk_id2, test_data2.len() as u64, 0)
        .await?;
    writer2.write_all(test_data2).await.unwrap();
    chunk_mgr
        .complete_chunk_writer_impl(&chunk_id2)
        .await
        .unwrap();

    let (mut writer3, _) = chunk_mgr
        .open_chunk_writer_impl(&chunk_id3, test_data3.len() as u64, 0)
        .await?;
    writer3.write_all(test_data3).await.unwrap();
    chunk_mgr
        .complete_chunk_writer_impl(&chunk_id3)
        .await
        .unwrap();

    // Bind chunks to paths
    chunk_mgr
        .create_file_impl(base_path, &chunk_id1.to_obj_id(), "test_app", "test_user")
        .await?;

    //chunk_mgr.sigh_path_obj(base_path path_obj_jwt).await?;
    info!("Created base path: {}", base_path);

    chunk_mgr
        .create_file_impl(sub_path1, &chunk_id2.to_obj_id(), "test_app", "test_user")
        .await?;
    info!("Created sub path 1: {}", sub_path1);

    chunk_mgr
        .create_file_impl(sub_path2, &chunk_id3.to_obj_id(), "test_app", "test_user")
        .await?;
    info!("Created sub path 2: {}", sub_path2);

    // Test find_longest_matching_path

    // Test case 1: Exact match
    info!("Test case 1: Exact match with {}", sub_path1);
    let (result_path, obj_id, path_obj_jwt, relative_path) =
        chunk_mgr.db().find_longest_matching_path(sub_path1)?;
    info!(
        "Result: path={}, obj_id={}, relative_path={:?}",
        result_path,
        obj_id.to_string(),
        relative_path
    );
    assert_eq!(result_path, sub_path1);
    assert_eq!(obj_id, chunk_id2.to_obj_id());
    assert_eq!(relative_path, Some("".to_string()));

    // Test case 2: Match with a parent path
    let test_path = "/test/path/subdir/file2.txt";
    info!("Test case 2: Match with parent path. Testing {}", test_path);
    let (result_path, obj_id, path_obj_jwt, relative_path) =
        chunk_mgr.db().find_longest_matching_path(test_path)?;
    info!(
        "Result: path={}, obj_id={}, relative_path={:?}",
        result_path,
        obj_id.to_string(),
        relative_path
    );
    assert_eq!(result_path, sub_path2);
    assert_eq!(obj_id, chunk_id3.to_obj_id());
    assert_eq!(relative_path, Some("/file2.txt".to_string()));

    // Test case 3: Match with the base path
    let test_path = "/test/path/unknown/file.txt";
    info!("Test case 3: Match with base path. Testing {}", test_path);
    let (result_path, obj_id, path_obj_jwt, relative_path) =
        chunk_mgr.db().find_longest_matching_path(test_path)?;
    info!(
        "Result: path={}, obj_id={}, relative_path={:?}",
        result_path,
        obj_id.to_string(),
        relative_path
    );
    assert_eq!(result_path, base_path);
    assert_eq!(obj_id, chunk_id1.to_obj_id());
    assert_eq!(relative_path, Some("/unknown/file.txt".to_string()));

    // Test case 4: No match (should return error)
    let test_path = "/other/path/file.txt";
    info!("Test case 4: No match. Testing {}", test_path);
    let result = chunk_mgr.db().find_longest_matching_path(test_path);
    match result {
        Ok(_) => {
            panic!("Expected error for path with no match, but got success");
        }
        Err(e) => {
            info!("Got expected error for non-matching path: {}", e);
            // Verify it's the expected error type
            match e {
                NdnError::DbError(_) => {
                    // This is the expected error type
                    info!("Error type is correct: DbError");
                }
                _ => {
                    panic!("Expected DbError, but got different error type: {:?}", e);
                }
            }
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_concurrent_path_access() -> NdnResult<()> {
    init_logging("ndn-lib test", false);
    let test_dir = tempdir().unwrap();
    let config = NamedDataMgrConfig {
        local_stores: vec![test_dir.path().to_str().unwrap().to_string()],
        local_cache: None,
        mmap_cache_dir: None,
    };

    let named_mgr = Arc::new(tokio::sync::Mutex::new(
        NamedDataMgr::from_config(
            Some("test".to_string()),
            test_dir.path().to_path_buf(),
            config,
        )
        .await?,
    ));

    let test_data = b"Test data for concurrent access";
    let chunk_id = ChunkId::new("sha256:1234").unwrap();
    let test_path = "/test/concurrent/path.txt";

    let (mut writer, _) = named_mgr
        .lock()
        .await
        .open_chunk_writer_impl(&chunk_id, test_data.len() as u64, 0)
        .await?;
    writer.write_all(test_data).await.unwrap();
    named_mgr
        .lock()
        .await
        .complete_chunk_writer_impl(&chunk_id)
        .await
        .unwrap();

    named_mgr
        .lock()
        .await
        .create_file_impl(test_path, &chunk_id.to_obj_id(), "test_app", "test_user")
        .await?;

    // 创建多个任务并发访问
    let mut handles = vec![];
    for i in 0..10 {
        let named_mgr_clone = named_mgr.clone();
        let chunk_id2 = chunk_id.clone();
        let handle = tokio::spawn(async move {
            let result = named_mgr_clone
                .lock()
                .await
                .db()
                .find_longest_matching_path(test_path);
            assert!(result.is_ok());
            let (result_path, obj_id, _, _) = result.unwrap();
            assert_eq!(result_path, test_path);
            assert_eq!(obj_id, chunk_id2.to_obj_id());
        });
        handles.push(handle);
    }

    // 等待所有任务完成
    for handle in handles {
        handle.await.unwrap();
    }

    Ok(())
}
