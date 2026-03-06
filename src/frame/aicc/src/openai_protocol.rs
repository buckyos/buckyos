use crate::aicc::ProviderError;
use buckyos_api::{features, AiToolSpec, CompleteRequest, RespFormat};
use serde_json::{json, Map, Value};

const OPENAI_OPTION_ALLOWLIST: &[&str] = &[
    "audio",
    "background",
    "conversation",
    "include",
    "instructions",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "parallel_tool_calls",
    "previous_response_id",
    "prompt",
    "reasoning",
    "service_tier",
    "store",
    "stream",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
    "verbosity",
];
const OPENAI_TOOL_TYPE_FUNCTION: &str = "function";
const OPENAI_TOOL_TYPE_WEB_SEARCH: &str = "web_search";
const OPENAI_TOOL_TYPE_WEB_SEARCH_PREVIEW: &str = "web_search_preview";
const OPENAI_RESPONSE_SCHEMA_NAME: &str = "aicc_response";
const OPENAI_FUNCTION_NAME_PATTERN: &str = "^[a-zA-Z0-9_-]+$";
const OPENAI_BUILTIN_TOOL_TYPES: &[&str] = &[
    OPENAI_TOOL_TYPE_WEB_SEARCH_PREVIEW,
    "file_search",
    "computer_use_preview",
    "image_generation",
    "code_interpreter",
    "mcp",
];

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

    let mut normalized = Map::new();
    normalized.insert(
        "type".to_string(),
        Value::String(OPENAI_TOOL_TYPE_FUNCTION.to_string()),
    );
    normalized.insert("name".to_string(), Value::String(name));
    if let Some(description) = tool
        .get("description")
        .and_then(|value| value.as_str())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        normalized.insert(
            "description".to_string(),
            Value::String(description.to_string()),
        );
    }
    normalized.insert("parameters".to_string(), parameters);
    Ok(Value::Object(normalized))
}

fn normalize_openai_function_tool(
    tool: &Map<String, Value>,
    idx: usize,
) -> Result<Value, ProviderError> {
    let (name_path, raw_name, description, parameters, strict) =
        if let Some(function_obj) = tool.get("function").and_then(|value| value.as_object()) {
            let Some(raw_name) = function_obj.get("name").and_then(|value| value.as_str()) else {
                return Err(ProviderError::fatal(format!(
                    "tools[{}].function.name is required",
                    idx
                )));
            };
            (
                format!("tools[{}].function.name", idx),
                raw_name,
                function_obj.get("description").cloned(),
                function_obj
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(default_tool_parameters),
                function_obj
                    .get("strict")
                    .cloned()
                    .or_else(|| tool.get("strict").cloned()),
            )
        } else {
            let Some(raw_name) = tool.get("name").and_then(|value| value.as_str()) else {
                return Err(ProviderError::fatal(format!(
                    "tools[{}].name is required when tools[{}].type=function",
                    idx, idx
                )));
            };
            (
                format!("tools[{}].name", idx),
                raw_name,
                tool.get("description").cloned(),
                tool.get("parameters")
                    .cloned()
                    .unwrap_or_else(default_tool_parameters),
                tool.get("strict").cloned(),
            )
        };

    let name = validate_openai_function_name(raw_name, name_path.as_str())?;
    if !parameters.is_object() {
        return Err(ProviderError::fatal(format!(
            "tools[{}].parameters must be an object",
            idx
        )));
    }

    let mut normalized = Map::new();
    normalized.insert(
        "type".to_string(),
        Value::String(OPENAI_TOOL_TYPE_FUNCTION.to_string()),
    );
    normalized.insert("name".to_string(), Value::String(name));
    if let Some(description) = description.and_then(|value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    }) {
        normalized.insert("description".to_string(), Value::String(description));
    }
    normalized.insert("parameters".to_string(), parameters);
    if let Some(strict) = strict.and_then(|value| value.as_bool()) {
        normalized.insert("strict".to_string(), Value::Bool(strict));
    }
    Ok(Value::Object(normalized))
}

fn normalize_builtin_tool(
    tool: &Map<String, Value>,
    idx: usize,
    tool_type: &str,
) -> Result<Value, ProviderError> {
    let normalized_tool_type = if tool_type == OPENAI_TOOL_TYPE_WEB_SEARCH {
        OPENAI_TOOL_TYPE_WEB_SEARCH_PREVIEW
    } else {
        tool_type
    };

    if !OPENAI_BUILTIN_TOOL_TYPES.contains(&normalized_tool_type) {
        return Err(ProviderError::fatal(format!(
            "tools[{}].type '{}' is unsupported",
            idx, tool_type
        )));
    }

    let mut normalized = tool.clone();
    normalized.insert(
        "type".to_string(),
        Value::String(normalized_tool_type.to_string()),
    );
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
            Some(other) => normalize_builtin_tool(tool_obj, idx, other)?,
            None => convert_internal_tool(tool_obj, idx)?,
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

fn set_text_format(target: &mut Map<String, Value>, format: Value) -> Result<(), ProviderError> {
    match target.entry("text".to_string()) {
        serde_json::map::Entry::Vacant(entry) => {
            entry.insert(json!({ "format": format }));
        }
        serde_json::map::Entry::Occupied(mut entry) => {
            let Some(text_obj) = entry.get_mut().as_object_mut() else {
                return Err(ProviderError::fatal("text option must be an object"));
            };
            text_obj.insert("format".to_string(), format);
        }
    }
    Ok(())
}

fn has_text_format(target: &Map<String, Value>) -> bool {
    target
        .get("text")
        .and_then(|value| value.as_object())
        .and_then(|value| value.get("format"))
        .is_some()
}

fn convert_response_schema_option(schema: &Value) -> Result<Value, ProviderError> {
    if !schema.is_object() {
        return Err(ProviderError::fatal("response_schema must be an object"));
    }

    Ok(json!({
        "type": "json_schema",
        "name": OPENAI_RESPONSE_SCHEMA_NAME,
        "schema": schema,
        "strict": true
    }))
}

fn convert_response_format_option(response_format: &Value) -> Result<Value, ProviderError> {
    let Some(response_format_obj) = response_format.as_object() else {
        return Err(ProviderError::fatal("response_format must be an object"));
    };

    let format_type = response_format_obj
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("text");
    if format_type == "json_schema" {
        if let Some(json_schema_obj) = response_format_obj
            .get("json_schema")
            .and_then(|value| value.as_object())
        {
            let schema = json_schema_obj.get("schema").cloned().ok_or_else(|| {
                ProviderError::fatal("response_format.json_schema.schema is required")
            })?;
            if !schema.is_object() {
                return Err(ProviderError::fatal(
                    "response_format.json_schema.schema must be an object",
                ));
            }
            let name = json_schema_obj
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(OPENAI_RESPONSE_SCHEMA_NAME);

            return Ok(json!({
                "type": "json_schema",
                "name": name,
                "schema": schema,
                "strict": json_schema_obj
                    .get("strict")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true)
            }));
        }

        let schema = response_format_obj.get("schema").cloned().ok_or_else(|| {
            ProviderError::fatal("response_format.schema is required for json_schema")
        })?;
        if !schema.is_object() {
            return Err(ProviderError::fatal(
                "response_format.schema must be an object for json_schema",
            ));
        }
        let name = response_format_obj
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(OPENAI_RESPONSE_SCHEMA_NAME);
        return Ok(json!({
            "type": "json_schema",
            "name": name,
            "schema": schema,
            "strict": response_format_obj
                .get("strict")
                .and_then(|value| value.as_bool())
                .unwrap_or(true)
        }));
    }

    if format_type == "json_object" || format_type == "text" {
        return Ok(json!({ "type": format_type }));
    }

    Ok(response_format.clone())
}

fn merge_reasoning_effort(
    target: &mut Map<String, Value>,
    effort: &Value,
) -> Result<(), ProviderError> {
    let Some(effort_str) = effort.as_str() else {
        return Err(ProviderError::fatal("reasoning_effort must be a string"));
    };

    match target.entry("reasoning".to_string()) {
        serde_json::map::Entry::Vacant(entry) => {
            entry.insert(json!({ "effort": effort_str }));
        }
        serde_json::map::Entry::Occupied(mut entry) => {
            let Some(reasoning_obj) = entry.get_mut().as_object_mut() else {
                return Err(ProviderError::fatal("reasoning option must be an object"));
            };
            reasoning_obj.insert("effort".to_string(), Value::String(effort_str.to_string()));
        }
    }
    Ok(())
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
        if key == "model" || key == "messages" || key == "input" {
            continue;
        }
        if key == "protocol" || key == "process_name" || key == "tool_messages" {
            ignored.push(key.clone());
            continue;
        }
        if key == "max_tokens" || key == "max_completion_tokens" {
            if !target.contains_key("max_output_tokens") {
                target.insert("max_output_tokens".to_string(), value.clone());
            }
            continue;
        }
        if key == "response_schema" {
            if has_text_format(target) {
                ignored.push(key.clone());
                continue;
            }
            set_text_format(target, convert_response_schema_option(value)?)?;
            continue;
        }
        if key == "response_format" {
            if has_text_format(target) {
                ignored.push(key.clone());
                continue;
            }
            set_text_format(target, convert_response_format_option(value)?)?;
            continue;
        }
        if key == "reasoning_effort" {
            merge_reasoning_effort(target, value)?;
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

pub(crate) fn merge_requirements_response_format(
    target: &mut Map<String, Value>,
    req: &CompleteRequest,
) {
    if has_text_format(target) {
        return;
    }

    let json_output_required = req.requirements.resp_foramt == RespFormat::Json
        || req
            .requirements
            .must_features
            .iter()
            .any(|feature| feature == features::JSON_OUTPUT);
    if json_output_required {
        let _ = set_text_format(target, json!({ "type": "json_object" }));
    }
}

pub(crate) fn strip_incompatible_sampling_options(
    target: &mut Map<String, Value>,
    provider_model: &str,
) -> Vec<String> {
    let model = provider_model.trim().to_ascii_lowercase();
    if !model.starts_with("gpt-5") {
        return vec![];
    }

    let is_old_gpt5 = model == "gpt-5"
        || model.starts_with("gpt-5-")
        || model.starts_with("gpt-5-mini")
        || model.starts_with("gpt-5-nano");
    let is_codex = model.contains("codex");
    let reasoning_effort = target
        .get("reasoning")
        .and_then(|value| value.as_object())
        .and_then(|value| value.get("effort"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase());
    let supports_sampling =
        !is_old_gpt5 && !is_codex && reasoning_effort.as_deref() == Some("none");
    if supports_sampling {
        return vec![];
    }

    let mut removed = vec![];
    for key in ["temperature", "top_p", "logprobs", "top_logprobs"] {
        if target.remove(key).is_some() {
            removed.push(key.to_string());
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        value_to_object_map, AiPayload, AiToolSpec, Capability, ModelSpec, Requirements,
    };
    use serde_json::json;

    fn base_request() -> CompleteRequest {
        CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::default(),
            None,
        )
    }

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
                .pointer("/tools/0/name")
                .and_then(|value| value.as_str()),
            Some("load_memory")
        );
        assert_eq!(
            target_value.pointer("/tools/0/parameters"),
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
                .pointer("/tools/0/name")
                .and_then(|value| value.as_str()),
            Some("workshop_exec_bash")
        );
        assert_eq!(
            target_value.pointer("/tools/0/parameters"),
            Some(&json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            }))
        );
    }

    #[test]
    fn merge_options_accepts_web_search_tool_and_upgrades_legacy_name() {
        let options = json!({
            "tools": [
                {
                    "type": "web_search"
                }
            ]
        });

        let mut target = Map::new();
        merge_options(&mut target, &options).expect("merge options should work");
        let target_value = Value::Object(target.clone());

        assert_eq!(
            target_value
                .pointer("/tools/0/type")
                .and_then(|value| value.as_str()),
            Some("web_search_preview")
        );
    }

    #[test]
    fn merge_options_maps_max_tokens_to_max_output_tokens() {
        let options = json!({
            "max_tokens": 321
        });

        let mut target = Map::new();
        merge_options(&mut target, &options).expect("merge options should work");
        assert_eq!(target.get("max_output_tokens"), Some(&json!(321)));
    }

    #[test]
    fn merge_options_maps_reasoning_effort_to_reasoning_object() {
        let options = json!({
            "reasoning_effort": "low"
        });

        let mut target = Map::new();
        merge_options(&mut target, &options).expect("merge options should work");
        let target_value = Value::Object(target);

        assert_eq!(
            target_value
                .pointer("/reasoning/effort")
                .and_then(|value| value.as_str()),
            Some("low")
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
    fn merge_options_maps_response_schema_to_text_format() {
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
                .pointer("/text/format/type")
                .and_then(|value| value.as_str()),
            Some("json_schema")
        );
        assert_eq!(
            target_value
                .pointer("/text/format/name")
                .and_then(|value| value.as_str()),
            Some("aicc_response")
        );
        assert_eq!(
            target_value.pointer("/text/format/schema"),
            Some(&json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" }
                },
                "required": ["ok"]
            }))
        );
    }

    #[test]
    fn merge_tool_calls_populates_tools_from_payload() {
        let mut target = Map::new();
        merge_tool_calls(
            &mut target,
            &[AiToolSpec {
                name: "workshop_exec_bash".to_string(),
                description: "Run shell command".to_string(),
                args_schema: value_to_object_map(json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    }
                })),
                output_schema: json!({"type": "object"}),
            }],
        )
        .expect("merge tool calls should work");
        let target_value = Value::Object(target);

        assert_eq!(
            target_value
                .pointer("/tools/0/name")
                .and_then(|value| value.as_str()),
            Some("workshop_exec_bash")
        );
        assert_eq!(
            target_value.pointer("/tools/0/parameters"),
            Some(&json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            }))
        );
    }

    #[test]
    fn merge_requirements_response_format_sets_json_object_for_json() {
        let mut target = Map::new();
        let mut req = base_request();
        req.requirements.resp_foramt = RespFormat::Json;

        merge_requirements_response_format(&mut target, &req);

        assert_eq!(
            target.get("text"),
            Some(&json!({
                "format": {
                    "type": "json_object"
                }
            }))
        );
    }

    #[test]
    fn merge_requirements_response_format_sets_json_object_for_json_feature() {
        let mut target = Map::new();
        let mut req = base_request();
        req.requirements.must_features = vec![features::JSON_OUTPUT.to_string()];

        merge_requirements_response_format(&mut target, &req);

        assert_eq!(
            target.get("text"),
            Some(&json!({
                "format": {
                    "type": "json_object"
                }
            }))
        );
    }

    #[test]
    fn merge_requirements_response_format_does_not_override_existing() {
        let mut target = Map::new();
        target.insert(
            "text".to_string(),
            json!({
                "format": {
                    "type": "json_schema",
                    "name": "custom",
                    "schema": {"type": "object"}
                }
            }),
        );
        let mut req = base_request();
        req.requirements.resp_foramt = RespFormat::Json;

        merge_requirements_response_format(&mut target, &req);

        assert_eq!(
            target.get("text"),
            Some(&json!({
                "format": {
                    "type": "json_schema",
                    "name": "custom",
                    "schema": {"type": "object"}
                }
            }))
        );
    }

    #[test]
    fn strip_incompatible_sampling_options_removes_for_gpt5_codex() {
        let mut target = json!({
            "temperature": 0.2,
            "top_p": 0.9,
            "logprobs": true,
            "top_logprobs": 5
        })
        .as_object()
        .cloned()
        .expect("object");

        let removed = strip_incompatible_sampling_options(&mut target, "gpt-5.2-codex");
        assert_eq!(removed.len(), 4);
        assert!(!target.contains_key("temperature"));
        assert!(!target.contains_key("top_p"));
        assert!(!target.contains_key("logprobs"));
        assert!(!target.contains_key("top_logprobs"));
    }

    #[test]
    fn strip_incompatible_sampling_options_keeps_for_gpt54_none_effort() {
        let mut target = json!({
            "reasoning": {"effort": "none"},
            "temperature": 0.2,
            "top_p": 0.9
        })
        .as_object()
        .cloned()
        .expect("object");

        let removed = strip_incompatible_sampling_options(&mut target, "gpt-5.4");
        assert!(removed.is_empty());
        assert_eq!(target.get("temperature"), Some(&json!(0.2)));
        assert_eq!(target.get("top_p"), Some(&json!(0.9)));
    }
}
