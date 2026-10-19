use super::super::{
    validate_discovery, DiscoveredModel, DiscoveryContext, ModelAvailability, ProviderDiscovery,
    ProviderDiscoverySnapshot, ProviderError, ProviderHealthState, ProviderResult,
};
#[cfg(test)]
use super::super::{DiscoveryMode, ProviderProfile};
use crate::catalog::{
    CatalogSnapshot, ModelIdentity, ModelMatchFailure, Pricing, ProviderModelMatch,
};
#[cfg(test)]
use crate::catalog::{CurrentCatalogFile, ProviderRulesCatalog};
use crate::protocol::openrouter_decisions::{
    DECISION_FEATURES, JEV_ALIAS_ID, JEV_BUILD_ID, JEV_CHANNEL_ID, JEV_ORIGIN_ID,
    OPENROUTER_DECISIONS_OPERATION_ID,
};
use crate::protocol::OPENAI_CHAT_COMPLETIONS_OPERATION_ID;
use crate::protocol::{
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, OPENAI_AUDIO_SPEECH_OPERATION_ID,
    OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID, OPENAI_EMBEDDINGS_OPERATION_ID,
    OPENAI_RESPONSES_OPERATION_ID, OPENROUTER_IMAGES_OPERATION_ID, OPENROUTER_MUSIC_OPERATION_ID,
    OPENROUTER_RERANK_OPERATION_ID, OPENROUTER_RESPONSES_ADAPTER_ID,
    OPENROUTER_VIDEOS_OPERATION_ID,
};
use async_trait::async_trait;
use buckyos_api::{features, ApiType};
use reqwest::header::ETAG;
use reqwest::{Method, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const OPENROUTER_PROVIDER_PROFILE_ID: &str = "openrouter";

const MODELS_RESPONSE_LIMIT: usize = 16 * 1024 * 1024;
const VIDEO_PRICING_SOURCE: &str = "https://openrouter.ai/api/v1/videos/models";

#[cfg(test)]
pub(crate) fn openrouter_profile() -> ProviderProfile {
    super::builtin_profile(OPENROUTER_PROVIDER_PROFILE_ID, DiscoveryMode::MachineApi)
}

#[cfg(test)]
pub(crate) fn openrouter_known_provider() -> crate::catalog::KnownProvider {
    super::builtin_known_provider(OPENROUTER_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn openrouter_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    super::builtin_provider_rules(OPENROUTER_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn openrouter_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[OPENROUTER_PROVIDER_PROFILE_ID])
}

#[async_trait]
trait OpenRouterModelsTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl OpenRouterModelsTransport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct OpenRouterDiscovery {
    transport: Arc<dyn OpenRouterModelsTransport>,
}

impl OpenRouterDiscovery {
    pub(crate) fn new(transport: HttpTransport) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn OpenRouterModelsTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl ProviderDiscovery for OpenRouterDiscovery {
    fn match_model_driver(&self, id: &str, catalog: &CatalogSnapshot) -> ProviderModelMatch {
        if id == JEV_CHANNEL_ID {
            return ProviderModelMatch::Matched(ModelIdentity {
                model_driver_id: "typesafe".into(),
                model_id: JEV_ORIGIN_ID.into(),
            });
        }
        if id.starts_with("typesafe/") || id.starts_with("~typesafe/") {
            return ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias);
        }
        let Some((vendor, model)) = id.split_once('/') else {
            return ProviderModelMatch::NotHandled;
        };
        let vendor = vendor.to_ascii_lowercase();
        let driver = match vendor.as_str() {
            "anthropic" => "claude",
            "google" => "gemini",
            "moonshotai" => "kimi",
            "z-ai" => "glm",
            other => other,
        };
        if catalog.resolve_model(driver, model).is_ok() {
            return ProviderModelMatch::Matched(ModelIdentity {
                model_driver_id: driver.into(),
                model_id: model.into(),
            });
        }
        let matches = catalog
            .model_driver(driver)
            .into_iter()
            .flat_map(|driver| &driver.models)
            .filter(|entry| entry.id.eq_ignore_ascii_case(model))
            .map(|entry| ModelIdentity {
                model_driver_id: driver.into(),
                model_id: entry.id.clone(),
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [identity] => ProviderModelMatch::Matched(identity.clone()),
            [] => ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias),
            _ => ProviderModelMatch::Failed(ModelMatchFailure::Ambiguous {
                candidates: matches,
            }),
        }
    }

    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_context(context)?;
        let response = self
            .get(context, models_endpoint(&context.instance.base_url)?)
            .await?;
        let wire: ModelsResponse = serde_json::from_slice(&response.body).map_err(|error| {
            ProviderError::DiscoveryResponse(format!(
                "OpenRouter models response is invalid: {error}"
            ))
        })?;
        let mut models = BTreeMap::new();
        let canonical_slugs = wire
            .data
            .iter()
            .map(|model| (model.id.clone(), model.canonical_slug.clone()))
            .collect::<BTreeMap<_, _>>();
        let verified_target = wire
            .data
            .iter()
            .find(|m| m.id == JEV_CHANNEL_ID)
            .is_some_and(|m| {
                m.canonical_slug.as_deref() == Some(JEV_BUILD_ID)
                    && m.context_length == Some(32000)
                    && m.architecture.as_ref().is_some_and(|a| {
                        a.output_modalities == ["decisions"] && a.input_modalities == ["text"]
                    })
            });
        for model in wire.data {
            if is_logical_model_id(&model.id) {
                continue;
            }
            let mut supported_features = BTreeSet::new();
            if model
                .supported_parameters
                .iter()
                .any(|parameter| parameter == "tools")
            {
                supported_features.insert(features::TOOL_CALL.to_owned());
            }
            if model
                .supported_parameters
                .iter()
                .any(|parameter| parameter == "response_format")
            {
                supported_features.insert(features::JSON_SCHEMA.to_owned());
            }
            if model.architecture.as_ref().is_some_and(|architecture| {
                architecture
                    .input_modalities
                    .iter()
                    .any(|item| item == "image")
            }) {
                supported_features.insert(features::VISION.to_owned());
            }
            let (api_types, remote_methods) = classify_modalities(model.architecture.as_ref());
            if api_types.iter().any(|api_type| {
                matches!(
                    api_type,
                    ApiType::ImageTextToImage | ApiType::ImageImageToImage
                )
            }) {
                supported_features.insert(features::IMAGE_GENERATION.to_owned());
            }
            let decision = api_types == [ApiType::Decision];
            let video = api_types == [ApiType::VideoGenerate];
            let mut availability = ModelAvailability::Available;
            if decision {
                supported_features = DECISION_FEATURES.into_iter().map(str::to_owned).collect();
                let identity_verified = match model.id.as_str() {
                    JEV_CHANNEL_ID => model.canonical_slug.as_deref() == Some(JEV_BUILD_ID),
                    JEV_ALIAS_ID => {
                        verified_target
                            && model
                                .alias_target
                                .as_ref()
                                .is_some_and(|target| target.slug == JEV_CHANNEL_ID)
                    }
                    _ => false,
                };
                if !identity_verified
                    || model.context_length != Some(32000)
                    || !model
                        .architecture
                        .as_ref()
                        .is_some_and(|a| a.input_modalities == ["text"])
                {
                    availability = ModelAvailability::Unavailable;
                    log::warn!("OpenRouter decision model {} unavailable: unverified build/alias target or channel context/input modalities", model.id);
                }
            }
            if models.contains_key(&model.id) {
                return Err(ProviderError::DiscoveryResponse(
                    "duplicate OpenRouter model ID".into(),
                ));
            }
            models.insert(
                model.id.clone(),
                DiscoveredModel {
                    provider_model_id: model.id,

                    api_types: Some(api_types),
                    supported_features: Some(supported_features),
                    unsupported_features: BTreeSet::new(),
                    remote_methods: Some(remote_methods),
                    availability,
                    deprecated: model.expiration_date.is_some(),
                    // Video jobs are priced by `/videos/models`, not per token.
                    pricing: if video {
                        None
                    } else {
                        parse_pricing(model.pricing)?
                    },
                    video: None,
                },
            );
        }
        if models
            .values()
            .any(|model| model.api_types.as_deref() == Some(&[ApiType::VideoGenerate]))
        {
            self.attach_video_catalog(context, &canonical_slugs, &mut models)
                .await?;
        }
        let models = models.into_values().collect::<Vec<_>>();
        let revision = response
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .or_else(|| Some(models_revision(&models)));
        let snapshot = ProviderDiscoverySnapshot {
            revision,
            discovered_at_ms: super::super::now_ms()?,
            health: ProviderHealthState::Healthy,
            models,
        };
        validate_discovery(&snapshot)?;
        Ok(snapshot)
    }
}

impl OpenRouterDiscovery {
    async fn get(
        &self,
        context: &DiscoveryContext<'_>,
        url: String,
    ) -> ProviderResult<HttpResponse> {
        let mut request = HttpRequest::new(Method::GET, url);
        context
            .credential
            .apply(&mut request.headers)
            .map_err(|error| ProviderError::Credential(error.to_string()))?;
        request.timeout = Some(Duration::from_secs(30));
        request.max_response_bytes = Some(MODELS_RESPONSE_LIMIT);
        let response = self
            .transport
            .send(request)
            .await
            .map_err(|error| ProviderError::Discovery(error.to_string()))?;
        ensure_success(&response)?;
        Ok(response)
    }

    /// Video models become routable only with the channel's own video
    /// catalog entry: its output facts form the channel contract and its SKUs
    /// the price. A model the catalog lacks, or whose canonical slug differs,
    /// stays listed but unavailable.
    async fn attach_video_catalog(
        &self,
        context: &DiscoveryContext<'_>,
        canonical_slugs: &BTreeMap<String, Option<String>>,
        models: &mut BTreeMap<String, DiscoveredModel>,
    ) -> ProviderResult<()> {
        let catalog = match self
            .get(
                context,
                api_endpoint(&context.instance.base_url, "videos/models")?,
            )
            .await
        {
            Ok(response) => serde_json::from_slice::<VideoModelsResponse>(&response.body)
                .map_err(|error| {
                    ProviderError::DiscoveryResponse(format!(
                        "OpenRouter video models response is invalid: {error}"
                    ))
                })?
                .data
                .into_iter()
                .map(|model| (model.id.clone(), model))
                .collect::<BTreeMap<_, _>>(),
            Err(error) => {
                log::warn!("OpenRouter video catalog is unavailable: {error}");
                BTreeMap::new()
            }
        };
        for (id, model) in models.iter_mut() {
            if model.api_types.as_deref() != Some(&[ApiType::VideoGenerate]) {
                continue;
            }
            let Some(entry) = catalog.get(id) else {
                log::warn!("OpenRouter video model {id} is missing from /videos/models");
                model.availability = ModelAvailability::Unavailable;
                continue;
            };
            let listed_slug = canonical_slugs.get(id).cloned().flatten();
            if listed_slug.is_some() && entry.canonical_slug != listed_slug {
                log::warn!(
                    "OpenRouter video model {id} canonical slug differs between catalogs: {:?} vs {:?}",
                    listed_slug,
                    entry.canonical_slug
                );
                model.availability = ModelAvailability::Unavailable;
                continue;
            }
            model.video = Some(crate::video::DiscoveredVideoFacts {
                durations: entry
                    .supported_durations
                    .iter()
                    .flatten()
                    .map(|seconds| *seconds as f64)
                    .collect(),
                resolutions: entry.supported_resolutions.clone().unwrap_or_default(),
                aspect_ratios: entry.supported_aspect_ratios.clone().unwrap_or_default(),
                sizes: entry.supported_sizes.clone().unwrap_or_default(),
                keyframes: entry
                    .supported_frame_images
                    .iter()
                    .flatten()
                    .filter_map(|frame| match frame.as_str() {
                        "first_frame" => Some(buckyos_api::KeyframePosition::Start),
                        "last_frame" => Some(buckyos_api::KeyframePosition::End),
                        _ => None,
                    })
                    .collect(),
                generate_audio: entry.generate_audio,
                seed: entry.seed,
            });
            model.pricing = entry
                .pricing_skus
                .as_ref()
                .map(|skus| parse_video_pricing(id, skus))
                .transpose()?
                .flatten();
        }
        Ok(())
    }
}

/// `duration_seconds` is a per-second price, `duration_seconds_<resolution>`
/// its per-resolution override and `reference_images` a per-image add-on.
/// Any other SKU (token-metered, audio variants, ...) leaves the price
/// unknown rather than undercounted.
fn parse_video_pricing(
    model: &str,
    skus: &BTreeMap<String, String>,
) -> ProviderResult<Option<Pricing>> {
    let mut base = None;
    let mut rules = Vec::new();
    let mut addons = Vec::new();
    for (sku, price) in skus {
        let amount = parse_nonnegative_price(sku, Some(price))?;
        let Some(amount) = amount else {
            return Ok(None);
        };
        if sku == "duration_seconds" {
            base = Some(amount);
        } else if let Some(resolution) = sku.strip_prefix("duration_seconds_") {
            rules.push(crate::catalog::PricingRule {
                when: crate::matching::MatchRule::Object(BTreeMap::from([(
                    "/video/resolution".to_owned(),
                    serde_json::Value::String(resolution.to_owned()),
                )])),
                amount,
                unit: Some(crate::catalog::PricingUnit::VideoSecond),
                input_token: None,
                output_token: None,
                cache_input_token: None,
                cache_write_input_token: None,
                cache_write_1h_input_token: None,
                audio_input_token: None,
                image_input_token: None,
                audio_output_token: None,
                image_output_token: None,
            });
        } else if sku == "reference_images" {
            addons.push(crate::catalog::PricingAddon {
                unit: crate::catalog::PricingUnit::ReferenceImage,
                amount,
            });
        } else {
            log::warn!("OpenRouter video model {model} has unsupported price SKU {sku}; price stays unknown");
            return Ok(None);
        }
    }
    if base.is_none() && rules.is_empty() {
        return Ok(None);
    }
    Ok(Some(Pricing {
        currency: "USD".to_owned(),
        source_url: Some(VIDEO_PRICING_SOURCE.to_owned()),
        verified_at: Some(super::super::now_ms()?.to_string()),
        ratio_exception: None,
        cache_write_input_token: None,
        audio_input_token: None,
        image_input_token: None,
        cache_write_1h_input_token: None,
        audio_output_token: None,
        image_output_token: None,
        input_token: None,
        output_token: None,
        cache_input_token: None,
        estimated_cost: None,
        unit: Some(crate::catalog::PricingUnit::VideoSecond),
        amount: base,
        rules,
        tiers: None,
        time_windows: Vec::new(),
        addons,
    }))
}

fn parse_pricing(pricing: Option<ModelPricing>) -> ProviderResult<Option<Pricing>> {
    let Some(pricing) = pricing else {
        return Ok(None);
    };
    let input_token = parse_nonnegative_price("prompt", pricing.prompt.as_deref())?;
    let output_token = parse_nonnegative_price("completion", pricing.completion.as_deref())?;
    let cache_input_token =
        parse_nonnegative_price("input_cache_read", pricing.input_cache_read.as_deref())?;
    let cache_write_input_token =
        parse_nonnegative_price("input_cache_write", pricing.input_cache_write.as_deref())?;
    let cache_write_1h_input_token = parse_nonnegative_price(
        "input_cache_write_1h",
        pricing.input_cache_write_1h.as_deref(),
    )?;
    let audio_input_token = parse_nonnegative_price("audio", pricing.audio.as_deref())?;
    let audio_output_token =
        parse_nonnegative_price("audio_output", pricing.audio_output.as_deref())?;
    // OpenRouter publishes `"request": "0"` next to token rates in its own catalog examples.
    // A zero fee is not unit billing, so it must not switch the entry to `PricingUnit::Request`.
    let amount = parse_nonnegative_price("request", pricing.request.as_deref())?
        .filter(|value| *value > 0.0);
    if input_token.is_none()
        && output_token.is_none()
        && cache_input_token.is_none()
        && cache_write_input_token.is_none()
        && cache_write_1h_input_token.is_none()
        && audio_input_token.is_none()
        && audio_output_token.is_none()
        && amount.is_none()
    {
        return Ok(None);
    }
    let mut ratio_exception = input_token.filter(|value| *value > 0.0).and_then(|input| {
        let anomalous = cache_input_token.is_some_and(|value| value > input)
            || cache_write_input_token.is_some_and(|value| value / input > 10.0)
            || cache_write_1h_input_token.is_some_and(|value| value / input > 10.0)
            || output_token.is_some_and(|value| value / input > 100.0);
        anomalous
            .then(|| "OpenRouter official model catalog reports this token price ratio".to_owned())
    });
    // `Pricing` cannot express a per-request fee together with token rates: catalog validation
    // rejects that combination and `discover()` validates the whole snapshot, so keeping both
    // would let one model block the entire provider catalog. Token rates stay authoritative and
    // the omitted fee is recorded so the catalog snapshot still explains the discrepancy.
    let has_token_rates = input_token.is_some()
        || output_token.is_some()
        || cache_input_token.is_some()
        || cache_write_input_token.is_some()
        || cache_write_1h_input_token.is_some()
        || audio_input_token.is_some()
        || audio_output_token.is_some();
    if has_token_rates {
        if let Some(fee) = amount {
            let note = format!(
                "OpenRouter also charges {fee} USD per request; AICC bills the declared token rates"
            );
            ratio_exception = Some(match ratio_exception {
                Some(existing) => format!("{existing}; {note}"),
                None => note,
            });
        }
    }
    let (unit, amount) = if has_token_rates {
        (None, None)
    } else {
        (amount.map(|_| crate::catalog::PricingUnit::Request), amount)
    };
    Ok(Some(Pricing {
        currency: "USD".to_owned(),
        source_url: Some("https://openrouter.ai/api/v1/models?output_modalities=all".into()),
        verified_at: Some(super::super::now_ms()?.to_string()),
        ratio_exception,
        cache_write_input_token,
        audio_input_token,
        image_input_token: None,
        cache_write_1h_input_token,
        audio_output_token,
        image_output_token: None,
        input_token,
        output_token,
        cache_input_token,
        estimated_cost: None,
        unit,
        amount,
        rules: Vec::new(),
        tiers: None,

        time_windows: Vec::new(),
        addons: Vec::new(),
    }))
}

fn parse_nonnegative_price(name: &str, value: Option<&str>) -> ProviderResult<Option<f64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.parse::<f64>().map_err(|_| {
        ProviderError::DiscoveryResponse(format!("OpenRouter {name} price is invalid"))
    })?;
    if value == -1.0 {
        return Ok(None);
    }
    if !value.is_finite() || value < 0.0 {
        return Err(ProviderError::DiscoveryResponse(format!(
            "OpenRouter {name} price must be finite and non-negative"
        )));
    }
    Ok(Some(value))
}

fn validate_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    if context.profile.provider_profile_id != OPENROUTER_PROVIDER_PROFILE_ID
        || context.profile.default_protocol_adapter_id != OPENROUTER_RESPONSES_ADAPTER_ID
        || context.instance.provider_profile_id != OPENROUTER_PROVIDER_PROFILE_ID
        || context.instance.protocol_adapter_id != OPENROUTER_RESPONSES_ADAPTER_ID
    {
        return Err(ProviderError::InvalidConfiguration(
            "OpenRouter discovery requires its builtin profile and adapter".to_owned(),
        ));
    }
    if context.credential.audit().kind != CredentialKind::Bearer {
        return Err(ProviderError::Credential(
            "OpenRouter discovery requires a Bearer credential".to_owned(),
        ));
    }
    if context.instance.account.is_some() {
        return Err(ProviderError::InvalidConfiguration(
            "OpenRouter profile does not accept account".to_owned(),
        ));
    }
    Ok(())
}

fn models_endpoint(base_url: &str) -> ProviderResult<String> {
    let mut url = Url::parse(&api_endpoint(base_url, "models")?).map_err(|_| {
        ProviderError::InvalidConfiguration("OpenRouter base_url is invalid".to_owned())
    })?;
    url.set_query(Some("output_modalities=all"));
    Ok(url.to_string())
}

fn api_endpoint(base_url: &str, endpoint: &str) -> ProviderResult<String> {
    let mut url = Url::parse(base_url).map_err(|_| {
        ProviderError::InvalidConfiguration("OpenRouter base_url is invalid".to_owned())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ProviderError::InvalidConfiguration(
            "OpenRouter base_url must be an absolute HTTP URL".to_owned(),
        ));
    }
    let path = url.path().trim_end_matches('/');
    let prefix = if path.ends_with("/api/v1") {
        path.to_owned()
    } else if path.is_empty() {
        "/api/v1".to_owned()
    } else {
        format!("{path}/api/v1")
    };
    url.set_path(&format!("{prefix}/{endpoint}"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn ensure_success(response: &HttpResponse) -> ProviderResult<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let message = format!(
        "OpenRouter models request failed with status {} (request {})",
        response.status, response.request_id
    );
    Err(if matches!(response.status.as_u16(), 401 | 403) {
        ProviderError::Credential(message)
    } else {
        ProviderError::Discovery(message)
    })
}

fn models_revision(models: &[DiscoveredModel]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(models).expect("validated discovery is serializable"));
    format!("sha256:{:x}", hasher.finalize())
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelObject>,
}

#[derive(Deserialize)]
struct ModelObject {
    id: String,
    canonical_slug: Option<String>,
    context_length: Option<u64>,
    #[serde(default)]
    supported_parameters: Vec<String>,
    architecture: Option<ModelArchitecture>,
    pricing: Option<ModelPricing>,
}

fn is_logical_model_id(id: &str) -> bool {
    id.starts_with('~')
        || id.starts_with("openrouter/")
        || id
            .rsplit_once('/')
            .is_some_and(|(_, model)| model.contains(':'))
}

fn classify_modalities(
    architecture: Option<&ModelArchitecture>,
) -> (Vec<ApiType>, BTreeSet<String>) {
    let Some(architecture) = architecture else {
        return (Vec::new(), BTreeSet::new());
    };
    let input = &architecture.input_modalities;
    let output = &architecture.output_modalities;
    let mut api_types = Vec::new();
    let mut operations = BTreeSet::new();
    if output.as_slice() == ["decisions"] {
        api_types.push(ApiType::Decision);
        operations.insert(OPENROUTER_DECISIONS_OPERATION_ID.to_owned());
    } else if output.as_slice() == ["rerank"] {
        api_types.push(ApiType::Rerank);
        operations.insert(OPENROUTER_RERANK_OPERATION_ID.to_owned());
    } else if output.as_slice() == ["embeddings"] {
        api_types.push(ApiType::EmbeddingText);
        operations.insert(OPENAI_EMBEDDINGS_OPERATION_ID.to_owned());
    } else if output.as_slice() == ["speech"] {
        api_types.push(ApiType::AudioTextToSpeech);
        operations.insert(OPENAI_AUDIO_SPEECH_OPERATION_ID.to_owned());
    } else if output.as_slice() == ["transcription"] {
        api_types.push(ApiType::AudioSpeechRecognition);
        operations.insert(OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID.to_owned());
    } else if output.as_slice() == ["video"] {
        api_types.push(ApiType::VideoGenerate);
        operations.insert(OPENROUTER_VIDEOS_OPERATION_ID.to_owned());
    } else if output.iter().any(|item| item == "audio")
        && output
            .iter()
            .all(|item| matches!(item.as_str(), "text" | "image" | "audio"))
    {
        api_types.push(ApiType::AudioMusic);
        operations.insert(OPENROUTER_MUSIC_OPERATION_ID.to_owned());
    } else if output
        .iter()
        .all(|item| matches!(item.as_str(), "text" | "image"))
    {
        if output.iter().any(|item| item == "text") {
            api_types.push(ApiType::Llm);
            operations.insert(OPENAI_CHAT_COMPLETIONS_OPERATION_ID.to_owned());
            operations.insert(OPENAI_RESPONSES_OPERATION_ID.to_owned());
            if input.iter().any(|item| item == "image") {
                api_types.push(ApiType::VisionOcr);
                api_types.push(ApiType::VisionCaption);
            }
        }
        if output.iter().any(|item| item == "image") {
            if input.iter().any(|item| item == "text") {
                api_types.push(ApiType::ImageTextToImage);
            }
            if input.iter().any(|item| item == "image") {
                api_types.push(ApiType::ImageImageToImage);
            }
            operations.insert(OPENROUTER_IMAGES_OPERATION_ID.to_owned());
        }
    }
    (api_types, operations)
}

#[derive(Deserialize)]
struct ModelArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    output_modalities: Vec<String>,
}

#[derive(Deserialize)]
struct VideoModelsResponse {
    data: Vec<VideoModelObject>,
}

#[derive(Deserialize)]
struct VideoModelObject {
    id: String,
    canonical_slug: Option<String>,
    supported_resolutions: Option<Vec<String>>,
    supported_aspect_ratios: Option<Vec<String>>,
    supported_sizes: Option<Vec<String>>,
    supported_durations: Option<Vec<u64>>,
    supported_frame_images: Option<Vec<String>>,
    generate_audio: Option<bool>,
    seed: Option<bool>,
    pricing_skus: Option<BTreeMap<String, String>>,
}

#[derive(Default, Deserialize)]
struct ModelPricing {
    prompt: Option<String>,
    completion: Option<String>,
    input_cache_read: Option<String>,
    input_cache_write: Option<String>,
    input_cache_write_1h: Option<String>,
    audio: Option<String>,
    audio_output: Option<String>,
    request: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ProtocolError, ResolvedCredential};
    use crate::provider::{CredentialReference, ProviderConnectionInput, ProviderInstanceConfig};
    use bytes::Bytes;
    use reqwest::header::{HeaderMap, AUTHORIZATION};
    use reqwest::StatusCode;
    use serde_json::json;
    use std::sync::Mutex;

    struct FakeTransport {
        request: Mutex<Option<HttpRequest>>,
        response: Mutex<Option<Result<HttpResponse, ProtocolError>>>,
    }

    #[async_trait]
    impl OpenRouterModelsTransport for FakeTransport {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
            *self.request.lock().unwrap() = Some(request);
            self.response.lock().unwrap().take().unwrap()
        }
    }

    #[test]
    fn official_extreme_token_price_ratio_is_preserved_with_source_explanation() {
        let pricing = parse_pricing(Some(ModelPricing {
            prompt: Some("0.000000037".into()),
            completion: Some("0.000007".into()),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
        assert_eq!(pricing.input_token, Some(0.000000037));
        assert_eq!(pricing.output_token, Some(0.000007));
        assert!(pricing.ratio_exception.is_some());
        crate::catalog::validate_pricing("z-ai/glm-5.2", &pricing).unwrap();
    }

    #[test]
    fn zero_request_fee_keeps_token_billing() {
        // The official catalog examples publish `"request": "0"` next to token rates. Treating
        // it as unit billing makes catalog validation reject the model, which fails the whole
        // discovery snapshot.
        let pricing = parse_pricing(Some(ModelPricing {
            prompt: Some("0.00003".into()),
            completion: Some("0.00006".into()),
            request: Some("0".into()),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
        assert_eq!(pricing.input_token, Some(0.00003));
        assert_eq!(pricing.output_token, Some(0.00006));
        assert_eq!(pricing.unit, None);
        assert_eq!(pricing.amount, None);
        crate::catalog::validate_pricing("openrouter/example", &pricing).unwrap();
    }

    #[test]
    fn non_zero_request_fee_with_token_rates_never_breaks_the_snapshot() {
        let pricing = parse_pricing(Some(ModelPricing {
            prompt: Some("0.00003".into()),
            completion: Some("0.00006".into()),
            request: Some("0.002".into()),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
        assert_eq!(pricing.unit, None);
        assert_eq!(pricing.amount, None);
        assert!(pricing
            .ratio_exception
            .as_deref()
            .is_some_and(|note| note.contains("0.002 USD per request")));
        crate::catalog::validate_pricing("openrouter/example", &pricing).unwrap();
    }

    #[test]
    fn request_only_model_stays_unit_priced() {
        let pricing = parse_pricing(Some(ModelPricing {
            request: Some("0.01".into()),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
        assert_eq!(pricing.unit, Some(crate::catalog::PricingUnit::Request));
        assert_eq!(pricing.amount, Some(0.01));
        crate::catalog::validate_pricing("openrouter/example", &pricing).unwrap();
    }

    #[test]
    fn connection_resolves_sovereign_region_base_urls() {
        let contract = super::super::builtin_connection_contract(OPENROUTER_PROVIDER_PROFILE_ID);
        assert_eq!(
            contract.resolve(Default::default()).unwrap().base_url,
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            contract
                .resolve(ProviderConnectionInput {
                    region: Some("us"),
                    ..Default::default()
                })
                .unwrap()
                .base_url,
            "https://us.openrouter.ai/api/v1"
        );
        assert_eq!(
            contract
                .resolve(ProviderConnectionInput {
                    region: Some("eu"),
                    ..Default::default()
                })
                .unwrap()
                .base_url,
            "https://eu.openrouter.ai/api/v1"
        );
    }

    #[test]
    fn provider_rules_narrow_gemini_images_without_changing_vendor_limits() {
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let argon = catalog
            .resolve_provider_rule(
                OPENROUTER_PROVIDER_PROFILE_ID,
                "google/gemini-4-argon",
                &Default::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(argon.action.provider_options["background"], true);
        assert_eq!(argon.action.provider_options["modalities"], json!(["text"]));
        assert_eq!(
            argon.action.operations["llm"],
            OPENAI_RESPONSES_OPERATION_ID
        );
        let vendor = catalog
            .resolve_model("gemini", "gemini-2.5-flash-image")
            .unwrap()
            .semantics
            .capabilities
            .unwrap();
        assert_eq!(vendor["image_generation"], true);
        assert_eq!(vendor["max_context_tokens"], 65_536);
        assert_eq!(vendor["max_output_tokens"], 32_768);

        let rule = catalog
            .resolve_provider_rule(
                OPENROUTER_PROVIDER_PROFILE_ID,
                "google/gemini-2.5-flash-image",
                &Default::default(),
            )
            .unwrap()
            .unwrap();
        let narrowed = rule.action.narrow(&Default::default(), &vendor);
        assert_eq!(narrowed.capabilities["image_generation"], true);
        assert_eq!(narrowed.capabilities["max_context_tokens"], 32_768);
        assert_eq!(narrowed.capabilities["max_output_tokens"], 8_192);
        assert_eq!(
            rule.action.operations["image.txt2img"],
            OPENROUTER_IMAGES_OPERATION_ID
        );

        let vendor = catalog
            .resolve_model("gemini", "gemini-nano-banana-2.1")
            .unwrap()
            .semantics
            .capabilities
            .unwrap();
        assert_eq!(vendor["max_context_tokens"], 131_072);
        assert_eq!(vendor["max_output_tokens"], 32_768);
        let rule = catalog
            .resolve_provider_rule(
                OPENROUTER_PROVIDER_PROFILE_ID,
                "google/gemini-nano-banana-2.1",
                &Default::default(),
            )
            .unwrap()
            .unwrap();
        let narrowed = rule.action.narrow(&Default::default(), &vendor);
        assert_eq!(narrowed.capabilities["max_context_tokens"], 65_536);
        assert_eq!(narrowed.capabilities["max_output_tokens"], 32_768);
        assert_eq!(
            rule.action.operations["image.img2img"],
            OPENROUTER_IMAGES_OPERATION_ID
        );
    }

    #[tokio::test]
    async fn discovery_preserves_aliases_for_identity_diagnostics_and_dynamic_prices() {
        let transport = Arc::new(FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(Some(Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from_static(br#"{"data":[{"id":"openai/gpt-5.4-mini","canonical_slug":"openai/gpt-5.4-mini","supported_parameters":["tools","response_format"],"architecture":{"input_modalities":["text","image"],"output_modalities":["text"]},"pricing":{"prompt":"0.000001","completion":"0.000002"},"expiration_date":null},{"id":"openai/text-embedding-3-small","canonical_slug":"openai/text-embedding-3-small","supported_parameters":[],"architecture":{"input_modalities":["text"],"output_modalities":["embeddings"]},"pricing":null,"expiration_date":null},{"id":"cohere/rerank-v3.5","canonical_slug":"cohere/rerank-v3.5","supported_parameters":[],"architecture":{"input_modalities":["text"],"output_modalities":["rerank"]},"pricing":null,"expiration_date":null},{"id":"openai/gpt-5.4-mini:free","canonical_slug":"openai/gpt-5.4-mini","supported_parameters":[],"architecture":null,"pricing":null,"expiration_date":null},{"id":"openrouter/auto","canonical_slug":"openrouter/auto","supported_parameters":[],"architecture":null,"pricing":{"prompt":"-1","completion":"-1"},"expiration_date":null}]}"#),
                request_id: "request-1".to_owned(),
                retry_after: None,
            }))),
        });
        let discovery = OpenRouterDiscovery::with_transport(transport.clone());
        let profile = openrouter_profile();
        let instance = ProviderInstanceConfig {
            provider_instance_name: "openrouter-main".to_owned(),
            provider_profile_id: OPENROUTER_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: OPENROUTER_RESPONSES_ADAPTER_ID.to_owned(),
            base_url: openrouter_known_provider().base_url,
            operation_base_urls: BTreeMap::new(),
            credential: CredentialReference {
                reference: "secret://openrouter".to_owned(),
            },
            credential_kind: None,
            provider_rules_id: Some(OPENROUTER_PROVIDER_PROFILE_ID.to_owned()),
            region: None,
            workspace: None,
            account: None,
            request_timeout: Duration::from_secs(120),
            auto_sync_models: true,
            instance_rules: None,
        };
        let credential = ResolvedCredential::bearer("secret://openrouter", "secret").unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();
        assert_eq!(snapshot.models.len(), 3);
        let language_model = snapshot
            .models
            .iter()
            .find(|model| model.provider_model_id == "openai/gpt-5.4-mini")
            .unwrap();
        assert_eq!(
            language_model.pricing.as_ref().unwrap().input_token,
            Some(0.000001)
        );
        assert!(language_model
            .supported_features
            .as_ref()
            .unwrap()
            .contains(features::VISION));
        let reranker = snapshot
            .models
            .iter()
            .find(|model| model.provider_model_id == "cohere/rerank-v3.5")
            .unwrap();
        assert_eq!(reranker.api_types, Some(vec![ApiType::Rerank]));
        assert_eq!(
            reranker.remote_methods,
            Some(BTreeSet::from([OPENROUTER_RERANK_OPERATION_ID.to_owned()]))
        );
        let embedding = snapshot
            .models
            .iter()
            .find(|model| model.provider_model_id == "openai/text-embedding-3-small")
            .unwrap();
        assert_eq!(embedding.api_types, Some(vec![ApiType::EmbeddingText]));
        assert_eq!(
            embedding.remote_methods,
            Some(BTreeSet::from([OPENAI_EMBEDDINGS_OPERATION_ID.to_owned()]))
        );
        assert!(snapshot
            .models
            .iter()
            .all(|model| model.provider_model_id != "openrouter/auto"
                && !model.provider_model_id.contains(':')));
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let providers = super::super::builtin_provider_registry(&catalog).unwrap();
        let inventory = crate::provider::InventoryBuilder::build_with_matcher(
            &profile,
            &instance,
            snapshot,
            &catalog,
            &providers.codecs(),
            Some(&discovery),
        )
        .unwrap();
        assert_eq!(inventory.models.len(), 3);
        assert!(inventory.unmatched_models.is_empty());
        let registry =
            crate::service::builtin_registry_for_test(&catalog, &[inventory.as_model_inventory()]);
        let candidates = registry
            .resolve_candidates("llm.gpt-mini", ApiType::Llm)
            .unwrap();
        assert!(!candidates.candidates.is_empty());
        let request = transport.request.lock().unwrap().take().unwrap();
        assert_eq!(
            request.url,
            "https://openrouter.ai/api/v1/models?output_modalities=all"
        );
        assert_eq!(request.headers[AUTHORIZATION], "Bearer secret");
        assert_eq!(
            openrouter_known_provider().base_url,
            "https://openrouter.ai/api/v1"
        );
        let rules = openrouter_provider_rules(3);
        let llm_pattern = rules
            .patterns
            .iter()
            .find(|pattern| pattern.operations.contains_key("llm"))
            .unwrap();
        assert_eq!(llm_pattern.operations["llm"], OPENAI_RESPONSES_OPERATION_ID);
    }

    fn jev_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../test/aicc_test/acceptance/fixtures/openrouter-jev-models.json"
        )))
        .unwrap()
    }

    struct SequenceTransport {
        urls: Mutex<Vec<String>>,
        responses: Mutex<std::collections::VecDeque<serde_json::Value>>,
    }

    #[async_trait]
    impl OpenRouterModelsTransport for SequenceTransport {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
            self.urls.lock().unwrap().push(request.url.clone());
            let body = self.responses.lock().unwrap().pop_front().unwrap();
            Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from(serde_json::to_vec(&body).unwrap()),
                request_id: "video-discovery".into(),
                retry_after: None,
            })
        }
    }

    /// `/models?output_modalities=all` and `/videos/models` entries as
    /// OpenRouter listed them on 2026-10-22.
    fn openrouter_video_catalogs(include_max: bool) -> (serde_json::Value, serde_json::Value) {
        let listed = |id: &str, slug: &str, inputs: &[&str]| {
            serde_json::json!({
                "id": id, "canonical_slug": slug, "context_length": 0,
                "architecture": {"input_modalities": inputs, "output_modalities": ["video"]},
                "pricing": {"prompt": "0", "completion": "0"},
                "supported_parameters": ["max_tokens", "temperature", "top_p"],
                "expiration_date": null
            })
        };
        let models = serde_json::json!({"data": [
            listed("minimax/hailuo-3", "minimax/hailuo-03-20260730", &["text", "image", "video", "audio"]),
            listed("minimax/hailuo-3-max", "minimax/hailuo-3-max-20260901", &["text", "image"]),
            listed("bytedance/seedance-2.0", "bytedance/seedance-2.0-20260414", &["text", "image", "video", "audio"]),
        ]});
        let ratios = ["21:9", "16:9", "4:3", "1:1", "3:4", "9:16"];
        let mut videos = vec![serde_json::json!({
            "id": "minimax/hailuo-3", "canonical_slug": "minimax/hailuo-03-20260730",
            "name": "MiniMax: H3", "created": 1785366648,
            "supported_resolutions": ["2K"], "supported_aspect_ratios": ratios,
            "supported_sizes": null, "supported_durations": [5,6,7,8,9,10,11,12,13,14,15],
            "supported_frame_images": ["first_frame", "last_frame"],
            "upscale_factor": null, "creativity": null, "generate_audio": true, "seed": false,
            "pricing_skus": {"duration_seconds": "0.13", "reference_images": "0.04"},
            "allowed_passthrough_parameters": ["aigc_watermark"]
        })];
        if include_max {
            videos.push(serde_json::json!({
                "id": "minimax/hailuo-3-max", "canonical_slug": "minimax/hailuo-3-max-20260901",
                "name": "MiniMax: H3 Max", "created": 1788313310,
                "supported_resolutions": ["768p", "480p"], "supported_aspect_ratios": ratios,
                "supported_sizes": null, "supported_durations": [5,6,7,8,9,10,11,12,13,14,15],
                "supported_frame_images": ["first_frame", "last_frame"],
                "upscale_factor": null, "creativity": null, "generate_audio": false, "seed": false,
                "pricing_skus": {"duration_seconds": "0.08", "duration_seconds_480p": "0.05", "duration_seconds_768p": "0.08"},
                "allowed_passthrough_parameters": ["aigc_watermark"]
            }));
        }
        videos.push(serde_json::json!({
            "id": "bytedance/seedance-2.0", "canonical_slug": "bytedance/seedance-2.0-20260414",
            "name": "ByteDance: Seedance 2.0", "created": 1776211362,
            "supported_resolutions": ["480p", "720p"], "supported_aspect_ratios": ratios,
            "supported_sizes": null, "supported_durations": [4, 5],
            "supported_frame_images": ["first_frame"], "upscale_factor": null, "creativity": null,
            "generate_audio": true, "seed": true,
            "pricing_skus": {"video_tokens": "0.000007"},
            "allowed_passthrough_parameters": []
        }));
        (models, serde_json::json!({"data": videos}))
    }

    async fn openrouter_video_inventory(
        include_max: bool,
    ) -> (
        crate::provider::ProviderDiscoverySnapshot,
        crate::provider::ProviderInventorySnapshot,
        Vec<String>,
    ) {
        let (models, videos) = openrouter_video_catalogs(include_max);
        let transport = Arc::new(SequenceTransport {
            urls: Mutex::new(Vec::new()),
            responses: Mutex::new([models, videos].into()),
        });
        let discovery = OpenRouterDiscovery::with_transport(transport.clone());
        let profile = openrouter_profile();
        let instance = ProviderInstanceConfig {
            provider_instance_name: "openrouter-main".into(),
            provider_profile_id: "openrouter".into(),
            protocol_adapter_id: "openrouter-responses".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            operation_base_urls: BTreeMap::new(),
            credential: CredentialReference {
                reference: "secret://openrouter".into(),
            },
            credential_kind: None,
            provider_rules_id: Some("openrouter".into()),
            region: None,
            workspace: None,
            account: None,
            request_timeout: Duration::from_secs(30),
            auto_sync_models: true,
            instance_rules: None,
        };
        let credential = ResolvedCredential::bearer("secret://openrouter", "sk-or").unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let providers = super::super::builtin_provider_registry(&catalog).unwrap();
        let inventory = crate::provider::InventoryBuilder::build_with_matcher(
            &profile,
            &instance,
            snapshot.clone(),
            &catalog,
            &providers.codecs(),
            Some(&discovery),
        )
        .unwrap();
        let urls = transport.urls.lock().unwrap().clone();
        (snapshot, inventory, urls)
    }

    #[tokio::test]
    async fn h3_channel_ids_map_to_minimax_identities_with_channel_contract_and_prices() {
        let (snapshot, inventory, urls) = openrouter_video_inventory(true).await;
        assert_eq!(
            urls,
            [
                "https://openrouter.ai/api/v1/models?output_modalities=all",
                "https://openrouter.ai/api/v1/videos/models",
            ]
        );
        let discovered = |id: &str| {
            snapshot
                .models
                .iter()
                .find(|model| model.provider_model_id == id)
                .unwrap()
        };
        let h3 = discovered("minimax/hailuo-3");
        assert_eq!(h3.api_types, Some(vec![ApiType::VideoGenerate]));
        assert_eq!(
            h3.remote_methods,
            Some(BTreeSet::from([OPENROUTER_VIDEOS_OPERATION_ID.to_owned()]))
        );
        let price = h3.pricing.as_ref().unwrap();
        assert_eq!(price.unit, Some(crate::catalog::PricingUnit::VideoSecond));
        assert_eq!(price.amount, Some(0.13));
        assert_eq!(
            price.addons[0].unit,
            crate::catalog::PricingUnit::ReferenceImage
        );
        assert_eq!(price.addons[0].amount, 0.04);
        let max_price = discovered("minimax/hailuo-3-max").pricing.as_ref().unwrap();
        assert_eq!(max_price.amount, Some(0.08));
        assert_eq!(max_price.rules.len(), 2);
        // A token-metered SKU is not mistaken for a free or per-second price.
        assert!(discovered("bytedance/seedance-2.0").pricing.is_none());

        let model = |id: &str| {
            inventory
                .models
                .iter()
                .find(|model| model.provider_model_id == id)
                .unwrap_or_else(|| panic!("{id} is not in the inventory"))
        };
        for (id, origin) in [
            ("minimax/hailuo-3", "MiniMax-H3"),
            ("minimax/hailuo-3-max", "MiniMax-H3-Max"),
        ] {
            let model = model(id);
            assert_eq!(model.model_driver_id, "minimax");
            assert_eq!(model.origin_model_id, origin);
            assert_eq!(model.api_types, vec![ApiType::VideoGenerate]);
            assert_eq!(model.operations["video.generate"], "videos.create");
            assert!(model
                .logical_mounts
                .contains(&"video.generate.minimax_h3".to_owned()));
            assert!(model.pricing.is_some());
        }
        let h3 = crate::video::VideoGenerationContract::from_capabilities(
            &model("minimax/hailuo-3").capabilities,
        )
        .unwrap();
        assert_eq!(h3.output.resolutions[0].name, "2K");
        assert_eq!(h3.output.audio, crate::video::AudioOutput::Always);
        assert_eq!(h3.references.image.as_ref().map(|limit| limit.max), Some(9));
        assert!(h3.references.video.is_none() && h3.references.audio.is_none());
        let max = crate::video::VideoGenerationContract::from_capabilities(
            &model("minimax/hailuo-3-max").capabilities,
        )
        .unwrap();
        assert!(max.references.image.is_none());
        assert_eq!(max.output.audio, crate::video::AudioOutput::Unknown);
        assert!(inventory
            .unmatched_models
            .iter()
            .any(|unmatched| unmatched.provider_model_id == "bytedance/seedance-2.0"));
    }

    #[tokio::test]
    async fn a_video_model_missing_from_the_video_catalog_is_not_routable() {
        let (snapshot, inventory, _) = openrouter_video_inventory(false).await;
        assert_eq!(
            snapshot
                .models
                .iter()
                .find(|model| model.provider_model_id == "minimax/hailuo-3-max")
                .unwrap()
                .availability,
            ModelAvailability::Unavailable
        );
        assert!(inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "minimax/hailuo-3-max")
            .is_none_or(|model| model.api_types.is_empty()));
    }

    async fn jev_inventory(
        body: serde_json::Value,
    ) -> ProviderResult<crate::provider::ProviderInventorySnapshot> {
        let transport = Arc::new(FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(Some(Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from(serde_json::to_vec(&body).unwrap()),
                request_id: "jev-discovery".into(),
                retry_after: None,
            }))),
        });
        let discovery = OpenRouterDiscovery::with_transport(transport);
        let profile = openrouter_profile();
        let instance = ProviderInstanceConfig {
            provider_instance_name: "openrouter-test".into(),
            provider_profile_id: "openrouter".into(),
            protocol_adapter_id: "openrouter-responses".into(),
            base_url: "https://proxy.test/api/v1".into(),
            operation_base_urls: BTreeMap::new(),
            credential: CredentialReference {
                reference: "secret://test".into(),
            },
            credential_kind: None,
            provider_rules_id: Some("openrouter".into()),
            region: None,
            workspace: None,
            account: None,
            request_timeout: Duration::from_secs(30),
            auto_sync_models: true,
            instance_rules: None,
        };
        let credential = ResolvedCredential::bearer("secret://test", "test").unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await?;
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let providers = super::super::builtin_provider_registry(&catalog).unwrap();
        crate::provider::InventoryBuilder::build_with_matcher(
            &profile,
            &instance,
            snapshot,
            &catalog,
            &providers.codecs(),
            Some(&discovery),
        )
    }

    #[tokio::test]
    async fn openrouter_decision_discovery_requires_verified_inventory_and_narrows_limits() {
        let inventory = jev_inventory(jev_fixture()).await.unwrap();
        assert_eq!(inventory.models.len(), 1);
        for model in &inventory.models {
            assert_eq!(model.origin_model_id, JEV_ORIGIN_ID);
            assert_eq!(model.api_types, vec![ApiType::Decision]);
            assert_eq!(
                model.operations,
                BTreeMap::from([("decision".into(), OPENROUTER_DECISIONS_OPERATION_ID.into())])
            );
            assert_eq!(model.capabilities["max_context_tokens"], 32000);
            assert_eq!(model.capabilities["decision.max_input_bytes"], 32000);
            assert_eq!(model.capabilities["decision.max_options"], 255);
            assert_eq!(model.capabilities["decision.max_levels"], 10);
            assert!(!model.capabilities.contains_key("tool_call"));
            let price = &model.pricing.as_ref().unwrap().value;
            assert_eq!(price.input_token, Some(0.000000042));
            assert_eq!(price.output_token, Some(0.0));
            assert!(price.source_url.as_ref().unwrap().contains("openrouter.ai"));
        }
        assert!(inventory
            .unmatched_models
            .iter()
            .any(|m| m.provider_model_id == "typesafe/jev-router"));
        assert!(jev_inventory(serde_json::json!({"data":[]}))
            .await
            .unwrap()
            .models
            .is_empty());
        for (field, value) in [
            (
                "canonical_slug",
                serde_json::json!("typesafe/jev-1.14-20261001"),
            ),
            ("context_length", serde_json::json!(16000)),
        ] {
            let mut fixture = jev_fixture();
            let target = fixture["data"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|m| m["id"] == JEV_CHANNEL_ID)
                .unwrap();
            target[field] = value;
            assert!(jev_inventory(fixture)
                .await
                .unwrap()
                .models
                .iter()
                .all(|m| m.api_types.is_empty()));
        }
        assert!(inventory
            .models
            .iter()
            .all(|model| model.provider_model_id
                != crate::protocol::openrouter_decisions::JEV_ALIAS_ID));
        let mut fixture = jev_fixture();
        for model in fixture["data"].as_array_mut().unwrap() {
            model["pricing"] = serde_json::Value::Null;
        }
        assert!(jev_inventory(fixture)
            .await
            .unwrap()
            .models
            .iter()
            .all(|m| m.pricing.is_none()));
        let mut fixture = jev_fixture();
        let duplicate = fixture["data"][0].clone();
        fixture["data"].as_array_mut().unwrap().push(duplicate);
        assert!(jev_inventory(fixture).await.is_err());
    }

    #[tokio::test]
    async fn openrouter_discovery_cannot_expand_vendor_model_api_types() {
        let inventory = jev_inventory(serde_json::json!({"data": [
            {
                "id": "google/gemini-2.5-flash-image",
                "canonical_slug": "google/gemini-2.5-flash-image",
                "context_length": 65536,
                "supported_parameters": [],
                "architecture": {
                    "input_modalities": ["text", "image"],
                    "output_modalities": ["text", "image"]
                },
                "pricing": null
            },
            {
                "id": "qwen/qwen3.8-27b",
                "canonical_slug": "qwen/qwen3.8-27b",
                "context_length": 1000000,
                "supported_parameters": [],
                "architecture": {
                    "input_modalities": ["text", "image"],
                    "output_modalities": ["text"]
                },
                "pricing": null
            }
        ]}))
        .await
        .unwrap();
        let api_types = |model_id: &str| {
            inventory
                .models
                .iter()
                .find(|model| model.provider_model_id == model_id)
                .unwrap()
                .api_types
                .clone()
        };
        let image_api_types = api_types("google/gemini-2.5-flash-image");
        assert_eq!(image_api_types.len(), 2);
        assert!(image_api_types.contains(&ApiType::ImageTextToImage));
        assert!(image_api_types.contains(&ApiType::ImageImageToImage));
        assert_eq!(api_types("qwen/qwen3.8-27b"), vec![ApiType::Llm]);
    }

    #[test]
    fn openrouter_unknown_modalities_and_future_identities_fail_closed() {
        let (api_types, operations) = classify_modalities(Some(&ModelArchitecture {
            input_modalities: vec!["text".into(), "image".into()],
            output_modalities: vec!["text".into(), "image".into()],
        }));
        assert_eq!(
            api_types,
            vec![
                ApiType::Llm,
                ApiType::VisionOcr,
                ApiType::VisionCaption,
                ApiType::ImageTextToImage,
                ApiType::ImageImageToImage,
            ]
        );
        assert!(operations.contains(OPENAI_CHAT_COMPLETIONS_OPERATION_ID));
        assert!(operations.contains(OPENAI_RESPONSES_OPERATION_ID));
        assert!(operations.contains(OPENROUTER_IMAGES_OPERATION_ID));

        let (api_types, operations) = classify_modalities(Some(&ModelArchitecture {
            input_modalities: vec!["text".into(), "image".into()],
            output_modalities: vec!["text".into(), "audio".into()],
        }));
        assert_eq!(api_types, vec![ApiType::AudioMusic]);
        assert_eq!(
            operations,
            BTreeSet::from([crate::protocol::OPENROUTER_MUSIC_OPERATION_ID.to_owned()])
        );
        for output in [
            vec!["decisions", "text"],
            vec!["rerank", "embeddings"],
            vec!["future"],
            vec![],
        ] {
            assert!(classify_modalities(Some(&ModelArchitecture {
                input_modalities: vec!["text".into()],
                output_modalities: output.into_iter().map(str::to_owned).collect()
            }))
            .0
            .is_empty());
        }
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &Default::default(),
        )
        .unwrap();
        let discovery = OpenRouterDiscovery::with_transport(Arc::new(FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(None),
        }));
        for id in [
            "typesafe/jev-1.14",
            JEV_BUILD_ID,
            "typesafe/jev-router",
            "typesafe/jev-1.13.0",
            "~typesafe/jev-preview",
        ] {
            assert!(matches!(
                discovery.match_model_driver(id, &catalog),
                ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias)
            ));
        }
        assert!(matches!(
            discovery.match_model_driver("openai/gpt-5.4-mini", &catalog),
            ProviderModelMatch::Matched(_)
        ));
        for (base, expected) in [
            (
                "https://proxy.test/tenant/api/v1/",
                "https://proxy.test/tenant/api/v1/models?output_modalities=all",
            ),
            (
                "https://proxy.test/tenant",
                "https://proxy.test/tenant/api/v1/models?output_modalities=all",
            ),
        ] {
            assert_eq!(models_endpoint(base).unwrap(), expected);
        }
    }

    #[test]
    fn endpoint_and_pricing_reject_invalid_boundary_values() {
        assert_eq!(
            models_endpoint("https://openrouter.ai").unwrap(),
            "https://openrouter.ai/api/v1/models?output_modalities=all"
        );
        assert!(models_endpoint("file:///tmp/openrouter").is_err());
        assert_eq!(parse_nonnegative_price("prompt", Some("-1")).unwrap(), None);
        assert_eq!(
            parse_nonnegative_price("completion", Some("-1.0")).unwrap(),
            None
        );
        assert_eq!(
            parse_nonnegative_price("prompt", Some("0")).unwrap(),
            Some(0.0)
        );
        let partial = parse_pricing(Some(ModelPricing {
            prompt: Some("-1".into()),
            completion: Some("0.000002".into()),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
        assert_eq!(partial.input_token, None);
        assert_eq!(partial.output_token, Some(0.000002));
        assert!(parse_nonnegative_price("prompt", Some("-0.1")).is_err());
        assert!(parse_nonnegative_price("prompt", Some("-2")).is_err());
        assert!(parse_nonnegative_price("prompt", Some("invalid")).is_err());
        assert!(parse_nonnegative_price("prompt", Some("NaN")).is_err());
    }
}
