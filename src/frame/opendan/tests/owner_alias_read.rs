//! The Loader delegates bare names to its object reader using the Agent App's
//! registered runtime. A disabled user override must reach the model as such.

use std::sync::Arc;

use agent_did_object_lib::ObjectUrlReader;
use agent_tool::{
    file_tools::FileToolConfig, read_tool::ReadTool, AgentTool, SessionRuntimeContext,
};
use axum::{routing::post, Json, Router};
use buckyos_api::{set_buckyos_api_runtime, BuckyOSRuntime, BuckyOSRuntimeType};
use opendan::urls::AgentUrlReader;
use serde_json::{json, Value};

#[tokio::test]
async fn loader_read_honors_the_verified_users_alias_override() {
    let root = tempfile::tempdir().unwrap();
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    std::env::set_var("BUCKYOS_ROOT", root.path());
    std::env::set_var("BUCKYOS_HOST_GATEWAY", "127.0.0.1");
    // A CLI override must never receive the Loader's application credential.
    std::env::set_var("BUCKYOS_TOOL_ENDPOINT", "http://127.0.0.1:1");
    std::env::remove_var("BUCKYOS_APPCLIENT_SESSION_TOKEN");
    let mut runtime = BuckyOSRuntime::new(
        "jarvis.test.buckyos.io",
        Some("alice".into()),
        BuckyOSRuntimeType::AppService,
    );
    runtime.node_gateway_port = addr.port();
    *runtime.session_token.write().await = "fixture-session".into();
    set_buckyos_api_runtime(runtime).unwrap();

    let gateway = Router::new().route(
        "/kapi/control-panel",
        post(|Json(req): Json<Value>| async move {
            assert_eq!(req["sys"][1], "fixture-session");
            assert_eq!(req["method"], "owner.context");
            assert_eq!(req["params"]["alias"], "owner.cal");
            Json(
                json!({"result": {"owner_did":"did:bns:alice", "object_alias":{
                    "name":"owner.cal", "kind":"disabled", "alias_revision":2
                }}}),
            )
        }),
    );
    tokio::spawn(async move { axum::serve(socket, gateway).await.unwrap() });

    let did = "did:web:jarvis.test.buckyos.io";
    let agent =
        libopendan::FsAgentStateClient::open(root.path().join("agent"), did, None, None).unwrap();
    let urls = AgentUrlReader {
        objects: ObjectUrlReader::builtin(),
        agent: Arc::new(agent),
        ops_dir: root.path().join("ops"),
    };
    let read = ReadTool::new(FileToolConfig::new(root.path())).with_urls(Some(Arc::new(urls)));
    let ctx = SessionRuntimeContext {
        trace_id: "trace".into(),
        agent_name: did.into(),
        behavior: "chat".into(),
        tool_call_index: 0,
        wakeup_id: "wakeup".into(),
        session_id: "session".into(),
        read_token_limit: agent_tool::DEFAULT_READ_TOKEN_LIMIT,
    };
    let error = read
        .call(&ctx, json!({"uri":"owner.cal"}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ALIAS_DISABLED"), "{error}");
}
