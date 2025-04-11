use std::sync::Arc;
use ::kRPC::*;

pub struct SchedulerClient {
    rpc_client: kRPC,
}

impl SchedulerClient {
    pub fn new(rpc_client: kRPC) -> Self {
        Self { rpc_client }
    }


}

