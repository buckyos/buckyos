use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_kit::buckyos_get_unix_timestamp;
use std::net::IpAddr;
use std::result::Result;
use cyfs_warp::*;

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
impl InnerServiceHandler for SchedulerServer {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse, RPCErrors> {
        unimplemented!()
    }

    async fn handle_http_get(&self, req_path: &str, ip_from: IpAddr) -> Result<String, RPCErrors> {
        return Err(RPCErrors::UnknownMethod(req_path.to_string()));
    }
}