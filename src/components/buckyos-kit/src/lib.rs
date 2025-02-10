mod path;
mod process;
mod time;
mod log_util;
mod stream;
mod json;
mod test_server;
mod serde_helper;
pub use path::*;
pub use process::*;
pub use time::*;
pub use log_util::*;
pub use stream::*;
pub use json::*;
pub use test_server::*;
pub use serde_helper::*;
#[cfg(test)]
mod test {
    use std::path::PathBuf;

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
        let pkg = ServicePkg::new(pkg_id, env_path);
        pkg.start(None).await.unwrap();
    }
}