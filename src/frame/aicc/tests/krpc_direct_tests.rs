mod common;

use aicc::{CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry};
use buckyos_api::{AiccServerHandler, Capability};
use common::*;
use kRPC::{RPCRequest, RPCResult};
use serde_json::json;
use std::sync::Arc;

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

#[tokio::test]
async fn krpc_direct_01_complete_minimal_llm_success() {
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
    let target = resolve_rpc_test_endpoint(h).await;
    let resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 201,
            token: None,
            trace_id: None,
        },
    )
        .await
        .unwrap();
    assert_eq!(resp.seq, 201);
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
async fn krpc_direct_02_complete_with_sys_seq_token_trace_success() {
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
    let target = resolve_rpc_test_endpoint(h).await;
    let resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 202,
            token: Some("tenant-a".into()),
            trace_id: Some("trace-a".into()),
        },
    )
        .await
        .unwrap();
    assert_eq!(resp.seq, 202);
    assert_eq!(resp.trace_id.as_deref(), Some("trace-a"));
}

#[tokio::test]
async fn krpc_direct_03_complete_invalid_sys_shape_returns_bad_request() {
    let h = Arc::new(AiccServerHandler::new(center_with_taskmgr(
        Registry::default(),
        ModelCatalog::default(),
    )));
    let target = resolve_rpc_test_endpoint(h).await;
    let err = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: json!({"bad":"payload"}),
            seq: 203,
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
async fn krpc_direct_04_cancel_cross_tenant_rejected() {
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
    let target = resolve_rpc_test_endpoint(h).await;
    let start = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 204,
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
            seq: 205,
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
async fn krpc_direct_05_cancel_same_tenant_accepted_or_graceful_false() {
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
    let target = resolve_rpc_test_endpoint(h).await;
    let start = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: serde_json::to_value(base_request()).unwrap(),
            seq: 206,
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
            seq: 207,
            token: Some("ta".into()),
            trace_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(cancel.seq, 207);
    match cancel.result {
        RPCResult::Success(v) => {
            assert_eq!(v.get("task_id").and_then(|x| x.as_str()), Some(tid.as_str()));
            assert_eq!(v.get("accepted").and_then(|x| x.as_bool()), Some(true));
        }
        _ => panic!("unexpected rpc failure"),
    }
}

