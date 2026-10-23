use crate::{
    agent_settings_key, agent_spec_key, user_app_spec_key, AccessDenial, AgentId,
    AgentPermissionMode, AgentSettings, AgentSpec, AppInstanceId, AppServiceSpec, AppType,
    DenyReason, PermissionItem, ServiceState, SystemConfigClient, SystemConfigError, UserSettings,
    APP_PERMISSION_USER_HOME,
};
use ::kRPC::{RPCErrors, Result};

pub async fn check_app_runtime_session(
    client: &SystemConfigClient,
    instance: &AppInstanceId,
) -> Result<AppServiceSpec> {
    let owner_key = format!("users/{}/settings", instance.owner_user_id());
    client.invalidate_cache(&owner_key).await;
    let owner = client
        .get(&owner_key)
        .await
        .map_err(|error| app_runtime_config_error(error, DenyReason::AccountState))?;
    let owner: UserSettings = serde_json::from_str(&owner.value)
        .map_err(|_| app_runtime_denied(DenyReason::AccountState, "app_runtime_owner_invalid"))?;
    let spec_key = user_app_spec_key(instance.owner_user_id(), instance.app_id());
    client.invalidate_cache(&spec_key).await;
    let spec = client
        .get(&spec_key)
        .await
        .map_err(|error| app_runtime_config_error(error, DenyReason::EntryUnavailable))?;
    let spec: AppServiceSpec = serde_json::from_str(&spec.value).map_err(|_| {
        app_runtime_denied(DenyReason::EntryUnavailable, "app_runtime_spec_invalid")
    })?;
    validate_app_runtime_session(instance, &owner, &spec)?;
    check_agent_runtime_session(client, instance)
        .await
        .map_err(|error| match error {
            RPCErrors::NoPermission(_) => {
                app_runtime_denied(DenyReason::EntryUnavailable, "agent_runtime_not_bound")
            }
            error => error,
        })?;
    Ok(spec)
}

fn app_runtime_denied(reason: DenyReason, detail: &str) -> RPCErrors {
    AccessDenial::new(reason)
        .with_detail(detail)
        .into_rpc_error()
}

fn app_runtime_config_error(error: SystemConfigError, reason: DenyReason) -> RPCErrors {
    match error {
        SystemConfigError::KeyNotFound(_) | SystemConfigError::NoPermission(_) => {
            app_runtime_denied(reason, "app_runtime_unavailable")
        }
        error => RPCErrors::ReasonError(format!("read App runtime authorization: {error}")),
    }
}

fn validate_app_runtime_session(
    instance: &AppInstanceId,
    owner: &UserSettings,
    spec: &AppServiceSpec,
) -> Result<()> {
    if owner.user_id != instance.owner_user_id() || !owner.is_active() {
        return Err(app_runtime_denied(
            DenyReason::AccountState,
            "app_runtime_owner_inactive",
        ));
    }
    if spec.app_instance_id != *instance || spec.owner_user_id != instance.owner_user_id() {
        return Err(app_runtime_denied(
            DenyReason::EntryUnavailable,
            "app_runtime_identity_mismatch",
        ));
    }
    if !spec.enable
        || spec.expected_instance_count == 0
        || matches!(
            spec.state,
            ServiceState::Stopped | ServiceState::Stopping | ServiceState::Deleted
        )
    {
        return Err(app_runtime_denied(
            DenyReason::EntryUnavailable,
            "app_runtime_unavailable",
        ));
    }
    spec.to_node_execution_spec().map_err(|_| {
        app_runtime_denied(DenyReason::EntryUnavailable, "app_runtime_spec_invalid")
    })?;
    Ok(())
}

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

/// An App session for an Agent-backed runtime (AppType::Agent, or the target
/// of some AgentSpec binding) requires a valid, enabled Agent bound to it.
/// Ordinary Apps are unaffected.
pub async fn check_agent_runtime_session(
    client: &SystemConfigClient,
    instance: &AppInstanceId,
) -> Result<()> {
    let owner = instance.owner_user_id();
    let entries = match client.list(&format!("users/{owner}/agents")).await {
        Ok(entries) => entries,
        Err(SystemConfigError::KeyNotFound(_)) => Vec::new(),
        Err(error) => {
            return Err(RPCErrors::ReasonError(format!(
                "read Agent bindings: {error}"
            )))
        }
    };
    let mut bound = Vec::new();
    for entry in entries {
        let Ok(agent_id) = AgentId::parse(&entry) else {
            continue;
        };
        let spec_key = agent_spec_key(owner, &agent_id);
        client.invalidate_cache(&spec_key).await;
        let spec = match client.get(&spec_key).await {
            Ok(value) => serde_json::from_str::<AgentSpec>(&value.value).ok(),
            Err(SystemConfigError::KeyNotFound(_)) => None,
            Err(error) => return Err(RPCErrors::ReasonError(format!("read AgentSpec: {error}"))),
        };
        let Some(spec) = spec else { continue };
        if !spec.binding.references_runtime(instance) {
            continue;
        }
        let valid = spec.validate().is_ok() && spec.agent_id == agent_id;
        let settings_key = agent_settings_key(owner, &agent_id);
        client.invalidate_cache(&settings_key).await;
        let enabled = match client.get(&settings_key).await {
            Ok(value) => serde_json::from_str::<AgentSettings>(&value.value)
                .map(|s| s.enabled)
                .unwrap_or(false),
            Err(SystemConfigError::KeyNotFound(_)) => true,
            Err(error) => {
                return Err(RPCErrors::ReasonError(format!(
                    "read AgentSettings: {error}"
                )))
            }
        };
        bound.push(valid && enabled);
    }
    let app_key = user_app_spec_key(owner, instance.app_id());
    client.invalidate_cache(&app_key).await;
    let app_type = match client.get(&app_key).await {
        Ok(value) => serde_json::from_str::<AppServiceSpec>(&value.value)
            .ok()
            .map(|spec| spec.app_doc.get_app_type()),
        Err(SystemConfigError::KeyNotFound(_)) => None,
        Err(error) => return Err(RPCErrors::ReasonError(format!("read App spec: {error}"))),
    };
    agent_runtime_session_allowed(&bound, app_type)
}

fn agent_runtime_session_allowed(bound: &[bool], app_type: Option<AppType>) -> Result<()> {
    if bound.is_empty() && app_type != Some(AppType::Agent) {
        return Ok(());
    }
    if bound.iter().any(|ok| *ok) {
        return Ok(());
    }
    Err(RPCErrors::NoPermission(
        "AgentRuntimeNotBound: no valid enabled Agent binds this App".to_string(),
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

    fn runtime_fixture() -> (UserSettings, AppServiceSpec) {
        let owner = serde_json::from_value(serde_json::json!({
            "user_id": "alice", "type": "user", "password": "", "state": "active",
            "res_pool_id": "default"
        }))
        .unwrap();
        let did = name_lib::DID::from_str("did:bns:alice").unwrap();
        let app_doc = crate::AppDoc::builder(AppType::Web, "notes", "0.1.0", "alice", &did)
            .web_pkg(
                crate::SubPkgDesc::new("all.web.notes.alice.bns.did#0.1.0")
                    .package_meta_object_id(ndn_lib::ObjId::new_by_raw("pkg".into(), vec![1; 32])),
            )
            .build()
            .unwrap();
        let instance = AppInstanceId::from_app_did(app_doc.app_did(), "alice").unwrap();
        let spec = AppServiceSpec {
            app_instance_id: instance.clone(),
            app_did: app_doc.app_did().clone(),
            deployment: crate::DeploymentIdentity {
                app_instance_id: instance,
                task_id: "install".into(),
                app_doc_object_id: ndn_lib::NamedObject::gen_obj_id(&app_doc).0,
                spec_generation: 1,
                pikg_digest: None,
            },
            app_doc,
            app_name: "notes".into(),
            app_host_name: "notes".into(),
            app_index: 1,
            owner_user_id: "alice".into(),
            permission: Vec::new(),
            selected_components: Vec::new(),
            packages: Vec::new(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Running,
            spec_config: crate::ServiceSpecConfig::default(),
        };
        (owner, spec)
    }

    #[test]
    fn app_runtime_reauthorization_rechecks_owner_and_current_installation() {
        let (owner, spec) = runtime_fixture();
        let instance = &spec.app_instance_id;
        assert!(validate_app_runtime_session(instance, &owner, &spec).is_ok());
        for state in [
            crate::UserState::Pending,
            crate::UserState::Suspended("disabled".into()),
            crate::UserState::Deleted,
            crate::UserState::Banned("disabled".into()),
        ] {
            let mut inactive = owner.clone();
            inactive.state = state;
            let error = validate_app_runtime_session(instance, &inactive, &spec).unwrap_err();
            assert_eq!(
                AccessDenial::from_rpc_error(&error).unwrap().reason,
                DenyReason::AccountState
            );
        }
        for state in [
            ServiceState::Stopped,
            ServiceState::Stopping,
            ServiceState::Deleted,
        ] {
            let mut unavailable = spec.clone();
            unavailable.state = state;
            let error = validate_app_runtime_session(instance, &owner, &unavailable).unwrap_err();
            assert_eq!(
                AccessDenial::from_rpc_error(&error).unwrap().reason,
                DenyReason::EntryUnavailable
            );
        }
        let mut disabled = spec.clone();
        disabled.enable = false;
        assert!(validate_app_runtime_session(instance, &owner, &disabled).is_err());
        disabled = spec.clone();
        disabled.expected_instance_count = 0;
        assert!(validate_app_runtime_session(instance, &owner, &disabled).is_err());
        disabled = spec.clone();
        disabled.owner_user_id = "bob".into();
        assert!(validate_app_runtime_session(instance, &owner, &disabled).is_err());
        let other = AppInstanceId::new(instance.app_id().clone(), "bob").unwrap();
        assert!(validate_app_runtime_session(&other, &owner, &spec).is_err());
    }

    #[test]
    fn app_runtime_missing_authorization_is_a_structured_denial() {
        let error = app_runtime_config_error(
            SystemConfigError::KeyNotFound("deleted app".into()),
            DenyReason::EntryUnavailable,
        );
        assert_eq!(
            AccessDenial::from_rpc_error(&error).unwrap().reason,
            DenyReason::EntryUnavailable
        );
        let temporary = app_runtime_config_error(
            SystemConfigError::ReasonError("connection timed out".into()),
            DenyReason::EntryUnavailable,
        );
        assert!(AccessDenial::from_rpc_error(&temporary).is_none());
        assert!(crate::BuckyOSRuntime::is_retryable_error(&temporary));
    }

    #[test]
    fn agent_runtime_sessions_need_an_enabled_binding() {
        assert!(agent_runtime_session_allowed(&[], None).is_ok());
        assert!(agent_runtime_session_allowed(&[], Some(AppType::AppService)).is_ok());
        assert!(agent_runtime_session_allowed(&[], Some(AppType::Agent)).is_err());
        assert!(agent_runtime_session_allowed(&[false], Some(AppType::AppService)).is_err());
        assert!(agent_runtime_session_allowed(&[false, true], Some(AppType::Agent)).is_ok());
    }

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
