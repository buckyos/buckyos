
use tokio::time::{timeout, Duration};
use tokio::process::Command;
use log::*;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServiceControlError {
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("No permission: {0}")]
    NoPermission(String),
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("Timeout: {0}")]
    Timeout(String),
}

type Result<T> = std::result::Result<T, ServiceControlError>;

pub async fn run_script_with_args(script_path: &str, timeout_secs:u64, args: &Option<Vec<String>>) -> Result<i32> {
    let mut command = Command::new("bash");
    command.arg(script_path);
    match args {
        Some(args) => {
            for arg in args {
                command.arg(arg);
            }
        }
        None => {}
    }

    let result = timeout(
        Duration::from_secs(timeout_secs), 
        command.output()).await;

    match result {
        Ok(Ok(output)) => {
            let status_code = output.status.code().unwrap_or(-1);  
            Ok(status_code)  
        }
        Ok(Err(e)) => Err(ServiceControlError::ReasonError(e.to_string())),  
        Err(_) => Err(ServiceControlError::Timeout("Script execution timed out".to_string())), 
    }


}