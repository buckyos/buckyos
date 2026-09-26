#![allow(dead_code, unused_imports, unused_must_use, unused_variables)]

#[path = "../src/system_config_builder.rs"]
mod system_config_builder;

use anyhow::anyhow;
use serde_json::{json, Value};
use system_config_builder::{derive_sn_ai_provider_endpoints, reconcile_managed_sn_ai_provider};

fn custom_provider() -> Value {
    json!({
        "provider_instance_name": "custom-provider",
        "provider_type": "cloud_api",
        "provider_profile_id": "openai",
        "protocol_adapter_id": "openai-responses",
        "provider_rules_id": "openai",
        "base_url": "https://custom.example/v1/",
        "enabled": true
    })
}

fn managed_provider(enabled: bool, base_url: &str) -> Value {
    let login_url = format!(
        "{}/api/user/login_by_device_token",
        base_url.trim_end_matches("/api/v1/ai/")
    );
    json!({
        "provider_instance_name": "sn-ai-provider-default",
        "provider_type": "cloud_api",
        "provider_profile_id": "sn",
        "protocol_adapter_id": "sn-openai",
        "provider_rules_id": "sn",
        "base_url": base_url,
        "credentials": {
            "device_token_ref": "runtime://device-jwt"
        },
        "auth": {
            "mode": "dynamic_login",
            "login_profile": "device_jwt",
            "login_endpoint": login_url
        },
        "account": "alice",
        "enabled": enabled
    })
}

fn managed_settings(enabled: bool, base_url: &str) -> Value {
    json!({
        "providers": [custom_provider(), managed_provider(enabled, base_url)]
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

    let providers = next["providers"].as_array().unwrap();
    assert_eq!(providers[0]["base_url"], "https://custom.example/v1/");
    assert_eq!(providers[1]["base_url"], "https://sn.buckyos.io/api/v1/ai/");
    assert_eq!(
        providers[1]["auth"]["login_endpoint"],
        "https://sn.buckyos.io/api/user/login_by_device_token"
    );
}

#[test]
fn task009_invalid_zone_preserves_activated_provider_for_retry() {
    let current = managed_settings(true, "https://sn.buckyos.ai/api/v1/ai/");
    let invalid = anyhow!("invalid ZoneDocument.sn");
    let next = reconcile_managed_sn_ai_provider(&current, Err(&invalid), None).unwrap();

    assert!(next.is_none());
}

#[test]
fn task009_valid_zone_preserves_explicitly_disabled_provider() {
    let current = managed_settings(false, "https://sn.buckyos.io/api/v1/ai/");
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    let next = reconcile_managed_sn_ai_provider(&current, Ok(&endpoints), Some("alice")).unwrap();

    assert!(next.is_none());
}

#[test]
fn task009_reconciliation_does_not_recreate_a_removed_managed_provider() {
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    for current in [
        json!({"providers": []}),
        json!({"providers": [custom_provider()]}),
    ] {
        let next =
            reconcile_managed_sn_ai_provider(&current, Ok(&endpoints), Some("alice")).unwrap();
        assert!(next.is_none());
    }
}

#[test]
fn task009_reconciliation_does_not_add_without_relay_or_user() {
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.io")).unwrap();
    let invalid = anyhow!("ZoneDocument.sn is missing");
    let activated = managed_settings(true, "https://sn.buckyos.ai/api/v1/ai/");
    assert!(
        reconcile_managed_sn_ai_provider(&activated, Err(&invalid), Some("alice"))
            .unwrap()
            .is_none()
    );
    assert!(
        reconcile_managed_sn_ai_provider(&activated, Ok(&endpoints), None)
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
