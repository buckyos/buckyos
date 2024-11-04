
use std::time::{SystemTime, UNIX_EPOCH};

use base64::prelude::BASE64_STANDARD_NO_PAD;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use serde::{Serialize, Deserialize};
use serde_json::json;
use thiserror::*;
use ed25519_dalek::{ed25519::signature::SignerMut, SigningKey};
use ed25519_dalek::{PublicKey, Verifier};

use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use rand::rngs::OsRng;
use base64;


use base64::{engine::general_purpose::URL_SAFE_NO_PAD, engine::general_purpose::STANDARD,Engine as _};
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    my_test_name: bool,
    exp: usize,
}
/*
iss (issuer)：签发人
exp (expiration time)：过期时间
sub (subject)：主题
aud (audience)：受众
nbf (Not Before)：生效时间
iat (Issued At)：签发时间
jti (JWT ID)：编号
*/
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

type NSResult<T> = Result<T, NSError>;

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

// 辅助函数：构建 PKCS#8 格式的私钥
fn build_pkcs8(private_key: &[u8]) -> Vec<u8> {
    let mut pkcs8 = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20
    ];
    pkcs8.extend_from_slice(private_key);
    pkcs8
}

pub fn generate_key_pair() {
    let mut csprng = OsRng{};

   
    let signing_key: SigningKey = SigningKey::generate(&mut csprng);


    // 构建私钥 PEM
    let private_key_bytes = signing_key.to_bytes();
    let pkcs8_bytes = build_pkcs8(&private_key_bytes);
    let private_key_pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
        STANDARD.encode(&pkcs8_bytes)
    );



    let public_key_jwk = json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes()),
    });

    println!("Genereate Private Key (PEM): {}", private_key_pem);
    println!("Generate Public Key (JWK): {}", public_key_jwk);
}

/// 生成并加密AES密钥的函数
// pub fn generate_and_encrypt_aes_key(
//     their_ed25519_public: &PublicKey,
// ) -> Result<(Vec<u8>, [u8; 32]), Box<dyn Error>> {
    // // 生成一个随机的32字节AES密钥
    // let mut aes_key = [0u8; 32];
    // OsRng.fill_bytes(&mut aes_key);
    
    // // 将Ed25519公钥转换为X25519公钥
    // // 注意：这一步在实际使用中需要确保对方的公钥是可信的
    // let their_x25519_public = convert_ed25519_to_x25519(their_ed25519_public)?;
    
    // // 生成自己的X25519密钥对
    // let my_secret = StaticSecret::new(OsRng);
    // let my_public = x25519_dalek::PublicKey::from(&my_secret);
    
    // // 计算共享密钥
    // let shared_secret = my_secret.diffie_hellman(&their_x25519_public);
    
    // // 使用共享密钥加密AES密钥
    // let cipher = Aes256Gcm::new_from_slice(shared_secret.as_bytes())?;
    // let nonce = Nonce::from_slice(&[0u8; 12]); // 在实际使用中应该使用随机nonce
    
    // // 加密AES密钥
    // let encrypted_key = cipher
    //     .encrypt(nonce, aes_key.as_ref())
    //     .map_err(|e| KeyExchangeError::EncryptionError(e.to_string()))?;
    
    // // 返回加密后的AES密钥和我们的公钥
    // // 接收方需要这个公钥来重建共享密钥
    // let mut response = Vec::with_capacity(32 + encrypted_key.len());
    // response.extend_from_slice(my_public.as_bytes());
    // response.extend_from_slice(&encrypted_key);
    
    // Ok((response, aes_key))
// }

fn main() {
    generate_key_pair();
    let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "oDrETgXBLCjN0RS4yeIePMtrTNZV5pDNncwR6eqq6f0"
            }
        );
    println!("Public Key (JWK): {:?}", jwk);

    // Private Key (Base64URL)
    let private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIKfb6WDUJcmV0rp5AM3rdaiHuhnW4+uQNV317sVaGr2G
-----END PRIVATE KEY-----
"#;
    //create JWT
    let my_claims = Claims {
        my_test_name: true,
        exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as usize + 3600, 
    };
    let private_key: EncodingKey = EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = None; // 默认为 JWT，设置为None以节约空间
    let token = encode(&header, &my_claims, &private_key).unwrap();
    println!("JWT: {}", token);

    // verify JWT
    let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
    let import_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();
    let validation = Validation::new(Algorithm::EdDSA);
    let decoded_token = decode::<Claims>(&token, &import_key, &validation).unwrap();

    println!("JWT verify OK!");
    println!("Protected Header: {:?}", decoded_token.header);
    println!("Payload: {:?}", decoded_token.claims);

    let decoded_token2 = decode_jwt_claim_without_verify(&token);
    println!("Decoded Token2: {:?}", decoded_token2);

}

