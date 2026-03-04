use std::path::PathBuf;

/// Environment key for overriding server bind address.
pub const SLOG_SERVER_BIND_ENV_KEY: &str = "SLOG_SERVER_BIND";
/// Environment key for overriding local storage directory.
pub const SLOG_STORAGE_DIR_ENV_KEY: &str = "SLOG_STORAGE_DIR";
/// Default bind address when no external config is provided.
pub const DEFAULT_SERVER_BIND: &str = "127.0.0.1:8089";

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub bind_addr: String,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub storage_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub network: NetworkConfig,
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Default)]
pub struct ServerEnvOverrides {
    pub bind_addr: Option<String>,
    pub storage_dir: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig {
                bind_addr: DEFAULT_SERVER_BIND.to_string(),
            },
            storage: StorageConfig {
                storage_dir: slog::get_buckyos_root_dir().join("slog_server"),
            },
        }
    }
}

impl ServerConfig {
    /// Apply env-derived overrides over an existing config.
    ///
    /// The caller (usually `main`) is responsible for reading environment
    /// variables and converting them into `ServerEnvOverrides`.
    /// This keeps config assembly open for future sources (file/remote service).
    pub fn apply_env_overrides(&mut self, overrides: ServerEnvOverrides) {
        if let Some(v) = overrides.bind_addr {
            self.network.bind_addr = v;
        }

        if let Some(v) = overrides.storage_dir {
            self.storage.storage_dir = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SERVER_BIND, SLOG_SERVER_BIND_ENV_KEY, SLOG_STORAGE_DIR_ENV_KEY, ServerConfig,
        ServerEnvOverrides,
    };
    use std::path::PathBuf;

    #[test]
    fn test_server_config_default_values() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.network.bind_addr, DEFAULT_SERVER_BIND);
        assert_eq!(
            cfg.storage.storage_dir,
            slog::get_buckyos_root_dir().join("slog_server")
        );
    }

    #[test]
    fn test_server_config_apply_env_overrides_all_fields() {
        let mut cfg = ServerConfig::default();
        let overrides = ServerEnvOverrides {
            bind_addr: Some("0.0.0.0:18089".to_string()),
            storage_dir: Some(PathBuf::from("/tmp/slog_server_data")),
        };
        cfg.apply_env_overrides(overrides);

        assert_eq!(cfg.network.bind_addr, "0.0.0.0:18089");
        assert_eq!(
            cfg.storage.storage_dir,
            PathBuf::from("/tmp/slog_server_data")
        );
    }

    #[test]
    fn test_server_config_apply_env_overrides_partial_fields() {
        let mut cfg = ServerConfig::default();
        let original_storage_dir = cfg.storage.storage_dir.clone();
        let overrides = ServerEnvOverrides {
            bind_addr: Some("127.0.0.1:19089".to_string()),
            storage_dir: None,
        };
        cfg.apply_env_overrides(overrides);

        assert_eq!(cfg.network.bind_addr, "127.0.0.1:19089");
        assert_eq!(cfg.storage.storage_dir, original_storage_dir);
    }

    #[test]
    fn test_server_config_env_keys_are_stable() {
        assert_eq!(SLOG_SERVER_BIND_ENV_KEY, "SLOG_SERVER_BIND");
        assert_eq!(SLOG_STORAGE_DIR_ENV_KEY, "SLOG_STORAGE_DIR");
    }
}
