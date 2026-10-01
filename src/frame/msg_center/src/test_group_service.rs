use crate::group_types::*;
use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use crate::owner_session::SessionTokenVerifier;
use buckyos_api::{
    bind_token_principal_kind, bind_token_target, AuthTarget, MailboxAddress, MailboxKind,
    MsgCenterHandler, SystemServiceId, TokenPrincipalKind, TokenUse,
};
use buckyos_http_server::ServerError;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use jsonwebtoken::{DecodingKey, EncodingKey};
use kRPC::{RPCContext, RPCSessionToken, RPCSessionTokenType};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject, MsgRelType, MsgRelation, NamedObject};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type TestBody = BoxBody<Bytes, ServerError>;
fn http_body(data: impl Into<Bytes>) -> TestBody {
    Full::new(data.into()).map_err(|e| match e {}).boxed()
}
async fn http_fixture<F, Fut>(handler: F) -> (String, tokio::task::JoinHandle<()>)
where
    F: Fn(Request<TestBody>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Response<TestBody>> + Send,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = vec![];
            let end = loop {
                let mut part = [0_u8; 4096];
                let n = stream.read(&mut part).await.unwrap();
                assert!(n > 0);
                raw.extend_from_slice(&part[..n]);
                if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(raw[..end].to_vec()).unwrap();
            let mut lines = headers.split("\r\n");
            let mut first = lines.next().unwrap().split_whitespace();
            let method = first.next().unwrap();
            let uri = first.next().unwrap();
            let mut builder = Request::builder().method(method).uri(uri);
            let mut length = 0;
            for line in lines.filter(|l| !l.is_empty()) {
                let (name, value) = line.split_once(':').unwrap();
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap();
                }
                builder = builder.header(name, value.trim());
            }
            while raw.len() < end + length {
                let mut part = [0_u8; 4096];
                let n = stream.read(&mut part).await.unwrap();
                assert!(n > 0);
                raw.extend_from_slice(&part[..n]);
            }
            let request = builder
                .body(http_body(raw[end..end + length].to_vec()))
                .unwrap();
            let response = handler(request).await;
            let (parts, body) = response.into_parts();
            let body = body.collect().await.unwrap().to_bytes();
            let mut head = format!(
                "HTTP/1.1 {}\r\nconnection: close\r\ncontent-length: {}\r\n",
                parts.status,
                body.len()
            );
            for (name, value) in &parts.headers {
                if name != "content-length" && name != "connection" {
                    head.push_str(&format!("{}: {}\r\n", name, value.to_str().unwrap()));
                }
            }
            head.push_str("\r\n");
            stream.write_all(head.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        }
    });
    (url, task)
}
async fn host_fixture(c: MessageCenter) -> (String, tokio::task::JoinHandle<()>) {
    http_fixture(move |mut req| {
        let c = c.clone();
        async move {
            crate::group_http::serve(&c, &mut req)
                .await
                .unwrap_or_else(|| {
                    Response::builder()
                        .status(404)
                        .body(http_body("{}"))
                        .unwrap()
                })
        }
    })
    .await
}
fn joined_route(upstream: String, d: &DID) -> crate::group_sync::JoinedGroupRoute {
    crate::group_sync::JoinedGroupRoute {
        owner_did: d.clone(),
        group_did: group(),
        host: "test.example".into(),
        upstream,
        authorization: format!("Bearer {}", context(d).token.unwrap()),
        proof_ids: vec![],
    }
}

const OWNER_KEY:&str="-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const MEMBER_KEY:&str="-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIF5zplRSY5MsYbBMdbzItH90daUlrx/OflPm8Kp9m2j1\n-----END PRIVATE KEY-----";
const GUEST_KEY:&str="-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIEw6BLpRcM9sEFpKnNdx0iKLC4w3KY+hfShQQJMPy60V\n-----END PRIVATE KEY-----";
fn owner() -> DID {
    DID::new("dev", "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8")
}
fn member() -> DID {
    DID::new("dev", "mL_tm5diVGGkpt_6WmCZQxUsee4rHacyz7mx7TkKb-0")
}
fn guest() -> DID {
    DID::new("dev", "IpMPQFSbItHA0O4_RkVTraZa11q5SaVY_x6s9wZ3kUM")
}
fn group() -> DID {
    DID::new("web", "support.test.example")
}
fn key(d: &DID) -> EncodingKey {
    EncodingKey::from_ed_pem(
        if *d == owner() {
            OWNER_KEY
        } else if *d == member() {
            MEMBER_KEY
        } else {
            GUEST_KEY
        }
        .as_bytes(),
    )
    .unwrap()
}
fn actor(d: DID) -> GroupActor {
    GroupActor {
        did: d,
        client: Some("messagehub".into()),
        remote: false,
    }
}
struct Verifier;
#[async_trait::async_trait]
impl SessionTokenVerifier for Verifier {
    async fn authorize(&self, _: &str, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    async fn verify(&self, t: &str) -> Result<RPCSessionToken> {
        let mut t = RPCSessionToken::from_string(t)?;
        t.verify_by_key(&DecodingKey::from_ed_components(&owner().id).unwrap())?;
        Ok(t)
    }
    async fn resolve_user_did(&self, id: &str) -> Result<DID> {
        DID::from_str(id).map_err(invalid)
    }
    async fn is_zone_agent(&self, _: &DID) -> Result<bool> {
        Ok(false)
    }
}
fn context(d: &DID) -> RPCContext {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(buckyos_api::VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(d.to_string()),
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
    RPCContext {
        token: Some(token.generate_jwt(None, &key(&owner())).unwrap()),
        from_ip: Some("127.0.0.1".parse().unwrap()),
        ..Default::default()
    }
}
fn proof(d: &DID, role: Option<&str>, scope: Value, invite: Option<String>) -> Value {
    let mut claims = json!({"obj_type":"buckyos.group_member_proof","schema_version":1,"group_did":group(),"member_did":d,"signer":d,"proof_scope":scope,"nonce":revision(),"issued_at_ms":MessageCenter::now_ms(),"expires_at_ms":MessageCenter::now_ms()+60_000});
    if let Some(r) = role {
        claims["role"] = json!(r);
    }
    if let Some(i) = invite {
        claims["invite_id"] = json!(i);
    }
    let mut h = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
    h.kid = Some(d.to_string());
    json!(jsonwebtoken::encode(&h, &claims, &key(d)).unwrap())
}
async fn opened(connection: &str) -> MessageCenter {
    let db = MsgBoxDbMgr::open_default_sqlite(connection).await.unwrap();
    let c = MessageCenter::open_with_db(db).await.unwrap();
    c.set_token_verifier(Arc::new(Verifier));
    c.cyfs_dispatch.write().unwrap().target_zone = Some("test.example".into());
    c.register_local_recipients([owner(), member(), guest()]);
    c.set_message_hub_did(DID::new("web", "hub.test.example"));
    c
}
async fn center() -> (MessageCenter, TempDir, String) {
    let tmp = tempdir().unwrap();
    let path = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join("group.db").display()
    );
    (opened(&path).await, tmp, path)
}
async fn call(c: &MessageCenter, d: &DID, m: &str, mut p: Value) -> Result<Value> {
    p["group_did"] = json!(group());
    c.group_rpc(m, p, context(d)).await
}
async fn create(c: &MessageCenter, config: Value) -> Value {
    call(c,&owner(),"group.create",json!({"idempotency_key":"create","configuration":config,"proof":proof(&owner(),Some("owner"),json!("group"),None)})).await.unwrap()
}
async fn join(c: &MessageCenter, d: &DID) -> Value {
    let invite = call(c, &owner(), "group.invite_member", json!({"member_did":d}))
        .await
        .unwrap();
    call(c,d,"group.submit_member_proof",json!({"proof":proof(d,Some("member"),json!("group"),Some(invite["invite_id"].as_str().unwrap().into()))})).await.unwrap()
}
fn message(d: &DID, s: Option<&str>, text: &str, created: u64) -> MsgObject {
    let mut msg = MsgObject {
        from: d.clone(),
        to: vec![group()],
        kind: MsgObjKind::GroupMsg,
        to_session: s.map(str::to_owned),
        created_at_ms: created,
        ..Default::default()
    };
    msg.content.content = text.into();
    msg
}
async fn send(c: &MessageCenter, d: &DID, s: Option<&str>, text: &str, created: u64) -> MsgObject {
    let msg = message(d, s, text, created);
    let r = c
        .accept_group_message(&actor(d.clone()), msg.clone(), None)
        .await
        .unwrap();
    assert!(r.ok, "{:?}", r.reason);
    msg
}

#[tokio::test]
async fn group_creation_is_atomic_idempotent_and_authenticates_the_owner() {
    let (c, _tmp, path) = center().await;
    let p = json!({"group_did":group(),"idempotency_key":"create","proof":proof(&owner(),Some("owner"),json!("group"),None),"sessions":[{"session_id":"announcements","rule_overrides":{"post":{"only":["owner","admin"]}}}]});
    assert!(c
        .group_rpc("group.create", p.clone(), RPCContext::default())
        .await
        .is_err());
    let a = c
        .group_rpc("group.create", p.clone(), context(&owner()))
        .await
        .unwrap();
    let b = c
        .group_rpc("group.create", p.clone(), context(&owner()))
        .await
        .unwrap();
    assert_eq!(a, b);
    let mut mismatch = p.clone();
    mismatch["profile"] = json!({"name":"changed"});
    assert!(c
        .group_rpc("group.create", mismatch, context(&owner()))
        .await
        .is_err());
    let mut forged = p.clone();
    forged["actor_did"] = json!(owner());
    assert!(c
        .group_rpc("group.create", forged, context(&guest()))
        .await
        .is_err());
    let g = c.groups.load(&group()).await.unwrap().unwrap();
    assert_eq!(g.members.len(), 1);
    assert_eq!(g.sessions.len(), 1);
    assert!(g.members[&owner().to_string()].proof_id.is_some());
    drop(c);
    let c = opened(&path).await;
    assert!(c.is_local_recipient(&group()));
    assert_eq!(
        c.groups.load(&group()).await.unwrap().unwrap().group_seq,
        g.group_seq
    );
}

#[tokio::test]
async fn session_isolation_guests_and_explicit_members_survive_restart() {
    let (c, _tmp, path) = center().await;
    create(&c, json!({})).await;
    join(&c, &member()).await;
    call(&c,&owner(),"group.create_session",json!({"session_id":"private / 客服","membership":"explicit","rule_overrides":{"allow_guests":true}})).await.unwrap();
    let missing = c
        .accept_group_message(
            &actor(member()),
            message(&member(), Some("private / 客服"), "not allowed", 1),
            None,
        )
        .await
        .unwrap();
    let unknown = c
        .accept_group_message(
            &actor(member()),
            message(&member(), Some("unknown"), "not allowed", 1),
            None,
        )
        .await
        .unwrap();
    assert_eq!(missing.reason, unknown.reason);
    assert_eq!(missing.reason.as_deref(), Some("not-found"));
    call(
        &c,
        &owner(),
        "group.invite_session_guest",
        json!({"session_id":"private / 客服","member_did":guest()}),
    )
    .await
    .unwrap();
    call(&c,&guest(),"group.submit_session_proof",json!({"session_id":"private / 客服","proof":proof(&guest(),None,json!({"session":"private / 客服"}),None)})).await.unwrap();
    let list = call(&c, &guest(), "group.list_sessions", json!({}))
        .await
        .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["items"][0]["has_guests"], true);
    assert!(call(&c, &guest(), "group.list_members", json!({}))
        .await
        .is_err());
    let msg = send(&c, &guest(), Some("private / 客服"), "customer", 9000).await;
    assert!(c
        .authorize_group_message(&actor(member()), &msg.gen_obj_id().0)
        .await
        .is_err());
    let mailbox = MailboxAddress::new(group(), Some("private / 客服".into())).unwrap();
    assert!(c
        .handle_list_box_by_time(
            mailbox,
            MailboxKind::GroupInbox,
            None,
            None,
            None,
            None,
            None,
            Some(true),
            context(&member())
        )
        .await
        .is_err());
    let key = session_key(&group(), Some("private / 客服")).unwrap();
    let local = c
        .msg_box_db
        .get_owner_session(&guest(), &key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(local.binding.unwrap()["authority_did"], json!(group()));
    drop(c);
    let c = opened(&path).await;
    c.authorize_group_message(&actor(guest()), &msg.gen_obj_id().0)
        .await
        .unwrap();
    assert_eq!(
        c.handle_get_message(msg.gen_obj_id().0, context(&guest()))
            .await
            .unwrap()
            .unwrap(),
        msg
    );
}

#[tokio::test]
async fn from_join_history_has_gaps_and_old_session_exclusions_expire_with_epoch() {
    let (c, _tmp, _) = center().await;
    create(&c, json!({})).await;
    let before = send(&c, &owner(), None, "before joining", 100).await;
    join(&c, &member()).await;
    let first = send(&c, &owner(), None, "first epoch", 200).await;
    call(
        &c,
        &owner(),
        "group.create_session",
        json!({"session_id":"team"}),
    )
    .await
    .unwrap();
    call(
        &c,
        &member(),
        "group.leave_session",
        json!({"session_id":"team"}),
    )
    .await
    .unwrap();
    call(&c, &member(), "group.leave", json!({})).await.unwrap();
    let gap = send(&c, &owner(), None, "away", 300).await;
    join(&c, &member()).await;
    let late = send(&c, &owner(), None, "late clock", 1).await;
    let g = c.groups.load(&group()).await.unwrap().unwrap();
    assert_eq!(g.members[&member().to_string()].epoch, 2);
    assert!(g.effective(Some("team"), &member()));
    for (msg, allowed) in [
        (&before, false),
        (&first, true),
        (&gap, false),
        (&late, true),
    ] {
        assert_eq!(
            c.authorize_group_message(&actor(member()), &msg.gen_obj_id().0)
                .await
                .is_ok(),
            allowed
        );
    }
    let mailbox = MailboxAddress::new(group(), None).unwrap();
    let page = c
        .handle_list_box_by_time(
            mailbox,
            MailboxKind::GroupInbox,
            None,
            Some(100),
            None,
            None,
            Some(false),
            Some(true),
            context(&member()),
        )
        .await
        .unwrap();
    let ids: Vec<_> = page.items.iter().map(|i| i.record.msg_id.clone()).collect();
    assert!(ids.contains(&first.gen_obj_id().0));
    assert!(ids.contains(&late.gen_obj_id().0));
    assert!(!ids.contains(&before.gen_obj_id().0));
    assert!(!ids.contains(&gap.gen_obj_id().0));
    assert!(
        g.messages[&late.gen_obj_id().0.to_string()].accepted_at_ms
            >= g.messages[&first.gen_obj_id().0.to_string()].accepted_at_ms
    );
}

#[tokio::test]
async fn messages_are_idempotent_under_concurrency_and_retractions_remove_bodies() {
    let (c, _tmp, path) = center().await;
    create(&c, json!({})).await;
    join(&c, &member()).await;
    let original = message(&member(), None, "original", 123);
    let sender = actor(member());
    let (a, b) = tokio::join!(
        c.accept_group_message(&sender, original.clone(), None),
        c.accept_group_message(&sender, original.clone(), None)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    let original_id = original.gen_obj_id().0;
    let mut edit = message(&member(), None, "edited", 124);
    edit.relates_to = Some(MsgRelation::new(MsgRelType::Edit, original_id.clone()));
    assert!(
        c.accept_group_message(&actor(member()), edit.clone(), None)
            .await
            .unwrap()
            .ok
    );
    let mut redact = message(&owner(), None, "moderated", 125);
    redact.relates_to = Some(MsgRelation::new(MsgRelType::Redact, original_id.clone()));
    assert!(
        c.accept_group_message(&actor(owner()), redact, None)
            .await
            .unwrap()
            .ok
    );
    for id in [original_id.clone(), edit.gen_obj_id().0] {
        assert!(c.groups.object(&id).await.unwrap().unwrap().1.is_none());
        assert!(c.handle_get_message(id, context(&member())).await.is_err());
    }
    let page = c
        .group_inbox(&actor(member()), &group(), None, 0, 100)
        .await
        .unwrap();
    assert!(page["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["obj_id"] == json!(original_id) && i["redacted"] == true));
    let g = c.groups.load(&group()).await.unwrap().unwrap();
    let mut seqs: Vec<_> = g
        .messages
        .values()
        .filter(|m| m.session_id.is_none())
        .map(|m| m.session_seq)
        .collect();
    seqs.sort();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    drop(c);
    let c = opened(&path).await;
    assert!(c
        .groups
        .object(&original_id)
        .await
        .unwrap()
        .unwrap()
        .1
        .is_none());
}

#[tokio::test]
async fn config_revisions_unknown_fields_and_template_tombstones_are_enforced() {
    let (c, _tmp, _) = center().await;
    let first=create(&c,json!({"custom":{"nested":"keep"},"session_templates":{"team":{"membership":"explicit","rules":{}}}})).await;
    let patch = json!({"expected_revision":first["revision"],"idempotency_key":"patch","patch":{"profile":{"name":"Support"},"future":{"enabled":true}}});
    let result = call(&c, &owner(), "group.apply_config", patch.clone())
        .await
        .unwrap();
    assert_eq!(
        call(&c, &owner(), "group.apply_config", patch)
            .await
            .unwrap(),
        result
    );
    let config = call(&c, &owner(), "group.get_config", json!({}))
        .await
        .unwrap();
    assert_eq!(config["custom"]["nested"], "keep");
    assert_eq!(config["future"]["enabled"], true);
    assert!(call(
        &c,
        &owner(),
        "group.apply_config",
        json!({"expected_revision":first["revision"],"idempotency_key":"stale","patch":{}})
    )
    .await
    .is_err());
    let session = call(
        &c,
        &owner(),
        "group.create_session",
        json!({"session_id":"fixed","template":"team"}),
    )
    .await
    .unwrap();
    assert!(call(&c,&owner(),"group.apply_config",json!({"expected_revision":result["revision"],"idempotency_key":"delete-template","patch":{"session_templates":{"team":null}}})).await.is_err());
    call(
        &c,
        &owner(),
        "group.delete_session",
        json!({"session_id":"fixed","expected_revision":session["revision"]}),
    )
    .await
    .unwrap();
    assert!(call(
        &c,
        &owner(),
        "group.create_session",
        json!({"session_id":"fixed"})
    )
    .await
    .is_err());
    assert!(call(
        &c,
        &owner(),
        "group.create_session",
        json!({"session_id":"_reserved"})
    )
    .await
    .is_err());
    assert!(call(
        &c,
        &owner(),
        "group.create_session",
        json!({"rule_overrides":{"allow_guests":true}})
    )
    .await
    .is_err());
}

#[tokio::test]
async fn proofs_invite_links_and_approval_cannot_resurrect_removed_members() {
    let (c, _tmp, _) = center().await;
    create(
        &c,
        json!({"membership":{"join_policy":"request_and_approve"}}),
    )
    .await;
    let mut forged = proof(&member(), Some("owner"), json!("group"), None)
        .as_str()
        .unwrap()
        .to_owned();
    forged.push('x');
    assert!(
        call(&c, &member(), "group.request_join", json!({"proof":forged}))
            .await
            .is_err()
    );
    let p = proof(&member(), Some("owner"), json!("group"), None);
    call(&c, &member(), "group.request_join", json!({"proof":p}))
        .await
        .unwrap();
    assert_eq!(
        c.groups.load(&group()).await.unwrap().unwrap().members[&member().to_string()].role,
        GroupRole::Member
    );
    call(
        &c,
        &owner(),
        "group.approve_member",
        json!({"member_did":member()}),
    )
    .await
    .unwrap();
    call(
        &c,
        &owner(),
        "group.remove_member",
        json!({"member_did":member()}),
    )
    .await
    .unwrap();
    assert!(call(
        &c,
        &owner(),
        "group.approve_member",
        json!({"member_did":member()})
    )
    .await
    .is_err());
    call(
        &c,
        &owner(),
        "group.moderate",
        json!({"member_did":member(),"blocked":true}),
    )
    .await
    .unwrap();
    assert!(call(
        &c,
        &owner(),
        "group.invite_member",
        json!({"member_did":member()})
    )
    .await
    .is_err());
    let links = call(
        &c,
        &owner(),
        "group.create_invite_link",
        json!({"max_uses":1}),
    )
    .await
    .unwrap();
    call(
        &c,
        &owner(),
        "group.revoke_invite_link",
        json!({"token":links["token"]}),
    )
    .await
    .unwrap();
    assert!(call(
        &c,
        &guest(),
        "group.submit_member_proof",
        json!({"invite":links["token"],"proof":proof(&guest(),Some("member"),json!("group"),None)})
    )
    .await
    .is_err());
}

#[tokio::test]
async fn read_markers_and_change_tokens_are_reader_scoped_and_monotonic() {
    let (c, _tmp, _) = center().await;
    create(&c, json!({"default_session":{"receipts":"readers"}})).await;
    join(&c, &member()).await;
    let m = send(&c, &owner(), None, "hello", 1).await;
    let seq = c.groups.load(&group()).await.unwrap().unwrap().messages
        [&m.gen_obj_id().0.to_string()]
        .session_seq;
    let changed = call(&c, &member(), "group.changes", json!({"limit":1}))
        .await
        .unwrap();
    assert!(changed.get("group_seq").is_none());
    let token = changed["next_token"].as_str().unwrap();
    assert!(token.parse::<u64>().is_err());
    let other = call(&c, &owner(), "group.changes", json!({"since":token}))
        .await
        .unwrap();
    assert_eq!(other["limited"], true);
    call(
        &c,
        &member(),
        "group.update_read_marker",
        json!({"last_read_seq":seq}),
    )
    .await
    .unwrap();
    let old = call(
        &c,
        &member(),
        "group.update_read_marker",
        json!({"last_read_seq":0}),
    )
    .await
    .unwrap();
    assert_eq!(old["last_read_seq"], seq);
    let receipts = call(
        &c,
        &owner(),
        "group.get_read_markers",
        json!({"session_seq":seq}),
    )
    .await
    .unwrap();
    assert_eq!(receipts["count"], 1);
    assert!(call(
        &c,
        &member(),
        "group.update_read_marker",
        json!({"last_read_seq":seq+100})
    )
    .await
    .is_err());
}

#[tokio::test]
async fn group_http_signed_delivery_and_joined_sync_preserve_identity_order_and_read_markers() {
    let (host, _htmp, _) = center().await;
    create(&host, json!({})).await;
    join(&host, &member()).await;
    let sid = "项目 / 计划";
    call(
        &host,
        &owner(),
        "group.create_session",
        json!({"session_id":sid}),
    )
    .await
    .unwrap();
    let (url, server) = host_fixture(host.clone()).await;
    let mut binding = joined_route(url.clone(), &member());
    binding.proof_ids = vec![host.groups.load(&group()).await.unwrap().unwrap().members
        [&member().to_string()]
        .proof_id
        .as_ref()
        .unwrap()
        .to_string()];
    let (local, _ltmp, path) = center().await;
    local
        .cyfs_dispatch
        .write()
        .unwrap()
        .joined_groups
        .push(binding.clone());
    let msg = message(&member(), Some(sid), "signed payload", u64::MAX - 100);
    let jwt = msg.to_jwt(&key(&member()), &member().to_string()).unwrap();
    let (route, proofs) = local
        .cyfs_dispatch
        .read()
        .unwrap()
        .message_route(&msg, &group())
        .unwrap()
        .unwrap();
    assert!(route.target.contains("/sessions/"));
    assert!(!route.target.contains("项目"));
    assert_eq!(proofs, binding.proof_ids);
    let result = crate::cyfs_dispatch::send_with_proofs(
        &route,
        &msg,
        Some(&jwt),
        &msg.gen_obj_id().0,
        &proofs,
    )
    .await
    .unwrap();
    assert!(result.ok, "{result:?}");
    let saved = host
        .groups
        .object(&msg.gen_obj_id().0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.2.as_deref(), Some(jwt.as_str()));
    let replay = crate::cyfs_dispatch::send_with_proofs(
        &route,
        &msg,
        Some(&jwt),
        &msg.gen_obj_id().0,
        &proofs,
    )
    .await
    .unwrap();
    assert!(replay.ok);
    let second = send(&host, &owner(), Some(sid), "older sender timestamp", 1).await;
    local.sync_joined_group(&binding).await.unwrap();
    let key = session_key(&group(), Some(sid)).unwrap();
    let mailbox = MailboxAddress::new(member(), Some(key.clone())).unwrap();
    let records = local
        .msg_box_db
        .list_records(&mailbox, &MailboxKind::Inbox, None, false)
        .await
        .unwrap();
    let first = records
        .iter()
        .find(|r| r.msg_id == msg.gen_obj_id().0)
        .unwrap();
    let after = records
        .iter()
        .find(|r| r.msg_id == second.gen_obj_id().0)
        .unwrap();
    let meta = host.groups.load(&group()).await.unwrap().unwrap().messages
        [&msg.gen_obj_id().0.to_string()]
        .clone();
    assert_eq!(first.sort_key, meta.accepted_at_ms);
    assert!(after.sort_key >= first.sort_key);
    assert_ne!(first.sort_key, msg.created_at_ms);
    assert_eq!(
        local
            .groups
            .object(&msg.gen_obj_id().0)
            .await
            .unwrap()
            .unwrap()
            .2,
        Some(jwt)
    );
    local
        .handle_update_record_state(
            after.record_id.clone(),
            buckyos_api::RecipientState::Read,
            context(&member()),
        )
        .await
        .unwrap();
    local.sync_joined_group(&binding).await.unwrap();
    let state = host.groups.load(&group()).await.unwrap().unwrap();
    assert_eq!(
        state.read_markers[sid][&member().to_string()],
        state.messages[&second.gen_obj_id().0.to_string()].session_seq
    );
    let count = records.len();
    drop(local);
    let reopened = opened(&path).await;
    reopened.sync_joined_group(&binding).await.unwrap();
    assert_eq!(
        reopened
            .msg_box_db
            .list_records(&mailbox, &MailboxKind::Inbox, None, false)
            .await
            .unwrap()
            .len(),
        count
    );
    assert!(reopened.joined_groups(&member()).await.unwrap()[0]
        .doc_cache
        .is_some());
    server.abort();
}

#[tokio::test]
async fn joined_sync_erases_redacted_bodies_and_stops_after_removal_or_group_deletion() {
    let (host, _htmp, _) = center().await;
    create(&host, json!({})).await;
    join(&host, &member()).await;
    let original = send(&host, &owner(), None, "secret", 1).await;
    let (url, server) = host_fixture(host.clone()).await;
    let state = host.groups.load(&group()).await.unwrap().unwrap();
    let mut member_route = joined_route(url.clone(), &member());
    member_route.proof_ids = vec![state.members[&member().to_string()]
        .proof_id
        .as_ref()
        .unwrap()
        .to_string()];
    let mut owner_route = joined_route(url, &owner());
    owner_route.proof_ids = vec![state.members[&owner().to_string()]
        .proof_id
        .as_ref()
        .unwrap()
        .to_string()];
    let (local, _ltmp, path) = center().await;
    local.sync_joined_group(&member_route).await.unwrap();
    local.sync_joined_group(&owner_route).await.unwrap();
    let mut redact = message(&owner(), None, "", 2);
    redact.relates_to = Some(MsgRelation {
        rel: MsgRelType::Redact,
        target: original.gen_obj_id().0,
        key: None,
    });
    assert!(
        host.accept_group_message(&actor(owner()), redact, None)
            .await
            .unwrap()
            .ok
    );
    local.sync_joined_group(&member_route).await.unwrap();
    assert!(local
        .groups
        .object(&original.gen_obj_id().0)
        .await
        .unwrap()
        .unwrap()
        .1
        .is_none());
    let mailbox = MailboxAddress::new(member(), Some(group().to_string())).unwrap();
    let original_record = local
        .msg_box_db
        .list_records(&mailbox, &MailboxKind::Inbox, None, false)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.msg_id == original.gen_obj_id().0)
        .unwrap();
    assert!(local
        .build_record_view(original_record, Some(true))
        .await
        .unwrap()
        .msg
        .is_none());
    call(
        &host,
        &owner(),
        "group.remove_member",
        json!({"member_did":member()}),
    )
    .await
    .unwrap();
    assert_eq!(
        local.sync_joined_group(&member_route).await.unwrap()["stopped"],
        true
    );
    assert_eq!(
        local.sync_joined_group(&owner_route).await.unwrap()["stopped"],
        false
    );
    call(
        &host,
        &owner(),
        "group.delete",
        json!({"idempotency_key":"delete"}),
    )
    .await
    .unwrap();
    assert_eq!(
        local.sync_joined_group(&owner_route).await.unwrap()["stopped"],
        true
    );
    let tombstone = host.groups.load(&group()).await.unwrap().unwrap();
    assert!(tombstone.members.is_empty() && tombstone.proofs.is_empty());
    assert_eq!(tombstone.changes.len(), 1);
    drop(local);
    let local = opened(&path).await;
    assert!(local.joined_groups(&owner()).await.unwrap()[0].stopped);
    assert!(local.joined_groups(&member()).await.unwrap()[0].stopped);
    server.abort();
}

#[tokio::test]
async fn guest_requests_are_atomic_idempotent_and_do_not_leak_other_sessions() {
    let (c, _tmp, _) = center().await;
    create(&c, json!({"membership":{"guest_entry":{"session_template":"tickets","max_open_per_guest":1}},"session_templates":{"tickets":{"membership":{"roles":["owner","admin"]},"rules":{"allow_guests":true}}}})).await;
    let p = json!({"request_id":"request1","proof":proof(&guest(),None,json!({"session_request":"request1"}),None)});
    let one = call(&c, &guest(), "group.submit_guest_request", p.clone())
        .await
        .unwrap();
    let two = call(&c, &guest(), "group.submit_guest_request", p)
        .await
        .unwrap();
    assert_eq!(one, two);
    assert_eq!(
        c.groups
            .load(&group())
            .await
            .unwrap()
            .unwrap()
            .sessions
            .len(),
        1
    );
    assert!(call(&c, &guest(), "group.submit_guest_request", json!({"request_id":"request2","proof":proof(&guest(),None,json!({"session_request":"request2"}),None)})).await.is_err());
    let sessions = c.group_sessions(&actor(guest()), &group()).await.unwrap();
    assert_eq!(sessions["items"].as_array().unwrap().len(), 1);
    assert!(sessions["group_doc"].is_null());
    assert_eq!(sessions["items"][0]["has_guests"], true);
    let sid = one["session_id"].as_str().unwrap();
    send(&c, &guest(), Some(sid), "ticket", 1).await;
    assert!(call(&c, &guest(), "group.list_members", json!({}))
        .await
        .is_err());
    let mut tx = c.groups.begin(&group()).await.unwrap();
    tx.state.configuration["access"] =
        json!({"allowed_clients":{"only":["staff"]},"allow_unverified_remote":false});
    tx.commit().await.unwrap();
    let remote = GroupActor {
        did: guest(),
        client: None,
        remote: true,
    };
    assert!(c.group_sessions(&remote, &group()).await.is_err());
}

#[tokio::test]
async fn post_hooks_cache_terminal_decisions_retry_unavailability_and_cannot_bypass_rate_limits() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let decision = Arc::new(std::sync::Mutex::new("deny".to_string()));
    let count = Arc::new(AtomicUsize::new(0));
    let hook_decision = decision.clone();
    let hook_count = count.clone();
    let (url, server) = http_fixture(move |req| {
        let decision = hook_decision.lock().unwrap().clone();
        hook_count.fetch_add(1, Ordering::SeqCst);
        async move {
            assert_eq!(req.headers()["authorization"], "Bearer group-service");
            let body: Value =
                serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(body["operation"], "post");
            assert_eq!(body["group_did"], json!(group()));
            assert!(body["msg"].is_object());
            Response::builder()
                .status(if decision == "unavailable" {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::OK
                })
                .header("content-type", "application/json")
                .body(http_body(json!({"decision":decision}).to_string()))
                .unwrap()
        }
    })
    .await;
    let (c, _tmp, _) = center().await;
    c.cyfs_dispatch.write().unwrap().group_hook_authorization = Some("Bearer group-service".into());
    create(&c, json!({"default_session":{"hooks":[{"point":"post","service":url,"may_grant":true,"on_unavailable":"deny","timeout_ms":1000}]}})).await;
    join(&c, &member()).await;
    let before = c.groups.load(&group()).await.unwrap().unwrap().group_seq;
    let denied = message(&member(), None, "denied", 1);
    let one = c
        .accept_group_message(&actor(member()), denied.clone(), None)
        .await
        .unwrap();
    assert!(!one.ok && one.reason.as_deref() == Some("hook-denied"));
    let two = c
        .accept_group_message(&actor(member()), denied, None)
        .await
        .unwrap();
    assert_eq!(one, two);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        c.groups.load(&group()).await.unwrap().unwrap().group_seq,
        before
    );
    *decision.lock().unwrap() = "unavailable".into();
    let retry = message(&member(), None, "retry", 2);
    assert!(c
        .accept_group_message(&actor(member()), retry.clone(), None)
        .await
        .is_err());
    assert!(!c
        .groups
        .load(&group())
        .await
        .unwrap()
        .unwrap()
        .operations
        .contains_key(&format!("message:{}", retry.gen_obj_id().0.to_string())));
    *decision.lock().unwrap() = "allow".into();
    assert!(
        c.accept_group_message(&actor(member()), retry, None)
            .await
            .unwrap()
            .ok
    );
    assert_eq!(count.load(Ordering::SeqCst), 3);
    let mut tx = c.groups.begin(&group()).await.unwrap();
    tx.state.configuration["default_session"]["post"] = json!("nobody");
    tx.state.configuration["default_session"]["hooks"][0]["on_unavailable"] = json!("native_only");
    tx.commit().await.unwrap();
    assert!(
        c.accept_group_message(&actor(member()), message(&member(), None, "grant", 3), None)
            .await
            .unwrap()
            .ok
    );
    *decision.lock().unwrap() = "unavailable".into();
    assert!(
        !c.accept_group_message(
            &actor(member()),
            message(&member(), None, "dynamic permission lost", 4),
            None
        )
        .await
        .unwrap()
        .ok
    );
    let mut tx = c.groups.begin(&group()).await.unwrap();
    tx.state.configuration["default_session"]["post"] = json!("all_participants");
    tx.state.configuration["limits"]["messages_per_window"] = json!(1);
    tx.state.rates.clear();
    tx.commit().await.unwrap();
    assert!(
        c.accept_group_message(
            &actor(member()),
            message(&member(), None, "native fallback", 5),
            None
        )
        .await
        .unwrap()
        .ok
    );
    let calls = count.load(Ordering::SeqCst);
    assert!(c
        .accept_group_message(
            &actor(member()),
            message(&member(), None, "rate limited", 6),
            None
        )
        .await
        .is_err());
    assert_eq!(count.load(Ordering::SeqCst), calls);
    server.abort();
}

#[tokio::test]
async fn attachments_require_the_exact_message_context_and_remote_client_policy() {
    let (c, _tmp, _) = center().await;
    create(&c, json!({})).await;
    join(&c, &member()).await;
    call(
        &c,
        &owner(),
        "group.create_session",
        json!({"session_id":"files"}),
    )
    .await
    .unwrap();
    let attachment = ndn_lib::build_named_object_by_json("file", &json!({"name":"file.txt"})).0;
    let unrelated = ndn_lib::build_named_object_by_json("file", &json!({"name":"other.txt"})).0;
    let mut msg = message(&owner(), Some("files"), "attachment", 1);
    msg.content.refs.push(ndn_lib::RefItem {
        role: ndn_lib::RefRole::Output,
        target: ndn_lib::RefTarget::DataObj {
            obj_id: attachment.clone(),
            uri_hint: None,
        },
        label: None,
    });
    assert!(
        c.accept_group_message(&actor(owner()), msg.clone(), None)
            .await
            .unwrap()
            .ok
    );
    let context_path = format!(
        "{}/{}",
        session_key(&group(), Some("files")).unwrap(),
        msg.gen_obj_id().0.to_string()
    );
    c.authorize_group_attachment(&actor(member()), &attachment, &context_path)
        .await
        .unwrap();
    assert!(c
        .authorize_group_attachment(&actor(guest()), &attachment, &context_path)
        .await
        .is_err());
    assert!(c
        .authorize_group_attachment(&actor(member()), &unrelated, &context_path)
        .await
        .is_err());
    assert!(c
        .authorize_group_attachment(
            &actor(member()),
            &attachment,
            &format!("{}/{}", group().to_string(), msg.gen_obj_id().0.to_string())
        )
        .await
        .is_err());
    let mut tx = c.groups.begin(&group()).await.unwrap();
    tx.state.configuration["access"] =
        json!({"allowed_clients":{"only":["messagehub"]},"allow_unverified_remote":false});
    tx.commit().await.unwrap();
    let remote = GroupActor {
        did: member(),
        client: None,
        remote: true,
    };
    assert!(c
        .authorize_group_attachment(&remote, &attachment, &context_path)
        .await
        .is_err());
    c.authorize_group_attachment(&actor(member()), &attachment, &context_path)
        .await
        .unwrap();
    let mut redact = message(&owner(), Some("files"), "", 2);
    redact.relates_to = Some(MsgRelation {
        rel: MsgRelType::Redact,
        target: msg.gen_obj_id().0,
        key: None,
    });
    assert!(
        c.accept_group_message(&actor(owner()), redact, None)
            .await
            .unwrap()
            .ok
    );
    assert!(c
        .authorize_group_attachment(&actor(member()), &attachment, &context_path)
        .await
        .is_err());
}

#[tokio::test]
async fn deleted_guest_sessions_keep_minimal_notifications_and_existing_local_copies() {
    let (host, _htmp, _) = center().await;
    create(&host, json!({"membership":{"guest_entry":{"session_template":"tickets","max_open_per_guest":1}},"session_templates":{"tickets":{"membership":{"roles":["owner"]},"rules":{"allow_guests":true}}}})).await;
    let session = call(&host, &guest(), "group.submit_guest_request", json!({"request_id":"ticket","proof":proof(&guest(),None,json!({"session_request":"ticket"}),None)})).await.unwrap();
    let sid = session["session_id"].as_str().unwrap();
    let msg = send(&host, &guest(), Some(sid), "my ticket", 1).await;
    let state = host.groups.load(&group()).await.unwrap().unwrap();
    let (url, server) = host_fixture(host.clone()).await;
    let mut route = joined_route(url, &guest());
    route.proof_ids = vec![state.participants[sid][&guest().to_string()]
        .proof_id
        .as_ref()
        .unwrap()
        .to_string()];
    let (local, _tmp, _) = center().await;
    local.sync_joined_group(&route).await.unwrap();
    call(
        &host,
        &owner(),
        "group.delete_session",
        json!({"session_id":sid,"expected_revision":state.sessions[sid].revision}),
    )
    .await
    .unwrap();
    local.sync_joined_group(&route).await.unwrap();
    let cached = local.joined_groups(&guest()).await.unwrap();
    assert!(cached[0].session_cache.is_empty());
    assert!(cached[0].stopped);
    assert!(local
        .groups
        .object(&msg.gen_obj_id().0)
        .await
        .unwrap()
        .unwrap()
        .1
        .is_some());
    let changes = host
        .group_changes(&actor(guest()), &group(), None, 100)
        .await
        .unwrap();
    assert!(changes["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["action"] == "session.deleted"));
    assert!(host
        .authorize_group_message(&actor(guest()), &msg.gen_obj_id().0)
        .await
        .is_err());
    server.abort();
}

#[tokio::test]
async fn generic_message_rpcs_cannot_bypass_group_actor_authentication() {
    let (c, _tmp, _) = center().await;
    create(&c, json!({})).await;
    join(&c, &member()).await;
    let msg = message(&member(), None, "authenticated", 1);
    assert!(c
        .handle_dispatch(msg.clone(), None, None, RPCContext::default())
        .await
        .is_err());
    assert!(c
        .handle_post_send(msg.clone(), None, RPCContext::default())
        .await
        .is_err());
    assert!(c
        .handle_dispatch(msg.clone(), None, None, context(&guest()))
        .await
        .is_err());
    assert!(c
        .handle_post_send(msg.clone(), None, context(&guest()))
        .await
        .is_err());
    assert!(
        c.handle_post_send(msg.clone(), None, context(&member()))
            .await
            .unwrap()
            .ok
    );
    assert!(c
        .handle_get_message(msg.gen_obj_id().0, RPCContext::default())
        .await
        .is_err());
    let mailbox = MailboxAddress::new(group(), None).unwrap();
    assert!(c
        .handle_list_box_by_time(
            mailbox.clone(),
            MailboxKind::GroupInbox,
            None,
            None,
            None,
            None,
            None,
            Some(true),
            context(&guest())
        )
        .await
        .is_err());
    assert!(!c
        .handle_list_box_by_time(
            mailbox,
            MailboxKind::GroupInbox,
            None,
            None,
            None,
            None,
            None,
            Some(true),
            context(&member())
        )
        .await
        .unwrap()
        .items
        .is_empty());
}

#[tokio::test]
async fn shadow_members_require_registered_tunnel_consent_and_authenticated_transport() {
    let (c, _tmp, _) = center().await;
    c.register_tunnel("test-tunnel".into(), owner(), "telegram".into())
        .unwrap();
    create(&c, json!({})).await;
    let shadow = DID::new("msgtunnel", "123.user.test-tunnel");
    let invite = call(
        &c,
        &owner(),
        "group.invite_member",
        json!({"member_did":shadow}),
    )
    .await
    .unwrap();
    let proof = json!({"obj_type":"buckyos.group_member_proof","schema_version":1,"group_did":group(),"member_did":shadow,"role":"member","proof_scope":"group","invite_id":invite["invite_id"],"nonce":revision(),"expires_at_ms":MessageCenter::now_ms()+60_000,"attested_by":owner(),"source_event":{"event_id":"platform-event","user_consent":true}});
    assert!(call(
        &c,
        &member(),
        "group.submit_member_proof",
        json!({"proof":proof})
    )
    .await
    .is_err());
    let mut no_consent = proof.clone();
    no_consent["source_event"]["user_consent"] = json!(false);
    assert!(call(
        &c,
        &owner(),
        "group.submit_member_proof",
        json!({"proof":no_consent})
    )
    .await
    .is_err());
    let p = json!({"proof":proof});
    let first = call(&c, &owner(), "group.submit_member_proof", p.clone())
        .await
        .unwrap();
    let second = call(&c, &owner(), "group.submit_member_proof", p)
        .await
        .unwrap();
    assert_eq!(first, second);
    let msg = message(&shadow, None, "external consented sender", 1);
    assert!(c
        .handle_dispatch(msg.clone(), None, None, context(&member()))
        .await
        .is_err());
    assert!(
        c.handle_dispatch(msg.clone(), None, None, context(&owner()))
            .await
            .unwrap()
            .ok
    );
    let mut remote_request = Request::builder()
        .method("GET")
        .uri(format!("/{}/inbox", group().to_string()))
        .header("host", "test.example")
        .header(
            "authorization",
            format!("Bearer {}", context(&shadow).token.unwrap()),
        )
        .header(ndn_lib::CYFS_HEADER_ORIGINAL_USER, shadow.to_string())
        .body(http_body(""))
        .unwrap();
    assert_eq!(
        crate::group_http::serve(&c, &mut remote_request)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn group_did_publication_is_atomic_retryable_and_survives_restart() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let available = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let success = available.clone();
    let requests = attempts.clone();
    let (url, server) = http_fixture(move |req| {
        let available = success.load(Ordering::SeqCst);
        let requests = requests.clone();
        async move {
            let request: Value =
                serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(request["method"], "sys_config_exec_tx");
            requests.lock().unwrap().push(request.clone());
            Response::builder()
                .status(if available {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                })
                .header("content-type", "application/json")
                .body(http_body(
                    json!({"sys":[request["sys"][0]],"result":0}).to_string(),
                ))
                .unwrap()
        }
    })
    .await;
    let client =
        buckyos_api::SystemConfigClient::new(Some(&url), context(&owner()).token.as_deref());
    let (c, _tmp, path) = center().await;
    create(&c, json!({})).await;
    assert!(c.publish_group_documents_with(&client).await.is_err());
    let base = format!("resolver/cache/{}/group", group().to_string());
    let first = attempts.lock().unwrap()[0].clone();
    let actions = &first["params"]["actions"];
    assert_eq!(actions[format!("{base}/state")]["action"], "update");
    let published: Value =
        serde_json::from_str(actions[format!("{base}/doc")]["value"].as_str().unwrap()).unwrap();
    assert_eq!(published["entity_type"], "group");
    assert_eq!(published["controller"], json!(owner()));
    assert!(published["iat"].as_u64().unwrap() > 0);
    drop(c);
    let c = opened(&path).await;
    available.store(true, Ordering::SeqCst);
    c.publish_group_documents_with(&client).await.unwrap();
    c.publish_group_documents_with(&client).await.unwrap();
    assert_eq!(attempts.lock().unwrap().len(), 2);
    send(&c, &owner(), None, "does not republish Doc", 1).await;
    c.publish_group_documents_with(&client).await.unwrap();
    assert_eq!(attempts.lock().unwrap().len(), 2);
    call(
        &c,
        &owner(),
        "group.delete",
        json!({"idempotency_key":"delete"}),
    )
    .await
    .unwrap();
    c.publish_group_documents_with(&client).await.unwrap();
    let requests = attempts.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let actions = &requests[2]["params"]["actions"];
    let state: Value =
        serde_json::from_str(actions[format!("{base}/state")]["value"].as_str().unwrap()).unwrap();
    assert_eq!(state["document_status"], "tombstoned");
    assert_eq!(actions[format!("{base}/doc")]["action"], "remove");
    server.abort();
}

#[test]
fn delegated_signatures_are_bound_to_the_author_document_and_authorized_key() {
    let author = DID::new("web", "author.test.example");
    let signer = member();
    let kid = format!("{}#allowed", signer.to_string());
    let doc = json!({"id":author,"authentication":[kid]});
    assert!(crate::cyfs_dispatch::authorizes_signer(
        &doc, &author, &signer, &kid
    ));
    assert!(!crate::cyfs_dispatch::authorizes_signer(
        &doc,
        &author,
        &signer,
        &format!("{}#other", signer.to_string())
    ));
    assert!(!crate::cyfs_dispatch::authorizes_signer(
        &doc,
        &group(),
        &signer,
        &kid
    ));
    let embedded = json!({"id":author,"authentication":[{"id":kid}]});
    assert!(crate::cyfs_dispatch::authorizes_signer(
        &embedded, &author, &signer, &kid
    ));
    let msg = message(&author, None, "delegated", 1);
    let jwt = msg.to_jwt(&key(&signer), &kid).unwrap();
    let verified =
        ndn_lib::verify_msg_object_jwt(&jwt, &DecodingKey::from_ed_components(&signer.id).unwrap())
            .unwrap();
    assert_eq!(verified.msg, msg);
    assert_eq!(verified.kid, kid);
}

#[tokio::test]
async fn invitation_expiration_is_persisted_once_and_can_be_reinvited() {
    let (c, _tmp, path) = center().await;
    create(&c, json!({})).await;
    let invite = call(
        &c,
        &owner(),
        "group.invite_member",
        json!({"member_did":member()}),
    )
    .await
    .unwrap();
    let mut tx = c.groups.begin(&group()).await.unwrap();
    tx.state
        .members
        .get_mut(&member().to_string())
        .unwrap()
        .expires_at_ms = Some(MessageCenter::now_ms() - 1);
    tx.commit().await.unwrap();
    call(&c, &owner(), "group.list_members", json!({}))
        .await
        .unwrap();
    let expired = c.groups.load(&group()).await.unwrap().unwrap();
    assert_eq!(
        expired.members[&member().to_string()].state,
        MemberStatus::Expired
    );
    c.expire_group_invitations(&group()).await.unwrap();
    assert_eq!(
        c.groups.load(&group()).await.unwrap().unwrap().group_seq,
        expired.group_seq
    );
    assert!(call(&c, &member(), "group.submit_member_proof", json!({"proof":proof(&member(),Some("member"),json!("group"),Some(invite["invite_id"].as_str().unwrap().into()))})).await.is_err());
    drop(c);
    let c = opened(&path).await;
    assert_eq!(
        c.groups.load(&group()).await.unwrap().unwrap().members[&member().to_string()].state,
        MemberStatus::Expired
    );
    join(&c, &member()).await;
    assert_eq!(
        c.groups.load(&group()).await.unwrap().unwrap().members[&member().to_string()].epoch,
        1
    );
}
