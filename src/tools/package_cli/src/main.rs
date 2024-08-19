use clap::{Parser, Subcommand};
use flate2::write::GzEncoder;
use flate2::Compression;
use log::*;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::Serialize;
use serde_json;
use sha2::{Digest, Sha256};
use simplelog::*;
use std::fs;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use tar::Builder;
use time::macros::format_description;
use toml::Value;

const PACKAGE_UPLOAD_URL: &str = "http://47.106.164.184/package/upload";

/// 命令行接口定义
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// 子命令定义
#[derive(Subcommand)]
enum Commands {
    Pack {
        #[arg(long, default_value = ".")]
        path: String,
    },
    Publish {
        #[arg(long, default_value = ".")]
        path: String,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Serialize, Debug)]
struct UploadPackageMeta {
    name: String,
    version: String,
    author: String,
    deps: Value,
    sha256: String,
}

#[derive(Deserialize)]
struct ServerError {
    error: String,
}

fn main() -> io::Result<()> {
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
    ])
    .unwrap();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Pack { path } => {
            pack(&path)?;
        }
        Commands::Publish { path, server } => {
            pack(&path)?;
            let server_url = server.as_deref().unwrap_or(PACKAGE_UPLOAD_URL);
            publish(&path, server_url)?;
        }
    }

    Ok(())
}

fn pack(path: &str) -> io::Result<()> {
    let path = Path::new(path);

    debug!("Starting the packing process for directory: {:?}", path);

    // 检查目录
    let package_toml_path = path.join("package.toml");
    if !package_toml_path.exists() {
        error!("package.toml not found in the specified directory");
        eprintln!("Error: package.toml not found in the specified directory");
        std::process::exit(1);
    }

    let items: Vec<_> = fs::read_dir(path)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name() != "target")
        .collect();
    if items.len() <= 1 {
        error!("No files or folders found to package (excluding target)");
        eprintln!("Error: No files or folders found to package (excluding target)");
        std::process::exit(1);
    }

    debug!("Found {} items to package (excluding target)", items.len());

    // 解析 package.toml
    let package_toml_content = fs::read_to_string(&package_toml_path)?;
    let package_data: Value = toml::from_str(&package_toml_content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let name = package_data
        .get("package")
        .and_then(|pkg| pkg.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Package name missing in package.toml",
            )
        })?;

    let version = package_data
        .get("package")
        .and_then(|pkg| pkg.get("version"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Package version missing in package.toml",
            )
        })?;

    let author = package_data
        .get("package")
        .and_then(|pkg| pkg.get("author"))
        .and_then(Value::as_str)
        .unwrap_or("Unknown");

    let default_dependencies = Value::Table(Default::default());
    let dependencies = package_data
        .get("dependencies")
        .unwrap_or(&default_dependencies);

    debug!(
        "Parsed package.toml: name = {}, version = {}, author = {}",
        name, version, author
    );
    debug!("Dependencies: {:?}", dependencies);

    // 创建 tarball
    let target_dir = path.join("target");
    fs::create_dir_all(&target_dir)?;
    let tarball_name = format!("{}-{}.bkz", name, version);
    let tarball_path = target_dir.join(&tarball_name);

    let tar_gz = File::create(&tarball_path)?;
    let enc = GzEncoder::new(tar_gz, Compression::default());
    let mut tar = Builder::new(enc);

    // 递归添加目录和文件
    fn append_dir_all(
        tar: &mut Builder<GzEncoder<File>>,
        path: &Path,
        base: &Path,
    ) -> io::Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            let name = path.strip_prefix(base).unwrap();

            // 排除 target 目录
            if name.starts_with("target") {
                continue;
            }

            if path.is_dir() {
                tar.append_dir(name, &path)?;
                append_dir_all(tar, &path, base)?;
            } else {
                tar.append_file(name, &mut File::open(&path)?)?;
            }
        }
        Ok(())
    }

    append_dir_all(&mut tar, path, path)?;

    tar.finish()?;

    info!(
        "Package {} version {} by {} has been packed successfully.",
        name, version, author
    );
    info!("Dependencies: {:?}", dependencies);
    info!("Tarball created at: {:?}", tarball_path);

    Ok(())
}

fn publish(path: &str, server: &str) -> io::Result<()> {
    let path = Path::new(path);

    // 解析 package.toml
    let package_toml_path = path.join("package.toml");
    let package_toml_content = fs::read_to_string(&package_toml_path)?;
    let package_data: Value = toml::from_str(&package_toml_content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let name: &str = package_data
        .get("package")
        .and_then(|pkg| pkg.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Package name missing in package.toml",
            )
        })?;

    let version = package_data
        .get("package")
        .and_then(|pkg| pkg.get("version"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Package version missing in package.toml",
            )
        })?;

    let author = package_data
        .get("package")
        .and_then(|pkg| pkg.get("author"))
        .and_then(Value::as_str)
        .unwrap_or("Unknown");

    let default_dependencies = Value::Table(Default::default());
    let dependencies = package_data
        .get("dependencies")
        .unwrap_or(&default_dependencies);

    debug!(
        "Parsed package.toml: name = {}, version = {}, author = {}",
        name, version, author
    );
    debug!("Dependencies: {:?}", dependencies);

    // 确保 tarball 存在
    let tarball_name = format!("{}-{}.bkz", name, version);
    let tarball_path = path.join("target").join(&tarball_name);

    if !tarball_path.exists() {
        error!("Tarball {} not found", tarball_name);
        eprintln!("Error: Tarball {} not found", tarball_name);
        std::process::exit(1);
    }

    // 计算 tarball 的 sha256
    let mut file = File::open(&tarball_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    hasher.update(&buffer);
    let hash = hasher.finalize();
    let hash_hex = format!("{:x}", hash);

    // 创建元数据
    let metadata = UploadPackageMeta {
        name: name.to_string(),
        version: version.to_string(),
        author: author.to_string(),
        deps: dependencies.clone(),
        sha256: hash_hex,
    };

    // 将元数据序列化为 JSON
    let metadata_json = serde_json::to_string(&metadata)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // 可读打印元数据
    info!("Package metadata: {:#?}", metadata);
    // 上传文件
    let client = Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        "Bkz-Upload-Metadata",
        HeaderValue::from_str(&metadata_json).unwrap(),
    );

    let response = client
        .post(server)
        .headers(headers)
        .body(buffer)
        .send()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    if response.status().is_success() {
        info!(
            "Package {} version {} by {} has been published successfully.",
            name, version, author
        );
    } else {
        let status = response.status();
        let error_message = response.text().unwrap_or_else(|e| {
            error!("Failed to read error message: {}", e);
            "No error message returned".to_string()
        });

        // 尝试解析 JSON 错误消息
        if let Ok(server_error) = serde_json::from_str::<ServerError>(&error_message) {
            error!(
                "Failed to publish package: {:?}. Error message: {}",
                status, server_error.error
            );
        } else {
            error!(
                "Failed to publish package: {:?}. Error message: {}",
                status, error_message
            );
        }

        std::process::exit(1);
    }

    Ok(())
}
