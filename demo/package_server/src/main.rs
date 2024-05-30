use async_std::fs::{self, File, OpenOptions};
use async_std::path::{Path, PathBuf};
use async_std::prelude::*;
use async_std::sync::{Mutex, RwLock};
use clap::{App, Arg};
use hex;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use sha2::{Digest, Sha256};
use simplelog::*;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tide::{Request, Response, StatusCode};
use time::macros::format_description;

const DEFAULT_SERVER_PORT: &str = "13030";

#[derive(Deserialize, Debug)]
struct UploadPackageMeta {
    name: String,
    version: String,
    deps: HashMap<String, String>,
    author: Option<String>,
    sha256: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IndexDB {
    packages: HashMap<String, HashMap<String, PackageMetaInfo>>,
}

#[derive(Serialize, Deserialize, Debug)]
struct PackageMetaInfo {
    deps: HashMap<String, String>,
    sha256: String,
}

#[derive(Clone)]
struct PackageUploadServer {
    save_path: PathBuf,
    index_path: PathBuf,
    index_mutex: Arc<RwLock<()>>,
}

impl PackageUploadServer {
    fn new() -> Self {
        let (save_path, index_path) = if cfg!(target_os = "windows") {
            let appdata_dir = env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
            (
                PathBuf::from(format!("{}/bkpackage/uploads", appdata_dir)),
                PathBuf::from(format!("{}/bkpackage/index.json", appdata_dir)),
            )
        } else {
            (
                PathBuf::from("/var/bkpackage/uploads"),
                PathBuf::from("/var/bkpackage/index.json"),
            )
        };

        PackageUploadServer {
            save_path,
            index_path,
            index_mutex: Arc::new(RwLock::new(())),
        }
    }

    async fn save_package(mut req: Request<PackageUploadServer>) -> tide::Result {
        let metadata_header = match req.header("Bkz-Upload-Metadata") {
            Some(header) => header,
            None => {
                return respond_with_error(
                    StatusCode::BadRequest,
                    "Missing Bkz-Upload-Metadata header",
                )
            }
        };

        let metadata: UploadPackageMeta =
            match serde_json::from_str(metadata_header.last().as_str()) {
                Ok(meta) => meta,
                Err(e) => {
                    return respond_with_error(
                        StatusCode::BadRequest,
                        &format!("Invalid JSON metadata: {}", e),
                    )
                }
            };

        info!("Parsed JSON metadata: {:?}", metadata);

        let filename = format!("{}_{}.bkz", metadata.name, metadata.version);
        let package_dir = req.state().save_path.join(&metadata.name);
        let file_path = package_dir.join(&filename);
        let temp_file_path = package_dir.join(format!("{}.tmp", filename));

        {
            let _lock = req.state().index_mutex.read().await;
            let index = match Self::load_index(&req.state().index_path).await {
                Ok(index) => index,
                Err(e) => {
                    return respond_with_error(
                        StatusCode::InternalServerError,
                        &format!("Failed to load index: {}", e),
                    )
                }
            };

            if let Some(package_entry) = index.packages.get(&metadata.name) {
                if package_entry.contains_key(&metadata.version) {
                    return respond_with_error(
                        StatusCode::Conflict,
                        &format!(
                            "Package version already exists: {}-{}",
                            metadata.name, metadata.version
                        ),
                    );
                }
            }
        }

        if file_path.exists().await {
            return respond_with_error(
                StatusCode::Conflict,
                &format!("File already exists: {}", file_path.display()),
            );
        }

        if let Err(e) = fs::create_dir_all(&package_dir).await {
            return respond_with_error(
                StatusCode::InternalServerError,
                &format!("Failed to create package directory: {}", e),
            );
        }

        let mut file = match File::create(&temp_file_path).await {
            Ok(file) => file,
            Err(e) => {
                return respond_with_error(
                    StatusCode::InternalServerError,
                    &format!("Failed to create temp file: {}", e),
                )
            }
        };

        let body = match req.body_bytes().await {
            Ok(body) => body,
            Err(e) => {
                return respond_with_error(
                    StatusCode::InternalServerError,
                    &format!("Failed to read request body: {}", e),
                )
            }
        };

        let sha256_hash = calculate_sha256(&body);

        if sha256_hash != metadata.sha256 {
            return respond_with_error(
                StatusCode::BadRequest,
                &format!(
                    "SHA-256 mismatch: expected {}, calculated {}",
                    metadata.sha256, sha256_hash
                ),
            );
        }

        if let Err(e) = file.write_all(&body).await {
            return respond_with_error(
                StatusCode::InternalServerError,
                &format!("Failed to write to temp file: {}", e),
            );
        }

        if let Err(e) = file.sync_all().await {
            return respond_with_error(
                StatusCode::InternalServerError,
                &format!("Failed to sync temp file: {}", e),
            );
        }

        if let Err(e) = fs::rename(&temp_file_path, &file_path).await {
            return respond_with_error(
                StatusCode::InternalServerError,
                &format!("Failed to rename temp file: {}", e),
            );
        }

        info!("File saved successfully: {}", filename);

        {
            let _lock = req.state().index_mutex.write().await;
            let mut index = match Self::load_index(&req.state().index_path).await {
                Ok(index) => index,
                Err(e) => {
                    return respond_with_error(
                        StatusCode::InternalServerError,
                        &format!("Failed to load index: {}", e),
                    )
                }
            };

            let package_entry = index
                .packages
                .entry(metadata.name.clone())
                .or_insert_with(HashMap::new);
            let insert_value = PackageMetaInfo {
                deps: metadata.deps,
                sha256: sha256_hash,
            };
            package_entry.insert(metadata.version.clone(), insert_value);
            info!(
                "Index updated, add entry: {}-{}",
                metadata.name, metadata.version
            );

            if let Err(e) = Self::save_index(&req.state().index_path, &index).await {
                return respond_with_error(
                    StatusCode::InternalServerError,
                    &format!("Failed to save index: {}", e),
                );
            }
        }

        Ok(Response::new(StatusCode::Ok))
    }

    async fn load_index(index_path: &Path) -> tide::Result<IndexDB> {
        if index_path.exists().await {
            let index_data = fs::read_to_string(index_path).await.map_err(|e| {
                tide::Error::from_str(
                    StatusCode::InternalServerError,
                    format!("Failed to read index file: {}", e),
                )
            })?;
            let index: IndexDB = serde_json::from_str(&index_data).map_err(|e| {
                tide::Error::from_str(
                    StatusCode::InternalServerError,
                    format!("Failed to parse index file: {}", e),
                )
            })?;
            Ok(index)
        } else {
            Ok(IndexDB {
                packages: HashMap::new(),
            })
        }
    }

    async fn save_index(index_path: &Path, index: &IndexDB) -> tide::Result<()> {
        let index_data = serde_json::to_string_pretty(index).map_err(|e| {
            tide::Error::from_str(
                StatusCode::InternalServerError,
                format!("Failed to serialize index: {}", e),
            )
        })?;

        fs::write(index_path, index_data).await.map_err(|e| {
            tide::Error::from_str(
                StatusCode::InternalServerError,
                format!("Failed to write index file: {}", e),
            )
        })
    }

    async fn download_package(req: Request<PackageUploadServer>) -> tide::Result {
        let package_name = req.param("package_name")?;
        let version = req
            .query::<HashMap<String, String>>()
            .ok()
            .and_then(|q| q.get("version").cloned());

        info!(
            "Request download package: {}, version: {}",
            package_name,
            &version.clone().unwrap_or("*".to_string())
        );

        let state = req.state();
        let package_dir = state.save_path.join(package_name);

        let (file_path, filename) = if let Some(version) = version {
            let filename = format!("{}_{}.bkz", package_name, version);
            (package_dir.join(&filename), filename)
        } else {
            let mut latest_version = None;
            let mut latest_file_path = None;
            let mut latest_filename = None;

            let _lock = state.index_mutex.read().await;
            let index = match Self::load_index(&state.index_path).await {
                Ok(index) => index,
                Err(e) => {
                    return respond_with_error(
                        StatusCode::InternalServerError,
                        &format!("Failed to load index: {}", e),
                    )
                }
            };

            if let Some(package_entry) = index.packages.get(package_name) {
                for (ver, _) in package_entry {
                    if latest_version.is_none() || ver > latest_version.as_ref().unwrap() {
                        latest_version = Some(ver.clone());
                        latest_filename = Some(format!("{}_{}.bkz", package_name, ver));
                        latest_file_path =
                            Some(package_dir.join(&latest_filename.as_ref().unwrap()));
                    }
                }
            }

            let file_path = match latest_file_path {
                Some(path) => path,
                None => {
                    return respond_with_error(
                        StatusCode::NotFound,
                        &format!("Package not found: {}", package_name),
                    )
                }
            };
            let filename = latest_filename.unwrap();

            (file_path, filename)
        };

        if !file_path.exists().await {
            return respond_with_error(StatusCode::NotFound, "File not found");
        }

        let mut file = match File::open(file_path).await {
            Ok(file) => file,
            Err(e) => {
                return respond_with_error(
                    StatusCode::InternalServerError,
                    &format!("Failed to open file: {}", e),
                )
            }
        };

        let mut contents = Vec::new();
        if let Err(e) = file.read_to_end(&mut contents).await {
            return respond_with_error(
                StatusCode::InternalServerError,
                &format!("Failed to read file: {}", e),
            );
        }

        Ok(Response::builder(StatusCode::Ok)
            .body(contents)
            .content_type("application/octet-stream")
            .header(
                "Content-Disposition",
                format!("attachment; filename=\"{}\"", filename),
            )
            .build())
    }

    async fn download_index(req: Request<PackageUploadServer>) -> tide::Result {
        let state = req.state();
        let index_path = &state.index_path;

        let index_data;

        {
            let _lock = state.index_mutex.read().await;

            if index_path.exists().await {
                index_data = match fs::read_to_string(index_path).await {
                    Ok(data) => data,
                    Err(e) => {
                        return respond_with_error(
                            StatusCode::InternalServerError,
                            &format!("Failed to read index file: {}", e),
                        );
                    }
                };
            } else {
                warn!("Index file not found: {}", index_path.display());
                index_data = match serde_json::to_string_pretty(&IndexDB {
                    packages: HashMap::new(),
                }) {
                    Ok(data) => data,
                    Err(e) => {
                        return respond_with_error(
                            StatusCode::InternalServerError,
                            &format!("Failed to serialize index: {}", e),
                        );
                    }
                };
            }
        }

        Ok(Response::builder(StatusCode::Ok)
            .body(index_data)
            .content_type("application/json")
            .build())
    }
}

fn calculate_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn respond_with_error(status: StatusCode, message: &str) -> tide::Result {
    error!("{}", message);
    Ok(Response::builder(status).body(message).build())
}

fn main() -> tide::Result<()> {
    let config = ConfigBuilder::new()
        .set_location_level(LevelFilter::Info)
        .set_time_format_custom(format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
        ))
        .set_time_offset_to_local()
        .unwrap()
        .build();

    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            config,
            std::fs::File::create("package_server.log").unwrap(),
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
        let server_state = PackageUploadServer::new();
        info!("Save path: {}", server_state.save_path.display());
        info!("Index path: {}", server_state.index_path.display());

        async_std::fs::create_dir_all(&server_state.save_path).await?;

        let mut app = tide::with_state(server_state);
        app.at("/upload").post(PackageUploadServer::save_package);
        app.at("/download/:package_name")
            .get(PackageUploadServer::download_package);
        app.at("/index").get(PackageUploadServer::download_index);

        info!("Starting server at http://{}", server_addr);
        app.listen(server_addr).await?;
        Ok(())
    })
}
