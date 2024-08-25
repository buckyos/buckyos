
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use serde::{Serialize, Deserialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    my_test_name: bool,
    exp: usize,
}


fn main() {
    // Public Key (JWK) 格式化

    let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"
            }
        );
    println!("Public Key (JWK): {:?}", jwk);

    // Private Key (Base64URL)
    let private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;
    // 创建 JWT
    let my_claims = Claims {
        my_test_name: true,
        exp: 1724625212, 
    };
    let private_key = EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = None; // 默认为 JWT，设置为None以节约空间
    let token = encode(&header, &my_claims, &private_key).unwrap();
    println!("JWT: {}", token);

    // 验证 JWT
    let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
    let import_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();
    let validation = Validation::new(Algorithm::EdDSA);
    let decoded_token = decode::<Claims>(&token, &import_key, &validation).unwrap();

    println!("JWT 验证成功");
    println!("Protected Header: {:?}", decoded_token.header.alg);
    println!("Payload: {:?}", decoded_token.claims);
}

