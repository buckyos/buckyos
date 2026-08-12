#![allow(dead_code)]

use crate::aicc::ProviderError;
use buckyos_api::{
    AiContent, AiMessage, AiMethodRequest, AiRole, AiToolResultContent, AiToolSpec, ResourceRef,
};
use serde_json::{json, Map, Value};

/// Tool result lowering dialect.
///
/// `Claude` emits native `tool_use` / `tool_result` content blocks.
/// `MiniMax` exposes the Anthropic surface but per spec we degrade tool
/// results to plain user text — the upstream endpoint does not reliably
/// honor native tool_result blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtocolDialect {
    Claude,
    MiniMax,
}

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
const FALLBACK_MAX_TOKENS: u64 = 1024;

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
    mut input_schema: Value,
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

    if let Some(map) = input_schema.as_object_mut() {
        // Claude (like OpenAI) requires `input_schema` to declare
        // `"type":"object"` at the top level. Hand-authored tool specs
        // sometimes omit it — fill it in rather than letting the API
        // reject the whole request.
        map.entry("type".to_string())
            .or_insert_with(|| Value::String("object".to_string()));
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
        .or_else(|| tool.get("args_json_schema"))
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
        .or_else(|| function_obj.get("args_json_schema"))
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
            Some(other) if is_claude_server_tool_type(other) => {
                // Anthropic-native server-side tools (`web_search_20250305`,
                // `code_execution_20250522`, etc.) pass through verbatim — the
                // Messages API expects `{type, name, ...}` as-is.
                Value::Object(tool_obj.clone())
            }
            Some(other) => {
                return Err(ProviderError::fatal(format!(
                    "tools[{}].type '{}' is unsupported; only 'function' or Anthropic server-side tools are supported",
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

pub(crate) fn merge_tool_calls(
    target: &mut Map<String, Value>,
    tool_calls: &[AiToolSpec],
) -> Result<(), ProviderError> {
    if tool_calls.is_empty() {
        return Ok(());
    }

    let raw_tools = serde_json::to_value(tool_calls).map_err(|err| {
        ProviderError::fatal(format!("failed to serialize payload.tool_calls: {err}"))
    })?;
    target.insert("tools".to_string(), normalize_tools_option(&raw_tools)?);
    Ok(())
}

/// Anthropic-native server-side tool type prefix. Used to recognise tools that
/// should pass through `normalize_tools_option` verbatim.
fn is_claude_server_tool_type(tool_type: &str) -> bool {
    // Anthropic versions server tools with a `_YYYYMMDD` suffix (e.g.
    // `web_search_20250305`, `code_execution_20250522`,
    // `computer_20250124`). Match the family by prefix so newly-released
    // versions don't require a code change.
    const PREFIXES: &[&str] = &[
        "web_search_",
        "code_execution_",
        "computer_",
        "text_editor_",
        "bash_",
    ];
    PREFIXES.iter().any(|p| tool_type.starts_with(p))
}

const CLAUDE_WEB_SEARCH_TOOL_TYPE: &str = "web_search_20250305";
const CLAUDE_WEB_SEARCH_TOOL_NAME: &str = "web_search";

/// Inject Anthropic's server-side `web_search` tool when the request
/// requires the `web_search` feature.
///
/// Mirrors `OpenAIProvider::merge_requirements_tools` — the router records
/// `requirements.web_search: true` for `llm.chat`, and each provider is
/// responsible for translating that into its native server-tool wire format.
pub(crate) fn merge_requirements_tools(
    target: &mut Map<String, Value>,
    req: &AiMethodRequest,
) -> Result<(), ProviderError> {
    let web_search_required = req
        .requirements
        .requires_feature(buckyos_api::features::WEB_SEARCH);
    if !web_search_required {
        return Ok(());
    }

    let web_search_tool = json!({
        "type": CLAUDE_WEB_SEARCH_TOOL_TYPE,
        "name": CLAUDE_WEB_SEARCH_TOOL_NAME,
    });

    if let Some(tools_value) = target.get_mut("tools") {
        let Some(tools) = tools_value.as_array_mut() else {
            return Err(ProviderError::fatal(
                "tools must be an array when enabling web_search",
            ));
        };
        let already_has_web_search = tools.iter().any(|item| {
            let type_matches = item
                .get("type")
                .and_then(|value| value.as_str())
                .map(|value| value.starts_with("web_search_"))
                .unwrap_or(false);
            let name_matches = item
                .get("name")
                .and_then(|value| value.as_str())
                .map(|value| value == CLAUDE_WEB_SEARCH_TOOL_NAME)
                .unwrap_or(false);
            type_matches || name_matches
        });
        if !already_has_web_search {
            tools.push(web_search_tool);
        }
        return Ok(());
    }

    target.insert("tools".to_string(), Value::Array(vec![web_search_tool]));
    Ok(())
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

fn build_fallback_content(req: &AiMethodRequest) -> Result<Option<String>, ProviderError> {
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

fn content_value_to_text(value: &Value) -> Result<Option<String>, ProviderError> {
    if let Some(text) = value.as_str() {
        let text = text.trim();
        return Ok((!text.is_empty()).then(|| text.to_string()));
    }

    let Some(parts) = value.as_array() else {
        return Ok(None);
    };

    let mut lines = Vec::new();
    for part in parts {
        let Some(part_obj) = part.as_object() else {
            continue;
        };
        match part_obj.get("type").and_then(|value| value.as_str()) {
            Some("text") => {
                if let Some(text) = part_obj
                    .get("text")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    lines.push(text.to_string());
                }
            }
            Some("resource") => {
                if let Some(resource_value) = part_obj.get("resource") {
                    let resource: ResourceRef = serde_json::from_value(resource_value.clone())
                        .map_err(|err| {
                            ProviderError::fatal(format!("invalid content resource part: {}", err))
                        })?;
                    match resource {
                        ResourceRef::Url { url, .. } => {
                            lines.push(format!("resource_url: {}", url));
                        }
                        ResourceRef::NamedObject { obj_id } => {
                            lines.push(format!("named_object: {}", obj_id));
                        }
                        ResourceRef::Base64 { .. } => {
                            return Err(ProviderError::fatal(
                                "claude provider does not support base64 resources in this version",
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if lines.is_empty() {
        Ok(None)
    } else {
        Ok(Some(lines.join("\n")))
    }
}

fn build_messages(
    req: &AiMethodRequest,
    dialect: ProtocolDialect,
) -> Result<(Option<String>, Vec<Value>), ProviderError> {
    let mut system_parts: Vec<String> = vec![];
    let mut messages: Vec<Value> = vec![];

    // 主路径:消费 typed `Vec<AiMessage>` —— 保留 ToolUse / ToolResult /
    // Image / Document / Thinking 等 block,不再压缩成纯文本。
    if !req.payload.messages.is_empty() {
        for msg in req.payload.messages.iter() {
            lower_ai_message(msg, dialect, &mut system_parts, &mut messages)?;
        }
    }

    // 兼容路径:caller 直接通过 input_json.messages 喂裸 JSON。这条线现在
    // 仅承担降级到 text 的兜底 —— 上游已经鼓励走 typed messages。
    if messages.is_empty() && system_parts.is_empty() {
        if let Some(canonical_messages) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("messages"))
            .and_then(|value| value.as_array())
        {
            for msg in canonical_messages {
                let Some(msg_obj) = msg.as_object() else {
                    continue;
                };
                let role = msg_obj
                    .get("role")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("user")
                    .to_lowercase();
                let Some(content) = msg_obj
                    .get("content")
                    .map(content_value_to_text)
                    .transpose()?
                    .flatten()
                else {
                    continue;
                };
                push_text_message_content(
                    &mut system_parts,
                    &mut messages,
                    role.as_str(),
                    content.as_str(),
                );
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

/// Lower a single `AiMessage` (typed content blocks) into Claude / MiniMax
/// native message form. System/Developer text accumulates into
/// `system_parts`; everything else lands in `messages`.
fn lower_ai_message(
    msg: &AiMessage,
    dialect: ProtocolDialect,
    system_parts: &mut Vec<String>,
    messages: &mut Vec<Value>,
) -> Result<(), ProviderError> {
    match msg.role {
        AiRole::System | AiRole::Developer => {
            let mut text = String::new();
            for block in &msg.content {
                if let AiContent::Text { text: chunk } = block {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(chunk);
                }
            }
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                system_parts.push(trimmed.to_string());
            }
        }
        AiRole::User => {
            let blocks = lower_user_blocks(&msg.content)?;
            if !blocks.is_empty() {
                messages.push(json!({ "role": "user", "content": blocks }));
            }
        }
        AiRole::Assistant => {
            let blocks = lower_assistant_blocks(&msg.content)?;
            if !blocks.is_empty() {
                messages.push(json!({ "role": "assistant", "content": blocks }));
            }
        }
        AiRole::Tool => {
            // Validator guarantees a single ToolResult block; defend against
            // misuse anyway.
            let Some(AiContent::ToolResult {
                call_id,
                content,
                is_error,
            }) = msg.content.first()
            else {
                return Ok(());
            };
            match dialect {
                ProtocolDialect::Claude => {
                    let blocks = lower_tool_result_content_claude(content)?;
                    let mut tr = json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": blocks,
                    });
                    if *is_error {
                        if let Value::Object(map) = &mut tr {
                            map.insert("is_error".to_string(), Value::Bool(true));
                        }
                    }
                    messages.push(json!({
                        "role": "user",
                        "content": [tr],
                    }));
                }
                ProtocolDialect::MiniMax => {
                    // Degrade to user text per the MiniMax spec.
                    let text = tool_result_content_to_text(content);
                    let body = if *is_error {
                        format!("tool_result[{}] (error): {}", call_id, text)
                    } else {
                        format!("tool_result[{}]: {}", call_id, text)
                    };
                    messages.push(text_message("user", body.as_str()));
                }
            }
        }
    }
    Ok(())
}

fn lower_user_blocks(content: &[AiContent]) -> Result<Vec<Value>, ProviderError> {
    let mut blocks = Vec::with_capacity(content.len());
    for block in content {
        match block {
            AiContent::Text { text } => {
                if !text.is_empty() {
                    blocks.push(text_content_block(text));
                }
            }
            AiContent::Image { source } => {
                blocks.push(image_block_from_source(source)?);
            }
            AiContent::Document { source, title } => {
                blocks.push(document_block_from_source(source, title.as_deref())?);
            }
            _ => {
                // Validator rejects other block types on User role; ignore
                // defensively rather than fatal.
            }
        }
    }
    Ok(blocks)
}

fn lower_assistant_blocks(content: &[AiContent]) -> Result<Vec<Value>, ProviderError> {
    let mut blocks = Vec::with_capacity(content.len());
    for block in content {
        match block {
            AiContent::Text { text } => {
                if !text.is_empty() {
                    blocks.push(text_content_block(text));
                }
            }
            AiContent::ToolUse {
                call_id,
                name,
                args,
            } => {
                let input = serde_json::to_value(args).unwrap_or_else(|_| json!({}));
                blocks.push(json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": input,
                }));
            }
            AiContent::Thinking {
                text,
                provider_metadata,
                summary,
            } => {
                // Claude `thinking` blocks pair `thinking` (plaintext) with
                // `signature` (verifier blob). When we don't have a signature
                // we fall back to emitting just the summary as text so the
                // assistant turn isn't reduced to nothing.
                let signature = provider_metadata
                    .as_ref()
                    .and_then(|value| value.get("signature"))
                    .and_then(|value| value.as_str());
                if let (Some(thinking), Some(sig)) = (text.as_deref(), signature) {
                    blocks.push(json!({
                        "type": "thinking",
                        "thinking": thinking,
                        "signature": sig,
                    }));
                } else if let Some(summary) = summary.as_deref().filter(|s| !s.is_empty()) {
                    blocks.push(text_content_block(summary));
                } else if let Some(thinking) = text.as_deref().filter(|s| !s.is_empty()) {
                    blocks.push(text_content_block(thinking));
                }
            }
            AiContent::ProviderState { provider, value } => {
                // Restore native item only when the block was authored by
                // this provider; other providers' opaque state is dropped.
                if provider.eq_ignore_ascii_case("anthropic") {
                    blocks.push(value.clone());
                }
            }
            _ => {
                // ToolResult / Image / Document not valid on Assistant role —
                // validator catches this upstream.
            }
        }
    }
    Ok(blocks)
}

fn lower_tool_result_content_claude(
    content: &[AiToolResultContent],
) -> Result<Vec<Value>, ProviderError> {
    let mut blocks = Vec::with_capacity(content.len());
    for item in content {
        match item {
            AiToolResultContent::Text { text } => {
                blocks.push(text_content_block(text));
            }
            AiToolResultContent::Image { source } => {
                blocks.push(image_block_from_source(source)?);
            }
            AiToolResultContent::Document { source, title } => {
                blocks.push(document_block_from_source(source, title.as_deref())?);
            }
        }
    }
    Ok(blocks)
}

fn tool_result_content_to_text(content: &[AiToolResultContent]) -> String {
    let mut parts = Vec::new();
    for item in content {
        match item {
            AiToolResultContent::Text { text } => parts.push(text.clone()),
            AiToolResultContent::Image { source } => {
                parts.push(resource_text_placeholder(source));
            }
            AiToolResultContent::Document { source, title } => {
                let mut line = resource_text_placeholder(source);
                if let Some(title) = title {
                    line.push_str(" (");
                    line.push_str(title);
                    line.push(')');
                }
                parts.push(line);
            }
        }
    }
    parts.join("\n")
}

fn resource_text_placeholder(source: &ResourceRef) -> String {
    match source {
        ResourceRef::Url { url, .. } => format!("resource_url: {}", url),
        ResourceRef::NamedObject { obj_id } => format!("named_object: {}", obj_id),
        ResourceRef::Base64 { mime, .. } => format!("inline_{}", mime),
    }
}

fn image_block_from_source(source: &ResourceRef) -> Result<Value, ProviderError> {
    Ok(json!({
        "type": "image",
        "source": claude_source_object(source)?,
    }))
}

fn document_block_from_source(
    source: &ResourceRef,
    title: Option<&str>,
) -> Result<Value, ProviderError> {
    let mut obj = Map::new();
    obj.insert("type".to_string(), Value::String("document".to_string()));
    obj.insert("source".to_string(), claude_source_object(source)?);
    if let Some(title) = title.filter(|s| !s.is_empty()) {
        obj.insert("title".to_string(), Value::String(title.to_string()));
    }
    Ok(Value::Object(obj))
}

fn claude_source_object(source: &ResourceRef) -> Result<Value, ProviderError> {
    match source {
        ResourceRef::Url { url, .. } => Ok(json!({
            "type": "url",
            "url": url,
        })),
        ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
            "type": "base64",
            "media_type": mime,
            "data": data_base64,
        })),
        ResourceRef::NamedObject { obj_id } => Err(ProviderError::fatal(format!(
            "claude provider cannot lower named_object source `{}` without resolution",
            obj_id
        ))),
    }
}

fn push_text_message_content(
    system_parts: &mut Vec<String>,
    messages: &mut Vec<Value>,
    role: &str,
    content: &str,
) {
    match role {
        "system" | "developer" => {
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

fn resolve_provider_model(req: &AiMethodRequest, provider_model: &str) -> Option<String> {
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
    req: &AiMethodRequest,
    provider_model: &str,
    default_max_tokens: Option<u64>,
) -> Result<(Map<String, Value>, Vec<String>), ProviderError> {
    convert_complete_request_with_dialect(
        req,
        provider_model,
        ProtocolDialect::Claude,
        default_max_tokens,
    )
}

pub(crate) fn convert_complete_request_with_dialect(
    req: &AiMethodRequest,
    provider_model: &str,
    dialect: ProtocolDialect,
    default_max_tokens: Option<u64>,
) -> Result<(Map<String, Value>, Vec<String>), ProviderError> {
    let model = resolve_provider_model(req, provider_model)
        .ok_or_else(|| ProviderError::fatal("provider model is required for claude request"))?;

    let (system, messages) = build_messages(req, dialect)?;

    let mut request = Map::new();
    request.insert("model".to_string(), Value::String(model));
    request.insert("messages".to_string(), Value::Array(messages));
    if let Some(system) = system {
        request.insert("system".to_string(), Value::String(system));
    }

    let mut ignored = vec![];
    let mut extra_messages = vec![];
    if let Some(input_json) = req.payload.input_json.as_ref() {
        let (ignored_options, converted_tool_messages) = merge_options(&mut request, input_json)?;
        ignored.extend(ignored_options);
        extra_messages.extend(converted_tool_messages);
    }
    if let Some(options) = req.payload.options.as_ref() {
        let (ignored_options, converted_tool_messages) = merge_options(&mut request, options)?;
        ignored.extend(ignored_options);
        extra_messages.extend(converted_tool_messages);
    }

    if !extra_messages.is_empty() {
        if let Some(message_array) = request
            .get_mut("messages")
            .and_then(|value| value.as_array_mut())
        {
            message_array.extend(extra_messages);
        }
    }

    merge_tool_calls(&mut request, req.payload.tool_specs.as_slice())?;

    if matches!(dialect, ProtocolDialect::Claude) {
        merge_requirements_tools(&mut request, req)?;
    }

    if !request.contains_key("max_tokens") {
        request.insert(
            "max_tokens".to_string(),
            Value::from(default_max_tokens.unwrap_or(FALLBACK_MAX_TOKENS)),
        );
    }

    Ok((request, ignored))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        value_to_object_map, AiMessage, AiMethodRequest, AiPayload, AiRole, AiToolSpec, Capability,
        ModelSpec, Requirements,
    };

    fn base_request() -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new(
                "llm.default".to_string(),
                Some("claude-3-5-sonnet-20241022".to_string()),
            ),
            Requirements::default(),
            AiPayload::new(None, vec![], vec![], vec![], None, None),
            None,
        )
    }

    #[test]
    fn convert_complete_request_maps_messages_and_tool_messages() {
        let mut req = base_request();
        req.payload.messages = vec![
            AiMessage::text(AiRole::System, "system rules"),
            AiMessage::text(AiRole::User, "hello"),
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

        let (request, ignored) = convert_complete_request(&req, "claude-3-7-sonnet-20250219", None)
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

        let (request, _ignored) = convert_complete_request(&req, "claude-3-5-haiku-20241022", None)
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
    fn convert_complete_request_uses_inventory_default_max_tokens() {
        let mut req = base_request();
        req.payload.messages = vec![AiMessage::text(AiRole::User, "hello")];

        let (request, _ignored) =
            convert_complete_request(&req, "claude-3-7-sonnet-20250219", Some(8192))
                .expect("convert should work");

        assert_eq!(request.get("max_tokens"), Some(&json!(8192)));
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

    #[test]
    fn convert_complete_request_prefers_payload_tool_calls_over_option_tools() {
        let mut req = base_request();
        req.payload.messages = vec![AiMessage::text(AiRole::User, "hello")];
        req.payload.tool_specs = vec![AiToolSpec {
            name: "payload_tool".to_string(),
            description: "from payload".to_string(),
            args_schema: value_to_object_map(json!({"type":"object"})),
            output_schema: json!({"type":"object"}),
        }];
        req.payload.options = Some(json!({
            "tools": [
                {
                    "name": "option_tool",
                    "description": "from options",
                    "args_schema": { "type": "object" }
                }
            ]
        }));

        let (request, _ignored) =
            convert_complete_request(&req, "claude-3-7-sonnet-20250219", None)
                .expect("convert should work");
        let request_value = Value::Object(request);

        assert_eq!(
            request_value
                .pointer("/tools/0/name")
                .and_then(|value| value.as_str()),
            Some("payload_tool")
        );
    }

    #[test]
    fn typed_assistant_tool_use_lowers_to_native_block() {
        use buckyos_api::AiContent;
        use std::collections::HashMap;

        let mut req = base_request();
        let mut args = HashMap::new();
        args.insert("topic".to_string(), json!("project"));
        req.payload.messages = vec![
            AiMessage::text(AiRole::User, "look it up"),
            AiMessage::new(
                AiRole::Assistant,
                vec![AiContent::tool_use("call-1", "load_memory", args)],
            ),
            AiMessage::new(
                AiRole::Tool,
                vec![AiContent::tool_result_text("call-1", "ok", false)],
            ),
        ];

        let (request, _) = convert_complete_request(&req, "claude-3-7-sonnet-20250219", None)
            .expect("convert should work");
        let value = Value::Object(request);

        assert_eq!(
            value
                .pointer("/messages/1/content/0/type")
                .and_then(|v| v.as_str()),
            Some("tool_use"),
            "assistant tool_use must lower to native Claude block: {}",
            value
        );
        assert_eq!(
            value
                .pointer("/messages/1/content/0/id")
                .and_then(|v| v.as_str()),
            Some("call-1")
        );
        assert_eq!(
            value
                .pointer("/messages/2/content/0/type")
                .and_then(|v| v.as_str()),
            Some("tool_result"),
            "tool role must become Claude tool_result: {}",
            value
        );
        assert_eq!(
            value
                .pointer("/messages/2/content/0/tool_use_id")
                .and_then(|v| v.as_str()),
            Some("call-1")
        );
    }

    #[test]
    fn minimax_dialect_degrades_tool_result_to_user_text() {
        use buckyos_api::AiContent;

        let mut req = base_request();
        req.payload.messages = vec![
            AiMessage::text(AiRole::User, "look it up"),
            AiMessage::new(
                AiRole::Tool,
                vec![AiContent::tool_result_text("call-1", "ok", false)],
            ),
        ];

        let (request, _) = convert_complete_request_with_dialect(
            &req,
            "MiniMax-M2.5",
            ProtocolDialect::MiniMax,
            None,
        )
        .expect("convert should work");
        let value = Value::Object(request);

        // The tool result must NOT carry a Claude-shaped tool_result block;
        // it must collapse to plain user text per the MiniMax spec.
        assert_ne!(
            value
                .pointer("/messages/1/content/0/type")
                .and_then(|v| v.as_str()),
            Some("tool_result"),
            "minimax should not emit tool_result blocks: {}",
            value
        );
        let body = value
            .pointer("/messages/1/content/0/text")
            .and_then(|v| v.as_str())
            .expect("tool result must land as text body");
        assert!(
            body.contains("call-1") && body.contains("ok"),
            "tool result text must carry call_id and payload: {}",
            body
        );
    }

    #[test]
    fn merge_requirements_tools_injects_anthropic_web_search() {
        let mut req = base_request();
        req.payload.messages = vec![AiMessage::text(AiRole::User, "search")];
        req.requirements
            .must_features
            .push(buckyos_api::features::WEB_SEARCH.to_string());

        let (request, _ignored) =
            convert_complete_request(&req, "claude-3-7-sonnet-20250219", None).expect("convert");
        let tools = request
            .get("tools")
            .and_then(|v| v.as_array())
            .expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(
            tools[0].get("type").and_then(|v| v.as_str()),
            Some("web_search_20250305")
        );
        assert_eq!(
            tools[0].get("name").and_then(|v| v.as_str()),
            Some("web_search")
        );
    }

    #[test]
    fn merge_requirements_tools_dedupes_existing_web_search() {
        let mut req = base_request();
        req.payload.messages = vec![AiMessage::text(AiRole::User, "search")];
        req.requirements
            .must_features
            .push(buckyos_api::features::WEB_SEARCH.to_string());
        req.payload.options = Some(json!({
            "tools": [
                {
                    "type": "web_search_20250305",
                    "name": "web_search",
                    "max_uses": 3,
                }
            ]
        }));

        let (request, _ignored) =
            convert_complete_request(&req, "claude-3-7-sonnet-20250219", None).expect("convert");
        let tools = request
            .get("tools")
            .and_then(|v| v.as_array())
            .expect("tools array");
        assert_eq!(tools.len(), 1, "should not duplicate web_search tool");
        assert_eq!(
            tools[0].get("max_uses").and_then(|v| v.as_u64()),
            Some(3),
            "existing user-supplied tool spec must be preserved"
        );
    }

    #[test]
    fn merge_requirements_tools_is_noop_without_requirement() {
        let mut req = base_request();
        req.payload.messages = vec![AiMessage::text(AiRole::User, "hello")];
        let (request, _ignored) =
            convert_complete_request(&req, "claude-3-7-sonnet-20250219", None).expect("convert");
        assert!(
            request.get("tools").is_none(),
            "tools must not be set when web_search not required: {:?}",
            request.get("tools")
        );
    }
}
