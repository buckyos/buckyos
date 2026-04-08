mod common;

use common::{
    resolve_gateway_aicc_endpoint_from_env, resolve_gateway_system_config_endpoint_from_env,
    resolve_remote_test_token,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
const ENV_AICC_TIMEOUT_SECONDS: &str = "AICC_TIMEOUT_SECONDS";
const ENV_AICC_MODEL_ALIAS: &str = "AICC_MODEL_ALIAS";

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const DEFAULT_MODEL_ALIAS: &str = "llm.plan.default";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";

const SETTINGS_KEY_AICC: &str = "services/aicc/settings";
const OPENAI_INSTANCE_ID: &str = "openai-real-workflow";
const OPENAI_PROVIDER_TYPE: &str = "openai";

const TRACE_SETUP_SETTINGS: &str = "workflow-real-setup-settings";
const TRACE_SETUP_RELOAD: &str = "workflow-real-setup-reload";
const TRACE_COMPLEX_DAG: &str = "workflow-real-complex-dag";
const TRACE_JSON_OUTPUT: &str = "workflow-real-json-output";
const TRACE_STREAM_OUTPUT: &str = "workflow-real-stream-output";

const MIN_EXPECTED_STEPS: usize = 6;
const MAX_TOKENS_COMPLEX_DAG: u64 = 1800;
const MAX_TOKENS_JSON_OUTPUT: u64 = 320;
const MAX_TOKENS_STREAM_OUTPUT: u64 = 256;
const TEMP_COMPLEX_DAG: f64 = 0.1;
const TEMP_JSON_OUTPUT: f64 = 0.0;
const TEMP_STREAM_OUTPUT: f64 = 0.2;

const COMPLEX_DAG_PROMPT: &str = r#"You are a workflow planner.
Return JSON only (no markdown).
Generate a DAG plan for "product release multimedia package" with:
- plan_id, goal, steps
- each step has: id, title, capability, model_alias, depends_on, parallel_group, acceptance_criteria
- acceptance_criteria MUST be a non-empty array of strings (never a single string)
- depends_on MUST be an array of step id strings
- at least 6 steps
- at least one parallel group containing >=2 steps
- at least one serial dependency step (depends_on not empty)
- at least one step with retry_policy or replan_trigger
"#;

const JSON_OUTPUT_PROMPT: &str = r#"Return strict JSON object only:
{"ok":true,"kind":"protocol-json","source":"aicc"}"#;

const STREAM_PROMPT: &str = "Protocol stream mode smoke check.";

fn now_seq() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn required_env(key: &str) -> String {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| panic!("missing required env '{}'", key))
}

fn build_sys(seq: u64, token: Option<&str>, trace_id: &str) -> Value {
    let mut arr = vec![json!(seq)];
    arr.push(token.map(Value::from).unwrap_or(Value::Null));
    arr.push(json!(trace_id));
    Value::Array(arr)
}

struct RemoteCtx {
    client: Client,
    aicc_endpoint: String,
    system_config_endpoint: String,
    token: Option<String>,
}

async fn build_remote_ctx() -> RemoteCtx {
    let aicc_endpoint = resolve_gateway_aicc_endpoint_from_env()
        .expect("missing gateway endpoint: set AICC_GATEWAY_HOST or AICC_HOST");
    let system_config_endpoint = resolve_gateway_system_config_endpoint_from_env()
        .expect("missing gateway system_config endpoint: set AICC_GATEWAY_HOST or AICC_HOST");
    let token = resolve_remote_test_token(Some(aicc_endpoint.as_str()))
        .await
        .expect("resolve remote rpc token");
    let timeout = env_u64(ENV_AICC_TIMEOUT_SECONDS, DEFAULT_TIMEOUT_SECONDS);
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout))
        .build()
        .expect("build reqwest client");

    RemoteCtx {
        client,
        aicc_endpoint,
        system_config_endpoint,
        token,
    }
}

async fn rpc_call(
    client: &Client,
    endpoint: &str,
    method: &str,
    params: Value,
    seq: u64,
    token: Option<&str>,
    trace_id: &str,
) -> Result<Value, String> {
    let req = json!({
        "method": method,
        "params": params,
        "sys": build_sys(seq, token, trace_id),
    });

    let resp = client
        .post(endpoint)
        .json(&req)
        .send()
        .await
        .map_err(|err| format!("http request failed: {}", err))?;
    let status = resp.status();
    let payload: Value = resp
        .json()
        .await
        .map_err(|err| format!("invalid rpc json response: {}", err))?;

    if !status.is_success() {
        return Err(format!("http status {} payload={}", status, payload));
    }
    if let Some(err) = payload.get("error") {
        return Err(format!("rpc error payload={}", err));
    }
    payload
        .get("result")
        .cloned()
        .ok_or_else(|| format!("rpc result missing: {}", payload))
}

async fn configure_real_openai_provider(ctx: &RemoteCtx) {
    let api_key = required_env(ENV_OPENAI_API_KEY);
    let alias = env::var(ENV_AICC_MODEL_ALIAS)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL_ALIAS.to_string());

    let mut alias_map = serde_json::Map::<String, Value>::new();
    alias_map.insert("llm.default".to_string(), json!(DEFAULT_OPENAI_MODEL));
    alias_map.insert("llm.chat.default".to_string(), json!(DEFAULT_OPENAI_MODEL));
    alias_map.insert("llm.plan.default".to_string(), json!(DEFAULT_OPENAI_MODEL));
    alias_map.insert("llm.code.default".to_string(), json!(DEFAULT_OPENAI_MODEL));
    alias_map.insert(alias, json!(DEFAULT_OPENAI_MODEL));

    let settings = json!({
        "openai": {
            "enabled": true,
            "api_token": api_key,
            "instances": [{
                "instance_id": OPENAI_INSTANCE_ID,
                "provider_type": OPENAI_PROVIDER_TYPE,
                "base_url": DEFAULT_OPENAI_BASE_URL,
                "timeout_ms": Duration::from_secs(DEFAULT_TIMEOUT_SECONDS).as_millis() as u64,
                "models": [DEFAULT_OPENAI_MODEL],
                "default_model": DEFAULT_OPENAI_MODEL,
                "features": ["plan", "json_output", "tool_calling", "web_search"],
            }],
            "alias_map": Value::Object(alias_map),
        }
    });

    rpc_call(
        &ctx.client,
        ctx.system_config_endpoint.as_str(),
        "sys_config_set",
        json!({
            "key": SETTINGS_KEY_AICC,
            "value": settings.to_string(),
        }),
        now_seq(),
        ctx.token.as_deref(),
        TRACE_SETUP_SETTINGS,
    )
    .await
    .expect("sys_config_set services/aicc/settings failed");

    rpc_call(
        &ctx.client,
        ctx.aicc_endpoint.as_str(),
        "service.reload_settings",
        json!({}),
        now_seq(),
        ctx.token.as_deref(),
        TRACE_SETUP_RELOAD,
    )
    .await
    .expect("service.reload_settings failed");
}

fn complete_params(
    prompt: &str,
    must_features: Vec<&str>,
    options: Value,
    model_alias: &str,
) -> Value {
    json!({
        "capability": "llm_router",
        "model": { "alias": model_alias },
        "requirements": { "must_features": must_features },
        "payload": {
            "messages": [{
                "role": "user",
                "content": prompt
            }],
            "options": options
        },
        "idempotency_key": format!("workflow-real-{}", now_seq())
    })
}

async fn complete_call(
    ctx: &RemoteCtx,
    trace_id: &str,
    params: Value,
) -> Result<Value, String> {
    rpc_call(
        &ctx.client,
        ctx.aicc_endpoint.as_str(),
        "complete",
        params,
        now_seq(),
        ctx.token.as_deref(),
        trace_id,
    )
    .await
}

fn strip_markdown_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let lines = trimmed.lines().collect::<Vec<_>>();
    if lines.len() < 3 {
        return trimmed.to_string();
    }
    lines[1..lines.len() - 1].join("\n")
}

fn parse_json_text(raw: &str) -> Result<Value, String> {
    let cleaned = strip_markdown_fence(raw);
    serde_json::from_str::<Value>(cleaned.as_str()).map_err(|err| {
        format!(
            "model output is not valid json: {}; raw_head={}",
            err,
            raw.chars().take(280).collect::<String>()
        )
    })
}

fn as_array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(field)
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("field '{}' must be an array: {}", field, value))
}

fn as_string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("field '{}' must be a non-empty string: {}", field, value))
}

fn validate_complex_plan(plan: &Value, min_steps: usize) -> Result<(), String> {
    let _ = as_string_field(plan, "plan_id")?;
    let _ = as_string_field(plan, "goal")?;
    let steps = as_array(plan, "steps")?;
    if steps.len() < min_steps {
        return Err(format!(
            "steps count {} is smaller than min_steps {}",
            steps.len(),
            min_steps
        ));
    }

    let mut parallel_groups = HashMap::<String, usize>::new();
    let mut has_dependency_step = false;
    let mut has_retry_or_replan = false;

    for step in steps {
        let _ = as_string_field(step, "id")?;
        let _ = as_string_field(step, "title")?;
        let _ = as_string_field(step, "capability")?;
        let _ = as_string_field(step, "model_alias")?;
        let _ = as_array(step, "acceptance_criteria")?;

        let depends_on = as_array(step, "depends_on")?;
        if !depends_on.is_empty() {
            has_dependency_step = true;
        }
        if step.get("retry_policy").is_some() || step.get("replan_trigger").is_some() {
            has_retry_or_replan = true;
        }

        if let Some(group) = step.get("parallel_group").and_then(|v| v.as_str()) {
            let normalized = group.trim();
            if !normalized.is_empty() {
                *parallel_groups.entry(normalized.to_string()).or_insert(0) += 1;
            }
        }
    }

    if !parallel_groups.values().any(|count| *count >= 2) {
        return Err("plan has no parallel group with at least 2 steps".to_string());
    }
    if !has_dependency_step {
        return Err("plan has no dependent(serial) step".to_string());
    }
    if !has_retry_or_replan {
        return Err("plan has no retry_policy/replan_trigger".to_string());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires target gateway and OPENAI_API_KEY"]
async fn workflow_real_01_gateway_complex_scenario_protocol_mix() {
    let ctx = build_remote_ctx().await;
    configure_real_openai_provider(&ctx).await;
    let model_alias = env::var(ENV_AICC_MODEL_ALIAS)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL_ALIAS.to_string());

    let complex_result = complete_call(
        &ctx,
        TRACE_COMPLEX_DAG,
        complete_params(
            COMPLEX_DAG_PROMPT,
            vec!["json_output"],
            json!({
                "temperature": TEMP_COMPLEX_DAG,
                "max_tokens": MAX_TOKENS_COMPLEX_DAG,
                "response_format": {"type": "json_object"},
            }),
            model_alias.as_str(),
        ),
    )
    .await
    .expect("complex dag rpc failed");

    let complex_status = complex_result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        complex_status, "succeeded",
        "complex dag status={} result={}",
        complex_status, complex_result
    );
    let complex_text = complex_result
        .pointer("/result/text")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("complex dag missing /result/text: {}", complex_result));
    let complex_plan = parse_json_text(complex_text).unwrap_or_else(|err| panic!("{}", err));
    validate_complex_plan(&complex_plan, MIN_EXPECTED_STEPS).unwrap_or_else(|err| {
        panic!(
            "complex plan validation failed: {}; plan={}",
            err, complex_plan
        )
    });

    let json_result = complete_call(
        &ctx,
        TRACE_JSON_OUTPUT,
        complete_params(
            JSON_OUTPUT_PROMPT,
            vec!["json_output"],
            json!({
                "temperature": TEMP_JSON_OUTPUT,
                "max_tokens": MAX_TOKENS_JSON_OUTPUT,
                "response_format": {"type": "json_object"},
            }),
            model_alias.as_str(),
        ),
    )
    .await
    .expect("json-output rpc failed");
    let json_status = json_result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        json_status, "succeeded",
        "json-output status={} result={}",
        json_status, json_result
    );
    let json_text = json_result
        .pointer("/result/text")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("json-output missing /result/text: {}", json_result));
    let parsed_json_output = parse_json_text(json_text).unwrap_or_else(|err| panic!("{}", err));
    assert_eq!(
        parsed_json_output
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        true,
        "json-output mismatch: {}",
        parsed_json_output
    );

    let stream_result = complete_call(
        &ctx,
        TRACE_STREAM_OUTPUT,
        complete_params(
            STREAM_PROMPT,
            vec![],
            json!({
                "temperature": TEMP_STREAM_OUTPUT,
                "max_tokens": MAX_TOKENS_STREAM_OUTPUT,
                "stream": true,
            }),
            model_alias.as_str(),
        ),
    )
    .await
    .expect("stream rpc failed");
    let stream_status = stream_result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        matches!(stream_status, "running" | "succeeded"),
        "stream status={} result={}",
        stream_status,
        stream_result
    );
}
