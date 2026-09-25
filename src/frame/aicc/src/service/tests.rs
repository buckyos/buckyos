use super::*;
use buckyos_api::{QuotaState, QuotaView, UsageQueryTimeRange};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct FakeAuthorizer {
    deny: bool,
    tenant: &'static str,
    calls: Mutex<Vec<(&'static str, &'static str)>>,
}

#[async_trait]
impl ServiceAuthorizer for FakeAuthorizer {
    async fn authorize(
        &self,
        _context: &RPCContext,
        action: &'static str,
        resource: &'static str,
    ) -> Result<AuthorizedCaller, RPCErrors> {
        self.calls.lock().await.push((action, resource));
        if self.deny {
            return Err(RPCErrors::NoPermission("denied".to_string()));
        }
        Ok(AuthorizedCaller {
            tenant_id: self.tenant.to_string(),
            user_id: "alice".to_string(),
            app_id: Some("app:test@alice".to_string()),
            token: "caller-token".to_string(),
        })
    }
}

struct FakeSettingsStore {
    value: Mutex<SettingsDocument>,
    writes: AtomicUsize,
    fail_cas: AtomicBool,
    tokens: Mutex<Vec<String>>,
}

impl FakeSettingsStore {
    fn new() -> Self {
        Self {
            value: Mutex::new(SettingsDocument::new(4, AiccSettings::default()).unwrap()),
            writes: AtomicUsize::new(0),
            fail_cas: AtomicBool::new(false),
            tokens: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SettingsStore for FakeSettingsStore {
    async fn load(&self, token: &str) -> Result<StoredSettings, RPCErrors> {
        self.tokens.lock().await.push(token.to_string());
        Ok(StoredSettings {
            document: self.value.lock().await.clone(),
        })
    }

    async fn compare_and_swap(
        &self,
        token: &str,
        expected_revision: u64,
        settings: &AiccSettings,
    ) -> Result<u64, RPCErrors> {
        self.tokens.lock().await.push(token.to_string());
        if self.fail_cas.load(Ordering::SeqCst) {
            return Err(conflict_error(expected_revision, expected_revision + 1));
        }
        let mut current = self.value.lock().await;
        if current.revision != expected_revision {
            return Err(conflict_error(expected_revision, current.revision));
        }
        let next_revision = expected_revision + 1;
        *current = SettingsDocument::new(next_revision, settings.clone()).unwrap();
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(next_revision)
    }
}

struct FakePreparedRuntime {
    runtime: Arc<FakeRuntime>,
    expected: u64,
    candidate: RuntimeAdminSnapshot,
}

#[async_trait]
impl PreparedSettingsRuntime for FakePreparedRuntime {
    fn expected_revision(&self) -> u64 {
        self.expected
    }

    fn settings_revision(&self) -> u64 {
        self.candidate.settings_revision
    }

    async fn publish(self: Box<Self>) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        self.runtime.publishes.fetch_add(1, Ordering::SeqCst);
        *self.runtime.snapshot.lock().await = self.candidate.clone();
        Ok(self.candidate)
    }

    async fn discard(self: Box<Self>) {
        self.runtime.discards.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeRuntime {
    snapshot: Mutex<RuntimeAdminSnapshot>,
    publishes: AtomicUsize,
    discards: AtomicUsize,
}

impl FakeRuntime {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(RuntimeAdminSnapshot {
                settings_revision: 4,
                models: json!({"models": []}),
                ..RuntimeAdminSnapshot::default()
            }),
            publishes: AtomicUsize::new(0),
            discards: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl ServiceRuntime for FakeRuntime {
    async fn capture(&self) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        Ok(self.snapshot.lock().await.clone())
    }

    async fn prepare_settings(
        &self,
        settings: SettingsDocument,
    ) -> Result<Box<dyn PreparedSettingsRuntime>, RPCErrors> {
        let mut candidate = self.snapshot.lock().await.clone();
        candidate.settings_revision = settings.revision;
        candidate.providers = settings
            .settings
            .providers
            .iter()
            .map(provider_public_view)
            .collect();
        candidate.routing = settings.settings.session_config.clone().unwrap_or_default();
        Ok(Box::new(FakePreparedRuntime {
            runtime: Arc::new(Self {
                snapshot: Mutex::new(self.snapshot.lock().await.clone()),
                publishes: AtomicUsize::new(self.publishes.load(Ordering::SeqCst)),
                discards: AtomicUsize::new(self.discards.load(Ordering::SeqCst)),
            }),
            expected: settings.revision.saturating_sub(1),
            candidate,
        }))
    }

    async fn refresh_provider(
        &self,
        _provider_instance_name: &str,
    ) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        Ok(self.snapshot.lock().await.clone())
    }
}

struct SharedFakeRuntime(Arc<FakeRuntime>);

#[async_trait]
impl ServiceRuntime for SharedFakeRuntime {
    async fn capture(&self) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        self.0.capture().await
    }

    async fn prepare_settings(
        &self,
        settings: SettingsDocument,
    ) -> Result<Box<dyn PreparedSettingsRuntime>, RPCErrors> {
        let mut candidate = self.0.snapshot.lock().await.clone();
        candidate.settings_revision = settings.revision;
        candidate.providers = settings
            .settings
            .providers
            .iter()
            .map(provider_public_view)
            .collect();
        candidate.routing = settings.settings.session_config.clone().unwrap_or_default();
        Ok(Box::new(FakePreparedRuntime {
            runtime: self.0.clone(),
            expected: settings.revision.saturating_sub(1),
            candidate,
        }))
    }

    async fn refresh_provider(
        &self,
        provider_instance_name: &str,
    ) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        self.0.refresh_provider(provider_instance_name).await
    }
}

struct FakeValidator {
    calls: AtomicUsize,
}

#[async_trait]
impl ProviderValidator for FakeValidator {
    async fn validate(
        &self,
        request: ProviderValidateRequest,
    ) -> Result<ProviderValidateResponse, RPCErrors> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ProviderValidateResponse {
            base_url_reachable: true,
            auth_valid: true,
            models_discovered: vec!["model-a".to_string()],
            balance_available: true,
            errors: Vec::new(),
            error_details: Vec::new(),
            resolved_protocol_adapter_id: request
                .protocol_adapter_id
                .or(Some("openai-responses".to_string())),
        })
    }
}

struct FakeUsage {
    tenant: Mutex<Option<String>>,
    usage_request: Mutex<Option<QueryUsageRequest>>,
}

#[async_trait]
impl UsageQueryPort for FakeUsage {
    async fn query_usage(
        &self,
        request: QueryUsageRequest,
    ) -> Result<QueryUsageResponse, RPCErrors> {
        *self.usage_request.lock().await = Some(request);
        Ok(QueryUsageResponse::default())
    }

    async fn query_trace(
        &self,
        tenant_id: &str,
        _request: QueryRouteTraceRequest,
    ) -> Result<QueryRouteTraceResponse, RPCErrors> {
        *self.tenant.lock().await = Some(tenant_id.to_string());
        Ok(QueryRouteTraceResponse::default())
    }
}

struct FakeQuota;

#[async_trait]
impl QuotaQueryPort for FakeQuota {
    async fn query_quota(
        &self,
        _caller: &AuthorizedCaller,
        _request: QuotaQueryRequest,
    ) -> Result<QuotaQueryResponse, RPCErrors> {
        Ok(QuotaQueryResponse {
            quota: QuotaView {
                state: QuotaState::Normal,
                remaining_request_units: Some(10),
                remaining_cost: None,
                reset_at: None,
            },
        })
    }
}

struct FakeMetadata;

#[async_trait]
impl DriverMetadataPort for FakeMetadata {
    async fn get(&self) -> Result<DriverMetadataUpdateView, RPCErrors> {
        Ok(disabled_metadata_view(0).settings)
    }

    async fn set(
        &self,
        _token: &str,
        expected_settings_revision: u64,
        _request: DriverMetadataUpdateSetReq,
    ) -> Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        Ok(disabled_metadata_view(
            expected_settings_revision.saturating_add(1),
        ))
    }
}

struct FakeInference {
    calls: Mutex<Vec<(String, Option<String>)>>,
    execution_modes: Mutex<Vec<buckyos_api::AiccExecutionMode>>,
}

#[async_trait]
impl InferencePort for FakeInference {
    async fn resolve_route(
        &self,
        _caller: &AuthorizedCaller,
        request: RouteResolveRequest,
    ) -> Result<RouteResolveResponse, RPCErrors> {
        self.calls
            .lock()
            .await
            .push(("route.resolve".to_string(), request.trace_id.clone()));
        Ok(RouteResolveResponse {
            selected_exact_model: "model-a@primary".to_string(),
            selected_model_uid: "model-uid-a".to_string(),
            provider_instance_name: "primary".to_string(),
            provider_profile_id: "openai".to_string(),
            protocol_adapter_id: "openai-responses".to_string(),
            model_driver_id: "model-a".to_string(),
            provider_driver: None,
            origin_model_id: "model-a".to_string(),
            provider_model_id: "model-a".to_string(),
            operation: request.api_type.typed_method().to_string(),
            enabled_capabilities: Vec::new(),
            disabled_capabilities: Vec::new(),
            fallback_attempts: Vec::new(),
            route_trace: RouteTrace {
                attempts: Vec::new(),
                final_model: Some("model-a@primary".to_string()),
            },
            inventory_revision: "inventory-1".to_string(),
        })
    }

    async fn invoke(&self, _caller: &AuthorizedCaller, call: AiccCall) -> Result<Value, RPCErrors> {
        self.execution_modes
            .lock()
            .await
            .push(call.execution_mode());
        self.calls.lock().await.push((
            call.method().to_string(),
            call.trace_id().map(str::to_owned),
        ));
        Ok(json!({
            "task_id": "task-1",
            "status": "succeeded",
            "event_ref": "event-1"
        }))
    }
}

struct Fixture {
    service: AiccService,
    settings: Arc<FakeSettingsStore>,
    runtime: Arc<FakeRuntime>,
    validator: Arc<FakeValidator>,
    usage: Arc<FakeUsage>,
    authorizer: Arc<FakeAuthorizer>,
    inference: Arc<FakeInference>,
}

fn fixture(deny: bool) -> Fixture {
    let settings = Arc::new(FakeSettingsStore::new());
    let runtime = FakeRuntime::new();
    let validator = Arc::new(FakeValidator {
        calls: AtomicUsize::new(0),
    });
    let usage = Arc::new(FakeUsage {
        tenant: Mutex::new(None),
        usage_request: Mutex::new(None),
    });
    let authorizer = Arc::new(FakeAuthorizer {
        deny,
        tenant: "tenant-a",
        calls: Mutex::new(Vec::new()),
    });
    let inference = Arc::new(FakeInference {
        calls: Mutex::new(Vec::new()),
        execution_modes: Mutex::new(Vec::new()),
    });
    let service = AiccService::new(
        authorizer.clone(),
        settings.clone(),
        Arc::new(SharedFakeRuntime(runtime.clone())),
        validator.clone(),
        usage.clone(),
        Arc::new(FakeQuota),
        Arc::new(FakeMetadata),
    )
    .with_inference(inference.clone());
    Fixture {
        service,
        settings,
        runtime,
        validator,
        usage,
        authorizer,
        inference,
    }
}

fn provider_add() -> ProviderAddRequest {
    let mut request = ProviderAddRequest::new(
        "primary",
        ProviderInstanceType::CloudApi,
        "openai",
        "https://api.example/v1",
        serde_json::from_value(json!({"api_token": {"locked": "top-secret"}})).unwrap(),
    );
    request.protocol_adapter_id = Some("openai-responses".to_string());
    request
}

fn sn_provider(name: &str, auth: Value) -> ProviderSettings {
    let non_deletable = auth.get("mode").and_then(Value::as_str) == Some("dynamic_login");
    ProviderSettings {
        provider_instance_name: name.to_string(),
        provider_type: ProviderInstanceType::CloudApi,
        provider_profile_id: "sn".to_string(),
        protocol_family_id: Some("openai".to_string()),
        protocol_adapter_id: "sn-openai".to_string(),
        base_url: "https://sn.buckyos.ai/api/v1/ai".to_string(),
        operation_base_urls: BTreeMap::new(),
        credentials: serde_json::from_value(json!({"api_token": {"locked": "top-secret"}}))
            .unwrap(),
        enabled: true,
        region: None,
        workspace: None,
        account: None,
        provider_rules_id: None,
        auth: Some(serde_json::from_value(auth).unwrap()),
        discovery: None,
        instance_rules: None,
        timeout_ms: None,
        auto_sync_models: None,
        lifecycle: ProviderLifecyclePolicy { non_deletable },
    }
}

fn provider_public_view(provider: &ProviderSettings) -> ProviderInstanceView {
    ProviderInstanceView {
        provider_instance_name: provider.provider_instance_name.clone(),
        provider_type: provider.provider_type.clone(),
        provider_profile_id: provider.provider_profile_id.clone(),
        protocol_adapter_id: provider.protocol_adapter_id.clone(),
        base_url: provider.base_url.clone(),
        operation_base_urls: provider.operation_base_urls.clone(),
        enabled: provider.enabled,
        auth: ProviderInstanceAuthView {
            mode: Some(ProviderInstanceAuthMode::ApiKey),
            credential_kind: Some("bearer".to_string()),
            configured: true,
        },
        inventory: ProviderInstanceInventoryView {
            state: if provider.enabled {
                ProviderInstanceInventoryState::Loaded
            } else {
                ProviderInstanceInventoryState::Disabled
            },
            revision: provider.enabled.then(|| "inventory-test".to_string()),
            model_count: u64::from(provider.enabled),
            updated_at_ms: provider.enabled.then_some(1),
        },
        health: ProviderInstanceHealthView {
            state: if provider.enabled {
                ProviderInstanceHealthState::Healthy
            } else {
                ProviderInstanceHealthState::Disabled
            },
            checked_at_ms: provider.enabled.then_some(1),
        },
    }
}

#[tokio::test]
async fn aicc_artifact_authorizer_is_tenant_scoped() {
    let storage = Arc::new(
        AiccStorage::open("sqlite::memory:", buckyos_api::RdbBackend::Sqlite)
            .await
            .unwrap(),
    );
    let obj_id = ndn_lib::ObjId::new("cyfile:010203").unwrap();
    storage
        .remember_artifact_scope(&obj_id.to_string(), "tenant-a", "alice", Some("app-a"), 1)
        .await
        .unwrap();
    let target = ResourceTarget::NamedObject { obj_id };
    let own = AuthenticatedResourceAuthorizer {
        tenant_id: "tenant-a".into(),
        caller_id: "alice".into(),
        storage: storage.clone(),
    };
    assert!(own
        .authorize(
            &ResourceAccessContext::new("tenant-a", "alice", "request-a").unwrap(),
            &target,
            ResourceAccessOperation::ReadContent,
        )
        .await
        .is_ok());
    let foreign = AuthenticatedResourceAuthorizer {
        tenant_id: "tenant-b".into(),
        caller_id: "bob".into(),
        storage,
    };
    assert!(foreign
        .authorize(
            &ResourceAccessContext::new("tenant-b", "bob", "request-b").unwrap(),
            &target,
            ResourceAccessOperation::Inspect,
        )
        .await
        .is_err());
}

#[tokio::test]
async fn route_helper_and_typed_handlers_use_inference_port() {
    let fixture = fixture(false);
    let mut route_request = RouteResolveRequest::new(buckyos_api::ApiType::Llm, "llm.chat");
    route_request.trace_id = Some("trace-route".to_string());
    let route = fixture
        .service
        .handle_route_resolve(route_request, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(route.selected_exact_model, "model-a@primary");

    let mut chat_request = LlmChatInvokeRequest::new("model-a@primary", Vec::new());
    chat_request.trace_id = Some("trace-chat".to_string());
    chat_request.execution_mode = buckyos_api::AiccExecutionMode::Stream;
    let chat = fixture
        .service
        .handle_chat_completions_create(chat_request, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(chat.task_id, "task-1");
    assert_eq!(chat.status, AiMethodStatus::Succeeded);

    let mut helper_request = LlmChatHelperRequest::new("llm.chat", Vec::new());
    helper_request.trace_id = Some("trace-helper".to_string());
    helper_request.execution_mode = buckyos_api::AiccExecutionMode::Stream;
    let helper = fixture
        .service
        .handle_helper_llm_chat(helper_request, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(helper.task_id, "task-1");

    let embedding = fixture
        .service
        .handle_embedding_text(
            EmbeddingTextRequest::new("embed-a@primary", Vec::new()),
            RPCContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(embedding.task_id, "task-1");
    assert_eq!(
        fixture.inference.calls.lock().await.as_slice(),
        [
            ("route.resolve".to_string(), Some("trace-route".to_string())),
            (
                buckyos_api::ai_methods::CHAT_COMPLETIONS_CREATE.to_string(),
                Some("trace-chat".to_string()),
            ),
            (
                buckyos_api::ai_methods::HELPER_LLM_CHAT.to_string(),
                Some("trace-helper".to_string()),
            ),
            (buckyos_api::ai_methods::EMBEDDING_TEXT.to_string(), None),
        ]
    );
    assert_eq!(
        fixture.inference.execution_modes.lock().await.as_slice(),
        [
            buckyos_api::AiccExecutionMode::Stream,
            buckyos_api::AiccExecutionMode::Stream,
            buckyos_api::AiccExecutionMode::Immediate,
        ]
    );
}

#[tokio::test]
async fn krpc_dispatch_preserves_stream_mode_for_typed_and_helper_calls() {
    let fixture = fixture(false);
    let inference = fixture.inference.clone();
    let server = AiccHttpServer::new(fixture.service);

    let mut typed = LlmChatInvokeRequest::new("model-a@primary", Vec::new());
    typed.execution_mode = buckyos_api::AiccExecutionMode::Stream;
    let typed_response = server
        .handle_rpc_call(
            RPCRequest {
                method: buckyos_api::ai_methods::CHAT_COMPLETIONS_CREATE.to_string(),
                params: serde_json::to_value(typed).unwrap(),
                seq: 18,
                token: Some("caller-token".to_string()),
                trace_id: Some("gateway-typed-stream".to_string()),
            },
            "127.0.0.1".parse().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(typed_response.seq, 18);

    let mut helper = LlmChatHelperRequest::new("llm.chat", Vec::new());
    helper.execution_mode = buckyos_api::AiccExecutionMode::Stream;
    let helper_response = server
        .handle_rpc_call(
            RPCRequest {
                method: buckyos_api::ai_methods::HELPER_LLM_CHAT.to_string(),
                params: serde_json::to_value(helper).unwrap(),
                seq: 19,
                token: Some("caller-token".to_string()),
                trace_id: Some("gateway-helper-stream".to_string()),
            },
            "127.0.0.1".parse().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(helper_response.seq, 19);
    assert_eq!(
        inference.execution_modes.lock().await.as_slice(),
        [
            buckyos_api::AiccExecutionMode::Stream,
            buckyos_api::AiccExecutionMode::Stream,
        ]
    );
}

#[test]
fn provider_execution_uses_the_resolved_mode_without_priority_guessing() {
    let supported = BTreeSet::from([ExecutionMode::Immediate, ExecutionMode::Stream]);

    assert_eq!(
        RuntimeProviderExecutionPort::execution_mode(ExecutionMode::Stream, &supported).unwrap(),
        ExecutionMode::Stream
    );
    assert!(
        RuntimeProviderExecutionPort::execution_mode(ExecutionMode::NativeTask, &supported)
            .is_err()
    );
}

#[tokio::test]
async fn inference_handlers_fail_closed_on_rbac_denial() {
    let fixture = fixture(true);
    let result = fixture
        .service
        .handle_images_generate(
            TextToImageInvokeRequest::new("image-a@primary", "cat"),
            RPCContext::default(),
        )
        .await;
    assert!(matches!(result, Err(RPCErrors::NoPermission(_))));
    assert!(fixture.inference.calls.lock().await.is_empty());
}

#[tokio::test]
async fn add_uses_caller_token_cas_and_publishes_after_write() {
    let fixture = fixture(false);
    let response = fixture
        .service
        .handle_add_provider(provider_add(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(response.settings_revision, 5);
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runtime.publishes.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.settings.tokens.lock().await.as_slice(),
        ["caller-token", "caller-token"]
    );
    let public = serde_json::to_string(&response).unwrap();
    assert!(!public.contains("top-secret"));
}

#[tokio::test]
async fn cas_failure_discards_candidate_without_runtime_publish() {
    let fixture = fixture(false);
    fixture.settings.fail_cas.store(true, Ordering::SeqCst);
    assert!(fixture
        .service
        .handle_add_provider(provider_add(), RPCContext::default())
        .await
        .is_err());
    assert_eq!(fixture.runtime.publishes.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runtime.discards.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn validate_is_side_effect_free() {
    let fixture = fixture(false);
    let request = validate_request(&provider_add());
    fixture
        .service
        .handle_validate_provider(request, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(fixture.validator.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runtime.publishes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn metadata_update_advances_runtime_with_persisted_revision() {
    let fixture = fixture(false);
    let response = fixture
        .service
        .handle_driver_metadata_update_set(
            DriverMetadataUpdateSetReq::new(false, None, None),
            RPCContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.settings_revision, 5);
    assert_eq!(fixture.runtime.publishes.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.runtime.capture().await.unwrap().settings_revision,
        5
    );
}

#[tokio::test]
async fn usage_and_trace_are_forced_to_callers_tenant() {
    let fixture = fixture(false);
    let request = QueryUsageRequest::new(UsageQueryTimeRange::Last1d);
    fixture
        .service
        .handle_query_usage(request, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(
        fixture
            .usage
            .usage_request
            .lock()
            .await
            .as_ref()
            .unwrap()
            .filters
            .tenant_ids,
        ["tenant-a"]
    );
    fixture
        .service
        .handle_query_trace(QueryRouteTraceRequest::new(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(
        fixture.usage.tenant.lock().await.as_deref(),
        Some("tenant-a")
    );
}

#[tokio::test]
async fn cross_tenant_usage_and_rbac_denial_fail_closed() {
    let scoped = fixture(false);
    let mut request = QueryUsageRequest::new(UsageQueryTimeRange::Last1d);
    request.filters.tenant_ids.push("tenant-b".to_string());
    assert!(matches!(
        scoped
            .service
            .handle_query_usage(request, RPCContext::default())
            .await,
        Err(RPCErrors::NoPermission(_))
    ));

    let denied = fixture(true);
    assert!(matches!(
        denied
            .service
            .handle_list_models(ListModelsRequest::new(), RPCContext::default())
            .await,
        Err(RPCErrors::NoPermission(_))
    ));
}

#[tokio::test]
async fn management_rbac_uses_platform_service_resources() {
    let fixture = fixture(false);
    fixture
        .service
        .handle_list_models(ListModelsRequest::new(), RPCContext::default())
        .await
        .unwrap();
    fixture
        .service
        .handle_add_provider(provider_add(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(
        fixture.authorizer.calls.lock().await.as_slice(),
        [
            ("read", "obj://config/services/aicc/info"),
            ("write", "obj://config/services/aicc/settings"),
        ]
    );
}

#[tokio::test]
async fn provider_list_returns_disabled_instances_revision_and_no_credentials() {
    let fixture = fixture(false);
    let disabled = ProviderSettings {
        provider_instance_name: "disabled-provider".to_string(),
        provider_type: ProviderInstanceType::CloudApi,
        provider_profile_id: "openai".to_string(),
        protocol_family_id: Some("openai".to_string()),
        protocol_adapter_id: "openai-responses".to_string(),
        base_url: "https://api.example/v1".to_string(),
        operation_base_urls: BTreeMap::new(),
        credentials: serde_json::from_value(json!({"api_token": {"locked": "must-not-leak"}}))
            .unwrap(),
        enabled: false,
        region: None,
        workspace: None,
        account: None,
        provider_rules_id: None,
        auth: Some(
            serde_json::from_value(json!({
                "mode": "api_key",
                "credential_ref": "locked://disabled-provider/api_token",
                "credential_kind": "bearer"
            }))
            .unwrap(),
        ),
        discovery: None,
        instance_rules: None,
        timeout_ms: None,
        auto_sync_models: None,
        lifecycle: Default::default(),
    };
    {
        let mut snapshot = fixture.runtime.snapshot.lock().await;
        snapshot.settings_revision = 12;
        snapshot.providers = vec![provider_public_view(&disabled)];
    }

    let response = fixture
        .service
        .handle_list_providers(
            ProviderListRequest::new(Some(
                buckyos_api::ai_methods::CHAT_COMPLETIONS_CREATE.to_string(),
            )),
            RPCContext::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.settings_revision, 12);
    assert_eq!(response.providers.len(), 1);
    assert_eq!(
        response.providers[0].provider_instance_name,
        "disabled-provider"
    );
    assert_eq!(
        response.providers[0].inventory.state,
        ProviderInstanceInventoryState::Disabled
    );
    assert_eq!(
        response.providers[0].health.state,
        ProviderInstanceHealthState::Disabled
    );
    let wire = serde_json::to_string(&response).unwrap();
    assert!(!wire.contains("must-not-leak"));
    assert!(!wire.contains("credential_ref"));
    assert!(!wire.contains("locked://"));
}

#[tokio::test]
async fn routing_update_validates_cas_weights_and_provider_references() {
    let fixture = fixture(false);
    fixture
        .service
        .handle_add_provider(provider_add(), RPCContext::default())
        .await
        .unwrap();
    {
        let current = fixture.settings.value.lock().await.clone();
        let mut settings = current.settings.as_ref().clone();
        settings.session_config = Some(buckyos_api::AiccRouteOverlay {
            revision: Some("keep-me".to_string()),
            ..Default::default()
        });
        *fixture.settings.value.lock().await =
            SettingsDocument::new(current.revision, settings).unwrap();
    }

    let response = fixture
        .service
        .handle_update_routing(
            RoutingUpdateRequest::new(5, BTreeMap::from([("primary".to_string(), 0.25)])),
            RPCContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.settings_revision, 6);
    assert_eq!(response.routing.provider_weights["primary"], 0.25);
    assert_eq!(response.routing.revision.as_deref(), Some("keep-me"));
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.runtime.publishes.load(Ordering::SeqCst), 2);

    let read = fixture
        .service
        .handle_get_routing(RoutingGetRequest::new(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(read.settings_revision, 6);
    assert_eq!(read.routing, response.routing);

    let unknown = fixture
        .service
        .handle_update_routing(
            RoutingUpdateRequest::new(6, BTreeMap::from([("missing".to_string(), 1.0)])),
            RPCContext::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        AiccError::from_krpc_error(&unknown).unwrap().code,
        buckyos_api::AiccErrorCode::InvalidRequest
    );

    for weight in [-0.1, f64::NAN] {
        let invalid = fixture
            .service
            .handle_update_routing(
                RoutingUpdateRequest::new(6, BTreeMap::from([("primary".to_string(), weight)])),
                RPCContext::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            AiccError::from_krpc_error(&invalid).unwrap().code,
            buckyos_api::AiccErrorCode::InvalidRequest
        );
    }

    let conflict = fixture
        .service
        .handle_update_routing(
            RoutingUpdateRequest::new(5, BTreeMap::new()),
            RPCContext::default(),
        )
        .await
        .unwrap_err();
    let conflict = AiccError::from_krpc_error(&conflict).unwrap();
    assert_eq!(
        conflict.code,
        buckyos_api::AiccErrorCode::SettingsRevisionConflict
    );
    assert_eq!(
        conflict.details,
        Some(json!({
            "expected_revision": 5,
            "actual_revision": 6
        }))
    );
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.runtime.publishes.load(Ordering::SeqCst), 2);

    fixture
        .service
        .handle_delete_provider(ProviderDeleteRequest::new("primary"), RPCContext::default())
        .await
        .unwrap();
    assert!(fixture
        .settings
        .value
        .lock()
        .await
        .settings
        .session_config
        .as_ref()
        .unwrap()
        .provider_weights
        .is_empty());
}

#[test]
fn builtin_logical_definitions_make_llm_chat_routable_without_routing_config() {
    let catalog = CatalogSnapshot::from_current_files(
        2,
        [crate::catalog::CurrentCatalogFile {
            kind: crate::catalog::CatalogKind::ModelDriver,
            contents: include_bytes!("../../driver_metadata/models/openai.model.json").to_vec(),
        }],
        &crate::catalog::CatalogBuildOptions::default(),
    )
    .unwrap();
    let inventory = ModelProviderInventory {
        provider_instance_name: "primary".to_string(),
        provider_profile_id: "openai".to_string(),
        protocol_adapter_id: "openai-responses".to_string(),
        inventory_revision: "inventory-1".to_string(),
        models: vec![crate::model::InventoryModel {
            provider_model_id: "gpt-5-mini".to_string(),
            model_driver_id: "openai".to_string(),
            origin_model_id: "gpt-5-mini".to_string(),
            api_types: vec![buckyos_api::ApiType::Llm],
            logical_mounts: vec!["llm.gpt-standard".to_string()],
            variants: Vec::new(),
            capabilities: BTreeMap::new(),
            canonical_fields: BTreeMap::new(),
            attributes: BTreeMap::new(),
            operations: BTreeMap::new(),
        }],
    };
    let registry = ModelRegistry::build(
        &catalog,
        &[inventory],
        builtin_logical_model_definitions(),
        RegistryLayers {
            factory: Some(&builtin_logical_tree_overlay()),
            ..RegistryLayers::default()
        },
    )
    .unwrap();

    let candidates = registry
        .resolve_candidates("llm.chat", buckyos_api::ApiType::Llm)
        .unwrap();
    assert_eq!(candidates.candidates.len(), 1);
    assert_eq!(
        candidates.candidates[0].model.exact_model.as_str(),
        "gpt-5-mini@primary"
    );
    assert_eq!(
        candidates.candidates[0].paths[0].logical_paths,
        vec!["llm.chat", "llm.gpt-standard"]
    );
}

#[test]
fn builtin_logical_definitions_make_llm_chat_routable_with_glm_only() {
    let catalog = CatalogSnapshot::from_current_files(
        2,
        [crate::catalog::CurrentCatalogFile {
            kind: crate::catalog::CatalogKind::ModelDriver,
            contents: include_bytes!("../../driver_metadata/models/glm.model.json").to_vec(),
        }],
        &crate::catalog::CatalogBuildOptions::default(),
    )
    .unwrap();
    let inventory = ModelProviderInventory {
        provider_instance_name: "glm-main".to_string(),
        provider_profile_id: "glm".to_string(),
        protocol_adapter_id: "glm-chat".to_string(),
        inventory_revision: "inventory-1".to_string(),
        models: vec![crate::model::InventoryModel {
            provider_model_id: "glm-5.3".to_string(),
            model_driver_id: "glm".to_string(),
            origin_model_id: "glm-5.3".to_string(),
            api_types: vec![buckyos_api::ApiType::Llm],
            logical_mounts: vec!["llm.glm".to_string()],
            variants: Vec::new(),
            capabilities: BTreeMap::new(),
            canonical_fields: BTreeMap::new(),
            attributes: BTreeMap::new(),
            operations: BTreeMap::new(),
        }],
    };
    let registry = ModelRegistry::build(
        &catalog,
        &[inventory],
        builtin_logical_model_definitions(),
        RegistryLayers {
            factory: Some(&builtin_logical_tree_overlay()),
            ..RegistryLayers::default()
        },
    )
    .unwrap();

    let candidates = registry
        .resolve_candidates("llm.chat", buckyos_api::ApiType::Llm)
        .unwrap();
    assert_eq!(candidates.resolved_logical_path, "llm.chat");
    assert_eq!(candidates.candidates.len(), 1);
    assert_eq!(
        candidates.candidates[0].model.exact_model.as_str(),
        "glm-5.3@glm-main"
    );
    assert_eq!(
        candidates.candidates[0].paths[0].logical_paths,
        vec!["llm.chat", "llm.glm"]
    );

    let directory = model_directory_json(&registry);
    let chat = directory
        .get("llm.chat")
        .and_then(|value| value.as_object())
        .unwrap();
    assert_eq!(chat["glm"]["target"], "llm.glm");
    assert!(!chat
        .values()
        .any(|item| item["target"] == "llm.deepseek-flash"));
    assert!(!chat
        .values()
        .any(|item| item["target"] == "llm.doubao-lite"));
    assert!(directory.get("llm.deepseek-flash").is_none());
    assert!(directory.get("llm.doubao-lite").is_none());
}

#[test]
fn builtin_logical_tree_keeps_custom_provider_auto_admission() {
    let catalog = CatalogSnapshot::from_current_files(
        1,
        [crate::catalog::CurrentCatalogFile {
            kind: crate::catalog::CatalogKind::ModelDriver,
            contents: include_bytes!("../../driver_metadata/models/openai.model.json").to_vec(),
        }],
        &crate::catalog::CatalogBuildOptions::default(),
    )
    .unwrap();
    let inventory = ModelProviderInventory {
        provider_instance_name: "custom".to_string(),
        provider_profile_id: "custom-openai-compatible".to_string(),
        protocol_adapter_id: "openai-responses".to_string(),
        inventory_revision: "inventory-1".to_string(),
        models: vec![crate::model::InventoryModel {
            provider_model_id: "custom-llm".to_string(),
            model_driver_id: "openai".to_string(),
            origin_model_id: "custom-llm".to_string(),
            api_types: vec![buckyos_api::ApiType::Llm],
            logical_mounts: Vec::new(),
            variants: Vec::new(),
            capabilities: BTreeMap::new(),
            canonical_fields: BTreeMap::new(),
            attributes: BTreeMap::new(),
            operations: BTreeMap::new(),
        }],
    };
    let registry = ModelRegistry::build(
        &catalog,
        &[inventory],
        builtin_logical_model_definitions(),
        RegistryLayers {
            factory: Some(&builtin_logical_tree_overlay()),
            ..RegistryLayers::default()
        },
    )
    .unwrap();

    let candidates = registry
        .resolve_candidates("llm.chat", buckyos_api::ApiType::Llm)
        .unwrap();
    assert_eq!(candidates.resolved_logical_path, "llm.chat");
    assert_eq!(candidates.candidates.len(), 1);
    assert_eq!(
        candidates.candidates[0].model.exact_model.as_str(),
        "custom-llm@custom"
    );
    assert_eq!(
        candidates.candidates[0].paths[0].sources,
        vec![crate::model::LogicalItemSource::AutoAdmission]
    );
}

#[test]
fn builtin_logical_definitions_gate_auto_mounts_with_min_line() {
    let catalog = CatalogSnapshot::from_current_files(
        1,
        [crate::catalog::CurrentCatalogFile {
            kind: crate::catalog::CatalogKind::ModelDriver,
            contents: include_bytes!("../../driver_metadata/models/openai.model.json").to_vec(),
        }],
        &crate::catalog::CatalogBuildOptions::default(),
    )
    .unwrap();
    let inventory = ModelProviderInventory {
        provider_instance_name: "primary".to_string(),
        provider_profile_id: "openai".to_string(),
        protocol_adapter_id: "openai-responses".to_string(),
        inventory_revision: "inventory-1".to_string(),
        models: vec![
            crate::model::InventoryModel {
                provider_model_id: "basic".to_string(),
                model_driver_id: "openai".to_string(),
                origin_model_id: "basic".to_string(),
                api_types: vec![buckyos_api::ApiType::Llm],
                logical_mounts: vec!["llm.plan".to_string()],
                variants: Vec::new(),
                capabilities: BTreeMap::new(),
                canonical_fields: BTreeMap::new(),
                attributes: BTreeMap::new(),
                operations: BTreeMap::new(),
            },
            crate::model::InventoryModel {
                provider_model_id: "planner".to_string(),
                model_driver_id: "openai".to_string(),
                origin_model_id: "planner".to_string(),
                api_types: vec![buckyos_api::ApiType::Llm],
                logical_mounts: vec!["llm.plan".to_string()],
                variants: Vec::new(),
                capabilities: BTreeMap::from([
                    ("tool_call".to_string(), true.into()),
                    ("json_schema".to_string(), true.into()),
                    ("max_context_tokens".to_string(), 65_536.into()),
                ]),
                canonical_fields: BTreeMap::new(),
                attributes: BTreeMap::new(),
                operations: BTreeMap::new(),
            },
        ],
    };
    let registry = ModelRegistry::build(
        &catalog,
        &[inventory],
        builtin_logical_model_definitions(),
        RegistryLayers {
            factory: Some(&builtin_logical_tree_overlay()),
            ..RegistryLayers::default()
        },
    )
    .unwrap();

    let candidates = registry
        .resolve_candidates("llm.plan", buckyos_api::ApiType::Llm)
        .unwrap();
    assert_eq!(candidates.candidates.len(), 1);
    assert_eq!(
        candidates.candidates[0].model.exact_model.as_str(),
        "planner@primary"
    );
    let rejected = candidates
        .admissions
        .iter()
        .find(|record| record.exact_model == "basic@primary" && record.logical_path == "llm.plan")
        .unwrap();
    assert!(!rejected.admitted);
    assert!(rejected
        .missing_requirements
        .contains(&"tool_call".to_string()));
}

#[test]
fn builtin_logical_tree_is_not_an_inventory_snapshot() {
    let catalog = CatalogSnapshot::from_current_files(
        1,
        [crate::catalog::CurrentCatalogFile {
            kind: crate::catalog::CatalogKind::ModelDriver,
            contents: include_bytes!("../../driver_metadata/models/openai.model.json").to_vec(),
        }],
        &crate::catalog::CatalogBuildOptions::default(),
    )
    .unwrap();
    let registry = ModelRegistry::build(
        &catalog,
        &[],
        builtin_logical_model_definitions(),
        RegistryLayers {
            factory: Some(&builtin_logical_tree_overlay()),
            ..RegistryLayers::default()
        },
    )
    .unwrap();
    let logical_paths = registry
        .logical_model_views()
        .into_iter()
        .map(|view| view.path)
        .collect::<BTreeSet<_>>();
    assert!(logical_paths.contains("llm.chat"));
    assert!(!logical_paths.contains("llm.gpt-standard"));
    let chat = registry
        .logical_model_views()
        .into_iter()
        .find(|view| view.path == "llm.chat")
        .unwrap();
    assert!(chat.items.iter().any(|item| item.target == "llm.qwen-plus"));
    assert!(chat.items.iter().any(|item| item.target == "llm.minimax"));
    let directory = model_directory_json(&registry);
    assert!(directory.get("llm").is_some());
    assert!(directory.get("llm.chat").is_some());
    assert!(directory.get("image.txt2img").is_some());
    assert!(directory.get("audio.tts").is_some());
    assert!(directory.get("video.txt2video").is_some());
    let definition_paths = logical_definitions_json(&registry)
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|definition| definition["path"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    assert!(definition_paths.contains("image.txt2img"));
    assert!(definition_paths.contains("audio.tts"));
    assert!(definition_paths.contains("video.txt2video"));
    let directory_chat = directory
        .get("llm.chat")
        .and_then(|value| value.as_object())
        .unwrap();
    assert!(!directory_chat
        .values()
        .any(|item| item["target"] == "llm.qwen-plus"));
    assert!(!directory_chat
        .values()
        .any(|item| item["target"] == "llm.minimax"));

    let inventory = ModelProviderInventory {
        provider_instance_name: "primary".to_string(),
        provider_profile_id: "openai".to_string(),
        protocol_adapter_id: "openai-responses".to_string(),
        inventory_revision: "inventory-1".to_string(),
        models: vec![crate::model::InventoryModel {
            provider_model_id: "gpt-5-mini".to_string(),
            model_driver_id: "openai".to_string(),
            origin_model_id: "gpt-5-mini".to_string(),
            api_types: vec![buckyos_api::ApiType::Llm],
            logical_mounts: vec!["llm.gpt-standard".to_string()],
            variants: Vec::new(),
            capabilities: BTreeMap::new(),
            canonical_fields: BTreeMap::new(),
            attributes: BTreeMap::new(),
            operations: BTreeMap::new(),
        }],
    };
    let registry = ModelRegistry::build(
        &catalog,
        &[inventory],
        builtin_logical_model_definitions(),
        RegistryLayers {
            factory: Some(&builtin_logical_tree_overlay()),
            ..RegistryLayers::default()
        },
    )
    .unwrap();
    let logical_paths = registry
        .logical_model_views()
        .into_iter()
        .map(|view| view.path)
        .collect::<BTreeSet<_>>();
    assert!(logical_paths.contains("llm.gpt-standard"));
}

#[tokio::test]
async fn update_and_delete_use_revision_cas_without_exposing_credentials() {
    let fixture = fixture(false);
    fixture
        .service
        .handle_add_provider(provider_add(), RPCContext::default())
        .await
        .unwrap();

    let mut update = ProviderUpdateRequest::new("primary", 5);
    update.base_url = Some("https://api.updated.example/v1".to_string());
    update.credential = Some(
        serde_json::from_value(json!({"api_token": {"locked": "replacement-secret"}})).unwrap(),
    );
    let response = fixture
        .service
        .handle_update_provider(update, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(response.settings_revision, 6);
    assert!(!serde_json::to_string(&response)
        .unwrap()
        .contains("replacement-secret"));
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 2);

    let stale = ProviderUpdateRequest::new("primary", 5);
    let conflict = fixture
        .service
        .handle_update_provider(stale, RPCContext::default())
        .await
        .unwrap_err();
    let conflict = AiccError::from_krpc_error(&conflict).unwrap();
    assert_eq!(
        conflict.code,
        buckyos_api::AiccErrorCode::SettingsRevisionConflict
    );
    assert_eq!(
        conflict.details,
        Some(json!({
            "expected_revision": 5,
            "actual_revision": 6
        }))
    );
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 2);

    let deleted = fixture
        .service
        .handle_delete_provider(ProviderDeleteRequest::new("primary"), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(deleted.settings_revision, Some(7));
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 3);
    assert!(fixture
        .settings
        .value
        .lock()
        .await
        .settings
        .providers
        .is_empty());
}

#[tokio::test]
async fn dynamic_login_sn_provider_cannot_be_deleted() {
    let fixture = fixture(false);
    {
        let current = fixture.settings.value.lock().await.clone();
        let mut settings = current.settings.as_ref().clone();
        settings.providers = vec![sn_provider(
            "sn-ai-provider-default",
            json!({
                "mode": "dynamic_login",
                "login_profile": "sn-router",
                "login_endpoint": "https://sn.buckyos.ai/kapi/sn"
            }),
        )];
        *fixture.settings.value.lock().await =
            SettingsDocument::new(current.revision, settings).unwrap();
    }

    let error = fixture
        .service
        .handle_delete_provider(
            ProviderDeleteRequest::new("sn-ai-provider-default"),
            RPCContext::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        AiccError::from_krpc_error(&error).unwrap().code,
        buckyos_api::AiccErrorCode::InvalidRequest
    );
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture.settings.value.lock().await.settings.providers.len(),
        1
    );
}

#[tokio::test]
async fn api_key_sn_provider_can_be_deleted() {
    let fixture = fixture(false);
    {
        let current = fixture.settings.value.lock().await.clone();
        let mut settings = current.settings.as_ref().clone();
        settings.providers = vec![sn_provider(
            "sn-router-api-key",
            json!({
                "mode": "api_key",
                "credential_ref": "locked://sn-router-api-key/api_token",
                "credential_kind": "bearer"
            }),
        )];
        *fixture.settings.value.lock().await =
            SettingsDocument::new(current.revision, settings).unwrap();
    }

    let deleted = fixture
        .service
        .handle_delete_provider(
            ProviderDeleteRequest::new("sn-router-api-key"),
            RPCContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.settings_revision, Some(5));
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 1);
    assert!(fixture
        .settings
        .value
        .lock()
        .await
        .settings
        .providers
        .is_empty());
}

#[tokio::test]
async fn management_reads_and_krpc_dispatch_use_one_runtime_view() {
    let fixture = fixture(false);
    {
        let mut snapshot = fixture.runtime.snapshot.lock().await;
        snapshot.catalog_revision = 12;
        snapshot.inventory_revision = "generation-7".to_string();
        snapshot.models = json!({"models": [{"exact_model": "model-a@primary"}]});
        snapshot.providers = vec![provider_public_view(&ProviderSettings {
            provider_instance_name: "primary".to_string(),
            provider_type: ProviderInstanceType::CloudApi,
            provider_profile_id: "openai".to_string(),
            protocol_family_id: Some("openai".to_string()),
            protocol_adapter_id: "openai-responses".to_string(),
            base_url: "https://api.example/v1".to_string(),
            operation_base_urls: BTreeMap::new(),
            credentials: serde_json::from_value(json!({"api_token": {"locked": "not-returned"}}))
                .unwrap(),
            enabled: true,
            region: None,
            workspace: None,
            account: None,
            provider_rules_id: None,
            auth: None,
            discovery: None,
            instance_rules: None,
            timeout_ms: None,
            auto_sync_models: None,
            lifecycle: Default::default(),
        })];
        snapshot
            .provider_health
            .insert("model-a@primary".to_string(), json!({"state": "available"}));
        snapshot.provider_catalog = ProviderCatalogResponse {
            catalog_revision: 12,
            providers: vec![buckyos_api::ProviderCatalogEntry {
                provider_profile_id: "openai".to_string(),
                display_name: "OpenAI".to_string(),
                base_url: "https://api.example/v1".to_string(),
                region_base_urls: BTreeMap::new(),
                operation_base_urls: BTreeMap::new(),
                protocol_adapter_id: "openai-responses".to_string(),
                discovery_behavior_id: "openai-models".to_string(),
                dynamic_login_behavior_id: None,
                connection_behavior_id: None,
                provider_rules_id: Some("openai".to_string()),
                ui_hints: BTreeMap::new(),
            }],
        };
    }

    let catalog = fixture
        .service
        .handle_provider_catalog(ProviderCatalogRequest::new(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(catalog.catalog_revision, 12);
    assert!(fixture
        .service
        .handle_list_protocol_adapters(ProtocolAdapterListRequest::new(), RPCContext::default(),)
        .await
        .unwrap()
        .adapters
        .is_empty());
    assert_eq!(
        fixture
            .service
            .handle_list_models(ListModelsRequest::new(), RPCContext::default())
            .await
            .unwrap()["models"][0]["exact_model"],
        "model-a@primary"
    );
    let providers = fixture
        .service
        .handle_list_providers(
            ProviderListRequest::new(Some(
                buckyos_api::ai_methods::CHAT_COMPLETIONS_CREATE.to_string(),
            )),
            RPCContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(providers.providers.len(), 1);
    assert_eq!(providers.settings_revision, 4);
    assert_eq!(providers.inventory_revision, "generation-7");
    assert_eq!(
        fixture
            .service
            .handle_provider_health(
                ProviderHealthRequest::new("model-a@primary"),
                RPCContext::default(),
            )
            .await
            .unwrap()
            .health["state"],
        "available"
    );

    let server = AiccServerHandler::new(fixture.service);
    let response = server
        .handle_rpc_call(
            RPCRequest {
                method: buckyos_api::ai_methods::MODELS_LIST.to_string(),
                params: json!({}),
                seq: 17,
                token: Some("caller-token".to_string()),
                trace_id: Some("management-dispatch".to_string()),
            },
            "127.0.0.1".parse().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.seq, 17);
}

#[tokio::test]
async fn reload_publishes_current_settings_without_persisting() {
    let fixture = fixture(false);
    let response = fixture
        .service
        .handle_reload_settings(ServiceReloadSettingsRequest::new(), RPCContext::default())
        .await
        .unwrap();
    assert!(response.ok);
    assert_eq!(response.settings_revision, 4);
    assert_eq!(fixture.settings.writes.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runtime.publishes.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.settings.tokens.lock().await.as_slice(),
        ["caller-token"]
    );
}

#[test]
fn task_manager_errors_are_redacted() {
    let error = task_manager_error(
        "runner_complete task_id=t-secret",
        RPCErrors::ReasonError("upstream response contained top-secret".to_string()),
    );
    assert_eq!(error.message, "TaskMgr operation failed");
    assert!(!format!("{error:?}").contains("top-secret"));
}

#[test]
fn inline_artifact_replacement_updates_nested_output_values() {
    let old = buckyos_api::ResourceRef::base64("image/png".to_string(), "cG5n".to_string());
    let new = buckyos_api::ResourceRef::url(
        "https://example.invalid/image.png".to_string(),
        Some("image/png".to_string()),
    );
    let mut value = json!({
        "images": [old],
        "nested": {
            "resource": old
        }
    });

    replace_resource_ref_value(&mut value, &old, &new).unwrap();

    assert_eq!(value["images"][0]["kind"], "url");
    assert_eq!(value["nested"]["resource"]["kind"], "url");
    assert!(!value.to_string().contains("cG5n"));
}

#[test]
fn inline_artifact_collection_finds_nested_base64_resources() {
    let mut found = Vec::new();
    collect_inline_base64_resource_refs(
        &json!({
            "image": {"kind":"base64","mime":"image/png","data_base64":"cG5n"},
            "text": "keep"
        }),
        &mut found,
    );
    assert_eq!(found.len(), 1);
    assert!(matches!(
        &found[0],
        buckyos_api::ResourceRef::Base64 { mime, .. } if mime == "image/png"
    ));
}

fn quota_record() -> QuotaTruthRecord {
    QuotaTruthRecord {
        period_start_ms: 1,
        period_end_ms: 10_000,
        max_request_units: Some(100),
        max_cost: Some(buckyos_api::Money::new(10.0, "USD")),
        reset_at: "2026-09-05T00:00:00Z".to_string(),
    }
}

#[test]
fn quota_combines_budget_usage_and_provider_minimum() {
    let usage = buckyos_api::UsageAggregate {
        consumed_request_units: 91,
        finance_totals: vec![buckyos_api::Money::new(9.2, "USD")],
        ..Default::default()
    };
    let provider = ProviderQuotaObservation {
        state: ProviderQuotaObservationState::Normal,
        remaining_request_units: Some(8),
        remaining_cost: Some(buckyos_api::AiCost {
            amount: 0.5,
            currency: "USD".to_string(),
        }),
        reset_at_ms: None,
        observed_at_ms: 1,
        source: "provider-api".to_string(),
    };
    let quota = combine_quota(quota_record(), &usage, Some(&provider)).unwrap();
    assert_eq!(quota.state, Some(QuotaState::NearLimit));
    assert_eq!(quota.remaining_request_units, Some(8));
    assert_eq!(
        quota.remaining_cost,
        Some(buckyos_api::Money::new(0.5, "USD"))
    );
}

#[test]
fn quota_rejects_invalid_local_finance_but_ignores_provider_failure() {
    let incomplete = buckyos_api::UsageAggregate {
        finance_complete: false,
        ..Default::default()
    };
    assert!(combine_quota(quota_record(), &incomplete, None).is_err());

    let mixed = buckyos_api::UsageAggregate {
        finance_totals: vec![
            buckyos_api::Money::new(1.0, "EUR"),
            buckyos_api::Money::new(2.0, "USD"),
        ],
        ..Default::default()
    };
    assert!(combine_quota(quota_record(), &mixed, None).is_err());

    let mismatched = buckyos_api::UsageAggregate {
        finance_totals: vec![buckyos_api::Money::new(1.0, "EUR")],
        ..Default::default()
    };
    assert!(combine_quota(quota_record(), &mismatched, None).is_err());

    let empty = combine_quota(
        quota_record(),
        &buckyos_api::UsageAggregate::default(),
        None,
    )
    .unwrap();
    assert_eq!(
        empty.remaining_cost,
        Some(buckyos_api::Money::new(10.0, "USD"))
    );

    let failed = ProviderQuotaObservation {
        state: ProviderQuotaObservationState::QueryFailed,
        remaining_request_units: None,
        remaining_cost: None,
        reset_at_ms: None,
        observed_at_ms: 1,
        source: "provider-api".to_string(),
    };
    let quota = combine_quota(
        quota_record(),
        &buckyos_api::UsageAggregate::default(),
        Some(&failed),
    )
    .unwrap();
    assert_eq!(quota.state, Some(QuotaState::Normal));
    assert_eq!(quota.remaining_request_units, Some(100));
    assert_eq!(
        quota.remaining_cost,
        Some(buckyos_api::Money::new(10.0, "USD"))
    );
}

#[test]
fn unavailable_provider_quota_is_unknown_and_exhausted_is_preserved() {
    let unsupported = ProviderQuotaObservation {
        state: ProviderQuotaObservationState::Unsupported,
        remaining_request_units: None,
        remaining_cost: None,
        reset_at_ms: None,
        observed_at_ms: 1,
        source: "unsupported".to_string(),
    };
    assert_eq!(
        provider_quota(Some(&unsupported)).state,
        Some(QuotaState::Unknown)
    );
    assert_eq!(provider_quota(None).state, Some(QuotaState::Unknown));

    let exhausted = ProviderQuotaObservation {
        state: ProviderQuotaObservationState::Exhausted,
        remaining_request_units: Some(0),
        remaining_cost: None,
        reset_at_ms: None,
        observed_at_ms: 1,
        source: "provider-api".to_string(),
    };
    assert_eq!(
        provider_quota(Some(&exhausted)).state,
        Some(QuotaState::Exhausted)
    );
}

#[test]
fn exhausted_provider_overrides_normal_budget() {
    let exhausted = ProviderQuotaObservation {
        state: ProviderQuotaObservationState::Exhausted,
        remaining_request_units: Some(0),
        remaining_cost: None,
        reset_at_ms: None,
        observed_at_ms: 1,
        source: "provider-api".to_string(),
    };
    let quota = combine_quota(
        quota_record(),
        &buckyos_api::UsageAggregate::default(),
        Some(&exhausted),
    )
    .unwrap();
    assert_eq!(quota.state, Some(QuotaState::Exhausted));
    assert_eq!(quota.remaining_request_units, Some(0));
}
