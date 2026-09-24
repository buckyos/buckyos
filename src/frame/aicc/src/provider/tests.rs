use super::*;
use crate::catalog::{
    CatalogBuildOptions, CatalogDocuments, ModelDriverCatalog, ProviderRulesCatalog,
};
use crate::model::{LogicalModelDefinition, ModelRegistry, MountMode, RegistryLayers};
use crate::protocol::{
    AdapterDescriptor, AdapterStatus, CodecCall, ExecutionMode, HttpRequest, HttpResponse,
    OperationBinding, OperationCodec, OperationDescriptor, ProtocolError, ProtocolExecution,
    ProtocolResultValue,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

struct FakeCodec {
    descriptor: OperationDescriptor,
}

#[test]
fn provider_auth_modes_are_structurally_exclusive() {
    let api_key: ProviderAuthConfig = serde_json::from_value(serde_json::json!({
        "mode": "api_key",
        "credential_ref": "system-config://secrets/aicc/sn-main"
    }))
    .unwrap();
    assert_eq!(api_key.mode(), ProviderAuthMode::ApiKey);
    assert_eq!(
        api_key.credential_reference().unwrap().reference,
        "system-config://secrets/aicc/sn-main"
    );
    assert!(api_key.validate().is_ok());

    let glm_jwt: ProviderAuthConfig = serde_json::from_value(serde_json::json!({
        "mode": "api_key",
        "credential_ref": "system-config://secrets/aicc/glm-main",
        "credential_kind": "glm_jwt"
    }))
    .unwrap();
    assert_eq!(glm_jwt.credential_kind(), Some(CredentialKind::GlmJwt));

    let dynamic: ProviderAuthConfig = serde_json::from_value(serde_json::json!({
        "mode": "dynamic_login",
        "login_profile": "device_jwt",
        "login_endpoint": "https://sn.example/api/user/login_by_device_token"
    }))
    .unwrap();
    assert_eq!(dynamic.mode(), ProviderAuthMode::DynamicLogin);
    assert!(dynamic.credential_reference().is_none());
    let context = dynamic.dynamic_login_context("sn-main", "alice").unwrap();
    assert_eq!(context.cache_key(), "sn-main");
    assert_eq!(context.user_name, "alice");

    assert!(
        serde_json::from_value::<ProviderAuthConfig>(serde_json::json!({
            "mode": "api_key",
            "credential_ref": "secret-ref",
            "login_profile": "device_jwt",
            "login_endpoint": "https://sn.example/login"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ProviderAuthConfig>(serde_json::json!({
            "mode": "dynamic_login",
            "credential_ref": "secret-ref",
            "login_profile": "device_jwt",
            "login_endpoint": "https://sn.example/login"
        }))
        .is_err()
    );
}

#[test]
fn provider_connection_resolves_workspace_and_default_base_url() {
    let contract = ProviderConnectionContract {
        default_base_url: "https://{workspace}.{region}.maas.example/compatible-mode/v1".into(),
        region: ProviderFieldSchema::optional_with_default("cn-beijing")
            .with_allowed_values(["cn-beijing", "cn-shanghai"]),
        workspace: ProviderFieldSchema::required(),
        account: ProviderFieldSchema::optional(),
        region_base_urls: BTreeMap::new(),
    };
    let resolved = contract
        .resolve(ProviderConnectionInput {
            workspace: Some("workspace-1"),
            account: Some("account_1"),
            ..ProviderConnectionInput::default()
        })
        .unwrap();
    assert_eq!(
        resolved.base_url,
        "https://workspace-1.cn-beijing.maas.example/compatible-mode/v1"
    );
    assert_eq!(resolved.region.as_deref(), Some("cn-beijing"));
    assert_eq!(resolved.workspace.as_deref(), Some("workspace-1"));
    assert_eq!(resolved.account.as_deref(), Some("account_1"));

    let overridden = contract
        .resolve(ProviderConnectionInput {
            base_url: Some("https://gateway.example/v1"),
            region: Some("cn-shanghai"),
            workspace: Some("workspace-1"),
            ..ProviderConnectionInput::default()
        })
        .unwrap();
    assert_eq!(overridden.base_url, "https://gateway.example/v1");
    assert_eq!(overridden.region.as_deref(), Some("cn-shanghai"));
}

#[test]
fn provider_connection_rejects_missing_or_unsupported_fields() {
    let contract = ProviderConnectionContract {
        default_base_url: "https://{workspace}.example/v1".into(),
        region: ProviderFieldSchema::unsupported(),
        workspace: ProviderFieldSchema::required(),
        account: ProviderFieldSchema::unsupported(),
        region_base_urls: BTreeMap::new(),
    };
    assert!(matches!(
        contract.resolve(ProviderConnectionInput::default()),
        Err(ProviderError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        contract.resolve(ProviderConnectionInput {
            region: Some("global"),
            workspace: Some("workspace-1"),
            ..ProviderConnectionInput::default()
        }),
        Err(ProviderError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        contract.resolve(ProviderConnectionInput {
            workspace: Some("unsafe/workspace"),
            ..ProviderConnectionInput::default()
        }),
        Err(ProviderError::InvalidConfiguration(_))
    ));
}

#[async_trait]
impl OperationCodec for FakeCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        ApiType::Llm
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, _call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        Err(ProtocolError::invalid_request("not used by provider tests"))
    }

    async fn decode(&self, _response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        Err(ProtocolError::invalid_response(
            "not used by provider tests",
        ))
    }
}

#[derive(Default)]
struct MemoryStore {
    records: Mutex<BTreeMap<String, InventoryLkgsRecord>>,
    commits: AtomicUsize,
}

#[async_trait]
impl ProviderInventoryStore for MemoryStore {
    async fn load(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<Option<InventoryLkgsRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .get(provider_instance_name)
            .cloned())
    }

    async fn commit(&self, record: &InventoryLkgsRecord) -> ProviderResult<()> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        self.records
            .lock()
            .await
            .insert(record.provider_instance_name.clone(), record.clone());
        Ok(())
    }
}

struct ScriptedDiscovery {
    results: Mutex<VecDeque<Result<ProviderDiscoverySnapshot, String>>>,
    fallback: ProviderDiscoverySnapshot,
    calls: AtomicUsize,
}

impl ScriptedDiscovery {
    fn new(
        results: impl IntoIterator<Item = Result<ProviderDiscoverySnapshot, String>>,
        fallback: ProviderDiscoverySnapshot,
    ) -> Self {
        Self {
            results: Mutex::new(results.into_iter().collect()),
            fallback,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ProviderDiscovery for ScriptedDiscovery {
    async fn discover(
        &self,
        _context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.results.lock().await.pop_front() {
            Some(Ok(snapshot)) => Ok(snapshot),
            Some(Err(error)) => Err(ProviderError::Discovery(error)),
            None => Ok(self.fallback.clone()),
        }
    }
}

struct WorkspaceRecordingDiscovery {
    snapshot: ProviderDiscoverySnapshot,
    workspaces: Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl ProviderDiscovery for WorkspaceRecordingDiscovery {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        self.workspaces
            .lock()
            .await
            .push(context.instance.workspace.clone());
        Ok(self.snapshot.clone())
    }
}

struct BlockingDiscovery {
    snapshot: ProviderDiscoverySnapshot,
    calls: AtomicUsize,
    started: Notify,
    release: Notify,
}

#[async_trait]
impl ProviderDiscovery for BlockingDiscovery {
    async fn discover(
        &self,
        _context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(self.snapshot.clone());
        }
        self.started.notify_one();
        self.release.notified().await;
        Ok(self.snapshot.clone())
    }
}

fn profile() -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: "openai".into(),
        display_name: "OpenAI".into(),
        default_protocol_adapter_id: "openai-responses".into(),
        credential: CredentialDescriptor {
            kind: CredentialKind::Bearer,
            header_name: None,
        },
        credential_variants: vec![CredentialDescriptor {
            kind: CredentialKind::GlmJwt,
            header_name: None,
        }],
        discovery_mode: DiscoveryMode::MachineApi,
        refresh: RefreshPolicy {
            interval: Duration::from_secs(3_600),
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(40),
        },
        default_inventory: None,
        accepts_any_adapter: false,
    }
}

fn instance(name: &str) -> ProviderInstanceConfig {
    ProviderInstanceConfig {
        provider_instance_name: name.into(),
        provider_profile_id: "openai".into(),
        protocol_adapter_id: "openai-responses".into(),
        base_url: "https://api.example.test/v1/".into(),
        credential: CredentialReference {
            reference: "system-config://secrets/aicc/openai".into(),
        },
        credential_kind: None,
        provider_rules_id: None,
        region: None,
        workspace: None,
        account: None,
        request_timeout: Duration::from_secs(120),
        auto_sync_models: true,
        instance_rules: None,
    }
}

fn discovery(model_id: &str) -> ProviderDiscoverySnapshot {
    ProviderDiscoverySnapshot {
        revision: Some("remote-1".into()),
        discovered_at_ms: 100,
        health: ProviderHealthState::Healthy,
        models: vec![DiscoveredModel {
            provider_model_id: model_id.into(),
            origin_model_id: None,
            api_types: Some(vec![ApiType::Llm, ApiType::EmbeddingText]),
            supported_features: Some(BTreeSet::from([
                buckyos_api::features::TOOL_CALL.into(),
                buckyos_api::features::JSON_SCHEMA.into(),
            ])),
            remote_methods: Some(BTreeSet::from(["responses.create".into()])),
            availability: ModelAvailability::Available,
            deprecated: false,
            pricing: Some(Pricing {
                currency: "USD".into(),
                input_token: Some(0.5),
                output_token: None,
                cache_input_token: None,
                estimated_cost: None,
                unit: None,
                amount: None,
                rules: vec![],
                tiers: None,

                time_windows: Vec::new(),
            }),
        }],
    }
}

fn catalog() -> Arc<CatalogSnapshot> {
    catalog_with_revision(7, 8192)
}

fn catalog_with_revision(revision_seq: u64, context_tokens: u64) -> Arc<CatalogSnapshot> {
    let model_driver: ModelDriverCatalog = serde_json::from_value(serde_json::json!({
        "format": "buckyos.aicc.model-driver-catalog",
        "schema_version": 1,
        "schema_revision": 0,
        "model_driver_id": "openai",
        "revision_seq": revision_seq,
        "models": [{
            "id": "gpt-test",
            "api_types": ["llm", "embedding.text"],
            "logical_mounts": ["llm.test"],
            "capabilities": {
                "tool_call": true,
                "json_schema": true,
                "context_tokens": context_tokens
            }
        }],
        "model_pricing": [{"id": "gpt-test", "pricing": {"currency": "USD", "input_token": 9.0}}],
        "patterns": [],
        "defaults": {},
        "variants": [],
        "version_rules": []
    }))
    .unwrap();
    let provider_rules: ProviderRulesCatalog = serde_json::from_value(serde_json::json!({
        "format": "buckyos.aicc.provider-rules-catalog",
        "schema_version": 1,
        "schema_revision": 0,
        "revision_seq": revision_seq,
        "provider_profile_id": "openai",
        "metadata_drivers": ["openai"],
        "models": [{
            "id": "gpt-test",
            "operations": {"llm": "responses.create"}
        }],
        "model_pricing": [{"id": "gpt-test", "pricing": {"currency": "USD", "input_token": 2.0}}],
        "patterns": [],
        "variants": []
    }))
    .unwrap();
    Arc::new(
        CatalogSnapshot::build(
            revision_seq,
            CatalogDocuments {
                model_drivers: vec![model_driver],
                provider_rules: vec![provider_rules],
                known_providers: vec![],
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap(),
    )
}

fn catalog_with_model_ids(revision_seq: u64, model_ids: &[&str]) -> Arc<CatalogSnapshot> {
    let models = model_ids
        .iter()
        .map(|id| serde_json::json!({"id": id, "api_types": ["llm"]}))
        .collect::<Vec<_>>();
    let provider_models = model_ids
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "operations": {"llm": "responses.create"}
            })
        })
        .collect::<Vec<_>>();
    let model_driver: ModelDriverCatalog = serde_json::from_value(serde_json::json!({
        "format": "buckyos.aicc.model-driver-catalog",
        "schema_version": 1,
        "schema_revision": 0,
        "model_driver_id": "vendor",
        "revision_seq": revision_seq,
        "models": models,
        "patterns": [],
        "defaults": {},
        "variants": [],
        "version_rules": []
    }))
    .unwrap();
    let provider_rules: ProviderRulesCatalog = serde_json::from_value(serde_json::json!({
        "format": "buckyos.aicc.provider-rules-catalog",
        "schema_version": 1,
        "schema_revision": 0,
        "revision_seq": revision_seq,
        "provider_profile_id": "vendor",
        "metadata_drivers": ["vendor"],
        "models": provider_models,
        "patterns": [],
        "variants": []
    }))
    .unwrap();
    Arc::new(
        CatalogSnapshot::build(
            revision_seq,
            CatalogDocuments {
                model_drivers: vec![model_driver],
                provider_rules: vec![provider_rules],
                known_providers: vec![],
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn catalog_managed_discovery_refreshes_model_ids_from_new_snapshot() {
    let first = catalog_with_model_ids(1, &["vendor-model-a"]);
    let discovery =
        CatalogOnlyDiscovery::catalog_managed(catalog_only_inventory(&first, "vendor").unwrap());
    let second = catalog_with_model_ids(2, &["vendor-model-a", "vendor-model-b"]);

    discovery.refresh_catalog(&second, "vendor").await.unwrap();

    assert_eq!(
        discovery
            .snapshot
            .read()
            .await
            .models
            .iter()
            .map(|model| model.provider_model_id.as_str())
            .collect::<Vec<_>>(),
        vec!["vendor-model-a", "vendor-model-b"]
    );
}

#[tokio::test]
async fn machine_discovery_failure_uses_static_fallback_as_degraded() {
    let primary = Arc::new(ScriptedDiscovery::new(
        [Err("models API unavailable".to_owned())],
        discovery("remote-model"),
    ));
    let fallback = Arc::new(CatalogOnlyDiscovery::new(discovery("configured-model")));
    let combined = FallbackDiscovery::new(primary.clone(), fallback);
    let credential = ResolvedCredential::bearer("secret://provider", "secret").unwrap();
    let snapshot = combined
        .discover(&DiscoveryContext {
            profile: &profile(),
            instance: &instance("primary"),
            credential: &credential,
        })
        .await
        .unwrap();

    assert_eq!(primary.calls.load(Ordering::SeqCst), 1);
    assert_eq!(snapshot.health, ProviderHealthState::Degraded);
    assert_eq!(snapshot.models[0].provider_model_id, "configured-model");
}

fn routed_catalog() -> Arc<CatalogSnapshot> {
    let driver = |model_driver_id: &str| -> ModelDriverCatalog {
        serde_json::from_value(serde_json::json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": model_driver_id,
            "revision_seq": 7,
            "models": [{
                "id": "shared-model",
                "api_types": ["llm"],
                "capabilities": {"tool_call": true}
            }],
            "patterns": [],
            "defaults": {},
            "variants": [],
            "version_rules": []
        }))
        .unwrap()
    };
    let provider_rules: ProviderRulesCatalog = serde_json::from_value(serde_json::json!({
        "format": "buckyos.aicc.provider-rules-catalog",
        "schema_version": 1,
        "schema_revision": 0,
        "revision_seq": 7,
        "provider_profile_id": "openrouter",
        "metadata_drivers": ["openai", "claude"],
        "origin_provider_aliases": {
            "openai": "openai",
            "anthropic": "claude"
        },
        "origin_mappings": [{
            "extract": {
                "source": "provider_model_id",
                "regex": "^(?<driver>[^/]+)/(?<model>.+)$"
            },
            "transforms": {
                "driver": [
                    {"op": "lowercase"},
                    {
                        "op": "alias",
                        "table": "origin_provider_aliases"
                    }
                ],
                "model": [{"op": "trim"}]
            }
        }],
        "models": [],
        "patterns": [{
            "match": "*",
            "operations": {"llm": "responses.create"}
        }],
        "variants": []
    }))
    .unwrap();
    Arc::new(
        CatalogSnapshot::build(
            7,
            CatalogDocuments {
                model_drivers: vec![driver("openai"), driver("claude")],
                provider_rules: vec![provider_rules],
                known_providers: vec![],
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap(),
    )
}

fn codecs() -> Arc<CodecRegistry> {
    let descriptor = OperationDescriptor {
        operation_id: "responses.create".into(),
        bindings: vec![OperationBinding {
            api_type: ApiType::Llm,
            capability: ApiType::Llm.capability(),
            supported_features: BTreeSet::from([
                buckyos_api::features::TOOL_CALL.into(),
                buckyos_api::features::JSON_SCHEMA.into(),
            ]),
            execution_modes: BTreeSet::from([ExecutionMode::Immediate]),
        }],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: 1024,
        max_response_bytes: 1024,
    };
    let adapter = AdapterDescriptor {
        protocol_family_id: "openai".into(),
        protocol_adapter_id: "openai-responses".into(),
        interface_generation: "responses-v1".into(),
        base_adapter_id: None,
        component_adapter_ids: Vec::new(),
        status: AdapterStatus::Stable,
        probe_priority: 0,
        probe_path: Some("probe".to_owned()),
        credential: crate::protocol::AdapterCredentialContract::bearer(),
        operations: BTreeMap::from([("responses.create".into(), descriptor.clone())]),
    };
    let mut registry = CodecRegistry::default();
    registry
        .register(adapter, vec![Arc::new(FakeCodec { descriptor })])
        .unwrap();
    Arc::new(registry)
}

fn resolver(secret: &str) -> Arc<StaticCredentialResolver> {
    Arc::new(StaticCredentialResolver::new(BTreeMap::from([(
        "system-config://secrets/aicc/openai".into(),
        secret.into(),
    )])))
}

fn manager(
    store: Arc<MemoryStore>,
    discovery: Arc<dyn ProviderDiscovery>,
) -> (Arc<ProviderRuntimeManager>, Arc<dyn ProviderDiscovery>) {
    let manager = Arc::new(
        ProviderRuntimeManager::new(
            [profile()],
            resolver("test-secret"),
            catalog(),
            codecs(),
            store,
        )
        .unwrap(),
    );
    (manager, discovery)
}

fn connection_contract() -> ProviderConnectionContract {
    ProviderConnectionContract {
        default_base_url: "https://{workspace}.example.test/v1/".into(),
        region: ProviderFieldSchema::unsupported(),
        workspace: ProviderFieldSchema::required(),
        account: ProviderFieldSchema::optional(),
        region_base_urls: BTreeMap::new(),
    }
}

fn draft(auth: ProviderAuthConfig) -> ProviderDraftConfig {
    ProviderDraftConfig {
        provider_instance_name: "draft-provider".into(),
        provider_profile_id: "openai".into(),
        protocol_adapter_id: "openai-responses".into(),
        provider_rules_id: None,
        base_url: None,
        region: None,
        workspace: Some("workspace-1".into()),
        account: None,
        auth,
        dynamic_login_user_name: None,
    }
}

#[derive(Default)]
struct FakeDynamicCredentialResolver {
    calls: AtomicUsize,
    contexts: Mutex<Vec<DynamicLoginContext>>,
}

#[async_trait]
impl DynamicLoginCredentialResolver for FakeDynamicCredentialResolver {
    async fn resolve_dynamic(
        &self,
        context: &DynamicLoginContext,
    ) -> ProviderResult<ResolvedCredential> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.contexts.lock().await.push(context.clone());
        ResolvedCredential::bearer("dynamic-login", "draft-secret")
            .map_err(|error| ProviderError::Credential(error.to_string()))
    }
}

#[test]
fn inventory_intersects_capabilities_and_uses_dynamic_pricing() {
    let inventory = InventoryBuilder::build(
        &profile(),
        &instance("primary"),
        discovery("gpt-test"),
        &catalog(),
        &codecs(),
    )
    .unwrap();
    assert_eq!(inventory.metadata_applied_seq, 7);
    assert_eq!(inventory.models.len(), 1);
    let model = &inventory.models[0];
    assert_eq!(model.api_types, vec![ApiType::Llm]);
    assert_eq!(model.operations["llm"], "responses.create");
    assert_eq!(model.capabilities["tool_call"], Value::Bool(true));
    assert_eq!(model.capabilities["json_schema"], Value::Bool(true));
    assert_eq!(model.capabilities["context_tokens"], Value::from(8192));
    assert_eq!(
        model.pricing.as_ref().unwrap().source,
        PricingSource::Discovery
    );
    assert_eq!(model.provider_rules_revision, Some(7));
    assert!(!inventory.provider_model_list_fingerprint.is_empty());
}

#[test]
fn version_rules_rank_numeric_versions_and_classify_tiers() {
    let rule: VersionRule = serde_json::from_value(serde_json::json!({
        "id": "gemini-flash-lite",
        "family": "gemini",
        "tier": "flash-lite",
        "match": "gemini-*-flash-lite",
        "tier_tokens": ["flash", "lite"],
        "exclude_tier_tokens": ["image"],
        "version_rank": { "prefix": "gemini" },
        "stability": {
            "unstable_tokens": ["preview", "beta"],
            "current_requires_stable": true
        },
        "current_mount": "llm.gemini-flash-lite",
        "version_mount": "llm.gemini.{model}",
        "auto_mounts": ["llm", "llm.gemini"]
    }))
    .unwrap();

    assert!(matches_version_tier("gemini-3.5-flash-lite", &rule));
    assert!(!matches_version_tier("gemini-3.5-flash", &rule));
    assert!(!matches_version_tier("gemini-3.1-flash-lite-image", &rule));
    assert!(
        version_rank("gemini-3.10-flash-lite", &rule)
            > version_rank("gemini-3.9-flash-lite", &rule)
    );

    let pro_rule: VersionRule = serde_json::from_value(serde_json::json!({
        "id": "gemini-pro",
        "family": "gemini",
        "tier": "pro",
        "match": "gemini-*",
        "tier_tokens": ["pro"],
        "exclude_tier_tokens": ["image", "tts", "transcribe", "omni"],
        "version_rank": { "prefix": "gemini" },
        "current_mount": "llm.gemini-pro",
        "version_mount": "llm.gemini.{model}"
    }))
    .unwrap();
    assert!(matches_version_tier("gemini-2.5-pro", &pro_rule));
    assert!(!matches_version_tier(
        "gemini-2.5-pro-preview-tts",
        &pro_rule
    ));
    assert!(!version_rank("gemini-3.11-flash-lite-preview", &rule).stable);
    assert_eq!(
        expand_version_mount(&rule.version_mount, "Gemini_3.5/Flash.Lite"),
        "llm.gemini.gemini-3-5-flash-lite"
    );
    assert_eq!(rule.auto_mounts, vec!["llm", "llm.gemini"]);
}

#[test]
fn version_rule_auto_mounts_are_applied_to_inventory_models() {
    let model_driver: ModelDriverCatalog = serde_json::from_value(serde_json::json!({
        "format": "buckyos.aicc.model-driver-catalog",
        "schema_version": 1,
        "schema_revision": 0,
        "model_driver_id": "openai",
        "revision_seq": 9,
        "models": [{
            "id": "gpt-test",
            "api_types": ["llm"],
            "logical_mounts": ["llm.{driver}.{model}"],
            "capabilities": {
                "tool_call": true,
                "json_schema": true
            },
            "version_rules": ["gpt-test-tier"]
        }],
        "patterns": [],
        "defaults": {},
        "variants": [],
        "version_rules": [{
            "id": "gpt-test-tier",
            "family": "gpt",
            "tier": "standard",
            "match": "gpt-*",
            "tier_tokens": ["test"],
            "current_mount": "llm.gpt-standard",
            "version_mount": "llm.openai.{model}",
            "auto_mounts": ["llm", "llm.gpt", "llm.plan", "image.txt2img"]
        }]
    }))
    .unwrap();
    let provider_rules: ProviderRulesCatalog = serde_json::from_value(serde_json::json!({
        "format": "buckyos.aicc.provider-rules-catalog",
        "schema_version": 1,
        "schema_revision": 0,
        "revision_seq": 9,
        "provider_profile_id": "openai",
        "metadata_drivers": ["openai"],
        "models": [{
            "id": "gpt-test",
            "operations": {"llm": "responses.create"}
        }],
        "patterns": [],
        "variants": []
    }))
    .unwrap();
    let catalog = CatalogSnapshot::build(
        9,
        CatalogDocuments {
            model_drivers: vec![model_driver],
            provider_rules: vec![provider_rules],
            known_providers: vec![],
        },
        &CatalogBuildOptions::default(),
    )
    .unwrap();

    let inventory = InventoryBuilder::build(
        &profile(),
        &instance("primary"),
        discovery("gpt-test"),
        &catalog,
        &codecs(),
    )
    .unwrap();
    let model = &inventory.models[0];

    assert!(model.logical_mounts.contains(&"llm".to_string()));
    assert!(model.logical_mounts.contains(&"llm.gpt".to_string()));
    assert!(model.logical_mounts.contains(&"llm.plan".to_string()));
    assert!(model
        .logical_mounts
        .contains(&"llm.gpt-standard".to_string()));
    assert!(model
        .logical_mounts
        .contains(&"llm.openai.gpt-test".to_string()));
    assert!(model
        .logical_mounts
        .iter()
        .all(|mount| !mount.contains('{') && !mount.contains('}')));
    assert!(!model.logical_mounts.contains(&"image.txt2img".to_string()));
}

#[tokio::test]
async fn draft_validation_negotiates_without_runtime_or_storage_side_effects() {
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(WorkspaceRecordingDiscovery {
        snapshot: discovery("gpt-test"),
        workspaces: Mutex::new(Vec::new()),
    });
    let (manager, _) = manager(store.clone(), discovery.clone());
    let draft = draft(ProviderAuthConfig::ApiKey {
        credential_ref: "system-config://secrets/aicc/openai".into(),
        credential_kind: None,
    });

    let negotiated = manager
        .validate_draft(&draft, &connection_contract(), discovery.as_ref(), None)
        .await
        .unwrap();

    assert_eq!(negotiated.provider_profile_id, "openai");
    assert_eq!(negotiated.protocol_adapter_id, "openai-responses");
    assert_eq!(negotiated.auth_mode, ProviderAuthMode::ApiKey);
    assert_eq!(
        negotiated.connection.workspace.as_deref(),
        Some("workspace-1")
    );
    assert_eq!(
        negotiated.connection.base_url,
        "https://workspace-1.example.test/v1/"
    );
    assert_eq!(negotiated.catalog_revision_seq, 7);
    assert_eq!(negotiated.inventory.models.len(), 1);
    assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    assert!(store.records.lock().await.is_empty());
    assert!(manager.runtimes.lock().await.is_empty());
    assert!(manager.registry().await.list().is_empty());
    tokio::task::yield_now().await;
    assert_eq!(
        discovery.workspaces.lock().await.as_slice(),
        &[Some("workspace-1".into())]
    );
}

#[tokio::test]
async fn runtime_uses_explicit_credential_variant() {
    let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let manager = ProviderRuntimeManager::new(
        [profile()],
        resolver("key-id.secret"),
        catalog(),
        codecs(),
        Arc::new(MemoryStore::default()),
    )
    .unwrap();
    let mut config = instance("glm-jwt");
    config.credential_kind = Some(CredentialKind::GlmJwt);

    let executable = manager.start(config, discovery).await.unwrap();
    assert_eq!(
        executable.resolve_credential().await.unwrap().audit().kind,
        CredentialKind::GlmJwt
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn workspace_survives_inventory_build_and_instance_replace() {
    let store = Arc::new(MemoryStore::default());
    let initial_discovery = Arc::new(WorkspaceRecordingDiscovery {
        snapshot: discovery("gpt-test"),
        workspaces: Mutex::new(Vec::new()),
    });
    let (manager, _) = manager(store, initial_discovery.clone());
    let mut initial = instance("primary");
    initial.workspace = Some("workspace-initial".into());
    manager
        .start(initial, initial_discovery.clone())
        .await
        .unwrap();
    assert_eq!(
        manager
            .registry()
            .await
            .get("primary")
            .unwrap()
            .config
            .workspace
            .as_deref(),
        Some("workspace-initial")
    );

    let replacement_discovery = Arc::new(WorkspaceRecordingDiscovery {
        snapshot: discovery("gpt-test"),
        workspaces: Mutex::new(Vec::new()),
    });
    let mut replacement = instance("primary");
    replacement.workspace = Some("workspace-reloaded".into());
    manager
        .replace(replacement, replacement_discovery.clone())
        .await
        .unwrap();
    manager.build_inventory_candidate("primary").await.unwrap();

    assert_eq!(
        replacement_discovery.workspaces.lock().await.as_slice(),
        &[
            Some("workspace-reloaded".into()),
            Some("workspace-reloaded".into())
        ]
    );
    assert_eq!(
        manager
            .registry()
            .await
            .get("primary")
            .unwrap()
            .config
            .workspace
            .as_deref(),
        Some("workspace-reloaded")
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn quota_view_ignores_untrusted_discovery_and_reports_provider_quota_unsupported() {
    let store = Arc::new(MemoryStore::default());
    let mut untrusted_discovery = discovery("gpt-test");
    untrusted_discovery.models[0]
        .supported_features
        .get_or_insert_default()
        .insert("remaining_request_units=999999".into());
    let untrusted_discovery_source = Arc::new(ScriptedDiscovery::new([], untrusted_discovery));
    let (unsupported_manager, unsupported_discovery) = manager(store, untrusted_discovery_source);
    unsupported_manager
        .start(instance("primary"), unsupported_discovery)
        .await
        .unwrap();
    let unsupported = unsupported_manager
        .quota_observation("primary")
        .await
        .unwrap();
    assert_eq!(
        unsupported.state,
        ProviderQuotaObservationState::Unsupported
    );
    assert_eq!(unsupported.remaining_request_units, None);
    assert_eq!(unsupported.remaining_cost_usd, None);
    assert_eq!(unsupported.source, "unsupported");
    unsupported_manager.shutdown().await;
}

#[tokio::test]
async fn draft_validation_resolves_dynamic_login_without_exposing_token() {
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, _) = manager(store.clone(), discovery.clone());
    let resolver = FakeDynamicCredentialResolver::default();
    let mut draft = draft(ProviderAuthConfig::DynamicLogin {
        login_profile: "device_jwt".into(),
        login_endpoint: "https://sn.example.test/login".into(),
    });
    draft.dynamic_login_user_name = Some("alice".into());

    let negotiated = manager
        .validate_draft(
            &draft,
            &connection_contract(),
            discovery.as_ref(),
            Some(&resolver),
        )
        .await
        .unwrap();

    assert_eq!(negotiated.auth_mode, ProviderAuthMode::DynamicLogin);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(resolver.contexts.lock().await[0].user_name, "alice");
    assert!(!format!("{negotiated:?}").contains("draft-secret"));
    assert!(!serde_json::to_string(negotiated.inventory.as_ref())
        .unwrap()
        .contains("draft-secret"));
    assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    assert!(manager.runtimes.lock().await.is_empty());
}

#[tokio::test]
async fn draft_validation_classifies_connection_auth_discovery_and_adapter_failures() {
    let store = Arc::new(MemoryStore::default());
    let discovery_impl = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, _) = manager(store.clone(), discovery_impl.clone());

    let mut invalid_connection = draft(ProviderAuthConfig::ApiKey {
        credential_ref: "system-config://secrets/aicc/openai".into(),
        credential_kind: None,
    });
    invalid_connection.workspace = None;
    let error = manager
        .validate_draft(
            &invalid_connection,
            &connection_contract(),
            discovery_impl.as_ref(),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.stage, ProviderDraftValidationStage::Connection);

    let invalid_auth = draft(ProviderAuthConfig::ApiKey {
        credential_ref: "missing-secret".into(),
        credential_kind: None,
    });
    let error = manager
        .validate_draft(
            &invalid_auth,
            &connection_contract(),
            discovery_impl.as_ref(),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.stage, ProviderDraftValidationStage::Authentication);
    assert_eq!(error.kind, ProviderRefreshFailure::Credential);

    let mut invalid_adapter = draft(ProviderAuthConfig::ApiKey {
        credential_ref: "system-config://secrets/aicc/openai".into(),
        credential_kind: None,
    });
    invalid_adapter.protocol_adapter_id = "missing-adapter".into();
    let error = manager
        .validate_draft(
            &invalid_adapter,
            &connection_contract(),
            discovery_impl.as_ref(),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.stage, ProviderDraftValidationStage::Protocol);

    let failed_discovery = ScriptedDiscovery::new(
        [Err("invalid discovery response".into())],
        discovery("gpt-test"),
    );
    let valid = draft(ProviderAuthConfig::ApiKey {
        credential_ref: "system-config://secrets/aicc/openai".into(),
        credential_kind: None,
    });
    let error = manager
        .validate_draft(&valid, &connection_contract(), &failed_discovery, None)
        .await
        .unwrap_err();
    assert_eq!(error.stage, ProviderDraftValidationStage::Discovery);
    assert_eq!(error.kind, ProviderRefreshFailure::Discovery);

    assert_eq!(discovery_impl.calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    assert!(manager.runtimes.lock().await.is_empty());
    assert!(manager.registry().await.list().is_empty());
}

#[test]
fn openai_inventory_satisfies_canonical_tool_and_schema_requirements() {
    let catalog = catalog();
    let inventory = InventoryBuilder::build(
        &profile(),
        &instance("primary"),
        discovery("gpt-test"),
        &catalog,
        &codecs(),
    )
    .unwrap()
    .as_model_inventory();
    let registry = ModelRegistry::build(
        &catalog,
        &[inventory],
        vec![LogicalModelDefinition {
            path: "llm.contract".into(),
            api_type: ApiType::Llm,
            min_line: buckyos_api::ModelRequirement {
                tool_call: true,
                json_schema: true,
                ..buckyos_api::ModelRequirement::default()
            },
            disable_line: buckyos_api::ModelDisable::default(),
            default_options: BTreeMap::new(),
            mount_mode: MountMode::Auto,
            scheduler_profile: buckyos_api::AiccSchedulerProfile::Balanced,
            fallback: None,
            route_policy: buckyos_api::AiccPolicyConfig::default(),
            user_visible_tier: None,
        }],
        RegistryLayers::default(),
    )
    .unwrap();

    let candidates = registry
        .resolve_candidates("llm.contract", ApiType::Llm)
        .unwrap();
    assert_eq!(candidates.candidates.len(), 1);
    assert!(candidates.admissions.iter().all(|record| record.admitted));
}

#[test]
fn inventory_without_remote_revision_uses_model_fingerprint() {
    let catalog = catalog();
    let mut discovered = discovery("gpt-test");
    discovered.revision = None;
    let snapshot = InventoryBuilder::build(
        &profile(),
        &instance("primary"),
        discovered,
        &catalog,
        &codecs(),
    )
    .unwrap();
    let inventory = snapshot.as_model_inventory();
    assert_eq!(
        inventory.inventory_revision,
        snapshot.provider_model_list_fingerprint
    );
    ModelRegistry::build(
        &catalog,
        &[inventory],
        Vec::new(),
        RegistryLayers::default(),
    )
    .unwrap();
}

#[test]
fn inventory_uses_provider_origin_mapping_to_select_unique_driver() {
    let mut profile = profile();
    profile.provider_profile_id = "openrouter".into();
    let mut instance = instance("router");
    instance.provider_profile_id = "openrouter".into();
    instance.provider_rules_id = Some("openrouter".into());
    let mut discovered = discovery("anthropic/shared-model");
    discovered.models[0].origin_model_id = Some("shared-model".into());

    let inventory = InventoryBuilder::build(
        &profile,
        &instance,
        discovered,
        &routed_catalog(),
        &codecs(),
    )
    .unwrap();
    assert_eq!(inventory.models.len(), 1);
    assert_eq!(
        inventory.models[0].provider_model_id,
        "anthropic/shared-model"
    );
    assert_eq!(inventory.models[0].origin_model_id, "shared-model");
    assert_eq!(inventory.models[0].model_driver_id, "claude");
}

#[tokio::test]
async fn resolved_credential_never_enters_inventory_or_debug_output() {
    let secret = "super-secret-value";
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, discovery) = manager(store, discovery);
    let executable = manager.start(instance("primary"), discovery).await.unwrap();
    let credential = executable.resolve_credential().await.unwrap();
    assert!(!format!("{credential:?}").contains(secret));
    let json = serde_json::to_string(executable.current_inventory().await.as_ref()).unwrap();
    assert!(!json.contains(secret));
    assert!(!json.contains("credential"));
    manager.shutdown().await;
}

#[tokio::test]
async fn discovery_failure_falls_back_to_valid_lkgs() {
    let store = Arc::new(MemoryStore::default());
    let good = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (first_manager, good) = manager(store.clone(), good);
    first_manager
        .start(instance("primary"), good)
        .await
        .unwrap();
    first_manager.shutdown().await;

    let failed = Arc::new(ScriptedDiscovery::new(
        [Err("offline".into())],
        discovery("gpt-test"),
    ));
    let (second_manager, failed) = manager(store, failed);
    let executable = second_manager
        .start(instance("primary"), failed)
        .await
        .unwrap();
    assert_eq!(executable.current_inventory().await.models.len(), 1);
    assert_eq!(
        executable.health().await.state,
        ProviderHealthState::Degraded
    );
    second_manager.shutdown().await;
}

#[tokio::test]
async fn unchanged_probe_updates_health_without_rewriting_inventory() {
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, discovery) = manager(store.clone(), discovery);
    manager.start(instance("primary"), discovery).await.unwrap();
    assert_eq!(store.commits.load(Ordering::SeqCst), 1);
    let runtime = manager
        .runtimes
        .lock()
        .await
        .get("primary")
        .cloned()
        .unwrap();
    runtime
        .refresh_once(false, ProviderRefreshTrigger::Manual, true)
        .await
        .unwrap();
    assert_eq!(store.commits.load(Ordering::SeqCst), 1);
    assert_eq!(
        runtime.health.read().await.state,
        ProviderHealthState::Healthy
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn reconciliation_uses_latest_catalog_and_atomically_publishes_registry() {
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, discovery) = manager(store, discovery);
    manager.start(instance("primary"), discovery).await.unwrap();
    let old_registry = manager.registry().await;
    assert_eq!(
        old_registry
            .get("primary")
            .unwrap()
            .current_inventory()
            .await
            .metadata_applied_seq,
        7
    );
    let mut events = manager.subscribe_refresh_events();

    let report = manager
        .reconcile_inventory(catalog_with_revision(8, 16_384))
        .await;

    assert_eq!(manager.current_catalog().await.target_revision_seq(), 8);
    assert_eq!(report.len(), 1);
    assert!(matches!(
        report[0].outcome,
        ProviderRefreshOutcome::Committed {
            metadata_applied_seq: 8,
            ..
        }
    ));
    let event = events.recv().await.unwrap();
    assert_eq!(event.trigger, ProviderRefreshTrigger::Reconciliation);
    assert!(matches!(
        event.outcome,
        ProviderRefreshOutcome::Committed {
            metadata_applied_seq: 8,
            ..
        }
    ));
    let new_registry = manager.registry().await;
    assert_eq!(
        new_registry
            .get("primary")
            .unwrap()
            .current_inventory()
            .await
            .models[0]
            .capabilities["context_tokens"],
        Value::from(16_384)
    );
    assert_eq!(
        old_registry
            .get("primary")
            .unwrap()
            .current_inventory()
            .await
            .metadata_applied_seq,
        7
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn candidate_commit_rejects_catalog_or_build_order_staleness() {
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, discovery) = manager(store, discovery);
    manager.start(instance("primary"), discovery).await.unwrap();

    let stale_catalog = manager.build_inventory_candidate("primary").await.unwrap();
    manager
        .reconcile_inventory(catalog_with_revision(8, 16_384))
        .await;
    assert!(matches!(
        manager
            .commit_inventory_candidate(stale_catalog, ProviderRefreshTrigger::Reconciliation)
            .await,
        Err(ProviderError::StaleCandidate)
    ));

    let stale_order = manager.build_inventory_candidate("primary").await.unwrap();
    let newest = manager.build_inventory_candidate("primary").await.unwrap();
    assert!(matches!(
        manager
            .commit_inventory_candidate(stale_order, ProviderRefreshTrigger::Manual)
            .await,
        Err(ProviderError::StaleCandidate)
    ));
    assert_eq!(
        manager
            .commit_inventory_candidate(newest, ProviderRefreshTrigger::Manual)
            .await
            .unwrap()
            .metadata_applied_seq,
        8
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn scheduled_refresh_publishes_success_and_error_without_credentials() {
    let store = Arc::new(MemoryStore::default());
    let secret = "event-must-not-contain-this-secret";
    let scripted = Arc::new(ScriptedDiscovery::new(
        [
            Ok(discovery("gpt-test")),
            Err("scheduled endpoint unavailable".into()),
        ],
        discovery("gpt-test"),
    ));
    let manager = Arc::new(
        ProviderRuntimeManager::new([profile()], resolver(secret), catalog(), codecs(), store)
            .unwrap(),
    );
    manager.start(instance("primary"), scripted).await.unwrap();
    let mut events = manager.subscribe_refresh_events();
    let runtime = manager.runtime("primary").await.unwrap();

    assert!(matches!(
        runtime
            .refresh_once(false, ProviderRefreshTrigger::Scheduled, true)
            .await,
        Err(ProviderError::Discovery(_))
    ));
    let event = events.recv().await.unwrap();
    assert_eq!(event.trigger, ProviderRefreshTrigger::Scheduled);
    assert!(matches!(
        event.outcome,
        ProviderRefreshOutcome::Failed { .. }
    ));
    assert!(!format!("{event:?}").contains(secret));
    manager.shutdown().await;
}

#[tokio::test]
async fn failed_refresh_preserves_lkgs_and_reports_degraded_health() {
    let store = Arc::new(MemoryStore::default());
    let scripted = Arc::new(ScriptedDiscovery::new(
        [Ok(discovery("gpt-test")), Err("temporary failure".into())],
        discovery("gpt-test"),
    ));
    let (manager, discovery) = manager(store.clone(), scripted);
    let executable = manager.start(instance("primary"), discovery).await.unwrap();
    let original = executable.current_inventory().await;
    assert!(matches!(
        manager.refresh("primary").await,
        Err(ProviderError::Discovery(_))
    ));
    assert_eq!(store.commits.load(Ordering::SeqCst), 1);
    assert_eq!(
        executable
            .current_inventory()
            .await
            .provider_model_list_fingerprint,
        original.provider_model_list_fingerprint
    );
    let health = executable.health().await;
    assert_eq!(health.state, ProviderHealthState::Degraded);
    assert_eq!(health.consecutive_failures, 1);
    assert!(health.last_error.unwrap().contains("temporary failure"));
    manager.shutdown().await;
}

#[tokio::test]
async fn registry_publication_is_copy_on_write_and_stop_is_idempotent() {
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, discovery) = manager(store, discovery);
    let before = manager.registry().await;
    manager.start(instance("primary"), discovery).await.unwrap();
    let after = manager.registry().await;
    assert!(before.get("primary").is_none());
    assert!(after.get("primary").is_some());
    assert_eq!(after.list().len(), 1);

    manager.stop_and_remove("primary").await.unwrap();
    manager.stop_and_remove("primary").await.unwrap();
    assert!(manager.registry().await.get("primary").is_none());
}

#[tokio::test]
async fn disabled_auto_sync_keeps_initial_discovery_without_periodic_task() {
    let store = Arc::new(MemoryStore::default());
    let scripted = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, discovery) = manager(store, scripted.clone());
    let mut config = instance("manual-refresh");
    config.auto_sync_models = false;

    manager.start(config, discovery).await.unwrap();

    assert_eq!(scripted.calls.load(Ordering::SeqCst), 1);
    let runtime = manager
        .runtimes
        .lock()
        .await
        .get("manual-refresh")
        .cloned()
        .unwrap();
    assert!(runtime.task.lock().await.is_none());
    manager.shutdown().await;
}

#[test]
fn instance_rules_exclude_models_before_inventory_publication() {
    let mut config = instance("filtered");
    config.instance_rules = Some(buckyos_api::ProviderInstanceRules {
        exclude_models: BTreeSet::from(["gpt-test".to_string()]),
        origin_model_overrides: BTreeMap::new(),
    });
    let inventory = InventoryBuilder::build(
        &profile(),
        &config,
        discovery("gpt-test"),
        &catalog(),
        &codecs(),
    )
    .unwrap();
    assert!(inventory.models.is_empty());
}

#[test]
fn instance_origin_override_maps_endpoint_ids_without_global_provider_rules() {
    let mut config = instance("doubao-endpoint");
    config.instance_rules = Some(buckyos_api::ProviderInstanceRules {
        exclude_models: BTreeSet::new(),
        origin_model_overrides: BTreeMap::from([("ep-user-specific".into(), "gpt-test".into())]),
    });
    let inventory = InventoryBuilder::build(
        &profile(),
        &config,
        discovery("ep-user-specific"),
        &catalog(),
        &codecs(),
    )
    .unwrap();
    assert_eq!(inventory.models.len(), 1);
    assert_eq!(inventory.models[0].provider_model_id, "ep-user-specific");
    assert_eq!(inventory.models[0].origin_model_id, "gpt-test");
}

#[test]
fn doubao_endpoint_ids_require_an_instance_origin_override() {
    let mut doubao = profile();
    doubao.provider_profile_id = "doubao".into();
    let mut config = instance("doubao-endpoint");
    config.provider_profile_id = "doubao".into();
    let error = InventoryBuilder::build(
        &doubao,
        &config,
        discovery("ep-user-specific"),
        &catalog(),
        &codecs(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("origin_model_overrides"));
}

#[tokio::test]
async fn invalid_lkgs_is_not_used_as_fallback() {
    let store = Arc::new(MemoryStore::default());
    let snapshot = InventoryBuilder::build(
        &profile(),
        &instance("primary"),
        discovery("gpt-test"),
        &catalog(),
        &codecs(),
    )
    .unwrap();
    let mut record = InventoryLkgsRecord::new(
        "primary",
        "openai",
        "openai-responses",
        &snapshot.provider_model_list_fingerprint,
        snapshot.metadata_applied_seq,
        snapshot.inventory_revision.clone(),
        snapshot.discovered_at_ms,
        serde_json::to_value(&snapshot).unwrap(),
        100,
    )
    .unwrap();
    record.provider_model_list_fingerprint = "tampered".into();
    store.records.lock().await.insert("primary".into(), record);
    let failed = Arc::new(ScriptedDiscovery::new(
        [Err("offline".into())],
        discovery("gpt-test"),
    ));
    let (manager, failed) = manager(store, failed);
    assert!(matches!(
        manager.start(instance("primary"), failed).await,
        Err(ProviderError::Discovery(_))
    ));
}

#[test]
fn invalid_discovery_facts_are_rejected() {
    let mut duplicate = discovery("gpt-test");
    duplicate.models.push(duplicate.models[0].clone());
    assert!(matches!(
        InventoryBuilder::build(
            &profile(),
            &instance("primary"),
            duplicate,
            &catalog(),
            &codecs(),
        ),
        Err(ProviderError::Discovery(_))
    ));

    let mut invalid_price = discovery("gpt-test");
    invalid_price.models[0]
        .pricing
        .as_mut()
        .unwrap()
        .input_token = Some(-1.0);
    assert!(matches!(
        InventoryBuilder::build(
            &profile(),
            &instance("primary"),
            invalid_price,
            &catalog(),
            &codecs(),
        ),
        Err(ProviderError::Discovery(_))
    ));
}

#[tokio::test]
async fn invalid_instance_is_rejected_before_discovery() {
    let store = Arc::new(MemoryStore::default());
    let scripted = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
    let (manager, discovery) = manager(store, scripted.clone());
    let mut invalid = instance("primary");
    invalid.base_url = "file:///tmp/provider".into();
    assert!(matches!(
        manager.start(invalid, discovery).await,
        Err(ProviderError::InvalidConfiguration(_))
    ));
    assert_eq!(scripted.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stop_is_idempotent_and_rejects_late_refresh_commit() {
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(BlockingDiscovery {
        snapshot: discovery("gpt-test"),
        calls: AtomicUsize::new(0),
        started: Notify::new(),
        release: Notify::new(),
    });
    let (manager, discovery_trait) = manager(store.clone(), discovery.clone());
    manager
        .start(instance("primary"), discovery_trait)
        .await
        .unwrap();
    assert_eq!(store.commits.load(Ordering::SeqCst), 1);

    let refresh_manager = manager.clone();
    let refresh = tokio::spawn(async move { refresh_manager.refresh("primary").await });
    discovery.started.notified().await;
    manager.stop_and_remove("primary").await.unwrap();
    discovery.release.notify_waiters();
    assert!(matches!(
        refresh.await.unwrap(),
        Err(ProviderError::Stopped)
    ));
    assert_eq!(store.commits.load(Ordering::SeqCst), 1);

    manager.shutdown().await;
}

#[tokio::test]
async fn concurrent_refreshes_are_serialized_per_instance() {
    let store = Arc::new(MemoryStore::default());
    let discovery = Arc::new(BlockingDiscovery {
        snapshot: discovery("gpt-test"),
        calls: AtomicUsize::new(0),
        started: Notify::new(),
        release: Notify::new(),
    });
    let (manager, discovery_trait) = manager(store, discovery.clone());
    manager
        .start(instance("primary"), discovery_trait)
        .await
        .unwrap();

    let first_manager = manager.clone();
    let first = tokio::spawn(async move { first_manager.refresh("primary").await });
    discovery.started.notified().await;
    let second_manager = manager.clone();
    let second = tokio::spawn(async move { second_manager.refresh("primary").await });
    tokio::task::yield_now().await;
    assert_eq!(discovery.calls.load(Ordering::SeqCst), 2);

    discovery.release.notify_one();
    discovery.started.notified().await;
    assert!(first.await.unwrap().is_ok());
    assert_eq!(discovery.calls.load(Ordering::SeqCst), 3);
    discovery.release.notify_one();
    assert!(second.await.unwrap().is_ok());
    manager.shutdown().await;
}

#[test]
fn backoff_is_bounded_and_fingerprint_is_order_independent() {
    let policy = RefreshPolicy {
        interval: Duration::from_secs(1),
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(40),
    };
    assert_eq!(exponential_backoff(&policy, 1), Duration::from_millis(10));
    assert_eq!(exponential_backoff(&policy, 2), Duration::from_millis(20));
    assert_eq!(exponential_backoff(&policy, 9), Duration::from_millis(40));
    let mut discovered = discovery("gpt-test");
    let first = vec![
        discovered.models.remove(0),
        DiscoveredModel {
            provider_model_id: "gpt-other".into(),
            origin_model_id: None,
            api_types: None,
            supported_features: None,
            remote_methods: None,
            availability: ModelAvailability::Unknown,
            deprecated: false,
            pricing: None,
        },
    ];
    let mut second = first.clone();
    second.reverse();
    assert_eq!(
        model_list_fingerprint(&first),
        model_list_fingerprint(&second)
    );
}
