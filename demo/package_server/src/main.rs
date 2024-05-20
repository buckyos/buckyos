use async_std::fs::{self, File};
use async_std::path::Path;
use async_std::prelude::*;
use async_std::sync::Mutex;
use clap::{App, Arg};
use hex;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use sha2::{Digest, Sha256}; // 引入 sha2 crate
use simplelog::*;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tide::{Request, Response, StatusCode};

const DEFAULT_SERVER_PORT: &str = "3030";

#[derive(Deserialize, Debug)]
struct UploadPackageMeta {
    name: String,
    version: String,
    deps: HashMap<String, String>,
    author: Option<String>,
    sha256: String,
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
        let metadata_header = match req.header("Bkz-Upload-Metadata") {
            Some(header) => header,
            None => {
                let err_msg = "Missing Bkz-Upload-Metadata header";
                error!("{}", err_msg);
                return Ok(Response::builder(StatusCode::BadRequest)
                    .body(err_msg)
                    .build());
            }
        };

        debug!("Metadata header: {:?}", metadata_header.last().as_str());

        let metadata: UploadPackageMeta =
            match serde_json::from_str(metadata_header.last().as_str()) {
                Ok(meta) => meta,
                Err(e) => {
                    let err_msg = format!("Invalid JSON metadata: {}", e);
                    error!("{}", err_msg);
                    return Ok(Response::builder(StatusCode::BadRequest)
                        .body(err_msg)
                        .build());
                }
            };

        info!("Parsed JSON metadata: {:?}", metadata);

        // 构建文件名和路径
        let filename = format!("{}_{}.bkz", metadata.name, metadata.version);
        let package_dir = Path::new(&req.state().save_path).join(&metadata.name);
        let file_path = package_dir.join(&filename);
        let temp_file_path = package_dir.join(format!("{}.tmp", filename));

        // 先检查索引是否已有相同的包和版本
        {
            let _lock = req.state().index_mutex.lock().await; // 加锁
            let index = match Self::load_index(&req.state().index_path).await {
                Ok(index) => index,
                Err(e) => {
                    let err_msg = format!("Failed to load index: {}", e);
                    error!("{}", err_msg);
                    return Ok(Response::builder(StatusCode::InternalServerError)
                        .body(err_msg)
                        .build());
                }
            };

            if let Some(package_entry) = index.get(&metadata.name) {
                if package_entry.contains_key(&metadata.version) {
                    let err_msg = format!(
                        "Package version already exists: {}-{}",
                        metadata.name, metadata.version
                    );
                    warn!("{}", err_msg);
                    return Ok(Response::builder(StatusCode::Conflict)
                        .body(err_msg)
                        .build());
                }
            }
        } // 解锁

        // 检查目标文件是否已经存在
        if file_path.exists().await {
            let err_msg = format!("File already exists: {}", file_path.display());
            warn!("{}", err_msg);
            return Ok(Response::builder(StatusCode::Conflict)
                .body(err_msg)
                .build());
        }

        // 保存文件
        if let Err(e) = fs::create_dir_all(&package_dir).await {
            let err_msg = format!("Failed to create package directory: {}", e);
            error!("{}", err_msg);
            return Ok(Response::builder(StatusCode::InternalServerError)
                .body(err_msg)
                .build());
        }

        let mut file = match File::create(&temp_file_path).await {
            Ok(file) => file,
            Err(e) => {
                let err_msg = format!("Failed to create temp file: {}", e);
                error!("{}", err_msg);
                return Ok(Response::builder(StatusCode::InternalServerError)
                    .body(err_msg)
                    .build());
            }
        };

        let body = match req.body_bytes().await {
            Ok(body) => body,
            Err(e) => {
                let err_msg = format!("Failed to read request body: {}", e);
                error!("{}", err_msg);
                return Ok(Response::builder(StatusCode::InternalServerError)
                    .body(err_msg)
                    .build());
            }
        };

        // 计算 SHA-256 哈希值
        let mut hasher = Sha256::new();
        hasher.update(&body);
        let hash_result = hasher.finalize();
        let sha256_hash = hex::encode(hash_result);

        // 比较客户端提供的 SHA-256 值与计算的值
        if sha256_hash != metadata.sha256 {
            let err_msg = format!(
                "SHA-256 mismatch: expected {}, calculated {}",
                metadata.sha256, sha256_hash
            );
            error!("{}", err_msg);
            return Ok(Response::builder(StatusCode::BadRequest)
                .body(err_msg)
                .build());
        }

        if let Err(e) = file.write_all(&body).await {
            let err_msg = format!("Failed to write to temp file: {}", e);
            error!("{}", err_msg);
            return Ok(Response::builder(StatusCode::InternalServerError)
                .body(err_msg)
                .build());
        }

        if let Err(e) = file.sync_all().await {
            let err_msg = format!("Failed to sync temp file: {}", e);
            error!("{}", err_msg);
            return Ok(Response::builder(StatusCode::InternalServerError)
                .body(err_msg)
                .build());
        }

        if let Err(e) = fs::rename(&temp_file_path, &file_path).await {
            let err_msg = format!("Failed to rename temp file: {}", e);
            error!("{}", err_msg);
            return Ok(Response::builder(StatusCode::InternalServerError)
                .body(err_msg)
                .build());
        }

        info!("File saved successfully: {}", filename);

        // 更新索引
        {
            let _lock = req.state().index_mutex.lock().await;
            let mut index = match Self::load_index(&req.state().index_path).await {
                Ok(index) => index,
                Err(e) => {
                    let err_msg = format!("Failed to load index: {}", e);
                    error!("{}", err_msg);
                    return Ok(Response::builder(StatusCode::InternalServerError)
                        .body(err_msg)
                        .build());
                }
            };

            let package_entry = index
                .entry(metadata.name.clone())
                .or_insert_with(HashMap::new);
            let insert_value = json!({
                "deps": metadata.deps,
                "author": metadata.author,
                "sha256": sha256_hash,
            });
            package_entry.insert(metadata.version.clone(), insert_value.clone());
            info!(
                "Index updated, add entry: {}-{}: {:?}",
                metadata.name, metadata.version, insert_value
            );

            if let Err(e) = Self::save_index(&req.state().index_path, &index).await {
                let err_msg = format!("Failed to save index: {}", e);
                error!("{}", err_msg);
                return Ok(Response::builder(StatusCode::InternalServerError)
                    .body(err_msg)
                    .build());
            }
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
            LevelFilter::Debug,
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

    let matches = App::new("File Upload Server")
        .version("1.0")
        .about("A simple package upload server")
        .arg(
            Arg::with_name("port")
                .long("port")
                .value_name("PORT")
                .help("Sets the server port")
                .takes_value(true),
        )
        .get_matches();

    let port = matches.value_of("port").unwrap_or(DEFAULT_SERVER_PORT);
    let server_addr = format!("127.0.0.1:{}", port);

    async_std::task::block_on(async {
        let server_state = FileUploadServer::new();
        info!("Save path: {}", server_state.save_path);
        info!("Index path: {}", server_state.index_path);

        async_std::fs::create_dir_all(&server_state.save_path).await?;

        let mut app = tide::with_state(server_state);
        app.at("/upload").post(FileUploadServer::save_file);

        info!("Starting server at http://{}", server_addr);
        app.listen(server_addr).await?;
        Ok(())
    })
}
