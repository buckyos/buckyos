//! Minimal JSON-Schema-subset validator for Task Input/Result contracts.
//!
//! TaskMgr 2.0 freezes an input/output schema per task (doc §9.1). The
//! platform ships no full JSON Schema engine, so this validates the subset
//! that task schemas actually use — `type`, `properties`, `required`,
//! `additionalProperties`, `items`, `enum` — and deliberately ignores every
//! other keyword instead of rejecting it. An empty object schema (`{}` or
//! `true`) accepts any value.

use serde_json::Value;

/// Validate `value` against `schema`. Returns the first violation as a
/// human-readable path + message.
pub fn validate_json_schema(schema: &Value, value: &Value) -> Result<(), String> {
    validate_at(schema, value, "$")
}

fn validate_at(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let schema_obj = match schema {
        Value::Bool(true) => return Ok(()),
        Value::Bool(false) => return Err(format!("{}: schema forbids any value", path)),
        Value::Object(map) => map,
        _ => return Ok(()),
    };
    if schema_obj.is_empty() {
        return Ok(());
    }

    if let Some(enum_values) = schema_obj.get("enum").and_then(Value::as_array) {
        if !enum_values.iter().any(|allowed| allowed == value) {
            return Err(format!("{}: value not in enum", path));
        }
    }

    if let Some(type_spec) = schema_obj.get("type") {
        let allowed: Vec<&str> = match type_spec {
            Value::String(s) => vec![s.as_str()],
            Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !allowed.is_empty() && !allowed.iter().any(|t| type_matches(t, value)) {
            return Err(format!(
                "{}: expected type {}, found {}",
                path,
                allowed.join("|"),
                json_type_name(value)
            ));
        }
    }

    if let Value::Object(value_map) = value {
        if let Some(required) = schema_obj.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !value_map.contains_key(key) {
                    return Err(format!("{}: missing required property '{}'", path, key));
                }
            }
        }
        let properties = schema_obj.get("properties").and_then(Value::as_object);
        if let Some(properties) = properties {
            for (key, prop_schema) in properties {
                if let Some(prop_value) = value_map.get(key) {
                    validate_at(prop_schema, prop_value, &format!("{}.{}", path, key))?;
                }
            }
        }
        if let Some(Value::Bool(false)) = schema_obj.get("additionalProperties") {
            let known = properties;
            for key in value_map.keys() {
                let is_known = known.map(|p| p.contains_key(key)).unwrap_or(false);
                if !is_known {
                    return Err(format!("{}: unexpected property '{}'", path, key));
                }
            }
        }
    }

    if let (Value::Array(items), Some(item_schema)) = (value, schema_obj.get("items")) {
        for (index, item) in items.iter().enumerate() {
            validate_at(item_schema, item, &format!("{}[{}]", path, index))?;
        }
    }

    Ok(())
}

fn type_matches(type_name: &str, value: &Value) -> bool {
    match type_name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_schema_accepts_anything() {
        assert!(validate_json_schema(&json!({}), &json!({"a": 1})).is_ok());
        assert!(validate_json_schema(&json!(true), &json!(null)).is_ok());
    }

    #[test]
    fn type_and_required() {
        let schema = json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {"type": "string"},
                "count": {"type": "integer"}
            }
        });
        assert!(validate_json_schema(&schema, &json!({"url": "http://x"})).is_ok());
        assert!(validate_json_schema(&schema, &json!({})).is_err());
        assert!(validate_json_schema(&schema, &json!({"url": 5})).is_err());
        assert!(validate_json_schema(&schema, &json!({"url": "x", "count": "no"})).is_err());
    }

    #[test]
    fn enums_arrays_and_additional() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "mode": {"enum": ["fast", "safe"]},
                "tags": {"type": "array", "items": {"type": "string"}}
            }
        });
        assert!(validate_json_schema(&schema, &json!({"mode": "fast", "tags": ["a"]})).is_ok());
        assert!(validate_json_schema(&schema, &json!({"mode": "slow"})).is_err());
        assert!(validate_json_schema(&schema, &json!({"tags": [1]})).is_err());
        assert!(validate_json_schema(&schema, &json!({"other": 1})).is_err());
    }
}
