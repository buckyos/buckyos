use super::anthropic_models::AnthropicModelsDiscovery;
use super::*;
use crate::catalog::{
    CatalogSnapshot, ProviderCredentialKind, ProviderFieldMode as CatalogProviderFieldMode,
    ResolvedProviderConfiguration,
};
use crate::protocol::{CodecRegistry, CredentialKind, HttpTransport, HttpTransportConfig};
use crate::provider::{
    catalog_only_inventory, validate_discovery, CatalogOnlyDiscovery, CredentialDescriptor,
    DiscoveryMode, DynamicLoginCredentialResolver, FallbackDiscovery, ProviderAuthMode,
    ProviderConnectionContract, ProviderDiscovery, ProviderDiscoverySnapshot, ProviderError,
    ProviderFieldMode, ProviderFieldSchema, ProviderInstanceConfig, ProviderProfile,
    ProviderResult, RefreshPolicy,
};
#[cfg(test)]
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuiltinDiscoveryFactory {
    CatalogOnly,
    OpenAi,
    Claude,
    MiniMax,
    Gemini,
    OpenRouter,
    Kimi,
    Glm,
    DeepSeek,
    Sn,
    Standard,
}

fn discovery_behaviors() -> BTreeMap<&'static str, BuiltinDiscoveryFactory> {
    BTreeMap::from([
        ("catalog-only", BuiltinDiscoveryFactory::CatalogOnly),
        ("openai-models", BuiltinDiscoveryFactory::OpenAi),
        ("anthropic-models", BuiltinDiscoveryFactory::Claude),
        ("minimax-models", BuiltinDiscoveryFactory::MiniMax),
        ("gemini-models", BuiltinDiscoveryFactory::Gemini),
        ("openrouter-models", BuiltinDiscoveryFactory::OpenRouter),
        ("kimi-models", BuiltinDiscoveryFactory::Kimi),
        ("glm-models", BuiltinDiscoveryFactory::Glm),
        ("deepseek-models", BuiltinDiscoveryFactory::DeepSeek),
        ("sn-models", BuiltinDiscoveryFactory::Sn),
        (
            "openai-compatible-models",
            BuiltinDiscoveryFactory::Standard,
        ),
        ("standard-models", BuiltinDiscoveryFactory::Standard),
    ])
}

#[derive(Clone)]
struct BuiltinProviderRegistration {
    profile: ProviderProfile,
    connection: BuiltinConnectionFactory,
    discovery_behavior_id: String,
    dynamic_login_behavior_id: Option<String>,
    supports_any_adapter: bool,
    instance_rules: Option<buckyos_api::ProviderInstanceRules>,
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
    pub instance_rules: Option<buckyos_api::ProviderInstanceRules>,
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
    dynamic_login_resolvers: BTreeMap<String, Arc<dyn DynamicLoginCredentialResolver>>,
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
        let dynamic_login_resolvers: BTreeMap<_, Arc<dyn DynamicLoginCredentialResolver>> =
            BTreeMap::from([(
                "sn".to_owned(),
                Arc::new(SnDynamicLoginResolver::new(
                    reqwest::Client::new(),
                    SN_DYNAMIC_LOGIN_PROFILE_ID.to_owned(),
                )) as Arc<dyn DynamicLoginCredentialResolver>,
            )]);
        Ok(Self {
            providers,
            codecs,
            transport_config,
            dynamic_login_resolvers,
        })
    }

    pub(crate) fn profiles(&self) -> impl ExactSizeIterator<Item = &ProviderProfile> {
        self.providers.values().map(|provider| &provider.profile)
    }

    pub(crate) fn codecs(&self) -> Arc<CodecRegistry> {
        self.codecs.clone()
    }

    pub(crate) fn dynamic_login_resolver(&self) -> Arc<dyn DynamicLoginCredentialResolver> {
        self.dynamic_login_resolvers
            .get("sn")
            .expect("SN dynamic-login behavior must be registered")
            .clone()
    }

    pub(crate) fn resolve(
        &self,
        request: BuiltinProviderRequest<'_>,
    ) -> ProviderResult<BuiltinProviderBinding> {
        let registration = self
            .providers
            .get(request.provider_profile_id)
            .ok_or_else(|| ProviderError::UnknownProfile(request.provider_profile_id.to_owned()))?;
        let adapter = self
            .codecs
            .adapter(request.protocol_adapter_id)
            .ok_or_else(|| ProviderError::UnknownAdapter(request.protocol_adapter_id.to_owned()))?;
        if !registration.supports_any_adapter
            && registration.profile.default_protocol_adapter_id != request.protocol_adapter_id
        {
            return Err(ProviderError::InvalidConfiguration(format!(
                "provider profile `{}` requires protocol adapter `{}`",
                request.provider_profile_id, registration.profile.default_protocol_adapter_id
            )));
        }
        let dynamic_login_resolver = if request.auth_mode == ProviderAuthMode::DynamicLogin {
            let behavior_id = registration
                .dynamic_login_behavior_id
                .as_deref()
                .ok_or_else(|| {
                    ProviderError::InvalidConfiguration(format!(
                        "provider profile `{}` does not support dynamic login",
                        request.provider_profile_id
                    ))
                })?;
            Some(
                self.dynamic_login_resolvers
                    .get(behavior_id)
                    .ok_or_else(|| {
                        ProviderError::InvalidConfiguration(format!(
                            "unknown dynamic-login behavior `{behavior_id}`"
                        ))
                    })?
                    .clone(),
            )
        } else {
            None
        };
        let profile = if registration.supports_any_adapter {
            custom_profile_for_adapter(&registration.profile, adapter, request.credential_kind)?
        } else {
            registration
                .profile
                .with_credential(request.credential_kind)?
        };
        let discovery = self.discovery(
            &registration.discovery_behavior_id,
            request.provider_profile_id,
            request.protocol_adapter_id,
            request.configured_inventory,
            registration.profile.default_inventory.clone(),
        )?;
        Ok(BuiltinProviderBinding {
            profile,
            connection: registration.connection.build(request.auth_mode),
            discovery,
            dynamic_login_resolver,
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
        behavior_id: &str,
        provider_profile_id: &str,
        protocol_adapter_id: &str,
        configured_inventory: Option<ProviderDiscoverySnapshot>,
        default_inventory: Option<ProviderDiscoverySnapshot>,
    ) -> ProviderResult<Arc<dyn ProviderDiscovery>> {
        let transport = || {
            HttpTransport::new(self.transport_config.clone())
                .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))
        };
        let factory = discovery_behaviors()
            .get(behavior_id)
            .copied()
            .ok_or_else(|| {
                ProviderError::InvalidConfiguration(format!(
                    "unknown provider discovery behavior `{behavior_id}`"
                ))
            })?;
        if factory == BuiltinDiscoveryFactory::CatalogOnly {
            let inventory = configured_inventory.or(default_inventory).ok_or_else(|| {
                ProviderError::InvalidConfiguration(format!(
                    "catalog-only provider `{provider_profile_id}` has no inventory"
                ))
            })?;
            validate_discovery(&inventory)?;
            return Ok(Arc::new(CatalogOnlyDiscovery::new(inventory)));
        }
        let primary: Arc<dyn ProviderDiscovery> = match factory {
            BuiltinDiscoveryFactory::CatalogOnly => unreachable!(),
            BuiltinDiscoveryFactory::OpenAi => Arc::new(OpenAiDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::Claude => Arc::new(claude_discovery(transport()?)),
            BuiltinDiscoveryFactory::MiniMax => Arc::new(minimax_discovery(transport()?)),
            BuiltinDiscoveryFactory::Gemini => Arc::new(GeminiDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::OpenRouter => Arc::new(OpenRouterDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::Kimi => Arc::new(KimiDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::Glm => Arc::new(glm_models_discovery(transport()?)),
            BuiltinDiscoveryFactory::DeepSeek => Arc::new(openai_compatible_models_discovery(
                DEEPSEEK_PROFILE_ID,
                crate::protocol::DEEPSEEK_RESPONSES_ADAPTER_ID,
                transport()?,
            )),
            BuiltinDiscoveryFactory::Sn => Arc::new(SnDiscovery::new(transport()?)),
            BuiltinDiscoveryFactory::Standard => match self
                .codecs
                .adapter(protocol_adapter_id)
                .map(|adapter| adapter.protocol_family_id.as_str())
            {
                Some("claude") => Arc::new(AnthropicModelsDiscovery::for_profile(
                    CLAUDE_SPEC,
                    provider_profile_id,
                    protocol_adapter_id,
                    transport()?,
                )),
                Some("gemini") => Arc::new(GeminiDiscovery::for_profile(
                    provider_profile_id,
                    protocol_adapter_id,
                    transport()?,
                )),
                _ => Arc::new(openai_compatible_models_discovery(
                    provider_profile_id,
                    protocol_adapter_id,
                    transport()?,
                )),
            },
        };
        let configured_fallback = configured_inventory.is_some();
        let fallback_inventory = configured_inventory.or(default_inventory);
        let Some(inventory) = fallback_inventory else {
            return Ok(primary);
        };
        super::super::validate_discovery(&inventory)?;
        let fallback: Arc<dyn ProviderDiscovery> = if configured_fallback {
            Arc::new(CatalogOnlyDiscovery::new(inventory))
        } else {
            Arc::new(CatalogOnlyDiscovery::catalog_managed(inventory))
        };
        Ok(Arc::new(FallbackDiscovery::new(primary, fallback)))
    }
}

pub(crate) fn custom_profile_for_adapter(
    profile: &ProviderProfile,
    adapter: &crate::protocol::AdapterDescriptor,
    requested: Option<CredentialKind>,
) -> ProviderResult<ProviderProfile> {
    let credential = CredentialDescriptor {
        kind: adapter.credential.kind,
        header_name: adapter.credential.header_name.clone(),
    };
    if requested.is_some_and(|kind| kind != credential.kind) {
        return Err(ProviderError::InvalidConfiguration(format!(
            "custom provider adapter `{}` requires credential kind `{}`",
            adapter.protocol_adapter_id,
            credential.kind.as_str()
        )));
    }
    let mut selected = profile.clone();
    selected.default_protocol_adapter_id = adapter.protocol_adapter_id.clone();
    selected.credential = credential;
    selected.credential_variants.clear();
    Ok(selected)
}

fn builtin_provider_registrations(
    catalog: &CatalogSnapshot,
) -> ProviderResult<Vec<BuiltinProviderRegistration>> {
    let mut providers = vec![custom_registration()];
    for known in catalog.known_providers() {
        providers.push(catalog_registration(
            catalog,
            &known.provider_profile_id,
            false,
        )?);
    }
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
    supports_any_adapter: bool,
) -> ProviderResult<BuiltinProviderRegistration> {
    let configuration = catalog
        .resolve_provider_configuration(provider_profile_id)
        .map_err(|error| ProviderError::InvalidConfiguration(error.to_string()))?;
    let mut profile = profile_from_catalog(&configuration);
    profile.default_inventory = catalog_only_inventory(catalog, provider_profile_id);
    let connection = connection_from_catalog(&configuration);
    let connection = match configuration.connection_behavior_id.as_deref() {
        None | Some("standard") => BuiltinConnectionFactory::Configured(connection),
        Some("sn") => BuiltinConnectionFactory::Sn(connection),
        Some(id) => {
            return Err(ProviderError::InvalidConfiguration(format!(
                "unknown provider connection behavior `{id}`"
            )))
        }
    };
    Ok(BuiltinProviderRegistration {
        profile,
        connection,
        discovery_behavior_id: configuration.discovery_behavior_id,
        dynamic_login_behavior_id: configuration.dynamic_login_behavior_id,
        supports_any_adapter,
        instance_rules: None,
    })
}

fn profile_from_catalog(configuration: &ResolvedProviderConfiguration) -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: configuration.provider_profile_id.clone(),
        display_name: configuration.display_name.clone(),
        default_protocol_adapter_id: configuration.protocol_adapter_id.clone(),
        credential: credential_from_catalog(&configuration.credential),
        credential_variants: configuration
            .credential_variants
            .iter()
            .map(credential_from_catalog)
            .collect(),
        discovery_mode: if configuration.discovery_behavior_id == "catalog-only" {
            DiscoveryMode::CatalogOnly
        } else {
            DiscoveryMode::MachineApi
        },
        refresh: RefreshPolicy::default(),
        default_inventory: None,
        accepts_any_adapter: false,
    }
}

pub(super) fn credential_from_catalog(
    credential: &crate::catalog::ProviderCredentialDescriptor,
) -> CredentialDescriptor {
    CredentialDescriptor {
        kind: match credential.kind {
            ProviderCredentialKind::Bearer => CredentialKind::Bearer,
            ProviderCredentialKind::NamedHeader => CredentialKind::NamedHeader,
            ProviderCredentialKind::FalKey => CredentialKind::FalKey,
            ProviderCredentialKind::GlmJwt => CredentialKind::GlmJwt,
        },
        header_name: credential.header_name.clone(),
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

pub(super) fn field_from_catalog(
    schema: &crate::catalog::ProviderFieldSchema,
) -> ProviderFieldSchema {
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
            discovery_mode: DiscoveryMode::MachineApi,
            refresh: RefreshPolicy::default(),
            default_inventory: None,
            accepts_any_adapter: true,
        },
        connection: BuiltinConnectionFactory::Configured(ProviderConnectionContract {
            default_base_url: String::new(),
            region: ProviderFieldSchema::optional(),
            workspace: ProviderFieldSchema::optional(),
            account: ProviderFieldSchema::optional(),
            region_base_urls: BTreeMap::new(),
        }),
        discovery_behavior_id: "standard-models".to_owned(),
        dynamic_login_behavior_id: None,
        supports_any_adapter: true,
        instance_rules: Some(Default::default()),
    }
}

fn builtin_codec_registry() -> ProviderResult<CodecRegistry> {
    let mut registry = CodecRegistry::default();
    crate::protocol::register_builtin_adapter_plugins(&mut registry)
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
        OPENAI_RESPONSES_ADAPTER_ID, OPENROUTER_RESPONSES_ADAPTER_ID,
    };
    use crate::provider::{
        CredentialReference, InventoryBuilder, ModelAvailability, ProviderConnectionInput,
        ProviderHealthState,
    };
    use crate::settings::{load_builtin_metadata, MetadataFile, MetadataSource, MetadataSources};
    use buckyos_api::ApiType;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};

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
        .build_snapshot(2, &crate::catalog::CatalogBuildOptions::default())
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

        let catalog = MetadataSources {
            builtin: load_builtin_metadata().unwrap(),
            ..MetadataSources::default()
        }
        .build_snapshot(2, &crate::catalog::CatalogBuildOptions::default())
        .unwrap();
        let behaviors = discovery_behaviors();
        for provider in catalog.known_providers() {
            assert!(
                behaviors.contains_key(provider.discovery_behavior_id.as_str()),
                "{} references an unregistered discovery behavior {}",
                provider.provider_profile_id,
                provider.discovery_behavior_id
            );
        }

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
            OPENROUTER_RESPONSES_ADAPTER_ID,
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
    fn builtin_metadata_inventory_and_mounts_match_golden() {
        let catalog = MetadataSources {
            builtin: load_builtin_metadata().unwrap(),
            ..MetadataSources::default()
        }
        .build_snapshot(2, &crate::catalog::CatalogBuildOptions::default())
        .unwrap();
        let registry = builtin_provider_registry(catalog.as_ref()).unwrap();

        let mut golden = BTreeMap::new();
        for profile in registry.profiles() {
            if profile.provider_profile_id == CUSTOM_PROVIDER_PROFILE_ID {
                continue;
            }
            let Some(discovery) = profile.default_inventory.clone() else {
                golden.insert(profile.provider_profile_id.clone(), "dynamic".to_owned());
                continue;
            };
            let instance = ProviderInstanceConfig {
                provider_instance_name: format!("{}-golden", profile.provider_profile_id),
                provider_profile_id: profile.provider_profile_id.clone(),
                protocol_adapter_id: profile.default_protocol_adapter_id.clone(),
                base_url: "https://provider.example/v1".to_owned(),
                credential: CredentialReference {
                    reference: "secret://provider".to_owned(),
                },
                credential_kind: None,
                provider_rules_id: Some(profile.provider_profile_id.clone()),
                region: None,
                workspace: None,
                account: None,
                request_timeout: std::time::Duration::from_secs(120),
                auto_sync_models: true,
                instance_rules: None,
            };
            let inventory = InventoryBuilder::build(
                profile,
                &instance,
                discovery,
                catalog.as_ref(),
                &registry.codecs(),
            )
            .unwrap_or_else(|error| panic!("{}: {error}", profile.provider_profile_id));

            assert!(
                !inventory.models.is_empty(),
                "{} produced an empty catalog inventory",
                profile.provider_profile_id
            );
            for model in &inventory.models {
                assert!(
                    !model.logical_mounts.is_empty(),
                    "{}:{} produced no logical mounts",
                    profile.provider_profile_id,
                    model.provider_model_id
                );
                assert!(
                    model
                        .logical_mounts
                        .iter()
                        .all(|mount| !mount.contains('{') && !mount.contains('}')),
                    "{}:{} has unexpanded mounts {:?}",
                    profile.provider_profile_id,
                    model.origin_model_id,
                    model.logical_mounts
                );
            }
            let encoded = serde_json::to_vec(&inventory.models).unwrap();
            golden.insert(
                profile.provider_profile_id.clone(),
                format!("{}:{:x}", inventory.models.len(), Sha256::digest(encoded)),
            );
        }
        assert_eq!(
            golden,
            BTreeMap::from([
                (
                    "claude".to_owned(),
                    "5:704897f72265b326a4e652367638ad3d5c9db1d1dac237f0fe26324c2da8f381".to_owned()
                ),
                (
                    "deepseek".to_owned(),
                    "3:1adead8cb4da22a0a844bfead8416f14a0c7e627fba9a4134b985178580e5557".to_owned()
                ),
                (
                    "doubao".to_owned(),
                    "1:c054587853f80372caf82eaac075f91fab1d7f5b8d0f2bfe9167b9fdb56d019c".to_owned()
                ),
                (
                    "fal".to_owned(),
                    "4:05760592a1391867052b6b68348099c6008dc8998d2fc8b5f957c96bb280a2d0".to_owned()
                ),
                (
                    "gemini".to_owned(),
                    "26:e3b77210882747c4c478ab6bd48bc4bdb990498a9a04c70e198afe4cbdb020d2"
                        .to_owned()
                ),
                (
                    "glm".to_owned(),
                    "46:21834a76909a74ad83bc30e84c408d6bba0dc5ba2e8cbb7019eee0e4f595078e"
                        .to_owned()
                ),
                (
                    "kimi".to_owned(),
                    "2:bbd95d92bef225aa080c8914667f255d278529032c0ef45110ef643cbc4b804a".to_owned()
                ),
                (
                    "minimax".to_owned(),
                    "19:889ef13b059216f0855dbc1fcb5571e10433c5dd21007096c6c3daad74a27340"
                        .to_owned()
                ),
                (
                    "openai".to_owned(),
                    "15:a2024fafdf9d7b6e171f2eda9f2576ff0771904047529da78d5816b2ebddd22e"
                        .to_owned()
                ),
                ("openrouter".to_owned(), "dynamic".to_owned()),
                (
                    "qwen".to_owned(),
                    "4:6a457f72a703c9f859f015977ecfc74e587d06d46e45d55b753f795f64f088c3".to_owned()
                ),
                ("sn".to_owned(), "dynamic".to_owned())
            ])
        );
    }

    #[test]
    fn catalog_registers_new_provider_profile_with_existing_adapter() {
        let local = [
            (
                CatalogKind::ModelDriver,
                json!({
                    "format": "buckyos.aicc.model-driver-catalog",
                    "schema_version": 1,
                    "schema_revision": 0,
                    "model_driver_id": "vendor",
                    "revision_seq": 2,
                    "models": [{"id": "vendor-model", "api_types": ["llm"]}],
                    "patterns": [],
                    "defaults": {},
                    "variants": [],
                    "version_rules": []
                }),
            ),
            (
                CatalogKind::ProviderRules,
                json!({
                    "format": "buckyos.aicc.provider-rules-catalog",
                    "schema_version": 1,
                    "schema_revision": 0,
                    "revision_seq": 2,
                    "provider_profile_id": "vendor",
                    "metadata_drivers": ["vendor"],
                    "models": [],
                    "patterns": [{
                        "match": "*",
                        "operations": {"llm": "responses.create"}
                    }],
                    "variants": []
                }),
            ),
            (
                CatalogKind::KnownProvider,
                json!({
                    "format": "buckyos.aicc.known-provider-catalog",
                    "schema_version": 1,
                    "schema_revision": 0,
                    "revision_seq": 2,
                    "catalog_id": "vendor",
                    "providers": [{
                        "provider_profile_id": "vendor",
                        "display_name": "Vendor",
                        "base_url": "https://vendor.example/v1",
                        "protocol_adapter_id": "openai-responses",
                        "discovery_behavior_id": "openai-compatible-models",
                        "provider_rules_id": "vendor",
                        "credential": {"kind": "bearer"},
                        "connection": {
                            "region": {"mode": "unsupported"},
                            "workspace": {"mode": "unsupported"},
                            "account": {"mode": "unsupported"}
                        }
                    }]
                }),
            ),
        ]
        .into_iter()
        .map(|(kind, document)| {
            MetadataFile::parse(
                MetadataSource::Local,
                kind,
                serde_json::to_vec(&document).unwrap(),
            )
            .unwrap()
        })
        .collect();
        let catalog = MetadataSources {
            builtin: load_builtin_metadata().unwrap(),
            local,
            ..MetadataSources::default()
        }
        .build_snapshot(2, &crate::catalog::CatalogBuildOptions::default())
        .unwrap();

        let registry = builtin_provider_registry(catalog.as_ref()).unwrap();
        let binding = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: "vendor",
                protocol_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                auth_mode: ProviderAuthMode::ApiKey,
                credential_kind: None,
                configured_inventory: None,
            })
            .unwrap();

        assert_eq!(binding.profile.provider_profile_id, "vendor");
        assert_eq!(binding.profile.discovery_mode, DiscoveryMode::MachineApi);
        assert_eq!(
            binding.profile.default_inventory.unwrap().models[0].provider_model_id,
            "vendor-model"
        );
    }

    #[test]
    fn metadata_source_manager_supplies_all_builtin_catalogs_to_registry() {
        let registry = registry();
        let files = load_builtin_metadata().unwrap();
        assert_eq!(files.len(), 35);
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
            11
        );

        let snapshot = MetadataSources {
            builtin: files,
            ..MetadataSources::default()
        }
        .build_snapshot(2, &crate::catalog::CatalogBuildOptions::default())
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
        .build_snapshot(2, &crate::catalog::CatalogBuildOptions::default())
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
    fn fal_uses_catalog_default_inventory_without_configured_discovery() {
        let registry = registry();
        let binding = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: FAL_PROVIDER_PROFILE_ID,
                protocol_adapter_id: FAL_QUEUE_ADAPTER_ID,
                auth_mode: ProviderAuthMode::ApiKey,
                credential_kind: None,
                configured_inventory: None,
            })
            .unwrap();
        let inventory = binding.profile.default_inventory.unwrap();
        assert_eq!(inventory.revision.as_deref(), Some("catalog-fal-1"));
        assert_eq!(inventory.models.len(), 4);
        assert!(inventory
            .models
            .iter()
            .all(|model| model.availability == ModelAvailability::Available));
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
        .build_snapshot(2, &crate::catalog::CatalogBuildOptions::default())
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
        for profile_id in [DOUBAO_PROFILE_ID, QWEN_PROFILE_ID] {
            assert_eq!(
                registry
                    .providers
                    .get(profile_id)
                    .unwrap()
                    .profile
                    .discovery_mode,
                crate::provider::DiscoveryMode::MachineApi
            );
        }
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
                request_timeout: std::time::Duration::from_secs(120),
                auto_sync_models: true,
                instance_rules: None,
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
                    .then(buckyos_api::ProviderInstanceRules::default)
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
        assert_eq!(binding.profile.credential.kind, CredentialKind::NamedHeader);
        assert_eq!(
            binding.profile.credential.header_name.as_deref(),
            Some("x-api-key")
        );
        assert_eq!(binding.instance_rules, Some(Default::default()));
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

        let missing_inventory = registry
            .resolve(BuiltinProviderRequest {
                provider_profile_id: CUSTOM_PROVIDER_PROFILE_ID,
                protocol_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                auth_mode: ProviderAuthMode::ApiKey,
                credential_kind: None,
                configured_inventory: None,
            })
            .unwrap();
        assert_eq!(
            missing_inventory.profile.discovery_mode,
            DiscoveryMode::MachineApi
        );

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

        for (adapter, kind, header) in [
            (OPENAI_RESPONSES_ADAPTER_ID, CredentialKind::Bearer, None),
            (
                crate::protocol::CLAUDE_MESSAGES_ADAPTER_ID,
                CredentialKind::NamedHeader,
                Some("x-api-key"),
            ),
            (
                crate::protocol::GEMINI_ADAPTER_ID,
                CredentialKind::NamedHeader,
                Some("x-goog-api-key"),
            ),
            (FAL_QUEUE_ADAPTER_ID, CredentialKind::FalKey, None),
        ] {
            let binding = registry
                .resolve(BuiltinProviderRequest {
                    provider_profile_id: CUSTOM_PROVIDER_PROFILE_ID,
                    protocol_adapter_id: adapter,
                    auth_mode: ProviderAuthMode::ApiKey,
                    credential_kind: None,
                    configured_inventory: Some(configured_inventory()),
                })
                .unwrap();
            assert_eq!(binding.profile.credential.kind, kind);
            assert_eq!(binding.profile.credential.header_name.as_deref(), header);
        }
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
