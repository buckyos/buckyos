use buckyos_kit::get_buckyos_service_data_dir;
use klog::KNodeId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const ENV_CONFIG_FILE: &str = "KLOG_CONFIG_FILE";
pub const ENV_NODE_ID: &str = "KLOG_NODE_ID";
pub const ENV_LISTEN_ADDR: &str = "KLOG_LISTEN_ADDR";
pub const ENV_ADVERTISE_ADDR: &str = "KLOG_ADVERTISE_ADDR";
pub const ENV_ADVERTISE_PORT: &str = "KLOG_ADVERTISE_PORT";
pub const ENV_DATA_DIR: &str = "KLOG_DATA_DIR";
pub const ENV_CLUSTER_NAME: &str = "KLOG_CLUSTER_NAME";
pub const ENV_AUTO_BOOTSTRAP: &str = "KLOG_AUTO_BOOTSTRAP";

const DEFAULT_NODE_ID: KNodeId = 1;
const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:21001";
const DEFAULT_ADVERTISE_ADDR: &str = "127.0.0.1";
const DEFAULT_ADVERTISE_PORT: u16 = 21001;
const DEFAULT_CLUSTER_NAME: &str = "klog";
const DEFAULT_AUTO_BOOTSTRAP: bool = true;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KLogRuntimeConfigSource {
    Env,
    File(PathBuf),
    Buckyos,
}

impl std::fmt::Display for KLogRuntimeConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Env => write!(f, "env"),
            Self::File(path) => write!(f, "file({})", path.display()),
            Self::Buckyos => write!(f, "buckyos"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KLogRuntimeConfig {
    pub node_id: KNodeId,
    pub listen_addr: String,
    pub advertise_addr: String,
    pub advertise_port: u16,
    pub data_dir: PathBuf,
    pub cluster_name: String,
    pub auto_bootstrap: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KLogRuntimeConfigPatch {
    pub node_id: Option<KNodeId>,
    pub listen_addr: Option<String>,
    pub advertise_addr: Option<String>,
    pub advertise_port: Option<u16>,
    pub data_dir: Option<PathBuf>,
    pub cluster_name: Option<String>,
    pub auto_bootstrap: Option<bool>,
}

// Placeholder type for future buckyos config integration.
// It intentionally mirrors daemon runtime fields to keep conversion simple.
pub type BuckyosKlogConfig = KLogRuntimeConfigPatch;

impl KLogRuntimeConfig {
    pub fn load() -> Result<(Self, KLogRuntimeConfigSource), String> {
        match std::env::var(ENV_CONFIG_FILE) {
            Ok(path_raw) => {
                let path = path_raw.trim();
                if path.is_empty() {
                    return Err(format!("{} is set but empty", ENV_CONFIG_FILE));
                }

                let path = PathBuf::from(path);
                let cfg = Self::from_file(&path)?;
                Ok((cfg, KLogRuntimeConfigSource::File(path)))
            }
            Err(std::env::VarError::NotPresent) => {
                let cfg = Self::from_env()?;
                Ok((cfg, KLogRuntimeConfigSource::Env))
            }
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(format!("{} contains invalid unicode", ENV_CONFIG_FILE))
            }
        }
    }

    pub fn from_env() -> Result<Self, String> {
        let patch = KLogRuntimeConfigPatch {
            node_id: parse_env_u64(ENV_NODE_ID)?,
            listen_addr: parse_env_string(ENV_LISTEN_ADDR)?,
            advertise_addr: parse_env_string(ENV_ADVERTISE_ADDR)?,
            advertise_port: parse_env_u16(ENV_ADVERTISE_PORT)?,
            data_dir: parse_env_pathbuf(ENV_DATA_DIR)?,
            cluster_name: parse_env_string(ENV_CLUSTER_NAME)?,
            auto_bootstrap: parse_env_bool(ENV_AUTO_BOOTSTRAP)?,
        };

        Ok(Self::from_patch(patch))
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path_ref = path.as_ref();
        let content = std::fs::read_to_string(path_ref).map_err(|e| {
            format!(
                "Failed to read klog config file {}: {}",
                path_ref.display(),
                e
            )
        })?;

        let patch: KLogRuntimeConfigPatch = toml::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse klog config file {}: {}",
                path_ref.display(),
                e
            )
        })?;

        Ok(Self::from_patch(patch))
    }

    pub fn from_buckyos_config(cfg: &BuckyosKlogConfig) -> Result<Self, String> {
        Ok(Self::from_patch(cfg.clone()))
    }

    pub fn load_from_buckyos(
        cfg: &BuckyosKlogConfig,
    ) -> Result<(Self, KLogRuntimeConfigSource), String> {
        let config = Self::from_buckyos_config(cfg)?;
        Ok((config, KLogRuntimeConfigSource::Buckyos))
    }

    fn from_patch(patch: KLogRuntimeConfigPatch) -> Self {
        let node_id = patch.node_id.unwrap_or(DEFAULT_NODE_ID);
        let default_data_dir = default_data_dir();

        Self {
            node_id,
            listen_addr: patch
                .listen_addr
                .unwrap_or_else(|| DEFAULT_LISTEN_ADDR.to_string()),
            advertise_addr: patch
                .advertise_addr
                .unwrap_or_else(|| DEFAULT_ADVERTISE_ADDR.to_string()),
            advertise_port: patch.advertise_port.unwrap_or(DEFAULT_ADVERTISE_PORT),
            data_dir: patch.data_dir.unwrap_or(default_data_dir),
            cluster_name: patch
                .cluster_name
                .unwrap_or_else(|| DEFAULT_CLUSTER_NAME.to_string()),
            auto_bootstrap: patch.auto_bootstrap.unwrap_or(DEFAULT_AUTO_BOOTSTRAP),
        }
    }
}

fn default_data_dir() -> PathBuf {
    get_buckyos_service_data_dir("klog")
}

fn parse_env_string(key: &str) -> Result<Option<String>, String> {
    match std::env::var(key) {
        Ok(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{} contains invalid unicode", key)),
    }
}

fn parse_env_pathbuf(key: &str) -> Result<Option<PathBuf>, String> {
    let value = parse_env_string(key)?;
    Ok(value.map(PathBuf::from))
}

fn parse_env_u64(key: &str) -> Result<Option<u64>, String> {
    match parse_env_string(key)? {
        Some(v) => v
            .parse::<u64>()
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_u16(key: &str) -> Result<Option<u16>, String> {
    match parse_env_string(key)? {
        Some(v) => v
            .parse::<u16>()
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_bool(key: &str) -> Result<Option<bool>, String> {
    match parse_env_string(key)? {
        Some(v) => {
            let s = v.to_ascii_lowercase();
            let parsed = match s.as_str() {
                "1" | "true" | "yes" | "y" | "on" => true,
                "0" | "false" | "no" | "n" | "off" => false,
                _ => {
                    return Err(format!("Invalid {}='{}': expected true/false", key, v));
                }
            };
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_file(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "buckyos_klog_daemon_cfg_{}_{}_{}.toml",
            std::process::id(),
            nanos,
            name
        ))
    }

    #[test]
    fn test_from_file_full_values() {
        let file = unique_test_file("full");
        let content = r#"
node_id = 9
listen_addr = "0.0.0.0:22001"
advertise_addr = "10.2.3.4"
advertise_port = 22001
data_dir = "/tmp/klog_cfg_test_full"
cluster_name = "cluster_a"
auto_bootstrap = false
"#;
        std::fs::write(&file, content).expect("write file");

        let cfg = KLogRuntimeConfig::from_file(&file).expect("parse file");
        assert_eq!(cfg.node_id, 9);
        assert_eq!(cfg.listen_addr, "0.0.0.0:22001");
        assert_eq!(cfg.advertise_addr, "10.2.3.4");
        assert_eq!(cfg.advertise_port, 22001);
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/klog_cfg_test_full"));
        assert_eq!(cfg.cluster_name, "cluster_a");
        assert!(!cfg.auto_bootstrap);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_partial_values() {
        let file = unique_test_file("partial");
        let content = r#"
node_id = 7
advertise_addr = "192.168.2.7"
"#;
        std::fs::write(&file, content).expect("write file");

        let cfg = KLogRuntimeConfig::from_file(&file).expect("parse file");
        assert_eq!(cfg.node_id, 7);
        assert_eq!(cfg.listen_addr, DEFAULT_LISTEN_ADDR);
        assert_eq!(cfg.advertise_addr, "192.168.2.7");
        assert_eq!(cfg.advertise_port, DEFAULT_ADVERTISE_PORT);
        assert_eq!(cfg.data_dir, default_data_dir());
        assert_eq!(cfg.cluster_name, DEFAULT_CLUSTER_NAME);
        assert_eq!(cfg.auto_bootstrap, DEFAULT_AUTO_BOOTSTRAP);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_buckyos_patch() {
        let patch = BuckyosKlogConfig {
            node_id: Some(3),
            listen_addr: Some("0.0.0.0:23001".to_string()),
            advertise_addr: Some("172.20.0.3".to_string()),
            advertise_port: Some(23001),
            data_dir: None,
            cluster_name: Some("bk".to_string()),
            auto_bootstrap: Some(true),
        };

        let (cfg, source) =
            KLogRuntimeConfig::load_from_buckyos(&patch).expect("build from buckyos patch");
        assert_eq!(source, KLogRuntimeConfigSource::Buckyos);
        assert_eq!(cfg.node_id, 3);
        assert_eq!(cfg.listen_addr, "0.0.0.0:23001");
        assert_eq!(cfg.advertise_addr, "172.20.0.3");
        assert_eq!(cfg.advertise_port, 23001);
        assert_eq!(cfg.data_dir, default_data_dir());
        assert_eq!(cfg.cluster_name, "bk");
        assert!(cfg.auto_bootstrap);
    }
}
