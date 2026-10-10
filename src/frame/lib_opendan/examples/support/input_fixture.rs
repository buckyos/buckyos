//! The input bus fixture (`14_input_bus`): hand-written bus records, what a
//! consumer does with each, and the byte-exact text the built-in formats
//! and the example templates render for one batch. Shared by the generator
//! and the test that checks the reference runner against it.
#![allow(dead_code)]

use buckyos_api::{AiContent, AiMessage, AiRole};
use libopendan::protocol::*;
use libopendan::runner::input_view::{
    event_view, media_blocks, message_view, InputItem, InputView,
};
use serde_json::{json, Value};

pub const AGENT_DID: &str = "did:bns:jarvis";
/// 2026-10-02T00:00:00Z.
pub const T0: u64 = 1_790_899_200_000;
/// Time of the rendered batch.
pub const BATCH_MS: u64 = T0 + 1_000;

fn hex_id(kind: &str, byte: u8) -> String {
    format!("{kind}:{}", hex::encode([byte; 32]))
}

fn record(input: PostedInput) -> Value {
    serde_json::to_value(input).unwrap()
}

/// The accepted records, by file name.
pub fn records() -> Vec<(&'static str, Value)> {
    let alice = parse_did("did:bns:alice").unwrap();
    let agent = parse_did(AGENT_DID).unwrap();
    let mut plain = text_msg(&alice, &agent, "查询构建进度");
    plain.created_at_ms = T0;
    plain.nonce = Some(4211);
    let mut plain = PostedInput::msg("did:bns:alice", plain, MsgDelivery::default()).unwrap();
    plain.at_ms = T0;

    let group: ndn_lib::MsgObject = serde_json::from_value(json!({
        "from": "did:bns:bob",
        "to": ["did:bns:dev-team"],
        "kind": "group_msg",
        "thread": { "reply_to": hex_id("cymsg", 0x99) },
        "mentions": { "dids": [AGENT_DID] },
        "created_at_ms": T0,
        "content": {
            "format": "text/plain",
            "content": "@jarvis 帮我看一下这张截图里的报错 <build>，日志在附件里",
            "refs": [
                { "role": "input", "target": { "type": "data_obj", "obj_id": hex_id("cyfile", 0xa1), "uri_hint": "screenshot.png" }, "label": "screenshot.png" },
                { "role": "input", "target": { "type": "data_obj", "obj_id": hex_id("cyfile", 0xb2), "uri_hint": "build.log" }, "label": "build.log" }
            ]
        }
    }))
    .unwrap();
    let mut group = PostedInput::msg(
        "app:msg-bridge@alice",
        group,
        MsgDelivery {
            from_name: Some("Bob".into()),
            conversation_name: Some("Dev Team".into()),
            record_id: Some("r-102".into()),
            tunnel: None,
            context: false,
        },
    )
    .unwrap();
    group.at_ms = T0 + 500;

    let mut task = PostedInput::event(
        "app:task-bridge@alice",
        "task:t1:revision:7",
        AgentEvent {
            subscription_id: Some("watch-t1".into()),
            source: EventSource::task("t1"),
            event: "updated".into(),
            seq: Some(7),
            summary: "任务 t1 的状态发生变化".into(),
            data_ref: None,
            terminal: false,
        },
    );
    task.at_ms = T0;

    let mut object = PostedInput::event(
        "app:obj-bridge@alice",
        "obj:doc-1:8",
        AgentEvent {
            subscription_id: Some("watch-obj".into()),
            source: EventSource::new("object", "doc-1"),
            event: "changed".into(),
            seq: Some(8),
            summary: "doc-1 was edited by Bob".into(),
            data_ref: None,
            terminal: false,
        },
    );
    object.at_ms = T0;

    let mut stop = PostedInput::control(
        "did:bns:alice",
        "ctl:stop:1790899260000",
        ControlCommand::Stop {
            reason: Some("用户取消".into()),
        },
    );
    stop.at_ms = T0 + 60_000;

    vec![
        ("01_msg_text.json", record(plain)),
        ("02_msg_group_attachments.json", record(group)),
        ("03_event_active_task.json", record(task)),
        ("04_event_semi_object.json", record(object)),
        ("05_control_stop.json", record(stop)),
    ]
}

/// Subscriptions the receiving session needs for the two events.
pub fn subscriptions() -> Value {
    json!([
        { "id": "watch-t1", "mode": "active", "source": { "type": "task", "task_id": "t1" } },
        { "id": "watch-obj", "mode": "semi", "source": { "type": "object_event", "object": "doc-1" } }
    ])
}

/// What a consumer does with each accepted record.
pub fn expectations() -> Value {
    json!([
        { "file": "01_msg_text.json", "type": "msg",
          "handling": "candidate of a controlled input batch; opens a Turn or joins the open one",
          "reply_after": { "route": "message", "to": "did:bns:alice", "kind": "chat" } },
        { "file": "02_msg_group_attachments.json", "type": "msg",
          "handling": "candidate of a controlled input batch; the speaker is msg.from (Bob), not the poster (the bridge)",
          "reply_after": { "route": "message", "to": "did:bns:dev-team", "kind": "group_msg" } },
        { "file": "03_event_active_task.json", "type": "event", "needs_subscription": "watch-t1",
          "handling": "Input: candidate of a controlled input batch; without the subscription it is dropped (event_dropped: unsubscribed)" },
        { "file": "04_event_semi_object.json", "type": "event", "needs_subscription": "watch-obj",
          "handling": "Observe: merged into state.pending_events and consumed; never triggers inference; shown as the semi_subscription_snapshot message before the next controlled input" },
        { "file": "05_control_stop.json", "type": "control",
          "handling": "applied by the driver (stop_requested); never enters the context, no receipt" }
    ])
}

/// One record per rejection reason that depends on the record alone.
pub fn rejected() -> Vec<(&'static str, &'static str, Value)> {
    let good = records();
    let plain = good[0].1.clone();
    let with = |base: &Value, f: &dyn Fn(&mut Value)| {
        let mut v = base.clone();
        f(&mut v);
        v
    };
    vec![
        (
            "unsupported_schema.json",
            "unsupported_schema",
            with(&plain, &|v| v["schema"] = json!("opendan.session_input/2")),
        ),
        (
            "unknown_type.json",
            "unknown_type",
            json!({ "schema": SESSION_INPUT_SCHEMA, "type": "change", "key": "cam#motion",
                    "from": "app:x@alice", "at_ms": T0, "payload": { "text": "motion" } }),
        ),
        (
            "invalid_envelope_key_is_not_the_obj_id.json",
            "invalid_envelope",
            with(&plain, &|v| v["key"] = json!("m-1")),
        ),
        (
            "invalid_envelope_missing_from.json",
            "invalid_envelope",
            with(&good[4].1, &|v| {
                v.as_object_mut().unwrap().remove("from");
            }),
        ),
        (
            "payload_not_json.json",
            "payload_not_json",
            with(&good[4].1, &|v| v["payload"] = json!("stop")),
        ),
        (
            "invalid_payload_event_without_summary.json",
            "invalid_payload",
            with(&good[2].1, &|v| {
                v["payload"].as_object_mut().unwrap().remove("summary");
            }),
        ),
        (
            "invalid_payload_msg_not_valid.json",
            "invalid_payload",
            json!({ "schema": SESSION_INPUT_SCHEMA, "type": "msg", "key": "cymsg:00",
                    "from": "did:bns:alice", "at_ms": T0,
                    "payload": { "msg": { "from": "did:bns:alice", "to": ["did:bns:a", "did:bns:b"],
                                           "kind": "chat", "to_session": "s1",
                                           "content": { "content": "hi" } } } }),
        ),
        (
            "unknown_command.json",
            "unknown_command",
            with(&good[4].1, &|v| v["payload"] = json!({ "command": "reboot" })),
        ),
    ]
}

/// A record whose payload exceeds the limit (not stored: 260 KB).
pub fn too_large() -> Value {
    json!({ "schema": SESSION_INPUT_SCHEMA, "type": "event", "key": "big:1",
            "from": "app:x@alice", "at_ms": T0,
            "payload": { "source": { "kind": "object", "id": "o" }, "event": "changed",
                          "summary": "s", "padding": "x".repeat(260 * 1024) } })
}

/// Files of the rendered batch, in consumption order.
pub const BATCH: &[&str] = &["02_msg_group_attachments.json", "03_event_active_task.json"];

fn item(rec: &Value) -> InputItem {
    let key = rec["key"].as_str().unwrap();
    let at_ms = rec["at_ms"].as_u64().unwrap();
    match parse_logical_record(rec).unwrap() {
        SessionInput::Msg(m) => InputItem::Msg(message_view(key, at_ms, &m, AGENT_DID)),
        SessionInput::Event(e) => InputItem::Event(event_view(key, &e)),
        SessionInput::Control(_) => panic!("controls never enter a batch"),
    }
}

/// The view of the batch (`input.*`).
pub fn batch_view(records: &[(String, Value)], hook: &str) -> InputView {
    let items = BATCH
        .iter()
        .map(|f| item(&records.iter().find(|(n, _)| n == f).unwrap().1))
        .collect();
    InputView::new(hook, BATCH_MS, items)
}

/// Template variables of the examples (`input` plus what a host assembles).
pub fn vars(view: &InputView) -> Value {
    json!({
        "input": view,
        "session": {
            "objective": "Fix the failing build",
            "background_hint_changed": true,
            "default_changed_background_hint_text": "Working directory: /workspace",
            "current_todo": { "id": "T3", "status": "in_progress", "summary": "Fix the <build> error" },
        },
        "runtime": { "clock_text": view.time },
    })
}

/// §3.3.8 examples: `(name, template)`.
pub const TEMPLATES: &[(&str, &str)] = &[
    (
        "example_1_builtin_equivalent",
        "<session_input hook=\"{{ input.hook }}\" time=\"{{ input.time }}\">\n{{ input | render_format: \"input.xml\" }}\n</session_input>\n",
    ),
    (
        "example_2_ui_dialogue",
        "{% if session.background_hint_changed %}\n<background_environment current_clock=\"{{ runtime.clock_text }}\">\n{{ session.default_changed_background_hint_text }}\n</background_environment>\n{% endif %}\n{{ session.current_todo | render_format: \"todo.summary_xml\" }}\n{{ input | render_format: \"input.xml\" }}\n",
    ),
    (
        "example_3_group_chat",
        "<group_chat time=\"{{ input.time }}\">\n{% for msg in input.messages %}\n{{ msg | render_format: \"message.xml\" }}\n{% endfor %}\n</group_chat>\nReply only if you were mentioned or the message is clearly addressed to you.\n",
    ),
    (
        "example_4_task_markdown",
        "# Task\n{{ session.objective }}\n\n{% for msg in input.messages %}\n{{ msg | render_format: \"message.markdown\" }}\n{% endfor %}\n",
    ),
    (
        "example_5_event_driven",
        "Wakeup at {{ input.time }}\n{% for ev in input.events %}\n{{ ev | render_format: \"event.summary_text\" }}\n{% endfor %}\n{% for msg in input.messages %}\n{{ msg | render_format: \"message.xml\" }}\n{% endfor %}\n",
    ),
    (
        "example_6_mixed_in_order",
        "{% for item in input.items %}\n{% if item.is_msg %}{{ item | render_format: \"message.xml\" }}{% else %}{{ item | render_format: \"event.xml\" }}{% endif %}\n{% endfor %}\n",
    ),
];

/// The user message a batch becomes (`input.media` decides the blocks
/// after the text; the text is the same either way).
pub fn user_message(text: &str, view: &InputView, media: InputMedia) -> AiMessage {
    let mut m = AiMessage::text(AiRole::User, text.to_string());
    let blocks: Vec<AiContent> = media_blocks(&view.messages, media);
    m.content.extend(blocks);
    m
}
