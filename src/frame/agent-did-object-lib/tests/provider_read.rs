//! `read` of an `obj://` object through a real DID Object Provider (buckyos-api `KrpcObjectProvider`) served over
//! HTTP, as an Agent reaches it through its node gateway.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_did_object_lib::adapters::{AdapterRegistry, CyfsAdapter, DidObjectProtocolAdapter};
use agent_did_object_lib::zone::ZoneAccess;
use agent_did_object_lib::{AgentDIDObjectRuntime, ObjectRouteConfig, ObjectUrlReader};
use agent_tool::read_tool::UrlReader;
use agent_tool::SessionRuntimeContext;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::Request;
use buckyos_api::{KrpcObjectProvider, ObjectKind, ProviderRpc, RpcBinding};
use buckyos_http_server::{DIDObjectHttpServer, HttpServer, StreamInfo};
use kRPC::RPCErrors;
use name_lib::DID;
use serde_json::{json, Value};

struct Sessions {
    tokens: Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl ProviderRpc for Sessions {
    async fn call(&self, method: &str, params: Value, token: Option<String>, _ip: Option<IpAddr>) -> Result<Value, RPCErrors> {
        self.tokens.lock().unwrap().push(token);
        match method {
            "sessions.list" => Ok(json!({ "owner": params["owner"], "items": [{ "session_id": "s-1", "title": "hello" }] })),
            other => Err(RPCErrors::ReasonError(format!("unknown {other}"))),
        }
    }
}

struct Zone {
    origin: String,
}

#[async_trait]
impl ZoneAccess for Zone {
    fn gateway_origin(&self) -> Option<String> {
        Some(self.origin.clone())
    }
    fn zone_host(&self) -> Option<String> {
        Some("test.buckyos.io".into())
    }
    fn data_root(&self) -> PathBuf {
        PathBuf::from("/nonexistent/data")
    }
    async fn session_token(&self) -> Option<String> {
        Some("token-1".into())
    }
}

async fn serve(server: Arc<DIDObjectHttpServer<KrpcObjectProvider>>, req: Request) -> axum::response::Response {
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    let response = server.serve_request(buckyos_api::buffered_request(parts, bytes), StreamInfo::default()).await.unwrap();
    response.map(Body::new)
}

async fn spawn_provider(backend: Arc<Sessions>) -> String {
    let kind = ObjectKind::new("owners/{owner}/sessions", "msg-center.sessions", "Sessions", "Every session of one mailbox owner.")
        .summary_property(
            "recent",
            "The newest sessions",
            json!({ "type": "object" }),
            RpcBinding::new("sessions.list", |c, _| Ok(json!({ "owner": c.get("owner").cloned().unwrap_or_default() }))),
        )
        .index(
            "List the owner's sessions",
            json!({ "title": "string" }),
            RpcBinding::new("sessions.list", |c, _| Ok(json!({ "owner": c.get("owner").cloned().unwrap_or_default() }))),
        );
    let server = Arc::new(
        KrpcObjectProvider::new("msg-center", DID::new("web", "test.buckyos.io"), vec![kind], backend).into_http_server(),
    );
    let (root, rest) = (server.clone(), server);
    let app = axum::Router::new()
        .route("/kapi/msg-center/obj", axum::routing::any(move |req| serve(root.clone(), req)))
        .route("/kapi/msg-center/obj/{*rest}", axum::routing::any(move |req| serve(rest.clone(), req)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    origin
}

fn reader(origin: &str) -> ObjectUrlReader {
    let zone: Arc<dyn ZoneAccess> = Arc::new(Zone { origin: origin.to_string() });
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(DidObjectProtocolAdapter::with_zone("did-object".into(), zone.clone())));
    registry.register(Arc::new(CyfsAdapter::with_zone("cyfs".into(), zone)));
    ObjectUrlReader::new(AgentDIDObjectRuntime::with_registry(ObjectRouteConfig::builtin(), registry).unwrap())
}

fn ctx() -> SessionRuntimeContext {
    SessionRuntimeContext {
        trace_id: "t".into(),
        agent_name: "a".into(),
        behavior: "b".into(),
        tool_call_index: 0,
        wakeup_id: "w".into(),
        session_id: "s".into(),
        read_token_limit: agent_tool::DEFAULT_READ_TOKEN_LIMIT,
    }
}

#[tokio::test]
async fn an_obj_url_reads_as_its_object_document_with_the_callers_session() {
    let backend = Arc::new(Sessions { tokens: Mutex::new(Vec::new()) });
    let origin = spawn_provider(backend.clone()).await;
    let reader = reader(&origin);
    let url = "obj://msg-center/owners/did:bns:alice/sessions";

    let doc = reader.read_url(&ctx(), url, None).await.unwrap();
    for expected in [
        "Object: obj://msg-center/owners/did:bns:alice/sessions",
        "Description: Every session of one mailbox owner.",
        "- recent: The newest sessions",
        "- query: List the owner's sessions (risk: read_only)",
        "Summary:",
        "hello",
    ] {
        assert!(doc.text.contains(expected), "missing `{expected}` in\n{}", doc.text);
    }
    assert_eq!(doc.details["object"], url);
    assert!(doc.details["source"].as_str().unwrap().starts_with(&origin));
    assert_eq!(backend.tokens.lock().unwrap().last().cloned().flatten().as_deref(), Some("token-1"));

    let property = reader.read_url(&ctx(), url, Some("recent")).await.unwrap();
    assert!(property.text.contains("did:bns:alice") && !property.text.contains("Object:"), "{}", property.text);

    let err = reader.read_url(&ctx(), url, Some("nope")).await.unwrap_err();
    assert!(err.to_string().contains("not declared"), "{err}");

    let err = reader.read_url(&ctx(), "obj://msg-center/unknown/kind", None).await.unwrap_err();
    assert!(err.to_string().contains("msg-center/unknown/kind"), "{err}");
}

#[tokio::test]
async fn an_xcall_reports_the_declared_risk() {
    let backend = Arc::new(Sessions { tokens: Mutex::new(Vec::new()) });
    let origin = spawn_provider(backend).await;
    let zone: Arc<dyn ZoneAccess> = Arc::new(Zone { origin });
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(DidObjectProtocolAdapter::with_zone("did-object".into(), zone)));
    let runtime = AgentDIDObjectRuntime::with_registry(ObjectRouteConfig::builtin(), registry).unwrap();
    let result = runtime
        .x_call(agent_did_object_lib::XCallInput {
            object: "obj://msg-center/owners/did:bns:alice/sessions".into(),
            action: "query".into(),
            params: json!({}),
            session_id: None,
            idempotency_key: None,
            confirm_token: None,
            trace_id: None,
        })
        .await
        .unwrap();
    assert_eq!(result.details["risk"], "read_only", "{:?}", result.details);
    assert_eq!(result.details["object"], "obj://msg-center/owners/did:bns:alice/sessions");
}
