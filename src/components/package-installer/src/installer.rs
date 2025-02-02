use buckyos_kit::get_buckyos_system_etc_dir;
use jsonwebtoken::EncodingKey;
//use kRPC::kRPC;
use flate2::read::GzDecoder;
use log::*;
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig};
use ndn_lib::*;
use package_lib::*;
use repo_service::*;
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tar::Archive;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct Installer;

#[derive(Debug)]
pub struct InstallResult {
    pub pkg_id: String,
    pub target: PathBuf,
    pub result: Result<(), String>,
}

#[derive(Deserialize, Debug)]
struct InstallRequestResult {
    task_id: String,
}

#[derive(Deserialize, Debug)]
struct NodeIdentityConfig {
    zone_name: String,                        // $name.buckyos.org or did:ens:$name
    owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner
    owner_name: String,                       //owner's name
    device_doc_jwt: String,                   //device document,jwt string,siged by owner
    zone_nonce: String,                       // random string, is default password of some service
                                              //device_private_key: ,storage in partical file
}

impl Installer {
    //将pkg下载到对应的目录，接收一个回调函数，用来返回下载结果
    pub async fn install(
        pkg_id_str: &str,
        target: &PathBuf,
        local_repo_url: &str,
        named_mgr_id: Option<&str>,
        callback: Option<Box<dyn FnOnce(InstallResult) + Send>>,
    ) -> Result<(), String> {
        let env = PackageEnv::new(target.clone());
        if env.is_pkg_ready(pkg_id_str).is_ok() {
            info!("package {} is already installed", pkg_id_str);
            if let Some(callback) = callback {
                callback(InstallResult {
                    pkg_id: pkg_id_str.to_string(),
                    target: target.clone(),
                    result: Ok(()),
                });
            }
            return Ok(());
        }

        //从repo中下载pkg
        let session_token = Self::gen_rpc_session_token().map_err(|err| {
            error!("generate rpc session token failed! {}", err);
            format!("generate rpc session token failed! {}", err)
        })?;

        let pkg_id = PackageId::from_str(pkg_id_str).map_err(|err| {
            error!("parse pkg id failed! {}", err);
            format!("parse pkg id failed! {}", err)
        })?;
        let client = kRPC::kRPC::new(local_repo_url, Some(session_token));
        let version = match pkg_id.sha256 {
            Some(sha256) => sha256,
            None => match pkg_id.version {
                Some(version) => version,
                None => "*".to_string(),
            },
        };
        let result = client
            .call(
                "install_pkg",
                json!({
                    "pkg_name": pkg_id.name,
                    "version": version
                }),
            )
            .await
            .map_err(|err| {
                error!("call install_pkg failed! {}", err);
                format!("call install_pkg failed! {}", err)
            })?;

        let install_task_result: InstallRequestResult =
            serde_json::from_value(result).map_err(|e| {
                error!(
                    "Failed to deserialize install request result from json_value, error:{:?}",
                    e
                );
                format!(
                    "Failed to deserialize install request result from json_value, error:{:?}",
                    e
                )
            })?;

        let task_id = install_task_result.task_id;

        //轮询task_id，直到下载完成，间隔2s
        loop {
            let result = client
                .call(
                    "query_task",
                    json!({
                        "task_id": task_id
                    }),
                )
                .await
                .map_err(|err| {
                    error!("call query_task failed! {}", err);
                    format!("call query_task failed! {}", err)
                })?;

            let task: Task = serde_json::from_value(result).map_err(|e| {
                error!(
                    "Failed to deserialize task status from json_value, error:{:?}",
                    e
                );
                format!(
                    "Failed to deserialize task status from json_value, error:{:?}",
                    e
                )
            })?;

            match task {
                Task::InstallTask {
                    id,
                    package_id,
                    status,
                    deps,
                    ..
                } => match status {
                    TaskStatus::Finished => {
                        info!(
                            "task {} finished, package_id:{:?}, deps:{:?}",
                            id, package_id, deps
                        );
                        match Self::install_pkgs_from_repo(&package_id, &deps, &env, named_mgr_id)
                            .await
                        {
                            Ok(_) => {
                                info!("install package {:?} success", package_id);
                            }
                            Err(e) => {
                                error!("install package {:?} failed! {}", package_id, e);
                                if let Some(callback) = callback {
                                    callback(InstallResult {
                                        pkg_id: pkg_id_str.to_string(),
                                        target: target.clone(),
                                        result: Err(e.clone()),
                                    });
                                }
                                return Err(e);
                            }
                        }
                        if let Some(callback) = callback {
                            callback(InstallResult {
                                pkg_id: pkg_id_str.to_string(),
                                target: target.clone(),
                                result: Ok(()),
                            });
                        }
                        break;
                    }
                    TaskStatus::Error(reason) => {
                        let err_msg = format!("task {} failed! {}", id, reason);
                        error!("{}", err_msg);
                        if let Some(callback) = callback {
                            callback(InstallResult {
                                pkg_id: pkg_id_str.to_string(),
                                target: target.clone(),
                                result: Err(err_msg.clone()),
                            });
                        }
                        return Err(err_msg);
                    }
                    v => {
                        info!("task {} is {:?}", id, v);
                    }
                },
                _ => {
                    let err_msg = format!("task {} is not an InstallTask", task_id);
                    warn!("{}", err_msg);
                    //调用回调函数
                    if let Some(callback) = callback {
                        callback(InstallResult {
                            pkg_id: pkg_id_str.to_string(),
                            target: target.clone(),
                            result: Err(err_msg.clone()),
                        });
                    }
                    return Err(err_msg);
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        Ok(())
    }

    async fn install_pkgs_from_repo(
        package_id: &PackageId,
        pkgs: &Vec<PackageMeta>,
        env: &PackageEnv,
        named_mgr_id: Option<&str>,
    ) -> Result<(), String> {
        //将pkgs依次下载到env的cache目录，下载时用一个临时文件名，下载完成后重命名为chunk_id，以防止下载失败时不完整文件
        //下载完成后，解压到env的install目录，并将pkg的meta信息写入env的meta目录
        for pkg in pkgs {
            let chunk_id = &pkg.chunk_id;
            let fix_chunk_id = chunk_id.replace(":", "-");
            let dest_file_tmp = env.get_cache_dir().join(format!("{}.tmp", fix_chunk_id));
            match Self::chunk_to_local_file(&chunk_id, named_mgr_id, &dest_file_tmp).await {
                Ok(_) => {
                    info!(
                        "chunk {} to local file {} success",
                        chunk_id,
                        dest_file_tmp.display()
                    );
                }
                Err(e) => {
                    error!(
                        "chunk {} to local file {} failed! {}",
                        chunk_id,
                        dest_file_tmp.display(),
                        e
                    );
                    return Err(e.to_string());
                }
            }

            //重命名文件
            let dest_file = env.get_cache_dir().join(fix_chunk_id);
            tokio::fs::rename(dest_file_tmp.clone(), dest_file.clone())
                .await
                .map_err(|e| {
                    let err_msg = format!(
                        "rename file {} to {} failed! {}",
                        dest_file_tmp.display(),
                        dest_file.display(),
                        e
                    );
                    error!("{}", err_msg);
                    err_msg
                })?;

            //解压文件
            let dest_dir = env
                .get_install_dir()
                .join(format!("{}#{}", pkg.pkg_name, pkg.version));
            match Self::unpack(&dest_file, &dest_dir) {
                Ok(_) => {
                    info!(
                        "unpack {} to {} success",
                        dest_file.display(),
                        dest_dir.display()
                    );
                }
                Err(e) => {
                    error!(
                        "unpack {} to {} failed! {}",
                        dest_file.display(),
                        dest_dir.display(),
                        e
                    );
                    return Err(e.to_string());
                }
            }

            //将pkg的meta信息写入env的meta目录
            let full_pkg_id = format!("{}#{}#{}", pkg.pkg_name, pkg.version, pkg.chunk_id);
            let full_pkg_id = PackageId::from_str(&full_pkg_id).map_err(|e| {
                error!("parse pkg id failed! {}", e);
                format!("parse pkg id failed! {}", e)
            })?;
            //转换dependencies(Value)为HashMap<String, String>
            let mut dependencies = HashMap::new();
            match &pkg.dependencies {
                Value::Object(map) => {
                    for (k, v) in map.iter() {
                        if let Value::String(v) = v {
                            dependencies.insert(k.clone(), v.clone());
                        }
                    }
                }
                _ => {
                    return Err("dependencies is not a object".to_string());
                }
            }
            env.write_meta_file(
                &full_pkg_id,
                &dependencies,
                &pkg.author_did,
                &pkg.author_name,
            )
            .map_err(|e| {
                error!("write meta file failed! {}", e);
                format!("write meta file failed! {}", e)
            })?;
        }

        Ok(())
    }

    pub async fn chunk_to_local_file(
        chunk_id: &str,
        chunk_mgr_id: Option<&str>,
        local_file: &PathBuf,
    ) -> RepoResult<()> {
        // TODO 下载完毕后检查chunk_id是否正确
        let named_mgr =
            NamedDataMgr::get_named_data_mgr_by_id(None)
                .await
                .ok_or(RepoError::NotFound(format!(
                    "chunk mgr {:?} not found",
                    chunk_mgr_id
                )))?;

        let chunk_id = ChunkId::new(chunk_id)
            .map_err(|e| RepoError::ParseError(chunk_id.to_string(), e.to_string()))?;

        let named_mgr = named_mgr.lock().await;

        let (mut reader, size) = named_mgr
            .open_chunk_reader(&chunk_id, SeekFrom::Start(0), true)
            .await
            .unwrap();

        let mut file = File::create(local_file).await?;

        let mut buf = vec![0u8; 1024];
        let mut read_size = 0;
        while read_size < size {
            let read_len = reader
                .read(&mut buf)
                .await
                .map_err(|e| RepoError::NdnError(format!("Read chunk error:{:?}", e)))?;
            if read_len == 0 {
                break;
            }
            read_size += read_len as u64;
            file.write_all(&buf[..read_len]).await?;
        }
        file.flush().await?;

        Ok(())
    }

    fn unpack(tar_gz_path: &PathBuf, target_dir: &PathBuf) -> io::Result<()> {
        if target_dir.exists() {
            fs::remove_dir_all(target_dir)?;
        }
        fs::create_dir_all(target_dir)?;
        let tar_gz = std::fs::File::open(tar_gz_path)?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        archive.unpack(target_dir)?;
        Ok(())
    }

    fn gen_rpc_session_token() -> Result<String, String> {
        let default_node_id = "node".to_string();
        let node_config = match Self::load_identity_config(default_node_id.as_ref()) {
            Ok(node_config) => node_config,
            Err(e) => {
                println!("{}", e);
                return Err(e);
            }
        };

        let device_private_key = match Self::load_device_private_key(default_node_id.as_str()) {
            Ok(device_private_key) => device_private_key,
            Err(e) => {
                println!("{}", e);
                return Err(e);
            }
        };

        let device_doc_json = match decode_json_from_jwt_with_default_pk(
            &node_config.device_doc_jwt,
            &node_config.owner_public_key,
        ) {
            Ok(device_doc_json) => device_doc_json,
            Err(_e) => {
                println!("decode device doc from jwt failed!");
                return Err("decode device doc from jwt failed!".to_string());
            }
        };

        let device_doc: DeviceConfig = match serde_json::from_value(device_doc_json) {
            Ok(device_doc) => device_doc,
            Err(e) => {
                println!("parse device doc failed! {}", e);
                return Err("parse device doc failed!".to_string());
            }
        };

        let now = SystemTime::now();
        let since_the_epoch = now.duration_since(UNIX_EPOCH).expect("Time went backwards");
        let timestamp = since_the_epoch.as_secs();
        let device_session_token = kRPC::RPCSessionToken {
            token_type: kRPC::RPCSessionTokenType::JWT,
            nonce: None,
            userid: Some(device_doc.name.clone()),
            appid: Some("kernel".to_string()),
            exp: Some(timestamp + 3600 * 24 * 7),
            iss: Some(device_doc.name.clone()),
            token: None,
        };

        let device_session_token_jwt = device_session_token
            .generate_jwt(Some(device_doc.did.clone()), &device_private_key)
            .map_err(|err| {
                println!("generate device session token failed! {}", err);
                return String::from("generate device session token failed!");
            })?;

        Ok(device_session_token_jwt)
    }

    fn load_identity_config(node_id: &str) -> Result<NodeIdentityConfig, String> {
        //load ./node_identity.toml for debug
        //load from /opt/buckyos/etc/node_identity.toml
        let mut file_path = PathBuf::from(format!("{}_identity.toml", node_id));
        let path = Path::new(&file_path);
        if !path.exists() {
            let etc_dir = get_buckyos_system_etc_dir();
            file_path = etc_dir.join(format!("{}_identity.toml", node_id));
        }

        let contents = std::fs::read_to_string(file_path.clone())
            .map_err(|err| format!("read node identity config failed! {}", err))?;

        let config: NodeIdentityConfig = toml::from_str(&contents)
            .map_err(|err| format!("Failed to parse NodeIdentityConfig TOML: {}", err))?;

        Ok(config)
    }

    fn load_device_private_key(node_id: &str) -> Result<EncodingKey, String> {
        let mut file_path = format!("{}_private_key.pem", node_id);
        let path = Path::new(file_path.as_str());
        if !path.exists() {
            let etc_dir = get_buckyos_system_etc_dir();
            file_path = format!("{}/{}_private_key.pem", etc_dir.to_string_lossy(), node_id);
        }
        let private_key = std::fs::read_to_string(file_path.clone())
            .map_err(|err| format!("read device private key failed! {}", err))?;

        let private_key: EncodingKey = EncodingKey::from_ed_pem(private_key.as_bytes())
            .map_err(|err| format!("parse device private key failed! {}", err))?;

        Ok(private_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::remove_file;

    #[tokio::test]
    async fn test_install() {
        let pkg_id = "buckyos-kit";
        let target = PathBuf::from("/tmp/buckyos");
        let result = Installer::install(pkg_id, &target, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_chunk_to_local_file() {
        let chunk_id = "buckyos-kit";
        let chunk_mgr_id = None;
        let local_file = PathBuf::from("/tmp/buckyos/buckyos-kit");
        let result = Installer::chunk_to_local_file(chunk_id, chunk_mgr_id, &local_file).await;
        assert!(result.is_ok());
        remove_file(local_file).unwrap();
    }
}
