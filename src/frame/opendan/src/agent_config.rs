//! §9.3 of NewOpenDANRuntime — Agent-level config.
//!
//! Loaded once at `AIAgent::open(root)`. Carries:
//!   - identity (agent_did / human-readable name / role.md / self.md files)
//!   - the default behavior name to pick on UI-session creation
//!   - the set of event types this agent subscribes to
//!   - simple paths to AgentRootFS subdirectories (memory, notepads, etc.)
//!
//! `behaviors/` directory is *enumerated*; each `.toml` produces a
//! `BehaviorCfg`. The agent_config doesn't embed behaviors — `AgentSession`
//! looks them up by name through [`AgentConfig::load_behavior`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::behavior_cfg::{BehaviorCfg, BehaviorCfgError};

/// Subdirectories under the agent root the runtime needs to know about.
/// All paths are absolute, resolved at load time.
#[derive(Debug, Clone)]
pub struct AgentLayout {
    pub root: PathBuf,
    pub behaviors_dir: PathBuf,
    pub sessions_dir: PathBuf,
    pub workspaces_dir: PathBuf,
    pub memory_dir: PathBuf,
    pub notepads_dir: PathBuf,
    pub tools_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub archive_dir: PathBuf,
}

impl AgentLayout {
    pub fn from_root(root: PathBuf) -> Self {
        Self {
            behaviors_dir: root.join("behaviors"),
            sessions_dir: root.join("session"),
            workspaces_dir: root.join("workspace"),
            memory_dir: root.join("memory"),
            notepads_dir: root.join("notepads"),
            tools_dir: root.join("tools"),
            skills_dir: root.join("skills"),
            archive_dir: root.join("archive"),
            root,
        }
    }

    pub fn behavior_path(&self, name: &str) -> PathBuf {
        self.behaviors_dir.join(format!("{name}.toml"))
    }

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(session_id)
    }
}

/// On-disk `agent.toml` (lives at agent_root/agent.toml). Everything default-able
/// so a near-empty file still boots.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentTomlFile {
    /// Agent's DID. Used as `from` identity for outgoing messages. Empty ⇒
    /// runtime fills in from buckyos identity at bootstrap.
    pub agent_did: String,
    /// Human-friendly display name (logs, UI). Empty ⇒ inferred from directory.
    pub display_name: String,
    /// Default behavior name picked when a UI session is created. Falls back
    /// to `"ui_default"` if empty.
    pub default_ui_behavior: String,
    /// Default behavior name picked when `try_create_worksession` creates a
    /// work session without a behavior hint. Falls back to `"work_default"`.
    pub default_work_behavior: String,
    /// Event types this agent listens for. Plumbed through to task_mgr at
    /// `AIAgent::init_subscribers` time.
    pub subscribe_events: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentConfigError {
    #[error("read {path}: {err}")]
    Io { path: String, err: std::io::Error },
    #[error("parse {path}: {err}")]
    Parse { path: String, err: toml::de::Error },
    #[error(transparent)]
    Behavior(#[from] BehaviorCfgError),
}

/// Loaded agent metadata + filesystem layout. Cheap to clone (paths only).
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub layout: AgentLayout,
    pub toml: AgentTomlFile,
}

impl AgentConfig {
    /// Open an agent root. Missing `agent.toml` is tolerated — the runtime
    /// proceeds with defaults so first-boot scenarios don't trip up.
    pub fn open(root: PathBuf) -> Result<Self, AgentConfigError> {
        let layout = AgentLayout::from_root(root);
        let toml_path = layout.root.join("agent.toml");
        let toml: AgentTomlFile = if toml_path.exists() {
            let bytes =
                std::fs::read_to_string(&toml_path).map_err(|err| AgentConfigError::Io {
                    path: toml_path.display().to_string(),
                    err,
                })?;
            toml::from_str(&bytes).map_err(|err| AgentConfigError::Parse {
                path: toml_path.display().to_string(),
                err,
            })?
        } else {
            AgentTomlFile::default()
        };
        Ok(Self { layout, toml })
    }

    pub fn default_ui_behavior(&self) -> &str {
        if self.toml.default_ui_behavior.trim().is_empty() {
            "ui_default"
        } else {
            self.toml.default_ui_behavior.as_str()
        }
    }

    pub fn default_work_behavior(&self) -> &str {
        if self.toml.default_work_behavior.trim().is_empty() {
            "work_default"
        } else {
            self.toml.default_work_behavior.as_str()
        }
    }

    /// Load a behavior by name. Errors if the file is missing or invalid —
    /// callers (session worker) decide whether to fall back to a built-in
    /// default behavior or to surface the error.
    pub fn load_behavior(&self, name: &str) -> Result<BehaviorCfg, AgentConfigError> {
        let path = self.layout.behavior_path(name);
        Ok(BehaviorCfg::load_from_file(&path)?)
    }

    /// Synthesize a minimal built-in `ui_default` behavior when no
    /// behaviors/ui_default.toml is present on disk. Keeps first-boot from
    /// requiring any manual setup.
    pub fn builtin_ui_default() -> BehaviorCfg {
        BehaviorCfg {
            name: "ui_default".to_string(),
            objective: "interactive UI session".to_string(),
            tool_whitelist: vec![
                "exec_bash".to_string(),
                "read_file".to_string(),
                "glob".to_string(),
                "grep".to_string(),
                "edit_file".to_string(),
                "write_file".to_string(),
            ],
            ..Default::default()
        }
    }

    /// Walk `behaviors/` and return all valid behavior names (no .toml suffix).
    /// Used at boot for `restore_active_sessions` / config diagnostics.
    pub fn list_behavior_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.layout.behaviors_dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.push(stem.to_string());
            }
        }
        out.sort();
        out
    }
}

/// Convenience: build an AgentConfig + its layout from a string path. Errors
/// pre-validated so the caller can attach `?` directly.
pub fn open_agent_root(root: impl AsRef<Path>) -> Result<AgentConfig, AgentConfigError> {
    AgentConfig::open(root.as_ref().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_when_no_toml() {
        let dir = tempdir().unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(cfg.default_ui_behavior(), "ui_default");
        assert_eq!(cfg.default_work_behavior(), "work_default");
        assert!(cfg.toml.agent_did.is_empty());
    }

    #[test]
    fn loads_agent_toml() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent.toml"),
            r#"
                agent_did = "did:dev:alice"
                display_name = "Alice"
                default_ui_behavior = "alice_ui"
                subscribe_events = ["msg.incoming", "task.completed"]
            "#,
        )
        .unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(cfg.toml.agent_did, "did:dev:alice");
        assert_eq!(cfg.default_ui_behavior(), "alice_ui");
        assert_eq!(cfg.toml.subscribe_events.len(), 2);
    }

    #[test]
    fn list_behaviors() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("behaviors")).unwrap();
        std::fs::write(
            dir.path().join("behaviors/ui_default.toml"),
            "name = \"ui_default\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("behaviors/explorer.toml"),
            "name = \"explorer\"\n",
        )
        .unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        let names = cfg.list_behavior_names();
        assert_eq!(names, vec!["explorer", "ui_default"]);
    }

    #[test]
    fn builtin_ui_default_has_tools() {
        let b = AgentConfig::builtin_ui_default();
        assert_eq!(b.name, "ui_default");
        assert!(b.tool_whitelist.contains(&"exec_bash".to_string()));
    }
}
