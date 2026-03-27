use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_kit::buckyos_get_unix_timestamp;
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use server_runner::*;
use serde_json::json;
use std::net::IpAddr;
use std::result::Result;
use std::sync::Arc;

use crate::thunk_runner::DefaultThunkRunner;

pub const SCHEDULER_SERVICE_MAIN_PORT: u16 = 3400;

#[derive(Clone)]
pub struct SchedulerServer {
    thunk_runner: Arc<DefaultThunkRunner>,
}

impl SchedulerServer {
    pub fn new() -> Self {
        Self {
            thunk_runner: Arc::new(DefaultThunkRunner::default()),
        }
    }
}

#[async_trait]
impl RPCHandler for SchedulerServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        let result = match req.method.as_str() {
            "run_thunk" => {
                let run_req: SchedulerRunThunkRequest =
                    serde_json::from_value(req.params).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "invalid run_thunk request payload: {}",
                            err
                        ))
                    })?;
                let response = self
                    .thunk_runner
                    .run_thunk(run_req.task_id, run_req.thunk, run_req.function_object)
                    .await
                    .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
                RPCResult::Success(json!(response))
            }
            _ => {
                return Err(RPCErrors::ReasonError(format!(
                    "unknown scheduler method: {}",
                    req.method
                )));
            }
        };

        Ok(RPCResponse::new(result, req.seq))
    }
}

#[async_trait]
impl HttpServer for SchedulerServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        return Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ));
    }

    fn id(&self) -> String {
        "scheduler-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}
