use crate::group_types::{session_key, GroupActor, MemberStatus};
use crate::message_hub::MessageHubExecutor;
use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use crate::msg_tunnel::DeliveryExecutor;
use crate::network_directory::test_support::StaticDirectory;
use crate::network_directory::ZoneSigner;
use crate::owner_session::{SessionTokenVerifier, ZoneAgentInfo};
use crate::test_group_service::{http_body, http_fixture};
use buckyos_api::{
    bind_token_principal_kind, bind_token_target, AuthTarget, DeliveryRecordWithObject,
    DeliveryState, LookupKind, MailboxAddress, MailboxKind, MsgCenterHandler, SystemServiceId,
    TokenPrincipalKind, TokenUse,
};
use http::{Method, Response, StatusCode};
use jsonwebtoken::{DecodingKey, EncodingKey};
use kRPC::{RPCContext, RPCErrors, RPCSessionToken, RPCSessionTokenType};
use name_lib::DID;
use ndn_lib::{MsgObjKind, MsgObject, NamedObject};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::{tempdir, TempDir};

const HOST_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const HOST_DEVICE: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
const LOCAL_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIF5zplRSY5MsYbBMdbzItH90daUlrx/OflPm8Kp9m2j1\n-----END PRIVATE KEY-----";
const LOCAL_DEVICE: &str = "mL_tm5diVGGkpt_6WmCZQxUsee4rHacyz7mx7TkKb-0";

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
fn group() -> DID {
    DID::new("web", "support.a.example")
}
fn hub() -> DID {
    DID::new("web", "hub.local.example")
}

struct Verifier {
    public_key: &'static str,
}

#[async_trait::async_trait]
impl SessionTokenVerifier for Verifier {
    async fn authorize(&self, _: &str, _: &str, _: &str) -> Result<(), RPCErrors> {
        Ok(())
    }
    async fn verify(&self, raw: &str) -> Result<RPCSessionToken, RPCErrors> {
        let mut token = RPCSessionToken::from_string(raw)?;
        token.verify_by_key(&DecodingKey::from_ed_components(self.public_key).unwrap())?;
        Ok(token)
    }
    async fn resolve_user_did(&self, id: &str) -> Result<DID, RPCErrors> {
        DID::from_str(id).map_err(|error| RPCErrors::ParseRequestError(error.to_string()))
    }
    async fn zone_agent(&self, _: &DID) -> Result<Option<ZoneAgentInfo>, RPCErrors> {
        Ok(None)
    }
    async fn sso_settings(&self) -> Result<buckyos_api::SsoSettings, RPCErrors> {
        Ok(Default::default())
    }
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

async fn open(path: &str, host: bool, directory: Arc<StaticDirectory>) -> MessageCenter {
    let db = MsgBoxDbMgr::open_default_sqlite(path).await.unwrap();
    let center = MessageCenter::open_with_db(db).await.unwrap();
    let (zone, key, device, users) = if host {
        ("a.example", HOST_KEY, HOST_DEVICE, vec![alice(), helper()])
    } else {
        ("b.example", LOCAL_KEY, LOCAL_DEVICE, vec![bob(), carol()])
    };
    center.cyfs_dispatch.write().unwrap().target_zone = Some(zone.into());
    center.register_local_recipients(users);
    center.set_message_hub_did(hub());
    center.set_token_verifier(Arc::new(Verifier { public_key: device }));
    center.directory.set(directory);
    center.zone_signer.set(ZoneSigner {
        key: EncodingKey::from_ed_pem(key.as_bytes()).unwrap(),
        kid: format!("{}#main_key", DID::new("dev", device).to_string()),
    });
    center
}

#[derive(Default)]
struct Faults {
    lose_join_reply: AtomicBool,
    fail_changes: AtomicBool,
}

struct TwoZones {
    host: MessageCenter,
    local: MessageCenter,
    local_path: String,
    host_url: String,
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
        (
            DID::new("dev", HOST_DEVICE),
            LookupKind::Device,
            "host-ood",
            "a.example",
        ),
        (
            DID::new("dev", LOCAL_DEVICE),
            LookupKind::Device,
            "local-ood",
            "b.example",
        ),
    ]);
    let host = open(&host_path, true, directory.clone()).await;
    let local = open(&local_path, false, directory.clone()).await;
    let faults = Arc::new(Faults::default());
    let serve = |center: MessageCenter, faults: Arc<Faults>| {
        http_fixture(move |mut request| {
            let center = center.clone();
            let faults = faults.clone();
            async move {
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
    };
    let (host_url, host_server) = serve(host.clone(), faults.clone()).await;
    let (local_url, local_server) = serve(local.clone(), Arc::new(Faults::default())).await;
    host.cyfs_dispatch
        .write()
        .unwrap()
        .zone_origins
        .insert("b.example".into(), local_url);
    local
        .cyfs_dispatch
        .write()
        .unwrap()
        .zone_origins
        .insert("a.example".into(), host_url.clone());
    TwoZones {
        host,
        local,
        local_path,
        host_url,
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
    params["group_did"] = json!(group());
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
        .accept_group_message(
            &GroupActor {
                did: alice(),
                client: Some("messagehub".into()),
                remote: false,
            },
            msg.clone(),
            None,
        )
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
    zones
        .local
        .cyfs_dispatch
        .write()
        .unwrap()
        .zone_origins
        .insert("a.example".into(), zones.host_url.clone());
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
    zones
        .local
        .cyfs_dispatch
        .write()
        .unwrap()
        .zone_origins
        .insert("a.example".into(), zones.host_url.clone());
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
    zones
        .local
        .cyfs_dispatch
        .write()
        .unwrap()
        .zone_origins
        .insert("a.example".into(), zones.host_url.clone());
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
