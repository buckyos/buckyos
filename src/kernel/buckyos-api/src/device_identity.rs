use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{DecodingKey, EncodingKey};
use name_client::{IdentityMaterial, IdentityRoots, IdentityUsage};
use name_lib::{
    decode_jwt_claim_without_verify, load_private_key, DIDDocumentTrait, DeviceDocument,
    EncodedDocument, DID,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const NODE_IDENTITY_SCHEMA_V2: &str = "buckyos.node_identity.v2";
pub const DEVICE_DOC_JWT_FILE_NAME: &str = "device_doc.jwt";
pub const DEVICE_MINI_DOC_JWT_FILE_NAME: &str = "device_mini_doc.jwt";
pub const NODE_GATEWAY_PARAMS_FILE_NAME: &str = "node_gateway_params.json";
pub const ZONE_DOCUMENT_JWT_FILE_NAME: &str = "zone_document.jwt";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalNodeIdentityConfig {
    #[serde(default)]
    pub schema: String,
    pub zone_did: DID,
    pub owner_did: DID,
    pub owner_public_key: Jwk,
    pub device_name: String,
    pub device_did: DID,
    pub zone_iat: u32,
}

impl LocalNodeIdentityConfig {
    pub fn new(
        zone_did: DID,
        owner_did: DID,
        owner_public_key: Jwk,
        device_name: String,
        device_did: DID,
        zone_iat: u32,
    ) -> Self {
        Self {
            schema: NODE_IDENTITY_SCHEMA_V2.to_string(),
            zone_did,
            owner_did,
            owner_public_key,
            device_name,
            device_did,
            zone_iat,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceIdentityPaths {
    pub public_dir: PathBuf,
    pub security_dir: PathBuf,
    pub did_json: PathBuf,
    pub device_doc_jwt: PathBuf,
    pub device_mini_doc_jwt: PathBuf,
    pub authentication_private_key: PathBuf,
}

pub fn load_local_node_identity_config(
    file_path: &Path,
) -> std::result::Result<LocalNodeIdentityConfig, String> {
    let contents = fs::read_to_string(file_path)
        .map_err(|err| format!("read {} failed: {}", file_path.display(), err))?;
    let config: LocalNodeIdentityConfig = serde_json::from_str(&contents)
        .map_err(|err| format!("parse {} failed: {}", file_path.display(), err))?;
    if config.schema != NODE_IDENTITY_SCHEMA_V2 {
        return Err(format!(
            "unsupported node_identity schema '{}', expected '{}'",
            config.schema, NODE_IDENTITY_SCHEMA_V2
        ));
    }
    Ok(config)
}

pub fn identity_roots() -> std::result::Result<IdentityRoots, String> {
    IdentityRoots::from_env_or_buckyos_root()
        .map_err(|err| format!("load identity roots failed: {}", err))
}

pub fn identity_roots_for_buckyos_root(root_dir: &Path) -> IdentityRoots {
    IdentityRoots::new(
        root_dir.join("local").join("identity"),
        root_dir.join("security"),
    )
}

pub fn device_identity_paths(device_did: &DID) -> std::result::Result<DeviceIdentityPaths, String> {
    let roots = identity_roots()?;
    device_identity_paths_for_roots(&roots, device_did)
}

pub fn device_identity_paths_for_roots(
    roots: &IdentityRoots,
    device_did: &DID,
) -> std::result::Result<DeviceIdentityPaths, String> {
    let device_did_str = device_did.to_string();
    let public_dir = roots
        .public_dir(device_did_str.as_str())
        .map_err(|err| format!("build public identity dir failed: {}", err))?;
    let security_dir = roots
        .security_dir(device_did_str.as_str())
        .map_err(|err| format!("build security identity dir failed: {}", err))?;
    let did_json = roots
        .public_file(
            device_did_str.as_str(),
            IdentityUsage::Authentication,
            IdentityMaterial::DidDocJson(None),
        )
        .map_err(|err| format!("build did.json path failed: {}", err))?;
    let authentication_private_key = roots
        .security_file(
            device_did_str.as_str(),
            IdentityUsage::Authentication,
            IdentityMaterial::PrivateKey,
        )
        .map_err(|err| format!("build authentication private key path failed: {}", err))?;
    Ok(DeviceIdentityPaths {
        device_doc_jwt: public_dir.join(DEVICE_DOC_JWT_FILE_NAME),
        device_mini_doc_jwt: public_dir.join(DEVICE_MINI_DOC_JWT_FILE_NAME),
        public_dir,
        security_dir,
        did_json,
        authentication_private_key,
    })
}

pub fn build_device_did(device_name: &str, zone_did: &DID) -> std::result::Result<DID, String> {
    let device_name = device_name.trim();
    if device_name.is_empty() {
        return Err("device name is empty".to_string());
    }
    let zone_name = match zone_did.method.as_str() {
        "web" => zone_did.to_raw_host_name(),
        "bns" => zone_did
            .id
            .split(':')
            .next()
            .unwrap_or(zone_did.id.as_str())
            .to_string(),
        _ => zone_did
            .id
            .split(':')
            .next()
            .unwrap_or(zone_did.id.as_str())
            .to_string(),
    };
    if zone_name.trim().is_empty() {
        return Err(format!(
            "zone DID {} has empty host/name",
            zone_did.to_string()
        ));
    }
    Ok(DID::new(
        zone_did.method.as_str(),
        format!("{}.{}", device_name, zone_name).as_str(),
    ))
}

pub fn bind_device_config_did(
    device_config: DeviceDocument,
    device_did: &DID,
) -> std::result::Result<DeviceDocument, String> {
    let mut value = serde_json::to_value(device_config)
        .map_err(|err| format!("serialize device config failed: {}", err))?;
    let device_did_str = device_did.to_string();
    value["id"] = Value::String(device_did_str.clone());

    let methods = value
        .get_mut("verificationMethod")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "device config verificationMethod is missing".to_string())?;
    for method in methods.iter_mut() {
        let method = method
            .as_object_mut()
            .ok_or_else(|| "device config verificationMethod item is not object".to_string())?;
        method.insert(
            "controller".to_string(),
            Value::String(device_did_str.clone()),
        );
    }

    serde_json::from_value(value)
        .map_err(|err| format!("parse rebound device config failed: {}", err))
}

pub fn new_device_config_by_jwk_with_did(
    name: &str,
    public_key: Jwk,
    device_did: &DID,
) -> std::result::Result<DeviceDocument, String> {
    bind_device_config_did(DeviceDocument::new_by_jwk(name, public_key), device_did)
}

pub fn load_device_doc_jwt(device_did: &DID) -> std::result::Result<String, String> {
    let paths = device_identity_paths(device_did)?;
    fs::read_to_string(paths.device_doc_jwt.as_path()).map_err(|err| {
        format!(
            "read device_doc.jwt {} failed: {}",
            paths.device_doc_jwt.display(),
            err
        )
    })
}

pub fn load_device_mini_doc_jwt(device_did: &DID) -> std::result::Result<String, String> {
    let paths = device_identity_paths(device_did)?;
    fs::read_to_string(paths.device_mini_doc_jwt.as_path()).map_err(|err| {
        format!(
            "read device_mini_doc.jwt {} failed: {}",
            paths.device_mini_doc_jwt.display(),
            err
        )
    })
}

pub fn load_local_device_config(
    node_identity: &LocalNodeIdentityConfig,
    verify: bool,
) -> std::result::Result<(String, DeviceDocument), String> {
    let device_doc_jwt = load_device_doc_jwt(&node_identity.device_did)?;
    let encoded_doc = EncodedDocument::from_str(device_doc_jwt.clone())
        .map_err(|err| format!("parse device_doc.jwt failed: {}", err))?;
    let owner_key = if verify {
        Some(
            DecodingKey::from_jwk(&node_identity.owner_public_key)
                .map_err(|err| format!("parse owner public key failed: {}", err))?,
        )
    } else {
        None
    };
    let device_config = DeviceDocument::decode(&encoded_doc, owner_key.as_ref())
        .map_err(|err| format!("decode device_doc.jwt failed: {}", err))?;
    if device_config.id != node_identity.device_did {
        return Err(format!(
            "device_doc.jwt id {} does not match node_identity device_did {}",
            device_config.id.to_string(),
            node_identity.device_did.to_string()
        ));
    }
    Ok((device_doc_jwt, device_config))
}

pub fn load_local_device_private_key(device_did: &DID) -> std::result::Result<EncodingKey, String> {
    let paths = device_identity_paths(device_did)?;
    let key_path = paths.authentication_private_key;
    load_private_key(key_path.as_path()).map_err(|err| {
        format!(
            "load device authentication private key {} failed: {}",
            key_path.display(),
            err
        )
    })
}

pub fn save_local_device_identity(
    etc_dir: &Path,
    node_identity: &LocalNodeIdentityConfig,
    device_config: &DeviceDocument,
    device_doc_jwt: &str,
    device_mini_doc_jwt: &str,
    device_private_key_pem: &str,
) -> std::result::Result<DeviceIdentityPaths, String> {
    let roots = identity_roots()?;
    save_local_device_identity_for_roots(
        etc_dir,
        &roots,
        node_identity,
        device_config,
        device_doc_jwt,
        device_mini_doc_jwt,
        device_private_key_pem,
    )
}

pub fn save_local_device_identity_for_roots(
    etc_dir: &Path,
    roots: &IdentityRoots,
    node_identity: &LocalNodeIdentityConfig,
    device_config: &DeviceDocument,
    device_doc_jwt: &str,
    device_mini_doc_jwt: &str,
    device_private_key_pem: &str,
) -> std::result::Result<DeviceIdentityPaths, String> {
    let paths = device_identity_paths_for_roots(roots, &node_identity.device_did)?;
    fs::create_dir_all(paths.public_dir.as_path()).map_err(|err| {
        format!(
            "create public identity dir {} failed: {}",
            paths.public_dir.display(),
            err
        )
    })?;
    fs::create_dir_all(paths.security_dir.as_path()).map_err(|err| {
        format!(
            "create security identity dir {} failed: {}",
            paths.security_dir.display(),
            err
        )
    })?;

    write_json_pretty(paths.did_json.as_path(), device_config)?;
    atomic_write(paths.device_doc_jwt.as_path(), device_doc_jwt.as_bytes())?;
    atomic_write(
        paths.device_mini_doc_jwt.as_path(),
        device_mini_doc_jwt.as_bytes(),
    )?;
    atomic_write(
        paths.authentication_private_key.as_path(),
        device_private_key_pem.as_bytes(),
    )?;
    save_node_gateway_params(etc_dir, &node_identity.device_did)?;
    let node_identity_path = etc_dir.join("node_identity.json");
    write_json_pretty(node_identity_path.as_path(), node_identity)?;

    Ok(paths)
}

pub fn save_zone_document_jwt(
    etc_dir: &Path,
    zone_document_jwt: &str,
) -> std::result::Result<PathBuf, String> {
    let path = etc_dir.join(ZONE_DOCUMENT_JWT_FILE_NAME);
    atomic_write(path.as_path(), zone_document_jwt.as_bytes())?;
    Ok(path)
}

pub fn atomic_write(path: &Path, content: &[u8]) -> std::result::Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|err| format!("create {} failed: {}", parent.display(), err))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid file name {}", path.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("build temporary file name failed: {}", err))?
        .as_nanos();
    let temp_path = parent.join(format!(
        ".{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        nonce
    ));
    let mut temp_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temp_path.as_path())
        .map_err(|err| format!("create {} failed: {}", temp_path.display(), err))?;
    temp_file
        .write_all(content)
        .and_then(|_| temp_file.sync_all())
        .map_err(|err| {
            let _ = fs::remove_file(temp_path.as_path());
            format!("write {} failed: {}", temp_path.display(), err)
        })?;
    drop(temp_file);
    fs::rename(temp_path.as_path(), path).map_err(|err| {
        let _ = fs::remove_file(temp_path.as_path());
        format!(
            "atomically replace {} from {} failed: {}",
            path.display(),
            temp_path.display(),
            err
        )
    })?;
    sync_parent_dir(parent)
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> std::result::Result<(), String> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|err| format!("sync {} failed: {}", parent.display(), err))
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> std::result::Result<(), String> {
    Ok(())
}

pub fn save_node_gateway_params(
    etc_dir: &Path,
    device_did: &DID,
) -> std::result::Result<(), String> {
    let params_path = etc_dir.join(NODE_GATEWAY_PARAMS_FILE_NAME);
    write_json_pretty(
        params_path.as_path(),
        &serde_json::json!({
            "params": {
                "device_did": device_did.to_string()
            }
        }),
    )
}

pub fn save_local_device_identity_for_buckyos_root(
    root_dir: &Path,
    etc_dir: &Path,
    node_identity: &LocalNodeIdentityConfig,
    device_config: &DeviceDocument,
    device_doc_jwt: &str,
    device_mini_doc_jwt: &str,
    device_private_key_pem: &str,
) -> std::result::Result<DeviceIdentityPaths, String> {
    let roots = identity_roots_for_buckyos_root(root_dir);
    save_local_device_identity_for_roots(
        etc_dir,
        &roots,
        node_identity,
        device_config,
        device_doc_jwt,
        device_mini_doc_jwt,
        device_private_key_pem,
    )
}

pub fn decode_device_config_without_verify(
    device_doc_jwt: &str,
) -> std::result::Result<DeviceDocument, String> {
    let value = decode_jwt_claim_without_verify(device_doc_jwt)
        .map_err(|err| format!("decode device doc jwt failed: {}", err))?;
    serde_json::from_value(value).map_err(|err| format!("parse device doc jwt failed: {}", err))
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> std::result::Result<(), String> {
    let content = serde_json::to_string_pretty(value)
        .map_err(|err| format!("serialize {} failed: {}", path.display(), err))?;
    atomic_write(path, content.as_bytes())
}
