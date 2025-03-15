use buckyos_kit::get_buckyos_system_etc_dir;
use jsonwebtoken::EncodingKey;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig};


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
