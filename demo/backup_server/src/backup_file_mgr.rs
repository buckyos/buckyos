use crate::backup_index::*;
use async_std::{
    fs::File,
    io::{BufWriter, ReadExt, WriteExt},
    path::Path,
    sync::{Arc, Mutex},
};
use base58::{FromBase58, ToBase58};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::error::Error;
use tide::Request;

const HTTP_HEADER_ZONEID: &'static str = "zone-id";
const HTTP_HEADER_KEY: &'static str = "backup-key";
const HTTP_HEADER_VERSION: &'static str = "backup-version";
const HTTP_HEADER_HASH: &'static str = "backup-hash";
const HTTP_HEADER_CHUNK_SEQ: &'static str = "backup-chunk-seq";
const HTTP_HEADER_CHUNK_RELATIVE_PATH: &'static str = "backup-chunk-relative-path";

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

#[derive(Clone, Deserialize, Serialize)]
pub struct QueryBackupVersionRespChunk {
    seq: u32,
    hash: String,
    size: u32,
    relative_path: String,
}

#[derive(Deserialize, Serialize)]
pub struct QueryBackupVersionResp {
    key: String,
    version: u32,
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
pub struct BackupFileMgr {
    save_path: Arc<String>,
    index_mgr: Arc<Mutex<BackupIndexSqlite>>, // files: Arc<Mutex<HashMap<String, BackupFile>>>,
}

impl BackupFileMgr {
    pub fn new(save_path: String) -> Result<Self, Box<dyn Error>> {
        let index_mgr =
            BackupIndexSqlite::init(format!("{}/backup.sqlite.db", save_path.as_str()).as_str())?;

        Ok(Self {
            save_path: Arc::new(save_path),
            index_mgr: Arc::new(Mutex::new(index_mgr)),
        })
    }

    /**
     * {
     *     zone_id: &str,
     *     key: &str,
     *     version: u32,
     *     meta: &str,
     * }
     */
    pub async fn create_backup(&self, mut req: Request<BackupFileMgr>) -> tide::Result {
        // TODO: 要防御一下版本号降低
        let req: CreateBackupReq = req.body_json().await?;
        self.index_mgr
            .lock()
            .await
            .insert_new_backup(
                req.zone_id.as_str(),
                req.key.as_str(),
                req.version,
                req.prev_version,
            )
            .map_err(|err| {
                log::error!("create_backup failed: {}", err);
                tide::Error::from_str(tide::StatusCode::InternalServerError, err.to_string())
            })?;

        Ok(tide::Response::new(tide::StatusCode::Ok))
    }

    pub async fn commit_backup(&self, mut req: Request<BackupFileMgr>) -> tide::Result {
        let req: CommitBackupReq = req.body_json().await?;
        let index_mgr = self.index_mgr.lock().await;
        let version_info = index_mgr
            .query_backup_version_info(req.zone_id.as_str(), req.key.as_str(), req.version)
            .map_err(|err| {
                log::error!(
                    "query_backup_version_info for commit-backup failed: {}",
                    err
                );
                tide::Error::from_str(tide::StatusCode::InternalServerError, err.to_string())
            })?;

        if version_info.chunk_count != req.chunk_count {
            log::error!("query_backup_version_info for commit-back chunk-count mismatch");
            return Err(tide::Error::from_str(
                tide::StatusCode::BadRequest,
                "chunk count mismatch",
            ));
        }

        if version_info.is_restorable {
            log::error!("has commited earlier");
            return Err(tide::Error::from_str(
                tide::StatusCode::BadRequest,
                "has commited earlier",
            ));
        }

        index_mgr
            .commit_backup(
                req.zone_id.as_str(),
                req.key.as_str(),
                req.version,
                req.meta.as_str(),
            )
            .map_err(|err| {
                log::error!("commit_backup failed: {}", err);
                tide::Error::from_str(tide::StatusCode::InternalServerError, err.to_string())
            })?;

        Ok(tide::Response::new(tide::StatusCode::Ok))
    }

    pub async fn save_chunk(&self, mut req: Request<BackupFileMgr>) -> tide::Result {
        log::info!("save_chunk http-headers: {:?}", req.header_names());
        let zone_id = match req.header(HTTP_HEADER_ZONEID) {
            Some(h) => h.last().to_string(),
            None => {
                log::error!("zone-id not found");
                return Err(tide::Error::from_str(
                    tide::StatusCode::NotFound,
                    "zone-id not found",
                ));
            }
        };

        let key = match req.header(HTTP_HEADER_KEY) {
            Some(h) => h.last().to_string(),
            None => {
                log::error!("key not found");
                return Err(tide::Error::from_str(
                    tide::StatusCode::NotFound,
                    "Key not found",
                ));
            }
        };

        let version = match req.header(HTTP_HEADER_VERSION) {
            Some(h) => {
                let version_str = h.last().to_string();
                u32::from_str_radix(version_str.as_str(), 10).map_err(|err| {
                    log::error!("parse version({}) for {} failed: {}", version_str, key, err);
                    tide::Error::from_str(
                        tide::StatusCode::BadRequest,
                        "Version should integer in radix-10",
                    )
                })?
            }
            None => {
                return Err(tide::Error::from_str(
                    tide::StatusCode::BadRequest,
                    "Version not found",
                ))
            }
        };

        let chunk_hash = match req.header(HTTP_HEADER_HASH) {
            Some(h) => {
                let hash = h.last().to_string();
                if let Err(err) = hash.from_base58() {
                    log::error!(
                        "parse hash({}) for {}-{} failed: {:?}",
                        hash,
                        key,
                        version,
                        err
                    );
                    return Err(tide::Error::from_str(
                        tide::StatusCode::BadRequest,
                        "hash should be base58",
                    ));
                }
                hash
            }
            None => {
                log::error!("hash not found for {}-{}", key, version);
                return Err(tide::Error::from_str(
                    tide::StatusCode::BadRequest,
                    "Version not found",
                ));
            }
        };

        let chunk_seq = match req.header(HTTP_HEADER_CHUNK_SEQ) {
            Some(h) => u32::from_str_radix(h.last().to_string().as_str(), 10).map_err(|err| {
                log::error!("parse chunk-seq for {}-{} failed: {}", key, version, err);
                tide::Error::from_str(
                    tide::StatusCode::BadRequest,
                    "Chunk-seq should integer in radix-10",
                )
            })?,
            None => {
                log::error!("chunk-seq not found for {}-{}", key, version);
                return Err(tide::Error::from_str(
                    tide::StatusCode::BadRequest,
                    "Chunk-seq not found",
                ));
            }
        };

        let chunk_relative_path = match req.header(HTTP_HEADER_CHUNK_RELATIVE_PATH) {
            Some(h) => h.last().to_string(),
            None => {
                log::error!("chunk-relative_path not found for {}-{}", key, version);
                return Err(tide::Error::from_str(
                    tide::StatusCode::BadRequest,
                    "Chunk-seq not found",
                ));
            }
        };

        let filename = Self::tmp_filename(zone_id.as_str(), key.as_str(), version, chunk_seq);
        let tmp_path = Path::new(self.save_path.as_str()).join(filename.as_str());
        let mut file = File::create(&tmp_path).await?;
        let mut writer = BufWriter::new(&mut file);
        let mut chunk_size = 0;

        let mut hasher = Sha256::new();

        // TODO 这里会一次接收整个body，可能会占用很大的内存
        loop {
            let body = req.body_bytes().await.map_err(|err| {
                log::error!("read stream {}-{} error: {}", key, version, err);
                err
            })?;

            if body.is_empty() {
                break;
            }

            hasher.update(body.as_slice());
            chunk_size += body.len();

            writer.write_all(body.as_slice()).await.map_err(|err| {
                log::error!("write stream {}-{} error: {}", key, version, err);
                err
            })?;
        }

        let hash = hasher.finalize();
        let hash = hash.as_slice().to_base58();

        writer.flush().await.map_err(|err| {
            log::error!("flush stream {}-{} error: {}", key, version, err);
            err
        })?;

        if hash != chunk_hash {
            log::error!(
                "check hash for chunk {}-{}-{} failed, should be {}, not {}",
                key,
                version,
                chunk_seq,
                chunk_hash,
                hash
            );

            let _todo = async_std::fs::remove_file(&tmp_path).await;

            return Err(tide::Error::from_str(
                tide::StatusCode::BadRequest,
                "hash unmatched",
            ));
        }

        let filename = Self::filename(
            zone_id.as_str(),
            key.as_str(),
            version,
            chunk_seq,
            chunk_hash.as_str(),
        );
        let file_path = Path::new(self.save_path.as_str()).join(filename.as_str());
        async_std::fs::rename(&tmp_path, &file_path)
            .await
            .map_err(|err| {
                log::error!(
                    "rename {} to {} failed: {}",
                    tmp_path.to_str().unwrap(),
                    file_path.to_str().unwrap(),
                    err
                );
                err
            })?;

        {
            let _todo = self.index_mgr
                .lock()
                .await
                .insert_new_chunk(
                    zone_id.as_str(),
                    key.as_str(),
                    version,
                    chunk_seq,
                    file_path.to_str().unwrap(),
                    chunk_hash.as_str(),
                    chunk_size as u32,
                    chunk_relative_path.as_str(),
                )
                .map_err(|err| {
                    log::warn!("insert_new_chunk failed: {}", err);
                    tide::Error::from_str(tide::StatusCode::InternalServerError, err.to_string())
                });
        }

        log::info!(
            "save chunk successed: {}-{}, path: {:?}",
            key,
            version,
            file_path
        );

        Ok(tide::Response::new(tide::StatusCode::Ok))
    }

    pub async fn query_versions(&self, mut req: Request<BackupFileMgr>) -> tide::Result {
        let req: QueryVersionReq = req.body_json().await?;

        let versions = {
            self.index_mgr
                .lock()
                .await
                .query_backup_versions(
                    req.zone_id.as_str(),
                    req.key.as_str(),
                    req.offset,
                    req.limit,
                    req.is_restorable_only,
                )
                .map_err(|err| {
                    tide::Error::from_str(tide::StatusCode::InternalServerError, err.to_string())
                })?
        };

        let resp_versions = versions
            .into_iter()
            .map(|v| QueryBackupVersionResp {
                key: v.key,
                version: v.version,
                meta: v.meta,
                is_restorable: v.is_restorable,
                chunk_count: v.chunk_count,
                prev_version: v.prev_version,
            })
            .collect::<Vec<_>>();
        let resp_body = serde_json::to_string(resp_versions.as_slice())?;

        let mut resp = tide::Response::new(tide::StatusCode::Ok);
        resp.set_content_type("application/json");
        resp.set_body(resp_body);

        Ok(resp)
    }

    pub async fn query_version_info(&self, mut req: Request<BackupFileMgr>) -> tide::Result {
        let req: QueryVersionInfoReq = req.body_json().await?;

        let version = {
            self.index_mgr
                .lock()
                .await
                .query_backup_version_info(req.zone_id.as_str(), req.key.as_str(), req.version)
                .map_err(|err| {
                    log::error!("query_version_info failed: {}", err);
                    tide::Error::from_str(tide::StatusCode::InternalServerError, err.to_string())
                })?
        };

        let resp_version = QueryBackupVersionResp {
            key: req.key,
            version: req.version,
            meta: version.meta,
            is_restorable: version.is_restorable,
            chunk_count: version.chunk_count,
            prev_version: version.prev_version,
        };
        let resp_body = serde_json::to_string(&resp_version)?;

        let mut resp = tide::Response::new(tide::StatusCode::Ok);
        resp.set_content_type("application/json");
        resp.set_body(resp_body);

        Ok(resp)
    }

    pub async fn query_chunk_info(&self, mut req: Request<BackupFileMgr>) -> tide::Result {
        let req: QueryChunkInfoReq = req.body_json().await?;

        let chunk = {
            self.index_mgr
                .lock()
                .await
                .query_chunk(
                    req.zone_id.as_str(),
                    req.key.as_str(),
                    req.version,
                    req.chunk_seq,
                )
                .map_err(|err| {
                    log::error!("query_version_info failed: {}", err);
                    tide::Error::from_str(tide::StatusCode::InternalServerError, err.to_string())
                })?
        };

        let resp_chunk = QueryBackupVersionRespChunk {
            seq: chunk.seq,
            hash: chunk.hash,
            size: chunk.size,
            relative_path: chunk.relative_path,
        };
        let resp_body = serde_json::to_string(&resp_chunk)?;

        let mut resp = tide::Response::new(tide::StatusCode::Ok);
        resp.set_content_type("application/json");
        resp.set_body(resp_body);

        Ok(resp)
    }

    pub async fn download_chunk(&self, mut req: Request<BackupFileMgr>) -> tide::Result {
        let req: DownloadBackupChunkReq = req.body_json().await?;
        let chunk = {
            self.index_mgr
                .lock()
                .await
                .query_chunk(
                    req.zone_id.as_str(),
                    req.key.as_str(),
                    req.version,
                    req.chunk_seq,
                )
                .map_err(|err| {
                    tide::Error::from_str(tide::StatusCode::InternalServerError, err.to_string())
                })?
        };

        // TODO: chunk太大可能很占内存
        let mut file = async_std::fs::File::open(chunk.path.as_str()).await?;
        let mut buf = vec![];
        file.read_to_end(&mut buf).await?;

        if buf.len() != chunk.size as usize {
            return Err(tide::Error::from_str(
                tide::StatusCode::InternalServerError,
                "chunk size mismatch",
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(buf.as_slice());
        let hash = hasher.finalize();
        let hash = hash.as_slice().to_base58();
        if hash != chunk.hash {
            return Err(tide::Error::from_str(
                tide::StatusCode::InternalServerError,
                "hash mismatch",
            ));
        }

        let mut resp = tide::Response::new(tide::StatusCode::Ok);
        // resp.set_content_type("application/json");
        resp.append_header(HTTP_HEADER_HASH, hash);
        resp.set_body(buf);

        Ok(resp)
    }

    fn tmp_filename(zone_id: &str, key: &str, version: u32, chunk_seq: u32) -> String {
        format!(
            "{}-{}-{}-{}.{}.{}.tmp",
            zone_id,
            key,
            version,
            chunk_seq,
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            rand::thread_rng().gen::<u64>()
        )
    }

    fn filename(zone_id: &str, key: &str, version: u32, chunk_seq: u32, hash: &str) -> String {
        format!("{}-{}-{}-{}.{}.bak", zone_id, key, version, chunk_seq, hash)
    }
}
