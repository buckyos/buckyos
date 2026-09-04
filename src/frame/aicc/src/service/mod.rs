mod cloud_update;

use anyhow::Context;
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, AiccHandler,
    AiccServerHandler, BuckyOSRuntimeType, CancelResponse, CreateTaskExecutor, CreateTaskReq,
    DriverMetadataRuntimeApply, DriverMetadataUpdateSetReq, DriverMetadataUpdateSetResponse,
    DriverMetadataUpdateStatus, DriverMetadataUpdateView, ListModelsRequest,
    ProtocolAdapterListRequest, ProtocolAdapterListResponse, ProviderAddRequest,
    ProviderAddResponse, ProviderCatalogRequest, ProviderCatalogResponse, ProviderDeleteRequest,
    ProviderDeleteResponse, ProviderHealthRequest, ProviderHealthResponse, ProviderListRequest,
    ProviderListResponse, ProviderRefreshModelsRequest, ProviderRefreshModelsResponse,
    ProviderReloadResult, ProviderUpdateRequest, ProviderUpdateResponse, ProviderValidateRequest,
    ProviderValidateResponse, QueryRouteTraceRequest, QueryRouteTraceResponse, QueryUsageRequest,
    QueryUsageResponse, QuotaQueryRequest, QuotaQueryResponse, QuotaState,
    ServiceReloadSettingsRequest, ServiceReloadSettingsResponse, SystemConfigClient,
    SystemConfigError, TaskManagerClient, UsageQueryOutputMode, UsageQueryTimeRange,
    AICC_COMPUTE_TASK_SCHEMA_ID,
};
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, Runner, ServerError, ServerErrorCode,
    ServerResult, StreamInfo,
};
use buckyos_kit::KVAction;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use kRPC::{RPCContext, RPCErrors, RPCRequest};
use kRPC::{RPCHandler, RPCResponse};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

use crate::catalog::CatalogSnapshot;
use crate::execution::{
    ExecutionEngine, ExecutionOutput, ExecutionState, NativeTaskPoll, NativeTaskResumeDescriptor,
    NativeTaskResumeError, PinnedProviderTask, ProviderExecution, ProviderExecutionPort,
    ProviderStartFailure, ResumeCredential, ResumeCredentialKind, TaskBinding, TaskManagerPort,
    TaskSpec,
};
use crate::model::{ModelRegistry, ProviderInventory as ModelProviderInventory, RegistryLayers};
use crate::protocol::{
    AdapterStatus, CodecContext, CodecLimits, CodecRegistry, CredentialKind, ExecutionMode,
    HttpTransport, HttpTransportConfig, NativeTaskInput, NativeTaskOperation, NativeTaskOutput,
    ProtocolError, ProtocolErrorKind,
};
use crate::provider::{
    builtin_provider_codecs, builtin_provider_registry, resolve_sn_provider_instance_with_config,
    BuiltinProviderRequest, CredentialReference, CredentialResolver, ProviderAuthConfig,
    ProviderConnectionInput, ProviderDiscoverySnapshot, ProviderDraftConfig,
    ProviderDraftValidationStage, ProviderInstanceConfig, ProviderQuotaObservation,
    ProviderQuotaObservationState, ProviderRefreshEvent, ProviderRuntimeManager,
    SnCredentialBroker, SnProviderInstanceInput, StaticCredentialResolver,
};
use crate::routing::{
    CallerIdentity, QuotaLookup, QuotaSnapshot, QuotaSourceError, QuotaSourceFactory,
    QuotaTruthPort,
};
use crate::runtime::{
    ConvergenceTrigger, ModelRegistryAssembler, PreparedRuntime, ProviderRuntimeBackend,
    RuntimeBackend, RuntimeFactory,
};
use crate::runtime::{PreparedRuntimeMutation, RuntimeState};
use crate::settings::{
    AiccSettings, MetadataSourceManager, ProductionMetadataOverrideLoader, ProductionRuntimeInputs,
    ProviderSettings, SettingsDocument,
};
use crate::storage::AiccStorage;
use cloud_update::{
    CloudUpdateClientProfile, CloudUpdateConfig, CloudUpdateManager, NdnCloudObjectFetcher,
};

const RESOURCE_SERVICE: &str = "services/aicc";
const RESOURCE_PROVIDERS: &str = "services/aicc/providers";
const RESOURCE_USAGE: &str = "services/aicc/usage";
const RESOURCE_TRACE: &str = "services/aicc/trace";
const RESOURCE_QUOTA: &str = "services/aicc/quota";
const RESOURCE_METADATA: &str = "services/aicc/driver-metadata-update";
const CLOUD_UPDATE_CONFIG_KEY: &str = "services/aicc/driver_metadata_update";

struct AiccHttpServer {
    handler: AiccServerHandler<AiccService>,
}

impl AiccHttpServer {
    fn new(service: AiccService) -> Self {
        Self {
            handler: AiccServerHandler::new(service),
        }
    }
}

#[async_trait]
impl RPCHandler for AiccHttpServer {
    async fn handle_rpc_call(
        &self,
        request: RPCRequest,
        ip_from: std::net::IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        self.handler.handle_rpc_call(request, ip_from).await
    }
}

#[async_trait]
impl HttpServer for AiccHttpServer {
    async fn serve_request(
        &self,
        request: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if request.method() == Method::POST {
            return serve_http_by_rpc_handler(request, info, self).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "method not allowed"
        ))
    }

    fn id(&self) -> String {
        buckyos_api::AICC_SERVICE_SERVICE_NAME.to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

async fn serve_service(
    service: AiccService,
    runtime: Arc<RuntimeState>,
    cloud_update: Arc<CloudUpdateManager>,
    mut provider_events: broadcast::Receiver<ProviderRefreshEvent>,
) -> anyhow::Result<()> {
    let server = Arc::new(AiccHttpServer::new(service));
    let runner = Runner::new(buckyos_api::AICC_SERVICE_SERVICE_PORT);
    runner
        .add_http_server("/kapi/aicc".to_string(), server)
        .context("register /kapi/aicc failed")?;
    let mut events = cloud_update.subscribe();
    let event_runtime = runtime.clone();
    let convergence_task = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(_) => {
                    let _ = event_runtime.metadata_refreshed().await;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    let provider_runtime = runtime.clone();
    let provider_convergence_task = tokio::spawn(async move {
        loop {
            match provider_events.recv().await {
                Ok(event) => {
                    let _ = provider_runtime.provider_refreshed(&event).await;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    cloud_update.start().await;
    let result = tokio::select! {
        result = runner.run() => result.context("AICC HTTP runner failed"),
        signal = tokio::signal::ctrl_c() => signal.context("listen for shutdown signal failed"),
    };
    cloud_update.shutdown().await;
    convergence_task.abort();
    let _ = convergence_task.await;
    provider_convergence_task.abort();
    let _ = provider_convergence_task.await;
    runtime.shutdown().await;
    result
}

pub(crate) async fn run_service() -> anyhow::Result<()> {
    buckyos_kit::init_logging(buckyos_api::AICC_SERVICE_SERVICE_NAME, true);
    let mut api_runtime = init_buckyos_api_runtime(
        buckyos_api::AICC_SERVICE_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::FrameService,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    api_runtime
        .set_main_service_port(buckyos_api::AICC_SERVICE_SERVICE_PORT)
        .await;
    api_runtime.login().await.map_err(anyhow::Error::msg)?;
    let data_dir = api_runtime.get_data_folder().map_err(anyhow::Error::msg)?;
    let buckyos_root_dir = api_runtime.buckyos_root_dir.clone();
    let system_config_url = api_runtime.get_system_config_url();
    let service_token = api_runtime.get_session_token().await;
    let task_manager = api_runtime
        .get_task_mgr_client()
        .await
        .map_err(anyhow::Error::msg)?;
    let client_version = api_runtime
        .device_config
        .as_ref()
        .and_then(|document| {
            document
                .extra_info
                .get("runtime_version")
                .or_else(|| document.extra_info.get("buckyos_version"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| std::env::var("BUCKYOS_VERSION").ok())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    set_buckyos_api_runtime(api_runtime).map_err(anyhow::Error::msg)?;

    let system_config = SystemConfigClient::new(
        Some(system_config_url.as_str()),
        Some(service_token.as_str()),
    );
    let stored_settings = system_config
        .get(crate::settings::AICC_SETTINGS_KEY)
        .await
        .context("load AICC settings")?;
    let settings = SettingsDocument::parse(stored_settings.version, &stored_settings.value)
        .context("parse AICC settings")?;
    let cloud_config = match system_config.get(CLOUD_UPDATE_CONFIG_KEY).await {
        Ok(value) => serde_json::from_str(&value.value).context("parse cloud update config")?,
        Err(SystemConfigError::KeyNotFound(_)) => CloudUpdateConfig::default(),
        Err(error) => return Err(anyhow::Error::msg(error.to_string())),
    };

    let codecs = builtin_provider_codecs().context("build builtin provider codecs")?;
    let metadata_overrides = Arc::new(ProductionMetadataOverrideLoader::new(
        buckyos_root_dir,
        system_config_url.clone(),
        service_token.clone(),
    ));
    let metadata_sources = MetadataSourceManager::new(metadata_overrides)
        .context("initialize metadata source manager")?;
    let cloud_update = CloudUpdateManager::new_with_source_manager(
        data_dir.join("driver_metadata").join("cloud"),
        Arc::new(NdnCloudObjectFetcher::new(service_token.clone())),
        CloudUpdateClientProfile {
            client_version,
            update_channel: std::env::var("BUCKYOS_UPDATE_CHANNEL")
                .unwrap_or_else(|_| "stable".to_string()),
            rollout_group: std::env::var("BUCKYOS_ROLLOUT_GROUP")
                .unwrap_or_else(|_| "default".to_string()),
            supported_features: Default::default(),
        },
        cloud_config,
        metadata_sources.clone(),
    )
    .context("initialize cloud update manager")?;
    let storage = Arc::new(
        AiccStorage::open_from_service_spec()
            .await
            .context("open AICC storage")?,
    );
    let service_factory = Arc::new(ServiceRuntimeFactory::new(storage.clone()));
    let provider_events = service_factory.subscribe_provider_refreshes();
    let factory: Arc<dyn RuntimeFactory> = service_factory;
    let runtime_inputs = ProductionRuntimeInputs::new(metadata_sources, cloud_update.clone());
    let runtime = RuntimeState::bootstrap(settings, runtime_inputs, factory)
        .await
        .context("bootstrap AICC runtime")?;
    let service_runtime: Arc<dyn ServiceRuntime> =
        Arc::new(RuntimeServiceAdapter::new(runtime.clone(), codecs.clone()));
    let execution = Arc::new(ExecutionEngine::new(
        storage.clone(),
        Arc::new(TaskManagerExecutionPort::new(task_manager)),
        Arc::new(RuntimeProviderExecutionPort::new(runtime.clone(), codecs)),
        storage.clone(),
    ));
    let recovery = execution.clone();
    tokio::spawn(async move {
        let _ = recovery.recover().await;
    });
    let quota_factory = Arc::new(QuotaSourceFactory::new(Arc::new(
        SystemConfigQuotaTruthPort::new(
            &system_config_url,
            &service_token,
            storage.clone(),
            runtime.clone(),
        ),
    )));
    let service = AiccService::new(
        Arc::new(RuntimeAuthorizer),
        Arc::new(SystemConfigSettingsStore::new(system_config_url.clone())),
        service_runtime,
        Arc::new(RuntimeProviderValidator::new(
            runtime.clone(),
            storage.clone(),
        )),
        Arc::new(StorageQueryPort::new(storage)),
        Arc::new(RoutingQuotaQueryPort::new(quota_factory)),
        Arc::new(CloudUpdateDriverMetadataPort::new(
            system_config_url,
            cloud_update.clone(),
            runtime.clone(),
        )),
    )
    .with_execution(execution);
    serve_service(service, runtime, cloud_update, provider_events).await
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedCaller {
    pub tenant_id: String,
    pub user_id: String,
    pub app_id: Option<String>,
    pub token: String,
}

#[async_trait]
pub(crate) trait ServiceAuthorizer: Send + Sync {
    async fn authorize(
        &self,
        context: &RPCContext,
        action: &'static str,
        resource: &'static str,
    ) -> Result<AuthorizedCaller, RPCErrors>;
}

#[derive(Clone, Debug)]
pub(crate) struct StoredSettings {
    pub document: SettingsDocument,
}

#[async_trait]
pub(crate) trait SettingsStore: Send + Sync {
    async fn load(&self, token: &str) -> Result<StoredSettings, RPCErrors>;

    async fn compare_and_swap(
        &self,
        token: &str,
        expected_revision: u64,
        settings: &AiccSettings,
    ) -> Result<u64, RPCErrors>;
}

#[async_trait]
pub(crate) trait PreparedSettingsRuntime: Send {
    fn expected_revision(&self) -> u64;
    fn settings_revision(&self) -> u64;
    async fn publish(self: Box<Self>) -> Result<RuntimeAdminSnapshot, RPCErrors>;
    async fn discard(self: Box<Self>);
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimeAdminSnapshot {
    pub settings_revision: u64,
    pub catalog_revision: u64,
    pub provider_catalog: ProviderCatalogResponse,
    pub protocol_adapters: ProtocolAdapterListResponse,
    pub models: Value,
    pub providers: Vec<Value>,
    pub inventory_revision: String,
    pub provider_health: BTreeMap<String, Value>,
}

#[async_trait]
pub(crate) trait ServiceRuntime: Send + Sync {
    async fn capture(&self) -> Result<RuntimeAdminSnapshot, RPCErrors>;
    async fn prepare_settings(
        &self,
        settings: SettingsDocument,
    ) -> Result<Box<dyn PreparedSettingsRuntime>, RPCErrors>;
    async fn refresh_provider(
        &self,
        provider_instance_name: &str,
    ) -> Result<RuntimeAdminSnapshot, RPCErrors>;
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

pub(crate) struct AiccService {
    authorizer: Arc<dyn ServiceAuthorizer>,
    settings: Arc<dyn SettingsStore>,
    runtime: Arc<dyn ServiceRuntime>,
    validator: Arc<dyn ProviderValidator>,
    usage: Arc<dyn UsageQueryPort>,
    quota: Arc<dyn QuotaQueryPort>,
    metadata: Arc<dyn DriverMetadataPort>,
    execution: Option<Arc<ExecutionEngine>>,
    settings_mutation: Mutex<()>,
}

impl AiccService {
    pub(crate) fn new(
        authorizer: Arc<dyn ServiceAuthorizer>,
        settings: Arc<dyn SettingsStore>,
        runtime: Arc<dyn ServiceRuntime>,
        validator: Arc<dyn ProviderValidator>,
        usage: Arc<dyn UsageQueryPort>,
        quota: Arc<dyn QuotaQueryPort>,
        metadata: Arc<dyn DriverMetadataPort>,
    ) -> Self {
        Self {
            authorizer,
            settings,
            runtime,
            validator,
            usage,
            quota,
            metadata,
            execution: None,
            settings_mutation: Mutex::new(()),
        }
    }

    pub(crate) fn with_execution(mut self, execution: Arc<ExecutionEngine>) -> Self {
        self.execution = Some(execution);
        self
    }

    async fn authorize(
        &self,
        context: &RPCContext,
        action: &'static str,
        resource: &'static str,
    ) -> Result<AuthorizedCaller, RPCErrors> {
        self.authorizer.authorize(context, action, resource).await
    }

    async fn mutate_settings<F>(
        &self,
        caller: &AuthorizedCaller,
        requested_revision: Option<u64>,
        mutate: F,
    ) -> Result<RuntimeAdminSnapshot, RPCErrors>
    where
        F: FnOnce(&mut AiccSettings) -> Result<(), RPCErrors> + Send,
    {
        let _guard = self.settings_mutation.lock().await;
        let current = self.settings.load(&caller.token).await?;
        if requested_revision.is_some_and(|revision| revision != current.document.revision) {
            return Err(conflict_error(
                requested_revision.unwrap(),
                current.document.revision,
            ));
        }
        let mut next = current.document.settings.as_ref().clone();
        mutate(&mut next)?;
        let candidate_revision = current.document.revision.saturating_add(1);
        let candidate =
            SettingsDocument::new(candidate_revision, next.clone()).map_err(to_rpc_error)?;
        let prepared = self.runtime.prepare_settings(candidate).await?;
        if prepared.expected_revision() != current.document.revision
            || prepared.settings_revision() != candidate_revision
        {
            prepared.discard().await;
            return Err(RPCErrors::ReasonError(
                "runtime settings revision changed while preparing candidate".to_string(),
            ));
        }
        let persisted_revision = match self
            .settings
            .compare_and_swap(&caller.token, current.document.revision, &next)
            .await
        {
            Ok(revision) if revision == candidate_revision => revision,
            Ok(revision) => {
                prepared.discard().await;
                return Err(RPCErrors::ReasonError(format!(
                    "system-config returned unexpected settings revision {revision}"
                )));
            }
            Err(error) => {
                prepared.discard().await;
                return Err(error);
            }
        };
        let snapshot = prepared.publish().await?;
        if snapshot.settings_revision != persisted_revision {
            return Err(RPCErrors::ReasonError(
                "published runtime revision does not match persisted settings".to_string(),
            ));
        }
        Ok(snapshot)
    }
}

#[async_trait]
impl AiccHandler for AiccService {
    async fn handle_cancel(
        &self,
        task_id: &str,
        ctx: RPCContext,
    ) -> Result<CancelResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_SERVICE).await?;
        let execution = self
            .execution
            .as_ref()
            .ok_or_else(|| RPCErrors::ReasonError("execution runtime is unavailable".into()))?;
        let accepted = execution
            .cancel(&caller.tenant_id, task_id)
            .await
            .map_err(|error| error.to_krpc_error())?;
        Ok(CancelResponse {
            task_id: task_id.to_string(),
            accepted,
        })
    }

    async fn handle_reload_settings(
        &self,
        _request: ServiceReloadSettingsRequest,
        ctx: RPCContext,
    ) -> Result<ServiceReloadSettingsResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_SERVICE).await?;
        let _guard = self.settings_mutation.lock().await;
        let current = self.settings.load(&caller.token).await?;
        let prepared = self
            .runtime
            .prepare_settings(current.document.clone())
            .await?;
        if prepared.settings_revision() != current.document.revision {
            prepared.discard().await;
            return Err(RPCErrors::ReasonError(
                "runtime prepared a different settings revision".to_string(),
            ));
        }
        let snapshot = prepared.publish().await?;
        Ok(ServiceReloadSettingsResponse {
            ok: true,
            settings_revision: snapshot.settings_revision,
        })
    }

    async fn handle_query_quota(
        &self,
        request: QuotaQueryRequest,
        ctx: RPCContext,
    ) -> Result<QuotaQueryResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_QUOTA).await?;
        self.quota.query_quota(&caller, request).await
    }

    async fn handle_query_usage(
        &self,
        mut request: QueryUsageRequest,
        ctx: RPCContext,
    ) -> Result<QueryUsageResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_USAGE).await?;
        if request.filters.tenant_ids.is_empty() {
            request.filters.tenant_ids.push(caller.tenant_id);
        } else if request.filters.tenant_ids != [caller.tenant_id.clone()] {
            return Err(RPCErrors::NoPermission(
                "usage query cannot cross tenant boundary".to_string(),
            ));
        }
        self.usage.query_usage(request).await
    }

    async fn handle_query_trace(
        &self,
        request: QueryRouteTraceRequest,
        ctx: RPCContext,
    ) -> Result<QueryRouteTraceResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "read", RESOURCE_TRACE).await?;
        self.usage.query_trace(&caller.tenant_id, request).await
    }

    async fn handle_provider_catalog(
        &self,
        _request: ProviderCatalogRequest,
        ctx: RPCContext,
    ) -> Result<ProviderCatalogResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_PROVIDERS).await?;
        Ok(self.runtime.capture().await?.provider_catalog)
    }

    async fn handle_list_protocol_adapters(
        &self,
        _request: ProtocolAdapterListRequest,
        ctx: RPCContext,
    ) -> Result<ProtocolAdapterListResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_PROVIDERS).await?;
        Ok(self.runtime.capture().await?.protocol_adapters)
    }

    async fn handle_validate_provider(
        &self,
        request: ProviderValidateRequest,
        ctx: RPCContext,
    ) -> Result<ProviderValidateResponse, RPCErrors> {
        self.authorize(&ctx, "write", RESOURCE_PROVIDERS).await?;
        self.validator.validate(request).await
    }

    async fn handle_add_provider(
        &self,
        request: ProviderAddRequest,
        ctx: RPCContext,
    ) -> Result<ProviderAddResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_PROVIDERS).await?;
        let validation = self.validator.validate(validate_request(&request)).await?;
        if !validation.errors.is_empty() || !validation.error_details.is_empty() {
            return Err(RPCErrors::ReasonError(
                "provider validation failed; settings were not changed".to_string(),
            ));
        }
        let name = request.provider_instance_name.clone();
        let mutation_name = name.clone();
        let provider = provider_from_add(request, validation.resolved_protocol_adapter_id)?;
        let snapshot = self
            .mutate_settings(&caller, None, move |settings| {
                if settings
                    .providers
                    .iter()
                    .any(|existing| existing.provider_instance_name == mutation_name)
                {
                    return Err(RPCErrors::ReasonError(
                        "provider instance already exists".to_string(),
                    ));
                }
                settings.providers.push(provider);
                Ok(())
            })
            .await?;
        Ok(ProviderAddResponse {
            ok: true,
            provider_instance_name: name,
            settings_revision: snapshot.settings_revision,
            reload: reload_result(&snapshot),
        })
    }

    async fn handle_list_providers(
        &self,
        request: ProviderListRequest,
        ctx: RPCContext,
    ) -> Result<ProviderListResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_PROVIDERS).await?;
        let snapshot = self.runtime.capture().await?;
        let providers = if let Some(method) = request.method {
            snapshot
                .providers
                .into_iter()
                .filter(|provider| value_supports_method(provider, &method))
                .collect()
        } else {
            snapshot.providers
        };
        Ok(ProviderListResponse {
            providers,
            inventory_revision: snapshot.inventory_revision,
        })
    }

    async fn handle_provider_health(
        &self,
        request: ProviderHealthRequest,
        ctx: RPCContext,
    ) -> Result<ProviderHealthResponse, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_PROVIDERS).await?;
        let snapshot = self.runtime.capture().await?;
        let health = snapshot
            .provider_health
            .get(&request.exact_model)
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("exact model was not found".to_string()))?;
        Ok(ProviderHealthResponse { health })
    }

    async fn handle_update_provider(
        &self,
        request: ProviderUpdateRequest,
        ctx: RPCContext,
    ) -> Result<ProviderUpdateResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_PROVIDERS).await?;
        let current = self.settings.load(&caller.token).await?;
        if current.document.revision != request.settings_revision {
            return Err(conflict_error(
                request.settings_revision,
                current.document.revision,
            ));
        }
        let mut candidate = current
            .document
            .settings
            .providers
            .iter()
            .find(|provider| provider.provider_instance_name == request.provider_instance_name)
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("provider instance was not found".into()))?;
        apply_provider_update(&mut candidate, request.clone());
        SettingsDocument::new(
            current.document.revision.saturating_add(1),
            AiccSettings {
                providers: vec![candidate.clone()],
                session_config: None,
            },
        )
        .map_err(to_rpc_error)?;
        if candidate.enabled {
            let validation = self
                .validator
                .validate(validate_settings_provider(&candidate))
                .await?;
            if !validation.errors.is_empty() || !validation.error_details.is_empty() {
                return Err(RPCErrors::ReasonError(
                    "provider validation failed; settings were not changed".to_string(),
                ));
            }
        }
        let name = request.provider_instance_name.clone();
        let mutation_name = name.clone();
        let expected = request.settings_revision;
        let snapshot = self
            .mutate_settings(&caller, Some(expected), move |settings| {
                let provider = settings
                    .providers
                    .iter_mut()
                    .find(|provider| provider.provider_instance_name == mutation_name)
                    .ok_or_else(|| {
                        RPCErrors::ReasonError("provider instance was not found".into())
                    })?;
                apply_provider_update(provider, request);
                Ok(())
            })
            .await?;
        let provider = snapshot
            .providers
            .iter()
            .find(|provider| provider["provider_instance_name"] == name)
            .cloned();
        Ok(ProviderUpdateResponse {
            ok: true,
            settings_revision: snapshot.settings_revision,
            provider,
        })
    }

    async fn handle_delete_provider(
        &self,
        request: ProviderDeleteRequest,
        ctx: RPCContext,
    ) -> Result<ProviderDeleteResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_PROVIDERS).await?;
        let name = request.provider_instance_name;
        let response_name = name.clone();
        let snapshot = self
            .mutate_settings(&caller, None, move |settings| {
                let before = settings.providers.len();
                settings
                    .providers
                    .retain(|provider| provider.provider_instance_name != name);
                if settings.providers.len() == before {
                    return Err(RPCErrors::ReasonError(
                        "provider instance was not found".into(),
                    ));
                }
                Ok(())
            })
            .await?;
        Ok(ProviderDeleteResponse {
            ok: true,
            provider_instance_name: Some(response_name),
            settings_revision: Some(snapshot.settings_revision),
            reload: Some(reload_result(&snapshot)),
            reason: None,
        })
    }

    async fn handle_refresh_provider_models(
        &self,
        request: ProviderRefreshModelsRequest,
        ctx: RPCContext,
    ) -> Result<ProviderRefreshModelsResponse, RPCErrors> {
        self.authorize(&ctx, "write", RESOURCE_PROVIDERS).await?;
        let snapshot = self
            .runtime
            .refresh_provider(&request.provider_instance_name)
            .await?;
        Ok(ProviderRefreshModelsResponse {
            ok: true,
            provider_instance_name: request.provider_instance_name,
            inventory_revision: snapshot.inventory_revision,
        })
    }

    async fn handle_list_models(
        &self,
        _request: ListModelsRequest,
        ctx: RPCContext,
    ) -> Result<Value, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_PROVIDERS).await?;
        Ok(self.runtime.capture().await?.models)
    }

    async fn handle_driver_metadata_update_get(
        &self,
        ctx: RPCContext,
    ) -> Result<DriverMetadataUpdateView, RPCErrors> {
        self.authorize(&ctx, "read", RESOURCE_METADATA).await?;
        self.metadata.get().await
    }

    async fn handle_driver_metadata_update_set(
        &self,
        request: DriverMetadataUpdateSetReq,
        ctx: RPCContext,
    ) -> Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        let caller = self.authorize(&ctx, "write", RESOURCE_METADATA).await?;
        let _guard = self.settings_mutation.lock().await;
        let current = self.settings.load(&caller.token).await?;
        let next_revision = current.document.revision.saturating_add(1);
        let candidate =
            SettingsDocument::new(next_revision, current.document.settings.as_ref().clone())
                .map_err(to_rpc_error)?;
        let prepared = self.runtime.prepare_settings(candidate).await?;
        if prepared.expected_revision() != current.document.revision
            || prepared.settings_revision() != next_revision
        {
            prepared.discard().await;
            return Err(RPCErrors::ReasonError(
                "runtime settings revision changed while preparing metadata update".to_string(),
            ));
        }
        let response = match self
            .metadata
            .set(&caller.token, current.document.revision, request)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                prepared.discard().await;
                return Err(error);
            }
        };
        let snapshot = prepared.publish().await?;
        if snapshot.settings_revision != response.settings_revision {
            return Err(RPCErrors::ReasonError(
                "metadata update runtime revision does not match persistence".to_string(),
            ));
        }
        Ok(response)
    }
}

fn validate_request(request: &ProviderAddRequest) -> ProviderValidateRequest {
    ProviderValidateRequest {
        provider_instance_name: Some(request.provider_instance_name.clone()),
        provider_type: request.provider_type.clone(),
        provider_profile_id: request.provider_profile_id.clone(),
        protocol_family_id: request.protocol_family_id.clone(),
        protocol_adapter_id: request.protocol_adapter_id.clone(),
        base_url: request.base_url.clone(),
        credentials: request.credentials.clone(),
        region: request.region.clone(),
        workspace: request.workspace.clone(),
        account: request.account.clone(),
        provider_rules_id: request.provider_rules_id.clone(),
        auth: request.auth.clone(),
        discovery: request.discovery.clone(),
        instance_rules: request.instance_rules.clone(),
        timeout_ms: request.timeout_ms,
        auto_sync_models: request.auto_sync_models,
    }
}

fn validate_settings_provider(provider: &ProviderSettings) -> ProviderValidateRequest {
    ProviderValidateRequest {
        provider_instance_name: Some(provider.provider_instance_name.clone()),
        provider_type: provider.provider_type.clone(),
        provider_profile_id: provider.provider_profile_id.clone(),
        protocol_family_id: None,
        protocol_adapter_id: Some(provider.protocol_adapter_id.clone()),
        base_url: provider.base_url.clone(),
        credentials: provider.credentials.clone(),
        region: provider.region.clone(),
        workspace: provider.workspace.clone(),
        account: provider.account.clone(),
        provider_rules_id: provider.provider_rules_id.clone(),
        auth: provider.auth.clone(),
        discovery: provider.discovery.clone(),
        instance_rules: provider.instance_rules.clone(),
        timeout_ms: provider.timeout_ms,
        auto_sync_models: provider.auto_sync_models,
    }
}

fn provider_from_add(
    request: ProviderAddRequest,
    resolved_adapter: Option<String>,
) -> Result<ProviderSettings, RPCErrors> {
    let protocol_adapter_id = request
        .protocol_adapter_id
        .or(resolved_adapter)
        .ok_or_else(|| RPCErrors::ReasonError("provider adapter was not resolved".to_string()))?;
    Ok(ProviderSettings {
        provider_instance_name: request.provider_instance_name,
        provider_type: request.provider_type,
        provider_profile_id: request.provider_profile_id,
        protocol_adapter_id,
        base_url: request.base_url,
        credentials: request.credentials,
        enabled: true,
        region: request.region,
        workspace: request.workspace,
        account: request.account,
        provider_rules_id: request.provider_rules_id,
        auth: request.auth,
        discovery: request.discovery,
        instance_rules: request.instance_rules,
        timeout_ms: request.timeout_ms,
        auto_sync_models: request.auto_sync_models,
    })
}

fn apply_provider_update(provider: &mut ProviderSettings, request: ProviderUpdateRequest) {
    if let Some(enabled) = request.enabled {
        provider.enabled = enabled;
    }
    if let Some(base_url) = request.base_url {
        provider.base_url = base_url;
    }
    if let Some(credential) = request.credential {
        provider.credentials = credential;
    }
    if let Some(profile) = request.provider_profile_id {
        provider.provider_profile_id = profile;
    }
    if let Some(adapter) = request.protocol_adapter_id {
        provider.protocol_adapter_id = adapter;
    }
    if let Some(discovery) = request.discovery {
        provider.discovery = Some(discovery);
    }
    if let Some(rules) = request.instance_rules {
        provider.instance_rules = Some(rules);
    }
}

fn reload_result(snapshot: &RuntimeAdminSnapshot) -> ProviderReloadResult {
    ProviderReloadResult {
        ok: true,
        providers_registered: snapshot.providers.len() as u64,
    }
}

fn value_supports_method(provider: &Value, method: &str) -> bool {
    provider
        .get("methods")
        .and_then(Value::as_array)
        .is_none_or(|methods| methods.iter().any(|candidate| candidate == method))
}

fn conflict_error(expected: u64, actual: u64) -> RPCErrors {
    RPCErrors::ReasonError(format!(
        "settings revision conflict: expected {expected}, actual {actual}"
    ))
}

fn to_rpc_error(error: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(error.to_string())
}

pub(crate) struct SystemConfigSettingsStore {
    service_url: String,
}

pub(crate) struct RuntimeAuthorizer;

#[async_trait]
impl ServiceAuthorizer for RuntimeAuthorizer {
    async fn authorize(
        &self,
        context: &RPCContext,
        action: &'static str,
        resource: &'static str,
    ) -> Result<AuthorizedCaller, RPCErrors> {
        let token = context
            .token
            .clone()
            .ok_or_else(|| RPCErrors::InvalidToken("session token is required".to_string()))?;
        let mut request = RPCRequest::new("aicc.authorize", Value::Null);
        request.token = Some(token.clone());
        let (user_id, target) = get_buckyos_api_runtime()?
            .enforce(&request, action, resource)
            .await?;
        Ok(AuthorizedCaller {
            tenant_id: user_id.clone(),
            user_id,
            app_id: Some(target),
            token,
        })
    }
}

pub(crate) struct RoutingQuotaQueryPort {
    factory: Arc<QuotaSourceFactory>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct QuotaTruthRecord {
    period_start_ms: i64,
    period_end_ms: i64,
    max_request_units: Option<u64>,
    max_cost: Option<buckyos_api::Money>,
    reset_at: String,
}

pub(crate) struct SystemConfigQuotaTruthPort {
    client: SystemConfigClient,
    storage: Arc<AiccStorage>,
    runtime: Arc<RuntimeState>,
}

impl SystemConfigQuotaTruthPort {
    pub(crate) fn new(
        service_url: &str,
        service_token: &str,
        storage: Arc<AiccStorage>,
        runtime: Arc<RuntimeState>,
    ) -> Self {
        Self {
            client: SystemConfigClient::new(Some(service_url), Some(service_token)),
            storage,
            runtime,
        }
    }
}

#[async_trait]
impl QuotaTruthPort for SystemConfigQuotaTruthPort {
    async fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
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
        let value = self.client.get(&key).await.map_err(|_| QuotaSourceError)?;
        let record: QuotaTruthRecord =
            serde_json::from_str(&value.value).map_err(|_| QuotaSourceError)?;
        validate_quota_record(&record)?;
        let now_ms = current_time_ms()?;
        if now_ms < record.period_start_ms || now_ms >= record.period_end_ms {
            return Err(QuotaSourceError);
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
        let usage = self
            .storage
            .query_usage(&usage_request, now_ms)
            .await
            .map_err(|_| QuotaSourceError)?;
        let provider = match lookup.provider_instance_name.as_deref() {
            Some(name) => Some(
                self.runtime
                    .capture()
                    .await
                    .providers
                    .quota_observation(name)
                    .await
                    .map_err(|_| QuotaSourceError)?,
            ),
            None => None,
        };
        combine_quota(record, &usage.total, provider.as_ref())
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

fn combine_quota(
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
            if let Some(currency) = usage.finance_currency.as_deref() {
                if currency != limit.currency {
                    return Err(QuotaSourceError);
                }
            }
            Some(buckyos_api::Money::new(
                (limit.amount - usage.finance_amount).max(0.0),
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
            ProviderQuotaObservationState::QueryFailed => return Err(QuotaSourceError),
        };
        remaining_units = minimum_option(remaining_units, provider.remaining_request_units);
        if let Some(provider_cost) = &provider.remaining_cost_usd {
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
        _ => QuotaState::Normal,
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

impl SystemConfigSettingsStore {
    pub(crate) fn new(service_url: impl Into<String>) -> Self {
        Self {
            service_url: service_url.into(),
        }
    }

    fn client(&self, token: &str) -> SystemConfigClient {
        SystemConfigClient::new(Some(&self.service_url), Some(token))
    }
}

#[async_trait]
impl SettingsStore for SystemConfigSettingsStore {
    async fn load(&self, token: &str) -> Result<StoredSettings, RPCErrors> {
        let value = self
            .client(token)
            .get(crate::settings::AICC_SETTINGS_KEY)
            .await
            .map_err(to_rpc_error)?;
        Ok(StoredSettings {
            document: SettingsDocument::parse(value.version, &value.value).map_err(to_rpc_error)?,
        })
    }

    async fn compare_and_swap(
        &self,
        token: &str,
        expected_revision: u64,
        settings: &AiccSettings,
    ) -> Result<u64, RPCErrors> {
        let client = self.client(token);
        let serialized = serde_json::to_string(settings).map_err(to_rpc_error)?;
        let mut actions = HashMap::new();
        actions.insert(
            crate::settings::AICC_SETTINGS_KEY.to_string(),
            KVAction::Update(serialized.clone()),
        );
        client
            .exec_tx(
                actions,
                Some((
                    crate::settings::AICC_SETTINGS_KEY.to_string(),
                    expected_revision,
                )),
            )
            .await
            .map_err(|error| {
                conflict_error(expected_revision, expected_revision.saturating_add(1))
                    .or_reason(error)
            })?;
        let persisted = client
            .get(crate::settings::AICC_SETTINGS_KEY)
            .await
            .map_err(to_rpc_error)?;
        if persisted.value != serialized {
            return Err(RPCErrors::ReasonError(
                "settings changed after compare-and-swap".to_string(),
            ));
        }
        Ok(persisted.version)
    }
}

trait RpcErrorContext {
    fn or_reason(self, source: impl std::fmt::Display) -> RPCErrors;
}

impl RpcErrorContext for RPCErrors {
    fn or_reason(self, source: impl std::fmt::Display) -> RPCErrors {
        RPCErrors::ReasonError(format!("{self}: {source}"))
    }
}

pub(crate) struct StorageQueryPort {
    storage: Arc<AiccStorage>,
}

impl StorageQueryPort {
    pub(crate) fn new(storage: Arc<AiccStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl UsageQueryPort for StorageQueryPort {
    async fn query_usage(
        &self,
        request: QueryUsageRequest,
    ) -> Result<QueryUsageResponse, RPCErrors> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(to_rpc_error)?
            .as_millis() as i64;
        self.storage
            .query_usage(&request, now_ms)
            .await
            .map_err(to_rpc_error)
    }

    async fn query_trace(
        &self,
        tenant_id: &str,
        request: QueryRouteTraceRequest,
    ) -> Result<QueryRouteTraceResponse, RPCErrors> {
        self.storage
            .query_route_traces(tenant_id, &request)
            .await
            .map_err(to_rpc_error)
    }
}

pub(crate) struct CloudUpdateDriverMetadataPort {
    service_url: String,
    manager: Arc<CloudUpdateManager>,
    runtime: Arc<RuntimeState>,
}

impl CloudUpdateDriverMetadataPort {
    pub(crate) fn new(
        service_url: impl Into<String>,
        manager: Arc<CloudUpdateManager>,
        runtime: Arc<RuntimeState>,
    ) -> Self {
        Self {
            service_url: service_url.into(),
            manager,
            runtime,
        }
    }

    fn client(&self, token: &str) -> SystemConfigClient {
        SystemConfigClient::new(Some(&self.service_url), Some(token))
    }

    async fn view(&self) -> DriverMetadataUpdateView {
        let config = self.manager.config().await;
        let status = self.manager.status().await;
        let runtime = self.runtime.metadata_status().await;
        let providers = runtime
            .providers
            .iter()
            .map(|(name, state)| {
                buckyos_api::DriverMetadataProviderStatus::new(
                    name.clone(),
                    state.metadata_applied_seq,
                )
            })
            .collect::<Vec<_>>();
        let convergence_degraded = runtime.providers.values().any(|provider| {
            provider.last_error.is_some() || provider.metadata_applied_seq < runtime.target_seq
        });
        DriverMetadataUpdateView {
            enabled: config.enabled,
            source_url: config.source_url.clone(),
            source_configured: config.source_url.is_some(),
            interval_secs: config.interval_secs,
            metadata_target_seq: runtime.target_seq,
            providers,
            status: if !config.enabled {
                DriverMetadataUpdateStatus::Disabled
            } else if status.updating {
                DriverMetadataUpdateStatus::Updating
            } else if status.last_error.is_some() {
                DriverMetadataUpdateStatus::Error
            } else if convergence_degraded {
                DriverMetadataUpdateStatus::Degraded
            } else if status.last_success_at_ms.is_some() {
                DriverMetadataUpdateStatus::Healthy
            } else {
                DriverMetadataUpdateStatus::Idle
            },
            active_revision: status.active_revision,
            last_attempt_at_ms: status.last_attempt_at_ms,
            last_success_at_ms: status.last_success_at_ms,
            last_error: status.last_error,
            consecutive_failures: status.consecutive_failures,
        }
    }
}

#[async_trait]
impl DriverMetadataPort for CloudUpdateDriverMetadataPort {
    async fn get(&self) -> Result<DriverMetadataUpdateView, RPCErrors> {
        Ok(self.view().await)
    }

    async fn set(
        &self,
        token: &str,
        expected_settings_revision: u64,
        request: DriverMetadataUpdateSetReq,
    ) -> Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        let current = self.manager.config().await;
        let next = CloudUpdateConfig {
            enabled: request.enabled,
            source_url: request.source_url.or(current.source_url),
            interval_secs: request.interval_secs.unwrap_or(current.interval_secs),
        };
        next.validate().map_err(to_rpc_error)?;
        let client = self.client(token);
        let settings = client
            .get(crate::settings::AICC_SETTINGS_KEY)
            .await
            .map_err(to_rpc_error)?;
        if settings.version != expected_settings_revision {
            return Err(conflict_error(expected_settings_revision, settings.version));
        }
        let action = match client.get(CLOUD_UPDATE_CONFIG_KEY).await {
            Ok(_) => KVAction::Update(serde_json::to_string(&next).map_err(to_rpc_error)?),
            Err(SystemConfigError::KeyNotFound(_)) => {
                KVAction::Create(serde_json::to_string(&next).map_err(to_rpc_error)?)
            }
            Err(error) => return Err(to_rpc_error(error)),
        };
        let mut actions = HashMap::new();
        actions.insert(CLOUD_UPDATE_CONFIG_KEY.to_string(), action);
        actions.insert(
            crate::settings::AICC_SETTINGS_KEY.to_string(),
            KVAction::Update(settings.value.clone()),
        );
        client
            .exec_tx(
                actions,
                Some((
                    crate::settings::AICC_SETTINGS_KEY.to_string(),
                    expected_settings_revision,
                )),
            )
            .await
            .map_err(to_rpc_error)?;
        let persisted_revision = client
            .get(crate::settings::AICC_SETTINGS_KEY)
            .await
            .map_err(to_rpc_error)?
            .version;
        if persisted_revision != expected_settings_revision.saturating_add(1) {
            return Err(RPCErrors::ReasonError(
                "metadata update CAS returned an unexpected settings revision".to_string(),
            ));
        }
        self.manager.set_config(next).await.map_err(to_rpc_error)?;
        Ok(DriverMetadataUpdateSetResponse {
            ok: true,
            settings_revision: persisted_revision,
            settings: self.view().await,
            runtime_apply: DriverMetadataRuntimeApply {
                ok: true,
                refresh_scheduled: Some(request.enabled),
                error: None,
            },
        })
    }
}

pub(crate) struct RuntimeServiceAdapter {
    runtime: Arc<RuntimeState>,
    codecs: Arc<CodecRegistry>,
}

impl RuntimeServiceAdapter {
    pub(crate) fn new(runtime: Arc<RuntimeState>, codecs: Arc<CodecRegistry>) -> Self {
        Self { runtime, codecs }
    }

    async fn snapshot(&self) -> RuntimeAdminSnapshot {
        runtime_admin_snapshot(self.runtime.capture().await.as_ref(), self.codecs.as_ref())
    }
}

struct PreparedRuntimeAdapter {
    prepared: PreparedRuntimeMutation,
    codecs: Arc<CodecRegistry>,
}

#[async_trait]
impl PreparedSettingsRuntime for PreparedRuntimeAdapter {
    fn expected_revision(&self) -> u64 {
        self.prepared.expected_settings_revision()
    }

    fn settings_revision(&self) -> u64 {
        self.prepared.settings_revision()
    }

    async fn publish(self: Box<Self>) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        let Self { prepared, codecs } = *self;
        let snapshot = prepared.publish().await;
        Ok(runtime_admin_snapshot(snapshot.as_ref(), codecs.as_ref()))
    }

    async fn discard(self: Box<Self>) {
        self.prepared.discard().await;
    }
}

#[async_trait]
impl ServiceRuntime for RuntimeServiceAdapter {
    async fn capture(&self) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        Ok(self.snapshot().await)
    }

    async fn prepare_settings(
        &self,
        settings: SettingsDocument,
    ) -> Result<Box<dyn PreparedSettingsRuntime>, RPCErrors> {
        let prepared = self
            .runtime
            .prepare_reload(settings)
            .await
            .map_err(to_rpc_error)?;
        Ok(Box::new(PreparedRuntimeAdapter {
            prepared,
            codecs: self.codecs.clone(),
        }))
    }

    async fn refresh_provider(
        &self,
        provider_instance_name: &str,
    ) -> Result<RuntimeAdminSnapshot, RPCErrors> {
        let snapshot = self
            .runtime
            .refresh_provider(provider_instance_name)
            .await
            .map_err(to_rpc_error)?;
        Ok(runtime_admin_snapshot(
            snapshot.as_ref(),
            self.codecs.as_ref(),
        ))
    }
}

pub(crate) struct RuntimeProviderExecutionPort {
    runtime: Arc<RuntimeState>,
    codecs: Arc<CodecRegistry>,
}

impl RuntimeProviderExecutionPort {
    pub(crate) fn new(runtime: Arc<RuntimeState>, codecs: Arc<CodecRegistry>) -> Self {
        Self { runtime, codecs }
    }

    fn transport(limits: &CodecLimits) -> Result<HttpTransport, ProtocolError> {
        HttpTransport::new(HttpTransportConfig {
            request_timeout: limits.request_timeout,
            max_request_bytes: limits.max_request_bytes,
            max_response_bytes: limits.max_response_bytes,
            max_json_bytes: limits.max_response_bytes,
            ..HttpTransportConfig::default()
        })
    }

    async fn send_cancelable<T>(
        cancellation: &crate::protocol::Cancellation,
        future: impl std::future::Future<Output = Result<T, ProtocolError>>,
    ) -> Result<T, ProtocolError> {
        tokio::select! {
            result = future => result,
            _ = cancellation.cancelled() => Err(ProtocolError::new(
                ProtocolErrorKind::Cancelled,
                "Provider request was cancelled",
            )),
        }
    }

    async fn resume_context(
        &self,
        binding: &PinnedProviderTask,
    ) -> Result<(CodecContext, BTreeMap<String, Value>), NativeTaskResumeError> {
        let resume = binding
            .resume
            .as_ref()
            .ok_or(NativeTaskResumeError::CredentialUnavailable)?;
        let snapshot = self.runtime.capture().await;
        let provider = snapshot
            .providers
            .get(&binding.provider_instance_name)
            .ok_or(NativeTaskResumeError::CredentialUnavailable)?;
        let credential = match &resume.credential {
            Some(expected) => {
                let reference = &provider.config.credential.reference;
                if reference != &expected.reference
                    || credential_fingerprint(reference) != expected.fingerprint
                {
                    return Err(NativeTaskResumeError::CredentialUnavailable);
                }
                let resolved = provider
                    .resolve_credential()
                    .await
                    .map_err(|_| NativeTaskResumeError::CredentialUnavailable)?;
                if resume_credential_kind(resolved.audit().kind) != expected.kind {
                    return Err(NativeTaskResumeError::CredentialUnavailable);
                }
                Some(resolved)
            }
            None => None,
        };
        let request_timeout = Duration::from_millis(resume.request_timeout_ms);
        let max_request_bytes = usize::try_from(resume.max_request_bytes)
            .map_err(|_| NativeTaskResumeError::CredentialUnavailable)?;
        let max_response_bytes = usize::try_from(resume.max_response_bytes)
            .map_err(|_| NativeTaskResumeError::CredentialUnavailable)?;
        Ok((
            CodecContext {
                base_url: resume.base_url.clone(),
                credential,
                resources: BTreeMap::new(),
                limits: CodecLimits {
                    request_timeout,
                    max_request_bytes,
                    max_response_bytes,
                },
            },
            resume.resolved_parameters.clone(),
        ))
    }

    async fn native_request(
        &self,
        binding: &PinnedProviderTask,
        operation: NativeTaskOperation,
        cancellation: Option<&crate::protocol::Cancellation>,
    ) -> Result<NativeTaskOutput, NativeTaskResumeError> {
        let remote_task_id = binding.remote_task_id.as_deref().ok_or_else(|| {
            NativeTaskResumeError::Protocol(ProtocolError::invalid_request(
                "native task binding has no remote task ID",
            ))
        })?;
        let (context, parameters) = self.resume_context(binding).await?;
        let request = self
            .codecs
            .encode_native(
                &binding.protocol_adapter_id,
                &binding.operation,
                binding.api_type,
                &NativeTaskInput {
                    operation,
                    remote_task_id: Some(remote_task_id),
                    codec_input: None,
                    resolved_parameters: &parameters,
                    context: &context,
                },
            )
            .map_err(NativeTaskResumeError::Protocol)?;
        let transport =
            Self::transport(&context.limits).map_err(NativeTaskResumeError::Protocol)?;
        let response = match cancellation {
            Some(cancellation) => {
                Self::send_cancelable(cancellation, transport.send(request)).await
            }
            None => transport.send(request).await,
        }
        .map_err(NativeTaskResumeError::Protocol)?;
        self.codecs
            .decode_native(
                &binding.protocol_adapter_id,
                &binding.operation,
                binding.api_type,
                operation,
                response,
            )
            .await
            .map_err(NativeTaskResumeError::Protocol)
    }
}

#[async_trait]
impl ProviderExecutionPort for RuntimeProviderExecutionPort {
    async fn start(
        &self,
        runtime_generation: u64,
        call: &crate::call::ResolvedProviderCall,
        cancellation: crate::protocol::Cancellation,
    ) -> Result<ProviderExecution, ProviderStartFailure> {
        let snapshot = self.runtime.capture().await;
        if snapshot.generation != runtime_generation {
            return Err(ProviderStartFailure::before_accept(
                ProtocolError::invalid_configuration("captured runtime generation was retired"),
                true,
            ));
        }
        let provider = snapshot
            .providers
            .get(&call.provider_instance_name)
            .ok_or_else(|| {
                ProviderStartFailure::before_accept(
                    ProtocolError::invalid_configuration("selected Provider is not published"),
                    false,
                )
            })?;
        let descriptor = self
            .codecs
            .operation_descriptor(&call.protocol_adapter_id, &call.operation, call.api_type)
            .and_then(|operation| {
                operation
                    .binding(call.api_type)
                    .map(|binding| (operation.supports_cancel, binding.execution_modes.clone()))
            })
            .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
        let transport = Self::transport(&call.context.limits)
            .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
        if descriptor.1.contains(&ExecutionMode::Immediate) {
            let request = self
                .codecs
                .encode(
                    &call.protocol_adapter_id,
                    &call.operation,
                    call.api_type,
                    &call.input,
                    &call.context,
                )
                .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
            let response = Self::send_cancelable(&cancellation, transport.send(request))
                .await
                .map_err(ProviderStartFailure::after_accept)?;
            let decoded = self
                .codecs
                .decode(
                    &call.protocol_adapter_id,
                    &call.operation,
                    call.api_type,
                    response,
                )
                .await
                .map_err(ProviderStartFailure::after_accept)?;
            return match decoded {
                crate::protocol::ProtocolExecution::Immediate(output) => {
                    Ok(ProviderExecution::Immediate(output))
                }
                _ => Err(ProviderStartFailure::after_accept(
                    ProtocolError::invalid_response(
                        "buffered Provider response returned an unexpected execution mode",
                    ),
                )),
            };
        }
        if descriptor.1.contains(&ExecutionMode::Stream) {
            let request = self
                .codecs
                .encode(
                    &call.protocol_adapter_id,
                    &call.operation,
                    call.api_type,
                    &call.input,
                    &call.context,
                )
                .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
            let response = Self::send_cancelable(&cancellation, transport.send_streaming(request))
                .await
                .map_err(ProviderStartFailure::after_accept)?;
            return self
                .codecs
                .decode_stream(
                    &call.protocol_adapter_id,
                    &call.operation,
                    call.api_type,
                    response,
                )
                .await
                .map(ProviderExecution::Stream)
                .map_err(ProviderStartFailure::after_accept);
        }
        let request = self
            .codecs
            .encode_native(
                &call.protocol_adapter_id,
                &call.operation,
                call.api_type,
                &NativeTaskInput {
                    operation: NativeTaskOperation::Submit,
                    remote_task_id: None,
                    codec_input: Some(&call.input),
                    resolved_parameters: &call.input.resolved_parameters,
                    context: &call.context,
                },
            )
            .map_err(|error| ProviderStartFailure::before_accept(error, false))?;
        let response = Self::send_cancelable(&cancellation, transport.send(request))
            .await
            .map_err(ProviderStartFailure::after_accept)?;
        let output = self
            .codecs
            .decode_native(
                &call.protocol_adapter_id,
                &call.operation,
                call.api_type,
                NativeTaskOperation::Submit,
                response,
            )
            .await
            .map_err(ProviderStartFailure::after_accept)?;
        let NativeTaskOutput::Submitted(handle) = output else {
            return Err(ProviderStartFailure::after_accept(
                ProtocolError::invalid_response("native submit returned a non-submit result"),
            ));
        };
        let credential = call
            .context
            .credential
            .as_ref()
            .map(|credential| ResumeCredential {
                reference: provider.config.credential.reference.clone(),
                kind: resume_credential_kind(credential.audit().kind),
                header_name: provider.profile.credential.header_name.clone(),
                fingerprint: credential_fingerprint(&provider.config.credential.reference),
            });
        Ok(ProviderExecution::NativeTask {
            handle,
            resume: NativeTaskResumeDescriptor {
                base_url: call.context.base_url.clone(),
                credential,
                resolved_parameters: call.input.resolved_parameters.clone(),
                request_timeout_ms: call.context.limits.request_timeout.as_millis() as u64,
                max_request_bytes: call.context.limits.max_request_bytes as u64,
                max_response_bytes: call.context.limits.max_response_bytes as u64,
            },
        })
    }

    async fn poll_native(
        &self,
        binding: &PinnedProviderTask,
        cancellation: crate::protocol::Cancellation,
    ) -> Result<NativeTaskPoll, NativeTaskResumeError> {
        match self
            .native_request(binding, NativeTaskOperation::Status, Some(&cancellation))
            .await?
        {
            NativeTaskOutput::Status { state, .. }
                if state == crate::protocol::NativeTaskState::Succeeded => {}
            NativeTaskOutput::Status {
                state:
                    state @ (crate::protocol::NativeTaskState::Submitted
                    | crate::protocol::NativeTaskState::Queued
                    | crate::protocol::NativeTaskState::Running),
                ..
            } => return Ok(NativeTaskPoll::Pending(state, None)),
            NativeTaskOutput::Status {
                state: crate::protocol::NativeTaskState::Cancelled,
                ..
            } => {
                return Ok(NativeTaskPoll::Failed(ProtocolError::new(
                    ProtocolErrorKind::Cancelled,
                    "native Provider task was cancelled",
                )))
            }
            NativeTaskOutput::Status {
                state: crate::protocol::NativeTaskState::Failed,
                ..
            } => {
                return Ok(NativeTaskPoll::Failed(ProtocolError::invalid_response(
                    "native Provider task reported failure",
                )))
            }
            _ => {
                return Err(NativeTaskResumeError::Protocol(
                    ProtocolError::invalid_response("native status returned an unexpected result"),
                ))
            }
        }
        match self
            .native_request(binding, NativeTaskOperation::Result, Some(&cancellation))
            .await?
        {
            NativeTaskOutput::Result(output) => Ok(NativeTaskPoll::Complete(output)),
            _ => Err(NativeTaskResumeError::Protocol(
                ProtocolError::invalid_response("native result returned an unexpected response"),
            )),
        }
    }

    async fn cancel_native(
        &self,
        binding: &PinnedProviderTask,
    ) -> Result<bool, NativeTaskResumeError> {
        match self
            .native_request(binding, NativeTaskOperation::Cancel, None)
            .await?
        {
            NativeTaskOutput::Cancelled { accepted } => Ok(accepted),
            _ => Err(NativeTaskResumeError::Protocol(
                ProtocolError::invalid_response(
                    "native cancellation returned an unexpected response",
                ),
            )),
        }
    }
}

fn resume_credential_kind(kind: CredentialKind) -> ResumeCredentialKind {
    match kind {
        CredentialKind::Bearer => ResumeCredentialKind::Bearer,
        CredentialKind::NamedHeader => ResumeCredentialKind::NamedHeader,
        CredentialKind::FalKey => ResumeCredentialKind::FalKey,
        CredentialKind::GlmJwt => ResumeCredentialKind::GlmJwt,
    }
}

fn credential_fingerprint(reference: &str) -> String {
    Sha256::digest(reference.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) struct TaskManagerExecutionPort {
    client: TaskManagerClient,
}

impl TaskManagerExecutionPort {
    pub(crate) fn new(client: TaskManagerClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TaskManagerPort for TaskManagerExecutionPort {
    async fn ensure_task(&self, spec: TaskSpec) -> Result<TaskBinding, buckyos_api::AiccError> {
        let task = self
            .client
            .create_task(CreateTaskReq {
                name: format!("AICC {}", spec.method),
                schema_id: AICC_COMPUTE_TASK_SCHEMA_ID.to_string(),
                schema_version: None,
                input: json!({
                    "request": {
                        "version": 1,
                        "tenant_id": spec.tenant_id,
                        "request": spec.input,
                    }
                }),
                executor: CreateTaskExecutor::SelfApp {
                    app_instance_id: None,
                },
                parent_id: spec.parent_id,
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                storage_domain: None,
                idempotency_key: spec.idempotency_key,
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await
            .map_err(task_manager_error)?;
        Ok(TaskBinding {
            event_ref: buckyos_api::task_mgr_task_event_path(&task.task_id),
            task_id: task.task_id,
        })
    }

    async fn report_state(
        &self,
        task_id: &str,
        state: ExecutionState,
        data: Value,
    ) -> Result<(), buckyos_api::AiccError> {
        if matches!(state, ExecutionState::Running) {
            self.client
                .runner_start(task_id)
                .await
                .map_err(task_manager_error)?;
        }
        self.client
            .runner_progress(task_id, Some(data), None)
            .await
            .map(|_| ())
            .map_err(task_manager_error)
    }

    async fn commit_result(
        &self,
        task_id: &str,
        output: &ExecutionOutput,
    ) -> Result<(), buckyos_api::AiccError> {
        self.client
            .runner_complete(
                task_id,
                serde_json::to_value(output).map_err(|_| {
                    buckyos_api::AiccError::new(
                        buckyos_api::AiccErrorCode::InternalError,
                        "execution result could not be serialized",
                    )
                })?,
            )
            .await
            .map(|_| ())
            .map_err(task_manager_error)
    }

    async fn fail_task(
        &self,
        task_id: &str,
        error: &buckyos_api::AiccError,
    ) -> Result<(), buckyos_api::AiccError> {
        self.client
            .runner_fail(
                task_id,
                error.code.as_str(),
                error.message.clone(),
                Some(error.to_task_event_data()),
            )
            .await
            .map(|_| ())
            .map_err(task_manager_error)
    }

    async fn cancel_task(&self, task_id: &str) -> Result<(), buckyos_api::AiccError> {
        self.client
            .cancel_task(task_id, false)
            .await
            .map(|_| ())
            .map_err(task_manager_error)
    }
}

fn task_manager_error(error: RPCErrors) -> buckyos_api::AiccError {
    buckyos_api::AiccError::new(
        buckyos_api::AiccErrorCode::InternalError,
        format!("TaskMgr operation failed: {error}"),
    )
}

struct ServiceModelAssembler {
    session: Option<buckyos_api::AiccRouteOverlay>,
}

#[async_trait]
impl ModelRegistryAssembler for ServiceModelAssembler {
    async fn build(
        &self,
        catalog: Arc<CatalogSnapshot>,
        inventories: Vec<ModelProviderInventory>,
    ) -> Result<Arc<ModelRegistry>, crate::runtime::RuntimeError> {
        ModelRegistry::build(
            catalog.as_ref(),
            &inventories,
            Vec::new(),
            RegistryLayers {
                session: self.session.as_ref(),
                ..RegistryLayers::default()
            },
        )
        .map(Arc::new)
        .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))
    }
}

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
    ) -> Result<PreparedRuntime, crate::runtime::RuntimeError> {
        let builtins = builtin_provider_registry(catalog.as_ref())
            .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))?;
        let (resolver, auth) = settings_credentials(settings.as_ref())
            .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))?;
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
            .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))?,
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
            let provider_auth = auth.get(&provider.provider_instance_name).ok_or_else(|| {
                crate::runtime::RuntimeError::Backend(
                    "provider authentication was not prepared".to_string(),
                )
            })?;
            let configured_inventory = provider
                .discovery
                .clone()
                .map(serde_json::from_value::<ProviderDiscoverySnapshot>)
                .transpose()
                .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))?;
            let binding = builtins
                .resolve(BuiltinProviderRequest {
                    provider_profile_id: &provider.provider_profile_id,
                    protocol_adapter_id: &provider.protocol_adapter_id,
                    auth_mode: provider_auth.mode(),
                    configured_inventory,
                })
                .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))?;
            let connection = binding
                .connection
                .resolve(ProviderConnectionInput {
                    base_url: Some(&provider.base_url),
                    region: provider.region.as_deref(),
                    workspace: provider.workspace.as_deref(),
                    account: provider.account.as_deref(),
                })
                .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))?;
            let provider_rules_id = provider.provider_rules_id.clone().or_else(|| {
                catalog
                    .resolve_provider_configuration(&provider.provider_profile_id)
                    .ok()
                    .map(|configuration| configuration.provider_rules_id)
            });
            let runtime_config = match provider_auth {
                ProviderAuthConfig::ApiKey { credential_ref } => ProviderInstanceConfig {
                    provider_instance_name: provider.provider_instance_name.clone(),
                    provider_profile_id: provider.provider_profile_id.clone(),
                    protocol_adapter_id: provider.protocol_adapter_id.clone(),
                    base_url: connection.base_url,
                    credential: CredentialReference {
                        reference: credential_ref.clone(),
                    },
                    provider_rules_id,
                    region: connection.region,
                    workspace: connection.workspace,
                    account: connection.account,
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
                    .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))?;
                    credential_broker
                        .register_dynamic_instance(resolved.clone())
                        .await
                        .map_err(|error| {
                            crate::runtime::RuntimeError::Backend(error.to_string())
                        })?;
                    resolved.runtime
                }
            };
            manager
                .start(runtime_config, binding.discovery)
                .await
                .map_err(|error| crate::runtime::RuntimeError::Backend(error.to_string()))?;
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
        let parsed = provider
            .auth
            .clone()
            .map(serde_json::from_value::<ProviderAuthConfig>)
            .transpose()
            .map_err(to_rpc_error)?;
        let parsed = match parsed {
            Some(value) => {
                if let ProviderAuthConfig::ApiKey { credential_ref } = &value {
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
                }
            }
        };
        auth.insert(provider.provider_instance_name.clone(), parsed);
    }
    Ok((StaticCredentialResolver::new(values), auth))
}

fn first_locked_credential(
    instance: &str,
    credentials: &Value,
) -> Result<(String, String), RPCErrors> {
    let object = credentials
        .as_object()
        .ok_or_else(|| RPCErrors::ReasonError("credentials must be an object".to_string()))?;
    for (name, value) in object {
        if let Some(secret) = value.get("locked").and_then(Value::as_str) {
            return Ok((format!("locked://{instance}/{name}"), secret.to_string()));
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
        let adapter = request
            .protocol_adapter_id
            .clone()
            .or_else(|| {
                builtins
                    .profiles()
                    .find(|profile| profile.provider_profile_id == request.provider_profile_id)
                    .map(|profile| profile.default_protocol_adapter_id.clone())
            })
            .ok_or_else(|| {
                RPCErrors::ReasonError("provider adapter was not resolved".to_string())
            })?;
        let auth = request
            .auth
            .clone()
            .map(serde_json::from_value::<ProviderAuthConfig>)
            .transpose()
            .map_err(to_rpc_error)?;
        let (auth, credentials) = match auth {
            Some(auth @ ProviderAuthConfig::DynamicLogin { .. }) => (auth, BTreeMap::new()),
            Some(auth @ ProviderAuthConfig::ApiKey { .. }) => {
                let credential_ref = match &auth {
                    ProviderAuthConfig::ApiKey { credential_ref } => credential_ref.clone(),
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
                    },
                    BTreeMap::from([(reference, secret)]),
                )
            }
        };
        let configured_inventory = request
            .discovery
            .clone()
            .map(serde_json::from_value::<ProviderDiscoverySnapshot>)
            .transpose()
            .map_err(to_rpc_error)?;
        let binding = builtins
            .resolve(BuiltinProviderRequest {
                provider_profile_id: &request.provider_profile_id,
                protocol_adapter_id: &adapter,
                auth_mode: auth.mode(),
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

fn runtime_admin_snapshot(
    snapshot: &crate::runtime::RuntimeSnapshot,
    codecs: &CodecRegistry,
) -> RuntimeAdminSnapshot {
    let providers = snapshot
        .providers
        .list()
        .into_iter()
        .map(|provider| {
            json!({
                "provider_instance_name": provider.config.provider_instance_name,
                "provider_profile_id": provider.config.provider_profile_id,
                "protocol_adapter_id": provider.config.protocol_adapter_id,
                "base_url": provider.config.base_url,
                "provider_rules_id": provider.config.provider_rules_id,
                "region": provider.config.region,
                "workspace": provider.config.workspace,
                "account": provider.config.account,
                "health": provider.inventory.health,
                "model_count": provider.inventory.models.len(),
                "methods": provider.inventory.models.iter()
                    .flat_map(|model| model.api_types.iter().map(|api_type| api_type.typed_method()))
                    .collect::<std::collections::BTreeSet<_>>(),
                "inventory_revision": provider.inventory.inventory_revision,
                "metadata_applied_seq": provider.inventory.metadata_applied_seq,
            })
        })
        .collect::<Vec<_>>();
    let inventory_revision = format!("{}:{}", snapshot.generation, snapshot.metadata_target_seq);
    let provider_health = snapshot
        .models
        .model_views()
        .into_iter()
        .filter_map(|model| {
            let provider = snapshot.providers.get(&model.provider_instance_name)?;
            Some((
                model.exact_model.clone(),
                json!({
                    "state": provider.inventory.health,
                    "provider_instance_name": model.provider_instance_name,
                    "inventory_revision": provider.inventory.inventory_revision,
                    "metadata_applied_seq": provider.inventory.metadata_applied_seq,
                }),
            ))
        })
        .collect();
    RuntimeAdminSnapshot {
        settings_revision: snapshot.settings_revision,
        catalog_revision: snapshot.catalog.target_revision_seq(),
        provider_catalog: ProviderCatalogResponse {
            catalog_revision: snapshot.catalog.target_revision_seq(),
            providers: snapshot
                .catalog
                .known_providers()
                .map(|provider| buckyos_api::ProviderCatalogEntry {
                    provider_profile_id: provider.provider_profile_id.clone(),
                    display_name: provider.display_name.clone(),
                    base_url: provider.base_url.clone(),
                    protocol_adapter_id: provider.protocol_adapter_id.clone(),
                    provider_rules_id: provider.provider_rules_id.clone(),
                    ui_hints: provider.ui_hints.clone(),
                })
                .collect(),
        },
        protocol_adapters: protocol_adapter_response(codecs),
        models: json!({
            "models": snapshot.models.model_views().into_iter().map(|model| json!({
                "exact_model": model.exact_model,
                "model_uid": model.model_uid,
                "provider_instance_name": model.provider_instance_name,
                "provider_profile_id": model.provider_profile_id,
                "protocol_adapter_id": model.protocol_adapter_id,
                "model_driver_id": model.model_driver_id,
                "origin_model_id": model.origin_model_id,
                "provider_model_id": model.provider_model_id,
                "variant": model.variant,
                "api_types": model.api_types,
                "logical_mounts": model.logical_mounts,
                "capabilities": model.capabilities,
                "attributes": model.attributes,
                "operations": model.operations,
                "inventory_revision": model.inventory_revision,
            })).collect::<Vec<_>>(),
            "generation": snapshot.generation,
        }),
        providers,
        inventory_revision,
        provider_health,
    }
}

fn protocol_adapter_response(codecs: &CodecRegistry) -> ProtocolAdapterListResponse {
    ProtocolAdapterListResponse {
        adapters: codecs
            .adapters()
            .enumerate()
            .map(|(priority, adapter)| buckyos_api::ProtocolAdapterView {
                protocol_family_id: adapter.protocol_family_id.clone(),
                protocol_adapter_id: adapter.protocol_adapter_id.clone(),
                interface_generation: adapter.interface_generation.clone(),
                status: match adapter.status {
                    AdapterStatus::Stable => buckyos_api::ProtocolAdapterStatus::Stable,
                    AdapterStatus::Preview => buckyos_api::ProtocolAdapterStatus::Preview,
                    AdapterStatus::Deprecated => buckyos_api::ProtocolAdapterStatus::Deprecated,
                },
                probe_priority: priority as u32,
                base_adapter_id: adapter.base_adapter_id.clone(),
                operations: adapter
                    .operations
                    .values()
                    .map(|operation| buckyos_api::ProtocolAdapterOperation {
                        operation_id: operation.operation_id.clone(),
                        api_types: operation
                            .bindings
                            .iter()
                            .map(|binding| binding.api_type)
                            .collect(),
                        capabilities: unique_values(
                            operation
                                .bindings
                                .iter()
                                .map(|binding| binding.capability.clone()),
                        ),
                        supported_features: operation
                            .bindings
                            .iter()
                            .flat_map(|binding| binding.supported_features.iter().cloned())
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                        execution_modes: unique_values(
                            operation
                                .bindings
                                .iter()
                                .flat_map(|binding| binding.execution_modes.iter())
                                .map(|mode| match mode {
                                    ExecutionMode::Immediate => {
                                        buckyos_api::ProtocolExecutionMode::Immediate
                                    }
                                    ExecutionMode::Stream => {
                                        buckyos_api::ProtocolExecutionMode::Stream
                                    }
                                    ExecutionMode::NativeTask => {
                                        buckyos_api::ProtocolExecutionMode::NativeTask
                                    }
                                }),
                        ),
                        supports_cancel: operation.supports_cancel,
                        supports_webhook: operation.supports_webhook,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn unique_values<T: PartialEq>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

pub(crate) fn disabled_metadata_view(settings_revision: u64) -> DriverMetadataUpdateSetResponse {
    DriverMetadataUpdateSetResponse {
        ok: true,
        settings_revision,
        settings: DriverMetadataUpdateView {
            enabled: false,
            source_url: None,
            source_configured: false,
            interval_secs: 0,
            metadata_target_seq: 0,
            providers: Vec::new(),
            status: DriverMetadataUpdateStatus::Disabled,
            active_revision: None,
            last_attempt_at_ms: None,
            last_success_at_ms: None,
            last_error: None,
            consecutive_failures: 0,
        },
        runtime_apply: DriverMetadataRuntimeApply {
            ok: true,
            refresh_scheduled: Some(false),
            error: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{QuotaState, QuotaView, UsageQueryTimeRange};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FakeAuthorizer {
        deny: bool,
        tenant: &'static str,
    }

    #[async_trait]
    impl ServiceAuthorizer for FakeAuthorizer {
        async fn authorize(
            &self,
            _context: &RPCContext,
            _action: &'static str,
            _resource: &'static str,
        ) -> Result<AuthorizedCaller, RPCErrors> {
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

    struct Fixture {
        service: AiccService,
        settings: Arc<FakeSettingsStore>,
        runtime: Arc<FakeRuntime>,
        validator: Arc<FakeValidator>,
        usage: Arc<FakeUsage>,
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
        let service = AiccService::new(
            Arc::new(FakeAuthorizer {
                deny,
                tenant: "tenant-a",
            }),
            settings.clone(),
            Arc::new(SharedFakeRuntime(runtime.clone())),
            validator.clone(),
            usage.clone(),
            Arc::new(FakeQuota),
            Arc::new(FakeMetadata),
        );
        Fixture {
            service,
            settings,
            runtime,
            validator,
            usage,
        }
    }

    fn provider_add() -> ProviderAddRequest {
        let mut request = ProviderAddRequest::new(
            "primary",
            "cloud_api",
            "openai",
            "https://api.example/v1",
            json!({"api_token": {"locked": "top-secret"}}),
        );
        request.protocol_adapter_id = Some("openai-responses".to_string());
        request
    }

    fn provider_public_view(provider: &ProviderSettings) -> Value {
        json!({
            "provider_instance_name": provider.provider_instance_name,
            "provider_profile_id": provider.provider_profile_id,
            "protocol_adapter_id": provider.protocol_adapter_id,
            "base_url": provider.base_url,
        })
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
            finance_amount: 9.2,
            finance_currency: Some("USD".to_string()),
            ..Default::default()
        };
        let provider = ProviderQuotaObservation {
            state: ProviderQuotaObservationState::Normal,
            remaining_request_units: Some(8),
            remaining_cost_usd: Some(buckyos_api::AiCost {
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
    fn quota_fails_closed_for_incomplete_finance_or_provider_failure() {
        let incomplete = buckyos_api::UsageAggregate {
            finance_complete: false,
            ..Default::default()
        };
        assert!(combine_quota(quota_record(), &incomplete, None).is_err());

        let failed = ProviderQuotaObservation {
            state: ProviderQuotaObservationState::QueryFailed,
            remaining_request_units: None,
            remaining_cost_usd: None,
            reset_at_ms: None,
            observed_at_ms: 1,
            source: "provider-api".to_string(),
        };
        assert!(combine_quota(
            quota_record(),
            &buckyos_api::UsageAggregate::default(),
            Some(&failed)
        )
        .is_err());
    }

    #[test]
    fn exhausted_provider_overrides_normal_budget() {
        let exhausted = ProviderQuotaObservation {
            state: ProviderQuotaObservationState::Exhausted,
            remaining_request_units: Some(0),
            remaining_cost_usd: None,
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
}
