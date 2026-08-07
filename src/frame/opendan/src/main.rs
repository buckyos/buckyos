//! opendan binary entry.
//!
//! Wires the §9 components together: bootstrap shared deps (aicc + worklog) →
//! open `AIAgent` over the configured agent root → run the dispatcher loop.
//! SIGINT triggers a graceful shutdown via `AIAgent::shutdown`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, load_app_identity_from_env,
    set_buckyos_api_runtime, BuckyOSRuntimeType, TaskStatus,
};
use buckyos_kit::{get_buckyos_app_data_dir, init_logging};
use log::{error, info, warn};
use name_lib::{AgentDocument, DIDDocumentTrait, EncodedDocument};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs as sync_fs;
use tokio::fs;

use opendan::agent::{AIAgent, CreateWorkSessionParams};
use opendan::agent_config::AgentTomlFile;
use opendan::agent_task_executor::TASK_TYPE_AGENT_DELEGATE;
use opendan::ai_runtime::AgentRuntime;
use opendan::session_model::{SessionStatus, SessionSummary};
use opendan::worklog::{WorklogService, WorklogToolConfig};

const WORKLOG_DB_ENV: &str = "OPENDAN_WORKLOG_DB";
const DEFAULT_WORKLOG_DB: &str = "/opt/buckyos/opendan/worklog.db";
const BUCKYOS_APPID_ENV: [&str; 1] = ["BUCKYOS_APP_ID"];
const PACKAGE_ROOT_ENVS: [&str; 2] = ["BUCKYOS_PKG_DIR", "BUCKYOS_PKG_SOURCE_DIR"];
const ROOTFS_SYNC_MANIFEST: &str = ".meta/rootfs_sync.json";
const ROOTFS_SYNC_VERSION: u32 = 1;

#[derive(Debug, Default)]
struct StartupArgs {
    appid: Option<String>,
    owner_id: Option<String>,
    agent_bin: Option<PathBuf>,
    worksession_test: Option<PathBuf>,
    worksession_task_test: Option<PathBuf>,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct WorkSessionQuickTestSpec {
    task_id: Option<i64>,
    title: String,
    objective: String,
    workspace_id: Option<String>,
    behavior: Option<String>,
    created_by_session_id: String,
    #[serde(alias = "reason_message")]
    reason_messages: Vec<String>,
    auto_start: bool,
}

impl Default for WorkSessionQuickTestSpec {
    fn default() -> Self {
        Self {
            task_id: None,
            title: String::new(),
            objective: String::new(),
            workspace_id: None,
            behavior: None,
            created_by_session_id: "worksession-test".to_string(),
            reason_messages: Vec::new(),
            auto_start: true,
        }
    }
}

impl WorkSessionQuickTestSpec {
    fn into_params(self) -> CreateWorkSessionParams {
        CreateWorkSessionParams {
            title: self.title,
            objective: self.objective,
            workspace_id: self.workspace_id,
            behavior: self.behavior,
            created_by_session_id: self.created_by_session_id,
            reason_messages: self.reason_messages,
            task_binding: None,
            task_id: self.task_id,
            auto_start: self.auto_start,
            bind_task: true,
        }
    }
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
                return Err(anyhow!(
                    "{arg} is no longer supported; opendan resolves agent_root from BuckyOS app data folder"
                ));
            }
            "--agent-bin" => {
                parsed.agent_bin = Some(PathBuf::from(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for {arg}"))?,
                ));
            }
            "--worksession-test" | "--work-session-test" => {
                parsed.worksession_test = Some(PathBuf::from(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for {arg}"))?,
                ));
            }
            "--worksession-task-test"
            | "--work-session-task-test"
            | "worksession-task-test"
            | "work-session-task-test" => {
                parsed.worksession_task_test = Some(PathBuf::from(
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
            other if other.starts_with("--agent-root=") || other.starts_with("--agent-env=") => {
                return Err(anyhow!(
                    "{other} is no longer supported; opendan resolves agent_root from BuckyOS app data folder"
                ));
            }
            other if other.starts_with("--agent-bin=") => {
                parsed.agent_bin = Some(PathBuf::from(&other["--agent-bin=".len()..]));
            }
            other if other.starts_with("--worksession-test=") => {
                parsed.worksession_test =
                    Some(PathBuf::from(&other["--worksession-test=".len()..]));
            }
            other if other.starts_with("--work-session-test=") => {
                parsed.worksession_test =
                    Some(PathBuf::from(&other["--work-session-test=".len()..]));
            }
            other if other.starts_with("--worksession-task-test=") => {
                parsed.worksession_task_test =
                    Some(PathBuf::from(&other["--worksession-task-test=".len()..]));
            }
            other if other.starts_with("--work-session-task-test=") => {
                parsed.worksession_task_test =
                    Some(PathBuf::from(&other["--work-session-task-test=".len()..]));
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
    if let Some((appid, _owner_id)) = load_app_identity_from_env()
        .map_err(|err| anyhow!("load app identity from app_instance_config failed: {err}"))?
    {
        return Ok(appid);
    }
    if let Some(appid) = first_env(&BUCKYOS_APPID_ENV) {
        return Ok(appid);
    }
    if let Some(appid) = startup
        .appid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(appid.to_string());
    }
    Err(anyhow!(
        "appid is required; pass --appid <id>, a positional appid, or set one of {:?}",
        BUCKYOS_APPID_ENV
    ))
}

fn resolve_owner_id(startup: &StartupArgs) -> Result<Option<String>> {
    if let Some((_appid, owner_id)) = load_app_identity_from_env()
        .map_err(|err| anyhow!("load app identity from app_instance_config failed: {err}"))?
    {
        return Ok(Some(owner_id));
    }
    if let Some(owner_id) = startup
        .owner_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(owner_id.to_string()));
    }
    Ok(None)
}

fn resolve_agent_root() -> Result<PathBuf> {
    let runtime =
        get_buckyos_api_runtime().map_err(|err| anyhow!("load buckyos runtime failed: {err}"))?;
    let appid = runtime.get_app_id();
    let owner_id = runtime
        .get_owner_user_id()
        .ok_or_else(|| anyhow!("resolve opendan app data folder failed: owner_user_id missing"))?;
    Ok(get_buckyos_app_data_dir(&appid, &owner_id))
}

fn resolve_agent_package_root(startup: &StartupArgs, appid: &str) -> Option<PathBuf> {
    if let Some(path) = startup.agent_bin.clone() {
        return Some(path);
    }
    if let Some(path) = first_env(&PACKAGE_ROOT_ENVS).map(PathBuf::from) {
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
        "notebook",
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

    // The KEvent client must come from the runtime: opendan runs in a
    // container, so its only global-event channel is node-daemon's bridge.
    // Building one here with no transport used to "succeed" against a private
    // ring buffer nobody else was attached to — every global event silently
    // stayed inside the container. This is a hard failure now: without the
    // bridge, agent event subscriptions do not work at all.
    let kevent_client = Arc::new(
        api_runtime
            .get_kevent_client()
            .await
            .map_err(|err| anyhow!("opendan.bootstrap: kevent client unavailable: {err:?}"))?,
    );
    let kevent_backend = kevent_client.transport_kind();
    let kevent_endpoint = kevent_client
        .transport_status()
        .map(|status| status.endpoint)
        .unwrap_or_else(|| "local".to_string());

    info!(
        "opendan.bootstrap: aicc=ready worklog_db={} msg_center={} task_mgr={} kevent_backend={:?} kevent_endpoint={}",
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
        },
        kevent_backend,
        kevent_endpoint
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

async fn run_worksession_quick_test(agent: Arc<AIAgent>, json_path: PathBuf) -> Result<()> {
    let result = async {
        let raw = fs::read_to_string(&json_path)
            .await
            .with_context(|| format!("read worksession test json {}", json_path.display()))?;
        let spec = serde_json::from_str::<WorkSessionQuickTestSpec>(&raw)
            .with_context(|| format!("parse worksession test json {}", json_path.display()))?;
        let outcome = agent
            .clone()
            .create_work_session(spec.into_params())
            .await
            .context("create worksession for quick test failed")?;
        info!(
            "opendan.worksession_test: created session_id={} workspace_id={} workspace_status={} behavior={}",
            outcome.session_id, outcome.workspace_id, outcome.workspace_status, outcome.behavior
        );
        let summary = wait_worksession_to_finish(agent.clone(), &outcome.session_id).await?;
        info!(
            "opendan.worksession_test: finished session_id={} status={:?}",
            summary.session_id, summary.status
        );
        Ok(())
    }
    .await;

    if let Err(err) = &result {
        error!("opendan.worksession_test: failed: {err:#}");
    }
    agent.shutdown().await;
    result
}

async fn run_worksession_task_quick_test(agent: Arc<AIAgent>, json_path: PathBuf) -> Result<()> {
    let result = async {
        let raw = fs::read_to_string(&json_path)
            .await
            .with_context(|| format!("read worksession task test json {}", json_path.display()))?;
        let spec = serde_json::from_str::<WorkSessionQuickTestSpec>(&raw)
            .with_context(|| format!("parse worksession task test json {}", json_path.display()))?;
        let task_mgr = agent
            .runtime
            .task_mgr
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("task manager unavailable"))?;
        let api_runtime = get_buckyos_api_runtime()
            .map_err(|err| anyhow!("load buckyos runtime failed: {err}"))?;
        let user_id = api_runtime
            .get_owner_user_id()
            .or_else(|| api_runtime.user_id.clone())
            .unwrap_or_else(|| agent.agent_id());
        let app_id = api_runtime.get_app_id();
        let task_name = if spec.title.trim().is_empty() {
            "worksession task test".to_string()
        } else {
            spec.title.trim().to_string()
        };
        let objective = spec.objective.trim().to_string();
        if objective.is_empty() {
            return Err(anyhow!("worksession task test objective must not be empty"));
        }
        let runner = agent.task_executor_runner_id()?;
        let mut workspace_hints = Vec::new();
        if let Some(workspace_id) = spec
            .workspace_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            workspace_hints.push(serde_json::json!({ "workspace_id": workspace_id }));
        }
        let task = task_mgr
            .create_task(
                &task_name,
                TASK_TYPE_AGENT_DELEGATE,
                Some(serde_json::json!({
                    "agent_delegate": {
                        "version": 1,
                        "source": "worksession-task-test",
                        "title": spec.title,
                        "purpose": objective,
                        "requester_agent_id": agent.agent_id(),
                        "input": {
                            "text": objective
                        },
                        "workspace_hints": workspace_hints,
                        "reason_messages": spec.reason_messages,
                        "execution": {
                            "runner": runner.clone(),
                            "behavior": spec.behavior,
                            "status": "assigned"
                        }
                    }
                })),
                &user_id,
                &app_id,
                None,
            )
            .await
            .map_err(|err| anyhow!("create worksession task test task failed: {err}"))?;
        info!(
            "opendan.worksession_task_test: created task_id={} runner={} user_id={} app_id={}",
            task.id, runner, task.user_id, task.app_id
        );
        let final_task = wait_task_to_finish(agent.clone(), task.id).await?;
        info!(
            "opendan.worksession_task_test: finished task_id={} status={:?}",
            final_task.id, final_task.status
        );
        match final_task.status {
            TaskStatus::Completed => Ok(()),
            other => Err(anyhow!(
                "worksession task test task {} ended with status {:?}: {}",
                final_task.id,
                other,
                final_task.message.unwrap_or_default()
            )),
        }
    }
    .await;

    if let Err(err) = &result {
        error!("opendan.worksession_task_test: failed: {err:#}");
    }
    agent.shutdown().await;
    result
}

async fn wait_worksession_to_finish(
    agent: Arc<AIAgent>,
    session_id: &str,
) -> Result<SessionSummary> {
    let mut last_status = None;
    loop {
        let session = agent
            .get_session(session_id)
            .await
            .ok_or_else(|| anyhow!("worksession `{session_id}` disappeared"))?;
        let summary = session.summary().await;
        if last_status != Some(summary.status) {
            info!(
                "opendan.worksession_test: session_id={} status={:?}",
                summary.session_id, summary.status
            );
            last_status = Some(summary.status);
        }
        match summary.status {
            SessionStatus::Ended => return Ok(summary),
            SessionStatus::Error => {
                return Err(anyhow!("worksession `{session_id}` entered Error status"));
            }
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

async fn wait_task_to_finish(agent: Arc<AIAgent>, task_id: i64) -> Result<buckyos_api::Task> {
    let task_mgr = agent
        .runtime
        .task_mgr
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("task manager unavailable"))?;
    let mut last_status = None;
    loop {
        let task = task_mgr
            .get_task(task_id)
            .await
            .map_err(|err| anyhow!("load task {task_id}: {err}"))?;
        if last_status != Some(task.status) {
            info!(
                "opendan.worksession_task_test: task_id={} status={:?} progress={}",
                task.id, task.status, task.progress
            );
            last_status = Some(task.status);
        }
        if task.status.is_terminal() {
            return Ok(task);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
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
    let runtime = bootstrap(&appid, owner_id).await?;
    let agent_root = resolve_agent_root()?;

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
    let worksession_test_handle = startup
        .worksession_test
        .clone()
        .map(|path| tokio::spawn(run_worksession_quick_test(agent.clone(), path)));
    let worksession_task_test_handle = startup
        .worksession_task_test
        .clone()
        .map(|path| tokio::spawn(run_worksession_task_quick_test(agent.clone(), path)));
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
    if let Some(mut handle) = worksession_test_handle {
        match tokio::time::timeout(Duration::from_secs(1), &mut handle).await {
            Ok(joined) => {
                joined.map_err(|err| anyhow!("worksession test task join failed: {err}"))??;
            }
            Err(_) => {
                handle.abort();
                let _ = handle.await;
            }
        }
    }
    if let Some(mut handle) = worksession_task_test_handle {
        match tokio::time::timeout(Duration::from_secs(1), &mut handle).await {
            Ok(joined) => {
                joined.map_err(|err| anyhow!("worksession task test join failed: {err}"))??;
            }
            Err(_) => {
                handle.abort();
                let _ = handle.await;
            }
        }
    }
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
        let parsed = parse_startup_args_from_iter(["buckyos_jarvis"]).expect("parse args");
        assert_eq!(parsed.appid.as_deref(), Some("buckyos_jarvis"));
    }

    #[test]
    fn parses_appid_flag() {
        let parsed =
            parse_startup_args_from_iter(["--appid", "buckyos_jarvis"]).expect("parse args");
        assert_eq!(parsed.appid.as_deref(), Some("buckyos_jarvis"));
    }

    #[test]
    fn keeps_agent_id_as_loader_alias() {
        let parsed =
            parse_startup_args_from_iter(["--agent-id=buckyos_jarvis"]).expect("parse args");
        assert_eq!(parsed.appid.as_deref(), Some("buckyos_jarvis"));
    }

    #[test]
    fn parses_agent_bin_flag() {
        let parsed = parse_startup_args_from_iter([
            "--appid=buckyos_jarvis",
            "--agent-bin",
            "/pkg/buckyos_jarvis",
        ])
        .expect("parse args");
        assert_eq!(
            parsed.agent_bin.unwrap(),
            std::path::PathBuf::from("/pkg/buckyos_jarvis")
        );
    }

    #[test]
    fn parses_worksession_test_flag() {
        let parsed = parse_startup_args_from_iter([
            "--appid=buckyos_jarvis",
            "--worksession-test",
            "/tmp/ws.json",
        ])
        .expect("parse args");
        assert_eq!(
            parsed.worksession_test.unwrap(),
            std::path::PathBuf::from("/tmp/ws.json")
        );
    }

    #[test]
    fn parses_worksession_task_test_flag() {
        let parsed = parse_startup_args_from_iter([
            "--appid=buckyos_jarvis",
            "--worksession-task-test",
            "/tmp/ws-task.json",
        ])
        .expect("parse args");
        assert_eq!(
            parsed.worksession_task_test.unwrap(),
            std::path::PathBuf::from("/tmp/ws-task.json")
        );
    }

    #[test]
    fn parses_worksession_task_test_separator_alias() {
        let parsed = parse_startup_args_from_iter([
            "--appid=buckyos_jarvis",
            "worksession-task-test",
            "/tmp/ws-task.json",
        ])
        .expect("parse args");
        assert_eq!(
            parsed.worksession_task_test.unwrap(),
            std::path::PathBuf::from("/tmp/ws-task.json")
        );
    }

    #[test]
    fn parses_worksession_test_json() {
        let spec: super::WorkSessionQuickTestSpec = serde_json::from_value(serde_json::json!({
            "title": "quick check",
            "objective": "run one complete task",
            "workspace_id": "ws-existing",
            "behavior": "work_default",
            "created_by_session_id": "ui-source",
            "reason_message": ["requested by cli"]
        }))
        .expect("parse spec");

        assert_eq!(spec.title, "quick check");
        assert_eq!(spec.objective, "run one complete task");
        assert_eq!(spec.workspace_id.as_deref(), Some("ws-existing"));
        assert_eq!(spec.behavior.as_deref(), Some("work_default"));
        assert_eq!(spec.created_by_session_id, "ui-source");
        assert_eq!(spec.reason_messages, vec!["requested by cli"]);
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
