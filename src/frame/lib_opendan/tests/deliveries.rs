//! The delivery index (Agent UI upgrade TODO, AR1): a report's artifact and
//! a reply's attachment are both found, each under the Session that
//! delivered it and the conversation it belongs to.

mod common;

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use libopendan::deliveries::{recent, DeliverySource};
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, ObjectStore, StopWhen, StoredObject};
use libopendan::SessionTemplate;
use serde_json::json;

const ROUTE: &str = "did:bns:agentx/dm%3Adid%3Abns%3Abob";

/// Keeps every file under an id made of its first byte.
struct Objects;

#[async_trait]
impl ObjectStore for Objects {
    async fn put_file(&self, path: &Path, name: &str) -> Result<StoredObject, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        Ok(StoredObject {
            obj_id: ndn_lib::ObjId::new_by_raw("cyfile".into(), vec![bytes[0]; 32]),
            url: Some(format!("obj://bucket/user/agentx/bob/artifacts/{:02x}/{name}", bytes[0])),
        })
    }
}

fn ui_spec() -> libopendan::api::SessionSpec {
    let template = SessionTemplate::load("ui", None).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.route_key = Some(ROUTE.into());
    spec.owner = Some("did:bns:bob".into());
    spec.outbound = Some(OutboundBinding {
        to: "did:bns:bob".into(),
        to_session: None,
        kind: "chat".into(),
    });
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [{ "groupname": "bash_core" }] },
        "json_schema": {
            "type": "object", "additionalProperties": false,
            "required": ["reply", "topic", "artifacts"],
            "properties": {
                "reply": { "type": "string" },
                "topic": { "type": ["object", "null"] },
                "artifacts": { "type": "array", "items": { "type": "string" } }
            }
        }
    });
    spec
}

#[tokio::test]
async fn reports_and_replies_are_indexed_under_their_session_and_conversation() {
    let env = Env::new();
    let root = env.agent_root.clone();
    assert!(recent(&root, 10, None, None).unwrap().entries.is_empty());

    // A reply of the conversation carries a file.
    let ui = env.create_work(ui_spec()).await;
    std::fs::write(ui.path().join("chart.png"), b"Achart").unwrap();
    post_blocking(&env.queue_dir, &queue_of(&ui), msg("draw it"));
    let llm = ScriptedLlm::new(|_, _| text(r#"{"reply":"here it is","topic":null,"artifacts":["chart.png"]}"#));
    let mut deps = env.deps(llm);
    deps.objects = Some(Arc::new(Objects));
    let result = drive(&ui, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");

    // A work the conversation started reports with a file.
    let mut spec = work_spec("write the answer");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true, "tools": [{ "groupname": "bash" }, { "name": "report" }] } });
    spec.origin = Some(Origin { parent_session: Some(ui.sid().to_string()), ..Default::default() });
    let work = env.create_work(spec).await;
    std::fs::write(work.path().join("answer.txt"), b"Banswer").unwrap();
    let llm = ScriptedLlm::new(|_, _| {
        tool_call("c1", "report", json!({ "report": "done", "artifacts": ["answer.txt"], "is_end": true }))
    });
    let mut work_deps = env.deps(llm);
    work_deps.objects = Some(Arc::new(Objects));
    let result = drive(&work, &work_deps, StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");

    let page = recent(&root, 10, None, None).unwrap();
    assert_eq!(page.next_before, None);
    let [report, reply] = page.entries.as_slice() else { panic!("{page:?}") };
    assert_eq!((report.name.as_str(), report.source, report.session_id.as_str()), ("answer.txt", DeliverySource::Report, work.sid()));
    assert_eq!((report.ui_session_id.as_deref(), report.route_key.as_deref()), (Some(ui.sid()), Some(ROUTE)));
    assert_eq!((report.mime.as_deref(), report.size), (Some("text/plain"), Some(7)));
    assert!(report.url.as_deref().is_some_and(|u| u.ends_with("/answer.txt")), "{report:?}");
    assert!(report.obj_id.starts_with("cyfile:"), "{report:?}");
    assert_eq!((reply.name.as_str(), reply.source, reply.session_id.as_str()), ("chart.png", DeliverySource::Reply, ui.sid()));
    assert_eq!((reply.ui_session_id.as_deref(), reply.route_key.as_deref()), (Some(ui.sid()), Some(ROUTE)));
    assert_eq!((reply.mime.as_deref(), reply.size), (Some("image/png"), Some(6)));
    assert!(report.at_ms >= reply.at_ms);

    // By conversation: both; another conversation: none.
    assert_eq!(recent(&root, 10, None, Some(ROUTE)).unwrap().entries.len(), 2);
    assert!(recent(&root, 10, None, Some("did:bns:agentx/dm%3Aother")).unwrap().entries.is_empty());

    // Driving again delivers nothing new.
    assert!(drive(&work, &work_deps, StopWhen::Idle).await.is_finished());
    assert_eq!(recent(&root, 10, None, None).unwrap().entries.len(), 2);

    // A host without an object store writes no line.
    let mut spec = work_spec("no store");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true, "tools": [{ "groupname": "bash" }, { "name": "report" }] } });
    let plain = env.create_work(spec).await;
    std::fs::write(plain.path().join("answer.txt"), b"Canswer").unwrap();
    let llm = ScriptedLlm::new(|_, _| {
        tool_call("c1", "report", json!({ "report": "done", "artifacts": ["answer.txt"], "is_end": true }))
    });
    assert!(drive(&plain, &env.deps(llm), StopWhen::Finished).await.is_finished());
    assert_eq!(recent(&root, 10, None, None).unwrap().entries.len(), 2);
}
