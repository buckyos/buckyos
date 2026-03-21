use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value as Json;

use crate::agent_tool::AgentToolError;

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn normalize_abs_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(seg) => normalized.push(seg),
        }
    }
    normalized
}

pub(crate) fn normalize_root_path(root: &Path) -> Result<PathBuf, AgentToolError> {
    let abs = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| AgentToolError::ExecFailed(format!("read current_dir failed: {err}")))?
            .join(root)
    };
    Ok(normalize_abs_path(&abs))
}

pub(crate) fn resolve_path_from_root(
    root: &Path,
    raw_path: &str,
) -> Result<PathBuf, AgentToolError> {
    if raw_path.trim().is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "path cannot be empty".to_string(),
        ));
    }
    let user_path = Path::new(raw_path);
    let candidate = if user_path.is_absolute() {
        user_path.to_path_buf()
    } else {
        root.join(user_path)
    };
    Ok(normalize_abs_path(&candidate))
}

pub(crate) fn resolve_path_under_root(
    root: &Path,
    raw_path: &str,
) -> Result<PathBuf, AgentToolError> {
    let normalized = resolve_path_from_root(root, raw_path)?;
    if !normalized.starts_with(root) {
        return Err(AgentToolError::InvalidArgs(format!(
            "path out of workspace scope: {raw_path}"
        )));
    }
    Ok(normalized)
}

pub(crate) fn optional_u64_arg(args: &Json, key: &str) -> Result<Option<u64>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be an unsigned integer")))
}

pub(crate) fn u64_to_usize_arg(value: u64, key: &str) -> Result<usize, AgentToolError> {
    usize::try_from(value).map_err(|_| {
        AgentToolError::InvalidArgs(format!("`{key}` is too large for current platform"))
    })
}

pub fn find_string_pointer<'a>(json: &'a Json, pointers: &[&str]) -> Option<&'a str> {
    pointers.iter().find_map(|pointer| {
        json.pointer(pointer)
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}
