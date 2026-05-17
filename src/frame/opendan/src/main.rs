//! opendan binary entry.
//!
//! Wires the §9 components together: bootstrap shared deps (aicc + worklog) →
//! open `AIAgent` over the configured agent root → run the dispatcher loop.
//! SIGINT triggers a graceful shutdown via `AIAgent::shutdown`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, load_app_identity_from_env,
    set_buckyos_api_runtime, BuckyOSRuntimeType, KEventClient, OPENDAN_SERVICE_NAME,
};
use buckyos_kit::{get_buckyos_root_dir, init_logging};
use log::{error, info, warn};
use name_lib::{AgentDocument, DIDDocumentTrait, EncodedDocument};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs as sync_fs;
use tokio::fs;

use opendan::agent::AIAgent;
use opendan::agent_config::AgentTomlFile;
use opendan::ai_runtime::AgentRuntime;
use opendan::worklog::{WorklogService, WorklogToolConfig};

const WORKLOG_DB_ENV: &str = "OPENDAN_WORKLOG_DB";
const DEFAULT_WORKLOG_DB: &str = "/opt/buckyos/opendan/worklog.db";
const AGENT_ROOT_ENV: &str = "OPENDAN_AGENT_ROOT";
const DEFAULT_AGENT_ROOT: &str = "/opt/buckyos/opendan/agent";
const OPENDAN_APPID_ENV: [&str; 3] = ["OPENDAN_APPID", "BUCKYOS_APP_ID", "OPENDAN_AGENT_ID"];
const OPENDAN_OWNER_ENV: [&str; 2] = ["OPENDAN_AGENT_OWNER", "BUCKYOS_OWNER_USER_ID"];
const OPENDAN_AGENT_BIN_ENV: [&str; 3] = [
    "OPENDAN_AGENT_BIN",
    "BUCKYOS_PKG_DIR",
    "BUCKYOS_PKG_SOURCE_DIR",
];
const ROOTFS_SYNC_MANIFEST: &str = ".meta/rootfs_sync.json";
const ROOTFS_SYNC_VERSION: u32 = 1;

#[derive(Debug, Default)]
struct StartupArgs {
    appid: Option<String>,
    owner_id: Option<String>,
    agent_root: Option<PathBuf>,
    agent_bin: Option<PathBuf>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RootfsSyncManifest {
    version: u32,
    files: BTreeMap<String, RootfsSyncEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RootfsSyncEntry {
    source_sha256: String,
    installed_sha256: String,
}

fn parse_startup_args_from_iter<I, S>(args: I) -> Result<StartupArgs>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parsed = StartupArgs::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        match arg {
            "--appid" | "--app-id" | "--agent-id" => {
                parsed.appid = Some(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for {arg}"))?,
                );
            }
            "--owner-id" | "--owner-user-id" => {
                parsed.owner_id = Some(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for {arg}"))?,
                );
            }
            "--agent-root" | "--agent-env" => {
                parsed.agent_root = Some(PathBuf::from(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for {arg}"))?,
                ));
            }
            "--agent-bin" => {
                parsed.agent_bin = Some(PathBuf::from(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for {arg}"))?,
                ));
            }
            other if other.starts_with("--appid=") => {
                parsed.appid = Some(other["--appid=".len()..].to_string());
            }
            other if other.starts_with("--app-id=") => {
                parsed.appid = Some(other["--app-id=".len()..].to_string());
            }
            other if other.starts_with("--agent-id=") => {
                parsed.appid = Some(other["--agent-id=".len()..].to_string());
            }
            other if other.starts_with("--owner-id=") => {
                parsed.owner_id = Some(other["--owner-id=".len()..].to_string());
            }
            other if other.starts_with("--owner-user-id=") => {
                parsed.owner_id = Some(other["--owner-user-id=".len()..].to_string());
            }
            other if other.starts_with("--agent-root=") => {
                parsed.agent_root = Some(PathBuf::from(&other["--agent-root=".len()..]));
            }
            other if other.starts_with("--agent-env=") => {
                parsed.agent_root = Some(PathBuf::from(&other["--agent-env=".len()..]));
            }
            other if other.starts_with("--agent-bin=") => {
                parsed.agent_bin = Some(PathBuf::from(&other["--agent-bin=".len()..]));
            }
            other if !other.starts_with('-') && parsed.appid.is_none() => {
                parsed.appid = Some(other.to_string());
            }
            _ => {}
        }
    }
    Ok(parsed)
}

fn parse_startup_args() -> Result<StartupArgs> {
    parse_startup_args_from_iter(std::env::args().skip(1))
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn resolve_appid(startup: &StartupArgs) -> Result<String> {
    if let Some(appid) = startup
        .appid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(appid.to_string());
    }
    if let Some(appid) = first_env(&OPENDAN_APPID_ENV) {
        return Ok(appid);
    }
    if let Some((appid, _owner_id)) = load_app_identity_from_env()
        .map_err(|err| anyhow!("load app identity from app_instance_config failed: {err}"))?
    {
        return Ok(appid);
    }
    Err(anyhow!(
        "appid is required; pass --appid <id>, a positional appid, or set one of {:?}",
        OPENDAN_APPID_ENV
    ))
}

fn resolve_owner_id(startup: &StartupArgs) -> Result<Option<String>> {
    if let Some(owner_id) = startup
        .owner_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(owner_id.to_string()));
    }
    if let Some(owner_id) = first_env(&OPENDAN_OWNER_ENV) {
        return Ok(Some(owner_id));
    }
    Ok(load_app_identity_from_env()
        .map_err(|err| anyhow!("load app identity from app_instance_config failed: {err}"))?
        .map(|(_appid, owner_id)| owner_id))
}

fn resolve_agent_root(startup: &StartupArgs, appid: &str, owner_id: Option<&str>) -> PathBuf {
    if let Some(path) = startup.agent_root.clone() {
        return path;
    }
    if let Ok(path) = std::env::var(AGENT_ROOT_ENV) {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Ok(path) = std::env::var("BUCKYOS_DATA_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Some(owner_id) = owner_id.map(str::trim).filter(|value| !value.is_empty()) {
        return get_buckyos_root_dir()
            .join("data")
            .join("home")
            .join(owner_id)
            .join(".local")
            .join("share")
            .join(appid);
    }
    PathBuf::from(DEFAULT_AGENT_ROOT)
}

fn resolve_agent_package_root(startup: &StartupArgs, appid: &str) -> Option<PathBuf> {
    if let Some(path) = startup.agent_bin.clone() {
        return Some(path);
    }
    if let Some(path) = first_env(&OPENDAN_AGENT_BIN_ENV).map(PathBuf::from) {
        return Some(path);
    }
    for path in [
        PathBuf::from("/opt/buckyos/bin").join(appid),
        PathBuf::from("/opt/buckyos/bin").join(format!("buckyos_{appid}")),
    ] {
        if path.is_dir() {
            return Some(path);
        }
    }
    None
}

async fn init_agent_rootfs(agent_root: &Path, package_root: Option<&Path>) -> Result<()> {
    ensure_agent_rootfs_layout(agent_root)
        .await
        .with_context(|| format!("initialize AgentRootFS layout at {}", agent_root.display()))?;
    let Some(package_root) = package_root else {
        warn!(
            "opendan.rootfs: no agent package/bin directory found; skip release sync into {}",
            agent_root.display()
        );
        return Ok(());
    };
    if !package_root.is_dir() {
        return Err(anyhow!(
            "agent package/bin directory does not exist or is not a directory: {}",
            package_root.display()
        ));
    }
    sync_agent_rootfs_from_package(package_root, agent_root)
        .await
        .with_context(|| {
            format!(
                "sync AgentRootFS from {} to {}",
                package_root.display(),
                agent_root.display()
            )
        })?;
    Ok(())
}

async fn ensure_agent_rootfs_layout(agent_root: &Path) -> Result<()> {
    for rel in [
        "",
        ".meta",
        "users",
        "memory",
        "notepads",
        "skills",
        "tools",
        "behaviors",
        "archive",
        "archive/skills",
        "archive/sessions",
        "archive/workspace",
        "workspace",
        "sessions",
    ] {
        fs::create_dir_all(agent_root.join(rel))
            .await
            .with_context(|| {
                format!("create AgentRootFS dir {}", agent_root.join(rel).display())
            })?;
    }
    Ok(())
}

async fn sync_agent_rootfs_from_package(package_root: &Path, agent_root: &Path) -> Result<()> {
    let manifest_path = agent_root.join(ROOTFS_SYNC_MANIFEST);
    let mut manifest = load_rootfs_sync_manifest(&manifest_path).await?;
    manifest.version = ROOTFS_SYNC_VERSION;

    let mut copied = 0usize;
    let mut updated = 0usize;
    let mut preserved = 0usize;
    let mut tracked = 0usize;
    for source in collect_package_files(package_root)? {
        let rel = source
            .strip_prefix(package_root)
            .with_context(|| format!("strip package prefix for {}", source.display()))?;
        let rel_key = rootfs_rel_key(rel);
        if rel_key == ROOTFS_SYNC_MANIFEST {
            continue;
        }
        let target = agent_root.join(rel);
        let source_hash = sha256_file(&source)?;
        let existing_hash = if target.is_file() {
            Some(sha256_file(&target)?)
        } else {
            None
        };
        let previous = manifest.files.get(&rel_key);
        let local_unmodified = match (existing_hash.as_deref(), previous) {
            (None, _) => true,
            (Some(current), Some(entry)) => current == entry.installed_sha256,
            (Some(current), None) => current == source_hash,
        };

        if existing_hash.is_none() {
            copy_package_file(&source, &target)?;
            copied += 1;
        } else if local_unmodified {
            if existing_hash.as_deref() != Some(source_hash.as_str()) {
                copy_package_file(&source, &target)?;
                updated += 1;
            } else {
                tracked += 1;
            }
        } else {
            preserved += 1;
            warn!(
                "opendan.rootfs: preserve locally modified file during package sync: {}",
                target.display()
            );
        }

        manifest.files.insert(
            rel_key,
            RootfsSyncEntry {
                source_sha256: source_hash.clone(),
                installed_sha256: if local_unmodified {
                    source_hash
                } else {
                    previous
                        .map(|entry| entry.installed_sha256.clone())
                        .unwrap_or_else(|| existing_hash.unwrap_or_default())
                },
            },
        );
    }
    save_rootfs_sync_manifest(&manifest_path, &manifest).await?;
    info!(
        "opendan.rootfs: synced package={} root={} copied={} updated={} tracked={} preserved_modified={}",
        package_root.display(),
        agent_root.display(),
        copied,
        updated,
        tracked,
        preserved
    );
    Ok(())
}

async fn load_rootfs_sync_manifest(path: &Path) -> Result<RootfsSyncManifest> {
    match fs::read_to_string(path).await {
        Ok(raw) => serde_json::from_str::<RootfsSyncManifest>(&raw)
            .with_context(|| format!("parse {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(RootfsSyncManifest::default()),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

async fn save_rootfs_sync_manifest(path: &Path, manifest: &RootfsSyncManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let raw = serde_json::to_vec_pretty(manifest).context("serialize rootfs sync manifest")?;
    fs::write(path, raw)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn collect_package_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect_package_files_inner(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_package_files_inner(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in sync_fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry.with_context(|| format!("read entry under {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("read file type {}", path.display()))?;
        if file_type.is_symlink() {
            warn!("opendan.rootfs: skip package symlink {}", path.display());
            continue;
        }
        if file_type.is_dir() {
            collect_package_files_inner(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn copy_package_file(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        sync_fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    sync_fs::copy(source, target)
        .with_context(|| format!("copy {} -> {}", source.display(), target.display()))?;
    let mut permissions = sync_fs::metadata(source)
        .with_context(|| format!("metadata {}", source.display()))?
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    sync_fs::set_permissions(target, permissions)
        .with_context(|| format!("set permissions {}", target.display()))?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = sync_fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(hex::encode(digest))
}

fn rootfs_rel_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

async fn load_agent_document(appid: &str) -> Result<AgentDocument> {
    let runtime = get_buckyos_api_runtime().context("load runtime failed before agent doc")?;
    let client = runtime
        .get_system_config_client()
        .await
        .context("init system_config client for opendan failed")?;
    let key = format!("agents/{appid}/doc");
    let value = client
        .get(&key)
        .await
        .with_context(|| format!("load agent document failed: key={key}"))?
        .value;
    let encoded = EncodedDocument::from_str(value)
        .map_err(|err| anyhow!("decode agent document failed: key={key} err={err}"))?;
    AgentDocument::decode(&encoded, None)
        .map_err(|err| anyhow!("decode AgentDocument failed: key={key} err={err}"))
}

async fn write_agent_did_to_toml(agent_root: &PathBuf, appid: &str, agent_did: &str) -> Result<()> {
    fs::create_dir_all(agent_root)
        .await
        .with_context(|| format!("create agent root at {}", agent_root.display()))?;
    let path = agent_root.join("agent.toml");
    let mut toml = match fs::read_to_string(&path).await {
        Ok(raw) => toml::from_str::<AgentTomlFile>(&raw)
            .with_context(|| format!("parse {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => AgentTomlFile::default(),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    toml.identity.agent_did = agent_did.to_string();
    if toml.identity.display_name.trim().is_empty() {
        toml.identity.display_name = appid.to_string();
    }
    let raw = toml::to_string_pretty(&toml).context("serialize agent.toml")?;
    fs::write(&path, raw)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

async fn bootstrap(appid: &str, owner_id: Option<String>) -> Result<Arc<AgentRuntime>> {
    let mut runtime = init_buckyos_api_runtime(appid, owner_id, BuckyOSRuntimeType::AppService)
        .await
        .map_err(|err| anyhow!("init buckyos runtime failed: {err}"))?;
    runtime
        .login()
        .await
        .map_err(|err| anyhow!("opendan login failed: {err}"))?;
    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow!("register buckyos runtime failed: {err}"))?;

    let api_runtime =
        get_buckyos_api_runtime().map_err(|err| anyhow!("load buckyos runtime failed: {err}"))?;
    let aicc = api_runtime
        .get_aicc_client()
        .await
        .map_err(|err| anyhow!("init aicc client failed: {err}"))?;

    let worklog_db = std::env::var(WORKLOG_DB_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_WORKLOG_DB));
    let worklog = WorklogService::new(WorklogToolConfig::with_db_path(worklog_db.clone()))
        .with_context(|| format!("open worklog db at {}", worklog_db.display()))?;

    let msg_center = match api_runtime.get_msg_center_client().await {
        Ok(client) => Some(Arc::new(client)),
        Err(err) => {
            warn!("opendan.bootstrap: msg-center unavailable — inbox pump disabled: {err}");
            None
        }
    };

    // task_mgr is optional too — failing to reach the task-mgr daemon
    // should degrade async-tool dispatch (PendingTool outcomes) to an
    // inline warn, not block the agent from running.
    let task_mgr = match api_runtime.get_task_mgr_client().await {
        Ok(client) => Some(Arc::new(client)),
        Err(err) => {
            warn!("opendan.bootstrap: task-mgr unavailable — async tool dispatch disabled: {err}");
            None
        }
    };

    // KEventClient is local to the process; `source_node` only matters as a
    // tag on locally published events. Use the opendan service name so the
    // node-local view is self-describing — the actual subscription patterns
    // are derived per-agent from `agent.toml`.
    let kevent_client = Arc::new(KEventClient::new_full(OPENDAN_SERVICE_NAME, None));

    info!(
        "opendan.bootstrap: aicc=ready worklog_db={} msg_center={} task_mgr={} kevent=ready",
        worklog_db.display(),
        if msg_center.is_some() {
            "ready"
        } else {
            "unavailable"
        },
        if task_mgr.is_some() {
            "ready"
        } else {
            "unavailable"
        }
    );

    let mut runtime =
        AgentRuntime::new(Arc::new(aicc), Arc::new(worklog)).with_kevent_client(kevent_client);
    if let Some(client) = msg_center {
        runtime = runtime.with_msg_center(client);
    }
    if let Some(client) = task_mgr {
        runtime = runtime.with_task_mgr(client);
    }
    Ok(Arc::new(runtime))
}

async fn run() -> Result<()> {
    let startup = parse_startup_args().context("parse opendan startup args failed")?;
    let appid = resolve_appid(&startup)?;
    let owner_id = resolve_owner_id(&startup)?;
    info!(
        "opendan.bootstrap: appid={} owner_id={}",
        appid,
        owner_id.as_deref().unwrap_or("<runtime-env>")
    );
    let agent_root = resolve_agent_root(&startup, &appid, owner_id.as_deref());
    let runtime = bootstrap(&appid, owner_id).await?;

    let agent_doc = load_agent_document(&appid).await?;
    let agent_did = agent_doc.get_id().to_string();
    let package_root = resolve_agent_package_root(&startup, &appid);
    init_agent_rootfs(&agent_root, package_root.as_deref()).await?;
    write_agent_did_to_toml(&agent_root, &appid, &agent_did).await?;
    info!(
        "opendan.bootstrap: agent_root={} package_root={} agent_did={}",
        agent_root.display(),
        package_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        agent_did
    );

    let agent = AIAgent::open(agent_root, runtime)?;
    let agent_for_signal = agent.clone();
    tokio::spawn(async move {
        if let Err(err) = tokio::signal::ctrl_c().await {
            error!("opendan: ctrl_c handler failed: {err}");
            return;
        }
        info!("opendan: received SIGINT, requesting shutdown");
        agent_for_signal.shutdown().await;
    });

    agent.run().await?;
    info!("opendan: AIAgent::run returned cleanly");
    Ok(())
}

fn main() {
    init_logging("opendan", true);
    let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
    if let Err(err) = rt.block_on(run()) {
        error!("opendan: startup failed: {err:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_startup_args_from_iter, sync_agent_rootfs_from_package, ROOTFS_SYNC_MANIFEST,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_positional_appid() {
        let parsed = parse_startup_args_from_iter(["jarvis"]).expect("parse args");
        assert_eq!(parsed.appid.as_deref(), Some("jarvis"));
    }

    #[test]
    fn parses_appid_flag() {
        let parsed = parse_startup_args_from_iter(["--appid", "jarvis"]).expect("parse args");
        assert_eq!(parsed.appid.as_deref(), Some("jarvis"));
    }

    #[test]
    fn keeps_agent_id_as_loader_alias() {
        let parsed = parse_startup_args_from_iter(["--agent-id=jarvis"]).expect("parse args");
        assert_eq!(parsed.appid.as_deref(), Some("jarvis"));
    }

    #[test]
    fn parses_agent_bin_flag() {
        let parsed = parse_startup_args_from_iter(["--appid=jarvis", "--agent-bin", "/pkg/jarvis"])
            .expect("parse args");
        assert_eq!(
            parsed.agent_bin.unwrap(),
            std::path::PathBuf::from("/pkg/jarvis")
        );
    }

    #[tokio::test]
    async fn rootfs_sync_copies_missing_release_files() {
        let dir = tempdir().unwrap();
        let package = dir.path().join("pkg");
        let root = dir.path().join("root");
        fs::create_dir_all(package.join("tools")).unwrap();
        fs::write(package.join("role.md"), "role v1").unwrap();
        fs::write(package.join("tools/hello.sh"), "#!/bin/sh\necho hi\n").unwrap();

        sync_agent_rootfs_from_package(&package, &root)
            .await
            .expect("sync rootfs");

        assert_eq!(fs::read_to_string(root.join("role.md")).unwrap(), "role v1");
        assert!(root.join("tools/hello.sh").is_file());
        assert!(root.join(ROOTFS_SYNC_MANIFEST).is_file());
    }

    #[tokio::test]
    async fn rootfs_sync_updates_unmodified_local_file() {
        let dir = tempdir().unwrap();
        let package = dir.path().join("pkg");
        let root = dir.path().join("root");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("self.md"), "self v1").unwrap();
        sync_agent_rootfs_from_package(&package, &root)
            .await
            .expect("initial sync");

        fs::write(package.join("self.md"), "self v2").unwrap();
        sync_agent_rootfs_from_package(&package, &root)
            .await
            .expect("second sync");

        assert_eq!(fs::read_to_string(root.join("self.md")).unwrap(), "self v2");
    }

    #[tokio::test]
    async fn rootfs_sync_preserves_locally_modified_file() {
        let dir = tempdir().unwrap();
        let package = dir.path().join("pkg");
        let root = dir.path().join("root");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("role.md"), "role v1").unwrap();
        sync_agent_rootfs_from_package(&package, &root)
            .await
            .expect("initial sync");

        fs::write(root.join("role.md"), "local role").unwrap();
        fs::write(package.join("role.md"), "role v2").unwrap();
        sync_agent_rootfs_from_package(&package, &root)
            .await
            .expect("second sync");

        assert_eq!(
            fs::read_to_string(root.join("role.md")).unwrap(),
            "local role"
        );
    }
}
