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
    if let Ok(endpoint) = std::env::var("AICC_GATEWAY_SYS_CONFIG_TEST_ENDPOINT") {
        let endpoint = endpoint.trim().to_string();
        if !endpoint.is_empty() {
            return RpcTestEndpoint::from_remote(endpoint);
        }
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

fn sys_config_admin_token() -> String {
    std::env::var("AICC_GATEWAY_SYS_CONFIG_TEST_TOKEN")
        .or_else(|_| std::env::var("AICC_RPC_TOKEN"))
        .unwrap_or_else(|_| "admin-token".to_string())
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
    let resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 1,
            token: None,
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
    assert!(
        matches!(status, "running" | "succeeded"),
        "unexpected status: {payload}"
    );
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
    let resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 2,
            token: Some("tenant-a".into()),
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
    let resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 3,
            token: None,
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
    let err = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: json!({"bad":"payload"}),
            seq: 4,
            token: None,
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
    let start = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 5,
            token: Some("ta".into()),
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
    let err = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "cancel".into(),
            params: json!({"task_id":tid}),
            seq: 6,
            token: Some("tb".into()),
            trace_id: None,
        },
    )
    .await
    .unwrap_err();
    let err_lower = err.to_ascii_lowercase();
    assert!(
        err_lower.contains("permission") || err_lower.contains("tenant"),
        "unexpected error: {}",
        err
    );
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
    let start = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 7,
            token: Some("ta".into()),
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
            token: Some("ta".into()),
            trace_id: None,
        },
    )
        .await
        .unwrap();
    assert_eq!(cancel.seq, 8);
    match cancel.result {
        RPCResult::Success(v) => {
            assert_eq!(v.get("task_id").and_then(|x| x.as_str()), Some(tid.as_str()));
            assert_eq!(v.get("accepted").and_then(|x| x.as_bool()), Some(true));
        }
        _ => panic!("unexpected rpc failure"),
    }
}

#[tokio::test]
async fn cfg_01_sys_config_get_aicc_settings_success() {
    let target = resolve_sys_config_test_endpoint().await;
    let admin_token = sys_config_admin_token();
    let key = "services/aicc/settings";
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
        None,
    )
    .await
    .expect("sys_config_get should succeed");
    assert_eq!(get_resp.seq, 1002);

    let payload = match get_resp.result {
        RPCResult::Success(v) => v,
        other => panic!("unexpected rpc result: {:?}", other),
    };
    assert_eq!(
        payload.get("key").and_then(|v| v.as_str()),
        Some("services/aicc/settings")
    );
    assert_eq!(payload.get("version").and_then(|v| v.as_u64()), Some(1));
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
    let admin_token = sys_config_admin_token();
    let key = "services/aicc/settings";
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
        None,
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
    let admin_token = sys_config_admin_token();
    let key = "services/aicc/settings";
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
        None,
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
    let err = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_set",
        json!({
            "key": "services/aicc/settings",
            "value": json!({"fallback_limit": 5}).to_string()
        }),
        1008,
        Some("tenant-a"),
    )
    .await
    .expect_err("write without permission should fail");
    let err_lower = err.to_ascii_lowercase();
    assert!(
        err_lower.contains("permission"),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn cfg_05_sys_config_value_not_json_string_rejected() {
    let target = resolve_sys_config_test_endpoint().await;
    let admin_token = sys_config_admin_token();
    let err = post_sys_config_rpc(
        &target.endpoint,
        "sys_config_set",
        json!({
            "key": "services/aicc/settings",
            "value": "not-json"
        }),
        1009,
        Some(admin_token.as_str()),
    )
    .await
    .expect_err("invalid json string value should fail");
    let err_lower = err.to_ascii_lowercase();
    assert!(
        err_lower.contains("json") || err_lower.contains("parse"),
        "unexpected error: {}",
        err
    );
}

