//! The reply path: a Turn's reply is committed with the Turn, handed to the
//! outbound sink afterwards and re-sent unchanged until the sink answers.

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::*;
use libopendan::bridge::OutboundRecord;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, OutboundSink, SendResult, StopWhen};
use libopendan::{AgentStateClient, SessionTemplate};
use serde_json::json;

const BOB: &str = "did:bns:bob";

#[derive(Default)]
struct Sink {
    /// Sends that fail before the sink starts answering.
    fail: Mutex<u32>,
    reject: bool,
    sent: Mutex<Vec<OutboundRecord>>,
}

#[async_trait]
impl OutboundSink for Sink {
    async fn send(&self, _sid: &str, record: &OutboundRecord) -> SendResult {
        self.sent.lock().unwrap().push(record.clone());
        let mut fail = self.fail.lock().unwrap();
        if *fail > 0 {
            *fail -= 1;
            return SendResult::Retry {
                error: "msg-center unreachable".into(),
            };
        }
        if self.reject {
            return SendResult::Rejected {
                reason: "blocked_author".into(),
            };
        }
        SendResult::Sent {
            msg_id: Some(msg_key(&record.msg)),
            deliveries: vec!["d-1".into()],
        }
    }
}

fn ui_spec(binding: Option<OutboundBinding>) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.route_key = Some(format!("{AGENT}/dm%3A{BOB}"));
    spec.outbound = binding;
    spec
}

fn from_bob(t: &str) -> PostedInput {
    let m = text_msg(&parse_did(BOB).unwrap(), &parse_did(AGENT).unwrap(), t);
    PostedInput::msg(APP, m, MsgDelivery::default()).unwrap()
}

fn bob_binding() -> OutboundBinding {
    OutboundBinding {
        to: BOB.into(),
        to_session: None,
        kind: "chat".into(),
    }
}

#[tokio::test]
async fn a_reply_is_committed_then_sent_along_the_way_the_message_came() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(Some(bob_binding()))).await;
    assert_eq!(sd.config().unwrap().session.kind, SessionKind::Ui);
    let agent = env.agent();
    let entry = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
    assert_eq!(entry.route_key, sd.config().unwrap().session.route_key);
    let input = from_bob("hello");
    libopendan::post_input(agent.as_ref(), sd.sid(), &input).await.unwrap();
    let sink = Arc::new(Sink::default());
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("hi bob")));
    deps.outbound = Some(sink.clone());
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 1, .. }), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.outbox.len(), 1);
    let e = &st.outbox[0];
    assert_eq!(e.status, OutboxStatus::Sent);
    assert_eq!(e.deliveries, vec!["d-1".to_string()]);
    assert_eq!(e.msg.content.content, "hi bob");
    assert_eq!(e.msg.to, vec![parse_did(BOB).unwrap()]);
    assert_eq!(e.msg.from, parse_did(AGENT).unwrap());
    assert_eq!(e.msg.thread.reply_to.as_ref().unwrap().to_string(), input.key);
    assert!(e.key.starts_with(&format!("{}:1:", sd.sid())), "{}", e.key);
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
    // Nothing is sent twice.
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_unreachable_sink_gets_the_same_message_again() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(Some(bob_binding()))).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("hello")).await.unwrap();
    let sink = Arc::new(Sink {
        fail: Mutex::new(1),
        ..Default::default()
    });
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("hi bob")));
    deps.outbound = Some(sink.clone());
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    // A failed send does not fail the Turn.
    assert!(
        matches!(r, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }),
        "{r:?}"
    );
    let st = sd.state().unwrap();
    assert_eq!(st.outbox[0].status, OutboxStatus::Pending);
    assert_eq!(st.outbox[0].attempts, 1);
    assert!(libopendan::runner::has_pending_outbound(&st));
    // A later drive (another process after a restart, as far as the session
    // can tell) hands over the stored message, not a new one.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    let mut deps = env.deps(ScriptedLlm::new(|_, _| panic!("no inference")));
    deps.outbound = Some(sink.clone());
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.outbox[0].status, OutboxStatus::Sent);
    let sent = sink.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], sent[1], "same key, same MsgObject (same ObjId)");
}

#[tokio::test]
async fn rejected_and_misrouted_replies_are_recorded_not_retried() {
    let env = Env::new();
    // Bound to another peer than the one the message came from.
    let mut binding = bob_binding();
    binding.to = "did:bns:carol".into();
    let sd = env.create_work(ui_spec(Some(binding))).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("hello")).await.unwrap();
    let sink = Arc::new(Sink::default());
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("hi")));
    deps.outbound = Some(sink.clone());
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let st = sd.state().unwrap();
    assert_eq!(st.outbox[0].status, OutboxStatus::Failed);
    assert!(st.outbox[0].error.as_deref().unwrap().starts_with("route_mismatch"));
    assert!(sink.sent.lock().unwrap().is_empty());

    let env = Env::new();
    let sd = env.create_work(ui_spec(None)).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("hello")).await.unwrap();
    let sink = Arc::new(Sink {
        reject: true,
        ..Default::default()
    });
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("hi")));
    deps.outbound = Some(sink.clone());
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    drive(&sd, &deps, StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert_eq!(st.outbox[0].status, OutboxStatus::Failed);
    assert_eq!(st.outbox[0].error.as_deref(), Some("blocked_author"));
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn without_a_sink_no_outbox_is_kept() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(None)).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("hello")).await.unwrap();
    let deps = env.deps(ScriptedLlm::new(|_, _| text("hi")));
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(sd.state().unwrap().outbox.is_empty());
}

/// Puts every file in as `cyfile:<sha256(name)>`, remembering what it got.
#[derive(Default)]
struct Objects {
    put: Mutex<Vec<(std::path::PathBuf, String)>>,
}

impl Objects {
    fn id_of(name: &str) -> ndn_lib::ObjId {
        use sha2::Digest;
        ndn_lib::ObjId::new_by_raw("cyfile".into(), sha2::Sha256::digest(name).to_vec())
    }
}

#[async_trait]
impl libopendan::runner::ObjectStore for Objects {
    async fn put_file(&self, path: &std::path::Path, name: &str) -> Result<libopendan::runner::StoredObject, String> {
        assert!(path.is_file(), "{}", path.display());
        self.put.lock().unwrap().push((path.to_path_buf(), name.to_string()));
        Ok(libopendan::runner::StoredObject { obj_id: Self::id_of(name), url: None })
    }

    async fn name_of(&self, obj_id: &ndn_lib::ObjId) -> Option<String> {
        (*obj_id == Self::id_of("from a work session"))
            .then(|| "game.zip".to_string())
            .or_else(|| {
                self.put.lock().unwrap().iter()
                    .find(|(_, name)| *obj_id == Self::id_of(name))
                    .map(|(_, name)| name.clone())
            })
    }
}

fn report_ui_spec() -> libopendan::api::SessionSpec {
    let mut spec = ui_spec(Some(bob_binding()));
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [{ "groupname": "bash" }, { "name": "report" }] }
    });
    spec
}

fn data_refs(msg: &ndn_lib::MsgObject) -> Vec<(String, Option<String>)> {
    msg.content
        .refs
        .iter()
        .map(|r| match &r.target {
            ndn_lib::RefTarget::DataObj { obj_id, .. } => (obj_id.to_string(), r.label.clone()),
            other => panic!("not a data object: {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn a_failed_artifact_upload_is_correctable_before_delivery() {
    #[derive(Default)]
    struct UnavailableOnce(Mutex<usize>);
    #[async_trait]
    impl libopendan::runner::ObjectStore for UnavailableOnce {
        async fn put_file(&self, path: &std::path::Path, name: &str) -> Result<libopendan::runner::StoredObject, String> {
            assert_eq!(std::fs::read(path).unwrap(), b"video content");
            let mut attempts = self.0.lock().unwrap();
            *attempts += 1;
            if *attempts == 1 {
                return Err("object store temporarily unavailable".into());
            }
            Ok(libopendan::runner::StoredObject { obj_id: Objects::id_of(name), url: None })
        }
    }
    let env = Env::new();
    let sd = env.create_work(report_ui_spec()).await;
    std::fs::write(sd.path().join("video.mp4"), b"video content").unwrap();
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => final_report("first", "video attached", json!(["video.mp4"])),
        1 => {
            assert!(has_tool_result(req, "first").unwrap().contains("could not be attached"));
            final_report("retry", "video attached", json!(["video.mp4"]))
        }
        _ => panic!("unexpected inference {n}"),
    });
    let sink = Arc::new(Sink::default());
    let objects = Arc::new(UnavailableOnce::default());
    let mut deps = env.deps(llm);
    deps.objects = Some(objects.clone());
    deps.outbound = Some(sink.clone());
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("video?")).await.unwrap();
    let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    assert_eq!(*objects.0.lock().unwrap(), 2);
    let sent = sink.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].msg.content.content, "video attached");
    assert_eq!(data_refs(&sent[0].msg), vec![(Objects::id_of("video.mp4").to_string(), Some("video.mp4".into()))]);
    assert_eq!(sd.state().unwrap().final_report.unwrap().source, ReportSource::Tool { call_id: "retry".into() });
}

#[tokio::test]
async fn a_final_report_is_the_reply_with_its_artifacts_as_objects() {
    let env = Env::new();
    let sd = env.create_work(report_ui_spec()).await;
    std::fs::write(sd.path().join("star-link.html"), "<html></html>").unwrap();
    let forwarded = Objects::id_of("from a work session").to_string();
    let agent = env.agent();
    let given = forwarded.clone();
    let llm = ScriptedLlm::new(move |_, n| match n {
        0 => final_report("r1", "here is the game", json!(["star-link.html", given])),
        1 => final_report("r2", "nothing to attach", json!([])),
        _ => panic!("unexpected inference {n}"),
    });
    let sink = Arc::new(Sink::default());
    let objects = Arc::new(Objects::default());
    let mut deps = env.deps(llm);
    deps.outbound = Some(sink.clone());
    deps.objects = Some(objects.clone());
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("make a game")).await.unwrap();
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 1, status: TurnStatus::Completed, .. }), "{r:?}");

    // The file is put in once, from its stable copy, under its own name.
    let report = sd.state().unwrap().final_report.unwrap();
    let put = objects.put.lock().unwrap().clone();
    assert_eq!(put, vec![(sd.path().join(&report.artifacts[0].reference), "star-link.html".to_string())]);
    let stored = Objects::id_of("star-link.html").to_string();
    assert_eq!(report.artifacts[0].obj_id.as_deref(), Some(stored.as_str()));
    assert_eq!(report.artifacts[1].obj_id.as_deref(), Some(forwarded.as_str()));
    assert!(report.artifacts[1].reference.is_empty());
    assert!(report.delivery_text().contains(&format!("[star-link.html]({stored})")));

    // The reply is the report's text; the artifacts are its attachments.
    let sent = sink.sent.lock().unwrap()[0].msg.clone();
    assert_eq!(sent.content.content, "here is the game");
    assert_eq!(
        data_refs(&sent),
        vec![(stored, Some("star-link.html".to_string())), (forwarded, Some("game.zip".to_string()))]
    );

    // Every Turn of the dialogue may end with a final report of its own.
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("thanks")).await.unwrap();
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 2, status: TurnStatus::Completed, .. }), "{r:?}");
    let sent = sink.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].msg.content.content, "nothing to attach");
    assert!(sent[1].msg.content.refs.is_empty());
}

/// A host that keeps everything in a bucket: files are put in, bucket
/// objects bound elsewhere are copied into `artifacts`.
#[derive(Default)]
struct Bucketed {
    put: Mutex<Vec<String>>,
    retained: Mutex<Vec<String>>,
}

const ARTIFACTS: &str = "obj://bucket/user/agentx/alice/artifacts";

#[async_trait]
impl libopendan::runner::ObjectStore for Bucketed {
    async fn put_file(&self, path: &std::path::Path, name: &str) -> Result<libopendan::runner::StoredObject, String> {
        assert!(path.is_file(), "{}", path.display());
        self.put.lock().unwrap().push(name.to_string());
        Ok(libopendan::runner::StoredObject { obj_id: Objects::id_of(name), url: Some(format!("{ARTIFACTS}/0001/{name}")) })
    }

    async fn retain_url(&self, url: &str) -> Result<libopendan::runner::StoredObject, String> {
        self.retained.lock().unwrap().push(url.to_string());
        let name = url.rsplit('/').next().unwrap();
        Ok(libopendan::runner::StoredObject { obj_id: Objects::id_of(name), url: Some(format!("{ARTIFACTS}/0002/{name}")) })
    }
}

fn uri_hints(msg: &ndn_lib::MsgObject) -> Vec<Option<String>> {
    msg.content
        .refs
        .iter()
        .map(|r| match &r.target {
            ndn_lib::RefTarget::DataObj { uri_hint, .. } => uri_hint.clone(),
            other => panic!("not a data object: {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn bucket_bindings_travel_with_the_reply_and_retained_copies_are_dropped() {
    let env = Env::new();
    let sd = env.create_work(report_ui_spec()).await;
    std::fs::write(sd.path().join("chart.png"), "png").unwrap();
    let generated = "obj://bucket/user/aicc/alice/outputs/agentx/0123/image_1.png";
    let agent = env.agent();
    let llm = ScriptedLlm::new(move |_, n| match n {
        0 => final_report("r1", "chart and picture", json!(["chart.png", generated])),
        1 => text(&format!("the picture again\n<attachment path=\"{generated}\" />")),
        _ => panic!("unexpected inference {n}"),
    });
    let sink = Arc::new(Sink::default());
    let objects = Arc::new(Bucketed::default());
    let mut deps = env.deps(llm);
    deps.outbound = Some(sink.clone());
    deps.objects = Some(objects.clone());
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("chart?")).await.unwrap();
    drive(&sd, &deps, StopWhen::TurnClosed).await;

    let report = sd.state().unwrap().final_report.unwrap();
    assert_eq!(report.artifacts[0].url.as_deref(), Some(format!("{ARTIFACTS}/0001/chart.png").as_str()));
    assert!(!sd.path().join(&report.artifacts[0].reference).exists(), "the bucket keeps the file");
    assert_eq!(report.artifacts[1].url.as_deref(), Some(format!("{ARTIFACTS}/0002/image_1.png").as_str()));
    assert_eq!(report.artifacts[1].name(), "image_1.png");
    let sent = sink.sent.lock().unwrap()[0].msg.clone();
    assert_eq!(
        data_refs(&sent),
        vec![
            (Objects::id_of("chart.png").to_string(), Some("chart.png".to_string())),
            (Objects::id_of("image_1.png").to_string(), Some("image_1.png".to_string())),
        ]
    );
    assert_eq!(
        uri_hints(&sent),
        vec![Some(format!("{ARTIFACTS}/0001/chart.png")), Some(format!("{ARTIFACTS}/0002/image_1.png"))]
    );

    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("once more")).await.unwrap();
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let sent = sink.sent.lock().unwrap()[1].msg.clone();
    assert_eq!(uri_hints(&sent), vec![Some(format!("{ARTIFACTS}/0002/image_1.png"))]);
    assert_eq!(*objects.retained.lock().unwrap(), vec![generated.to_string(), generated.to_string()]);
    assert_eq!(*objects.put.lock().unwrap(), vec!["chart.png".to_string()]);
}

#[tokio::test]
async fn without_an_object_store_artifacts_are_named_and_attached_paths_rejected() {
    let env = Env::new();
    let sd = env.create_work(report_ui_spec()).await;
    std::fs::write(sd.path().join("notes.txt"), "notes").unwrap();
    let agent = env.agent();
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => final_report("r1", "see the notes", json!(["notes.txt"])),
        1 => text("again\n<attachment path=\"notes.txt\" />"),
        _ => panic!("unexpected inference {n}"),
    });
    let sink = Arc::new(Sink::default());
    let mut deps = env.deps(llm);
    deps.outbound = Some(sink.clone());
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("notes?")).await.unwrap();
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("once more")).await.unwrap();
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let sent = sink.sent.lock().unwrap();
    assert_eq!(sent[0].msg.content.content, "see the notes\n\n[notes.txt] (not attached)");
    assert!(sent[0].msg.content.refs.is_empty());
    assert!(sent[1].msg.content.content.contains("no object store"), "{}", sent[1].msg.content.content);
    assert!(sent[1].msg.content.refs.is_empty());
}

#[tokio::test]
async fn attached_local_files_of_an_answer_become_objects() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(Some(bob_binding()))).await;
    std::fs::write(sd.path().join("chart.png"), "png").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let report = elsewhere.path().join("report.pdf");
    std::fs::write(&report, "pdf").unwrap();
    let answer = format!(
        "both files\n<attachment path=\"chart.png\" />\n<attachment path=\"{}\" />",
        report.display()
    );
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("files?")).await.unwrap();
    let sink = Arc::new(Sink::default());
    let objects = Arc::new(Objects::default());
    let mut deps = env.deps(ScriptedLlm::new(move |_, _| text(&answer)));
    deps.outbound = Some(sink.clone());
    deps.objects = Some(objects.clone());
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let sent = sink.sent.lock().unwrap()[0].msg.clone();
    assert_eq!(sent.content.content, "both files");
    let ids: Vec<String> = data_refs(&sent).into_iter().map(|(id, _)| id).collect();
    assert_eq!(
        ids,
        vec![Objects::id_of("chart.png").to_string(), Objects::id_of("report.pdf").to_string()]
    );
    let put = objects.put.lock().unwrap();
    assert_eq!(put[0].0, sd.path().canonicalize().unwrap().join("chart.png"));
    assert_eq!(put[1].0, report);
}

fn final_report(call_id: &str, report: &str, artifacts: serde_json::Value) -> buckyos_api::AiResponse {
    tool_call(
        call_id,
        "report",
        json!({ "report": report, "artifacts": artifacts, "is_end": true }),
    )
}

#[tokio::test]
async fn a_work_report_object_is_forwarded_without_accessing_the_workers_files() {
    let env = Env::new();
    let mut spec = SessionTemplate::load("work", None).unwrap().spec("produce a video");
    spec.prompt.llm_context = report_ui_spec().prompt.llm_context;
    let work = env.create_work(spec).await;
    let source = work.path().join("cat_halloween.mp4");
    let bytes = b"\0\0\0\x18ftypmp42\0\xff\x80binary video artifact";
    std::fs::write(&source, bytes).unwrap();
    let objects = Arc::new(Objects::default());
    let mut deps = env.deps(ScriptedLlm::new(|_, n| {
        assert_eq!(n, 0);
        final_report("video", "video ready", json!(["cat_halloween.mp4"]))
    }));
    deps.objects = Some(objects.clone());
    let result = drive(&work, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    let report = work.state().unwrap().final_report.unwrap();
    let artifact = &report.artifacts[0];
    let object_id = artifact.obj_id.clone().unwrap();
    assert_eq!(std::fs::read(work.path().join(&artifact.reference)).unwrap(), bytes);
    assert!(report.delivery_text().contains(&object_id));
    std::fs::remove_file(source).unwrap();

    let ui = env.create_work(report_ui_spec()).await;
    assert!(!ui.path().join("cat_halloween.mp4").exists());
    let forwarded = object_id.clone();
    let mut deps = env.deps(ScriptedLlm::new(move |_, n| {
        assert_eq!(n, 0);
        final_report("relay", "here is your video", json!([forwarded]))
    }));
    let sink = Arc::new(Sink::default());
    deps.objects = Some(objects.clone());
    deps.outbound = Some(sink.clone());
    libopendan::post_input(env.agent().as_ref(), ui.sid(), &from_bob("show the video")).await.unwrap();
    let result = drive(&ui, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    let sent = sink.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].msg.content.content, "here is your video");
    assert_eq!(data_refs(&sent[0].msg), vec![(object_id, Some("cat_halloween.mp4".into()))]);
    assert_eq!(objects.put.lock().unwrap().len(), 1);
}
