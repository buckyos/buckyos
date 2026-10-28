use super::model_defaults::{builtin_logical_model_definitions, builtin_logical_tree_overlay};
use super::*;
use crate::model::ModelRegistry;
use buckyos_api::ApiType;

pub(crate) struct ServiceRuntimeFactory {
    storage: Arc<AiccStorage>,
    events: Arc<AiccEventLog>,
    provider_refreshes: broadcast::Sender<ProviderRefreshEvent>,
}

impl ServiceRuntimeFactory {
    pub(crate) fn new(storage: Arc<AiccStorage>, events: Arc<AiccEventLog>) -> Self {
        let (provider_refreshes, _) = broadcast::channel(64);
        Self {
            storage,
            events,
            provider_refreshes,
        }
    }

    pub(crate) fn subscribe_provider_refreshes(&self) -> broadcast::Receiver<ProviderRefreshEvent> {
        self.provider_refreshes.subscribe()
    }
}

#[async_trait]
impl RuntimeFactory for ServiceRuntimeFactory {
    async fn prepare(
        &self,
        settings: Arc<AiccSettings>,
        catalog: Arc<CatalogSnapshot>,
        target_seq: u64,
    ) -> Result<PreparedRuntime, RuntimeError> {
        let mut effective_catalog = (*catalog).clone();
        for provider in &settings.providers {
            if let Some(value) = provider
                .instance_rules
                .as_ref()
                .and_then(|rules| rules.catalog.as_ref())
            {
                if provider.provider_profile_id != "custom" {
                    return Err(RuntimeError::Backend(
                        "instance catalogs are only supported for custom providers".into(),
                    ));
                }
                effective_catalog = effective_catalog
                    .with_instance_catalog(&provider.provider_instance_name, value)
                    .map_err(|error| RuntimeError::Backend(error.to_string()))?;
            }
        }
        let catalog = Arc::new(effective_catalog);
        let builtins = builtin_provider_registry(catalog.as_ref())
            .map_err(|error| RuntimeError::Backend(error.to_string()))?;
        let (resolver, auth) = settings_credentials(settings.as_ref());
        let static_resolver: Arc<dyn CredentialResolver> = Arc::new(resolver);
        let credential_broker = Arc::new(SnCredentialBroker::new(
            static_resolver,
            builtins.dynamic_login_resolver(),
        ));
        let manager = Arc::new(
            ProviderRuntimeManager::new(
                builtins.profiles().cloned().collect::<Vec<_>>(),
                credential_broker.clone(),
                catalog.clone(),
                builtins.codecs(),
                self.storage.clone(),
            )
            .map_err(|error| RuntimeError::Backend(error.to_string()))?,
        );
        let mut manager_events = manager.subscribe_refresh_events();
        let provider_refreshes = self.provider_refreshes.clone();
        tokio::spawn(async move {
            loop {
                match manager_events.recv().await {
                    Ok(event) => {
                        let _ = provider_refreshes.send(event);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        let mut startup_failures = BTreeMap::new();
        for provider in settings
            .providers
            .iter()
            .filter(|provider| provider.enabled)
        {
            let started: Result<(), RuntimeError> = async {
                builtins
                    .codecs()
                    .resolve_adapter_id(
                        provider.protocol_family_id.as_deref(),
                        Some(&provider.protocol_adapter_id),
                        None,
                    )
                    .map_err(|error| RuntimeError::Backend(error.to_string()))?;
                let provider_auth = auth
                    .get(&provider.provider_instance_name)
                    .ok_or_else(|| {
                        RuntimeError::Backend(
                            "provider authentication was not prepared".to_string(),
                        )
                    })?
                    .as_ref()
                    .map_err(|error| RuntimeError::Backend(error.clone()))?;
                let configured_inventory = provider
                    .discovery
                    .as_ref()
                    .map(provider_discovery_snapshot)
                    .transpose()
                    .map_err(|error| RuntimeError::Backend(error.to_string()))?;
                let binding = builtins
                    .resolve(BuiltinProviderRequest {
                        provider_profile_id: &provider.provider_profile_id,
                        protocol_adapter_id: &provider.protocol_adapter_id,
                        auth_mode: provider_auth.mode(),
                        credential_kind: provider_auth.credential_kind(),
                        configured_inventory,
                    })
                    .map_err(|error| RuntimeError::Backend(error.to_string()))?;
                let connection = binding
                    .connection
                    .resolve(ProviderConnectionInput {
                        base_url: Some(&provider.base_url),
                        region: provider.region.as_deref(),
                        workspace: provider.workspace.as_deref(),
                        account: provider.account.as_deref(),
                        operation_base_urls: Some(&provider.operation_base_urls),
                    })
                    .map_err(|error| RuntimeError::Backend(error.to_string()))?;
                let provider_rules_id = provider
                    .instance_rules
                    .as_ref()
                    .and_then(|rules| rules.catalog.as_ref())
                    .map(|_| CatalogSnapshot::instance_rules_id(&provider.provider_instance_name))
                    .or_else(|| provider.provider_rules_id.clone())
                    .or_else(|| {
                        catalog
                            .resolve_provider_configuration(&provider.provider_profile_id)
                            .ok()
                            .map(|configuration| configuration.provider_rules_id)
                    });
                let request_timeout = provider
                    .timeout_ms
                    .map(Duration::from_millis)
                    .unwrap_or_else(|| HttpTransportConfig::default().request_timeout);
                let mut runtime_config = match provider_auth {
                    ProviderAuthConfig::ApiKey {
                        credential_ref,
                        credential_kind,
                    } => ProviderInstanceConfig {
                        provider_instance_name: provider.provider_instance_name.clone(),
                        provider_profile_id: provider.provider_profile_id.clone(),
                        protocol_adapter_id: provider.protocol_adapter_id.clone(),
                        base_url: connection.base_url,
                        operation_base_urls: connection.operation_base_urls,
                        credential: CredentialReference {
                            reference: credential_ref.clone(),
                        },
                        credential_kind: *credential_kind,
                        provider_rules_id,
                        region: connection.region,
                        workspace: connection.workspace,
                        account: connection.account,
                        request_timeout,
                        auto_sync_models: provider.auto_sync_models.unwrap_or(true),
                        instance_rules: provider
                            .instance_rules
                            .clone()
                            .or(binding.instance_rules.clone()),
                    },
                    ProviderAuthConfig::DynamicLogin { .. } => {
                        let resolved = resolve_sn_provider_instance_with_config(
                            &binding.profile,
                            &binding.connection,
                            provider_rules_id,
                            SnProviderInstanceInput {
                                provider_instance_name: &provider.provider_instance_name,
                                base_url: Some(&provider.base_url),
                                account: provider.account.as_deref(),
                                auth: provider_auth.clone(),
                            },
                        )
                        .map_err(|error| RuntimeError::Backend(error.to_string()))?;
                        credential_broker
                            .register_dynamic_instance(resolved.clone())
                            .await
                            .map_err(|error| RuntimeError::Backend(error.to_string()))?;
                        resolved.runtime
                    }
                };
                runtime_config.request_timeout = request_timeout;
                runtime_config.auto_sync_models = provider.auto_sync_models.unwrap_or(true);
                runtime_config.instance_rules = provider
                    .instance_rules
                    .clone()
                    .or(binding.instance_rules.clone());
                if let Some(schema) = &binding.connection.policy_region {
                    let policy_region = schema
                        .resolve(
                            "policy_region",
                            runtime_config
                                .instance_rules
                                .as_ref()
                                .and_then(|rules| rules.policy_region.as_deref()),
                        )
                        .map_err(|error| RuntimeError::Backend(error.to_string()))?;
                    runtime_config
                        .instance_rules
                        .get_or_insert_with(Default::default)
                        .policy_region = policy_region;
                }
                manager
                    .start(runtime_config, binding.discovery)
                    .await
                    .map_err(|error| RuntimeError::Backend(error.to_string()))?;
                Ok(())
            }
            .await;
            if let Err(error) = started {
                log::warn!(
                    "provider {} disabled at runtime: {}",
                    provider.provider_instance_name,
                    error
                );
                self.events.record(buckyos_api::AiccEventLevel::Warning, "provider_configuration_error",
                    format!("Provider {} disabled at runtime: {error}", provider.provider_instance_name),
                    json!({"provider_instance_name": provider.provider_instance_name, "error": error.to_string()}));
                startup_failures.insert(provider.provider_instance_name.clone(), error.to_string());
            }
        }
        let models: Arc<dyn ModelRegistryAssembler> = Arc::new(ServiceModelAssembler {
            session: settings.session_config.clone(),
            events: Some(self.events.clone()),
        });
        let backend: Arc<dyn RuntimeBackend> = Arc::new(ProviderRuntimeBackend::new(
            manager,
            models,
            startup_failures,
        ));
        let state = backend
            .converge(catalog, target_seq, ConvergenceTrigger::MetadataRefresh)
            .await?;
        Ok(PreparedRuntime { backend, state })
    }
}

fn settings_credentials(
    settings: &AiccSettings,
) -> (
    StaticCredentialResolver,
    BTreeMap<String, Result<ProviderAuthConfig, String>>,
) {
    let mut values = BTreeMap::new();
    let mut auth = BTreeMap::new();
    for provider in settings
        .providers
        .iter()
        .filter(|provider| provider.enabled)
    {
        let parsed = (|| -> Result<ProviderAuthConfig, RPCErrors> {
            let parsed = provider.auth.as_ref().map(provider_auth_config);
            let parsed = match parsed {
                Some(value) => {
                    if let ProviderAuthConfig::ApiKey { credential_ref, .. } = &value {
                        let (_, secret) = first_resolvable_credential(
                            &provider.provider_instance_name,
                            &provider.credentials,
                        )?;
                        values.insert(credential_ref.clone(), secret);
                    }
                    value
                }
                None => {
                    let (reference, secret) = first_resolvable_credential(
                        &provider.provider_instance_name,
                        &provider.credentials,
                    )?;
                    values.insert(reference.clone(), secret);
                    ProviderAuthConfig::ApiKey {
                        credential_ref: reference,
                        credential_kind: None,
                    }
                }
            };
            Ok(parsed)
        })()
        .map_err(|error| error.to_string());
        auth.insert(provider.provider_instance_name.clone(), parsed);
    }
    (StaticCredentialResolver::new(values), auth)
}

pub(super) fn provider_auth_config(settings: &ProviderAuthSettings) -> ProviderAuthConfig {
    match settings {
        ProviderAuthSettings::ApiKey {
            credential_ref,
            credential_kind,
        } => ProviderAuthConfig::ApiKey {
            credential_ref: credential_ref.clone(),
            credential_kind: credential_kind.map(|kind| match kind {
                ApiProviderCredentialKind::Bearer => CredentialKind::Bearer,
                ApiProviderCredentialKind::NamedHeader => CredentialKind::NamedHeader,
                ApiProviderCredentialKind::FalKey => CredentialKind::FalKey,
                ApiProviderCredentialKind::GlmJwt => CredentialKind::GlmJwt,
            }),
        },
        ProviderAuthSettings::DynamicLogin {
            login_profile,
            login_endpoint,
        } => ProviderAuthConfig::DynamicLogin {
            login_profile: login_profile.clone(),
            login_endpoint: login_endpoint.clone(),
        },
    }
}

fn provider_discovery_snapshot(
    settings: &ProviderDiscoverySettings,
) -> Result<ProviderDiscoverySnapshot, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(settings)?)
}

fn first_resolvable_credential(
    instance: &str,
    credentials: &ProviderCredentials,
) -> Result<(String, String), RPCErrors> {
    for (name, value) in credentials {
        match value {
            ProviderCredential::InlineSecret(secret) if !secret.is_empty() => {
                return Ok((format!("inline-secret://{instance}/{name}"), secret.clone()));
            }
            ProviderCredential::SecretRef(reference) if !reference.is_empty() => {
                return Err(RPCErrors::ReasonError(format!(
                    "secret_ref credential `{reference}` is not supported until a secret resolver is configured"
                )));
            }
            ProviderCredential::RuntimeRef(reference) if !reference.is_empty() => {
                return Err(RPCErrors::ReasonError(format!(
                    "runtime_ref credential `{reference}` cannot be used for static API key authentication"
                )));
            }
            _ => {}
        }
    }
    Err(RPCErrors::ReasonError(
        "no resolvable credential was provided".to_string(),
    ))
}

pub(crate) struct RuntimeProviderValidator {
    runtime: Arc<RuntimeState>,
    storage: Arc<AiccStorage>,
}

impl RuntimeProviderValidator {
    pub(crate) fn new(runtime: Arc<RuntimeState>, storage: Arc<AiccStorage>) -> Self {
        Self { runtime, storage }
    }
}

#[async_trait]
impl ProviderValidator for RuntimeProviderValidator {
    async fn validate(
        &self,
        request: ProviderValidateRequest,
    ) -> Result<ProviderValidateResponse, RPCErrors> {
        let snapshot = self.runtime.capture().await;
        let builtins =
            builtin_provider_registry(snapshot.catalog.as_ref()).map_err(to_rpc_error)?;
        let provider_name = request
            .provider_instance_name
            .clone()
            .unwrap_or_else(|| "provider-validation".to_string());
        let profile = builtins
            .profiles()
            .find(|profile| profile.provider_profile_id == request.provider_profile_id);
        let profile_default = profile
            .filter(|profile| !profile.accepts_any_adapter)
            .map(|profile| profile.default_protocol_adapter_id.as_str());
        if profile.is_some_and(|profile| profile.accepts_any_adapter)
            && request.protocol_adapter_id.is_none()
        {
            return Err(RPCErrors::ReasonError(
                "custom providers require an explicit protocol adapter".to_owned(),
            ));
        }
        let adapter = builtins
            .codecs()
            .resolve_adapter_id(
                request.protocol_family_id.as_deref(),
                request.protocol_adapter_id.as_deref(),
                profile_default,
            )
            .map_err(to_rpc_error)?;
        let auth = request.auth.as_ref().map(provider_auth_config);
        let (auth, credentials) = match auth {
            Some(auth @ ProviderAuthConfig::DynamicLogin { .. }) => (auth, BTreeMap::new()),
            Some(auth @ ProviderAuthConfig::ApiKey { .. }) => {
                let credential_ref = match &auth {
                    ProviderAuthConfig::ApiKey { credential_ref, .. } => credential_ref.clone(),
                    ProviderAuthConfig::DynamicLogin { .. } => unreachable!(),
                };
                let (_, secret) =
                    first_resolvable_credential(&provider_name, &request.credentials)?;
                (auth, BTreeMap::from([(credential_ref, secret)]))
            }
            None => {
                let (reference, secret) =
                    first_resolvable_credential(&provider_name, &request.credentials)?;
                (
                    ProviderAuthConfig::ApiKey {
                        credential_ref: reference.clone(),
                        credential_kind: None,
                    },
                    BTreeMap::from([(reference, secret)]),
                )
            }
        };
        let configured_inventory = request
            .discovery
            .as_ref()
            .map(provider_discovery_snapshot)
            .transpose()
            .map_err(to_rpc_error)?;
        let mut catalog = snapshot.catalog.clone();
        let mut rules_id = request.provider_rules_id.clone();
        if let Some(value) = request
            .instance_rules
            .as_ref()
            .and_then(|rules| rules.catalog.as_ref())
        {
            if request.provider_profile_id != "custom" {
                return Err(RPCErrors::ReasonError(
                    "instance catalogs are only supported for custom providers".into(),
                ));
            }
            catalog = Arc::new(
                catalog
                    .with_instance_catalog(&provider_name, value)
                    .map_err(to_rpc_error)?,
            );
            rules_id = Some(CatalogSnapshot::instance_rules_id(&provider_name));
        }
        let binding = builtins
            .resolve(BuiltinProviderRequest {
                provider_profile_id: &request.provider_profile_id,
                protocol_adapter_id: &adapter,
                auth_mode: auth.mode(),
                credential_kind: auth.credential_kind(),
                configured_inventory,
            })
            .map_err(to_rpc_error)?;
        let manager = ProviderRuntimeManager::new(
            builtins.profiles().cloned().collect::<Vec<_>>(),
            Arc::new(StaticCredentialResolver::new(credentials)),
            catalog.clone(),
            builtins.codecs(),
            self.storage.clone(),
        )
        .map_err(to_rpc_error)?;
        let draft = ProviderDraftConfig {
            provider_instance_name: provider_name,
            provider_profile_id: request.provider_profile_id,
            protocol_adapter_id: adapter.clone(),
            provider_rules_id: rules_id,
            instance_rules: request.instance_rules.or(binding.instance_rules.clone()),
            base_url: Some(request.base_url),
            operation_base_urls: request.operation_base_urls,
            region: request.region,
            workspace: request.workspace,
            account: request.account.clone(),
            auth,
            dynamic_login_user_name: request.account,
        };
        match manager
            .validate_draft(
                &draft,
                &binding.connection,
                binding.discovery.as_ref(),
                binding.dynamic_login_resolver.as_deref(),
            )
            .await
        {
            Ok(negotiated) => Ok(provider_inventory_validation(
                &catalog,
                &negotiated.inventory,
                negotiated.protocol_adapter_id,
            )),
            Err(error) => {
                let kind = match error.stage {
                    ProviderDraftValidationStage::Connection => {
                        buckyos_api::ProviderValidationErrorKind::BaseUrl
                    }
                    ProviderDraftValidationStage::Authentication => {
                        buckyos_api::ProviderValidationErrorKind::Authentication
                    }
                    ProviderDraftValidationStage::Protocol => {
                        buckyos_api::ProviderValidationErrorKind::Protocol
                    }
                    ProviderDraftValidationStage::Discovery
                    | ProviderDraftValidationStage::Inventory => {
                        buckyos_api::ProviderValidationErrorKind::Models
                    }
                };
                let message = format!(
                    "provider validation failed at {:?}: {}",
                    error.stage, error.message
                );
                let provider_name =
                    crate::observability::redact(&json!(draft.provider_instance_name));
                log::warn!("provider {provider_name}: {message}");
                Ok(ProviderValidateResponse {
                    base_url_reachable: !matches!(
                        error.stage,
                        ProviderDraftValidationStage::Connection
                    ),
                    auth_valid: matches!(
                        error.stage,
                        ProviderDraftValidationStage::Discovery
                            | ProviderDraftValidationStage::Inventory
                    ),
                    models_discovered: Vec::new(),
                    balance_available: false,
                    errors: vec![message.clone()],
                    error_details: vec![buckyos_api::ProviderValidationErrorDetail {
                        kind,
                        message,
                    }],
                    resolved_protocol_adapter_id: Some(adapter),
                    ..Default::default()
                })
            }
        }
    }
}

fn provider_inventory_validation(
    catalog: &CatalogSnapshot,
    inventory: &crate::provider::ProviderInventorySnapshot,
    adapter: String,
) -> ProviderValidateResponse {
    let recognized = inventory
        .models
        .iter()
        .map(|model| model.provider_model_id.clone())
        .collect::<Vec<_>>();
    let mut discovered = recognized.iter().cloned().collect::<BTreeSet<_>>();
    discovered.extend(
        inventory
            .unmatched_models
            .iter()
            .map(|model| model.provider_model_id.clone()),
    );
    let mut response = ProviderValidateResponse {
        base_url_reachable: true,
        auth_valid: true,
        balance_available: false,
        models_discovered: discovered.into_iter().collect(),
        models_recognized: recognized,
        filtered_capabilities: inventory.filtered_capabilities.clone(),
        unmatched_models: inventory
            .unmatched_models
            .iter()
            .map(|model| json!(model))
            .collect(),
        unavailable_presets: inventory
            .unavailable_presets
            .iter()
            .map(|preset| json!(preset))
            .collect(),
        resolved_protocol_adapter_id: Some(adapter),
        ..Default::default()
    };
    let factory = builtin_logical_tree_overlay();
    let registry = match ModelRegistry::build(
        catalog,
        &[inventory.as_model_inventory()],
        builtin_logical_model_definitions(),
        crate::model::RegistryLayers {
            factory: Some(&factory),
            ..Default::default()
        },
    ) {
        Ok(registry) => registry,
        Err(error) => {
            response.errors.push(error.to_string());
            response
                .error_details
                .push(buckyos_api::ProviderValidationErrorDetail {
                    kind: buckyos_api::ProviderValidationErrorKind::Configuration,
                    message: error.to_string(),
                });
            return response;
        }
    };
    for view in registry.logical_model_views().into_iter().filter(|view| {
        view.kind == buckyos_api::AiccLogicalDirectoryKind::Task
            && view.api_type.as_deref() == Some("llm")
    }) {
        let candidates = registry
            .resolve_available_candidates(&view.path, ApiType::Llm, &mut |_, _| true)
            .map(|expanded| {
                expanded
                    .candidates
                    .into_iter()
                    .map(|candidate| candidate.model.exact_model.to_string())
                    .collect()
            })
            .unwrap_or_default();
        response.routing_candidates.insert(view.path, candidates);
    }
    if !response.unmatched_models.is_empty() {
        response
            .warnings
            .push("Some discovered models need an explicit model identity mapping".into());
    }
    if !response.unavailable_presets.is_empty() {
        response
            .warnings
            .push("Some manufacturer presets have no mapping on this channel".into());
    }
    if inventory
        .models
        .iter()
        .any(|model| model.api_types.contains(&ApiType::Llm))
        && response
            .routing_candidates
            .get("llm.chat")
            .is_none_or(Vec::is_empty)
    {
        response.warnings.push(
            "No llm.chat candidate: check channel presets and specification admission".into(),
        );
    }
    response
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    use crate::error::SettingsError;
    use crate::settings::RuntimeInputs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct BuiltinRuntimeInputs {
        catalog: Arc<CatalogSnapshot>,
    }

    #[async_trait]
    impl RuntimeInputs for BuiltinRuntimeInputs {
        async fn metadata_target_seq(&self) -> Result<u64, SettingsError> {
            Ok(self.catalog.target_revision_seq())
        }

        async fn load_catalog(
            &self,
            _target_seq: u64,
        ) -> Result<Arc<CatalogSnapshot>, SettingsError> {
            Ok(self.catalog.clone())
        }
    }

    #[test]
    fn readiness_separates_identity_presets_and_chat_admission() {
        use crate::provider::{CredentialReference, ProviderInstanceConfig};
        use crate::provider::{
            DiscoveredModel, InventoryBuilder, ModelAvailability, ProviderDiscoverySnapshot,
            ProviderHealthState,
        };
        use crate::settings::MetadataSources;
        let catalog = MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let providers = builtin_provider_registry(&catalog).unwrap();
        let mut profile = providers
            .profiles()
            .find(|profile| profile.provider_profile_id == "sn")
            .unwrap()
            .clone();
        profile.provider_profile_id = "custom".into();
        let config = ProviderInstanceConfig {
            provider_instance_name: "readiness".into(),
            provider_profile_id: "custom".into(),
            protocol_adapter_id: "sn-openai".into(),
            base_url: "https://sn.example/api/v1/ai".into(),
            operation_base_urls: Default::default(),
            credential: CredentialReference {
                reference: "test".into(),
            },
            credential_kind: None,
            provider_rules_id: None,
            region: None,
            workspace: None,
            account: None,
            request_timeout: std::time::Duration::from_secs(30),
            auto_sync_models: true,
            instance_rules: None,
        };
        let discovery = ProviderDiscoverySnapshot {
            revision: None,
            discovered_at_ms: 1,
            health: ProviderHealthState::Healthy,
            models: ["qwen3.8-max-0902", "unknown-model"]
                .into_iter()
                .map(|id| DiscoveredModel {
                    provider_model_id: id.into(),
                    api_types: None,
                    supported_features: None,
                    unsupported_features: Default::default(),
                    remote_methods: None,
                    availability: ModelAvailability::Available,
                    deprecated: false,
                    pricing: None,
                    video: None,
                })
                .collect(),
        };
        let inventory =
            InventoryBuilder::build(&profile, &config, discovery, &catalog, &providers.codecs())
                .unwrap();
        let mut inventory = inventory;
        inventory
            .filtered_capabilities
            .push(buckyos_api::FilteredProviderCapability {
                provider_model_id: "qwen3.8-max-0902".into(),
                protocol_adapter_id: "sn-openai".into(),
                api_type: "image.img2img".into(),
                operation_id: Some("images.generate".into()),
                reason: "unregistered_binding".into(),
            });
        let response = provider_inventory_validation(&catalog, &inventory, "sn-openai".into());
        assert_eq!(
            response.filtered_capabilities,
            inventory.filtered_capabilities
        );
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(
            wire["filtered_capabilities"][0]["api_type"],
            "image.img2img"
        );
        assert!(response.errors.is_empty());
        assert_eq!(response.models_discovered.len(), 2);
        assert_eq!(response.models_recognized, ["qwen3.8-max-0902"]);
        assert_eq!(response.unmatched_models.len(), 1);
        assert_eq!(response.unavailable_presets.len(), 4);
        assert!(response.routing_candidates["llm.chat"].is_empty());
        assert_eq!(response.warnings.len(), 3);
    }

    #[tokio::test]
    async fn custom_validation_requires_explicit_adapter_without_probing() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 4096];
                let read = stream.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /v1/models "));
                let body = r#"{"data":[{"id":"gpt-6-astra"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let storage = Arc::new(
            AiccStorage::open("sqlite::memory:", buckyos_api::RdbBackend::Sqlite)
                .await
                .unwrap(),
        );
        let runtime = RuntimeState::bootstrap(
            SettingsDocument::new(1, AiccSettings::default()).unwrap(),
            Arc::new(BuiltinRuntimeInputs { catalog }),
            Arc::new(ServiceRuntimeFactory::new(
                storage.clone(),
                Arc::new(AiccEventLog::default()),
            )),
        )
        .await
        .unwrap();
        let validator = RuntimeProviderValidator::new(runtime.clone(), storage);
        let mut request = ProviderValidateRequest::new(
            ProviderInstanceType::CloudApi,
            "custom",
            format!("http://{address}/v1"),
            serde_json::from_value(json!({"api_token": {"inline_secret": "mock-key"}})).unwrap(),
        );
        request.protocol_family_id = Some("openai".into());
        assert!(validator
            .validate(request.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("explicit protocol adapter"));
        for adapter in ["openai-chat-completions", "openai-responses"] {
            request.protocol_adapter_id = Some(adapter.into());
            let response = validator.validate(request.clone()).await.unwrap();
            assert!(response.errors.is_empty(), "{:?}", response.errors);
            assert_eq!(
                response.resolved_protocol_adapter_id.as_deref(),
                Some(adapter)
            );
            assert_eq!(response.models_discovered, ["gpt-6-astra"]);
        }
        server.await.unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn openrouter_validation_preserves_discovery_price_error_details() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (prompt, completion) in [("0.0000000341", "0.000007"), ("1", "501")] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 4096];
                let read = stream.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..read])
                    .starts_with("GET /api/v1/models?output_modalities=all "));
                let body = json!({
                    "data": [{
                        "id": "z-ai/glm-5.2",
                        "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]},
                        "supported_parameters": ["tools", "response_format"],
                        "pricing": {"prompt": prompt, "completion": completion}
                    }]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let storage = Arc::new(
            AiccStorage::open("sqlite::memory:", buckyos_api::RdbBackend::Sqlite)
                .await
                .unwrap(),
        );
        let runtime = RuntimeState::bootstrap(
            SettingsDocument::new(1, AiccSettings::default()).unwrap(),
            Arc::new(BuiltinRuntimeInputs { catalog }),
            Arc::new(ServiceRuntimeFactory::new(
                storage.clone(),
                Arc::new(AiccEventLog::default()),
            )),
        )
        .await
        .unwrap();
        let validator = RuntimeProviderValidator::new(runtime.clone(), storage);
        let request = ProviderValidateRequest::new(
            ProviderInstanceType::CloudApi,
            "openrouter",
            format!("http://{address}/api/v1"),
            serde_json::from_value(json!({
                "api_token": {"inline_secret": "sk-test-validation-secret"}
            }))
            .unwrap(),
        );

        let valid = validator.validate(request.clone()).await.unwrap();
        assert!(valid.errors.is_empty(), "{:?}", valid.errors);
        assert!(valid.error_details.is_empty());
        assert_eq!(valid.models_discovered, ["z-ai/glm-5.2"]);

        let invalid = validator.validate(request).await.unwrap();
        assert!(invalid.models_discovered.is_empty());
        assert_eq!(invalid.errors.len(), 1);
        assert_eq!(invalid.error_details.len(), 1);
        assert_eq!(
            invalid.error_details[0].kind,
            buckyos_api::ProviderValidationErrorKind::Models
        );
        assert_eq!(invalid.error_details[0].message, invalid.errors[0]);
        assert!(invalid.errors[0].contains("provider validation failed at Discovery"));
        assert!(invalid.errors[0].contains("z-ai/glm-5.2"));
        assert!(invalid.errors[0].contains("pricing.ratio_exception"));
        assert!(invalid.errors[0].contains("501.00 exceeds 500"));
        assert!(!serde_json::to_string(&invalid)
            .unwrap()
            .contains("sk-test-validation-secret"));
        server.await.unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn disabled_custom_provider_keeps_cached_discovery_and_its_saved_catalog() {
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }.build_snapshot(crate::settings::BUILTIN_CATALOG_REVISION_SEQ, &Default::default()).unwrap();
        let codecs = builtin_provider_registry(&catalog).unwrap().codecs();
        let mut rules = serde_json::to_value(catalog.provider_rules("openai").unwrap()).unwrap();
        rules["model_variants"] = json!({"local-model":[]});
        let provider: ProviderSettings = serde_json::from_value(json!({
            "provider_instance_name":"disabled-custom",
            "provider_type":"cloud_api",
            "provider_profile_id":"custom",
            "protocol_family_id":"openai",
            "protocol_adapter_id":"openai-responses",
            "base_url":"https://unused.example/v1",
            "credentials":{"api_token":{"inline_secret":"not-used"}},
            "enabled":false,
            "instance_rules":{"catalog":rules}
        })).unwrap();
        let storage = Arc::new(AiccStorage::open("sqlite::memory:", buckyos_api::RdbBackend::Sqlite).await.unwrap());
        let record = crate::storage::InventoryLkgsRecord::new(
            "disabled-custom", "custom", "openai-responses", "previous", 1, None, 1,
            json!({"discovered_model_ids":["local-model","unknown-model"]}), 1
        ).unwrap();
        storage.upsert_inventory(&record).await.unwrap();
        let runtime = RuntimeState::bootstrap(
            SettingsDocument::new(1, AiccSettings {providers: vec![provider], ..Default::default()}).unwrap(),
            Arc::new(BuiltinRuntimeInputs { catalog }),
            Arc::new(ServiceRuntimeFactory::new(storage.clone(), Arc::new(AiccEventLog::default()))),
        ).await.unwrap();
        let adapter = RuntimeServiceAdapter::new(runtime.clone(), codecs, storage);
        let view = adapter.capture().await.unwrap();
        assert_eq!(view.providers[0].discovered_model_ids, BTreeSet::from(["local-model".into(), "unknown-model".into()]));
        assert_eq!(view.providers[0].rules_catalog.as_ref().unwrap()["model_variants"]["local-model"], json!([]));
        assert_eq!(view.providers[0].inventory.state, ProviderInstanceInventoryState::Disabled);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn custom_gemini_rules_view_uses_instance_inventory_without_builtin_prices() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let read = stream.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /v1beta/models"));
            let body = r#"{"models":[{"name":"models/gemini-4-argon"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }.build_snapshot(crate::settings::BUILTIN_CATALOG_REVISION_SEQ, &Default::default()).unwrap();
        assert!(!catalog.provider_rules("gemini").unwrap().model_pricing.is_empty());
        let codecs = builtin_provider_registry(&catalog).unwrap().codecs();
        let provider: ProviderSettings = serde_json::from_value(json!({
            "provider_instance_name":"my-gemini",
            "provider_type":"cloud_api",
            "provider_profile_id":"custom",
            "protocol_family_id":"gemini",
            "protocol_adapter_id":"gemini-interactions",
            "base_url":format!("http://{address}/v1beta"),
            "credentials":{"api_token":{"inline_secret":"local-test"}},
            "auto_sync_models":false
        })).unwrap();
        let storage = Arc::new(AiccStorage::open("sqlite::memory:", buckyos_api::RdbBackend::Sqlite).await.unwrap());
        let runtime = RuntimeState::bootstrap(
            SettingsDocument::new(1, AiccSettings { providers: vec![provider], ..Default::default() }).unwrap(),
            Arc::new(BuiltinRuntimeInputs { catalog }),
            Arc::new(ServiceRuntimeFactory::new(storage, Arc::new(AiccEventLog::default()))),
        ).await.unwrap();
        let snapshot = runtime.capture().await;
        let provider = snapshot.providers.get("my-gemini").expect("custom Gemini must load");
        let model = provider.inventory.models.iter().find(|model| model.provider_model_id == "gemini-4-argon").unwrap();
        assert!(model.pricing.is_none());
        let view = runtime_admin_snapshot(&snapshot, &codecs);
        assert!(view.providers[0].rules_catalog.is_none(), "unbound Gemini catalogs must not be returned");
        let resolved = &view.providers[0].resolved_model_rules["gemini-4-argon"];
        assert_eq!(resolved["pricing"], Value::Null);
        assert_eq!(resolved["operations"], serde_json::to_value(&model.operations).unwrap());
        let variants = resolved["variants"].as_array().unwrap();
        assert_eq!(resolved["variants"], serde_json::to_value(&model.variant_rules).unwrap());
        assert_eq!(variants.len(), model.variants.len());
        assert!(variants.iter().all(|variant| model.variants.iter().any(|actual| variant["variant"] == actual.name)));
        assert!(resolved["rule"].is_null(), "unbound request rules must not be imported");
        server.await.unwrap();
        runtime.shutdown().await;
    }

    #[test]
    fn credential_sources_are_resolved_explicitly() {
        let inline: ProviderCredentials = serde_json::from_value(json!({
            "api_token": {"inline_secret": "secret"}
        }))
        .unwrap();
        assert_eq!(
            first_resolvable_credential("provider", &inline).unwrap(),
            (
                "inline-secret://provider/api_token".to_string(),
                "secret".to_string()
            )
        );

        let secret_ref: ProviderCredentials = serde_json::from_value(json!({
            "api_token": {"secret_ref": "vault://aicc/provider"}
        }))
        .unwrap();
        assert!(first_resolvable_credential("provider", &secret_ref)
            .unwrap_err()
            .to_string()
            .contains("secret resolver"));

        let runtime_ref: ProviderCredentials = serde_json::from_value(json!({
            "api_token": {"runtime_ref": "runtime://device-jwt"}
        }))
        .unwrap();
        assert!(first_resolvable_credential("provider", &runtime_ref)
            .unwrap_err()
            .to_string()
            .contains("static API key"));
    }

    #[tokio::test]
    async fn persisted_reserved_custom_adapters_do_not_block_startup_or_reload() {
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let storage = Arc::new(
            AiccStorage::open("sqlite::memory:", buckyos_api::RdbBackend::Sqlite)
                .await
                .unwrap(),
        );
        let events = Arc::new(AiccEventLog::default());
        let factory = ServiceRuntimeFactory::new(storage, events.clone());
        let mut settings = AiccSettings::default();
        for adapter in [
            "glm-chat",
            "kimi-chat",
            "deepseek-responses",
            "openrouter-responses",
        ] {
            settings.providers.push(serde_json::from_value(json!({
                "provider_instance_name": adapter, "provider_type": "cloud_api",
                "provider_profile_id": "custom", "protocol_adapter_id": adapter,
                "base_url": "https://example.invalid/v1", "credentials": {"api_token":{"inline_secret":"test-secret"}},
                "auto_sync_models": false
            })).unwrap());
        }
        settings.providers.push(
            serde_json::from_value(json!({
                "provider_instance_name": "healthy", "provider_type": "cloud_api",
                "provider_profile_id": "doubao", "protocol_adapter_id": "doubao-responses",
                "base_url": "https://ark.cn-beijing.volces.com/api/v3",
                "credentials": {"api_token":{"inline_secret":"test-secret"}}, "auto_sync_models": false
            }))
            .unwrap(),
        );
        for (timeout_ms, expected_timeout_secs) in [(None, 600), (Some(45_000), 45)] {
            settings.providers.last_mut().unwrap().timeout_ms = timeout_ms;
            let prepared = factory
                .prepare(
                    Arc::new(settings.clone()),
                    catalog.clone(),
                    catalog.target_revision_seq(),
                )
                .await
                .unwrap();
            assert_eq!(
                prepared
                    .state
                    .providers
                    .get("healthy")
                    .unwrap()
                    .config
                    .request_timeout,
                Duration::from_secs(expected_timeout_secs)
            );
            for adapter in [
                "glm-chat",
                "kimi-chat",
                "deepseek-responses",
                "openrouter-responses",
            ] {
                let state = &prepared.state.provider_metadata[adapter];
                assert!(!state.routable);
                assert!(state.last_error.as_deref().unwrap().contains("reserved"));
            }
            let converged = prepared
                .backend
                .converge(
                    catalog.clone(),
                    catalog.target_revision_seq(),
                    ConvergenceTrigger::Inference,
                )
                .await
                .unwrap();
            assert!(converged.provider_metadata["glm-chat"].last_error.is_some());
            prepared.backend.shutdown().await;
        }
        assert!(events
            .list(20)
            .0
            .iter()
            .any(|event| event.kind == "provider_configuration_error"));
    }
}
