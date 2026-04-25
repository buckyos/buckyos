use crate::aicc::{
    image_logical_mounts, llm_logical_mounts, provider_model_metadata, provider_type_from_settings,
    AIComputeCenter, Provider, ProviderError, ProviderInstance, ProviderStartResult,
    ResolvedRequest, TaskEventSink,
};
use crate::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, PricingMode, ProviderInventory, ProviderOrigin,
    ProviderTypeTrustedSource, QuotaState,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    features, AiArtifact, AiCost, AiMethodRequest, AiResponseSummary, AiUsage, Capability, Feature,
    ResourceRef,
};
use log::{info, warn};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_GIMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_GIMINI_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_GIMINI_MODELS: &str = "gemini-2.5-flash,gemini-2.5-pro";
const DEFAULT_GIMINI_IMAGE_MODELS: &str =
    "gemini-2.0-flash-exp-image-generation,gemini-2.5-flash-image-preview";
const GIMINI_IMAGE_INPUT_ALLOWLIST: &[&str] = &[
    "candidate_count",
    "max_output_tokens",
    "n",
    "prompt",
    "response_mime_type",
    "response_modalities",
    "seed",
    "stop",
    "temperature",
    "top_k",
    "top_p",
];
const GIMINI_IMAGE_OPTION_ALLOWLIST: &[&str] = &[
    "candidate_count",
    "max_output_tokens",
    "n",
    "output_format",
    "quality",
    "response_mime_type",
    "response_modalities",
    "seed",
    "size",
    "stop",
    "style",
    "temperature",
    "top_k",
    "top_p",
    "user",
];

#[derive(Debug, Clone)]
pub struct GoogleGiminiInstanceConfig {
    pub provider_instance_name: String,
    pub provider_type: String,
    pub provider_driver: String,
    pub base_url: String,
    pub timeout_ms: u64,
    pub models: Vec<String>,
    pub default_model: Option<String>,
    pub image_models: Vec<String>,
    pub default_image_model: Option<String>,
    pub features: Vec<Feature>,
    pub alias_map: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct GoogleGiminiProvider {
    instance: ProviderInstance,
    inventory: ProviderInventory,
    client: Client,
    api_token: String,
    base_url: String,
}

impl GoogleGiminiProvider {
    pub fn new(cfg: GoogleGiminiInstanceConfig, api_token: String) -> Result<Self> {
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_GIMINI_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for google gimini provider")?;

        let provider_type = provider_type_from_settings(cfg.provider_type.as_str());
        let provider_instance_name = cfg.provider_instance_name.clone();
        let provider_driver = cfg.provider_driver.clone();
        let instance = ProviderInstance {
            provider_instance_name: provider_instance_name.clone(),
            provider_type: provider_type.clone(),
            provider_driver: provider_driver.clone(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            capabilities: vec![Capability::Llm, Capability::Image],
            features: cfg.features.clone(),
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };
        let mut models = Vec::new();
        for model in cfg
            .models
            .iter()
            .filter(|model| !is_text2image_model_name(model))
        {
            models.push(provider_model_metadata(
                provider_instance_name.as_str(),
                provider_type.clone(),
                model.as_str(),
                ApiType::LlmChat,
                llm_logical_mounts(provider_driver.as_str(), model.as_str()),
                &cfg.features,
                Some(0.01),
                Some(1400),
            ));
        }
        for model in cfg.image_models.iter() {
            models.push(provider_model_metadata(
                provider_instance_name.as_str(),
                provider_type.clone(),
                model.as_str(),
                ApiType::ImageTextToImage,
                image_logical_mounts(provider_driver.as_str(), model.as_str()),
                &cfg.features,
                Some(0.04),
                Some(6000),
            ));
        }
        let inventory = ProviderInventory {
            provider_instance_name,
            provider_type,
            provider_driver,
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            version: None,
            inventory_revision: Some("settings-v1".to_string()),
            models,
        };

        Ok(Self {
            instance,
            inventory,
            client,
            api_token,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
        })
    }

    fn price_per_1m_tokens(model: &str) -> (f64, f64) {
        let lowered = model.to_ascii_lowercase();
        if lowered.contains("2.5-pro") {
            (1.25, 10.0)
        } else if lowered.contains("2.5-flash") {
            (0.30, 2.50)
        } else if lowered.contains("1.5-pro") {
            (1.25, 5.0)
        } else if lowered.contains("1.5-flash") {
            (0.075, 0.30)
        } else {
            (0.50, 2.0)
        }
    }

    fn estimate_tokens(req: &AiMethodRequest) -> (u64, u64) {
        let mut text_len = 0usize;

        if let Some(text) = req.payload.text.as_ref() {
            text_len += text.len();
        }

        for message in req.payload.messages.iter() {
            text_len += message.content.len();
        }

        for resource in req.payload.resources.iter() {
            match resource {
                ResourceRef::Url { url, .. } => {
                    text_len += url.len();
                }
                ResourceRef::NamedObject { obj_id } => {
                    text_len += obj_id.to_string().len();
                }
                ResourceRef::Base64 { .. } => {
                    text_len += 256;
                }
            }
        }

        let input_tokens = ((text_len as f64) / 4.0).ceil() as u64;
        let output_tokens = req
            .payload
            .options
            .as_ref()
            .and_then(|value| value.get("max_tokens"))
            .and_then(|value| value.as_u64())
            .or_else(|| {
                req.payload
                    .options
                    .as_ref()
                    .and_then(|value| value.get("max_output_tokens"))
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(1024);

        (input_tokens.max(1), output_tokens.max(1))
    }

    fn estimate_image_count(req: &AiMethodRequest) -> u64 {
        req.payload
            .options
            .as_ref()
            .and_then(|value| value.get("n"))
            .and_then(|value| value.as_u64())
            .or_else(|| {
                req.payload
                    .options
                    .as_ref()
                    .and_then(|value| value.get("candidate_count"))
                    .and_then(|value| value.as_u64())
            })
            .or_else(|| {
                req.payload
                    .input_json
                    .as_ref()
                    .and_then(|value| value.get("n"))
                    .and_then(|value| value.as_u64())
            })
            .or_else(|| {
                req.payload
                    .input_json
                    .as_ref()
                    .and_then(|value| value.get("candidate_count"))
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(1)
            .max(1)
    }

    fn estimate_text2image_cost(req: &AiMethodRequest, model: &str) -> Option<f64> {
        let lowered = model.to_ascii_lowercase();
        let per_image = if lowered.contains("2.5-flash-image") {
            0.039
        } else if lowered.contains("2.0-flash-exp-image-generation")
            || lowered.contains("2.0-flash-preview-image-generation")
        {
            0.03
        } else if lowered.contains("2.5") {
            0.04
        } else {
            0.03
        };
        Some((Self::estimate_image_count(req) as f64) * per_image)
    }

    fn estimate_cost_for_usage(&self, model: &str, usage: &AiUsage) -> Option<AiCost> {
        let input_tokens = usage.input_tokens? as f64;
        let output_tokens = usage.output_tokens? as f64;
        let (input_per_m, output_per_m) = Self::price_per_1m_tokens(model);

        let amount = ((input_tokens / 1_000_000.0) * input_per_m)
            + ((output_tokens / 1_000_000.0) * output_per_m);

        Some(AiCost {
            amount,
            currency: "USD".to_string(),
        })
    }

    fn role_to_gimini(role: &str) -> &'static str {
        match role.trim().to_ascii_lowercase().as_str() {
            "assistant" => "model",
            _ => "user",
        }
    }

    fn build_contents(&self, req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut contents = vec![];

        for msg in req.payload.messages.iter() {
            if msg.content.trim().is_empty() {
                continue;
            }
            contents.push(json!({
                "role": Self::role_to_gimini(msg.role.as_str()),
                "parts": [
                    {
                        "text": msg.content
                    }
                ]
            }));
        }

        if contents.is_empty() {
            let mut content = String::new();
            if let Some(text) = req.payload.text.as_ref() {
                content.push_str(text);
            }

            let mut resource_lines = vec![];
            for resource in req.payload.resources.iter() {
                match resource {
                    ResourceRef::Url { url, .. } => {
                        resource_lines.push(format!("resource_url: {}", url));
                    }
                    ResourceRef::NamedObject { obj_id } => {
                        resource_lines.push(format!("named_object: {}", obj_id));
                    }
                    ResourceRef::Base64 { .. } => {
                        return Err(ProviderError::fatal(
                            "google gimini provider does not support base64 resources in this version",
                        ));
                    }
                }
            }

            if !resource_lines.is_empty() {
                if !content.is_empty() {
                    content.push('\n');
                    content.push('\n');
                }
                content.push_str(resource_lines.join("\n").as_str());
            }

            if !content.trim().is_empty() {
                contents.push(json!({
                    "role": "user",
                    "parts": [
                        {
                            "text": content
                        }
                    ]
                }));
            }
        }

        if contents.is_empty() {
            return Err(ProviderError::fatal(
                "request payload has no usable text/messages for llm",
            ));
        }

        Ok(contents)
    }

    fn extract_text_content(body: &Value) -> Option<String> {
        let parts = body.pointer("/candidates/0/content/parts")?.as_array()?;
        let joined = parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    fn classify_api_error(status: StatusCode, message: String) -> ProviderError {
        if status.as_u16() == 429 || status.is_server_error() {
            ProviderError::retryable(message)
        } else {
            ProviderError::fatal(message)
        }
    }

    fn extract_text2image_prompt(req: &AiMethodRequest) -> Option<String> {
        if let Some(text) = req
            .payload
            .text
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Some(text.to_string());
        }

        let message_prompt = req
            .payload
            .messages
            .iter()
            .map(|msg| msg.content.trim())
            .filter(|msg| !msg.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !message_prompt.is_empty() {
            return Some(message_prompt);
        }

        if let Some(prompt) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("prompt"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Some(prompt.to_string());
        }

        req.payload
            .options
            .as_ref()
            .and_then(|value| value.get("prompt"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    }

    fn normalize_stop_sequences(stop: &Value) -> Result<Value, ProviderError> {
        if let Some(stop_str) = stop.as_str() {
            return Ok(Value::Array(vec![Value::String(stop_str.to_string())]));
        }

        let Some(stop_values) = stop.as_array() else {
            return Err(ProviderError::fatal(
                "stop must be a string or array of strings",
            ));
        };

        let mut normalized = Vec::with_capacity(stop_values.len());
        for (idx, item) in stop_values.iter().enumerate() {
            let Some(stop_str) = item.as_str() else {
                return Err(ProviderError::fatal(format!(
                    "stop[{}] must be a string",
                    idx
                )));
            };
            normalized.push(Value::String(stop_str.to_string()));
        }

        Ok(Value::Array(normalized))
    }

    fn ensure_generation_config(target: &mut Map<String, Value>) -> &mut Map<String, Value> {
        if !target.contains_key("generationConfig") {
            target.insert("generationConfig".to_string(), Value::Object(Map::new()));
        }
        target
            .get_mut("generationConfig")
            .and_then(|value| value.as_object_mut())
            .expect("generationConfig should be an object")
    }

    fn merge_llm_options(
        target: &mut Map<String, Value>,
        options: &Value,
        json_output_required: bool,
    ) -> Result<Vec<String>, ProviderError> {
        let Some(options_map) = options.as_object() else {
            if json_output_required {
                let generation = Self::ensure_generation_config(target);
                if !generation.contains_key("responseMimeType") {
                    generation.insert(
                        "responseMimeType".to_string(),
                        Value::String("application/json".to_string()),
                    );
                }
            }
            return Ok(vec![]);
        };

        let mut ignored = vec![];
        for (key, value) in options_map.iter() {
            if key == "model" || key == "messages" {
                continue;
            }
            if key == "protocol" || key == "process_name" || key == "tool_messages" {
                ignored.push(key.clone());
                continue;
            }

            match key.as_str() {
                "temperature" => {
                    Self::ensure_generation_config(target)
                        .insert("temperature".to_string(), value.clone());
                }
                "top_p" | "topP" => {
                    Self::ensure_generation_config(target)
                        .insert("topP".to_string(), value.clone());
                }
                "top_k" | "topK" => {
                    Self::ensure_generation_config(target)
                        .insert("topK".to_string(), value.clone());
                }
                "max_tokens" | "max_completion_tokens" | "max_output_tokens" => {
                    Self::ensure_generation_config(target)
                        .insert("maxOutputTokens".to_string(), value.clone());
                }
                "candidate_count" => {
                    Self::ensure_generation_config(target)
                        .insert("candidateCount".to_string(), value.clone());
                }
                "stop" => {
                    Self::ensure_generation_config(target).insert(
                        "stopSequences".to_string(),
                        Self::normalize_stop_sequences(value)?,
                    );
                }
                "response_mime_type" => {
                    Self::ensure_generation_config(target)
                        .insert("responseMimeType".to_string(), value.clone());
                }
                "response_schema" => {
                    let generation = Self::ensure_generation_config(target);
                    generation.insert("responseSchema".to_string(), value.clone());
                    if !generation.contains_key("responseMimeType") {
                        generation.insert(
                            "responseMimeType".to_string(),
                            Value::String("application/json".to_string()),
                        );
                    }
                }
                _ => {
                    ignored.push(key.clone());
                }
            }
        }

        if json_output_required {
            let generation = Self::ensure_generation_config(target);
            if !generation.contains_key("responseMimeType") {
                generation.insert(
                    "responseMimeType".to_string(),
                    Value::String("application/json".to_string()),
                );
            }
        }

        Ok(ignored)
    }

    fn merge_text2image_input_json(
        target: &mut Map<String, Value>,
        input_json: &Value,
    ) -> Result<(), ProviderError> {
        let Some(input_map) = input_json.as_object() else {
            return Ok(());
        };

        for (key, value) in input_map.iter() {
            if !GIMINI_IMAGE_INPUT_ALLOWLIST.contains(&key.as_str()) {
                continue;
            }
            match key.as_str() {
                "prompt" => {
                    target.insert("prompt".to_string(), value.clone());
                }
                "response_modalities" => {
                    Self::ensure_generation_config(target)
                        .insert("responseModalities".to_string(), value.clone());
                }
                "response_mime_type" => {
                    Self::ensure_generation_config(target)
                        .insert("responseMimeType".to_string(), value.clone());
                }
                "max_output_tokens" => {
                    Self::ensure_generation_config(target)
                        .insert("maxOutputTokens".to_string(), value.clone());
                }
                "candidate_count" | "n" => {
                    Self::ensure_generation_config(target)
                        .insert("candidateCount".to_string(), value.clone());
                }
                "stop" => {
                    Self::ensure_generation_config(target).insert(
                        "stopSequences".to_string(),
                        Self::normalize_stop_sequences(value)?,
                    );
                }
                "temperature" => {
                    Self::ensure_generation_config(target)
                        .insert("temperature".to_string(), value.clone());
                }
                "top_k" => {
                    Self::ensure_generation_config(target)
                        .insert("topK".to_string(), value.clone());
                }
                "top_p" => {
                    Self::ensure_generation_config(target)
                        .insert("topP".to_string(), value.clone());
                }
                "seed" => {
                    Self::ensure_generation_config(target)
                        .insert("seed".to_string(), value.clone());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn merge_text2image_options(
        target: &mut Map<String, Value>,
        options: &Value,
    ) -> Result<Vec<String>, ProviderError> {
        let Some(options_map) = options.as_object() else {
            return Ok(vec![]);
        };

        let mut ignored = vec![];
        for (key, value) in options_map.iter() {
            if key == "model" || key == "messages" {
                continue;
            }
            if key == "protocol" || key == "process_name" || key == "tool_messages" {
                ignored.push(key.clone());
                continue;
            }
            if !GIMINI_IMAGE_OPTION_ALLOWLIST.contains(&key.as_str()) && key != "prompt" {
                ignored.push(key.clone());
                continue;
            }

            match key.as_str() {
                "prompt" => {
                    target.insert("prompt".to_string(), value.clone());
                }
                "response_modalities" => {
                    Self::ensure_generation_config(target)
                        .insert("responseModalities".to_string(), value.clone());
                }
                "response_mime_type" | "output_format" => {
                    Self::ensure_generation_config(target)
                        .insert("responseMimeType".to_string(), value.clone());
                }
                "max_output_tokens" => {
                    Self::ensure_generation_config(target)
                        .insert("maxOutputTokens".to_string(), value.clone());
                }
                "candidate_count" | "n" => {
                    Self::ensure_generation_config(target)
                        .insert("candidateCount".to_string(), value.clone());
                }
                "stop" => {
                    Self::ensure_generation_config(target).insert(
                        "stopSequences".to_string(),
                        Self::normalize_stop_sequences(value)?,
                    );
                }
                "temperature" => {
                    Self::ensure_generation_config(target)
                        .insert("temperature".to_string(), value.clone());
                }
                "top_k" => {
                    Self::ensure_generation_config(target)
                        .insert("topK".to_string(), value.clone());
                }
                "top_p" => {
                    Self::ensure_generation_config(target)
                        .insert("topP".to_string(), value.clone());
                }
                "seed" => {
                    Self::ensure_generation_config(target)
                        .insert("seed".to_string(), value.clone());
                }
                _ => {
                    ignored.push(key.clone());
                }
            }
        }
        Ok(ignored)
    }

    fn parse_text2image_result(
        body: &Value,
    ) -> Result<(Vec<AiArtifact>, Option<String>), ProviderError> {
        let Some(candidates) = body.get("candidates").and_then(|value| value.as_array()) else {
            return Err(ProviderError::fatal(
                "google gimini image response is missing candidates array",
            ));
        };

        let mut artifacts = vec![];
        let mut text_notes = vec![];
        for candidate in candidates.iter() {
            if let Some(parts) = candidate
                .pointer("/content/parts")
                .and_then(|value| value.as_array())
            {
                for part in parts.iter() {
                    if let Some(inline_data) =
                        part.get("inlineData").and_then(|value| value.as_object())
                    {
                        let Some(data_base64) =
                            inline_data.get("data").and_then(|value| value.as_str())
                        else {
                            continue;
                        };
                        let mime = inline_data
                            .get("mimeType")
                            .and_then(|value| value.as_str())
                            .unwrap_or("image/png");
                        if general_purpose::STANDARD.decode(data_base64).is_err() {
                            warn!(
                                "aicc.gimini received invalid inlineData base64 in image response"
                            );
                            continue;
                        }

                        let seq = artifacts.len() + 1;
                        artifacts.push(AiArtifact {
                            name: format!("image_{}", seq),
                            resource: ResourceRef::Base64 {
                                mime: mime.to_string(),
                                data_base64: data_base64.to_string(),
                            },
                            mime: Some(mime.to_string()),
                            metadata: None,
                        });
                        continue;
                    }

                    if let Some(file_data) =
                        part.get("fileData").and_then(|value| value.as_object())
                    {
                        let Some(uri) = file_data.get("fileUri").and_then(|value| value.as_str())
                        else {
                            continue;
                        };
                        let mime = file_data
                            .get("mimeType")
                            .and_then(|value| value.as_str())
                            .unwrap_or("image/png");
                        let seq = artifacts.len() + 1;
                        artifacts.push(AiArtifact {
                            name: format!("image_{}", seq),
                            resource: ResourceRef::Url {
                                url: uri.to_string(),
                                mime_hint: Some(mime.to_string()),
                            },
                            mime: Some(mime.to_string()),
                            metadata: None,
                        });
                        continue;
                    }

                    if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                        if !text.trim().is_empty() {
                            text_notes.push(text.trim().to_string());
                        }
                    }
                }
            }
        }

        if artifacts.is_empty() {
            return Err(ProviderError::fatal(
                "google gimini image response has no usable image outputs",
            ));
        }

        let text = if text_notes.is_empty() {
            None
        } else {
            Some(text_notes.join("\n"))
        };
        Ok((artifacts, text))
    }

    async fn post_generate_content(
        &self,
        provider_model: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let url = format!(
            "{}/models/{}:generateContent",
            self.base_url, provider_model
        );
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url.as_str())
            .header("x-goog-api-key", self.api_token.as_str())
            .json(request_obj)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() || err.is_connect() {
                    ProviderError::retryable(format!("google gimini request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("google gimini request failed: {}", err))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;

        let status = response.status();
        let body: Value = response.json().await.map_err(|err| {
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(format!(
                    "failed to parse google gimini response body: {}",
                    err
                ))
            } else {
                ProviderError::fatal(format!(
                    "failed to parse google gimini response body: {}",
                    err
                ))
            }
        })?;

        Ok((status, body, latency_ms))
    }

    async fn start_llm(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let contents = self.build_contents(req)?;
        let mut request_obj = Map::new();
        request_obj.insert("contents".to_string(), Value::Array(contents));

        let json_output_required = req
            .requirements
            .must_features
            .iter()
            .any(|feature| feature == features::JSON_OUTPUT);
        let mut ignored_options = vec![];
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options =
                Self::merge_llm_options(&mut request_obj, options, json_output_required)?;
        } else if json_output_required {
            let generation = Self::ensure_generation_config(&mut request_obj);
            generation.insert(
                "responseMimeType".to_string(),
                Value::String("application/json".to_string()),
            );
        }

        if !ignored_options.is_empty() {
            warn!(
                "aicc.gimini ignored unsupported llm options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let request_log = Value::Object(request_obj.clone()).to_string();
        info!(
            "aicc.gimini.llm.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        let response_log = body.to_string();

        if !status.is_success() {
            warn!(
                "aicc.gimini.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("google gimini api error [{}]: {}", code, message),
            ));
        }

        info!(
            "aicc.gimini.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let content = Self::extract_text_content(&body);
        let usage = body.get("usageMetadata").map(|usage| AiUsage {
            input_tokens: usage
                .get("promptTokenCount")
                .and_then(|value| value.as_u64()),
            output_tokens: usage
                .get("candidatesTokenCount")
                .and_then(|value| value.as_u64()),
            total_tokens: usage
                .get("totalTokenCount")
                .and_then(|value| value.as_u64()),
        });

        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));

        let mut extra = Map::new();
        extra.insert(
            "provider".to_string(),
            Value::String("google_gimini".to_string()),
        );
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": Value::Object(request_obj.clone()),
                "output": body.clone()
            }),
        );

        let summary = AiResponseSummary {
            text: content,
            tool_calls: vec![],
            artifacts: vec![],
            usage,
            cost,
            finish_reason: body
                .pointer("/candidates/0/finishReason")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            provider_task_ref: None,
            extra: Some(Value::Object(extra)),
        };

        Ok(ProviderStartResult::Immediate(summary))
    }

    async fn start_text2image(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let mut request_obj = Map::new();
        if let Some(input_json) = req.payload.input_json.as_ref() {
            Self::merge_text2image_input_json(&mut request_obj, input_json)?;
        }

        if let Some(prompt) = Self::extract_text2image_prompt(req) {
            request_obj.insert("prompt".to_string(), Value::String(prompt));
        }
        if !request_obj.contains_key("prompt") {
            return Err(ProviderError::fatal(
                "text2image request requires prompt in payload.text/messages/input_json/options",
            ));
        }

        let mut ignored_options = vec![];
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options = Self::merge_text2image_options(&mut request_obj, options)?;
        }
        if !ignored_options.is_empty() {
            warn!(
                "aicc.gimini ignored unsupported text2image options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let prompt = request_obj
            .get("prompt")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ProviderError::fatal("text2image prompt must be a string"))?
            .to_string();

        let generation = Self::ensure_generation_config(&mut request_obj);
        if !generation.contains_key("responseModalities") {
            generation.insert("responseModalities".to_string(), json!(["IMAGE"]));
        }
        let contents = json!([
            {
                "role": "user",
                "parts": [
                    {
                        "text": prompt
                    }
                ]
            }
        ]);
        request_obj.insert("contents".to_string(), contents);
        request_obj.remove("prompt");

        let request_log = Value::Object(request_obj.clone()).to_string();
        info!(
            "aicc.gimini.text2image.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        let response_log = body.to_string();

        if !status.is_success() {
            warn!(
                "aicc.gimini.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("google gimini api error [{}]: {}", code, message),
            ));
        }

        info!(
            "aicc.gimini.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let (artifacts, text) = Self::parse_text2image_result(&body)?;
        let estimated_cost =
            Self::estimate_text2image_cost(req, provider_model).map(|amount| AiCost {
                amount,
                currency: "USD".to_string(),
            });

        let mut extra = Map::new();
        extra.insert(
            "provider".to_string(),
            Value::String("google_gimini".to_string()),
        );
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": Value::Object(request_obj.clone()),
                "output": body.clone()
            }),
        );

        let summary = AiResponseSummary {
            text,
            tool_calls: vec![],
            artifacts,
            usage: None,
            cost: estimated_cost,
            finish_reason: body
                .pointer("/candidates/0/finishReason")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            provider_task_ref: None,
            extra: Some(Value::Object(extra)),
        };

        Ok(ProviderStartResult::Immediate(summary))
    }
}

#[async_trait]
impl Provider for GoogleGiminiProvider {
    fn inventory(&self) -> ProviderInventory {
        self.inventory.clone()
    }

    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput {
        let provider_model = provider_model_from_exact(input.exact_model.as_str());
        if input.api_type == ApiType::ImageTextToImage {
            return CostEstimateOutput {
                estimated_cost_usd: 0.04,
                pricing_mode: PricingMode::PerToken,
                quota_state: QuotaState::Normal,
                confidence: 0.5,
                estimated_latency_ms: Some(6000),
            };
        }

        let input_tokens = input.input_tokens.max(1);
        let output_tokens = input.estimated_output_tokens.unwrap_or(1024).max(1);
        let usage = AiUsage {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: Some(input_tokens.saturating_add(output_tokens)),
        };

        let estimated_cost_usd = self
            .estimate_cost_for_usage(provider_model, &usage)
            .map(|cost| cost.amount)
            .unwrap_or(1.0);

        CostEstimateOutput {
            estimated_cost_usd,
            pricing_mode: PricingMode::PerToken,
            quota_state: QuotaState::Normal,
            confidence: 0.7,
            estimated_latency_ms: Some(1400),
        }
    }

    async fn start(
        &self,
        ctx: crate::aicc::InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        _sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        match req.request.capability {
            Capability::Llm => {
                self.start_llm(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            Capability::Image => {
                self.start_text2image(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            capability => Err(ProviderError::fatal(format!(
                "google gimini provider does not support capability '{:?}'",
                capability
            ))),
        }
    }

    async fn cancel(
        &self,
        _ctx: crate::aicc::InvokeCtx,
        _task_id: &str,
    ) -> std::result::Result<(), ProviderError> {
        Ok(())
    }
}

fn provider_model_from_exact(exact_model: &str) -> &str {
    exact_model
        .rsplit_once('@')
        .map(|(model, _)| model)
        .unwrap_or(exact_model)
}

#[derive(Debug, Deserialize, Default)]
struct GiminiSettings {
    #[serde(default = "default_gimini_enabled")]
    enabled: bool,
    #[serde(default, alias = "api_key", alias = "apiKey")]
    api_token: String,
    #[serde(default)]
    alias_map: HashMap<String, String>,
    #[serde(default)]
    instances: Vec<SettingsGoogleGiminiInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsGoogleGiminiInstanceConfig {
    #[serde(default = "default_instance_id")]
    provider_instance_name: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default = "default_provider_driver")]
    provider_driver: String,
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    image_models: Vec<String>,
    #[serde(default)]
    default_image_model: Option<String>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    alias_map: HashMap<String, String>,
}

fn default_gimini_enabled() -> bool {
    true
}

fn default_instance_id() -> String {
    "google-gimini-default".to_string()
}

fn default_provider_type() -> String {
    "cloud_api".to_string()
}

fn default_provider_driver() -> String {
    "google-gimini".to_string()
}

fn default_base_url() -> String {
    DEFAULT_GIMINI_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_GIMINI_TIMEOUT_MS
}

fn default_features() -> Vec<String> {
    vec![
        features::PLAN.to_string(),
        features::JSON_OUTPUT.to_string(),
    ]
}

fn is_text2image_model_name(model: &str) -> bool {
    let lowered = model.trim().to_ascii_lowercase();
    lowered.contains("image") || lowered.contains("nano-banana")
}

fn normalize_model_list(models: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::<String>::new();
    let mut normalized = vec![];
    for model in models.into_iter() {
        let value = model.trim();
        if value.is_empty() {
            continue;
        }
        let key = value.to_ascii_lowercase();
        if seen.insert(key) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect::<Vec<_>>()
}

fn parse_gimini_settings(settings: &Value) -> Result<Option<GiminiSettings>> {
    let raw = settings
        .get("gimini")
        .or_else(|| settings.get("gemini"))
        .or_else(|| settings.get("google_gimini"))
        .or_else(|| settings.get("google"));
    let Some(raw_settings) = raw else {
        return Ok(None);
    };
    if raw_settings.is_null() {
        return Ok(None);
    }

    let gimini_settings = serde_json::from_value::<GiminiSettings>(raw_settings.clone())
        .map_err(|err| anyhow!("failed to parse gimini settings: {}", err))?;
    if !gimini_settings.enabled {
        return Ok(None);
    }

    Ok(Some(gimini_settings))
}

fn build_gimini_instances(settings: &GiminiSettings) -> Result<Vec<GoogleGiminiInstanceConfig>> {
    let raw_instances = if settings.instances.is_empty() {
        vec![SettingsGoogleGiminiInstanceConfig {
            provider_instance_name: default_instance_id(),
            provider_type: default_provider_type(),
            provider_driver: default_provider_driver(),
            base_url: default_base_url(),
            timeout_ms: default_timeout_ms(),
            models: vec![],
            default_model: None,
            image_models: vec![],
            default_image_model: None,
            features: vec![],
            alias_map: HashMap::new(),
        }]
    } else {
        settings.instances.clone()
    };

    let mut instances = vec![];
    for raw_instance in raw_instances.into_iter() {
        let mut models = normalize_model_list(raw_instance.models);
        if models.is_empty() {
            models = normalize_model_list(parse_csv_list(DEFAULT_GIMINI_MODELS));
        }
        if models.is_empty() {
            return Err(anyhow!(
                "gimini instance {} has no models configured",
                raw_instance.provider_instance_name
            ));
        }

        let default_model = raw_instance
            .default_model
            .or_else(|| {
                models
                    .iter()
                    .find(|model| !is_text2image_model_name(model))
                    .cloned()
            })
            .or_else(|| models.first().cloned());
        let mut image_models = normalize_model_list(raw_instance.image_models);
        if image_models.is_empty() {
            image_models = models
                .iter()
                .filter(|model| is_text2image_model_name(model))
                .cloned()
                .collect::<Vec<_>>();
        }
        if image_models.is_empty() {
            image_models = normalize_model_list(parse_csv_list(DEFAULT_GIMINI_IMAGE_MODELS));
        }
        let default_image_model = raw_instance
            .default_image_model
            .or_else(|| image_models.first().cloned());
        let features = if raw_instance.features.is_empty() {
            default_features()
        } else {
            raw_instance.features
        };

        instances.push(GoogleGiminiInstanceConfig {
            provider_instance_name: raw_instance.provider_instance_name,
            provider_type: raw_instance.provider_type,
            provider_driver: raw_instance.provider_driver,
            base_url: raw_instance.base_url,
            timeout_ms: raw_instance.timeout_ms,
            models,
            default_model,
            image_models,
            default_image_model,
            features,
            alias_map: raw_instance.alias_map,
        });
    }

    Ok(instances)
}

fn register_default_aliases(
    center: &AIComputeCenter,
    provider_type: &str,
    models: &[String],
    default_model: Option<&str>,
    image_models: &[String],
    default_image_model: Option<&str>,
) {
    for model in models.iter() {
        if is_text2image_model_name(model) {
            continue;
        }
        center.model_catalog().set_mapping(
            Capability::Llm,
            model.as_str(),
            provider_type,
            model.as_str(),
        );

        center.model_catalog().set_mapping(
            Capability::Llm,
            format!("llm.{}", model),
            provider_type,
            model.as_str(),
        );
    }

    if let Some(default_model) = default_model.filter(|model| !is_text2image_model_name(model)) {
        for alias in [
            "llm.default",
            "llm.chat.default",
            "llm.plan.default",
            "llm.code.default",
        ] {
            center.model_catalog().set_mapping(
                Capability::Llm,
                alias,
                provider_type,
                default_model,
            );
        }
    }

    for model in image_models.iter() {
        center.model_catalog().set_mapping(
            Capability::Image,
            model.as_str(),
            provider_type,
            model.as_str(),
        );
        for alias in [
            format!("text2image.{}", model),
            format!("t2i.{}", model),
            format!("image.{}", model),
        ] {
            center.model_catalog().set_mapping(
                Capability::Image,
                alias,
                provider_type,
                model.as_str(),
            );
        }
    }

    if let Some(default_image_model) = default_image_model {
        for alias in [
            "text2image.default",
            "t2i.default",
            "image.default",
            "text2image.nano_banana",
            "t2i.nano_banana",
        ] {
            center.model_catalog().set_mapping(
                Capability::Image,
                alias,
                provider_type,
                default_image_model,
            );
        }
    }
}

fn register_custom_aliases(
    center: &AIComputeCenter,
    provider_type: &str,
    alias_map: &HashMap<String, String>,
) {
    for (alias, model) in alias_map.iter() {
        let normalized_alias = alias.to_ascii_lowercase();
        let capability = if normalized_alias.starts_with("text2image.")
            || normalized_alias.starts_with("t2i.")
            || normalized_alias.starts_with("image.")
        {
            Capability::Image
        } else {
            Capability::Llm
        };
        center.model_catalog().set_mapping(
            capability,
            alias.as_str(),
            provider_type,
            model.as_str(),
        );
    }
}

pub fn register_google_gimini_providers(
    center: &AIComputeCenter,
    settings: &Value,
) -> Result<usize> {
    let Some(gimini_settings) = parse_gimini_settings(settings)? else {
        info!("aicc google gimini provider is disabled (gimini settings missing or disabled)");
        return Ok(0);
    };
    if gimini_settings.api_token.trim().is_empty() {
        return Err(anyhow!(
            "gimini.api_token (or api_key) is required when gimini provider is enabled"
        ));
    }

    let instances = build_gimini_instances(&gimini_settings)?;
    let mut prepared = Vec::<(GoogleGiminiInstanceConfig, Arc<dyn Provider>)>::new();
    for config in instances.iter() {
        let provider =
            GoogleGiminiProvider::new(config.clone(), gimini_settings.api_token.clone())?;
        prepared.push((config.clone(), Arc::new(provider)));
    }

    for (config, provider) in prepared.into_iter() {
        let inventory = center.registry().add_provider(provider);
        info!(
            "registered google gimini base_url={} inventory={:?}",
            config.base_url, inventory
        );
        center
            .model_registry()
            .write()
            .map_err(|_| anyhow!("model registry lock poisoned"))?
            .apply_inventory(inventory)
            .map_err(|err| anyhow!("failed to apply gimini inventory: {}", err))?;
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aicc::ModelCatalog;
    use buckyos_api::{AiPayload, ModelSpec, Requirements};
    use serde_json::json;

    fn build_text2image_request(options: Option<Value>) -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Image,
            ModelSpec::new("text2image.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                Some("draw a banana".to_string()),
                vec![],
                vec![],
                vec![],
                None,
                options,
            ),
            None,
        )
    }

    #[test]
    fn build_gimini_instances_infers_image_models() {
        let settings = GiminiSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![SettingsGoogleGiminiInstanceConfig {
                provider_instance_name: "gimini-1".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "google-gimini".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
                models: vec![
                    "gemini-2.5-flash".to_string(),
                    "gemini-2.0-flash-exp-image-generation".to_string(),
                ],
                default_model: None,
                image_models: vec![],
                default_image_model: None,
                features: vec![],
                alias_map: HashMap::new(),
            }],
        };

        let instances = build_gimini_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(
            instances[0].default_model.as_deref(),
            Some("gemini-2.5-flash")
        );
        assert_eq!(
            instances[0].default_image_model.as_deref(),
            Some("gemini-2.0-flash-exp-image-generation")
        );
    }

    #[test]
    fn register_custom_aliases_routes_text2image_prefix() {
        let center = AIComputeCenter::new(Default::default(), ModelCatalog::default());
        let aliases = HashMap::from([
            (
                "llm.plan.default".to_string(),
                "gemini-2.5-flash".to_string(),
            ),
            (
                "text2image.nano_banana".to_string(),
                "gemini-2.0-flash-exp-image-generation".to_string(),
            ),
        ]);
        register_custom_aliases(&center, "google-gimini", &aliases);

        let llm = center.model_catalog().resolve(
            "",
            &Capability::Llm,
            "llm.plan.default",
            "google-gimini",
        );
        let image = center.model_catalog().resolve(
            "",
            &Capability::Image,
            "text2image.nano_banana",
            "google-gimini",
        );
        assert_eq!(llm.as_deref(), Some("gemini-2.5-flash"));
        assert_eq!(
            image.as_deref(),
            Some("gemini-2.0-flash-exp-image-generation")
        );
    }

    #[test]
    fn register_default_aliases_exposes_code_default_not_json_default() {
        let center = AIComputeCenter::new(Default::default(), ModelCatalog::default());
        let models = vec!["gemini-2.5-flash".to_string()];
        let image_models = Vec::<String>::new();
        register_default_aliases(
            &center,
            "google-gimini",
            &models,
            Some("gemini-2.5-flash"),
            &image_models,
            None,
        );

        let code_alias = center.model_catalog().resolve(
            "",
            &Capability::Llm,
            "llm.code.default",
            "google-gimini",
        );
        let removed_alias = center.model_catalog().resolve(
            "",
            &Capability::Llm,
            "llm.json.default",
            "google-gimini",
        );

        assert_eq!(code_alias.as_deref(), Some("gemini-2.5-flash"));
        assert!(removed_alias.is_none());
    }

    #[test]
    fn estimate_text2image_cost_covers_current_image_models() {
        let preview = build_text2image_request(Some(json!({ "n": 2 })));
        assert_eq!(
            GoogleGiminiProvider::estimate_text2image_cost(
                &preview,
                "gemini-2.5-flash-image-preview"
            ),
            Some(0.078)
        );

        let legacy = build_text2image_request(None);
        assert_eq!(
            GoogleGiminiProvider::estimate_text2image_cost(
                &legacy,
                "gemini-2.0-flash-exp-image-generation"
            ),
            Some(0.03)
        );
    }

    #[test]
    fn parse_text2image_result_supports_inline_data() {
        let body = json!({
            "candidates": [
                {
                    "content": {
                        "parts": [
                            {
                                "inlineData": {
                                    "mimeType": "image/png",
                                    "data": "aGVsbG8="
                                }
                            }
                        ]
                    }
                }
            ]
        });

        let (artifacts, text) =
            GoogleGiminiProvider::parse_text2image_result(&body).expect("artifacts should parse");
        assert_eq!(artifacts.len(), 1);
        assert!(text.is_none());
        match &artifacts[0].resource {
            ResourceRef::Base64 { data_base64, .. } => assert_eq!(data_base64, "aGVsbG8="),
            other => panic!("unexpected resource: {:?}", other),
        }
    }
}
