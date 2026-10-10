//! Session templates (xAgent §4.7): presets of a [`SessionSpec`] selected by
//! `session.class`. A template is resolved when the session is created and
//! lands in existing config fields plus `session.policy`; it is not a
//! protocol object of its own.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::{InputChannel, SessionSpec};
use crate::error::{OpenDanError, Result};
use crate::protocol::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Turns {
    /// One Turn: the session finishes with its result.
    One,
    /// The session waits for input between Turns and never finishes by
    /// itself.
    Unbounded,
    N(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionTemplate {
    pub class: String,
    pub kind: SessionKind,
    pub turns: Turns,
    pub wait_user_msg: WaitPolicy,
    pub completion: CompletionPolicy,
    pub input: InputChannel,
    pub observe: ObserveScope,
    pub load_hints: bool,
    pub default_behavior: Option<String>,
    pub max_process_depth: u8,
    pub max_sub_sessions: u8,
    pub max_session_depth: u8,
    /// Subscriptions the template registers (there is no "delivered without
    /// a subscription" rule).
    pub implicit_subscriptions: Vec<Subscription>,
}

impl SessionTemplate {
    /// The built-in templates: `work`, `ui`, `self_improve`, `self_check`.
    pub fn builtin(class: &str) -> Option<Self> {
        let policy = SessionPolicy::default();
        let base = |kind, turns, wait, input, observe, hints| SessionTemplate {
            class: class.to_string(),
            kind,
            turns,
            wait_user_msg: wait,
            completion: CompletionPolicy::Natural,
            input,
            observe,
            load_hints: hints,
            default_behavior: None,
            max_process_depth: policy.max_process_depth,
            max_sub_sessions: policy.max_sub_sessions,
            max_session_depth: policy.max_session_depth,
            implicit_subscriptions: Vec::new(),
        };
        Some(match class {
            "work" => base(
                SessionKind::Work,
                Turns::One,
                WaitPolicy::FinishFailed,
                InputChannel::None,
                ObserveScope::Events,
                false,
            ),
            // The long-lived dialogue entry: bound to one conversation by
            // its `route_key`, replies leave through the outbound sink.
            "ui" => base(
                SessionKind::Ui,
                Turns::Unbounded,
                WaitPolicy::Allowed,
                InputChannel::Queue,
                ObserveScope::EventsAndActive,
                true,
            ),
            "self_improve" => base(
                SessionKind::SelfImprove,
                Turns::One,
                WaitPolicy::FinishFailed,
                InputChannel::None,
                ObserveScope::Off,
                false,
            ),
            "self_check" => {
                let mut t = base(
                    SessionKind::SelfCheck,
                    Turns::Unbounded,
                    WaitPolicy::FinishFailed,
                    InputChannel::Queue,
                    ObserveScope::Off,
                    true,
                );
                t.implicit_subscriptions.push(Subscription {
                    id: "_self_check_timer".into(),
                    mode: SubscriptionMode::Active,
                    source: SubscriptionSource::Timer {
                        name: "self_check".into(),
                    },
                    watch: Vec::new(),
                });
                t
            }
            _ => return None,
        })
    }

    /// The template of `class`: the built-in one (`work` for an unknown
    /// class) with the overrides of `agent.toml [session.<class>]`.
    pub fn load(class: &str, agent_root: Option<&Path>) -> Result<Self> {
        let over = agent_root
            .map(|r| r.join("agent.toml"))
            .filter(|p| p.is_file())
            .map(|p| {
                let text = std::fs::read_to_string(&p).map_err(|e| OpenDanError::io(&p, e))?;
                let table: toml::Table = toml::from_str(&text)
                    .map_err(|e| OpenDanError::InvalidArgument(format!("{}: {e}", p.display())))?;
                Ok::<_, OpenDanError>(
                    table
                        .get("session")
                        .and_then(|s| s.get(class))
                        .and_then(|c| c.as_table())
                        .cloned(),
                )
            })
            .transpose()?
            .flatten();
        let mut t = match (Self::builtin(class), &over) {
            (Some(t), _) => t,
            // A class of the package: `base` names the built-in it starts from.
            (None, Some(o)) => {
                let base = o.get("base").and_then(|b| b.as_str()).unwrap_or("work");
                let mut t = Self::builtin(base).ok_or_else(|| {
                    OpenDanError::InvalidArgument(format!(
                        "agent.toml [session.{class}] base: unknown built-in `{base}`"
                    ))
                })?;
                t.class = class.to_string();
                t
            }
            (None, None) => {
                return Err(OpenDanError::InvalidArgument(format!(
                    "unknown session class `{class}` (built-in: work, ui, self_improve, self_check; others need agent.toml [session.{class}])"
                )))
            }
        };
        let Some(over) = over else {
            return Ok(t);
        };
        let bad = |key: &str| {
            OpenDanError::InvalidArgument(format!("agent.toml [session.{class}] {key}: bad value"))
        };
        let parse = |key: &str| -> Result<Option<serde_json::Value>> {
            over.get(key)
                .map(|v| serde_json::to_value(v).map_err(|_| bad(key)))
                .transpose()
        };
        if let Some(v) = over.get("turns") {
            t.turns = match (v.as_str(), v.as_integer()) {
                (Some("one"), _) => Turns::One,
                (Some("unbounded"), _) => Turns::Unbounded,
                (_, Some(n)) if n > 0 => Turns::N(n as u64),
                _ => return Err(bad("turns")),
            };
        }
        if let Some(v) = parse("completion")? {
            t.completion = serde_json::from_value(v).map_err(|_| bad("completion"))?;
        }
        if let Some(v) = parse("wait_user_msg")? {
            t.wait_user_msg = serde_json::from_value(v).map_err(|_| bad("wait_user_msg"))?;
        }
        if let Some(v) = parse("input")? {
            t.input = serde_json::from_value(v).map_err(|_| bad("input"))?;
        }
        if let Some(v) = parse("observe")? {
            t.observe = serde_json::from_value(v).map_err(|_| bad("observe"))?;
        }
        if let Some(v) = over.get("load_hints") {
            t.load_hints = v.as_bool().ok_or_else(|| bad("load_hints"))?;
        }
        if let Some(v) = over.get("default_behavior") {
            t.default_behavior = Some(
                v.as_str()
                    .ok_or_else(|| bad("default_behavior"))?
                    .to_string(),
            );
        }
        for (key, slot) in [
            ("max_process_depth", &mut t.max_process_depth),
            ("max_sub_sessions", &mut t.max_sub_sessions),
            ("max_session_depth", &mut t.max_session_depth),
        ] {
            if let Some(v) = over.get(key) {
                *slot = v
                    .as_integer()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| bad(key))?;
            }
        }
        Ok(t)
    }

    pub fn policy(&self) -> SessionPolicy {
        SessionPolicy {
            wait_user_msg: self.wait_user_msg,
            completion: self.completion,
            observe: self.observe,
            load_hints: self.load_hints,
            max_process_depth: self.max_process_depth,
            max_sub_sessions: self.max_sub_sessions,
            max_session_depth: self.max_session_depth,
        }
    }

    /// A spec of this template. Subscriptions that need an external producer
    /// (anything but pulled session state) raise `input` to a queue; call
    /// [`SessionTemplate::settle_input`] after adding subscriptions.
    pub fn spec(&self, objective: impl Into<String>) -> SessionSpec {
        let mut spec = SessionSpec::work(objective);
        spec.kind = self.kind;
        spec.class = self.class.clone();
        spec.end_condition = match self.turns {
            Turns::One => EndCondition::default(),
            Turns::Unbounded => EndCondition {
                kind: EndConditionType::MaxTurns,
                detail: json!({ "n": u64::MAX }),
            },
            Turns::N(n) => EndCondition {
                kind: EndConditionType::MaxTurns,
                detail: json!({ "n": n }),
            },
        };
        spec.policy = self.policy();
        spec.prompt.behavior = self.default_behavior.clone();
        spec.subscriptions = self.implicit_subscriptions.clone();
        spec.input_channel = Some(self.input);
        self.settle_input(&mut spec);
        spec
    }

    /// Decide the input channel once the subscriptions are known.
    pub fn settle_input(&self, spec: &mut SessionSpec) {
        let external = spec
            .subscriptions
            .iter()
            .any(|s| !matches!(s.source, SubscriptionSource::Session { .. }));
        spec.input_channel = Some(
            if external || spec.input_channel == Some(InputChannel::Queue) {
                InputChannel::Queue
            } else {
                self.input
            },
        );
    }
}
