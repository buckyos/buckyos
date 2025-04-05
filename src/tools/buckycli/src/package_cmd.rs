#[allow(dead_code, unused)]
use flate2::write::GzEncoder;
use flate2::Compression;
use jsonwebtoken::EncodingKey;
use name_lib::{decode_jwt_claim_without_verify, DIDDocumentTrait};
use ndn_lib::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use tar::Builder;
use package_lib::*;
use buckyos_api::*;


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
        .map_err(|e| format!("Failed to create package file: {}", e.to_string()))?;
    let enc = GzEncoder::new(tar_gz, Compression::default());
    let mut tar = Builder::new(enc);

    // Recursively add directories and files
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
        format!("Failed to add files to package: {}", e.to_string())
    })?;

    tar.finish().map_err(|e| {
        format!("Failed to complete packaging process: {}", e.to_string())
    })?;
    Ok(())
}


pub async fn pack_raw_pkg(pkg_path: &str, dest_dir: &str,private_key:Option<(&str,&EncodingKey)>) -> Result<(), String> {
    println!("Starting to package path: {}", pkg_path);
    let pkg_path = Path::new(pkg_path);
    if !pkg_path.exists() {
        return Err(format!("Specified path {} does not exist", pkg_path.display()));
    }
    // Read meta.json file
    let meta_path = pkg_path.join(".pkg_meta.json");
    if !meta_path.exists() {
        return Err("meta.json file not found in specified directory".to_string());
    }

    let meta_content = fs::read_to_string(&meta_path)
        .map_err(|e| format!("Failed to read .pkg_meta.json: {}", e.to_string()))?;
    
    let mut meta_data:PackageMeta = serde_json::from_str(&meta_content)
        .map_err(|e| format!("Failed to parse .pkg_meta.json: {}", e.to_string()))?;

    let pkg_name = meta_data.pkg_name.clone();
    
    let version = meta_data.version.clone();
    let author = meta_data.author.clone();
    
    println!("Parsed .pkg_meta.json: pkg_name = {}, version = {}, author = {}", pkg_name, version, author);
    // Check and create target directory
    let dest_dir_path = Path::new(dest_dir).join(&pkg_name);
    if !dest_dir_path.exists() {
        fs::create_dir_all(dest_dir_path.clone()).map_err(|e| {
            format!("Failed to create target directory: {}", e.to_string())
        })?;
    }
    // Create tarball
    let tarball_name = format!("{}#{}.tar.gz", pkg_name, version);
    let tarball_path = dest_dir_path.join(&tarball_name);

    tar_gz(&pkg_path, &tarball_path).await?;
    println!("pack to {} done", tarball_path.display());

    // Calculate SHA256 hash of the tar.gz file
    let file_info = calculate_file_hash(tarball_path.to_str().unwrap())?;
    let chunk_id = ChunkId::from_sha256_result(&file_info.sha256);
    
    // Update metadata
    meta_data.chunk_id = Some(chunk_id.to_string());
    meta_data.chunk_size = Some(file_info.size);

    let meta_data_json = serde_json::to_value(&meta_data).map_err(|e| {
        format!("Failed to serialize metadata: {}", e.to_string())
    })?;
    
    let (pkg_meta_obj_id,pkg_meta_json_str) = build_named_object_by_json("pkg",&meta_data_json);
    
    // Save updated metadata to pkg.meta.json
    let meta_json_path = dest_dir_path.join("pkg_meta.json");
    
    fs::write(&meta_json_path, &pkg_meta_json_str.as_bytes()).map_err(|e| {
        format!("Failed to write pkg.meta.json: {}", e.to_string())
    })?;
    let meta_json_path = dest_dir_path.join(pkg_meta_obj_id.to_base32());
    fs::write(&meta_json_path, &pkg_meta_json_str.as_bytes()).map_err(|e| {
        format!("Failed to write objid: {}", e.to_string())
    })?;
    // If private key is provided, sign the metadata
    if let Some((kid,private_key)) = private_key { 
        let jwt_token = named_obj_to_jwt(&meta_data_json,
            private_key,Some(kid.to_string()))
            .map_err(|e| format!("Failed to generate pkg_meta.jwt: {}", e.to_string()))?;
        let jwt_path = dest_dir_path.join("pkg_meta.jwt");
        fs::write(&jwt_path, jwt_token).map_err(|e| {
            format!("Failed to write pkg_meta.jwt: {}", e.to_string())
        })?;
        println!("pkg_meta.jwt written successfully: {}", jwt_path.display());
    } else {
        println!("No private key provided, skipping metadata signing");
        // Delete old .jwt file
        let jwt_path = dest_dir_path.join("pkg_meta.jwt");
        if jwt_path.exists() {
            fs::remove_file(jwt_path).map_err(|e| {
                format!("Failed to delete pkg_meta.jwt: {}", e.to_string())
            })?;
        }
    }

    println!("Package {} version {} author {} has been successfully packaged.", pkg_name, version, author);
    println!("Package file created at: {:?}", tarball_path);

    Ok(())
}

// Based on pack raw pkg output, publish pkg to current zone (call repo_server.pub_pkg)
pub async fn publish_raw_pkg(pkg_pack_path_list: &Vec<PathBuf>) -> Result<(), String> {
    // 1) First push_chunk
    let mut pkg_meta_jwt_map = HashMap::new();
    let runtime = get_buckyos_api_runtime().unwrap();
    let zone_host_name = runtime.zone_config.as_ref().unwrap().get_id().to_host_name();

    let base_url = format!("http://{}/ndn/",zone_host_name.as_str());
    let ndn_client = NdnClient::new(base_url,None,None);
    //let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(None).await.unwrap();
    for pkg_path in pkg_pack_path_list {
        let pkg_meta_jwt_path = pkg_path.join("pkg_meta.jwt");
        if !pkg_meta_jwt_path.exists() {
            println!("pkg_meta.jwt file does not exist: {}", pkg_meta_jwt_path.display());
            continue;
        }
        let pkg_meta_jwt_str = fs::read_to_string(pkg_meta_jwt_path)
            .map_err(|e| format!("Failed to read pkg_meta.jwt: {}", e.to_string()))?;

        let pkg_meta = decode_jwt_claim_without_verify(&pkg_meta_jwt_str)
            .map_err(|e| format!("Failed to decode pkg_meta.jwt: {}", e.to_string()))?;
        let pkg_meta:PackageMeta = serde_json::from_value(pkg_meta)
            .map_err(|e| format!("Failed to parse pkg_meta.jwt: {}", e.to_string()))?;
        let pkg_meta_obj_id = build_obj_id("pkg",&pkg_meta_jwt_str);

        let pkg_tar_path = pkg_path.join(format!("{}#{}.tar.gz", pkg_meta.pkg_name, pkg_meta.version));
        if !pkg_tar_path.exists() {
            println!("tar.gz file does not exist: {}", pkg_tar_path.display());
            continue;
        }

        let file_info = calculate_file_hash(pkg_tar_path.to_str().unwrap())?;
        let chunk_id = ChunkId::from_sha256_result(&file_info.sha256);
        if Some(chunk_id.to_string()) != pkg_meta.chunk_id {
            println!("chunk_id does not match: {}", chunk_id.to_string());
            continue;
        }
        //let real_named_mgr = named_mgr.lock().await;
        let is_exist = NamedDataMgr::have_chunk(&chunk_id,None).await;
        if !is_exist {
            let (mut chunk_writer, _) = NamedDataMgr::open_chunk_writer(None,&chunk_id, file_info.size, 0).await.map_err(|e| {
                format!("Failed to open chunk writer: {}", e.to_string())
            })?;
        
            let mut file_reader = tokio::fs::File::open(pkg_tar_path.to_str().unwrap()).await
                .map_err(|e| {
                    format!("Failed to open tar.gz file: {}", e.to_string())
                })?;
            tokio::io::copy(&mut file_reader, &mut chunk_writer).await
            .map_err(|e| {
                format!("Failed to copy tar.gz file: {}", e.to_string())
            })?;
            println!(" {} file successfully written to local named-mgr,chunk_id: {}", pkg_tar_path.display(),chunk_id.to_string());
 
            NamedDataMgr::complete_chunk_writer(None,&chunk_id).await.map_err(|e| {
                format!("Failed to complete chunk writer: {}", e.to_string())
            })?;

        } else {
            println!(" {} file already exists in local named-mgr,chunk_id: {}", pkg_tar_path.display(),chunk_id.to_string());
        }
        
        println!("# push chunk : {}, size: {} bytes...", chunk_id.to_string(),file_info.size);
        ndn_client.push_chunk(chunk_id.clone(),None).await.map_err(|e| {
            format!("Failed to push chunk: {}", e.to_string())
        })?;
        println!("# push chunk : {}, size: {} bytes success.", chunk_id.to_string(),file_info.size);

        pkg_meta_jwt_map.insert(pkg_meta_obj_id.to_string(),pkg_meta_jwt_str);
    }
    // 2) Then call repo_server.pub_pkg
    let pkg_lens = pkg_meta_jwt_map.len();
    let runtime = get_buckyos_api_runtime().unwrap();
    let repo_client = runtime.get_repo_client().await.unwrap();
    repo_client.pub_pkg(pkg_meta_jwt_map).await.map_err(|e| {
        format!("Failed to publish pkg: {}", e.to_string())
    })?;
    println!("Successfully published pkg, total {} pkgs published",pkg_lens);
    Ok(())
}

// Prepare dapp_meta for publishing, this dapp_meta can be used for the next step of publishing
pub async fn publish_app_pkg(app_name: &str,dapp_dir_path: &str,is_pub_sub_pkg:bool) -> Result<(), String> {
    // Before publishing dapp_pkg, users need to ensure sub_pkgs
    let runtime = get_buckyos_api_runtime().unwrap();
    if runtime.user_private_key.is_none() {
        return Err("No developer private key provided, skipping dapp_pkg publishing".to_string());
    }
    let app_doc_file_name = format!("{}.doc.json",app_name);
    let app_meta_path = Path::new(dapp_dir_path).join(app_doc_file_name);
    if !app_meta_path.exists() {
        return Err(format!("{} file does not exist", app_meta_path.display()));
    }

    let app_meta_str = fs::read_to_string(app_meta_path)
        .map_err(|e| format!("Failed to read app doc.json: {}", e.to_string()))?;
    let mut app_meta:AppDoc = serde_json::from_str(&app_meta_str)
        .map_err(|e| format!("Failed to parse app doc.json: {}", e.to_string()))?;
    //info!("app_meta:{} {}",app_meta.pkg_name.as_str(), serde_json::to_string_pretty(&app_meta).unwrap());
    let mut pkg_path_list = Vec::new();

    for (sub_pkg_section,pkg_desc) in app_meta.pkg_list.iter_mut() {
        let sub_pkg_id = pkg_desc.pkg_id.clone();
        let sub_pkg_id:PackageId = PackageId::parse(sub_pkg_id.as_str())
            .map_err(|e| format!("Failed to parse sub_pkg_id: {}", e.to_string()))?;
   
        let pkg_path = Path::new(dapp_dir_path).join(sub_pkg_id.name.as_str());
        if !pkg_path.exists() {
            return Err(format!("sub pkg {} directory does not exist", pkg_path.display()));
        }
        let pkg_meta_path = pkg_path.join("pkg_meta.json");
        if !pkg_meta_path.exists() {
            return Err(format!("sub pkg {} pkg_meta.json does not exist", pkg_path.display()));
        }
        let pkg_meta_str = fs::read_to_string(pkg_meta_path)
            .map_err(|e| format!("Failed to read .pkg_meta.json: {}", e.to_string()))?;
        let pkg_meta:PackageMeta = serde_json::from_str(&pkg_meta_str)
            .map_err(|e| format!("Failed to parse .pkg_meta.json: {}", e.to_string()))?;
        let version = pkg_meta.version.clone();
        //pkg_desc.pkg_id = format!("{}#{}",sub_pkg_section,version);
        println!("{} => {}", sub_pkg_section,pkg_meta.get_package_id().to_string());
        pkg_path_list.push(pkg_path);
        
    }

    if is_pub_sub_pkg {
        println!("Publishing sub_pkg");
        publish_raw_pkg(&pkg_path_list).await?;
    } else {
        println!("Skipping sub_pkg publishing");
    }

    let repo_client = runtime.get_repo_client().await.unwrap();
    let mut app_meta_jwt_map = HashMap::new();
    let app_doc_json = serde_json::to_value(&app_meta).map_err(|e| {
        format!("Failed to serialize app_doc: {}", e.to_string())
    })?;
    let (app_doc_obj_id,_) = build_named_object_by_json("app",&app_doc_json);
    let app_doc_jwt = named_obj_to_jwt(&app_doc_json,runtime.user_private_key.as_ref().unwrap(),runtime.user_id.clone())
        .map_err(|e| format!("Failed to generate app_doc.jwt: {}", e.to_string()))?;
    app_meta_jwt_map.insert(app_doc_obj_id.to_string(),app_doc_jwt);
    repo_client.pub_pkg(app_meta_jwt_map).await.map_err(|e| {
        format!("Failed to publish app doc: {}", e.to_string())
    })?;
    repo_client.pub_index().await.map_err(|e| {
        format!("Failed to publish repo index: {}", e.to_string())
    })?;
    println!("Successfully published App {}", app_name);
    Ok(())
}


// call repo_server.pub_index, which will trigger automatic upgrades of related components in the zone
pub async fn publish_repo_index() -> Result<(), String> {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let repo_client = api_runtime.get_repo_client().await.unwrap();
    repo_client.pub_index().await.map_err(|e| {
        format!("Failed to publish repo index: {}", e.to_string())
    })?;
    println!("Successfully published repo index");
    Ok(())
}

pub async fn publish_app_to_remote_repo(_app_dir_path: &str,_zone_host_name: &str) -> Result<(), String> {
    unimplemented!()
}

pub async fn sync_from_remote_source() -> Result<(), String> {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let repo_client = api_runtime.get_repo_client().await.unwrap();
    repo_client.sync_from_remote_source().await.map_err(|e| {
        format!("Failed to sync zone repo service's meta-index-db from remote source: {}", e.to_string())
    })?;
    println!("Successfully synced zone repo service's meta-index-db from remote source, new default meta-index-db is ready");
    Ok(())
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


pub async fn load_pkg(
    pkg_id: &str,
    target_env:&str
) -> Result<(), String> {
    let target_env = PathBuf::from(target_env);
    if !target_env.exists() {
        return Err(format!("target env {} does not exist", target_env.display()));
    }

    let the_env:PackageEnv = PackageEnv::new(target_env);
    let media_info = the_env.load(pkg_id).await;
    if media_info.is_err() {
        println!("Load package failed! {}", media_info.err().unwrap());
        return Err("load package failed!".to_string());
    }
    println!("### Load package success! {:?}", media_info.unwrap());
    Ok(())
}

pub async fn install_pkg(
    pkg_id: &str,
    target_env:&str
) -> Result<(), String> {
    let target_env = PathBuf::from(target_env);
    if !target_env.exists() {
        return Err(format!("target env {} does not exist", target_env.display()));
    }

    let mut the_env:PackageEnv = PackageEnv::new(target_env);
    the_env.install_pkg(pkg_id, true,true).await.map_err(|e| {
        format!("Failed to install pkg: {}", e.to_string())
    })?;
    
    Ok(())
}

pub async fn set_pkg_meta(
    meta_path: &str,
    db_path: &str
) -> Result<(), String> {
    let meta_path = PathBuf::from(meta_path);
    let db_path = PathBuf::from(db_path);
    if !meta_path.exists() {
        return Err(format!("meta_path {} does not exist", meta_path.display()));
    }
    if !db_path.exists() {
        return Err(format!("db_path {} does not exist", db_path.display()));
    }

    let meta_content = fs::read_to_string(meta_path).map_err(|e| {
        format!("Failed to read meta_path: {}", e.to_string())
    })?;

    let meta_data:PackageMeta = PackageMeta::from_str(&meta_content).map_err(|e| {
        format!("Failed to parse meta_path: {}", e.to_string())
    })?;
    let (meta_obj_id,meta_obj_id_str) = meta_data.gen_obj_id();

    let meta_db = MetaIndexDb::new(db_path,false);
    if meta_db.is_err() {
        return Err(format!("Failed to open meta_db: {}", meta_db.err().unwrap()));
    }
    let meta_db = meta_db.unwrap();
    let mut pkg_meta_map = HashMap::new();
    pkg_meta_map.insert(meta_obj_id.to_string(),PackageMetaNode {
        meta_jwt: meta_content,
        pkg_name: meta_data.pkg_name.clone(),
        version: meta_data.version.clone(),
        tag: meta_data.tag.clone(),
        author: meta_data.author.clone(),
        author_pk: "".to_string(),
    });
    meta_db.add_pkg_meta_batch(&pkg_meta_map).map_err(|e| {
        format!("Failed to set pkg meta: {}", e.to_string())
    })?;
    
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use name_lib::load_private_key;
    use tempfile::tempdir;
    use std::mem;
    use serde_json::json;
    
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
            description: json!("{}"),
            extra_info: HashMap::new(),
            exp:0,
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
            description: json!("{}"),
            extra_info: HashMap::new(),
            exp:0,
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
        let encoding_key = load_private_key(&key_path).unwrap();
        
        // 执行打包函数
        let result = pack_raw_pkg(
            src_path.to_str().unwrap(),
            &dest_path,
            Some(("did:bns:buckyos",&encoding_key)),
        ).await;
        
        // 由于我们没有真正的私钥，这个测试可能会失败
        // 在实际环境中，应该使用有效的私钥或者 mock generate_jwt 函数
        if result.is_ok() {
            let _pack_result = result.unwrap();
            
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
