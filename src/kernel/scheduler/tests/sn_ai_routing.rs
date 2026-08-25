#![allow(dead_code, unused_imports, unused_must_use, unused_variables)]

#[path = "../src/system_config_builder.rs"]
mod system_config_builder;

use anyhow::anyhow;
use serde_json::{json, Value};
use system_config_builder::{derive_sn_ai_provider_endpoints, reconcile_managed_sn_ai_provider};

fn managed_settings(enabled: bool, base_url: &str) -> Value {
    let login_url = format!(
        "{}/api/user/login_by_device_token",
        base_url.trim_end_matches("/api/v1/ai/")
    );
    json!({
        "sn-ai-provider": {
            "enabled": enabled,
            "instances": [
                {
                    "id": "custom-provider",
                    "provider_driver": "openai",
                    "base_url": "https://custom.example/v1/"
                },
                {
                    "id": "system-sn-provider",
                    "provider_driver": "sn-ai-provider",
                    "base_url": base_url,
                    "login_url": login_url,
                    "user_name": "alice"
                }
            ]
        },
        "unrelated": {"preserved": true}
    })
}

#[test]
fn task009_derives_endpoints_from_bare_host_and_https_origin() {
    let bare = derive_sn_ai_provider_endpoints(Some(" sn.buckyos.io ")).unwrap();
    assert_eq!(
        bare.login_url,
        "https://sn.buckyos.io/api/user/login_by_device_token"
    );
    assert_eq!(bare.responses_url, "https://sn.buckyos.io/api/v1/ai/");

    let origin = derive_sn_ai_provider_endpoints(Some("https://sn.example:8443/")).unwrap();
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
fn task009_patches_only_the_managed_sn_instance() {
    let current = managed_settings(true, "https://sn.buckyos.ai/api/v1/ai/");
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    let next = reconcile_managed_sn_ai_provider(&current, Ok(&endpoints), Some("alice"))
        .unwrap()
        .expect("managed URL should change");

    let instances = next["sn-ai-provider"]["instances"].as_array().unwrap();
    assert_eq!(instances[0]["base_url"], "https://custom.example/v1/");
    assert_eq!(instances[1]["base_url"], "https://sn.buckyos.io/api/v1/ai/");
    assert_eq!(
        instances[1]["login_url"],
        "https://sn.buckyos.io/api/user/login_by_device_token"
    );
    assert_eq!(next["unrelated"], current["unrelated"]);
}

#[test]
fn task009_invalid_zone_disables_managed_provider_without_rewriting_its_url() {
    let current = managed_settings(true, "https://sn.buckyos.ai/api/v1/ai/");
    let invalid = anyhow!("invalid ZoneDocument.sn");
    let next = reconcile_managed_sn_ai_provider(&current, Err(&invalid), None)
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
    let next = reconcile_managed_sn_ai_provider(&current, Ok(&endpoints), Some("alice"))
        .unwrap()
        .expect("managed provider should be enabled");

    assert_eq!(next["sn-ai-provider"]["enabled"], true);
}

#[test]
fn task009_reconciliation_adds_missing_managed_provider() {
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    for current in [
        json!({}),
        json!({"sn-ai-provider": {"enabled": false}}),
        json!({
            "sn-ai-provider": {
                "enabled": true,
                "instances": [{
                    "provider_driver": "openai",
                    "base_url": "https://custom.example/v1/"
                }]
            }
        }),
    ] {
        let next = reconcile_managed_sn_ai_provider(&current, Ok(&endpoints), Some("alice"))
            .unwrap()
            .expect("managed provider should be added");
        let instances = next["sn-ai-provider"]["instances"].as_array().unwrap();
        let managed = instances
            .iter()
            .find(|instance| instance["provider_driver"] == "sn-ai-provider")
            .expect("managed instance");
        assert_eq!(managed["provider_instance_name"], "sn-ai-provider-default");
        assert_eq!(managed["base_url"], "https://sn.buckyos.io/api/v1/ai/");
        assert_eq!(
            managed["login_url"],
            "https://sn.buckyos.io/api/user/login_by_device_token"
        );
        assert_eq!(managed["user_name"], "alice");
        assert_eq!(next["sn-ai-provider"]["enabled"], true);
    }
}

#[test]
fn task009_reconciliation_does_not_add_without_relay_or_user() {
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    let invalid = anyhow!("ZoneDocument.sn is missing");
    assert!(
        reconcile_managed_sn_ai_provider(&json!({}), Err(&invalid), Some("alice"))
            .unwrap()
            .is_none()
    );
    assert!(
        reconcile_managed_sn_ai_provider(&json!({}), Ok(&endpoints), None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn task009_reconciliation_is_noop_when_managed_instance_is_current() {
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    let current = managed_settings(true, "https://sn.buckyos.io/api/v1/ai/");
    assert!(
        reconcile_managed_sn_ai_provider(&current, Ok(&endpoints), Some("alice"))
            .unwrap()
            .is_none()
    );
}
