use buckyos_kit::get_buckyos_system_etc_dir;
use jsonwebtoken::EncodingKey;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig};

#[derive(Deserialize, Debug)]
pub struct NodeIdentityConfig {
    pub zone_name: String, // $name.buckyos.org or did:ens:$name
    pub owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner
    pub owner_name: String, //owner's name
    pub device_doc_jwt: String, //device document,jwt string,siged by owner
    pub zone_nonce: String, // random string, is default password of some service
                           //device_private_key: ,storage in partical file
}

pub fn load_identity_config(node_id: &str) -> Result<NodeIdentityConfig, String> {
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

pub fn load_device_private_key(node_id: &str) -> Result<EncodingKey, String> {
    let mut file_path = format!("{}_private_key.pem", node_id);
    let path = Path::new(file_path.as_str());
    if !path.exists() {
        let etc_dir = get_buckyos_system_etc_dir();
        file_path = format!("{}/{}_private_key.pem", etc_dir.to_string_lossy(), node_id);
    }

    load_private_key_from_file(file_path.as_str())
}

pub fn load_private_key_from_file(file_path: &str) -> Result<EncodingKey, String> {
    let private_key = std::fs::read_to_string(file_path)
        .map_err(|err| format!("read device private key failed! {}", err))?;

    let private_key: EncodingKey = EncodingKey::from_ed_pem(private_key.as_bytes())
        .map_err(|err| format!("parse device private key failed! {}", err))?;

    Ok(private_key)
}

pub(crate) fn get_device_doc(node_id: &str) -> Result<DeviceConfig, String> {
    let node_identity = load_identity_config(node_id)?;
    let device_doc_json = decode_json_from_jwt_with_default_pk(&node_identity.device_doc_jwt, &node_identity.owner_public_key).unwrap();
    serde_json::from_value(device_doc_json).unwrap()
}

pub(crate) fn get_device_token_jwt(device_private_key: &EncodingKey, device_doc: &DeviceConfig) -> Result<String, String> {
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
