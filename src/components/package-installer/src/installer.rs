use buckyos_kit::get_buckyos_system_etc_dir;
use flate2::read::GzDecoder;
use jsonwebtoken::EncodingKey;
use log::*;
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig};
use ndn_lib::*;
use package_lib::*;
use repo_service::*;
use serde::de;
use serde::Deserialize;
use serde_json::{json, Value};
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
    zone_name: String,
    owner_public_key: jsonwebtoken::jwk::Jwk,
    owner_name: String,
    device_doc_jwt: String,
    zone_nonce: String,
}

impl Installer {
    pub async fn install(
        pkg_id_str: &str,
        target: &PathBuf,
        local_repo_url: &str,
        named_mgr_id: Option<&str>,
    ) -> Result<Vec<PackageId>, String> {
        let env = PackageEnv::new(target.clone());
        if let Ok(deps) = env.check_pkg_ready(pkg_id_str) {
            info!(
                "Package {} is already installed, deps:{:?}",
                pkg_id_str, deps
            );
            return Ok(deps);
        }

        let session_token = Self::gen_rpc_session_token()?;
        let pkg_id = PackageId::from_str(pkg_id_str)
            .map_err(|err| Self::log_error(format!("Parse package id failed: {}", err)))?;

        let client = kRPC::kRPC::new(local_repo_url, Some(session_token));
        let version = pkg_id
            .sha256
            .or(pkg_id.version)
            .unwrap_or_else(|| "*".to_string());

        let result = client
            .call(
                "install_pkg",
                json!({ "pkg_name": pkg_id.name, "version": version }),
            )
            .await
            .map_err(|err| Self::log_error(format!("Call install_pkg failed: {}", err)))?;

        let install_task_result: InstallRequestResult =
            serde_json::from_value(result).map_err(|err| {
                Self::log_error(format!(
                    "Failed to deserialize install task result: {}",
                    err
                ))
            })?;

        Self::poll_task(&client, install_task_result.task_id, &env, named_mgr_id).await
    }

    async fn poll_task(
        client: &kRPC::kRPC,
        task_id: String,
        env: &PackageEnv,
        named_mgr_id: Option<&str>,
    ) -> Result<Vec<PackageId>, String> {
        loop {
            let result = client
                .call("query_task", json!({ "task_id": task_id }))
                .await
                .map_err(|err| Self::log_error(format!("Call query_task failed: {}", err)))?;

            let task: Task = serde_json::from_value(result)
                .map_err(|err| Self::log_error(format!("Failed to deserialize task: {}", err)))?;

            match task {
                Task::InstallTask {
                    id,
                    package_id,
                    status,
                    deps,
                    ..
                } => match status {
                    TaskStatus::Finished => {
                        info!("Task {} finished", id);
                        return Self::install_pkgs_from_repo(&package_id, &deps, env, named_mgr_id)
                            .await;
                    }
                    TaskStatus::Error(reason) => {
                        return Err(Self::log_error(format!("Task {} failed: {}", id, reason)));
                    }
                    _ => info!("Task {} is {:?}", id, status),
                },
                _ => {
                    return Err(Self::log_error(format!("Invalid task type: {:?}", task)));
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn install_pkgs_from_repo(
        package_id: &PackageId,
        pkgs: &[PackageMeta],
        env: &PackageEnv,
        named_mgr_id: Option<&str>,
    ) -> Result<Vec<PackageId>, String> {
        let mut deps = Vec::new();
        for pkg in pkgs {
            debug!("Installing package {:?}", pkg);
            let chunk_id = &pkg.chunk_id.replace(":", "-");
            let dest_file_tmp = env.get_cache_dir().join(format!("{}.tmp", chunk_id));
            let dest_file = env.get_cache_dir().join(chunk_id);

            if !dest_file.exists()
                || !Self::verify_file_chunk_id(&dest_file, &pkg.chunk_id)
                    .await
                    .is_ok()
            {
                debug!(
                    "Downloading chunk {} to {}",
                    pkg.chunk_id,
                    dest_file_tmp.display()
                );
                Self::chunk_to_local_file(&pkg.chunk_id, named_mgr_id, &dest_file_tmp)
                    .await
                    .map_err(|e| e.to_string())?;
                tokio::fs::rename(&dest_file_tmp, &dest_file)
                    .await
                    .map_err(|err| {
                        Self::log_error(format!(
                            "Rename {} to {} failed: {}",
                            dest_file_tmp.display(),
                            dest_file.display(),
                            err
                        ))
                    })?;
            }

            let dest_dir = env
                .get_install_dir()
                .join(format!("{}#{}", pkg.pkg_name, pkg.version));

            Self::unpack(&dest_file, &dest_dir).map_err(|err| {
                Self::log_error(format!("Unpack {} failed: {}", dest_file.display(), err))
            })?;

            let full_pkg_id = format!("{}#{}#{}", pkg.pkg_name, pkg.version, pkg.chunk_id);
            let full_pkg_id = PackageId::from_str(&full_pkg_id)
                .map_err(|err| Self::log_error(format!("Parse full package id failed: {}", err)))?;

            let dependencies = Self::extract_dependencies(&pkg.dependencies)?;
            env.write_meta_file(
                &full_pkg_id,
                &dependencies,
                &pkg.author_did,
                &pkg.author_name,
            )
            .map_err(|err| {
                Self::log_error(format!(
                    "Write meta file for {:?} failed: {}",
                    full_pkg_id, err
                ))
            })?;

            deps.push(full_pkg_id);
        }
        Ok(deps)
    }

    async fn verify_file_chunk_id(file: &PathBuf, chunk_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn chunk_to_local_file(
        chunk_id: &str,
        chunk_mgr_id: Option<&str>,
        local_file: &PathBuf,
    ) -> RepoResult<()> {
        let named_mgr =
            NamedDataMgr::get_named_data_mgr_by_id(None)
                .await
                .ok_or(RepoError::NotFound(format!(
                    "Chunk mgr {:?} not found",
                    chunk_mgr_id
                )))?;

        let chunk_id = ChunkId::new(chunk_id)
            .map_err(|e| RepoError::ParseError(chunk_id.to_string(), e.to_string()))?;
        let named_mgr = named_mgr.lock().await;
        let (mut reader, size) = named_mgr
            .open_chunk_reader(&chunk_id, SeekFrom::Start(0), true)
            .await
            .map_err(|e| RepoError::NdnError(format!("Open chunk reader error:{:?}", e)))?;

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
        debug!(
            "Unpacking {} to {}",
            tar_gz_path.display(),
            target_dir.display()
        );
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
        let node_config = Self::load_identity_config(&default_node_id)?;
        let device_private_key = Self::load_device_private_key(&default_node_id)?;

        let device_doc_json = decode_json_from_jwt_with_default_pk(
            &node_config.device_doc_jwt,
            &node_config.owner_public_key,
        )
        .map_err(|_| "Decode device doc from JWT failed!".to_string())?;

        let device_doc: DeviceConfig = serde_json::from_value(device_doc_json)
            .map_err(|e| format!("Parse device doc failed! {}", e))?;

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

        device_session_token
            .generate_jwt(Some(device_doc.did.clone()), &device_private_key)
            .map_err(|err| format!("Generate device session token failed! {}", err))
    }

    fn load_identity_config(node_id: &str) -> Result<NodeIdentityConfig, String> {
        let mut file_path = PathBuf::from(format!("{}_identity.toml", node_id));
        if !file_path.exists() {
            file_path = get_buckyos_system_etc_dir().join(format!("{}_identity.toml", node_id));
        }

        let contents = fs::read_to_string(&file_path)
            .map_err(|err| format!("Read node identity config failed! {}", err))?;
        toml::from_str(&contents)
            .map_err(|err| format!("Failed to parse NodeIdentityConfig TOML: {}", err))
    }

    fn load_device_private_key(node_id: &str) -> Result<EncodingKey, String> {
        let mut file_path = format!("{}_private_key.pem", node_id);
        if !Path::new(&file_path).exists() {
            file_path = format!(
                "{}/{}_private_key.pem",
                get_buckyos_system_etc_dir().to_string_lossy(),
                node_id
            );
        }
        let private_key = fs::read_to_string(&file_path)
            .map_err(|err| format!("Read device private key failed! {}", err))?;
        EncodingKey::from_ed_pem(private_key.as_bytes())
            .map_err(|err| format!("Parse device private key failed! {}", err))
    }

    fn extract_dependencies(dependencies: &Value) -> Result<HashMap<String, String>, String> {
        let mut result = HashMap::new();
        if let Value::Object(map) = dependencies {
            for (k, v) in map.iter() {
                if let Value::String(v) = v {
                    result.insert(k.clone(), v.clone());
                }
            }
        } else {
            return Err("Invalid dependencies format".to_string());
        }
        Ok(result)
    }

    fn log_error(err_msg: String) -> String {
        error!("{}", err_msg);
        err_msg
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
        let result = Installer::install(pkg_id, &target, "http://example.com", None).await;
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
