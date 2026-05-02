//! Workflow Service 入口。
//!
//! 这里只搭"服务化"的架子：注册 buckyos-api runtime、启动 server-runner、
//! 把 kRPC 请求路由到 [`WorkflowRpcHandler`]。Definition / Run / Amendment /
//! 事件等具体业务逻辑由 [`workflow`] crate 中已存在的编排器、object store、
//! task tracker 承担；本文件目前只把它们以 method stub 的方式串起来，
//! 留给后续提交逐个把 stub 换成真正的实现。
//!
//! 设计参考 [doc/workflow/workflow service.md](../../doc/workflow/workflow%20service.md) §3 的方法清单。

mod server;

use ::kRPC::*;
use anyhow::Result;
use buckyos_api::{init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType};
use buckyos_kit::init_logging;
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info};
use server_runner::Runner;
use std::sync::Arc;

use crate::server::WorkflowRpcHandler;

/// 服务名 / 端口 / kRPC HTTP path。在迁入 buckyos-api 公共常量之前，先在
/// service 内部声明，避免引入跨 crate 依赖噪音。
pub const WORKFLOW_SERVICE_NAME: &str = "workflow";
pub const WORKFLOW_SERVICE_PORT: u16 = 4070;
pub const WORKFLOW_SERVICE_HTTP_PATH: &str = "/kapi/workflow";

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
    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow::anyhow!("register workflow runtime failed: {}", err))?;

    let rpc = Arc::new(WorkflowRpcHandler::new());
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
    init_logging("workflow_service", true);
    if let Err(err) = start_workflow_service().await {
        error!("workflow service start failed: {:?}", err);
    }
}
