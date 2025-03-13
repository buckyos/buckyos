use std::{env, path::{Path, PathBuf}};

pub fn get_buckyos_root_dir() -> PathBuf {
    if env::var("BUCKYOS_ROOT").is_ok() {
        return Path::new(&env::var("BUCKYOS_ROOT").unwrap()).to_path_buf();
    }

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

pub fn get_buckyos_log_dir(service: &str) -> PathBuf {
    get_buckyos_root_dir().join("logs").join(service)
}

pub fn get_buckyos_service_data_dir(service_name: &str) -> PathBuf {
    get_buckyos_root_dir().join("data").join(service_name)
}

pub fn get_buckyos_service_local_data_dir(service_name: &str,disk_id: Option<&str>) -> PathBuf {
    if disk_id.is_some() {
        get_buckyos_root_dir().join("local").join(disk_id.unwrap()).join(service_name)
    } else {
        get_buckyos_root_dir().join("local").join(service_name)
    }
}

pub fn adjust_path(old_path: &str) -> std::io::Result<PathBuf> {
    let new_path= old_path.replace("{BUCKYOS_ROOT}", &get_buckyos_root_dir().to_string_lossy());
    // can adjust other Placeholders

    std::path::absolute(new_path)?.canonicalize()
}

pub fn get_buckyos_named_data_dir(mgr_id: Option<&str>) -> PathBuf {
    if mgr_id.is_none() {
        get_buckyos_root_dir().join("data").join("ndn")
    } else {
        get_buckyos_root_dir().join("data").join("ndn").join(mgr_id.unwrap())
    }
}

pub fn get_relative_path(base_path: &str, full_path: &str) -> String {
    if full_path.starts_with(base_path) {
        if base_path.ends_with('/') {
            full_path[base_path.len()-1..].to_string()
        } else {
            full_path[base_path.len()..].to_string()
        }
    } else {
        full_path.to_string()
    }
}

pub fn path_join(base: &str, sub_path: &str) -> PathBuf {
    PathBuf::from(base).join(sub_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_relative_path() {
        let base_path = "/opt/buckyos/data/chunk";
        let full_path = "/opt/buckyos/data/chunk/1234567890";
        let relative_path = get_relative_path(base_path, full_path);
        assert_eq!(relative_path, "/1234567890");

        let base_path = "/opt/buckyos/data/chunk/";
        let full_path = "/opt/buckyos/data/chunk/1234567890/asdf?a=1&b=2";
        let relative_path = get_relative_path(base_path, full_path);
        assert_eq!(relative_path, "/1234567890/asdf?a=1&b=2");

    }
}
