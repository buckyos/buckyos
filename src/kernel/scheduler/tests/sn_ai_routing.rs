#![allow(dead_code, unused_imports, unused_must_use, unused_variables)]

#[path = "../src/system_config_builder.rs"]
mod system_config_builder;

use anyhow::anyhow;
use serde_json::{json, Value};
use system_config_builder::{derive_sn_ai_provider_endpoints, reconcile_managed_sn_ai_provider};

fn managed_settings(enabled: bool, base_url: &str) -> Value {
    json!({
        "sn-ai-provider": {
            "enabled": enabled,
            "instances": [
                {
                    "id": "custom-provider",
                    "provider_driver": "openai",
                    "auth_mode": "api_key",
                    "base_url": "https://custom.example/v1/"
                },
                {
                    "id": "system-sn-provider",
                    "provider_driver": "sn-ai-provider",
                    "auth_mode": "runtime_session",
                    "base_url": base_url,
                    "models": ["gpt-5.4-mini"]
                }
            ]
        },
        "unrelated": {"preserved": true}
    })
}

#[test]
fn task009_derives_endpoints_from_bare_host_and_https_origin() {
    let bare = derive_sn_ai_provider_endpoints(Some(" sn.buckyos.io ")).unwrap();
    assert_eq!(bare.models_url, "https://sn.buckyos.io/api/v1/ai/models");
    assert_eq!(bare.responses_url, "https://sn.buckyos.io/api/v1/ai/");

    let origin = derive_sn_ai_provider_endpoints(Some("https://sn.example:8443/")).unwrap();
    assert_eq!(
        origin.models_url,
        "https://sn.example:8443/api/v1/ai/models"
    );
    assert_eq!(origin.responses_url, "https://sn.example:8443/api/v1/ai/");
}

#[test]
fn task009_rejects_missing_or_unsafe_zone_sn_values() {
    for value in [
        None,
        Some(""),
        Some("   "),
        Some("http://sn.buckyos.io"),
        Some("https://user@sn.buckyos.io"),
        Some("https://sn.buckyos.io/path"),
        Some("https://sn.buckyos.io?region=us"),
        Some("https://sn.buckyos.io#fragment"),
    ] {
        assert!(
            derive_sn_ai_provider_endpoints(value).is_err(),
            "expected rejection for {value:?}"
        );
    }
}

#[test]
fn task009_patches_only_the_managed_runtime_session_instance() {
    let current = managed_settings(true, "https://sn.buckyos.ai/api/v1/ai/");
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    let next = reconcile_managed_sn_ai_provider(&current, Ok(&endpoints))
        .unwrap()
        .expect("managed URL should change");

    let instances = next["sn-ai-provider"]["instances"].as_array().unwrap();
    assert_eq!(instances[0]["base_url"], "https://custom.example/v1/");
    assert_eq!(instances[1]["base_url"], "https://sn.buckyos.io/api/v1/ai/");
    assert_eq!(instances[1]["models"], json!(["gpt-5.4-mini"]));
    assert_eq!(next["unrelated"], current["unrelated"]);
}

#[test]
fn task009_invalid_zone_disables_managed_provider_without_rewriting_its_url() {
    let current = managed_settings(true, "https://sn.buckyos.ai/api/v1/ai/");
    let invalid = anyhow!("invalid ZoneDocument.sn");
    let next = reconcile_managed_sn_ai_provider(&current, Err(&invalid))
        .unwrap()
        .expect("managed provider should be disabled");

    assert_eq!(next["sn-ai-provider"]["enabled"], false);
    assert_eq!(
        next["sn-ai-provider"]["instances"][1]["base_url"],
        "https://sn.buckyos.ai/api/v1/ai/"
    );
}

#[test]
fn task009_valid_zone_reenables_a_disabled_managed_provider() {
    let current = managed_settings(false, "https://sn.buckyos.io/api/v1/ai/");
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    let next = reconcile_managed_sn_ai_provider(&current, Ok(&endpoints))
        .unwrap()
        .expect("managed provider should be enabled");

    assert_eq!(next["sn-ai-provider"]["enabled"], true);
}

#[test]
fn task009_reconciliation_is_noop_without_a_managed_instance_or_change() {
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    let current = managed_settings(true, "https://sn.buckyos.io/api/v1/ai/");
    assert!(reconcile_managed_sn_ai_provider(&current, Ok(&endpoints))
        .unwrap()
        .is_none());

    for value in [
        json!({}),
        json!({"sn-ai-provider": {"enabled": true}}),
        json!({
            "sn-ai-provider": {
                "enabled": true,
                "instances": [{
                    "provider_driver": "openai",
                    "auth_mode": "api_key"
                }]
            }
        }),
    ] {
        assert!(reconcile_managed_sn_ai_provider(&value, Ok(&endpoints))
            .unwrap()
            .is_none());
    }
}
