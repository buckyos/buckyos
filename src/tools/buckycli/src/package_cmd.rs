use base64::{engine::general_purpose, Engine as _};
use core::hash;
use ed25519_dalek::{pkcs8::DecodePrivateKey, Signature, Signer, SigningKey};
use flate2::write::GzEncoder;
use flate2::Compression;
use ndn_lib::*;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json;
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use tar::Builder;
use tokio::io::AsyncWriteExt;
use toml::Value;

#[derive(Serialize, Debug)]
struct UploadPackageMeta {
    name: String,
    version: String,
    author: String,
    deps: Value,
    sha256: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct PackResult {
    pkg_id: String,
    version: String,
    vendor_did: String,
    target_file_path: PathBuf, // tarball path
}

#[derive(Deserialize)]
struct ServerError {
    error: String,
}

#[derive(Clone, Debug)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub author: String, //author did
    pub chunk_id: String,
    pub dependencies: String,
    pub sign: String, //sign of the chunk_id
}

/*
meta.json
{
    "name" : "Home Station",
    "description" : "Home Station",
    "vendor_did" : "did:bns:buckyos",
    "pkg_id" : "home-station",
    "version" : "0.1.0",
    "pkg_list" : {
        "amd64_docker_image" : {
            "pkg_id":"home-station-x86-img",
            "docker_image_name":"filebrowser/filebrowser:s6"
        },
        "aarch64_docker_image" : {
            "pkg_id":"home-station-arm64-img",
            "docker_image_name":"filebrowser/filebrowser:s6"
        },
        "web_pages" :{
            "pkg_id" : "home-station-web-page"
        }
    }
}
 */

pub async fn pack(path: &str) -> Result<PackResult, String> {
    let path = Path::new(path);
    if !path.exists() {
        eprintln!("Error: Specified directory does not exist");
        return Err(format!("Path {} does not exist", path.display()));
    }

    println!("Starting the packing process for directory: {:?}", path);

    // 检查目录
    let meta_path = path.join("meta.json");
    if !meta_path.exists() {
        eprintln!("Error: meta.json not found in directory");
        return Err("meta.json not found in directory".to_string());
    }

    let items: Vec<_> = fs::read_dir(path)
        .map_err(|e| format!("Error: Failed to read directory: {}", e.to_string()))?
        .filter_map(|entry| entry.ok())
        .collect();

    if items.len() <= 1 {
        eprintln!("Error: No files or folders found to pack");
        return Err("No files or folders found to pack".to_string());
    }

    println!("Found {} items to pack", items.len());

    // 解析 meta.json
    let meta_content = fs::read_to_string(&meta_path)
        .map_err(|err| format!("Error: Failed to read meta.json: {}", err.to_string()))?;
    let meta_data: Value = serde_json::from_str(&meta_content)
        .map_err(|err| format!("Error: Failed to parse meta.json: {}", err.to_string()))?;

    let pkg_id = meta_data
        .get("pkg_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: pkg_id missing in meta.json"))?;

    let version = meta_data
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: version missing in meta.json"))?;

    let vendor_did = meta_data
        .get("vendor_did")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: vendor_did missing in meta.json"))?;

    // TODO dependencies？
    println!(
        "Parsed meta.json: pkg_id = {}, version = {}, vendor_did = {}",
        pkg_id, version, vendor_did
    );

    // 创建 tarball
    let parent_dir = path.parent().unwrap();
    let tarball_name = format!("{}-{}.bkz", pkg_id, version);
    let tarball_path = parent_dir.join(&tarball_name);

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
        pkg_id, version, vendor_did
    );
    println!("Package created at: {:?}", tarball_path);

    let pack_ret = PackResult {
        pkg_id: pkg_id.to_string(),
        version: version.to_string(),
        vendor_did: vendor_did.to_string(),
        target_file_path: tarball_path,
    };

    Ok(pack_ret)
}

struct FileInfo {
    sha256: Vec<u8>,
    size: u64,
}

fn calculate_file_hash(file_path: &str) -> Result<FileInfo, String> {
    let file: File = File::open(file_path).map_err(|err| {
        format!(
            "Error: Failed to open package file {}: {}",
            file_path,
            err.to_string()
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 10 * 1024 * 1024]; // 10MB 缓冲区
    let mut file_size = 0;

    loop {
        let bytes_read = reader.read(&mut buffer).map_err(|err| {
            format!(
                "Error: Failed to read package file {} for hashing: {}",
                file_path,
                err.to_string()
            )
        })?;
        if bytes_read == 0 {
            break;
        }
        file_size += bytes_read as u64;
        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize().to_vec();

    Ok(FileInfo {
        sha256: hash,
        size: file_size,
    })
}

fn sign_data(pem_file: &str, data: &str) -> Result<String, String> {
    let signing_key = SigningKey::read_pkcs8_pem_file(pem_file).map_err(|e| {
        format!(
            "Error: Failed to parse private key from file {}: {}",
            pem_file,
            e.to_string()
        )
    })?;

    // 对数据进行签名
    let signature: Signature = signing_key.sign(data.as_bytes());

    // 将签名转换为 base64 编码的字符串
    let signature_base64 = general_purpose::STANDARD.encode(signature.to_bytes());

    Ok(signature_base64)
}

pub async fn publish(path: &str, private_key_file: &str) -> Result<(), String> {
    let pack_ret = pack(path).await?;

    let pack_file_path = pack_ret.target_file_path.clone();

    if !pack_file_path.exists() {
        eprintln!("Error: Package file {} not found", pack_file_path.display());
        return Err(format!(
            "Package file {} not found",
            pack_file_path.display()
        ));
    }

    let file_info = calculate_file_hash(pack_file_path.to_str().unwrap())?;
    let chunk_id = ChunkId::from_sha256_result(&file_info.sha256);

    let sign: String = sign_data(private_key_file, &chunk_id.to_string())?;
    println!(
        "chunk_id: {}, signature_base64: {}:",
        chunk_id.to_string(),
        sign
    );

    // 创建元数据
    let pkg_meta = PackageMeta {
        name: pack_ret.pkg_id,
        version: pack_ret.version,
        author: pack_ret.vendor_did,
        chunk_id: chunk_id.to_string(),
        dependencies: "".to_string(),
        sign,
    };

    // 上传chunk到repo
    let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some("repo_chunk_mgr"))
        .await
        .ok_or_else(|| "Failed to get repo named data mgr".to_string())?;

    println!("upload chunk_id: {}", chunk_id.to_string());

    let named_mgr = named_mgr.lock().await;

    let (mut chunk_writer, progress_info) = named_mgr
        .open_chunk_writer(&chunk_id, file_info.size, 0)
        .await
        .map_err(|e| {
            format!(
                "Failed to open chunk writer for chunk_id: {}, err:{}",
                chunk_id.to_string(),
                e.to_string()
            )
        })?;

    // 读取文件，按块写入
    let file = File::open(&pack_file_path).map_err(|e| {
        format!(
            "Failed to open package file: {}, err:{}",
            pack_file_path.display(),
            e.to_string()
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut buffer = [0u8; 10 * 1024 * 1024]; // 10MB 缓冲区
    loop {
        let bytes_read = reader.read(&mut buffer).map_err(|e| {
            format!(
                "Failed to read package file: {}, err:{}",
                pack_file_path.display(),
                e.to_string()
            )
        })?;
        if bytes_read == 0 {
            break;
        }
        chunk_writer
            .write(&buffer[..bytes_read])
            .await
            .map_err(|e| {
                format!(
                    "Failed to write chunk data for chunk_id: {}, err:{}",
                    chunk_id.to_string(),
                    e.to_string()
                )
            })?;
    }
    drop(chunk_writer);
    named_mgr
        .complete_chunk_writer(&chunk_id)
        .await
        .map_err(|e| {
            format!(
                "Failed to complete chunk writer for chunk_id: {}, err:{}",
                chunk_id.to_string(),
                e.to_string()
            )
        })?;
    drop(named_mgr);

    // 上传元数据到repo

    // let metadata = UploadPackageMeta {
    //     name: name.to_string(),
    //     version: version.to_string(),
    //     author: author.to_string(),
    //     deps: dependencies.clone(),
    //     sha256: hash_hex,
    // };

    // // 将元数据序列化为 JSON
    // let metadata_json = serde_json::to_string(&metadata).map_err(|e| {
    //     format!(
    //         "Error: Failed to serialize package metadata to JSON: {}",
    //         e.to_string()
    //     )
    // })?;

    // // 可读打印元数据
    // println!("Package metadata: {:#?}", metadata);
    // // 上传文件
    // let client = Client::new();
    // let mut headers = HeaderMap::new();
    // headers.insert(
    //     "Bkz-Upload-Metadata",
    //     HeaderValue::from_str(&metadata_json).unwrap(),
    // );

    // let response = client
    //     .post(server)
    //     .headers(headers)
    //     .body(buffer)
    //     .send()
    //     .await
    //     .map_err(|e| format!("Error: Failed to send package to server: {}", e.to_string()))?;

    // if response.status().is_success() {
    //     println!(
    //         "Package {} version {} by {} has been published successfully.",
    //         name, version, author
    //     );
    // } else {
    //     let status = response.status();
    //     let error_message = response.text().await.unwrap_or_else(|e| {
    //         eprintln!("Failed to read error message: {}", e);
    //         "No error message returned".to_string()
    //     });

    //     // 尝试解析 JSON 错误消息
    //     if let Ok(server_error) = serde_json::from_str::<ServerError>(&error_message) {
    //         eprintln!(
    //             "Failed to publish package: {:?}. Error message: {}",
    //             status, server_error.error
    //         );
    //         return Err(server_error.error);
    //     } else {
    //         eprintln!(
    //             "Failed to publish package: {:?}. Error message: {}",
    //             status, error_message
    //         );
    //         return Err(error_message);
    //     }
    // }

    Ok(())
}
