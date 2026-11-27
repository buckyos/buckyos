//! DID Object Provider of one Agent (notepads/done/agent-object-url-todo.md §4.1, D5):
//! `obj://<agent-id>`, `/sessions[/<sid>]`, `/workspaces[/<ws-id>]`, `/goals[/<gid>]`,
//! `/artifacts[/<aid>]`, served at `/kapi/<agent_id>/obj/...` (the loader's
//! `/kapi/opendan/obj` and `/api/opendan/obj` reach the same objects). Every read
//! is the Agent State kRPC method with the caller's token, so owner / root /
//! session-token rules stay those of `StateService`.

use std::sync::{Arc, Weak};

use buckyos_api::{obj_service_url, Captures, KrpcObjectProvider, ObjectKind, RpcBinding, RpcHandlerBackend};
use kRPC::RPCHandler;
use name_lib::DID;
use serde_json::{json, Value};

fn cap(captures: &Captures, name: &str) -> String {
    captures.get(name).cloned().unwrap_or_default()
}

/// The fields of an item a collection's rows show, as `(cell, JSON pointer)`:
/// a list names each item in one line; reading the item gives the rest.
type Cells = &'static [(&'static str, &'static str)];

fn rows(agent: &str, collection: &str, id_field: &str, cells: Cells, items: Value) -> Value {
    let list = match items {
        Value::Array(list) => list,
        Value::Object(mut map) => match map.remove("items").or_else(|| map.remove("entries")) {
            Some(Value::Array(list)) => list,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    let rows: Vec<Value> = list
        .into_iter()
        .filter_map(|item| {
            let id = item.get(id_field)?.as_str()?.to_string();
            let cells: serde_json::Map<String, Value> = cells
                .iter()
                .filter_map(|(cell, pointer)| Some((cell.to_string(), item.pointer(pointer).filter(|v| !v.is_null())?.clone())))
                .collect();
            Some(json!({ "row_id": id, "object": obj_service_url(agent, &[collection, &id]), "cells": cells }))
        })
        .collect();
    json!({ "rows": rows })
}

fn collection(agent: &str, name: &str, title: &str, description: &str, method: &str, params: Value, id_field: &'static str, cells: Cells) -> ObjectKind {
    let (a, b, c) = (agent.to_string(), name.to_string(), params.clone());
    let (a2, b2) = (agent.to_string(), name.to_string());
    ObjectKind::new(name, &format!("opendan.{name}"), title, description)
        .summary_property(
            "items",
            &format!("Every item of {name}"),
            json!({ "type": "array" }),
            RpcBinding::new(method, move |_, _| Ok(c.clone())).map(move |_, v| rows(&a, &b, id_field, cells, v)),
        )
        .index(
            &format!("List {name}"),
            json!({}),
            RpcBinding::new(method, move |_, _| Ok(params.clone())).map(move |_, v| rows(&a2, &b2, id_field, cells, v)),
        )
}

/// `agent` is the agent-id (`obj://<agent-id>` host, the `/kapi/<agent_id>` mount).
pub fn object_kinds(agent: &str) -> Vec<ObjectKind> {
    let object = json!({ "type": "object" });
    let root = ObjectKind::new(
        "",
        "opendan.agent",
        "Agent",
        "The Agent itself. Its sessions, workspaces, goals and artifacts are child objects.",
    )
    .summary_property("info", "Agent identity and runtime", object.clone(), RpcBinding::new("agent.info", |_, _| Ok(json!({}))))
    .property("profile", "Agent profile", object.clone(), RpcBinding::new("agent.profile", |_, _| Ok(json!({}))));
    let session = ObjectKind::new(
        "sessions/{sid}",
        "opendan.session",
        "Agent session",
        "One session of the Agent: its registry entry, report and recent worklog. The session directory is a separate file resource.",
    )
    .summary_property(
        "entry",
        "Registry entry (kind, class, driver, state)",
        object.clone(),
        RpcBinding::new("sessions.lookup", |c, _| Ok(json!({ "sid": cap(c, "sid") }))),
    )
    .property(
        "report",
        "Session view with its report and the last 40 worklog lines",
        object.clone(),
        RpcBinding::new("session.read", |c, _| Ok(json!({ "sid": cap(c, "sid") }))),
    )
    .property(
        "worklog",
        "Worklog of the session",
        object.clone(),
        RpcBinding::new("session.worklog", |c, _| Ok(json!({ "sid": cap(c, "sid") }))),
    )
    .event("changed", "The session's state changed (run state, outcome, one-line status)")
    .event("finished", "The session finished");
    let workspace = ObjectKind::new(
        "workspaces/{workspace_id}",
        "opendan.workspace",
        "Agent workspace",
        "Metadata of one workspace (name, runtime, url, lease). Its directory content is reached by file / cyfs references.",
    )
    .summary_property(
        "record",
        "Workspace record",
        object.clone(),
        RpcBinding::new("workspaces.lookup", |c, _| Ok(json!({ "workspace_id": cap(c, "workspace_id") }))),
    );
    let goal = ObjectKind::new("goals/{goal_id}", "opendan.goal", "Agent goal", "One goal of the Agent and its state.")
        .summary_property(
            "goal",
            "Goal definition",
            object.clone(),
            RpcBinding::new("goals.get", |c, _| Ok(json!({ "goal_id": cap(c, "goal_id") }))),
        )
        .property(
            "state",
            "Goal state",
            object.clone(),
            RpcBinding::new("goals.state", |c, _| Ok(json!({ "goal_id": cap(c, "goal_id") }))),
        );
    let artifact = ObjectKind::new(
        "artifacts/{aid}",
        "opendan.artifact",
        "Agent artifact",
        "One artifact: its head and versions. A version's content is a cyfs:// ObjId reference.",
    )
    .summary_property(
        "head",
        "Current head of the artifact",
        object.clone(),
        RpcBinding::new("artifacts.head", |c, _| Ok(json!({ "aid": cap(c, "aid") }))),
    )
    .property(
        "versions",
        "Every version of the artifact",
        object,
        RpcBinding::new("artifacts.versions", |c, _| Ok(json!({ "aid": cap(c, "aid") }))),
    );
    vec![
        root,
        collection(agent, "sessions", "Agent sessions", "Sessions of the Agent", "sessions.query", json!({}), "session_id",
            &[("kind", "/kind"), ("status", "/status/run_state"), ("title", "/title"), ("objective", "/objective")]),
        session,
        collection(agent, "workspaces", "Agent workspaces", "Workspaces of the Agent", "home.workspaces", json!({}), "workspace_id",
            &[("name", "/name"), ("lifecycle", "/lifecycle"), ("availability", "/availability"), ("description", "/description")]),
        workspace,
        collection(agent, "goals", "Agent goals", "Goals of the Agent", "goals.list", json!({}), "goal_id",
            &[("title", "/title"), ("status", "/status"), ("lifecycle", "/lifecycle")]),
        goal,
        collection(agent, "artifacts", "Agent artifacts", "Artifacts of the Agent", "artifacts.list", json!({}), "aid",
            &[("head", "/head"), ("rev", "/rev"), ("workspace", "/workspace/workspace_id")]),
        artifact,
    ]
}

/// The Provider of the Agent `agent_id` (DID `agent_did`), mounted under `/kapi/<agent_id>/obj`.
pub fn provider<H: RPCHandler + Send + Sync + 'static>(handler: Weak<H>, agent_id: &str, agent_did: DID) -> KrpcObjectProvider {
    KrpcObjectProvider::new(agent_id, agent_did, object_kinds(agent_id), Arc::new(RpcHandlerBackend(handler)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_http_server::{HttpServer, StreamInfo};
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
    use std::net::IpAddr;

    struct FakeState;

    #[async_trait::async_trait]
    impl RPCHandler for FakeState {
        async fn handle_rpc_call(&self, req: RPCRequest, _ip: IpAddr) -> Result<RPCResponse, RPCErrors> {
            if req.token.as_deref() != Some("owner") {
                return Err(RPCErrors::NoPermission("not the owner".into()));
            }
            let value = match req.method.as_str() {
                "sessions.query" => json!([{ "session_id": "s-1", "kind": "ui" }, { "session_id": "s 2", "kind": "goal" }]),
                "sessions.lookup" => json!({ "session_id": req.params["sid"] }),
                other => json!({ "method": other }),
            };
            Ok(RPCResponse::new(RPCResult::Success(value), req.seq))
        }
    }

    async fn send(server: &buckyos_http_server::DIDObjectHttpServer<KrpcObjectProvider>, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let req = http::Request::builder()
            .method(method)
            .uri(path)
            .header("host", "test.buckyos.io")
            .header("authorization", "Bearer owner")
            .body(Full::new(Bytes::from(body.map(|b| b.to_string()).unwrap_or_default())).map_err(|never| match never {}).boxed())
            .unwrap();
        let resp = server.serve_request(req, StreamInfo::default()).await.unwrap();
        let status = resp.status().as_u16();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn agent_objects_answer_with_agent_id_urls() {
        let state = Arc::new(FakeState);
        let agent = "xiaobai.test.buckyos.io";
        let server = provider(Arc::downgrade(&state), agent, DID::new("web", agent)).into_http_server();
        let base = format!("/kapi/{agent}/obj");
        for path in ["", "/sessions", "/sessions/s-1", "/workspaces/ws-1", "/goals/g-1", "/artifacts/a-1"] {
            let (status, profile) = send(&server, "GET", &format!("{base}{path}?type=profile"), None).await;
            assert_eq!(status, 200, "{path}");
            name_lib::ObjectProfile::from_value(profile).unwrap().validate().unwrap();
        }
        let (status, card) = send(&server, "GET", &format!("{base}/sessions/s-1"), None).await;
        assert_eq!(status, 200, "{card}");
        assert_eq!(card["id"], "did:web:xiaobai.test.buckyos.io:sessions:s-1");
        assert_eq!(card["alsoKnownAs"][1], "obj://xiaobai.test.buckyos.io/sessions/s-1");
        let (_, page) = send(&server, "POST", &format!("{base}/sessions/methods/query"), Some(json!({ "method": "query", "params": {} }))).await;
        let objects: Vec<&str> = page["result"]["rows"].as_array().unwrap().iter().map(|r| r["object"].as_str().unwrap()).collect();
        assert_eq!(objects, ["obj://xiaobai.test.buckyos.io/sessions/s-1", "obj://xiaobai.test.buckyos.io/sessions/s%202"]);
    }

    struct TenSessions;

    #[async_trait::async_trait]
    impl RPCHandler for TenSessions {
        async fn handle_rpc_call(&self, req: RPCRequest, _ip: IpAddr) -> Result<RPCResponse, RPCErrors> {
            let value = match req.method.as_str() {
                "sessions.query" => json!((1..=10)
                    .map(|i| json!({
                        "session_id": format!("work-{i}"),
                        "kind": "work",
                        "class": "",
                        "location": "/agents/jarvis/sessions",
                        "driver": { "principal": "did:bns:alice" },
                        "status": { "rev": i, "run_state": "running", "one_line_status": "building" },
                        "objective": format!("Fix build {i}\nthen run the tests"),
                    }))
                    .collect::<Vec<_>>()),
                other => json!({ "method": other }),
            };
            Ok(RPCResponse::new(RPCResult::Success(value), req.seq))
        }
    }

    struct Zone(String);

    #[async_trait::async_trait]
    impl agent_did_object_lib::zone::ZoneAccess for Zone {
        fn gateway_origin(&self) -> Option<String> {
            Some(self.0.clone())
        }
        fn zone_host(&self) -> Option<String> {
            Some("test.buckyos.io".into())
        }
        fn data_root(&self) -> std::path::PathBuf {
            "/nonexistent/data".into()
        }
        async fn session_token(&self) -> Option<String> {
            Some("owner".into())
        }
    }

    /// `read <agent>/sessions`: one line per session, with the fields its row names.
    #[tokio::test]
    async fn a_session_list_reads_one_line_per_session() {
        use agent_did_object_lib::adapters::{AdapterRegistry, DidObjectProtocolAdapter};
        use agent_did_object_lib::{AgentDIDObjectRuntime, ObjectRouteConfig, ObjectUrlReader};
        use agent_tool::read_tool::UrlReader;
        let state = Arc::new(TenSessions);
        let agent = "jarvis.test.buckyos.io";
        let server = Arc::new(provider(Arc::downgrade(&state), agent, DID::new("web", agent)).into_http_server());
        let serve = move |req: axum::extract::Request| {
            let server = server.clone();
            async move {
                let (parts, body) = req.into_parts();
                let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
                let response = server.serve_request(buckyos_api::buffered_request(parts, bytes), StreamInfo::default()).await.unwrap();
                response.map(axum::body::Body::new)
            }
        };
        let app = axum::Router::new()
            .route(&format!("/kapi/{agent}/obj"), axum::routing::any(serve.clone()))
            .route(&format!("/kapi/{agent}/obj/{{*rest}}"), axum::routing::any(serve));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(DidObjectProtocolAdapter::with_zone("did-object".into(), Arc::new(Zone(origin)))));
        let reader = ObjectUrlReader::new(AgentDIDObjectRuntime::with_registry(ObjectRouteConfig::builtin(), registry).unwrap());
        let ctx = agent_tool::SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: agent.into(),
            behavior: "b".into(),
            tool_call_index: 0,
            wakeup_id: "w".into(),
            session_id: "s".into(),
            read_token_limit: agent_tool::DEFAULT_READ_TOKEN_LIMIT,
        };
        let doc = reader.read_url(&ctx, &format!("obj://{agent}/sessions"), None).await.unwrap();
        let rows: Vec<&str> = doc.text.lines().filter(|l| l.contains("/sessions/work-")).collect();
        assert_eq!(rows.len(), 10, "{}", doc.text);
        assert_eq!(
            rows[0],
            "  - obj://jarvis.test.buckyos.io/sessions/work-1 — kind: work · objective: Fix build 1 then run the tests · status: running"
        );
        assert!(!doc.text.contains("did:bns:alice") && !doc.text.contains("building"), "{}", doc.text);
    }
}

/// Object events of the Agent's sessions (`obj://<agent-id>/sessions/<sid>`, doc/key url.md §7): `changed` on every
/// reported state change, `finished` once when a session finishes; what `ctl subscribe` and the kevent bridge follow.
pub struct SessionEvents {
    pub client: Arc<buckyos_api::KEventClient>,
    pub agent: DID,
    pub finished: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl SessionEvents {
    pub fn new(client: Arc<buckyos_api::KEventClient>, agent: DID) -> Self {
        Self { client, agent, finished: Default::default() }
    }

    async fn publish(&self, sid: &str, event: &str, data: &Value) {
        let host = self.agent.to_host_name();
        let key = buckyos_api::obj_event_key(&host, &["sessions", sid], event);
        if let Err(e) = self.client.pub_event(&key, data.clone()).await {
            log::debug!("session event {key}: {e}");
        }
    }
}

#[async_trait::async_trait]
impl libopendan::channel::Notifier for SessionEvents {
    async fn session_changed(&self, sid: &str, status: &libopendan::protocol::SessionStatus) {
        let state = status.run_state.as_str();
        let data = json!({
            "session": buckyos_api::obj_entity_url(&self.agent, &["sessions", sid]),
            "rev": status.rev,
            "run_state": state,
            "outcome": status.outcome,
            "one_line_status": status.one_line_status,
            "summary": format!("{state}: {}", status.one_line_status),
        });
        self.publish(sid, "changed", &data).await;
        let first_finish = status.run_state == libopendan::protocol::RunState::Finished
            && self.finished.lock().map(|mut f| f.insert(sid.to_string())).unwrap_or(false);
        if first_finish {
            self.publish(sid, "finished", &data).await;
        }
    }
}

#[cfg(test)]
mod session_event_tests {
    use super::*;
    use libopendan::channel::Notifier;
    use libopendan::protocol::{RunState, SessionStatus};

    #[tokio::test]
    async fn a_session_reports_changed_each_time_and_finished_once() {
        let client = Arc::new(buckyos_api::KEventClient::new_local("ood1"));
        let agent = DID::from_str("did:web:jarvis.test.buckyos.io").unwrap();
        let host = "jarvis.test.buckyos.io";
        let reader = client
            .create_event_reader(vec![buckyos_api::obj_event_pattern(host, &["sessions", "work-1"])])
            .await
            .unwrap();
        let events = SessionEvents::new(client, agent);
        let mut status = SessionStatus::created(1);
        status.run_state = RunState::Running;
        events.session_changed("work-1", &status).await;
        status.run_state = RunState::Finished;
        events.session_changed("work-1", &status).await;
        events.session_changed("work-1", &status).await;
        let mut ids = Vec::new();
        loop {
            match reader.pull_event(Some(200)).await {
                Ok(Some(ev)) => ids.push(ev.eventid),
                Ok(None) => break,
                Err(e) => panic!("pull: {e}"),
            }
        }
        let changed = buckyos_api::obj_event_key(host, &["sessions", "work-1"], "changed");
        let finished = buckyos_api::obj_event_key(host, &["sessions", "work-1"], "finished");
        assert_eq!(ids.iter().filter(|i| **i == changed).count(), 3, "{ids:?}");
        assert_eq!(ids.iter().filter(|i| **i == finished).count(), 1, "{ids:?}");
        // The key a subscription to the object computes is the one published.
        let url = buckyos_api::parse_object_url("obj://jarvis.test.buckyos.io/sessions/work-1").unwrap();
        assert_eq!(url.object_event_key("finished").unwrap(), finished);
    }
}
