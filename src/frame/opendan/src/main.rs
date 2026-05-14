//! opendan binary entry.
//!
//! The refactor (notepads/NewOpenDANRuntime.md) builds the new runtime bottom-up.
//! Step 2 — `LLMContextDeps` assembly — is wired in `ai_runtime.rs`. The
//! top-level `AIAgent::run` (§9 step 6) is not yet implemented, so `main`
//! initialises the shared dependencies and parks until SIGINT/SIGTERM so a
//! service supervisor sees the process as alive.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime,
    BuckyOSRuntimeType, OPENDAN_SERVICE_NAME,
};
use buckyos_kit::init_logging;
use log::{error, info, warn};

use opendan::ai_runtime::AgentRuntime;
use opendan::worklog::{WorklogService, WorklogToolConfig};

const WORKLOG_DB_ENV: &str = "OPENDAN_WORKLOG_DB";
const DEFAULT_WORKLOG_DB: &str = "/opt/buckyos/opendan/worklog.db";

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
    let _runtime = bootstrap().await?;
    // TODO(notepads/NewOpenDANRuntime.md §9 step 6): construct `AIAgent`
    // from the agent root + the runtime above, then call `AIAgent::run` here.
    warn!(
        "opendan.runtime: AIAgent::run is not yet implemented (see notepads/NewOpenDANRuntime.md §9 step 6); parking process until shutdown signal"
    );
    tokio::signal::ctrl_c()
        .await
        .map_err(|err| anyhow!("install ctrl_c handler failed: {err}"))?;
    info!("opendan: received shutdown signal, exiting");
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
