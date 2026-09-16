use crate::catalog::{CatalogKind, ResolvedProviderOrigin};
use crate::provider::{ProviderDraftValidationStage, ProviderRefreshFailure};
use crate::routing::policy::PolicyScope;
use crate::routing::{FilterReasonTrace, FilteredCandidateTrace};
use crate::settings::MetadataSource;
use buckyos_api::{AiccError, AiccErrorCode};
use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug)]
pub(crate) enum CallLoweringError {
    UnsupportedCanonicalCall(String),
    InvalidCanonicalRequest(String),
    InvalidExactModel(ModelRegistryError),
    RouteMismatch(String),
    Catalog(CatalogResolveError),
    MissingModelVariant {
        model_driver_id: String,
        variant: String,
    },
    AmbiguousModelVariant {
        model_driver_id: String,
        variant: String,
    },
    MissingProviderVariant {
        provider_rules_id: String,
        variant: String,
    },
    AmbiguousProviderVariant {
        provider_rules_id: String,
        variant: String,
    },
    UnknownAdapter(String),
    MissingDefaultOperation {
        adapter_id: String,
        api_type: String,
    },
    AmbiguousDefaultOperation {
        adapter_id: String,
        api_type: String,
    },
    RouteOperationMismatch {
        routed: String,
        lowered: String,
    },
    UnsupportedOperation(String),
    UnsupportedExecutionMode {
        adapter_id: String,
        operation: String,
        api_type: String,
        execution_mode: String,
    },
    InvalidRule(String),
}

#[derive(Debug)]
pub(crate) enum CatalogBuildError {
    InvalidJson {
        kind: CatalogKind,
        position: usize,
        source: serde_json::Error,
    },
    InvalidFormat {
        kind: CatalogKind,
        id: String,
        expected: &'static str,
        actual: String,
    },
    UnsupportedSchema {
        kind: CatalogKind,
        id: String,
        schema_version: u32,
        schema_revision: u32,
    },
    UnsupportedFeature {
        owner: String,
        feature: String,
    },
    RevisionAheadOfSnapshot {
        kind: CatalogKind,
        id: String,
        revision_seq: u64,
        target_revision_seq: u64,
    },
    DuplicateCatalog {
        kind: CatalogKind,
        id: String,
    },
    DuplicateExactRule {
        kind: CatalogKind,
        catalog_id: String,
        model_id: String,
    },
    DuplicateKnownProvider {
        provider_profile_id: String,
    },
    UnknownReference {
        owner: String,
        field: &'static str,
        target: String,
    },
    ReferenceMismatch {
        owner: String,
        field: &'static str,
        target: String,
        expected: String,
    },
    InvalidValue {
        owner: String,
        field: &'static str,
        reason: String,
    },
    StaticDynamicBoundary {
        owner: String,
        field: String,
    },
    Match(MatchCompileError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CatalogResolveError {
    UnknownKnownProvider {
        provider_profile_id: String,
    },
    MissingProviderRulesReference {
        provider_profile_id: String,
    },
    ProviderRulesIdentityMismatch {
        provider_profile_id: String,
        provider_rules_id: String,
        rules_provider_profile_id: String,
    },
    UnknownModelDriver {
        model_driver_id: String,
    },
    UnknownProviderRules {
        provider_profile_id: String,
    },
    AmbiguousModelDrivers {
        origin_model_id: String,
        model_driver_ids: Vec<String>,
    },
    OriginMappingNotFound {
        provider_profile_id: String,
        provider_model_id: String,
    },
    UnknownOriginProvider {
        provider_profile_id: String,
        origin_provider: String,
    },
    OriginDriverOutsideMetadataDrivers {
        provider_profile_id: String,
        model_driver_id: String,
    },
    ConflictingOriginMappings {
        provider_profile_id: String,
        provider_model_id: String,
        resolved: Vec<ResolvedProviderOrigin>,
    },
}

#[derive(Debug, Error)]
pub(crate) enum CloudUpdateError {
    #[error("invalid cloud update config: {0}")]
    InvalidConfig(String),
    #[error("invalid cloud update protocol: {0}")]
    InvalidProtocol(String),
    #[error("cloud update download failed: {0}")]
    Download(String),
    #[error("cloud update cache I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("cloud update JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cloud catalog is invalid: {0}")]
    Catalog(String),
}

#[derive(Clone, Debug)]
pub(crate) enum NativeTaskResumeError {
    CredentialUnavailable,
    Protocol(crate::protocol::ProtocolError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MatchCompileError {
    pub schema: &'static str,
    pub dimension: Option<String>,
    pub kind: MatchCompileErrorKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MatchCompileErrorKind {
    ShorthandNotAllowed,
    EmptyObject,
    UnknownDimension,
    InvalidValueType,
    InvalidOperator,
    InvalidNotOperand,
    InvalidExistsOperand,
    RangeNotAllowed,
    EmptyRange,
    RangeFlagWithoutBound,
    InvalidRangeFlag,
    InvalidRangeBound,
    ReversedRange,
    InvalidEscape,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ModelRegistryError {
    InvalidIdentity {
        field: &'static str,
        value: String,
    },
    InvalidExactModelName(String),
    InvalidVariant(String),
    InvalidLogicalPath(String),
    ApiNamespaceMismatch {
        path: String,
        api_type: String,
    },
    MountApiMismatch {
        path: String,
        provider_model_id: String,
    },
    CrossNamespaceLink {
        from: String,
        to: String,
    },
    DuplicateProviderInstance(String),
    DuplicateExactModel(String),
    DuplicateVariant {
        provider_model_id: String,
        variant: String,
    },
    UnknownModelDriver(String),
    MissingApiTypes(String),
    DuplicateLogicalDefinition(String),
    DuplicateOverlayPath(String),
    InvalidItemName(String),
    InvalidWeight {
        field: String,
        weight: f64,
    },
    ItemsAndOverridesConflict(String),
    UnknownItemOverride {
        path: String,
        item: String,
    },
    UnknownLogicalProfile(String),
    InvalidFallbackRule(String),
    LogicalTreeLoop(String),
    FallbackLoop(String),
    FallbackDepthExceeded(usize),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum PolicyError {
    LockedOverride {
        field: &'static str,
        locked_by: PolicyScope,
        attempted_by: PolicyScope,
    },
    InvalidPolicy(String),
    QuotaSourceUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtocolErrorKind {
    InvalidConfiguration,
    InvalidRequest,
    Authentication,
    Transport,
    Timeout,
    ResponseTooLarge,
    InvalidResponse,
    ProviderRejected,
    DuplicateAdapter,
    UnknownAdapter,
    UnsupportedOperation,
    DeadlineExceeded,
    Cancelled,
    WebhookRejected,
}

pub(crate) fn protocol_error_kind_from_http_status(
    status: reqwest::StatusCode,
) -> ProtocolErrorKind {
    use reqwest::StatusCode;
    match status {
        StatusCode::BAD_REQUEST
        | StatusCode::NOT_FOUND
        | StatusCode::METHOD_NOT_ALLOWED
        | StatusCode::UNPROCESSABLE_ENTITY => ProtocolErrorKind::InvalidRequest,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProtocolErrorKind::Authentication,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => ProtocolErrorKind::Timeout,
        _ => ProtocolErrorKind::Transport,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProtocolError {
    pub kind: ProtocolErrorKind,
    pub message: String,
    pub provider_code: Option<String>,
    pub request_id: Option<String>,
    pub retry_after: Option<Duration>,
}

pub(crate) type ProtocolResultValue<T> = Result<T, ProtocolError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QuotaSourceError;

#[derive(Debug, Error)]
pub(crate) enum ProviderError {
    #[error("invalid provider configuration: {0}")]
    InvalidConfiguration(String),
    #[error("provider profile `{0}` is not registered")]
    UnknownProfile(String),
    #[error("protocol adapter `{0}` is not registered")]
    UnknownAdapter(String),
    #[error("provider instance `{0}` is already registered")]
    DuplicateInstance(String),
    #[error("provider instance `{0}` is not registered")]
    UnknownInstance(String),
    #[error("credential resolution failed: {0}")]
    Credential(String),
    #[error("provider discovery failed: {0}")]
    Discovery(String),
    #[error("inventory build failed: {0}")]
    Inventory(String),
    #[error("inventory storage failed: {0}")]
    Storage(String),
    #[error("provider instance stopped before refresh could commit")]
    Stopped,
    #[error("provider inventory candidate is stale")]
    StaleCandidate,
}

pub(crate) type ProviderResult<T> = Result<T, ProviderError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderDraftValidationError {
    pub stage: ProviderDraftValidationStage,
    pub kind: ProviderRefreshFailure,
}

#[derive(Clone, Error, PartialEq, Eq)]
#[error("resource_invalid: {message}")]
pub(crate) struct ResourceError {
    pub failure: crate::resource::ResourceFailure,
    pub(crate) message: String,
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum RoutingError {
    InvalidRequest(String),
    InvalidExactModel(String),
    ExactModelUnavailable {
        exact_model: String,
        reasons: Vec<FilterReasonTrace>,
    },
    NoCandidate {
        model: String,
        filtered: Vec<FilteredCandidateTrace>,
    },
    FallbackNotAllowed(String),
    InvalidFallback(String),
    FallbackLoop(String),
    FallbackDepthExceeded(usize),
    Registry(ModelRegistryError),
}

#[derive(Debug, Error)]
pub(crate) enum RuntimeError {
    #[error("AICC runtime has stopped")]
    Stopped,
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("runtime backend failed: {0}")]
    Backend(String),
    #[error("metadata target sequence moved backwards from {current} to {observed}")]
    MetadataRollback { current: u64, observed: u64 },
    #[error("candidate catalog sequence is {actual}, expected {expected}")]
    CandidateCatalogMismatch { expected: u64, actual: u64 },
    #[error("reload candidate reused the currently published runtime backend")]
    CandidateReusesBackend,
    #[error("candidate provider set differs from enabled settings")]
    CandidateProviderSetMismatch {
        expected: BTreeSet<String>,
        actual: BTreeSet<String>,
    },
    #[error("candidate provider `{0}` still has metadata_updating_seq")]
    CandidateStillUpdating(String),
    #[error(
        "candidate provider `{provider_instance_name}` is routable at applied seq {applied}, target is {target}"
    )]
    MixedCatalogRevision {
        provider_instance_name: String,
        applied: u64,
        target: u64,
    },
    #[error("model `{exact_model}` references non-routable provider `{provider_instance_name}`")]
    UnconvergedModel {
        exact_model: String,
        provider_instance_name: String,
    },
}

#[derive(Debug, Error)]
pub(crate) enum SettingsError {
    #[error("invalid AICC settings JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("invalid settings field `{field}`: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("provider instance `{0}` appears more than once")]
    DuplicateProvider(String),
    #[error("duplicate {kind} catalog `{catalog_id}` in {metadata_source:?} metadata source")]
    DuplicateMetadataFile {
        metadata_source: MetadataSource,
        kind: CatalogKind,
        catalog_id: String,
    },
    #[error(
        "metadata file `{catalog_id}` declares {actual:?} source but was placed in {expected:?}"
    )]
    MetadataSourceMismatch {
        expected: MetadataSource,
        actual: MetadataSource,
        catalog_id: String,
    },
    #[error("effective catalog is invalid: {0}")]
    InvalidCatalog(#[from] CatalogBuildError),
    #[error("metadata filesystem I/O failed: {0}")]
    MetadataIo(#[source] std::io::Error),
    #[error("invalid metadata path `{path}`: {reason}")]
    InvalidMetadataPath { path: PathBuf, reason: String },
    #[error("{0:?} metadata changed while a reload snapshot was being captured")]
    MetadataSourceChanged(MetadataSource),
    #[error("unsupported system-config metadata schema version {0}")]
    UnsupportedMetadataSchema(u32),
    #[error("system-config metadata read failed: {0}")]
    SystemConfig(String),
    #[error("cloud metadata read failed: {0}")]
    Cloud(String),
}

#[derive(Debug, Error)]
pub(crate) enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid storage record: {0}")]
    InvalidRecord(String),
    #[error("invalid cursor")]
    InvalidCursor,
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub(crate) type StorageResult<T> = Result<T, StorageError>;

impl NativeTaskResumeError {
    pub(crate) fn into_aicc_error(self) -> AiccError {
        match self {
            Self::CredentialUnavailable => AiccError {
                code: AiccErrorCode::ProviderError,
                message: "pinned native task credential can no longer be resolved".to_string(),
                provider_code: None,
                retriable: false,
                details: None,
            },
            Self::Protocol(error) => error.into(),
        }
    }
}

impl ProtocolError {
    pub(crate) fn new(kind: ProtocolErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            provider_code: None,
            request_id: None,
            retry_after: None,
        }
    }

    pub(crate) fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    pub(crate) fn with_provider_code(mut self, provider_code: Option<String>) -> Self {
        self.provider_code = provider_code;
        self
    }

    pub(crate) fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }

    pub(crate) fn invalid_configuration(message: impl Into<String>) -> Self {
        Self::new(ProtocolErrorKind::InvalidConfiguration, message)
    }

    pub(crate) fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ProtocolErrorKind::InvalidRequest, message)
    }

    pub(crate) fn invalid_response(message: impl Into<String>) -> Self {
        Self::new(ProtocolErrorKind::InvalidResponse, message)
    }

    pub(crate) fn is_model_unavailable(&self) -> bool {
        matches!(self.provider_code.as_deref(), Some("1211" | "1212"))
            || contains_any(
                &self.message,
                &[
                    "model does not exist",
                    "model not found",
                    "unknown model",
                    "当前模型不支持",
                    "始终思考",
                    "不支持关闭思考",
                ],
            )
    }

    pub(crate) fn is_account_exhausted(&self) -> bool {
        contains_any(
            &self.message,
            &[
                "余额不足",
                "请充值",
                "insufficient balance",
                "insufficient quota",
                "quota exhausted",
                "quota exceeded",
            ],
        )
    }

    pub(crate) fn retry_same_model(&self) -> bool {
        !self.requires_candidate_change()
            && (matches!(
                self.kind,
                ProtocolErrorKind::Transport
                    | ProtocolErrorKind::Timeout
                    | ProtocolErrorKind::DeadlineExceeded
                    | ProtocolErrorKind::InvalidResponse
            ) || self.retry_after.is_some())
    }

    pub(crate) fn allows_model_failover(&self) -> bool {
        self.requires_candidate_change() || self.retry_same_model()
    }

    fn requires_candidate_change(&self) -> bool {
        self.is_model_unavailable()
            || contains_any(
                &self.message,
                &[
                    "quota exhausted",
                    "quota exceeded",
                    "insufficient quota",
                    "insufficient balance",
                    "model temporarily unavailable",
                    "余额不足",
                    "请充值",
                ],
            )
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    let value = value.to_ascii_lowercase();
    needles.iter().any(|needle| value.contains(needle))
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProtocolError {}

impl From<ProtocolError> for AiccError {
    fn from(error: ProtocolError) -> Self {
        let retriable = matches!(
            error.kind,
            ProtocolErrorKind::Transport
                | ProtocolErrorKind::Timeout
                | ProtocolErrorKind::DeadlineExceeded
        );
        let code = match error.kind {
            ProtocolErrorKind::InvalidConfiguration
            | ProtocolErrorKind::DuplicateAdapter
            | ProtocolErrorKind::UnknownAdapter => AiccErrorCode::InternalError,
            ProtocolErrorKind::InvalidRequest => AiccErrorCode::InvalidRequest,
            ProtocolErrorKind::UnsupportedOperation => AiccErrorCode::UnsupportedOperation,
            ProtocolErrorKind::Authentication | ProtocolErrorKind::ProviderRejected => {
                AiccErrorCode::ProviderError
            }
            ProtocolErrorKind::WebhookRejected => AiccErrorCode::PolicyDenied,
            ProtocolErrorKind::Timeout | ProtocolErrorKind::DeadlineExceeded => {
                AiccErrorCode::Timeout
            }
            ProtocolErrorKind::Cancelled => AiccErrorCode::Cancelled,
            ProtocolErrorKind::Transport
            | ProtocolErrorKind::ResponseTooLarge
            | ProtocolErrorKind::InvalidResponse => AiccErrorCode::ProviderError,
        };
        let mut mapped = AiccError::new(code, error.message);
        mapped.provider_code = error.provider_code;
        mapped.retriable = retriable;
        mapped.details = (error.request_id.is_some() || error.retry_after.is_some()).then(|| {
            serde_json::json!({
                "request_id": error.request_id,
                "retry_after_ms": error.retry_after.map(|duration| duration.as_millis() as u64),
            })
        });
        mapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_protocol_failures_to_stable_public_errors() {
        let timeout: AiccError = ProtocolError::new(ProtocolErrorKind::Timeout, "timed out").into();
        assert_eq!(timeout.code, AiccErrorCode::Timeout);

        let malformed: AiccError = ProtocolError::invalid_response("bad response").into();
        assert_eq!(malformed.code, AiccErrorCode::ProviderError);

        let configuration: AiccError = ProtocolError::invalid_configuration("bad adapter").into();
        assert_eq!(configuration.code, AiccErrorCode::InternalError);
    }

    #[test]
    fn retry_classification_separates_jitter_from_candidate_change() {
        let timeout = ProtocolError::new(ProtocolErrorKind::Timeout, "timed out");
        assert!(timeout.retry_same_model());
        assert!(timeout.allows_model_failover());

        for error in [
            ProtocolError::new(ProtocolErrorKind::InvalidRequest, "model does not exist")
                .with_provider_code(Some("1211".to_owned())),
            ProtocolError::new(ProtocolErrorKind::ProviderRejected, "quota exhausted"),
        ] {
            assert!(!error.retry_same_model());
            assert!(error.allows_model_failover());
        }

        let authentication =
            ProtocolError::new(ProtocolErrorKind::Authentication, "invalid API key");
        assert!(!authentication.retry_same_model());
        assert!(!authentication.allows_model_failover());
    }

    #[test]
    fn glm_thinking_capability_errors_are_model_unavailable() {
        for message in [
            "OpenAI 1210: 该模型始终思考，不支持关闭思考；请使用 low、high 或 max。",
            "OpenAI 1210: 该模型不支持关闭思考",
            "OpenAI 1212: 当前模型不支持 embeddings 调用方式",
        ] {
            let error = ProtocolError::new(ProtocolErrorKind::ProviderRejected, message);
            assert!(error.is_model_unavailable(), "{message}");
            assert!(!error.retry_same_model());
            assert!(error.allows_model_failover());
        }

        let generic_param_error = ProtocolError::new(
            ProtocolErrorKind::ProviderRejected,
            "OpenAI 1210: API 调用参数有误，请检查文档",
        );
        assert!(!generic_param_error.is_model_unavailable());

        let model_missing = ProtocolError::new(
            ProtocolErrorKind::ProviderRejected,
            "OpenAI 1211: 模型不存在，请检查模型代码",
        )
        .with_provider_code(Some("1211".to_owned()));
        assert!(model_missing.is_model_unavailable());
    }

    #[test]
    fn account_exhaustion_is_recognized_and_not_retried_same_model() {
        let error = ProtocolError::new(
            ProtocolErrorKind::Transport,
            "OpenAI 1113: 余额不足或无可用资源包,请充值。",
        );
        assert!(error.is_account_exhausted());
        assert!(!error.is_model_unavailable());
        assert!(!error.retry_same_model());
        assert!(error.allows_model_failover());
    }
}
