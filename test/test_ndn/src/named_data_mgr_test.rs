use log::*;
use sha2::{Digest, Sha256};
use std::io::SeekFrom;

use ndn_lib::{ChunkId, NamedDataMgr, NdnError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct NamedDataMgrTest;

impl NamedDataMgrTest {
    pub async fn write_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId, chunk_data: &[u8]) {
        let (mut chunk_writer, _progress_info) =
            NamedDataMgr::open_chunk_writer(Some(ndn_mgr_id), chunk_id, chunk_data.len() as u64, 0)
                .await
                .expect("open chunk writer failed");
        chunk_writer
            .write_all(chunk_data)
            .await
            .expect("write chunk to ndn-mgr failed");
        NamedDataMgr::complete_chunk_writer(Some(ndn_mgr_id), chunk_id)
            .await
            .expect("wait chunk writer complete failed.");
    }

    pub async fn read_chunk(ndn_mgr_id: &str, chunk_id: &ChunkId) -> Vec<u8> {
        let (mut chunk_reader, len) =
            NamedDataMgr::open_chunk_reader(Some(ndn_mgr_id), chunk_id, SeekFrom::Start(0), false)
                .await
                .expect("open reader from ndn-mgr failed.");

        let mut buffer = vec![0u8; len as usize];
        chunk_reader
            .read_exact(&mut buffer)
            .await
            .expect("read chunk from ndn-mgr failed");

        buffer
    }

    pub async fn read_chunk_with_check(
        ndn_mgr_id: &str,
        chunk_id: &ChunkId,
        expect: &[u8],
    ) -> Vec<u8> {
        let buffer = Self::read_chunk(ndn_mgr_id, chunk_id).await;
        assert_eq!(buffer.len(), expect.len(), "chunk length mismatch");
        assert_eq!(
            Sha256::digest(buffer.as_slice()),
            Sha256::digest(expect),
            "chunk data mismatch"
        );
        buffer
    }

    pub async fn open_chunk_reader_not_found(ndn_mgr_id: &str, chunk_id: &ChunkId) {
        let ret =
            NamedDataMgr::open_chunk_reader(Some(ndn_mgr_id), &chunk_id, SeekFrom::Start(0), false)
                .await;

        match ret {
            Ok(_) => assert!(false, "should no chunk found"),
            Err(err) => match err {
                NdnError::NotFound(_) => {
                    info!("Chunk not found as expected");
                }
                _ => {
                    assert!(false, "Unexpected error type, {:?}", err);
                }
            },
        }
    }
}
