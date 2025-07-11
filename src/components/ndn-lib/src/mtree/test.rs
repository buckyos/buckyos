use super::calculator::SerializeHashCalculator;
use super::locator::HashNodeLocator;
use super::mtree::{MerkleTreeObject, MerkleTreeObjectGenerator, MerkleTreeProofPathVerifier};
use super::stream::{
    MtreeReadSeek, MtreeReadWriteSeekWithSharedBuffer, MtreeWriteSeek, SharedBuffer,
};
use crate::hash::HashMethod;
use crate::HashHelper;
use std::io::SeekFrom;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::test;

#[test]
async fn test_locator() {
    let total_depth = HashNodeLocator::calc_depth(6);
    println!("Total depth: {}", total_depth);
    assert!(total_depth == 3);

    let total_depth = HashNodeLocator::calc_depth(5);
    println!("Total depth: {}", total_depth);
    assert!(total_depth == 3);

    let total_depth = HashNodeLocator::calc_depth(4);
    println!("Total depth: {}", total_depth);
    assert!(total_depth == 2);

    let total_depth = HashNodeLocator::calc_depth(2);
    println!("Total depth: {}", total_depth);
    assert!(total_depth == 1);

    let counts = HashNodeLocator::calc_count_per_depth(3);
    println!("Counts: {:?}", counts);

    let counts = HashNodeLocator::calc_count_per_depth(6);
    println!("Counts: {:?}", counts);

    let counts = HashNodeLocator::calc_prev_count_per_depth(6);
    println!("Prev counts: {:?}", counts);

    let counts = HashNodeLocator::calc_count_per_depth(10);
    println!("Counts: {:?}", counts);

    let counts = HashNodeLocator::calc_prev_count_per_depth(10);
    println!("Prev counts: {:?}", counts);

    let locator = HashNodeLocator::new(6);
    let indexes = locator.get_proof_path_by_leaf_index(0).unwrap();
    println!("Indexes: {:?}", indexes);

    let indexes = locator.get_proof_path_by_leaf_index(5).unwrap();
    println!("Indexes: {:?}", indexes);

    assert_eq!(HashNodeLocator::calc_total_count(2), 3);
    assert_eq!(HashNodeLocator::calc_total_count(3), 7);
    assert_eq!(HashNodeLocator::calc_total_count(4), 7);
    assert_eq!(HashNodeLocator::calc_total_count(5), 13);
    assert_eq!(HashNodeLocator::calc_total_count(6), 13);

    assert_eq!(HashNodeLocator::calc_total_count(10), 23);
}

// Get file size and then calc leaf count of the file
async fn get_leaf_count_of_file(file: &File, chunk_size: usize) -> u64 {
    // Get file size and then calc leaf count of the file
    let file_size = file.metadata().await.unwrap().len();
    assert!(file_size > 0);

    let mut leaf_count = file_size / chunk_size as u64;
    if file_size % chunk_size as u64 != 0 {
        leaf_count += 1;
    }

    println!("File size: {}, Leaf count: {}", file_size, leaf_count);
    leaf_count
}

async fn read_chunk(file: &mut File, chunk_size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; chunk_size];

    let mut total_read = 0;
    while total_read < chunk_size {
        match file.read(&mut buf[total_read..]).await.unwrap() {
            0 => {
                // EOF
                break;
            }
            n => {
                total_read += n;
            }
        }
    }

    // println!("Read {} bytes", total_read);
    // Truncate the buffer to the actual read size
    if total_read < chunk_size {
        buf.truncate(total_read);
    }

    buf
}

//#[test]
async fn test_generator() {
    let test_file: &str = "D:\\test";

    let chunk_size = 1024 * 1024 * 4;

    let stream;
    let root_hash;
    {
        let mut file = tokio::fs::File::open(test_file).await.unwrap();

        let data_size = file.metadata().await.unwrap().len();

        let total = MerkleTreeObjectGenerator::estimate_output_bytes(
            data_size,
            chunk_size,
            Some(HashMethod::Sha256),
        );
        println!("Estimated output bytes: {}", total);

        let buf = SharedBuffer::with_size(total as usize);
        stream = MtreeReadWriteSeekWithSharedBuffer::new(buf);
        let writer = Box::new(stream.clone()) as Box<dyn MtreeWriteSeek>;

        let mut gen = MerkleTreeObjectGenerator::new(
            data_size,
            chunk_size as u64,
            Some(HashMethod::Sha256),
            writer,
        )
        .await
        .unwrap();

        let leaf_count = get_leaf_count_of_file(&file, chunk_size as usize).await;
        let mut hash_list = Vec::new();
        loop {
            let buf = read_chunk(&mut file, chunk_size as usize).await;
            if buf.len() == 0 {
                break;
            }

            let hash = HashHelper::calc_hash(HashMethod::Sha256, &buf);
            hash_list.push(hash.to_vec());
        }

        assert!(hash_list.len() == leaf_count as usize);
        gen.append_leaf_hashes(&hash_list).await.unwrap();

        root_hash = gen.finalize().await.unwrap();
        println!("Root hash: {:?}", root_hash);
    }

    {
        // Create mtree object and load from buf previously without verify
        let mut stream = stream.clone();
        stream.seek(SeekFrom::Start(0)).await.unwrap();
        let reader = Box::new(stream.clone()) as Box<dyn MtreeReadSeek>;
        let mut obj = MerkleTreeObject::load_from_reader(reader, false)
            .await
            .unwrap();

        let root_hash1 = obj.get_root_hash();
        println!("Root hash: {:?}", root_hash1);
        assert_eq!(root_hash, root_hash1);
    }

    {
        // Create mtree object and load from buf previously
        let mut stream = stream.clone();
        stream.seek(SeekFrom::Start(0)).await.unwrap();
        let reader = Box::new(stream.clone()) as Box<dyn MtreeReadSeek>;
        let mut obj = MerkleTreeObject::load_from_reader(reader, true)
            .await
            .unwrap();

        let root_hash1 = obj.get_root_hash();
        println!("Root hash: {:?}", root_hash1);
        assert_eq!(root_hash, root_hash1);

        for leaf_index in 0..obj.get_leaf_count() {
            // Verify the proof path for the leaf node
            let proof_verify = MerkleTreeProofPathVerifier::new(HashMethod::Sha256);
            let mut proof = obj.get_proof_path_by_leaf_index(leaf_index).await.unwrap();

            // Proof last node must be the root node hash
            assert_eq!(proof[proof.len() - 1].1, root_hash);

            assert_eq!(proof_verify.verify(&proof).unwrap(), true);

            // Replace leaf node hash with error hash, then verify will failed!
            println!("Proof leaf node: {:?}", proof[0]);
            proof[0].1[0] = !proof[0].1[0];
            println!("Error proof leaf node: {:?}", proof[0]);
            assert_eq!(proof_verify.verify(&proof).unwrap(), false);
        }
    }
}

//#[test]
async fn test_serialize_hash_calculator() {
    //let test_file: &str =
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test.shc.data");
    println!("test_file: {}", test_file.to_str().unwrap());
    let chunk_size = 1024 * 64;

    let mut root_hash1;
    let mut root_hash2;
    {
        let mut file = tokio::fs::File::open(test_file.clone()).await.unwrap();
        let leaf_count = get_leaf_count_of_file(&file, chunk_size).await;

        // Read the file by chunk and calculate the leaf node hashes
        let mut calculator =
            SerializeHashCalculator::new(leaf_count, HashMethod::Sha256, None, None);
        let mut buf = vec![0u8; chunk_size];
        let mut hash_list = Vec::new();
        loop {
            let buf = read_chunk(&mut file, chunk_size).await;
            if buf.len() == 0 {
                break;
            }

            let hash = HashHelper::calc_hash(HashMethod::Sha256, &buf);
            hash_list.push(hash.to_vec());
        }

        assert!(hash_list.len() == leaf_count as usize);
        calculator.append_leaf_hashes(&hash_list).await.unwrap();
        root_hash1 = calculator.finalize().await.unwrap();
        println!("Root hash: {:?}", root_hash1);
    }

    {
        let mut file = tokio::fs::File::open(test_file.clone()).await.unwrap();
        let leaf_count = get_leaf_count_of_file(&file, chunk_size).await;

        let size =
            SerializeHashCalculator::estimate_output_bytes(leaf_count, HashMethod::Sha256) as usize;

        let data = SharedBuffer::with_size(size);
        let buffer = MtreeReadWriteSeekWithSharedBuffer::new(data);

        let mut writer = Box::new(buffer.clone()) as Box<dyn MtreeWriteSeek>;

        // Read the file by chunk and calculate the leaf node hashes
        let mut calculator =
            SerializeHashCalculator::new(leaf_count, HashMethod::Sha256, Some(writer), None);
        let mut buf = vec![0u8; chunk_size];

        loop {
            let buf = read_chunk(&mut file, chunk_size).await;
            if buf.len() == 0 {
                break;
            }

            let hash = HashHelper::calc_hash(HashMethod::Sha256, &buf);
            calculator
                .append_leaf_hashes(&vec![hash.to_vec()])
                .await
                .unwrap();
        }

        root_hash2 = calculator.finalize().await.unwrap();
        println!("Root hash: {:?}", root_hash2);

        // print the whole buffer
        // println!("Buffer: {:?}", buffer.buffer().lock().unwrap());

        // Clone the buf from writer and then create a reader from it
        let mut reader = Box::new(buffer) as Box<dyn MtreeReadSeek>;
        reader.seek(SeekFrom::Start(0)).await.unwrap();

        // Calc with reader verify
        let mut calculator =
            SerializeHashCalculator::new(leaf_count, HashMethod::Sha256, None, Some(reader));
        let mut buf = vec![0u8; chunk_size];

        // Read the file at beginning
        file.seek(SeekFrom::Start(0)).await.unwrap();
        loop {
            let buf = read_chunk(&mut file, chunk_size).await;
            if buf.len() == 0 {
                break;
            }

            let hash = HashHelper::calc_hash(HashMethod::Sha256, &buf);
            calculator
                .append_leaf_hashes(&vec![hash.to_vec()])
                .await
                .unwrap();
        }

        let root_hash3 = calculator.finalize().await.unwrap();
        assert_eq!(root_hash2, root_hash3);
        // println!("Root hash: {:?}", root_hash3);
    }

    assert_eq!(root_hash1, root_hash2);
}
