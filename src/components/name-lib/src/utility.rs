use thiserror::Error;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
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
