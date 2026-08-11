//! §3 — slash-command dispatcher.
//!
//! `llm_context::msg_parser::parse_msg_object` receives the registry below
//! and only returns `SystemControlCommand` when the message exactly matches
//! one of these command names. Everything else is treated as ordinary text
//! and flows into normal LLM inference, including paths such as
//! `/etc/nginx/conf`.
//!
//! Each `CommandSpec` carries a short `summary` so `/help` can enumerate
//! the registry without duplicating documentation.

use crate::agent::AIAgent;
use crate::agent_session::{AgentSession, ManualCompressOutcome};
use crate::session_model::{InterruptMode, SessionSummary};
use anyhow::Result;
use std::sync::Arc;

/// Names of all built-in commands. Anything not in this list is **not**
/// treated as a command — the message falls through to LLM inference.
pub const BUILTIN_COMMANDS: &[&str] = &[
    "new", "clean", "stop", "cancel", "info", "list", "switch", "compress", "help",
];

/// Returns `true` iff `name` matches a built-in command (case-insensitive
/// on the leading byte so `/Help` still works; the rest is exact-match so
/// `/cl3an` is treated as text, not as `/clean`).
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
/// existing agent methods. Adding new commands means
/// extending `BUILTIN_COMMANDS` and the `match` arm below.
pub async fn run_command(
    agent: &Arc<AIAgent>,
    invocation: &CommandInvocation,
) -> Result<CommandOutcome> {
    let cmd = invocation.command.trim().to_ascii_lowercase();
    match cmd.as_str() {
        "new" => new_session(agent, invocation).await,
        "clean" => clean_session(agent, invocation).await,
        "stop" => stop_session(agent, invocation).await,
        "cancel" => cancel_session(agent, invocation).await,
        "info" => info_session(agent, invocation).await,
        "list" => list_sessions(agent).await,
        "switch" => switch_session(agent, invocation).await,
        "compress" => compress_session(agent, invocation).await,
        "help" => Ok(CommandOutcome {
            reply: render_help(),
        }),
        _ => Ok(CommandOutcome {
            reply: format!(
                "unknown command `/{}` — try /help for the full list",
                invocation.command
            ),
        }),
    }
}

async fn new_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    let sid = agent
        .clone()
        .create_ui_session_for_tunnel(
            &inv.from,
            inv.from_did.as_deref(),
            inv.tunnel_did.as_deref(),
        )
        .await?;
    Ok(CommandOutcome {
        reply: format!("new session `{sid}` created"),
    })
}

async fn clean_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    let previous = agent.resolve_session_for_command(&inv.from).await.ok();
    if let Some(sid) = previous.as_ref() {
        agent.unbind_tunnel_if_session(&inv.from, sid).await;
        agent.delete_session_physical(sid).await?;
    }
    let sid = agent
        .clone()
        .create_ui_session_for_tunnel(
            &inv.from,
            inv.from_did.as_deref(),
            inv.tunnel_did.as_deref(),
        )
        .await?;
    let reply = match previous {
        Some(old) => format!("session `{old}` deleted; new session `{sid}` created"),
        None => format!("new session `{sid}` created"),
    };
    Ok(CommandOutcome { reply })
}

async fn stop_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    control_current_session(agent, inv, InterruptMode::Graceful, "stopped").await
}

async fn cancel_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    control_current_session(agent, inv, InterruptMode::Discard, "cancelled").await
}

async fn control_current_session(
    agent: &Arc<AIAgent>,
    inv: &CommandInvocation,
    mode: InterruptMode,
    verb: &str,
) -> Result<CommandOutcome> {
    let (sid, session) = ensure_current_session(agent, inv).await?;
    session.interrupt(mode).await?;
    Ok(CommandOutcome {
        reply: format!("session `{sid}` {verb}"),
    })
}

async fn info_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    let session = match ensure_current_session(agent, inv).await {
        Ok((_sid, session)) => session,
        Err(_) => {
            return Ok(CommandOutcome {
                reply: render_agent_status(agent, &inv.from).await,
            });
        }
    };
    Ok(CommandOutcome {
        reply: render_summary(&session.summary().await),
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
        let kind = s.kind.as_str();
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

fn render_summary(s: &SessionSummary) -> String {
    let kind = s.kind.as_str();
    let title = if s.title.is_empty() {
        "(no title)"
    } else {
        s.title.as_str()
    };
    let mut out = String::new();
    out.push_str(&format!("session: {}\n", s.session_id));
    out.push_str(&format!("kind: {kind}\n"));
    out.push_str(&format!("title: {title}\n"));
    out.push_str(&format!("status: {:?}\n", s.status));
    out.push_str(&format!("behavior: {}", s.current_behavior));
    if !s.one_line_status.trim().is_empty() {
        out.push_str(&format!("\nactivity: {}", s.one_line_status.trim()));
    }
    out
}

async fn render_agent_status(agent: &Arc<AIAgent>, from: &str) -> String {
    let summaries = agent.list_session_summaries(None).await;
    let mut out = String::new();
    out.push_str(&format!("tunnel: {from}\n"));
    out.push_str("current session: none\n");
    out.push_str(&format!("active sessions: {}", summaries.len()));
    if !summaries.is_empty() {
        out.push('\n');
        for s in summaries {
            let kind = s.kind.as_str();
            out.push_str(&format!("  - {} [{kind}] {:?}\n", s.session_id, s.status));
        }
        if out.ends_with('\n') {
            out.pop();
        }
    }
    out
}

async fn switch_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    let target = inv.args.trim();
    if target.is_empty() {
        return Ok(CommandOutcome {
            reply: "usage: /switch <session_id>".to_string(),
        });
    }
    if agent.clone().ensure_session(target).await.is_err() {
        return Ok(CommandOutcome {
            reply: format!("session `{target}` not found"),
        });
    }
    agent.bind_tunnel_to_session(&inv.from, target).await;
    Ok(CommandOutcome {
        reply: format!("tunnel `{}` now bound to session `{target}`", inv.from),
    })
}

async fn compress_session(agent: &Arc<AIAgent>, inv: &CommandInvocation) -> Result<CommandOutcome> {
    let (sid, session) = ensure_current_session(agent, inv).await?;
    let reply = match session.compress_context_window_once().await {
        Ok(ManualCompressOutcome::NoSnapshot) => {
            format!("session `{sid}` has no saved context to compress")
        }
        Ok(ManualCompressOutcome::NoChange {
            messages,
            tokens,
            target_tokens,
        }) => format!(
            "session `{sid}` context already within target: {messages} messages, {tokens} tokens (target {target_tokens}); no compression applied"
        ),
        Ok(ManualCompressOutcome::Applied {
            before_messages,
            after_messages,
            before_tokens,
            after_tokens,
            target_tokens,
        }) => format!(
            "session `{sid}` compressed context: messages {before_messages} -> {after_messages}, tokens {before_tokens} -> {after_tokens} (target {target_tokens})"
        ),
        Err(err) => format!("session `{sid}` compress failed: {err}"),
    };
    Ok(CommandOutcome { reply })
}

async fn ensure_current_session(
    agent: &Arc<AIAgent>,
    inv: &CommandInvocation,
) -> Result<(String, Arc<AgentSession>)> {
    let sid = agent.resolve_session_for_command(&inv.from).await?;
    let session = agent.clone().ensure_session(&sid).await?;
    Ok((sid, session))
}

fn render_help() -> String {
    let mut out = String::from("available commands:\n");
    out.push_str("  /new              create a new session\n");
    out.push_str("  /clean            delete current session and create a new one\n");
    out.push_str("  /stop             stop current response\n");
    out.push_str("  /cancel           cancel current response\n");
    out.push_str("  /info             show current session status\n");
    out.push_str("  /list             list active sessions on this agent\n");
    out.push_str("  /switch <id>      bind this tunnel to a different session\n");
    out.push_str("  /compress         manually compress current session context\n");
    out.push_str("  /help             show this message\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_is_case_insensitive() {
        assert!(is_known_command("clean"));
        assert!(is_known_command("CLEAN"));
        assert!(is_known_command("new"));
        assert!(is_known_command("stop"));
        assert!(is_known_command("cancel"));
        assert!(is_known_command("info"));
        assert!(is_known_command("compress"));
        assert!(is_known_command("Help"));
        assert!(!is_known_command("etc"));
        assert!(!is_known_command("clear"));
        assert!(!is_known_command("cl3an"));
    }
}
