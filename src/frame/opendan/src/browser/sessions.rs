//! Where groups meet the agent's sessions: a group is the tabs of one
//! MessageHub conversation, so a session belongs to the group of the
//! conversation its UI session answers in, and what happens to a group is
//! told to that UI session as a `_browser` observation.

use std::collections::BTreeMap;

use libopendan::protocol::*;
use libopendan::state::AgentStateClient;
use libopendan::SessionDir;

use super::groups::DEFAULT_GROUP;

/// The group of a conversation addressed by `to_session`: the default one
/// for a conversation without a session of its own and for the one the
/// default group is bound to (`default_session`), else the conversation's.
pub fn group_of_conversation(to_session: Option<&str>, default_session: Option<&str>) -> String {
    match to_session {
        Some(s) if !s.is_empty() && !s.starts_with("dm:") && Some(s) != default_session => s.to_string(),
        _ => DEFAULT_GROUP.to_string(),
    }
}

/// The group a session works in: that of the conversation of the UI session
/// it belongs to (a Work's nearest UI ancestor, a Goal's creating one). A
/// session with no such conversation works in the default group.
pub async fn group_of_session(agent: &dyn AgentStateClient, session_id: &str, default_session: Option<&str>) -> String {
    let mut source = buckyos_api::NotifySource {
        session_id: Some(session_id.to_string()),
        ..Default::default()
    };
    libopendan::notify::complete_notify_source(agent, &mut source).await;
    group_of_conversation(source.conversation.as_ref().and_then(|c| c.to_session.as_deref()), default_session)
}

/// The unfinished UI sessions of the owner's conversations, by group: the
/// latest one of each; for the default group one of the conversation it is
/// bound to before any other.
async fn ui_sessions(agent: &dyn AgentStateClient, owner_did: Option<&str>, default_session: Option<&str>) -> BTreeMap<String, RegistryEntry> {
    let query = RegistryQuery { kind: Some(SessionKind::Ui), ..Default::default() };
    let mut latest: BTreeMap<String, ((bool, u64), RegistryEntry)> = BTreeMap::new();
    for entry in agent.sessions().query(&query).await.unwrap_or_default() {
        if entry.status.run_state == RunState::Finished {
            continue;
        }
        let Ok(cfg) = SessionDir::open(&entry.location).and_then(|sd| sd.config()) else { continue };
        let Some(outbound) = cfg.channels.outbound.as_ref() else { continue };
        if owner_did.is_some_and(|owner| outbound.to != owner) {
            continue;
        }
        let group = group_of_conversation(outbound.to_session.as_deref(), default_session);
        let rank = (default_session.is_some() && outbound.to_session.as_deref() == default_session, cfg.session.created_at_ms);
        if latest.get(&group).is_none_or(|(best, _)| *best <= rank) {
            latest.insert(group, (rank, entry));
        }
    }
    latest.into_iter().map(|(group, (_, entry))| (group, entry)).collect()
}

/// The groups in whose conversation the agent waits for an answer of the
/// user: a question it asked (`ask_user`: an `input` notification) is still
/// open. A session's own `needs_input` does not say this: it is set for
/// every conversation that waits for its next message, and not for a
/// session that waits on a question it asked.
pub async fn groups_waiting_answer(agent: &dyn AgentStateClient, default_session: Option<&str>) -> Vec<String> {
    let Some(notify) = agent.notifies() else { return Vec::new() };
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
            return Vec::new();
        }
    };
    let mut waiting = Vec::new();
    for question in asked {
        let group = match (&question.source.conversation, &question.source.session_id) {
            (Some(conversation), _) => group_of_conversation(conversation.to_session.as_deref(), default_session),
            (None, Some(session_id)) => group_of_session(agent, session_id, default_session).await,
            (None, None) => DEFAULT_GROUP.to_string(),
        };
        if !waiting.contains(&group) {
            waiting.push(group);
        }
    }
    waiting
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

/// What the user did to a group.
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

/// The record of `what` for the group's conversation; `page` is the group's
/// current tab (`(url, title)`).
pub fn record(group_id: &str, what: Handed, page: Option<(&str, &str)>) -> AgentEvent {
    let page = match page {
        Some((url, title)) if !url.is_empty() => format!(" The current page is \"{}\" ({}).", cut(title, 120), cut(url, 300)),
        _ => " No page is open in it.".to_string(),
    };
    let summary = match what {
        Handed::TakeOver => format!(
            "[browser {group_id}] The user took over the browser. Do not operate it until the user hands it back: `agent-browser` commands that operate it fail with `user_in_control` meanwhile."
        ),
        Handed::HandBack => format!(
            "[browser {group_id}] The user handed the browser back.{page} The page may have changed while the user had it: read it again (`agent-browser snapshot`) before you act on it."
        ),
        Handed::HandToAgent => format!(
            "[browser {group_id}] The user asks you to go on from the current page in the browser.{page} Read the page first (`agent-browser snapshot`)."
        ),
    };
    AgentEvent {
        subscription_id: Some(BROWSER_SUBSCRIPTION.to_string()),
        source: EventSource::new("system", format!("{BROWSER_SOURCE_ID}:{group_id}")),
        event: what.event().to_string(),
        seq: None,
        summary,
        data_ref: None,
        terminal: false,
        call_id: None,
    }
}

/// Post `event` to the UI session of the group's conversation. Returns the
/// session it went to (`None`: the conversation has no session yet, so
/// there is nobody to tell).
pub async fn post_record(
    agent: &dyn AgentStateClient,
    owner_did: Option<&str>,
    default_session: Option<&str>,
    who: &str,
    group_id: &str,
    event: AgentEvent,
) -> Option<String> {
    let entry = ui_sessions(agent, owner_did, default_session).await.remove(group_id)?;
    let key = format!("browser:{group_id}:{}:{}", event.event, libopendan::now_ms());
    match agent.sessions().post_input(&entry.session_id, &PostedInput::event(who, key, event)).await {
        Ok(_) => Some(entry.session_id),
        Err(e) => {
            log::warn!("browser: session {} was not told about group {group_id}: {e}", entry.session_id);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_without_a_named_session_is_the_default_group() {
        assert_eq!(group_of_conversation(None, None), "default");
        assert_eq!(group_of_conversation(Some(""), None), "default");
        assert_eq!(group_of_conversation(Some("dm:did:bns:bob"), None), "default");
        assert_eq!(group_of_conversation(Some("3f2a-thing"), None), "3f2a-thing");
        // The conversation the default group is bound to.
        assert_eq!(group_of_conversation(Some("3f2a-thing"), Some("3f2a-thing")), "default");
        assert_eq!(group_of_conversation(Some("77aa-other"), Some("3f2a-thing")), "77aa-other");
        assert_eq!(group_of_conversation(None, Some("3f2a-thing")), "default");
    }

    #[test]
    fn records_name_the_group_and_the_page() {
        let back = record("default", Handed::HandBack, Some(("https://example.com/a", "Example")));
        assert_eq!(back.subscription_id.as_deref(), Some("_browser"));
        assert_eq!((back.source.kind.as_str(), back.source.id.as_str()), ("system", "browser:default"));
        assert_eq!(back.event, "hand_back");
        assert!(back.summary.contains("\"Example\" (https://example.com/a)"), "{}", back.summary);
        assert!(back.summary.contains("read it again"));
        let over = record("thing", Handed::TakeOver, None);
        assert!(over.summary.contains("user_in_control") && over.summary.starts_with("[browser thing]"));
        let go = record("thing", Handed::HandToAgent, Some(("", "")));
        assert!(go.summary.contains("No page is open"));
        assert!(record("g", Handed::HandBack, Some((&"x".repeat(5000), "t"))).summary.len() < 1024);
    }
}
