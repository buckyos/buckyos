use std::{env, path::{Path, PathBuf}};

pub fn get_buckyos_root_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        let user_data_dir = env::var("APPDATA").unwrap_or_else(|_| {
            env::var("USERPROFILE").unwrap_or_else(|_| ".".to_string())
        });
        Path::new(&user_data_dir).join("buckyos")
    } else {
        Path::new("/opt/buckyos").to_path_buf()
    }
}

pub fn get_buckyos_system_bin_dir() -> PathBuf {
    get_buckyos_root_dir().join("bin")
}

pub fn get_buckyos_system_etc_dir() -> PathBuf {
    get_buckyos_root_dir().join("etc")
}

pub fn get_buckyos_system_config_service_path() -> PathBuf {
    get_buckyos_system_bin_dir().join("sys_config_service")
}