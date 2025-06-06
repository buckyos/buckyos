use crate::chunk::*;
use crate::hash::HashMethod;
use rand::Rng;
use rand::SeedableRng;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::object_array::{ObjectArrayStorageFactory, GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY};


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
    let mut file = tokio::fs::File::open(file_path).await.unwrap();
    let file_size = file.metadata().await.unwrap().len() as usize;

    let mut builder = ChunkListBuilder::new(HashMethod::Sha256, None)
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
    let (chunk_list_id, body) = chunk_list.calc_id();
    println!("Generated chunk list ID: {}", chunk_list_id.to_base32());

    chunk_list
}

#[tokio::test]
async fn test_chunk_list_main() {
    let data_dir = std::env::temp_dir().join("ndn_chunk_test");
    if !data_dir.exists() {
        // tokio::fs::remove_dir_all(&data_dir).await.unwrap();
        tokio::fs::create_dir_all(&data_dir).await.unwrap();
    }

    let factory = ObjectArrayStorageFactory::new(&data_dir);
    if let Err(_) = GLOBAL_OBJECT_ARRAY_STORAGE_FACTORY.set(factory) {
        error!("Object array storage factory already initialized");
    }

    let seed = 123456789;
    let file_size = 1024 * 1024 * 16 + 100;
    let chunk_size = 1024 * 4; // 1 KB

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
    assert_eq!(chunk_list.get_total_size(), file_size as u64);
    assert_eq!(
        chunk_list.get_len() ,
        file_size / chunk_size + 1
    );

    println!(
        "Chunk list generated successfully with {} chunks",
        chunk_list.len()
    );
}
