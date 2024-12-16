use crate::def::*;
use crate::error::*;
use crate::verifier::Verifier;
use core::error;
use futures_util::StreamExt;
use hex;
use log::*;
use ndn_lib::*;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

pub async fn chunk_to_local_file(
    chunk_id: &str,
    chunk_mgr_id: &str,
    local_file: &PathBuf,
) -> RepoResult<()> {
    unimplemented!("chunk_to_local_file")
}

const REPO_CHUNK_MGR_ID: &str = "repo_chunk_mgr";

#[derive(Debug, Clone)]
pub struct Downloader {}

impl Downloader {
    pub async fn pull_remote_chunk(
        url: &str,
        author: &str,
        sign: &str,
        chunk_id: &str,
    ) -> RepoResult<()> {
        //先验证
        if let Err(e) = Verifier::verify(author, chunk_id, sign).await {
            return Err(RepoError::VerifyError(format!(
                "Verify failed, author: {}, chunk_id: {}, sign: {}, err: {}",
                author,
                chunk_id,
                sign,
                e.to_string()
            )));
        }

        // let ndn_client = NdnClient::new(url.to_string(), None, Some(REPO_CHUNK_MGR_ID.to_string()));
        // let chunk_id = ChunkId::new(chunk_id)
        //     .map_err(|e| RepoError::ParseError(chunk_id.to_string(), e.to_string()))?;
        // match ndn_client
        //     .pull_chunk(chunk_id.clone(), Some(REPO_CHUNK_MGR_ID))
        //     .await
        // {
        //     Ok(_) => Ok(()),
        //     Err(e) => {
        //         if let NdnError::AlreadyExists(_) = e {
        //             info!("chunk {} already exists", chunk_id.to_string());
        //             Ok(())
        //         } else {
        //             error!(
        //                 "pull remote chunk {} failed:{}",
        //                 chunk_id.to_string(),
        //                 e.to_string()
        //             );
        //             Err(RepoError::DownloadError(
        //                 chunk_id.to_string(),
        //                 e.to_string(),
        //             ))
        //         }
        //     }
        // }
        unimplemented!()
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
