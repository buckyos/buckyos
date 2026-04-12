use crate::{AppDoc, AppType, FunctionObject, SelectorType, ThunkObject};
use ::kRPC::*;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEDULER_SERVICE_UNIQUE_ID: &str = "scheduler";
pub const SCHEDULER_SERVICE_SERVICE_NAME: &str = "scheduler";
pub const SCHEDULER_SERVICE_SERVICE_PORT: u16 = 3220;

//define the resource type
pub const RESOURCE_TYPE_CPU: &str = "cpu"; //mhz
pub const RESOURCE_TYPE_MEMORY: &str = "memory"; //bytes
pub const RESOURCE_TYPE_DISK_CACHE: &str = "disk_cache"; //bytes
pub const RESOURCE_TYPE_UPLOAD: &str = "upload";
pub const RESOURCE_TYPE_DOWNLOAD: &str = "download";
pub const RESOURCE_TYPE_GPU_MEMORY: &str = "gpu_memory"; //bytes
pub const RESOURCE_TYPE_GPU: &str = "gpu_tflops"; //tflops
pub const RESOURCE_TYPE_GPU_CORES: &str = "gpu_cores";
pub const RESOURCE_TYPE_STORAGE: &str = RESOURCE_TYPE_GPU_MEMORY;
pub const RESOURCE_TYPE_TEMP: &str = RESOURCE_TYPE_GPU_CORES;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerRunThunkRequest {
    pub task_id: i64,
    pub thunk: ThunkObject,
    pub function_object: FunctionObject,
}

impl SchedulerRunThunkRequest {
    pub fn new(task_id: i64, thunk: ThunkObject, function_object: FunctionObject) -> Self {
        Self {
            task_id,
            thunk,
            function_object,
        }
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
    pub task_id: Option<i64>,
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

    pub async fn run_thunk(
        &self,
        thunk: ThunkObject,
        function_object: FunctionObject,
        task_id: i64,
    ) -> Result<SchedulerRunThunkResponse> {
        let req = SchedulerRunThunkRequest::new(task_id, thunk, function_object);
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
