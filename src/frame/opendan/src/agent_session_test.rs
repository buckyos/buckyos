use super::*;

#[test]
fn compose_human_text_skips_empties() {
    let v = vec!["  ".to_string(), "hello".to_string(), "".to_string()];
    assert_eq!(compose_human_text(&v).as_deref(), Some("hello"));
}

#[test]
fn compose_human_text_joins() {
    let v = vec!["a".to_string(), "b".to_string()];
    assert_eq!(compose_human_text(&v).as_deref(), Some("a\n\nb"));
}

#[test]
fn compose_turn_message_preserves_structured_blocks() {
    let msg = AiMessage::new(
        AiRole::User,
        vec![
            AiContent::text("see this"),
            AiContent::Image {
                source: buckyos_api::ResourceRef::url(
                    "https://example.test/a.png".to_string(),
                    Some("image/png".to_string()),
                ),
            },
        ],
    );
    let out = compose_turn_message(&[msg], Some("[environment]".to_string())).unwrap();
    assert_eq!(out.role, AiRole::User);
    assert_eq!(out.content.len(), 2);
    assert_eq!(out.text_content(), "[environment]\n\nsee this");
    assert!(matches!(out.content[1], AiContent::Image { .. }));
}

#[test]
fn output_text_extraction() {
    let out = ContextOutput::Text {
        content: "hi".to_string(),
    };
    assert_eq!(output_to_text(&out).as_deref(), Some("hi"));
    let out = ContextOutput::Text {
        content: String::new(),
    };
    assert!(output_to_text(&out).is_none());
}

#[test]
fn pending_input_dedup_key_distinguishes_variants() {
    let msg = PendingInput::Msg {
        record_id: "abc".to_string(),
        from: "alice".to_string(),
        from_did: None,
        from_name: None,
        tunnel_did: None,
        text: "hi".to_string(),
        ai_message: AiMessage::text(AiRole::User, "hi"),
    };
    let event = PendingInput::Event {
        event_id: "abc".to_string(),
        data: serde_json::Value::Null,
    };
    assert_eq!(msg.dedup_key(), "msg:abc");
    assert_eq!(event.dedup_key(), "event:abc");
    assert_ne!(msg.dedup_key(), event.dedup_key());
}

#[test]
fn format_event_for_turn_includes_id_and_data() {
    let s = format_event_for_turn("/timer/wake", &serde_json::json!({"tick": 1}));
    assert!(s.contains("/timer/wake"));
    assert!(s.contains("tick"));
}

#[test]
fn format_event_for_turn_handles_null_payload() {
    let s = format_event_for_turn("/timer/wake", &serde_json::Value::Null);
    assert!(s.contains("/timer/wake"));
    assert!(!s.contains("null"));
}

#[test]
fn format_event_for_turn_uses_subscription_template() {
    let subscriptions = vec![EventSubscription {
        pattern: "/approval/**".to_string(),
        subscribed_at_ms: 0,
        message_template: Some("Approval changed to {status}: {message}".to_string()),
    }];
    let s = format_event_for_turn_with_subscriptions(
        "/approval/doc-1",
        &serde_json::json!({"status": "approved", "message": "ready"}),
        &subscriptions,
    );
    assert_eq!(s, "Approval changed to approved: ready");
}

#[test]
fn event_batch_formats_single_user_wakeup() {
    let batch = format_event_batch_for_turn(&[
        EventForTurn {
            event_id: "/approval/doc-1".to_string(),
            data: serde_json::json!({"status": "approved"}),
            message: "Approval changed to approved".to_string(),
        },
        EventForTurn {
            event_id: "/task/7".to_string(),
            data: serde_json::Value::Null,
            message: "Task 7 completed".to_string(),
        },
    ])
    .expect("batch");
    assert!(batch.starts_with("[event batch]"));
    assert!(batch.contains("handled together as one wakeup"));
    assert!(batch.contains("Approval changed"));
    assert!(batch.contains("Task 7 completed"));
}

#[test]
fn pending_event_replacement_keeps_terminal_over_progress() {
    let existing = PendingInput::Event {
        event_id: "/task/7".to_string(),
        data: serde_json::json!({"to_status": "Completed"}),
    };
    let incoming = PendingInput::Event {
        event_id: "/task/7".to_string(),
        data: serde_json::json!({"to_status": "Running"}),
    };
    assert!(!should_replace_pending_event(&existing, &incoming));
    assert!(should_replace_pending_event(&incoming, &existing));
}

#[test]
fn session_meta_round_trips_pending_inputs() {
    // SessionMeta + PendingInput must round-trip through JSON so
    // `.meta/session.json` correctly preserves unconsumed inputs across
    // process restarts. If this breaks, persisted pendings are lost.
    let meta = SessionMeta {
        session_id: "s1".to_string(),
        kind: SessionKind::Ui,
        current_behavior: "ui_default".to_string(),
        status: SessionStatus::WaitingInput,
        owner: "alice".to_string(),
        one_line_status: String::new(),
        pending_inputs: vec![
            PendingInput::Msg {
                record_id: "rec-1".to_string(),
                from: "alice".to_string(),
                from_did: Some("did:dev:alice".to_string()),
                from_name: Some("Alice".to_string()),
                tunnel_did: Some("did:dev:tunnel".to_string()),
                text: "hi".to_string(),
                ai_message: AiMessage::text(AiRole::User, "hi"),
            },
            PendingInput::Event {
                event_id: "/timer/wake".to_string(),
                data: serde_json::json!({"tick": 7}),
            },
        ],
        peer_did: Some("did:dev:alice".to_string()),
        peer_tunnel_did: Some("did:dev:tunnel".to_string()),
        event_subscriptions: vec![EventSubscription {
            pattern: "/timer/**".to_string(),
            subscribed_at_ms: 0,
            message_template: None,
        }],
        workspace_id: Some("ws-1".to_string()),
        pending_task_calls: vec![PendingTaskCall {
            call_id: "call-1".to_string(),
            tool_name: "download".to_string(),
            task_id: 42,
            event_pattern: "/task_mgr/42".to_string(),
        }],
        title: "design review".to_string(),
        objective: "draft the rollout plan".to_string(),
        bootstrap_done: true,
        process_entry: "planner".to_string(),
        process_stack: vec![ProcessFrame {
            entry: "ui_default".to_string(),
            current: "ui_default".to_string(),
        }],
    };
    let json = serde_json::to_string(&meta).unwrap();
    let restored: SessionMeta = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.pending_inputs.len(), 2);
    match &restored.pending_inputs[0] {
        PendingInput::Msg {
            record_id,
            text,
            from_did,
            from_name,
            tunnel_did,
            ..
        } => {
            assert_eq!(record_id, "rec-1");
            assert_eq!(text, "hi");
            assert_eq!(from_did.as_deref(), Some("did:dev:alice"));
            assert_eq!(from_name.as_deref(), Some("Alice"));
            assert_eq!(tunnel_did.as_deref(), Some("did:dev:tunnel"));
        }
        _ => panic!("expected Msg variant first"),
    }
    match &restored.pending_inputs[1] {
        PendingInput::Event { event_id, data } => {
            assert_eq!(event_id, "/timer/wake");
            assert_eq!(data.get("tick").and_then(|v| v.as_i64()), Some(7));
        }
        _ => panic!("expected Event variant second"),
    }
    assert_eq!(restored.peer_did.as_deref(), Some("did:dev:alice"));
    assert_eq!(restored.event_subscriptions.len(), 1);
    assert_eq!(restored.event_subscriptions[0].pattern, "/timer/**");
    assert_eq!(restored.workspace_id.as_deref(), Some("ws-1"));
    assert_eq!(restored.pending_task_calls.len(), 1);
    assert_eq!(restored.pending_task_calls[0].task_id, 42);
    assert_eq!(restored.pending_task_calls[0].call_id, "call-1");
    assert_eq!(restored.title, "design review");
    assert_eq!(restored.objective, "draft the rollout plan");
    assert!(restored.bootstrap_done);
    assert_eq!(restored.process_entry, "planner");
    assert_eq!(restored.process_stack.len(), 1);
    assert_eq!(restored.process_stack[0].entry, "ui_default");
    assert_eq!(restored.process_stack[0].current, "ui_default");
}

#[test]
fn session_meta_backfills_process_entry_for_legacy_json() {
    // Older `.meta/session.json` files predate the
    // `process_entry` / `process_stack` fields. They must still
    // deserialize (serde defaults) and `AgentSession::new`'s restore
    // path backfills `process_entry` from `current_behavior` so the
    // independent-mode snapshot path is well-formed.
    let legacy = serde_json::json!({
        "session_id": "s2",
        "kind": "ui",
        "current_behavior": "ui_default",
        "status": "idle",
    });
    let restored: SessionMeta = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.process_entry, "");
    assert!(restored.process_stack.is_empty());
    // (The backfill itself lives in AgentSession::new and is exercised
    // by the restore-path integration tests; here we only assert that
    // the legacy JSON does NOT fail to deserialize.)
}

#[test]
fn observation_from_task_event_translates_completed() {
    let payload = serde_json::json!({
        "to_status": "Completed",
        "data": {"result": "ok"},
    });
    let obs = observation_from_task_event("call-9", &payload).expect("terminal observation");
    match obs {
        Observation::Success {
            call_id, content, ..
        } => {
            assert_eq!(call_id, "call-9");
            assert_eq!(content.get("result").and_then(|v| v.as_str()), Some("ok"));
        }
        _ => panic!("expected Success"),
    }
}

#[test]
fn observation_from_task_event_translates_failed() {
    let payload = serde_json::json!({
        "to_status": "Failed",
        "message": "network unreachable",
    });
    let obs = observation_from_task_event("call-9", &payload).expect("terminal observation");
    match obs {
        Observation::Error { call_id, message } => {
            assert_eq!(call_id, "call-9");
            assert!(message.contains("network"));
        }
        _ => panic!("expected Error"),
    }
}

#[test]
fn observation_from_task_event_ignores_non_terminal_status() {
    // Running / Progress events shouldn't move the session — they emit
    // frequently and the session must wait for the terminal one.
    let payload = serde_json::json!({"to_status": "Running"});
    assert!(observation_from_task_event("c", &payload).is_none());
}

#[test]
fn compress_messages_preserves_short_history_verbatim() {
    // Under the keep-tail threshold ⇒ no compression, output == input.
    let msgs = vec![
        AiMessage::text(AiRole::System, "sys"),
        AiMessage::text(AiRole::User, "u1"),
        AiMessage::text(AiRole::Assistant, "a1"),
    ];
    let out = compress_messages_for_context_limit(msgs.clone());
    assert_eq!(out.len(), msgs.len());
    assert_eq!(out[0].role, AiRole::System);
}

#[test]
fn compress_messages_drops_middle_and_keeps_tail() {
    let mut msgs = vec![AiMessage::text(AiRole::System, "sys")];
    // Generate alternating user/assistant pairs well beyond the tail cap.
    for i in 0..(COMPRESS_KEEP_TAIL + 20) {
        let role = if i % 2 == 0 {
            AiRole::User
        } else {
            AiRole::Assistant
        };
        msgs.push(AiMessage::text(role, format!("m-{i}")));
    }
    let out = compress_messages_for_context_limit(msgs);
    assert_eq!(out[0].role, AiRole::System);
    // Second message is the synthetic compression note.
    assert_eq!(out[1].role, AiRole::User);
    let note = out[1]
        .content
        .iter()
        .find_map(|b| match b {
            AiContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    assert!(note.contains("context compressed"));
    assert!(note.contains("earlier"));
    // Tail length is at most the keep cap (may be one less when we
    // realign past a leading Assistant).
    let tail_len = out.len() - 2;
    assert!(tail_len <= COMPRESS_KEEP_TAIL);
    assert!(tail_len >= COMPRESS_KEEP_TAIL - 1);
    // No two assistant messages in a row (our realignment guarantee).
    for w in out.windows(2) {
        assert!(
            !(w[0].role == AiRole::Assistant && w[1].role == AiRole::Assistant),
            "compress must not produce back-to-back assistant messages"
        );
    }
}

#[test]
fn merge_env_and_human_combines_both_with_env_first() {
    let m = merge_env_and_human(Some("E".into()), Some("H".into()));
    assert_eq!(m.as_deref(), Some("E\n\nH"));
}

#[test]
fn merge_env_and_human_handles_missing_pieces() {
    assert_eq!(
        merge_env_and_human(None, Some("h".into())).as_deref(),
        Some("h")
    );
    assert_eq!(
        merge_env_and_human(Some("e".into()), None).as_deref(),
        Some("e")
    );
    assert!(merge_env_and_human(None, None).is_none());
}

#[test]
fn session_meta_tolerates_missing_pending_inputs_field() {
    // Older session.json files were written before pending_inputs
    // existed; restoring them must default the field to an empty
    // vec rather than erroring out.
    let legacy = r#"{
        "session_id": "old",
        "kind": "ui",
        "current_behavior": "ui_default",
        "status": "idle",
        "owner": "alice"
    }"#;
    let meta: SessionMeta = serde_json::from_str(legacy).unwrap();
    assert!(meta.pending_inputs.is_empty());
    assert_eq!(meta.owner, "alice");
}
