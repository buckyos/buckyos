use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::Value as Json;

pub(crate) const WORKSHOP_INDEX_FILE_NAME: &str = "index.json";
pub(crate) const WORKSHOP_TODO_DB_REL_PATH: &str = "todo/todo.db";
pub(crate) const WORKSHOP_WORKLOG_DB_REL_PATH: &str = "worklog/worklog.db";
pub(crate) const LOCAL_WORKSPACE_WORKLOG_DB_REL_PATH: &str = "worklog/worklog.db";
pub(crate) const WORKSPACES_LOCAL_DIR: &str = "workspaces";

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspaceBindingRef {
    pub local_workspace_id: String,
    pub workspace_path: String,
    pub workspace_rel_path: String,
    pub agent_env_root: String,
}

impl WorkspaceBindingRef {
    pub(crate) fn normalized_local_workspace_id(&self) -> Option<String> {
        normalize_optional_text(Some(self.local_workspace_id.as_str()))
    }

    fn normalized_workspace_path(&self) -> Option<PathBuf> {
        non_empty_path_str(&self.workspace_path)
    }

    fn normalized_agent_env_root(&self) -> Option<PathBuf> {
        non_empty_path_str(&self.agent_env_root)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct WorkspaceInfoView {
    binding: Option<WorkspaceBindingRef>,
    local_workspace_id: String,
}

pub(crate) fn resolve_workspace_binding_ref(
    workspace_info: Option<&Json>,
) -> Option<WorkspaceBindingRef> {
    workspace_info_view(workspace_info).and_then(|info| info.binding)
}

pub(crate) fn resolve_bound_workspace_root(workspace_info: Option<&Json>) -> Option<PathBuf> {
    resolve_workspace_binding_ref(workspace_info)
        .and_then(|binding| binding.normalized_workspace_path())
}

pub(crate) fn resolve_bound_workspace_id(workspace_info: Option<&Json>) -> Option<String> {
    let info = workspace_info_view(workspace_info)?;
    normalize_optional_text(Some(info.local_workspace_id.as_str())).or_else(|| {
        info.binding
            .as_ref()
            .and_then(WorkspaceBindingRef::normalized_local_workspace_id)
    })
}

pub(crate) fn resolve_session_workspace_root(
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    resolve_bound_workspace_root(workspace_info).or_else(|| non_empty_path(session_cwd))
}

pub(crate) fn resolve_agent_env_root(
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    resolve_agent_env_root_from_info(workspace_info).or_else(|| non_empty_path(session_cwd))
}

pub(crate) fn resolve_agent_env_root_from_local_workspace_hint(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    let _ = local_workspace_id;
    resolve_agent_env_root(workspace_info, session_cwd)
}

pub(crate) fn resolve_default_local_workspace_path(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    let local_workspace_id = normalize_optional_text(local_workspace_id)?;
    if let Some(info) = workspace_info_view(workspace_info) {
        if let Some(binding) = info.binding.as_ref() {
            let binding_workspace_id =
                normalize_optional_text(Some(binding.local_workspace_id.as_str()))
                    .or_else(|| local_workspace_id_from_info(&info));
            if binding_workspace_id.as_deref() == Some(local_workspace_id.as_str()) {
                if let Some(path) = derive_local_workspace_root_from_path(
                    Path::new(binding.workspace_path.as_str()),
                    local_workspace_id.as_str(),
                )
                .or_else(|| non_empty_path_str(&binding.workspace_path))
                {
                    return Some(path);
                }
            }
        }
    }
    if let Some(path) =
        derive_local_workspace_root_from_path(session_cwd, local_workspace_id.as_str())
    {
        return Some(path);
    }

    resolve_agent_env_root_from_local_workspace_hint(
        Some(local_workspace_id.as_str()),
        workspace_info,
        session_cwd,
    )
    .map(|root| root.join(WORKSPACES_LOCAL_DIR).join(local_workspace_id))
}

fn workspace_info_view(workspace_info: Option<&Json>) -> Option<WorkspaceInfoView> {
    let info = workspace_info?;
    serde_json::from_value::<WorkspaceInfoView>(info.clone()).ok()
}

fn resolve_agent_env_root_from_info(workspace_info: Option<&Json>) -> Option<PathBuf> {
    resolve_workspace_binding_ref(workspace_info)
        .and_then(|binding| binding.normalized_agent_env_root())
}

fn local_workspace_id_from_info(info: &WorkspaceInfoView) -> Option<String> {
    normalize_optional_text(Some(info.local_workspace_id.as_str()))
}

fn derive_local_workspace_root_from_path(path: &Path, local_workspace_id: &str) -> Option<PathBuf> {
    let suffix = Path::new(WORKSPACES_LOCAL_DIR).join(local_workspace_id);
    prefix_through_subpath(path, &suffix)
}

fn prefix_through_subpath(path: &Path, subpath: &Path) -> Option<PathBuf> {
    let (normalized_path_components, normalized_subpath_components) =
        normalized_component_pair(path, subpath)?;
    let start_idx = find_subpath_start(
        normalized_path_components.as_slice(),
        normalized_subpath_components.as_slice(),
    )?;
    build_path_from_components(
        normalized_path_components[..start_idx + normalized_subpath_components.len()].as_ref(),
    )
}

fn normalized_component_pair<'a>(
    path: &'a Path,
    subpath: &'a Path,
) -> Option<(Vec<Component<'a>>, Vec<Component<'a>>)> {
    let normalized_path_components = path
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect::<Vec<_>>();
    let normalized_subpath_components = subpath
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect::<Vec<_>>();
    if normalized_subpath_components.is_empty()
        || normalized_path_components.len() < normalized_subpath_components.len()
    {
        return None;
    }
    Some((normalized_path_components, normalized_subpath_components))
}

fn find_subpath_start(path: &[Component<'_>], subpath: &[Component<'_>]) -> Option<usize> {
    path.windows(subpath.len())
        .position(|window| window == subpath)
}

fn build_path_from_components(components: &[Component<'_>]) -> Option<PathBuf> {
    if components.is_empty() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in components {
        out.push(component.as_os_str());
    }
    Some(out)
}

pub(crate) fn non_empty_path(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path.to_path_buf())
    }
}

fn non_empty_path_str(value: &str) -> Option<PathBuf> {
    normalize_optional_text(Some(value)).map(PathBuf::from)
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        resolve_agent_env_root, resolve_bound_workspace_root, resolve_default_local_workspace_path,
    };

    #[test]
    fn resolve_agent_env_root_prefers_explicit_binding_root() {
        let workspace_info = json!({
            "binding": {
                "local_workspace_id": "ws-demo",
                "workspace_path": "/tmp/demo/workspaces/ws-demo",
                "workspace_rel_path": "workspaces/ws-demo",
                "agent_env_root": "/tmp/demo"
            }
        });

        assert_eq!(
            resolve_agent_env_root(Some(&workspace_info), std::path::Path::new("")),
            Some(std::path::PathBuf::from("/tmp/demo"))
        );
    }

    #[test]
    fn resolve_agent_env_root_requires_explicit_binding_root() {
        let workspace_info = json!({
            "binding": {
                "local_workspace_id": "ws-demo",
                "workspace_path": "/tmp/demo/workspaces/ws-demo"
            }
        });

        assert_eq!(
            resolve_agent_env_root(Some(&workspace_info), std::path::Path::new("")),
            None
        );
    }

    #[test]
    fn resolve_default_local_workspace_path_prefers_bound_workspace_path() {
        let workspace_info = json!({
            "binding": {
                "local_workspace_id": "ws-demo",
                "workspace_path": "/tmp/demo/workspaces/ws-demo",
                "agent_env_root": "/tmp/demo"
            }
        });

        assert_eq!(
            resolve_bound_workspace_root(Some(&workspace_info)),
            Some(std::path::PathBuf::from("/tmp/demo/workspaces/ws-demo"))
        );
        assert_eq!(
            resolve_default_local_workspace_path(
                Some("ws-demo"),
                Some(&workspace_info),
                std::path::Path::new("")
            ),
            Some(std::path::PathBuf::from("/tmp/demo/workspaces/ws-demo"))
        );
    }
}
