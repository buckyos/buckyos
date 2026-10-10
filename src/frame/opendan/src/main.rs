//! `opendan` — the Agent Loader process.
//!
//! In a zone it starts as the runtime app of one agent (`opendan <appid>`):
//! identity and services come from the BuckyOS runtime, the agent from the
//! AgentSpec bound to this app instance. `--dev` runs the same Loader
//! outside a zone on a development file queue (no msg-center).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_tool::xllm::XllmDeps;
use anyhow::{anyhow, bail, Context, Result};
use buckyos_api::{
    agent_spec_key, get_buckyos_api_runtime, init_buckyos_api_runtime, load_app_identity_from_env,
    set_buckyos_api_runtime, user_app_spec_key, AgentId, AgentSpec, AppInstanceId,
    AppServiceInstanceConfig, AppServiceSpec, BuckyOSRuntimeType, OPENDAN_SERVICE_PORT,
};
use buckyos_kit::{get_buckyos_app_data_dir, init_logging};
use libopendan::channel::{KEventWaker, KmsgChannels, PollWaker};
use libopendan::protocol::parse_did;
use libopendan::runner::RunnerOptions;
use opendan::loader::{Loader, LoaderEnv, SpecRef};
use opendan::records::{self, AgentRecords, LocalRecords, ZoneRecords};
use opendan::service::Access;
use opendan::tasks::ZoneTaskService;
use opendan::ui::ZoneMailService;

const USAGE: &str = "\
opendan — Agent Loader

  opendan [<appid>] [--app-id <id>] [--owner-id <user>] [--agent-bin <package dir>] [--service-port <n>] [--web <dir>]
          [--trust-loopback]   local debugging: a loopback caller without a token is the owner
  opendan --dev --agent-root <dir> --agent-did <did> --queue-dir <dir>
          [--owner-did <did>] [--records <dir>] [--who <principal>] [--agent-bin <package dir>] [--port <n>]
          [--web <dir>] [--poll-ms <n>]
          --records: settings.json / profile.json / info.json of the agent (default: defaults, in memory)

environment: BUCKYOS_APP_ID, BUCKYOS_SERVICE_PORT | OPENDAN_SERVICE_PORT, BUCKYOS_PKG_DIR | BUCKYOS_PKG_SOURCE_DIR,
             app_instance_config (the version of the agent package)
";

/// Port assigned by the scheduler, as node-daemon and `service_debug` pass it.
const SERVICE_PORT_ENVS: [&str; 2] = ["BUCKYOS_SERVICE_PORT", "OPENDAN_SERVICE_PORT"];
const PACKAGE_ROOT_ENVS: [&str; 2] = ["BUCKYOS_PKG_DIR", "BUCKYOS_PKG_SOURCE_DIR"];
const AGENT_BINDING_POLL: Duration = Duration::from_secs(5);
const AGENT_BINDING_LOG_EVERY: u32 = 12;

#[derive(Debug, Default)]
struct Args {
    dev: bool,
    appid: Option<String>,
    owner_id: Option<String>,
    agent_bin: Option<PathBuf>,
    agent_root: Option<PathBuf>,
    agent_did: Option<String>,
    owner_did: Option<String>,
    records: Option<PathBuf>,
    queue_dir: Option<PathBuf>,
    who: Option<String>,
    port: Option<u16>,
    web: Option<PathBuf>,
    poll_ms: Option<u64>,
    trust_loopback: bool,
}

fn parse_args(argv: impl IntoIterator<Item = String>) -> Result<Args> {
    let mut a = Args::default();
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |what: &str| -> Result<String> {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or_else(|| anyhow!("missing value for {what}"))
        };
        match name.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--dev" => a.dev = true,
            "--trust-loopback" => a.trust_loopback = true,
            "--appid" | "--app-id" => a.appid = Some(value(&name)?),
            "--owner-id" | "--owner-user-id" => a.owner_id = Some(value(&name)?),
            "--agent-bin" => a.agent_bin = Some(value(&name)?.into()),
            "--agent-root" => a.agent_root = Some(value(&name)?.into()),
            "--agent-did" => a.agent_did = Some(value(&name)?),
            "--owner-did" => a.owner_did = Some(value(&name)?),
            "--records" => a.records = Some(value(&name)?.into()),
            "--queue-dir" => a.queue_dir = Some(value(&name)?.into()),
            "--who" => a.who = Some(value(&name)?),
            "--web" => a.web = Some(value(&name)?.into()),
            "--port" | "--service-port" => a.port = Some(value(&name)?.parse().context("--port")?),
            "--poll-ms" => a.poll_ms = Some(value(&name)?.parse().context("--poll-ms")?),
            other if other.starts_with('-') => bail!("unknown option {other}\n{USAGE}"),
            _ if a.appid.is_none() => a.appid = Some(arg),
            other => bail!("unexpected argument {other}\n{USAGE}"),
        }
    }
    Ok(a)
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

/// Next to the executable: the `xagent` command and the built WebUI.
fn beside_exe(name: &str) -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let path = dir.join(name);
    path.exists().then_some(path)
}

fn port_of(args: &Args) -> u16 {
    args.port
        .or_else(|| {
            first_env(&SERVICE_PORT_ENVS)
                .and_then(|p| p.parse().ok())
                .filter(|p| *p != 0)
        })
        .unwrap_or(OPENDAN_SERVICE_PORT)
}

fn options_of(args: &Args) -> RunnerOptions {
    let mut options = RunnerOptions::default();
    if let Some(ms) = args.poll_ms {
        options.poll_interval = Duration::from_millis(ms.max(10));
    }
    options
}

/// Version of the agent package node-daemon deployed for this instance
/// (`all.agent.<agent_id>#<version>`).
fn package_version() -> Option<String> {
    let raw = std::env::var("app_instance_config").ok()?;
    let config: AppServiceInstanceConfig = serde_json::from_str(&raw).ok()?;
    let pkg_id = &config.node_execution_spec.packages.get("agent")?.pkg_id;
    pkg_id
        .split_once('#')
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty() && !v.starts_with("pkg:"))
}

/// The deployed package is pinned by ObjectId; the version it was built at is
/// the version of the instance's AppDoc (the agent's App is built from its
/// template at the template's version).
async fn installed_app_version(instance: &AppInstanceId) -> Option<String> {
    let client = get_buckyos_api_runtime().ok()?.get_system_config_client().await.ok()?;
    let key = user_app_spec_key(instance.owner_user_id(), instance.app_id());
    match client.get(&key).await {
        Ok(value) => serde_json::from_str::<AppServiceSpec>(&value.value)
            .ok()
            .map(|spec| spec.app_doc.version),
        Err(e) => {
            log::warn!("opendan: AppSpec {key} not readable: {e}");
            None
        }
    }
}

/// AgentSpecs of the owner bound to this app instance. A spec of another
/// agent that cannot be read is that agent's problem, not this one's.
async fn bound_agent_specs(instance: &AppInstanceId) -> Result<Vec<AgentSpec>> {
    let client = get_buckyos_api_runtime()?
        .get_system_config_client()
        .await
        .context("system-config client")?;
    let owner = instance.owner_user_id();
    let root = format!("users/{owner}/agents");
    let mut specs = Vec::new();
    for raw in client.list(&root).await.with_context(|| format!("list {root}"))? {
        let checked = async {
            let agent_id = AgentId::parse(&raw).map_err(|e| anyhow!("{root}/{raw}: {e}"))?;
            let key = agent_spec_key(owner, &agent_id);
            let value = match client.get(&key).await {
                Ok(v) => v,
                // Being created: no spec yet.
                Err(buckyos_api::SystemConfigError::KeyNotFound(_)) => return Ok(None),
                Err(e) => return Err(anyhow!("get {key}: {e}")),
            };
            let spec: AgentSpec =
                serde_json::from_str(&value.value).with_context(|| format!("decode {key}"))?;
            spec.validate().map_err(|e| anyhow!("{key}: {e}"))?;
            if spec.agent_id != agent_id {
                bail!("{key}: AgentSpec names {}", spec.agent_id);
            }
            Ok(Some(spec))
        };
        match checked.await {
            Ok(Some(spec)) if spec.binding.references_runtime(instance) => specs.push(spec),
            Ok(_) => {}
            Err(e) => log::warn!("opendan: {e:#}"),
        }
    }
    Ok(specs)
}

/// The agent bound to this app instance. One process hosts one agent; the
/// app may be installed before its agent is bound to it, so without one
/// the process waits (it does not exit to be restarted).
async fn wait_for_bound_agent(instance: &AppInstanceId) -> Result<AgentSpec> {
    let mut rounds = 0u32;
    loop {
        match bound_agent_specs(instance).await {
            Ok(mut specs) if specs.len() == 1 => return Ok(specs.remove(0)),
            Ok(specs) if specs.len() > 1 => bail!(
                "{instance} has {} agents bound; one process hosts exactly one",
                specs.len()
            ),
            Ok(_) if rounds % AGENT_BINDING_LOG_EVERY == 0 => {
                log::info!("opendan: no AgentSpec is bound to {instance} yet; waiting")
            }
            Err(e) if rounds % AGENT_BINDING_LOG_EVERY == 0 => {
                log::warn!("opendan: AgentSpecs of {instance} not readable yet: {e:#}")
            }
            _ => {}
        }
        rounds = rounds.wrapping_add(1);
        tokio::time::sleep(AGENT_BINDING_POLL).await;
    }
}

async fn zone_env(args: &Args) -> Result<LoaderEnv> {
    let identity = load_app_identity_from_env()
        .map_err(|e| anyhow!("app identity from the environment: {e}"))?;
    let appid = identity
        .as_ref()
        .map(|(app, _)| app.clone())
        .or_else(|| first_env(&["BUCKYOS_APP_ID"]))
        .or_else(|| args.appid.clone())
        .ok_or_else(|| anyhow!("appid is required (positional, --appid or $BUCKYOS_APP_ID)"))?;
    let owner = identity.map(|(_, owner)| owner).or_else(|| args.owner_id.clone());
    let mut runtime = init_buckyos_api_runtime(&appid, owner, BuckyOSRuntimeType::AppService)
        .await
        .map_err(|e| anyhow!("init BuckyOS runtime: {e}"))?;
    runtime.login().await.map_err(|e| anyhow!("login: {e}"))?;
    set_buckyos_api_runtime(runtime).map_err(|e| anyhow!("register BuckyOS runtime: {e}"))?;
    let runtime = get_buckyos_api_runtime()?;
    let instance = runtime
        .get_app_instance_id()
        .map_err(|e| anyhow!("app instance id: {e}"))?;
    let owner = runtime
        .get_owner_user_id()
        .ok_or_else(|| anyhow!("the runtime has no owner user id"))?;
    let spec = wait_for_bound_agent(&instance).await?;
    let records: Arc<dyn AgentRecords> = Arc::new(ZoneRecords::new(&owner, &spec.agent_id));
    let settings = records::settings(records.as_ref())
        .await
        .map_err(|e| anyhow!("settings of {}: {e}", spec.agent_id))?;
    match records::profile(records.as_ref()).await {
        Ok(profile) => log::info!(
            "opendan: agent {} ({}) of {} is bound to {instance}",
            spec.agent_id,
            profile.resolved_display_name(&spec.agent_id),
            spec.agent_doc.owner.to_string()
        ),
        Err(e) => log::warn!("opendan: profile of {} not readable: {e}", spec.agent_id),
    }
    let agent_root = get_buckyos_app_data_dir(&runtime.get_app_id(), &owner)
        .join("agents")
        .join(spec.agent_id.as_str());
    let package_root = args
        .agent_bin
        .clone()
        .or_else(|| first_env(&PACKAGE_ROOT_ENVS).map(PathBuf::from))
        .or_else(|| {
            [appid.clone(), format!("buckyos_{appid}")]
                .into_iter()
                .map(|n| PathBuf::from("/opt/buckyos/bin").join(n))
                .find(|p| p.is_dir())
        });
    let queue = runtime
        .get_msg_queue_client()
        .await
        .map_err(|e| anyhow!("kmsg client: {e}"))?;
    // Without the kevent bridge every notification is lost; polling still
    // carries the work, so this is reported, not fatal.
    let kevent = match runtime.get_kevent_client().await {
        Ok(k) => Some(Arc::new(k)),
        Err(e) => {
            log::warn!("opendan: kevent client unavailable ({e:?}); polling only");
            None
        }
    };
    Ok(LoaderEnv {
        who: format!("app:{}@{owner}", runtime.get_app_id()),
        agent_did: spec.agent_did.to_string(),
        agent_id: Some(spec.agent_id.as_str().to_string()),
        owner_did: Some(spec.agent_doc.owner.clone()),
        spec: Some(SpecRef {
            agent_doc_object_id: spec.agent_doc_object_id.clone(),
            generation: spec.generation,
        }),
        settings,
        records,
        agent_root,
        package_root,
        package_version: match package_version() {
            Some(version) => Some(version),
            None => installed_app_version(&instance).await,
        },
        channels: Arc::new(KmsgChannels::new(Arc::new(queue))),
        waker: match &kevent {
            Some(k) => Arc::new(KEventWaker::new(k.clone())),
            None => Arc::new(PollWaker),
        },
        kevent,
        mail: Some(Arc::new(ZoneMailService)),
        tasks: Some(Arc::new(ZoneTaskService)),
        access: Access::Zone {
            owner,
            app_id: runtime.get_app_id(),
            trust_loopback: args.trust_loopback,
            zone_host: Some(runtime.zone_id.to_host_name()).filter(|h| !h.is_empty()),
        },
        port: port_of(args),
        web_dir: args.web.clone().or_else(|| beside_exe("web")),
        xllm: XllmDeps::default(),
        session_cli: beside_exe("xagent"),
        options: options_of(args),
    })
}

async fn dev_env(args: &Args) -> Result<LoaderEnv> {
    let need = |v: &Option<PathBuf>, name: &str| {
        v.clone().ok_or_else(|| anyhow!("--dev needs {name}\n{USAGE}"))
    };
    let queue_dir = need(&args.queue_dir, "--queue-dir")?;
    let agent_did = args
        .agent_did
        .clone()
        .ok_or_else(|| anyhow!("--dev needs --agent-did\n{USAGE}"))?;
    let agent_root = need(&args.agent_root, "--agent-root")?;
    let owner_did = args
        .owner_did
        .as_deref()
        .map(|d| parse_did(d).map_err(|e| anyhow!("--owner-did: {e}")))
        .transpose()?;
    let records: Arc<dyn AgentRecords> = match &args.records {
        Some(dir) => Arc::new(LocalRecords::dir(dir)),
        None => Arc::new(LocalRecords::memory()),
    };
    let settings = records::settings(records.as_ref())
        .await
        .map_err(|e| anyhow!("--records: {e}"))?;
    // Commands of a session's shell use the same development queue.
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &queue_dir);
    let who = args.who.clone().unwrap_or_else(|| "app:opendan@local".to_string());
    Ok(LoaderEnv {
        who: who.clone(),
        agent_did,
        agent_id: None,
        owner_did,
        spec: None,
        settings,
        records,
        agent_root,
        package_root: args.agent_bin.clone(),
        package_version: None,
        channels: Arc::new(
            KmsgChannels::dir(&queue_dir).map_err(|e| anyhow!("queue dir: {e}"))?,
        ),
        waker: Arc::new(PollWaker),
        kevent: None,
        mail: None,
        tasks: None,
        access: Access::Open { who },
        port: port_of(args),
        web_dir: args.web.clone().or_else(|| beside_exe("web")),
        xllm: XllmDeps::default(),
        session_cli: beside_exe("xagent"),
        options: options_of(args),
    })
}

async fn run() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    let env = if args.dev {
        dev_env(&args).await?
    } else {
        zone_env(&args).await?
    };
    let loader = Loader::start(env).await?;
    wait_exit_signal().await?;
    // Leaving ends the hosting only; no session is stopped.
    log::info!("opendan: leaving, waiting for the serving loops");
    loader.shutdown().await;
    Ok(())
}

#[cfg(unix)]
async fn wait_exit_signal() -> Result<()> {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("SIGTERM handler")?;
    tokio::select! {
        r = tokio::signal::ctrl_c() => r.context("SIGINT handler")?,
        _ = term.recv() => {}
    }
    Ok(())
}

#[cfg(windows)]
async fn wait_exit_signal() -> Result<()> {
    use tokio::signal::windows;
    let mut brk = windows::ctrl_break().context("CTRL_BREAK handler")?;
    let mut close = windows::ctrl_close().context("CTRL_CLOSE handler")?;
    let mut shutdown = windows::ctrl_shutdown().context("CTRL_SHUTDOWN handler")?;
    tokio::select! {
        r = tokio::signal::ctrl_c() => r.context("CTRL_C handler")?,
        _ = brk.recv() => {}
        _ = close.recv() => {}
        _ = shutdown.recv() => {}
    }
    Ok(())
}

/// An app container is recreated on every start and gets a new hostname;
/// the sessions stay bound to the app instance on its node.
fn pin_host_identity() {
    use agent_tool::runtime::HOST_ID_ENV;
    if std::env::var_os(HOST_ID_ENV).is_some() {
        return;
    }
    let Some(instance) = first_env(&["BUCKYOS_APP_INSTANCE_ID"]) else {
        return;
    };
    let device = first_env(&["BUCKYOS_THIS_DEVICE"])
        .and_then(|doc| serde_json::from_str::<serde_json::Value>(&doc).ok())
        .and_then(|doc| doc.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .unwrap_or_default();
    std::env::set_var(HOST_ID_ENV, format!("{device}/{instance}"));
}

fn main() {
    init_logging("opendan", true);
    pin_host_identity();
    let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
    if let Err(err) = rt.block_on(run()) {
        log::error!("opendan: {err:#}");
        eprintln!("opendan: {err:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn zone_and_dev_arguments() {
        let a = parse(&["xiaobai.test.buckyos.io", "--agent-bin", "/pkg", "--port=14060"]).unwrap();
        assert_eq!(a.appid.as_deref(), Some("xiaobai.test.buckyos.io"));
        assert_eq!(a.agent_bin, Some(PathBuf::from("/pkg")));
        assert_eq!(a.port, Some(14060));
        let a = parse(&["--dev", "--agent-root", "/r", "--agent-did", "did:bns:j", "--queue-dir", "/q", "--owner-did", "did:bns:bob"]).unwrap();
        assert!(a.dev && a.agent_root == Some(PathBuf::from("/r")));
        assert_eq!(a.owner_did.as_deref(), Some("did:bns:bob"));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        assert!(rt.block_on(dev_env(&parse(&["--dev"]).unwrap())).is_err());
        assert!(parse(&["--agent-id", "x"]).is_err());
        // The command line node-daemon and service_debug use.
        let a = parse(&["--app-id", "jarvis", "--agent-bin", "/pkg", "--service-port", "10016"]).unwrap();
        assert_eq!((a.appid.as_deref(), a.port), (Some("jarvis"), Some(10016)));
    }
}
