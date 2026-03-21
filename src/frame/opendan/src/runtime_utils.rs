use std::path::{Path, PathBuf};

use serde_json::Value as Json;

use crate::agent_tool::{
    normalize_abs_path as shared_normalize_abs_path,
    normalize_root_path as shared_normalize_root_path, optional_u64_arg as shared_optional_u64_arg,
    resolve_path_from_root as shared_resolve_path_from_root,
    resolve_path_under_root as shared_resolve_path_under_root,
    u64_to_usize_arg as shared_u64_to_usize_arg, AgentToolError,
};

pub(crate) fn now_ms() -> u64 {
    ::agent_tool::now_ms()
}

pub(crate) fn normalize_abs_path(path: &Path) -> PathBuf {
    shared_normalize_abs_path(path)
}

pub(crate) fn normalize_root_path(root: &Path) -> Result<PathBuf, AgentToolError> {
    shared_normalize_root_path(root)
}

pub(crate) fn resolve_path_from_root(
    root: &Path,
    raw_path: &str,
) -> Result<PathBuf, AgentToolError> {
    shared_resolve_path_from_root(root, raw_path)
}

pub(crate) fn resolve_path_under_root(
    root: &Path,
    raw_path: &str,
) -> Result<PathBuf, AgentToolError> {
    shared_resolve_path_under_root(root, raw_path)
}

pub(crate) fn optional_u64_arg(args: &Json, key: &str) -> Result<Option<u64>, AgentToolError> {
    shared_optional_u64_arg(args, key)
}

pub(crate) fn u64_to_usize_arg(value: u64, key: &str) -> Result<usize, AgentToolError> {
    shared_u64_to_usize_arg(value, key)
}

pub fn find_string_pointer<'a>(json: &'a Json, pointers: &[&str]) -> Option<&'a str> {
    pointers.iter().find_map(|pointer| {
        json.pointer(pointer)
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}
