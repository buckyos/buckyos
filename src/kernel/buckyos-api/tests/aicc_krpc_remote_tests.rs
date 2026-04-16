mod aicc_remote_common;

use aicc_remote_common::*;
use kRPC::{RPCContext, RPCRequest};
use serde_json::json;

async fn token_for_remote_target(target: &RpcTestEndpoint) -> Option<String> {
    if target.is_remote {
        resolve_remote_test_token(Some(&target.endpoint))
            .await
            .expect("resolve remote test token")
    } else {
        None
    }
}

#[tokio::test]
async fn krpc_direct_01_complete_minimal_llm_success() {
    let target = resolve_krpc_target().await;
    let remote_token = token_for_remote_target(&target).await;

    let client = build_client(&target.endpoint);
    client
        .set_context(RPCContext {
            token: remote_token,
            ..Default::default()
        })
        .await;

    let resp = client.complete(base_request()).await.unwrap();
    assert!(!resp.task_id.is_empty(), "missing task_id");
    if target.is_remote {
        assert!(
            matches!(
                resp.status,
                buckyos_api::CompleteStatus::Running
                    | buckyos_api::CompleteStatus::Succeeded
                    | buckyos_api::CompleteStatus::Failed
            ),
            "unexpected status: {:?}",
            resp.status
        );
    } else {
        assert!(
            matches!(
                resp.status,
                buckyos_api::CompleteStatus::Running | buckyos_api::CompleteStatus::Succeeded
            ),
            "unexpected status: {:?}",
            resp.status
        );
    }
}

#[tokio::test]
async fn krpc_direct_02_complete_with_sys_seq_token_trace_success() {
    let target = resolve_krpc_target().await;
    let remote_token = token_for_remote_target(&target).await;

    let client = build_client(&target.endpoint);
    client
        .set_context(RPCContext {
            token: remote_token.or_else(|| Some("tenant-a".into())),
            trace_id: Some("trace-a".into()),
            ..Default::default()
        })
        .await;

    let resp = client.complete(base_request()).await.unwrap();
    assert!(!resp.task_id.is_empty(), "missing task_id");
}

#[tokio::test]
async fn krpc_direct_03_complete_invalid_sys_shape_returns_bad_request() {
    let target = resolve_krpc_target().await;
    let remote_token = token_for_remote_target(&target).await;

    let err = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "complete".into(),
            params: json!({"bad":"payload"}),
            seq: 203,
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
async fn krpc_direct_04_cancel_cross_tenant_rejected() {
    let target = resolve_krpc_target().await;

    let client_a = build_client(&target.endpoint);
    client_a
        .set_context(RPCContext {
            token: Some("ta".to_string()),
            ..Default::default()
        })
        .await;

    let start = client_a.complete(base_request()).await.unwrap();

    let cross_tenant_token = if target.is_remote {
        Some("cross-tenant-test-invalid-token".to_string())
    } else {
        Some("tb".to_string())
    };

    let client_b = build_client(&target.endpoint);
    client_b
        .set_context(RPCContext {
            token: cross_tenant_token,
            ..Default::default()
        })
        .await;

    let cancel_result = client_b.cancel(&start.task_id).await;
    match cancel_result {
        Ok(resp) => {
            assert_eq!(resp.task_id, start.task_id);
            assert!(!resp.accepted, "cross tenant cancel should not be accepted");
        }
        Err(err) => {
            let err_lower = err.to_string().to_ascii_lowercase();
            assert!(
                err_lower.contains("permission") || err_lower.contains("tenant"),
                "unexpected error: {}",
                err
            );
        }
    }
}

#[tokio::test]
async fn krpc_direct_05_cancel_same_tenant_accepted_or_graceful_false() {
    let target = resolve_krpc_target().await;
    let remote_token = token_for_remote_target(&target).await;

    let client = build_client(&target.endpoint);
    client
        .set_context(RPCContext {
            token: remote_token.or_else(|| Some("ta".to_string())),
            ..Default::default()
        })
        .await;

    let start = client.complete(base_request()).await.unwrap();
    assert!(
        !start.task_id.is_empty(),
        "complete should return task_id before cancel"
    );

    let cancel = client.cancel(&start.task_id).await.unwrap();
    assert_eq!(cancel.task_id, start.task_id);
    if !target.is_remote {
        assert!(cancel.accepted);
    }
}
