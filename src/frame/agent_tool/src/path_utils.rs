use std::path::{Component, Path, PathBuf};

use crate::AgentToolError;

pub const MAX_SESSION_ID_LEN: usize = 180;

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

pub fn sanitize_session_id_for_path(session_id: &str) -> Result<String, AgentToolError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot be empty".to_string(),
        ));
    }
    if session_id.len() > MAX_SESSION_ID_LEN {
        return Err(AgentToolError::InvalidArgs(format!(
            "session_id too long (>{MAX_SESSION_ID_LEN})"
        )));
    }
    if session_id == "." || session_id == ".." {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot be `.` or `..`".to_string(),
        ));
    }
    if session_id.contains('/') || session_id.contains('\\') {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot contain path separators".to_string(),
        ));
    }
    if session_id.chars().any(|ch| ch.is_control()) {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot contain control characters".to_string(),
        ));
    }
    Ok(session_id.to_string())
}

pub fn session_record_path(
    sessions_root: &Path,
    session_id: &str,
    file_name: &str,
) -> Result<PathBuf, AgentToolError> {
    let session_id = sanitize_session_id_for_path(session_id)?;
    Ok(sessions_root.join(session_id).join(file_name))
}
