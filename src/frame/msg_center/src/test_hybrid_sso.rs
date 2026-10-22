//! Hybrid SSO in msg-center: restricted sessions reach only the methods that
//! open their scope, Session Only guests use hosted groups without personal
//! mailboxes, the public guest entry and request-time membership answers.

use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use crate::owner_session::{serve_method, SessionTokenVerifier, ZoneAgentInfo};
use crate::test_group_service::{http_body, TestBody};
use buckyos_api::{
    bind_token_auth_method, bind_token_ceiling, bind_token_external, bind_token_principal_kind,
    bind_token_target, token_session_ceiling, AccessDenial, AuthMethod, AuthTarget,
    ContactCheckMembershipReq, ContactQuery, DenyReason, EntryDescription, EntryState,
    MailboxAddress, MailboxKind, MsgCenterHandler, SessionCeiling, SessionScope, SsoSettings,
    SystemServiceId, TokenPrincipalKind, TokenUse, UserGroup, UserGroupDirectory, UserGroupRef,
    UserSettings, UserType,
};
use http::{Request, StatusCode};
use jsonwebtoken::{DecodingKey, EncodingKey};
use kRPC::{RPCContext, RPCErrors, RPCSessionToken, RPCSessionTokenType};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject, NamedObject, ObjId};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::{tempdir, TempDir};

type Result<T> = std::result::Result<T, RPCErrors>;

const ZONE: &str = "test.example";
const ZONE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const ZONE_PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
/// Customer service staff (User).
const SVC: &str = "svc";
/// A customer (Limited, group `customer`).
const CUST: &str = "cust";
/// Another customer (Limited).
const ALICE: &str = "alice";

/// Account DIDs are what `users/<id>/doc` says (did:dev keeps name lookups
/// off the network).
fn account(user_id: &str) -> DID {
    DID::new("dev", user_id)
}
fn external_did() -> DID {
    DID::new("dev", "bob-of-another-zone")
}
fn group() -> DID {
    DID::new("web", "support.test.example")
}

fn settings(user_id: &str, user_type: UserType, groups: &[&str]) -> UserSettings {
    serde_json::from_value(json!({
        "user_id": user_id, "type": user_type.as_str(), "password": "", "state": "active",
        "res_pool_id": "default", "groups": groups,
    }))
    .unwrap()
}

fn customer_groups() -> UserGroupDirectory {
    let group = |id: &str| UserGroup {
        group_id: id.into(),
        name: id.into(),
        description: None,
        updated_by: "admin".into(),
        created_at: 0,
        updated_at: 0,
    };
    UserGroupDirectory {
        groups: vec![group("customer"), group("service")],
        projections: Some(buckyos_api::ContactProjectionConfig {
            rules: vec![buckyos_api::ContactProjectionRule::new(
                UserGroupRef::Group("customer".into()),
                UserGroupRef::Group("service".into()),
            )],
            updated_by: "admin".into(),
            updated_at: 0,
        }),
    }
}

/// Verifies like the runtime: ordinary verification refuses restricted
/// sessions, the opened one checks the ceiling. RBAC follows the Zone's
/// policy for msg-center: users reach it, Limited accounts only their own
/// address book, External sessions nothing. Account state is read now.
struct GateVerifier {
    accounts: Mutex<HashMap<String, UserSettings>>,
    groups: Mutex<UserGroupDirectory>,
    sso: Mutex<SsoSettings>,
    unavailable: AtomicBool,
}

impl GateVerifier {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            accounts: Mutex::new(HashMap::from([
                (SVC.to_string(), settings(SVC, UserType::User, &["service"])),
                (CUST.to_string(), settings(CUST, UserType::Limited, &["customer"])),
                (ALICE.to_string(), settings(ALICE, UserType::Limited, &["customer"])),
            ])),
            groups: Mutex::new(customer_groups()),
            sso: Mutex::new(SsoSettings::default()),
            unavailable: AtomicBool::new(false),
        })
    }

    fn decode(token: &str) -> Result<RPCSessionToken> {
        let mut token = RPCSessionToken::from_string(token)?;
        token.verify_by_key(&DecodingKey::from_ed_components(ZONE_PUBLIC).unwrap())?;
        Ok(token)
    }

    fn rbac(&self, token: &RPCSessionToken, resource: &str) -> Result<()> {
        let sub = token.sub.clone().unwrap_or_default();
        let kind = buckyos_api::token_principal_kind(token)?;
        if kind == TokenPrincipalKind::System {
            return Ok(());
        }
        if buckyos_api::token_is_external(token) {
            return Err(RPCErrors::NoPermission("rbac".into()));
        }
        let user_type = self.accounts.lock().unwrap().get(&sub).map(|s| s.user_type);
        let own_contacts = MailboxAddress::new(account(&sub), None)
            .unwrap()
            .contacts_resource();
        match user_type {
            Some(UserType::Limited) if resource == own_contacts => Ok(()),
            Some(UserType::User | UserType::Admin) => Ok(()),
            _ => Err(RPCErrors::NoPermission("rbac".into())),
        }
    }

    fn read_now(&self) -> Result<()> {
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(RPCErrors::ReasonError("system-config unavailable".into()));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl SessionTokenVerifier for GateVerifier {
    async fn authorize(&self, token: &str, resource: &str, _: &str) -> Result<()> {
        let token = self.verify(token).await?;
        self.rbac(&token, resource)
    }
    async fn authorize_opened(
        &self,
        token: &str,
        resource: &str,
        _: &str,
        scope: SessionScope,
    ) -> Result<()> {
        let token = self.verify_opened(token, scope).await?;
        self.rbac(&token, resource)
    }
    async fn verify(&self, token: &str) -> Result<RPCSessionToken> {
        let token = Self::decode(token)?;
        if token_session_ceiling(&token)?.restricted {
            return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)
                .at("msg-center", "*")
                .into_rpc_error());
        }
        Ok(token)
    }
    async fn verify_opened(&self, token: &str, scope: SessionScope) -> Result<RPCSessionToken> {
        let token = Self::decode(token)?;
        if !token_session_ceiling(&token)?.allows(scope) {
            return Err(AccessDenial::new(DenyReason::SessionCeilingExceeded)
                .at("msg-center", scope.as_str())
                .into_rpc_error());
        }
        Ok(token)
    }
    async fn resolve_user_did(&self, user_id: &str) -> Result<DID> {
        Ok(account(user_id))
    }
    async fn zone_agent(&self, _: &DID) -> Result<Option<ZoneAgentInfo>> {
        Ok(None)
    }
    async fn sso_settings(&self) -> Result<SsoSettings> {
        self.read_now()?;
        Ok(self.sso.lock().unwrap().clone())
    }
    async fn account_settings(&self, user_id: &str) -> Result<Option<UserSettings>> {
        self.read_now()?;
        Ok(self.accounts.lock().unwrap().get(user_id).cloned())
    }
    async fn user_groups(&self) -> Result<UserGroupDirectory> {
        self.read_now()?;
        Ok(self.groups.lock().unwrap().clone())
    }
}

fn token(
    sub: &str,
    kind: TokenPrincipalKind,
    ceiling: SessionCeiling,
    method: Option<AuthMethod>,
    external: bool,
) -> RPCContext {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(buckyos_api::VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(sub.to_string()),
        appid: Some("control-panel".into()),
        sudo: false,
        extra: HashMap::new(),
    };
    bind_token_principal_kind(&mut token, kind);
    bind_token_target(
        &mut token,
        &AuthTarget::system(SystemServiceId::parse("control-panel").unwrap()),
        TokenUse::Session,
    )
    .unwrap();
    bind_token_ceiling(&mut token, &ceiling);
    if let Some(method) = method {
        bind_token_auth_method(&mut token, method);
    }
    if external {
        bind_token_external(&mut token);
    }
    let key = EncodingKey::from_ed_pem(ZONE_KEY.as_bytes()).unwrap();
    RPCContext {
        token: Some(token.generate_jwt(None, &key).unwrap()),
        from_ip: Some("127.0.0.1".parse().unwrap()),
        ..Default::default()
    }
}
fn user(user_id: &str) -> RPCContext {
    token(
        user_id,
        TokenPrincipalKind::User,
        SessionCeiling::unrestricted(),
        Some(AuthMethod::Password),
        false,
    )
}
fn limited(user_id: &str, method: AuthMethod) -> RPCContext {
    token(
        user_id,
        TokenPrincipalKind::User,
        SessionCeiling::for_user_type(UserType::Limited),
        Some(method),
        false,
    )
}
fn external() -> RPCContext {
    token(
        &external_did().to_string(),
        TokenPrincipalKind::User,
        SessionCeiling::external(),
        Some(AuthMethod::DidFederation),
        true,
    )
}
fn service() -> RPCContext {
    token(
        "ood1",
        TokenPrincipalKind::System,
        SessionCeiling::unrestricted(),
        None,
        false,
    )
}

async fn center() -> (MessageCenter, TempDir, Arc<GateVerifier>) {
    let tmp = tempdir().unwrap();
    let path = format!("sqlite://{}?mode=rwc", tmp.path().join("sso.db").display());
    let db = MsgBoxDbMgr::open_default_sqlite(&path).await.unwrap();
    let c = MessageCenter::open_with_db(db).await.unwrap();
    let verifier = GateVerifier::new();
    c.set_token_verifier(verifier.clone());
    c.cyfs_dispatch.write().unwrap().target_zone = Some(ZONE.into());
    c.set_message_hub_did(DID::new("web", "hub.test.example"));
    let directory = crate::test_zone_directory(
        &[
            (&account(SVC), UserType::User, &["service"]),
            (&account(CUST), UserType::Limited, &["customer"]),
            (&account(ALICE), UserType::Limited, &["customer"]),
        ],
        customer_groups(),
    );
    crate::apply_zone_directory(&c, &directory).await.unwrap();
    (c, tmp, verifier)
}

fn denial(result: Result<impl std::fmt::Debug>) -> AccessDenial {
    let error = result.expect_err("expected a denial");
    AccessDenial::from_rpc_error(&error).unwrap_or_else(|| panic!("not a D-4 denial: {error}"))
}

async fn call(c: &MessageCenter, ctx: &RPCContext, method: &str, mut p: Value) -> Result<Value> {
    p["group_did"] = json!(group());
    serve_method(method, c.group_rpc(method, p, ctx.clone())).await
}

/// The support group of `svc` with a ticket Session `cust` is invited into.
async fn support_group(c: &MessageCenter, auth_methods: Value) {
    let config = json!({
        "profile": {"name": "Support"},
        "membership": {"guest_entry": {"session_template": "tickets", "max_open_per_guest": 2, "auth_methods": auth_methods}},
        "session_templates": {"tickets": {"membership": "explicit", "rules": {"allow_guests": true}}},
    });
    call(c, &user(SVC), "group.create", json!({"idempotency_key":"create","configuration":config}))
        .await
        .unwrap();
    for sid in ["ticket-1", "ticket-2"] {
        call(
            c,
            &user(SVC),
            "group.create_session",
            json!({"session_id":sid,"membership":"explicit","rule_overrides":{"allow_guests":true}}),
        )
        .await
        .unwrap();
    }
}

fn group_message(from: &DID, sid: &str, text: &str, created: u64) -> MsgObject {
    let mut msg = MsgObject {
        from: from.clone(),
        to: vec![group()],
        kind: MsgObjKind::GroupMsg,
        to_session: Some(sid.into()),
        created_at_ms: created,
        ..Default::default()
    };
    msg.content.content = text.into();
    msg
}

fn with_attachment(mut msg: MsgObject, id: &ObjId) -> MsgObject {
    msg.content.refs.push(ndn_lib::RefItem {
        role: ndn_lib::RefRole::Output,
        target: ndn_lib::RefTarget::DataObj {
            obj_id: id.clone(),
            uri_hint: None,
        },
        label: None,
    });
    msg
}

async fn post(c: &MessageCenter, ctx: &RPCContext, msg: MsgObject) -> Result<buckyos_api::PostSendResult> {
    serve_method("msg.post_send", c.handle_post_send(msg, None, ctx.clone())).await
}

async fn rows(c: &MessageCenter, sql: &str, value: &str) -> i64 {
    use sqlx::Row;
    let sql = c.msg_box_db.render_sql(sql);
    sqlx::query(&sql)
        .bind(value)
        .fetch_one(c.msg_box_db.pool())
        .await
        .unwrap()
        .try_get::<i64, _>(0)
        .unwrap()
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// An object read: its status and body. An attachment answers 503 when it
/// was authorized (no named store in tests); a refusal is 404 with a D-4
/// denial.
async fn fetch_object(
    c: &MessageCenter,
    ctx: &RPCContext,
    id: &ObjId,
    context: Option<&str>,
) -> (StatusCode, Value) {
    use http_body_util::BodyExt;
    let mut uri = format!("{}{}", crate::object_access::OBJECTS_PATH, id.to_string());
    if let Some(context) = context {
        uri.push_str(&format!("?context_path={}", encode(context)));
    }
    let req: Request<TestBody> = Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {}", ctx.token.as_deref().unwrap()))
        .body(http_body(""))
        .unwrap();
    let response = crate::object_access::serve(c, &req).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

async fn read_object(c: &MessageCenter, ctx: &RPCContext, id: &ObjId, context: Option<&str>) -> StatusCode {
    let (status, body) = fetch_object(c, ctx, id, context).await;
    if status == StatusCode::NOT_FOUND {
        assert_eq!(body["code"], "access_denied");
        let denial: AccessDenial = serde_json::from_value(body["data"].clone()).unwrap();
        assert_eq!(denial.reason, DenyReason::ResourceGrantMissing);
    }
    status
}

#[tokio::test]
async fn restricted_sessions_reach_only_the_methods_that_open_their_scope() {
    let (c, _tmp, _) = center().await;
    let cust = limited(CUST, AuthMethod::ExclusiveInvite);
    let owner = account(CUST);

    // Owner mailboxes, sessions, notify and routing helpers stay closed.
    let sessions = serve_method(
        "msg.list_sessions",
        c.handle_list_sessions(owner.clone(), None, None, None, None, None, None, cust.clone()),
    )
    .await;
    let refused = denial(sessions);
    assert_eq!(refused.reason, DenyReason::SessionKindNotAllowed);
    assert_eq!(refused.method.as_deref(), Some("msg.list_sessions"));
    let inbox = MailboxAddress::new(owner.clone(), None).unwrap();
    assert_eq!(
        denial(serve_method("msg.peek_box", c.handle_peek_box(inbox, MailboxKind::Inbox, None, None, None, cust.clone())).await).reason,
        DenyReason::SessionKindNotAllowed
    );
    assert_eq!(
        denial(serve_method("notify.list", c.notify_rpc("notify.list", json!({}), cust.clone())).await).reason,
        DenyReason::SessionKindNotAllowed
    );
    let resolved = serve_method(
        "contact.resolve_did",
        c.handle_resolve_did("telegram".into(), "1".into(), None, None, cust.clone()),
    )
    .await;
    assert_eq!(denial(resolved).reason, DenyReason::SessionKindNotAllowed);
    // A method that opens no scope refuses even when called directly.
    assert_eq!(
        denial(c.handle_resolve_canonical_did(owner.clone(), None, cust.clone()).await).reason,
        DenyReason::SessionKindNotAllowed
    );
    assert_eq!(
        denial(serve_method("reload_settings", c.require_service_caller(&cust)).await).reason,
        DenyReason::SessionKindNotAllowed
    );

    // Its own address book (account_self); never another owner's or the
    // system scope.
    let own = serve_method(
        "contact.list_contacts",
        c.handle_list_contacts(ContactQuery::default(), Some(owner.clone()), cust.clone()),
    )
    .await;
    assert!(own.is_ok(), "{own:?}");
    let other = serve_method(
        "contact.list_contacts",
        c.handle_list_contacts(ContactQuery::default(), Some(account(SVC)), cust.clone()),
    )
    .await;
    assert_eq!(denial(other).reason, DenyReason::ResourceGrantMissing);
    let system = serve_method(
        "contact.list_contacts",
        c.handle_list_contacts(ContactQuery::default(), None, cust.clone()),
    )
    .await;
    assert_eq!(denial(system).reason, DenyReason::SessionKindNotAllowed);

    // External sessions never reach an address book.
    let bob = external();
    let external_book = serve_method(
        "contact.list_contacts",
        c.handle_list_contacts(ContactQuery::default(), Some(external_did()), bob.clone()),
    )
    .await;
    assert_eq!(denial(external_book).reason, DenyReason::SessionCeilingExceeded);

    // Group paths are open; the group ACLs decide.
    assert!(call(&c, &cust, "group.list_by_member", json!({})).await.is_ok());
    assert!(call(&c, &bob, "group.list_by_member", json!({})).await.is_ok());

    // Maintenance is for services only.
    assert!(matches!(
        serve_method("reload_settings", c.require_service_caller(&user(SVC))).await,
        Err(RPCErrors::NoPermission(_))
    ));
    assert!(serve_method("reload_settings", c.require_service_caller(&service())).await.is_ok());
}

#[tokio::test]
async fn limited_guest_joins_posts_and_reads_without_personal_mailboxes() {
    let (c, _tmp, _) = center().await;
    support_group(&c, json!({})).await;
    let svc = user(SVC);
    let cust = limited(CUST, AuthMethod::ExclusiveInvite);
    let cust_did = account(CUST);
    assert!(c.is_account_without_inbox(&cust_did));

    // Exclusive invite: the admin side invites the account's DID; the invite
    // waits in the group, nothing is delivered to a mailbox.
    call(&c, &svc, "group.invite_session_guest", json!({"session_id":"ticket-1","member_did":cust_did}))
        .await
        .unwrap();
    let pending = call(&c, &cust, "group.list_by_member", json!({})).await.unwrap();
    assert_eq!(pending["session_invitations"][0]["session_id"], "ticket-1");
    assert_eq!(pending["session_invitations"][0]["group_name"], "Support");
    // Before accepting nothing of the Session is readable.
    assert!(call(&c, &cust, "group.list_messages", json!({"session_id":"ticket-1"})).await.is_err());
    call(&c, &cust, "group.accept_session_invitation", json!({"session_id":"ticket-1"}))
        .await
        .unwrap();

    // The guest posts as itself into the hosted group without a SENT box.
    let sent = post(&c, &cust, group_message(&cust_did, "ticket-1", "my order is late", 1)).await.unwrap();
    assert!(sent.ok, "{:?}", sent.reason);
    let forged = post(&c, &cust, group_message(&account(SVC), "ticket-1", "forged", 2)).await;
    assert!(forged.is_err());
    // No media uploads in the first phase, no direct messages.
    let file = ndn_lib::build_named_object_by_json("file", &json!({"name":"photo.png"})).0;
    let upload = post(&c, &cust, with_attachment(group_message(&cust_did, "ticket-1", "photo", 3), &file)).await;
    assert_eq!(denial(upload).reason, DenyReason::SessionKindNotAllowed);
    let mut direct = group_message(&cust_did, "ticket-1", "hi", 4);
    direct.kind = MsgObjKind::Chat;
    direct.to = vec![account(SVC)];
    direct.to_session = None;
    assert_eq!(denial(post(&c, &cust, direct).await).reason, DenyReason::SessionKindNotAllowed);

    // Staff answer with an attachment; the guest reads the timeline from the
    // group alone and the attachment through its message.
    let reply = with_attachment(group_message(&account(SVC), "ticket-1", "see the label", 5), &file);
    let answered = post(&c, &svc, reply.clone()).await.unwrap();
    assert!(answered.ok, "{:?}", answered.reason);
    let page = call(&c, &cust, "group.list_messages", json!({"session_id":"ticket-1","with_object":true}))
        .await
        .unwrap();
    let texts: Vec<_> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["msg"]["kind"] == "group_msg")
        .map(|item| item["msg"]["content"]["content"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(texts, vec!["my order is late", "see the label"]);
    assert!(call(&c, &cust, "group.changes", json!({})).await.is_ok());
    let sessions = call(&c, &cust, "group.list_sessions", json!({})).await.unwrap();
    assert_eq!(sessions["items"].as_array().unwrap().len(), 1);
    let context = format!(
        "{}/{}",
        crate::group_types::session_key(&group(), Some("ticket-1")).unwrap(),
        reply.gen_obj_id().0.to_string()
    );
    assert_eq!(read_object(&c, &cust, &file, Some(&context)).await, StatusCode::SERVICE_UNAVAILABLE);
    // Message bodies come through the same route, the message as context.
    let session = crate::group_types::session_key(&group(), Some("ticket-1")).unwrap();
    let reply_id = reply.gen_obj_id().0;
    let (status, body) =
        fetch_object(&c, &cust, &reply_id, Some(&format!("{session}/{}", reply_id.to_string()))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["content"]["content"], "see the label");
    assert_eq!(read_object(&c, &cust, &reply_id, None).await, StatusCode::NOT_FOUND);
    // No context, a guessed object or another Session's context: refused.
    assert_eq!(read_object(&c, &cust, &file, None).await, StatusCode::NOT_FOUND);
    let guessed = ndn_lib::build_named_object_by_json("file", &json!({"name":"secret.pdf"})).0;
    assert_eq!(read_object(&c, &cust, &guessed, Some(&context)).await, StatusCode::NOT_FOUND);
    let other_ticket = with_attachment(group_message(&account(SVC), "ticket-2", "other ticket", 6), &guessed);
    assert!(post(&c, &svc, other_ticket.clone()).await.unwrap().ok);
    let other_context = format!(
        "{}/{}",
        crate::group_types::session_key(&group(), Some("ticket-2")).unwrap(),
        other_ticket.gen_obj_id().0.to_string()
    );
    assert_eq!(read_object(&c, &cust, &guessed, Some(&other_context)).await, StatusCode::NOT_FOUND);
    assert!(call(&c, &cust, "group.list_messages", json!({"session_id":"ticket-2"})).await.is_err());
    // Another customer of the Zone sees none of it.
    let alice = limited(ALICE, AuthMethod::Password);
    assert_eq!(read_object(&c, &alice, &file, Some(&context)).await, StatusCode::NOT_FOUND);
    assert!(call(&c, &alice, "group.list_messages", json!({"session_id":"ticket-1"})).await.is_err());

    // Nothing personal was created for the guest; staff keep their mailbox.
    let owner = cust_did.to_string();
    assert_eq!(rows(&c, "SELECT COUNT(*) FROM mailbox_records WHERE owner=?", &owner).await, 0);
    assert_eq!(rows(&c, "SELECT COUNT(*) FROM owner_sessions WHERE owner=?", &owner).await, 0);
    assert_eq!(rows(&c, "SELECT COUNT(*) FROM delivery_records WHERE target_did=?", &owner).await, 0);
    assert_eq!(rows(&c, "SELECT COUNT(*) FROM msg_refs WHERE owner=?", &owner).await, 0);
    assert!(rows(&c, "SELECT COUNT(*) FROM mailbox_records WHERE owner=?", &account(SVC).to_string()).await > 0);

    // Removal takes effect on the next request: no reads, posts or objects.
    let revision = c.groups.load(&group()).await.unwrap().unwrap().sessions["ticket-1"].revision.clone();
    call(
        &c,
        &svc,
        "group.remove_session_member",
        json!({"session_id":"ticket-1","member_did":cust_did,"expected_revision":revision}),
    )
    .await
    .unwrap();
    let listed = denial(call(&c, &cust, "group.list_messages", json!({"session_id":"ticket-1"})).await);
    assert_eq!(listed.reason, DenyReason::ResourceGrantMissing);
    // The group's own reason would tell an existing group from a missing one.
    assert_eq!(listed.detail, None);
    let elsewhere = json!({"group_did":DID::new("web","none.test.example"),"session_id":"ticket-1"});
    let missing = serve_method(
        "group.list_messages",
        c.group_rpc("group.list_messages", elsewhere, cust.clone()),
    )
    .await;
    assert_eq!(denial(missing), listed);
    assert_eq!(read_object(&c, &cust, &file, Some(&context)).await, StatusCode::NOT_FOUND);
    let message_context = format!("{session}/{}", reply_id.to_string());
    assert_eq!(read_object(&c, &cust, &reply_id, Some(&message_context)).await, StatusCode::NOT_FOUND);
    let after = post(&c, &cust, group_message(&cust_did, "ticket-1", "still there?", 7)).await;
    assert_eq!(denial(after).reason, DenyReason::ResourceGrantMissing);
    assert_eq!(rows(&c, "SELECT COUNT(*) FROM mailbox_records WHERE owner=?", &owner).await, 0);
}

#[tokio::test]
async fn guest_entry_is_described_publicly_and_admits_only_allowed_methods() {
    let (c, _tmp, verifier) = center().await;
    let describe = |group: DID| {
        let c = c.clone();
        async move {
            serde_json::from_value::<EntryDescription>(
                c.describe_group_entry(&json!({"group_did":group})).await.unwrap(),
            )
            .unwrap()
        }
    };
    assert_eq!(describe(group()).await, EntryDescription::unavailable());
    support_group(&c, json!({"password":true})).await;
    let entry = describe(group()).await;
    assert_eq!(entry.state, EntryState::Active);
    assert!(entry.methods.password && !entry.methods.guest && !entry.methods.did_login);
    let resource = entry.resource.unwrap();
    assert_eq!(resource.kind, "message_group");
    assert_eq!(resource.title, "Support");
    assert_eq!(resource.path, format!("/messagehub/guest?group={}", group().to_string()));

    // A local account signed in by password or exclusive invite opens a
    // ticket; a DID login needs `did_login` on the entry.
    let alice = limited(ALICE, AuthMethod::Password);
    let ticket = call(&c, &alice, "group.submit_guest_request", json!({"request_id":"r1"}))
        .await
        .unwrap();
    let sid = ticket["session_id"].as_str().unwrap().to_string();
    let sent = post(&c, &alice, group_message(&account(ALICE), &sid, "hello", 1)).await.unwrap();
    assert!(sent.ok, "{:?}", sent.reason);
    let bob = external();
    let refused = call(&c, &bob, "group.submit_guest_request", json!({"request_id":"r2"})).await;
    assert_eq!(denial(refused).reason, DenyReason::EntryUnavailable);

    // The owner allows DID login; the Zone switch still decides.
    let revision = c.groups.load(&group()).await.unwrap().unwrap().config().unwrap().revision;
    call(
        &c,
        &user(SVC),
        "group.apply_config",
        json!({"expected_revision":revision,"idempotency_key":"did","patch":{"membership":{"guest_entry":{"auth_methods":{"password":true,"did_login":true}}}}}),
    )
    .await
    .unwrap();
    assert!(describe(group()).await.methods.did_login);
    verifier.sso.lock().unwrap().did_login_enabled = false;
    let closed = call(&c, &bob, "group.submit_guest_request", json!({"request_id":"r2"})).await;
    assert_eq!(denial(closed).reason, DenyReason::ZoneMethodDisabled);
    verifier.unavailable.store(true, Ordering::SeqCst);
    let unknown = call(&c, &bob, "group.submit_guest_request", json!({"request_id":"r2"})).await;
    assert_eq!(denial(unknown).reason, DenyReason::AuthorizationUnavailable);
    verifier.unavailable.store(false, Ordering::SeqCst);
    verifier.sso.lock().unwrap().did_login_enabled = true;
    let opened = call(&c, &bob, "group.submit_guest_request", json!({"request_id":"r2"}))
        .await
        .unwrap();
    // The External guest uses its Session as the page does.
    let bob_sid = opened["session_id"].as_str().unwrap().to_string();
    let hello = group_message(&external_did(), &bob_sid, "from another zone", 2);
    assert!(post(&c, &bob, hello.clone()).await.unwrap().ok);
    let page = call(&c, &bob, "group.list_messages", json!({"session_id":bob_sid})).await.unwrap();
    assert!(page["items"].as_array().unwrap().iter().any(|item| item["obj_id"] == json!(hello.gen_obj_id().0)));
    let hello_id = hello.gen_obj_id().0;
    let context = format!(
        "{}/{}",
        crate::group_types::session_key(&group(), Some(&bob_sid)).unwrap(),
        hello_id.to_string()
    );
    assert_eq!(fetch_object(&c, &bob, &hello_id, Some(&context)).await.0, StatusCode::OK);
    call(&c, &bob, "group.update_read_marker", json!({"session_id":bob_sid,"last_read_seq":1}))
        .await
        .unwrap();
    assert!(call(&c, &bob, "group.list_session_members", json!({"session_id":bob_sid})).await.is_ok());
    assert!(call(&c, &bob, "group.get_doc", json!({})).await.is_ok());
    // Another visitor's Session stays out of reach.
    let foreign = call(&c, &bob, "group.list_messages", json!({"session_id":sid})).await;
    assert_eq!(denial(foreign).reason, DenyReason::ResourceGrantMissing);

    // An archived group has no entry.
    call(&c, &user(SVC), "group.archive", json!({"idempotency_key":"archive"}))
        .await
        .unwrap();
    assert_eq!(describe(group()).await.state, EntryState::Unavailable);
    let archived = call(&c, &alice, "group.submit_guest_request", json!({"request_id":"r3"})).await;
    assert_eq!(denial(archived).reason, DenyReason::EntryUnavailable);
}

#[tokio::test]
async fn group_http_keeps_the_real_client_of_a_restricted_account() {
    let (c, _tmp, _) = center().await;
    support_group(&c, json!({})).await;
    let cust_did = account(CUST);
    call(&c, &user(SVC), "group.invite_session_guest", json!({"session_id":"ticket-1","member_did":cust_did}))
        .await
        .unwrap();
    let cust = limited(CUST, AuthMethod::ExclusiveInvite);
    call(&c, &cust, "group.accept_session_invitation", json!({"session_id":"ticket-1"}))
        .await
        .unwrap();
    let sessions = |ctx: &RPCContext, did: &DID| {
        let mut req: Request<TestBody> = Request::builder()
            .uri(format!("/{}/sessions", group().to_string()))
            .header("host", ZONE)
            .header("authorization", format!("Bearer {}", ctx.token.as_deref().unwrap()))
            .header(ndn_lib::CYFS_HEADER_ORIGINAL_USER, did.to_string())
            .body(http_body(""))
            .unwrap();
        let c = c.clone();
        async move { crate::group_http::serve(&c, &mut req).await.unwrap().status() }
    };
    assert_eq!(sessions(&cust, &cust_did).await, StatusCode::OK);
    // Only the Message Hub client may read; a Limited session issued to
    // another client does not pass as an unverified remote.
    let mut tx = c.groups.begin(&group()).await.unwrap();
    tx.state.configuration["access"] =
        json!({"allowed_clients":{"only":["messagehub"]},"allow_unverified_remote":true});
    tx.commit().await.unwrap();
    assert_eq!(sessions(&cust, &cust_did).await, StatusCode::FORBIDDEN);
    assert!(call(&c, &cust, "group.list_sessions", json!({})).await.is_err());
}

#[tokio::test]
async fn an_account_is_its_doc_did_whatever_its_profile_says() {
    let (c, _tmp, _) = center().await;
    support_group(&c, json!({})).await;
    // `cust` wrote svc's DID into its own profile; msg-center resolves the
    // account through `users/<id>/doc` only, so it stays `cust`.
    let cust = limited(CUST, AuthMethod::Password);
    let inbox = MailboxAddress::new(account(SVC), None).unwrap();
    assert!(serve_method(
        "msg.list_box_by_time",
        c.handle_list_box_by_time(inbox, MailboxKind::Inbox, None, None, None, None, None, None, cust.clone())
    )
    .await
    .is_err());
    let book = serve_method(
        "contact.list_contacts",
        c.handle_list_contacts(ContactQuery::default(), Some(account(SVC)), cust.clone()),
    )
    .await;
    assert_eq!(denial(book).reason, DenyReason::ResourceGrantMissing);
    assert!(call(&c, &cust, "group.list_members", json!({})).await.is_err());
    // Limited accounts are no local recipients: no personal inbox.
    assert!(!c.is_local_recipient(&account(CUST)));
    assert!(c.is_local_recipient(&account(SVC)));
    // Membership answers name the account by its DID.
    let member = c
        .check_contact_membership(&ContactCheckMembershipReq {
            owner: account(SVC),
            did: account(CUST),
            tag: "group:customer".into(),
        })
        .await
        .unwrap();
    assert!(member.member);
}

#[tokio::test]
async fn restricted_contact_edits_follow_the_account_settings_now() {
    let (c, _tmp, verifier) = center().await;
    let edit = |user_id: &'static str| {
        let c = c.clone();
        async move {
            serve_method(
                "contact.update_tags",
                c.contact_ext_rpc(
                    "contact.update_tags",
                    json!({"did":account(SVC),"add":["support"],"contact_mgr_owner":account(user_id)}),
                    limited(user_id, AuthMethod::Password),
                ),
            )
            .await
        }
    };
    // A Limited account may not edit its book unless allowed, whether or not
    // the last zone sync knew it.
    assert!(matches!(edit(CUST).await, Err(RPCErrors::NoPermission(_))));
    verifier
        .accounts
        .lock()
        .unwrap()
        .insert("newbie".into(), settings("newbie", UserType::Limited, &[]));
    assert!(c.contact_mgr.owner_can_edit(&account("newbie")));
    assert!(matches!(edit("newbie").await, Err(RPCErrors::NoPermission(_))));
    verifier
        .accounts
        .lock()
        .unwrap()
        .get_mut(CUST)
        .unwrap()
        .allow_contact_edit = Some(true);
    let allowed = edit(CUST).await;
    assert!(allowed.is_ok(), "{allowed:?}");
    verifier.unavailable.store(true, Ordering::SeqCst);
    assert_eq!(denial(edit(CUST).await).reason, DenyReason::AuthorizationUnavailable);
}

#[tokio::test]
async fn accounts_newer_than_the_zone_sync_get_no_personal_records() {
    let (c, _tmp, verifier) = center().await;
    {
        let mut accounts = verifier.accounts.lock().unwrap();
        accounts.insert("newbie".into(), settings("newbie", UserType::Limited, &[]));
        accounts.insert("staff2".into(), settings("staff2", UserType::User, &[]));
    }
    let local = |name: &str| DID::new("web", &format!("{name}.{ZONE}"));
    assert!(!c.unsynced_account_is_personal(&local("newbie"), ZONE).await);
    assert!(c.unsynced_account_is_personal(&local("staff2"), ZONE).await);
    // No account of that name: an Agent or an alias under the Zone host.
    assert!(c.unsynced_account_is_personal(&local("xiaobai"), ZONE).await);
    verifier.unavailable.store(true, Ordering::SeqCst);
    assert!(!c.unsynced_account_is_personal(&local("staff2"), ZONE).await);
    verifier.unavailable.store(false, Ordering::SeqCst);

    // A direct message never lands in the inbox of an account without one.
    let mut direct = group_message(&account(SVC), "x", "hi", 1);
    direct.kind = MsgObjKind::Chat;
    direct.to = vec![account(CUST)];
    direct.to_session = None;
    let result = c
        .handle_dispatch(direct, None, None, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(result.dropped_recipients, vec![account(CUST)]);
    let owner = account(CUST).to_string();
    assert_eq!(rows(&c, "SELECT COUNT(*) FROM mailbox_records WHERE owner=?", &owner).await, 0);
}
