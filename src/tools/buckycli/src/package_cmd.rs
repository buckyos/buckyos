#[allow(dead_code, unused_variables)]
use crate::util::*;
use flate2::write::GzEncoder;
use flate2::Compression;
use jsonwebtoken::{encode, Algorithm, Header};
use kRPC::kRPC;
use ndn_lib::*;
use package_installer::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use tar::Builder;
use tokio::io::AsyncWriteExt;

#[derive(Serialize, Deserialize, Debug)]
pub struct PackResult {
    pkg_name: String,
    version: String,
    hostname: String,
    dependencies: String,
    target_file_path: PathBuf, // tarball path
    meta_content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackagePubMeta {
    pub pkg_name: String,
    pub version: String,
    pub hostname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    pub dependencies: String,
}

//index的chunkid需要在repo中计算
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexPubMeta {
    pub version: String,
    pub hostname: String,
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

pub async fn pack_pkg(pkg_path: &str) -> Result<PackResult, String> {
    println!("pack path: {}", pkg_path);
    let pkg_path = Path::new(pkg_path);
    if !pkg_path.exists() {
        eprintln!("Error: Specified directory does not exist");
        return Err(format!("Path {} does not exist", pkg_path.display()));
    }

    println!("Starting the packing process for directory: {:?}", pkg_path);

    // 检查目录
    let meta_path = pkg_path.join("meta.json");
    if !meta_path.exists() {
        eprintln!("Error: meta.json not found in directory");
        return Err("meta.json not found in directory".to_string());
    }

    let items: Vec<_> = fs::read_dir(pkg_path)
        .map_err(|e| format!("Error: Failed to read directory: {}", e.to_string()))?
        .filter_map(|entry| entry.ok())
        .collect();

    // if items.len() <= 1 {
    //     eprintln!("Error: No files or folders found to pack");
    //     return Err("No files or folders found to pack".to_string());
    // }

    println!("Found {} items to pack", items.len());

    // 解析 meta.json
    /*
        {
        "name" : "Home Station",
        "description" : "Home Station",
        "hostname" : "test.buckyos.io",
        "pkg_name" : "test_pkg",
        "version" : "0.1.0",
        "pkg_list" : {
            "amd64_docker_image" : {
                "package_name":"home-station-x86-img",
                "version": "*"
                "docker_image_name":"filebrowser/filebrowser:s6"
            },
            "web_pages" :{
                "package_name" : "home-station-web-page",
                "version": "0.0.1"
            }
        }
    }
         */
    let meta_content = fs::read_to_string(&meta_path)
        .map_err(|err| format!("Error: Failed to read meta.json: {}", err.to_string()))?;
    let meta_data: Value = serde_json::from_str(&meta_content)
        .map_err(|err| format!("Error: Failed to parse meta.json: {}", err.to_string()))?;

    let pkg_name = meta_data
        .get("pkg_name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: pkg_name missing in meta.json"))?;

    let version = meta_data
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: version missing in meta.json"))?;

    let hostname = meta_data
        .get("hostname")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Error: hostname missing in meta.json"))?;

    let deps = match meta_data.get("pkg_list") {
        Some(deps) => {
            //转换deps为Value::Object
            let deps = deps
                .as_object()
                .ok_or_else(|| format!("Error: pkg_list is not a map"))?;

            //依次遍历deps，获取pkg_name和version
            let mut deps_pkgs = HashMap::new();
            for (item_name, pkg_data) in deps.iter() {
                let pkg_data = pkg_data
                    .as_object()
                    .ok_or_else(|| format!("Error: pkg_list item {} is not a map", pkg_name))?;

                let pkg_name = pkg_data
                    .get("package_name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        format!("Error: sub pkg name missing in pkg_list item {}", item_name)
                    })?;

                let version = pkg_data
                    .get("version")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        format!(
                            "Error: sub pkg version missing in pkg_list item {}",
                            item_name
                        )
                    })?;

                deps_pkgs.insert(pkg_name.to_string(), version.to_string());
            }
            serde_json::to_string(&deps_pkgs).map_err(|e| {
                format!(
                    "Error: Failed to serialize dependencies to json string: {}",
                    e.to_string()
                )
            })?
        }
        None => "{}".to_string(),
    };

    // TODO dependencies？
    println!(
        "Parsed meta.json: pkg_name = {}, version = {}, hostname = {}, dependencies = {}",
        pkg_name, version, hostname, deps
    );

    // 创建 tarball
    let parent_dir = pkg_path.parent().unwrap();
    let tarball_name = format!("{}-{}.bkz", pkg_name, version);
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

    append_dir_all(&mut tar, pkg_path, pkg_path).map_err(|err| {
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
        pkg_name, version, hostname
    );
    println!("Package created at: {:?}", tarball_path);

    let pack_ret = PackResult {
        pkg_name: pkg_name.to_string(),
        version: version.to_string(),
        hostname: hostname.to_string(),
        target_file_path: tarball_path,
        dependencies: deps,
        meta_content,
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
    let mut buffer = vec![0u8; 10 * 1024 * 1024]; // 10MB 缓冲区
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

fn generate_jwt(pem_file: &str, data: &String) -> Result<String, String> {
    let private_key = load_private_key_from_file(pem_file)?;
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = None;
    header.typ = None;
    let token = encode(&header, data, &private_key)
        .map_err(|e| format!("Failed to encode data to JWT: {}", e.to_string()))?;

    Ok(token)
}

async fn write_file_to_chunk(
    chunk_id: &ChunkId,
    file_path: &PathBuf,
    file_size: u64,
    chunk_mgr_id: Option<&str>,
) -> Result<(), String> {
    let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(chunk_mgr_id)
        .await
        .ok_or_else(|| "Failed to get repo named data mgr".to_string())?;

    println!("upload chunk_id: {}", chunk_id.to_string());

    let named_mgr = named_mgr.lock().await;

    // 可能重复pub，需要排除AlreadyExists错误
    let (mut chunk_writer, progress_info) =
        match named_mgr.open_chunk_writer(chunk_id, file_size, 0).await {
            Ok(v) => v,
            Err(e) => {
                if let NdnError::AlreadyExists(_) = e {
                    println!("chunk {} already exists", chunk_id.to_string());
                    return Ok(());
                } else {
                    return Err(format!(
                        "Failed to open chunk writer for chunk_id: {}, err:{}",
                        chunk_id.to_string(),
                        e.to_string()
                    ));
                }
            }
        };

    // 读取文件，按块写入
    let file = File::open(&file_path).map_err(|e| {
        format!(
            "Failed to open package file: {}, err:{}",
            file_path.display(),
            e.to_string()
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut buffer = vec![0u8; 10 * 1024 * 1024]; // 使用 Vec 在堆上分配
    loop {
        let bytes_read = reader.read(&mut buffer).map_err(|e| {
            format!(
                "Failed to read package file: {}, err:{}",
                file_path.display(),
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
        .complete_chunk_writer(chunk_id)
        .await
        .map_err(|e| {
            format!(
                "Failed to complete chunk writer for chunk_id: {}, err:{}",
                chunk_id.to_string(),
                e.to_string()
            )
        })?;

    Ok(())
}

pub async fn publish_package(
    pkg_path: &str,
    did: &str,
    hostname: &str,
    pem_file: &str,
    url: &str,
    session_token: &str,
) -> Result<(), String> {
    let pack_ret = pack_pkg(pkg_path).await?;

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

    // 上传chunk到repo
    let chunk_mgr_id = None;
    write_file_to_chunk(&chunk_id, &pack_file_path, file_info.size, chunk_mgr_id)
        .await
        .map_err(|e| {
            format!(
                "Failed to upload package file to chunk mgr:{:?}, err:{:?}",
                chunk_mgr_id, e
            )
        })?;

    println!("upload chunk to chunk mgr:{:?} success", chunk_mgr_id);

    // 创建元数据
    let pkg_meta = PackagePubMeta {
        pkg_name: pack_ret.pkg_name,
        version: pack_ret.version,
        hostname: hostname.to_string(),
        chunk_id: Some(chunk_id.to_string()),
        dependencies: pack_ret.dependencies,
    };

    let jwt_token: String = generate_jwt(pem_file, &pack_ret.meta_content)?;
    println!("pub meta: {:?}, jwt_token: {}:", pkg_meta, jwt_token);

    // 上传元数据到repo
    let client = kRPC::new(url, Some(session_token.to_string()));

    client
        .call(
            "pub_pkg",
            json!({
                "pkg_name": pkg_meta.pkg_name,
                "version": pkg_meta.version,
                "hostname": pkg_meta.hostname,
                "chunk_id": pkg_meta.chunk_id.unwrap(),
                "dependencies": pkg_meta.dependencies,
                "jwt": jwt_token,
            }),
        )
        .await
        .map_err(|e| format!("Failed to publish package meta to repo, err:{:?}", e))?;

    Ok(())
}

pub async fn publish_index(
    pem_file: &str,
    version: &str,
    hostname: &str,
    url: &str,
    session_token: &str,
) -> Result<(), String> {
    let pub_meta = IndexPubMeta {
        version: version.to_string(),
        hostname: hostname.to_string(),
    };
    let meta_json_value = serde_json::to_string(&pub_meta).map_err(|e| {
        format!(
            "Failed to serialize index meta to json value, err:{:?}",
            e.to_string()
        )
    })?;
    let jwt_token: String = generate_jwt(pem_file, &meta_json_value)?;

    let client = kRPC::new(url, Some(session_token.to_string()));

    client
        .call(
            "pub_index",
            json!({
                "version": version.to_string(),
                "hostname": hostname.to_string(),
                "jwt": jwt_token,
            }),
        )
        .await
        .map_err(|e| format!("Failed to publish index, err:{:?}", e))?;

    Ok(())
}

pub async fn install_pkg(
    pkg_name: &str,
    version: &str,
    dest_dir: &str,
    url: &str,
) -> Result<(), String> {
    println!(
        "install package: {}, version: {}, dest_dir: {}, url: {}",
        pkg_name, version, dest_dir, url
    );
    let pkg_id = format!("{}#{}", pkg_name, version);

    let deps = Installer::install(&pkg_id, &PathBuf::from(dest_dir), url, None)
        .await
        .map_err(|e| format!("Failed to call install package, err:{:?}", e))?;

    println!("install package success, deps: {:?}", deps);

    Ok(())
}

pub async fn publish_app(
    app_desc_file: &PathBuf,
    did: &str,
    hostname: &str,
    pem_file: &str,
    url: &str,
    session_token: &str,
) -> Result<(), String> {
    if !app_desc_file.exists() {
        eprintln!("Error: App desc file {} not found", app_desc_file.display());
        return Err(format!(
            "App desc file {} not found",
            app_desc_file.display()
        ));
    }

    let desc_content = fs::read_to_string(app_desc_file)
        .map_err(|err| format!("Error: Failed to read app desc file: {}", err.to_string()))?;

    let app_desc: HashMap<String, String> = serde_json::from_str(&desc_content)
        .map_err(|err| format!("Error: Failed to parse app desc file: {}", err.to_string()))?;

    let app_name = app_desc
        .get("app_name")
        .ok_or_else(|| format!("Error: app_name missing in app desc file"))?;
    let version = app_desc
        .get("version")
        .ok_or_else(|| format!("Error: version missing in app desc file"))?;
    let hostname = app_desc
        .get("hostname")
        .ok_or_else(|| format!("Error: hostname missing in app desc file"))?;
    let pkg_list = app_desc
        .get("pkg_list")
        .ok_or_else(|| format!("Error: pkg_list missing in app desc file"))?;

    // 创建元数据
    let pkg_meta = PackagePubMeta {
        pkg_name: app_name.to_string(),
        version: version.to_string(),
        hostname: hostname.to_string(),
        chunk_id: None,
        dependencies: pkg_list.to_string(),
    };

    let meta_json_value = serde_json::to_value(&pkg_meta).map_err(|e| {
        format!(
            "Failed to serialize app meta to json value, err:{:?}",
            e.to_string()
        )
    })?;

    let jwt_token: String = generate_jwt(pem_file, &desc_content)?;

    // 上传元数据到repo
    let client = kRPC::new(url, Some(session_token.to_string()));

    client
        .call(
            "pub_app",
            json!({
                "pkg_name": pkg_meta.pkg_name,
                "version": pkg_meta.version,
                "hostname": hostname.to_string(),
                "dependencies": pkg_meta.dependencies,
                "jwt": jwt_token,
            }),
        )
        .await
        .map_err(|e| format!("Failed to publish app meta to repo, err:{:?}", e))?;

    Ok(())
}
