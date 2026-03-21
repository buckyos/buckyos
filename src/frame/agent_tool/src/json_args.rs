use serde_json::{Map, Value as Json};

use crate::AgentToolError;

pub fn require_string_arg(args: &Json, key: &str) -> Result<String, AgentToolError> {
    let value = args
        .get(key)
        .and_then(Json::as_str)
        .map(|value| value.to_string())
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("missing or invalid `{key}`")))?;
    if value.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "`{key}` cannot be empty"
        )));
    }
    Ok(value)
}

pub fn require_trimmed_string_arg(args: &Json, key: &str) -> Result<String, AgentToolError> {
    let value = args
        .get(key)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("missing or invalid `{key}`")))?;
    Ok(value.to_string())
}

pub fn optional_string_arg(args: &Json, key: &str) -> Result<Option<String>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    Ok(Some(raw.to_string()))
}

pub fn optional_trimmed_string_arg(
    args: &Json,
    key: &str,
) -> Result<Option<String>, AgentToolError> {
    match args.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Some(_) => Err(AgentToolError::InvalidArgs(format!(
            "`{key}` must be a string"
        ))),
    }
}

pub fn optional_u64_arg(args: &Json, key: &str) -> Result<Option<u64>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be an unsigned integer")))
}

pub fn u64_to_usize_arg(value: u64, key: &str) -> Result<usize, AgentToolError> {
    usize::try_from(value).map_err(|_| {
        AgentToolError::InvalidArgs(format!("`{key}` is too large for current platform"))
    })
}

pub fn read_string_from_map(
    map: &Map<String, Json>,
    key: &str,
) -> Result<Option<String>, AgentToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    Ok(Some(value.to_string()))
}

pub fn read_u64_from_map(
    map: &Map<String, Json>,
    key: &str,
) -> Result<Option<u64>, AgentToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be an integer")))
}

pub fn read_bool_from_map(
    map: &Map<String, Json>,
    key: &str,
) -> Result<Option<bool>, AgentToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a boolean")))
}
