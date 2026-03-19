#[allow(non_snake_case)]
pub mod agent;
pub mod agent_bash;
pub mod agent_config;
pub mod agent_environment;
pub mod agent_memory;
pub mod agent_session;
pub mod agent_tool;
pub mod ai_runtime;
pub mod behavior;
pub mod buildin_tool;
#[cfg(test)]
pub mod test_utils;
pub mod worklog;
pub mod workspace;
pub mod workspace_path;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use buckyos_api::msg_queue::MsgQueueClient;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, load_app_identity_from_env,
    set_buckyos_api_runtime, AppDoc, AppServiceInstanceConfig, AppServiceSpec, BuckyOSRuntimeType,
    ServiceInstallConfig, AICC_SERVICE_SERVICE_NAME, OPENDAN_SERVICE_NAME,
    OPENDAN_SERVICE_PORT as DEFAULT_OPENDAN_SERVICE_PORT,
};
use buckyos_kit::{get_buckyos_root_dir, init_logging};
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use name_lib::{AgentDocument, DIDDocumentTrait, EncodedDocument};
use serde_json::Value as Json;
use server_runner::Runner;
use tokio::fs;
#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{sleep, Duration};

use crate::agent::{AIAgent, AIAgentDeps};
use crate::agent_config::AIAgentConfig;
use crate::ai_runtime::{AiRuntime, AiRuntimeConfig, OpenDanRuntimeKrpcHandler};

struct OpenDanHttpServer {
    rpc_handler: buckyos_api::OpenDanServerHandler<OpenDanRuntimeKrpcHandler>,
}

impl OpenDanHttpServer {
    fn new(runtime: Arc<AiRuntime>) -> Self {
        Self {
            rpc_handler: OpenDanRuntimeKrpcHandler::new(runtime).into_server_handler(),
        }
    }
}

#[async_trait::async_trait]
impl kRPC::RPCHandler for OpenDanHttpServer {
    async fn handle_rpc_call(
        &self,
        req: kRPC::RPCRequest,
        ip_from: std::net::IpAddr,
    ) -> std::result::Result<kRPC::RPCResponse, kRPC::RPCErrors> {
        self.rpc_handler.handle_rpc_call(req, ip_from).await
    }
}

#[async_trait::async_trait]
impl HttpServer for OpenDanHttpServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        OPENDAN_SERVICE_NAME.to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

const OPENDAN_AGENT_ID_ENV: [&str; 3] = ["OPENDAN_AGENT_ID", "AGENT_ID", "AGENT_INSTANCE_ID"];
const OPENDAN_AGENT_ENV_ENV: [&str; 3] = ["OPENDAN_AGENT_ENV", "AGENT_ENV", "AGENT_ROOT"];
const OPENDAN_AGENT_BIN_ENV: [&str; 3] = ["OPENDAN_AGENT_BIN", "AGENT_BIN", "AGENT_PACKAGE_ROOT"];
const OPENDAN_AGENT_OWNER_ENV: [&str; 4] = [
    "OPENDAN_AGENT_OWNER",
    "AGENT_OWNER",
    "OWNER_USER_ID",
    "APP_OWNER_ID",
];
const OPENDAN_SERVICE_PORT_ENV: [&str; 3] = ["OPENDAN_SERVICE_PORT", "SERVICE_PORT", "LISTEN_PORT"];
const OPENDAN_SESSION_WORKER_THREADS_ENV: &str = "OPENDAN_SESSION_WORKER_THREADS";
const OPENDAN_STARTUP_DEP_READY_WAIT_SECS: u64 = 10;

#[derive(Clone, Debug, Default)]
struct StartupArgs {
    agent_id: Option<String>,
    agent_env: Option<PathBuf>,
    agent_bin: Option<PathBuf>,
    service_port: Option<u16>,
}

#[derive(Clone, Debug)]
struct AgentInstanceDoc {
    doc: AgentDocument,
    json: Json,
}

#[derive(Clone)]
struct AgentAppSpec {
    key: String,
    json: Json,
    app_doc: AppDoc,
    install_config: ServiceInstallConfig,
    user_id: Option<String>,
}

#[derive(Clone)]
struct LaunchConfig {
    agent_id: String,
    agent_env_root: PathBuf,
    agent_package_root: Option<PathBuf>,
    agent_did: Option<String>,
    agent_owner_did: Option<String>,
    agent_doc: Option<AgentInstanceDoc>,
    agent_spec: Option<AgentAppSpec>,
}

fn parse_u16_arg(value: &str, arg_name: &str) -> Result<u16> {
    value
        .parse::<u16>()
        .map_err(|err| anyhow!("invalid value for {}: {} ({})", arg_name, value, err))
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
            "--agent-id" => {
                parsed.agent_id = Some(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for --agent-id"))?,
                );
            }
            "--agent-env" => {
                parsed.agent_env = Some(PathBuf::from(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for --agent-env"))?,
                ));
            }
            "--agent-bin" => {
                parsed.agent_bin = Some(PathBuf::from(
                    args.next()
                        .map(|value| value.as_ref().to_string())
                        .ok_or_else(|| anyhow!("missing value for --agent-bin"))?,
                ));
            }
            "--service-port" => {
                let value = args
                    .next()
                    .map(|value| value.as_ref().to_string())
                    .ok_or_else(|| anyhow!("missing value for --service-port"))?;
                parsed.service_port = Some(parse_u16_arg(&value, "--service-port")?);
            }
            other if other.starts_with("--agent-id=") => {
                parsed.agent_id = Some(other["--agent-id=".len()..].to_string());
            }
            other if other.starts_with("--agent-env=") => {
                parsed.agent_env = Some(PathBuf::from(&other["--agent-env=".len()..]));
            }
            other if other.starts_with("--agent-bin=") => {
                parsed.agent_bin = Some(PathBuf::from(&other["--agent-bin=".len()..]));
            }
            other if other.starts_with("--service-port=") => {
                parsed.service_port = Some(parse_u16_arg(
                    &other["--service-port=".len()..],
                    "--service-port",
                )?);
            }
            _ => {}
        }
    }
    Ok(parsed)
}

fn parse_startup_args() -> Result<StartupArgs> {
    parse_startup_args_from_iter(std::env::args().skip(1))
}

fn resolve_owner_from_agent_env(agent_env: Option<&PathBuf>) -> Option<String> {
    let agent_env = agent_env?;
    let components = agent_env
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    for (index, component) in components.iter().enumerate() {
        if component != "home" {
            continue;
        }
        if index + 3 >= components.len() {
            continue;
        }
        if components[index + 2] == ".local" && components[index + 3] == "share" {
            let owner = components[index + 1].trim();
            if !owner.is_empty() {
                return Some(owner.to_string());
            }
        }
    }

    None
}

fn get_first_env_var(keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn resolve_requested_service_port(startup: &StartupArgs) -> Result<u16> {
    if let Some(service_port) = startup.service_port {
        return Ok(service_port);
    }

    if let Some(value) = get_first_env_var(&OPENDAN_SERVICE_PORT_ENV) {
        return parse_u16_arg(&value, "OPENDAN_SERVICE_PORT");
    }

    Ok(DEFAULT_OPENDAN_SERVICE_PORT)
}

fn resolve_requested_agent_id(startup: &StartupArgs) -> Result<String> {
    if let Some(agent_id) = startup
        .agent_id
        .clone()
        .filter(|value| !value.trim().is_empty())
    {
        info!("resolved opendan agent_id from cli: {}", agent_id);
        return Ok(agent_id);
    }

    if let Some(agent_id) = get_first_env_var(&OPENDAN_AGENT_ID_ENV) {
        info!("resolved opendan agent_id from env: {}", agent_id);
        return Ok(agent_id);
    }

    if let Some((app_id, _owner_id)) = load_app_identity_from_env()
        .map_err(|err| anyhow!("load app identity from app_instance_config failed: {}", err))?
    {
        info!(
            "resolved opendan agent_id from app_instance_config: {}",
            app_id
        );
        return Ok(app_id);
    }

    warn!("failed to resolve opendan agent_id from cli/env/app_instance_config");
    Err(anyhow!(
        "agent instance id is required; pass --agent-id, set one of {:?}, or provide app_instance_config",
        OPENDAN_AGENT_ID_ENV
    ))
}

fn resolve_requested_owner_id(startup: &StartupArgs) -> Result<String> {
    if let Some(owner_id) = get_first_env_var(&OPENDAN_AGENT_OWNER_ENV) {
        info!("resolved opendan owner_id from env: {}", owner_id);
        return Ok(owner_id);
    }

    if let Some(owner_id) = resolve_owner_from_agent_env(startup.agent_env.as_ref())
        .filter(|value| !value.trim().is_empty())
    {
        info!(
            "resolved opendan owner_id from agent_env path {} => {}",
            startup
                .agent_env
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            owner_id
        );
        return Ok(owner_id);
    }

    if let Some((_app_id, owner_id)) = load_app_identity_from_env()
        .map_err(|err| anyhow!("load app identity from app_instance_config failed: {}", err))?
    {
        info!(
            "resolved opendan owner_id from app_instance_config: {}",
            owner_id
        );
        return Ok(owner_id);
    }

    warn!("failed to resolve opendan owner_id from env/agent_env/app_instance_config");
    Err(anyhow!(
        "agent owner id is required; set one of {:?}, use an --agent-env under data/home/<owner>/.local/share/<appid>, or provide app_instance_config",
        OPENDAN_AGENT_OWNER_ENV
    ))
}

async fn load_agent_instance_doc(agent_id: &str) -> Result<Option<AgentInstanceDoc>> {
    let runtime = get_buckyos_api_runtime().context("load runtime failed before sys_config")?;
    let client = runtime
        .get_system_config_client()
        .await
        .context("init system_config client for opendan failed")?;
    let key = format!("agents/{agent_id}/doc");
    let value = match client.get(&key).await {
        Ok(value) => value.value,
        Err(err) => {
            warn!("load agent instance doc failed: key={} err={}", key, err);
            return Ok(None);
        }
    };
    let encoded = EncodedDocument::from_str(value.clone())
        .map_err(|err| anyhow!("decode agent instance doc failed: key={} err={}", key, err))?;
    let doc = AgentDocument::decode(&encoded, None).map_err(|err| {
        anyhow!(
            "decode agent instance AgentDocument failed: key={} err={}",
            key,
            err
        )
    })?;
    let json = encoded.to_json_value().map_err(|err| {
        anyhow!(
            "convert agent instance doc to json failed: key={} err={}",
            key,
            err
        )
    })?;
    Ok(Some(AgentInstanceDoc { doc, json }))
}

fn owner_key_candidates(owner: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let trimmed = owner.trim();
    if trimmed.is_empty() {
        return candidates;
    }

    candidates.push(trimmed.to_string());

    if let Some(value) = trimmed.rsplit(':').next() {
        let value = value.trim();
        if !value.is_empty() && !candidates.iter().any(|item| item == value) {
            candidates.push(value.to_string());
        }
    }

    candidates
}

fn agent_spec_key_candidates(agent_id: &str, owner: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for owner_key in owner_key_candidates(owner) {
        keys.push(format!("users/{owner_key}/agents/{agent_id}/spec"));
    }
    keys
}

fn parse_agent_app_spec(key: &str, raw: &str) -> Result<AgentAppSpec> {
    let json: Json = serde_json::from_str(raw)
        .map_err(|err| anyhow!("parse agent spec json failed: key={} err={}", key, err))?;

    if let Ok(spec) = serde_json::from_str::<AppServiceInstanceConfig>(raw) {
        return Ok(AgentAppSpec {
            key: key.to_string(),
            json,
            app_doc: spec.app_spec.app_doc,
            install_config: spec.app_spec.install_config,
            user_id: Some(spec.app_spec.user_id),
        });
    }

    if let Ok(spec) = serde_json::from_str::<AppServiceSpec>(raw) {
        return Ok(AgentAppSpec {
            key: key.to_string(),
            json,
            app_doc: spec.app_doc,
            install_config: spec.install_config,
            user_id: Some(spec.user_id),
        });
    }

    if let Ok(app_doc) = serde_json::from_str::<AppDoc>(raw) {
        return Ok(AgentAppSpec {
            key: key.to_string(),
            json,
            app_doc,
            install_config: ServiceInstallConfig::default(),
            user_id: None,
        });
    }

    Err(anyhow!(
        "unsupported agent spec payload: key={} expected AppServiceInstanceConfig/AppServiceSpec/AppDoc",
        key
    ))
}

async fn load_agent_app_spec(agent_id: &str, owner: Option<&str>) -> Result<Option<AgentAppSpec>> {
    let Some(owner) = owner.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let runtime = get_buckyos_api_runtime().context("load runtime failed before agent spec")?;
    let client = runtime
        .get_system_config_client()
        .await
        .context("init system_config client for opendan agent spec failed")?;

    let mut last_err: Option<anyhow::Error> = None;
    for key in agent_spec_key_candidates(agent_id, owner) {
        match client.get(&key).await {
            Ok(value) => {
                return parse_agent_app_spec(&key, &value.value).map(Some);
            }
            Err(err) => {
                last_err = Some(anyhow!("key={} err={}", key, err));
            }
        }
    }

    if let Some(err) = last_err {
        warn!(
            "load agent spec skipped: agent_id={} owner={} detail={}",
            agent_id, owner, err
        );
    }

    Ok(None)
}

fn json_string_pointer<'a>(json: &'a Json, pointers: &[&str]) -> Option<&'a str> {
    pointers.iter().find_map(|pointer| {
        json.pointer(pointer)
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn resolve_optional_path(
    cli_value: Option<&PathBuf>,
    env_keys: &[&str],
    spec: Option<&AgentAppSpec>,
    spec_pointers: &[&str],
    doc: Option<&AgentInstanceDoc>,
    doc_pointers: &[&str],
) -> Option<PathBuf> {
    cli_value
        .cloned()
        .or_else(|| get_first_env_var(env_keys).map(PathBuf::from))
        .or_else(|| {
            spec.and_then(|spec| json_string_pointer(&spec.json, spec_pointers).map(PathBuf::from))
        })
        .or_else(|| {
            doc.and_then(|doc| json_string_pointer(&doc.json, doc_pointers).map(PathBuf::from))
        })
}

fn resolve_agent_env_root(
    startup: &StartupArgs,
    spec: Option<&AgentAppSpec>,
    doc: Option<&AgentInstanceDoc>,
    agent_id: &str,
) -> PathBuf {
    let direct = resolve_optional_path(
        startup.agent_env.as_ref(),
        &OPENDAN_AGENT_ENV_ENV,
        spec,
        &[
            "/agent_env",
            "/agent_env_root",
            "/runtime/agent_env",
            "/app_spec/install_config/custom_config/agent_env",
            "/app_spec/install_config/custom_config/agent_env_root",
            "/app_spec/app_doc/install_config_tips/custom_config/agent_env",
            "/app_spec/app_doc/install_config_tips/custom_config/agent_env_root",
            "/install_config/custom_config/agent_env",
            "/install_config/custom_config/agent_env_root",
            "/app_doc/install_config_tips/custom_config/agent_env",
            "/app_doc/install_config_tips/custom_config/agent_env_root",
        ],
        doc,
        &[
            "/agent_env",
            "/agent_env_root",
            "/paths/agent_env",
            "/runtime/agent_env",
        ],
    );
    if direct.is_some() {
        return direct.unwrap();
    }

    if let Some(path) = spec.and_then(resolve_agent_env_root_from_spec_mounts) {
        return path;
    }

    get_buckyos_root_dir().join("agents").join(agent_id)
}

fn resolve_agent_package_root(
    startup: &StartupArgs,
    spec: Option<&AgentAppSpec>,
    doc: Option<&AgentInstanceDoc>,
) -> Option<PathBuf> {
    let direct = resolve_optional_path(
        startup.agent_bin.as_ref(),
        &OPENDAN_AGENT_BIN_ENV,
        spec,
        &[
            "/agent_bin",
            "/agent_package_root",
            "/package_root",
            "/runtime/agent_bin",
            "/runtime/package_root",
            "/app_spec/install_config/custom_config/agent_bin",
            "/app_spec/install_config/custom_config/agent_package_root",
            "/app_spec/app_doc/install_config_tips/custom_config/agent_bin",
            "/app_spec/app_doc/install_config_tips/custom_config/agent_package_root",
            "/install_config/custom_config/agent_bin",
            "/install_config/custom_config/agent_package_root",
            "/app_doc/install_config_tips/custom_config/agent_bin",
            "/app_doc/install_config_tips/custom_config/agent_package_root",
        ],
        doc,
        &[
            "/agent_bin",
            "/agent_package_root",
            "/package_root",
            "/package/root",
            "/paths/agent_bin",
            "/paths/package",
            "/runtime/agent_bin",
            "/runtime/package_root",
        ],
    );
    if direct.is_some() {
        return direct;
    }

    if let Some(pkg_name) = spec.and_then(resolve_agent_pkg_name_from_spec) {
        return Some(resolve_package_root_candidates(&pkg_name));
    }

    let pkg_name = doc.and_then(|doc| {
        json_string_pointer(
            &doc.json,
            &[
                "/pkg_name",
                "/package/pkg_name",
                "/package/pkg_id",
                "/agent_pkg_name",
                "/app/pkg_name",
            ],
        )
        .map(pkg_unique_name)
        .map(str::to_string)
    })?;

    Some(resolve_package_root_candidates(&pkg_name))
}

fn pkg_unique_name(pkg_id_or_name: &str) -> &str {
    pkg_id_or_name
        .split('#')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(pkg_id_or_name)
}

fn resolve_agent_env_root_from_spec_mounts(spec: &AgentAppSpec) -> Option<PathBuf> {
    for key in ["root", "agent_root", "agent_env", "workspace"] {
        if let Some(path) = spec.install_config.data_mount_point.get(key) {
            let path = path.trim();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    None
}

fn resolve_agent_pkg_name_from_spec(spec: &AgentAppSpec) -> Option<String> {
    if let Some(agent_pkg) = spec.app_doc.pkg_list.agent.as_ref() {
        return Some(pkg_unique_name(agent_pkg.pkg_id.as_str()).to_string());
    }

    let app_name = spec.app_doc.name.trim();
    if !app_name.is_empty() {
        return Some(app_name.to_string());
    }

    None
}

fn resolve_package_root_candidates(pkg_name: &str) -> PathBuf {
    let installed = get_buckyos_root_dir().join("bin").join(pkg_name);
    if installed.exists() {
        return installed;
    }

    let Some(current_dir) = std::env::current_dir().ok() else {
        return installed;
    };
    for candidate in [
        current_dir.join("src/rootfs/bin").join(pkg_name),
        current_dir.join("../rootfs/bin").join(pkg_name),
        current_dir.join("rootfs/bin").join(pkg_name),
    ] {
        if candidate.exists() {
            return candidate;
        }
    }

    installed
}

fn resolve_agent_did(doc: Option<&AgentInstanceDoc>) -> Option<String> {
    doc.map(|doc| doc.doc.get_id().to_string())
}

fn resolve_agent_owner_did(doc: Option<&AgentInstanceDoc>) -> Option<String> {
    doc.map(|doc| doc.doc.owner.to_string())
}

fn absolutize_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()
        .context("read current_dir failed")?
        .join(path))
}

async fn write_cached_agent_doc(agent_env_root: &PathBuf, doc: &AgentInstanceDoc) -> Result<()> {
    fs::create_dir_all(agent_env_root).await.map_err(|err| {
        anyhow!(
            "create agent env root failed: path={} err={}",
            agent_env_root.display(),
            err
        )
    })?;
    let path = agent_env_root.join("agent.json.doc");
    let pretty = serde_json::to_string_pretty(&doc.json)
        .context("serialize cached agent instance doc failed")?;
    fs::write(&path, pretty).await.map_err(|err| {
        anyhow!(
            "write cached agent instance doc failed: path={} err={}",
            path.display(),
            err
        )
    })?;
    Ok(())
}

async fn write_cached_agent_spec(agent_env_root: &PathBuf, spec: &AgentAppSpec) -> Result<()> {
    fs::create_dir_all(agent_env_root).await.map_err(|err| {
        anyhow!(
            "create agent env root failed before spec cache: path={} err={}",
            agent_env_root.display(),
            err
        )
    })?;
    let path = agent_env_root.join("agent.app.json");
    let pretty =
        serde_json::to_string_pretty(&spec.json).context("serialize cached agent spec failed")?;
    fs::write(&path, pretty).await.map_err(|err| {
        anyhow!(
            "write cached agent spec failed: path={} err={}",
            path.display(),
            err
        )
    })?;
    Ok(())
}

fn resolve_session_worker_threads(default_value: usize) -> usize {
    let Ok(raw) = std::env::var(OPENDAN_SESSION_WORKER_THREADS_ENV) else {
        return default_value;
    };
    let parsed = raw.trim().parse::<usize>();
    match parsed {
        Ok(value) if value > 0 => value,
        _ => {
            warn!(
                "invalid {} value `{}`; fallback to {}",
                OPENDAN_SESSION_WORKER_THREADS_ENV, raw, default_value
            );
            default_value
        }
    }
}

async fn run_agent(launch: &LaunchConfig, deps: AIAgentDeps) -> Result<()> {
    let mut cfg = AIAgentConfig::new(&launch.agent_env_root);
    cfg.agent_instance_id = launch.agent_id.clone();
    cfg.agent_package_root = launch.agent_package_root.clone();
    cfg.agent_did = launch.agent_did.clone();
    cfg.agent_owner_did = launch.agent_owner_did.clone();
    cfg.session_worker_threads = resolve_session_worker_threads(cfg.session_worker_threads);
    let session_worker_threads = cfg.session_worker_threads;

    let agent = Arc::new(AIAgent::load(cfg, deps).await.map_err(|err| {
        anyhow!(
            "load agent failed: instance={} root={}, err={}",
            launch.agent_id,
            launch.agent_env_root.display(),
            err
        )
    })?);
    info!(
        "opendan agent loaded: instance={} did={} root={} package_root={} session_workers={}",
        launch.agent_id,
        agent.did(),
        launch.agent_env_root.display(),
        launch
            .agent_package_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        session_worker_threads
    );
    agent.clone().run_agent_loop(None).await.map_err(|err| {
        anyhow!(
            "agent loop failed: instance={} did={} root={}, err={}",
            launch.agent_id,
            agent.did(),
            launch.agent_env_root.display(),
            err
        )
    })?;
    Ok(())
}

async fn service_main() -> Result<()> {
    init_logging("opendan", true);
    //install_panic_hook();
    //install_signal_logging();
    info!("starting opendan service...");
    let startup = parse_startup_args().context("parse opendan startup args failed")?;
    info!(
        "opendan startup args: agent_id={} agent_env={} agent_bin={} service_port={}",
        startup.agent_id.as_deref().unwrap_or("<none>"),
        startup
            .agent_env
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        startup
            .agent_bin
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        startup
            .service_port
            .map(|port| port.to_string())
            .unwrap_or_else(|| "<none>".to_string())
    );
    let agent_id = resolve_requested_agent_id(&startup)?;
    let owner_id = resolve_requested_owner_id(&startup)?;
    let service_port = resolve_requested_service_port(&startup)?;
    info!(
        "opendan runtime init request: agent_id={} owner_id={} resolved_service_port={}",
        agent_id, owner_id, service_port
    );

    let mut runtime = init_buckyos_api_runtime(
        &agent_id,
        Some(owner_id.clone()),
        BuckyOSRuntimeType::AppService,
    )
    .await
    .context("init buckyos runtime for opendan failed")?;
    runtime
        .login()
        .await
        .context("opendan login to buckyos failed")?;
    info!(
        "opendan runtime initialized: app_id={} owner_id={}",
        agent_id, owner_id
    );
    info!(
        "opendan login succeeded, waiting {}s for dependent services to get ready",
        OPENDAN_STARTUP_DEP_READY_WAIT_SECS
    );
    sleep(Duration::from_secs(OPENDAN_STARTUP_DEP_READY_WAIT_SECS)).await;
    info!("opendan startup wait finished, continue service bootstrap");
    info!("setting opendan main service port to {}", service_port);
    runtime.set_main_service_port(service_port).await;
    info!("registering global buckyos runtime for opendan");
    set_buckyos_api_runtime(runtime);
    info!("global buckyos runtime registered for opendan");

    info!("loading opendan agent instance doc: agent_id={}", agent_id);
    let agent_doc = load_agent_instance_doc(&agent_id).await?;
    info!(
        "loaded opendan agent instance doc: found={}",
        agent_doc.is_some()
    );
    let agent_owner_did = resolve_agent_owner_did(agent_doc.as_ref());
    info!(
        "loading opendan agent spec: agent_id={} owner_did={}",
        agent_id,
        agent_owner_did.as_deref().unwrap_or("<none>")
    );
    let agent_spec = load_agent_app_spec(&agent_id, agent_owner_did.as_deref()).await?;
    info!("loaded opendan agent spec: found={}", agent_spec.is_some());
    let agent_env_root = absolutize_path(resolve_agent_env_root(
        &startup,
        agent_spec.as_ref(),
        agent_doc.as_ref(),
        &agent_id,
    ))?;
    let agent_package_root =
        resolve_agent_package_root(&startup, agent_spec.as_ref(), agent_doc.as_ref())
            .map(absolutize_path)
            .transpose()?;
    let launch = LaunchConfig {
        agent_id: agent_id.clone(),
        agent_env_root,
        agent_package_root,
        agent_did: resolve_agent_did(agent_doc.as_ref()),
        agent_owner_did,
        agent_doc,
        agent_spec,
    };
    if let Some(doc) = &launch.agent_doc {
        write_cached_agent_doc(&launch.agent_env_root, doc).await?;
    }
    if let Some(spec) = &launch.agent_spec {
        write_cached_agent_spec(&launch.agent_env_root, spec).await?;
    }
    info!(
        "opendan launch resolved: instance={} service_port={} env_root={} package_root={} did={} owner={} spec_key={} spec_user={}",
        launch.agent_id,
        service_port,
        launch.agent_env_root.display(),
        launch
            .agent_package_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        launch.agent_did.as_deref().unwrap_or("<auto>"),
        launch.agent_owner_did.as_deref().unwrap_or("<none>"),
        launch
            .agent_spec
            .as_ref()
            .map(|spec| spec.key.as_str())
            .unwrap_or("<none>"),
        launch
            .agent_spec
            .as_ref()
            .and_then(|spec| spec.user_id.as_deref())
            .unwrap_or("<none>")
    );

    let runtime = get_buckyos_api_runtime().context("load runtime failed after init")?;
    info!("resolved global runtime after registration");

    info!("initializing task-manager client");
    let taskmgr = Arc::new(
        runtime
            .get_task_mgr_client()
            .await
            .context("init task-manager client failed")?,
    );
    if let Ok(url) = runtime
        .get_zone_service_url(AICC_SERVICE_SERVICE_NAME, runtime.force_https)
        .await
    {
        info!("opendan resolved aicc endpoint: {}", url);
    }
    let msg_center = match runtime.get_msg_center_client().await {
        Ok(client) => Some(Arc::new(client)),
        Err(err) => {
            warn!("init msg-center client failed, continue without chat history: {err}");
            None
        }
    };

    let msg_queue: Option<MsgQueueClient> = match runtime.get_msg_queue_client().await {
        Ok(client) => Some(client),
        Err(err) => {
            warn!("init msg-queue client failed, continue without queue polling: {err}");
            None
        }
    };

    let deps = AIAgentDeps {
        taskmgr,
        msg_center,
        msg_queue: msg_queue.map(Arc::new),
    };

    info!(
        "initializing opendan ai runtime: env_root={}",
        launch.agent_env_root.display()
    );
    let ai_runtime = Arc::new(
        AiRuntime::new(AiRuntimeConfig::new(&launch.agent_env_root))
            .await
            .context("init opendan ai runtime for rpc failed")?,
    );
    info!("opendan ai runtime initialized");
    if let Some(agent_did) = launch.agent_did.as_deref() {
        info!("registering root agent into ai runtime: did={}", agent_did);
        ai_runtime
            .register_agent(agent_did, &launch.agent_env_root)
            .await
            .context("register root agent into opendan rpc runtime failed")?;
        info!("registered root agent into ai runtime: did={}", agent_did);
    }
    let server = Arc::new(OpenDanHttpServer::new(ai_runtime));
    let runner = Runner::new(service_port);
    runner
        .add_http_server("/kapi/opendan".to_string(), server)
        .map_err(|err| anyhow!("failed to add opendan http server: {err:?}"))?;
    info!(
        "opendan http server registered, entering select loop: service_port={}",
        service_port
    );

    tokio::select! {
        runner_result = runner.run() => {
            runner_result
                .map_err(|err| anyhow!("opendan runner exited with error: {err:?}"))?;
            Err(anyhow!("opendan runner exited unexpectedly"))
        }
        agent_result = run_agent(&launch, deps) => {
            agent_result
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = service_main().await {
        error!("opendan service exited with error: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        agent_spec_key_candidates, parse_startup_args_from_iter, resolve_owner_from_agent_env,
        resolve_requested_owner_id, resolve_requested_service_port, StartupArgs,
        DEFAULT_OPENDAN_SERVICE_PORT,
    };
    use std::path::PathBuf;

    #[test]
    fn agent_spec_key_candidates_include_agents_and_apps_paths() {
        let keys = agent_spec_key_candidates("jarvis", "did:bns:alice");
        assert_eq!(
            keys,
            vec![
                "users/did:bns:alice/agents/jarvis/spec".to_string(),
                "users/alice/agents/jarvis/spec".to_string(),
            ]
        );
    }

    #[test]
    fn parse_startup_args_accepts_service_port_override() {
        let parsed =
            parse_startup_args_from_iter(["--agent-id", "jarvis", "--service-port", "12016"])
                .expect("parse startup args");

        assert_eq!(parsed.agent_id.as_deref(), Some("jarvis"));
        assert_eq!(parsed.service_port, Some(12016));
    }

    #[test]
    fn resolve_owner_from_agent_env_extracts_home_owner() {
        let owner = resolve_owner_from_agent_env(Some(&PathBuf::from(
            "/opt/buckyos/data/home/devtest/.local/share/jarvis",
        )));

        assert_eq!(owner.as_deref(), Some("devtest"));
    }

    #[test]
    fn resolve_requested_owner_id_uses_agent_env_as_fallback() {
        let startup = StartupArgs {
            agent_env: Some(PathBuf::from(
                "/opt/buckyos/data/home/devtest/.local/share/jarvis",
            )),
            ..Default::default()
        };

        assert_eq!(
            resolve_requested_owner_id(&startup).expect("resolve owner"),
            "devtest"
        );
    }

    #[test]
    fn resolve_requested_service_port_prefers_cli_value() {
        let startup = StartupArgs {
            service_port: Some(12016),
            ..Default::default()
        };

        assert_eq!(
            resolve_requested_service_port(&startup).expect("resolve service port"),
            12016
        );
        assert_ne!(DEFAULT_OPENDAN_SERVICE_PORT, 12016);
    }
}
