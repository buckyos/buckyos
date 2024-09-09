mod path;
mod process;
mod time;
pub use path::*;
pub use process::*;
pub use time::*;

use serde_json::json;
use ed25519_dalek::{SigningKey};
use rand::rngs::OsRng;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
pub fn generate_ed25519_key_pair() -> (String, serde_json::Value) {
    let mut csprng = OsRng{};

    let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    let private_key_pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
        URL_SAFE_NO_PAD.encode(signing_key.to_bytes())
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
    use super::*;
    use env_logger;
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
        let mut pkg = ServicePkg::new(pkg_id, env_path);
        pkg.start().await.unwrap();
    }
}