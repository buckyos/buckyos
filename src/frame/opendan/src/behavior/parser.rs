use serde_json::Value as Json;

use super::types::{BehaviorLLMResult, LLMOutput};
use super::LLMRawResponse;
use crate::agent_tool::ToolCall;

pub struct BehaviorResultParser;

impl BehaviorResultParser {
    pub fn parse_first(
        raw: &LLMRawResponse,
        force_json: bool,
    ) -> Result<(BehaviorLLMResult, LLMOutput), String> {
        if !raw.tool_calls.is_empty() {
            return Ok(step_output_from_function_calls(&raw.tool_calls));
        }
        Self::parse_final_content(&raw.content, force_json)
    }

    pub fn parse_followup(
        raw: &LLMRawResponse,
        force_json: bool,
    ) -> Result<(BehaviorLLMResult, LLMOutput), String> {
        if !raw.tool_calls.is_empty() {
            return Ok(step_output_from_function_calls(&raw.tool_calls));
        }
        Self::parse_final_content(&raw.content, force_json)
    }

    fn parse_final_content(
        content: &str,
        force_json: bool,
    ) -> Result<(BehaviorLLMResult, LLMOutput), String> {
        if let Ok(value) = serde_json::from_str::<Json>(content) {
            return Self::from_json(value);
        }

        if force_json {
            if let Some(extracted) = try_extract_json_block(content) {
                let value = serde_json::from_str::<Json>(&extracted)
                    .map_err(|err| format!("json extract ok but parse failed: {err}"))?;
                return Self::from_json(value);
            }
            return Err("force_json enabled but failed to parse JSON".to_string());
        }

        Ok((BehaviorLLMResult::default(), LLMOutput::Text(content.to_string())))
    }

    fn from_json(value: Json) -> Result<(BehaviorLLMResult, LLMOutput), String> {
        let mut result = match serde_json::from_value::<BehaviorLLMResult>(value.clone()) {
            Ok(parsed) => parsed,
            Err(err) => {
                // Some stage behaviors (for example resolve_router) emit
                // business-shaped JSON instead of BehaviorLLMResult.
                if !contains_non_executor_fields(&value) {
                    return Err(format!("invalid behavior output schema: {err}"));
                }
                BehaviorLLMResult::default()
            }
        };

        if result.actions.is_empty() {
            if let Some(actions_value) = value.get("actions") {
                if let Ok(actions) = serde_json::from_value(actions_value.clone()) {
                    result.actions = actions;
                }
            }
        }

        if result.next_behavior.is_none() {
            if let Some(next) = value
                .get("next_behavior")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
            {
                result.next_behavior = Some(next);
            } else if value
                .get("is_sleep")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                result.next_behavior = Some("END".to_string());
            }
        }

        let raw_output = if let Some(output_value) = value.get("output") {
            json_value_to_llm_output(output_value.clone())
        } else {
            LLMOutput::Json(value)
        };

        if result.tool_calls.is_empty() {
            if let LLMOutput::Json(Json::Object(obj)) = &raw_output {
                if let Some(tool_calls_value) = obj.get("tool_calls") {
                    if let Ok(tool_calls) = serde_json::from_value(tool_calls_value.clone()) {
                        result.tool_calls = tool_calls;
                    }
                }
            }
        }

        Ok((result, raw_output))
    }
}

fn json_value_to_llm_output(value: Json) -> LLMOutput {
    match value {
        Json::String(text) => LLMOutput::Text(text),
        Json::Null => LLMOutput::Json(Json::Null),
        other => LLMOutput::Json(other),
    }
}

fn contains_non_executor_fields(value: &Json) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.keys().any(|k| {
        !matches!(
            k.as_str(),
            "next_behavior"
                | "thinking"
                | "reply"
                | "tool_calls"
                | "todo"
                | "set_memory"
                | "actions"
                | "session_delta"
                | "is_sleep"
                | "output"
        )
    })
}

fn try_extract_json_block(content: &str) -> Option<String> {
    let fence_parts: Vec<&str> = content.split("```").collect();
    if fence_parts.len() >= 3 {
        for segment in fence_parts.iter().skip(1).step_by(2) {
            let trimmed = segment.trim();
            let payload = if let Some(rest) = trimmed.strip_prefix("json") {
                rest.trim()
            } else {
                trimmed
            };
            if serde_json::from_str::<Json>(payload).is_ok() {
                return Some(payload.to_string());
            }
        }
    }

    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }

    let candidate = &content[start..=end];
    if serde_json::from_str::<Json>(candidate).is_ok() {
        return Some(candidate.to_string());
    }

    None
}

fn step_output_from_function_calls(tool_calls: &[ToolCall]) -> (BehaviorLLMResult, LLMOutput) {
    let result = BehaviorLLMResult {
        tool_calls: tool_calls.to_vec(),
        ..Default::default()
    };
    let output = serde_json::to_value(&result)
        .map(LLMOutput::Json)
        .unwrap_or_else(|_| LLMOutput::Text(String::new()));
    (result, output)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value as Json};

    use super::*;
    use crate::agent_tool::ToolCall;

    fn raw_response(content: &str, tool_calls: Vec<ToolCall>) -> LLMRawResponse {
        LLMRawResponse {
            content: content.to_string(),
            tool_calls,
            model: "test-model".to_string(),
            provider: "test-provider".to_string(),
            latency_ms: 1,
        }
    }

    #[test]
    fn parse_first_short_circuit_when_tool_calls_present() {
        let call = ToolCall {
            name: "tool.echo".to_string(),
            args: json!({ "msg": "hello" }),
            call_id: "call-1".to_string(),
        };
        let raw = raw_response("not-json", vec![call.clone()]);

        let (parsed, output) =
            BehaviorResultParser::parse_first(&raw, true).expect("parse_first should succeed");
        assert_eq!(parsed.tool_calls, vec![call]);
        assert!(parsed.actions.is_empty());
        assert!(matches!(output, LLMOutput::Json(_)));
        assert_eq!(parsed.next_behavior, None);
    }

    #[test]
    fn parse_followup_short_circuit_when_tool_calls_present() {
        let call = ToolCall {
            name: "tool.read".to_string(),
            args: json!({ "path": "/tmp/a.txt" }),
            call_id: "call-2".to_string(),
        };
        let raw = raw_response("still-not-json", vec![call.clone()]);

        let (parsed, output) =
            BehaviorResultParser::parse_followup(&raw, false).expect("parse_followup should succeed");
        assert_eq!(parsed.tool_calls, vec![call]);
        assert!(matches!(output, LLMOutput::Json(_)));
    }

    #[test]
    fn parse_plain_text_when_force_json_disabled() {
        let raw = raw_response("plain text output", vec![]);

        let (parsed, output) =
            BehaviorResultParser::parse_first(&raw, false).expect("plain text parsing should succeed");
        assert!(parsed.tool_calls.is_empty());
        assert!(parsed.actions.is_empty());
        assert_eq!(
            output,
            LLMOutput::Text("plain text output".to_string())
        );
        assert_eq!(parsed.next_behavior, None);
    }

    #[test]
    fn parse_json_from_code_fence_when_force_json_enabled() {
        let content = "prefix\n```json\n{\"output\":\"ok\",\"is_sleep\":true}\n```\nsuffix";
        let raw = raw_response(content, vec![]);

        let (parsed, output) =
            BehaviorResultParser::parse_first(&raw, true).expect("json fence parse should work");
        assert_eq!(output, LLMOutput::Text("ok".to_string()));
        assert_eq!(parsed.next_behavior.as_deref(), Some("END"));
    }

    #[test]
    fn parse_json_from_brace_slice_when_force_json_enabled() {
        let content = "result => {\"output\":{\"v\":1},\"next_behavior\":\"NEXT\"} trailing";
        let raw = raw_response(content, vec![]);

        let (parsed, output) =
            BehaviorResultParser::parse_first(&raw, true).expect("brace-slice parse should work");
        assert_eq!(output, LLMOutput::Json(json!({ "v": 1 })));
        assert_eq!(parsed.next_behavior.as_deref(), Some("NEXT"));
    }

    #[test]
    fn force_json_returns_error_when_no_json_found() {
        let raw = raw_response("no json here", vec![]);

        let err = BehaviorResultParser::parse_first(&raw, true).expect_err("should fail");
        assert!(err.contains("force_json enabled"));
    }

    #[test]
    fn parse_behavior_result_fills_output_and_infers_end_behavior() {
        let payload = json!({
            "next_behavior": "END",
            "thinking": "analyze",
            "reply": [{
                "audience": "user",
                "format": "markdown",
                "content": "done"
            }],
            "tool_calls": [],
            "todo": [],
            "set_memory": [],
            "actions": [],
            "session_delta": []
        });
        let raw = raw_response(&payload.to_string(), vec![]);

        let (parsed, output) =
            BehaviorResultParser::parse_first(&raw, true).expect("behavior parse should work");
        assert_eq!(parsed.next_behavior.as_deref(), Some("END"));
        assert_eq!(output, LLMOutput::Json(payload));
    }

    #[test]
    fn parse_router_style_json_as_raw_output() {
        let payload = json!({
            "session_id": "session-router-1",
            "new_session": null,
            "next_behavior": "on_wakeup",
            "memory_queries": ["project status", "todo follow-up"],
            "reply": "收到，先整理项目状态。"
        });
        let raw = raw_response(&payload.to_string(), vec![]);

        let (parsed, output) =
            BehaviorResultParser::parse_first(&raw, true).expect("router-style parse should work");
        assert_eq!(parsed.next_behavior.as_deref(), Some("on_wakeup"));
        assert_eq!(output, LLMOutput::Json(payload));
    }

    #[test]
    fn parse_executor_tool_calls_only_json() {
        let raw = raw_response(
            &json!({
                "tool_calls": [{
                    "name": "tool.echo",
                    "args": {"msg":"hello"},
                    "call_id": "executor-call-1"
                }]
            })
            .to_string(),
            vec![],
        );

        let (parsed, _) = BehaviorResultParser::parse_first(&raw, true).expect("parse should succeed");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "tool.echo");
        assert_eq!(parsed.tool_calls[0].call_id, "executor-call-1");
    }

    #[test]
    fn explicit_is_sleep_false_overrides_executor_stop_flag() {
        let raw = raw_response(
            &json!({
                "is_sleep": false,
                "thinking": "analyze",
                "reply": [{
                    "audience": "user",
                    "format": "markdown",
                    "content": "done"
                }],
                "tool_calls": [],
                "todo": [],
                "set_memory": [],
                "actions": [],
                "session_delta": []
            })
            .to_string(),
            vec![],
        );

        let (parsed, _) = BehaviorResultParser::parse_first(&raw, true).expect("parse should succeed");
        assert_eq!(parsed.next_behavior, None);
    }

    #[test]
    fn invalid_schema_returns_error() {
        let raw = raw_response(
            &json!({
                "output": "ok",
                "actions": [{
                    "kind": "bash"
                }]
            })
            .to_string(),
            vec![],
        );

        let err = BehaviorResultParser::parse_first(&raw, true).expect_err("schema should fail");
        assert!(err.contains("invalid behavior output schema"));
    }

    #[test]
    fn try_extract_json_block_prefers_valid_fence_segment() {
        let input = "```txt\nnot-json\n```\n```json\n{\"a\":1}\n```";
        let extracted = try_extract_json_block(input).expect("should extract");
        let value: Json = serde_json::from_str(&extracted).expect("valid json");
        assert_eq!(value, json!({ "a": 1 }));
    }
}
