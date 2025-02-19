use std::path::PathBuf;

use crate::def::*;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Header, TokenData, Validation};
use log::info;
use name_client::*;
use serde_json::Value;

pub async fn verify(hostname: &str, jwt: &str) -> RepoResult<Value> {
    let (auth_key, remote_did_id) = resolve_ed25519_auth_key(hostname).await.map_err(|e| {
        log::error!(
            "resolve_ed25519_auth_key failed, author: {}, {:?}",
            hostname,
            e
        );
        RepoError::VerifyError(format!(
            "resolve_ed25519_auth_key failed, author: {}, {:?}",
            hostname, e
        ))
    })?;
    let public_key = DecodingKey::from_ed_der(&auth_key);

    let header: Header = decode_header(jwt).map_err(|error| {
        log::error!("decode jwt header failed: {:?}", error);
        RepoError::VerifyError(format!("decode jwt header failed: {:?}", error))
    })?;

    let validation = Validation::new(header.alg);

    let decoded_token =
        decode::<serde_json::Value>(jwt, &public_key, &validation).map_err(|error| {
            log::error!("decode jwt token failed: {:?}", error);
            RepoError::VerifyError(format!("decode jwt token failed: {:?}", error))
        })?;

    let decoded_json = match decoded_token.claims.as_object() {
        Some(json) => json.clone(),
        None => {
            log::error!("decode jwt token failed: invalid json");
            return Err(RepoError::VerifyError(
                "decode jwt token failed: invalid json".to_string(),
            ));
        }
    };
    let result = Value::Object(decoded_json);

    Ok(result)
}

pub fn sign_data(pem_file: &str, data: &str) -> RepoResult<String> {
    //TODO: 服务内部不应该直接操作私钥，应该通过调用签名服务来签名
    !unimplemented!("sign_data");
    // let signing_key = SigningKey::read_pkcs8_pem_file(pem_file).map_err(|e| {
    //     RepoError::LoadError(
    //         pem_file.to_string(),
    //         format!("read pkcs8 pem file failed: {:?}", e),
    //     )
    // })?;

    // let signature: Signature = signing_key.sign(data.as_bytes());

    // // convert signature to base64
    // let signature_base64 = general_purpose::STANDARD.encode(signature.to_bytes());

    // Ok(signature_base64)
}
