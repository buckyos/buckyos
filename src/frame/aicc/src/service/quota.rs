use super::*;

pub(crate) struct RoutingQuotaQueryPort {
    factory: Arc<QuotaSourceFactory>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QuotaTruthRecord {
    pub(super) period_start_ms: i64,
    pub(super) period_end_ms: i64,
    pub(super) max_request_units: Option<u64>,
    pub(super) max_cost: Option<buckyos_api::Money>,
    pub(super) reset_at: String,
}

pub(crate) struct SystemConfigQuotaTruthPort {
    storage: Arc<AiccStorage>,
    runtime: Arc<RuntimeState>,
}

impl SystemConfigQuotaTruthPort {
    pub(crate) fn new(storage: Arc<AiccStorage>, runtime: Arc<RuntimeState>) -> Self {
        Self { storage, runtime }
    }

    async fn client(&self) -> Result<SystemConfigClient, QuotaSourceError> {
        let runtime = get_buckyos_api_runtime().map_err(|_| QuotaSourceError)?;
        let service_url = runtime.get_system_config_url();
        let service_token = runtime.get_session_token().await;
        Ok(SystemConfigClient::new(
            Some(service_url.as_str()),
            Some(service_token.as_str()),
        ))
    }
}

#[async_trait]
impl QuotaTruthPort for SystemConfigQuotaTruthPort {
    async fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
        let provider_observation = match lookup.provider_instance_name.as_deref() {
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
        let capability = lookup
            .capability
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .ok()
            .flatten()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "all".to_string());
        let method = lookup.method.as_deref().unwrap_or("all");
        let provider = lookup
            .provider_instance_name
            .as_deref()
            .unwrap_or("_all_providers");
        let app = lookup.caller.app_id.as_deref().unwrap_or("_all_apps");
        if [
            lookup.caller.tenant_id.as_str(),
            lookup.caller.user_id.as_str(),
            app,
            capability.as_str(),
            method,
            provider,
        ]
        .iter()
        .any(|part| part.is_empty() || part.contains('/') || part.contains(".."))
        {
            return Err(QuotaSourceError);
        }
        let key = format!(
            "services/aicc/quota/{}/{}/{}/{}/{}/{}",
            lookup.caller.tenant_id, lookup.caller.user_id, app, capability, method, provider
        );
        let value = match self.client().await {
            Ok(client) => client.get(&key).await.ok(),
            Err(_) => None,
        };
        let Some(record) = value
            .and_then(|value| serde_json::from_str::<QuotaTruthRecord>(&value.value).ok())
            .filter(|record| validate_quota_record(record).is_ok())
        else {
            return Ok(provider_quota(provider_observation.as_ref()));
        };
        let now_ms = current_time_ms()?;
        if now_ms < record.period_start_ms || now_ms >= record.period_end_ms {
            return Ok(provider_quota(provider_observation.as_ref()));
        }
        let mut usage_request = QueryUsageRequest::new(UsageQueryTimeRange::Explicit {
            start_time_ms: record.period_start_ms,
            end_time_ms: record.period_end_ms,
        });
        usage_request.output_mode = UsageQueryOutputMode::Summary;
        usage_request
            .filters
            .tenant_ids
            .push(lookup.caller.tenant_id.clone());
        usage_request
            .filters
            .user_ids
            .push(lookup.caller.user_id.clone());
        if let Some(app_id) = &lookup.caller.app_id {
            usage_request.filters.caller_app_ids.push(app_id.clone());
        }
        if let Some(capability) = &lookup.capability {
            let capability = serde_json::to_value(capability).map_err(|_| QuotaSourceError)?;
            usage_request
                .filters
                .capabilities
                .push(capability.as_str().ok_or(QuotaSourceError)?.to_string());
        }
        if let Some(method) = &lookup.method {
            usage_request.filters.methods.push(method.clone());
        }
        if let Some(provider) = &lookup.provider_instance_name {
            usage_request
                .filters
                .provider_instance_names
                .push(provider.clone());
        }
        let usage = match self.storage.query_usage(&usage_request, now_ms).await {
            Ok(usage) => usage,
            Err(_) => return Ok(provider_quota(provider_observation.as_ref())),
        };
        Ok(
            combine_quota(record, &usage.total, provider_observation.as_ref())
                .unwrap_or_else(|_| provider_quota(provider_observation.as_ref())),
        )
    }
}

pub(super) fn provider_quota(provider: Option<&ProviderQuotaObservation>) -> QuotaSnapshot {
    let Some(provider) = provider else {
        return QuotaSnapshot {
            state: Some(QuotaState::Unknown),
            remaining_request_units: None,
            remaining_cost: None,
            reset_at: None,
        };
    };
    let state = match provider.state {
        ProviderQuotaObservationState::Normal => QuotaState::Normal,
        ProviderQuotaObservationState::NearLimit => QuotaState::NearLimit,
        ProviderQuotaObservationState::Exhausted => QuotaState::Exhausted,
        ProviderQuotaObservationState::Unsupported | ProviderQuotaObservationState::QueryFailed => {
            return QuotaSnapshot {
                state: Some(QuotaState::Unknown),
                remaining_request_units: None,
                remaining_cost: None,
                reset_at: None,
            };
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

fn validate_quota_record(record: &QuotaTruthRecord) -> Result<(), QuotaSourceError> {
    if record.period_start_ms < 0
        || record.period_end_ms <= record.period_start_ms
        || record.reset_at.trim().is_empty()
        || (record.max_request_units.is_none() && record.max_cost.is_none())
        || record.max_cost.as_ref().is_some_and(|cost| {
            !cost.amount.is_finite() || cost.amount < 0.0 || cost.currency.trim().is_empty()
        })
    {
        return Err(QuotaSourceError);
    }
    Ok(())
}

pub(super) fn combine_quota(
    record: QuotaTruthRecord,
    usage: &buckyos_api::UsageAggregate,
    provider: Option<&ProviderQuotaObservation>,
) -> Result<QuotaSnapshot, QuotaSourceError> {
    let max_cost_amount = record.max_cost.as_ref().map(|cost| cost.amount);
    let budget_units = record
        .max_request_units
        .map(|limit| limit.saturating_sub(usage.consumed_request_units));
    let budget_cost = match record.max_cost {
        Some(limit) => {
            if !usage.finance_complete {
                return Err(QuotaSourceError);
            }
            let consumed = match usage.finance_totals.as_slice() {
                [] => 0.0,
                [subtotal]
                    if subtotal.amount.is_finite()
                        && subtotal.amount >= 0.0
                        && !subtotal.currency.trim().is_empty()
                        && subtotal.currency == limit.currency =>
                {
                    subtotal.amount
                }
                _ => {
                    return Err(QuotaSourceError);
                }
            };
            Some(buckyos_api::Money::new(
                (limit.amount - consumed).max(0.0),
                limit.currency,
            ))
        }
        None => None,
    };
    let mut state = budget_state(
        record.max_request_units,
        budget_units,
        max_cost_amount,
        budget_cost.as_ref().map(|cost| cost.amount),
    );
    let mut remaining_units = budget_units;
    let mut remaining_cost = budget_cost;
    if let Some(provider) = provider {
        state = match provider.state {
            ProviderQuotaObservationState::Normal | ProviderQuotaObservationState::Unsupported => {
                state
            }
            ProviderQuotaObservationState::NearLimit => {
                worst_quota_state(state, QuotaState::NearLimit)
            }
            ProviderQuotaObservationState::Exhausted => {
                worst_quota_state(state, QuotaState::Exhausted)
            }
            ProviderQuotaObservationState::QueryFailed => state,
        };
        remaining_units = minimum_option(remaining_units, provider.remaining_request_units);
        if let Some(provider_cost) = &provider.remaining_cost {
            if !provider_cost.amount.is_finite()
                || provider_cost.amount < 0.0
                || provider_cost.currency.trim().is_empty()
            {
                return Err(QuotaSourceError);
            }
            let provider_cost =
                buckyos_api::Money::new(provider_cost.amount, provider_cost.currency.clone());
            remaining_cost = match remaining_cost {
                Some(budget) if budget.currency == provider_cost.currency => {
                    Some(buckyos_api::Money::new(
                        budget.amount.min(provider_cost.amount),
                        budget.currency,
                    ))
                }
                Some(_) => return Err(QuotaSourceError),
                None => Some(provider_cost),
            };
        }
    }
    if remaining_units == Some(0)
        || remaining_cost
            .as_ref()
            .is_some_and(|cost| cost.amount == 0.0)
    {
        state = QuotaState::Exhausted;
    }
    Ok(QuotaSnapshot {
        state: Some(state),
        remaining_request_units: remaining_units,
        remaining_cost,
        reset_at: Some(record.reset_at),
    })
}

fn minimum_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn budget_state(
    max_units: Option<u64>,
    remaining_units: Option<u64>,
    max_cost: Option<f64>,
    remaining_cost: Option<f64>,
) -> QuotaState {
    if remaining_units == Some(0) || remaining_cost == Some(0.0) {
        return QuotaState::Exhausted;
    }
    let units_near = max_units
        .zip(remaining_units)
        .is_some_and(|(limit, remaining)| limit > 0 && remaining.saturating_mul(10) <= limit);
    let cost_near = max_cost
        .zip(remaining_cost)
        .is_some_and(|(limit, remaining)| limit > 0.0 && remaining * 10.0 <= limit);
    if units_near || cost_near {
        QuotaState::NearLimit
    } else {
        QuotaState::Normal
    }
}

fn worst_quota_state(left: QuotaState, right: QuotaState) -> QuotaState {
    match (left, right) {
        (QuotaState::Exhausted, _) | (_, QuotaState::Exhausted) => QuotaState::Exhausted,
        (QuotaState::NearLimit, _) | (_, QuotaState::NearLimit) => QuotaState::NearLimit,
        (QuotaState::Normal, _) | (_, QuotaState::Normal) => QuotaState::Normal,
        _ => QuotaState::Unknown,
    }
}

fn current_time_ms() -> Result<i64, QuotaSourceError> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| QuotaSourceError)?;
    i64::try_from(duration.as_millis()).map_err(|_| QuotaSourceError)
}

impl RoutingQuotaQueryPort {
    pub(crate) fn new(factory: Arc<QuotaSourceFactory>) -> Self {
        Self { factory }
    }
}

#[async_trait]
impl QuotaQueryPort for RoutingQuotaQueryPort {
    async fn query_quota(
        &self,
        caller: &AuthorizedCaller,
        request: QuotaQueryRequest,
    ) -> Result<QuotaQueryResponse, RPCErrors> {
        self.factory
            .query_quota(
                &CallerIdentity {
                    tenant_id: caller.tenant_id.clone(),
                    user_id: caller.user_id.clone(),
                    app_id: caller.app_id.clone(),
                },
                request,
            )
            .await
            .map_err(to_rpc_error)
    }
}
