//! A Zone source synced by an Agent's sync task (AI Native 余下任务 §8.2): a triggered refresh has the workspace owner's
//! Agent run the task (`sync.run` at `<agent_gateway>/<agent_id>`) and then reads its output; an Agent that is not the
//! owner's, unreachable, or refusing fails the refresh with `SYNC_UNAVAILABLE`, a failed run with its reason, and the data
//! stays at its last valid version; the timer only reads the file again; whoever has `run` triggers it.

#[path = "common/wishkit.rs"]
mod wishkit;
use axum::extract::{Path, State};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use wishkit::*;

const OUT: &str = "/home/alice/agents/bot.example/sync/quotes/out.json";

fn tokens() -> Value {
    json!({ "tok-alice": { "principal": "alice" }, "tok-bob": { "principal": "bob" },
            "tok-bot": { "principal": "did:web:bot.example", "class": "agent", "owner": "alice" },
            "tok-bobs": { "principal": "did:web:bobs.example", "class": "agent", "owner": "bob" } })
}

/// What the fake Agent does with `sync.run`: write this output and answer this run, or answer this error.
#[derive(Clone)]
enum Answer {
    Run { out: Option<Value>, run: Value },
    Error(&'static str),
}

struct Agent {
    zone: std::path::PathBuf,
    answer: Mutex<Answer>,
    calls: Mutex<Vec<(String, String, Value)>>,
}

async fn krpc(State(agent): State<Arc<Agent>>, Path(agent_id): Path<String>, axum::Json(req): axum::Json<Value>) -> axum::Json<Value> {
    let seq = req["sys"][0].clone();
    agent.calls.lock().unwrap().push((agent_id, req["method"].as_str().unwrap_or("").to_string(), req["params"].clone()));
    let answer = agent.answer.lock().unwrap().clone();
    axum::Json(match answer {
        Answer::Run { out, run } => {
            if let Some(out) = out {
                let file = agent.zone.join(OUT.trim_start_matches('/'));
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, serde_json::to_vec(&out).unwrap()).unwrap();
            }
            json!({ "result": { "run": run }, "sys": [seq] })
        }
        Answer::Error(e) => json!({ "error": e, "sys": [seq] }),
    })
}

fn envelope(price: f64) -> Value {
    json!({ "format": "aiws.sync.v1", "status": "ok", "program": { "name": "quotes", "version": "2" }, "business_time": "2026-11-16T08:00:00Z",
            "produced_at": "2026-11-16T08:00:05Z", "records": [{ "symbol": "AAPL", "price": price }] })
}

fn source(agent: &str) -> Value {
    json!({ "kind": "zone", "path": OUT.replace("bot.example", agent), "refresh": { "interval_s": 900 }, "sync": { "agent_id": agent, "task_id": "quotes" },
            "def": { "meaning": "Daily closing prices", "primary_key": ["symbol"] } })
}

async fn refresh_as(srv: &Srv, token: &str, entity: &str) -> Value {
    static K: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let key = format!("s{}", K.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let run = srv.call_as(token, "proc.start", json!({ "program": "source.refresh@1", "idempotency_key": key, "params": { "entity_id": entity } })).await;
    match run["result"]["run_id"].as_str() {
        Some(id) => srv.wait_as(token, id).await,
        None => run,
    }
}

/// The service as `Srv::start_with_limits` starts it, keeping its state for the timer.
async fn start(limits: aiworkspace_server::Limits) -> (Srv, Arc<aiworkspace_server::AppState>) {
    let dir = tempfile::tempdir().unwrap();
    let nfs = wishkit::nfs::Nfs::start(dir.path()).await;
    let svc = aiworkspace_store::Service::open(&dir.path().join("data"), nfs.objects().await).unwrap();
    let tokens: aiworkspace_server::auth::StaticTokens = serde_json::from_value(json!({ "tokens": tokens() })).unwrap();
    let state = aiworkspace_server::AppState::new(svc, Arc::new(tokens), limits, None, aiworkspace_server::wish::WishConfig::default());
    let _ = state.ndm_gateway.set(nfs.base.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = aiworkspace_server::build_router(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (Srv { base: format!("http://{addr}/kapi/aiworkspace"), http: reqwest::Client::new(), model: model(), _dir: dir, ws: String::new(), nfs }, state)
}

fn price(rows: &[Value]) -> Option<f64> {
    rows.first().and_then(|r| r["values"]["price"].as_f64())
}

#[test]
fn a_triggered_refresh_runs_the_sync_task_first() {
    run_test(|| async move {
    let zone = tempfile::tempdir().unwrap();
    let agent = Arc::new(Agent { zone: zone.path().to_path_buf(), answer: Mutex::new(Answer::Error("unset")), calls: Mutex::default() });
    let app = axum::Router::new().route("/kapi/{agent_id}", axum::routing::post(krpc)).with_state(agent.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway = format!("http://{}/kapi", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let limits = aiworkspace_server::Limits { zone_root: Some(zone.path().to_path_buf()), agent_gateway: Some(gateway), ..Default::default() };
    let (mut srv, state) = start(limits).await;
    srv.ws = srv.rpc("ws.create", json!({ "title": "行情" })).await["workspace_id"].as_str().unwrap().to_string();
    let fields = json!([{ "field_id": "symbol", "name": "Symbol", "type": "text" }, { "field_id": "price", "name": "Price", "type": "number" }]);
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "quotes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "quotes", "payload": { "fields": fields, "source": source("bot.example") } },
        { "op": "entity.create", "entity_id": "theirs", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "theirs", "payload": { "fields": fields, "source": source("bobs.example") } },
        { "op": "entity.create", "entity_id": "stranger", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "c", "name": "stranger", "payload": { "fields": fields, "source": source("nobody.example") } },
        { "op": "entity.create", "entity_id": "timed", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "d", "name": "timed", "payload": { "fields": fields, "source": source("bot.example") } },
    ])).await;
    let m = srv.rpc("doc.read", json!({ "entity_id": "quotes", "as": "meta" })).await;
    assert_eq!(m["data"]["sync"], json!({ "agent_id": "bot.example", "task_id": "quotes" }), "{m}");

    // the task runs on the owner's Agent, with the task id and the wait; then its output is read
    *agent.answer.lock().unwrap() = Answer::Run { out: Some(envelope(190.0)), run: json!({ "run_id": "r1", "status": "succeeded", "trigger": "aiworkspace" }) };
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!(done["state"], "succeeded", "{done}");
    assert_eq!(price(&srv.records("quotes").await), Some(190.0));
    assert_eq!(agent.calls.lock().unwrap().clone(), vec![("bot.example".to_string(), "sync.run".to_string(), json!({ "task_id": "quotes", "wait_ms": 120000 }))]);

    // whoever has `run` triggers it; the task's run is the Agent's to report
    srv.rpc("acl.set", json!({ "changes": [{ "subject": "bob", "resource": "/", "allow": ["read", "run"] }] })).await;
    *agent.answer.lock().unwrap() = Answer::Run { out: Some(envelope(191.0)), run: json!({ "run_id": "r2", "status": "succeeded" }) };
    assert_eq!(refresh_as(&srv, "tok-bob", "quotes").await["state"], "succeeded");
    assert_eq!(price(&srv.records("quotes").await), Some(191.0));

    // a disabled task (the Agent refuses), a failed run: the refresh fails, says why, the data stays
    *agent.answer.lock().unwrap() = Answer::Error("the sync task quotes is disabled");
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!((done["state"].as_str(), done["result"]["error"]["sub_code"].as_str()), (Some("failed"), Some("SYNC_UNAVAILABLE")), "{done}");
    assert!(done["result"]["error"]["detail"].as_str().unwrap().contains("disabled"), "{done}");
    let m = srv.rpc("doc.read", json!({ "entity_id": "quotes", "as": "meta" })).await;
    assert_eq!((m["data"]["state"].as_str(), m["data"]["attempt"]["error"]["sub_code"].as_str()), (Some("valid"), Some("SYNC_UNAVAILABLE")), "{m}");
    *agent.answer.lock().unwrap() = Answer::Run { out: None, run: json!({ "run_id": "r3", "status": "failed", "kind": "transient", "error": "the exchange answered 503" }) };
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!(done["state"], "failed", "{done}");
    assert!(done["result"]["error"]["detail"].as_str().unwrap().contains("the exchange answered 503") && done["result"]["error"]["sub_code"].is_null(), "{done}");
    assert_eq!(price(&srv.records("quotes").await), Some(191.0));

    // only an Agent of the current owner is asked: another user's, or no Agent at all, is never called
    let before = agent.calls.lock().unwrap().len();
    for id in ["theirs", "stranger"] {
        let done = refresh_as(&srv, "tok-alice", id).await;
        assert!(done["result"]["error"]["sub_code"] == "SYNC_UNAVAILABLE" && done.to_string().contains("not an Agent of this workspace's owner (alice)"), "{id}: {done}");
    }
    assert_eq!(agent.calls.lock().unwrap().len(), before);

    // the timer only reads the output again: the Agent keeps the task's schedule ("timed" never synced, so it is due)
    let file = zone.path().join(OUT.trim_start_matches('/'));
    std::fs::write(&file, serde_json::to_vec(&envelope(192.0)).unwrap()).unwrap();
    let st = state.clone();
    tokio::task::spawn_blocking(move || aiworkspace_server::source::tick(&st)).await.unwrap();
    for _ in 0..200 {
        if srv.rpc("doc.read", json!({ "entity_id": "timed", "as": "meta" })).await["data"]["state"] == "valid" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(price(&srv.records("timed").await), Some(192.0));
    assert_eq!(agent.calls.lock().unwrap().len(), before);
    });
}

/// Without an Agent gateway the task cannot run: the refresh says so.
#[test]
fn without_an_agent_gateway_the_sync_is_unavailable() {
    run_test(|| async move {
    let zone = tempfile::tempdir().unwrap();
    let limits = aiworkspace_server::Limits { zone_root: Some(zone.path().to_path_buf()), ..Default::default() };
    let (mut srv, _state) = start(limits).await;
    srv.ws = srv.rpc("ws.create", json!({ "title": "q" })).await["workspace_id"].as_str().unwrap().to_string();
    srv.commit(json!([{ "op": "entity.create", "entity_id": "quotes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "quotes",
                        "payload": { "fields": [{ "field_id": "symbol", "name": "Symbol", "type": "text" }], "source": source("bot.example") } }])).await;
    let done = refresh_as(&srv, "tok-alice", "quotes").await;
    assert_eq!((done["state"].as_str(), done["result"]["error"]["sub_code"].as_str()), (Some("failed"), Some("SYNC_UNAVAILABLE")), "{done}");
    assert_eq!(srv.rpc("doc.read", json!({ "entity_id": "quotes", "as": "meta" })).await["data"]["state"], "pending");
    });
}
