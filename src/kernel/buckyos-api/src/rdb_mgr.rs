/*!
 * rdb instance manager
 *
 * Resolves a connection string for a named relational-db instance so an
 * app / service can talk to sqlite or postgres through `sqlx` without caring
 * which backend is configured in the current zone.
 *
 * Each instance is identified by `(appid, owner_user_id, instance_id)` and
 * backed by a config document stored in `system_config` at
 *   `settings/rdb/{instance_id}`
 *
 * On first boot the config is seeded from the `rdb_instance.json` manifest
 * that ships in the app binary directory (next to the current executable).
 * Schema upgrades are detected by comparing the `version` field in the
 * manifest with the one already stored in `system_config`.
 */

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ::kRPC::{RPCErrors, Result};
use log::*;
use serde::{Deserialize, Serialize};

use crate::get_buckyos_api_runtime;
use crate::system_config::{SystemConfigClient, SystemConfigError};

const RDB_INSTANCE_MANIFEST: &str = "rdb_instance.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RdbBackend {
    Sqlite,
    Postgres,
}

/// Config document stored at `settings/rdb/{instance_id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RdbInstanceConfig {
    pub backend: RdbBackend,
    #[serde(default = "default_schema_version")]
    pub version: u64,
    #[serde(default)]
    pub schema: String,
    /// Template used to build the sqlx connection string. `$appdata` is
    /// replaced with the caller's resolved data folder, `$instance` with the
    /// instance id. When empty a sensible default is generated for sqlite.
    #[serde(default)]
    pub connection: String,
}

fn default_schema_version() -> u64 {
    1
}

#[derive(Debug, Deserialize)]
struct RdbInstanceManifest {
    #[serde(default)]
    instances: HashMap<String, RdbInstanceConfig>,
}

/// Return an `sqlx`-compatible connection string for the given instance.
///
/// `owner_user_id` is optional — when `None` the instance is treated as a
/// service-scoped database; otherwise it is a per-user app database.
pub async fn get_rdb_instance(
    appid: &str,
    owner_user_id: Option<String>,
    instance_id: &str,
) -> Result<String> {
    if appid.is_empty() {
        return Err(RPCErrors::ReasonError("appid is empty".to_string()));
    }
    if instance_id.is_empty() {
        return Err(RPCErrors::ReasonError("instance_id is empty".to_string()));
    }

    let runtime = get_buckyos_api_runtime()?;
    let sys_cfg = runtime.get_system_config_client().await?;
    let settings_key = settings_key_of(instance_id);

    let stored: Option<RdbInstanceConfig> = match sys_cfg.get(&settings_key).await {
        Ok(value) => Some(parse_config(&value.value, &settings_key)?),
        Err(SystemConfigError::KeyNotFound(_)) => None,
        Err(err) => {
            return Err(RPCErrors::ReasonError(format!(
                "read rdb settings {} failed: {}",
                settings_key, err
            )));
        }
    };

    let manifest = load_manifest_for_instance(instance_id)?;

    let active = match (stored, manifest) {
        (Some(existing), Some(from_disk)) => {
            if from_disk.version > existing.version {
                warn!(
                    "rdb instance {} schema bumped {} -> {}, refreshing system_config",
                    instance_id, existing.version, from_disk.version
                );
                write_config(&sys_cfg, &settings_key, &from_disk).await?;
                from_disk
            } else {
                existing
            }
        }
        (Some(existing), None) => existing,
        (None, Some(from_disk)) => {
            info!(
                "rdb instance {} not yet registered, seeding system_config from {}",
                instance_id, RDB_INSTANCE_MANIFEST
            );
            write_config(&sys_cfg, &settings_key, &from_disk).await?;
            from_disk
        }
        (None, None) => {
            return Err(RPCErrors::ReasonError(format!(
                "rdb instance {} has no config in system_config and no {} manifest alongside the binary",
                instance_id, RDB_INSTANCE_MANIFEST
            )));
        }
    };

    build_connection_string(&active, appid, owner_user_id.as_deref(), instance_id)
}

fn settings_key_of(instance_id: &str) -> String {
    format!("settings/rdb/{}", instance_id)
}

fn parse_config(raw: &str, key: &str) -> Result<RdbInstanceConfig> {
    serde_json::from_str(raw).map_err(|err| {
        RPCErrors::ReasonError(format!("parse rdb config at {} failed: {}", key, err))
    })
}

async fn write_config(
    sys_cfg: &SystemConfigClient,
    key: &str,
    cfg: &RdbInstanceConfig,
) -> Result<()> {
    let body = serde_json::to_string(cfg)
        .map_err(|err| RPCErrors::ReasonError(format!("serialize rdb config failed: {}", err)))?;
    sys_cfg
        .set(key, &body)
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("write {} failed: {}", key, err)))?;
    Ok(())
}

fn load_manifest_for_instance(instance_id: &str) -> Result<Option<RdbInstanceConfig>> {
    let Some(manifest_path) = locate_manifest() else {
        return Ok(None);
    };
    let raw = match std::fs::read_to_string(&manifest_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RPCErrors::ReasonError(format!(
                "read {} failed: {}",
                manifest_path.display(),
                err
            )));
        }
    };
    let manifest: RdbInstanceManifest = serde_json::from_str(&raw).map_err(|err| {
        RPCErrors::ReasonError(format!("parse {} failed: {}", manifest_path.display(), err))
    })?;
    Ok(manifest.instances.get(instance_id).cloned())
}

fn locate_manifest() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join(RDB_INSTANCE_MANIFEST);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn build_connection_string(
    cfg: &RdbInstanceConfig,
    appid: &str,
    owner_user_id: Option<&str>,
    instance_id: &str,
) -> Result<String> {
    let appdata = resolve_appdata_dir(appid, owner_user_id)?;
    let appdata_str = appdata.to_string_lossy().to_string();

    let template = if cfg.connection.is_empty() {
        match cfg.backend {
            RdbBackend::Sqlite => format!("sqlite://{}/{}.db", appdata_str, instance_id),
            RdbBackend::Postgres => {
                return Err(RPCErrors::ReasonError(format!(
                    "rdb instance {} uses postgres backend but has no connection string configured",
                    instance_id
                )));
            }
        }
    } else {
        cfg.connection.clone()
    };

    let resolved = template
        .replace("$appdata", &appdata_str)
        .replace("$instance", instance_id);

    if cfg.backend == RdbBackend::Sqlite {
        ensure_sqlite_dir(&resolved)?;
    }

    Ok(resolved)
}

fn resolve_appdata_dir(appid: &str, owner_user_id: Option<&str>) -> Result<PathBuf> {
    let runtime = get_buckyos_api_runtime()?;
    let is_self = runtime.get_app_id() == appid
        && runtime.get_owner_user_id().as_deref() == owner_user_id;
    if is_self {
        return runtime.get_data_folder();
    }

    // Cross-app lookup: reproduce the layout used by `BuckyOSRuntime::get_data_folder`.
    let root = runtime.buckyos_root_dir.clone();
    let path = match owner_user_id {
        Some(user) => root.join("data").join(user).join(appid),
        None => root.join("data").join(appid),
    };
    Ok(path)
}

fn ensure_sqlite_dir(connection: &str) -> Result<()> {
    // sqlite URLs look like `sqlite://<path>` or `sqlite:<path>` — extract the
    // filesystem path and make sure the parent directory exists so sqlx can
    // open the file on first use.
    let path_str = connection
        .strip_prefix("sqlite://")
        .or_else(|| connection.strip_prefix("sqlite:"))
        .unwrap_or(connection);
    let path_str = path_str.split('?').next().unwrap_or(path_str);
    if path_str.is_empty() || path_str == ":memory:" {
        return Ok(());
    }
    if let Some(parent) = Path::new(path_str).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "create sqlite dir {} failed: {}",
                    parent.display(),
                    err
                ))
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_picks_named_instance() {
        let raw = r#"{
            "instances": {
                "main": {
                    "backend": "sqlite",
                    "version": 2,
                    "schema": "CREATE TABLE t(id INTEGER);",
                    "connection": "sqlite://$appdata/main.db"
                }
            }
        }"#;
        let manifest: RdbInstanceManifest = serde_json::from_str(raw).unwrap();
        let cfg = manifest.instances.get("main").cloned().unwrap();
        assert_eq!(cfg.backend, RdbBackend::Sqlite);
        assert_eq!(cfg.version, 2);
        assert_eq!(cfg.connection, "sqlite://$appdata/main.db");
    }

    #[test]
    fn settings_key_follows_doc_layout() {
        assert_eq!(settings_key_of("main"), "settings/rdb/main");
    }
}
