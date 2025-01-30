use buckyos_kit::get_buckyos_system_etc_dir;
use jsonwebtoken::EncodingKey;
//use kRPC::kRPC;
use log::*;
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig};
use ndn_lib::*;
use package_lib::*;
use repo_service::*;
use serde::Deserialize;
use serde_json::json;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
        pkg_id: &str,
        target: &PathBuf,
        callback: Option<Box<dyn FnOnce(Result<(), String>) + Send>>,
    ) -> Result<(), String> {
        let env = PackageEnv::new(target.clone());
        if env.is_pkg_ready(pkg_id).map_err(|err| {
            error!("check pkg ready failed! {:?}", err);
            format!("check pkg ready failed! {:?}", err)
        })? {
            return Ok(());
        }

        //从repo中下载pkg
        let session_token = Self::gen_rpc_session_token().map_err(|err| {
            error!("generate rpc session token failed! {}", err);
            format!("generate rpc session token failed! {}", err)
        })?;

        let pkg_id = PackageId::from_str(pkg_id).map_err(|err| {
            error!("parse pkg id failed! {}", err);
            format!("parse pkg id failed! {}", err)
        })?;
        let client = kRPC::kRPC::new("http://127.0.0.1:4000/kapi/repo", Some(session_token));
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

        let install_result: InstallRequestResult = serde_json::from_value(result).map_err(|e| {
            error!(
                "Failed to deserialize install request result from json_value, error:{:?}",
                e
            );
            format!(
                "Failed to deserialize install request result from json_value, error:{:?}",
                e
            )
        })?;

        let task_id = install_result.task_id;

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
                        info!("task {} finished", id);
                        for meta in deps {
                            // let chunk_id = meta.chunk_id;
                            // let dest_file =
                            //     target.join(format!("{}#{}", meta.pkg_name, meta.version));
                            // match Self::chunk_to_local_file(&chunk_id, None, &local_file).await {
                            //     Ok(_) => {
                            //         info!(
                            //             "chunk {} to local file {} success",
                            //             chunk_id,
                            //             local_file.display()
                            //         );
                            //     }
                            //     Err(e) => {
                            //         error!(
                            //             "chunk {} to local file {} failed! {}",
                            //             chunk_id,
                            //             local_file.display(),
                            //             e
                            //         );
                            //         return Err(e);
                            //     }
                            // }
                            //TODO: install all deps
                        }
                        break;
                    }
                    TaskStatus::Error(reason) => {
                        let err_msg = format!("task {} failed! {}", id, reason);
                        error!("{}", err_msg);
                        return Err(err_msg);
                    }
                    v => {
                        info!("task {} is {:?}", id, v);
                    }
                },
                _ => {
                    let err_msg = format!("task {} is not an InstallTask", task_id);
                    warn!("{}", err_msg);
                    return Err(err_msg);
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        Ok(())
    }

    pub async fn chunk_to_local_file(
        chunk_id: &str,
        chunk_mgr_id: Option<&str>,
        local_file: &PathBuf,
    ) -> RepoResult<()> {
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
