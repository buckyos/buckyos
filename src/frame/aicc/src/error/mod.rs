use crate::catalog::{CatalogKind, ResolvedProviderOrigin};
use crate::provider::{ProviderDraftValidationStage, ProviderRefreshFailure};
use crate::routing::policy::PolicyScope;
use crate::routing::{FilterReasonTrace, FilteredCandidateTrace};
use crate::settings::MetadataSource;
use buckyos_api::{AiccError, AiccErrorCode};
use std::collections::BTreeSet;
use std::path::PathBuf;
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
    #[error("provider completion is missing usage")]
    MissingUsage,
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
