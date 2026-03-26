use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub fn resolve_schema(schema: &Value, defs: &BTreeMap<String, Value>) -> Option<Value> {
    if let Some(ref_name) = schema.get("$ref").and_then(Value::as_str) {
        let name = ref_name.strip_prefix("#/defs/")?;
        return defs.get(name).cloned();
    }
    Some(schema.clone())
}

pub fn schema_at_path(
    schema: &Value,
    path: &[String],
    defs: &BTreeMap<String, Value>,
) -> Option<Value> {
    let mut current = resolve_schema(schema, defs)?;
    if path.is_empty() {
        return Some(current);
    }

    for segment in path {
        current = resolve_schema(&current, defs)?;
        let object = current.as_object()?;
        let properties = object.get("properties")?.as_object()?;
        current = properties.get(segment)?.clone();
    }
    resolve_schema(&current, defs)
}

pub fn schema_enum_values(schema: &Value, defs: &BTreeMap<String, Value>) -> Option<Vec<String>> {
    let resolved = resolve_schema(schema, defs)?;
    resolved
        .get("enum")?
        .as_array()?
        .iter()
        .map(|value| value.as_str().map(|item| item.to_string()))
        .collect::<Option<Vec<_>>>()
}

pub fn schema_accepts_null(schema: &Value, defs: &BTreeMap<String, Value>) -> bool {
    let Some(resolved) = resolve_schema(schema, defs) else {
        return false;
    };

    if let Some(type_value) = resolved.get("type") {
        match type_value {
            Value::String(name) => return name == "null",
            Value::Array(items) => {
                return items.iter().any(|item| item.as_str() == Some("null"));
            }
            _ => {}
        }
    }

    for keyword in ["anyOf", "oneOf"] {
        if let Some(branches) = resolved.get(keyword).and_then(Value::as_array) {
            if branches.iter().any(|item| schema_accepts_null(item, defs)) {
                return true;
            }
        }
    }

    false
}

pub fn schemas_compatible(
    actual: &Value,
    expected: &Value,
    defs: &BTreeMap<String, Value>,
) -> bool {
    let Some(actual) = resolve_schema(actual, defs) else {
        return false;
    };
    let Some(expected) = resolve_schema(expected, defs) else {
        return false;
    };

    let actual_type = schema_type_names(&actual);
    let expected_type = schema_type_names(&expected);
    if !expected_type.is_empty()
        && !actual_type.is_empty()
        && actual_type.iter().all(|kind| !expected_type.contains(kind))
    {
        return false;
    }

    if expected.get("enum").is_some() && actual.get("enum").is_some() {
        let actual_enum = schema_enum_values(&actual, defs).unwrap_or_default();
        let expected_enum = schema_enum_values(&expected, defs).unwrap_or_default();
        if actual_enum.iter().any(|item| !expected_enum.contains(item)) {
            return false;
        }
    }

    if expected_type.contains(&"object".to_string()) {
        let expected_required = expected
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let actual_props = expected_or_empty(actual.get("properties"));
        let expected_props = expected_or_empty(expected.get("properties"));
        for required in expected_required {
            let Some(required_name) = required.as_str() else {
                continue;
            };
            let Some(actual_schema) = actual_props.get(required_name) else {
                return false;
            };
            let Some(expected_schema) = expected_props.get(required_name) else {
                return false;
            };
            if !schemas_compatible(actual_schema, expected_schema, defs) {
                return false;
            }
        }
    }

    if expected_type.contains(&"array".to_string()) {
        match (actual.get("items"), expected.get("items")) {
            (Some(actual_items), Some(expected_items)) => {
                if !schemas_compatible(actual_items, expected_items, defs) {
                    return false;
                }
            }
            (_, Some(_)) => return false,
            _ => {}
        }
    }

    true
}

pub fn schemas_equal(left: &Value, right: &Value, defs: &BTreeMap<String, Value>) -> bool {
    let Some(left) = normalize_schema(resolve_schema(left, defs).unwrap_or(Value::Null), defs)
    else {
        return false;
    };
    let Some(right) = normalize_schema(resolve_schema(right, defs).unwrap_or(Value::Null), defs)
    else {
        return false;
    };
    left == right
}

fn schema_type_names(schema: &Value) -> Vec<String> {
    match schema.get("type") {
        Some(Value::String(value)) => vec![value.to_string()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|item| item.as_str().map(|value| value.to_string()))
            .collect(),
        _ => vec![],
    }
}

fn expected_or_empty(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new)
}

fn normalize_schema(schema: Value, defs: &BTreeMap<String, Value>) -> Option<Value> {
    let mut resolved = resolve_schema(&schema, defs)?;
    match &mut resolved {
        Value::Object(map) => {
            if let Some(properties) = map.get_mut("properties").and_then(Value::as_object_mut) {
                let keys = properties.keys().cloned().collect::<Vec<_>>();
                for key in keys {
                    let value = properties.get(&key)?.clone();
                    properties.insert(key, normalize_schema(value, defs)?);
                }
            }
            if let Some(items) = map.get("items").cloned() {
                map.insert("items".to_string(), normalize_schema(items, defs)?);
            }
            Some(Value::Object(map.clone()))
        }
        _ => Some(resolved),
    }
}
