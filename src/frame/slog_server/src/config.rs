use std::path::PathBuf;

use crate::storage::{LogStorageType, PartitionBucket, SqlitePartitionedConfig};

/// Environment key for overriding server bind address.
pub const SLOG_SERVER_BIND_ENV_KEY: &str = "SLOG_SERVER_BIND";
/// Environment key for overriding local storage directory.
pub const SLOG_STORAGE_DIR_ENV_KEY: &str = "SLOG_STORAGE_DIR";
/// Environment key for selecting storage backend type (`sqlite` / `sqlite_partitioned`).
pub const SLOG_STORAGE_TYPE_ENV_KEY: &str = "SLOG_STORAGE_TYPE";
/// Environment key for partition bucket (currently supports `day`).
pub const SLOG_STORAGE_PARTITION_BUCKET_ENV_KEY: &str = "SLOG_STORAGE_PARTITION_BUCKET";
/// Environment key for max rows in one partition DB file.
pub const SLOG_STORAGE_PARTITION_MAX_ROWS_ENV_KEY: &str = "SLOG_STORAGE_PARTITION_MAX_ROWS";
/// Environment key for max DB size (MB) in one partition DB file.
pub const SLOG_STORAGE_PARTITION_MAX_SIZE_MB_ENV_KEY: &str = "SLOG_STORAGE_PARTITION_MAX_SIZE_MB";
/// Default bind address when no external config is provided.
pub const DEFAULT_SERVER_BIND: &str = "127.0.0.1:22001";
/// Default backend type.
pub const DEFAULT_STORAGE_TYPE: StorageEngine = StorageEngine::SqlitePartitioned;
/// Default partition bucket.
pub const DEFAULT_STORAGE_PARTITION_BUCKET: PartitionBucket = PartitionBucket::Day;
/// Default row cap per partition DB.
pub const DEFAULT_STORAGE_PARTITION_MAX_ROWS: u64 = 5_000_000;
/// Default size cap per partition DB, in MB.
pub const DEFAULT_STORAGE_PARTITION_MAX_SIZE_MB: u64 = 2048;

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub bind_addr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEngine {
    Sqlite,
    SqlitePartitioned,
}

impl StorageEngine {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Some(Self::Sqlite),
            "sqlite_partitioned" | "sqlite-partitioned" | "partitioned" => {
                Some(Self::SqlitePartitioned)
            }
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::SqlitePartitioned => "sqlite_partitioned",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoragePartitionConfig {
    pub bucket: PartitionBucket,
    pub max_rows_per_partition: u64,
    pub max_partition_size_mb: u64,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub storage_dir: PathBuf,
    pub storage_engine: StorageEngine,
    pub partition: StoragePartitionConfig,
}

impl StorageConfig {
    pub fn to_storage_type(&self) -> LogStorageType {
        match self.storage_engine {
            StorageEngine::Sqlite => LogStorageType::Sqlite,
            StorageEngine::SqlitePartitioned => {
                let max_partition_size_bytes = self
                    .partition
                    .max_partition_size_mb
                    .saturating_mul(1024 * 1024);
                LogStorageType::SqlitePartitioned(SqlitePartitionedConfig {
                    bucket: self.partition.bucket.clone(),
                    max_rows_per_partition: self.partition.max_rows_per_partition,
                    max_partition_size_bytes,
                })
            }
        }
    }
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
    pub storage_engine: Option<StorageEngine>,
    pub partition_bucket: Option<PartitionBucket>,
    pub partition_max_rows: Option<u64>,
    pub partition_max_size_mb: Option<u64>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig {
                bind_addr: DEFAULT_SERVER_BIND.to_string(),
            },
            storage: StorageConfig {
                storage_dir: slog::get_buckyos_root_dir().join("slog_server"),
                storage_engine: DEFAULT_STORAGE_TYPE,
                partition: StoragePartitionConfig {
                    bucket: DEFAULT_STORAGE_PARTITION_BUCKET,
                    max_rows_per_partition: DEFAULT_STORAGE_PARTITION_MAX_ROWS,
                    max_partition_size_mb: DEFAULT_STORAGE_PARTITION_MAX_SIZE_MB,
                },
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

        if let Some(v) = overrides.storage_engine {
            self.storage.storage_engine = v;
        }

        if let Some(v) = overrides.partition_bucket {
            self.storage.partition.bucket = v;
        }

        if let Some(v) = overrides.partition_max_rows {
            self.storage.partition.max_rows_per_partition = v;
        }

        if let Some(v) = overrides.partition_max_size_mb {
            self.storage.partition.max_partition_size_mb = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SERVER_BIND, DEFAULT_STORAGE_PARTITION_MAX_ROWS,
        DEFAULT_STORAGE_PARTITION_MAX_SIZE_MB, DEFAULT_STORAGE_TYPE, SLOG_SERVER_BIND_ENV_KEY,
        SLOG_STORAGE_DIR_ENV_KEY, SLOG_STORAGE_PARTITION_BUCKET_ENV_KEY,
        SLOG_STORAGE_PARTITION_MAX_ROWS_ENV_KEY, SLOG_STORAGE_PARTITION_MAX_SIZE_MB_ENV_KEY,
        SLOG_STORAGE_TYPE_ENV_KEY, ServerConfig, ServerEnvOverrides, StorageEngine,
    };
    use crate::storage::{LogStorageType, PartitionBucket};
    use std::path::PathBuf;

    #[test]
    fn test_server_config_default_values() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.network.bind_addr, DEFAULT_SERVER_BIND);
        assert_eq!(
            cfg.storage.storage_dir,
            slog::get_buckyos_root_dir().join("slog_server")
        );
        assert_eq!(cfg.storage.storage_engine, DEFAULT_STORAGE_TYPE);
        assert_eq!(cfg.storage.partition.bucket, PartitionBucket::Day);
        assert_eq!(
            cfg.storage.partition.max_rows_per_partition,
            DEFAULT_STORAGE_PARTITION_MAX_ROWS
        );
        assert_eq!(
            cfg.storage.partition.max_partition_size_mb,
            DEFAULT_STORAGE_PARTITION_MAX_SIZE_MB
        );
    }

    #[test]
    fn test_server_config_apply_env_overrides_all_fields() {
        let mut cfg = ServerConfig::default();
        let overrides = ServerEnvOverrides {
            bind_addr: Some("0.0.0.0:18089".to_string()),
            storage_dir: Some(PathBuf::from("/tmp/slog_server_data")),
            storage_engine: Some(StorageEngine::SqlitePartitioned),
            partition_bucket: Some(PartitionBucket::Day),
            partition_max_rows: Some(1000),
            partition_max_size_mb: Some(128),
        };
        cfg.apply_env_overrides(overrides);

        assert_eq!(cfg.network.bind_addr, "0.0.0.0:18089");
        assert_eq!(
            cfg.storage.storage_dir,
            PathBuf::from("/tmp/slog_server_data")
        );
        assert_eq!(cfg.storage.storage_engine, StorageEngine::SqlitePartitioned);
        assert_eq!(cfg.storage.partition.bucket, PartitionBucket::Day);
        assert_eq!(cfg.storage.partition.max_rows_per_partition, 1000);
        assert_eq!(cfg.storage.partition.max_partition_size_mb, 128);
    }

    #[test]
    fn test_server_config_apply_env_overrides_partial_fields() {
        let mut cfg = ServerConfig::default();
        let original_storage_dir = cfg.storage.storage_dir.clone();
        let overrides = ServerEnvOverrides {
            bind_addr: Some("127.0.0.1:19089".to_string()),
            storage_dir: None,
            storage_engine: None,
            partition_bucket: None,
            partition_max_rows: None,
            partition_max_size_mb: None,
        };
        cfg.apply_env_overrides(overrides);

        assert_eq!(cfg.network.bind_addr, "127.0.0.1:19089");
        assert_eq!(cfg.storage.storage_dir, original_storage_dir);
    }

    #[test]
    fn test_server_config_env_keys_are_stable() {
        assert_eq!(SLOG_SERVER_BIND_ENV_KEY, "SLOG_SERVER_BIND");
        assert_eq!(SLOG_STORAGE_DIR_ENV_KEY, "SLOG_STORAGE_DIR");
        assert_eq!(SLOG_STORAGE_TYPE_ENV_KEY, "SLOG_STORAGE_TYPE");
        assert_eq!(
            SLOG_STORAGE_PARTITION_BUCKET_ENV_KEY,
            "SLOG_STORAGE_PARTITION_BUCKET"
        );
        assert_eq!(
            SLOG_STORAGE_PARTITION_MAX_ROWS_ENV_KEY,
            "SLOG_STORAGE_PARTITION_MAX_ROWS"
        );
        assert_eq!(
            SLOG_STORAGE_PARTITION_MAX_SIZE_MB_ENV_KEY,
            "SLOG_STORAGE_PARTITION_MAX_SIZE_MB"
        );
    }

    #[test]
    fn test_storage_engine_parse_and_to_storage_type() {
        assert_eq!(StorageEngine::parse("sqlite"), Some(StorageEngine::Sqlite));
        assert_eq!(
            StorageEngine::parse("sqlite_partitioned"),
            Some(StorageEngine::SqlitePartitioned)
        );
        assert_eq!(
            StorageEngine::parse("partitioned"),
            Some(StorageEngine::SqlitePartitioned)
        );
        assert_eq!(StorageEngine::parse("unknown"), None);

        let mut cfg = ServerConfig::default();
        cfg.storage.storage_engine = StorageEngine::SqlitePartitioned;
        cfg.storage.partition.max_partition_size_mb = 64;
        cfg.storage.partition.max_rows_per_partition = 123;

        match cfg.storage.to_storage_type() {
            LogStorageType::Sqlite => panic!("expected partitioned storage type"),
            LogStorageType::SqlitePartitioned(partitioned) => {
                assert_eq!(partitioned.bucket, PartitionBucket::Day);
                assert_eq!(partitioned.max_rows_per_partition, 123);
                assert_eq!(partitioned.max_partition_size_bytes, 64 * 1024 * 1024);
            }
        }
    }
}
