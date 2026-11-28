//! Inventory snapshots of every built-in Provider: for each sample model, the
//! API types, capabilities, operation IDs, presets and mounts it ends up with.
//! Protocol client IDs are left out, so regrouping operations into clients
//! must not change an entry. The golden file holds one digest per entry;
//! `AICC_INVENTORY_SNAPSHOT_DUMP=<file>` writes the full contents and
//! `AICC_INVENTORY_SNAPSHOT_UPDATE=1` rewrites the golden file.

use super::registry::{
    BuiltinProviderRegistry, BuiltinProviderRequest, CUSTOM_PROVIDER_PROFILE_ID,
};
use super::*;
use crate::catalog::{CatalogBuildOptions, CatalogSnapshot};
use crate::protocol::CredentialKind;
use crate::provider::inventory::ProviderConnectionInput;
use crate::provider::inventory::{DiscoveryContext, StaticCredentialResolver};
use crate::provider::*;
use crate::settings::{load_builtin_metadata, MetadataSources};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

const GOLDEN_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/provider/builtin/inventory_snapshots.golden"
);

fn catalog() -> std::sync::Arc<CatalogSnapshot> {
    MetadataSources {
        builtin: load_builtin_metadata().unwrap(),
        ..Default::default()
    }
    .build_snapshot(
        crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
        &CatalogBuildOptions::default(),
    )
    .unwrap()
}

fn discovered(id: &str) -> DiscoveredModel {
    DiscoveredModel {
        provider_model_id: id.into(),
        api_types: None,
        supported_features: None,
        unsupported_features: BTreeSet::new(),
        remote_methods: None,
        availability: ModelAvailability::Available,
        deprecated: false,
        pricing: None,
        video: None,
    }
}

pub(super) fn sample_model_ids(
    catalog: &CatalogSnapshot,
    profile_id: &str,
    rules_id: &str,
) -> Vec<String> {
    let mut ids = BTreeSet::new();
    if let Some(rules) = catalog.provider_rules(rules_id) {
        ids.extend(rules.models.iter().map(|rule| rule.id.clone()));
        ids.extend(rules.static_inventory_models.iter().cloned());
        ids.extend(rules.model_driver_overrides.keys().cloned());
    }
    let driver_id = match profile_id {
        "doubao" | "doubao-agent-plan" => "doubao",
        "bailian" => "qwen",
        other => other,
    };
    for driver in catalog.model_drivers() {
        if driver.model_driver_id == driver_id {
            ids.extend(driver.models.iter().map(|model| model.id.clone()));
        }
        if profile_id == "openrouter" {
            let vendor = match driver.model_driver_id.as_str() {
                "claude" => "anthropic",
                "gemini" => "google",
                "kimi" => "moonshotai",
                "glm" => "z-ai",
                "xai" => "x-ai",
                "mistral" => "mistralai",
                other => other,
            };
            ids.extend(
                driver
                    .models
                    .iter()
                    .map(|model| format!("{vendor}/{}", model.id)),
            );
        }
    }
    ids.into_iter().collect()
}

pub(super) fn bare_operations(operations: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    operations
        .iter()
        .map(|(api_type, binding)| {
            (
                api_type.clone(),
                binding.rsplit('/').next().unwrap_or(binding).to_owned(),
            )
        })
        .collect()
}

fn snapshot_profile(
    catalog: &CatalogSnapshot,
    registry: &BuiltinProviderRegistry,
    profile_id: &str,
    credential_kind: Option<CredentialKind>,
    entries: &mut BTreeMap<String, Value>,
) {
    let default_adapter = registry
        .profiles()
        .find(|profile| profile.provider_profile_id == profile_id)
        .unwrap()
        .default_protocol_adapter_id
        .clone();
    let binding = registry
        .resolve(BuiltinProviderRequest {
            provider_profile_id: profile_id,
            protocol_adapter_id: &default_adapter,
            auth_mode: ProviderAuthMode::ApiKey,
            credential_kind,
            configured_inventory: None,
        })
        .unwrap();
    let connection = binding
        .connection
        .resolve(ProviderConnectionInput::default())
        .unwrap();
    let configuration = catalog.resolve_provider_configuration(profile_id).unwrap();
    let mut instance_rules = binding.instance_rules.clone();
    if let Some(schema) = &binding.connection.policy_region {
        instance_rules
            .get_or_insert_with(Default::default)
            .policy_region = schema.resolve("policy_region", None).unwrap();
    }
    let instance = ProviderInstanceConfig {
        provider_instance_name: "snap".into(),
        provider_profile_id: profile_id.into(),
        protocol_adapter_id: default_adapter,
        base_url: connection.base_url,
        operation_base_urls: connection.operation_base_urls,
        credential: CredentialReference {
            reference: "secret://snapshot".into(),
        },
        credential_kind,
        provider_rules_id: Some(configuration.provider_rules_id.clone()),
        region: connection.region,
        workspace: connection.workspace,
        account: connection.account,
        request_timeout: Duration::from_secs(120),
        auto_sync_models: true,
        instance_rules,
    };
    let codecs = registry.codecs();
    let samples = |models: Vec<DiscoveredModel>| ProviderDiscoverySnapshot {
        revision: None,
        discovered_at_ms: 10,
        health: ProviderHealthState::Healthy,
        models,
    };
    let build = |discovery: ProviderDiscoverySnapshot| {
        InventoryBuilder::build_with_matcher(
            &binding.profile,
            &instance,
            discovery,
            catalog,
            &codecs,
            Some(binding.discovery.as_ref()),
        )
    };
    let discovery = match &binding.profile.default_inventory {
        Some(inventory) if binding.profile.discovery_mode == DiscoveryMode::CatalogOnly => {
            inventory.clone()
        }
        _ => samples(
            sample_model_ids(catalog, profile_id, &configuration.provider_rules_id)
                .iter()
                .map(|id| discovered(id))
                .filter(|model| build(samples(vec![model.clone()])).is_ok())
                .collect(),
        ),
    };
    let inventory = build(discovery).unwrap();
    let label = match credential_kind {
        Some(kind) => format!("{profile_id}#{}", kind.as_str()),
        None => profile_id.to_owned(),
    };
    for model in &inventory.models {
        entries.insert(
            format!("{label}|{}|inventory", model.provider_model_id),
            json!({
                "model_driver_id": model.model_driver_id,
                "origin_model_id": model.origin_model_id,
                "api_types": model.api_types,
                "capabilities": model.capabilities,
                "operations": bare_operations(&model.operations),
                "variants": model.variants.iter().map(|variant| &variant.name).collect::<Vec<_>>(),
                "variant_rules": model.variant_rules,
                "logical_mounts": model.logical_mounts,
                "server_default_routes": model.server_default_routes,
                "canonical_fields": model.canonical_fields,
                "pricing": model.pricing,
                "inventory_source": model.inventory_source,
            }),
        );
    }
    let filtered = inventory
        .filtered_capabilities
        .iter()
        .map(|item| {
            json!({
                "provider_model_id": item.provider_model_id,
                "api_type": item.api_type,
                "operation_id": item.operation_id,
                "reason": item.reason,
            })
        })
        .collect::<Vec<_>>();
    entries.insert(format!("{label}|*|filtered"), Value::Array(filtered));
    entries.insert(
        format!("{label}|*|unmatched"),
        json!(inventory
            .unmatched_models
            .iter()
            .map(|model| &model.provider_model_id)
            .collect::<Vec<_>>()),
    );
    entries.insert(
        format!("{label}|*|unavailable_presets"),
        json!(inventory
            .unavailable_presets
            .iter()
            .map(|preset| (&preset.provider_model_id, &preset.effort, &preset.reason))
            .collect::<Vec<_>>()),
    );
}

#[tokio::test]
async fn builtin_provider_inventory_snapshots_are_stable() {
    let catalog = catalog();
    let registry = builtin_provider_registry(&catalog).unwrap();
    let mut profiles = registry
        .profiles()
        .map(|profile| profile.provider_profile_id.clone())
        .filter(|id| id != CUSTOM_PROVIDER_PROFILE_ID)
        .collect::<Vec<_>>();
    profiles.sort();
    let mut entries = BTreeMap::new();
    for profile_id in &profiles {
        let mut kinds = vec![None];
        if profile_id == "glm" {
            kinds.push(Some(CredentialKind::GlmJwt));
        }
        for kind in kinds {
            snapshot_profile(&catalog, &registry, profile_id, kind, &mut entries);
        }
    }
    if let Ok(path) = std::env::var("AICC_INVENTORY_SNAPSHOT_DUMP") {
        let mut dump = String::new();
        for (key, value) in &entries {
            dump.push_str(key);
            dump.push('\n');
            dump.push_str(&serde_json::to_string_pretty(value).unwrap());
            dump.push_str("\n\n");
        }
        std::fs::write(path, dump).unwrap();
    }
    let actual = entries
        .iter()
        .map(|(key, value)| {
            format!(
                "{key}\t{:x}\n",
                Sha256::digest(serde_json::to_vec(value).unwrap())
            )
        })
        .collect::<String>();
    if std::env::var("AICC_INVENTORY_SNAPSHOT_UPDATE").is_ok_and(|value| value == "1") {
        std::fs::write(GOLDEN_FILE, &actual).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(GOLDEN_FILE).unwrap_or_default();
    let expected = golden
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect::<BTreeMap<_, _>>();
    let actual = actual
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect::<BTreeMap<_, _>>();
    let changed = actual
        .iter()
        .filter(|(key, digest)| expected.get(*key) != Some(*digest))
        .map(|(key, _)| format!("changed or added: {key}"))
        .chain(
            expected
                .keys()
                .filter(|key| !actual.contains_key(*key))
                .map(|key| format!("removed: {key}")),
        )
        .collect::<Vec<_>>();
    assert!(
        changed.is_empty(),
        "{} inventory snapshot entries differ (dump with AICC_INVENTORY_SNAPSHOT_DUMP=<file>):\n{}",
        changed.len(),
        changed
            .iter()
            .take(80)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// What each built-in discovery sends. The Provider's base path is kept and
/// only the origin points at a local server, which answers every list with
/// an empty one of the shape the request path implies.
async fn discovery_requests(
    catalog: &CatalogSnapshot,
    registry: &BuiltinProviderRegistry,
    profile_id: &str,
) -> Option<Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let default_adapter = registry
        .profiles()
        .find(|profile| profile.provider_profile_id == profile_id)?
        .default_protocol_adapter_id
        .clone();
    let binding = registry
        .resolve(BuiltinProviderRequest {
            provider_profile_id: profile_id,
            protocol_adapter_id: &default_adapter,
            auth_mode: ProviderAuthMode::ApiKey,
            credential_kind: None,
            configured_inventory: None,
        })
        .ok()?;
    if binding.profile.discovery_mode == DiscoveryMode::CatalogOnly {
        return None;
    }
    let connection = binding
        .connection
        .resolve(ProviderConnectionInput::default())
        .unwrap();
    let real = reqwest::Url::parse(&connection.base_url).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let recorded = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_recorded = recorded.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buffer = vec![0u8; 16 * 1024];
            let read = stream.read(&mut buffer).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]).to_string();
            let mut lines = head.split("\r\n");
            let request_line = lines.next().unwrap_or_default().to_owned();
            let mut headers = BTreeMap::new();
            for line in lines.take_while(|line| !line.is_empty()) {
                if let Some((name, value)) = line.split_once(':') {
                    let name = name.trim().to_ascii_lowercase();
                    if matches!(name.as_str(), "host" | "content-length" | "accept-encoding") {
                        continue;
                    }
                    let value = value.trim();
                    let value = if name == "x-request-id" {
                        "<id>".to_owned()
                    } else if value.contains("snapshot-secret") {
                        value.replace("snapshot-secret", "***")
                    } else {
                        value.to_owned()
                    };
                    headers.insert(name, value);
                }
            }
            let path = request_line
                .split(' ')
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let body = if path.contains("/videos/models") {
                r#"{"data":[]}"#
            } else if headers.contains_key("x-goog-api-key") {
                r#"{"models":[]}"#
            } else if headers.contains_key("anthropic-version") || headers.contains_key("x-api-key")
            {
                r#"{"data":[],"has_more":false,"first_id":null,"last_id":null}"#
            } else {
                r#"{"object":"list","data":[],"models":[]}"#
            };
            server_recorded.lock().await.push(json!({
                "request": request_line,
                "headers": headers,
            }));
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    let mut instance_rules = binding.instance_rules.clone();
    if let Some(schema) = &binding.connection.policy_region {
        instance_rules
            .get_or_insert_with(Default::default)
            .policy_region = schema.resolve("policy_region", None).unwrap();
    }
    let instance = ProviderInstanceConfig {
        provider_instance_name: "snap".into(),
        provider_profile_id: profile_id.into(),
        protocol_adapter_id: default_adapter,
        base_url: format!("http://{address}{}", real.path()),
        operation_base_urls: BTreeMap::new(),
        credential: CredentialReference {
            reference: "secret://snapshot".into(),
        },
        credential_kind: None,
        provider_rules_id: Some(
            catalog
                .resolve_provider_configuration(profile_id)
                .unwrap()
                .provider_rules_id,
        ),
        region: connection.region,
        workspace: connection.workspace,
        account: connection.account,
        request_timeout: Duration::from_secs(10),
        auto_sync_models: true,
        instance_rules,
    };
    let credential = StaticCredentialResolver::new(BTreeMap::from([(
        "secret://snapshot".to_owned(),
        "snapshot-secret".to_owned(),
    )]))
    .resolve(&binding.profile.credential, &instance.credential)
    .await
    .unwrap();
    let outcome = binding
        .discovery
        .discover(&DiscoveryContext {
            profile: &binding.profile,
            instance: &instance,
            credential: &credential,
        })
        .await;
    server.abort();
    let requests = recorded.lock().await.clone();
    Some(json!({
        "requests": requests,
        "outcome": match outcome {
            Ok(snapshot) => json!({"models": snapshot.models.len()}),
            Err(error) => json!({"error": error.to_string()}),
        },
    }))
}

#[tokio::test]
async fn builtin_provider_discovery_requests_are_stable() {
    let catalog = catalog();
    let registry = builtin_provider_registry(&catalog).unwrap();
    let mut profiles = registry
        .profiles()
        .map(|profile| profile.provider_profile_id.clone())
        .filter(|id| id != CUSTOM_PROVIDER_PROFILE_ID)
        .collect::<Vec<_>>();
    profiles.sort();
    let mut entries = BTreeMap::new();
    for profile_id in &profiles {
        if let Some(value) = discovery_requests(&catalog, &registry, profile_id).await {
            entries.insert(format!("{profile_id}|*|discovery"), value);
        }
    }
    let rendered = serde_json::to_string_pretty(&entries).unwrap();
    if let Ok(path) = std::env::var("AICC_DISCOVERY_SNAPSHOT_DUMP") {
        std::fs::write(path, &rendered).unwrap();
    }
    let golden_file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/provider/builtin/discovery_snapshots.json"
    );
    if std::env::var("AICC_DISCOVERY_SNAPSHOT_UPDATE").is_ok_and(|value| value == "1") {
        std::fs::write(golden_file, format!("{rendered}\n")).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(golden_file).unwrap_or_default();
    assert_eq!(rendered.trim(), golden.trim(), "discovery requests changed");
}
