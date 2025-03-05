use ::kRPC::*;
use serde_json::Value;

pub struct VerifyHubClient {
    krpc_client: kRPC,
}

impl VerifyHubClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self { krpc_client }
    }

    pub async fn login(&self, login_params: Option<Value>, login_config: Option<Value>) -> Result<RPCSessionToken> {
        unimplemented!()
    }
}
