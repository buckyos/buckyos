//! What starts a turn in a group chat (chat route TODO §3, D7).
//!
//! The owner's settings hold a list of conditions (`group_chat.triggers`)
//! that the host checks before any inference; any one lets the message
//! through. The check is an interface like a Goal's check script
//! ([`crate::goal_check`]): the message, the latest background and the
//! budget in, `{run, reason}` out. [`MessageConditions`] is the built-in one;
//! another kind of condition, or a script, is another [`GroupTrigger`].
//!
//! A built-in condition says who and how:
//!
//! ```json
//! {"type": "message", "who": "owner", "how": "mention"}
//! ```
//!
//! `who`: `owner`, `everyone`, or a list of member DIDs; `how`: `mention`
//! (the message mentions the agent) or `reply` (it answers a message of the
//! agent). `type` may be left out. A condition this host does not know is
//! not met. Without a list the owner's mention is the only condition.

use async_trait::async_trait;
use name_lib::DID;
use ndn_lib::MsgObject;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::goal_check::CheckVerdict;

/// What a check decides: `run`, and why (the same result a Goal's check
/// script gives).
pub type TriggerVerdict = CheckVerdict;

/// An earlier message of the group.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecentMessage {
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub text: String,
    pub at_ms: u64,
}

/// What a trigger sees: plain data, so a script can take it as it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TriggerInput {
    pub now_ms: u64,
    pub owner: String,
    pub sender: String,
    pub text: String,
    pub sent_at_ms: u64,
    pub mentions_agent: bool,
    /// Filled for a trigger that [`TriggerNeeds::reply`]s.
    pub replies_to_agent: bool,
    /// The latest messages, oldest first ([`TriggerNeeds::recent`]).
    pub recent: Vec<RecentMessage>,
    /// The agent's background budget ([`TriggerNeeds::budget`]).
    pub budget: Value,
}

impl TriggerInput {
    pub fn new(msg: &MsgObject, sender: &DID, owner: &DID, agent: Option<&DID>) -> Self {
        Self {
            now_ms: crate::now_ms(),
            owner: owner.to_string(),
            sender: sender.to_string(),
            text: msg.content.content.clone(),
            sent_at_ms: msg.created_at_ms,
            mentions_agent: agent.is_some_and(|agent| {
                msg.mentions.as_ref().is_some_and(|m| m.dids.contains(agent))
            }),
            replies_to_agent: false,
            recent: Vec::new(),
            budget: Value::Null,
        }
    }
}

/// What the host has to look up for a trigger, for every message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TriggerNeeds {
    pub reply: bool,
    pub recent: bool,
    pub budget: bool,
}

#[async_trait]
pub trait GroupTrigger: Send + Sync {
    fn needs(&self) -> TriggerNeeds {
        TriggerNeeds::default()
    }

    /// Runs before any inference and costs none.
    async fn check(&self, input: &TriggerInput) -> TriggerVerdict;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NamedWho {
    Owner,
    Everyone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum Who {
    Named(NamedWho),
    Members(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum How {
    Mention,
    Reply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageCondition {
    #[serde(rename = "type", default = "message_type")]
    kind: String,
    who: Who,
    how: How,
}

const CONDITION_MESSAGE: &str = "message";

fn message_type() -> String {
    CONDITION_MESSAGE.to_string()
}

impl MessageCondition {
    fn parse(value: &Value) -> Result<Self, String> {
        let kind = value.get("type").and_then(Value::as_str).unwrap_or(CONDITION_MESSAGE);
        if kind != CONDITION_MESSAGE {
            return Err(format!("unknown condition type `{kind}`"));
        }
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())
    }

    fn met(&self, i: &TriggerInput) -> bool {
        let who = match &self.who {
            Who::Named(NamedWho::Owner) => i.sender == i.owner,
            Who::Named(NamedWho::Everyone) => true,
            Who::Members(list) => list.contains(&i.sender),
        };
        who && match self.how {
            How::Mention => i.mentions_agent,
            How::Reply => i.replies_to_agent,
        }
    }

    fn describe(&self) -> String {
        let who = match &self.who {
            Who::Named(NamedWho::Owner) => "owner",
            Who::Named(NamedWho::Everyone) => "everyone",
            Who::Members(_) => "members",
        };
        let how = match self.how {
            How::Mention => "mention",
            How::Reply => "reply",
        };
        format!("{who}/{how}")
    }
}

/// The built-in conditions, in the settings' order.
#[derive(Debug, Clone, Default)]
pub struct MessageConditions {
    conditions: Vec<(usize, MessageCondition)>,
    /// Entries that are not met whatever happens: where, and why.
    pub unknown: Vec<(usize, String)>,
}

impl MessageConditions {
    /// `None`: the default, the owner's mention.
    pub fn new(triggers: Option<&[Value]>) -> Self {
        let default = [json!({"type": CONDITION_MESSAGE, "who": "owner", "how": "mention"})];
        let mut out = Self::default();
        for (n, value) in triggers.unwrap_or(&default).iter().enumerate() {
            match MessageCondition::parse(value) {
                Ok(c) => out.conditions.push((n + 1, c)),
                Err(e) => out.unknown.push((n + 1, e)),
            }
        }
        out
    }
}

#[async_trait]
impl GroupTrigger for MessageConditions {
    fn needs(&self) -> TriggerNeeds {
        TriggerNeeds {
            reply: self.conditions.iter().any(|(_, c)| c.how == How::Reply),
            ..Default::default()
        }
    }

    async fn check(&self, input: &TriggerInput) -> TriggerVerdict {
        let met = self.conditions.iter().find(|(_, c)| c.met(input));
        TriggerVerdict {
            run: met.is_some(),
            reason: met.map(|(n, c)| format!("condition {n}: {}", c.describe())),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "did:bns:bob";

    fn input(sender: &str, mention: bool, reply: bool) -> TriggerInput {
        TriggerInput {
            now_ms: 1,
            owner: OWNER.into(),
            sender: sender.into(),
            text: "hi".into(),
            sent_at_ms: 1,
            mentions_agent: mention,
            replies_to_agent: reply,
            recent: vec![],
            budget: Value::Null,
        }
    }

    async fn run(c: &MessageConditions, sender: &str, mention: bool, reply: bool) -> Option<String> {
        let v = c.check(&input(sender, mention, reply)).await;
        assert_eq!(v.run, v.reason.is_some());
        v.reason
    }

    #[tokio::test]
    async fn the_default_is_the_owners_mention() {
        let c = MessageConditions::new(None);
        assert_eq!(c.needs(), TriggerNeeds::default());
        assert_eq!(run(&c, OWNER, true, false).await.as_deref(), Some("condition 1: owner/mention"));
        assert_eq!(run(&c, OWNER, false, true).await, None);
        assert_eq!(run(&c, "did:bns:carol", true, true).await, None);
        // An empty list is a list: nothing starts a turn.
        assert_eq!(run(&MessageConditions::new(Some(&[])), OWNER, true, true).await, None);
    }

    #[tokio::test]
    async fn any_condition_lets_a_message_through() {
        let list = [
            json!({"who": "owner", "how": "reply"}),
            json!({"type": "quiet_hours", "from": "22:00"}),
            json!({"who": ["did:bns:carol"], "how": "mention"}),
            json!({"who": "everyone", "how": "poke"}),
            json!({"type": "message", "who": "everyone", "how": "mention", "extra": 1}),
        ];
        let c = MessageConditions::new(Some(&list));
        assert!(c.needs().reply && !c.needs().recent && !c.needs().budget);
        assert_eq!(c.unknown.iter().map(|(n, _)| *n).collect::<Vec<_>>(), [2, 4, 5]);
        assert!(c.unknown[0].1.contains("quiet_hours"));
        assert_eq!(run(&c, OWNER, false, true).await.as_deref(), Some("condition 1: owner/reply"));
        assert_eq!(run(&c, "did:bns:carol", true, false).await.as_deref(), Some("condition 3: members/mention"));
        assert_eq!(run(&c, "did:bns:dave", true, true).await, None);
        let all = MessageConditions::new(Some(&[json!({"who": "everyone", "how": "mention"})]));
        assert_eq!(run(&all, "did:bns:dave", true, false).await.as_deref(), Some("condition 1: everyone/mention"));
        assert_eq!(run(&all, "did:bns:dave", false, true).await, None);
    }
}
