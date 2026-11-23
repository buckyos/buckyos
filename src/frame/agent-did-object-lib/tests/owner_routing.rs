use agent_did_object_lib::{AgentDIDObjectRuntime, ObjectRouteConfig, ReadInput, XCallInput};
use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use name_lib::{DIDObjectCard, DID};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

async fn listen() -> (tokio::net::TcpListener, String) {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", socket.local_addr().unwrap());
    (socket, url)
}

fn action(object: &str, method: &str) -> XCallInput {
    XCallInput {
        object: object.into(),
        action: method.into(),
        params: json!({}),
        idempotency_key: Some("only-once".into()),
        trace_id: Some("trace-one".into()),
        confirm_token: Some("confirmed".into()),
        session_id: None,
    }
}

#[tokio::test]
async fn local_and_link_bindings_dispatch_independently_under_one_session() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = dir.path();
    let tool = sandbox.join("libexec/buckyos-tool");
    let pkg = tool.join("dist/owner-extensions");
    fs::create_dir_all(&pkg).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rootfs/libexec/buckyos-tool/runtime")
        .join(if cfg!(windows) { "deno.exe" } else { "deno" });
    let has_deno = source.is_file();
    if has_deno {
        fs::create_dir_all(tool.join("runtime")).unwrap();
        fs::hard_link(
            &source,
            tool.join("runtime").join(source.file_name().unwrap()),
        )
        .or_else(|_| {
            fs::copy(
                &source,
                tool.join("runtime").join(source.file_name().unwrap()),
            )
            .map(|_| ())
        })
        .unwrap();
    }
    let mut manifest = "version = 1\nreserved_roots = [\"owner\"]\n".to_owned();
    for (alias, obj, method) in [("owner", "person", "notify"), ("owner.cal", "cal", "add")] {
        let profile = json!({"@context":"https://www.w3.org/2022/wot/td/v1.1","id":format!("urn:{obj}:v1"),"actions":{method:{"input":{"type":"object","additionalProperties":false},"output":{"type":"object"},"x-buckyos:action":{"effect":"write"}}}});
        fs::write(pkg.join(format!("{obj}.json")), profile.to_string()).unwrap();
        manifest.push_str(&format!("[[objects]]\nid='personal-tools/{obj}'\nprofile='{obj}.json'\n[objects.actions.{method}]\nscript='action.mjs'\nhost_calls=['owner.who','notify.send']\n[[aliases]]\nname='{alias}'\nkind='local'\nobject='personal-tools/{obj}'\n"));
    }
    fs::write(pkg.join("object_extensions.toml"), manifest).unwrap();
    fs::write(pkg.join("action.mjs"),"export default (req,host) => ({result:{source:'local', owner:host.context.owner_did, key:req.idempotency_key}})").unwrap();
    let (provider_socket, provider_origin) = listen().await;
    let external = format!("{provider_origin}/alice");
    let card = DIDObjectCard::new(
        DID::from_str("did:bns:alice").unwrap(),
        &external,
        None,
        format!("{external}/profile"),
        Some("fixture"),
    );
    let log = Arc::new(Mutex::new(Vec::<Value>::new()));
    let backend = log.clone();
    let app = Router::new()
        .route("/alice",get(move |headers:HeaderMap|async move{assert!(headers.get("authorization").is_none());Json(card)}))
        .route("/alice/profile",get(||async{Json(json!({"@context":"https://www.w3.org/2022/wot/td/v1.1","id":"urn:fixture","actions":{
            "add":{"forms":[{"href":"dispatch","op":"invokeaction"}],"x-buckyos:action":{"effect":"write"}},
            "notify":{"forms":[{"href":"dispatch","op":"invokeaction"}],"x-buckyos:action":{"effect":"write"}}
        }}))}))
        .route("/alice/dispatch",post(move|headers:HeaderMap,Json(inv):Json<Value>|async move{
            assert!(headers.get("authorization").is_none());
            backend.lock().unwrap().push(inv.clone());
            Json(json!({"result":{"source":"linked","key":inv["idempotency_key"]}}))
        }));
    tokio::spawn(async move { axum::serve(provider_socket, app).await.unwrap() });
    let (gateway_socket, gateway) = listen().await;
    let aliases = Arc::new(Mutex::new(json!({})));
    let state = aliases.clone();
    let published = external.clone();
    let app = Router::new()
        .route("/kapi/control-panel", post(move |State(state): State<Arc<Mutex<Value>>>, Json(rpc): Json<Value>| async move {
            assert_eq!(rpc["sys"][1], "fixture-token");
            assert_eq!(rpc["method"], "owner.context");
            let name = rpc["params"]["alias"].as_str().unwrap();
            let state = state.lock().unwrap();
            let person = state.get("person_ref").cloned().unwrap_or_else(|| json!(published));
            Json(json!({"result":{"owner_did":"did:bns:alice","read_only":state["read_only"] == true,"person_ref":person,"object_alias":state[name]}}))
        }))
        .with_state(state);
    tokio::spawn(async move { axum::serve(gateway_socket, app).await.unwrap() });
    std::env::set_var("BUCKYOS_ROOT", sandbox);
    std::env::set_var("BUCKYOS_TOOL_ENDPOINT", &gateway);
    std::env::set_var("BUCKYOS_APPCLIENT_SESSION_TOKEN", "fixture-token");
    let runtime = AgentDIDObjectRuntime::new(ObjectRouteConfig::builtin()).unwrap();
    for (linked, linked_action, local, local_action) in [
        ("owner.cal", "add", "owner", "notify"),
        ("owner", "notify", "owner.cal", "add"),
    ] {
        *aliases.lock().unwrap() =
            json!({linked:{"name":linked,"kind":"link","url":external,"alias_revision":2}});
        let read = runtime
            .read(ReadInput {
                object: local.into(),
                purpose: None,
                session_id: None,
                range: None,
                max_tokens: None,
                content_only: true,
                options: json!({}),
            })
            .await
            .unwrap();
        assert!(read.output.unwrap().contains(local_action));
        if has_deno {
            let result = runtime.x_call(action(local, local_action)).await.unwrap();
            assert_eq!(
                result.details["result"],
                json!({"source":"local","owner":"did:bns:alice","key":"only-once"})
            );
        }
        let result = runtime.x_call(action(linked, linked_action)).await.unwrap();
        let direct = runtime
            .x_call(action(&external, linked_action))
            .await
            .unwrap();
        assert_eq!(result.details["result"], direct.details["result"]);
        assert_eq!(result.details["object"], linked);
        let calls = log.lock().unwrap();
        let pair = &calls[calls.len() - 2..];
        assert_eq!(pair[0], pair[1]);
        assert_eq!(pair[0]["trace_id"], "trace-one");
        assert_eq!(pair[0]["confirm_token"], "confirmed");
    }

    let sent = log.lock().unwrap().len();
    for (disabled, active, method) in [
        ("owner", "owner.cal", "add"),
        ("owner.cal", "owner", "notify"),
    ] {
        *aliases.lock().unwrap() = json!({
            disabled:{"name":disabled,"kind":"disabled","alias_revision":5},
            active:{"name":active,"kind":"link","url":external,"alias_revision":8}
        });
        assert_eq!(
            runtime
                .x_call(action(disabled, "notify"))
                .await
                .unwrap_err()
                .detail()["code"],
            "ALIAS_DISABLED"
        );
        let r = runtime.x_call(action(active, method)).await.unwrap();
        assert_eq!(r.details["alias_revision"], 8);
    }
    assert_eq!(log.lock().unwrap().len(), sent + 2);
    *aliases.lock().unwrap() =
        json!({"owner.cal":{"name":"owner.cal","kind":"link","url":external,"alias_revision":9}});
    assert_eq!(
        runtime
            .x_call(action("owner.cal", "unknown"))
            .await
            .unwrap_err()
            .detail()["code"],
        "UNSUPPORTED_ACTION"
    );
    *aliases.lock().unwrap() = json!({"owner.cal":{"name":"owner.cal","kind":"local","object":"removed/calendar","alias_revision":10}});
    assert_eq!(
        runtime
            .x_call(action("owner.cal", "add"))
            .await
            .unwrap_err()
            .detail()["code"],
        "EXTENSION_UNAVAILABLE"
    );
    let (offline, url) = listen().await;
    drop(offline);
    *aliases.lock().unwrap() =
        json!({"owner.cal":{"name":"owner.cal","kind":"link","url":url,"alias_revision":11}});
    assert_eq!(
        runtime
            .x_call(action("owner.cal", "add"))
            .await
            .unwrap_err()
            .detail()["code"],
        "PROVIDER_UNAVAILABLE"
    );
    *aliases.lock().unwrap() = json!({"person_ref":"obj://control-panel/people/other","owner":{"name":"owner","kind":"link","url":external,"alias_revision":12}});
    assert_eq!(
        runtime
            .x_call(action("owner", "notify"))
            .await
            .unwrap_err()
            .detail()["code"],
        "OWNER_IDENTITY_UNVERIFIED"
    );
    *aliases.lock().unwrap() = json!({"read_only":true});
    assert_eq!(
        runtime
            .x_call(action("owner", "notify"))
            .await
            .unwrap_err()
            .detail()["code"],
        "PERMISSION_DENIED"
    );
    *aliases.lock().unwrap() = json!({});
    let mut invalid = action("owner", "notify");
    invalid.params = json!({"to":"bob"});
    assert_eq!(
        runtime.x_call(invalid).await.unwrap_err().detail()["code"],
        "INVALID_PARAMS"
    );
    if has_deno {
        for (body, expected) in [
            ("await host.call('unregistered.method', {})", "HOST_CALL_DENIED"),
            ("const value = Deno.env.get('BUCKYOS_APPCLIENT_SESSION_TOKEN'); return {result:{value}}", "INVALID_SCRIPT_RESPONSE"),
            ("await host.call('owner.who', {}); throw new Error('failed after read')", "INVALID_SCRIPT_RESPONSE"),
        ] {
            fs::write(pkg.join("action.mjs"), format!("export default async (req, host) => {{ {body} }}")).unwrap();
            let error = runtime.x_call(action("owner", "notify")).await.unwrap_err();
            assert_eq!(error.detail()["code"], expected);
        }
    }
    assert_eq!(log.lock().unwrap().len(), sent + 2);
}
