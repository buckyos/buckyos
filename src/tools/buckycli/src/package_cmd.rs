use flate2::write::GzEncoder;
use flate2::Compression;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json;
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use tar::Builder;
use toml::Value;

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

pub async fn pack(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if !path.exists() {
        eprintln!("Error: Specified directory does not exist");
        return Err(format!("Path {} does not exist", path.display()));
    }

    println!("Starting the packing process for directory: {:?}", path);

    // 检查目录
    let package_toml_path = path.join("package.toml");
    if !package_toml_path.exists() {
        eprintln!("Error: package.toml not found in the specified directory");
        return Err("package.toml not found in the specified directory".to_string());
    }

    let items: Vec<_> = fs::read_dir(path)
        .map_err(|e| format!("Error: Failed to read directory: {}", e.to_string()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name() != "target")
        .collect();

    if items.len() <= 1 {
        eprintln!("Error: No files or folders found to package (excluding target)");
        return Err("No files or folders found to package (excluding target)".to_string());
    }

    println!("Found {} items to package (excluding target)", items.len());

    // 解析 package.toml
    let package_toml_content = fs::read_to_string(&package_toml_path)
        .map_err(|err| format!("Error: Failed to read package.toml: {}", err.to_string()))?;
    let package_data: Value = toml::from_str(&package_toml_content)
        .map_err(|err| format!("Error: Failed to parse package.toml: {}", err.to_string()))?;

    let name = package_data
        .get("package")
        .and_then(|pkg| pkg.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: Package name missing in package.toml"))?;

    let version = package_data
        .get("package")
        .and_then(|pkg| pkg.get("version"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: Package version missing in package.toml"))?;

    let author = package_data
        .get("package")
        .and_then(|pkg| pkg.get("author"))
        .and_then(Value::as_str)
        .unwrap_or("Unknown");

    let default_dependencies = Value::Table(Default::default());
    let dependencies = package_data
        .get("dependencies")
        .unwrap_or(&default_dependencies);

    println!(
        "Parsed package.toml: name = {}, version = {}, author = {}",
        name, version, author
    );
    println!("Dependencies: {:?}", dependencies);

    // 创建 tarball
    let target_dir = path.join("target");
    fs::create_dir_all(&target_dir).map_err(|err| {
        format!(
            "Error: Failed to create target directory for package: {}",
            err.to_string()
        )
    })?;
    let tarball_name = format!("{}-{}.bkz", name, version);
    let tarball_path = target_dir.join(&tarball_name);

    let tar_gz = File::create(&tarball_path)
        .map_err(|err| format!("Error: Failed to create package file: {}", err.to_string()))?;
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

    append_dir_all(&mut tar, path, path).map_err(|err| {
        format!(
            "Error: Failed to add files and directories to tarball: {}",
            err.to_string()
        )
    })?;

    tar.finish().map_err(|err| {
        format!(
            "Error: Failed to finalize package creation: {}",
            err.to_string()
        )
    })?;

    println!(
        "Package {} version {} by {} has been packed successfully.",
        name, version, author
    );
    println!("Package created at: {:?}", tarball_path);

    Ok(())
}

pub async fn publish(path: &str, server: &str) -> Result<(), String> {
    let path = Path::new(path);

    // 解析 package.toml
    let package_toml_path = path.join("package.toml");
    let package_toml_content = fs::read_to_string(&package_toml_path)
        .map_err(|err| format!("Error: Failed to read package.toml: {}", err.to_string()))?;
    let package_data: Value = toml::from_str(&package_toml_content)
        .map_err(|e| format!("Error: Failed to parse package.toml: {}", e.to_string()))?;

    let name: &str = package_data
        .get("package")
        .and_then(|pkg| pkg.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: Package name missing in package.toml"))?;

    let version = package_data
        .get("package")
        .and_then(|pkg| pkg.get("version"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "Error: Package version missing in package.toml for package {}",
                name
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

    println!(
        "Parsed package.toml: name = {}, version = {}, author = {}",
        name, version, author
    );
    println!("Dependencies: {:?}", dependencies);

    // 确保 tarball 存在
    let tarball_name = format!("{}-{}.bkz", name, version);
    let tarball_path = path.join("target").join(&tarball_name);

    if !tarball_path.exists() {
        eprintln!("Error: Package file {} not found", tarball_name);
        return Err(format!("Package file {} not found", tarball_name));
    }

    // 计算 tarball 的 sha256
    let mut file = File::open(&tarball_path).map_err(|err| {
        format!(
            "Error: Failed to open package file {} for reading: {}",
            tarball_name,
            err.to_string()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(|err| {
        format!(
            "Error: Failed to read package file {} for hashing: {}",
            tarball_name,
            err.to_string()
        )
    })?;
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
    let metadata_json = serde_json::to_string(&metadata).map_err(|e| {
        format!(
            "Error: Failed to serialize package metadata to JSON: {}",
            e.to_string()
        )
    })?;

    // 可读打印元数据
    println!("Package metadata: {:#?}", metadata);
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
        .await
        .map_err(|e| format!("Error: Failed to send package to server: {}", e.to_string()))?;

    if response.status().is_success() {
        println!(
            "Package {} version {} by {} has been published successfully.",
            name, version, author
        );
    } else {
        let status = response.status();
        let error_message = response.text().await.unwrap_or_else(|e| {
            eprintln!("Failed to read error message: {}", e);
            "No error message returned".to_string()
        });

        // 尝试解析 JSON 错误消息
        if let Ok(server_error) = serde_json::from_str::<ServerError>(&error_message) {
            eprintln!(
                "Failed to publish package: {:?}. Error message: {}",
                status, server_error.error
            );
            return Err(server_error.error);
        } else {
            eprintln!(
                "Failed to publish package: {:?}. Error message: {}",
                status, error_message
            );
            return Err(error_message);
        }
    }

    Ok(())
}
