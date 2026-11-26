use crate::group_types::{session_key, MemberStatus};
use crate::message_hub::MessageHubExecutor;
use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use crate::msg_tunnel::DeliveryExecutor;
use crate::network_directory::test_support::{GroupDocumentSource, StaticDirectory};
use crate::test_cross_zone::{
    connect, documents, sign_as, verify_hub, TestZone, ZONE_A, ZONE_B, ZONE_C,
};
use crate::test_group_service::{http_body, http_fixture};
use buckyos_api::StaticCustodyResolver;
use buckyos_api::{
    bind_token_principal_kind, bind_token_target, AuthTarget, DeliveryRecordWithObject,
    DeliveryState, LookupKind, MailboxAddress, MailboxKind, MsgCenterHandler, SystemServiceId,
    TokenPrincipalKind, TokenUse,
};
use http::{HeaderMap, Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCContext, RPCErrors, RPCSessionToken, RPCSessionTokenType};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject, NamedObject};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::{tempdir, TempDir};

const HOST_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const LOCAL_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIF5zplRSY5MsYbBMdbzItH90daUlrx/OflPm8Kp9m2j1\n-----END PRIVATE KEY-----";

fn user(name: &str, zone: &str) -> DID {
    DID::new("web", &format!("{name}.{zone}"))
}
fn alice() -> DID {
    user("alice", "a.example")
}
fn helper() -> DID {
    user("helper", "a.example")
}
fn bob() -> DID {
    user("bob", "b.example")
}
fn carol() -> DID {
    user("carol", "b.example")
}
fn dora() -> DID {
    user("dora", "c.example")
}
fn other_group() -> DID {
    DID::new("web", "social.a.example")
}
fn group() -> DID {
    DID::new("web", "support.a.example")
}
fn hub() -> DID {
    DID::new("web", "hub.local.example")
}

fn context(who: &DID) -> RPCContext {
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
    bind_token_target(
        &mut token,
        &AuthTarget::system(SystemServiceId::parse("msg-center").unwrap()),
        TokenUse::Session,
    )
    .unwrap();
    let key = if who.id.ends_with(".a.example") {
        HOST_KEY
    } else if who.id.ends_with(".c.example") {
        ZONE_C.key
    } else {
        LOCAL_KEY
    };
    RPCContext {
        token: Some(
            token
                .generate_jwt(None, &EncodingKey::from_ed_pem(key.as_bytes()).unwrap())
                .unwrap(),
        ),
        from_ip: Some("127.0.0.1".parse().unwrap()),
        ..Default::default()
    }
}

fn zone_documents() -> Arc<StaticCustodyResolver> {
    let mut resolver = documents(&[
        (&ZONE_A, vec![alice(), helper()]),
        (&ZONE_B, vec![bob(), carol()]),
        (&ZONE_C, vec![dora()]),
    ]);
    for zone in [ZONE_A, ZONE_B, ZONE_C] {
        resolver = resolver.with_doc(&zone.did(), Some(name_lib::DidDocType::Owner), json!({
            "@context":"https://www.buckyos.org/ns/owner", "id":zone.did(), "binded_zone_list":[]
        }));
    }
    Arc::new(resolver)
}

async fn open(path: &str, host: bool, directory: Arc<StaticDirectory>) -> MessageCenter {
    let db = MsgBoxDbMgr::open_default_sqlite(path).await.unwrap();
    let center = MessageCenter::open_with_db(db).await.unwrap();
    let (zone, users) = if host {
        (&ZONE_A, vec![alice(), helper()])
    } else {
        (&ZONE_B, vec![bob(), carol()])
    };
    center.cyfs_dispatch.write().unwrap().target_zone = Some(zone.host.into());
    center.register_local_recipients(users);
    center.set_message_hub_did(hub());
    center.directory.set(directory);
    sign_as(&center, zone, zone_documents());
    center
}

fn reconnect(local: &MessageCenter, host_url: &str) {
    connect(
        local,
        &ZONE_B,
        &zone_documents(),
        HashMap::from([("a.example".to_string(), host_url.to_string())]),
    );
}

#[derive(Default)]
struct Faults {
    requests: Mutex<Vec<String>>,
    pushes: Mutex<Vec<CapturedPush>>,
    drop_push: AtomicBool,
    block_push: AtomicBool,
    release_push: tokio::sync::Notify,
    timeout_push: AtomicBool,
    hold_object: AtomicBool,
    release_object: tokio::sync::Notify,
    lose_join_reply: AtomicBool,
    fail_changes: AtomicBool,
}

#[derive(Clone)]
struct CapturedPush {
    path: String,
    headers: HeaderMap,
    body: Value,
}

impl CapturedPush {
    async fn replay(&self, receiver: &MessageCenter) -> StatusCode {
        let mut request = Request::post(&self.path)
            .body(http_body(self.body.to_string()))
            .unwrap();
        *request.headers_mut() = self.headers.clone();
        crate::group_http::serve(receiver, &mut request)
            .await
            .unwrap()
            .status()
    }
}

impl Faults {
    fn pulls(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.contains(path))
            .count()
    }

    fn clear_traffic(&self) {
        self.requests.lock().unwrap().clear();
        self.pushes.lock().unwrap().clear();
    }
}

struct TwoZones {
    host: MessageCenter,
    local: MessageCenter,
    local_path: String,
    host_url: String,
    local_url: String,
    directory: Arc<StaticDirectory>,
    faults: Arc<Faults>,
    _dirs: (TempDir, TempDir),
    servers: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for TwoZones {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}

async fn serve_zone(
    center: MessageCenter,
    zone: &'static TestZone,
    faults: Arc<Faults>,
) -> (String, tokio::task::JoinHandle<()>) {
    http_fixture(move |request| {
        let center = center.clone();
        let faults = faults.clone();
        async move {
            let resolver = center.custody.get();
            let mut request = match verify_hub(zone, resolver.as_ref(), request).await {
                Ok(response) => return response,
                Err(request) => request,
            };
            let path = request.uri().path().to_string();
            faults.requests.lock().unwrap().push(path.clone());
            if path.ends_with("/updates") {
                let (parts, body) = request.into_parts();
                let bytes = body.collect().await.unwrap().to_bytes();
                faults.pushes.lock().unwrap().push(CapturedPush {
                    path: parts.uri.to_string(),
                    headers: parts.headers.clone(),
                    body: serde_json::from_slice(&bytes).unwrap(),
                });
                if faults.block_push.load(Ordering::SeqCst) {
                    faults.release_push.notified().await;
                }
                if faults.timeout_push.swap(false, Ordering::SeqCst) {
                    return Response::builder()
                        .status(StatusCode::GATEWAY_TIMEOUT)
                        .body(http_body("{}"))
                        .unwrap();
                }
                if faults.drop_push.load(Ordering::SeqCst) {
                    return Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .body(http_body("{}"))
                        .unwrap();
                }
                request = Request::from_parts(parts, http_body(bytes));
            }
            if path.contains("/objects/") && faults.hold_object.swap(false, Ordering::SeqCst) {
                faults.release_object.notified().await;
            }
            let is_join =
                request.method() == Method::PUT && request.uri().path().ends_with("/join");
            if request.uri().path().ends_with("/changes")
                && faults.fail_changes.swap(false, Ordering::SeqCst)
            {
                return Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(http_body(
                        r#"{"error":"temporary-sync-failure","retryable":true}"#,
                    ))
                    .unwrap();
            }
            let response =
                if let Some(response) = crate::group_http::serve(&center, &mut request).await {
                    response
                } else {
                    crate::cyfs_dispatch::serve(&center, request).await
                };
            if is_join
                && response.status().is_success()
                && faults.lose_join_reply.swap(false, Ordering::SeqCst)
            {
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(http_body(r#"{"error":"lost-join-reply","retryable":true}"#))
                    .unwrap()
            } else {
                response
            }
        }
    })
    .await
}

async fn two_zones() -> TwoZones {
    let host_dir = tempdir().unwrap();
    let local_dir = tempdir().unwrap();
    let host_path = format!(
        "sqlite://{}?mode=rwc",
        host_dir.path().join("host.db").display()
    );
    let local_path = format!(
        "sqlite://{}?mode=rwc",
        local_dir.path().join("local.db").display()
    );
    let directory = StaticDirectory::with(vec![
        (alice(), LookupKind::User, "Alice", "a.example"),
        (helper(), LookupKind::User, "Helper", "a.example"),
        (bob(), LookupKind::User, "Bob", "b.example"),
        (carol(), LookupKind::User, "Carol", "b.example"),
        (group(), LookupKind::Group, "Support", "a.example"),
        (other_group(), LookupKind::Group, "Social", "a.example"),
        (dora(), LookupKind::User, "Dora", "c.example"),
    ]);
    let host = open(&host_path, true, directory.clone()).await;
    let local = open(&local_path, false, directory.clone()).await;
    let faults = Arc::new(Faults::default());
    let (host_url, host_server) = serve_zone(host.clone(), &ZONE_A, faults.clone()).await;
    let (local_url, local_server) = serve_zone(local.clone(), &ZONE_B, faults.clone()).await;
    connect(
        &host,
        &ZONE_A,
        &zone_documents(),
        HashMap::from([("b.example".to_string(), local_url.clone())]),
    );
    reconnect(&local, &host_url);
    TwoZones {
        host,
        local,
        local_path,
        host_url,
        local_url,
        directory,
        faults,
        _dirs: (host_dir, local_dir),
        servers: vec![host_server, local_server],
    }
}

async fn rpc(
    center: &MessageCenter,
    who: &DID,
    method: &str,
    mut params: Value,
) -> Result<Value, RPCErrors> {
    if params.get("group_did").is_none() {
        params["group_did"] = json!(group());
    }
    center.group_rpc(method, params, context(who)).await
}
async fn create(zones: &TwoZones) -> Value {
    rpc(
        &zones.host,
        &alice(),
        "group.create",
        json!({
            "idempotency_key":"create", "configuration":{"default_session":{"history":"all"}}
        }),
    )
    .await
    .unwrap()
}
async fn invite(zones: &TwoZones, inviter: &DID, member: &DID) -> Value {
    rpc(
        &zones.host,
        inviter,
        "group.invite_member",
        json!({"member_did":member}),
    )
    .await
    .unwrap()
}
async fn accept(center: &MessageCenter, who: &DID, invitation: &Value) -> Result<Value, RPCErrors> {
    rpc(
        center,
        who,
        "group.accept_invitation",
        json!({"invitation_id":invitation["invite_id"]}),
    )
    .await
}

#[tokio::test]
async fn accepting_an_invitation_recovers_from_unpublished_group_negative_caches() {
    let zones = two_zones().await;
    create(&zones).await;
    let invitation = invite(&zones, &alice(), &bob()).await;
    pump_one(&zones.host).await;
    let source = Arc::new(GroupDocumentSource::default());
    let (directory, client) = source.directory();
    zones.local.directory.set(directory);

    assert!(matches!(
        rpc(&zones.local, &bob(), "group.get_doc", json!({})).await,
        Err(RPCErrors::NoPermission(reason)) if reason == "remote-group-not-found"
    ));
    assert!(matches!(
        accept(&zones.local, &bob(), &invitation).await,
        Err(RPCErrors::NoPermission(reason)) if reason == "remote-group-not-found"
    ));
    assert!(list(&zones.local, &bob()).await["joined"]
        .as_array()
        .unwrap()
        .is_empty());
    let state = zones.host.groups.load(&group()).await.unwrap().unwrap();
    assert_eq!(
        state.members[&bob().to_string()].state,
        MemberStatus::Invited
    );
    *source.document.lock().unwrap() = Some(state.public_doc().unwrap());
    source.calls.lock().unwrap().clear();

    assert!(client.resolve_did(&group(), None).await.is_err());
    assert!(rpc(&zones.local, &bob(), "group.get_doc", json!({}))
        .await
        .is_err());
    assert!(source.calls.lock().unwrap().is_empty());
    let accepted = accept(&zones.local, &bob(), &invitation).await.unwrap();
    assert_eq!(*source.calls.lock().unwrap(), vec![true]);
    assert_eq!(accepted["state"], "active");
    assert_eq!(accepted["sync_pending"], false);
    assert!(matches!(
        accept(&zones.local, &bob(), &json!({"invite_id":"wrong-invitation"})).await,
        Err(RPCErrors::NoPermission(reason)) if reason == "invitation-mismatch"
    ));
    assert_eq!(
        zones
            .host
            .groups
            .load(&group())
            .await
            .unwrap()
            .unwrap()
            .members[&bob().to_string()]
            .state,
        MemberStatus::Active
    );
    let seq = zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .group_seq;
    assert_eq!(
        accept(&zones.local, &bob(), &invitation).await.unwrap(),
        accepted
    );
    assert_eq!(
        zones
            .host
            .groups
            .load(&group())
            .await
            .unwrap()
            .unwrap()
            .group_seq,
        seq
    );
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["group_did"],
        json!(group())
    );
    assert!(rpc(&zones.local, &bob(), "group.get_doc", json!({}))
        .await
        .is_ok());

    let incoming = host_message(&zones.host, "after publication", 1).await;
    sync(&zones.local, &bob()).await;
    assert!(projected(&zones.local, &bob()).await.contains(&incoming));
    let outgoing = message(&bob(), "joined after refresh", 2);
    zones
        .local
        .handle_post_send(outgoing.clone(), None, context(&bob()))
        .await
        .unwrap();
    pump_one(&zones.local).await;
    assert!(zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .messages
        .contains_key(&outgoing.gen_obj_id().0.to_string()));
}

async fn sync(center: &MessageCenter, who: &DID) -> Value {
    rpc(center, who, "group.sync_joined", json!({}))
        .await
        .unwrap()
}
async fn list(center: &MessageCenter, who: &DID) -> Value {
    rpc(center, who, "group.list_by_member", json!({}))
        .await
        .unwrap()
}
async fn pump_one(center: &MessageCenter) {
    let record = center
        .msg_box_db
        .take_next_delivery(&hub(), u64::MAX / 2, true)
        .await
        .unwrap()
        .unwrap();
    let msg = center.load_message(&record.envelope.msg_id).await.unwrap();
    let id = record.delivery_id.clone();
    let report = MessageHubExecutor::new(hub(), center.clone())
        .execute_delivery(DeliveryRecordWithObject {
            record,
            msg: Some(msg),
        })
        .await
        .unwrap();
    assert!(report.ok, "{report:?}");
    center
        .handle_report_delivery(id.clone(), report, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(
        center
            .msg_box_db
            .get_delivery(&id)
            .await
            .unwrap()
            .unwrap()
            .state,
        DeliveryState::Sent
    );
}
fn message(who: &DID, text: &str, nonce: u64) -> MsgObject {
    let mut msg = MsgObject {
        from: who.clone(),
        to: vec![group()],
        kind: MsgObjKind::GroupMsg,
        created_at_ms: nonce,
        nonce: Some(nonce),
        ..Default::default()
    };
    msg.content.content = text.into();
    msg
}
async fn host_message(center: &MessageCenter, text: &str, nonce: u64) -> MsgObject {
    let msg = message(&alice(), text, nonce);
    let result = center
        .handle_post_send(msg.clone(), None, context(&alice()))
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    msg
}
async fn projected(center: &MessageCenter, who: &DID) -> Vec<MsgObject> {
    center
        .handle_list_session(
            who.clone(),
            session_key(&group(), None).unwrap(),
            Some(100),
            None,
            None,
            Some(false),
            Some(true),
            context(who),
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| item.msg)
        .collect()
}

#[tokio::test]
async fn a_remote_member_first_syncs_the_configured_earlier_messages() {
    let zones = two_zones().await;
    rpc(&zones.host, &alice(), "group.create", json!({
        "idempotency_key":"create", "configuration":{"default_session":{"history_before_join":1}}
    }))
    .await
    .unwrap();
    let older = host_message(&zones.host, "older", 1).await;
    let latest = host_message(&zones.host, "latest", 2).await;
    let invitation = invite(&zones, &alice(), &bob()).await;
    pump_one(&zones.host).await;
    assert_eq!(accept(&zones.local, &bob(), &invitation).await.unwrap()["sync_pending"], false);
    let seen = projected(&zones.local, &bob()).await;
    assert!(seen.contains(&latest));
    assert!(!seen.contains(&older));
}

#[tokio::test]
async fn cross_zone_invitation_delivers_joins_syncs_sends_and_survives_restart() {
    let mut zones = two_zones().await;
    create(&zones).await;
    let history = host_message(&zones.host, "before invitation", 1).await;
    let invitation = invite(&zones, &alice(), &bob()).await;
    pump_one(&zones.host).await;
    let notice_mailbox =
        MailboxAddress::new(bob(), Some(format!("dm:{}", alice().to_string()))).unwrap();
    let mut notices = vec![];
    for kind in [MailboxKind::Inbox, MailboxKind::RequestBox] {
        notices.extend(
            zones
                .local
                .msg_box_db
                .list_records(&notice_mailbox, &kind, None, false)
                .await
                .unwrap(),
        );
    }
    assert!(!notices.is_empty());
    let notice = zones.local.load_message(&notices[0].msg_id).await.unwrap();
    let notice = serde_json::to_value(notice.content.machine.unwrap().data).unwrap();
    assert_eq!(notice["data"]["invite_id"], invitation["invite_id"]);
    let prior_seq = zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .group_seq;
    let member = bob();
    let (accepted, repeated) = tokio::join!(
        accept(&zones.local, &member, &invitation),
        accept(&zones.local, &member, &invitation),
    );
    let accepted = accepted.unwrap();
    assert_eq!(accepted, repeated.unwrap());
    assert_eq!(
        zones
            .host
            .groups
            .load(&group())
            .await
            .unwrap()
            .unwrap()
            .group_seq,
        prior_seq + 1
    );
    assert_eq!(accepted["state"], "active");
    assert_eq!(accepted["sync_pending"], false);
    assert_eq!(
        zones
            .host
            .groups
            .load(&group())
            .await
            .unwrap()
            .unwrap()
            .members[&bob().to_string()]
            .state,
        MemberStatus::Active
    );
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["group_did"],
        json!(group())
    );
    let sessions = zones
        .local
        .handle_list_sessions(bob(), None, None, None, None, None, None, context(&bob()))
        .await
        .unwrap();
    assert!(sessions
        .items
        .iter()
        .any(|item| item.session_id == session_key(&group(), None).unwrap()));
    assert!(projected(&zones.local, &bob()).await.contains(&history));
    let fresh = host_message(&zones.host, "after joining", 2).await;
    sync(&zones.local, &bob()).await;
    assert!(projected(&zones.local, &bob()).await.contains(&fresh));
    let outbound = message(&bob(), "reply from remote member", 3);
    zones
        .local
        .handle_post_send(outbound.clone(), None, context(&bob()))
        .await
        .unwrap();
    pump_one(&zones.local).await;
    assert!(zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .messages
        .contains_key(&outbound.gen_obj_id().0.to_string()));
    assert!(list(&zones.local, &carol()).await["joined"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(rpc(&zones.local, &carol(), "group.sync_joined", json!({}))
        .await
        .is_err());

    zones.local = open(&zones.local_path, false, zones.directory.clone()).await;
    reconnect(&zones.local, &zones.host_url);
    let after_restart = host_message(&zones.host, "after restart", 4).await;
    sync(&zones.local, &bob()).await;
    assert!(projected(&zones.local, &bob())
        .await
        .contains(&after_restart));
    assert_eq!(
        list(&zones.local, &bob()).await["joined"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let reply = message(&bob(), "route restored", 5);
    zones
        .local
        .handle_post_send(reply.clone(), None, context(&bob()))
        .await
        .unwrap();
    pump_one(&zones.local).await;
    assert!(zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .messages
        .contains_key(&reply.gen_obj_id().0.to_string()));
    let invitation = invite(&zones, &alice(), &carol()).await;
    accept(&zones.local, &carol(), &invitation).await.unwrap();
    let peer_history = projected(&zones.local, &carol()).await;
    assert!(peer_history.contains(&outbound));
    assert!(peer_history.contains(&reply));
}

#[tokio::test]
async fn cross_zone_accept_recovers_lost_response_and_first_sync_failure() {
    let mut zones = two_zones().await;
    create(&zones).await;
    let invitation = invite(&zones, &alice(), &bob()).await;
    zones.faults.lose_join_reply.store(true, Ordering::SeqCst);
    assert!(accept(&zones.local, &bob(), &invitation).await.is_err());
    let state = zones.host.groups.load(&group()).await.unwrap().unwrap();
    assert_eq!(
        state.members[&bob().to_string()].state,
        MemberStatus::Active
    );
    let seq = state.group_seq;
    zones.local = open(&zones.local_path, false, zones.directory.clone()).await;
    reconnect(&zones.local, &zones.host_url);
    zones.local.sync_remote_groups().await.unwrap();
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["group_did"],
        json!(group())
    );
    assert_eq!(
        zones
            .host
            .groups
            .load(&group())
            .await
            .unwrap()
            .unwrap()
            .group_seq,
        seq
    );
    assert_eq!(
        accept(&zones.local, &bob(), &invitation).await.unwrap()["state"],
        "active"
    );

    let invitation = invite(&zones, &alice(), &carol()).await;
    zones.faults.fail_changes.store(true, Ordering::SeqCst);
    let accepted = accept(&zones.local, &carol(), &invitation).await.unwrap();
    assert_eq!(accepted["state"], "active");
    assert_eq!(accepted["sync_pending"], true);
    let seq = zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .group_seq;
    let retried = accept(&zones.local, &carol(), &invitation).await.unwrap();
    assert_eq!(retried["sync_pending"], false);
    assert_eq!(
        zones
            .host
            .groups
            .load(&group())
            .await
            .unwrap()
            .unwrap()
            .group_seq,
        seq
    );
    assert_eq!(
        list(&zones.local, &carol()).await["joined"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn cross_zone_invites_enforce_identity_lifetime_and_admin_approval() {
    let zones = two_zones().await;
    let created = create(&zones).await;
    let first = invite(&zones, &alice(), &bob()).await;
    assert!(accept(&zones.local, &carol(), &first).await.is_err());
    assert!(rpc(
        &zones.local,
        &carol(),
        "group.accept_invitation",
        json!({
            "invitation_id":first["invite_id"],"member_did":bob()
        })
    )
    .await
    .is_err());
    rpc(
        &zones.host,
        &alice(),
        "group.revoke_invite",
        json!({"member_did":bob()}),
    )
    .await
    .unwrap();
    assert!(accept(&zones.local, &bob(), &first).await.is_err());
    let second = invite(&zones, &alice(), &bob()).await;
    assert!(accept(&zones.local, &bob(), &first).await.is_err());
    let mut tx = zones.host.groups.begin(&group()).await.unwrap();
    tx.state
        .members
        .get_mut(&bob().to_string())
        .unwrap()
        .expires_at_ms = Some(MessageCenter::now_ms() - 1);
    tx.commit().await.unwrap();
    assert!(accept(&zones.local, &bob(), &second).await.is_err());
    assert!(list(&zones.local, &bob()).await["joined"]
        .as_array()
        .unwrap()
        .is_empty());
    rpc(
        &zones.host,
        &alice(),
        "group.apply_config",
        json!({
            "expected_revision":created["revision"],"idempotency_key":"member-invites",
            "patch":{"roles":{"member":["session.post","session.read","group.invite_member"]}}
        }),
    )
    .await
    .unwrap();
    let invitation = invite(&zones, &alice(), &helper()).await;
    accept(&zones.host, &helper(), &invitation).await.unwrap();
    let pending = invite(&zones, &helper(), &bob()).await;
    let accepted = accept(&zones.local, &bob(), &pending).await.unwrap();
    assert_eq!(accepted["state"], "pending_admin_approval");
    assert!(projected(&zones.local, &bob()).await.is_empty());
    rpc(
        &zones.host,
        &alice(),
        "group.approve_member",
        json!({"member_did":bob()}),
    )
    .await
    .unwrap();
    sync(&zones.local, &bob()).await;
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["group_did"],
        json!(group())
    );
    assert_eq!(
        accept(&zones.local, &bob(), &pending).await.unwrap()["state"],
        "active"
    );
}

#[tokio::test]
async fn cross_zone_removed_member_can_rejoin_without_replaying_old_removal() {
    let zones = two_zones().await;
    create(&zones).await;
    let first = invite(&zones, &alice(), &bob()).await;
    accept(&zones.local, &bob(), &first).await.unwrap();
    rpc(
        &zones.host,
        &alice(),
        "group.remove_member",
        json!({"member_did":bob()}),
    )
    .await
    .unwrap();
    sync(&zones.local, &bob()).await;
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["stopped"],
        true
    );
    assert!(accept(&zones.local, &bob(), &first).await.is_err());
    let second = invite(&zones, &alice(), &bob()).await;
    let joined = accept(&zones.local, &bob(), &second).await.unwrap();
    assert_eq!(joined["state"], "active");
    assert_eq!(joined["epoch"], 2);
    assert_eq!(joined["sync_pending"], false);
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["stopped"],
        false
    );
    let fresh = host_message(&zones.host, "new membership", 20).await;
    sync(&zones.local, &bob()).await;
    assert!(projected(&zones.local, &bob()).await.contains(&fresh));
    let outbound = message(&bob(), "new membership reply", 21);
    zones
        .local
        .handle_post_send(outbound.clone(), None, context(&bob()))
        .await
        .unwrap();
    pump_one(&zones.local).await;
    assert!(zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .messages
        .contains_key(&outbound.gen_obj_id().0.to_string()));
}

#[tokio::test]
async fn stale_invitation_cannot_erase_recovery_of_a_confirmed_invitation() {
    let mut zones = two_zones().await;
    create(&zones).await;
    let history = host_message(&zones.host, "recover accepted membership", 30).await;
    let stale = invite(&zones, &alice(), &bob()).await;
    let current = invite(&zones, &alice(), &bob()).await;
    assert_ne!(stale["invite_id"], current["invite_id"]);
    zones.faults.lose_join_reply.store(true, Ordering::SeqCst);
    assert!(accept(&zones.local, &bob(), &current).await.is_err());
    let state = zones.host.groups.load(&group()).await.unwrap().unwrap();
    let member = &state.members[&bob().to_string()];
    assert_eq!(member.state, MemberStatus::Active);
    assert_eq!(
        member.invitation_id.as_deref(),
        current["invite_id"].as_str()
    );
    let seq = state.group_seq;
    let epoch = member.epoch;

    zones.faults.fail_changes.store(true, Ordering::SeqCst);
    assert!(accept(&zones.local, &bob(), &stale).await.is_err());
    assert!(list(&zones.local, &bob()).await["joined"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(projected(&zones.local, &bob()).await.is_empty());
    zones.local = open(&zones.local_path, false, zones.directory.clone()).await;
    reconnect(&zones.local, &zones.host_url);
    zones.local.sync_remote_groups().await.unwrap();
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["group_did"],
        json!(group())
    );
    assert!(projected(&zones.local, &bob()).await.contains(&history));
    assert!(list(&zones.local, &carol()).await["joined"]
        .as_array()
        .unwrap()
        .is_empty());
    let recovered = accept(&zones.local, &bob(), &current).await.unwrap();
    assert_eq!(recovered["state"], "active");
    assert_eq!(recovered["epoch"], epoch);
    assert_eq!(
        zones
            .host
            .groups
            .load(&group())
            .await
            .unwrap()
            .unwrap()
            .group_seq,
        seq
    );
}

#[tokio::test]
async fn cross_zone_member_reads_members_and_participants_through_its_zone() {
    let zones = two_zones().await;
    create(&zones).await;
    let invitation = invite(&zones, &alice(), &bob()).await;
    accept(&zones.local, &bob(), &invitation).await.unwrap();
    rpc(
        &zones.host,
        &alice(),
        "group.create_session",
        json!({"session_id":"team / 客服"}),
    )
    .await
    .unwrap();
    let dids = |value: &Value, field: &str| {
        let mut out: Vec<String> = value["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item[field].as_str().unwrap().to_owned())
            .collect();
        out.sort();
        out
    };
    let mut everyone = vec![alice().to_string(), bob().to_string()];
    everyone.sort();
    let members = rpc(&zones.local, &bob(), "group.list_members", json!({}))
        .await
        .unwrap();
    assert_eq!(dids(&members, "member_did"), everyone);
    let default = rpc(
        &zones.local,
        &bob(),
        "group.list_session_members",
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(default["complete"], true);
    assert_eq!(dids(&default, "member_did"), everyone);
    let named = rpc(
        &zones.local,
        &bob(),
        "group.list_session_members",
        json!({"session_id":"team / 客服"}),
    )
    .await
    .unwrap();
    assert_eq!(dids(&named, "member_did"), everyone);
    assert!(rpc(&zones.local, &carol(), "group.list_members", json!({}))
        .await
        .is_err());
    rpc(
        &zones.host,
        &alice(),
        "group.remove_member",
        json!({"member_did":bob()}),
    )
    .await
    .unwrap();
    assert!(rpc(&zones.local, &bob(), "group.list_members", json!({}))
        .await
        .is_err());
}

async fn join_for_push(zones: &TwoZones, reader: &DID) {
    let invitation = invite(zones, &alice(), reader).await;
    let result = accept(&zones.local, reader, &invitation).await.unwrap();
    assert_eq!(result["sync_pending"], false);
}

async fn inbox(
    center: &MessageCenter,
    reader: &DID,
    session: Option<&str>,
) -> Vec<buckyos_api::MailboxRecord> {
    let key = session_key(&group(), session).unwrap();
    let mailbox = MailboxAddress::new(reader.clone(), Some(key)).unwrap();
    center
        .msg_box_db
        .list_records(&mailbox, &MailboxKind::Inbox, None, false)
        .await
        .unwrap()
}

async fn wait_inbox(
    center: &MessageCenter,
    reader: &DID,
    message: &MsgObject,
) -> buckyos_api::MailboxRecord {
    let id = message.gen_obj_id().0;
    let key = session_key(&message.to[0], message.to_session.as_deref()).unwrap();
    let address = MailboxAddress::new(reader.clone(), Some(key)).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(record) = center
                .msg_box_db
                .list_records(&address, &MailboxKind::Inbox, None, false)
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.msg_id == id)
            {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("missing projection of {id} for {}", reader.to_string()))
}

async fn settle(faults: &Faults) {
    let mut before = faults.requests.lock().unwrap().len();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let now = faults.requests.lock().unwrap().len();
        if now == before {
            break;
        }
        before = now;
    }
}

fn assert_no_pull(faults: &Faults) {
    for path in ["/changes", "/sessions", "/objects/"] {
        assert_eq!(faults.pulls(path), 0, "unexpected host request for {path}");
    }
}

async fn post_in(center: &MessageCenter, session: &str, text: &str, nonce: u64) -> MsgObject {
    let mut msg = message(&alice(), text, nonce);
    msg.to_session = Some(session.into());
    let result = center
        .handle_post_send(msg.clone(), None, context(&alice()))
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    msg
}

#[tokio::test]
async fn cross_zone_push_commits_two_successive_messages_without_host_reads_and_preserves_read_state(
) {
    let zones = two_zones().await;
    create(&zones).await;
    join_for_push(&zones, &bob()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();

    // This federated account can receive local mail even when its cached profile is stale.
    zones.host.register_local_recipients([bob()]);
    assert!(zones.host.is_hosted_identity(&bob()));
    zones
        .directory
        .0
        .lock()
        .unwrap()
        .get_mut(&bob().to_string())
        .unwrap()
        .zone = Some(ZONE_C.host.into());
    let first = host_message(&zones.host, "first push", 100).await;
    let record = wait_inbox(&zones.local, &bob(), &first).await;
    let second = host_message(&zones.host, "next push", 99).await;
    let second_record = wait_inbox(&zones.local, &bob(), &second).await;
    assert_no_pull(&zones.faults);
    let pushes = zones.faults.pushes.lock().unwrap().clone();
    assert_eq!(pushes.len(), 2);
    let delta = |n: usize| pushes[n].body["entries"][0]["delta"].clone();
    assert!(delta(0)["previous_token"].is_string());
    assert_eq!(delta(0)["next_token"], delta(1)["previous_token"]);
    assert_eq!(
        delta(1)["change"]["message"]["obj_id"],
        json!(second.gen_obj_id().0)
    );
    let host = zones.host.groups.load(&group()).await.unwrap().unwrap();
    assert_eq!(
        second_record.sort_key,
        host.messages[&second.gen_obj_id().0.to_string()].accepted_at_ms
    );

    zones
        .local
        .handle_update_record_state(
            record.record_id.clone(),
            buckyos_api::RecipientState::Read,
            context(&bob()),
        )
        .await
        .unwrap();
    let committed = zones.local.joined_groups(&bob()).await.unwrap()[0].clone();
    assert!(pushes[1].replay(&zones.local).await.is_success());
    assert_no_pull(&zones.faults);
    assert!(pushes[0].replay(&zones.local).await.is_success());
    settle(&zones.faults).await;
    let after = zones.local.joined_groups(&bob()).await.unwrap()[0].clone();
    assert_eq!(after.changes_token, committed.changes_token);
    assert_no_pull(&zones.faults);
    assert_eq!(after.session_seqs, committed.session_seqs);
    assert_eq!(after.last_read_seq, committed.last_read_seq);
    let copies: Vec<_> = inbox(&zones.local, &bob(), None)
        .await
        .into_iter()
        .filter(|r| r.msg_id == first.gen_obj_id().0)
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].state, buckyos_api::RecipientState::Read);
}

#[tokio::test]
async fn replay_after_32_newer_pushes_preserves_read_and_cursor_without_a_host_request() {
    let zones = two_zones().await;
    create(&zones).await;
    rpc(
        &zones.host,
        &alice(),
        "group.create_session",
        json!({"session_id":"other"}),
    )
    .await
    .unwrap();
    join_for_push(&zones, &bob()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();

    let first = host_message(&zones.host, "delayed replay", 700).await;
    let record = wait_inbox(&zones.local, &bob(), &first).await;
    let captured = zones.faults.pushes.lock().unwrap()[0].clone();
    for n in 0..32 {
        let next = if n % 2 == 0 {
            host_message(&zones.host, "newer push", 701 + n).await
        } else {
            post_in(&zones.host, "other", "newer push", 701 + n).await
        };
        wait_inbox(&zones.local, &bob(), &next).await;
    }
    assert_eq!(zones.faults.pushes.lock().unwrap().len(), 33);
    assert_no_pull(&zones.faults);
    zones
        .local
        .handle_update_record_state(
            record.record_id.clone(),
            buckyos_api::RecipientState::Read,
            context(&bob()),
        )
        .await
        .unwrap();
    let before = zones.local.joined_groups(&bob()).await.unwrap().remove(0);
    let first_token = captured.body["entries"][0]["delta"]["next_token"]
        .as_str()
        .unwrap();
    assert!(!before.recent_push_tokens.iter().any(|t| t == first_token));
    assert!(captured.replay(&zones.local).await.is_success());
    settle(&zones.faults).await;
    assert_no_pull(&zones.faults);
    let after = zones.local.joined_groups(&bob()).await.unwrap().remove(0);
    assert_eq!(after.changes_token, before.changes_token);
    assert_eq!(after.session_seqs, before.session_seqs);
    assert_eq!(after.last_read_seq, before.last_read_seq);
    let copies: Vec<_> = inbox(&zones.local, &bob(), None)
        .await
        .into_iter()
        .filter(|r| r.msg_id == record.msg_id)
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].state, buckyos_api::RecipientState::Read);

    // The shared object exists, but it wasn't committed at this other position.
    let mut mismatched = captured;
    let seq = &mut mismatched.body["entries"][0]["delta"]["change"]["message"]["seq"];
    *seq = json!(seq.as_u64().unwrap() + 1);
    assert!(mismatched.replay(&zones.local).await.is_success());
    settle(&zones.faults).await;
    assert_eq!(zones.faults.pulls("/changes"), 1);
}

#[tokio::test]
async fn one_push_after_a_disconnect_drains_over_one_hundred_visible_changes() {
    let zones = two_zones().await;
    create(&zones).await;
    for i in 0..6 {
        rpc(
            &zones.host,
            &alice(),
            "group.create_session",
            json!({"session_id":format!("room{i}")}),
        )
        .await
        .unwrap();
    }
    join_for_push(&zones, &bob()).await;
    settle(&zones.faults).await;
    zones.faults.drop_push.store(true, Ordering::SeqCst);
    let mut messages = vec![];
    for i in 0..100 {
        messages.push(
            post_in(
                &zones.host,
                &format!("room{}", i % 6),
                "offline backlog",
                1000 + i,
            )
            .await,
        );
    }
    settle(&zones.faults).await;
    assert!(!zones.faults.pushes.lock().unwrap().is_empty());
    assert!(!inbox(&zones.local, &bob(), Some("room0"))
        .await
        .iter()
        .any(|r| r.msg_id == messages[0].gen_obj_id().0));
    zones.faults.clear_traffic();
    zones.faults.drop_push.store(false, Ordering::SeqCst);
    let last = post_in(&zones.host, "room0", "last before silence", 1100).await;
    wait_inbox(&zones.local, &bob(), &last).await;
    assert_eq!(zones.faults.pushes.lock().unwrap().len(), 1);
    assert!(zones.faults.pulls("/changes") >= 2);
    for room in 0..6 {
        let sid = format!("room{room}");
        let rows = inbox(&zones.local, &bob(), Some(&sid)).await;
        for msg in messages
            .iter()
            .filter(|m| m.to_session.as_deref() == Some(&sid))
        {
            assert_eq!(
                rows.iter()
                    .filter(|r| r.msg_id == msg.gen_obj_id().0)
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn cross_zone_push_authorizes_each_reader_before_sending_a_private_message() {
    let zones = two_zones().await;
    create(&zones).await;
    join_for_push(&zones, &bob()).await;
    join_for_push(&zones, &carol()).await;
    rpc(
        &zones.host,
        &alice(),
        "group.create_session",
        json!({
            "session_id":"private","membership":"explicit","members":[bob()]
        }),
    )
    .await
    .unwrap();
    settle(&zones.faults).await;
    zones.faults.clear_traffic();
    let private = post_in(&zones.host, "private", "body for Bob only", 331).await;
    wait_inbox(&zones.local, &bob(), &private).await;
    let pushes = zones.faults.pushes.lock().unwrap().clone();
    let deltas: Vec<_> = pushes
        .iter()
        .flat_map(|p| p.body["entries"].as_array().unwrap())
        .filter(|e| !e["delta"].is_null())
        .collect();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0]["reader_did"], json!(bob()));
    assert_eq!(
        deltas[0]["delta"]["change"]["message"]["obj_id"],
        json!(private.gen_obj_id().0)
    );
    assert!(inbox(&zones.local, &carol(), Some("private"))
        .await
        .is_empty());
}

#[tokio::test]
async fn compensation_recovers_the_last_dropped_push_and_expired_cursor_after_restart() {
    let mut zones = two_zones().await;
    create(&zones).await;
    join_for_push(&zones, &bob()).await;
    join_for_push(&zones, &carol()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();
    zones.host.groups.push.saturate_discovery();
    let before_drop = zones.host.groups.push.dropped.load(Ordering::Relaxed);
    let msg = host_message(&zones.host, "last dropped notification", 555).await;
    assert_eq!(
        zones.host.groups.push.dropped.load(Ordering::Relaxed),
        before_drop + 1
    );
    assert!(zones.faults.pushes.lock().unwrap().is_empty());
    assert_no_pull(&zones.faults);
    let token = zones.local.joined_groups(&bob()).await.unwrap()[0]
        .changes_token
        .clone()
        .unwrap();
    sqlx::query("UPDATE group_sync_tokens SET created_at_ms=0 WHERE token=?")
        .bind(token)
        .execute(zones.host.msg_box_db.pool())
        .await
        .unwrap();

    for server in zones.servers.drain(..) {
        server.abort();
    }
    let host_path = format!(
        "sqlite://{}?mode=rwc",
        zones._dirs.0.path().join("host.db").display()
    );
    zones.host = open(&host_path, true, zones.directory.clone()).await;
    zones.local = open(&zones.local_path, false, zones.directory.clone()).await;
    let (host_url, host_server) =
        serve_zone(zones.host.clone(), &ZONE_A, zones.faults.clone()).await;
    let (local_url, local_server) =
        serve_zone(zones.local.clone(), &ZONE_B, zones.faults.clone()).await;
    connect(
        &zones.host,
        &ZONE_A,
        &zone_documents(),
        HashMap::from([("b.example".into(), local_url)]),
    );
    reconnect(&zones.local, &host_url);
    zones.servers.extend([host_server, local_server]);
    zones.faults.clear_traffic();
    let route = zones
        .local
        .load_group_binding(&carol(), &group())
        .await
        .unwrap()
        .unwrap()
        .route(&zones.local);
    sqlx::query("DELETE FROM group_joined_bindings WHERE owner=? AND group_did=?")
        .bind(carol().to_string())
        .bind(group().to_string())
        .execute(zones.local.msg_box_db.pool())
        .await
        .unwrap();
    zones
        .local
        .cyfs_dispatch
        .write()
        .unwrap()
        .joined_groups
        .push(route);
    let (sleeps, mut timers) = tokio::sync::mpsc::unbounded_channel();
    zones
        .local
        .start_group_sync_with_clock(Duration::from_secs(900), move |delay| {
            let (tick, timer) = tokio::sync::oneshot::channel::<()>();
            sleeps.send((delay, tick)).unwrap();
            async move {
                timer.await.unwrap();
            }
        });
    let mut ticks = vec![];
    for _ in 0..2 {
        let (delay, tick) = tokio::time::timeout(Duration::from_secs(5), timers.recv())
            .await
            .unwrap()
            .unwrap();
        assert!((Duration::from_secs(810)..=Duration::from_secs(900)).contains(&delay));
        ticks.push(tick);
        println!(
            "joined-group compensation timer armed for {} ms",
            delay.as_millis()
        );
    }
    assert_no_pull(&zones.faults);
    assert!(zones
        .local
        .groups
        .object(&msg.gen_obj_id().0)
        .await
        .unwrap()
        .is_none());
    for tick in ticks {
        tick.send(()).unwrap();
    }
    wait_inbox(&zones.local, &bob(), &msg).await;
    wait_inbox(&zones.local, &carol(), &msg).await;
    assert!(zones.faults.pulls("/inbox") > 0);
    assert!(zones.faults.pulls("/changes") > 0);
    assert_eq!(zones.faults.pushes.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn invalid_push_entries_leave_the_cursor_untouched_and_old_epoch_replays_are_ignored() {
    let zones = two_zones().await;
    create(&zones).await;
    join_for_push(&zones, &bob()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();
    zones.faults.drop_push.store(true, Ordering::SeqCst);
    let msg = host_message(&zones.host, "pending valid delta", 645).await;
    settle(&zones.faults).await;
    let captured = zones.faults.pushes.lock().unwrap().last().unwrap().clone();
    assert!(!captured.body["entries"][0]["delta"].is_null());
    let old = zones.local.joined_groups(&bob()).await.unwrap()[0]
        .changes_token
        .clone();

    for mutation in [
        "source",
        "child-source",
        "local-session",
        "reader",
        "epoch",
        "object-id",
        "session",
        "object",
    ] {
        let mut invalid = captured.clone();
        let entry = &mut invalid.body["entries"][0];
        match mutation {
            "local-session" => {
                let token = context(&ZONE_A.did()).token.unwrap();
                invalid
                    .headers
                    .insert("authorization", format!("Bearer {token}").parse().unwrap());
            }
            "source" | "child-source" => {
                let subject = if mutation == "source" {
                    "did:web:b.example"
                } else {
                    "did:web:a.example:child"
                };
                let token = crate::test_cross_zone::mint(
                    &ZONE_B,
                    subject,
                    &buckyos_api::SessionCeiling::restricted([buckyos_api::SessionScope::MsgGroup]),
                    TokenUse::Session,
                );
                invalid
                    .headers
                    .insert("authorization", format!("Bearer {token}").parse().unwrap());
            }
            "reader" => entry["reader_did"] = json!(carol()),
            "epoch" => entry["member_epoch"] = json!(999),
            "object-id" => {
                entry["delta"]["change"]["message"]["obj_id"] =
                    json!(message(&alice(), "mismatched", 1).gen_obj_id().0)
            }
            "session" => entry["delta"]["change"]["session_id"] = json!("wrong"),
            "object" => entry["delta"]["object"] = json!("{}"),
            _ => unreachable!(),
        }
        invalid.replay(&zones.local).await;
        assert_eq!(
            zones.local.joined_groups(&bob()).await.unwrap()[0].changes_token,
            old,
            "{mutation}"
        );
    }
    settle(&zones.faults).await;
    assert_no_pull(&zones.faults);
    assert!(zones
        .local
        .groups
        .object(&msg.gen_obj_id().0)
        .await
        .unwrap()
        .is_none());
    assert!(captured.replay(&zones.local).await.is_success());
    wait_inbox(&zones.local, &bob(), &msg).await;

    zones.faults.drop_push.store(false, Ordering::SeqCst);
    rpc(
        &zones.host,
        &alice(),
        "group.remove_member",
        json!({"member_did":bob()}),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !zones.local.joined_groups(&bob()).await.unwrap()[0].stopped {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    join_for_push(&zones, &bob()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();
    let new = zones.local.joined_groups(&bob()).await.unwrap()[0]
        .changes_token
        .clone();
    captured.replay(&zones.local).await;
    settle(&zones.faults).await;
    assert_eq!(
        zones.local.joined_groups(&bob()).await.unwrap()[0].changes_token,
        new
    );
    assert_no_pull(&zones.faults);
}

#[tokio::test]
async fn a_slow_target_does_not_hold_acceptance_another_groups_push_or_read_progress() {
    let mut zones = two_zones().await;
    let dir = tempdir().unwrap();
    let path = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("zone-c.db").display()
    );
    let db = MsgBoxDbMgr::open_default_sqlite(&path).await.unwrap();
    let slow = MessageCenter::open_with_db(db).await.unwrap();
    slow.cyfs_dispatch.write().unwrap().target_zone = Some(ZONE_C.host.into());
    slow.register_local_recipients(vec![dora()]);
    slow.directory.set(zones.directory.clone());
    sign_as(&slow, &ZONE_C, zone_documents());
    let slow_faults = Arc::new(Faults::default());
    let (slow_url, server) = serve_zone(slow.clone(), &ZONE_C, slow_faults.clone()).await;
    zones.servers.push(server);
    connect(
        &zones.host,
        &ZONE_A,
        &zone_documents(),
        HashMap::from([
            (ZONE_B.host.into(), zones.local_url.clone()),
            (ZONE_C.host.into(), slow_url),
        ]),
    );
    connect(
        &slow,
        &ZONE_C,
        &zone_documents(),
        HashMap::from([(ZONE_A.host.into(), zones.host_url.clone())]),
    );
    create(&zones).await;
    let invitation = invite(&zones, &alice(), &dora()).await;
    assert_eq!(
        accept(&slow, &dora(), &invitation).await.unwrap()["sync_pending"],
        false
    );

    rpc(
        &zones.host,
        &alice(),
        "group.create",
        json!({"group_did":other_group(),"idempotency_key":"second"}),
    )
    .await
    .unwrap();
    let invitation = rpc(
        &zones.host,
        &alice(),
        "group.invite_member",
        json!({"group_did":other_group(),"member_did":bob()}),
    )
    .await
    .unwrap();
    let accepted = rpc(
        &zones.local,
        &bob(),
        "group.accept_invitation",
        json!({
            "group_did":other_group(),"invitation_id":invitation["invite_id"]
        }),
    )
    .await
    .unwrap();
    assert_eq!(accepted["sync_pending"], false);
    zones.local.start_group_sync();
    slow.start_group_sync();
    settle(&zones.faults).await;
    settle(&slow_faults).await;
    zones.faults.clear_traffic();
    slow_faults.clear_traffic();
    let stats = &zones.host.groups.push;
    let before = (
        stats.merged.load(Ordering::Relaxed),
        stats.dropped.load(Ordering::Relaxed),
        zones
            .local
            .groups
            .pulls
            .fast_commits
            .load(Ordering::Relaxed),
        slow.groups.pulls.catchup_runs.load(Ordering::Relaxed),
    );
    slow_faults.block_push.store(true, Ordering::SeqCst);
    slow_faults.timeout_push.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(5), async {
        host_message(&zones.host, "blocked destination", 800).await;
        while slow_faults.pushes.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        host_message(&zones.host, "accepted during blocked push", 801).await;
        let mut msg = message(&alice(), "independent group", 802);
        msg.to = vec![other_group()];
        let result = zones
            .host
            .handle_post_send(msg.clone(), None, context(&alice()))
            .await
            .unwrap();
        assert!(result.ok, "{result:?}");
        let record = wait_inbox(&zones.local, &bob(), &msg).await;
        zones
            .local
            .handle_update_record_state(
                record.record_id,
                buckyos_api::RecipientState::Read,
                context(&bob()),
            )
            .await
            .unwrap();
        let state = zones
            .local
            .joined_groups(&bob())
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.group_did == other_group())
            .unwrap();
        assert!(state.last_read_seq[&session_key(&other_group(), None).unwrap()] > 0);
    })
    .await
    .expect("other group must make progress while the first response is held");
    assert_no_pull(&zones.faults);
    slow_faults.block_push.store(false, Ordering::SeqCst);
    slow_faults.release_push.notify_waiters();
    wait_inbox(
        &slow,
        &dora(),
        &message(&alice(), "accepted during blocked push", 801),
    )
    .await;
    let b_pushes = zones.faults.pushes.lock().unwrap().len();
    let c_pushes = slow_faults.pushes.lock().unwrap().len();
    let peak = stats.peak_sends.load(Ordering::Relaxed);
    assert!((2..=4).contains(&peak));
    assert_eq!(b_pushes, 1);
    assert_eq!(c_pushes, 2);
    let traffic = (
        zones.faults.requests.lock().unwrap().len(),
        slow_faults.requests.lock().unwrap().len(),
    );
    settle(&zones.faults).await;
    let idle = zones.faults.requests.lock().unwrap().len()
        + slow_faults.requests.lock().unwrap().len()
        - traffic.0
        - traffic.1;
    assert_eq!(idle, 0);
    println!("3-zone loopback HTTP, 2 groups: pushes[b.example]={b_pushes}, pushes[c.example]={c_pushes}, merged={}, queue_drops={}, fast_commits={}, catchup_runs={}, peak_sends={peak}, idle_requests_250ms={idle}",
        stats.merged.load(Ordering::Relaxed)-before.0, stats.dropped.load(Ordering::Relaxed)-before.1,
        zones.local.groups.pulls.fast_commits.load(Ordering::Relaxed)-before.2,
        slow.groups.pulls.catchup_runs.load(Ordering::Relaxed)-before.3);
}

#[tokio::test]
async fn holding_a_push_while_a_message_is_retracted_keeps_its_body_off_the_wire() {
    let zones = two_zones().await;
    create(&zones).await;
    join_for_push(&zones, &bob()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();
    zones.faults.block_push.store(true, Ordering::SeqCst);
    host_message(&zones.host, "block the first response", 710).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while zones.faults.pushes.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let original = host_message(&zones.host, "withdrawal-sensitive-text", 711).await;
    let mut redact = message(&alice(), "", 712);
    redact.relates_to = Some(ndn_lib::MsgRelation {
        rel: ndn_lib::MsgRelType::Redact,
        target: original.gen_obj_id().0,
        key: None,
    });
    let result = zones
        .host
        .handle_post_send(redact.clone(), None, context(&alice()))
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    zones.faults.block_push.store(false, Ordering::SeqCst);
    zones.faults.release_push.notify_waiters();
    wait_inbox(&zones.local, &bob(), &redact).await;
    let replica = zones
        .local
        .groups
        .object(&original.gen_obj_id().0)
        .await
        .unwrap()
        .unwrap();
    assert!(replica.1.is_none());
    let pushes = zones.faults.pushes.lock().unwrap();
    assert!(pushes.len() >= 2);
    assert!(pushes
        .iter()
        .all(|p| !p.body.to_string().contains("withdrawal-sensitive-text")));
}

#[tokio::test]
async fn fast_path_keeps_the_signed_original_and_batches_two_readers_in_one_zone() {
    let zones = two_zones().await;
    create(&zones).await;
    join_for_push(&zones, &bob()).await;
    join_for_push(&zones, &carol()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();
    let msg = message(&alice(), "signed original", 621);
    let signer = ZONE_A.signer();
    let jwt = msg.to_jwt(signer.key(), &signer.kid).unwrap();
    let actor = crate::group_types::GroupActor {
        did: alice(),
        client: None,
        remote: false,
    };
    let result = zones
        .host
        .accept_group_message(&actor, msg.clone(), Some(jwt.clone()))
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    wait_inbox(&zones.local, &bob(), &msg).await;
    wait_inbox(&zones.local, &carol(), &msg).await;
    assert_no_pull(&zones.faults);
    let pushes = zones.faults.pushes.lock().unwrap().clone();
    assert_eq!(pushes.len(), 1);
    let entries = pushes[0].body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for reader in [bob(), carol()] {
        let entry = entries
            .iter()
            .find(|e| e["reader_did"] == json!(reader))
            .unwrap();
        assert_eq!(entry["delta"]["object"], jwt);
    }
    assert_eq!(
        zones
            .local
            .groups
            .object(&msg.gen_obj_id().0)
            .await
            .unwrap()
            .unwrap()
            .2,
        Some(jwt)
    );
}

#[tokio::test]
async fn delta_arriving_during_a_pull_commits_after_the_earlier_page() {
    let zones = two_zones().await;
    create(&zones).await;
    join_for_push(&zones, &bob()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();
    zones.faults.drop_push.store(true, Ordering::SeqCst);
    let missed = host_message(&zones.host, "body missed while disconnected", 740).await;
    settle(&zones.faults).await;
    let mut hint = zones.faults.pushes.lock().unwrap()[0].clone();
    hint.body["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("delta");
    zones.faults.drop_push.store(false, Ordering::SeqCst);
    zones.faults.hold_object.store(true, Ordering::SeqCst);
    assert!(hint.replay(&zones.local).await.is_success());
    tokio::time::timeout(Duration::from_secs(3), async {
        while zones.faults.pulls("/objects/") == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let second = host_message(&zones.host, "pushed while previous body in flight", 741).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while zones.faults.pushes.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let newest_token = zones.faults.pushes.lock().unwrap()[1].body["entries"][0]["delta"]
        ["next_token"]
        .as_str()
        .unwrap()
        .to_owned();
    zones.faults.release_object.notify_one();
    wait_inbox(&zones.local, &bob(), &second).await;
    let state = zones.local.joined_groups(&bob()).await.unwrap().remove(0);
    assert_eq!(state.changes_token, Some(newest_token));
    let rows = inbox(&zones.local, &bob(), None).await;
    for msg in [missed, second] {
        assert_eq!(
            rows.iter()
                .filter(|r| r.msg_id == msg.gen_obj_id().0)
                .count(),
            1
        );
    }
    assert_eq!(zones.faults.pulls("/changes"), 1);
    assert_eq!(zones.faults.pulls("/objects/"), 1);
}

#[tokio::test]
async fn changing_a_read_hooks_answer_before_send_withholds_the_body() {
    let mut zones = two_zones().await;
    let allow = Arc::new(AtomicBool::new(true));
    let denied_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let decision = allow.clone();
    let denied = denied_calls.clone();
    let (hook_url, hook) = http_fixture(move |request| {
        let decision = decision.clone();
        let denied = denied.clone();
        async move {
            let body = request.into_body().collect().await.unwrap().to_bytes();
            let call: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(call["operation"], "read");
            let deny = call["actor"] == json!(bob()) && !decision.load(Ordering::SeqCst);
            if deny {
                denied.fetch_add(1, Ordering::SeqCst);
            }
            let result = if deny { "deny" } else { "allow" };
            Response::builder()
                .header("content-type", "application/json")
                .body(http_body(json!({"decision":result}).to_string()))
                .unwrap()
        }
    })
    .await;
    zones.servers.push(hook);
    rpc(&zones.host, &alice(), "group.create", json!({
        "idempotency_key":"read-hook","configuration":{"default_session":{"history":"all","hooks":[{
            "point":"read","service":hook_url,"on_unavailable":"deny","timeout_ms":1000
        }]}}
    })).await.unwrap();
    join_for_push(&zones, &bob()).await;
    settle(&zones.faults).await;
    zones.faults.clear_traffic();
    zones.faults.drop_push.store(true, Ordering::SeqCst);
    allow.store(false, Ordering::SeqCst);
    host_message(&zones.host, "denied after baseline", 795).await;
    settle(&zones.faults).await;
    let pushes = zones.faults.pushes.lock().unwrap();
    assert_eq!(pushes.len(), 1);
    assert!(pushes[0].body["entries"][0]["delta"].is_null());
    assert!(denied_calls.load(Ordering::SeqCst) > 0);
    assert_no_pull(&zones.faults);
}
async fn request_join(
    center: &MessageCenter,
    who: &DID,
    invite: Option<&Value>,
) -> Result<Value, RPCErrors> {
    let mut params = json!({});
    if let Some(token) = invite {
        params["invite"] = token.clone();
    }
    rpc(center, who, "group.request_join", params).await
}
async fn set_join_policy(zones: &TwoZones, policy: &str) {
    let revision = rpc(&zones.host, &alice(), "group.get_config", json!({}))
        .await
        .unwrap()["revision"]
        .clone();
    rpc(
        &zones.host,
        &alice(),
        "group.apply_config",
        json!({
            "expected_revision":revision,"idempotency_key":format!("policy-{policy}"),
            "patch":{"membership":{"join_policy":policy}}
        }),
    )
    .await
    .unwrap();
}
async fn lookup_state(center: &MessageCenter, who: &DID) -> Option<String> {
    center
        .contact_lookup(
            who,
            buckyos_api::ContactLookupReq {
                input: group().to_string(),
                kinds: Some(vec![LookupKind::Group]),
                limit: None,
                contact_mgr_owner: Some(who.clone()),
            },
        )
        .await
        .unwrap()
        .candidates
        .into_iter()
        .find(|candidate| candidate.did == group())
        .expect("the group is found")
        .relation
        .group_state
}
async fn host_member_state(zones: &TwoZones, who: &DID) -> Option<MemberStatus> {
    zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .members
        .get(&who.to_string())
        .map(|member| member.state)
}
async fn host_seq(zones: &TwoZones) -> u64 {
    zones
        .host
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .group_seq
}

#[tokio::test]
async fn cross_zone_join_requests_follow_the_hosts_join_policy() {
    let zones = two_zones().await;
    create(&zones).await;
    assert!(matches!(
        request_join(&zones.local, &bob(), None).await,
        Err(RPCErrors::NoPermission(reason)) if reason == "invite-required"
    ));
    assert!(zones
        .local
        .load_group_binding(&bob(), &group())
        .await
        .unwrap()
        .is_none());
    assert_eq!(lookup_state(&zones.local, &bob()).await, None);

    set_join_policy(&zones, "request_and_approve").await;
    let requested = request_join(&zones.local, &bob(), None).await.unwrap();
    assert_eq!(requested["state"], "pending_admin_approval");
    assert!(requested["invitation_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    let listed = list(&zones.local, &bob()).await;
    assert_eq!(listed["invitations"][0]["group_did"], json!(group()));
    assert_eq!(
        listed["invitations"][0]["member"]["state"],
        "pending_admin_approval"
    );
    assert_eq!(
        lookup_state(&zones.local, &bob()).await.as_deref(),
        Some("pending_admin_approval")
    );
    // Asking again while waiting is refused by the host and keeps the wait as it was.
    let seq = host_seq(&zones).await;
    assert!(matches!(
        request_join(&zones.local, &bob(), None).await,
        Err(RPCErrors::NoPermission(reason)) if reason == "member-already-participating"
    ));
    assert_eq!(host_seq(&zones).await, seq);
    assert_eq!(
        lookup_state(&zones.local, &bob()).await.as_deref(),
        Some("pending_admin_approval")
    );
    zones.local.sync_remote_groups().await.unwrap();
    assert!(list(&zones.local, &bob()).await["joined"]
        .as_array()
        .unwrap()
        .is_empty());

    rpc(
        &zones.host,
        &alice(),
        "group.approve_member",
        json!({"member_did":bob()}),
    )
    .await
    .unwrap();
    zones.local.sync_remote_groups().await.unwrap();
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["group_did"],
        json!(group())
    );
    assert!(list(&zones.local, &bob()).await["invitations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        lookup_state(&zones.local, &bob()).await.as_deref(),
        Some("active")
    );
    let history = host_message(&zones.host, "after approval", 1).await;
    sync(&zones.local, &bob()).await;
    assert!(projected(&zones.local, &bob()).await.contains(&history));

    // A rejection ends the request; following it up never asks the host again.
    assert_eq!(
        request_join(&zones.local, &carol(), None).await.unwrap()["state"],
        "pending_admin_approval"
    );
    rpc(
        &zones.host,
        &alice(),
        "group.reject_member",
        json!({"member_did":carol()}),
    )
    .await
    .unwrap();
    zones.local.sync_remote_groups().await.unwrap();
    assert!(zones
        .local
        .load_group_binding(&carol(), &group())
        .await
        .unwrap()
        .is_none());
    zones.local.sync_remote_groups().await.unwrap();
    assert_eq!(
        host_member_state(&zones, &carol()).await,
        Some(MemberStatus::Rejected)
    );
    assert_eq!(lookup_state(&zones.local, &carol()).await, None);

    set_join_policy(&zones, "open").await;
    let joined = request_join(&zones.local, &carol(), None).await.unwrap();
    assert_eq!(joined["state"], "active");
    assert_eq!(joined["sync_pending"], false);
    assert!(projected(&zones.local, &carol()).await.contains(&history));
    assert_eq!(
        lookup_state(&zones.local, &carol()).await.as_deref(),
        Some("active")
    );
}

#[tokio::test]
async fn cross_zone_invite_link_join_survives_a_lost_reply_without_using_the_link_twice() {
    let mut zones = two_zones().await;
    create(&zones).await;
    let link = rpc(
        &zones.host,
        &alice(),
        "group.create_invite_link",
        json!({"max_uses":1}),
    )
    .await
    .unwrap();
    let token = link["token"].as_str().unwrap().to_owned();
    let used = |zones: &TwoZones| {
        let center = zones.host.clone();
        let token = token.clone();
        async move {
            center
                .groups
                .load(&group())
                .await
                .unwrap()
                .unwrap()
                .invite_links[&token]
                .used
        }
    };
    zones.faults.lose_join_reply.store(true, Ordering::SeqCst);
    assert!(matches!(
        request_join(&zones.local, &bob(), Some(&link["token"])).await,
        Err(RPCErrors::ReasonError(_))
    ));
    assert_eq!(
        host_member_state(&zones, &bob()).await,
        Some(MemberStatus::Active)
    );
    assert_eq!(used(&zones).await, 1);
    let seq = host_seq(&zones).await;

    zones.local = open(&zones.local_path, false, zones.directory.clone()).await;
    reconnect(&zones.local, &zones.host_url);
    zones.local.sync_remote_groups().await.unwrap();
    assert_eq!(
        list(&zones.local, &bob()).await["joined"][0]["group_did"],
        json!(group())
    );
    assert_eq!(used(&zones).await, 1);
    assert_eq!(host_seq(&zones).await, seq);
    assert_eq!(
        lookup_state(&zones.local, &bob()).await.as_deref(),
        Some("active")
    );

    // The link is used up: the next person is refused and nothing is left behind.
    assert!(matches!(
        request_join(&zones.local, &carol(), Some(&link["token"])).await,
        Err(RPCErrors::NoPermission(reason)) if reason == "invalid-invite-link"
    ));
    assert!(zones
        .local
        .load_group_binding(&carol(), &group())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn cross_zone_invitations_wait_with_the_members_invitations_until_answered() {
    let zones = two_zones().await;
    rpc(
        &zones.host,
        &alice(),
        "group.create",
        json!({
            "idempotency_key":"create",
            "configuration":{"default_session":{"history":"all"},"profile":{"name":"Support Desk"}}
        }),
    )
    .await
    .unwrap();
    let invitation = invite(&zones, &alice(), &bob()).await;
    pump_one(&zones.host).await;
    let listed = list(&zones.local, &bob()).await;
    let items = listed["invitations"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["group_did"], json!(group()));
    assert_eq!(items[0]["hosted_here"], false);
    assert!(items[0]["doc"].is_null());
    assert_eq!(items[0]["profile"]["name"], "Support Desk");
    assert_eq!(items[0]["member"]["state"], "invited");
    assert_eq!(items[0]["member"]["invitation_id"], invitation["invite_id"]);
    assert_eq!(items[0]["member"]["invited_by"], json!(alice()));
    assert_eq!(items[0]["member"]["role"], "member");
    assert_eq!(
        lookup_state(&zones.local, &bob()).await.as_deref(),
        Some("invited")
    );
    assert!(list(&zones.local, &carol()).await["invitations"]
        .as_array()
        .unwrap()
        .is_empty());

    assert_eq!(
        accept(&zones.local, &bob(), &invitation).await.unwrap()["state"],
        "active"
    );
    let listed = list(&zones.local, &bob()).await;
    assert!(listed["invitations"].as_array().unwrap().is_empty());
    assert_eq!(listed["joined"][0]["group_did"], json!(group()));

    // The host does not tell the invitee about a revocation: accepting it finds out.
    let revoked = invite(&zones, &alice(), &carol()).await;
    pump_one(&zones.host).await;
    assert_eq!(
        list(&zones.local, &carol()).await["invitations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    rpc(
        &zones.host,
        &alice(),
        "group.revoke_invite",
        json!({"member_did":carol()}),
    )
    .await
    .unwrap();
    assert!(matches!(
        accept(&zones.local, &carol(), &revoked).await,
        Err(RPCErrors::NoPermission(reason)) if reason == "invitation-revoked"
    ));
    assert!(list(&zones.local, &carol()).await["invitations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(lookup_state(&zones.local, &carol()).await, None);

    // A request answered by the host's pending invitation also clears it.
    let pending = invite(&zones, &alice(), &carol()).await;
    pump_one(&zones.host).await;
    let joined = request_join(&zones.local, &carol(), None).await.unwrap();
    assert_eq!(joined["state"], "active");
    assert_eq!(joined["invitation_id"], pending["invite_id"]);
    assert!(list(&zones.local, &carol()).await["invitations"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn expired_cross_zone_invitations_are_not_listed() {
    let zones = two_zones().await;
    create(&zones).await;
    rpc(
        &zones.host,
        &alice(),
        "group.invite_member",
        json!({"member_did":bob(),"expires_at_ms":MessageCenter::now_ms() + 800}),
    )
    .await
    .unwrap();
    pump_one(&zones.host).await;
    assert_eq!(
        list(&zones.local, &bob()).await["invitations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(list(&zones.local, &bob()).await["invitations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(lookup_state(&zones.local, &bob()).await, None);
}
