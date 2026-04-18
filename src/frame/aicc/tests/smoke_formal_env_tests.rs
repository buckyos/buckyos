mod common;

use aicc::{
    AIComputeCenter, CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry,
};
use buckyos_api::{Capability, CompleteRequest};
use common::*;
use reqwest::Client;
use serde_json::{json, Value};
use std::env;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

enum SmokeClient {
    Remote {
        client: Client,
        url: String,
        token: Option<String>,
    },
    Local {
        center: AIComputeCenter,
    },
}

impl SmokeClient {
    async fn from_env_or_local() -> Self {
        match resolve_endpoint_from_env(&[], &["AICC_HOST"], "/kapi/aicc") {
            Some(url) => {
                let timeout = env::var("AICC_TIMEOUT_SECONDS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(30);
                let client = Client::builder()
                    .timeout(Duration::from_secs(timeout))
                    .build()
                    .expect("build reqwest client");
                let token = resolve_remote_test_token(Some(&url))
                    .await
                    .expect("resolve remote test token");
                Self::Remote { client, url, token }
            }
            None => {
                let registry = Registry::default();
                let catalog = ModelCatalog::default();
                add_llm(
                    &registry,
                    &catalog,
                    "smoke-local-p1",
                    "mock",
                    0.01,
                    10,
                    Ok(ProviderStartResult::Started),
                );
                Self::Local {
                    center: center_with_taskmgr(registry, catalog),
                }
            }
        }
    }

    async fn call_rpc(
        &self,
        method: &str,
        params: Value,
        seq: u64,
        trace_id: Option<&str>,
    ) -> Result<Value, String> {
        match self {
            SmokeClient::Remote { client, url, token } => {
                let sys = build_sys(seq, token.as_deref(), trace_id);
                let body = json!({
                    "method": method,
                    "params": params,
                    "sys": sys,
                });
                let resp = client
                    .post(url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|err| format!("request failed: {err}"))?;
                let status = resp.status();
                let payload: Value = resp
                    .json()
                    .await
                    .map_err(|err| format!("invalid JSON response: {err}"))?;
                if !status.is_success() {
                    return Err(format!("http status {} payload={payload}", status));
                }
                if let Some(err) = payload.get("error") {
                    return Err(format!("rpc error: {err}"));
                }
                payload
                    .get("result")
                    .cloned()
                    .ok_or_else(|| format!("rpc result missing: {payload}"))
            }
            SmokeClient::Local { center } => match method {
                "complete" => {
                    let req: CompleteRequest = serde_json::from_value(params)
                        .map_err(|err| format!("invalid complete params: {err}"))?;
                    let ctx = rpc_ctx_with_tenant(None);
                    let resp = center
                        .complete(req, ctx)
                        .await
                        .map_err(|err| format!("local complete failed: {err}"))?;
                    serde_json::to_value(resp)
                        .map_err(|err| format!("serialize complete result failed: {err}"))
                }
                "cancel" => {
                    let task_id = params
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "cancel params missing task_id".to_string())?;
                    let ctx = rpc_ctx_with_tenant(None);
                    let resp = center
                        .cancel(task_id, ctx)
                        .await
                        .map_err(|err| format!("local cancel failed: {err}"))?;
                    serde_json::to_value(resp)
                        .map_err(|err| format!("serialize cancel result failed: {err}"))
                }
                _ => Err(format!("unsupported method in smoke local mode: {method}")),
            },
        }
    }

    fn is_remote(&self) -> bool {
        matches!(self, SmokeClient::Remote { .. })
    }
}

fn build_sys(seq: u64, token: Option<&str>, trace_id: Option<&str>) -> Value {
    let mut arr = vec![json!(seq)];
    if token.is_some() || trace_id.is_some() {
        arr.push(token.map(Value::from).unwrap_or(Value::Null));
    }
    if let Some(trace_id) = trace_id {
        arr.push(json!(trace_id));
    }
    Value::Array(arr)
}

fn now_seq() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn complete_params(prompt: &str, must_features: Vec<&str>, options: Value) -> Value {
    let alias = env::var("AICC_MODEL_ALIAS").unwrap_or_else(|_| "llm.plan.default".to_string());
    json!({
        "capability": "llm_router",
        "model": {
            "alias": alias,
        },
        "requirements": {
            "must_features": must_features,
        },
        "payload": {
            "messages": [
                {
                    "role": "user",
                    "content": prompt,
                }
            ],
            "options": options,
        },
        "idempotency_key": format!("smoke-{}", now_seq()),
    })
}

fn complete_params_with_alias(alias: &str, prompt: &str, options: Value) -> Value {
    json!({
        "capability": "llm_router",
        "model": {
            "alias": alias,
        },
        "requirements": {
            "must_features": [],
        },
        "payload": {
            "messages": [
                {
                    "role": "user",
                    "content": prompt,
                }
            ],
            "options": options,
        },
        "idempotency_key": format!("smoke-{}", now_seq()),
    })
}

#[tokio::test]
async fn smoke_01_complete_basic_succeeds_on_assigned_url() {
    let client = SmokeClient::from_env_or_local().await;
    let result = client
        .call_rpc(
            "complete",
            complete_params(
                "Reply in one short sentence that smoke_01 passed.",
                vec![],
                json!({"temperature": 0.2, "max_tokens": 96}),
            ),
            now_seq(),
            Some("smoke-01"),
        )
        .await
        .expect("complete should succeed");

    let task_id = result.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!task_id.is_empty(), "missing task_id: {result}");
    let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        matches!(status, "succeeded" | "running"),
        "unexpected status: {result}"
    );
}

#[tokio::test]
async fn smoke_02_json_output_path_succeeds_on_assigned_url() {
    let client = SmokeClient::from_env_or_local().await;
    let must_features = if client.is_remote() {
        vec!["json_output"]
    } else {
        vec![]
    };
    let options = if client.is_remote() {
        json!({
            "temperature": 0,
            "max_tokens": 96,
            "response_format": {"type": "json_object"},
        })
    } else {
        json!({
            "temperature": 0,
            "max_tokens": 96,
        })
    };
    let result = client
        .call_rpc(
            "complete",
            complete_params(
                "Return JSON only: {\"ok\": true, \"source\": \"aicc\"}",
                must_features,
                options,
            ),
            now_seq(),
            Some("smoke-02"),
        )
        .await
        .expect("complete with json_output should succeed");

    let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        matches!(status, "succeeded" | "running"),
        "unexpected status: {result}"
    );
}

#[tokio::test]
async fn smoke_03_cancel_endpoint_reachable_on_assigned_url() {
    let client = SmokeClient::from_env_or_local().await;
    let cancel_result = client
        .call_rpc(
            "cancel",
            json!({"task_id": format!("smoke-task-{}", now_seq())}),
            now_seq(),
            Some("smoke-03"),
        )
        .await
        .expect("cancel should be reachable");

    let task_id = cancel_result
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(!task_id.is_empty(), "missing task_id: {cancel_result}");
    assert!(
        cancel_result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .is_some(),
        "missing accepted bool: {cancel_result}"
    );
}

#[tokio::test]
async fn smoke_04_stream_poll_basic_path_on_assigned_url() {
    let client = SmokeClient::from_env_or_local().await;
    let result = client
        .call_rpc(
            "complete",
            complete_params(
                "stream smoke test",
                vec![],
                json!({"temperature": 0.2, "max_tokens": 96, "stream": true}),
            ),
            now_seq(),
            Some("smoke-04"),
        )
        .await
        .expect("stream complete should succeed");

    let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        matches!(status, "succeeded" | "running"),
        "unexpected status: {result}"
    );

    if status == "running" {
        assert!(
            result.get("event_ref").and_then(|v| v.as_str()).is_some() || client.is_remote(),
            "running response should include event_ref in local mode: {result}"
        );
    }
}

#[tokio::test]
async fn smoke_05_monitor_alarm_trigger_and_recovery() {
    let client = SmokeClient::from_env_or_local().await;

    // Trigger phase: force a routing/config failure with an unknown alias.
    let trigger = client
        .call_rpc(
            "complete",
            complete_params_with_alias(
                "smoke.invalid.alias",
                "trigger monitor smoke path",
                json!({"temperature": 0.2, "max_tokens": 32}),
            ),
            now_seq(),
            Some("smoke-05-trigger"),
        )
        .await;
    match trigger {
        Err(trigger_err) => {
            let err_lower = trigger_err.to_ascii_lowercase();
            assert!(
                err_lower.contains("model_alias_not_mapped")
                    || err_lower.contains("no_provider_available")
                    || err_lower.contains("failed"),
                "unexpected trigger error: {trigger_err}"
            );
        }
        Ok(trigger_result) => {
            let status = trigger_result
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert_eq!(
                status, "failed",
                "invalid alias should reach failed status: {trigger_result}"
            );
        }
    }

    // Recovery phase: switch back to the configured alias and verify success.
    let recover_result = client
        .call_rpc(
            "complete",
            complete_params(
                "smoke recovery path",
                vec![],
                json!({"temperature": 0.2, "max_tokens": 64}),
            ),
            now_seq(),
            Some("smoke-05-recover"),
        )
        .await
        .expect("recovery complete should succeed");
    let status = recover_result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        matches!(status, "running" | "succeeded"),
        "unexpected recovery status: {recover_result}"
    );
}

#[tokio::test]
async fn smoke_06_bug_context_capture_template_complete() {
    let client = SmokeClient::from_env_or_local().await;
    let trace_id = "smoke-06-trace";
    let req_params = complete_params_with_alias(
        "smoke.invalid.alias",
        "force failure for context capture",
        json!({"temperature": 0.1, "max_tokens": 32}),
    );
    let trigger = client
        .call_rpc("complete", req_params.clone(), now_seq(), Some(trace_id))
        .await;
    let (error_message, task_id) = match trigger {
        Err(rpc_err) => {
            let err_lower = rpc_err.to_ascii_lowercase();
            assert!(
                err_lower.contains("model_alias_not_mapped")
                    || err_lower.contains("no_provider_available")
                    || err_lower.contains("failed"),
                "unexpected trigger error: {rpc_err}"
            );
            (rpc_err, String::new())
        }
        Ok(result) => {
            let status = result
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            assert_eq!(
                status, "failed",
                "invalid alias should reach failed status: {result}"
            );
            let tid = result
                .get("task_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            assert!(
                !tid.is_empty(),
                "failed result should still include task_id: {result}"
            );
            (format!("business failed with status={}", status), tid)
        }
    };
    let log_snippet = format!(
        "[ERROR] trace={} rpc_error={}",
        trace_id,
        context_safe_snippet(&error_message)
    );

    let context = json!({
        "request": {
            "method": "complete",
            "trace_id": trace_id,
            "tenant": env::var("AICC_RPC_TOKEN").ok().unwrap_or_else(|| "local-default".to_string()),
            "idempotency_key": req_params.get("idempotency_key").and_then(|v| v.as_str()).unwrap_or(""),
            "alias": req_params.pointer("/model/alias").and_then(|v| v.as_str()).unwrap_or(""),
        },
        "execution": {
            "error_message": error_message,
        },
        "artifacts": {
            "task_id": task_id,
            "event_ref": "",
        },
        "logs": {
            "snippet": log_snippet,
        }
    });

    assert_eq!(
        context.pointer("/request/method").and_then(|v| v.as_str()),
        Some("complete")
    );
    assert_eq!(
        context
            .pointer("/request/trace_id")
            .and_then(|v| v.as_str()),
        Some(trace_id)
    );
    assert!(
        !context
            .pointer("/request/idempotency_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty(),
        "missing idempotency_key in context: {context}"
    );
    assert_eq!(
        context.pointer("/request/alias").and_then(|v| v.as_str()),
        Some("smoke.invalid.alias")
    );
    assert!(
        context
            .pointer("/execution/error_message")
            .and_then(|v| v.as_str())
            .map(|v| !v.is_empty())
            .unwrap_or(false),
        "missing execution error message in context: {context}"
    );
    assert!(
        context
            .pointer("/execution/error_message")
            .and_then(|v| v.as_str())
            .map(|v| {
                let lowered = v.to_ascii_lowercase();
                lowered.contains("failed")
                    || lowered.contains("model_alias_not_mapped")
                    || lowered.contains("no_provider_available")
                    || lowered.contains("rpc error")
            })
            .unwrap_or(false),
        "unexpected execution error message in context: {context}"
    );
    assert!(
        context
            .pointer("/logs/snippet")
            .and_then(|v| v.as_str())
            .map(|v| v.contains(trace_id))
            .unwrap_or(false),
        "log snippet should contain trace_id: {context}"
    );
    let log_prefix = format!("[ERROR] trace={} rpc_error=", trace_id);
    assert!(
        context
            .pointer("/logs/snippet")
            .and_then(|v| v.as_str())
            .map(|v| v.len() <= log_prefix.len() + 160)
            .unwrap_or(false),
        "log snippet should keep bounded error snippet: {context}"
    );
}

fn context_safe_snippet(input: &str) -> String {
    let mut s = input.to_string();
    if s.len() > 160 {
        s.truncate(160);
    }
    s
}
