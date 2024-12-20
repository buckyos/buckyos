use sfo_log::Logger;
use buckyos_kit::get_buckyos_root_dir;
use sys_config::SystemConfigClient;

#[cfg(target_os = "linux")]
mod linux_smb;
mod error;

#[cfg(target_os = "windows")]
mod windows_smb;

#[tokio::main]
async fn main() {
    Logger::new("smb-service")
        .set_log_path(get_buckyos_root_dir().join("logs").to_string_lossy().to_string().as_str())
        .set_log_to_file(true)
        .set_log_level("info")
        .start().unwrap();

    let rpc_session_token_str = std::env::var("SMB_SESSION_TOKEN");
    if rpc_session_token_str.is_err() {
        log::error!("SMB_SESSION_TOKEN is not set");
        return;
    }

    let rpc_session_token = rpc_session_token_str.unwrap();
    let system_config_client = SystemConfigClient::new(None,Some(rpc_session_token.as_str()));

    #[cfg(target_os = "linux")]
    {
    }
}
