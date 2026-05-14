//! §9.5 of NewOpenDANRuntime — UI-session default tool wiring.
//!
//! Registers the built-in tool catalogue described in §2 / §3 of the notepad
//! onto an `AgentToolManager`:
//!   - `exec_bash` (with overlay PATH pointing at the session bin)
//!   - `read_file` / `write_file` / `edit_file`
//!   - `glob` / `grep`
//!
//! 4-layer overlay (System / Runtime / Agent / Session) is **stubbed** in MVP:
//! the `BinOverlayConfig` in upstream `agent_tool` only carries a single
//! `bin_dir`. We expose a single "session bin" directory which the agent layer
//! is free to populate by symlinking + scripts from the other three layers at
//! session start. Promoting this to a true 4-layer overlay only requires
//! extending [`BinOverlayConfig`] upstream — no caller-side changes here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_tool::{
    AgentToolManager, BinOverlayConfig, EditFileTool, ExecBashTool, FileToolConfig, GlobTool,
    GrepTool, LlmBashConfig, NoopFileWriteAudit, ReadFileTool, WriteFileTool,
};

/// Layout for the 4 PATH layers as described in §2 of the notepad. MVP holds
/// only the session bin path; the runtime is responsible for populating it
/// (symlinks to system/runtime/agent binaries + on-the-fly session scripts).
#[derive(Debug, Clone)]
pub struct SessionBinLayout {
    pub system_bin: Option<PathBuf>,
    pub runtime_bin: Option<PathBuf>,
    pub agent_bin: Option<PathBuf>,
    /// Live session-bin directory. The runtime creates it at session boot and
    /// symlinks lower layers into it. Becomes the single `BinOverlayConfig.bin_dir`.
    pub session_bin: PathBuf,
}

impl SessionBinLayout {
    /// Create a session-scoped layout. Defaults all higher layers to `None` —
    /// callers that have system/runtime/agent bin paths can `set_*` afterwards.
    pub fn for_session(session_bin: impl Into<PathBuf>) -> Self {
        Self {
            system_bin: None,
            runtime_bin: None,
            agent_bin: None,
            session_bin: session_bin.into(),
        }
    }

    /// Stub: populate `session_bin` by symlinking the lower layers into it
    /// (later-wins ordering: session > agent > runtime > system). MVP only
    /// creates the directory; full symlink synthesis lands when the 4-layer
    /// `BinOverlayConfig` upstream extension is added.
    pub fn ensure_session_bin(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.session_bin)
    }

    /// Convert into the upstream `BinOverlayConfig`. Single-layer for now —
    /// once §2's 4-layer overlay extension lands upstream this will fold the
    /// other three layers in alongside `session_bin`.
    pub fn to_overlay(&self) -> BinOverlayConfig {
        BinOverlayConfig::local(&self.session_bin)
    }
}

/// Bundle of paths used to configure the file tools' read/write roots.
#[derive(Debug, Clone)]
pub struct FsRoots {
    /// Workspace root — both read and write allowed.
    pub workspace_root: PathBuf,
    /// Additional read-only roots (agent root, etc.). Granted read but not write.
    pub extra_read_roots: Vec<PathBuf>,
}

impl FsRoots {
    pub fn workspace_only(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            extra_read_roots: Vec::new(),
        }
    }

    pub fn with_extra_read(mut self, root: impl Into<PathBuf>) -> Self {
        self.extra_read_roots.push(root.into());
        self
    }

    fn to_file_tool_config(&self) -> FileToolConfig {
        let mut cfg = FileToolConfig::new(self.workspace_root.clone());
        cfg.allowed_read_roots.extend(self.extra_read_roots.clone());
        cfg
    }
}

/// Build a tool manager pre-populated with the §3 UI-session defaults.
/// `session_bin` is the overlay PATH entry — the runtime is expected to
/// have created (and optionally symlinked-into) it before this call.
pub fn build_default_tool_manager(
    fs_roots: FsRoots,
    session_bin: &Path,
) -> Arc<AgentToolManager> {
    let manager = AgentToolManager::new();

    let bash_cfg = LlmBashConfig::local_workspace(&fs_roots.workspace_root)
        .with_overlay(BinOverlayConfig::local(session_bin));
    let _ = manager.register_tool(ExecBashTool::new(bash_cfg));

    let file_cfg = fs_roots.to_file_tool_config();
    let audit = Arc::new(NoopFileWriteAudit);
    let _ = manager.register_typed_tool(ReadFileTool::new(file_cfg.clone()));
    let _ = manager.register_typed_tool(WriteFileTool::new(file_cfg.clone(), audit.clone()));
    let _ = manager.register_typed_tool(EditFileTool::new(file_cfg.clone(), audit));
    let _ = manager.register_typed_tool(GlobTool::new(file_cfg.clone()));
    let _ = manager.register_typed_tool(GrepTool::new(file_cfg));

    Arc::new(manager)
}

/// Higher-level convenience used by `AIAgent` on session create — bundles the
/// session bin directory creation + tool manager bootstrap.
pub fn build_session_tools(
    workspace_root: impl Into<PathBuf>,
    session_dir: impl AsRef<Path>,
) -> std::io::Result<Arc<AgentToolManager>> {
    let session_dir = session_dir.as_ref().to_path_buf();
    let layout = SessionBinLayout::for_session(session_dir.join("bin"));
    layout.ensure_session_bin()?;
    let manager = build_default_tool_manager(
        FsRoots::workspace_only(workspace_root),
        &layout.session_bin,
    );
    Ok(manager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn registers_default_tools() {
        let dir = tempdir().unwrap();
        let manager = build_session_tools(dir.path(), dir.path().join("session"))
            .expect("build tools");
        // The LLM-callable subset (`has_tool` resolves through the LLM
        // namespace). write_file / edit_file are upstream-tagged ACTION-only
        // and intentionally not surfaced to the LLM as tool-call specs; they
        // still resolve via `call_tool` since dispatch is namespace-agnostic.
        for name in ["exec_bash", "read_file", "Glob", "Grep"] {
            assert!(manager.has_tool(name), "tool {name} not registered");
        }
        for action_only in ["write_file", "edit_file"] {
            assert!(
                manager.get_any_tool(action_only).is_some(),
                "tool {action_only} not registered"
            );
        }
    }

    #[test]
    fn session_bin_dir_created() {
        let dir = tempdir().unwrap();
        let layout = SessionBinLayout::for_session(dir.path().join("bin"));
        layout.ensure_session_bin().unwrap();
        assert!(layout.session_bin.exists());
        let overlay = layout.to_overlay();
        assert!(overlay.enabled);
    }
}
