#![allow(dead_code)]

use crate::catalog::{CatalogResolveError, CatalogSnapshot, Pricing, ResolvedProviderRule};
use crate::matching::MatchContext;
use crate::model::{ExactModelName, ModelRegistryError};
use crate::protocol::{
    CodecContext, CodecInput, CodecLimits, CodecRegistry, CredentialAudit, ResolvedCredential,
};
use crate::routing::{RouteDecision, SelectedRoute};
use buckyos_api::{AiccCall, ApiType, ResourceRef};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PricingSource {
    Discovery,
    ProviderRules,
    ModelDriver,
    RouteEstimate,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ResolvedPricing {
    pub source: PricingSource,
    pub pricing: Option<Pricing>,
    pub matched_amount: Option<f64>,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Clone)]
pub(crate) struct ProviderCallTarget {
    pub provider_rules_id: String,
    pub base_url: String,
    pub credential: ResolvedCredential,
    pub limits: CodecLimits,
    pub pricing: Option<ResolvedPricing>,
    pub match_dimensions: MatchContext,
}

impl fmt::Debug for ProviderCallTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCallTarget")
            .field("provider_rules_id", &self.provider_rules_id)
            .field("base_url", &self.base_url)
            .field("credential", &self.credential.audit())
            .field("limits", &self.limits)
            .field("pricing", &self.pricing)
            .field(
                "match_dimension_names",
                &self.match_dimensions.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResourceRequirement {
    pub request_pointer: String,
    pub resource: ResourceRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct LoweringRevisions {
    pub catalog_target_seq: u64,
    pub model_driver_revision_seq: u64,
    pub provider_rules_revision_seq: u64,
    pub inventory_revision: String,
}

#[derive(Clone)]
pub(crate) struct ResolvedProviderCall {
    pub exact_model: String,
    pub provider_model_id: String,
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub variant: Option<String>,
    pub method: String,
    pub api_type: ApiType,
    pub operation: String,
    pub input: CodecInput,
    pub context: CodecContext,
    pub credential: CredentialAudit,
    pub resource_requirements: Vec<ResourceRequirement>,
    pub pricing: ResolvedPricing,
    pub revisions: LoweringRevisions,
}

impl fmt::Debug for ResolvedProviderCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedProviderCall")
            .field("exact_model", &self.exact_model)
            .field("provider_model_id", &self.provider_model_id)
            .field("provider_instance_name", &self.provider_instance_name)
            .field("provider_profile_id", &self.provider_profile_id)
            .field("protocol_adapter_id", &self.protocol_adapter_id)
            .field("model_driver_id", &self.model_driver_id)
            .field("origin_model_id", &self.origin_model_id)
            .field("variant", &self.variant)
            .field("method", &self.method)
            .field("api_type", &self.api_type)
            .field("operation", &self.operation)
            .field("input", &self.input)
            .field("context", &self.context)
            .field("credential", &self.credential)
            .field(
                "resource_requirement_pointers",
                &self
                    .resource_requirements
                    .iter()
                    .map(|requirement| requirement.request_pointer.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("pricing", &self.pricing)
            .field("revisions", &self.revisions)
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DeterministicCallView<'a> {
    exact_model: &'a str,
    provider_model_id: &'a str,
    provider_instance_name: &'a str,
    provider_profile_id: &'a str,
    protocol_adapter_id: &'a str,
    model_driver_id: &'a str,
    origin_model_id: &'a str,
    variant: Option<&'a str>,
    method: &'a str,
    api_type: &'static str,
    operation: &'a str,
    resolved_parameters: &'a BTreeMap<String, Value>,
    credential_kind: &'static str,
    credential_ref: &'a str,
    resource_pointers: Vec<&'a str>,
    pricing: &'a ResolvedPricing,
    revisions: &'a LoweringRevisions,
}

impl ResolvedProviderCall {
    pub(crate) fn deterministic_view(&self) -> DeterministicCallView<'_> {
        DeterministicCallView {
            exact_model: &self.exact_model,
            provider_model_id: &self.provider_model_id,
            provider_instance_name: &self.provider_instance_name,
            provider_profile_id: &self.provider_profile_id,
            protocol_adapter_id: &self.protocol_adapter_id,
            model_driver_id: &self.model_driver_id,
            origin_model_id: &self.origin_model_id,
            variant: self.variant.as_deref(),
            method: &self.method,
            api_type: api_type_name(self.api_type),
            operation: &self.operation,
            resolved_parameters: &self.input.resolved_parameters,
            credential_kind: self.credential.kind.as_str(),
            credential_ref: self.credential.anonymous_ref.as_str(),
            resource_pointers: self
                .resource_requirements
                .iter()
                .map(|requirement| requirement.request_pointer.as_str())
                .collect(),
            pricing: &self.pricing,
            revisions: &self.revisions,
        }
    }
}

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
    InvalidRule(String),
}

impl fmt::Display for CallLoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedCanonicalCall(v) => write!(f, "{v} is not a typed inference call"),
            Self::InvalidCanonicalRequest(v) => write!(f, "invalid canonical request: {v}"),
            Self::InvalidExactModel(v) => v.fmt(f),
            Self::RouteMismatch(v) => write!(f, "route decision mismatch: {v}"),
            Self::Catalog(v) => v.fmt(f),
            Self::MissingModelVariant {
                model_driver_id,
                variant,
            } => write!(
                f,
                "model driver `{model_driver_id}` does not define variant `{variant}` for this model"
            ),
            Self::AmbiguousModelVariant {
                model_driver_id,
                variant,
            } => write!(
                f,
                "model driver `{model_driver_id}` defines variant `{variant}` more than once"
            ),
            Self::MissingProviderVariant {
                provider_rules_id,
                variant,
            } => write!(
                f,
                "provider rules `{provider_rules_id}` do not lower variant `{variant}`"
            ),
            Self::AmbiguousProviderVariant {
                provider_rules_id,
                variant,
            } => write!(
                f,
                "provider rules `{provider_rules_id}` lower variant `{variant}` more than once"
            ),
            Self::UnknownAdapter(v) => write!(f, "unknown adapter `{v}`"),
            Self::MissingDefaultOperation {
                adapter_id,
                api_type,
            } => write!(
                f,
                "adapter `{adapter_id}` has no default operation for `{api_type}`"
            ),
            Self::AmbiguousDefaultOperation {
                adapter_id,
                api_type,
            } => write!(
                f,
                "adapter `{adapter_id}` has multiple default operations for `{api_type}`"
            ),
            Self::RouteOperationMismatch { routed, lowered } => write!(
                f,
                "routed operation `{routed}` differs from lowered operation `{lowered}`"
            ),
            Self::UnsupportedOperation(v) => f.write_str(v),
            Self::InvalidRule(v) => write!(f, "invalid request rule: {v}"),
        }
    }
}

impl Error for CallLoweringError {}
impl From<CatalogResolveError> for CallLoweringError {
    fn from(value: CatalogResolveError) -> Self {
        Self::Catalog(value)
    }
}
impl From<ModelRegistryError> for CallLoweringError {
    fn from(value: ModelRegistryError) -> Self {
        Self::InvalidExactModel(value)
    }
}

pub(crate) struct CallResolver<'a> {
    catalog: &'a CatalogSnapshot,
    codecs: &'a CodecRegistry,
}

impl<'a> CallResolver<'a> {
    pub(crate) fn new(catalog: &'a CatalogSnapshot, codecs: &'a CodecRegistry) -> Self {
        Self { catalog, codecs }
    }

    pub(crate) fn lower(
        &self,
        decision: &RouteDecision,
        canonical_request: &AiccCall,
        target: ProviderCallTarget,
    ) -> Result<ResolvedProviderCall, CallLoweringError> {
        let api_type = call_api_type(canonical_request)?;
        let method = canonical_request.method().to_owned();
        let exact_model = call_exact_model(canonical_request)?;
        validate_route(&decision.selected, exact_model, api_type)?;
        let parsed_exact = ExactModelName::parse(exact_model)?;
        let api_name = api_type_name(api_type);
        let mut identity_context = target.match_dimensions.clone();
        identity_context.extend([
            (
                "provider_model_id".into(),
                Value::String(decision.selected.provider_model_id.clone()),
            ),
            (
                "origin_model_id".into(),
                Value::String(decision.selected.origin_model_id.clone()),
            ),
            (
                "model_driver_id".into(),
                Value::String(decision.selected.model_driver_id.clone()),
            ),
            ("api_type".into(), Value::String(api_name.into())),
        ]);
        if let Some(variant) = parsed_exact.variant() {
            identity_context.insert("variant".into(), Value::String(variant.into()));
        }
        let provider_rule = self.catalog.resolve_provider_rule(
            &target.provider_rules_id,
            &decision.selected.provider_model_id,
            &identity_context,
        )?;
        let operation = self.resolve_operation(
            &decision.selected,
            provider_rule.as_ref(),
            api_type,
            &method,
        )?;
        let canonical_json = serialize_call(canonical_request)?;
        let option_keys = canonical_option_keys(canonical_request)?;
        let mut normalized = Value::Object(Map::new());
        if let Some(rule) = &provider_rule {
            merge_overwrite(
                &mut normalized,
                &Value::Object(rule.action.provider_options.clone().into_iter().collect()),
            );
        }
        let variant_options = self.resolve_variant(
            &target.provider_rules_id,
            &decision.selected,
            parsed_exact.variant(),
            &identity_context,
        )?;
        merge_overwrite(
            &mut normalized,
            &Value::Object(variant_options.into_iter().collect()),
        );
        merge_user_options(&mut normalized, &canonical_json, option_keys);
        if let Some(rule) = &provider_rule {
            let defaults_context = request_match_context(api_name, &operation, &normalized);
            for request_rule in rule.matching_request_rules(&defaults_context) {
                fill_defaults(
                    &mut normalized,
                    &Value::Object(request_rule.defaults.clone().into_iter().collect()),
                );
            }
            let rewrite_context = request_match_context(api_name, &operation, &normalized);
            for request_rule in rule.matching_request_rules(&rewrite_context) {
                merge_overwrite(
                    &mut normalized,
                    &Value::Object(request_rule.set.clone().into_iter().collect()),
                );
                for pointer in &request_rule.remove {
                    remove_pointer(&mut normalized, pointer)?;
                }
            }
        }
        let rewritten_json = rewrite_canonical_options(&canonical_json, &normalized, option_keys)?;
        let rewritten_request =
            AiccCall::from_method_and_params(&method, rewritten_json.clone())
                .map_err(|error| CallLoweringError::InvalidCanonicalRequest(error.to_string()))?;
        let mut resolved_parameters = provider_parameters(&normalized, option_keys);
        resolved_parameters.insert(
            "provider_model_id".into(),
            Value::String(decision.selected.provider_model_id.clone()),
        );
        let input = CodecInput {
            canonical_request: rewritten_request,
            resolved_parameters,
        };
        let descriptor = self
            .codecs
            .operation_descriptor(&decision.selected.protocol_adapter_id, &operation, api_type)
            .map_err(|error| CallLoweringError::UnsupportedOperation(error.to_string()))?;
        input
            .validate_for(
                descriptor
                    .binding(api_type)
                    .map_err(|error| CallLoweringError::UnsupportedOperation(error.to_string()))?,
            )
            .map_err(|error| CallLoweringError::InvalidCanonicalRequest(error.to_string()))?;
        let resources = collect_resource_requirements(&rewritten_json);
        let model_revision = self
            .catalog
            .model_driver(&decision.selected.model_driver_id)
            .ok_or_else(|| {
                CallLoweringError::RouteMismatch(format!(
                    "model driver `{}` is absent from the catalog snapshot",
                    decision.selected.model_driver_id
                ))
            })?
            .revision_seq;
        let provider_revision = self
            .catalog
            .provider_rules(&target.provider_rules_id)
            .ok_or_else(|| {
                CallLoweringError::RouteMismatch(format!(
                    "provider rules `{}` are absent from the catalog snapshot",
                    target.provider_rules_id
                ))
            })?
            .revision_seq;
        let pricing_context = request_match_context(api_name, &operation, &normalized);
        let pricing = resolve_pricing(
            target.pricing,
            provider_rule
                .as_ref()
                .and_then(|rule| rule.action.pricing.clone()),
            provider_rule
                .as_ref()
                .and_then(|rule| rule.price_for(&pricing_context)),
            decision.selected.estimated_cost_usd,
        );
        let credential = target.credential.audit().clone();
        let context = CodecContext {
            base_url: target.base_url,
            credential: Some(target.credential),
            resources: BTreeMap::new(),
            limits: target.limits,
        };
        context
            .validate()
            .map_err(|error| CallLoweringError::InvalidCanonicalRequest(error.to_string()))?;
        Ok(ResolvedProviderCall {
            exact_model: exact_model.into(),
            provider_model_id: decision.selected.provider_model_id.clone(),
            provider_instance_name: decision.selected.provider_instance_name.clone(),
            provider_profile_id: decision.selected.provider_profile_id.clone(),
            protocol_adapter_id: decision.selected.protocol_adapter_id.clone(),
            model_driver_id: decision.selected.model_driver_id.clone(),
            origin_model_id: decision.selected.origin_model_id.clone(),
            variant: parsed_exact.variant().map(str::to_owned),
            method,
            api_type,
            operation,
            input,
            context,
            credential,
            resource_requirements: resources,
            pricing,
            revisions: LoweringRevisions {
                catalog_target_seq: self.catalog.target_revision_seq(),
                model_driver_revision_seq: model_revision,
                provider_rules_revision_seq: provider_revision,
                inventory_revision: decision.selected.inventory_revision.clone(),
            },
        })
    }

    fn resolve_operation(
        &self,
        selected: &SelectedRoute,
        rule: Option<&ResolvedProviderRule>,
        api_type: ApiType,
        method: &str,
    ) -> Result<String, CallLoweringError> {
        let empty = BTreeMap::new();
        let mappings = rule.map_or(&empty, |rule| &rule.action.operations);
        let operation = select_operation(
            self.codecs,
            &selected.protocol_adapter_id,
            mappings,
            api_type,
            method,
        )?;
        if selected.operation != operation {
            return Err(CallLoweringError::RouteOperationMismatch {
                routed: selected.operation.clone(),
                lowered: operation,
            });
        }
        Ok(selected.operation.clone())
    }

    fn resolve_variant(
        &self,
        rules_id: &str,
        selected: &SelectedRoute,
        variant: Option<&str>,
        context: &MatchContext,
    ) -> Result<BTreeMap<String, Value>, CallLoweringError> {
        let Some(variant) = variant else {
            return Ok(BTreeMap::new());
        };
        match self
            .catalog
            .matching_model_variants(&selected.model_driver_id, context)?
            .into_iter()
            .filter(|candidate| candidate.name == variant)
            .count()
        {
            0 => {
                return Err(CallLoweringError::MissingModelVariant {
                    model_driver_id: selected.model_driver_id.clone(),
                    variant: variant.into(),
                });
            }
            1 => {}
            _ => {
                return Err(CallLoweringError::AmbiguousModelVariant {
                    model_driver_id: selected.model_driver_id.clone(),
                    variant: variant.into(),
                });
            }
        }
        let matches = self
            .catalog
            .matching_provider_variants(rules_id, context)?
            .into_iter()
            .filter(|candidate| {
                candidate.model_driver == selected.model_driver_id && candidate.variant == variant
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Err(CallLoweringError::MissingProviderVariant {
                provider_rules_id: rules_id.into(),
                variant: variant.into(),
            }),
            [matched] => Ok(matched.provider_options.clone()),
            _ => Err(CallLoweringError::AmbiguousProviderVariant {
                provider_rules_id: rules_id.into(),
                variant: variant.into(),
            }),
        }
    }
}

fn select_operation(
    codecs: &CodecRegistry,
    adapter_id: &str,
    mappings: &BTreeMap<String, String>,
    api_type: ApiType,
    method: &str,
) -> Result<String, CallLoweringError> {
    let api_name = api_type_name(api_type);
    let configured = mappings
        .get(method)
        .or_else(|| mappings.get(api_name))
        .cloned();
    let operation = match configured {
        Some(operation) => operation,
        None => {
            let adapter = codecs
                .adapter(adapter_id)
                .ok_or_else(|| CallLoweringError::UnknownAdapter(adapter_id.into()))?;
            let matching = adapter
                .operations
                .values()
                .filter(|operation| {
                    operation
                        .bindings
                        .iter()
                        .any(|binding| binding.api_type == api_type)
                })
                .map(|operation| operation.operation_id.clone())
                .collect::<Vec<_>>();
            match matching.as_slice() {
                [] => {
                    return Err(CallLoweringError::MissingDefaultOperation {
                        adapter_id: adapter_id.into(),
                        api_type: api_name.into(),
                    });
                }
                [operation] => operation.clone(),
                _ => {
                    return Err(CallLoweringError::AmbiguousDefaultOperation {
                        adapter_id: adapter_id.into(),
                        api_type: api_name.into(),
                    });
                }
            }
        }
    };
    codecs
        .operation_descriptor(adapter_id, &operation, api_type)
        .map_err(|error| CallLoweringError::UnsupportedOperation(error.to_string()))?;
    Ok(operation)
}

fn validate_route(
    selected: &SelectedRoute,
    exact_model: &str,
    _api_type: ApiType,
) -> Result<(), CallLoweringError> {
    if selected.exact_model != exact_model {
        return Err(CallLoweringError::RouteMismatch(
            "canonical exact_model differs from the selected route".into(),
        ));
    }
    let parsed = ExactModelName::parse(exact_model)?;
    if parsed.provider_model_id() != selected.provider_model_id
        || parsed.provider_instance_name() != selected.provider_instance_name
    {
        return Err(CallLoweringError::RouteMismatch(
            "exact model identity differs from the selected route".into(),
        ));
    }
    Ok(())
}

fn call_api_type(call: &AiccCall) -> Result<ApiType, CallLoweringError> {
    match call {
        AiccCall::ChatCompletionsCreate(_) => Ok(ApiType::Llm),
        AiccCall::ImagesGenerate(_) => Ok(ApiType::ImageTextToImage),
        AiccCall::EmbeddingText(_) => Ok(ApiType::EmbeddingText),
        AiccCall::EmbeddingMultimodal(_) => Ok(ApiType::EmbeddingMultimodal),
        AiccCall::Rerank(_) => Ok(ApiType::Rerank),
        AiccCall::ImageToImage(_) => Ok(ApiType::ImageImageToImage),
        AiccCall::ImageInpaint(_) => Ok(ApiType::ImageInpaint),
        AiccCall::ImageUpscale(_) => Ok(ApiType::ImageUpscale),
        AiccCall::ImageBackgroundRemove(_) => Ok(ApiType::ImageBackgroundRemove),
        AiccCall::VisionOcr(_) => Ok(ApiType::VisionOcr),
        AiccCall::VisionCaption(_) => Ok(ApiType::VisionCaption),
        AiccCall::VisionDetect(_) => Ok(ApiType::VisionDetect),
        AiccCall::VisionSegment(_) => Ok(ApiType::VisionSegment),
        AiccCall::AudioTextToSpeech(_) => Ok(ApiType::AudioTextToSpeech),
        AiccCall::AudioSpeechRecognition(_) => Ok(ApiType::AudioSpeechRecognition),
        AiccCall::AudioMusic(_) => Ok(ApiType::AudioMusic),
        AiccCall::AudioEnhance(_) => Ok(ApiType::AudioEnhance),
        AiccCall::VideoTextToVideo(_) => Ok(ApiType::VideoTextToVideo),
        AiccCall::VideoImageToVideo(_) => Ok(ApiType::VideoImageToVideo),
        AiccCall::VideoToVideo(_) => Ok(ApiType::VideoToVideo),
        AiccCall::VideoExtend(_) => Ok(ApiType::VideoExtend),
        AiccCall::VideoUpscale(_) => Ok(ApiType::VideoUpscale),
        AiccCall::ComputerUse(_) => Ok(ApiType::AgentComputerUse),
        AiccCall::RouteResolve(_) | AiccCall::HelperLlmChat(_) | AiccCall::HelperTextToImage(_) => {
            Err(CallLoweringError::UnsupportedCanonicalCall(
                call.method().into(),
            ))
        }
    }
}

macro_rules! call_request {
    ($call:expr, $binding:ident => $value:expr) => {
        match $call {
            AiccCall::ChatCompletionsCreate($binding) => $value,
            AiccCall::ImagesGenerate($binding) => $value,
            AiccCall::EmbeddingText($binding) => $value,
            AiccCall::EmbeddingMultimodal($binding) => $value,
            AiccCall::Rerank($binding) => $value,
            AiccCall::ImageToImage($binding) => $value,
            AiccCall::ImageInpaint($binding) => $value,
            AiccCall::ImageUpscale($binding) => $value,
            AiccCall::ImageBackgroundRemove($binding) => $value,
            AiccCall::VisionOcr($binding) => $value,
            AiccCall::VisionCaption($binding) => $value,
            AiccCall::VisionDetect($binding) => $value,
            AiccCall::VisionSegment($binding) => $value,
            AiccCall::AudioTextToSpeech($binding) => $value,
            AiccCall::AudioSpeechRecognition($binding) => $value,
            AiccCall::AudioMusic($binding) => $value,
            AiccCall::AudioEnhance($binding) => $value,
            AiccCall::VideoTextToVideo($binding) => $value,
            AiccCall::VideoImageToVideo($binding) => $value,
            AiccCall::VideoToVideo($binding) => $value,
            AiccCall::VideoExtend($binding) => $value,
            AiccCall::VideoUpscale($binding) => $value,
            AiccCall::ComputerUse($binding) => $value,
            AiccCall::RouteResolve(_)
            | AiccCall::HelperLlmChat(_)
            | AiccCall::HelperTextToImage(_) => {
                return Err(CallLoweringError::UnsupportedCanonicalCall(
                    $call.method().into(),
                ));
            }
        }
    };
}

fn serialize_call(call: &AiccCall) -> Result<Value, CallLoweringError> {
    call_request!(call, request => serde_json::to_value(request))
        .map_err(|error| CallLoweringError::InvalidCanonicalRequest(error.to_string()))
}

fn call_exact_model(call: &AiccCall) -> Result<&str, CallLoweringError> {
    Ok(call_request!(call, request => request.exact_model.as_str()))
}

fn canonical_option_keys(call: &AiccCall) -> Result<&'static [&'static str], CallLoweringError> {
    Ok(match call {
        AiccCall::ChatCompletionsCreate(_) => &[
            "response_format",
            "temperature",
            "top_p",
            "max_output_tokens",
            "seed",
            "stop",
            "output",
        ],
        AiccCall::ImagesGenerate(_) => &[
            "negative_prompt",
            "n",
            "aspect_ratio",
            "size",
            "quality",
            "style",
            "seed",
            "output",
        ],
        AiccCall::EmbeddingText(_) => &[
            "chunking",
            "embedding_space_id",
            "dimensions",
            "normalize",
            "prefer_artifact",
        ],
        AiccCall::EmbeddingMultimodal(_) => &["dimensions", "normalize"],
        AiccCall::Rerank(_) => &["n", "return_documents"],
        AiccCall::ImageToImage(_) => &["strength", "output"],
        AiccCall::ImageInpaint(_) => &["mask_semantics", "output"],
        AiccCall::ImageUpscale(_) => &[
            "scale",
            "target_width",
            "target_height",
            "preserve_faces",
            "output",
        ],
        AiccCall::ImageBackgroundRemove(_) => &["mode", "output"],
        AiccCall::VisionOcr(_) => &[
            "level",
            "language_hints",
            "return_layout",
            "return_artifacts",
        ],
        AiccCall::VisionCaption(_) => &["style", "language", "n"],
        AiccCall::VisionDetect(_) => &["classes", "score_threshold", "bbox_spec"],
        AiccCall::VisionSegment(_) => &["mask_format", "return_bitmap_mask"],
        AiccCall::AudioTextToSpeech(_) => &["voice", "speed", "output"],
        AiccCall::AudioSpeechRecognition(_) => {
            &["language", "timestamps", "diarization", "output_formats"]
        }
        AiccCall::AudioMusic(_) => &[
            "duration_seconds",
            "instrumental",
            "lyrics",
            "seed",
            "output",
        ],
        AiccCall::AudioEnhance(_) => &["strength", "return_stems"],
        AiccCall::VideoTextToVideo(_) => &[
            "duration_seconds",
            "aspect_ratio",
            "resolution",
            "generate_audio",
            "seed",
            "output",
        ],
        AiccCall::VideoImageToVideo(_) => &["duration_seconds", "aspect_ratio", "resolution"],
        AiccCall::VideoToVideo(_) => &["preserve_motion", "time_range"],
        AiccCall::VideoExtend(_) => &["continuation_handle", "duration_seconds", "resolution"],
        AiccCall::VideoUpscale(_) => &["denoise", "sharpen", "output"],
        AiccCall::ComputerUse(_) => &[],
        AiccCall::RouteResolve(_) | AiccCall::HelperLlmChat(_) | AiccCall::HelperTextToImage(_) => {
            return Err(CallLoweringError::UnsupportedCanonicalCall(
                call.method().into(),
            ));
        }
    })
}

fn api_type_name(api_type: ApiType) -> &'static str {
    match api_type {
        ApiType::Llm => "llm",
        ApiType::EmbeddingText => "embedding.text",
        ApiType::EmbeddingMultimodal => "embedding.multimodal",
        ApiType::Rerank => "rerank",
        ApiType::ImageTextToImage => "image.txt2img",
        ApiType::ImageImageToImage => "image.img2img",
        ApiType::ImageInpaint => "image.inpaint",
        ApiType::ImageUpscale => "image.upscale",
        ApiType::ImageBackgroundRemove => "image.bg_remove",
        ApiType::VisionOcr => "vision.ocr",
        ApiType::VisionCaption => "vision.caption",
        ApiType::VisionDetect => "vision.detect",
        ApiType::VisionSegment => "vision.segment",
        ApiType::AudioTextToSpeech => "audio.tts",
        ApiType::AudioSpeechRecognition => "audio.asr",
        ApiType::AudioMusic => "audio.music",
        ApiType::AudioEnhance => "audio.enhance",
        ApiType::VideoTextToVideo => "video.txt2video",
        ApiType::VideoImageToVideo => "video.img2video",
        ApiType::VideoToVideo => "video.video2video",
        ApiType::VideoExtend => "video.extend",
        ApiType::VideoUpscale => "video.upscale",
        ApiType::AgentComputerUse => "agent.computer_use",
    }
}

fn merge_user_options(target: &mut Value, canonical: &Value, keys: &[&str]) {
    let (Some(target), Some(canonical)) = (target.as_object_mut(), canonical.as_object()) else {
        return;
    };
    for key in keys {
        if let Some(value) = canonical.get(*key) {
            match target.get_mut(*key) {
                Some(existing) if existing.is_object() && value.is_object() => {
                    merge_overwrite(existing, value);
                }
                _ => {
                    target.insert((*key).into(), value.clone());
                }
            }
        }
    }
}

fn merge_overwrite(target: &mut Value, overlay: &Value) {
    match (target, overlay) {
        (Value::Object(target), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match target.get_mut(key) {
                    Some(existing) if existing.is_object() && value.is_object() => {
                        merge_overwrite(existing, value);
                    }
                    _ => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, overlay) => *target = overlay.clone(),
    }
}

fn fill_defaults(target: &mut Value, defaults: &Value) {
    if let (Value::Object(target), Value::Object(defaults)) = (target, defaults) {
        for (key, value) in defaults {
            match target.get_mut(key) {
                Some(existing) if existing.is_object() && value.is_object() => {
                    fill_defaults(existing, value);
                }
                Some(_) => {}
                None => {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

fn remove_pointer(value: &mut Value, pointer: &str) -> Result<(), CallLoweringError> {
    if pointer.is_empty() || !pointer.starts_with('/') {
        return Err(CallLoweringError::InvalidRule(format!(
            "invalid normalized JSON Pointer `{pointer}`"
        )));
    }
    let mut parts = pointer[1..]
        .split('/')
        .map(unescape_pointer)
        .collect::<Result<Vec<_>, _>>()?;
    let last = parts
        .pop()
        .ok_or_else(|| CallLoweringError::InvalidRule("empty remove pointer".into()))?;
    let mut parent = value;
    for part in parts {
        parent = match parent {
            Value::Object(object) => match object.get_mut(&part) {
                Some(value) => value,
                None => return Ok(()),
            },
            Value::Array(array) => {
                match part
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| array.get_mut(index))
                {
                    Some(value) => value,
                    None => return Ok(()),
                }
            }
            _ => return Ok(()),
        };
    }
    match parent {
        Value::Object(object) => {
            object.remove(&last);
        }
        Value::Array(array) => {
            if let Ok(index) = last.parse::<usize>() {
                if index < array.len() {
                    array.remove(index);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn unescape_pointer(value: &str) -> Result<String, CallLoweringError> {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            output.push(character);
            continue;
        }
        match chars.next() {
            Some('0') => output.push('~'),
            Some('1') => output.push('/'),
            _ => {
                return Err(CallLoweringError::InvalidRule(
                    "invalid JSON Pointer escape".into(),
                ));
            }
        }
    }
    Ok(output)
}

fn request_match_context(api_type: &str, operation: &str, normalized: &Value) -> MatchContext {
    let mut context = MatchContext::from([
        ("api_type".into(), Value::String(api_type.into())),
        ("operation".into(), Value::String(operation.into())),
    ]);
    flatten_match_dimensions(normalized, "", &mut context);
    context
}

fn flatten_match_dimensions(value: &Value, pointer: &str, context: &mut MatchContext) {
    if !pointer.is_empty() {
        context.insert(pointer.into(), value.clone());
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                flatten_match_dimensions(
                    value,
                    &format!("{pointer}/{}", escape_pointer(key)),
                    context,
                );
            }
        }
        Value::Array(array) => {
            for (index, value) in array.iter().enumerate() {
                flatten_match_dimensions(value, &format!("{pointer}/{index}"), context);
            }
        }
        _ => {}
    }
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn rewrite_canonical_options(
    canonical: &Value,
    normalized: &Value,
    keys: &[&str],
) -> Result<Value, CallLoweringError> {
    let mut rewritten = canonical.clone();
    let object = rewritten.as_object_mut().ok_or_else(|| {
        CallLoweringError::InvalidCanonicalRequest("request must serialize as an object".into())
    })?;
    let normalized = normalized.as_object().ok_or_else(|| {
        CallLoweringError::InvalidRule("normalized options must be an object".into())
    })?;
    for key in keys {
        object.remove(*key);
        if let Some(value) = normalized.get(*key) {
            object.insert((*key).into(), value.clone());
        }
    }
    Ok(rewritten)
}

fn provider_parameters(normalized: &Value, canonical_keys: &[&str]) -> BTreeMap<String, Value> {
    normalized
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !canonical_keys.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn collect_resource_requirements(value: &Value) -> Vec<ResourceRequirement> {
    fn visit(value: &Value, pointer: &str, output: &mut Vec<ResourceRequirement>) {
        if value.get("kind").and_then(Value::as_str).is_some() {
            if let Ok(resource) = serde_json::from_value::<ResourceRef>(value.clone()) {
                output.push(ResourceRequirement {
                    request_pointer: pointer.into(),
                    resource,
                });
                return;
            }
        }
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    visit(value, &format!("{pointer}/{}", escape_pointer(key)), output);
                }
            }
            Value::Array(array) => {
                for (index, value) in array.iter().enumerate() {
                    visit(value, &format!("{pointer}/{index}"), output);
                }
            }
            _ => {}
        }
    }
    let mut output = Vec::new();
    visit(value, "", &mut output);
    output
}

fn resolve_pricing(
    target: Option<ResolvedPricing>,
    provider_pricing: Option<Pricing>,
    matched_amount: Option<f64>,
    estimated_cost_usd: Option<f64>,
) -> ResolvedPricing {
    if let Some(target) = target {
        return target;
    }
    if let Some(pricing) = provider_pricing {
        return ResolvedPricing {
            source: PricingSource::ProviderRules,
            pricing: Some(pricing),
            matched_amount,
            estimated_cost_usd,
        };
    }
    ResolvedPricing {
        source: PricingSource::RouteEstimate,
        pricing: None,
        matched_amount: None,
        estimated_cost_usd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        CatalogBuildOptions, CatalogDocuments, CatalogKind, CurrentCatalogFile, ModelDriverCatalog,
        ProviderRulesCatalog, ResolvedProviderConfiguration,
    };
    use crate::protocol::{openai_responses_adapter, CodecRegistry};
    use crate::provider::claude_messages_adapter;
    use crate::routing::{RouteModelKind, RoutingTrace, ScoreBreakdown, UserFacingRouteSummary};
    use buckyos_api::{AiMessage, AiRole, LlmChatInvokeRequest};
    use serde_json::json;
    use std::time::Duration;

    fn catalog() -> CatalogSnapshot {
        catalog_with_provider_variant(true)
    }

    fn catalog_with_provider_variant(include_provider_variant: bool) -> CatalogSnapshot {
        let model: ModelDriverCatalog = serde_json::from_value(json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": "openai",
            "revision_seq": 7,
            "models": [{"id": "gpt-5.2", "api_types": ["llm"]}],
            "defaults": {},
            "variants": [{"name": "reasoning-high", "match": "gpt-*"}],
            "version_rules": []
        }))
        .unwrap();
        let mut rules_json = json!({
            "format": "buckyos.aicc.provider-rules-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "revision_seq": 9,
            "provider_profile_id": "openai",
            "models": [{
                "id": "gpt-5.2",
                "operations": {
                    "llm": "responses.create",
                    "chat.completions.create": "responses.create"
                },
                "provider_options": {
                    "reasoning": {"effort": "minimal"},
                    "service_tier": "auto"
                },
                "request_rules": [
                    {"defaults": {"temperature": 0.2, "top_p": 0.8, "max_output_tokens": 100}},
                    {
                        "when": {"/reasoning/effort": {"not": "none"}},
                        "set": {"service_tier": "priority"},
                        "remove": ["/temperature", "/top_p"]
                    }
                ],
                "pricing": {"currency": "USD", "unit": "request", "amount": 0.01}
            }],
            "patterns": [],
            "variants": [{
                "model_driver": "openai",
                "variant": "reasoning-high",
                "match": {"provider_model_id": "gpt-*", "variant": "reasoning-high"},
                "provider_options": {"reasoning": {"effort": "high"}}
            }]
        });
        if !include_provider_variant {
            rules_json["variants"] = json!([]);
        }
        let rules: ProviderRulesCatalog = serde_json::from_value(rules_json).unwrap();
        CatalogSnapshot::build(
            11,
            CatalogDocuments {
                model_drivers: vec![model],
                provider_rules: vec![rules],
                known_providers: Vec::new(),
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap()
    }

    fn codecs() -> CodecRegistry {
        let mut registry = CodecRegistry::default();
        let (descriptor, codecs) = openai_responses_adapter();
        registry.register_codecs(descriptor, codecs).unwrap();
        registry
    }

    fn all_codecs() -> CodecRegistry {
        use crate::protocol::{
            fal_queue_adapter, gemini_interactions_adapter, glm_chat_adapter, kimi_chat_adapter,
            minimax_messages_adapter, openai_chat_completions_adapter,
            openai_responses_compatible_adapters, openrouter_chat_adapter,
        };
        use crate::provider::register_sn_openai_adapter;

        let mut registry = CodecRegistry::default();
        let (responses, registration) = openai_responses_adapter();
        registry.register_codecs(responses, registration).unwrap();
        for (descriptor, registration) in openai_responses_compatible_adapters().unwrap() {
            registry.register_derived(descriptor, registration).unwrap();
        }
        register_sn_openai_adapter(&mut registry).unwrap();

        let (claude, registration) = claude_messages_adapter();
        registry.register_codecs(claude, registration).unwrap();
        let (minimax, registration) = minimax_messages_adapter();
        registry.register_derived(minimax, registration).unwrap();

        let (gemini, registration) = gemini_interactions_adapter();
        registry.register_codecs(gemini, registration).unwrap();
        let (chat, registration) = openai_chat_completions_adapter();
        registry.register_codecs(chat, registration).unwrap();
        for (descriptor, registration) in [
            openrouter_chat_adapter(),
            kimi_chat_adapter(),
            glm_chat_adapter(),
        ] {
            registry.register_derived(descriptor, registration).unwrap();
        }
        let (fal, registration) = fal_queue_adapter();
        registry.register_codecs(fal, registration).unwrap();
        registry
    }

    fn metadata_snapshot() -> CatalogSnapshot {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("driver_metadata");
        let mut files = Vec::new();
        for (directory, kind) in [
            ("models", CatalogKind::ModelDriver),
            ("providers", CatalogKind::ProviderRules),
            ("known-providers", CatalogKind::KnownProvider),
        ] {
            let mut paths = std::fs::read_dir(root.join(directory))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "json")
                })
                .collect::<Vec<_>>();
            paths.sort();
            files.extend(paths.into_iter().map(|path| CurrentCatalogFile {
                kind,
                contents: std::fs::read(path).unwrap(),
            }));
        }
        CatalogSnapshot::from_current_files(1, files, &CatalogBuildOptions::default()).unwrap()
    }

    fn built_in_rules() -> Vec<(ResolvedProviderConfiguration, ProviderRulesCatalog)> {
        let catalog = metadata_snapshot();
        catalog
            .known_providers()
            .map(|known_provider| {
                let configuration = catalog
                    .resolve_provider_configuration(&known_provider.provider_profile_id)
                    .unwrap();
                let rules = catalog
                    .provider_rules(&configuration.provider_rules_id)
                    .unwrap()
                    .clone();
                (configuration, rules)
            })
            .collect()
    }

    fn api_type_from_mapping_key(key: &str) -> ApiType {
        match key {
            "chat.completions.create" => ApiType::Llm,
            "images.generate" => ApiType::ImageTextToImage,
            key => serde_json::from_value(Value::String(key.into())).unwrap(),
        }
    }

    fn decision(exact_model: &str) -> RouteDecision {
        let selected = SelectedRoute {
            exact_model: exact_model.into(),
            model_uid: "openai:gpt-5.2:openai-responses:reasoning-high".into(),
            provider_instance_name: "openai-primary".into(),
            provider_profile_id: "openai".into(),
            protocol_adapter_id: "openai-responses".into(),
            model_driver_id: "openai".into(),
            origin_model_id: "gpt-5.2".into(),
            provider_model_id: "gpt-5.2".into(),
            operation: "responses.create".into(),
            inventory_revision: "inventory-3".into(),
            enabled_capabilities: vec!["reasoning".into()],
            disabled_capabilities: Vec::new(),
            estimated_cost_usd: Some(0.01),
            final_score: 1.0,
        };
        RouteDecision {
            selected,
            fallback_candidates: Vec::new(),
            trace: RoutingTrace {
                trace_id: "trace-1".into(),
                request_id: "request-1".into(),
                api_type: "llm".into(),
                requested_model: exact_model.into(),
                requested_model_type: RouteModelKind::Exact,
                resolved_logical_path: None,
                selected_exact_model: exact_model.into(),
                selected_provider_instance_name: "openai-primary".into(),
                candidate_count_before_filter: 1,
                candidate_count_after_filter: 1,
                filtered_candidates: Vec::new(),
                ranked_candidates: Vec::new(),
                fallback_applied: false,
                fallback_chain: Vec::new(),
                scheduler_profile: "balanced".into(),
                score_breakdown: ScoreBreakdown {
                    cost: 0.0,
                    latency: 0.0,
                    reliability: 0.0,
                    quality: 0.0,
                    preference: 0.0,
                    cache: 0.0,
                    local: 0.0,
                    final_score: 1.0,
                },
                estimated_cost_usd: Some(0.01),
                runtime_failover_count: 0,
                logical_item_sources: Vec::new(),
                logical_admission: Vec::new(),
                disabled_capabilities: Vec::new(),
                user_summary: UserFacingRouteSummary {
                    display_name: "gpt-5.2".into(),
                    model_family: "openai".into(),
                    provider_origin: "openai".into(),
                    reason_short: "selected".into(),
                    was_fallback: false,
                    was_failover: false,
                },
            },
        }
    }

    fn target(secret: &str) -> ProviderCallTarget {
        ProviderCallTarget {
            provider_rules_id: "openai".into(),
            base_url: "https://api.openai.test/v1".into(),
            credential: ResolvedCredential::bearer("secret://openai/main", secret).unwrap(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 1024 * 1024,
            },
            pricing: None,
            match_dimensions: MatchContext::new(),
        }
    }

    fn call() -> AiccCall {
        let mut request = LlmChatInvokeRequest::new(
            "gpt-5.2:reasoning-high@openai-primary",
            vec![AiMessage::text(AiRole::User, "hello")],
        );
        request.temperature = Some(0.7);
        request.top_p = Some(0.9);
        request.max_output_tokens = Some(200);
        AiccCall::ChatCompletionsCreate(request)
    }

    #[test]
    fn golden_variant_lowering_obeys_operation_and_parameter_precedence() {
        let catalog = catalog();
        let codecs = codecs();
        let resolver = CallResolver::new(&catalog, &codecs);
        let call = call();
        let lowered = resolver
            .lower(
                &decision(call_exact_model(&call).unwrap()),
                &call,
                target("credential-secret"),
            )
            .unwrap();

        assert_eq!(lowered.operation, "responses.create");
        assert_eq!(lowered.variant.as_deref(), Some("reasoning-high"));
        assert_eq!(
            lowered.input.resolved_parameters,
            BTreeMap::from([
                ("provider_model_id".into(), json!("gpt-5.2")),
                ("reasoning".into(), json!({"effort": "high"})),
                ("service_tier".into(), json!("priority")),
            ])
        );
        let AiccCall::ChatCompletionsCreate(rewritten) = &lowered.input.canonical_request else {
            panic!("expected chat request")
        };
        assert_eq!(rewritten.temperature, None);
        assert_eq!(rewritten.top_p, None);
        assert_eq!(rewritten.max_output_tokens, Some(200));
        assert_eq!(lowered.pricing.source, PricingSource::ProviderRules);
        assert_eq!(lowered.pricing.matched_amount, Some(0.01));
        assert_eq!(lowered.revisions.catalog_target_seq, 11);
        let golden = serde_json::to_value(lowered.deterministic_view()).unwrap();
        assert_eq!(golden["operation"], "responses.create");
        assert_eq!(golden["credential_kind"], "bearer");
        assert!(!golden.to_string().contains("credential-secret"));
    }

    #[test]
    fn identical_inputs_produce_identical_safe_golden_output() {
        let catalog = catalog();
        let codecs = codecs();
        let resolver = CallResolver::new(&catalog, &codecs);
        let call = call();
        let route = decision(call_exact_model(&call).unwrap());
        let first = resolver
            .lower(&route, &call, target("same-secret"))
            .unwrap();
        let second = resolver
            .lower(&route, &call, target("same-secret"))
            .unwrap();
        assert_eq!(
            serde_json::to_value(first.deterministic_view()).unwrap(),
            serde_json::to_value(second.deterministic_view()).unwrap()
        );
    }

    #[test]
    fn missing_provider_variant_fails_before_protocol_execution() {
        let catalog = catalog();
        let codecs = codecs();
        let resolver = CallResolver::new(&catalog, &codecs);
        let call = AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
            "gpt-5.2:unknown@openai-primary",
            Vec::new(),
        ));
        let error = resolver
            .lower(
                &decision(call_exact_model(&call).unwrap()),
                &call,
                target("secret"),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CallLoweringError::MissingModelVariant { .. }
        ));
    }

    #[test]
    fn model_variant_requires_provider_lowering_coverage() {
        let catalog = catalog_with_provider_variant(false);
        let codecs = codecs();
        let resolver = CallResolver::new(&catalog, &codecs);
        let call = call();
        let error = resolver
            .lower(
                &decision(call_exact_model(&call).unwrap()),
                &call,
                target("secret"),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CallLoweringError::MissingProviderVariant { .. }
        ));
    }

    #[test]
    fn resource_discovery_is_pointer_stable_and_debug_is_redacted() {
        let value = json!({
            "images": [
                {"kind": "url", "url": "https://example.test/input.png"},
                {"kind": "base64", "mime": "image/png", "data_base64": "sensitive-bytes"}
            ]
        });
        let requirements = collect_resource_requirements(&value);
        assert_eq!(
            requirements
                .iter()
                .map(|requirement| requirement.request_pointer.as_str())
                .collect::<Vec<_>>(),
            vec!["/images/0", "/images/1"]
        );
        let rendered = format!("{:?}", target("credential-secret"));
        assert!(!rendered.contains("credential-secret"));
    }

    #[test]
    fn operation_precedence_is_method_then_api_type_then_unique_adapter_default() {
        let codecs = codecs();
        let mappings = BTreeMap::from([
            ("images.generate".into(), "images.generate".into()),
            ("image.txt2img".into(), "responses.create".into()),
        ]);
        assert_eq!(
            select_operation(
                &codecs,
                "openai-responses",
                &mappings,
                ApiType::ImageTextToImage,
                "images.generate",
            )
            .unwrap(),
            "images.generate"
        );
        let api_only = BTreeMap::from([("image.txt2img".into(), "responses.create".into())]);
        assert_eq!(
            select_operation(
                &codecs,
                "openai-responses",
                &api_only,
                ApiType::ImageTextToImage,
                "images.generate",
            )
            .unwrap(),
            "responses.create"
        );
        assert!(matches!(
            select_operation(
                &codecs,
                "openai-responses",
                &BTreeMap::new(),
                ApiType::ImageTextToImage,
                "images.generate",
            ),
            Err(CallLoweringError::AmbiguousDefaultOperation { .. })
        ));
        assert_eq!(
            select_operation(
                &codecs,
                "openai-responses",
                &BTreeMap::new(),
                ApiType::Llm,
                "chat.completions.create",
            )
            .unwrap(),
            "responses.create"
        );
    }

    #[test]
    fn every_builtin_provider_operation_has_a_golden_lowering_binding() {
        let codecs = all_codecs();
        let providers = built_in_rules();
        let actual_profiles = providers
            .iter()
            .map(|(profile, _)| profile.provider_profile_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            actual_profiles,
            std::collections::BTreeSet::from([
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

        let mut golden = Vec::new();
        for (profile, rules) in providers {
            let mappings = rules
                .models
                .iter()
                .map(|rule| &rule.operations)
                .chain(rules.patterns.iter().map(|rule| &rule.operations));
            for mappings in mappings {
                for (key, expected_operation) in mappings {
                    let api_type = api_type_from_mapping_key(key);
                    let actual = select_operation(
                        &codecs,
                        &profile.protocol_adapter_id,
                        mappings,
                        api_type,
                        api_type.typed_method(),
                    )
                    .unwrap();
                    assert_eq!(&actual, expected_operation);
                    golden.push(format!(
                        "{}|{}|{}|{}",
                        profile.provider_profile_id,
                        profile.protocol_adapter_id,
                        api_type_name(api_type),
                        actual
                    ));
                }
            }
        }
        golden.sort();
        golden.dedup();
        assert_eq!(golden.len(), 48);
        assert!(golden.contains(&"openai|openai-responses|llm|responses.create".into()));
        assert!(golden.contains(&"claude|claude-messages|llm|messages.create".into()));
        assert!(golden
            .contains(&"gemini|gemini-interactions|video.extend|models.predictLongRunning".into()));
        assert!(golden.contains(&"fal|fal-queue|image.upscale|queue.submit".into()));
        assert!(golden.contains(&"qwen|qwen-responses|llm|responses.create".into()));
        assert!(golden.contains(&"sn|sn-openai|llm|responses.create".into()));
    }
}
