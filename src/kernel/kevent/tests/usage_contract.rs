use buckyos_api::{match_event_patterns, validate_eventid, validate_pattern, KEventClient};
use serde_json::json;

fn assert_patterns_match(patterns: &[String], eventid: &str) {
    validate_eventid(eventid).unwrap();
    for pattern in patterns {
        validate_pattern(pattern).unwrap();
    }
    assert!(
        match_event_patterns(patterns, eventid),
        "patterns {:?} should match eventid {}",
        patterns,
        eventid
    );
}

fn assert_patterns_do_not_match(patterns: &[String], eventid: &str) {
    validate_eventid(eventid).unwrap();
    for pattern in patterns {
        validate_pattern(pattern).unwrap();
    }
    assert!(
        !match_event_patterns(patterns, eventid),
        "patterns {:?} should not match eventid {}",
        patterns,
        eventid
    );
}

#[test]
fn msg_center_box_events_match_current_consumers() {
    let owner = "alice";
    let control_panel_patterns = vec![
        format!("/msg_center/{owner}/box/in/**"),
        format!("/msg_center/{owner}/box/out/**"),
    ];
    let opendan_patterns = [
        "in",
        "inbox",
        "group_in",
        "group_inbox",
        "request",
        "request_box",
    ]
    .into_iter()
    .flat_map(|box_name| {
        [
            format!("/msg_center/{owner}/box/{box_name}/**"),
            format!("/msg_center/{owner}/{box_name}/**"),
        ]
    })
    .collect::<Vec<_>>();

    assert_patterns_match(&control_panel_patterns, "/msg_center/alice/box/in/changed");
    assert_patterns_match(&control_panel_patterns, "/msg_center/alice/box/out/changed");
    assert_patterns_do_not_match(
        &control_panel_patterns,
        "/msg_center/alice/box/group_in/changed",
    );

    for eventid in [
        "/msg_center/alice/box/in/changed",
        "/msg_center/alice/box/inbox/changed",
        "/msg_center/alice/box/group_in/changed",
        "/msg_center/alice/box/group_inbox/changed",
        "/msg_center/alice/box/request/changed",
        "/msg_center/alice/box/request_box/changed",
        "/msg_center/alice/in/changed",
        "/msg_center/alice/group_in/changed",
        "/msg_center/alice/request/changed",
    ] {
        assert_patterns_match(&opendan_patterns, eventid);
    }
}

#[test]
fn task_manager_wait_pattern_matches_task_events_only() {
    let task_id = 42;
    let wait_patterns = vec![format!("/task_mgr/{task_id}")];

    assert_patterns_match(&wait_patterns, "/task_mgr/42");
    assert_patterns_do_not_match(&wait_patterns, "/task_mgr/43");
    assert_patterns_do_not_match(&wait_patterns, "/task_mgr/42/done");
}

#[tokio::test]
async fn dynamic_readers_fanout_unsubscribe_and_rebuild() {
    let client = KEventClient::new_local("usage_contract");
    let reader_a = client
        .create_event_reader(vec![
            "session_event".to_string(),
            "session_event".to_string(),
        ])
        .await
        .unwrap();
    let reader_b = client
        .create_event_reader(vec!["other_event".to_string()])
        .await
        .unwrap();
    reader_b
        .add_patterns(vec!["session_event".to_string()])
        .await
        .unwrap();

    client
        .pub_event("session_event", json!({ "seq": 1 }))
        .await
        .unwrap();
    assert_eq!(
        reader_a.pull_event(Some(0)).await.unwrap().unwrap().data["seq"],
        json!(1)
    );
    assert_eq!(
        reader_b.pull_event(Some(0)).await.unwrap().unwrap().data["seq"],
        json!(1)
    );
    assert!(reader_a.pull_event(Some(0)).await.unwrap().is_none());
    assert!(reader_b.pull_event(Some(0)).await.unwrap().is_none());

    reader_b
        .remove_patterns(vec!["session_event".to_string()])
        .await
        .unwrap();
    client
        .pub_event("session_event", json!({ "seq": 2 }))
        .await
        .unwrap();
    assert_eq!(
        reader_a.pull_event(Some(0)).await.unwrap().unwrap().data["seq"],
        json!(2)
    );
    assert!(reader_b.pull_event(Some(0)).await.unwrap().is_none());

    reader_a.close().await.unwrap();
    client
        .pub_event("session_event", json!({ "seq": 3 }))
        .await
        .unwrap();
    let reader_c = client
        .create_event_reader(vec!["session_event".to_string()])
        .await
        .unwrap();
    assert!(reader_c.pull_event(Some(0)).await.unwrap().is_none());

    client
        .pub_event("session_event", json!({ "seq": 4 }))
        .await
        .unwrap();
    assert_eq!(
        reader_c.pull_event(Some(0)).await.unwrap().unwrap().data["seq"],
        json!(4)
    );
}
