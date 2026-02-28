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
pub const ENV_STATE_STORE_SYNC_WRITE: &str = "KLOG_STATE_STORE_SYNC_WRITE";
pub const ENV_JOIN_TARGETS: &str = "KLOG_JOIN_TARGETS";
pub const ENV_JOIN_RETRY_INTERVAL_MS: &str = "KLOG_JOIN_RETRY_INTERVAL_MS";
pub const ENV_JOIN_MAX_ATTEMPTS: &str = "KLOG_JOIN_MAX_ATTEMPTS";
pub const ENV_JOIN_BLOCKING: &str = "KLOG_JOIN_BLOCKING";
pub const ENV_JOIN_TARGET_ROLE: &str = "KLOG_JOIN_TARGET_ROLE";

const DEFAULT_NODE_ID: KNodeId = 1;
const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:21001";
const DEFAULT_ADVERTISE_ADDR: &str = "127.0.0.1";
const DEFAULT_ADVERTISE_PORT: u16 = 21001;
const DEFAULT_CLUSTER_NAME: &str = "klog";
const DEFAULT_AUTO_BOOTSTRAP: bool = true;
const DEFAULT_STATE_STORE_SYNC_WRITE: bool = true;
const DEFAULT_JOIN_RETRY_INTERVAL_MS: u64 = 3_000;
const DEFAULT_JOIN_MAX_ATTEMPTS: u32 = 0; // 0 means retry forever.
const DEFAULT_JOIN_BLOCKING: bool = false;
const DEFAULT_JOIN_TARGET_ROLE: KLogJoinTargetRole = KLogJoinTargetRole::Voter;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KLogJoinTargetRole {
    Learner,
    Voter,
}

impl KLogJoinTargetRole {
    pub fn as_str(self) -> &'static str {
        match self {
            KLogJoinTargetRole::Learner => "learner",
            KLogJoinTargetRole::Voter => "voter",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "learner" => Ok(KLogJoinTargetRole::Learner),
            "voter" => Ok(KLogJoinTargetRole::Voter),
            _ => Err("expected learner or voter".to_string()),
        }
    }
}

impl std::fmt::Display for KLogJoinTargetRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

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
    pub state_store_sync_write: bool,
    pub join_targets: Vec<String>,
    pub join_retry_interval_ms: u64,
    pub join_max_attempts: u32,
    pub join_blocking: bool,
    pub join_target_role: KLogJoinTargetRole,
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
    pub state_store_sync_write: Option<bool>,
    pub join_targets: Option<Vec<String>>,
    pub join_retry_interval_ms: Option<u64>,
    pub join_max_attempts: Option<u32>,
    pub join_blocking: Option<bool>,
    pub join_target_role: Option<KLogJoinTargetRole>,
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
            state_store_sync_write: parse_env_bool(ENV_STATE_STORE_SYNC_WRITE)?,
            join_targets: parse_env_string_list(ENV_JOIN_TARGETS)?,
            join_retry_interval_ms: parse_env_u64(ENV_JOIN_RETRY_INTERVAL_MS)?,
            join_max_attempts: parse_env_u32(ENV_JOIN_MAX_ATTEMPTS)?,
            join_blocking: parse_env_bool(ENV_JOIN_BLOCKING)?,
            join_target_role: parse_env_join_target_role(ENV_JOIN_TARGET_ROLE)?,
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
            state_store_sync_write: patch
                .state_store_sync_write
                .unwrap_or(DEFAULT_STATE_STORE_SYNC_WRITE),
            join_targets: patch.join_targets.unwrap_or_default(),
            join_retry_interval_ms: patch
                .join_retry_interval_ms
                .unwrap_or(DEFAULT_JOIN_RETRY_INTERVAL_MS),
            join_max_attempts: patch.join_max_attempts.unwrap_or(DEFAULT_JOIN_MAX_ATTEMPTS),
            join_blocking: patch.join_blocking.unwrap_or(DEFAULT_JOIN_BLOCKING),
            join_target_role: patch.join_target_role.unwrap_or(DEFAULT_JOIN_TARGET_ROLE),
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

fn parse_env_string_list(key: &str) -> Result<Option<Vec<String>>, String> {
    match std::env::var(key) {
        Ok(v) => {
            let items = v
                .split(',')
                .map(|x| x.trim())
                .filter(|x| !x.is_empty())
                .map(|x| x.to_string())
                .collect::<Vec<_>>();
            if items.is_empty() {
                Ok(None)
            } else {
                Ok(Some(items))
            }
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{} contains invalid unicode", key)),
    }
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

fn parse_env_u32(key: &str) -> Result<Option<u32>, String> {
    match parse_env_string(key)? {
        Some(v) => v
            .parse::<u32>()
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

fn parse_env_join_target_role(key: &str) -> Result<Option<KLogJoinTargetRole>, String> {
    match parse_env_string(key)? {
        Some(v) => KLogJoinTargetRole::parse(&v)
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
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
state_store_sync_write = false
join_targets = ["127.0.0.1:21001", "127.0.0.1:21002"]
join_retry_interval_ms = 1500
join_max_attempts = 9
join_blocking = true
join_target_role = "learner"
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
        assert!(!cfg.state_store_sync_write);
        assert_eq!(
            cfg.join_targets,
            vec!["127.0.0.1:21001".to_string(), "127.0.0.1:21002".to_string()]
        );
        assert_eq!(cfg.join_retry_interval_ms, 1500);
        assert_eq!(cfg.join_max_attempts, 9);
        assert!(cfg.join_blocking);
        assert_eq!(cfg.join_target_role, KLogJoinTargetRole::Learner);

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
        assert_eq!(cfg.state_store_sync_write, DEFAULT_STATE_STORE_SYNC_WRITE);
        assert!(cfg.join_targets.is_empty());
        assert_eq!(cfg.join_retry_interval_ms, DEFAULT_JOIN_RETRY_INTERVAL_MS);
        assert_eq!(cfg.join_max_attempts, DEFAULT_JOIN_MAX_ATTEMPTS);
        assert_eq!(cfg.join_blocking, DEFAULT_JOIN_BLOCKING);
        assert_eq!(cfg.join_target_role, DEFAULT_JOIN_TARGET_ROLE);

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
            state_store_sync_write: Some(false),
            join_targets: Some(vec!["10.0.0.1:21001".to_string()]),
            join_retry_interval_ms: Some(6000),
            join_max_attempts: Some(3),
            join_blocking: Some(true),
            join_target_role: Some(KLogJoinTargetRole::Learner),
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
        assert!(!cfg.state_store_sync_write);
        assert_eq!(cfg.join_targets, vec!["10.0.0.1:21001".to_string()]);
        assert_eq!(cfg.join_retry_interval_ms, 6000);
        assert_eq!(cfg.join_max_attempts, 3);
        assert!(cfg.join_blocking);
        assert_eq!(cfg.join_target_role, KLogJoinTargetRole::Learner);
    }
}
