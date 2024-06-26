use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use base58::ToBase58;
use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HTTP_HEADER_ZONEID: &'static str = "zone-id";
const HTTP_HEADER_KEY: &'static str = "backup-key";
const HTTP_HEADER_VERSION: &'static str = "backup-version";
const HTTP_HEADER_HASH: &'static str = "backup-hash";
const HTTP_HEADER_CHUNK_SEQ: &'static str = "backup-chunk-seq";
const HTTP_HEADER_CHUNK_RELATIVE_PATH: &'static str = "backup-chunk-relative-path";

/**
 * TODO:
 * 1. 为备份实现断点续传
 * 2. 增量备份
*/

#[derive(Deserialize, Serialize)]
struct CreateBackupReq {
    zone_id: String,
    key: String,
    version: u32,
    prev_version: Option<u32>,
}

#[derive(Deserialize, Serialize)]
struct CommitBackupReq {
    zone_id: String,
    key: String,
    version: u32,
    meta: String,
    chunk_count: u32,
}

#[derive(Deserialize, Serialize)]
struct QueryVersionReq {
    zone_id: String,
    key: String,
    offset: i32,
    limit: u32,
    is_restorable_only: bool,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct QueryBackupVersionRespChunk {
    seq: u32,
    hash: String,
    size: u32,
    relative_path: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct QueryBackupVersionResp {
    key: String,
    pub version: u32,
    prev_version: Option<u32>,
    meta: String,
    is_restorable: bool,
    chunk_count: u32,
}

#[derive(Deserialize, Serialize)]
struct QueryVersionInfoReq {
    zone_id: String,
    key: String,
    version: u32,
}

#[derive(Deserialize, Serialize)]
struct QueryChunkInfoReq {
    zone_id: String,
    key: String,
    version: u32,
    chunk_seq: u32,
}

#[derive(Deserialize, Serialize)]
struct DownloadBackupChunkReq {
    zone_id: String,
    key: String,
    version: u32,
    chunk_seq: u32,
}

#[derive(Clone)]
pub struct Backup {
    url: String,
    zone_id: String,
    storage_dir: PathBuf,
}

pub enum ListOffset {
    FromFirst(u32),
    FromLast(u32),
}

impl Backup {
    pub fn new(url: String, zone_id: String, storage_dir: &str) -> Self {
        Self {
            url,
            zone_id,
            storage_dir: PathBuf::from(storage_dir),
        }
    }

    // TODO: 可能还需要一个公钥作为身份标识，否则可能被恶意应用篡改
    pub async fn post_backup(
        &self,
        key: &str,
        version: u32,
        prev_version: Option<u32>,
        meta: &impl ToString,
        chunk_file_dir: &std::path::Path,
        chunk_relative_path_list: &[&std::path::Path],
    ) -> Result<(), Box<dyn std::error::Error>> {
        if version == 0 {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "version should be greater than 0",
            )));
        }

        // 1. begin
        // 2. upload chunk files
        // 3. put meta
        let url = format!("{}/{}", self.url.as_str(), "new_backup");
        let client = reqwest::Client::new();
        match client
            .post(url.as_str())
            .json(&CreateBackupReq {
                zone_id: self.zone_id.clone(),
                key: key.to_string(),
                version,
                prev_version,
            })
            // .header(
            //     reqwest::header::CONTENT_TYPE,
            //     reqwest::header::HeaderValue::from_static("application/json"),
            // )
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

        let rets = futures::future::join_all(chunk_relative_path_list.iter().enumerate().map(
            |(chunk_seq, relative_path)| {
                self.upload_chunk(
                    key,
                    version,
                    chunk_seq as u32,
                    chunk_file_dir,
                    *relative_path,
                )
            },
        ))
        .await;

        rets.into_iter()
            .find(|r| r.is_err())
            .map_or(Ok(()), |err| err)?;

        let url = format!("{}/{}", self.url.as_str(), "commit_backup");
        let client = reqwest::Client::new();
        match client
            .post(url.as_str())
            .json(&CommitBackupReq {
                zone_id: self.zone_id.clone(),
                key: key.to_string(),
                version,
                meta: meta.to_string(),
                chunk_count: chunk_relative_path_list.len() as u32,
            })
            // .header(
            //     reqwest::header::CONTENT_TYPE,
            //     reqwest::header::HeaderValue::from_static("application/json"),
            // )
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

                    Err(Box::new(resp.error_for_status().unwrap_err()))
                } else {
                    log::trace!("send commit backup meta from http({}) succeeded", url);
                    Ok(())
                }
            }
            Err(err) => {
                log::error!("send backup meta from http({}) failed: {}.", self.url, err);
                Err(Box::new(err))
            }
        }
    }

    pub async fn query_versions(
        &self,
        key: &str,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<QueryBackupVersionResp>, Box<dyn std::error::Error>> {
        let url = format!("{}/{}", self.url.as_str(), "query_versions");

        let client = reqwest::Client::new();
        match client
            .get(url.as_str())
            .body(
                serde_json::to_string(&QueryVersionReq {
                    zone_id: self.zone_id.clone(),
                    key: key.to_string(),
                    offset: match offset {
                        ListOffset::FromFirst(n) => n as i32,
                        ListOffset::FromLast(n) => -((n + 1) as i32),
                    },
                    limit,
                    is_restorable_only,
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

    pub async fn query_last_versions(
        &self,
        key: &str,
        is_restorable_only: bool,
    ) -> Result<QueryBackupVersionResp, Box<dyn std::error::Error>> {
        match self
            .query_versions(key, ListOffset::FromLast(0), 1, is_restorable_only)
            .await
        {
            Ok(mut v) => {
                if v.len() > 0 {
                    Ok(v.remove(0))
                } else {
                    Err(Box::new(std::io::Error::new(
                        io::ErrorKind::NotFound,
                        "no version found",
                    )))
                }
            }
            Err(err) => Err(err),
        }
    }

    pub async fn download_backup(
        &self,
        key: &str,
        version: u32,
        dir_path: Option<&Path>, // = ${storage_dir}
    ) -> Result<(std::path::PathBuf, Vec<std::path::PathBuf>), Box<dyn std::error::Error>> {
        // 1. get chunk file list
        // 2. download chunk files to the dir_path

        // TODO: Write a record into the database as a task is executing,
        // And continue the task when the system recovered.

        let url = format!("{}/{}", self.url.as_str(), "version_info");

        let client = reqwest::Client::new();
        let version_info = match client
            .get(url.as_str())
            .json(&QueryVersionInfoReq {
                key: key.to_string(),
                version,
                zone_id: self.zone_id.clone(),
            })
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

        let dir_path = dir_path.unwrap_or(&self.storage_dir);
        let mut rets = futures::future::join_all(
            (0..version_info.chunk_count)
                .map(|chunk_seq| self.download_chunk(key, version, chunk_seq, dir_path)),
        )
        .await;

        if let Some(err) = rets.iter_mut().find(|r| r.is_err()) {
            let mut v = Ok(PathBuf::new());
            std::mem::swap(err, &mut v);
            return Err(v.unwrap_err());
        }

        Ok((
            dir_path.to_path_buf(),
            rets.into_iter()
                .map(|chunk_path| chunk_path.unwrap())
                .collect(),
        ))
    }

    async fn upload_chunk(
        &self,
        key: &str,
        version: u32,
        chunk_seq: u32,
        chunk_file_dir: &std::path::Path,
        chunk_relative_path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chunk_full_path = chunk_file_dir.join(chunk_relative_path);
        let mut file = tokio::fs::File::open(chunk_full_path).await?;
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
                .header(HTTP_HEADER_ZONEID, self.zone_id.as_str())
                .header(HTTP_HEADER_KEY, key)
                .header(HTTP_HEADER_VERSION, version)
                .header(HTTP_HEADER_HASH, hash.as_str())
                .header(HTTP_HEADER_CHUNK_SEQ, chunk_seq)
                .header(
                    HTTP_HEADER_CHUNK_RELATIVE_PATH,
                    chunk_relative_path.display().to_string(),
                )
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
                            chunk_relative_path.display(),
                            url,
                            resp.status()
                        );
                    } else {
                        log::trace!(
                            "send backup chunk({}-{}-{}:{}) from http({}) succeeded",
                            key,
                            version,
                            chunk_seq,
                            chunk_relative_path.display(),
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
                        chunk_relative_path.display(),
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
        dir_path: &std::path::Path,
    ) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        let url = format!("{}/{}", self.url.as_str(), "chunk_info");

        let chunk_info = loop {
            let client = reqwest::Client::new();
            let _ = match client
                .get(url.as_str())
                .json(&QueryChunkInfoReq {
                    key: key.to_string(),
                    version,
                    zone_id: self.zone_id.clone(),
                    chunk_seq,
                })
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
                    } else {
                        log::trace!(
                            "send backup version_info request from http({}) succeeded",
                            url
                        );
                        if let Ok(chunk_info) = resp.json::<QueryBackupVersionRespChunk>().await {
                            break chunk_info;
                        }
                    }
                }
                Err(err) => {
                    log::error!(
                        "send version_info request from http({}) failed: {}.",
                        self.url,
                        err
                    );
                }
            };
        };

        // let filename = Self::download_chunk_file_name(key, version, chunk_seq);
        let chunk_path = dir_path.join(chunk_info.relative_path.as_str());
        if chunk_path.exists() {
            if std::fs::metadata(&chunk_path)?.len() as u32 == chunk_info.size {
                let mut file = tokio::fs::File::open(&chunk_path).await?;
                let mut buf = vec![];
                if file.read_to_end(&mut buf).await.is_ok() {
                    let mut hasher = sha2::Sha256::new();
                    hasher.update(buf.as_slice());
                    let file_hash = hasher.finalize();
                    let file_hash = file_hash.as_slice().to_base58();

                    if chunk_info.hash == file_hash {
                        return Ok(
                            std::path::PathBuf::from_str(chunk_info.relative_path.as_str())
                                .expect("invalid path"),
                        );
                    }
                }
            }

            let _ = tokio::fs::remove_file(&chunk_path).await;
        }

        let url = format!("{}/{}", self.url.as_str(), "chunk");

        loop {
            let client = reqwest::Client::new();
            match client
                .get(url.as_str())
                .body(
                    serde_json::to_string(&DownloadBackupChunkReq {
                        zone_id: self.zone_id.clone(),
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
                                if buf.len() as u32 == chunk_info.size {
                                    let mut hasher = sha2::Sha256::new();
                                    hasher.update(buf.as_ref());
                                    let file_hash = hasher.finalize();
                                    let file_hash = file_hash.as_slice().to_base58();
                                    if file_hash == chunk_info.hash
                                        && (resp_hash.is_none()
                                            || resp_hash.unwrap() == chunk_info.hash)
                                    {
                                        let mut file = tokio::fs::File::create(&chunk_path).await?;
                                        file.write_all(buf.as_ref()).await?;
                                        return Ok(std::path::PathBuf::from_str(
                                            chunk_info.relative_path.as_str(),
                                        )
                                        .expect("invalid path"));
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

    // fn download_chunk_file_name(key: &str, version: u32, chunk_seq: u32) -> String {
    //     format!("{}-{}-{}.bak", key, version, chunk_seq)
    // }
}

#[cfg(test)]
mod tests {
    use crate::backup::{Backup, ListOffset, QueryBackupVersionRespChunk};
    use base58::ToBase58;
    use sha2::Digest;
    use std::{collections::HashMap, str::FromStr};

    #[tokio::test]
    async fn test() {
        // 1. 准备10个文件
        // let origin_path = std::path::Path::new("c:/origin");
        // let restore_path = "c:/restore";
        let origin_path = std::path::Path::new("~/origin");
        let restore_path = "~/restore";
        let key_count = 2;
        let version_count = 3;

        tokio::fs::create_dir_all(origin_path)
            .await
            .expect("create origin directory failed");
        tokio::fs::create_dir_all(restore_path)
            .await
            .expect("create restore directory failed");

        let mut origin_chunk_infos: HashMap<
            String,
            HashMap<u32, (String, Vec<(String, QueryBackupVersionRespChunk)>)>,
        > = HashMap::new();

        for k in 0..key_count {
            let key = format!("key-{}", k);
            origin_chunk_infos.insert(key.clone(), HashMap::new());
            let mut versions = origin_chunk_infos.get_mut(&key).unwrap();

            for version in 1..(version_count + 1) {
                versions.insert(version, (format!("{}-{}", key, version), vec![]));
                let mut chunks = &mut versions.get_mut(&version).unwrap().1;
                for i in 0..version {
                    let file_name = format!("{}-{}-{}", key, version, i);
                    let file_path = origin_path.join(file_name.as_str());

                    let mut content = key.as_bytes().to_vec();
                    for _ in 0..(version + 1) * (i + 1) {
                        content.push(version as u8);
                        content.push(i as u8);
                    }

                    let mut hasher = sha2::Sha256::new();
                    hasher.update(content.as_slice());
                    let hash = hasher.finalize();
                    let hash = hash.as_slice().to_base58();

                    chunks.push((
                        file_name.clone(),
                        QueryBackupVersionRespChunk {
                            seq: i,
                            hash,
                            size: content.len() as u32,
                            relative_path: file_name,
                        },
                    ));

                    tokio::fs::write(file_path, content)
                        .await
                        .expect("create origin file failed");
                }
            }
        }
        let backup = Backup::new(
            "http://47.106.164.184".to_string(),
            // "http://192.168.100.120:8000".to_string(),
            "test-case-zone".to_string(),
            restore_path,
        );
        let mut tasks = vec![];

        for (key, versions) in origin_chunk_infos.iter() {
            for (version, version_info) in versions.iter() {
                let backup = backup.clone();
                tasks.push(async move {
                    let chunk_relative_paths = version_info
                        .1
                        .iter()
                        .map(|(path, _)| std::path::PathBuf::from(path))
                        .collect::<Vec<_>>();
                    let chunk_path_refs = chunk_relative_paths
                        .iter()
                        .map(|p| p.as_ref())
                        .collect::<Vec<&std::path::Path>>();
                    backup
                        .post_backup(
                            key,
                            *version,
                            None,
                            &version_info.0,
                            &origin_path,
                            chunk_path_refs.as_slice(),
                        )
                        .await
                })
            }
        }

        let rets = futures::future::join_all(tasks).await;
        let err = rets.iter().find(|r| r.is_err());
        assert!(
            err.is_none(),
            "backup failed {}",
            err.unwrap().as_ref().unwrap_err()
        );

        let mut tasks = vec![];
        for (key, versions) in origin_chunk_infos.iter() {
            for (version, version_info) in versions.iter() {
                let backup = backup.clone();
                tasks.push(async move {
                    backup
                        .query_versions(key.as_str(), ListOffset::FromLast(10), 20, true)
                        .await
                })
            }
        }

        let versions = futures::future::join_all(tasks).await;
        let err = versions.iter().find(|r| r.is_err());
        assert!(
            err.is_none(),
            "query versions failed {}",
            err.unwrap().as_ref().unwrap_err().to_string()
        );

        for versions in versions {
            let versions = versions.unwrap();
            for version in versions {
                let versions = origin_chunk_infos
                    .get(version.key.as_str())
                    .expect("key missed");
                let (meta, chunks) = versions.get(&version.version).expect("version missed");
                assert_eq!(
                    meta, &version.meta,
                    "meta mismatch, key: {}, version: {}, expect: {}, got: {}",
                    version.key, version.version, meta, version.meta
                );

                assert_eq!(
                    version.chunk_count,
                    chunks.len() as u32,
                    "chunk count mismatch, key: {}, version: {}, expect: {}, got: {}",
                    version.key,
                    version.version,
                    chunks.len(),
                    version.chunk_count
                );

                // for i in 0..version.chunk_count as usize {
                //     let (chunk_path, expect_chunk) = chunks.get(i).expect("chunk missed");
                //     let queried_chunk = version.chunks.get(i).expect("chunk from server missed");
                //     assert_eq!(
                //         expect_chunk.seq, queried_chunk.seq,
                //         "seq mismatch, key: {}, version: {}, expect: {}, got: {}",
                //         version.key, version.version, expect_chunk.seq, queried_chunk.seq
                //     );
                //     assert_eq!(
                //         expect_chunk.hash,
                //         queried_chunk.hash,
                //         "hash mismatch, key: {}, version: {}, seq: {}, expect: {}, got: {}",
                //         version.key,
                //         version.version,
                //         queried_chunk.seq,
                //         expect_chunk.hash,
                //         queried_chunk.hash
                //     );
                //     assert_eq!(
                //         expect_chunk.size,
                //         queried_chunk.size,
                //         "hash mismatch, key: {}, version: {}, seq: {}, expect: {}, got: {}",
                //         version.key,
                //         version.version,
                //         queried_chunk.seq,
                //         expect_chunk.size,
                //         queried_chunk.size
                //     );
                // }
            }
        }

        let mut tasks = vec![];
        for (key, _) in origin_chunk_infos.iter() {
            let backup = backup.clone();
            tasks.push(async move { backup.query_last_versions(key.as_str(), true).await })
        }

        let versions = futures::future::join_all(tasks).await;
        let err = versions.iter().find(|r| r.is_err());
        assert!(
            err.is_none(),
            "query versions failed {}",
            err.unwrap().as_ref().unwrap_err().to_string()
        );

        for version in versions.iter() {
            let version = version.as_ref().unwrap();
            let max_version = *origin_chunk_infos
                .get(&version.key)
                .as_ref()
                .unwrap()
                .keys()
                .max()
                .unwrap();
            assert_eq!(
                version.version, max_version,
                "last version error: expected: {}, got: {}",
                max_version, version.version
            )
        }

        let mut tasks = vec![];
        let _restore_path = std::path::PathBuf::from_str(restore_path).unwrap();
        for (key, versions) in origin_chunk_infos.iter() {
            for (version, version_info) in versions.iter() {
                let backup = backup.clone();
                let _restore_path = _restore_path.clone();
                tasks.push(async move {
                    backup
                        .download_backup(key, *version, Some(_restore_path.as_path()))
                        .await
                        .map(|chunk_paths| (key.clone(), *version, chunk_paths))
                })
            }
        }

        let download_chunk_paths = futures::future::join_all(tasks).await;
        let err = download_chunk_paths.iter().find(|r| r.is_err());
        assert!(
            err.is_none(),
            "download chunk failed {}",
            err.unwrap().as_ref().unwrap_err()
        );

        for download_chunks in download_chunk_paths {
            let (key, version, (dir_path, download_chunk_relative_paths)) =
                download_chunks.unwrap();
            for (chunk_seq, relative_path) in download_chunk_relative_paths.iter().enumerate() {
                let versions = origin_chunk_infos
                    .get(key.as_str())
                    .expect("download key missed");
                let (meta, chunks) = versions.get(&version).expect("download version missed");
                let chunk_path = dir_path.join(relative_path);
                let download_chunk = tokio::fs::read(chunk_path.as_path()).await.expect(
                    format!("read download chunk to path({:?}) failed", chunk_path).as_str(),
                );

                let mut hasher = sha2::Sha256::new();
                hasher.update(download_chunk.as_slice());
                let hash = hasher.finalize();
                let hash = hash.as_slice().to_base58();

                let expect_hash = chunks.get(chunk_seq).expect("download chunk missed");

                assert_eq!(
                    &hash, &expect_hash.1.hash,
                    "download hash mismatch, expect: {}, got: {}",
                    expect_hash.1.hash, hash
                );
            }
        }
    }
}
