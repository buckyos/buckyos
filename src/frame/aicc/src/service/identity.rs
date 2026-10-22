//! Who a call is charged to (PRD §3.1–§3.3): the quota user, the trusted
//! business App and the operator, resolved from the verified session alone.

use super::*;
use buckyos_api::{
    agent_spec_key, current_aicc_on_behalf_of, token_is_external, token_principal_kind, AgentId,
    AgentSpec, AiccAttribution, AiccAttributionMode, AiccRunRef, AuthTarget, SessionScope,
    TokenPrincipalKind, UserSettings, UserState, UserType, AICC_ON_BEHALF_OF_RESOURCE,
    USER_GROUPS_CONFIG_PREFIX,
};
use kRPC::RPCSessionToken;

/// What AICC needs to know about an account of this Zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountFacts {
    pub user_id: String,
    pub user_type: UserType,
    pub active: bool,
    /// System user groups the account belongs to that are still defined.
    pub groups: Vec<String>,
}

#[async_trait]
pub(crate) trait UserDirectory: Send + Sync {
    async fn account(&self, user_id: &str) -> Result<Option<AccountFacts>, RPCErrors>;
    /// Owner account and AgentId of an Agent DID of this Zone.
    async fn agent_owner(&self, agent_did: &str) -> Result<Option<(String, String)>, RPCErrors>;
    async fn accounts(&self) -> Result<Vec<AccountFacts>, RPCErrors>;
    async fn defined_groups(&self) -> Result<BTreeSet<String>, RPCErrors>;
    /// AppIds installed for the user and AgentIds of the user's Agents.
    async fn user_apps(&self, user_id: &str) -> Result<BTreeSet<String>, RPCErrors>;
}

#[derive(Default)]
pub(crate) struct SystemConfigUserDirectory {
    agents: std::sync::Mutex<HashMap<String, (String, String)>>,
}

impl SystemConfigUserDirectory {
    async fn client(&self) -> Result<Arc<SystemConfigClient>, RPCErrors> {
        get_buckyos_api_runtime()?.get_system_config_client().await
    }

    fn facts(settings: UserSettings, defined: &BTreeSet<String>) -> AccountFacts {
        AccountFacts {
            active: matches!(settings.state, UserState::Active),
            groups: settings
                .groups
                .into_iter()
                .filter(|group| defined.contains(group))
                .collect(),
            user_type: settings.user_type,
            user_id: settings.user_id,
        }
    }
}

#[async_trait]
impl UserDirectory for SystemConfigUserDirectory {
    async fn account(&self, user_id: &str) -> Result<Option<AccountFacts>, RPCErrors> {
        if user_id.is_empty() || user_id.contains('/') {
            return Ok(None);
        }
        let client = self.client().await?;
        let settings = match client.get(&format!("users/{user_id}/settings")).await {
            Ok(value) => serde_json::from_str::<UserSettings>(&value.value).map_err(to_rpc_error)?,
            Err(SystemConfigError::KeyNotFound(_)) => return Ok(None),
            Err(error) => return Err(to_rpc_error(error)),
        };
        let defined = self.defined_groups().await?;
        Ok(Some(Self::facts(settings, &defined)))
    }

    async fn agent_owner(&self, agent_did: &str) -> Result<Option<(String, String)>, RPCErrors> {
        if let Some(hit) = self
            .agents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_did)
            .cloned()
        {
            return Ok(Some(hit));
        }
        let Ok(did) = name_lib::DID::from_str(agent_did) else {
            return Ok(None);
        };
        let Ok(agent_id) = AgentId::from_agent_did(&did) else {
            return Ok(None);
        };
        let client = self.client().await?;
        let control_panel = get_buckyos_api_runtime()?.get_control_panel_client().await?;
        let users = match client.list("users").await {
            Ok(users) => users,
            Err(SystemConfigError::KeyNotFound(_)) => Vec::new(),
            Err(error) => return Err(to_rpc_error(error)),
        };
        for user_id in users {
            let Ok(spec) = client.get(&agent_spec_key(&user_id, &agent_id)).await else {
                continue;
            };
            let Ok(spec) = serde_json::from_str::<AgentSpec>(&spec.value) else {
                continue;
            };
            if spec.agent_doc.id != did {
                continue;
            }
            // An Agent belongs to the account it is stored under only when its
            // spec names that account's DID as owner.
            if control_panel.get_user_did(&user_id).await.ok().as_ref() != Some(&spec.agent_doc.owner) {
                continue;
            }
            let hit = (user_id, agent_id.as_str().to_string());
            self.agents
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(agent_did.to_string(), hit.clone());
            return Ok(Some(hit));
        }
        Ok(None)
    }

    async fn accounts(&self) -> Result<Vec<AccountFacts>, RPCErrors> {
        let client = self.client().await?;
        let defined = self.defined_groups().await?;
        let users = match client.list("users").await {
            Ok(users) => users,
            Err(SystemConfigError::KeyNotFound(_)) => Vec::new(),
            Err(error) => return Err(to_rpc_error(error)),
        };
        let mut accounts = Vec::new();
        for user_id in users {
            let Ok(value) = client.get(&format!("users/{user_id}/settings")).await else {
                continue;
            };
            if let Ok(settings) = serde_json::from_str::<UserSettings>(&value.value) {
                accounts.push(Self::facts(settings, &defined));
            }
        }
        Ok(accounts)
    }

    async fn user_apps(&self, user_id: &str) -> Result<BTreeSet<String>, RPCErrors> {
        if user_id.is_empty() || user_id.contains('/') {
            return Ok(BTreeSet::new());
        }
        let client = self.client().await?;
        let mut apps = BTreeSet::new();
        for folder in ["apps", "agents"] {
            match client.list(&format!("users/{user_id}/{folder}")).await {
                Ok(ids) => apps.extend(ids),
                Err(SystemConfigError::KeyNotFound(_)) => {}
                Err(error) => return Err(to_rpc_error(error)),
            }
        }
        Ok(apps)
    }

    async fn defined_groups(&self) -> Result<BTreeSet<String>, RPCErrors> {
        match self.client().await?.list(USER_GROUPS_CONFIG_PREFIX).await {
            Ok(ids) => Ok(ids
                .into_iter()
                .filter(|id| buckyos_api::validate_user_group_id(id).is_ok())
                .collect()),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(BTreeSet::new()),
            Err(error) => Err(to_rpc_error(error)),
        }
    }
}

pub(crate) fn aicc_rpc_error(code: AiccErrorCode, message: impl Into<String>) -> RPCErrors {
    AiccError::new(code, message).to_krpc_error()
}

fn attribution_unavailable(mode: AiccAttributionMode, message: &str) -> RPCErrors {
    let mut error = AiccError::new(AiccErrorCode::AttributionUnavailable, message);
    error.details = serde_json::to_value(buckyos_api::AttributionUnavailableDetails { mode }).ok();
    error.to_krpc_error()
}

/// The trusted business App of a session target: the AppId of an App
/// instance (an AgentId for an Agent runtime) or `system:<service>`.
pub(crate) fn business_app(target: &str) -> String {
    match AuthTarget::from_canonical_key(target) {
        Ok(AuthTarget::App { app_instance_id }) => app_instance_id.app_id().as_str().to_string(),
        Ok(AuthTarget::System { service_id }) => format!("system:{service_id}"),
        Err(_) => target.to_string(),
    }
}

pub(crate) struct RuntimeAuthorizer {
    directory: Arc<dyn UserDirectory>,
}

impl RuntimeAuthorizer {
    pub(crate) fn new(directory: Arc<dyn UserDirectory>) -> Self {
        Self { directory }
    }

    async fn active_account(
        &self,
        user_id: &str,
        mode: AiccAttributionMode,
    ) -> Result<AccountFacts, RPCErrors> {
        match self.directory.account(user_id).await? {
            Some(account) if account.active && account.user_type != UserType::Root => Ok(account),
            _ if mode == AiccAttributionMode::Direct => Err(aicc_rpc_error(
                AiccErrorCode::PolicyDenied,
                "the caller is not an active account of this Zone",
            )),
            _ => Err(attribution_unavailable(
                mode,
                "the AICC user of this run is not an active account of this Zone",
            )),
        }
    }

    async fn resolve(
        &self,
        context: &RPCContext,
        action: &'static str,
        resource: &'static str,
        scope: Option<SessionScope>,
    ) -> Result<AuthorizedCaller, RPCErrors> {
        let token = context
            .token
            .clone()
            .ok_or_else(|| RPCErrors::InvalidToken("session token is required".to_string()))?;
        let mut request = RPCRequest::new("aicc.authorize", Value::Null);
        request.token = Some(token.clone());
        let runtime = get_buckyos_api_runtime()?;
        let (subject, target) = match scope {
            Some(scope) => runtime.enforce_opened(&request, action, resource, scope).await?,
            None => runtime.enforce(&request, action, resource).await?,
        };
        let session = RPCSessionToken::from_string(&token)?;
        let on_behalf_of = current_aicc_on_behalf_of();
        let kind = token_principal_kind(&session)?;
        if on_behalf_of.is_some()
            && !matches!(kind, TokenPrincipalKind::System | TokenPrincipalKind::Device)
        {
            return Err(aicc_rpc_error(
                AiccErrorCode::PolicyDenied,
                "only a system service may call on behalf of a user",
            ));
        }
        let direct = |user_id: String, app_id: String, account: AccountFacts| AuthorizedCaller {
            tenant_id: user_id.clone(),
            user_id: user_id.clone(),
            app_id: Some(app_id),
            token: token.clone(),
            principal: CallerPrincipal::Account {
                user_type: account.user_type,
                groups: account.groups,
                operator_id: Some(user_id),
                caller: target.clone(),
                mode: AiccAttributionMode::Direct,
                run_ref: None,
            },
        };
        match kind {
            TokenPrincipalKind::User if token_is_external(&session) => Err(aicc_rpc_error(
                AiccErrorCode::ResourceNotPermitted,
                "you have no AI resources in this Zone",
            )),
            TokenPrincipalKind::User | TokenPrincipalKind::App => {
                let account = self
                    .active_account(&subject, AiccAttributionMode::Direct)
                    .await?;
                Ok(direct(subject, business_app(&target), account))
            }
            TokenPrincipalKind::Agent => {
                let (owner, agent_id) =
                    self.directory.agent_owner(&subject).await?.ok_or_else(|| {
                        aicc_rpc_error(
                            AiccErrorCode::PolicyDenied,
                            "the Agent does not belong to an account of this Zone",
                        )
                    })?;
                let account = self
                    .active_account(&owner, AiccAttributionMode::Direct)
                    .await?;
                let mut caller = direct(owner, agent_id, account);
                if let CallerPrincipal::Account { operator_id, .. } = &mut caller.principal {
                    *operator_id = None;
                }
                Ok(caller)
            }
            TokenPrincipalKind::System | TokenPrincipalKind::Device => {
                let Some(on_behalf_of) = on_behalf_of else {
                    return Ok(AuthorizedCaller {
                        tenant_id: subject.clone(),
                        user_id: subject,
                        app_id: Some(target.clone()),
                        token,
                        principal: CallerPrincipal::System { caller: target },
                    });
                };
                if runtime
                    .enforce(&request, "write", AICC_ON_BEHALF_OF_RESOURCE)
                    .await
                    .is_err()
                {
                    return Err(attribution_unavailable(
                        on_behalf_of.mode,
                        "the service may not call AICC on behalf of users",
                    ));
                }
                if on_behalf_of.mode == AiccAttributionMode::Direct {
                    return Err(aicc_rpc_error(
                        AiccErrorCode::InvalidRequest,
                        "a delegated call must name its attribution mode",
                    ));
                }
                let account = self
                    .active_account(&on_behalf_of.user_id, on_behalf_of.mode)
                    .await?;
                let app_id = match AuthTarget::from_canonical_key(&target) {
                    Ok(AuthTarget::System { service_id }) => service_id.to_string(),
                    _ => business_app(&target),
                };
                Ok(AuthorizedCaller {
                    tenant_id: on_behalf_of.user_id.clone(),
                    user_id: on_behalf_of.user_id.clone(),
                    app_id: Some(app_id),
                    token,
                    principal: CallerPrincipal::Account {
                        user_type: account.user_type,
                        groups: account.groups,
                        operator_id: on_behalf_of.operator_id,
                        caller: target,
                        mode: on_behalf_of.mode,
                        run_ref: on_behalf_of.run_ref,
                    },
                })
            }
        }
    }
}

#[async_trait]
impl ServiceAuthorizer for RuntimeAuthorizer {
    async fn authorize(
        &self,
        context: &RPCContext,
        action: &'static str,
        resource: &'static str,
    ) -> Result<AuthorizedCaller, RPCErrors> {
        self.resolve(context, action, resource, None).await
    }

    async fn authorize_opened(
        &self,
        context: &RPCContext,
        action: &'static str,
        resource: &'static str,
    ) -> Result<AuthorizedCaller, RPCErrors> {
        self.resolve(context, action, resource, Some(SessionScope::Aicc))
            .await
    }
}

impl AuthorizedCaller {
    /// Admin / Root accounts: may manage quota rules and visibility.
    pub(crate) fn is_admin(&self) -> bool {
        matches!(
            &self.principal,
            CallerPrincipal::Account {
                user_type: UserType::Admin | UserType::Root,
                ..
            }
        )
    }

    pub(crate) fn operator_id(&self) -> Option<&str> {
        match &self.principal {
            CallerPrincipal::Account { operator_id, .. } => operator_id.as_deref(),
            CallerPrincipal::System { .. } => None,
        }
    }

    pub(crate) fn attribution(&self) -> AiccAttribution {
        match &self.principal {
            CallerPrincipal::Account {
                operator_id,
                caller,
                mode,
                run_ref,
                ..
            } => AiccAttribution {
                user_id: self.user_id.clone(),
                app_id: self.app_id.clone(),
                operator_id: operator_id.clone(),
                caller: caller.clone(),
                mode: *mode,
                run_ref: run_ref.clone(),
            },
            CallerPrincipal::System { caller } => AiccAttribution {
                user_id: self.user_id.clone(),
                app_id: self.app_id.clone(),
                operator_id: None,
                caller: caller.clone(),
                mode: AiccAttributionMode::Direct,
                run_ref: None::<AiccRunRef>,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn business_app_drops_the_instance_owner() {
        assert_eq!(
            business_app("app:filebrowser.buckyos.ai@alice"),
            "filebrowser.buckyos.ai"
        );
        assert_eq!(business_app("system:control-panel"), "system:control-panel");
    }
}
