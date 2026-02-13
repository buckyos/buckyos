use crate::aicc::ProviderError;
use serde_json::{json, Map, Value};

const OPENAI_OPTION_ALLOWLIST: &[&str] = &[
    "audio",
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "max_completion_tokens",
    "max_tokens",
    "metadata",
    "modalities",
    "n",
    "parallel_tool_calls",
    "presence_penalty",
    "reasoning_effort",
    "response_format",
    "seed",
    "service_tier",
    "stop",
    "store",
    "stream",
    "stream_options",
    "temperature",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "user",
];
const OPENAI_TOOL_TYPE_FUNCTION: &str = "function";
const OPENAI_RESPONSE_SCHEMA_NAME: &str = "aicc_response";
const OPENAI_FUNCTION_NAME_PATTERN: &str = "^[a-zA-Z0-9_-]+$";

fn is_valid_openai_function_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-')
}

fn validate_openai_function_name(
    raw_name: &str,
    field_path: &str,
) -> Result<String, ProviderError> {
    let name = raw_name.trim();
    if !is_valid_openai_function_name(name) {
        return Err(ProviderError::fatal(format!(
            "{} is invalid; expected pattern '{}'",
            field_path, OPENAI_FUNCTION_NAME_PATTERN
        )));
    }
    Ok(name.to_string())
}

fn default_tool_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true,
    })
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
    let name = validate_openai_function_name(raw_name, format!("tools[{}].name", idx).as_str())?;

    let parameters = tool
        .get("args_schema")
        .cloned()
        .unwrap_or_else(default_tool_parameters);
    if !parameters.is_object() {
        return Err(ProviderError::fatal(format!(
            "tools[{}].args_schema must be an object",
            idx
        )));
    }

    let mut function_obj = Map::new();
    function_obj.insert("name".to_string(), Value::String(name));
    if let Some(description) = tool
        .get("description")
        .and_then(|value| value.as_str())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        function_obj.insert(
            "description".to_string(),
            Value::String(description.to_string()),
        );
    }
    function_obj.insert("parameters".to_string(), parameters);

    let mut normalized = Map::new();
    normalized.insert(
        "type".to_string(),
        Value::String(OPENAI_TOOL_TYPE_FUNCTION.to_string()),
    );
    normalized.insert("function".to_string(), Value::Object(function_obj));
    Ok(Value::Object(normalized))
}

fn normalize_openai_function_tool(
    tool: &Map<String, Value>,
    idx: usize,
) -> Result<Value, ProviderError> {
    let Some(mut function_obj) = tool
        .get("function")
        .and_then(|value| value.as_object())
        .cloned()
    else {
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
    let name =
        validate_openai_function_name(raw_name, format!("tools[{}].function.name", idx).as_str())?;
    function_obj.insert("name".to_string(), Value::String(name));

    if !function_obj.contains_key("parameters") {
        function_obj.insert("parameters".to_string(), default_tool_parameters());
    } else if !function_obj["parameters"].is_object() {
        return Err(ProviderError::fatal(format!(
            "tools[{}].function.parameters must be an object",
            idx
        )));
    }

    let mut normalized = Map::new();
    normalized.insert(
        "type".to_string(),
        Value::String(OPENAI_TOOL_TYPE_FUNCTION.to_string()),
    );
    normalized.insert("function".to_string(), Value::Object(function_obj));
    Ok(Value::Object(normalized))
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
            Some(OPENAI_TOOL_TYPE_FUNCTION) => normalize_openai_function_tool(tool_obj, idx)?,
            Some(other) => {
                return Err(ProviderError::fatal(format!(
                    "tools[{}].type '{}' is unsupported; only 'function' is supported",
                    idx, other
                )));
            }
            None => convert_internal_tool(tool_obj, idx)?,
        };
        normalized.push(converted);
    }

    Ok(Value::Array(normalized))
}

fn convert_response_schema_option(schema: &Value) -> Result<Value, ProviderError> {
    if !schema.is_object() {
        return Err(ProviderError::fatal("response_schema must be an object"));
    }

    Ok(json!({
        "type": "json_schema",
        "json_schema": {
            "name": OPENAI_RESPONSE_SCHEMA_NAME,
            "schema": schema,
        }
    }))
}

pub(crate) fn merge_options(
    target: &mut Map<String, Value>,
    options: &Value,
) -> Result<Vec<String>, ProviderError> {
    let Some(options_map) = options.as_object() else {
        return Ok(vec![]);
    };

    let mut ignored = vec![];
    for (key, value) in options_map.iter() {
        if key == "model" || key == "messages" {
            continue;
        }
        if key == "protocol" || key == "process_name" || key == "tool_messages" {
            ignored.push(key.clone());
            continue;
        }
        if key == "response_schema" {
            if target.contains_key("response_format") {
                ignored.push(key.clone());
                continue;
            }
            target.insert(
                "response_format".to_string(),
                convert_response_schema_option(value)?,
            );
            continue;
        }
        if key == "tools" {
            target.insert("tools".to_string(), normalize_tools_option(value)?);
            continue;
        }
        if !OPENAI_OPTION_ALLOWLIST.contains(&key.as_str()) {
            ignored.push(key.clone());
            continue;
        }
        target.insert(key.clone(), value.clone());
    }
    Ok(ignored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_options_converts_internal_tools_to_openai_format() {
        let options = json!({
            "protocol": "opendan_llm_behavior_v1",
            "process_name": "jarvis",
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
                    },
                    "output_schema": {
                        "type": "object"
                    }
                }
            ]
        });

        let mut target = Map::new();
        let ignored = merge_options(&mut target, &options).expect("merge options should work");
        let target_value = Value::Object(target.clone());

        assert_eq!(target.get("temperature"), Some(&json!(0.2)));
        assert_eq!(
            target_value
                .pointer("/tools/0/type")
                .and_then(|value| value.as_str()),
            Some("function")
        );
        assert_eq!(
            target_value
                .pointer("/tools/0/function/name")
                .and_then(|value| value.as_str()),
            Some("load_memory")
        );
        assert_eq!(
            target_value.pointer("/tools/0/function/parameters"),
            Some(&json!({
                "type": "object",
                "properties": {
                    "token_limit": { "type": "integer" }
                }
            }))
        );
        assert!(ignored.iter().any(|item| item == "protocol"));
        assert!(ignored.iter().any(|item| item == "process_name"));
    }

    #[test]
    fn merge_options_accepts_openai_function_tool_and_fills_default_parameters() {
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
                .pointer("/tools/0/function/name")
                .and_then(|value| value.as_str()),
            Some("workshop_exec_bash")
        );
        assert_eq!(
            target_value.pointer("/tools/0/function/parameters"),
            Some(&json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            }))
        );
    }

    #[test]
    fn merge_options_rejects_internal_tool_without_name() {
        let options = json!({
            "tools": [
                {
                    "description": "bad tool",
                    "args_schema": { "type": "object" }
                }
            ]
        });

        let mut target = Map::new();
        let err = merge_options(&mut target, &options).expect_err("merge options should fail");
        assert!(
            err.to_string().contains("tools[0].name is required"),
            "unexpected err: {}",
            err
        );
    }

    #[test]
    fn merge_options_rejects_internal_tool_name_with_dot() {
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
    fn merge_options_rejects_openai_function_tool_name_with_dot() {
        let options = json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "workshop.exec_bash",
                        "description": "Run command"
                    }
                }
            ]
        });

        let mut target = Map::new();
        let err = merge_options(&mut target, &options).expect_err("merge options should fail");
        assert!(
            err.to_string()
                .contains("tools[0].function.name is invalid; expected pattern '^[a-zA-Z0-9_-]+$'"),
            "unexpected err: {}",
            err
        );
    }

    #[test]
    fn merge_options_maps_response_schema_to_response_format() {
        let options = json!({
            "response_schema": {
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" }
                },
                "required": ["ok"]
            }
        });

        let mut target = Map::new();
        merge_options(&mut target, &options).expect("merge options should work");
        let target_value = Value::Object(target.clone());

        assert_eq!(
            target_value
                .pointer("/response_format/type")
                .and_then(|value| value.as_str()),
            Some("json_schema")
        );
        assert_eq!(
            target_value
                .pointer("/response_format/json_schema/name")
                .and_then(|value| value.as_str()),
            Some("aicc_response")
        );
        assert_eq!(
            target_value.pointer("/response_format/json_schema/schema"),
            Some(&json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" }
                },
                "required": ["ok"]
            }))
        );
    }
}
