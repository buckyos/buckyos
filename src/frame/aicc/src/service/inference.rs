use super::*;
use crate::error::RoutingError;

mod usecases;
pub(crate) use usecases::{apply_zone_usecase_set, UsecaseTarget};
use crate::routing::usecase::{LayerId, UsecaseLayers};
use crate::routing::SelectedRoute;
use buckyos_api::{OnUnavailable, AICC_FALLBACK_LLM};
use std::collections::BTreeSet;

#[derive(Clone)]
pub(crate) struct RuntimeInferencePort {
    runtime: Arc<RuntimeState>,
    codecs: Arc<CodecRegistry>,
    quota: Arc<QuotaSourceFactory>,
    execution: Arc<ExecutionEngine>,
    pub(super) storage: Arc<AiccStorage>,
    pub(super) resource_store: Arc<dyn ResourceStore>,
    pub(super) url_fetcher: Arc<dyn UrlResourceFetcher>,
    artifact_url_reader: Arc<RuntimeProviderExecutionPort>,
    model_health: Arc<ModelHealthRegistry>,
    governance: Arc<PolicyService>,
    events: Option<Arc<AiccEventLog>>,
}

fn named_object_url_provider(settings: &AiccSettings) -> Arc<dyn NamedObjectUrlProvider> {
    match settings.public_named_object_base_url.as_deref() {
        Some(base_url) => Arc::new(ZoneNamedObjectUrlProvider::new(base_url)),
        None => Arc::new(DisabledNamedObjectUrlProvider),
    }
}

fn cached_materialization(
    cache: &[(
        ResourceInputForm,
        Vec<ResourceRef>,
        Vec<RawMaterializedResource>,
    )],
    input_form: ResourceInputForm,
    resources: &[ResourceRef],
) -> Option<Vec<RawMaterializedResource>> {
    cache
        .iter()
        .find(|(form, cached_resources, _)| *form == input_form && cached_resources == resources)
        .map(|(_, _, materialized)| materialized.clone())
}

pub(super) fn same_url_origin(left: &str, right: &str) -> bool {
    let Ok(left) = reqwest::Url::parse(left) else {
        return false;
    };
    let Ok(right) = reqwest::Url::parse(right) else {
        return false;
    };
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

struct ProviderArtifactUrlFetcher {
    reader: Arc<RuntimeProviderExecutionPort>,
    tenant_id: String,
    user_id: String,
}

fn provider_url_requires_bytes(
    input_form: ResourceInputForm,
    url: &str,
    source_instance: &str,
    target_instance: &str,
    source_base_url: Option<&str>,
) -> bool {
    if input_form == ResourceInputForm::BytesOnly {
        true
    } else if input_form.requires_url() || source_instance == target_instance {
        false
    } else {
        source_base_url
            .filter(|base_url| !base_url.is_empty())
            .map(|base_url| same_url_origin(url, base_url))
            .unwrap_or(true)
    }
}

#[async_trait]
impl UrlResourceFetcher for ProviderArtifactUrlFetcher {
    async fn fetch(
        &self,
        url: &reqwest::Url,
        max_bytes: u64,
    ) -> Result<FetchedResource, ResourceError> {
        self.reader
            .fetch_artifact_url_resource(&self.tenant_id, &self.user_id, url, max_bytes)
            .await
    }
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
        artifact_url_reader: Arc<RuntimeProviderExecutionPort>,
        model_health: Arc<ModelHealthRegistry>,
        governance: Arc<PolicyService>,
    ) -> Self {
        Self {
            runtime,
            codecs,
            quota,
            execution,
            storage,
            resource_store,
            url_fetcher,
            artifact_url_reader,
            model_health,
            governance,
            events: None,
        }
    }

    pub(crate) fn with_events(mut self, events: Arc<AiccEventLog>) -> Self {
        self.events = Some(events);
        self
    }

    async fn access(
        &self,
        caller: &AuthorizedCaller,
        snapshot: &crate::runtime::RuntimeSnapshot,
    ) -> Result<crate::policy::Access, RPCErrors> {
        self.governance
            .access(
                caller,
                &crate::policy::Inventory::from_runtime(snapshot),
            )
            .await
    }

    /// The layers of the usecase table that apply to `caller`: the zone layer
    /// from the settings, the caller's own layer, the group bound to the
    /// caller's App and the session group the request names (§12).
    pub(super) async fn usecase_layers(
        &self,
        caller: &AuthorizedCaller,
        snapshot: &crate::runtime::RuntimeSnapshot,
        route_group: Option<&str>,
    ) -> Result<UsecaseLayers, RPCErrors> {
        let storage_error =
            |error: crate::error::StorageError| inference_error(AiccErrorCode::InternalError, error.to_string());
        let mut layers = UsecaseLayers::default();
        if let Some(routing) = &snapshot.settings.routing {
            layers.layers.push((LayerId::Zone, routing.usecases.clone()));
        }
        let subject = format!("user:{}", caller.user_id);
        let (user, _) = self.storage.routing_layer(&subject).await.map_err(storage_error)?;
        layers.layers.push((LayerId::User, user));
        if let Some(app_id) = caller.app_id.as_deref() {
            if let Some(group_id) = self
                .storage
                .routing_binding(&subject, app_id)
                .await
                .map_err(storage_error)?
            {
                if let Some((_, group)) =
                    self.storage.routing_group(&group_id).await.map_err(storage_error)?
                {
                    layers.app_group = Some(group_id.clone());
                    layers.layers.push((LayerId::Group(group_id), group.usecases));
                }
            }
        }
        if let Some(group_id) = route_group.filter(|group| !group.trim().is_empty()) {
            match self.storage.routing_group(group_id).await.map_err(storage_error)? {
                Some((owner, group)) if owner == subject => {
                    layers.session_group = Some(group_id.to_owned());
                    layers
                        .layers
                        .push((LayerId::Group(group_id.to_owned()), group.usecases));
                }
                _ => layers.route_group_ignored = true,
            }
        }
        Ok(layers)
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
        let models = snapshot.models.as_ref();
        let access = self.access(caller, snapshot.as_ref()).await?;
        let layers = self
            .usecase_layers(caller, snapshot.as_ref(), input.route_group.as_deref())
            .await?;
        let provider_names = if input.model.contains('@') {
            if !access.allows_model(&input.model) {
                return Err(not_permitted());
            }
            if access.is_unpriced(&input.model) {
                return Err(unpriced(&input.model));
            }
            // A permitted exact model answers a video demand it cannot meet
            // with the field at fault, not an anonymous "no candidate".
            if let Some(unmet) = exact_video_unmet(
                models,
                &input.model,
                input.requirements.video.as_ref(),
                &input.video_clips,
            ) {
                return Err(buckyos_api::video_error(
                    AiccErrorCode::UnsupportedOperation,
                    format!(
                        "{} cannot serve this request: {}",
                        input.model, unmet.reason
                    ),
                    &unmet.field,
                    None,
                    None,
                    None,
                )
                .to_krpc_error());
            }
            vec![crate::model::ExactModelName::parse(&input.model)
                .map_err(|error| inference_error(AiccErrorCode::InvalidRequest, error.to_string()))?
                .provider_instance_name()
                .to_owned()]
        } else {
            let reachable = usecase_reachable_models(models, &layers, &input.model, input.api_type)
                .map_err(|error| {
                    inference_error(AiccErrorCode::NoCandidateModel, error.to_string())
                })?;
            if reachable.is_empty() {
                let mut error = AiccError::new(
                    AiccErrorCode::NoCandidateModel,
                    format!("no connected model serves {}", input.model),
                );
                error.details = Some(json!({ "usecase": input.model }));
                return Err(error.to_krpc_error());
            }
            let permitted = reachable
                .into_iter()
                .filter(|model| access.usable_model(&model.exact_model.to_string()))
                .map(|model| model.identity.provider_instance_name)
                .collect::<Vec<_>>();
            if permitted.is_empty() {
                return Err(not_permitted());
            }
            permitted
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
            input.requirements.video.as_ref(),
            &access,
        )
        .await;
        let policy = policy_engine_for_route(
            models,
            &input.model,
            snapshot.settings.routing.as_ref(),
            input.policy.as_ref(),
            quota,
        )
        .map_err(|error| inference_error(AiccErrorCode::PolicyDenied, error.to_string()))?;
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
        request.video_clips = input.video_clips;
        let routed = crate::routing::usecase::resolve(
            &Router::new(models, &policy, &runtime_states),
            &layers,
            &request,
            true,
        )
        .map_err(|error| {
            // Too large for every window that could take it: the caller
            // shrinks the request instead of giving up on the route.
            let (code, restricted_message) = if error.is_context_length_insufficient() {
                (
                    AiccErrorCode::ContextLengthExceeded,
                    "the request does not fit the context window of any permitted model",
                )
            } else {
                (
                    AiccErrorCode::NoCandidateModel,
                    "no permitted model is available for this request",
                )
            };
            let mut error = if access.is_restricted() {
                AiccError::new(code, restricted_message)
            } else {
                AiccError::new(code, error.to_string())
            };
            if crate::routing::usecase::is_usecase(&request.model) {
                error.details = Some(json!({ "usecase": request.model }));
            }
            error.to_krpc_error()
        })?;
        let mut decision = routed.decision;
        if access.is_restricted() {
            decision
                .trace
                .retain_models(&|model| access.allows_model(model));
        }
        let on_unavailable = if policy.policy().no_fallback {
            OnUnavailable::None
        } else {
            routed.on_unavailable
        };
        let context = Arc::new(RouteContext {
            snapshot: snapshot.clone(),
            policy,
            runtime_states,
            layers,
            request,
            coverage: routed.coverage,
            on_unavailable,
            access,
        });
        Ok(RoutedInference {
            snapshot,
            decision,
            context,
            trace_id,
            request_id,
        })
    }

    async fn lower_call(
        &self,
        snapshot: &crate::runtime::RuntimeSnapshot,
        decision: &RouteDecision,
        call: &AiccCall,
        video_facts: &crate::video::VideoMaterialFacts,
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
        let credential = provider
            .resolve_client_credential(&selected.protocol_adapter_id)
            .await
            .map_err(|error| {
                inference_error(AiccErrorCode::NoProviderAvailable, error.to_string())
            })?;
        let provider_rules_id = provider.config.provider_rules_id.clone();
        let transport = HttpTransportConfig {
            request_timeout: provider.config.request_timeout,
            ..HttpTransportConfig::default()
        };
        let inventory_model = provider.inventory.models.iter().find(|model| {
            model.provider_model_id == selected.provider_model_id
                && model.model_driver_id == selected.model_driver_id
        });
        let video_contract = inventory_model.and_then(|model| {
            crate::video::VideoGenerationContract::from_capabilities(&model.capabilities)
        });
        let pricing = inventory_model
            .and_then(|model| model.pricing.as_ref())
            .map(|pricing| ResolvedPricing {
                source: match pricing.source {
                    crate::provider::PricingSource::Discovery => CallPricingSource::Discovery,
                    crate::provider::PricingSource::ProviderRules => {
                        CallPricingSource::ProviderRules
                    }
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
            (
                "policy_region",
                provider
                    .config
                    .instance_rules
                    .as_ref()
                    .and_then(|rules| rules.policy_region.as_ref()),
            ),
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
                    protocol_clients: provider.profile.protocol_clients(),
                    state_namespace: provider.profile.state_namespace.clone(),
                    use_server_defaults: crate::model::ExactModelName::parse(
                        &decision.selected.exact_model,
                    )
                    .is_ok_and(|model| model.variant().is_none())
                        && inventory_model.is_some_and(|model| !model.server_default_routes.is_empty()),
                    base_url: provider.config.base_url.clone(),
                    operation_base_urls: provider.config.operation_base_urls.clone(),
                    credential,
                    credential_reference: provider.config.credential.reference.clone(),
                    credential_header_name: provider
                        .profile
                        .credential_for_client(
                            &selected.protocol_adapter_id,
                            provider.config.credential_kind,
                            self.codecs.as_ref(),
                        )
                        .ok()
                        .and_then(|credential| credential.header_name),
                    limits: CodecLimits {
                        request_timeout: transport.request_timeout,
                        max_request_bytes: transport.max_request_bytes,
                        max_response_bytes: transport.max_response_bytes,
                    },
                    pricing,
                    match_dimensions,
                    video_contract,
                    video_facts: video_facts.clone(),
                },
            )
            .map_err(|error| match error {
                crate::error::CallLoweringError::Video(error) => error.to_krpc_error(),
                error => inference_error(error.code(), error.to_string()),
            })
    }

    async fn materialize_resources(
        &self,
        settings: &AiccSettings,
        caller: &AuthorizedCaller,
        request_id: &str,
        call: &mut ResolvedProviderCall,
        cache: &mut Vec<(
            ResourceInputForm,
            Vec<ResourceRef>,
            Vec<RawMaterializedResource>,
        )>,
    ) -> Result<(), RPCErrors> {
        let context = ResourceAccessContext::new(
            caller.tenant_id.clone(),
            caller.user_id.clone(),
            request_id.to_string(),
        )
        .map_err(resource_rpc_error)?
        .with_app(caller.artifact_app());
        call.resource_access_context = Some(context.clone());
        if call.resource_requirements.is_empty() {
            return Ok(());
        }
        // The codec decides what the Provider protocol accepts as a resource
        // input form: an `llm` codec writes the URL itself into the request
        // body, while a multipart upload codec needs the bytes. The form is read
        // through the resolved execution mode so a native task call is answered
        // by its `NativeTaskCodec` rather than by a buffered codec that happens
        // to be registered on the same binding.
        let input_form = self.codecs.resource_input_form(
            &call.protocol_adapter_id,
            &call.operation,
            call.api_type,
            call.execution_mode,
            &call
                .resource_requirements
                .iter()
                .map(|requirement| requirement.request_pointer.as_str())
                .collect::<Vec<_>>(),
        );
        let resources = call
            .resource_requirements
            .iter()
            .map(|requirement| requirement.resource.clone())
            .collect::<Vec<_>>();
        for resource in &resources {
            if let ResourceRef::NamedObject { obj_id } = resource {
                AuthenticatedResourceAuthorizer {
                    tenant_id: caller.tenant_id.clone(),
                    caller_id: caller.user_id.clone(),
                    storage: self.storage.clone(),
                    token: Some(caller.token.clone()),
                }
                .authorize(
                    &context,
                    &ResourceTarget::NamedObject {
                        obj_id: obj_id.clone(),
                    },
                    ResourceAccessOperation::Inspect,
                )
                .await
                .map_err(resource_rpc_error)?;
                if let Some(lease) = self
                    .resource_store
                    .retain(obj_id, None)
                    .await
                    .map_err(resource_rpc_error)?
                {
                    call.resource_leases.push(lease);
                }
            }
        }
        let runtime = self.runtime.capture().await;
        let mut protected_resources = Vec::new();
        let mut ordinary_resources = Vec::new();
        for resource in &resources {
            let protected = match resource {
                ResourceRef::Url { url, .. } => {
                    let source = self
                        .storage
                        .artifact_url_source(url, now_ms() as i64)
                        .await
                        .map_err(|_| {
                            inference_error(
                                AiccErrorCode::InternalError,
                                "Provider artifact source lookup failed",
                            )
                        })?;
                    if let Some(source) = source {
                        if source.tenant_id != caller.tenant_id || source.user_id != caller.user_id
                        {
                            return Err(inference_error(
                                AiccErrorCode::PolicyDenied,
                                "Provider artifact belongs to another user",
                            ));
                        }
                        provider_url_requires_bytes(
                            input_form,
                            url,
                            &source.provider_instance_name,
                            &call.provider_instance_name,
                            runtime
                                .providers
                                .get(&source.provider_instance_name)
                                .map(|provider| provider.config.base_url.as_str()),
                        )
                    } else {
                        false
                    }
                }
                ResourceRef::NamedObject { obj_id } => {
                    let owner = self
                        .storage
                        .artifact_owner(&obj_id.to_string())
                        .await
                        .map_err(|_| {
                            inference_error(
                                AiccErrorCode::InternalError,
                                "artifact ownership lookup failed",
                            )
                        })?;
                    if let Some((tenant_id, user_id)) = owner {
                        if tenant_id != caller.tenant_id || user_id != caller.user_id {
                            return Err(inference_error(
                                AiccErrorCode::PolicyDenied,
                                "AICC artifact belongs to another user",
                            ));
                        }
                        true
                    } else {
                        false
                    }
                }
                // Inline bytes are private request content. They may be sent
                // to a byte-capable Provider, but must never be published to a
                // public object URL just to satisfy a URL-only wire grammar.
                ResourceRef::Base64 { .. } => true,
            };
            if protected {
                protected_resources.push(resource.clone());
            } else {
                ordinary_resources.push(resource.clone());
            }
        }
        if input_form.requires_url() && !protected_resources.is_empty() {
            return Err(inference_error(
                AiccErrorCode::ResourceInvalid,
                "Provider protocol requires a public URL but the input resource is owner-scoped",
            ));
        }
        // A `ResourceRef::NamedObject` input carries the object id with it, and
        // the resource key is derived from that same id — so this map recovers
        // the id the key was built from. That matters because the object id is
        // the only cache key reachable here without reading the payload.
        let object_ids = resources
            .iter()
            .filter_map(|resource| match resource {
                ResourceRef::NamedObject { obj_id } => Some((
                    crate::resource::ResourceKey::from_ref(resource).into_string(),
                    obj_id.to_string(),
                )),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let authorizer = Arc::new(AuthenticatedResourceAuthorizer {
            tenant_id: caller.tenant_id.clone(),
            caller_id: caller.user_id.clone(),
            storage: self.storage.clone(),
            token: Some(caller.token.clone()),
        });
        let mut materialized = Vec::new();
        if !ordinary_resources.is_empty() {
            let ordinary = match cached_materialization(cache, input_form, &ordinary_resources) {
                Some(materialized) => materialized,
                None => {
                    let manager = ResourceManager::new(
                        authorizer.clone(),
                        self.resource_store.clone(),
                        self.url_fetcher.clone(),
                        ResourceLimits::default(),
                    )
                    .map_err(resource_rpc_error)?
                    .with_input_form(input_form)
                    .with_object_url_provider(named_object_url_provider(settings));
                    let inspected = manager
                        .inspect(&context, &ordinary_resources)
                        .await
                        .map_err(resource_rpc_error)?;
                    let materialized = manager
                        .materialize_after_provider_selected(&context, &call.exact_model, inspected)
                        .await
                        .map_err(resource_rpc_error)?;
                    cache.push((input_form, ordinary_resources.clone(), materialized.clone()));
                    materialized
                }
            };
            materialized.extend(ordinary);
        }
        if !protected_resources.is_empty() {
            let protected_form = ResourceInputForm::BytesOnly;
            let protected =
                match cached_materialization(cache, protected_form, &protected_resources) {
                    Some(materialized) => materialized,
                    None => {
                        let manager = ResourceManager::new(
                            authorizer,
                            self.resource_store.clone(),
                            Arc::new(ProviderArtifactUrlFetcher {
                                reader: self.artifact_url_reader.clone(),
                                tenant_id: caller.tenant_id.clone(),
                                user_id: caller.user_id.clone(),
                            }),
                            ResourceLimits::default(),
                        )
                        .map_err(resource_rpc_error)?
                        .with_input_form(protected_form);
                        let inspected = manager
                            .inspect(&context, &protected_resources)
                            .await
                            .map_err(resource_rpc_error)?;
                        let materialized = manager
                            .materialize_after_provider_selected(
                                &context,
                                &call.exact_model,
                                inspected,
                            )
                            .await
                            .map_err(resource_rpc_error)?;
                        cache.push((
                            protected_form,
                            protected_resources.clone(),
                            materialized.clone(),
                        ));
                        materialized
                    }
                };
            materialized.extend(protected);
        }
        for resource in materialized {
            let parts = resource.into_codec_parts().map_err(resource_rpc_error)?;
            if let Some(url) = parts.url.clone() {
                // The codec is handed this resource as a URL, so there are no
                // bytes and therefore no content digest. An object-form input
                // can still be resolved to a Provider handle: this exact object
                // may already have been handed to this Provider, and when it was
                // the handle saves the Provider the download entirely.
                //
                // There is deliberately *no* fallback to the content-digest
                // cache on a miss. Computing that digest means reading the whole
                // object, which is precisely the cost this path exists to avoid,
                // so a miss hands over the URL and stops there.
                let provider_artifact_id = match object_ids.get(parts.key.as_str()) {
                    Some(obj_id) => self
                        .storage
                        .provider_artifact(
                            &ProviderArtifactKey::ObjectId(obj_id.clone()),
                            &call.provider_instance_name,
                            &call.context.state_coordinate.origin_provider,
                            &context.tenant_id,
                            now_ms() as i64,
                        )
                        .await
                        .map_err(|_| {
                            inference_error(
                                AiccErrorCode::InternalError,
                                "Provider artifact ID lookup failed",
                            )
                        })?,
                    None => None,
                };
                if let Some(artifact_id) = provider_artifact_id.as_deref() {
                    log::info!(
                        "provider_artifact_object_hit: artifact_id={} provider_instance_name={} origin_provider={} exact_model={} resource_key={}",
                        artifact_id,
                        call.provider_instance_name,
                        call.context.state_coordinate.origin_provider,
                        call.exact_model,
                        parts.key.as_str()
                    );
                }
                log::debug!(
                    "resource_url_form: resource_key={} provider_instance_name={} exact_model={} provider_artifact_object={}",
                    parts.key.as_str(),
                    call.provider_instance_name,
                    call.exact_model,
                    provider_artifact_id.is_some()
                );
                call.context.resources.insert(
                    parts.key.into_string(),
                    CodecMaterializedResource::from_url(
                        url,
                        parts
                            .mime
                            .unwrap_or_else(|| DEFAULT_RESOURCE_MIME.to_string()),
                        parts.file_name,
                    )
                    .map_err(|error| {
                        inference_error(AiccErrorCode::ResourceInvalid, error.to_string())
                    })?
                    .with_provider_artifact_id(provider_artifact_id),
                );
                continue;
            }
            let parts_mime = parts
                .mime
                .clone()
                .unwrap_or_else(|| DEFAULT_RESOURCE_MIME.to_string());
            let content_digest = format!(
                "sha256:{}",
                Sha256::digest(&parts.bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let provider_artifact_id = self
                .storage
                .provider_artifact(
                    &ProviderArtifactKey::ContentDigest(content_digest.clone()),
                    &call.provider_instance_name,
                    &call.context.state_coordinate.origin_provider,
                    &context.tenant_id,
                    now_ms() as i64,
                )
                .await
                .map_err(|_| {
                    inference_error(
                        AiccErrorCode::InternalError,
                        "Provider artifact ID lookup failed",
                    )
                })?;
            if let Some(artifact_id) = provider_artifact_id.as_deref() {
                log::info!(
                    "provider_artifact_id_hit: artifact_id={} content_digest={} provider_instance_name={} origin_provider={} exact_model={} resource_key={}",
                    artifact_id,
                    content_digest,
                    call.provider_instance_name,
                    call.context.state_coordinate.origin_provider,
                    call.exact_model,
                    parts.key.as_str()
                );
            }
            call.context.resources.insert(
                parts.key.into_string(),
                CodecMaterializedResource::new(parts.bytes, parts_mime, parts.file_name)
                    .map_err(|error| {
                        inference_error(AiccErrorCode::ResourceInvalid, error.to_string())
                    })?
                    .with_provider_artifact_id(provider_artifact_id),
            );
        }
        Ok(())
    }
}

pub(super) struct AuthenticatedResourceAuthorizer {
    pub(super) tenant_id: String,
    pub(super) caller_id: String,
    pub(super) storage: Arc<AiccStorage>,
    pub(super) token: Option<String>,
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
                    .artifact_owner(&obj_id.to_string())
                    .await
                    .map_err(|_| {
                        ResourceError::new(
                            ResourceFailure::Unavailable,
                            "artifact ownership lookup failed",
                        )
                    })?;
                if owner.as_ref().is_some_and(|(tenant_id, user_id)| {
                    tenant_id != &context.tenant_id || user_id != &context.caller_id
                }) {
                    return Err(ResourceError::new(
                        ResourceFailure::Unauthorized,
                        "AICC artifact belongs to another user",
                    ));
                }
                if owner.is_none() {
                    let token = self
                        .token
                        .as_deref()
                        .filter(|token| !token.is_empty())
                        .ok_or_else(|| {
                            ResourceError::new(
                                ResourceFailure::Unauthorized,
                                "NDM input requires the authenticated caller session",
                            )
                        })?;
                    authorize_ndm_input(token, obj_id).await?;
                }
            }
        }
        Ok(())
    }
}

async fn authorize_ndm_input(token: &str, obj_id: &ndn_lib::ObjId) -> Result<(), ResourceError> {
    let unavailable = |_| {
        ResourceError::new(
            ResourceFailure::Unavailable,
            "NDM authorization is unavailable",
        )
    };
    let runtime = get_buckyos_api_runtime().map_err(unavailable)?;
    let endpoint = runtime
        .get_zone_service_url(buckyos_api::NFS_SERVER_SERVICE_NAME, false)
        .await
        .map_err(unavailable)?;
    let mut url = reqwest::Url::parse(&endpoint)
        .map_err(|_| ResourceError::new(ResourceFailure::Unavailable, "invalid NDM service URL"))?;
    url.set_path("/ndm/v1/store/get_object");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| {
            ResourceError::new(
                ResourceFailure::Unavailable,
                "NDM authorization client failed",
            )
        })?;
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&json!({ "obj_id": obj_id.to_string() }))
        .send()
        .await
        .map_err(|_| {
            ResourceError::new(
                ResourceFailure::Unavailable,
                "NDM authorization request failed",
            )
        })?;
    if !response.status().is_success() {
        return Err(ResourceError::new(
            if matches!(response.status().as_u16(), 401 | 403 | 404) {
                ResourceFailure::Unauthorized
            } else {
                ResourceFailure::Unavailable
            },
            "input object is not readable by the authenticated caller",
        ));
    }
    Ok(())
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
                    route_group: request.route_group,
                    estimated_input_tokens: request.estimated_input_tokens,
                    estimated_output_tokens: request.estimated_output_tokens,
                    video_clips: Default::default(),
                },
            )
            .await?;
        Ok(route_response(&routed.decision))
    }

    async fn preview_routes(
        &self,
        caller: &AuthorizedCaller,
        request: RoutingPreviewRequest,
    ) -> Result<RoutingPreviewResponse, RPCErrors> {
        let snapshot = self.runtime.capture().await;
        let models = snapshot.models.as_ref();
        let paths = if request.paths.is_empty() {
            models
                .logical_model_views()
                .into_iter()
                .map(|view| view.path)
                .collect::<Vec<_>>()
        } else {
            request.paths.clone()
        };
        // The caller's own view, as `routing.usecases` shows it.
        let mut layers = UsecaseLayers::default();
        if let Some(routing) = &snapshot.settings.routing {
            layers.layers.push((LayerId::Zone, routing.usecases.clone()));
        }
        let (user, _) = self
            .storage
            .routing_layer(&format!("user:{}", caller.user_id))
            .await
            .map_err(|error| inference_error(AiccErrorCode::InternalError, error.to_string()))?;
        layers.layers.push((LayerId::User, user));
        let preview = Preview {
            snapshot: snapshot.as_ref(),
            models,
            layers: &layers,
            assume_ready: None,
            excluded_providers: BTreeSet::new(),
            requirements: request.requirements.clone().unwrap_or_default(),
            with_backup: false,
        };
        let access = self.access(caller, snapshot.as_ref()).await?;
        let mut entries = Vec::with_capacity(paths.len());
        for previewed in self.preview(caller, &preview, &paths).await? {
            let mut entry = RoutingPreviewEntry {
                kind: models
                    .logical_directory_kind(&previewed.path)
                    .unwrap_or(AiccLogicalDirectoryKind::Directory),
                path: previewed.path,
                api_type: previewed.api_type.unwrap_or_default(),
                available: false,
                selected_exact_model: None,
                error: None,
                trace: None,
            };
            match previewed.route {
                Ok(route) => {
                    entry.available = true;
                    entry.selected_exact_model = Some(route.decision.selected.exact_model.clone());
                    if request.explain {
                        entry.trace = Some(public_route_trace_json(&route.decision)?);
                    }
                }
                Err(error) => {
                    entry.error = Some(if access.is_restricted() {
                        "no permitted model is available".to_string()
                    } else {
                        error
                    })
                }
            }
            entries.push(entry);
        }
        Ok(RoutingPreviewResponse {
            settings_revision: snapshot.settings_revision,
            entries,
        })
    }

    async fn usecase_rpc(
        &self,
        caller: &AuthorizedCaller,
        method: &str,
        params: Value,
    ) -> Result<Value, RPCErrors> {
        use buckyos_api::routing_methods as methods;
        match method {
            methods::ROUTING_USECASES => {
                let request = serde_json::from_value(params).map_err(|error| {
                    RPCErrors::ParseRequestError(format!("{method}: {error}"))
                })?;
                serde_json::to_value(self.usecase_table(caller, request).await?)
                    .map_err(|error| RPCErrors::ReasonError(error.to_string()))
            }
            methods::ROUTING_USECASE_SET => {
                let request: buckyos_api::RoutingUsecaseSetRequest =
                    serde_json::from_value(params).map_err(|error| {
                        RPCErrors::ParseRequestError(format!("{method}: {error}"))
                    })?;
                let target = UsecaseTarget::parse(&request.target)?;
                serde_json::to_value(self.set_usecase(caller, &target, &request).await?)
                    .map_err(|error| RPCErrors::ReasonError(error.to_string()))
            }
            _ => self.routing_store_rpc(caller, method, params).await,
        }
    }

    async fn validate_usecase_route(
        &self,
        caller: &AuthorizedCaller,
        zone: bool,
        usecase: &str,
        route: &buckyos_api::AiccUsecaseRoute,
    ) -> Result<(), RPCErrors> {
        RuntimeInferencePort::validate_usecase_route(self, caller, zone, usecase, route).await
    }

    async fn coverage_preview(
        &self,
        caller: &AuthorizedCaller,
        added: &crate::model::ProviderInventory,
    ) -> Result<Vec<buckyos_api::UsecaseCoverageChange>, RPCErrors> {
        RuntimeInferencePort::coverage_preview(self, caller, added).await
    }

    fn provider_changed(&self, provider_instance_name: &str) {
        self.model_health.clear_provider(provider_instance_name);
    }

    async fn invoke(&self, caller: &AuthorizedCaller, call: AiccCall) -> Result<Value, RPCErrors> {
        let route_input = route_input_for_call(&call)?;
        let request_model = route_input.model.clone();
        let has_explicit_trace_id = call.trace_id().is_some();
        // A helper's identity is the request the caller sent, not the model it
        // happened to route to, so a retry finds the same task.
        let helper_body = matches!(
            call,
            AiccCall::HelperLlmChat(_)
                | AiccCall::HelperTextToImage(_)
                | AiccCall::HelperVideoGenerate(_)
        )
        .then(|| call_params(&call))
        .transpose()?;
        let canonical_body = match helper_body {
            Some(body) => body,
            None => call_params(&call)?,
        };
        if let Some(key) = canonical_body
            .get("idempotency_key")
            .and_then(Value::as_str)
        {
            if let Some((receipt, exact_model)) = self
                .execution
                .replay(&caller.tenant_id, exact_method(&call), key, &canonical_body)
                .await
                .map_err(|error| error.to_krpc_error())?
            {
                return inference_response(
                    receipt,
                    RouteTrace {
                        attempts: Vec::new(),
                        final_model: exact_model,
                        fallback: None,
                    },
                );
            }
        }
        // The first route applies privacy, local-only and visibility policy
        // before any material is fetched.
        let mut routed = self.route(caller, route_input.clone()).await?;
        let mut exact_call = exact_call_for_route(call, &routed.decision.selected.exact_model)?;
        let mut video_facts = crate::video::VideoMaterialFacts::new();
        if let AiccCall::VideoGenerate(request) = &exact_call {
            let prepared = self
                .prepare_video_materials(
                    caller,
                    &routed.request_id,
                    &routed.decision.selected.exact_model,
                    request,
                )
                .await?;
            let mut final_input = route_input;
            final_input.trace_id = Some(routed.trace_id.clone());
            final_input.request_id = Some(routed.request_id.clone());
            final_input.video_clips =
                crate::video::clip_summary(&prepared.request, &prepared.facts);
            routed = self.route(caller, final_input).await?;
            exact_call = AiccCall::VideoGenerate(buckyos_api::VideoGenerateRequest {
                exact_model: routed.decision.selected.exact_model.clone(),
                ..prepared.request
            });
            video_facts = prepared.facts;
        }
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
        let fallback = Arc::new(InferenceFallback {
            port: self.clone(),
            caller: caller.clone(),
            context: routed.context.clone(),
            decision: routed.decision.clone(),
            call: exact_call.clone(),
            video_facts: video_facts.clone(),
            request_id: routed.request_id.clone(),
            used: Default::default(),
        });
        // A Provider an administrator marked faulty is never called: the
        // usecase's policy takes over at once, or the request fails.
        let selected_provider = routed.decision.selected.provider_instance_name.clone();
        let marked_faulty = faulty_providers(routed.snapshot.as_ref()).contains(&selected_provider);
        let mut primary = if marked_faulty {
            match fallback
                .prepare(&routed.decision.selected.exact_model, "marked_faulty")
                .await
                .map_err(|error| error.to_krpc_error())?
            {
                Some((_, call)) => call,
                None => {
                    let mut error = AiccError::new(
                        AiccErrorCode::NoProviderAvailable,
                        format!(
                            "Provider {selected_provider} is marked faulty by an administrator"
                        ),
                    );
                    error.details = Some(json!({
                        "reason": "marked_faulty",
                        "provider_instance_name": selected_provider,
                        "exact_model": routed.decision.selected.exact_model,
                    }));
                    return Err(error.to_krpc_error());
                }
            }
        } else {
            let mut primary = self
                .lower_call(
                    routed.snapshot.as_ref(),
                    &routed.decision,
                    &exact_call,
                    &video_facts,
                )
                .await?;
            self.materialize_resources(
                routed.snapshot.settings.as_ref(),
                caller,
                &routed.request_id,
                &mut primary,
                &mut Vec::new(),
            )
            .await?;
            primary
        };
        if let Err(denial) = self.governance.precheck(caller, &primary).await {
            // A user policy that refuses the model lets the usecase's
            // policy pick another one, once.
            let backup = if marked_faulty {
                None
            } else {
                fallback
                    .prepare(&primary.exact_model, "quota_denied")
                    .await
                    .map_err(|error| error.to_krpc_error())?
            };
            let Some((_, backup)) = backup else {
                return Err(denial.to_krpc_error());
            };
            if let Err(error) = self.governance.precheck(caller, &backup).await {
                return Err(crate::execution::with_original_failure(
                    Some((primary.exact_model.clone(), denial)),
                    error,
                    &backup.exact_model,
                )
                .to_krpc_error());
            }
            primary = backup;
        }
        let switched = fallback.used.lock().expect("fallback lock").is_some();
        let engine_fallback: Option<Arc<dyn crate::execution::UnavailableFallback>> =
            (!switched && routed.context.on_unavailable() != OnUnavailable::None)
                .then(|| fallback.clone() as Arc<dyn crate::execution::UnavailableFallback>);
        let receipt = self
            .execution
            .execute(crate::execution::ExecutionRequest {
                tenant_id: caller.tenant_id.clone(),
                user_id: caller.user_id.clone(),
                caller_app_id: caller.app_id.clone(),
                attribution: Some(caller.attribution()),
                principal: super::policy::principal_of(caller),
                trace_id: Some(routed.trace_id.clone()),
                request_model: request_model.clone(),
                idempotency_key,
                canonical_body,
                parent_task_id,
                runtime_generation: routed.snapshot.generation,
                primary,
                fallback: engine_fallback,
                now_ms: now_ms(),
                idempotency_window_ms: None,
            })
            .await
            .map_err(|error| error.to_krpc_error())?;
        if let Some(decision) = fallback.used.lock().expect("fallback lock").take() {
            if let (Some(events), Some(switch)) = (&self.events, decision.trace.fallback.as_ref()) {
                events.record(
                    AiccEventLevel::Warning,
                    super::events::EVENT_UNAVAILABLE_FALLBACK,
                    format!("{} was unavailable; {} served the request", switch.from, switch.to),
                    json!({
                        "from": switch.from,
                        "to": switch.to,
                        "policy": switch.policy,
                        "reason": switch.reason,
                        "usecase": decision.trace.usecase.as_ref().map(|usecase| &usecase.usecase),
                        "task_id": receipt.task_id,
                    }),
                );
            }
            routed.decision = decision;
        }
        if receipt.provider_task_ref.is_some() && !receipt.state.is_terminal() {
            let execution = Arc::clone(&self.execution);
            let task_id = receipt.task_id.clone();
            let initial_poll_after = receipt.initial_poll_after;
            tokio::spawn(async move {
                if let Err(error) = execution.drive_native_after(&task_id, initial_poll_after).await {
                    log::error!("native task driver stopped: task_id={task_id} error={error}");
                }
            });
        }
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
                    route_trace_json: public_route_trace_json(&routed.decision)?,
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
        inference_response(receipt, public_route_trace(&routed.decision))
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

fn unpriced(exact_model: &str) -> RPCErrors {
    let mut error = AiccError::new(
        AiccErrorCode::ResourceNotPermitted,
        "the model has no price; ask an administrator to price it",
    );
    error.details = serde_json::to_value(buckyos_api::ResourceUnpricedDetails {
        reason: "unpriced".to_string(),
        provider_instance_name: exact_model
            .rsplit_once('@')
            .map(|(_, provider)| provider.to_string())
            .unwrap_or_default(),
        exact_model: exact_model.to_string(),
    })
    .ok();
    error.to_krpc_error()
}

fn not_permitted() -> RPCErrors {
    inference_error(
        AiccErrorCode::ResourceNotPermitted,
        "no permitted model is available for this request",
    )
}

async fn candidate_runtime_states(
    snapshot: &crate::runtime::RuntimeSnapshot,
    caller: &AuthorizedCaller,
    model_health: &ModelHealthRegistry,
    estimated_input_tokens: Option<u64>,
    estimated_output_tokens: Option<u64>,
    video: Option<&buckyos_api::VideoGenerationRequirement>,
    access: &crate::policy::Access,
) -> BTreeMap<String, CandidateRuntimeState> {
    runtime_states_for(
        snapshot,
        snapshot.models.as_ref(),
        None,
        caller,
        model_health,
        estimated_input_tokens,
        estimated_output_tokens,
        video,
        access,
    )
    .await
}

/// Static routing facts of every model in `models`. `assume_ready` names a
/// Provider that is not running yet (one being added) and is treated as
/// enabled, configured and visible.
#[allow(clippy::too_many_arguments)]
async fn runtime_states_for(
    snapshot: &crate::runtime::RuntimeSnapshot,
    models: &crate::model::ModelRegistry,
    assume_ready: Option<&str>,
    caller: &AuthorizedCaller,
    model_health: &ModelHealthRegistry,
    estimated_input_tokens: Option<u64>,
    estimated_output_tokens: Option<u64>,
    video: Option<&buckyos_api::VideoGenerationRequirement>,
    access: &crate::policy::Access,
) -> BTreeMap<String, CandidateRuntimeState> {
    let mut credentials = BTreeMap::new();
    for provider in snapshot.providers.list() {
        let name = provider.config.provider_instance_name.clone();
        credentials.insert(name, provider.resolve_credential().await.is_ok());
    }
    models
        .model_views()
        .into_iter()
        .map(|model| {
            let ready = assume_ready == Some(model.provider_instance_name.as_str());
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
            let metadata_routable = snapshot
                .provider_metadata
                .get(&model.provider_instance_name)
                .is_none_or(|metadata| metadata.routable);
            // Only static facts: whether the Provider is configured, enabled
            // and visible. Its observed health never changes the route.
            let enabled = ready
                || (settings.is_some_and(|provider| provider.enabled)
                    && runtime.is_some()
                    && metadata_routable
                    && access.usable_model(&model.exact_model));
            // Observed latency only gates a request that set `max_latency_ms`.
            let observed =
                model_health.snapshot(&model.exact_model, &model.provider_instance_name, now_ms());
            let state = CandidateRuntimeState {
                enabled,
                credential_available: ready
                    || credentials
                        .get(&model.provider_instance_name)
                        .copied()
                        .unwrap_or(false),
                provider_privacy: if local {
                    ProviderPrivacy::Local
                } else {
                    ProviderPrivacy::PublicCloud
                },
                trust: Some(ProviderTrustView {
                    provider_type,
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
                            video.map(|video| video_usage_estimate(video, &model.capabilities)),
                        )
                    })
                    .and_then(|cost| {
                        snapshot
                            .catalog
                            .cost_in_usd(cost, now_ms().min(i64::MAX as u64) as i64)
                    }),
                p95_latency_ms: observed.p95_latency_ms,
                cache_hit_probability: None,
            };
            (model.exact_model, state)
        })
        .collect()
}

/// Units a video request is expected to bill: its output seconds (requested,
/// else the model default) and its reference images. Unknown seconds stay
/// unknown so the estimate is not mistaken for free.
fn video_usage_estimate(
    video: &buckyos_api::VideoGenerationRequirement,
    capabilities: &BTreeMap<String, Value>,
) -> buckyos_api::AiUsage {
    let default_seconds = crate::video::VideoGenerationContract::from_capabilities(capabilities)
        .and_then(|contract| contract.output.default_duration_seconds);
    buckyos_api::AiUsage {
        video_seconds: video
            .output
            .as_ref()
            .and_then(|output| output.duration_seconds)
            .or(default_seconds),
        reference_images: Some(u64::from(video.reference_images)),
        request_units: Some(1),
        ..Default::default()
    }
}

fn estimate_model_cost(
    pricing: &crate::catalog::Pricing,
    estimated_input_tokens: Option<u64>,
    estimated_output_tokens: Option<u64>,
    video_usage: Option<buckyos_api::AiUsage>,
) -> Option<buckyos_api::Money> {
    if let Some(amount) = pricing.estimated_cost {
        return (amount.is_finite() && amount >= 0.0)
            .then(|| buckyos_api::Money::new(amount, pricing.currency.to_ascii_uppercase()));
    }
    let usage = video_usage.unwrap_or_else(|| buckyos_api::AiUsage {
        input_tokens: Some(estimated_input_tokens.unwrap_or(1_000)),
        output_tokens: Some(estimated_output_tokens.unwrap_or(1_000)),
        request_units: Some(1),
        ..Default::default()
    });
    let quote = crate::execution::PinnedPricingSnapshot::from_pricing(
        pricing,
        None,
        std::time::SystemTime::now(),
    )
    .ok()??
    .completion_cost(&usage)?;
    Some(buckyos_api::Money::new(quote.amount, quote.currency))
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
            route_group: request.route_group.clone(),
            estimated_input_tokens: None,
            estimated_output_tokens: request.max_output_tokens,
            video_clips: Default::default(),
        }),
        AiccCall::HelperTextToImage(request) => Ok(InferenceRouteInput {
            trace_id: request.trace_id.clone(),
            request_id: None,
            model: request.logical_model.clone(),
            api_type: buckyos_api::ApiType::ImageTextToImage,
            requirements: request.requirements.clone().into(),
            disable: request.disable.clone(),
            policy: request.policy.clone(),
            route_group: request.route_group.clone(),
            estimated_input_tokens: None,
            estimated_output_tokens: None,
            video_clips: Default::default(),
        }),
        AiccCall::HelperVideoGenerate(request) => {
            let mut requirements: buckyos_api::ModelRequirement =
                request.requirements.clone().into();
            requirements.video = Some(request.requirement().map_err(|e| e.to_krpc_error())?);
            Ok(InferenceRouteInput {
                trace_id: request.trace_id.clone(),
                request_id: None,
                model: request.logical_model.clone(),
                api_type: buckyos_api::ApiType::VideoGenerate,
                requirements,
                disable: request.disable.clone(),
                policy: request.policy.clone(),
                route_group: request.route_group.clone(),
                estimated_input_tokens: None,
                estimated_output_tokens: None,
                video_clips: Default::default(),
            })
        }
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
            let mut requirements = if let AiccCall::DecisionEvaluate(request) = call {
                request.validate().map_err(|e| e.to_krpc_error())?;
                if request.execution_mode == buckyos_api::AiccExecutionMode::Stream {
                    return Err(inference_error(
                        AiccErrorCode::UnsupportedExecutionMode,
                        "decision streaming is not supported",
                    ));
                }
                request.requirements()
            } else {
                buckyos_api::ModelRequirement::default()
            };
            if let AiccCall::ChatCompletionsCreate(request) = call {
                requirements.web_search = request.web_search;
            }
            if let AiccCall::VideoGenerate(request) = call {
                requirements.video = Some(request.requirement());
            }
            if let Some(voice) = params.get("voice") {
                requirements.canonical_fields.insert(
                    "/voice".into(),
                    buckyos_api::CanonicalFieldRequirement::new(voice.clone()),
                );
            }
            Ok(InferenceRouteInput {
                trace_id: call.trace_id().map(str::to_owned),
                request_id: None,
                model,
                api_type: call.api_type().ok_or_else(|| {
                    inference_error(AiccErrorCode::InvalidMethod, "unsupported inference method")
                })?,
                requirements,
                disable: Default::default(),
                policy: None,
                route_group: None,
                estimated_input_tokens: None,
                estimated_output_tokens: None,
                video_clips: Default::default(),
            })
        }
    }
}

/// Routing for one caller without a request: `routing.preview`, the usecase
/// table and the coverage of a Provider being added.
pub(super) struct Preview<'a> {
    pub snapshot: &'a crate::runtime::RuntimeSnapshot,
    pub models: &'a crate::model::ModelRegistry,
    pub layers: &'a UsecaseLayers,
    /// A Provider being added: treated as enabled and configured.
    pub assume_ready: Option<&'a str>,
    /// Providers treated as disabled (a deletion preview).
    pub excluded_providers: BTreeSet<String>,
    pub requirements: buckyos_api::ModelRequirement,
    /// Also predict what the row's "when unavailable" policy would use.
    pub with_backup: bool,
}

pub(super) struct PreviewedPath {
    pub path: String,
    pub api_type: Option<String>,
    pub route: Result<crate::routing::usecase::UsecaseRoute, String>,
    /// `NoCandidate` with nothing filtered: no model of the API type at all.
    pub no_model: bool,
    pub backup: Option<Result<Option<RouteDecision>, String>>,
}

impl RuntimeInferencePort {
    pub(super) async fn preview(
        &self,
        caller: &AuthorizedCaller,
        preview: &Preview<'_>,
        paths: &[String],
    ) -> Result<Vec<PreviewedPath>, RPCErrors> {
        let caller_identity = CallerIdentity {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            app_id: caller.app_id.clone(),
        };
        let access = self.access(caller, preview.snapshot).await?;
        let runtime_states = runtime_states_for(
            preview.snapshot,
            preview.models,
            preview.assume_ready,
            caller,
            self.model_health.as_ref(),
            None,
            None,
            None,
            &access,
        )
        .await;
        let api_types = preview
            .models
            .logical_model_views()
            .into_iter()
            .filter_map(|view| view.api_type.map(|api_type| (view.path, api_type)))
            .collect::<BTreeMap<_, _>>();
        let mut provider_names = preview
            .snapshot
            .providers
            .list()
            .into_iter()
            .map(|provider| provider.config.provider_instance_name.clone())
            .collect::<Vec<_>>();
        provider_names.extend(preview.assume_ready.map(str::to_owned));
        let order = provider_order(preview.snapshot);
        let faulty = faulty_providers(preview.snapshot);
        let price_of = |exact: &str| model_pricing(preview.snapshot, exact);
        let to_usd = |amount: f64, currency: &str| usd(preview.snapshot, amount, currency);
        let restrict = |decision: &mut RouteDecision| {
            if access.is_restricted() {
                decision.trace.retain_models(&|model| access.allows_model(model));
            }
        };
        let mut quotas = std::collections::HashMap::new();
        let mut previewed = Vec::with_capacity(paths.len());
        for path in paths {
            let api_type_name = inherited_api_type(&api_types, path);
            let mut entry = PreviewedPath {
                path: path.clone(),
                api_type: api_type_name.clone(),
                route: Err("logical directory has no API type".to_string()),
                no_model: false,
                backup: None,
            };
            let Some(api_type) = api_type_name.and_then(|name| {
                serde_json::from_value::<buckyos_api::ApiType>(Value::String(name)).ok()
            }) else {
                previewed.push(entry);
                continue;
            };
            if !quotas.contains_key(&api_type) {
                let quota = self
                    .quota
                    .prepare_route(
                        &caller_identity,
                        api_type.capability(),
                        api_type.typed_method(),
                        provider_names.clone(),
                    )
                    .await
                    .map_err(|_| {
                        inference_error(AiccErrorCode::PolicyDenied, "quota scope is invalid")
                    })?;
                quotas.insert(api_type, quota);
            }
            let policy = match policy_engine_for_route(
                preview.models,
                path,
                preview.snapshot.settings.routing.as_ref(),
                None,
                quotas[&api_type].clone(),
            ) {
                Ok(policy) => policy,
                Err(error) => {
                    entry.route = Err(error.to_string());
                    previewed.push(entry);
                    continue;
                }
            };
            let mut request = RoutingRequest::new(
                next_inference_id(),
                next_inference_id(),
                path.clone(),
                api_type,
                caller_identity.clone(),
            );
            request.requirements = preview.requirements.clone();
            request.excluded_providers = preview.excluded_providers.clone();
            let router = Router::new(preview.models, &policy, &runtime_states);
            match crate::routing::usecase::resolve(&router, preview.layers, &request, true) {
                Ok(mut route) => {
                    restrict(&mut route.decision);
                    if preview.with_backup && route.on_unavailable != OnUnavailable::None {
                        entry.backup = Some(
                            crate::routing::usecase::unavailable_fallback(
                                &router,
                                preview.layers,
                                &request,
                                route.coverage,
                                &route.decision.selected,
                                route.on_unavailable,
                                &crate::routing::usecase::FallbackSearch {
                                    price_of: &price_of,
                                    to_usd: &to_usd,
                                    provider_order: &order,
                                    faulty: &faulty,
                                },
                            )
                            .map(|backup| {
                                backup.map(|mut decision| {
                                    restrict(&mut decision);
                                    decision
                                })
                            })
                            .map_err(|error| error.to_string()),
                        );
                    }
                    entry.route = Ok(route);
                }
                Err(error) => {
                    entry.no_model = matches!(
                        &error,
                        RoutingError::NoCandidate { filtered, .. } if filtered.is_empty()
                    );
                    entry.route = Err(error.to_string());
                }
            }
            previewed.push(entry);
        }
        Ok(previewed)
    }
}

/// Every model a usecase may route to: its default expansion, the models
/// its layers chose and, for an LLM usecase, the fallback LLM's.
fn usecase_reachable_models(
    models: &crate::model::ModelRegistry,
    layers: &UsecaseLayers,
    path: &str,
    api_type: buckyos_api::ApiType,
) -> Result<Vec<crate::model::RegisteredModel>, crate::error::ModelRegistryError> {
    let mut reachable = models.reachable_models(path, api_type)?;
    let mut rows = vec![path.to_owned()];
    if crate::routing::usecase::LLM_USECASES.contains(&path) && layers.fallback_llm_enabled() {
        reachable.extend(models.reachable_models(AICC_FALLBACK_LLM, api_type)?);
        rows.push(AICC_FALLBACK_LLM.to_owned());
    }
    for row in rows.iter().filter(|row| crate::routing::usecase::is_usecase(row)) {
        for choice in layers.cells(row).models {
            if let Some(model) = models.exact_model(&choice.model) {
                reachable.push(model.clone());
            }
        }
    }
    let mut seen = BTreeSet::new();
    reachable.retain(|model| seen.insert(model.exact_model.to_string()));
    Ok(reachable)
}

/// What a routed request keeps for its "when unavailable" policy.
pub(crate) struct RouteContext {
    snapshot: Arc<crate::runtime::RuntimeSnapshot>,
    policy: crate::routing::PolicyEngine<crate::routing::policy::PreparedQuotaSource>,
    runtime_states: BTreeMap<String, CandidateRuntimeState>,
    layers: UsecaseLayers,
    request: RoutingRequest,
    coverage: bool,
    on_unavailable: OnUnavailable,
    access: crate::policy::Access,
}

impl RouteContext {
    pub(crate) fn on_unavailable(&self) -> OnUnavailable {
        self.on_unavailable
    }

    /// The backup route for `failed` under the request's policy.
    fn backup_route(&self, failed: &SelectedRoute) -> Result<Option<RouteDecision>, RoutingError> {
        if self.on_unavailable == OnUnavailable::None {
            return Ok(None);
        }
        let router = Router::new(self.snapshot.models.as_ref(), &self.policy, &self.runtime_states);
        let snapshot = self.snapshot.as_ref();
        let price_of = |exact: &str| model_pricing(snapshot, exact);
        let to_usd = |amount: f64, currency: &str| usd(snapshot, amount, currency);
        let order = provider_order(snapshot);
        let faulty = faulty_providers(snapshot);
        let mut decision = crate::routing::usecase::unavailable_fallback(
            &router,
            &self.layers,
            &self.request,
            self.coverage,
            failed,
            self.on_unavailable,
            &crate::routing::usecase::FallbackSearch {
                price_of: &price_of,
                to_usd: &to_usd,
                provider_order: &order,
                faulty: &faulty,
            },
        )?;
        if let Some(decision) = decision.as_mut() {
            if self.access.is_restricted() {
                let access = &self.access;
                decision.trace.retain_models(&|model| access.allows_model(model));
            }
        }
        Ok(decision)
    }
}

/// The unit prices of an exact model from its Provider's inventory.
pub(super) fn model_pricing(
    snapshot: &crate::runtime::RuntimeSnapshot,
    exact_model: &str,
) -> Option<crate::catalog::Pricing> {
    let model = snapshot.models.exact_model(exact_model)?;
    snapshot
        .providers
        .get(&model.identity.provider_instance_name)?
        .inventory
        .models
        .iter()
        .find(|candidate| {
            candidate.provider_model_id == model.identity.provider_model_id
                && candidate.model_driver_id == model.identity.model_driver_id
        })?
        .pricing
        .as_ref()
        .map(|pricing| pricing.value.clone())
}

pub(super) fn usd(
    snapshot: &crate::runtime::RuntimeSnapshot,
    amount: f64,
    currency: &str,
) -> Option<f64> {
    snapshot
        .catalog
        .cost_in_usd(
            buckyos_api::Money::new(amount, currency.to_ascii_uppercase()),
            now_ms().min(i64::MAX as u64) as i64,
        )
        .map(|cost| cost.amount)
}

pub(super) fn provider_order(snapshot: &crate::runtime::RuntimeSnapshot) -> Vec<String> {
    snapshot
        .settings
        .providers
        .iter()
        .map(|provider| provider.provider_instance_name.clone())
        .collect()
}

pub(super) fn faulty_providers(snapshot: &crate::runtime::RuntimeSnapshot) -> BTreeSet<String> {
    snapshot
        .settings
        .providers
        .iter()
        .filter(|provider| provider.marked_faulty)
        .map(|provider| provider.provider_instance_name.clone())
        .collect()
}

/// The usecase policy of one request, handed to the execution engine.
struct InferenceFallback {
    port: RuntimeInferencePort,
    caller: AuthorizedCaller,
    context: Arc<RouteContext>,
    decision: RouteDecision,
    call: AiccCall,
    video_facts: crate::video::VideoMaterialFacts,
    request_id: String,
    /// The backup decision once one was made.
    used: std::sync::Mutex<Option<RouteDecision>>,
}

impl InferenceFallback {
    /// The backup decision and the call lowered for it, or `None`.
    async fn prepare(
        &self,
        failed_model: &str,
        reason: &str,
    ) -> Result<Option<(RouteDecision, ResolvedProviderCall)>, AiccError> {
        if self.used.lock().expect("fallback lock").is_some() {
            return Ok(None);
        }
        let mut failed = self.decision.selected.clone();
        if failed.exact_model != failed_model {
            return Ok(None);
        }
        failed.exact_model = failed_model.to_owned();
        let Some(mut decision) = self
            .context
            .backup_route(&failed)
            .map_err(|error| AiccError::new(AiccErrorCode::NoCandidateModel, error.to_string()))?
        else {
            return Ok(None);
        };
        if let Some(fallback) = decision.trace.fallback.as_mut() {
            fallback.reason = Some(reason.to_owned());
        }
        decision.trace.usecase = self.decision.trace.usecase.clone();
        decision.trace.requested_model = self.decision.trace.requested_model.clone();
        let rpc = |error: RPCErrors| {
            AiccError::from_krpc_error(&error)
                .unwrap_or_else(|| AiccError::new(AiccErrorCode::InternalError, error.to_string()))
        };
        let exact_call = call_with_exact_model(&self.call, &decision.selected.exact_model).map_err(rpc)?;
        let mut lowered = self
            .port
            .lower_call(self.context.snapshot.as_ref(), &decision, &exact_call, &self.video_facts)
            .await
            .map_err(rpc)?;
        self.port
            .materialize_resources(
                self.context.snapshot.settings.as_ref(),
                &self.caller,
                &self.request_id,
                &mut lowered,
                &mut Vec::new(),
            )
            .await
            .map_err(rpc)?;
        lowered.fallback_from = Some(failed_model.to_owned());
        *self.used.lock().expect("fallback lock") = Some(decision.clone());
        log::warn!(
            "unavailable_fallback: from={} to={} policy={:?} reason={reason}",
            failed_model,
            decision.selected.exact_model,
            self.context.on_unavailable()
        );
        Ok(Some((decision, lowered)))
    }
}

#[async_trait]
impl crate::execution::UnavailableFallback for InferenceFallback {
    async fn backup(
        &self,
        failed: &ResolvedProviderCall,
        reason: &str,
    ) -> Result<Option<ResolvedProviderCall>, AiccError> {
        Ok(self
            .prepare(&failed.exact_model, reason)
            .await?
            .map(|(_, call)| call))
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn named_object_urls_require_explicit_public_base_url() {
        let obj_id = ndn_lib::ObjId::new_by_raw("chunk".to_owned(), vec![1; 32]);
        let settings = AiccSettings::default();
        assert!(named_object_url_provider(&settings)
            .object_url(&obj_id)
            .is_none());

        let settings = AiccSettings {
            public_named_object_base_url: Some("https://zone.example/ndn".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            named_object_url_provider(&settings).object_url(&obj_id),
            Some(format!("https://zone.example/ndn/{}", obj_id.to_string()))
        );
    }

    #[test]
    fn exact_chat_search_requires_explicit_boolean_opt_in() {
        for enabled in [None, Some(false), Some(true)] {
            let mut params = serde_json::json!({
                "exact_model": "claude-sonnet-5:reasoning-high@claude-default",
                "messages": [{"role": "user", "content": [{"type": "text", "text": "搜索巴克云"}]}]
            });
            if let Some(enabled) = enabled {
                params["web_search"] = Value::Bool(enabled);
            }
            let request = buckyos_api::LlmChatInvokeRequest::from_json(params).unwrap();
            let route = route_input_for_call(&AiccCall::ChatCompletionsCreate(request)).unwrap();
            assert_eq!(route.requirements.web_search, enabled == Some(true));
            assert!(!route.disable.web_search);
        }
        assert!(
            buckyos_api::LlmChatInvokeRequest::from_json(serde_json::json!({
                "exact_model": "claude-sonnet-5:reasoning-high@claude-default",
                "messages": [], "web_search": "true"
            }))
            .is_err()
        );
    }
}

/// The first video demand an exact model's contract cannot meet.
fn exact_video_unmet(
    models: &crate::model::ModelRegistry,
    exact_model: &str,
    requirement: Option<&buckyos_api::VideoGenerationRequirement>,
    clips: &crate::video::ClipSummary,
) -> Option<crate::video::Unmet> {
    let model = models.exact_model(exact_model)?;
    let contract = crate::video::VideoGenerationContract::from_capabilities(&model.capabilities)?;
    crate::video::unmet_requirements(&contract, requirement?, clips)
        .into_iter()
        .next()
}

/// The exact method a helper call executes as; other calls execute as
/// themselves.
fn helper_exact_method(call: &AiccCall) -> Option<&'static str> {
    Some(match call {
        AiccCall::HelperLlmChat(_) => buckyos_api::ai_methods::CHAT_COMPLETIONS_CREATE,
        AiccCall::HelperTextToImage(_) => buckyos_api::ai_methods::IMAGES_GENERATE,
        AiccCall::HelperVideoGenerate(_) => buckyos_api::ai_methods::VIDEO_GENERATE,
        _ => return None,
    })
}

fn exact_method(call: &AiccCall) -> &'static str {
    helper_exact_method(call).unwrap_or_else(|| call.method())
}

fn exact_call_for_route(call: AiccCall, exact_model: &str) -> Result<AiccCall, RPCErrors> {
    let Some(method) = helper_exact_method(&call) else {
        return Ok(call);
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
        "route_group",
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

fn inherited_api_type(api_types: &BTreeMap<String, String>, path: &str) -> Option<String> {
    let mut current = Some(path);
    while let Some(path) = current {
        if let Some(api_type) = api_types.get(path) {
            return Some(api_type.clone());
        }
        current = path.rfind('.').map(|index| &path[..index]);
    }
    None
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
        route_trace: public_route_trace(decision),
        inventory_revision: selected.inventory_revision.clone(),
    }
}

fn public_route_trace(decision: &RouteDecision) -> RouteTrace {
    RouteTrace {
        attempts: Vec::new(),
        final_model: Some(decision.selected.exact_model.clone()),
        fallback: decision.trace.fallback.clone(),
    }
}

fn public_route_trace_json(decision: &RouteDecision) -> Result<Value, RPCErrors> {
    serde_json::to_value(&decision.trace)
        .map_err(|error| inference_error(AiccErrorCode::InternalError, error.to_string()))
}

fn inference_response(
    receipt: crate::execution::ExecutionReceipt,
    route_trace: RouteTrace,
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
        serde_json::to_value(route_trace).expect("RouteTrace serializes"),
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

    /// Also the inventory the Provider would add, for the usecase coverage.
    async fn validate_with_inventory(
        &self,
        request: ProviderValidateRequest,
    ) -> Result<(ProviderValidateResponse, Option<crate::model::ProviderInventory>), RPCErrors>
    {
        Ok((self.validate(request).await?, None))
    }
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
pub(crate) trait DriverMetadataPort: Send + Sync {
    async fn get(&self) -> Result<DriverMetadataUpdateView, RPCErrors>;
    async fn set(
        &self,
        token: &str,
        expected_settings_revision: u64,
        request: DriverMetadataUpdateSetReq,
    ) -> Result<DriverMetadataUpdateSetResponse, RPCErrors>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_url_input_policy_uses_instance_and_protocol_form() {
        let base = Some("https://provider.example/v1");
        for (url, source_base) in [
            ("https://provider.example/result.png", base),
            ("https://cdn.example/result.png?signature=x", base),
            ("https://provider.example/result.png", None),
        ] {
            for target in ["source", "other"] {
                assert!(provider_url_requires_bytes(
                    ResourceInputForm::BytesOnly, url, "source", target, source_base
                ));
                assert!(!provider_url_requires_bytes(
                    ResourceInputForm::UrlOnly, url, "source", target, source_base
                ));
            }
            assert!(!provider_url_requires_bytes(
                ResourceInputForm::UrlOrBytes, url, "source", "source", source_base
            ));
        }
        assert!(provider_url_requires_bytes(
            ResourceInputForm::UrlOrBytes,
            "https://provider.example/result.png", "source", "other", base
        ));
        assert!(!provider_url_requires_bytes(
            ResourceInputForm::UrlOrBytes,
            "https://cdn.example/result.png?signature=x", "source", "other", base
        ));
        for missing_base in [None, Some("")] {
            assert!(provider_url_requires_bytes(
                ResourceInputForm::UrlOrBytes,
                "https://provider.example/result.png", "source", "other", missing_base
            ));
        }
    }

    #[test]
    fn resource_materialization_cache_is_scoped_by_codec_input_form() {
        let resources = vec![ResourceRef::url(
            "https://cdn.example.com/audio.mp3".to_owned(),
            None,
        )];
        let cache: Vec<(
            ResourceInputForm,
            Vec<ResourceRef>,
            Vec<RawMaterializedResource>,
        )> = vec![(ResourceInputForm::UrlOrBytes, resources.clone(), Vec::new())];

        assert!(
            cached_materialization(&cache, ResourceInputForm::UrlOrBytes, &resources).is_some()
        );
        assert!(cached_materialization(&cache, ResourceInputForm::BytesOnly, &resources).is_none());
    }
}
