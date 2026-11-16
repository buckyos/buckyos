//! Where the browser meets the agent's sessions. Every session works in the
//! agent's one group; the conversation the user has with the agent about
//! the browser is the browser conversation (`browser.bind_default`): what
//! the user does to the browser is told to its UI session as a `_browser`
//! observation, and its questions are what the browser shows the agent
//! waiting for.

use libopendan::protocol::*;
use libopendan::state::AgentStateClient;
use libopendan::SessionDir;

use super::groups::DEFAULT_GROUP;

/// The manual of the browser, which the browser conversation has.
pub const CMD_SKILL: &str = "browser";

/// The UI session of the owner's conversation `to_session`: the latest
/// unfinished one (a conversation gets one with its first message).
pub async fn ui_session_of(agent: &dyn AgentStateClient, owner_did: Option<&str>, to_session: &str) -> Option<RegistryEntry> {
    let query = RegistryQuery { kind: Some(SessionKind::Ui), ..Default::default() };
    let mut latest: Option<(u64, RegistryEntry)> = None;
    for entry in agent.sessions().query(&query).await.unwrap_or_default() {
        if entry.status.run_state == RunState::Finished {
            continue;
        }
        let Ok(cfg) = SessionDir::open(&entry.location).and_then(|sd| sd.config()) else { continue };
        let Some(outbound) = cfg.channels.outbound.as_ref() else { continue };
        if owner_did.is_some_and(|owner| outbound.to != owner) || outbound.to_session.as_deref() != Some(to_session) {
            continue;
        }
        if latest.as_ref().is_none_or(|(at, _)| *at <= cfg.session.created_at_ms) {
            latest = Some((cfg.session.created_at_ms, entry));
        }
    }
    latest.map(|(_, entry)| entry)
}

/// Whether the agent waits in the browser conversation for an answer of
/// the user: a question it asked there or in a Work of it (`ask_user`: an
/// `input` notification) is still open. A session's own `needs_input` does
/// not say this: it is set for every conversation that waits for its next
/// message, and not for a session that waits on a question it asked.
pub async fn waiting_answer(agent: &dyn AgentStateClient, default_session: Option<&str>) -> bool {
    let (Some(notify), Some(default_session)) = (agent.notifies(), default_session) else { return false };
    let request = buckyos_api::NotifyListReq {
        status: Some(vec![buckyos_api::NotifyStatus::Open]),
        kind: Some(buckyos_api::NotifyKind::Input),
        agent_did: Some(agent.agent_did().to_string()),
        ..Default::default()
    };
    let asked = match notify.list(request).await {
        Ok(asked) => asked,
        Err(e) => {
            log::debug!("browser: open questions not listed: {e}");
            return false;
        }
    };
    for mut question in asked {
        if question.source.conversation.is_none() && question.source.session_id.is_some() {
            libopendan::notify::complete_notify_source(agent, &mut question.source).await;
        }
        if question.source.conversation.as_ref().and_then(|c| c.to_session.as_deref()) == Some(default_session) {
            return true;
        }
    }
    false
}

fn cut(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// What the user did to the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handed {
    TakeOver,
    HandBack,
    HandToAgent,
}

impl Handed {
    /// `event` of the record.
    pub fn event(&self) -> &'static str {
        match self {
            Handed::TakeOver => "take_over",
            Handed::HandBack => "hand_back",
            Handed::HandToAgent => "hand_to_agent",
        }
    }
}

/// The record of `what` for the browser conversation; `page` is the
/// browser's current tab (`(url, title)`).
pub fn record(what: Handed, page: Option<(&str, &str)>) -> AgentEvent {
    let page = match page {
        Some((url, title)) if !url.is_empty() => format!(" The current page is \"{}\" ({}).", cut(title, 120), cut(url, 300)),
        _ => " No page is open in it.".to_string(),
    };
    let summary = match what {
        Handed::TakeOver => "[browser] The user took over the browser. Do not operate it until the user hands it back: `agent-browser` commands that operate it fail with `user_in_control` meanwhile.".to_string(),
        Handed::HandBack => format!(
            "[browser] The user handed the browser back.{page} The page may have changed while the user had it: read it again (`agent-browser snapshot`) before you act on it."
        ),
        Handed::HandToAgent => format!(
            "[browser] The user asks you to go on from the current page in the browser.{page} Read the page first (`agent-browser snapshot`)."
        ),
    };
    AgentEvent {
        subscription_id: Some(BROWSER_SUBSCRIPTION.to_string()),
        source: EventSource::new("system", format!("{BROWSER_SOURCE_ID}:{DEFAULT_GROUP}")),
        event: what.event().to_string(),
        seq: None,
        summary,
        data_ref: None,
        terminal: false,
        call_id: None,
    }
}

/// Post `event` to the UI session of the browser conversation. Returns the
/// session it went to (`None`: there is no browser conversation, or it has
/// no session yet, so there is nobody to tell).
pub async fn post_record(agent: &dyn AgentStateClient, owner_did: Option<&str>, default_session: Option<&str>, who: &str, event: AgentEvent) -> Option<String> {
    let entry = ui_session_of(agent, owner_did, default_session?).await?;
    let key = format!("browser:{}:{}", event.event, libopendan::now_ms());
    match agent.sessions().post_input(&entry.session_id, &PostedInput::event(who, key, event)).await {
        Ok(_) => Some(entry.session_id),
        Err(e) => {
            log::warn!("browser: session {} was not told about the browser: {e}", entry.session_id);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_name_the_browser_and_the_page() {
        let back = record(Handed::HandBack, Some(("https://example.com/a", "Example")));
        assert_eq!(back.subscription_id.as_deref(), Some("_browser"));
        assert_eq!((back.source.kind.as_str(), back.source.id.as_str()), ("system", "browser:default"));
        assert_eq!(back.event, "hand_back");
        assert!(back.summary.starts_with("[browser] ") && back.summary.contains("\"Example\" (https://example.com/a)"), "{}", back.summary);
        assert!(back.summary.contains("read it again"));
        let over = record(Handed::TakeOver, None);
        assert!(over.summary.contains("user_in_control") && over.summary.starts_with("[browser] "));
        let go = record(Handed::HandToAgent, Some(("", "")));
        assert!(go.summary.contains("No page is open"));
        assert!(record(Handed::HandBack, Some((&"x".repeat(5000), "t"))).summary.len() < 1024);
    }
}
