//! Workflow Service 入口。
//!
//! 负责"服务化"那一层：注册 buckyos-api runtime、装配 [`DefinitionStore`] /
//! [`RunStore`] / orchestrator / function registry / local thunk runner /
//! task tracker，把 kRPC 请求路由到 [`WorkflowRpcHandler`]。
//!
//! 设计参考 [doc/workflow/workflow service.md](../../../../doc/workflow/workflow%20service.md)
//! 与 notepads/llm-media-pipeline-workflow-todo.md。

mod adapters;
mod analysis;
mod cache_index;
mod compiler;
mod dispatcher;
mod driver;
mod dsl;
mod error;
mod executor_adapter;
mod function_registry;
mod functions;
mod io;
mod local_thunk_runner;
mod native;
mod object_store;
mod orchestrator;
mod persistence;
mod runtime;
mod scheduled_task_manager;
mod schema;
mod send_message_executor;
mod server;
mod state;
mod subscriptions;
mod system_defs;
mod task_tracker;
mod types;

pub mod service_schemas {
    //! workflow 视角的服务 schema 定义。它们不是协议本身，是 DSL 作者写
    //! `executor: "service::xxx.yyy"` 时引擎用来约束输入输出的 workflow 子集。
    pub mod aicc {
        pub use crate::adapters::aicc::{
            aicc_method_schema, aicc_method_schemas, AiccAdapter, AiccMethodSchema,
            AICC_EXECUTOR_PREFIX,
        };
    }
}

pub use analysis::{analyze_workflow, AnalysisReport, AnalysisSeverity};
pub use buckyos_api::{
    FunctionObject, FunctionParamType, FunctionRegistration, FunctionResultType, FunctionType,
    ResourceRequirements, ThunkExecution, ThunkExecutionError, ThunkExecutionResult,
    ThunkExecutionStatus, ThunkObject,
};
pub use cache_index::{CacheEntry, CacheIndex, MemoryCacheIndex};
pub use compiler::{
    compile_workflow, compile_workflow_with, CompileOutput, CompiledNode, CompiledWorkflow,
    WorkflowGraph,
};
pub use dispatcher::{InMemoryThunkDispatcher, ScheduledThunk, ThunkDispatcher};
pub use driver::RunDriver;
pub use dsl::*;
pub use error::{WorkflowError, WorkflowResult};
pub use executor_adapter::{ExecutorAdapter, ExecutorRegistry, NamespaceAdapter};
pub use function_registry::{FunctionRegistry, RegisteredFunction, RegistrationSpec};
pub use io::{MemoryNativeIo, ZoneNativeIo};
pub use local_thunk_runner::LocalThunkRunner;
pub use native::*;
pub use object_store::{InMemoryObjectStore, NamedStoreObjectStore, WorkflowObjectStore};
pub use orchestrator::{RunOptions, WorkflowOrchestrator};
pub use persistence::{ArtifactIndex, ExecutionLedger, MemoryExecutionLedger, WorkflowDb};
pub use runtime::*;
pub use server::ServiceOrchestrator;
pub use task_tracker::{
    MapShardTaskView, NoopTaskTracker, RecordingTaskTracker, StepTaskView, TaskManagerTaskTracker,
    ThunkTaskPhase, ThunkTaskView, WorkflowTaskTracker,
};
pub use types::*;

use ::kRPC::*;
use anyhow::Result;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType,
    WORKFLOW_SERVICE_HTTP_PATH, WORKFLOW_SERVICE_NAME, WORKFLOW_SERVICE_PORT,
};
use buckyos_http_server::Runner;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use buckyos_kit::init_logging;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use std::sync::Arc;

use crate::scheduled_task_manager::{ScheduleStore, ScheduleTaskMirrorClient};
use crate::send_message_executor::SendMessageTaskExecutor;
use crate::server::{NdmInputAuthorizer, WorkflowRpcHandler};
use crate::service_schemas::aicc::AiccAdapter;
use crate::state::{DefinitionStore, RunStore, ServiceTracker};
use crate::subscriptions::RunSubscriptionManager;

struct WorkflowHttpServer {
    rpc: Arc<WorkflowRpcHandler>,
}

impl WorkflowHttpServer {
    fn new(rpc: Arc<WorkflowRpcHandler>) -> Self {
        Self { rpc }
    }
}

#[async_trait::async_trait]
impl RPCHandler for WorkflowHttpServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: std::net::IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        self.rpc.handle_rpc_call(req, ip_from).await
    }
}

#[async_trait::async_trait]
impl HttpServer for WorkflowHttpServer {
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
        WORKFLOW_SERVICE_NAME.to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_workflow_service() -> Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        WORKFLOW_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    runtime
        .login()
        .await
        .map_err(|err| anyhow::anyhow!("workflow service login failed: {:?}", err))?;
    runtime.set_main_service_port(WORKFLOW_SERVICE_PORT).await;
    let buckyos_root = runtime.buckyos_root_dir.clone();
    let app_id = runtime.app_id.clone();
    let named_store = runtime
        .get_local_named_store()
        .await
        .map_err(|err| anyhow::anyhow!("workflow local named store unavailable: {:?}", err))?;

    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow::anyhow!("register workflow runtime failed: {}", err))?;

    // Service-local persistent state: cache index, execution epoch, ledger.
    let data_dir = buckyos_root.join("data").join("srv").join("workflow");
    let db = WorkflowDb::open(&data_dir.join("workflow.db"))
        .map_err(|err| anyhow::anyhow!("open workflow db failed: {}", err))?;
    let epoch = db
        .next_epoch()
        .map_err(|err| anyhow::anyhow!("advance workflow epoch failed: {}", err))?;
    match db.purge_expired_cache(chrono::Utc::now().timestamp()) {
        Ok(purged) if purged > 0 => info!("workflow cache index: purged {} expired entries", purged),
        Ok(_) => {}
        Err(err) => warn!("workflow cache index purge failed: {}", err),
    }
    info!("workflow service epoch {}", epoch);

    let tracker = Arc::new(ServiceTracker::from_runtime(app_id.clone()));
    info!("workflow tracker bound to task_manager runtime client");

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(AiccAdapter::from_runtime()));
    info!("workflow registered service::aicc.* runtime adapter");
    let registry = Arc::new(registry);

    // Built-in FunctionObjects, published into the Named Store under their
    // real ids so every bound fun_id can be read back.
    let object_store: Arc<dyn WorkflowObjectStore> = Arc::new(NamedStoreObjectStore::new(
        named_store.clone(),
        std::time::Duration::from_secs(u64::from(buckyos_api::WORKFLOW_ARTIFACTS_TTL_DAYS) * 86_400),
    ));
    let mut functions = FunctionRegistry::new();
    functions::register_builtin_functions(&mut functions)
        .map_err(|err| anyhow::anyhow!("register builtin functions failed: {}", err))?;
    functions
        .publish(object_store.as_ref())
        .await
        .map_err(|err| anyhow::anyhow!("publish builtin functions failed: {}", err))?;
    let functions = Arc::new(functions);
    info!("workflow published {} builtin functions", functions.list().len());

    // Tool environment of this host; media function fingerprints derive from it.
    let environment = Arc::new(
        tokio::task::spawn_blocking(functions::media::tools::probe_default)
            .await
            .map_err(|err| anyhow::anyhow!("probe media tools failed: {}", err))?,
    );
    info!(
        "workflow media tools revision={} present={:?} missing={:?}",
        environment.revision,
        environment.tools.keys().collect::<Vec<_>>(),
        environment.missing
    );
    let environment_provider: Arc<dyn EnvironmentProvider> =
        Arc::new(StaticEnvironment(environment.clone()));

    let artifacts_bucket = buckyos_api::workflow_artifacts_bucket();
    let io: Arc<dyn NativeIo> = Arc::new(ZoneNativeIo::new(
        named_store,
        object_store.clone(),
        artifacts_bucket,
        buckyos_api::WORKFLOW_ARTIFACTS_TTL_DAYS,
        db.clone(),
    ));

    let (result_tx, result_rx) = tokio::sync::mpsc::channel(256);
    let temp_root = buckyos_root
        .join("data")
        .join("cache")
        .join("workflow")
        .join("exec");
    let runner = Arc::new(LocalThunkRunner::new(
        functions.clone(),
        io.clone(),
        environment_provider.clone(),
        tracker.clone(),
        db.clone(),
        result_tx,
        temp_root,
        epoch,
        ExecutionLimits::default(),
    ));
    match runner.cleanup_stale().await {
        Ok(stale) if !stale.is_empty() => {
            info!("workflow runner: {} interrupted executions cleaned", stale.len())
        }
        Ok(_) => {}
        Err(err) => warn!("workflow runner: stale execution cleanup failed: {}", err),
    }

    let definitions = Arc::new(DefinitionStore::new());
    let runs = Arc::new(RunStore::new());
    let schedule_mirror = Arc::new(ScheduleTaskMirrorClient::from_runtime(app_id.clone()));
    let schedules = Arc::new(ScheduleStore::new_memory());
    SendMessageTaskExecutor::from_runtime(schedules.clone(), buckyos_root).start();
    info!("workflow.send_message executor started");

    let dispatcher: Arc<dyn ThunkDispatcher> = runner.clone();
    let orchestrator = Arc::new(
        WorkflowOrchestrator::new(dispatcher, object_store.clone(), tracker)
            .with_executor_registry(registry)
            .with_functions(functions.clone())
            .with_cache(db.clone())
            .with_environment(environment_provider)
            .with_io(io),
    );

    let seeded = system_defs::seed_system_definitions(&definitions, &functions)
        .await
        .map_err(|err| anyhow::anyhow!("seed system definitions failed: {}", err))?;
    info!(
        "workflow seeded {} system definitions: {:?}",
        seeded.len(),
        seeded.iter().map(|d| (d.id.clone(), d.compiled.revision.clone())).collect::<Vec<_>>()
    );

    let kevent_client = get_buckyos_api_runtime()
        .map_err(|err| anyhow::anyhow!("workflow api runtime unavailable: {:?}", err))?
        .get_kevent_client()
        .await
        .map_err(|err| anyhow::anyhow!("workflow kevent client unavailable: {:?}", err))?;
    let subscriptions = RunSubscriptionManager::new(
        kevent_client,
        runs.clone(),
        definitions.clone(),
        orchestrator.clone(),
        None,
    );
    subscriptions.start().await;

    let rpc_handler = WorkflowRpcHandler::new(definitions, runs, orchestrator, object_store)
        .with_schedules(schedules)
        .with_subscriptions(subscriptions)
        .with_schedule_mirror(schedule_mirror)
        .with_input_authorizer(Arc::new(NdmInputAuthorizer))
        .with_execution_epoch(epoch);
    rpc_handler.driver().start_result_loop(result_rx);
    let rpc = Arc::new(rpc_handler);
    start_schedule_loop(rpc.clone());
    let server = Arc::new(WorkflowHttpServer::new(rpc));

    let runner = Runner::new(WORKFLOW_SERVICE_PORT);
    runner
        .add_http_server(WORKFLOW_SERVICE_HTTP_PATH.to_string(), server)
        .map_err(|err| anyhow::anyhow!("add workflow http server failed: {:?}", err))?;

    info!(
        "workflow service starting at port {} path {}",
        WORKFLOW_SERVICE_PORT, WORKFLOW_SERVICE_HTTP_PATH
    );
    runner
        .run()
        .await
        .map_err(|err| anyhow::anyhow!("workflow runner exited: {:?}", err))?;
    Ok(())
}

fn start_schedule_loop(rpc: Arc<WorkflowRpcHandler>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            ticker.tick().await;
            rpc.scan_due_schedules().await;
        }
    });
}

#[tokio::main]
async fn main() {
    init_logging("", true);
    if let Err(err) = start_workflow_service().await {
        error!("workflow service start failed: {:?}", err);
    }
}
