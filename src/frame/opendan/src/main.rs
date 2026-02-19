#[allow(non_snake_case)]
pub mod agent;
pub mod agent_enviroment;
pub mod agent_memory;
pub mod agent_session;
pub mod agent_tool;
pub mod ai_runtime;
pub mod ai_thread;
pub mod behavior;
pub mod workspace;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use buckyos_api::msg_queue::MsgQueueClient;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType,
    AICC_SERVICE_SERVICE_NAME, OPENDAN_SERVICE_NAME, OPENDAN_SERVICE_PORT,
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
use server_runner::Runner;
use tokio::fs;
use tokio::task::JoinSet;

use crate::agent::{AIAgent, AIAgentConfig, AIAgentDeps};
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

const OPENDAN_AGENTS_ROOT_ENV: &str = "OPENDAN_AGENTS_ROOT";

fn resolve_agents_root() -> Result<PathBuf> {
    if let Ok(path) = std::env::var(OPENDAN_AGENTS_ROOT_ENV) {
        let path = path.trim();
        if !path.is_empty() {
            let root = PathBuf::from(path);
            return if root.is_absolute() {
                Ok(root)
            } else {
                Ok(std::env::current_dir()
                    .context("read current_dir failed")?
                    .join(root))
            };
        }
    }

    Ok(get_buckyos_root_dir().join("agents"))
}

async fn discover_agent_roots(agents_root: &Path) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(agents_root).await.map_err(|err| {
        anyhow!(
            "create agents root failed: path={}, err={}",
            agents_root.display(),
            err
        )
    })?;

    let mut roots = Vec::<PathBuf>::new();
    let mut read_dir = fs::read_dir(agents_root).await.map_err(|err| {
        anyhow!(
            "read agents root failed: path={}, err={}",
            agents_root.display(),
            err
        )
    })?;

    while let Some(entry) = read_dir.next_entry().await.map_err(|err| {
        anyhow!(
            "iterate agents root failed: path={}, err={}",
            agents_root.display(),
            err
        )
    })? {
        if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            roots.push(entry.path());
        }
    }

    roots.sort_by_key(|path| path.to_string_lossy().to_string());
    Ok(roots)
}

async fn run_agent(agent_root: PathBuf, deps: AIAgentDeps) -> Result<()> {
    let agent = AIAgent::load(AIAgentConfig::new(&agent_root), deps)
        .await
        .map_err(|err| {
            anyhow!(
                "load agent failed: root={}, err={}",
                agent_root.display(),
                err
            )
        })?;
    info!(
        "opendan agent loaded: did={} root={}",
        agent.did(),
        agent_root.display()
    );
    agent.start(None).await.map_err(|err| {
        anyhow!(
            "agent loop failed: did={} root={}, err={}",
            agent.did(),
            agent_root.display(),
            err
        )
    })?;
    Ok(())
}

async fn run_agents_supervisor(agent_roots: Vec<PathBuf>, deps: AIAgentDeps) -> Result<()> {
    let mut join_set = JoinSet::new();
    for agent_root in agent_roots {
        let deps = deps.clone();
        let root_for_log = agent_root.display().to_string();
        join_set.spawn(async move { (root_for_log, run_agent(agent_root, deps).await) });
    }

    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok((agent_root, Ok(()))) => warn!("agent loop exited: root={agent_root}"),
            Ok((agent_root, Err(err))) => {
                error!("agent loop failed: root={agent_root}, err={err}");
            }
            Err(err) => {
                error!("agent task join failed: {err}");
            }
        }
    }

    Err(anyhow!("all opendan agents exited"))
}

async fn service_main() -> Result<()> {
    init_logging("opendan", true);
    info!("starting opendan service...");

    let mut runtime =
        init_buckyos_api_runtime(OPENDAN_SERVICE_NAME, None, BuckyOSRuntimeType::FrameService)
            .await
            .context("init buckyos runtime for opendan failed")?;
    runtime
        .login()
        .await
        .context("opendan login to buckyos failed")?;
    runtime.set_main_service_port(OPENDAN_SERVICE_PORT).await;
    set_buckyos_api_runtime(runtime);

    let runtime = get_buckyos_api_runtime().context("load runtime failed after init")?;

    let taskmgr = Arc::new(
        runtime
            .get_task_mgr_client()
            .await
            .context("init task-manager client failed")?,
    );
    let aicc = Arc::new(
        runtime
            .get_aicc_client()
            .await
            .context("init aicc client failed")?,
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

    let _msg_queue: Option<MsgQueueClient> = match runtime.get_msg_queue_client().await {
        Ok(client) => Some(client),
        Err(err) => {
            warn!("init msg-queue client failed, continue without queue polling: {err}");
            None
        }
    };

    let agents_root = resolve_agents_root()?;
    let agent_roots = discover_agent_roots(&agents_root).await?;
    if agent_roots.is_empty() {
        return Err(anyhow!(
            "no agent found in {}, set {} to override",
            agents_root.display(),
            OPENDAN_AGENTS_ROOT_ENV
        ));
    }

    info!(
        "opendan discovered {} agent(s) in {}",
        agent_roots.len(),
        agents_root.display()
    );

    let deps = AIAgentDeps {
        taskmgr,
        aicc,
        msg_center,
    };

    let ai_runtime = Arc::new(
        AiRuntime::new(AiRuntimeConfig::new(&agents_root))
            .await
            .context("init opendan ai runtime for rpc failed")?,
    );
    let server = Arc::new(OpenDanHttpServer::new(ai_runtime));
    let runner = Runner::new(OPENDAN_SERVICE_PORT);
    runner
        .add_http_server("/kapi/opendan".to_string(), server)
        .map_err(|err| anyhow!("failed to add opendan http server: {err:?}"))?;

    tokio::select! {
        runner_result = runner.run() => {
            runner_result
                .map_err(|err| anyhow!("opendan runner exited with error: {err:?}"))?;
            Err(anyhow!("opendan runner exited unexpectedly"))
        }
        agents_result = run_agents_supervisor(agent_roots, deps) => {
            agents_result
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
