use super::*;

pub(crate) struct ServiceRuntimeFactory {
    storage: Arc<AiccStorage>,
    provider_refreshes: broadcast::Sender<ProviderRefreshEvent>,
}

impl ServiceRuntimeFactory {
    pub(crate) fn new(storage: Arc<AiccStorage>) -> Self {
        let (provider_refreshes, _) = broadcast::channel(64);
        Self {
            storage,
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
        let builtins = builtin_provider_registry(catalog.as_ref())
            .map_err(|error| RuntimeError::Backend(error.to_string()))?;
        let (resolver, auth) = settings_credentials(settings.as_ref())
            .map_err(|error| RuntimeError::Backend(error.to_string()))?;
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
        for provider in settings
            .providers
            .iter()
            .filter(|provider| provider.enabled)
        {
            builtins
                .codecs()
                .resolve_adapter_id(
                    provider.protocol_family_id.as_deref(),
                    Some(&provider.protocol_adapter_id),
                    None,
                )
                .map_err(|error| RuntimeError::Backend(error.to_string()))?;
            let provider_auth = auth.get(&provider.provider_instance_name).ok_or_else(|| {
                RuntimeError::Backend("provider authentication was not prepared".to_string())
            })?;
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
                })
                .map_err(|error| RuntimeError::Backend(error.to_string()))?;
            let provider_rules_id = provider.provider_rules_id.clone().or_else(|| {
                catalog
                    .resolve_provider_configuration(&provider.provider_profile_id)
                    .ok()
                    .map(|configuration| configuration.provider_rules_id)
            });
            let mut runtime_config = match provider_auth {
                ProviderAuthConfig::ApiKey {
                    credential_ref,
                    credential_kind,
                } => ProviderInstanceConfig {
                    provider_instance_name: provider.provider_instance_name.clone(),
                    provider_profile_id: provider.provider_profile_id.clone(),
                    protocol_adapter_id: provider.protocol_adapter_id.clone(),
                    base_url: connection.base_url,
                    credential: CredentialReference {
                        reference: credential_ref.clone(),
                    },
                    credential_kind: *credential_kind,
                    provider_rules_id,
                    region: connection.region,
                    workspace: connection.workspace,
                    account: connection.account,
                    request_timeout: Duration::from_millis(provider.timeout_ms.unwrap_or(120_000)),
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
            runtime_config.request_timeout =
                Duration::from_millis(provider.timeout_ms.unwrap_or(120_000));
            runtime_config.auto_sync_models = provider.auto_sync_models.unwrap_or(true);
            runtime_config.instance_rules = provider
                .instance_rules
                .clone()
                .or(binding.instance_rules.clone());
            manager
                .start(runtime_config, binding.discovery)
                .await
                .map_err(|error| RuntimeError::Backend(error.to_string()))?;
        }
        let models: Arc<dyn ModelRegistryAssembler> = Arc::new(ServiceModelAssembler {
            session: settings.session_config.clone(),
        });
        let backend: Arc<dyn RuntimeBackend> =
            Arc::new(ProviderRuntimeBackend::new(manager, models));
        let state = backend
            .converge(catalog, target_seq, ConvergenceTrigger::MetadataRefresh)
            .await?;
        Ok(PreparedRuntime { backend, state })
    }
}

fn settings_credentials(
    settings: &AiccSettings,
) -> Result<
    (
        StaticCredentialResolver,
        BTreeMap<String, ProviderAuthConfig>,
    ),
    RPCErrors,
> {
    let mut values = BTreeMap::new();
    let mut auth = BTreeMap::new();
    for provider in &settings.providers {
        let parsed = provider.auth.as_ref().map(provider_auth_config);
        let parsed = match parsed {
            Some(value) => {
                if let ProviderAuthConfig::ApiKey { credential_ref, .. } = &value {
                    let (_, secret) = first_locked_credential(
                        &provider.provider_instance_name,
                        &provider.credentials,
                    )?;
                    values.insert(credential_ref.clone(), secret);
                }
                value
            }
            None => {
                let (reference, secret) = first_locked_credential(
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
        auth.insert(provider.provider_instance_name.clone(), parsed);
    }
    Ok((StaticCredentialResolver::new(values), auth))
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

fn first_locked_credential(
    instance: &str,
    credentials: &ProviderCredentials,
) -> Result<(String, String), RPCErrors> {
    for (name, value) in credentials {
        if !value.locked.is_empty() {
            return Ok((format!("locked://{instance}/{name}"), value.locked.clone()));
        }
    }
    Err(RPCErrors::ReasonError(
        "no locked credential was provided".to_string(),
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
        let mut adapter = builtins
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
                let (_, secret) = first_locked_credential(&provider_name, &request.credentials)?;
                (auth, BTreeMap::from([(credential_ref, secret)]))
            }
            None => {
                let (reference, secret) =
                    first_locked_credential(&provider_name, &request.credentials)?;
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
        if request.protocol_adapter_id.is_none()
            && request.protocol_family_id.is_some()
            && profile.is_some_and(|profile| profile.accepts_any_adapter)
        {
            if let ProviderAuthConfig::ApiKey { credential_ref, .. } = &auth {
                let resolver = StaticCredentialResolver::new(credentials.clone());
                let family = request.protocol_family_id.as_deref().unwrap_or_default();
                let mut selected = None;
                for candidate in builtins
                    .codecs()
                    .probe_candidates(family)
                    .map_err(to_rpc_error)?
                {
                    let candidate_binding = builtins
                        .resolve(BuiltinProviderRequest {
                            provider_profile_id: &request.provider_profile_id,
                            protocol_adapter_id: &candidate,
                            auth_mode: auth.mode(),
                            credential_kind: auth.credential_kind(),
                            configured_inventory: configured_inventory.clone(),
                        })
                        .map_err(to_rpc_error)?;
                    let connection = candidate_binding
                        .connection
                        .resolve(ProviderConnectionInput {
                            base_url: Some(&request.base_url),
                            region: request.region.as_deref(),
                            workspace: request.workspace.as_deref(),
                            account: request.account.as_deref(),
                        })
                        .map_err(to_rpc_error)?;
                    let descriptor = candidate_binding
                        .profile
                        .credential_for(auth.credential_kind())
                        .map_err(to_rpc_error)?;
                    let credential = resolver
                        .resolve(
                            descriptor,
                            &CredentialReference {
                                reference: credential_ref.clone(),
                            },
                        )
                        .await
                        .map_err(to_rpc_error)?;
                    if builtins
                        .codecs()
                        .probe_adapter(
                            &candidate,
                            &connection.base_url,
                            &credential,
                            Duration::from_secs(10),
                        )
                        .await
                        .map_err(to_rpc_error)?
                    {
                        selected = Some(candidate);
                        break;
                    }
                }
                adapter = selected.ok_or_else(|| {
                    RPCErrors::ReasonError(format!(
                        "protocol family `{family}` is not supported by the provider endpoint"
                    ))
                })?;
            } else {
                return Err(RPCErrors::ReasonError(
                    "protocol-family negotiation requires API-key authentication or an explicit adapter"
                        .to_owned(),
                ));
            }
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
            snapshot.catalog.clone(),
            builtins.codecs(),
            self.storage.clone(),
        )
        .map_err(to_rpc_error)?;
        let draft = ProviderDraftConfig {
            provider_instance_name: provider_name,
            provider_profile_id: request.provider_profile_id,
            protocol_adapter_id: adapter.clone(),
            provider_rules_id: request.provider_rules_id,
            base_url: Some(request.base_url),
            region: request.region,
            workspace: request.workspace,
            account: request.account,
            auth,
            dynamic_login_user_name: None,
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
            Ok(negotiated) => Ok(ProviderValidateResponse {
                base_url_reachable: true,
                auth_valid: true,
                models_discovered: negotiated
                    .inventory
                    .models
                    .iter()
                    .map(|model| model.provider_model_id.clone())
                    .collect(),
                balance_available: true,
                errors: Vec::new(),
                error_details: Vec::new(),
                resolved_protocol_adapter_id: Some(negotiated.protocol_adapter_id),
            }),
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
                    "provider validation failed at {:?}: {:?}",
                    error.stage, error.kind
                );
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
                })
            }
        }
    }
}
