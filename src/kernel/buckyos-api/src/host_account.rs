use serde::{Deserialize, Serialize};

pub const HOST_ACCOUNT_FILE: &str = "host_account.json";

/// Node-local account selected during activation for host executor access.
#[derive(Serialize, Deserialize)]
#[serde(tag = "schema", deny_unknown_fields)]
pub enum HostAccountFile {
    #[serde(rename = "buckyos.host_account.v1")]
    V1 { account: String },
}
