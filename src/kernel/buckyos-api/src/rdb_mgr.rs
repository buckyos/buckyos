/*!
 * rdb instance manager
 *
 * Resolves a connection string for a named relational-db instance so an
 * app / service can talk to sqlite or postgres through `sqlx` without caring
 * which backend is configured in the current zone.
 *
 * Each instance is identified by `(appid, owner_user_id, instance_id)`. The
 * backend + connection string + schema have already been picked when the app
 * was installed and are serialized into the app's `ServiceInstallConfig` at
 *   `users/{user}/apps/{appid}/spec`   (AppService)
 *   `users/{user}/agents/{appid}/spec` (Agent)
 *   `services/{appid}/spec`            (Kernel/Frame service)
 *
 * This module only reads that spec — it does not seed from a local manifest
 * or write back to `system_config`.
 */

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ::kRPC::{RPCErrors, Result};
use serde::{Deserialize, Serialize};

use crate::get_buckyos_api_runtime;
use crate::system_config::{SystemConfigClient, SystemConfigError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RdbBackend {
    Sqlite,
    Postgres,
}

impl Default for RdbBackend {
    fn default() -> Self {
        RdbBackend::Sqlite
    }
}

/// Resolved instance — what callers actually need to talk to the db.
#[derive(Debug, Clone)]
pub struct RdbInstance {
    pub backend: RdbBackend,
    pub version: u64,
    pub connection: String,
    /// Schema DDL for the active backend, already selected from the per-backend
    /// map. Callers run this on first open to initialize tables.
    pub schema: Option<String>,
}

/// Per-instance config carried inside `ServiceInstallConfig.rdb_instances`.
///
/// The installer fills this in: it picks a `backend`, writes the final
/// `connection` string (may still contain `$appdata` / `$instance`
/// placeholders that are resolved at open time), and records the schema DDL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdbInstanceConfig {
    pub backend: RdbBackend,
    #[serde(default = "default_schema_version")]
    pub version: u64,
    #[serde(default)]
    pub schema: HashMap<RdbBackend, String>,
    /// Template used to build the sqlx connection string. `$appdata` is
    /// replaced with the caller's resolved data folder, `$instance` with the
    /// instance id. When empty a sensible default is generated for sqlite.
    #[serde(default)]
    pub connection: String,
}

fn default_schema_version() -> u64 {
    1
}

/// Minimal view of the app/service spec that just exposes `install_config`.
/// Works against `AppServiceSpec` and `KernelServiceSpec` alike.
#[derive(Debug, Deserialize)]
struct SpecInstallView {
    install_config: InstallConfigView,
}

#[derive(Debug, Deserialize)]
struct InstallConfigView {
    #[serde(default)]
    rdb_instances: HashMap<String, RdbInstanceConfig>,
}

/// Return an `sqlx`-compatible connection string for the given instance.
///
/// `owner_user_id` is optional — when `None` the instance is treated as a
/// service-scoped database; otherwise it is a per-user app database.
pub async fn get_rdb_instance(
    appid: &str,
    owner_user_id: Option<String>,
    instance_id: &str,
) -> Result<RdbInstance> {
    if appid.is_empty() {
        return Err(RPCErrors::ReasonError("appid is empty".to_string()));
    }
    if instance_id.is_empty() {
        return Err(RPCErrors::ReasonError("instance_id is empty".to_string()));
    }

    let runtime = get_buckyos_api_runtime()?;
    let sys_cfg = runtime.get_system_config_client().await?;

    let cfg =
        load_install_rdb_config(&sys_cfg, appid, owner_user_id.as_deref(), instance_id).await?;

    let connection = build_connection_string(&cfg, appid, owner_user_id.as_deref(), instance_id)?;
    let schema = pick_schema(&cfg);
    Ok(RdbInstance {
        backend: cfg.backend,
        version: cfg.version,
        connection,
        schema,
    })
}

/// Try each candidate spec key until we find one that parses and carries the
/// requested instance. Missing keys are skipped; other errors are surfaced.
async fn load_install_rdb_config(
    sys_cfg: &SystemConfigClient,
    appid: &str,
    owner_user_id: Option<&str>,
    instance_id: &str,
) -> Result<RdbInstanceConfig> {
    let candidates = spec_key_candidates(appid, owner_user_id);
    let mut tried: Vec<String> = Vec::with_capacity(candidates.len());

    for key in &candidates {
        tried.push(key.clone());
        let raw = match sys_cfg.get(key).await {
            Ok(value) => value.value,
            Err(SystemConfigError::KeyNotFound(_)) => continue,
            Err(err) => {
                return Err(RPCErrors::ReasonError(format!(
                    "read spec {} failed: {}",
                    key, err
                )));
            }
        };
        let view: SpecInstallView = serde_json::from_str(&raw).map_err(|err| {
            RPCErrors::ReasonError(format!("parse spec at {} failed: {}", key, err))
        })?;
        if let Some(cfg) = view.install_config.rdb_instances.get(instance_id) {
            return Ok(cfg.clone());
        }
        return Err(RPCErrors::ReasonError(format!(
            "rdb instance {} not declared in install_config.rdb_instances at {}",
            instance_id, key
        )));
    }

    Err(RPCErrors::ReasonError(format!(
        "spec for appid={} not found (tried: {})",
        appid,
        tried.join(", ")
    )))
}

fn spec_key_candidates(appid: &str, owner_user_id: Option<&str>) -> Vec<String> {
    match owner_user_id {
        Some(user) => vec![
            format!("users/{}/apps/{}/spec", user, appid),
            format!("users/{}/agents/{}/spec", user, appid),
        ],
        None => vec![format!("services/{}/spec", appid)],
    }
}

/// Pick the schema DDL that matches the active backend.
fn pick_schema(cfg: &RdbInstanceConfig) -> Option<String> {
    if let Some(sql) = cfg.schema.get(&cfg.backend) {
        if !sql.trim().is_empty() {
            return Some(sql.clone());
        }
    }
    None
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
            // `mode=rwc` lets sqlx auto-create the db file on first open —
            // without it the default mode is `rw` and the pool fails to connect
            // the very first time the service starts.
            RdbBackend::Sqlite => {
                format!("sqlite://{}/{}.db?mode=rwc", appdata_str, instance_id)
            }
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
    let is_self =
        runtime.get_app_id() == appid && runtime.get_owner_user_id().as_deref() == owner_user_id;
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
    fn pick_schema_selects_active_backend() {
        let mut schema = HashMap::new();
        schema.insert(
            RdbBackend::Sqlite,
            "CREATE TABLE t(id INTEGER);".to_string(),
        );
        schema.insert(
            RdbBackend::Postgres,
            "CREATE TABLE t(id BIGINT);".to_string(),
        );
        let cfg = RdbInstanceConfig {
            backend: RdbBackend::Postgres,
            version: 1,
            schema,
            connection: "postgres://user:pass@localhost/db".to_string(),
        };
        assert_eq!(
            pick_schema(&cfg).as_deref(),
            Some("CREATE TABLE t(id BIGINT);")
        );
    }

    #[test]
    fn spec_key_candidates_for_app_and_service() {
        assert_eq!(
            spec_key_candidates("demo", Some("alice")),
            vec![
                "users/alice/apps/demo/spec".to_string(),
                "users/alice/agents/demo/spec".to_string(),
            ]
        );
        assert_eq!(
            spec_key_candidates("verify-hub", None),
            vec!["services/verify-hub/spec".to_string()]
        );
    }

    #[test]
    fn parse_spec_view_extracts_rdb_instances() {
        let raw = r#"{
            "install_config": {
                "data_mount_point": {},
                "cache_mount_point": [],
                "local_cache_mount_point": [],
                "expose_config": {},
                "res_pool_id": "default",
                "allow_public_access": false,
                "rdb_instances": {
                    "main": {
                        "backend": "sqlite",
                        "version": 3,
                        "schema": { "sqlite": "CREATE TABLE t(id INTEGER);" },
                        "connection": "sqlite://$appdata/main.db"
                    }
                }
            }
        }"#;
        let view: SpecInstallView = serde_json::from_str(raw).unwrap();
        let cfg = view
            .install_config
            .rdb_instances
            .get("main")
            .cloned()
            .unwrap();
        assert_eq!(cfg.backend, RdbBackend::Sqlite);
        assert_eq!(cfg.version, 3);
        assert_eq!(cfg.connection, "sqlite://$appdata/main.db");
    }
}
