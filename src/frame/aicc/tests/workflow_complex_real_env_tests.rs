mod common;

use aicc::openai::register_openai_llm_providers;
use aicc::{AIComputeCenter, ModelCatalog, Registry, TaskEvent};
use buckyos_api::{AiccHandler, CompleteRequest};
use common::{center_with_taskmgr, rpc_ctx_with_tenant, CollectingSinkFactory};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
const ENV_AICC_MODEL_ALIAS: &str = "AICC_MODEL_ALIAS";

const DEFAULT_MODEL_ALIAS: &str = "llm.plan.default";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";

const OPENAI_INSTANCE_ID: &str = "openai-real-workflow";
const OPENAI_PROVIDER_TYPE: &str = "openai";

const TRACE_COMPLEX_DAG: &str = "workflow-real-complex-dag-local";
const TRACE_JSON_OUTPUT: &str = "workflow-real-json-output-local";
const TRACE_STREAM_OUTPUT: &str = "workflow-real-stream-output-local";

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

fn required_env(key: &str) -> String {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| panic!("missing required env '{}'", key))
}

fn model_alias_from_env() -> String {
    env::var(ENV_AICC_MODEL_ALIAS)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL_ALIAS.to_string())
}

fn build_local_openai_settings(model_alias: &str) -> Value {
    let api_key = required_env(ENV_OPENAI_API_KEY);

    let mut alias_map = serde_json::Map::<String, Value>::new();
    alias_map.insert("llm.default".to_string(), json!(DEFAULT_OPENAI_MODEL));
    alias_map.insert("llm.chat.default".to_string(), json!(DEFAULT_OPENAI_MODEL));
    alias_map.insert("llm.plan.default".to_string(), json!(DEFAULT_OPENAI_MODEL));
    alias_map.insert("llm.code.default".to_string(), json!(DEFAULT_OPENAI_MODEL));
    alias_map.insert(model_alias.to_string(), json!(DEFAULT_OPENAI_MODEL));

    json!({
        "openai": {
            "enabled": true,
            "api_token": api_key,
            "instances": [{
                "instance_id": OPENAI_INSTANCE_ID,
                "provider_type": OPENAI_PROVIDER_TYPE,
                "base_url": DEFAULT_OPENAI_BASE_URL,
                "timeout_ms": 120000_u64,
                "models": [DEFAULT_OPENAI_MODEL],
                "default_model": DEFAULT_OPENAI_MODEL,
                "features": ["plan", "json_output", "tool_calling", "web_search"],
            }],
            "alias_map": Value::Object(alias_map),
        }
    })
}

fn setup_local_center() -> (AIComputeCenter, Arc<CollectingSinkFactory>, String) {
    let registry = Registry::default();
    let catalog = ModelCatalog::default();
    let mut center = center_with_taskmgr(registry, catalog);
    let sink = Arc::new(CollectingSinkFactory::new());
    center.set_task_event_sink_factory(sink.clone());

    let model_alias = model_alias_from_env();
    let settings = build_local_openai_settings(model_alias.as_str());
    register_openai_llm_providers(&center, &settings).expect("register openai provider");
    (center, sink, model_alias)
}

fn complete_params(prompt: &str, must_features: Vec<&str>, options: Value, model_alias: &str) -> Value {
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
        "idempotency_key": format!("workflow-local-{}", now_seq())
    })
}

async fn complete_call(
    center: &AIComputeCenter,
    trace_id: &str,
    params: Value,
) -> Result<Value, String> {
    let req: CompleteRequest =
        serde_json::from_value(params).map_err(|err| format!("invalid complete params: {}", err))?;
    let mut ctx = rpc_ctx_with_tenant(None);
    ctx.trace_id = Some(trace_id.to_string());
    let resp = center
        .handle_complete(req, ctx)
        .await
        .map_err(|err| format!("local complete failed: {}", err))?;
    serde_json::to_value(resp).map_err(|err| format!("serialize complete result failed: {}", err))
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

fn format_task_error(sink: &CollectingSinkFactory, task_id: &str) -> String {
    let events = sink.events_for(task_id);
    if events.is_empty() {
        return "events=[]".to_string();
    }
    let last_error = events.iter().rev().find_map(|event| extract_error_message(event));
    let encoded_events =
        serde_json::to_string(&events).unwrap_or_else(|err| format!("event_serialize_failed:{err}"));
    match last_error {
        Some(msg) => format!("last_error={msg} events={encoded_events}"),
        None => format!("events={encoded_events}"),
    }
}

fn extract_error_message(event: &TaskEvent) -> Option<String> {
    let data = event.data.as_ref()?;
    let code = data.get("code").and_then(|v| v.as_str()).unwrap_or("unknown");
    let message = data
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    Some(format!("code={} message={}", code, message))
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn workflow_real_01_gateway_complex_scenario_protocol_mix() {
    let (center, sink, model_alias) = setup_local_center();

    let complex_result = complete_call(
        &center,
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
    .expect("complex dag local call failed");

    let complex_status = complex_result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let complex_task_id = complex_result
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        complex_status,
        "succeeded",
        "complex dag status={} result={} detail={}",
        complex_status,
        complex_result,
        format_task_error(&sink, complex_task_id)
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
        &center,
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
    .expect("json-output local call failed");
    let json_status = json_result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let json_task_id = json_result
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        json_status,
        "succeeded",
        "json-output status={} result={} detail={}",
        json_status,
        json_result,
        format_task_error(&sink, json_task_id)
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
        &center,
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
    .expect("stream local call failed");
    let stream_status = stream_result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let stream_task_id = stream_result
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        matches!(stream_status, "running" | "succeeded"),
        "stream status={} result={} detail={}",
        stream_status,
        stream_result,
        format_task_error(&sink, stream_task_id)
    );
}
