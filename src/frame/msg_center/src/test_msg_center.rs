use crate::contact_mgr::ContactMgr;
use crate::msg_center::MessageCenter;
use buckyos_api::{
    BoxKind, DeliveryReportResult, IngressContext, MsgCenterHandler, MsgObject, MsgState,
    ReadReceiptState, SendContext,
};
use kRPC::RPCContext;
use name_lib::DID;
use ndn_lib::ObjId;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_MSG_SEQ: AtomicU64 = AtomicU64::new(1000);
static TEST_TIME_SEQ: AtomicU64 = AtomicU64::new(10_000);

fn next_obj_id() -> ObjId {
    let seq = TEST_MSG_SEQ.fetch_add(1, Ordering::SeqCst);
    ObjId::new(&format!("chunk:{}", seq)).unwrap()
}

fn next_created_at_ms() -> u64 {
    TEST_TIME_SEQ.fetch_add(1, Ordering::SeqCst)
}

fn test_db_path(tag: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "msg_center_{}_{}_{}.sqlite3",
        tag,
        std::process::id(),
        now
    ))
}

fn new_center(tag: &str) -> MessageCenter {
    let mgr = ContactMgr::new_with_path(test_db_path(tag)).unwrap();
    MessageCenter::new(mgr)
}

fn make_msg(from: DID, source: Option<DID>, to: Vec<DID>) -> MsgObject {
    MsgObject {
        id: next_obj_id(),
        from,
        source,
        to,
        thread_key: None,
        payload: json!({
            "kind": "text",
            "text": "hello",
        }),
        meta: None,
        created_at_ms: next_created_at_ms(),
    }
}

fn ctx() -> RPCContext {
    RPCContext::default()
}

#[tokio::test]
async fn dispatch_single_chat_goes_to_inbox_and_locking_moves_state() {
    let center = new_center("dispatch_inbox");
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

    let msg = make_msg(sender.clone(), None, vec![recipient.clone()]);
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
        .handle_peek_box(recipient.clone(), BoxKind::Inbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].record.state, MsgState::Unread);

    let next = center
        .handle_get_next(recipient.clone(), BoxKind::Inbox, None, None, ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.record.state, MsgState::Reading);

    let no_more_unread = center
        .handle_get_next(recipient, BoxKind::Inbox, None, None, ctx())
        .await
        .unwrap();
    assert!(no_more_unread.is_none());
}

#[tokio::test]
async fn dispatch_stranger_goes_to_request_box() {
    let center = new_center("dispatch_request");
    let sender = DID::new("bns", "sender-b");
    let recipient = DID::new("bns", "recipient-b");
    let msg = make_msg(sender, None, vec![recipient.clone()]);

    let dispatch = center
        .handle_dispatch(msg, None, None, ctx())
        .await
        .unwrap();
    assert!(dispatch.ok);
    assert!(dispatch.delivered_recipients.contains(&recipient));

    let inbox = center
        .handle_peek_box(recipient.clone(), BoxKind::Inbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(inbox.len(), 0);

    let request_box = center
        .handle_peek_box(recipient, BoxKind::RequestBox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(request_box.len(), 1);
    assert_eq!(request_box[0].record.state, MsgState::Unread);
}

#[tokio::test]
async fn dispatch_group_message_creates_group_and_agent_views() {
    let center = new_center("dispatch_group");
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

    let msg = make_msg(group_id.clone(), Some(author), Vec::new());
    let dispatch = center
        .handle_dispatch(msg, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(dispatch.delivered_group, Some(group_id.clone()));
    assert_eq!(dispatch.delivered_agents.len(), 2);

    let group_box = center
        .handle_peek_box(group_id.clone(), BoxKind::GroupInbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(group_box.len(), 1);

    let agent1_box = center
        .handle_peek_box(agent_1, BoxKind::Inbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(agent1_box.len(), 1);

    let agent2_box = center
        .handle_peek_box(agent_2, BoxKind::Inbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(agent2_box.len(), 1);
}

#[tokio::test]
async fn post_send_creates_owner_and_tunnel_outbox_records() {
    let center = new_center("post_send");
    let author = DID::new("bns", "author-b");
    let target = DID::new("bns", "target-b");
    let msg = make_msg(author.clone(), None, vec![target]);

    let post_send = center
        .handle_post_send(
            msg,
            Some(SendContext {
                priority: Some(5),
                ..Default::default()
            }),
            None,
            ctx(),
        )
        .await
        .unwrap();
    assert!(post_send.ok);
    assert_eq!(post_send.deliveries.len(), 1);

    let owner_outbox = center
        .handle_peek_box(author, BoxKind::Outbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(owner_outbox.len(), 1);
    assert_eq!(owner_outbox[0].record.state, MsgState::Sent);

    let tunnel = post_send.deliveries[0].tunnel_did.clone();
    let tunnel_outbox = center
        .handle_peek_box(tunnel.clone(), BoxKind::TunnelOutbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(tunnel_outbox.len(), 1);
    assert_eq!(tunnel_outbox[0].record.state, MsgState::Wait);

    let next = center
        .handle_get_next(tunnel, BoxKind::TunnelOutbox, None, None, ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.record.state, MsgState::Sending);
}

#[tokio::test]
async fn report_delivery_handles_success_and_failure_paths() {
    let center = new_center("report_delivery");
    let sender = DID::new("bns", "sender-c");
    let target = DID::new("bns", "target-c");

    let fail_msg = make_msg(sender.clone(), None, vec![target.clone()]);
    let fail_post = center
        .handle_post_send(fail_msg, None, None, ctx())
        .await
        .unwrap();
    let fail_record_id = fail_post.deliveries[0].record_id.clone();

    let failed_record = center
        .handle_report_delivery(
            fail_record_id,
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
    assert_eq!(failed_record.state, MsgState::Dead);
    assert_eq!(failed_record.delivery.unwrap().attempts, 1);

    let success_msg = make_msg(sender, None, vec![target]);
    let success_post = center
        .handle_post_send(success_msg, None, None, ctx())
        .await
        .unwrap();
    let success_record_id = success_post.deliveries[0].record_id.clone();

    let success_record = center
        .handle_report_delivery(
            success_record_id,
            DeliveryReportResult {
                ok: true,
                external_msg_id: Some("ext-1".to_string()),
                ..Default::default()
            },
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(success_record.state, MsgState::Sent);
    assert_eq!(
        success_record.delivery.unwrap().external_msg_id,
        Some("ext-1".to_string())
    );
}

#[tokio::test]
async fn update_record_state_checks_transition_rules() {
    let center = new_center("update_state");
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

    let msg = make_msg(sender, None, vec![recipient.clone()]);
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
        .handle_peek_box(recipient, BoxKind::Inbox, None, None, ctx())
        .await
        .unwrap();
    let record_id = inbox[0].record.record_id.clone();

    let updated = center
        .handle_update_record_state(record_id.clone(), MsgState::Readed, None, ctx())
        .await
        .unwrap();
    assert_eq!(updated.state, MsgState::Readed);

    let invalid = center
        .handle_update_record_state(record_id, MsgState::Sent, None, ctx())
        .await;
    assert!(invalid.is_err());
}

#[tokio::test]
async fn list_box_by_time_supports_pagination() {
    let center = new_center("list_pagination");
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

    let first_msg = make_msg(sender.clone(), None, vec![recipient.clone()]);
    let second_msg = make_msg(sender, None, vec![recipient.clone()]);
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
            BoxKind::Inbox,
            None,
            Some(1),
            None,
            None,
            Some(true),
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
            BoxKind::Inbox,
            None,
            Some(1),
            page_1.next_cursor_sort_key,
            page_1.next_cursor_record_id,
            Some(true),
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(page_2.items.len(), 1);
}

#[tokio::test]
async fn read_receipt_can_be_set_and_queried() {
    let center = new_center("read_receipt");
    let group = DID::new("bns", "group-b");
    let author = DID::new("bns", "author-b");
    let reader = DID::new("bns", "reader-b");
    let msg = make_msg(group.clone(), Some(author), Vec::new());
    let msg_id = msg.id.clone();

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
    let center = new_center("idempotency");
    let sender = DID::new("bns", "sender-f");
    let recipient = DID::new("bns", "recipient-f");

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

    let dispatch_msg = make_msg(sender.clone(), None, vec![recipient.clone()]);
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
        .handle_peek_box(recipient.clone(), BoxKind::Inbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(inbox.len(), 1);

    let send_msg = make_msg(sender, None, vec![recipient]);
    let first_post = center
        .handle_post_send(
            send_msg.clone(),
            None,
            Some("post-idempotent-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    let second_post = center
        .handle_post_send(
            send_msg,
            None,
            Some("post-idempotent-key".to_string()),
            ctx(),
        )
        .await
        .unwrap();
    assert_eq!(first_post.deliveries.len(), 1);
    assert_eq!(first_post.deliveries, second_post.deliveries);

    let tunnel = first_post.deliveries[0].tunnel_did.clone();
    let tunnel_outbox = center
        .handle_peek_box(tunnel, BoxKind::TunnelOutbox, None, None, ctx())
        .await
        .unwrap();
    assert_eq!(tunnel_outbox.len(), 1);
}
