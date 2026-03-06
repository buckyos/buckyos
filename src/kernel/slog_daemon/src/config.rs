use crate::constants::{
    DEFAULT_NODE_ID, DEFAULT_SERVER_ENDPOINT, DEFAULT_UPLOAD_GLOBAL_CONCURRENCY,
    DEFAULT_UPLOAD_TIMEOUT_SECS,
};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub node_id: String,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub endpoint: String,
}

#[derive(Debug, Clone)]
pub struct PathConfig {
    pub log_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct UploadConfig {
    pub timeout_secs: u64,
    pub global_concurrency: usize,
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub node: NodeConfig,
    pub server: ServerConfig,
    pub path: PathConfig,
    pub upload: UploadConfig,
}

#[derive(Debug, Clone, Default)]
pub struct DaemonEnvOverrides {
    pub node_id: Option<String>,
    pub server_endpoint: Option<String>,
    pub log_dir: Option<PathBuf>,
    pub upload_timeout_secs: Option<u64>,
    pub upload_global_concurrency: Option<usize>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            node: NodeConfig {
                node_id: DEFAULT_NODE_ID.to_string(),
            },
            server: ServerConfig {
                endpoint: DEFAULT_SERVER_ENDPOINT.to_string(),
            },
            path: PathConfig {
                log_dir: slog::get_buckyos_log_root_dir(),
            },
            upload: UploadConfig {
                timeout_secs: DEFAULT_UPLOAD_TIMEOUT_SECS,
                global_concurrency: DEFAULT_UPLOAD_GLOBAL_CONCURRENCY,
            },
        }
    }
}

impl DaemonConfig {
    /// Apply env-derived overrides over an existing config.
    ///
    /// The caller (usually `main`) is responsible for reading environment
    /// variables and converting them into `DaemonEnvOverrides`.
    /// This keeps config assembly open for future sources (file/remote service).
    pub fn apply_env_overrides(&mut self, overrides: DaemonEnvOverrides) {
        if let Some(v) = overrides.node_id {
            self.node.node_id = v;
        }

        if let Some(v) = overrides.server_endpoint {
            self.server.endpoint = v;
        }

        if let Some(v) = overrides.log_dir {
            self.path.log_dir = v;
        }

        if let Some(v) = overrides.upload_timeout_secs {
            self.upload.timeout_secs = v;
        }

        if let Some(v) = overrides.upload_global_concurrency {
            self.upload.global_concurrency = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DaemonConfig, DaemonEnvOverrides};
    use crate::constants::{
        DEFAULT_NODE_ID, DEFAULT_SERVER_ENDPOINT, DEFAULT_UPLOAD_GLOBAL_CONCURRENCY,
        DEFAULT_UPLOAD_TIMEOUT_SECS, SLOG_LOG_DIR_ENV_KEY, SLOG_NODE_ID_ENV_KEY,
        SLOG_SERVER_ENDPOINT_ENV_KEY, SLOG_UPLOAD_GLOBAL_CONCURRENCY_ENV_KEY,
        SLOG_UPLOAD_TIMEOUT_SECS_ENV_KEY,
    };
    use std::path::PathBuf;

    #[test]
    fn test_daemon_config_default_values() {
        let cfg = DaemonConfig::default();
        assert_eq!(cfg.node.node_id, DEFAULT_NODE_ID);
        assert_eq!(cfg.server.endpoint, DEFAULT_SERVER_ENDPOINT);
        assert_eq!(cfg.upload.timeout_secs, DEFAULT_UPLOAD_TIMEOUT_SECS);
        assert_eq!(
            cfg.upload.global_concurrency,
            DEFAULT_UPLOAD_GLOBAL_CONCURRENCY
        );
        assert_eq!(cfg.path.log_dir, slog::get_buckyos_log_root_dir());
    }

    #[test]
    fn test_daemon_config_apply_env_overrides_all_fields() {
        let mut cfg = DaemonConfig::default();
        let overrides = DaemonEnvOverrides {
            node_id: Some("node-test-001".to_string()),
            server_endpoint: Some("http://10.10.0.2:22001/logs".to_string()),
            log_dir: Some(PathBuf::from("/tmp/slog_log_root")),
            upload_timeout_secs: Some(33),
            upload_global_concurrency: Some(8),
        };
        cfg.apply_env_overrides(overrides);

        assert_eq!(cfg.node.node_id, "node-test-001");
        assert_eq!(cfg.server.endpoint, "http://10.10.0.2:22001/logs");
        assert_eq!(cfg.path.log_dir, PathBuf::from("/tmp/slog_log_root"));
        assert_eq!(cfg.upload.timeout_secs, 33);
        assert_eq!(cfg.upload.global_concurrency, 8);
    }

    #[test]
    fn test_daemon_config_apply_env_overrides_partial_fields() {
        let mut cfg = DaemonConfig::default();
        let original_endpoint = cfg.server.endpoint.clone();
        let original_log_dir = cfg.path.log_dir.clone();

        let overrides = DaemonEnvOverrides {
            node_id: Some("node-test-002".to_string()),
            server_endpoint: None,
            log_dir: None,
            upload_timeout_secs: Some(12),
            upload_global_concurrency: None,
        };
        cfg.apply_env_overrides(overrides);

        assert_eq!(cfg.node.node_id, "node-test-002");
        assert_eq!(cfg.server.endpoint, original_endpoint);
        assert_eq!(cfg.path.log_dir, original_log_dir);
        assert_eq!(cfg.upload.timeout_secs, 12);
        assert_eq!(
            cfg.upload.global_concurrency,
            DEFAULT_UPLOAD_GLOBAL_CONCURRENCY
        );
    }

    #[test]
    fn test_daemon_config_env_keys_are_stable() {
        assert_eq!(SLOG_NODE_ID_ENV_KEY, "SLOG_NODE_ID");
        assert_eq!(SLOG_SERVER_ENDPOINT_ENV_KEY, "SLOG_SERVER_ENDPOINT");
        assert_eq!(SLOG_LOG_DIR_ENV_KEY, "SLOG_LOG_DIR");
        assert_eq!(SLOG_UPLOAD_TIMEOUT_SECS_ENV_KEY, "SLOG_UPLOAD_TIMEOUT_SECS");
        assert_eq!(
            SLOG_UPLOAD_GLOBAL_CONCURRENCY_ENV_KEY,
            "SLOG_UPLOAD_GLOBAL_CONCURRENCY"
        );
    }
}
