use crate::message_hub::MessageHubExecutor;
use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use crate::msg_tunnel::DeliveryExecutor;
use crate::network_directory::test_support::StaticDirectory;
use crate::network_directory::ZoneSigner;
use crate::owner_session::{SessionTokenVerifier, ZoneAgentInfo};
use crate::test_group_service::http_fixture;
use buckyos_api::{
    bind_token_principal_kind, bind_token_target, AccessGroupLevel, AuthTarget, Contact,
    ContactQuery, DeliveryRecordWithObject, FriendRequest, FriendRequestPage, FriendRequestResult,
    FriendState, LookupKind, LookupResult, MailboxAddress, MailboxKind, MsgCenterHandler,
    SystemServiceId, TokenPrincipalKind, TokenUse,
};
use jsonwebtoken::{DecodingKey, EncodingKey};
use kRPC::{RPCContext, RPCErrors, RPCSessionToken, RPCSessionTokenType};
use name_lib::DID;
use ndn_lib::{MsgContent, MsgObjKind, MsgObject};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};

const ZONE_A_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const ZONE_A_DEVICE: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
const ZONE_B_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIF5zplRSY5MsYbBMdbzItH90daUlrx/OflPm8Kp9m2j1\n-----END PRIVATE KEY-----";
const ZONE_B_DEVICE: &str = "mL_tm5diVGGkpt_6WmCZQxUsee4rHacyz7mx7TkKb-0";

fn user(name: &str, zone: &str) -> DID {
    DID::new("web", &format!("{name}.{zone}"))
}

fn hub() -> DID {
    DID::new("web", "hub.local.example")
}

async fn open(tag: &str, zone: &str, users: &[DID]) -> (MessageCenter, TempDir) {
    let tmp = tempdir().unwrap();
    let path = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join(format!("{tag}.db")).display()
    );
    let db = MsgBoxDbMgr::open_default_sqlite(&path).await.unwrap();
    let center = MessageCenter::open_with_db(db).await.unwrap();
    center.cyfs_dispatch.write().unwrap().target_zone = Some(zone.into());
    center.register_local_recipients(users.iter().cloned());
    center.set_message_hub_did(hub());
    (center, tmp)
}

async fn rpc(c: &MessageCenter, method: &str, params: Value) -> Result<Value, RPCErrors> {
    c.contact_ext_rpc(method, params, RPCContext::default()).await
}

async fn send_request(c: &MessageCenter, owner: &DID, peer: &DID) -> FriendRequestResult {
    serde_json::from_value(
        rpc(
            c,
            "contact.send_friend_request",
            json!({"did":peer,"message":"hi","contact_mgr_owner":owner}),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}

async fn respond(c: &MessageCenter, owner: &DID, request_id: &str, decision: &str) -> FriendRequestResult {
    serde_json::from_value(
        rpc(
            c,
            "contact.respond_friend_request",
            json!({"request_id":request_id,"decision":decision,"contact_mgr_owner":owner}),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}

async fn requests(c: &MessageCenter, owner: &DID) -> Vec<FriendRequest> {
    let page: FriendRequestPage = serde_json::from_value(
        rpc(
            c,
            "contact.list_friend_requests",
            json!({"contact_mgr_owner":owner}),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    page.items
}

async fn contact(c: &MessageCenter, owner: &DID, peer: &DID) -> Option<Contact> {
    c.contact_mgr
        .get_contact(peer.clone(), Some(owner.clone()))
        .await
        .unwrap()
}

async fn friend_state(c: &MessageCenter, owner: &DID, peer: &DID) -> FriendState {
    contact(c, owner, peer)
        .await
        .map(|c| c.friend_state)
        .unwrap_or_default()
}

async fn dm_records(c: &MessageCenter, owner: &DID, peer: &DID, kind: MailboxKind) -> usize {
    let mailbox = MailboxAddress::new(owner.clone(), Some(format!("dm:{}", peer.to_string()))).unwrap();
    c.msg_box_db
        .list_records(&mailbox, &kind, None, false)
        .await
        .unwrap()
        .len()
}

/// Run the native executor over everything queued, as the delivery pump does.
async fn pump(c: &MessageCenter) -> usize {
    let executor = MessageHubExecutor::new(hub(), c.clone());
    let mut delivered = 0;
    while let Some(record) = c
        .msg_box_db
        .take_next_delivery(&hub(), u64::MAX / 2, true)
        .await
        .unwrap()
    {
        let msg = c.load_message(&record.envelope.msg_id).await.unwrap();
        let id = record.delivery_id.clone();
        let report = executor
            .execute_delivery(DeliveryRecordWithObject {
                record,
                msg: Some(msg),
            })
            .await
            .unwrap();
        if report.ok {
            delivered += 1;
        }
        c.handle_report_delivery(id, report, RPCContext::default())
            .await
            .unwrap();
    }
    delivered
}

#[tokio::test]
async fn friend_requests_flow_inside_one_zone() {
    let alice = user("alice", "z.example");
    let bob = user("bob", "z.example");
    let (c, _tmp) = open("one-zone", "z.example", &[alice.clone(), bob.clone()]).await;

    let sent = send_request(&c, &alice, &bob).await;
    assert_eq!(sent.friend_state, FriendState::OutgoingPending);
    let request = sent.request.unwrap();
    assert_eq!(
        request.delivery.as_ref().map(|d| d.state),
        Some(buckyos_api::FriendDeliveryState::Delivered)
    );
    assert_eq!(friend_state(&c, &bob, &alice).await, FriendState::IncomingPending);
    assert_eq!(dm_records(&c, &bob, &alice, MailboxKind::RequestBox).await, 1);
    assert_eq!(dm_records(&c, &alice, &bob, MailboxKind::Sent).await, 1);
    let incoming = requests(&c, &bob).await;
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].request_id, request.request_id);
    assert_eq!(incoming[0].message.as_deref(), Some("hi"));

    // Replaying the same request message changes nothing.
    let msg = c
        .load_message(&ndn_lib::ObjId::new(request.msg_id.as_deref().unwrap()).unwrap())
        .await
        .unwrap();
    c.dispatch_to_receiver(msg, None, bob.clone(), "cyfs://local/replay/inbox".into())
        .await
        .unwrap();
    assert_eq!(requests(&c, &bob).await.len(), 1);
    // Asking again while pending returns the same request.
    assert_eq!(
        send_request(&c, &alice, &bob).await.request.unwrap().request_id,
        request.request_id
    );

    let accepted = respond(&c, &bob, &request.request_id, "accept").await;
    assert_eq!(accepted.friend_state, FriendState::Friend);
    assert_eq!(friend_state(&c, &alice, &bob).await, FriendState::Friend);
    for (owner, peer) in [(&alice, &bob), (&bob, &alice)] {
        assert_eq!(
            contact(&c, owner, peer).await.unwrap().access_level,
            AccessGroupLevel::Allow
        );
    }
    assert_eq!(
        requests(&c, &alice).await[0].state,
        buckyos_api::FriendRequestState::Accepted
    );
    // Accept / control messages leave no mailbox record.
    assert_eq!(dm_records(&c, &alice, &bob, MailboxKind::Inbox).await, 0);

    let removed: FriendRequestResult = serde_json::from_value(
        rpc(
            &c,
            "contact.remove_friend",
            json!({"did":bob,"contact_mgr_owner":alice}),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(removed.friend_state, FriendState::None);
    assert_eq!(friend_state(&c, &bob, &alice).await, FriendState::None);
    assert_eq!(
        contact(&c, &bob, &alice).await.unwrap().access_level,
        AccessGroupLevel::Stranger
    );
}

#[tokio::test]
async fn ignore_cancel_block_and_cooldown() {
    let zone = "z.example";
    let bob = user("bob", zone);
    let carol = user("carol", zone);
    let dave = user("dave", zone);
    let gina = user("gina", zone);
    let (c, _tmp) = open("rules", zone, &[bob.clone(), carol.clone(), dave.clone(), gina.clone()]).await;

    let first = send_request(&c, &carol, &bob).await.request.unwrap();
    let ignored = respond(&c, &bob, &first.request_id, "ignore").await;
    assert_eq!(ignored.friend_state, FriendState::None);
    assert_eq!(friend_state(&c, &bob, &carol).await, FriendState::None);
    // The sender is not told; a new request inside the cooldown is dropped.
    assert_eq!(friend_state(&c, &carol, &bob).await, FriendState::OutgoingPending);
    rpc(
        &c,
        "contact.cancel_friend_request",
        json!({"request_id":first.request_id,"contact_mgr_owner":carol}),
    )
    .await
    .unwrap();
    let again = send_request(&c, &carol, &bob).await.request.unwrap();
    assert_ne!(again.request_id, first.request_id);
    assert!(requests(&c, &bob)
        .await
        .iter()
        .all(|r| r.request_id != again.request_id));

    let pending = send_request(&c, &dave, &bob).await.request.unwrap();
    assert_eq!(friend_state(&c, &bob, &dave).await, FriendState::IncomingPending);
    rpc(
        &c,
        "contact.cancel_friend_request",
        json!({"request_id":pending.request_id,"contact_mgr_owner":dave}),
    )
    .await
    .unwrap();
    assert_eq!(friend_state(&c, &bob, &dave).await, FriendState::None);
    assert!(requests(&c, &bob)
        .await
        .iter()
        .any(|r| r.request_id == pending.request_id
            && r.state == buckyos_api::FriendRequestState::Cancelled));

    c.handle_block_contact(gina.clone(), None, Some(bob.clone()), RPCContext::default())
        .await
        .unwrap();
    let blocked = send_request(&c, &gina, &bob).await.request.unwrap();
    assert_eq!(
        blocked.delivery.map(|d| d.state),
        Some(buckyos_api::FriendDeliveryState::Failed)
    );
    assert!(requests(&c, &bob)
        .await
        .iter()
        .all(|r| r.peer_did != gina));
    assert_eq!(dm_records(&c, &bob, &gina, MailboxKind::RequestBox).await, 0);
    // The owner cannot ask someone they block.
    assert!(rpc(
        &c,
        "contact.send_friend_request",
        json!({"did":gina,"contact_mgr_owner":bob}),
    )
    .await
    .is_err());
}

#[tokio::test]
async fn zone_relations_are_friends_and_cannot_be_removed() {
    let zone = "z.example";
    let alice = user("alice", zone);
    let bob = user("bob", zone);
    let (c, _tmp) = open("system", zone, &[alice.clone(), bob.clone()]).await;
    c.upsert_zone_user_contacts(
        vec![crate::contact_mgr::ZoneUserContactSeed {
            did: bob.clone(),
            name: "Bob".into(),
            avatar: None,
            note: None,
            bindings: vec![],
            tags: vec![buckyos_api::CONTACT_TAG_ZONE_USER.into()],
        }],
        Some(alice.clone()),
    )
    .await
    .unwrap();
    assert_eq!(friend_state(&c, &alice, &bob).await, FriendState::Friend);
    let sent = send_request(&c, &alice, &bob).await;
    assert_eq!(sent.friend_state, FriendState::Friend);
    assert!(sent.request.is_none());
    for method in ["contact.remove_friend", "contact.remove_contact"] {
        let error = rpc(&c, method, json!({"did":bob,"contact_mgr_owner":alice}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("system-relation"), "{method}: {error}");
    }
    let friends = c
        .contact_mgr
        .list_contacts(
            ContactQuery {
                friend_state: Some(FriendState::Friend),
                tag: Some(buckyos_api::CONTACT_TAG_ZONE_USER.into()),
                ..Default::default()
            },
            Some(alice.clone()),
        )
        .await
        .unwrap();
    assert_eq!(friends.len(), 1);
}

#[tokio::test]
async fn removing_a_contact_withdraws_requests_and_keeps_history() {
    let zone = "z.example";
    let alice = user("alice", zone);
    let bob = user("bob", zone);
    let (c, _tmp) = open("remove", zone, &[alice.clone(), bob.clone()]).await;
    send_request(&c, &alice, &bob).await;
    rpc(&c, "contact.remove_contact", json!({"did":bob,"contact_mgr_owner":alice}))
        .await
        .unwrap();
    assert!(contact(&c, &alice, &bob).await.is_none());
    assert_eq!(friend_state(&c, &bob, &alice).await, FriendState::None);
    assert_eq!(dm_records(&c, &alice, &bob, MailboxKind::Sent).await, 1);
    let unknown = rpc(&c, "contact.remove_contact", json!({"did":bob,"contact_mgr_owner":alice}))
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("not-found"));
}

#[tokio::test]
async fn lookup_merges_relations_zone_and_resolved_candidates() {
    let zone = "z.example";
    let alice = user("alice", zone);
    let bob = user("bob", zone);
    let remote = DID::new("bns", "nora");
    let (c, _tmp) = open("lookup", zone, &[alice.clone(), bob.clone()]).await;
    c.directory.set(StaticDirectory::with(vec![(
        remote.clone(),
        LookupKind::User,
        "Nora",
        "nora.example",
    )]));
    c.upsert_zone_user_contacts(
        vec![crate::contact_mgr::ZoneUserContactSeed {
            did: bob.clone(),
            name: "Bob Chen".into(),
            avatar: None,
            note: None,
            bindings: vec![],
            tags: vec![],
        }],
        Some(alice.clone()),
    )
    .await
    .unwrap();
    let lookup = |input: &str| {
        let c = c.clone();
        let alice = alice.clone();
        let input = input.to_string();
        async move {
            serde_json::from_value::<LookupResult>(
                rpc(&c, "contact.lookup", json!({"input":input,"contact_mgr_owner":alice}))
                    .await
                    .unwrap(),
            )
            .unwrap()
        }
    };
    let text = lookup("Bob Ch").await;
    assert_eq!(text.candidates.len(), 1);
    assert_eq!(text.candidates[0].did, bob);
    assert_eq!(text.candidates[0].section, buckyos_api::LookupSection::Zone);
    assert_eq!(text.candidates[0].relation.friend_state, FriendState::Friend);
    assert_eq!(
        text.candidates[0].reachability.state,
        buckyos_api::ReachabilityState::Local
    );

    // A bare name of someone already known does not go to the name service.
    let known = lookup("bob").await;
    assert_eq!(known.input_kind, buckyos_api::LookupInputKind::Name);
    assert_eq!(known.candidates.len(), 1);
    assert_eq!(known.candidates[0].did, bob);
    assert!(known.error.is_none());

    let resolved = lookup("nora").await;
    assert_eq!(resolved.input_kind, buckyos_api::LookupInputKind::Name);
    let nora = &resolved.candidates[0];
    assert_eq!(nora.did, remote);
    assert_eq!(nora.name.as_deref(), Some("Nora"));
    assert_eq!(nora.section, buckyos_api::LookupSection::Resolved);
    assert_eq!(nora.reachability.state, buckyos_api::ReachabilityState::Routable);
    assert_eq!(nora.reachability.zone.as_deref(), Some("nora.example"));
    assert!(!nora.relation.is_contact);

    let missing = lookup("did:bns:ghost").await;
    assert!(missing.candidates.is_empty());
    assert_eq!(missing.error.unwrap().code, "not-found");
    let bad = lookup("did:web:g.example?nothing=1").await;
    assert_eq!(bad.error.unwrap().code, "invalid-input");
}

#[tokio::test]
async fn lookup_finds_remote_user_from_zone_url_without_creating_contact() {
    let zone = "z.example";
    let alice = user("alice", zone);
    let remote = DID::new("bns", "lurenpluto");
    let (c, _tmp) = open("lookup-url", zone, &[alice.clone()]).await;
    c.directory.set(StaticDirectory::with(vec![(
        remote.clone(),
        LookupKind::User,
        "lurenpluto",
        "lurenpluto.web3.buckyos.ai",
    )]));
    for input in [
        "https://lurenpluto.web3.buckyos.ai",
        "https://lurenpluto.web3.buckyos.ai/",
        "lurenpluto.web3.buckyos.ai",
    ] {
        let result: LookupResult = serde_json::from_value(
            rpc(
                &c,
                "contact.lookup",
                json!({
                    "input":input,
                    "kinds":["user","agent","msgtunnel","unknown"],
                    "contact_mgr_owner":alice,
                }),
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(result.input, input);
        assert_eq!(result.input_kind, buckyos_api::LookupInputKind::Name);
        assert!(result.error.is_none());
        assert_eq!(result.candidates.len(), 1);
        let candidate = &result.candidates[0];
        assert_eq!(candidate.did, remote);
        assert_eq!(candidate.kind, LookupKind::User);
        assert_eq!(candidate.name.as_deref(), Some("lurenpluto"));
        assert_eq!(candidate.section, buckyos_api::LookupSection::Resolved);
        assert_eq!(
            candidate.reachability.state,
            buckyos_api::ReachabilityState::Routable
        );
        assert_eq!(
            candidate.reachability.zone.as_deref(),
            Some("lurenpluto.web3.buckyos.ai")
        );
        assert!(!candidate.relation.is_contact);
        assert!(contact(&c, &alice, &remote).await.is_none());
    }
}

struct Verifier;

#[async_trait::async_trait]
impl SessionTokenVerifier for Verifier {
    async fn authorize(&self, _: &str, resource: &str, _: &str) -> Result<(), RPCErrors> {
        if resource.ends_with("/contact_tags/aiworkspace") {
            return Err(RPCErrors::NoPermission("rbac".into()));
        }
        Ok(())
    }
    async fn verify(&self, t: &str) -> Result<RPCSessionToken, RPCErrors> {
        let mut t = RPCSessionToken::from_string(t)?;
        t.verify_by_key(&DecodingKey::from_ed_components(ZONE_A_DEVICE).unwrap())?;
        Ok(t)
    }
    async fn resolve_user_did(&self, id: &str) -> Result<DID, RPCErrors> {
        DID::from_str(id).map_err(|e| RPCErrors::ParseRequestError(e.to_string()))
    }
    async fn zone_agent(&self, _: &DID) -> Result<Option<ZoneAgentInfo>, RPCErrors> {
        Ok(None)
    }
}

fn token_context(sub: &str, kind: TokenPrincipalKind, holder: &str) -> RPCContext {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(buckyos_api::VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(sub.to_string()),
        appid: Some(holder.into()),
        sudo: false,
        extra: HashMap::new(),
    };
    bind_token_principal_kind(&mut token, kind);
    bind_token_target(
        &mut token,
        &AuthTarget::system(SystemServiceId::parse(holder).unwrap()),
        TokenUse::Session,
    )
    .unwrap();
    let key = EncodingKey::from_ed_pem(ZONE_A_KEY.as_bytes()).unwrap();
    RPCContext {
        token: Some(token.generate_jwt(None, &key).unwrap()),
        from_ip: Some("127.0.0.1".parse().unwrap()),
        ..Default::default()
    }
}

#[tokio::test]
async fn tags_are_written_only_by_their_owner() {
    let zone = "z.example";
    let alice = user("alice", zone);
    let bob = user("bob", zone);
    let (c, _tmp) = open("tags", zone, &[alice.clone(), bob.clone()]).await;
    c.set_token_verifier(Arc::new(Verifier));
    let as_alice = token_context(&alice.to_string(), TokenPrincipalKind::User, "control-panel");
    let as_bob = token_context(&bob.to_string(), TokenPrincipalKind::User, "control-panel");
    let as_homestation = token_context("ood1", TokenPrincipalKind::System, "homestation");
    let as_workspace = token_context("ood1", TokenPrincipalKind::System, "aiworkspace");
    let tags = |ctx: &RPCContext, add: &str| {
        c.contact_ext_rpc(
            "contact.update_tags",
            json!({"did":bob,"add":[add],"contact_mgr_owner":alice}),
            ctx.clone(),
        )
    };

    let family: Contact = serde_json::from_value(tags(&as_alice, "家人").await.unwrap()).unwrap();
    assert_eq!(family.tags, vec!["家人".to_string()]);
    assert_eq!(family.source, buckyos_api::ContactSource::ManualCreate);
    for reserved in ["zone_user", "homestation.following"] {
        assert!(tags(&as_alice, reserved).await.is_err(), "{reserved}");
    }
    assert!(tags(&as_bob, "家人").await.is_err());
    let following: Contact =
        serde_json::from_value(tags(&as_homestation, "homestation.following").await.unwrap()).unwrap();
    assert!(following.tags.iter().any(|t| t == "homestation.following"));
    assert!(tags(&as_homestation, "家人").await.is_err());
    assert!(tags(&as_homestation, "aiworkspace.pinned").await.is_err());
    assert!(tags(&as_workspace, "aiworkspace.pinned").await.is_err());
    let removed: Contact = serde_json::from_value(
        c.contact_ext_rpc(
            "contact.update_tags",
            json!({"did":bob,"remove":["homestation.following"],"contact_mgr_owner":alice}),
            as_homestation.clone(),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(removed.tags, vec!["家人".to_string()]);
}

/// Two msg-center instances stand for two Zones: a request and its answer
/// travel as signed messages without any configured route or Bearer token.
#[tokio::test]
async fn friend_request_and_chat_cross_two_zones() {
    let alice = user("alice", "a.example");
    let bob = user("bob", "b.example");
    let device_a = DID::new("dev", ZONE_A_DEVICE);
    let device_b = DID::new("dev", ZONE_B_DEVICE);
    let (a, _ta) = open("zone-a", "a.example", &[alice.clone()]).await;
    let (b, _tb) = open("zone-b", "b.example", &[bob.clone()]).await;
    let directory = StaticDirectory::with(vec![
        (alice.clone(), LookupKind::User, "Alice", "a.example"),
        (bob.clone(), LookupKind::User, "Bob", "b.example"),
        (device_a.clone(), LookupKind::Device, "ood-a", "a.example"),
        (device_b.clone(), LookupKind::Device, "ood-b", "b.example"),
        (user("ghost", "b.example"), LookupKind::User, "Ghost", "b.example"),
    ]);
    for (center, key, device) in [(&a, ZONE_A_KEY, &device_a), (&b, ZONE_B_KEY, &device_b)] {
        center.directory.set(directory.clone());
        center.zone_signer.set(ZoneSigner {
            key: EncodingKey::from_ed_pem(key.as_bytes()).unwrap(),
            kid: format!("{}#main_key", device.to_string()),
        });
    }
    let serve = |center: MessageCenter| {
        http_fixture(move |req| {
            let center = center.clone();
            async move { crate::cyfs_dispatch::serve(&center, req).await }
        })
    };
    let (url_a, _sa) = serve(a.clone()).await;
    let (url_b, _sb) = serve(b.clone()).await;
    a.cyfs_dispatch
        .write()
        .unwrap()
        .zone_origins
        .insert("b.example".into(), url_b);
    b.cyfs_dispatch
        .write()
        .unwrap()
        .zone_origins
        .insert("a.example".into(), url_a);

    let sent = send_request(&a, &alice, &bob).await.request.unwrap();
    assert_eq!(
        sent.delivery.as_ref().map(|d| d.state),
        Some(buckyos_api::FriendDeliveryState::Pending)
    );
    assert_eq!(pump(&a).await, 1);
    assert_eq!(
        requests(&a, &alice).await[0].delivery.as_ref().map(|d| d.state),
        Some(buckyos_api::FriendDeliveryState::Delivered)
    );
    assert_eq!(friend_state(&b, &bob, &alice).await, FriendState::IncomingPending);
    assert_eq!(dm_records(&b, &bob, &alice, MailboxKind::RequestBox).await, 1);
    let incoming = requests(&b, &bob).await;
    assert_eq!(incoming[0].peer_name.as_deref(), None);

    respond(&b, &bob, &incoming[0].request_id, "accept").await;
    assert_eq!(pump(&b).await, 1);
    assert_eq!(friend_state(&a, &alice, &bob).await, FriendState::Friend);
    assert_eq!(friend_state(&b, &bob, &alice).await, FriendState::Friend);

    // Replaying the delivered request changes nothing on the receiver.
    let jwt = a
        .msg_box_db
        .get_msg_jwt(&ndn_lib::ObjId::new(sent.msg_id.as_deref().unwrap()).unwrap())
        .await
        .unwrap()
        .unwrap();
    let route = crate::cyfs_dispatch::auto_route(&a, &bob).await.unwrap();
    let msg = a
        .load_message(&ndn_lib::ObjId::new(sent.msg_id.as_deref().unwrap()).unwrap())
        .await
        .unwrap();
    let replay = crate::cyfs_dispatch::send(&route, &msg, Some(&jwt), &ndn_lib::NamedObject::gen_obj_id(&msg).0)
        .await
        .unwrap();
    assert!(replay.ok);
    assert_eq!(requests(&b, &bob).await.len(), 1);

    // Ordinary chat now crosses too and lands in the INBOX (friends).
    let chat = MsgObject {
        from: bob.clone(),
        to: vec![alice.clone()],
        kind: MsgObjKind::Chat,
        created_at_ms: MessageCenter::now_ms(),
        content: MsgContent {
            content: "hello across zones".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let post = b.post_send_internal(chat, None).await.unwrap();
    assert!(post.ok, "{:?}", post.reason);
    assert_eq!(pump(&b).await, 1);
    assert_eq!(dm_records(&a, &alice, &bob, MailboxKind::Inbox).await, 1);

    // Unsigned objects without a Bearer token are refused.
    let unsigned = MsgObject {
        from: bob.clone(),
        to: vec![alice.clone()],
        kind: MsgObjKind::Chat,
        created_at_ms: MessageCenter::now_ms(),
        ..Default::default()
    };
    let report = crate::cyfs_dispatch::send(
        &crate::cyfs_dispatch::auto_route(&b, &alice).await.unwrap(),
        &unsigned,
        None,
        &ndn_lib::NamedObject::gen_obj_id(&unsigned).0,
    )
    .await
    .unwrap();
    assert!(!report.ok);

    // Zone B serves the agreed path only for the users it hosts.
    let ghost = user("ghost", "b.example");
    let to_ghost = MsgObject {
        from: alice.clone(),
        to: vec![ghost.clone()],
        kind: MsgObjKind::Chat,
        created_at_ms: MessageCenter::now_ms(),
        ..Default::default()
    };
    let signed = to_ghost
        .to_jwt(
            &EncodingKey::from_ed_pem(ZONE_A_KEY.as_bytes()).unwrap(),
            &format!("{}#main_key", device_a.to_string()),
        )
        .unwrap();
    let report = crate::cyfs_dispatch::send(
        &crate::cyfs_dispatch::auto_route(&a, &ghost).await.unwrap(),
        &to_ghost,
        Some(&signed),
        &ndn_lib::NamedObject::gen_obj_id(&to_ghost).0,
    )
    .await
    .unwrap();
    assert!(!report.ok);

    // A device of another Zone cannot sign for Alice.
    let forged = MsgObject {
        from: alice.clone(),
        to: vec![bob.clone()],
        kind: MsgObjKind::Chat,
        created_at_ms: MessageCenter::now_ms(),
        ..Default::default()
    };
    let wrong = forged
        .to_jwt(
            &EncodingKey::from_ed_pem(ZONE_B_KEY.as_bytes()).unwrap(),
            &format!("{}#main_key", device_b.to_string()),
        )
        .unwrap();
    let report = crate::cyfs_dispatch::send(
        &crate::cyfs_dispatch::auto_route(&a, &bob).await.unwrap(),
        &forged,
        Some(&wrong),
        &ndn_lib::NamedObject::gen_obj_id(&forged).0,
    )
    .await
    .unwrap();
    assert!(!report.ok);
}

#[tokio::test]
async fn friend_state_cannot_be_bypassed() {
    let zone = "z.example";
    let alice = user("alice", zone);
    let bob = user("bob", zone);
    let (c, _tmp) = open("bypass", zone, &[alice.clone(), bob.clone()]).await;

    // Friend messages are built by the contact RPCs only.
    let forged = MsgObject {
        from: alice.clone(),
        to: vec![bob.clone()],
        kind: MsgObjKind::Operation,
        created_at_ms: MessageCenter::now_ms(),
        content: MsgContent {
            machine: Some(ndn_lib::MachineContent {
                intent: Some(buckyos_api::FRIEND_INTENT.into()),
                data: serde_json::from_value(json!({"action":"request","request_id":"forged"})).unwrap(),
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let error = c
        .handle_post_send(forged, None, RPCContext::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("friend-messages-use-contact-rpc"));

    // Imported groupings keep only user tags.
    c.handle_import_contacts(
        vec![buckyos_api::ImportContactEntry {
            name: "Imported".into(),
            avatar: None,
            note: None,
            bindings: vec![buckyos_api::AccountBinding {
                platform: "email".into(),
                account_id: "imported@example.com".into(),
                display_id: "imported@example.com".into(),
                tunnel_instance_id: "email-main".into(),
                account_type: String::new(),
                endpoint_did: None,
                last_active_at: 1,
                meta: HashMap::new(),
            }],
            tags: vec!["家人".into(), "zone_user".into(), "homestation.following".into()],
        }],
        Some(false),
        Some(alice.clone()),
        RPCContext::default(),
    )
    .await
    .unwrap();
    let imported = c
        .contact_mgr
        .list_contacts(ContactQuery { keyword: Some("Imported".into()), ..Default::default() }, Some(alice.clone()))
        .await
        .unwrap();
    assert_eq!(imported[0].tags, vec!["家人".to_string()]);

    // Blocking through update_contact settles the pending request too.
    let pending = send_request(&c, &bob, &alice).await.request.unwrap();
    c.handle_update_contact(
        bob.clone(),
        buckyos_api::ContactPatch {
            access_level: Some(AccessGroupLevel::Block),
            ..Default::default()
        },
        Some(alice.clone()),
        RPCContext::default(),
    )
    .await
    .unwrap();
    let mine = requests(&c, &alice).await;
    assert!(mine
        .iter()
        .any(|r| r.request_id == pending.request_id && r.state == buckyos_api::FriendRequestState::Ignored));
    assert_eq!(friend_state(&c, &alice, &bob).await, FriendState::None);
}
