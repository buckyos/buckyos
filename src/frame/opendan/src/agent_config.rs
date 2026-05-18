//! `agent.toml` schema — Gateway + Session-class skeleton.
//!
//! Per doc/opendan/Agent配置改进.md §4. The on-disk file maps directly
//! onto this struct tree:
//!
//! ```text
//! [identity]            agent_did / display_name
//! [runtime]             cancel_reason / preserve_attachment_tag_in_egress
//! [[channel]]           Gateway inbound sources (msg_center / kevent / ...)
//! [dispatch]            default_class + ordered match-rule list
//! [session.<class>]     per-class loop_mode / default_behavior / subscribe /
//!                       session_id_strategy / switch_mode / keep_alive / kind
//! ```
//!
//! Loaded once at `AIAgent::open(root)`. `[session]` is a map keyed by
//! class name and exposed via [`AgentConfig::session_class`] for the
//! dispatcher / session worker.
//!
//! **No backward compatibility** with the pre-beta2.2 5-field schema —
//! see doc §1 and §9.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::behavior_cfg::{BehaviorCfg, BehaviorCfgError};
use crate::session_model::SessionKind;

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
    pub tool_plans_dir: PathBuf,
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
            tool_plans_dir: root.join("tool_plans"),
            skills_dir: root.join("skills"),
            archive_dir: root.join("archive"),
            root,
        }
    }

    pub fn behavior_path(&self, name: &str) -> PathBuf {
        self.behaviors_dir.join(format!("{name}.toml"))
    }

    pub fn tool_plan_path(&self, name: &str) -> PathBuf {
        self.tool_plans_dir.join(format!("{name}.toml"))
    }

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(session_id)
    }
}

// ─── `[identity]` ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct IdentityCfg {
    /// Agent's DID. Used as `from` identity for outgoing messages. Empty ⇒
    /// runtime fills in from buckyos identity at bootstrap.
    pub agent_did: String,
    /// Human-friendly display name (logs, UI). Empty ⇒ inferred from directory.
    pub display_name: String,
}

// ─── `[runtime]` ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RuntimeCfg {
    /// Text injected as the `reason` of `Observation::Cancelled` when the
    /// session-layer interrupt path winds down outstanding tool calls.
    /// Empty ⇒ runtime falls back to a built-in default.
    pub cancel_reason: String,
    pub preserve_attachment_tag_in_egress: bool,
}

impl Default for RuntimeCfg {
    fn default() -> Self {
        Self {
            cancel_reason: String::new(),
            preserve_attachment_tag_in_egress: false,
        }
    }
}

// ─── `[[channel]]` ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ChannelCfg {
    /// Channel kind. v0 honors `"msg_center"` and `"kevent"`; unknown
    /// kinds log a warning on load but don't fail boot, leaving room
    /// for plugin-driven channels.
    #[serde(rename = "type")]
    pub kind: String,
    /// Kevent subscription patterns when `kind == "kevent"`. Ignored for
    /// other channel kinds.
    pub filters: Vec<String>,
}

// ─── `[dispatch]` ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DispatchCfg {
    /// Fallback session-class when no rule matches. Required at runtime
    /// (a missing one means inbound events have nowhere to go).
    pub default_class: String,
    /// Ordered match rules. First hit wins. Tail wildcard (`prefix.*`)
    /// is the only sub-event match v0 supports — see doc §7.1.
    #[serde(rename = "rule")]
    pub rules: Vec<DispatchRule>,
}

impl Default for DispatchCfg {
    fn default() -> Self {
        Self {
            default_class: "ui".to_string(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DispatchRule {
    /// Event type pattern: exact (`"msg.chat"`) or tail-wildcard
    /// (`"task_mgr.*"`).
    pub on: String,
    pub session_class: String,
}

// ─── `[session.<class>]` ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    /// Traditional agent loop driven by provider `tool_calls`.
    Agent,
    /// Behavior outer-loop — parser+renderer plug into deps.
    Behavior,
}

impl Default for LoopMode {
    fn default() -> Self {
        LoopMode::Agent
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionIdStrategy {
    /// `<class>-<sanitized_peer>`. UI typical.
    PerPeer,
    /// `<class>-<sanitized_group>`. Group chat.
    PerGroup,
    /// `<event.session_id>`. Worksession routing.
    PerEventSession,
    /// `<class>`. One session per class, agent-global.
    Singleton,
}

impl Default for SessionIdStrategy {
    fn default() -> Self {
        SessionIdStrategy::PerPeer
    }
}

/// Switch mode is a session-class property — the LLM picks `<next_behavior>`
/// but the runtime, not the LLM, decides whether the switch is normal /
/// fork / independent. See doc §4.2.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwitchMode {
    Normal,
    Fork,
    Independent,
}

impl Default for SwitchMode {
    fn default() -> Self {
        SwitchMode::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SessionClassCfg {
    /// Maps the class to the on-disk [`SessionKind`] enum that lifecycle
    /// code branches on (UI vs Work). Default is `Work` — every new class
    /// is autonomous unless explicitly tagged as UI.
    pub kind: SessionKind,
    pub loop_mode: LoopMode,
    /// Behavior name new sessions of this class start with. Empty ⇒
    /// `"<class>_default"` is the implicit fallback.
    pub default_behavior: String,
    /// Kevent patterns every session of this class auto-subscribes to on
    /// creation. Replaces the pre-beta2.2 global `subscribe_events`.
    pub subscribe_events: Vec<String>,
    pub session_id_strategy: SessionIdStrategy,
    pub switch_mode: SwitchMode,
    /// Maximum depth of the process stack inside one session (independent
    /// switches push frames). 0 ⇒ unbounded (v0 still accepts this).
    pub process_stack_limit: u32,
    /// `true` ⇒ session is always "active" (UI). `false` ⇒ active iff
    /// `status != Ended` (Work).
    pub keep_alive: bool,
}

impl Default for SessionClassCfg {
    fn default() -> Self {
        Self {
            kind: SessionKind::Work,
            loop_mode: LoopMode::Agent,
            default_behavior: String::new(),
            subscribe_events: Vec::new(),
            session_id_strategy: SessionIdStrategy::default(),
            switch_mode: SwitchMode::Normal,
            process_stack_limit: 0,
            keep_alive: false,
        }
    }
}

impl SessionClassCfg {
    /// Resolve the default behavior name for this class, falling back to
    /// `"<class_name>_default"` when the file leaves the field blank.
    pub fn default_behavior_or(&self, class_name: &str) -> String {
        let trimmed = self.default_behavior.trim();
        if trimmed.is_empty() {
            format!("{class_name}_default")
        } else {
            trimmed.to_string()
        }
    }
}

// ─── `agent.toml` root ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentTomlFile {
    pub identity: IdentityCfg,
    pub runtime: RuntimeCfg,
    /// `[[channel]]` array. Order is preserved for diagnostics, but the
    /// gateway treats it as a set.
    #[serde(rename = "channel")]
    pub channels: Vec<ChannelCfg>,
    pub dispatch: DispatchCfg,
    /// `[session.<class>]` table. Keys are class names referenced by
    /// `dispatch.rule[*].session_class` and `dispatch.default_class`.
    pub session: BTreeMap<String, SessionClassCfg>,
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

    /// Text used as `Observation::Cancelled.reason` during session-layer
    /// interrupts. Falls back to a built-in default when the on-disk value
    /// is empty (matches the "near-empty agent.toml still boots" contract).
    pub fn cancel_reason(&self) -> &str {
        let configured = self.toml.runtime.cancel_reason.trim();
        if configured.is_empty() {
            "user requested cancel"
        } else {
            configured
        }
    }

    /// Borrow a session class config by name. The class name comes from
    /// the dispatcher (rule match or `default_class`) or, during restore,
    /// from the on-disk SessionMeta.kind ⇒ class lookup helper.
    pub fn session_class(&self, name: &str) -> Option<&SessionClassCfg> {
        self.toml.session.get(name)
    }

    /// Resolve a class for an on-disk [`SessionKind`]. Used by the restore
    /// path which only has the persisted kind to go on. Prefer the canonical
    /// "ui" / "work" class when present, then pick the first configured class
    /// with a matching `kind`; fall back to the canonical literal.
    pub fn class_name_for_kind(&self, kind: SessionKind) -> String {
        let canonical = match kind {
            SessionKind::Ui => "ui",
            SessionKind::Work => "work",
        };
        if self
            .toml
            .session
            .get(canonical)
            .map(|cfg| cfg.kind == kind)
            .unwrap_or(false)
        {
            return canonical.to_string();
        }
        for (name, cfg) in self.toml.session.iter() {
            if cfg.kind == kind {
                return name.clone();
            }
        }
        canonical.to_string()
    }

    /// Default behavior for a class name. Used when ensure_session_inner
    /// gets a fresh session and the meta has no behavior_hint yet.
    pub fn default_behavior_for_class(&self, class: &str) -> String {
        match self.session_class(class) {
            Some(cfg) => cfg.default_behavior_or(class),
            None => format!("{class}_default"),
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
    /// `behaviors/ui_default.toml` is on disk. Keeps first-boot from
    /// requiring any manual setup. The Action set aligns with
    /// `doc/opendan/Agent Actions.md` §1.
    pub fn builtin_ui_default() -> BehaviorCfg {
        use crate::behavior_cfg::{CapabilitiesCfg, MetaCfg};
        BehaviorCfg {
            meta: MetaCfg {
                name: "ui_default".to_string(),
                objective: "interactive UI session".to_string(),
            },
            capabilities: CapabilitiesCfg {
                // No provider-native tools by default — the builtin UI gives
                // the LLM the full XML action surface and nothing else.
                tool_whitelist: Vec::new(),
                action_whitelist: vec![
                    "exec_bash".to_string(),
                    "write_file".to_string(),
                    "edit_file".to_string(),
                    "read".to_string(),
                    "subscribe_event".to_string(),
                    "unsubscribe_event".to_string(),
                ],
                ..Default::default()
            },
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
        assert!(cfg.toml.identity.agent_did.is_empty());
        assert_eq!(cfg.cancel_reason(), "user requested cancel");
        assert_eq!(cfg.default_behavior_for_class("ui"), "ui_default");
        assert_eq!(cfg.default_behavior_for_class("work"), "work_default");
    }

    #[test]
    fn loads_new_schema() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent.toml"),
            r#"
                [identity]
                agent_did = "did:dev:alice"
                display_name = "Alice"

                [runtime]
                cancel_reason = "user canceled"
                preserve_attachment_tag_in_egress = true

                [[channel]]
                type = "msg_center"

                [[channel]]
                type = "kevent"
                filters = ["task_mgr/**"]

                [dispatch]
                default_class = "ui"

                [[dispatch.rule]]
                on = "msg.chat"
                session_class = "ui"

                [[dispatch.rule]]
                on = "task_mgr.*"
                session_class = "work"

                [session.ui]
                kind = "ui"
                loop_mode = "agent"
                default_behavior = "alice_ui"
                subscribe_events = ["msg.incoming"]
                session_id_strategy = "per_peer"
                switch_mode = "normal"
                keep_alive = true

                [session.work]
                kind = "work"
                loop_mode = "behavior"
                default_behavior = "work_default"
                session_id_strategy = "per_event_session"
                switch_mode = "normal"
                process_stack_limit = 8
                keep_alive = false
            "#,
        )
        .unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(cfg.toml.identity.agent_did, "did:dev:alice");
        assert_eq!(cfg.toml.identity.display_name, "Alice");
        assert_eq!(cfg.cancel_reason(), "user canceled");
        assert!(cfg.toml.runtime.preserve_attachment_tag_in_egress);
        assert_eq!(cfg.toml.channels.len(), 2);
        assert_eq!(cfg.toml.channels[1].kind, "kevent");
        assert_eq!(cfg.toml.dispatch.default_class, "ui");
        assert_eq!(cfg.toml.dispatch.rules.len(), 2);
        assert_eq!(cfg.toml.dispatch.rules[0].on, "msg.chat");
        let ui = cfg.session_class("ui").unwrap();
        assert_eq!(ui.kind, SessionKind::Ui);
        assert_eq!(ui.loop_mode, LoopMode::Agent);
        assert_eq!(ui.default_behavior, "alice_ui");
        assert!(ui.keep_alive);
        assert_eq!(ui.subscribe_events, vec!["msg.incoming"]);
        let work = cfg.session_class("work").unwrap();
        assert_eq!(work.kind, SessionKind::Work);
        assert_eq!(work.loop_mode, LoopMode::Behavior);
        assert_eq!(work.session_id_strategy, SessionIdStrategy::PerEventSession);
        assert_eq!(work.process_stack_limit, 8);
        assert!(!work.keep_alive);
    }

    #[test]
    fn class_name_for_kind_prefers_canonical_then_first_match() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent.toml"),
            r#"
                [session.group]
                kind = "ui"

                [session.ui]
                kind = "ui"

                [session.chat]
                kind = "ui"

                [session.ops]
                kind = "work"
            "#,
        )
        .unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(cfg.class_name_for_kind(SessionKind::Ui), "ui");
        assert_eq!(cfg.class_name_for_kind(SessionKind::Work), "ops");
    }

    #[test]
    fn class_name_for_kind_falls_back_to_canonical_names() {
        let dir = tempdir().unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(cfg.class_name_for_kind(SessionKind::Ui), "ui");
        assert_eq!(cfg.class_name_for_kind(SessionKind::Work), "work");
    }

    #[test]
    fn list_behaviors() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("behaviors")).unwrap();
        std::fs::write(
            dir.path().join("behaviors/ui_default.toml"),
            "[meta]\nname = \"ui_default\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("behaviors/explorer.toml"),
            "[meta]\nname = \"explorer\"\n",
        )
        .unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        let names = cfg.list_behavior_names();
        assert_eq!(names, vec!["explorer", "ui_default"]);
    }

    #[test]
    fn builtin_ui_default_has_tools() {
        let b = AgentConfig::builtin_ui_default();
        assert_eq!(b.meta.name, "ui_default");
        // exec_bash is an XML action, not a provider-native tool — it lives
        // on the action surface post-beta2.2 split.
        assert!(b
            .capabilities
            .action_whitelist
            .contains(&"exec_bash".to_string()));
        assert!(b.capabilities.tool_whitelist.is_empty());
    }

    /// Pins the on-disk minimal demo (`doc/opendan/mini_agent_demo/`) into
    /// the test suite. Any schema drift that makes the demo file stop
    /// parsing trips here so README/example/code stay in sync.
    #[test]
    fn mini_agent_demo_parses() {
        let demo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../doc/opendan/mini_agent_demo");
        let cfg = AgentConfig::open(demo_root.clone()).expect("open demo agent root");
        assert_eq!(cfg.toml.identity.display_name, "echo-bot");
        assert_eq!(cfg.toml.dispatch.default_class, "ui");
        let ui = cfg.session_class("ui").expect("demo defines [session.ui]");
        assert_eq!(ui.default_behavior, "ui_default");
        assert!(ui.keep_alive);

        let beh = cfg.load_behavior("ui_default").expect("load demo behavior");
        assert_eq!(beh.name(), "ui_default");
        // Echo-bot demo only emits <report> / <next_behavior>, neither of
        // which is a dispatchable invocation — so both whitelists are empty.
        assert!(beh.capabilities.tool_whitelist.is_empty());
        assert!(beh.capabilities.action_whitelist.is_empty());
        assert!(beh.prompt.on_init.contains("{agent_name}"));
    }
}
