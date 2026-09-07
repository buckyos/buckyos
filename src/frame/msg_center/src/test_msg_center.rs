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
    let db_path = db_path.to_string_lossy().replace('\\', "/");
    let conn = format!("sqlite:///{}?mode=rwc", db_path);
    let center = open_center_at(&conn).await;
    (center, tmp)
}

async fn open_center_at(conn: &str) -> MessageCenter {
    let msg_box_db = MsgBoxDbMgr::open_default_sqlite(&conn).await.unwrap();
    MessageCenter::open_with_db(msg_box_db).await.unwrap()
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
async fn cyfs_dispatch_confirms_only_the_current_receiver_and_survives_restart() {
    let (center, tmp) = new_center("cyfs_dispatch").await;
    let sender = DID::new("bns", "sender");
    let alice = DID::new("bns", "alice");
    let bob = DID::new("bns", "bob");
    center.register_local_recipients([alice.clone(), bob.clone()]);
    let msg = make_msg(
        sender.clone(),
        vec![alice.clone(), bob.clone()],
        MsgObjKind::Chat,
    );
    let id = msg.gen_obj_id().0;
    let target = "cyfs://alice.example/inbox";
    let first = center
        .dispatch_to_receiver(msg.clone(), alice.clone(), target.into())
        .await
        .unwrap();
    assert_eq!(first.msg_id, id);
    assert_eq!(first.delivered_recipients, vec![alice.clone()]);
    assert!(!first.delivered_recipients.contains(&bob));
    let duplicate = center
        .dispatch_to_receiver(msg.clone(), alice.clone(), target.into())
        .await
        .unwrap();
    assert_eq!(duplicate.delivered_recipients, first.delivered_recipients);
    assert!(center
        .query_cyfs_dispatch(&bob.to_string(), target, &id)
        .await
        .unwrap()
        .is_none());
    assert!(center
        .query_cyfs_dispatch(&sender.to_string(), "cyfs://alice.example/other", &id)
        .await
        .unwrap()
        .is_none());
    drop(center);
    let path = tmp
        .path()
        .join("msg-center.db")
        .to_string_lossy()
        .replace('\\', "/");
    let center = open_center_at(&format!("sqlite:///{path}?mode=rwc")).await;
    center.register_local_recipients([alice.clone(), bob.clone()]);
    assert_eq!(
        center
            .query_cyfs_dispatch(&sender.to_string(), target, &id)
            .await
            .unwrap()
            .unwrap()
            .msg_id,
        id
    );
    let second = center
        .dispatch_to_receiver(msg, bob.clone(), "cyfs://alice.example/second".into())
        .await
        .unwrap();
    assert_eq!(second.delivered_recipients, vec![bob]);
}

#[tokio::test]
async fn cyfs_dispatch_cached_reports_retry_and_late_failure_cannot_downgrade_accepted() {
    let (center, _tmp) = new_center("cyfs_sender").await;
    let hub = DID::new("bns", "hub");
    center.set_message_hub_did(hub);
    let recipient = DID::new("bns", "remote");
    let msg = make_msg(DID::new("bns", "sender"), vec![recipient], MsgObjKind::Chat);
    let (id, original) = msg.gen_obj_id();
    let post = center.handle_post_send(msg, None, ctx()).await.unwrap();
    let delivery_id = post.deliveries[0].delivery_id.clone();
    let result = ndn_lib::CyfsDispatchResult::new(
        Some(id.clone()),
        "cyfs://remote.example/inbox".into(),
        ndn_lib::CyfsDispatchStatus::Cached,
    );
    let report = crate::cyfs_dispatch::delivery_report(result);
    assert!(!report.ok);
    let pending = center
        .handle_report_delivery(delivery_id.clone(), report.clone(), ctx())
        .await
        .unwrap();
    assert_eq!(pending.state, DeliveryState::Wait);
    assert!(pending.next_retry_at_ms.is_some());
    assert_eq!(pending.envelope.msg_id, id);
    let accepted = center
        .handle_report_delivery(
            delivery_id.clone(),
            DeliveryReportResult {
                ok: true,
                ..Default::default()
            },
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.state, DeliveryState::Sent);
    let late = center
        .handle_report_delivery(delivery_id, report, ctx())
        .await
        .unwrap();
    assert_eq!(late.state, DeliveryState::Sent);
    assert_eq!(late.delivered_at_ms, accepted.delivered_at_ms);
    assert_eq!(
        ndn_lib::validate_cyfs_dispatch_object(original.as_bytes(), Some(&id.to_string())).unwrap(),
        id
    );
}

#[tokio::test]
async fn cyfs_dispatch_http_rejects_forged_identity_before_receiving() {
    use http_body_util::{BodyExt, Full};
    let (center, _tmp) = new_center("cyfs_auth").await;
    let receiver = DID::new("bns", "alice");
    center.register_local_recipients([receiver]);
    *center.cyfs_dispatch.write().unwrap() = crate::cyfs_dispatch::CyfsDispatchSettings::parse(&json!({
        "cyfs_dispatch": {"target_zone":"alice.example", "accepted_paths":{"/inbox":"did:bns:alice"}}
    })).unwrap();
    let req = http::Request::builder()
        .method("PUT")
        .uri("/inbox")
        .header("host", "alice.example")
        .header("cyfs-original-user", "did:bns:sender")
        .header("content-type", ndn_lib::CYFS_CONTENT_TYPE_NAMED_OBJECT_JSON)
        .body(
            Full::new(bytes::Bytes::from_static(b"{}"))
                .map_err(|e| match e {})
                .boxed(),
        )
        .unwrap();
    let response = crate::cyfs_dispatch::serve(&center, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()[ndn_lib::CYFS_HEADER_DISPATCH_STATUS],
        "rejected"
    );
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
async fn dispatch_replay_preserves_existing_recipient_state() {
    let (center, _tmp) = new_center("replay_state").await;
    let sender = DID::new("bns", "sender-replay-state");
    let recipient = DID::new("bns", "recipient-replay-state");
    let context_id = "ctx-replay-state".to_string();

    center
        .handle_grant_temporary_access(
            vec![sender.clone()],
            context_id.clone(),
            60,
            Some(recipient.clone()),
            ctx(),
        )
        .await
        .unwrap();

    let msg = make_msg(sender, vec![recipient.clone()], MsgObjKind::Chat);
    let ingress = IngressContext {
        context_id: Some(context_id),
        ..Default::default()
    };
    center
        .handle_dispatch(
            msg.clone(),
            Some(ingress.clone()),
            Some("replay-state-a".to_string()),
            ctx(),
        )
        .await
        .unwrap();
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
    let record_id = inbox[0].record.record_id.clone();

    center
        .handle_update_record_state(record_id.clone(), RecipientState::Read, ctx())
        .await
        .unwrap();
    center
        .handle_dispatch(
            msg.clone(),
            Some(ingress.clone()),
            Some("replay-state-b".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    let record = center
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
    assert_eq!(record[0].record.state, RecipientState::Read);

    center
        .handle_update_record_state(record_id.clone(), RecipientState::Archived, ctx())
        .await
        .unwrap();
    center
        .handle_dispatch(msg.clone(), Some(ingress.clone()), None, ctx())
        .await
        .unwrap();
    let record = center
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
    assert_eq!(record[0].record.state, RecipientState::Archived);

    center
        .handle_update_record_state(record_id, RecipientState::Deleted, ctx())
        .await
        .unwrap();
    center
        .handle_dispatch(msg, Some(ingress), None, ctx())
        .await
        .unwrap();
    let record = center
        .handle_peek_box(recipient, MailboxKind::Inbox, None, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(record[0].record.state, RecipientState::Deleted);
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
        .handle_list_sessions(owner.clone(), None, None, None, None, None, None, ctx())
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

#[tokio::test]
async fn idempotency_key_survives_message_center_restart() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("msg-center.db");
    let db_path = db_path.to_string_lossy().replace('\\', "/");
    let conn = format!("sqlite:///{}?mode=rwc", db_path);
    let first_center = open_center_at(&conn).await;
    let sender = DID::new("bns", "sender-g");
    let recipient = DID::new("bns", "recipient-g");

    first_center
        .handle_grant_temporary_access(
            vec![sender.clone()],
            "ctx-persist-idem".to_string(),
            60,
            Some(recipient.clone()),
            ctx(),
        )
        .await
        .unwrap();

    let msg = make_msg(sender, vec![recipient.clone()], MsgObjKind::Chat);
    let first_dispatch = first_center
        .handle_dispatch(
            msg.clone(),
            Some(IngressContext {
                context_id: Some("ctx-persist-idem".to_string()),
                ..Default::default()
            }),
            Some("dispatch-persisted-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    assert!(first_dispatch.ok);
    drop(first_center);

    let second_center = open_center_at(&conn).await;
    let second_dispatch = second_center
        .handle_dispatch(
            msg,
            Some(IngressContext {
                context_id: Some("ctx-persist-idem".to_string()),
                ..Default::default()
            }),
            Some("dispatch-persisted-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(first_dispatch, second_dispatch);

    let inbox = second_center
        .handle_peek_box(recipient, MailboxKind::Inbox, None, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(inbox.len(), 1);
}

#[tokio::test]
async fn idempotency_key_is_isolated_by_message_owner() {
    let (center, _tmp) = new_center("idempotency_owner_scope").await;
    let transport_did = DID::new("bns", "tg-owner-scope-tunnel");
    center
        .register_tunnel(
            "tg-owner-scope".to_string(),
            transport_did.clone(),
            "telegram".to_string(),
        )
        .unwrap();
    let target = DID::new("msgtunnel", "888.user.tg-owner-scope");
    let first = center
        .handle_post_send(
            make_msg(
                DID::new("bns", "owner-scope-a"),
                vec![target.clone()],
                MsgObjKind::Chat,
            ),
            Some("shared-caller-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    let second = center
        .handle_post_send(
            make_msg(
                DID::new("bns", "owner-scope-b"),
                vec![target],
                MsgObjKind::Chat,
            ),
            Some("shared-caller-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();

    assert_ne!(first.msg_id, second.msg_id);
    assert_ne!(first.deliveries, second.deliveries);
    let first_delivery = center
        .handle_get_next_delivery(transport_did.clone(), Some(true), None, ctx())
        .await
        .unwrap();
    let second_delivery = center
        .handle_get_next_delivery(transport_did, Some(true), None, ctx())
        .await
        .unwrap();
    assert!(first_delivery.is_some());
    assert!(second_delivery.is_some());
}

#[test]
fn telegram_retention_bucket_uses_bot_and_chat_not_sender() {
    let msg = make_msg(
        DID::new("bns", "external-sender"),
        vec![DID::new("bns", "recipient")],
        MsgObjKind::Chat,
    );
    let ingress = |sender: &str, bot: &str| IngressContext {
        transport_did: Some(DID::new("bns", "tg-retention-tunnel")),
        platform: Some("telegram".to_string()),
        chat_id: Some("-100123".to_string()),
        source_account_id: Some(sender.to_string()),
        extra: Some(json!({"tunnel_account_id": bot})),
        ..Default::default()
    };

    let first = MessageCenter::dispatch_idempotency_retention_key(
        &msg,
        Some(&ingress("telegram-user-1", "@zone_bot")),
    );
    let second = MessageCenter::dispatch_idempotency_retention_key(
        &msg,
        Some(&ingress("telegram-user-2", "@zone_bot")),
    );
    let other_bot = MessageCenter::dispatch_idempotency_retention_key(
        &msg,
        Some(&ingress("telegram-user-1", "@other_bot")),
    );

    assert_eq!(first, second);
    assert_ne!(first, other_bot);
}

// ---------------------------------------------------------------------------
// Owner-scoped session lifecycle, activity ordering, registration, auth.
// ---------------------------------------------------------------------------

mod owner_session_tests {
    use super::*;
    use crate::owner_session::SessionTokenVerifier;
    use buckyos_api::{
        bind_token_principal_kind, bind_token_target, AuthTarget, MsgCenterCreateSessionReq,
        SessionLifecycle, SessionListLifecycleFilter, SessionListOrder, SystemServiceId,
        TokenPrincipalKind, TokenUse, VERIFY_HUB_UNIQUE_ID,
    };
    use jsonwebtoken::{DecodingKey, EncodingKey};
    use kRPC::{RPCErrors, RPCSessionToken, RPCSessionTokenType};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn chat_at(from: &DID, to: Vec<DID>, text: &str, created_at_ms: u64) -> MsgObject {
        let mut msg = make_msg(from.clone(), to, MsgObjKind::Chat);
        msg.created_at_ms = created_at_ms;
        msg.content.content = text.to_string();
        msg
    }

    async fn grant(center: &MessageCenter, peer: &DID, owner: &DID, context: &str) {
        center
            .handle_grant_temporary_access(
                vec![peer.clone()],
                context.to_string(),
                3600,
                Some(owner.clone()),
                ctx(),
            )
            .await
            .unwrap();
    }

    async fn inbound(center: &MessageCenter, msg: MsgObject, context: &str, key: &str) {
        center
            .handle_dispatch(
                msg,
                Some(IngressContext {
                    context_id: Some(context.to_string()),
                    ..Default::default()
                }),
                Some(key.to_string()),
                ctx(),
            )
            .await
            .unwrap();
    }

    async fn list(
        center: &MessageCenter,
        owner: &DID,
        lifecycle: SessionListLifecycleFilter,
        order: SessionListOrder,
        limit: Option<usize>,
        cursor: Option<(u64, String)>,
    ) -> buckyos_api::SessionSummaryPage {
        let (cursor_value, cursor_session) = match cursor {
            Some((value, id)) => (Some(value), Some(id)),
            None => (None, None),
        };
        center
            .handle_list_sessions(
                owner.clone(),
                limit,
                cursor_value,
                cursor_session,
                Some(false),
                Some(lifecycle),
                Some(order),
                ctx(),
            )
            .await
            .unwrap()
    }

    async fn timeline_len(center: &MessageCenter, owner: &DID, session_id: &str) -> usize {
        center
            .handle_list_session(
                owner.clone(),
                session_id.to_string(),
                None,
                None,
                None,
                Some(false),
                Some(false),
                ctx(),
            )
            .await
            .unwrap()
            .items
            .len()
    }

    #[tokio::test]
    async fn lifecycle_archive_restore_delete_and_watermark() {
        let (center, _tmp) = new_center("lifecycle").await;
        let owner = DID::new("bns", "lc-owner");
        let other = DID::new("bns", "lc-other");
        let peer = DID::new("bns", "lc-peer");
        grant(&center, &peer, &owner, "lc").await;
        grant(&center, &peer, &other, "lc").await;
        let session_id = format!("dm:{}", peer.to_string());

        let msg1 = chat_at(&peer, vec![owner.clone(), other.clone()], "one", 2_000_000);
        inbound(&center, msg1.clone(), "lc", "lc-1").await;
        let active = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert_eq!(active.items.len(), 1);
        assert_eq!(active.items[0].session_id, session_id);
        assert_eq!(active.items[0].unread_count, 1);
        assert_eq!(active.items[0].last_activity_ms, 2_000_000);
        assert_eq!(active.items[0].lifecycle, SessionLifecycle::Active);

        // Archive keeps the per-record read state and the unread count.
        let archived = center
            .handle_archive_session(owner.clone(), session_id.clone(), ctx())
            .await
            .unwrap();
        assert_eq!(archived.lifecycle, SessionLifecycle::Archived);
        assert!(archived.archived_at_ms.is_some());
        assert!(list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None
        )
        .await
        .items
        .is_empty());
        let archived_list = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Archived,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert_eq!(archived_list.items.len(), 1);
        assert_eq!(archived_list.items[0].unread_count, 1);
        assert_eq!(archived_list.items[0].lifecycle, SessionLifecycle::Archived);
        // Idempotent repeat.
        center
            .handle_archive_session(owner.clone(), session_id.clone(), ctx())
            .await
            .unwrap();

        // Restore keeps history and activity time.
        let restored = center
            .handle_restore_session(owner.clone(), session_id.clone(), ctx())
            .await
            .unwrap();
        assert_eq!(restored.lifecycle, SessionLifecycle::Active);
        let active = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert_eq!(active.items.len(), 1);
        assert_eq!(active.items[0].last_activity_ms, 2_000_000);
        assert_eq!(timeline_len(&center, &owner, &session_id).await, 1);

        // Archive again: a read-state change must not re-activate, an event
        // message must not re-activate, a new chat message must.
        center
            .handle_archive_session(owner.clone(), session_id.clone(), ctx())
            .await
            .unwrap();
        let record_id = center
            .handle_list_session(
                owner.clone(),
                session_id.clone(),
                None,
                None,
                None,
                None,
                None,
                ctx(),
            )
            .await
            .unwrap()
            .items[0]
            .record_id
            .clone();
        center
            .handle_update_record_state(record_id, RecipientState::Read, ctx())
            .await
            .unwrap();
        assert!(list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None
        )
        .await
        .items
        .is_empty());
        let mut event = chat_at(&peer, vec![owner.clone()], "log", 2_000_500);
        event.kind = MsgObjKind::Event;
        event.thread.topic = Some(session_id.clone());
        inbound(&center, event, "lc", "lc-event").await;
        assert!(list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None
        )
        .await
        .items
        .is_empty());
        let msg2 = chat_at(&peer, vec![owner.clone()], "two", 2_001_000);
        inbound(&center, msg2, "lc", "lc-2").await;
        let active = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert_eq!(active.items.len(), 1);
        assert_eq!(active.items[0].lifecycle, SessionLifecycle::Active);
        assert_eq!(active.items[0].unread_count, 2);
        assert_eq!(active.items[0].last_activity_ms, 2_001_000);
        assert_eq!(timeline_len(&center, &owner, &session_id).await, 3);

        // Delete: only this owner's visibility changes.
        let deleted = center
            .handle_delete_session(owner.clone(), session_id.clone(), ctx())
            .await
            .unwrap();
        assert!(deleted.delete_watermark_sort_key.unwrap() >= 2_001_000);
        assert!(!deleted.registered);
        assert!(list(
            &center,
            &owner,
            SessionListLifecycleFilter::All,
            SessionListOrder::Activity,
            None,
            None
        )
        .await
        .items
        .is_empty());
        assert_eq!(timeline_len(&center, &owner, &session_id).await, 0);
        let others = list(
            &center,
            &other,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert_eq!(others.items.len(), 1);
        assert_eq!(timeline_len(&center, &other, &session_id).await, 1);

        // Replaying the deleted message does not resurrect it.
        inbound(&center, msg1, "lc", "lc-replay").await;
        assert!(list(
            &center,
            &owner,
            SessionListLifecycleFilter::All,
            SessionListOrder::Activity,
            None,
            None
        )
        .await
        .items
        .is_empty());
        assert_eq!(timeline_len(&center, &owner, &session_id).await, 0);

        // A genuinely new message forms a fresh visible history.
        let watermark = deleted.delete_watermark_sort_key.unwrap();
        let msg3 = chat_at(&peer, vec![owner.clone()], "three", watermark + 10);
        inbound(&center, msg3, "lc", "lc-3").await;
        let active = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert_eq!(active.items.len(), 1);
        assert_eq!(active.items[0].unread_count, 1);
        assert_eq!(active.items[0].last_activity_ms, watermark + 10);
        assert_eq!(timeline_len(&center, &owner, &session_id).await, 1);
    }

    #[tokio::test]
    async fn activity_order_ignores_read_state_and_events_and_pages_consistently() {
        let (center, _tmp) = new_center("activity").await;
        let owner = DID::new("bns", "act-owner");
        let peer = DID::new("bns", "act-peer");
        grant(&center, &peer, &owner, "act").await;

        let mut s1 = chat_at(&peer, vec![owner.clone()], "s1", 3_000_000);
        s1.thread.topic = Some("s1".to_string());
        let mut s2 = chat_at(&peer, vec![owner.clone()], "s2", 3_000_100);
        s2.thread.topic = Some("s2".to_string());
        inbound(&center, s1, "act", "act-1").await;
        inbound(&center, s2, "act", "act-2").await;

        let order = |page: &buckyos_api::SessionSummaryPage| {
            page.items
                .iter()
                .map(|i| i.session_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            order(
                &list(
                    &center,
                    &owner,
                    SessionListLifecycleFilter::Active,
                    SessionListOrder::Activity,
                    None,
                    None
                )
                .await
            ),
            vec!["s2", "s1"]
        );

        // Reading s1 bumps updated_at_ms (legacy order) but not activity.
        let record_id = center
            .handle_list_session(
                owner.clone(),
                "s1".to_string(),
                None,
                None,
                None,
                None,
                None,
                ctx(),
            )
            .await
            .unwrap()
            .items[0]
            .record_id
            .clone();
        center
            .handle_update_record_state(record_id, RecipientState::Read, ctx())
            .await
            .unwrap();
        assert_eq!(
            order(
                &list(
                    &center,
                    &owner,
                    SessionListLifecycleFilter::Active,
                    SessionListOrder::Updated,
                    None,
                    None
                )
                .await
            ),
            vec!["s1", "s2"]
        );
        assert_eq!(
            order(
                &list(
                    &center,
                    &owner,
                    SessionListLifecycleFilter::Active,
                    SessionListOrder::Activity,
                    None,
                    None
                )
                .await
            ),
            vec!["s2", "s1"]
        );

        // An event (action log) in s1 does not move it either.
        let mut event = chat_at(&peer, vec![owner.clone()], "log", 3_000_200);
        event.kind = MsgObjKind::Event;
        event.thread.topic = Some("s1".to_string());
        inbound(&center, event, "act", "act-3").await;
        let page = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert_eq!(order(&page), vec!["s2", "s1"]);
        assert_eq!(page.items[1].last_activity_ms, 3_000_000);
        assert_eq!(
            page.items[1].unread_count, 1,
            "persistent event still counts as unread"
        );

        // A new chat in s1 moves it first; paging with limit 1 is stable.
        let mut chat = chat_at(&peer, vec![owner.clone()], "s1 again", 3_000_300);
        chat.thread.topic = Some("s1".to_string());
        inbound(&center, chat, "act", "act-4").await;
        let first = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            Some(1),
            None,
        )
        .await;
        assert_eq!(order(&first), vec!["s1"]);
        assert_eq!(first.next_cursor_updated_at_ms, Some(3_000_300));
        assert_eq!(first.next_cursor_session_id.as_deref(), Some("s1"));
        let second = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            Some(1),
            Some((3_000_300, "s1".to_string())),
        )
        .await;
        assert_eq!(order(&second), vec!["s2"]);
        assert!(second.next_cursor_session_id.is_none());
    }

    #[tokio::test]
    async fn create_session_registers_empty_session_and_first_message_joins_it() {
        let (center, _tmp) = new_center("create").await;
        let hub = DID::new("bns", "hub-create");
        center.set_message_hub_did(hub);
        let owner = DID::new("bns", "cr-owner");
        let agent = DID::new("bns", "cr-agent");
        center.register_local_recipients([agent.clone()]);

        let create = |title: &str| MsgCenterCreateSessionReq {
            owner: owner.clone(),
            peer_did: agent.clone(),
            session_id: None,
            title: Some(title.to_string()),
            binding: Some(json!({ "kind": "native", "targetDid": agent.to_string() })),
            origin: None,
        };
        let first = center
            .handle_create_session(create("Plan"), ctx())
            .await
            .unwrap();
        let second = center
            .handle_create_session(create("Plan"), ctx())
            .await
            .unwrap();
        assert_ne!(
            first.session_id, second.session_id,
            "same title, independent sessions"
        );
        assert!(first.registered);
        assert_eq!(first.title.as_deref(), Some("Plan"));
        assert_eq!(first.origin.as_deref(), Some("manual"));

        let page = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert_eq!(page.items.len(), 2);
        let empty = page
            .items
            .iter()
            .find(|i| i.session_id == first.session_id)
            .unwrap();
        assert!(empty.last_record.is_none());
        assert_eq!(empty.unread_count, 0);
        assert_eq!(empty.last_activity_ms, first.created_at_ms);
        assert_eq!(timeline_len(&center, &owner, &first.session_id).await, 0);

        // Caller-chosen id is idempotent; a title over 64 chars is rejected.
        let mut fixed = create("Fixed");
        fixed.session_id = Some("fixed-id".to_string());
        let a = center
            .handle_create_session(fixed.clone(), ctx())
            .await
            .unwrap();
        let b = center.handle_create_session(fixed, ctx()).await.unwrap();
        assert_eq!(a, b);
        assert!(center
            .handle_create_session(create(&"x".repeat(65)), ctx())
            .await
            .is_err());

        // The first message uses the session id as topic and lands there.
        let mut msg = chat_at(&owner, vec![agent.clone()], "hello", 4_000_000);
        msg.thread.topic = Some(first.session_id.clone());
        let post = center.handle_post_send(msg, None, ctx()).await.unwrap();
        assert!(post.ok, "{:?}", post.reason);
        assert_eq!(timeline_len(&center, &owner, &first.session_id).await, 1);
        let page = list(
            &center,
            &owner,
            SessionListLifecycleFilter::Active,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        let joined = page
            .items
            .iter()
            .find(|i| i.session_id == first.session_id)
            .unwrap();
        assert_eq!(joined.last_activity_ms, 4_000_000);
        assert_eq!(
            joined.last_record.as_ref().unwrap().record.box_kind,
            MailboxKind::Sent
        );
        assert!(joined.state.as_ref().unwrap().registered);

        // Deleting a registered session drops the registration too.
        center
            .handle_delete_session(owner.clone(), first.session_id.clone(), ctx())
            .await
            .unwrap();
        let page = list(
            &center,
            &owner,
            SessionListLifecycleFilter::All,
            SessionListOrder::Activity,
            None,
            None,
        )
        .await;
        assert!(page.items.iter().all(|i| i.session_id != first.session_id));
    }

    #[tokio::test]
    async fn owner_scoped_ui_state_is_isolated_and_cleared_by_delete() {
        let (center, _tmp) = new_center("ui-state").await;
        let a = DID::new("bns", "ui-a");
        let b = DID::new("bns", "ui-b");
        let peer = DID::new("bns", "ui-peer");
        grant(&center, &peer, &a, "ui").await;
        inbound(
            &center,
            chat_at(&peer, vec![a.clone()], "hi", 5_000_000),
            "ui",
            "ui-1",
        )
        .await;
        let session_id = format!("dm:{}", peer.to_string());

        center
            .handle_update_owner_ui_session_state(
                a.clone(),
                session_id.clone(),
                "ui.title".into(),
                json!("Mine"),
                ctx(),
            )
            .await
            .unwrap();
        let mine = center
            .handle_get_owner_ui_session_state(
                a.clone(),
                session_id.clone(),
                "ui.title".into(),
                ctx(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mine.value, json!("Mine"));
        assert!(center
            .handle_get_owner_ui_session_state(
                b.clone(),
                session_id.clone(),
                "ui.title".into(),
                ctx()
            )
            .await
            .unwrap()
            .is_none());
        assert!(
            center
                .handle_get_ui_session_state(session_id.clone(), "ui.title".into(), ctx())
                .await
                .unwrap()
                .is_none(),
            "legacy session-only KV is untouched"
        );
        assert_eq!(
            center
                .handle_list_owner_ui_session_state(a.clone(), session_id.clone(), ctx())
                .await
                .unwrap()
                .len(),
            1
        );
        center
            .handle_delete_session(a.clone(), session_id.clone(), ctx())
            .await
            .unwrap();
        assert!(center
            .handle_list_owner_ui_session_state(a.clone(), session_id.clone(), ctx())
            .await
            .unwrap()
            .is_empty());
    }

    // ---- authorization -------------------------------------------------

    const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#;
    const TEST_PUBLIC_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

    struct StaticKeyVerifier {
        key: DecodingKey,
    }

    #[async_trait::async_trait]
    impl SessionTokenVerifier for StaticKeyVerifier {
        async fn verify(&self, token: &str) -> std::result::Result<RPCSessionToken, RPCErrors> {
            let mut parsed = RPCSessionToken::from_string(token)?;
            parsed.verify_by_key(&self.key)?;
            Ok(parsed)
        }
    }

    fn signed_token(user_id: &str, principal_kind: TokenPrincipalKind) -> String {
        let now = buckyos_kit::buckyos_get_unix_timestamp();
        let mut token = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            exp: Some(now + 3600),
            iss: Some(VERIFY_HUB_UNIQUE_ID.to_string()),
            jti: None,
            sub: Some(user_id.to_string()),
            appid: None,
            sudo: false,
            extra: HashMap::new(),
        };
        bind_token_principal_kind(&mut token, principal_kind);
        bind_token_target(
            &mut token,
            &AuthTarget::system(SystemServiceId::parse("control-panel").unwrap()),
            TokenUse::Session,
        )
        .unwrap();
        token
            .generate_jwt(
                None,
                &EncodingKey::from_ed_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
            )
            .unwrap()
    }

    fn user_ctx(user_id: &str) -> RPCContext {
        RPCContext {
            token: Some(signed_token(user_id, TokenPrincipalKind::User)),
            ..Default::default()
        }
    }

    fn is_denied<T: std::fmt::Debug>(result: std::result::Result<T, RPCErrors>) -> bool {
        matches!(result, Err(RPCErrors::NoPermission(_)))
    }

    #[tokio::test]
    async fn zone_user_tokens_scope_reads_and_writes_to_the_owner() {
        let (center, _tmp) = new_center("auth").await;
        center.set_token_verifier(Arc::new(StaticKeyVerifier {
            key: DecodingKey::from_ed_components(TEST_PUBLIC_X).unwrap(),
        }));
        let alice = DID::new("bns", "alice");
        let bob = DID::new("bns", "bob");
        let agent = DID::new("web", "jarvis.zone.example");
        center.register_local_recipients([alice.clone(), bob.clone(), agent.clone()]);
        let peer = DID::new("bns", "auth-peer");
        for owner in [&alice, &bob, &agent] {
            grant(&center, &peer, owner, "auth").await;
        }
        inbound(
            &center,
            chat_at(
                &peer,
                vec![alice.clone(), bob.clone(), agent.clone()],
                "hi",
                6_000_000,
            ),
            "auth",
            "auth-1",
        )
        .await;
        let session_id = format!("dm:{}", peer.to_string());

        let list_as = |owner: DID, ctx: RPCContext| {
            let center = center.clone();
            async move {
                center
                    .handle_list_sessions(owner, None, None, None, None, None, None, ctx)
                    .await
            }
        };
        // Self and zone-hosted agent are readable, another user is not.
        assert_eq!(
            list_as(alice.clone(), user_ctx("alice"))
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(
            list_as(agent.clone(), user_ctx("alice"))
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        assert!(is_denied(list_as(bob.clone(), user_ctx("alice")).await));
        assert!(is_denied(
            center
                .handle_list_session(
                    bob.clone(),
                    session_id.clone(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    user_ctx("alice")
                )
                .await
        ));
        // Observation is read-only: no writes as the agent or as another user.
        assert!(is_denied(
            center
                .handle_archive_session(agent.clone(), session_id.clone(), user_ctx("alice"))
                .await
        ));
        assert!(is_denied(
            center
                .handle_update_owner_ui_session_state(
                    agent.clone(),
                    session_id.clone(),
                    "ui.title".into(),
                    json!("x"),
                    user_ctx("alice")
                )
                .await
        ));
        let bob_record = center
            .handle_list_session(
                bob.clone(),
                session_id.clone(),
                None,
                None,
                None,
                None,
                None,
                user_ctx("bob"),
            )
            .await
            .unwrap()
            .items[0]
            .record_id
            .clone();
        assert!(is_denied(
            center
                .handle_update_record_state(
                    bob_record.clone(),
                    RecipientState::Read,
                    user_ctx("alice")
                )
                .await
        ));
        assert!(center
            .handle_update_record_state(bob_record, RecipientState::Read, user_ctx("bob"))
            .await
            .is_ok());
        assert!(is_denied(
            center
                .handle_post_send(
                    chat_at(&bob, vec![peer.clone()], "as bob", 6_000_100),
                    None,
                    user_ctx("alice")
                )
                .await
        ));
        // Own writes succeed; a forged token is rejected; no token keeps the
        // legacy in-process behaviour.
        assert!(center
            .handle_archive_session(alice.clone(), session_id.clone(), user_ctx("alice"))
            .await
            .is_ok());
        let forged = RPCContext {
            token: Some("eyJhbGciOiJFZERTQSJ9.e30.invalid".to_string()),
            ..Default::default()
        };
        assert!(is_denied(list_as(alice.clone(), forged).await));
        assert_eq!(list_as(bob.clone(), ctx()).await.unwrap().items.len(), 1);
        // System principals keep their service-level access.
        let system = RPCContext {
            token: Some(signed_token("msg-center", TokenPrincipalKind::System)),
            ..Default::default()
        };
        assert_eq!(list_as(bob.clone(), system).await.unwrap().items.len(), 1);
    }
}
