/*!
 * rdb instance manager
 *
 * Resolves a connection string for a named relational-db instance from the
 * app/service spec stored in system_config.
 */

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::path::{Component, Path, PathBuf};

use ::kRPC::{RPCErrors, Result};
use serde::{Deserialize, Serialize};

use crate::get_buckyos_api_runtime;
use crate::system_config::{SystemConfigClient, SystemConfigError};

pub const RDB_ERR_PARTITION_NOT_DECLARED: &str = "partition_not_declared";
pub const RDB_ERR_PARTITION_AMBIGUOUS: &str = "partition_ambiguous";
pub const RDB_ERR_PARTITION_NOT_ALLOWED_FOR_APP: &str = "partition_not_allowed_for_app";
pub const RDB_ERR_PARTITION_PATH_ESCAPE: &str = "partition_path_escape";
pub const RDB_ERR_PARTITION_PLACEHOLDER_CONFLICT: &str = "partition_placeholder_conflict";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RdbBackend {
    #[default]
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdbPartition {
    #[default]
    UserData,
    Local,
    Cache,
    Storage,
}

impl RdbPartition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserData => "user_data",
            Self::Local => "local",
            Self::Cache => "cache",
            Self::Storage => "storage",
        }
    }
}

impl Display for RdbPartition {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn default_partitions() -> Vec<RdbPartition> {
    vec![RdbPartition::UserData]
}

#[derive(Debug, Clone)]
pub struct RdbInstance {
    pub backend: RdbBackend,
    pub version: u64,
    pub partition: RdbPartition,
    pub connection: String,
    pub schema: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdbInstanceConfig {
    pub backend: RdbBackend,
    #[serde(default = "default_schema_version")]
    pub version: u64,
    #[serde(default)]
    pub schema: HashMap<RdbBackend, String>,
    #[serde(default)]
    pub connection: String,
    #[serde(default = "default_partitions")]
    pub partitions: Vec<RdbPartition>,
}

fn default_schema_version() -> u64 {
    1
}

#[derive(Debug, Deserialize)]
struct SpecInstallView {
    spec_config: InstallConfigView,
}

#[derive(Debug, Deserialize)]
struct InstallConfigView {
    #[serde(default)]
    rdb_instances: HashMap<String, RdbInstanceConfig>,
}

pub async fn get_rdb_instance(
    appid: &str,
    owner_user_id: Option<String>,
    instance_id: &str,
) -> Result<RdbInstance> {
    validate_identity(appid, owner_user_id.as_deref(), instance_id)?;
    let runtime = get_buckyos_api_runtime()?;
    let sys_cfg = runtime.get_system_config_client().await?;
    let configs = load_install_rdb_configs(&sys_cfg, appid, owner_user_id.as_deref()).await?;
    let cfg = get_config(&configs, appid, instance_id)?;
    validate_rdb_instance_config(cfg, appid, instance_id, owner_user_id.is_some())?;

    let partition_paths = if cfg.partitions.len() == 1 {
        String::new()
    } else {
        cfg.partitions
            .iter()
            .filter_map(|partition| {
                resolve_partition_base_dir(
                    &runtime.buckyos_root_dir,
                    appid,
                    owner_user_id.as_deref(),
                    *partition,
                )
                .ok()
            })
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(",")
    };
    let partition = select_implicit_partition(cfg, appid, instance_id, &partition_paths)?;

    resolve_instance(
        cfg,
        &runtime.buckyos_root_dir,
        appid,
        owner_user_id.as_deref(),
        instance_id,
        partition,
        true,
    )
}

fn select_implicit_partition(
    cfg: &RdbInstanceConfig,
    appid: &str,
    instance_id: &str,
    paths: &str,
) -> Result<RdbPartition> {
    if cfg.partitions.len() != 1 {
        return Err(rdb_error(
            RDB_ERR_PARTITION_AMBIGUOUS,
            appid,
            instance_id,
            None,
            paths,
            "instance declares multiple partitions; use get_rdb_instance_in",
        ));
    }

    Ok(cfg.partitions[0])
}

pub async fn get_rdb_instance_in(
    appid: &str,
    owner_user_id: Option<String>,
    instance_id: &str,
    partition: RdbPartition,
) -> Result<RdbInstance> {
    validate_identity(appid, owner_user_id.as_deref(), instance_id)?;
    let runtime = get_buckyos_api_runtime()?;
    let sys_cfg = runtime.get_system_config_client().await?;
    let configs = load_install_rdb_configs(&sys_cfg, appid, owner_user_id.as_deref()).await?;
    let cfg = get_config(&configs, appid, instance_id)?;
    validate_rdb_instance_config(cfg, appid, instance_id, owner_user_id.is_some())?;
    let base_dir = resolve_partition_base_dir(
        &runtime.buckyos_root_dir,
        appid,
        owner_user_id.as_deref(),
        partition,
    )?;
    ensure_partition_declared(cfg, appid, instance_id, partition, &base_dir)?;

    resolve_instance(
        cfg,
        &runtime.buckyos_root_dir,
        appid,
        owner_user_id.as_deref(),
        instance_id,
        partition,
        true,
    )
}

fn ensure_partition_declared(
    cfg: &RdbInstanceConfig,
    appid: &str,
    instance_id: &str,
    partition: RdbPartition,
    base_dir: &Path,
) -> Result<()> {
    if cfg.partitions.contains(&partition) {
        return Ok(());
    }
    Err(rdb_error(
        RDB_ERR_PARTITION_NOT_DECLARED,
        appid,
        instance_id,
        Some(partition),
        &base_dir.to_string_lossy(),
        "requested partition is not declared by the instance",
    ))
}

pub async fn list_rdb_instances(
    appid: &str,
    owner_user_id: Option<String>,
) -> Result<Vec<(String, RdbPartition, String)>> {
    validate_identity(appid, owner_user_id.as_deref(), "<list>")?;
    let runtime = get_buckyos_api_runtime()?;
    let sys_cfg = runtime.get_system_config_client().await?;
    let configs = load_install_rdb_configs(&sys_cfg, appid, owner_user_id.as_deref()).await?;
    let mut instance_ids = configs.keys().cloned().collect::<Vec<_>>();
    instance_ids.sort();
    let mut resolved = Vec::new();
    for instance_id in instance_ids {
        validate_identity(appid, owner_user_id.as_deref(), &instance_id)?;
        let cfg = &configs[&instance_id];
        validate_rdb_instance_config(cfg, appid, &instance_id, owner_user_id.is_some())?;
        for partition in &cfg.partitions {
            let instance = resolve_instance(
                cfg,
                &runtime.buckyos_root_dir,
                appid,
                owner_user_id.as_deref(),
                &instance_id,
                *partition,
                false,
            )?;
            resolved.push((instance_id.clone(), *partition, instance.connection));
        }
    }
    Ok(resolved)
}

pub fn validate_rdb_instance_config(
    cfg: &RdbInstanceConfig,
    appid: &str,
    instance_id: &str,
    is_app: bool,
) -> Result<()> {
    if cfg.partitions.is_empty() {
        return Err(rdb_error(
            "invalid_partitions",
            appid,
            instance_id,
            None,
            "<unresolved>",
            "partitions must not be empty",
        ));
    }
    let mut seen = HashSet::with_capacity(cfg.partitions.len());
    for partition in &cfg.partitions {
        if !seen.insert(*partition) {
            return Err(rdb_error(
                "invalid_partitions",
                appid,
                instance_id,
                Some(*partition),
                "<unresolved>",
                "partitions must not contain duplicates",
            ));
        }
        if is_app && matches!(partition, RdbPartition::Local | RdbPartition::Storage) {
            return Err(rdb_error(
                RDB_ERR_PARTITION_NOT_ALLOWED_FOR_APP,
                appid,
                instance_id,
                Some(*partition),
                "<unresolved>",
                "AppService instances may only use user_data or cache",
            ));
        }
    }
    Ok(())
}

async fn load_install_rdb_configs(
    sys_cfg: &SystemConfigClient,
    appid: &str,
    owner_user_id: Option<&str>,
) -> Result<HashMap<String, RdbInstanceConfig>> {
    let key = spec_key(appid, owner_user_id);
    let raw = match sys_cfg.get(&key).await {
        Ok(value) => value.value,
        Err(SystemConfigError::KeyNotFound(_)) => {
            return Err(RPCErrors::ReasonError(format!(
                "spec for appid={} not found (tried: {})",
                appid, key
            )));
        }
        Err(err) => {
            return Err(RPCErrors::ReasonError(format!(
                "read spec {} failed: {}",
                key, err
            )));
        }
    };
    let view: SpecInstallView = serde_json::from_str(&raw)
        .map_err(|err| RPCErrors::ReasonError(format!("parse spec at {} failed: {}", key, err)))?;
    Ok(view.spec_config.rdb_instances)
}

fn get_config<'a>(
    configs: &'a HashMap<String, RdbInstanceConfig>,
    appid: &str,
    instance_id: &str,
) -> Result<&'a RdbInstanceConfig> {
    configs.get(instance_id).ok_or_else(|| {
        RPCErrors::ReasonError(format!(
            "rdb instance {} not declared in spec_config.rdb_instances for appid={}",
            instance_id, appid
        ))
    })
}

fn spec_key(appid: &str, owner_user_id: Option<&str>) -> String {
    match owner_user_id {
        Some(user) => format!("users/{}/apps/{}/spec", user, appid),
        None => format!("services/{}/spec", appid),
    }
}

fn pick_schema(cfg: &RdbInstanceConfig) -> Option<String> {
    cfg.schema
        .get(&cfg.backend)
        .filter(|sql| !sql.trim().is_empty())
        .cloned()
}

fn resolve_instance(
    cfg: &RdbInstanceConfig,
    buckyos_root: &Path,
    appid: &str,
    owner_user_id: Option<&str>,
    instance_id: &str,
    partition: RdbPartition,
    ensure_dir: bool,
) -> Result<RdbInstance> {
    let base_dir = resolve_partition_base_dir(buckyos_root, appid, owner_user_id, partition)?;
    let context = ConnectionResolveContext {
        buckyos_root,
        appid,
        owner_user_id,
        instance_id,
        partition,
        base_dir: &base_dir,
        ensure_dir,
    };
    let connection = build_connection_string(cfg, &context)?;
    Ok(RdbInstance {
        backend: cfg.backend,
        version: cfg.version,
        partition,
        connection,
        schema: pick_schema(cfg),
    })
}

struct ConnectionResolveContext<'a> {
    buckyos_root: &'a Path,
    appid: &'a str,
    owner_user_id: Option<&'a str>,
    instance_id: &'a str,
    partition: RdbPartition,
    base_dir: &'a Path,
    ensure_dir: bool,
}

fn build_connection_string(
    cfg: &RdbInstanceConfig,
    context: &ConnectionResolveContext<'_>,
) -> Result<String> {
    let buckyos_root = context.buckyos_root;
    let appid = context.appid;
    let owner_user_id = context.owner_user_id;
    let instance_id = context.instance_id;
    let partition = context.partition;
    let base_dir = context.base_dir;
    let ensure_dir = context.ensure_dir;
    let partdata = base_dir.to_string_lossy();
    if partition != RdbPartition::UserData && cfg.connection.contains("$appdata") {
        return Err(rdb_error(
            RDB_ERR_PARTITION_PLACEHOLDER_CONFLICT,
            appid,
            instance_id,
            Some(partition),
            &partdata,
            "$appdata may only be used by the user_data partition",
        ));
    }
    if cfg.partitions.len() > 1
        && !cfg.connection.is_empty()
        && !cfg.connection.contains("$partdata")
        && !cfg.connection.contains("$partition")
    {
        return Err(rdb_error(
            RDB_ERR_PARTITION_PLACEHOLDER_CONFLICT,
            appid,
            instance_id,
            Some(partition),
            &partdata,
            "multi-partition connection must contain $partdata or $partition",
        ));
    }
    if cfg.backend == RdbBackend::Sqlite && !cfg.connection.is_empty() {
        let uses_partition_base = cfg.connection.contains("$partdata")
            || (partition == RdbPartition::UserData && cfg.connection.contains("$appdata"));
        if !uses_partition_base {
            return Err(rdb_error(
                RDB_ERR_PARTITION_PATH_ESCAPE,
                appid,
                instance_id,
                Some(partition),
                &partdata,
                "sqlite connection must derive its path from $partdata or $appdata",
            ));
        }
    }

    let template = if cfg.connection.is_empty() {
        match cfg.backend {
            RdbBackend::Sqlite => {
                format!("sqlite://{}/{}.db?mode=rwc", partdata, instance_id)
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

    let appdata =
        resolve_partition_base_dir(buckyos_root, appid, owner_user_id, RdbPartition::UserData)?;
    let resolved = template
        .replace("$partdata", &partdata)
        .replace("$appdata", &appdata.to_string_lossy())
        .replace("$partition", partition.as_str())
        .replace("$instance", instance_id);

    if cfg.backend != RdbBackend::Sqlite {
        return Ok(resolved);
    }
    let resolved = normalize_sqlite_url(&resolved);
    validate_sqlite_partition_path(
        &resolved,
        base_dir,
        appid,
        instance_id,
        partition,
        ensure_dir,
    )?;
    Ok(resolved)
}

fn resolve_partition_base_dir(
    buckyos_root: &Path,
    appid: &str,
    owner_user_id: Option<&str>,
    partition: RdbPartition,
) -> Result<PathBuf> {
    validate_path_component("appid", appid)?;
    if let Some(user) = owner_user_id {
        validate_path_component("owner_user_id", user)?;
    }
    let (partition_root, base_dir) = match partition {
        RdbPartition::UserData => {
            let root = buckyos_root.join("data");
            let base = match owner_user_id {
                Some(user) => root.join(user).join(appid),
                None => root.join(appid),
            };
            (root, base)
        }
        RdbPartition::Local => {
            let root = buckyos_root.join("local");
            (root.clone(), root.join(appid))
        }
        RdbPartition::Cache => {
            let root = buckyos_root.join("data").join("cache");
            let base = match owner_user_id {
                Some(user) => root.join(user).join(appid),
                None => root.join(appid),
            };
            (root, base)
        }
        RdbPartition::Storage => {
            let root = buckyos_root.join("storage");
            (root.clone(), root.join(appid))
        }
    };
    let normalized_root = normalize_lexical_path(&partition_root).ok_or_else(|| {
        RPCErrors::ReasonError(format!(
            "invalid partition root {}",
            partition_root.display()
        ))
    })?;
    let normalized_base = normalize_lexical_path(&base_dir).ok_or_else(|| {
        RPCErrors::ReasonError(format!("invalid partition base {}", base_dir.display()))
    })?;
    if !normalized_base.starts_with(&normalized_root) {
        return Err(rdb_error(
            RDB_ERR_PARTITION_PATH_ESCAPE,
            appid,
            "<base>",
            Some(partition),
            &normalized_base.to_string_lossy(),
            "partition base escapes its partition root",
        ));
    }
    Ok(normalized_base)
}

fn normalize_sqlite_url(connection: &str) -> String {
    let body = connection
        .strip_prefix("sqlite://")
        .or_else(|| connection.strip_prefix("sqlite:"))
        .unwrap_or(connection);
    let (path, params) = match body.split_once('?') {
        Some((path, params)) => (path, Some(params)),
        None => (body, None),
    };
    if path.is_empty() || path == ":memory:" {
        return connection.to_string();
    }
    let path = path.replace('\\', "/");
    match params {
        Some(params) => format!("sqlite:{}?{}", path, params),
        None => format!("sqlite:{}", path),
    }
}

fn validate_sqlite_partition_path(
    connection: &str,
    base_dir: &Path,
    appid: &str,
    instance_id: &str,
    partition: RdbPartition,
    ensure_dir: bool,
) -> Result<()> {
    let path_str = connection
        .strip_prefix("sqlite:")
        .and_then(|body| body.split('?').next())
        .unwrap_or_default();
    let path = Path::new(path_str);
    let normalized_path = normalize_lexical_path(path);
    let normalized_base = normalize_lexical_path(base_dir);
    let valid = !path_str.is_empty()
        && !path_str.contains('%')
        && path.is_absolute()
        && normalized_path
            .as_ref()
            .zip(normalized_base.as_ref())
            .is_some_and(|(path, base)| path.starts_with(base));
    if !valid {
        return Err(rdb_error(
            RDB_ERR_PARTITION_PATH_ESCAPE,
            appid,
            instance_id,
            Some(partition),
            path_str,
            "sqlite path must be absolute and remain inside the partition base",
        ));
    }
    let normalized_path = normalized_path.expect("checked above");
    let normalized_base = normalized_base.expect("checked above");
    validate_existing_ancestor(
        &normalized_path,
        &normalized_base,
        appid,
        instance_id,
        partition,
    )?;
    if !ensure_dir {
        return Ok(());
    }
    std::fs::create_dir_all(&normalized_base).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "create rdb partition base {} failed: {}",
            normalized_base.display(),
            error
        ))
    })?;
    let parent = normalized_path.parent().ok_or_else(|| {
        rdb_error(
            RDB_ERR_PARTITION_PATH_ESCAPE,
            appid,
            instance_id,
            Some(partition),
            path_str,
            "sqlite path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "create sqlite dir {} failed: {}",
            parent.display(),
            error
        ))
    })?;
    validate_existing_ancestor(
        &normalized_path,
        &normalized_base,
        appid,
        instance_id,
        partition,
    )
}

fn validate_existing_ancestor(
    path: &Path,
    base_dir: &Path,
    appid: &str,
    instance_id: &str,
    partition: RdbPartition,
) -> Result<()> {
    if !base_dir.exists() {
        return Ok(());
    }
    let canonical_base = base_dir.canonicalize().map_err(|error| {
        RPCErrors::ReasonError(format!(
            "canonicalize partition base {} failed: {}",
            base_dir.display(),
            error
        ))
    })?;
    let mut ancestor = path.to_path_buf();
    while !ancestor.exists() && ancestor.pop() {}
    if ancestor.exists() {
        let canonical_ancestor = ancestor.canonicalize().map_err(|error| {
            RPCErrors::ReasonError(format!(
                "canonicalize sqlite path {} failed: {}",
                ancestor.display(),
                error
            ))
        })?;
        if !canonical_ancestor.starts_with(&canonical_base) {
            return Err(rdb_error(
                RDB_ERR_PARTITION_PATH_ESCAPE,
                appid,
                instance_id,
                Some(partition),
                &canonical_ancestor.to_string_lossy(),
                "sqlite path escapes through a symlink",
            ));
        }
    }
    Ok(())
}

fn normalize_lexical_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    Some(normalized)
}

fn validate_identity(appid: &str, owner_user_id: Option<&str>, instance_id: &str) -> Result<()> {
    validate_path_component("appid", appid)?;
    if let Some(user) = owner_user_id {
        validate_path_component("owner_user_id", user)?;
    }
    if instance_id != "<list>" {
        validate_path_component("instance_id", instance_id)?;
    }
    Ok(())
}

fn validate_path_component(name: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(RPCErrors::ReasonError(format!(
            "{} is not a safe path component: {}",
            name, value
        )));
    }
    Ok(())
}

fn rdb_error(
    code: &str,
    appid: &str,
    instance_id: &str,
    partition: Option<RdbPartition>,
    path: &str,
    detail: &str,
) -> RPCErrors {
    RPCErrors::ReasonError(format!(
        "{}: appid={} instance_id={} partition={} path={} detail={}",
        code,
        appid,
        instance_id,
        partition.map_or("<none>", RdbPartition::as_str),
        if path.is_empty() {
            "<unresolved>"
        } else {
            path
        },
        detail
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_config(partitions: Vec<RdbPartition>, connection: &str) -> RdbInstanceConfig {
        RdbInstanceConfig {
            backend: RdbBackend::Sqlite,
            version: 3,
            schema: HashMap::from([(
                RdbBackend::Sqlite,
                "CREATE TABLE t(id INTEGER);".to_string(),
            )]),
            connection: connection.to_string(),
            partitions,
        }
    }

    fn resolve_for_test(
        root: &Path,
        cfg: &RdbInstanceConfig,
        partition: RdbPartition,
        ensure_dir: bool,
    ) -> Result<RdbInstance> {
        validate_rdb_instance_config(cfg, "demo", "main", false)?;
        resolve_instance(cfg, root, "demo", None, "main", partition, ensure_dir)
    }

    #[test]
    fn old_spec_defaults_to_user_data_and_keeps_connection() {
        let raw = r#"{
            "backend": "sqlite",
            "version": 3,
            "schema": { "sqlite": "CREATE TABLE t(id INTEGER);" },
            "connection": "sqlite://$appdata/main.db"
        }"#;
        let cfg: RdbInstanceConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.partitions, vec![RdbPartition::UserData]);
        let instance = resolve_for_test(
            Path::new("/opt/buckyos"),
            &cfg,
            RdbPartition::UserData,
            false,
        )
        .unwrap();
        assert_eq!(instance.partition, RdbPartition::UserData);
        assert_eq!(instance.connection, "sqlite:/opt/buckyos/data/demo/main.db");
    }

    #[test]
    fn old_spec_keeps_cross_app_connection() {
        let cfg: RdbInstanceConfig =
            serde_json::from_str(r#"{"backend":"sqlite","version":1,"schema":{},"connection":""}"#)
                .unwrap();
        let instance = resolve_instance(
            &cfg,
            Path::new("/opt/buckyos"),
            "demo",
            Some("alice"),
            "main",
            RdbPartition::UserData,
            false,
        )
        .unwrap();
        assert_eq!(
            instance.connection,
            "sqlite:/opt/buckyos/data/alice/demo/main.db?mode=rwc"
        );
    }

    #[test]
    fn implicit_partition_rejects_multi_partition_instance() {
        let cfg = sqlite_config(vec![RdbPartition::UserData, RdbPartition::Local], "");
        let error = select_implicit_partition(
            &cfg,
            "demo",
            "main",
            "/opt/buckyos/data/demo,/opt/buckyos/local/demo",
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains(RDB_ERR_PARTITION_AMBIGUOUS));
        assert!(message.contains("appid=demo"));
        assert!(message.contains("instance_id=main"));
    }

    #[test]
    fn undeclared_partition_is_rejected() {
        let cfg = sqlite_config(vec![RdbPartition::UserData], "");
        let error = ensure_partition_declared(
            &cfg,
            "demo",
            "main",
            RdbPartition::Cache,
            Path::new("/opt/buckyos/data/cache/demo"),
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains(RDB_ERR_PARTITION_NOT_DECLARED));
        assert!(message.contains("partition=cache"));
        assert!(message.contains("path=/opt/buckyos/data/cache/demo"));
    }

    #[test]
    fn partition_base_dirs_follow_root() {
        let root = Path::new("/srv/custom-buckyos");
        assert_eq!(
            resolve_partition_base_dir(root, "demo", None, RdbPartition::UserData).unwrap(),
            root.join("data/demo")
        );
        assert_eq!(
            resolve_partition_base_dir(root, "demo", None, RdbPartition::Local).unwrap(),
            root.join("local/demo")
        );
        assert_eq!(
            resolve_partition_base_dir(root, "demo", None, RdbPartition::Cache).unwrap(),
            root.join("data/cache/demo")
        );
        assert_eq!(
            resolve_partition_base_dir(root, "demo", None, RdbPartition::Storage).unwrap(),
            root.join("storage/demo")
        );
        assert_eq!(
            resolve_partition_base_dir(root, "demo", Some("alice"), RdbPartition::Cache).unwrap(),
            root.join("data/cache/alice/demo")
        );
    }

    #[test]
    fn built_in_instances_declare_expected_partitions() {
        assert_eq!(
            crate::task_manager_default_rdb_instance_config().partitions,
            vec![RdbPartition::UserData, RdbPartition::Local]
        );
        assert_eq!(
            crate::task_dispatcher_default_rdb_instance_config().partitions,
            vec![RdbPartition::Local]
        );
        for partitions in [
            crate::repo_service_default_rdb_instance_config().partitions,
            crate::msg_center_default_rdb_instance_config().partitions,
            crate::aicc_usage_log_default_rdb_instance_config().partitions,
        ] {
            assert_eq!(partitions, vec![RdbPartition::UserData]);
        }
    }

    #[test]
    fn multi_partition_default_resolves_to_distinct_connections() {
        let cfg = sqlite_config(vec![RdbPartition::UserData, RdbPartition::Local], "");
        let user = resolve_for_test(
            Path::new("/opt/buckyos"),
            &cfg,
            RdbPartition::UserData,
            false,
        )
        .unwrap();
        let local =
            resolve_for_test(Path::new("/opt/buckyos"), &cfg, RdbPartition::Local, false).unwrap();
        assert_eq!(
            user.connection,
            "sqlite:/opt/buckyos/data/demo/main.db?mode=rwc"
        );
        assert_eq!(
            local.connection,
            "sqlite:/opt/buckyos/local/demo/main.db?mode=rwc"
        );
    }

    #[test]
    fn empty_and_duplicate_partitions_are_rejected() {
        let empty = sqlite_config(Vec::new(), "");
        assert!(validate_rdb_instance_config(&empty, "demo", "main", false).is_err());
        let duplicate = sqlite_config(vec![RdbPartition::Cache, RdbPartition::Cache], "");
        assert!(validate_rdb_instance_config(&duplicate, "demo", "main", false).is_err());
    }

    #[test]
    fn app_rejects_host_partitions() {
        for partition in [RdbPartition::Local, RdbPartition::Storage] {
            let cfg = sqlite_config(vec![partition], "");
            let error = validate_rdb_instance_config(&cfg, "demo", "main", true).unwrap_err();
            assert!(error
                .to_string()
                .contains(RDB_ERR_PARTITION_NOT_ALLOWED_FOR_APP));
        }
    }

    #[test]
    fn placeholder_conflicts_are_rejected() {
        let cfg = sqlite_config(vec![RdbPartition::Local], "sqlite://$appdata/main.db");
        let error = resolve_for_test(Path::new("/opt/buckyos"), &cfg, RdbPartition::Local, false)
            .unwrap_err();
        assert!(error
            .to_string()
            .contains(RDB_ERR_PARTITION_PLACEHOLDER_CONFLICT));

        let cfg = sqlite_config(
            vec![RdbPartition::UserData, RdbPartition::Local],
            "sqlite:///opt/buckyos/shared.db",
        );
        let error = resolve_for_test(
            Path::new("/opt/buckyos"),
            &cfg,
            RdbPartition::UserData,
            false,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains(RDB_ERR_PARTITION_PLACEHOLDER_CONFLICT));
    }

    #[test]
    fn partition_placeholder_distinguishes_postgres_databases() {
        let cfg = RdbInstanceConfig {
            backend: RdbBackend::Postgres,
            version: 2,
            schema: HashMap::new(),
            connection: "postgres://svc@pg/task_$partition".to_string(),
            partitions: vec![RdbPartition::UserData, RdbPartition::Local],
        };
        let local =
            resolve_for_test(Path::new("/opt/buckyos"), &cfg, RdbPartition::Local, false).unwrap();
        assert_eq!(local.connection, "postgres://svc@pg/task_local");
    }

    #[test]
    fn sqlite_path_escape_is_rejected() {
        for connection in [
            "sqlite://$partdata/../../data/evil.db",
            "sqlite:///tmp/evil.db",
            "sqlite:relative.db",
            "sqlite://$partdata/%2e%2e/evil.db",
            "sqlite:///opt/buckyos/local/demo/hardcoded.db",
        ] {
            let cfg = sqlite_config(vec![RdbPartition::Local], connection);
            let error =
                resolve_for_test(Path::new("/opt/buckyos"), &cfg, RdbPartition::Local, false)
                    .unwrap_err();
            assert!(error.to_string().contains(RDB_ERR_PARTITION_PATH_ESCAPE));
        }
    }

    #[test]
    fn missing_partition_directory_is_created() {
        let root = std::env::temp_dir().join(format!("rdb-mgr-{}", uuid::Uuid::new_v4()));
        let cfg = sqlite_config(vec![RdbPartition::Local], "");
        let instance = resolve_for_test(&root, &cfg, RdbPartition::Local, true).unwrap();
        assert!(root.join("local/demo").is_dir());
        assert!(instance
            .connection
            .ends_with("/local/demo/main.db?mode=rwc"));
        let path = instance
            .connection
            .strip_prefix("sqlite:")
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        rusqlite::Connection::open(path).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("rdb-mgr-{}", uuid::Uuid::new_v4()));
        let outside = std::env::temp_dir().join(format!("rdb-outside-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("local/demo")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("local/demo/link")).unwrap();
        let cfg = sqlite_config(vec![RdbPartition::Local], "sqlite://$partdata/link/evil.db");
        let error = resolve_for_test(&root, &cfg, RdbPartition::Local, true).unwrap_err();
        assert!(error.to_string().contains(RDB_ERR_PARTITION_PATH_ESCAPE));
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn normalize_sqlite_url_keeps_windows_drive_letter() {
        assert_eq!(
            normalize_sqlite_url("sqlite://C:\\Users\\dev\\buckyos\\data\\main.db?mode=rwc"),
            "sqlite:C:/Users/dev/buckyos/data/main.db?mode=rwc"
        );
    }

    #[test]
    fn normalize_sqlite_url_keeps_posix_path() {
        assert_eq!(
            normalize_sqlite_url("sqlite:///opt/buckyos/data/main.db?mode=rwc"),
            "sqlite:/opt/buckyos/data/main.db?mode=rwc"
        );
        assert_eq!(
            normalize_sqlite_url("sqlite:/opt/buckyos/data/main.db"),
            "sqlite:/opt/buckyos/data/main.db"
        );
    }

    #[test]
    fn spec_key_for_app_and_service() {
        assert_eq!(
            spec_key("demo", Some("alice")),
            "users/alice/apps/demo/spec"
        );
        assert_eq!(spec_key("verify-hub", None), "services/verify-hub/spec");
    }
}
