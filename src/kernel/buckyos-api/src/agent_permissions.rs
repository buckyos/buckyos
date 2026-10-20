use crate::{
    agent_spec_key, user_app_spec_key, AgentId, AgentPermissionMode, AgentSpec, AppInstanceId,
    AppServiceSpec, AppType, PermissionItem, SystemConfigClient, SystemConfigError,
    APP_PERMISSION_USER_HOME,
};
use ::kRPC::{RPCErrors, Result};

pub async fn load_agent_permission_mode(
    client: &SystemConfigClient,
    instance: &AppInstanceId,
) -> Result<Option<AgentPermissionMode>> {
    let owner = instance.owner_user_id();
    let root = format!("users/{owner}/agents");
    let entries = client
        .list(&root)
        .await
        .map_err(|error| RPCErrors::ReasonError(format!("read Agent permissions: {error}")))?;
    let mut mode = None;
    for entry in entries {
        let agent_id = AgentId::parse(&entry).map_err(RPCErrors::ReasonError)?;
        let key = agent_spec_key(owner, &agent_id);
        let value = match client.get(&key).await {
            Ok(value) => value,
            Err(SystemConfigError::KeyNotFound(_)) => continue,
            Err(error) => {
                return Err(RPCErrors::ReasonError(format!(
                    "read Agent permissions at {key}: {error}"
                )))
            }
        };
        let spec: AgentSpec = serde_json::from_str(&value.value).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid AgentSpec at {key}: {error}"))
        })?;
        spec.validate().map_err(RPCErrors::ReasonError)?;
        if spec.agent_id != agent_id || spec.binding.target_app_instance_id.owner_user_id() != owner
        {
            return Err(RPCErrors::ReasonError(format!(
                "AgentSpec identity or owner does not match {key}"
            )));
        }
        if spec.binding.references_runtime(instance) {
            mode = merge_permission_mode(mode, spec.permission_mode);
        }
    }
    if mode.is_some() {
        return Ok(mode);
    }
    let key = user_app_spec_key(owner, instance.app_id());
    let value = match client.get(&key).await {
        Ok(value) => value,
        Err(SystemConfigError::KeyNotFound(_)) => return Ok(None),
        Err(error) => {
            return Err(RPCErrors::ReasonError(format!(
                "read Agent runtime permissions at {key}: {error}"
            )))
        }
    };
    let spec: AppServiceSpec = serde_json::from_str(&value.value).map_err(|error| {
        RPCErrors::ReasonError(format!("invalid AppServiceSpec at {key}: {error}"))
    })?;
    if spec.app_instance_id != *instance || spec.owner_user_id != owner {
        return Err(RPCErrors::ReasonError(format!(
            "AppServiceSpec identity or owner does not match {key}"
        )));
    }
    if spec.app_doc.get_app_type() == AppType::Agent && !spec.is_installed() {
        return Ok(Some(AgentPermissionMode::ReadOnly));
    }
    Ok(runtime_permission_mode(
        spec.app_doc.get_app_type(),
        &spec.permission,
    ))
}

fn runtime_permission_mode(
    app_type: AppType,
    permissions: &[PermissionItem],
) -> Option<AgentPermissionMode> {
    if app_type != AppType::Agent {
        return None;
    }
    let mut mode = None;
    for permission in permissions {
        if permission.scope_path == APP_PERMISSION_USER_HOME {
            mode = merge_permission_mode(
                mode,
                if permission.actions.iter().any(|action| action == "write") {
                    AgentPermissionMode::ReadWrite
                } else {
                    AgentPermissionMode::ReadOnly
                },
            );
        }
    }
    Some(mode.unwrap_or(AgentPermissionMode::ReadOnly))
}

fn merge_permission_mode(
    current: Option<AgentPermissionMode>,
    next: AgentPermissionMode,
) -> Option<AgentPermissionMode> {
    Some(match (current, next) {
        (Some(AgentPermissionMode::ReadOnly), _) | (_, AgentPermissionMode::ReadOnly) => {
            AgentPermissionMode::ReadOnly
        }
        _ => AgentPermissionMode::ReadWrite,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_permission_shared_runtime_keeps_most_restrictive_mode() {
        use AgentPermissionMode::{ReadOnly, ReadWrite};
        assert_eq!(merge_permission_mode(None, ReadWrite), Some(ReadWrite));
        assert_eq!(merge_permission_mode(None, ReadOnly), Some(ReadOnly));
        assert_eq!(
            merge_permission_mode(Some(ReadWrite), ReadOnly),
            Some(ReadOnly)
        );
        assert_eq!(
            merge_permission_mode(Some(ReadOnly), ReadWrite),
            Some(ReadOnly)
        );
    }

    #[test]
    fn agent_permission_unbound_runtime_retains_installed_grant() {
        use AgentPermissionMode::{ReadOnly, ReadWrite};
        let read = PermissionItem {
            scope_path: APP_PERMISSION_USER_HOME.to_string(),
            required: true,
            actions: vec!["read".to_string()],
            exp: None,
        };
        let mut write = read.clone();
        write.actions.push("write".to_string());
        assert_eq!(
            runtime_permission_mode(AppType::AppService, &[read.clone()]),
            None
        );
        assert_eq!(runtime_permission_mode(AppType::Agent, &[]), Some(ReadOnly));
        assert_eq!(
            runtime_permission_mode(AppType::Agent, &[read.clone()]),
            Some(ReadOnly)
        );
        assert_eq!(
            runtime_permission_mode(AppType::Agent, &[write.clone()]),
            Some(ReadWrite)
        );
        assert_eq!(
            runtime_permission_mode(AppType::Agent, &[write, read]),
            Some(ReadOnly)
        );
    }
}
