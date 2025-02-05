use crate::crypto_utils::*;
use crate::def::*;
use buckyos_kit::get_buckyos_service_data_dir;
use core::error;
use futures_util::StreamExt;
use hex;
use log::*;
use ndn_lib::*;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::io::SeekFrom;
use std::path::PathBuf;
use std::vec;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn chunk_to_local_file(
    chunk_id: &str,
    chunk_mgr_id: &str,
    local_file: &PathBuf,
) -> RepoResult<()> {
    unimplemented!("chunk_to_local_file")
}

// async fn test_pull_chunk() {
//     let client = NdnClient::new("test_url".to_string(), None, None);
//     let chunk_id = ChunkId::new("example_chunk_id").unwrap();

//     info!("1");
//     let rt = tokio::runtime::Runtime::new().unwrap();
//     rt.spawn_blocking(move || {
//         let local_rt = tokio::runtime::Runtime::new().unwrap();
//         local_rt.block_on(async {
//             let _ = client.pull_chunk(chunk_id, None).await;
//         });
//     });
//     info!("3");
// }

#[derive(Debug, Clone)]
pub struct Downloader {}

impl Downloader {
    pub async fn init_repo_chunk_mgr() -> RepoResult<()> {
        let repo_dir = get_buckyos_service_data_dir(SERVICE_NAME);
        let repo_chunk_dir = repo_dir.join("chunks");
        if !repo_chunk_dir.exists() {
            std::fs::create_dir_all(&repo_chunk_dir)?;
        }

        let repo_chunk_mgr_config = NamedDataMgrConfig {
            local_stores: vec![repo_chunk_dir.to_string_lossy().to_string()],
            local_cache: None,
            mmap_cache_dir: None,
        };

        let name_mgr = NamedDataMgr::from_config(
            Some(REPO_CHUNK_MGR_ID.to_string()),
            repo_chunk_dir,
            repo_chunk_mgr_config,
        )
        .await
        .map_err(|e| RepoError::NdnError(e.to_string()))?;

        NamedDataMgr::set_mgr_by_id(Some(REPO_CHUNK_MGR_ID), name_mgr)
            .await
            .map_err(|e| {
                RepoError::NdnError(format!(
                    "Failed to set repo chunk mgr by id, err:{}",
                    e.to_string()
                ))
            })?;

        info!("init repo chunk mgr success");

        Ok(())
    }

    pub async fn pull_remote_chunk(
        url: &str,
        author: &str,
        sign: &str,
        chunk_id: &str,
    ) -> RepoResult<()> {
        //先验证
        if let Err(e) = verify(author, chunk_id, sign).await {
            return Err(RepoError::VerifyError(format!(
                "Verify failed, author: {}, chunk_id: {}, sign: {}, err: {}",
                author,
                chunk_id,
                sign,
                e.to_string()
            )));
        }

        info!(
            "will pull remote chunk, url:{}, author:{}, chunk_id:{}",
            url, author, chunk_id
        );

        let ndn_client = NdnClient::new(url.to_string(), None, None);
        let chunk_id = ChunkId::new(chunk_id)
            .map_err(|e| RepoError::ParseError(chunk_id.to_string(), e.to_string()))?;
        match ndn_client.pull_chunk(chunk_id.clone(), None).await {
            Ok(_) => Ok(()),
            Err(e) => {
                if let NdnError::AlreadyExists(_) = e {
                    info!("chunk {} already exists", chunk_id.to_string());
                    Ok(())
                } else {
                    error!(
                        "pull remote chunk {} failed:{}",
                        chunk_id.to_string(),
                        e.to_string()
                    );
                    Err(RepoError::DownloadError(
                        chunk_id.to_string(),
                        e.to_string(),
                    ))
                }
            }
        }
    }

    pub async fn chunk_to_local_file(
        chunk_id: &str,
        chunk_mgr_id: Option<&str>,
        local_file: &PathBuf,
    ) -> RepoResult<()> {
        let named_mgr =
            NamedDataMgr::get_named_data_mgr_by_id(None)
                .await
                .ok_or(RepoError::NotFound(format!(
                    "chunk mgr {:?} not found",
                    chunk_mgr_id
                )))?;

        let chunk_id = ChunkId::new(chunk_id)
            .map_err(|e| RepoError::ParseError(chunk_id.to_string(), e.to_string()))?;

        let named_mgr = named_mgr.lock().await;

        let (mut reader, size) = named_mgr
            .open_chunk_reader(&chunk_id, SeekFrom::Start(0), true)
            .await
            .unwrap();

        let mut file = File::create(local_file).await?;

        let mut buf = vec![0u8; 1024];
        let mut read_size = 0;
        while read_size < size {
            let read_len = reader
                .read(&mut buf)
                .await
                .map_err(|e| RepoError::NdnError(format!("Read chunk error:{:?}", e)))?;
            if read_len == 0 {
                break;
            }
            read_size += read_len as u64;
            file.write_all(&buf[..read_len]).await?;
        }
        file.flush().await?;

        Ok(())
    }

    pub async fn download_file(url: &str, local_path: &PathBuf, sha256: &str) -> RepoResult<()> {
        let client = Client::new();

        let response = client.get(url).send().await.map_err(|e| {
            error!("Failed to send request: {}", e);
            RepoError::DownloadError(url.to_string(), format!("Failed to send request: {}", e))
        })?;

        if !response.status().is_success() {
            error!("HTTP error: {}", response.status());
            return Err(RepoError::DownloadError(
                url.to_string(),
                format!("HTTP error: {}", response.status()),
            ));
        }

        let mut file = File::create(local_path).await?;
        let mut hasher = Sha256::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    file.write_all(&bytes).await?;
                    hasher.update(&bytes);
                }
                Err(e) => {
                    error!("Stream error: {}", e);
                    return Err(RepoError::DownloadError(
                        url.to_string(),
                        format!("Stream error: {}", e),
                    ));
                }
            }
        }

        let hash_result = hasher.finalize().to_vec();
        let calculated_sha256 = hex::encode(hash_result);

        if calculated_sha256 != sha256 {
            let err_msg = format!(
                "Sha256 mismatch: expected {}, got {}",
                sha256, calculated_sha256
            );
            error!("{}", err_msg);
            return Err(RepoError::DownloadError(url.to_string(), err_msg));
        }

        Ok(())
    }
}
