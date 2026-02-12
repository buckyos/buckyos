use crate::error::{into_smb_err, smb_err, SmbErrorCode, SmbResult};
#[cfg(target_os = "linux")]
use crate::linux_smb::{check_samba_status, stop_smb_service, update_samba_conf};
#[cfg(target_os = "macos")]
use crate::macos_smb::{check_samba_status, stop_smb_service, update_samba_conf};
use crate::samba::{SmbItem, SmbUserItem};
#[cfg(target_os = "windows")]
use crate::windows_smb::{check_samba_status, stop_smb_service, update_samba_conf};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType,
    SystemConfigError,
};
use buckyos_kit::{get_buckyos_root_dir, get_version};
use clap::Command;
use fs2::FileExt;
use sfo_log::Logger;
use std::fs::File;
use std::sync::OnceLock;
use std::time::Duration;
use sysinfo::{ProcessesToUpdate, System};

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct UserSambaInfo {
    password: String,
    is_enable: bool,
}

#[cfg(target_os = "linux")]
mod linux_smb;

#[cfg(target_os = "windows")]
mod windows_smb;

#[cfg(target_os = "macos")]
mod macos_smb;

mod error;
mod samba;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct UserInfo {
    #[serde(rename = "type")]
    ty: String,
    username: String,
    password: String,
}

static PROC_LOCK: OnceLock<File> = OnceLock::new();
fn check_process_exist(name: &str) -> bool {
    let lock_file = std::env::temp_dir().join(format!("{}.lock", name));
    let lock_file = match File::create(lock_file) {
        Ok(file) => file,
        Err(e) => {
            log::error!("create lock file failed: {}", e);
            return false;
        }
    };

    if lock_file.try_lock_exclusive().is_err() {
        true
    } else {
        PROC_LOCK.get_or_init(|| lock_file);
        false
    }
}

async fn async_main() {
    let matches = Command::new("smb-service")
        .version(get_version())
        .subcommand(Command::new("start"))
        .subcommand(Command::new("stop"))
        .subcommand(Command::new("status"))
        .get_matches();

    match matches.subcommand() {
        Some(("start", _)) => {
            if check_process_exist("smb_service") {
                println!("smb_service is already running");
                return;
            }
            Logger::new("smb-service")
                .set_log_path(
                    get_buckyos_root_dir()
                        .join("logs")
                        .join("smb")
                        .to_string_lossy()
                        .to_string()
                        .as_str(),
                )
                .set_log_to_file(true)
                .set_log_file_count(5)
                .set_log_level("info")
                .start()
                .unwrap();

            std::panic::set_hook(Box::new(|panic_info| {
                if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                    sfo_log::error!("panic occurred: {s}");
                } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                    sfo_log::error!("panic occurred: {s}");
                } else {
                    sfo_log::error!("panic occurred");
                }
                // sfo_log::error!("panic: {:?}", panic_info);
            }));
            let mut runtime = match init_buckyos_api_runtime(
                "smb-service",
                None,
                BuckyOSRuntimeType::KernelService,
            )
            .await
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    log::error!("init_buckyos_api_runtime failed: {}", e);
                    return;
                }
            };
            if let Err(e) = runtime.login().await {
                log::error!("login failed: {}", e);
                return;
            }
            set_buckyos_api_runtime(runtime);

            enter_update_smb_loop().await;
        }
        Some(("stop", _)) => {
            if let Err(e) = stop_service().await {
                log::error!("stop service failed {}", e);
            }
        }
        Some(("status", _)) => match check_status().await {
            Ok(ret) => {
                println!("status {}", ret);
                std::process::exit(ret);
            }
            Err(e) => {
                println!("check status failed: {}", e);
                std::process::exit(1);
            }
        },
        _ => unreachable!(),
    }
}

async fn enter_update_smb_loop() {
    let mut is_first = true;
    loop {
        if let Err(e) = check_and_update_smb_service(is_first).await {
            log::error!("check_and_update_smb_service failed: {}", e);
        } else {
            is_first = false;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn check_and_update_smb_service(is_first: bool) -> SmbResult<()> {
    let system_config_client = get_buckyos_api_runtime()
        .unwrap()
        .get_system_config_client()
        .await
        .map_err(into_smb_err!(
            SmbErrorCode::Failed,
            "get system config client failed"
        ))?;

    let mut latest_smb_items = match system_config_client
        .get("services/smb-service/latest_smb_items")
        .await
    {
        Ok(latest_smb_items_str) => serde_json::from_str(latest_smb_items_str.value.as_str())
            .map_err(into_smb_err!(
                SmbErrorCode::Failed,
                "parse latest_smb_items failed"
            ))?,
        Err(e) => {
            if let SystemConfigError::KeyNotFound(_path) = e {
                Vec::new()
            } else {
                return Err(smb_err!(
                    SmbErrorCode::Failed,
                    "get latest_smb_items failed: {}",
                    e
                ));
            }
        }
    };
    let mut latest_users = match system_config_client
        .get("services/smb-service/latest_users")
        .await
    {
        Ok(latest_users_str) => serde_json::from_str(latest_users_str.value.as_str()).map_err(
            into_smb_err!(SmbErrorCode::Failed, "parse latest_users failed"),
        )?,
        Err(e) => {
            if let SystemConfigError::KeyNotFound(_path) = e {
                Vec::new()
            } else {
                return Err(smb_err!(
                    SmbErrorCode::Failed,
                    "get latest_smb_items failed: {}",
                    e
                ));
            }
        }
    };

    let list = system_config_client
        .list("users")
        .await
        .map_err(into_smb_err!(
            SmbErrorCode::ListUserFailed,
            "get user list failed"
        ))?;
    let mut smb_items = Vec::new();
    let mut smb_users = Vec::new();
    let mut root_users = Vec::new();
    for user in list {
        let buckyos_user_settings = match system_config_client
            .get(format!("users/{}/settings", user).as_str())
            .await
        {
            Ok(get_result) => {
                let info: UserInfo = serde_json::from_str(get_result.value.as_str()).map_err(
                    into_smb_err!(SmbErrorCode::Failed, "parse user info failed"),
                )?;
                info
            }
            Err(e) => {
                if let SystemConfigError::KeyNotFound(_path) = e {
                    log::debug!("user {} samba_info not found", user);
                    continue;
                } else {
                    return Err(smb_err!(
                        SmbErrorCode::Failed,
                        "get samba_info failed: {}",
                        e
                    ));
                }
            }
        };

        let user_info = match system_config_client
            .get(format!("users/{}/samba/settings", user).as_str())
            .await
        {
            Ok(get_result) => {
                let samba_info: UserSambaInfo = serde_json::from_str(get_result.value.as_str())
                    .map_err(into_smb_err!(
                        SmbErrorCode::Failed,
                        "parse samba_info failed"
                    ))?;
                samba_info
            }
            Err(e) => {
                if let SystemConfigError::KeyNotFound(_path) = e {
                    log::debug!("user {} samba_info not found", user);
                    continue;
                } else {
                    return Err(smb_err!(
                        SmbErrorCode::Failed,
                        "get samba_info failed: {}",
                        e
                    ));
                }
            }
        };
        if !user_info.is_enable || user_info.password.is_empty() {
            log::debug!("user {} samba is not enable or password is empty", user);
            continue;
        }

        if user == "root" {
            root_users.push(user.clone());
        }

        let user_home = get_buckyos_root_dir()
            .join("data")
            .join(buckyos_user_settings.username.as_str())
            .join("home");
        if !user_home.exists() {
            //create user home
            std::fs::create_dir_all(user_home.clone()).map_err(into_smb_err!(
                SmbErrorCode::Failed,
                "create user home failed"
            ))?;
        }
        // 转换成绝对路径
        let user_home = std::fs::canonicalize(user_home).map_err(into_smb_err!(
            SmbErrorCode::Failed,
            "canonicalize user home failed"
        ))?;
        let smb_item = SmbItem {
            smb_name: format!("{} Home", buckyos_user_settings.username.as_str()),
            allow_users: vec![buckyos_user_settings.username.clone()],
            path: user_home.to_string_lossy().to_string(),
        };
        smb_items.push(smb_item);
        smb_users.push(SmbUserItem {
            user,
            password: user_info.password,
        });
    }

    for root_user in root_users {
        for smb_item in smb_items.iter_mut() {
            if !smb_item.allow_users.contains(&root_user) {
                smb_item.allow_users.push(root_user.clone());
            }
        }
    }

    let mut delete_users = Vec::new();
    for user in latest_users.iter() {
        if !smb_users.contains(user) {
            delete_users.push(user.clone());
        }
    }

    let mut delete_smb_items = Vec::new();
    for smb_item in latest_smb_items.iter() {
        if !smb_items.contains(smb_item) {
            delete_smb_items.push(smb_item.clone());
        }
    }

    let new_users = smb_users
        .iter()
        .filter(|user| !latest_users.contains(user))
        .collect::<Vec<_>>();
    let new_smb_items = smb_items
        .iter()
        .filter(|smb_item| !latest_smb_items.contains(smb_item))
        .collect::<Vec<_>>();
    if !is_first {
        if new_users.is_empty()
            && new_smb_items.is_empty()
            && delete_users.is_empty()
            && delete_smb_items.is_empty()
        {
            log::info!("samba config no change");
            return Ok(());
        }
    }

    update_samba_conf(
        delete_users,
        smb_users.clone(),
        delete_smb_items,
        smb_items.clone(),
    )
    .await
    .map_err(into_smb_err!(
        SmbErrorCode::Failed,
        "update_samba_conf failed"
    ))?;

    latest_smb_items = smb_items;
    latest_users = smb_users;
    system_config_client
        .set(
            "services/smb-service/latest_users",
            serde_json::to_string(&latest_users).unwrap().as_str(),
        )
        .await
        .map_err(into_smb_err!(
            SmbErrorCode::Failed,
            "set latest_users failed"
        ))?;
    system_config_client
        .set(
            "services/smb-service/latest_smb_items",
            serde_json::to_string(&latest_smb_items).unwrap().as_str(),
        )
        .await
        .map_err(into_smb_err!(
            SmbErrorCode::Failed,
            "set latest_smb_items failed"
        ))?;

    Ok(())
}

async fn stop_service() -> SmbResult<()> {
    stop_smb_service().await?;

    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);

    for process in system.processes_by_name("smb_service".as_ref()) {
        for param in process.cmd().iter() {
            if param == "start" {
                process.kill();
            }
        }
    }

    Ok(())
}

async fn check_status() -> SmbResult<i32> {
    let ret = check_samba_status().await;
    if ret != 0 {
        return Ok(ret);
    }
    let mut system = System::new_all();
    system.refresh_all();

    let mut is_smb_service_running = false;
    for process in system.processes_by_name("smb_service".as_ref()) {
        for param in process.cmd().iter() {
            if param == "start" {
                is_smb_service_running = true;
                break;
            }
        }
    }

    if is_smb_service_running {
        Ok(0)
    } else {
        Ok(1)
    }
}

fn main() {
    if num_cpus::get() < 2 {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async_main());
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async_main());
    }
}
