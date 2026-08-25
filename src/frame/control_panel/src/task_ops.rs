use crate::{ControlPanelServer, RpcAuthPrincipal, UserType};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    get_buckyos_api_runtime, ActorRef, AppendAuditEventReq, AuditOutcome, ListAuditEventsReq,
    TaskOutcome, TaskPhase, APP_INSTALL_TASK_SCHEMA_ID, APP_START_TASK_SCHEMA_ID,
    APP_UNINSTALL_TASK_SCHEMA_ID, APP_UPDATE_BATCH_TASK_SCHEMA_ID, APP_UPDATE_TASK_SCHEMA_ID,
};
use serde_json::json;

impl ControlPanelServer {
    pub(crate) async fn append_rpc_audit(
        &self,
        principal: &RpcAuthPrincipal,
        action: &str,
        params: &serde_json::Value,
        trace_id: Option<String>,
        succeeded: bool,
    ) {
        if !Self::is_audited_rpc(action) {
            return;
        }
        let resource = [
            "task_id",
            "bundle_id",
            "app_instance_id",
            "app_name",
            "selector",
            "username",
            "user_id",
        ]
        .iter()
        .find_map(|key| {
            params
                .get(*key)
                .and_then(serde_json::Value::as_str)
                .map(|value| {
                    let kind = match *key {
                        "task_id" => "task",
                        "bundle_id" => "diagnostic",
                        "username" | "user_id" => "user",
                        _ => "app",
                    };
                    format!("{}:{}", kind, value)
                })
        })
        .unwrap_or_else(|| action.to_string());
        let request = AppendAuditEventReq {
            actor: ActorRef::new(
                principal.username.clone(),
                principal.authenticated_app_id.clone(),
            ),
            action: action.to_string(),
            resource,
            trace_id,
            outcome: if succeeded {
                AuditOutcome::Succeeded
            } else {
                AuditOutcome::Failed
            },
            error_code: (!succeeded).then(|| "rpc_error".to_string()),
            details: json!({}),
            redaction_version: crate::redaction::REDACTION_VERSION,
        };
        let result = async {
            get_buckyos_api_runtime()?
                .get_task_mgr_client()
                .await?
                .append_audit_event(request)
                .await
        }
        .await;
        if let Err(error) = result {
            log::warn!("append audit event for {} failed: {}", action, error);
        }
    }

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

    pub(crate) async fn handle_audit_query(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let scope = Self::param_str(&req, "scope").unwrap_or_else(|| "own".to_string());
        if scope != "own" && scope != "zone" {
            return Err(RPCErrors::ParseRequestError(
                "audit scope must be own or zone".to_string(),
            ));
        }
        let requested_actor = Self::param_str(&req, "actor");
        if scope == "zone" && !Self::is_privileged(principal) {
            return Err(RPCErrors::NoPermission(
                "zone audit scope requires administrator privileges".to_string(),
            ));
        }
        if requested_actor
            .as_deref()
            .is_some_and(|actor| actor != principal.username && !Self::is_privileged(principal))
        {
            return Err(RPCErrors::NoPermission(
                "cross-user audit query requires administrator privileges".to_string(),
            ));
        }
        if scope == "own"
            && requested_actor
                .as_deref()
                .is_some_and(|actor| actor != principal.username)
        {
            return Err(RPCErrors::ParseRequestError(
                "actor outside the current principal requires zone scope".to_string(),
            ));
        }

        let actor_user_id = if scope == "zone" {
            requested_actor
        } else {
            Some(principal.username.clone())
        };
        let page = get_buckyos_api_runtime()?
            .get_task_mgr_client()
            .await?
            .list_audit_events(ListAuditEventsReq {
                actor_user_id,
                actor_app_id: Self::param_str(&req, "actor_app"),
                action: Self::param_str(&req, "action"),
                resource: Self::param_str(&req, "resource"),
                trace_id: Self::param_str(&req, "trace_id"),
                created_after: Self::param_u64(&req, "created_after"),
                created_before: Self::param_u64(&req, "created_before"),
                cursor: Self::param_str(&req, "cursor"),
                limit: Self::param_u64(&req, "limit").map(|value| value.clamp(1, 500) as u32),
            })
            .await?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "items": page.events,
                "next_cursor": page.next_cursor,
            })),
            req.seq,
        ))
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

    fn is_audited_rpc(method: &str) -> bool {
        matches!(
            method,
            "ui.locale.set"
                | "user.create"
                | "user.update"
                | "user.update_contact"
                | "user.profile.set"
                | "user.set_msg_tunnel"
                | "user.remove_msg_tunnel"
                | "user.invite.create"
                | "user.invite.accept"
                | "user.delete"
                | "user.change_password"
                | "user.change_state"
                | "user.change_type"
                | "agent.create"
                | "agent.update"
                | "agent.delete"
                | "agent.profile.set"
                | "agent.set_msg_tunnel"
                | "agent.remove_msg_tunnel"
                | "ai.provider.set"
                | "ai.provider.weight.set"
                | "ai.reload"
                | "ai.model.set"
                | "ai.policy.set"
                | "apps.availability.set"
                | "apps.staging.finalize"
                | "apps.staging.release"
                | "apps.submit"
                | "apps.install"
                | "apps.install.confirm"
                | "apps.install.retry"
                | "apps.install.cancel"
                | "apps.upgrade"
                | "apps.uninstall"
                | "apps.start"
                | "apps.stop"
                | "apps.restart"
                | "app.publish"
                | "task.retry"
                | "system.logs.download"
                | "diagnostic.collect"
                | "diagnostic.export"
                | "container.action"
                | "containers.action"
                | "docker.action"
        )
    }
}
