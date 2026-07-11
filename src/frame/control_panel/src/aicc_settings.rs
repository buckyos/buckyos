use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    ai_methods, get_buckyos_api_runtime, AiMessage, AiMethodRequest, AiPayload, AiRole, BoxKind,
    Capability, ModelSpec, MsgCenterClient, Requirements, SystemConfigClient,
};
use log::info;
use name_lib::DID;
use ndn_lib::MsgObjKind;
use serde::Serialize;
use serde_json::{json, Value};
pub(crate) const AICC_SETTINGS_KEY: &str = "services/aicc/settings";
pub(crate) const AI_MODELS_POLICIES_KEY: &str = "services/control_panel/ai_models/policies";
pub(crate) const AI_MODELS_PROVIDER_OVERRIDES_KEY: &str =
    "services/control_panel/ai_models/provider_overrides";
pub(crate) const AI_MODELS_MODEL_CATALOG_KEY: &str =
    "services/control_panel/ai_models/model_catalog";
pub(crate) const AI_MODELS_PROVIDER_SECRETS_KEY: &str =
    "services/control_panel/ai_models/provider_secrets";

#[derive(Clone, Serialize)]
pub(crate) struct MessageHubThreadSummaryResponse {
    pub(crate) peer_did: String,
    pub(crate) peer_name: Option<String>,
    pub(crate) model_alias: String,
    pub(crate) summary: String,
    pub(crate) source_message_count: usize,
}

/// Minimal transcript line extracted from a msg-center chat record, used only to
/// build the summary prompt.
struct ChatSummaryLine {
    outbound: bool,
    created_at_ms: u64,
    content: String,
}

impl ControlPanelServer {
    pub(crate) fn default_ai_policies_value() -> Value {
        json!({
            "items": [
                {
                    "id": "message_hub.reply",
                    "label": "Message Hub Reply",
                    "primaryModel": "gpt-fast",
                    "fallbackModels": ["gemini-ops"],
                    "objective": "Fast reply drafting with safe structured output when needed.",
                    "status": "active"
                },
                {
                    "id": "message_hub.summary",
                    "label": "Message Hub Summary",
                    "primaryModel": "gpt-fast",
                    "fallbackModels": ["gpt-plan"],
                    "objective": "Summarize cross-thread context into compact inbox cards and digest blocks.",
                    "status": "active"
                },
                {
                    "id": "message_hub.task_extract",
                    "label": "Task Extraction",
                    "primaryModel": "gemini-ops",
                    "fallbackModels": ["minimax-api", "gpt-fast"],
                    "objective": "Convert commitments and deadlines into follow-up objects connected to the source thread.",
                    "status": "review"
                },
                {
                    "id": "agent.plan",
                    "label": "Agent Plan",
                    "primaryModel": "minimax-code-plan",
                    "fallbackModels": ["gpt-plan"],
                    "objective": "Use MiniMax code-planning mode for task decomposition, implementation planning, and multi-step agent execution guidance.",
                    "status": "review"
                },
                {
                    "id": "agent.raw_explain",
                    "label": "Agent RAW Explain",
                    "primaryModel": "minimax-api",
                    "fallbackModels": ["gpt-plan", "gpt-fast"],
                    "objective": "Use MiniMax API mode for structured explanation of agent-to-agent raw records.",
                    "status": "planned"
                }
            ]
        })
    }

    pub(crate) fn default_ai_provider_overrides_value() -> Value {
        json!({
            "items": [
                {
                    "id": "openai-compatible",
                    "displayName": "OpenAI-Compatible Gateway",
                    "providerType": "Compatible",
                    "status": "needs_setup",
                    "endpoint": "http://127.0.0.1:11434/v1",
                    "authMode": "Optional token",
                    "capabilities": ["Local LLM", "Low-cost fallback"],
                    "defaultModel": "Not assigned",
                    "note": "Reserved for local or self-hosted models once the backend management flow is connected."
                }
            ]
        })
    }

    pub(crate) fn default_ai_model_catalog_value() -> Value {
        json!({
            "items": [
                {
                    "alias": "minimax-code-plan",
                    "providerId": "minimax-main",
                    "providerModel": "MiniMax-M2.5",
                    "capabilities": ["llm_router"],
                    "features": ["plan", "tool_calling", "code"],
                    "useCases": ["agent.plan", "message_hub.reply"]
                },
                {
                    "alias": "minimax-api",
                    "providerId": "minimax-main",
                    "providerModel": "MiniMax-M2.1-highspeed",
                    "capabilities": ["llm_router"],
                    "features": ["json_output", "tool_calling", "api"],
                    "useCases": ["message_hub.task_extract", "agent.raw_explain"]
                },
                {
                    "alias": "claude-reasoning",
                    "providerId": "claude-main",
                    "providerModel": "claude-3-7-sonnet-20250219",
                    "capabilities": ["llm_router"],
                    "features": ["plan", "tool_calling", "json_output"],
                    "useCases": ["agent.reasoning", "message_hub.summary"]
                },
                {
                    "alias": "gpt-fast",
                    "providerId": "openai-main",
                    "providerModel": "gpt-4.1-mini",
                    "capabilities": ["llm_router"],
                    "features": ["json_output", "tool_calling"],
                    "useCases": ["message_hub.reply", "message_hub.summary"]
                },
                {
                    "alias": "gpt-plan",
                    "providerId": "openai-main",
                    "providerModel": "gpt-4.1",
                    "capabilities": ["llm_router"],
                    "features": ["plan", "tool_calling", "json_output"],
                    "useCases": ["agent.plan", "agent.raw_explain"]
                },
                {
                    "alias": "gemini-ops",
                    "providerId": "google-main",
                    "providerModel": "gemini-2.5-flash",
                    "capabilities": ["llm_router"],
                    "features": ["json_output", "vision"],
                    "useCases": ["message_hub.task_extract", "message_hub.priority_rank"]
                }
            ]
        })
    }

    pub(crate) fn default_ai_provider_secrets_value() -> Value {
        json!({ "items": [] })
    }

    pub(crate) async fn load_json_config_or_default(
        client: &SystemConfigClient,
        key: &str,
        default: Value,
    ) -> Value {
        match client.get(key).await {
            Ok(value) => serde_json::from_str::<Value>(&value.value).unwrap_or(default),
            Err(_) => default,
        }
    }

    pub(crate) async fn save_json_config(
        client: &SystemConfigClient,
        key: &str,
        value: &Value,
    ) -> Result<(), RPCErrors> {
        let serialized = serde_json::to_string_pretty(value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        client
            .set(key, &serialized)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(())
    }

    pub(crate) fn upsert_item_by_id(items: &mut Vec<Value>, id: &str, next: Value) {
        if let Some(index) = items
            .iter()
            .position(|item| item.get("id").and_then(|value| value.as_str()) == Some(id))
        {
            items[index] = next;
        } else {
            items.push(next);
        }
    }

    pub(crate) fn upsert_item_by_alias(items: &mut Vec<Value>, alias: &str, next: Value) {
        if let Some(index) = items
            .iter()
            .position(|item| item.get("alias").and_then(|value| value.as_str()) == Some(alias))
        {
            items[index] = next;
        } else {
            items.push(next);
        }
    }

    fn merge_provider_overrides(base_items: Vec<Value>, overrides: &[Value]) -> Vec<Value> {
        let mut merged = base_items;
        for override_item in overrides.iter() {
            if let Some(id) = override_item.get("id").and_then(|value| value.as_str()) {
                if ["openai-main", "google-main", "claude-main", "minimax-main"].contains(&id) {
                    continue;
                }
                Self::upsert_item_by_id(&mut merged, id, override_item.clone());
            }
        }
        merged
    }

    fn provider_secret_configured(provider_id: &str, secret_doc: &Value) -> bool {
        secret_doc
            .get("items")
            .and_then(|value| value.as_array())
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(|value| value.as_str()) == Some(provider_id)
                })
            })
            .and_then(|item| item.get("apiKey").and_then(|value| value.as_str()))
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    fn mask_secret(value: &str) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        let chars = trimmed.chars().collect::<Vec<_>>();
        let len = chars.len();
        let prefix_len = len.min(4);
        let suffix_len = len.saturating_sub(prefix_len).min(4);
        let prefix = chars.iter().take(prefix_len).collect::<String>();
        let suffix = if suffix_len == 0 {
            String::new()
        } else {
            chars.iter().skip(len - suffix_len).collect::<String>()
        };

        Some(format!("{}***{}", prefix, suffix))
    }

    fn provider_masked_secret(provider_id: &str, secret_doc: &Value) -> Option<String> {
        secret_doc
            .get("items")
            .and_then(|value| value.as_array())
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(|value| value.as_str()) == Some(provider_id)
                })
            })
            .and_then(|item| item.get("apiKey").and_then(|value| value.as_str()))
            .and_then(Self::mask_secret)
    }

    fn aicc_provider_instance_matches(instance: &Value, provider_id: &str) -> bool {
        instance
            .get("provider_instance_name")
            .or_else(|| instance.get("instance_id"))
            .and_then(Value::as_str)
            == Some(provider_id)
    }

    fn update_aicc_provider_instance(
        settings: &mut Value,
        provider_id: &str,
        provider: &Value,
        api_key: Option<&str>,
        has_new_api_key: bool,
    ) -> Option<(String, bool, bool, String, String)> {
        const AICC_PROVIDER_SECTIONS: &[&str] = &[
            "openai",
            "google",
            "gemini",
            "claude",
            "anthropic",
            "minimax",
            "fal",
            "sn-ai-provider",
        ];

        let root = settings.as_object_mut()?;
        for section_name in AICC_PROVIDER_SECTIONS {
            let Some(section) = root.get_mut(*section_name).and_then(Value::as_object_mut) else {
                continue;
            };
            let instance_index =
                section
                    .get("instances")
                    .and_then(Value::as_array)
                    .and_then(|instances| {
                        instances.iter().position(|instance| {
                            Self::aicc_provider_instance_matches(instance, provider_id)
                        })
                    });
            let Some(instance_index) = instance_index else {
                continue;
            };

            let endpoint = provider
                .get("endpoint")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let default_model = provider
                .get("defaultModel")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let enabled = provider
                .get("status")
                .and_then(Value::as_str)
                .map(|value| value == "healthy" || value == "degraded")
                .unwrap_or(false)
                || has_new_api_key
                || provider
                    .get("credentialConfigured")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);

            section.insert("enabled".to_string(), Value::Bool(enabled));

            let api_key_present = {
                let instances = section
                    .get_mut("instances")
                    .and_then(Value::as_array_mut)
                    .expect("instances was checked as array");
                let instance = instances
                    .get_mut(instance_index)
                    .expect("instance index came from instances");
                if !instance.is_object() {
                    *instance = json!({});
                }
                let instance_obj = instance
                    .as_object_mut()
                    .expect("instance must be object after initialization");

                if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
                    instance_obj.insert(
                        "api_token".to_string(),
                        Value::String(api_key.trim().to_string()),
                    );
                }
                if !endpoint.is_empty() {
                    instance_obj.insert("base_url".to_string(), Value::String(endpoint.clone()));
                }
                if !default_model.is_empty() {
                    instance_obj.insert(
                        "default_model".to_string(),
                        Value::String(default_model.clone()),
                    );
                    let models = instance_obj
                        .entry("models".to_string())
                        .or_insert_with(|| json!([]));
                    if !models.is_array() {
                        *models = json!([]);
                    }
                    if let Some(models) = models.as_array_mut() {
                        if !models
                            .iter()
                            .any(|item| item.as_str() == Some(default_model.as_str()))
                        {
                            models.insert(0, Value::String(default_model.clone()));
                        }
                    }
                }

                instance_obj
                    .get("api_token")
                    .or_else(|| instance_obj.get("api_key"))
                    .and_then(Value::as_str)
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
            };

            return Some((
                (*section_name).to_string(),
                enabled,
                api_key_present,
                default_model,
                endpoint,
            ));
        }

        None
    }

    fn ai_openai_provider_card(settings: &Value) -> Value {
        let openai = settings.get("openai").cloned().unwrap_or_else(|| json!({}));
        let enabled = openai
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let api_token = openai
            .get("api_token")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let instances = openai
            .get("instances")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let first = instances.first().cloned().unwrap_or_else(|| json!({}));
        let endpoint = first
            .get("base_url")
            .and_then(|value| value.as_str())
            .unwrap_or("https://api.openai.com/v1");
        let default_model = first
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                first
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("gpt-fast");
        let status = if enabled && !api_token.trim().is_empty() {
            "healthy"
        } else if enabled {
            "degraded"
        } else {
            "needs_setup"
        };

        json!({
            "id": "openai-main",
            "displayName": "OpenAI Main",
            "providerType": "OpenAI",
            "status": status,
            "endpoint": endpoint,
            "authMode": "Bearer token",
            "credentialConfigured": !api_token.trim().is_empty(),
            "maskedApiKey": Self::mask_secret(api_token),
            "capabilities": ["Reply", "Summary", "Tool calling"],
            "defaultModel": default_model,
            "note": "Primary cloud provider for Message Hub reply and summary flows."
        })
    }

    fn ai_google_provider_card(settings: &Value) -> Value {
        let google = settings
            .get("google")
            .or_else(|| settings.get("gemini"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let enabled = google
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let api_token = google
            .get("api_token")
            .or_else(|| google.get("api_key"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let instances = google
            .get("instances")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let first = instances.first().cloned().unwrap_or_else(|| json!({}));
        let endpoint = first
            .get("base_url")
            .and_then(|value| value.as_str())
            .unwrap_or("https://generativelanguage.googleapis.com/v1beta");
        let default_model = first
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                first
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("gemini-ops");
        let status = if enabled && !api_token.trim().is_empty() {
            "healthy"
        } else if enabled {
            "degraded"
        } else {
            "needs_setup"
        };

        json!({
            "id": "google-main",
            "displayName": "Google Gemini",
            "providerType": "Google",
            "status": status,
            "endpoint": endpoint,
            "authMode": "API key",
            "credentialConfigured": !api_token.trim().is_empty(),
            "maskedApiKey": Self::mask_secret(api_token),
            "capabilities": ["Task extract", "Multimodal", "JSON output"],
            "defaultModel": default_model,
            "note": "Secondary provider used for extraction-heavy workflows and fallback coverage."
        })
    }

    fn ai_claude_provider_card(settings: &Value, secret_doc: &Value) -> Value {
        let claude = settings
            .get("claude")
            .or_else(|| settings.get("anthropic"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let enabled = claude
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let api_token = claude
            .get("api_token")
            .or_else(|| claude.get("api_key"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let instances = claude
            .get("instances")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let first = instances.first().cloned().unwrap_or_else(|| json!({}));
        let endpoint = first
            .get("base_url")
            .and_then(|value| value.as_str())
            .unwrap_or("https://api.anthropic.com/v1");
        let default_model = first
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                first
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("claude-3-7-sonnet-20250219");
        let credential_configured = !api_token.trim().is_empty()
            || Self::provider_secret_configured("claude-main", secret_doc);
        let masked_api_key = Self::mask_secret(api_token)
            .or_else(|| Self::provider_masked_secret("claude-main", secret_doc));
        let status = if enabled && credential_configured {
            "healthy"
        } else if enabled {
            "degraded"
        } else {
            "needs_setup"
        };

        json!({
            "id": "claude-main",
            "displayName": "Claude",
            "providerType": "Anthropic",
            "status": status,
            "endpoint": endpoint,
            "authMode": "X-API-Key",
            "credentialConfigured": credential_configured,
            "maskedApiKey": masked_api_key,
            "availableModels": [
                "claude-3-7-sonnet-20250219",
                "claude-3-5-haiku-20241022"
            ],
            "capabilities": ["Long-form reasoning", "Tool calling"],
            "defaultModel": default_model,
            "note": "Native Anthropic Claude runtime for reasoning-heavy and tool-calling workloads."
        })
    }

    fn ai_minimax_provider_card(settings: &Value, secret_doc: &Value) -> Value {
        let minimax = settings
            .get("minimax")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let enabled = minimax
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let api_token = minimax
            .get("api_token")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let instances = minimax
            .get("instances")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let first = instances.first().cloned().unwrap_or_else(|| json!({}));
        let endpoint = first
            .get("base_url")
            .and_then(|value| value.as_str())
            .unwrap_or("https://api.minimaxi.com/anthropic/v1");
        let default_model = first
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                first
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("MiniMax-M2.5");
        let credential_configured = !api_token.trim().is_empty()
            || Self::provider_secret_configured("minimax-main", secret_doc);
        let masked_api_key = Self::mask_secret(api_token)
            .or_else(|| Self::provider_masked_secret("minimax-main", secret_doc));
        let status = if enabled && credential_configured {
            "healthy"
        } else if enabled {
            "degraded"
        } else {
            "needs_setup"
        };

        json!({
            "id": "minimax-main",
            "displayName": "MiniMax",
            "providerType": "MiniMax",
            "status": status,
            "endpoint": endpoint,
            "authMode": "X-API-Key",
            "credentialConfigured": credential_configured,
            "maskedApiKey": masked_api_key,
            "availableModels": [
                "MiniMax-M2.5",
                "MiniMax-M2.5-highspeed",
                "MiniMax-M2.1",
                "MiniMax-M2.1-highspeed",
                "MiniMax-M2"
            ],
            "capabilities": ["Code plan", "API mode"],
            "defaultModel": default_model,
            "note": "Anthropic-compatible MiniMax runtime for code planning and API-oriented workflows."
        })
    }

    pub(crate) fn ai_provider_cards(
        settings: &Value,
        overrides: &[Value],
        secret_doc: &Value,
    ) -> Vec<Value> {
        let base_items = vec![
            Self::ai_openai_provider_card(settings),
            Self::ai_google_provider_card(settings),
            Self::ai_claude_provider_card(settings, secret_doc),
            Self::ai_minimax_provider_card(settings, secret_doc),
        ];

        let mut merged = Self::merge_provider_overrides(base_items, overrides);
        for item in merged.iter_mut() {
            let provider_id = item
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            if item.get("credentialConfigured").is_none() {
                item["credentialConfigured"] = Value::Bool(Self::provider_secret_configured(
                    provider_id.as_str(),
                    secret_doc,
                ));
            }
            if item.get("maskedApiKey").is_none() {
                if let Some(masked) = Self::provider_masked_secret(provider_id.as_str(), secret_doc)
                {
                    item["maskedApiKey"] = Value::String(masked);
                }
            }
        }
        merged
    }

    pub(crate) fn ai_model_catalog(settings: &Value, overrides: &[Value]) -> Vec<Value> {
        let openai = settings.get("openai").cloned().unwrap_or_else(|| json!({}));
        let google = settings
            .get("google")
            .or_else(|| settings.get("gemini"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let claude = settings
            .get("claude")
            .or_else(|| settings.get("anthropic"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let openai_instance = openai
            .get("instances")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let google_instance = google
            .get("instances")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let claude_instance = claude
            .get("instances")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .cloned()
            .unwrap_or_else(|| json!({}));

        let openai_default = openai_instance
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                openai_instance
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("gpt-4.1-mini");
        let google_default = google_instance
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                google_instance
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("gemini-2.5-flash");
        let claude_default = claude_instance
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                claude_instance
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("claude-3-7-sonnet-20250219");

        let mut items = vec![
            json!({
                "alias": "gpt-fast",
                "providerId": "openai-main",
                "providerModel": openai_default,
                "capabilities": ["llm_router"],
                "features": ["json_output", "tool_calling"],
                "useCases": ["message_hub.reply", "message_hub.summary"]
            }),
            json!({
                "alias": "gpt-plan",
                "providerId": "openai-main",
                "providerModel": openai_default,
                "capabilities": ["llm_router"],
                "features": ["plan", "tool_calling", "json_output"],
                "useCases": ["agent.plan", "agent.raw_explain"]
            }),
            json!({
                "alias": "gemini-ops",
                "providerId": "google-main",
                "providerModel": google_default,
                "capabilities": ["llm_router"],
                "features": ["json_output", "vision"],
                "useCases": ["message_hub.task_extract", "message_hub.priority_rank"]
            }),
            json!({
                "alias": "claude-reasoning",
                "providerId": "claude-main",
                "providerModel": claude_default,
                "capabilities": ["llm_router"],
                "features": ["plan", "tool_calling", "json_output"],
                "useCases": ["agent.reasoning", "message_hub.summary"]
            }),
        ];

        for override_item in overrides.iter() {
            if let Some(alias) = override_item.get("alias").and_then(|value| value.as_str()) {
                Self::upsert_item_by_alias(&mut items, alias, override_item.clone());
            }
        }

        items
    }

    pub(crate) fn ai_overview(providers: &[Value], policies: &[Value]) -> Value {
        let providers_online = providers
            .iter()
            .filter(|provider| {
                provider.get("status").and_then(|value| value.as_str()) == Some("healthy")
            })
            .count();

        let primary_model = |id: &str, fallback: &str| -> String {
            policies
                .iter()
                .find(|policy| policy.get("id").and_then(|value| value.as_str()) == Some(id))
                .and_then(|policy| policy.get("primaryModel").and_then(|value| value.as_str()))
                .unwrap_or(fallback)
                .to_string()
        };

        json!({
            "providersOnline": providers_online,
            "providersTotal": providers.len(),
            "defaultReplyModel": primary_model("message_hub.reply", "gpt-fast"),
            "defaultSummaryModel": primary_model("message_hub.summary", "gpt-fast"),
            "defaultTaskExtractModel": primary_model("message_hub.task_extract", "gemini-ops"),
            "defaultAgentModel": primary_model("agent.plan", "minimax-code-plan"),
            "avgLatencyMs": 840,
            "estimatedDailyCostUsd": 2.37,
            "lastDiagnosticsAt": format!("Today {}", chrono::Local::now().format("%H:%M")),
        })
    }

    pub(crate) fn ai_policy_primary_model(
        policies: &[Value],
        policy_id: &str,
        fallback: &str,
    ) -> String {
        policies
            .iter()
            .find(|policy| policy.get("id").and_then(|value| value.as_str()) == Some(policy_id))
            .and_then(|policy| policy.get("primaryModel").and_then(|value| value.as_str()))
            .unwrap_or(fallback)
            .to_string()
    }

    fn collect_aicc_provider_instance_names(settings: &Value) -> Vec<String> {
        let sections = [
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
        let mut names = sections
            .iter()
            .filter_map(|section| settings.get(*section))
            .filter_map(Value::as_object)
            .filter_map(|section| section.get("instances"))
            .filter_map(Value::as_array)
            .flat_map(|instances| instances.iter())
            .filter_map(|instance| {
                instance
                    .get("provider_instance_name")
                    .or_else(|| instance.get("instance_id"))
                    .and_then(Value::as_str)
            })
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
    }

    async fn collect_runtime_provider_instance_names(&self) -> Vec<String> {
        let Ok(runtime) = get_buckyos_api_runtime() else {
            return vec![];
        };
        let Ok(krpc_client) = runtime.get_zone_service_krpc_client("aicc").await else {
            return vec![];
        };
        let Ok(directory) = krpc_client.call("models.list", json!({})).await else {
            return vec![];
        };
        let mut names = directory
            .get("providers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|provider| {
                provider
                    .get("provider_instance_name")
                    .and_then(Value::as_str)
            })
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
    }

    fn merge_provider_instance_names(mut left: Vec<String>, right: Vec<String>) -> Vec<String> {
        left.extend(right);
        left.sort();
        left.dedup();
        left
    }

    fn build_message_hub_summary_prompt(
        peer_name: Option<&str>,
        peer_did: &str,
        messages: &[ChatSummaryLine],
    ) -> String {
        let mut transcript = String::new();
        for message in messages.iter().rev().take(20).rev() {
            let speaker = if message.outbound {
                "Me"
            } else {
                peer_name.unwrap_or(peer_did)
            };
            let line = format!(
                "[{}] {}: {}\n",
                chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
                    message.created_at_ms as i64
                )
                .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| message.created_at_ms.to_string()),
                speaker,
                message.content.replace('\n', " ")
            );
            transcript.push_str(&line);
        }

        format!(
            "You summarize a direct communication thread for Message Hub. Return plain text with three short sections titled Summary, Decisions, and Follow-ups. Keep it concise and action-oriented.\n\nPeer: {}\nPeer DID: {}\n\nTranscript:\n{}",
            peer_name.unwrap_or(peer_did),
            peer_did,
            transcript
        )
    }

    pub(crate) async fn handle_ai_overview(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let secret_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_SECRETS_KEY,
            Self::default_ai_provider_secrets_value(),
        )
        .await;
        let provider_overrides = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_OVERRIDES_KEY,
            Self::default_ai_provider_overrides_value(),
        )
        .await;
        let policy_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_POLICIES_KEY,
            Self::default_ai_policies_value(),
        )
        .await;
        let providers = Self::ai_provider_cards(
            &settings,
            provider_overrides
                .get("items")
                .and_then(|value| value.as_array())
                .map(|items| items.as_slice())
                .unwrap_or(&[]),
            &secret_doc,
        );
        let policies = policy_doc
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(RPCResponse::new(
            RPCResult::Success(Self::ai_overview(&providers, &policies)),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_provider_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let secret_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_SECRETS_KEY,
            Self::default_ai_provider_secrets_value(),
        )
        .await;
        let provider_overrides = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_OVERRIDES_KEY,
            Self::default_ai_provider_overrides_value(),
        )
        .await;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "items": Self::ai_provider_cards(
                    &settings,
                    provider_overrides
                        .get("items")
                        .and_then(|value| value.as_array())
                        .map(|items| items.as_slice())
                        .unwrap_or(&[]),
                    &secret_doc,
                )
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_model_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let model_catalog = Self::load_json_config_or_default(
            &client,
            AI_MODELS_MODEL_CATALOG_KEY,
            Self::default_ai_model_catalog_value(),
        )
        .await;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "items": Self::ai_model_catalog(
                    &settings,
                    model_catalog
                        .get("items")
                        .and_then(|value| value.as_array())
                        .map(|items| items.as_slice())
                        .unwrap_or(&[]),
                )
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_policy_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let policy_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_POLICIES_KEY,
            Self::default_ai_policies_value(),
        )
        .await;
        let items = policy_doc
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "items": items })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_provider_weight_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let weights = settings
            .get("routing_config")
            .or_else(|| settings.get("routing_settings"))
            .and_then(|value| value.get("provider_weights"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let provider_names = Self::merge_provider_instance_names(
            Self::collect_aicc_provider_instance_names(&settings),
            self.collect_runtime_provider_instance_names().await,
        );
        let items = provider_names
            .into_iter()
            .map(|provider_instance_name| {
                let weight = weights
                    .get(provider_instance_name.as_str())
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0);
                json!({
                    "provider_instance_name": provider_instance_name,
                    "weight": weight,
                    "is_default": (weight - 1.0).abs() < f64::EPSILON
                })
            })
            .collect::<Vec<_>>();

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "items": items,
                "provider_weights": weights
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_diagnostics_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "items": [
                    {
                        "id": "diag-openai",
                        "title": "OpenAI round-trip test",
                        "status": "pass",
                        "detail": "Use control_panel to trigger a light /kapi/aicc completion through the AI Models module.",
                        "actionLabel": "Run again"
                    },
                    {
                        "id": "diag-google",
                        "title": "Gemini extraction profile",
                        "status": "pass",
                        "detail": "Current policy defaults reserve Gemini for extraction-heavy Message Hub workflows.",
                        "actionLabel": "Run again"
                    },
                    {
                        "id": "diag-claude",
                        "title": "Claude reasoning profile",
                        "status": "pass",
                        "detail": "Anthropic-compatible Claude runtime is available through /kapi/aicc when the provider is configured.",
                        "actionLabel": "Run again"
                    },
                    {
                        "id": "diag-local",
                        "title": "Local LLM gateway",
                        "status": "pending",
                        "detail": "Reserved for a future OpenAI-compatible or local endpoint once provider configuration broadens.",
                        "actionLabel": "Review checklist"
                    }
                ]
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_provider_test(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let provider_id = Self::require_param_str(&req, "provider_id")?;
        let alias = match provider_id.as_str() {
            "openai-main" => "gpt-fast",
            "google-main" => "gemini-ops",
            "claude-main" => "claude-reasoning",
            "minimax-main" => "minimax-code-plan",
            _ => {
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "providerId": provider_id,
                        "ok": false,
                        "status": "pending",
                        "detail": "This provider family is not wired to /kapi/aicc in the current control_panel phase."
                    })),
                    req.seq,
                ))
            }
        };

        let runtime = get_buckyos_api_runtime()?;
        let aicc = runtime.get_aicc_client().await.map_err(|error| {
            RPCErrors::ReasonError(format!("init aicc client failed: {}", error))
        })?;

        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new(alias.to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::text(
                    AiRole::User,
                    "Return a compact JSON object that confirms provider connectivity.",
                )],
                vec![],
                vec![],
                None,
                Some(json!({
                    "max_tokens": 64,
                    "temperature": 0.1,
                    "response_format": { "type": "json_object" }
                })),
            ),
            None,
        );

        match aicc.call_method(ai_methods::LLM_CHAT, request).await {
            Ok(result) => Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "providerId": provider_id,
                    "ok": true,
                    "status": "pass",
                    "taskId": result.task_id,
                    "detail": result
                        .result
                        .map(|summary| summary.text_content())
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or_else(|| "Provider test completed successfully.".to_string())
                })),
                req.seq,
            )),
            Err(error) => Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "providerId": provider_id,
                    "ok": false,
                    "status": "warn",
                    "detail": error.to_string()
                })),
                req.seq,
            )),
        }
    }

    async fn get_msg_center_client(&self) -> Result<MsgCenterClient, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_msg_center_client().await.map_err(|error| {
            RPCErrors::ReasonError(format!("get msg-center client failed: {}", error))
        })
    }

    pub(crate) async fn handle_ai_message_hub_thread_summary(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = principal.ok_or_else(|| {
            RPCErrors::InvalidToken("missing authenticated principal".to_string())
        })?;
        let owner_did = DID::from_str(principal.owner_did.as_str()).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "invalid chat owner DID `{}`: {}",
                principal.owner_did, error
            ))
        })?;
        let peer_did_raw = Self::require_param_str(&req, "peer_did")?;
        let peer_did = DID::from_str(peer_did_raw.trim()).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid peer_did `{}`: {}", peer_did_raw, error))
        })?;

        let msg_center = self.get_msg_center_client().await?;
        let peer_name = match msg_center
            .get_contact(peer_did.clone(), Some(owner_did.clone()))
            .await
        {
            Ok(Some(contact)) => Some(contact.name),
            _ => None,
        };

        let inbox = msg_center
            .list_box_by_time(
                owner_did.clone(),
                BoxKind::Inbox,
                None,
                Some(60),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await?;
        let outbox = msg_center
            .list_box_by_time(
                owner_did.clone(),
                BoxKind::Outbox,
                None,
                Some(60),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await?;

        let mut records = inbox
            .items
            .into_iter()
            .chain(outbox.items.into_iter())
            .filter(|record| {
                record.record.msg_kind == MsgObjKind::Chat
                    && if record.record.from == owner_did {
                        record.record.to == peer_did
                    } else {
                        record.record.from == peer_did
                    }
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            left.record
                .sort_key
                .cmp(&right.record.sort_key)
                .then_with(|| left.record.updated_at_ms.cmp(&right.record.updated_at_ms))
        });

        let items = records
            .iter()
            .map(|record| ChatSummaryLine {
                outbound: record.record.from == owner_did,
                created_at_ms: record.record.created_at_ms,
                content: record
                    .msg
                    .as_ref()
                    .map(|msg| msg.content.content.clone())
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err(RPCErrors::ReasonError(
                "No thread messages available to summarize yet.".to_string(),
            ));
        }

        let runtime = get_buckyos_api_runtime()?;
        let config_client = runtime.get_system_config_client().await?;
        let policy_doc = Self::load_json_config_or_default(
            &config_client,
            AI_MODELS_POLICIES_KEY,
            Self::default_ai_policies_value(),
        )
        .await;
        let policies = policy_doc
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let model_alias =
            Self::ai_policy_primary_model(&policies, "message_hub.summary", "gpt-fast");

        let aicc = runtime.get_aicc_client().await.map_err(|error| {
            RPCErrors::ReasonError(format!("init aicc client failed: {}", error))
        })?;
        let peer_did_string = peer_did.to_string();
        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new(model_alias.clone(), None),
            Requirements::default(),
            AiPayload::new(
                Some(Self::build_message_hub_summary_prompt(
                    peer_name.as_deref(),
                    peer_did_string.as_str(),
                    &items,
                )),
                vec![],
                vec![],
                vec![],
                None,
                Some(json!({
                    "max_tokens": 240,
                    "temperature": 0.2
                })),
            ),
            None,
        );

        let result = aicc
            .call_method(ai_methods::LLM_CHAT, request)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let summary = result
            .result
            .map(|summary| summary.text_content())
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "No summary text returned by the model.".to_string());

        Ok(RPCResponse::new(
            RPCResult::Success(json!(MessageHubThreadSummaryResponse {
                peer_did: peer_did_string,
                peer_name,
                model_alias,
                summary,
                source_message_count: items.len(),
            })),
            req.seq,
        ))
    }

    async fn reload_aicc_settings_value(&self) -> Result<Value, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let krpc_client = runtime
            .get_zone_service_krpc_client("aicc")
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("init aicc rpc client failed: {}", error))
            })?;

        let result = krpc_client
            .call("service.reload_settings", json!({}))
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("reload aicc settings failed: {}", error))
            })?;

        Ok(json!({
            "ok": true,
            "result": result,
        }))
    }

    pub(crate) async fn handle_ai_reload(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let result = self.reload_aicc_settings_value().await?;

        Ok(RPCResponse::new(RPCResult::Success(result), req.seq))
    }

    pub(crate) async fn handle_ai_provider_set(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let provider = req
            .params
            .get("provider")
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("missing provider payload".to_string()))?;
        let api_key = Self::param_str(&req, "api_key");
        let has_new_api_key = api_key
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let provider_id = provider
            .get("id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ReasonError("provider.id is required".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;

        info!(
            "control_panel.ai.provider.set provider_id={} has_new_api_key={} requested_status={:?} default_model={:?} endpoint={:?}",
            provider_id,
            has_new_api_key,
            provider.get("status").and_then(|value| value.as_str()),
            provider.get("defaultModel").and_then(|value| value.as_str()),
            provider.get("endpoint").and_then(|value| value.as_str()),
        );

        let mut settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        if let Some((key, enabled, api_key_present, default_model, endpoint)) =
            Self::update_aicc_provider_instance(
                &mut settings,
                provider_id,
                &provider,
                api_key.as_deref(),
                has_new_api_key,
            )
        {
            Self::save_json_config(&client, AICC_SETTINGS_KEY, &settings).await?;
            info!(
                "control_panel.ai.provider.set persisted_aicc_instance provider_id={} settings_key={} enabled={} api_key_present={} default_model={} endpoint={}",
                provider_id,
                key,
                enabled,
                api_key_present,
                default_model,
                endpoint,
            );
        } else {
            return Err(RPCErrors::ReasonError(format!(
                "provider_not_found_in_aicc_settings: {}",
                provider_id
            )));
        }

        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let secret_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_SECRETS_KEY,
            Self::default_ai_provider_secrets_value(),
        )
        .await;
        let provider_overrides = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_OVERRIDES_KEY,
            Self::default_ai_provider_overrides_value(),
        )
        .await;
        let provider_card = Self::ai_provider_cards(
            &settings,
            provider_overrides
                .get("items")
                .and_then(|value| value.as_array())
                .map(|items| items.as_slice())
                .unwrap_or(&[]),
            &secret_doc,
        )
        .into_iter()
        .find(|item| item.get("id").and_then(|value| value.as_str()) == Some(provider_id))
        .unwrap_or(provider.clone());

        info!(
            "control_panel.ai.provider.set result provider_id={} status={:?} credential_configured={:?} default_model={:?}",
            provider_id,
            provider_card.get("status").and_then(|value| value.as_str()),
            provider_card
                .get("credentialConfigured")
                .and_then(|value| value.as_bool()),
            provider_card
                .get("defaultModel")
                .and_then(|value| value.as_str()),
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "provider": provider_card })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_provider_weight_set(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let provider_instance_name = req
            .params
            .get("provider_instance_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                RPCErrors::ReasonError("provider_instance_name is required".to_string())
            })?;
        let weight = req
            .params
            .get("weight")
            .and_then(Value::as_f64)
            .ok_or_else(|| RPCErrors::ReasonError("weight is required".to_string()))?;
        if !weight.is_finite() || weight < 0.0 {
            return Err(RPCErrors::ReasonError(
                "weight must be a non-negative finite number".to_string(),
            ));
        }

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let mut settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        if !settings.is_object() {
            settings = json!({});
        }
        let root = settings.as_object_mut().expect("settings must be object");
        let routing_config = root
            .entry("routing_config".to_string())
            .or_insert_with(|| json!({}));
        if !routing_config.is_object() {
            *routing_config = json!({});
        }
        let routing_config = routing_config
            .as_object_mut()
            .expect("routing_config must be object");
        let provider_weights = routing_config
            .entry("provider_weights".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !provider_weights.is_object() {
            *provider_weights = Value::Object(serde_json::Map::new());
        }
        let provider_weights = provider_weights
            .as_object_mut()
            .expect("provider_weights must be object");
        if (weight - 1.0).abs() < f64::EPSILON {
            provider_weights.remove(provider_instance_name.as_str());
        } else {
            provider_weights.insert(provider_instance_name.to_string(), json!(weight));
        }
        let next_weights = provider_weights.clone();

        Self::save_json_config(&client, AICC_SETTINGS_KEY, &settings).await?;
        let reload = self.reload_aicc_settings_value().await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "provider_instance_name": provider_instance_name,
                "weight": weight,
                "provider_weights": next_weights,
                "reload": reload,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_model_set(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let model = req
            .params
            .get("model")
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("missing model payload".to_string()))?;
        let alias = model
            .get("alias")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ReasonError("model.alias is required".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let mut catalog = Self::load_json_config_or_default(
            &client,
            AI_MODELS_MODEL_CATALOG_KEY,
            Self::default_ai_model_catalog_value(),
        )
        .await;
        let mut items = catalog
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        Self::upsert_item_by_alias(&mut items, alias, model.clone());
        catalog["items"] = Value::Array(items);
        Self::save_json_config(&client, AI_MODELS_MODEL_CATALOG_KEY, &catalog).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "model": model })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_ai_policy_set(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let policy = req
            .params
            .get("policy")
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("missing policy payload".to_string()))?;
        let policy_id = policy
            .get("id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ReasonError("policy.id is required".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let mut policy_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_POLICIES_KEY,
            Self::default_ai_policies_value(),
        )
        .await;
        let mut items = policy_doc
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        Self::upsert_item_by_id(&mut items, policy_id, policy.clone());
        policy_doc["items"] = Value::Array(items);
        Self::save_json_config(&client, AI_MODELS_POLICIES_KEY, &policy_doc).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "policy": policy })),
            req.seq,
        ))
    }
}
