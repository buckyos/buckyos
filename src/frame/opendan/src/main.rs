//! opendan binary entry.
//!
//! Wires the §9 components together: bootstrap shared deps (aicc + worklog) →
//! open `AIAgent` over the configured agent root → run the dispatcher loop.
//! SIGINT triggers a graceful shutdown via `AIAgent::shutdown`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime,
    BuckyOSRuntimeType, OPENDAN_SERVICE_NAME,
};
use buckyos_kit::init_logging;
use log::{error, info};

use opendan::agent::AIAgent;
use opendan::ai_runtime::AgentRuntime;
use opendan::worklog::{WorklogService, WorklogToolConfig};

const WORKLOG_DB_ENV: &str = "OPENDAN_WORKLOG_DB";
const DEFAULT_WORKLOG_DB: &str = "/opt/buckyos/opendan/worklog.db";
const AGENT_ROOT_ENV: &str = "OPENDAN_AGENT_ROOT";
const DEFAULT_AGENT_ROOT: &str = "/opt/buckyos/opendan/agent";

async fn bootstrap() -> Result<Arc<AgentRuntime>> {
    let runtime = init_buckyos_api_runtime(
        OPENDAN_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::FrameService,
    )
    .await
    .map_err(|err| anyhow!("init buckyos runtime failed: {err}"))?;
    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow!("register buckyos runtime failed: {err}"))?;

    let api_runtime = get_buckyos_api_runtime()
        .map_err(|err| anyhow!("load buckyos runtime failed: {err}"))?;
    let aicc = api_runtime
        .get_aicc_client()
        .await
        .map_err(|err| anyhow!("init aicc client failed: {err}"))?;

    let worklog_db = std::env::var(WORKLOG_DB_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_WORKLOG_DB));
    let worklog = WorklogService::new(WorklogToolConfig::with_db_path(worklog_db.clone()))
        .with_context(|| format!("open worklog db at {}", worklog_db.display()))?;

    info!(
        "opendan.bootstrap: aicc=ready worklog_db={}",
        worklog_db.display()
    );
    Ok(Arc::new(AgentRuntime::new(Arc::new(aicc), Arc::new(worklog))))
}

async fn run() -> Result<()> {
    let runtime = bootstrap().await?;

    let agent_root = std::env::var(AGENT_ROOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_AGENT_ROOT));
    std::fs::create_dir_all(&agent_root)
        .with_context(|| format!("create agent root at {}", agent_root.display()))?;
    info!("opendan.bootstrap: agent_root={}", agent_root.display());

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
