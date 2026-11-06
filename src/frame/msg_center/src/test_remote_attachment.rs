//! Two Zones and a private message with an attachment (issue #689 P1, bucket design N18):
//! the receiver starts with nothing and ends with a verified binding; only recipients
//! get a token; what the sender's Zone does not hold never appears.

use crate::buckets::{BucketHost, Buckets};
use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use crate::network_directory::test_support::StaticDirectory;
use crate::remote_attachment::RemoteAttachment;
use crate::test_cross_zone::{connect, documents, sign_as, verify_hub, ZONE_A, ZONE_B};
use crate::test_group_service::{http_body, http_fixture, TestBody};
use buckyos_api::bucket_client::BucketId;
use buckyos_api::{bind_token_principal_kind, bind_token_target, AuthTarget, StaticCustodyResolver, SystemServiceId, TokenPrincipalKind, TokenUse};
use http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCContext, RPCSessionToken, RPCSessionTokenType};
use name_lib::DID;
use named_store::NamedDataMgr;
use ndn_lib::{MsgContent, MsgObjKind, MsgObject, NamedObject, ObjId, RefItem, RefRole, RefTarget};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};

fn alice() -> DID {
    DID::new("web", "alice.a.example")
}
fn bob() -> DID {
    DID::new("web", "bob.b.example")
}
fn carol() -> DID {
    DID::new("web", "carol.b.example")
}

fn context(who: &DID, key: &str) -> RPCContext {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(buckyos_api::VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(who.to_string()),
        appid: Some("messagehub".into()),
        sudo: false,
        extra: HashMap::new(),
    };
    bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
    bind_token_target(&mut token, &AuthTarget::system(SystemServiceId::parse("msg-center").unwrap()), TokenUse::Session).unwrap();
    RPCContext {
        token: Some(token.generate_jwt(None, &EncodingKey::from_ed_pem(key.as_bytes()).unwrap()).unwrap()),
        from_ip: Some("127.0.0.1".parse().unwrap()),
        ..Default::default()
    }
}

struct Zone {
    center: MessageCenter,
    buckets: Buckets,
    /// `http://host:port` of the zone's nfs-server.
    nfs: String,
    _tmp: TempDir,
}

async fn zone(zone: &'static crate::test_cross_zone::TestZone, users: Vec<DID>, docs: Arc<StaticCustodyResolver>) -> Zone {
    let tmp = tempdir().unwrap();
    let db = MsgBoxDbMgr::open_default_sqlite(&format!("sqlite://{}?mode=rwc", tmp.path().join("msg.db").display())).await.unwrap();
    let center = MessageCenter::open_with_db(db).await.unwrap();
    center.cyfs_dispatch.write().unwrap().target_zone = Some(zone.host.into());
    center.register_local_recipients(users);
    center.directory.set(Arc::new(StaticDirectory::default()));
    sign_as(&center, zone, docs);

    let mut config = nfs_server::ServerConfig::new(tmp.path().join("nfs"), vec![]);
    config.named_store = nfs_server::config::NamedStoreSource::Dir(tmp.path().join("store"));
    config.bucket_specs = serde_json::from_value(json!({"msg-center": {
        "buckets": buckyos_api::msg_center_bucket_decls(),
        "storage_quota_mib": buckyos_api::MSG_CENTER_STORAGE_QUOTA_MIB,
    }}))
    .unwrap();
    let state = nfs_server::AppState::new(config).unwrap();
    let store = state.share.store(&state.config).await.unwrap().clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let nfs = format!("http://{}", listener.local_addr().unwrap());
    let app = nfs_server::build_router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let buckets = Buckets { client: Arc::new(buckyos_api::BucketClient::new(kRPC::kRPC::new(&format!("{nfs}/kapi/nfs-server"), None))), store };
    center.set_bucket_host(BucketHost::Fixed(buckets.clone()));
    Zone { center, buckets, nfs, _tmp: tmp }
}

/// The source Zone as seen from outside: verify-hub, msg-center kRPC and `/ndn/`.
async fn expose(source: &Zone, hits: Arc<std::sync::Mutex<Vec<String>>>) -> (String, tokio::task::JoinHandle<()>) {
    let center = source.center.clone();
    let nfs = source.nfs.clone();
    http_fixture(move |request: Request<TestBody>| {
        let center = center.clone();
        let nfs = nfs.clone();
        let hits = hits.clone();
        async move {
            let resolver = center.custody.get();
            let request = match verify_hub(&ZONE_A, resolver.as_ref(), request).await {
                Ok(response) => return response,
                Err(request) => request,
            };
            let path = request.uri().path().to_string();
            hits.lock().unwrap().push(path.clone());
            if path.starts_with("/ndn/") {
                let url = format!("{nfs}{}", request.uri().path_and_query().unwrap());
                let answered = reqwest::get(url).await.unwrap();
                let status = answered.status();
                let headers = answered.headers().clone();
                let bytes = answered.bytes().await.unwrap();
                let mut builder = Response::builder().status(status);
                for (name, value) in &headers {
                    builder = builder.header(name, value);
                }
                return builder.body(http_body(bytes)).unwrap();
            }
            let token = request.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(String::from);
            let body = request.into_body().collect().await.unwrap().to_bytes();
            let call: Value = serde_json::from_slice(&body).unwrap();
            let token = call["sys"][1].as_str().map(String::from).or(token);
            let ctx = RPCContext { token, from_ip: Some("127.0.0.1".parse().unwrap()), ..Default::default() };
            let method = call["method"].as_str().unwrap_or_default().to_string();
            let params = call["params"].clone();
            let outcome = crate::owner_session::serve_method(&method, center.attachment_ticket(params, &ctx)).await;
            let answer = match outcome {
                Ok(value) => json!({"result": value, "sys": [call["sys"][0]]}),
                Err(e) => json!({"error": e.to_string(), "sys": [call["sys"][0]]}),
            };
            Response::builder().status(StatusCode::OK).header("content-type", "application/json").body(http_body(answer.to_string())).unwrap()
        }
    })
    .await
}

struct Scene {
    a: Zone,
    b: Zone,
    hits: Arc<std::sync::Mutex<Vec<String>>>,
    _server: tokio::task::JoinHandle<()>,
}

async fn scene() -> Scene {
    let docs = Arc::new(documents(&[(&ZONE_A, vec![alice()]), (&ZONE_B, vec![bob(), carol()])]));
    let a = zone(&ZONE_A, vec![alice()], docs.clone()).await;
    let b = zone(&ZONE_B, vec![bob(), carol()], docs.clone()).await;
    let hits = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (origin, server) = expose(&a, hits.clone()).await;
    let origins = HashMap::from([(ZONE_A.host.to_string(), origin.clone())]);
    connect(&b.center, &ZONE_B, &docs, origins);
    b.center.cyfs_dispatch.write().unwrap().zone_origins.insert(ZONE_A.host.to_string(), origin);
    Scene { a, b, hits, _server: server }
}

async fn stored_file(store: &NamedDataMgr, name: &str, data: &[u8]) -> ObjId {
    let chunk = ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(data).unwrap();
    store.put_chunk(&chunk, data).await.unwrap();
    let (id, body) = ndn_lib::FileObject::new(name.to_string(), data.len() as u64, chunk.to_string()).gen_obj_id();
    store.put_object(&id, &body).await.unwrap();
    id
}

fn message(from: &DID, to: &DID, attachment: &ObjId, created: u64) -> MsgObject {
    MsgObject {
        from: from.clone(),
        to: vec![to.clone()],
        kind: MsgObjKind::Chat,
        to_session: None,
        thread: Default::default(),
        relates_to: None,
        mentions: None,
        workspace: None,
        created_at_ms: created,
        expires_at_ms: None,
        nonce: None,
        content: MsgContent {
            content: "see attached".into(),
            refs: vec![RefItem { role: RefRole::Input, target: RefTarget::DataObj { obj_id: attachment.clone(), uri_hint: None }, label: Some("a.txt".into()) }],
            ..Default::default()
        },
        meta: Default::default(),
    }
}

/// What a local send leaves behind in the sender's Zone: the message and its attachment bound.
async fn sent(a: &Zone, msg: &MsgObject) {
    let (id, json) = msg.gen_obj_id();
    a.center.store_message(&id, &json, msg, crate::buckets::Arrival::Local).await.unwrap();
}

async fn read(c: &MessageCenter, ctx: &RPCContext, id: &ObjId, msg: &ObjId) -> (StatusCode, Vec<u8>) {
    let uri = format!("{}{}/content?context_path={}", crate::object_access::OBJECTS_PATH, id.to_string(), msg.to_string());
    let req: Request<TestBody> = Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {}", ctx.token.as_deref().unwrap()))
        .body(http_body(""))
        .unwrap();
    let response = crate::object_access::serve(c, &req).await.unwrap();
    let status = response.status();
    (status, response.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn attachments() -> BucketId {
    crate::buckets::zone_bucket(buckyos_api::MSG_CENTER_ATTACHMENTS_BUCKET)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recipient_reads_an_attachment_it_never_had() {
    let s = scene().await;
    let data = b"bytes that only exist in the sender's zone";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    let msg = message(&alice(), &bob(), &file, 1);
    let msg_id = msg.gen_obj_id().0;
    sent(&s.a, &msg).await;
    assert!(s.b.center.dispatch_to_receiver(msg.clone(), None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);
    assert!(!s.b.buckets.store.is_object_stored(&file, None).await.unwrap(), "nothing moved with the message");

    let bob_ctx = context(&bob(), ZONE_B.key);
    let (status, body) = read(&s.b.center, &bob_ctx, &file, &msg_id).await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, &data[..]));
    assert!(s.b.buckets.store.is_object_stored(&file, None).await.unwrap());
    let key = format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg));
    assert_eq!(s.b.buckets.client.get(&attachments(), &key, None).await.unwrap().obj_id.as_deref(), Some(file.to_string().as_str()));

    // Bound now: the next read does not go to the sender's zone, and it still reads when that is gone.
    let before = s.hits.lock().unwrap().len();
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.0, StatusCode::OK);
    assert_eq!(s.hits.lock().unwrap().len(), before);
    // Someone who cannot read the message starts nothing.
    let carol_ctx = context(&carol(), ZONE_B.key);
    assert_eq!(read(&s.b.center, &carol_ctx, &file, &msg_id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(s.hits.lock().unwrap().len(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_recipient_gets_a_ticket() {
    let s = scene().await;
    let file = stored_file(&s.a.buckets.store, "a.txt", b"for bob").await;
    let msg = message(&alice(), &bob(), &file, 1);
    let msg_id = msg.gen_obj_id().0;
    sent(&s.a, &msg).await;
    let params = || json!({"msg_id": msg_id.to_string(), "obj_id": file.to_string()});
    let ticket = |who: &DID, params: Value| {
        let ctx = context(who, ZONE_A.key);
        let center = s.a.center.clone();
        let method = buckyos_api::METHOD_MSG_ATTACHMENT_TICKET;
        async move { crate::owner_session::serve_method(method, center.attachment_ticket(params, &ctx)).await }
    };
    let issued = ticket(&bob(), params()).await.unwrap();
    assert!(issued["token"].as_str().unwrap().starts_with("bkt_"));
    assert!(ticket(&carol(), params()).await.is_err(), "not a recipient");
    let other = stored_file(&s.a.buckets.store, "other.txt", b"not in this message").await;
    assert!(ticket(&bob(), json!({"msg_id": msg_id.to_string(), "obj_id": other.to_string()})).await.is_err(), "not referenced");
    let unknown = message(&alice(), &carol(), &file, 2).gen_obj_id().0;
    assert!(ticket(&carol(), json!({"msg_id": unknown.to_string(), "obj_id": file.to_string()})).await.is_err(), "no such message");
    // The ticket works for that key, as `/ndn/` judges it, and nothing else.
    let hint = ndn_lib::NdnHint { path: None, token: Some(issued["token"].as_str().unwrap().to_string()) };
    let url = buckyos_api::ndn_read_url(&s.a.nfs, &file.to_string(), Some(&hint));
    assert_eq!(reqwest::get(&url).await.unwrap().status(), 200);
    let url = buckyos_api::ndn_read_url(&s.a.nfs, &other.to_string(), Some(&hint));
    assert_eq!(reqwest::get(&url).await.unwrap().status(), 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_naming_what_its_sender_lacks_stays_unreadable() {
    let s = scene().await;
    // The private object is here in the receiving Zone; the sender's Zone never had it.
    let secret = stored_file(&s.b.buckets.store, "secret.txt", b"carol's private file").await;
    let msg = message(&alice(), &bob(), &secret, 1);
    let msg_id = msg.gen_obj_id().0;
    assert!(s.b.center.dispatch_to_receiver(msg.clone(), None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);
    let bob_ctx = context(&bob(), ZONE_B.key);
    let (status, _) = read(&s.b.center, &bob_ctx, &secret, &msg_id).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "the reader may retry; nothing was served");
    let key = format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg));
    assert!(s.b.buckets.client.get(&attachments(), &key, None).await.is_err(), "and nothing was bound");
    // Failures back off: the next read does not call the sender's zone again.
    let before = s.hits.lock().unwrap().len();
    assert_eq!(read(&s.b.center, &bob_ctx, &secret, &msg_id).await.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(s.hits.lock().unwrap().len(), before);
    assert_eq!(s.b.center.ensure_remote_attachment(&s.b.buckets, &msg_id, &secret).await, RemoteAttachment::Unavailable);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_message_never_reaches_out() {
    let s = scene().await;
    let file = stored_file(&s.b.buckets.store, "a.txt", b"local").await;
    let msg = message(&carol(), &bob(), &file, 1);
    let msg_id = msg.gen_obj_id().0;
    sent(&s.b, &msg).await;
    let key = format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg));
    s.b.buckets.client.delete(&attachments(), &key, None).await.unwrap();
    assert_eq!(s.b.center.ensure_remote_attachment(&s.b.buckets, &msg_id, &file).await, RemoteAttachment::NotRemote);
    assert!(s.hits.lock().unwrap().is_empty());
}
