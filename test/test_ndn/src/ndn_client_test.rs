use std::{ops::Range, path::PathBuf};

use log::*;
use ndn_lib::{
    CYFSHttpRespHeaders, ChunkId, ChunkReader, NdnClient, NdnError, ObjId,
    build_named_object_by_json,
};
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::{
    common::{DOWNLOAD_DIR, TEST_DIR},
    named_data_mgr_test::NamedDataMgrTest,
};

#[async_trait::async_trait]
pub trait NdnClientTest {
    async fn get_obj_by_url_with_check(
        &self,
        url: &str,
        known_obj_id: Option<&ObjId>,
        expect_obj: Option<&serde_json::Value>,
        expect_obj_str_and_type: Option<(&str, &str)>,
    ) -> (ObjId, serde_json::Value);
    async fn get_obj_by_url_err(&self, url: &str, known_obj_id: Option<&ObjId>) -> NdnError;
    async fn get_obj_by_url_not_found(&self, url: &str);
    async fn get_obj_by_url_invalid_id(&self, url: &str, known_obj_id: Option<&ObjId>);
    async fn pull_chunk_by_url_with_check(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        ndn_mgr_id: &str,
        expect: &[u8],
    ) -> Vec<u8>;
    async fn pull_chunk_by_url_err(&self, chunk_url: &str, chunk_id: &ChunkId) -> NdnError;
    async fn pull_chunk_by_url_not_found(&self, chunk_url: &str, chunk_id: &ChunkId);
    async fn pull_chunk_by_url_invalid_id(&self, chunk_url: &str, chunk_id: &ChunkId);
    async fn open_chunk_reader_by_url_err(
        &self,
        chunk_url: &str,
        chunk_id: Option<&ChunkId>,
    ) -> NdnError;
    async fn open_chunk_reader_by_url_not_found(&self, chunk_url: &str, chunk_id: Option<&ChunkId>);
    async fn open_chunk_reader_by_url_verify_error(
        &self,
        chunk_url: &str,
        chunk_id: Option<&ChunkId>,
    );
    async fn open_chunk_reader_by_url_with_check(
        &self,
        chunk_url: &str,
        expect_chunk_id: Option<&ChunkId>,
        expect_chunk: &[u8],
        root_obj_id: &ObjId,
    ) -> CYFSHttpRespHeaders;
    async fn open_chunk_reader_by_url_range_with_check(
        &self,
        chunk_url: &str,
        expect_chunk_id: Option<&ChunkId>,
        expect_chunk: &[u8],
        root_obj_id: &ObjId,
    );
    async fn download_chunk_to_local_with_check(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        no_verify: bool,
        expect_chunk: &[u8],
    ) -> (ChunkId, u64);
    async fn download_chunk_to_local_err(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        no_verify: bool,
    ) -> NdnError;
    async fn download_chunk_to_local_not_found(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        no_verify: bool,
    );
    async fn download_chunk_to_local_verify_failed(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        no_verify: bool,
    );
}

#[async_trait::async_trait]
impl NdnClientTest for NdnClient {
    async fn get_obj_by_url_with_check(
        &self,
        url: &str,
        known_obj_id: Option<&ObjId>,
        expect_obj: Option<&serde_json::Value>,
        expect_obj_str_and_type: Option<(&str, &str)>,
    ) -> (ObjId, serde_json::Value) {
        let (got_obj_id, got_obj) = self
            .get_obj_by_url(url, known_obj_id.map(|id| id.clone()))
            .await
            .expect("get obj fromfailed");

        if let Some(expect_id) = known_obj_id {
            assert_eq!(&got_obj_id, expect_id, "got obj-id from mismatch");
        }

        if let Some(expect_obj) = expect_obj {
            assert_eq!(&got_obj, expect_obj, "got obj from mismatch");
        }

        if let Some((expect_obj_str, obj_type)) = expect_obj_str_and_type {
            let (_, got_obj_str) = build_named_object_by_json(obj_type, &got_obj);
            assert_eq!(got_obj_str, *expect_obj_str, "got obj mismatch");
        }
        (got_obj_id, got_obj)
    }

    async fn get_obj_by_url_err(&self, url: &str, known_obj_id: Option<&ObjId>) -> NdnError {
        self.get_obj_by_url(url, known_obj_id.map(|id| id.clone()))
            .await
            .expect_err("get obj should failed")
    }

    async fn get_obj_by_url_not_found(&self, url: &str) {
        let err = self.get_obj_by_url_err(url, None).await;

        if let NdnError::NotFound(_) = err {
        } else {
            assert!(false, "unexpect error, should obj id not found: {:?}", err);
        }
    }

    async fn get_obj_by_url_invalid_id(&self, url: &str, known_obj_id: Option<&ObjId>) {
        let err = self.get_obj_by_url_err(url, known_obj_id).await;

        if let NdnError::InvalidId(_) = err {
        } else {
            assert!(
                false,
                "unexpect error, should obj id verify failed. {:?}",
                err
            );
        }
    }

    async fn pull_chunk_by_url_with_check(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        ndn_mgr_id: &str,
        expect: &[u8],
    ) -> Vec<u8> {
        self.pull_chunk_by_url(chunk_url.to_string(), chunk_id.clone(), Some(ndn_mgr_id))
            .await
            .expect("push chunk failed");

        NamedDataMgrTest::read_chunk_with_check(ndn_mgr_id, chunk_id, expect).await
    }

    async fn pull_chunk_by_url_err(&self, chunk_url: &str, chunk_id: &ChunkId) -> NdnError {
        self.pull_chunk_by_url(chunk_url.to_string(), chunk_id.clone(), None)
            .await
            .expect_err("pull chunk should failed")
    }

    async fn pull_chunk_by_url_not_found(&self, chunk_url: &str, chunk_id: &ChunkId) {
        let err = self.pull_chunk_by_url_err(chunk_url, chunk_id).await;

        match err {
            NdnError::NotFound(_) => {
                info!("real chunk not found as expected");
            }
            _ => {
                assert!(false, "Unexpected error type {:?}", err);
            }
        }
    }

    async fn pull_chunk_by_url_invalid_id(&self, chunk_url: &str, chunk_id: &ChunkId) {
        let err = self.pull_chunk_by_url_err(chunk_url, chunk_id).await;

        match err {
            NdnError::InvalidId(_) => {
                info!("real chunk not found as expected");
            }
            _ => {
                assert!(false, "Unexpected error type {:?}", err);
            }
        }
    }

    async fn open_chunk_reader_by_url_with_check(
        &self,
        chunk_url: &str,
        expect_chunk_id: Option<&ChunkId>,
        expect_chunk: &[u8],
        root_obj_id: &ObjId,
    ) -> CYFSHttpRespHeaders {
        let (mut reader, resp_headers) = self
            .open_chunk_reader_by_url(chunk_url, expect_chunk_id.map(|id| id.clone()), None)
            .await
            .expect("open chunk-reader failed");

        let content_len = resp_headers
            .obj_size
            .expect("content-length should exist in http-headers");
        assert_eq!(
            content_len,
            expect_chunk.len() as u64,
            "content-length in http-header should equal with chunk.len"
        );
        if let Some(chunk_id) = expect_chunk_id {
            assert_eq!(
                resp_headers.obj_id,
                Some(chunk_id.to_obj_id()),
                "obj-id in http-header should equal with chunk-id"
            );
        }
        assert_eq!(
            resp_headers.root_obj_id,
            Some(root_obj_id.clone()),
            "root-obj-id in http-header should equal with file-id"
        );

        let mut buffer = vec![0u8; 0];
        let len = reader
            .read_to_end(&mut buffer)
            .await
            .expect("read chunk failed");
        assert_eq!(
            len as u64, content_len,
            "length of data in http-body should equal with content-length"
        );
        assert_eq!(
            len,
            buffer.len(),
            "length of read data should equal with content-length"
        );
        assert_eq!(
            Sha256::digest(buffer.as_slice()),
            Sha256::digest(expect_chunk),
            "chunk content mismatch"
        );
        resp_headers
    }
    async fn open_chunk_reader_by_url_err(
        &self,
        chunk_url: &str,
        chunk_id: Option<&ChunkId>,
    ) -> NdnError {
        self.open_chunk_reader_by_url(chunk_url, chunk_id.map(|id| id.clone()), None)
            .await
            .map(|_| ())
            .expect_err("open chunk-reader should failed")
    }
    async fn open_chunk_reader_by_url_not_found(
        &self,
        chunk_url: &str,
        chunk_id: Option<&ChunkId>,
    ) {
        let err = self.open_chunk_reader_by_url_err(chunk_url, chunk_id).await;
        match err {
            NdnError::NotFound(_) => {
                info!("chunk not found as expected");
            }
            _ => {
                assert!(false, "Unexpected error type {:?}", err);
            }
        }
    }
    async fn open_chunk_reader_by_url_verify_error(
        &self,
        _chunk_url: &str,
        _chunk_id: Option<&ChunkId>,
    ) {
        // TODO: verify error
        // let err = self.open_chunk_reader_by_url_err(chunk_url, chunk_id).await;
        // match err {
        //     NdnError::VerifyError(_) => {
        //         info!("chunk verify failed as expected");
        //     }
        //     _ => {
        //         assert!(false, "Unexpected error type {:?}", err);
        //     }
        // }
    }
    async fn open_chunk_reader_by_url_range_with_check(
        &self,
        _chunk_url: &str,
        _expect_chunk_id: Option<&ChunkId>,
        _expect_chunk: &[u8],
        _root_obj_id: &ObjId,
    ) {
        // TODO: range
        // 1. get chunk range by range
        // let mut read_pos = 0;
        // let mut read_buffers = vec![];

        // loop {
        //     let read_len = {
        //         let mut rng = rand::rng();
        //         let max_len = expect_chunk.len() as u64 - read_pos;
        //         if max_len > 1 {
        //             rng.random_range(1u64..max_len)
        //         } else {
        //             1u64
        //         }
        //     };
        //     let end_pos = read_pos + read_len;

        //     let (mut reader, resp_headers) = self
        //         .open_chunk_reader_by_url(
        //             chunk_url,
        //             expect_chunk_id.map(|id| id.clone()),
        //             Some(read_pos..end_pos),
        //         )
        //         .await
        //         .expect("open chunk-reader failed");

        //     let content_len = resp_headers
        //         .obj_size
        //         .expect("content-length should exist in http-headers");
        //     assert_eq!(
        //         content_len, read_len,
        //         "content-length in http-header should equal with read_len"
        //     );

        //     if let Some(chunk_id) = expect_chunk_id {
        //         assert_eq!(
        //             resp_headers.obj_id,
        //             Some(chunk_id.to_obj_id()),
        //             "obj-id in http-header should equal with chunk-id"
        //         );
        //     }
        //     assert_eq!(
        //         resp_headers.root_obj_id,
        //         Some(root_obj_id.clone()),
        //         "root-obj-id in http-header should equal with file-id"
        //     );

        //     let mut buffer = vec![0u8; 0];
        //     let len = reader
        //         .read_to_end(&mut buffer)
        //         .await
        //         .expect("read chunk failed");
        //     assert_eq!(
        //         len as u64, read_len,
        //         "length of data in http-body should equal with content-length"
        //     );
        //     assert_eq!(
        //         len,
        //         buffer.len(),
        //         "length of read data should equal with content-length"
        //     );
        //     assert_eq!(
        //         buffer.as_slice(),
        //         &expect_chunk[read_pos as usize..end_pos as usize],
        //         "chunk range mismatch"
        //     );
        //     read_buffers.push(buffer);

        //     // todo: verify chunk with mtree

        //     read_pos += read_len;
        //     if read_pos >= expect_chunk.len() as u64 {
        //         break;
        //     }
        // }

        // let read_chunk = read_buffers.concat();
        // assert_eq!(
        //     Sha256::digest(read_chunk.as_slice()),
        //     Sha256::digest(expect_chunk),
        //     "chunk data mismatch"
        // );
    }

    async fn download_chunk_to_local_with_check(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        no_verify: bool,
        expect_chunk: &[u8],
    ) -> (ChunkId, u64) {
        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());
        let _ = std::fs::remove_file(download_path.as_path());
        let (download_chunk_id, download_chunk_len) = self
            .download_chunk_to_local(chunk_url, chunk_id.clone(), &download_path, Some(no_verify))
            .await
            .expect("download chunk to local failed");

        assert_eq!(
            &download_chunk_id, chunk_id,
            "download chunk-id should equal with chunk-id"
        );
        assert_eq!(
            download_chunk_len,
            expect_chunk.len() as u64,
            "download chunk-size should equal with chunk-data len"
        );

        let download_chunk =
            std::fs::read(download_path.as_path()).expect("chunk should exists in local");
        assert_eq!(
            Sha256::digest(download_chunk.as_slice()),
            Sha256::digest(expect_chunk),
            "should be same as chunk-content, len: {}",
            download_chunk_len,
        );

        std::fs::remove_file(download_path.as_path()).expect("remove download chunk file failed");
        (download_chunk_id, download_chunk_len)
    }

    async fn download_chunk_to_local_err(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        no_verify: bool,
    ) -> NdnError {
        let download_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join(TEST_DIR)
            .join(DOWNLOAD_DIR)
            .join(chunk_id.to_base32());

        let _ = std::fs::remove_file(download_path.as_path());

        let err = self
            .download_chunk_to_local(chunk_url, chunk_id.clone(), &download_path, Some(no_verify))
            .await
            .expect_err("download chunk to local should failed");
        // TODO: invalid file reserved
        // assert!(
        //     !std::fs::exists(download_path.as_path()).expect("unknown error for filesystem"),
        //     "chunk should removed for verify failed"
        // );
        err
    }

    async fn download_chunk_to_local_not_found(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        no_verify: bool,
    ) {
        let err = self
            .download_chunk_to_local_err(chunk_url, chunk_id, no_verify)
            .await;

        if let NdnError::NotFound(_) = err {
        } else {
            assert!(false, "unexpect error, should not found. {:?}", err)
        }
    }
    async fn download_chunk_to_local_verify_failed(
        &self,
        chunk_url: &str,
        chunk_id: &ChunkId,
        no_verify: bool,
    ) {
        let err = self
            .download_chunk_to_local_err(chunk_url, chunk_id, no_verify)
            .await;

        if let NdnError::VerifyError(_) = err {
        } else {
            assert!(false, "unexpect error, should verify failed. {:?}", err)
        }
    }
}
