//! Zones exchanging messages with attachments (issue #689, bucket design N18): the receiver
//! starts with nothing and ends with a verified binding; only those entitled get a token; what
//! the source does not hold never appears. Private messages between two Zones, then a group
//! with its author, its host and a member's Zone apart.

use crate::buckets::{BucketHost, Buckets};
use crate::group_types::{session_key, GroupActor};
use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use crate::network_directory::test_support::StaticDirectory;
use crate::remote_attachment::RemoteAttachment;
use crate::test_cross_zone::{connect, documents, sign_as, verify_hub, TestZone, ZONE_A, ZONE_B, ZONE_C};
use crate::test_group_service::{http_body, http_fixture, TestBody};
use buckyos_api::bucket_client::BucketId;
use buckyos_api::{
    bind_token_principal_kind, bind_token_target, AccessGroupLevel, AuthTarget, ContactPatch, CrossZoneError, LookupKind, MsgCenterHandler,
    Principal, StaticCustodyResolver, SystemServiceId, TokenPrincipalKind, TokenUse,
};
use http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCContext, RPCSessionToken, RPCSessionTokenType};
use name_lib::DID;
use named_store::NamedDataMgr;
use ndn_lib::{MsgContent, MsgObjKind, MsgObject, MsgRelType, MsgRelation, NamedObject, ObjId, RefItem, RefRole, RefTarget};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tokio::sync::Notify;

fn alice() -> DID {
    DID::new("web", "alice.a.example")
}
fn bob() -> DID {
    DID::new("web", "bob.b.example")
}
fn carol() -> DID {
    DID::new("web", "carol.b.example")
}
fn dana() -> DID {
    DID::new("web", "dana.c.example")
}
fn group() -> DID {
    DID::new("web", "team.c.example")
}
fn alices_agent() -> DID {
    DID::new("web", "helper.a.example")
}
fn bobs_agent() -> DID {
    DID::new("web", "helper.b.example")
}

fn context(who: &DID, key: &str) -> RPCContext {
    session(&who.to_string(), key)
}

/// The sessions of Zone B: with account names as subjects, as a collection needs them, and
/// Bob's Agent.
struct AccountsOfB;

#[async_trait::async_trait]
impl crate::owner_session::SessionTokenVerifier for AccountsOfB {
    async fn authorize(&self, _: &str, _: &str, _: &str) -> Result<(), kRPC::RPCErrors> {
        Ok(())
    }
    async fn verify(&self, raw: &str) -> Result<RPCSessionToken, kRPC::RPCErrors> {
        let mut token = RPCSessionToken::from_string(raw)?;
        token.verify_by_key(&jsonwebtoken::DecodingKey::from_ed_components(ZONE_B.x).unwrap())?;
        Ok(token)
    }
    async fn resolve_user_did(&self, id: &str) -> Result<DID, kRPC::RPCErrors> {
        Ok(if id == "bob" { bob() } else { DID::from_str(id).map_err(|e| kRPC::RPCErrors::ParseRequestError(e.to_string()))? })
    }
    async fn zone_agent(&self, did: &DID) -> Result<Option<crate::owner_session::ZoneAgentInfo>, kRPC::RPCErrors> {
        Ok((*did == bobs_agent()).then(|| crate::owner_session::ZoneAgentInfo { owner: bob(), allow_group: true }))
    }
    async fn sso_settings(&self) -> Result<buckyos_api::SsoSettings, kRPC::RPCErrors> {
        Ok(Default::default())
    }
    async fn authorize_imported_attachment(&self, _: &str, _: &ObjId) -> Result<bool, kRPC::RPCErrors> {
        Ok(false)
    }
}

/// The session of the App an Agent of Zone B runs as.
fn app_session(owner: &DID, app: &str) -> RPCContext {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(buckyos_api::VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(owner.to_string()),
        appid: Some(app.into()),
        sudo: false,
        extra: HashMap::new(),
    };
    bind_token_principal_kind(&mut token, TokenPrincipalKind::App);
    bind_token_target(&mut token, &AuthTarget::App { app_instance_id: format!("{app}@bob").parse().unwrap() }, TokenUse::Session).unwrap();
    RPCContext {
        token: Some(token.generate_jwt(None, &EncodingKey::from_ed_pem(ZONE_B.key.as_bytes()).unwrap()).unwrap()),
        from_ip: Some("127.0.0.1".parse().unwrap()),
        ..Default::default()
    }
}

fn session(subject: &str, key: &str) -> RPCContext {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(buckyos_api::VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(subject.to_string()),
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

async fn zone(zone: &'static TestZone, users: Vec<DID>, docs: Arc<StaticCustodyResolver>) -> Zone {
    let tmp = tempdir().unwrap();
    let db = MsgBoxDbMgr::open_default_sqlite(&format!("sqlite://{}?mode=rwc", tmp.path().join("msg.db").display())).await.unwrap();
    let center = MessageCenter::open_with_db(db).await.unwrap();
    center.cyfs_dispatch.write().unwrap().target_zone = Some(zone.host.into());
    center.register_local_recipients(users);
    center.set_message_hub_did(DID::new("web", "hub.local.example"));
    center.directory.set(StaticDirectory::with(vec![
        (alice(), LookupKind::User, "Alice", "a.example"),
        (bob(), LookupKind::User, "Bob", "b.example"),
        (carol(), LookupKind::User, "Carol", "b.example"),
        (dana(), LookupKind::User, "Dana", "c.example"),
        (group(), LookupKind::Group, "Team", "c.example"),
    ]));
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

/// What reached a Zone from outside, and a gate that holds chunk requests back.
#[derive(Clone, Default)]
struct Wire {
    hits: Arc<std::sync::Mutex<Vec<String>>>,
    hold_chunks: Arc<AtomicBool>,
    release: Arc<Notify>,
}

/// A Zone as seen from outside: verify-hub, msg-center kRPC and group routes, `/ndn/`.
async fn expose(zone: &'static TestZone, source: &Zone, wire: Wire) -> (String, tokio::task::JoinHandle<()>) {
    let center = source.center.clone();
    let nfs = source.nfs.clone();
    http_fixture(move |request: Request<TestBody>| {
        let center = center.clone();
        let nfs = nfs.clone();
        let wire = wire.clone();
        async move {
            let resolver = center.custody.get();
            let mut request = match verify_hub(zone, resolver.as_ref(), request).await {
                Ok(response) => return response,
                Err(request) => request,
            };
            let path = request.uri().path().to_string();
            wire.hits.lock().unwrap().push(path.clone());
            if path.starts_with("/ndn/") {
                if path.contains("mix256") && wire.hold_chunks.load(Ordering::SeqCst) {
                    wire.release.notified().await;
                }
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
            if path != "/kapi/msg-center" {
                return match crate::group_http::serve(&center, &mut request).await {
                    Some(response) => response,
                    None => crate::cyfs_dispatch::serve(&center, request).await,
                };
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
                Err(e) => {
                    let mut answer = json!({"error": e.to_string(), "sys": [call["sys"][0]]});
                    if let Some(detail) = e.detail() {
                        answer["code"] = json!(detail.code);
                    }
                    answer
                }
            };
            Response::builder().status(StatusCode::OK).header("content-type", "application/json").body(http_body(answer.to_string())).unwrap()
        }
    })
    .await
}

struct Scene {
    a: Zone,
    b: Zone,
    docs: Arc<StaticCustodyResolver>,
    wire: Wire,
    hits: Arc<std::sync::Mutex<Vec<String>>>,
    _server: tokio::task::JoinHandle<()>,
}

async fn scene() -> Scene {
    let agent_doc = |agent: DID, owner: DID| json!({"@context": "https://www.buckyos.org/ns/agent", "id": agent, "owner": owner});
    let docs = documents(&[(&ZONE_A, vec![alice()]), (&ZONE_B, vec![bob(), carol()])])
        .with_doc(&alices_agent(), None, agent_doc(alices_agent(), alice()))
        .with_doc(&bobs_agent(), None, agent_doc(bobs_agent(), bob()));
    let docs = Arc::new(docs);
    let a = zone(&ZONE_A, vec![alice()], docs.clone()).await;
    let b = zone(&ZONE_B, vec![bob(), carol()], docs.clone()).await;
    let wire = Wire::default();
    let (origin, server) = expose(&ZONE_A, &a, wire.clone()).await;
    reach(&b, &ZONE_B, &docs, &HashMap::from([(ZONE_A.host.to_string(), origin)]));
    let hits = wire.hits.clone();
    Scene { a, b, docs, wire, hits, _server: server }
}

/// `zone` reaches the others at `origins`, its messages and its pulls alike.
fn reach(zone: &Zone, test_zone: &TestZone, docs: &Arc<StaticCustodyResolver>, origins: &HashMap<String, String>) {
    connect(&zone.center, test_zone, docs, origins.clone());
    zone.center.cyfs_dispatch.write().unwrap().zone_origins = origins.clone();
}

/// The reader takes messages from the sender, as "allow" does in Desktop.
async fn allow(reader: &Zone, owner: &DID, sender: &DID) {
    let patch = ContactPatch { access_level: Some(AccessGroupLevel::Allow), ..Default::default() };
    reader.center.contact_mgr.update_contact(sender.clone(), patch, Some(owner.clone())).await.unwrap();
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

async fn get(c: &MessageCenter, ctx: &RPCContext, uri: String) -> Response<crate::test_group_service::TestBody> {
    let req: Request<TestBody> = Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {}", ctx.token.as_deref().unwrap()))
        .body(http_body(""))
        .unwrap();
    crate::object_access::serve(c, &req).await.unwrap()
}

async fn answer(response: Response<TestBody>) -> (StatusCode, Vec<u8>) {
    let status = response.status();
    (status, response.into_body().collect().await.unwrap().to_bytes().to_vec())
}

async fn read(c: &MessageCenter, ctx: &RPCContext, id: &ObjId, msg: &ObjId) -> (StatusCode, Vec<u8>) {
    let uri = format!("{}{}/content?context_path={}", crate::object_access::OBJECTS_PATH, id.to_string(), msg.to_string());
    answer(get(c, ctx, uri).await).await
}

async fn describe(c: &MessageCenter, ctx: &RPCContext, id: &ObjId, msg: &ObjId) -> (StatusCode, Vec<u8>) {
    let uri = format!("{}{}?context_path={}", crate::object_access::OBJECTS_PATH, id.to_string(), msg.to_string());
    answer(get(c, ctx, uri).await).await
}

async fn is_bound(zone: &Zone, key: &str) -> bool {
    zone.buckets.client.get(&attachments(), key, None).await.is_ok()
}

/// What the source's `msg.attachment_ticket` answers `who`, coming from `from`.
async fn ask_ticket(from: &MessageCenter, zone: &TestZone, who: &DID, params: Value) -> Result<Value, CrossZoneError> {
    let client = from.cross_zone.get().unwrap();
    client.call(&zone.did(), "msg-center", &Principal::new(who.clone()), buckyos_api::METHOD_MSG_ATTACHMENT_TICKET, params).await
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
    allow(&s.b, &bob(), &alice()).await;
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
    allow(&s.b, &bob(), &alice()).await;
    assert!(s.b.center.dispatch_to_receiver(msg.clone(), None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);
    let bob_ctx = context(&bob(), ZONE_B.key);
    // The source says no: not found, not "try again", and not its name either.
    let (status, _) = describe(&s.b.center, &bob_ctx, &secret, &msg_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    s.b.center.remote_fetches.forget_failures();
    let (status, _) = read(&s.b.center, &bob_ctx, &secret, &msg_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nothing was served");
    let key = format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg));
    assert!(s.b.buckets.client.get(&attachments(), &key, None).await.is_err(), "and nothing was bound");
    // Failures back off: the next read does not call the sender's zone again.
    let before = s.hits.lock().unwrap().len();
    assert_eq!(read(&s.b.center, &bob_ctx, &secret, &msg_id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(describe(&s.b.center, &bob_ctx, &secret, &msg_id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(s.hits.lock().unwrap().len(), before);
    assert_eq!(s.b.center.ensure_remote_attachment(&s.b.buckets, &msg_id, &secret, Some(&bob())).await, RemoteAttachment::Refused);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attachment_a_read_could_not_hand_over_is_not_fetched() {
    let s = scene().await;
    // The decision is on the verified object's size, before any chunk.
    let data = b"its object declares far more than this";
    let chunk = ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(data).unwrap();
    s.a.buckets.store.put_chunk(&chunk, data).await.unwrap();
    let (file, body) = ndn_lib::FileObject::new("big.bin".to_string(), crate::object_access::MAX_CONTENT_BYTES + 1, chunk.to_string()).gen_obj_id();
    s.a.buckets.store.put_object(&file, &body).await.unwrap();
    let msg = message(&alice(), &bob(), &file, 1);
    let msg_id = msg.gen_obj_id().0;
    sent(&s.a, &msg).await;
    allow(&s.b, &bob(), &alice()).await;
    assert!(s.b.center.dispatch_to_receiver(msg.clone(), None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);

    let bob_ctx = context(&bob(), ZONE_B.key);
    // Not "try again": the answer will not change, and a second read says the same.
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!s.b.buckets.store.is_object_stored(&file, None).await.unwrap());
    let key = format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg));
    assert!(s.b.buckets.client.get(&attachments(), &key, None).await.is_err(), "nothing was bound");
    let hits = s.hits.lock().unwrap();
    assert!(hits.iter().any(|path| path.starts_with("/ndn/cyfile")), "the object was asked for: {hits:?}");
    assert!(!hits.iter().any(|path| path.contains("mix256")), "and no chunk: {hits:?}");
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
    assert_eq!(s.b.center.ensure_remote_attachment(&s.b.buckets, &msg_id, &file, Some(&bob())).await, RemoteAttachment::NotRemote);
    assert!(s.hits.lock().unwrap().is_empty());
}

/// Alice sent Bob `file`; he has the message, and nothing of the file.
async fn arrived(s: &Scene, file: &ObjId, created: u64) -> (MsgObject, ObjId) {
    let msg = message(&alice(), &bob(), file, created);
    let msg_id = msg.gen_obj_id().0;
    sent(&s.a, &msg).await;
    assert!(s.b.center.dispatch_to_receiver(msg.clone(), None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);
    (msg, msg_id)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stranger_gets_nothing_out_of_a_reader_until_accepted() {
    let s = scene().await;
    let file = stored_file(&s.a.buckets.store, "a.png", b"from someone new").await;
    let (msg, msg_id) = arrived(&s, &file, 1).await;
    let bob_ctx = context(&bob(), ZONE_B.key);

    // Nothing reaches the sender's gateway: no read time, no address.
    for attempt in [describe(&s.b.center, &bob_ctx, &file, &msg_id).await, read(&s.b.center, &bob_ctx, &file, &msg_id).await] {
        assert_eq!(attempt.0, StatusCode::FORBIDDEN);
        assert_eq!(attempt.1, b"attachment-awaiting-acceptance");
    }
    let params = json!({"obj_id": file.to_string(), "context_path": msg_id.to_string()});
    assert!(s.b.center.collect_attachment(params, &context(&bob(), ZONE_B.key)).await.is_err());
    assert!(s.hits.lock().unwrap().is_empty(), "{:?}", s.hits.lock().unwrap());
    let key = format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg));
    assert!(!is_bound(&s.b, &key).await);

    // Accepted: the same message reads, though its record stays in the request box.
    allow(&s.b, &bob(), &alice()).await;
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.1, b"from someone new");
    assert!(is_bound(&s.b, &key).await);

    // Blocked afterwards: what came stays, nothing new goes out.
    let later = stored_file(&s.a.buckets.store, "b.png", b"more from them").await;
    let (_, later_id) = arrived(&s, &later, 2).await;
    s.b.center.contact_mgr.block_contact(alice(), None, Some(bob())).await.unwrap();
    let before = s.hits.lock().unwrap().len();
    assert_eq!(read(&s.b.center, &bob_ctx, &later, &later_id).await.0, StatusCode::FORBIDDEN);
    assert_eq!(s.hits.lock().unwrap().len(), before);
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.0, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_card_asks_for_the_description_only() {
    let s = scene().await;
    let data = b"opened only when clicked";
    let file = stored_file(&s.a.buckets.store, "report.pdf", data).await;
    allow(&s.b, &bob(), &alice()).await;
    let (msg, msg_id) = arrived(&s, &file, 1).await;
    let key = format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg));
    let bob_ctx = context(&bob(), ZONE_B.key);

    let (status, body) = describe(&s.b.center, &bob_ctx, &file, &msg_id).await;
    assert_eq!(status, StatusCode::OK);
    let info: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info["name"], "report.pdf");
    {
        let hits = s.hits.lock().unwrap();
        assert!(hits.iter().any(|path| path.starts_with("/ndn/cyfile")), "{hits:?}");
        assert!(!hits.iter().any(|path| path.contains("mix256")), "no chunk moved: {hits:?}");
    }
    assert!(!is_bound(&s.b, &key).await, "and nothing was bound");
    assert!(!s.b.buckets.store.is_object_stored(&file, None).await.unwrap());

    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.1, data);
    assert!(s.hits.lock().unwrap().iter().any(|path| path.contains("mix256")));
    assert!(is_bound(&s.b, &key).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recipient_the_sender_blocked_gets_no_ticket() {
    let s = scene().await;
    let file = stored_file(&s.a.buckets.store, "a.txt", b"for bob").await;
    let other = stored_file(&s.a.buckets.store, "b.txt", b"for bob too").await;
    allow(&s.b, &bob(), &alice()).await;
    let (_, msg_id) = arrived(&s, &file, 1).await;
    let (_, other_id) = arrived(&s, &other, 2).await;
    let bob_ctx = context(&bob(), ZONE_B.key);
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.0, StatusCode::OK);

    s.a.center.contact_mgr.block_contact(bob(), None, Some(alice())).await.unwrap();
    let asked = ask_ticket(&s.b.center, &ZONE_A, &bob(), json!({"msg_id": other_id.to_string(), "obj_id": other.to_string()})).await.unwrap_err();
    let unknown = message(&alice(), &carol(), &other, 3).gen_obj_id().0;
    let none = ask_ticket(&s.b.center, &ZONE_A, &bob(), json!({"msg_id": unknown.to_string(), "obj_id": other.to_string()})).await.unwrap_err();
    assert_eq!(asked.to_string(), none.to_string(), "the refusal does not tell why");
    assert_eq!(read(&s.b.center, &bob_ctx, &other, &other_id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.0, StatusCode::OK, "what came stays");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declared_size_is_not_believed() {
    let s = scene().await;
    let chunk_of = |data: &[u8]| ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(data).unwrap();
    allow(&s.b, &bob(), &alice()).await;
    let bob_ctx = context(&bob(), ZONE_B.key);

    // Says 10 bytes, its chunks add up to more than a read can hand over.
    let big = vec![7u8; 32 * 1024 * 1024];
    let big_chunk = chunk_of(&big);
    s.a.buckets.store.put_chunk(&big_chunk, &big).await.unwrap();
    let (list, list_body) = ndn_lib::ChunkList::from_chunk_list(vec![big_chunk.clone(), big_chunk.clone(), big_chunk]).unwrap().gen_obj_id();
    s.a.buckets.store.put_object(&list, &list_body).await.unwrap();
    let (forged, body) = ndn_lib::FileObject::new("small.bin".into(), 10, list.to_string()).gen_obj_id();
    s.a.buckets.store.put_object(&forged, &body).await.unwrap();
    let (msg, msg_id) = arrived(&s, &forged, 1).await;
    assert_eq!(read(&s.b.center, &bob_ctx, &forged, &msg_id).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!is_bound(&s.b, &format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg))).await);
    assert!(!s.hits.lock().unwrap().iter().any(|path| path.contains("mix256")), "no chunk was fetched");
    assert!(!s.b.buckets.store.is_object_stored(&forged, None).await.unwrap());

    // Within the limit but not what it says: a verification failure.
    let small = b"not ten bytes at all";
    let small_chunk = chunk_of(small);
    s.a.buckets.store.put_chunk(&small_chunk, small).await.unwrap();
    let (lying, body) = ndn_lib::FileObject::new("lying.bin".into(), 10, small_chunk.to_string()).gen_obj_id();
    s.a.buckets.store.put_object(&lying, &body).await.unwrap();
    let (msg, msg_id) = arrived(&s, &lying, 2).await;
    assert_eq!(read(&s.b.center, &bob_ctx, &lying, &msg_id).await.0, StatusCode::BAD_GATEWAY);
    assert!(!is_bound(&s.b, &format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg))).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_source_that_is_away_is_retried_later() {
    let mut s = scene().await;
    let data = b"worth waiting for";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    allow(&s.b, &bob(), &alice()).await;
    let (_, msg_id) = arrived(&s, &file, 1).await;
    s._server.abort();
    let bob_ctx = context(&bob(), ZONE_B.key);

    for uri in [
        format!("{}{}?context_path={}", crate::object_access::OBJECTS_PATH, file.to_string(), msg_id.to_string()),
        format!("{}{}/content?context_path={}", crate::object_access::OBJECTS_PATH, file.to_string(), msg_id.to_string()),
    ] {
        let response = get(&s.b.center, &bob_ctx, uri).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["retry-after"], "30", "as long as the back-off");
        assert_eq!(answer(response).await.1, b"attachment-source-unreachable");
    }

    let (origin, server) = expose(&ZONE_A, &s.a, s.wire.clone()).await;
    s._server = server;
    reach(&s.b, &ZONE_B, &s.docs, &HashMap::from([(ZONE_A.host.to_string(), origin)]));
    s.b.center.remote_fetches.forget_failures();
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.1, data, "no resend needed");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_readers_download_once() {
    let s = scene().await;
    let data = b"everyone at once";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    allow(&s.b, &bob(), &alice()).await;
    let (_, msg_id) = arrived(&s, &file, 1).await;
    let bob_ctx = context(&bob(), ZONE_B.key);
    let (first, second, third) = tokio::join!(
        read(&s.b.center, &bob_ctx, &file, &msg_id),
        read(&s.b.center, &bob_ctx, &file, &msg_id),
        read(&s.b.center, &bob_ctx, &file, &msg_id),
    );
    for reply in [first, second, third] {
        assert_eq!(reply.1, data);
    }
    assert_eq!(s.hits.lock().unwrap().iter().filter(|path| path.contains("mix256")).count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_answers_that_it_is_not_supported_yet() {
    let s = scene().await;
    let (dir, _) = ndn_lib::DirObject::new(Some("photos".into())).gen_obj_id().unwrap();
    allow(&s.b, &bob(), &alice()).await;
    let msg = message(&alice(), &bob(), &dir, 1);
    let msg_id = msg.gen_obj_id().0;
    assert!(s.b.center.dispatch_to_receiver(msg, None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);
    let bob_ctx = context(&bob(), ZONE_B.key);
    assert_eq!(describe(&s.b.center, &bob_ctx, &dir, &msg_id).await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(read(&s.b.center, &bob_ctx, &dir, &msg_id).await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(s.hits.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn forwarding_and_collecting_an_unread_attachment_fetch_it_first() {
    let s = scene().await;
    let data = b"not read yet";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    allow(&s.b, &bob(), &alice()).await;
    let (msg, msg_id) = arrived(&s, &file, 1).await;
    let bob_ctx = context(&bob(), ZONE_B.key);

    // Bob sends it on without having opened it: the source's rules still decide.
    let onward = message(&bob(), &carol(), &file, 2);
    s.b.center.handle_post_send(onward.clone(), None, bob_ctx.clone()).await.expect("a legitimate forward");
    assert!(is_bound(&s.b, &format!("{}0", MessageCenter::attachment_prefix(&msg_id, &msg))).await);
    assert!(is_bound(&s.b, &format!("{}0", MessageCenter::attachment_prefix(&onward.gen_obj_id().0, &onward))).await);

    // The same for a collection, of another message.
    let other = stored_file(&s.a.buckets.store, "b.txt", b"collected unread").await;
    let (_, other_id) = arrived(&s, &other, 3).await;
    let params = json!({"obj_id": other.to_string(), "context_path": other_id.to_string()});
    s.b.center.set_token_verifier(Arc::new(AccountsOfB));
    let collected = s.b.center.collect_attachment(params, &session("bob", ZONE_B.key)).await.expect("collected");
    let dst = BucketId::user("msg-center", "bob", "collected").unwrap();
    let copy = s.b.buckets.client.get(&dst, collected["key"].as_str().unwrap(), None).await.unwrap();
    assert_eq!(copy.obj_id, Some(other.to_string()));

    // And for saving one into the reader's files.
    let saved_file = stored_file(&s.a.buckets.store, "saved.txt", b"saved unread").await;
    let (saved_msg, saved_id) = arrived(&s, &saved_file, 6).await;
    let params = json!({"obj_id": saved_file.to_string(), "context_path": saved_id.to_string()});
    let saved = s.b.center.prepare_save_attachment(params, &session("bob", ZONE_B.key)).await.expect("prepared");
    assert_eq!(saved, json!({"obj_id": saved_file.to_string(), "kind": "file"}));
    assert!(is_bound(&s.b, &format!("{}0", MessageCenter::attachment_prefix(&saved_id, &saved_msg))).await);

    // A source that is away: the send is refused, and may be tried again.
    let third = stored_file(&s.a.buckets.store, "c.txt", b"later").await;
    let (_, _) = arrived(&s, &third, 4).await;
    s._server.abort();
    let again = message(&bob(), &carol(), &third, 5);
    let refused = s.b.center.handle_post_send(again.clone(), None, bob_ctx.clone()).await.unwrap_err();
    assert_eq!(refused.detail().map(|detail| detail.code.as_str()), Some("attachment_pending"), "{refused}");
    assert!(!is_bound(&s.b, &format!("{}0", MessageCenter::attachment_prefix(&again.gen_obj_id().0, &again))).await);
}

/// N1: a remote sender cannot have a local user reissue a private object of this Zone.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_naming_a_private_object_cannot_be_forwarded_into_it() {
    let s = scene().await;
    let secret = stored_file(&s.b.buckets.store, "secret.txt", b"carol's private file").await;
    allow(&s.b, &bob(), &alice()).await;
    let msg = message(&alice(), &bob(), &secret, 1);
    assert!(s.b.center.dispatch_to_receiver(msg, None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);

    let bob_ctx = context(&bob(), ZONE_B.key);
    let resend = message(&bob(), &carol(), &secret, 2);
    let resend_id = resend.gen_obj_id().0;
    let refused = s.b.center.handle_post_send(resend.clone(), None, bob_ctx.clone()).await.unwrap_err();
    assert!(refused.detail().is_none(), "not one to try again: {refused}");
    assert!(!is_bound(&s.b, &format!("{}0", MessageCenter::attachment_prefix(&resend_id, &resend))).await);
    assert_eq!(read(&s.b.center, &bob_ctx, &secret, &resend_id).await.0, StatusCode::NOT_FOUND);
}

/// A read by object id alone uses the message that has the file, whatever arrived since.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_message_naming_a_file_does_not_take_it_away() {
    let s = scene().await;
    let data = b"already here";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    allow(&s.b, &bob(), &alice()).await;
    let (_, msg_id) = arrived(&s, &file, 1).await;
    let bob_ctx = context(&bob(), ZONE_B.key);
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.1, data);

    let newer = message(&alice(), &bob(), &file, 2);
    assert!(s.b.center.dispatch_to_receiver(newer, None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);
    let before = s.hits.lock().unwrap().len();
    let uri = format!("{}{}/content", crate::object_access::OBJECTS_PATH, file.to_string());
    assert_eq!(answer(get(&s.b.center, &bob_ctx, uri).await).await.1, data);
    assert_eq!(s.hits.lock().unwrap().len(), before, "and nothing went out");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_on_either_side_goes_the_same_way() {
    let s = scene().await;
    s.a.center.register_local_recipients([alices_agent()]);
    s.b.center.register_local_recipients([bobs_agent()]);
    s.b.center.set_token_verifier(Arc::new(AccountsOfB));
    let data = b"from one agent to another";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    let msg = message(&alices_agent(), &bobs_agent(), &file, 1);
    sent(&s.a, &msg).await;
    assert!(s.b.center.dispatch_to_receiver(msg, None, bobs_agent(), "https://b.example/inbox".into()).await.unwrap().ok);

    // Its runtime reads by object id alone; the policy that counts is the Agent's.
    let runtime = app_session(&bob(), "helper.b.example");
    let uri = format!("{}{}/content", crate::object_access::OBJECTS_PATH, file.to_string());
    assert_eq!(answer(get(&s.b.center, &runtime, uri.clone()).await).await.0, StatusCode::FORBIDDEN);
    allow(&s.b, &bobs_agent(), &alices_agent()).await;
    assert_eq!(answer(get(&s.b.center, &runtime, uri).await).await.1, data);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_brings_its_attachments_by_its_own_message() {
    let s = scene().await;
    let first = stored_file(&s.a.buckets.store, "first.txt", b"first version").await;
    let second = stored_file(&s.a.buckets.store, "second.txt", b"second version").await;
    allow(&s.b, &bob(), &alice()).await;
    let (_, original_id) = arrived(&s, &first, 1).await;
    let mut edit = message(&alice(), &bob(), &second, 2);
    edit.relates_to = Some(MsgRelation { rel: MsgRelType::Edit, target: original_id.clone(), key: None });
    let edit_id = edit.gen_obj_id().0;
    sent(&s.a, &edit).await;
    assert!(s.b.center.dispatch_to_receiver(edit, None, bob(), "https://b.example/inbox".into()).await.unwrap().ok);
    let bob_ctx = context(&bob(), ZONE_B.key);
    assert_eq!(read(&s.b.center, &bob_ctx, &second, &edit_id).await.1, b"second version");
    assert_eq!(read(&s.b.center, &bob_ctx, &first, &original_id).await.1, b"first version");
    assert_eq!(read(&s.b.center, &bob_ctx, &first, &edit_id).await.0, StatusCode::NOT_FOUND, "only through the message that names it");
}

// ── a group: its author in Zone A, its host in Zone C, a member's Zone B ─────

struct GroupScene {
    a: Zone,
    b: Zone,
    c: Zone,
    docs: Arc<StaticCustodyResolver>,
    origins: HashMap<String, String>,
    wire_a: Wire,
    wire_c: Wire,
    server_a: tokio::task::JoinHandle<()>,
    _server_c: tokio::task::JoinHandle<()>,
    _server_b: tokio::task::JoinHandle<()>,
}

fn group_document(owner: &DID) -> Value {
    json!({"@context": ["https://www.w3.org/ns/did/v1", "https://buckyos.org/ns/group/v1"], "id": group(),
        "entity_type": "group", "host": "did:web:c.example", "controller": owner, "owner": owner})
}

async fn rpc(center: &MessageCenter, zone: &TestZone, who: &DID, method: &str, mut params: Value) -> Result<Value, kRPC::RPCErrors> {
    params["group_did"] = json!(group());
    center.group_rpc(method, params, context(who, zone.key)).await
}

async fn group_scene() -> GroupScene {
    let docs = documents(&[(&ZONE_A, vec![alice()]), (&ZONE_B, vec![bob(), carol()]), (&ZONE_C, vec![dana()])])
        .with_doc(&group(), None, group_document(&dana()));
    let docs = Arc::new(docs);
    let a = zone(&ZONE_A, vec![alice()], docs.clone()).await;
    let b = zone(&ZONE_B, vec![bob(), carol()], docs.clone()).await;
    let c = zone(&ZONE_C, vec![dana()], docs.clone()).await;
    let (wire_a, wire_c) = (Wire::default(), Wire::default());
    let (origin_a, server_a) = expose(&ZONE_A, &a, wire_a.clone()).await;
    let (origin_b, server_b) = expose(&ZONE_B, &b, Wire::default()).await;
    let (origin_c, server_c) = expose(&ZONE_C, &c, wire_c.clone()).await;
    let origins = HashMap::from([(ZONE_A.host.to_string(), origin_a), (ZONE_B.host.to_string(), origin_b), (ZONE_C.host.to_string(), origin_c)]);
    reach(&a, &ZONE_A, &docs, &origins);
    reach(&b, &ZONE_B, &docs, &origins);
    reach(&c, &ZONE_C, &docs, &origins);
    rpc(&c.center, &ZONE_C, &dana(), "group.create", json!({"idempotency_key": "create", "configuration": {"default_session": {"history": "all"}}})).await.unwrap();
    for (center, zone, who) in [(&a.center, &ZONE_A, alice()), (&b.center, &ZONE_B, bob()), (&b.center, &ZONE_B, carol())] {
        let invitation = rpc(&c.center, &ZONE_C, &dana(), "group.invite_member", json!({"member_did": who})).await.unwrap();
        rpc(center, zone, &who, "group.accept_invitation", json!({"invitation_id": invitation["invite_id"]})).await.unwrap();
    }
    GroupScene { a, b, c, docs, origins, wire_a, wire_c, server_a, _server_c: server_c, _server_b: server_b }
}

/// What a member's Zone does to post: signs in at the host for the member, puts the signed message.
async fn put_to_group_inbox(from: &MessageCenter, zone: &TestZone, who: &DID, msg: &MsgObject) -> reqwest::Response {
    let signer = zone.signer();
    let jwt = msg.to_jwt(signer.key(), &signer.kid).unwrap();
    let request = buckyos_api::CrossZoneRequest::new(http::Method::PUT, format!("/kapi/msg-center/groups/{}/inbox", group().to_string()))
        .header("content-type", ndn_lib::CYFS_CONTENT_TYPE_NAMED_OBJECT_JWT)
        .header(ndn_lib::CYFS_HEADER_OBJ_ID, msg.gen_obj_id().0.to_string())
        .body(jwt);
    from.cross_zone.get().unwrap().request(&ZONE_C.did(), "msg-center", &Principal::new(who.clone()), request).await.unwrap()
}

fn group_message(from: &DID, attachment: Option<&ObjId>, created: u64) -> MsgObject {
    let mut msg = MsgObject { from: from.clone(), to: vec![group()], kind: MsgObjKind::GroupMsg, created_at_ms: created, nonce: Some(created), ..Default::default() };
    msg.content.content = "for the group".into();
    if let Some(id) = attachment {
        msg.content.refs.push(RefItem { role: RefRole::Input, target: RefTarget::DataObj { obj_id: id.clone(), uri_hint: None }, label: Some("a.txt".into()) });
    }
    msg
}

impl GroupScene {
    /// Alice posts from her Zone: bound there as a local send, then put to the host's inbox.
    async fn alice_posts(&self, msg: &MsgObject) {
        let (id, json) = msg.gen_obj_id();
        self.a.center.store_message(&id, &json, msg, crate::buckets::Arrival::Local).await.unwrap();
        let answer = put_to_group_inbox(&self.a.center, &ZONE_A, &alice(), msg).await;
        assert_eq!(answer.status(), 200, "{}", answer.text().await.unwrap());
    }

    async fn sync_member(&self, who: &DID) {
        rpc(&self.b.center, &ZONE_B, who, "group.sync_joined", json!({})).await.unwrap();
    }

    fn key(msg: &MsgObject) -> String {
        format!("{}0", MessageCenter::attachment_prefix(&msg.gen_obj_id().0, msg))
    }

    async fn host_has(&self, msg: &MsgObject) {
        for _ in 0..200 {
            if is_bound(&self.c, &Self::key(msg)).await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the host never got the attachment");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_attachment_travels_author_host_member() {
    let s = group_scene().await;
    let data = b"the group's file";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    let msg = group_message(&alice(), Some(&file), 1);
    let msg_id = msg.gen_obj_id().0;
    s.alice_posts(&msg).await;

    // The host fetched it on its own, as the group; the author can leave now.
    s.host_has(&msg).await;
    assert!(s.wire_a.hits.lock().unwrap().iter().any(|path| path.contains("mix256")));
    s.server_a.abort();
    let context_path = format!("{}/{}", session_key(&group(), None).unwrap(), msg_id.to_string());
    let hosted = format!("{}{}/content?context_path={}", crate::object_access::OBJECTS_PATH, file.to_string(), context_path.replace('/', "%2F"));
    let (status, body) = answer(get(&s.c.center, &context(&dana(), ZONE_C.key), hosted).await).await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, &data[..]), "the host reads what it fetched");

    // A member's Zone: its local mailbox decides, then the host; it never looks for the author.
    s.sync_member(&bob()).await;
    let bob_ctx = context(&bob(), ZONE_B.key);
    let (status, body) = describe(&s.b.center, &bob_ctx, &file, &msg_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["name"], "a.txt");
    assert!(!s.wire_c.hits.lock().unwrap().iter().any(|path| path.contains("mix256")), "a card moves no chunk");
    assert!(!is_bound(&s.b, &GroupScene::key(&msg)).await);
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.1, data);
    assert!(is_bound(&s.b, &GroupScene::key(&msg)).await);

    // Who is not in the group gets nothing from the host, however it asks.
    let params = json!({"obj_id": file.to_string(), "context_path": context_path});
    assert!(ask_ticket(&s.b.center, &ZONE_C, &bob(), params.clone()).await.is_ok(), "a member does");
    let outsider = DID::new("web", "erin.b.example");
    let not_member = ask_ticket(&s.b.center, &ZONE_C, &outsider, params.clone()).await;
    assert!(not_member.is_err());
    let by_message = json!({"obj_id": file.to_string(), "msg_id": msg_id.to_string()});
    assert!(ask_ticket(&s.b.center, &ZONE_C, &bob(), by_message).await.is_err(), "by the message alone only the group itself may");

    // A removed member: no new tickets; what came stays readable here.
    rpc(&s.c.center, &ZONE_C, &dana(), "group.remove_member", json!({"member_did": bob()})).await.unwrap();
    assert!(ask_ticket(&s.b.center, &ZONE_C, &bob(), params).await.is_err());
    assert_eq!(read(&s.b.center, &bob_ctx, &file, &msg_id).await.1, data);
}

/// A group's message comes from the group: what a member thinks of its author, or the author of
/// the member, decides nothing. Nobody in this scene is anybody's contact to begin with.
#[tokio::test(flavor = "multi_thread")]
async fn a_member_who_blocked_the_author_still_gets_it_from_the_group() {
    let s = group_scene().await;
    s.b.center.contact_mgr.block_contact(alice(), None, Some(bob())).await.unwrap();
    s.c.center.contact_mgr.block_contact(alice(), None, Some(dana())).await.unwrap();
    s.a.center.contact_mgr.block_contact(carol(), None, Some(alice())).await.unwrap();

    let data = b"for everyone in the group";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    let msg = group_message(&alice(), Some(&file), 1);
    let msg_id = msg.gen_obj_id().0;
    s.alice_posts(&msg).await;
    s.host_has(&msg).await;

    let received = |records: &[buckyos_api::MailboxRecord], who: &DID| records.iter().any(|r| &r.owner == who && r.box_kind == buckyos_api::MailboxKind::Inbox);
    let at_host = s.c.center.msg_box_db.list_message_records(&msg_id).await.unwrap();
    assert!(received(&at_host, &dana()), "a member at the host who blocked the author");
    let context_path = format!("{}/{}", session_key(&group(), None).unwrap(), msg_id.to_string());
    let hosted = format!("{}{}/content?context_path={}", crate::object_access::OBJECTS_PATH, file.to_string(), context_path.replace('/', "%2F"));
    let (status, body) = answer(get(&s.c.center, &context(&dana(), ZONE_C.key), hosted).await).await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, &data[..]));

    s.sync_member(&bob()).await;
    s.sync_member(&carol()).await;
    let at_member = s.b.center.msg_box_db.list_message_records(&msg_id).await.unwrap();
    assert!(received(&at_member, &bob()), "a member elsewhere who blocked the author");
    assert!(received(&at_member, &carol()), "a member the author blocked");
    for who in [bob(), carol()] {
        let ctx = context(&who, ZONE_B.key);
        assert_eq!(describe(&s.b.center, &ctx, &file, &msg_id).await.0, StatusCode::OK);
        assert_eq!(read(&s.b.center, &ctx, &file, &msg_id).await.1, data);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_missed_the_author_catches_up_on_a_read() {
    let mut s = group_scene().await;
    s.server_a.abort();
    let data = b"posted and gone";
    let file = stored_file(&s.a.buckets.store, "a.txt", data).await;
    let msg = group_message(&alice(), Some(&file), 1);
    let msg_id = msg.gen_obj_id().0;
    s.alice_posts(&msg).await;
    s.sync_member(&bob()).await;

    // The member's read goes through the host, which goes to the author: not there.
    let bob_ctx = context(&bob(), ZONE_B.key);
    let response = get(&s.b.center, &bob_ctx, format!("{}{}/content?context_path={}", crate::object_access::OBJECTS_PATH, file.to_string(), msg_id.to_string())).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key("retry-after"));

    // The author is back; the same read gets it, no resend.
    let (origin, server) = expose(&ZONE_A, &s.a, s.wire_a.clone()).await;
    s.server_a = server;
    s.origins.insert(ZONE_A.host.to_string(), origin);
    reach(&s.c, &ZONE_C, &s.docs, &s.origins);
    s.b.center.remote_fetches.forget_failures();
    s.c.center.remote_fetches.forget_failures();
    let mut got = None;
    for _ in 0..200 {
        let reply = read(&s.b.center, &bob_ctx, &file, &msg_id).await;
        if reply.0 == StatusCode::OK {
            got = Some(reply.1);
            break;
        }
        assert_eq!(reply.0, StatusCode::SERVICE_UNAVAILABLE);
        s.b.center.remote_fetches.forget_failures();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(got.as_deref(), Some(&data[..]));
    assert!(is_bound(&s.c, &GroupScene::key(&msg)).await, "and the host kept its copy");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recall_during_the_pull_is_not_undone() {
    let s = group_scene().await;
    let file = stored_file(&s.a.buckets.store, "a.txt", b"recalled in flight").await;
    s.wire_a.hold_chunks.store(true, Ordering::SeqCst);
    let msg = group_message(&alice(), Some(&file), 1);
    let msg_id = msg.gen_obj_id().0;
    s.alice_posts(&msg).await;
    // The host's background pull is at the chunk now, held back.
    for _ in 0..400 {
        if s.wire_a.hits.lock().unwrap().iter().any(|path| path.contains("mix256")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(s.wire_a.hits.lock().unwrap().iter().any(|path| path.contains("mix256")));

    let mut recall = group_message(&dana(), None, 2);
    recall.relates_to = Some(MsgRelation { rel: MsgRelType::Redact, target: msg_id.clone(), key: None });
    let owner = GroupActor { did: dana(), client: Some("messagehub".into()), remote: false };
    assert!(s.c.center.accept_group_message(&owner, recall, None).await.unwrap().ok);
    s.c.center.purge_group_objects().await.unwrap();

    s.wire_a.hold_chunks.store(false, Ordering::SeqCst);
    s.wire_a.release.notify_one();
    // However long the pull takes, the key stays empty and no read revives it.
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert!(!is_bound(&s.c, &GroupScene::key(&msg)).await);
    }
    assert_eq!(s.c.center.ensure_remote_attachment(&s.c.buckets, &msg_id, &file, None).await, RemoteAttachment::NotRemote);
    assert!(!is_bound(&s.c, &GroupScene::key(&msg)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joined_member_reads_what_a_display_shows() {
    let s = group_scene().await;
    let data = b"house rules";
    let rules = stored_file(&s.c.buckets.store, "rules.pdf", data).await;
    let menu = stored_file(&s.c.buckets.store, "menu.pdf", b"menu").await;
    let hidden = stored_file(&s.c.buckets.store, "hidden.pdf", b"not shown").await;
    let revision = rpc(&s.c.center, &ZONE_C, &dana(), "group.get_config", json!({})).await.unwrap()["revision"].clone();
    let patch = json!({"expected_revision": revision, "idempotency_key": "rules", "patch": {"display": {"files": [{"title": "Rules", "obj_id": rules}]}}});
    let applied = rpc(&s.c.center, &ZONE_C, &dana(), "group.apply_config", patch).await.unwrap();
    s.sync_member(&bob()).await;

    let session = session_key(&group(), None).unwrap();
    let context_path = format!("{session}/_display").replace('/', "%2F");
    let bob_ctx = context(&bob(), ZONE_B.key);
    let uri = |id: &ObjId, content: bool| format!("{}{}{}?context_path={context_path}", crate::object_access::OBJECTS_PATH, id.to_string(), if content { "/content" } else { "" });
    let (status, body) = answer(get(&s.b.center, &bob_ctx, uri(&rules, false)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["name"], "rules.pdf");
    assert_eq!(answer(get(&s.b.center, &bob_ctx, uri(&rules, true)).await).await.1, data);
    let key = format!("{}{}", crate::buckets::display_prefix(&group()).unwrap(), rules.to_string());
    assert!(is_bound(&s.b, &key).await);
    // What no display shows, and who is not a member here, get nothing.
    assert_eq!(answer(get(&s.b.center, &bob_ctx, uri(&hidden, true)).await).await.0, StatusCode::NOT_FOUND);
    let outsider = context(&DID::new("web", "erin.b.example"), ZONE_B.key);
    assert_eq!(answer(get(&s.b.center, &outsider, uri(&rules, true)).await).await.0, StatusCode::NOT_FOUND);

    // A token issued before the change dies with the host's binding.
    let params = json!({"obj_id": rules.to_string(), "context_path": format!("{session}/_display")});
    let ticket = ask_ticket(&s.b.center, &ZONE_C, &bob(), params.clone()).await.unwrap();
    let hint = ndn_lib::NdnHint { path: None, token: Some(ticket["token"].as_str().unwrap().to_string()) };
    let url = buckyos_api::ndn_read_url(&s.c.nfs, &rules.to_string(), Some(&hint));
    assert_eq!(reqwest::get(&url).await.unwrap().status(), 200);
    let patch = json!({"expected_revision": applied["revision"], "idempotency_key": "menu", "patch": {"display": {"files": [{"title": "Menu", "obj_id": menu}]}}});
    rpc(&s.c.center, &ZONE_C, &dana(), "group.apply_config", patch).await.unwrap();
    assert_eq!(reqwest::get(&url).await.unwrap().status(), 404);
    assert!(ask_ticket(&s.b.center, &ZONE_C, &bob(), params).await.is_err());
}

/// The host tells a member elsewhere as the group: past the contact policy of the member's
/// Zone, into the session of the group, for a group this Zone knows the member is in.
#[tokio::test(flavor = "multi_thread")]
async fn a_member_elsewhere_hears_from_the_group_whoever_acted() {
    use crate::msg_tunnel::DeliveryExecutor;
    let s = group_scene().await;
    s.b.center.contact_mgr.block_contact(dana(), None, Some(bob())).await.unwrap();
    rpc(&s.c.center, &ZONE_C, &dana(), "group.remove_member", json!({"member_did": bob()})).await.unwrap();

    let transport = s.c.center.message_hub_did().unwrap();
    let hub = crate::message_hub::MessageHubExecutor::new(transport.clone(), s.c.center.clone());
    let from_group = |target: DID| {
        let (center, transport) = (s.c.center.clone(), transport.clone());
        async move {
            loop {
                let queued = center.msg_box_db.take_next_delivery(&transport, MessageCenter::now_ms() + 1000, true).await.unwrap().unwrap();
                let msg = center.load_message(&queued.envelope.msg_id).await.unwrap();
                if queued.envelope.target_did == target && msg.from == group() {
                    return (queued, msg);
                }
            }
        }
    };
    let (queued, notice) = from_group(bob()).await;
    assert_eq!(serde_json::to_value(&notice.content.machine.as_ref().unwrap().data).unwrap()["by"], json!(dana()));
    let notice_id = queued.envelope.msg_id.clone();
    let report = hub.execute_delivery(buckyos_api::DeliveryRecordWithObject { record: queued, msg: Some(notice.clone()) }).await.unwrap();
    assert!(report.ok, "{report:?}");

    let records = s.b.center.msg_box_db.list_message_records(&notice_id).await.unwrap();
    let record = records.iter().find(|r| r.owner == bob()).unwrap();
    assert_eq!(record.box_kind, buckyos_api::MailboxKind::Inbox);
    assert_eq!(record.session_id, Some(session_key(&group(), None).unwrap()));
    assert!(record.tags.contains(&format!("group:{}", group().to_string())));

    // Blocking the group itself is what stops it.
    s.b.center.contact_mgr.block_contact(group(), None, Some(carol())).await.unwrap();
    rpc(&s.c.center, &ZONE_C, &dana(), "group.remove_member", json!({"member_did": carol()})).await.unwrap();
    let (queued, msg) = from_group(carol()).await;
    let id = queued.envelope.msg_id.clone();
    let report = hub.execute_delivery(buckyos_api::DeliveryRecordWithObject { record: queued, msg: Some(msg) }).await.unwrap();
    assert!(!report.ok, "{report:?}");
    assert!(s.b.center.msg_box_db.list_message_records(&id).await.unwrap().is_empty());

    // Not one of its own, or not really the group speaking: the contact policy as usual.
    let outsider = DID::new("web", "erin.b.example");
    assert!(!s.b.center.is_notice_of_own_group(&notice, &outsider).await);
    let mut other = notice.clone();
    other.from = alice();
    assert!(!s.b.center.is_notice_of_own_group(&other, &bob()).await);
    let mut chat = notice.clone();
    chat.kind = MsgObjKind::Chat;
    assert!(!s.b.center.is_notice_of_own_group(&chat, &bob()).await);
    assert!(s.b.center.is_notice_of_own_group(&notice, &bob()).await);
}

/// A member's Zone posts media for the member; nobody else gets that way in.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_members_zone_posts_attachments_to_the_host() {
    let s = group_scene().await;
    let file = stored_file(&s.a.buckets.store, "a.txt", b"for the group").await;
    let msg = group_message(&alice(), Some(&file), 1);
    s.alice_posts(&msg).await;
    s.host_has(&msg).await;

    // Not in the group: its Zone signs in, and finds no such group.
    let erin = DID::new("web", "erin.b.example");
    let docs = documents(&[(&ZONE_A, vec![alice()]), (&ZONE_B, vec![bob(), carol(), erin.clone()]), (&ZONE_C, vec![dana()])]).with_doc(&group(), None, group_document(&dana()));
    let docs = Arc::new(docs);
    reach(&s.b, &ZONE_B, &docs, &s.origins);
    s.c.center.custody.set(docs.clone());
    let visiting = group_message(&erin, Some(&file), 2);
    let answer = put_to_group_inbox(&s.b.center, &ZONE_B, &erin, &visiting).await;
    assert_eq!(answer.status(), 404, "{}", answer.text().await.unwrap());
    assert!(!is_bound(&s.c, &GroupScene::key(&visiting)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_host_speaks_for_the_group() {
    let s = group_scene().await;
    let file = stored_file(&s.a.buckets.store, "a.txt", b"for the group").await;
    let msg = group_message(&alice(), Some(&file), 1);
    let msg_id = msg.gen_obj_id().0;
    let (id, json_body) = msg.gen_obj_id();
    s.a.center.store_message(&id, &json_body, &msg, crate::buckets::Arrival::Local).await.unwrap();
    let params = json!({"msg_id": msg_id.to_string(), "obj_id": file.to_string()});

    let issued = ask_ticket(&s.c.center, &ZONE_A, &group(), params.clone()).await.expect("the host's device");
    assert!(issued["token"].as_str().unwrap().starts_with("bkt_"));
    let other_zone = ask_ticket(&s.b.center, &ZONE_A, &group(), params.clone()).await.unwrap_err();
    assert!(matches!(other_zone, CrossZoneError::Auth { .. }), "{other_zone}");
    // A member of the group, asking as itself, is not the group.
    assert!(ask_ticket(&s.b.center, &ZONE_A, &bob(), params).await.is_err());

    // The owner's Zone is the host: an owner in another Zone would move the group.
    let across = rpc(&s.c.center, &ZONE_C, &dana(), "group.transfer_owner", json!({"member_did": bob()})).await.unwrap_err();
    assert!(across.to_string().contains("owner-must-be-in-the-host-zone"), "{across}");
    // Admins are another matter: a role on the host, wherever they live.
    rpc(&s.c.center, &ZONE_C, &dana(), "group.update_member_role", json!({"member_did": bob(), "role": "admin"})).await.unwrap();
}
