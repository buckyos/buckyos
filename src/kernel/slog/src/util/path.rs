use std::path::{Path, PathBuf};

pub fn get_buckyos_root_dir() -> PathBuf {
    if std::env::var("BUCKYOS_ROOT").is_ok() {
        return Path::new(&std::env::var("BUCKYOS_ROOT").unwrap()).to_path_buf();
    }

    if cfg!(target_os = "windows") {
        Path::new(r"C:\buckyos").to_path_buf()
    } else {
        Path::new("/opt/buckyos").to_path_buf()
    }
}

// Get the root log directory for buckyos service logs
pub fn get_buckyos_log_root_dir() -> PathBuf {
    get_buckyos_root_dir().join("logs")
}
