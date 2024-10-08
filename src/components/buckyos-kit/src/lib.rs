mod path;
mod process;
mod time;
mod config;
mod log_util;

pub use path::*;
pub use process::*;
pub use time::*;
pub use log_util::*;

use serde_json::json;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, engine::general_purpose::STANDARD,Engine as _};

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
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
        STANDARD.encode(&pkcs8_bytes)
    );

    let public_key_jwk = json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes()),
    });

    (private_key_pem, public_key_jwk)
}


#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use super::*;
    use env_logger;

    #[test]
    fn test_generate_ed25519_key_pair() {
        let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
        println!("private_key_pem: {}", private_key_pem);
        println!("public_key_jwk: {}", public_key_jwk);
    }

    #[test]
    fn test_get_unix_timestamp() {
        let now = std::time::SystemTime::now();
        let unix_time = now.duration_since(std::time::UNIX_EPOCH).unwrap();
        assert_eq!(buckyos_get_unix_timestamp(), unix_time.as_secs());
    }

    #[tokio::test]
    async fn test_execute() {
        let path = "d:\\temp\\test";
        let args = vec![];
        let result = execute(&PathBuf::from(path), 5, Some(&args), None, None).await;
        match result {
            Ok((exit_code, output)) => {
                println!("Exit code: {}", exit_code);
                println!("Output: {}", String::from_utf8_lossy(&output));
            }
            Err(e) => println!("Error: {:?}", e),
        }

        // Uncomment and modify the following lines to test with notepad.exe
        // let path = "C:\\Windows\\System32\\notepad.exe";
        // let args = vec![];
        // let result = execute(&PathBuf::from(path), 5, Some(&args), None, None).await;
        // match result {
        //     Ok((exit_code, output)) => {
        //         println!("Exit code: {}", exit_code);
        //         println!("Output: {}", String::from_utf8_lossy(&output));
        //     }
        //     Err(e) => println!("Error: {:?}", e),
        // }
    }
    #[tokio::test]
    async fn test_execute_service_pkg() {
        // 初始化日志系统
        let _ = env_logger::builder().is_test(true).try_init();

        let pkg_id = "test2".to_string();
        let env_path = PathBuf::from("d:\\temp\\");
        let pkg = ServicePkg::new(pkg_id, env_path);
        pkg.start(None).await.unwrap();
    }
}