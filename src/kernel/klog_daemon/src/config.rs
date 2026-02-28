use buckyos_kit::get_buckyos_service_data_dir;
use klog::KNodeId;
use log::error;
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
pub const ENV_ADMIN_LOCAL_ONLY: &str = "KLOG_ADMIN_LOCAL_ONLY";

const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:21001";
const DEFAULT_ADVERTISE_ADDR: &str = "127.0.0.1";
const DEFAULT_ADVERTISE_PORT: u16 = 21001;
const DEFAULT_CLUSTER_NAME: &str = "klog";
const DEFAULT_AUTO_BOOTSTRAP: bool = false;
const DEFAULT_STATE_STORE_SYNC_WRITE: bool = true;
const DEFAULT_JOIN_RETRY_INTERVAL_MS: u64 = 3_000;
const DEFAULT_JOIN_MAX_ATTEMPTS: u32 = 0; // 0 means retry forever.
const DEFAULT_JOIN_BLOCKING: bool = false;
const DEFAULT_JOIN_TARGET_ROLE: KLogJoinTargetRole = KLogJoinTargetRole::Voter;
const DEFAULT_ADMIN_LOCAL_ONLY: bool = true;

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
    pub admin_local_only: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogNetworkConfigPatch {
    pub listen_addr: Option<String>,
    pub advertise_addr: Option<String>,
    pub advertise_port: Option<u16>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogStorageConfigPatch {
    pub data_dir: Option<PathBuf>,
    pub state_store_sync_write: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogClusterConfigPatch {
    pub name: Option<String>,
    pub auto_bootstrap: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogJoinConfigPatch {
    pub targets: Option<Vec<String>>,
    pub retry_interval_ms: Option<u64>,
    pub max_attempts: Option<u32>,
    pub blocking: Option<bool>,
    pub target_role: Option<KLogJoinTargetRole>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogAdminConfigPatch {
    pub local_only: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogRuntimeConfigPatch {
    pub network: Option<KLogNetworkConfigPatch>,
    pub storage: Option<KLogStorageConfigPatch>,
    pub cluster: Option<KLogClusterConfigPatch>,
    pub join: Option<KLogJoinConfigPatch>,
    pub admin: Option<KLogAdminConfigPatch>,
    pub node_id: Option<KNodeId>,
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
            network: Some(KLogNetworkConfigPatch {
                listen_addr: parse_env_string(ENV_LISTEN_ADDR)?,
                advertise_addr: parse_env_string(ENV_ADVERTISE_ADDR)?,
                advertise_port: parse_env_u16(ENV_ADVERTISE_PORT)?,
            }),
            storage: Some(KLogStorageConfigPatch {
                data_dir: parse_env_pathbuf(ENV_DATA_DIR)?,
                state_store_sync_write: parse_env_bool(ENV_STATE_STORE_SYNC_WRITE)?,
            }),
            cluster: Some(KLogClusterConfigPatch {
                name: parse_env_string(ENV_CLUSTER_NAME)?,
                auto_bootstrap: parse_env_bool(ENV_AUTO_BOOTSTRAP)?,
            }),
            join: Some(KLogJoinConfigPatch {
                targets: parse_env_string_list(ENV_JOIN_TARGETS)?,
                retry_interval_ms: parse_env_u64(ENV_JOIN_RETRY_INTERVAL_MS)?,
                max_attempts: parse_env_u32(ENV_JOIN_MAX_ATTEMPTS)?,
                blocking: parse_env_bool(ENV_JOIN_BLOCKING)?,
                target_role: parse_env_join_target_role(ENV_JOIN_TARGET_ROLE)?,
            }),
            admin: Some(KLogAdminConfigPatch {
                local_only: parse_env_bool(ENV_ADMIN_LOCAL_ONLY)?,
            }),
            ..Default::default()
        };

        Self::from_patch(patch)
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

        Self::from_patch(patch)
    }

    pub fn from_buckyos_config(cfg: &BuckyosKlogConfig) -> Result<Self, String> {
        Self::from_patch(cfg.clone())
    }

    pub fn load_from_buckyos(
        cfg: &BuckyosKlogConfig,
    ) -> Result<(Self, KLogRuntimeConfigSource), String> {
        let config = Self::from_buckyos_config(cfg)?;
        Ok((config, KLogRuntimeConfigSource::Buckyos))
    }

    fn from_patch(patch: KLogRuntimeConfigPatch) -> Result<Self, String> {
        let KLogRuntimeConfigPatch {
            network,
            storage,
            cluster,
            join,
            admin,
            node_id,
        } = patch;

        let network = network.unwrap_or_default();
        let storage = storage.unwrap_or_default();
        let cluster = cluster.unwrap_or_default();
        let join = join.unwrap_or_default();
        let admin = admin.unwrap_or_default();

        let node_id = match node_id {
            Some(v) => v,
            None => {
                let msg = "Missing required field: node_id (or KLOG_NODE_ID)".to_string();
                error!("{}", msg);
                return Err(msg);
            }
        };
        if node_id == 0 {
            let msg = "Invalid node_id=0: node_id must be greater than 0".to_string();
            error!("{}", msg);
            return Err(msg);
        }

        let join_targets = join.targets.unwrap_or_default();
        let auto_bootstrap = cluster.auto_bootstrap.unwrap_or(DEFAULT_AUTO_BOOTSTRAP);
        if auto_bootstrap && !join_targets.is_empty() {
            let msg = "Invalid config: auto_bootstrap=true must not be combined with non-empty join.targets".to_string();
            error!("{}", msg);
            return Err(msg);
        }

        let default_data_dir = default_data_dir();

        Ok(Self {
            node_id,
            listen_addr: network
                .listen_addr
                .unwrap_or_else(|| DEFAULT_LISTEN_ADDR.to_string()),
            advertise_addr: network
                .advertise_addr
                .unwrap_or_else(|| DEFAULT_ADVERTISE_ADDR.to_string()),
            advertise_port: network.advertise_port.unwrap_or(DEFAULT_ADVERTISE_PORT),
            data_dir: storage.data_dir.unwrap_or(default_data_dir),
            cluster_name: cluster
                .name
                .unwrap_or_else(|| DEFAULT_CLUSTER_NAME.to_string()),
            auto_bootstrap,
            state_store_sync_write: storage
                .state_store_sync_write
                .unwrap_or(DEFAULT_STATE_STORE_SYNC_WRITE),
            join_targets,
            join_retry_interval_ms: join
                .retry_interval_ms
                .unwrap_or(DEFAULT_JOIN_RETRY_INTERVAL_MS),
            join_max_attempts: join.max_attempts.unwrap_or(DEFAULT_JOIN_MAX_ATTEMPTS),
            join_blocking: join.blocking.unwrap_or(DEFAULT_JOIN_BLOCKING),
            join_target_role: join.target_role.unwrap_or(DEFAULT_JOIN_TARGET_ROLE),
            admin_local_only: admin.local_only.unwrap_or(DEFAULT_ADMIN_LOCAL_ONLY),
        })
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

[network]
listen_addr = "0.0.0.0:22001"
advertise_addr = "10.2.3.4"
advertise_port = 22001

[storage]
data_dir = "/tmp/klog_cfg_test_full"
state_store_sync_write = false

[cluster]
name = "cluster_a"
auto_bootstrap = false

[join]
targets = ["127.0.0.1:21001", "127.0.0.1:21002"]
retry_interval_ms = 1500
max_attempts = 9
blocking = true
target_role = "learner"

[admin]
local_only = false
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
        assert!(!cfg.admin_local_only);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_legacy_flat_values_rejected() {
        let file = unique_test_file("legacy_flat_rejected");
        let content = r#"
node_id = 6
listen_addr = "0.0.0.0:26001"
advertise_addr = "10.0.0.6"
advertise_port = 26001
data_dir = "/tmp/klog_cfg_test_legacy"
cluster_name = "legacy_cluster"
auto_bootstrap = false
state_store_sync_write = false
join_targets = ["127.0.0.1:21001"]
join_retry_interval_ms = 2500
join_max_attempts = 5
join_blocking = true
join_target_role = "learner"
"#;
        std::fs::write(&file, content).expect("write file");

        let err = KLogRuntimeConfig::from_file(&file).expect_err("legacy flat fields must fail");
        assert!(err.contains("unknown field"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_partial_values() {
        let file = unique_test_file("partial");
        let content = r#"
node_id = 7

[network]
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
        assert_eq!(cfg.admin_local_only, DEFAULT_ADMIN_LOCAL_ONLY);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_missing_node_id_rejected() {
        let file = unique_test_file("missing_node_id");
        let content = r#"
[network]
listen_addr = "0.0.0.0:22001"
"#;
        std::fs::write(&file, content).expect("write file");

        let err = KLogRuntimeConfig::from_file(&file).expect_err("missing node_id must fail");
        assert!(err.contains("Missing required field: node_id"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_auto_bootstrap_conflicts_with_join_targets() {
        let file = unique_test_file("bootstrap_join_conflict");
        let content = r#"
node_id = 9

[cluster]
auto_bootstrap = true

[join]
targets = ["127.0.0.1:21001"]
"#;
        std::fs::write(&file, content).expect("write file");

        let err =
            KLogRuntimeConfig::from_file(&file).expect_err("bootstrap+join.targets must fail");
        assert!(err.contains("auto_bootstrap=true"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_buckyos_patch() {
        let patch = BuckyosKlogConfig {
            network: Some(KLogNetworkConfigPatch {
                listen_addr: Some("0.0.0.0:23001".to_string()),
                advertise_addr: Some("172.20.0.3".to_string()),
                advertise_port: Some(23001),
            }),
            storage: Some(KLogStorageConfigPatch {
                data_dir: None,
                state_store_sync_write: Some(false),
            }),
            cluster: Some(KLogClusterConfigPatch {
                name: Some("bk".to_string()),
                auto_bootstrap: Some(false),
            }),
            join: Some(KLogJoinConfigPatch {
                targets: Some(vec!["10.0.0.1:21001".to_string()]),
                retry_interval_ms: Some(6000),
                max_attempts: Some(3),
                blocking: Some(true),
                target_role: Some(KLogJoinTargetRole::Learner),
            }),
            admin: Some(KLogAdminConfigPatch {
                local_only: Some(false),
            }),
            node_id: Some(3),
            ..Default::default()
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
        assert!(!cfg.auto_bootstrap);
        assert!(!cfg.state_store_sync_write);
        assert_eq!(cfg.join_targets, vec!["10.0.0.1:21001".to_string()]);
        assert_eq!(cfg.join_retry_interval_ms, 6000);
        assert_eq!(cfg.join_max_attempts, 3);
        assert!(cfg.join_blocking);
        assert_eq!(cfg.join_target_role, KLogJoinTargetRole::Learner);
        assert!(!cfg.admin_local_only);
    }
}
