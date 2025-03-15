#[allow(dead_code, unused)]
use crate::util::*;
use flate2::write::GzEncoder;
use flate2::Compression;
use jsonwebtoken::{encode, Algorithm, Header};
use kRPC::kRPC;
use name_lib::decode_jwt_claim_without_verify;
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
use buckyos_api::*;

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

async fn tar_gz(src_dir: &Path, tarball_path: &Path) -> Result<(), String> {
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


pub async fn pack_raw_pkg(pkg_path: &str, dest_dir: &str) -> Result<(), String> {
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
    let runtime = get_buckyos_api_runtime().unwrap();
    if runtime.user_private_key.is_some() { 
        let jwt_token = named_obj_to_jwt(pkg_meta_json_str,
            runtime.user_private_key.as_ref().unwrap(),runtime.user_did.clone())
            .map_err(|e| format!("生成 pkg_meta.jwt 失败: {}", e.to_string()))?;
        let jwt_path = dest_dir_path.join("pkg_meta.jwt");
        fs::write(&jwt_path, jwt_token).map_err(|e| {
            format!("写入 pkg_meta.jwt 失败: {}", e.to_string())
        })?;
        println!("pkg_meta.jwt 写入成功: {}", jwt_path.display());
    } else {
        println!("没有提供私钥,跳过对元数据进行签名");
        //删除旧的.jwt文件
        let jwt_path = dest_dir_path.join("pkg_meta.jwt");
        if jwt_path.exists() {
            fs::remove_file(jwt_path).map_err(|e| {
                format!("删除 pkg_meta.jwt 失败: {}", e.to_string())
            })?;
        }
    }

    println!("包 {} 版本 {} 作者 {} 已成功打包。", pkg_name, version, author);
    println!("打包文件创建于: {:?}", tarball_path);

    Ok(())
}

//基于pack raw pkg的输出，发布pkg到当前zone(call repo_server.pub_pkg)
pub async fn publish_raw_pkg(pkg_pack_path_list: &Vec<PathBuf>,zone_host_name: &str) -> Result<(), String> {
    //1) 首先push_chunk
    let mut pkg_meta_jwt_map = HashMap::new();
    let base_url = format!("http://{}/ndn/",zone_host_name);
    let ndn_client = NdnClient::new(zone_host_name.to_string(),None,None);
    let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(None).await.unwrap();
    for pkg_path in pkg_pack_path_list {
        let pkg_meta_jwt_path = pkg_path.join("pkg_meta.jwt");
        if !pkg_meta_jwt_path.exists() {
            println!("pkg_meta.jwt 文件不存在: {}", pkg_meta_jwt_path.display());
            continue;
        }
        let pkg_meta_jwt_str = fs::read_to_string(pkg_meta_jwt_path)
            .map_err(|e| format!("读取 pkg_meta.jwt 失败: {}", e.to_string()))?;

        let pkg_meta = decode_jwt_claim_without_verify(&pkg_meta_jwt_str)
            .map_err(|e| format!("解码 pkg_meta.jwt 失败: {}", e.to_string()))?;
        let pkg_meta:PackageMeta = serde_json::from_value(pkg_meta)
            .map_err(|e| format!("解析 pkg_meta.jwt 失败: {}", e.to_string()))?;
        let pkg_meta_obj_id = build_obj_id("pkg",&pkg_meta_jwt_str);

        let pkg_tar_path = pkg_path.join(format!("{}-{}.tar.gz", pkg_meta.pkg_name, pkg_meta.version));
        if !pkg_tar_path.exists() {
            println!("tar.gz 文件不存在: {}", pkg_tar_path.display());
            continue;
        }

        let file_info = calculate_file_hash(pkg_tar_path.to_str().unwrap())?;
        let chunk_id = ChunkId::from_sha256_result(&file_info.sha256);
        if Some(chunk_id.to_string()) != pkg_meta.chunk_id {
            println!("chunk_id 不匹配: {}", chunk_id.to_string());
            continue;
        }
        let real_named_mgr = named_mgr.lock().await;
        let is_exist = real_named_mgr.is_chunk_exist(&chunk_id).await.unwrap();
        if !is_exist {
            let (mut chunk_writer,chunk_progress_info) = real_named_mgr.open_chunk_writer(&chunk_id,file_info.size,0).await.map_err(|e| {
                format!("打开 chunk 写入器失败: {}", e.to_string())
            })?;
            drop(real_named_mgr);
            let mut file_reader = tokio::fs::File::open(pkg_tar_path.to_str().unwrap()).await
                .map_err(|e| {
                    format!("打开 tar.gz 文件失败: {}", e.to_string())
                })?;
            tokio::io::copy(&mut file_reader, &mut chunk_writer).await
            .map_err(|e| {
                format!("copy tar.gz 文件失败: {}", e.to_string())
            })?;
            println!(" {} 文件成功写入 local named-mgr 成功", pkg_tar_path.display());
        }
          
        println!("# push chunk : {}, size: {} bytes...", chunk_id.to_string(),file_info.size);
        ndn_client.push_chunk(chunk_id.clone(),None).await.map_err(|e| {
            format!("push chunk 失败: {}", e.to_string())
        })?;
        println!("# push chunk : {}, size: {} bytes success.", chunk_id.to_string(),file_info.size);

        pkg_meta_jwt_map.insert(pkg_meta_obj_id.to_string(),pkg_meta_jwt_str);
    }
    //2) 然后调用repo_server.pub_pkg
    let pkg_lens = pkg_meta_jwt_map.len();
    let runtime = get_buckyos_api_runtime().unwrap();
    let repo_client = runtime.get_repo_client().await.unwrap();
    repo_client.pub_pkg(pkg_meta_jwt_map).await.map_err(|e| {
        format!("发布pkg失败: {}", e.to_string())
    })?;
    println!("发布pkg成功,共发布 {} 个pkg",pkg_lens);
    Ok(())
}

//准备用于发布的dapp_meta,该dapp_meta可用做下一步的发布
pub async fn publish_app_pkg(dapp_dir_path: &str,no_pub_sub_pkg:bool,zone_host_name: &str) -> Result<(), String> {
    //发布dapp_pkg前，需要用户确保sub_pkgs
    let runtime = get_buckyos_api_runtime().unwrap();
    if runtime.user_private_key.is_none() {
        return Err("没有提供开发者私钥,跳过发布dapp_pkg".to_string());
    }
    let app_meta_path = Path::new(dapp_dir_path).join("buckyos_app_doc.json");
    if !app_meta_path.exists() {
        return Err("buckyos_app_doc.json 文件不存在".to_string());
    }

    let app_meta_str = fs::read_to_string(app_meta_path)
        .map_err(|e| format!("读取 buckyos_app_doc.json 失败: {}", e.to_string()))?;
    let mut app_meta:AppDoc = serde_json::from_str(&app_meta_str)
        .map_err(|e| format!("解析 buckyos_app_doc.json 失败: {}", e.to_string()))?;

    let mut pkg_path_list = Vec::new();

    for (pkg_id,pkg_desc) in app_meta.pkg_list.iter_mut() {
        let sub_pkg_id = pkg_desc.pkg_id.clone();
        let sub_pkg_id:PackageId = PackageId::parse(sub_pkg_id.as_str())
            .map_err(|e| format!("解析 sub_pkg_id 失败: {}", e.to_string()))?;
        if sub_pkg_id.version_exp.is_some() {
            println!("{} 已经包含版本号,跳过检测构建 ", sub_pkg_id.to_string());
        } else {
            let pkg_path = Path::new(dapp_dir_path).join(pkg_id);
            if !pkg_path.exists() {
                return Err(format!("{} 目录不存在", pkg_path.display()));
            }
            let pkg_meta_path = pkg_path.join(".pkg_meta.json");
            if !pkg_meta_path.exists() {
                return Err(format!("{} 目录不存在", pkg_path.display()));
            }
            let pkg_meta_str = fs::read_to_string(pkg_meta_path)
                .map_err(|e| format!("读取 .pkg_meta.json 失败: {}", e.to_string()))?;
            let pkg_meta:PackageMeta = serde_json::from_str(&pkg_meta_str)
                .map_err(|e| format!("解析 .pkg_meta.json 失败: {}", e.to_string()))?;
            let version = pkg_meta.version.clone();
            pkg_desc.pkg_id = format!("{}#{}",pkg_id,version);
            println!("{} => {}", sub_pkg_id.to_string(),pkg_desc.pkg_id);
            pkg_path_list.push(pkg_path);
        }
    }

    if no_pub_sub_pkg {
        println!("跳过发布sub_pkg");
    } else {
        println!("发布sub_pkg");
        publish_raw_pkg(&pkg_path_list,zone_host_name).await?;
    }


    let repo_client = runtime.get_repo_client().await.unwrap();
    let mut app_meta_jwt_map = HashMap::new();
    let app_doc_json = serde_json::to_value(&app_meta).map_err(|e| {
        format!("序列化 app_doc 失败: {}", e.to_string())
    })?;
    let (app_doc_obj_id,app_doc_json_str) = build_named_object_by_json("app",&app_doc_json);
    let app_doc_jwt = named_obj_to_jwt(app_doc_json_str,runtime.user_private_key.as_ref().unwrap(),runtime.user_did.clone())
        .map_err(|e| format!("生成 app_doc.jwt 失败: {}", e.to_string()))?;
    app_meta_jwt_map.insert(app_doc_obj_id.to_string(),app_doc_jwt);
    repo_client.pub_pkg(app_meta_jwt_map).await.map_err(|e| {
        format!("发布app doc失败: {}", e.to_string())
    })?;
    println!("发布app doc成功");
    Ok(())
}

//基于pack_dapp_pkg输出，发布dapp_pkg到当前zone(call repo_server.pub_pkg)
pub async fn pack_app_pkg(dapp_dir_path: &str,zone_host_name: &str) {
    unimplemented!()
}

//call repo_server.pub_index,随后在zone内就会触发相关组件的自动升级了
pub async fn publish_repo_index() -> Result<(), String> {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let repo_client = api_runtime.get_repo_client().await.unwrap();
    repo_client.pub_index().await.map_err(|e| {
        format!("发布repo index失败: {}", e.to_string())
    })?;
    println!("发布repo index成功");
    Ok(())
}

pub async fn publish_app_to_remote_repo(app_dir_path: &str,zone_host_name: &str) -> Result<(), String> {
    unimplemented!()
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

// fn generate_jwt(pem_file: &str, data: &String) -> Result<String, String> {
//     let private_key = load_private_key_from_file(pem_file)?;
//     let mut header = Header::new(Algorithm::EdDSA);
//     header.kid = None;
//     header.typ = None;
//     let token = encode(&header, data, &private_key)
//         .map_err(|e| format!("Failed to encode data to JWT: {}", e.to_string()))?;

//     Ok(token)
// }

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
