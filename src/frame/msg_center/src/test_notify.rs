use crate::msg_box_db::MsgBoxDbMgr;
use crate::msg_center::MessageCenter;
use crate::notify::NotifyTasks;
use crate::owner_session::{serve_with_request_identity, SessionTokenVerifier, ZoneAgentInfo};
use async_trait::async_trait;
use buckyos_api::*;
use jsonwebtoken::{DecodingKey, EncodingKey};
use kRPC::{RPCContext, RPCErrors, RPCSessionToken, RPCSessionTokenType};
use name_lib::DID;
use ndn_lib::{MsgContent, MsgContentFormat, MsgObjKind, MsgObject, NamedObject};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::{tempdir, TempDir};

const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#;
const TEST_PUBLIC_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

static SEQ: AtomicU64 = AtomicU64::new(1_000_000);

fn next_ms() -> u64 {
    SEQ.fetch_add(10, Ordering::SeqCst)
}

fn alice() -> DID {
    DID::new("bns", "alice")
}

fn bob() -> DID {
    DID::new("bns", "bob")
}

fn jarvis() -> DID {
    DID::new("web", "jarvis.test.buckyos.io")
}

fn zone() -> DID {
    DID::new("web", "test.buckyos.io")
}

struct Verifier {
    key: DecodingKey,
}

#[async_trait]
impl SessionTokenVerifier for Verifier {
    async fn authorize(&self, _token: &str, _resource: &str, _action: &str) -> Result<(), RPCErrors> {
        Ok(())
    }

    async fn verify(&self, token: &str) -> Result<RPCSessionToken, RPCErrors> {
        let mut parsed = RPCSessionToken::from_string(token)?;
        parsed.verify_by_key(&self.key)?;
        Ok(parsed)
    }

    async fn resolve_user_did(&self, user_id: &str) -> Result<DID, RPCErrors> {
        match user_id {
            "alice" => Ok(alice()),
            "bob" => Ok(bob()),
            _ => Err(RPCErrors::KeyNotExist(format!("users/{user_id}/profile"))),
        }
    }

    async fn zone_agent(&self, did: &DID) -> Result<Option<ZoneAgentInfo>, RPCErrors> {
        Ok((did == &jarvis()).then(|| ZoneAgentInfo {
            owner: alice(),
            allow_group: false,
        }))
    }
}

fn token(sub: &str, kind: TokenPrincipalKind, target: AuthTarget) -> RPCContext {
    let now = buckyos_kit::buckyos_get_unix_timestamp();
    token_expiring_at(sub, kind, target, now + 3600)
}

fn token_expiring_at(
    sub: &str,
    kind: TokenPrincipalKind,
    target: AuthTarget,
    exp: u64,
) -> RPCContext {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(exp),
        iss: Some(VERIFY_HUB_UNIQUE_ID.to_string()),
        jti: None,
        sub: Some(sub.to_string()),
        appid: None,
        sudo: false,
        extra: HashMap::new(),
    };
    bind_token_principal_kind(&mut token, kind);
    bind_token_target(&mut token, &target, TokenUse::Session).unwrap();
    RPCContext {
        token: Some(
            token
                .generate_jwt(None, &EncodingKey::from_ed_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap())
                .unwrap(),
        ),
        ..Default::default()
    }
}

fn app_ctx(app: &str, user: &str) -> RPCContext {
    token(
        user,
        TokenPrincipalKind::App,
        AuthTarget::app(format!("{app}@{user}").parse().unwrap()),
    )
}

fn agent_ctx() -> RPCContext {
    app_ctx("jarvis.test.buckyos.io", "alice")
}

fn user_ctx(user: &str) -> RPCContext {
    token(
        user,
        TokenPrincipalKind::User,
        AuthTarget::system(SystemServiceId::parse("control-panel").unwrap()),
    )
}

fn system_ctx() -> RPCContext {
    token(
        "ood1",
        TokenPrincipalKind::System,
        AuthTarget::system(SystemServiceId::parse("aiworkspace").unwrap()),
    )
}

fn in_process() -> RPCContext {
    RPCContext::default()
}

/// HumanSet tasks in memory.
#[derive(Default)]
struct FakeTasks {
    tasks: Mutex<HashMap<String, Task>>,
    grants: Mutex<Vec<(String, TaskAclGrantSpec)>>,
}

impl FakeTasks {
    fn answer(&self, task_id: &str, answer: NotifyAnswer) {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks.get_mut(task_id).unwrap();
        task.result = Some(answer.to_task_result("alice", 1));
        task.outcome = Some(TaskOutcome::Succeeded);
        task.phase = TaskPhase::Terminal;
        task.completed_at = Some(5);
        task.completed_by = Some(ActorRef::new("alice", "system:control-panel"));
        task.revision += 1;
    }

    fn count(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }
}

#[async_trait]
impl NotifyTasks for FakeTasks {
    async fn create(&self, req: CreateTaskReq) -> Result<Task, RPCErrors> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(t) = tasks.values().find(|t| t.idempotency_key == req.idempotency_key) {
            assert_eq!(t.input, req.input, "an idempotent replay carries the same input");
            return Ok(t.clone());
        }
        let task_id = format!("t-{}", tasks.len() + 1);
        let CreateTaskExecutor::HumanSet { assignees } = req.executor else {
            panic!("notify tasks are human tasks");
        };
        let task = Task {
            task_id: task_id.clone(),
            name: req.name,
            parent_id: None,
            root_id: task_id.clone(),
            child_control_policy: ChildControlPolicy::default(),
            schema_id: req.schema_id,
            schema_version: 1,
            input: req.input,
            input_digest: String::new(),
            creator: ActorRef::new("ood1", "system:msg-center"),
            storage_domain: req.storage_domain.unwrap_or_default(),
            idempotency_key: req.idempotency_key,
            origin_ref: None,
            retry_of: None,
            supersedes: None,
            executor: TaskExecutor::HumanSet,
            runner_epoch: 0,
            assignees: Some(assignees),
            phase: TaskPhase::Waiting,
            wait_reason: Some(TaskWaitReason::new(TaskWaitReasonKind::HumanInput)),
            pending_control: None,
            control_profile: TaskControlProfile::baseline(0),
            progress: None,
            message: req.message,
            outcome: None,
            result: None,
            error: None,
            completed_by: None,
            policy_preset: TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1.into(),
            permission_boundary: false,
            revision: 1,
            data_scope: None,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            archived_at: None,
        };
        tasks.insert(task_id, task.clone());
        Ok(task)
    }

    async fn grant(&self, task_id: &str, grants: Vec<TaskAclGrantSpec>) -> Result<(), RPCErrors> {
        let mut all = self.grants.lock().unwrap();
        all.extend(grants.into_iter().map(|g| (task_id.to_string(), g)));
        Ok(())
    }

    async fn get(&self, task_id: &str) -> Result<Task, RPCErrors> {
        self.tasks
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .ok_or_else(|| task_mgr_error(TASK_ERR_NOT_FOUND, task_id))
    }

    async fn cancel(&self, task_id: &str, _request_id: &str) -> Result<(), RPCErrors> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks.get_mut(task_id).unwrap();
        if !task.phase.is_terminal() {
            task.phase = TaskPhase::Terminal;
            task.outcome = Some(TaskOutcome::Canceled);
            task.completed_at = Some(9);
            task.revision += 1;
        }
        Ok(())
    }
}

struct Env {
    center: MessageCenter,
    tasks: Arc<FakeTasks>,
    _tmp: TempDir,
}

async fn env() -> Env {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("msg-center.db");
    let conn = format!("sqlite:///{}?mode=rwc", path.to_string_lossy().replace('\\', "/"));
    let db = MsgBoxDbMgr::open_default_sqlite(&conn).await.unwrap();
    let center = MessageCenter::open_with_db(db).await.unwrap();
    center.set_token_verifier(Arc::new(Verifier {
        key: DecodingKey::from_ed_components(TEST_PUBLIC_X).unwrap(),
    }));
    let tasks = Arc::new(FakeTasks::default());
    center.set_notify_tasks(tasks.clone());
    center.set_notify_identity(zone(), Some("test.buckyos.io".into()));
    center.set_message_hub_did(DID::new("web", "hub.test.buckyos.io"));
    center.register_local_recipients([alice(), bob(), jarvis()]);
    Env {
        center,
        tasks,
        _tmp: tmp,
    }
}

async fn call<T: serde::de::DeserializeOwned>(
    center: &MessageCenter,
    method: &str,
    params: Value,
    ctx: RPCContext,
) -> Result<T, RPCErrors> {
    let value = center.handle_notify(method, params, ctx).await?;
    Ok(serde_json::from_value(value).unwrap())
}

async fn send(center: &MessageCenter, req: NotifySendReq, ctx: RPCContext) -> Result<NotifyView, RPCErrors> {
    call(center, METHOD_NOTIFY_SEND, serde_json::to_value(req).unwrap(), ctx).await
}

async fn get(center: &MessageCenter, id: &str, ctx: RPCContext) -> Result<NotifyView, RPCErrors> {
    call(center, METHOD_NOTIFY_GET, json!({"notify_id": id}), ctx).await
}

async fn record(center: &MessageCenter, owner: &DID, record_id: &str) -> MailboxRecord {
    center
        .msg_box_db
        .get_record(owner, record_id)
        .await
        .unwrap()
        .unwrap()
}

async fn session_records(center: &MessageCenter, owner: &DID, session: &str) -> Vec<MailboxRecord> {
    center
        .msg_box_db
        .list_visible_session_records(owner, session, 100, None, None, true, None)
        .await
        .unwrap()
}

/// The user writes to the Agent in conversation `to_session` (`from` is
/// the user or a tunnel endpoint speaking for the user).
async fn user_says(center: &MessageCenter, from: &DID, to_session: &str, at: u64) {
    center
        .handle_grant_temporary_access(vec![alice()], "chat".into(), 3600, Some(jarvis()), in_process())
        .await
        .unwrap();
    let msg = MsgObject {
        from: from.clone(),
        to: vec![jarvis()],
        to_session: Some(to_session.to_string()),
        kind: MsgObjKind::Chat,
        created_at_ms: at,
        content: MsgContent {
            format: Some(MsgContentFormat::TextPlain),
            content: format!("hi in {to_session}"),
            ..Default::default()
        },
        ..Default::default()
    };
    let ingress = IngressContext {
        context_id: Some("chat".into()),
        extra: (from != &alice()).then(|| json!({"principal_did": alice().to_string()})),
        ..Default::default()
    };
    let key = msg.gen_obj_id().0.to_string();
    let r = center
        .handle_dispatch(msg, Some(ingress), Some(key), in_process())
        .await
        .unwrap();
    assert_eq!(r.delivered_recipients, vec![jarvis()]);
}

fn work_question(key: &str, level: NotifyLevel) -> NotifySendReq {
    let mut req = NotifySendReq::input(
        "alice",
        "Which database?",
        "The migration needs a target.",
        vec![
            HumanPromptOption::new("pg", "Postgres"),
            HumanPromptOption::new("sqlite", "SQLite"),
        ],
        true,
        key,
    );
    req.level = Some(level);
    req.source = Some(NotifySource {
        session_id: Some("work-1".into()),
        turn: Some(3),
        call_id: Some("call-9".into()),
        operation: Some("migrate the notes app".into()),
        conversation: Some(NotifyConversation {
            to: alice().to_string(),
            to_session: Some("chat-a".into()),
        }),
        ..Default::default()
    });
    req
}

#[tokio::test]
async fn an_app_notice_is_one_record_in_the_app_session_and_replays_once() {
    let env = env().await;
    let c = &env.center;
    let mut req = NotifySendReq::notice("alice", "Backup done", "Last night's backup finished.", "backup:1");
    req.source = Some(NotifySource {
        app_id: Some("forged".into()),
        agent_did: Some(jarvis().to_string()),
        link: Some("https://notes.test.buckyos.io/backup".into()),
        ..Default::default()
    });
    let view = send(c, req.clone(), app_ctx("notes", "alice")).await.unwrap();
    assert!(view.notify_id.starts_with("nt-"));
    assert_eq!(view.url, format!("https://test.buckyos.io/messagehub?notify={}", view.notify_id));
    assert_eq!(view.category, NotifyCategory::App);
    assert_eq!(view.kind, NotifyKind::Notice);
    assert_eq!(view.status, NotifyStatus::Open);
    assert_eq!(view.source.app_id.as_deref(), Some("notes"));
    assert_eq!(view.source.agent_did, None, "the source is the caller, not what it claims");
    assert!(view.task_id.is_none());
    assert!(view.deliveries.is_empty());
    assert_eq!(env.tasks.count(), 0, "a notice has no task");

    let home = view.home.clone().unwrap();
    assert_eq!(home.session_id, "sysnotify:app");
    assert_eq!(home.from, zone().to_string());
    let rec = record(c, &alice(), &home.record_id).await;
    assert_eq!(rec.box_kind, MailboxKind::Inbox, "never the request box");
    assert_eq!(rec.state, RecipientState::Unread);
    assert_eq!(rec.msg_kind, MsgObjKind::Notify);
    let msg = c
        .get_message_internal(rec.msg_id.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(msg.content.content.ends_with(&format!("处理：{}", view.url)));
    let data = notify_message_data(&msg).unwrap();
    assert!(data.home);
    assert_eq!(data.notify_id, view.notify_id);

    let session = c
        .msg_box_db
        .get_owner_session(&alice(), "sysnotify:app")
        .await
        .unwrap()
        .unwrap();
    assert!(session.registered);
    assert_eq!(session.peer_did, Some(zone()));
    assert_eq!(session.title.as_deref(), Some("应用通知"));
    let sessions = c
        .list_sessions_scoped(alice(), None, None, None, None, None, Some(SessionListOrder::Activity))
        .await
        .unwrap();
    let app = sessions.items.iter().find(|s| s.session_id == "sysnotify:app").unwrap();
    assert_eq!(app.unread_count, 1);
    assert!(app.last_activity_ms > 0, "a notification is session activity");

    let again = send(c, req, app_ctx("notes", "alice")).await.unwrap();
    assert_eq!(again.notify_id, view.notify_id);
    assert_eq!(session_records(c, &alice(), "sysnotify:app").await.len(), 1);
}

#[tokio::test]
async fn categories_and_permissions_follow_the_caller() {
    let env = env().await;
    let c = &env.center;
    let mut todo = NotifySendReq::notice("alice", "Review", "", "t1");
    todo.category = Some(NotifyCategory::Todo);
    assert!(matches!(
        send(c, todo.clone(), app_ctx("notes", "alice")).await,
        Err(RPCErrors::NoPermission(_))
    ));
    let v = send(c, todo, system_ctx()).await.unwrap();
    assert_eq!(v.category, NotifyCategory::Todo);
    assert_eq!(v.home.unwrap().session_id, "sysnotify:todo");

    let a = send(c, NotifySendReq::notice("alice", "From jarvis", "", "a1"), agent_ctx())
        .await
        .unwrap();
    assert_eq!(a.category, NotifyCategory::Agent);
    assert_eq!(a.source.agent_did, Some(jarvis().to_string()));
    assert_eq!(a.home.unwrap().session_id, "sysnotify:agent");

    assert!(matches!(
        send(c, NotifySendReq::notice("bob", "Hi bob", "", "b1"), app_ctx("notes", "alice")).await,
        Err(RPCErrors::NoPermission(_))
    ));
    let to_bob = send(c, NotifySendReq::notice("bob", "Hi bob", "", "b1"), system_ctx())
        .await
        .unwrap();
    assert!(get(c, &to_bob.notify_id, user_ctx("alice")).await.is_err());
    assert!(get(c, &to_bob.notify_id, user_ctx("bob")).await.is_ok());
    assert!(get(c, &a.notify_id, user_ctx("alice")).await.is_ok());
    let alice_list: NotifyListResult =
        call(c, METHOD_NOTIFY_LIST, json!({}), user_ctx("alice")).await.unwrap();
    assert_eq!(alice_list.items.len(), 2);
    assert!(alice_list.items.iter().all(|v| v.to_user == "alice"));
    let bob_list: NotifyListResult = call(c, METHOD_NOTIFY_LIST, json!({}), user_ctx("bob")).await.unwrap();
    assert_eq!(bob_list.items.len(), 1);
    let by_agent: NotifyListResult = call(
        c,
        METHOD_NOTIFY_LIST,
        json!({"agent_did": jarvis().to_string()}),
        agent_ctx(),
    )
    .await
    .unwrap();
    assert_eq!(by_agent.items.len(), 1);
    assert!(call::<NotifyListResult>(c, METHOD_NOTIFY_LIST, json!({"to_user": "bob"}), user_ctx("alice"))
        .await
        .is_err());
    // Only the sender cancels.
    assert!(matches!(
        call::<NotifyView>(c, METHOD_NOTIFY_CANCEL, json!({"notify_id": a.notify_id}), app_ctx("notes", "alice")).await,
        Err(RPCErrors::NoPermission(_))
    ));
}

#[tokio::test]
async fn levels_decide_copies_and_where_unread_lands() {
    let env = env().await;
    let c = &env.center;
    // No conversation yet: only the home record, unread.
    let lonely = send(c, work_question("q0", NotifyLevel::Normal), agent_ctx()).await.unwrap();
    assert!(lonely.deliveries.is_empty());
    let home = lonely.home.unwrap();
    assert_eq!(record(c, &alice(), &home.record_id).await.state, RecipientState::Unread);

    user_says(c, &alice(), "chat-a", next_ms()).await;
    user_says(c, &alice(), "chat-b", next_ms()).await;
    let tg = DID::new("msgtunnel", "777.user.tg-main");
    c.register_tunnel("tg-main".into(), DID::new("bns", "tg-box"), "telegram".into())
        .unwrap();
    user_says(c, &tg, "tg:jarvis-bot:777", next_ms()).await;
    let conversations = c.agent_conversations(&jarvis(), &alice()).await.unwrap();
    assert_eq!(conversations.len(), 3);
    assert_eq!(conversations[0].platform, "telegram");
    assert_eq!(conversations[0].to, tg.to_string());

    let quiet = send(c, work_question("q1", NotifyLevel::Quiet), agent_ctx()).await.unwrap();
    assert!(quiet.deliveries.is_empty());
    assert_eq!(
        record(c, &alice(), &quiet.home.unwrap().record_id).await.state,
        RecipientState::Read
    );

    let normal = send(c, work_question("q2", NotifyLevel::Normal), agent_ctx()).await.unwrap();
    assert_eq!(normal.deliveries.len(), 1);
    assert_eq!(normal.deliveries[0].session_id, "chat-a", "the source conversation");
    assert!(normal.deliveries[0].ok);
    assert_eq!(
        record(c, &alice(), &normal.home.unwrap().record_id).await.state,
        RecipientState::Read,
        "unread lands on the copy"
    );

    let timely = send(c, work_question("q3", NotifyLevel::Timely), agent_ctx()).await.unwrap();
    let sessions: Vec<&str> = timely.deliveries.iter().map(|d| d.session_id.as_str()).collect();
    assert_eq!(sessions, vec!["chat-a", "tg:jarvis-bot:777"]);
    assert_eq!(timely.deliveries[1].platform, "telegram");

    let urgent = send(c, work_question("q4", NotifyLevel::Urgent), agent_ctx()).await.unwrap();
    assert_eq!(urgent.deliveries.len(), 3);
    assert_eq!(
        record(c, &alice(), &urgent.home.unwrap().record_id).await.state,
        RecipientState::Unread
    );

    // The Telegram copy is plain text with the link, through the tunnel.
    let taken = c
        .handle_get_next_delivery(DID::new("bns", "tg-box"), Some(true), Some(false), in_process())
        .await
        .unwrap()
        .unwrap();
    let msg = c
        .get_message_internal(taken.record.envelope.msg_id.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(msg.content.content.contains("Postgres / SQLite"));
    assert!(msg.content.content.contains("/messagehub?notify=nt-"));
    assert_eq!(msg.from, jarvis());

    // Every copy leaves an observation record in the Agent's own INBOX.
    let obs = session_records(c, &jarvis(), "chat-a").await;
    let delivered: Vec<NotifyDeliveredData> = {
        let mut out = vec![];
        for r in obs.iter().filter(|r| r.msg_kind == MsgObjKind::Event) {
            assert_eq!(r.from, jarvis());
            assert_eq!(r.state, RecipientState::Unread);
            let m = c.get_message_internal(r.msg_id.clone()).await.unwrap().unwrap();
            assert_eq!(m.to_session.as_deref(), Some("chat-a"));
            out.push(machine_data(&m, NOTIFY_DELIVERED_INTENT).unwrap());
        }
        out
    };
    assert_eq!(delivered.len(), 3, "normal, timely and urgent reached chat-a");
    assert_eq!(delivered[0].reply_route.to, alice().to_string());
    assert_eq!(delivered[0].reply_route.to_session.as_deref(), Some("chat-a"));
    assert_eq!(delivered[0].options.len(), 2);
    let tg_obs = session_records(c, &jarvis(), "tg:jarvis-bot:777").await;
    assert_eq!(tg_obs.iter().filter(|r| r.msg_kind == MsgObjKind::Event).count(), 2);
}

#[tokio::test]
async fn an_input_has_a_task_and_its_status_comes_from_it() {
    let env = env().await;
    let c = &env.center;
    let view = send(c, work_question("ask-1", NotifyLevel::Timely), app_ctx("notes", "alice"))
        .await
        .unwrap();
    let task_id = view.task_id.clone().unwrap();
    assert_eq!(view.status, NotifyStatus::Open);
    let task = env.tasks.get(&task_id).await.unwrap();
    assert_eq!(task.schema_id, HUMAN_INPUT_TASK_SCHEMA_ID);
    assert_eq!(task.assignees, Some(vec!["alice".to_string()]));
    assert_eq!(task.idempotency_key, format!("notify:{}", view.notify_id));
    let input: NotifyTaskInput = serde_json::from_value(task.input.clone()).unwrap();
    assert_eq!(input.notify.notify_id, view.notify_id);
    let grants = env.tasks.grants.lock().unwrap().clone();
    assert!(grants.iter().any(|(_, g)| g.subject
        == TaskGrantSubject::User { user_id: "alice".into() }
        && g.actions == vec![TaskAction::ReadResult]));
    assert!(grants.iter().any(|(_, g)| g.subject
        == TaskGrantSubject::Principal {
            user_id: "alice".into(),
            app_id: "app:notes@alice".into()
        }
        && g.actions.contains(&TaskAction::Control)));
    let msg = c
        .get_message_internal(ObjIdOf(&view.home.as_ref().unwrap().msg_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(notify_message_data(&msg).unwrap().task_id, Some(task_id.clone()));

    env.tasks.answer(&task_id, NotifyAnswer::option("pg"));
    let done = get(c, &view.notify_id, app_ctx("notes", "alice")).await.unwrap();
    assert_eq!(done.status, NotifyStatus::Answered);
    assert_eq!(done.result.as_ref().unwrap().option(), Some("pg"));

    let second = send(c, work_question("ask-2", NotifyLevel::Normal), app_ctx("notes", "alice"))
        .await
        .unwrap();
    env.tasks
        .answer(second.task_id.as_ref().unwrap(), NotifyAnswer::dismiss());
    assert_eq!(
        get(c, &second.notify_id, user_ctx("alice")).await.unwrap().status,
        NotifyStatus::Dismissed
    );
    // Nothing to ack on an input.
    assert!(call::<NotifyView>(c, METHOD_NOTIFY_ACK, json!({"notify_id": second.notify_id}), user_ctx("alice"))
        .await
        .is_err());
    // Replay: same task, no second create.
    send(c, work_question("ask-1", NotifyLevel::Timely), app_ctx("notes", "alice"))
        .await
        .unwrap();
    assert_eq!(env.tasks.count(), 2);
}

#[allow(non_snake_case)]
fn ObjIdOf(s: &str) -> ndn_lib::ObjId {
    ndn_lib::ObjId::new(s).unwrap()
}

#[tokio::test]
async fn ack_dismiss_cancel_and_expiry() {
    let env = env().await;
    let c = &env.center;
    user_says(c, &alice(), "chat-a", next_ms()).await;
    let n = send(c, NotifySendReq::notice("alice", "Reminder", "Water the plants", "r1"), agent_ctx())
        .await
        .unwrap();
    assert_eq!(n.status, NotifyStatus::Open);
    // The Agent acknowledges on the user's behalf.
    let acked: NotifyView = call(
        c,
        METHOD_NOTIFY_ACK,
        json!({"notify_id": n.notify_id, "on_behalf": "jarvis in ui-1"}),
        agent_ctx(),
    )
    .await
    .unwrap();
    assert_eq!(acked.status, NotifyStatus::Read);
    assert_eq!(acked.acked_by.as_deref(), Some("jarvis in ui-1"));

    let m = send(c, NotifySendReq::notice("alice", "News", "", "r2"), app_ctx("notes", "alice"))
        .await
        .unwrap();
    // Reading the home record is reading the notice.
    let home = m.home.clone().unwrap();
    let rec = record(c, &alice(), &home.record_id).await;
    c.update_record_state_internal(rec, RecipientState::Read).await.unwrap();
    assert_eq!(get(c, &m.notify_id, user_ctx("alice")).await.unwrap().status, NotifyStatus::Read);
    let dismissed: NotifyView =
        call(c, METHOD_NOTIFY_DISMISS, json!({"notify_id": m.notify_id}), user_ctx("alice")).await.unwrap();
    assert_eq!(dismissed.status, NotifyStatus::Dismissed);
    assert_eq!(record(c, &alice(), &home.record_id).await.state, RecipientState::Archived);

    let k = send(c, NotifySendReq::notice("alice", "Old", "", "r3"), app_ctx("notes", "alice"))
        .await
        .unwrap();
    let canceled: NotifyView = call(
        c,
        METHOD_NOTIFY_CANCEL,
        json!({"notify_id": k.notify_id, "reason": "no_longer_needed"}),
        app_ctx("notes", "alice"),
    )
    .await
    .unwrap();
    assert_eq!(canceled.status, NotifyStatus::Canceled);
    assert_eq!(canceled.closed_reason.as_deref(), Some("no_longer_needed"));

    let ask = send(c, work_question("ask", NotifyLevel::Normal), agent_ctx()).await.unwrap();
    let stopped: NotifyView = call(
        c,
        METHOD_NOTIFY_CANCEL,
        json!({"notify_id": ask.notify_id, "reason": "turn_stopped"}),
        agent_ctx(),
    )
    .await
    .unwrap();
    assert_eq!(stopped.status, NotifyStatus::Canceled);
    assert_eq!(stopped.closed_reason.as_deref(), Some("turn_stopped"));

    let mut soon = NotifySendReq::notice("alice", "Soon", "", "e1");
    soon.expires_at_ms = Some(1);
    let soon = send(c, soon, app_ctx("notes", "alice")).await.unwrap();
    let mut late = work_question("e2", NotifyLevel::Quiet);
    late.expires_at_ms = Some(1);
    let late = send(c, late, app_ctx("notes", "alice")).await.unwrap();
    let answered = send(c, work_question("e3", NotifyLevel::Quiet), app_ctx("notes", "alice"))
        .await
        .unwrap();
    env.tasks.answer(answered.task_id.as_ref().unwrap(), NotifyAnswer::text("ok"));
    c.sweep_notifies().await.unwrap();
    assert_eq!(get(c, &soon.notify_id, user_ctx("alice")).await.unwrap().status, NotifyStatus::Expired);
    let late = get(c, &late.notify_id, user_ctx("alice")).await.unwrap();
    assert_eq!(late.status, NotifyStatus::Expired);
    assert_eq!(late.closed_reason.as_deref(), Some("expired"));
    let open: NotifyListResult = call(
        c,
        METHOD_NOTIFY_LIST,
        json!({"status": ["open", "read"]}),
        user_ctx("alice"),
    )
    .await
    .unwrap();
    assert!(open.items.iter().all(|v| !v.status.is_closed()));
    let rows = c.notifies.get(&answered.notify_id).await.unwrap().unwrap();
    assert_eq!(rows.state, crate::notify::NotifyRowState::Closed, "the sweep closes answered rows");
}

#[tokio::test]
async fn an_interrupted_send_resumes_with_the_same_key() {
    let env = env().await;
    let c = &env.center;
    user_says(c, &alice(), "chat-a", next_ms()).await;
    let req = work_question("crash", NotifyLevel::Normal);
    let first = send(c, req.clone(), agent_ctx()).await.unwrap();
    // Pretend the process died before the copy: drop the progress.
    let mut row = c.notifies.get(&first.notify_id).await.unwrap().unwrap();
    row.state = crate::notify::NotifyRowState::Delivering;
    row.deliveries.clear();
    row.observed.clear();
    row.home = None;
    row.task_granted = false;
    c.notifies.update(&row).await.unwrap();
    let second = send(c, req, agent_ctx()).await.unwrap();
    assert_eq!(second.notify_id, first.notify_id);
    assert_eq!(second.task_id, first.task_id);
    assert_eq!(second.home.as_ref().unwrap().record_id, first.home.as_ref().unwrap().record_id);
    assert_eq!(second.deliveries[0].msg_id, first.deliveries[0].msg_id);
    assert_eq!(env.tasks.count(), 1);
    assert_eq!(session_records(c, &alice(), "sysnotify:agent").await.len(), 1);
    let obs = session_records(c, &jarvis(), "chat-a").await;
    assert_eq!(obs.iter().filter(|r| r.msg_kind == MsgObjKind::Event).count(), 1);
}

#[derive(Default)]
struct IdentityCalls {
    verify: std::sync::atomic::AtomicUsize,
    resolve_user_did: std::sync::atomic::AtomicUsize,
    zone_agent: std::sync::atomic::AtomicUsize,
}

impl IdentityCalls {
    fn take(&self) -> (usize, usize, usize) {
        (
            self.verify.swap(0, Ordering::SeqCst),
            self.resolve_user_did.swap(0, Ordering::SeqCst),
            self.zone_agent.swap(0, Ordering::SeqCst),
        )
    }
}

struct CountingVerifier {
    inner: Verifier,
    calls: Arc<IdentityCalls>,
}

#[async_trait]
impl SessionTokenVerifier for CountingVerifier {
    async fn authorize(
        &self,
        _token: &str,
        _resource: &str,
        _action: &str,
    ) -> Result<(), RPCErrors> {
        Ok(())
    }

    async fn verify(&self, token: &str) -> Result<RPCSessionToken, RPCErrors> {
        self.calls.verify.fetch_add(1, Ordering::SeqCst);
        self.inner.verify(token).await
    }

    async fn resolve_user_did(&self, user_id: &str) -> Result<DID, RPCErrors> {
        self.calls.resolve_user_did.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve_user_did(user_id).await
    }

    async fn zone_agent(&self, did: &DID) -> Result<Option<ZoneAgentInfo>, RPCErrors> {
        self.calls.zone_agent.fetch_add(1, Ordering::SeqCst);
        self.inner.zone_agent(did).await
    }
}

#[tokio::test]
async fn one_request_establishes_its_caller_once() {
    let e = env().await;
    let c = &e.center;
    let calls = Arc::new(IdentityCalls::default());
    c.set_token_verifier(Arc::new(CountingVerifier {
        inner: Verifier {
            key: DecodingKey::from_ed_components(TEST_PUBLIC_X).unwrap(),
        },
        calls: calls.clone(),
    }));
    let agent_app = || app_ctx("jarvis.test.buckyos.io", "alice");
    let send_req = || NotifySendReq::notice("alice", "Hi", "", "memo-1");

    let unscoped = send(c, send_req(), agent_app()).await.unwrap();
    let (verify, resolve, zone_agent) = calls.take();
    assert!(
        verify >= 2 && resolve >= 2 && zone_agent >= 1,
        "{verify} {resolve} {zone_agent}"
    );

    let scoped = serve_with_request_identity(send(c, send_req(), agent_app()))
        .await
        .unwrap();
    assert_eq!(scoped.notify_id, unscoped.notify_id);
    assert_eq!(calls.take(), (1, 1, 1));

    serve_with_request_identity(send(c, send_req(), agent_app()))
        .await
        .unwrap();
    assert_eq!(calls.take(), (1, 1, 1));

    let expired = token_expiring_at(
        "alice",
        TokenPrincipalKind::App,
        AuthTarget::app("jarvis.test.buckyos.io@alice".parse().unwrap()),
        buckyos_kit::buckyos_get_unix_timestamp() - 3600,
    );
    serve_with_request_identity(async {
        send(c, send_req(), agent_app()).await.unwrap();
        assert!(send(c, send_req(), expired).await.is_err());
    })
    .await;
    assert_eq!(calls.take().0, 2);

    let mut forged = agent_app();
    forged.token = Some(format!("{}x", forged.token.unwrap()));
    serve_with_request_identity(async {
        assert!(send(c, send_req(), forged.clone()).await.is_err());
        assert!(send(c, send_req(), forged.clone()).await.is_err());
    })
    .await;
    assert_eq!(calls.take().0, 2);
}
