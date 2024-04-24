use async_std::fs::File;
use async_std::io::BufWriter;
use async_std::prelude::*;
use async_std::sync::Mutex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tide::Request;

use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    save_path: String,
    interface: String,
    port: u16,
}

const HTTP_HEADER_KEY: &'static str = "BACKUP_KEY";
const HTTP_HEADER_VERSION: &'static str = "BACKUP_VERSION";
const HTTP_HEADER_METADATA: &'static str = "BACKUP_METADATA";

enum BackupStatus {
    Transfering,
    Temperature,
    Saved,
}

struct BackupFileVersion {
    version: u64,
    meta: String,
    status: BackupStatus,
    file_paths: Vec<String>,
}

struct BackupFile {
    versions: Vec<BackupFileVersion>, // 按版本号升序
}

#[derive(Clone)]
struct BackupFileMgr {
    save_path: Arc<String>,
    files: Arc<Mutex<HashMap<String, BackupFile>>>,
}

impl BackupFileMgr {
    pub fn new(save_path: String) -> Self {
        Self {
            save_path: Arc::new(save_path),
            files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn save_file(&self, mut req: Request<BackupFileMgr>) -> tide::Result {
        // 解析 multipart 表单
        let key = match req.header(HTTP_HEADER_KEY) {
            Some(h) => h.to_string(),
            None => {
                return Err(tide::Error::from_str(
                    tide::StatusCode::NotFound,
                    "Key not found",
                ))
            }
        };

        let version = match req.header(HTTP_HEADER_VERSION) {
            Some(h) => u64::from_str_radix(h.to_string().as_str(), 10).map_err(|err| {
                log::error!("parse version for {} failed: {}", key, err);
                tide::Error::from_str(
                    tide::StatusCode::BadRequest,
                    "Version should integer in radix-10",
                )
            })?,
            None => {
                return Err(tide::Error::from_str(
                    tide::StatusCode::BadRequest,
                    "Version not found",
                ))
            }
        };

        let meta = req
            .header(HTTP_HEADER_METADATA)
            .map(|m| m.to_string())
            .unwrap_or("".to_string());

        {
            let mut files_lock_guard = self.files.lock().await;

            let files = files_lock_guard
                .entry(key.clone())
                .or_insert(BackupFile { versions: vec![] });

            if let Some(last_version) = files.versions.last() {
                if last_version.version >= version {
                    return Err(tide::Error::from_str(
                        tide::StatusCode::BadRequest,
                        format!("Version should be larger than {}", last_version.version),
                    ));
                }
            }

            files.versions.push(BackupFileVersion {
                version,
                meta,
                status: BackupStatus::Transfering,
                file_paths: vec![],
            });
        }

        let filename = format!("{}-{}-{}.tmp", key, version, 0);
        let path = Path::new(self.save_path.as_str()).join(filename.as_str());
        let mut file = File::create(&path).await?;
        let mut writer = BufWriter::new(&mut file);

        // TODO 这里会一次接收整个body，可能会占用很大的内存
        loop {
            let body = req.body_bytes().await.map_err(|err| {
                log::error!("read stream {}-{} error: {}", key, version, err);
                err
            })?;

            if body.is_empty() {
                writer.flush().await.map_err(|err| {
                    log::error!("flush stream {}-{} error: {}", key, version, err);
                    err
                })?;

                let mut files_lock_guard = self.files.lock().await;

                let files = files_lock_guard.get_mut(key.as_str());

                // Temperature
                if let Some(files) = files {
                    if let Some(last_version) = files.versions.last() {
                        if last_version.version >= version {
                            return Err(tide::Error::from_str(
                                tide::StatusCode::BadRequest,
                                format!("Version should be larger than {}", last_version.version),
                            ));
                        }
                    }

                    if let Some(version) = files.versions.iter_mut().rfind(|v| v.version == version)
                    {
                        version.status = BackupStatus::Temperature;
                        version.file_paths.push(path.to_str().unwrap().to_owned());
                    }
                }

                break;
            }

            writer.write_all(body.as_slice()).await.map_err(|err| {
                log::error!("write stream {}-{} error: {}", key, version, err);
                err
            })?;
        }

        Ok(tide::Response::new(tide::StatusCode::Ok))
    }
}

async fn upload_file(mut req: Request<BackupFileMgr>) -> tide::Result {
    let backup_file_mgr = req.state().clone();

    backup_file_mgr.save_file(req).await
}

#[async_std::main]
async fn main() -> tide::Result<()> {
    let config_path = "./config.toml";
    let contents = tokio::fs::read_to_string(config_path)
        .await
        .map_err(|err| {
            log::error!("read config file failed: {}", err);
            err
        })
        .unwrap();

    // 解析 TOML 字符串到 Config
    let config: Config = toml::from_str(&contents)
        .map_err(|err| {
            log::error!("parse config file failed: {}", err);
            err
        })
        .unwrap();

    let mut app = tide::with_state(BackupFileMgr::new(config.save_path.clone()));

    app.at("/upload").post(upload_file);
    app.listen(format!("{}:{}", config.interface, config.port))
        .await
        .map_err(|err| {
            log::error!("listen failed: {}", err);
            err
        });

    Ok(())
}
