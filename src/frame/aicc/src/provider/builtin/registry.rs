use super::*;
use crate::catalog::{
    CatalogSnapshot, ProviderCredentialKind, ProviderFieldMode as CatalogProviderFieldMode,
    ResolvedProviderConfiguration,
};
use crate::protocol::{
    fal_queue_adapter, gemini_interactions_adapter, glm_chat_adapter, kimi_chat_adapter,
    minimax_messages_adapter, openai_chat_completions_adapter, openai_responses_adapter,
    openai_responses_compatible_adapters, openrouter_chat_adapter, CodecRegistry, CredentialKind,
    HttpTransport, HttpTransportConfig,
};
use crate::provider::{
    CatalogOnlyDiscovery, CredentialDescriptor, DiscoveryMode, DynamicLoginCredentialResolver,
    ProviderAuthMode, ProviderConnectionContract, ProviderDiscovery, ProviderDiscoverySnapshot,
    ProviderError, ProviderFieldMode, ProviderFieldSchema, ProviderInstanceConfig, ProviderProfile,
    ProviderResult, RefreshPolicy,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuiltinDiscoveryFactory {
    OpenAi,
    Claude,
    MiniMax,
    Gemini,
    OpenRouter,
    Kimi,
    DeepSeek,
    Sn,
    CatalogOnly,
}

#[derive(Clone)]
struct BuiltinProviderRegistration {
    profile: ProviderProfile,
    connection: BuiltinConnectionFactory,
    discovery: BuiltinDiscoveryFactory,
    supports_dynamic_login: bool,
    supports_any_adapter: bool,
    instance_rules: Option<Value>,
}

#[derive(Clone)]
enum BuiltinConnectionFactory {
    Configured(ProviderConnectionContract),
    Sn(ProviderConnectionContract),
}

impl BuiltinConnectionFactory {
    fn build(&self, auth_mode: ProviderAuthMode) -> ProviderConnectionContract {
        match self {
            Self::Configured(connection) => connection.clone(),
            Self::Sn(connection) => {
                let mut connection = connection.clone();
                if auth_mode == ProviderAuthMode::DynamicLogin {
                    connection.account = ProviderFieldSchema::required();
                }
                connection
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct BuiltinProviderBinding {
    pub profile: ProviderProfile,
    pub connection: ProviderConnectionContract,
    pub discovery: Arc<dyn ProviderDiscovery>,
    pub dynamic_login_resolver: Option<Arc<dyn DynamicLoginCredentialResolver>>,
    pub instance_rules: Option<Value>,
}

pub(crate) struct BuiltinProviderRequest<'a> {
    pub provider_profile_id: &'a str,
    pub protocol_adapter_id: &'a str,
    pub auth_mode: ProviderAuthMode,
    pub credential_kind: Option<CredentialKind>,
    pub configured_inventory: Option<ProviderDiscoverySnapshot>,
}

pub(crate) struct BuiltinProviderRegistry {
    providers: BTreeMap<String, BuiltinProviderRegistration>,
    codecs: Arc<CodecRegistry>,
    transport_config: HttpTransportConfig,
    dynamic_login_resolver: Arc<dyn DynamicLoginCredentialResolver>,
}

pub(crate) const CUSTOM_PROVIDER_PROFILE_ID: &str = "custom";

impl std::fmt::Debug for BuiltinProviderRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BuiltinProviderRegistry")
            .field(
                "provider_profiles",
                &self.providers.keys().collect::<Vec<_>>(),
            )
            .field("codecs", &self.codecs)
            .finish_non_exhaustive()
    }
}

pub(crate) fn builtin_provider_registry(
    catalog: &CatalogSnapshot,
) -> ProviderResult<BuiltinProviderRegistry> {
    BuiltinProviderRegistry::new(catalog, HttpTransportConfig::default())
}

pub(crate) fn builtin_provider_codecs() -> ProviderResult<Arc<CodecRegistry>> {
    builtin_codec_registry().map(Arc::new)
}

impl BuiltinProviderRegistry {
    pub(crate) fn new(
        catalog: &CatalogSnapshot,
        transport_config: HttpTransportConfig,
    ) -> ProviderResult<Self> {
        let providers = builtin_provider_registrations(catalog)?
            .into_iter()
            .map(|registration| {
                (
                    registration.profile.provider_profile_id.clone(),
                    registration,
                )
            })
            .collect();
        let codecs = Arc::new(builtin_codec_registry()?);
        let dynamic_login_resolver = Arc::new(SnDynamicLoginResolver::new(
            reqwest::Client::new(),
            SN_DYNAMIC_LOGIN_PROFILE_ID.to_owned(),
        ));
        Ok(Self {
            providers,
            codecs,
            transport_config,
            dynamic_login_resolver,
        })
    }

    pub(crate) fn profiles(&self) -> impl ExactSizeIterator<Item = &ProviderProfile> {
        self.providers.values().map(|provider| &provider.profile)
    }

    pub(crate) fn codecs(&self) -> Arc<CodecRegistry> {
        self.codecs.clone()
    }

    pub(crate) fn dynamic_login_resolver(&self) -> Arc<dyn DynamicLoginCredentialResolver> {
        self.dynamic_login_resolver.clone()
    }

    pub(crate) fn resolve(
        &self,
        request: BuiltinProviderRequest<'_>,
    ) -> ProviderResult<BuiltinProviderBinding> {
        let registration = self
            .providers
            .get(request.provider_profile_id)
            .ok_or_else(|| ProviderError::UnknownProfile(request.provider_profile_id.to_owned()))?;
        if self.codecs.adapter(request.protocol_adapter_id).is_none() {
            return Err(ProviderError::UnknownAdapter(
                request.protocol_adapter_id.to_owned(),
            ));
        }
        if !registration.supports_any_adapter
            && registration.profile.default_protocol_adapter_id != request.protocol_adapter_id
        {
            return Err(ProviderError::InvalidConfiguration(format!(
                "provider profile `{}` requires protocol adapter `{}`",
                request.provider_profile_id, registration.profile.default_protocol_adapter_id
            )));
        }
        if request.auth_mode == ProviderAuthMode::DynamicLogin
            && !registration.supports_dynamic_login
        {
            return Err(ProviderError::InvalidConfiguration(format!(
                "provider profile `{}` does not support dynamic login",
                request.provider_profile_id
            )));
        }
        let profile = registration
            .profile
            .with_credential(request.credential_kind)?;
        let discovery = self.discovery(registration.discovery, request.configured_inventory)?;
        Ok(BuiltinProviderBinding {
            profile,
            connection: registration.connection.build(request.auth_mode),
            discovery,
            dynamic_login_resolver: (request.auth_mode == ProviderAuthMode::DynamicLogin)
                .then(|| self.dynamic_login_resolver.clone()),
            instance_rules: registration.instance_rules.clone(),
        })
    }

    pub(crate) fn resolve_instance(
        &self,
        instance: &ProviderInstanceConfig,
        auth_mode: ProviderAuthMode,
        configured_inventory: Option<ProviderDiscoverySnapshot>,
    ) -> ProviderResult<BuiltinProviderBinding> {
        self.resolve(BuiltinProviderRequest {
            provider_profile_id: &instance.provider_profile_id,
            protocol_adapter_id: &instance.protocol_adapter_id,
            auth_mode,
            credential_kind: instance.credential_kind,
            configured_inventory,
        })
    }

    fn discovery(
        &self,
        factory: BuiltinDiscoveryFactory,
        configured_inventory: Option<ProviderDiscoverySnapshot>,
    ) -> ProviderResult<Arc<dyn ProviderDiscovery>> {
        if factory == BuiltinDiscoveryFactory::CatalogOnly {
            let inventory = configured_inventory.ok_or_else(|| {
                ProviderError::InvalidConfiguration(
                    "catalog-only provider requires configured discovery inventory".to_owned(),
                )
            })?;
            super::super::validate_discovery(&inventory)?;
            return Ok(Arc::new(CatalogOnlyDiscovery::new(inventory)));
        }
        if configured_inventory.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "machine-API provider does not accept configured discovery inventory".to_owned(),
            ));
        }
        let transport = || {
            HttpTransport::new(self.transport_config.clone())
                .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))
        };
        Ok(match factory {
            BuiltinDiscoveryFactory::OpenAi => Arc::new(OpenAiDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::Claude => Arc::new(claude_discovery(transport()?)),
            BuiltinDiscoveryFactory::MiniMax => Arc::new(minimax_discovery(transport()?)),
            BuiltinDiscoveryFactory::Gemini => Arc::new(GeminiDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::OpenRouter => Arc::new(OpenRouterDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::Kimi => Arc::new(KimiDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::DeepSeek => Arc::new(deepseek_models_discovery(transport()?)),
            BuiltinDiscoveryFactory::Sn => Arc::new(SnDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::CatalogOnly => unreachable!(),
        })
    }
}

fn builtin_provider_registrations(
    catalog: &CatalogSnapshot,
) -> ProviderResult<Vec<BuiltinProviderRegistration>> {
    let mut providers = vec![
        Ok(custom_registration()),
        catalog_registration(
            catalog,
            OPENAI_PROVIDER_PROFILE_ID,
            BuiltinDiscoveryFactory::OpenAi,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            CLAUDE_PROVIDER_PROFILE_ID,
            BuiltinDiscoveryFactory::Claude,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            MINIMAX_PROVIDER_PROFILE_ID,
            BuiltinDiscoveryFactory::MiniMax,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            GEMINI_PROVIDER_PROFILE_ID,
            BuiltinDiscoveryFactory::Gemini,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            OPENROUTER_PROVIDER_PROFILE_ID,
            BuiltinDiscoveryFactory::OpenRouter,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            KIMI_PROVIDER_PROFILE_ID,
            BuiltinDiscoveryFactory::Kimi,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            GLM_PROVIDER_PROFILE_ID,
            BuiltinDiscoveryFactory::CatalogOnly,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            FAL_PROVIDER_PROFILE_ID,
            BuiltinDiscoveryFactory::CatalogOnly,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            DEEPSEEK_PROFILE_ID,
            BuiltinDiscoveryFactory::DeepSeek,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            DOUBAO_PROFILE_ID,
            BuiltinDiscoveryFactory::CatalogOnly,
            false,
            false,
        ),
        catalog_registration(
            catalog,
            QWEN_PROFILE_ID,
            BuiltinDiscoveryFactory::CatalogOnly,
            false,
            false,
        ),
    ];
    let mut sn = catalog_registration(
        catalog,
        SN_PROVIDER_PROFILE_ID,
        BuiltinDiscoveryFactory::Sn,
        true,
        false,
    )?;
    sn.connection = BuiltinConnectionFactory::Sn(match sn.connection {
        BuiltinConnectionFactory::Configured(connection) => connection,
        BuiltinConnectionFactory::Sn(_) => unreachable!(),
    });
    providers.push(Ok(sn));
    let providers = providers.into_iter().collect::<ProviderResult<Vec<_>>>()?;
    let mut unique = BTreeMap::new();
    for provider in &providers {
        let id = provider.profile.provider_profile_id.clone();
        if unique.insert(id.clone(), ()).is_some() {
            return Err(ProviderError::InvalidConfiguration(format!(
                "duplicate builtin provider profile `{id}`"
            )));
        }
    }
    Ok(providers)
}

fn catalog_registration(
    catalog: &CatalogSnapshot,
    provider_profile_id: &str,
    discovery: BuiltinDiscoveryFactory,
    supports_dynamic_login: bool,
    supports_any_adapter: bool,
) -> ProviderResult<BuiltinProviderRegistration> {
    let configuration = catalog
        .resolve_provider_configuration(provider_profile_id)
        .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))?;
    Ok(BuiltinProviderRegistration {
        profile: profile_from_catalog(&configuration, discovery),
        connection: BuiltinConnectionFactory::Configured(connection_from_catalog(&configuration)),
        discovery,
        supports_dynamic_login,
        supports_any_adapter,
        instance_rules: None,
    })
}

fn profile_from_catalog(
    configuration: &ResolvedProviderConfiguration,
    discovery: BuiltinDiscoveryFactory,
) -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: configuration.provider_profile_id.clone(),
        display_name: configuration.display_name.clone(),
        default_protocol_adapter_id: configuration.protocol_adapter_id.clone(),
        credential: CredentialDescriptor {
            kind: match configuration.credential.kind {
                ProviderCredentialKind::Bearer => crate::protocol::CredentialKind::Bearer,
                ProviderCredentialKind::NamedHeader => crate::protocol::CredentialKind::NamedHeader,
                ProviderCredentialKind::FalKey => crate::protocol::CredentialKind::FalKey,
                ProviderCredentialKind::GlmJwt => crate::protocol::CredentialKind::GlmJwt,
            },
            header_name: configuration.credential.header_name.clone(),
        },
        credential_variants: configuration
            .credential_variants
            .iter()
            .map(|credential| CredentialDescriptor {
                kind: match credential.kind {
                    ProviderCredentialKind::Bearer => CredentialKind::Bearer,
                    ProviderCredentialKind::NamedHeader => CredentialKind::NamedHeader,
                    ProviderCredentialKind::FalKey => CredentialKind::FalKey,
                    ProviderCredentialKind::GlmJwt => CredentialKind::GlmJwt,
                },
                header_name: credential.header_name.clone(),
            })
            .collect(),
        discovery_mode: if discovery == BuiltinDiscoveryFactory::CatalogOnly {
            DiscoveryMode::CatalogOnly
        } else {
            DiscoveryMode::MachineApi
        },
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

fn connection_from_catalog(
    configuration: &ResolvedProviderConfiguration,
) -> ProviderConnectionContract {
    ProviderConnectionContract {
        default_base_url: configuration.default_base_url.clone(),
        region: field_from_catalog(&configuration.connection.region),
        workspace: field_from_catalog(&configuration.connection.workspace),
        account: field_from_catalog(&configuration.connection.account),
        region_base_urls: configuration.connection.region_base_urls.clone(),
    }
}

fn field_from_catalog(schema: &crate::catalog::ProviderFieldSchema) -> ProviderFieldSchema {
    ProviderFieldSchema {
        mode: match schema.mode {
            CatalogProviderFieldMode::Unsupported => ProviderFieldMode::Unsupported,
            CatalogProviderFieldMode::Optional => ProviderFieldMode::Optional,
            CatalogProviderFieldMode::Required => ProviderFieldMode::Required,
        },
        default_value: schema.default_value.clone(),
        allowed_values: schema.allowed_values.iter().cloned().collect(),
    }
}

fn custom_registration() -> BuiltinProviderRegistration {
    BuiltinProviderRegistration {
        profile: ProviderProfile {
            provider_profile_id: CUSTOM_PROVIDER_PROFILE_ID.to_owned(),
            display_name: "Custom Provider".to_owned(),
            default_protocol_adapter_id: crate::protocol::OPENAI_RESPONSES_ADAPTER_ID.to_owned(),
            credential: CredentialDescriptor {
                kind: crate::protocol::CredentialKind::Bearer,
                header_name: None,
            },
            credential_variants: Vec::new(),
            discovery_mode: DiscoveryMode::CatalogOnly,
            refresh: RefreshPolicy::default(),
            default_inventory: None,
        },
        connection: BuiltinConnectionFactory::Configured(ProviderConnectionContract {
            default_base_url: String::new(),
            region: ProviderFieldSchema::optional(),
            workspace: ProviderFieldSchema::optional(),
            account: ProviderFieldSchema::optional(),
            region_base_urls: BTreeMap::new(),
        }),
        discovery: BuiltinDiscoveryFactory::CatalogOnly,
        supports_dynamic_login: false,
        supports_any_adapter: true,
        instance_rules: Some(Value::Object(Map::new())),
    }
}

fn builtin_codec_registry() -> ProviderResult<CodecRegistry> {
    let mut registry = CodecRegistry::default();
    for (descriptor, codecs) in [
        openai_responses_adapter(),
        claude_messages_adapter(),
        gemini_interactions_adapter(),
        openai_chat_completions_adapter(),
        fal_queue_adapter(),
    ] {
        registry
            .register_codecs(descriptor, codecs)
            .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))?;
    }
    for (descriptor, codecs) in [
        minimax_messages_adapter(),
        openrouter_chat_adapter(),
        kimi_chat_adapter(),
        glm_chat_adapter(),
    ] {
        registry
            .register_derived(descriptor, codecs)
            .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))?;
    }
    for (descriptor, codecs) in openai_responses_compatible_adapters()
        .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))?
    {
        registry
            .register_derived(descriptor, codecs)
            .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))?;
    }
    register_sn_openai_adapter(&mut registry)
        .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogKind;
    use crate::protocol::{
        FAL_QUEUE_ADAPTER_ID, FAL_QUEUE_OPERATION_ID, GLM_CHAT_ADAPTER_ID, KIMI_CHAT_ADAPTER_ID,
        MINIMAX_MESSAGES_ADAPTER_ID, OPENAI_CHAT_COMPLETIONS_ADAPTER_ID,
        OPENAI_RESPONSES_ADAPTER_ID, OPENROUTER_CHAT_ADAPTER_ID,
    };
    use crate::provider::{CredentialReference, ProviderConnectionInput, ProviderHealthState};
    use crate::settings::{load_builtin_metadata, MetadataFile, MetadataSource, MetadataSources};
    use buckyos_api::ApiType;
    use std::collections::BTreeSet;

    fn configured_inventory() -> ProviderDiscoverySnapshot {
        ProviderDiscoverySnapshot {
            revision: Some("configured-1".to_owned()),
            discovered_at_ms: 1,
            health: ProviderHealthState::Healthy,
            models: Vec::new(),
        }
    }

    fn registry() -> BuiltinProviderRegistry {
        let catalog = MetadataSources {
            builtin: load_builtin_metadata().unwrap(),
            ..MetadataSources::default()
        }
        .build_snapshot(1, &crate::catalog::CatalogBuildOptions::default())
        .unwrap();
        builtin_provider_registry(catalog.as_ref()).unwrap()
    }

    #[test]
    fn production_registry_contains_every_builtin_once() {
        let registry = registry();
        let profile_ids = registry
            .profiles()
            .map(|profile| profile.provider_profile_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            profile_ids,
            BTreeSet::from([
                "claude",
                "custom",
                "deepseek",
                "doubao",
                "fal",
                "gemini",
                "glm",
                "kimi",
                "minimax",
                "openai",
                "openrouter",
                "qwen",
                "sn",
            ])
        );
        assert_eq!(registry.profiles().len(), profile_ids.len());

        let codecs = registry.codecs();
        let adapter_ids = codecs
            .adapters()
            .map(|adapter| adapter.protocol_adapter_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(codecs.adapters().len(), adapter_ids.len());
        assert_eq!(adapter_ids.len(), 13);
        for profile in registry.profiles() {
            assert!(adapter_ids.contains(profile.default_protocol_adapter_id.as_str()));
        }
        for adapter_id in [
            OPENAI_RESPONSES_ADAPTER_ID,
            OPENAI_CHAT_COMPLETIONS_ADAPTER_ID,
            MINIMAX_MESSAGES_ADAPTER_ID,
            OPENROUTER_CHAT_ADAPTER_ID,
            KIMI_CHAT_ADAPTER_ID,
            GLM_CHAT_ADAPTER_ID,
            FAL_QUEUE_ADAPTER_ID,
            SN_OPENAI_ADAPTER_ID,
        ] {
            assert!(adapter_ids.contains(adapter_id));
        }
        assert!(codecs
            .native_task_codec(
                FAL_QUEUE_ADAPTER_ID,
                FAL_QUEUE_OPERATION_ID,
                ApiType::ImageTextToImage,
            )
            .is_ok());
    }

    #[test]
    fn metadata_source_manager_supplies_all_builtin_catalogs_to_registry() {
        let registry = registry();
        let files = load_builtin_metadata().unwrap();
        assert_eq!(files.len(), 33);
        assert_eq!(
            files
                .iter()
                .filter(|file| file.kind == CatalogKind::KnownProvider)
                .count(),
            12
        );
        assert_eq!(
            files
                .iter()
                .filter(|file| file.kind == CatalogKind::ProviderRules)
                .count(),
            12
        );
        assert_eq!(
            files
                .iter()
                .filter(|file| file.kind == CatalogKind::ModelDriver)
                .count(),
            9
        );

        let snapshot = MetadataSources {
            builtin: files,
            ..MetadataSources::default()
        }
        .build_snapshot(1, &crate::catalog::CatalogBuildOptions::default())
        .unwrap();
        for profile in registry.profiles() {
            if profile.provider_profile_id == CUSTOM_PROVIDER_PROFILE_ID {
                assert!(snapshot
                    .known_provider(CUSTOM_PROVIDER_PROFILE_ID)
                    .is_none());
                assert!(snapshot
                    .provider_rules(CUSTOM_PROVIDER_PROFILE_ID)
                    .is_none());
                continue;
            }
            assert!(snapshot
                .known_provider(&profile.provider_profile_id)
                .is_some());
            assert!(snapshot
                .provider_rules(&profile.provider_profile_id)
                .is_some());
        }
    }

    #[test]
    fn registry_configuration_comes_from_effective_snapshot() {
        let mut document: Value = serde_json::from_slice(
            &load_builtin_metadata()
                .unwrap()
                .into_iter()
                .find(|file| {
                    file.kind == CatalogKind::KnownProvider
                        && file.catalog_id == OPENAI_PROVIDER_PROFILE_ID
                })
                .unwrap()
                .contents,
        )
        .unwrap();
        document["providers"][0]["display_name"] = Value::String("Local OpenAI".to_owned());
        document["providers"][0]["base_url"] = Value::String("https://local.example/v1".to_owned());
        let local = MetadataFile::parse(
            MetadataSource::Local,
            CatalogKind::KnownProvider,
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        let catalog = MetadataSources {
            builtin: load_builtin_metadata().unwrap(),
            local: vec![local],
            ..MetadataSources::default()
        }
        .build_snapshot(1, &crate::catalog::CatalogBuildOptions::default())
        .unwrap();

        let registry = builtin_provider_registry(catalog.as_ref()).unwrap();
        let profile = registry
            .profiles()
            .find(|profile| profile.provider_profile_id == OPENAI_PROVIDER_PROFILE_ID)
            .unwrap();
        assert_eq!(profile.display_name, "Local OpenAI");
        let binding = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: OPENAI_PROVIDER_PROFILE_ID,
                protocol_adapter_id: &profile.default_protocol_adapter_id,
                auth_mode: ProviderAuthMode::ApiKey,
                credential_kind: None,
                configured_inventory: None,
            })
            .unwrap();
        assert_eq!(
            binding.connection.default_base_url,
            "https://local.example/v1"
        );
    }

    #[test]
    fn sn_registry_does_not_require_ui_hints() {
        let mut document: Value = serde_json::from_slice(
            &load_builtin_metadata()
                .unwrap()
                .into_iter()
                .find(|file| {
                    file.kind == CatalogKind::KnownProvider
                        && file.catalog_id == SN_PROVIDER_PROFILE_ID
                })
                .unwrap()
                .contents,
        )
        .unwrap();
        document["providers"][0]
            .as_object_mut()
            .unwrap()
            .remove("ui_hints");
        let local = MetadataFile::parse(
            MetadataSource::Local,
            CatalogKind::KnownProvider,
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        let catalog = MetadataSources {
            builtin: load_builtin_metadata().unwrap(),
            local: vec![local],
            ..MetadataSources::default()
        }
        .build_snapshot(1, &crate::catalog::CatalogBuildOptions::default())
        .unwrap();

        let registry = builtin_provider_registry(catalog.as_ref()).unwrap();
        let binding = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: SN_PROVIDER_PROFILE_ID,
                protocol_adapter_id: SN_OPENAI_ADAPTER_ID,
                auth_mode: ProviderAuthMode::DynamicLogin,
                credential_kind: None,
                configured_inventory: None,
            })
            .unwrap();
        assert!(binding.dynamic_login_resolver.is_some());
    }

    #[test]
    fn glm_credential_variant_and_regional_endpoints_use_typed_catalog() {
        let registry = registry();
        let glm = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: GLM_PROVIDER_PROFILE_ID,
                protocol_adapter_id: GLM_CHAT_ADAPTER_ID,
                auth_mode: ProviderAuthMode::ApiKey,
                credential_kind: Some(CredentialKind::GlmJwt),
                configured_inventory: Some(configured_inventory()),
            })
            .unwrap();
        assert_eq!(glm.profile.credential.kind, CredentialKind::GlmJwt);
        assert_eq!(
            glm.connection
                .resolve(ProviderConnectionInput {
                    region: Some("china"),
                    ..ProviderConnectionInput::default()
                })
                .unwrap()
                .base_url,
            "https://open.bigmodel.cn/api/paas/v4"
        );
        assert_eq!(
            glm.connection
                .resolve(ProviderConnectionInput {
                    base_url: Some("https://glm-proxy.example/v1"),
                    region: Some("china"),
                    ..ProviderConnectionInput::default()
                })
                .unwrap()
                .base_url,
            "https://glm-proxy.example/v1"
        );

        let minimax = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: MINIMAX_PROVIDER_PROFILE_ID,
                protocol_adapter_id: MINIMAX_MESSAGES_ADAPTER_ID,
                auth_mode: ProviderAuthMode::ApiKey,
                credential_kind: None,
                configured_inventory: None,
            })
            .unwrap();
        assert_eq!(
            minimax
                .connection
                .resolve(ProviderConnectionInput {
                    region: Some("china"),
                    ..ProviderConnectionInput::default()
                })
                .unwrap()
                .base_url,
            "https://api.minimaxi.com/anthropic"
        );

        assert!(matches!(
            registry.resolve(BuiltinProviderRequest {
                provider_profile_id: OPENAI_PROVIDER_PROFILE_ID,
                protocol_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                auth_mode: ProviderAuthMode::ApiKey,
                credential_kind: Some(CredentialKind::GlmJwt),
                configured_inventory: None,
            }),
            Err(ProviderError::InvalidConfiguration(message))
                if message.contains("does not support credential kind `glm_jwt`")
        ));
    }

    #[test]
    fn every_profile_resolves_through_the_same_instance_entrypoint() {
        let registry = registry();
        for profile in registry.profiles() {
            let configured_inventory = (profile.discovery_mode
                == crate::provider::DiscoveryMode::CatalogOnly)
                .then(configured_inventory);
            let instance = ProviderInstanceConfig {
                provider_instance_name: format!("{}-main", profile.provider_profile_id),
                provider_profile_id: profile.provider_profile_id.clone(),
                protocol_adapter_id: profile.default_protocol_adapter_id.clone(),
                base_url: "https://provider.example/v1".to_owned(),
                credential: CredentialReference {
                    reference: "secret://provider".to_owned(),
                },
                credential_kind: None,
                provider_rules_id: (profile.provider_profile_id != CUSTOM_PROVIDER_PROFILE_ID)
                    .then(|| profile.provider_profile_id.clone()),
                region: None,
                workspace: None,
                account: None,
            };
            let binding = registry
                .resolve_instance(&instance, ProviderAuthMode::ApiKey, configured_inventory)
                .unwrap();
            assert_eq!(
                binding.profile.provider_profile_id,
                profile.provider_profile_id
            );
            assert!(binding.dynamic_login_resolver.is_none());
            assert_eq!(
                binding.instance_rules,
                (profile.provider_profile_id == CUSTOM_PROVIDER_PROFILE_ID)
                    .then(|| Value::Object(Map::new()))
            );
        }

        let sn = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: SN_PROVIDER_PROFILE_ID,
                protocol_adapter_id: SN_OPENAI_ADAPTER_ID,
                auth_mode: ProviderAuthMode::DynamicLogin,
                credential_kind: None,
                configured_inventory: None,
            })
            .unwrap();
        assert!(sn.dynamic_login_resolver.is_some());
    }

    #[test]
    fn custom_provider_has_production_binding_without_builtin_catalog() {
        let registry = registry();
        let binding = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: CUSTOM_PROVIDER_PROFILE_ID,
                protocol_adapter_id: MINIMAX_MESSAGES_ADAPTER_ID,
                auth_mode: ProviderAuthMode::ApiKey,
                credential_kind: None,
                configured_inventory: Some(configured_inventory()),
            })
            .unwrap();

        assert_eq!(
            binding.profile.provider_profile_id,
            CUSTOM_PROVIDER_PROFILE_ID
        );
        assert_eq!(binding.instance_rules, Some(Value::Object(Map::new())));
        assert_eq!(
            binding
                .connection
                .resolve(ProviderConnectionInput {
                    base_url: Some("https://custom.example/v1"),
                    ..ProviderConnectionInput::default()
                })
                .unwrap()
                .base_url,
            "https://custom.example/v1"
        );
        assert!(binding
            .connection
            .resolve(ProviderConnectionInput::default())
            .is_err());

        let missing_inventory = registry.resolve(BuiltinProviderRequest {
            provider_profile_id: CUSTOM_PROVIDER_PROFILE_ID,
            protocol_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
            auth_mode: ProviderAuthMode::ApiKey,
            credential_kind: None,
            configured_inventory: None,
        });
        assert!(matches!(
            missing_inventory,
            Err(ProviderError::InvalidConfiguration(message))
                if message == "catalog-only provider requires configured discovery inventory"
        ));

        let dynamic_login = registry.resolve(BuiltinProviderRequest {
            provider_profile_id: CUSTOM_PROVIDER_PROFILE_ID,
            protocol_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
            auth_mode: ProviderAuthMode::DynamicLogin,
            credential_kind: None,
            configured_inventory: Some(configured_inventory()),
        });
        assert!(matches!(
            dynamic_login,
            Err(ProviderError::InvalidConfiguration(message))
                if message == "provider profile `custom` does not support dynamic login"
        ));
    }

    #[test]
    fn unknown_profile_and_adapter_have_stable_errors() {
        let registry = registry();
        let unknown_profile = registry.resolve(BuiltinProviderRequest {
            provider_profile_id: "missing-profile",
            protocol_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
            auth_mode: ProviderAuthMode::ApiKey,
            credential_kind: None,
            configured_inventory: None,
        });
        assert!(matches!(
            unknown_profile,
            Err(ProviderError::UnknownProfile(id)) if id == "missing-profile"
        ));

        let unknown_adapter = registry.resolve(BuiltinProviderRequest {
            provider_profile_id: OPENAI_PROVIDER_PROFILE_ID,
            protocol_adapter_id: "missing-adapter",
            auth_mode: ProviderAuthMode::ApiKey,
            credential_kind: None,
            configured_inventory: None,
        });
        assert!(matches!(
            unknown_adapter,
            Err(ProviderError::UnknownAdapter(id)) if id == "missing-adapter"
        ));
    }
}
