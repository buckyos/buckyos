//! Validation of a reply against the JSON Schema of a reply contract
//! (`OutputSpec::Json { schema, max_repairs: Some(_) }`).
//!
//! A subset of JSON Schema, enough for the reply contracts hosts declare:
//! `type` (one name or a list), `enum`, `const`, `properties`, `required`,
//! `additionalProperties` (bool or schema), `items` (one schema),
//! `minItems` / `maxItems`, `minLength` / `maxLength`, `minimum` /
//! `maximum`, `anyOf` / `oneOf` / `allOf`. Any other keyword is ignored:
//! the provider enforces the schema natively where it can, this check is the
//! safety net behind it, never stricter than the schema says.

use serde_json::Value;

/// `Ok` when `value` matches `schema`; otherwise the first mismatch, with
/// its JSON pointer (`/reply: expected string, got number`).
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    check(schema, value, "")
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn is_type(v: &Value, name: &str) -> bool {
    match name {
        "number" => v.is_number(),
        "integer" => {
            v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0)
        }
        other => type_name(v) == other,
    }
}

fn at(path: &str) -> &str {
    if path.is_empty() {
        "the reply"
    } else {
        path
    }
}

fn check(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Some(s) = schema.as_object() else {
        // `true` / `{}` accept anything; `false` accepts nothing.
        return match schema {
            Value::Bool(false) => Err(format!("{}: no value is allowed here", at(path))),
            _ => Ok(()),
        };
    };
    if let Some(t) = s.get("type") {
        let names: Vec<&str> = match t {
            Value::String(n) => vec![n.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !names.is_empty() && !names.iter().any(|n| is_type(value, n)) {
            return Err(format!(
                "{}: expected {}, got {}",
                at(path),
                names.join(" or "),
                type_name(value)
            ));
        }
    }
    if let Some(Value::Array(options)) = s.get("enum") {
        if !options.contains(value) {
            let list: Vec<String> = options.iter().map(Value::to_string).collect();
            return Err(format!("{}: must be one of {}", at(path), list.join(", ")));
        }
    }
    if let Some(c) = s.get("const") {
        if c != value {
            return Err(format!("{}: must be {c}", at(path)));
        }
    }
    match value {
        Value::Object(map) => {
            let props = s.get("properties").and_then(Value::as_object);
            if let Some(Value::Array(required)) = s.get("required") {
                for key in required.iter().filter_map(Value::as_str) {
                    if !map.contains_key(key) {
                        return Err(format!("{}: missing required field `{key}`", at(path)));
                    }
                }
            }
            for (key, v) in map {
                let child = format!("{path}/{key}");
                match props.and_then(|p| p.get(key)) {
                    Some(sub) => check(sub, v, &child)?,
                    None => match s.get("additionalProperties") {
                        Some(Value::Bool(false)) => {
                            let known: Vec<&str> = props
                                .map(|p| p.keys().map(String::as_str).collect())
                                .unwrap_or_default();
                            return Err(format!(
                                "{}: unknown field `{key}` (allowed: {})",
                                at(path),
                                known.join(", ")
                            ));
                        }
                        Some(sub @ Value::Object(_)) => check(sub, v, &child)?,
                        _ => {}
                    },
                }
            }
        }
        Value::Array(items) => {
            if let Some(n) = s.get("minItems").and_then(Value::as_u64) {
                if (items.len() as u64) < n {
                    return Err(format!("{}: needs at least {n} items", at(path)));
                }
            }
            if let Some(n) = s.get("maxItems").and_then(Value::as_u64) {
                if (items.len() as u64) > n {
                    return Err(format!("{}: allows at most {n} items", at(path)));
                }
            }
            if let Some(sub) = s.get("items").filter(|i| i.is_object() || i.is_boolean()) {
                for (i, v) in items.iter().enumerate() {
                    check(sub, v, &format!("{path}/{i}"))?;
                }
            }
        }
        Value::String(text) => {
            let len = text.chars().count() as u64;
            if let Some(n) = s.get("minLength").and_then(Value::as_u64) {
                if len < n {
                    return Err(format!("{}: needs at least {n} characters", at(path)));
                }
            }
            if let Some(n) = s.get("maxLength").and_then(Value::as_u64) {
                if len > n {
                    return Err(format!("{}: allows at most {n} characters", at(path)));
                }
            }
        }
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or_default();
            if let Some(min) = s.get("minimum").and_then(Value::as_f64) {
                if f < min {
                    return Err(format!("{}: must be at least {min}", at(path)));
                }
            }
            if let Some(max) = s.get("maximum").and_then(Value::as_f64) {
                if f > max {
                    return Err(format!("{}: must be at most {max}", at(path)));
                }
            }
        }
        _ => {}
    }
    if let Some(Value::Array(all)) = s.get("allOf") {
        for sub in all {
            check(sub, value, path)?;
        }
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(options)) = s.get(key) {
            let mut first = None;
            let matched = options
                .iter()
                .filter(|sub| match check(sub, value, path) {
                    Ok(()) => true,
                    Err(e) => {
                        first.get_or_insert(e);
                        false
                    }
                })
                .count();
            let ok = if key == "oneOf" { matched == 1 } else { matched >= 1 };
            if !ok {
                return Err(match (matched, first) {
                    (0, Some(e)) => e,
                    _ => format!("{}: must match exactly one alternative", at(path)),
                });
            }
        }
    }
    Ok(())
}

/// The JSON value of a reply: the text itself, or the body of one Markdown
/// code fence around it (```` ```json ... ``` ````) — models add one even
/// when told not to; nothing else is stripped.
pub fn parse_reply(text: &str) -> Result<Value, serde_json::Error> {
    let trimmed = text.trim();
    match serde_json::from_str(trimmed) {
        Ok(v) => Ok(v),
        Err(e) => {
            let fenced = trimmed
                .strip_prefix("```")
                .and_then(|rest| rest.strip_suffix("```"))
                .map(|body| {
                    let body = body.strip_prefix("json").unwrap_or(body);
                    body.trim()
                });
            match fenced {
                Some(body) => serde_json::from_str(body),
                None => Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reply_schema() -> Value {
        json!({
            "type": "object", "additionalProperties": false, "required": ["reply"],
            "properties": {
                "reply": {"type": "string"},
                "topic": {"type": "object", "properties": {
                    "title": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}, "maxItems": 2}
                }},
                "outcome": {"enum": ["a", "b"]}
            }
        })
    }

    #[test]
    fn accepts_and_rejects_with_a_pointer() {
        let s = reply_schema();
        assert!(validate(&s, &json!({"reply": "hi", "topic": {"tags": ["x"]}})).is_ok());
        assert_eq!(
            validate(&s, &json!({})).unwrap_err(),
            "the reply: missing required field `reply`"
        );
        assert_eq!(
            validate(&s, &json!({"reply": 1})).unwrap_err(),
            "/reply: expected string, got integer"
        );
        assert!(validate(&s, &json!({"reply": "x", "extra": 1}))
            .unwrap_err()
            .contains("unknown field `extra`"));
        assert_eq!(
            validate(&s, &json!({"reply": "x", "topic": {"tags": ["a", "b", "c"]}})).unwrap_err(),
            "/topic/tags: allows at most 2 items"
        );
        assert!(validate(&s, &json!({"reply": "x", "outcome": "c"}))
            .unwrap_err()
            .starts_with("/outcome: must be one of"));
        assert!(validate(&json!({"type": ["string", "null"]}), &Value::Null).is_ok());
        assert!(validate(&json!({"anyOf": [{"type": "string"}, {"type": "integer"}]}), &json!(true)).is_err());
    }

    #[test]
    fn a_fenced_reply_is_unwrapped() {
        assert_eq!(parse_reply(" {\"a\":1} ").unwrap(), json!({"a": 1}));
        assert_eq!(parse_reply("```json\n{\"a\":1}\n```").unwrap(), json!({"a": 1}));
        assert!(parse_reply("here: {\"a\":1}").is_err());
    }
}
