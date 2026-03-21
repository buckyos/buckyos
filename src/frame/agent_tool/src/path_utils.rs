use std::path::{Component, Path, PathBuf};

use crate::AgentToolError;

pub fn normalize_abs_path(path: &Path) -> PathBuf {
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

pub fn to_abs_path(path: &Path) -> Result<PathBuf, AgentToolError> {
    if path.is_absolute() {
        Ok(normalize_abs_path(path))
    } else {
        std::env::current_dir()
            .map(|cwd| normalize_abs_path(&cwd.join(path)))
            .map_err(|err| AgentToolError::ExecFailed(format!("read current_dir failed: {err}")))
    }
}

pub fn normalize_root_path(root: &Path) -> Result<PathBuf, AgentToolError> {
    to_abs_path(root)
}

pub fn resolve_path_from_root(root: &Path, raw_path: &str) -> Result<PathBuf, AgentToolError> {
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

pub fn resolve_path_under_root(root: &Path, raw_path: &str) -> Result<PathBuf, AgentToolError> {
    let normalized = resolve_path_from_root(root, raw_path)?;
    if !normalized.starts_with(root) {
        return Err(AgentToolError::InvalidArgs(format!(
            "path out of workspace scope: {raw_path}"
        )));
    }
    Ok(normalized)
}
