//! msg bridge: msg-center records → bus records, and the reply path back.
//!
//! The bridge is the only consumer of a msg-center inbox (a session never
//! reads msg-center itself). It does not convert messages: the MsgObject
//! goes onto the bus as it is, the bridge only filters, splits off slash
//! commands and adds the delivery layer information.
//!
//! Delivery and acknowledgement: `post` to the target session's queue
//! first, acknowledge the msg-center record after it succeeded. A crash in
//! between re-posts the record; the consumer deduplicates by `key` (the
//! message's ObjId). Which session a record goes to (`to` / `to_session`)
//! is decided by the application hosting the bridge, not by this protocol.

use std::collections::BTreeMap;

use name_lib::DID;
use ndn_lib::{MsgContent, MsgObjKind, MsgObject, MsgRelType, ObjId, TopicThread};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::protocol::*;

/// The msg-center record a message was read from.
pub type MsgRecord = buckyos_api::MailboxRecord;

/// What a registered slash command does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// Mapped to a session control command (`/stop` → `stop`).
    Stop,
    /// Registered, but handled by the application hosting the bridge.
    App,
}

/// Everything `route_msg_record` needs besides the record: prepared by the
/// caller, so routing stays a pure function (no network).
#[derive(Debug, Clone, Default)]
pub struct MsgBridgeCtx {
    /// The agent the inbox belongs to (its own group messages are echoes).
    pub agent_did: Option<DID>,
    /// Who may issue slash commands: the session's driver and the agent's
    /// owner. Anyone else's `/stop` is an ordinary message.
    pub command_senders: Vec<DID>,
    /// Registered slash commands (name without `/`).
    pub commands: BTreeMap<String, SlashCommand>,
    /// Display name of the speaker from the contact lookup, used when the
    /// record carries none.
    pub contact_name: Option<String>,
    /// Display name of the conversation (group name).
    pub conversation_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MsgBridgeOutput {
    /// Post the message as it is; `key` = its ObjId.
    Deliver { delivery: MsgDelivery },
    /// Post a control record instead of the message.
    Control { key: String, command: ControlCommand },
    /// A registered command the application handles itself: nothing is
    /// posted, the record is acknowledged.
    AppCommand { name: String, args: String },
    /// Acknowledge the msg-center record without posting.
    Drop { reason: &'static str },
}

/// `/name` or `/name args` when `text` is exactly that and `name` is
/// registered.
fn slash_command<'a>(
    text: &'a str,
    commands: &BTreeMap<String, SlashCommand>,
) -> Option<(&'a str, &'a str)> {
    let rest = text.strip_prefix('/')?;
    let (name, args) = match rest.find(char::is_whitespace) {
        Some(i) => (&rest[..i], rest[i..].trim()),
        None => (rest, ""),
    };
    if name.is_empty() || text.contains('\n') && args.is_empty() {
        return None;
    }
    commands.contains_key(name).then_some((name, args))
}

/// Filter and split one inbox record (mechanical rules, in this order).
/// The session never parses message text: slash text this function does
/// not recognize is an ordinary message for the LLM.
pub fn route_msg_record(record: &MsgRecord, msg: &MsgObject, ctx: &MsgBridgeCtx) -> MsgBridgeOutput {
    // On the bus the speaker is `msg.from` only.
    if record.from != msg.from {
        return MsgBridgeOutput::Drop {
            reason: "record sender differs from msg.from",
        };
    }
    let intent = msg.content.machine.as_ref().and_then(|m| m.intent.as_deref());
    if intent == Some("buckyos.group_invitation") {
        return MsgBridgeOutput::Drop {
            reason: "group invitation notice",
        };
    }
    if msg.kind == MsgObjKind::GroupMsg && ctx.agent_did.as_ref() == Some(&msg.from) {
        return MsgBridgeOutput::Drop {
            reason: "own group message echo",
        };
    }
    let text = msg.content.content.trim();
    if text.is_empty() && msg.content.refs.is_empty() && msg.content.machine.is_none() {
        return MsgBridgeOutput::Drop {
            reason: "empty message",
        };
    }
    if msg
        .relates_to
        .as_ref()
        .is_some_and(|r| r.rel == MsgRelType::Reaction)
    {
        return MsgBridgeOutput::Drop { reason: "reaction" };
    }
    if msg.content.refs.is_empty()
        && msg.content.machine.is_none()
        && ctx.command_senders.contains(&msg.from)
    {
        if let Some((name, args)) = slash_command(text, &ctx.commands) {
            return match ctx.commands.get(name) {
                Some(SlashCommand::Stop) => MsgBridgeOutput::Control {
                    key: format!("ctl:{}", msg_key(msg)),
                    command: ControlCommand::Stop {
                        reason: Some(args.to_string()).filter(|a| !a.is_empty()),
                    },
                },
                _ => MsgBridgeOutput::AppCommand {
                    name: name.to_string(),
                    args: args.to_string(),
                },
            };
        }
    }
    MsgBridgeOutput::Deliver {
        delivery: MsgDelivery {
            from_name: record
                .from_name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .or_else(|| ctx.contact_name.clone()),
            conversation_name: ctx.conversation_name.clone(),
            record_id: Some(record.record_id.clone()),
            tunnel: record
                .ingress
                .as_ref()
                .and_then(|i| i.transport_did.as_ref())
                .map(|d| d.to_string()),
        },
    }
}

impl MsgBridgeOutput {
    /// The bus record to post for this output (`None`: nothing is posted).
    pub fn into_input(self, poster: &str, msg: &MsgObject) -> Result<Option<PostedInput>> {
        Ok(match self {
            MsgBridgeOutput::Deliver { delivery } => {
                Some(PostedInput::msg(poster, msg.clone(), delivery)?)
            }
            MsgBridgeOutput::Control { key, command } => {
                Some(PostedInput::control(poster, key, command))
            }
            MsgBridgeOutput::AppCommand { .. } | MsgBridgeOutput::Drop { .. } => None,
        })
    }
}

/// An outbound message of a session: both directions use MsgObject on the
/// wire, the session being the only place that converts between MsgObject
/// and AiMessage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboundRecord {
    /// Idempotency key; a bridge re-sending the same key never produces a
    /// second message.
    pub key: String,
    pub msg: MsgObject,
}

/// `(sid, turn, run_id, n)`.
pub fn outbound_key(sid: &str, turn: u64, run_id: &str, n: u32) -> String {
    format!("{sid}:{turn}:{run_id}:{n}")
}

/// Envelope of a reply along the session's default reply path
/// (`state.reply`): hand it with the assistant message to
/// `llm_context::ai_message_to_msg_object_with_base_validated_async`, which
/// fills the content (text, `<attachment>` tags and named objects as refs).
/// `None`: the session has no message route (headless sub session: its
/// result goes to the parent session instead).
pub fn outbound_base(reply: &ReplyRoute, agent_did: &str) -> Result<Option<MsgObject>> {
    let ReplyRoute::Message {
        to,
        to_session,
        kind,
        reply_to,
        ..
    } = reply
    else {
        return Ok(None);
    };
    let kind: MsgObjKind = serde_json::from_value(serde_json::Value::String(kind.clone()))
        .unwrap_or(MsgObjKind::Chat);
    Ok(Some(MsgObject {
        from: parse_did(agent_did)?,
        to: vec![parse_did(to)?],
        kind,
        to_session: to_session.clone(),
        thread: TopicThread {
            reply_to: reply_to.as_deref().and_then(|r| ObjId::new(r).ok()),
            ..Default::default()
        },
        created_at_ms: crate::now_ms(),
        content: MsgContent::default(),
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::input_view::{message_view, render_msg_xml};

    const AGENT: &str = "did:bns:jarvis";

    fn did(s: &str) -> DID {
        parse_did(s).unwrap()
    }

    fn record(from: &str, msg: &MsgObject) -> MsgRecord {
        MsgRecord {
            mailbox: buckyos_api::MailboxAddress::new(did(AGENT), None).unwrap(),
            record_id: "r-1".into(),
            owner: did(AGENT),
            box_kind: buckyos_api::MailboxKind::Inbox,
            msg_id: ObjId::new(&msg_key(msg)).unwrap(),
            msg_kind: msg.kind,
            state: buckyos_api::RecipientState::Unread,
            from: did(from),
            from_name: Some("Bob".into()),
            to: did(AGENT),
            session_id: None,
            sort_key: 1,
            tags: Vec::new(),
            ingress: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn ctx() -> MsgBridgeCtx {
        MsgBridgeCtx {
            agent_did: Some(did(AGENT)),
            command_senders: vec![did("did:bns:alice")],
            commands: [
                ("stop".to_string(), SlashCommand::Stop),
                ("model".to_string(), SlashCommand::App),
            ]
            .into_iter()
            .collect(),
            contact_name: None,
            conversation_name: None,
        }
    }

    fn chat(from: &str, text: &str) -> MsgObject {
        let mut m = text_msg(&did(from), &did(AGENT), text);
        m.created_at_ms = 1_790_899_200_000;
        m.nonce = Some(1);
        m
    }

    #[test]
    fn delivers_the_message_as_it_is_with_the_delivery_layer() {
        let msg = chat("did:bns:bob", "hello");
        let out = route_msg_record(&record("did:bns:bob", &msg), &msg, &ctx());
        let MsgBridgeOutput::Deliver { delivery } = &out else {
            panic!("{out:?}")
        };
        assert_eq!(delivery.from_name.as_deref(), Some("Bob"));
        assert_eq!(delivery.record_id.as_deref(), Some("r-1"));
        let posted = out.into_input("app:msg-bridge@alice", &msg).unwrap().unwrap();
        assert_eq!(posted.key, msg_key(&msg), "key = ObjId");
        let SessionInput::Msg(m) = &posted.input else {
            panic!()
        };
        assert_eq!(m.msg, msg, "the MsgObject is not rewritten");
        // A message built with the helper and the same message through the
        // bridge render the same way (the speaker name comes from delivery).
        let by_hand = PostedInput::msg(
            "did:bns:bob",
            msg.clone(),
            MsgDelivery {
                from_name: Some("Bob".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let SessionInput::Msg(h) = &by_hand.input else {
            panic!()
        };
        assert_eq!(
            render_msg_xml(&message_view(&posted.key, 5, m, AGENT)),
            render_msg_xml(&message_view(&by_hand.key, 5, h, AGENT))
        );
    }

    #[test]
    fn filters() {
        let c = ctx();
        let msg = chat("did:bns:bob", "hi");
        // The record's sender must be the speaker.
        assert!(matches!(
            route_msg_record(&record("did:bns:mallory", &msg), &msg, &c),
            MsgBridgeOutput::Drop { .. }
        ));
        let mut empty = chat("did:bns:bob", "  ");
        empty.content.content = "  ".into();
        assert!(matches!(
            route_msg_record(&record("did:bns:bob", &empty), &empty, &c),
            MsgBridgeOutput::Drop { reason: "empty message" }
        ));
        let mut echo = chat(AGENT, "my own words");
        echo.kind = MsgObjKind::GroupMsg;
        assert!(matches!(
            route_msg_record(&record(AGENT, &echo), &echo, &c),
            MsgBridgeOutput::Drop { reason: "own group message echo" }
        ));
        let mut reaction = chat("did:bns:bob", "👍");
        reaction.relates_to = Some(ndn_lib::MsgRelation::reaction(
            ObjId::new(&format!("cymsg:{}", "11".repeat(32))).unwrap(),
            "👍",
        ));
        assert!(matches!(
            route_msg_record(&record("did:bns:bob", &reaction), &reaction, &c),
            MsgBridgeOutput::Drop { reason: "reaction" }
        ));
    }

    #[test]
    fn slash_commands_need_registration_and_an_authorized_sender() {
        let c = ctx();
        let stop = chat("did:bns:alice", "/stop too slow");
        match route_msg_record(&record("did:bns:alice", &stop), &stop, &c) {
            MsgBridgeOutput::Control { key, command } => {
                assert_eq!(key, format!("ctl:{}", msg_key(&stop)));
                assert_eq!(
                    command,
                    ControlCommand::Stop {
                        reason: Some("too slow".into())
                    }
                );
            }
            o => panic!("{o:?}"),
        }
        // Someone else's `/stop` is an ordinary message for the LLM.
        let other = chat("did:bns:bob", "/stop");
        assert!(matches!(
            route_msg_record(&record("did:bns:bob", &other), &other, &c),
            MsgBridgeOutput::Deliver { .. }
        ));
        // Unregistered slash text is ordinary text.
        let path = chat("did:bns:alice", "/etc/nginx is broken");
        assert!(matches!(
            route_msg_record(&record("did:bns:alice", &path), &path, &c),
            MsgBridgeOutput::Deliver { .. }
        ));
        // Registered but not a session control: the application's business.
        let app = chat("did:bns:alice", "/model fast");
        assert_eq!(
            route_msg_record(&record("did:bns:alice", &app), &app, &c),
            MsgBridgeOutput::AppCommand {
                name: "model".into(),
                args: "fast".into()
            }
        );
    }

    #[test]
    fn reply_envelope_follows_the_reply_path() {
        let mut group = chat("did:bns:bob", "@jarvis ping");
        group.kind = MsgObjKind::GroupMsg;
        group.to = vec![did("did:bns:dev-team")];
        let key = msg_key(&group);
        let route = ReplyRoute::of_msg(
            &key,
            &SessionMsg {
                msg: group,
                delivery: MsgDelivery::default(),
            },
        );
        let base = outbound_base(&route, AGENT).unwrap().unwrap();
        assert_eq!(base.from, did(AGENT));
        assert_eq!(base.to, vec![did("did:bns:dev-team")]);
        assert_eq!(base.kind, MsgObjKind::GroupMsg);
        assert_eq!(base.thread.reply_to.unwrap().to_string(), key);
        let parent = ReplyRoute::ParentSession {
            session_id: "ui-1".into(),
        };
        assert!(outbound_base(&parent, AGENT).unwrap().is_none());
        assert_eq!(outbound_key("s", 3, "r", 0), "s:3:r:0");
    }
}
