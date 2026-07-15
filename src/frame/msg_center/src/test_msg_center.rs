use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use buckyos_api::{
    DeliveryReportResult, DeliveryState, IngressContext, MailboxKind, MsgCenterHandler,
    ReadReceiptState, RecipientState, SessionDeliveryOverall, SessionMessageDirection,
    TransportKind,
};
use kRPC::RPCContext;
use name_lib::DID;
use ndn_lib::{MsgContent, MsgContentFormat, MsgObjKind, MsgObject, NamedObject};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::{tempdir, TempDir};

static TEST_TIME_SEQ: AtomicU64 = AtomicU64::new(10_000);

fn next_created_at_ms() -> u64 {
    TEST_TIME_SEQ.fetch_add(1, Ordering::SeqCst)
}

async fn new_center(_tag: &str) -> (MessageCenter, TempDir) {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("msg-center.db");
    let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
    let msg_box_db = MsgBoxDbMgr::open_default_sqlite(&conn).await.unwrap();
    let center = MessageCenter::open_with_db(msg_box_db).await.unwrap();
    (center, tmp)
}

fn make_msg(from: DID, to: Vec<DID>, kind: MsgObjKind) -> MsgObject {
    MsgObject {
        from,
        to,
        kind,
        content: MsgContent {
            format: Some(MsgContentFormat::TextPlain),
            content: "hello".to_string(),
            ..Default::default()
        },
        created_at_ms: next_created_at_ms(),
        ..Default::default()
    }
}

fn ctx() -> RPCContext {
    RPCContext::default()
}

#[tokio::test]
async fn dispatch_single_chat_goes_to_inbox_and_locking_moves_state() {
    let (center, _tmp) = new_center("dispatch_inbox").await;
    let sender = DID::new("bns", "sender-a");
    let recipient = DID::new("bns", "recipient-a");

    center
        .handle_grant_temporary_access(
            vec![sender.clone()],
            "ctx-inbox".to_string(),
            60,
            Some(recipient.clone()),
            ctx(),
        )
        .await
        .unwrap();

    let msg = make_msg(sender.clone(), vec![recipient.clone()], MsgObjKind::Chat);
    let dispatch = center
        .handle_dispatch(
            msg,
            Some(IngressContext {
                context_id: Some("ctx-inbox".to_string()),
                ..Default::default()
            }),
            None,
            ctx(),
        )
        .await
        .unwrap();

    assert!(dispatch.ok);
    assert_eq!(dispatch.delivered_recipients, vec![recipient.clone()]);

    let inbox = center
        .handle_peek_box(
            recipient.clone(),
            MailboxKind::Inbox,
            None,
            None,
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].record.state, RecipientState::Unread);
    // DM records key their session on the peer DID.
    assert_eq!(
        inbox[0].record.session_id.as_deref(),
        Some(format!("dm:{}", sender.to_string()).as_str())
    );

    let next = center
        .handle_get_next(
            recipient.clone(),
            MailboxKind::Inbox,
            None,
            None,
            None,
            ctx(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.record.state, RecipientState::Reading);

    let no_more_unread = center
        .handle_get_next(recipient, MailboxKind::Inbox, None, None, None, ctx())
        .await
        .unwrap();
    assert!(no_more_unread.is_none());
}

#[tokio::test]
async fn dispatch_stranger_goes_to_request_box() {
    let (center, _tmp) = new_center("dispatch_request").await;
    let sender = DID::new("bns", "sender-b");
    let recipient = DID::new("bns", "recipient-b");
    let msg = make_msg(sender, vec![recipient.clone()], MsgObjKind::Chat);

    let dispatch = center
        .handle_dispatch(msg, None, None, ctx())
        .await
        .unwrap();
    assert!(dispatch.ok);
    assert!(dispatch.delivered_recipients.contains(&recipient));

    let inbox = center
        .handle_peek_box(
            recipient.clone(),
            MailboxKind::Inbox,
            None,
            None,
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(inbox.len(), 0);

    let request_box = center
        .handle_peek_box(recipient, MailboxKind::RequestBox, None, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(request_box.len(), 1);
    assert_eq!(request_box[0].record.state, RecipientState::Unread);
}

#[tokio::test]
async fn dispatch_group_message_creates_group_and_agent_views() {
    let (center, _tmp) = new_center("dispatch_group").await;
    let group_id = DID::new("bns", "group-a");
    let author = DID::new("bns", "author-a");
    let agent_1 = DID::new("bns", "agent-a1");
    let agent_2 = DID::new("bns", "agent-a2");

    center
        .handle_set_group_subscribers(
            group_id.clone(),
            vec![agent_1.clone(), agent_2.clone(), agent_2.clone()],
            None,
            ctx(),
        )
        .await
        .unwrap();

    let msg = make_msg(author, vec![group_id.clone()], MsgObjKind::GroupMsg);
    let dispatch = center
        .handle_dispatch(msg, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(dispatch.delivered_group, Some(group_id.clone()));
    assert_eq!(dispatch.delivered_agents.len(), 2);

    let group_box = center
        .handle_peek_box(
            group_id.clone(),
            MailboxKind::GroupInbox,
            None,
            None,
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(group_box.len(), 1);
    // Group records key their session on the group DID.
    assert_eq!(
        group_box[0].record.session_id.as_deref(),
        Some(group_id.to_string().as_str())
    );

    let agent1_box = center
        .handle_peek_box(agent_1, MailboxKind::Inbox, None, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(agent1_box.len(), 1);

    let agent2_box = center
        .handle_peek_box(agent_2, MailboxKind::Inbox, None, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(agent2_box.len(), 1);
}

#[tokio::test]
async fn dispatch_group_message_without_group_target_fails() {
    let (center, _tmp) = new_center("dispatch_group_no_target").await;
    let author = DID::new("bns", "author-empty-group");
    // Legacy group messages used to fall back to `from` when `to` was empty;
    // the frozen model requires from=actor, to=group with no fallback.
    let msg = make_msg(author, Vec::new(), MsgObjKind::GroupMsg);

    let err = center.handle_dispatch(msg, None, None, ctx()).await;
    assert!(err.is_err());
}

#[tokio::test]
async fn post_send_to_endpoint_did_creates_sent_and_delivery_records() {
    let (center, _tmp) = new_center("post_send").await;
    let transport_did = DID::new("bns", "tg-tunnel-box");
    center
        .register_tunnel(
            "tg-main-tunnel".to_string(),
            transport_did.clone(),
            "telegram".to_string(),
        )
        .unwrap();

    let author = DID::new("bns", "author-b");
    // A determined shadow endpoint DID carries its own routing identity.
    let target = DID::new("msgtunnel", "12345.user.tg-main-tunnel");
    let msg = make_msg(author.clone(), vec![target.clone()], MsgObjKind::Chat);

    let post_send = center.handle_post_send(msg, None, ctx()).await.unwrap();
    assert!(post_send.ok);
    assert_eq!(post_send.deliveries.len(), 1);
    assert_eq!(post_send.deliveries[0].transport_did, transport_did);
    assert!(matches!(
        post_send.deliveries[0].transport,
        TransportKind::Tunnel { .. }
    ));

    let sent_box = center
        .handle_peek_box(author, MailboxKind::Sent, None, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(sent_box.len(), 1);
    // SENT is send history, not delivery success.
    assert_eq!(sent_box[0].record.state, RecipientState::Read);

    // The executor consumes the DELIVERY_QUEUE: the envelope carries the full
    // address snapshot (decoded from the shadow DID), never guessed later.
    let taken = center
        .handle_get_next_delivery(transport_did.clone(), Some(true), None, ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(taken.record.state, DeliveryState::Sending);
    assert_eq!(taken.record.envelope.target_did, target);
    let snapshot = taken.record.envelope.address.as_ref().unwrap();
    assert_eq!(snapshot.chat_id.as_deref(), Some("12345"));
    assert_eq!(snapshot.platform.as_deref(), Some("telegram"));

    // Queue drained (single record moved to SENDING).
    let empty = center
        .handle_get_next_delivery(transport_did, Some(true), None, ctx())
        .await
        .unwrap();
    assert!(empty.is_none());
}

#[tokio::test]
async fn post_send_to_shareable_did_uses_message_hub_plan() {
    let (center, _tmp) = new_center("post_send_hub").await;
    let hub_did = DID::new("bns", "msg-hub");
    center.set_message_hub_did(hub_did.clone());

    let author = DID::new("bns", "author-hub");
    let target = DID::new("bns", "bob");
    let msg = make_msg(author.clone(), vec![target.clone()], MsgObjKind::Chat);

    let post_send = center.handle_post_send(msg, None, ctx()).await.unwrap();
    assert!(post_send.ok);
    assert_eq!(post_send.deliveries.len(), 1);
    assert_eq!(post_send.deliveries[0].transport_did, hub_did);
    assert_eq!(post_send.deliveries[0].transport, TransportKind::Native);

    let taken = center
        .handle_get_next_delivery(hub_did, Some(true), None, ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(taken.record.envelope.target_did, target);
    assert!(taken.record.envelope.address.is_none());
}

#[tokio::test]
async fn post_send_to_shareable_did_fails_without_message_hub() {
    let (center, _tmp) = new_center("post_send_no_hub").await;
    let author = DID::new("bns", "author-c");
    let target = DID::new("bns", "bob");
    let msg = make_msg(author.clone(), vec![target], MsgObjKind::Chat);

    let post_send = center.handle_post_send(msg, None, ctx()).await.unwrap();
    // No registered hub executor and no implicit binding selection: post_send
    // fails with a clear reason instead of routing to a default tunnel.
    assert!(!post_send.ok);
    assert!(post_send.deliveries.is_empty());
    assert!(post_send.reason.is_some());

    // Phase-1 failure keeps the database clean: no SENT record was written.
    let sent_box = center
        .handle_peek_box(author, MailboxKind::Sent, None, None, None, ctx())
        .await
        .unwrap();
    assert!(sent_box.is_empty());
}

#[tokio::test]
async fn post_send_to_unknown_tunnel_fails() {
    let (center, _tmp) = new_center("post_send_unknown_tunnel").await;
    let author = DID::new("bns", "author-d");
    // Endpoint DID whose tunnel_instance_id has no registered route.
    let target = DID::new("msgtunnel", "12345.user.ghost-tunnel");
    let msg = make_msg(author.clone(), vec![target], MsgObjKind::Chat);

    let post_send = center.handle_post_send(msg, None, ctx()).await.unwrap();
    assert!(!post_send.ok);
    assert!(post_send.deliveries.is_empty());
}

#[tokio::test]
async fn post_send_rejects_message_without_target() {
    let (center, _tmp) = new_center("post_send_without_target").await;
    let author = DID::new("bns", "author-empty-target");
    let msg = make_msg(author.clone(), Vec::new(), MsgObjKind::Chat);

    let err = center.handle_post_send(msg, None, ctx()).await.unwrap_err();

    assert!(matches!(err, kRPC::RPCErrors::ParseRequestError(_)));
    let sent_box = center
        .handle_peek_box(author, MailboxKind::Sent, None, None, None, ctx())
        .await
        .unwrap();
    assert!(sent_box.is_empty());
}

#[tokio::test]
async fn tunnel_registry_rejects_duplicate_instance_id() {
    let (center, _tmp) = new_center("registry_duplicate").await;
    center
        .register_tunnel(
            "tg-main-tunnel".to_string(),
            DID::new("bns", "tg-a"),
            "telegram".to_string(),
        )
        .unwrap();
    // Same instance id again — even with a different transport DID — must fail
    // instead of silently overwriting (shadow DID stability).
    let err = center.register_tunnel(
        "tg-main-tunnel".to_string(),
        DID::new("bns", "tg-b"),
        "telegram".to_string(),
    );
    assert!(err.is_err());

    // After an explicit clear (settings reload) the id can be reused.
    center.clear_tunnel_registry();
    center
        .register_tunnel(
            "tg-main-tunnel".to_string(),
            DID::new("bns", "tg-b"),
            "telegram".to_string(),
        )
        .unwrap();
}

#[tokio::test]
async fn report_delivery_handles_success_and_failure_paths() {
    let (center, _tmp) = new_center("report_delivery").await;
    let transport_did = DID::new("bns", "tg-tunnel-box");
    center
        .register_tunnel(
            "tg-main-tunnel".to_string(),
            transport_did.clone(),
            "telegram".to_string(),
        )
        .unwrap();
    let sender = DID::new("bns", "sender-c");
    let target = DID::new("msgtunnel", "777.user.tg-main-tunnel");

    let fail_msg = make_msg(sender.clone(), vec![target.clone()], MsgObjKind::Chat);
    let fail_post = center
        .handle_post_send(fail_msg, None, ctx())
        .await
        .unwrap();
    let fail_delivery_id = fail_post.deliveries[0].delivery_id.clone();

    let failed_record = center
        .handle_report_delivery(
            fail_delivery_id,
            DeliveryReportResult {
                ok: false,
                error_message: Some("unrecoverable".to_string()),
                retryable: Some(false),
                ..Default::default()
            },
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(failed_record.state, DeliveryState::Dead);
    assert_eq!(failed_record.attempts, 1);
    assert!(failed_record.last_error.is_some());

    let success_msg = make_msg(sender, vec![target], MsgObjKind::Chat);
    let success_post = center
        .handle_post_send(success_msg, None, ctx())
        .await
        .unwrap();
    let success_delivery_id = success_post.deliveries[0].delivery_id.clone();

    let success_record = center
        .handle_report_delivery(
            success_delivery_id,
            DeliveryReportResult {
                ok: true,
                external_msg_id: Some("ext-1".to_string()),
                ..Default::default()
            },
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(success_record.state, DeliveryState::Sent);
    assert_eq!(success_record.external_msg_id, Some("ext-1".to_string()));
}

#[tokio::test]
async fn retryable_failure_requeues_with_backoff() {
    let (center, _tmp) = new_center("report_retry").await;
    let transport_did = DID::new("bns", "tg-retry");
    center
        .register_tunnel(
            "tg-retry-tunnel".to_string(),
            transport_did.clone(),
            "telegram".to_string(),
        )
        .unwrap();
    let sender = DID::new("bns", "sender-retry");
    let target = DID::new("msgtunnel", "42.user.tg-retry-tunnel");
    let msg = make_msg(sender, vec![target], MsgObjKind::Chat);
    let post = center.handle_post_send(msg, None, ctx()).await.unwrap();
    let delivery_id = post.deliveries[0].delivery_id.clone();

    let retried = center
        .handle_report_delivery(
            delivery_id.clone(),
            DeliveryReportResult {
                ok: false,
                error_message: Some("HTTP 429".to_string()),
                retryable: Some(true),
                retry_after_ms: Some(60_000),
                ..Default::default()
            },
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(retried.state, DeliveryState::Wait);
    assert!(retried.next_retry_at_ms.is_some());

    // The retry is not due yet, so the executor gets nothing.
    let not_due = center
        .handle_get_next_delivery(transport_did, Some(true), None, ctx())
        .await
        .unwrap();
    assert!(not_due.is_none());
}

#[tokio::test]
async fn update_record_state_checks_transition_rules() {
    let (center, _tmp) = new_center("update_state").await;
    let sender = DID::new("bns", "sender-d");
    let recipient = DID::new("bns", "recipient-d");

    center
        .handle_grant_temporary_access(
            vec![sender.clone()],
            "ctx-state".to_string(),
            60,
            Some(recipient.clone()),
            ctx(),
        )
        .await
        .unwrap();

    let msg = make_msg(sender, vec![recipient.clone()], MsgObjKind::Chat);
    center
        .handle_dispatch(
            msg,
            Some(IngressContext {
                context_id: Some("ctx-state".to_string()),
                ..Default::default()
            }),
            None,
            ctx(),
        )
        .await
        .unwrap();

    let inbox = center
        .handle_peek_box(recipient, MailboxKind::Inbox, None, None, None, ctx())
        .await
        .unwrap();
    let record_id = inbox[0].record.record_id.clone();

    let updated = center
        .handle_update_record_state(record_id.clone(), RecipientState::Read, ctx())
        .await
        .unwrap();
    assert_eq!(updated.state, RecipientState::Read);

    // READ can go back to READING but never directly to UNREAD.
    let invalid = center
        .handle_update_record_state(record_id, RecipientState::Unread, ctx())
        .await;
    assert!(invalid.is_err());
}

#[tokio::test]
async fn update_record_session_sets_session_id() {
    let (center, _tmp) = new_center("update_record_session").await;
    let sender = DID::new("bns", "sender-ui-session");
    let recipient = DID::new("bns", "recipient-ui-session");

    center
        .handle_grant_temporary_access(
            vec![sender.clone()],
            "ctx-ui-session".to_string(),
            60,
            Some(recipient.clone()),
            ctx(),
        )
        .await
        .unwrap();

    let msg = make_msg(sender, vec![recipient.clone()], MsgObjKind::Chat);
    center
        .handle_dispatch(
            msg,
            Some(IngressContext {
                context_id: Some("ctx-ui-session".to_string()),
                ..Default::default()
            }),
            None,
            ctx(),
        )
        .await
        .unwrap();

    let inbox = center
        .handle_peek_box(recipient, MailboxKind::Inbox, None, None, None, ctx())
        .await
        .unwrap();
    let record_id = inbox[0].record.record_id.clone();

    let updated = center
        .handle_update_record_session(record_id.clone(), " ui-session-record ".to_string(), ctx())
        .await
        .unwrap();
    assert_eq!(updated.session_id.as_deref(), Some("ui-session-record"));

    let loaded = center
        .handle_get_record(record_id, None, ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.record.session_id.as_deref(),
        Some("ui-session-record")
    );
}

#[tokio::test]
async fn ui_session_state_is_key_value_state() {
    let (center, _tmp) = new_center("ui_session_state").await;

    let first = center
        .handle_update_ui_session_state(
            "ui-session-1".to_string(),
            "typing".to_string(),
            json!({"active": true, "source": "telegram"}),
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(first.session_id, "ui-session-1");
    assert_eq!(first.key, "typing");
    assert_eq!(first.value["active"], true);

    let updated = center
        .handle_update_ui_session_state(
            " ui-session-1 ".to_string(),
            " typing ".to_string(),
            json!({"active": false}),
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(updated.value["active"], false);
    assert!(updated.updated_at_ms >= first.updated_at_ms);

    let loaded = center
        .handle_get_ui_session_state("ui-session-1".to_string(), "typing".to_string(), ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.value, json!({"active": false}));

    center
        .handle_update_ui_session_state(
            "ui-session-1".to_string(),
            "status".to_string(),
            json!("ready"),
            ctx(),
        )
        .await
        .unwrap();
    let listed = center
        .handle_list_ui_session_state("ui-session-1".to_string(), ctx())
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].key, "status");
    assert_eq!(listed[1].key, "typing");

    let empty_session = center
        .handle_update_ui_session_state(" ".to_string(), "typing".to_string(), json!(true), ctx())
        .await;
    assert!(empty_session.is_err());
}

#[tokio::test]
async fn list_box_by_time_supports_pagination() {
    let (center, _tmp) = new_center("list_pagination").await;
    let sender = DID::new("bns", "sender-e");
    let recipient = DID::new("bns", "recipient-e");

    center
        .handle_grant_temporary_access(
            vec![sender.clone()],
            "ctx-page".to_string(),
            60,
            Some(recipient.clone()),
            ctx(),
        )
        .await
        .unwrap();

    let first_msg = make_msg(sender.clone(), vec![recipient.clone()], MsgObjKind::Chat);
    let second_msg = make_msg(sender, vec![recipient.clone()], MsgObjKind::Chat);
    center
        .handle_dispatch(
            first_msg,
            Some(IngressContext {
                context_id: Some("ctx-page".to_string()),
                ..Default::default()
            }),
            None,
            ctx(),
        )
        .await
        .unwrap();
    center
        .handle_dispatch(
            second_msg,
            Some(IngressContext {
                context_id: Some("ctx-page".to_string()),
                ..Default::default()
            }),
            None,
            ctx(),
        )
        .await
        .unwrap();

    let page_1 = center
        .handle_list_box_by_time(
            recipient.clone(),
            MailboxKind::Inbox,
            None,
            Some(1),
            None,
            None,
            Some(true),
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(page_1.items.len(), 1);
    assert!(page_1.next_cursor_sort_key.is_some());
    assert!(page_1.next_cursor_record_id.is_some());

    let page_2 = center
        .handle_list_box_by_time(
            recipient,
            MailboxKind::Inbox,
            None,
            Some(1),
            page_1.next_cursor_sort_key,
            page_1.next_cursor_record_id,
            Some(true),
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(page_2.items.len(), 1);
}

#[tokio::test]
async fn session_projection_merges_directions_and_aggregates_delivery() {
    let (center, _tmp) = new_center("session_projection").await;
    let transport_did = DID::new("bns", "tg-session-tunnel");
    center
        .register_tunnel(
            "tg-main-tunnel".to_string(),
            transport_did.clone(),
            "telegram".to_string(),
        )
        .unwrap();

    let owner = DID::new("bns", "alice");
    let peer = DID::new("msgtunnel", "999.user.tg-main-tunnel");

    // Outbound: post_send writes SENT + one delivery record.
    let out_msg = make_msg(owner.clone(), vec![peer.clone()], MsgObjKind::Chat);
    let post = center.handle_post_send(out_msg, None, ctx()).await.unwrap();
    assert!(post.ok);
    let delivery_id = post.deliveries[0].delivery_id.clone();

    // Inbound reply from the same peer endpoint (allowed via temporary grant).
    center
        .handle_grant_temporary_access(
            vec![peer.clone()],
            "ctx-session".to_string(),
            60,
            Some(owner.clone()),
            ctx(),
        )
        .await
        .unwrap();
    let in_msg = make_msg(peer.clone(), vec![owner.clone()], MsgObjKind::Chat);
    center
        .handle_dispatch(
            in_msg,
            Some(IngressContext {
                context_id: Some("ctx-session".to_string()),
                ..Default::default()
            }),
            None,
            ctx(),
        )
        .await
        .unwrap();

    // Both directions land in the same peer-keyed session.
    let session_id = format!("dm:{}", peer.to_string());
    let sessions = center
        .handle_list_sessions(owner.clone(), None, None, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(sessions.items.len(), 1);
    assert_eq!(sessions.items[0].session_id, session_id);
    assert_eq!(sessions.items[0].unread_count, 1);

    let timeline = center
        .handle_list_session(
            owner.clone(),
            session_id.clone(),
            None,
            None,
            None,
            Some(false),
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(timeline.items.len(), 2);
    let out_item = timeline
        .items
        .iter()
        .find(|item| item.direction == SessionMessageDirection::Out)
        .unwrap();
    let in_item = timeline
        .items
        .iter()
        .find(|item| item.direction == SessionMessageDirection::In)
        .unwrap();
    assert_eq!(in_item.recipient_state, Some(RecipientState::Unread));
    let delivery = out_item.delivery.as_ref().unwrap();
    assert_eq!(delivery.overall, SessionDeliveryOverall::Sending);
    assert_eq!(delivery.per_target.len(), 1);
    assert_eq!(delivery.per_target[0].state, DeliveryState::Wait);

    // Transport accepted → the aggregated view flips to delivered.
    center
        .handle_report_delivery(
            delivery_id,
            DeliveryReportResult {
                ok: true,
                external_msg_id: Some("tg-msg-1".to_string()),
                ..Default::default()
            },
            ctx(),
        )
        .await
        .unwrap();

    let timeline = center
        .handle_list_session(
            owner,
            session_id,
            None,
            None,
            None,
            Some(false),
            None,
            ctx(),
        )
        .await
        .unwrap();
    let out_item = timeline
        .items
        .iter()
        .find(|item| item.direction == SessionMessageDirection::Out)
        .unwrap();
    let delivery = out_item.delivery.as_ref().unwrap();
    assert_eq!(delivery.overall, SessionDeliveryOverall::Delivered);
    assert_eq!(delivery.per_target[0].state, DeliveryState::Sent);
}

#[tokio::test]
async fn read_receipt_can_be_set_and_queried() {
    let (center, _tmp) = new_center("read_receipt").await;
    let group = DID::new("bns", "group-b");
    let author = DID::new("bns", "author-b");
    let reader = DID::new("bns", "reader-b");
    let msg = make_msg(author, vec![group.clone()], MsgObjKind::GroupMsg);
    let msg_id = msg.gen_obj_id().0;

    center
        .handle_dispatch(msg, None, None, ctx())
        .await
        .unwrap();

    let receipt = center
        .handle_set_read_state(
            group.clone(),
            msg_id.clone(),
            reader.clone(),
            ReadReceiptState::Reading,
            Some("processing".to_string()),
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.reader, reader);
    assert_eq!(receipt.status, ReadReceiptState::Reading);

    let receipts = center
        .handle_list_read_receipts(msg_id, Some(group), None, Some(10), Some(0), ctx())
        .await
        .unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].status, ReadReceiptState::Reading);
}

#[tokio::test]
async fn idempotency_key_prevents_duplicate_records() {
    let (center, _tmp) = new_center("idempotency").await;
    let transport_did = DID::new("bns", "tg-tunnel-box");
    center
        .register_tunnel(
            "tg-main-tunnel".to_string(),
            transport_did.clone(),
            "telegram".to_string(),
        )
        .unwrap();
    let sender = DID::new("bns", "sender-f");
    let recipient = DID::new("bns", "recipient-f");
    let endpoint_target = DID::new("msgtunnel", "888.user.tg-main-tunnel");

    center
        .handle_grant_temporary_access(
            vec![sender.clone()],
            "ctx-idem".to_string(),
            60,
            Some(recipient.clone()),
            ctx(),
        )
        .await
        .unwrap();

    let dispatch_msg = make_msg(sender.clone(), vec![recipient.clone()], MsgObjKind::Chat);
    let first_dispatch = center
        .handle_dispatch(
            dispatch_msg.clone(),
            Some(IngressContext {
                context_id: Some("ctx-idem".to_string()),
                ..Default::default()
            }),
            Some("dispatch-idempotent-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    let second_dispatch = center
        .handle_dispatch(
            dispatch_msg,
            Some(IngressContext {
                context_id: Some("ctx-idem".to_string()),
                ..Default::default()
            }),
            Some("dispatch-idempotent-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(first_dispatch.msg_id, second_dispatch.msg_id);

    let inbox = center
        .handle_peek_box(
            recipient.clone(),
            MailboxKind::Inbox,
            None,
            None,
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(inbox.len(), 1);

    let send_msg = make_msg(sender, vec![endpoint_target], MsgObjKind::Chat);
    let first_post = center
        .handle_post_send(
            send_msg.clone(),
            Some("post-idempotent-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    let second_post = center
        .handle_post_send(send_msg, Some("post-idempotent-key".to_string()), ctx())
        .await
        .unwrap();
    assert_eq!(first_post.deliveries.len(), 1);
    assert_eq!(first_post.deliveries, second_post.deliveries);

    // Exactly one delivery record exists for the duplicate submission.
    let taken = center
        .handle_get_next_delivery(transport_did.clone(), Some(true), None, ctx())
        .await
        .unwrap();
    assert!(taken.is_some());
    let empty = center
        .handle_get_next_delivery(transport_did, Some(true), None, ctx())
        .await
        .unwrap();
    assert!(empty.is_none());
}
