mod common;

use aicc::{
    CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry, RouteConfig,
};
use async_trait::async_trait;
use buckyos_api::{AiccServerHandler, Capability};
use common::*;
use kRPC::{RPCErrors, RPCHandler, RPCRequest, RPCResponse, RPCResult};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

async fn token_for_remote_target(target: &RpcTestEndpoint) -> Option<String> {
    if target.is_remote {
        resolve_remote_test_token(Some(&target.endpoint))
            .await
            .expect("resolve remote test token")
    } else {
        None
    }
}

fn add_llm(
    registry: &Registry,
    catalog: &ModelCatalog,
    id: &str,
    ptype: &str,
    cost: f64,
    lat: u64,
    r: std::result::Result<ProviderStartResult, ProviderError>,
) -> Arc<MockProvider> {
    catalog.set_mapping(Capability::LlmRouter, "llm.plan.default", ptype, "m");
    let p = Arc::new(MockProvider::new(
        mock_instance(id, ptype, vec![Capability::LlmRouter], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(cost),
            estimated_latency_ms: Some(lat),
        },
        vec![r],
    ));
    registry.add_provider(p.clone());
    p
}

#[derive(Default)]
struct MockSystemConfigHandler {
    inner: Mutex<HashMap<String, (String, u64)>>,
}

impl MockSystemConfigHandler {
    fn ensure_write_permission(token: Option<&str>) -> std::result::Result<(), RPCErrors> {
        if token == Some("admin-token") {
            Ok(())
        } else {
            Err(RPCErrors::ReasonError(
                "NoPermission: write requires admin-token".to_string(),
            ))
        }
    }

    fn parse_json_string(value: &str) -> std::result::Result<Value, RPCErrors> {
        serde_json::from_str::<Value>(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "value must be valid JSON string, parse failed: {}",
                e
            ))
        })
    }

    fn set_value_by_json_path(
        root: &mut Value,
        json_path: &str,
        patch_value: Value,
    ) -> std::result::Result<(), RPCErrors> {
        if !json_path.starts_with('/') {
            return Err(RPCErrors::ParseRequestError(
                "json_path must start with '/'".to_string(),
            ));
        }
        let tokens: Vec<&str> = json_path.split('/').skip(1).collect();
        if tokens.is_empty() {
            *root = patch_value;
            return Ok(());
        }

        let mut cursor = root;
        for key in &tokens[..tokens.len() - 1] {
            if !cursor.is_object() {
                *cursor = json!({});
            }
            let map = cursor.as_object_mut().expect("cursor must be object");
            cursor = map.entry((*key).to_string()).or_insert_with(|| json!({}));
        }
        if !cursor.is_object() {
            *cursor = json!({});
        }
        let last = tokens[tokens.len() - 1].to_string();
        cursor
            .as_object_mut()
            .expect("cursor must be object")
            .insert(last, patch_value);
        Ok(())
    }
}

#[async_trait]
impl RPCHandler for MockSystemConfigHandler {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: std::net::IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let token = req.token.as_deref();

        let result = match req.method.as_str() {
            "sys_config_get" => {
                let key = req
                    .params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RPCErrors::ParseRequestError("missing key".to_string()))?;
                let store = self.inner.lock().expect("mock sys-config lock");
                if let Some((value, version)) = store.get(key) {
                    RPCResult::Success(json!({
                        "key": key,
                        "value": value,
                        "version": *version
                    }))
                } else {
                    RPCResult::Success(Value::Null)
                }
            }
            "sys_config_set" => {
                Self::ensure_write_permission(token)?;
                let key = req
                    .params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RPCErrors::ParseRequestError("missing key".to_string()))?;
                let value = req
                    .params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RPCErrors::ParseRequestError("missing value".to_string()))?;
                let _ = Self::parse_json_string(value)?;

                let mut store = self.inner.lock().expect("mock sys-config lock");
                let next_version = store.get(key).map(|(_, ver)| ver + 1).unwrap_or(1);
                store.insert(key.to_string(), (value.to_string(), next_version));
                RPCResult::Success(json!({
                    "ok": true,
                    "version": next_version
                }))
            }
            "sys_config_set_by_json_path" => {
                Self::ensure_write_permission(token)?;
                let key = req
                    .params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RPCErrors::ParseRequestError("missing key".to_string()))?;
                let json_path = req
                    .params
                    .get("json_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RPCErrors::ParseRequestError("missing json_path".to_string()))?;
                let value = req
                    .params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RPCErrors::ParseRequestError("missing value".to_string()))?;
                let patch_value = Self::parse_json_string(value)?;

                let mut store = self.inner.lock().expect("mock sys-config lock");
                let (current_value, current_ver) = store
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| ("{}".to_string(), 0));
                let mut root = Self::parse_json_string(&current_value)?;
                Self::set_value_by_json_path(&mut root, json_path, patch_value)?;
                let next_ver = current_ver + 1;
                store.insert(key.to_string(), (root.to_string(), next_ver));
                RPCResult::Success(json!({
                    "ok": true,
                    "version": next_ver
                }))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method)),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

async fn resolve_sys_config_test_endpoint() -> RpcTestEndpoint {
    if let Some(endpoint) = resolve_gateway_system_config_endpoint_from_env() {
        return RpcTestEndpoint::from_remote(endpoint);
    }

    let server = spawn_rpc_http_server(Arc::new(MockSystemConfigHandler::default())).await;
    let mut endpoint = RpcTestEndpoint::from_local(server);
    endpoint.endpoint = endpoint.endpoint.replace("/kapi/aicc", "/kapi/system_config");
    endpoint
}

async fn post_sys_config_rpc(
    endpoint: &str,
    method: &str,
    params: Value,
    seq: u64,
    token: Option<&str>,
) -> std::result::Result<RPCResponse, String> {
    let is_remote_endpoint = reqwest::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_string()))
        .map(|host| host != "127.0.0.1" && host != "localhost")
        .unwrap_or(false);

    if is_remote_endpoint {
        let mut sys = vec![json!(seq)];
        if token.is_some() {
            sys.push(json!(token.unwrap()));
        }
        let body = json!({
            "method": method,
            "params": params,
            "sys": sys,
        });
        let client = reqwest::Client::new();
        let resp = client
            .post(endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("http request failed: {}", e))?;
        let status = resp.status();
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("decode response json failed: {}", e))?;
        if !status.is_success() {
            return Err(format!("http status {} body {}", status, value));
        }
        if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
            return Err(err.to_string());
        }
        let result = value
            .get("result")
            .cloned()
            .ok_or_else(|| format!("rpc result missing: {}", value))?;
        return Ok(RPCResponse {
            result: RPCResult::Success(result),
            seq,
            trace_id: None,
        });
    }

    post_rpc_over_http(
        endpoint,
        &RPCRequest {
            method: method.to_string(),
            params,
            seq,
            token: token.map(|v| v.to_string()),
            trace_id: Some(format!("cfg-seq-{}", seq)),
        },
    )
    .await
}

async fn sys_config_admin_token(target: &RpcTestEndpoint) -> String {
    if let Ok(token) = std::env::var("AICC_SYS_CONFIG_RPC_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return token;
        }
    }

    if target.is_remote {
        if let Some(token) = resolve_remote_test_token(Some(&target.endpoint))
            .await
            .expect("resolve remote sys-config token")
        {
            return token;
        }
    }

    std::env::var("AICC_RPC_TOKEN").unwrap_or_else(|_| "admin-token".to_string())
}

fn sys_config_test_key(suffix: &str) -> String {
    format!("services/aicc/test_settings/{}", suffix)
}

#[tokio::test]
async fn krpc_01_gateway_complete_minimal_llm_success() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let h = Arc::new(AiccServerHandler::new(center_with_taskmgr(r, c)));
    let target = resolve_rpc_gateway_test_endpoint(h).await;
    let remote_token = token_for_remote_target(&target).await;
    let resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 1,
            token: remote_token.clone(),
            trace_id: None,
        },
    )
        .await
        .unwrap();
    assert_eq!(resp.seq, 1);
    assert!(resp.trace_id.is_none());
    let payload = match resp.result {
        RPCResult::Success(v) => v,
        other => panic!("unexpected rpc result: {:?}", other),
    };
    let task_id = payload
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let status = payload
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(!task_id.is_empty(), "missing task_id: {payload}");
    if target.is_remote {
        assert!(
            matches!(status, "running" | "succeeded" | "failed"),
            "unexpected status: {payload}"
        );
    } else {
        assert!(
            matches!(status, "running" | "succeeded"),
            "unexpected status: {payload}"
        );
    }
}

#[tokio::test]
async fn krpc_02_gateway_complete_with_sys_seq_token_trace_success() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let h = Arc::new(AiccServerHandler::new(center_with_taskmgr(r, c)));
    let target = resolve_rpc_gateway_test_endpoint(h).await;
    let remote_token = token_for_remote_target(&target).await;
    let resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 2,
            token: remote_token.or_else(|| Some("tenant-a".into())),
            trace_id: Some("trace".into()),
        },
    )
        .await
        .unwrap();
    assert_eq!(resp.seq, 2);
    assert_eq!(resp.trace_id.as_deref(), Some("trace"));
    let payload = match resp.result {
        RPCResult::Success(v) => v,
        other => panic!("unexpected rpc result: {:?}", other),
    };
    assert!(
        payload.get("task_id").and_then(|v| v.as_str()).is_some(),
        "missing task_id: {payload}"
    );
}

#[tokio::test]
async fn krpc_03_gateway_complete_without_token_with_trace_uses_null_placeholder() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let h = Arc::new(AiccServerHandler::new(center_with_taskmgr(r, c)));
    let target = resolve_rpc_gateway_test_endpoint(h).await;
    let remote_token = token_for_remote_target(&target).await;
    let resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 3,
            token: remote_token,
            trace_id: Some("trace".into()),
        },
    )
        .await
        .unwrap();
    assert_eq!(resp.seq, 3);
    assert_eq!(resp.trace_id.as_deref(), Some("trace"));
    let payload = match resp.result {
        RPCResult::Success(v) => v,
        other => panic!("unexpected rpc result: {:?}", other),
    };
    assert!(
        payload.get("task_id").and_then(|v| v.as_str()).is_some(),
        "missing task_id: {payload}"
    );
}

#[tokio::test]
async fn krpc_04_gateway_complete_invalid_sys_shape_returns_bad_request() {
    let h = Arc::new(AiccServerHandler::new(center_with_taskmgr(
        Registry::default(),
        ModelCatalog::default(),
    )));
    let target = resolve_rpc_gateway_test_endpoint(h).await;
    let remote_token = token_for_remote_target(&target).await;
    let err = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: json!({"bad":"payload"}),
            seq: 4,
            token: remote_token,
            trace_id: None,
        },
    )
    .await
    .unwrap_err();
    let err_lower = err.to_ascii_lowercase();
    assert!(
        err_lower.contains("failed to parse completerequest")
            || err_lower.contains("parse request"),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn krpc_05_gateway_cancel_cross_tenant_rejected() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let h = Arc::new(AiccServerHandler::new(center_with_taskmgr(r, c)));
    let target = resolve_rpc_gateway_test_endpoint(h).await;
    let remote_token = token_for_remote_target(&target).await;
    let start_token = remote_token.clone().or_else(|| Some("ta".into()));
    let cross_tenant_token = if target.is_remote {
        Some("cross-tenant-test-invalid-token".into())
    } else {
        Some("tb".into())
    };
    let start = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 5,
            token: start_token,
            trace_id: None,
        },
    )
        .await
        .unwrap();
    let tid = match start.result {
        RPCResult::Success(v) => v
            .get("task_id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    let cancel_result = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "cancel".into(),
            params: json!({"task_id":tid}),
            seq: 6,
            token: cross_tenant_token,
            trace_id: None,
        },
    )
    .await
    ;
    match cancel_result {
        Ok(resp) => {
            let payload = match resp.result {
                RPCResult::Success(v) => v,
                other => panic!("unexpected rpc result: {:?}", other),
            };
            assert_eq!(payload.get("task_id").and_then(|x| x.as_str()), Some(tid.as_str()));
            assert_eq!(payload.get("accepted").and_then(|x| x.as_bool()), Some(false));
        }
        Err(err) => {
            let err_lower = err.to_ascii_lowercase();
            assert!(
                err_lower.contains("permission") || err_lower.contains("tenant"),
                "unexpected error: {}",
                err
            );
        }
    }
}

#[tokio::test]
async fn krpc_06_gateway_cancel_same_tenant_accepted_or_graceful_false() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let h = Arc::new(AiccServerHandler::new(center_with_taskmgr(r, c)));
    let target = resolve_rpc_gateway_test_endpoint(h).await;
    let remote_token = token_for_remote_target(&target).await;
    let same_tenant_token = remote_token.clone().or_else(|| Some("ta".into()));
    let start = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 7,
            token: same_tenant_token.clone(),
            trace_id: None,
        },
    )
        .await
        .unwrap();
    let tid = match start.result {
        RPCResult::Success(v) => v
            .get("task_id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    assert!(!tid.is_empty(), "complete should return task_id before cancel");
    let cancel = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "cancel".into(),
            params: json!({"task_id":tid}),
            seq: 8,
            token: same_tenant_token,
            trace_id: None,
        },
    )
        .await
        .unwrap();
    assert_eq!(cancel.seq, 8);
    match cancel.result {
        RPCResult::Success(v) => {
            assert_eq!(v.get("task_id").and_then(|x| x.as_str()), Some(tid.as_str()));
            if target.is_remote {
                assert!(
                    v.get("accepted").and_then(|x| x.as_bool()).is_some(),
                    "unexpected payload: {}",
                    v
                );
            } else {
                assert_eq!(v.get("accepted").and_then(|x| x.as_bool()), Some(true));
            }
        }
        _ => panic!("unexpected rpc failure"),
    }
}

#[tokio::test]
async fn cfg_01_sys_config_get_aicc_settings_success() {
    let target = resolve_sys_config_test_endpoint().await;
    let admin_token = sys_config_admin_token(&target).await;
    let key = sys_config_test_key("cfg_01");
    let value = json!({
        "fallback_limit": RouteConfig::default().fallback_limit
    })
    .to_string();
    let set_resp = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_set",
        json!({"key": key, "value": value}),
        1001,
        Some(admin_token.as_str()),
    )
    .await
    .expect("sys_config_set should succeed");
    assert_eq!(set_resp.seq, 1001);

    let get_resp = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_get",
        json!({"key": key}),
        1002,
        Some(admin_token.as_str()),
    )
    .await
    .expect("sys_config_get should succeed");
    assert_eq!(get_resp.seq, 1002);

    let payload = match get_resp.result {
        RPCResult::Success(v) => v,
        other => panic!("unexpected rpc result: {:?}", other),
    };
    if !target.is_remote {
        assert_eq!(payload.get("key").and_then(|v| v.as_str()), Some(key.as_str()));
    }
    assert!(
        payload
            .get("version")
            .and_then(|v| v.as_u64())
            .map(|v| v >= 1)
            .unwrap_or(false),
        "missing version in payload: {}",
        payload
    );
    let raw = payload
        .get("value")
        .and_then(|v| v.as_str())
        .expect("sys_config_get should return string value");
    let parsed: Value = serde_json::from_str(raw).expect("route config json");
    assert_eq!(
        parsed["fallback_limit"].as_u64(),
        Some(RouteConfig::default().fallback_limit as u64)
    );
}

#[tokio::test]
async fn cfg_02_sys_config_set_full_value_effective() {
    let target = resolve_sys_config_test_endpoint().await;
    let admin_token = sys_config_admin_token(&target).await;
    let key = sys_config_test_key("cfg_02");
    let new_cfg = json!({
        "fallback_limit": 7,
        "weights": {
            "cost": 0.4,
            "latency": 0.3,
            "error": 0.2,
            "load": 0.1
        }
    });
    let value = new_cfg.to_string();
    let set_resp = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_set",
        json!({"key": key, "value": value}),
        1003,
        Some(admin_token.as_str()),
    )
    .await
    .expect("sys_config_set full value should succeed");
    assert_eq!(set_resp.seq, 1003);

    let get_resp = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_get",
        json!({"key": key}),
        1004,
        Some(admin_token.as_str()),
    )
    .await
    .expect("sys_config_get should succeed");
    let payload = match get_resp.result {
        RPCResult::Success(v) => v,
        other => panic!("unexpected rpc result: {:?}", other),
    };
    let persisted: Value = serde_json::from_str(
        payload
            .get("value")
            .and_then(|v| v.as_str())
            .expect("value should be a json string"),
    )
    .expect("persisted value should be valid json");
    assert_eq!(persisted["fallback_limit"], 7);
    assert_eq!(persisted["weights"]["cost"], 0.4);
}

#[tokio::test]
async fn cfg_03_sys_config_set_by_json_path_partial_update_effective() {
    let target = resolve_sys_config_test_endpoint().await;
    let admin_token = sys_config_admin_token(&target).await;
    let key = sys_config_test_key("cfg_03");
    let init_cfg = json!({
        "fallback_limit": 2,
        "weights": {
            "cost": 0.2,
            "latency": 0.3
        }
    });
    post_sys_config_rpc(
        &target.endpoint,
        "sys_config_set",
        json!({"key": key, "value": init_cfg.to_string()}),
        1005,
        Some(admin_token.as_str()),
    )
    .await
    .expect("prepare initial value");

    let patch_resp = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_set_by_json_path",
        json!({
            "key": key,
            "json_path": "/weights/cost",
            "value": "0.9"
        }),
        1006,
        Some(admin_token.as_str()),
    )
    .await
    .expect("set_by_json_path should succeed");
    assert_eq!(patch_resp.seq, 1006);

    let get_resp = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_get",
        json!({"key": key}),
        1007,
        Some(admin_token.as_str()),
    )
    .await
    .expect("sys_config_get should succeed");
    let payload = match get_resp.result {
        RPCResult::Success(v) => v,
        other => panic!("unexpected rpc result: {:?}", other),
    };
    let persisted: Value = serde_json::from_str(
        payload
            .get("value")
            .and_then(|v| v.as_str())
            .expect("value should be a json string"),
    )
    .expect("persisted value should be valid json");
    assert_eq!(persisted["weights"]["cost"], 0.9);
    assert_eq!(persisted["weights"]["latency"], 0.3);
    assert_eq!(persisted["fallback_limit"], 2);
}

#[tokio::test]
async fn cfg_04_sys_config_write_without_permission_rejected() {
    let target = resolve_sys_config_test_endpoint().await;
    let key = sys_config_test_key("cfg_04");
    let err = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_set",
        json!({
            "key": key,
            "value": json!({"fallback_limit": 5}).to_string()
        }),
        1008,
        Some("tenant-a"),
    )
    .await
    .expect_err("write without permission should fail");
    let err_lower = err.to_ascii_lowercase();
    assert!(
        err_lower.contains("permission") || err_lower.contains("jwt"),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn cfg_05_sys_config_value_not_json_string_rejected() {
    let target = resolve_sys_config_test_endpoint().await;
    let admin_token = sys_config_admin_token(&target).await;
    let key = sys_config_test_key("cfg_05");
    let set_result = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_set",
        json!({
            "key": key,
            "value": "not-json"
        }),
        1009,
        Some(admin_token.as_str()),
    )
    .await;

    if target.is_remote {
        let resp = set_result.expect("remote sys_config_set should accept plain string");
        assert_eq!(resp.seq, 1009);
    } else {
        let err = set_result.expect_err("invalid json string value should fail");
        let err_lower = err.to_ascii_lowercase();
        assert!(
            err_lower.contains("json") || err_lower.contains("parse"),
            "unexpected error: {}",
            err
        );
    }
}

