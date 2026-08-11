//! v0 dispatcher + session-id evaluators.
//!
//! These two traits are the seam that future script-engine integration
//! plugs into. v0 ships exactly one impl of each — `FixedRulesDispatch` and
//! `EnumSessionIdStrategy` — driven by the literal `agent.toml`
//! `[dispatch]` / `[session.<class>].session_id_strategy` fields. Future
//! revisions can supply a script-driven evaluator without touching
//! `agent.rs::dispatch_inbound`.
//!
//! Per doc/opendan/Agent配置改进.md §7.1 (the "v0 故意不开表达式"
//! contract): v0 evaluators only support exact match + tail-wildcard
//! (`prefix.*`), and the 4-strategy session-id enum. Anything richer is
//! deferred to the Workflow LLMContext DSL switch.

use serde::{Deserialize, Serialize};

use crate::agent::Inbound;
use crate::agent_config::{DispatchCfg, DispatchRule, SessionIdStrategy};

/// Decide which session-class name handles an inbound event. Implementors
/// are stateless evaluators over the agent's static config.
pub trait DispatchEvaluator: Send + Sync {
    /// `event_type` is the dotted event identifier the channel feeds the
    /// agent with: `"msg.chat"`, `"task_mgr.completed"`, etc. Returns the
    /// matching session-class name, or `None` if the dispatcher should
    /// drop the event (callers usually fall back to the configured
    /// `default_class`).
    fn route(&self, event_type: &str) -> Option<String>;
}

/// Minimal input shape feeding [`SessionIdEvaluator`]. Only the fields
/// the v0 strategies actually look at — pulled out of [`Inbound`] so the
/// evaluator doesn't depend on the runtime's full inbound enum.
#[derive(Debug, Clone)]
pub struct SessionIdInput<'a> {
    pub session_class: &'a str,
    pub from: Option<&'a str>,
    pub from_did: Option<&'a str>,
    pub group_id: Option<&'a str>,
    pub event_session_id: Option<&'a str>,
}

impl<'a> SessionIdInput<'a> {
    /// Borrow-only adapter from [`Inbound`]. Returns `None` for
    /// `Command` (slash commands route by tunnel, not by class).
    pub fn from_inbound(class: &'a str, inbound: &'a Inbound) -> Option<Self> {
        match inbound {
            Inbound::Msg {
                from,
                from_did,
                group_id,
                ai_message: _,
                ..
            } => Some(Self {
                session_class: class,
                from: Some(from.as_str()),
                from_did: from_did.as_deref(),
                group_id: group_id.as_deref(),
                event_session_id: None,
            }),
            Inbound::Event {
                target_session_id, ..
            } => Some(Self {
                session_class: class,
                from: None,
                from_did: None,
                group_id: None,
                event_session_id: target_session_id.as_deref(),
            }),
            Inbound::Command { .. } => None,
        }
    }
}

/// Compute the session id under a given strategy. v0 covers the 4-strategy
/// enum from doc §4.2; a script-driven evaluator can later override the
/// `Singleton`-and-beyond cases without touching the agent's dispatch.
pub trait SessionIdEvaluator: Send + Sync {
    fn compute(&self, strategy: SessionIdStrategy, input: &SessionIdInput<'_>) -> Option<String>;
}

/// v0 dispatcher: walks `[[dispatch.rule]]` in order, applying exact-match
/// or tail-wildcard (`prefix.*`) only. First hit wins. Empty rule list
/// returns `None` so the caller falls back to `default_class`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FixedRulesDispatch {
    rules: Vec<DispatchRule>,
}

impl FixedRulesDispatch {
    pub fn new(cfg: &DispatchCfg) -> Self {
        Self {
            rules: cfg.rules.clone(),
        }
    }

    fn matches(pattern: &str, event_type: &str) -> bool {
        if let Some(prefix) = pattern.strip_suffix(".*") {
            // tail wildcard: `task_mgr.*` matches `task_mgr.completed`.
            // The prefix-only `task_mgr` doesn't match — by design, the
            // wildcard is the only sub-event form v0 accepts.
            event_type
                .strip_prefix(prefix)
                .map(|rest| rest.starts_with('.'))
                .unwrap_or(false)
        } else {
            pattern == event_type
        }
    }
}

impl DispatchEvaluator for FixedRulesDispatch {
    fn route(&self, event_type: &str) -> Option<String> {
        for rule in &self.rules {
            if Self::matches(&rule.on, event_type) {
                return Some(rule.session_class.clone());
            }
        }
        None
    }
}

/// v0 session-id evaluator: 4-strategy enum dispatch with conservative
/// "missing field ⇒ drop" semantics. See doc §4.2 for the strategy table.
#[derive(Debug, Clone, Default)]
pub struct EnumSessionIdStrategy;

impl SessionIdEvaluator for EnumSessionIdStrategy {
    fn compute(&self, strategy: SessionIdStrategy, input: &SessionIdInput<'_>) -> Option<String> {
        match strategy {
            SessionIdStrategy::PerPeer => {
                // The on-disk key is `from_did` when present; fall back to
                // `from` (tunnel name) so locally-injected msgs without a
                // real DID still land on a stable id.
                let key = input.from_did.or(input.from)?;
                let safe = sanitize_segment(key);
                Some(format!("{}-{}", input.session_class, safe))
            }
            SessionIdStrategy::PerGroup => {
                let gid = input.group_id?;
                Some(format!("{}-{}", input.session_class, sanitize_segment(gid)))
            }
            SessionIdStrategy::PerEventSession => input
                .event_session_id
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            SessionIdStrategy::Singleton => Some(input.session_class.to_string()),
        }
    }
}

fn sanitize_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("anon");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_config::{DispatchCfg, DispatchRule};

    fn rule(on: &str, class: &str) -> DispatchRule {
        DispatchRule {
            on: on.to_string(),
            session_class: class.to_string(),
        }
    }

    #[test]
    fn fixed_rules_exact_match() {
        let cfg = DispatchCfg {
            default_class: "ui".to_string(),
            rules: vec![rule("msg.chat", "ui")],
        };
        let d = FixedRulesDispatch::new(&cfg);
        assert_eq!(d.route("msg.chat").as_deref(), Some("ui"));
        assert!(d.route("msg.group").is_none());
    }

    #[test]
    fn fixed_rules_tail_wildcard() {
        let cfg = DispatchCfg {
            default_class: "ui".to_string(),
            rules: vec![rule("task_mgr.*", "work")],
        };
        let d = FixedRulesDispatch::new(&cfg);
        assert_eq!(d.route("task_mgr.completed").as_deref(), Some("work"));
        assert_eq!(d.route("task_mgr.failed").as_deref(), Some("work"));
        // bare prefix doesn't match — only `prefix.<sub>`
        assert!(d.route("task_mgr").is_none());
        // sibling prefix doesn't match
        assert!(d.route("task_mgr_other.completed").is_none());
    }

    #[test]
    fn first_rule_wins() {
        let cfg = DispatchCfg {
            default_class: "ui".to_string(),
            rules: vec![rule("msg.*", "ui"), rule("msg.group", "group")],
        };
        let d = FixedRulesDispatch::new(&cfg);
        // msg.* matches first ⇒ wins
        assert_eq!(d.route("msg.group").as_deref(), Some("ui"));
    }

    #[test]
    fn per_peer_uses_from_did_else_from() {
        let s = EnumSessionIdStrategy;
        let id = s
            .compute(
                SessionIdStrategy::PerPeer,
                &SessionIdInput {
                    session_class: "ui",
                    from: Some("tunnel"),
                    from_did: Some("did:dev:alice"),
                    group_id: None,
                    event_session_id: None,
                },
            )
            .unwrap();
        assert_eq!(id, "ui-did_dev_alice");

        let id = s
            .compute(
                SessionIdStrategy::PerPeer,
                &SessionIdInput {
                    session_class: "ui",
                    from: Some("tunnel"),
                    from_did: None,
                    group_id: None,
                    event_session_id: None,
                },
            )
            .unwrap();
        assert_eq!(id, "ui-tunnel");
    }

    #[test]
    fn singleton_uses_class_name() {
        let s = EnumSessionIdStrategy;
        let id = s
            .compute(
                SessionIdStrategy::Singleton,
                &SessionIdInput {
                    session_class: "scheduler",
                    from: None,
                    from_did: None,
                    group_id: None,
                    event_session_id: None,
                },
            )
            .unwrap();
        assert_eq!(id, "scheduler");
    }

    #[test]
    fn per_group_uses_group_id() {
        let s = EnumSessionIdStrategy;
        let id = s
            .compute(
                SessionIdStrategy::PerGroup,
                &SessionIdInput {
                    session_class: "group",
                    from: Some("tunnel"),
                    from_did: Some("did:dev:alice"),
                    group_id: Some("did:dev:family"),
                    event_session_id: None,
                },
            )
            .unwrap();
        assert_eq!(id, "group-did_dev_family");

        assert!(s
            .compute(
                SessionIdStrategy::PerGroup,
                &SessionIdInput {
                    session_class: "group",
                    from: Some("tunnel"),
                    from_did: None,
                    group_id: None,
                    event_session_id: None,
                },
            )
            .is_none());
    }

    #[test]
    fn per_event_session_requires_event_session_id() {
        let s = EnumSessionIdStrategy;
        assert!(s
            .compute(
                SessionIdStrategy::PerEventSession,
                &SessionIdInput {
                    session_class: "work",
                    from: None,
                    from_did: None,
                    group_id: None,
                    event_session_id: None,
                },
            )
            .is_none());
    }
}
