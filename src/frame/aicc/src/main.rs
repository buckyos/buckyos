mod aicc;
mod aicc_usage_log_db;
mod claude;
mod claude_protocol;
mod complete_request_queue;
mod gimini;
mod minimax;
mod model_registry;
mod model_router;
mod model_scheduler;
mod model_session;
mod model_types;
mod openai;
mod openai_protocol;

use ::kRPC::*;
use anyhow::Result;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, AiccServerHandler,
    BuckyOSRuntimeType, AICC_SERVICE_SERVICE_NAME,
};
use buckyos_kit::init_logging;
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use server_runner::Runner;
use std::net::IpAddr;
use std::sync::Arc;

use crate::aicc::AIComputeCenter;
use crate::aicc_usage_log_db::AiccUsageLogDb;
use crate::claude::register_claude_providers;
use crate::gimini::register_google_gimini_providers;
use crate::minimax::register_minimax_providers;
use crate::openai::register_openai_llm_providers;

const AICC_SERVICE_MAIN_PORT: u16 = 4040;
const METHOD_RELOAD_SETTINGS: &str = "reload_settings";
const METHOD_SERVICE_RELOAD_SETTINGS: &str = "service.reload_settings";
const METHOD_REALOAD_SETTINGS: &str = "reaload_settings";
const METHOD_SERVICE_REALOAD_SETTINGS: &str = "service.reaload_settings";
const REDACTED_SECRET: &str = "***";

struct AiccHttpServer {
    rpc_handler: AiccServerHandler<AIComputeCenter>,
}

fn apply_provider_settings(
    center: &AIComputeCenter,
    settings: &serde_json::Value,
) -> Result<usize> {
    center.registry().clear();
    center.reset_model_routes();

    let mut registered_total = 0usize;
    let mut errors = vec![];

    match register_openai_llm_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("openai: {}", err));
        }
    }

    match register_google_gimini_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("gimini: {}", err));
        }
    }

    match register_claude_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("claude: {}", err));
        }
    }

    match register_minimax_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("minimax: {}", err));
        }
    }

    if !errors.is_empty() {
        warn!(
            "aicc provider registration has errors: registered_total={} errors={}",
            registered_total,
            errors.join(" | ")
        );
    }

    if registered_total == 0 && !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "all provider registrations failed: {}",
            errors.join(" | ")
        ));
    }

    Ok(registered_total)
}

fn redact_settings_for_log(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut next = serde_json::Map::new();
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                if lower == "api_token" || lower == "api_key" || lower == "authorization" {
                    next.insert(
                        k.clone(),
                        serde_json::Value::String(REDACTED_SECRET.to_string()),
                    );
                } else {
                    next.insert(k.clone(), redact_settings_for_log(v));
                }
            }
            serde_json::Value::Object(next)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(redact_settings_for_log)
                .collect::<Vec<_>>(),
        ),
        _ => value.clone(),
    }
}

impl AiccHttpServer {
    fn new(center: AIComputeCenter) -> Self {
        Self {
            rpc_handler: AiccServerHandler::new(center),
        }
    }

    async fn handle_reload_settings(&self) -> std::result::Result<serde_json::Value, RPCErrors> {
        let runtime = get_buckyos_api_runtime()
            .map_err(|err| RPCErrors::ReasonError(format!("get runtime failed: {}", err)))?;
        let settings = match runtime.get_my_settings().await {
            Ok(settings) => settings,
            Err(err) => {
                warn!(
                    "load aicc settings failed during reload, use empty settings: {}",
                    err
                );
                serde_json::json!({})
            }
        };
        let settings_for_log = redact_settings_for_log(&settings);
        info!(
            "aicc.reload_settings current settings: {}",
            settings_for_log
        );

        let registered =
            apply_provider_settings(&self.rpc_handler.0, &settings).map_err(|err| {
                RPCErrors::ReasonError(format!("reload aicc settings failed: {}", err))
            })?;
        Ok(serde_json::json!({
            "ok": true,
            "providers_registered": registered
        }))
    }
}

#[async_trait::async_trait]
impl RPCHandler for AiccHttpServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        if req.method == METHOD_RELOAD_SETTINGS
            || req.method == METHOD_SERVICE_RELOAD_SETTINGS
            || req.method == METHOD_REALOAD_SETTINGS
            || req.method == METHOD_SERVICE_REALOAD_SETTINGS
        {
            let result = self.handle_reload_settings().await?;
            return Ok(RPCResponse {
                result: RPCResult::Success(result),
                seq: req.seq,
                trace_id: req.trace_id,
            });
        }
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

pub async fn start_aicc_service(mut center: AIComputeCenter) -> Result<()> {
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
    let taskmgr = runtime
        .get_task_mgr_client()
        .await
        .map_err(|err| anyhow::anyhow!("init task-manager client for aicc failed: {}", err))?;
    center.set_task_manager_client(Arc::new(taskmgr));

    let settings = match runtime.get_my_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!(
                "load aicc settings failed, fallback to empty settings, err={}",
                err
            );
            serde_json::json!({})
        }
    };
    match apply_provider_settings(&center, &settings) {
        Ok(registered) => {
            info!("aicc providers initialized with {} instances", registered);
        }
        Err(err) => {
            warn!(
                "aicc settings apply failed during startup, continue without providers: {}",
                err
            );
        }
    }

    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow::anyhow!("register aicc runtime failed: {}", err))?;

    match AiccUsageLogDb::open_from_service_spec().await {
        Ok(db) => {
            info!("aicc usage-log db opened");
            center.set_usage_log_db(Arc::new(db));
        }
        Err(err) => {
            warn!(
                "open aicc usage-log db failed, usage events will not be persisted: {}",
                err
            );
        }
    }

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
        start_aicc_service(center).await
    }) {
        error!("aicc service start failed: {:?}", err);
    }
}
