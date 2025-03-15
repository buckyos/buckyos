use async_trait::async_trait;

use ::kRPC::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
use log::*;

pub struct RepoClient {
    krpc_client: kRPC,
}

impl RepoClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self { krpc_client }
    }

    pub async fn pub_index(&self) -> Result<()> {
        let params = json!({});
        let _result = self.krpc_client.call("pub_index", params).await?;
        Ok(())
    }
}