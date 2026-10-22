use super::identity::{aicc_rpc_error, UserDirectory};
use super::*;
use crate::execution::{AdmissionPort, AttemptSettlement, ExecutionRecord};
use crate::quota::{
    attempt_budget, denial_error, mark_resource_unavailable, unavailable_error,
    usage_total_tokens, LONG_TASK_TIMEOUT_MS, RECONCILE_AFTER_MS,
};
use crate::storage::{AdmitOutcome, AttemptAdmission, QuotaWriteError, SettleOutcome};
use crate::visibility::{self, Access, Inventory};
use buckyos_api::{
    quota_methods, AiccAttributionMode, QuotaAccountGetRequest, QuotaAccountGetResponse,
    QuotaKind, QuotaListScope, QuotaObject, QuotaQueryRequest, QuotaQueryResponse,
    QuotaRechargeGetRequest, QuotaRechargeGetResponse, QuotaRechargeRequest,
    QuotaRechargeResponse, QuotaResource, QuotaRule, QuotaRuleCreateRequest,
    QuotaRuleDeleteRequest, QuotaRuleDeleteResponse, QuotaRuleDetail, QuotaRuleDraft,
    QuotaRuleGetRequest, QuotaRuleListRequest, QuotaRuleListResponse, QuotaRulePreviewRequest,
    QuotaRulePreviewResponse, QuotaRuleSource, QuotaRuleUpdateRequest, QuotaRuleView,
    QuotaStatus, VisibilityGetRequest, VisibilityGetResponse,
    VisibilityPreviewRequest, VisibilityPreviewResponse, VisibilityProviderInventory,
    VisibilitySubjectPolicy, VisibilityUpdateRequest,
};

/// Upstream quota of Provider accounts (`quota_state`), used by routing only;
/// unrelated to the AICC token quota.
pub(crate) struct ProviderQuotaTruthPort {
    runtime: Arc<RuntimeState>,
}

impl ProviderQuotaTruthPort {
    pub(crate) fn new(runtime: Arc<RuntimeState>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl QuotaTruthPort for ProviderQuotaTruthPort {
    async fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
        let observation = match lookup.provider_instance_name.as_deref() {
            Some(name) => self
                .runtime
                .capture()
                .await
                .providers
                .quota_observation(name)
                .await
                .ok(),
            None => None,
        };
        Ok(provider_quota(observation.as_ref()))
    }
}

pub(super) fn provider_quota(provider: Option<&ProviderQuotaObservation>) -> QuotaSnapshot {
    let unknown = QuotaSnapshot {
        state: Some(QuotaState::Unknown),
        remaining_request_units: None,
        remaining_cost: None,
        reset_at: None,
    };
    let Some(provider) = provider else {
        return unknown;
    };
    let state = match provider.state {
        ProviderQuotaObservationState::Normal => QuotaState::Normal,
        ProviderQuotaObservationState::NearLimit => QuotaState::NearLimit,
        ProviderQuotaObservationState::Exhausted => QuotaState::Exhausted,
        ProviderQuotaObservationState::Unsupported | ProviderQuotaObservationState::QueryFailed => {
            return unknown;
        }
    };
    let remaining_cost = provider.remaining_cost.as_ref().and_then(|cost| {
        (cost.amount.is_finite() && cost.amount >= 0.0 && !cost.currency.trim().is_empty())
            .then(|| buckyos_api::Money::new(cost.amount, cost.currency.clone()))
    });
    QuotaSnapshot {
        state: Some(state),
        remaining_request_units: provider.remaining_request_units,
        remaining_cost,
        reset_at: None,
    }
}

/// Visibility evaluation and the `quota.*` / `visibility.*` methods.
#[async_trait]
pub(crate) trait QuotaPort: Send + Sync {
    async fn access(
        &self,
        caller: &AuthorizedCaller,
        inventory: &Inventory,
    ) -> Result<Access, RPCErrors>;

    async fn handle(
        &self,
        caller: &AuthorizedCaller,
        method: &str,
        params: Value,
        inventory: &Inventory,
    ) -> Result<Value, RPCErrors>;
}

pub(crate) struct QuotaService {
    storage: Arc<AiccStorage>,
    directory: Arc<dyn UserDirectory>,
}

const MAX_RECHARGE_ID_LEN: usize = 128;
const MAX_NOTE_LEN: usize = 500;
const RECONCILE_INTERVAL: Duration = Duration::from_secs(60);

fn now_i64() -> i64 {
    now_ms().min(i64::MAX as u64) as i64
}

fn parse<T: DeserializeOwned>(params: Value, method: &str) -> Result<T, RPCErrors> {
    serde_json::from_value(params)
        .map_err(|error| RPCErrors::ParseRequestError(format!("invalid {method} request: {error}")))
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, RPCErrors> {
    serde_json::to_value(value).map_err(to_rpc_error)
}

fn invalid(message: &'static str) -> RPCErrors {
    aicc_rpc_error(AiccErrorCode::InvalidRequest, message)
}

fn denied(message: &'static str) -> RPCErrors {
    aicc_rpc_error(AiccErrorCode::PolicyDenied, message)
}

fn storage_unavailable(scope: &str) -> RPCErrors {
    unavailable_error(scope, now_i64()).to_krpc_error()
}

fn write_error(error: QuotaWriteError) -> RPCErrors {
    match error {
        QuotaWriteError::NotFound => invalid("the quota rule or account was not found"),
        QuotaWriteError::VersionConflict { expected, actual } => {
            conflict_error(expected, actual)
        }
        QuotaWriteError::Duplicate(rule_id) => {
            let mut error = AiccError::new(
                AiccErrorCode::InvalidRequest,
                "a rule with the same source, object, resource and period already exists",
            );
            error.details = Some(json!({ "existing_rule_id": rule_id }));
            error.to_krpc_error()
        }
        QuotaWriteError::RechargeConflict => aicc_rpc_error(
            AiccErrorCode::IdempotencyConflict,
            "the recharge id was already used with different parameters",
        ),
        QuotaWriteError::Invalid(message) => invalid(message),
        QuotaWriteError::Storage(error) => {
            log::warn!("quota write failed: {error}");
            storage_unavailable("quota_configuration")
        }
    }
}

fn objects_overlap(left: &QuotaObject, right: &QuotaObject) -> bool {
    match (left, right) {
        (QuotaObject::User { user_id: a }, QuotaObject::User { user_id: b }) => a == b,
        (QuotaObject::User { user_id: a }, QuotaObject::UserApp { user_id: b, .. })
        | (QuotaObject::UserApp { user_id: b, .. }, QuotaObject::User { user_id: a }) => a == b,
        (QuotaObject::App { app_id: a }, QuotaObject::App { app_id: b }) => a == b,
        (QuotaObject::App { app_id: a }, QuotaObject::UserApp { app_id: b, .. })
        | (QuotaObject::UserApp { app_id: b, .. }, QuotaObject::App { app_id: a }) => a == b,
        (
            QuotaObject::UserApp {
                user_id: ua,
                app_id: aa,
            },
            QuotaObject::UserApp {
                user_id: ub,
                app_id: ab,
            },
        ) => ua == ub && aa == ab,
        (QuotaObject::User { .. }, QuotaObject::App { .. })
        | (QuotaObject::App { .. }, QuotaObject::User { .. }) => false,
    }
}

fn resources_overlap(left: &QuotaResource, right: &QuotaResource) -> bool {
    match (left, right) {
        (QuotaResource::Hidden, _) | (_, QuotaResource::Hidden) => false,
        (QuotaResource::All, _) | (_, QuotaResource::All) => true,
        (
            QuotaResource::Provider {
                provider_instance_name: a,
            },
            QuotaResource::Provider {
                provider_instance_name: b,
            },
        ) => a == b,
        (QuotaResource::Model { exact_model: a }, QuotaResource::Model { exact_model: b }) => a == b,
        (QuotaResource::Provider { provider_instance_name }, model @ QuotaResource::Model { .. })
        | (model @ QuotaResource::Model { .. }, QuotaResource::Provider { provider_instance_name }) => {
            model.provider_instance_name() == Some(provider_instance_name.as_str())
        }
    }
}

impl QuotaService {
    pub(crate) fn new(storage: Arc<AiccStorage>, directory: Arc<dyn UserDirectory>) -> Self {
        Self { storage, directory }
    }

    /// Charge reservations whose usage never arrived, switch due periods and
    /// prune the ledger, once a minute.
    pub(crate) fn spawn_reconciler(self: &Arc<Self>) {
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                let now = now_i64();
                if let Err(error) = service.storage.quota_apply_due_switches(now).await {
                    log::warn!("quota period switch failed: {error}");
                }
                match service.storage.quota_reconcile(now).await {
                    Ok(0) => {}
                    Ok(count) => log::info!("quota reconciled {count} attempts at their reservation"),
                    Err(error) => log::warn!("quota reconciliation failed: {error}"),
                }
                tokio::time::sleep(RECONCILE_INTERVAL).await;
            }
        });
    }

    /// Read-only check of one route candidate; `quota_unavailable` when the
    /// ledger cannot be read.
    pub(crate) async fn precheck(
        &self,
        caller: &AuthorizedCaller,
        call: &crate::call::ResolvedProviderCall,
    ) -> Result<(), AiccError> {
        let Some(budget) = attempt_budget(call) else {
            return Ok(());
        };
        let now = now_i64();
        match self
            .storage
            .quota_check(
                &caller.user_id,
                caller.app_id.as_deref(),
                &call.provider_instance_name,
                &call.exact_model,
                budget.total(),
                now,
            )
            .await
        {
            Ok(None) => Ok(()),
            Ok(Some(details)) => Err(denial_error(details)),
            Err(error) => {
                log::warn!("quota check failed: {error}");
                Err(unavailable_error("quota_admission", now))
            }
        }
    }

    async fn subjects(&self, caller: &AuthorizedCaller) -> Option<Vec<String>> {
        match &caller.principal {
            CallerPrincipal::Account {
                user_type, groups, ..
            } => Some(visibility::subjects_for(*user_type, groups)),
            CallerPrincipal::System { .. } => None,
        }
    }

    async fn access_for_subjects(
        &self,
        subjects: Option<&[String]>,
        inventory: &Inventory,
    ) -> Result<Access, RPCErrors> {
        let Some(subjects) = subjects else {
            return Ok(Access::Unrestricted);
        };
        let (_, policy) = self.storage.visibility_policy().await.map_err(|error| {
            log::warn!("visibility policy unavailable: {error}");
            let mut error = AiccError::new(
                AiccErrorCode::InternalError,
                "resource permissions are temporarily unavailable",
            );
            error.retriable = true;
            error.to_krpc_error()
        })?;
        Ok(visibility::access_for(&policy, subjects, inventory))
    }

    /// Admin or a trusted system principal that passed the settings RBAC.
    fn manages(caller: &AuthorizedCaller) -> bool {
        caller.is_admin() || matches!(caller.principal, CallerPrincipal::System { .. })
    }

    fn require_admin(caller: &AuthorizedCaller) -> Result<(), RPCErrors> {
        if Self::manages(caller) {
            Ok(())
        } else {
            Err(denied("this operation requires an administrator"))
        }
    }

    /// Rules a non-admin may see: the ones that constrain them and their own.
    fn concerns(caller: &AuthorizedCaller, rule: &QuotaRule) -> bool {
        match &rule.object {
            QuotaObject::User { user_id } | QuotaObject::UserApp { user_id, .. } => {
                user_id == &caller.user_id
            }
            QuotaObject::App { .. } => rule.source == QuotaRuleSource::Admin,
        }
    }

    fn editable(caller: &AuthorizedCaller, rule: &QuotaRule) -> bool {
        if rule.deleted {
            return false;
        }
        match rule.source {
            QuotaRuleSource::Admin => Self::manages(caller),
            QuotaRuleSource::User => rule.object.user_id() == Some(caller.user_id.as_str()),
        }
    }

    async fn view(
        &self,
        caller: &AuthorizedCaller,
        mut rule: QuotaRule,
        access: &Access,
        inventory: &Inventory,
        now: i64,
    ) -> Result<QuotaRuleView, RPCErrors> {
        let mut state = self
            .storage
            .quota_rule_state(&rule, now)
            .await
            .map_err(|_| storage_unavailable("quota_state"))?;
        let in_inventory = match &rule.resource {
            QuotaResource::All | QuotaResource::Hidden => true,
            QuotaResource::Provider {
                provider_instance_name,
            } => inventory.providers.contains_key(provider_instance_name),
            QuotaResource::Model { exact_model } => inventory.contains_model(exact_model),
        };
        if !in_inventory {
            mark_resource_unavailable(&mut state);
        }
        let hidden = match &rule.resource {
            QuotaResource::Provider {
                provider_instance_name,
            } => !access.allows_provider(provider_instance_name),
            QuotaResource::Model { exact_model } => !access.allows_model(exact_model),
            QuotaResource::All | QuotaResource::Hidden => false,
        };
        if hidden && !Self::manages(caller) {
            rule.resource = QuotaResource::Hidden;
        }
        let editable = Self::editable(caller, &rule);
        Ok(QuotaRuleView {
            rule,
            state,
            editable,
        })
    }

    async fn views(
        &self,
        caller: &AuthorizedCaller,
        rules: Vec<QuotaRule>,
        access: &Access,
        inventory: &Inventory,
    ) -> Result<Vec<QuotaRuleView>, RPCErrors> {
        let now = now_i64();
        let mut views = Vec::with_capacity(rules.len());
        for rule in rules {
            views.push(self.view(caller, rule, access, inventory, now).await?);
        }
        Ok(views)
    }

    async fn rules(&self, include_deleted: bool) -> Result<Vec<QuotaRule>, RPCErrors> {
        self.storage
            .quota_rules(include_deleted)
            .await
            .map_err(|_| storage_unavailable("quota_configuration"))
    }

    /// Apps a user may set their own limit on: installed Apps, their
    /// Agents, AIWorkspace and Apps already seen in their usage.
    async fn user_apps(&self, user_id: &str) -> Result<BTreeSet<String>, RPCErrors> {
        let mut apps = self.directory.user_apps(user_id).await?;
        apps.insert("aiworkspace".to_string());
        apps.extend(
            self.storage
                .usage_apps(user_id)
                .await
                .map_err(|_| storage_unavailable("usage"))?,
        );
        Ok(apps)
    }

    /// Check a draft against the caller's rights; returns its source.
    async fn validate_draft(
        &self,
        caller: &AuthorizedCaller,
        draft: &QuotaRuleDraft,
        access: &Access,
        inventory: &Inventory,
    ) -> Result<QuotaRuleSource, RPCErrors> {
        let source = draft.source.unwrap_or(if Self::manages(caller) {
            QuotaRuleSource::Admin
        } else {
            QuotaRuleSource::User
        });
        let ids = match &draft.object {
            QuotaObject::User { user_id } => vec![user_id],
            QuotaObject::App { app_id } => vec![app_id],
            QuotaObject::UserApp { user_id, app_id } => vec![user_id, app_id],
        };
        if ids
            .iter()
            .any(|id| id.trim().is_empty() || id.contains('\u{1f}') || id.len() > 256)
        {
            return Err(invalid("quota object ids must be non-empty"));
        }
        match &draft.resource {
            QuotaResource::All => {}
            QuotaResource::Provider {
                provider_instance_name,
            } => {
                if !inventory.providers.contains_key(provider_instance_name) {
                    return Err(invalid("the Provider instance is not configured"));
                }
                if !access.allows_provider(provider_instance_name) && !Self::manages(caller) {
                    return Err(aicc_rpc_error(
                        AiccErrorCode::ResourceNotPermitted,
                        "the resource is not permitted",
                    ));
                }
            }
            QuotaResource::Model { exact_model } => {
                if !inventory.contains_model(exact_model) {
                    return Err(invalid("the exact model is not in any Provider inventory"));
                }
                if !access.allows_model(exact_model) && !Self::manages(caller) {
                    return Err(aicc_rpc_error(
                        AiccErrorCode::ResourceNotPermitted,
                        "the resource is not permitted",
                    ));
                }
            }
            QuotaResource::Hidden => return Err(invalid("a rule must name its resource")),
        }
        match draft.kind {
            QuotaKind::Periodic => {
                if draft.period.is_none() || draft.limit_tokens.is_none() {
                    return Err(invalid("a periodic rule needs a period and a token limit"));
                }
                if draft.initial_tokens.is_some() {
                    return Err(invalid("only a total rule has an initial grant"));
                }
            }
            QuotaKind::Total => {
                if draft.period.is_some() || draft.limit_tokens.is_some() {
                    return Err(invalid("a total rule has no period or limit"));
                }
            }
        }
        match source {
            QuotaRuleSource::Admin => {
                Self::require_admin(caller)?;
                if let Some(user_id) = draft.object.user_id() {
                    if self.directory.account(user_id).await?.is_none() {
                        return Err(invalid("the user is not an account of this Zone"));
                    }
                }
            }
            QuotaRuleSource::User => {
                let QuotaObject::UserApp { user_id, app_id } = &draft.object else {
                    return Err(denied("a personal limit applies to one of your own Apps"));
                };
                if user_id != &caller.user_id
                    || !matches!(caller.principal, CallerPrincipal::Account { .. })
                {
                    return Err(denied("a personal limit applies to one of your own Apps"));
                }
                if draft.kind != QuotaKind::Periodic {
                    return Err(denied("only an administrator manages total quota"));
                }
                if !self.user_apps(user_id).await?.contains(app_id) {
                    return Err(denied("the App is not one of yours"));
                }
            }
        }
        Ok(source)
    }

    /// Enabled rules that constrain the same calls as `object` / `resource`.
    async fn constraints(
        &self,
        caller: &AuthorizedCaller,
        object: &QuotaObject,
        resource: &QuotaResource,
        excluding: Option<&str>,
        access: &Access,
        inventory: &Inventory,
    ) -> Result<Vec<QuotaRuleView>, RPCErrors> {
        let manages = Self::manages(caller);
        let rules = self
            .rules(false)
            .await?
            .into_iter()
            .filter(|rule| rule.enabled && Some(rule.rule_id.as_str()) != excluding)
            .filter(|rule| objects_overlap(&rule.object, object))
            .filter(|rule| resources_overlap(&rule.resource, resource))
            .filter(|rule| manages || Self::concerns(caller, rule))
            .collect();
        self.views(caller, rules, access, inventory).await
    }

    async fn query(
        &self,
        caller: &AuthorizedCaller,
        request: QuotaQueryRequest,
        access: &Access,
        inventory: &Inventory,
    ) -> Result<QuotaQueryResponse, RPCErrors> {
        let rules = self
            .rules(false)
            .await?
            .into_iter()
            .filter(|rule| Self::concerns(caller, rule))
            .filter(|rule| match (&request.app_id, rule.object.app_id()) {
                (Some(app), Some(rule_app)) => app == rule_app,
                _ => true,
            })
            .collect();
        let rules = self.views(caller, rules, access, inventory).await?;
        let counted = rules.iter().filter(|view| view.rule.enabled);
        let near_limit_count = counted
            .clone()
            .filter(|view| view.state.status == QuotaStatus::NearLimit)
            .count() as u64;
        let exhausted_count = counted
            .filter(|view| {
                matches!(
                    view.state.status,
                    QuotaStatus::Exhausted | QuotaStatus::Forbidden
                )
            })
            .count() as u64;
        Ok(QuotaQueryResponse {
            user_id: caller.user_id.clone(),
            rules,
            near_limit_count,
            exhausted_count,
        })
    }

    async fn rule_for(
        &self,
        caller: &AuthorizedCaller,
        rule_id: &str,
    ) -> Result<QuotaRule, RPCErrors> {
        let rule = self
            .storage
            .quota_rule(rule_id)
            .await
            .map_err(|_| storage_unavailable("quota_configuration"))?
            .ok_or_else(|| invalid("the quota rule was not found"))?;
        if !Self::manages(caller) && !Self::concerns(caller, &rule) {
            return Err(invalid("the quota rule was not found"));
        }
        Ok(rule)
    }

    async fn detail(
        &self,
        caller: &AuthorizedCaller,
        rule_id: &str,
        access: &Access,
        inventory: &Inventory,
    ) -> Result<QuotaRuleDetail, RPCErrors> {
        let rule = self.rule_for(caller, rule_id).await?;
        let manages = Self::manages(caller);
        let own = rule.object.user_id() == Some(caller.user_id.as_str());
        let account = match rule.account_id.as_deref() {
            Some(account_id) if manages || own => self
                .storage
                .quota_account(account_id)
                .await
                .map_err(|_| storage_unavailable("quota_account"))?,
            _ => None,
        };
        let grants = match &account {
            Some(account) => self
                .storage
                .quota_grants(&account.account_id)
                .await
                .map_err(|_| storage_unavailable("quota_account"))?,
            None => Vec::new(),
        };
        let audit = if manages || rule.source == QuotaRuleSource::User {
            let mut targets = vec![rule.rule_id.clone()];
            if manages {
                targets.extend(account.iter().map(|account| account.account_id.clone()));
            }
            self.storage
                .quota_audit(&targets)
                .await
                .map_err(|_| storage_unavailable("quota_audit"))?
        } else {
            Vec::new()
        };
        let enforced = rule.enabled && !rule.deleted;
        let mut constraints = self
            .constraints(
                caller,
                &rule.object,
                &rule.resource,
                Some(&rule.rule_id),
                access,
                inventory,
            )
            .await?;
        let view = self
            .view(caller, rule, access, inventory, now_i64())
            .await?;
        if view.rule.enabled {
            constraints.insert(0, view.clone());
        }
        let blocking_rule_ids = constraints
            .iter()
            .filter(|constraint| constraint.state.available_tokens == 0)
            .map(|constraint| constraint.rule.rule_id.clone())
            .collect();
        Ok(QuotaRuleDetail {
            rule: view,
            constraints,
            blocking_rule_ids,
            account: account.map(|account| account.view(enforced)),
            grants,
            audit,
        })
    }

    async fn preview(
        &self,
        caller: &AuthorizedCaller,
        draft: QuotaRuleDraft,
        access: &Access,
        inventory: &Inventory,
    ) -> Result<QuotaRulePreviewResponse, RPCErrors> {
        let source = self
            .validate_draft(caller, &draft, access, inventory)
            .await?;
        let now = now_i64();
        let rules = self.rules(false).await?;
        let candidate = QuotaRule {
            rule_id: String::new(),
            source,
            object: draft.object.clone(),
            resource: draft.resource.clone(),
            kind: draft.kind,
            period: draft.period,
            limit_tokens: draft.limit_tokens,
            account_id: None,
            enabled: true,
            deleted: false,
            pending_period: None,
            version: 0,
            created_by: caller.user_id.clone(),
            created_at_ms: now,
            updated_by: caller.user_id.clone(),
            updated_at_ms: now,
        };
        let conflict_rule_id = rules
            .iter()
            .find(|rule| {
                rule.source == source
                    && rule.kind == draft.kind
                    && rule.object == draft.object
                    && rule.resource == draft.resource
                    && (draft.kind == QuotaKind::Total
                        || rule.period == draft.period
                        || rule
                            .pending_period
                            .as_ref()
                            .is_some_and(|pending| Some(pending.period) == draft.period))
            })
            .map(|rule| rule.rule_id.clone());
        let existing_account = if draft.kind == QuotaKind::Total {
            self.storage
                .quota_account_for_scope(&draft.object, &draft.resource)
                .await
                .map_err(|_| storage_unavailable("quota_account"))?
        } else {
            None
        };
        let state = match draft.kind {
            QuotaKind::Periodic => self
                .storage
                .quota_rule_state(&candidate, now)
                .await
                .map_err(|_| storage_unavailable("quota_state"))?,
            QuotaKind::Total => match &existing_account {
                Some(account) => crate::quota::total_state(
                    true,
                    account.granted,
                    account.ledger,
                    account.start_at_ms,
                    account.updated_at_ms,
                ),
                None => crate::quota::total_state(
                    true,
                    draft.initial_tokens.unwrap_or(0),
                    crate::quota::Ledger::default(),
                    now,
                    now,
                ),
            },
        };
        let other_constraints = self
            .constraints(caller, &draft.object, &draft.resource, None, access, inventory)
            .await?;
        Ok(QuotaRulePreviewResponse {
            state,
            existing_account: existing_account.map(|account| account.view(false)),
            conflict_rule_id,
            other_constraints,
        })
    }

    async fn create(
        &self,
        caller: &AuthorizedCaller,
        draft: QuotaRuleDraft,
        access: &Access,
        inventory: &Inventory,
    ) -> Result<QuotaRuleView, RPCErrors> {
        let source = self
            .validate_draft(caller, &draft, access, inventory)
            .await?;
        let rule = self
            .storage
            .create_quota_rule(
                source,
                &draft.object,
                &draft.resource,
                draft.kind,
                draft.period,
                draft.limit_tokens,
                draft.initial_tokens.unwrap_or(0),
                &caller.user_id,
                now_i64(),
            )
            .await
            .map_err(write_error)?;
        self.view(caller, rule, access, inventory, now_i64()).await
    }

    async fn update(
        &self,
        caller: &AuthorizedCaller,
        request: QuotaRuleUpdateRequest,
        access: &Access,
        inventory: &Inventory,
    ) -> Result<QuotaRuleView, RPCErrors> {
        let rule = self.rule_for(caller, &request.rule_id).await?;
        if !Self::editable(caller, &rule) {
            return Err(denied("the quota rule cannot be changed by the caller"));
        }
        let rule = self
            .storage
            .update_quota_rule(
                &request.rule_id,
                request.expected_version,
                request.limit_tokens,
                request.period,
                request.enabled,
                request.cancel_pending_period,
                &caller.user_id,
                now_i64(),
            )
            .await
            .map_err(write_error)?;
        self.view(caller, rule, access, inventory, now_i64()).await
    }

    async fn delete(
        &self,
        caller: &AuthorizedCaller,
        request: QuotaRuleDeleteRequest,
    ) -> Result<QuotaRuleDeleteResponse, RPCErrors> {
        let rule = self.rule_for(caller, &request.rule_id).await?;
        if !Self::editable(caller, &rule) {
            return Err(denied("the quota rule cannot be changed by the caller"));
        }
        self.storage
            .delete_quota_rule(
                &request.rule_id,
                request.expected_version,
                &caller.user_id,
                now_i64(),
            )
            .await
            .map_err(write_error)?;
        Ok(QuotaRuleDeleteResponse {
            ok: true,
            rule_id: request.rule_id,
        })
    }

    async fn account_enforced(&self, account_id: &str) -> Result<bool, RPCErrors> {
        Ok(self
            .rules(false)
            .await?
            .iter()
            .any(|rule| rule.enabled && rule.account_id.as_deref() == Some(account_id)))
    }

    async fn account(
        &self,
        caller: &AuthorizedCaller,
        request: QuotaAccountGetRequest,
    ) -> Result<QuotaAccountGetResponse, RPCErrors> {
        let account = self
            .storage
            .quota_account(&request.account_id)
            .await
            .map_err(|_| storage_unavailable("quota_account"))?
            .ok_or_else(|| invalid("the quota account was not found"))?;
        let own = account.object.user_id() == Some(caller.user_id.as_str());
        if !Self::manages(caller) && !own {
            return Err(invalid("the quota account was not found"));
        }
        let grants = self
            .storage
            .quota_grants(&account.account_id)
            .await
            .map_err(|_| storage_unavailable("quota_account"))?;
        let enforced = self.account_enforced(&account.account_id).await?;
        Ok(QuotaAccountGetResponse {
            account: account.view(enforced),
            grants,
        })
    }

    async fn recharge(
        &self,
        caller: &AuthorizedCaller,
        request: QuotaRechargeRequest,
    ) -> Result<QuotaRechargeResponse, RPCErrors> {
        Self::require_admin(caller)?;
        if request.amount_tokens == 0 {
            return Err(invalid("a recharge adds a positive number of tokens"));
        }
        if request.recharge_id.trim().is_empty() || request.recharge_id.len() > MAX_RECHARGE_ID_LEN
        {
            return Err(invalid("the recharge id is invalid"));
        }
        if request
            .note
            .as_ref()
            .is_some_and(|note| note.len() > MAX_NOTE_LEN)
        {
            return Err(invalid("the recharge note is too long"));
        }
        let (recharge, replayed) = self
            .storage
            .recharge_quota_account(
                &request.account_id,
                &request.recharge_id,
                request.amount_tokens,
                request.note.as_deref(),
                &caller.user_id,
                now_i64(),
            )
            .await
            .map_err(write_error)?;
        let account = self
            .storage
            .quota_account(&request.account_id)
            .await
            .map_err(|_| storage_unavailable("quota_account"))?
            .ok_or_else(|| invalid("the quota account was not found"))?;
        let enforced = self.account_enforced(&account.account_id).await?;
        Ok(QuotaRechargeResponse {
            recharge,
            account: account.view(enforced),
            replayed,
        })
    }

    async fn recharge_get(
        &self,
        caller: &AuthorizedCaller,
        request: QuotaRechargeGetRequest,
    ) -> Result<QuotaRechargeGetResponse, RPCErrors> {
        Self::require_admin(caller)?;
        let recharge = self
            .storage
            .quota_grant(&request.recharge_id)
            .await
            .map_err(|_| storage_unavailable("quota_account"))?;
        let account = match &recharge {
            Some(recharge) => {
                let account = self
                    .storage
                    .quota_account(&recharge.account_id)
                    .await
                    .map_err(|_| storage_unavailable("quota_account"))?;
                match account {
                    Some(account) => {
                        let enforced = self.account_enforced(&account.account_id).await?;
                        Some(account.view(enforced))
                    }
                    None => None,
                }
            }
            None => None,
        };
        Ok(QuotaRechargeGetResponse { recharge, account })
    }

    async fn visibility_get(&self, inventory: &Inventory) -> Result<VisibilityGetResponse, RPCErrors> {
        let (revision, subjects) = self
            .storage
            .visibility_policy()
            .await
            .map_err(|_| storage_unavailable("visibility"))?;
        let exposed = visibility::exposed_models(&subjects, inventory);
        let unexposed_models = inventory
            .providers
            .values()
            .flatten()
            .filter(|model| !exposed.contains(*model))
            .cloned()
            .collect();
        Ok(VisibilityGetResponse {
            revision,
            subjects,
            providers: inventory
                .providers
                .iter()
                .map(|(provider, models)| VisibilityProviderInventory {
                    provider_instance_name: provider.clone(),
                    enabled: !inventory.disabled.contains(provider),
                    exact_models: models.iter().cloned().collect(),
                })
                .collect(),
            unexposed_models,
        })
    }

    async fn visibility_update(
        &self,
        caller: &AuthorizedCaller,
        request: VisibilityUpdateRequest,
        inventory: &Inventory,
    ) -> Result<VisibilityGetResponse, RPCErrors> {
        let groups = self.directory.defined_groups().await?;
        visibility::validate_subject(&request.subject, &groups).map_err(invalid)?;
        visibility::validate_entries(&request.entries).map_err(invalid)?;
        self.storage
            .update_visibility(
                request.expected_revision,
                &request.subject,
                &request.entries,
                &caller.user_id,
                now_i64(),
            )
            .await
            .map_err(write_error)?;
        self.visibility_get(inventory).await
    }

    async fn visibility_preview(
        &self,
        caller: &AuthorizedCaller,
        request: VisibilityPreviewRequest,
        inventory: &Inventory,
    ) -> Result<VisibilityPreviewResponse, RPCErrors> {
        let (_, policy) = self
            .storage
            .visibility_policy()
            .await
            .map_err(|_| storage_unavailable("visibility"))?;
        let user_id = request.user_id.unwrap_or_else(|| caller.user_id.clone());
        let account = self
            .directory
            .account(&user_id)
            .await?
            .ok_or_else(|| invalid("the user is not an account of this Zone"))?;
        let subjects = visibility::subjects_for(account.user_type, &account.groups);
        let drafted = |policy: &[VisibilitySubjectPolicy], draft: &buckyos_api::VisibilityDraft| {
            let mut next = policy
                .iter()
                .filter(|item| item.subject != draft.subject)
                .cloned()
                .collect::<Vec<_>>();
            if !draft.entries.is_empty() {
                next.push(VisibilitySubjectPolicy {
                    subject: draft.subject.clone(),
                    entries: draft.entries.clone(),
                    updated_by: caller.user_id.clone(),
                    updated_at_ms: now_i64(),
                });
            }
            next
        };
        let effective = match &request.draft {
            Some(draft) => {
                let groups = self.directory.defined_groups().await?;
                visibility::validate_subject(&draft.subject, &groups).map_err(invalid)?;
                visibility::validate_entries(&draft.entries).map_err(invalid)?;
                drafted(&policy, draft)
            }
            None => policy.clone(),
        };
        let grants = visibility::grant_list(&visibility::grants(&effective, &subjects, inventory));
        let mut response = VisibilityPreviewResponse {
            user_id,
            grants,
            added: Vec::new(),
            removed: Vec::new(),
            affected_users: Vec::new(),
            caller_loses: Vec::new(),
        };
        if let Some(draft) = &request.draft {
            let models = |policy: &[VisibilitySubjectPolicy], subjects: &[String]| {
                visibility::grants(policy, subjects, inventory)
                    .into_keys()
                    .filter_map(|(_, model)| model)
                    .collect::<BTreeSet<_>>()
            };
            let subject = [draft.subject.clone()];
            let before = models(&policy, &subject);
            let after = models(&effective, &subject);
            response.added = after.difference(&before).cloned().collect();
            response.removed = before.difference(&after).cloned().collect();
            response.affected_users = self
                .directory
                .accounts()
                .await?
                .into_iter()
                .filter(|account| account.user_type != buckyos_api::UserType::Root)
                .filter(|account| {
                    visibility::subjects_for(account.user_type, &account.groups)
                        .contains(&draft.subject)
                })
                .map(|account| account.user_id)
                .collect();
            if let Some(caller_subjects) = self.subjects(caller).await {
                let mine_before = models(&policy, &caller_subjects);
                let mine_after = models(&effective, &caller_subjects);
                response.caller_loses = mine_before.difference(&mine_after).cloned().collect();
            }
        }
        Ok(response)
    }
}

#[async_trait]
impl QuotaPort for QuotaService {
    async fn access(
        &self,
        caller: &AuthorizedCaller,
        inventory: &Inventory,
    ) -> Result<Access, RPCErrors> {
        let subjects = self.subjects(caller).await;
        self.access_for_subjects(subjects.as_deref(), inventory)
            .await
    }

    async fn handle(
        &self,
        caller: &AuthorizedCaller,
        method: &str,
        params: Value,
        inventory: &Inventory,
    ) -> Result<Value, RPCErrors> {
        let access = self.access(caller, inventory).await?;
        match method {
            quota_methods::QUERY => to_value(
                self.query(caller, parse(params, method)?, &access, inventory)
                    .await?,
            ),
            quota_methods::RULE_LIST => {
                let request: QuotaRuleListRequest = parse(params, method)?;
                let rules = self.rules(request.include_deleted).await?;
                let rules = match request.scope {
                    QuotaListScope::Manage => {
                        Self::require_admin(caller)?;
                        rules
                    }
                    QuotaListScope::Mine => rules
                        .into_iter()
                        .filter(|rule| Self::concerns(caller, rule))
                        .collect(),
                };
                to_value(QuotaRuleListResponse {
                    rules: self.views(caller, rules, &access, inventory).await?,
                })
            }
            quota_methods::RULE_GET => {
                let request: QuotaRuleGetRequest = parse(params, method)?;
                to_value(
                    self.detail(caller, &request.rule_id, &access, inventory)
                        .await?,
                )
            }
            quota_methods::RULE_PREVIEW => {
                let request: QuotaRulePreviewRequest = parse(params, method)?;
                to_value(
                    self.preview(caller, request.draft, &access, inventory)
                        .await?,
                )
            }
            quota_methods::RULE_CREATE => {
                let request: QuotaRuleCreateRequest = parse(params, method)?;
                to_value(
                    self.create(caller, request.draft, &access, inventory)
                        .await?,
                )
            }
            quota_methods::RULE_UPDATE => to_value(
                self.update(caller, parse(params, method)?, &access, inventory)
                    .await?,
            ),
            quota_methods::RULE_DELETE => {
                to_value(self.delete(caller, parse(params, method)?).await?)
            }
            quota_methods::ACCOUNT_GET => {
                to_value(self.account(caller, parse(params, method)?).await?)
            }
            quota_methods::ACCOUNT_RECHARGE => {
                to_value(self.recharge(caller, parse(params, method)?).await?)
            }
            quota_methods::RECHARGE_GET => {
                to_value(self.recharge_get(caller, parse(params, method)?).await?)
            }
            quota_methods::VISIBILITY_GET => {
                Self::require_admin(caller)?;
                let _: VisibilityGetRequest = parse(params, method)?;
                to_value(self.visibility_get(inventory).await?)
            }
            quota_methods::VISIBILITY_UPDATE => {
                Self::require_admin(caller)?;
                to_value(
                    self.visibility_update(caller, parse(params, method)?, inventory)
                        .await?,
                )
            }
            quota_methods::VISIBILITY_PREVIEW => {
                Self::require_admin(caller)?;
                to_value(
                    self.visibility_preview(caller, parse(params, method)?, inventory)
                        .await?,
                )
            }
            _ => Err(RPCErrors::UnknownMethod(method.to_string())),
        }
    }
}

#[async_trait]
impl AdmissionPort for QuotaService {
    async fn admit(
        &self,
        record: &ExecutionRecord,
        call: &crate::call::ResolvedProviderCall,
        attempt_id: &str,
    ) -> Result<Option<String>, AiccError> {
        let Some(budget) = attempt_budget(call) else {
            return Ok(None);
        };
        let now = now_i64();
        let long_task = call.execution_mode == ExecutionMode::NativeTask;
        let deadline = now
            + RECONCILE_AFTER_MS
            + if long_task { LONG_TASK_TIMEOUT_MS } else { 0 };
        let attribution = record.attribution.as_ref();
        let admission = AttemptAdmission {
            attempt_id: attempt_id.to_string(),
            task_id: record.task_id.clone(),
            user_id: record.user_id.clone(),
            app_id: record.caller_app_id.clone(),
            operator_id: attribution.and_then(|attribution| attribution.operator_id.clone()),
            provider_instance_name: call.provider_instance_name.clone(),
            exact_model: call.exact_model.clone(),
            method: call.method.clone(),
            budget_tokens: budget.total(),
            hard_limit: budget.hard_limit,
            now_ms: now,
            reconcile_deadline_ms: deadline,
            attribution_json: attribution
                .filter(|attribution| attribution.mode != AiccAttributionMode::Direct)
                .and_then(|attribution| serde_json::to_string(attribution).ok()),
        };
        match self.storage.quota_admit(&admission).await {
            Ok(AdmitOutcome::Admitted | AdmitOutcome::Replayed) => Ok(Some(attempt_id.to_string())),
            Ok(AdmitOutcome::Denied(details)) => Err(denial_error(details)),
            Err(error) => {
                log::warn!("quota admission failed for {attempt_id}: {error}");
                Err(unavailable_error("quota_admission", now))
            }
        }
    }

    async fn settle(&self, attempt_id: &str, settlement: AttemptSettlement) {
        let outcome = match settlement {
            AttemptSettlement::Usage(usage, api_type) => match usage_total_tokens(&usage, api_type) {
                Some(tokens) => SettleOutcome::Actual(tokens),
                None => SettleOutcome::Unknown,
            },
            AttemptSettlement::Released => SettleOutcome::Release,
            AttemptSettlement::Unknown => SettleOutcome::Unknown,
        };
        if let Err(error) = self
            .storage
            .quota_settle(attempt_id, outcome, now_i64())
            .await
        {
            log::warn!("quota settlement of {attempt_id} failed ({outcome:?}): {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::identity::AccountFacts;
    use buckyos_api::{QuotaPeriod, RdbBackend, UserType};

    struct Directory {
        accounts: Vec<AccountFacts>,
    }

    #[async_trait]
    impl UserDirectory for Directory {
        async fn account(&self, user_id: &str) -> Result<Option<AccountFacts>, RPCErrors> {
            Ok(self.accounts.iter().find(|account| account.user_id == user_id).cloned())
        }
        async fn agent_owner(&self, _agent_did: &str) -> Result<Option<(String, String)>, RPCErrors> {
            Ok(None)
        }
        async fn accounts(&self) -> Result<Vec<AccountFacts>, RPCErrors> {
            Ok(self.accounts.clone())
        }
        async fn defined_groups(&self) -> Result<BTreeSet<String>, RPCErrors> {
            Ok(BTreeSet::from(["customer".to_string()]))
        }
        async fn user_apps(&self, user_id: &str) -> Result<BTreeSet<String>, RPCErrors> {
            Ok(if user_id == "bob" {
                BTreeSet::from(["writer".to_string()])
            } else {
                BTreeSet::new()
            })
        }
    }

    fn account(user_id: &str, user_type: UserType, groups: &[&str]) -> AccountFacts {
        AccountFacts {
            user_id: user_id.into(),
            user_type,
            active: true,
            groups: groups.iter().map(|group| group.to_string()).collect(),
        }
    }

    fn caller(facts: &AccountFacts) -> AuthorizedCaller {
        AuthorizedCaller {
            tenant_id: facts.user_id.clone(),
            user_id: facts.user_id.clone(),
            app_id: Some("system:control-panel".into()),
            token: "token".into(),
            principal: CallerPrincipal::Account {
                user_type: facts.user_type,
                groups: facts.groups.clone(),
                operator_id: Some(facts.user_id.clone()),
                caller: "system:control-panel".into(),
                mode: AiccAttributionMode::Direct,
                run_ref: None,
            },
        }
    }

    fn inventory() -> Inventory {
        Inventory {
            providers: BTreeMap::from([
                (
                    "cloud".to_string(),
                    BTreeSet::from(["a@cloud".to_string(), "b@cloud".to_string()]),
                ),
                ("other".to_string(), BTreeSet::from(["c@other".to_string()])),
            ]),
            disabled: BTreeSet::new(),
        }
    }

    async fn service() -> (QuotaService, AuthorizedCaller, AuthorizedCaller, AuthorizedCaller) {
        let storage = Arc::new(AiccStorage::open("sqlite::memory:", RdbBackend::Sqlite).await.unwrap());
        let admin = account("alice", UserType::Admin, &[]);
        let user = account("bob", UserType::User, &[]);
        let limited = account("lin", UserType::Limited, &["customer"]);
        let directory = Arc::new(Directory {
            accounts: vec![admin.clone(), user.clone(), limited.clone()],
        });
        (
            QuotaService::new(storage, directory),
            caller(&admin),
            caller(&user),
            caller(&limited),
        )
    }

    async fn call(
        service: &QuotaService,
        caller: &AuthorizedCaller,
        method: &str,
        params: Value,
    ) -> Result<Value, RPCErrors> {
        service.handle(caller, method, params, &inventory()).await
    }

    fn code(error: RPCErrors) -> AiccErrorCode {
        AiccError::from_krpc_error(&error).expect("AICC error").code
    }

    #[tokio::test]
    async fn users_manage_only_their_own_app_limits() {
        // Q03 / Q31.
        let (service, admin, bob, _) = service().await;
        let own = json!({"draft": {"object": {"type": "user_app", "user_id": "bob", "app_id": "writer"},
            "resource": {"type": "all"}, "kind": "periodic", "period": "4h", "limit_tokens": 30000}});
        let created: QuotaRuleView = serde_json::from_value(
            call(&service, &bob, quota_methods::RULE_CREATE, own).await.unwrap(),
        )
        .unwrap();
        assert_eq!(created.rule.source, QuotaRuleSource::User);
        assert!(created.editable);
        let forged = json!({"draft": {"object": {"type": "user_app", "user_id": "alice", "app_id": "writer"},
            "resource": {"type": "all"}, "kind": "periodic", "period": "4h", "limit_tokens": 1}});
        assert_eq!(
            code(call(&service, &bob, quota_methods::RULE_CREATE, forged).await.unwrap_err()),
            AiccErrorCode::PolicyDenied
        );
        let foreign_app = json!({"draft": {"object": {"type": "user_app", "user_id": "bob", "app_id": "not-mine"},
            "resource": {"type": "all"}, "kind": "periodic", "period": "4h", "limit_tokens": 1}});
        assert_eq!(
            code(call(&service, &bob, quota_methods::RULE_CREATE, foreign_app).await.unwrap_err()),
            AiccErrorCode::PolicyDenied
        );
        let admin_rule: QuotaRuleView = serde_json::from_value(
            call(&service, &admin, quota_methods::RULE_CREATE, json!({"draft": {
                "object": {"type": "user", "user_id": "bob"}, "resource": {"type": "all"},
                "kind": "total", "initial_tokens": 1000}}))
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(admin_rule.rule.source, QuotaRuleSource::Admin);
        let mine: QuotaQueryResponse = serde_json::from_value(
            call(&service, &bob, quota_methods::QUERY, json!({})).await.unwrap(),
        )
        .unwrap();
        assert_eq!(mine.rules.len(), 2);
        let admin_view = mine
            .rules
            .iter()
            .find(|view| view.rule.rule_id == admin_rule.rule.rule_id)
            .unwrap();
        assert!(!admin_view.editable);
        assert_eq!(
            code(call(&service, &bob, quota_methods::RULE_DELETE, json!({
                "rule_id": admin_rule.rule.rule_id, "expected_version": admin_rule.rule.version}))
            .await
            .unwrap_err()),
            AiccErrorCode::PolicyDenied
        );
        assert_eq!(
            code(call(&service, &bob, quota_methods::ACCOUNT_RECHARGE, json!({
                "account_id": admin_rule.rule.account_id, "recharge_id": "r1", "amount_tokens": 5}))
            .await
            .unwrap_err()),
            AiccErrorCode::PolicyDenied
        );
        assert_eq!(
            code(call(&service, &bob, quota_methods::RULE_CREATE, json!({"draft": {
                "object": {"type": "user_app", "user_id": "bob", "app_id": "writer"},
                "resource": {"type": "all"}, "kind": "total", "source": "user"}}))
            .await
            .unwrap_err()),
            AiccErrorCode::PolicyDenied
        );
        // Admin rules are not changed through the user's identity, and the
        // admin cannot edit a personal limit either.
        let admin_list: QuotaRuleListResponse = serde_json::from_value(
            call(&service, &admin, quota_methods::RULE_LIST, json!({"scope": "manage"})).await.unwrap(),
        )
        .unwrap();
        assert_eq!(admin_list.rules.len(), 2);
        assert!(admin_list
            .rules
            .iter()
            .any(|view| view.rule.source == QuotaRuleSource::User && !view.editable));
        assert_eq!(
            code(call(&service, &bob, quota_methods::RULE_LIST, json!({"scope": "manage"})).await.unwrap_err()),
            AiccErrorCode::PolicyDenied
        );
    }

    #[tokio::test]
    async fn visibility_follows_groups_and_hides_resources_in_rules() {
        // Q22 / Q23 / Q37 / Q38.
        let (service, admin, bob, lin) = service().await;
        assert_eq!(
            service.access(&lin, &inventory()).await.unwrap(),
            Access::Restricted(BTreeMap::new())
        );
        let open = service.access(&bob, &inventory()).await.unwrap();
        assert!(open.allows_model("c@other"));
        let policy: VisibilityGetResponse = serde_json::from_value(
            call(&service, &admin, quota_methods::VISIBILITY_UPDATE, json!({
                "expected_revision": 1, "subject": "group:customer",
                "entries": [{"type": "provider_models", "provider_instance_name": "cloud", "exact_models": ["a@cloud"]}]}))
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(policy.revision, 2);
        let lin_access = service.access(&lin, &inventory()).await.unwrap();
        assert!(lin_access.allows_model("a@cloud"));
        assert!(!lin_access.allows_model("b@cloud"));
        assert!(!lin_access.allows_provider("other"));
        // Taking type:user away leaves Bob nothing; the preview says so.
        let preview: VisibilityPreviewResponse = serde_json::from_value(
            call(&service, &admin, quota_methods::VISIBILITY_PREVIEW, json!({
                "user_id": "bob", "draft": {"subject": "type:user", "entries": []}}))
            .await
            .unwrap(),
        )
        .unwrap();
        assert!(preview.grants.is_empty());
        assert_eq!(preview.removed.len(), 3);
        assert_eq!(preview.affected_users, ["bob"]);
        assert!(preview.caller_loses.is_empty());
        let self_loss: VisibilityPreviewResponse = serde_json::from_value(
            call(&service, &admin, quota_methods::VISIBILITY_PREVIEW, json!({
                "draft": {"subject": "type:admin", "entries": []}}))
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(self_loss.caller_loses.len(), 3);
        // An admin rule on a model Lin cannot see shows no resource identity.
        call(&service, &admin, quota_methods::RULE_CREATE, json!({"draft": {
            "object": {"type": "user", "user_id": "lin"}, "resource": {"type": "model", "exact_model": "b@cloud"},
            "kind": "periodic", "period": "24h", "limit_tokens": 10}}))
        .await
        .unwrap();
        let mine: QuotaQueryResponse = serde_json::from_value(
            call(&service, &lin, quota_methods::QUERY, json!({})).await.unwrap(),
        )
        .unwrap();
        assert_eq!(mine.rules[0].rule.resource, QuotaResource::Hidden);
        // Only admins read or change the policy.
        assert_eq!(
            code(call(&service, &bob, quota_methods::VISIBILITY_GET, json!({})).await.unwrap_err()),
            AiccErrorCode::PolicyDenied
        );
        assert_eq!(
            code(call(&service, &admin, quota_methods::VISIBILITY_UPDATE, json!({
                "expected_revision": 2, "subject": "group:missing", "entries": []}))
            .await
            .unwrap_err()),
            AiccErrorCode::InvalidRequest
        );
    }

    #[tokio::test]
    async fn rule_detail_lists_every_constraint_and_the_blocking_one() {
        let (service, admin, bob, _) = service().await;
        let create = |object: Value, resource: Value, limit: u64, period: QuotaPeriod| {
            json!({"draft": {"object": object, "resource": resource, "kind": "periodic",
                "period": period, "limit_tokens": limit}})
        };
        let user_rule: QuotaRuleView = serde_json::from_value(
            call(&service, &admin, quota_methods::RULE_CREATE, create(
                json!({"type": "user", "user_id": "bob"}), json!({"type": "provider", "provider_instance_name": "cloud"}),
                0, QuotaPeriod::Day)).await.unwrap(),
        )
        .unwrap();
        let own: QuotaRuleView = serde_json::from_value(
            call(&service, &bob, quota_methods::RULE_CREATE, create(
                json!({"type": "user_app", "user_id": "bob", "app_id": "writer"}), json!({"type": "all"}),
                5_000, QuotaPeriod::FourHours)).await.unwrap(),
        )
        .unwrap();
        let detail: QuotaRuleDetail = serde_json::from_value(
            call(&service, &bob, quota_methods::RULE_GET, json!({"rule_id": own.rule.rule_id})).await.unwrap(),
        )
        .unwrap();
        assert_eq!(detail.constraints.len(), 2);
        assert_eq!(detail.blocking_rule_ids, [user_rule.rule.rule_id.clone()]);
        let duplicate = call(&service, &bob, quota_methods::RULE_CREATE, create(
            json!({"type": "user_app", "user_id": "bob", "app_id": "writer"}), json!({"type": "all"}),
            9, QuotaPeriod::FourHours)).await.unwrap_err();
        assert_eq!(code(duplicate), AiccErrorCode::InvalidRequest);
    }
}
