//! Workflow Service 入口。
//!
//! 负责"服务化"那一层：注册 buckyos-api runtime、装配 [`DefinitionStore`] /
//! [`RunStore`] / orchestrator / executor registry / task tracker，把 kRPC
//! 请求路由到 [`WorkflowRpcHandler`]。Definition / Run / Amendment / 事件等
//! 具体业务逻辑由 [`workflow`] crate 中已有的编排器、object store、task tracker
//! 承担。
//!
//! 设计参考 [doc/workflow/workflow service.md](../../../../doc/workflow/workflow%20service.md)。

mod adapters;
mod analysis;
mod compiler;
mod dispatcher;
mod dsl;
mod error;
mod executor_adapter;
mod object_store;
mod orchestrator;
mod runtime;
mod schema;
mod server;
mod state;
mod subscriptions;
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
    FunctionObject, FunctionParamType, FunctionResultType, FunctionType, ResourceRequirements,
    ThunkExecutionResult, ThunkExecutionStatus, ThunkObject,
};
pub use compiler::{
    compile_workflow, CompileOutput, CompiledNode, CompiledWorkflow, WorkflowGraph,
};
pub use dispatcher::{InMemoryThunkDispatcher, ScheduledThunk, ThunkDispatcher};
pub use dsl::*;
pub use error::{WorkflowError, WorkflowResult};
pub use executor_adapter::{ExecutorAdapter, ExecutorRegistry, NamespaceAdapter};
pub use object_store::{InMemoryObjectStore, NamedStoreObjectStore, WorkflowObjectStore};
pub use orchestrator::WorkflowOrchestrator;
pub use runtime::*;
pub use task_tracker::{
    MapShardTaskView, NoopTaskTracker, RecordingTaskTracker, StepTaskView, TaskManagerTaskTracker,
    ThunkTaskView, WorkflowTaskTracker,
};
pub use types::*;

use ::kRPC::*;
use anyhow::Result;
use buckyos_api::{
    init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType, KEventClient,
    WORKFLOW_SERVICE_HTTP_PATH, WORKFLOW_SERVICE_NAME, WORKFLOW_SERVICE_PORT,
};
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use buckyos_kit::init_logging;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use buckyos_http_server::Runner;
use std::sync::Arc;

use crate::server::WorkflowRpcHandler;
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

    // §6.3：Run / Step / Map shard / Thunk 同步到 task_manager。客户端拿不到
    // 时退化为 noop（参考 §1.2 的"task_manager 不可用时仍可推进 adapter 主路径"）。
    let user_id = runtime.user_id.clone().unwrap_or_default();
    let app_id = runtime.app_id.clone();
    let tracker = match runtime.get_task_mgr_client().await {
        Ok(client) => {
            info!("workflow tracker bound to task_manager");
            ServiceTracker::from_task_manager(Arc::new(client), user_id, app_id)
        }
        Err(err) => {
            warn!(
                "task_manager client unavailable, workflow runs will not be mirrored to task tree: {}",
                err
            );
            ServiceTracker::noop()
        }
    };

    // §1.2 / §9：一期 P0 必须有的 service:: adapter。aicc 拿不到客户端时不算
    // 致命错误——其他 adapter 仍然能跑，只是涉及 aicc 的 step 会落到通用错误
    // 路径并按 retry / human fallback 处理。
    let mut registry = ExecutorRegistry::new();
    match runtime.get_aicc_client().await {
        Ok(client) => {
            registry.register(Arc::new(AiccAdapter::new(Arc::new(client))));
            info!("workflow registered service::aicc.* adapter");
        }
        Err(err) => warn!(
            "aicc client unavailable, service::aicc.* steps will fail until aicc is online: {}",
            err
        ),
    }
    let registry = Arc::new(registry);

    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow::anyhow!("register workflow runtime failed: {}", err))?;

    let definitions = Arc::new(DefinitionStore::new());
    let runs = Arc::new(RunStore::new());
    // 一期使用进程内 dispatcher / object store；§5.1 提到的 sled / Named Object
    // Store 持久化是后续提交的工作。`func::*` 调度路径暂时不接 Scheduler，命中
    // 时会按 §6.2 落到 require_function_object 的明确错误。
    let dispatcher = Arc::new(InMemoryThunkDispatcher::new());
    let object_store = Arc::new(InMemoryObjectStore::new());
    let orchestrator = Arc::new(
        WorkflowOrchestrator::new(dispatcher, object_store, Arc::new(tracker))
            .with_executor_registry(registry),
    );

    // §3.3：用户在 TaskMgr UI 上点按钮 = 写一次 TaskData，task_manager 把
    // TaskData 变更扇出到 `/task_mgr/{run_id}` 子树 channel。订阅管理器把这些
    // 事件回灌到 orchestrator.apply_task_data，把 human_action 翻译成内部状态机
    // 动作。每个 run 对应一条动态加进来的 pattern，run 终态后摘掉。
    let subscriptions = RunSubscriptionManager::new(
        KEventClient::new_full(WORKFLOW_SERVICE_NAME, None),
        runs.clone(),
        definitions.clone(),
        orchestrator.clone(),
    );
    subscriptions.start().await;

    let rpc = Arc::new(
        WorkflowRpcHandler::new(definitions, runs, orchestrator)
            .with_subscriptions(subscriptions),
    );
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

#[tokio::main]
async fn main() {
    init_logging("", true);
    if let Err(err) = start_workflow_service().await {
        error!("workflow service start failed: {:?}", err);
    }
}
