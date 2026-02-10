mod aicc;
mod openai;

use ::kRPC::*;
use anyhow::Result;
use buckyos_api::{
    init_buckyos_api_runtime, set_buckyos_api_runtime, AiccServerHandler, BuckyOSRuntimeType,
    AICC_SERVICE_SERVICE_NAME,
};
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
use std::net::IpAddr;
use std::sync::Arc;

use crate::aicc::AIComputeCenter;
use crate::openai::register_openai_llm_providers;

const AICC_SERVICE_MAIN_PORT: u16 = 4040;

struct AiccHttpServer {
    rpc_handler: AiccServerHandler<AIComputeCenter>,
}

impl AiccHttpServer {
    fn new(center: AIComputeCenter) -> Self {
        Self {
            rpc_handler: AiccServerHandler::new(center),
        }
    }
}

#[async_trait::async_trait]
impl RPCHandler for AiccHttpServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        self.rpc_handler.handle_rpc_call(req, ip_from).await
    }
}

#[async_trait::async_trait]
impl HttpServer for AiccHttpServer {
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
        "aicc".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_aicc_service(center: AIComputeCenter) -> Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        AICC_SERVICE_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    let login_result = runtime.login().await;
    if login_result.is_err() {
        error!(
            "aicc service login to system failed! err:{:?}",
            login_result
        );
        return Err(anyhow::anyhow!(
            "aicc service login to system failed! err:{:?}",
            login_result
        ));
    }
    runtime.set_main_service_port(AICC_SERVICE_MAIN_PORT).await;
    set_buckyos_api_runtime(runtime);

    let server = AiccHttpServer::new(center);

    let runner = Runner::new(AICC_SERVICE_MAIN_PORT);
    if let Err(err) = runner.add_http_server("/kapi/aicc".to_string(), Arc::new(server)) {
        error!("failed to add aicc http server: {:?}", err);
        return Err(anyhow::anyhow!("failed to add aicc http server: {:?}", err));
    }
    if let Err(err) = runner.run().await {
        error!("aicc runner exited with error: {:?}", err);
        return Err(anyhow::anyhow!("aicc runner exited with error: {:?}", err));
    }

    info!("aicc service started at port {}", AICC_SERVICE_MAIN_PORT);
    Ok(())
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    if let Err(err) = rt.block_on(async {
        init_logging("aicc", true);
        let center = AIComputeCenter::default();
        let registered = register_openai_llm_providers(&center)?;
        info!(
            "aicc openai provider initialized with {} instances",
            registered
        );
        start_aicc_service(center).await
    }) {
        error!("aicc service start failed: {:?}", err);
    }
}
