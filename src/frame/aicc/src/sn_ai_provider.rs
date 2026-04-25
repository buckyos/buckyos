use crate::aicc::{AIComputeCenter, Provider};
use crate::openai::{OpenAIInstanceConfig, OpenAIProvider};
use anyhow::{anyhow, Result};
use log::info;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

const SN_AI_PROVIDER_SETTINGS_KEY: &str = "sn-ai-provider";
const DEFAULT_SN_AI_PROVIDER_BASE_URL: &str = "https://sn.buckyos.ai/api/v1/ai/";
const DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS: u64 = 300_000;
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
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_auth_mode")]
    auth_mode: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
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

fn default_auth_mode() -> String {
    DEFAULT_AUTH_MODE.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS
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
            base_url: default_base_url(),
            auth_mode: default_auth_mode(),
            timeout_ms: default_timeout_ms(),
        }]
    } else {
        settings.instances.clone()
    };

    let mut instances = vec![];
    for raw_instance in raw_instances.into_iter() {
        instances.push(OpenAIInstanceConfig {
            provider_instance_name: raw_instance.provider_instance_name,
            provider_type: raw_instance.provider_type,
            base_url: raw_instance.base_url,
            auth_mode: raw_instance.auth_mode,
            timeout_ms: raw_instance.timeout_ms,
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
        let provider = Arc::new(OpenAIProvider::new(config.clone(), api_token.as_str())?);
        provider.clone().start_inventory_refresh();
        prepared.push((config.clone(), provider));
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
                base_url: "https://sn.buckyos.ai/api/v1/ai/".to_string(),
                auth_mode: "device_jwt".to_string(),
                timeout_ms: default_timeout_ms(),
            }],
        };

        let instances = build_sn_ai_provider_instances(&settings).expect("instances");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].auth_mode, "device_jwt");
        assert_eq!(instances[0].provider_instance_name, "sn-ai-provider-1");
    }

    #[test]
    fn parse_sn_ai_provider_settings_accepts_instance_id_alias() {
        let settings = json!({
            "sn-ai-provider": {
                "enabled": true,
                "instances": [
                    {
                        "instance_id": "sn-ai-provider-alias"
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
