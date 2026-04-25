use crate::aicc::{AIComputeCenter, Provider};
use crate::openai::{OpenAIInstanceConfig, OpenAIProvider};
use anyhow::{anyhow, Result};
use buckyos_api::features;
use log::info;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const SN_AI_PROVIDER_SETTINGS_KEY: &str = "sn-ai-provider";
const DEFAULT_SN_AI_PROVIDER_BASE_URL: &str = "https://sn.buckyos.ai/api/v1/ai/";
const DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_SN_AI_PROVIDER_MODELS: &str = "gpt-5.4,gpt-5.4-mini,gpt-5.4-nano,gpt-5.4-pro";
const DEFAULT_SN_AI_PROVIDER_IMAGE_MODELS: &str = "dall-e-3,dall-e-2";
const DEFAULT_SN_AI_PROVIDER_DRIVER: &str = "sn-ai-provider";
const DEFAULT_AUTH_MODE: &str = "device_jwt";

#[derive(Debug, Deserialize, Default)]
struct SnAIProviderSettings {
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    api_token: String,
    #[serde(default)]
    instances: Vec<SettingsSnAIProviderInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsSnAIProviderInstanceConfig {
    #[serde(default = "default_instance_id", alias = "instance_id")]
    provider_instance_name: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default = "default_provider_driver")]
    provider_driver: String,
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_auth_mode")]
    auth_mode: String,
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

fn default_enabled() -> bool {
    true
}

fn default_instance_id() -> String {
    "sn-ai-provider-default".to_string()
}

fn default_provider_type() -> String {
    "cloud_api".to_string()
}

fn default_provider_driver() -> String {
    DEFAULT_SN_AI_PROVIDER_DRIVER.to_string()
}

fn default_base_url() -> String {
    DEFAULT_SN_AI_PROVIDER_BASE_URL.to_string()
}

fn default_auth_mode() -> String {
    DEFAULT_AUTH_MODE.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS
}

fn default_features() -> Vec<String> {
    vec![
        features::PLAN.to_string(),
        features::JSON_OUTPUT.to_string(),
        features::TOOL_CALLING.to_string(),
        features::WEB_SEARCH.to_string(),
    ]
}

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

fn normalize_model_list(models: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut normalized = vec![];
    for model in models.into_iter() {
        let value = model.trim();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.to_ascii_lowercase()) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn is_text2image_model_name(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.starts_with("dall-e") || normalized == "gpt-image-1"
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
) -> Result<Vec<OpenAIInstanceConfig>> {
    let raw_instances = if settings.instances.is_empty() {
        vec![SettingsSnAIProviderInstanceConfig {
            provider_instance_name: default_instance_id(),
            provider_type: default_provider_type(),
            provider_driver: default_provider_driver(),
            base_url: default_base_url(),
            auth_mode: default_auth_mode(),
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
            models = normalize_model_list(parse_csv_list(DEFAULT_SN_AI_PROVIDER_MODELS));
        }
        if models.is_empty() {
            return Err(anyhow!(
                "sn-ai-provider instance {} has no models configured",
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
                .collect();
        }
        if image_models.is_empty() {
            image_models =
                normalize_model_list(parse_csv_list(DEFAULT_SN_AI_PROVIDER_IMAGE_MODELS));
        }
        let default_image_model = raw_instance
            .default_image_model
            .or_else(|| image_models.first().cloned());
        let features = if raw_instance.features.is_empty() {
            default_features()
        } else {
            raw_instance.features
        };

        instances.push(OpenAIInstanceConfig {
            provider_instance_name: raw_instance.provider_instance_name,
            provider_type: raw_instance.provider_type,
            provider_driver: raw_instance.provider_driver,
            base_url: raw_instance.base_url,
            auth_mode: raw_instance.auth_mode,
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

pub fn register_sn_ai_provider(center: &AIComputeCenter, settings: &Value) -> Result<usize> {
    let Some(sn_settings) = parse_sn_ai_provider_settings(settings)? else {
        info!("aicc sn-ai-provider is disabled (settings.sn-ai-provider missing or disabled)");
        return Ok(0);
    };
    let instances = build_sn_ai_provider_instances(&sn_settings)?;
    let api_token = sn_settings.api_token.trim().to_string();
    let mut prepared = Vec::<(OpenAIInstanceConfig, Arc<dyn Provider>)>::new();
    for config in instances.iter() {
        let provider = OpenAIProvider::new(config.clone(), api_token.as_str())?;
        prepared.push((config.clone(), Arc::new(provider)));
    }

    for (config, provider) in prepared.into_iter() {
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
    use serde_json::json;

    #[test]
    fn build_sn_ai_provider_instances_allows_device_jwt_without_api_token() {
        let settings = SnAIProviderSettings {
            enabled: true,
            api_token: String::new(),
            instances: vec![SettingsSnAIProviderInstanceConfig {
                provider_instance_name: "sn-ai-provider-1".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "sn-ai-provider".to_string(),
                base_url: "https://sn.buckyos.ai/api/v1/ai/".to_string(),
                auth_mode: "device_jwt".to_string(),
                timeout_ms: default_timeout_ms(),
                models: vec!["gpt-5.4-mini".to_string()],
                default_model: Some("gpt-5.4-mini".to_string()),
                image_models: vec![],
                default_image_model: None,
                features: vec![],
                alias_map: HashMap::new(),
            }],
        };

        let instances = build_sn_ai_provider_instances(&settings).expect("instances");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_driver, "sn-ai-provider");
        assert_eq!(instances[0].auth_mode, "device_jwt");
    }

    #[test]
    fn parse_sn_ai_provider_settings_accepts_instance_id_alias() {
        let settings = json!({
            "sn-ai-provider": {
                "enabled": true,
                "instances": [
                    {
                        "instance_id": "sn-ai-provider-alias",
                        "models": ["gpt-5.4-mini"]
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
}
