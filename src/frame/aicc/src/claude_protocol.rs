#![allow(dead_code)]

use crate::aicc::ProviderError;
use buckyos_api::{CompleteRequest, ResourceRef};
use serde_json::{json, Map, Value};

const CLAUDE_OPTION_ALLOWLIST: &[&str] = &[
    "max_tokens",
    "metadata",
    "stop_sequences",
    "stream",
    "temperature",
    "tool_choice",
    "tools",
    "top_k",
    "top_p",
    "thinking",
    "system",
];
const CLAUDE_TOOL_NAME_PATTERN: &str = "^[a-zA-Z0-9_-]+$";
const DEFAULT_MAX_TOKENS: u64 = 1024;

fn is_valid_claude_tool_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-')
}

fn validate_claude_tool_name(raw_name: &str, field_path: &str) -> Result<String, ProviderError> {
    let name = raw_name.trim();
    if !is_valid_claude_tool_name(name) {
        return Err(ProviderError::fatal(format!(
            "{} is invalid; expected pattern '{}'",
            field_path, CLAUDE_TOOL_NAME_PATTERN
        )));
    }
    Ok(name.to_string())
}

fn default_tool_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true,
    })
}

fn build_claude_tool(
    raw_name: &str,
    description: Option<&str>,
    input_schema: Value,
    field_path: &str,
) -> Result<Value, ProviderError> {
    let name = validate_claude_tool_name(raw_name, field_path)?;

    let mut normalized = Map::new();
    normalized.insert("name".to_string(), Value::String(name));

    if let Some(description) = description
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        normalized.insert(
            "description".to_string(),
            Value::String(description.to_string()),
        );
    }

    normalized.insert("input_schema".to_string(), input_schema);
    Ok(Value::Object(normalized))
}

fn convert_internal_tool(tool: &Map<String, Value>, idx: usize) -> Result<Value, ProviderError> {
    let Some(raw_name) = tool
        .get("name")
        .and_then(|value| value.as_str())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Err(ProviderError::fatal(format!(
            "tools[{}].name is required for internal tool format",
            idx
        )));
    };

    let input_schema = tool
        .get("args_schema")
        .cloned()
        .unwrap_or_else(default_tool_input_schema);
    if !input_schema.is_object() {
        return Err(ProviderError::fatal(format!(
            "tools[{}].args_schema must be an object",
            idx
        )));
    }

    build_claude_tool(
        raw_name,
        tool.get("description").and_then(|value| value.as_str()),
        input_schema,
        format!("tools[{}].name", idx).as_str(),
    )
}

fn normalize_openai_function_tool(
    tool: &Map<String, Value>,
    idx: usize,
) -> Result<Value, ProviderError> {
    let Some(function_obj) = tool.get("function").and_then(|value| value.as_object()) else {
        return Err(ProviderError::fatal(format!(
            "tools[{}].function is required when tools[{}].type=function",
            idx, idx
        )));
    };

    let Some(raw_name) = function_obj.get("name").and_then(|value| value.as_str()) else {
        return Err(ProviderError::fatal(format!(
            "tools[{}].function.name is required",
            idx
        )));
    };

    let input_schema = function_obj
        .get("parameters")
        .cloned()
        .unwrap_or_else(default_tool_input_schema);
    if !input_schema.is_object() {
        return Err(ProviderError::fatal(format!(
            "tools[{}].function.parameters must be an object",
            idx
        )));
    }

    build_claude_tool(
        raw_name,
        function_obj
            .get("description")
            .and_then(|value| value.as_str()),
        input_schema,
        format!("tools[{}].function.name", idx).as_str(),
    )
}

fn normalize_claude_tool(tool: &Map<String, Value>, idx: usize) -> Result<Value, ProviderError> {
    let Some(raw_name) = tool.get("name").and_then(|value| value.as_str()) else {
        return Err(ProviderError::fatal(format!(
            "tools[{}].name is required",
            idx
        )));
    };

    let input_schema = tool
        .get("input_schema")
        .cloned()
        .unwrap_or_else(default_tool_input_schema);
    if !input_schema.is_object() {
        return Err(ProviderError::fatal(format!(
            "tools[{}].input_schema must be an object",
            idx
        )));
    }

    build_claude_tool(
        raw_name,
        tool.get("description").and_then(|value| value.as_str()),
        input_schema,
        format!("tools[{}].name", idx).as_str(),
    )
}

fn normalize_tools_option(tools: &Value) -> Result<Value, ProviderError> {
    let Some(items) = tools.as_array() else {
        return Err(ProviderError::fatal("tools must be an array"));
    };

    let mut normalized = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(tool_obj) = item.as_object() else {
            return Err(ProviderError::fatal(format!(
                "tools[{}] must be an object",
                idx
            )));
        };

        let converted = match tool_obj.get("type").and_then(|value| value.as_str()) {
            Some("function") => normalize_openai_function_tool(tool_obj, idx)?,
            Some(other) => {
                return Err(ProviderError::fatal(format!(
                    "tools[{}].type '{}' is unsupported; only 'function' is supported",
                    idx, other
                )));
            }
            None => {
                if tool_obj.contains_key("args_schema") {
                    convert_internal_tool(tool_obj, idx)?
                } else {
                    normalize_claude_tool(tool_obj, idx)?
                }
            }
        };
        normalized.push(converted);
    }

    Ok(Value::Array(normalized))
}

fn normalize_stop_sequences_option(stop: &Value) -> Result<Value, ProviderError> {
    if let Some(stop_str) = stop.as_str() {
        return Ok(Value::Array(vec![Value::String(stop_str.to_string())]));
    }

    let Some(stop_values) = stop.as_array() else {
        return Err(ProviderError::fatal(
            "stop must be a string or array of strings",
        ));
    };

    let mut normalized = Vec::with_capacity(stop_values.len());
    for (idx, item) in stop_values.iter().enumerate() {
        let Some(stop_str) = item.as_str() else {
            return Err(ProviderError::fatal(format!(
                "stop[{}] must be a string",
                idx
            )));
        };
        normalized.push(Value::String(stop_str.to_string()));
    }

    Ok(Value::Array(normalized))
}

fn normalize_tool_choice_option(tool_choice: &Value) -> Result<Value, ProviderError> {
    if let Some(choice) = tool_choice.as_str() {
        return match choice {
            "auto" => Ok(json!({ "type": "auto" })),
            "required" | "any" => Ok(json!({ "type": "any" })),
            "none" => Ok(json!({ "type": "none" })),
            other => Err(ProviderError::fatal(format!(
                "tool_choice '{}' is unsupported",
                other
            ))),
        };
    }

    let Some(choice_obj) = tool_choice.as_object() else {
        return Err(ProviderError::fatal(
            "tool_choice must be a string or object",
        ));
    };

    let Some(choice_type) = choice_obj.get("type").and_then(|value| value.as_str()) else {
        return Err(ProviderError::fatal("tool_choice.type is required"));
    };

    match choice_type {
        "function" => {
            let Some(function_obj) = choice_obj
                .get("function")
                .and_then(|value| value.as_object())
            else {
                return Err(ProviderError::fatal(
                    "tool_choice.function is required when tool_choice.type=function",
                ));
            };

            let Some(raw_name) = function_obj.get("name").and_then(|value| value.as_str()) else {
                return Err(ProviderError::fatal(
                    "tool_choice.function.name is required",
                ));
            };
            let name = validate_claude_tool_name(raw_name, "tool_choice.function.name")?;

            Ok(json!({
                "type": "tool",
                "name": name,
            }))
        }
        "tool" => {
            let Some(raw_name) = choice_obj.get("name").and_then(|value| value.as_str()) else {
                return Err(ProviderError::fatal(
                    "tool_choice.name is required when type=tool",
                ));
            };
            let name = validate_claude_tool_name(raw_name, "tool_choice.name")?;
            Ok(json!({
                "type": "tool",
                "name": name,
            }))
        }
        "auto" | "any" | "none" => Ok(json!({ "type": choice_type })),
        other => Err(ProviderError::fatal(format!(
            "tool_choice.type '{}' is unsupported",
            other
        ))),
    }
}

fn text_content_block(text: &str) -> Value {
    json!({
        "type": "text",
        "text": text,
    })
}

fn text_message(role: &str, text: &str) -> Value {
    json!({
        "role": role,
        "content": [text_content_block(text)],
    })
}

fn parse_tool_calls_content(content: &str, idx: usize) -> Result<Option<Value>, ProviderError> {
    let Ok(parsed) = serde_json::from_str::<Value>(content) else {
        return Ok(None);
    };

    let Some(tool_calls) = parsed.get("tool_calls") else {
        return Ok(None);
    };

    let Some(tool_calls_array) = tool_calls.as_array() else {
        return Err(ProviderError::fatal(format!(
            "tool_messages[{}].content.tool_calls must be an array",
            idx
        )));
    };

    let mut content_blocks = vec![];
    for (call_idx, tool_call) in tool_calls_array.iter().enumerate() {
        let Some(call_obj) = tool_call.as_object() else {
            return Err(ProviderError::fatal(format!(
                "tool_messages[{}].content.tool_calls[{}] must be an object",
                idx, call_idx
            )));
        };

        let Some(raw_name) = call_obj.get("name").and_then(|value| value.as_str()) else {
            return Err(ProviderError::fatal(format!(
                "tool_messages[{}].content.tool_calls[{}].name is required",
                idx, call_idx
            )));
        };
        let name = validate_claude_tool_name(
            raw_name,
            format!(
                "tool_messages[{}].content.tool_calls[{}].name",
                idx, call_idx
            )
            .as_str(),
        )?;

        let Some(call_id) = call_obj.get("call_id").and_then(|value| value.as_str()) else {
            return Err(ProviderError::fatal(format!(
                "tool_messages[{}].content.tool_calls[{}].call_id is required",
                idx, call_idx
            )));
        };

        let input = call_obj
            .get("args")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        if !input.is_object() {
            return Err(ProviderError::fatal(format!(
                "tool_messages[{}].content.tool_calls[{}].args must be an object",
                idx, call_idx
            )));
        }

        content_blocks.push(json!({
            "type": "tool_use",
            "id": call_id,
            "name": name,
            "input": input,
        }));
    }

    if content_blocks.is_empty() {
        Ok(None)
    } else {
        Ok(Some(json!({
            "role": "assistant",
            "content": content_blocks,
        })))
    }
}

fn convert_tool_messages_option(tool_messages: &Value) -> Result<Vec<Value>, ProviderError> {
    let Some(items) = tool_messages.as_array() else {
        return Err(ProviderError::fatal("tool_messages must be an array"));
    };

    let mut converted = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(msg_obj) = item.as_object() else {
            return Err(ProviderError::fatal(format!(
                "tool_messages[{}] must be an object",
                idx
            )));
        };

        let Some(role) = msg_obj.get("role").and_then(|value| value.as_str()) else {
            return Err(ProviderError::fatal(format!(
                "tool_messages[{}].role is required",
                idx
            )));
        };
        let content = msg_obj
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let content = content.trim();
        if content.is_empty() {
            continue;
        }

        match role {
            "assistant" => {
                if let Some(tool_use_message) = parse_tool_calls_content(content, idx)? {
                    converted.push(tool_use_message);
                } else {
                    converted.push(text_message("assistant", content));
                }
            }
            "tool" => {
                let tool_result = serde_json::from_str::<Value>(content)
                    .ok()
                    .and_then(|parsed| parsed.as_object().cloned())
                    .and_then(|parsed_obj| {
                        parsed_obj
                            .get("call_id")
                            .and_then(|value| value.as_str())
                            .map(|call_id| {
                                let result_content = parsed_obj
                                    .get("content")
                                    .cloned()
                                    .unwrap_or_else(|| Value::Object(parsed_obj.clone()));
                                (call_id.to_string(), result_content)
                            })
                    });

                if let Some((call_id, result_content)) = tool_result {
                    let result_text = if let Some(text) = result_content.as_str() {
                        text.to_string()
                    } else {
                        serde_json::to_string(&result_content).unwrap_or_else(|_| "{}".to_string())
                    };
                    converted.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": call_id,
                            "content": result_text,
                        }],
                    }));
                } else {
                    let prefix = msg_obj
                        .get("name")
                        .and_then(|value| value.as_str())
                        .map(|name| format!("tool[{}]: ", name))
                        .unwrap_or_default();
                    converted.push(text_message(
                        "user",
                        format!("{}{}", prefix, content).as_str(),
                    ));
                }
            }
            "user" => converted.push(text_message("user", content)),
            "system" => converted.push(text_message("user", content)),
            "assistant_tool" => converted.push(text_message("assistant", content)),
            other => {
                return Err(ProviderError::fatal(format!(
                    "tool_messages[{}].role '{}' is unsupported",
                    idx, other
                )));
            }
        }
    }

    Ok(converted)
}

fn merge_system_prompt(target: &mut Map<String, Value>, system: Value) {
    if !target.contains_key("system") {
        target.insert("system".to_string(), system);
        return;
    }

    let merged = match (target.get("system"), system) {
        (Some(Value::String(existing)), Value::String(incoming)) if !incoming.trim().is_empty() => {
            Value::String(format!("{}\n\n{}", existing, incoming))
        }
        (_, incoming) => incoming,
    };
    target.insert("system".to_string(), merged);
}

pub(crate) fn merge_options(
    target: &mut Map<String, Value>,
    options: &Value,
) -> Result<(Vec<String>, Vec<Value>), ProviderError> {
    let Some(options_map) = options.as_object() else {
        return Ok((vec![], vec![]));
    };

    let mut ignored = vec![];
    let mut extra_messages = vec![];

    for (key, value) in options_map.iter() {
        if key == "model" || key == "messages" {
            continue;
        }
        if key == "protocol" || key == "process_name" {
            ignored.push(key.clone());
            continue;
        }
        if key == "tool_messages" {
            extra_messages.extend(convert_tool_messages_option(value)?);
            continue;
        }
        if key == "response_schema" {
            ignored.push(key.clone());
            continue;
        }
        if key == "max_completion_tokens" {
            if !target.contains_key("max_tokens") {
                target.insert("max_tokens".to_string(), value.clone());
            }
            continue;
        }
        if key == "stop" {
            target.insert(
                "stop_sequences".to_string(),
                normalize_stop_sequences_option(value)?,
            );
            continue;
        }
        if key == "tools" {
            target.insert("tools".to_string(), normalize_tools_option(value)?);
            continue;
        }
        if key == "tool_choice" {
            target.insert(
                "tool_choice".to_string(),
                normalize_tool_choice_option(value)?,
            );
            continue;
        }
        if key == "system" {
            merge_system_prompt(target, value.clone());
            continue;
        }
        if !CLAUDE_OPTION_ALLOWLIST.contains(&key.as_str()) {
            ignored.push(key.clone());
            continue;
        }
        target.insert(key.clone(), value.clone());
    }

    Ok((ignored, extra_messages))
}

fn build_fallback_content(req: &CompleteRequest) -> Result<Option<String>, ProviderError> {
    let mut content = req
        .payload
        .text
        .as_ref()
        .map(|text| text.trim().to_string())
        .unwrap_or_default();

    let mut resource_lines = vec![];
    for resource in req.payload.resources.iter() {
        match resource {
            ResourceRef::Url { url, .. } => {
                resource_lines.push(format!("resource_url: {}", url));
            }
            ResourceRef::NamedObject { obj_id } => {
                resource_lines.push(format!("named_object: {}", obj_id));
            }
            ResourceRef::Base64 { .. } => {
                return Err(ProviderError::fatal(
                    "claude provider does not support base64 resources in this version",
                ));
            }
        }
    }

    if !resource_lines.is_empty() {
        if !content.is_empty() {
            content.push('\n');
            content.push('\n');
        }
        content.push_str(resource_lines.join("\n").as_str());
    }

    if content.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(content))
    }
}

fn build_messages(req: &CompleteRequest) -> Result<(Option<String>, Vec<Value>), ProviderError> {
    let mut system_parts = vec![];
    let mut messages = vec![];

    for msg in req.payload.messages.iter() {
        let role = msg.role.trim().to_lowercase();
        let content = msg.content.trim();
        if role.is_empty() || content.is_empty() {
            continue;
        }

        match role.as_str() {
            "system" => {
                system_parts.push(content.to_string());
            }
            "user" => {
                messages.push(text_message("user", content));
            }
            "assistant" => {
                messages.push(text_message("assistant", content));
            }
            "tool" => {
                messages.push(text_message("user", format!("tool: {}", content).as_str()));
            }
            other => {
                messages.push(text_message(
                    "user",
                    format!("{}: {}", other, content).as_str(),
                ));
            }
        }
    }

    if messages.is_empty() {
        if let Some(fallback_content) = build_fallback_content(req)? {
            messages.push(text_message("user", fallback_content.as_str()));
        }
    }

    if messages.is_empty() {
        return Err(ProviderError::fatal(
            "request payload has no usable text/messages for llm",
        ));
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    Ok((system, messages))
}

fn resolve_provider_model(req: &CompleteRequest, provider_model: &str) -> Option<String> {
    if !provider_model.trim().is_empty() {
        return Some(provider_model.trim().to_string());
    }

    req.model
        .provider_model_hint
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| {
            if req.model.alias.trim().is_empty() {
                None
            } else {
                Some(req.model.alias.trim().to_string())
            }
        })
}

pub(crate) fn convert_complete_request(
    req: &CompleteRequest,
    provider_model: &str,
) -> Result<(Map<String, Value>, Vec<String>), ProviderError> {
    let model = resolve_provider_model(req, provider_model)
        .ok_or_else(|| ProviderError::fatal("provider model is required for claude request"))?;

    let (system, messages) = build_messages(req)?;

    let mut request = Map::new();
    request.insert("model".to_string(), Value::String(model));
    request.insert("messages".to_string(), Value::Array(messages));
    if let Some(system) = system {
        request.insert("system".to_string(), Value::String(system));
    }

    let mut ignored = vec![];
    let mut extra_messages = vec![];
    if let Some(options) = req.payload.options.as_ref() {
        let (ignored_options, converted_tool_messages) = merge_options(&mut request, options)?;
        ignored = ignored_options;
        extra_messages = converted_tool_messages;
    }

    if !extra_messages.is_empty() {
        if let Some(message_array) = request
            .get_mut("messages")
            .and_then(|value| value.as_array_mut())
        {
            message_array.extend(extra_messages);
        }
    }

    if !request.contains_key("max_tokens") {
        request.insert("max_tokens".to_string(), Value::from(DEFAULT_MAX_TOKENS));
    }

    Ok((request, ignored))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{AiMessage, AiPayload, Capability, CompleteRequest, ModelSpec, Requirements};

    fn base_request() -> CompleteRequest {
        CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new(
                "llm.default".to_string(),
                Some("claude-3-5-sonnet-20241022".to_string()),
            ),
            Requirements::default(),
            AiPayload::new(None, vec![], vec![], None, None),
            None,
        )
    }

    #[test]
    fn convert_complete_request_maps_messages_and_tool_messages() {
        let mut req = base_request();
        req.payload.messages = vec![
            AiMessage::new("system".to_string(), "system rules".to_string()),
            AiMessage::new("user".to_string(), "hello".to_string()),
        ];
        req.payload.options = Some(json!({
            "max_completion_tokens": 333,
            "temperature": 0.2,
            "protocol": "opendan_llm_behavior_v1",
            "tool_messages": [
                {
                    "role": "assistant",
                    "name": "tool_calls",
                    "content": "{\"tool_calls\":[{\"name\":\"load_memory\",\"args\":{\"topic\":\"project\"},\"call_id\":\"call-1\"}]}"
                },
                {
                    "role": "tool",
                    "name": "load_memory",
                    "content": "{\"call_id\":\"call-1\",\"content\":\"ok\"}"
                }
            ]
        }));

        let (request, ignored) = convert_complete_request(&req, "claude-3-7-sonnet-20250219")
            .expect("convert should work");
        let request_value = Value::Object(request);

        assert_eq!(
            request_value.get("model").and_then(|value| value.as_str()),
            Some("claude-3-7-sonnet-20250219")
        );
        assert_eq!(request_value.get("max_tokens"), Some(&json!(333)));
        assert_eq!(
            request_value
                .get("temperature")
                .and_then(|value| value.as_f64()),
            Some(0.2)
        );
        assert_eq!(
            request_value.get("system").and_then(|value| value.as_str()),
            Some("system rules")
        );
        assert_eq!(
            request_value
                .pointer("/messages/0/role")
                .and_then(|value| value.as_str()),
            Some("user")
        );
        assert_eq!(
            request_value
                .pointer("/messages/1/content/0/type")
                .and_then(|value| value.as_str()),
            Some("tool_use")
        );
        assert_eq!(
            request_value
                .pointer("/messages/2/content/0/type")
                .and_then(|value| value.as_str()),
            Some("tool_result")
        );

        assert!(ignored.iter().any(|item| item == "protocol"));
    }

    #[test]
    fn convert_complete_request_builds_fallback_message_from_text_and_resources() {
        let mut req = base_request();
        req.payload.text = Some("summarize updates".to_string());
        req.payload.resources = vec![ResourceRef::url(
            "https://example.com/doc".to_string(),
            Some("text/plain".to_string()),
        )];

        let (request, _ignored) = convert_complete_request(&req, "claude-3-5-haiku-20241022")
            .expect("convert should work");
        let request_value = Value::Object(request);

        assert_eq!(
            request_value
                .pointer("/messages/0/content/0/text")
                .and_then(|value| value.as_str()),
            Some("summarize updates\n\nresource_url: https://example.com/doc")
        );
    }

    #[test]
    fn merge_options_converts_internal_tools_to_claude_format() {
        let options = json!({
            "temperature": 0.2,
            "tools": [
                {
                    "name": "load_memory",
                    "description": "Load memory",
                    "args_schema": {
                        "type": "object",
                        "properties": {
                            "token_limit": { "type": "integer" }
                        }
                    }
                }
            ]
        });

        let mut target = Map::new();
        let (ignored, _) = merge_options(&mut target, &options).expect("merge options should work");
        let target_value = Value::Object(target.clone());

        assert_eq!(target.get("temperature"), Some(&json!(0.2)));
        assert_eq!(
            target_value
                .pointer("/tools/0/name")
                .and_then(|value| value.as_str()),
            Some("load_memory")
        );
        assert_eq!(
            target_value.pointer("/tools/0/input_schema"),
            Some(&json!({
                "type": "object",
                "properties": {
                    "token_limit": { "type": "integer" }
                }
            }))
        );
        assert!(ignored.is_empty());
    }

    #[test]
    fn merge_options_accepts_openai_function_tool_and_fills_default_input_schema() {
        let options = json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "workshop_exec_bash",
                        "description": "Run command"
                    }
                }
            ]
        });

        let mut target = Map::new();
        merge_options(&mut target, &options).expect("merge options should work");
        let target_value = Value::Object(target.clone());

        assert_eq!(
            target_value
                .pointer("/tools/0/name")
                .and_then(|value| value.as_str()),
            Some("workshop_exec_bash")
        );
        assert_eq!(
            target_value.pointer("/tools/0/input_schema"),
            Some(&json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            }))
        );
    }

    #[test]
    fn merge_options_rejects_invalid_tool_name() {
        let options = json!({
            "tools": [
                {
                    "name": "workshop.exec_bash",
                    "args_schema": { "type": "object" }
                }
            ]
        });

        let mut target = Map::new();
        let err = merge_options(&mut target, &options).expect_err("merge options should fail");
        assert!(
            err.to_string()
                .contains("tools[0].name is invalid; expected pattern '^[a-zA-Z0-9_-]+$'"),
            "unexpected err: {}",
            err
        );
    }

    #[test]
    fn merge_options_maps_stop_to_stop_sequences() {
        let options = json!({
            "stop": ["END", "STOP"]
        });

        let mut target = Map::new();
        merge_options(&mut target, &options).expect("merge options should work");

        assert_eq!(target.get("stop_sequences"), Some(&json!(["END", "STOP"])));
    }
}
