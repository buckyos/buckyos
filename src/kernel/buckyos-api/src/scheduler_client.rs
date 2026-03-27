use crate::{AppDoc, AppType, SelectorType, ThunkObject};
use ::kRPC::*;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEDULER_SERVICE_UNIQUE_ID: &str = "scheduler";
pub const SCHEDULER_SERVICE_SERVICE_NAME: &str = "scheduler";
pub const SCHEDULER_SERVICE_SERVICE_PORT: u16 = 3220;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerRunThunkRequest {
    pub thunk: ThunkObject,
}

impl SchedulerRunThunkRequest {
    pub fn new(thunk: ThunkObject) -> Self {
        Self { thunk }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerRunThunkStatus {
    Dispatched,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerDispatchReceipt {
    pub node_id: String,
    pub dispatch_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_hint_source: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerRunThunkResponse {
    pub thunk_obj_id: String,
    pub status: SchedulerRunThunkStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<SchedulerDispatchReceipt>,
}

pub struct SchedulerClient {
    rpc_client: kRPC,
}

impl SchedulerClient {
    pub fn new(rpc_client: kRPC) -> Self {
        Self { rpc_client }
    }

    pub async fn run_thunk(&self, thunk: ThunkObject,func_obj: FunctionObject) -> Result<SchedulerRunThunkResponse> {
        let req = SchedulerRunThunkRequest::new(thunk);
        let req_json = serde_json::to_value(&req).map_err(|err| {
            RPCErrors::ReasonError(format!("failed to serialize run_thunk request: {}", err))
        })?;
        let result = self.rpc_client.call("run_thunk", req_json).await?;
        serde_json::from_value(result).map_err(|err| {
            RPCErrors::ParserResponseError(format!(
                "expected SchedulerRunThunkResponse response: {}",
                err
            ))
        })
    }
}

pub fn generate_scheduler_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        SCHEDULER_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Scheduler")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}

mod tests {

    #[test]
    fn test_generate_scheduler_service_doc() {
        use super::generate_scheduler_service_doc;
        let doc = generate_scheduler_service_doc();
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }
}
