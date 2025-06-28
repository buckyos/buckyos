use crate::chunk::*;
use crate::hash::HashMethod;
use crate::NamedDataMgrRef;
use crate::{NamedDataMgr, NamedDataMgrConfig};
use crate::object_array::{
    ObjectArrayStorageFactory, ObjectArrayStorageType, GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY,
};
use chrono::offset;
use rand::Rng;
use rand::SeedableRng;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

async fn gen_random_file(seed: u64, len: usize, path: &Path) {
    println!("Generating random file at {:?} with size {}", path, len);
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    // Create target file and fill it with random data
    let mut file = tokio::fs::File::create(path).await.unwrap();
    let mut data = vec![0u8; 1024 * 64];
    let mut total_written = 0;
    while total_written < len {
        let to_write = std::cmp::min(len - total_written, data.len());
        if to_write == 0 {
            break; // No more data to write
        }

        rng.fill(&mut data[..to_write]);
        file.write_all(&data[..to_write]).await.unwrap();

        total_written += to_write;
    }

    file.flush().await.unwrap();
    println!(
        "Generated random file at {:?} with size {}",
        path, total_written
    );

    // Check file size
    let file_size = file.metadata().await.unwrap().len() as usize;
    if file_size != len {
        panic!("File size mismatch: expected {}, got {}", len, file_size);
    }
}

async fn gen_fix_size_chunk_list_from_file(file_path: &Path, chunk_size: usize) -> ChunkList {
    let chunk_mgr = NamedDataMgr::get_named_data_mgr_by_id(None).await.unwrap();

    let mut file = tokio::fs::File::open(file_path).await.unwrap();
    let file_size = file.metadata().await.unwrap().len() as usize;

    let mut builder = ChunkListBuilder::new(HashMethod::Sha256)
        .with_fixed_size(chunk_size as u64)
        .with_total_size(file_size as u64);

    let mut chunk_data = vec![0u8; chunk_size];
    for (i, offset) in (0..file_size).step_by(chunk_size).enumerate() {
        let size = std::cmp::min(chunk_size, file_size - offset);
        // println!("Reading chunk at offset: {}, {}, {}", i, offset, size);

        // Read the chunk data
        file.read_exact(&mut chunk_data[..size]).await.unwrap();

        let mut hasher = ChunkHasher::new_with_hash_type(HashMethod::Sha256).unwrap();
        let mix_chunk_id = hasher.calc_mix_chunk_id_from_bytes(&chunk_data[..size]);

        let length = mix_chunk_id.get_length().unwrap_or(0);
        assert_eq!(length as usize, size, "Chunk length mismatch");

        // Write the chunk data to chunk manager
        let (mut writer, _) = NamedDataMgr::open_chunk_writer(None, &mix_chunk_id, size as u64, 0)
            .await
            .unwrap();
        writer.write_all(&chunk_data[..size]).await.unwrap();
        NamedDataMgr::complete_chunk_writer(None, &mix_chunk_id)
            .await
            .unwrap();

        {
            let mut hasher = ChunkHasher::new_with_hash_type(HashMethod::Sha256).unwrap();
            let chunk_id = hasher.calc_chunk_id_from_bytes(&chunk_data[..size]);

            assert_eq!(
                chunk_id.get_length().unwrap_or(0) as usize,
                0,
                "Chunk ID length mismatch"
            );
            let chunk_hash = chunk_id.get_hash();
            assert!(chunk_hash.len() > 0, "Chunk hash should not be empty");
            assert_eq!(chunk_hash, mix_chunk_id.get_hash(), "Chunk hash mismatch");
        }

        builder.append(mix_chunk_id).unwrap();
    }

    println!("Append all chunks to builder");

    let chunk_list = builder.build().await.unwrap();
    let (chunk_list_id, body) = chunk_list.calc_obj_id();
    println!("Generated chunk list ID: {}", chunk_list_id.to_base32());

    chunk_list
}

async fn test_read_chunk_list(target_file: &Path, chunk_list: &ChunkList, offset: u64) {
    let named_data_mgr = NamedDataMgr::get_named_data_mgr_by_id(None).await.unwrap();
    let auto_cache = true;

    // Load the chunk reader for the specified chunk index and offset
    let mut reader = ChunkListReader::new(
        named_data_mgr.clone(),
        chunk_list.clone().unwrap(),
        std::io::SeekFrom::Start(offset),
        false,
    )
    .await
    .unwrap();

    // Read data from the chunk list and verify the content
    let total_size = chunk_list.total_size();
    let mut file = tokio::fs::File::open(target_file).await.unwrap();
    let file_size = file.metadata().await.unwrap().len() as u64;
    assert_eq!(total_size, file_size, "Chunk list total size mismatch");

    let mut buf = vec![0u8; 1024];
    let mut file_buf = vec![0u8; 1024];

    // Read from offset to the end of the chunk list, stepping through 1024 bytes at a time
    for pos in (offset..total_size).step_by(buf.len()) {
        let read_size = std::cmp::min(buf.len(), (total_size - pos) as usize);
        reader.read_exact(&mut buf[..read_size]).await.unwrap();

        // Verify the content matches the original file
        file.seek(std::io::SeekFrom::Start(pos)).await.unwrap();
        file.read_exact(&mut file_buf[..read_size]).await.unwrap();

        assert_eq!(
            &buf[..read_size],
            &file_buf[..read_size],
            "Data mismatch at offset {}",
            offset
        );
    }

    println!(
        "Successfully read chunk list from offset {} with size {}",
        offset, total_size
    );
}
#[tokio::test]
async fn test_chunk_list_main() {
    let data_dir = std::env::temp_dir().join("ndn_chunk_test");
    if !data_dir.exists() {
        // tokio::fs::remove_dir_all(&data_dir).await.unwrap();
        tokio::fs::create_dir_all(&data_dir).await.unwrap();
    }

    // Init NamedDataMgr
    let test_dir = data_dir.join("test_ndn_mgr");
    if test_dir.exists() {
        println!("Removing existing test directory: {:?}", test_dir);
        tokio::fs::remove_dir_all(&test_dir).await.unwrap();
    }
    if !test_dir.exists() {
        tokio::fs::create_dir_all(&test_dir).await.unwrap();
    }

    let config = NamedDataMgrConfig {
        local_stores: vec![test_dir.to_str().unwrap().to_string()],
        local_cache: None,
        mmap_cache_dir: None,
    };

    let chunk_mgr = NamedDataMgr::from_config(
        Some("test_chunk_list".to_string()),
        test_dir.clone(),
        config,
    )
    .await
    .unwrap();
    NamedDataMgr::set_mgr_by_id(None, chunk_mgr).await.unwrap();

    // Init ObjectArray Storage Factory for later use
    let factory = ObjectArrayStorageFactory::new(&data_dir);
    if let Err(_) = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.set(factory) {
        error!("Object array storage factory already initialized");
    }

    let seed = 123456789;
    let file_size = 1024 * 1024 * 16 + 100;
    let chunk_size = 1024 * 1024; // 1MB

    let file_path = data_dir.join("test_file.bin");
    if file_path.exists() {
        println!("Removing existing file: {:?}", file_path);
        tokio::fs::remove_file(&file_path).await.unwrap();
    }

    // Generate a random file
    gen_random_file(seed, file_size, &file_path).await;

    // Generate a fixed-size chunk list from the file
    let chunk_list = gen_fix_size_chunk_list_from_file(&file_path, chunk_size).await;

    // Validate the generated chunk list
    assert_eq!(chunk_list.total_size(), file_size as u64);
    assert_eq!(chunk_list.len(), file_size / chunk_size + 1);

    println!(
        "Chunk list generated successfully with {} chunks",
        chunk_list.len()
    );

    assert_eq!(
        chunk_list.storage_type(),
        ObjectArrayStorageType::Arrow,
        "Chunk list storage type should be Arrow"
    );

    // Test reading the chunk list
    test_read_chunk_list(&file_path, &chunk_list, 0).await;
    test_read_chunk_list(&file_path, &chunk_list, 1024 * 5 + 100).await;
    test_read_chunk_list(&file_path, &chunk_list, file_size as u64).await;
}
