mod cloud_update;
mod inference;
mod management;
mod model_defaults;
mod ports;
mod provider_execution;
mod quota;
mod settings_runtime;

use inference::{inference_error, next_inference_id, now_ms, AuthenticatedResourceAuthorizer};
pub(crate) use inference::{
    DriverMetadataPort, ProviderValidator, QuotaQueryPort, RuntimeInferencePort, UsageQueryPort,
};
use ports::{
    CloudUpdateDriverMetadataPort, RuntimeAuthorizer, RuntimeServiceAdapter, StorageQueryPort,
    SystemConfigSettingsStore, TaskManagerExecutionPort,
};
pub(crate) use provider_execution::RuntimeProviderExecutionPort;
#[cfg(test)]
use quota::{combine_quota, provider_quota, QuotaTruthRecord};
pub(crate) use quota::{RoutingQuotaQueryPort, SystemConfigQuotaTruthPort};
use settings_runtime::provider_auth_config;
pub(crate) use settings_runtime::{RuntimeProviderValidator, ServiceRuntimeFactory};

use anyhow::Context;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, AckControlReq,
    ActorRef, AiMethodStatus, AiccCall, AiccError, AiccErrorCode, AiccHandler, AiccRouteTraceEvent,
    AiccServerHandler, AudioEnhanceRequest, AudioEnhanceResponse, AudioMusicRequest,
    AudioMusicResponse, AudioSpeechRecognitionRequest, AudioSpeechRecognitionResponse,
    AudioTextToSpeechRequest, AudioTextToSpeechResponse, BuckyOSRuntimeType, CancelResponse,
    ComputerUseRequest, ComputerUseResponse, CreateDelegatedTaskReq, DriverMetadataRuntimeApply,
    DriverMetadataUpdateSetReq, DriverMetadataUpdateSetResponse, DriverMetadataUpdateStatus,
    DriverMetadataUpdateView, EmbeddingMultimodalRequest, EmbeddingMultimodalResponse,
    EmbeddingTextRequest, EmbeddingTextResponse, ImageBackgroundRemoveRequest,
    ImageBackgroundRemoveResponse, ImageInpaintRequest, ImageInpaintResponse, ImageToImageRequest,
    ImageToImageResponse, ImageUpscaleRequest, ImageUpscaleResponse, ListModelsRequest,
    LlmChatHelperRequest, LlmChatInvokeRequest, LlmChatInvokeResponse, ProtocolAdapterListRequest,
    ProtocolAdapterListResponse, ProviderAddRequest, ProviderAddResponse, ProviderAuthSettings,
    ProviderCatalogRequest, ProviderCatalogResponse,
    ProviderCredentialKind as ApiProviderCredentialKind, ProviderCredentials,
    ProviderDeleteRequest, ProviderDeleteResponse, ProviderDiscoverySettings,
    ProviderHealthRequest, ProviderHealthResponse, ProviderInstanceAuthMode,
    ProviderInstanceAuthView, ProviderInstanceHealthState, ProviderInstanceHealthView,
    ProviderInstanceInventoryState, ProviderInstanceInventoryView, ProviderInstanceType,
    ProviderInstanceView, ProviderListRequest, ProviderListResponse, ProviderRefreshModelsRequest,
    ProviderRefreshModelsResponse, ProviderReloadResult, ProviderUpdateRequest,
    ProviderUpdateResponse, ProviderValidateRequest, ProviderValidateResponse,
    QueryRouteTraceRequest, QueryRouteTraceResponse, QueryUsageRequest, QueryUsageResponse,
    QuotaQueryRequest, QuotaQueryResponse, QuotaState, RequestControlResult,
    RequestDelegatedControlReq, RerankRequest, RerankResponse, RouteFallbackAttempt,
    RouteResolveRequest, RouteResolveResponse, RouteTrace, RoutingGetRequest, RoutingGetResponse,
    RoutingUpdateRequest, RoutingUpdateResponse, RunnerWriteEnvelope, ServiceReloadSettingsRequest,
    ServiceReloadSettingsResponse, SystemConfigClient, SystemConfigError, TaskControlAction,
    TaskExecutor, TaskManagerClient, TextToImageHelperRequest, TextToImageInvokeRequest,
    TextToImageInvokeResponse, UsageQueryOutputMode, UsageQueryTimeRange, VideoExtendRequest,
    VideoExtendResponse, VideoImageToVideoRequest, VideoImageToVideoResponse,
    VideoTextToVideoRequest, VideoTextToVideoResponse, VideoToVideoRequest, VideoToVideoResponse,
    VideoUpscaleRequest, VideoUpscaleResponse, VisionCaptionRequest, VisionCaptionResponse,
    VisionDetectRequest, VisionDetectResponse, VisionOcrRequest, VisionOcrResponse,
    VisionSegmentRequest, VisionSegmentResponse, AICC_COMPUTE_TASK_SCHEMA_ID,
};
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, Runner, ServerError, ServerErrorCode,
    ServerResult, StreamInfo,
};
use buckyos_kit::KVAction;
use bytes::Bytes;
use futures_util::{stream, StreamExt, TryStreamExt};
use http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE};
use http::{Method, StatusCode, Version};
use http_body_util::{combinators::BoxBody, BodyExt};
use kRPC::{RPCContext, RPCErrors, RPCRequest};
use kRPC::{RPCHandler, RPCResponse};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Mutex};

use crate::call::{
    CallResolver, PricingSource as CallPricingSource, ProviderCallTarget, ResolvedPricing,
    ResolvedProviderCall,
};
use crate::catalog::CatalogSnapshot;
use crate::error::{NativeTaskResumeError, QuotaSourceError, ResourceError, RuntimeError};
use crate::execution::{
    ExecutionEngine, ExecutionOutput, ExecutionState, NativeTaskPoll, NativeTaskResumeDescriptor,
    PinnedProviderTask, ProviderExecution, ProviderExecutionPort, ProviderStartFailure,
    ResumeCredential, ResumeCredentialKind, TaskBinding, TaskManagerPort, TaskSpec,
};
use crate::health::{HealthFailureKind, ModelHealthRegistry};
#[cfg(test)]
use crate::model::{ModelRegistry, ProviderInventory as ModelProviderInventory, RegistryLayers};
use crate::protocol::{
    AdapterStatus, CodecContext, CodecLimits, CodecRegistry, CredentialKind, ExecutionMode,
    HttpTransport, HttpTransportConfig, MaterializedResource as CodecMaterializedResource,
    NativeTaskInput, NativeTaskOperation, NativeTaskOutput, ProtocolError, ProtocolErrorKind,
    ProtocolEvent, ProtocolOutput, ProtocolStream,
};
use crate::provider::{
    builtin_provider_codecs, builtin_provider_registry, resolve_sn_provider_instance_with_config,
    BuiltinProviderRequest, CredentialReference, CredentialResolver, ProviderAuthConfig,
    ProviderConnectionInput, ProviderDiscoverySnapshot, ProviderDraftConfig,
    ProviderDraftValidationStage, ProviderHealthState, ProviderInstanceConfig,
    ProviderQuotaObservation, ProviderQuotaObservationState, ProviderRefreshEvent,
    ProviderRuntimeManager, SnCredentialBroker, SnProviderInstanceInput, StaticCredentialResolver,
};
use crate::resource::{
    ArtifactSpec, EmbeddingArtifactMetadata, NamedDataMgrResourceStore, ReqwestUrlResourceFetcher,
    ResourceAccessContext, ResourceAccessOperation, ResourceAuthorizer, ResourceFailure,
    ResourceLimits, ResourceManager, ResourceStore, ResourceTarget, UrlResourceFetcher,
};
use crate::routing::policy::{
    CredentialScope, ProviderPrivacy, ProviderTrustLevel, ProviderTrustView, ProviderType,
    ProviderTypeSource,
};
use crate::routing::{
    policy_engine_for_route, CallerIdentity, CandidateRuntimeState, ProviderHealthStatus,
    QuotaLookup, QuotaSnapshot, QuotaSourceFactory, QuotaTruthPort, RouteDecision, Router,
    RoutingRequest,
};
use crate::runtime::{
    ConvergenceTrigger, ModelRegistryAssembler, PreparedRuntime, ProviderRuntimeBackend,
    RuntimeBackend, RuntimeFactory,
};
use crate::runtime::{PreparedRuntimeMutation, RuntimeState};
use crate::settings::{
    AiccSettings, MetadataSourceManager, ProductionMetadataOverrideLoader, ProductionRuntimeInputs,
    ProviderLifecyclePolicy, ProviderSettings, SettingsDocument,
};
use crate::storage::{
    AiccStorage, ArtifactUrlSourceRecord, ProviderArtifactIdRecord, RouteTraceRecord,
};
use cloud_update::{
    CloudUpdateClientProfile, CloudUpdateConfig, CloudUpdateManager, NdnCloudObjectFetcher,
};
use model_defaults::ServiceModelAssembler;
#[cfg(test)]
use model_defaults::{builtin_logical_model_definitions, builtin_logical_tree_overlay};

const RESOURCE_INFO: &str = "obj://config/services/aicc/info";
const RESOURCE_SETTINGS: &str = "obj://config/services/aicc/settings";
const CLOUD_UPDATE_CONFIG_KEY: &str = "services/aicc/driver_metadata_update";
const ARTIFACT_OPEN_PATH: &str = "/kapi/aicc/artifact/open";
const MAX_ARTIFACT_OPEN_REQUEST_BYTES: usize = 32 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactOpenRequest {
    url: String,
    #[serde(default)]
    artifact_id: Option<String>,
}

struct AiccHttpServer {
    handler: AiccServerHandler<AiccService>,
}

impl AiccHttpServer {
    fn new(service: AiccService) -> Self {
        Self {
            handler: AiccServerHandler::new(service),
        }
    }

    fn session_token(request: &http::Request<BoxBody<Bytes, ServerError>>) -> Option<String> {
        request
            .headers()
            .get("X-Auth")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                request
                    .headers()
                    .get(AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("Bearer "))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            })
    }

    fn artifact_error_response(
        error: AiccError,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let status = match error.code {
            AiccErrorCode::InvalidRequest => StatusCode::BAD_REQUEST,
            AiccErrorCode::ResourceInvalid => StatusCode::NOT_FOUND,
            AiccErrorCode::PolicyDenied => StatusCode::FORBIDDEN,
            AiccErrorCode::NoProviderAvailable => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_GATEWAY,
        };
        let bytes = serde_json::to_vec(&error).map_err(|error| {
            server_err!(
                ServerErrorCode::InvalidData,
                "serialize artifact error response failed: {}",
                error
            )
        })?;
        Ok(http::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "application/json")
            .body(BoxBody::new(
                http_body_util::Full::new(Bytes::from(bytes)).map_err(|never| match never {}),
            ))
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "build artifact error response failed: {}",
                    error
                )
            })?)
    }

    async fn open_artifact(
        &self,
        request: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let token = match Self::session_token(&request) {
            Some(token) => token,
            None => {
                return Self::artifact_error_response(AiccError::new(
                    AiccErrorCode::PolicyDenied,
                    "session token is required",
                ))
            }
        };
        let body = request.into_body().collect().await.map_err(|error| {
            server_err!(
                ServerErrorCode::BadRequest,
                "read artifact open request failed: {}",
                error
            )
        })?;
        let body = body.to_bytes();
        if body.len() > MAX_ARTIFACT_OPEN_REQUEST_BYTES {
            return Self::artifact_error_response(AiccError::new(
                AiccErrorCode::InvalidRequest,
                "artifact open request is too large",
            ));
        }
        let input: ArtifactOpenRequest = match serde_json::from_slice(&body) {
            Ok(input) => input,
            Err(_) => {
                return Self::artifact_error_response(AiccError::new(
                    AiccErrorCode::InvalidRequest,
                    "artifact open request is invalid",
                ))
            }
        };
        let mut context = RPCContext::default();
        context.token = Some(token);
        let reader = match self
            .handler
            .0
            .open_artifact_url_reader(&input.url, input.artifact_id.as_deref(), context)
            .await
        {
            Ok(reader) => reader,
            Err(error) => return Self::artifact_error_response(error),
        };
        let content_type = reader
            .content_type
            .as_deref()
            .unwrap_or("application/octet-stream");
        let stream = reader.body.map_err(|error| {
            std::io::Error::other(format!(
                "Provider artifact stream failed: {}",
                error.message
            ))
        });
        let body = reqwest::Body::wrap_stream(stream);
        let mut response = http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, content_type)
            .header(CACHE_CONTROL, "no-store");
        if let Some(content_length) = reader.content_length {
            response = response.header(CONTENT_LENGTH, content_length);
        }
        response
            .body(
                BodyExt::map_err(body, |error| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "Provider artifact stream failed: {}",
                        error
                    )
                })
                .boxed(),
            )
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "build artifact stream response failed: {}",
                    error
                )
            })
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
        if request.method() == Method::POST && request.uri().path() == ARTIFACT_OPEN_PATH {
            return self.open_artifact(request).await;
        }
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
        BuckyOSRuntimeType::KernelService,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    api_runtime.login().await.map_err(anyhow::Error::msg)?;
    api_runtime
        .renew_token_from_verify_hub()
        .await
        .map_err(anyhow::Error::msg)?;
    api_runtime
        .set_main_service_port(buckyos_api::AICC_SERVICE_SERVICE_PORT)
        .await;
    let data_dir = api_runtime.get_data_folder().map_err(anyhow::Error::msg)?;
    let buckyos_root_dir = api_runtime.buckyos_root_dir.clone();
    let system_config_url = api_runtime.get_system_config_url();
    let service_token = api_runtime.get_session_token().await;
    let named_store = api_runtime
        .get_named_store()
        .await
        .context("open AICC named resource store")?;
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
    let resource_store: Arc<dyn ResourceStore> =
        Arc::new(NamedDataMgrResourceStore::new(named_store));
    let url_fetcher: Arc<dyn UrlResourceFetcher> =
        Arc::new(ReqwestUrlResourceFetcher::new().context("initialize AICC URL resource fetcher")?);
    let service_runtime: Arc<dyn ServiceRuntime> =
        Arc::new(RuntimeServiceAdapter::new(runtime.clone(), codecs.clone()));
    let model_health = Arc::new(ModelHealthRegistry::default());
    let provider_execution = Arc::new(RuntimeProviderExecutionPort::new(
        runtime.clone(),
        codecs.clone(),
        resource_store.clone(),
        url_fetcher.clone(),
        storage.clone(),
        model_health.clone(),
    ));
    let execution = Arc::new(ExecutionEngine::new(
        storage.clone(),
        Arc::new(TaskManagerExecutionPort::new()),
        provider_execution.clone(),
        storage.clone(),
    ));
    let recovery = execution.clone();
    tokio::spawn(async move {
        if let Err(error) = recovery.recover().await {
            log::error!("AICC task recovery failed: {error}");
        }
    });
    let quota_factory = Arc::new(QuotaSourceFactory::new(Arc::new(
        SystemConfigQuotaTruthPort::new(storage.clone(), runtime.clone()),
    )));
    let inference = Arc::new(RuntimeInferencePort::new(
        runtime.clone(),
        codecs,
        quota_factory.clone(),
        execution.clone(),
        storage.clone(),
        resource_store,
        url_fetcher,
        model_health,
    ));
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
    .with_execution(execution)
    .with_inference(inference)
    .with_artifact_url_reader(provider_execution);
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
    pub routing: buckyos_api::AiccRouteOverlay,
    pub providers: Vec<ProviderInstanceView>,
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
pub(crate) trait InferencePort: Send + Sync {
    async fn resolve_route(
        &self,
        caller: &AuthorizedCaller,
        request: RouteResolveRequest,
    ) -> Result<RouteResolveResponse, RPCErrors>;

    async fn invoke(&self, caller: &AuthorizedCaller, call: AiccCall) -> Result<Value, RPCErrors>;
}

struct InferenceRouteInput {
    trace_id: Option<String>,
    request_id: Option<String>,
    model: String,
    api_type: buckyos_api::ApiType,
    requirements: buckyos_api::ModelRequirement,
    disable: buckyos_api::ModelDisable,
    policy: Option<buckyos_api::RoutePolicy>,
    session_overlay: Option<buckyos_api::AiccRouteOverlay>,
    session_id: Option<String>,
    estimated_input_tokens: Option<u64>,
    estimated_output_tokens: Option<u64>,
}

struct RoutedInference {
    snapshot: Arc<crate::runtime::RuntimeSnapshot>,
    decision: RouteDecision,
    runtime_failover: bool,
    trace_id: String,
    request_id: String,
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
    inference: Option<Arc<dyn InferencePort>>,
    artifact_url_reader: Option<Arc<RuntimeProviderExecutionPort>>,
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
            inference: None,
            artifact_url_reader: None,
            settings_mutation: Mutex::new(()),
        }
    }

    pub(crate) fn with_execution(mut self, execution: Arc<ExecutionEngine>) -> Self {
        self.execution = Some(execution);
        self
    }

    pub(crate) fn with_inference(mut self, inference: Arc<dyn InferencePort>) -> Self {
        self.inference = Some(inference);
        self
    }

    pub(crate) fn with_artifact_url_reader(
        mut self,
        artifact_url_reader: Arc<RuntimeProviderExecutionPort>,
    ) -> Self {
        self.artifact_url_reader = Some(artifact_url_reader);
        self
    }

    async fn open_artifact_url_reader(
        &self,
        url: &str,
        artifact_id: Option<&str>,
        context: RPCContext,
    ) -> Result<crate::protocol::ArtifactUrlReader, AiccError> {
        let caller = self
            .authorize(&context, "read", RESOURCE_INFO)
            .await
            .map_err(|_| {
                AiccError::new(AiccErrorCode::PolicyDenied, "artifact access is denied")
            })?;
        let reader = self.artifact_url_reader.as_ref().ok_or_else(|| {
            AiccError::new(
                AiccErrorCode::InternalError,
                "artifact URL reader is unavailable",
            )
        })?;
        reader
            .open_artifact_url_reader(&caller.tenant_id, url, artifact_id)
            .await
    }

    async fn authorize(
        &self,
        context: &RPCContext,
        action: &'static str,
        resource: &'static str,
    ) -> Result<AuthorizedCaller, RPCErrors> {
        self.authorizer.authorize(context, action, resource).await
    }

    async fn invoke_typed<R>(
        &self,
        caller: &AuthorizedCaller,
        call: AiccCall,
    ) -> Result<R, RPCErrors>
    where
        R: DeserializeOwned,
    {
        let inference = self.inference.as_ref().ok_or_else(|| {
            inference_error(
                AiccErrorCode::InternalError,
                "inference runtime is unavailable",
            )
        })?;
        let response = inference.invoke(caller, call).await?;
        serde_json::from_value(response).map_err(|error| {
            inference_error(
                AiccErrorCode::InternalError,
                format!("canonical inference response is invalid: {error}"),
            )
        })
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

fn validate_request(request: &ProviderAddRequest) -> ProviderValidateRequest {
    ProviderValidateRequest {
        provider_instance_name: Some(request.provider_instance_name.clone()),
        provider_type: request.provider_type.clone(),
        provider_profile_id: request.provider_profile_id.clone(),
        protocol_family_id: request.protocol_family_id.clone(),
        protocol_adapter_id: request.protocol_adapter_id.clone(),
        base_url: request.base_url.clone(),
        operation_base_urls: request.operation_base_urls.clone(),
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
        protocol_family_id: provider.protocol_family_id.clone(),
        protocol_adapter_id: Some(provider.protocol_adapter_id.clone()),
        base_url: provider.base_url.clone(),
        operation_base_urls: provider.operation_base_urls.clone(),
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
    let dynamic_login = request
        .auth
        .as_ref()
        .is_some_and(|auth| matches!(auth, ProviderAuthSettings::DynamicLogin { .. }));
    Ok(ProviderSettings {
        provider_instance_name: request.provider_instance_name,
        provider_type: request.provider_type,
        provider_profile_id: request.provider_profile_id,
        protocol_family_id: request.protocol_family_id,
        protocol_adapter_id,
        base_url: request.base_url,
        operation_base_urls: request.operation_base_urls,
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
        lifecycle: ProviderLifecyclePolicy {
            non_deletable: dynamic_login,
        },
    })
}

fn apply_provider_update(provider: &mut ProviderSettings, request: ProviderUpdateRequest) {
    if let Some(enabled) = request.enabled {
        provider.enabled = enabled;
    }
    if let Some(base_url) = request.base_url {
        provider.base_url = base_url;
    }
    if let Some(operation_base_urls) = request.operation_base_urls {
        provider.operation_base_urls = operation_base_urls;
    }
    if let Some(credential) = request.credential {
        provider.credentials = credential;
    }
    if let Some(profile) = request.provider_profile_id {
        provider.provider_profile_id = profile;
    }
    if let Some(family) = request.protocol_family_id {
        provider.protocol_family_id = Some(family);
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

fn conflict_error(expected: u64, actual: u64) -> RPCErrors {
    AiccError::settings_revision_conflict(expected, actual).to_krpc_error()
}

fn invalid_request(message: &'static str) -> RPCErrors {
    AiccError::new(buckyos_api::AiccErrorCode::InvalidRequest, message).to_krpc_error()
}

fn is_non_deletable_dynamic_login_provider(provider: &ProviderSettings) -> bool {
    if !provider.lifecycle.non_deletable {
        return false;
    }
    provider
        .auth
        .as_ref()
        .is_some_and(|auth| matches!(auth, ProviderAuthSettings::DynamicLogin { .. }))
}

fn to_rpc_error(error: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(error.to_string())
}

fn task_manager_error(
    operation: impl std::fmt::Display,
    error: RPCErrors,
) -> buckyos_api::AiccError {
    if error
        .to_string()
        .contains(buckyos_api::TASK_ERR_IDEMPOTENCY_CONFLICT)
    {
        return buckyos_api::AiccError::new(
            buckyos_api::AiccErrorCode::IdempotencyConflict,
            "idempotency key was already used with a different canonical request body",
        );
    }
    log::warn!(
        "TaskMgr operation failed for AICC runner: operation=\"{}\" error=\"{}\"",
        operation,
        error
    );
    buckyos_api::AiccError::new(
        buckyos_api::AiccErrorCode::InternalError,
        "TaskMgr operation failed",
    )
}

fn replace_resource_ref_value(
    value: &mut Value,
    old: &buckyos_api::ResourceRef,
    new: &buckyos_api::ResourceRef,
) -> Result<(), ProtocolError> {
    let old = serde_json::to_value(old).map_err(|_| {
        ProtocolError::invalid_configuration("inline artifact resource could not be serialized")
    })?;
    let new = serde_json::to_value(new).map_err(|_| {
        ProtocolError::invalid_configuration("inline artifact resource could not be serialized")
    })?;
    replace_json_value(value, &old, &new);
    Ok(())
}

fn append_materialized_artifact_metadata(
    value: &mut Value,
    obj_id: String,
    name: String,
    mime: Option<String>,
) {
    let Some(root) = value.as_object_mut() else {
        return;
    };
    let extra = root
        .entry("extra".to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extra) = extra.as_object_mut() else {
        return;
    };
    let materialized = extra
        .entry("materialized_artifacts".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(materialized) = materialized.as_array_mut() else {
        return;
    };
    materialized.push(json!({
        "obj_id": obj_id,
        "name": name,
        "mime": mime,
    }));
}

fn collect_inline_base64_resource_refs(value: &Value, found: &mut Vec<buckyos_api::ResourceRef>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_inline_base64_resource_refs(item, found);
            }
        }
        Value::Object(object) => {
            if object.get("kind").and_then(Value::as_str) == Some("base64") {
                if let Ok(resource @ buckyos_api::ResourceRef::Base64 { .. }) =
                    serde_json::from_value::<buckyos_api::ResourceRef>(Value::Object(
                        object.clone(),
                    ))
                {
                    if !found.contains(&resource) {
                        found.push(resource);
                    }
                    return;
                }
            }
            for item in object.values() {
                collect_inline_base64_resource_refs(item, found);
            }
        }
        _ => {}
    }
}

fn replace_json_value(value: &mut Value, old: &Value, new: &Value) {
    if value == old {
        *value = new.clone();
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                replace_json_value(item, old, new);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                replace_json_value(item, old, new);
            }
        }
        _ => {}
    }
}

fn runtime_admin_snapshot(
    snapshot: &crate::runtime::RuntimeSnapshot,
    codecs: &CodecRegistry,
) -> RuntimeAdminSnapshot {
    let providers = snapshot
        .settings
        .providers
        .iter()
        .map(|settings| {
            let runtime = settings
                .enabled
                .then(|| snapshot.providers.get(&settings.provider_instance_name))
                .flatten();
            let configured_auth = settings.auth.as_ref().map(provider_auth_config);
            let default_credential_kind = runtime
                .map(|provider| provider.profile.credential.kind.as_str().to_string())
                .or_else(|| {
                    snapshot
                        .catalog
                        .resolve_provider_configuration(&settings.provider_profile_id)
                        .ok()
                        .map(|configuration| {
                            provider_credential_kind_name(configuration.credential.kind).to_string()
                        })
                });
            let auth = match configured_auth {
                Some(ProviderAuthConfig::ApiKey {
                    credential_kind, ..
                }) => ProviderInstanceAuthView {
                    mode: Some(ProviderInstanceAuthMode::ApiKey),
                    credential_kind: credential_kind
                        .map(|kind| kind.as_str().to_string())
                        .or(default_credential_kind),
                    configured: provider_credentials_configured(&settings.credentials),
                },
                Some(ProviderAuthConfig::DynamicLogin { .. }) => ProviderInstanceAuthView {
                    mode: Some(ProviderInstanceAuthMode::DynamicLogin),
                    credential_kind: None,
                    configured: provider_credentials_configured(&settings.credentials),
                },
                None => ProviderInstanceAuthView {
                    mode: Some(ProviderInstanceAuthMode::ApiKey),
                    credential_kind: default_credential_kind,
                    configured: provider_credentials_configured(&settings.credentials),
                },
            };
            let (inventory, health) = if !settings.enabled {
                (
                    ProviderInstanceInventoryView {
                        state: ProviderInstanceInventoryState::Disabled,
                        revision: None,
                        model_count: 0,
                        updated_at_ms: None,
                    },
                    ProviderInstanceHealthView {
                        state: ProviderInstanceHealthState::Disabled,
                        checked_at_ms: None,
                    },
                )
            } else if let Some(runtime) = runtime {
                (
                    ProviderInstanceInventoryView {
                        state: ProviderInstanceInventoryState::Loaded,
                        revision: runtime.inventory.inventory_revision.clone(),
                        model_count: runtime.inventory.models.len() as u64,
                        updated_at_ms: Some(runtime.inventory.discovered_at_ms),
                    },
                    ProviderInstanceHealthView {
                        state: provider_health_state(runtime.inventory.health),
                        checked_at_ms: Some(runtime.inventory.discovered_at_ms),
                    },
                )
            } else {
                (
                    ProviderInstanceInventoryView {
                        state: ProviderInstanceInventoryState::NotLoaded,
                        revision: None,
                        model_count: 0,
                        updated_at_ms: None,
                    },
                    ProviderInstanceHealthView {
                        state: ProviderInstanceHealthState::NotLoaded,
                        checked_at_ms: None,
                    },
                )
            };
            ProviderInstanceView {
                provider_instance_name: settings.provider_instance_name.clone(),
                provider_type: settings.provider_type.clone(),
                provider_profile_id: settings.provider_profile_id.clone(),
                protocol_adapter_id: settings.protocol_adapter_id.clone(),
                base_url: settings.base_url.clone(),
                operation_base_urls: settings.operation_base_urls.clone(),
                enabled: settings.enabled,
                auth,
                inventory,
                health,
            }
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
                    region_base_urls: provider.connection.region_base_urls.clone(),
                    operation_base_urls: provider.connection.operation_base_urls.clone(),
                    protocol_adapter_id: provider.protocol_adapter_id.clone(),
                    discovery_behavior_id: provider.discovery_behavior_id.clone(),
                    dynamic_login_behavior_id: provider.dynamic_login_behavior_id.clone(),
                    connection_behavior_id: provider.connection_behavior_id.clone(),
                    provider_rules_id: provider.provider_rules_id.clone(),
                    ui_hints: provider.ui_hints.clone(),
                })
                .collect(),
        },
        protocol_adapters: protocol_adapter_response(
            codecs,
            &snapshot.catalog.custom_provider_adapter_ids(),
        ),
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
            "directory": model_directory_json(&snapshot.models),
            "logical_definitions": logical_definitions_json(&snapshot.models),
            "catalog_models": snapshot.catalog.catalog_model_mounts(),
            "catalog_patterns": snapshot.catalog.model_drivers().map(|driver| json!({
                "model_driver_id": driver.model_driver_id,
                "patterns": driver.patterns,
            })).collect::<Vec<_>>(),
            "generation": snapshot.generation,
        }),
        routing: snapshot.settings.session_config.clone().unwrap_or_default(),
        providers,
        inventory_revision,
        provider_health,
    }
}

fn model_directory_json(models: &crate::model::ModelRegistry) -> Value {
    let visible_paths = visible_logical_paths(models);
    let exact_models = models
        .model_views()
        .into_iter()
        .map(|model| model.exact_model)
        .collect::<BTreeSet<_>>();
    let directory = models
        .logical_model_views()
        .into_iter()
        .filter(|logical| visible_paths.contains(&logical.path))
        .map(|logical| {
            let items = logical
                .items
                .into_iter()
                .filter(|item| {
                    visible_paths.contains(&item.target) || exact_models.contains(&item.target)
                })
                .map(|item| {
                    (
                        item.name,
                        json!({
                            "target": item.target,
                            "weight": item.weight,
                        }),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            (logical.path, Value::Object(items))
        })
        .collect::<serde_json::Map<_, _>>();
    Value::Object(directory)
}

fn logical_definitions_json(models: &crate::model::ModelRegistry) -> Value {
    let visible_paths = visible_logical_paths(models);
    Value::Array(
        models
            .logical_model_views()
            .into_iter()
            .filter(|logical| visible_paths.contains(&logical.path))
            .map(|logical| {
                json!({
                    "path": logical.path,
                    "api_type": logical.api_type,
                    "min_line": logical.min_line,
                    "disable_line": logical.disable_line,
                    "default_options": logical.default_options,
                    "scheduler_profile": logical.scheduler_profile,
                    "fallback": logical.fallback,
                })
            })
            .collect(),
    )
}

fn visible_logical_paths(models: &crate::model::ModelRegistry) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    let logical_views = models.logical_model_views();
    for logical in &logical_views {
        if logical.api_type.is_some() {
            insert_logical_path_with_parents(&mut paths, &logical.path);
        }
    }

    for model in models.model_views() {
        for mount in model.logical_mounts {
            insert_logical_path_with_parents(&mut paths, &mount);
        }
    }

    let exact_models = models
        .model_views()
        .into_iter()
        .map(|model| model.exact_model)
        .collect::<BTreeSet<_>>();
    let mut changed = true;
    while changed {
        changed = false;
        for logical in &logical_views {
            if paths.contains(&logical.path) {
                continue;
            }
            if logical
                .items
                .iter()
                .any(|item| paths.contains(&item.target) || exact_models.contains(&item.target))
            {
                let before = paths.len();
                insert_logical_path_with_parents(&mut paths, &logical.path);
                changed = paths.len() != before;
            }
        }
    }

    paths
}

fn insert_logical_path_with_parents(paths: &mut BTreeSet<String>, path: &str) {
    let mut current = Some(path);
    while let Some(path) = current {
        paths.insert(path.to_string());
        current = parent_logical_path(path);
    }
}

fn parent_logical_path(path: &str) -> Option<&str> {
    path.rfind('.').map(|index| &path[..index])
}

fn provider_credentials_configured(credentials: &ProviderCredentials) -> bool {
    !credentials.is_empty()
}

fn provider_credential_kind_name(kind: crate::catalog::ProviderCredentialKind) -> &'static str {
    match kind {
        crate::catalog::ProviderCredentialKind::Bearer => "bearer",
        crate::catalog::ProviderCredentialKind::NamedHeader => "named_header",
        crate::catalog::ProviderCredentialKind::FalKey => "fal_key",
        crate::catalog::ProviderCredentialKind::GlmJwt => "glm_jwt",
    }
}

fn provider_health_state(health: ProviderHealthState) -> ProviderInstanceHealthState {
    match health {
        ProviderHealthState::Unknown => ProviderInstanceHealthState::Unknown,
        ProviderHealthState::Healthy => ProviderInstanceHealthState::Healthy,
        ProviderHealthState::Degraded => ProviderInstanceHealthState::Degraded,
        ProviderHealthState::Unavailable | ProviderHealthState::Stopped => {
            ProviderInstanceHealthState::Unavailable
        }
    }
}

fn protocol_adapter_response(
    codecs: &CodecRegistry,
    custom_provider_adapter_ids: &BTreeSet<String>,
) -> ProtocolAdapterListResponse {
    ProtocolAdapterListResponse {
        adapters: codecs
            .adapters()
            .map(|adapter| buckyos_api::ProtocolAdapterView {
                protocol_family_id: adapter.protocol_family_id.clone(),
                protocol_adapter_id: adapter.protocol_adapter_id.clone(),
                custom_provider_selectable: custom_provider_adapter_ids
                    .contains(&adapter.protocol_adapter_id),
                interface_generation: adapter.interface_generation.clone(),
                status: match adapter.status {
                    AdapterStatus::Stable => buckyos_api::ProtocolAdapterStatus::Stable,
                    AdapterStatus::Preview => buckyos_api::ProtocolAdapterStatus::Preview,
                    AdapterStatus::Deprecated => buckyos_api::ProtocolAdapterStatus::Deprecated,
                },
                probe_priority: adapter.probe_priority,
                probe_path: adapter.probe_path.clone(),
                base_adapter_id: adapter.base_adapter_id.clone(),
                component_adapter_ids: adapter.component_adapter_ids.clone(),
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

#[cfg(test)]
fn disabled_metadata_view(settings_revision: u64) -> DriverMetadataUpdateSetResponse {
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
mod tests;
