mod aicc;
mod aicc_usage_log_db;
mod claude;
mod claude_protocol;
mod complete_request_queue;
mod default_logical_tree;
mod fal;
mod gemini;
mod metadata_resolver;
mod metadata_updater;
mod minimax;
mod model_registry;
mod model_router;
mod model_scheduler;
mod model_session;
mod model_types;
mod openai;
mod openai_protocol;
mod sn_ai_provider;

use ::kRPC::*;
use anyhow::Result;
use buckyos_api::{
    ai_methods, get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime,
    AiccServerHandler, BuckyOSRuntimeType, DriverMetadataRuntimeApply, DriverMetadataUpdateGetReq,
    DriverMetadataUpdateSetReq, DriverMetadataUpdateSetResponse, DriverMetadataUpdateStatus,
    DriverMetadataUpdateView, QueryRouteTraceRequest, QueryRouteTraceResponse, QueryUsageRequest,
    QueryUsageResponse, SystemConfigClient, SystemConfigError, AICC_SERVICE_SERVICE_NAME,
};
use buckyos_http_server::Runner;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use buckyos_kit::{init_logging, KVAction};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::aicc::{AIComputeCenter, NamedStoreResourceResolver};
use crate::aicc_usage_log_db::AiccUsageLogDb;
use crate::claude::register_claude_providers;
use crate::fal::register_fal_providers;
use crate::gemini::register_google_gemini_providers;
use crate::metadata_updater::{
    active_remote_metadata_revision, configure_remote_metadata_source,
    normalize_update_interval_secs, DriverMetadataUpdateOutcome, DriverMetadataUpdateSettings,
    DriverMetadataUpdater,
};
use crate::minimax::register_minimax_providers;
use crate::openai::register_openai_llm_providers;
use crate::sn_ai_provider::register_sn_ai_provider;

const AICC_SERVICE_MAIN_PORT: u16 = 4040;
const METHOD_RELOAD_SETTINGS: &str = "reload_settings";
const METHOD_SERVICE_RELOAD_SETTINGS: &str = "service.reload_settings";
const METHOD_REALOAD_SETTINGS: &str = "reaload_settings";
const METHOD_SERVICE_REALOAD_SETTINGS: &str = "service.reaload_settings";
const METHOD_MODELS_LIST: &str = "models.list";
const METHOD_SERVICE_MODELS_LIST: &str = "service.models.list";
const METHOD_PROVIDER_VALIDATE: &str = "provider.validate";
const METHOD_PROVIDER_ADD: &str = "provider.add";
const METHOD_PROVIDER_DELETE: &str = "provider.delete";
const METHOD_PROVIDER_REFRESH_MODELS: &str = "provider.refresh_models";
const METHOD_USAGE_QUERY: &str = "usage.query";
const METHOD_TRACE_QUERY: &str = "trace.query";
const AICC_SETTINGS_KEY: &str = "services/aicc/settings";
const REDACTED_SECRET: &str = "***";
const PROVIDER_VALIDATION_CACHE_TTL: Duration = Duration::from_secs(300);
const PROVIDER_SECTIONS: &[&str] = &[
    "sn-ai-provider",
    "openai",
    "google",
    "gemini",
    "google_gemini",
    "claude",
    "anthropic",
    "minimax",
    "fal",
];

struct AiccHttpServer {
    rpc_handler: AiccServerHandler<AIComputeCenter>,
    provider_validation_cache: Mutex<HashMap<String, ProviderValidationCacheEntry>>,
    metadata_settings_tx: tokio::sync::watch::Sender<Option<Value>>,
    metadata_runtime_state: Mutex<DriverMetadataRuntimeState>,
}

#[derive(Clone, Debug)]
struct DriverMetadataRuntimeState {
    status: &'static str,
    active_revision: Option<u64>,
    last_attempt_at_ms: Option<u64>,
    last_success_at_ms: Option<u64>,
    last_error: Option<String>,
    consecutive_failures: u32,
}

impl DriverMetadataRuntimeState {
    fn for_settings(settings: Option<&Value>) -> Self {
        let enabled = settings
            .and_then(|value| value.get("driver_metadata_update"))
            .and_then(|value| value.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let active_revision = settings.and_then(active_remote_metadata_revision);
        Self {
            status: if !enabled {
                "disabled"
            } else if active_revision.is_some() {
                "healthy"
            } else {
                "idle"
            },
            active_revision,
            last_attempt_at_ms: None,
            last_success_at_ms: None,
            last_error: None,
            consecutive_failures: 0,
        }
    }
}

fn record_metadata_update_success(
    state: &mut DriverMetadataRuntimeState,
    revision_seq: u64,
    refresh_error_count: Option<usize>,
) {
    state.status = if refresh_error_count.is_some_and(|count| count > 0) {
        "degraded"
    } else {
        "healthy"
    };
    state.active_revision = Some(revision_seq);
    state.last_success_at_ms = Some(metadata_now_ms());
    state.last_error = refresh_error_count
        .filter(|count| *count > 0)
        .map(|count| format!("{} provider inventory refreshes failed", count));
    state.consecutive_failures = 0;
}

struct ProviderValidationCacheEntry {
    fingerprint: String,
    validated_at: Instant,
}

struct SettingsDocument {
    value: Value,
    version: u64,
    exists: bool,
}

fn apply_provider_settings(
    center: &AIComputeCenter,
    settings: &serde_json::Value,
) -> Result<usize> {
    if let Err(err) = configure_remote_metadata_source(settings) {
        warn!("invalid driver metadata update settings: {}", err);
    }
    center.registry().clear();
    center.reset_model_routes();

    let mut registered_total = 0usize;
    let mut errors = vec![];

    match register_openai_llm_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("openai: {}", err));
        }
    }

    match register_sn_ai_provider(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("sn-ai-provider: {}", err));
        }
    }

    match register_google_gemini_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("gemini: {}", err));
        }
    }

    match register_claude_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("claude: {}", err));
        }
    }

    match register_minimax_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("minimax: {}", err));
        }
    }

    match register_fal_providers(center, settings) {
        Ok(count) => {
            registered_total = registered_total.saturating_add(count);
        }
        Err(err) => {
            errors.push(format!("fal: {}", err));
        }
    }

    if !errors.is_empty() {
        warn!(
            "aicc provider registration has errors: registered_total={} errors={}",
            registered_total,
            errors.join(" | ")
        );
    }

    if registered_total == 0 && !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "all provider registrations failed: {}",
            errors.join(" | ")
        ));
    }

    match apply_logical_directory_settings(center, settings) {
        Ok(definition_count) => {
            info!(
                "aicc system routing applied: {} logical definitions",
                definition_count
            );
        }
        Err(err) => {
            warn!("aicc logical directory apply failed: {}", err);
        }
    }

    Ok(registered_total)
}

fn apply_logical_directory_settings(
    center: &AIComputeCenter,
    settings: &serde_json::Value,
) -> Result<usize> {
    center
        .apply_system_routing_config(settings)
        .map_err(|err| anyhow::anyhow!("apply system routing config failed: {}", err))
}

fn redact_settings_for_log(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut next = serde_json::Map::new();
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                if lower == "api_token" || lower == "api_key" || lower == "authorization" {
                    next.insert(
                        k.clone(),
                        serde_json::Value::String(REDACTED_SECRET.to_string()),
                    );
                } else if lower == "source_url" {
                    next.insert(k.clone(), redact_url_userinfo(v));
                } else {
                    next.insert(k.clone(), redact_settings_for_log(v));
                }
            }
            serde_json::Value::Object(next)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(redact_settings_for_log)
                .collect::<Vec<_>>(),
        ),
        _ => value.clone(),
    }
}

fn redact_url_userinfo(value: &Value) -> Value {
    let Some(raw) = value.as_str() else {
        return redact_settings_for_log(value);
    };
    let Ok(mut url) = reqwest::Url::parse(raw) else {
        return if raw.contains('@') {
            Value::String(REDACTED_SECRET.to_string())
        } else {
            value.clone()
        };
    };
    if url.username().is_empty() && url.password().is_none() {
        return value.clone();
    }
    let _ = url.set_username(REDACTED_SECRET);
    if url.password().is_some() {
        let _ = url.set_password(Some(REDACTED_SECRET));
    }
    Value::String(url.to_string())
}

fn metadata_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn redact_metadata_error(error: &str, settings: &Value) -> String {
    let mut redacted = error.to_string();
    if let Ok(userinfo_pattern) = Regex::new(r"(?i)https://[^/\s]+@") {
        redacted = userinfo_pattern
            .replace_all(redacted.as_str(), "https://***@")
            .into_owned();
    }
    if let Some(source_url) = settings
        .get("driver_metadata_update")
        .and_then(|value| value.get("source_url"))
        .and_then(Value::as_str)
    {
        let replacement = redact_url_userinfo(&Value::String(source_url.to_string()))
            .as_str()
            .unwrap_or(REDACTED_SECRET)
            .to_string();
        redacted = redacted.replace(source_url, replacement.as_str());
    }
    redacted.chars().take(512).collect()
}

fn driver_metadata_update_view(
    settings: &Value,
    runtime: &DriverMetadataRuntimeState,
) -> DriverMetadataUpdateView {
    let configured = settings
        .get("driver_metadata_update")
        .and_then(Value::as_object);
    let status = match runtime.status {
        "idle" => DriverMetadataUpdateStatus::Idle,
        "updating" => DriverMetadataUpdateStatus::Updating,
        "healthy" => DriverMetadataUpdateStatus::Healthy,
        "degraded" => DriverMetadataUpdateStatus::Degraded,
        "error" => DriverMetadataUpdateStatus::Error,
        _ => DriverMetadataUpdateStatus::Disabled,
    };
    DriverMetadataUpdateView {
        enabled: configured
            .and_then(|value| value.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        source_url: configured
            .and_then(|value| value.get("source_url"))
            .map(redact_url_userinfo)
            .and_then(|value| value.as_str().map(ToOwned::to_owned)),
        source_configured: configured
            .and_then(|value| value.get("source_url"))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        interval_secs: configured
            .and_then(|value| value.get("interval_secs"))
            .and_then(Value::as_u64)
            .map(normalize_update_interval_secs)
            .unwrap_or(3600),
        status,
        active_revision: runtime.active_revision,
        last_attempt_at_ms: runtime.last_attempt_at_ms,
        last_success_at_ms: runtime.last_success_at_ms,
        last_error: runtime.last_error.clone(),
        consecutive_failures: runtime.consecutive_failures,
    }
}

fn update_driver_metadata_settings(
    settings: &mut Value,
    request: &DriverMetadataUpdateSetReq,
) -> std::result::Result<(), RPCErrors> {
    let enabled = request.enabled;
    let root = settings_object(settings);
    let update = root
        .entry("driver_metadata_update".to_string())
        .or_insert_with(|| json!({}));
    if !update.is_object() {
        *update = json!({});
    }
    let update = update
        .as_object_mut()
        .expect("driver metadata update settings must be an object");
    update.insert("enabled".to_string(), Value::Bool(enabled));
    if let Some(source_url) = request
        .source_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
    {
        update.insert("source_url".to_string(), Value::String(source_url));
    }
    if let Some(interval_secs) = request.interval_secs {
        update.insert(
            "interval_secs".to_string(),
            json!(normalize_update_interval_secs(interval_secs)),
        );
    }
    if enabled {
        DriverMetadataUpdateSettings::from_aicc_settings(settings)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    }
    Ok(())
}

fn driver_metadata_update_set_result(
    settings_revision: u64,
    settings: DriverMetadataUpdateView,
    runtime_apply: DriverMetadataRuntimeApply,
) -> DriverMetadataUpdateSetResponse {
    DriverMetadataUpdateSetResponse {
        ok: true,
        settings_revision,
        settings,
        runtime_apply,
    }
}

fn rpc_success(req: &RPCRequest, result: Value) -> RPCResponse {
    RPCResponse {
        result: RPCResult::Success(result),
        seq: req.seq,
        trace_id: req.trace_id.clone(),
    }
}

fn system_config_error_to_rpc(err: SystemConfigError) -> RPCErrors {
    match err {
        SystemConfigError::KeyNotFound(key) => {
            RPCErrors::ReasonError(format!("key_not_found: {}", key))
        }
        SystemConfigError::NoPermission(reason) => {
            RPCErrors::ReasonError(format!("no_permission: {}", reason))
        }
        SystemConfigError::Timeout(reason) => {
            RPCErrors::ReasonError(format!("timeout: {}", reason))
        }
        SystemConfigError::ReasonError(reason) => {
            let lower = reason.to_ascii_lowercase();
            if lower.contains("revision")
                || lower.contains("version")
                || lower.contains("conflict")
                || lower.contains("main_key")
            {
                RPCErrors::ReasonError("settings_conflict".to_string())
            } else {
                RPCErrors::ReasonError(reason)
            }
        }
    }
}

fn param_string(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn typed_request_params(params: &Value) -> Value {
    let mut params = params.clone();
    if let Some(params) = params.as_object_mut() {
        params.remove("session_token");
    }
    params
}

fn param_bool(params: &Value, key: &str) -> Option<bool> {
    params.get(key).and_then(Value::as_bool)
}

fn validate_provider_instance_name(value: &str) -> std::result::Result<(), RPCErrors> {
    if value.trim().is_empty() {
        return Err(RPCErrors::ReasonError(
            "provider_instance_name is required".to_string(),
        ));
    }
    let valid = value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    if !valid {
        return Err(RPCErrors::ReasonError(
            "provider_instance_name contains invalid characters".to_string(),
        ));
    }
    Ok(())
}

fn section_for_provider_type(provider_type: &str) -> std::result::Result<&'static str, RPCErrors> {
    match provider_type {
        "sn_router" => Ok("sn-ai-provider"),
        "openai" | "openrouter" | "custom" => Ok("openai"),
        "anthropic" => Ok("claude"),
        "google" => Ok("google"),
        "minimax" => Ok("minimax"),
        "fal" => Ok("fal"),
        _ => Err(RPCErrors::ReasonError(format!(
            "unsupported provider_type: {}",
            provider_type
        ))),
    }
}

fn default_endpoint(provider_type: &str) -> &'static str {
    match provider_type {
        "sn_router" => "https://sn.buckyos.ai/api/v1/ai/",
        "openai" | "custom" => "https://api.openai.com/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "anthropic" => "https://api.anthropic.com/v1",
        "google" => "https://generativelanguage.googleapis.com/v1beta",
        "minimax" => "https://api.minimax.io/v1",
        "fal" => "https://fal.run",
        _ => "",
    }
}

fn provider_driver_for_request(provider_type: &str) -> String {
    match provider_type {
        "sn_router" => "sn-ai-provider".to_string(),
        "openrouter" => "openrouter".to_string(),
        "anthropic" => "claude".to_string(),
        "google" => "google-gemini".to_string(),
        "minimax" => "minimax".to_string(),
        "fal" => "fal".to_string(),
        "custom" => "openai".to_string(),
        _ => provider_type.to_string(),
    }
}

fn settings_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("settings must be object")
}

fn ensure_section<'a>(settings: &'a mut Value, section: &str) -> &'a mut Map<String, Value> {
    let root = settings_object(settings);
    let entry = root.entry(section.to_string()).or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    entry.as_object_mut().expect("section must be object")
}

fn collect_provider_instance_names(settings: &Value) -> HashSet<String> {
    let mut names = HashSet::new();
    let Some(root) = settings.as_object() else {
        return names;
    };
    for section in PROVIDER_SECTIONS {
        let Some(instances) = root
            .get(*section)
            .and_then(Value::as_object)
            .and_then(|section| section.get("instances"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for instance in instances {
            if let Some(name) = instance
                .get("provider_instance_name")
                .or_else(|| instance.get("instance_id"))
                .and_then(Value::as_str)
            {
                names.insert(name.to_string());
            }
        }
    }
    names
}

fn collect_provider_display_names(settings: &Value) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let Some(root) = settings.as_object() else {
        return names;
    };
    for section in PROVIDER_SECTIONS {
        let Some(instances) = root
            .get(*section)
            .and_then(Value::as_object)
            .and_then(|section| section.get("instances"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for instance in instances {
            let Some(instance_name) = instance
                .get("provider_instance_name")
                .or_else(|| instance.get("instance_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(display_name) = instance
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            names.insert(instance_name.to_string(), display_name.to_string());
        }
    }
    names
}

fn enrich_provider_display_names(directory: &mut Value, settings: &Value) {
    let display_names = collect_provider_display_names(settings);
    if display_names.is_empty() {
        return;
    }
    let Some(providers) = directory.get_mut("providers").and_then(Value::as_array_mut) else {
        return;
    };
    for provider in providers {
        let Some(provider_obj) = provider.as_object_mut() else {
            continue;
        };
        let Some(provider_instance_name) = provider_obj
            .get("provider_instance_name")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        if let Some(display_name) = display_names.get(provider_instance_name.as_str()) {
            provider_obj.insert("name".to_string(), Value::String(display_name.clone()));
        }
    }
}

fn build_provider_instance_settings(params: &Value) -> std::result::Result<Value, RPCErrors> {
    let provider_instance_name = param_string(params, "provider_instance_name")
        .ok_or_else(|| RPCErrors::ReasonError("provider_instance_name is required".to_string()))?;
    validate_provider_instance_name(provider_instance_name.as_str())?;
    let provider_type = param_string(params, "provider_type")
        .ok_or_else(|| RPCErrors::ReasonError("provider_type is required".to_string()))?;
    let section = section_for_provider_type(provider_type.as_str())?;
    let endpoint = param_string(params, "endpoint")
        .unwrap_or_else(|| default_endpoint(provider_type.as_str()).to_string());
    if provider_type == "custom" && endpoint.trim().is_empty() {
        return Err(RPCErrors::ReasonError(
            "endpoint is required for custom provider".to_string(),
        ));
    }

    let api_key = param_string(params, "api_key").unwrap_or_default();
    if provider_type != "sn_router" && api_key.trim().is_empty() {
        return Err(RPCErrors::ReasonError("api_key is required".to_string()));
    }

    let mut instance = Map::new();
    instance.insert(
        "provider_instance_name".to_string(),
        Value::String(provider_instance_name),
    );
    instance.insert(
        "provider_type".to_string(),
        Value::String("cloud_api".to_string()),
    );
    instance.insert(
        "provider_driver".to_string(),
        Value::String(provider_driver_for_request(provider_type.as_str())),
    );
    if !api_key.trim().is_empty() {
        instance.insert("api_token".to_string(), Value::String(api_key));
    }
    if !endpoint.trim().is_empty() {
        instance.insert("base_url".to_string(), Value::String(endpoint));
    }
    instance.insert(
        "auth_mode".to_string(),
        Value::String(
            if provider_type == "sn_router" {
                "runtime_session"
            } else {
                "bearer"
            }
            .to_string(),
        ),
    );
    instance.insert("timeout_ms".to_string(), json!(300_000u64));
    if let Some(name) = param_string(params, "name") {
        instance.insert("name".to_string(), Value::String(name));
    }
    if let Some(protocol_type) = param_string(params, "protocol_type") {
        instance.insert("protocol_type".to_string(), Value::String(protocol_type));
    }
    if let Some(auto_sync) = param_bool(params, "auto_sync_models") {
        instance.insert("auto_sync_models".to_string(), Value::Bool(auto_sync));
    }

    let mut wrapped = Map::new();
    wrapped.insert("section".to_string(), Value::String(section.to_string()));
    wrapped.insert("instance".to_string(), Value::Object(instance));
    Ok(Value::Object(wrapped))
}

fn normalized_endpoint_for_validation(provider_type: &str, params: &Value) -> String {
    param_string(params, "endpoint")
        .unwrap_or_else(|| default_endpoint(provider_type).to_string())
        .trim_end_matches('/')
        .to_string()
}

fn provider_validation_cache_key(params: &Value) -> String {
    let provider_type = param_string(params, "provider_type").unwrap_or_else(|| "custom".into());
    let endpoint = normalized_endpoint_for_validation(provider_type.as_str(), params);
    let protocol_type = param_string(params, "protocol_type");
    let api_key = param_string(params, "api_key").unwrap_or_default();
    serde_json::to_string(&json!({
        "provider_type": provider_type,
        "endpoint": endpoint,
        "protocol_type": protocol_type,
        "api_key": api_key,
    }))
    .expect("provider validation cache key must serialize")
}

fn provider_validation_fingerprint(cache_key: &str) -> String {
    let mut hasher = DefaultHasher::new();
    cache_key.hash(&mut hasher);
    format!("provider-validation-{:x}", hasher.finish())
}

fn validation_issue(kind: &str, message: impl Into<String>) -> Value {
    json!({
        "kind": kind,
        "message": message.into()
    })
}

fn validation_issue_message(issue: &Value) -> String {
    issue
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("provider validation failed")
        .to_string()
}

fn validation_issue_kind(issue: &Value) -> Option<&str> {
    issue.get("kind").and_then(Value::as_str)
}

fn truncate_error_body(body: &str) -> String {
    const LIMIT: usize = 500;
    let clean = body.replace(['\r', '\n'], " ");
    if clean.chars().count() <= LIMIT {
        clean
    } else {
        format!("{}...", clean.chars().take(LIMIT).collect::<String>())
    }
}

fn discovery_http_issue(provider: &str, status: reqwest::StatusCode, body: String) -> Value {
    let kind = if status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
    {
        "auth"
    } else if status == reqwest::StatusCode::NOT_FOUND || status.is_server_error() {
        "endpoint"
    } else {
        "models"
    };
    validation_issue(
        kind,
        format!(
            "{} model discovery failed: status={} body={}",
            provider,
            status.as_u16(),
            truncate_error_body(body.as_str())
        ),
    )
}

fn request_error_issue(provider: &str, err: reqwest::Error) -> Value {
    let kind = if err.is_timeout() || err.is_connect() || err.is_request() {
        "endpoint"
    } else {
        "models"
    };
    validation_issue(
        kind,
        format!("{} model discovery request failed: {}", provider, err),
    )
}

async fn send_discovery_request(
    provider: &str,
    request: reqwest::RequestBuilder,
) -> std::result::Result<Value, Value> {
    let response = request
        .send()
        .await
        .map_err(|err| request_error_issue(provider, err))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(discovery_http_issue(provider, status, body));
    }
    response.json::<Value>().await.map_err(|err| {
        validation_issue(
            "models",
            format!(
                "{} model discovery response is not valid JSON: {}",
                provider, err
            ),
        )
    })
}

fn collect_model_id_from_entry(entry: &Value) -> Option<String> {
    let raw = if let Some(value) = entry.as_str() {
        Some(value)
    } else {
        entry
            .get("id")
            .or_else(|| entry.get("name"))
            .or_else(|| entry.get("provider_model_id"))
            .or_else(|| entry.get("provider_actual_model_id"))
            .and_then(Value::as_str)
    }?;
    let id = raw
        .trim()
        .strip_prefix("models/")
        .unwrap_or(raw.trim())
        .trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

fn collect_model_ids(body: &Value) -> Vec<String> {
    let mut ids = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    for key in ["data", "models"] {
        let Some(items) = body.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(id) = collect_model_id_from_entry(item) else {
                continue;
            };
            if seen.insert(id.clone()) {
                ids.push(id);
            }
        }
    }
    ids
}

fn openai_models_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("/chat/completions") {
        let prefix = &trimmed[..trimmed.len() - "/chat/completions".len()];
        return format!("{}/models", prefix.trim_end_matches('/'));
    }
    if lower.ends_with("/responses") || lower.ends_with("/images/generations") {
        if let Some((prefix, _)) = trimmed.rsplit_once('/') {
            return format!("{}/models", prefix.trim_end_matches('/'));
        }
    }
    format!("{}/models", trimmed)
}

async fn discover_openai_compatible_models(
    client: &reqwest::Client,
    provider: &str,
    endpoint: &str,
    api_key: &str,
) -> std::result::Result<Vec<String>, Value> {
    let body = send_discovery_request(
        provider,
        client
            .get(openai_models_endpoint(endpoint).as_str())
            .bearer_auth(api_key),
    )
    .await?;
    let ids = collect_model_ids(&body);
    if ids.is_empty() {
        Err(validation_issue(
            "models",
            format!("{} model discovery returned no models", provider),
        ))
    } else {
        Ok(ids)
    }
}

async fn discover_anthropic_models(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
) -> std::result::Result<Vec<String>, Value> {
    let url = format!("{}/models?limit=1000", endpoint.trim_end_matches('/'));
    let body = send_discovery_request(
        "anthropic",
        client
            .get(url.as_str())
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
    )
    .await?;
    let ids = collect_model_ids(&body);
    if ids.is_empty() {
        Err(validation_issue(
            "models",
            "anthropic model discovery returned no models",
        ))
    } else {
        Ok(ids)
    }
}

async fn discover_google_models(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
) -> std::result::Result<Vec<String>, Value> {
    let url = format!("{}/models", endpoint.trim_end_matches('/'));
    let body = send_discovery_request(
        "google",
        client
            .get(url.as_str())
            .query(&[("key", api_key), ("pageSize", "1000")]),
    )
    .await?;
    let ids = collect_model_ids(&body);
    if ids.is_empty() {
        Err(validation_issue(
            "models",
            "google model discovery returned no models",
        ))
    } else {
        Ok(ids)
    }
}

impl AiccHttpServer {
    fn new(center: AIComputeCenter, initial_settings: Option<Value>) -> Self {
        let metadata_runtime_state =
            DriverMetadataRuntimeState::for_settings(initial_settings.as_ref());
        let (metadata_settings_tx, _) = tokio::sync::watch::channel(initial_settings);
        Self {
            rpc_handler: AiccServerHandler::new(center),
            provider_validation_cache: Mutex::new(HashMap::new()),
            metadata_settings_tx,
            metadata_runtime_state: Mutex::new(metadata_runtime_state),
        }
    }

    fn metadata_runtime_state(&self) -> DriverMetadataRuntimeState {
        self.metadata_runtime_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| DriverMetadataRuntimeState::for_settings(None))
    }

    fn update_metadata_runtime_state(&self, update: impl FnOnce(&mut DriverMetadataRuntimeState)) {
        if let Ok(mut state) = self.metadata_runtime_state.lock() {
            update(&mut state);
        }
    }

    fn metadata_update_view(&self, settings: &Value) -> DriverMetadataUpdateView {
        driver_metadata_update_view(settings, &self.metadata_runtime_state())
    }

    async fn handle_models_list(
        &self,
        params: &Value,
    ) -> std::result::Result<serde_json::Value, RPCErrors> {
        let mut directory = self.rpc_handler.0.dump_model_directory()?;
        match get_buckyos_api_runtime() {
            Ok(runtime) => match runtime.get_my_settings().await {
                Ok(settings) => enrich_provider_display_names(&mut directory, &settings),
                Err(err) => warn!("load aicc settings failed while listing models: {}", err),
            },
            Err(err) => warn!("get runtime failed while listing models: {}", err),
        }
        let logical_path = params
            .get("logical_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        Ok(match logical_path {
            Some(path) => filter_model_directory_by_path(directory, path),
            None => directory,
        })
    }

    fn apply_settings_value(&self, settings: &Value) -> std::result::Result<Value, RPCErrors> {
        let settings_for_log = redact_settings_for_log(&settings);
        info!(
            "aicc.reload_settings current settings: {}",
            settings_for_log
        );

        let registered =
            apply_provider_settings(&self.rpc_handler.0, &settings).map_err(|err| {
                RPCErrors::ReasonError(format!("reload aicc settings failed: {}", err))
            })?;
        self.metadata_settings_tx
            .send_replace(Some(settings.clone()));
        Ok(serde_json::json!({
            "ok": true,
            "providers_registered": registered
        }))
    }

    async fn handle_reload_settings(&self) -> std::result::Result<serde_json::Value, RPCErrors> {
        let runtime = get_buckyos_api_runtime()
            .map_err(|err| RPCErrors::ReasonError(format!("get runtime failed: {}", err)))?;
        let settings = runtime.get_my_settings().await.map_err(|err| {
            RPCErrors::ReasonError(format!("load aicc settings during reload failed: {}", err))
        })?;
        self.apply_settings_value(&settings)
    }

    async fn load_settings_for_request(
        &self,
        req: &RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<(Arc<SystemConfigClient>, SettingsDocument), RPCErrors> {
        let runtime = get_buckyos_api_runtime()
            .map_err(|err| RPCErrors::ReasonError(format!("get runtime failed: {}", err)))?;
        let client = runtime
            .get_system_config_client()
            .await
            .map_err(|err| RPCErrors::ReasonError(format!("get system_config failed: {}", err)))?;
        client
            .set_context(RPCContext::from_request(req, ip_from))
            .await
            .map_err(system_config_error_to_rpc)?;
        let current = client.get(AICC_SETTINGS_KEY).await;
        let doc = match current {
            Ok(value) => SettingsDocument {
                value: serde_json::from_str(value.value.as_str()).map_err(|err| {
                    RPCErrors::ReasonError(format!("parse aicc settings failed: {}", err))
                })?,
                version: value.version,
                exists: true,
            },
            Err(SystemConfigError::KeyNotFound(_)) => SettingsDocument {
                value: json!({}),
                version: 0,
                exists: false,
            },
            Err(err) => return Err(system_config_error_to_rpc(err)),
        };
        Ok((client, doc))
    }

    async fn write_settings(
        &self,
        client: Arc<SystemConfigClient>,
        doc: &SettingsDocument,
        next: &Value,
    ) -> std::result::Result<u64, RPCErrors> {
        let settings_json = serde_json::to_string_pretty(next).map_err(|err| {
            RPCErrors::ReasonError(format!("serialize aicc settings failed: {}", err))
        })?;
        let mut tx = HashMap::new();
        tx.insert(
            AICC_SETTINGS_KEY.to_string(),
            if doc.exists {
                KVAction::Update(settings_json)
            } else {
                KVAction::Create(settings_json)
            },
        );
        let main_key = if doc.exists {
            Some((AICC_SETTINGS_KEY.to_string(), doc.version))
        } else {
            None
        };
        client
            .exec_tx(tx, main_key)
            .await
            .map_err(system_config_error_to_rpc)?;
        let updated = client
            .get(AICC_SETTINGS_KEY)
            .await
            .map_err(system_config_error_to_rpc)?;
        Ok(updated.version)
    }

    async fn reload_result_value(&self) -> Value {
        match self.handle_reload_settings().await {
            Ok(value) => value,
            Err(err) => json!({
                "ok": false,
                "error": err.to_string()
            }),
        }
    }

    fn remember_provider_validation(&self, cache_key: String, fingerprint: String) {
        match self.provider_validation_cache.lock() {
            Ok(mut cache) => {
                cache.retain(|_, entry| {
                    entry.validated_at.elapsed() <= PROVIDER_VALIDATION_CACHE_TTL
                });
                cache.insert(
                    cache_key,
                    ProviderValidationCacheEntry {
                        fingerprint,
                        validated_at: Instant::now(),
                    },
                );
            }
            Err(err) => {
                warn!("provider validation cache lock poisoned: {}", err);
            }
        }
    }

    fn require_recent_provider_validation(
        &self,
        params: &Value,
    ) -> std::result::Result<String, RPCErrors> {
        let cache_key = provider_validation_cache_key(params);
        let expected_fingerprint = provider_validation_fingerprint(cache_key.as_str());
        let mut cache = self.provider_validation_cache.lock().map_err(|err| {
            RPCErrors::ReasonError(format!("provider validation cache unavailable: {}", err))
        })?;
        cache.retain(|_, entry| entry.validated_at.elapsed() <= PROVIDER_VALIDATION_CACHE_TTL);
        let Some(entry) = cache.get(cache_key.as_str()) else {
            return Err(RPCErrors::ReasonError(
                "provider_validation_required".to_string(),
            ));
        };
        if entry.fingerprint != expected_fingerprint {
            return Err(RPCErrors::ReasonError(
                "provider_validation_mismatch".to_string(),
            ));
        }
        Ok(expected_fingerprint)
    }

    async fn handle_provider_validate(
        &self,
        params: &Value,
    ) -> std::result::Result<Value, RPCErrors> {
        let validation_cache_key = provider_validation_cache_key(params);
        let validation_fingerprint = provider_validation_fingerprint(validation_cache_key.as_str());
        let mut issues = Vec::<Value>::new();
        let provider_type =
            param_string(params, "provider_type").unwrap_or_else(|| "custom".into());

        if let Some(name) = param_string(params, "provider_instance_name") {
            if let Err(err) = validate_provider_instance_name(name.as_str()) {
                issues.push(validation_issue("models", err.to_string()));
            }
        }

        let endpoint = param_string(params, "endpoint")
            .unwrap_or_else(|| default_endpoint(provider_type.as_str()).to_string());
        if provider_type == "custom" && endpoint.is_empty() {
            issues.push(validation_issue(
                "endpoint",
                "endpoint is required for custom provider",
            ));
        }
        if !endpoint.is_empty() && reqwest::Url::parse(endpoint.as_str()).is_err() {
            issues.push(validation_issue("endpoint", "endpoint is not a valid URL"));
        }

        let api_key = param_string(params, "api_key").unwrap_or_default();
        if provider_type != "sn_router" && api_key.is_empty() {
            issues.push(validation_issue("auth", "api_key is required"));
        }

        let protocol_type = param_string(params, "protocol_type");
        if provider_type == "custom" && protocol_type.is_none() {
            issues.push(validation_issue(
                "models",
                "protocol_type is required for custom provider",
            ));
        }

        let mut models_discovered = Vec::<String>::new();
        if issues.is_empty() && provider_type != "sn_router" {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("build discovery client failed: {}", err))
                })?;
            let discovery = match provider_type.as_str() {
                "openai" => {
                    discover_openai_compatible_models(
                        &client,
                        "openai",
                        endpoint.as_str(),
                        api_key.as_str(),
                    )
                    .await
                }
                "openrouter" => {
                    discover_openai_compatible_models(
                        &client,
                        "openrouter",
                        endpoint.as_str(),
                        api_key.as_str(),
                    )
                    .await
                }
                "anthropic" => {
                    discover_anthropic_models(&client, endpoint.as_str(), api_key.as_str()).await
                }
                "google" => {
                    discover_google_models(&client, endpoint.as_str(), api_key.as_str()).await
                }
                "custom" => match protocol_type.as_deref() {
                    Some("openai_compatible") => {
                        discover_openai_compatible_models(
                            &client,
                            "custom openai-compatible",
                            endpoint.as_str(),
                            api_key.as_str(),
                        )
                        .await
                    }
                    Some("anthropic_compatible") => {
                        discover_anthropic_models(&client, endpoint.as_str(), api_key.as_str())
                            .await
                    }
                    Some("google_compatible") => {
                        discover_google_models(&client, endpoint.as_str(), api_key.as_str()).await
                    }
                    Some(other) => Err(validation_issue(
                        "models",
                        format!("unsupported custom protocol_type: {}", other),
                    )),
                    None => Err(validation_issue(
                        "models",
                        "protocol_type is required for custom provider",
                    )),
                },
                other => Err(validation_issue(
                    "models",
                    format!("unsupported provider_type: {}", other),
                )),
            };

            match discovery {
                Ok(models) => {
                    models_discovered = models;
                }
                Err(issue) => {
                    issues.push(issue);
                }
            }
        }

        let endpoint_reachable = !issues
            .iter()
            .any(|issue| validation_issue_kind(issue) == Some("endpoint"));
        let auth_valid = !issues
            .iter()
            .any(|issue| validation_issue_kind(issue) == Some("auth"));
        let errors = issues
            .iter()
            .map(validation_issue_message)
            .collect::<Vec<_>>();

        if issues.is_empty() {
            self.remember_provider_validation(validation_cache_key, validation_fingerprint.clone());
        }

        Ok(json!({
            "endpoint_reachable": endpoint_reachable,
            "auth_valid": auth_valid,
            "models_discovered": models_discovered,
            "balance_available": provider_type != "custom" && auth_valid,
            "errors": errors,
            "error_details": issues,
            "validation_fingerprint": validation_fingerprint,
            "validation_ttl_ms": PROVIDER_VALIDATION_CACHE_TTL.as_millis() as u64,
        }))
    }

    async fn handle_provider_add(
        &self,
        req: &RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<Value, RPCErrors> {
        let (client, mut doc) = self.load_settings_for_request(req, ip_from).await?;
        let built = build_provider_instance_settings(&req.params)?;
        let validation_fingerprint = self.require_recent_provider_validation(&req.params)?;
        let section = built
            .get("section")
            .and_then(Value::as_str)
            .ok_or_else(|| RPCErrors::ReasonError("missing provider section".to_string()))?;
        let instance = built
            .get("instance")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("missing provider instance".to_string()))?;
        let provider_instance_name = instance
            .get("provider_instance_name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RPCErrors::ReasonError("provider_instance_name is required".to_string())
            })?
            .to_string();
        if collect_provider_instance_names(&doc.value).contains(provider_instance_name.as_str()) {
            return Err(RPCErrors::ReasonError(
                "provider already exists".to_string(),
            ));
        }

        let section_obj = ensure_section(&mut doc.value, section);
        section_obj.insert("enabled".to_string(), Value::Bool(true));
        let instances = section_obj
            .entry("instances".to_string())
            .or_insert_with(|| Value::Array(vec![]));
        if !instances.is_array() {
            *instances = Value::Array(vec![]);
        }
        instances
            .as_array_mut()
            .expect("instances must be array")
            .push(Value::Object(instance));

        let settings_revision = self.write_settings(client, &doc, &doc.value).await?;
        let reload = self.reload_result_value().await;
        Ok(json!({
            "ok": true,
            "provider_instance_name": provider_instance_name,
            "settings_revision": settings_revision,
            "validation_fingerprint": validation_fingerprint,
            "reload": reload
        }))
    }

    async fn handle_provider_delete(
        &self,
        req: &RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<Value, RPCErrors> {
        let provider_instance_name = param_string(&req.params, "provider_instance_name")
            .ok_or_else(|| {
                RPCErrors::ReasonError("provider_instance_name is required".to_string())
            })?;
        let (client, mut doc) = self.load_settings_for_request(req, ip_from).await?;
        let mut removed = false;
        if let Some(root) = doc.value.as_object_mut() {
            for section_name in PROVIDER_SECTIONS {
                let Some(section) = root.get_mut(*section_name).and_then(Value::as_object_mut)
                else {
                    continue;
                };
                let Some(instances) = section.get_mut("instances").and_then(Value::as_array_mut)
                else {
                    continue;
                };
                let before = instances.len();
                instances.retain(|item| {
                    item.get("provider_instance_name")
                        .or_else(|| item.get("instance_id"))
                        .and_then(Value::as_str)
                        != Some(provider_instance_name.as_str())
                });
                if instances.len() != before {
                    removed = true;
                    if instances.is_empty() {
                        section.insert("enabled".to_string(), Value::Bool(false));
                    }
                }
            }
        }

        if !removed {
            return Ok(json!({
                "ok": false,
                "reason": "provider_not_found"
            }));
        }

        let settings_revision = self.write_settings(client, &doc, &doc.value).await?;
        let reload = self.reload_result_value().await;
        Ok(json!({
            "ok": true,
            "provider_instance_name": provider_instance_name,
            "settings_revision": settings_revision,
            "reload": reload
        }))
    }

    async fn handle_provider_refresh_models(
        &self,
        params: &Value,
    ) -> std::result::Result<Value, RPCErrors> {
        let provider_instance_name =
            param_string(params, "provider_instance_name").ok_or_else(|| {
                RPCErrors::ReasonError("provider_instance_name is required".to_string())
            })?;
        let inventory = self
            .rpc_handler
            .0
            .refresh_provider_inventory(provider_instance_name.as_str())
            .await?;
        Ok(json!({
            "ok": true,
            "provider_instance_name": provider_instance_name,
            "inventory_revision": inventory.inventory_revision
        }))
    }

    async fn handle_driver_metadata_update_get(
        &self,
        req: &RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<DriverMetadataUpdateView, RPCErrors> {
        DriverMetadataUpdateGetReq::from_json(typed_request_params(&req.params))?;
        let (_, doc) = self.load_settings_for_request(req, ip_from).await?;
        Ok(self.metadata_update_view(&doc.value))
    }

    fn apply_driver_metadata_settings_runtime(
        &self,
        settings: &Value,
    ) -> DriverMetadataRuntimeApply {
        if let Err(err) = configure_remote_metadata_source(settings) {
            let message = redact_metadata_error(err.to_string().as_str(), settings);
            self.update_metadata_runtime_state(|state| {
                state.status = "error";
                state.last_attempt_at_ms = Some(metadata_now_ms());
                state.last_error = Some(message.clone());
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
            });
            self.metadata_settings_tx
                .send_replace(Some(settings.clone()));
            return DriverMetadataRuntimeApply {
                ok: false,
                refresh_scheduled: None,
                error: Some(message),
            };
        }

        let enabled = DriverMetadataUpdateSettings::from_aicc_settings(settings)
            .ok()
            .flatten()
            .is_some();
        let active_revision = enabled
            .then(|| active_remote_metadata_revision(settings))
            .flatten();
        self.update_metadata_runtime_state(|state| {
            state.status = if enabled { "updating" } else { "disabled" };
            state.active_revision = active_revision;
            state.last_attempt_at_ms = Some(metadata_now_ms());
            state.last_error = None;
            state.consecutive_failures = 0;
        });

        self.metadata_settings_tx
            .send_replace(Some(settings.clone()));
        DriverMetadataRuntimeApply {
            ok: true,
            refresh_scheduled: Some(true),
            error: None,
        }
    }

    async fn handle_driver_metadata_update_set(
        &self,
        req: &RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        let update_request =
            DriverMetadataUpdateSetReq::from_json(typed_request_params(&req.params))?;
        let (client, mut doc) = self.load_settings_for_request(req, ip_from).await?;
        update_driver_metadata_settings(&mut doc.value, &update_request)?;
        let settings_revision = self.write_settings(client, &doc, &doc.value).await?;
        let runtime_apply = self.apply_driver_metadata_settings_runtime(&doc.value);
        Ok(driver_metadata_update_set_result(
            settings_revision,
            self.metadata_update_view(&doc.value),
            runtime_apply,
        ))
    }

    async fn handle_usage_query(&self, params: &Value) -> std::result::Result<Value, RPCErrors> {
        let query: QueryUsageRequest = serde_json::from_value(params.clone()).map_err(|err| {
            RPCErrors::ReasonError(format!("invalid usage.query request: {}", err))
        })?;
        let response = match self.rpc_handler.0.usage_log_db() {
            Some(db) => db.query_usage(&query).await?,
            None => QueryUsageResponse::default(),
        };
        serde_json::to_value(response)
            .map_err(|err| RPCErrors::ReasonError(format!("serialize usage response: {}", err)))
    }

    async fn handle_trace_query(&self, params: &Value) -> std::result::Result<Value, RPCErrors> {
        let query: QueryRouteTraceRequest =
            serde_json::from_value(params.clone()).map_err(|err| {
                RPCErrors::ReasonError(format!("invalid trace.query request: {}", err))
            })?;
        let response = match self.rpc_handler.0.usage_log_db() {
            Some(db) => db.query_route_traces(&query).await?,
            None => QueryRouteTraceResponse::default(),
        };
        serde_json::to_value(response)
            .map_err(|err| RPCErrors::ReasonError(format!("serialize trace response: {}", err)))
    }
}

fn start_driver_metadata_update_loop(server: Arc<AiccHttpServer>) {
    let mut settings_rx = server.metadata_settings_tx.subscribe();
    let spawn_result = std::thread::Builder::new()
        .name("aicc-metadata-updater".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    error!("create metadata updater runtime failed: {}", err);
                    return;
                }
            };
            runtime.block_on(async move {
                let mut consecutive_failures = 0u32;
                let mut refresh_after_settings_change = false;
                loop {
                    let Some(settings_value) = settings_rx.borrow().clone() else {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        server.update_metadata_runtime_state(|state| {
                            state.status = "error";
                            state.last_attempt_at_ms = Some(metadata_now_ms());
                            state.last_error = Some("aicc settings unavailable".to_string());
                            state.consecutive_failures = consecutive_failures;
                        });
                        let delay = metadata_retry_delay(consecutive_failures, 3600);
                        match get_buckyos_api_runtime() {
                            Ok(runtime) => match runtime.get_my_settings().await {
                                Ok(settings) => {
                                    match apply_provider_settings(&server.rpc_handler.0, &settings) {
                                        Ok(registered) => {
                                            consecutive_failures = 0;
                                            server
                                                .metadata_settings_tx
                                                .send_replace(Some(settings));
                                            info!(
                                                "aicc settings recovery registered {} providers",
                                                registered
                                            );
                                            continue;
                                        }
                                        Err(err) => warn!(
                                            "apply recovered aicc settings failed retry_secs={}: {}",
                                            delay, err
                                        ),
                                    }
                                }
                                Err(err) => warn!(
                                    "load unavailable driver metadata settings failed retry_secs={}: {}",
                                    delay, err
                                ),
                            },
                            Err(err) => warn!(
                                "get runtime for unavailable driver metadata settings failed retry_secs={}: {}",
                                delay, err
                            ),
                        }
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
                            changed = settings_rx.changed() => {
                                if changed.is_ok() {
                                    refresh_after_settings_change = true;
                                }
                            }
                        }
                        continue;
                    };
                    if let Err(err) = configure_remote_metadata_source(&settings_value) {
                        warn!("invalid driver metadata update settings: {}", err);
                    }
                    let update_settings = match DriverMetadataUpdateSettings::from_aicc_settings(
                        &settings_value,
                    ) {
                        Ok(Some(settings)) => settings,
                        Ok(None) => {
                            consecutive_failures = 0;
                            let refresh_error_count = if refresh_after_settings_change {
                                server.update_metadata_runtime_state(|state| {
                                    state.status = "updating";
                                    state.last_attempt_at_ms = Some(metadata_now_ms());
                                });
                                let count = refresh_provider_inventories_for_metadata(
                                    server.as_ref(),
                                    "settings-disabled",
                                )
                                .await;
                                refresh_after_settings_change = false;
                                count
                            } else {
                                0
                            };
                            server.update_metadata_runtime_state(|state| {
                                state.status = if refresh_error_count == 0 {
                                    "disabled"
                                } else {
                                    "degraded"
                                };
                                state.active_revision = None;
                                state.last_error = (refresh_error_count > 0).then(|| {
                                    format!(
                                        "{} provider inventory refreshes failed",
                                        refresh_error_count
                                    )
                                });
                                state.consecutive_failures = 0;
                            });
                            if settings_rx.changed().await.is_ok() {
                                refresh_after_settings_change = true;
                            }
                            continue;
                        }
                        Err(err) => {
                            warn!("invalid driver metadata update settings: {}", err);
                            let message =
                                redact_metadata_error(err.to_string().as_str(), &settings_value);
                            server.update_metadata_runtime_state(|state| {
                                state.status = "error";
                                state.last_attempt_at_ms = Some(metadata_now_ms());
                                state.last_error = Some(message);
                                state.consecutive_failures =
                                    state.consecutive_failures.saturating_add(1);
                            });
                            if settings_rx.changed().await.is_ok() {
                                refresh_after_settings_change = true;
                            }
                            continue;
                        }
                    };
                    let normal_interval = update_settings.interval_secs;
                    server.update_metadata_runtime_state(|state| {
                        state.status = "updating";
                        state.last_attempt_at_ms = Some(metadata_now_ms());
                    });
                    let result = match DriverMetadataUpdater::new(update_settings) {
                        Ok(updater) => {
                            let update = updater.update_once();
                            tokio::pin!(update);
                            tokio::select! {
                                result = &mut update => Some(result),
                                changed = settings_rx.changed() => {
                                    match changed {
                                        Ok(()) => {
                                            info!("aicc driver metadata update canceled after settings changed");
                                            refresh_after_settings_change = true;
                                            None
                                        }
                                        Err(err) => Some(Err(anyhow::anyhow!(
                                            "metadata settings channel closed: {}",
                                            err
                                        ))),
                                    }
                                }
                            }
                        }
                        Err(err) => Some(Err(err)),
                    };
                    let Some(result) = result else {
                        consecutive_failures = 0;
                        continue;
                    };
                    let delay = match result {
                        Ok(DriverMetadataUpdateOutcome::Activated { revision_seq }) => {
                            consecutive_failures = 0;
                            let refresh_error_count = refresh_provider_inventories_for_metadata(
                                server.as_ref(),
                                "activation-committed",
                            )
                            .await;
                            refresh_after_settings_change = false;
                            server.update_metadata_runtime_state(|state| {
                                record_metadata_update_success(
                                    state,
                                    revision_seq,
                                    Some(refresh_error_count),
                                );
                            });
                            normal_interval
                        }
                        Ok(DriverMetadataUpdateOutcome::Unchanged { revision_seq }) => {
                            consecutive_failures = 0;
                            info!("aicc driver metadata unchanged revision={}", revision_seq);
                            let retry_provider_refresh = refresh_after_settings_change
                                || server.metadata_runtime_state().status == "degraded";
                            let refresh_error_count = if retry_provider_refresh {
                                Some(
                                    refresh_provider_inventories_for_metadata(
                                        server.as_ref(),
                                        "activation-unchanged",
                                    )
                                    .await,
                                )
                            } else {
                                None
                            };
                            refresh_after_settings_change = false;
                            server.update_metadata_runtime_state(|state| {
                                record_metadata_update_success(
                                    state,
                                    revision_seq,
                                    refresh_error_count,
                                );
                            });
                            normal_interval
                        }
                        Err(err) => {
                            consecutive_failures = consecutive_failures.saturating_add(1);
                            let delay =
                                metadata_retry_delay(consecutive_failures, normal_interval);
                            if refresh_after_settings_change {
                                refresh_provider_inventories_for_metadata(
                                    server.as_ref(),
                                    "update-failed-after-settings-change",
                                )
                                .await;
                                refresh_after_settings_change = false;
                            }
                            let message =
                                redact_metadata_error(err.to_string().as_str(), &settings_value);
                            warn!(
                                "aicc driver metadata update failed failures={} retry_secs={} err={}",
                                consecutive_failures,
                                delay,
                                message
                            );
                            server.update_metadata_runtime_state(|state| {
                                state.status = "error";
                                state.last_error = Some(message);
                                state.consecutive_failures = consecutive_failures;
                            });
                            delay
                        }
                    };
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(delay.max(1))) => {}
                        changed = settings_rx.changed() => {
                            if changed.is_ok() {
                                refresh_after_settings_change = true;
                            }
                        }
                    }
                }
            });
        });
    if let Err(err) = spawn_result {
        error!("start metadata updater thread failed: {}", err);
    }
}

async fn refresh_provider_inventories_for_metadata(server: &AiccHttpServer, reason: &str) -> usize {
    let (refreshed, refresh_errors) = server
        .rpc_handler
        .0
        .refresh_all_provider_inventories()
        .await;
    info!(
        "aicc driver metadata provider refresh reason={} refreshed_providers={} refresh_errors={}",
        reason,
        refreshed,
        refresh_errors.len()
    );
    for (provider_instance_name, err) in refresh_errors.iter() {
        warn!(
            "aicc driver metadata provider refresh failed reason={} provider_instance_name={} err={}",
            reason, provider_instance_name, err
        );
    }
    refresh_errors.len()
}

fn metadata_retry_delay(consecutive_failures: u32, normal_interval: u64) -> u64 {
    let shift = consecutive_failures.saturating_sub(1).min(16);
    let base = 60u64.saturating_mul(1u64 << shift);
    let capped = base.min(normal_interval).max(1);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as u64 % (capped / 4 + 1))
        .unwrap_or(0);
    capped.saturating_add(jitter).min(normal_interval.max(1))
}

fn filter_model_directory_by_path(mut value: Value, logical_path: &str) -> Value {
    let mut keep_paths = HashSet::new();
    if let Some(directory) = value.get("directory").and_then(Value::as_object) {
        for path in directory.keys() {
            if logical_path_matches(path, logical_path) {
                keep_paths.insert(path.clone());
            }
        }
    }
    if let Some(definitions) = value.get("logical_definitions").and_then(Value::as_array) {
        for definition in definitions {
            if let Some(path) = definition.get("path").and_then(Value::as_str) {
                if logical_path_matches(path, logical_path) {
                    keep_paths.insert(path.to_string());
                }
            }
        }
    }
    keep_paths.insert(logical_path.to_string());

    if let Some(directory) = value.get_mut("directory").and_then(Value::as_object_mut) {
        directory.retain(|path, _| keep_paths.contains(path));
    }

    if let Some(definitions) = value
        .get_mut("logical_definitions")
        .and_then(Value::as_array_mut)
    {
        definitions.retain(|definition| {
            definition
                .get("path")
                .and_then(Value::as_str)
                .map(|path| keep_paths.contains(path))
                .unwrap_or(false)
        });
    }

    if let Some(providers) = value.get_mut("providers").and_then(Value::as_array_mut) {
        providers.retain_mut(|provider| {
            let Some(models) = provider.get_mut("models").and_then(Value::as_array_mut) else {
                return false;
            };
            models.retain(|model| model_mounts_under_path(model, logical_path));
            !models.is_empty()
        });
    }

    value
}

fn logical_path_matches(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{}.", root))
}

fn model_mounts_under_path(model: &Value, logical_path: &str) -> bool {
    model
        .get("logical_mounts")
        .and_then(Value::as_array)
        .map(|mounts| {
            mounts.iter().any(|mount| {
                mount
                    .as_str()
                    .map(|path| logical_path_matches(path, logical_path))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

#[async_trait::async_trait]
impl RPCHandler for AiccHttpServer {
    async fn handle_rpc_call(
        &self,
        mut req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        if req.token.is_none() {
            req.token = param_string(&req.params, "session_token");
        }
        if req.method == METHOD_RELOAD_SETTINGS
            || req.method == METHOD_SERVICE_RELOAD_SETTINGS
            || req.method == METHOD_REALOAD_SETTINGS
            || req.method == METHOD_SERVICE_REALOAD_SETTINGS
        {
            let result = self.handle_reload_settings().await?;
            return Ok(rpc_success(&req, result));
        }
        if req.method == METHOD_MODELS_LIST || req.method == METHOD_SERVICE_MODELS_LIST {
            let result = self.handle_models_list(&req.params).await?;
            return Ok(rpc_success(&req, result));
        }
        if req.method == METHOD_PROVIDER_VALIDATE {
            let result = self.handle_provider_validate(&req.params).await?;
            return Ok(rpc_success(&req, result));
        }
        if req.method == METHOD_PROVIDER_ADD {
            let result = self.handle_provider_add(&req, ip_from).await?;
            return Ok(rpc_success(&req, result));
        }
        if req.method == METHOD_PROVIDER_DELETE {
            let result = self.handle_provider_delete(&req, ip_from).await?;
            return Ok(rpc_success(&req, result));
        }
        if req.method == METHOD_PROVIDER_REFRESH_MODELS {
            let result = self.handle_provider_refresh_models(&req.params).await?;
            return Ok(rpc_success(&req, result));
        }
        if req.method == ai_methods::DRIVER_METADATA_UPDATE_GET {
            let result = self
                .handle_driver_metadata_update_get(&req, ip_from)
                .await?;
            return Ok(rpc_success(
                &req,
                serde_json::to_value(result).map_err(|err| {
                    RPCErrors::ReasonError(format!(
                        "serialize driver metadata update view failed: {}",
                        err
                    ))
                })?,
            ));
        }
        if req.method == ai_methods::DRIVER_METADATA_UPDATE_SET {
            let result = self
                .handle_driver_metadata_update_set(&req, ip_from)
                .await?;
            return Ok(rpc_success(
                &req,
                serde_json::to_value(result).map_err(|err| {
                    RPCErrors::ReasonError(format!(
                        "serialize driver metadata update response failed: {}",
                        err
                    ))
                })?,
            ));
        }
        if req.method == METHOD_USAGE_QUERY {
            let result = self.handle_usage_query(&req.params).await?;
            return Ok(rpc_success(&req, result));
        }
        if req.method == METHOD_TRACE_QUERY {
            let result = self.handle_trace_query(&req.params).await?;
            return Ok(rpc_success(&req, result));
        }
        self.rpc_handler.handle_rpc_call(req, ip_from).await
    }
}

#[async_trait::async_trait]
impl HttpServer for AiccHttpServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        "aicc".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_aicc_service(mut center: AIComputeCenter) -> Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        AICC_SERVICE_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    let login_result = runtime.login().await;
    if login_result.is_err() {
        error!(
            "aicc service login to system failed! err:{:?}",
            login_result
        );
        return Err(anyhow::anyhow!(
            "aicc service login to system failed! err:{:?}",
            login_result
        ));
    }
    runtime.set_main_service_port(AICC_SERVICE_MAIN_PORT).await;
    let taskmgr = runtime
        .get_task_mgr_client()
        .await
        .map_err(|err| anyhow::anyhow!("init task-manager client for aicc failed: {}", err))?;
    center.set_task_manager_client(Arc::new(taskmgr));
    center.set_resource_resolver(Arc::new(NamedStoreResourceResolver));

    let settings = match runtime.get_my_settings().await {
        Ok(settings) => Some(settings),
        Err(err) => {
            warn!("load aicc settings failed, wait for retry, err={}", err);
            None
        }
    };
    if let Some(settings) = settings.as_ref() {
        match apply_provider_settings(&center, settings) {
            Ok(registered) => {
                info!("aicc providers initialized with {} instances", registered);
            }
            Err(err) => {
                warn!(
                    "aicc settings apply failed during startup, continue without providers: {}",
                    err
                );
            }
        }
    }

    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow::anyhow!("register aicc runtime failed: {}", err))?;

    match AiccUsageLogDb::open_from_service_spec().await {
        Ok(db) => {
            info!("aicc usage-log db opened");
            center.set_usage_log_db(Arc::new(db));
        }
        Err(err) => {
            warn!(
                "open aicc usage-log db failed, usage events will not be persisted: {}",
                err
            );
        }
    }

    let server = Arc::new(AiccHttpServer::new(center, settings));
    start_driver_metadata_update_loop(server.clone());

    let runner = Runner::new(AICC_SERVICE_MAIN_PORT);
    if let Err(err) = runner.add_http_server("/kapi/aicc".to_string(), server) {
        error!("failed to add aicc http server: {:?}", err);
        return Err(anyhow::anyhow!("failed to add aicc http server: {:?}", err));
    }
    if let Err(err) = runner.run().await {
        error!("aicc runner exited with error: {:?}", err);
        return Err(anyhow::anyhow!("aicc runner exited with error: {:?}", err));
    }

    info!("aicc service started at port {}", AICC_SERVICE_MAIN_PORT);
    Ok(())
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    if let Err(err) = rt.block_on(async {
        init_logging("aicc", true);
        let center = AIComputeCenter::default();
        start_aicc_service(center).await
    }) {
        error!("aicc service start failed: {:?}", err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_log_redacts_source_url_userinfo() {
        let settings = serde_json::json!({
            "driver_metadata_update": {
                "source_url": "https://user:secret@metadata.example/aicc/driver-metadata/index.json"
            }
        });
        let redacted = redact_settings_for_log(&settings).to_string();
        assert!(!redacted.contains("user"));
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("metadata.example"));
    }

    #[test]
    fn metadata_error_redacts_userinfo_from_derived_urls() {
        let settings = serde_json::json!({
            "driver_metadata_update": {
                "source_url": "https://user:secret@metadata.example/aicc/driver-metadata/index.json"
            }
        });
        let error = "download manifest failed for https://user:secret@metadata.example/aicc/driver-metadata/v1/manifest-2.json";
        let redacted = redact_metadata_error(error, &settings);
        assert!(!redacted.contains("user"));
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("https://***@metadata.example/"));
    }

    #[test]
    fn metadata_refresh_state_stays_degraded_until_retry_succeeds() {
        let mut state = DriverMetadataRuntimeState::for_settings(None);
        record_metadata_update_success(&mut state, 7, Some(2));
        assert_eq!(state.status, "degraded");
        assert_eq!(state.active_revision, Some(7));
        assert_eq!(
            state.last_error.as_deref(),
            Some("2 provider inventory refreshes failed")
        );

        record_metadata_update_success(&mut state, 7, Some(0));
        assert_eq!(state.status, "healthy");
        assert_eq!(state.last_error, None);
    }

    #[test]
    fn metadata_update_settings_can_be_enabled_and_disabled_without_losing_source() {
        let mut settings = json!({});
        update_driver_metadata_settings(
            &mut settings,
            &DriverMetadataUpdateSetReq::new(
                true,
                Some("https://metadata.example/aicc/driver-metadata/index.json".to_string()),
                Some(900),
            ),
        )
        .unwrap();
        assert_eq!(settings["driver_metadata_update"]["enabled"], true);
        assert_eq!(settings["driver_metadata_update"]["interval_secs"], 900);

        update_driver_metadata_settings(
            &mut settings,
            &DriverMetadataUpdateSetReq::new(false, None, None),
        )
        .unwrap();
        assert_eq!(settings["driver_metadata_update"]["enabled"], false);
        assert_eq!(
            settings["driver_metadata_update"]["source_url"],
            "https://metadata.example/aicc/driver-metadata/index.json"
        );
    }

    #[test]
    fn metadata_update_settings_reject_invalid_enabled_source() {
        let mut settings = json!({});
        assert!(update_driver_metadata_settings(
            &mut settings,
            &DriverMetadataUpdateSetReq::new(
                true,
                Some("http://metadata.example/index.json".to_string()),
                None,
            ),
        )
        .is_err());
    }

    #[test]
    fn metadata_update_interval_is_normalized_before_persisting_and_reading() {
        let mut settings = json!({});
        update_driver_metadata_settings(
            &mut settings,
            &DriverMetadataUpdateSetReq::new(
                true,
                Some("https://metadata.example/aicc/driver-metadata/index.json".to_string()),
                Some(1),
            ),
        )
        .unwrap();
        assert_eq!(settings["driver_metadata_update"]["interval_secs"], 60);

        update_driver_metadata_settings(
            &mut settings,
            &DriverMetadataUpdateSetReq::new(true, None, Some(100_000)),
        )
        .unwrap();
        assert_eq!(settings["driver_metadata_update"]["interval_secs"], 86_400);

        settings["driver_metadata_update"]["interval_secs"] = json!(1);
        let runtime = DriverMetadataRuntimeState::for_settings(Some(&settings));
        assert_eq!(
            driver_metadata_update_view(&settings, &runtime).interval_secs,
            60
        );
    }

    #[test]
    fn metadata_update_set_reports_background_refresh_scheduling() {
        let settings = json!({
            "driver_metadata_update": {
                "enabled": true,
                "source_url": "https://metadata.example/aicc/driver-metadata/index.json",
                "interval_secs": 900,
            }
        });
        let result = driver_metadata_update_set_result(
            42,
            driver_metadata_update_view(
                &settings,
                &DriverMetadataRuntimeState::for_settings(Some(&settings)),
            ),
            DriverMetadataRuntimeApply {
                ok: true,
                refresh_scheduled: Some(true),
                error: None,
            },
        );

        assert!(result.ok);
        assert_eq!(result.settings_revision, 42);
        assert!(result.settings.enabled);
        assert_eq!(result.settings.status, DriverMetadataUpdateStatus::Idle);
        assert!(result.runtime_apply.ok);
        assert_eq!(result.runtime_apply.refresh_scheduled, Some(true));
    }

    #[test]
    fn typed_metadata_request_ignores_transport_token_only() {
        let params = typed_request_params(&json!({
            "enabled": true,
            "source_url": "https://metadata.example/aicc/driver-metadata/index.json",
            "session_token": "test-token",
        }));
        assert!(DriverMetadataUpdateSetReq::from_json(params).is_ok());

        let params = typed_request_params(&json!({
            "enabled": true,
            "unexpected": true,
            "session_token": "test-token",
        }));
        assert!(DriverMetadataUpdateSetReq::from_json(params).is_err());
    }
}
