use super::super::{
    validate_discovery, CredentialDescriptor, CredentialReference, CredentialResolver,
    DiscoveredModel, DiscoveryContext, DynamicLoginContext, DynamicLoginCredentialResolver,
    ModelAvailability, ProviderAuthConfig, ProviderAuthMode, ProviderConnectionContract,
    ProviderConnectionInput, ProviderDiscovery, ProviderDiscoverySnapshot, ProviderError,
    ProviderFieldSchema, ProviderHealthState, ProviderInstanceConfig, ProviderProfile,
    ProviderResult,
};
#[cfg(test)]
use super::super::{DiscoveryMode, RefreshPolicy};
#[cfg(test)]
use crate::catalog::KnownProvider;
#[cfg(test)]
use crate::catalog::{CatalogKind, CurrentCatalogFile, KnownProviderCatalog, ProviderRulesCatalog};
use crate::protocol::{
    openai_responses_adapter, AdapterDescriptor, AdapterStatus, CodecRegistration, CodecRegistry,
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, ProtocolResultValue,
    ResolvedCredential, OPENAI_RESPONSES_ADAPTER_ID, OPENAI_RESPONSES_OPERATION_ID,
};
use async_trait::async_trait;
use buckyos_api::{generate_sn_user_device_token, login_sn_user_by_device_token};
use reqwest::header::ETAG;
use reqwest::{Method, Url};
#[cfg(test)]
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};

pub(crate) const SN_PROVIDER_PROFILE_ID: &str = "sn";
pub(crate) const SN_DYNAMIC_LOGIN_PROFILE_ID: &str = "device_jwt";
pub(crate) const SN_OPENAI_ADAPTER_ID: &str = "sn-openai";

const SN_MODELS_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;

#[cfg(test)]
pub(crate) fn sn_profile() -> ProviderProfile {
    let known = sn_known_provider();
    let credential: CredentialDeclaration = embedded_value(
        &known,
        "credential",
        "SN Known Provider credential declaration",
    );
    assert!(credential.required && credential.secret);
    let credential = match credential.kind.as_str() {
        "bearer" => bearer_credential_descriptor(),
        kind => panic!("SN Known Provider uses unsupported credential kind `{kind}`"),
    };
    ProviderProfile {
        provider_profile_id: SN_PROVIDER_PROFILE_ID.to_owned(),
        display_name: known.display_name,
        default_protocol_adapter_id: known.protocol_adapter_id,
        credential,
        credential_variants: Vec::new(),
        discovery_mode: DiscoveryMode::MachineApi,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

#[cfg(test)]
pub(crate) fn sn_known_provider() -> KnownProvider {
    super::builtin_catalog_document::<KnownProviderCatalog>(
        CatalogKind::KnownProvider,
        SN_PROVIDER_PROFILE_ID,
    )
    .providers
    .into_iter()
    .find(|provider| provider.provider_profile_id == SN_PROVIDER_PROFILE_ID)
    .expect("SN Known Provider catalog must contain the SN profile")
}

#[cfg(test)]
pub(crate) fn sn_connection_contract(auth_mode: ProviderAuthMode) -> ProviderConnectionContract {
    let known = sn_known_provider();
    let fields: InstanceFieldDeclarations = embedded_value(
        &known,
        "instance_fields",
        "SN Known Provider instance fields",
    );
    ProviderConnectionContract {
        default_base_url: known.base_url,
        region: fields.region,
        workspace: fields.workspace,
        account: match auth_mode {
            ProviderAuthMode::ApiKey => fields.account.api_key,
            ProviderAuthMode::DynamicLogin => fields.account.dynamic_login,
        },
        region_base_urls: known.connection.region_base_urls,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SnProviderInstanceInput<'a> {
    pub provider_instance_name: &'a str,
    pub base_url: Option<&'a str>,
    pub account: Option<&'a str>,
    pub auth: ProviderAuthConfig,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedSnProviderInstance {
    pub runtime: ProviderInstanceConfig,
    pub auth: ProviderAuthConfig,
}

pub(crate) fn resolve_sn_provider_instance_with_config(
    profile: &ProviderProfile,
    connection_contract: &ProviderConnectionContract,
    provider_rules_id: Option<String>,
    input: SnProviderInstanceInput<'_>,
) -> ProviderResult<ResolvedSnProviderInstance> {
    input.auth.validate()?;
    let mut connection_contract = connection_contract.clone();
    if input.auth.mode() == ProviderAuthMode::DynamicLogin {
        connection_contract.account = ProviderFieldSchema::required();
    }
    let connection = connection_contract.resolve(ProviderConnectionInput {
        base_url: input.base_url,
        account: input.account,
        ..ProviderConnectionInput::default()
    })?;
    let credential = input
        .auth
        .credential_reference()
        .unwrap_or_else(|| CredentialReference {
            reference: format!("sn-dynamic-login://{}", input.provider_instance_name),
        });
    Ok(ResolvedSnProviderInstance {
        runtime: ProviderInstanceConfig {
            provider_instance_name: input.provider_instance_name.to_owned(),
            provider_profile_id: profile.provider_profile_id.clone(),
            protocol_adapter_id: profile.default_protocol_adapter_id.clone(),
            base_url: connection.base_url,
            credential,
            credential_kind: input.auth.credential_kind(),
            provider_rules_id,
            region: connection.region,
            workspace: connection.workspace,
            account: connection.account,
        },
        auth: input.auth,
    })
}

#[cfg(test)]
pub(crate) fn resolve_sn_provider_instance(
    input: SnProviderInstanceInput<'_>,
) -> ProviderResult<ResolvedSnProviderInstance> {
    let profile = sn_profile();
    let connection = sn_connection_contract(input.auth.mode());
    resolve_sn_provider_instance_with_config(
        &profile,
        &connection,
        sn_known_provider().provider_rules_id,
        input,
    )
}

#[cfg(test)]
pub(crate) fn sn_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    super::builtin_catalog_document(CatalogKind::ProviderRules, SN_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn sn_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[SN_PROVIDER_PROFILE_ID])
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialDeclaration {
    kind: String,
    required: bool,
    secret: bool,
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceFieldDeclarations {
    region: ProviderFieldSchema,
    workspace: ProviderFieldSchema,
    account: AccountFieldDeclarations,
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountFieldDeclarations {
    api_key: ProviderFieldSchema,
    dynamic_login: ProviderFieldSchema,
}

#[cfg(test)]
fn sn_dynamic_login_profile() -> String {
    SN_DYNAMIC_LOGIN_PROFILE_ID.to_owned()
}

#[cfg(test)]
fn embedded_value<T: DeserializeOwned>(known: &KnownProvider, key: &str, label: &str) -> T {
    serde_json::from_value(
        known
            .ui_hints
            .get(key)
            .unwrap_or_else(|| panic!("{label} is missing"))
            .clone(),
    )
    .unwrap_or_else(|error| panic!("{label} is invalid: {error}"))
}

#[cfg(test)]
fn embedded_json<T: DeserializeOwned>(contents: &[u8], label: &str) -> T {
    serde_json::from_slice(contents).unwrap_or_else(|error| panic!("{label} is invalid: {error}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SnDialectContract {
    pub base_adapter_id: &'static str,
    pub override_points: BTreeSet<&'static str>,
    pub unsupported_api_types: BTreeSet<String>,
}

#[cfg(test)]
pub(crate) fn sn_dialect_contract() -> SnDialectContract {
    SnDialectContract {
        base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
        override_points: BTreeSet::from(["credential_resolution"]),
        unsupported_api_types: sn_provider_rules(0)
            .patterns
            .into_iter()
            .flat_map(|rule| rule.remove_api_types)
            .collect(),
    }
}

pub(crate) fn sn_openai_adapter() -> ProtocolResultValue<AdapterDescriptor> {
    let (base, _) = openai_responses_adapter();
    let responses = base
        .operations
        .get(OPENAI_RESPONSES_OPERATION_ID)
        .cloned()
        .ok_or_else(|| {
            crate::protocol::ProtocolError::invalid_configuration(
                "OpenAI Responses operation is not registered",
            )
        })?;
    Ok(AdapterDescriptor {
        protocol_family_id: base.protocol_family_id,
        protocol_adapter_id: SN_OPENAI_ADAPTER_ID.to_owned(),
        interface_generation: base.interface_generation,
        base_adapter_id: Some(OPENAI_RESPONSES_ADAPTER_ID.to_owned()),
        status: AdapterStatus::Stable,
        operations: BTreeMap::from([(responses.operation_id.clone(), responses)]),
    })
}

pub(crate) fn register_sn_openai_adapter(registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
    registry.register_derived(sn_openai_adapter()?, CodecRegistration::default())
}

#[async_trait]
trait SnModelsTransport: Send + Sync {
    async fn send(&self, request: HttpRequest) -> ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl SnModelsTransport for HttpTransport {
    async fn send(&self, request: HttpRequest) -> ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct SnDiscovery {
    transport: Arc<dyn SnModelsTransport>,
}

impl SnDiscovery {
    pub(crate) fn new(transport: HttpTransport) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn SnModelsTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl ProviderDiscovery for SnDiscovery {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_sn_context(context)?;
        let mut request =
            HttpRequest::new(Method::GET, sn_models_endpoint(&context.instance.base_url)?);
        context
            .credential
            .apply(&mut request.headers)
            .map_err(|error| ProviderError::Credential(error.to_string()))?;
        request.timeout = Some(Duration::from_secs(30));
        request.max_response_bytes = Some(SN_MODELS_RESPONSE_LIMIT);
        let response = self
            .transport
            .send(request)
            .await
            .map_err(|error| ProviderError::Discovery(error.to_string()))?;
        ensure_sn_success(&response)?;
        let header_revision = response
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        parse_sn_models(&response.body, header_revision)
    }
}

fn validate_sn_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    if context.profile.provider_profile_id != SN_PROVIDER_PROFILE_ID
        || context.profile.default_protocol_adapter_id != SN_OPENAI_ADAPTER_ID
        || context.instance.provider_profile_id != SN_PROVIDER_PROFILE_ID
        || context.instance.protocol_adapter_id != SN_OPENAI_ADAPTER_ID
    {
        return Err(ProviderError::InvalidConfiguration(
            "SN discovery requires the SN profile and sn-openai adapter".to_owned(),
        ));
    }
    if context.credential.audit().kind != CredentialKind::Bearer {
        return Err(ProviderError::Credential(
            "SN discovery requires a resolved Bearer credential".to_owned(),
        ));
    }
    if context.instance.region.is_some() {
        return Err(ProviderError::InvalidConfiguration(
            "SN profile does not accept region".to_owned(),
        ));
    }
    Ok(())
}

fn sn_models_endpoint(base_url: &str) -> ProviderResult<String> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ProviderError::InvalidConfiguration("SN base_url is invalid".to_owned()))?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ProviderError::InvalidConfiguration(
            "SN base_url must be an absolute HTTP URL".to_owned(),
        ));
    }
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/models"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn parse_sn_models(
    body: &[u8],
    header_revision: Option<String>,
) -> ProviderResult<ProviderDiscoverySnapshot> {
    let envelope: SnModelsEnvelope = serde_json::from_slice(body).map_err(|error| {
        ProviderError::Discovery(format!("SN models response is invalid: {error}"))
    })?;
    let entries = match (envelope.models, envelope.items) {
        (Some(models), None) => models,
        (None, Some(items)) => items,
        _ => {
            return Err(ProviderError::Discovery(
                "SN models response must contain exactly one of models or items".to_owned(),
            ));
        }
    };
    let mut models = entries
        .into_iter()
        .map(|entry| {
            let provider_model_id = entry
                .provider_actual_model_id
                .or(entry.provider_model_id)
                .or(entry.model)
                .or(entry.id)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ProviderError::Discovery("SN model entry has no model ID".to_owned())
                })?;
            Ok(DiscoveredModel {
                provider_model_id: provider_model_id.clone(),
                origin_model_id: Some(provider_model_id),
                api_types: Some(vec![buckyos_api::ApiType::Llm]),
                supported_features: None,
                remote_methods: Some(BTreeSet::from([OPENAI_RESPONSES_OPERATION_ID.to_owned()])),
                availability: ModelAvailability::Available,
                deprecated: false,
                pricing: None,
            })
        })
        .collect::<ProviderResult<Vec<_>>>()?;
    models.sort_by(|left, right| left.provider_model_id.cmp(&right.provider_model_id));
    let snapshot = ProviderDiscoverySnapshot {
        revision: header_revision.or(envelope.revision),
        discovered_at_ms: super::super::now_ms()?,
        health: ProviderHealthState::Healthy,
        models,
    };
    validate_discovery(&snapshot)?;
    Ok(snapshot)
}

fn ensure_sn_success(response: &HttpResponse) -> ProviderResult<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let message = serde_json::from_slice::<Value>(&response.body)
        .ok()
        .and_then(|value| {
            value
                .get("msg")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            response
                .status
                .canonical_reason()
                .unwrap_or("request failed")
                .to_owned()
        });
    Err(ProviderError::Discovery(format!(
        "SN models request failed with status {} (request {}): {message}",
        response.status, response.request_id
    )))
}

#[derive(Deserialize)]
struct SnModelsEnvelope {
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    models: Option<Vec<SnModelEntry>>,
    #[serde(default)]
    items: Option<Vec<SnModelEntry>>,
}

#[derive(Deserialize)]
struct SnModelEntry {
    #[serde(default)]
    provider_actual_model_id: Option<String>,
    #[serde(default)]
    provider_model_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Clone)]
pub(crate) struct SnCredentialBroker {
    api_key_resolver: Arc<dyn CredentialResolver>,
    dynamic_resolver: Arc<dyn DynamicLoginCredentialResolver>,
    dynamic_instances: Arc<RwLock<BTreeMap<String, ResolvedSnProviderInstance>>>,
}

impl SnCredentialBroker {
    pub(crate) fn new(
        api_key_resolver: Arc<dyn CredentialResolver>,
        dynamic_resolver: Arc<dyn DynamicLoginCredentialResolver>,
    ) -> Self {
        Self {
            api_key_resolver,
            dynamic_resolver,
            dynamic_instances: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub(crate) async fn register_dynamic_instance(
        &self,
        instance: ResolvedSnProviderInstance,
    ) -> ProviderResult<()> {
        if instance.auth.mode() != ProviderAuthMode::DynamicLogin {
            return Err(ProviderError::InvalidConfiguration(
                "only an SN dynamic_login instance may be registered".to_owned(),
            ));
        }
        let reference = instance.runtime.credential.reference.clone();
        let expected = format!(
            "sn-dynamic-login://{}",
            instance.runtime.provider_instance_name
        );
        if reference != expected {
            return Err(ProviderError::InvalidConfiguration(
                "SN dynamic credential reference does not match its instance".to_owned(),
            ));
        }
        if instance.runtime.provider_profile_id != SN_PROVIDER_PROFILE_ID
            || instance.runtime.protocol_adapter_id != SN_OPENAI_ADAPTER_ID
            || instance.runtime.account.is_none()
        {
            return Err(ProviderError::InvalidConfiguration(
                "SN dynamic login instance identity is invalid".to_owned(),
            ));
        }
        self.dynamic_resolver
            .invalidate(&instance.runtime.provider_instance_name)
            .await;
        self.dynamic_instances
            .write()
            .await
            .insert(reference, instance);
        Ok(())
    }

    pub(crate) async fn unregister_dynamic_instance(&self, provider_instance_name: &str) {
        let reference = format!("sn-dynamic-login://{provider_instance_name}");
        self.dynamic_instances.write().await.remove(&reference);
        self.dynamic_resolver
            .invalidate(provider_instance_name)
            .await;
    }

    pub(crate) async fn resolve(
        &self,
        instance: &ResolvedSnProviderInstance,
    ) -> ProviderResult<ResolvedCredential> {
        match &instance.auth {
            ProviderAuthConfig::ApiKey { credential_ref, .. } => {
                self.api_key_resolver
                    .resolve(
                        &bearer_credential_descriptor(),
                        &CredentialReference {
                            reference: credential_ref.clone(),
                        },
                    )
                    .await
            }
            ProviderAuthConfig::DynamicLogin { .. } => {
                let user_name = instance.runtime.account.as_deref().ok_or_else(|| {
                    ProviderError::InvalidConfiguration(
                        "SN dynamic login requires an account".to_owned(),
                    )
                })?;
                let context = instance.auth.dynamic_login_context(
                    instance.runtime.provider_instance_name.clone(),
                    user_name,
                )?;
                self.dynamic_resolver.resolve_dynamic(&context).await
            }
        }
    }

    pub(crate) async fn invalidate(&self, provider_instance_name: &str) {
        self.dynamic_resolver
            .invalidate(provider_instance_name)
            .await;
    }
}

#[async_trait]
impl CredentialResolver for SnCredentialBroker {
    async fn resolve(
        &self,
        descriptor: &CredentialDescriptor,
        reference: &CredentialReference,
    ) -> ProviderResult<ResolvedCredential> {
        let instance = self
            .dynamic_instances
            .read()
            .await
            .get(&reference.reference)
            .cloned();
        if let Some(instance) = instance {
            if descriptor != &bearer_credential_descriptor() {
                return Err(ProviderError::Credential(
                    "SN dynamic login resolves only Bearer credentials".to_owned(),
                ));
            }
            return self.resolve(&instance).await;
        }
        self.api_key_resolver.resolve(descriptor, reference).await
    }
}

fn bearer_credential_descriptor() -> CredentialDescriptor {
    CredentialDescriptor {
        kind: CredentialKind::Bearer,
        header_name: None,
    }
}

#[derive(Clone)]
pub(crate) struct SnDynamicLoginResolver {
    client: Arc<dyn SnLoginClient>,
    clock: Arc<dyn SnClock>,
    supported_login_profile: String,
    slots: CredentialSlots,
}

type CredentialSlot = Arc<Mutex<Option<CachedCredential>>>;
type CredentialSlots = Arc<Mutex<BTreeMap<String, CredentialSlot>>>;

impl SnDynamicLoginResolver {
    pub(crate) fn new(client: reqwest::Client, supported_login_profile: String) -> Self {
        Self::with_dependencies(
            Arc::new(SystemSnLoginClient { client }),
            Arc::new(SystemSnClock),
            supported_login_profile,
        )
    }

    fn with_dependencies(
        client: Arc<dyn SnLoginClient>,
        clock: Arc<dyn SnClock>,
        supported_login_profile: String,
    ) -> Self {
        Self {
            client,
            clock,
            supported_login_profile,
            slots: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    async fn slot(&self, key: &str) -> CredentialSlot {
        let mut slots = self.slots.lock().await;
        slots
            .entry(key.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    }
}

#[async_trait]
impl DynamicLoginCredentialResolver for SnDynamicLoginResolver {
    async fn resolve_dynamic(
        &self,
        context: &DynamicLoginContext,
    ) -> ProviderResult<ResolvedCredential> {
        if context.login_profile != self.supported_login_profile {
            return Err(ProviderError::InvalidConfiguration(format!(
                "SN dynamic login only supports login_profile={}",
                self.supported_login_profile
            )));
        }
        let slot = self.slot(context.cache_key()).await;
        let mut cached = slot.lock().await;
        let now = self.clock.now_epoch_seconds()?;
        if let Some(current) = cached.as_ref().filter(|entry| now < entry.refresh_at) {
            return Ok(current.credential.clone());
        }
        let session = self.client.login(context).await.map_err(|error| {
            ProviderError::Credential(format!(
                "SN dynamic login failed ({})",
                if error.retryable {
                    "retryable"
                } else {
                    "fatal"
                }
            ))
        })?;
        if session.session_token.trim().is_empty() || session.expires_in_seconds == 0 {
            return Err(ProviderError::Credential(
                "SN dynamic login returned an invalid session".to_owned(),
            ));
        }
        let refresh_skew = (session.expires_in_seconds / 5).min(60);
        let refresh_at = now
            .checked_add(session.expires_in_seconds.saturating_sub(refresh_skew))
            .ok_or_else(|| ProviderError::Credential("SN token expiry overflow".to_owned()))?;
        let credential = ResolvedCredential::bearer(
            &format!("sn-dynamic-login://{}", context.cache_key()),
            session.session_token,
        )
        .map_err(|error| ProviderError::Credential(error.to_string()))?;
        *cached = Some(CachedCredential {
            credential: credential.clone(),
            refresh_at,
        });
        Ok(credential)
    }

    async fn invalidate(&self, provider_instance_name: &str) {
        let slot = self.slots.lock().await.get(provider_instance_name).cloned();
        if let Some(slot) = slot {
            *slot.lock().await = None;
        }
    }
}

struct CachedCredential {
    credential: ResolvedCredential,
    refresh_at: u64,
}

struct SnLoginSession {
    session_token: String,
    expires_in_seconds: u64,
}

struct SnLoginError {
    retryable: bool,
}

#[async_trait]
trait SnLoginClient: Send + Sync {
    async fn login(&self, context: &DynamicLoginContext) -> Result<SnLoginSession, SnLoginError>;
}

struct SystemSnLoginClient {
    client: reqwest::Client,
}

#[async_trait]
impl SnLoginClient for SystemSnLoginClient {
    async fn login(&self, context: &DynamicLoginContext) -> Result<SnLoginSession, SnLoginError> {
        let device_token =
            generate_sn_user_device_token(&context.user_name).map_err(|error| SnLoginError {
                retryable: error.is_retryable(),
            })?;
        let session =
            login_sn_user_by_device_token(&self.client, &context.login_endpoint, &device_token)
                .await
                .map_err(|error| SnLoginError {
                    retryable: error.is_retryable(),
                })?;
        Ok(SnLoginSession {
            session_token: session.session_token,
            expires_in_seconds: session.expires_in,
        })
    }
}

trait SnClock: Send + Sync {
    fn now_epoch_seconds(&self) -> ProviderResult<u64>;
}

struct SystemSnClock;

impl SnClock for SystemSnClock {
    fn now_epoch_seconds(&self) -> ProviderResult<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(|_| ProviderError::Credential("system time precedes Unix epoch".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        CatalogBuildOptions, CatalogDocuments, CatalogSnapshot, KnownProviderCatalog,
        ModelDriverCatalog,
    };
    use crate::protocol::{HttpBody, ProtocolError, OPENAI_RESPONSES_OPERATION_ID};
    use crate::provider::{InventoryBuilder, StaticCredentialResolver};
    use crate::settings::{MetadataFile, MetadataSource, MetadataSources};
    use bytes::Bytes;
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
    use reqwest::StatusCode;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    struct FakeModelsTransport {
        response: StdMutex<Option<Result<HttpResponse, ProtocolError>>>,
        request: StdMutex<Option<HttpRequest>>,
    }

    impl FakeModelsTransport {
        fn response(status: StatusCode, headers: HeaderMap, body: Value) -> Arc<Self> {
            Arc::new(Self {
                response: StdMutex::new(Some(Ok(HttpResponse {
                    status,
                    headers,
                    body: Bytes::from(serde_json::to_vec(&body).unwrap()),
                    request_id: "sn-request-1".to_owned(),
                    retry_after: None,
                }))),
                request: StdMutex::new(None),
            })
        }
    }

    #[async_trait]
    impl SnModelsTransport for FakeModelsTransport {
        async fn send(&self, request: HttpRequest) -> ProtocolResultValue<HttpResponse> {
            *self.request.lock().unwrap() = Some(request);
            self.response.lock().unwrap().take().unwrap()
        }
    }

    struct FakeClock(AtomicU64);

    impl FakeClock {
        fn set(&self, now: u64) {
            self.0.store(now, Ordering::Release);
        }
    }

    impl SnClock for FakeClock {
        fn now_epoch_seconds(&self) -> ProviderResult<u64> {
            Ok(self.0.load(Ordering::Acquire))
        }
    }

    struct FakeLoginClient {
        calls: AtomicUsize,
        fail: bool,
    }

    #[async_trait]
    impl SnLoginClient for FakeLoginClient {
        async fn login(
            &self,
            _context: &DynamicLoginContext,
        ) -> Result<SnLoginSession, SnLoginError> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
            if self.fail {
                return Err(SnLoginError { retryable: false });
            }
            tokio::task::yield_now().await;
            Ok(SnLoginSession {
                session_token: format!("dynamic-secret-{call}"),
                expires_in_seconds: 100,
            })
        }
    }

    fn dynamic_instance() -> ResolvedSnProviderInstance {
        resolve_sn_provider_instance(SnProviderInstanceInput {
            provider_instance_name: "sn-main",
            base_url: None,
            account: Some("alice"),
            auth: ProviderAuthConfig::DynamicLogin {
                login_profile: sn_dynamic_login_profile(),
                login_endpoint: "https://sn.buckyos.ai/api/user/login_by_device_token".to_owned(),
            },
        })
        .unwrap()
    }

    #[test]
    fn profile_rules_auth_and_connection_are_stable() {
        let profile = sn_profile();
        let known = sn_known_provider();
        let rules = sn_provider_rules(9);
        assert_eq!(profile.provider_profile_id, "sn");
        assert_eq!(profile.default_protocol_adapter_id, "sn-openai");
        assert_eq!(known.display_name, "BuckyOS SN");
        assert_eq!(known.base_url, "https://sn.buckyos.ai/api/v1/ai");
        assert_eq!(rules.metadata_drivers, None);
        assert_eq!(
            rules.patterns[0].operations["llm"],
            OPENAI_RESPONSES_OPERATION_ID
        );
        assert_eq!(
            rules.patterns[0].remove_api_types,
            BTreeSet::from(["image.img2img".to_owned(), "image.txt2img".to_owned()])
        );
        assert_eq!(sn_dynamic_login_profile(), SN_DYNAMIC_LOGIN_PROFILE_ID);

        let dynamic = dynamic_instance();
        assert_eq!(dynamic.runtime.base_url, known.base_url);
        assert_eq!(dynamic.runtime.account.as_deref(), Some("alice"));
        assert_eq!(
            dynamic.runtime.credential.reference,
            "sn-dynamic-login://sn-main"
        );
        assert!(resolve_sn_provider_instance(SnProviderInstanceInput {
            provider_instance_name: "sn-invalid",
            base_url: None,
            account: None,
            auth: dynamic.auth,
        })
        .is_err());
    }

    #[test]
    fn wp15_loads_sn_provider_catalogs_without_an_sn_model_driver() {
        let builtin = sn_catalog_files()
            .into_iter()
            .map(|file| MetadataFile::parse(MetadataSource::Builtin, file.kind, file.contents))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let catalog = MetadataSources {
            builtin,
            ..MetadataSources::default()
        }
        .build_snapshot(1, &CatalogBuildOptions::default())
        .unwrap();

        assert_eq!(
            catalog
                .known_provider(SN_PROVIDER_PROFILE_ID)
                .unwrap()
                .display_name,
            "BuckyOS SN"
        );
        assert_eq!(
            catalog
                .provider_rules(SN_PROVIDER_PROFILE_ID)
                .unwrap()
                .revision_seq,
            1
        );
        assert!(catalog.model_driver(SN_PROVIDER_PROFILE_ID).is_none());
    }

    #[test]
    fn derived_adapter_delegates_without_mutating_openai() {
        let (base_descriptor, base_codecs) = openai_responses_adapter();
        let original_base = base_descriptor.clone();
        let mut registry = CodecRegistry::default();
        registry
            .register_codecs(base_descriptor, base_codecs)
            .unwrap();
        let base_codec = registry
            .codec(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                buckyos_api::ApiType::Llm,
            )
            .unwrap();
        register_sn_openai_adapter(&mut registry).unwrap();
        let sn_descriptor = registry.adapter(SN_OPENAI_ADAPTER_ID).unwrap();
        assert_eq!(
            sn_descriptor.base_adapter_id.as_deref(),
            Some(OPENAI_RESPONSES_ADAPTER_ID)
        );
        assert_eq!(
            sn_dialect_contract().override_points,
            BTreeSet::from(["credential_resolution"])
        );
        assert_eq!(
            sn_dialect_contract().unsupported_api_types,
            BTreeSet::from(["image.img2img".to_owned(), "image.txt2img".to_owned()])
        );
        let derived_codec = registry
            .codec(
                SN_OPENAI_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                buckyos_api::ApiType::Llm,
            )
            .unwrap();
        assert!(Arc::ptr_eq(&base_codec, &derived_codec));
        assert_eq!(
            registry.adapter(OPENAI_RESPONSES_ADAPTER_ID),
            Some(&original_base)
        );
    }

    #[tokio::test]
    async fn discovery_reads_sn_inventory_with_resolved_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(ETAG, HeaderValue::from_static("sn-models-v2"));
        let transport = FakeModelsTransport::response(
            StatusCode::OK,
            headers,
            json!({
                "models": [
                    {"provider_model_id": "alias", "provider_actual_model_id": "gpt-5"},
                    {"provider_model_id": "gpt-4.1"}
                ]
            }),
        );
        let discovery = SnDiscovery::with_transport(transport.clone());
        let profile = sn_profile();
        let instance = dynamic_instance().runtime;
        let credential =
            ResolvedCredential::bearer("sn-dynamic-login://sn-main", "secret").unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();
        assert_eq!(snapshot.revision.as_deref(), Some("sn-models-v2"));
        assert_eq!(snapshot.models[0].provider_model_id, "gpt-4.1");
        assert_eq!(snapshot.models[1].provider_model_id, "gpt-5");
        assert_eq!(
            snapshot.models[0].api_types,
            Some(vec![buckyos_api::ApiType::Llm])
        );
        let request = transport.request.lock().unwrap();
        let request = request.as_ref().unwrap();
        assert_eq!(request.method, Method::GET);
        assert_eq!(request.url, "https://sn.buckyos.ai/api/v1/ai/models");
        assert!(matches!(request.body, HttpBody::Empty));
        assert_eq!(request.headers[AUTHORIZATION], "Bearer secret");
        assert!(!format!("{request:?}").contains("secret"));
    }

    #[tokio::test]
    async fn discovery_rejects_invalid_inventory_and_reports_bounded_errors() {
        let invalid = FakeModelsTransport::response(
            StatusCode::OK,
            HeaderMap::new(),
            json!({"models": [], "items": []}),
        );
        let discovery = SnDiscovery::with_transport(invalid);
        let profile = sn_profile();
        let instance = dynamic_instance().runtime;
        let credential =
            ResolvedCredential::bearer("sn-dynamic-login://sn-main", "secret").unwrap();
        let error = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exactly one of models or items"));

        let denied = FakeModelsTransport::response(
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            json!({"msg": "SN session rejected"}),
        );
        let discovery = SnDiscovery::with_transport(denied);
        let error = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("status 401"));
        assert!(error.to_string().contains("request sn-request-1"));
        assert!(error.to_string().contains("SN session rejected"));
        assert!(!error.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn dynamic_login_is_single_flight_cached_refreshed_and_invalidated() {
        let client = Arc::new(FakeLoginClient {
            calls: AtomicUsize::new(0),
            fail: false,
        });
        let clock = Arc::new(FakeClock(AtomicU64::new(1_000)));
        let resolver = Arc::new(SnDynamicLoginResolver::with_dependencies(
            client.clone(),
            clock.clone(),
            sn_dynamic_login_profile(),
        ));
        let context = dynamic_instance()
            .auth
            .dynamic_login_context("sn-main", "alice")
            .unwrap();
        let (first, second) = tokio::join!(
            resolver.resolve_dynamic(&context),
            resolver.resolve_dynamic(&context)
        );
        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(client.calls.load(Ordering::Acquire), 1);

        clock.set(1_079);
        resolver.resolve_dynamic(&context).await.unwrap();
        assert_eq!(client.calls.load(Ordering::Acquire), 1);
        clock.set(1_080);
        resolver.resolve_dynamic(&context).await.unwrap();
        assert_eq!(client.calls.load(Ordering::Acquire), 2);
        resolver.invalidate("sn-main").await;
        resolver.resolve_dynamic(&context).await.unwrap();
        assert_eq!(client.calls.load(Ordering::Acquire), 3);
        assert!(
            !format!("{:?}", resolver.resolve_dynamic(&context).await.unwrap())
                .contains("dynamic-secret")
        );
    }

    #[tokio::test]
    async fn credential_broker_keeps_modes_explicit_and_redacts_login_failure() {
        let static_resolver = Arc::new(StaticCredentialResolver::new(BTreeMap::from([(
            "secret://sn/api-key".to_owned(),
            "static-secret".to_owned(),
        )])));
        let failing = Arc::new(SnDynamicLoginResolver::with_dependencies(
            Arc::new(FakeLoginClient {
                calls: AtomicUsize::new(0),
                fail: true,
            }),
            Arc::new(FakeClock(AtomicU64::new(1_000))),
            sn_dynamic_login_profile(),
        ));
        let broker = SnCredentialBroker::new(static_resolver, failing);
        let api_key = resolve_sn_provider_instance(SnProviderInstanceInput {
            provider_instance_name: "sn-key",
            base_url: None,
            account: None,
            auth: ProviderAuthConfig::ApiKey {
                credential_ref: "secret://sn/api-key".to_owned(),
                credential_kind: None,
            },
        })
        .unwrap();
        let credential = broker.resolve(&api_key).await.unwrap();
        assert_eq!(credential.audit().kind, CredentialKind::Bearer);
        assert!(!format!("{credential:?}").contains("static-secret"));

        let dynamic = dynamic_instance();
        broker
            .register_dynamic_instance(dynamic.clone())
            .await
            .unwrap();
        let error = CredentialResolver::resolve(
            &broker,
            &bearer_credential_descriptor(),
            &dynamic.runtime.credential,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ProviderError::Credential(_)));
        assert_eq!(
            error.to_string(),
            "credential resolution failed: SN dynamic login failed (fatal)"
        );
        assert!(!error.to_string().contains("secret"));
        broker.unregister_dynamic_instance("sn-main").await;
        assert!(broker.dynamic_instances.read().await.is_empty());
    }

    #[test]
    fn rules_and_discovery_build_llm_inventory() {
        let model_driver: ModelDriverCatalog = serde_json::from_value(json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": "openai",
            "revision_seq": 1,
            "models": [{"id": "gpt-5", "api_types": ["llm", "image.txt2img"]}],
            "patterns": [],
            "defaults": {},
            "variants": [],
            "version_rules": []
        }))
        .unwrap();
        let catalog = CatalogSnapshot::build(
            1,
            CatalogDocuments {
                model_drivers: vec![model_driver],
                provider_rules: vec![sn_provider_rules(1)],
                known_providers: vec![KnownProviderCatalog {
                    format: "buckyos.aicc.known-provider-catalog".to_owned(),
                    schema_version: 1,
                    schema_revision: 0,
                    revision_seq: 1,
                    catalog_id: "builtin".to_owned(),
                    providers: vec![sn_known_provider()],
                }],
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        let (base, codecs) = openai_responses_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(base, codecs).unwrap();
        register_sn_openai_adapter(&mut registry).unwrap();
        let instance = dynamic_instance().runtime;
        let inventory = InventoryBuilder::build(
            &sn_profile(),
            &instance,
            ProviderDiscoverySnapshot {
                revision: Some("sn-v1".to_owned()),
                discovered_at_ms: 1,
                health: ProviderHealthState::Healthy,
                models: vec![DiscoveredModel {
                    provider_model_id: "gpt-5".to_owned(),
                    origin_model_id: Some("gpt-5".to_owned()),
                    api_types: Some(vec![buckyos_api::ApiType::Llm]),
                    supported_features: None,
                    remote_methods: Some(BTreeSet::from(
                        [OPENAI_RESPONSES_OPERATION_ID.to_owned()],
                    )),
                    availability: ModelAvailability::Available,
                    deprecated: false,
                    pricing: None,
                }],
            },
            &catalog,
            &registry,
        )
        .unwrap();
        assert_eq!(inventory.provider_profile_id, SN_PROVIDER_PROFILE_ID);
        assert_eq!(inventory.protocol_adapter_id, SN_OPENAI_ADAPTER_ID);
        assert_eq!(inventory.models.len(), 1);
        assert_eq!(
            inventory.models[0].api_types,
            vec![buckyos_api::ApiType::Llm]
        );
        assert_eq!(
            inventory.models[0].operations["llm"],
            OPENAI_RESPONSES_OPERATION_ID
        );
    }
}
