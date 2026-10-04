//! The Loader's part of `agent.toml`. Everything else in that file belongs
//! to `libopendan` (`[session.<class>]` templates) or to the agent package.
//! A wrong configuration is reported at start-up; the Loader never starts
//! with a configuration it only half understands.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use libopendan::protocol::SessionKind;
use libopendan::{InputChannel, SessionTemplate};
use serde::Deserialize;
use serde_json::Value;

pub const ON_MSG_CHAT: &str = "msg.chat";
pub const ON_MSG_GROUP: &str = "msg.group";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiRule {
    /// `msg.chat` | `msg.group`.
    pub on: String,
    pub session_class: String,
    /// `per_peer` | `per_group`: one session per inbox either way; the name
    /// documents which inboxes the rule is meant for.
    #[serde(default)]
    pub sid_strategy: Option<String>,
    #[serde(default)]
    pub idle_unload_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleSwitch {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoaderSection {
    #[serde(default = "yes")]
    pub agent_state_service: bool,
    #[serde(default = "yes")]
    pub webui: bool,
    /// Idle unload of hosted sessions without a rule of their own.
    #[serde(default = "default_idle")]
    pub idle_unload_secs: u64,
    #[serde(default)]
    pub ui: Vec<UiRule>,
    #[serde(default)]
    pub self_check: ModuleSwitch,
    #[serde(default)]
    pub self_improve: ModuleSwitch,
}

fn yes() -> bool {
    true
}

fn default_idle() -> u64 {
    900
}

impl Default for LoaderSection {
    fn default() -> Self {
        Self {
            agent_state_service: true,
            webui: true,
            idle_unload_secs: default_idle(),
            ui: Vec::new(),
            self_check: ModuleSwitch::default(),
            self_improve: ModuleSwitch::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeSection {
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AgentToml {
    #[serde(default)]
    loader: LoaderSection,
    /// Base `.llm_context` of the sessions the Loader creates (provider,
    /// model, tools, runtime), in its TOML form.
    #[serde(default)]
    llm_context: Option<toml::Table>,
    #[serde(default)]
    runtime: RuntimeSection,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub loader: LoaderSection,
    pub llm_context: Value,
    pub language: String,
}

impl AgentConfig {
    /// Read and check `<agent_root>/agent.toml`. A missing file is a
    /// package without Loader modules.
    pub fn load(agent_root: &Path) -> Result<Self> {
        let path = agent_root.join("agent.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let toml: AgentToml =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        let llm_context = match toml.llm_context {
            Some(t) => serde_json::to_value(t).context("[llm_context]")?,
            None => serde_json::json!({ "tools": { "enabled": true } }),
        };
        let cfg = Self {
            loader: toml.loader,
            llm_context,
            language: toml.runtime.language.unwrap_or_else(|| "en".to_string()),
        };
        cfg.check(agent_root)
            .with_context(|| format!("{}", path.display()))?;
        Ok(cfg)
    }

    fn check(&self, agent_root: &Path) -> Result<()> {
        let mut seen = Vec::new();
        for (i, rule) in self.loader.ui.iter().enumerate() {
            let at = format!("[[loader.ui]] #{}", i + 1);
            if !matches!(rule.on.as_str(), ON_MSG_CHAT | ON_MSG_GROUP) {
                bail!("{at}: on = \"{}\" (expected msg.chat | msg.group)", rule.on);
            }
            if seen.contains(&rule.on) {
                bail!("{at}: a second rule for {}", rule.on);
            }
            seen.push(rule.on.clone());
            if let Some(s) = &rule.sid_strategy {
                if !matches!(s.as_str(), "per_peer" | "per_group") {
                    bail!("{at}: sid_strategy = \"{s}\" (expected per_peer | per_group)");
                }
            }
            let template = SessionTemplate::load(&rule.session_class, Some(agent_root))
                .map_err(|e| anyhow!("{at}: {e}"))?;
            if template.kind != SessionKind::Ui {
                bail!(
                    "{at}: session class `{}` is a {} session; a ui entry needs a class based on `ui` ([session.{}] base = \"ui\")",
                    rule.session_class,
                    template.kind.as_str(),
                    rule.session_class
                );
            }
            if template.input != InputChannel::Queue {
                bail!("{at}: session class `{}` has no input queue", rule.session_class);
            }
        }
        Ok(())
    }

    pub fn ui_rule(&self, on: &str) -> Option<&UiRule> {
        self.loader.ui.iter().find(|r| r.on == on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(text: &str) -> Result<AgentConfig> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent.toml"), text).unwrap();
        AgentConfig::load(dir.path())
    }

    #[test]
    fn a_package_without_agent_toml_has_no_modules() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = AgentConfig::load(dir.path()).unwrap();
        assert!(cfg.loader.ui.is_empty() && cfg.loader.agent_state_service);
        assert!(!cfg.loader.self_check.enabled && !cfg.loader.self_improve.enabled);
    }

    #[test]
    fn loader_sections_are_parsed() {
        let cfg = load(
            r#"
[runtime]
language = "zh"

[llm_context]
model = "llm.default"
[llm_context.provider]
type = "buckyos"

[loader]
webui = false

[[loader.ui]]
on = "msg.chat"
session_class = "ui"
sid_strategy = "per_peer"
idle_unload_secs = 60

[[loader.ui]]
on = "msg.group"
session_class = "group"
sid_strategy = "per_group"

[loader.self_check]
enabled = false

[session.ui]
default_behavior = "chat_route"

[session.group]
base = "ui"
default_behavior = "groupchat_route"
"#,
        )
        .unwrap();
        assert_eq!(cfg.language, "zh");
        assert!(!cfg.loader.webui);
        assert_eq!(cfg.llm_context["provider"]["type"], "buckyos");
        assert_eq!(cfg.ui_rule(ON_MSG_CHAT).unwrap().idle_unload_secs, Some(60));
        assert_eq!(cfg.ui_rule(ON_MSG_GROUP).unwrap().session_class, "group");
    }

    #[test]
    fn wrong_configuration_is_refused() {
        for (text, needle) in [
            ("[[loader.ui]]\non = \"timer\"\nsession_class = \"ui\"\n", "expected msg.chat"),
            ("[[loader.ui]]\non = \"msg.chat\"\nsession_class = \"work\"\n", "based on `ui`"),
            ("[[loader.ui]]\non = \"msg.chat\"\nsession_class = \"nope\"\n", "unknown session class"),
            ("[loader]\nwebiu = true\n", "unknown field"),
            ("[[channel]]\ntype = \"msg_center\"\n[loader]\n[[loader.ui]]\non = \"msg.chat\"\nsession_class = \"ui\"\n[[loader.ui]]\non = \"msg.chat\"\nsession_class = \"ui\"\n", "second rule"),
        ] {
            let err = format!("{:#}", load(text).unwrap_err());
            assert!(err.contains(needle), "{text}: {err}");
        }
    }
}
