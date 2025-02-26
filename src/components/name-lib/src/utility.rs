use std::net::{IpAddr, Ipv6Addr};
use std::str::FromStr;
use tokio::net::UdpSocket;
use std::net::ToSocketAddrs;
use std::path::Path;
use serde::{Serialize, Deserialize};
use serde_json::json;
use thiserror::Error;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use rand::rngs::OsRng;
use ed25519_dalek::{ed25519::signature::SignerMut, SigningKey};
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};
use curve25519_dalek::montgomery::MontgomeryPoint;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, engine::general_purpose::STANDARD,Engine as _};
use base64::prelude::BASE64_STANDARD_NO_PAD;
use crate::config::DeviceConfig;
use sysinfo::{Components, Disks, Networks, System};
use log::*;

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
    #[error("Final Error: {0}")]
    FinalError(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),
}

pub type NSResult<T> = Result<T, NSError>;

pub fn is_did(identifier: &str) -> bool {
    if identifier.starts_with("did:") {
        let parts: Vec<&str> = identifier.split(':').collect();
        return parts.len() == 3 && !parts[1].is_empty() && !parts[2].is_empty();
    }
    false
}

pub fn get_x_from_jwk_string(jwk_string: &str) -> NSResult<String> {
    let jwk_json = serde_json::from_str::<serde_json::Value>(jwk_string).map_err(|_| NSError::Failed("Invalid jwk".to_string()))?;
    let x = jwk_json.get("x")
        .ok_or(NSError::Failed("Invalid jwk".to_string()))?;
    let x_str = x.as_str().unwrap().to_string();
    Ok(x_str)
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

pub fn from_pkcs8(pkcs8: &[u8]) -> NSResult<[u8;32]> {
    // Check if input has the minimum required length (16 bytes header + 32 bytes key)
    if pkcs8.len() < 48 {
        return Err(NSError::Failed("Invalid PKCS#8 data length".to_string()));
    }

    // Verify PKCS#8 header for Ed25519
    let expected_header = [
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20
    ];
    
    if &pkcs8[..16] != expected_header {
        return Err(NSError::Failed("Invalid PKCS#8 header".to_string()));
    }

    // Extract the 32-byte private key
    let mut private_key = [0u8; 32];
    private_key.copy_from_slice(&pkcs8[16..48]);
    
    Ok(private_key)
}

//TODO: would use a PEM parser library
pub fn load_pem_private_key<P: AsRef<Path>>(file_path: P) -> NSResult<[u8;48]> {
    // load from /etc/buckyos/node_private_key.toml
    let contents = std::fs::read_to_string(&file_path).map_err(|err| {
        error!("read private key failed! {}", err);
        return NSError::ReadLocalFileError(file_path.as_ref().to_string_lossy().to_string());
    })?;

    let start_pos = contents.find("-----BEGIN PRIVATE KEY-----");
    let end_pos = contents.find("-----END PRIVATE KEY-----");
    if start_pos.is_none() || end_pos.is_none() {
        return Err(NSError::Failed("Invalid private key".to_string()));
    }
    let start_pos = start_pos.unwrap() + "-----BEGIN PRIVATE KEY-----".len();
    let end_pos = end_pos.unwrap();
    let mut b64content = contents[start_pos..end_pos].to_string();
    let b64content = b64content.trim();
    let private_key_bytes = STANDARD.decode(b64content)
        .map_err(|err| NSError::Failed(format!("base64 decode error:{}",err)))?;

    //from_pkcs8(&private_key_bytes)
    Ok(private_key_bytes.try_into().unwrap())
}

// Generate a random private key and return the PKCS#8 encoded bytes
pub fn generate_ed25519_key() -> (SigningKey, [u8;48]) {
    let mut csprng = rand::rngs::OsRng{};
    let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    let private_key_bytes = signing_key.to_bytes();
    let pkcs8_bytes = build_pkcs8(&private_key_bytes);

    (signing_key, pkcs8_bytes.try_into().unwrap())
}

// Encode the Ed25519 public key to a JWK
pub fn encode_ed25519_sk_to_pk_jwt(sk: &SigningKey) -> serde_json::Value {
    let public_key_jwk = json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": encode_ed25519_sk_to_pk(sk),
    });

    public_key_jwk
}

pub fn encode_ed25519_sk_to_pk(sk: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(sk.verifying_key().to_bytes())
}

pub fn encode_ed25519_pkcs8_sk_to_pk(pkcs8_bytes: &[u8]) -> String {
    let sk_bytes = from_pkcs8(pkcs8_bytes).unwrap();
    let sk = SigningKey::from_bytes(&sk_bytes);

    encode_ed25519_sk_to_pk(&sk)
}

pub fn generate_ed25519_key_pair() -> (String, serde_json::Value) {
    
    let (signing_key, pkcs8_bytes) = generate_ed25519_key();

    let private_key_pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        STANDARD.encode(&pkcs8_bytes)
    );

    let public_key_jwk = encode_ed25519_sk_to_pk_jwt(&signing_key);

    (private_key_pem, public_key_jwk)
}


pub fn generate_x25519_key_pair() -> (PublicKey, StaticSecret) {
    let mut csprng = OsRng;
    let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    
    let private_key_bytes = signing_key.to_bytes();
    let public_key_bytes = signing_key.verifying_key().to_bytes();
    println!("public_key_bytes: {:?}",public_key_bytes);
    println!("private_key_bytes: {:?}",private_key_bytes);

    let public_key_jwk = json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": URL_SAFE_NO_PAD.encode(public_key_bytes),
    });
    println!("{}",public_key_jwk);

    let pkcs8_bytes = build_pkcs8(&private_key_bytes);
    let private_key_pem = format!(
        "\n-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        STANDARD.encode(&pkcs8_bytes)
    );
    println!("{}",private_key_pem);


    let x25519_public_key = ed25519_to_curve25519::ed25519_pk_to_curve25519(public_key_bytes);
    println!("x25519_public_key: {:?}",x25519_public_key);
    let x25519_private_key = ed25519_to_curve25519::ed25519_sk_to_curve25519(private_key_bytes);
    println!("x25519_private_key: {:?}",x25519_private_key);

    let x25519_public_key = x25519_dalek::PublicKey::from(x25519_public_key);
    let x25519_private_key = x25519_dalek::StaticSecret::from(x25519_private_key);
    //let x25519_private_key = x25519_dalek::EphemeralSecret::from(x25519_private_key);

    (x25519_public_key, x25519_private_key)
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


mod test {
    use crate::DID;

    use super::*;

    #[test]
    fn test_generate_x25519_key_pair_share_secret() {
        let my_secret = EphemeralSecret::random();
        let my_public = PublicKey::from(&my_secret);

        //let (public_key1, private_key1) = generate_x25519_key_pair();
        let (public_key2, private_key2) = generate_x25519_key_pair();

        let share_secret1 = my_secret.diffie_hellman(&public_key2);
        let share_secret2 = private_key2.diffie_hellman(&my_public);
       
        assert_eq!(share_secret1.to_bytes(), share_secret2.to_bytes());  
    }

    #[test]
    fn test_generate_ed25519_key_pair() {
        let (private_key, public_key) = generate_ed25519_key_pair();
        println!("private_key: {}",private_key);
        println!("public_key: {}",serde_json::to_string(&public_key).unwrap());

    }

    #[test]
    fn generate_ed25519_key_pair_to_local() {
        // Get temp path
        let temp_dir = std::env::temp_dir();
        let key_dir = temp_dir.join("buckyos").join("keys");
        if !key_dir.is_dir() {
            std::fs::create_dir_all(&key_dir).unwrap();
        }
        println!("key_dir: {:?}",key_dir);

        let (private_key, public_key) = generate_ed25519_key_pair();

        let sk_file = key_dir.join("private_key.pem");
        std::fs::write(&sk_file, private_key).unwrap();

        let pk_file = key_dir.join("public_key.json");
        std::fs::write(&pk_file, serde_json::to_string(&public_key).unwrap()).unwrap();
    }

    #[test]
    fn test_load_pem_private_key() {
        let private_key = load_pem_private_key("d:\\temp\\device_key.pem").unwrap();
        println!("private_key: {:?}",private_key);
        let private_key_der = from_pkcs8(&private_key).unwrap();
        println!("private_key_der: {:?}",private_key_der);

        let private_key_x25519 = ed25519_to_curve25519::ed25519_sk_to_curve25519(private_key_der);
        println!("private_key_x25519: {:?}",private_key_x25519);

        let file_content = std::fs::read_to_string("d:\\temp\\device_key.pem").unwrap();
        println!("file_content: {}",file_content);

        //let encoding_key = EncodingKey::from_ed_pem(file_content.as_bytes()).unwrap();
        let encoding_key = EncodingKey::from_ed_der(&private_key);
        let encoding_key2 = EncodingKey::from_ed_pem(file_content.as_bytes()).unwrap();
        let test_payload = json!({
            "sub": "1234567890",
            "name": "John Doe",
            "iat": 135790
        });
        let mut header = Header::new(Algorithm::EdDSA);

        let token = encode(&header, &test_payload, &encoding_key).unwrap();
        let token2 = encode(&header, &test_payload, &encoding_key2).unwrap();
        println!("token: {}",token);
        println!("token2: {}",token2);
        assert_eq!(token, token2);

        //let sn_public_key 
        let did_str ="8vlobDX73HQj-w5TUjC_ynr_ljsWcDAgVOzsqXCw7no.dev.did";
        let sn_did = DID::from_host_name(did_str).unwrap();
        let sn_public_key = sn_did.get_auth_key().unwrap();
        println!("sn_public_key: {:?}",sn_public_key);
        let sn_x25519_public_key = ed25519_to_curve25519::ed25519_pk_to_curve25519(sn_public_key);
        println!("sn_x_public_key: {:?}",sn_x25519_public_key);
        let sn_x_public_key = x25519_dalek::PublicKey::from(sn_x25519_public_key);

        let my_secret = EphemeralSecret::random();
        let my_public = PublicKey::from(&my_secret);
        let share_secret1 = my_secret.diffie_hellman(&sn_x_public_key);

        let sn_x25519_sk = x25519_dalek::StaticSecret::from(private_key_x25519);
        let share_secret2 = sn_x25519_sk.diffie_hellman(&my_public);
        assert_eq!(share_secret1.to_bytes(), share_secret2.to_bytes());
    }
}
