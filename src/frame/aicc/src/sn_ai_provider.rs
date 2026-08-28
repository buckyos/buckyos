use crate::aicc::{
    emit_background_provider_result, provider_type_from_settings, redacted_json_log,
    AIComputeCenter, InvokeCtx, Provider, ProviderError, ProviderInstance, ProviderRefreshTask,
    ProviderStartResult, ResolvedRequest, TaskEventSink,
};
use crate::metadata_resolver::{
    driver_metadata_model_ids, driver_model_has_specific_metadata, max_driver_metadata_cost,
    resolve_driver_inventory, DriverModelResolveRequest,
};
use crate::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, ModelMetadata, PricingMode, ProviderInventory,
    ProviderOrigin, ProviderType, ProviderTypeTrustedSource, QuotaState,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    ai_methods, features, generate_sn_user_device_token, login_sn_user_by_device_token,
    value_to_object_map, AiArtifact, AiContent, AiCost, AiMessage, AiMethodRequest, AiResponse,
    AiRole, AiToolCall, AiToolResultContent, AiToolSpec, AiUsage, ResourceRef, RespFormat,
};
use image::imageops::FilterType;
use image::ImageFormat;
use log::{error, info, warn};
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::error::Error as _;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time;

const SN_AI_PROVIDER_SETTINGS_KEY: &str = "sn-ai-provider";
const SN_AI_PROVIDER_DRIVER: &str = "sn-ai-provider";
const DEFAULT_SN_AI_PROVIDER_BASE_URL: &str = "https://sn.buckyos.ai/api/v1/ai/";
const DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_INVENTORY_REFRESH_INTERVAL_SECS: u64 = 300;
const SN_VIDEO_POLL_INTERVAL: Duration = Duration::from_secs(2);
const SN_VIDEO_MAX_WAIT: Duration = Duration::from_secs(600);
const SN_LLM_OPTION_ALLOWLIST: &[&str] = &[
    "audio",
    "background",
    "conversation",
    "include",
    "instructions",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "parallel_tool_calls",
    "previous_response_id",
    "prompt",
    "reasoning",
    "service_tier",
    "store",
    "stream",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
    "verbosity",
];
const SN_FUNCTION_NAME_PATTERN: &str = "^[a-zA-Z0-9_-]+$";
const SN_BUILTIN_TOOL_TYPES: &[&str] = &[
    "web_search_preview",
    "file_search",
    "computer_use_preview",
    "image_generation",
    "code_interpreter",
    "mcp",
];
const SN_CONTROL_OPTION_KEYS: &[&str] = &[
    "owner_session_id",
    "root_id",
    "rootid",
    "session_id",
    "session_overlay",
];
const SN_IMAGE_OPTION_ALLOWLIST: &[&str] = &[
    "background",
    "n",
    "output_compression",
    "output_format",
    "quality",
    "response_format",
    "size",
    "style",
    "user",
];
const SN_IMAGE_INPUT_ALLOWLIST: &[&str] = &[
    "background",
    "n",
    "output_compression",
    "output_format",
    "prompt",
    "quality",
    "response_format",
    "size",
    "style",
    "user",
];
const SN_IMAGE_EDIT_OPTION_ALLOWLIST: &[&str] = &[
    "background",
    "n",
    "output_compression",
    "output_format",
    "quality",
    "size",
    "user",
];

fn valid_sn_function_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-')
}

fn validate_sn_function_name(raw_name: &str, field_path: &str) -> Result<String, ProviderError> {
    let name = raw_name.trim();
    if !valid_sn_function_name(name) {
        return Err(ProviderError::fatal(format!(
            "{} is invalid; expected pattern '{}'",
            field_path, SN_FUNCTION_NAME_PATTERN
        )));
    }
    Ok(name.to_string())
}

fn default_sn_tool_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true,
    })
}

fn ensure_sn_object_parameters(parameters: &mut Value) {
    let Some(map) = parameters.as_object_mut() else {
        return;
    };
    map.entry("type".to_string())
        .or_insert_with(|| Value::String("object".to_string()));
}

fn normalize_sn_function_tool(
    tool: &Map<String, Value>,
    idx: usize,
) -> Result<Value, ProviderError> {
    let (name_path, raw_name, description, parameters, strict) =
        if let Some(function) = tool.get("function").and_then(Value::as_object) {
            let Some(raw_name) = function.get("name").and_then(Value::as_str) else {
                return Err(ProviderError::fatal(format!(
                    "tools[{}].function.name is required",
                    idx
                )));
            };
            (
                format!("tools[{}].function.name", idx),
                raw_name,
                function.get("description").cloned(),
                function
                    .get("parameters")
                    .or_else(|| function.get("args_json_schema"))
                    .cloned()
                    .unwrap_or_else(default_sn_tool_parameters),
                function
                    .get("strict")
                    .cloned()
                    .or_else(|| tool.get("strict").cloned()),
            )
        } else {
            let Some(raw_name) = tool.get("name").and_then(Value::as_str) else {
                return Err(ProviderError::fatal(format!(
                    "tools[{}].name is required when tools[{}].type=function",
                    idx, idx
                )));
            };
            (
                format!("tools[{}].name", idx),
                raw_name,
                tool.get("description").cloned(),
                tool.get("parameters")
                    .or_else(|| tool.get("args_json_schema"))
                    .cloned()
                    .unwrap_or_else(default_sn_tool_parameters),
                tool.get("strict").cloned(),
            )
        };

    let name = validate_sn_function_name(raw_name, &name_path)?;
    if !parameters.is_object() {
        return Err(ProviderError::fatal(format!(
            "tools[{}].parameters must be an object",
            idx
        )));
    }
    let mut parameters = parameters;
    ensure_sn_object_parameters(&mut parameters);

    let mut normalized = Map::new();
    normalized.insert("type".to_string(), Value::String("function".to_string()));
    normalized.insert("name".to_string(), Value::String(name));
    if let Some(description) = description.and_then(|value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }) {
        normalized.insert("description".to_string(), Value::String(description));
    }
    normalized.insert("parameters".to_string(), parameters);
    if let Some(strict) = strict.and_then(|value| value.as_bool()) {
        normalized.insert("strict".to_string(), Value::Bool(strict));
    }
    Ok(Value::Object(normalized))
}

fn normalize_sn_internal_tool(
    tool: &Map<String, Value>,
    idx: usize,
) -> Result<Value, ProviderError> {
    let Some(raw_name) = tool.get("name").and_then(Value::as_str) else {
        return Err(ProviderError::fatal(format!(
            "tools[{}].name is required for internal tool format",
            idx
        )));
    };
    let name = validate_sn_function_name(raw_name, &format!("tools[{}].name", idx))?;
    let mut parameters = tool
        .get("args_schema")
        .or_else(|| tool.get("args_json_schema"))
        .cloned()
        .unwrap_or_else(default_sn_tool_parameters);
    if !parameters.is_object() {
        return Err(ProviderError::fatal(format!(
            "tools[{}].args_schema must be an object",
            idx
        )));
    }
    ensure_sn_object_parameters(&mut parameters);

    let mut normalized = Map::new();
    normalized.insert("type".to_string(), Value::String("function".to_string()));
    normalized.insert("name".to_string(), Value::String(name));
    if let Some(description) = tool
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        normalized.insert(
            "description".to_string(),
            Value::String(description.to_string()),
        );
    }
    normalized.insert("parameters".to_string(), parameters);
    Ok(Value::Object(normalized))
}

fn normalize_sn_tools(tools: &Value) -> Result<Value, ProviderError> {
    let Some(items) = tools.as_array() else {
        return Err(ProviderError::fatal("tools must be an array"));
    };
    let mut normalized = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(tool) = item.as_object() else {
            return Err(ProviderError::fatal(format!(
                "tools[{}] must be an object",
                idx
            )));
        };
        let value = match tool.get("type").and_then(Value::as_str) {
            Some("function") => normalize_sn_function_tool(tool, idx)?,
            Some(tool_type) => {
                let normalized_type = if tool_type == "web_search" {
                    "web_search_preview"
                } else {
                    tool_type
                };
                if !SN_BUILTIN_TOOL_TYPES.contains(&normalized_type) {
                    return Err(ProviderError::fatal(format!(
                        "tools[{}].type '{}' is unsupported",
                        idx, tool_type
                    )));
                }
                let mut builtin = tool.clone();
                builtin.insert(
                    "type".to_string(),
                    Value::String(normalized_type.to_string()),
                );
                Value::Object(builtin)
            }
            None => normalize_sn_internal_tool(tool, idx)?,
        };
        normalized.push(value);
    }
    Ok(Value::Array(normalized))
}

fn merge_sn_tool_specs(
    target: &mut Map<String, Value>,
    tool_specs: &[AiToolSpec],
) -> Result<(), ProviderError> {
    if tool_specs.is_empty() {
        return Ok(());
    }
    let tools = serde_json::to_value(tool_specs).map_err(|err| {
        ProviderError::fatal(format!("failed to serialize payload.tool_calls: {err}"))
    })?;
    target.insert("tools".to_string(), normalize_sn_tools(&tools)?);
    Ok(())
}

fn set_sn_text_format(target: &mut Map<String, Value>, format: Value) -> Result<(), ProviderError> {
    match target.entry("text".to_string()) {
        serde_json::map::Entry::Vacant(entry) => {
            entry.insert(json!({ "format": format }));
        }
        serde_json::map::Entry::Occupied(mut entry) => {
            let Some(text) = entry.get_mut().as_object_mut() else {
                return Err(ProviderError::fatal("text option must be an object"));
            };
            text.insert("format".to_string(), format);
        }
    }
    Ok(())
}

fn has_sn_text_format(target: &Map<String, Value>) -> bool {
    target
        .get("text")
        .and_then(Value::as_object)
        .and_then(|text| text.get("format"))
        .is_some()
}

fn sn_response_format(response_format: &Value) -> Result<Value, ProviderError> {
    let Some(format) = response_format.as_object() else {
        return Err(ProviderError::fatal("response_format must be an object"));
    };
    let format_type = format.get("type").and_then(Value::as_str).unwrap_or("text");
    if format_type == "json_schema" {
        let schema_holder = format
            .get("json_schema")
            .and_then(Value::as_object)
            .unwrap_or(format);
        let schema = schema_holder.get("schema").cloned().ok_or_else(|| {
            ProviderError::fatal("response_format schema is required for json_schema")
        })?;
        if !schema.is_object() {
            return Err(ProviderError::fatal(
                "response_format schema must be an object for json_schema",
            ));
        }
        let name = schema_holder
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("aicc_response");
        return Ok(json!({
            "type": "json_schema",
            "name": name,
            "schema": schema,
            "strict": schema_holder
                .get("strict")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        }));
    }
    if format_type == "json_object" || format_type == "text" {
        return Ok(json!({ "type": format_type }));
    }
    Ok(response_format.clone())
}

fn merge_sn_reasoning(
    target: &mut Map<String, Value>,
    effort: &Value,
) -> Result<(), ProviderError> {
    let Some(effort) = effort.as_str() else {
        return Err(ProviderError::fatal("reasoning_effort must be a string"));
    };
    match target.entry("reasoning".to_string()) {
        serde_json::map::Entry::Vacant(entry) => {
            entry.insert(json!({ "effort": effort }));
        }
        serde_json::map::Entry::Occupied(mut entry) => {
            let Some(reasoning) = entry.get_mut().as_object_mut() else {
                return Err(ProviderError::fatal("reasoning option must be an object"));
            };
            reasoning.insert("effort".to_string(), Value::String(effort.to_string()));
        }
    }
    Ok(())
}

fn merge_sn_llm_options(
    target: &mut Map<String, Value>,
    options: &Value,
) -> Result<Vec<String>, ProviderError> {
    let Some(options) = options.as_object() else {
        return Ok(vec![]);
    };
    let mut ignored = vec![];
    for (key, value) in options {
        if key == "model" || key == "messages" || key == "input" {
            continue;
        }
        if SN_CONTROL_OPTION_KEYS.contains(&key.as_str()) {
            continue;
        }
        if key == "protocol" || key == "process_name" || key == "tool_messages" {
            ignored.push(key.clone());
            continue;
        }
        if key == "max_tokens" || key == "max_completion_tokens" {
            if !target.contains_key("max_output_tokens") {
                target.insert("max_output_tokens".to_string(), value.clone());
            }
            continue;
        }
        if key == "response_schema" {
            if has_sn_text_format(target) {
                ignored.push(key.clone());
            } else if value.is_object() {
                set_sn_text_format(
                    target,
                    json!({
                        "type": "json_schema",
                        "name": "aicc_response",
                        "schema": value,
                        "strict": true
                    }),
                )?;
            } else {
                return Err(ProviderError::fatal("response_schema must be an object"));
            }
            continue;
        }
        if key == "response_format" {
            if has_sn_text_format(target) {
                ignored.push(key.clone());
            } else {
                set_sn_text_format(target, sn_response_format(value)?)?;
            }
            continue;
        }
        if key == "reasoning_effort" {
            merge_sn_reasoning(target, value)?;
            continue;
        }
        if key == "tools" {
            target.insert("tools".to_string(), normalize_sn_tools(value)?);
            continue;
        }
        if !SN_LLM_OPTION_ALLOWLIST.contains(&key.as_str()) {
            ignored.push(key.clone());
            continue;
        }
        target.insert(key.clone(), value.clone());
    }
    Ok(ignored)
}

fn apply_sn_model_defaults(target: &mut Map<String, Value>, provider_model: &str) {
    let model = provider_model.trim().to_ascii_lowercase();
    if !(model.starts_with("gpt-5-nano") || model.starts_with("gpt-5-nono")) {
        return;
    }
    target
        .entry("reasoning".to_string())
        .or_insert_with(|| json!({ "effort": "minimal" }));
    if target.contains_key("verbosity") {
        return;
    }
    match target.entry("text".to_string()) {
        serde_json::map::Entry::Vacant(entry) => {
            entry.insert(json!({ "verbosity": "low" }));
        }
        serde_json::map::Entry::Occupied(mut entry) => {
            if let Some(text) = entry.get_mut().as_object_mut() {
                text.entry("verbosity".to_string())
                    .or_insert_with(|| Value::String("low".to_string()));
            }
        }
    }
}

fn merge_sn_required_response_format(target: &mut Map<String, Value>, req: &AiMethodRequest) {
    if has_sn_text_format(target) {
        return;
    }
    if req.requirements.resp_format == RespFormat::Json
        || req.requirements.requires_feature(features::JSON_OUTPUT)
    {
        let _ = set_sn_text_format(target, json!({ "type": "json_object" }));
    }
}

fn merge_sn_required_tools(
    target: &mut Map<String, Value>,
    req: &AiMethodRequest,
) -> Result<(), ProviderError> {
    if !req.requirements.requires_feature(features::WEB_SEARCH) {
        return Ok(());
    }
    let web_search = json!({ "type": "web_search_preview" });
    if let Some(tools) = target.get_mut("tools") {
        let Some(tools) = tools.as_array_mut() else {
            return Err(ProviderError::fatal(
                "tools must be an array when enabling web_search",
            ));
        };
        if !tools.iter().any(|tool| {
            matches!(
                tool.get("type").and_then(Value::as_str),
                Some("web_search" | "web_search_preview")
            )
        }) {
            tools.push(web_search);
        }
    } else {
        target.insert("tools".to_string(), Value::Array(vec![web_search]));
    }
    Ok(())
}

fn normalize_sn_web_search_reasoning(target: &mut Map<String, Value>) -> bool {
    let has_web_search = target
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|tool| {
                matches!(
                    tool.get("type").and_then(Value::as_str),
                    Some("web_search" | "web_search_preview")
                )
            })
        });
    if !has_web_search {
        return false;
    }
    let Some(effort) = target
        .get_mut("reasoning")
        .and_then(Value::as_object_mut)
        .and_then(|reasoning| reasoning.get_mut("effort"))
    else {
        return false;
    };
    if effort.as_str() != Some("minimal") {
        return false;
    }
    *effort = Value::String("low".to_string());
    true
}

fn strip_sn_sampling_options(target: &mut Map<String, Value>, provider_model: &str) -> Vec<String> {
    let model = provider_model.trim().to_ascii_lowercase();
    if !model.starts_with("gpt-5") {
        return vec![];
    }
    let is_old_gpt5 = model == "gpt-5"
        || model.starts_with("gpt-5-")
        || model.starts_with("gpt-5-mini")
        || model.starts_with("gpt-5-nano");
    let is_codex = model.contains("codex");
    let reasoning_effort = target
        .get("reasoning")
        .and_then(Value::as_object)
        .and_then(|reasoning| reasoning.get("effort"))
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    if !is_old_gpt5 && !is_codex && reasoning_effort.as_deref() == Some("none") {
        return vec![];
    }
    let mut removed = vec![];
    for key in ["temperature", "top_p", "logprobs", "top_logprobs"] {
        if target.remove(key).is_some() {
            removed.push(key.to_string());
        }
    }
    removed
}

#[derive(Debug, Deserialize, Default)]
struct SnAIProviderSettings {
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    instances: Vec<SettingsSnAIProviderInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsSnAIProviderInstanceConfig {
    #[serde(default = "default_instance_id", alias = "instance_id")]
    provider_instance_name: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default = "default_base_url")]
    base_url: String,
    login_url: String,
    user_name: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

#[derive(Debug, Clone)]
struct SnAIProviderInstanceConfig {
    provider_instance_name: String,
    provider_type: String,
    base_url: String,
    login_url: String,
    user_name: String,
    timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SnAIProvider {
    instance: ProviderInstance,
    inventory: Arc<RwLock<ProviderInventory>>,
    client: Client,
    base_url: String,
    login_url: String,
    user_name: String,
    provider_type: ProviderType,
    inventory_refresh_interval: Duration,
    refresh_task: Arc<Mutex<Option<Arc<ProviderRefreshTask>>>>,
    auth_session: Arc<AsyncMutex<Option<CachedSnSession>>>,
}

#[derive(Debug, Clone)]
struct CachedSnSession {
    session_token: String,
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct SnModelsResponse {
    #[serde(default)]
    data: Vec<SnModelEntry>,
}

#[derive(Debug, Deserialize)]
struct SnModelEntry {
    id: String,
}

fn default_enabled() -> bool {
    true
}

fn default_instance_id() -> String {
    "sn-ai-provider-default".to_string()
}

fn default_provider_type() -> String {
    "cloud_api".to_string()
}

fn default_base_url() -> String {
    DEFAULT_SN_AI_PROVIDER_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS
}

fn normalize_model_list(models: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for model in models {
        let model = model.trim();
        if !model.is_empty() && !normalized.iter().any(|item: &String| item == model) {
            normalized.push(model.to_string());
        }
    }
    normalized
}

fn configured_model_ids() -> Vec<String> {
    let mut models = Vec::new();
    for api_type in sn_supported_api_types() {
        for model in driver_metadata_model_ids(SN_AI_PROVIDER_DRIVER, api_type) {
            if !models.contains(&model) {
                models.push(model);
            }
        }
    }
    models
}

fn sn_supported_api_types() -> &'static [ApiType] {
    &[
        ApiType::Llm,
        ApiType::Embedding,
        ApiType::Rerank,
        ApiType::VisionOcr,
        ApiType::VisionCaption,
        ApiType::ImageTextToImage,
        ApiType::ImageToImage,
        ApiType::ImageInpaint,
        ApiType::AudioAsr,
        ApiType::AudioTts,
        ApiType::VideoTextToVideo,
        ApiType::VideoImageToVideo,
    ]
}

fn is_supported_sn_api_type(api_type: &ApiType) -> bool {
    sn_supported_api_types().contains(api_type)
}

fn sn_form_field(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn parse_sn_ai_provider_settings(settings: &Value) -> Result<Option<SnAIProviderSettings>> {
    let Some(raw_settings) = settings.get(SN_AI_PROVIDER_SETTINGS_KEY) else {
        return Ok(None);
    };
    if raw_settings.is_null() {
        return Ok(None);
    }

    let parsed = serde_json::from_value::<SnAIProviderSettings>(raw_settings.clone())
        .map_err(|err| anyhow!("failed to parse settings.sn-ai-provider: {}", err))?;
    if !parsed.enabled {
        return Ok(None);
    }

    Ok(Some(parsed))
}

fn build_sn_ai_provider_instances(
    settings: &SnAIProviderSettings,
) -> Result<Vec<SnAIProviderInstanceConfig>> {
    let raw_instances = settings.instances.clone();

    raw_instances
        .into_iter()
        .map(|raw_instance| {
            Ok(SnAIProviderInstanceConfig {
                provider_instance_name: raw_instance.provider_instance_name,
                provider_type: raw_instance.provider_type,
                base_url: raw_instance.base_url,
                login_url: raw_instance.login_url,
                user_name: raw_instance.user_name,
                timeout_ms: raw_instance.timeout_ms,
            })
        })
        .collect()
}

impl SnAIProvider {
    fn new(cfg: SnAIProviderInstanceConfig) -> Result<Self> {
        if cfg.login_url.trim().is_empty() {
            return Err(anyhow!("sn-ai-provider login_url is required"));
        }
        if cfg.user_name.trim().is_empty() {
            return Err(anyhow!("sn-ai-provider user_name is required"));
        }
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for sn-ai-provider")?;
        let provider_type = provider_type_from_settings(cfg.provider_type.as_str());
        let instance = ProviderInstance {
            provider_instance_name: cfg.provider_instance_name.clone(),
            provider_type: provider_type.clone(),
            provider_driver: SN_AI_PROVIDER_DRIVER.to_string(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };
        let models = configured_model_ids();
        let inventory = Self::build_inventory(
            cfg.provider_instance_name.as_str(),
            provider_type.clone(),
            models.as_slice(),
            Some("default-v1".to_string()),
        );
        Ok(Self {
            instance,
            inventory: Arc::new(RwLock::new(inventory)),
            client,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            login_url: cfg.login_url,
            user_name: cfg.user_name,
            provider_type,
            inventory_refresh_interval: Duration::from_secs(
                DEFAULT_INVENTORY_REFRESH_INTERVAL_SECS,
            ),
            refresh_task: Arc::new(Mutex::new(None)),
            auth_session: Arc::new(AsyncMutex::new(None)),
        })
    }

    fn start_inventory_refresh(self: Arc<Self>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }

        let (refresh_task, shutdown_rx) = ProviderRefreshTask::new();
        let existing = match self.refresh_task.lock() {
            Ok(mut current) => current.replace(refresh_task.clone()),
            Err(_) => {
                warn!(
                    "aicc.sn_ai_provider.inventory.refresh_task_lock_poisoned provider_instance_name={}",
                    self.instance.provider_instance_name
                );
                return;
            }
        };
        if let Some(existing) = existing {
            existing.shutdown();
        }
        let provider = Arc::downgrade(&self);
        let provider_instance_name = self.instance.provider_instance_name.clone();
        let refresh_interval = self.inventory_refresh_interval;
        tokio::spawn(async move {
            Self::run_inventory_refresh(
                provider,
                provider_instance_name,
                refresh_task,
                shutdown_rx,
                refresh_interval,
            )
            .await;
        });
    }

    async fn run_inventory_refresh(
        provider: Weak<Self>,
        provider_instance_name: String,
        refresh_task: Arc<ProviderRefreshTask>,
        mut shutdown_rx: watch::Receiver<bool>,
        refresh_interval: Duration,
    ) {
        if !refresh_task.try_start_request() {
            return;
        }
        let Some(current) = provider.upgrade() else {
            refresh_task.shutdown();
            return;
        };
        let initial_result = current.refresh_inventory_once().await;
        refresh_task.finish_request();
        drop(current);
        if let Err(err) = initial_result {
            warn!(
                    "aicc.sn_ai_provider.inventory.initial_refresh_failed provider_instance_name={} err={}",
                    provider_instance_name, err
                );
        }

        let mut interval = time::interval(refresh_interval);
        interval.tick().await;
        loop {
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
                _ = interval.tick() => {
                    if !refresh_task.try_start_request() {
                        return;
                    }
                    let Some(current) = provider.upgrade() else {
                        refresh_task.shutdown();
                        return;
                    };
                    let result = current.refresh_inventory_once().await;
                    refresh_task.finish_request();
                    drop(current);
                    if let Err(err) = result {
                    warn!(
                        "aicc.sn_ai_provider.inventory.refresh_failed provider_instance_name={} err={}",
                        provider_instance_name, err
                    );
                    }
                }
            }
        }
    }

    fn stop_inventory_refresh(&self) {
        if let Ok(mut current) = self.refresh_task.lock() {
            if let Some(task) = current.take() {
                task.shutdown();
            }
        }
    }

    fn build_inventory(
        provider_instance_name: &str,
        provider_type: ProviderType,
        models: &[String],
        revision: Option<String>,
    ) -> ProviderInventory {
        let requests = models
            .iter()
            .map(|model| DriverModelResolveRequest::new(model.clone(), vec![]))
            .collect::<Vec<_>>();
        let mut inventory = resolve_driver_inventory(
            provider_instance_name,
            provider_type,
            SN_AI_PROVIDER_DRIVER,
            requests.as_slice(),
            revision,
        );
        for model in &mut inventory.models {
            model.api_types.retain(is_supported_sn_api_type);
        }
        inventory.models.retain(|model| !model.api_types.is_empty());
        inventory
    }

    fn normalize_remote_provider_inventory(
        &self,
        inventory: ProviderInventory,
    ) -> ProviderInventory {
        let models = inventory
            .models
            .iter()
            .map(|model| {
                model
                    .provider_actual_model_id
                    .clone()
                    .unwrap_or_else(|| model.provider_model_id.clone())
            })
            .collect::<Vec<_>>();
        let mut normalized = Self::build_inventory(
            self.instance.provider_instance_name.as_str(),
            self.provider_type.clone(),
            models.as_slice(),
            inventory.inventory_revision.clone(),
        );
        normalized.version = inventory.version;
        normalized
    }

    fn build_inventory_from_remote_value(&self, body: Value) -> Result<ProviderInventory> {
        if body
            .get("models")
            .and_then(|value| value.as_array())
            .is_some()
        {
            let inventory = serde_json::from_value::<ProviderInventory>(body)
                .context("failed to parse sn-ai-provider inventory response")?;
            return Ok(self.normalize_remote_provider_inventory(inventory));
        }

        let mut model_ids = Vec::<String>::new();
        if let Some(items) = body.get("items").and_then(Value::as_array) {
            for item in items {
                if let Some(model) = item
                    .get("model")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    model_ids.push(model.to_string());
                }
            }
        } else {
            let response = serde_json::from_value::<SnModelsResponse>(body)
                .context("failed to parse sn-ai-provider models response")?;
            model_ids.extend(response.data.into_iter().map(|entry| entry.id));
        }

        let models = normalize_model_list(model_ids);
        Ok(Self::build_inventory(
            self.instance.provider_instance_name.as_str(),
            self.provider_type.clone(),
            models.as_slice(),
            Some(inventory_revision(models.as_slice())),
        ))
    }

    fn models_endpoint(&self) -> String {
        if self.base_url.to_ascii_lowercase().ends_with("/responses") {
            if let Some((prefix, _)) = self.base_url.rsplit_once('/') {
                return format!("{}/models", prefix.trim_end_matches('/'));
            }
        }
        format!("{}/models", self.base_url.trim_end_matches('/'))
    }

    fn responses_endpoint(&self) -> String {
        if self.base_url.to_ascii_lowercase().ends_with("/responses") {
            self.base_url.clone()
        } else {
            format!("{}/responses", self.base_url.trim_end_matches('/'))
        }
    }

    fn images_generations_endpoint(&self) -> String {
        self.litellm_endpoint("images/generations")
    }

    fn litellm_endpoint(&self, path: &str) -> String {
        let root = if self.base_url.to_ascii_lowercase().ends_with("/responses") {
            self.base_url
                .rsplit_once('/')
                .map(|(prefix, _)| prefix)
                .unwrap_or(self.base_url.as_str())
        } else {
            self.base_url.as_str()
        }
        .trim_end_matches('/');
        if root.to_ascii_lowercase().ends_with("/v1") {
            format!("{}/{}", root, path.trim_start_matches('/'))
        } else {
            format!("{}/v1/{}", root, path.trim_start_matches('/'))
        }
    }

    async fn refresh_inventory_once(&self) -> Result<ProviderInventory> {
        let url = self.models_endpoint();
        let response = self
            .send_authenticated(&InvokeCtx::default(), url.as_str(), |client, token| {
                client.get(url.as_str()).bearer_auth(token)
            })
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "sn-ai-provider inventory refresh failed status={} body={}",
                status,
                body
            ));
        }
        let body = response
            .json::<Value>()
            .await
            .context("failed to parse sn-ai-provider inventory response")?;
        let inventory = self.build_inventory_from_remote_value(body)?;
        {
            let mut current = self
                .inventory
                .write()
                .map_err(|_| anyhow!("sn-ai-provider inventory lock poisoned"))?;
            *current = inventory.clone();
        }
        info!(
            "aicc.sn_ai_provider.inventory.refreshed provider_instance_name={} models={}",
            self.instance.provider_instance_name,
            inventory.models.len()
        );
        Ok(inventory)
    }

    async fn build_auth_token(&self, _ctx: &InvokeCtx) -> Result<String, ProviderError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut cached = self.auth_session.lock().await;
        if let Some(session) = cached.as_ref() {
            if session.expires_at > now.saturating_add(30) {
                return Ok(session.session_token.clone());
            }
        }
        let device_token = generate_sn_user_device_token(self.user_name.as_str())
            .map_err(|err| ProviderError::fatal(err.to_string()))?;
        let session = login_sn_user_by_device_token(
            &self.client,
            self.login_url.as_str(),
            device_token.as_str(),
        )
        .await
        .map_err(|err| {
            if err.is_retryable() {
                ProviderError::retryable(err.to_string())
            } else {
                ProviderError::fatal(err.to_string())
            }
        })?;
        let session_token = session.session_token;
        *cached = Some(CachedSnSession {
            session_token: session_token.clone(),
            expires_at: now.saturating_add(session.expires_in),
        });
        Ok(session_token)
    }

    async fn invalidate_auth_token(&self, rejected_token: &str) {
        let mut cached = self.auth_session.lock().await;
        if cached
            .as_ref()
            .is_some_and(|session| session.session_token == rejected_token)
        {
            *cached = None;
        }
    }

    async fn send_authenticated(
        &self,
        ctx: &InvokeCtx,
        url: &str,
        build_request: impl Fn(&Client, &str) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, ProviderError> {
        for attempt in 0..=1 {
            let auth_token = self.build_auth_token(ctx).await?;
            let response = build_request(&self.client, auth_token.as_str())
                .send()
                .await
                .map_err(|err| {
                    let retryable = err.is_timeout() || err.is_connect();
                    error!(
                        "aicc.sn_ai_provider.http_send_failed provider_instance_name={} url={} retryable={} status={:?} err_chain={}",
                        self.instance.provider_instance_name,
                        url,
                        retryable,
                        err.status(),
                        Self::format_error_chain(&err)
                    );
                    if retryable {
                        ProviderError::retryable(format!("SN request failed: {err}"))
                    } else {
                        ProviderError::fatal(format!("SN request failed: {err}"))
                    }
                })?;
            if response.status() != StatusCode::UNAUTHORIZED || attempt == 1 {
                return Ok(response);
            }
            self.invalidate_auth_token(auth_token.as_str()).await;
        }
        unreachable!("authenticated request loop always returns")
    }

    fn format_error_chain(err: &reqwest::Error) -> String {
        let mut segments = vec![err.to_string()];
        let mut source = err.source();
        while let Some(cause) = source {
            segments.push(cause.to_string());
            source = cause.source();
        }
        segments.join(" | caused_by: ")
    }

    fn process_sse_event(
        payload: &str,
        final_response: &mut Option<Value>,
        accumulated_text: &mut String,
    ) -> Result<(), String> {
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return Ok(());
        }
        let event: Value = serde_json::from_str(payload)
            .map_err(|err| format!("invalid SSE event JSON: {err}; payload={payload}"))?;
        if let Some(response) = event.get("response").filter(|value| value.is_object()) {
            *final_response = Some(response.clone());
        }
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if (event_type == "response.output_text.delta" || event_type.ends_with("output_text.delta"))
            && event.get("delta").and_then(Value::as_str).is_some()
        {
            accumulated_text.push_str(
                event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
        }
        if let Some(choices) = event.get("choices").and_then(Value::as_array) {
            for choice in choices {
                if let Some(text) = choice.pointer("/delta/content").and_then(Value::as_str) {
                    accumulated_text.push_str(text);
                }
            }
        }
        Ok(())
    }

    fn parse_sse_response(raw: &str) -> Result<Value, String> {
        let mut final_response = None;
        let mut accumulated_text = String::new();
        let mut data_lines = vec![];
        for line in raw.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                if !data_lines.is_empty() {
                    Self::process_sse_event(
                        &data_lines.join("\n"),
                        &mut final_response,
                        &mut accumulated_text,
                    )?;
                    data_lines.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                data_lines.push(data.trim_start().to_string());
            }
        }
        if !data_lines.is_empty() {
            Self::process_sse_event(
                &data_lines.join("\n"),
                &mut final_response,
                &mut accumulated_text,
            )?;
        }
        if let Some(mut response) = final_response {
            if !accumulated_text.is_empty()
                && response
                    .get("output_text")
                    .and_then(Value::as_str)
                    .is_none_or(|text| text.trim().is_empty())
            {
                response
                    .as_object_mut()
                    .expect("SSE final response is an object")
                    .insert("output_text".to_string(), Value::String(accumulated_text));
            }
            return Ok(response);
        }
        if !accumulated_text.is_empty() {
            return Ok(json!({
                "status": "completed",
                "output_text": accumulated_text,
                "output": []
            }));
        }
        Err("SSE stream ended without response payload".to_string())
    }

    async fn post_json(
        &self,
        ctx: &InvokeCtx,
        url: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let started_at = std::time::Instant::now();
        let response = self
            .send_authenticated(ctx, url, |client, token| {
                client.post(url).bearer_auth(token).json(request_obj)
            })
            .await?;
        let latency_ms = started_at.elapsed().as_millis() as u64;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let content_encoding = response
            .headers()
            .get(CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let raw_body = response.text().await.map_err(|err| {
            Self::classify_api_error(
                status,
                format!(
                    "failed to decode sn-ai-provider response body: {}; content_type={} content_encoding={}",
                    err,
                    content_type,
                    if content_encoding.is_empty() {
                        "<none>"
                    } else {
                        content_encoding.as_str()
                    }
                ),
            )
        })?;
        let body_result = if content_type.contains("text/event-stream") {
            Self::parse_sse_response(&raw_body)
        } else {
            serde_json::from_str::<Value>(&raw_body).map_err(|err| err.to_string())
        };
        let body = body_result.map_err(|err| {
            Self::classify_api_error(
                status,
                format!(
                    "invalid sn-ai-provider response: {}; body_head={}",
                    err,
                    raw_body.chars().take(320).collect::<String>()
                ),
            )
        })?;
        Ok((status, body, latency_ms))
    }

    async fn post_binary_json(
        &self,
        ctx: &InvokeCtx,
        url: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Vec<u8>, String, u64), ProviderError> {
        let started_at = std::time::Instant::now();
        let response = self
            .send_authenticated(ctx, url, |client, token| {
                client.post(url).bearer_auth(token).json(request_obj)
            })
            .await?;
        let latency_ms = started_at.elapsed().as_millis() as u64;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = response.bytes().await.map_err(|err| {
            Self::classify_api_error(status, format!("failed to decode SN response: {err}"))
        })?;
        Ok((status, bytes.to_vec(), content_type, latency_ms))
    }

    async fn resource_to_file_bytes(
        &self,
        resource: &ResourceRef,
        fallback_name: &str,
    ) -> Result<(String, String, Vec<u8>), ProviderError> {
        match resource {
            ResourceRef::Base64 { mime, data_base64 } => {
                let bytes = general_purpose::STANDARD
                    .decode(data_base64)
                    .map_err(|err| {
                        ProviderError::fatal(format!("invalid base64 resource: {err}"))
                    })?;
                Ok((fallback_name.to_string(), mime.clone(), bytes))
            }
            ResourceRef::Url { url, mime_hint } => {
                let response = self.client.get(url).send().await.map_err(|err| {
                    if err.is_timeout() || err.is_connect() {
                        ProviderError::retryable(format!("failed to fetch resource url: {err}"))
                    } else {
                        ProviderError::fatal(format!("failed to fetch resource url: {err}"))
                    }
                })?;
                let status = response.status();
                if !status.is_success() {
                    return Err(Self::classify_api_error(
                        status,
                        format!("resource url returned status {}", status.as_u16()),
                    ));
                }
                let content_type = response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
                    .or_else(|| mime_hint.clone())
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let bytes = response.bytes().await.map_err(|err| {
                    ProviderError::fatal(format!("failed to read resource bytes: {err}"))
                })?;
                Ok((fallback_name.to_string(), content_type, bytes.to_vec()))
            }
            ResourceRef::NamedObject { obj_id } => Err(ProviderError::fatal(format!(
                "SN provider cannot resolve named object resource {obj_id} without resolver bytes"
            ))),
        }
    }

    async fn post_multipart(
        &self,
        ctx: &InvokeCtx,
        url: &str,
        fields: Vec<(String, String)>,
        files: Vec<(String, String, String, Vec<u8>)>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let boundary = format!("aicc-sn-{nonce}");
        let mut body = Vec::new();
        for (name, value) in fields {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            );
            body.extend_from_slice(value.as_bytes());
            body.extend_from_slice(b"\r\n");
        }
        for (field, filename, mime, data) in files {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(format!("Content-Type: {mime}\r\n\r\n").as_bytes());
            body.extend_from_slice(&data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

        let started_at = std::time::Instant::now();
        let response = self
            .send_authenticated(ctx, url, |client, token| {
                client
                    .post(url)
                    .bearer_auth(token)
                    .header(
                        CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(body.clone())
            })
            .await?;
        let latency_ms = started_at.elapsed().as_millis() as u64;
        let status = response.status();
        let body = response.json::<Value>().await.map_err(|err| {
            Self::classify_api_error(
                status,
                format!("failed to parse SN multipart response: {err}"),
            )
        })?;
        Ok((status, body, latency_ms))
    }

    async fn get_json(
        &self,
        ctx: &InvokeCtx,
        url: &str,
    ) -> Result<(StatusCode, Value), ProviderError> {
        let response = self
            .send_authenticated(ctx, url, |client, token| client.get(url).bearer_auth(token))
            .await?;
        let status = response.status();
        let body = response.json::<Value>().await.map_err(|err| {
            Self::classify_api_error(status, format!("failed to parse SN status response: {err}"))
        })?;
        Ok((status, body))
    }

    async fn get_binary(
        &self,
        ctx: &InvokeCtx,
        url: &str,
    ) -> Result<(StatusCode, Vec<u8>, String), ProviderError> {
        let response = self
            .send_authenticated(ctx, url, |client, token| client.get(url).bearer_auth(token))
            .await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = response.bytes().await.map_err(|err| {
            Self::classify_api_error(status, format!("failed to read SN content: {err}"))
        })?;
        Ok((status, bytes.to_vec(), content_type))
    }

    fn classify_api_error(status: StatusCode, message: String) -> ProviderError {
        let lower = message.to_ascii_lowercase();
        if status.as_u16() == 401 {
            return ProviderError::retryable(format!(
                "{}; SN AI Provider rejected the refreshed sn-sso session",
                message
            ));
        }
        if status.as_u16() == 403
            && (lower.contains("no active token gateway key")
                || lower.contains("token gateway key")
                || lower.contains("not active")
                || lower.contains("not available"))
        {
            return ProviderError::fatal(format!(
                "{}; SN AI Provider is not active for this device/user yet. It requires SN relay traffic mode and invite-code activation. Activation may have been completed from another client, so retry after that state syncs.",
                message
            ));
        }
        if status.as_u16() == 429 || status.is_server_error() {
            ProviderError::retryable(message)
        } else {
            ProviderError::fatal(message)
        }
    }

    fn estimate_cost_for_usage(&self, model: &str, usage: &AiUsage) -> Option<buckyos_api::AiCost> {
        let input_tokens = usage.input_tokens?;
        let output_tokens = usage.output_tokens?;
        let pricing = self.inventory.read().ok().and_then(|inventory| {
            inventory.models.iter().find_map(|metadata| {
                (metadata.provider_model_id == model).then(|| {
                    (
                        metadata.pricing.currency.clone(),
                        metadata.pricing.input_token,
                        metadata.pricing.output_token,
                        metadata.pricing.estimated_cost,
                    )
                })
            })
        });
        if !driver_model_has_specific_metadata(SN_AI_PROVIDER_DRIVER, model) {
            return None;
        }
        if let Some((currency, Some(input_price), Some(output_price), _)) = pricing.as_ref() {
            return Some(buckyos_api::AiCost {
                amount: (input_tokens as f64 * input_price) + (output_tokens as f64 * output_price),
                currency: currency.clone(),
            });
        }
        let (currency, _, _, estimated_cost) = pricing?;
        Some(buckyos_api::AiCost {
            amount: estimated_cost?,
            currency,
        })
    }

    fn ai_role_to_sn_protocol(role: &AiRole) -> &'static str {
        match role {
            AiRole::System => "system",
            AiRole::Developer => "developer",
            AiRole::User => "user",
            AiRole::Assistant => "assistant",
            _ => "user",
        }
    }

    fn resource_text(resource: &ResourceRef) -> String {
        match resource {
            ResourceRef::Url { url, .. } => format!("resource_url: {}", url),
            ResourceRef::NamedObject { obj_id } => format!("named_object: {}", obj_id),
            ResourceRef::Base64 { mime, data_base64 } => {
                format!("data:{};base64,{}", mime, data_base64)
            }
        }
    }

    fn responses_image_part(source: &ResourceRef) -> Value {
        match source {
            ResourceRef::Url { url, .. } => json!({
                "type": "input_image",
                "image_url": url,
            }),
            ResourceRef::Base64 { mime, data_base64 } => json!({
                "type": "input_image",
                "image_url": format!("data:{};base64,{}", mime, data_base64),
            }),
            ResourceRef::NamedObject { obj_id } => json!({
                "type": "input_text",
                "text": format!("named_object: {}", obj_id),
            }),
        }
    }

    fn build_messages(req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut items: Vec<Value> = vec![];
        for msg in &req.payload.messages {
            let role_str = Self::ai_role_to_sn_protocol(&msg.role);
            if role_str == "assistant" {
                let provider_items = msg
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        AiContent::ProviderState { provider, value }
                            if provider.eq_ignore_ascii_case(SN_AI_PROVIDER_DRIVER) =>
                        {
                            Some(value.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if !provider_items.is_empty() {
                    items.extend(provider_items);
                    continue;
                }
            }
            let content_type = if role_str == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            let mut pending_text_parts: Vec<Value> = Vec::new();
            for block in &msg.content {
                match block {
                    AiContent::Text { text } if !text.is_empty() => {
                        pending_text_parts.push(json!({
                            "type": content_type,
                            "text": text,
                        }));
                    }
                    AiContent::ToolUse {
                        call_id,
                        name,
                        args,
                    } => {
                        if !pending_text_parts.is_empty() {
                            items.push(json!({
                                "role": role_str,
                                "content": std::mem::take(&mut pending_text_parts),
                            }));
                        }
                        items.push(json!({
                            "type": "function_call",
                            "call_id": call_id,
                            "name": name,
                            "arguments": serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
                        }));
                    }
                    AiContent::ToolResult {
                        call_id, content, ..
                    } => {
                        if !pending_text_parts.is_empty() {
                            items.push(json!({
                                "role": role_str,
                                "content": std::mem::take(&mut pending_text_parts),
                            }));
                        }
                        let output = content
                            .iter()
                            .filter_map(|item| match item {
                                AiToolResultContent::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        items.push(json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": output,
                        }));
                    }
                    AiContent::Image { source } if role_str == "user" => {
                        pending_text_parts.push(Self::responses_image_part(source));
                    }
                    AiContent::Document { source, title } if role_str == "user" => {
                        let mut text = Self::resource_text(source);
                        if let Some(title) = title.as_deref().filter(|value| !value.is_empty()) {
                            text = format!("{} ({})", text, title);
                        }
                        pending_text_parts.push(json!({
                            "type": content_type,
                            "text": text,
                        }));
                    }
                    _ => {}
                }
            }
            if !pending_text_parts.is_empty() {
                items.push(json!({
                    "role": role_str,
                    "content": pending_text_parts,
                }));
            }
        }

        if items.is_empty() {
            let mut content = req.payload.text.clone().unwrap_or_default();
            let resource_lines = req
                .payload
                .resources
                .iter()
                .map(Self::resource_text)
                .collect::<Vec<_>>();
            if !resource_lines.is_empty() {
                if !content.is_empty() {
                    content.push_str("\n\n");
                }
                content.push_str(resource_lines.join("\n").as_str());
            }
            if !content.trim().is_empty() {
                items.push(json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": content }],
                }));
            }
        }

        if items.is_empty() {
            return Err(ProviderError::fatal(
                "request payload has no usable text/messages for sn-ai-provider llm",
            ));
        }
        Ok(items)
    }

    fn parse_tool_arguments(raw: Value, field_path: &str) -> Option<Value> {
        match raw {
            Value::String(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    return Some(Value::Object(Map::new()));
                }
                match serde_json::from_str::<Value>(trimmed) {
                    Ok(parsed) => Some(parsed),
                    Err(err) => {
                        warn!(
                            "aicc.sn_ai_provider {} is invalid json arguments: {}",
                            field_path, err
                        );
                        None
                    }
                }
            }
            Value::Null => Some(Value::Object(Map::new())),
            other => Some(other),
        }
    }

    fn extract_text_content(payload: &Value) -> Option<String> {
        if let Some(text) = payload.get("output_text").and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }

        let mut parts = Vec::new();
        if let Some(output_items) = payload.get("output").and_then(Value::as_array) {
            for item in output_items {
                let Some(item_obj) = item.as_object() else {
                    continue;
                };
                if item_obj.get("type").and_then(Value::as_str) == Some("output_text") {
                    if let Some(text) = item_obj.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            parts.push(text.to_string());
                        }
                    }
                    continue;
                }
                if item_obj.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                if let Some(content_items) = item_obj.get("content").and_then(Value::as_array) {
                    for content_item in content_items {
                        let Some(content_obj) = content_item.as_object() else {
                            continue;
                        };
                        let content_type = content_obj.get("type").and_then(Value::as_str);
                        if content_type != Some("output_text") && content_type != Some("text") {
                            continue;
                        }
                        if let Some(text) = content_obj.get("text").and_then(Value::as_str) {
                            if !text.trim().is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
            }
        }
        if parts.is_empty() {
            payload
                .pointer("/choices/0/message/content")
                .and_then(|content| match content {
                    Value::String(text) => Some(text.trim().to_string()),
                    Value::Array(items) => Some(
                        items
                            .iter()
                            .filter_map(|item| item.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(""),
                    ),
                    _ => None,
                })
                .filter(|text| !text.is_empty())
        } else {
            Some(parts.concat().trim().to_string())
        }
    }

    fn extract_tool_choices(payload: &Value) -> Vec<AiToolCall> {
        let mut tool_choices = Vec::new();
        for (idx, item) in payload
            .get("output")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(item_obj) = item.as_object() else {
                continue;
            };
            if item_obj.get("type").and_then(Value::as_str) != Some("function_call") {
                continue;
            }
            let Some(call_id) = item_obj
                .get("call_id")
                .or_else(|| item_obj.get("id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
            else {
                warn!(
                    "aicc.sn_ai_provider output[{}] function_call missing id",
                    idx
                );
                continue;
            };
            let Some(name) = item_obj
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                warn!(
                    "aicc.sn_ai_provider output[{}] function_call missing name",
                    idx
                );
                continue;
            };
            let Some(args) = Self::parse_tool_arguments(
                item_obj
                    .get("arguments")
                    .or_else(|| item_obj.get("args"))
                    .cloned()
                    .unwrap_or(Value::Null),
                format!("output[{}].arguments", idx).as_str(),
            ) else {
                continue;
            };
            if !args.is_object() {
                warn!(
                    "aicc.sn_ai_provider output[{}].arguments must decode to an object",
                    idx
                );
                continue;
            }
            tool_choices.push(AiToolCall {
                name: name.to_string(),
                args: value_to_object_map(args),
                call_id,
            });
        }
        if !tool_choices.is_empty() {
            return tool_choices;
        }
        let Some(items) = payload
            .pointer("/choices/0/message/tool_calls")
            .and_then(Value::as_array)
        else {
            return tool_choices;
        };
        for (idx, item) in items.iter().enumerate() {
            let Some(call_id) = item
                .get("id")
                .or_else(|| item.get("call_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                warn!("aicc.sn_ai_provider tool_calls[{}] missing id", idx);
                continue;
            };
            let function = item.get("function").unwrap_or(item);
            let Some(name) = function
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                warn!("aicc.sn_ai_provider tool_calls[{}] missing name", idx);
                continue;
            };
            let Some(args) = Self::parse_tool_arguments(
                function
                    .get("arguments")
                    .or_else(|| function.get("args"))
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                &format!("tool_calls[{}].arguments", idx),
            ) else {
                continue;
            };
            if args.is_object() {
                tool_choices.push(AiToolCall {
                    name: name.to_string(),
                    args: value_to_object_map(args),
                    call_id: call_id.to_string(),
                });
            }
        }
        tool_choices
    }

    fn message_from_response_body(
        body: &Value,
        text: Option<String>,
        tool_calls: Vec<AiToolCall>,
    ) -> buckyos_api::AiMessage {
        let mut message = AiResponse::message_from_parts(text, tool_calls, vec![]);
        if let Some(output_items) = body.get("output").and_then(Value::as_array) {
            message
                .content
                .extend(
                    output_items
                        .iter()
                        .cloned()
                        .map(|value| AiContent::ProviderState {
                            provider: SN_AI_PROVIDER_DRIVER.to_string(),
                            value,
                        }),
                );
        }
        message
    }

    fn incomplete_output_error(
        body: &Value,
        content: Option<&str>,
        tool_calls: &[AiToolCall],
    ) -> Option<ProviderError> {
        if body.get("status").and_then(Value::as_str) != Some("incomplete") {
            return None;
        }
        let reason = body
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let response_id = body.get("id").and_then(Value::as_str).unwrap_or_default();
        if reason == "max_output_tokens" {
            return Some(ProviderError::fatal(format!(
                "TOKEN_LIMIT_EXCEEDED: SN max_output_tokens exhausted before response completed{}",
                if response_id.is_empty() {
                    String::new()
                } else {
                    format!(" (response_id={response_id})")
                }
            )));
        }
        if content.is_some_and(|text| !text.trim().is_empty()) || !tool_calls.is_empty() {
            return None;
        }
        Some(ProviderError::fatal(format!(
            "SN response incomplete before output text/tool calls (reason={reason}{})",
            if response_id.is_empty() {
                String::new()
            } else {
                format!(", response_id={response_id}")
            }
        )))
    }

    fn unsupported_request_param(body: &Value) -> Option<String> {
        let param = body
            .pointer("/error/param")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let message = body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        (message.contains("unsupported parameter") || message.contains("not supported"))
            .then(|| param.trim_matches(['\'', '"']).to_string())
    }

    fn remove_retryable_option(request: &mut Map<String, Value>, param: &str) -> bool {
        ["temperature", "top_p", "top_logprobs", "logprobs"].contains(&param)
            && request.remove(param).is_some()
    }

    async fn start_llm(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert(
            "input".to_string(),
            Value::Array(Self::build_messages(req)?),
        );

        let mut ignored_options = vec![];
        if let Some(input_json) = req.payload.input_json.as_ref() {
            ignored_options.extend(merge_sn_llm_options(&mut request_obj, input_json)?);
        }
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options.extend(merge_sn_llm_options(&mut request_obj, options)?);
        }
        apply_sn_model_defaults(&mut request_obj, provider_model);
        let stripped_options = strip_sn_sampling_options(&mut request_obj, provider_model);
        if !stripped_options.is_empty() {
            info!(
                "aicc.sn_ai_provider omitted incompatible llm options: provider_instance_name={} model={} trace_id={:?} omitted={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, stripped_options
            );
        }
        merge_sn_required_response_format(&mut request_obj, req);
        merge_sn_tool_specs(&mut request_obj, req.payload.tool_specs.as_slice())?;
        merge_sn_required_tools(&mut request_obj, req)?;
        if normalize_sn_web_search_reasoning(&mut request_obj) {
            info!(
                "aicc.sn_ai_provider adjusted reasoning.effort for web_search: provider_instance_name={} model={} trace_id={:?} effort=low",
                self.instance.provider_instance_name, provider_model, ctx.trace_id
            );
        }
        if !ignored_options.is_empty() {
            warn!(
                "aicc.sn_ai_provider ignored unsupported llm options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        info!(
            "aicc.sn_ai_provider.llm.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            redacted_json_log(&Value::Object(request_obj.clone()))
        );
        let endpoint = self.responses_endpoint();
        let mut retried_without_option = false;
        let (status, body, latency_ms) = loop {
            let result = self.post_json(ctx, &endpoint, &request_obj).await?;
            if result.0 == StatusCode::BAD_REQUEST && !retried_without_option {
                if let Some(param) = Self::unsupported_request_param(&result.1) {
                    if Self::remove_retryable_option(&mut request_obj, &param) {
                        warn!(
                            "aicc.sn_ai_provider.llm.retry_without_option provider_instance_name={} model={} trace_id={:?} param={}",
                            self.instance.provider_instance_name, provider_model, ctx.trace_id, param
                        );
                        retried_without_option = true;
                        continue;
                    }
                }
            }
            break result;
        };
        let response_log = redacted_json_log(&body);

        if !status.is_success() {
            warn!(
                "aicc.sn_ai_provider.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("sn-ai-provider api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("sn-ai-provider api error [{}]: {}", code, message),
            ));
        }
        info!(
            "aicc.sn_ai_provider.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let content = Self::extract_text_content(&body);
        let tool_choices = Self::extract_tool_choices(&body);
        if let Some(error) = Self::incomplete_output_error(&body, content.as_deref(), &tool_choices)
        {
            return Err(error);
        }
        let usage = body.get("usage").map(|usage| AiUsage {
            input_tokens: usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .or_else(|| usage.get("prompt_tokens").and_then(Value::as_u64)),
            output_tokens: usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .or_else(|| usage.get("completion_tokens").and_then(Value::as_u64)),
            total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
            request_units: None,
        });
        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));

        let summary = AiResponse {
            message: Self::message_from_response_body(&body, content, tool_choices),
            usage,
            cost,
            finish_reason: body
                .get("status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            provider_task_ref: body
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            extra: Some(json!({
                "provider": SN_AI_PROVIDER_DRIVER,
                "model": provider_model,
                "latency_ms": latency_ms,
                "provider_io": {
                    "input": Value::Object(request_obj),
                    "output": body
                }
            })),
        };
        Ok(ProviderStartResult::Immediate(summary))
    }

    fn extract_text2image_prompt(req: &AiMethodRequest) -> Option<String> {
        if let Some(prompt) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("prompt"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(prompt.to_string());
        }
        if let Some(text) = req
            .payload
            .text
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Some(text.to_string());
        }
        let messages = req
            .payload
            .messages
            .iter()
            .map(|message| message.text_content().trim().to_string())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !messages.is_empty() {
            return Some(messages);
        }
        req.payload
            .options
            .as_ref()
            .and_then(|value| value.get("prompt"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn build_text2image_request(
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<(Map<String, Value>, Vec<String>), ProviderError> {
        let mut target = Map::new();
        target.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        if let Some(input) = req.payload.input_json.as_ref().and_then(Value::as_object) {
            for (key, value) in input {
                if SN_IMAGE_INPUT_ALLOWLIST.contains(&key.as_str()) {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
        if let Some(prompt) = Self::extract_text2image_prompt(req) {
            target.insert("prompt".to_string(), Value::String(prompt));
        }
        if !target.contains_key("prompt") {
            return Err(ProviderError::fatal(
                "SN text2image request requires prompt in payload.text/messages/input_json/options",
            ));
        }

        let mut ignored = vec![];
        if let Some(options) = req.payload.options.as_ref().and_then(Value::as_object) {
            for (key, value) in options {
                if key == "model" || key == "messages" || key == "prompt" {
                    continue;
                }
                if key == "protocol" || key == "process_name" || key == "tool_messages" {
                    ignored.push(key.clone());
                } else if SN_IMAGE_OPTION_ALLOWLIST.contains(&key.as_str()) {
                    target.insert(key.clone(), value.clone());
                } else {
                    ignored.push(key.clone());
                }
            }
        }
        Ok((target, ignored))
    }

    fn parse_text2image_artifacts(body: &Value) -> Result<Vec<AiArtifact>, ProviderError> {
        let Some(items) = body.get("data").and_then(Value::as_array) else {
            return Err(ProviderError::fatal(
                "SN image response is missing data array",
            ));
        };
        let mut artifacts = vec![];
        for (idx, item) in items.iter().enumerate() {
            let metadata = item
                .get("revised_prompt")
                .and_then(Value::as_str)
                .map(|prompt| json!({ "revised_prompt": prompt }));
            if let Some(url) = item
                .get("url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                artifacts.push(AiArtifact {
                    name: format!("image_{}", idx + 1),
                    resource: ResourceRef::Url {
                        url: url.to_string(),
                        mime_hint: Some("image/png".to_string()),
                    },
                    mime: Some("image/png".to_string()),
                    metadata,
                });
                continue;
            }
            if let Some(data) = item
                .get("b64_json")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if general_purpose::STANDARD.decode(data).is_err() {
                    warn!(
                        "aicc.sn_ai_provider received invalid b64_json at index {} in image response",
                        idx
                    );
                    continue;
                }
                artifacts.push(AiArtifact {
                    name: format!("image_{}", idx + 1),
                    resource: ResourceRef::Base64 {
                        mime: "image/png".to_string(),
                        data_base64: data.to_string(),
                    },
                    mime: Some("image/png".to_string()),
                    metadata,
                });
            }
        }
        if artifacts.is_empty() {
            return Err(ProviderError::fatal(
                "SN image response has no usable image outputs",
            ));
        }
        Ok(artifacts)
    }

    fn embedding_inputs(
        req: &AiMethodRequest,
    ) -> Result<(Value, Vec<Option<String>>), ProviderError> {
        if let Some(items) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("items"))
            .and_then(Value::as_array)
        {
            let mut texts = Vec::with_capacity(items.len());
            let mut ids = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
                    .ok_or_else(|| {
                        ProviderError::fatal(format!(
                            "embedding.text item {} must contain text; resource items are unsupported by SN text embeddings",
                            index
                        ))
                    })?;
                texts.push(Value::String(text.to_string()));
                ids.push(
                    item.get("id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                );
            }
            if !texts.is_empty() {
                return Ok((Value::Array(texts), ids));
            }
        }
        if let Some(text) = req.payload.text.as_ref() {
            return Ok((Value::String(text.clone()), vec![None]));
        }
        let texts = req
            .payload
            .messages
            .iter()
            .map(|message| message.text_content().trim().to_string())
            .filter(|text| !text.is_empty())
            .map(Value::String)
            .collect::<Vec<_>>();
        if !texts.is_empty() {
            let ids = vec![None; texts.len()];
            return Ok((Value::Array(texts), ids));
        }
        Err(ProviderError::fatal(
            "embedding.text requires payload.input_json.items or payload.text",
        ))
    }

    async fn start_embedding(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let input_json = req.payload.input_json.as_ref();
        if input_json.and_then(|value| value.get("chunking")).is_some() {
            return Err(ProviderError::fatal(
                "SN embedding endpoint does not support canonical chunking",
            ));
        }
        if input_json
            .and_then(|value| value.get("normalize"))
            .and_then(Value::as_bool)
            == Some(false)
        {
            return Err(ProviderError::fatal(
                "SN OpenAI-compatible embeddings cannot satisfy normalize=false",
            ));
        }
        let (embedding_input, input_ids) = Self::embedding_inputs(req)?;
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert("input".to_string(), embedding_input);
        if let Some(dimensions) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("dimensions"))
            .cloned()
        {
            request_obj.insert("dimensions".to_string(), dimensions);
        }
        let endpoint = self.litellm_endpoint("embeddings");
        let (status, body, latency_ms) =
            self.post_json(ctx, endpoint.as_str(), &request_obj).await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("SN embeddings returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let dimensions = body
            .pointer("/data/0/embedding")
            .and_then(Value::as_array)
            .map(Vec::len);
        let embedding_space_id = input_json
            .and_then(|value| value.get("embedding_space_id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                format!(
                    "sn:{}:{}",
                    provider_model,
                    dimensions
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                )
            });
        let mut data = body
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (index, item) in data.iter_mut().enumerate() {
            if let (Some(id), Some(object)) = (
                input_ids.get(index).and_then(|value| value.as_ref()),
                item.as_object_mut(),
            ) {
                object.insert("id".to_string(), Value::String(id.clone()));
            }
        }
        let prefer_artifact = data.len() > 100
            || input_json
                .and_then(|value| value.get("prefer_artifact"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
            || input_json
                .and_then(|value| value.get("output"))
                .and_then(|value| value.get("resource_format"))
                .and_then(Value::as_str)
                .is_some_and(|value| value == "named_object");
        let artifact = if prefer_artifact {
            let bytes = serde_json::to_vec(&json!({
                "embedding_space_id": embedding_space_id.clone(),
                "data": data.clone(),
            }))
            .map_err(|error| {
                ProviderError::fatal(format!("serialize embedding artifact failed: {}", error))
            })?;
            Some(AiArtifact {
                name: "embeddings.json".to_string(),
                resource: ResourceRef::Base64 {
                    mime: "application/json".to_string(),
                    data_base64: general_purpose::STANDARD.encode(bytes),
                },
                mime: Some("application/json".to_string()),
                metadata: Some(json!({
                    "rows": data.len(),
                    "dimensions": dimensions,
                    "embedding_space_id": embedding_space_id.clone(),
                })),
            })
        } else {
            None
        };
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(
                None,
                vec![],
                artifact.clone().into_iter().collect(),
            ),
            finish_reason: Some("stop".to_string()),
            extra: Some(json!({
                "embedding": {
                    "data": if prefer_artifact { Value::Array(vec![]) } else { Value::Array(data.clone()) },
                    "embedding_space_id": embedding_space_id.clone(),
                    "artifact": artifact.as_ref().map(|value| json!({
                        "name": value.name.clone(),
                        "mime": value.mime.clone(),
                        "rows": data.len(),
                        "dimensions": dimensions,
                        "embedding_space_id": embedding_space_id.clone(),
                    })),
                    "provider_io": {
                        "input": Value::Object(request_obj),
                        "output": body
                    },
                    "latency_ms": latency_ms
                }
            })),
            ..Default::default()
        }))
    }

    async fn start_rerank(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let input = req.payload.input_json.clone().unwrap_or_else(|| json!({}));
        let prompt = format!(
            "Rerank the documents for the query. Return only JSON with key results, where each result has index, id and score from 0 to 1.\n{}",
            input
        );
        let rerank_req = AiMethodRequest {
            payload: buckyos_api::AiPayload::new(
                Some(prompt),
                vec![],
                vec![],
                vec![],
                Some(json!({
                    "text": {
                        "format": {
                            "type": "json_schema",
                            "name": "rerank_result",
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "results": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "index": { "type": "integer" },
                                                "id": { "type": "string" },
                                                "score": { "type": "number" }
                                            },
                                            "required": ["index", "id", "score"],
                                            "additionalProperties": false
                                        }
                                    }
                                },
                                "required": ["results"],
                                "additionalProperties": false
                            }
                        }
                    }
                })),
                req.payload.options.clone(),
            ),
            ..req.clone()
        };
        let mut result = self.start_llm(ctx, provider_model, &rerank_req).await?;
        if let ProviderStartResult::Immediate(summary) = &mut result {
            let text = summary.text_content();
            let rerank = serde_json::from_str::<Value>(&text)
                .unwrap_or_else(|_| json!({ "raw_text": text }));
            let mut extra = summary
                .extra
                .take()
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            extra.insert("rerank".to_string(), rerank);
            summary.extra = Some(Value::Object(extra));
        }
        Ok(result)
    }

    async fn start_tts(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let text = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("text"))
            .and_then(Value::as_str)
            .or(req.payload.text.as_deref())
            .ok_or_else(|| ProviderError::fatal("audio.tts requires text"))?;
        let voice = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.pointer("/voice/voice_id"))
            .and_then(Value::as_str)
            .or_else(|| {
                req.payload
                    .input_json
                    .as_ref()
                    .and_then(|value| value.get("voice"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("alloy");
        let response_format = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.pointer("/output/media_type"))
            .and_then(Value::as_str)
            .map(|mime| if mime.contains("wav") { "wav" } else { "mp3" })
            .unwrap_or("mp3");
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert("input".to_string(), Value::String(text.to_string()));
        request_obj.insert("voice".to_string(), Value::String(voice.to_string()));
        request_obj.insert(
            "response_format".to_string(),
            Value::String(response_format.to_string()),
        );
        let endpoint = self.litellm_endpoint("audio/speech");
        let (status, bytes, content_type, latency_ms) = self
            .post_binary_json(ctx, endpoint.as_str(), &request_obj)
            .await?;
        if !status.is_success() {
            return Err(Self::classify_api_error(
                status,
                String::from_utf8_lossy(&bytes).to_string(),
            ));
        }
        let mime = if content_type.contains("audio") {
            content_type
        } else if response_format == "wav" {
            "audio/wav".to_string()
        } else {
            "audio/mpeg".to_string()
        };
        let artifact = AiArtifact {
            name: "audio".to_string(),
            resource: ResourceRef::Base64 {
                mime: mime.clone(),
                data_base64: general_purpose::STANDARD.encode(bytes),
            },
            mime: Some(mime),
            metadata: None,
        };
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiMessage::new(AiRole::Assistant, vec![artifact.into_content()]),
            finish_reason: Some("stop".to_string()),
            extra: Some(json!({
                "provider": SN_AI_PROVIDER_DRIVER,
                "model": provider_model,
                "latency_ms": latency_ms
            })),
            ..Default::default()
        }))
    }

    async fn start_asr(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let resource = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["audio"]))
            .ok_or_else(|| ProviderError::fatal("audio.asr requires canonical audio input"))?;
        let (filename, mime, bytes) = self.resource_to_file_bytes(&resource, "audio").await?;
        let mut fields = vec![
            ("model".to_string(), provider_model.to_string()),
            ("response_format".to_string(), "json".to_string()),
        ];
        if let Some(language) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("language"))
            .and_then(Value::as_str)
        {
            fields.push(("language".to_string(), language.to_string()));
        }
        let endpoint = self.litellm_endpoint("audio/transcriptions");
        let (status, body, latency_ms) = self
            .post_multipart(
                ctx,
                endpoint.as_str(),
                fields,
                vec![("file".to_string(), filename, mime, bytes)],
            )
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("SN transcription returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let text = body.get("text").and_then(Value::as_str).map(str::to_string);
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(text, vec![], vec![]),
            finish_reason: Some("stop".to_string()),
            extra: Some(json!({
                "asr": {
                    "segments": body.get("segments").cloned().unwrap_or_else(|| Value::Array(vec![])),
                    "provider_io": { "output": body },
                    "latency_ms": latency_ms
                }
            })),
            ..Default::default()
        }))
    }

    fn resource_from_input_json(req: &AiMethodRequest, keys: &[&str]) -> Option<ResourceRef> {
        let input = req.payload.input_json.as_ref()?;
        keys.iter().find_map(|key| {
            let value = input.get(*key)?;
            serde_json::from_value(value.clone()).ok().or_else(|| {
                value
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
            })
        })
    }

    fn vision_prompt(method: &str, req: &AiMethodRequest) -> String {
        if let Some(prompt) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("prompt"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return prompt.to_string();
        }
        let mut prompt = if method == ai_methods::VISION_OCR {
            "Extract all readable text from this image. Preserve reading order, line breaks, and layout as closely as possible. Return only the extracted text.".to_string()
        } else {
            "Describe this image accurately and concisely.".to_string()
        };
        if let Some(options) = req.payload.input_json.as_ref().and_then(Value::as_object) {
            let mut options = options.clone();
            options.remove("image");
            options.remove("document");
            options.remove("prompt");
            if !options.is_empty() {
                prompt.push_str(" Follow these request options: ");
                prompt.push_str(&Value::Object(options).to_string());
            }
        }
        prompt
    }

    async fn start_vision(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let resource = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["image", "document"]))
            .ok_or_else(|| ProviderError::fatal("vision request requires an image resource"))?;
        let mut vision_req = req.clone();
        vision_req.payload.text = None;
        vision_req.payload.messages = vec![AiMessage::new(
            AiRole::User,
            vec![
                AiContent::text(Self::vision_prompt(method, req)),
                AiContent::image(resource),
            ],
        )];
        vision_req.payload.tool_specs.clear();
        vision_req.payload.resources.clear();
        vision_req.payload.input_json = None;
        vision_req.payload.options = None;
        match self.start_llm(ctx, provider_model, &vision_req).await? {
            ProviderStartResult::Immediate(mut response) => {
                let key = if method == ai_methods::VISION_OCR {
                    "ocr"
                } else {
                    "captions"
                };
                let text = response.text_content();
                let extra = response.extra.get_or_insert_with(|| json!({}));
                if !extra.is_object() {
                    *extra = json!({});
                }
                extra
                    .as_object_mut()
                    .expect("extra normalized to object")
                    .insert(key.to_string(), json!({ "text": text }));
                Ok(ProviderStartResult::Immediate(response))
            }
            other => Ok(other),
        }
    }

    fn video_option(req: &AiMethodRequest, keys: &[&str]) -> Option<Value> {
        for source in [
            req.payload.input_json.as_ref(),
            req.payload.options.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            for key in keys {
                if let Some(value) = source.get(*key) {
                    return Some(value.clone());
                }
            }
        }
        None
    }

    fn video_size(req: &AiMethodRequest) -> Option<String> {
        let raw = Self::video_option(req, &["size", "resolution"])?;
        let value = raw.as_str()?.trim();
        if value.contains('x') {
            return Some(value.to_string());
        }
        let aspect_ratio = Self::video_option(req, &["aspect_ratio", "aspectRatio"])
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "16:9".to_string());
        match (value.to_ascii_lowercase().as_str(), aspect_ratio.as_str()) {
            ("720p", "9:16") => Some("720x1280".to_string()),
            ("720p", _) => Some("1280x720".to_string()),
            ("1024p", "9:16") => Some("1024x1792".to_string()),
            ("1024p", _) => Some("1792x1024".to_string()),
            _ => None,
        }
    }

    fn video_dimensions(size: &str) -> Option<(u32, u32)> {
        match size {
            "720x1280" => Some((720, 1280)),
            "1280x720" => Some((1280, 720)),
            "1024x1792" => Some((1024, 1792)),
            "1792x1024" => Some((1792, 1024)),
            _ => None,
        }
    }

    fn normalize_video_reference_image(
        bytes: &[u8],
        requested_size: Option<&str>,
    ) -> Result<(String, Vec<u8>), ProviderError> {
        let image = image::load_from_memory(bytes).map_err(|err| {
            ProviderError::fatal(format!("failed to decode video reference image: {err}"))
        })?;
        let size = requested_size.map(str::to_string).unwrap_or_else(|| {
            if image.width() > image.height() {
                "1280x720".to_string()
            } else {
                "720x1280".to_string()
            }
        });
        let (width, height) = Self::video_dimensions(&size)
            .ok_or_else(|| ProviderError::fatal(format!("unsupported SN video size '{size}'")))?;
        let normalized = image.resize_to_fill(width, height, FilterType::Lanczos3);
        let mut output = Cursor::new(Vec::new());
        normalized
            .write_to(&mut output, ImageFormat::Png)
            .map_err(|err| {
                ProviderError::fatal(format!("failed to encode video reference image: {err}"))
            })?;
        Ok((size, output.into_inner()))
    }

    async fn start_video(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
        sink: Arc<dyn TaskEventSink>,
    ) -> Result<ProviderStartResult, ProviderError> {
        let prompt = Self::extract_text2image_prompt(req)
            .ok_or_else(|| ProviderError::fatal(format!("{method} requires a non-empty prompt")))?;
        let mut fields = vec![
            ("model".to_string(), provider_model.to_string()),
            ("prompt".to_string(), prompt),
        ];
        if let Some(seconds) = Self::video_option(
            req,
            &["seconds", "duration", "duration_seconds", "durationSeconds"],
        ) {
            fields.push(("seconds".to_string(), sn_form_field(&seconds)));
        }
        let requested_size = Self::video_size(req);
        let mut files = vec![];
        if method == ai_methods::VIDEO_IMG2VIDEO {
            let resource = req
                .payload
                .resources
                .first()
                .cloned()
                .or_else(|| Self::resource_from_input_json(req, &["image", "images"]))
                .ok_or_else(|| {
                    ProviderError::fatal("video.img2video requires canonical image input")
                })?;
            let (name, mime, bytes) = self
                .resource_to_file_bytes(&resource, "input_reference.png")
                .await?;
            let normalize_size = requested_size.clone();
            let (size, normalized_bytes) = tokio::task::spawn_blocking(move || {
                Self::normalize_video_reference_image(&bytes, normalize_size.as_deref())
            })
            .await
            .map_err(|err| {
                ProviderError::fatal(format!("failed to normalize video reference image: {err}"))
            })??;
            info!(
                "aicc.sn_ai_provider.video.input_reference_normalized provider_instance_name={} model={} source_name={} source_mime={} target_size={}",
                self.instance.provider_instance_name, provider_model, name, mime, size
            );
            fields.push(("size".to_string(), size));
            files.push((
                "input_reference".to_string(),
                "input_reference.png".to_string(),
                "image/png".to_string(),
                normalized_bytes,
            ));
        } else if let Some(size) = requested_size {
            fields.push(("size".to_string(), size));
        }

        let started_at = std::time::Instant::now();
        let endpoint = self.litellm_endpoint("videos");
        let (status, job, _) = self
            .post_multipart(ctx, endpoint.as_str(), fields, files)
            .await?;
        if !status.is_success() {
            let message = job
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("SN video create returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let video_id = job
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ProviderError::fatal("SN video create response is missing id"))?
            .to_string();
        let provider = self.clone();
        let task_id = ctx.task_id.clone().unwrap_or_else(|| video_id.clone());
        let invoke_ctx = ctx.clone();
        let provider_model = provider_model.to_string();
        let method = method.to_string();
        let request = req.clone();
        tokio::spawn(async move {
            let result = provider
                .finish_video(
                    &invoke_ctx,
                    &provider_model,
                    &method,
                    &video_id,
                    job,
                    started_at,
                )
                .await;
            emit_background_provider_result(sink, &task_id, &request, result).await;
        });
        Ok(ProviderStartResult::Started)
    }

    async fn finish_video(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        method: &str,
        video_id: &str,
        mut job: Value,
        started_at: std::time::Instant,
    ) -> Result<AiResponse, ProviderError> {
        loop {
            match job
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("queued")
            {
                "completed" => break,
                "queued" | "in_progress" => {}
                "failed" => {
                    let message = job
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("SN video generation failed");
                    return Err(ProviderError::fatal(message.to_string()));
                }
                state => {
                    return Err(ProviderError::fatal(format!(
                        "SN video generation returned unknown status '{state}'"
                    )));
                }
            }
            if started_at.elapsed() >= SN_VIDEO_MAX_WAIT {
                return Err(ProviderError::fatal(format!(
                    "SN video generation timed out after {} seconds",
                    SN_VIDEO_MAX_WAIT.as_secs()
                )));
            }
            time::sleep(SN_VIDEO_POLL_INTERVAL).await;
            let endpoint = self.litellm_endpoint(&format!("videos/{video_id}"));
            let (status, next_job) = self.get_json(ctx, &endpoint).await?;
            if !status.is_success() {
                let message = next_job
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("SN video status returned non-success status");
                return Err(Self::classify_api_error(status, message.to_string()));
            }
            job = next_job;
        }
        let endpoint = self.litellm_endpoint(&format!("videos/{video_id}/content"));
        let (status, bytes, content_type) = self.get_binary(ctx, &endpoint).await?;
        if !status.is_success() {
            return Err(Self::classify_api_error(
                status,
                format!("SN video download returned status {}", status.as_u16()),
            ));
        }
        let mime = if content_type.contains("video/") {
            content_type
        } else {
            "video/mp4".to_string()
        };
        let artifact = AiArtifact {
            name: "video.mp4".to_string(),
            resource: ResourceRef::Base64 {
                mime: mime.clone(),
                data_base64: general_purpose::STANDARD.encode(bytes),
            },
            mime: Some(mime),
            metadata: None,
        };
        Ok(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], vec![artifact]),
            usage: Some(AiUsage::request_units(1)),
            cost: Some(buckyos_api::AiCost {
                amount: if provider_model.contains("pro") {
                    1.2
                } else {
                    0.4
                },
                currency: "USD".to_string(),
            }),
            finish_reason: Some("stop".to_string()),
            provider_task_ref: Some(video_id.to_string()),
            extra: Some(json!({
                "provider": SN_AI_PROVIDER_DRIVER,
                "method": method,
                "model": provider_model,
                "latency_ms": started_at.elapsed().as_millis() as u64,
                "provider_io": { "output": job }
            })),
            ..Default::default()
        })
    }

    async fn start_image_edit(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
        with_mask: bool,
    ) -> Result<ProviderStartResult, ProviderError> {
        let image = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["image", "images"]))
            .ok_or_else(|| ProviderError::fatal("image edit requires canonical image input"))?;
        let (name, mime, bytes) = self.resource_to_file_bytes(&image, "image.png").await?;
        let mut files = vec![("image".to_string(), name, mime, bytes)];
        if with_mask {
            let mask = req
                .payload
                .resources
                .get(1)
                .cloned()
                .or_else(|| Self::resource_from_input_json(req, &["mask"]))
                .ok_or_else(|| {
                    ProviderError::fatal("image.inpaint requires canonical mask input")
                })?;
            let (name, mime, bytes) = self.resource_to_file_bytes(&mask, "mask.png").await?;
            files.push(("mask".to_string(), name, mime, bytes));
        }
        let prompt = Self::extract_text2image_prompt(req).ok_or_else(|| {
            ProviderError::fatal("image edit requires prompt in payload.text/input_json/options")
        })?;
        let mut fields = vec![
            ("model".to_string(), provider_model.to_string()),
            ("prompt".to_string(), prompt),
        ];
        for source in [
            req.payload.input_json.as_ref(),
            req.payload.options.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(options) = source.as_object() {
                for (key, value) in options {
                    if key != "prompt"
                        && key != "model"
                        && SN_IMAGE_EDIT_OPTION_ALLOWLIST.contains(&key.as_str())
                    {
                        fields.push((key.clone(), sn_form_field(value)));
                    }
                }
            }
        }
        let endpoint = self.litellm_endpoint("images/edits");
        let (status, body, latency_ms) = self
            .post_multipart(ctx, endpoint.as_str(), fields, files)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("SN image edit returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let artifacts = Self::parse_text2image_artifacts(&body)?;
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], artifacts),
            finish_reason: Some("stop".to_string()),
            extra: Some(json!({
                "provider": SN_AI_PROVIDER_DRIVER,
                "model": provider_model,
                "latency_ms": latency_ms,
                "provider_io": { "output": body }
            })),
            ..Default::default()
        }))
    }

    async fn start_text2image(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let (request_obj, ignored_options) = Self::build_text2image_request(provider_model, req)?;
        if !ignored_options.is_empty() {
            warn!(
                "aicc.sn_ai_provider ignored unsupported text2image options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }
        let request_log = redacted_json_log(&Value::Object(request_obj.clone()));
        info!(
            "aicc.sn_ai_provider.text2image.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );
        let endpoint = self.images_generations_endpoint();
        let (status, body, latency_ms) =
            self.post_json(ctx, endpoint.as_str(), &request_obj).await?;
        let response_log = redacted_json_log(&body);
        if !status.is_success() {
            warn!(
                "aicc.sn_ai_provider.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("sn-ai-provider image api returned non-success status");
            let code = body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("sn-ai-provider image api error [{}]: {}", code, message),
            ));
        }
        info!(
            "aicc.sn_ai_provider.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );
        let artifacts = Self::parse_text2image_artifacts(&body)?;
        let revised_prompt = body
            .pointer("/data/0/revised_prompt")
            .and_then(Value::as_str)
            .map(str::to_string);
        let cost = self.inventory.read().ok().and_then(|inventory| {
            inventory.models.iter().find_map(|metadata| {
                (metadata.provider_model_id == provider_model)
                    .then_some(metadata.pricing.estimated_cost)
                    .flatten()
                    .map(|amount| AiCost {
                        amount,
                        currency: metadata.pricing.currency.clone(),
                    })
            })
        });
        let summary = AiResponse {
            message: AiResponse::message_from_parts(revised_prompt, vec![], artifacts),
            usage: Some(AiUsage::request_units(1)),
            cost,
            finish_reason: Some("stop".to_string()),
            provider_task_ref: body.get("id").and_then(Value::as_str).map(str::to_string),
            extra: Some(json!({
                "provider": "sn-ai-provider",
                "model": provider_model,
                "latency_ms": latency_ms,
                "provider_io": {
                    "input": Value::Object(request_obj),
                    "output": body
                }
            })),
        };
        Ok(ProviderStartResult::Immediate(summary))
    }
}

fn provider_model_from_exact(exact_model: &str) -> String {
    exact_model
        .split_once('@')
        .map(|(model, _)| model)
        .unwrap_or(exact_model)
        .to_string()
}

fn inventory_revision(models: &[String]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for model in models {
        model.hash(&mut hasher);
    }
    format!("sn-ai-provider-{:x}", hasher.finish())
}

#[async_trait]
impl Provider for SnAIProvider {
    fn inventory(&self) -> ProviderInventory {
        self.inventory
            .read()
            .map(|inventory| inventory.clone())
            .unwrap_or_else(|_| {
                warn!("sn-ai-provider inventory lock poisoned, returning empty inventory");
                ProviderInventory {
                    provider_instance_name: self.instance.provider_instance_name.clone(),
                    provider_type: self.provider_type.clone(),
                    provider_driver: SN_AI_PROVIDER_DRIVER.to_string(),
                    provider_origin: ProviderOrigin::SystemConfig,
                    provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
                    provider_type_revision: None,
                    driver_metadata_generation: 0,
                    version: None,
                    inventory_revision: None,
                    models: Vec::<ModelMetadata>::new(),
                }
            })
    }

    fn legacy_instance(&self) -> Option<&ProviderInstance> {
        Some(&self.instance)
    }

    fn shutdown(&self) {
        self.stop_inventory_refresh();
    }

    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput {
        let usage = AiUsage {
            input_tokens: Some(input.input_tokens.max(1)),
            output_tokens: Some(input.estimated_output_tokens.unwrap_or(1024).max(1)),
            total_tokens: None,
            request_units: None,
        };
        let provider_model = provider_model_from_exact(input.exact_model.as_str());
        let estimated_cost_usd = self
            .estimate_cost_for_usage(provider_model.as_str(), &usage)
            .map(|cost| cost.amount)
            .or_else(|| {
                max_driver_metadata_cost(
                    SN_AI_PROVIDER_DRIVER,
                    &input.api_type,
                    usage.input_tokens.unwrap_or_default(),
                    usage.output_tokens.unwrap_or_default(),
                )
                .map(|(amount, _)| amount)
            })
            .unwrap_or_default();
        CostEstimateOutput {
            estimated_cost_usd,
            pricing_mode: PricingMode::PerToken,
            quota_state: QuotaState::Normal,
            confidence: 0.7,
            estimated_latency_ms: Some(1200),
        }
    }

    async fn refresh_inventory(&self) -> std::result::Result<ProviderInventory, ProviderError> {
        self.refresh_inventory_once()
            .await
            .map_err(|err| ProviderError::retryable(err.to_string()))
    }

    async fn start(
        &self,
        ctx: InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        match req.method.as_str() {
            ai_methods::LLM_CHAT => {
                self.start_llm(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::IMAGE_TXT2IMG => {
                self.start_text2image(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::IMAGE_IMG2IMG => {
                self.start_image_edit(&ctx, provider_model.as_str(), &req.request, false)
                    .await
            }
            ai_methods::IMAGE_INPAINT => {
                self.start_image_edit(&ctx, provider_model.as_str(), &req.request, true)
                    .await
            }
            ai_methods::EMBEDDING_TEXT => {
                self.start_embedding(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::RERANK => {
                self.start_rerank(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::AUDIO_TTS => {
                self.start_tts(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::AUDIO_ASR => {
                self.start_asr(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::VISION_OCR | ai_methods::VISION_CAPTION => {
                self.start_vision(
                    &ctx,
                    provider_model.as_str(),
                    req.method.as_str(),
                    &req.request,
                )
                .await
            }
            ai_methods::VIDEO_TXT2VIDEO | ai_methods::VIDEO_IMG2VIDEO => {
                self.start_video(
                    &ctx,
                    provider_model.as_str(),
                    req.method.as_str(),
                    &req.request,
                    sink,
                )
                .await
            }
            method => Err(ProviderError::fatal(format!(
                "sn-ai-provider does not support method '{}'",
                method
            ))),
        }
    }

    async fn cancel(
        &self,
        _ctx: InvokeCtx,
        _task_id: &str,
    ) -> std::result::Result<(), ProviderError> {
        Err(ProviderError::fatal(
            "sn-ai-provider cancellation is unsupported",
        ))
    }
}

impl Drop for SnAIProvider {
    fn drop(&mut self) {
        self.stop_inventory_refresh();
    }
}

pub fn register_sn_ai_provider(center: &AIComputeCenter, settings: &Value) -> Result<usize> {
    let Some(sn_settings) = parse_sn_ai_provider_settings(settings)? else {
        info!("aicc sn-ai-provider is disabled (settings.sn-ai-provider missing or disabled)");
        return Ok(0);
    };
    let instances = build_sn_ai_provider_instances(&sn_settings)?;
    let mut prepared = Vec::<(SnAIProviderInstanceConfig, Arc<SnAIProvider>)>::new();
    for config in instances.iter() {
        let provider = Arc::new(SnAIProvider::new(config.clone())?);
        prepared.push((config.clone(), provider));
    }

    for (config, provider) in prepared.into_iter() {
        provider.clone().start_inventory_refresh();
        let inventory = center.registry().add_provider(provider);
        info!(
            "registered sn-ai-provider base_url={} inventory={:?}",
            config.base_url, inventory
        );
        center
            .model_registry()
            .write()
            .map_err(|_| anyhow!("model registry lock poisoned"))?
            .apply_inventory(inventory)
            .map_err(|err| anyhow!("failed to apply sn-ai-provider inventory: {}", err))?;
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{AiMessage, AiPayload, Capability, ModelSpec, Requirements};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn test_instance_config() -> SnAIProviderInstanceConfig {
        SnAIProviderInstanceConfig {
            provider_instance_name: "sn-ai-provider-1".to_string(),
            provider_type: "cloud_api".to_string(),
            base_url: default_base_url(),
            login_url: "https://sn.buckyos.ai/api/user/login_by_device_token".to_string(),
            user_name: "alice".to_string(),
            timeout_ms: default_timeout_ms(),
        }
    }

    #[test]
    fn build_sn_ai_provider_instances_uses_endpoint_config() {
        let settings = SnAIProviderSettings {
            enabled: true,
            instances: vec![SettingsSnAIProviderInstanceConfig {
                provider_instance_name: "sn-ai-provider-1".to_string(),
                provider_type: "cloud_api".to_string(),
                base_url: "https://sn.buckyos.ai/api/v1/ai/".to_string(),
                login_url: "https://sn.buckyos.ai/api/user/login_by_device_token".to_string(),
                user_name: "alice".to_string(),
                timeout_ms: default_timeout_ms(),
            }],
        };

        let instances = build_sn_ai_provider_instances(&settings).expect("instances");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_instance_name, "sn-ai-provider-1");
    }

    #[test]
    fn provider_uses_metadata_models_for_supported_api_types() {
        let instances = build_sn_ai_provider_instances(&SnAIProviderSettings {
            enabled: true,
            instances: vec![SettingsSnAIProviderInstanceConfig {
                provider_instance_name: "sn-ai-provider-default".to_string(),
                provider_type: "cloud_api".to_string(),
                base_url: default_base_url(),
                login_url: "https://sn.buckyos.ai/api/user/login_by_device_token".to_string(),
                user_name: "alice".to_string(),
                timeout_ms: default_timeout_ms(),
            }],
        })
        .expect("instances");

        assert_eq!(instances.len(), 1);
        let provider = SnAIProvider::new(instances[0].clone()).expect("provider");
        assert_eq!(
            provider
                .inventory()
                .models
                .iter()
                .filter(|model| !model.provider_model_id.contains(':'))
                .map(|model| model.provider_model_id.clone())
                .collect::<Vec<_>>(),
            configured_model_ids()
        );
        let image = provider
            .inventory()
            .models
            .iter()
            .find(|model| model.provider_model_id == "gpt-image-2")
            .expect("metadata image model")
            .clone();
        assert!(image.supports_api_type(&ApiType::ImageTextToImage));
        assert!(image.supports_api_type(&ApiType::ImageToImage));
        assert!(image.supports_api_type(&ApiType::ImageInpaint));
        assert!(!image.supports_api_type(&ApiType::Llm));
        assert!(!provider
            .inventory()
            .models
            .into_iter()
            .any(|model| model.api_types.iter().any(|api_type| matches!(
                api_type,
                ApiType::VideoTextToVideo | ApiType::VideoImageToVideo
            ))));
    }

    #[test]
    fn parse_sn_ai_provider_settings_accepts_instance_id_alias() {
        let settings = json!({
            "sn-ai-provider": {
                "enabled": true,
                "instances": [
                    {
                        "instance_id": "sn-ai-provider-alias",
                        "login_url": "https://sn.buckyos.ai/api/user/login_by_device_token",
                        "user_name": "alice"
                    }
                ]
            }
        });

        let parsed = parse_sn_ai_provider_settings(&settings)
            .expect("parse")
            .expect("settings");
        assert_eq!(
            parsed.instances[0].provider_instance_name,
            "sn-ai-provider-alias"
        );
    }

    #[test]
    fn parse_sn_ai_provider_settings_skips_disabled_provider() {
        let settings = json!({
            "sn-ai-provider": {
                "enabled": false,
                "instances": [
                    {
                        "provider_instance_name": "sn-ai-provider-default",
                        "login_url": "https://sn.buckyos.ai/api/user/login_by_device_token",
                        "user_name": "alice"
                    }
                ]
            }
        });

        assert!(parse_sn_ai_provider_settings(&settings)
            .expect("parse")
            .is_none());
    }

    #[test]
    fn responses_endpoint_appends_responses_to_base_url() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        assert_eq!(
            provider.responses_endpoint(),
            "https://sn.buckyos.ai/api/v1/ai/responses"
        );
        assert_eq!(
            provider.models_endpoint(),
            "https://sn.buckyos.ai/api/v1/ai/models"
        );
        assert_eq!(
            provider.images_generations_endpoint(),
            "https://sn.buckyos.ai/api/v1/ai/v1/images/generations"
        );
        assert_eq!(
            provider.litellm_endpoint("embeddings"),
            "https://sn.buckyos.ai/api/v1/ai/v1/embeddings"
        );
        assert_eq!(
            provider.litellm_endpoint("audio/speech"),
            "https://sn.buckyos.ai/api/v1/ai/v1/audio/speech"
        );
        assert_eq!(
            provider.litellm_endpoint("videos/video-1/content"),
            "https://sn.buckyos.ai/api/v1/ai/v1/videos/video-1/content"
        );
    }

    #[test]
    fn sse_response_parsing_supports_responses_and_chat_chunks() {
        let responses = SnAIProvider::parse_sse_response(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello \"}\n\n\
             data: {\"type\":\"response.output_text.delta\",\"delta\":\"world\"}\n\n\
             data: [DONE]\n\n",
        )
        .expect("responses SSE");
        assert_eq!(responses.get("output_text"), Some(&json!("hello world")));

        let chat = SnAIProvider::parse_sse_response(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n\
             data: [DONE]\n\n",
        )
        .expect("chat SSE");
        assert_eq!(chat.get("output_text"), Some(&json!("hello")));
    }

    #[test]
    fn build_messages_keeps_user_image_blocks() {
        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::new(
                    AiRole::User,
                    vec![
                        AiContent::Text {
                            text: "what is in this image?".to_string(),
                        },
                        AiContent::Image {
                            source: ResourceRef::Base64 {
                                mime: "image/png".to_string(),
                                data_base64: "aGVsbG8=".to_string(),
                            },
                        },
                    ],
                )],
                vec![],
                vec![],
                None,
                None,
            ),
            None,
        );

        let messages = SnAIProvider::build_messages(&request).expect("messages");
        assert_eq!(
            messages[0]
                .pointer("/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            messages[0]
                .pointer("/content/1/type")
                .and_then(Value::as_str),
            Some("input_image")
        );
    }

    #[test]
    fn remote_items_inventory_is_normalized_as_sn_provider() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");

        let inventory = provider
            .build_inventory_from_remote_value(json!({
                "items": [
                    {"model": "gpt-5.4"},
                    {"model": "gpt-image-1"},
                    {"id": "gpt-5.4-mini"},
                    {"model": "vendor-new-model"}
                ]
            }))
            .expect("inventory");

        assert_eq!(inventory.provider_driver, SN_AI_PROVIDER_DRIVER);
        assert!(inventory
            .models
            .iter()
            .any(|model| model.provider_model_id == "gpt-5.4"));
        assert!(inventory
            .models
            .iter()
            .any(|model| model.provider_model_id == "vendor-new-model"));
        let image = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gpt-image-1")
            .expect("remote image model");
        assert!(image.supports_api_type(&ApiType::ImageTextToImage));
        assert!(!image.supports_api_type(&ApiType::Llm));
    }

    #[test]
    fn successful_empty_remote_inventory_clears_candidates() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");

        for body in [json!({ "items": [] }), json!({ "data": [] })] {
            let inventory = provider
                .build_inventory_from_remote_value(body)
                .expect("empty inventory snapshot");
            assert!(inventory.models.is_empty());
            assert!(inventory.inventory_revision.is_some());
        }
    }

    #[test]
    fn text2image_request_and_response_use_sn_proxy_shape() {
        let request = AiMethodRequest::new(
            Capability::Image,
            ModelSpec::new("image.txt2img".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                Some("draw a tiger".to_string()),
                vec![],
                vec![],
                vec![],
                Some(json!({ "size": "1024x1024" })),
                Some(json!({ "quality": "high" })),
            ),
            None,
        );
        let (body, ignored) =
            SnAIProvider::build_text2image_request("gpt-image-1", &request).expect("request");
        assert!(ignored.is_empty());
        assert_eq!(body.get("model"), Some(&json!("gpt-image-1")));
        assert_eq!(body.get("prompt"), Some(&json!("draw a tiger")));
        assert_eq!(body.get("size"), Some(&json!("1024x1024")));
        assert_eq!(body.get("quality"), Some(&json!("high")));

        let artifacts =
            SnAIProvider::parse_text2image_artifacts(&json!({"data": [{"b64_json": "aGVsbG8="}]}))
                .expect("artifacts");
        assert_eq!(artifacts.len(), 1);
        assert!(matches!(artifacts[0].resource, ResourceRef::Base64 { .. }));
    }

    #[tokio::test]
    async fn text2image_posts_to_sn_litellm_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = socket.read(&mut buffer).await.expect("read");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or_default();
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let response_body = r#"{"id":"image-1","data":[{"b64_json":"aGVsbG8="}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket.write_all(response.as_bytes()).await.expect("write");
            String::from_utf8(request).expect("request utf8")
        });

        let mut config = test_instance_config();
        config.base_url = format!("http://{}/api/v1/ai/", address);
        let provider = SnAIProvider::new(config).expect("provider");
        *provider.auth_session.lock().await = Some(CachedSnSession {
            session_token: "sn-session".to_string(),
            expires_at: u64::MAX,
        });
        let request = AiMethodRequest::new(
            Capability::Image,
            ModelSpec::new("image.txt2img".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                Some("draw a tiger".to_string()),
                vec![],
                vec![],
                vec![],
                None,
                None,
            ),
            None,
        );
        let result = provider
            .start_text2image(&InvokeCtx::default(), "gpt-image-1", &request)
            .await
            .expect("text2image");
        let ProviderStartResult::Immediate(response) = result else {
            panic!("expected immediate image response");
        };
        assert_eq!(response.artifacts().len(), 1);

        let raw_request = server.await.expect("server");
        assert!(raw_request.starts_with("POST /api/v1/ai/v1/images/generations HTTP/1.1"));
        assert!(raw_request
            .to_ascii_lowercase()
            .contains("authorization: bearer sn-session"));
        assert!(raw_request.contains(r#""model":"gpt-image-1""#));
        assert!(raw_request.contains(r#""prompt":"draw a tiger""#));
    }

    #[test]
    fn cost_estimate_uses_model_price_then_same_api_type_maximum_for_unknown_model() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        let refreshed_inventory = provider
            .build_inventory_from_remote_value(json!({
                "items": [
                    { "model": "gpt-5.4" },
                    { "model": "vendor-new-model" }
                ]
            }))
            .expect("remote inventory");
        *provider.inventory.write().expect("inventory") = refreshed_inventory;
        {
            let mut inventory = provider.inventory.write().expect("inventory");
            let metadata = inventory
                .models
                .iter_mut()
                .find(|metadata| metadata.provider_model_id == "gpt-5.4")
                .expect("configured model metadata");
            metadata.pricing.currency = "USD".to_string();
            metadata.pricing.input_token = Some(0.001);
            metadata.pricing.output_token = Some(0.002);
        }
        let known = provider.estimate_cost(&CostEstimateInput {
            api_type: ApiType::Llm,
            exact_model: "gpt-5.4@sn-ai-provider-1".to_string(),
            input_tokens: 1_000,
            estimated_output_tokens: Some(1_000),
            cached_input_tokens: None,
            request_features: vec![],
        });
        let unknown = provider.estimate_cost(&CostEstimateInput {
            api_type: ApiType::Llm,
            exact_model: "unknown@sn-ai-provider-1".to_string(),
            input_tokens: 1_000,
            estimated_output_tokens: Some(1_000),
            cached_input_tokens: None,
            request_features: vec![],
        });

        assert_eq!(known.estimated_cost_usd, 3.0);
        assert_eq!(unknown.estimated_cost_usd, 0.024);
        assert!(provider
            .estimate_cost_for_usage(
                "unknown",
                &AiUsage {
                    input_tokens: Some(1_000),
                    output_tokens: Some(1_000),
                    total_tokens: Some(2_000),
                    request_units: None,
                },
            )
            .is_none());
    }

    #[test]
    fn responses_output_extracts_text_and_tool_calls() {
        let body = json!({
            "output": [
                {
                    "type": "message",
                    "content": [{"type": "output_text", "text": "hello"}]
                },
                {
                    "type": "function_call",
                    "call_id": "call-1",
                    "name": "lookup",
                    "arguments": "{\"query\":\"buckyos\"}"
                }
            ]
        });

        assert_eq!(
            SnAIProvider::extract_text_content(&body).as_deref(),
            Some("hello")
        );
        let calls = SnAIProvider::extract_tool_choices(&body);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_id, "call-1");
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(calls[0].args.get("query"), Some(&json!("buckyos")));

        let message =
            SnAIProvider::message_from_response_body(&body, Some("hello".to_string()), calls);
        assert!(message.content.iter().any(|content| matches!(
            content,
            AiContent::ProviderState { provider, .. } if provider == SN_AI_PROVIDER_DRIVER
        )));

        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(None, vec![message], vec![], vec![], None, None),
            None,
        );
        let replay = SnAIProvider::build_messages(&request).expect("provider state replay");
        assert_eq!(
            replay,
            body["output"].as_array().expect("output array").clone()
        );

        let chat_body = json!({
            "choices": [{
                "message": {
                    "content": "chat reply",
                    "tool_calls": [{
                        "id": "call-2",
                        "type": "function",
                        "function": {
                            "name": "lookup",
                            "arguments": "{\"query\":\"liteLLM\"}"
                        }
                    }]
                }
            }]
        });
        assert_eq!(
            SnAIProvider::extract_text_content(&chat_body).as_deref(),
            Some("chat reply")
        );
        let chat_calls = SnAIProvider::extract_tool_choices(&chat_body);
        assert_eq!(chat_calls.len(), 1);
        assert_eq!(chat_calls[0].call_id, "call-2");
        assert_eq!(chat_calls[0].args.get("query"), Some(&json!("liteLLM")));
    }

    #[tokio::test]
    async fn sn_auth_ignores_invocation_session_and_uses_cached_sn_session() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        *provider.auth_session.lock().await = Some(CachedSnSession {
            session_token: "sn-sso-session".to_string(),
            expires_at: now + 300,
        });
        let ctx = InvokeCtx {
            session_token: Some("caller-session".to_string()),
            ..Default::default()
        };

        assert_eq!(
            provider.build_auth_token(&ctx).await.expect("token"),
            "sn-sso-session"
        );
    }

    #[tokio::test]
    async fn rejected_auth_token_does_not_clear_a_concurrently_refreshed_session() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        *provider.auth_session.lock().await = Some(CachedSnSession {
            session_token: "refreshed-session".to_string(),
            expires_at: u64::MAX,
        });

        provider.invalidate_auth_token("rejected-session").await;
        assert_eq!(
            provider
                .auth_session
                .lock()
                .await
                .as_ref()
                .map(|session| session.session_token.as_str()),
            Some("refreshed-session")
        );

        provider.invalidate_auth_token("refreshed-session").await;
        assert!(provider.auth_session.lock().await.is_none());
    }

    #[test]
    fn unauthorized_after_relogin_allows_runtime_failover() {
        let error = SnAIProvider::classify_api_error(
            StatusCode::UNAUTHORIZED,
            "SN request returned 401".to_string(),
        );

        assert!(error.is_retryable());
    }

    #[test]
    fn dropping_provider_stops_refresh_task() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        let (refresh_task, _) = ProviderRefreshTask::new();
        *provider.refresh_task.lock().expect("refresh task lock") = Some(refresh_task.clone());

        drop(provider);

        assert!(refresh_task.is_stopped());
    }
}
