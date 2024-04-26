use serde::{Deserialize, Serialize};
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;
use std::{path::PathBuf, str::FromStr};

use base58::ToBase58;
use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HTTP_HEADER_KEY: &'static str = "BACKUP_KEY";
const HTTP_HEADER_VERSION: &'static str = "BACKUP_VERSION";
const HTTP_HEADER_HASH: &'static str = "BACKUP_HASH";
const HTTP_HEADER_CHUNK_SEQ: &'static str = "BACKUP_CHUNK_SEQ";

#[derive(Deserialize, Serialize)]
struct CreateBackupReq {
    key: String,
    version: u32,
    meta: String,
    chunk_count: u32,
}

#[derive(Deserialize, Serialize)]
struct QueryVersionReq {
    key: String,
    offset: i32,
    limit: u32,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct QueryBackupVersionRespChunk {
    seq: u32,
    hash: String,
    size: u32,
}

// chunk_count > chunks.len(): 备份还没完成
// chunk_count < chunks.len(): 出现错误，不可用
#[derive(Deserialize, Serialize)]
pub struct QueryBackupVersionResp {
    key: String,
    version: u32,
    meta: String,
    chunk_count: u32,

    chunks: Vec<QueryBackupVersionRespChunk>,
}

#[derive(Deserialize, Serialize)]
struct QueryVersionInfoReq {
    key: String,
    version: u32,
}

#[derive(Deserialize, Serialize)]
struct DownloadBackupVersionReq {
    key: String,
    version: u32,
}

#[derive(Deserialize, Serialize)]
struct DownloadBackupChunkReq {
    key: String,
    version: u32,
    chunk_seq: u32,
}

pub struct Backup {
    url: String,
}

pub enum ListOffset {
    FromFirst(u32),
    FromLast(u32),
}

impl Backup {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }

    // TODO: 可能还需要一个公钥作为身份标识，否则可能被恶意应用篡改
    pub async fn post_backup(
        &self,
        key: &str,
        version: u32,
        meta: &impl ToString,
        chunk_file_list: &[&std::path::Path],
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 1. put meta
        // 2. upload chunk files
        let url = format!("{}/{}", self.url.as_str(), "new_backup");
        let client = reqwest::Client::new();
        match client
            .post(url.as_str())
            .body(
                serde_json::to_string(&CreateBackupReq {
                    key: key.to_string(),
                    version,
                    meta: meta.to_string(),
                    chunk_count: chunk_file_list.len() as u32,
                })
                .unwrap(),
            )
            .header(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            )
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status() != reqwest::StatusCode::OK {
                    log::error!(
                        "send backup meta from http({}) failed: {}.",
                        url,
                        resp.status()
                    );
                    return Err(Box::new(resp.error_for_status().unwrap_err()));
                } else {
                    log::trace!("send backup meta from http({}) succeeded", url);
                }
            }
            Err(err) => {
                log::error!("send backup meta from http({}) failed: {}.", self.url, err);
                return Err(Box::new(err));
            }
        }

        let rets = futures::future::join_all(chunk_file_list.iter().enumerate().map(
            |(chunk_seq, chunk_path)| {
                self.upload_chunk(key, version, chunk_seq as u32, *chunk_path)
            },
        ))
        .await;

        rets.into_iter()
            .find(|r| r.is_err())
            .map_or(Ok(()), |err| err)
    }

    pub async fn query_versions<Meta: FromStr>(
        &self,
        key: &str,
        offset: ListOffset,
        limit: u32,
    ) -> Result<Vec<QueryBackupVersionResp>, Box<dyn std::error::Error>> {
        let url = format!("{}/{}", self.url.as_str(), "query_versions");

        let client = reqwest::Client::new();
        match client
            .post(url.as_str())
            .body(
                serde_json::to_string(&QueryVersionReq {
                    key: key.to_string(),
                    offset: match offset {
                        ListOffset::FromFirst(n) => n as i32,
                        ListOffset::FromLast(n) => -(n as i32),
                    },
                    limit,
                })
                .unwrap(),
            )
            .header(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            )
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status() != reqwest::StatusCode::OK {
                    log::error!(
                        "send backup query_versions request from http({}) failed: {}.",
                        url,
                        resp.status()
                    );
                    return Err(Box::new(resp.error_for_status().unwrap_err()));
                } else {
                    log::trace!(
                        "send backup query_versions request from http({}) succeeded",
                        url
                    );
                    match resp.json().await {
                        Ok(v) => Ok(v),
                        Err(e) => Err(Box::new(e)),
                    }
                }
            }
            Err(err) => {
                log::error!(
                    "send query_versions request from http({}) failed: {}.",
                    self.url,
                    err
                );
                return Err(Box::new(err));
            }
        }
    }

    pub async fn download_backup(
        &self,
        key: &str,
        version: u32,
        dir_path: &Path,
    ) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
        // 1. get chunk file list
        // 2. download chunk files to the dir_path

        let url = format!("{}/{}", self.url.as_str(), "version_info");

        let client = reqwest::Client::new();
        let version_info = match client
            .post(url.as_str())
            .body(
                serde_json::to_string(&QueryVersionInfoReq {
                    key: key.to_string(),
                    version,
                })
                .unwrap(),
            )
            .header(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            )
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status() != reqwest::StatusCode::OK {
                    log::error!(
                        "send backup version_info request from http({}) failed: {}.",
                        url,
                        resp.status()
                    );
                    return Err(Box::new(resp.error_for_status().unwrap_err()));
                } else {
                    log::trace!(
                        "send backup version_info request from http({}) succeeded",
                        url
                    );
                    resp.json::<QueryBackupVersionResp>().await?
                }
            }
            Err(err) => {
                log::error!(
                    "send version_info request from http({}) failed: {}.",
                    self.url,
                    err
                );
                return Err(Box::new(err));
            }
        };

        let mut rets = futures::future::join_all(version_info.chunks.iter().map(|chunk| {
            self.download_chunk(
                key,
                version,
                chunk.seq,
                chunk.size,
                chunk.hash.as_str(),
                dir_path,
            )
        }))
        .await;

        if let Some(err) = rets.iter_mut().find(|r| r.is_err()) {
            let mut v = Ok(PathBuf::new());
            std::mem::swap(err, &mut v);
            return Err(v.unwrap_err());
        }

        Ok(rets
            .into_iter()
            .map(|chunk_path| chunk_path.unwrap())
            .collect())
    }

    async fn upload_chunk(
        &self,
        key: &str,
        version: u32,
        chunk_seq: u32,
        chunk_path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut file = tokio::fs::File::open(chunk_path).await?;
        let mut buf = vec![];
        file.read_to_end(&mut buf).await?;

        let mut hasher = sha2::Sha256::new();
        hasher.update(buf.as_slice());
        let hash = hasher.finalize();
        let hash = hash.as_slice().to_base58();

        let url = format!("{}/{}", self.url.as_str(), "new_chunk");

        loop {
            let client = reqwest::Client::new();
            match client
                .post(url.as_str())
                .body(buf.clone())
                .header(HTTP_HEADER_KEY, key)
                .header(HTTP_HEADER_VERSION, version)
                .header(HTTP_HEADER_HASH, hash.as_str())
                .header(HTTP_HEADER_CHUNK_SEQ, chunk_seq)
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status() != reqwest::StatusCode::OK {
                        log::error!(
                            "send backup chunk({}-{}-{}:{}) from http({}) failed: {}.",
                            key,
                            version,
                            chunk_seq,
                            chunk_path.display(),
                            url,
                            resp.status()
                        );
                    } else {
                        log::trace!(
                            "send backup chunk({}-{}-{}:{}) from http({}) succeeded",
                            key,
                            version,
                            chunk_seq,
                            chunk_path.display(),
                            url
                        );

                        return Ok(());
                    }
                }
                Err(err) => {
                    log::error!(
                        "send backup chunk({}-{}-{}:{}) from http({}) failed: {}.",
                        key,
                        version,
                        chunk_seq,
                        chunk_path.display(),
                        url,
                        err
                    );
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn download_chunk(
        &self,
        key: &str,
        version: u32,
        chunk_seq: u32,
        chunk_size: u32,
        hash: &str,
        dir_path: &std::path::Path,
    ) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        let filename = Self::download_chunk_file_name(key, version, chunk_seq);
        let chunk_path = dir_path.join(filename.as_str());
        if chunk_path.exists() {
            let mut file = tokio::fs::File::open(&chunk_path).await?;
            if file.metadata().await?.file_size() as u32 == chunk_size {
                let mut buf = vec![];
                if file.read_to_end(&mut buf).await.is_ok() {
                    let mut hasher = sha2::Sha256::new();
                    hasher.update(buf.as_slice());
                    let file_hash = hasher.finalize();
                    let file_hash = file_hash.as_slice().to_base58();

                    if (hash == file_hash.as_str()) {
                        return Ok(chunk_path);
                    }
                }
            }

            tokio::fs::remove_file(&chunk_path);
        }

        let url = format!("{}/{}", self.url.as_str(), "chunk");

        loop {
            let client = reqwest::Client::new();
            match client
                .post(url.as_str())
                .body(
                    serde_json::to_string(&DownloadBackupChunkReq {
                        key: key.to_string(),
                        version,
                        chunk_seq,
                    })
                    .unwrap(),
                )
                .header(
                    reqwest::header::CONTENT_TYPE,
                    reqwest::header::HeaderValue::from_static("application/json"),
                )
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status() != reqwest::StatusCode::OK {
                        log::error!(
                            "send backup download chunk({}-{}-{}:{}) from http({}) failed: {}.",
                            key,
                            version,
                            chunk_seq,
                            chunk_path.display(),
                            url,
                            resp.status()
                        );
                    } else {
                        log::trace!(
                            "send backup download chunk({}-{}-{}:{}) from http({}) succeeded",
                            key,
                            version,
                            chunk_seq,
                            chunk_path.display(),
                            url
                        );

                        let resp_hash = resp.headers().get(HTTP_HEADER_HASH).cloned();
                        match resp.bytes().await {
                            Ok(buf) => {
                                if buf.len() as u32 == chunk_size {
                                    let mut hasher = sha2::Sha256::new();
                                    hasher.update(buf.as_ref());
                                    let file_hash = hasher.finalize();
                                    let file_hash = file_hash.as_slice().to_base58();
                                    if file_hash == hash
                                        && (resp_hash.is_none() || resp_hash.unwrap() == hash)
                                    {
                                        let mut file = tokio::fs::File::create(&chunk_path).await?;
                                        file.write_all(buf.as_ref()).await?;
                                        return Ok(chunk_path);
                                    }
                                }
                            }
                            Err(err) => {
                                log::error!(
                                    "backup download chunk({}-{}-{}:{}) from http({}) failed: {}.",
                                    key,
                                    version,
                                    chunk_seq,
                                    chunk_path.display(),
                                    url,
                                    err
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    log::error!(
                        "send backup download chunk({}-{}-{}:{}) from http({}) failed: {}.",
                        key,
                        version,
                        chunk_seq,
                        chunk_path.display(),
                        url,
                        err
                    );
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    fn download_chunk_file_name(key: &str, version: u32, chunk_seq: u32) -> String {
        format!("{}-{}-{}.bak", key, version, chunk_seq)
    }
}
