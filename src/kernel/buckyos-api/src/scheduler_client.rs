use crate::{
    AppDoc, AppInstanceId, AppRegistry, AppType, FunctionObject, InstallError, InstallPlan,
    SelectorType, TaskId, ThunkObject,
};
use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::IpAddr;

pub const SCHEDULER_SERVICE_UNIQUE_ID: &str = "scheduler";
pub const SCHEDULER_SERVICE_SERVICE_NAME: &str = "scheduler";
pub const SCHEDULER_SERVICE_SERVICE_PORT: u16 = 3400;
pub const INSTALL_PLAN_EXECUTION_SCHEMA_VERSION: u32 = 1;

pub const RESOURCE_TYPE_CPU: &str = "cpu";
pub const RESOURCE_TYPE_MEMORY: &str = "memory";
pub const RESOURCE_TYPE_DISK_CACHE: &str = "disk_cache";
pub const RESOURCE_TYPE_UPLOAD: &str = "upload";
pub const RESOURCE_TYPE_DOWNLOAD: &str = "download";
pub const RESOURCE_TYPE_GPU_MEMORY: &str = "gpu_memory";
pub const RESOURCE_TYPE_GPU: &str = "gpu_tflops";
pub const RESOURCE_TYPE_GPU_CORES: &str = "gpu_cores";
pub const RESOURCE_TYPE_STORAGE: &str = RESOURCE_TYPE_GPU_MEMORY;
pub const RESOURCE_TYPE_TEMP: &str = RESOURCE_TYPE_GPU_CORES;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value)
            .map_err(|error| RPCErrors::ParseRequestError(error.to_string()))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerRefreshRbacResponse {
    pub updated: bool,
    pub tx_action_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPlanExecutionKey {
    pub app_instance_id: AppInstanceId,
    pub task_id: TaskId,
    pub plan_fingerprint: String,
}

impl InstallPlanExecutionKey {
    pub fn from_plan(plan: &InstallPlan) -> Self {
        Self {
            app_instance_id: plan.app_instance_id.clone(),
            task_id: plan.task_id.clone(),
            plan_fingerprint: plan.plan_fingerprint.clone(),
        }
    }

    pub fn storage_key(&self) -> String {
        let digest = Sha256::digest(
            format!(
                "{}\n{}\n{}",
                self.app_instance_id, self.task_id, self.plan_fingerprint
            )
            .as_bytes(),
        );
        format!(
            "system/scheduler/install_plan_executions/{}",
            hex_digest(&digest)
        )
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPlanCommitPoint {
    BeforeClaim,
    Claimed,
    DesiredStateCommitted,
    NodeConfigPublished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPlanExecutionState {
    Pending,
    Claimed,
    Committed,
    Scheduled,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPlanExecutionRecord {
    pub schema_version: u32,
    pub key: InstallPlanExecutionKey,
    pub plan: InstallPlan,
    pub state: InstallPlanExecutionState,
    pub commit_point: InstallPlanCommitPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_spec_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<AppRegistry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<InstallError>,
    pub claimed_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerSubmitInstallPlanReq {
    pub plan: InstallPlan,
}

impl SchedulerSubmitInstallPlanReq {
    pub fn new(plan: InstallPlan) -> Self {
        Self { plan }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value)
            .map_err(|error| RPCErrors::ParseRequestError(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerInstallPlanKeyReq {
    pub key: InstallPlanExecutionKey,
}

impl SchedulerInstallPlanKeyReq {
    pub fn new(key: InstallPlanExecutionKey) -> Self {
        Self { key }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value)
            .map_err(|error| RPCErrors::ParseRequestError(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerShortcutMutationPlan {
    pub schema_version: u32,
    pub task_id: TaskId,
    pub shortcut_hostname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_app_instance_id: Option<AppInstanceId>,
    pub plan_fingerprint: String,
}

impl SchedulerShortcutMutationPlan {
    pub fn expected_fingerprint(&self) -> String {
        let material = json!({
            "schema_version": self.schema_version,
            "task_id": self.task_id,
            "shortcut_hostname": self.shortcut_hostname,
            "target_app_instance_id": self.target_app_instance_id,
        });
        let (object_id, _) = ndn_lib::build_named_object_by_json("shortcutfp", &material);
        object_id.to_string()
    }

    pub fn fingerprint_is_valid(&self) -> bool {
        self.plan_fingerprint == self.expected_fingerprint()
    }

    pub fn storage_key(&self) -> String {
        let digest =
            Sha256::digest(format!("{}\n{}", self.task_id, self.plan_fingerprint).as_bytes());
        format!(
            "system/scheduler/shortcut_mutations/{}/{}",
            self.shortcut_hostname,
            hex_digest(&digest)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerShortcutMutationState {
    Claimed,
    Committed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerShortcutMutationRecord {
    pub schema_version: u32,
    pub plan: SchedulerShortcutMutationPlan,
    pub state: SchedulerShortcutMutationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<InstallError>,
    pub claimed_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerMutateShortcutReq {
    pub plan: SchedulerShortcutMutationPlan,
}

impl SchedulerMutateShortcutReq {
    pub fn new(plan: SchedulerShortcutMutationPlan) -> Self {
        Self { plan }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value)
            .map_err(|error| RPCErrors::ParseRequestError(error.to_string()))
    }
}

#[async_trait]
pub trait SchedulerHandler: Send + Sync {
    async fn handle_run_thunk(
        &self,
        request: SchedulerRunThunkRequest,
        ctx: RPCContext,
    ) -> Result<SchedulerRunThunkResponse>;

    async fn handle_refresh_rbac(&self, ctx: RPCContext) -> Result<SchedulerRefreshRbacResponse>;

    async fn handle_submit_install_plan(
        &self,
        plan: InstallPlan,
        ctx: RPCContext,
    ) -> Result<InstallPlanExecutionRecord>;

    async fn handle_get_install_plan_status(
        &self,
        key: InstallPlanExecutionKey,
        ctx: RPCContext,
    ) -> Result<InstallPlanExecutionRecord>;

    async fn handle_cancel_install_plan(
        &self,
        key: InstallPlanExecutionKey,
        ctx: RPCContext,
    ) -> Result<InstallPlanExecutionRecord>;

    async fn handle_retry_install_plan(
        &self,
        key: InstallPlanExecutionKey,
        ctx: RPCContext,
    ) -> Result<InstallPlanExecutionRecord>;

    async fn handle_mutate_shortcut(
        &self,
        plan: SchedulerShortcutMutationPlan,
        ctx: RPCContext,
    ) -> Result<SchedulerShortcutMutationRecord>;
}

pub enum SchedulerClient {
    InProcess(Box<dyn SchedulerHandler>),
    KRPC(Box<kRPC>),
}

impl SchedulerClient {
    pub fn new(rpc_client: kRPC) -> Self {
        Self::KRPC(Box::new(rpc_client))
    }

    pub fn new_in_process(handler: Box<dyn SchedulerHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(client: Box<kRPC>) -> Self {
        Self::KRPC(client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        if let Self::KRPC(client) = self {
            client.set_context(context).await;
        }
    }

    pub async fn run_thunk(
        &self,
        thunk: ThunkObject,
        function_object: FunctionObject,
        task_id: i64,
    ) -> Result<SchedulerRunThunkResponse> {
        let request = SchedulerRunThunkRequest::new(task_id, thunk, function_object);
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_run_thunk(request, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => call_typed(client, "run_thunk", &request).await,
        }
    }

    pub async fn refresh_rbac(&self) -> Result<SchedulerRefreshRbacResponse> {
        match self {
            Self::InProcess(handler) => handler.handle_refresh_rbac(RPCContext::default()).await,
            Self::KRPC(client) => call_typed(client, "refresh_rbac", &json!({})).await,
        }
    }

    pub async fn submit_install_plan(
        &self,
        plan: InstallPlan,
    ) -> Result<InstallPlanExecutionRecord> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_submit_install_plan(plan, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                call_typed(
                    client,
                    "submit_install_plan",
                    &SchedulerSubmitInstallPlanReq::new(plan),
                )
                .await
            }
        }
    }

    pub async fn get_install_plan_status(
        &self,
        key: InstallPlanExecutionKey,
    ) -> Result<InstallPlanExecutionRecord> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_get_install_plan_status(key, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                call_typed(
                    client,
                    "get_install_plan_status",
                    &SchedulerInstallPlanKeyReq::new(key),
                )
                .await
            }
        }
    }

    pub async fn cancel_install_plan(
        &self,
        key: InstallPlanExecutionKey,
    ) -> Result<InstallPlanExecutionRecord> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_cancel_install_plan(key, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                call_typed(
                    client,
                    "cancel_install_plan",
                    &SchedulerInstallPlanKeyReq::new(key),
                )
                .await
            }
        }
    }

    pub async fn retry_install_plan(
        &self,
        key: InstallPlanExecutionKey,
    ) -> Result<InstallPlanExecutionRecord> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_retry_install_plan(key, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                call_typed(
                    client,
                    "retry_install_plan",
                    &SchedulerInstallPlanKeyReq::new(key),
                )
                .await
            }
        }
    }

    pub async fn mutate_shortcut(
        &self,
        plan: SchedulerShortcutMutationPlan,
    ) -> Result<SchedulerShortcutMutationRecord> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_mutate_shortcut(plan, RPCContext::default())
                    .await
            }
            Self::KRPC(client) => {
                call_typed(
                    client,
                    "mutate_shortcut",
                    &SchedulerMutateShortcutReq::new(plan),
                )
                .await
            }
        }
    }
}

async fn call_typed<T: for<'de> Deserialize<'de>>(
    client: &kRPC,
    method: &str,
    request: &impl Serialize,
) -> Result<T> {
    let params =
        serde_json::to_value(request).map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let result = client.call(method, params).await?;
    serde_json::from_value(result).map_err(|error| {
        RPCErrors::ParserResponseError(format!("invalid {method} response: {error}"))
    })
}

pub struct SchedulerServerHandler<T: SchedulerHandler>(pub T);

impl<T: SchedulerHandler> SchedulerServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: SchedulerHandler> RPCHandler for SchedulerServerHandler<T> {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);
        let result = match req.method.as_str() {
            "run_thunk" => {
                let request = SchedulerRunThunkRequest::from_json(req.params)?;
                RPCResult::Success(json!(self.0.handle_run_thunk(request, ctx).await?))
            }
            "refresh_rbac" => RPCResult::Success(json!(self.0.handle_refresh_rbac(ctx).await?)),
            "submit_install_plan" => {
                let request = SchedulerSubmitInstallPlanReq::from_json(req.params)?;
                RPCResult::Success(json!(
                    self.0.handle_submit_install_plan(request.plan, ctx).await?
                ))
            }
            "get_install_plan_status" => {
                let request = SchedulerInstallPlanKeyReq::from_json(req.params)?;
                RPCResult::Success(json!(
                    self.0
                        .handle_get_install_plan_status(request.key, ctx)
                        .await?
                ))
            }
            "cancel_install_plan" => {
                let request = SchedulerInstallPlanKeyReq::from_json(req.params)?;
                RPCResult::Success(json!(
                    self.0.handle_cancel_install_plan(request.key, ctx).await?
                ))
            }
            "retry_install_plan" => {
                let request = SchedulerInstallPlanKeyReq::from_json(req.params)?;
                RPCResult::Success(json!(
                    self.0.handle_retry_install_plan(request.key, ctx).await?
                ))
            }
            "mutate_shortcut" => {
                let request = SchedulerMutateShortcutReq::from_json(req.params)?;
                RPCResult::Success(json!(
                    self.0.handle_mutate_shortcut(request.plan, ctx).await?
                ))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method)),
        };
        Ok(RPCResponse {
            result,
            seq,
            trace_id,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_key_is_exact_plan_idempotency_key() {
        let key = InstallPlanExecutionKey {
            app_instance_id: "demo.example.com@alice".parse().unwrap(),
            task_id: "task-1".into(),
            plan_fingerprint: "planfp:abc".into(),
        };
        let value = serde_json::to_value(&key).unwrap();
        let decoded: InstallPlanExecutionKey = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, key);
    }

    #[test]
    fn test_generate_scheduler_service_doc() {
        assert_eq!(generate_scheduler_service_doc().doc_type.to_string(), "app");
    }
}
