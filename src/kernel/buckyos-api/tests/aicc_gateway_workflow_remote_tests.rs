mod aicc_remote_common;

use ::kRPC::{kRPC as KRpcClient, RPCContext, RPCRequest, RPCResult};
use aicc_remote_common::*;
use buckyos_api::{
    AiMessage, AiPayload, AiccClient, Capability, CompleteRequest, CompleteStatus, ModelSpec,
    Requirements,
};
use reqwest::Url;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const ENV_AICC_MODEL_ALIAS: &str = "AICC_MODEL_ALIAS";
const ENV_AICC_OPENAI_API_KEY: &str = "AICC_OPENAI_API_KEY";
const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
const ENV_AICC_OPENAI_BASE_URL: &str = "AICC_OPENAI_BASE_URL";
const ENV_AICC_SN_OPENAI_BASE_URL: &str = "AICC_SN_OPENAI_BASE_URL";
const ENV_AICC_OPENAI_MODEL: &str = "AICC_OPENAI_MODEL";
const ENV_AICC_SN_AUTH_SUBJECT: &str = "AICC_SN_AUTH_SUBJECT";
const ENV_AICC_SN_AUTH_APPID: &str = "AICC_SN_AUTH_APPID";
const ENV_AICC_SN_AUTH_PRIVATE_KEY_PATH: &str = "AICC_SN_AUTH_PRIVATE_KEY_PATH";
const DEFAULT_MODEL_ALIAS: &str = "llm.plan.default";
const FIXED_SN_MODEL_ALIAS: &str = "llm.plan.default";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const FIXED_SN_OPENAI_BASE_URL: &str = "https://sn.buckyos.ai/api/v1/ai/chat/completions";
const DEFAULT_OPENAI_MODEL: &str = "gpt-5.4";

const MIN_EXPECTED_STEPS: usize = 6;
const MAX_TOKENS_COMPLEX_DAG: u64 = 1800;
const MAX_TOKENS_JSON_OUTPUT: u64 = 320;
const MAX_TOKENS_STREAM_OUTPUT: u64 = 1024;
const TEMP_COMPLEX_DAG: f64 = 0.1;
const TEMP_JSON_OUTPUT: f64 = 0.0;
const TEMP_STREAM_OUTPUT: f64 = 0.0;
const WORKFLOW_REMOTE_CLIENT_TIMEOUT_SECS: u64 = 120;

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

fn required_env(key: &str) -> String {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| panic!("missing required env '{}'", key))
}

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn model_alias_from_env() -> String {
    optional_env(ENV_AICC_MODEL_ALIAS).unwrap_or_else(|| DEFAULT_MODEL_ALIAS.to_string())
}

fn sn_openai_base_url_from_env() -> String {
    optional_env(ENV_AICC_SN_OPENAI_BASE_URL)
        .or_else(|| optional_env(ENV_AICC_OPENAI_BASE_URL))
        .unwrap_or_else(|| FIXED_SN_OPENAI_BASE_URL.to_string())
}

fn complete_request(
    prompt: &str,
    must_features: Vec<&str>,
    options: Value,
    model_alias: &str,
) -> CompleteRequest {
    let messages = vec![AiMessage::new("user".to_string(), prompt.to_string())];
    CompleteRequest::new(
        Capability::LlmRouter,
        ModelSpec::new(model_alias.to_string(), None),
        Requirements::new(
            must_features
                .into_iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
            Some(100000),
            Some(0.2),
            None,
        ),
        AiPayload::new(None, messages, vec![], vec![], None, Some(options)),
        Some(format!("workflow-remote-{}", now_seq())),
    )
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

fn require_remote_target() -> RpcTestEndpoint {
    let endpoint = required_env("AICC_GATEWAY_HOST");
    let gateway = resolve_endpoint_from_env(&[], &["AICC_GATEWAY_HOST"], "/kapi/aicc")
        .unwrap_or_else(|| panic!("failed to derive gateway endpoint from '{}'", endpoint));
    RpcTestEndpoint::from_remote(gateway)
}

async fn token_for_remote_target(target: &RpcTestEndpoint) -> Option<String> {
    if target.is_remote {
        resolve_remote_test_token(Some(&target.endpoint))
            .await
            .expect("resolve remote test token")
    } else {
        None
    }
}

fn require_openai_api_key() -> String {
    optional_env(ENV_AICC_OPENAI_API_KEY)
        .or_else(|| optional_env(ENV_OPENAI_API_KEY))
        .unwrap_or_else(|| {
            panic!(
                "missing required env '{}' (or '{}') for provider bootstrap",
                ENV_AICC_OPENAI_API_KEY, ENV_OPENAI_API_KEY
            )
        })
}

fn resolve_system_config_endpoint(aicc_endpoint: &str) -> String {
    let mut parsed = Url::parse(aicc_endpoint)
        .unwrap_or_else(|err| panic!("invalid AICC endpoint '{}': {}", aicc_endpoint, err));
    parsed.set_path("/kapi/system_config");
    parsed.set_query(None);
    parsed.to_string()
}

#[derive(Clone, Copy)]
enum ProviderBootstrapMode {
    OpenAiEnv,
    SnOpenAiJwt,
}

async fn ensure_provider_configured_for_remote(
    target: &RpcTestEndpoint,
    mode: ProviderBootstrapMode,
) -> Option<String> {
    if !target.is_remote {
        return None;
    }

    let sys_config_endpoint = resolve_system_config_endpoint(&target.endpoint);
    let token = resolve_remote_test_token(Some(&sys_config_endpoint))
        .await
        .expect("resolve remote token for provider bootstrap")
        .unwrap_or_else(|| {
            panic!("remote provider bootstrap requires AICC_RPC_TOKEN or username/password env")
        });
    let (api_token, base_url, provider_type, auth_mode) = match mode {
        ProviderBootstrapMode::OpenAiEnv => (
            require_openai_api_key(),
            optional_env(ENV_AICC_OPENAI_BASE_URL)
                .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string()),
            "openai".to_string(),
            "bearer".to_string(),
        ),
        ProviderBootstrapMode::SnOpenAiJwt => (
            String::new(),
            sn_openai_base_url_from_env(),
            "sn-openai".to_string(),
            "device_jwt".to_string(),
        ),
    };
    let model =
        optional_env(ENV_AICC_OPENAI_MODEL).unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_string());

    let mut instance_settings = json!({
        "instance_id": "workflow-openai-remote",
        "provider_type": provider_type,
        "base_url": base_url,
        "auth_mode": auth_mode,
        "timeout_ms": 120000,
        "models": [model.clone()],
        "default_model": model,
        "features": ["plan", "json_output", "tool_calling", "web_search"]
    });
    if matches!(mode, ProviderBootstrapMode::SnOpenAiJwt) {
        if let Some(subject) = optional_env(ENV_AICC_SN_AUTH_SUBJECT) {
            instance_settings["auth_subject"] = json!(subject);
        }
        if let Some(appid) = optional_env(ENV_AICC_SN_AUTH_APPID) {
            instance_settings["auth_appid"] = json!(appid);
        }
        if let Some(private_key_path) = optional_env(ENV_AICC_SN_AUTH_PRIVATE_KEY_PATH) {
            instance_settings["auth_private_key_path"] = json!(private_key_path);
        }
    }

    let openai_settings = json!({
        "enabled": true,
        "api_token": api_token,
        "instances": [instance_settings]
    });

    let set_result = post_rpc_over_http(
        &sys_config_endpoint,
        &RPCRequest {
            method: "sys_config_set_by_json_path".to_string(),
            params: json!({
                "key": "services/aicc/settings",
                "json_path": "/openai",
                "value": openai_settings.to_string(),
            }),
            seq: now_seq(),
            token: Some(token.clone()),
            trace_id: Some("workflow-bootstrap-provider-config".to_string()),
        },
    )
    .await
    .unwrap_or_else(|err| panic!("sys_config_set_by_json_path failed: {}", err));

    if !matches!(set_result.result, RPCResult::Success(_)) {
        panic!(
            "sys_config_set_by_json_path returned non-success: {:?}",
            set_result.result
        );
    }

    let reload_resp = post_rpc_over_http(
        &target.endpoint,
        &RPCRequest {
            method: "service.reload_settings".to_string(),
            params: json!({}),
            seq: now_seq(),
            token: Some(token.clone()),
            trace_id: Some("workflow-bootstrap-provider-reload".to_string()),
        },
    )
    .await
    .unwrap_or_else(|err| panic!("reload_settings after provider bootstrap failed: {}", err));

    let payload = match reload_resp.result {
        RPCResult::Success(value) => value,
        other => panic!("reload_settings returned non-success: {:?}", other),
    };

    let providers = payload
        .get("providers_registered")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        providers > 0,
        "remote aicc has no providers after bootstrap+reload; providers_registered={}, payload={}",
        providers,
        payload
    );
    Some(token)
}

async fn run_remote_workflow_suite(model_alias: &str, trace_id: &str, mode: ProviderBootstrapMode) {
    let target = require_remote_target();
    let bootstrap_token = ensure_provider_configured_for_remote(&target, mode).await;
    let token = if bootstrap_token.is_some() {
        bootstrap_token
    } else {
        token_for_remote_target(&target).await
    };
    if let Some(token_value) = token.as_ref() {
        assert!(
            token_value.split('.').count() == 3,
            "AICC token should be a JWT-like token (three dot-separated segments)"
        );
    }

    let client = AiccClient::new(KRpcClient::new_with_timeout_secs(
        &target.endpoint,
        None,
        WORKFLOW_REMOTE_CLIENT_TIMEOUT_SECS,
    ));
    client
        .set_context(RPCContext {
            token,
            trace_id: Some(trace_id.to_string()),
            ..Default::default()
        })
        .await;

    let complex = client
        .complete(complete_request(
            COMPLEX_DAG_PROMPT,
            vec!["json_output"],
            json!({
                "temperature": TEMP_COMPLEX_DAG,
                "max_tokens": MAX_TOKENS_COMPLEX_DAG,
                "response_format": {"type": "json_object"},
            }),
            model_alias,
        ))
        .await
        .unwrap_or_else(|err| panic!("complex dag remote call failed: {}", err));
    assert_eq!(
        complex.status,
        CompleteStatus::Succeeded,
        "complex dag status={:?} task_id={} result={:?} event_ref={:?}",
        complex.status,
        complex.task_id,
        complex.result,
        complex.event_ref
    );
    let complex_text = complex
        .result
        .as_ref()
        .and_then(|r| r.text.as_ref())
        .unwrap_or_else(|| panic!("complex dag missing result.text: {:?}", complex));
    let complex_plan = parse_json_text(complex_text).unwrap_or_else(|err| panic!("{}", err));
    validate_complex_plan(&complex_plan, MIN_EXPECTED_STEPS).unwrap_or_else(|err| {
        panic!(
            "complex plan validation failed: {}; plan={}",
            err, complex_plan
        )
    });

    let json_resp = client
        .complete(complete_request(
            JSON_OUTPUT_PROMPT,
            vec!["json_output"],
            json!({
                "temperature": TEMP_JSON_OUTPUT,
                "max_tokens": MAX_TOKENS_JSON_OUTPUT,
                "response_format": {"type": "json_object"},
            }),
            model_alias,
        ))
        .await
        .unwrap_or_else(|err| panic!("json-output remote call failed: {}", err));
    assert_eq!(
        json_resp.status,
        CompleteStatus::Succeeded,
        "json-output status={:?} task_id={} result={:?} event_ref={:?}",
        json_resp.status,
        json_resp.task_id,
        json_resp.result,
        json_resp.event_ref
    );
    let json_text = json_resp
        .result
        .as_ref()
        .and_then(|r| r.text.as_ref())
        .unwrap_or_else(|| panic!("json-output missing result.text: {:?}", json_resp));
    let parsed_json = parse_json_text(json_text).unwrap_or_else(|err| panic!("{}", err));
    assert_eq!(
        parsed_json.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "json-output mismatch: {}",
        parsed_json
    );

    let stream_resp = client
        .complete(complete_request(
            STREAM_PROMPT,
            vec![],
            json!({
                "temperature": TEMP_STREAM_OUTPUT,
                "max_tokens": MAX_TOKENS_STREAM_OUTPUT,
                "stream": true,
            }),
            model_alias,
        ))
        .await
        .unwrap_or_else(|err| panic!("stream remote call failed: {}", err));
    assert!(
        matches!(
            stream_resp.status,
            CompleteStatus::Running | CompleteStatus::Succeeded
        ),
        "stream status={:?} task_id={} result={:?} event_ref={:?}",
        stream_resp.status,
        stream_resp.task_id,
        stream_resp.result,
        stream_resp.event_ref
    );
}

#[tokio::test]
#[ignore = "requires real gateway(AICC_GATEWAY_HOST) and valid auth(AICC_RPC_TOKEN or username/password)"]
async fn workflow_remote_01_gateway_complex_scenario_protocol_mix() {
    let model_alias = model_alias_from_env();
    run_remote_workflow_suite(
        model_alias.as_str(),
        "workflow-real-openai-remote",
        ProviderBootstrapMode::OpenAiEnv,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires real gateway(AICC_GATEWAY_HOST) and valid auth(AICC_RPC_TOKEN or username/password)"]
async fn workflow_remote_02_sn_openai_complex_scenario_protocol_mix() {
    run_remote_workflow_suite(
        FIXED_SN_MODEL_ALIAS,
        "workflow-real-sn-openai-remote",
        ProviderBootstrapMode::SnOpenAiJwt,
    )
    .await;
}
