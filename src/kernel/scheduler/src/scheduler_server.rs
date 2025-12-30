use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_kit::buckyos_get_unix_timestamp;
use std::net::IpAddr;
use std::result::Result;
use std::sync::Arc;
use cyfs_gateway_lib::{HttpServer, ServerError, ServerResult, StreamInfo, serve_http_by_rpc_handler, server_err, ServerErrorCode};
use server_runner::*;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;

pub const SCHEDULER_SERVICE_MAIN_PORT: u16 = 3400;

#[derive(Clone)]
pub struct SchedulerServer {
    
}

impl SchedulerServer {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl RPCHandler for SchedulerServer {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
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
        return Err(server_err!(ServerErrorCode::BadRequest, "Method not allowed"));
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