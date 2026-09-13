use super::*;

pub(crate) struct RuntimeInferencePort {
    runtime: Arc<RuntimeState>,
    codecs: Arc<CodecRegistry>,
    quota: Arc<QuotaSourceFactory>,
    execution: Arc<ExecutionEngine>,
    storage: Arc<AiccStorage>,
    resource_store: Arc<dyn ResourceStore>,
    url_fetcher: Arc<dyn UrlResourceFetcher>,
    model_health: Arc<ModelHealthRegistry>,
}

impl RuntimeInferencePort {
    pub(crate) fn new(
        runtime: Arc<RuntimeState>,
        codecs: Arc<CodecRegistry>,
        quota: Arc<QuotaSourceFactory>,
        execution: Arc<ExecutionEngine>,
        storage: Arc<AiccStorage>,
        resource_store: Arc<dyn ResourceStore>,
        url_fetcher: Arc<dyn UrlResourceFetcher>,
        model_health: Arc<ModelHealthRegistry>,
    ) -> Self {
        Self {
            runtime,
            codecs,
            quota,
            execution,
            storage,
            resource_store,
            url_fetcher,
            model_health,
        }
    }

    async fn route(
        &self,
        caller: &AuthorizedCaller,
        input: InferenceRouteInput,
    ) -> Result<RoutedInference, RPCErrors> {
        let snapshot = self.runtime.capture().await;
        let caller_identity = CallerIdentity {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            app_id: caller.app_id.clone(),
        };
        let trace_id = input.trace_id.unwrap_or_else(next_inference_id);
        let request_id = input.request_id.unwrap_or_else(next_inference_id);
        let route_models = input
            .session_overlay
            .as_ref()
            .map(|overlay| snapshot.models.with_session_overlay(overlay))
            .transpose()
            .map_err(|error| inference_error(AiccErrorCode::InvalidRequest, error.to_string()))?;
        let models = route_models.as_ref().unwrap_or(snapshot.models.as_ref());
        let provider_names = if input.model.contains('@') {
            vec![crate::model::ExactModelName::parse(&input.model)
                .map_err(|error| inference_error(AiccErrorCode::InvalidRequest, error.to_string()))?
                .provider_instance_name()
                .to_owned()]
        } else {
            models
                .resolve_candidates(&input.model, input.api_type)
                .map_err(|error| {
                    inference_error(AiccErrorCode::NoCandidateModel, error.to_string())
                })?
                .candidates
                .into_iter()
                .map(|candidate| candidate.model.identity.provider_instance_name)
                .collect::<Vec<_>>()
        };
        let quota = self
            .quota
            .prepare_route(
                &caller_identity,
                input.api_type.capability(),
                input.api_type.typed_method(),
                provider_names,
            )
            .await
            .map_err(|_| inference_error(AiccErrorCode::PolicyDenied, "quota scope is invalid"))?;
        let runtime_states = candidate_runtime_states(
            snapshot.as_ref(),
            caller,
            self.model_health.as_ref(),
            input.estimated_input_tokens,
            input.estimated_output_tokens,
        )
        .await;
        let session_overlay = input
            .session_overlay
            .as_ref()
            .or(snapshot.settings.session_config.as_ref());
        let policy = policy_engine_for_route(
            models,
            &input.model,
            session_overlay,
            input.policy.as_ref(),
            quota,
        )
        .map_err(|error| inference_error(AiccErrorCode::PolicyDenied, error.to_string()))?;
        let runtime_failover = policy.policy().runtime_failover.value;
        let mut request = RoutingRequest::new(
            trace_id.clone(),
            request_id.clone(),
            input.model,
            input.api_type,
            caller_identity,
        );
        request.requirements = input.requirements;
        request.disable = input.disable;
        request.estimated_input_tokens = input.estimated_input_tokens;
        request.estimated_output_tokens = input.estimated_output_tokens;
        if let Some(session_id) = input.session_id.as_deref() {
            if session_id.trim().is_empty() || session_id.len() > 512 {
                return Err(inference_error(
                    AiccErrorCode::InvalidRequest,
                    "session_id must contain 1..512 bytes",
                ));
            }
            request.previous_exact_model = self
                .storage
                .session_exact_model(
                    &caller.tenant_id,
                    &caller.user_id,
                    caller.app_id.as_deref(),
                    session_id,
                )
                .await
                .map_err(|error| {
                    inference_error(AiccErrorCode::InternalError, error.to_string())
                })?;
        }
        let decision = Router::new(models, &policy, &runtime_states)
            .route(&request)
            .map_err(|error| inference_error(AiccErrorCode::NoCandidateModel, error.to_string()))?;
        if let Some(session_id) = input.session_id.as_deref() {
            self.storage
                .remember_session_exact_model(
                    &caller.tenant_id,
                    &caller.user_id,
                    caller.app_id.as_deref(),
                    session_id,
                    &decision.selected.exact_model,
                    now_ms() as i64,
                )
                .await
                .map_err(|error| {
                    inference_error(AiccErrorCode::InternalError, error.to_string())
                })?;
        }
        Ok(RoutedInference {
            snapshot,
            decision,
            runtime_failover,
            trace_id,
            request_id,
        })
    }

    async fn lower_call(
        &self,
        snapshot: &crate::runtime::RuntimeSnapshot,
        decision: &RouteDecision,
        call: &AiccCall,
    ) -> Result<ResolvedProviderCall, RPCErrors> {
        let selected = &decision.selected;
        let provider = snapshot
            .providers
            .get(&selected.provider_instance_name)
            .ok_or_else(|| {
                inference_error(
                    AiccErrorCode::NoProviderAvailable,
                    "selected Provider is unavailable",
                )
            })?;
        let credential = provider.resolve_credential().await.map_err(|error| {
            inference_error(AiccErrorCode::NoProviderAvailable, error.to_string())
        })?;
        let provider_rules_id = provider.config.provider_rules_id.clone();
        let transport = HttpTransportConfig {
            request_timeout: provider.config.request_timeout,
            ..HttpTransportConfig::default()
        };
        let pricing = provider
            .inventory
            .models
            .iter()
            .find(|model| {
                model.provider_model_id == selected.provider_model_id
                    && model.model_driver_id == selected.model_driver_id
            })
            .and_then(|model| model.pricing.as_ref())
            .map(|pricing| ResolvedPricing {
                source: match pricing.source {
                    crate::provider::PricingSource::Discovery => CallPricingSource::Discovery,
                    crate::provider::PricingSource::ProviderRules => {
                        CallPricingSource::ProviderRules
                    }
                    crate::provider::PricingSource::ModelDriver => CallPricingSource::ModelDriver,
                },
                pricing: Some(pricing.value.clone()),
                matched_amount: None,
                estimated_cost: selected.estimated_cost.clone(),
            });
        let mut match_dimensions = BTreeMap::new();
        for (name, value) in [
            ("region", provider.config.region.as_ref()),
            ("workspace", provider.config.workspace.as_ref()),
            ("account", provider.config.account.as_ref()),
        ] {
            if let Some(value) = value {
                match_dimensions.insert(name.to_string(), Value::String(value.clone()));
            }
        }
        CallResolver::new(snapshot.catalog.as_ref(), self.codecs.as_ref())
            .lower(
                decision,
                call,
                ProviderCallTarget {
                    provider_rules_id,
                    base_url: provider.config.base_url.clone(),
                    credential,
                    credential_reference: provider.config.credential.reference.clone(),
                    credential_header_name: provider.profile.credential.header_name.clone(),
                    limits: CodecLimits {
                        request_timeout: transport.request_timeout,
                        max_request_bytes: transport.max_request_bytes,
                        max_response_bytes: transport.max_response_bytes,
                    },
                    pricing,
                    match_dimensions,
                },
            )
            .map_err(|error| inference_error(error.code(), error.to_string()))
    }

    async fn materialize_resources(
        &self,
        caller: &AuthorizedCaller,
        request_id: &str,
        call: &mut ResolvedProviderCall,
    ) -> Result<(), RPCErrors> {
        let context = ResourceAccessContext::new(
            caller.tenant_id.clone(),
            caller.user_id.clone(),
            request_id.to_string(),
        )
        .map_err(resource_rpc_error)?;
        call.resource_access_context = Some(context.clone());
        if call.resource_requirements.is_empty() {
            return Ok(());
        }
        let manager = ResourceManager::new(
            Arc::new(AuthenticatedResourceAuthorizer {
                tenant_id: caller.tenant_id.clone(),
                caller_id: caller.user_id.clone(),
                storage: self.storage.clone(),
            }),
            self.resource_store.clone(),
            self.url_fetcher.clone(),
            ResourceLimits::default(),
        )
        .map_err(resource_rpc_error)?;
        let resources = call
            .resource_requirements
            .iter()
            .map(|requirement| requirement.resource.clone())
            .collect::<Vec<_>>();
        let inspected = manager
            .inspect(&context, &resources)
            .await
            .map_err(resource_rpc_error)?;
        let materialized = manager
            .materialize_after_provider_selected(&context, &call.exact_model, inspected)
            .await
            .map_err(resource_rpc_error)?;
        for resource in materialized {
            let parts = resource.into_codec_parts().map_err(resource_rpc_error)?;
            call.context.resources.insert(
                parts.key.into_string(),
                CodecMaterializedResource::new(parts.bytes, parts.mime, parts.file_name).map_err(
                    |error| inference_error(AiccErrorCode::ResourceInvalid, error.to_string()),
                )?,
            );
        }
        Ok(())
    }
}

pub(super) struct AuthenticatedResourceAuthorizer {
    pub(super) tenant_id: String,
    pub(super) caller_id: String,
    pub(super) storage: Arc<AiccStorage>,
}

#[async_trait]
impl ResourceAuthorizer for AuthenticatedResourceAuthorizer {
    async fn authorize(
        &self,
        context: &ResourceAccessContext,
        target: &ResourceTarget,
        operation: ResourceAccessOperation,
    ) -> Result<(), ResourceError> {
        if context.tenant_id != self.tenant_id || context.caller_id != self.caller_id {
            return Err(ResourceError::new(
                ResourceFailure::Unauthorized,
                "resource caller scope does not match the authenticated request",
            ));
        }
        if matches!(
            operation,
            ResourceAccessOperation::Inspect | ResourceAccessOperation::ReadContent
        ) {
            if let ResourceTarget::NamedObject { obj_id } = target {
                let owner = self
                    .storage
                    .artifact_tenant(&obj_id.to_string())
                    .await
                    .map_err(|_| {
                        ResourceError::new(
                            ResourceFailure::Unavailable,
                            "artifact ownership lookup failed",
                        )
                    })?;
                if owner.is_some_and(|owner| owner != context.tenant_id) {
                    return Err(ResourceError::new(
                        ResourceFailure::Unauthorized,
                        "AICC artifact belongs to another tenant",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn resource_rpc_error(error: ResourceError) -> RPCErrors {
    error.to_aicc_error().to_krpc_error()
}

#[async_trait]
impl InferencePort for RuntimeInferencePort {
    async fn resolve_route(
        &self,
        caller: &AuthorizedCaller,
        request: RouteResolveRequest,
    ) -> Result<RouteResolveResponse, RPCErrors> {
        let routed = self
            .route(
                caller,
                InferenceRouteInput {
                    trace_id: request.trace_id,
                    request_id: request.request_id,
                    model: request.logical_model,
                    api_type: request.api_type,
                    requirements: request.requirements,
                    disable: request.disable,
                    policy: request.policy,
                    session_overlay: request.session_overlay,
                    session_id: request.session_id,
                    estimated_input_tokens: request.estimated_input_tokens,
                    estimated_output_tokens: request.estimated_output_tokens,
                },
            )
            .await?;
        Ok(route_response(&routed.decision))
    }

    async fn invoke(&self, caller: &AuthorizedCaller, call: AiccCall) -> Result<Value, RPCErrors> {
        let route_input = route_input_for_call(&call)?;
        let request_model = route_input.model.clone();
        let has_explicit_trace_id = call.trace_id().is_some();
        let mut routed = self.route(caller, route_input).await?;
        let exact_call = exact_call_for_route(call, &routed.decision.selected.exact_model)?;
        let canonical_body = call_params(&exact_call)?;
        let idempotency_key = canonical_body
            .get("idempotency_key")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| routed.request_id.clone());
        if !has_explicit_trace_id {
            let digest = Sha256::digest(
                format!(
                    "{}\0{}\0{}",
                    caller.tenant_id,
                    exact_call.method(),
                    idempotency_key
                )
                .as_bytes(),
            );
            routed.trace_id = format!(
                "aicc-idem-{}",
                digest[..16]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
        }
        let parent_task_id = canonical_body
            .pointer("/task_options/parent_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut primary = self
            .lower_call(routed.snapshot.as_ref(), &routed.decision, &exact_call)
            .await?;
        self.materialize_resources(caller, &routed.request_id, &mut primary)
            .await?;
        let mut failover = Vec::new();
        for candidate in &routed.decision.fallback_candidates {
            let mut fallback_decision = routed.decision.clone();
            fallback_decision.selected = candidate.clone();
            let fallback_call = call_with_exact_model(&exact_call, &candidate.exact_model)?;
            if let Ok(mut call) = self
                .lower_call(routed.snapshot.as_ref(), &fallback_decision, &fallback_call)
                .await
            {
                call.context.resources = primary.context.resources.clone();
                failover.push(call);
            }
        }
        let receipt = self
            .execution
            .execute(crate::execution::ExecutionRequest {
                tenant_id: caller.tenant_id.clone(),
                user_id: caller.user_id.clone(),
                caller_app_id: caller.app_id.clone(),
                trace_id: Some(routed.trace_id.clone()),
                request_model: request_model.clone(),
                idempotency_key,
                canonical_body,
                parent_task_id,
                runtime_generation: routed.snapshot.generation,
                primary,
                failover,
                runtime_failover: routed.runtime_failover,
                now_ms: now_ms(),
                idempotency_window_ms: None,
            })
            .await
            .map_err(|error| error.to_krpc_error())?;
        self.storage
            .write_route_trace(&RouteTraceRecord {
                trace: AiccRouteTraceEvent {
                    trace_id: routed.trace_id.clone(),
                    tenant_id: caller.tenant_id.clone(),
                    caller_app_id: caller.app_id.clone(),
                    task_id: receipt.task_id.clone(),
                    request_model,
                    selected_exact_model: Some(routed.decision.selected.exact_model.clone()),
                    provider_instance_name: Some(
                        routed.decision.selected.provider_instance_name.clone(),
                    ),
                    api_type: routed.decision.trace.api_type.clone(),
                    route_trace_json: public_route_trace(&routed.decision),
                    created_at_ms: now_ms() as i64,
                },
                request_id: Some(routed.request_id.clone()),
                route_id: None,
                provider_trace_id: None,
                scheduler_profile: Some(routed.decision.trace.scheduler_profile.clone()),
                outcome: Some(execution_state_name(receipt.state).to_string()),
            })
            .await
            .map_err(|error| inference_error(AiccErrorCode::InternalError, error.to_string()))?;
        if receipt.provider_task_ref.is_some() && !receipt.state.is_terminal() {
            let execution = Arc::clone(&self.execution);
            let task_id = receipt.task_id.clone();
            let initial_poll_after = receipt.initial_poll_after;
            tokio::spawn(async move {
                let _ = execution
                    .drive_native_after(&task_id, initial_poll_after)
                    .await;
            });
        }
        inference_response(receipt, &routed.decision)
    }
}

static INFERENCE_ID: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_inference_id() -> String {
    format!(
        "aicc-{}-{}",
        now_ms(),
        INFERENCE_ID.fetch_add(1, Ordering::Relaxed)
    )
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn execution_state_name(state: ExecutionState) -> &'static str {
    match state {
        ExecutionState::Submitted => "submitted",
        ExecutionState::Queued => "queued",
        ExecutionState::Running => "running",
        ExecutionState::Succeeded => "succeeded",
        ExecutionState::Failed => "failed",
        ExecutionState::Cancelled => "cancelled",
    }
}

pub(super) fn inference_error(code: AiccErrorCode, message: impl Into<String>) -> RPCErrors {
    AiccError::new(code, message).to_krpc_error()
}

async fn candidate_runtime_states(
    snapshot: &crate::runtime::RuntimeSnapshot,
    caller: &AuthorizedCaller,
    model_health: &ModelHealthRegistry,
    estimated_input_tokens: Option<u64>,
    estimated_output_tokens: Option<u64>,
) -> BTreeMap<String, CandidateRuntimeState> {
    let mut credentials = BTreeMap::new();
    let mut health_states = BTreeMap::new();
    for provider in snapshot.providers.list() {
        let name = provider.config.provider_instance_name.clone();
        credentials.insert(name.clone(), provider.resolve_credential().await.is_ok());
        health_states.insert(name, provider.health().await.state);
    }
    snapshot
        .models
        .model_views()
        .into_iter()
        .map(|model| {
            let settings =
                snapshot.settings.providers.iter().find(|provider| {
                    provider.provider_instance_name == model.provider_instance_name
                });
            let runtime = snapshot.providers.get(&model.provider_instance_name);
            let provider_type = match settings.map(|provider| provider.provider_type) {
                Some(ProviderInstanceType::LocalInference) => ProviderType::LocalInference,
                Some(ProviderInstanceType::CloudApi) => ProviderType::CloudApi,
                Some(ProviderInstanceType::ProxyUnknown) | None => ProviderType::ProxyUnknown,
            };
            let local = provider_type == ProviderType::LocalInference;
            let health = match health_states.get(&model.provider_instance_name) {
                Some(ProviderHealthState::Healthy) => ProviderHealthStatus::Available,
                Some(ProviderHealthState::Degraded) => ProviderHealthStatus::Degraded,
                _ => ProviderHealthStatus::Unavailable,
            };
            let metadata_routable = snapshot
                .provider_metadata
                .get(&model.provider_instance_name)
                .is_none_or(|metadata| metadata.routable);
            let enabled = settings.is_some_and(|provider| provider.enabled) && metadata_routable;
            let observed =
                model_health.snapshot(&model.exact_model, &model.provider_instance_name, now_ms());
            let health = if observed.circuit_open {
                ProviderHealthStatus::CircuitOpen
            } else if health == ProviderHealthStatus::Available && observed.degraded {
                ProviderHealthStatus::Degraded
            } else {
                health
            };
            let state = CandidateRuntimeState {
                enabled,
                credential_available: credentials
                    .get(&model.provider_instance_name)
                    .copied()
                    .unwrap_or(false),
                model_available: runtime.is_some() && observed.model_available,
                health,
                provider_privacy: if local {
                    ProviderPrivacy::Local
                } else {
                    ProviderPrivacy::PublicCloud
                },
                trust: Some(ProviderTrustView {
                    provider_type,
                    provider_type_source: ProviderTypeSource::SystemConfig,
                    provider_type_revision: snapshot.settings_revision.to_string(),
                    asserted_at_ms: 0,
                    trust_level: if local {
                        ProviderTrustLevel::Verified
                    } else {
                        ProviderTrustLevel::Registered
                    },
                }),
                credential_scope: CredentialScope::Tenant {
                    tenant_id: caller.tenant_id.clone(),
                },
                estimated_cost: runtime
                    .as_ref()
                    .and_then(|runtime| {
                        runtime.inventory.models.iter().find(|candidate| {
                            candidate.provider_model_id == model.provider_model_id
                                && candidate.model_driver_id == model.model_driver_id
                        })
                    })
                    .and_then(|model| model.pricing.as_ref())
                    .and_then(|pricing| {
                        estimate_model_cost(
                            &pricing.value,
                            estimated_input_tokens,
                            estimated_output_tokens,
                        )
                    }),
                p50_latency_ms: observed.p50_latency_ms,
                p95_latency_ms: observed.p95_latency_ms,
                error_rate_5m: observed.error_rate_5m,
                recent_failures: observed.recent_failures,
                quality_score: None,
                cache_hit_probability: None,
            };
            (model.exact_model, state)
        })
        .collect()
}

fn estimate_model_cost(
    pricing: &crate::catalog::Pricing,
    estimated_input_tokens: Option<u64>,
    estimated_output_tokens: Option<u64>,
) -> Option<buckyos_api::Money> {
    let amount = if let Some(amount) = pricing.estimated_cost {
        amount
    } else if pricing.input_token.is_some() || pricing.output_token.is_some() {
        let input = pricing.input_token.unwrap_or(0.0) * estimated_input_tokens? as f64;
        let output = pricing.output_token.unwrap_or(0.0) * estimated_output_tokens? as f64;
        input + output
    } else if pricing.unit == Some(crate::catalog::PricingUnit::Request) {
        pricing.amount?
    } else {
        return None;
    };
    (amount.is_finite() && amount >= 0.0 && !pricing.currency.trim().is_empty())
        .then(|| buckyos_api::Money::new(amount, pricing.currency.trim().to_ascii_uppercase()))
}

fn route_input_for_call(call: &AiccCall) -> Result<InferenceRouteInput, RPCErrors> {
    match call {
        AiccCall::HelperLlmChat(request) => Ok(InferenceRouteInput {
            trace_id: request.trace_id.clone(),
            request_id: None,
            model: request.logical_model.clone(),
            api_type: buckyos_api::ApiType::Llm,
            requirements: request.requirements.clone().into(),
            disable: request.disable.clone(),
            policy: request.policy.clone(),
            session_overlay: request.session_overlay.clone(),
            session_id: request.session_id.clone(),
            estimated_input_tokens: None,
            estimated_output_tokens: request.max_output_tokens,
        }),
        AiccCall::HelperTextToImage(request) => Ok(InferenceRouteInput {
            trace_id: request.trace_id.clone(),
            request_id: None,
            model: request.logical_model.clone(),
            api_type: buckyos_api::ApiType::ImageTextToImage,
            requirements: request.requirements.clone().into(),
            disable: request.disable.clone(),
            policy: request.policy.clone(),
            session_overlay: request.session_overlay.clone(),
            session_id: request.session_id.clone(),
            estimated_input_tokens: None,
            estimated_output_tokens: None,
        }),
        AiccCall::RouteResolve(_) => Err(inference_error(
            AiccErrorCode::InvalidMethod,
            "route.resolve is not an inference invocation",
        )),
        _ => {
            let params = call_params(call)?;
            let model = params
                .get("exact_model")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    inference_error(AiccErrorCode::InvalidModelName, "exact_model is missing")
                })?
                .to_string();
            Ok(InferenceRouteInput {
                trace_id: call.trace_id().map(str::to_owned),
                request_id: None,
                model,
                api_type: call.api_type().ok_or_else(|| {
                    inference_error(AiccErrorCode::InvalidMethod, "unsupported inference method")
                })?,
                requirements: Default::default(),
                disable: Default::default(),
                policy: None,
                session_overlay: None,
                session_id: params
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                estimated_input_tokens: None,
                estimated_output_tokens: None,
            })
        }
    }
}

fn exact_call_for_route(call: AiccCall, exact_model: &str) -> Result<AiccCall, RPCErrors> {
    let method = match &call {
        AiccCall::HelperLlmChat(_) => buckyos_api::ai_methods::CHAT_COMPLETIONS_CREATE,
        AiccCall::HelperTextToImage(_) => buckyos_api::ai_methods::IMAGES_GENERATE,
        _ => return Ok(call),
    };
    let mut params = call_params(&call)?;
    let object = params.as_object_mut().ok_or_else(|| {
        inference_error(
            AiccErrorCode::InvalidRequest,
            "helper request must be an object",
        )
    })?;
    for field in [
        "logical_model",
        "requirements",
        "disable",
        "policy",
        "session_overlay",
    ] {
        object.remove(field);
    }
    object.insert("exact_model".into(), Value::String(exact_model.to_string()));
    AiccCall::from_method_and_params(method, params)
}

fn call_with_exact_model(call: &AiccCall, exact_model: &str) -> Result<AiccCall, RPCErrors> {
    let mut params = call_params(call)?;
    params
        .as_object_mut()
        .ok_or_else(|| {
            inference_error(
                AiccErrorCode::InvalidRequest,
                "canonical request must be an object",
            )
        })?
        .insert(
            "exact_model".to_string(),
            Value::String(exact_model.to_string()),
        );
    AiccCall::from_method_and_params(call.method(), params)
}

fn call_params(call: &AiccCall) -> Result<Value, RPCErrors> {
    call.to_params()
        .map_err(|error| inference_error(AiccErrorCode::InvalidRequest, error.to_string()))
}

fn route_response(decision: &RouteDecision) -> RouteResolveResponse {
    let selected = &decision.selected;
    RouteResolveResponse {
        selected_exact_model: selected.exact_model.clone(),
        selected_model_uid: selected.model_uid.clone(),
        provider_instance_name: selected.provider_instance_name.clone(),
        provider_profile_id: selected.provider_profile_id.clone(),
        protocol_adapter_id: selected.protocol_adapter_id.clone(),
        model_driver_id: selected.model_driver_id.clone(),
        provider_driver: None,
        origin_model_id: selected.origin_model_id.clone(),
        provider_model_id: selected.provider_model_id.clone(),
        operation: selected.operation.clone(),
        enabled_capabilities: selected.enabled_capabilities.clone(),
        disabled_capabilities: selected.disabled_capabilities.clone(),
        fallback_attempts: decision
            .fallback_candidates
            .iter()
            .map(|candidate| RouteFallbackAttempt {
                exact_model: candidate.exact_model.clone(),
                provider_instance_name: candidate.provider_instance_name.clone(),
                provider_model_id: candidate.provider_model_id.clone(),
            })
            .collect(),
        route_trace: public_route_trace(decision),
        inventory_revision: selected.inventory_revision.clone(),
    }
}

fn public_route_trace(decision: &RouteDecision) -> RouteTrace {
    RouteTrace {
        attempts: Vec::new(),
        final_model: Some(decision.selected.exact_model.clone()),
    }
}

fn inference_response(
    receipt: crate::execution::ExecutionReceipt,
    decision: &RouteDecision,
) -> Result<Value, RPCErrors> {
    let mut response = match receipt.output.as_ref() {
        Some(output) => match &output.value {
            Value::Object(object) => object.clone(),
            Value::Null => Map::new(),
            _ => {
                return Err(inference_error(
                    AiccErrorCode::ProviderError,
                    "Provider returned a non-object canonical response",
                ));
            }
        },
        None => Map::new(),
    };
    let status = match receipt.state {
        ExecutionState::Succeeded => AiMethodStatus::Succeeded,
        ExecutionState::Failed | ExecutionState::Cancelled => AiMethodStatus::Failed,
        ExecutionState::Submitted | ExecutionState::Queued | ExecutionState::Running => {
            AiMethodStatus::Running
        }
    };
    response.insert("task_id".into(), Value::String(receipt.task_id));
    response.insert(
        "status".into(),
        serde_json::to_value(status).expect("AiMethodStatus serializes"),
    );
    if let Some(output) = receipt.output {
        response.insert(
            "usage".into(),
            serde_json::to_value(output.usage).expect("AiUsage serializes"),
        );
        if let Some(cost) = output.cost {
            response.insert(
                "cost".into(),
                serde_json::to_value(cost).expect("AiCost serializes"),
            );
        }
    }
    response.insert(
        "route_trace".into(),
        serde_json::to_value(public_route_trace(decision)).expect("RouteTrace serializes"),
    );
    response.insert("event_ref".into(), Value::String(receipt.event_ref));
    if let Some(provider_task_ref) = receipt.provider_task_ref {
        response.insert("provider_task_ref".into(), Value::String(provider_task_ref));
    }
    if let Some(error) = receipt.error {
        response.insert(
            "error".into(),
            serde_json::to_value(error).expect("AiccError serializes"),
        );
    }
    Ok(Value::Object(response))
}

#[async_trait]
pub(crate) trait ProviderValidator: Send + Sync {
    async fn validate(
        &self,
        request: ProviderValidateRequest,
    ) -> Result<ProviderValidateResponse, RPCErrors>;
}

#[async_trait]
pub(crate) trait UsageQueryPort: Send + Sync {
    async fn query_usage(
        &self,
        request: QueryUsageRequest,
    ) -> Result<QueryUsageResponse, RPCErrors>;
    async fn query_trace(
        &self,
        tenant_id: &str,
        request: QueryRouteTraceRequest,
    ) -> Result<QueryRouteTraceResponse, RPCErrors>;
}

#[async_trait]
pub(crate) trait QuotaQueryPort: Send + Sync {
    async fn query_quota(
        &self,
        caller: &AuthorizedCaller,
        request: QuotaQueryRequest,
    ) -> Result<QuotaQueryResponse, RPCErrors>;
}

#[async_trait]
pub(crate) trait DriverMetadataPort: Send + Sync {
    async fn get(&self) -> Result<DriverMetadataUpdateView, RPCErrors>;
    async fn set(
        &self,
        token: &str,
        expected_settings_revision: u64,
        request: DriverMetadataUpdateSetReq,
    ) -> Result<DriverMetadataUpdateSetResponse, RPCErrors>;
}
