use super::*;
use crate::protocol::{
    fal_queue_adapter, gemini_interactions_adapter, glm_chat_adapter, kimi_chat_adapter,
    minimax_messages_adapter, openai_chat_completions_adapter, openai_responses_adapter,
    openai_responses_compatible_adapters, openrouter_chat_adapter, CodecRegistry, HttpTransport,
    HttpTransportConfig,
};
use crate::provider::{
    CatalogOnlyDiscovery, DynamicLoginCredentialResolver, ProviderAuthMode,
    ProviderConnectionContract, ProviderDiscovery, ProviderDiscoverySnapshot, ProviderError,
    ProviderInstanceConfig, ProviderProfile, ProviderResult,
};
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
}

#[derive(Clone)]
enum BuiltinConnectionFactory {
    Static(fn() -> ProviderConnectionContract),
    Configured(ProviderConnectionContract),
    Sn,
}

impl BuiltinConnectionFactory {
    fn build(&self, auth_mode: ProviderAuthMode) -> ProviderConnectionContract {
        match self {
            Self::Static(factory) => factory(),
            Self::Configured(connection) => connection.clone(),
            Self::Sn => sn_connection_contract(auth_mode),
        }
    }
}

#[derive(Clone)]
pub(crate) struct BuiltinProviderBinding {
    pub profile: ProviderProfile,
    pub connection: ProviderConnectionContract,
    pub discovery: Arc<dyn ProviderDiscovery>,
    pub dynamic_login_resolver: Option<Arc<dyn DynamicLoginCredentialResolver>>,
}

pub(crate) struct BuiltinProviderRequest<'a> {
    pub provider_profile_id: &'a str,
    pub protocol_adapter_id: &'a str,
    pub auth_mode: ProviderAuthMode,
    pub configured_inventory: Option<ProviderDiscoverySnapshot>,
}

pub(crate) struct BuiltinProviderRegistry {
    providers: BTreeMap<String, BuiltinProviderRegistration>,
    codecs: Arc<CodecRegistry>,
    transport_config: HttpTransportConfig,
    dynamic_login_resolver: Arc<dyn DynamicLoginCredentialResolver>,
}

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

pub(crate) fn builtin_provider_registry() -> ProviderResult<BuiltinProviderRegistry> {
    BuiltinProviderRegistry::new(HttpTransportConfig::default())
}

impl BuiltinProviderRegistry {
    pub(crate) fn new(transport_config: HttpTransportConfig) -> ProviderResult<Self> {
        let providers = builtin_provider_registrations()?
            .into_iter()
            .map(|registration| {
                (
                    registration.profile.provider_profile_id.clone(),
                    registration,
                )
            })
            .collect();
        let codecs = Arc::new(builtin_codec_registry()?);
        let dynamic_login_resolver = Arc::new(SnDynamicLoginResolver::new(reqwest::Client::new()));
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
        if registration.profile.default_protocol_adapter_id != request.protocol_adapter_id {
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
        let discovery = self.discovery(registration.discovery, request.configured_inventory)?;
        Ok(BuiltinProviderBinding {
            profile: registration.profile.clone(),
            connection: registration.connection.build(request.auth_mode),
            discovery,
            dynamic_login_resolver: (request.auth_mode == ProviderAuthMode::DynamicLogin)
                .then(|| self.dynamic_login_resolver.clone()),
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

fn builtin_provider_registrations() -> ProviderResult<Vec<BuiltinProviderRegistration>> {
    let compatible = openai_responses_compatible_builtin_providers();
    let mut providers = vec![
        registration(
            openai_profile(),
            openai_connection_contract,
            BuiltinDiscoveryFactory::OpenAi,
        ),
        registration(
            claude_profile(),
            claude_connection_contract,
            BuiltinDiscoveryFactory::Claude,
        ),
        registration(
            minimax_profile(),
            minimax_connection_contract,
            BuiltinDiscoveryFactory::MiniMax,
        ),
        registration(
            gemini_profile(),
            gemini_connection_contract,
            BuiltinDiscoveryFactory::Gemini,
        ),
        registration(
            openrouter_profile(),
            openrouter_connection_contract,
            BuiltinDiscoveryFactory::OpenRouter,
        ),
        registration(
            kimi_profile(),
            kimi_connection_contract,
            BuiltinDiscoveryFactory::Kimi,
        ),
        registration(
            glm_profile(),
            glm_connection_contract,
            BuiltinDiscoveryFactory::CatalogOnly,
        ),
        registration(
            fal_profile(),
            fal_connection_contract,
            BuiltinDiscoveryFactory::CatalogOnly,
        ),
    ];
    for descriptor in compatible {
        let profile_id = descriptor.profile.provider_profile_id.as_str();
        let discovery = if profile_id == DEEPSEEK_PROFILE_ID {
            BuiltinDiscoveryFactory::DeepSeek
        } else {
            BuiltinDiscoveryFactory::CatalogOnly
        };
        providers.push(BuiltinProviderRegistration {
            profile: descriptor.profile,
            connection: BuiltinConnectionFactory::Configured(descriptor.connection),
            discovery,
            supports_dynamic_login: false,
        });
    }
    providers.push(BuiltinProviderRegistration {
        profile: sn_profile(),
        connection: BuiltinConnectionFactory::Sn,
        discovery: BuiltinDiscoveryFactory::Sn,
        supports_dynamic_login: true,
    });
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

fn registration(
    profile: ProviderProfile,
    connection: fn() -> ProviderConnectionContract,
    discovery: BuiltinDiscoveryFactory,
) -> BuiltinProviderRegistration {
    BuiltinProviderRegistration {
        profile,
        connection: BuiltinConnectionFactory::Static(connection),
        discovery,
        supports_dynamic_login: false,
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
    use crate::protocol::{
        FAL_QUEUE_ADAPTER_ID, FAL_QUEUE_OPERATION_ID, GLM_CHAT_ADAPTER_ID, KIMI_CHAT_ADAPTER_ID,
        MINIMAX_MESSAGES_ADAPTER_ID, OPENAI_CHAT_COMPLETIONS_ADAPTER_ID,
        OPENAI_RESPONSES_ADAPTER_ID, OPENROUTER_CHAT_ADAPTER_ID,
    };
    use crate::provider::{CredentialReference, ProviderHealthState};
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

    #[test]
    fn production_registry_contains_every_builtin_once() {
        let registry = builtin_provider_registry().unwrap();
        let profile_ids = registry
            .profiles()
            .map(|profile| profile.provider_profile_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            profile_ids,
            BTreeSet::from([
                "claude",
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
    fn every_profile_resolves_through_the_same_instance_entrypoint() {
        let registry = builtin_provider_registry().unwrap();
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
                provider_rules_id: Some(profile.provider_profile_id.clone()),
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
        }

        let sn = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: SN_PROVIDER_PROFILE_ID,
                protocol_adapter_id: SN_OPENAI_ADAPTER_ID,
                auth_mode: ProviderAuthMode::DynamicLogin,
                configured_inventory: None,
            })
            .unwrap();
        assert!(sn.dynamic_login_resolver.is_some());
    }

    #[test]
    fn unknown_profile_and_adapter_have_stable_errors() {
        let registry = builtin_provider_registry().unwrap();
        let unknown_profile = registry.resolve(BuiltinProviderRequest {
            provider_profile_id: "missing-profile",
            protocol_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
            auth_mode: ProviderAuthMode::ApiKey,
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
            configured_inventory: None,
        });
        assert!(matches!(
            unknown_adapter,
            Err(ProviderError::UnknownAdapter(id)) if id == "missing-adapter"
        ));
    }
}
