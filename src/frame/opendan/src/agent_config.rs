//! `agent.toml` schema — Gateway + Session-class skeleton.
//!
//! Per doc/opendan/Agent配置改进.md §4. The on-disk file maps directly
//! onto this struct tree:
//!
//! ```text
//! [identity]            agent_did / display_name
//! [runtime]             cancel_reason / language / preserve_attachment_tag_in_egress / filesystem_policy / task_executor / task_route
//! [dispatch]            default_class + ordered match-rule list
//! [session.<class>]     per-class kind / loop_mode / default_behavior /
//!                       session_id_strategy / process_stack_limit / driver
//! ```
//!
//! v0 hardcodes the Gateway inbound channels (`msg_center` + `kevent`) —
//! `[[channel]]` is reserved for a future schema upgrade (see doc §10 #4).
//!
//! Loaded once at `AIAgent::open(root)`. `[session]` is a map keyed by
//! class name and exposed via [`AgentConfig::session_class`] for the
//! dispatcher / session worker.
//!
//! **No backward compatibility** with the pre-beta2.2 5-field schema —
//! see doc §1 and §9.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::behavior_cfg::{BehaviorCfg, BehaviorCfgError};
use crate::i18n::AgentI18n;
use crate::session_model::{SessionKind, TimerEventKind};

/// Subdirectories under the agent root the runtime needs to know about.
/// All paths are absolute, resolved at load time.
#[derive(Debug, Clone)]
pub struct AgentLayout {
    pub root: PathBuf,
    pub behaviors_dir: PathBuf,
    pub sessions_dir: PathBuf,
    pub workspaces_dir: PathBuf,
    pub memory_dir: PathBuf,
    pub notebook_dir: PathBuf,
    pub tools_dir: PathBuf,
    pub tool_plans_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub i18n_dir: PathBuf,
    pub archive_dir: PathBuf,
}

impl AgentLayout {
    pub fn from_root(root: PathBuf) -> Self {
        Self {
            behaviors_dir: root.join("behaviors"),
            sessions_dir: root.join("sessions"),
            workspaces_dir: root.join("workspace"),
            memory_dir: root.join("memory"),
            notebook_dir: root.join("notebook"),
            tools_dir: root.join("tools"),
            tool_plans_dir: root.join("tool_plans"),
            skills_dir: root.join("skills"),
            i18n_dir: root.join("i18n"),
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
    /// Current language for agent-local i18n dictionaries under `i18n/`.
    /// Empty ⇒ `en`.
    pub language: String,
    pub preserve_attachment_tag_in_egress: bool,
    /// Filesystem path policy. `"workspace"` (default) confines file access
    /// to configured roots; `"unrestricted"` lifts the workspace fence so the
    /// agent can read or attach host-readable files. Path traversal (`..`) is
    /// still rejected by each tool.
    pub filesystem_policy: FilesystemPolicy,
    pub task_executor: TaskExecutorCfg,
    pub task_route: TaskRouteCfg,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemPolicy {
    Workspace,
    Unrestricted,
}

impl Default for FilesystemPolicy {
    fn default() -> Self {
        FilesystemPolicy::Workspace
    }
}

impl Default for RuntimeCfg {
    fn default() -> Self {
        Self {
            cancel_reason: String::new(),
            language: "en".to_string(),
            preserve_attachment_tag_in_egress: false,
            filesystem_policy: FilesystemPolicy::default(),
            task_executor: TaskExecutorCfg::default(),
            task_route: TaskRouteCfg::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TaskExecutorCfg {
    pub enabled: bool,
    pub runner_id: String,
    pub poll_interval_ms: u64,
}

impl Default for TaskExecutorCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            runner_id: String::new(),
            // The sweep is a recovery backstop, not the discovery path:
            // activation pushes and the bound-task kevent subscription wake
            // the executor directly, so a minute-level cadence suffices.
            poll_interval_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TaskRouteCfg {
    pub enabled: bool,
}

impl Default for TaskRouteCfg {
    fn default() -> Self {
        Self { enabled: true }
    }
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportDeliveryMode {
    FinalOnly,
    TopLevel,
    All,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SessionHookPoint {
    OnInit,
    OnBehaviorSwitch,
    OnBehaviorStepOb,
    OnWakeup,
}

impl SessionHookPoint {
    pub fn as_key(self) -> &'static str {
        match self {
            SessionHookPoint::OnInit => "on_init",
            SessionHookPoint::OnBehaviorSwitch => "on_behavior_switch",
            SessionHookPoint::OnBehaviorStepOb => "on_behavior_step_ob",
            SessionHookPoint::OnWakeup => "on_wakeup",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SessionDriverCfg {
    pub switch_mode: SwitchMode,
    pub inject_background_environment: bool,
    pub report_delivery: ReportDeliveryMode,
    pub task_feedback: TaskFeedbackCfg,
    pub on_init: HookPointCfg,
    pub on_behavior_switch: HookPointCfg,
    pub on_behavior_step_ob: HookPointCfg,
    pub on_wakeup: HookPointCfg,
}

impl Default for SessionDriverCfg {
    fn default() -> Self {
        Self {
            switch_mode: SwitchMode::Normal,
            inject_background_environment: true,
            report_delivery: ReportDeliveryMode::FinalOnly,
            task_feedback: TaskFeedbackCfg::default(),
            on_init: HookPointCfg {
                filter: BehaviorFilter::All,
                pull_msg: PullMsgPolicy::None,
                pull_event: PullEventPolicy::None,
                load_background_hits: LoadBackgroundHintsPolicy::None,
            },
            on_behavior_switch: HookPointCfg {
                filter: BehaviorFilter::Top,
                pull_msg: PullMsgPolicy::All,
                pull_event: PullEventPolicy::All,
                load_background_hits: LoadBackgroundHintsPolicy::None,
            },
            on_behavior_step_ob: HookPointCfg {
                filter: BehaviorFilter::Top,
                pull_msg: PullMsgPolicy::All,
                pull_event: PullEventPolicy::None,
                load_background_hits: LoadBackgroundHintsPolicy::None,
            },
            on_wakeup: HookPointCfg {
                filter: BehaviorFilter::Top,
                pull_msg: PullMsgPolicy::All,
                pull_event: PullEventPolicy::None,
                load_background_hits: LoadBackgroundHintsPolicy::None,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TaskFeedbackCfg {
    pub enabled: bool,
    pub complete_on_end: bool,
    pub fail_on_error: bool,
    pub cancel_on_interrupt: bool,
    pub create_human_input_on_wait: bool,
}

impl Default for TaskFeedbackCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            complete_on_end: true,
            fail_on_error: true,
            cancel_on_interrupt: true,
            create_human_input_on_wait: true,
        }
    }
}

impl SessionDriverCfg {
    pub fn hook(&self, point: SessionHookPoint) -> &HookPointCfg {
        match point {
            SessionHookPoint::OnInit => &self.on_init,
            SessionHookPoint::OnBehaviorSwitch => &self.on_behavior_switch,
            SessionHookPoint::OnBehaviorStepOb => &self.on_behavior_step_ob,
            SessionHookPoint::OnWakeup => &self.on_wakeup,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HookPointCfg {
    pub filter: BehaviorFilter,
    pub pull_msg: PullMsgPolicy,
    pub pull_event: PullEventPolicy,
    #[serde(default, alias = "load_background_hints")]
    pub load_background_hits: LoadBackgroundHintsPolicy,
}

impl Default for HookPointCfg {
    fn default() -> Self {
        Self {
            filter: BehaviorFilter::None,
            pull_msg: PullMsgPolicy::None,
            pull_event: PullEventPolicy::None,
            load_background_hits: LoadBackgroundHintsPolicy::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BehaviorFilter {
    Top,
    DefaultOnly,
    All,
    None,
    Behavior(String),
}

impl Default for BehaviorFilter {
    fn default() -> Self {
        BehaviorFilter::None
    }
}

impl Serialize for BehaviorFilter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self {
            BehaviorFilter::Top => "top",
            BehaviorFilter::DefaultOnly => "default_only",
            BehaviorFilter::All => "all",
            BehaviorFilter::None => "none",
            BehaviorFilter::Behavior(name) => name.as_str(),
        })
    }
}

impl<'de> Deserialize<'de> for BehaviorFilter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let trimmed = raw.trim();
        Ok(match trimmed {
            "top" => BehaviorFilter::Top,
            "default_only" => BehaviorFilter::DefaultOnly,
            "all" => BehaviorFilter::All,
            "none" | "" => BehaviorFilter::None,
            name => BehaviorFilter::Behavior(name.to_string()),
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PullMsgPolicy {
    None,
    One,
    All,
}

impl Default for PullMsgPolicy {
    fn default() -> Self {
        PullMsgPolicy::None
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadBackgroundHintsPolicy {
    None,
    All,
}

impl Default for LoadBackgroundHintsPolicy {
    fn default() -> Self {
        LoadBackgroundHintsPolicy::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullEventPolicy {
    None,
    All,
    Filter(String),
}

impl Default for PullEventPolicy {
    fn default() -> Self {
        PullEventPolicy::None
    }
}

impl Serialize for PullEventPolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self {
            PullEventPolicy::None => "none",
            PullEventPolicy::All => "all",
            PullEventPolicy::Filter(name) => name.as_str(),
        })
    }
}

impl<'de> Deserialize<'de> for PullEventPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let trimmed = raw.trim();
        Ok(match trimmed {
            "none" | "" => PullEventPolicy::None,
            "all" => PullEventPolicy::All,
            name => PullEventPolicy::Filter(name.to_string()),
        })
    }
}

impl Default for ReportDeliveryMode {
    fn default() -> Self {
        ReportDeliveryMode::FinalOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SessionClassCfg {
    /// Disabled classes stay parseable but are ignored by dispatch and
    /// startup-owned background workers. This keeps beta rollbacks as a
    /// config-only change.
    pub enabled: bool,
    /// Maps the class to the on-disk [`SessionKind`] enum that lifecycle
    /// code branches on (UI vs Work). Default is `Work` — every new class
    /// is autonomous unless explicitly tagged as UI.
    pub kind: SessionKind,
    pub loop_mode: LoopMode,
    /// Behavior name new sessions of this class start with. Empty ⇒
    /// `"<class>_default"` is the implicit fallback.
    pub default_behavior: String,
    pub session_id_strategy: SessionIdStrategy,
    /// Maximum depth of the process stack inside one session (independent
    /// switches push frames). 0 ⇒ unbounded (v0 still accepts this).
    pub process_stack_limit: u32,
    pub driver: SessionDriverCfg,
}

impl Default for SessionClassCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: SessionKind::Work,
            loop_mode: LoopMode::Agent,
            default_behavior: String::new(),
            session_id_strategy: SessionIdStrategy::default(),
            process_stack_limit: 0,
            driver: SessionDriverCfg::default(),
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
    #[error("invalid driver filter in {path}: {message}")]
    InvalidDriverFilter { path: String, message: String },
    #[error(transparent)]
    Behavior(#[from] BehaviorCfgError),
}

/// Loaded agent metadata + filesystem layout. Cheap to clone (paths only).
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub layout: AgentLayout,
    pub toml: AgentTomlFile,
    pub i18n: AgentI18n,
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
            let parsed: AgentTomlFile =
                toml::from_str(&bytes).map_err(|err| AgentConfigError::Parse {
                    path: toml_path.display().to_string(),
                    err,
                })?;
            validate_driver_filters(&toml_path, &parsed)?;
            parsed
        } else {
            AgentTomlFile::default()
        };
        let i18n = AgentI18n::load(&layout.root, toml.runtime.language.as_str());
        Ok(Self { layout, toml, i18n })
    }

    pub fn default_driver_for_kind(&self, kind: SessionKind) -> SessionDriverCfg {
        match kind {
            SessionKind::SelfCheck => self
                .session_class(&self.class_name_for_kind(kind))
                .map(|cfg| cfg.driver.clone())
                .unwrap_or_else(default_self_check_driver),
            SessionKind::SelfImprove => self
                .session_class(&self.class_name_for_kind(kind))
                .map(|cfg| cfg.driver.clone())
                .unwrap_or_else(default_self_improve_driver),
            _ => self
                .session_class(&self.class_name_for_kind(kind))
                .map(|cfg| cfg.driver.clone())
                .unwrap_or_default(),
        }
    }

    pub fn default_loop_mode_for_kind(&self, kind: SessionKind) -> LoopMode {
        self.session_class(&self.class_name_for_kind(kind))
            .map(|cfg| cfg.loop_mode)
            .unwrap_or(if matches!(kind, SessionKind::Ui) {
                LoopMode::Agent
            } else {
                LoopMode::Behavior
            })
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

    pub fn language(&self) -> &str {
        self.i18n.language()
    }

    /// Borrow a session class config by name. The class name comes from
    /// the dispatcher (rule match or `default_class`) or, during restore,
    /// from the on-disk SessionMeta.kind ⇒ class lookup helper.
    pub fn session_class(&self, name: &str) -> Option<&SessionClassCfg> {
        self.toml.session.get(name)
    }

    pub fn session_class_enabled(&self, name: &str) -> bool {
        self.session_class(name)
            .map(|cfg| cfg.enabled)
            .unwrap_or(true)
    }

    /// Resolve a class for an on-disk [`SessionKind`]. Used by the restore
    /// path which only has the persisted kind to go on. Prefer the canonical
    /// "ui" / "work" class when present, then pick the first configured class
    /// with a matching `kind`; fall back to the canonical literal.
    pub fn class_name_for_kind(&self, kind: SessionKind) -> String {
        let canonical = match kind {
            SessionKind::Ui => "ui",
            SessionKind::Work => "work",
            SessionKind::SelfCheck => "self_check",
            SessionKind::SelfImprove => "self_improve",
        };
        if self
            .toml
            .session
            .get(canonical)
            .map(|cfg| cfg.enabled && cfg.kind == kind)
            .unwrap_or(false)
        {
            return canonical.to_string();
        }
        for (name, cfg) in self.toml.session.iter() {
            if cfg.enabled && cfg.kind == kind {
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
            None if class == "self_check" || class == "self_improve" => class.to_string(),
            None => format!("{class}_default"),
        }
    }

    /// Load a behavior by name. Errors if the file is missing or invalid —
    /// callers (session worker) decide whether to fall back to a built-in
    /// default behavior or to surface the error.
    pub fn load_behavior(&self, name: &str) -> Result<BehaviorCfg, AgentConfigError> {
        let exact_path = self.layout.behavior_path(name);
        let mut entries = std::fs::read_dir(&self.layout.behaviors_dir)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect::<Vec<_>>();
        entries.sort();

        let exact_file = format!("{name}.toml");
        let path = entries
            .iter()
            .find(|path| path.file_name().and_then(|s| s.to_str()) == Some(exact_file.as_str()))
            .cloned()
            .or_else(|| {
                entries.into_iter().find(|path| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .map(|stem| stem.eq_ignore_ascii_case(name))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(exact_path);
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
                    "sendmsg".to_string(),
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

fn default_self_check_driver() -> SessionDriverCfg {
    SessionDriverCfg {
        inject_background_environment: false,
        on_init: HookPointCfg {
            filter: BehaviorFilter::All,
            pull_msg: PullMsgPolicy::None,
            pull_event: PullEventPolicy::Filter("timer.*".to_string()),
            load_background_hits: LoadBackgroundHintsPolicy::None,
        },
        on_behavior_switch: HookPointCfg {
            filter: BehaviorFilter::Top,
            pull_msg: PullMsgPolicy::None,
            pull_event: PullEventPolicy::Filter("timer.*".to_string()),
            load_background_hits: LoadBackgroundHintsPolicy::None,
        },
        on_behavior_step_ob: HookPointCfg::default(),
        on_wakeup: HookPointCfg {
            filter: BehaviorFilter::Top,
            pull_msg: PullMsgPolicy::All,
            pull_event: PullEventPolicy::Filter("timer.*".to_string()),
            load_background_hits: LoadBackgroundHintsPolicy::None,
        },
        ..SessionDriverCfg::default()
    }
}

fn default_self_improve_driver() -> SessionDriverCfg {
    SessionDriverCfg {
        inject_background_environment: false,
        on_init: HookPointCfg {
            filter: BehaviorFilter::All,
            pull_msg: PullMsgPolicy::None,
            pull_event: PullEventPolicy::None,
            load_background_hits: LoadBackgroundHintsPolicy::None,
        },
        on_behavior_switch: HookPointCfg {
            filter: BehaviorFilter::Top,
            pull_msg: PullMsgPolicy::None,
            pull_event: PullEventPolicy::None,
            load_background_hits: LoadBackgroundHintsPolicy::None,
        },
        on_behavior_step_ob: HookPointCfg::default(),
        on_wakeup: HookPointCfg::default(),
        ..SessionDriverCfg::default()
    }
}

fn validate_driver_filters(path: &Path, cfg: &AgentTomlFile) -> Result<(), AgentConfigError> {
    for (class, session) in &cfg.session {
        for (hook, hook_cfg) in [
            ("on_init", &session.driver.on_init),
            ("on_behavior_switch", &session.driver.on_behavior_switch),
            ("on_behavior_step_ob", &session.driver.on_behavior_step_ob),
            ("on_wakeup", &session.driver.on_wakeup),
        ] {
            let PullEventPolicy::Filter(name) = &hook_cfg.pull_event else {
                continue;
            };
            if name == "timer.*" || !name.starts_with("timer.") {
                continue;
            }
            if TimerEventKind::parse_event_id(name).is_none() {
                return Err(AgentConfigError::InvalidDriverFilter {
                    path: path.display().to_string(),
                    message: format!(
                        "[session.{class}.driver.{hook}].pull_event uses unknown timer filter `{name}`"
                    ),
                });
            }
        }
    }
    Ok(())
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
        assert_eq!(cfg.language(), "en");
        assert!(cfg.toml.runtime.task_executor.enabled);
        assert!(cfg.toml.runtime.task_route.enabled);
        assert_eq!(cfg.default_behavior_for_class("ui"), "ui_default");
        assert_eq!(cfg.default_behavior_for_class("work"), "work_default");
        assert_eq!(cfg.default_behavior_for_class("self_check"), "self_check");
        assert_eq!(
            cfg.default_driver_for_kind(SessionKind::SelfCheck)
                .on_behavior_switch
                .pull_event,
            PullEventPolicy::Filter("timer.*".to_string())
        );
        assert_eq!(
            cfg.default_driver_for_kind(SessionKind::SelfImprove)
                .on_behavior_switch
                .pull_event,
            PullEventPolicy::None
        );
    }

    #[test]
    fn layout_uses_agent_notebook_root() {
        let root = PathBuf::from("/tmp/agent-root");
        let layout = AgentLayout::from_root(root.clone());
        assert_eq!(layout.notebook_dir, root.join("notebook"));
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
                language = "zh-CN"
                preserve_attachment_tag_in_egress = true
                filesystem_policy = "unrestricted"

                [runtime.task_executor]
                enabled = false

                [runtime.task_route]
                enabled = false

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
                session_id_strategy = "per_peer"

                [session.ui.driver]
                switch_mode = "normal"
                inject_background_environment = true
                report_delivery = "final_only"

                [session.work]
                kind = "work"
                loop_mode = "behavior"
                default_behavior = "work_default"
                session_id_strategy = "per_event_session"
                process_stack_limit = 8

                [session.work.driver]
                inject_background_environment = false
                report_delivery = "top_level"
                switch_mode = "normal"

                [session.work.driver.on_behavior_switch]
                filter = "top"
                pull_msg = "all"
                pull_event = "ban_lifted"

                [session.work.driver.on_wakeup]
                filter = "none"
                pull_msg = "none"
                pull_event = "none"
            "#,
        )
        .unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(cfg.toml.identity.agent_did, "did:dev:alice");
        assert_eq!(cfg.toml.identity.display_name, "Alice");
        assert_eq!(cfg.cancel_reason(), "user canceled");
        assert_eq!(cfg.language(), "zh");
        assert!(!cfg.toml.runtime.task_executor.enabled);
        assert!(!cfg.toml.runtime.task_route.enabled);
        assert!(cfg.toml.runtime.preserve_attachment_tag_in_egress);
        assert_eq!(
            cfg.toml.runtime.filesystem_policy,
            FilesystemPolicy::Unrestricted
        );
        assert_eq!(cfg.toml.dispatch.default_class, "ui");
        assert_eq!(cfg.toml.dispatch.rules.len(), 2);
        assert_eq!(cfg.toml.dispatch.rules[0].on, "msg.chat");
        let ui = cfg.session_class("ui").unwrap();
        assert!(ui.enabled);
        assert_eq!(ui.kind, SessionKind::Ui);
        assert_eq!(ui.loop_mode, LoopMode::Agent);
        assert_eq!(ui.default_behavior, "alice_ui");
        let work = cfg.session_class("work").unwrap();
        assert_eq!(work.kind, SessionKind::Work);
        assert_eq!(work.loop_mode, LoopMode::Behavior);
        assert_eq!(work.session_id_strategy, SessionIdStrategy::PerEventSession);
        assert_eq!(work.process_stack_limit, 8);
        assert!(!work.driver.inject_background_environment);
        assert_eq!(work.driver.report_delivery, ReportDeliveryMode::TopLevel);
        assert_eq!(
            work.driver.on_behavior_switch.pull_event,
            PullEventPolicy::Filter("ban_lifted".to_string())
        );
        assert_eq!(work.driver.on_wakeup.filter, BehaviorFilter::None);
        assert!(ui.driver.inject_background_environment);
        assert_eq!(ui.driver.report_delivery, ReportDeliveryMode::FinalOnly);
    }

    #[test]
    fn driver_hook_point_round_trips() {
        let src = r#"
            [on_init]
            filter = "all"
            pull_msg = "none"
            pull_event = "none"

            [on_behavior_switch]
            filter = "top"
            pull_msg = "all"
            pull_event = "timer.reminder_check"

            [on_behavior_step_ob]
            filter = "plan"
            pull_msg = "one"
            pull_event = "all"

            [on_wakeup]
            filter = "default_only"
            pull_msg = "all"
            pull_event = "none"
            load_background_hits = "all"
        "#;
        let cfg: SessionDriverCfg = toml::from_str(src).unwrap();
        assert_eq!(cfg.on_init.filter, BehaviorFilter::All);
        assert_eq!(cfg.on_behavior_switch.pull_msg, PullMsgPolicy::All);
        assert_eq!(
            cfg.on_behavior_switch.pull_event,
            PullEventPolicy::Filter("timer.reminder_check".to_string())
        );
        assert_eq!(
            cfg.on_behavior_step_ob.filter,
            BehaviorFilter::Behavior("plan".to_string())
        );
        assert_eq!(
            cfg.hook(SessionHookPoint::OnWakeup).pull_msg,
            PullMsgPolicy::All
        );
        assert_eq!(
            cfg.hook(SessionHookPoint::OnWakeup).load_background_hits,
            LoadBackgroundHintsPolicy::All
        );

        let out = toml::to_string(&cfg).unwrap();
        assert!(out.contains("[on_wakeup]"));
        let reparsed: SessionDriverCfg = toml::from_str(&out).unwrap();
        assert_eq!(reparsed, cfg);
    }

    #[test]
    fn rejects_unknown_timer_driver_filter() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent.toml"),
            r#"
                [session.self_check]
                kind = "self_check"

                [session.self_check.driver.on_behavior_switch]
                pull_event = "timer.unknown"
            "#,
        )
        .unwrap();
        let err = AgentConfig::open(dir.path().to_path_buf()).unwrap_err();
        assert!(matches!(err, AgentConfigError::InvalidDriverFilter { .. }));
    }

    #[test]
    fn jarvis_work_session_uses_fork_switch_mode() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rootfs/bin/buckyos_jarvis");
        let cfg = AgentConfig::open(root).unwrap();
        let work = cfg.session_class("work").unwrap();
        assert_eq!(work.kind, SessionKind::Work);
        assert_eq!(work.loop_mode, LoopMode::Behavior);
        assert_eq!(work.default_behavior, "plan");
        assert_eq!(work.driver.switch_mode, SwitchMode::Fork);
        assert_eq!(work.process_stack_limit, 8);
        assert_eq!(work.driver.report_delivery, ReportDeliveryMode::TopLevel);
        let self_check = cfg.session_class("self_check").unwrap();
        assert!(self_check.enabled);
        assert_eq!(self_check.kind, SessionKind::SelfCheck);
        assert_eq!(
            self_check.driver.on_behavior_switch.pull_event,
            PullEventPolicy::Filter("timer.*".to_string())
        );
        assert_eq!(
            self_check.driver.on_wakeup.pull_event,
            PullEventPolicy::Filter("timer.*".to_string())
        );
        assert_eq!(self_check.driver.on_wakeup.pull_msg, PullMsgPolicy::All);
        let self_improve = cfg.session_class("self_improve").unwrap();
        assert!(self_improve.enabled);
        assert_eq!(self_improve.kind, SessionKind::SelfImprove);
        assert_eq!(self_improve.default_behavior, "self_improve_signals");
        assert_eq!(
            self_improve.driver.on_behavior_switch.pull_event,
            PullEventPolicy::None
        );
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
    fn disabled_session_class_is_not_selected_for_kind_lookup() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent.toml"),
            r#"
                [session.self_check]
                enabled = false
                kind = "self_check"
            "#,
        )
        .unwrap();
        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        assert!(!cfg.session_class_enabled("self_check"));
        assert_eq!(
            cfg.class_name_for_kind(SessionKind::SelfCheck),
            "self_check"
        );
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
    fn load_behavior_matches_name_case_insensitively() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("behaviors")).unwrap();
        std::fs::write(
            dir.path().join("behaviors/do.toml"),
            "[meta]\nname = \"do\"\n",
        )
        .unwrap();

        let cfg = AgentConfig::open(dir.path().to_path_buf()).unwrap();
        let behavior = cfg.load_behavior("DO").expect("load lowercase file");
        assert_eq!(behavior.name(), "do");
        let expected_path = dir.path().join("behaviors/do.toml");
        assert_eq!(
            behavior.source_path.as_deref(),
            Some(expected_path.as_path())
        );
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

        let beh = cfg.load_behavior("ui_default").expect("load demo behavior");
        assert_eq!(beh.name(), "ui_default");
        // Echo-bot demo only emits <report> / <next_behavior>, neither of
        // which is a dispatchable invocation — so both whitelists are empty.
        assert!(beh.capabilities.tool_whitelist.is_empty());
        assert!(beh.capabilities.action_whitelist.is_empty());
        assert!(beh.prompt.on_init.contains("{agent_name}"));
    }
}
