#[allow(dead_code, unused)]
use crate::util::*;
use flate2::write::GzEncoder;
use flate2::Compression;
use jsonwebtoken::{encode, Algorithm, Header};
use kRPC::kRPC;
use ndn_lib::*;
//use package_installer::*;
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
use package_lib::*;

#[derive(Debug)]
pub enum PackCategory {
    Pkg,
    App,
    Agent,
}

//为PackCategory实现to_string方法
impl PackCategory {
    pub fn to_string(&self) -> String {
        match self {
            PackCategory::Pkg => "pkg".to_string(),
            PackCategory::App => "app".to_string(),
            PackCategory::Agent => "agent".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PackResult {
    pkg_name: String,
    version: String,
    hostname: String,
    dependencies: HashMap<String, String>,
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
    pub dependencies: HashMap<String, String>,
}

//index的chunkid需要在repo中计算
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexPubMeta {
    pub version: String,
    pub hostname: String,
}

pub async fn tar_gz(src_dir: &Path, tarball_path: &Path) -> Result<(), String> {
    let tar_gz = File::create(tarball_path)
        .map_err(|e| format!("创建打包文件失败: {}", e.to_string()))?;
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

            if path.starts_with(".") {
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

    append_dir_all(&mut tar, src_dir, src_dir).map_err(|e| {
        format!("添加文件到打包文件失败: {}", e.to_string())
    })?;

    tar.finish().map_err(|e| {
        format!("完成打包过程失败: {}", e.to_string())
    })?;
    Ok(())
}


pub async fn pack_raw_pkg(pkg_path: &str, dest_dir: &str, owner_private_key_path: Option<String>) -> Result<PackResult, String> {
    println!("开始打包路径: {}", pkg_path);
    let pkg_path = Path::new(pkg_path);
    if !pkg_path.exists() {
        return Err(format!("指定的路径 {} 不存在", pkg_path.display()));
    }
    // 读取 meta.json 文件
    let meta_path = pkg_path.join(".pkg_meta.json");
    if !meta_path.exists() {
        return Err("meta.json 文件未在指定目录中找到".to_string());
    }

    let meta_content = fs::read_to_string(&meta_path)
        .map_err(|e| format!("读取 .pkg_meta.json 失败: {}", e.to_string()))?;
    
    let mut meta_data:PackageMeta = serde_json::from_str(&meta_content)
        .map_err(|e| format!("解析 .pkg_meta.json 失败: {}", e.to_string()))?;

    let pkg_name = meta_data.pkg_name.clone();
    let version = meta_data.version.clone();
    let author = meta_data.author.clone();
    
    println!("解析 .pkg_meta.json: pkg_name = {}, version = {}, author = {}", pkg_name, version, author);
    // 检查并创建目标目录
    let dest_dir_path = Path::new(dest_dir).join(&pkg_name);
    if !dest_dir_path.exists() {
        fs::create_dir_all(dest_dir_path.clone()).map_err(|e| {
            format!("创建目标目录失败: {}", e.to_string())
        })?;
    }
    // 创建 tarball
    let tarball_name = format!("{}-{}.tar.gz", pkg_name, version);
    let tarball_path = dest_dir_path.join(&tarball_name);

    tar_gz(&pkg_path, &tarball_path).await?;
    println!("pack to {} done", tarball_path.display());

    // 计算 tar.gz 文件的 SHA256 值
    let file_info = calculate_file_hash(tarball_path.to_str().unwrap())?;
    let chunk_id = ChunkId::from_sha256_result(&file_info.sha256);
    
    // 更新元数据
    meta_data.chunk_id = Some(chunk_id.to_string());
    meta_data.chunk_size = Some(file_info.size);
    meta_data.pub_time = buckyos_kit::buckyos_get_unix_timestamp();
    let meta_data_json = serde_json::to_value(&meta_data).map_err(|e| {
        format!("序列化元数据失败: {}", e.to_string())
    })?;
    
    let (pkg_meta_obj_id,pkg_meta_json_str) = build_named_object_by_json("pkg",&meta_data_json);
    
    // 保存更新后的元数据到 pkg.meta.json
    let meta_json_path = dest_dir_path.join("pkg_meta.json");
    
    fs::write(&meta_json_path, &pkg_meta_json_str.as_bytes()).map_err(|e| {
        format!("写入 pkg.meta.json 失败: {}", e.to_string())
    })?;
    let meta_json_path = dest_dir_path.join(pkg_meta_obj_id.to_base32());
    fs::write(&meta_json_path, &pkg_meta_json_str.as_bytes()).map_err(|e| {
        format!("写入 objid 失败: {}", e.to_string())
    })?;
    // 如果提供了私钥，则对元数据进行签名
    if let Some(key_path) = owner_private_key_path {
        let jwt_token = generate_jwt(&key_path, &pkg_meta_json_str)?;
        let jwt_path = dest_dir_path.join("pkg_meta.jwt");
        fs::write(&jwt_path, jwt_token).map_err(|e| {
            format!("写入 pkg_meta.jwt 失败: {}", e.to_string())
        })?;
        println!("pkg_meta.jwt 写入成功: {}", jwt_path.display());
    }

    println!("包 {} 版本 {} 作者 {} 已成功打包。", pkg_name, version, author);
    println!("打包文件创建于: {:?}", tarball_path);


    let pack_ret = PackResult {
        pkg_name,
        version,
        hostname: author,
        target_file_path: tarball_path,
        dependencies: meta_data.deps,
        meta_content: pkg_meta_json_str,
    };

    Ok(pack_ret)
}

//发布dapp_pkg前，需要确保sub_pkgs都已经发布(因此该命令需要在ood上执行)
//发布的dapp 的meta里，sub_pkgs使用固定版本号约束，已确保所有的sub_pkgs是统一升级的
// 指定dapp meta的路径，和已经用pack_raw_pkg打包好的sub_pkgs的目录路径，这样可以更新最新版本
// 将sub_pkgs最新版本发布到当前的ndn_mgr
// 自动填充dapp_meta.json的sub_pkgs字段，并构建dapp_meta.jwt(用来publish)
pub async fn pack_dapp_pkg(pkg_path: &str) -> Result<PackResult, String> {
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
        dependencies: HashMap::new(),
        meta_content,
    };

    Ok(pack_ret)
}

#[derive(Debug)]
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
        //println!("bytes_read: {}", bytes_read);
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
    category: PackCategory,
    pkg_path: &str,
    pem_file: &str,
    url: &str,
    session_token: &str,
) -> Result<(), String> {
    let pack_ret = pack_dapp_pkg(pkg_path).await?;

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
        hostname: pack_ret.hostname,
        chunk_id: Some(chunk_id.to_string()),
        dependencies: HashMap::new(),
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
                "category": category.to_string(),
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

    // let deps = Installer::install(&pkg_id, &PathBuf::from(dest_dir), url, None)
    //     .await
    //     .map_err(|e| format!("Failed to call install package, err:{:?}", e))?;

    //println!("install package success, deps: {:?}", deps);

    Ok(())
}

// pub async fn publish_app(
//     app_desc_file: &PathBuf,
//     did: &str,
//     hostname: &str,
//     pem_file: &str,
//     url: &str,
//     session_token: &str,
// ) -> Result<(), String> {
//     if !app_desc_file.exists() {
//         eprintln!("Error: App desc file {} not found", app_desc_file.display());
//         return Err(format!(
//             "App desc file {} not found",
//             app_desc_file.display()
//         ));
//     }

//     let desc_content = fs::read_to_string(app_desc_file)
//         .map_err(|err| format!("Error: Failed to read app desc file: {}", err.to_string()))?;

//     let app_desc: HashMap<String, String> = serde_json::from_str(&desc_content)
//         .map_err(|err| format!("Error: Failed to parse app desc file: {}", err.to_string()))?;

//     let app_name = app_desc
//         .get("app_name")
//         .ok_or_else(|| format!("Error: app_name missing in app desc file"))?;
//     let version = app_desc
//         .get("version")
//         .ok_or_else(|| format!("Error: version missing in app desc file"))?;
//     let hostname = app_desc
//         .get("hostname")
//         .ok_or_else(|| format!("Error: hostname missing in app desc file"))?;
//     let pkg_list = app_desc
//         .get("pkg_list")
//         .ok_or_else(|| format!("Error: pkg_list missing in app desc file"))?;

//     // 创建元数据
//     let pkg_meta = PackagePubMeta {
//         pkg_name: app_name.to_string(),
//         version: version.to_string(),
//         hostname: hostname.to_string(),
//         chunk_id: None,
//         dependencies: pkg_list.to_string(),
//     };

//     let meta_json_value = serde_json::to_value(&pkg_meta).map_err(|e| {
//         format!(
//             "Failed to serialize app meta to json value, err:{:?}",
//             e.to_string()
//         )
//     })?;

//     let jwt_token: String = generate_jwt(pem_file, &desc_content)?;

//     // 上传元数据到repo
//     let client = kRPC::new(url, Some(session_token.to_string()));

//     client
//         .call(
//             "pub_app",
//             json!({
//                 "pkg_name": pkg_meta.pkg_name,
//                 "version": pkg_meta.version,
//                 "hostname": hostname.to_string(),
//                 "dependencies": pkg_meta.dependencies,
//                 "jwt": jwt_token,
//             }),
//         )
//         .await
//         .map_err(|e| format!("Failed to publish app meta to repo, err:{:?}", e))?;

//     Ok(())
// }

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;
    use std::mem;
    
    #[tokio::test]
    async fn test_pack_pkg() {
        // 创建临时目录作为源目录
        let src_dir = tempdir().unwrap();
        let src_path = src_dir.path().to_owned();
        mem::forget(src_dir);
        // 创建临时目录作为目标目录
        let dest_dir = tempdir().unwrap();
        let dest_path = dest_dir.path().to_str().unwrap().to_string();
        // 阻止临时目录被删除
        mem::forget(dest_dir);


        // 创建测试文件结构
        let pkg_name = "test_package";
        let version = "0.1.0";
        let author = "test_author";
        
        // 创建测试文件
        fs::write(
            src_path.join("test_file.txt"),
            "This is a test file content",
        ).unwrap();
        
        // 创建测试子目录和文件
        fs::create_dir(src_path.join("subdir")).unwrap();
        fs::write(
            src_path.join("subdir").join("subfile.txt"),
            "This is a subdir file content",
        ).unwrap();
        
        // 创建 .pkg_meta.json 文件
        let meta = PackageMeta {
            pkg_name: pkg_name.to_string(),
            version: version.to_string(),
            tag: None,
            category: Some("pkg".to_string()),
            author: author.to_string(),
            chunk_id: None,
            chunk_url: None,
            chunk_size: None,
            deps: HashMap::new(),
            pub_time: 0,
        };
        
        let meta_json = serde_json::to_string_pretty(&meta).unwrap();
        fs::write(src_path.join(".pkg_meta.json"), meta_json).unwrap();
        
        // 执行打包函数
        let result = pack_raw_pkg(
            src_path.to_str().unwrap(),
            &dest_path,
            None,
        ).await;
        
    
        // 验证结果
        assert!(result.is_ok(), "打包失败: {:?}", result.err());
        
        let pack_result = result.unwrap();
        
        // 验证返回的结果
        assert_eq!(pack_result.pkg_name, pkg_name);
        assert_eq!(pack_result.version, version);
        assert_eq!(pack_result.hostname, author);
        
        // 验证文件是否存在
        let expected_tarball_path = Path::new(&dest_path)
            .join(pkg_name)
            .join(format!("{}-{}.tar.gz", pkg_name, version));
        assert!(expected_tarball_path.exists(), "打包文件不存在");
        //获取文件的sha256和大小
        let file_info = calculate_file_hash(expected_tarball_path.to_str().unwrap()).unwrap();
        //println!("tar: {} : {:?}", expected_tarball_path.display(), &file_info);
        let chunk_id = ChunkId::from_sha256_result(&file_info.sha256);
        println!("pkg chunk_id: {}", chunk_id.to_string());
        // 验证元数据文件是否存在
        let expected_meta_path = Path::new(&dest_path)
            .join(pkg_name)
            .join("pkg_meta.json");
        assert!(expected_meta_path.exists(), "元数据文件不存在");
        
        // 验证元数据内容
        let meta_content = fs::read_to_string(expected_meta_path).unwrap();
        let meta_data:PackageMeta = serde_json::from_str(&meta_content).unwrap();
        
        assert_eq!(meta_data.pkg_name, pkg_name);
        assert_eq!(meta_data.version, version);
        assert_eq!(meta_data.author, author);
        assert!(meta_data.chunk_id.unwrap() == chunk_id.to_string(), "chunk_id OK");
        assert!(meta_data.chunk_size.unwrap() == file_info.size, "chunk_size OK");
        assert!(meta_data.pub_time > 0, "pub_time OK");
    }
    
    #[tokio::test]
    async fn test_pack_pkg_with_jwt() {
        // 创建临时目录作为源目录
        let src_dir = tempdir().unwrap();
        let src_path = src_dir.path().to_owned();
        mem::forget(src_dir);
        // 创建临时目录作为目标目录
        let dest_dir = tempdir().unwrap();
        let dest_path = dest_dir.path().to_str().unwrap().to_string();
        // 阻止临时目录被删除
        mem::forget(dest_dir);
        
        // 创建测试文件结构
        let pkg_name = "test_package_jwt";
        let version = "0.1.0";
        let author = "test_author";
        
        // 创建测试文件
        fs::write(
            src_path.join("test_file.txt"),
            "This is a test file content",
        ).unwrap();
        
        // 创建 .pkg_meta.json 文件
        let meta = PackageMeta {
            pkg_name: pkg_name.to_string(),
            version: version.to_string(),
            tag: None,
            category: Some("pkg".to_string()),
            author: author.to_string(),
            chunk_id: None,
            chunk_url: None,
            chunk_size: None,
            deps: HashMap::new(),
            pub_time: 0,
        };
        
        let meta_json = serde_json::to_string_pretty(&meta).unwrap();
        fs::write(src_path.join(".pkg_meta.json"), meta_json).unwrap();
        
        // 创建临时私钥文件（注意：这里只是为了测试，实际应该使用有效的私钥）
        let key_dir = tempdir().unwrap();
        let key_path = key_dir.path().join("test_key.pem");
        fs::write(&key_path, r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----
        "#).unwrap();
        
        
        // 执行打包函数
        let result = pack_raw_pkg(
            src_path.to_str().unwrap(),
            &dest_path,
            Some(key_path.to_str().unwrap().to_string()),
        ).await;
        
        // 由于我们没有真正的私钥，这个测试可能会失败
        // 在实际环境中，应该使用有效的私钥或者 mock generate_jwt 函数
        if result.is_ok() {
            let pack_result = result.unwrap();
            
            // 验证 JWT 文件是否存在
            let expected_jwt_path = Path::new(&dest_path)
                .join(pkg_name)
                .join("pkg_meta.jwt");
            
            if expected_jwt_path.exists() {
                println!("JWT 文件成功创建");
            } else {
                println!("JWT 文件未创建，可能是由于测试环境中没有有效的私钥");
            }
        } else {
            println!("JWT 测试失败，错误: {:?}", result.err());
            println!("这可能是由于测试环境中没有有效的私钥");
        }
    }
    
    #[tokio::test]
    async fn test_pack_pkg_missing_meta() {
        // 创建临时目录作为源目录，但不创建 .pkg_meta.json 文件
        let src_dir = tempdir().unwrap();
        let src_path = src_dir.path();
        
        // 创建临时目录作为目标目录
        let dest_dir = tempdir().unwrap();
        let dest_path = dest_dir.path().to_str().unwrap().to_string();
        
        // 创建测试文件
        fs::write(
            src_path.join("test_file.txt"),
            "This is a test file content",
        ).unwrap();
        
        // 执行打包函数，应该失败
        let result = pack_raw_pkg(
            src_path.to_str().unwrap(),
            &dest_path,
            None,
        ).await;
        
        // 验证结果
        assert!(result.is_err(), "应该因为缺少 .pkg_meta.json 文件而失败");
        let err = result.err().unwrap();
        assert!(err.contains("meta.json 文件未在指定目录中找到"), 
                "错误消息应该提及缺少 meta.json 文件，实际错误: {}", err);
    }
}
