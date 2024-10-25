use std::net::{IpAddr, Ipv6Addr};
use std::str::FromStr;
use tokio::net::UdpSocket;
use std::net::ToSocketAddrs;
use serde::{Serialize,Deserialize};
use serde_json::json;
use thiserror::Error;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use rand::rngs::OsRng;
use ed25519_dalek::{ed25519::signature::SignerMut, SigningKey};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, engine::general_purpose::STANDARD,Engine as _};
use base64::prelude::BASE64_STANDARD_NO_PAD;
use crate::config::DeviceConfig;
use sysinfo::{Components, Disks, Networks, System};

#[derive(Error, Debug)]
pub enum NSError {
    #[error("Failed: {0}")]
    Failed(String),
    #[error("Invalid response")]
    InvalidData,
    #[error("{0} not found")]
    NotFound(String),
    #[error("decode txt record error")]
    DnsTxtEncodeError,
    #[error("forbidden")]
    Forbid,
    #[error("DNS protocl error: {0}")]
    DNSProtoError(String),
    #[error("Failed to serialize extra: {0}")]
    ReadLocalFileError(String),
    #[error("Failed to decode jwt {0}")]
    DecodeJWTError(String),
}

pub type NSResult<T> = Result<T, NSError>;

pub fn is_did(identifier: &str) -> bool {
    if identifier.starts_with("did:") {
        let parts: Vec<&str> = identifier.split(':').collect();
        return parts.len() == 3 && !parts[1].is_empty() && !parts[2].is_empty();
    }
    false
}


pub fn decode_jwt_claim_without_verify(jwt: &str) -> NSResult<serde_json::Value> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(NSError::Failed("parts.len != 3".to_string())); // JWT 应该由三个部分组成
    }
    let claims_part = parts[1];
    let claims_bytes = URL_SAFE_NO_PAD.decode(claims_part).map_err(|_| NSError::Failed("base64 decode error".to_string()))?;
    let claims_str = String::from_utf8(claims_bytes).map_err(|_| NSError::Failed("String::from_utf8 error".to_string()))?;
    let claims: serde_json::Value = serde_json::from_str(claims_str.as_str()).map_err(|_| NSError::Failed("serde_json::from_str error".to_string()))?;

    Ok(claims)
}

pub fn decode_json_from_jwt_with_default_pk(jwt: &str,jwk:&jsonwebtoken::jwk::Jwk) -> NSResult<serde_json::Value> {

    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        NSError::DecodeJWTError("JWT decode header error".to_string())
    })?;

    let public_key = DecodingKey::from_jwk(jwk).unwrap();
    let validation = Validation::new(header.alg);

    let decoded_token = decode::<serde_json::Value>(jwt, &public_key, &validation).map_err(
        |error| NSError::DecodeJWTError(format!("JWT decode error:{}",error))
    )?;

    let decoded_json = decoded_token.claims.as_object()
        .ok_or(NSError::DecodeJWTError("Invalid token".to_string()))?;

    let result_value = serde_json::Value::Object(decoded_json.clone());

    Ok(result_value)
}

pub fn decode_json_from_jwt_with_pk(jwt: &str,pk:&jsonwebtoken::DecodingKey) -> NSResult<serde_json::Value> {

    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        NSError::DecodeJWTError("JWT decode header error".to_string())
    })?;

    let validation = Validation::new(header.alg);

    let decoded_token = decode::<serde_json::Value>(jwt,pk, &validation).map_err(
        |error| NSError::DecodeJWTError(format!("JWT decode error:{}",error))
    )?;

    let decoded_json = decoded_token.claims.as_object()
        .ok_or(NSError::DecodeJWTError("Invalid token".to_string()))?;

    let result_value = serde_json::Value::Object(decoded_json.clone());

    Ok(result_value)
}

fn build_pkcs8(private_key: &[u8]) -> Vec<u8> {
    let mut pkcs8 = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20
    ];
    pkcs8.extend_from_slice(private_key);
    pkcs8
}

pub fn generate_ed25519_key_pair() -> (String, serde_json::Value) {
    let mut csprng = OsRng{};
    let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    let private_key_bytes = signing_key.to_bytes();
    let pkcs8_bytes = build_pkcs8(&private_key_bytes);
    let private_key_pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        STANDARD.encode(&pkcs8_bytes)
    );

    let public_key_jwk = json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes()),
    });

    (private_key_pem, public_key_jwk)
}


pub fn get_device_did_from_ed25519_jwk_str(public_key: &str) -> NSResult<String> {
    let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_str(public_key)
        .map_err(|_| NSError::Failed("Invalid public key".to_string()))?;
    let jwk_value = serde_json::to_value(jwk)
        .map_err(|_| NSError::Failed("Invalid public key".to_string()))?;
    let x = jwk_value.get("x")
        .ok_or(NSError::Failed("Invalid public key".to_string()))?;
    let did = format!("did:dev:{}",x.as_str().unwrap());
    Ok(did)
}

pub fn get_device_did_from_ed25519_jwk(public_key: &serde_json::Value) -> NSResult<String> {
    let x = public_key.get("x")
        .ok_or(NSError::Failed("Invalid public key".to_string()))?;
    let did = format!("did:dev:{}",x.as_str().unwrap());
    Ok(did)
}
