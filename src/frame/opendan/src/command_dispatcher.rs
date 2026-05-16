//! §3 — slash-command dispatcher.
//!
//! `llm_context::msg_parser::parse_msg_object` recognizes anything whose
//! `MsgContent.content` starts with `/` as a candidate
//! `SystemControlCommand`. The protocol requires a **strict whitelist**:
//! the command name must be in the registry below, otherwise the message
//! is treated as ordinary text and flows into normal LLM inference. This
//! is what eliminates the "`/etc/nginx/conf` looks like a command"
//! false-positive — the parser is permissive on shape, this dispatcher
//! is strict on identity.
//!
//! Each `CommandSpec` carries a short `summary` so `/help` can enumerate
//! the registry without duplicating documentation.

use crate::agent::AIAgent;
use crate::session_model::SessionKind;
use anyhow::Result;
use std::sync::Arc;

/// Names of all built-in commands. Anything not in this list is **not**
/// treated as a command — the message falls through to LLM inference.
pub const BUILTIN_COMMANDS: &[&str] = &["clear", "list", "switch", "help"];

/// Returns `true` iff `name` matches a built-in command (case-insensitive
/// on the leading byte so `/Help` still works; the rest is exact-match so
/// `/cl3ar` is treated as text, not as `/clear`).
pub fn is_known_command(name: &str) -> bool {
    let n = name.trim();
    BUILTIN_COMMANDS.iter().any(|c| c.eq_ignore_ascii_case(n))
}

/// One inbound command dispatched from `msg_center_pump`'s `parse_msg_object`
/// branch. Wraps the original `record_id` so the agent can ack the
/// underlying msg-center record after the command finishes, mirroring the
/// normal `Inbound::Msg` lifecycle.
#[derive(Debug, Clone)]
pub struct CommandInvocation {
    pub record_id: String,
    pub from: String,
    pub from_did: Option<String>,
    pub tunnel_did: Option<String>,
    pub command: String,
    pub args: String,
}

/// Result of running a command. Carries the reply text that should be
/// sent back as an Assistant-role MsgObject through the same outbound
/// path as a normal LLM reply, so existing tunnel routing applies.
#[derive(Debug, Clone)]
pub struct CommandOutcome {
    pub reply: String,
}

/// Execute a command against the live agent. The function is intentionally
/// small — it only routes by name and delegates the side-effects to
/// existing agent methods (clear/list/switch). Adding new commands means
/// extending `BUILTIN_COMMANDS` and the `match` arm below.
pub async fn run_command(agent: &Arc<AIAgent>, invocation: &CommandInvocation) -> Result<CommandOutcome> {
    let cmd = invocation.command.trim().to_ascii_lowercase();
    match cmd.as_str() {
        "clear" => clear_session(agent, invocation).await,
        "list" => list_sessions(agent).await,
        "switch" => switch_session(agent, invocation).await,
        "help" => Ok(CommandOutcome { reply: render_help() }),
        _ => Ok(CommandOutcome {
            reply: format!(
                "unknown command `/{}` — try /help for the full list",
                invocation.command
            ),
        }),
    }
}

async fn clear_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    // Resolve the session the same way Inbound::Msg does: explicit > tunnel binding.
    let sid = agent.resolve_session_for_command(&inv.from).await?;
    let Some(session) = agent.get_session(&sid).await else {
        return Ok(CommandOutcome {
            reply: format!("no active session for `{}` — nothing to clear", inv.from),
        });
    };
    session.clear_history().await?;
    Ok(CommandOutcome {
        reply: format!("session `{sid}` history cleared"),
    })
}

async fn list_sessions(agent: &Arc<AIAgent>) -> Result<CommandOutcome> {
    let summaries = agent.list_session_summaries(None).await;
    if summaries.is_empty() {
        return Ok(CommandOutcome {
            reply: "no active sessions".to_string(),
        });
    }
    let mut out = String::from("active sessions:\n");
    for s in summaries {
        let kind = match s.kind {
            SessionKind::Ui => "ui",
            SessionKind::Work => "work",
        };
        let title = if s.title.is_empty() {
            "(no title)".to_string()
        } else {
            s.title.clone()
        };
        out.push_str(&format!(
            "  - {} [{kind}] {} — {:?}\n",
            s.session_id, title, s.status
        ));
    }
    Ok(CommandOutcome { reply: out })
}

async fn switch_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    let target = inv.args.trim();
    if target.is_empty() {
        return Ok(CommandOutcome {
            reply: "usage: /switch <session_id>".to_string(),
        });
    }
    if agent.get_session(target).await.is_none() {
        return Ok(CommandOutcome {
            reply: format!("session `{target}` not found"),
        });
    }
    agent.bind_tunnel_to_session(&inv.from, target).await;
    Ok(CommandOutcome {
        reply: format!("tunnel `{}` now bound to session `{target}`", inv.from),
    })
}

fn render_help() -> String {
    let mut out = String::from("available commands:\n");
    out.push_str("  /clear            clear current session history\n");
    out.push_str("  /list             list active sessions on this agent\n");
    out.push_str("  /switch <id>      bind this tunnel to a different session\n");
    out.push_str("  /help             show this message\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_is_case_insensitive() {
        assert!(is_known_command("clear"));
        assert!(is_known_command("CLEAR"));
        assert!(is_known_command("Help"));
        assert!(!is_known_command("etc"));
        assert!(!is_known_command("cl3ar"));
    }
}
