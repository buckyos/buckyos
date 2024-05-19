use async_std::fs::{self, File};
use async_std::path::Path;
use async_std::prelude::*;
use async_std::sync::Mutex;
use hex;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use sha2::{Digest, Sha256}; // 引入 sha2 crate
use simplelog::*;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tide::{Request, Response, StatusCode};

const SERVER_ADDR: &str = "127.0.0.1:3030";

#[derive(Deserialize, Debug)]
struct UploadMetadata {
    package_name: String,
    version: String,
    deps: Vec<String>,
    author: Option<String>,
}

#[derive(Clone)]
struct FileUploadServer {
    pub save_path: String,
    pub index_path: String,
    index_mutex: Arc<Mutex<()>>,
}

impl FileUploadServer {
    fn new() -> Self {
        let save_path;
        let index_path;
        if cfg!(target_os = "windows") {
            let appdata_dir = env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
            save_path = format!("{}/bkpackage/uploads", appdata_dir);
            index_path = format!("{}/bkpackage/index.json", appdata_dir);
        } else {
            save_path = "/var/bkpackage/uploads".to_string();
            index_path = "/var/bkpackage/index.json".to_string();
        }

        FileUploadServer {
            save_path,
            index_path,
            index_mutex: Arc::new(Mutex::new(())),
        }
    }

    async fn save_file(mut req: Request<FileUploadServer>) -> tide::Result {
        // 获取并解析元数据头
        let metadata_header = req.header("X-Upload-Metadata").ok_or_else(|| {
            let err_msg = "Missing X-Upload-Metadata header";
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::BadRequest, err_msg)
        })?;

        let metadata: UploadMetadata = serde_json::from_str(metadata_header.last().as_str())
            .map_err(|e| {
                let err_msg = format!("Invalid JSON metadata: {}", e);
                error!("{}", err_msg);
                tide::Error::from_str(StatusCode::BadRequest, err_msg)
            })?;

        info!("Parsed JSON metadata: {:?}", metadata);

        // 构建文件名和路径
        let filename = format!("{}_{}.bkz", metadata.package_name, metadata.version);
        let package_dir = Path::new(&req.state().save_path).join(&metadata.package_name);
        let file_path = package_dir.join(&filename);
        let temp_file_path = package_dir.join(format!("{}.tmp", filename));

        // 先检查索引是否已有相同的包和版本
        {
            let _lock = req.state().index_mutex.lock().await; // 加锁
            let index = Self::load_index(&req.state().index_path)
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to load index: {}", e);
                    error!("{}", err_msg);
                    tide::Error::from_str(StatusCode::InternalServerError, err_msg)
                })?;

            if let Some(package_entry) = index.get(&metadata.package_name) {
                if package_entry.contains_key(&metadata.version) {
                    let err_msg = format!(
                        "Package version already exists: {}-{}",
                        metadata.package_name, metadata.version
                    );
                    warn!("{}", err_msg);
                    return Err(tide::Error::from_str(StatusCode::Conflict, err_msg));
                }
            }
        } // 解锁

        // 检查目标文件是否已经存在
        if file_path.exists().await {
            let err_msg = format!("File already exists: {}", file_path.display());
            warn!("{}", err_msg);
            return Err(tide::Error::from_str(StatusCode::Conflict, err_msg));
        }

        // 保存文件
        fs::create_dir_all(&package_dir).await.map_err(|e| {
            let err_msg = format!("Failed to create package directory: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;

        let mut file = File::create(&temp_file_path).await.map_err(|e| {
            let err_msg = format!("Failed to create temp file: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;
        let body = req.body_bytes().await.map_err(|e| {
            let err_msg = format!("Failed to read request body: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;

        // 计算 SHA-256 哈希值，TODO，这里客户端还需要上传 SHA-256 值，并与计算的对比
        let mut hasher = Sha256::new();
        hasher.update(&body);
        let hash_result = hasher.finalize();
        let sha256_hash = hex::encode(hash_result);

        file.write_all(&body).await.map_err(|e| {
            let err_msg = format!("Failed to write to temp file: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;
        file.sync_all().await.map_err(|e| {
            let err_msg = format!("Failed to sync temp file: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;
        fs::rename(&temp_file_path, &file_path).await.map_err(|e| {
            let err_msg = format!("Failed to rename temp file: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;

        info!("File saved successfully: {}", filename);

        // 更新索引
        {
            let _lock = req.state().index_mutex.lock().await;
            let mut index = Self::load_index(&req.state().index_path)
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to load index: {}", e);
                    error!("{}", err_msg);
                    tide::Error::from_str(StatusCode::InternalServerError, err_msg)
                })?;

            let package_entry = index
                .entry(metadata.package_name.clone())
                .or_insert_with(HashMap::new);
            let insert_value = json!({
                "deps": metadata.deps,
                "author": metadata.author,
                "sha256": sha256_hash,
            });
            package_entry.insert(metadata.version.clone(), insert_value.clone());
            info!(
                "Index updated, add entry: {}-{}: {:?}",
                metadata.package_name, metadata.version, insert_value
            );
            Self::save_index(&req.state().index_path, &index)
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to save index: {}", e);
                    error!("{}", err_msg);
                    tide::Error::from_str(StatusCode::InternalServerError, err_msg)
                })?;
        }

        Ok(Response::new(StatusCode::Ok))
    }

    async fn load_index(index_path: &str) -> tide::Result<HashMap<String, HashMap<String, Value>>> {
        if Path::new(index_path).exists().await {
            let index_data = fs::read_to_string(index_path).await.map_err(|e| {
                let err_msg = format!("Failed to read index file: {}", e);
                error!("{}", err_msg);
                tide::Error::from_str(StatusCode::InternalServerError, err_msg)
            })?;
            let index: HashMap<String, HashMap<String, Value>> = serde_json::from_str(&index_data)
                .map_err(|e| {
                    let err_msg = format!("Failed to parse index file: {}", e);
                    error!("{}", err_msg);
                    tide::Error::from_str(StatusCode::InternalServerError, err_msg)
                })?;
            Ok(index)
        } else {
            Ok(HashMap::new())
        }
    }

    async fn save_index(
        index_path: &str,
        index: &HashMap<String, HashMap<String, Value>>,
    ) -> tide::Result<()> {
        let index_data = serde_json::to_string_pretty(index).map_err(|e| {
            let err_msg = format!("Failed to serialize index: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;
        let mut file = File::create(index_path).await.map_err(|e| {
            let err_msg = format!("Failed to create index file: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;
        file.write_all(index_data.as_bytes()).await.map_err(|e| {
            let err_msg = format!("Failed to write to index file: {}", e);
            error!("{}", err_msg);
            tide::Error::from_str(StatusCode::InternalServerError, err_msg)
        })?;
        Ok(())
    }
}

fn main() -> tide::Result<()> {
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Info,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            Config::default(),
            std::fs::File::create("package_server.log")?,
        ),
    ])?;

    async_std::task::block_on(async {
        let server_state = FileUploadServer::new();
        info!("Save path: {}", server_state.save_path);
        info!("Index path: {}", server_state.index_path);

        async_std::fs::create_dir_all(&server_state.save_path).await?;

        let mut app = tide::with_state(server_state);
        app.at("/upload").post(FileUploadServer::save_file);

        info!("Starting server at http://{}", SERVER_ADDR);
        app.listen(SERVER_ADDR).await?;
        Ok(())
    })
}
