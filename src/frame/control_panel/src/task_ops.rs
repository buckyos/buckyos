use crate::{ControlPanelServer, RpcAuthPrincipal, UserType};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse};
use buckyos_api::{
    get_buckyos_api_runtime, TaskOutcome, TaskPhase, APP_INSTALL_TASK_SCHEMA_ID,
    APP_START_TASK_SCHEMA_ID, APP_UNINSTALL_TASK_SCHEMA_ID, APP_UPDATE_BATCH_TASK_SCHEMA_ID,
    APP_UPDATE_TASK_SCHEMA_ID,
};

impl ControlPanelServer {
    pub(crate) async fn handle_task_retry(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let task_id = Self::require_param_str(&req, "task_id")?;
        Self::require_param_str(&req, "idempotency_key")?;
        let task = get_buckyos_api_runtime()?
            .get_task_mgr_client()
            .await?
            .get_task(&task_id)
            .await?;
        if task.phase != TaskPhase::Terminal || task.outcome != Some(TaskOutcome::Failed) {
            return Err(RPCErrors::ReasonError(format!(
                "TASK_NOT_RETRYABLE: task {} is not a terminal failed task",
                task_id
            )));
        }
        if !Self::is_retryable_task_schema(&task.schema_id) {
            return Err(RPCErrors::ReasonError(format!(
                "TASK_NOT_RETRYABLE: task schema {} does not declare a retry handler",
                task.schema_id
            )));
        }
        if task.creator.user_id != principal.username && !Self::is_privileged(principal) {
            return Err(RPCErrors::NoPermission(
                "only the task owner or an administrator can retry this task".to_string(),
            ));
        }
        self.handle_apps_install_retry(req, Some(principal)).await
    }

    pub(crate) fn is_privileged(principal: &RpcAuthPrincipal) -> bool {
        matches!(principal.user_type, UserType::Admin | UserType::Root)
    }

    fn is_retryable_task_schema(schema_id: &str) -> bool {
        matches!(
            schema_id,
            APP_INSTALL_TASK_SCHEMA_ID
                | APP_UNINSTALL_TASK_SCHEMA_ID
                | APP_START_TASK_SCHEMA_ID
                | APP_UPDATE_TASK_SCHEMA_ID
                | APP_UPDATE_BATCH_TASK_SCHEMA_ID
        )
    }
}
