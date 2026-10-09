//! Model access of a wish run: the `.llm_context` of a stage (written as JSON, which YAML reads),
//! the deployment's provider, and `aiws.llm.map` (许愿格 §9.5) — per-item judgements, validated
//! against a schema, cached per deployment by (model, instruction, schema, item).

use super::{StageState, WishConfig};
use agent_tool::xllm::{DefaultLlmClientFactory, LlmClientFactory, ProviderConfig, ProviderKind, SecretRef};
use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use buckyos_api::{AiMessage, AiRole};
use llm_context::{InferenceAbortToken, LlmClient, LlmInferenceRequest};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

/// The provider block of the deployment configuration → the SDK's provider configuration.
pub fn provider_config(v: &Value) -> ProviderConfig {
    let kind = match v.get("type").and_then(Value::as_str) {
        Some("openai") => Some(ProviderKind::Openai),
        _ => Some(ProviderKind::Buckyos),
    };
    let mut p = ProviderConfig { kind, base_url: v.get("base_url").and_then(Value::as_str).map(str::to_string), ..Default::default() };
    if let Some(e) = v.get("api_key_env").and_then(Value::as_str) {
        p.api_key = Some(SecretRef::Env { name: e.to_string() });
    }
    if let Some(h) = v.get("headers").and_then(Value::as_object) {
        for (k, x) in h {
            if let Some(s) = x.as_str() {
                p.headers.insert(k.clone(), s.to_string());
            }
        }
    }
    p
}

/// `.llm_context` of one stage: everything explicit — provider, model, loop, budgets, run records,
/// and exactly the tools this stage offers.
pub fn llm_context(cfg: &WishConfig, model: &str, iterations: u32, runs_dir: &Path, tools: &[&str], shell: bool) -> Value {
    let mut list: Vec<Value> = Vec::new();
    if shell {
        list.push(json!({ "groupname": "bash" }));
    }
    for t in tools {
        list.push(json!({ "name": t }));
    }
    let mut provider = json!({ "type": cfg.provider.get("type").and_then(Value::as_str).unwrap_or("buckyos") });
    for k in ["base_url", "api_key_env", "headers"] {
        if let Some(v) = cfg.provider.get(k) {
            provider[k] = v.clone();
        }
    }
    let mut v = json!({
        "provider": provider,
        "model": model,
        "loop_model": "function_call",
        "max_tokens": cfg.max_output_tokens,
        "max_tool_iterations": iterations,
        "timeout": cfg.stage_timeout_secs,
        "runs_dir": runs_dir.display().to_string(),
        "run_logs": "warn",
        "tools": {
            "enabled": true,
            "filesystem_policy": "workspace",
            "tools": list,
            "shell": { "mode": "wait", "timeout_ms": cfg.program_timeout_secs * 1000 + 30_000, "max_timeout_ms": cfg.program_timeout_secs * 2000 + 60_000 },
        },
    });
    if let Some(w) = cfg.context_window {
        v["context_window"] = json!(w);
    }
    v
}

pub async fn client(cfg: &WishConfig) -> Result<Arc<dyn LlmClient>, String> {
    if let Some(c) = &cfg.llm_override {
        return Ok(c.clone());
    }
    DefaultLlmClientFactory.create(&provider_config(&cfg.provider)).await.map_err(|e| e.to_string())
}

// ---- schema subset: type / enum / properties / required / items

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.as_f64().is_some_and(|n| n.fract() == 0.0),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "null" => v.is_null(),
        _ => true,
    }
}

pub fn validate(schema: &Value, v: &Value) -> Result<(), String> {
    match schema.get("type") {
        Some(Value::String(t)) if !type_ok(t, v) => return Err(format!("expected {t}, got {v}")),
        Some(Value::Array(ts)) if !ts.iter().filter_map(Value::as_str).any(|t| type_ok(t, v)) => return Err(format!("expected one of {}, got {v}", Value::Array(ts.clone()))),
        _ => {}
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(v) {
            return Err(format!("{v} is not one of {}", Value::Array(e.clone())));
        }
    }
    if let (Some(props), Some(o)) = (schema.get("properties").and_then(Value::as_object), v.as_object()) {
        for (k, s) in props {
            if let Some(x) = o.get(k) {
                validate(s, x).map_err(|e| format!("{k}: {e}"))?;
            }
        }
    }
    if let (Some(req), Some(o)) = (schema.get("required").and_then(Value::as_array), v.as_object()) {
        for k in req.iter().filter_map(Value::as_str) {
            if !o.contains_key(k) {
                return Err(format!("missing {k}"));
            }
        }
    }
    if let (Some(items), Some(a)) = (schema.get("items"), v.as_array()) {
        for (i, x) in a.iter().enumerate() {
            validate(items, x).map_err(|e| format!("[{i}]: {e}"))?;
        }
    }
    Ok(())
}

/// The JSON a model answered (code fences and surrounding prose tolerated).
pub fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    let t = t.trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    for (open, close) in [('[', ']'), ('{', '}')] {
        if let (Some(a), Some(b)) = (t.find(open), t.rfind(close)) {
            if a < b {
                if let Ok(v) = serde_json::from_str::<Value>(&t[a..=b]) {
                    return Some(v);
                }
            }
        }
    }
    None
}

async fn ask(llm: &Arc<dyn LlmClient>, model: &str, system: &str, user: String) -> Result<String, String> {
    let req = LlmInferenceRequest {
        trace_id: None,
        messages: vec![AiMessage::text(AiRole::System, system), AiMessage::text(AiRole::User, user)],
        model_alias: model.to_string(),
        fallbacks: Vec::new(),
        temperature: Some(0.0),
        max_completion_tokens: Some(4096),
        force_json: false,
        json_schema: None,
        provider_options: None,
        disable_capabilities: Vec::new(),
        tool_specs: Vec::new(),
        allow_tool_calls: false,
        abort: InferenceAbortToken::noop(),
    };
    let r = llm.infer(req).await.map_err(|e| format!("model call failed: {e:?}"))?;
    Ok(r.message.text_content())
}

const MAP_SYSTEM: &str = "你是一个逐项判断器。按指令对每个输入项独立作出判断，只输出 JSON，不要解释。";

/// `POST …/wish-host/<token>/llm_map` `{ items, instruction, schema, batch? }` →
/// `{ results: [{ value } | { error }], calls, cached }`.
pub async fn llm_map(state: &Arc<StageState>, body: &Value) -> Value {
    let cfg = &state.runtime.config;
    let items: Vec<Value> = body["items"].as_array().cloned().unwrap_or_default();
    let instruction = body["instruction"].as_str().unwrap_or("").to_string();
    let schema = body.get("schema").cloned().unwrap_or(json!({ "type": "string" }));
    if instruction.trim().is_empty() {
        return json!({ "error": "llm.map needs an instruction" });
    }
    let used = state.map_items(items.len());
    if used > cfg.llm_map_max_items {
        return json!({ "error": format!("llm.map budget exceeded: at most {} items per run — narrow the range", cfg.llm_map_max_items) });
    }
    let model = cfg.map_model.clone();
    let cache_dir = state.ws_dir.join("cache").join("llm_map");
    let _ = std::fs::create_dir_all(&cache_dir);
    let key_of = |item: &Value| sha256_hex(canonical_json(&json!({ "m": model, "i": instruction, "s": schema, "x": item })).unwrap_or_default().as_bytes());
    let mut results: Vec<Option<Value>> = vec![None; items.len()];
    let mut cached = 0;
    let mut todo = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match std::fs::read_to_string(cache_dir.join(format!("{}.json", key_of(item)))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            Some(v) => {
                results[i] = Some(json!({ "value": v }));
                cached += 1;
            }
            None => todo.push(i),
        }
    }
    let llm = match client(cfg).await {
        Ok(c) => c,
        Err(e) => return json!({ "error": e }),
    };
    let batch = body["batch"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
    let mut calls = 0;
    let schema_text = schema.to_string();
    for chunk in todo.chunks(batch) {
        let list: Vec<&Value> = chunk.iter().map(|i| &items[*i]).collect();
        let user = format!(
            "指令：{instruction}\n每一项的输出必须符合 JSON schema：{schema_text}\n输入项（JSON 数组，共 {} 项）：\n{}\n\n只输出一个 JSON 数组，第 i 个元素是第 i 项的结果，长度必须是 {}。",
            list.len(),
            serde_json::to_string(&list).unwrap_or_default(),
            list.len()
        );
        calls += 1;
        let answer = ask(&llm, &model, MAP_SYSTEM, user).await;
        let parsed = answer.as_ref().ok().and_then(|t| extract_json(t)).and_then(|v| v.as_array().cloned()).filter(|a| a.len() == chunk.len());
        for (j, i) in chunk.iter().enumerate() {
            let v = parsed.as_ref().map(|a| a[j].clone());
            if let Some(v) = v.filter(|v| validate(&schema, v).is_ok()) {
                let _ = std::fs::write(cache_dir.join(format!("{}.json", key_of(&items[*i]))), v.to_string());
                results[*i] = Some(json!({ "value": v }));
            }
        }
    }
    // failures are retried one by one; what still fails is reported, never invented
    for i in 0..items.len() {
        if results[i].is_some() {
            continue;
        }
        calls += 1;
        let user = format!("指令：{instruction}\n输出必须符合 JSON schema：{schema_text}\n输入项：{}\n\n只输出这一项的 JSON 结果。", items[i]);
        let answer = ask(&llm, &model, MAP_SYSTEM, user).await;
        results[i] = Some(match answer.as_ref().ok().and_then(|t| extract_json(t).or_else(|| (schema["type"] == json!("string")).then(|| json!(t.trim())))) {
            Some(v) => match validate(&schema, &v) {
                Ok(()) => {
                    let _ = std::fs::write(cache_dir.join(format!("{}.json", key_of(&items[i]))), v.to_string());
                    json!({ "value": v })
                }
                Err(e) => json!({ "error": format!("invalid answer: {e}") }),
            },
            None => json!({ "error": answer.err().unwrap_or_else(|| "no JSON in the answer".into()) }),
        });
    }
    state.note_map(calls, cached);
    json!({ "results": results.into_iter().map(|r| r.unwrap_or(json!({ "error": "missing" }))).collect::<Vec<_>>(), "calls": calls, "cached": cached })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_subset_and_json_extraction() {
        let s = json!({ "type": "object", "properties": { "tag": { "enum": ["正面", "负面"] }, "n": { "type": ["number", "null"] } }, "required": ["tag"] });
        assert!(validate(&s, &json!({ "tag": "正面", "n": null })).is_ok());
        assert!(validate(&s, &json!({ "tag": "中性" })).is_err());
        assert!(validate(&s, &json!({ "n": 1 })).is_err());
        assert_eq!(extract_json("```json\n[1, 2]\n```"), Some(json!([1, 2])));
        assert_eq!(extract_json("结果如下：[\"a\"] 完"), Some(json!(["a"])));
    }
}
