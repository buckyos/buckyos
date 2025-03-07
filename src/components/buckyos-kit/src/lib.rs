mod path;
mod process;
mod time;
mod log_util;
mod stream;
mod json;
mod test_server;
mod serde_helper;
mod config;
mod channel;
mod event;
mod provider;

#[macro_use]
extern crate log;

pub use path::*;
pub use process::*;
pub use time::*;
pub use log_util::*;
pub use stream::*;
pub use json::*;
pub use test_server::*;
pub use serde_helper::*;
pub use config::*;
pub use channel::*;
pub use event::*;
pub use provider::*;
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
}